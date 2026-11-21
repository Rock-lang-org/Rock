use std::collections::HashMap;

use crate::hir::{HirBindingMode, HirLiteralPattern, HirPattern};
use crate::ids::VariantId;
use crate::lexer::Span;
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
            HirPattern::Binding {
                name,
                local_id,
                mutable,
                mode,
            } => {
                let binding_span = self.source_local_span(self.current_source_owner, *local_id);
                // A binding reached through a dereferenced reference borrows
                // its place; a borrowed enum scrutinee lends its payloads.
                let borrow = match mode {
                    HirBindingMode::Ref { mutable } => Some(if *mutable {
                        Mutability::Mut
                    } else {
                        Mutability::Not
                    }),
                    HirBindingMode::Value
                        if enum_payload && scrutinee_mode == MatchScrutineeMode::Borrowed =>
                    {
                        Some(Mutability::Not)
                    }
                    HirBindingMode::Value => None,
                };
                if enum_payload && borrow.is_none() && self.needs_move(source_ty) {
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
                let binding_ty = match borrow {
                    Some(borrow) => Type::Reference {
                        mutable: borrow == Mutability::Mut,
                        inner: Box::new(source_ty.clone()),
                    },
                    None => source_ty.clone(),
                };
                let local = self.new_local_with_source(
                    self.type_id_for(&binding_ty),
                    mutability,
                    Some(name.clone()),
                    LocalSource::UserBinding,
                    binding_span.clone(),
                );
                self.emit_storage_live(local, binding_span.clone());
                if let Some(scope_locals) = self.scope_locals.last_mut() {
                    scope_locals.push(local);
                }
                restored_bindings.push((name.clone(), self.var_map.insert(name.clone(), local)));
                let dest = Place {
                    local,
                    projection: vec![],
                };
                let rvalue = match borrow {
                    Some(borrow) => Rvalue::Ref(borrow, source_place),
                    None => Rvalue::Use(self.operand_for_place(source_ty, source_place, guarded)),
                };
                self.emit_assign(dest, rvalue, binding_span);
            }
            HirPattern::Deref(inner) => {
                let Type::Reference { inner: target, .. } = source_ty else {
                    return;
                };
                let mut target_place = source_place;
                target_place.projection.push(Projection::Deref);
                self.bind_match_pattern_places(
                    inner,
                    target_place,
                    target,
                    guarded,
                    enum_payload,
                    scrutinee_mode,
                    moved_enum_scrutinee,
                    restored_bindings,
                );
            }
            HirPattern::Array { patterns, rest, .. } => {
                if guarded
                    && !source_place.projection.contains(&Projection::Deref)
                    && self.needs_move(source_ty)
                {
                    let span = self.locals[source_place.local.0].span.clone();
                    self.diagnostics.push(match span {
                        Some(span) => crate::diagnostic::Diagnostic::new(
                            "unsupported guarded non-copy array pattern".to_string(),
                            span,
                        ),
                        None => crate::diagnostic::Diagnostic::for_internal(
                            "unsupported guarded non-copy array pattern".to_string(),
                        ),
                    });
                    return;
                }
                for (pattern, place, ty) in
                    self.array_pattern_children(patterns, *rest, &source_place, source_ty, true)
                {
                    self.bind_match_pattern_places(
                        &pattern,
                        place,
                        &ty,
                        guarded,
                        enum_payload,
                        scrutinee_mode,
                        moved_enum_scrutinee,
                        restored_bindings,
                    );
                }
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

    pub(super) fn match_arm_is_catch_all(&self, pattern: &HirPattern) -> bool {
        Self::pattern_is_irrefutable_for_struct_match(pattern)
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
            HirPattern::Deref(inner) => Self::pattern_is_irrefutable_for_struct_match(inner),
            HirPattern::Array {
                patterns,
                irrefutable,
                ..
            } => {
                *irrefutable
                    && patterns
                        .iter()
                        .all(Self::pattern_is_irrefutable_for_struct_match)
            }
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
    ) -> bool {
        // False means a fixed array length makes this pattern impossible.
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
                let payload_block = self.new_block();
                let discr_temp =
                    self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
                self.emit_storage_live(discr_temp, None);
                let discr_place = Place {
                    local: discr_temp,
                    projection: vec![],
                };
                self.emit_assign(
                    discr_place.clone(),
                    Rvalue::Discriminant(source_place.clone()),
                    None,
                );
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(crate::mir::Terminator::switch_int(
                        Operand::Copy(discr_place),
                        vec![(location.variant_id.0 as i64, payload_block)],
                        failure,
                        self.source_origin(origin_span.clone()),
                    ));
                }
                self.current_block = Some(payload_block);
                return self.lower_enum_payload_pattern_checks(
                    pattern,
                    source_place,
                    source_ty,
                    success,
                    failure,
                    origin_span,
                );
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
            HirPattern::Deref(inner) => {
                // Tests run on the referenced value.
                let target = match source_ty {
                    Type::Reference { inner, .. } => inner.as_ref().clone(),
                    _ => Type::Error,
                };
                let mut target_place = source_place;
                target_place.projection.push(Projection::Deref);
                return self.lower_match_pattern_check(
                    inner,
                    target_place,
                    &target,
                    success,
                    failure,
                    origin_span,
                );
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
            HirPattern::Tuple(patterns) => {
                let Type::Tuple(types) = source_ty else {
                    return true;
                };
                let children = patterns
                    .iter()
                    .zip(types)
                    .enumerate()
                    .map(|(index, (pattern, ty))| {
                        let mut place = source_place.clone();
                        place.projection.push(Projection::Field {
                            index,
                            identity: None,
                        });
                        (pattern, place, ty.clone())
                    })
                    .collect();
                return self.check_pattern_children(children, success, failure, origin_span);
            }
            HirPattern::Array { patterns, rest, .. } => {
                let required = patterns.len() - usize::from(rest.is_some());
                if let Type::Array(_, crate::types::ArrayLen::Value(length)) = source_ty {
                    let required = required as u64;
                    if *length < required || (rest.is_none() && *length != required) {
                        if let Some(current) = self.current_block {
                            self.blocks[current.0].terminator = Some(crate::mir::Terminator::goto(
                                failure,
                                self.source_origin(origin_span),
                            ));
                        }
                        return false;
                    }
                }
                let length = self.array_pattern_length(&source_place, source_ty);
                let local = self.new_local(self.type_id_for(&Type::Bool), Mutability::Not, None);
                self.emit_storage_live(local, origin_span.clone());
                let condition = Place {
                    local,
                    projection: vec![],
                };
                self.emit_assign(
                    condition.clone(),
                    Rvalue::BinaryOp(
                        if rest.is_some() {
                            MirBinOp::Ge
                        } else {
                            MirBinOp::Eq
                        },
                        length,
                        Operand::Constant(crate::mir::Constant::Int(required as i64)),
                    ),
                    origin_span.clone(),
                );
                let elements = self.new_block();
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(crate::mir::Terminator::switch_int(
                        Operand::Copy(condition),
                        vec![(1, elements)],
                        failure,
                        self.source_origin(origin_span.clone()),
                    ));
                }
                self.current_block = Some(elements);
                // A rest binding itself imposes no value constraint.
                let checks: Vec<_> = patterns
                    .iter()
                    .enumerate()
                    .map(|(index, pattern)| {
                        if *rest == Some(index) || matches!(pattern, HirPattern::Binding { .. }) {
                            HirPattern::Wildcard
                        } else {
                            pattern.clone()
                        }
                    })
                    .collect();
                let children =
                    self.array_pattern_children(&checks, *rest, &source_place, source_ty, false);
                let children = children
                    .iter()
                    .map(|(pattern, place, ty)| (pattern, place.clone(), ty.clone()))
                    .collect();
                return self.check_pattern_children(children, success, failure, origin_span);
            }
            HirPattern::Struct(_, Some(id), args, fields) => {
                let Some((_, structure)) = self.program.struct_by_id(*id) else {
                    return true;
                };
                let substitution = args
                    .iter()
                    .enumerate()
                    .map(|(index, ty)| {
                        (
                            crate::types::GenericParamId {
                                owner: *id,
                                index: index as u32,
                            },
                            ty.clone(),
                        )
                    })
                    .collect();
                let children = fields
                    .iter()
                    .filter_map(|field| {
                        let (index, definition) = structure
                            .fields
                            .iter()
                            .enumerate()
                            .find(|(_, definition)| definition.name == field.name)?;
                        let mut place = source_place.clone();
                        place.projection.push(self.field_projection(
                            index,
                            Some(*id),
                            Some(definition.id),
                        ));
                        Some((
                            &field.pattern,
                            place,
                            definition.ty.substitute_generics(&substitution),
                        ))
                    })
                    .collect();
                return self.check_pattern_children(children, success, failure, origin_span);
            }
            HirPattern::Or(_)
            | HirPattern::Struct(_, None, _, _)
            | HirPattern::Enum(_, _, None, _) => {
                panic!("unsupported refutable MIR match pattern: {pattern:?}");
            }
        }
        true
    }

    fn check_pattern_children(
        &mut self,
        children: Vec<(&HirPattern, Place, Type)>,
        success: BasicBlockId,
        failure: BasicBlockId,
        origin: Option<Span>,
    ) -> bool {
        let count = children.len();
        if count == 0 {
            if let Some(current) = self.current_block {
                self.blocks[current.0].terminator = Some(crate::mir::Terminator::goto(
                    success,
                    self.source_origin(origin),
                ));
            }
            return true;
        }
        let mut possible = true;
        for (index, (pattern, place, ty)) in children.into_iter().enumerate() {
            let next = if index + 1 == count {
                success
            } else {
                self.new_block()
            };
            possible &=
                self.lower_match_pattern_check(pattern, place, &ty, next, failure, origin.clone());
            if index + 1 != count {
                self.current_block = Some(next);
            }
        }
        possible
    }

    pub(super) fn lower_enum_payload_pattern_checks(
        &mut self,
        pattern: &HirPattern,
        source_place: Place,
        source_ty: &Type,
        success: BasicBlockId,
        failure: BasicBlockId,
        origin_span: Option<Span>,
    ) -> bool {
        let HirPattern::Enum(_, _, Some(location), subpatterns) = pattern else {
            if let Some(current) = self.current_block {
                self.blocks[current.0].terminator = Some(crate::mir::Terminator::goto(
                    success,
                    self.source_origin(origin_span.clone()),
                ));
            }
            return true;
        };

        if subpatterns.is_empty() {
            if let Some(current) = self.current_block {
                self.blocks[current.0].terminator = Some(crate::mir::Terminator::goto(
                    success,
                    self.source_origin(origin_span.clone()),
                ));
            }
            return true;
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

        let mut possible = true;
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
            possible &= self.lower_match_pattern_check(
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
        possible
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
