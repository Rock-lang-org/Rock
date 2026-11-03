use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::body_validation::product_function_interface_requires_reusable_body;
use super::ProductIdentityRemap;
use crate::ids::DefId;
use crate::products::{
    CompilerProducts, ProductDefId, ProductFunctionInterface, ProductImplInterface,
};
use crate::types::Type;

pub(super) fn validate_object_link_records(products: &CompilerProducts) -> Result<(), String> {
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

pub(super) fn backend_symbols_from_products(
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

pub(super) fn product_type_contains_forbidden_sentinel(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| {
        matches!(nested, Type::TypeVar(_) | Type::Error)
    })
}

pub(super) fn resolve_product_object_path(
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
