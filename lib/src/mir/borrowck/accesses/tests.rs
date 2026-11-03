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
