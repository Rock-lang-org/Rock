use super::*;

impl<'a> SelectionService<'a> {
    pub(super) fn impl_receiver_substitution(
        &self,
        imp: &HirImpl,
        method_name: &str,
        receiver_ty: &Type,
    ) -> Option<HashMap<GenericParamId, Type>> {
        if self.impl_method(imp, method_name).is_none() {
            return None;
        }
        impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())
    }

    pub(super) fn impl_matches_trait_ref(
        &self,
        imp: &HirImpl,
        receiver_ty: &Type,
        trait_id: DefId,
        trait_args: Option<&[Type]>,
    ) -> bool {
        self.impl_matches_trait_ref_with_bound_filter(
            imp,
            receiver_ty,
            trait_id,
            trait_args,
            |_| true,
        )
    }

    pub(super) fn impl_matches_trait_ref_strict(
        &self,
        imp: &HirImpl,
        receiver_ty: &Type,
        trait_id: DefId,
        trait_args: Option<&[Type]>,
    ) -> bool {
        // Proving a trait obligation is not method lookup: receiver autoref and
        // autoderef must not turn `&T` into an implementation for `&mut T`.
        if let HirImplReceiverPattern::Exact(pattern) = &imp.receiver_pattern {
            if !type_pattern_matches(pattern, receiver_ty, &mut HashMap::new()) {
                return false;
            }
        }
        self.impl_matches_trait_ref_with_bound_filter(
            imp,
            receiver_ty,
            trait_id,
            trait_args,
            |obligations| obligations.is_empty(),
        )
    }

    fn impl_matches_trait_ref_with_bound_filter<F>(
        &self,
        imp: &HirImpl,
        receiver_ty: &Type,
        trait_id: DefId,
        trait_args: Option<&[Type]>,
        mut bound_filter: F,
    ) -> bool
    where
        F: FnMut(&[(Type, TraitBound)]) -> bool,
    {
        if imp.trait_id != Some(trait_id)
            || trait_args.is_some_and(|trait_args| imp.trait_arg_types.len() != trait_args.len())
        {
            return false;
        }

        let Some(mut subst) =
            impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())
        else {
            return false;
        };

        if let Some(trait_args) = trait_args {
            for (expected, actual) in imp.trait_arg_types.iter().zip(trait_args.iter()) {
                if !type_pattern_matches(expected, actual, &mut subst) {
                    return false;
                }
            }
        }

        let Some(obligations) = self.impl_bound_obligations(imp, &subst) else {
            return false;
        };
        bound_filter(&obligations)
    }

    pub(super) fn impl_trait_ref_substitution(
        &self,
        imp: &HirImpl,
        receiver_ty: &Type,
        trait_id: DefId,
        trait_args: Option<&[Type]>,
    ) -> Option<HashMap<GenericParamId, Type>> {
        if imp.trait_id != Some(trait_id)
            || trait_args.is_some_and(|trait_args| imp.trait_arg_types.len() != trait_args.len())
        {
            return None;
        }

        let mut subst = impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())?;

        if let Some(trait_args) = trait_args {
            for (expected, actual) in imp.trait_arg_types.iter().zip(trait_args.iter()) {
                if !type_pattern_matches(expected, actual, &mut subst) {
                    return None;
                }
            }
        }

        self.impl_bound_obligations(imp, &subst)?;
        Some(subst)
    }

    pub(super) fn impl_bounds_satisfied(
        &self,
        imp: &HirImpl,
        subst: &HashMap<GenericParamId, Type>,
    ) -> bool {
        self.impl_bound_obligations(imp, subst).is_some()
    }

    pub(super) fn impl_bound_obligations(
        &self,
        imp: &HirImpl,
        subst: &HashMap<GenericParamId, Type>,
    ) -> Option<Vec<(Type, TraitBound)>> {
        if imp.bounds.is_empty() && imp.bounds.predicates.is_empty() {
            return Some(Vec::new());
        }

        let mut obligations = Vec::new();
        if !imp.bounds.predicates.is_empty() {
            for predicate in &imp.bounds.predicates {
                let crate::types::Predicate::Trait {
                    subject,
                    trait_id,
                    args,
                } = predicate.substitute_generics(subst);
                let bound = TraitBound {
                    trait_id,
                    type_args: args,
                };
                if self.assumed_trait_bound_satisfied(&subject, &bound) {
                    continue;
                }
                if Self::type_contains_type_var(&subject)
                    || bound.type_args.iter().any(Self::type_contains_type_var)
                {
                    obligations.push((subject, bound));
                    continue;
                }
                if !self.trait_bound_satisfied(&subject, &bound) {
                    return None;
                }
            }
            return Some(obligations);
        }

        for (param, bounds) in &imp.bounds {
            let bounded_ty = Type::Generic(*param).substitute_generics(subst);
            for bound in bounds {
                let bound = TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|arg| arg.substitute_generics(subst))
                        .collect(),
                };
                if self.assumed_trait_bound_satisfied(&bounded_ty, &bound) {
                    continue;
                }
                if Self::type_contains_type_var(&bounded_ty)
                    || bound.type_args.iter().any(Self::type_contains_type_var)
                {
                    obligations.push((bounded_ty.clone(), bound));
                    continue;
                }
                if !self.trait_bound_satisfied(&bounded_ty, &bound) {
                    return None;
                }
            }
        }

        Some(obligations)
    }

    pub(super) fn type_contains_type_var(ty: &Type) -> bool {
        crate::type_services::visit::type_any(ty, |nested| {
            matches!(nested, Type::TypeVar(_) | Type::Generic(_))
        })
    }

    pub(super) fn impl_matches_receiver_type(&self, imp: &HirImpl, receiver_ty: &Type) -> bool {
        impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values()).is_some()
    }

    pub(super) fn impl_index_args_substitution(
        &self,
        imp: &HirImpl,
        member_id: DefId,
        receiver_ty: &Type,
        trait_args: &[Type],
    ) -> Option<HashMap<GenericParamId, Type>> {
        let mut subst = impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())?;
        if self.impl_method_for_trait_member(imp, member_id).is_none()
            || imp.trait_arg_types.len() != trait_args.len()
        {
            return None;
        }

        for (expected, actual) in imp.trait_arg_types.iter().zip(trait_args.iter()) {
            if !type_pattern_matches(expected, actual, &mut subst) {
                return None;
            }
        }

        Some(subst)
    }
}
