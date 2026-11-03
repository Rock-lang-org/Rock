use super::body_remap::remap_function_child_locations;
use super::child_location_validator::ProductChildLocationValidator;
use super::generic_remap::{
    remap_generic_bounds, remap_generic_param_decls, remap_predicates, remap_type_def_ids,
};
use super::nominal_validator::ProductNominalTypeValidator;
use super::product_names::qualify_product_name;
use super::ProductIdentityRemap;
use crate::products::{
    ProductDefId, ProductEnumInterface, ProductExternInterface, ProductFunctionInterface,
    ProductImplInterface, ProductStructInterface, ProductTraitInterface, ProductTypeAliasInterface,
};

pub(super) fn remap_function_id(
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

pub(super) fn remap_function_interface_id(
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

pub(super) fn remap_struct_interface_id(
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

pub(super) fn remap_type_alias_interface_id(
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

pub(super) fn remap_enum_interface_id(
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

pub(super) fn remap_trait_interface_ids(
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

pub(super) fn remap_impl_interface_ids(
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

pub(super) fn remap_extern_interface_id(
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

pub(super) fn remap_trait_ids(
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

pub(super) fn remap_impl_ids(
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
