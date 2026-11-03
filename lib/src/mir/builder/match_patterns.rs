use std::collections::HashMap;

use crate::hir::{HirLiteralPattern, HirPattern};
use crate::ids::VariantId;
use crate::lexer::Span;
use crate::type_services::facts::TypeFacts;
use crate::types::Type;

use crate::mir::{
    BasicBlockId, Local, LocalSource, MirBinOp, Mutability, Operand, Place, Projection, Rvalue,
};

use super::*;

impl<'a> MirBuilder<'a> {
    pub(super) fn bind_match_pattern_places(
        &mut self,
        pattern: &HirPattern,
        source_place: Place,
        source_ty: &Type,
        guarded: bool,
        enum_payload: bool,
        scrutinee_mode: MatchScrutineeMode,
        moved_enum_scrutinee: &mut Option<Local>,
        restored_bindings: &mut Vec<(String, Option<Local>)>,
    ) {
        match pattern {
            HirPattern::Binding { name, mutable, .. } => {
                let borrowed_payload =
                    enum_payload && scrutinee_mode == MatchScrutineeMode::Borrowed;
                if enum_payload && !borrowed_payload && !TypeFacts::is_copy(source_ty) {
                    if guarded {
                        return;
                    }
                    *moved_enum_scrutinee = Some(source_place.local);
                }
                let mutability = if *mutable {
                    Mutability::Mut
                } else {
                    Mutability::Not
                };
                let binding_ty = if borrowed_payload {
                    Type::Reference {
                        mutable: false,
                        inner: Box::new(source_ty.clone()),
                    }
                } else {
                    source_ty.clone()
                };
                let local = self.new_local_with_source(
                    self.type_id_for(&binding_ty),
                    mutability,
                    Some(name.clone()),
                    LocalSource::UserBinding,
                    None,
                );
                self.emit_storage_live(local, None);
                if let Some(scope_locals) = self.scope_locals.last_mut() {
                    scope_locals.push(local);
                }
                restored_bindings.push((name.clone(), self.var_map.insert(name.clone(), local)));
                let dest = Place {
                    local,
                    projection: vec![],
                };
                let rvalue = if borrowed_payload {
                    Rvalue::Ref(Mutability::Not, source_place)
                } else {
                    Rvalue::Use(self.operand_for_place(source_ty, source_place, guarded))
                };
                self.emit_assign(dest, rvalue, None);
            }
            HirPattern::Enum(_, _, Some(location), subpatterns) => {
                let field_types =
                    self.enum_variant_field_types(location.owner, location.variant_id);
                let generic_subst = match source_ty {
                    Type::Enum { args, .. } => {
                        let mut generic_params = std::collections::HashSet::new();
                        for field_ty in &field_types {
                            field_ty.collect_generic_params(&mut generic_params);
                        }
                        generic_params
                            .into_iter()
                            .filter_map(|param| {
                                args.get(param.index as usize)
                                    .cloned()
                                    .map(|arg| (param, arg))
                            })
                            .collect::<HashMap<_, _>>()
                    }
                    _ => HashMap::new(),
                };
                for (index, subpattern) in subpatterns.iter().enumerate() {
                    let mut field_place = source_place.clone();
                    field_place
                        .projection
                        .push(Projection::Downcast(location.variant_id));
                    field_place.projection.push(Projection::Field {
                        index,
                        identity: None,
                    });
                    let field_ty = field_types
                        .get(index)
                        .map(|ty| ty.substitute_generics(&generic_subst))
                        .unwrap_or(Type::Unit);
                    self.bind_match_pattern_places(
                        subpattern,
                        field_place,
                        &field_ty,
                        guarded,
                        true,
                        scrutinee_mode,
                        moved_enum_scrutinee,
                        restored_bindings,
                    );
                }
            }
            HirPattern::Tuple(subpatterns) => {
                let field_types = match source_ty {
                    Type::Tuple(fields) => fields.as_slice(),
                    _ => &[],
                };
                for (index, subpattern) in subpatterns.iter().enumerate() {
                    let mut field_place = source_place.clone();
                    field_place.projection.push(Projection::Field {
                        index,
                        identity: None,
                    });
                    let field_ty = field_types.get(index).unwrap_or(&Type::Unit);
                    self.bind_match_pattern_places(
                        subpattern,
                        field_place,
                        field_ty,
                        guarded,
                        enum_payload,
                        scrutinee_mode,
                        moved_enum_scrutinee,
                        restored_bindings,
                    );
                }
            }
            HirPattern::Struct(_, Some(struct_id), _, fields) => {
                let Some((_, structure)) = self.program.struct_by_id(*struct_id) else {
                    return;
                };
                let generic_subst = match source_ty {
                    Type::Struct { args, .. } => {
                        let mut generic_params = std::collections::HashSet::new();
                        for field in &structure.fields {
                            field.ty.collect_generic_params(&mut generic_params);
                        }
                        generic_params
                            .into_iter()
                            .filter_map(|param| {
                                args.get(param.index as usize)
                                    .cloned()
                                    .map(|arg| (param, arg))
                            })
                            .collect::<HashMap<_, _>>()
                    }
                    _ => HashMap::new(),
                };
                for field in fields {
                    let Some((index, hir_field)) =
                        structure.fields.iter().enumerate().find(|(_, hir_field)| {
                            field.field.as_ref().is_some_and(|location| {
                                location.owner == *struct_id
                                    && location.field_id == hir_field.id
                                    && location.name == hir_field.name
                            }) || hir_field.name == field.name
                        })
                    else {
                        continue;
                    };
                    let identity = field.field.as_ref().filter(|location| {
                        location.owner == *struct_id
                            && location.field_id == hir_field.id
                            && location.name == hir_field.name
                    });
                    let mut field_place = source_place.clone();
                    field_place.projection.push(self.field_projection(
                        index,
                        identity.map(|location| location.owner),
                        identity.map(|location| location.field_id),
                    ));
                    self.bind_match_pattern_places(
                        &field.pattern,
                        field_place,
                        &hir_field.ty.substitute_generics(&generic_subst),
                        guarded,
                        enum_payload,
                        scrutinee_mode,
                        moved_enum_scrutinee,
                        restored_bindings,
                    );
                }
            }
            HirPattern::Wildcard
            | HirPattern::Literal(_)
            | HirPattern::Or(_)
            | HirPattern::Enum(_, _, None, _)
            | HirPattern::Struct(_, None, _, _) => {}
        }
    }

    pub(super) fn match_arm_variant_id(&self, pattern: &HirPattern) -> Option<VariantId> {
        match pattern {
            HirPattern::Enum(_, _, Some(location), _) => Some(location.variant_id),
            _ => None,
        }
    }

    pub(super) fn enum_payload_patterns_irrefutable(pattern: &HirPattern) -> bool {
        let HirPattern::Enum(_, _, Some(_), subpatterns) = pattern else {
            return false;
        };

        subpatterns
            .iter()
            .all(Self::pattern_is_irrefutable_for_struct_match)
    }

    fn enum_payload_pattern_check_supported(pattern: &HirPattern) -> bool {
        match pattern {
            HirPattern::Literal(
                HirLiteralPattern::Int(_) | HirLiteralPattern::Bool(_) | HirLiteralPattern::Char(_),
            ) => true,
            HirPattern::Literal(HirLiteralPattern::Float(_) | HirLiteralPattern::String(_)) => {
                false
            }
            HirPattern::Enum(_, _, Some(_), _) => Self::enum_payload_patterns_irrefutable(pattern),
            _ => Self::pattern_is_irrefutable_for_struct_match(pattern),
        }
    }

    fn assert_enum_payload_patterns_supported(&self, pattern: &HirPattern) {
        let HirPattern::Enum(_, _, Some(_), subpatterns) = pattern else {
            return;
        };

        if subpatterns
            .iter()
            .any(|pattern| !Self::enum_payload_pattern_check_supported(pattern))
        {
            panic!("unsupported refutable enum payload pattern: {pattern:?}");
        }
    }

    pub(super) fn match_arm_is_catch_all(&self, pattern: &HirPattern) -> bool {
        matches!(pattern, HirPattern::Wildcard | HirPattern::Binding { .. })
    }

    fn pattern_is_irrefutable_for_struct_match(pattern: &HirPattern) -> bool {
        match pattern {
            HirPattern::Wildcard | HirPattern::Binding { .. } => true,
            HirPattern::Tuple(fields) => fields
                .iter()
                .all(Self::pattern_is_irrefutable_for_struct_match),
            HirPattern::Struct(_, _, _, fields) => fields
                .iter()
                .all(|field| Self::pattern_is_irrefutable_for_struct_match(&field.pattern)),
            HirPattern::Literal(_) | HirPattern::Enum(_, _, _, _) | HirPattern::Or(_) => false,
        }
    }

    pub(super) fn lower_match_pattern_check(
        &mut self,
        pattern: &HirPattern,
        source_place: Place,
        source_ty: &Type,
        success: BasicBlockId,
        failure: BasicBlockId,
        origin_span: Option<Span>,
    ) {
        match pattern {
            HirPattern::Wildcard | HirPattern::Binding { .. } => {
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(crate::mir::Terminator::goto(
                        success,
                        self.source_origin(origin_span.clone()),
                    ));
                }
            }
            HirPattern::Enum(_, _, Some(location), _) => {
                let discr_temp =
                    self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
                self.emit_storage_live(discr_temp, None);
                let discr_place = Place {
                    local: discr_temp,
                    projection: vec![],
                };
                self.emit_assign(
                    discr_place.clone(),
                    Rvalue::Discriminant(source_place),
                    None,
                );
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(crate::mir::Terminator::switch_int(
                        Operand::Copy(discr_place),
                        vec![(location.variant_id.0 as i64, success)],
                        failure,
                        self.source_origin(origin_span.clone()),
                    ));
                }
            }
            HirPattern::Literal(literal) => {
                let bool_temp =
                    self.new_local(self.type_id_for(&Type::Bool), Mutability::Not, None);
                self.emit_storage_live(bool_temp, None);
                let bool_place = Place {
                    local: bool_temp,
                    projection: vec![],
                };
                let literal_operand = match literal {
                    HirLiteralPattern::Int(value) => {
                        Operand::Constant(crate::mir::Constant::Int(*value))
                    }
                    HirLiteralPattern::Float(value) => {
                        Operand::Constant(crate::mir::Constant::Float(*value))
                    }
                    HirLiteralPattern::Bool(value) => {
                        Operand::Constant(crate::mir::Constant::Bool(*value))
                    }
                    HirLiteralPattern::String(value) => {
                        Operand::Constant(crate::mir::Constant::String(value.clone()))
                    }
                    HirLiteralPattern::Char(value) => {
                        Operand::Constant(crate::mir::Constant::Char(*value))
                    }
                };
                self.emit_assign(
                    bool_place.clone(),
                    Rvalue::BinaryOp(
                        MirBinOp::Eq,
                        self.operand_for_place(source_ty, source_place, true),
                        literal_operand,
                    ),
                    None,
                );
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(crate::mir::Terminator::switch_int(
                        Operand::Copy(bool_place),
                        vec![(1, success)],
                        failure,
                        self.source_origin(origin_span.clone()),
                    ));
                }
            }
            HirPattern::Tuple(_) | HirPattern::Struct(_, _, _, _)
                if Self::pattern_is_irrefutable_for_struct_match(pattern) =>
            {
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(crate::mir::Terminator::goto(
                        success,
                        crate::mir::MirOrigin::synthetic(
                            crate::mir::MirSyntheticOrigin::ControlFlow,
                            None,
                        ),
                    ));
                }
            }
            HirPattern::Tuple(_)
            | HirPattern::Struct(_, _, _, _)
            | HirPattern::Or(_)
            | HirPattern::Enum(_, _, None, _) => {
                panic!("unsupported refutable MIR match pattern: {pattern:?}");
            }
        }
    }

    pub(super) fn lower_enum_payload_pattern_checks(
        &mut self,
        pattern: &HirPattern,
        source_place: Place,
        source_ty: &Type,
        success: BasicBlockId,
        failure: BasicBlockId,
        origin_span: Option<Span>,
    ) {
        let HirPattern::Enum(_, _, Some(location), subpatterns) = pattern else {
            if let Some(current) = self.current_block {
                self.blocks[current.0].terminator = Some(crate::mir::Terminator::goto(
                    success,
                    self.source_origin(origin_span.clone()),
                ));
            }
            return;
        };

        self.assert_enum_payload_patterns_supported(pattern);

        if subpatterns.is_empty() {
            if let Some(current) = self.current_block {
                self.blocks[current.0].terminator = Some(crate::mir::Terminator::goto(
                    success,
                    self.source_origin(origin_span.clone()),
                ));
            }
            return;
        }

        let field_types = self.enum_variant_field_types(location.owner, location.variant_id);
        let generic_subst = match source_ty {
            Type::Enum { args, .. } => {
                let mut generic_params = std::collections::HashSet::new();
                for field_ty in &field_types {
                    field_ty.collect_generic_params(&mut generic_params);
                }
                generic_params
                    .into_iter()
                    .filter_map(|param| {
                        args.get(param.index as usize)
                            .cloned()
                            .map(|arg| (param, arg))
                    })
                    .collect::<HashMap<_, _>>()
            }
            _ => HashMap::new(),
        };

        for (index, subpattern) in subpatterns.iter().enumerate() {
            let next_success = if index + 1 == subpatterns.len() {
                success
            } else {
                self.new_block()
            };
            let mut field_place = source_place.clone();
            field_place
                .projection
                .push(Projection::Downcast(location.variant_id));
            field_place.projection.push(Projection::Field {
                index,
                identity: None,
            });
            let field_ty = field_types
                .get(index)
                .map(|ty| ty.substitute_generics(&generic_subst))
                .unwrap_or(Type::Unit);
            self.lower_match_pattern_check(
                subpattern,
                field_place,
                &field_ty,
                next_success,
                failure,
                origin_span.clone(),
            );
            if index + 1 < subpatterns.len() {
                self.current_block = Some(next_success);
            }
        }
    }

    pub(super) fn restore_match_bindings(
        &mut self,
        restored_bindings: Vec<(String, Option<Local>)>,
    ) {
        for (name, previous) in restored_bindings.into_iter().rev() {
            if let Some(previous) = previous {
                self.var_map.insert(name, previous);
            } else {
                self.var_map.remove(&name);
            }
        }
    }
}
