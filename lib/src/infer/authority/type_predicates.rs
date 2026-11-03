use crate::types::Type;

pub(super) fn contains_recovery_type(ty: &Type) -> bool {
    let mut contains_recovery = false;
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        contains_recovery |= matches!(nested, Type::TypeVar(_) | Type::Error);
    });
    contains_recovery
}

pub(super) fn contains_inference_type(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested: &Type| {
        matches!(nested, Type::TypeVar(_) | Type::Generic(_) | Type::Error)
    })
}

pub(super) fn try_carrier_has_unknown_head(ty: &Type) -> bool {
    matches!(
        ty,
        Type::TypeVar(_)
            | Type::Generic(_)
            | Type::Projection { .. }
            | Type::Apply { .. }
            | Type::Constructor { .. }
            | Type::Lambda { .. }
            | Type::BoundVar { .. }
            | Type::Error
    )
}

pub(super) fn try_type_head_is_unresolved(ty: &Type) -> bool {
    matches!(
        ty,
        Type::TypeVar(_)
            | Type::Generic(_)
            | Type::Projection { .. }
            | Type::Apply { .. }
            | Type::Constructor { .. }
            | Type::Lambda { .. }
            | Type::BoundVar { .. }
            | Type::Error
    )
}
