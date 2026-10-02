use std::collections::{BTreeMap, HashMap};

use super::object_abi::*;
use super::*;
use crate::hir::{AcceptedHir, HirFunctionSig, HirTraitFor};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mir::{MirObjectResult, MirPassMode};
use crate::type_context::TypeContext;
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::{GenericParamDecl, GenericParamId, ObjectType, ReceiverMode, TraitBound, Type};

fn id(crate_id: u32, local: u32) -> DefId {
    DefId::new(CrateId(crate_id), LocalDefId(local))
}

fn trait_decl(owner: DefId, generic: bool) -> HirTraitFor<AcceptedHir> {
    let self_param = GenericParamId {
        owner,
        index: u32::from(generic),
    };
    HirTraitFor {
        id: owner,
        name: format!("Trait{}", owner.local.0),
        generic_params: if generic {
            vec![GenericParamDecl::type_param(
                GenericParamId { owner, index: 0 },
                "A",
            )]
        } else {
            vec![]
        },
        target: None,
        predicates: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "view".into(),
            HirFunctionSig {
                id: id(owner.crate_id.0, owner.local.0 + 10),
                name: "view".into(),
                generic_params: vec![],
                params: vec![Type::Generic(self_param)],
                ret: Type::Reference {
                    inner: Box::new(Type::Generic(self_param)),
                    mutable: false,
                },
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    }
}

fn abi_for(
    objects: impl IntoIterator<Item = Type>,
    traits: &HashMap<DefId, HirTraitFor<AcceptedHir>>,
) -> ProductObjectAbi {
    let env = TypeNormalizationEnv::new();
    let mut all = std::collections::BTreeSet::new();
    for object in objects {
        all.extend(
            crate::mono::object_schema::admitted_object_views(&object, traits, &env).unwrap(),
        );
        all.insert(object);
    }
    let mut abi = ProductObjectAbi {
        target: Some(host_object_target()),
        ..Default::default()
    };
    for (id, declaration) in traits {
        abi.trait_members
            .insert(*id, declaration_member_order(&declaration.signatures));
    }
    for object in all {
        let Type::Object(signature) = &object else {
            unreachable!()
        };
        let mut slots = Vec::new();
        for bound in std::iter::once(&signature.principal).chain(signature.guarantees.iter()) {
            let declaration = &traits[&bound.trait_id];
            for member in &abi.trait_members[&bound.trait_id] {
                let signature = declaration
                    .signatures
                    .values()
                    .find(|s| s.id == *member)
                    .unwrap();
                let expected = crate::mono::object_schema::object_method_abi(
                    &object,
                    bound,
                    declaration,
                    signature,
                    &env,
                )
                .unwrap();
                slots.push(ProductObjectSlot {
                    trait_id: bound.trait_id,
                    member_id: *member,
                    trait_args: bound.type_args.clone(),
                    receiver: expected.receiver,
                    params: expected
                        .params
                        .into_iter()
                        .enumerate()
                        .map(|(index, ty)| ProductObjectParam {
                            ty,
                            pass_mode: if index == 0 {
                                MirPassMode::FatDirect
                            } else {
                                MirPassMode::Direct
                            },
                        })
                        .collect(),
                    ret: expected.ret.clone(),
                    abi_ret: expected.ret,
                    result: expected.result,
                });
            }
        }
        let views = crate::mono::object_schema::admitted_object_views(&object, traits, &env)
            .unwrap()
            .into_iter()
            .rev()
            .collect();
        abi.schemas.push(ProductObjectSchema {
            erased_slots: vec![],
            object,
            slots,
            views,
        });
    }
    abi
}

fn products_with_scoped_schema() -> CompilerProducts {
    let declaration = trait_decl(id(0, 1), true);
    let traits = HashMap::from([(declaration.id, declaration.clone())]);
    let object = Type::Object(Box::new(ObjectType::new(TraitBound {
        trait_id: declaration.id,
        type_args: vec![Type::ObjectSelf { depth: 0 }],
    })));
    let mut interface = ProductInterface::default();
    interface.insert_trait(
        declaration.id.into(),
        ProductTraitInterface::from(&declaration),
    );
    interface.object_abi = abi_for([object], &traits);
    CompilerProducts {
        crate_identity: ProductCrateIdentity::local("schema_fixture".into()),
        identity_table: ProductIdentityTable {
            local_crate: Some(ProductCrateId(0)),
            ..Default::default()
        },
        interface,
        bodies: ProductBodies::default(),
        link: ProductLinkData::default(),
        dependencies: vec![],
        source_fingerprint: Default::default(),
        infix_precedence: BTreeMap::new(),
        proc_macros: vec![],
    }
}

#[test]
fn producer_schema_roundtrip_preserves_scoped_self_and_result_adaptation() {
    let products = products_with_scoped_schema();
    let bytes = products.to_artifact_bytes().unwrap();
    let decoded = CompilerProducts::from_artifact_bytes(&bytes).unwrap();
    assert_eq!(decoded.interface.object_abi, products.interface.object_abi);
    let slot = &decoded.interface.object_abi.schemas[0].slots[0];
    assert_eq!(slot.trait_args, [Type::ObjectSelf { depth: 0 }]);
    assert_eq!(
        slot.result,
        MirObjectResult::BorrowedSelf { mutable: false }
    );
    assert_eq!(PRODUCT_ARTIFACT_FORMAT_VERSION, 52);
}

#[test]
fn attachment_reads_the_final_mir_type_context_not_portable_or_frontend_indices() {
    let mut products = products_with_scoped_schema();
    let expected = products.interface.object_abi.clone();
    products.interface.object_abi.schemas.clear();
    products.interface.object_abi.target = None;
    let mut context = TypeContext::new();
    for ty in [Type::Bool, Type::I64, Type::U8, Type::Unit] {
        context.intern_type(&ty);
    }
    let mut contract = crate::mir::MirBackendContract::default();
    for portable in &expected.schemas {
        let schema = portable
            .intern(&mut context, &TypeNormalizationEnv::new())
            .unwrap();
        contract.object_schemas.insert(schema.object, schema);
    }
    let mir = crate::mir::MirProgram {
        type_context: context,
        functions: BTreeMap::new(),
        backend_contract: contract,
    };
    let remap = [id(0, 1), id(0, 11)]
        .into_iter()
        .map(|id| {
            (
                ProductDefId::from(id),
                std::collections::BTreeSet::from([ProductDefId::from(id)]),
            )
        })
        .collect();
    products.attach_object_schemas(&mir, &remap).unwrap();
    assert_eq!(products.interface.object_abi, expected);
}

#[test]
fn schema_component_binder_does_not_legalize_an_ordinary_unbound_type_root() {
    let products = products_with_scoped_schema();
    let mut artifact =
        type_table::products_to_artifact(&products, PRODUCT_ARTIFACT_FORMAT_VERSION).unwrap();
    let slot = &mut artifact.products.interface.object_abi.schemas[0].slots[0];
    slot.ret = slot.trait_args[0];
    slot.abi_ret = slot.ret;
    let bytes =
        encode_product_artifact(&ProductArtifactHeader::from_products(&products), &artifact)
            .unwrap();
    let error = CompilerProducts::from_artifact_bytes(&bytes).unwrap_err();
    assert!(
        error.contains("out-of-scope product object Self"),
        "{error}"
    );
}

#[test]
fn producer_schema_cannot_bind_a_body_local_witness() {
    let owner = id(0, 1);
    let object = Type::Object(Box::new(ObjectType::new(TraitBound {
        trait_id: owner,
        type_args: vec![Type::I64],
    })));
    let original = abi_for([object], &HashMap::from([(owner, trait_decl(owner, true))]));
    original.validate_shape().unwrap();
    let witness = Type::Witness(crate::types::WitnessId {
        owner: id(0, 90),
        local: crate::ids::HirLocalId(3),
    });
    for mutation in 0..4 {
        let mut malformed = original.clone();
        let schema = &mut malformed.schemas[0];
        match mutation {
            0 => {
                let Type::Object(object) = &mut schema.object else {
                    unreachable!()
                };
                object.principal.type_args[0] = witness.clone();
            }
            1 => schema.slots[0].params[0].ty = witness.clone(),
            2 => schema.slots[0].abi_ret = witness.clone(),
            3 => schema.slots[0].trait_args[0] = witness.clone(),
            _ => unreachable!(),
        }
        let error = malformed.validate_shape().unwrap_err();
        assert!(error.contains("unbound"), "mutation {mutation}: {error}");
    }
}

#[test]
fn malformed_producer_schema_signatures_and_view_closures_are_rejected() {
    let declarations = HashMap::from([
        (id(0, 1), trait_decl(id(0, 1), false)),
        (id(0, 2), trait_decl(id(0, 2), false)),
    ]);
    let mut object = ObjectType::new(TraitBound {
        trait_id: id(0, 1),
        type_args: vec![],
    });
    object.guarantees.insert(TraitBound {
        trait_id: id(0, 2),
        type_args: vec![],
    });
    let original = abi_for([Type::Object(Box::new(object))], &declarations);
    original
        .validate_declarations(
            &declarations,
            &original.trait_members,
            &TypeNormalizationEnv::new(),
        )
        .unwrap();
    for mutation in 0..5 {
        let mut abi = original.clone();
        match mutation {
            0 => abi.schemas[0].slots[0].member_id = id(0, 999),
            1 => abi.schemas[0].slots[0].ret = Type::I64,
            2 => abi.schemas[0].slots[0].result = MirObjectResult::Direct,
            3 => {
                let schema = abi
                    .schemas
                    .iter_mut()
                    .find(|s| !s.views.is_empty())
                    .unwrap();
                schema.views.pop();
            }
            4 => {
                let duplicate = abi.schemas[0].slots[0].clone();
                abi.schemas[0].slots.push(duplicate);
            }
            _ => unreachable!(),
        }
        assert!(
            abi.validate_declarations(
                &declarations,
                &abi.trait_members,
                &TypeNormalizationEnv::new()
            )
            .is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn importer_remapping_preserves_producer_slot_and_view_ordinals() {
    let declarations = HashMap::from([
        (id(1, 1), trait_decl(id(1, 1), false)),
        (id(2, 1), trait_decl(id(2, 1), false)),
    ]);
    let mut object = ObjectType::new(TraitBound {
        trait_id: id(1, 1),
        type_args: vec![],
    });
    object.guarantees.insert(TraitBound {
        trait_id: id(2, 1),
        type_args: vec![],
    });
    let mut abi = abi_for([Type::Object(Box::new(object))], &declarations);
    let original = abi.clone();
    let mut remap = |definition: DefId| {
        Ok(id(
            if definition.crate_id.0 == 1 { 90 } else { 3 },
            definition.local.0,
        ))
    };
    // Deliberately reverse the numerical ordering of the defining crates.
    abi.try_remap_def_ids(&mut remap).unwrap();
    for (source, imported) in original.schemas.iter().zip(&abi.schemas) {
        assert_eq!(
            source
                .slots
                .iter()
                .map(|s| s.member_id.local)
                .collect::<Vec<_>>(),
            imported
                .slots
                .iter()
                .map(|s| s.member_id.local)
                .collect::<Vec<_>>()
        );
        for (source_slot, imported_slot) in source.slots.iter().zip(&imported.slots) {
            assert_eq!(
                imported_slot.trait_id.crate_id.0,
                if source_slot.trait_id.crate_id.0 == 1 {
                    90
                } else {
                    3
                }
            );
        }
        for (source_view, imported_view) in source.views.iter().zip(&imported.views) {
            let mut expected = source_view.clone();
            expected.try_remap_def_ids(&mut remap).unwrap();
            assert_eq!(&expected, imported_view);
        }
        let mut context = TypeContext::new();
        let runtime = imported
            .intern(&mut context, &TypeNormalizationEnv::new())
            .unwrap();
        assert_eq!(
            ProductObjectSchema::from_mir(&runtime, &context).unwrap(),
            *imported
        );
    }
}

#[test]
fn imported_member_ids_cannot_replace_recorded_producer_slot_order() {
    let owner = id(1, 1);
    let mut declaration = trait_decl(owner, false);
    let mut read = declaration.signatures["view"].clone();
    read.id = id(1, 21);
    read.name = "read".into();
    read.ret = Type::I64;
    declaration.signatures.insert("read".into(), read);
    let object = Type::Object(Box::new(ObjectType::new(TraitBound {
        trait_id: owner,
        type_args: vec![],
    })));
    let mut abi = abi_for([object], &HashMap::from([(owner, declaration.clone())]));
    let mut remap = |definition| {
        Ok::<_, String>(if definition == id(1, 11) {
            id(1, 90)
        } else if definition == id(1, 21) {
            id(1, 2)
        } else {
            definition
        })
    };
    abi.try_remap_def_ids(&mut remap).unwrap();
    // Rebuild the member map in the opposite insertion order as well as
    // reversing member-ID order; neither is the imported ABI authority.
    declaration.signatures = ["read", "view"]
        .into_iter()
        .map(|name| {
            let mut signature = declaration.signatures[name].clone();
            signature.id = remap(signature.id).unwrap();
            (name.to_string(), signature)
        })
        .collect();
    let recorded = &abi.trait_members[&owner];
    assert_eq!(recorded, &[id(1, 90), id(1, 2)]);
    assert_ne!(recorded, &declaration_member_order(&declaration.signatures));
    let declarations = HashMap::from([(owner, declaration)]);
    abi.validate_declarations(
        &declarations,
        &abi.trait_members,
        &TypeNormalizationEnv::new(),
    )
    .unwrap();

    abi.schemas[0].slots.sort_by_key(|slot| slot.member_id);
    let error = abi
        .validate_declarations(
            &declarations,
            &abi.trait_members,
            &TypeNormalizationEnv::new(),
        )
        .unwrap_err();
    assert!(
        error.contains("member order/completeness mismatch"),
        "{error}"
    );
}
