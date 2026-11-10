use super::*;

use crate::hir::{HirGenericBounds, HirImplOwner};
use crate::ids::{CrateId, LocalDefId};
use crate::types::GenericParamDecl;

fn id(crate_id: u32, local: u32) -> DefId {
    DefId::new(CrateId(crate_id), LocalDefId(local))
}

fn unary_kind() -> Kind {
    Kind::arrow(Kind::Type, Kind::Type)
}

fn trait_def(trait_id: DefId, target_kind: Kind) -> HirTrait {
    HirTrait {
        id: trait_id,
        name: format!("Trait{}", trait_id.local.0),
        generic_params: Vec::new(),
        target: Some(GenericParamDecl::new(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "F",
            target_kind,
        )),
        predicates: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    }
}

fn structure(struct_id: DefId, arity: usize) -> HirStruct {
    HirStruct {
        id: struct_id,
        name: format!("Type{}", struct_id.local.0),
        generic_params: (0..arity)
            .map(|index| {
                GenericParamDecl::type_param(
                    GenericParamId {
                        owner: struct_id,
                        index: index as u32,
                    },
                    format!("T{index}"),
                )
            })
            .collect(),
        fields: Vec::new(),
    }
}

fn trait_impl(
    impl_id: DefId,
    trait_id: DefId,
    target: HirImplReceiverPattern,
    generics: Vec<GenericParamDecl>,
) -> HirImpl {
    HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("target".to_string()),
        type_name: "target".to_string(),
        type_generics: generics,
        receiver_pattern: target,
        trait_name: Some("Trait".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HirGenericBounds::new(),
        methods: HashMap::new(),
    }
}

fn check(
    traits: &[HirTrait],
    impls: &[HirImpl],
    structs: &[HirStruct],
    aliases: &[HirTypeAlias],
) -> Vec<String> {
    check_with_sized(traits, impls, structs, aliases, None)
}

fn check_with_sized(
    traits: &[HirTrait],
    impls: &[HirImpl],
    structs: &[HirStruct],
    aliases: &[HirTypeAlias],
    sized_trait_id: Option<DefId>,
) -> Vec<String> {
    validate_coherence(
        traits.iter(),
        impls.iter(),
        structs.iter(),
        std::iter::empty(),
        aliases.iter(),
        sized_trait_id,
    )
}

#[test]
fn constructor_impl_target_kind_must_match_trait_target_kind() {
    let trait_id = id(0, 1);
    let imp = trait_impl(
        id(0, 2),
        trait_id,
        HirImplReceiverPattern::Constructor(Type::I64),
        Vec::new(),
    );

    let errors = check(&[trait_def(trait_id, unary_kind())], &[imp], &[], &[]);

    assert!(errors.iter().any(|error| error.contains("has kind Type")));
}

#[test]
fn orphan_rules_accept_one_local_side_and_reject_two_foreign_sides() {
    let foreign_constructor = id(2, 1);
    let local_constructor = id(0, 2);
    let local_trait = id(0, 3);
    let foreign_trait = id(1, 4);
    let target = |constructor| {
        HirImplReceiverPattern::Exact(Type::Struct {
            id: constructor,
            args: Vec::new(),
        })
    };
    let impls = vec![
        trait_impl(
            id(0, 10),
            local_trait,
            target(foreign_constructor),
            Vec::new(),
        ),
        trait_impl(
            id(0, 11),
            foreign_trait,
            target(local_constructor),
            Vec::new(),
        ),
        trait_impl(
            id(0, 12),
            foreign_trait,
            target(foreign_constructor),
            Vec::new(),
        ),
    ];

    let errors = check(
        &[
            trait_def(local_trait, Kind::Type),
            trait_def(foreign_trait, Kind::Type),
        ],
        &impls,
        &[
            structure(foreign_constructor, 0),
            structure(local_constructor, 0),
        ],
        &[],
    );

    assert_eq!(
        errors
            .iter()
            .filter(|error| error.contains("orphan impl"))
            .count(),
        1
    );
    assert!(errors
        .iter()
        .any(|error| error.contains("Trait4") && error.contains("Type1")));
}

#[test]
fn orphan_locality_uses_alias_expanded_outer_constructor() {
    let trait_id = id(1, 1);
    let alias_id = id(0, 2);
    let foreign = id(2, 3);
    let alias = HirTypeAlias {
        id: alias_id,
        name: "LocalAlias".to_string(),
        generic_params: Vec::new(),
        ty: Type::Struct {
            id: foreign,
            args: Vec::new(),
        },
    };
    let imp = trait_impl(
        id(0, 4),
        trait_id,
        HirImplReceiverPattern::Exact(Type::Constructor {
            id: alias_id,
            flavor: NominalTypeKind::Alias,
        }),
        Vec::new(),
    );

    let errors = check(
        &[trait_def(trait_id, Kind::Type)],
        &[imp],
        &[structure(foreign, 0)],
        &[alias],
    );

    assert!(errors.iter().any(|error| error.contains("orphan impl")));
}

#[test]
fn alpha_renamed_constructor_targets_overlap() {
    let trait_id = id(0, 1);
    let option = id(0, 2);
    let lambda = || Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(Type::Struct {
            id: option,
            args: vec![Type::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            }],
        }),
    };
    let impls = vec![
        trait_impl(
            id(0, 3),
            trait_id,
            HirImplReceiverPattern::Constructor(lambda()),
            Vec::new(),
        ),
        trait_impl(
            id(0, 4),
            trait_id,
            HirImplReceiverPattern::Constructor(lambda()),
            Vec::new(),
        ),
    ];

    let errors = check(
        &[trait_def(trait_id, unary_kind())],
        &impls,
        &[structure(option, 1)],
        &[],
    );

    assert!(errors
        .iter()
        .any(|error| error.contains("overlapping impls")));
}

#[test]
fn result_section_overlaps_explicit_lambda_with_first_order_witness() {
    let trait_id = id(0, 1);
    let result = id(0, 2);
    let left_id = id(0, 3);
    let generic = GenericParamDecl::type_param(
        GenericParamId {
            owner: left_id,
            index: 0,
        },
        "E",
    );
    let body = |error: Type| Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(Type::Struct {
            id: result,
            args: vec![
                Type::BoundVar {
                    depth: 0,
                    index: 0,
                    kind: Kind::Type,
                },
                error,
            ],
        }),
    };
    let impls = vec![
        trait_impl(
            left_id,
            trait_id,
            HirImplReceiverPattern::Constructor(body(Type::Generic(generic.id))),
            vec![generic],
        ),
        trait_impl(
            id(0, 4),
            trait_id,
            HirImplReceiverPattern::Constructor(body(Type::I64)),
            Vec::new(),
        ),
    ];

    let errors = check(
        &[trait_def(trait_id, unary_kind())],
        &impls,
        &[structure(result, 2)],
        &[],
    );

    assert!(errors.iter().any(|error| {
        error.contains("Trait1") && error.contains("Type2") && error.contains("E = I64")
    }));
}

#[test]
fn overlap_errors_retain_semantic_ids_targets_and_witnesses() {
    let trait_id = id(0, 1);
    let target_id = id(0, 2);
    let left_id = id(0, 3);
    let right_id = id(0, 4);
    let generic = GenericParamDecl::type_param(
        GenericParamId {
            owner: left_id,
            index: 0,
        },
        "T",
    );
    let target = |ty| {
        HirImplReceiverPattern::Exact(Type::Struct {
            id: target_id,
            args: vec![ty],
        })
    };
    let impls = vec![
        trait_impl(
            left_id,
            trait_id,
            target(Type::Generic(generic.id)),
            vec![generic.clone()],
        ),
        trait_impl(right_id, trait_id, target(Type::I64), Vec::new()),
    ];

    let errors = validate_coherence_with_ids(
        [&trait_def(trait_id, Kind::Type)].into_iter(),
        impls.iter(),
        [&structure(target_id, 1)].into_iter(),
        std::iter::empty(),
        std::iter::empty(),
        None,
    );
    let error = errors
        .iter()
        .find(|error| matches!(&error.kind, CoherenceErrorKind::Overlapping { .. }))
        .expect("overlap error");

    assert_eq!(error.impl_id, left_id);
    let CoherenceErrorKind::Overlapping {
        left_impl_id,
        right_impl_id,
        trait_id: found_trait_id,
        left_target,
        right_target,
        witness,
    } = &error.kind
    else {
        unreachable!();
    };
    assert_eq!(*left_impl_id, left_id);
    assert_eq!(*right_impl_id, right_id);
    assert_eq!(*found_trait_id, trait_id);
    assert!(matches!(
        left_target,
        CanonicalTarget::Exact(Type::Struct { id, args })
            if *id == target_id && args == &vec![Type::Generic(generic.id)]
    ));
    assert!(matches!(
        right_target,
        CanonicalTarget::Exact(Type::Struct { id, args })
            if *id == target_id && args == &vec![Type::I64]
    ));
    assert_eq!(
        witness,
        &vec![CoherenceWitnessBinding {
            parameter: generic.id,
            value: Type::I64,
        }]
    );
}

#[test]
fn coherence_compares_local_and_dependency_impls() {
    let trait_id = id(0, 1);
    let target_id = id(0, 2);
    let target = || {
        HirImplReceiverPattern::Exact(Type::Struct {
            id: target_id,
            args: Vec::new(),
        })
    };
    let impls = vec![
        trait_impl(id(0, 3), trait_id, target(), Vec::new()),
        trait_impl(id(1, 4), trait_id, target(), Vec::new()),
    ];

    let errors = check(
        &[trait_def(trait_id, Kind::Type)],
        &impls,
        &[structure(target_id, 0)],
        &[],
    );

    assert!(errors.iter().any(|error| {
        error.contains("Trait1") && error.contains("Type2") && !error.contains("impl#")
    }));
}

#[test]
fn constructor_variable_blanket_target_is_rejected() {
    let trait_id = id(0, 1);
    let impl_id = id(0, 2);
    let generic = GenericParamDecl::new(
        GenericParamId {
            owner: impl_id,
            index: 0,
        },
        "F",
        unary_kind(),
    );
    let imp = trait_impl(
        impl_id,
        trait_id,
        HirImplReceiverPattern::Constructor(Type::Generic(generic.id)),
        vec![generic],
    );

    let errors = check(&[trait_def(trait_id, unary_kind())], &[imp], &[], &[]);

    assert!(errors
        .iter()
        .any(|error| error.contains("concrete nominal outer constructor")));
}

#[test]
fn coherence_diagnostics_are_independent_of_impl_insertion_order() {
    let trait_id = id(0, 1);
    let target_id = id(0, 2);
    let target = || {
        HirImplReceiverPattern::Exact(Type::Struct {
            id: target_id,
            args: Vec::new(),
        })
    };
    let left = trait_impl(id(0, 3), trait_id, target(), Vec::new());
    let right = trait_impl(id(0, 4), trait_id, target(), Vec::new());
    let traits = [trait_def(trait_id, Kind::Type)];
    let structs = [structure(target_id, 0)];

    assert_eq!(
        check(&traits, &[left.clone(), right.clone()], &structs, &[]),
        check(&traits, &[right, left], &structs, &[]),
    );
}

#[test]
fn sized_blanket_cannot_overlap_an_unsized_slice_target() {
    let index_trait = id(0, 1);
    let sized_trait = id(0, 2);
    let blanket_id = id(0, 3);
    let slice_id = id(0, 4);
    let blanket_generic = GenericParamDecl::type_param(
        GenericParamId {
            owner: blanket_id,
            index: 0,
        },
        "T",
    );
    let slice_generic = GenericParamDecl::type_param(
        GenericParamId {
            owner: slice_id,
            index: 0,
        },
        "U",
    );
    let mut blanket = trait_impl(
        blanket_id,
        index_trait,
        HirImplReceiverPattern::Exact(Type::Pointer(Box::new(Type::Generic(blanket_generic.id)))),
        vec![blanket_generic.clone()],
    );
    blanket.bounds.insert(
        blanket_generic.id,
        vec![crate::types::TraitBound {
            trait_id: sized_trait,
            type_args: Vec::new(),
        }],
    );
    let slice = trait_impl(
        slice_id,
        index_trait,
        HirImplReceiverPattern::Exact(Type::Pointer(Box::new(Type::Slice(Box::new(
            Type::Generic(slice_generic.id),
        ))))),
        vec![slice_generic],
    );
    let traits = [
        trait_def(index_trait, Kind::Type),
        trait_def(sized_trait, Kind::Type),
    ];

    assert!(check_with_sized(
        &traits,
        &[blanket.clone(), slice.clone()],
        &[],
        &[],
        Some(sized_trait),
    )
    .iter()
    .all(|error| !error.contains("overlapping impls")));
    assert!(check(&traits, &[blanket, slice], &[], &[])
        .iter()
        .any(|error| error.contains("overlapping impls")));
}

fn array_impl(
    impl_id: DefId,
    trait_id: DefId,
    len: crate::types::ArrayLen,
    generics: Vec<GenericParamDecl>,
) -> HirImpl {
    trait_impl(
        impl_id,
        trait_id,
        HirImplReceiverPattern::Exact(Type::Array(Box::new(Type::I64), len)),
        generics,
    )
}

fn const_param(owner: DefId) -> GenericParamDecl {
    GenericParamDecl::new(GenericParamId { owner, index: 0 }, "N", Kind::Usize)
}

#[test]
fn const_param_array_impl_overlaps_concrete_length_impl() {
    let trait_id = id(0, 1);
    let generic_impl = id(0, 2);
    let concrete_impl = id(0, 3);
    let impls = vec![
        array_impl(
            generic_impl,
            trait_id,
            crate::types::ArrayLen::Param(GenericParamId {
                owner: generic_impl,
                index: 0,
            }),
            vec![const_param(generic_impl)],
        ),
        array_impl(
            concrete_impl,
            trait_id,
            crate::types::ArrayLen::Value(3),
            Vec::new(),
        ),
    ];

    let errors = check(&[trait_def(trait_id, Kind::Type)], &impls, &[], &[]);

    assert!(errors
        .iter()
        .any(|error| error.contains("overlapping impls")));
}

#[test]
fn distinct_concrete_array_lengths_do_not_overlap() {
    let trait_id = id(0, 1);
    let impls = vec![
        array_impl(id(0, 2), trait_id, crate::types::ArrayLen::Value(3), Vec::new()),
        array_impl(id(0, 3), trait_id, crate::types::ArrayLen::Value(4), Vec::new()),
    ];

    let errors = check(&[trait_def(trait_id, Kind::Type)], &impls, &[], &[]);

    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn const_param_cannot_bind_to_type_kinded_witness() {
    // `N: usize` unifies only with const-kinded types, never with `I64`.
    let trait_id = id(0, 1);
    let const_impl = id(0, 2);
    let type_impl = id(0, 3);
    let impls = vec![
        array_impl(
            const_impl,
            trait_id,
            crate::types::ArrayLen::Param(GenericParamId {
                owner: const_impl,
                index: 0,
            }),
            vec![const_param(const_impl)],
        ),
        trait_impl(
            type_impl,
            trait_id,
            HirImplReceiverPattern::Exact(Type::I64),
            Vec::new(),
        ),
    ];

    let errors = check(&[trait_def(trait_id, Kind::Type)], &impls, &[], &[]);

    assert!(errors.is_empty(), "{errors:?}");
}
