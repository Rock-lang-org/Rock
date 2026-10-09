use super::Monomorphizer;
use crate::hir::{AcceptedHir, HirAssociatedTypeDecl, HirTraitFor};
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::type_services::kind::Kind;
use crate::types::{ObjectAssociatedType, ObjectBinding, ObjectType, TraitBound, Type};
use std::collections::HashMap;

fn bound(index: u32) -> TraitBound {
    TraitBound {
        trait_id: DefId::new(CrateId(0), LocalDefId(index)),
        type_args: vec![],
    }
}

#[test]
fn factored_trait_evidence_admits_unmaterialized_subsets_with_exact_bindings() {
    let mut mono = Monomorphizer::new();
    let mut object = ObjectType::new(bound(1));
    object.guarantees.extend([bound(2), bound(3)]);
    for index in 1..=3 {
        let trait_id = bound(index).trait_id;
        mono.object_traits.insert(
            trait_id,
            HirTraitFor::<AcceptedHir> {
                id: trait_id,
                name: format!("Trait{index}"),
                generic_params: vec![],
                target: None,
                predicates: vec![],
                associated_types: vec![HirAssociatedTypeDecl {
                    id: AssocTypeId(0),
                    name: "Associated".into(),
                    kind: Kind::Type,
                }],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        object.bindings.insert(ObjectBinding {
            key: ObjectAssociatedType {
                trait_ref: bound(index),
                member: AssocTypeId(0),
            },
            ty: if index == 2 { Type::Bool } else { Type::I64 },
        });
    }
    let source = mono
        .ensure_object_schema(&Type::Object(Box::new(object.clone())))
        .unwrap();
    let mut target = ObjectType::new(bound(2));
    target.guarantees.insert(bound(3));
    target.bindings.extend(
        object
            .bindings
            .iter()
            .filter(|binding| binding.key.trait_ref != bound(1))
            .cloned(),
    );
    let target_ty = Type::Object(Box::new(target));
    assert!(
        mono.type_context.id_for_type(&target_ty).is_none(),
        "a producer must not enumerate consumer-selected guarantee subsets"
    );
    assert_eq!(
        mono.object_schemas.len(),
        4,
        "one source and three independent trait schemas"
    );
    assert_eq!(mono.object_schemas[&source].views.len(), 3);
    let target_id = mono.ensure_object_schema(&target_ty).unwrap();
    assert!(crate::mir::object::object_view_is_admitted(
        &mono.type_context,
        source,
        target_id
    ));
    assert_eq!(mono.type_for(target_id), target_ty);
    let mut changed_binding = mono.type_for(target_id);
    let Type::Object(object) = &mut changed_binding else {
        unreachable!()
    };
    let mut binding = object
        .bindings
        .iter()
        .find(|binding| binding.key.trait_ref == bound(2))
        .unwrap()
        .clone();
    object.bindings.remove(&binding);
    binding.ty = Type::U8;
    object.bindings.insert(binding);
    let changed_id = mono.intern_type(&changed_binding);
    assert!(
        !crate::mir::object::object_view_is_admitted(&mono.type_context, source, changed_id),
        "factoring cannot erase the associated binding proof"
    );
}

#[test]
fn independent_trait_evidence_scales_linearly_without_upcast_consumers() {
    for count in [1, 3, 5, 7, 32] {
        let mut mono = Monomorphizer::new();
        let mut object = ObjectType::new(bound(1));
        for index in 1..=count {
            mono.object_traits.insert(
                bound(index).trait_id,
                HirTraitFor::<AcceptedHir> {
                    id: bound(index).trait_id,
                    name: format!("Trait{index}"),
                    generic_params: vec![],
                    target: None,
                    predicates: vec![],
                    associated_types: vec![],
                    methods: HashMap::new(),
                    signatures: HashMap::new(),
                },
            );
            if index != 1 {
                object.guarantees.insert(bound(index));
            }
        }
        let source = mono
            .ensure_object_schema(&Type::Object(Box::new(object)))
            .unwrap();
        assert_eq!(
            mono.object_schemas.len(),
            if count == 1 { 1 } else { count as usize + 1 }
        );
        let edges = mono
            .object_schemas
            .values()
            .map(|schema| schema.views.len())
            .sum::<usize>();
        assert_eq!(edges, if count == 1 { 0 } else { count as usize });
        assert!(mono.object_schemas[&source].views.len() <= count as usize);
    }
}

#[test]
fn schema_self_returns_bind_to_the_exact_borrowed_receiver_view() {
    for mutable in [false, true] {
        let mut mono = Monomorphizer::new();
        let trait_id = bound(1).trait_id;
        let hidden = Type::Generic(crate::types::GenericParamId {
            owner: trait_id,
            index: 0,
        });
        mono.object_traits.insert(
            trait_id,
            HirTraitFor::<AcceptedHir> {
                id: trait_id,
                name: "Identity".into(),
                generic_params: vec![],
                target: None,
                predicates: vec![],
                associated_types: vec![],
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "other".into(),
                    crate::hir::HirFunctionSig {
                        id: bound(2).trait_id,
                        name: "other".into(),
                        generic_params: vec![],
                        params: vec![hidden.clone()],
                        ret: Type::Reference {
                            inner: Box::new(hidden),
                            mutable,
                        },
                        generic_bounds: HashMap::new().into(),
                        self_receiver: Some(if mutable {
                            crate::types::ReceiverMode::Mut
                        } else {
                            crate::types::ReceiverMode::Shared
                        }),
                        is_unsafe: false,
                    },
                )]),
            },
        );
        let object = Type::Object(Box::new(ObjectType::new(bound(1))));
        let id = mono.ensure_object_schema(&object).unwrap();
        let slot = &mono.object_schemas[&id].slots[0];
        assert_eq!(
            slot.result,
            crate::mir::MirObjectResult::BorrowedSelf { mutable }
        );
        assert_eq!(
            mono.type_for(slot.signature.ret.semantic_ty),
            Type::Reference {
                inner: Box::new(object),
                mutable
            }
        );
    }
}

#[test]
fn concrete_evidence_opens_hidden_self_components_before_comparison() {
    let mono = Monomorphizer::new();
    let formal = Type::Reference {
        inner: Box::new(Type::ObjectSelf { depth: 0 }),
        mutable: false,
    };
    assert_eq!(
        mono.open_object_component(&formal, &Type::I64).unwrap(),
        Type::Reference {
            inner: Box::new(Type::I64),
            mutable: false
        }
    );
}

#[test]
fn admitted_imported_schema_is_used_without_rebuilding_producer_ordinals() {
    use crate::mir::{MirObjectResult, MirPassMode};
    use crate::products::object_abi::{
        host_object_target, ProductObjectAbi, ProductObjectParam, ProductObjectSchema,
        ProductObjectSlot,
    };
    let trait_id = DefId::new(CrateId(9), LocalDefId(1));
    let member = |local| DefId::new(CrateId(9), LocalDefId(local));
    let hidden = Type::Generic(crate::types::GenericParamId {
        owner: trait_id,
        index: 0,
    });
    let mut declaration = HirTraitFor::<AcceptedHir> {
        id: trait_id,
        name: "Producer".into(),
        generic_params: vec![],
        target: None,
        predicates: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::new(),
    };
    for (id, name, ret) in [
        (member(2), "first", Type::I64),
        (member(3), "second", Type::Bool),
    ] {
        declaration.signatures.insert(
            name.into(),
            crate::hir::HirFunctionSig {
                id,
                name: name.into(),
                generic_params: vec![],
                params: vec![hidden.clone()],
                ret,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(crate::types::ReceiverMode::Shared),
                is_unsafe: false,
            },
        );
    }
    let object = Type::Object(Box::new(ObjectType::new(TraitBound {
        trait_id,
        type_args: vec![],
    })));
    let mut abi = ProductObjectAbi {
        target: Some(host_object_target()),
        ..Default::default()
    };
    abi.trait_members
        .insert(trait_id, vec![member(3), member(2)]);
    abi.schemas.push(ProductObjectSchema {
        erased_slots: vec![],
        object: object.clone(),
        views: vec![],
        slots: [member(3), member(2)]
            .into_iter()
            .map(|id| {
                let sig = declaration
                    .signatures
                    .values()
                    .find(|s| s.id == id)
                    .unwrap();
                ProductObjectSlot {
                    trait_id,
                    member_id: id,
                    trait_args: vec![],
                    receiver: crate::types::ReceiverMode::Shared,
                    params: vec![ProductObjectParam {
                        ty: Type::Reference {
                            inner: Box::new(object.clone()),
                            mutable: false,
                        },
                        pass_mode: MirPassMode::FatDirect,
                    }],
                    ret: sig.ret.clone(),
                    abi_ret: sig.ret.clone(),
                    result: MirObjectResult::Direct,
                }
            })
            .collect(),
    });
    abi.validate_declarations(
        &HashMap::from([(trait_id, declaration.clone())]),
        &abi.trait_members,
        &Default::default(),
    )
    .unwrap();
    let mut interface = crate::crate_artifact::ArtifactCrateInterface::default();
    interface.traits.insert(
        trait_id,
        crate::products::ProductTraitInterface::from(&declaration),
    );
    interface.object_abi = abi;
    let mut crates = crate::crate_system::CrateContext::new();
    crates
        .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
            CrateId(9),
            "producer".into(),
            crate::crate_system::ExternCrateMetadata::new(
                interface,
                Default::default(),
                Default::default(),
            ),
            crate::crate_system::ExternCrateBodies::default(),
            crate::crate_system::ExternCrateLink::metadata_only(Default::default()),
        ))
        .unwrap();
    let mut mono = Monomorphizer::new();
    mono.import_object_abis(&crates).unwrap();
    // Deliberately do not install importer trait maps: this is an admitted schema.
    let object = mono.ensure_object_schema(&object).unwrap();
    assert_eq!(
        mono.object_schemas[&object]
            .slots
            .iter()
            .map(|slot| slot.member_id)
            .collect::<Vec<_>>(),
        [member(3), member(2)]
    );
}
