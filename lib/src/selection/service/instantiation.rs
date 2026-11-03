use super::*;

impl<'a> SelectionService<'a> {
    pub(super) fn instantiate_impl_method(
        &self,
        imp: &HirImpl,
        receiver: &HirExpr,
        receiver_ty: &Type,
        method_name: &str,
        receiver_adjustment: ReceiverAdjustment,
    ) -> Option<SelectedMethod> {
        self.instantiate_impl_method_with_subst(
            imp,
            receiver,
            receiver_ty,
            method_name,
            HashMap::new(),
            receiver_adjustment,
        )
    }

    pub(super) fn instantiate_impl_method_with_subst(
        &self,
        imp: &HirImpl,
        receiver: &HirExpr,
        receiver_ty: &Type,
        method_name: &str,
        subst: HashMap<GenericParamId, Type>,
        receiver_adjustment: ReceiverAdjustment,
    ) -> Option<SelectedMethod> {
        let method_func = self.impl_method(imp, method_name)?;
        let trait_member_id = match imp.trait_id {
            Some(trait_id) => Some(self.trait_member_id(trait_id, method_name)?),
            None => None,
        };
        let inherited_trait_default =
            !imp.methods.contains_key(method_name) && imp.trait_id.is_some() && {
                let member_id = trait_member_id.expect("trait impl selected above");
                !self
                    .effective_trait_methods
                    .contains_key(&(imp.id, member_id))
            };
        self.instantiate_method_function_with_subst(
            imp,
            receiver,
            receiver_ty,
            method_func,
            trait_member_id,
            inherited_trait_default,
            subst,
            receiver_adjustment,
        )
    }

    pub(super) fn instantiate_method_function_with_subst(
        &self,
        imp: &HirImpl,
        receiver: &HirExpr,
        receiver_ty: &Type,
        mut method_func: HirFunction,
        trait_member_id: Option<DefId>,
        inherited_trait_default: bool,
        mut subst: HashMap<GenericParamId, Type>,
        receiver_adjustment: ReceiverAdjustment,
    ) -> Option<SelectedMethod> {
        let receiver_subst =
            impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())?;
        for (param, ty) in receiver_subst {
            if subst.get(&param).is_some_and(|existing| existing != &ty) {
                return None;
            }
            subst.insert(param, ty);
        }
        if method_func.is_method {
            if let Some(self_param) = method_func.params.first() {
                let expected_self = Self::receiver_mode_expected_self_type(
                    self_param.ty.substitute_generics(&subst),
                    method_func.self_receiver,
                );
                let can_autoref_mut = matches!(receiver_adjustment, ReceiverAdjustment::AutorefMut);
                if Self::self_type_adjustment(
                    &expected_self,
                    receiver_ty,
                    can_autoref_mut,
                    self.impl_owner_is_reference_like(imp),
                    imp.trait_id.is_some(),
                    &mut subst,
                )
                .is_none()
                {
                    return None;
                }
            }
        }

        for param in &mut method_func.params {
            param.ty = param.ty.substitute_generics(&subst);
        }
        if method_func.is_method {
            if let Some(self_param) = method_func.params.first_mut() {
                self_param.ty = receiver_ty.clone();
            }
        }
        let pending_impl_bounds = self.impl_bound_obligations(imp, &subst)?;
        let owner_generic_params: Vec<GenericParamId> =
            imp.type_generics.iter().map(|decl| decl.id).collect();
        let associated_types = imp
            .associated_types
            .iter()
            .map(|assoc| HirAssociatedTypeDef {
                id: assoc.id,
                name: assoc.name.clone(),
                kind: assoc.kind.clone(),
                ty: assoc.ty.substitute_generics(&subst),
            })
            .collect();
        method_func.ret_type = method_func.ret_type.substitute_generics(&subst);
        Self::substitute_function_generic_bounds(&mut method_func, &subst);
        let selected_trait = match imp.trait_id {
            Some(trait_id) => {
                let member_id = trait_member_id?;
                Some(HirSelectedTraitMember {
                    trait_id,
                    member_id,
                    trait_args: imp
                        .trait_arg_types
                        .iter()
                        .map(|arg| arg.substitute_generics(&subst))
                        .collect(),
                })
            }
            None => None,
        };
        let mut target = if inherited_trait_default {
            let selected_trait = selected_trait.clone()?;
            HirMethodCallTarget::trait_method(
                selected_trait.trait_id,
                selected_trait.member_id,
                selected_trait.trait_args,
                HirTraitDispatchKind::TraitBound,
            )
        } else {
            HirMethodCallTarget::impl_method(imp.id, method_func.id, selected_trait)
        };
        let mut owner_substitution = subst
            .iter()
            .filter(|(param, _)| owner_generic_params.contains(param))
            .map(|(&param, ty)| HirTypeBinding {
                param,
                ty: ty.clone(),
            })
            .collect::<Vec<_>>();
        owner_substitution.sort_by_key(|binding| {
            (
                binding.param.owner.crate_id.0,
                binding.param.owner.local.0,
                binding.param.index,
            )
        });
        target.owner_substitution = owner_substitution;
        let origin = match imp.trait_id {
            Some(trait_id) => SelectedOrigin::TraitImpl {
                impl_id: imp.id,
                trait_id,
            },
            None => SelectedOrigin::InherentImpl { impl_id: imp.id },
        };
        let param_start = if method_func.is_method { 1 } else { 0 };

        Some(SelectedMethod {
            receiver: receiver.clone(),
            substituted_params: method_func.params[param_start..].to_vec(),
            return_type: method_func.ret_type.clone(),
            function: Some(method_func),
            impl_def: Some(imp.clone()),
            target,
            origin,
            receiver_adjustment,
            pending_impl_bounds,
            associated_types,
            owner_substitution: subst,
            owner_generic_params,
        })
    }

    pub(super) fn instantiate_constructor_member_with_subst(
        &self,
        imp: &HirImpl,
        mut method_func: HirFunction,
        trait_member_id: DefId,
        subst: HashMap<GenericParamId, Type>,
    ) -> Option<SelectedConstructorMember> {
        if function_has_receiver(&method_func) {
            return None;
        }
        for param in &mut method_func.params {
            param.ty = param.ty.substitute_generics(&subst);
        }
        method_func.ret_type = method_func.ret_type.substitute_generics(&subst);
        Self::substitute_function_generic_bounds(&mut method_func, &subst);
        let pending_impl_bounds = self.impl_bound_obligations(imp, &subst)?;
        let owner_generic_params = imp
            .type_generics
            .iter()
            .map(|param| param.id)
            .collect::<Vec<_>>();
        let selected_trait = HirSelectedTraitMember {
            trait_id: imp.trait_id?,
            member_id: trait_member_id,
            trait_args: imp
                .trait_arg_types
                .iter()
                .map(|arg| arg.substitute_generics(&subst))
                .collect(),
        };
        let mut target =
            HirMethodCallTarget::impl_method(imp.id, method_func.id, Some(selected_trait));
        let mut owner_substitution = subst
            .iter()
            .filter(|(param, _)| owner_generic_params.contains(param))
            .map(|(&param, ty)| HirTypeBinding {
                param,
                ty: ty.clone(),
            })
            .collect::<Vec<_>>();
        owner_substitution.sort_by_key(|binding| {
            (
                binding.param.owner.crate_id.0,
                binding.param.owner.local.0,
                binding.param.index,
            )
        });
        target.owner_substitution = owner_substitution;

        Some(SelectedConstructorMember {
            substituted_params: method_func.params.clone(),
            return_type: method_func.ret_type.clone(),
            function: method_func,
            impl_def: imp.clone(),
            target,
            pending_impl_bounds,
            owner_substitution: subst,
            owner_generic_params,
        })
    }

    fn substitute_function_generic_bounds(
        function: &mut HirFunction,
        subst: &HashMap<GenericParamId, Type>,
    ) {
        for bounds in function.generic_bounds.values_mut() {
            for bound in bounds {
                bound.type_args = bound
                    .type_args
                    .iter()
                    .map(|arg| arg.substitute_generics(subst))
                    .collect();
            }
        }
        function.generic_bounds.predicates = function
            .generic_bounds
            .predicates
            .iter()
            .map(|predicate| predicate.substitute_generics(subst))
            .collect();
    }

    pub(super) fn selected_current_trait_method(
        receiver: HirExpr,
        trait_id: DefId,
        trait_args: Vec<Type>,
        method_func: HirFunction,
    ) -> SelectedMethod {
        let method_id = method_func.id;
        let param_start = if method_func.is_method { 1 } else { 0 };
        let target = HirMethodCallTarget::trait_method(
            trait_id,
            method_id,
            trait_args,
            HirTraitDispatchKind::CurrentTrait,
        );

        SelectedMethod {
            receiver,
            substituted_params: method_func.params[param_start..].to_vec(),
            return_type: method_func.ret_type.clone(),
            function: Some(method_func),
            impl_def: None,
            target,
            origin: SelectedOrigin::CurrentTrait { trait_id },
            receiver_adjustment: ReceiverAdjustment::None,
            pending_impl_bounds: Vec::new(),
            associated_types: Vec::new(),
            owner_substitution: HashMap::new(),
            owner_generic_params: Vec::new(),
        }
    }

    pub(super) fn array_receiver_matches_slice_self(
        expected_self: &Type,
        receiver_ty: &Type,
    ) -> bool {
        match (expected_self, receiver_ty) {
            (Type::Slice(expected_elem), Type::Array(actual_elem, _)) => {
                type_pattern_matches(expected_elem, actual_elem, &mut HashMap::new())
            }
            _ => false,
        }
    }
}
