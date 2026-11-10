use super::*;

impl<'a> SelectionService<'a> {
    pub(crate) fn trait_bound_satisfied(&self, ty: &Type, bound: &TraitBound) -> bool {
        if self.assumed_trait_bound_satisfied(ty, bound) {
            return true;
        }

        if self.is_builtin_sized_trait(bound.trait_id) {
            return self.type_is_sized(ty);
        }

        // The same impl may solve a structurally smaller obligation (for
        // example nested carriers); reject cycles in goals, not impl IDs.
        let goal = (ty.clone(), bound.clone());
        {
            let mut active = self.active_obligations.borrow_mut();
            if active.len() >= 128 || !active.insert(goal.clone()) {
                return false;
            }
        }
        let satisfied = self
            .impl_ids_in_order()
            .into_iter()
            .filter_map(|id| self.impls.get(&id))
            .any(|imp| {
                self.impl_matches_trait_ref_strict(imp, ty, bound.trait_id, Some(&bound.type_args))
            });
        self.active_obligations.borrow_mut().remove(&goal);
        satisfied
    }

    pub(crate) fn trait_bounds_with_supertraits(
        &self,
        subject: &Type,
        bounds: &[TraitBound],
    ) -> Vec<TraitBound> {
        fn expand(
            service: &SelectionService<'_>,
            subject: &Type,
            bound: TraitBound,
            visiting: &mut std::collections::HashSet<crate::types::Predicate>,
            output: &mut Vec<TraitBound>,
        ) {
            let key = crate::types::Predicate::Trait {
                subject: subject.clone(),
                trait_id: bound.trait_id,
                args: bound.type_args.clone(),
            };
            if !visiting.insert(key.clone()) {
                return;
            }
            if !output.contains(&bound) {
                output.push(bound.clone());
            }

            if let Some(trait_def) = service.traits.get(&bound.trait_id) {
                let mut subst = crate::selection::generic_substitution_for_owner(
                    &trait_def.generic_params,
                    &bound.type_args,
                );
                let target_id =
                    trait_def
                        .target
                        .as_ref()
                        .map(|target| target.id)
                        .unwrap_or(GenericParamId {
                            owner: trait_def.id,
                            index: trait_def.generic_params.len() as u32,
                        });
                subst.insert(target_id, subject.clone());
                for predicate in &trait_def.predicates {
                    let crate::types::Predicate::Trait {
                        subject: implied_subject,
                        trait_id,
                        args,
                    } = predicate.substitute_generics(&subst);
                    if implied_subject == *subject {
                        expand(
                            service,
                            subject,
                            TraitBound {
                                trait_id,
                                type_args: args,
                            },
                            visiting,
                            output,
                        );
                    }
                }
            }
            visiting.remove(&key);
        }

        let mut output = Vec::new();
        let mut visiting = std::collections::HashSet::new();
        for bound in bounds {
            expand(self, subject, bound.clone(), &mut visiting, &mut output);
        }
        output.sort_by_key(|bound| (bound.trait_id, format!("{:?}", bound.type_args)));
        output
    }

    pub(super) fn assumed_trait_bound_satisfied(&self, ty: &Type, bound: &TraitBound) -> bool {
        if let Type::Generic(param) = ty {
            return self.current_impl_bounds.get(param).is_some_and(|bounds| {
                self.trait_bounds_with_supertraits(ty, bounds)
                    .contains(bound)
            });
        }
        false
    }

    pub(super) fn is_builtin_sized_trait(&self, trait_id: DefId) -> bool {
        self.sized_trait_id == Some(trait_id)
    }

    fn type_is_sized(&self, ty: &Type) -> bool {
        match ty {
            Type::Slice(_) | Type::Str => false,
            Type::Array(inner, _) => self.type_is_sized(inner),
            Type::Tuple(elems) => elems.iter().all(|elem| self.type_is_sized(elem)),
            Type::I8
            | Type::I16
            | Type::I32
            | Type::I64
            | Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::F32
            | Type::F64
            | Type::Bool
            | Type::Char
            | Type::Unit
            | Type::Never
            | Type::Struct { .. }
            | Type::Enum { .. }
            | Type::Reference { .. }
            | Type::Pointer(_)
            | Type::Function { .. } => true,
            Type::Generic(param) => self.current_impl_bounds.get(param).is_some_and(|bounds| {
                bounds
                    .iter()
                    .any(|bound| self.is_builtin_sized_trait(bound.trait_id))
            }),
            Type::Projection { .. }
            | Type::TypeVar(_)
            | Type::Constructor { .. }
            | Type::Apply { .. }
            | Type::Lambda { .. }
            | Type::BoundVar { .. }
            | Type::ConstValue(_)
            | Type::Error => false,
        }
    }
}
