use crate::type_services::projection::{ProjectionNormalizer, ProjectionProvider};
use crate::types::Type;

pub struct TypeLayout;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeTypeError {
    UnsaturatedConstructor,
    AbstractApplication,
    TypeLambda,
    BoundVariable,
    UnresolvedProjection,
    Generic,
    InferenceVariable,
    RecoveryType,
}

impl TypeLayout {
    pub fn validate_runtime_type(ty: &Type) -> Result<(), RuntimeTypeError> {
        let mut result = Ok(());
        crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
            if result.is_err() {
                return;
            }
            result = match nested {
                Type::Constructor { .. } => Err(RuntimeTypeError::UnsaturatedConstructor),
                Type::Apply { .. } => Err(RuntimeTypeError::AbstractApplication),
                Type::Lambda { .. } => Err(RuntimeTypeError::TypeLambda),
                Type::BoundVar { .. } => Err(RuntimeTypeError::BoundVariable),
                Type::Projection { .. } => Err(RuntimeTypeError::UnresolvedProjection),
                Type::Generic(_) => Err(RuntimeTypeError::Generic),
                Type::TypeVar(_) => Err(RuntimeTypeError::InferenceVariable),
                Type::Error => Err(RuntimeTypeError::RecoveryType),
                _ => Ok(()),
            };
        });
        result
    }

    pub fn is_slice_shape(ty: &Type) -> bool {
        matches!(ty, Type::Slice(_))
    }

    pub fn is_str_shape(ty: &Type) -> bool {
        matches!(ty, Type::Str)
    }

    pub fn is_fat_pointer_shape(ty: &Type) -> bool {
        match ty {
            Type::Reference { inner, .. } | Type::Pointer(inner) => {
                matches!(inner.as_ref(), Type::Slice(_) | Type::Str)
            }
            _ => false,
        }
    }

    pub fn is_slice_type<P: ProjectionProvider + ?Sized>(provider: &P, ty: &Type) -> bool {
        matches!(
            ProjectionNormalizer::normalize(provider, ty),
            Type::Slice(_)
        )
    }

    pub fn is_fat_pointer_type<P: ProjectionProvider + ?Sized>(provider: &P, ty: &Type) -> bool {
        Self::is_fat_pointer_shape(&ProjectionNormalizer::normalize(provider, ty))
    }
}

#[cfg(test)]
mod tests;
