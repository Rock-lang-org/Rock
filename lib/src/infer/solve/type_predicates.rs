use crate::types::Type;

pub(super) fn try_head_is_pending(ty: &Type) -> bool {
    match ty {
        Type::TypeVar(_) | Type::Projection { .. } | Type::Error => true,
        Type::Apply { constructor, .. } => try_head_is_pending(constructor),
        _ => false,
    }
}

pub(super) fn contains_unresolved_type(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| {
        matches!(
            nested,
            Type::TypeVar(_)
                | Type::Generic(_)
                | Type::Projection { .. }
                | Type::BoundVar { .. }
                | Type::Error
        )
    })
}

pub(super) fn contains_inference_pending_type(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| match nested {
        Type::TypeVar(_) | Type::Projection { .. } | Type::Error => true,
        Type::Apply { constructor, args } => {
            contains_inference_pending_type(constructor)
                || args.iter().any(contains_inference_pending_type)
        }
        _ => false,
    })
}
