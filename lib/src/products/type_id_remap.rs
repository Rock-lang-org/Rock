use super::artifact_metadata::ProductBodies;
use super::body_type_remap::{
    remap_function_body_types, remap_function_signature_types, remap_impl_interface_types,
    remap_impl_types,
};
use super::identity_recording::{product_def_id_to_def_id, unambiguous_product_id};
use super::interface_types::{ProductFunctionInterface, ProductImplInterface, ProductInterface};
use super::location_id_remap::{
    remap_function_body_location_product_ids, remap_impl_body_location_product_ids,
    remap_product_def_id_owner,
};
use super::{ProductDefId, ProductIdRemap};
use crate::hir::{HirImplFor, HirPhase, HirVariantFields};
use crate::types::GenericParamDecl;

pub(super) fn remap_product_type_def_ids(
    interface: &mut ProductInterface,
    bodies: &mut ProductBodies,
    id_remap: &ProductIdRemap,
) {
    let mut remap_type = |ty: &mut crate::types::Type| {
        ty.remap_def_ids(&mut |id| {
            unambiguous_product_id(ProductDefId::from(id), id_remap)
                .map(product_def_id_to_def_id)
                .unwrap_or(id)
        });
    };

    for function in interface.functions.values_mut() {
        remap_generic_param_decls_product_ids(&mut function.generic_params, id_remap);
        remap_generic_bounds_product_ids(&mut function.generic_bounds, id_remap, &mut remap_type);
        remap_function_interface_signature_types(function, &mut remap_type);
    }
    for function in bodies.functions.values_mut() {
        remap_generic_param_decls_product_ids(&mut function.generic_params, id_remap);
        remap_generic_bounds_product_ids(&mut function.generic_bounds, id_remap, &mut remap_type);
        remap_function_signature_types(function, &mut remap_type);
        remap_function_body_types(function, &mut remap_type);
        remap_function_body_location_product_ids(function, id_remap);
    }
    for function in bodies.trait_default_methods.values_mut() {
        remap_generic_param_decls_product_ids(&mut function.generic_params, id_remap);
        remap_generic_bounds_product_ids(&mut function.generic_bounds, id_remap, &mut remap_type);
        remap_function_signature_types(function, &mut remap_type);
        remap_function_body_types(function, &mut remap_type);
        remap_function_body_location_product_ids(function, id_remap);
    }

    for strukt in interface.structs.values_mut() {
        remap_generic_param_decls_product_ids(&mut strukt.generic_params, id_remap);
        for field in &mut strukt.fields {
            remap_type(&mut field.ty);
        }
    }

    for enm in interface.enums.values_mut() {
        remap_generic_param_decls_product_ids(&mut enm.generic_params, id_remap);
        for variant in &mut enm.variants {
            match &mut variant.fields {
                HirVariantFields::Named(fields) => {
                    for field in fields {
                        remap_type(&mut field.ty);
                    }
                }
                HirVariantFields::Positional(types) => {
                    for ty in types {
                        remap_type(ty);
                    }
                }
                HirVariantFields::Unit => {}
            }
        }
    }

    for alias in interface.type_aliases.values_mut() {
        remap_generic_param_decls_product_ids(&mut alias.generic_params, id_remap);
        remap_type(&mut alias.ty);
    }

    for trait_ in interface.traits.values_mut() {
        remap_generic_param_decls_product_ids(&mut trait_.generic_params, id_remap);
        if let Some(target) = &mut trait_.target {
            remap_generic_param_decls_product_ids(std::slice::from_mut(target), id_remap);
        }
        for predicate in &mut trait_.predicates {
            predicate.remap_def_ids(&mut |id| {
                unambiguous_product_id(ProductDefId::from(id), id_remap)
                    .map(product_def_id_to_def_id)
                    .unwrap_or(id)
            });
        }
        for method in trait_.methods.values_mut() {
            remap_generic_param_decls_product_ids(&mut method.generic_params, id_remap);
            remap_generic_bounds_product_ids(&mut method.generic_bounds, id_remap, &mut remap_type);
            remap_function_interface_signature_types(method, &mut remap_type);
        }
        for sig in trait_.signatures.values_mut() {
            remap_generic_param_decls_product_ids(&mut sig.generic_params, id_remap);
            remap_generic_bounds_product_ids(&mut sig.generic_bounds, id_remap, &mut remap_type);
            for param in &mut sig.params {
                remap_type(param);
            }
            remap_type(&mut sig.ret);
        }
    }

    for imp in interface.impls.values_mut() {
        remap_generic_param_decls_product_ids(&mut imp.type_generics, id_remap);
        remap_generic_param_decls_product_ids(&mut imp.trait_generics, id_remap);
        remap_impl_interface_generic_param_decls_product_ids(imp, id_remap, &mut remap_type);
        remap_impl_interface_trait_ids_product_ids(imp, id_remap);
        remap_impl_interface_types(imp, &mut remap_type);
    }
    for imp in bodies.generic_impls.values_mut() {
        remap_generic_param_decls_product_ids(&mut imp.type_generics, id_remap);
        remap_generic_param_decls_product_ids(&mut imp.trait_generics, id_remap);
        remap_impl_generic_param_decls_product_ids(imp, id_remap, &mut remap_type);
        remap_impl_trait_ids_product_ids(imp, id_remap);
        remap_impl_types(imp, &mut remap_type);
        remap_impl_body_location_product_ids(imp, id_remap);
    }

    for ext in interface.externs.values_mut() {
        for param in &mut ext.params {
            remap_type(param);
        }
        remap_type(&mut ext.ret);
    }
}

fn remap_generic_param_decls_product_ids(
    generic_param_decls: &mut [GenericParamDecl],
    id_remap: &ProductIdRemap,
) {
    for generic_decl in generic_param_decls {
        if let Some(product_id) =
            unambiguous_product_id(ProductDefId::from(generic_decl.id.owner), id_remap)
        {
            generic_decl.id.owner = product_def_id_to_def_id(product_id);
        }
        if let Some(default) = &mut generic_decl.default {
            default.remap_def_ids(&mut |id| {
                unambiguous_product_id(ProductDefId::from(id), id_remap)
                    .map(product_def_id_to_def_id)
                    .unwrap_or(id)
            });
        }
    }
}

fn remap_generic_bounds_product_ids<F>(
    bounds: &mut crate::hir::HirGenericBounds,
    id_remap: &ProductIdRemap,
    remap_type: &mut F,
) where
    F: FnMut(&mut crate::types::Type),
{
    let mut original = std::mem::take(bounds);
    let mut predicates = std::mem::take(&mut original.predicates);
    for predicate in &mut predicates {
        match predicate {
            crate::types::Predicate::Trait {
                subject,
                trait_id,
                args,
            } => {
                remap_product_def_id_owner(trait_id, id_remap);
                remap_type(subject);
                for arg in args {
                    remap_type(arg);
                }
            }
        }
    }
    for (mut generic_param, mut trait_bounds) in original {
        if let Some(product_id) =
            unambiguous_product_id(ProductDefId::from(generic_param.owner), id_remap)
        {
            generic_param.owner = product_def_id_to_def_id(product_id);
        }
        for bound in &mut trait_bounds {
            remap_product_def_id_owner(&mut bound.trait_id, id_remap);
            for ty in &mut bound.type_args {
                remap_type(ty);
            }
        }
        bounds.insert(generic_param, trait_bounds);
    }
    bounds.predicates = predicates;
}

fn remap_impl_generic_param_decls_product_ids<P: HirPhase, F>(
    imp: &mut HirImplFor<P>,
    id_remap: &ProductIdRemap,
    remap_type: &mut F,
) where
    F: FnMut(&mut crate::types::Type),
{
    for method in imp.methods.values_mut() {
        remap_generic_param_decls_product_ids(&mut method.generic_params, id_remap);
        remap_generic_bounds_product_ids(&mut method.generic_bounds, id_remap, remap_type);
    }
}

fn remap_impl_interface_generic_param_decls_product_ids<F>(
    imp: &mut ProductImplInterface,
    id_remap: &ProductIdRemap,
    remap_type: &mut F,
) where
    F: FnMut(&mut crate::types::Type),
{
    for method in imp.methods.values_mut() {
        remap_generic_param_decls_product_ids(&mut method.generic_params, id_remap);
        remap_generic_bounds_product_ids(&mut method.generic_bounds, id_remap, remap_type);
    }
}

fn remap_impl_trait_ids_product_ids<P: HirPhase>(
    imp: &mut HirImplFor<P>,
    id_remap: &ProductIdRemap,
) {
    if let Some(trait_id) = &mut imp.trait_id {
        remap_product_def_id_owner(trait_id, id_remap);
    }
    let original = std::mem::take(&mut imp.bounds);
    for (mut generic_param, mut trait_bounds) in original {
        if let Some(product_id) =
            unambiguous_product_id(ProductDefId::from(generic_param.owner), id_remap)
        {
            generic_param.owner = product_def_id_to_def_id(product_id);
        }
        for bound in &mut trait_bounds {
            remap_product_def_id_owner(&mut bound.trait_id, id_remap);
        }
        imp.bounds.insert(generic_param, trait_bounds);
    }
}

fn remap_impl_interface_trait_ids_product_ids(
    imp: &mut ProductImplInterface,
    id_remap: &ProductIdRemap,
) {
    if let Some(trait_id) = &mut imp.trait_id {
        remap_product_def_id_owner(trait_id, id_remap);
    }
    let original = std::mem::take(&mut imp.bounds);
    for (mut generic_param, mut trait_bounds) in original {
        if let Some(product_id) =
            unambiguous_product_id(ProductDefId::from(generic_param.owner), id_remap)
        {
            generic_param.owner = product_def_id_to_def_id(product_id);
        }
        for bound in &mut trait_bounds {
            remap_product_def_id_owner(&mut bound.trait_id, id_remap);
        }
        imp.bounds.insert(generic_param, trait_bounds);
    }
}

pub(super) fn remap_function_interface_signature_types<F>(
    function: &mut ProductFunctionInterface,
    remap_type: &mut F,
) where
    F: FnMut(&mut crate::types::Type),
{
    for param in &mut function.params {
        remap_type(param);
    }
    remap_type(&mut function.ret_type);
}
