use std::collections::BTreeSet;

use super::child_location_validator::{
    ProductChildLocationValidator, MAX_ARTIFACT_HIR_REMAP_DEPTH,
};
use super::generic_remap::remap_type_def_ids;
use super::nominal_validator::ProductNominalTypeValidator;
use super::ProductIdentityRemap;
use crate::hir::HirMethodCallTarget;
use crate::products::ProductDefId;

pub(super) fn remap_var_target_child_location(
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

pub(super) fn remap_call_target_child_location(
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

pub(super) fn remap_method_target_child_location(
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

pub(super) fn remap_pattern_child_locations(
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
