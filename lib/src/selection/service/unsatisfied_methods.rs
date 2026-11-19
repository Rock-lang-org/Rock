//! Explains why no method was selected when a method with the requested name
//! exists for the receiver's type but its impl `where` bounds do not hold.

use super::*;

/// Nesting limit when following a failed bound into the impls that could
/// have satisfied it.
const MAX_BOUND_CHAIN: usize = 8;

/// A method whose impl matches the receiver type but whose bounds fail.
pub struct UnsatisfiedMethodBound {
    /// The receiver type the method was looked up on.
    pub receiver: Type,
    /// The trait providing the method, when it comes from a trait impl.
    pub method_trait: Option<String>,
    /// Failed bounds, outermost first: each is `(type, trait name)`. The last
    /// entry is the type with no applicable impl at all.
    pub chain: Vec<(Type, String)>,
}

impl<'a> SelectionService<'a> {
    pub fn explain_unsatisfied_method<F>(
        &self,
        receiver_candidates: &[ReceiverCandidate],
        method_name: &str,
        mut normalize: F,
    ) -> Option<UnsatisfiedMethodBound>
    where
        F: FnMut(&Type) -> Type,
    {
        for candidate in receiver_candidates {
            let receiver = normalize(&candidate.expr.ty);
            for imp in self
                .impl_ids_in_order()
                .filter_map(|id| self.impls.get(&id))
            {
                if !self
                    .impl_method(imp, method_name)
                    .is_some_and(|method| function_has_receiver(&method))
                {
                    continue;
                }
                let Some(subst) = self.impl_receiver_substitution(imp, method_name, &receiver)
                else {
                    continue;
                };
                let Err(failed) = self.check_impl_bounds(imp, &subst) else {
                    continue;
                };
                let mut chain = Vec::new();
                self.follow_failed_bound(failed, &mut chain);
                return Some(UnsatisfiedMethodBound {
                    receiver,
                    method_trait: imp.trait_name.clone(),
                    chain,
                });
            }
        }
        None
    }

    /// Records `failed`, then descends into the first impl of its trait that
    /// matches the type but whose own bounds fail (`Vec T: Show` needs
    /// `T: Show`), so the chain ends at the type that actually lacks the impl.
    fn follow_failed_bound(&self, failed: (Type, TraitBound), chain: &mut Vec<(Type, String)>) {
        let (subject, bound) = failed;
        let trait_name = self
            .traits
            .get(&bound.trait_id)
            .map(|tr| tr.name.clone())
            .unwrap_or_else(|| format!("{:?}", bound.trait_id));
        chain.push((subject.clone(), trait_name));
        if chain.len() >= MAX_BOUND_CHAIN {
            return;
        }
        let nested = self
            .impl_ids_in_order()
            .filter_map(|id| self.impls.get(&id))
            .find_map(|imp| {
                if imp.trait_id != Some(bound.trait_id)
                    || imp.trait_arg_types.len() != bound.type_args.len()
                {
                    return None;
                }
                let mut subst =
                    impl_receiver_pattern_substitution(imp, &subject, self.impls.values())?;
                for (expected, actual) in imp.trait_arg_types.iter().zip(&bound.type_args) {
                    if !type_pattern_matches(expected, actual, &mut subst) {
                        return None;
                    }
                }
                self.check_impl_bounds(imp, &subst).err()
            });
        if let Some(nested) = nested {
            self.follow_failed_bound(nested, chain);
        }
    }
}
