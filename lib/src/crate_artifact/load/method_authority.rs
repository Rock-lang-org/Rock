use super::dependency_definitions::ProductDependencyDefinitions;
use super::object_link::product_type_contains_forbidden_sentinel;
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::products::{CompilerProducts, ProductDefId};

pub(super) fn validate_product_method_authority_maps(
    products: &CompilerProducts,
    dependencies: &ProductDependencyDefinitions,
) -> Result<(), String> {
    for (impl_id, imp) in &products.interface.impls {
        let pattern = &imp.receiver_pattern;
        let pattern_has_forbidden_type = match pattern {
            crate::hir::HirImplReceiverPattern::Exact(ty)
            | crate::hir::HirImplReceiverPattern::Constructor(ty) => {
                product_type_contains_forbidden_sentinel(ty)
            }
            crate::hir::HirImplReceiverPattern::SliceFamily { element } => {
                product_type_contains_forbidden_sentinel(element)
            }
        };
        if pattern_has_forbidden_type {
            return Err(format!(
                "Product artifact impl {}::{} typed receiver pattern contains an unresolved type",
                impl_id.crate_id.0, impl_id.local_id.0
            ));
        }
        let mut actual = std::collections::HashSet::new();
        match pattern {
            crate::hir::HirImplReceiverPattern::Exact(ty)
            | crate::hir::HirImplReceiverPattern::Constructor(ty) => {
                ty.collect_generic_params(&mut actual)
            }
            crate::hir::HirImplReceiverPattern::SliceFamily { element } => {
                element.collect_generic_params(&mut actual)
            }
        }
        for trait_arg in &imp.trait_arg_types {
            trait_arg.collect_generic_params(&mut actual);
        }
        let owner = DefId::new(CrateId(impl_id.crate_id.0), LocalDefId(impl_id.local_id.0));
        let expected = imp
            .type_generics
            .iter()
            .map(|decl| decl.id)
            .collect::<std::collections::HashSet<_>>();
        let invalid_expected = expected
            .iter()
            .any(|param| param.owner != owner || param.index as usize >= imp.type_generics.len());
        let invalid_actual = actual
            .iter()
            .any(|param| param.owner != owner || param.index as usize >= imp.type_generics.len());
        if invalid_expected || invalid_actual || expected != actual {
            return Err(format!(
                "Product artifact impl {}::{} typed receiver/trait argument generic bindings are incomplete or invalid: required {expected:?}, found {actual:?}",
                impl_id.crate_id.0, impl_id.local_id.0
            ));
        }
    }

    let mut expected_keys = std::collections::BTreeSet::new();
    for (impl_id, imp) in &products.interface.impls {
        let Some(trait_id) = imp.trait_id.map(ProductDefId::from) else {
            continue;
        };
        let trait_def = products.interface.traits.get(&trait_id);
        if trait_def.is_none() && !dependencies.traits.contains_key(&trait_id) {
            return Err(format!(
                "Product artifact impl {}::{} references missing trait {}::{}",
                impl_id.crate_id.0, impl_id.local_id.0, trait_id.crate_id.0, trait_id.local_id.0
            ));
        }
        let member_ids = if let Some(trait_def) = trait_def {
            trait_def
                .methods
                .values()
                .map(|member| ProductDefId::from(member.id))
                .chain(
                    trait_def
                        .signatures
                        .iter()
                        .filter(|(name, _)| !trait_def.methods.contains_key(*name))
                        .map(|(_, member)| ProductDefId::from(member.id)),
                )
                .collect::<Vec<_>>()
        } else {
            let methods = dependencies.trait_methods.get(&trait_id);
            dependencies
                .trait_methods
                .get(&trait_id)
                .into_iter()
                .flat_map(|members| members.values().copied())
                .chain(
                    dependencies
                        .trait_signatures
                        .get(&trait_id)
                        .into_iter()
                        .flat_map(|members| members.iter())
                        .filter(|(name, _)| {
                            methods.is_none_or(|methods| !methods.contains_key(*name))
                        })
                        .map(|(_, member_id)| *member_id),
                )
                .collect::<Vec<_>>()
        };
        for member_id in member_ids {
            let key = (*impl_id, member_id);
            expected_keys.insert(key);
            let Some(method_id) = products.interface.effective_trait_methods.get(&key) else {
                return Err(format!(
                    "Product artifact trait impl {}::{} has no effective method for member {}::{}, available={:?}",
                    impl_id.crate_id.0,
                    impl_id.local_id.0,
                    member_id.crate_id.0,
                    member_id.local_id.0,
                    products
                        .interface
                        .effective_trait_methods
                        .keys()
                        .filter(|(candidate_impl, _)| candidate_impl == impl_id)
                        .collect::<Vec<_>>(),
                ));
            };
            let Some(method) = imp
                .methods
                .values()
                .find(|method| ProductDefId::from(method.id) == *method_id)
            else {
                return Err(format!(
                    "Product artifact effective method {}::{} does not belong to impl {}::{}",
                    method_id.crate_id.0,
                    method_id.local_id.0,
                    impl_id.crate_id.0,
                    impl_id.local_id.0,
                ));
            };
            let member_id = DefId::new(
                CrateId(member_id.crate_id.0),
                LocalDefId(member_id.local_id.0),
            );
            let method_id = DefId::new(
                CrateId(method_id.crate_id.0),
                LocalDefId(method_id.local_id.0),
            );
            let member_has_receiver = if let Some(trait_def) = trait_def {
                let mut member_receiver_shape = None;
                for shape in trait_def
                    .methods
                    .values()
                    .filter(|member| member.id == member_id)
                    .map(|member| member.is_method || member.self_receiver.is_some())
                    .chain(
                        trait_def
                            .signatures
                            .values()
                            .filter(|member| member.id == member_id)
                            .map(|member| member.self_receiver.is_some()),
                    )
                {
                    if let Some(existing) = member_receiver_shape {
                        if existing != shape {
                            return Err(format!(
                                "effective trait member {member_id:?} has conflicting receiver/static representations"
                            ));
                        }
                    } else {
                        member_receiver_shape = Some(shape);
                    }
                }
                let Some(member_has_receiver) = member_receiver_shape else {
                    return Err(format!(
                        "Product artifact effective trait member {member_id:?} is not declared by trait {:?}",
                        trait_def.id
                    ));
                };
                member_has_receiver
            } else {
                let product_member_id = ProductDefId::from(member_id);
                let Some(receiver_mode) =
                    dependencies.method_receiver_modes.get(&product_member_id)
                else {
                    return Err(format!(
                        "Product artifact effective trait member {member_id:?} is not declared by dependency trait {trait_id:?}"
                    ));
                };
                receiver_mode.is_some()
            };
            crate::hir::validate_effective_trait_member_receiver_shape(
                member_id,
                method_id,
                member_has_receiver,
                method.is_method || method.self_receiver.is_some(),
            )?;
        }
    }

    let actual_keys = products
        .interface
        .effective_trait_methods
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if actual_keys != expected_keys {
        let missing = expected_keys
            .difference(&actual_keys)
            .take(8)
            .collect::<Vec<_>>();
        let extra = products
            .interface
            .effective_trait_methods
            .keys()
            .filter(|key| !expected_keys.contains(key))
            .take(8)
            .collect::<Vec<_>>();
        return Err(format!(
            "Product artifact effective trait method relation is incomplete or inconsistent; missing={missing:?}, extra={extra:?}"
        ));
    }
    Ok(())
}
