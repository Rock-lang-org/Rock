use std::collections::{BTreeSet, HashMap};

use super::product_def_id;
use super::validation::validate_embedded_id;
use crate::hir::{HirLanguageItems, HirNameTables, HirProgram};
use crate::products::{CompilerProducts, ProductDefId};

pub(super) fn product_language_item_program(
    products: &CompilerProducts,
) -> Result<HirProgram, String> {
    let ids = product_language_item_ids(products);
    let functions = products
        .interface
        .functions
        .iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, function)| {
            validate_embedded_id("function", *id, function.id)?;
            Ok((
                product_def_id(*id),
                crate::crate_artifact::types::hir_function_from_interface(function),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let structs = products
        .interface
        .structs
        .iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, strukt)| {
            validate_embedded_id("struct", *id, strukt.id)?;
            Ok((
                product_def_id(*id),
                crate::crate_artifact::types::hir_struct_from_interface(strukt),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let enums = products
        .interface
        .enums
        .iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, enm)| {
            validate_embedded_id("enum", *id, enm.id)?;
            Ok((
                product_def_id(*id),
                crate::crate_artifact::types::hir_enum_from_interface(enm),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let traits = products
        .interface
        .traits
        .iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, trt)| {
            validate_embedded_id("trait", *id, trt.id)?;
            Ok((
                product_def_id(*id),
                crate::crate_artifact::types::hir_trait_from_interface(trt),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let impls = products
        .interface
        .impls
        .iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, imp)| {
            validate_embedded_id("impl", *id, imp.id)?;
            Ok((
                product_def_id(*id),
                crate::crate_artifact::types::hir_impl_from_interface(imp),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let externs = products
        .interface
        .externs
        .iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, ext)| {
            validate_embedded_id("extern", *id, ext.id)?;
            Ok((
                product_def_id(*id),
                crate::crate_artifact::types::hir_extern_from_interface(ext),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;

    Ok(HirProgram::from_id_parts_with_names_and_canonical_names(
        functions,
        structs,
        enums,
        traits,
        impls,
        externs,
        HirNameTables::default(),
        HirLanguageItems::default(),
        &HashMap::new(),
    ))
}

fn product_language_item_ids(products: &CompilerProducts) -> BTreeSet<ProductDefId> {
    let language_items = &products.interface.language_items;
    let mut ids = BTreeSet::new();
    let mut member_ids = BTreeSet::new();
    let mut assoc_ids = BTreeSet::new();
    let mut variant_ids = BTreeSet::new();
    if let Some(items) = &language_items.fold {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
    }
    if let Some(items) = &language_items.sized {
        ids.insert(items.trait_id);
    }
    if let Some(items) = &language_items.drop {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
    }
    if let Some(items) = &language_items.index {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
        assoc_ids.insert(items.output_id);
    }
    if let Some(items) = &language_items.index_mut {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
        assoc_ids.insert(items.output_id);
    }
    if let Some(items) = &language_items.fn_once {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
        assoc_ids.insert(items.output_id);
    }
    if let Some(items) = &language_items.fn_mut {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
        assoc_ids.insert(items.output_id);
    }
    if let Some(items) = &language_items.fn_trait {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
        assoc_ids.insert(items.output_id);
    }
    if let Some(items) = &language_items.send {
        ids.insert(items.trait_id);
    }
    if let Some(items) = &language_items.sync {
        ids.insert(items.trait_id);
    }
    if let Some(items) = &language_items.try_protocol {
        ids.insert(items.try_trait_id);
        ids.insert(items.branch_method_id);
        ids.insert(items.from_residual_trait_id);
        ids.insert(items.from_residual_method_id);
        ids.insert(items.control_flow_enum_id);
        member_ids.insert(items.branch_method_id);
        member_ids.insert(items.from_residual_method_id);
        assoc_ids.insert(items.output_id);
        assoc_ids.insert(items.residual_id);
        variant_ids.insert(items.break_variant_id);
        variant_ids.insert(items.continue_variant_id);
    }
    if let Some(items) = &language_items.range {
        ids.insert(items.enum_id);
        variant_ids.extend([
            items.full_variant_id,
            items.from_variant_id,
            items.to_variant_id,
            items.to_inclusive_variant_id,
            items.exclusive_variant_id,
            items.inclusive_variant_id,
        ]);
    }

    for (trait_id, trait_def) in &products.interface.traits {
        let owns_member = trait_def
            .methods
            .values()
            .any(|method| member_ids.contains(&ProductDefId::from(method.id)))
            || trait_def
                .signatures
                .values()
                .any(|signature| member_ids.contains(&ProductDefId::from(signature.id)));
        let owns_associated_type = trait_def
            .associated_types
            .iter()
            .any(|associated_type| assoc_ids.contains(&associated_type.id));
        if owns_member || owns_associated_type {
            ids.insert(*trait_id);
        }
    }
    for (enum_id, enum_def) in &products.interface.enums {
        if enum_def
            .variants
            .iter()
            .any(|variant| variant_ids.contains(&variant.id))
        {
            ids.insert(*enum_id);
        }
    }

    ids
}
