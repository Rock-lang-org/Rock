use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mir::*;
use crate::type_context::TypeContext;
use crate::type_services::kind::Kind;
use crate::types::{GenericParamId, ReceiverMode, Type};

pub(crate) fn fixture() -> (MirProgram, MirErasedKey, DefId) {
    let id = |n| DefId::new(CrateId(0), LocalDefId(n));
    let mut tc = TypeContext::new();
    let result = MirErasedType::Concrete(tc.intern_type(&Type::I64));
    let parameter = MirErasedType::Parameter(0);
    let reference = MirErasedType::Reference { inner: Box::new(parameter.clone()), mutable: false };
    let schema = |trait_id, member_id| MirErasedDictionarySchema {
        subject: parameter.clone(), trait_id, trait_args: vec![], members: vec![MirErasedDictionaryMember {
            member_id, binder: Default::default(), receiver: Some(ReceiverMode::Shared), params: vec![parameter.clone()], ret: result.clone(),
            borrowing: erased_borrow_contract(&[parameter.clone()], &result, Some(ReceiverMode::Shared), &tc).unwrap(),
        }],
    };
    let read = schema(id(10), id(11)); let relay = schema(id(20), id(21));
    let implementation = id(30); let method = id(31);
    let key = |definition| MirErasedKey { definition, static_args: vec![] };
    let producer = key(id(40)); let helper = key(id(41)); let entry = key(method);
    let params = |owner| vec![MirErasedParameter { source: GenericParamId { owner, index: 0 }, kind: Kind::Type }];
    let signature = |owner, inputs: Vec<MirErasedType>, dictionaries: Vec<MirErasedDictionarySchema>, layouts| MirErasedSignature {
        parameters: params(owner), require_borrow_free: erased_callback_storage_requirements(&inputs, &dictionaries),
        params: inputs, ret: result.clone(), dictionaries, layouts,
    };
    let template = MirDictionaryFactoryTemplate { implementation, parameters: params(implementation), result: relay.clone(), bounds: vec![read.clone()],
        members: vec![MirDictionaryFactoryMember { member_id: id(21), method_id: method, entry: entry.clone() }] };
    let body = |receiver: MirErasedPlace, dictionary| MirErasedBody {
        locals: vec![if receiver.projection.is_empty() { parameter.clone() } else { reference.clone() }, result.clone()],
        scopes: vec![], mutable_locals: BTreeSet::new(), blocks: vec![MirErasedBasicBlock {
            statements: vec![MirErasedAssignment { destination: MirErasedLocal(1), value: MirErasedRvalue::DictionaryCall {
                type_args: vec![], dictionaries: vec![],
                dictionary, member: 0, receiver: Some(receiver), args: vec![],
            } }], terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(1)))),
        }],
    };
    let mut contract = MirBackendContract::default();
    contract.descriptor_origins.insert(CrateId(0), crate::products::ProductCrateIdentity::local("dictionary_fixture".into()));
    let MirErasedType::Concrete(result_id) = &result else { unreachable!() };
    let result_id = *result_id;
    let drop = payload_drop_plan(&contract, &mut tc, result_id).unwrap();
    contract.descriptor_identities.insert(result_id, descriptor_identity::descriptor_identity(&contract, &tc, &drop).unwrap());
    contract.erased.descriptors.insert(result_id, MirTypeDescriptor { ty: result_id, drop, fields: vec![] });
    contract.dictionary_impl_members.insert(implementation, MirDictionaryImplAuthority {
        trait_id: id(20), members: BTreeMap::from([(id(21), method)]), factory_compatible: true, template: Some(template.clone()),
    });
    contract.dictionary_impl_members.insert(id(50), MirDictionaryImplAuthority { trait_id: id(10), members: BTreeMap::from([(id(11), id(51))]), factory_compatible: true, template: None });
    contract.erased.dictionary_templates.insert(implementation, template);
    contract.erased.dictionary_factories.insert(producer.clone(), vec![MirDictionaryFactoryRequirement {
        implementation, arguments: vec![parameter.clone()], children: vec![0], result: relay.clone(),
    }]);
    contract.erased.functions.insert(producer.clone(), MirErasedFunction {
        key: producer.clone(), symbol: "selected_producer".into(), linkage: MirLinkage::Internal,
        signature: signature(id(40), vec![parameter.clone()], vec![read.clone()], vec![parameter.clone()]),
        body: Some(MirErasedBody { locals: vec![parameter.clone(), result.clone()], scopes: vec![], mutable_locals: BTreeSet::new(), blocks: vec![MirErasedBasicBlock {
            statements: vec![MirErasedAssignment { destination: MirErasedLocal(1), value: MirErasedRvalue::Call {
                target: helper.clone(), type_args: vec![parameter.clone()], dictionaries: vec![1], args: vec![MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(0)))],
            } }], terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(1)))),
        }] }),
    });
    contract.erased.functions.insert(helper.clone(), MirErasedFunction {
        key: helper, symbol: "selected_helper".into(), linkage: MirLinkage::Internal,
        signature: signature(id(41), vec![parameter.clone()], vec![relay], vec![parameter.clone()]),
        body: Some(body(MirErasedPlace::local(MirErasedLocal(0)), 0)),
    });
    contract.erased.functions.insert(entry.clone(), MirErasedFunction {
        key: entry, symbol: "selected_relay_member".into(), linkage: MirLinkage::Internal,
        signature: signature(implementation, vec![reference.clone()], vec![read], vec![parameter.clone(), reference.clone()]),
        body: Some(body(MirErasedPlace { local: MirErasedLocal(0), projection: vec![MirErasedProjection::Deref] }, 0)),
    });
    contract.runtime_requirements = crate::mir::backend_contract::runtime_requirements_for_backend(&contract, std::iter::empty());
    (MirProgram { functions: BTreeMap::new(), type_context: tc, backend_contract: contract }, producer, implementation)
}

#[test]
fn selected_blanket_environment_preserves_public_read_only_signature() {
    let (program, producer, _) = fixture();
    let errors = validate_erased_contract(&program);
    assert!(errors.is_empty(), "selected dictionary fixture: {errors:?}");
    let contract = &program.backend_contract;
    let function = &contract.erased.functions[&producer];
    let dictionaries = validate_dictionary_factories(function, contract, &program.type_context, &BTreeSet::new()).unwrap();
    assert_eq!(function.signature.dictionaries.len(), 1); assert_eq!(dictionaries.len(), 2);
    assert_eq!(dictionaries[0].trait_id.local.0, 10); assert_eq!(dictionaries[1].trait_id.local.0, 20);
    assert!(contract.runtime_requirements.contains(&MirRuntimeHelper::SelectedDictionaryFactory));
    // A scalar's conservative origin envelope is not a returned reference, and
    // must not be discarded to obtain a factory key. Unit has the same rule.
    let mut schema = dictionaries[0].clone();
    assert!(!schema.members[0].borrowing.returns.is_empty());
    let scalar = dictionary_schema_key(contract, &program.type_context, &schema, false).unwrap();
    assert!(scalar.windows(4).any(|bytes| bytes == b"BQ01"));
    let mut tc = program.type_context.clone();
    schema.members[0].ret = MirErasedType::Concrete(tc.intern_type(&Type::Unit));
    schema.members[0].borrowing = erased_borrow_contract(&schema.members[0].params, &schema.members[0].ret, schema.members[0].receiver, &tc).unwrap();
    let unit = dictionary_schema_key(contract, &tc, &schema, false).unwrap();
    assert_ne!(scalar, unit);
    schema.members[0].borrowing.returns.clear();
    assert_eq!(dictionary_schema_key(contract, &tc, &schema, false), Err(MirErasedError::InvalidDictionary));
    schema.members[0].ret = MirErasedType::Concrete(tc.intern_type(&Type::Reference { inner: Box::new(Type::I64), mutable: false }));
    schema.members[0].borrowing = erased_borrow_contract(&schema.members[0].params, &schema.members[0].ret, schema.members[0].receiver, &tc).unwrap();
    assert_eq!(dictionary_schema_key(contract, &tc, &schema, false), Err(MirErasedError::UnsupportedOperation));
    schema.members[0].ret = MirErasedType::Concrete(tc.intern_type(&Type::Unit));
    schema.members[0].receiver = Some(ReceiverMode::Mut);
    schema.members[0].borrowing = erased_borrow_contract(&schema.members[0].params, &schema.members[0].ret, schema.members[0].receiver, &tc).unwrap();
    assert_eq!(dictionary_schema_key(contract, &tc, &schema, false), Err(MirErasedError::UnsupportedOperation));
}

#[test]
fn absent_factory_never_inherits_the_helpers_dictionary() {
    let (mut program, _, _) = fixture();
    program.backend_contract.erased.dictionary_factories.clear();
    assert!(!validate_erased_contract(&program).is_empty());
}

#[test]
fn forged_impl_schema_capture_child_member_entry_and_environment_reject() {
    for control in 0..15 {
        let (mut program, producer, implementation) = fixture();
        let contract = &mut program.backend_contract;
        match control {
            0 => contract.erased.dictionary_factories.get_mut(&producer).unwrap()[0].implementation.local.0 += 1,
            1 => contract.erased.dictionary_factories.get_mut(&producer).unwrap()[0].children.clear(),
            2 => contract.erased.dictionary_factories.get_mut(&producer).unwrap()[0].children[0] = 1,
            3 => contract.erased.dictionary_factories.get_mut(&producer).unwrap()[0].arguments[0] = MirErasedType::Parameter(9),
            4 => contract.erased.dictionary_factories.get_mut(&producer).unwrap()[0].result.members[0].member_id.local.0 += 1,
            5 => contract.erased.dictionary_templates.get_mut(&implementation).unwrap().members[0].method_id.local.0 += 1,
            6 => contract.erased.dictionary_templates.get_mut(&implementation).unwrap().bounds.clear(),
            7 => { let entry = contract.erased.dictionary_templates[&implementation].members[0].entry.clone(); contract.erased.functions.get_mut(&entry).unwrap().body = None; },
            8 => { let duplicate = contract.erased.dictionary_factories[&producer][0].clone(); contract.erased.dictionary_factories.get_mut(&producer).unwrap().push(duplicate); },
            9 => contract.erased.dictionary_templates.get_mut(&implementation).unwrap().parameters[0].kind = Kind::arrow(Kind::Type, Kind::Type),
            10 => contract.erased.dictionary_factories.get_mut(&producer).unwrap()[0].arguments[0] = MirErasedType::Scoped(0),
            11 => contract.erased.dictionary_factories.get_mut(&producer).unwrap()[0].result.trait_id.local.0 += 1,
            12 => contract.erased.dictionary_templates.get_mut(&implementation).unwrap().result.members[0].borrowing.returns.clear(),
            13 => contract.erased.dictionary_templates.get_mut(&implementation).unwrap().bounds[0].members[0].borrowing.returns[0].sources[0].argument = 1,
            _ => contract.descriptor_origins.clear(),
        }
        assert!(!validate_erased_contract(&program).is_empty(), "control {control}");
    }
}

#[test]
fn missing_stale_or_orphaned_dictionary_runtime_roots_reject_agreement() {
    let (mut program, producer, _) = fixture();
    program.backend_contract.runtime_requirements.remove(&MirRuntimeHelper::SelectedDictionaryFactory);
    assert!(!agreement::check_mir_runtime_agreement(&program).is_clean());
    program.backend_contract.erased.dictionary_factories.clear();
    program.backend_contract.runtime_requirements.insert(MirRuntimeHelper::SelectedDictionaryFactory);
    assert!(!agreement::check_mir_runtime_agreement(&program).is_clean());
    program.backend_contract.erased.dictionary_factories.insert(producer.clone(), vec![]);
    assert!(!validate_erased_contract(&program).is_empty());
}

#[test]
fn bodyless_import_keeps_only_the_typed_selected_dictionary_runtime_root() {
    let (mut program, _, _) = fixture();
    program.backend_contract.erased.functions.clear(); program.backend_contract.erased.dictionary_factories.clear();
    program.backend_contract.erased.dictionary_templates.clear();
    let mut target = b"RT01".to_vec();
    for part in [b"x86_64-unknown-linux-gnu".as_slice(), b"e-p:64:64".as_slice()] { dictionary_frame(&mut target, part); }
    program.backend_contract.erased.imported_runtime_support.insert(crate::products::ProductRuntimeSupport::selected_dictionaries(target));
    let requirements = crate::mir::backend_contract::runtime_requirements_for_backend(&program.backend_contract, std::iter::empty());
    assert_eq!(requirements, BTreeSet::from([MirRuntimeHelper::SelectedDictionaryFactory]));
}

#[test]
fn native_witness_keys_bind_exact_impl_member_receiver_and_callable_abi() {
    let (mut program, producer, _) = fixture();
    let id = |n| DefId::new(CrateId(0), LocalDefId(n));
    let tc = &mut program.type_context; let contract = &mut program.backend_contract;
    let subject = tc.intern_type(&Type::I64);
    let reference = tc.intern_type(&Type::Reference { inner: Box::new(Type::I64), mutable: false });
    let method = id(51); let target = MirCallableKey::Extern(method); let dictionary = MirDictionaryId(0);
    contract.callables.insert(target.clone(), MirCallableDecl { key: target.clone(), source_def_id: Some(method),
        kind: MirCallableKind::Extern { link_name: "native_read".into(), variadic: false }, llvm_symbol: "native_read".into(), linkage: MirLinkage::External,
        signature: MirCallableSignature::from_instance_type_ids(&[reference], subject, true, tc) });
    let schema = contract.erased.functions[&producer].signature.dictionaries[0].substitute(&[MirErasedType::Concrete(subject)], tc).unwrap();
    contract.erased.dictionaries.insert(dictionary, MirConcreteDictionary { subject, trait_id: id(10), trait_args: vec![],
        members: vec![MirConcreteDictionaryMember { member_id: id(11), receiver: Some(ReceiverMode::Shared), target: MirDictionaryMemberTarget::Native(target.clone()) }] });
    contract.erased.dictionary_witnesses.insert(dictionary, MirDictionaryWitness { implementation: id(50), schema, methods: vec![method] });
    let keys = native_dictionary_keys(contract, tc, dictionary).unwrap();
    assert_eq!(&keys.0[..4], b"DK01"); assert_eq!(&keys.1[..4], b"DI01");
    for control in 0..7 {
        let mut corrupt = contract.clone();
        match control {
            0 => corrupt.erased.dictionary_witnesses.get_mut(&dictionary).unwrap().implementation.local.0 += 1,
            1 => corrupt.erased.dictionary_witnesses.get_mut(&dictionary).unwrap().methods[0].local.0 += 1,
            2 => corrupt.erased.dictionary_witnesses.get_mut(&dictionary).unwrap().schema.members[0].member_id.local.0 += 1,
            3 => corrupt.callables.get_mut(&target).unwrap().signature.params[0].semantic_ty = subject,
            4 => corrupt.callables.get_mut(&target).unwrap().source_def_id = Some(id(52)),
            5 => corrupt.callables.get_mut(&target).unwrap().signature.params[0].pass_mode = MirPassMode::Direct,
            _ => corrupt.callables.get_mut(&target).unwrap().signature.ret.abi_ty = reference,
        }
        assert!(native_dictionary_keys(&corrupt, tc, dictionary).is_err(), "native control {control}");
    }
}
