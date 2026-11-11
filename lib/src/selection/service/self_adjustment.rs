use super::*;

impl<'a> SelectionService<'a> {
    pub(super) fn impl_method(&self, imp: &HirImpl, method_name: &str) -> Option<HirFunction> {
        if imp.trait_id.is_none() {
            return imp.methods.get(method_name).cloned();
        }
        let trait_id = imp.trait_id?;
        let member_id = self.trait_member_id(trait_id, method_name)?;
        let method_id = *self.effective_trait_methods.get(&(imp.id, member_id))?;
        imp.methods
            .values()
            .find(|method| method.id == method_id)
            .cloned()
    }

    pub(super) fn impl_method_for_trait_member(
        &self,
        imp: &HirImpl,
        member_id: DefId,
    ) -> Option<EffectiveTraitMethod> {
        let method_id = *self.effective_trait_methods.get(&(imp.id, member_id))?;
        if let Some(function) = imp
            .methods
            .values()
            .find(|method| method.id == method_id)
            .cloned()
        {
            return Some(EffectiveTraitMethod {
                function,
                inherited_default: false,
            });
        }

        let trait_id = imp.trait_id?;
        let function = self
            .traits
            .get(&trait_id)?
            .methods
            .values()
            .find(|method| method.id == member_id && method.id == method_id)
            .cloned()?;
        Some(EffectiveTraitMethod {
            function,
            inherited_default: true,
        })
    }

    pub(super) fn impl_method_self_type_adjustment(
        &self,
        imp: &HirImpl,
        method_name: &str,
        receiver_ty: &Type,
        can_autoref_mut: bool,
    ) -> Option<ReceiverAdjustment> {
        let method = self.impl_method(imp, method_name)?;
        self.method_self_type_adjustment(imp, &method, receiver_ty, can_autoref_mut)
    }

    pub(super) fn method_self_type_adjustment(
        &self,
        imp: &HirImpl,
        method: &HirFunction,
        receiver_ty: &Type,
        can_autoref_mut: bool,
    ) -> Option<ReceiverAdjustment> {
        if !function_has_receiver(method) {
            return Some(ReceiverAdjustment::None);
        }
        let self_param = method.params.first()?;
        let mut subst = impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())?;
        if Self::generic_self_binds_receiver_by_value(
            &self_param.ty,
            method.self_receiver,
            receiver_ty,
            &mut subst,
        ) {
            return Some(ReceiverAdjustment::None);
        }
        let expected_self = Self::receiver_mode_expected_self_type(
            self_param.ty.substitute_generics(&subst),
            method.self_receiver,
        );
        Self::self_type_adjustment(
            &expected_self,
            receiver_ty,
            can_autoref_mut,
            self.impl_owner_is_reference_like(imp),
            imp.trait_id.is_some(),
            &mut subst,
        )
    }

    /// A by-value `self` declared as a bare generic parameter (`impl Tr for T`)
    /// matches the receiver type exactly as written, including `&U`. Without
    /// this check the parameter is substituted first and the resulting
    /// reference type is misread as a `&self` pattern.
    pub(super) fn generic_self_binds_receiver_by_value(
        declared_self: &Type,
        self_receiver: Option<crate::types::ReceiverMode>,
        receiver_ty: &Type,
        subst: &mut HashMap<GenericParamId, Type>,
    ) -> bool {
        matches!(declared_self, Type::Generic(_))
            && matches!(
                self_receiver,
                Some(crate::types::ReceiverMode::Move) | None
            )
            && Self::type_pattern_matches_committing(declared_self, receiver_ty, subst)
    }

    pub(super) fn receiver_mode_expected_self_type(
        ty: Type,
        self_receiver: Option<crate::types::ReceiverMode>,
    ) -> Type {
        if matches!(ty, Type::Reference { .. }) {
            return ty;
        }

        match self_receiver {
            Some(crate::types::ReceiverMode::Shared) => Type::Reference {
                mutable: false,
                inner: Box::new(ty),
            },
            Some(crate::types::ReceiverMode::Mut) => Type::Reference {
                mutable: true,
                inner: Box::new(ty),
            },
            Some(crate::types::ReceiverMode::Move) | None => ty,
        }
    }

    pub(super) fn self_type_adjustment(
        expected_self: &Type,
        receiver_ty: &Type,
        can_autoref_mut: bool,
        allow_reference_receiver: bool,
        allow_unresolved_self_referent: bool,
        subst: &mut HashMap<GenericParamId, Type>,
    ) -> Option<ReceiverAdjustment> {
        if let Type::Reference { mutable, inner } = expected_self {
            if *mutable
                && can_autoref_mut
                && matches!(inner.as_ref(), Type::Reference { .. })
                && Self::type_pattern_matches_committing(inner, receiver_ty, subst)
            {
                return Some(ReceiverAdjustment::AutorefMut);
            }
            if let Type::Reference {
                mutable: receiver_mutable,
                inner: receiver_inner,
            } = receiver_ty
            {
                if !allow_reference_receiver {
                    return None;
                }

                if *mutable {
                    if *receiver_mutable
                        && Self::type_pattern_matches_committing(inner, receiver_inner, subst)
                    {
                        return Some(ReceiverAdjustment::None);
                    }
                    return None;
                }

                if allow_reference_receiver {
                    if Self::type_pattern_matches_committing(inner, receiver_ty, subst) {
                        return Some(ReceiverAdjustment::None);
                    }

                    if let Type::Reference {
                        mutable: false,
                        inner: expected_referent,
                    } = inner.as_ref()
                    {
                        if Self::type_pattern_matches_committing(
                            expected_referent,
                            receiver_inner,
                            subst,
                        ) {
                            return Some(if *receiver_mutable {
                                ReceiverAdjustment::MutToSharedRef
                            } else {
                                ReceiverAdjustment::None
                            });
                        }
                    }
                }

                if Self::type_pattern_matches_committing(inner, receiver_inner, subst) {
                    return Some(if *receiver_mutable {
                        ReceiverAdjustment::MutToSharedRef
                    } else {
                        ReceiverAdjustment::None
                    });
                }

                return None;
            }

            if Self::array_receiver_matches_slice_self(inner, receiver_ty)
                || Self::self_referent_matches_receiver(
                    inner,
                    receiver_ty,
                    allow_unresolved_self_referent,
                    subst,
                )
            {
                if *mutable {
                    return can_autoref_mut.then_some(ReceiverAdjustment::AutorefMut);
                }
                return Some(ReceiverAdjustment::AutorefShared);
            }

            if allow_reference_receiver && !*mutable {
                if let Type::Reference {
                    mutable: false,
                    inner: expected_referent,
                } = inner.as_ref()
                {
                    if Self::self_referent_matches_receiver(
                        expected_referent,
                        receiver_ty,
                        allow_unresolved_self_referent,
                        subst,
                    ) {
                        return Some(ReceiverAdjustment::AutorefShared);
                    }
                }
            }

            return None;
        }

        if matches!(expected_self, Type::Generic(_) | Type::TypeVar(_))
            || Self::array_receiver_matches_slice_self(expected_self, receiver_ty)
            || Self::type_pattern_matches_committing(expected_self, receiver_ty, subst)
        {
            return Some(ReceiverAdjustment::None);
        }

        None
    }

    fn self_referent_matches_receiver(
        expected_referent: &Type,
        receiver_ty: &Type,
        allow_unresolved: bool,
        subst: &mut HashMap<GenericParamId, Type>,
    ) -> bool {
        allow_unresolved && matches!(expected_referent, Type::TypeVar(_))
            || Self::type_pattern_matches_committing(expected_referent, receiver_ty, subst)
    }

    pub(super) fn impl_owner_is_reference_like(&self, imp: &HirImpl) -> bool {
        matches!(
            &imp.receiver_pattern,
            HirImplReceiverPattern::Exact(Type::Reference { .. })
        )
    }

    pub(super) fn type_pattern_matches_committing(
        pattern: &Type,
        actual: &Type,
        subst: &mut HashMap<GenericParamId, Type>,
    ) -> bool {
        let mut trial = subst.clone();
        if type_pattern_matches(pattern, actual, &mut trial) {
            *subst = trial;
            true
        } else {
            false
        }
    }
}
