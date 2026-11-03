use super::nominal_validator::ProductNominalTypeValidator;
use crate::hir::HirMethodCallTarget;
use crate::ids::{AssocTypeId, Idx};
use crate::products::ProductDefId;
use crate::type_services::normalize::TypeNormalizer;
use crate::types::Type;

impl ProductNominalTypeValidator {
    pub(super) fn validate_function_target(
        &self,
        id: ProductDefId,
        _name: &str,
    ) -> Result<(), String> {
        if id.crate_id == self.local_crate {
            if self.functions.contains(&id) {
                return Ok(());
            }
            if self.static_methods.contains(&id) {
                return Ok(());
            }
            if self.methods.contains(&id) {
                return Err(format!(
                    "Product artifact references receiver method target {} as function",
                    id.local_id.0
                ));
            }
            if self.externs.contains(&id) {
                return Err(format!(
                    "Product artifact references extern target {} as function",
                    id.local_id.0
                ));
            }
            return Err(format!(
                "Product artifact references unknown function target {}",
                id.local_id.0
            ));
        }

        if self.dependency_functions.contains(&id) {
            return Ok(());
        }
        if self.dependency_static_methods.contains(&id) {
            return Ok(());
        }
        if self.dependency_methods.contains(&id) {
            return Err(format!(
                "Product artifact references dependency receiver method target {}::{} as function",
                id.crate_id.0, id.local_id.0
            ));
        }
        if self.dependency_externs.contains(&id) {
            return Err(format!(
                "Product artifact references dependency extern target {}::{} as function",
                id.crate_id.0, id.local_id.0
            ));
        }
        Err(format!(
            "Product artifact references unknown dependency function target {}::{}",
            id.crate_id.0, id.local_id.0
        ))
    }

    pub(super) fn validate_extern_target(
        &self,
        id: ProductDefId,
        _name: &str,
    ) -> Result<(), String> {
        if id.crate_id == self.local_crate {
            if self.externs.contains(&id) {
                return Ok(());
            }
            if self.functions.contains(&id) {
                return Err(format!(
                    "Product artifact references function target {} as extern",
                    id.local_id.0
                ));
            }
            return Err(format!(
                "Product artifact references unknown extern target {}",
                id.local_id.0
            ));
        }

        if self.dependency_externs.contains(&id) {
            return Ok(());
        }
        if self.dependency_functions.contains(&id) {
            return Err(format!(
                "Product artifact references dependency function target {}::{} as extern",
                id.crate_id.0, id.local_id.0
            ));
        }
        Err(format!(
            "Product artifact references unknown dependency extern target {}::{}",
            id.crate_id.0, id.local_id.0
        ))
    }

    pub(super) fn validate_function_call_target(&self, id: ProductDefId) -> Result<(), String> {
        if id.crate_id == self.local_crate {
            if self.functions.contains(&id) || self.static_methods.contains(&id) {
                return Ok(());
            }
            if self.methods.contains(&id) {
                return Err(format!(
                    "Product artifact references receiver method target {} as function",
                    id.local_id.0
                ));
            }
            if self.externs.contains(&id) {
                return Err(format!(
                    "Product artifact references extern target {} as function",
                    id.local_id.0
                ));
            }
            return Err(format!(
                "Product artifact references unknown function target {}",
                id.local_id.0
            ));
        }

        if self.dependency_functions.contains(&id) || self.dependency_static_methods.contains(&id) {
            return Ok(());
        }
        if self.dependency_methods.contains(&id) {
            return Err(format!(
                "Product artifact references dependency receiver method target {}::{} as function",
                id.crate_id.0, id.local_id.0
            ));
        }
        if self.dependency_externs.contains(&id) {
            return Err(format!(
                "Product artifact references dependency extern target {}::{} as function",
                id.crate_id.0, id.local_id.0
            ));
        }
        Err(format!(
            "Product artifact references unknown dependency function target {}::{}",
            id.crate_id.0, id.local_id.0
        ))
    }

    pub(super) fn validate_extern_call_target(&self, id: ProductDefId) -> Result<(), String> {
        if id.crate_id == self.local_crate {
            if self.externs.contains(&id) {
                return Ok(());
            }
            if self.functions.contains(&id) {
                return Err(format!(
                    "Product artifact references function target {} as extern",
                    id.local_id.0
                ));
            }
            return Err(format!(
                "Product artifact references unknown extern target {}",
                id.local_id.0
            ));
        }

        if self.dependency_externs.contains(&id) {
            return Ok(());
        }
        if self.dependency_functions.contains(&id) {
            return Err(format!(
                "Product artifact references dependency function target {}::{} as extern",
                id.crate_id.0, id.local_id.0
            ));
        }
        Err(format!(
            "Product artifact references unknown dependency extern target {}::{}",
            id.crate_id.0, id.local_id.0
        ))
    }

    pub(super) fn validate_struct(&self, id: ProductDefId) -> Result<(), String> {
        if id.crate_id == self.local_crate && !self.structs.contains(&id) {
            return Err(format!(
                "Product artifact references unknown nominal type definition {} as struct",
                id.local_id.0
            ));
        }

        if id.crate_id != self.local_crate && !self.dependency_structs.contains(&id) {
            return Err(format!(
                "Product artifact references unknown dependency nominal type definition {}::{} as struct",
                id.crate_id.0, id.local_id.0
            ));
        }

        Ok(())
    }

    pub(super) fn validate_enum(&self, id: ProductDefId) -> Result<(), String> {
        if id.crate_id == self.local_crate && !self.enums.contains(&id) {
            return Err(format!(
                "Product artifact references unknown nominal type definition {} as enum",
                id.local_id.0
            ));
        }

        if id.crate_id != self.local_crate && !self.dependency_enums.contains(&id) {
            return Err(format!(
                "Product artifact references unknown dependency nominal type definition {}::{} as enum",
                id.crate_id.0, id.local_id.0
            ));
        }

        Ok(())
    }

    pub(super) fn validate_type_alias(&self, id: ProductDefId) -> Result<(), String> {
        if id.crate_id == self.local_crate && !self.type_aliases.contains(&id) {
            return Err(format!(
                "Product artifact references unknown type alias definition {}",
                id.local_id.0
            ));
        }
        if id.crate_id != self.local_crate && !self.dependency_type_aliases.contains(&id) {
            return Err(format!(
                "Product artifact references unknown dependency type alias definition {}::{}",
                id.crate_id.0, id.local_id.0
            ));
        }
        Ok(())
    }

    pub(super) fn validate_projection(
        &self,
        trait_id: ProductDefId,
        assoc_owner: ProductDefId,
        assoc_type_id: AssocTypeId,
    ) -> Result<(), String> {
        if assoc_owner != trait_id {
            return Err(format!(
                "Product artifact projection trait {} does not match associated type owner {}",
                trait_id.local_id.0, assoc_owner.local_id.0
            ));
        }

        if trait_id.crate_id == self.local_crate && !self.traits.contains_key(&trait_id) {
            return Err(format!(
                "Product artifact references unknown trait definition {} in projection",
                trait_id.local_id.0
            ));
        }

        if trait_id.crate_id != self.local_crate && !self.dependency_traits.contains_key(&trait_id)
        {
            return Err(format!(
                "Product artifact references unknown dependency trait definition {}::{} in projection",
                trait_id.crate_id.0, trait_id.local_id.0
            ));
        }

        if assoc_owner.crate_id == self.local_crate {
            let Some(associated_types) = self.traits.get(&assoc_owner) else {
                return Err(format!(
                    "Product artifact references unknown associated type owner {} in projection",
                    assoc_owner.local_id.0
                ));
            };

            if !associated_types.contains(&assoc_type_id) {
                return Err(format!(
                    "Product artifact references unknown associated type ID {} on trait {}",
                    assoc_type_id.raw(),
                    assoc_owner.local_id.0
                ));
            }
        } else {
            let Some(associated_types) = self.dependency_traits.get(&assoc_owner) else {
                return Err(format!(
                    "Product artifact references unknown dependency associated type owner {}::{} in projection",
                    assoc_owner.crate_id.0, assoc_owner.local_id.0
                ));
            };

            if !associated_types.contains(&assoc_type_id) {
                return Err(format!(
                    "Product artifact references unknown dependency associated type ID {} on trait {}::{}",
                    assoc_type_id.raw(),
                    assoc_owner.crate_id.0,
                    assoc_owner.local_id.0
                ));
            }
        }

        Ok(())
    }

    fn trait_generic_count(&self, trait_id: ProductDefId) -> Option<usize> {
        if trait_id.crate_id == self.local_crate {
            self.trait_generic_counts.get(&trait_id).copied()
        } else {
            self.dependency_trait_generic_counts.get(&trait_id).copied()
        }
    }

    fn impl_trait_args(&self, impl_id: ProductDefId) -> Option<&[Type]> {
        if impl_id.crate_id == self.local_crate {
            self.impl_trait_args.get(&impl_id).map(Vec::as_slice)
        } else {
            self.dependency_impl_trait_args
                .get(&impl_id)
                .map(Vec::as_slice)
        }
    }

    fn method_receiver_mode(
        &self,
        method_id: ProductDefId,
    ) -> Option<Option<crate::types::ReceiverMode>> {
        if method_id.crate_id == self.local_crate {
            self.method_receiver_modes.get(&method_id).copied()
        } else {
            self.dependency_method_receiver_modes
                .get(&method_id)
                .copied()
        }
    }

    fn validate_trait_arg_arity(
        &self,
        trait_id: ProductDefId,
        trait_args: &[Type],
    ) -> Result<(), String> {
        let expected = self.trait_generic_count(trait_id).ok_or_else(|| {
            format!(
                "Product artifact references unknown trait target {}::{} while validating trait arguments",
                trait_id.crate_id.0, trait_id.local_id.0
            )
        })?;
        if trait_args.len() != expected {
            return Err(format!(
                "Product artifact method authority trait argument arity mismatch for {}::{}: expected {}, found {}",
                trait_id.crate_id.0,
                trait_id.local_id.0,
                expected,
                trait_args.len()
            ));
        }
        Ok(())
    }

    pub(super) fn validate_method_call_target(
        &self,
        target: &HirMethodCallTarget,
    ) -> Result<(), String> {
        self.validate_method_target(target)?;
        let method_id = ProductDefId::from(target.method_id().ok_or_else(|| {
            "Product artifact method-call authority has no selected method identity".to_string()
        })?);
        if self.method_receiver_mode(method_id) == Some(None) {
            return Err(
                "Product artifact method-call authority references a static method".to_string(),
            );
        }
        Ok(())
    }

    pub(super) fn validate_static_method_target(
        &self,
        target: &HirMethodCallTarget,
    ) -> Result<(), String> {
        self.validate_method_target(target)?;
        let method_id = ProductDefId::from(target.method_id().ok_or_else(|| {
            "Product artifact static authority has no selected method identity".to_string()
        })?);
        if self
            .method_receiver_mode(method_id)
            .is_some_and(|mode| mode.is_some())
        {
            return Err(
                "Product artifact static method authority references a receiver method".to_string(),
            );
        }
        Ok(())
    }

    pub(super) fn validate_method_target(
        &self,
        target: &HirMethodCallTarget,
    ) -> Result<(), String> {
        let validate_impl_method = |impl_id: ProductDefId,
                                    method_id: ProductDefId|
         -> Result<(), String> {
            if impl_id.crate_id == self.local_crate && !self.impls.contains(&impl_id) {
                return Err(format!(
                    "Product artifact references unknown impl target {} in method call",
                    impl_id.local_id.0
                ));
            }

            if impl_id.crate_id != self.local_crate && !self.dependency_impls.contains(&impl_id) {
                return Err(format!(
                    "Product artifact references unknown dependency impl target {}::{} in method call",
                    impl_id.crate_id.0, impl_id.local_id.0
                ));
            }

            let method_exists = if impl_id.crate_id == self.local_crate {
                self.impl_methods.get(&impl_id).is_some_and(|methods| {
                    methods.values().any(|candidate| *candidate == method_id)
                })
            } else {
                self.dependency_impl_methods
                    .get(&impl_id)
                    .is_some_and(|methods| {
                        methods.values().any(|candidate| *candidate == method_id)
                    })
            };
            if !method_exists {
                return Err(format!(
                    "Product artifact impl target {} does not own selected method target {}",
                    impl_id.local_id.0, method_id.local_id.0
                ));
            }
            Ok(())
        };

        let validate_trait_member = |trait_id: ProductDefId,
                                     member_id: ProductDefId|
         -> Result<(), String> {
            if trait_id.crate_id == self.local_crate && !self.traits.contains_key(&trait_id) {
                return Err(format!(
                    "Product artifact references unknown trait target {} in method call",
                    trait_id.local_id.0
                ));
            }

            if trait_id.crate_id != self.local_crate
                && !self.dependency_traits.contains_key(&trait_id)
            {
                return Err(format!(
                    "Product artifact references unknown dependency trait target {}::{} in method call",
                    trait_id.crate_id.0, trait_id.local_id.0
                ));
            }

            let member_exists = if trait_id.crate_id == self.local_crate {
                self.trait_methods
                    .get(&trait_id)
                    .into_iter()
                    .chain(self.trait_signatures.get(&trait_id))
                    .any(|members| members.values().any(|candidate| *candidate == member_id))
            } else {
                self.dependency_trait_methods
                    .get(&trait_id)
                    .into_iter()
                    .chain(self.dependency_trait_signatures.get(&trait_id))
                    .any(|members| members.values().any(|candidate| *candidate == member_id))
            };
            if !member_exists {
                return Err(format!(
                    "Product artifact trait target {} does not own selected member target {}",
                    trait_id.local_id.0, member_id.local_id.0
                ));
            }
            Ok(())
        };

        match &target.target {
            crate::hir::HirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait,
            } => {
                let impl_id = ProductDefId::from(*impl_id);
                validate_impl_method(impl_id, ProductDefId::from(*method_id))?;
                if let Some(selected_trait) = selected_trait {
                    let trait_id = ProductDefId::from(selected_trait.trait_id);
                    let actual_trait_id = if impl_id.crate_id == self.local_crate {
                        self.impl_traits.get(&impl_id)
                    } else {
                        self.dependency_impl_traits.get(&impl_id)
                    };
                    if actual_trait_id != Some(&Some(trait_id)) {
                        return Err(format!(
                            "Product artifact method call impl target {} does not match trait target {}",
                            impl_id.local_id.0, trait_id.local_id.0
                        ));
                    }
                    validate_trait_member(trait_id, ProductDefId::from(selected_trait.member_id))?;
                    self.validate_trait_arg_arity(trait_id, &selected_trait.trait_args)?;
                    let owner_substitution = target
                        .owner_substitution
                        .iter()
                        .map(|binding| (binding.param, binding.ty.clone()))
                        .collect::<std::collections::HashMap<_, _>>();
                    let expected_trait_args = self
                        .impl_trait_args(impl_id)
                        .unwrap_or_default()
                        .iter()
                        .map(|arg| {
                            TypeNormalizer::new(&self.normalization_env)
                                .normalize(&arg.substitute_generics(&owner_substitution))
                        })
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|error| format!("Product artifact method authority trait arguments cannot be normalized: {error}"))?;
                    if expected_trait_args != selected_trait.trait_args {
                        return Err(format!(
                            "Product artifact method authority trait arguments do not match impl {}::{}: expected {expected_trait_args:?}, found {:?}",
                            impl_id.crate_id.0,
                            impl_id.local_id.0,
                            selected_trait.trait_args
                        ));
                    }
                    let member_id = ProductDefId::from(selected_trait.member_id);
                    let method_id = ProductDefId::from(*method_id);
                    let effective_method = if impl_id.crate_id == self.local_crate {
                        self.effective_trait_methods.get(&(impl_id, member_id))
                    } else {
                        self.dependency_effective_trait_methods
                            .get(&(impl_id, member_id))
                    };
                    if effective_method != Some(&method_id) {
                        return Err(format!(
                            "Product artifact method target {} is not the effective body for trait member {} on impl {}",
                            method_id.local_id.0,
                            member_id.local_id.0,
                            impl_id.local_id.0,
                        ));
                    }
                } else {
                    let actual_trait_id = if impl_id.crate_id == self.local_crate {
                        self.impl_traits.get(&impl_id)
                    } else {
                        self.dependency_impl_traits.get(&impl_id)
                    };
                    if actual_trait_id.is_some_and(Option::is_some) {
                        return Err(format!(
                            "Product artifact trait impl method authority for impl {}::{} has no selected trait identity",
                            impl_id.crate_id.0, impl_id.local_id.0
                        ));
                    }
                }
            }
            crate::hir::HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                trait_args,
                ..
            } => {
                let trait_id = ProductDefId::from(*trait_id);
                validate_trait_member(trait_id, ProductDefId::from(*member_id))?;
                self.validate_trait_arg_arity(trait_id, trait_args)?;
            }
        }
        Ok(())
    }
}
