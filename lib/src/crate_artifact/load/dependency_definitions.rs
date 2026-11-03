use std::collections::{BTreeMap, BTreeSet};

use super::ProductIdentityRemap;
use crate::crate_artifact::ArtifactCrateInterface;
use crate::crate_system::CrateContext;
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::products::{ProductCrateId, ProductDefId, ProductFunctionInterface, ProductLocalDefId};
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::{GenericParamDecl, GenericParamId, NominalTypeKind, Type};

#[derive(Debug, Clone, Default)]
pub(super) struct ProductDependencyDefinitions {
    pub(super) normalization_env: TypeNormalizationEnv,
    pub(super) structs: BTreeSet<ProductDefId>,
    pub(super) enums: BTreeSet<ProductDefId>,
    pub(super) type_aliases: BTreeSet<ProductDefId>,
    pub(super) traits: BTreeMap<ProductDefId, BTreeSet<AssocTypeId>>,
    pub(super) trait_generic_counts: BTreeMap<ProductDefId, usize>,
    pub(super) trait_methods: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    pub(super) trait_signatures: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    pub(super) impls: BTreeSet<ProductDefId>,
    pub(super) impl_traits: BTreeMap<ProductDefId, Option<ProductDefId>>,
    pub(super) impl_trait_args: BTreeMap<ProductDefId, Vec<Type>>,
    pub(super) impl_methods: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    pub(super) effective_trait_methods: BTreeMap<(ProductDefId, ProductDefId), ProductDefId>,
    pub(super) methods: BTreeSet<ProductDefId>,
    pub(super) static_methods: BTreeSet<ProductDefId>,
    pub(super) method_receiver_modes: BTreeMap<ProductDefId, Option<crate::types::ReceiverMode>>,
    pub(super) functions: BTreeSet<ProductDefId>,
    pub(super) externs: BTreeSet<ProductDefId>,
    function_names: BTreeMap<ProductDefId, BTreeSet<String>>,
    extern_names: BTreeMap<ProductDefId, BTreeSet<String>>,
}

impl ProductDependencyDefinitions {
    pub(super) fn from_context(ctx: &CrateContext, remap: &ProductIdentityRemap) -> Self {
        let consumer_to_product_crate: BTreeMap<CrateId, ProductCrateId> = remap
            .crate_ids
            .iter()
            .filter_map(|(product_crate, consumer_crate)| {
                (*product_crate != remap.local_crate).then_some((*consumer_crate, *product_crate))
            })
            .collect();

        let mut defs = Self::default();

        let product_id_for_def = |id: DefId| {
            consumer_to_product_crate
                .get(&id.crate_id)
                .copied()
                .map(|crate_id| ProductDefId {
                    crate_id,
                    local_id: ProductLocalDefId(id.local.0),
                })
        };

        for dep in ctx.extern_crates() {
            let interface = dep.metadata().interface();

            for strukt in interface.structs.values() {
                if let Some(id) = product_id_for_def(strukt.id) {
                    defs.structs.insert(id);
                    defs.normalization_env.register_constructor(
                        DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0)),
                        NominalTypeKind::Struct,
                        crate::type_lowering::constructor_kind(&strukt.generic_params),
                    );
                }
            }
            for alias in interface.type_aliases.values() {
                if let Some(id) = product_id_for_def(alias.id) {
                    defs.type_aliases.insert(id);
                }
            }

            let crate_name = dep.name();

            for function in interface.functions.values() {
                register_product_generic_kinds(
                    &mut defs.normalization_env,
                    &function.generic_params,
                    &product_id_for_def,
                );
                if let Some(id) = product_id_for_def(function.id) {
                    defs.functions.insert(id);
                    let canonical_name = interface
                        .canonical_name(function.id)
                        .unwrap_or(function.name.as_str());
                    add_callable_names(
                        &mut defs.function_names,
                        id,
                        crate_name,
                        [canonical_name, function.name.as_str()],
                    );
                }
            }

            for ext in interface.externs.values() {
                if let Some(id) = product_id_for_def(ext.id) {
                    defs.externs.insert(id);
                    let canonical_name = interface
                        .canonical_name(ext.id)
                        .unwrap_or(ext.name.as_str());
                    add_callable_names(
                        &mut defs.extern_names,
                        id,
                        crate_name,
                        [canonical_name, ext.name.as_str()],
                    );
                }
            }

            for enum_ in interface.enums.values() {
                if let Some(id) = product_id_for_def(enum_.id) {
                    defs.enums.insert(id);
                    defs.normalization_env.register_constructor(
                        DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0)),
                        NominalTypeKind::Enum,
                        crate::type_lowering::constructor_kind(&enum_.generic_params),
                    );
                }
            }

            for trait_def in interface.traits.values() {
                register_product_generic_kinds(
                    &mut defs.normalization_env,
                    &trait_def.generic_params,
                    &product_id_for_def,
                );
                register_product_generic_kinds(
                    &mut defs.normalization_env,
                    trait_def.target.iter(),
                    &product_id_for_def,
                );
                let Some(trait_id) = product_id_for_def(trait_def.id) else {
                    continue;
                };

                defs.traits.insert(
                    trait_id,
                    trait_def
                        .associated_types
                        .iter()
                        .map(|assoc| assoc.id)
                        .collect(),
                );
                defs.trait_generic_counts
                    .insert(trait_id, trait_def.generic_params.len());
                defs.trait_methods.insert(
                    trait_id,
                    trait_def
                        .methods
                        .iter()
                        .filter_map(|(name, method)| {
                            product_id_for_def(method.id).map(|id| (name.clone(), id))
                        })
                        .collect(),
                );
                defs.trait_signatures.insert(
                    trait_id,
                    trait_def
                        .signatures
                        .iter()
                        .filter_map(|(name, sig)| {
                            product_id_for_def(sig.id).map(|id| (name.clone(), id))
                        })
                        .collect(),
                );
                defs.methods.extend(
                    trait_def
                        .methods
                        .values()
                        .filter_map(|method| product_id_for_def(method.id)),
                );
                for method in trait_def.methods.values() {
                    register_product_generic_kinds(
                        &mut defs.normalization_env,
                        &method.generic_params,
                        &product_id_for_def,
                    );
                    if let Some(id) = product_id_for_def(method.id) {
                        defs.method_receiver_modes.insert(id, method.self_receiver);
                    }
                }
                for signature in trait_def.signatures.values() {
                    register_product_generic_kinds(
                        &mut defs.normalization_env,
                        &signature.generic_params,
                        &product_id_for_def,
                    );
                    if let Some(id) = product_id_for_def(signature.id) {
                        defs.method_receiver_modes
                            .insert(id, signature.self_receiver);
                    }
                }
                defs.methods.extend(
                    trait_def
                        .signatures
                        .values()
                        .filter_map(|sig| product_id_for_def(sig.id)),
                );
            }

            for imp in interface.impl_items() {
                record_dependency_impl(&mut defs, &imp, interface, crate_name, &product_id_for_def);
            }
            defs.effective_trait_methods.extend(
                interface.effective_trait_methods.iter().filter_map(
                    |(&(impl_id, member_id), &method_id)| {
                        Some((
                            (product_id_for_def(impl_id)?, product_id_for_def(member_id)?),
                            product_id_for_def(method_id)?,
                        ))
                    },
                ),
            );
        }

        defs
    }
}

fn record_dependency_impl<P: crate::hir::HirPhase>(
    defs: &mut ProductDependencyDefinitions,
    imp: &crate::hir::HirImplFor<P>,
    interface: &ArtifactCrateInterface,
    crate_name: &str,
    product_id_for_def: &impl Fn(DefId) -> Option<ProductDefId>,
) {
    let Some(impl_id) = product_id_for_def(imp.id) else {
        return;
    };

    defs.impls.insert(impl_id);
    register_product_generic_kinds(
        &mut defs.normalization_env,
        imp.type_generics.iter().chain(&imp.trait_generics),
        product_id_for_def,
    );
    defs.impl_traits
        .insert(impl_id, imp.trait_id.and_then(product_id_for_def));
    let mut trait_args = imp.trait_arg_types.clone();
    for ty in &mut trait_args {
        ty.remap_def_ids(&mut |id| {
            product_id_for_def(id)
                .map(|id| DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0)))
                .unwrap_or(id)
        });
    }
    defs.impl_trait_args.insert(impl_id, trait_args);
    defs.impl_methods.insert(
        impl_id,
        imp.methods
            .iter()
            .filter_map(|(name, method)| product_id_for_def(method.id).map(|id| (name.clone(), id)))
            .collect(),
    );
    for method in imp.methods.values() {
        register_product_generic_kinds(
            &mut defs.normalization_env,
            &method.generic_params,
            product_id_for_def,
        );
        let Some(id) = product_id_for_def(method.id) else {
            continue;
        };
        defs.methods.insert(id);
        defs.method_receiver_modes.insert(id, method.self_receiver);
        if !method.is_method {
            defs.static_methods.insert(id);
            let display_name = interface.canonical_name(method.id);
            let names = static_method_callable_names(method, display_name);
            add_callable_names(
                &mut defs.function_names,
                id,
                crate_name,
                names.iter().map(String::as_str),
            );
        }
    }
}

pub(super) fn add_callable_names<'a>(
    names_by_id: &mut BTreeMap<ProductDefId, BTreeSet<String>>,
    id: ProductDefId,
    crate_name: &str,
    names: impl IntoIterator<Item = &'a str>,
) {
    for name in names {
        if name.is_empty() {
            continue;
        }

        let names = names_by_id.entry(id).or_default();
        names.insert(name.to_string());
        if !name.contains("::") {
            names.insert(format!("{}::{}", crate_name, name));
        }
    }
}

fn static_method_callable_names<P: crate::hir::HirPhase>(
    method: &crate::hir::HirFunctionFor<P>,
    display_name: Option<&str>,
) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(display_name) = display_name {
        names.push(display_name.to_string());
    }
    names.push(method.name.clone());
    names
}

pub(super) fn static_method_interface_callable_names(
    method: &ProductFunctionInterface,
    display_name: Option<&str>,
) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(display_name) = display_name {
        names.push(display_name.to_string());
    }
    names.push(method.name.clone());
    names
}

pub(super) fn register_product_generic_kinds<'a>(
    env: &mut TypeNormalizationEnv,
    params: impl IntoIterator<Item = &'a GenericParamDecl>,
    product_id_for_def: &impl Fn(DefId) -> Option<ProductDefId>,
) {
    for param in params {
        if let Some(owner) = product_id_for_def(param.id.owner) {
            env.register_generic_kind(
                GenericParamId {
                    owner: DefId::new(CrateId(owner.crate_id.0), LocalDefId(owner.local_id.0)),
                    index: param.id.index,
                },
                param.kind.clone(),
            );
        }
    }
}
