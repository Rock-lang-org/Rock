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
use crate::type_services::kind::Kind;
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
            .with_proc_macros(products.proc_macros.clone())
            .with_macros(
                products.interface.macros.clone(),
                products.interface.prelude_macros.clone(),
            ),
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
            if let Some(method) = trait_def.methods.get(signature_name)
                .filter(|method| method.id == signature.id)
            {
                // This is one member with both a declaration and a default body,
                // not a second callable reusing an unrelated declaration's ID.
                if method.params != signature.params
                    || method.ret_type != signature.ret
                    || method.generic_params != signature.generic_params
                    || method.self_receiver != signature.self_receiver
                    || method.is_unsafe != signature.is_unsafe
                {
                    return Err(format!(
                        "Product artifact trait {} signature '{}' disagrees with its default method sharing the same ID",
                        id.local_id.0, signature_name,
                    ));
                }
                continue;
            }
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
    if let Some(summary) = &function.constant_arithmetic {
        summary.validate(function.params.len())?;
    }
    function.id = remap.def_id(id)?;
    remap_generic_param_decls(&mut function.generic_params, remap, type_validator)?;
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
    remap_generic_param_decls(&mut strukt.generic_params, remap, type_validator)?;
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
    remap_generic_param_decls(&mut alias.generic_params, remap, type_validator)?;
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
    remap_generic_param_decls(&mut enm.generic_params, remap, type_validator)?;
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
    remap_generic_param_decls(&mut trait_def.generic_params, remap, type_validator)?;
    if let Some(target) = &mut trait_def.target {
        remap_generic_param_decls(std::slice::from_mut(target), remap, type_validator)?;
    }
    remap_predicates(&mut trait_def.predicates, remap, type_validator)?;
    for method in trait_def.methods.values_mut() {
        let method_id = ProductDefId::from(method.id);
        *method = remap_function_interface_id(method.clone(), method_id, remap, type_validator)?;
    }
    for sig in trait_def.signatures.values_mut() {
        sig.id = remap.def_id(ProductDefId::from(sig.id))?;
        remap_generic_param_decls(&mut sig.generic_params, remap, type_validator)?;
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
    remap_generic_param_decls(&mut imp.type_generics, remap, type_validator)?;
    remap_generic_param_decls(&mut imp.trait_generics, remap, type_validator)?;
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

    remap_generic_param_decls(&mut function.generic_params, remap, type_validator)?;
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
    validator: &ProductNominalTypeValidator,
) -> Result<(), String> {
    let mut env = validator.normalization_env.clone();
    for decl in decls.iter() {
        env.register_generic_kind(decl.id, decl.kind.clone());
    }
    let mut default_seen = false;
    for decl in decls {
        if let Some(default) = &mut decl.default {
            default_seen = true;
            let owner = ProductDefId::from(decl.id.owner);
            let subject_index = validator.trait_generic_counts.get(&owner).copied();
            if let Some(index) = subject_index {
                let subject = GenericParamId { owner: decl.id.owner, index: index as u32 };
                if TypeNormalizer::new(&env).kind_of(&Type::Generic(subject)).is_err() {
                    env.register_generic_kind(subject, crate::type_services::kind::Kind::Type);
                }
            }
            let mut invalid_generic = false;
            crate::type_services::visit::visit_type(default, &mut |ty: &Type| {
                if let Type::Generic(param) = ty {
                    if param.owner != decl.id.owner || (param.index >= decl.id.index && Some(param.index as usize) != subject_index) {
                        invalid_generic = true;
                    }
                }
            });
            if invalid_generic { return Err("Product artifact generic default references an out-of-scope parameter".into()); }
            struct BoundScope { groups: Vec<Vec<Kind>>, valid: bool }
            impl crate::type_services::visit::TypeVisitor for BoundScope {
                fn enter_binders(&mut self, params: &[Kind]) { self.groups.push(params.to_vec()); }
                fn exit_binders(&mut self) { self.groups.pop(); }
                fn visit_type(&mut self, ty: &Type) {
                    if let Type::BoundVar { depth, index, kind } = ty {
                        self.valid &= self.groups.iter().rev().nth(*depth as usize)
                            .and_then(|group| group.get(*index as usize)) == Some(kind);
                    }
                    crate::type_services::visit::visit_type_children(ty, self);
                }
            }
            let mut scope = BoundScope { groups: Vec::new(), valid: true };
            crate::type_services::visit::visit_type(default, &mut scope);
            if !scope.valid { return Err("Product artifact generic default contains an escaping type binder".into()); }
            let kind = TypeNormalizer::new(&env).kind_of(default).map_err(|error| error.to_string())?;
            if kind != decl.kind { return Err(format!("Product artifact generic default has kind {kind}, expected {}", decl.kind)); }
            remap_type_def_ids(default, remap, validator)?;
        } else if default_seen {
            return Err("Product artifact required parameter follows a defaulted parameter".into());
        }
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
    remap_generic_param_decls(&mut value.generic_params, remap, type_validator)?;
    if let Some(target) = &mut value.target {
        remap_generic_param_decls(std::slice::from_mut(target), remap, type_validator)?;
    }
    remap_predicates(&mut value.predicates, remap, type_validator)?;
    for signature in value.signatures.values_mut() {
        signature.id = remap.def_id(ProductDefId::from(signature.id))?;
        remap_generic_param_decls(&mut signature.generic_params, remap, type_validator)?;
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
    remap_generic_param_decls(&mut value.type_generics, remap, type_validator)?;
    remap_generic_param_decls(&mut value.trait_generics, remap, type_validator)?;
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
mod tests;
