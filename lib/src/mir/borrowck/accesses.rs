use crate::ids::TypeId;
use crate::mir::{
    MirClosureCaptureKind, Mutability, Operand, Place, Rvalue, StatementData, StatementKind,
    Terminator,
};
use crate::type_context::{TypeContext, TypeView};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Read,
    Write,
    Move,
    BorrowShared,
    BorrowMut,
    Drop,
    CaptureShared,
    CaptureMut,
    CaptureMove,
    RawPointerCast,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessEvent {
    pub kind: AccessKind,
    pub place: Place,
}

impl AccessKind {
    pub fn for_operand(operand: &Operand) -> Option<Self> {
        match operand {
            Operand::Copy(_) => Some(Self::Read),
            Operand::Move(_) => Some(Self::Move),
            Operand::Constant(_) => None,
        }
    }

    pub fn for_borrow(mutability: Mutability) -> Self {
        match mutability {
            Mutability::Not => Self::BorrowShared,
            Mutability::Mut => Self::BorrowMut,
        }
    }

    pub fn for_capture(kind: MirClosureCaptureKind) -> Option<Self> {
        match kind {
            MirClosureCaptureKind::ByRef => Some(Self::CaptureShared),
            MirClosureCaptureKind::ByMutRef => Some(Self::CaptureMut),
            MirClosureCaptureKind::ByValue => Some(Self::CaptureMove),
        }
    }
}

pub fn classify_statement(stmt: &StatementData, type_context: &TypeContext) -> Vec<AccessEvent> {
    match &stmt.kind {
        StatementKind::Assign(dest, rvalue) => {
            let mut events = classify_rvalue(rvalue, type_context);
            events.push(AccessEvent {
                kind: AccessKind::Write,
                place: dest.clone(),
            });
            events
        }
        StatementKind::Assert(assertion) => assertion
            .operands
            .iter()
            .flat_map(classify_assert_operand)
            .collect(),
        StatementKind::StorageLive(_) => Vec::new(),
        StatementKind::StorageDead(local) => vec![AccessEvent {
            kind: AccessKind::Drop,
            place: Place {
                local: *local,
                projection: vec![],
            },
        }],
    }
}

pub fn classify_terminator(term: &Terminator) -> Vec<AccessEvent> {
    match term {
        Terminator::SwitchInt { discr, .. } | Terminator::SwitchIntWithOrigin { discr, .. } => {
            classify_operand(discr)
        }
        Terminator::Call {
            func,
            args,
            destination,
            ..
        } => {
            let mut events = classify_operand(func);
            for arg in args {
                events.extend(classify_operand(arg));
            }
            events.push(AccessEvent {
                kind: AccessKind::Write,
                place: destination.clone(),
            });
            events
        }
        Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } => {
            vec![AccessEvent {
                kind: AccessKind::Drop,
                place: place.clone(),
            }]
        }
        Terminator::Goto(_)
        | Terminator::Unreachable { .. }
        | Terminator::GotoWithOrigin { .. }
        | Terminator::Return
        | Terminator::ReturnWithOrigin { .. } => Vec::new(),
    }
}

fn classify_rvalue(rvalue: &Rvalue, type_context: &TypeContext) -> Vec<AccessEvent> {
    match rvalue {
        Rvalue::Object(operand, _) | Rvalue::Use(operand) => classify_operand(operand),
        Rvalue::Ref(mutability, place) => vec![AccessEvent {
            kind: AccessKind::for_borrow(*mutability),
            place: place.clone(),
        }],
        Rvalue::Cast(operand, target) if target_is_pointer(type_context, *target) => {
            match operand {
                Operand::Copy(place) | Operand::Move(place) => vec![AccessEvent {
                    kind: AccessKind::RawPointerCast,
                    place: place.clone(),
                }],
                Operand::Constant(_) => Vec::new(),
            }
        }
        Rvalue::Cast(operand, _) => classify_operand(operand),
        Rvalue::Closure(closure) => {
            let mut events = Vec::new();
            for capture in &closure.captures {
                if let Some(kind) = AccessKind::for_capture(capture.kind) {
                    events.push(AccessEvent {
                        kind,
                        place: capture.place(),
                    });
                }
            }
            events
        }
        Rvalue::BinaryOp(_, a, b) => {
            let mut events = classify_operand(a);
            events.extend(classify_operand(b));
            events
        }
        Rvalue::UnaryOp(_, a) => classify_operand(a),
        Rvalue::Discriminant(place) => vec![AccessEvent {
            kind: AccessKind::Read,
            place: place.clone(),
        }],
        Rvalue::Aggregate(_, operands) => {
            let mut events = Vec::new();
            for operand in operands {
                events.extend(classify_operand(operand));
            }
            events
        }
    }
}

fn target_is_pointer(type_context: &TypeContext, target: TypeId) -> bool {
    super::type_id_is_pointer(TypeView::new(type_context), target)
}

fn classify_operand(operand: &Operand) -> Vec<AccessEvent> {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => AccessKind::for_operand(operand)
            .map(|kind| AccessEvent {
                kind,
                place: place.clone(),
            })
            .into_iter()
            .collect(),
        Operand::Constant(_) => Vec::new(),
    }
}

fn classify_assert_operand(operand: &Operand) -> Vec<AccessEvent> {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => vec![AccessEvent {
            kind: AccessKind::Read,
            place: place.clone(),
        }],
        Operand::Constant(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_statement, AccessKind};
    use crate::mir::{Local, Mutability, Operand, Place, Rvalue, StatementData};
    use crate::type_context::TypeContext;
    use crate::types::Type;

    #[test]
    fn test_access_kind_for_mut_borrow_assignment() {
        let stmt = StatementData::assign(
            Place {
                local: Local(0),
                projection: vec![],
            },
            Rvalue::Ref(
                Mutability::Mut,
                Place {
                    local: Local(1),
                    projection: vec![],
                },
            ),
            None,
        );

        let type_context = TypeContext::new();
        let events = classify_statement(&stmt, &type_context);
        assert_eq!(events[0].kind, AccessKind::BorrowMut);
        assert_eq!(events[1].kind, AccessKind::Write);
    }

    #[test]
    fn test_access_kind_for_pointer_cast_assignment() {
        let mut type_context = TypeContext::new();
        let stmt = StatementData::assign(
            Place {
                local: Local(0),
                projection: vec![],
            },
            Rvalue::Cast(
                Operand::Copy(Place {
                    local: Local(1),
                    projection: vec![],
                }),
                type_context.intern_type(&Type::Pointer(Box::new(Type::I64))),
            ),
            None,
        );

        let events = classify_statement(&stmt, &type_context);
        assert_eq!(events[0].kind, AccessKind::RawPointerCast);
        assert_eq!(events[1].kind, AccessKind::Write);
    }

    #[test]
    fn test_access_kind_for_non_pointer_cast_assignment() {
        let mut type_context = TypeContext::new();
        let stmt = StatementData::assign(
            Place {
                local: Local(0),
                projection: vec![],
            },
            Rvalue::Cast(
                Operand::Copy(Place {
                    local: Local(1),
                    projection: vec![],
                }),
                type_context.intern_type(&Type::I64),
            ),
            None,
        );

        let events = classify_statement(&stmt, &type_context);
        assert_eq!(events[0].kind, AccessKind::Read);
        assert_eq!(events[1].kind, AccessKind::Write);
    }

    #[test]
    fn test_access_kind_for_move_assignment() {
        let stmt = StatementData::assign(
            Place {
                local: Local(0),
                projection: vec![],
            },
            Rvalue::Use(Operand::Move(Place {
                local: Local(1),
                projection: vec![],
            })),
            None,
        );

        let type_context = TypeContext::new();
        let events = classify_statement(&stmt, &type_context);
        assert_eq!(events[0].kind, AccessKind::Move);
        assert_eq!(events[1].kind, AccessKind::Write);
    }
}
