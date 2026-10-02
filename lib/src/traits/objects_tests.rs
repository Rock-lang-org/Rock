use crate::hir::HirTrait;
use crate::ids::{CrateId, LocalDefId};
use crate::types::{AssociatedTypeKey, ObjectBinding};

use super::*;

fn id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}
fn param(owner: u32, index: u32) -> GenericParamId {
    GenericParamId {
        owner: id(owner),
        index,
    }
}
fn bound(owner: u32, type_args: Vec<Type>) -> TraitBound {
    TraitBound {
        trait_id: id(owner),
        type_args,
    }
}
fn declaration(owner: u32, arity: u32, parents: Vec<TraitBound>, associated: bool) -> HirTrait {
    HirTrait {
        id: id(owner),
        name: "Trait".to_string(),
        generic_params: (0..arity)
            .map(|index| GenericParamDecl::type_param(param(owner, index), "A"))
            .collect(),
        target: None,
        associated_types: if associated {
            vec![HirAssociatedTypeDecl {
                id: AssocTypeId(0),
                name: "Item".to_string(),
                kind: Kind::Type,
            }]
        } else {
            vec![]
        },
        predicates: parents
            .into_iter()
            .map(|parent| Predicate::Trait {
                subject: Type::Generic(param(owner, arity)),
                trait_id: parent.trait_id,
                args: parent.type_args,
            })
            .collect(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    }
}
fn binding(owner: u32, args: Vec<Type>, ty: Type) -> ObjectBinding {
    ObjectBinding {
        key: ObjectAssociatedType {
            trait_ref: bound(owner, args),
            member: AssocTypeId(0),
        },
        ty,
    }
}

#[test]
fn admission_binds_hidden_self_without_capturing_outer_trait_parameters() {
    let traits = HashMap::from([
        (
            id(1),
            declaration(
                1,
                1,
                vec![bound(
                    2,
                    vec![Type::Generic(param(1, 0)), Type::Generic(param(1, 1))],
                )],
                false,
            ),
        ),
        (id(2), declaration(2, 2, vec![], false)),
    ]);
    let object = ObjectType::new(bound(1, vec![Type::Generic(param(1, 1))]));
    let admitted = admit_object(&object, &traits, &TypeNormalizationEnv::new()).unwrap();
    assert!(admitted.guarantees.contains(&bound(
        2,
        vec![Type::Generic(param(1, 1)), Type::ObjectSelf { depth: 0 }]
    )));
    assert!(!crate::type_services::substitution::has_free_object_self(
        &Type::Object(Box::new(admitted))
    ));
}

#[test]
fn admission_requires_every_inherited_associated_binding_and_rejects_unrelated_owners() {
    let traits = HashMap::from([
        (id(1), declaration(1, 0, vec![bound(2, vec![])], false)),
        (id(2), declaration(2, 0, vec![], true)),
        (id(3), declaration(3, 0, vec![], true)),
    ]);
    let mut object = ObjectType::new(bound(1, vec![]));
    assert!(
        matches!(admit_object(&object, &traits, &TypeNormalizationEnv::new()), Err(ObjectAdmissionError::MissingBinding(key)) if key.trait_ref.trait_id == id(2))
    );
    object.bindings.insert(binding(2, vec![], Type::I64));
    let admitted = admit_object(&object, &traits, &TypeNormalizationEnv::new()).unwrap();
    assert!(admitted.guarantees.contains(&bound(2, vec![])));
    assert_eq!(
        admit_object(&admitted, &traits, &TypeNormalizationEnv::new()).unwrap(),
        admitted
    );
    object.bindings.insert(binding(3, vec![], Type::I64));
    assert!(matches!(
        admit_object(&object, &traits, &TypeNormalizationEnv::new()),
        Err(ObjectAdmissionError::UnrelatedBinding(_))
    ));
}

#[test]
fn admission_substitutes_associated_outputs_into_supertrait_arguments() {
    let projection = Type::Projection {
        ty: Box::new(Type::Generic(param(1, 0))),
        trait_id: id(1),
        assoc_type: AssociatedTypeKey {
            owner: id(1),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![],
    };
    let traits = HashMap::from([
        (
            id(1),
            declaration(1, 0, vec![bound(2, vec![projection])], true),
        ),
        (id(2), declaration(2, 1, vec![], false)),
    ]);
    let mut object = ObjectType::new(bound(1, vec![]));
    object.bindings.insert(binding(1, vec![], Type::I64));
    let admitted = admit_object(&object, &traits, &TypeNormalizationEnv::new()).unwrap();
    assert!(admitted.guarantees.contains(&bound(2, vec![Type::I64])));
}

#[test]
fn admission_rejects_projection_and_growing_supertrait_cycles() {
    let traits = HashMap::from([(id(1), declaration(1, 0, vec![], true))]);
    let mut object = ObjectType::new(bound(1, vec![]));
    object.bindings.insert(binding(
        1,
        vec![],
        Type::Projection {
            ty: Box::new(Type::ObjectSelf { depth: 0 }),
            trait_id: id(1),
            assoc_type: AssociatedTypeKey {
                owner: id(1),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![],
        },
    ));
    assert!(matches!(
        admit_object(&object, &traits, &TypeNormalizationEnv::new()),
        Err(ObjectAdmissionError::ProjectionCycle(_))
    ));

    let traits = HashMap::from([(
        id(1),
        declaration(
            1,
            1,
            vec![bound(
                1,
                vec![Type::Array(Box::new(Type::Generic(param(1, 0))), 1)],
            )],
            false,
        ),
    )]);
    let object = ObjectType::new(bound(1, vec![Type::I64]));
    assert!(matches!(
        admit_object(&object, &traits, &TypeNormalizationEnv::new()),
        Err(ObjectAdmissionError::SupertraitCycle(_))
    ));
}

#[test]
fn admission_checks_nested_object_self_scope_independently_of_lambdas() {
    let traits = HashMap::from([(id(1), declaration(1, 1, vec![], false))]);
    let nested = ObjectType::new(bound(1, vec![Type::ObjectSelf { depth: 1 }]));
    assert!(matches!(
        admit_object(&nested, &traits, &TypeNormalizationEnv::new()),
        Err(ObjectAdmissionError::UnboundObjectSelf)
    ));
    let outer = Type::Object(Box::new(ObjectType::new(bound(
        1,
        vec![Type::Object(Box::new(nested))],
    ))));
    assert!(admit_type_objects(&outer, &traits, &TypeNormalizationEnv::new()).is_ok());
    let escaped = Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(Type::ObjectSelf { depth: 0 }),
    };
    assert!(matches!(
        admit_type_objects(&escaped, &traits, &TypeNormalizationEnv::new()),
        Err(ObjectAdmissionError::UnboundObjectSelf)
    ));
}
