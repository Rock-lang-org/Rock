use std::collections::{HashMap, HashSet};

use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::ids::DefId;
use crate::type_services::kind::Kind;
use crate::type_services::normalize::{TypeNormalizationEnv, TypeNormalizer};
use crate::type_services::visit::{visit_type, visit_type_children, TypeVisitor};
use crate::types::{ArrayLen, GenericParamId, Type};

use super::Monomorphizer;

impl Monomorphizer {
    /// Rejects instances carrying symbolic lengths anywhere in their executable HIR.
    ///
    /// This is the authority for array lengths: MIR and code generation assume
    /// runtime lengths are concrete; bound constructor parameters stay symbolic.
    pub(super) fn check_instance_array_lengths(&mut self) {
        let mut offenders = Vec::new();
        for record in self.instances.records() {
            let mut limit_exceeded = false;
            // Observation must not change canonical TypeId allocation or artifacts.
            let mut observed_types = crate::type_context::TypeContext::new();
            let ids = [self.instances.pre_mir_body(record.id), record.declared.as_ref()]
                .into_iter().flatten().map(|function| {
                crate::hir::collect_function_type_ids(function, &mut observed_types)
            }).collect::<Vec<_>>();
            if ids.iter().flat_map(|ids| ids.iter()).any(|(location, id)| {
                let Ok(ty) = TypeNormalizer::new(&self.normalization_env)
                    .normalize(&observed_types.type_for(id)) else {
                    return true;
                };
                if matches!(location, crate::hir::HirTypeLocation::ArrayRepeatLength { .. }) {
                    return !matches!(ty, Type::ConstValue(_));
                }
                let mut validator = LengthValidator::new(&self.nominal_field_types, &self.normalization_env);
                visit_type(&ty, &mut validator);
                limit_exceeded |= validator.limit_exceeded;
                validator.invalid
            })
                || record.substitution.iter().any(|id| {
                    let Ok(ty) = TypeNormalizer::new(&self.normalization_env)
                        .normalize(&self.type_context.type_for(*id)) else {
                        return true;
                    };
                    let mut validator = LengthValidator::new(&self.nominal_field_types, &self.normalization_env);
                    visit_type(&ty, &mut validator);
                    limit_exceeded |= validator.limit_exceeded;
                    validator.invalid
                })
            {
                let id = match record.origin {
                    super::InstanceOrigin::Function(id) => id,
                    super::InstanceOrigin::ImplMethod { method, .. }
                    | super::InstanceOrigin::TraitDefault { method, .. } => method,
                };
                offenders.push((id, record.symbols.source_name.clone(), limit_exceeded));
            }
        }
        for (id, name, limit_exceeded) in offenders {
            let message = format!("cannot determine the array length used by `{name}`");
            let mut diagnostic = match self.source_map.definition_span(id) {
                Some(span) => Diagnostic::new(message, span.clone()),
                None => Diagnostic::for_internal(message),
            }
            .with_code(DiagnosticCode::Mono)
            .with_help("add a type annotation that fixes the array length");
            if limit_exceeded {
                diagnostic = diagnostic.with_note("array layout validation exceeded 128 distinct nested nominal instantiations".to_string());
            }
            self.diagnostics.push(diagnostic);
        }
    }
}

pub(super) fn unresolved_array_length(ty: &Type) -> bool {
    let fields = Default::default();
    let env = TypeNormalizationEnv::default();
    let mut validator = LengthValidator::new(&fields, &env);
    visit_type(ty, &mut validator);
    validator.invalid
}

struct LengthValidator<'a> {
    fields: &'a HashMap<DefId, Vec<Type>>,
    env: &'a TypeNormalizationEnv,
    nominal_stack: HashSet<Type>,
    binders: Vec<Vec<Kind>>,
    invalid: bool,
    limit_exceeded: bool,
}

impl<'a> LengthValidator<'a> {
    fn new(fields: &'a HashMap<DefId, Vec<Type>>, env: &'a TypeNormalizationEnv) -> Self {
        Self { fields, env, nominal_stack: Default::default(), binders: Vec::new(), invalid: false, limit_exceeded: false }
    }
}

impl TypeVisitor for LengthValidator<'_> {
    fn enter_binders(&mut self, params: &[Kind]) {
        self.binders.push(params.to_vec());
    }

    fn exit_binders(&mut self) {
        self.binders.pop();
    }

    fn visit_type(&mut self, ty: &Type) {
        if let Type::Array(_, len) = ty {
            let bound_here = match len {
                ArrayLen::Bound { depth, index } => self.binders
                    .iter().rev().nth(*depth as usize)
                    .and_then(|params| params.get(*index as usize)) == Some(&Kind::Usize),
                _ => false,
            };
            self.invalid |= !len.is_concrete() && !bound_here;
        }
        visit_type_children(ty, self);
        if let Type::Struct { id, args } | Type::Enum { id, args } = ty {
            // An exact instantiated cycle is safe to revisit only once. A
            // growing sequence of distinct instantiations must fail closed,
            // rather than skip a later instantiation's invalid fields.
            if self.nominal_stack.contains(ty) {
                return;
            }
            if self.nominal_stack.len() >= 128 {
                self.invalid = true;
                self.limit_exceeded = true;
                return;
            }
            if self.nominal_stack.insert(ty.clone()) {
                if let Some(fields) = self.fields.get(id) {
                    let substitution = args.iter().enumerate().map(|(index, ty)| {
                        (GenericParamId { owner: *id, index: index as u32 }, ty.clone())
                    }).collect();
                    for field in fields {
                        match TypeNormalizer::new(self.env).normalize(&field.substitute_generics(&substitution)) {
                            Ok(field) => self.visit_type(&field),
                            Err(_) => self.invalid = true,
                        }
                    }
                }
                self.nominal_stack.remove(ty);
            }
        }
    }
}

#[cfg(test)]
mod tests;
