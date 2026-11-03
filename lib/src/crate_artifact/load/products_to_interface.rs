use std::collections::BTreeMap;

use super::body_validation::{
    trait_with_default_bodies_from_interface, validate_product_body_rows_have_interface,
};
use super::child_location_validator::ProductChildLocationValidator;
use super::dependency_definitions::ProductDependencyDefinitions;
use super::id_remap::{
    remap_enum_interface_id, remap_extern_interface_id, remap_function_id,
    remap_function_interface_id, remap_impl_ids, remap_impl_interface_ids,
    remap_struct_interface_id, remap_trait_ids, remap_trait_interface_ids,
    remap_type_alias_interface_id,
};
use super::method_authority::validate_product_method_authority_maps;
use super::nominal_validator::ProductNominalTypeValidator;
use super::product_names::{
    product_def_id_to_existing_def_id, product_display_name, product_id_has_canonical_binding,
    qualify_product_name,
};
use super::ProductIdentityRemap;
use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::{ArtifactCrateInterface, ArtifactExport};
use crate::crate_system::CrateContext;
use crate::ids::DefId;
use crate::products::{CompilerProducts, ProductDefId};

pub(super) fn interface_from_products(
    products: &CompilerProducts,
    crate_name: &str,
    remap: &ProductIdentityRemap,
    ctx: &CrateContext,
    language_items: &crate::hir::HirLanguageItems,
) -> Result<ArtifactCrateInterface, String> {
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

    let interface = ArtifactCrateInterface {
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

pub(super) fn resolver_from_products(
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

pub(super) fn cross_crate_hir_from_products(
    products: &CompilerProducts,
    crate_name: &str,
    remap: &ProductIdentityRemap,
    ctx: &CrateContext,
) -> Result<crate::crate_artifact::ArtifactCrossCrateHir, String> {
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

    Ok(crate::crate_artifact::ArtifactCrossCrateHir {
        generic_functions,
        traits_with_defaults,
        generic_impls,
    })
}
