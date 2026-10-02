use std::collections::BTreeMap;

use inkwell::{context::Context, OptimizationLevel};

use crate::ids::{CrateId, DefId, InstanceId, LocalDefId, TypeId};
use crate::mir::*;
use crate::type_context::TypeContext;
use crate::types::{ObjectType, ReceiverMode, TraitBound, Type};

#[path = "object_borrow_tests.rs"]
mod borrow_tests;

pub(super) fn definition(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}
pub(super) fn place(local: usize) -> Place {
    Place {
        local: Local(local),
        projection: Vec::new(),
    }
}
fn pointee(local: usize) -> Place {
    Place {
        local: Local(local),
        projection: vec![Projection::Deref],
    }
}
pub(super) fn local(ty: TypeId, source: LocalSource) -> LocalDecl {
    LocalDecl {
        ty,
        source,
        mutability: Mutability::Mut,
        name: None,
        span: None,
    }
}

pub(super) fn insert_body(
    program: &mut MirProgram,
    body: MirFunction,
    symbol: &str,
    params: &[TypeId],
    method: bool,
) {
    let MirFunctionId::Instance(instance) = body.id else {
        unreachable!()
    };
    let key = MirCallableKey::Instance(instance);
    program.backend_contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(definition(instance.0)),
            kind: MirCallableKind::LocalBody {
                function_id: body.id.clone(),
            },
            llvm_symbol: symbol.into(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_instance_type_ids(
                params,
                body.ret_type,
                method,
                &program.type_context,
            ),
        },
    );
    program
        .backend_contract
        .function_bodies
        .insert(body.id.clone(), key);
    program.functions.insert(body.id.clone(), body);
}

/// Real contract fixture: the selected supertrait entry is not a prefix slot.
pub(super) fn object_program(mutable: bool) -> MirProgram {
    let mut tc = TypeContext::new();
    let scalar = tc.intern_type(&Type::I64);
    let concrete_ref = tc.intern_type(&Type::Reference {
        inner: Box::new(Type::I64),
        mutable,
    });
    let first = TraitBound {
        trait_id: definition(10),
        type_args: vec![],
    };
    let second = TraitBound {
        trait_id: definition(11),
        type_args: vec![],
    };
    let mut root_object = ObjectType::new(first.clone());
    root_object.guarantees.insert(second.clone());
    let root_ty = Type::Object(Box::new(root_object));
    let target_ty = Type::Object(Box::new(ObjectType::new(second.clone())));
    let root = tc.intern_type(&root_ty);
    let target = tc.intern_type(&target_ty);
    let root_ref = tc.intern_type(&Type::Reference {
        inner: Box::new(root_ty),
        mutable,
    });
    let target_ref = tc.intern_type(&Type::Reference {
        inner: Box::new(target_ty),
        mutable,
    });
    let receiver = if mutable {
        ReceiverMode::Mut
    } else {
        ReceiverMode::Shared
    };
    let slot = |object_ref, trait_id, member_id| {
        let mut signature =
            MirCallableSignature::from_type_ids(&[object_ref], scalar, MirPassMode::Direct);
        signature.params[0].pass_mode = MirPassMode::FatDirect;
        MirObjectSlot {
            result: MirObjectResult::Direct,
            trait_id,
            member_id,
            trait_args: vec![],
            receiver,
            signature,
        }
    };
    let mut contract = MirBackendContract::default();
    contract.object_schemas.insert(
        root,
        MirObjectSchema {
            erased_slots: vec![],
            object: root,
            slots: vec![
                slot(root_ref, first.trait_id, definition(12)),
                slot(root_ref, second.trait_id, definition(13)),
            ],
            views: vec![target],
        },
    );
    contract.object_schemas.insert(
        target,
        MirObjectSchema {
            erased_slots: vec![],
            object: target,
            slots: vec![slot(target_ref, second.trait_id, definition(13))],
            views: vec![],
        },
    );
    contract.vtables.insert(
        MirVtableId(0),
        MirVtable {
            erased_methods: vec![],
            object: root,
            concrete: scalar,
            methods: vec![
                MirCallableKey::Instance(InstanceId(2)),
                MirCallableKey::Instance(InstanceId(1)),
            ],
            views: vec![MirVtableId(1)],
            drop: None,
        },
    );
    contract.vtables.insert(
        MirVtableId(1),
        MirVtable {
            erased_methods: vec![],
            object: target,
            concrete: scalar,
            methods: vec![MirCallableKey::Instance(InstanceId(1))],
            views: vec![],
            drop: None,
        },
    );
    let mut program = MirProgram {
        functions: BTreeMap::new(),
        type_context: tc,
        backend_contract: contract,
    };
    let mut method_statements = Vec::new();
    if mutable {
        method_statements.push(StatementData::assign(
            pointee(1),
            Rvalue::BinaryOp(
                MirBinOp::Add,
                Operand::Copy(pointee(1)),
                Operand::Constant(Constant::Int(1)),
            ),
            None,
        ));
    }
    method_statements.push(StatementData::assign(
        place(0),
        Rvalue::Use(Operand::Copy(pointee(1))),
        None,
    ));
    insert_body(
        &mut program,
        MirFunction {
            id: MirFunctionId::Instance(InstanceId(1)),
            name: "read_or_increment".into(),
            local_decls: vec![
                local(scalar, LocalSource::ReturnPlace),
                local(concrete_ref, LocalSource::Argument),
            ],
            arg_count: 1,
            ret_type: scalar,
            closure_captures: vec![],
            ownership: Default::default(),
            basic_blocks: vec![BasicBlock {
                statements: method_statements,
                terminator: Some(Terminator::Return),
            }],
        },
        "read_or_increment",
        &[concrete_ref],
        true,
    );
    insert_body(
        &mut program,
        MirFunction {
            id: MirFunctionId::Instance(InstanceId(2)),
            name: "different_slot".into(),
            local_decls: vec![
                local(scalar, LocalSource::ReturnPlace),
                local(concrete_ref, LocalSource::Argument),
            ],
            arg_count: 1,
            ret_type: scalar,
            closure_captures: vec![],
            ownership: Default::default(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    place(0),
                    Rvalue::Use(Operand::Constant(Constant::Int(999))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
        },
        "different_slot",
        &[concrete_ref],
        true,
    );
    insert_body(
        &mut program,
        MirFunction {
            id: MirFunctionId::Instance(InstanceId(0)),
            name: "run_object".into(),
            local_decls: vec![
                local(scalar, LocalSource::ReturnPlace),
                local(scalar, LocalSource::UserBinding),
                local(concrete_ref, LocalSource::Temporary),
                local(root_ref, LocalSource::UserBinding),
                local(target_ref, LocalSource::UserBinding),
            ],
            arg_count: 0,
            ret_type: scalar,
            closure_captures: vec![],
            ownership: Default::default(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![
                        StatementData::assign(
                            place(1),
                            Rvalue::Use(Operand::Constant(Constant::Int(41))),
                            None,
                        ),
                        StatementData::assign(
                            place(2),
                            Rvalue::Ref(
                                if mutable {
                                    Mutability::Mut
                                } else {
                                    Mutability::Not
                                },
                                place(1),
                            ),
                            None,
                        ),
                        StatementData::assign(
                            place(3),
                            Rvalue::Object(
                                Operand::Move(place(2)),
                                MirObjectConversion::Concrete(MirVtableId(0)),
                            ),
                            None,
                        ),
                        StatementData::assign(
                            place(4),
                            Rvalue::Object(
                                Operand::Move(place(3)),
                                MirObjectConversion::Upcast {
                                    source: root,
                                    view: 0,
                                },
                            ),
                            None,
                        ),
                    ],
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::VirtualTarget(MirVirtualTarget {
                            object: target,
                            slot: 0,
                        })),
                        args: vec![Operand::Move(place(4))],
                        destination: place(0),
                        target: BasicBlockId(1),
                        span: None,
                    }),
                },
                BasicBlock {
                    statements: vec![],
                    terminator: Some(Terminator::Return),
                },
            ],
        },
        "run_object",
        &[],
        false,
    );
    program
}

#[test]
fn borrowed_object_vtables_execute_shared_and_mutable_nonprefix_upcasts() {
    for mutable in [false, true] {
        let program = object_program(mutable);
        let report = agreement::check_mir_runtime_agreement(&program);
        assert!(report.is_clean(), "{report:?}");
        let context = Context::create();
        let mut codegen = super::CodeGen::new(&context, "object-execution");
        codegen.compile_program_from_mir(&program).unwrap();
        codegen.module.verify().unwrap();
        let ir = codegen.get_ir();
        assert!(ir.contains("private constant"), "{ir}");
        assert!(ir.contains("object.call"), "{ir}");
        let engine = codegen
            .module
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        unsafe {
            let run = engine
                .get_function::<unsafe extern "C" fn() -> i64>("run_object")
                .unwrap();
            assert_eq!(run.call(), if mutable { 42 } else { 41 });
        }
    }
}

#[test]
fn object_preflight_rejects_bad_slot_receiver_payload_and_view_contracts() {
    for mutation in 0..5 {
        let mut program = object_program(false);
        match mutation {
            0 => {
                program
                    .backend_contract
                    .vtables
                    .get_mut(&MirVtableId(0))
                    .unwrap()
                    .methods
                    .pop();
            }
            1 => {
                program
                    .backend_contract
                    .vtables
                    .get_mut(&MirVtableId(0))
                    .unwrap()
                    .views[0] = MirVtableId(99);
            }
            2 => {
                program
                    .backend_contract
                    .object_schemas
                    .values_mut()
                    .next()
                    .unwrap()
                    .slots[0]
                    .receiver = ReceiverMode::Move;
            }
            3 => {
                program
                    .backend_contract
                    .vtables
                    .get_mut(&MirVtableId(0))
                    .unwrap()
                    .concrete = TypeId(u32::MAX);
            }
            4 => {
                let function = program
                    .functions
                    .get_mut(&MirFunctionId::Instance(InstanceId(0)))
                    .unwrap();
                let Some(Terminator::Call {
                    func: Operand::Constant(Constant::VirtualTarget(target)),
                    ..
                }) = &mut function.basic_blocks[0].terminator
                else {
                    unreachable!()
                };
                target.slot = 99;
            }
            _ => unreachable!(),
        }
        assert!(!validate_object_contract(&program).is_empty());
        let context = Context::create();
        let mut codegen = super::CodeGen::new(&context, "invalid-object");
        assert!(
            codegen.compile_program_from_mir(&program).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn object_construction_and_upcast_keep_payload_loan_live() {
    let mut program = object_program(false);
    crate::mir::borrowck::BorrowChecker::run(&program).unwrap();
    let caller = program
        .functions
        .get_mut(&MirFunctionId::Instance(InstanceId(0)))
        .unwrap();
    caller.basic_blocks[0]
        .statements
        .push(StatementData::assign(
            place(1),
            Rvalue::Use(Operand::Constant(Constant::Int(100))),
            None,
        ));
    assert!(crate::mir::borrowck::BorrowChecker::run(&program).is_err());
}

#[test]
fn object_vtable_uses_object_provided_link_symbol() {
    unsafe extern "C" fn external_method(receiver: *const i64) -> i64 {
        *receiver + 5
    }
    let mut program = object_program(false);
    let id = MirFunctionId::Instance(InstanceId(1));
    program.functions.remove(&id);
    program.backend_contract.function_bodies.remove(&id);
    let declaration = program
        .backend_contract
        .callables
        .get_mut(&MirCallableKey::Instance(InstanceId(1)))
        .unwrap();
    declaration.kind = MirCallableKind::ObjectProvided;
    declaration.source_def_id = Some(DefId::new(CrateId(17), LocalDefId(91)));
    declaration.llvm_symbol = "opaque_link_record_symbol".into();
    let context = Context::create();
    let mut codegen = super::CodeGen::new(&context, "external-object-entry");
    codegen.compile_program_from_mir(&program).unwrap();
    codegen.module.verify().unwrap();
    let imported = codegen
        .module
        .get_function("opaque_link_record_symbol")
        .unwrap();
    let engine = codegen
        .module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    engine.add_global_mapping(&imported, external_method as *const () as usize);
    unsafe {
        let run = engine
            .get_function::<unsafe extern "C" fn() -> i64>("run_object")
            .unwrap();
        assert_eq!(run.call(), 46);
    }
}

#[test]
fn virtual_reference_return_executes_and_cannot_escape_local_payload() {
    let mut program = object_program(false);
    let reference = program.type_context.intern_type(&Type::Reference {
        inner: Box::new(Type::I64),
        mutable: false,
    });
    let method_id = MirFunctionId::Instance(InstanceId(1));
    let method = program.functions.get_mut(&method_id).unwrap();
    method.ret_type = reference;
    method.local_decls[0].ty = reference;
    method.basic_blocks[0].statements = vec![StatementData::assign(
        place(0),
        Rvalue::Use(Operand::Copy(place(1))),
        None,
    )];
    let ret = MirReturnAbi {
        semantic_ty: reference,
        abi_ty: reference,
    };
    program
        .backend_contract
        .callables
        .get_mut(&MirCallableKey::Instance(InstanceId(1)))
        .unwrap()
        .signature
        .ret = ret.clone();
    for schema in program.backend_contract.object_schemas.values_mut() {
        for slot in &mut schema.slots {
            if slot.member_id == definition(13) {
                slot.signature.ret = ret.clone();
            }
        }
    }
    let caller_id = MirFunctionId::Instance(InstanceId(0));
    let caller = program.functions.get_mut(&caller_id).unwrap();
    caller
        .local_decls
        .push(local(reference, LocalSource::Temporary));
    let Some(Terminator::Call { destination, .. }) = &mut caller.basic_blocks[0].terminator else {
        unreachable!()
    };
    *destination = place(5);
    caller.basic_blocks[1]
        .statements
        .push(StatementData::assign(
            place(0),
            Rvalue::Use(Operand::Copy(pointee(5))),
            None,
        ));
    crate::mir::borrowck::BorrowChecker::run(&program).unwrap();
    let context = Context::create();
    let mut codegen = super::CodeGen::new(&context, "object-reference-return");
    codegen.compile_program_from_mir(&program).unwrap();
    codegen.module.verify().unwrap();
    let engine = codegen
        .module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    unsafe {
        let run = engine
            .get_function::<unsafe extern "C" fn() -> i64>("run_object")
            .unwrap();
        assert_eq!(run.call(), 41);
    }
    let caller = program.functions.get_mut(&caller_id).unwrap();
    caller.ret_type = reference;
    caller.local_decls[0].ty = reference;
    caller.basic_blocks[1].statements = vec![StatementData::assign(
        place(0),
        Rvalue::Use(Operand::Copy(place(5))),
        None,
    )];
    program
        .backend_contract
        .callables
        .get_mut(&MirCallableKey::Instance(InstanceId(0)))
        .unwrap()
        .signature
        .ret = ret;
    assert!(crate::mir::borrowck::BorrowChecker::run(&program).is_err());
}
