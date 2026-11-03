use std::collections::{BTreeMap, BTreeSet};

use super::dependency_definitions::{
    add_callable_names, register_product_generic_kinds, static_method_interface_callable_names,
    ProductDependencyDefinitions,
};
use super::product_names::product_display_name;
use super::ProductIdentityRemap;
use crate::crate_system::CrateContext;
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::products::{CompilerProducts, ProductCrateId, ProductDefId};
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::{NominalTypeKind, Type};

#[derive(Debug, Clone)]
pub(super) struct ProductNominalTypeValidator {
    pub(super) normalization_env: TypeNormalizationEnv,
    pub(super) local_crate: ProductCrateId,
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
    pub(super) dependency_structs: BTreeSet<ProductDefId>,
    pub(super) dependency_enums: BTreeSet<ProductDefId>,
    pub(super) dependency_type_aliases: BTreeSet<ProductDefId>,
    pub(super) dependency_traits: BTreeMap<ProductDefId, BTreeSet<AssocTypeId>>,
    pub(super) dependency_trait_generic_counts: BTreeMap<ProductDefId, usize>,
    pub(super) dependency_trait_methods: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    pub(super) dependency_trait_signatures: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    pub(super) dependency_impls: BTreeSet<ProductDefId>,
    pub(super) dependency_impl_traits: BTreeMap<ProductDefId, Option<ProductDefId>>,
    pub(super) dependency_impl_trait_args: BTreeMap<ProductDefId, Vec<Type>>,
    pub(super) dependency_impl_methods: BTreeMap<ProductDefId, BTreeMap<String, ProductDefId>>,
    pub(super) dependency_effective_trait_methods:
        BTreeMap<(ProductDefId, ProductDefId), ProductDefId>,
    pub(super) dependency_methods: BTreeSet<ProductDefId>,
    pub(super) dependency_static_methods: BTreeSet<ProductDefId>,
    pub(super) dependency_method_receiver_modes:
        BTreeMap<ProductDefId, Option<crate::types::ReceiverMode>>,
    pub(super) dependency_functions: BTreeSet<ProductDefId>,
    pub(super) dependency_externs: BTreeSet<ProductDefId>,
}

impl ProductNominalTypeValidator {
    #[cfg(test)]
    pub(super) fn from_products(products: &CompilerProducts, remap: &ProductIdentityRemap) -> Self {
        Self::from_products_with_dependencies(
            products,
            remap,
            ProductDependencyDefinitions::default(),
        )
    }

    pub(super) fn from_products_with_context(
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

    pub(super) fn from_products_with_dependencies(
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
}
