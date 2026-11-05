use std::collections::{BTreeMap, HashMap};

use crate::products::{
    CompilerProducts, ProductDefId, ProductFunctionInterface, ProductImplInterface,
    ProductTraitInterface,
};

/// Checks that body rows and interface rows agree, in one linear pass. The
/// producer guarantees this, but the artifact is read from disk: a missing
/// generic body would otherwise surface as an internal error at instantiation.
pub(super) fn validate_product_body_rows_have_interface(
    products: &CompilerProducts,
) -> Result<(), String> {
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

pub(super) fn product_function_interface_requires_reusable_body(
    function: &ProductFunctionInterface,
) -> bool {
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

pub(super) fn trait_with_default_bodies_from_interface(
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
