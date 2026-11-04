pub(crate) use super::type_patterns::type_pattern_matches_after_subst;
use super::*;
use crate::hir::{HirImpl, HirImplOwner, HirMethodCallTarget};
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::types::{AssociatedTypeKey, Type};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

#[test]
fn canonical_nominal_receiver_pattern_matches_concrete_argument() {
    let impl_id = def_id(70);
    let nominal_id = def_id(69);
    let pattern = HirImplReceiverPattern::Exact(Type::Struct {
        id: nominal_id,
        args: vec![Type::Generic(GenericParamId {
            owner: impl_id,
            index: 0,
        })],
    });
    let receiver = Type::Struct {
        id: nominal_id,
        args: vec![Type::U8],
    };

    assert_eq!(
        receiver_pattern_substitution(&pattern, &receiver),
        Some(HashMap::from([(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            Type::U8,
        )])),
    );
}

#[test]
fn ordinary_receiver_mismatches_do_not_visit_the_impl_registry() {
    let nominal = |id, args| Type::Struct {
        id: def_id(id),
        args,
    };
    for (expected, actual) in [
        (Type::I64, Type::Bool),
        (nominal(70, vec![]), nominal(71, vec![])),
        (nominal(70, vec![Type::I64]), nominal(70, vec![Type::Bool])),
    ] {
        let imp = HirImpl {
            id: def_id(72),
            owner: HirImplOwner::Named("test".to_string()),
            type_name: "test".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(expected),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: Default::default(),
            methods: HashMap::new(),
        };
        let registry = std::iter::once(&imp).inspect(|_| {
            panic!("ordinary mismatches must not enumerate impls");
        });
        assert!(impl_receiver_pattern_substitution(&imp, &actual, registry).is_none());
    }
}

#[test]
fn higher_kinded_blanket_matches_constructor_sections_without_guessing() {
    use crate::type_services::kind::Kind;
    use crate::types::{NominalTypeKind, TraitBound};

    let make_impl = |id, trait_id, receiver_pattern| HirImpl {
        id,
        owner: HirImplOwner::Named("test".to_string()),
        type_name: "test".to_string(),
        type_generics: Vec::new(),
        receiver_pattern,
        trait_name: None,
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: Default::default(),
        methods: HashMap::new(),
    };
    let head = GenericParamId {
        owner: def_id(80),
        index: 0,
    };
    let item = GenericParamId {
        owner: def_id(80),
        index: 1,
    };
    let foldable = def_id(81);
    let mut blanket = make_impl(
        def_id(80),
        def_id(82),
        HirImplReceiverPattern::Exact(Type::Apply {
            constructor: Box::new(Type::Generic(head)),
            args: vec![Type::Generic(item)],
        }),
    );
    blanket.bounds.insert(
        head,
        vec![TraitBound {
            trait_id: foldable,
            type_args: vec![],
        }],
    );
    let option = make_impl(
        def_id(83),
        foldable,
        HirImplReceiverPattern::Constructor(Type::Constructor {
            id: def_id(84),
            flavor: NominalTypeKind::Enum,
        }),
    );
    let option_value = Type::Enum {
        id: def_id(84),
        args: vec![Type::I64],
    };
    let substitution =
        impl_receiver_pattern_substitution(&blanket, &option_value, [&option]).unwrap();
    assert_eq!(substitution[&item], Type::I64);
    assert!(impl_receiver_pattern_substitution(&blanket, &option_value, []).is_none());

    let section = |left_hole| Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(Type::Enum {
            id: def_id(85),
            args: if left_hole {
                vec![
                    Type::BoundVar {
                        depth: 0,
                        index: 0,
                        kind: Kind::Type,
                    },
                    Type::Bool,
                ]
            } else {
                vec![
                    Type::I64,
                    Type::BoundVar {
                        depth: 0,
                        index: 0,
                        kind: Kind::Type,
                    },
                ]
            },
        }),
    };
    let result = make_impl(
        def_id(86),
        foldable,
        HirImplReceiverPattern::Constructor(section(true)),
    );
    let result_value = Type::Enum {
        id: def_id(85),
        args: vec![Type::I64, Type::Bool],
    };
    let substitution =
        impl_receiver_pattern_substitution(&blanket, &result_value, [&result]).unwrap();
    assert_eq!(substitution[&head], section(true));
    assert_eq!(substitution[&item], Type::I64);
    let other_section = make_impl(
        def_id(87),
        foldable,
        HirImplReceiverPattern::Constructor(section(false)),
    );
    assert!(
        impl_receiver_pattern_substitution(&blanket, &result_value, [&result, &other_section])
            .is_none()
    );

    let inner = GenericParamId {
        owner: def_id(80),
        index: 2,
    };
    let nested_pattern = |inner_head| {
        HirImplReceiverPattern::Exact(Type::Apply {
            constructor: Box::new(Type::Generic(head)),
            args: vec![Type::Apply {
                constructor: Box::new(Type::Generic(inner_head)),
                args: vec![Type::Generic(item)],
            }],
        })
    };
    blanket.receiver_pattern = nested_pattern(inner);
    let inner_bounds = blanket.bounds[&head].clone();
    blanket.bounds.insert(inner, inner_bounds);
    let nested_value = Type::Enum {
        id: def_id(84),
        args: vec![result_value],
    };
    let substitution =
        impl_receiver_pattern_substitution(&blanket, &nested_value, [&option, &result]).unwrap();
    assert_eq!(substitution[&item], Type::I64);
    assert_eq!(substitution[&inner], section(true));
    assert_eq!(
        substitution[&head],
        Type::Constructor {
            id: def_id(84),
            flavor: NominalTypeKind::Enum,
        }
    );
    assert!(impl_receiver_pattern_substitution(&blanket, &nested_value, [&option]).is_none());
    assert!(impl_receiver_pattern_substitution(
        &blanket,
        &nested_value,
        [&option, &result, &other_section]
    )
    .is_none());

    // A repeated head must denote the same constructor at both levels.
    blanket.receiver_pattern = nested_pattern(head);
    assert!(
        impl_receiver_pattern_substitution(&blanket, &nested_value, [&option, &result]).is_none()
    );
}

#[test]
fn selection_receiver_pattern_cover_nominal_array_slice_and_ref() {
    assert_eq!(
        receiver_pattern(&Type::Array(Box::new(Type::U8), 4)),
        vec![Type::U8]
    );
    assert_eq!(
        receiver_pattern(&Type::Reference {
            mutable: false,
            inner: Box::new(Type::Slice(Box::new(Type::I64))),
        }),
        vec![Type::I64]
    );
    assert_eq!(receiver_pattern(&Type::Str), Vec::<Type>::new());
}

#[test]
fn type_pattern_matches_allows_safe_callable_for_unsafe_slot() {
    assert!(type_pattern_matches_after_subst(
        &Type::unsafe_function(vec![Type::I64], Type::Bool),
        &Type::function(vec![Type::I64], Type::Bool),
    ));
    assert!(!type_pattern_matches_after_subst(
        &Type::function(vec![Type::I64], Type::Bool),
        &Type::unsafe_function(vec![Type::I64], Type::Bool),
    ));
}

#[test]
fn type_pattern_matches_allows_nested_safe_callable_for_unsafe_slot() {
    assert!(type_pattern_matches_after_subst(
        &Type::Tuple(vec![
            Type::unsafe_function(vec![Type::I64], Type::Bool),
            Type::I64,
        ]),
        &Type::Tuple(vec![Type::function(vec![Type::I64], Type::Bool), Type::I64,]),
    ));
}

#[test]
fn type_pattern_matches_treats_empty_capture_signature_metadata_as_wildcard() {
    let actual = Type::function_with_metadata(
        vec![Type::I64],
        Type::Bool,
        FunctionSafety::Safe,
        crate::types::CallableKind::FnMut,
        vec![crate::types::FunctionCapture::new(
            crate::types::CaptureKind::MutableBorrow,
            Type::I64,
        )],
    );

    assert!(type_pattern_matches_after_subst(
        &Type::function(vec![Type::I64], Type::Bool),
        &actual,
    ));
}

#[test]
fn type_pattern_matches_uses_contravariant_function_params() {
    assert!(!type_pattern_matches_after_subst(
        &Type::function(
            vec![Type::unsafe_function(Vec::new(), Type::I64)],
            Type::I64
        ),
        &Type::function(vec![Type::function(Vec::new(), Type::I64)], Type::I64),
    ));
    assert!(type_pattern_matches_after_subst(
        &Type::function(vec![Type::function(Vec::new(), Type::I64)], Type::I64),
        &Type::function(
            vec![Type::unsafe_function(Vec::new(), Type::I64)],
            Type::I64
        ),
    ));
}

#[test]
fn type_pattern_matches_recurses_projection_args() {
    let trait_id = def_id(10);
    let assoc_type = AssociatedTypeKey {
        owner: trait_id,
        assoc_type_id: AssocTypeId(0),
    };

    assert!(type_pattern_matches_after_subst(
        &Type::Projection {
            ty: Box::new(Type::I64),
            trait_id,
            assoc_type,
            trait_args: vec![Type::unsafe_function(Vec::new(), Type::I64)],
        },
        &Type::Projection {
            ty: Box::new(Type::I64),
            trait_id,
            assoc_type,
            trait_args: vec![Type::function(Vec::new(), Type::I64)],
        },
    ));
}

#[test]
fn type_pattern_matches_treats_mut_ref_and_pointer_as_invariant() {
    assert!(!type_pattern_matches_after_subst(
        &Type::Reference {
            mutable: true,
            inner: Box::new(Type::unsafe_function(Vec::new(), Type::I64)),
        },
        &Type::Reference {
            mutable: true,
            inner: Box::new(Type::function(Vec::new(), Type::I64)),
        },
    ));
    assert!(!type_pattern_matches_after_subst(
        &Type::Pointer(Box::new(Type::unsafe_function(Vec::new(), Type::I64))),
        &Type::Pointer(Box::new(Type::function(Vec::new(), Type::I64))),
    ));
}

#[test]
fn selection_target_matching_rejects_wrong_impl_identity() {
    let imp = HirImpl {
        id: def_id(1),
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: std::collections::HashMap::new(),
    };
    let target = HirMethodCallTarget::impl_method(def_id(2), def_id(3), None);

    assert!(!target_matches_impl(&imp, Some(&target)));
    assert!(target_matches_impl(&imp, None));
}

#[test]
fn selection_target_matching_rejects_wrong_trait_identity_when_impl_matches() {
    let imp = HirImpl {
        id: def_id(1),
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: Some("RightTrait".to_string()),
        trait_id: Some(def_id(10)),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: std::collections::HashMap::new(),
    };
    let target = HirMethodCallTarget::impl_method(
        def_id(1),
        def_id(3),
        Some(crate::hir::HirSelectedTraitMember {
            trait_id: def_id(11),
            member_id: def_id(3),
            trait_args: Vec::new(),
        }),
    );

    assert!(!target_matches_impl(&imp, Some(&target)));
}
