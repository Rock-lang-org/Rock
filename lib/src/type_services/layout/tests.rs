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
