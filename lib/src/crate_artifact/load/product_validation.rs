use std::collections::BTreeMap;

use crate::ids::DefId;
use crate::products::{CompilerProducts, ProductCrateId, ProductDefId, ProductFunctionInterface};

pub(super) fn validate_product_interface_rows(products: &CompilerProducts) -> Result<(), String> {
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
            if let Some(method) = trait_def
                .methods
                .get(signature_name)
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
