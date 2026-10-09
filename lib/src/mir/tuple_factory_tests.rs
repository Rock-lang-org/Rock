use crate::mir::*;
use crate::type_context::TypeContext;
use crate::type_services::kind::Kind;
use std::collections::{BTreeMap, BTreeSet};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mir::backend_contract::runtime_requirements_for_backend;
use crate::types::{GenericParamId, Type};

pub(crate) fn fixture() -> (MirProgram, MirErasedKey) {
    let mut tc = TypeContext::new();
    let unit = tc.intern_type(&Type::Unit);
    let mut contract = MirBackendContract::default();
    let drop = payload_drop_plan(&contract, &mut tc, unit).unwrap();
    let identity = descriptor_identity::descriptor_identity(&contract, &tc, &drop).unwrap();
    contract.descriptor_identities.insert(unit, identity);
    contract.erased.descriptors.insert(unit, MirTypeDescriptor { ty: unit, drop, fields: vec![] });
    let id = DefId::new(CrateId(0), LocalDefId(50));
    let key = MirErasedKey { definition: id, static_args: vec![] };
    let helper_id = DefId::new(CrateId(0), LocalDefId(51));
    let helper_key = MirErasedKey { definition: helper_id, static_args: vec![] };
    let parameter = MirErasedType::Parameter(0);
    let tuple = MirErasedType::Tuple(vec![parameter.clone(), parameter.clone()]);
    let signature = |owner, params| MirErasedSignature {
        parameters: vec![MirErasedParameter { source: GenericParamId { owner, index: 0 }, kind: Kind::Type }],
        params, ret: MirErasedType::Concrete(unit), dictionaries: vec![],
        layouts: vec![parameter.clone()], require_borrow_free: BTreeSet::new(),
    };
    let operand = |local| MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(local)));
    let function = MirErasedFunction {
        key: key.clone(), symbol: "factory_body".into(), linkage: MirLinkage::Internal,
        signature: signature(id, vec![parameter.clone(), parameter.clone()]),
        body: Some(MirErasedBody {
            locals: vec![parameter.clone(), parameter.clone(), tuple.clone(), MirErasedType::Concrete(unit)],
            mutable_locals: BTreeSet::new(), scopes: vec![], blocks: vec![MirErasedBasicBlock {
                statements: vec![
                    MirErasedAssignment { destination: MirErasedLocal(2), value: MirErasedRvalue::Aggregate(vec![operand(0), operand(1)]) },
                    MirErasedAssignment { destination: MirErasedLocal(3), value: MirErasedRvalue::Call { target: helper_key.clone(), type_args: vec![tuple], dictionaries: vec![], args: vec![operand(2)] } },
                ],
                terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(3)))),
            }],
        }),
    };
    let requirements = collect_body_factories(&function, &contract, &tc).unwrap().into_iter().map(|r| {
        let MirBodyFactoryRequirement::Tuple(r) = r else { panic!("tuple fixture"); }; r
    }).collect();
    contract.erased.tuple_factories.insert(key.clone(), requirements);
    contract.erased.functions.insert(key.clone(), function);
    contract.erased.functions.insert(helper_key.clone(), MirErasedFunction {
        key: helper_key, symbol: "discard_pair".into(), linkage: MirLinkage::Internal,
        signature: signature(helper_id, vec![parameter.clone()]),
        body: Some(MirErasedBody { locals: vec![parameter, MirErasedType::Concrete(unit)], mutable_locals: BTreeSet::new(), scopes: vec![], blocks: vec![MirErasedBasicBlock {
            statements: vec![MirErasedAssignment { destination: MirErasedLocal(1), value: MirErasedRvalue::Literal(MirErasedLiteral::Unit) }],
            terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(1)))),
        }] }),
    });
    contract.runtime_requirements = runtime_requirements_for_backend(&contract, std::iter::empty());
    (MirProgram { functions: BTreeMap::new(), type_context: tc, backend_contract: contract }, key)
}

#[test]
fn checked_body_local_tuple_forwards_whole_noncopy_value_to_an_erased_helper() {
    let (program, key) = fixture();
    let contract = &program.backend_contract;
    assert!(validate_erased_contract(&program).is_empty());
    let function = &contract.erased.functions[&key];
    assert_eq!(contract.erased.tuple_factories[&key].len(), 1);
    assert!(!function.signature.layouts.iter().any(|ty| matches!(ty, MirErasedType::Tuple(_))));
    assert!(contract.runtime_requirements.contains(&MirRuntimeHelper::TupleDescriptorFactory));
}

#[test]
fn malformed_factory_arity_target_kind_and_child_ordinal_are_rejected() {
    for control in 0..4 {
        let (mut program, key) = fixture();
        let requirement = &mut program.backend_contract.erased.tuple_factories.get_mut(&key).unwrap()[0];
        let expected = match control {
            0 => { requirement.children.pop(); MirErasedError::InvalidType },
            1 => { requirement.descriptor_abi = 99; MirErasedError::UnsupportedOperation },
            2 => { requirement.result = MirErasedType::Parameter(0); MirErasedError::InvalidType },
            _ => { requirement.children[0] = MirErasedType::Parameter(99); requirement.result = MirErasedType::Tuple(requirement.children.clone()); MirErasedError::UnboundParameter },
        };
        assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(expected));
    }
}

#[test]
fn factory_children_cannot_use_shape_only_scoped_evidence() {
    let (mut program, key) = fixture();
    let requirement = &mut program.backend_contract.erased.tuple_factories.get_mut(&key).unwrap()[0];
    requirement.children[0] = MirErasedType::Scoped(0);
    requirement.result = MirErasedType::Tuple(requirement.children.clone());
    assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::UnboundParameter));
}

#[test]
fn dependency_first_factory_order_is_required_before_local_allocation() {
    let (mut program, key) = fixture();
    let tuple = program.backend_contract.erased.tuple_factories[&key][0].result.clone();
    let nested = MirErasedType::Tuple(vec![tuple.clone(), MirErasedType::Parameter(0)]);
    program.backend_contract.erased.functions.get_mut(&key).unwrap().body.as_mut().unwrap().locals.push(nested.clone());
    let entry = program.backend_contract.erased.tuple_factories.get_mut(&key).unwrap();
    entry.push(MirTupleFactoryRequirement { descriptor_abi: 1, result: nested, children: vec![tuple, MirErasedType::Parameter(0)] });
    assert!(validate_erased_contract(&program).is_empty());
    program.backend_contract.erased.tuple_factories.get_mut(&key).unwrap().reverse();
    assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::MissingLayout));
}

#[test]
fn a_missing_factory_does_not_inherit_a_callee_forwarded_layout() {
    let (mut program, key) = fixture();
    program.backend_contract.erased.tuple_factories.clear();
    assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::MissingLayout));
}

#[test]
fn callee_public_layouts_must_be_available_from_the_actual_caller_map() {
    let (mut program, key) = fixture();
    let helper = program.backend_contract.erased.functions.values_mut().find(|entry| entry.key != key).unwrap();
    helper.signature.layouts.push(MirErasedType::Tuple(vec![MirErasedType::Parameter(0); 3]));
    assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::MissingLayout));
}

#[test]
fn a_constructor_kind_parameter_is_not_a_sized_descriptor_child() {
    let (mut program, key) = fixture();
    program.backend_contract.erased.functions.get_mut(&key).unwrap().signature.parameters[0].kind = Kind::Arrow(Box::new(Kind::Type), Box::new(Kind::Type));
    assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::KindMismatch));
}

#[test]
fn emitted_erased_bodies_keep_the_runtime_edge_and_helper_sets_stay_exact() {
    let (mut program, _) = fixture();
    assert!(program.functions.is_empty()); // No concrete instance/call is needed.
    let observed = runtime_requirements_for_backend(&program.backend_contract, program.functions.values());
    assert!(observed.contains(&MirRuntimeHelper::TupleDescriptorFactory));
    assert!(agreement::check_mir_runtime_agreement(&program).is_clean());
    program.backend_contract.runtime_requirements.remove(&MirRuntimeHelper::TupleDescriptorFactory);
    assert!(!agreement::check_mir_runtime_agreement(&program).is_clean());
    program.backend_contract.erased.tuple_factories.clear();
    program.backend_contract.erased.functions.clear();
    program.backend_contract.runtime_requirements.insert(MirRuntimeHelper::TupleDescriptorFactory);
    assert!(!agreement::check_mir_runtime_agreement(&program).is_clean());
}

#[test]
fn bodyless_import_roots_keep_producer_only_runtime_support() {
    let (mut program, _) = fixture();
    program.backend_contract.erased.tuple_factories.clear();
    program.backend_contract.erased.functions.clear();
    let target = {
        let mut key = b"RT01".to_vec();
        for bytes in [b"x86_64-unknown-linux-gnu".as_slice(), b"e-p:64:64".as_slice()] { key.extend_from_slice(&(bytes.len() as u64).to_le_bytes()); key.extend_from_slice(bytes); }
        key
    };
    program.backend_contract.erased.imported_runtime_support.insert(crate::products::ProductRuntimeSupport::tuple_descriptors(target));
    assert!(validate_erased_contract(&program).is_empty());
    assert!(runtime_requirements_for_backend(&program.backend_contract, program.functions.values()).contains(&MirRuntimeHelper::TupleDescriptorFactory));
}

#[test]
fn orphaned_or_bodyless_factories_do_not_fabricate_an_observed_runtime_edge() {
    let (mut program, key) = fixture();
    program.backend_contract.erased.functions.get_mut(&key).unwrap().body = None;
    assert!(!runtime_requirements_for_backend(&program.backend_contract, program.functions.values()).contains(&MirRuntimeHelper::TupleDescriptorFactory));
    assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::UnsupportedOperation));
    program.backend_contract.erased.functions.remove(&key);
    assert!(!validate_erased_contract(&program).is_empty());
}

#[test]
fn concrete_factory_child_requires_an_actual_semantic_descriptor_not_just_layout() {
    let (mut program, key) = fixture();
    let unit = program.backend_contract.erased.descriptors.keys().next().copied().unwrap();
    let requirement = &mut program.backend_contract.erased.tuple_factories.get_mut(&key).unwrap()[0];
    requirement.children[0] = MirErasedType::Concrete(unit);
    requirement.result = MirErasedType::Tuple(requirement.children.clone());
    program.backend_contract.descriptor_identities.remove(&unit);
    assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::MissingLayout));
}
