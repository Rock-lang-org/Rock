use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use inkwell::{context::Context, OptimizationLevel};
use crate::ids::{CrateId, DefId, InstanceId, LocalDefId};
use crate::mir::*;
use crate::type_context::TypeContext;
use crate::type_services::kind::Kind;
use crate::types::{GenericParamId, Type};
use super::object_tests::{insert_body, local, place};

fn erased_identity() -> MirErasedFunction {
    let definition = DefId::new(CrateId(1), LocalDefId(7));
    let ty = MirErasedType::Parameter(0);
    MirErasedFunction { key: MirErasedKey { definition, static_args: vec![] }, symbol: "aot.identity".into(), linkage: MirLinkage::External,
        signature: MirErasedSignature { parameters: vec![MirErasedParameter { source: GenericParamId { owner: definition, index: 0 }, kind: Kind::Type }], params: vec![ty.clone()], ret: ty.clone(), dictionaries: vec![], layouts: vec![ty.clone()], require_borrow_free: BTreeSet::new() },
        body: Some(MirErasedBody { locals: vec![ty.clone(), ty], mutable_locals: BTreeSet::new(), statements: vec![MirErasedAssignment { destination: MirErasedLocal(1), value: MirErasedRvalue::Use(MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(0)))) }], result: MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(1))) }) }
}

fn identity_producer() -> MirProgram {
    let function = erased_identity();
    let mut contract = MirBackendContract::default();
    contract.erased.functions.insert(function.key.clone(), function);
    MirProgram { functions: BTreeMap::new(), type_context: TypeContext::new(), backend_contract: contract }
}

#[test]
fn aot_identity_executes_with_later_noncopy_type_and_descriptor_drop() {
    static DROPS: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" fn record(value: i64) { DROPS.fetch_add(value as u64, Ordering::SeqCst); }
    let context = Context::create();
    let producer = identity_producer();
    let mut producer_codegen = super::CodeGen::new(&context, "producer");
    producer_codegen.compile_program_from_mir(&producer).unwrap();
    producer_codegen.module.verify().unwrap();
    let before = producer_codegen.get_ir();
    assert!(before.contains("erased.storage.aligned"));
    assert!(before.contains("llvm.memmove"));
    let mut tc = TypeContext::new();
    let scalar = tc.intern_type(&Type::I64);
    let unit = tc.intern_type(&Type::Unit);
    let late_id = DefId::new(CrateId(9), LocalDefId(1));
    let late_ty = Type::Struct { id: late_id, args: vec![] };
    assert!(!late_ty.is_copy());
    let late = tc.intern_type(&late_ty);
    let mut contract = MirBackendContract::default();
    contract.nominal_layouts.insert(late_id, MirNominalLayout::Struct { id: late_id, fields: vec![("a".into(), scalar), ("b".into(), scalar), ("c".into(), scalar)], generic_params: vec![] });
    let record_key = MirCallableKey::Extern(DefId::new(CrateId(9), LocalDefId(2)));
    contract.callables.insert(record_key.clone(), MirCallableDecl { key: record_key.clone(), source_def_id: None, kind: MirCallableKind::Extern { link_name: "record_late_drop".into(), variadic: false }, llvm_symbol: "record_late_drop".into(), linkage: MirLinkage::External, signature: MirCallableSignature::from_type_ids(&[scalar], unit, MirPassMode::Direct) });
    let mut caller = MirProgram { functions: BTreeMap::new(), type_context: tc, backend_contract: contract };
    let field = |index| Place { local: Local(1), projection: vec![Projection::Field { index, identity: None }] };
    insert_body(&mut caller, MirFunction { id: MirFunctionId::Instance(InstanceId(1)), name: "drop_late".into(), local_decls: vec![local(unit, LocalSource::ReturnPlace), local(late, LocalSource::Argument)], arg_count: 1, ret_type: unit, closure_captures: vec![], ownership: Default::default(),
        basic_blocks: vec![BasicBlock { statements: vec![], terminator: Some(Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(record_key))), args: vec![Operand::Copy(field(0))], destination: place(0), target: BasicBlockId(1), span: None }) }, BasicBlock { statements: vec![], terminator: Some(Terminator::Return) }],
    }, "drop_late", &[late], true);
    caller.backend_contract.drop_glue.insert(late, MirCallableKey::Instance(InstanceId(1)));
    let drop = payload_drop_plan(&caller.backend_contract, &mut caller.type_context, late).unwrap();
    caller.backend_contract.erased.descriptors.insert(late, MirTypeDescriptor { ty: late, drop, fields: vec![scalar; 3] });
    let mut declaration = erased_identity(); declaration.body = None;
    let target = declaration.key.clone();
    caller.backend_contract.erased.functions.insert(target.clone(), declaration);
    let out_field = |index| Place { local: Local(2), projection: vec![Projection::Field { index, identity: None }] };
    insert_body(&mut caller, MirFunction { id: MirFunctionId::Instance(InstanceId(0)), name: "run_late_identity".into(), local_decls: vec![local(scalar, LocalSource::ReturnPlace), local(late, LocalSource::UserBinding), local(late, LocalSource::UserBinding), local(scalar, LocalSource::Temporary)], arg_count: 0, ret_type: scalar, closure_captures: vec![], ownership: Default::default(),
        basic_blocks: vec![BasicBlock { statements: vec![StatementData::assign(place(1), Rvalue::Aggregate(AggregateKind::Struct { id: late_id, display_name: "Late".into() }, vec![Operand::Constant(Constant::Int(40)), Operand::Constant(Constant::Int(1)), Operand::Constant(Constant::Int(1))]), None)],
            terminator: Some(Terminator::Call { func: Operand::Constant(Constant::ErasedCall(MirErasedCall { target: MirErasedCallTarget::Direct(target), type_args: vec![late], dictionaries: vec![] })), args: vec![Operand::Move(place(1))], destination: place(2), target: BasicBlockId(1), span: None }) },
            BasicBlock { statements: vec![StatementData::assign(place(3), Rvalue::BinaryOp(MirBinOp::Add, Operand::Copy(out_field(0)), Operand::Copy(out_field(1))), None), StatementData::assign(place(0), Rvalue::BinaryOp(MirBinOp::Add, Operand::Copy(place(3)), Operand::Copy(out_field(2))), None)], terminator: Some(Terminator::Drop { place: place(2), target: BasicBlockId(2) }) },
            BasicBlock { statements: vec![], terminator: Some(Terminator::Return) }],
    }, "run_late_identity", &[], false);
    assert!(validate_erased_contract(&caller).is_empty(), "{:?}", validate_erased_contract(&caller));
    let mut caller_codegen = super::CodeGen::new(&context, "caller");
    caller_codegen.compile_program_from_mir(&caller).unwrap();
    caller_codegen.module.verify().unwrap();
    assert_eq!(producer_codegen.get_ir(), before, "producer entry must not be rebuilt for a future type");
    let recorder = caller_codegen.module.get_function("record_late_drop").unwrap();
    let engine = producer_codegen.module.create_jit_execution_engine(OptimizationLevel::None).unwrap();
    engine.add_module(&caller_codegen.module).unwrap();
    engine.add_global_mapping(&recorder, record as *const () as usize);
    DROPS.store(0, Ordering::SeqCst);
    unsafe { assert_eq!(engine.get_function::<unsafe extern "C" fn() -> i64>("run_late_identity").unwrap().call(), 42); }
    assert_eq!(DROPS.load(Ordering::SeqCst), 40, "returned ownership must be destroyed exactly once by its caller");
}

#[test]
fn erased_contract_rejects_unbound_parameters_and_unproved_copy() {
    for unbound in [false, true] {
        let mut program = identity_producer();
        let function = program.backend_contract.erased.functions.values_mut().next().unwrap();
        if unbound { function.body.as_mut().unwrap().locals[1] = MirErasedType::Parameter(99); }
        else { function.body.as_mut().unwrap().statements[0].value = MirErasedRvalue::Use(MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(0)))); }
        let errors = validate_erased_contract(&program);
        assert!(errors.iter().any(|(_, error)| if unbound { *error == MirErasedError::UnboundParameter } else { *error == MirErasedError::CopyWithoutProof }), "{errors:?}");
        let context = Context::create();
        assert!(super::CodeGen::new(&context, "invalid-erased").compile_program_from_mir(&program).is_err());
    }
}

fn dictionary_callback_producer(receiver: crate::types::ReceiverMode) -> MirProgram {
    let mut program = identity_producer();
    let scalar = program.type_context.intern_type(&Type::I64);
    let drop = payload_drop_plan(&program.backend_contract, &mut program.type_context, scalar).unwrap();
    program.backend_contract.erased.descriptors.insert(scalar, MirTypeDescriptor { ty: scalar, drop, fields: vec![] });
    let function = program.backend_contract.erased.functions.values_mut().next().unwrap();
    let callback = MirErasedType::Parameter(0);
    let argument = MirErasedType::Concrete(scalar);
    function.signature.params = vec![callback.clone(), argument.clone()];
    function.signature.ret = argument.clone();
    function.signature.dictionaries = vec![MirErasedDictionarySchema {
        subject: callback.clone(), trait_id: DefId::new(CrateId(1), LocalDefId(8)),
        trait_args: vec![argument.clone(), argument.clone()],
        members: vec![MirErasedDictionaryMember {
            member_id: DefId::new(CrateId(1), LocalDefId(9)), receiver,
            params: vec![callback.clone(), argument.clone()], ret: argument.clone(),
        }],
    }];
    function.body = Some(MirErasedBody {
        locals: vec![callback, argument.clone(), argument.clone(), argument],
        mutable_locals: [MirErasedLocal(0)].into_iter().collect(),
        statements: vec![MirErasedAssignment {
            destination: MirErasedLocal(2), value: MirErasedRvalue::DictionaryCall {
                dictionary: 0, member: 0, receiver: MirErasedPlace::local(MirErasedLocal(0)),
                args: vec![MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(1)))],
            },
        }],
        result: MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(2))),
    });
    program
}

#[test]
fn erased_mutable_callback_requires_write_access_and_excludes_live_loans() {
    let producer = dictionary_callback_producer(crate::types::ReceiverMode::Mut);
    assert!(validate_erased_contract(&producer).is_empty());
    let mut immutable = producer.clone();
    immutable.backend_contract.erased.functions.values_mut().next().unwrap().body.as_mut().unwrap().mutable_locals.clear();
    assert!(validate_erased_contract(&immutable).iter().any(|(_, error)| *error == MirErasedError::BorrowConflict));
    let mut borrowed = producer;
    let function = borrowed.backend_contract.erased.functions.values_mut().next().unwrap();
    let reference = MirErasedType::Reference { inner: Box::new(MirErasedType::Parameter(0)), mutable: false };
    function.signature.layouts.push(reference.clone());
    let body = function.body.as_mut().unwrap();
    body.locals.push(reference);
    body.statements.insert(0, MirErasedAssignment {
        destination: MirErasedLocal(4), value: MirErasedRvalue::Borrow { place: MirErasedPlace::local(MirErasedLocal(0)), mutable: false },
    });
    assert!(validate_erased_contract(&borrowed).iter().any(|(_, error)| *error == MirErasedError::BorrowConflict));
}

#[test]
fn erased_consuming_callback_cannot_be_invoked_twice() {
    let mut producer = dictionary_callback_producer(crate::types::ReceiverMode::Move);
    assert!(validate_erased_contract(&producer).is_empty());
    let body = producer.backend_contract.erased.functions.values_mut().next().unwrap().body.as_mut().unwrap();
    let mut second = body.statements[0].clone();
    second.destination = MirErasedLocal(3);
    body.statements.push(second);
    assert!(validate_erased_contract(&producer).iter().any(|(_, error)| *error == MirErasedError::UninitializedOrMoved));
}
