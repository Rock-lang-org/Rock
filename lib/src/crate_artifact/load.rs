use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::{ArtifactCrateInterface, ArtifactExport};
use crate::crate_system::{
    CrateContext, ExternCrateBodies, ExternCrateLink, ExternCrateMetadata, ExternCrateRecord,
};
use crate::hir::{HirLanguageItems, HirMethodCallTarget, HirNameTables};
use crate::ids::{AssocTypeId, CrateId, DefId, Idx, LocalDefId};
use crate::products::{
    CompilerProducts, PortableProductArtifact, ProductArtifactHeader, ProductCrateId, ProductDefId,
    ProductEnumInterface, ProductExternInterface, ProductFunctionInterface, ProductImplInterface,
    ProductLocalDefId, ProductStructInterface, ProductTraitInterface, ProductTypeAliasInterface,
};
use crate::type_services::normalize::{TypeNormalizationEnv, TypeNormalizer};
use crate::types::{GenericParamDecl, GenericParamId, NominalTypeKind, Type};

#[derive(Debug, Clone)]
pub(super) struct ProductIdentityRemap {
    local_crate: ProductCrateId,
    crate_ids: BTreeMap<ProductCrateId, CrateId>,
}

impl ProductIdentityRemap {
    pub(super) fn from_products(
        ctx: &mut CrateContext,
        products: &CompilerProducts,
    ) -> Result<Self, String> {
        let local_crate = products.identity_table.local_crate.ok_or_else(|| {
            format!(
                "Product artifact for crate '{}' has no local product crate ID",
                products.crate_identity.name
            )
        })?;

        let mut crate_ids = BTreeMap::new();
        crate_ids.insert(
            local_crate,
            ctx.consumer_crate_id_for_product_identity(&products.crate_identity),
        );

        for (product_crate_id, identity) in &products.identity_table.dependencies {
            if *product_crate_id == local_crate {
                return Err(format!(
                    "Product artifact for crate '{}' maps dependency '{}' to local product crate ID {}",
                    products.crate_identity.name,
                    identity.name,
                    product_crate_id.0
                ));
            }

            if let Some(loaded_identity) = ctx
                .product_crate_ids
                .keys()
                .find(|loaded_identity| loaded_identity.name == identity.name)
            {
                if loaded_identity != identity {
                    return Err(format!(
                        "Product artifact dependency identity mismatch for crate '{}': artifact expects {:?}, loaded {:?}",
                        identity.name, identity, loaded_identity
                    ));
                }
            }

            crate_ids.insert(
                *product_crate_id,
                ctx.consumer_crate_id_for_product_identity(identity),
            );
        }

        Ok(Self {
            local_crate,
            crate_ids,
        })
    }

    pub(super) fn def_id(&self, id: ProductDefId) -> Result<DefId, String> {
        let Some(crate_id) = self.crate_ids.get(&id.crate_id).copied() else {
            return Err(format!(
                "Product artifact references unmapped product DefId {}::{}",
                id.crate_id.0, id.local_id.0
            ));
        };

        Ok(DefId::new(crate_id, LocalDefId(id.local_id.0)))
    }
}

#[derive(Debug, Clone)]
struct ProductNominalTypeValidator {
    normalization_env: TypeNormalizationEnv,
    local_crate: ProductCrateId,
    structs: BTreeSet<ProductDefId>,
    enums: BTreeSet<ProductDefId>,
    type_aliases: BTreeSet<ProductDefId>,
    traits: BTreeMap<ProductDefId, BTreeSet<AssocTypeId>>,
    trait_generic_counts: BTreeMap<ProductDefId, usize>,
    trait_methods: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    trait_signatures: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    impls: BTreeSet<ProductDefId>,
    impl_traits: BTreeMap<ProductDefId, Option<ProductDefId>>,
    impl_trait_args: BTreeMap<ProductDefId, Vec<Type>>,
    impl_methods: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    effective_trait_methods: BTreeMap<(ProductDefId, ProductDefId), ProductDefId>,
    methods: BTreeSet<ProductDefId>,
    static_methods: BTreeSet<ProductDefId>,
    method_receiver_modes: BTreeMap<ProductDefId, Option<crate::types::ReceiverMode>>,
    functions: BTreeSet<ProductDefId>,
    externs: BTreeSet<ProductDefId>,
    dependency_structs: BTreeSet<ProductDefId>,
    dependency_enums: BTreeSet<ProductDefId>,
    dependency_type_aliases: BTreeSet<ProductDefId>,
    dependency_traits: BTreeMap<ProductDefId, BTreeSet<AssocTypeId>>,
    dependency_trait_generic_counts: BTreeMap<ProductDefId, usize>,
    dependency_trait_methods: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    dependency_trait_signatures: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    dependency_impls: BTreeSet<ProductDefId>,
    dependency_impl_traits: BTreeMap<ProductDefId, Option<ProductDefId>>,
    dependency_impl_trait_args: BTreeMap<ProductDefId, Vec<Type>>,
    dependency_impl_methods: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    dependency_effective_trait_methods: BTreeMap<(ProductDefId, ProductDefId), ProductDefId>,
    dependency_methods: BTreeSet<ProductDefId>,
    dependency_static_methods: BTreeSet<ProductDefId>,
    dependency_method_receiver_modes: BTreeMap<ProductDefId, Option<crate::types::ReceiverMode>>,
    dependency_functions: BTreeSet<ProductDefId>,
    dependency_externs: BTreeSet<ProductDefId>,
}

#[derive(Debug, Clone, Default)]
struct ProductDependencyDefinitions {
    normalization_env: TypeNormalizationEnv,
    structs: BTreeSet<ProductDefId>,
    enums: BTreeSet<ProductDefId>,
    type_aliases: BTreeSet<ProductDefId>,
    traits: BTreeMap<ProductDefId, BTreeSet<AssocTypeId>>,
    trait_generic_counts: BTreeMap<ProductDefId, usize>,
    trait_methods: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    trait_signatures: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    impls: BTreeSet<ProductDefId>,
    impl_traits: BTreeMap<ProductDefId, Option<ProductDefId>>,
    impl_trait_args: BTreeMap<ProductDefId, Vec<Type>>,
    impl_methods: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    effective_trait_methods: BTreeMap<(ProductDefId, ProductDefId), ProductDefId>,
    methods: BTreeSet<ProductDefId>,
    static_methods: BTreeSet<ProductDefId>,
    method_receiver_modes: BTreeMap<ProductDefId, Option<crate::types::ReceiverMode>>,
    functions: BTreeSet<ProductDefId>,
    externs: BTreeSet<ProductDefId>,
    function_names: BTreeMap<ProductDefId, BTreeSet<String>>,
    extern_names: BTreeMap<ProductDefId, BTreeSet<String>>,
}

impl ProductDependencyDefinitions {
    fn from_context(ctx: &CrateContext, remap: &ProductIdentityRemap) -> Self {
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

fn add_callable_names<'a>(
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

fn static_method_interface_callable_names(
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

fn register_product_generic_kinds<'a>(
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

impl ProductNominalTypeValidator {
    #[cfg(test)]
    fn from_products(products: &CompilerProducts, remap: &ProductIdentityRemap) -> Self {
        Self::from_products_with_dependencies(
            products,
            remap,
            ProductDependencyDefinitions::default(),
        )
    }

    fn from_products_with_context(
        products: &CompilerProducts,
        remap: &ProductIdentityRemap,
        ctx: &CrateContext,
    ) -> Self {
        Self::from_products_with_dependencies(
            products,
            remap,
            ProductDependencyDefinitions::from_context(ctx, remap),
        )
    }

    fn from_products_with_dependencies(
        products: &CompilerProducts,
        remap: &ProductIdentityRemap,
        dependencies: ProductDependencyDefinitions,
    ) -> Self {
        let mut impls = BTreeSet::new();
        let mut trait_generic_counts = BTreeMap::new();
        let mut trait_methods = BTreeMap::new();
        let mut trait_signatures = BTreeMap::new();
        let mut impl_traits = BTreeMap::new();
        let mut impl_trait_args = BTreeMap::new();
        let mut impl_methods = BTreeMap::new();
        let mut methods = BTreeSet::new();
        let mut static_methods = BTreeSet::new();
        let mut method_receiver_modes = BTreeMap::new();
        let mut function_names = BTreeMap::new();
        let mut extern_names = BTreeMap::new();
        let mut normalization_env = dependencies.normalization_env;
        let local_id = |id| Some(ProductDefId::from(id));
        for (id, structure) in &products.interface.structs {
            normalization_env.register_constructor(
                DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0)),
                NominalTypeKind::Struct,
                crate::type_lowering::constructor_kind(&structure.generic_params),
            );
        }
        for (id, enumeration) in &products.interface.enums {
            normalization_env.register_constructor(
                DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0)),
                NominalTypeKind::Enum,
                crate::type_lowering::constructor_kind(&enumeration.generic_params),
            );
        }

        for (id, trait_def) in &products.interface.traits {
            register_product_generic_kinds(
                &mut normalization_env,
                &trait_def.generic_params,
                &local_id,
            );
            register_product_generic_kinds(
                &mut normalization_env,
                trait_def.target.iter(),
                &local_id,
            );
            trait_generic_counts.insert(*id, trait_def.generic_params.len());
            trait_methods.insert(
                *id,
                trait_def
                    .methods
                    .iter()
                    .map(|(name, method)| (name.clone(), ProductDefId::from(method.id)))
                    .collect(),
            );
            trait_signatures.insert(
                *id,
                trait_def
                    .signatures
                    .iter()
                    .map(|(name, sig)| (name.clone(), ProductDefId::from(sig.id)))
                    .collect(),
            );
            methods.extend(
                trait_def
                    .methods
                    .values()
                    .map(|method| ProductDefId::from(method.id)),
            );
            methods.extend(
                trait_def
                    .signatures
                    .values()
                    .map(|signature| ProductDefId::from(signature.id)),
            );
            for method in trait_def.methods.values() {
                register_product_generic_kinds(
                    &mut normalization_env,
                    &method.generic_params,
                    &local_id,
                );
                method_receiver_modes.insert(ProductDefId::from(method.id), method.self_receiver);
            }
            for signature in trait_def.signatures.values() {
                register_product_generic_kinds(
                    &mut normalization_env,
                    &signature.generic_params,
                    &local_id,
                );
                method_receiver_modes
                    .insert(ProductDefId::from(signature.id), signature.self_receiver);
            }
        }

        for (id, imp) in &products.interface.impls {
            register_product_generic_kinds(
                &mut normalization_env,
                imp.type_generics.iter().chain(&imp.trait_generics),
                &local_id,
            );
            impls.insert(*id);
            impl_traits.insert(*id, imp.trait_id.map(ProductDefId::from));
            impl_trait_args.insert(*id, imp.trait_arg_types.clone());
            impl_methods.insert(
                *id,
                imp.methods
                    .iter()
                    .map(|(name, method)| (name.clone(), ProductDefId::from(method.id)))
                    .collect(),
            );
            methods.extend(
                imp.methods
                    .values()
                    .map(|method| ProductDefId::from(method.id)),
            );
            for method in imp.methods.values().filter(|method| !method.is_method) {
                let method_id = ProductDefId::from(method.id);
                static_methods.insert(method_id);
                let display_name = product_display_name(products, method_id);
                let names = static_method_interface_callable_names(method, display_name.as_deref());
                add_callable_names(
                    &mut function_names,
                    method_id,
                    &products.crate_identity.name,
                    names.iter().map(String::as_str),
                );
            }
            for method in imp.methods.values() {
                register_product_generic_kinds(
                    &mut normalization_env,
                    &method.generic_params,
                    &local_id,
                );
                method_receiver_modes.insert(ProductDefId::from(method.id), method.self_receiver);
            }
        }

        for (id, function) in &products.interface.functions {
            register_product_generic_kinds(
                &mut normalization_env,
                &function.generic_params,
                &local_id,
            );
            let display_name = product_display_name(products, *id);
            add_callable_names(
                &mut function_names,
                *id,
                &products.crate_identity.name,
                display_name
                    .as_deref()
                    .into_iter()
                    .chain(std::iter::once(function.name.as_str())),
            );
        }

        for (id, ext) in &products.interface.externs {
            let display_name = product_display_name(products, *id);
            add_callable_names(
                &mut extern_names,
                *id,
                &products.crate_identity.name,
                display_name
                    .as_deref()
                    .into_iter()
                    .chain(std::iter::once(ext.name.as_str())),
            );
        }

        Self {
            normalization_env,
            local_crate: remap.local_crate,
            structs: products.interface.structs.keys().copied().collect(),
            enums: products.interface.enums.keys().copied().collect(),
            type_aliases: products.interface.type_aliases.keys().copied().collect(),
            traits: products
                .interface
                .traits
                .iter()
                .map(|(id, trait_def)| {
                    (
                        *id,
                        trait_def
                            .associated_types
                            .iter()
                            .map(|assoc| assoc.id)
                            .collect(),
                    )
                })
                .collect(),
            trait_generic_counts,
            trait_methods,
            trait_signatures,
            impls,
            impl_traits,
            impl_trait_args,
            impl_methods,
            effective_trait_methods: products.interface.effective_trait_methods.clone(),
            methods,
            static_methods,
            method_receiver_modes,
            functions: products.interface.functions.keys().copied().collect(),
            externs: products.interface.externs.keys().copied().collect(),
            dependency_structs: dependencies.structs,
            dependency_enums: dependencies.enums,
            dependency_type_aliases: dependencies.type_aliases,
            dependency_traits: dependencies.traits,
            dependency_trait_generic_counts: dependencies.trait_generic_counts,
            dependency_trait_methods: dependencies.trait_methods,
            dependency_trait_signatures: dependencies.trait_signatures,
            dependency_impls: dependencies.impls,
            dependency_impl_traits: dependencies.impl_traits,
            dependency_impl_trait_args: dependencies.impl_trait_args,
            dependency_impl_methods: dependencies.impl_methods,
            dependency_effective_trait_methods: dependencies.effective_trait_methods,
            dependency_methods: dependencies.methods,
            dependency_static_methods: dependencies.static_methods,
            dependency_method_receiver_modes: dependencies.method_receiver_modes,
            dependency_functions: dependencies.functions,
            dependency_externs: dependencies.externs,
        }
    }

    fn validate_function_target(&self, id: ProductDefId, _name: &str) -> Result<(), String> {
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

    fn validate_extern_target(&self, id: ProductDefId, _name: &str) -> Result<(), String> {
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

    fn validate_function_call_target(&self, id: ProductDefId) -> Result<(), String> {
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

    fn validate_extern_call_target(&self, id: ProductDefId) -> Result<(), String> {
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

    fn validate_struct(&self, id: ProductDefId) -> Result<(), String> {
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

    fn validate_enum(&self, id: ProductDefId) -> Result<(), String> {
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

    fn validate_type_alias(&self, id: ProductDefId) -> Result<(), String> {
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

    fn validate_projection(
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

    fn validate_method_call_target(&self, target: &HirMethodCallTarget) -> Result<(), String> {
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

    fn validate_static_method_target(&self, target: &HirMethodCallTarget) -> Result<(), String> {
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

    fn validate_method_target(&self, target: &HirMethodCallTarget) -> Result<(), String> {
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

impl CrateContext {
    pub fn load_product_artifact_from_path(
        &mut self,
        artifact_path: PathBuf,
    ) -> Result<(), String> {
        let mut context = crate::type_context::TypeContext::new();
        self.load_product_artifact_from_path_with_type_context(artifact_path, &mut context)
    }

    pub(crate) fn load_product_artifact_from_path_with_type_context(
        &mut self,
        artifact_path: PathBuf,
        context: &mut crate::type_context::TypeContext,
    ) -> Result<(), String> {
        let header = ProductArtifactHeader::read_from_path_bounded(&artifact_path)?;
        let products =
            PortableProductArtifact::read_payload_from_path_bounded(&artifact_path, &header)?
                .into_products()?;
        super::language_items::validate_product_language_items(&products)?;
        let mut staged_crate_context = self.clone();
        let remap = ProductIdentityRemap::from_products(&mut staged_crate_context, &products)?;
        let extern_record =
            extern_crate_from_products(&products, &artifact_path, &remap, &staged_crate_context)?;
        let mut staged_context = context.clone();
        intern_loaded_extern_crate_types(&extern_record, &mut staged_context)?;
        staged_crate_context.add_extern_crate(extern_record)?;

        *self = staged_crate_context;
        *context = staged_context;

        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn load_product_artifact_from_path_as(
        &mut self,
        expected_name: &str,
        artifact_path: PathBuf,
    ) -> Result<(), String> {
        let mut context = crate::type_context::TypeContext::new();
        self.load_product_artifact_from_path_as_with_type_context(
            expected_name,
            artifact_path,
            &mut context,
        )
    }

    #[cfg(test)]
    pub(crate) fn load_product_artifact_from_path_as_with_type_context(
        &mut self,
        expected_name: &str,
        artifact_path: PathBuf,
        context: &mut crate::type_context::TypeContext,
    ) -> Result<(), String> {
        let header = ProductArtifactHeader::read_from_path_bounded(&artifact_path)?;
        self.load_product_artifact_from_header_as_with_type_context(
            expected_name,
            artifact_path,
            &header,
            context,
        )
    }

    pub(crate) fn load_product_artifact_from_header_as_with_type_context(
        &mut self,
        expected_name: &str,
        artifact_path: PathBuf,
        header: &ProductArtifactHeader,
        context: &mut crate::type_context::TypeContext,
    ) -> Result<(), String> {
        let products =
            PortableProductArtifact::read_payload_from_path_bounded(&artifact_path, header)?
                .into_products()?;
        let crate_name = products.crate_identity.name.clone();
        if crate_name != expected_name {
            return Err(format!(
                "Product artifact {} declares crate '{}', but --extern-artifact used name '{}'",
                artifact_path.display(),
                crate_name,
                expected_name
            ));
        }

        super::language_items::validate_product_language_items(&products)?;

        let mut staged_crate_context = self.clone();
        let remap = ProductIdentityRemap::from_products(&mut staged_crate_context, &products)?;
        let extern_record =
            extern_crate_from_products(&products, &artifact_path, &remap, &staged_crate_context)?;
        let mut staged_context = context.clone();
        intern_loaded_extern_crate_types(&extern_record, &mut staged_context)?;
        staged_crate_context.add_extern_crate(extern_record)?;

        *self = staged_crate_context;
        *context = staged_context;

        Ok(())
    }
}

pub(crate) fn intern_loaded_interface_types(
    interface: &super::ArtifactCrateInterface,
    resolver: &ResolverTables,
    context: &mut crate::type_context::TypeContext,
) -> Result<(), String> {
    macro_rules! id_map_from_interface {
        ($entries:expr, $convert:path, $kind:literal) => {{
            let mut items = HashMap::new();
            for (id, item) in $entries {
                let item = $convert(item);
                if item.id != *id {
                    return Err(format!(
                        "artifact {} ID mismatch: key {:?}, payload {:?}",
                        $kind, id, item.id
                    ));
                }
                items.insert(*id, item);
            }
            items
        }};
    }

    let functions = id_map_from_interface!(
        &interface.functions,
        crate::crate_artifact::types::hir_function_from_interface,
        "function"
    );
    let structs = id_map_from_interface!(
        &interface.structs,
        crate::crate_artifact::types::hir_struct_from_interface,
        "struct"
    );
    let enums = id_map_from_interface!(
        &interface.enums,
        crate::crate_artifact::types::hir_enum_from_interface,
        "enum"
    );
    let type_aliases = id_map_from_interface!(
        &interface.type_aliases,
        crate::crate_artifact::types::hir_type_alias_from_interface,
        "type alias"
    );
    let traits = id_map_from_interface!(
        &interface.traits,
        crate::crate_artifact::types::hir_trait_from_interface,
        "trait"
    );
    let impls = id_map_from_interface!(
        &interface.impls,
        crate::crate_artifact::types::hir_impl_from_interface,
        "impl"
    );
    let externs = id_map_from_interface!(
        &interface.externs,
        crate::crate_artifact::types::hir_extern_from_interface,
        "extern"
    );
    let names = artifact_hir_name_tables(
        interface,
        resolver,
        &functions,
        &structs,
        &enums,
        &type_aliases,
        &traits,
        &externs,
    )?;
    let canonical_names = artifact_canonical_names(
        interface,
        resolver,
        &functions,
        &structs,
        &enums,
        &type_aliases,
        &traits,
        &externs,
    );
    let mut program = crate::hir::HirProgram::from_id_parts_with_names_and_canonical_names(
        functions,
        structs,
        enums,
        traits,
        impls,
        externs,
        names,
        HirLanguageItems::default(),
        &canonical_names,
    );
    program.type_aliases = type_aliases;
    program.rebuild_indexes_with_canonical_names(&canonical_names);
    let _ = crate::hir::collect_hir_type_ids(&program, context);
    Ok(())
}

fn artifact_hir_name_tables(
    interface: &ArtifactCrateInterface,
    resolver: &ResolverTables,
    functions: &HashMap<DefId, crate::hir::HirFunction>,
    structs: &HashMap<DefId, crate::hir::HirStruct>,
    enums: &HashMap<DefId, crate::hir::HirEnum>,
    type_aliases: &HashMap<DefId, crate::hir::HirTypeAlias>,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
    externs: &HashMap<DefId, crate::hir::HirExtern>,
) -> Result<HirNameTables, String> {
    fn names_for<T>(
        interface: &ArtifactCrateInterface,
        resolver: &ResolverTables,
        items: &HashMap<DefId, T>,
        kind: &str,
    ) -> Result<HashMap<String, DefId>, String> {
        fn add_candidates<T>(
            candidates: &mut Vec<(String, DefId)>,
            entries: &HashMap<String, DefId>,
            items: &HashMap<DefId, T>,
        ) {
            candidates.extend(
                entries
                    .iter()
                    .filter(|(_, id)| items.contains_key(id))
                    .map(|(name, id)| (name.clone(), *id)),
            );
        }

        let mut names = HashMap::new();
        let mut candidates = Vec::new();
        add_candidates(&mut candidates, &resolver.item_paths, items);
        for (id, name) in &resolver.item_names_by_id {
            if items.contains_key(id) {
                candidates.push((name.clone(), *id));
            }
        }
        for aliases in [
            &resolver.import_aliases,
            &resolver.export_aliases,
            &resolver.module_aliases,
        ] {
            add_candidates(&mut candidates, aliases, items);
        }
        for (id, name) in &interface.canonical_names {
            if items.contains_key(id) {
                candidates.push((name.clone(), *id));
            }
        }
        for (name, export) in &interface.root_export_ids {
            if items.contains_key(&export.id) {
                candidates.push((name.clone(), export.id));
                candidates.push((export.source.clone(), export.id));
            }
        }
        candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

        for (name, id) in candidates {
            if let Some(previous) = names.get(&name) {
                if *previous != id {
                    return Err(format!(
                        "artifact {kind} display alias '{name}' resolves to both {previous:?} and {id:?}"
                    ));
                }
            }
            names.insert(name, id);
        }
        Ok(names)
    }

    Ok(HirNameTables {
        functions_by_name: names_for(interface, resolver, functions, "function")?,
        structs_by_name: names_for(interface, resolver, structs, "struct")?,
        enums_by_name: names_for(interface, resolver, enums, "enum")?,
        traits_by_name: names_for(interface, resolver, traits, "trait")?,
        externs_by_name: names_for(interface, resolver, externs, "extern")?,
        type_aliases_by_name: names_for(interface, resolver, type_aliases, "type alias")?,
    })
}

fn artifact_canonical_names(
    interface: &ArtifactCrateInterface,
    resolver: &ResolverTables,
    functions: &HashMap<DefId, crate::hir::HirFunction>,
    structs: &HashMap<DefId, crate::hir::HirStruct>,
    enums: &HashMap<DefId, crate::hir::HirEnum>,
    type_aliases: &HashMap<DefId, crate::hir::HirTypeAlias>,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
    externs: &HashMap<DefId, crate::hir::HirExtern>,
) -> HashMap<DefId, String> {
    let contains_id = |id: &DefId| {
        functions.contains_key(id)
            || structs.contains_key(id)
            || enums.contains_key(id)
            || type_aliases.contains_key(id)
            || traits.contains_key(id)
            || externs.contains_key(id)
    };
    let mut names = interface
        .canonical_names
        .iter()
        .filter(|(id, _)| contains_id(id))
        .map(|(id, name)| (*id, name.clone()))
        .collect::<HashMap<_, _>>();
    names.extend(
        resolver
            .item_names_by_id
            .iter()
            .filter(|(id, _)| contains_id(id))
            .map(|(id, name)| (*id, name.clone())),
    );
    names
}

pub(crate) fn intern_loaded_body_types(
    bodies: &ExternCrateBodies,
    context: &mut crate::type_context::TypeContext,
) {
    let providers = bodies.providers();
    let program = crate::hir::HirProgramFor::<crate::hir::AcceptedHir>::from_accepted_id_parts_with_names_and_canonical_names(
        providers.generic_functions().clone().into_iter().collect(),
        HashMap::new(),
        HashMap::new(),
        providers.traits_with_defaults().clone().into_iter().collect(),
        providers.generic_impls().clone().into_iter().collect(),
        HashMap::new(),
        HirNameTables::default(),
        &HashMap::new(),
    );
    let _ = crate::hir::collect_hir_type_ids(&program, context);
}

pub(crate) fn intern_loaded_extern_crate_types(
    record: &ExternCrateRecord,
    context: &mut crate::type_context::TypeContext,
) -> Result<(), String> {
    intern_loaded_interface_types(
        record.metadata().interface(),
        record.metadata().resolver(),
        context,
    )?;
    intern_loaded_body_types(record.bodies(), context);
    Ok(())
}

fn extern_crate_from_products(
    products: &CompilerProducts,
    artifact_path: &Path,
    remap: &ProductIdentityRemap,
    ctx: &CrateContext,
) -> Result<ExternCrateRecord, String> {
    let crate_name = products.crate_identity.name.clone();
    let crate_id = remap
        .crate_ids
        .get(&remap.local_crate)
        .copied()
        .ok_or_else(|| {
            format!(
                "Product artifact for crate '{}' has no remapped local crate ID",
                crate_name
            )
        })?;
    validate_product_interface_rows(products)?;
    validate_product_type_fields(products, remap, ctx)?;
    let language_items = super::language_items::language_items_from_products(products, remap)?;
    validate_object_link_records(products)?;
    let backend_symbols = backend_symbols_from_products(products, remap)?;
    let link = if products.link.object_path.is_some() {
        ExternCrateLink::object(
            resolve_product_object_path(products, artifact_path)?,
            backend_symbols,
        )
    } else {
        ExternCrateLink::metadata_only(backend_symbols)
    };
    let interface = interface_from_products(products, &crate_name, remap, ctx, &language_items)?;
    let resolver = resolver_from_products(products, &crate_name, remap, ctx)?;
    let cross_crate_hir = cross_crate_hir_from_products(products, &crate_name, remap, ctx)?;
    let prelude_export_ids = normalize_product_prelude_exports(
        products,
        remap,
        &interface,
        &crate_name,
        &format!("{}::prelude", crate_name),
    )?;

    Ok(ExternCrateRecord::new(
        crate_id,
        crate_name,
        ExternCrateMetadata::new(interface, resolver, prelude_export_ids)
            .with_language_items(language_items)
            .with_proc_macros(products.proc_macros.clone()),
        ExternCrateBodies::from_cross_crate_hir(cross_crate_hir),
        link,
    ))
}

fn validate_product_type_fields(
    products: &CompilerProducts,
    remap: &ProductIdentityRemap,
    ctx: &CrateContext,
) -> Result<(), String> {
    let child_validator =
        ProductChildLocationValidator::from_products_with_context(products, remap, ctx);
    let type_validator =
        ProductNominalTypeValidator::from_products_with_context(products, remap, ctx);

    for (id, function) in &products.interface.functions {
        remap_function_interface_id(function.clone(), *id, remap, &type_validator)?;
    }
    for (id, strukt) in &products.interface.structs {
        remap_struct_interface_id(strukt.clone(), *id, remap, &type_validator)?;
    }
    for (id, enm) in &products.interface.enums {
        remap_enum_interface_id(enm.clone(), *id, remap, &type_validator)?;
    }
    for (id, alias) in &products.interface.type_aliases {
        remap_type_alias_interface_id(alias.clone(), *id, remap, &type_validator)?;
    }
    for (id, trait_def) in &products.interface.traits {
        remap_trait_interface_ids(trait_def.clone(), *id, remap, &type_validator)?;
    }
    for (id, impl_def) in &products.interface.impls {
        remap_impl_interface_ids(impl_def.clone(), *id, remap, &type_validator)?;
    }
    for (id, extern_def) in &products.interface.externs {
        remap_extern_interface_id(extern_def.clone(), *id, remap, &type_validator)?;
    }

    for (id, function) in &products.bodies.functions {
        remap_function_id(
            function.clone(),
            *id,
            remap,
            &child_validator,
            &type_validator,
        )?;
    }
    for (id, impl_def) in &products.bodies.generic_impls {
        remap_impl_ids(
            impl_def.clone(),
            *id,
            &products.crate_identity.name,
            remap,
            &child_validator,
            &type_validator,
        )?;
    }
    for (id, function) in &products.bodies.trait_default_methods {
        remap_function_id(
            function.clone(),
            *id,
            remap,
            &child_validator,
            &type_validator,
        )?;
    }

    Ok(())
}

fn validate_product_interface_rows(products: &CompilerProducts) -> Result<(), String> {
    let local_crate = products.identity_table.local_crate.ok_or_else(|| {
        format!(
            "Product artifact for crate '{}' has no local product crate ID",
            products.crate_identity.name
        )
    })?;
    let mut callable_ids = BTreeMap::new();

    for (id, function) in &products.interface.functions {
        validate_product_interface_local_id("function", *id, local_crate)?;
        validate_product_interface_def_id("function", *id, function.id)?;
        record_product_callable_id(
            &mut callable_ids,
            *id,
            format!("function '{}'", function.name),
        )?;
    }

    for (id, struct_def) in &products.interface.structs {
        validate_product_interface_local_id("struct", *id, local_crate)?;
        validate_product_interface_def_id("struct", *id, struct_def.id)?;
    }

    for (id, enum_def) in &products.interface.enums {
        validate_product_interface_local_id("enum", *id, local_crate)?;
        validate_product_interface_def_id("enum", *id, enum_def.id)?;
    }

    for (id, alias) in &products.interface.type_aliases {
        validate_product_interface_local_id("type alias", *id, local_crate)?;
        validate_product_interface_def_id("type alias", *id, alias.id)?;
    }

    for (id, trait_def) in &products.interface.traits {
        validate_product_interface_local_id("trait", *id, local_crate)?;
        validate_product_interface_def_id("trait", *id, trait_def.id)?;
        for (method_name, method) in &trait_def.methods {
            validate_product_interface_method_name("trait", *id, method_name, method)?;
            record_product_nested_callable_id(
                &mut callable_ids,
                local_crate,
                "trait",
                *id,
                "method",
                method_name,
                method.id,
            )?;
        }
        for (signature_name, signature) in &trait_def.signatures {
            validate_product_interface_signature_name("trait", *id, signature_name, signature)?;
            record_product_nested_callable_id(
                &mut callable_ids,
                local_crate,
                "trait",
                *id,
                "signature",
                signature_name,
                signature.id,
            )?;
        }
    }

    for (id, impl_def) in &products.interface.impls {
        validate_product_interface_local_id("impl", *id, local_crate)?;
        validate_product_interface_def_id("impl", *id, impl_def.id)?;
        for (method_name, method) in &impl_def.methods {
            validate_product_interface_method_name("impl", *id, method_name, method)?;
            record_product_nested_callable_id(
                &mut callable_ids,
                local_crate,
                "impl",
                *id,
                "method",
                method_name,
                method.id,
            )?;
        }
    }

    for (id, extern_def) in &products.interface.externs {
        validate_product_interface_local_id("extern", *id, local_crate)?;
        validate_product_interface_def_id("extern", *id, extern_def.id)?;
        record_product_callable_id(
            &mut callable_ids,
            *id,
            format!("extern '{}'", extern_def.name),
        )?;
    }

    Ok(())
}

fn validate_product_interface_local_id(
    row_kind: &str,
    row_id: ProductDefId,
    local_crate: ProductCrateId,
) -> Result<(), String> {
    if row_id.crate_id != local_crate {
        return Err(format!(
            "Product artifact interface {row_kind} row {}::{} is non-local; expected product crate {}",
            row_id.crate_id.0, row_id.local_id.0, local_crate.0
        ));
    }

    Ok(())
}

fn validate_product_interface_def_id(
    row_kind: &str,
    row_id: ProductDefId,
    embedded_id: DefId,
) -> Result<(), String> {
    let embedded_product_id = ProductDefId::from(embedded_id);
    if embedded_product_id != row_id {
        return Err(format!(
            "Product artifact interface {row_kind} row {} has embedded ID {}",
            row_id.local_id.0, embedded_product_id.local_id.0
        ));
    }

    Ok(())
}

fn validate_product_interface_method_name(
    owner_kind: &str,
    owner_id: ProductDefId,
    method_name: &str,
    method: &ProductFunctionInterface,
) -> Result<(), String> {
    if method.name != method_name {
        return Err(format!(
            "Product artifact interface {owner_kind} {} method '{}' has name '{}'",
            owner_id.local_id.0, method_name, method.name
        ));
    }

    Ok(())
}

fn record_product_nested_callable_id(
    callable_ids: &mut BTreeMap<ProductDefId, String>,
    local_crate: ProductCrateId,
    owner_kind: &str,
    owner_id: ProductDefId,
    callable_kind: &str,
    callable_name: &str,
    embedded_id: DefId,
) -> Result<(), String> {
    let id = ProductDefId::from(embedded_id);
    if id.crate_id != local_crate {
        return Err(format!(
            "Product artifact interface {owner_kind} {} {callable_kind} '{}' uses non-local callable ID {}::{}; expected product crate {}",
            owner_id.local_id.0, callable_name, id.crate_id.0, id.local_id.0, local_crate.0
        ));
    }

    record_product_callable_id(
        callable_ids,
        id,
        format!(
            "{owner_kind} {} {callable_kind} '{}'",
            owner_id.local_id.0, callable_name
        ),
    )
}

fn record_product_callable_id(
    callable_ids: &mut BTreeMap<ProductDefId, String>,
    id: ProductDefId,
    label: String,
) -> Result<(), String> {
    if let Some(previous) = callable_ids.insert(id, label.clone()) {
        return Err(format!(
            "Product artifact duplicate callable ID {}::{} for {}; already used by {}",
            id.crate_id.0, id.local_id.0, label, previous
        ));
    }

    Ok(())
}

fn validate_product_interface_signature_name(
    owner_kind: &str,
    owner_id: ProductDefId,
    signature_name: &str,
    signature: &crate::hir::HirFunctionSig,
) -> Result<(), String> {
    if signature.name != signature_name {
        return Err(format!(
            "Product artifact interface {owner_kind} {} signature '{}' has name '{}'",
            owner_id.local_id.0, signature_name, signature.name
        ));
    }

    Ok(())
}

fn validate_object_link_records(products: &CompilerProducts) -> Result<(), String> {
    if products.link.object_path.is_none() {
        return Ok(());
    }

    for (id, function) in &products.interface.functions {
        if product_function_requires_object_link_record(function)
            && !products.link.records.contains_key(id)
        {
            return Err(format!(
                "Product artifact object-backed function '{}' ID {}::{} is missing link record backend symbol",
                function.name, id.crate_id.0, id.local_id.0
            ));
        }
    }

    for (impl_id, impl_def) in &products.interface.impls {
        for (method_name, method) in &impl_def.methods {
            let method_id = ProductDefId::from(method.id);
            if product_impl_method_requires_object_link_record(impl_def, method)
                && !products.link.records.contains_key(&method_id)
            {
                return Err(format!(
                    "Product artifact object-backed impl {} method '{}' ID {}::{} is missing link record backend symbol",
                    impl_id.local_id.0, method_name, method_id.crate_id.0, method_id.local_id.0
                ));
            }
        }
    }

    Ok(())
}

fn backend_symbols_from_products(
    products: &CompilerProducts,
    remap: &ProductIdentityRemap,
) -> Result<BTreeMap<DefId, String>, String> {
    let mut backend_symbols = BTreeMap::new();

    for (id, record) in &products.link.records {
        validate_product_backend_symbol_id(products, *id, "link record")?;
        if record.backend_symbol.is_empty() {
            return Err(format!(
                "Product artifact link record for ID {}::{} has empty backend symbol",
                id.crate_id.0, id.local_id.0
            ));
        }
        backend_symbols.insert(remap.def_id(*id)?, record.backend_symbol.clone());
    }

    Ok(backend_symbols)
}

fn validate_product_backend_symbol_id(
    products: &CompilerProducts,
    id: ProductDefId,
    row_kind: &str,
) -> Result<(), String> {
    if product_id_has_interface_callable_binding(products, id) {
        return Ok(());
    }

    Err(format!(
        "Product artifact {row_kind} backend symbol for ID {}::{} has no interface callable declaration",
        id.crate_id.0, id.local_id.0
    ))
}

fn product_id_has_interface_callable_binding(
    products: &CompilerProducts,
    id: ProductDefId,
) -> bool {
    products.interface.functions.contains_key(&id)
        || products.interface.traits.values().any(|trait_def| {
            trait_def
                .methods
                .values()
                .any(|method| ProductDefId::from(method.id) == id)
        })
        || products.interface.impls.values().any(|impl_def| {
            impl_def
                .methods
                .values()
                .any(|method| ProductDefId::from(method.id) == id)
        })
}

fn product_function_requires_downstream_specialization(
    function: &ProductFunctionInterface,
) -> bool {
    product_function_interface_requires_reusable_body(function)
}

fn product_function_is_codegen_concrete(function: &ProductFunctionInterface) -> bool {
    function.params.iter().all(product_type_is_codegen_concrete)
        && product_type_is_codegen_concrete(&function.ret_type)
}

fn product_function_requires_object_link_record(function: &ProductFunctionInterface) -> bool {
    !product_function_requires_downstream_specialization(function)
        && product_function_is_codegen_concrete(function)
}

fn product_impl_method_requires_object_link_record(
    impl_def: &ProductImplInterface,
    method: &ProductFunctionInterface,
) -> bool {
    product_impl_link_shape_is_codegen_concrete(impl_def)
        && product_function_requires_object_link_record(method)
}

fn product_impl_link_shape_is_codegen_concrete(impl_def: &ProductImplInterface) -> bool {
    impl_def.type_generics.is_empty()
        && match &impl_def.receiver_pattern {
            crate::hir::HirImplReceiverPattern::Exact(ty)
            | crate::hir::HirImplReceiverPattern::Constructor(ty) => {
                product_type_is_codegen_concrete(ty)
            }
            crate::hir::HirImplReceiverPattern::SliceFamily { element } => {
                product_type_is_codegen_concrete(element)
            }
        }
        && impl_def
            .trait_arg_types
            .iter()
            .all(product_type_is_codegen_concrete)
        && impl_def
            .associated_types
            .iter()
            .all(|associated| product_type_is_codegen_concrete(&associated.ty))
}

fn product_type_is_codegen_concrete(ty: &Type) -> bool {
    crate::type_services::facts::TypeFacts::is_codegen_concrete(ty)
}

fn product_type_contains_forbidden_sentinel(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| {
        matches!(nested, Type::TypeVar(_) | Type::Error)
    })
}

fn normalize_product_prelude_exports(
    products: &CompilerProducts,
    remap: &ProductIdentityRemap,
    interface: &super::ArtifactCrateInterface,
    crate_name: &str,
    prelude_prefix: &str,
) -> Result<BTreeMap<String, ArtifactExport>, String> {
    let mut normalized_ids = BTreeMap::new();

    for (alias, id) in &products.identity_table.prelude_export_names {
        let display_name = product_display_name(products, *id).ok_or_else(|| {
            format!(
                "Product artifact prelude export '{}' has no display name or canonical DefId",
                alias
            )
        })?;
        let def_id =
            product_def_id_to_prelude_export_def_id(products, remap, *id)?.ok_or_else(|| {
                format!(
                    "Product artifact prelude export '{}' has no canonical prelude item ID",
                    alias
                )
            })?;
        let source = qualify_product_name(crate_name, &display_name);
        normalized_ids.insert(alias.clone(), ArtifactExport { source, id: def_id });
    }

    for name in product_interface_prelude_names(interface, prelude_prefix) {
        if let Some(alias) = name.strip_prefix(&format!("{}::", prelude_prefix)) {
            if !normalized_ids.contains_key(alias) {
                let export = artifact_export_for_source(interface, &name).ok_or_else(|| {
                    format!(
                        "Product artifact prelude export '{}' references '{}' without a canonical DefId",
                        alias, name
                    )
                })?;
                normalized_ids.insert(alias.to_string(), export);
            }
        }
    }

    Ok(normalized_ids)
}

fn artifact_export_for_source(
    interface: &super::ArtifactCrateInterface,
    source: &str,
) -> Option<ArtifactExport> {
    interface.export_for_source(source)
}

fn product_interface_prelude_names(
    interface: &super::ArtifactCrateInterface,
    prelude_prefix: &str,
) -> Vec<String> {
    interface.prelude_source_names(prelude_prefix)
}

fn resolve_product_object_path(
    products: &CompilerProducts,
    artifact_path: &Path,
) -> Result<PathBuf, String> {
    let Some(object_path) = products.link.object_path.clone() else {
        return Err(format!(
            "Product artifact {} does not record an object output path",
            artifact_path.display()
        ));
    };
    let candidate = if object_path.is_absolute() {
        object_path
    } else {
        resolve_relative_product_object_path(&object_path, artifact_path)?
    };

    if !candidate.exists() {
        return Err(format!(
            "Product artifact {} references missing object file {}",
            artifact_path.display(),
            candidate.display()
        ));
    }

    Ok(candidate.canonicalize().unwrap_or(candidate))
}

fn resolve_relative_product_object_path(
    object_path: &Path,
    artifact_path: &Path,
) -> Result<PathBuf, String> {
    let artifact_parent = artifact_path.parent().unwrap_or_else(|| Path::new("."));
    let artifact_relative = artifact_parent.join(object_path);
    if artifact_relative.exists() {
        return Ok(artifact_relative);
    }

    if let Some(parent) = artifact_parent.parent() {
        let sibling_relative = parent.join(object_path);
        if sibling_relative.exists() {
            return Ok(sibling_relative);
        }
    }

    if object_path.exists() {
        return Ok(object_path.to_path_buf());
    }

    Ok(artifact_relative)
}

fn remap_function_id(
    mut function: crate::hir::AcceptedHirFunction,
    id: ProductDefId,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
) -> Result<crate::hir::AcceptedHirFunction, String> {
    function.id = remap.def_id(id)?;
    remap_function_child_locations(&mut function, remap, validator, type_validator)?;
    Ok(function)
}

fn remap_function_interface_id(
    mut function: ProductFunctionInterface,
    id: ProductDefId,
    remap: &ProductIdentityRemap,
    type_validator: &ProductNominalTypeValidator,
) -> Result<ProductFunctionInterface, String> {
    function.id = remap.def_id(id)?;
    remap_generic_param_decls(&mut function.generic_params, remap)?;
    remap_generic_bounds(&mut function.generic_bounds, remap, type_validator)?;
    for param in &mut function.params {
        remap_type_def_ids(param, remap, type_validator)?;
    }
    remap_type_def_ids(&mut function.ret_type, remap, type_validator)?;
    Ok(function)
}

fn remap_struct_interface_id(
    mut strukt: ProductStructInterface,
    id: ProductDefId,
    remap: &ProductIdentityRemap,
    type_validator: &ProductNominalTypeValidator,
) -> Result<ProductStructInterface, String> {
    strukt.id = remap.def_id(id)?;
    remap_generic_param_decls(&mut strukt.generic_params, remap)?;
    for field in &mut strukt.fields {
        remap_type_def_ids(&mut field.ty, remap, type_validator)?;
    }
    Ok(strukt)
}

fn remap_type_alias_interface_id(
    mut alias: ProductTypeAliasInterface,
    id: ProductDefId,
    remap: &ProductIdentityRemap,
    type_validator: &ProductNominalTypeValidator,
) -> Result<ProductTypeAliasInterface, String> {
    alias.id = remap.def_id(id)?;
    remap_generic_param_decls(&mut alias.generic_params, remap)?;
    remap_type_def_ids(&mut alias.ty, remap, type_validator)?;
    Ok(alias)
}

fn remap_enum_interface_id(
    mut enm: ProductEnumInterface,
    id: ProductDefId,
    remap: &ProductIdentityRemap,
    type_validator: &ProductNominalTypeValidator,
) -> Result<ProductEnumInterface, String> {
    enm.id = remap.def_id(id)?;
    remap_generic_param_decls(&mut enm.generic_params, remap)?;
    for variant in &mut enm.variants {
        remap_variant_fields_def_ids(&mut variant.fields, remap, type_validator)?;
    }
    Ok(enm)
}

fn remap_variant_fields_def_ids(
    fields: &mut crate::hir::HirVariantFields,
    remap: &ProductIdentityRemap,
    type_validator: &ProductNominalTypeValidator,
) -> Result<(), String> {
    match fields {
        crate::hir::HirVariantFields::Named(fields) => {
            for field in fields {
                remap_type_def_ids(&mut field.ty, remap, type_validator)?;
            }
        }
        crate::hir::HirVariantFields::Positional(types) => {
            for ty in types {
                remap_type_def_ids(ty, remap, type_validator)?;
            }
        }
        crate::hir::HirVariantFields::Unit => {}
    }
    Ok(())
}

fn remap_trait_interface_ids(
    mut trait_def: ProductTraitInterface,
    id: ProductDefId,
    remap: &ProductIdentityRemap,
    type_validator: &ProductNominalTypeValidator,
) -> Result<ProductTraitInterface, String> {
    trait_def.id = remap.def_id(id)?;
    remap_generic_param_decls(&mut trait_def.generic_params, remap)?;
    if let Some(target) = &mut trait_def.target {
        remap_generic_param_decls(std::slice::from_mut(target), remap)?;
    }
    remap_predicates(&mut trait_def.predicates, remap, type_validator)?;
    for method in trait_def.methods.values_mut() {
        let method_id = ProductDefId::from(method.id);
        *method = remap_function_interface_id(method.clone(), method_id, remap, type_validator)?;
    }
    for sig in trait_def.signatures.values_mut() {
        sig.id = remap.def_id(ProductDefId::from(sig.id))?;
        remap_generic_param_decls(&mut sig.generic_params, remap)?;
        remap_generic_bounds(&mut sig.generic_bounds, remap, type_validator)?;
        for param in &mut sig.params {
            remap_type_def_ids(param, remap, type_validator)?;
        }
        remap_type_def_ids(&mut sig.ret, remap, type_validator)?;
    }
    Ok(trait_def)
}

fn remap_impl_interface_ids(
    mut imp: ProductImplInterface,
    id: ProductDefId,
    remap: &ProductIdentityRemap,
    type_validator: &ProductNominalTypeValidator,
) -> Result<ProductImplInterface, String> {
    imp.id = remap.def_id(id)?;
    remap_generic_param_decls(&mut imp.type_generics, remap)?;
    remap_generic_param_decls(&mut imp.trait_generics, remap)?;
    if let Some(trait_id) = &mut imp.trait_id {
        *trait_id = remap.def_id(ProductDefId::from(*trait_id))?;
    }
    match &mut imp.receiver_pattern {
        crate::hir::HirImplReceiverPattern::Exact(ty)
        | crate::hir::HirImplReceiverPattern::Constructor(ty) => {
            remap_type_def_ids(ty, remap, type_validator)?;
        }
        crate::hir::HirImplReceiverPattern::SliceFamily { element } => {
            remap_type_def_ids(element, remap, type_validator)?;
        }
    }
    for ty in &mut imp.trait_arg_types {
        remap_type_def_ids(ty, remap, type_validator)?;
    }
    for associated_type in &mut imp.associated_types {
        remap_type_def_ids(&mut associated_type.ty, remap, type_validator)?;
    }
    remap_generic_bounds(&mut imp.bounds, remap, type_validator)?;
    for method in imp.methods.values_mut() {
        let method_id = ProductDefId::from(method.id);
        *method = remap_function_interface_id(method.clone(), method_id, remap, type_validator)?;
    }
    Ok(imp)
}

fn remap_extern_interface_id(
    mut ext: ProductExternInterface,
    id: ProductDefId,
    remap: &ProductIdentityRemap,
    type_validator: &ProductNominalTypeValidator,
) -> Result<ProductExternInterface, String> {
    ext.id = remap.def_id(id)?;
    for param in &mut ext.params {
        remap_type_def_ids(param, remap, type_validator)?;
    }
    remap_type_def_ids(&mut ext.ret, remap, type_validator)?;
    Ok(ext)
}

struct ProductChildLocationValidator {
    fields_by_id: HashMap<(ProductDefId, crate::ids::FieldId), String>,
    variants_by_id: HashMap<(ProductDefId, crate::ids::VariantId), String>,
}

const MAX_ARTIFACT_HIR_REMAP_DEPTH: usize = 4096;

impl ProductChildLocationValidator {
    fn from_products(products: &CompilerProducts) -> Self {
        let mut fields_by_id = HashMap::new();
        let mut variants_by_id = HashMap::new();

        for (id, struct_def) in &products.interface.structs {
            for field in &struct_def.fields {
                fields_by_id.insert((*id, field.id), field.name.clone());
            }
        }

        for (id, enum_def) in &products.interface.enums {
            for variant in &enum_def.variants {
                variants_by_id.insert((*id, variant.id), variant.name.clone());
                if let crate::hir::HirVariantFields::Named(fields) = &variant.fields {
                    for field in fields {
                        fields_by_id.insert((*id, field.id), field.name.clone());
                    }
                }
            }
        }

        Self {
            fields_by_id,
            variants_by_id,
        }
    }

    fn from_products_with_context(
        products: &CompilerProducts,
        remap: &ProductIdentityRemap,
        ctx: &CrateContext,
    ) -> Self {
        let mut validator = Self::from_products(products);
        let consumer_to_product_crate: BTreeMap<CrateId, ProductCrateId> = remap
            .crate_ids
            .iter()
            .filter_map(|(product_crate, consumer_crate)| {
                (*product_crate != remap.local_crate).then_some((*consumer_crate, *product_crate))
            })
            .collect();

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

            for struct_def in interface.structs.values() {
                let Some(owner) = product_id_for_def(struct_def.id) else {
                    continue;
                };
                for field in &struct_def.fields {
                    validator
                        .fields_by_id
                        .insert((owner, field.id), field.name.clone());
                }
            }

            for enum_def in interface.enums.values() {
                let Some(owner) = product_id_for_def(enum_def.id) else {
                    continue;
                };
                for variant in &enum_def.variants {
                    validator
                        .variants_by_id
                        .insert((owner, variant.id), variant.name.clone());
                    if let crate::hir::HirVariantFields::Named(fields) = &variant.fields {
                        for field in fields {
                            validator
                                .fields_by_id
                                .insert((owner, field.id), field.name.clone());
                        }
                    }
                }
            }
        }

        validator
    }

    fn validate_field(
        &self,
        owner: ProductDefId,
        field_id: crate::ids::FieldId,
        location_name: &str,
        expr_name: &str,
    ) -> Result<(), String> {
        let Some(metadata_name) = self.fields_by_id.get(&(owner, field_id)) else {
            return Err(format!(
                "Product artifact references unknown field ID {} on definition {}",
                field_id.raw(),
                owner.local_id.0
            ));
        };

        if metadata_name != location_name || metadata_name != expr_name {
            return Err(format!(
                "Product artifact field sidecar mismatch for definition {} field {}: expression '{}', sidecar '{}', metadata '{}'",
                owner.local_id.0,
                field_id.raw(),
                expr_name,
                location_name,
                metadata_name
            ));
        }

        Ok(())
    }

    fn validate_variant(
        &self,
        owner: ProductDefId,
        variant_id: crate::ids::VariantId,
        location_name: &str,
        expr_name: &str,
    ) -> Result<(), String> {
        let Some(metadata_name) = self.variants_by_id.get(&(owner, variant_id)) else {
            return Err(format!(
                "Product artifact references unknown variant ID {} on definition {}",
                variant_id.raw(),
                owner.local_id.0
            ));
        };

        if metadata_name != location_name || metadata_name != expr_name {
            return Err(format!(
                "Product artifact variant sidecar mismatch for definition {} variant {}: expression '{}', sidecar '{}', metadata '{}'",
                owner.local_id.0,
                variant_id.raw(),
                expr_name,
                location_name,
                metadata_name
            ));
        }

        Ok(())
    }
}

fn remap_function_child_locations<P: crate::hir::HirPhase>(
    function: &mut crate::hir::HirFunctionFor<P>,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
) -> Result<(), String> {
    let mut local_scope = BTreeSet::new();
    for param in &function.params {
        local_scope.insert(param.local_id);
    }

    remap_generic_param_decls(&mut function.generic_params, remap)?;
    remap_generic_bounds(&mut function.generic_bounds, remap, type_validator)?;
    for param in &mut function.params {
        remap_type_def_ids(&mut param.ty, remap, type_validator)?;
    }
    remap_type_def_ids(&mut function.ret_type, remap, type_validator)?;
    remap_block_child_locations(
        &mut function.body,
        remap,
        validator,
        type_validator,
        &mut local_scope,
        0,
    )
}

fn add_pattern_local_ids(
    pattern: &crate::hir::HirPattern,
    local_scope: &mut BTreeSet<crate::ids::HirLocalId>,
) {
    match pattern {
        crate::hir::HirPattern::Binding { local_id, .. } => {
            local_scope.insert(*local_id);
        }
        crate::hir::HirPattern::Tuple(patterns)
        | crate::hir::HirPattern::Enum(_, _, _, patterns)
        | crate::hir::HirPattern::Or(patterns) => {
            for pattern in patterns {
                add_pattern_local_ids(pattern, local_scope);
            }
        }
        crate::hir::HirPattern::Struct(_, _, _, fields) => {
            for field in fields {
                add_pattern_local_ids(&field.pattern, local_scope);
            }
        }
        crate::hir::HirPattern::Wildcard | crate::hir::HirPattern::Literal(_) => {}
    }
}

fn remap_generic_bounds(
    bounds: &mut crate::hir::HirGenericBounds,
    remap: &ProductIdentityRemap,
    type_validator: &ProductNominalTypeValidator,
) -> Result<(), String> {
    let mut original = std::mem::take(bounds);
    let predicates = std::mem::take(&mut original.predicates);
    for (mut generic_param, mut trait_bounds) in original {
        generic_param.owner = remap.def_id(ProductDefId::from(generic_param.owner))?;
        for bound in &mut trait_bounds {
            bound.trait_id = remap.def_id(ProductDefId::from(bound.trait_id))?;
            for ty in &mut bound.type_args {
                remap_type_def_ids(ty, remap, type_validator)?;
            }
        }
        bounds.insert(generic_param, trait_bounds);
    }
    bounds.predicates = predicates;
    remap_predicates(&mut bounds.predicates, remap, type_validator)?;
    Ok(())
}

fn remap_predicates(
    predicates: &mut [crate::types::Predicate],
    remap: &ProductIdentityRemap,
    type_validator: &ProductNominalTypeValidator,
) -> Result<(), String> {
    for predicate in predicates {
        match predicate {
            crate::types::Predicate::Trait {
                subject,
                trait_id,
                args,
            } => {
                remap_type_def_ids(subject, remap, type_validator)?;
                *trait_id = remap.def_id(ProductDefId::from(*trait_id))?;
                for arg in args {
                    remap_type_def_ids(arg, remap, type_validator)?;
                }
            }
        }
    }
    Ok(())
}

fn remap_generic_param_decls(
    decls: &mut [crate::types::GenericParamDecl],
    remap: &ProductIdentityRemap,
) -> Result<(), String> {
    for decl in decls {
        decl.id.owner = remap.def_id(ProductDefId::from(decl.id.owner))?;
    }
    Ok(())
}

fn remap_type_def_ids(
    ty: &mut crate::types::Type,
    remap: &ProductIdentityRemap,
    validator: &ProductNominalTypeValidator,
) -> Result<(), String> {
    struct ProductTypeRemapper<'a> {
        remap: &'a ProductIdentityRemap,
        validator: &'a ProductNominalTypeValidator,
    }

    impl crate::type_services::visit::TryTypeFolder for ProductTypeRemapper<'_> {
        type Error = String;

        fn try_fold_type(
            &mut self,
            mut ty: crate::types::Type,
        ) -> Result<crate::types::Type, Self::Error> {
            match &mut ty {
                crate::types::Type::Struct { id, .. } => {
                    let product_id = ProductDefId::from(*id);
                    self.validator.validate_struct(product_id)?;
                    *id = self.remap.def_id(product_id)?;
                }
                crate::types::Type::Enum { id, .. } => {
                    let product_id = ProductDefId::from(*id);
                    self.validator.validate_enum(product_id)?;
                    *id = self.remap.def_id(product_id)?;
                }
                crate::types::Type::Constructor { id, flavor } => {
                    let product_id = ProductDefId::from(*id);
                    match flavor {
                        crate::types::NominalTypeKind::Struct => {
                            self.validator.validate_struct(product_id)?
                        }
                        crate::types::NominalTypeKind::Enum => {
                            self.validator.validate_enum(product_id)?
                        }
                        crate::types::NominalTypeKind::Alias => {
                            self.validator.validate_type_alias(product_id)?
                        }
                    }
                    *id = self.remap.def_id(product_id)?;
                }
                crate::types::Type::Projection {
                    trait_id,
                    assoc_type,
                    ..
                } => {
                    let product_trait_id = ProductDefId::from(*trait_id);
                    let product_assoc_owner = ProductDefId::from(assoc_type.owner);
                    self.validator.validate_projection(
                        product_trait_id,
                        product_assoc_owner,
                        assoc_type.assoc_type_id,
                    )?;
                    *trait_id = self.remap.def_id(product_trait_id)?;
                    assoc_type.owner = self.remap.def_id(product_assoc_owner)?;
                }
                crate::types::Type::TypeVar(_) => {
                    return Err(format!(
                        "Product artifact contains unresolved type Type::TypeVar in executable metadata: {ty:?}"
                    ));
                }
                crate::types::Type::Error => {
                    return Err(format!(
                        "Product artifact contains unresolved type Type::Error in executable metadata: {ty:?}"
                    ));
                }
                crate::types::Type::Generic(param) => {
                    let product_id = ProductDefId::from(param.owner);
                    param.owner = self.remap.def_id(product_id)?;
                }
                _ => {}
            }

            crate::type_services::visit::try_fold_type_children(ty, self)
        }
    }

    let remapped = crate::type_services::visit::try_fold_type(
        ty.clone(),
        &mut ProductTypeRemapper { remap, validator },
    )?;
    *ty = remapped;
    Ok(())
}

fn remap_block_child_locations<P: crate::hir::HirPhase>(
    block: &mut crate::hir::HirBlockFor<P>,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
    local_scope: &mut BTreeSet<crate::ids::HirLocalId>,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_ARTIFACT_HIR_REMAP_DEPTH {
        return Err("Product artifact HIR body exceeds maximum remap depth".to_string());
    }

    for stmt in &mut block.stmts {
        remap_stmt_child_locations(stmt, remap, validator, type_validator, local_scope, depth)?;
    }
    remap_type_def_ids(&mut block.ty, remap, type_validator)?;
    Ok(())
}

fn remap_stmt_child_locations<P: crate::hir::HirPhase>(
    stmt: &mut crate::hir::HirStmtFor<P>,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
    local_scope: &mut BTreeSet<crate::ids::HirLocalId>,
    depth: usize,
) -> Result<(), String> {
    match stmt {
        crate::hir::HirStmtFor::Let {
            local_id,
            ty,
            value,
            ..
        } => {
            remap_type_def_ids(ty, remap, type_validator)?;
            remap_expr_child_locations_with_scope(
                value,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            local_scope.insert(*local_id);
            Ok(())
        }
        crate::hir::HirStmtFor::Expr(value) => remap_expr_child_locations_with_scope(
            value,
            remap,
            validator,
            type_validator,
            local_scope,
            depth + 1,
        ),
        crate::hir::HirStmtFor::Return(Some(value))
        | crate::hir::HirStmtFor::Break(Some(value)) => remap_expr_child_locations_with_scope(
            value,
            remap,
            validator,
            type_validator,
            local_scope,
            depth + 1,
        ),
        crate::hir::HirStmtFor::Return(None)
        | crate::hir::HirStmtFor::Break(None)
        | crate::hir::HirStmtFor::Continue => Ok(()),
    }
}

#[cfg(test)]
fn remap_expr_child_locations<P: crate::hir::HirPhase>(
    expr: &mut crate::hir::HirExprFor<P>,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
    depth: usize,
) -> Result<(), String> {
    let mut local_scope = BTreeSet::new();
    remap_expr_child_locations_with_scope(
        expr,
        remap,
        validator,
        type_validator,
        &mut local_scope,
        depth,
    )
}

fn remap_expr_child_locations_with_scope<P: crate::hir::HirPhase>(
    expr: &mut crate::hir::HirExprFor<P>,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
    local_scope: &mut BTreeSet<crate::ids::HirLocalId>,
    depth: usize,
) -> Result<(), String> {
    use crate::hir::HirExprKindFor;

    if depth > MAX_ARTIFACT_HIR_REMAP_DEPTH {
        return Err("Product artifact HIR body exceeds maximum remap depth".to_string());
    }

    remap_type_def_ids(&mut expr.ty, remap, type_validator)?;

    match &mut expr.kind {
        HirExprKindFor::IntLiteral(_)
        | HirExprKindFor::FloatLiteral(_)
        | HirExprKindFor::BoolLiteral(_)
        | HirExprKindFor::StringLiteral(_)
        | HirExprKindFor::CharLiteral(_)
        | HirExprKindFor::Unit
        | HirExprKindFor::Var(_) => Ok(()),
        HirExprKindFor::ResolvedVar(reference) => {
            remap_var_target_child_location(reference, remap, type_validator, local_scope)
        }
        HirExprKindFor::ArrayLiteral(values) | HirExprKindFor::TupleLiteral(values) => {
            for value in values {
                remap_expr_child_locations_with_scope(
                    value,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::ArrayRepeat(value, _) => remap_expr_child_locations_with_scope(
            value,
            remap,
            validator,
            type_validator,
            local_scope,
            depth + 1,
        ),
        HirExprKindFor::FieldAccess(base, field_name, location) => {
            remap_expr_child_locations_with_scope(
                base,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            if let Some(location) = location {
                let product_owner = ProductDefId::from(location.owner);
                validator.validate_field(
                    product_owner,
                    location.field_id,
                    &location.name,
                    field_name,
                )?;
                location.owner = remap.def_id(product_owner)?;
            }
            Ok(())
        }
        HirExprKindFor::TupleIndex(base, _)
        | HirExprKindFor::UnaryOp(_, base)
        | HirExprKindFor::Ref(_, base)
        | HirExprKindFor::Deref(base) => remap_expr_child_locations_with_scope(
            base,
            remap,
            validator,
            type_validator,
            local_scope,
            depth + 1,
        ),
        HirExprKindFor::BinOp(_, base, index) | HirExprKindFor::Assign(base, index) => {
            remap_expr_child_locations_with_scope(
                base,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            remap_expr_child_locations_with_scope(
                index,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )
        }
        HirExprKindFor::Call(callee, args, target) => {
            let authorized_static_callee = matches!(
                (&callee.kind, target.as_ref()),
                (
                    HirExprKindFor::ResolvedVar(crate::hir::HirVarRef {
                        target: crate::hir::HirVarTarget::Function(callee_id),
                        ..
                    }),
                    Some(crate::hir::HirCallTarget::StaticMethod(target))
                ) if target.method.method_id() == Some(*callee_id)
            );
            if let Some(target) = target {
                remap_call_target_child_location(target, remap, type_validator, local_scope)?;
            }
            if authorized_static_callee {
                let Some(crate::hir::HirCallTarget::StaticMethod(target)) = target.as_ref() else {
                    unreachable!("authorized static callee checked above")
                };
                let method_id = target.method.method_id().ok_or_else(|| {
                    "Product artifact static authority has no selected method identity".to_string()
                })?;
                remap_type_def_ids(&mut callee.ty, remap, type_validator)?;
                let HirExprKindFor::ResolvedVar(reference) = &mut callee.kind else {
                    unreachable!("authorized static callee checked above")
                };
                reference.target = crate::hir::HirVarTarget::Function(method_id);
            } else {
                remap_expr_child_locations_with_scope(
                    callee,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            for arg in args {
                remap_expr_child_locations_with_scope(
                    arg,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::MethodCall(receiver, _, args, _, target) => {
            if let Some(target) = P::method_authority_mut(target) {
                type_validator.validate_method_call_target(target)?;
                remap_method_target_child_location(target, remap, type_validator)?;
            }
            remap_expr_child_locations_with_scope(
                receiver,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            for arg in args {
                remap_expr_child_locations_with_scope(
                    arg,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::Try {
            expr,
            branch_method,
            branch_target,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            control_flow_enum,
            break_variant,
            continue_variant,
            ..
        } => {
            remap_type_def_ids(output_ty, remap, type_validator)?;
            remap_type_def_ids(residual_ty, remap, type_validator)?;
            remap_type_def_ids(return_ty, remap, type_validator)?;

            if let Some(target) = P::method_authority_mut(branch_method) {
                type_validator.validate_method_call_target(target)?;
                remap_method_target_child_location(target, remap, type_validator)?;
            }
            if branch_target.is_some() {
                return Err(
                    "Product artifact contains a local Try branch instance target".to_string(),
                );
            }

            if let Some(target) = P::residual_authority_mut(from_residual_target) {
                remap_call_target_child_location(target, remap, type_validator, local_scope)?;
            }

            let product_control_flow = ProductDefId::from(*control_flow_enum);
            type_validator.validate_enum(product_control_flow)?;
            *control_flow_enum = remap.def_id(product_control_flow)?;
            for location in [break_variant, continue_variant] {
                let product_owner = ProductDefId::from(location.owner);
                validator.validate_variant(
                    product_owner,
                    location.variant_id,
                    &location.name,
                    &location.name,
                )?;
                location.owner = remap.def_id(product_owner)?;
            }

            remap_expr_child_locations_with_scope(
                expr,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )
        }
        HirExprKindFor::StructLiteral(_, struct_id, fields) => {
            let product_struct_id = struct_id.map(ProductDefId::from);
            if let Some(struct_id) = struct_id {
                let product_id = ProductDefId::from(*struct_id);
                type_validator.validate_struct(product_id)?;
                *struct_id = remap.def_id(product_id)?;
            }
            for field in fields {
                if let Some(location) = &mut field.field {
                    let product_owner = ProductDefId::from(location.owner);
                    if let Some(product_struct_id) = product_struct_id {
                        if product_owner != product_struct_id {
                            return Err(format!(
                                "Product artifact struct literal field owner mismatch: literal owner {} but field '{}' owner {}",
                                product_struct_id.local_id.0,
                                field.name,
                                product_owner.local_id.0
                            ));
                        }
                    }
                    validator.validate_field(
                        product_owner,
                        location.field_id,
                        &location.name,
                        &field.name,
                    )?;
                    location.owner = remap.def_id(product_owner)?;
                }
                remap_expr_child_locations_with_scope(
                    &mut field.value,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::EnumVariant(_, variant_name, args, location) => {
            if let Some(location) = location {
                let product_owner = ProductDefId::from(location.owner);
                validator.validate_variant(
                    product_owner,
                    location.variant_id,
                    &location.name,
                    variant_name,
                )?;
                location.owner = remap.def_id(product_owner)?;
            }
            for arg in args {
                remap_expr_child_locations_with_scope(
                    arg,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            remap_expr_child_locations_with_scope(
                condition,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            let mut then_scope = local_scope.clone();
            remap_block_child_locations(
                then_branch,
                remap,
                validator,
                type_validator,
                &mut then_scope,
                depth + 1,
            )?;
            if let Some(else_branch) = else_branch {
                let mut else_scope = local_scope.clone();
                remap_block_child_locations(
                    else_branch,
                    remap,
                    validator,
                    type_validator,
                    &mut else_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::Match { scrutinee, arms } => {
            remap_expr_child_locations_with_scope(
                scrutinee,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            for arm in arms {
                let mut arm_scope = local_scope.clone();
                remap_pattern_child_locations(
                    &mut arm.pattern,
                    remap,
                    validator,
                    type_validator,
                    depth + 1,
                )?;
                add_pattern_local_ids(&arm.pattern, &mut arm_scope);
                if let Some(guard) = &mut arm.guard {
                    remap_expr_child_locations_with_scope(
                        guard,
                        remap,
                        validator,
                        type_validator,
                        &mut arm_scope,
                        depth + 1,
                    )?;
                }
                remap_block_child_locations(
                    &mut arm.body,
                    remap,
                    validator,
                    type_validator,
                    &mut arm_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::While { condition, body } => {
            remap_expr_child_locations_with_scope(
                condition,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            let mut body_scope = local_scope.clone();
            remap_block_child_locations(
                body,
                remap,
                validator,
                type_validator,
                &mut body_scope,
                depth + 1,
            )
        }
        HirExprKindFor::For {
            local_id,
            iter,
            body,
            ..
        } => {
            remap_expr_child_locations_with_scope(
                iter,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            let mut body_scope = local_scope.clone();
            body_scope.insert(*local_id);
            remap_block_child_locations(
                body,
                remap,
                validator,
                type_validator,
                &mut body_scope,
                depth + 1,
            )
        }
        HirExprKindFor::Loop(body)
        | HirExprKindFor::Block(body)
        | HirExprKindFor::UnsafeBlock(body) => {
            let mut body_scope = local_scope.clone();
            remap_block_child_locations(
                body,
                remap,
                validator,
                type_validator,
                &mut body_scope,
                depth + 1,
            )
        }
        HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => {
            let mut lambda_scope = BTreeSet::new();
            for param in params {
                remap_type_def_ids(&mut param.ty, remap, type_validator)?;
                lambda_scope.insert(param.local_id);
            }
            for capture in captures {
                if !local_scope.contains(&capture.local_id) {
                    return Err(format!(
                        "Product artifact references unknown local capture {:?}",
                        capture.local_id
                    ));
                }
                remap_type_def_ids(&mut capture.ty, remap, type_validator)?;
                lambda_scope.insert(capture.local_id);
            }
            remap_block_child_locations(
                body,
                remap,
                validator,
                type_validator,
                &mut lambda_scope,
                depth + 1,
            )
        }
        HirExprKindFor::Cast(value, ty) => {
            remap_type_def_ids(ty, remap, type_validator)?;
            remap_expr_child_locations_with_scope(
                value,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )
        }
        HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                remap_expr_child_locations_with_scope(
                    arg,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
    }
}

fn remap_var_target_child_location(
    reference: &mut crate::hir::HirVarRef,
    remap: &ProductIdentityRemap,
    validator: &ProductNominalTypeValidator,
    local_scope: &BTreeSet<crate::ids::HirLocalId>,
) -> Result<(), String> {
    match &mut reference.target {
        crate::hir::HirVarTarget::Function(id) => {
            let product_id = ProductDefId::from(*id);
            validator.validate_function_target(product_id, &reference.name)?;
            *id = remap.def_id(product_id)?;
        }
        crate::hir::HirVarTarget::Extern(id) => {
            let product_id = ProductDefId::from(*id);
            validator.validate_extern_target(product_id, &reference.name)?;
            *id = remap.def_id(product_id)?;
        }
        crate::hir::HirVarTarget::Instance(id) => {
            return Err(format!(
                "Product artifact contains local instance target {:?} for '{}'",
                id, reference.name
            ));
        }
        crate::hir::HirVarTarget::Local(id) => {
            if !local_scope.contains(id) {
                return Err(format!(
                    "Product artifact references unknown local var target {:?}",
                    id
                ));
            }
        }
    }
    Ok(())
}

fn remap_call_target_child_location(
    target: &mut crate::hir::HirCallTarget,
    remap: &ProductIdentityRemap,
    validator: &ProductNominalTypeValidator,
    local_ids: &BTreeSet<crate::ids::HirLocalId>,
) -> Result<(), String> {
    match target {
        crate::hir::HirCallTarget::Function(id) => {
            let product_id = ProductDefId::from(*id);
            validator.validate_function_call_target(product_id)?;
            *id = remap.def_id(product_id)?;
        }
        crate::hir::HirCallTarget::Extern(id) => {
            let product_id = ProductDefId::from(*id);
            validator.validate_extern_call_target(product_id)?;
            *id = remap.def_id(product_id)?;
        }
        crate::hir::HirCallTarget::Local(id) => {
            if !local_ids.contains(id) {
                return Err(format!(
                    "Product artifact references unknown local call target {:?}",
                    id
                ));
            }
        }
        crate::hir::HirCallTarget::StaticMethod(static_target) => {
            remap_type_def_ids(&mut static_target.owner_ty, remap, validator)?;
            validator.validate_static_method_target(&static_target.method)?;
            remap_method_target_child_location(&mut static_target.method, remap, validator)?;
        }
        crate::hir::HirCallTarget::Instance(id) => {
            return Err(format!(
                "Product artifact contains local instance call target {id:?}"
            ));
        }
        crate::hir::HirCallTarget::Intrinsic(_) => {}
    }

    Ok(())
}

fn remap_method_target_child_location(
    target: &mut HirMethodCallTarget,
    remap: &ProductIdentityRemap,
    validator: &ProductNominalTypeValidator,
) -> Result<(), String> {
    validator.validate_method_target(target)?;

    let mut id_error = None;
    target.for_each_def_id_mut(|id| {
        if id_error.is_none() {
            id_error = remap
                .def_id(ProductDefId::from(*id))
                .map(|mapped| *id = mapped)
                .err();
        }
    });
    if let Some(error) = id_error {
        return Err(error);
    }

    let mut type_error = None;
    target.for_each_type_mut(|ty| {
        if type_error.is_none() {
            type_error = remap_type_def_ids(ty, remap, validator).err();
        }
    });
    if let Some(error) = type_error {
        return Err(error);
    }

    Ok(())
}

fn remap_pattern_child_locations(
    pattern: &mut crate::hir::HirPattern,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_ARTIFACT_HIR_REMAP_DEPTH {
        return Err("Product artifact HIR body exceeds maximum remap depth".to_string());
    }

    match pattern {
        crate::hir::HirPattern::Struct(_, struct_id, type_args, field_patterns) => {
            let product_struct_id = struct_id.map(ProductDefId::from);
            if let Some(struct_id) = struct_id {
                let product_id = ProductDefId::from(*struct_id);
                type_validator.validate_struct(product_id)?;
                *struct_id = remap.def_id(product_id)?;
            }
            for type_arg in type_args {
                remap_type_def_ids(type_arg, remap, type_validator)?;
            }
            for field in field_patterns {
                if let Some(location) = &mut field.field {
                    let product_owner = ProductDefId::from(location.owner);
                    if let Some(product_struct_id) = product_struct_id {
                        if product_owner != product_struct_id {
                            return Err(format!(
                                "Product artifact struct pattern field owner mismatch: pattern owner {} but field '{}' owner {}",
                                product_struct_id.local_id.0,
                                field.name,
                                product_owner.local_id.0
                            ));
                        }
                    }
                    validator.validate_field(
                        product_owner,
                        location.field_id,
                        &location.name,
                        &field.name,
                    )?;
                    location.owner = remap.def_id(product_owner)?;
                }
                remap_pattern_child_locations(
                    &mut field.pattern,
                    remap,
                    validator,
                    type_validator,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        crate::hir::HirPattern::Enum(_, variant_name, location, sub_patterns) => {
            if let Some(location) = location {
                let product_owner = ProductDefId::from(location.owner);
                validator.validate_variant(
                    product_owner,
                    location.variant_id,
                    &location.name,
                    variant_name,
                )?;
                location.owner = remap.def_id(product_owner)?;
            }
            for pattern in sub_patterns {
                remap_pattern_child_locations(
                    pattern,
                    remap,
                    validator,
                    type_validator,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        crate::hir::HirPattern::Tuple(sub_patterns) | crate::hir::HirPattern::Or(sub_patterns) => {
            for pattern in sub_patterns {
                remap_pattern_child_locations(
                    pattern,
                    remap,
                    validator,
                    type_validator,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        crate::hir::HirPattern::Wildcard
        | crate::hir::HirPattern::Binding { .. }
        | crate::hir::HirPattern::Literal(_) => Ok(()),
    }
}

fn remap_trait_ids(
    mut value: crate::hir::AcceptedHirTrait,
    id: ProductDefId,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
) -> Result<crate::hir::AcceptedHirTrait, String> {
    value.id = remap.def_id(id)?;
    remap_generic_param_decls(&mut value.generic_params, remap)?;
    if let Some(target) = &mut value.target {
        remap_generic_param_decls(std::slice::from_mut(target), remap)?;
    }
    remap_predicates(&mut value.predicates, remap, type_validator)?;
    for signature in value.signatures.values_mut() {
        signature.id = remap.def_id(ProductDefId::from(signature.id))?;
        remap_generic_param_decls(&mut signature.generic_params, remap)?;
        remap_generic_bounds(&mut signature.generic_bounds, remap, type_validator)?;
        for param in &mut signature.params {
            remap_type_def_ids(param, remap, type_validator)?;
        }
        remap_type_def_ids(&mut signature.ret, remap, type_validator)?;
    }
    for method in value.methods.values_mut() {
        method.id = remap.def_id(ProductDefId::from(method.id))?;
        remap_function_child_locations(method, remap, validator, type_validator)?;
    }
    Ok(value)
}

fn remap_impl_ids(
    mut value: crate::hir::AcceptedHirImpl,
    id: ProductDefId,
    crate_name: &str,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
) -> Result<crate::hir::AcceptedHirImpl, String> {
    value.id = remap.def_id(id)?;
    remap_generic_param_decls(&mut value.type_generics, remap)?;
    remap_generic_param_decls(&mut value.trait_generics, remap)?;
    if let crate::hir::HirImplOwner::Named(owner) = &mut value.owner {
        *owner = qualify_product_name(crate_name, owner);
    }
    if let Some(trait_id) = value.trait_id.as_mut() {
        *trait_id = remap.def_id(ProductDefId::from(*trait_id))?;
    }
    match &mut value.receiver_pattern {
        crate::hir::HirImplReceiverPattern::Exact(ty)
        | crate::hir::HirImplReceiverPattern::Constructor(ty) => {
            remap_type_def_ids(ty, remap, type_validator)?;
        }
        crate::hir::HirImplReceiverPattern::SliceFamily { element } => {
            remap_type_def_ids(element, remap, type_validator)?;
        }
    }
    for ty in &mut value.trait_arg_types {
        remap_type_def_ids(ty, remap, type_validator)?;
    }
    for assoc in &mut value.associated_types {
        remap_type_def_ids(&mut assoc.ty, remap, type_validator)?;
    }
    remap_generic_bounds(&mut value.bounds, remap, type_validator)?;
    for method in value.methods.values_mut() {
        method.id = remap.def_id(ProductDefId::from(method.id))?;
        remap_function_child_locations(method, remap, validator, type_validator)?;
    }
    Ok(value)
}

fn interface_from_products(
    products: &CompilerProducts,
    crate_name: &str,
    remap: &ProductIdentityRemap,
    ctx: &CrateContext,
    language_items: &crate::hir::HirLanguageItems,
) -> Result<super::ArtifactCrateInterface, String> {
    let type_validator =
        ProductNominalTypeValidator::from_products_with_context(products, remap, ctx);
    let mut canonical_names = BTreeMap::new();
    for (id, name) in &products.identity_table.display_names {
        if let Some(def_id) = product_def_id_to_existing_def_id(products, remap, *id)? {
            canonical_names.insert(def_id, qualify_product_name(crate_name, name));
        }
    }

    let mut root_export_ids = BTreeMap::new();
    for (alias, id) in &products.identity_table.export_names {
        let display_name = product_display_name(products, *id).ok_or_else(|| {
            format!(
                "Product artifact root export '{}' has no display name or canonical DefId",
                alias
            )
        })?;
        let def_id = product_def_id_to_existing_def_id(products, remap, *id)?.ok_or_else(|| {
            format!(
                "Product artifact root export '{}' has no canonical DefId",
                alias
            )
        })?;
        let source = qualify_product_name(crate_name, &display_name);
        root_export_ids.insert(alias.clone(), ArtifactExport { source, id: def_id });
    }

    let functions = products
        .interface
        .functions
        .iter()
        .map(|(id, function)| (*id, function.clone()))
        .map(|(id, function)| {
            remap_function_interface_id(function, id, remap, &type_validator)
                .map(|function| (function.id, function))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    let structs = products
        .interface
        .structs
        .iter()
        .map(|(id, value)| (*id, value.clone()))
        .map(|(id, value)| {
            remap_struct_interface_id(value, id, remap, &type_validator)
                .map(|value| (value.id, value))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    let enums = products
        .interface
        .enums
        .iter()
        .map(|(id, value)| (*id, value.clone()))
        .map(|(id, value)| {
            remap_enum_interface_id(value, id, remap, &type_validator)
                .map(|value| (value.id, value))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    let type_aliases = products
        .interface
        .type_aliases
        .iter()
        .map(|(id, value)| (*id, value.clone()))
        .map(|(id, value)| {
            remap_type_alias_interface_id(value, id, remap, &type_validator)
                .map(|value| (value.id, value))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    let traits = products
        .interface
        .traits
        .iter()
        .map(|(id, value)| (*id, value.clone()))
        .map(|(id, value)| {
            remap_trait_interface_ids(value, id, remap, &type_validator)
                .map(|value| (value.id, value))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let supertrait_errors = crate::types::validate_supertrait_graph(
        traits
            .values()
            .map(|trait_def| {
                (
                    trait_def.id,
                    trait_def.name.as_str(),
                    trait_def.predicates.as_slice(),
                )
            })
            .chain(ctx.extern_crates().flat_map(|dependency| {
                dependency
                    .metadata()
                    .interface()
                    .traits
                    .values()
                    .map(|trait_def| {
                        (
                            trait_def.id,
                            trait_def.name.as_str(),
                            trait_def.predicates.as_slice(),
                        )
                    })
            })),
    );
    if !supertrait_errors.is_empty() {
        return Err(supertrait_errors.join("\n"));
    }

    let impls = products
        .interface
        .impls
        .iter()
        .map(|(id, value)| remap_impl_interface_ids(value.clone(), *id, remap, &type_validator))
        .map(|result| result.map(|value| (value.id, value)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    let externs = products
        .interface
        .externs
        .iter()
        .map(|(id, value)| remap_extern_interface_id(value.clone(), *id, remap, &type_validator))
        .map(|result| result.map(|value| (value.id, value)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    let effective_trait_methods = products
        .interface
        .effective_trait_methods
        .iter()
        .map(|((impl_id, member_id), method_id)| {
            Ok((
                (remap.def_id(*impl_id)?, remap.def_id(*member_id)?),
                remap.def_id(*method_id)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;

    let interface = super::ArtifactCrateInterface {
        root_export_ids,
        canonical_names,
        functions,
        structs,
        enums,
        type_aliases,
        traits,
        impls,
        externs,
        effective_trait_methods,
        infix_precedence: products.infix_precedence.clone(),
    };
    let mut coherence_traits = interface
        .trait_items()
        .into_iter()
        .map(|(_, trait_def)| trait_def)
        .collect::<Vec<_>>();
    let mut coherence_impls = interface.impl_items();
    let mut coherence_structs = interface
        .struct_items()
        .into_iter()
        .map(|(_, structure)| structure)
        .collect::<Vec<_>>();
    let mut coherence_enums = interface
        .enum_items()
        .into_iter()
        .map(|(_, enumeration)| enumeration)
        .collect::<Vec<_>>();
    let mut coherence_aliases = interface
        .type_alias_items()
        .into_iter()
        .map(|(_, alias)| alias)
        .collect::<Vec<_>>();
    for dependency in ctx.extern_crates() {
        let dependency = dependency.metadata().interface();
        coherence_traits.extend(
            dependency
                .trait_items()
                .into_iter()
                .map(|(_, trait_def)| trait_def),
        );
        coherence_impls.extend(dependency.impl_items());
        coherence_structs.extend(
            dependency
                .struct_items()
                .into_iter()
                .map(|(_, structure)| structure),
        );
        coherence_enums.extend(
            dependency
                .enum_items()
                .into_iter()
                .map(|(_, enumeration)| enumeration),
        );
        coherence_aliases.extend(
            dependency
                .type_alias_items()
                .into_iter()
                .map(|(_, alias)| alias),
        );
    }
    let coherence_errors = crate::traits::coherence::validate_coherence(
        coherence_traits.iter(),
        coherence_impls.iter(),
        coherence_structs.iter(),
        coherence_enums.iter(),
        coherence_aliases.iter(),
        language_items
            .sized
            .as_ref()
            .map(|items| items.trait_id)
            .or_else(|| {
                ctx.extern_crates().find_map(|dependency| {
                    dependency
                        .metadata()
                        .language_items()
                        .sized
                        .as_ref()
                        .map(|items| items.trait_id)
                })
            }),
    );
    if !coherence_errors.is_empty() {
        return Err(coherence_errors.join("\n"));
    }

    Ok(interface)
}

fn resolver_from_products(
    products: &CompilerProducts,
    crate_name: &str,
    remap: &ProductIdentityRemap,
    ctx: &CrateContext,
) -> Result<ResolverTables, String> {
    let mut resolver = ResolverTables::default();
    for (id, name) in &products.identity_table.display_names {
        if let Some(def_id) = product_def_id_to_existing_def_id(products, remap, *id)? {
            let source = qualify_product_name(crate_name, name);
            resolver.item_paths.insert(source.clone(), def_id);
            resolver.item_names_by_id.insert(def_id, source);
        }
    }
    for (alias, id) in &products.identity_table.export_names {
        if let Some(def_id) = product_def_id_to_existing_def_id(products, remap, *id)? {
            resolver.export_aliases.insert(alias.clone(), def_id);
        }
    }
    for (alias, id) in &products.identity_table.import_alias_names {
        if let Some(def_id) = product_def_id_to_alias_def_id(products, remap, *id)? {
            if let Some(source) = canonical_alias_source(&resolver, ctx, def_id) {
                resolver.insert_import_alias_with_name(alias.clone(), source, def_id);
            }
        }
    }
    for (alias, id) in &products.identity_table.module_alias_names {
        if let Some(def_id) = product_def_id_to_alias_def_id(products, remap, *id)? {
            if let Some(source) = canonical_alias_source(&resolver, ctx, def_id) {
                resolver.insert_module_alias_with_name(alias.clone(), source, def_id);
            }
        }
    }

    Ok(resolver)
}

fn product_def_id_to_alias_def_id(
    products: &CompilerProducts,
    remap: &ProductIdentityRemap,
    id: ProductDefId,
) -> Result<Option<DefId>, String> {
    if product_id_has_canonical_binding(products, id)
        || products.identity_table.local_crate != Some(id.crate_id)
    {
        Ok(Some(remap.def_id(id)?))
    } else {
        Ok(None)
    }
}

fn canonical_alias_source(
    resolver: &ResolverTables,
    ctx: &CrateContext,
    def_id: DefId,
) -> Option<String> {
    resolver
        .canonical_name(def_id)
        .map(str::to_string)
        .or_else(|| {
            ctx.extern_crates().find_map(|dep| {
                (dep.crate_id() == def_id.crate_id)
                    .then(|| dep.metadata().resolver().canonical_name(def_id))
                    .flatten()
                    .map(str::to_string)
            })
        })
}

fn cross_crate_hir_from_products(
    products: &CompilerProducts,
    crate_name: &str,
    remap: &ProductIdentityRemap,
    ctx: &CrateContext,
) -> Result<super::ArtifactCrossCrateHir, String> {
    let dependencies = ProductDependencyDefinitions::from_context(ctx, remap);
    validate_product_body_rows_have_interface(products)?;
    validate_product_method_authority_maps(products, &dependencies)?;

    let validator = ProductChildLocationValidator::from_products_with_context(products, remap, ctx);
    let type_validator =
        ProductNominalTypeValidator::from_products_with_dependencies(products, remap, dependencies);

    let generic_functions = products
        .bodies
        .functions
        .iter()
        .map(|(id, function)| (*id, function.clone()))
        .map(|(id, function)| {
            remap_function_id(function, id, remap, &validator, &type_validator)
                .map(|function| (function.id, function))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    let mut traits_with_defaults = BTreeMap::new();
    for (id, trait_def) in &products.interface.traits {
        let Some(trait_def) = trait_with_default_bodies_from_interface(*id, trait_def, products)?
        else {
            continue;
        };
        let trait_def = remap_trait_ids(trait_def, *id, remap, &validator, &type_validator)?;
        traits_with_defaults.insert(trait_def.id, trait_def);
    }

    let generic_impls = products
        .bodies
        .generic_impls
        .iter()
        .map(|(id, imp)| {
            remap_impl_ids(
                imp.clone(),
                *id,
                crate_name,
                remap,
                &validator,
                &type_validator,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(super::ArtifactCrossCrateHir {
        generic_functions,
        traits_with_defaults,
        generic_impls,
    })
}

fn validate_product_method_authority_maps(
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

fn validate_product_body_rows_have_interface(products: &CompilerProducts) -> Result<(), String> {
    for (id, body) in &products.bodies.functions {
        if !products.interface.functions.contains_key(id) {
            return Err(format!(
                "Product artifact function body {} has no interface declaration",
                id.local_id.0
            ));
        }

        let embedded_id = ProductDefId::from(body.id);
        if embedded_id != *id {
            return Err(format!(
                "Product artifact function body {} has embedded ID {} with no matching interface declaration",
                id.local_id.0, embedded_id.local_id.0
            ));
        }
    }

    for (id, function) in &products.interface.functions {
        if product_function_interface_requires_reusable_body(function)
            && !products.bodies.functions.contains_key(id)
        {
            return Err(format!(
                "Product artifact generic function {} has no serialized body",
                id.local_id.0
            ));
        }
    }

    let trait_default_ids = products
        .interface
        .traits
        .iter()
        .flat_map(|(trait_id, trait_def)| {
            trait_def
                .methods
                .iter()
                .map(|(name, method)| (ProductDefId::from(method.id), (*trait_id, name.as_str())))
        })
        .collect::<BTreeMap<_, _>>();
    for (id, body) in &products.bodies.trait_default_methods {
        let Some((trait_id, method_name)) = trait_default_ids.get(id) else {
            return Err(format!(
                "Product artifact trait default body {} has no interface declaration",
                id.local_id.0
            ));
        };

        let embedded_id = ProductDefId::from(body.id);
        if embedded_id != *id {
            return Err(format!(
                "Product artifact trait default body {} for trait {} method '{}' has embedded ID {} with no matching interface declaration",
                id.local_id.0, trait_id.local_id.0, method_name, embedded_id.local_id.0
            ));
        }
    }

    for (id, interface_impl) in &products.interface.impls {
        if product_impl_interface_requires_reusable_body(interface_impl)
            && !products.bodies.generic_impls.contains_key(id)
        {
            return Err(format!(
                "Product artifact generic impl {} has no serialized body",
                id.local_id.0
            ));
        }
    }

    for (id, imp) in &products.bodies.generic_impls {
        let Some(interface_impl) = products.interface.impls.get(id) else {
            return Err(format!(
                "Product artifact generic impl body {} has no interface declaration",
                id.local_id.0
            ));
        };

        let embedded_id = ProductDefId::from(imp.id);
        if embedded_id != *id {
            return Err(format!(
                "Product artifact generic impl body {} has embedded ID {} with no matching interface declaration",
                id.local_id.0, embedded_id.local_id.0
            ));
        }

        for (method_name, method) in &imp.methods {
            if method.name != *method_name {
                return Err(format!(
                    "Product artifact generic impl body {} method '{}' has name '{}'",
                    id.local_id.0, method_name, method.name
                ));
            }
            let Some(interface_method) = interface_impl.methods.get(method_name) else {
                return Err(format!(
                    "Product artifact generic impl body {} method '{}' has no interface declaration",
                    id.local_id.0, method_name
                ));
            };
            let method_id = ProductDefId::from(method.id);
            let interface_method_id = ProductDefId::from(interface_method.id);
            if method_id != interface_method_id {
                return Err(format!(
                    "Product artifact generic impl body {} method '{}' has embedded ID {} but interface declaration ID {}",
                    id.local_id.0,
                    method_name,
                    method_id.local_id.0,
                    interface_method_id.local_id.0
                ));
            }
        }

        for (method_name, interface_method) in &interface_impl.methods {
            let Some(body_method) = imp.methods.get(method_name) else {
                return Err(format!(
                    "Product artifact generic impl body {} method '{}' has no serialized body",
                    id.local_id.0, method_name
                ));
            };
            let body_method_id = ProductDefId::from(body_method.id);
            let interface_method_id = ProductDefId::from(interface_method.id);
            if body_method_id != interface_method_id {
                return Err(format!(
                    "Product artifact generic impl body {} method '{}' has embedded ID {} but interface declaration ID {}",
                    id.local_id.0,
                    method_name,
                    body_method_id.local_id.0,
                    interface_method_id.local_id.0
                ));
            }
        }
    }

    Ok(())
}

fn product_function_interface_requires_reusable_body(function: &ProductFunctionInterface) -> bool {
    !function.generic_params.is_empty()
}

fn product_impl_interface_requires_reusable_body(impl_def: &ProductImplInterface) -> bool {
    !impl_def.type_generics.is_empty()
        || !impl_def.trait_generics.is_empty()
        || impl_def
            .methods
            .values()
            .any(product_function_interface_requires_reusable_body)
}

fn trait_with_default_bodies_from_interface(
    trait_id: ProductDefId,
    trait_def: &ProductTraitInterface,
    products: &CompilerProducts,
) -> Result<Option<crate::hir::AcceptedHirTrait>, String> {
    if trait_def.methods.is_empty() {
        return Ok(None);
    }

    let mut methods = HashMap::new();
    for (name, method) in &trait_def.methods {
        let method_id = ProductDefId::from(method.id);
        let Some(body) = products.bodies.trait_default_methods.get(&method_id) else {
            return Err(format!(
                "Product artifact trait {} default method '{}' has no serialized body",
                trait_id.local_id.0, name
            ));
        };
        methods.insert(name.clone(), body.clone());
    }

    Ok(Some(crate::hir::HirTraitFor::<crate::hir::AcceptedHir> {
        id: trait_def.id,
        name: trait_def.name.clone(),
        generic_params: trait_def.generic_params.clone(),
        target: trait_def.target.clone(),
        predicates: trait_def.predicates.clone(),
        associated_types: trait_def.associated_types.clone(),
        methods,
        signatures: trait_def.signatures.clone(),
    }))
}

fn product_display_name(products: &CompilerProducts, id: ProductDefId) -> Option<String> {
    products.identity_table.display_names.get(&id).cloned()
}

fn qualify_product_name(crate_name: &str, name: &str) -> String {
    if name
        .strip_prefix(crate_name)
        .is_some_and(|rest| rest.starts_with("::"))
    {
        name.to_string()
    } else {
        format!("{}::{}", crate_name, name)
    }
}

fn product_def_id_to_existing_def_id(
    products: &CompilerProducts,
    remap: &ProductIdentityRemap,
    id: ProductDefId,
) -> Result<Option<DefId>, String> {
    if product_id_has_canonical_binding(products, id) {
        Ok(Some(remap.def_id(id)?))
    } else {
        Ok(None)
    }
}

fn product_def_id_to_prelude_export_def_id(
    products: &CompilerProducts,
    remap: &ProductIdentityRemap,
    id: ProductDefId,
) -> Result<Option<DefId>, String> {
    if products.interface.functions.contains_key(&id)
        || products.interface.structs.contains_key(&id)
        || products.interface.enums.contains_key(&id)
        || products.interface.type_aliases.contains_key(&id)
        || products.interface.traits.contains_key(&id)
        || products.interface.externs.contains_key(&id)
    {
        Ok(Some(remap.def_id(id)?))
    } else {
        Ok(None)
    }
}

fn product_id_has_canonical_binding(products: &CompilerProducts, id: ProductDefId) -> bool {
    products.interface.functions.contains_key(&id)
        || products.interface.structs.contains_key(&id)
        || products.interface.enums.contains_key(&id)
        || products.interface.type_aliases.contains_key(&id)
        || products.interface.traits.contains_key(&id)
        || products.interface.externs.contains_key(&id)
        || products.interface.traits.values().any(|trait_def| {
            trait_def
                .methods
                .values()
                .any(|method| ProductDefId::from(method.id) == id)
        })
        || products.interface.impls.values().any(|impl_def| {
            impl_def
                .methods
                .values()
                .any(|method| ProductDefId::from(method.id) == id)
        })
}

#[cfg(test)]
mod product_tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::fs;
    use std::path::PathBuf;

    use crate::collect::resolver::ResolverTables;
    use crate::crate_artifact::{ArtifactCrateInterface, ArtifactCrossCrateHir};
    use crate::crate_system::{
        CrateContext, ExternCrateBodies, ExternCrateLink, ExternCrateMetadata, ExternCrateRecord,
    };
    use crate::hir::{
        AcceptedHir, AcceptedHirBlock as HirBlock, AcceptedHirExpr as HirExpr,
        AcceptedHirFunction as HirFunction, AcceptedHirImpl as HirImpl,
        AcceptedHirTrait as HirTrait, HirAssociatedTypeDecl, HirCallTarget, HirClosureCapture,
        HirClosureCaptureKind, HirEnum, HirExprKindFor, HirExtern, HirField, HirFieldLocation,
        HirFunctionSig, HirImplOwner, HirMethodCallTarget, HirParam, HirPattern, HirStruct,
        HirStructPatternField, HirVarRef, HirVarTarget, HirVariant, HirVariantFields,
        HirVariantLocation,
    };
    use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
    use crate::language_items::DropLanguageItems;
    use crate::lexer::Span;
    use crate::products::{
        CompilerProducts, ProductBodies, ProductCrateId, ProductCrateIdentity, ProductDefId,
        ProductEnumInterface, ProductExternInterface, ProductFieldInterface,
        ProductFunctionInterface, ProductIdentityTable, ProductImplInterface, ProductInterface,
        ProductLinkData, ProductLinkRecord, ProductLocalDefId, ProductSourceFingerprint,
        ProductStructInterface, ProductTraitInterface, PRODUCT_ARTIFACT_MAGIC,
    };
    use crate::type_services::kind::Kind;
    use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, Type};

    type HirMatchArm = crate::hir::HirMatchArmFor<AcceptedHir>;
    type HirStmt = crate::hir::HirStmtFor<AcceptedHir>;

    fn product_def_id(crate_id: u32, local_id: u32) -> ProductDefId {
        ProductDefId {
            crate_id: ProductCrateId(crate_id),
            local_id: ProductLocalDefId(local_id),
        }
    }

    fn product_hir_def_id(id: ProductDefId) -> DefId {
        DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0))
    }

    fn generic_params(owner: DefId, names: &[&str]) -> Vec<GenericParamDecl> {
        GenericParamDecl::type_params(owner, names.iter().copied())
    }

    #[test]
    fn remap_var_target_child_location_rejects_serialized_instance_target() {
        let product_crate = ProductCrateId(1);
        let remap = super::ProductIdentityRemap {
            local_crate: product_crate,
            crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
        };
        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table: ProductIdentityTable::default(),
            interface: crate::products::ProductInterface::default(),
            bodies: ProductBodies::default(),
            link: ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        };
        let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
        let mut reference = HirVarRef {
            name: "identity".to_string(),
            target: HirVarTarget::Instance(crate::ids::InstanceId(5)),
        };

        let err = super::remap_var_target_child_location(
            &mut reference,
            &remap,
            &validator,
            &BTreeSet::new(),
        )
        .unwrap_err();

        assert!(
            err.contains("local instance target"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn remap_pattern_child_locations_remaps_new_pattern_sidecars() {
        let product_crate = ProductCrateId(1);
        let old_struct = product_def_id(1, 10);
        let old_enum = product_def_id(1, 11);
        let remap = super::ProductIdentityRemap {
            local_crate: product_crate,
            crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
        };
        let mut products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table: ProductIdentityTable::default(),
            interface: crate::products::ProductInterface::default(),
            bodies: ProductBodies::default(),
            link: ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        };
        let struct_def = HirStruct {
            id: product_hir_def_id(old_struct),
            name: "dep::Box".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        };
        products
            .interface
            .structs
            .insert(old_struct, ProductStructInterface::from(&struct_def));
        let enum_def = HirEnum {
            id: product_hir_def_id(old_enum),
            name: "dep::Option".to_string(),
            generic_params: Vec::new(),
            variants: vec![HirVariant {
                id: VariantId(0),
                name: "Some".to_string(),
                fields: HirVariantFields::Positional(vec![Type::I64]),
            }],
        };
        products
            .interface
            .enums
            .insert(old_enum, ProductEnumInterface::from(&enum_def));
        let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let mut pattern = HirPattern::Enum(
            "dep::Option".to_string(),
            "Some".to_string(),
            Some(HirVariantLocation {
                owner: product_hir_def_id(old_enum),
                variant_id: VariantId(0),
                name: "Some".to_string(),
            }),
            vec![HirPattern::Struct(
                "dep::Box".to_string(),
                Some(product_hir_def_id(old_struct)),
                vec![Type::Struct {
                    id: product_hir_def_id(old_struct),
                    args: Vec::new(),
                }],
                vec![HirStructPatternField {
                    name: "value".to_string(),
                    field: Some(HirFieldLocation {
                        owner: product_hir_def_id(old_struct),
                        field_id: FieldId(0),
                        name: "value".to_string(),
                    }),
                    pattern: HirPattern::Wildcard,
                }],
            )],
        );

        super::remap_pattern_child_locations(&mut pattern, &remap, &child_validator, &validator, 0)
            .unwrap();

        let expected_struct = remap.def_id(old_struct).unwrap();
        let expected_enum = remap.def_id(old_enum).unwrap();
        let HirPattern::Enum(_, _, Some(location), payloads) = pattern else {
            panic!("expected enum pattern");
        };
        assert_eq!(location.owner, expected_enum);
        let HirPattern::Struct(_, Some(struct_id), args, fields) = &payloads[0] else {
            panic!("expected struct payload pattern");
        };
        assert_eq!(*struct_id, expected_struct);
        assert_eq!(
            args[0],
            Type::Struct {
                id: expected_struct,
                args: Vec::new(),
            }
        );
        assert_eq!(fields[0].field.as_ref().unwrap().owner, expected_struct);
    }

    #[test]
    fn remap_expr_child_locations_remaps_call_target_sidecars() {
        let product_crate = ProductCrateId(1);
        let function_product_id = product_def_id(1, 10);
        let extern_product_id = product_def_id(1, 11);
        let function_id = product_hir_def_id(function_product_id);
        let extern_id = product_hir_def_id(extern_product_id);
        let remap = super::ProductIdentityRemap {
            local_crate: product_crate,
            crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
        };
        let mut products = product_with_function(
            "dep",
            product_crate,
            function_product_id.local_id.0,
            PathBuf::from("dep.o"),
        );
        insert_interface_extern(
            &mut products,
            extern_product_id,
            HirExtern {
                id: extern_id,
                name: "foreign".to_string(),
                params: Vec::new(),
                ret: Type::I64,
                variadic: false,
                is_unsafe: false,
            },
        );
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
        let mut expr = HirExpr {
            kind: HirExprKindFor::TupleLiteral(vec![
                HirExpr {
                    kind: HirExprKindFor::Call(
                        Box::new(HirExpr {
                            kind: HirExprKindFor::Var("dep::answer".to_string()),
                            ty: Type::function(Vec::new(), Type::I64),
                            span: Span::test(),
                        }),
                        Vec::new(),
                        Some(HirCallTarget::Function(function_id)),
                    ),
                    ty: Type::I64,
                    span: Span::test(),
                },
                HirExpr {
                    kind: HirExprKindFor::Call(
                        Box::new(HirExpr {
                            kind: HirExprKindFor::Var("foreign".to_string()),
                            ty: Type::function(Vec::new(), Type::I64),
                            span: Span::test(),
                        }),
                        Vec::new(),
                        Some(HirCallTarget::Extern(extern_id)),
                    ),
                    ty: Type::I64,
                    span: Span::test(),
                },
            ]),
            ty: Type::Tuple(vec![Type::I64, Type::I64]),
            span: Span::test(),
        };

        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap();

        let HirExprKindFor::TupleLiteral(items) = &expr.kind else {
            panic!("expected tuple literal");
        };
        let HirExprKindFor::Call(_, _, Some(function_target)) = &items[0].kind else {
            panic!("expected function call target");
        };
        assert_eq!(
            function_target,
            &HirCallTarget::Function(remap.def_id(function_product_id).unwrap())
        );
        let HirExprKindFor::Call(_, _, Some(extern_target)) = &items[1].kind else {
            panic!("expected extern call target");
        };
        assert_eq!(
            extern_target,
            &HirCallTarget::Extern(remap.def_id(extern_product_id).unwrap())
        );
    }

    #[test]
    fn remap_function_child_locations_rejects_unknown_local_call_target() {
        let product_crate = ProductCrateId(1);
        let function_product_id = product_def_id(1, 10);
        let remap = super::ProductIdentityRemap {
            local_crate: product_crate,
            crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
        };
        let products = product_with_function(
            "dep",
            product_crate,
            function_product_id.local_id.0,
            PathBuf::from("dep.o"),
        );
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
        let mut function = products.bodies.functions[&function_product_id].clone();
        function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::Call(
                    Box::new(HirExpr {
                        kind: HirExprKindFor::Var("local_fn".to_string()),
                        ty: Type::function(Vec::new(), Type::I64),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    Some(HirCallTarget::Local(crate::ids::HirLocalId(99))),
                ),
                ty: Type::I64,
                span: Span::test(),
            })],
            ty: Type::I64,
        };

        let err = super::remap_function_child_locations(
            &mut function,
            &remap,
            &child_validator,
            &type_validator,
        )
        .unwrap_err();

        assert!(
            err.contains("unknown local call target"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn remap_function_child_locations_rejects_unknown_local_resolved_var_target() {
        let product_crate = ProductCrateId(1);
        let function_product_id = product_def_id(1, 10);
        let remap = super::ProductIdentityRemap {
            local_crate: product_crate,
            crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
        };
        let products = product_with_function(
            "dep",
            product_crate,
            function_product_id.local_id.0,
            PathBuf::from("dep.o"),
        );
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
        let mut function = products.bodies.functions[&function_product_id].clone();
        function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::ResolvedVar(HirVarRef {
                    name: "local_fn".to_string(),
                    target: HirVarTarget::Local(crate::ids::HirLocalId(99)),
                }),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            })],
            ty: Type::function(Vec::new(), Type::I64),
        };

        let err = super::remap_function_child_locations(
            &mut function,
            &remap,
            &child_validator,
            &type_validator,
        )
        .unwrap_err();

        assert!(
            err.contains("unknown local var target"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn remap_function_child_locations_rejects_call_target_to_lambda_local() {
        let product_crate = ProductCrateId(1);
        let function_product_id = product_def_id(1, 10);
        let lambda_local = crate::ids::HirLocalId(42);
        let remap = super::ProductIdentityRemap {
            local_crate: product_crate,
            crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
        };
        let products = product_with_function(
            "dep",
            product_crate,
            function_product_id.local_id.0,
            PathBuf::from("dep.o"),
        );
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
        let mut function = products.bodies.functions[&function_product_id].clone();
        function.body = HirBlock {
            stmts: vec![
                HirStmt::Expr(HirExpr {
                    kind: HirExprKindFor::Lambda {
                        params: vec![HirParam {
                            name: "callback".to_string(),
                            local_id: lambda_local,
                            ty: Type::function(Vec::new(), Type::I64),
                            mutable: false,
                            is_ref: false,
                        }],
                        body: HirBlock {
                            stmts: Vec::new(),
                            ty: Type::Unit,
                        },
                        captures: Vec::new(),
                    },
                    ty: Type::function(Vec::new(), Type::Unit),
                    span: Span::test(),
                }),
                HirStmt::Expr(HirExpr {
                    kind: HirExprKindFor::Call(
                        Box::new(HirExpr {
                            kind: HirExprKindFor::Var("callback".to_string()),
                            ty: Type::function(Vec::new(), Type::I64),
                            span: Span::test(),
                        }),
                        Vec::new(),
                        Some(HirCallTarget::Local(lambda_local)),
                    ),
                    ty: Type::I64,
                    span: Span::test(),
                }),
            ],
            ty: Type::I64,
        };

        let err = super::remap_function_child_locations(
            &mut function,
            &remap,
            &child_validator,
            &type_validator,
        )
        .unwrap_err();

        assert!(
            err.contains("unknown local call target"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn remap_function_child_locations_rejects_call_target_to_current_let_local() {
        let product_crate = ProductCrateId(1);
        let function_product_id = product_def_id(1, 10);
        let let_local = crate::ids::HirLocalId(7);
        let remap = super::ProductIdentityRemap {
            local_crate: product_crate,
            crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
        };
        let products = product_with_function(
            "dep",
            product_crate,
            function_product_id.local_id.0,
            PathBuf::from("dep.o"),
        );
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
        let mut function = products.bodies.functions[&function_product_id].clone();
        function.body = HirBlock {
            stmts: vec![HirStmt::Let {
                name: "local_fn".to_string(),
                local_id: let_local,
                ty: Type::I64,
                value: HirExpr {
                    kind: HirExprKindFor::Call(
                        Box::new(HirExpr {
                            kind: HirExprKindFor::Var("local_fn".to_string()),
                            ty: Type::function(Vec::new(), Type::I64),
                            span: Span::test(),
                        }),
                        Vec::new(),
                        Some(HirCallTarget::Local(let_local)),
                    ),
                    ty: Type::I64,
                    span: Span::test(),
                },
                mutable: false,
            }],
            ty: Type::I64,
        };

        let err = super::remap_function_child_locations(
            &mut function,
            &remap,
            &child_validator,
            &type_validator,
        )
        .unwrap_err();

        assert!(
            err.contains("unknown local call target"),
            "unexpected error: {err}"
        );
    }

    fn test_function(id: DefId, name: &str) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::I64,
            body: HirBlock {
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKindFor::IntLiteral(5),
                    ty: Type::I64,
                    span: Span::test(),
                })],
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    #[test]
    fn intern_loaded_interface_types_rejects_one_display_alias_for_distinct_ids() {
        let first = DefId::new(CrateId(0), LocalDefId(1));
        let second = DefId::new(CrateId(0), LocalDefId(2));
        let mut second_function = test_function(second, "payload_second");
        second_function.ret_type = Type::Bool;
        second_function.body = HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        };

        let mut interface = ArtifactCrateInterface::default();
        interface.insert_function(
            "same_alias".to_string(),
            test_function(first, "payload_first"),
        );
        interface.insert_function("same_alias".to_string(), second_function);
        let mut context = crate::type_context::TypeContext::new();

        let error = super::intern_loaded_interface_types(
            &interface,
            &ResolverTables::default(),
            &mut context,
        )
        .expect_err("distinct artifact IDs must not share one display alias");

        assert!(error.contains("artifact function display alias 'same_alias'"));
    }

    #[test]
    fn artifact_name_tables_keep_resolver_aliases_on_the_payload_def_id() {
        let id = DefId::new(CrateId(0), LocalDefId(3));
        let function = test_function(id, "payload_display");
        let mut interface = ArtifactCrateInterface::default();
        interface.insert_function("dep::canonical".to_string(), function.clone());
        let mut resolver = ResolverTables::default();
        resolver
            .item_names_by_id
            .insert(id, "dep::canonical".to_string());
        resolver.import_aliases.insert("short".to_string(), id);
        let unresolved =
            crate::crate_artifact::types::hir_function_from_interface(&interface.functions[&id]);

        let names = super::artifact_hir_name_tables(
            &interface,
            &resolver,
            &HashMap::from([(id, unresolved)]),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        )
        .expect("resolver alias should remain attached to the exact artifact DefId");

        assert_eq!(names.functions_by_name["short"], id);
        assert_eq!(names.functions_by_name["dep::canonical"], id);
    }

    #[test]
    fn artifact_alias_conflicts_are_stable_across_reverse_insertion() {
        fn conflict(reverse: bool) -> String {
            let first = DefId::new(CrateId(0), LocalDefId(4));
            let second = DefId::new(CrateId(0), LocalDefId(5));
            let mut resolver = ResolverTables::default();
            let (first_source, second_source) = if reverse {
                (first, second)
            } else {
                (second, first)
            };
            resolver.item_paths.insert("same".to_string(), first_source);
            resolver
                .item_names_by_id
                .insert(second_source, "same".to_string());
            let functions = HashMap::from([
                (
                    first,
                    crate::crate_artifact::types::hir_function_from_interface(
                        &ProductFunctionInterface::from(&test_function(first, "first")),
                    ),
                ),
                (
                    second,
                    crate::crate_artifact::types::hir_function_from_interface(
                        &ProductFunctionInterface::from(&test_function(second, "second")),
                    ),
                ),
            ]);

            super::artifact_hir_name_tables(
                &ArtifactCrateInterface::default(),
                &resolver,
                &functions,
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
            )
            .expect_err("same artifact alias must have a stable conflict")
        }

        let expected = "artifact function display alias 'same' resolves to both DefId { crate_id: CrateId(0), local: LocalDefId(4) } and DefId { crate_id: CrateId(0), local: LocalDefId(5) }";
        assert_eq!(conflict(false), expected);
        assert_eq!(conflict(true), expected);
    }

    fn test_generic_impl(id: DefId, methods: HashMap<String, HirFunction>) -> HirImpl {
        let receiver_pattern = methods
            .values()
            .find(|method| method.is_method)
            .and_then(|method| method.params.first())
            .map(|param| match &param.ty {
                Type::Reference { inner, .. } => inner.as_ref().clone(),
                ty => ty.clone(),
            })
            .map(crate::hir::HirImplReceiverPattern::Exact)
            .unwrap_or_else(|| {
                crate::hir::HirImplReceiverPattern::Exact(Type::Generic(
                    crate::types::GenericParamId {
                        owner: id,
                        index: 0,
                    },
                ))
            });
        HirImpl {
            id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: generic_params(id, &["T"]),
            receiver_pattern,
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods,
        }
    }

    fn insert_interface_function(
        products: &mut CompilerProducts,
        id: ProductDefId,
        function: &HirFunction,
    ) {
        products
            .interface
            .functions
            .insert(id, ProductFunctionInterface::from(function));
    }

    fn insert_function_body(
        products: &mut CompilerProducts,
        id: ProductDefId,
        function: HirFunction,
    ) {
        insert_interface_function(products, id, &function);
        products.bodies.functions.insert(id, function);
    }

    fn insert_interface_struct(
        products: &mut CompilerProducts,
        id: ProductDefId,
        strukt: HirStruct,
    ) {
        products
            .interface
            .structs
            .insert(id, ProductStructInterface::from(&strukt));
    }

    fn insert_interface_enum(products: &mut CompilerProducts, id: ProductDefId, enm: HirEnum) {
        products
            .interface
            .enums
            .insert(id, ProductEnumInterface::from(&enm));
    }

    fn insert_interface_trait(
        products: &mut CompilerProducts,
        id: ProductDefId,
        trait_def: HirTrait,
    ) {
        products
            .interface
            .traits
            .insert(id, ProductTraitInterface::from(&trait_def));
    }

    fn insert_interface_impl(products: &mut CompilerProducts, id: ProductDefId, imp: HirImpl) {
        products
            .interface
            .impls
            .insert(id, ProductImplInterface::from(&imp));
    }

    #[test]
    fn product_artifact_rejects_receiver_pattern_missing_impl_generic() {
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let impl_id = product_def_id(0, 7);
        let imp = test_generic_impl(product_hir_def_id(impl_id), HashMap::new());
        insert_interface_impl(&mut products, impl_id, imp);
        products
            .interface
            .impls
            .get_mut(&impl_id)
            .unwrap()
            .receiver_pattern = crate::hir::HirImplReceiverPattern::Exact(Type::Unit);

        let error = super::validate_product_method_authority_maps(
            &products,
            &super::ProductDependencyDefinitions::default(),
        )
        .expect_err("receiver pattern must bind every impl generic");

        assert!(
            error.contains("receiver/trait argument generic bindings") && error.contains("0::7"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn product_artifact_allows_impl_generic_bound_by_trait_argument() {
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let impl_id = product_def_id(0, 8);
        let hir_impl_id = product_hir_def_id(impl_id);
        let mut imp = test_generic_impl(hir_impl_id, HashMap::new());
        imp.type_generics = generic_params(hir_impl_id, &["T", "U"]);
        imp.receiver_pattern = crate::hir::HirImplReceiverPattern::Exact(Type::Generic(
            crate::types::GenericParamId {
                owner: hir_impl_id,
                index: 0,
            },
        ));
        imp.trait_arg_types = vec![Type::Generic(crate::types::GenericParamId {
            owner: hir_impl_id,
            index: 1,
        })];
        insert_interface_impl(&mut products, impl_id, imp);

        super::validate_product_method_authority_maps(
            &products,
            &super::ProductDependencyDefinitions::default(),
        )
        .expect("trait arguments may bind impl generics absent from the receiver");
    }

    fn insert_interface_extern(products: &mut CompilerProducts, id: ProductDefId, ext: HirExtern) {
        products
            .interface
            .externs
            .insert(id, ProductExternInterface::from(&ext));
    }

    fn product_with_function(
        crate_name: &str,
        product_crate_id: ProductCrateId,
        local_id: u32,
        object_path: PathBuf,
    ) -> CompilerProducts {
        let product_id = ProductDefId {
            crate_id: product_crate_id,
            local_id: ProductLocalDefId(local_id),
        };
        let function_id = DefId::new(CrateId(product_crate_id.0), LocalDefId(local_id));
        let function_name = format!("{}::answer", crate_name);
        let function = test_function(function_id, &function_name);

        let mut identity_table = ProductIdentityTable::default();
        identity_table.local_crate = Some(product_crate_id);
        identity_table
            .display_names
            .insert(product_id, function_name.clone());
        identity_table
            .export_names
            .insert("answer".to_string(), product_id);

        let mut bodies = ProductBodies::default();
        bodies.functions.insert(product_id, function);

        let mut interface = ProductInterface::default();
        interface.functions.insert(
            product_id,
            ProductFunctionInterface::from(&bodies.functions[&product_id]),
        );

        let mut link_records = BTreeMap::new();
        link_records.insert(
            product_id,
            ProductLinkRecord {
                backend_symbol: format!("{}_answer", crate_name),
            },
        );

        CompilerProducts {
            crate_identity: ProductCrateIdentity::local(crate_name.to_string()),
            identity_table,
            interface,
            bodies,
            link: ProductLinkData {
                object_path: Some(object_path),
                records: link_records,
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        }
    }

    fn write_product_fixture(base: &std::path::Path, crate_name: &str) -> PathBuf {
        let artifact_path = base.join(format!("{}.rkca", crate_name));
        let object_path = base.join(format!("{}.o", crate_name));
        fs::write(&object_path, []).unwrap();

        let products = product_with_function(crate_name, ProductCrateId(0), 0, object_path);
        products.write_artifact_to_path(&artifact_path).unwrap();

        artifact_path
    }

    struct TempDirCleanup(PathBuf);

    impl Drop for TempDirCleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp_test_dir(label: &str) -> (PathBuf, TempDirCleanup) {
        let base = std::env::temp_dir().join(format!(
            "rock_product_loader_{}_{}",
            std::process::id(),
            label
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let cleanup = TempDirCleanup(base.clone());
        (base, cleanup)
    }

    #[test]
    fn load_product_artifact_inserts_artifact_only_extern_record() {
        let (base, _cleanup) = temp_test_dir("artifact_only_extern_record");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let products = product_with_function("dep", ProductCrateId(0), 0, object_path.clone());
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx
            .extern_crate("dep")
            .expect("extern record should be inserted");
        let answer_id = dep
            .metadata()
            .interface()
            .function_by_canonical_name("dep::answer")
            .expect("function interface should exist")
            .id;
        assert!(dep.body_providers().generic_function(answer_id).is_some());
        assert_eq!(
            dep.link().object_path(),
            Some(&object_path.canonicalize().unwrap())
        );
    }

    #[test]
    fn load_product_artifact_rejects_body_only_function_without_interface_declaration() {
        let (base, _cleanup) = temp_test_dir("body_only_function_without_interface");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products.interface.functions.clear();
        products.identity_table.export_names.clear();
        products.link.records.clear();
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("body-only function rows should be rejected");

        assert!(
            err.contains("function body") && err.contains("interface declaration"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_link_record_for_undeclared_callable_id() {
        let (base, _cleanup) = temp_test_dir("link_record_undeclared_callable");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let undeclared_id = product_def_id(0, 9);
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products.link.records.insert(
            undeclared_id,
            ProductLinkRecord {
                backend_symbol: "dep_hidden".to_string(),
            },
        );
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("link records for undeclared callable IDs should be rejected");

        assert!(
            err.contains("link record")
                && err.contains("backend symbol")
                && err.contains("interface callable declaration"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_empty_link_record_backend_symbol() {
        let (base, _cleanup) = temp_test_dir("empty_link_record_backend_symbol");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let function_id = product_def_id(0, 0);
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products
            .link
            .records
            .get_mut(&function_id)
            .unwrap()
            .backend_symbol = String::new();
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("empty backend symbols should be rejected");

        assert!(
            err.contains("0::0") && err.contains("empty backend symbol"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_object_function_missing_link_record() {
        let (base, _cleanup) = temp_test_dir("object_function_missing_link_record");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products.bodies.functions.clear();
        products.link.records.clear();
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("object-backed concrete functions require link records");

        assert!(
            err.contains("missing link record")
                && err.contains("backend symbol")
                && err.contains("function"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_object_concrete_function_body_without_link_record() {
        let (base, _cleanup) = temp_test_dir("object_concrete_function_body_missing_link_record");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products.link.records.clear();
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("object-backed concrete function bodies still require link records");

        assert!(
            err.contains("missing link record")
                && err.contains("backend symbol")
                && err.contains("function"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_object_impl_method_missing_link_record() {
        let (base, _cleanup) = temp_test_dir("object_impl_method_missing_link_record");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let artifact_path = base.join("dep.rkca");
        let impl_id = DefId::new(CrateId(0), LocalDefId(7));
        let method_id = DefId::new(CrateId(0), LocalDefId(8));
        let product_impl_id = ProductDefId::from(impl_id);
        let product_method_id = ProductDefId::from(method_id);
        let mut method = test_function(method_id, "make");
        method.is_method = true;
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("make".to_string(), method)]),
        };

        let mut identity_table = ProductIdentityTable::default();
        identity_table.local_crate = Some(ProductCrateId(0));
        identity_table
            .display_names
            .insert(product_impl_id, "Box".to_string());
        identity_table
            .display_names
            .insert(product_method_id, "Box::make".to_string());
        let mut interface = ProductInterface::default();
        interface
            .impls
            .insert(product_impl_id, ProductImplInterface::from(&imp));
        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table,
            interface,
            bodies: ProductBodies::default(),
            link: ProductLinkData {
                object_path: Some(object_path),
                records: BTreeMap::new(),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("object-backed concrete impl methods require link records");

        assert!(
            err.contains("missing link record")
                && err.contains("backend symbol")
                && err.contains("impl")
                && err.contains("method 'make'"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_object_concrete_impl_method_when_sibling_is_generic() {
        let (base, _cleanup) = temp_test_dir("object_impl_concrete_method_generic_sibling");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let artifact_path = base.join("dep.rkca");
        let impl_id = DefId::new(CrateId(0), LocalDefId(7));
        let method_id = DefId::new(CrateId(0), LocalDefId(8));
        let generic_method_id = DefId::new(CrateId(0), LocalDefId(9));
        let product_impl_id = ProductDefId::from(impl_id);
        let product_method_id = ProductDefId::from(method_id);
        let product_generic_method_id = ProductDefId::from(generic_method_id);

        let mut method = test_function(method_id, "make");
        method.is_method = true;

        let generic_param = crate::types::GenericParamId {
            owner: generic_method_id,
            index: 0,
        };
        let mut generic_method = test_function(generic_method_id, "id");
        generic_method.is_method = true;
        generic_method
            .generic_params
            .push(GenericParamDecl::type_param(generic_param, "T"));

        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([
                ("make".to_string(), method),
                ("id".to_string(), generic_method),
            ]),
        };

        let mut identity_table = ProductIdentityTable::default();
        identity_table.local_crate = Some(ProductCrateId(0));
        identity_table
            .display_names
            .insert(product_impl_id, "Box".to_string());
        identity_table
            .display_names
            .insert(product_method_id, "Box::make".to_string());
        identity_table
            .display_names
            .insert(product_generic_method_id, "Box::id".to_string());
        let mut interface = ProductInterface::default();
        interface
            .impls
            .insert(product_impl_id, ProductImplInterface::from(&imp));
        let mut bodies = ProductBodies::default();
        bodies.generic_impls.insert(product_impl_id, imp);
        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table,
            interface,
            bodies,
            link: ProductLinkData {
                object_path: Some(object_path),
                records: BTreeMap::new(),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("object-backed concrete impl methods require link records");

        assert!(
            err.contains("missing link record")
                && err.contains("backend symbol")
                && err.contains("impl")
                && err.contains("method 'make'"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_allows_generic_function_without_link_record() {
        let (base, _cleanup) = temp_test_dir("generic_function_without_link_record");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let artifact_path = base.join("dep.rkca");
        let product_id = product_def_id(0, 0);
        let function_id = product_hir_def_id(product_id);
        let mut function = test_function(function_id, "dep::id");
        function.generic_params.push(GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: function_id,
                index: 0,
            },
            "T",
        ));

        let mut identity_table = ProductIdentityTable::default();
        identity_table.local_crate = Some(ProductCrateId(0));
        identity_table
            .display_names
            .insert(product_id, "dep::id".to_string());
        let mut interface = ProductInterface::default();
        interface
            .functions
            .insert(product_id, ProductFunctionInterface::from(&function));
        let mut bodies = ProductBodies::default();
        bodies.functions.insert(product_id, function);
        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table,
            interface,
            bodies,
            link: ProductLinkData {
                object_path: Some(object_path),
                records: BTreeMap::new(),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();
    }

    #[test]
    fn load_product_artifact_rejects_body_only_function_link_record_without_interface_declaration()
    {
        let (base, _cleanup) = temp_test_dir("body_only_function_link_record");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let body_only_id = product_def_id(0, 9);
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products.bodies.functions.insert(
            body_only_id,
            test_function(product_hir_def_id(body_only_id), "hidden"),
        );
        products.link.records.insert(
            body_only_id,
            ProductLinkRecord {
                backend_symbol: "dep_hidden".to_string(),
            },
        );
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("body-only function link records should be rejected as backend symbols");

        assert!(
            err.contains("link record")
                && err.contains("backend symbol")
                && err.contains("interface callable declaration"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_body_only_trait_default_without_interface_declaration() {
        let (base, _cleanup) = temp_test_dir("body_only_trait_default_without_interface");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let method_id = product_def_id(0, 7);
        let method_def = product_hir_def_id(method_id);
        let mut products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table: ProductIdentityTable {
                local_crate: Some(ProductCrateId(0)),
                display_names: BTreeMap::from([(method_id, "dep::Show::show".to_string())]),
                ..ProductIdentityTable::default()
            },
            interface: ProductInterface::default(),
            bodies: ProductBodies::default(),
            link: ProductLinkData {
                object_path: Some(object_path),
                records: BTreeMap::new(),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        };
        products
            .bodies
            .trait_default_methods
            .insert(method_id, test_function(method_def, "show"));
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("body-only trait default rows should be rejected");

        assert!(
            err.contains("trait default body") && err.contains("interface declaration"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_body_only_generic_impl_without_interface_declaration() {
        let (base, _cleanup) = temp_test_dir("body_only_generic_impl_without_interface");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let impl_id = product_def_id(0, 7);
        products.bodies.generic_impls.insert(
            impl_id,
            HirImpl {
                id: product_hir_def_id(impl_id),
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: generic_params(product_hir_def_id(impl_id), &["T"]),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::new(),
            },
        );
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("body-only generic impl rows should be rejected");

        assert!(
            err.contains("generic impl body") && err.contains("interface declaration"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_generic_impl_body_extra_method_without_interface_declaration()
    {
        let (base, _cleanup) = temp_test_dir("generic_impl_body_extra_method");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let impl_id = product_def_id(0, 7);
        let method_id = product_def_id(0, 8);
        let extra_method_id = product_def_id(0, 9);

        let mut interface_method = test_function(product_hir_def_id(method_id), "value");
        interface_method.is_method = true;
        let interface_impl = test_generic_impl(
            product_hir_def_id(impl_id),
            HashMap::from([("value".to_string(), interface_method.clone())]),
        );
        insert_interface_impl(&mut products, impl_id, interface_impl);

        let mut extra_method = test_function(product_hir_def_id(extra_method_id), "hidden");
        extra_method.is_method = true;
        let body_impl = test_generic_impl(
            product_hir_def_id(impl_id),
            HashMap::from([
                ("value".to_string(), interface_method),
                ("hidden".to_string(), extra_method),
            ]),
        );
        products.bodies.generic_impls.insert(impl_id, body_impl);
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("extra generic impl body methods should be rejected");

        assert!(
            err.contains("generic impl body")
                && err.contains("method 'hidden'")
                && err.contains("interface declaration"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_generic_impl_body_method_id_mismatch() {
        let (base, _cleanup) = temp_test_dir("generic_impl_body_method_id_mismatch");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let impl_id = product_def_id(0, 7);
        let method_id = product_def_id(0, 8);
        let mismatched_method_id = product_def_id(0, 9);

        let mut interface_method = test_function(product_hir_def_id(method_id), "value");
        interface_method.is_method = true;
        let interface_impl = test_generic_impl(
            product_hir_def_id(impl_id),
            HashMap::from([("value".to_string(), interface_method)]),
        );
        insert_interface_impl(&mut products, impl_id, interface_impl);

        let mut body_method = test_function(product_hir_def_id(mismatched_method_id), "value");
        body_method.is_method = true;
        let body_impl = test_generic_impl(
            product_hir_def_id(impl_id),
            HashMap::from([("value".to_string(), body_method)]),
        );
        products.bodies.generic_impls.insert(impl_id, body_impl);
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("generic impl body method ID mismatches should be rejected");

        assert!(
            err.contains("generic impl body")
                && err.contains("method 'value'")
                && err.contains("interface declaration"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_trait_default_body_embedded_id_mismatch() {
        let (base, _cleanup) = temp_test_dir("trait_default_body_embedded_id_mismatch");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let trait_id = product_def_id(0, 7);
        let method_id = product_def_id(0, 8);
        let mismatched_method_id = product_def_id(0, 9);

        let mut interface_method = test_function(product_hir_def_id(method_id), "show");
        interface_method.is_method = true;
        let trait_def = HirTrait {
            target: None,
            predicates: Vec::new(),
            id: product_hir_def_id(trait_id),
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("show".to_string(), interface_method)]),
            signatures: HashMap::new(),
        };
        insert_interface_trait(&mut products, trait_id, trait_def);
        products
            .identity_table
            .display_names
            .insert(trait_id, "dep::Show".to_string());

        let mut body = test_function(product_hir_def_id(mismatched_method_id), "show");
        body.is_method = true;
        products
            .bodies
            .trait_default_methods
            .insert(method_id, body);
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("trait default body embedded ID mismatches should be rejected");

        assert!(
            err.contains("trait default body")
                && err.contains("embedded ID")
                && err.contains("interface declaration"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_invalid_drop_trait_language_item_id() {
        let (base, _cleanup) = temp_test_dir("invalid_drop_trait_language_item");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products.interface.language_items.drop = Some(DropLanguageItems {
            trait_id: product_def_id(0, 99),
            method_id: product_def_id(0, 100),
        });
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("invalid drop trait language items should be rejected");

        assert_eq!(
            err,
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(99) }: drop.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(99) } is no declaration"
        );
    }

    #[test]
    fn load_product_artifact_rejects_nonlocal_sized_language_item_without_mutating_context() {
        let (base, _cleanup) = temp_test_dir("nonlocal_sized_language_item");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products.interface.language_items.sized = Some(crate::language_items::SizedLanguageItems {
            trait_id: product_def_id(1, 7),
        });
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let mut type_context = crate::type_context::TypeContext::new();
        let error = ctx
            .load_product_artifact_from_path_with_type_context(artifact_path, &mut type_context)
            .expect_err("non-local language items must be rejected before loading");

        assert_eq!(
            error,
            "Product artifact language item sized.trait uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(7) }; expected product crate 0"
        );
        assert!(ctx.extern_crate("dep").is_none());
        assert_eq!(type_context.len(), 0);
    }

    #[test]
    fn load_product_artifact_rejects_mismatched_drop_method_language_item_id() {
        let (base, _cleanup) = temp_test_dir("mismatched_drop_method_language_item");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let drop_trait_id = product_def_id(0, 7);
        let other_trait_id = product_def_id(0, 9);
        let other_method_id = product_def_id(0, 10);
        let other_method_def = product_hir_def_id(other_method_id);

        insert_interface_trait(
            &mut products,
            drop_trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: product_hir_def_id(drop_trait_id),
                name: "Drop".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        products
            .identity_table
            .display_names
            .insert(drop_trait_id, "dep::Drop".to_string());
        insert_interface_trait(
            &mut products,
            other_trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: product_hir_def_id(other_trait_id),
                name: "Other".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "drop".to_string(),
                    HirFunctionSig {
                        id: other_method_def,
                        name: "drop".to_string(),
                        generic_params: Vec::new(),
                        params: Vec::new(),
                        ret: Type::Unit,
                        generic_bounds: HashMap::new().into(),
                        self_receiver: None,
                        is_unsafe: false,
                    },
                )]),
            },
        );
        products
            .identity_table
            .display_names
            .insert(other_trait_id, "dep::Other".to_string());
        products.interface.language_items.drop = Some(DropLanguageItems {
            trait_id: drop_trait_id,
            method_id: other_method_id,
        });
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("drop method language items must belong to the drop trait");

        assert_eq!(
            err,
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(7) }: drop.method DefId { crate_id: CrateId(0), local: LocalDefId(10) } is owned by trait DefId { crate_id: CrateId(0), local: LocalDefId(9) }, not DefId { crate_id: CrateId(0), local: LocalDefId(7) }"
        );
    }

    #[test]
    fn load_product_artifact_rejects_drop_method_language_item_not_declared_by_trait() {
        let (base, _cleanup) = temp_test_dir("drop_method_not_declared_by_trait");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let drop_trait_id = product_def_id(0, 7);
        insert_interface_trait(
            &mut products,
            drop_trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: product_hir_def_id(drop_trait_id),
                name: "Drop".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        products
            .identity_table
            .display_names
            .insert(drop_trait_id, "dep::Drop".to_string());
        products.interface.language_items.drop = Some(DropLanguageItems {
            trait_id: drop_trait_id,
            method_id: product_def_id(0, 8),
        });
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("drop method language item must be declared by the drop trait");

        assert_eq!(
            err,
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(7) }: drop.method DefId { crate_id: CrateId(0), local: LocalDefId(8) } is not declared by any trait"
        );
    }

    #[test]
    fn load_product_artifact_rejects_generic_impl_body_omitted_interface_method() {
        let (base, _cleanup) = temp_test_dir("generic_impl_body_omitted_interface_method");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let impl_id = product_def_id(0, 7);
        let method_id = product_def_id(0, 8);

        let mut interface_method = test_function(product_hir_def_id(method_id), "value");
        interface_method.is_method = true;
        let interface_impl = test_generic_impl(
            product_hir_def_id(impl_id),
            HashMap::from([("value".to_string(), interface_method)]),
        );
        insert_interface_impl(&mut products, impl_id, interface_impl);

        let body_impl = test_generic_impl(product_hir_def_id(impl_id), HashMap::new());
        products.bodies.generic_impls.insert(impl_id, body_impl);
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("generic impl bodies must include every interface method body");

        assert!(
            err.contains("generic impl body")
                && err.contains("method 'value'")
                && err.contains("serialized body"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_generic_function_interface_without_body() {
        let (base, _cleanup) = temp_test_dir("generic_function_interface_without_body");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let function_id = product_def_id(0, 0);
        let function_def = product_hir_def_id(function_id);
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let generic = crate::types::GenericParamId {
            owner: function_def,
            index: 0,
        };
        let function = products.interface.functions.get_mut(&function_id).unwrap();
        function
            .generic_params
            .push(GenericParamDecl::type_param(generic, "T"));
        products.bodies.functions.remove(&function_id);
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("generic function interface declarations need serialized bodies");

        assert!(
            err.contains("generic function") && err.contains("serialized body"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_generic_impl_interface_without_body() {
        let (base, _cleanup) = temp_test_dir("generic_impl_interface_without_body");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let impl_id = product_def_id(0, 7);
        let method_id = product_def_id(0, 8);
        let mut method = test_function(product_hir_def_id(method_id), "value");
        method.is_method = true;
        let interface_impl = test_generic_impl(
            product_hir_def_id(impl_id),
            HashMap::from([("value".to_string(), method)]),
        );
        insert_interface_impl(&mut products, impl_id, interface_impl);
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("generic impl interface declarations need serialized bodies");

        assert!(
            err.contains("generic impl") && err.contains("serialized body"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_interface_function_embedded_id_mismatch() {
        let (base, _cleanup) = temp_test_dir("interface_function_embedded_id_mismatch");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let function_id = product_def_id(0, 0);
        let mismatched_id = product_def_id(0, 9);
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products
            .interface
            .functions
            .get_mut(&function_id)
            .unwrap()
            .id = product_hir_def_id(mismatched_id);
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("interface function ID mismatches should be rejected");

        assert!(
            err.contains("interface function") && err.contains("embedded ID"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_interface_function_foreign_row_key() {
        let (base, _cleanup) = temp_test_dir("interface_function_foreign_row_key");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let foreign_function_id = product_def_id(1, 9);
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products.identity_table.dependencies.insert(
            ProductCrateId(1),
            ProductCrateIdentity::local("foreign".to_string()),
        );
        products
            .identity_table
            .display_names
            .insert(foreign_function_id, "foreign::answer".to_string());
        insert_interface_function(
            &mut products,
            foreign_function_id,
            &test_function(product_hir_def_id(foreign_function_id), "foreign::answer"),
        );
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("interface row keys must be owned by the artifact crate");

        assert!(
            err.contains("interface function") && err.contains("non-local"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_interface_trait_method_foreign_product_crate_id() {
        let (base, _cleanup) = temp_test_dir("interface_trait_method_foreign_product_crate");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let trait_id = product_def_id(0, 7);
        let foreign_method_id = product_def_id(1, 8);
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products.identity_table.dependencies.insert(
            ProductCrateId(1),
            ProductCrateIdentity::local("foreign".to_string()),
        );
        let mut method = test_function(product_hir_def_id(foreign_method_id), "show");
        method.is_method = true;
        insert_interface_trait(
            &mut products,
            trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: product_hir_def_id(trait_id),
                name: "Show".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::from([("show".to_string(), method.clone())]),
                signatures: HashMap::new(),
            },
        );
        products
            .identity_table
            .display_names
            .insert(trait_id, "dep::Show".to_string());
        products
            .bodies
            .trait_default_methods
            .insert(foreign_method_id, method);
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("nested trait method IDs must be owned by the artifact crate");

        assert!(
            err.contains("trait") && err.contains("method 'show'") && err.contains("non-local"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_nested_method_reusing_top_level_function_id() {
        let (base, _cleanup) = temp_test_dir("nested_method_reuses_function_id");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let function_id = product_def_id(0, 0);
        let impl_id = product_def_id(0, 7);
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let mut method = test_function(product_hir_def_id(function_id), "value");
        method.is_method = true;
        insert_interface_impl(
            &mut products,
            impl_id,
            HirImpl {
                id: product_hir_def_id(impl_id),
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), method)]),
            },
        );
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("callable declaration IDs must be unique");

        assert!(
            err.contains("duplicate callable ID")
                && err.contains("function")
                && err.contains("method 'value'"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_interface_impl_method_name_mismatch() {
        let (base, _cleanup) = temp_test_dir("interface_impl_method_name_mismatch");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let impl_id = product_def_id(0, 7);
        let method_id = product_def_id(0, 8);
        let mut method = test_function(product_hir_def_id(method_id), "hidden");
        method.is_method = true;
        let interface_impl = test_generic_impl(
            product_hir_def_id(impl_id),
            HashMap::from([("value".to_string(), method)]),
        );
        insert_interface_impl(&mut products, impl_id, interface_impl);
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("interface impl method names must match method row keys");

        assert!(
            err.contains("interface impl")
                && err.contains("method 'value'")
                && err.contains("name 'hidden'"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_reinterns_structural_types_for_consumer_context() {
        let (base, _cleanup) = temp_test_dir("reinterns_structural_types");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();
        let bytes = std::fs::read(&artifact_path).unwrap();
        assert!(crate::products::artifact_type_table_len_for_test(&bytes).unwrap() > 0);

        let mut type_context = crate::type_context::TypeContext::new();
        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path_with_type_context(artifact_path, &mut type_context)
            .unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        let function = dep
            .metadata()
            .interface()
            .function_by_canonical_name("dep::answer")
            .unwrap();
        let ret_id = type_context.id_for_type(&function.ret_type).unwrap();

        assert_eq!(type_context.type_for(ret_id), function.ret_type);
    }

    #[test]
    fn load_product_artifact_as_reinterns_structural_types_for_consumer_context() {
        let (base, _cleanup) = temp_test_dir("as_reinterns_structural_types");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();
        let bytes = std::fs::read(&artifact_path).unwrap();
        assert!(crate::products::artifact_type_table_len_for_test(&bytes).unwrap() > 0);

        let mut type_context = crate::type_context::TypeContext::new();
        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path_as_with_type_context(
            "dep",
            artifact_path,
            &mut type_context,
        )
        .unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        let function = dep
            .metadata()
            .interface()
            .function_by_canonical_name("dep::answer")
            .unwrap();
        let ret_id = type_context.id_for_type(&function.ret_type).unwrap();

        assert_eq!(type_context.type_for(ret_id), function.ret_type);
    }

    #[test]
    fn load_product_artifact_does_not_mutate_type_context_when_add_fails() {
        let (base, _cleanup) = temp_test_dir("failed_add_does_not_mutate_context");
        let first_object_path = base.join("dep.o");
        fs::write(&first_object_path, []).unwrap();
        let first_products = product_with_function("dep", ProductCrateId(0), 0, first_object_path);
        let first_artifact_path = base.join("dep.rkca");
        first_products
            .write_artifact_to_path(&first_artifact_path)
            .unwrap();

        let duplicate_object_path = base.join("dep_duplicate.o");
        fs::write(&duplicate_object_path, []).unwrap();
        let mut duplicate_products =
            product_with_function("dep", ProductCrateId(0), 0, duplicate_object_path);
        let product_id = product_def_id(0, 0);
        if let Some(function) = duplicate_products.interface.functions.get_mut(&product_id) {
            function.params.push(Type::Bool);
            function.ret_type = Type::Bool;
        }
        if let Some(function) = duplicate_products.bodies.functions.get_mut(&product_id) {
            function.params.push(HirParam {
                name: "flag".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Bool,
                mutable: false,
                is_ref: false,
            });
            function.ret_type = Type::Bool;
            function.body = HirBlock {
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKindFor::BoolLiteral(true),
                    ty: Type::Bool,
                    span: Span::test(),
                })],
                ty: Type::Bool,
            };
        }
        let duplicate_artifact_path = base.join("dep_duplicate.rkca");
        duplicate_products
            .write_artifact_to_path(&duplicate_artifact_path)
            .unwrap();

        let mut type_context = crate::type_context::TypeContext::new();
        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path_with_type_context(
            first_artifact_path,
            &mut type_context,
        )
        .unwrap();

        let err = ctx
            .load_product_artifact_from_path_with_type_context(
                duplicate_artifact_path,
                &mut type_context,
            )
            .unwrap_err();

        assert!(err.contains("duplicate external crate"));
        assert!(type_context.id_for_type(&Type::Bool).is_none());
    }

    #[test]
    fn load_product_artifact_failure_does_not_mutate_crate_context_identity_state() {
        let (base, _cleanup) = temp_test_dir("failed_load_does_not_mutate_crate_context");
        let bad_object_path = base.join("bad.o");
        fs::write(&bad_object_path, []).unwrap();
        let mut bad_products = product_with_function("bad", ProductCrateId(0), 0, bad_object_path);
        bad_products.identity_table.dependencies.insert(
            ProductCrateId(0),
            ProductCrateIdentity::local("invalid_dep".to_string()),
        );
        let bad_artifact_path = base.join("bad.rkca");
        bad_products
            .write_artifact_to_path(&bad_artifact_path)
            .unwrap();

        let mut type_context = crate::type_context::TypeContext::new();
        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path_with_type_context(bad_artifact_path, &mut type_context)
            .unwrap_err();

        assert!(err.contains("maps dependency 'invalid_dep' to local product crate ID 0"));
        assert!(ctx.product_crate_ids.is_empty());

        let good_artifact_path = write_product_fixture(&base, "good");
        ctx.load_product_artifact_from_path_with_type_context(
            good_artifact_path,
            &mut type_context,
        )
        .unwrap();

        assert_eq!(ctx.extern_crate("good").unwrap().crate_id(), CrateId(1));
    }

    #[test]
    fn load_product_artifact_rejects_previous_format_before_context_mutation() {
        let (base, _cleanup) = temp_test_dir("rejects_previous_format_before_mutation");
        let artifact_path = base.join("old.rkca");
        let mut preamble = Vec::new();
        preamble.extend_from_slice(&PRODUCT_ARTIFACT_MAGIC);
        preamble.extend_from_slice(&43u32.to_le_bytes());
        preamble.extend_from_slice(&0u64.to_le_bytes());
        preamble.extend_from_slice(&0u64.to_le_bytes());
        fs::write(&artifact_path, preamble).unwrap();

        let mut ctx = CrateContext::new();
        let mut type_context = crate::type_context::TypeContext::new();
        let error = ctx
            .load_product_artifact_from_path_with_type_context(artifact_path, &mut type_context)
            .unwrap_err();

        assert!(error.contains("Unsupported product artifact format 43"));
        assert!(ctx.product_crate_ids.is_empty());
        assert!(type_context.id_for_type(&Type::I64).is_none());
    }

    #[test]
    fn load_product_artifact_remap_failure_is_transactional_for_contexts() {
        let (base, _cleanup) = temp_test_dir("remap_failure_is_transactional");
        let good_object_path = base.join("good.o");
        fs::write(&good_object_path, []).unwrap();
        let good_artifact_path = base.join("good.rkca");
        product_with_function("good", ProductCrateId(0), 0, good_object_path)
            .write_artifact_to_path(&good_artifact_path)
            .unwrap();

        let bad_object_path = base.join("bad.o");
        fs::write(&bad_object_path, []).unwrap();
        let bad_artifact_path = base.join("bad.rkca");
        let bad_id = product_def_id(0, 0);
        let mut bad_products = product_with_function("bad", ProductCrateId(0), 0, bad_object_path);
        let mut bad_function = bad_products.bodies.functions[&bad_id].clone();
        bad_function.params.push(HirParam {
            name: "ghost".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(999)),
                args: Vec::new(),
            },
            mutable: false,
            is_ref: false,
        });
        insert_function_body(&mut bad_products, bad_id, bad_function);
        bad_products
            .write_artifact_to_path(&bad_artifact_path)
            .unwrap();

        let mut ctx = CrateContext::new();
        let mut type_context = crate::type_context::TypeContext::new();
        ctx.load_product_artifact_from_path_with_type_context(
            good_artifact_path,
            &mut type_context,
        )
        .unwrap();
        let product_ids_before = ctx.product_crate_ids.clone();
        let i64_id_before = type_context.id_for_type(&Type::I64);

        let error = ctx
            .load_product_artifact_from_path_with_type_context(bad_artifact_path, &mut type_context)
            .unwrap_err();

        assert!(error.contains("unknown nominal type definition"));
        assert_eq!(ctx.product_crate_ids, product_ids_before);
        assert_eq!(type_context.id_for_type(&Type::I64), i64_id_before);
        assert!(ctx.extern_crate("good").is_some());
        assert!(ctx.extern_crate("bad").is_none());
    }

    #[test]
    fn product_artifact_rejects_nested_error_in_unexported_interface_type() {
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let struct_id = product_def_id(0, 7);
        products.interface.structs.insert(
            struct_id,
            ProductStructInterface {
                id: product_hir_def_id(struct_id),
                name: "dep::Private".to_string(),
                generic_params: Vec::new(),
                fields: vec![ProductFieldInterface {
                    id: FieldId(0),
                    name: "value".to_string(),
                    ty: Type::Tuple(vec![Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::Error),
                    }]),
                    public: false,
                }],
            },
        );

        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let error = super::validate_product_type_fields(&products, &remap, &CrateContext::new())
            .unwrap_err();

        assert!(error.contains("Type::Error"), "unexpected error: {error}");
    }

    #[test]
    fn product_artifact_rejects_nested_type_var_in_unexported_body_type() {
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let function_id = product_def_id(0, 0);
        products
            .bodies
            .functions
            .get_mut(&function_id)
            .unwrap()
            .body
            .ty = Type::Tuple(vec![Type::Reference {
            mutable: false,
            inner: Box::new(Type::TypeVar(crate::ids::TypeVarId(0))),
        }]);

        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let error = super::validate_product_type_fields(&products, &remap, &CrateContext::new())
            .unwrap_err();

        assert!(
            error.contains("unresolved type"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn load_product_artifact_remaps_id_backed_import_aliases() {
        let (base, _cleanup) = temp_test_dir("id_backed_import_aliases");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 1, object_path);
        products
            .identity_table
            .import_alias_names
            .insert("short".to_string(), product_def_id(0, 1));

        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();
        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();
        let record = ctx.extern_crate("dep").unwrap();
        let resolver = record.metadata().resolver();

        assert_eq!(
            resolver.resolve_item_or_alias("short"),
            resolver.resolve_item_or_alias("dep::answer")
        );
    }

    #[test]
    fn load_product_artifact_drops_aliases_without_canonical_bindings() {
        let (base, _cleanup) = temp_test_dir("stale_id_backed_aliases");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();
        let mut products = product_with_function("dep", ProductCrateId(0), 1, object_path);
        products
            .identity_table
            .import_alias_names
            .insert("stale".to_string(), product_def_id(0, 99));
        products
            .identity_table
            .module_alias_names
            .insert("stale_mod".to_string(), product_def_id(0, 100));

        let artifact_path = base.join("dep.rkca");
        products.write_artifact_to_path(&artifact_path).unwrap();
        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();
        let record = ctx.extern_crate("dep").unwrap();
        let resolver = record.metadata().resolver();

        assert_eq!(resolver.resolve_item_or_alias("stale"), None);
        assert_eq!(resolver.resolve_item_or_alias("stale_mod"), None);
    }

    #[test]
    fn load_product_artifacts_remap_same_local_crate_ids_to_distinct_consumer_crates() {
        let (base, _cleanup) = temp_test_dir("distinct_consumer_crates");

        let a_artifact = write_product_fixture(&base, "a");
        let b_artifact = write_product_fixture(&base, "b");

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(a_artifact).unwrap();
        ctx.load_product_artifact_from_path(b_artifact).unwrap();

        let a_id = ctx
            .extern_crate("a")
            .unwrap()
            .metadata()
            .resolver()
            .item_paths["a::answer"];
        let b_id = ctx
            .extern_crate("b")
            .unwrap()
            .metadata()
            .resolver()
            .item_paths["b::answer"];

        assert_ne!(a_id.crate_id, b_id.crate_id);
        assert_eq!(a_id.local, LocalDefId(0));
        assert_eq!(b_id.local, LocalDefId(0));
    }

    #[test]
    fn product_crate_id_remap_reuses_shared_transitive_dependency_identity() {
        let shared = ProductCrateIdentity::local("shared".to_string());
        let mut first =
            product_with_function("first", ProductCrateId(0), 0, PathBuf::from("first.o"));
        first
            .identity_table
            .dependencies
            .insert(ProductCrateId(1), shared.clone());

        let mut second =
            product_with_function("second", ProductCrateId(0), 0, PathBuf::from("second.o"));
        second
            .identity_table
            .dependencies
            .insert(ProductCrateId(1), shared);

        let mut ctx = CrateContext::new();
        let first_remap = super::ProductIdentityRemap::from_products(&mut ctx, &first).unwrap();
        let second_remap = super::ProductIdentityRemap::from_products(&mut ctx, &second).unwrap();

        assert_ne!(
            first_remap.crate_ids[&ProductCrateId(0)],
            second_remap.crate_ids[&ProductCrateId(0)]
        );
        assert_eq!(
            first_remap.crate_ids[&ProductCrateId(1)],
            second_remap.crate_ids[&ProductCrateId(1)]
        );
    }

    #[test]
    fn load_product_artifact_rejects_dependency_identity_mismatch() {
        let (base, _cleanup) = temp_test_dir("dependency_identity_mismatch");
        let dep_artifact = base.join("dep.rkca");
        let dep_object = base.join("dep.o");
        fs::write(&dep_object, []).unwrap();
        let app_artifact = base.join("app.rkca");
        let app_object = base.join("app.o");
        fs::write(&app_object, []).unwrap();

        let dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
        dep_products.write_artifact_to_path(&dep_artifact).unwrap();

        let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
        let mut mismatched_identity = ProductCrateIdentity::local("dep".to_string());
        mismatched_identity.format_version = mismatched_identity.format_version + 1;
        app_products
            .identity_table
            .dependencies
            .insert(ProductCrateId(1), mismatched_identity);
        app_products.write_artifact_to_path(&app_artifact).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(dep_artifact).unwrap();
        let err = ctx
            .load_product_artifact_from_path(app_artifact)
            .unwrap_err();

        assert!(
            err.contains("dependency identity mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_dependency_fingerprint_mismatch() {
        let (base, _cleanup) = temp_test_dir("dependency_fingerprint_mismatch");
        let dep_artifact = base.join("dep.rkca");
        let dep_object = base.join("dep.o");
        fs::write(&dep_object, []).unwrap();
        let app_artifact = base.join("app.rkca");
        let app_object = base.join("app.o");
        fs::write(&app_object, []).unwrap();

        let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
        dep_products.crate_identity.source_fingerprint.source_hash = Some("dep-v1".to_string());
        dep_products.write_artifact_to_path(&dep_artifact).unwrap();

        let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
        let mut expected_identity = ProductCrateIdentity::local("dep".to_string());
        expected_identity.source_fingerprint.source_hash = Some("dep-v2".to_string());
        app_products
            .identity_table
            .dependencies
            .insert(ProductCrateId(1), expected_identity);
        app_products.write_artifact_to_path(&app_artifact).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(dep_artifact).unwrap();
        let err = ctx
            .load_product_artifact_from_path(app_artifact)
            .unwrap_err();

        assert!(
            err.contains("dependency identity mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_remaps_generic_body_ids_with_interface_ids() {
        let (base, _cleanup) = temp_test_dir("generic_body_ids");
        let artifact_path = write_product_fixture(&base, "dep");

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        let interface_id = dep
            .metadata()
            .interface()
            .function_by_canonical_name("dep::answer")
            .expect("function interface should exist")
            .id;
        let body_id = dep
            .body_providers()
            .generic_function(interface_id)
            .expect("generic function body should exist")
            .id;

        assert_eq!(interface_id, body_id);
        assert_ne!(body_id.crate_id, CrateId(0));
    }

    #[test]
    fn product_artifact_generic_param_descriptor_round_trip_remaps_owners() {
        let (base, _cleanup) = temp_test_dir("generic_param_owner_remap");
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let function_def = DefId::new(CrateId(0), LocalDefId(0));
        let generic = crate::types::GenericParamId {
            owner: function_def,
            index: 0,
        };
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let mut function = products.bodies.functions[&function_id].clone();
        function.generic_params = vec![GenericParamDecl::type_param(generic, "T")];
        function.params = vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(generic),
            mutable: false,
            is_ref: false,
        }];
        function.ret_type = Type::Generic(generic);
        function.body.ty = Type::Generic(generic);
        insert_function_body(&mut products, function_id, function);
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        let interface_function = dep
            .metadata()
            .interface()
            .function_by_canonical_name("dep::answer")
            .expect("function interface should exist");
        let expected_generic = GenericParamDecl::new(
            GenericParamId {
                owner: interface_function.id,
                index: 0,
            },
            "T",
            Kind::Type,
        );
        assert_eq!(
            interface_function.generic_params,
            vec![expected_generic.clone()]
        );
        let body_function = dep
            .body_providers()
            .generic_function(interface_function.id)
            .expect("generic function body should exist");
        assert_eq!(body_function.generic_params, vec![expected_generic]);
        let Type::Generic(param) = body_function.params[0].ty else {
            panic!("expected remapped generic param in artifact body");
        };

        assert_eq!(param.owner, interface_function.id);
        assert_ne!(param.owner.crate_id, CrateId(0));
    }

    #[test]
    fn product_artifact_remaps_function_generic_bounds() {
        let (base, _cleanup) = temp_test_dir("function_generic_bounds_remap");
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let trait_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(1),
        };
        let function_def = DefId::new(CrateId(0), LocalDefId(0));
        let trait_def = DefId::new(CrateId(0), LocalDefId(1));
        let generic = crate::types::GenericParamId {
            owner: function_def,
            index: 0,
        };
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products
            .identity_table
            .display_names
            .insert(trait_id, "dep::Bound".to_string());
        insert_interface_trait(
            &mut products,
            trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_def,
                name: "Bound".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );

        let mut function = products.bodies.functions[&function_id].clone();
        function.generic_params = vec![GenericParamDecl::type_param(generic, "T")];
        function.params = vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(generic),
            mutable: false,
            is_ref: false,
        }];
        function.generic_bounds.insert(
            generic,
            vec![crate::types::TraitBound {
                trait_id: trait_def,
                type_args: vec![Type::Generic(generic)],
            }],
        );
        insert_function_body(&mut products, function_id, function);
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        let interface_function = dep
            .metadata()
            .interface()
            .function_by_canonical_name("dep::answer")
            .expect("function interface should exist");
        let body_function = dep
            .body_providers()
            .generic_function(interface_function.id)
            .expect("generic function body should exist");
        let remapped_generic = crate::types::GenericParamId {
            owner: interface_function.id,
            index: 0,
        };
        let bounds = body_function
            .generic_bounds
            .get(&remapped_generic)
            .expect("generic bound key should be remapped to interface function ID");

        assert_eq!(bounds[0].trait_id.local, LocalDefId(1));
        assert_ne!(bounds[0].trait_id.crate_id, CrateId(0));
        assert_eq!(bounds[0].trait_id.crate_id, interface_function.id.crate_id);
        assert_eq!(bounds[0].type_args, vec![Type::Generic(remapped_generic)]);
    }

    #[test]
    fn product_artifact_remaps_signature_generic_bounds() {
        let (base, _cleanup) = temp_test_dir("signature_generic_bounds_remap");
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let trait_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(1),
        };
        let trait_def = DefId::new(CrateId(0), LocalDefId(1));
        let generic = crate::types::GenericParamId {
            owner: trait_def,
            index: 0,
        };
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products
            .identity_table
            .display_names
            .insert(trait_id, "dep::Bound".to_string());

        let mut signature_bounds = HashMap::new();
        signature_bounds.insert(
            generic,
            vec![crate::types::TraitBound {
                trait_id: trait_def,
                type_args: vec![Type::Generic(generic)],
            }],
        );
        insert_interface_trait(
            &mut products,
            trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_def,
                name: "Bound".to_string(),
                generic_params: generic_params(trait_def, &["T"]),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "value".to_string(),
                    HirFunctionSig {
                        id: DefId::new(CrateId(0), LocalDefId(31)),
                        name: "value".to_string(),
                        generic_params: vec![GenericParamDecl::type_param(generic, "T")],
                        params: vec![Type::Generic(generic)],
                        ret: Type::Generic(generic),
                        generic_bounds: signature_bounds.into(),
                        self_receiver: None,
                        is_unsafe: false,
                    },
                )]),
            },
        );
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        let loaded_trait = dep
            .metadata()
            .interface()
            .trait_by_canonical_name("dep::Bound")
            .expect("trait interface should exist");
        let signature = &loaded_trait.signatures["value"];
        let remapped_generic = crate::types::GenericParamId {
            owner: loaded_trait.id,
            index: 0,
        };
        let bounds = signature
            .generic_bounds
            .get(&remapped_generic)
            .expect("signature generic bound key should be remapped to trait ID");

        assert_eq!(bounds[0].trait_id, loaded_trait.id);
        assert_eq!(bounds[0].type_args, vec![Type::Generic(remapped_generic)]);
    }

    #[test]
    fn load_product_artifact_remaps_child_locations_in_generic_body() {
        let (base, _cleanup) = temp_test_dir("generic_body_child_locations");
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let struct_product_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(5),
        };
        let enum_product_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(6),
        };
        let struct_def = DefId::new(CrateId(0), LocalDefId(5));
        let enum_def = DefId::new(CrateId(0), LocalDefId(6));

        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let mut function = products.bodies.functions[&function_id].clone();
        function.body.stmts = vec![
            HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::FieldAccess(
                    Box::new(HirExpr {
                        kind: HirExprKindFor::Var("widget".to_string()),
                        ty: Type::Struct {
                            id: struct_def,
                            args: Vec::new(),
                        },
                        span: Span::test(),
                    }),
                    "value".to_string(),
                    Some(HirFieldLocation {
                        owner: struct_def,
                        field_id: FieldId(0),
                        name: "value".to_string(),
                    }),
                ),
                ty: Type::I64,
                span: Span::test(),
            }),
            HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::EnumVariant(
                    "dep::Choice".to_string(),
                    "Some".to_string(),
                    Vec::new(),
                    Some(HirVariantLocation {
                        owner: enum_def,
                        variant_id: VariantId(0),
                        name: "Some".to_string(),
                    }),
                ),
                ty: Type::Enum {
                    id: enum_def,
                    args: Vec::new(),
                },
                span: Span::test(),
            }),
        ];
        products.bodies.functions.insert(function_id, function);
        products
            .identity_table
            .display_names
            .insert(struct_product_id, "dep::Widget".to_string());
        products
            .identity_table
            .display_names
            .insert(enum_product_id, "dep::Choice".to_string());
        insert_interface_struct(
            &mut products,
            struct_product_id,
            HirStruct {
                id: struct_def,
                name: "dep::Widget".to_string(),
                generic_params: Vec::new(),
                fields: vec![HirField {
                    id: FieldId(0),
                    name: "value".to_string(),
                    ty: Type::I64,
                    public: true,
                }],
            },
        );
        insert_interface_enum(
            &mut products,
            enum_product_id,
            HirEnum {
                id: enum_def,
                name: "dep::Choice".to_string(),
                generic_params: Vec::new(),
                variants: vec![HirVariant {
                    id: VariantId(0),
                    name: "Some".to_string(),
                    fields: HirVariantFields::Unit,
                }],
            },
        );
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        let interface = dep.metadata().interface();
        let function_id = interface
            .function_by_canonical_name("dep::answer")
            .expect("function interface should exist")
            .id;
        let body = dep
            .body_providers()
            .generic_function(function_id)
            .expect("generic function body should exist");
        let struct_owner = interface
            .struct_by_canonical_name("dep::Widget")
            .expect("struct interface should exist")
            .id;
        let enum_owner = interface
            .enum_by_canonical_name("dep::Choice")
            .expect("enum interface should exist")
            .id;
        assert_ne!(struct_owner.crate_id, CrateId(0));
        assert_ne!(enum_owner.crate_id, CrateId(0));

        match &body.body.stmts[0] {
            HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::FieldAccess(_, _, Some(location)),
                ..
            }) => assert_eq!(location.owner, struct_owner),
            other => panic!("expected remapped field location, got {other:?}"),
        }
        match &body.body.stmts[1] {
            HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::EnumVariant(_, _, _, Some(location)),
                ..
            }) => assert_eq!(location.owner, enum_owner),
            other => panic!("expected remapped variant location, got {other:?}"),
        }
    }

    #[test]
    fn product_artifact_remaps_nominal_type_ids_in_hir_types() {
        let (base, _cleanup) = temp_test_dir("nominal_type_ids");
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let struct_product_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(5),
        };
        let enum_product_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(6),
        };
        let struct_def = DefId::new(CrateId(0), LocalDefId(5));
        let enum_def = DefId::new(CrateId(0), LocalDefId(6));

        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let mut function = products.bodies.functions[&function_id].clone();
        function.params.push(crate::hir::HirParam {
            name: "widget".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Struct {
                id: struct_def,
                args: vec![Type::Enum {
                    id: enum_def,
                    args: Vec::new(),
                }],
            },
            mutable: false,
            is_ref: false,
        });
        function.body.stmts = vec![
            HirStmt::Let {
                name: "local".to_string(),
                local_id: crate::ids::HirLocalId(1),
                ty: Type::Struct {
                    id: struct_def,
                    args: vec![Type::Enum {
                        id: enum_def,
                        args: Vec::new(),
                    }],
                },
                value: HirExpr {
                    kind: HirExprKindFor::Var("widget".to_string()),
                    ty: Type::Struct {
                        id: struct_def,
                        args: vec![Type::Enum {
                            id: enum_def,
                            args: Vec::new(),
                        }],
                    },
                    span: Span::test(),
                },
                mutable: false,
            },
            HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::Match {
                    scrutinee: Box::new(HirExpr {
                        kind: HirExprKindFor::Var("choice".to_string()),
                        ty: Type::Enum {
                            id: enum_def,
                            args: Vec::new(),
                        },
                        span: Span::test(),
                    }),
                    arms: vec![HirMatchArm {
                        pattern: HirPattern::Or(vec![
                            HirPattern::Struct(
                                "dep::Widget".to_string(),
                                None,
                                vec![Type::Enum {
                                    id: enum_def,
                                    args: Vec::new(),
                                }],
                                vec![HirStructPatternField {
                                    name: "field".to_string(),
                                    field: None,
                                    pattern: HirPattern::Struct(
                                        "dep::Nested".to_string(),
                                        None,
                                        vec![Type::Struct {
                                            id: struct_def,
                                            args: Vec::new(),
                                        }],
                                        Vec::new(),
                                    ),
                                }],
                            ),
                            HirPattern::Enum(
                                "dep::Choice".to_string(),
                                "Some".to_string(),
                                None,
                                vec![HirPattern::Tuple(vec![HirPattern::Struct(
                                    "dep::Widget".to_string(),
                                    None,
                                    vec![Type::Struct {
                                        id: struct_def,
                                        args: Vec::new(),
                                    }],
                                    Vec::new(),
                                )])],
                            ),
                        ]),
                        guard: None,
                        body: HirBlock {
                            stmts: vec![HirStmt::Expr(HirExpr {
                                kind: HirExprKindFor::IntLiteral(1),
                                ty: Type::I64,
                                span: Span::test(),
                            })],
                            ty: Type::I64,
                        },
                    }],
                },
                ty: Type::I64,
                span: Span::test(),
            }),
            HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::Lambda {
                    params: vec![HirParam {
                        name: "captured_widget".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Struct {
                            id: struct_def,
                            args: Vec::new(),
                        },
                        mutable: false,
                        is_ref: false,
                    }],
                    body: HirBlock {
                        stmts: Vec::new(),
                        ty: Type::Enum {
                            id: enum_def,
                            args: Vec::new(),
                        },
                    },
                    captures: vec![HirClosureCapture {
                        name: "widget".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        kind: HirClosureCaptureKind::SharedBorrow,
                        mutable: false,
                        ty: Type::Struct {
                            id: struct_def,
                            args: vec![Type::Enum {
                                id: enum_def,
                                args: Vec::new(),
                            }],
                        },
                    }],
                },
                ty: Type::function(
                    vec![Type::Struct {
                        id: struct_def,
                        args: Vec::new(),
                    }],
                    Type::Enum {
                        id: enum_def,
                        args: Vec::new(),
                    },
                ),
                span: Span::test(),
            }),
        ];
        function.ret_type = Type::Enum {
            id: enum_def,
            args: Vec::new(),
        };
        insert_function_body(&mut products, function_id, function);
        products
            .identity_table
            .display_names
            .insert(struct_product_id, "dep::Widget".to_string());
        products
            .identity_table
            .display_names
            .insert(enum_product_id, "dep::Choice".to_string());
        insert_interface_struct(
            &mut products,
            struct_product_id,
            HirStruct {
                id: struct_def,
                name: "dep::Widget".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        );
        insert_interface_enum(
            &mut products,
            enum_product_id,
            HirEnum {
                id: enum_def,
                name: "dep::Choice".to_string(),
                generic_params: Vec::new(),
                variants: Vec::new(),
            },
        );
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        let interface = dep.metadata().interface();
        let function_id = interface
            .function_by_canonical_name("dep::answer")
            .expect("function interface should exist")
            .id;
        let body = dep
            .body_providers()
            .generic_function(function_id)
            .expect("generic function body should exist");
        let struct_owner = interface
            .struct_by_canonical_name("dep::Widget")
            .expect("struct interface should exist")
            .id;
        let enum_owner = interface
            .enum_by_canonical_name("dep::Choice")
            .expect("enum interface should exist")
            .id;

        assert_eq!(
            body.params[0].ty,
            Type::Struct {
                id: struct_owner,
                args: vec![Type::Enum {
                    id: enum_owner,
                    args: Vec::new(),
                }],
            }
        );
        assert_eq!(
            body.ret_type,
            Type::Enum {
                id: enum_owner,
                args: Vec::new(),
            }
        );
        match &body.body.stmts[0] {
            HirStmt::Let { ty, .. } => assert_eq!(
                ty,
                &Type::Struct {
                    id: struct_owner,
                    args: vec![Type::Enum {
                        id: enum_owner,
                        args: Vec::new(),
                    }],
                }
            ),
            other => panic!("expected remapped let type, got {other:?}"),
        }
        let HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::Match { arms, .. },
            ..
        }) = &body.body.stmts[1]
        else {
            panic!("expected match expression");
        };
        let HirPattern::Or(or_patterns) = &arms[0].pattern else {
            panic!("expected or pattern");
        };
        let HirPattern::Struct(_, _, struct_args, struct_fields) = &or_patterns[0] else {
            panic!("expected struct pattern");
        };
        assert_eq!(
            struct_args,
            &vec![Type::Enum {
                id: enum_owner,
                args: Vec::new(),
            }]
        );
        let HirPattern::Struct(_, _, nested_args, _) = &struct_fields[0].pattern else {
            panic!("expected nested struct pattern");
        };
        assert_eq!(
            nested_args,
            &vec![Type::Struct {
                id: struct_owner,
                args: Vec::new(),
            }]
        );
        let HirPattern::Enum(_, _, _, enum_payloads) = &or_patterns[1] else {
            panic!("expected enum pattern");
        };
        let HirPattern::Tuple(tuple_patterns) = &enum_payloads[0] else {
            panic!("expected tuple pattern");
        };
        let HirPattern::Struct(_, _, enum_nested_args, _) = &tuple_patterns[0] else {
            panic!("expected nested enum payload pattern");
        };
        assert_eq!(
            enum_nested_args,
            &vec![Type::Struct {
                id: struct_owner,
                args: Vec::new(),
            }]
        );
        let HirStmt::Expr(HirExpr {
            kind:
                HirExprKindFor::Lambda {
                    params,
                    body,
                    captures,
                },
            ty,
            ..
        }) = &body.body.stmts[2]
        else {
            panic!("expected lambda expression");
        };
        assert_eq!(
            params[0].ty,
            Type::Struct {
                id: struct_owner,
                args: Vec::new(),
            }
        );
        assert_eq!(
            captures[0].ty,
            Type::Struct {
                id: struct_owner,
                args: vec![Type::Enum {
                    id: enum_owner,
                    args: Vec::new(),
                }],
            }
        );
        assert_eq!(
            body.ty,
            Type::Enum {
                id: enum_owner,
                args: Vec::new(),
            }
        );
        assert_eq!(
            *ty,
            Type::function(
                vec![Type::Struct {
                    id: struct_owner,
                    args: Vec::new(),
                }],
                Type::Enum {
                    id: enum_owner,
                    args: Vec::new(),
                },
            )
        );
    }

    #[test]
    fn load_product_artifact_rejects_unknown_nominal_type_id() {
        let (base, _cleanup) = temp_test_dir("unknown_nominal_type_id");
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let mut function = products.bodies.functions[&function_id].clone();
        function.params.push(HirParam {
            name: "ghost".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(999)),
                args: Vec::new(),
            },
            mutable: false,
            is_ref: false,
        });
        insert_function_body(&mut products, function_id, function);
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .expect_err("unknown nominal type ID should be rejected during artifact load");

        assert!(
            err.contains("unknown nominal type definition"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_invalid_projection_type_ids() {
        let (base, _cleanup) = temp_test_dir("invalid_projection_type_ids");
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let trait_product_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(4),
        };
        let trait_id = DefId::new(CrateId(0), LocalDefId(4));
        insert_interface_trait(
            &mut products,
            trait_product_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "dep::Deref".to_string(),
                generic_params: Vec::new(),
                associated_types: vec![HirAssociatedTypeDecl {
                    id: AssocTypeId(0),
                    name: "Target".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                }],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );

        let function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let mut function = products.bodies.functions[&function_id].clone();
        function.params.push(HirParam {
            name: "bad".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Projection {
                ty: Box::new(Type::I64),
                trait_id,
                assoc_type: AssociatedTypeKey {
                    owner: trait_id,
                    assoc_type_id: AssocTypeId(99),
                },
                trait_args: Vec::new(),
            },
            mutable: false,
            is_ref: false,
        });
        insert_function_body(&mut products, function_id, function);
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .unwrap_err();

        assert!(
            err.contains("unknown associated type ID"),
            "unexpected error: {err}"
        );
    }

    fn product_with_projection_trait(
        assoc_type_id: AssocTypeId,
    ) -> (
        CompilerProducts,
        super::ProductIdentityRemap,
        super::ProductNominalTypeValidator,
    ) {
        let trait_product_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(4),
        };
        let trait_def_id = DefId::new(CrateId(0), LocalDefId(4));
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        insert_interface_trait(
            &mut products,
            trait_product_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_def_id,
                name: "dep::Deref".to_string(),
                generic_params: Vec::new(),
                associated_types: vec![HirAssociatedTypeDecl {
                    id: assoc_type_id,
                    name: "Target".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                }],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );

        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        (products, remap, validator)
    }

    #[test]
    fn product_artifact_remaps_projection_type_ids() {
        let (_products, remap, validator) = product_with_projection_trait(AssocTypeId(0));
        let mut ty = Type::Projection {
            ty: Box::new(Type::I64),
            trait_id: DefId::new(CrateId(0), LocalDefId(4)),
            assoc_type: AssociatedTypeKey {
                owner: DefId::new(CrateId(0), LocalDefId(4)),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: Vec::new(),
        };

        super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap();

        assert_eq!(
            ty,
            Type::Projection {
                ty: Box::new(Type::I64),
                trait_id: DefId::new(CrateId(7), LocalDefId(4)),
                assoc_type: AssociatedTypeKey {
                    owner: DefId::new(CrateId(7), LocalDefId(4)),
                    assoc_type_id: AssocTypeId(0),
                },
                trait_args: Vec::new(),
            }
        );
    }

    #[test]
    fn product_artifact_rejects_unknown_projection_trait_id() {
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        products.interface.traits.clear();
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
        let mut ty = Type::Projection {
            ty: Box::new(Type::I64),
            trait_id: DefId::new(CrateId(0), LocalDefId(4)),
            assoc_type: AssociatedTypeKey {
                owner: DefId::new(CrateId(0), LocalDefId(4)),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: Vec::new(),
        };

        let err = super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap_err();

        assert!(
            err.contains("unknown trait definition"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_error_type_in_executable_body() {
        let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
        let mut expr = HirExpr {
            kind: HirExprKindFor::IntLiteral(1),
            ty: Type::Error,
            span: Span::test(),
        };

        let error = super::remap_expr_child_locations(
            &mut expr,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(
            error.contains("unresolved type"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn product_artifact_rejects_error_type_in_impl_receiver_metadata() {
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let impl_id = product_def_id(0, 7);
        insert_interface_impl(
            &mut products,
            impl_id,
            HirImpl {
                id: product_hir_def_id(impl_id),
                owner: HirImplOwner::Named("Broken".to_string()),
                type_name: "Broken".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Error),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::new(),
            },
        );

        let error = super::validate_product_method_authority_maps(
            &products,
            &super::ProductDependencyDefinitions::default(),
        )
        .unwrap_err();

        assert!(
            error.contains("unresolved type"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn product_artifact_rejects_unknown_projection_assoc_type_id() {
        let (_products, remap, validator) = product_with_projection_trait(AssocTypeId(0));
        let mut ty = Type::Projection {
            ty: Box::new(Type::I64),
            trait_id: DefId::new(CrateId(0), LocalDefId(4)),
            assoc_type: AssociatedTypeKey {
                owner: DefId::new(CrateId(0), LocalDefId(4)),
                assoc_type_id: AssocTypeId(9),
            },
            trait_args: Vec::new(),
        };

        let err = super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap_err();

        assert!(
            err.contains("unknown associated type ID"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_projection_assoc_owner_mismatch_across_crates() {
        let (_products, mut remap, validator) = product_with_projection_trait(AssocTypeId(0));
        remap.crate_ids.insert(ProductCrateId(1), CrateId(8));
        let mut ty = Type::Projection {
            ty: Box::new(Type::I64),
            trait_id: DefId::new(CrateId(0), LocalDefId(4)),
            assoc_type: AssociatedTypeKey {
                owner: DefId::new(CrateId(1), LocalDefId(4)),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: Vec::new(),
        };

        let err = super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap_err();

        assert!(
            err.contains("does not match associated type owner"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_unknown_dependency_projection_trait_id() {
        let (_products, mut remap, validator) = product_with_projection_trait(AssocTypeId(0));
        remap.crate_ids.insert(ProductCrateId(1), CrateId(8));
        let mut ty = Type::Projection {
            ty: Box::new(Type::I64),
            trait_id: DefId::new(CrateId(1), LocalDefId(44)),
            assoc_type: AssociatedTypeKey {
                owner: DefId::new(CrateId(1), LocalDefId(44)),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: Vec::new(),
        };

        let err = super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap_err();

        assert!(
            err.contains("unknown dependency trait definition"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_unknown_method_call_target_impl_id() {
        let mut expr = HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                "value".to_string(),
                Vec::new(),
                None,
                HirMethodCallTarget::impl_method(
                    DefId::new(CrateId(0), LocalDefId(99)),
                    DefId::new(CrateId(0), LocalDefId(0)),
                    None,
                ),
            ),
            ty: Type::I64,
            span: Span::test(),
        };
        let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        let err = super::remap_expr_child_locations(
            &mut expr,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(
            err.contains("unknown impl target"),
            "unexpected error: {err}"
        );
    }

    fn validate_artifact_expr(
        products: &CompilerProducts,
        expr: &mut HirExpr,
    ) -> Result<(), String> {
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(products);
        let type_validator = super::ProductNominalTypeValidator::from_products(products, &remap);
        super::remap_expr_child_locations(expr, &remap, &child_validator, &type_validator, 0)
    }

    fn trait_method_fixture(
        generic_params: Vec<GenericParamDecl>,
        self_receiver: Option<crate::types::ReceiverMode>,
    ) -> (CompilerProducts, DefId, DefId) {
        let trait_product_id = product_def_id(0, 30);
        let trait_id = product_hir_def_id(trait_product_id);
        let member_id = DefId::new(CrateId(0), LocalDefId(31));
        let mut member = test_function(member_id, "value");
        member.is_method = self_receiver.is_some();
        member.self_receiver = self_receiver;
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        insert_interface_trait(
            &mut products,
            trait_product_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "dep::Trait".to_string(),
                generic_params,
                associated_types: Vec::new(),
                methods: HashMap::from([("value".to_string(), member)]),
                signatures: HashMap::new(),
            },
        );
        (products, trait_id, member_id)
    }

    #[test]
    fn product_artifact_load_rejects_receiver_trait_member_mapped_to_static_impl_method() {
        let (mut products, trait_id, member_id) =
            trait_method_fixture(Vec::new(), Some(crate::types::ReceiverMode::Move));
        let impl_product_id = product_def_id(0, 40);
        let impl_id = product_hir_def_id(impl_product_id);
        let method_id = DefId::new(CrateId(0), LocalDefId(41));
        insert_interface_impl(
            &mut products,
            impl_product_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
                trait_name: Some("Trait".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([(
                    "different_source_name".to_string(),
                    test_function(method_id, "different_source_name"),
                )]),
            },
        );
        products.interface.effective_trait_methods.insert(
            (impl_product_id, ProductDefId::from(member_id)),
            ProductDefId::from(method_id),
        );
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };

        let error =
            super::cross_crate_hir_from_products(&products, "dep", &remap, &CrateContext::new())
                .expect_err("malformed effective trait-member relation must fail artifact loading");

        assert!(
            error.contains("effective trait member")
                && error.contains(&format!("{member_id:?}"))
                && error.contains(&format!("{method_id:?}"))
                && error.contains("receiver/static mismatch"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn product_artifact_rejects_trait_method_target_with_missing_trait_argument() {
        let (products, trait_id, member_id) = trait_method_fixture(
            generic_params(DefId::new(CrateId(0), LocalDefId(30)), &["T"]),
            Some(crate::types::ReceiverMode::Move),
        );
        let mut expr = HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                "value".to_string(),
                Vec::new(),
                Some(crate::types::ReceiverMode::Move),
                HirMethodCallTarget::trait_method(
                    trait_id,
                    member_id,
                    Vec::new(),
                    crate::hir::HirTraitDispatchKind::TraitBound,
                ),
            ),
            ty: Type::I64,
            span: Span::test(),
        };

        let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

        assert!(
            error.contains("trait argument arity"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn product_artifact_rejects_impl_target_with_inconsistent_trait_arguments() {
        let (mut products, trait_id, member_id) = trait_method_fixture(
            generic_params(DefId::new(CrateId(0), LocalDefId(30)), &["T"]),
            Some(crate::types::ReceiverMode::Move),
        );
        let impl_product_id = product_def_id(0, 40);
        let impl_id = product_hir_def_id(impl_product_id);
        let method_id = DefId::new(CrateId(0), LocalDefId(41));
        let mut method = test_function(method_id, "value");
        method.is_method = true;
        method.self_receiver = Some(crate::types::ReceiverMode::Move);
        insert_interface_impl(
            &mut products,
            impl_product_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
                trait_name: Some("Trait".to_string()),
                trait_id: Some(trait_id),
                trait_generics: generic_params(trait_id, &["T"]),
                trait_arg_types: vec![Type::I64],
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), method)]),
            },
        );
        products.interface.effective_trait_methods.insert(
            (impl_product_id, ProductDefId::from(member_id)),
            ProductDefId::from(method_id),
        );
        let mut expr = HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                "value".to_string(),
                Vec::new(),
                Some(crate::types::ReceiverMode::Move),
                HirMethodCallTarget::impl_method(
                    impl_id,
                    method_id,
                    Some(crate::hir::HirSelectedTraitMember {
                        trait_id,
                        member_id,
                        trait_args: vec![Type::Bool],
                    }),
                ),
            ),
            ty: Type::I64,
            span: Span::test(),
        };

        let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

        assert!(
            error.contains("trait arguments do not match impl"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn product_artifact_rejects_trait_impl_target_without_selected_trait_identity() {
        let (mut products, trait_id, member_id) =
            trait_method_fixture(Vec::new(), Some(crate::types::ReceiverMode::Move));
        let impl_product_id = product_def_id(0, 40);
        let impl_id = product_hir_def_id(impl_product_id);
        let method_id = DefId::new(CrateId(0), LocalDefId(41));
        let mut method = test_function(method_id, "value");
        method.is_method = true;
        method.self_receiver = Some(crate::types::ReceiverMode::Move);
        insert_interface_impl(
            &mut products,
            impl_product_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
                trait_name: Some("Trait".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), method)]),
            },
        );
        products.interface.effective_trait_methods.insert(
            (impl_product_id, ProductDefId::from(member_id)),
            ProductDefId::from(method_id),
        );
        let mut expr = HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                "value".to_string(),
                Vec::new(),
                Some(crate::types::ReceiverMode::Move),
                HirMethodCallTarget::impl_method(impl_id, method_id, None),
            ),
            ty: Type::I64,
            span: Span::test(),
        };

        let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

        assert!(
            error.contains("has no selected trait identity"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn product_artifact_rejects_method_call_authority_for_static_method() {
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let impl_product_id = product_def_id(0, 40);
        let impl_id = product_hir_def_id(impl_product_id);
        let method_id = DefId::new(CrateId(0), LocalDefId(41));
        insert_interface_impl(
            &mut products,
            impl_product_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("make".to_string(), test_function(method_id, "make"))]),
            },
        );
        let mut expr = HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                "make".to_string(),
                Vec::new(),
                None,
                HirMethodCallTarget::impl_method(impl_id, method_id, None),
            ),
            ty: Type::I64,
            span: Span::test(),
        };

        let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

        assert!(
            error.contains("references a static method"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn product_artifact_rejects_static_authority_for_receiver_method() {
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let impl_product_id = product_def_id(0, 40);
        let impl_id = product_hir_def_id(impl_product_id);
        let method_id = DefId::new(CrateId(0), LocalDefId(41));
        let mut method = test_function(method_id, "value");
        method.is_method = true;
        method.self_receiver = Some(crate::types::ReceiverMode::Move);
        insert_interface_impl(
            &mut products,
            impl_product_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), method)]),
            },
        );
        let mut expr = HirExpr {
            kind: HirExprKindFor::Call(
                Box::new(HirExpr {
                    kind: HirExprKindFor::Var("value".to_string()),
                    ty: Type::function(Vec::new(), Type::I64),
                    span: Span::test(),
                }),
                Vec::new(),
                Some(crate::hir::HirCallTarget::StaticMethod(
                    crate::hir::HirStaticMethodTarget {
                        owner_ty: Type::I64,
                        method: HirMethodCallTarget::impl_method(impl_id, method_id, None),
                    },
                )),
            ),
            ty: Type::I64,
            span: Span::test(),
        };

        let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

        assert!(
            error.contains("references a receiver method"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn product_artifact_rejects_malformed_trait_method_authority_with_substitutions() {
        let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let mut target = HirMethodCallTarget::trait_method(
            DefId::new(CrateId(0), LocalDefId(999)),
            DefId::new(CrateId(0), LocalDefId(998)),
            Vec::new(),
            crate::hir::HirTraitDispatchKind::TraitBound,
        );
        target.owner_substitution.push(crate::hir::HirTypeBinding {
            param: crate::types::GenericParamId {
                owner: DefId::new(CrateId(0), LocalDefId(1)),
                index: 0,
            },
            ty: Type::I64,
        });
        let mut expr = HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::ArrayLiteral(Vec::new()),
                    ty: Type::Array(Box::new(Type::I64), 0),
                    span: Span::test(),
                }),
                "index".to_string(),
                vec![HirExpr {
                    kind: HirExprKindFor::IntLiteral(0),
                    ty: Type::I64,
                    span: Span::test(),
                }],
                None,
                target,
            ),
            ty: Type::I64,
            span: Span::test(),
        };

        let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

        assert!(
            error.contains("references unknown trait target"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn product_artifact_rejects_method_call_target_method_from_other_impl() {
        let first_impl_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(10),
        };
        let second_impl_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(20),
        };
        let first_method_id = DefId::new(CrateId(0), LocalDefId(11));
        let second_method_id = DefId::new(CrateId(0), LocalDefId(21));
        let mut first_method = test_function(first_method_id, "dep::Box_First_value");
        first_method.is_method = true;
        let mut second_method = test_function(second_method_id, "dep::Box_Second_value");
        second_method.is_method = true;

        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        insert_interface_impl(
            &mut products,
            first_impl_id,
            HirImpl {
                id: DefId::new(CrateId(0), LocalDefId(10)),
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), first_method)]),
            },
        );
        insert_interface_impl(
            &mut products,
            second_impl_id,
            HirImpl {
                id: DefId::new(CrateId(0), LocalDefId(20)),
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), second_method)]),
            },
        );

        let mut expr = HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                "value".to_string(),
                Vec::new(),
                None,
                HirMethodCallTarget::impl_method(
                    DefId::new(CrateId(0), LocalDefId(10)),
                    second_method_id,
                    None,
                ),
            ),
            ty: Type::I64,
            span: Span::test(),
        };
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        let err = super::remap_expr_child_locations(
            &mut expr,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(
            err.contains("method target") && err.contains("impl target"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_trait_target_on_inherent_impl_method_call() {
        let trait_product_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(30),
        };
        let impl_product_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(10),
        };
        let method_id = DefId::new(CrateId(0), LocalDefId(11));
        let mut method = test_function(method_id, "dep::Box_value");
        method.is_method = true;

        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        insert_interface_trait(
            &mut products,
            trait_product_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: DefId::new(CrateId(0), LocalDefId(30)),
                name: "dep::Show".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        insert_interface_impl(
            &mut products,
            impl_product_id,
            HirImpl {
                id: DefId::new(CrateId(0), LocalDefId(10)),
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), method)]),
            },
        );

        let mut expr = HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                "value".to_string(),
                Vec::new(),
                None,
                HirMethodCallTarget::impl_method(
                    DefId::new(CrateId(0), LocalDefId(10)),
                    method_id,
                    Some(crate::hir::HirSelectedTraitMember {
                        trait_id: DefId::new(CrateId(0), LocalDefId(30)),
                        member_id: method_id,
                        trait_args: Vec::new(),
                    }),
                ),
            ),
            ty: Type::I64,
            span: Span::test(),
        };
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        let err = super::remap_expr_child_locations(
            &mut expr,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(
            err.contains("impl target") && err.contains("trait target"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_method_call_target_method_from_other_trait() {
        let first_trait_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(30),
        };
        let second_trait_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(40),
        };
        let first_method_id = DefId::new(CrateId(0), LocalDefId(31));
        let second_method_id = DefId::new(CrateId(0), LocalDefId(41));
        let mut first_method = test_function(first_method_id, "dep::First_value");
        first_method.is_method = true;
        let mut second_method = test_function(second_method_id, "dep::Second_value");
        second_method.is_method = true;

        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        insert_interface_trait(
            &mut products,
            first_trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: DefId::new(CrateId(0), LocalDefId(30)),
                name: "dep::First".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::from([("value".to_string(), first_method)]),
                signatures: HashMap::new(),
            },
        );
        insert_interface_trait(
            &mut products,
            second_trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: DefId::new(CrateId(0), LocalDefId(40)),
                name: "dep::Second".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::from([("value".to_string(), second_method)]),
                signatures: HashMap::new(),
            },
        );

        let mut expr = HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                "value".to_string(),
                Vec::new(),
                None,
                HirMethodCallTarget::trait_method(
                    DefId::new(CrateId(0), LocalDefId(30)),
                    second_method_id,
                    Vec::new(),
                    crate::hir::HirTraitDispatchKind::TraitBound,
                ),
            ),
            ty: Type::I64,
            span: Span::test(),
        };
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        let err = super::remap_expr_child_locations(
            &mut expr,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(
            err.contains("does not own selected member"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_trait_signature_target_using_trait_id_sentinel() {
        let trait_product_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(30),
        };
        let trait_id = DefId::new(CrateId(0), LocalDefId(30));
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        insert_interface_trait(
            &mut products,
            trait_product_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "dep::Show".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "value".to_string(),
                    HirFunctionSig {
                        id: DefId::new(CrateId(0), LocalDefId(31)),
                        name: "value".to_string(),
                        generic_params: Vec::new(),
                        params: Vec::new(),
                        ret: Type::I64,
                        generic_bounds: HashMap::new().into(),
                        self_receiver: None,
                        is_unsafe: false,
                    },
                )]),
            },
        );

        let mut expr = HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                "value".to_string(),
                Vec::new(),
                None,
                HirMethodCallTarget::trait_method(
                    trait_id,
                    trait_id,
                    Vec::new(),
                    crate::hir::HirTraitDispatchKind::TraitBound,
                ),
            ),
            ty: Type::I64,
            span: Span::test(),
        };
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        let err = super::remap_expr_child_locations(
            &mut expr,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(
            err.contains("does not own selected member"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_unknown_resolved_var_function_target() {
        let mut expr = HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "missing".to_string(),
                target: HirVarTarget::Function(DefId::new(CrateId(0), LocalDefId(99))),
            }),
            ty: Type::I64,
            span: Span::test(),
        };
        let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        let err = super::remap_expr_child_locations(
            &mut expr,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(err.contains("unknown function"), "unexpected error: {err}");
    }

    #[test]
    fn product_artifact_rejects_resolved_var_function_target_to_extern() {
        let extern_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(7),
        };
        let extern_def = DefId::new(CrateId(0), LocalDefId(7));
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        insert_interface_extern(
            &mut products,
            extern_id,
            HirExtern {
                id: extern_def,
                name: "dep::foreign".to_string(),
                params: Vec::new(),
                ret: Type::I64,
                variadic: false,
                is_unsafe: false,
            },
        );
        let mut expr = HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "foreign".to_string(),
                target: HirVarTarget::Function(extern_def),
            }),
            ty: Type::I64,
            span: Span::test(),
        };
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        let err = super::remap_expr_child_locations(
            &mut expr,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(
            err.contains("function") && err.contains("extern"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_resolved_var_extern_target_to_function() {
        let function_def = DefId::new(CrateId(0), LocalDefId(0));
        let mut expr = HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "answer".to_string(),
                target: HirVarTarget::Extern(function_def),
            }),
            ty: Type::I64,
            span: Span::test(),
        };
        let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        let err = super::remap_expr_child_locations(
            &mut expr,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(
            err.contains("extern") && err.contains("function"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_unknown_struct_literal_sidecar_without_fields() {
        let missing_struct = DefId::new(CrateId(0), LocalDefId(99));
        let mut expr = HirExpr {
            kind: HirExprKindFor::StructLiteral(
                "Ghost".to_string(),
                Some(missing_struct),
                Vec::new(),
            ),
            ty: Type::I64,
            span: Span::test(),
        };
        let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        let err = super::remap_expr_child_locations(
            &mut expr,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(
            err.contains("unknown nominal type definition") && err.contains("as struct"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_unknown_struct_pattern_sidecar_without_fields() {
        let missing_struct = DefId::new(CrateId(0), LocalDefId(99));
        let mut pattern = HirPattern::Struct(
            "Ghost".to_string(),
            Some(missing_struct),
            Vec::new(),
            Vec::new(),
        );
        let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        let err = super::remap_pattern_child_locations(
            &mut pattern,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(
            err.contains("unknown nominal type definition") && err.contains("as struct"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_struct_literal_field_owner_mismatch() {
        let first_struct_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(5),
        };
        let second_struct_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(6),
        };
        let first_struct_def = DefId::new(CrateId(0), LocalDefId(5));
        let second_struct_def = DefId::new(CrateId(0), LocalDefId(6));
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        insert_interface_struct(
            &mut products,
            first_struct_id,
            HirStruct {
                id: first_struct_def,
                name: "dep::First".to_string(),
                generic_params: Vec::new(),
                fields: vec![HirField {
                    id: FieldId(0),
                    name: "value".to_string(),
                    ty: Type::I64,
                    public: true,
                }],
            },
        );
        insert_interface_struct(
            &mut products,
            second_struct_id,
            HirStruct {
                id: second_struct_def,
                name: "dep::Second".to_string(),
                generic_params: Vec::new(),
                fields: vec![HirField {
                    id: FieldId(0),
                    name: "value".to_string(),
                    ty: Type::I64,
                    public: true,
                }],
            },
        );
        let mut expr = HirExpr {
            kind: HirExprKindFor::StructLiteral(
                "First".to_string(),
                Some(first_struct_def),
                vec![crate::hir::HirStructLiteralFieldFor::<AcceptedHir> {
                    name: "value".to_string(),
                    value: HirExpr {
                        kind: HirExprKindFor::IntLiteral(1),
                        ty: Type::I64,
                        span: Span::test(),
                    },
                    field: Some(HirFieldLocation {
                        owner: second_struct_def,
                        field_id: FieldId(0),
                        name: "value".to_string(),
                    }),
                }],
            ),
            ty: Type::I64,
            span: Span::test(),
        };
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        let err = super::remap_expr_child_locations(
            &mut expr,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(
            err.contains("struct literal field owner mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_rejects_struct_pattern_field_owner_mismatch() {
        let first_struct_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(5),
        };
        let second_struct_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(6),
        };
        let first_struct_def = DefId::new(CrateId(0), LocalDefId(5));
        let second_struct_def = DefId::new(CrateId(0), LocalDefId(6));
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        insert_interface_struct(
            &mut products,
            first_struct_id,
            HirStruct {
                id: first_struct_def,
                name: "dep::First".to_string(),
                generic_params: Vec::new(),
                fields: vec![HirField {
                    id: FieldId(0),
                    name: "value".to_string(),
                    ty: Type::I64,
                    public: true,
                }],
            },
        );
        insert_interface_struct(
            &mut products,
            second_struct_id,
            HirStruct {
                id: second_struct_def,
                name: "dep::Second".to_string(),
                generic_params: Vec::new(),
                fields: vec![HirField {
                    id: FieldId(0),
                    name: "value".to_string(),
                    ty: Type::I64,
                    public: true,
                }],
            },
        );
        let mut pattern = HirPattern::Struct(
            "First".to_string(),
            Some(first_struct_def),
            Vec::new(),
            vec![HirStructPatternField {
                name: "value".to_string(),
                field: Some(HirFieldLocation {
                    owner: second_struct_def,
                    field_id: FieldId(0),
                    name: "value".to_string(),
                }),
                pattern: HirPattern::Wildcard,
            }],
        );
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        let err = super::remap_pattern_child_locations(
            &mut pattern,
            &remap,
            &child_validator,
            &type_validator,
            0,
        )
        .unwrap_err();

        assert!(
            err.contains("struct pattern field owner mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_accepts_dependency_resolved_var_function_target() {
        let (base, _cleanup) = temp_test_dir("dependency_resolved_var_function_target");
        let dep_artifact = base.join("dep.rkca");
        let dep_object = base.join("dep.o");
        fs::write(&dep_object, []).unwrap();
        let app_artifact = base.join("app.rkca");
        let app_object = base.join("app.o");
        fs::write(&app_object, []).unwrap();

        let dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
        dep_products.write_artifact_to_path(&dep_artifact).unwrap();

        let app_function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
        app_products.identity_table.dependencies.insert(
            ProductCrateId(1),
            ProductCrateIdentity::local("dep".to_string()),
        );
        let mut app_function = app_products.bodies.functions[&app_function_id].clone();
        app_function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::ResolvedVar(HirVarRef {
                    name: "dep::answer".to_string(),
                    target: HirVarTarget::Function(DefId::new(CrateId(1), LocalDefId(0))),
                }),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            })],
            ty: Type::function(Vec::new(), Type::I64),
        };
        app_products
            .bodies
            .functions
            .insert(app_function_id, app_function);
        app_products.write_artifact_to_path(&app_artifact).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(dep_artifact).unwrap();
        ctx.load_product_artifact_from_path(app_artifact)
            .expect("dependency function ResolvedVar targets should validate");
    }

    #[test]
    fn load_product_artifact_rejects_dependency_resolved_var_function_target_to_extern() {
        let (base, _cleanup) = temp_test_dir("dependency_resolved_var_wrong_kind");
        let dep_artifact = base.join("dep.rkca");
        let dep_object = base.join("dep.o");
        fs::write(&dep_object, []).unwrap();
        let app_artifact = base.join("app.rkca");
        let app_object = base.join("app.o");
        fs::write(&app_object, []).unwrap();

        let dep_extern_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(7),
        };
        let dep_extern_def = DefId::new(CrateId(0), LocalDefId(7));
        let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
        insert_interface_extern(
            &mut dep_products,
            dep_extern_id,
            HirExtern {
                id: dep_extern_def,
                name: "dep::foreign".to_string(),
                params: Vec::new(),
                ret: Type::I64,
                variadic: false,
                is_unsafe: false,
            },
        );
        dep_products.write_artifact_to_path(&dep_artifact).unwrap();

        let app_function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
        app_products.identity_table.dependencies.insert(
            ProductCrateId(1),
            ProductCrateIdentity::local("dep".to_string()),
        );
        let mut app_function = app_products.bodies.functions[&app_function_id].clone();
        app_function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::ResolvedVar(HirVarRef {
                    name: "dep::foreign".to_string(),
                    target: HirVarTarget::Function(DefId::new(CrateId(1), LocalDefId(7))),
                }),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            })],
            ty: Type::function(Vec::new(), Type::I64),
        };
        app_products
            .bodies
            .functions
            .insert(app_function_id, app_function);
        app_products.write_artifact_to_path(&app_artifact).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(dep_artifact).unwrap();
        let err = ctx
            .load_product_artifact_from_path(app_artifact)
            .expect_err("dependency extern target used as function should be rejected");

        assert!(
            err.contains("dependency extern") && err.contains("as function"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_accepts_dependency_resolved_var_extern_target() {
        let (base, _cleanup) = temp_test_dir("dependency_resolved_var_extern_target");
        let dep_artifact = base.join("dep.rkca");
        let dep_object = base.join("dep.o");
        fs::write(&dep_object, []).unwrap();
        let app_artifact = base.join("app.rkca");
        let app_object = base.join("app.o");
        fs::write(&app_object, []).unwrap();

        let dep_extern_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(7),
        };
        let dep_extern_def = DefId::new(CrateId(0), LocalDefId(7));
        let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
        insert_interface_extern(
            &mut dep_products,
            dep_extern_id,
            HirExtern {
                id: dep_extern_def,
                name: "dep::foreign".to_string(),
                params: Vec::new(),
                ret: Type::I64,
                variadic: false,
                is_unsafe: false,
            },
        );
        dep_products.write_artifact_to_path(&dep_artifact).unwrap();

        let app_function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
        app_products.identity_table.dependencies.insert(
            ProductCrateId(1),
            ProductCrateIdentity::local("dep".to_string()),
        );
        let mut app_function = app_products.bodies.functions[&app_function_id].clone();
        app_function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::ResolvedVar(HirVarRef {
                    name: "dep::foreign".to_string(),
                    target: HirVarTarget::Extern(DefId::new(CrateId(1), LocalDefId(7))),
                }),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            })],
            ty: Type::function(Vec::new(), Type::I64),
        };
        app_products
            .bodies
            .functions
            .insert(app_function_id, app_function);
        app_products.write_artifact_to_path(&app_artifact).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(dep_artifact).unwrap();
        ctx.load_product_artifact_from_path(app_artifact)
            .expect("dependency extern ResolvedVar targets should validate");
    }

    #[test]
    fn load_product_artifact_rejects_dependency_resolved_var_extern_target_to_function() {
        let (base, _cleanup) = temp_test_dir("dependency_resolved_var_extern_wrong_kind");
        let dep_artifact = base.join("dep.rkca");
        let dep_object = base.join("dep.o");
        fs::write(&dep_object, []).unwrap();
        let app_artifact = base.join("app.rkca");
        let app_object = base.join("app.o");
        fs::write(&app_object, []).unwrap();

        let dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
        dep_products.write_artifact_to_path(&dep_artifact).unwrap();

        let app_function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
        app_products.identity_table.dependencies.insert(
            ProductCrateId(1),
            ProductCrateIdentity::local("dep".to_string()),
        );
        let mut app_function = app_products.bodies.functions[&app_function_id].clone();
        app_function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::ResolvedVar(HirVarRef {
                    name: "dep::answer".to_string(),
                    target: HirVarTarget::Extern(DefId::new(CrateId(1), LocalDefId(0))),
                }),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            })],
            ty: Type::function(Vec::new(), Type::I64),
        };
        app_products
            .bodies
            .functions
            .insert(app_function_id, app_function);
        app_products.write_artifact_to_path(&app_artifact).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(dep_artifact).unwrap();
        let err = ctx
            .load_product_artifact_from_path(app_artifact)
            .expect_err("dependency function target used as extern should be rejected");

        assert!(
            err.contains("dependency function") && err.contains("as extern"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_accepts_resolved_var_function_display_name_mismatch() {
        let (base, _cleanup) = temp_test_dir("resolved_var_function_name_mismatch");
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let mut function = products.bodies.functions[&function_id].clone();
        function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::ResolvedVar(HirVarRef {
                    name: "not_answer".to_string(),
                    target: HirVarTarget::Function(DefId::new(CrateId(0), LocalDefId(0))),
                }),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            })],
            ty: Type::function(Vec::new(), Type::I64),
        };
        products.bodies.functions.insert(function_id, function);
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();
    }

    #[test]
    fn product_artifact_rejects_receiver_method_as_function_target() {
        let impl_id = product_def_id(0, 10);
        let method_id = product_def_id(0, 11);
        let method_def = product_hir_def_id(method_id);
        let mut method = test_function(method_def, "value");
        method.is_method = true;
        method.params.push(HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        });
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        insert_interface_impl(
            &mut products,
            impl_id,
            HirImpl {
                id: product_hir_def_id(impl_id),
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), method)]),
            },
        );
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
        let mut reference = HirVarRef {
            name: "Box_value".to_string(),
            target: HirVarTarget::Function(method_def),
        };

        let err = super::remap_var_target_child_location(
            &mut reference,
            &remap,
            &validator,
            &BTreeSet::new(),
        )
        .unwrap_err();

        assert!(
            err.contains("receiver method") && err.contains("as function"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_accepts_static_method_function_display_name_mismatch() {
        let impl_id = product_def_id(0, 12);
        let method_id = product_def_id(0, 13);
        let method_def = product_hir_def_id(method_id);
        let mut method = test_function(method_def, "make");
        method.is_method = false;
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        insert_interface_impl(
            &mut products,
            impl_id,
            HirImpl {
                id: product_hir_def_id(impl_id),
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("make".to_string(), method)]),
            },
        );
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
        let mut reference = HirVarRef {
            name: "not_make".to_string(),
            target: HirVarTarget::Function(method_def),
        };

        super::remap_var_target_child_location(
            &mut reference,
            &remap,
            &validator,
            &BTreeSet::new(),
        )
        .unwrap();
    }

    #[test]
    fn product_artifact_rejects_dependency_receiver_method_as_function_target() {
        let app_products =
            product_with_function("app", ProductCrateId(0), 0, PathBuf::from("app.o"));
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([
                (ProductCrateId(0), CrateId(7)),
                (ProductCrateId(1), CrateId(8)),
            ]),
        };
        let method_id = product_def_id(1, 11);
        let mut dependencies = super::ProductDependencyDefinitions::default();
        dependencies.methods.insert(method_id);
        let validator = super::ProductNominalTypeValidator::from_products_with_dependencies(
            &app_products,
            &remap,
            dependencies,
        );
        let mut reference = HirVarRef {
            name: "dep::Box_value".to_string(),
            target: HirVarTarget::Function(product_hir_def_id(method_id)),
        };

        let err = super::remap_var_target_child_location(
            &mut reference,
            &remap,
            &validator,
            &BTreeSet::new(),
        )
        .unwrap_err();

        assert!(
            err.contains("dependency receiver method") && err.contains("as function"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_artifact_allows_dependency_static_impl_method_canonical_name_as_function_target() {
        let app_products =
            product_with_function("app", ProductCrateId(0), 0, PathBuf::from("app.o"));
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([
                (ProductCrateId(0), CrateId(7)),
                (ProductCrateId(1), CrateId(8)),
            ]),
        };

        let dep_impl_id = DefId::new(CrateId(8), LocalDefId(10));
        let dep_method_id = DefId::new(CrateId(8), LocalDefId(11));
        let dep_method_product_id = product_def_id(1, 11);
        let dep_method = test_function(dep_method_id, "make");
        let dep_impl = HirImpl {
            id: dep_impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("make".to_string(), dep_method)]),
        };
        let mut interface = ArtifactCrateInterface::default();
        interface
            .canonical_names
            .insert(dep_method_id, "dep::Box::make".to_string());
        interface
            .impls
            .insert(dep_impl_id, ProductImplInterface::from(&dep_impl));
        let mut ctx = CrateContext::new();
        ctx.add_extern_crate(ExternCrateRecord::new(
            CrateId(8),
            "dep".to_string(),
            ExternCrateMetadata::new(
                interface,
                crate::collect::resolver::ResolverTables::default(),
                BTreeMap::new(),
            ),
            ExternCrateBodies::default(),
            ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .unwrap();

        let validator = super::ProductNominalTypeValidator::from_products_with_context(
            &app_products,
            &remap,
            &ctx,
        );
        let mut reference = HirVarRef {
            name: "dep::Box::make".to_string(),
            target: HirVarTarget::Function(product_hir_def_id(dep_method_product_id)),
        };

        super::remap_var_target_child_location(
            &mut reference,
            &remap,
            &validator,
            &BTreeSet::new(),
        )
        .unwrap();

        assert_eq!(
            reference.target,
            HirVarTarget::Function(DefId::new(CrateId(8), LocalDefId(11)))
        );
    }

    #[test]
    fn product_dependency_definitions_ignore_body_provider_generic_impls() {
        let app_products =
            product_with_function("app", ProductCrateId(0), 0, PathBuf::from("app.o"));
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([
                (ProductCrateId(0), CrateId(7)),
                (ProductCrateId(1), CrateId(8)),
            ]),
        };

        let dep_impl_id = DefId::new(CrateId(8), LocalDefId(10));
        let dep_method_id = DefId::new(CrateId(8), LocalDefId(11));
        let mut dep_method = test_function(dep_method_id, "value");
        dep_method.is_method = true;
        let dep_impl = HirImpl {
            id: dep_impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: generic_params(dep_impl_id, &["T"]),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Generic(
                crate::types::GenericParamId {
                    owner: dep_impl_id,
                    index: 0,
                },
            )),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), dep_method)]),
        };
        let bodies = ExternCrateBodies::from_cross_crate_hir(ArtifactCrossCrateHir {
            generic_functions: BTreeMap::new(),
            traits_with_defaults: BTreeMap::new(),
            generic_impls: vec![dep_impl],
        });
        let mut ctx = CrateContext::new();
        ctx.add_extern_crate(ExternCrateRecord::new(
            CrateId(8),
            "dep".to_string(),
            ExternCrateMetadata::new(
                ArtifactCrateInterface::default(),
                crate::collect::resolver::ResolverTables::default(),
                BTreeMap::new(),
            ),
            bodies,
            ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .unwrap();

        let validator = super::ProductChildLocationValidator::from_products_with_context(
            &app_products,
            &remap,
            &ctx,
        );
        let type_validator = super::ProductNominalTypeValidator::from_products_with_context(
            &app_products,
            &remap,
            &ctx,
        );
        let mut expr = HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                "value".to_string(),
                Vec::new(),
                None,
                HirMethodCallTarget::impl_method(
                    DefId::new(CrateId(1), LocalDefId(10)),
                    DefId::new(CrateId(1), LocalDefId(11)),
                    None,
                ),
            ),
            ty: Type::I64,
            span: Span::test(),
        };

        let err =
            super::remap_expr_child_locations(&mut expr, &remap, &validator, &type_validator, 0)
                .expect_err("dependency body providers must not authorize method targets");

        assert!(
            err.contains("unknown dependency impl target"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_accepts_resolved_var_extern_display_name_mismatch() {
        let (base, _cleanup) = temp_test_dir("resolved_var_extern_name_mismatch");
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let extern_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(7),
        };
        let extern_def = DefId::new(CrateId(0), LocalDefId(7));
        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        insert_interface_extern(
            &mut products,
            extern_id,
            HirExtern {
                id: extern_def,
                name: "dep::foreign".to_string(),
                params: Vec::new(),
                ret: Type::I64,
                variadic: false,
                is_unsafe: false,
            },
        );
        let mut function = products.bodies.functions[&function_id].clone();
        function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::ResolvedVar(HirVarRef {
                    name: "answer".to_string(),
                    target: HirVarTarget::Extern(extern_def),
                }),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            })],
            ty: Type::function(Vec::new(), Type::I64),
        };
        products.bodies.functions.insert(function_id, function);
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();
    }

    #[test]
    fn load_product_artifact_accepts_dependency_resolved_var_function_display_name_mismatch() {
        let (base, _cleanup) = temp_test_dir("dependency_resolved_var_function_name_mismatch");
        let dep_artifact = base.join("dep.rkca");
        let dep_object = base.join("dep.o");
        fs::write(&dep_object, []).unwrap();
        let app_artifact = base.join("app.rkca");
        let app_object = base.join("app.o");
        fs::write(&app_object, []).unwrap();

        let dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
        dep_products.write_artifact_to_path(&dep_artifact).unwrap();

        let app_function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
        app_products.identity_table.dependencies.insert(
            ProductCrateId(1),
            ProductCrateIdentity::local("dep".to_string()),
        );
        let mut app_function = app_products.bodies.functions[&app_function_id].clone();
        app_function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::ResolvedVar(HirVarRef {
                    name: "app::answer".to_string(),
                    target: HirVarTarget::Function(DefId::new(CrateId(1), LocalDefId(0))),
                }),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            })],
            ty: Type::function(Vec::new(), Type::I64),
        };
        app_products
            .bodies
            .functions
            .insert(app_function_id, app_function);
        app_products.write_artifact_to_path(&app_artifact).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(dep_artifact).unwrap();
        ctx.load_product_artifact_from_path(app_artifact).unwrap();
    }

    #[test]
    fn product_artifact_accepts_ambiguous_display_name_with_exact_function_target() {
        let left_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(10),
        };
        let right_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(11),
        };
        let left_def = DefId::new(CrateId(0), LocalDefId(10));
        let right_def = DefId::new(CrateId(0), LocalDefId(11));
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        products
            .identity_table
            .display_names
            .insert(left_id, "left::value".to_string());
        products
            .identity_table
            .display_names
            .insert(right_id, "right::value".to_string());
        insert_interface_function(&mut products, left_id, &test_function(left_def, "value"));
        insert_interface_function(&mut products, right_id, &test_function(right_def, "value"));
        let mut expr = HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "value".to_string(),
                target: HirVarTarget::Function(right_def),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        };
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        let child_validator = super::ProductChildLocationValidator::from_products(&products);
        let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap();
    }

    #[test]
    fn load_product_artifact_accepts_dependency_generic_impl_method_target() {
        let (base, _cleanup) = temp_test_dir("dependency_generic_impl_method_target");
        let dep_artifact = base.join("dep.rkca");
        let dep_object = base.join("dep.o");
        fs::write(&dep_object, []).unwrap();
        let app_artifact = base.join("app.rkca");
        let app_object = base.join("app.o");
        fs::write(&app_object, []).unwrap();

        let dep_impl_id = DefId::new(CrateId(0), LocalDefId(10));
        let dep_method_id = DefId::new(CrateId(0), LocalDefId(11));
        let mut dep_method = test_function(dep_method_id, "value");
        dep_method.is_method = true;
        dep_method.self_receiver = Some(crate::types::ReceiverMode::Move);
        dep_method.params.push(crate::hir::HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(crate::types::GenericParamId {
                owner: dep_impl_id,
                index: 0,
            }),
            mutable: false,
            is_ref: false,
        });
        let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
        let dep_impl_product_id = ProductDefId::from(dep_impl_id);
        let dep_impl = HirImpl {
            id: dep_impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: generic_params(dep_impl_id, &["T"]),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Generic(
                crate::types::GenericParamId {
                    owner: dep_impl_id,
                    index: 0,
                },
            )),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), dep_method)]),
        };
        insert_interface_impl(&mut dep_products, dep_impl_product_id, dep_impl.clone());
        dep_products
            .bodies
            .generic_impls
            .insert(dep_impl_product_id, dep_impl);
        dep_products.write_artifact_to_path(&dep_artifact).unwrap();

        let app_function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
        app_products.identity_table.dependencies.insert(
            ProductCrateId(1),
            ProductCrateIdentity::local("dep".to_string()),
        );
        let mut app_function = app_products.bodies.functions[&app_function_id].clone();
        app_function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::MethodCall(
                    Box::new(HirExpr {
                        kind: HirExprKindFor::IntLiteral(1),
                        ty: Type::I64,
                        span: Span::test(),
                    }),
                    "value".to_string(),
                    Vec::new(),
                    Some(crate::types::ReceiverMode::Move),
                    HirMethodCallTarget::impl_method(
                        DefId::new(CrateId(1), LocalDefId(10)),
                        DefId::new(CrateId(1), LocalDefId(11)),
                        None,
                    ),
                ),
                ty: Type::I64,
                span: Span::test(),
            })],
            ty: Type::I64,
        };
        app_products
            .bodies
            .functions
            .insert(app_function_id, app_function);
        app_products.write_artifact_to_path(&app_artifact).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(dep_artifact).unwrap();
        ctx.load_product_artifact_from_path(app_artifact)
            .expect("dependency generic impl method targets should validate");
    }

    #[test]
    fn load_product_artifact_rejects_dependency_field_sidecar_mismatch() {
        let (base, _cleanup) = temp_test_dir("dependency_field_sidecar_mismatch");
        let dep_artifact = base.join("dep.rkca");
        let dep_object = base.join("dep.o");
        fs::write(&dep_object, []).unwrap();
        let app_artifact = base.join("app.rkca");
        let app_object = base.join("app.o");
        fs::write(&app_object, []).unwrap();

        let dep_struct_id = DefId::new(CrateId(0), LocalDefId(5));
        let dep_struct_product_id = ProductDefId::from(dep_struct_id);
        let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
        dep_products
            .identity_table
            .display_names
            .insert(dep_struct_product_id, "dep::Widget".to_string());
        insert_interface_struct(
            &mut dep_products,
            dep_struct_product_id,
            HirStruct {
                id: dep_struct_id,
                name: "dep::Widget".to_string(),
                generic_params: Vec::new(),
                fields: vec![HirField {
                    id: FieldId(0),
                    name: "value".to_string(),
                    ty: Type::I64,
                    public: true,
                }],
            },
        );
        dep_products.write_artifact_to_path(&dep_artifact).unwrap();

        let app_function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let dep_struct_ref = DefId::new(CrateId(1), LocalDefId(5));
        let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
        app_products.identity_table.dependencies.insert(
            ProductCrateId(1),
            ProductCrateIdentity::local("dep".to_string()),
        );
        let mut app_function = app_products.bodies.functions[&app_function_id].clone();
        app_function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::FieldAccess(
                    Box::new(HirExpr {
                        kind: HirExprKindFor::Var("widget".to_string()),
                        ty: Type::Struct {
                            id: dep_struct_ref,
                            args: Vec::new(),
                        },
                        span: Span::test(),
                    }),
                    "value".to_string(),
                    Some(HirFieldLocation {
                        owner: dep_struct_ref,
                        field_id: FieldId(0),
                        name: "other".to_string(),
                    }),
                ),
                ty: Type::I64,
                span: Span::test(),
            })],
            ty: Type::I64,
        };
        app_products
            .bodies
            .functions
            .insert(app_function_id, app_function);
        app_products.write_artifact_to_path(&app_artifact).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(dep_artifact).unwrap();
        let err = ctx
            .load_product_artifact_from_path(app_artifact)
            .expect_err("dependency field sidecar mismatch should be rejected");

        assert!(
            err.contains("field sidecar mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_product_artifact_rejects_dependency_variant_sidecar_mismatch() {
        let (base, _cleanup) = temp_test_dir("dependency_variant_sidecar_mismatch");
        let dep_artifact = base.join("dep.rkca");
        let dep_object = base.join("dep.o");
        fs::write(&dep_object, []).unwrap();
        let app_artifact = base.join("app.rkca");
        let app_object = base.join("app.o");
        fs::write(&app_object, []).unwrap();

        let dep_enum_id = DefId::new(CrateId(0), LocalDefId(6));
        let dep_enum_product_id = ProductDefId::from(dep_enum_id);
        let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
        dep_products
            .identity_table
            .display_names
            .insert(dep_enum_product_id, "dep::Choice".to_string());
        insert_interface_enum(
            &mut dep_products,
            dep_enum_product_id,
            HirEnum {
                id: dep_enum_id,
                name: "dep::Choice".to_string(),
                generic_params: Vec::new(),
                variants: vec![HirVariant {
                    id: VariantId(0),
                    name: "Some".to_string(),
                    fields: HirVariantFields::Unit,
                }],
            },
        );
        dep_products.write_artifact_to_path(&dep_artifact).unwrap();

        let app_function_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let dep_enum_ref = DefId::new(CrateId(1), LocalDefId(6));
        let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
        app_products.identity_table.dependencies.insert(
            ProductCrateId(1),
            ProductCrateIdentity::local("dep".to_string()),
        );
        let mut app_function = app_products.bodies.functions[&app_function_id].clone();
        app_function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::EnumVariant(
                    "Choice".to_string(),
                    "Some".to_string(),
                    Vec::new(),
                    Some(HirVariantLocation {
                        owner: dep_enum_ref,
                        variant_id: VariantId(0),
                        name: "None".to_string(),
                    }),
                ),
                ty: Type::Enum {
                    id: dep_enum_ref,
                    args: Vec::new(),
                },
                span: Span::test(),
            })],
            ty: Type::Enum {
                id: dep_enum_ref,
                args: Vec::new(),
            },
        };
        app_function.ret_type = Type::Enum {
            id: dep_enum_ref,
            args: Vec::new(),
        };
        insert_function_body(&mut app_products, app_function_id, app_function);
        app_products.write_artifact_to_path(&app_artifact).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(dep_artifact).unwrap();
        let err = ctx
            .load_product_artifact_from_path(app_artifact)
            .expect_err("dependency variant sidecar mismatch should be rejected");

        assert!(
            err.contains("variant sidecar mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_child_location_validator_rejects_mismatched_field_sidecar() {
        let struct_product_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(5),
        };
        let struct_def = DefId::new(CrateId(0), LocalDefId(5));
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        insert_interface_struct(
            &mut products,
            struct_product_id,
            HirStruct {
                id: struct_def,
                name: "dep::Widget".to_string(),
                generic_params: Vec::new(),
                fields: vec![HirField {
                    id: FieldId(0),
                    name: "value".to_string(),
                    ty: Type::I64,
                    public: true,
                }],
            },
        );

        let validator = super::ProductChildLocationValidator::from_products(&products);
        let err = validator
            .validate_field(struct_product_id, FieldId(0), "other", "value")
            .unwrap_err();

        assert!(
            err.contains("field sidecar mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_child_location_remap_rejects_excessive_depth() {
        let mut expr = HirExpr {
            kind: HirExprKindFor::IntLiteral(1),
            ty: Type::I64,
            span: Span::test(),
        };
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::new(),
        };
        let validator = super::ProductChildLocationValidator {
            fields_by_id: HashMap::new(),
            variants_by_id: HashMap::new(),
        };
        let type_validator = super::ProductNominalTypeValidator {
            normalization_env: Default::default(),
            local_crate: ProductCrateId(0),
            structs: BTreeSet::new(),
            enums: BTreeSet::new(),
            type_aliases: BTreeSet::new(),
            traits: BTreeMap::new(),
            trait_generic_counts: BTreeMap::new(),
            trait_methods: BTreeMap::new(),
            trait_signatures: BTreeMap::new(),
            impls: BTreeSet::new(),
            impl_traits: BTreeMap::new(),
            impl_trait_args: BTreeMap::new(),
            impl_methods: BTreeMap::new(),
            effective_trait_methods: BTreeMap::new(),
            methods: BTreeSet::new(),
            static_methods: BTreeSet::new(),
            method_receiver_modes: BTreeMap::new(),
            functions: BTreeSet::new(),
            externs: BTreeSet::new(),
            dependency_structs: BTreeSet::new(),
            dependency_enums: BTreeSet::new(),
            dependency_type_aliases: BTreeSet::new(),
            dependency_traits: BTreeMap::new(),
            dependency_trait_generic_counts: BTreeMap::new(),
            dependency_trait_methods: BTreeMap::new(),
            dependency_trait_signatures: BTreeMap::new(),
            dependency_impls: BTreeSet::new(),
            dependency_impl_traits: BTreeMap::new(),
            dependency_impl_trait_args: BTreeMap::new(),
            dependency_impl_methods: BTreeMap::new(),
            dependency_effective_trait_methods: BTreeMap::new(),
            dependency_methods: BTreeSet::new(),
            dependency_static_methods: BTreeSet::new(),
            dependency_method_receiver_modes: BTreeMap::new(),
            dependency_functions: BTreeSet::new(),
            dependency_externs: BTreeSet::new(),
        };

        let err = super::remap_expr_child_locations(
            &mut expr,
            &remap,
            &validator,
            &type_validator,
            super::MAX_ARTIFACT_HIR_REMAP_DEPTH + 1,
        )
        .unwrap_err();

        assert!(err.contains("maximum remap depth"));
    }

    #[test]
    fn load_product_artifact_rejects_unmapped_product_crate_id() {
        let (base, _cleanup) = temp_test_dir("unmapped_product_crate");
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let function_id = product_def_id(0, 0);
        products
            .interface
            .functions
            .get_mut(&function_id)
            .unwrap()
            .generic_params
            .push(GenericParamDecl::type_param(
                crate::types::GenericParamId {
                    owner: DefId::new(CrateId(9), LocalDefId(0)),
                    index: 0,
                },
                "T",
            ));
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .unwrap_err();

        assert!(err.contains("unmapped product DefId 9::0"), "{err}");
    }

    #[test]
    fn load_product_artifact_registers_object_backed_crate() {
        let base = std::env::temp_dir().join(format!(
            "rock_product_loader_{}_{}",
            std::process::id(),
            "object_crate"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let function_id = DefId::new(CrateId(1), LocalDefId(7));
        let product_id = ProductDefId::from(function_id);
        let mut identity_table = ProductIdentityTable::default();
        identity_table.local_crate = Some(ProductCrateId(1));
        identity_table
            .display_names
            .insert(product_id, "dep::answer".to_string());
        identity_table
            .export_names
            .insert("answer".to_string(), product_id);
        let mut interface = ProductInterface::default();
        interface.functions.insert(
            product_id,
            ProductFunctionInterface::from(&test_function(function_id, "dep::answer")),
        );
        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table,
            interface,
            bodies: Default::default(),
            link: ProductLinkData {
                object_path: Some(object_path.clone()),
                records: BTreeMap::from([(
                    product_id,
                    ProductLinkRecord {
                        backend_symbol: "dep_answer".to_string(),
                    },
                )]),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        assert!(dep.link().is_object_backed());
        assert_eq!(dep.link().object_path().cloned(), Some(object_path));
        assert_eq!(dep.link().backend_symbol(function_id), Some("dep_answer"));
        let interface = dep.metadata().interface();
        assert!(interface
            .function_by_canonical_name("dep::answer")
            .is_some());
        assert_eq!(
            interface
                .root_export_ids
                .get("answer")
                .map(|export| export.source.as_str()),
            Some("dep::answer")
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn load_product_artifact_records_root_export_ids() {
        let base = std::env::temp_dir().join(format!(
            "rock_product_loader_{}_{}",
            std::process::id(),
            "root_export_ids"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        let interface = dep.metadata().interface();
        let exported = interface.root_export_ids.get("answer").unwrap();
        let function = interface.function_by_canonical_name("dep::answer").unwrap();

        assert_eq!(exported.source, "dep::answer");
        assert_eq!(exported.id, function.id);
        assert_eq!(dep.link().backend_symbol(function.id), Some("dep_answer"));

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn load_product_artifact_qualifies_method_like_root_export_sources() {
        let base = std::env::temp_dir().join(format!(
            "rock_product_loader_{}_{}",
            std::process::id(),
            "method_like_root_export"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let artifact_path = base.join("dep.rkca");

        let trait_id = DefId::new(CrateId(1), LocalDefId(8));
        let method_id = DefId::new(CrateId(1), LocalDefId(9));
        let product_trait_id = ProductDefId::from(trait_id);
        let product_method_id = ProductDefId::from(method_id);
        let mut identity_table = ProductIdentityTable::default();
        identity_table.local_crate = Some(ProductCrateId(1));
        identity_table
            .display_names
            .insert(product_trait_id, "Show".to_string());
        identity_table
            .display_names
            .insert(product_method_id, "Show::println".to_string());
        identity_table
            .export_names
            .insert("Show::println".to_string(), product_method_id);

        let method = test_function(method_id, "println");
        let mut bodies = ProductBodies::default();
        bodies
            .trait_default_methods
            .insert(product_method_id, method.clone());
        let mut interface = ProductInterface::default();
        interface.traits.insert(
            product_trait_id,
            ProductTraitInterface::from(&HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Show".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::from([("println".to_string(), method)]),
                signatures: HashMap::new(),
            }),
        );

        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table,
            interface,
            bodies,
            link: ProductLinkData {
                object_path: None,
                records: Default::default(),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        let interface = dep.metadata().interface();
        let exported = interface.root_export_ids.get("Show::println").unwrap();

        assert_eq!(exported.source, "dep::Show::println");
        assert_eq!(exported.id.local, LocalDefId(9));
        assert_eq!(
            dep.metadata()
                .resolver()
                .item_paths
                .get("dep::Show::println"),
            Some(&exported.id)
        );
        assert_eq!(
            dep.metadata().resolver().item_names_by_id.get(&exported.id),
            Some(&"dep::Show::println".to_string())
        );

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn load_product_artifact_rejects_root_export_without_canonical_id() {
        let base = std::env::temp_dir().join(format!(
            "rock_product_loader_{}_{}",
            std::process::id(),
            "bad_root_export_id"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let missing_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(99),
        };
        products
            .identity_table
            .display_names
            .insert(missing_id, "dep::missing".to_string());
        products
            .identity_table
            .export_names
            .insert("missing".to_string(), missing_id);
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .unwrap_err();

        assert!(
            err.contains("root export 'missing'") && err.contains("canonical DefId"),
            "unexpected error: {}",
            err
        );

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn load_product_artifact_rejects_root_export_without_display_name() {
        let base = std::env::temp_dir().join(format!(
            "rock_product_loader_{}_{}",
            std::process::id(),
            "bad_root_export_display_name"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
        let missing_name_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(99),
        };
        products
            .identity_table
            .export_names
            .insert("missing".to_string(), missing_name_id);
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .unwrap_err();

        assert!(
            err.contains("root export 'missing'") && err.contains("display name"),
            "unexpected error: {}",
            err
        );

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn load_product_artifact_resolves_common_relative_object_path() {
        let base = std::env::temp_dir().join(format!(
            "rock_product_loader_{}_{}",
            std::process::id(),
            "relative_object"
        ));
        let _ = fs::remove_dir_all(&base);
        let build_dir = base.join("build");
        fs::create_dir_all(&build_dir).unwrap();
        let artifact_path = build_dir.join("dep.rkca");
        let object_path = build_dir.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table: ProductIdentityTable {
                local_crate: Some(ProductCrateId(0)),
                ..Default::default()
            },
            interface: ProductInterface::default(),
            bodies: Default::default(),
            link: ProductLinkData {
                object_path: Some(PathBuf::from("build/dep.o")),
                records: Default::default(),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        assert_eq!(
            dep.link().object_path().cloned(),
            Some(object_path.canonicalize().unwrap())
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn load_product_artifact_prefers_artifact_relative_object_over_cwd() {
        let test_name = format!(
            "rock_product_loader_conflict_{}_{}",
            std::process::id(),
            "artifact_relative"
        );
        let base = std::env::temp_dir().join(&test_name);
        let _ = fs::remove_dir_all(&base);
        let artifact_dir = base.join("artifacts");
        fs::create_dir_all(&artifact_dir).unwrap();
        let artifact_path = artifact_dir.join("dep.rkca");

        let object_path = PathBuf::from("..")
            .join("target")
            .join(&test_name)
            .join("dep.o");
        let cwd_object_path = std::env::current_dir().unwrap().join(&object_path);
        let artifact_object_path = artifact_dir.join(&object_path);
        fs::create_dir_all(cwd_object_path.parent().unwrap()).unwrap();
        fs::create_dir_all(artifact_object_path.parent().unwrap()).unwrap();
        fs::write(&cwd_object_path, b"wrong object").unwrap();
        fs::write(&artifact_object_path, b"right object").unwrap();

        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table: ProductIdentityTable {
                local_crate: Some(ProductCrateId(0)),
                ..Default::default()
            },
            interface: ProductInterface::default(),
            bodies: Default::default(),
            link: ProductLinkData {
                object_path: Some(object_path),
                records: Default::default(),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        assert_eq!(
            dep.link().object_path().cloned(),
            Some(artifact_object_path.canonicalize().unwrap())
        );

        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(
            std::env::current_dir()
                .unwrap()
                .join("..")
                .join("target")
                .join(test_name),
        );
    }

    #[test]
    fn load_product_artifact_keeps_generic_body_without_display_metadata() {
        let base = std::env::temp_dir().join(format!(
            "rock_product_loader_{}_{}",
            std::process::id(),
            "generic_names"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("dep.o");
        fs::write(&object_path, []).unwrap();

        let function_id = DefId::new(CrateId(1), LocalDefId(7));
        let product_id = ProductDefId::from(function_id);
        let mut identity_table = ProductIdentityTable::default();
        identity_table.local_crate = Some(ProductCrateId(1));
        let mut identity = test_function(function_id, "identity");
        identity.generic_params = vec![GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: function_id,
                index: 0,
            },
            "T",
        )];
        let mut interface = ProductInterface::default();
        interface
            .functions
            .insert(product_id, ProductFunctionInterface::from(&identity));
        let mut bodies = crate::products::ProductBodies::default();
        bodies.functions.insert(product_id, identity);
        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table,
            interface,
            bodies,
            link: ProductLinkData {
                object_path: Some(object_path.clone()),
                records: Default::default(),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let dep = ctx.extern_crate("dep").unwrap();
        assert!(dep.metadata().interface().canonical_names.is_empty());
        let loaded_id = *dep
            .metadata()
            .interface()
            .functions
            .keys()
            .next()
            .expect("function interface should survive without display metadata");
        assert!(dep.body_providers().generic_function(loaded_id).is_some());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn resolver_from_products_ignores_impl_display_names() {
        let impl_id = DefId::new(CrateId(1), LocalDefId(8));
        let product_id = ProductDefId::from(impl_id);
        let mut identity_table = ProductIdentityTable::default();
        identity_table
            .display_names
            .insert(product_id, "Box as Show".to_string());

        let mut interface = ProductInterface::default();
        interface.impls.insert(
            product_id,
            ProductImplInterface::from(&HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
                trait_name: Some("Show".to_string()),
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            }),
        );
        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table,
            interface,
            bodies: Default::default(),
            link: ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };

        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::new(),
        };
        let resolver =
            super::resolver_from_products(&products, "dep", &remap, &CrateContext::new()).unwrap();

        assert!(!resolver.item_paths.contains_key("Box as Show"));
    }

    #[test]
    fn load_product_artifact_records_id_backed_prelude_exports() {
        let base = std::env::temp_dir().join(format!(
            "rock_product_loader_{}_{}",
            std::process::id(),
            "id_backed_prelude_exports"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let artifact_path = base.join("stdlib.rkca");
        let object_path = base.join("stdlib.o");
        fs::write(&object_path, []).unwrap();

        let struct_id = DefId::new(CrateId(1), LocalDefId(7));
        let product_id = ProductDefId::from(struct_id);
        let mut identity_table = ProductIdentityTable::default();
        identity_table.local_crate = Some(ProductCrateId(1));
        identity_table
            .display_names
            .insert(product_id, "stdlib::prelude::String".to_string());
        identity_table
            .prelude_export_names
            .insert("Text".to_string(), product_id);

        let mut interface = ProductInterface::default();
        interface.structs.insert(
            product_id,
            ProductStructInterface::from(&HirStruct {
                id: struct_id,
                name: "String".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            }),
        );

        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("stdlib".to_string()),
            identity_table,
            interface,
            bodies: Default::default(),
            link: ProductLinkData {
                object_path: Some(object_path.clone()),
                records: Default::default(),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let stdlib = ctx.extern_crate("stdlib").unwrap();
        let export = stdlib.metadata().prelude_export_ids().get("Text").unwrap();
        assert_eq!(export.source, "stdlib::prelude::String");
        assert_eq!(export.id.crate_id, CrateId(1));

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn load_product_artifact_rejects_prelude_export_id_without_display_name() {
        let base = std::env::temp_dir().join(format!(
            "rock_product_loader_{}_{}",
            std::process::id(),
            "bad_id_backed_prelude_export"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let artifact_path = base.join("stdlib.rkca");
        let object_path = base.join("stdlib.o");
        fs::write(&object_path, []).unwrap();

        let product_id = ProductDefId {
            crate_id: ProductCrateId(1),
            local_id: ProductLocalDefId(7),
        };
        let mut identity_table = ProductIdentityTable::default();
        identity_table.local_crate = Some(ProductCrateId(1));
        identity_table
            .prelude_export_names
            .insert("String".to_string(), product_id);

        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("stdlib".to_string()),
            identity_table,
            interface: ProductInterface::default(),
            bodies: Default::default(),
            link: ProductLinkData {
                object_path: Some(object_path.clone()),
                records: Default::default(),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .unwrap_err();

        assert!(
            err.contains("prelude export 'String'") && err.contains("display name"),
            "unexpected error: {}",
            err
        );

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn load_product_artifact_rejects_sidecar_id_backed_prelude_export() {
        let base = std::env::temp_dir().join(format!(
            "rock_product_loader_{}_{}",
            std::process::id(),
            "sidecar_id_backed_prelude_export"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let artifact_path = base.join("demo.rkca");
        let object_path = base.join("demo.o");
        fs::write(&object_path, []).unwrap();

        let trait_id = DefId::new(CrateId(1), LocalDefId(7));
        let trait_product_id = ProductDefId::from(trait_id);
        let method_id = DefId::new(CrateId(1), LocalDefId(8));
        let method_product_id = ProductDefId::from(method_id);
        let mut identity_table = ProductIdentityTable::default();
        identity_table.local_crate = Some(ProductCrateId(1));
        identity_table
            .display_names
            .insert(trait_product_id, "Trait".to_string());
        identity_table
            .display_names
            .insert(method_product_id, "default_method".to_string());
        identity_table
            .prelude_export_names
            .insert("default_method".to_string(), method_product_id);

        let method = test_function(method_id, "default_method");
        let mut bodies = ProductBodies::default();
        bodies
            .trait_default_methods
            .insert(method_product_id, method.clone());
        let mut interface = ProductInterface::default();
        interface.traits.insert(
            trait_product_id,
            ProductTraitInterface::from(&HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Trait".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::from([("default_method".to_string(), method)]),
                signatures: HashMap::new(),
            }),
        );

        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("demo".to_string()),
            identity_table,
            interface,
            bodies,
            link: ProductLinkData {
                object_path: Some(object_path.clone()),
                records: Default::default(),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        let err = ctx
            .load_product_artifact_from_path(artifact_path)
            .unwrap_err();

        assert!(
            err.contains("prelude export 'default_method'")
                && err.contains("canonical prelude item ID"),
            "unexpected error: {}",
            err
        );

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn load_product_artifact_derives_missing_prelude_exports_from_prelude_items() {
        let base = std::env::temp_dir().join(format!(
            "rock_product_loader_{}_{}",
            std::process::id(),
            "derived_prelude_exports"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let artifact_path = base.join("stdlib.rkca");
        let object_path = base.join("stdlib.o");
        fs::write(&object_path, []).unwrap();

        let struct_id = DefId::new(CrateId(1), LocalDefId(7));
        let product_id = ProductDefId::from(struct_id);
        let mut identity_table = ProductIdentityTable::default();
        identity_table.local_crate = Some(ProductCrateId(1));
        identity_table
            .display_names
            .insert(product_id, "stdlib::prelude::String".to_string());

        let mut interface = ProductInterface::default();
        interface.structs.insert(
            product_id,
            ProductStructInterface::from(&HirStruct {
                id: struct_id,
                name: "String".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            }),
        );

        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("stdlib".to_string()),
            identity_table,
            interface,
            bodies: Default::default(),
            link: ProductLinkData {
                object_path: Some(object_path.clone()),
                records: Default::default(),
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let mut ctx = CrateContext::new();
        ctx.load_product_artifact_from_path(artifact_path).unwrap();

        let stdlib = ctx.extern_crate("stdlib").unwrap();
        let interface = stdlib.metadata().interface();
        let exported_struct = interface
            .struct_by_canonical_name("stdlib::prelude::String")
            .unwrap();
        let prelude_export = stdlib
            .metadata()
            .prelude_export_ids()
            .get("String")
            .unwrap();
        assert_eq!(prelude_export.source, "stdlib::prelude::String");
        assert_eq!(prelude_export.id, exported_struct.id);

        let _ = fs::remove_dir_all(base);
    }
}
