use std::collections::HashMap;

use std::collections::HashSet;

use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, TypeVarId};

use super::{
    validate_supertrait_graph, AssociatedTypeKey, CallableKind, CaptureKind, FunctionCapture,
    FunctionSafety, GenericParamId, Predicate, TraitBound, Type,
};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

#[test]
fn generic_param_identity_is_owner_and_index() {
    let first_owner = def_id(1);
    let second_owner = def_id(2);
    let first = GenericParamId {
        owner: first_owner,
        index: 0,
    };
    let same = GenericParamId {
        owner: first_owner,
        index: 0,
    };
    let different_owner = GenericParamId {
        owner: second_owner,
        index: 0,
    };
    let different_index = GenericParamId {
        owner: first_owner,
        index: 1,
    };

    let identities = HashSet::from([first, same, different_owner, different_index]);

    assert_eq!(first, same);
    assert_eq!(identities.len(), 3);
}

#[test]
fn associated_type_key_is_owner_scoped() {
    let first = AssociatedTypeKey {
        owner: def_id(1),
        assoc_type_id: AssocTypeId(0),
    };
    let same = AssociatedTypeKey {
        owner: def_id(1),
        assoc_type_id: AssocTypeId(0),
    };
    let different_owner = AssociatedTypeKey {
        owner: def_id(2),
        assoc_type_id: AssocTypeId(0),
    };
    let different_assoc = AssociatedTypeKey {
        owner: def_id(1),
        assoc_type_id: AssocTypeId(1),
    };

    let identities = HashSet::from([first, same, different_owner, different_assoc]);

    assert_eq!(first, same);
    assert_eq!(identities.len(), 3);
}

#[test]
fn trait_bound_identity_uses_trait_def_id() {
    let first = TraitBound {
        trait_id: def_id(1),
        type_args: vec![Type::I64],
    };
    let same = TraitBound {
        trait_id: def_id(1),
        type_args: vec![Type::I64],
    };
    let different_trait = TraitBound {
        trait_id: def_id(2),
        type_args: vec![Type::I64],
    };

    assert_eq!(first, same);
    assert_ne!(first, different_trait);
}

#[test]
fn supertrait_cycles_are_detected_by_canonical_trait_id() {
    let first = def_id(20);
    let second = def_id(21);
    let first_predicates = vec![Predicate::Trait {
        subject: Type::Generic(GenericParamId {
            owner: first,
            index: 0,
        }),
        trait_id: second,
        args: Vec::new(),
    }];
    let second_predicates = vec![Predicate::Trait {
        subject: Type::Generic(GenericParamId {
            owner: second,
            index: 0,
        }),
        trait_id: first,
        args: Vec::new(),
    }];

    let errors = validate_supertrait_graph([
        (first, "First", first_predicates.as_slice()),
        (second, "Second", second_predicates.as_slice()),
    ]);

    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("First"));
    assert!(errors[0].contains("Second"));
    assert!(!errors[0].contains("trait#"));
}

#[test]
fn same_spelled_traits_remain_distinct_in_supertrait_graph() {
    let first = def_id(22);
    let second = def_id(23);
    let predicates = vec![Predicate::Trait {
        subject: Type::I64,
        trait_id: second,
        args: Vec::new(),
    }];

    assert!(validate_supertrait_graph([
        (first, "pkg::Marker", predicates.as_slice()),
        (second, "other::Marker", [].as_slice()),
    ])
    .is_empty());
}

#[test]
fn projection_identity_uses_trait_and_assoc_type_ids() {
    let base = Type::Generic(GenericParamId {
        owner: def_id(10),
        index: 0,
    });
    let first = Type::Projection {
        ty: Box::new(base.clone()),
        trait_id: def_id(1),
        assoc_type: AssociatedTypeKey {
            owner: def_id(1),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![Type::I64],
    };
    let same = Type::Projection {
        ty: Box::new(base.clone()),
        trait_id: def_id(1),
        assoc_type: AssociatedTypeKey {
            owner: def_id(1),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![Type::I64],
    };
    let different_trait = Type::Projection {
        ty: Box::new(base),
        trait_id: def_id(2),
        assoc_type: AssociatedTypeKey {
            owner: def_id(2),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![Type::I64],
    };

    assert_eq!(first, same);
    assert_ne!(first, different_trait);
}

#[test]
fn identity_key_remap_updates_owner_fields() {
    let mut generic = GenericParamId {
        owner: def_id(1),
        index: 0,
    };
    let mut assoc = AssociatedTypeKey {
        owner: def_id(2),
        assoc_type_id: AssocTypeId(0),
    };
    let mut visited = Vec::new();
    let mut remap = |id: DefId| {
        visited.push(id);
        DefId::new(CrateId(id.crate_id.0 + 10), LocalDefId(id.local.0 + 100))
    };

    generic.remap_def_ids(&mut remap);
    assoc.remap_def_ids(&mut remap);

    assert_eq!(generic.owner, DefId::new(CrateId(10), LocalDefId(101)));
    assert_eq!(assoc.owner, DefId::new(CrateId(10), LocalDefId(102)));
    assert_eq!(visited, vec![def_id(1), def_id(2)]);
}

#[test]
fn same_named_nominal_types_are_distinct_by_def_id() {
    let first = Type::Struct {
        id: def_id(1),
        args: vec![Type::I64],
    };
    let second = Type::Struct {
        id: def_id(2),
        args: vec![Type::I64],
    };

    assert_ne!(first, second);
}

#[test]
fn generic_params_with_same_name_are_distinct_by_owner() {
    let first_owner_t = GenericParamId {
        owner: def_id(1),
        index: 0,
    };
    let second_owner_t = GenericParamId {
        owner: def_id(2),
        index: 0,
    };

    assert_ne!(Type::Generic(first_owner_t), Type::Generic(second_owner_t));
}

#[test]
fn substitute_generics_uses_generic_param_identity() {
    let first_owner_t = GenericParamId {
        owner: def_id(1),
        index: 0,
    };
    let second_owner_t = GenericParamId {
        owner: def_id(2),
        index: 0,
    };
    let ty = Type::function(
        vec![Type::Generic(first_owner_t)],
        Type::Generic(second_owner_t),
    );
    let subst = HashMap::from([(first_owner_t, Type::I64), (second_owner_t, Type::Bool)]);

    assert_eq!(
        ty.substitute_generics(&subst),
        Type::function(vec![Type::I64], Type::Bool)
    );
}

#[test]
fn type_def_id_remap_updates_nominal_ids_and_nested_args() {
    let generic = GenericParamId {
        owner: def_id(4),
        index: 0,
    };
    let mut ty = Type::function(
        vec![Type::Struct {
            id: def_id(1),
            args: vec![Type::Generic(generic)],
        }],
        Type::Projection {
            ty: Box::new(Type::Enum {
                id: def_id(2),
                args: vec![Type::Struct {
                    id: def_id(3),
                    args: Vec::new(),
                }],
            }),
            trait_id: def_id(5),
            assoc_type: AssociatedTypeKey {
                owner: def_id(5),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![Type::Generic(generic)],
        },
    );
    let mut visited = Vec::new();
    let mut remap = |id: DefId| {
        visited.push(id);
        DefId::new(CrateId(id.crate_id.0 + 10), LocalDefId(id.local.0 + 100))
    };

    ty.remap_def_ids(&mut remap);

    assert_eq!(
        ty,
        Type::function(
            vec![Type::Struct {
                id: DefId::new(CrateId(10), LocalDefId(101)),
                args: vec![Type::Generic(GenericParamId {
                    owner: DefId::new(CrateId(10), LocalDefId(104)),
                    index: 0,
                })],
            }],
            Type::Projection {
                ty: Box::new(Type::Enum {
                    id: DefId::new(CrateId(10), LocalDefId(102)),
                    args: vec![Type::Struct {
                        id: DefId::new(CrateId(10), LocalDefId(103)),
                        args: Vec::new(),
                    }],
                }),
                trait_id: DefId::new(CrateId(10), LocalDefId(105)),
                assoc_type: AssociatedTypeKey {
                    owner: DefId::new(CrateId(10), LocalDefId(105)),
                    assoc_type_id: AssocTypeId(0),
                },
                trait_args: vec![Type::Generic(GenericParamId {
                    owner: DefId::new(CrateId(10), LocalDefId(104)),
                    index: 0,
                })],
            }
        )
    );
    assert_eq!(
        visited,
        vec![
            def_id(1),
            def_id(4),
            def_id(5),
            def_id(5),
            def_id(2),
            def_id(3),
            def_id(4)
        ]
    );
}

#[test]
fn type_def_id_remap_updates_projection_type_ids() {
    let mut ty = Type::Projection {
        ty: Box::new(Type::Generic(GenericParamId {
            owner: def_id(1),
            index: 0,
        })),
        trait_id: def_id(2),
        assoc_type: AssociatedTypeKey {
            owner: def_id(2),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![Type::Struct {
            id: def_id(3),
            args: vec![Type::Generic(GenericParamId {
                owner: def_id(4),
                index: 1,
            })],
        }],
    };

    ty.remap_def_ids(&mut |id| def_id(id.local.0 + 10));

    assert_eq!(
        ty,
        Type::Projection {
            ty: Box::new(Type::Generic(GenericParamId {
                owner: def_id(11),
                index: 0,
            })),
            trait_id: def_id(12),
            assoc_type: AssociatedTypeKey {
                owner: def_id(12),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![Type::Struct {
                id: def_id(13),
                args: vec![Type::Generic(GenericParamId {
                    owner: def_id(14),
                    index: 1,
                })],
            }],
        }
    );
}

#[test]
fn substitute_uses_typed_type_var_id_keys() {
    let ty = Type::function(
        vec![Type::TypeVar(TypeVarId(0))],
        Type::TypeVar(TypeVarId(1)),
    );
    let subst = HashMap::from([(TypeVarId(0), Type::I64), (TypeVarId(1), Type::Bool)]);

    assert_eq!(
        ty.substitute(&subst),
        Type::function(vec![Type::I64], Type::Bool)
    );
}

#[test]
fn test_display_formats_slice_and_fixed_array_differently() {
    let slice = Type::Slice(Box::new(Type::I64));
    let array = Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Value(4));

    assert_eq!(slice.to_string(), "[I64]");
    assert_eq!(array.to_string(), "[I64; 4]");
}

#[test]
fn test_display_formats_str_with_rock_spelling() {
    assert_eq!(Type::Str.to_string(), "Str");
    assert_eq!(
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        }
        .to_string(),
        "&Str"
    );
}

#[test]
fn callable_kinds_are_ordered_by_capability() {
    assert!(CallableKind::Fn < CallableKind::FnMut);
    assert!(CallableKind::FnMut < CallableKind::FnOnce);
    assert_eq!(
        CallableKind::from_captures(&[FunctionCapture::new(CaptureKind::Move, Type::I64)]),
        CallableKind::Fn
    );
    assert_eq!(
        CallableKind::from_captures(&[FunctionCapture::new(
            CaptureKind::Move,
            Type::Struct {
                id: def_id(91),
                args: Vec::new(),
            }
        )]),
        CallableKind::FnOnce
    );
}

#[test]
fn function_type_substitution_preserves_capture_types() {
    let generic = GenericParamId {
        owner: def_id(90),
        index: 0,
    };
    let function = Type::function_with_metadata(
        vec![Type::Generic(generic)],
        Type::Bool,
        FunctionSafety::Safe,
        CallableKind::FnMut,
        vec![FunctionCapture::new(
            CaptureKind::MutableBorrow,
            Type::Generic(generic),
        )],
    );
    let substituted = function.substitute_generics(&HashMap::from([(generic, Type::I64)]));

    assert_eq!(
        substituted,
        Type::function_with_metadata(
            vec![Type::I64],
            Type::Bool,
            FunctionSafety::Safe,
            CallableKind::FnMut,
            vec![FunctionCapture::new(CaptureKind::MutableBorrow, Type::I64)],
        )
    );
}
