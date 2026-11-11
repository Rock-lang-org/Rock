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
            let function = self.instances.pre_mir_body(record.id).or(record.declared.as_ref());
            // Observation must not change canonical TypeId allocation or artifacts.
            let mut observed_types = crate::type_context::TypeContext::new();
            let ids = function.map(|function| {
                crate::hir::collect_function_type_ids(function, &mut observed_types)
            }).unwrap_or_default();
            if ids.iter().any(|(location, id)| {
                let Ok(ty) = TypeNormalizer::new(&self.normalization_env)
                    .normalize(&observed_types.type_for(id)) else {
                    return true;
                };
                if matches!(location, crate::hir::HirTypeLocation::ArrayRepeatLength { .. }) {
                    return !matches!(ty, Type::ConstValue(_));
                }
                let mut validator = LengthValidator::new(&self.nominal_field_types, &self.normalization_env);
                visit_type(&ty, &mut validator);
                validator.invalid
            })
                || record.substitution.iter().any(|id| unresolved_array_length(&self.type_context.type_for(*id)))
            {
                let id = match record.origin {
                    super::InstanceOrigin::Function(id) => id,
                    super::InstanceOrigin::ImplMethod { method, .. }
                    | super::InstanceOrigin::TraitDefault { method, .. } => method,
                };
                offenders.push((id, record.symbols.source_name.clone()));
            }
        }
        for (id, name) in offenders {
            let message = format!("cannot determine the array length used by `{name}`");
            let diagnostic = match self.source_map.definition_span(id) {
                Some(span) => Diagnostic::new(message, span.clone()),
                None => Diagnostic::for_internal(message),
            }
            .with_code(DiagnosticCode::Mono)
            .with_help("add a type annotation that fixes the array length");
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
    nominal_stack: HashSet<DefId>,
    binders: Vec<Vec<Kind>>,
    invalid: bool,
}

impl<'a> LengthValidator<'a> {
    fn new(fields: &'a HashMap<DefId, Vec<Type>>, env: &'a TypeNormalizationEnv) -> Self {
        Self { fields, env, nominal_stack: Default::default(), binders: Vec::new(), invalid: false }
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
            // Do not expand recursive nominal layouts indefinitely. Their type
            // arguments are still checked by visit_type_children above.
            if self.nominal_stack.insert(*id) {
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
                self.nominal_stack.remove(id);
            }
        }
    }
}

#[cfg(test)]
mod tests;
