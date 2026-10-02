use crate::type_services::projection::{ProjectionNormalizer, ProjectionProvider};
use crate::types::{GenericParamId, Type};

pub struct TypeLayout;

/// Whether a type has a fixed size for each valid instantiation.
/// This does not imply that its layout is known or valid for concrete MIR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sizedness {
    Sized,
    Unsized,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeTypeError {
    ObjectRequiresEvidence,
    UnboundObjectSelf,
    WitnessRequiresScopedDescriptor,
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
    /// Classify a normalized type using the caller's generic-bound evidence.
    /// Missing evidence remains unknown rather than proving unsizedness.
    pub fn sizedness(
        ty: &Type,
        generic_sizedness: &impl Fn(GenericParamId) -> Sizedness,
    ) -> Sizedness {
        match ty {
            Type::Slice(_) | Type::Str | Type::Object(_) => Sizedness::Unsized,
            Type::Array(inner, _) => Self::sizedness(inner, generic_sizedness),
            Type::Tuple(elements) => {
                let mut result = Sizedness::Sized;
                for element in elements {
                    match Self::sizedness(element, generic_sizedness) {
                        Sizedness::Unsized => return Sizedness::Unsized,
                        Sizedness::Unknown => result = Sizedness::Unknown,
                        Sizedness::Sized => {}
                    }
                }
                result
            }
            Type::I8
            | Type::ObjectSelf { .. }
            | Type::Witness(_)
            | Type::I16
            | Type::I32
            | Type::I64
            | Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::F32
            | Type::F64
            | Type::Bool
            | Type::Char
            | Type::Unit
            | Type::Never
            | Type::Struct { .. }
            | Type::Enum { .. }
            | Type::Reference { .. }
            | Type::Pointer(_)
            | Type::Function { .. } => Sizedness::Sized,
            Type::Generic(param) => generic_sizedness(*param),
            Type::Projection { .. }
            | Type::TypeVar(_)
            | Type::Constructor { .. }
            | Type::Apply { .. }
            | Type::Lambda { .. }
            | Type::BoundVar { .. }
            | Type::Error => Sizedness::Unknown,
        }
    }

    pub fn validate_runtime_type(ty: &Type) -> Result<(), RuntimeTypeError> {
        let mut result = Ok(());
        crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
            if result.is_err() {
                return;
            }
            result = match nested {
                Type::Object(_) => Err(RuntimeTypeError::ObjectRequiresEvidence),
                Type::ObjectSelf { .. } => Err(RuntimeTypeError::UnboundObjectSelf),
                Type::Witness(_) => Err(RuntimeTypeError::WitnessRequiresScopedDescriptor),
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
mod tests {
    use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
    use crate::type_services::projection::{ProjectionAssociatedType, ProjectionImpl};
    use crate::types::AssociatedTypeKey;

    use super::*;

    struct NoProjectionProvider;

    impl ProjectionProvider for NoProjectionProvider {
        fn find_projection_impl(
            &self,
            _base_ty: &Type,
            _trait_id: DefId,
            _trait_args: &[Type],
        ) -> Option<ProjectionImpl> {
            None
        }
    }

    struct SliceProjectionProvider {
        trait_id: DefId,
        assoc_type_id: AssocTypeId,
    }

    impl ProjectionProvider for SliceProjectionProvider {
        fn find_projection_impl(
            &self,
            base_ty: &Type,
            trait_id: DefId,
            trait_args: &[Type],
        ) -> Option<ProjectionImpl> {
            if trait_id != self.trait_id {
                return None;
            }

            Some(ProjectionImpl {
                impl_id: def_id(30),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(base_ty.clone()),
                trait_arg_types: trait_args.to_vec(),
                associated_types: vec![ProjectionAssociatedType {
                    id: self.assoc_type_id,
                    name: "Output".to_string(),
                    ty: Type::Slice(Box::new(Type::U8)),
                }],
            })
        }
    }

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    #[test]
    fn sizedness_preserves_missing_generic_evidence_in_aggregates() {
        let param = GenericParamId {
            owner: def_id(1),
            index: 0,
        };
        let aggregate = Type::Tuple(vec![
            Type::I64,
            Type::Array(Box::new(Type::Generic(param)), 2),
        ]);

        assert_eq!(
            TypeLayout::sizedness(&aggregate, &|_| Sizedness::Unknown),
            Sizedness::Unknown
        );
        assert_eq!(
            TypeLayout::sizedness(&aggregate, &|id| {
                assert_eq!(id, param);
                Sizedness::Sized
            }),
            Sizedness::Sized
        );

        // Definite unsizedness must not depend on the order of unresolved fields.
        for elements in [
            vec![aggregate.clone(), Type::Str],
            vec![Type::Str, aggregate],
        ] {
            assert_eq!(
                TypeLayout::sizedness(&Type::Tuple(elements), &|_| Sizedness::Unknown),
                Sizedness::Unsized
            );
        }
    }

    #[test]
    fn sized_indirection_does_not_admit_unresolved_concrete_mir() {
        let generic = Type::Generic(GenericParamId {
            owner: def_id(2),
            index: 0,
        });
        for handle in [
            Type::Reference {
                mutable: false,
                inner: Box::new(generic.clone()),
            },
            Type::Pointer(Box::new(generic)),
        ] {
            assert_eq!(
                TypeLayout::sizedness(&handle, &|_| panic!("pointee evidence is unnecessary")),
                Sizedness::Sized
            );
            assert_eq!(
                TypeLayout::validate_runtime_type(&handle),
                Err(RuntimeTypeError::Generic)
            );
        }

        let slice_ref = Type::Reference {
            mutable: true,
            inner: Box::new(Type::Slice(Box::new(Type::U8))),
        };
        assert_eq!(
            TypeLayout::sizedness(&slice_ref, &|_| Sizedness::Unknown),
            Sizedness::Sized
        );
        assert!(TypeLayout::is_fat_pointer_shape(&slice_ref));
        assert_eq!(TypeLayout::validate_runtime_type(&slice_ref), Ok(()));
    }

    #[test]
    fn type_layout_identifies_slice_and_fat_pointer_shapes() {
        let provider = NoProjectionProvider;
        let trait_id = def_id(10);
        let assoc_type_id = AssocTypeId(0);
        let projection_provider = SliceProjectionProvider {
            trait_id,
            assoc_type_id,
        };
        let slice = Type::Slice(Box::new(Type::U8));
        let str_ref = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        };
        let slice_ptr = Type::Pointer(Box::new(Type::Slice(Box::new(Type::I64))));
        let projection_slice = Type::Projection {
            ty: Box::new(Type::Struct {
                id: def_id(20),
                args: Vec::new(),
            }),
            trait_id,
            assoc_type: AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: Vec::new(),
        };

        assert!(TypeLayout::is_slice_shape(&slice));
        assert!(TypeLayout::is_slice_type(&provider, &slice));
        assert!(TypeLayout::is_slice_type(
            &projection_provider,
            &projection_slice
        ));
        assert!(TypeLayout::is_fat_pointer_shape(&str_ref));
        assert!(TypeLayout::is_fat_pointer_shape(&slice_ptr));
        assert!(TypeLayout::is_fat_pointer_type(&provider, &str_ref));
        assert!(!TypeLayout::is_fat_pointer_type(
            &provider,
            &Type::Pointer(Box::new(Type::I64))
        ));
    }

    #[test]
    fn type_layout_rejects_compile_time_constructor_terms() {
        let constructor = Type::Constructor {
            id: def_id(40),
            flavor: crate::types::NominalTypeKind::Struct,
        };
        assert_eq!(
            TypeLayout::validate_runtime_type(&constructor),
            Err(RuntimeTypeError::UnsaturatedConstructor)
        );
        assert_eq!(
            TypeLayout::validate_runtime_type(&Type::Apply {
                constructor: Box::new(constructor),
                args: vec![Type::I64],
            }),
            Err(RuntimeTypeError::AbstractApplication)
        );
        assert_eq!(
            TypeLayout::validate_runtime_type(&Type::Lambda {
                params: vec![crate::type_services::kind::Kind::Type],
                body: Box::new(Type::I64),
            }),
            Err(RuntimeTypeError::TypeLambda)
        );

        let projection = Type::Projection {
            ty: Box::new(Type::I64),
            trait_id: def_id(41),
            assoc_type: AssociatedTypeKey {
                owner: def_id(41),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: Vec::new(),
        };
        let applied_projection = Type::Apply {
            constructor: Box::new(projection),
            args: vec![Type::I64],
        };
        assert!(!crate::type_services::facts::TypeFacts::is_concrete(
            &applied_projection
        ));
        assert_eq!(
            TypeLayout::validate_runtime_type(&applied_projection),
            Err(RuntimeTypeError::AbstractApplication)
        );
    }
}
