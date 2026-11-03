use std::collections::HashMap;
use std::path::Path;

use super::object_link::{
    backend_symbols_from_products, resolve_product_object_path, validate_object_link_records,
};
use super::prelude_exports::normalize_product_prelude_exports;
use super::product_validation::{validate_product_interface_rows, validate_product_type_fields};
use super::products_to_interface::{
    cross_crate_hir_from_products, interface_from_products, resolver_from_products,
};
use super::ProductIdentityRemap;
use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::ArtifactCrateInterface;
use crate::crate_system::{
    CrateContext, ExternCrateBodies, ExternCrateLink, ExternCrateMetadata, ExternCrateRecord,
};
use crate::hir::{HirLanguageItems, HirNameTables};
use crate::ids::DefId;
use crate::products::CompilerProducts;

pub(crate) fn intern_loaded_interface_types(
    interface: &ArtifactCrateInterface,
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

pub(super) fn artifact_hir_name_tables(
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

pub(super) fn extern_crate_from_products(
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
    let language_items =
        crate::crate_artifact::language_items::language_items_from_products(products, remap)?;
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
