//! Instantiation of function types and resolved value types.

use crate::hir::*;
use crate::ids::TypeVarId;
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::types::{GenericParamId, Type};
use std::collections::{HashMap, HashSet};

impl Lowerer {
    pub(crate) fn instantiate_function_type(&mut self, func: &HirFunction, span: Span) -> Type {
        if func.generic_params.is_empty()
            && self.should_instantiate_inferred_function(func.id)
            && self
                .function_type_vars
                .get(&func.id)
                .is_some_and(|vars| !vars.is_empty())
        {
            let mut representatives = HashMap::<TypeVarId, Type>::new();
            let mut substitution = HashMap::<TypeVarId, Type>::new();
            for variable in self
                .function_type_vars
                .get(&func.id)
                .into_iter()
                .flatten()
                .copied()
            {
                let resolved = self.engine.resolve(&Type::TypeVar(variable));
                let replacement = match resolved {
                    Type::TypeVar(representative) => representatives
                        .entry(representative)
                        .or_insert_with(|| {
                            let kind = self.engine.kind_of_type_var(representative);
                            self.engine.fresh_type_var_at_kind(span.clone(), kind)
                        })
                        .clone(),
                    resolved => resolved,
                };
                substitution.insert(variable, replacement);
            }

            // Inferred callable bounds are part of the function scheme, just
            // like its parameter/result types. Freshen their variables together
            // so separate calls neither lose context nor specialize each other.
            substitution.extend(representatives.iter().map(|(id, ty)| (*id, ty.clone())));
            let inferred_bounds = representatives
                .keys()
                .copied()
                .map(|id| (id, self.engine.get_bounds(id)))
                .collect::<Vec<_>>();
            for (_, bounds) in &inferred_bounds {
                for bound in bounds {
                    for argument in &bound.type_args {
                        crate::type_services::visit::visit_type(
                            &self.engine.resolve(argument),
                            &mut |ty: &Type| {
                                if let Type::TypeVar(id) = ty {
                                    if !substitution.contains_key(id) {
                                        let kind = self.engine.kind_of_type_var(*id);
                                        substitution.insert(
                                            *id,
                                            self.engine.fresh_type_var_at_kind(span.clone(), kind),
                                        );
                                    }
                                }
                            },
                        );
                    }
                }
            }
            for (id, bounds) in inferred_bounds {
                let Some(Type::TypeVar(instance)) = substitution.get(&id).cloned() else {
                    continue;
                };
                for bound in bounds {
                    let bound = crate::types::TraitBound {
                        trait_id: bound.trait_id,
                        type_args: bound
                            .type_args
                            .iter()
                            .map(|arg| self.engine.resolve(arg).substitute(&substitution))
                            .collect(),
                    };
                    self.engine.add_bound(instance, bound.clone());
                    self.constraint_store.add_trait(
                        Type::TypeVar(instance),
                        bound,
                        span.clone(),
                        "inferred callable argument",
                    );
                }
            }

            let params = func
                .params
                .iter()
                .map(|param| param.ty.substitute(&substitution))
                .collect();
            let ret = func.ret_type.substitute(&substitution);
            return Type::function_item(
                params,
                ret,
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            );
        }

        let mut generic_params = HashSet::new();
        // Parameters used only in bounds still need fresh call-site variables
        // (for example T in `C: FoldableValue T, A: FnMut T, ()`).
        generic_params.extend(func.generic_params.iter().map(|param| param.id));
        for param in &func.params {
            param.ty.collect_generic_params(&mut generic_params);
        }
        func.ret_type.collect_generic_params(&mut generic_params);

        let subst: HashMap<GenericParamId, Type> = generic_params
            .into_iter()
            .map(|param| {
                let kind = self
                    .engine
                    .kind_of(&Type::Generic(param))
                    .unwrap_or(crate::type_services::kind::Kind::Type);
                (
                    param,
                    self.engine.fresh_type_var_at_kind(span.clone(), kind),
                )
            })
            .collect();

        let context = format!("call to generic function '{}'", func.name);
        for (generic_param, bounds) in &func.generic_bounds {
            let Some(ty) = subst.get(generic_param).cloned() else {
                continue;
            };
            for bound in bounds {
                let instantiated_bound = crate::types::TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|arg| arg.substitute_generics(&subst))
                        .collect(),
                };
                if let Type::TypeVar(id) = &ty {
                    self.engine.add_bound(*id, instantiated_bound.clone());
                }
                self.constraint_store.add_trait(
                    ty.clone(),
                    instantiated_bound,
                    span.clone(),
                    &context,
                );
            }
        }

        let params = func
            .params
            .iter()
            .map(|param| param.ty.substitute_generics(&subst))
            .collect();
        let ret = func.ret_type.substitute_generics(&subst);
        Type::function_item(
            params,
            ret,
            crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
        )
    }

    pub(super) fn infer_bounded_constructor_heads(
        &mut self,
        function_ty: &Type,
        args: &[&HirExpr],
    ) {
        let Type::Function { params, .. } = function_ty else {
            return;
        };

        for (expected, actual) in params.iter().zip(args) {
            let resolved_expected = self.engine.resolve(expected);
            let Type::Apply {
                ref constructor, ..
            } = resolved_expected
            else {
                continue;
            };
            let Type::TypeVar(constructor_id) = self.engine.resolve(constructor) else {
                continue;
            };
            let bounds = self.engine.get_bounds(constructor_id);
            if bounds.is_empty() {
                continue;
            }

            let actual_ty = self.resolve_projection_type(&self.engine.resolve(&actual.ty));
            if let Type::TypeVar(actual_id) = actual_ty {
                if let Err(error) = self
                    .engine
                    .bind_pending_type_var(actual_id, &resolved_expected)
                {
                    self.diagnostics
                        .push_type_with_span(error.render(&self.engine), actual.span.clone());
                }
                continue;
            }
            let mut targets = Vec::new();
            for bound in bounds {
                let trait_args = bound
                    .type_args
                    .iter()
                    .map(|arg| self.engine.resolve(arg))
                    .collect::<Vec<_>>();
                let result = self
                    .selection_service()
                    .infer_constructor_target_for_applied_type(
                        &actual_ty,
                        bound.trait_id,
                        &trait_args,
                    );
                match result {
                    Ok(Some(target)) => targets.push(target),
                    Ok(None) => {}
                    Err(error) => {
                        let message = self.display_selection_error(&error);
                        self.diagnostics
                            .push_selection_with_span(message, actual.span.clone());
                        return;
                    }
                }
            }
            targets.sort_by_key(ToString::to_string);
            targets.dedup();
            if let [target] = targets.as_slice() {
                if let Err(error) = self.engine.unify(&Type::TypeVar(constructor_id), target) {
                    self.diagnostics
                        .push_type_with_span(error.render(&self.engine), actual.span.clone());
                }
            }
        }
    }

    pub(crate) fn instantiate_resolved_value_type(
        &mut self,
        resolved: &crate::lower::resolution::LowerResolvedValue,
        span: Span,
    ) -> Type {
        if let Some(HirVarTarget::Function(function_id)) = resolved.target {
            if let Some(func) = self.items.function(function_id).cloned() {
                return self.instantiate_function_type(&func, span);
            }
        }

        let mut ty = if resolved.should_instantiate {
            self.instantiate_generics(resolved.ty.clone(), span)
        } else {
            resolved.ty.clone()
        };
        if matches!(resolved.target, Some(HirVarTarget::Function(_) | HirVarTarget::Extern(_))) {
            if let Type::Function { captures_known, .. } = &mut ty {
                *captures_known = true;
            }
        }
        ty
    }
}
