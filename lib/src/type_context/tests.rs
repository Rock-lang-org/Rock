use super::*;
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, TypeVarId};
use crate::type_context::TypeView;
use crate::type_services::kind::Kind;
use crate::type_services::normalize::{
    NormalizationLimits, NormalizeError, TypeNormalizationEnv, TypeNormalizer,
};
use crate::types::{
    AssociatedTypeKey, CallableKind, CaptureKind, FunctionCapture, GenericParamId, NominalTypeKind,
    Type,
};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn generic(owner: DefId, index: u32) -> GenericParamId {
    GenericParamId { owner, index }
}

fn assoc_type(owner: DefId, index: u32) -> AssociatedTypeKey {
    AssociatedTypeKey {
        owner,
        assoc_type_id: AssocTypeId(index),
    }
}

#[test]
fn normalized_interning_uses_one_canonical_type_id() {
    let option = def_id(40);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(option, NominalTypeKind::Enum, 1);
    let constructor = Type::Constructor {
        id: option,
        flavor: NominalTypeKind::Enum,
    };
    let eta_equivalent = Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(Type::Apply {
            constructor: Box::new(constructor.clone()),
            args: vec![Type::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            }],
        }),
    };
    let mut context = TypeContext::new();

    let constructor_id = context.intern_normalized_type(&constructor, &env).unwrap();
    let eta_id = context
        .intern_normalized_type(&eta_equivalent, &env)
        .unwrap();

    assert_eq!(constructor_id, eta_id);

    let generic = generic(def_id(43), 0);
    env.register_generic_kind(generic, Kind::arrow(Kind::Type, Kind::Type));
    let abstract_application = Type::Apply {
        constructor: Box::new(Type::Generic(generic)),
        args: vec![Type::I64],
    };
    let application_id = context
        .intern_normalized_type(&abstract_application, &env)
        .unwrap();
    assert_eq!(context.type_for(application_id), abstract_application);

    let lambda = Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(Type::Tuple(vec![
            Type::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            },
            Type::I64,
        ])),
    };
    let lambda_id = context.intern_normalized_type(&lambda, &env).unwrap();
    assert_eq!(context.type_for(lambda_id), lambda);
    context.validate_canonical_types(&env).unwrap();
}

#[test]
fn constructor_type_ids_retain_kinds_without_becoming_runtime_layouts() {
    let option = def_id(44);
    let unary = Kind::arrow(Kind::Type, Kind::Type);
    let mut env = TypeNormalizationEnv::new();
    env.register_constructor(option, NominalTypeKind::Enum, unary.clone());
    let constructor = Type::Constructor {
        id: option,
        flavor: NominalTypeKind::Enum,
    };
    let mut context = TypeContext::new();

    let constructor_id = context.intern_normalized_type(&constructor, &env).unwrap();
    let applied_id = context
        .intern_normalized_type(
            &Type::Apply {
                constructor: Box::new(constructor.clone()),
                args: vec![Type::I64],
            },
            &env,
        )
        .unwrap();

    assert_eq!(context.kind(constructor_id), &unary);
    assert_eq!(context.kind(applied_id), &Kind::Type);
    assert_eq!(
        crate::type_services::layout::TypeLayout::validate_runtime_type(
            &context.type_for(constructor_id)
        ),
        Err(crate::type_services::layout::RuntimeTypeError::UnsaturatedConstructor)
    );
    assert_eq!(
        context.type_for(applied_id),
        Type::Enum {
            id: option,
            args: vec![Type::I64],
        }
    );
}

#[test]
#[should_panic(expected = "higher-kinded terms must enter TypeContext")]
fn raw_higher_kinded_terms_cannot_bypass_normalization() {
    let mut context = TypeContext::new();
    context.intern_type(&Type::Constructor {
        id: def_id(42),
        flavor: NominalTypeKind::Struct,
    });
}

#[test]
fn normalization_failure_does_not_partially_insert_types() {
    let alias = def_id(41);
    let mut env = TypeNormalizationEnv::new();
    env.register_alias(
        alias,
        Kind::Type,
        Type::Constructor {
            id: alias,
            flavor: NominalTypeKind::Alias,
        },
    );
    let mut context = TypeContext::new();
    let before = context.len();
    let result = context.intern_normalized_type(
        &Type::Constructor {
            id: alias,
            flavor: NominalTypeKind::Alias,
        },
        &env,
    );

    assert!(matches!(result, Err(NormalizeError::AliasCycle(_))));
    assert_eq!(context.len(), before);

    let limited = NormalizationLimits {
        max_depth: 0,
        max_nodes: 1,
    };
    assert!(matches!(
        TypeNormalizer::with_limits(&TypeNormalizationEnv::new(), limited)
            .normalize(&Type::Slice(Box::new(Type::I64))),
        Err(NormalizeError::DepthLimit { .. }) | Err(NormalizeError::NodeLimit { .. })
    ));
    assert_eq!(context.len(), before);
}

#[test]
fn type_view_reports_shape_without_structural_type_storage() {
    let mut context = TypeContext::new();
    let elem = context.intern_type(&Type::U8);
    let slice = context.intern_ty(Ty::Slice(elem));
    let shared_ref = context.intern_ty(Ty::Reference {
        mutable: false,
        inner: slice,
    });
    let view = TypeView::new(&context);

    assert!(view.is_slice_shape(slice));
    assert!(view.is_fat_pointer_shape(shared_ref));
    assert!(view.is_copy(shared_ref));
}

#[test]
fn type_view_exposes_callable_metadata() {
    let mut context = TypeContext::new();
    let capture_ty = context.intern_type(&Type::I64);
    let function = context.intern_ty(Ty::Function {
        params: Vec::new(),
        ret: capture_ty,
        safety: FunctionSafety::Safe,
        callable_kind: CallableKind::FnMut,
        captures: vec![TyFunctionCapture {
            kind: CaptureKind::MutableBorrow,
            effect: crate::types::CaptureEffect::Mutate,
            ty: capture_ty,
        }],
    });
    let view = TypeView::new(&context);

    assert_eq!(view.callable_kind(function), Some(CallableKind::FnMut));
    assert_eq!(view.captures(function).unwrap()[0].ty, capture_ty);
}

#[test]
fn type_context_finds_previously_interned_structural_type_without_mutation() {
    let mut context = TypeContext::new();
    let ty = Type::function(vec![Type::I64], Type::Bool);
    let id = context.intern_type(&ty);

    assert_eq!(context.id_for_type(&ty), Some(id));
    assert_eq!(context.type_for(id), ty);
}

#[test]
fn type_context_preserves_function_safety() {
    let mut context = TypeContext::new();
    let safe = Type::function(vec![Type::I64], Type::Bool);
    let unsafe_fn = Type::unsafe_function(vec![Type::I64], Type::Bool);

    let safe_id = context.intern_type(&safe);
    let unsafe_id = context.intern_type(&unsafe_fn);

    assert_ne!(safe_id, unsafe_id);
    assert_eq!(context.type_for(safe_id), safe);
    assert_eq!(context.type_for(unsafe_id), unsafe_fn);
    assert_eq!(context.id_for_type(&safe), Some(safe_id));
    assert_eq!(context.id_for_type(&unsafe_fn), Some(unsafe_id));
}

#[test]
fn type_context_preserves_callable_metadata_and_capture_types() {
    let mut context = TypeContext::new();
    let ty = Type::function_with_metadata(
        vec![Type::I64],
        Type::Bool,
        FunctionSafety::Safe,
        CallableKind::FnMut,
        vec![FunctionCapture::new(CaptureKind::MutableBorrow, Type::Str)],
    );
    let id = context.intern_type(&ty);

    assert_eq!(context.type_for(id), ty);
    assert_eq!(context.id_for_type(&ty), Some(id));
}

#[test]
fn type_interner_substitutes_generics_to_type_ids() {
    let owner = def_id(70);
    let generic_param = GenericParamId { owner, index: 0 };
    let mut context = TypeContext::new();
    let generic = context.intern_ty(Ty::Generic(generic_param));
    let replacement = context.intern_type(&Type::U64);
    let tuple = context.intern_ty(Ty::Tuple(vec![generic]));
    let mut subst = HashMap::new();
    subst.insert(generic_param, replacement);

    let substituted = context.substitute_generics(tuple, &subst);

    assert_eq!(context.type_for(substituted), Type::Tuple(vec![Type::U64]));
}

#[test]
fn type_interner_substitutes_chained_generics_to_type_ids() {
    let owner = def_id(71);
    let first_param = GenericParamId { owner, index: 0 };
    let second_param = GenericParamId { owner, index: 1 };
    let mut context = TypeContext::new();
    let first = context.intern_ty(Ty::Generic(first_param));
    let second = context.intern_ty(Ty::Generic(second_param));
    let replacement = context.intern_type(&Type::U64);
    let mut subst = HashMap::new();
    subst.insert(first_param, second);
    subst.insert(second_param, replacement);

    let substituted = context.substitute_generics(first, &subst);

    assert_eq!(context.type_for(substituted), Type::U64);
}

#[test]
fn type_interner_substitute_generics_breaks_cycles() {
    let owner = def_id(72);
    let first_param = GenericParamId { owner, index: 0 };
    let second_param = GenericParamId { owner, index: 1 };
    let mut context = TypeContext::new();
    let first = context.intern_ty(Ty::Generic(first_param));
    let second = context.intern_ty(Ty::Generic(second_param));
    let mut subst = HashMap::new();
    subst.insert(first_param, second);
    subst.insert(second_param, first);

    let substituted = context.substitute_generics(first, &subst);

    assert_eq!(context.type_for(substituted), Type::Generic(first_param));
}

#[test]
fn type_interner_does_not_resolve_unused_substitution_ids() {
    let owner = def_id(73);
    let unused_param = GenericParamId { owner, index: 0 };
    let mut context = TypeContext::new();
    let concrete = context.intern_type(&Type::I64);
    let mut subst = HashMap::new();
    subst.insert(unused_param, TypeId(u32::MAX));

    let substituted = context.substitute_generics(concrete, &subst);

    assert_eq!(substituted, concrete);
}

#[test]
fn type_context_interns_same_primitive_once() {
    let mut context = TypeContext::new();

    let first = context.intern_ty(Ty::I64);
    let second = context.intern_ty(Ty::I64);
    let different = context.intern_ty(Ty::Bool);

    assert_eq!(first, second);
    assert_ne!(first, different);
    assert_eq!(context.ty(first), &Ty::I64);
}

#[test]
fn type_context_roundtrips_structural_types() {
    let mut context = TypeContext::new();
    let owner = def_id(30);
    let generic_param = generic(owner, 0);
    let projection_trait = def_id(31);

    let cases = vec![
        Type::I8,
        Type::I16,
        Type::I32,
        Type::I64,
        Type::U8,
        Type::U16,
        Type::U32,
        Type::U64,
        Type::F32,
        Type::F64,
        Type::Bool,
        Type::Str,
        Type::Char,
        Type::Unit,
        Type::Never,
        Type::Error,
        Type::Slice(Box::new(Type::I64)),
        Type::Array(Box::new(Type::Bool), crate::types::ArrayLen::Value(4)),
        Type::Tuple(vec![Type::I64, Type::Bool]),
        Type::function(vec![Type::I64, Type::Bool], Type::Unit),
        Type::Struct {
            id: def_id(32),
            args: vec![Type::Generic(generic_param)],
        },
        Type::Enum {
            id: def_id(33),
            args: vec![Type::I64],
        },
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        },
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::I64),
        },
        Type::Pointer(Box::new(Type::I64)),
        Type::TypeVar(TypeVarId(7)),
        Type::Generic(generic_param),
        Type::Projection {
            ty: Box::new(Type::Struct {
                id: def_id(34),
                args: vec![Type::Generic(generic_param)],
            }),
            trait_id: projection_trait,
            assoc_type: assoc_type(projection_trait, 0),
            trait_args: vec![Type::Reference {
                mutable: false,
                inner: Box::new(Type::Generic(generic_param)),
            }],
        },
    ];

    for ty in cases {
        let id = context.intern_type(&ty);
        let roundtrip = context.type_for(id);

        assert_eq!(roundtrip, ty);
        assert_eq!(context.intern_type(&roundtrip), id);
    }
}

#[test]
fn type_context_intern_type_reuses_equal_structural_types() {
    let mut context = TypeContext::new();
    let first = Type::function(vec![Type::I64], Type::Bool);
    let second = Type::function(vec![Type::I64], Type::Bool);
    let different = Type::function(vec![Type::Bool], Type::Bool);

    let first_id = context.intern_type(&first);
    let second_id = context.intern_type(&second);
    let different_id = context.intern_type(&different);

    assert_eq!(first_id, second_id);
    assert_ne!(first_id, different_id);
}

#[test]
fn type_context_keeps_type_var_ids_distinct_from_type_ids() {
    let mut context = TypeContext::new();

    let first = context.intern_type(&Type::TypeVar(TypeVarId(0)));
    let same = context.intern_type(&Type::TypeVar(TypeVarId(0)));
    let different = context.intern_type(&Type::TypeVar(TypeVarId(1)));

    assert_eq!(first, same);
    assert_ne!(first, different);
    assert_eq!(context.type_for(first), Type::TypeVar(TypeVarId(0)));
}

#[test]
fn type_context_interns_equal_composite_tys() {
    let mut context = TypeContext::new();
    let i64_ty = context.intern_ty(Ty::I64);
    let bool_ty = context.intern_ty(Ty::Bool);

    assert_eq!(
        context.intern_ty(Ty::Slice(i64_ty)),
        context.intern_ty(Ty::Slice(i64_ty))
    );
    assert_eq!(
        context.intern_ty(Ty::Array {
            inner: i64_ty,
            len: crate::types::ArrayLen::Value(4),
        }),
        context.intern_ty(Ty::Array {
            inner: i64_ty,
            len: crate::types::ArrayLen::Value(4),
        })
    );
    assert_ne!(
        context.intern_ty(Ty::Array {
            inner: i64_ty,
            len: crate::types::ArrayLen::Value(4),
        }),
        context.intern_ty(Ty::Array {
            inner: i64_ty,
            len: crate::types::ArrayLen::Value(8),
        })
    );
    assert_eq!(
        context.intern_ty(Ty::Tuple(vec![i64_ty, bool_ty])),
        context.intern_ty(Ty::Tuple(vec![i64_ty, bool_ty]))
    );
    assert_eq!(
        context.intern_ty(Ty::Function {
            params: vec![i64_ty],
            ret: bool_ty,
            safety: FunctionSafety::Safe,
            callable_kind: CallableKind::Fn,
            captures: Vec::new(),
        }),
        context.intern_ty(Ty::Function {
            params: vec![i64_ty],
            ret: bool_ty,
            safety: FunctionSafety::Safe,
            callable_kind: CallableKind::Fn,
            captures: Vec::new(),
        })
    );
    assert_eq!(
        context.intern_ty(Ty::Reference {
            mutable: false,
            inner: i64_ty,
        }),
        context.intern_ty(Ty::Reference {
            mutable: false,
            inner: i64_ty,
        })
    );
    assert_ne!(
        context.intern_ty(Ty::Reference {
            mutable: false,
            inner: i64_ty,
        }),
        context.intern_ty(Ty::Reference {
            mutable: true,
            inner: i64_ty,
        })
    );
    assert_eq!(
        context.intern_ty(Ty::Pointer(i64_ty)),
        context.intern_ty(Ty::Pointer(i64_ty))
    );
}

#[test]
fn type_context_uses_identities_for_nominal_generic_projection_and_type_vars() {
    let mut context = TypeContext::new();
    let owner = def_id(1);
    let other_owner = def_id(2);

    let first_generic = context.intern_ty(Ty::Generic(generic(owner, 0)));
    let same_generic = context.intern_ty(Ty::Generic(generic(owner, 0)));
    let different_owner_generic = context.intern_ty(Ty::Generic(generic(other_owner, 0)));
    let different_index_generic = context.intern_ty(Ty::Generic(generic(owner, 1)));

    assert_eq!(first_generic, same_generic);
    assert_ne!(first_generic, different_owner_generic);
    assert_ne!(first_generic, different_index_generic);

    let type_var = context.intern_ty(Ty::TypeVar(TypeVarId(7)));
    let same_type_var = context.intern_ty(Ty::TypeVar(TypeVarId(7)));
    let different_type_var = context.intern_ty(Ty::TypeVar(TypeVarId(8)));

    assert_eq!(type_var, same_type_var);
    assert_ne!(type_var, different_type_var);

    assert_ne!(
        context.intern_ty(Ty::Struct {
            id: def_id(10),
            args: vec![first_generic],
        }),
        context.intern_ty(Ty::Struct {
            id: def_id(11),
            args: vec![first_generic],
        })
    );
    assert_ne!(
        context.intern_ty(Ty::Enum {
            id: def_id(12),
            args: vec![first_generic],
        }),
        context.intern_ty(Ty::Enum {
            id: def_id(13),
            args: vec![first_generic],
        })
    );

    let trait_id = def_id(20);
    let projection = context.intern_ty(Ty::Projection {
        ty: first_generic,
        trait_id,
        assoc_type: assoc_type(trait_id, 0),
        trait_args: vec![first_generic],
    });

    assert_eq!(
        projection,
        context.intern_ty(Ty::Projection {
            ty: first_generic,
            trait_id,
            assoc_type: assoc_type(trait_id, 0),
            trait_args: vec![first_generic],
        })
    );
    assert_ne!(
        projection,
        context.intern_ty(Ty::Projection {
            ty: different_owner_generic,
            trait_id,
            assoc_type: assoc_type(trait_id, 0),
            trait_args: vec![first_generic],
        })
    );
    assert_ne!(
        projection,
        context.intern_ty(Ty::Projection {
            ty: first_generic,
            trait_id: def_id(21),
            assoc_type: assoc_type(def_id(21), 0),
            trait_args: vec![first_generic],
        })
    );
    assert_ne!(
        projection,
        context.intern_ty(Ty::Projection {
            ty: first_generic,
            trait_id,
            assoc_type: assoc_type(trait_id, 1),
            trait_args: vec![first_generic],
        })
    );
    assert_ne!(
        projection,
        context.intern_ty(Ty::Projection {
            ty: first_generic,
            trait_id,
            assoc_type: assoc_type(trait_id, 0),
            trait_args: vec![different_index_generic],
        })
    );
}
