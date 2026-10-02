use super::object_tests::{definition, insert_body, local, object_program, place};
use crate::ids::InstanceId;
use crate::mir::*;
use crate::type_context::TypeContext;
use crate::types::{CallableKind, ObjectType, ReceiverMode, TraitBound, Type};
use inkwell::{context::Context, OptimizationLevel};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

fn adapter(
    program: &mut MirProgram,
    key: MirObjectAdapterKey,
    body: MirObjectAdapter,
    signature: MirCallableSignature,
    symbol: &str,
) -> MirCallableKey {
    let key = MirCallableKey::ObjectAdapter(key);
    program.backend_contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: None,
            kind: MirCallableKind::ObjectAdapter(body),
            llvm_symbol: symbol.into(),
            // Test entry adapters are exported for MCJIT address lookup.
            linkage: MirLinkage::External,
            signature,
        },
    );
    key
}

#[test]
fn borrowed_self_result_reuses_metadata_but_not_receiver_address() {
    static OTHER: i64 = 87;
    unsafe extern "C" fn other(_: *const i64) -> *const i64 {
        &OTHER
    }
    let mut program = object_program(false);
    let source_ref = program.type_context.intern_type(&Type::Reference {
        inner: Box::new(Type::I64),
        mutable: false,
    });
    let target = program.backend_contract.vtables[&MirVtableId(1)].object;
    let target_ref = program.type_context.intern_type(&Type::Reference {
        inner: Box::new(program.type_context.type_for(target)),
        mutable: false,
    });
    let key = MirCallableKey::Instance(InstanceId(3));
    program.backend_contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(definition(14)),
            kind: MirCallableKind::ObjectProvided,
            llvm_symbol: "another_self_link_record".into(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(
                &[source_ref],
                source_ref,
                MirPassMode::Pointer,
            ),
        },
    );
    let mut signature =
        MirCallableSignature::from_type_ids(&[target_ref], target_ref, MirPassMode::FatDirect);
    signature.params[0].pass_mode = MirPassMode::FatDirect;
    program
        .backend_contract
        .object_schemas
        .get_mut(&target)
        .unwrap()
        .slots
        .push(MirObjectSlot {
            trait_id: definition(11),
            member_id: definition(14),
            trait_args: vec![],
            receiver: ReceiverMode::Shared,
            signature,
            result: MirObjectResult::BorrowedSelf { mutable: false },
        });
    program
        .backend_contract
        .vtables
        .get_mut(&MirVtableId(1))
        .unwrap()
        .methods
        .push(key);
    let caller = program
        .functions
        .get_mut(&MirFunctionId::Instance(InstanceId(0)))
        .unwrap();
    caller
        .local_decls
        .push(local(target_ref, LocalSource::Temporary));
    let Some(Terminator::Call {
        func, destination, ..
    }) = &mut caller.basic_blocks[0].terminator
    else {
        unreachable!()
    };
    *func = Operand::Constant(Constant::VirtualTarget(MirVirtualTarget {
        object: target,
        slot: 1,
    }));
    *destination = place(5);
    caller.basic_blocks[1].terminator = Some(Terminator::Call {
        func: Operand::Constant(Constant::VirtualTarget(MirVirtualTarget {
            object: target,
            slot: 0,
        })),
        args: vec![Operand::Copy(place(5))],
        destination: place(0),
        target: BasicBlockId(2),
        span: None,
    });
    caller.basic_blocks.push(BasicBlock {
        statements: vec![],
        terminator: Some(Terminator::Return),
    });
    assert!(validate_object_contract(&program).is_empty());
    let context = Context::create();
    let mut codegen = super::CodeGen::new(&context, "borrowed-self");
    codegen.compile_program_from_mir(&program).unwrap();
    codegen.module.verify().unwrap();
    let function = codegen
        .module
        .get_function("another_self_link_record")
        .unwrap();
    let engine = codegen
        .module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    engine.add_global_mapping(&function, other as *const () as usize);
    unsafe {
        assert_eq!(
            engine
                .get_function::<unsafe extern "C" fn() -> i64>("run_object")
                .unwrap()
                .call(),
            87
        );
    }
    let slot = program
        .backend_contract
        .object_schemas
        .get_mut(&target)
        .unwrap()
        .slots
        .last_mut()
        .unwrap();
    slot.result = MirObjectResult::BorrowedSelf { mutable: true };
    assert!(
        !validate_object_contract(&program).is_empty(),
        "shared receiver must not gain mutable permission"
    );
}

fn native_callable_program(mutable: bool) -> MirProgram {
    let mut tc = TypeContext::new();
    let scalar = tc.intern_type(&Type::I64);
    let unit = tc.intern_type(&Type::Unit);
    let concrete_ty = Type::function(vec![], Type::I64);
    let concrete = tc.intern_type(&concrete_ty);
    let source_ref = tc.intern_type(&Type::Reference {
        inner: Box::new(concrete_ty.clone()),
        mutable,
    });
    let source_owned = tc.intern_type(&Type::Pointer(Box::new(concrete_ty)));
    let mut object = ObjectType::new(TraitBound {
        trait_id: definition(20),
        type_args: vec![Type::Unit, Type::I64],
    });
    object.guarantees.insert(TraitBound {
        trait_id: definition(21),
        type_args: vec![Type::Unit, Type::I64],
    });
    let object_ty = Type::Object(Box::new(object));
    let object = tc.intern_type(&object_ty);
    let reference = tc.intern_type(&Type::Reference {
        inner: Box::new(object_ty.clone()),
        mutable,
    });
    let owned = tc.intern_type(&Type::Pointer(Box::new(object_ty)));
    let mut program = MirProgram {
        functions: BTreeMap::new(),
        type_context: tc,
        backend_contract: MirBackendContract::default(),
    };
    let mut slots = Vec::new();
    let mut methods = Vec::new();
    for (kind, trait_id, member_id, receiver_ty, object_receiver, receiver) in [
        (
            if mutable {
                CallableKind::FnMut
            } else {
                CallableKind::Fn
            },
            definition(20),
            definition(22),
            source_ref,
            reference,
            if mutable {
                ReceiverMode::Mut
            } else {
                ReceiverMode::Shared
            },
        ),
        (
            CallableKind::FnOnce,
            definition(21),
            definition(23),
            source_owned,
            owned,
            ReceiverMode::Move,
        ),
    ] {
        let mut signature = MirCallableSignature::from_type_ids(
            &[object_receiver, unit],
            scalar,
            MirPassMode::Direct,
        );
        signature.params[0].pass_mode = if receiver == ReceiverMode::Move {
            MirPassMode::Pointer
        } else {
            MirPassMode::FatDirect
        };
        slots.push(MirObjectSlot {
            trait_id,
            member_id,
            trait_args: vec![unit, scalar],
            receiver,
            signature: signature.clone(),
            result: MirObjectResult::Direct,
        });
        signature.params[0] = MirParamAbi {
            semantic_ty: receiver_ty,
            pass_mode: MirPassMode::Pointer,
        };
        methods.push(adapter(
            &mut program,
            MirObjectAdapterKey::NativeCallable { concrete, kind },
            MirObjectAdapter::NativeCallable {
                concrete,
                kind,
                trait_id,
                member_id,
                release_environment: false,
            },
            signature,
            match kind {
                CallableKind::Fn => "native.shared",
                CallableKind::FnMut => "native.mutable",
                CallableKind::FnOnce => "native.owned",
            },
        ));
    }
    program.backend_contract.object_schemas.insert(
        object,
        MirObjectSchema {
            erased_slots: vec![],
            object,
            slots,
            views: vec![],
        },
    );
    program.backend_contract.vtables.insert(
        MirVtableId(0),
        MirVtable {
            erased_methods: vec![],
            object,
            concrete,
            methods,
            views: vec![],
            drop: None,
        },
    );
    insert_body(
        &mut program,
        MirFunction {
            id: MirFunctionId::Instance(InstanceId(7)),
            name: "callback".into(),
            local_decls: vec![local(scalar, LocalSource::ReturnPlace)],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: scalar,
            ownership: Default::default(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    place(0),
                    Rvalue::Use(Operand::Constant(Constant::Int(55))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
        },
        "callback",
        &[],
        false,
    );
    insert_body(
        &mut program,
        MirFunction {
            id: MirFunctionId::Instance(InstanceId(0)),
            name: "run_native_object".into(),
            local_decls: vec![
                local(scalar, LocalSource::ReturnPlace),
                local(concrete, LocalSource::UserBinding),
                local(source_ref, LocalSource::Temporary),
                local(reference, LocalSource::UserBinding),
                local(unit, LocalSource::Temporary),
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: scalar,
            ownership: Default::default(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![
                        StatementData::assign(
                            place(1),
                            Rvalue::Use(Operand::Constant(Constant::Callable(
                                MirCallable::Resolved(MirCallableKey::Instance(InstanceId(7))),
                            ))),
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
                            Rvalue::Use(Operand::Constant(Constant::Unit)),
                            None,
                        ),
                    ],
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::VirtualTarget(MirVirtualTarget {
                            object,
                            slot: 0,
                        })),
                        args: vec![Operand::Move(place(3)), Operand::Copy(place(4))],
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
        "run_native_object",
        &[],
        false,
    );
    program
}

#[test]
fn native_callable_object_executes_without_granting_consuming_supertrait_access() {
    for mutable in [false, true] {
        let mut program = native_callable_program(mutable);
        assert!(
            validate_object_contract(&program).is_empty(),
            "{:?}",
            validate_object_contract(&program)
        );
        let context = Context::create();
        let mut codegen = super::CodeGen::new(&context, "native-callable-object");
        codegen.compile_program_from_mir(&program).unwrap();
        codegen.module.verify().unwrap();
        let engine = codegen
            .module
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        unsafe {
            assert_eq!(
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("run_native_object")
                    .unwrap()
                    .call(),
                55
            );
        }
        let caller = program
            .functions
            .get_mut(&MirFunctionId::Instance(InstanceId(0)))
            .unwrap();
        let Some(Terminator::Call {
            func: Operand::Constant(Constant::VirtualTarget(selector)),
            ..
        }) = &mut caller.basic_blocks[0].terminator
        else {
            unreachable!()
        };
        selector.slot = 1;
        assert!(validate_object_contract(&program)
            .iter()
            .any(|error| matches!(error, MirObjectError::InvalidCall(_))));
    }
}

fn payload_program(payload_ty: Type) -> (MirProgram, MirCallableKey) {
    let mut tc = TypeContext::new();
    let scalar = tc.intern_type(&Type::I64);
    let unit = tc.intern_type(&Type::Unit);
    let payload = tc.intern_type(&payload_ty);
    let payload_ref = tc.intern_type(&Type::Reference {
        inner: Box::new(payload_ty.clone()),
        mutable: true,
    });
    let object = tc.intern_type(&Type::Object(Box::new(ObjectType::new(TraitBound {
        trait_id: definition(30),
        type_args: vec![],
    }))));
    let mut program = MirProgram {
        functions: BTreeMap::new(),
        type_context: tc,
        backend_contract: MirBackendContract::default(),
    };
    let user_drop = MirCallableKey::Function(definition(31));
    program.backend_contract.callables.insert(
        user_drop.clone(),
        MirCallableDecl {
            key: user_drop.clone(),
            source_def_id: Some(definition(31)),
            kind: MirCallableKind::ObjectProvided,
            llvm_symbol: "payload_user_drop_link".into(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[scalar], unit, MirPassMode::Direct),
        },
    );
    program.backend_contract.drop_glue.insert(scalar, user_drop);
    if let Type::Enum { id, .. } = &payload_ty {
        program.backend_contract.nominal_layouts.insert(
            *id,
            MirNominalLayout::Enum {
                id: *id,
                generic_params: vec![],
                variants: vec![
                    MirEnumVariantLayout {
                        name: "First".into(),
                        fields: MirVariantLayoutFields::Positional(vec![scalar]),
                    },
                    MirEnumVariantLayout {
                        name: "Second".into(),
                        fields: MirVariantLayoutFields::Positional(vec![scalar]),
                    },
                ],
            },
        );
    }
    if let Type::Function { captures, .. } = &payload_ty {
        for capture in captures {
            if let Type::Struct { id, .. } = &capture.ty {
                program.backend_contract.nominal_layouts.insert(
                    *id,
                    MirNominalLayout::Struct {
                        id: *id,
                        generic_params: vec![],
                        fields: vec![("resource".into(), scalar)],
                    },
                );
            }
        }
    }
    let plan = payload_drop_plan(
        &program.backend_contract,
        &mut program.type_context,
        payload,
    )
    .unwrap();
    let key = adapter(
        &mut program,
        MirObjectAdapterKey::PayloadDrop(payload),
        MirObjectAdapter::PayloadDrop(plan),
        MirCallableSignature::from_type_ids(&[payload_ref], unit, MirPassMode::Pointer),
        "drop.complete.payload",
    );
    program.backend_contract.object_schemas.insert(
        object,
        MirObjectSchema {
            erased_slots: vec![],
            object,
            slots: vec![],
            views: vec![],
        },
    );
    program.backend_contract.vtables.insert(
        MirVtableId(0),
        MirVtable {
            erased_methods: vec![],
            object,
            concrete: payload,
            methods: vec![],
            views: vec![],
            drop: Some(key.clone()),
        },
    );
    program.backend_contract.runtime_requirements =
        crate::mir::backend_contract::runtime_requirements_for_backend(
            &program.backend_contract,
            program.functions.values(),
        );
    (program, key)
}

#[test]
fn payload_drop_adapter_destroys_structural_fields_and_rejects_partial_plan() {
    static ORDER: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" fn drop_scalar(value: i64) {
        ORDER.store(
            ORDER.load(Ordering::SeqCst) * 10 + value as u64,
            Ordering::SeqCst,
        );
    }
    let (mut program, key) = payload_program(Type::Tuple(vec![Type::I64, Type::I64]));
    let context = Context::create();
    let mut codegen = super::CodeGen::new(&context, "structural-payload-drop");
    codegen.compile_program_from_mir(&program).unwrap();
    codegen.module.verify().unwrap();
    let external = codegen
        .module
        .get_function("payload_user_drop_link")
        .unwrap();
    let engine = codegen
        .module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    engine.add_global_mapping(&external, drop_scalar as *const () as usize);
    ORDER.store(0, Ordering::SeqCst);
    let mut fields = [1i64, 2];
    unsafe {
        engine
            .get_function::<unsafe extern "C" fn(*mut i64)>("drop.complete.payload")
            .unwrap()
            .call(fields.as_mut_ptr());
    }
    assert_eq!(ORDER.load(Ordering::SeqCst), 21);
    let MirCallableKind::ObjectAdapter(MirObjectAdapter::PayloadDrop(plan)) = &mut program
        .backend_contract
        .callables
        .get_mut(&key)
        .unwrap()
        .kind
    else {
        unreachable!()
    };
    plan.children = MirPayloadDropChildren::None;
    assert!(validate_object_contract(&program)
        .iter()
        .any(|error| matches!(error, MirObjectError::InvalidAdapter(_))));
}

#[test]
fn payload_drop_adapter_visits_array_elements_and_only_active_enum_variant() {
    static ORDER: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" fn record(value: i64) {
        ORDER.store(
            ORDER.load(Ordering::SeqCst) * 10 + value as u64,
            Ordering::SeqCst,
        );
    }
    #[repr(C)]
    struct EnumStorage {
        tag: u32,
        first: i64,
        second: i64,
    }
    for payload in [
        Type::Array(Box::new(Type::I64), 3),
        Type::Enum {
            id: definition(40),
            args: vec![],
        },
    ] {
        let enumeration = matches!(payload, Type::Enum { .. });
        let (program, _) = payload_program(payload);
        let context = Context::create();
        let mut codegen = super::CodeGen::new(&context, "drop-aggregate");
        codegen.compile_program_from_mir(&program).unwrap();
        codegen.module.verify().unwrap();
        let external = codegen
            .module
            .get_function("payload_user_drop_link")
            .unwrap();
        let engine = codegen
            .module
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        engine.add_global_mapping(&external, record as *const () as usize);
        ORDER.store(0, Ordering::SeqCst);
        unsafe {
            let drop = engine
                .get_function::<unsafe extern "C" fn(*mut u8)>("drop.complete.payload")
                .unwrap();
            if enumeration {
                let mut value = EnumStorage {
                    tag: 1,
                    first: 5,
                    second: 7,
                };
                drop.call((&mut value as *mut EnumStorage).cast());
                assert_eq!(ORDER.load(Ordering::SeqCst), 7);
            } else {
                let mut value = [1i64, 2, 3];
                drop.call(value.as_mut_ptr().cast());
                assert_eq!(ORDER.load(Ordering::SeqCst), 321);
            }
        }
    }
}

#[test]
fn never_virtual_call_has_void_abi_and_unreachable_continuation() {
    let mut program = object_program(false);
    let never = program.type_context.intern_type(&Type::Never);
    let function = program
        .functions
        .get_mut(&MirFunctionId::Instance(InstanceId(1)))
        .unwrap();
    function.ret_type = never;
    function.local_decls[0].ty = never;
    function.basic_blocks = vec![
        BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Goto(BasicBlockId(1))),
        },
        BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Goto(BasicBlockId(1))),
        },
    ];
    program
        .backend_contract
        .callables
        .get_mut(&MirCallableKey::Instance(InstanceId(1)))
        .unwrap()
        .signature
        .ret = MirReturnAbi {
        semantic_ty: never,
        abi_ty: never,
    };
    for schema in program.backend_contract.object_schemas.values_mut() {
        for slot in &mut schema.slots {
            if slot.member_id == definition(13) {
                slot.signature.ret = MirReturnAbi {
                    semantic_ty: never,
                    abi_ty: never,
                };
            }
        }
    }
    program
        .functions
        .get_mut(&MirFunctionId::Instance(InstanceId(0)))
        .unwrap()
        .basic_blocks[1]
        .terminator = Some(Terminator::Unreachable {
        origin: MirOrigin::synthetic(MirSyntheticOrigin::ControlFlow, None),
    });
    let context = Context::create();
    let mut codegen = super::CodeGen::new(&context, "never-object");
    codegen.compile_program_from_mir(&program).unwrap();
    codegen.module.verify().unwrap();
    assert!(codegen
        .module
        .get_function("read_or_increment")
        .unwrap()
        .get_type()
        .get_return_type()
        .is_none());
    assert!(codegen.get_ir().contains("unreachable"));
}

#[test]
fn payload_drop_distinguishes_owned_and_borrowed_closure_captures() {
    static DROPPED: AtomicU64 = AtomicU64::new(0);
    static RELEASED: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" {
        fn malloc(size: usize) -> *mut std::ffi::c_void;
        fn free(pointer: *mut std::ffi::c_void);
    }
    unsafe extern "C" fn drop_capture(value: i64) {
        DROPPED.store(value as u64, Ordering::SeqCst);
    }
    unsafe extern "C" fn release(pointer: *mut u8) {
        RELEASED.fetch_add(1, Ordering::SeqCst);
        free(pointer.cast());
    }
    unsafe extern "C" fn body(_: *mut u8) {}
    #[repr(C)]
    struct ClosureStorage {
        code: *const (),
        environment: *mut u8,
    }
    for borrowed in [false, true] {
        let resource = if borrowed {
            Type::I64
        } else {
            Type::Struct {
                id: definition(50),
                args: vec![],
            }
        };
        let payload = Type::Function {
            params: vec![],
            ret: Box::new(Type::Unit),
            safety: crate::types::FunctionSafety::Safe,
            callable_kind: if borrowed {
                CallableKind::FnMut
            } else {
                CallableKind::FnOnce
            },
            captures: vec![crate::types::FunctionCapture::new(
                if borrowed {
                    crate::types::CaptureKind::MutableBorrow
                } else {
                    crate::types::CaptureKind::Move
                },
                resource,
            )],
        };
        let (mut program, _) = payload_program(payload);
        assert!(program
            .backend_contract
            .runtime_requirements
            .contains(&MirRuntimeHelper::HeapFree));
        let context = Context::create();
        let mut codegen = super::CodeGen::new(&context, "owned-closure-drop");
        codegen.compile_program_from_mir(&program).unwrap();
        codegen.module.verify().unwrap();
        assert!(!codegen.get_ir().contains("@malloc"));
        let destructor = codegen
            .module
            .get_function("payload_user_drop_link")
            .unwrap();
        let deallocator = codegen.module.get_function("free").unwrap();
        let engine = codegen
            .module
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        engine.add_global_mapping(&destructor, drop_capture as *const () as usize);
        engine.add_global_mapping(&deallocator, release as *const () as usize);
        DROPPED.store(0, Ordering::SeqCst);
        RELEASED.store(0, Ordering::SeqCst);
        let mut referent = 42i64;
        unsafe {
            let environment = malloc(if borrowed {
                std::mem::size_of::<*mut i64>()
            } else {
                std::mem::size_of::<i64>()
            })
            .cast::<u8>();
            assert!(!environment.is_null());
            if borrowed {
                environment.cast::<*mut i64>().write(&mut referent);
            } else {
                environment.cast::<i64>().write(42);
            }
            let mut closure = ClosureStorage {
                code: body as *const (),
                environment,
            };
            engine
                .get_function::<unsafe extern "C" fn(*mut ClosureStorage)>("drop.complete.payload")
                .unwrap()
                .call(&mut closure);
        }
        assert_eq!(
            DROPPED.load(Ordering::SeqCst),
            if borrowed { 0 } else { 42 }
        );
        assert_eq!(referent, 42);
        assert_eq!(RELEASED.load(Ordering::SeqCst), 1);
        program.backend_contract.runtime_requirements.clear();
        let context = Context::create();
        let mut rejected = super::CodeGen::new(&context, "missing-free-contract");
        assert!(rejected.compile_program_from_mir(&program).is_err());
    }
}

#[test]
fn consuming_entry_adapter_obeys_concrete_direct_and_indirect_value_abis() {
    static RELEASED: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" fn release(pointer: *const i64, state: i64) {
        RELEASED.fetch_add(
            if *pointer == 42 { state as u64 } else { 1000 },
            Ordering::SeqCst,
        );
    }
    for mode in [MirPassMode::Direct, MirPassMode::Pointer] {
        let mut tc = TypeContext::new();
        let scalar = tc.intern_type(&Type::I64);
        let pointer = tc.intern_type(&Type::Pointer(Box::new(Type::I64)));
        let object_ty = Type::Object(Box::new(ObjectType::new(TraitBound {
            trait_id: definition(60),
            type_args: vec![],
        })));
        let object = tc.intern_type(&object_ty);
        let erased = tc.intern_type(&Type::Pointer(Box::new(object_ty)));
        let mut program = MirProgram {
            functions: BTreeMap::new(),
            type_context: tc,
            backend_contract: MirBackendContract::default(),
        };
        insert_body(
            &mut program,
            MirFunction {
                id: MirFunctionId::Instance(InstanceId(8)),
                name: "consume_value".into(),
                local_decls: vec![
                    local(scalar, LocalSource::ReturnPlace),
                    local(scalar, LocalSource::Argument),
                ],
                arg_count: 1,
                ret_type: scalar,
                closure_captures: vec![],
                ownership: Default::default(),
                basic_blocks: vec![BasicBlock {
                    statements: vec![StatementData::assign(
                        place(0),
                        Rvalue::BinaryOp(
                            MirBinOp::Add,
                            Operand::Move(place(1)),
                            Operand::Constant(Constant::Int(1)),
                        ),
                        None,
                    )],
                    terminator: Some(Terminator::Return),
                }],
            },
            "consume_value",
            &[scalar],
            true,
        );
        program
            .backend_contract
            .callables
            .get_mut(&MirCallableKey::Instance(InstanceId(8)))
            .unwrap()
            .signature
            .params[0]
            .pass_mode = mode;
        let signature =
            MirCallableSignature::from_type_ids(&[pointer], scalar, MirPassMode::Pointer);
        let key = adapter(
            &mut program,
            MirObjectAdapterKey::ConsumingMethod {
                concrete: scalar,
                target: InstanceId(8),
            },
            MirObjectAdapter::ConsumingMethod {
                concrete: scalar,
                target: MirCallableKey::Instance(InstanceId(8)),
            },
            signature,
            "consume.entry",
        );
        program.backend_contract.object_schemas.insert(
            object,
            MirObjectSchema {
                erased_slots: vec![],
                object,
                slots: vec![MirObjectSlot {
                    trait_id: definition(60),
                    member_id: definition(61),
                    trait_args: vec![],
                    receiver: ReceiverMode::Move,
                    signature: MirCallableSignature::from_type_ids(
                        &[erased],
                        scalar,
                        MirPassMode::Pointer,
                    ),
                    result: MirObjectResult::Direct,
                }],
                views: vec![],
            },
        );
        program.backend_contract.vtables.insert(
            MirVtableId(0),
            MirVtable {
                erased_methods: vec![],
                object,
                concrete: scalar,
                methods: vec![key],
                views: vec![],
                drop: None,
            },
        );
        add_owned_consuming_caller(&mut program, scalar, pointer, object, erased);
        let context = Context::create();
        let mut codegen = super::CodeGen::new(&context, "consume-adapter");
        codegen.compile_program_from_mir(&program).unwrap();
        codegen.module.verify().unwrap();
        let engine = codegen
            .module
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        let mut owned = 42i64;
        RELEASED.store(0, Ordering::SeqCst);
        engine.add_global_mapping(
            &codegen.module.get_function("release.owner").unwrap(),
            release as usize,
        );
        unsafe {
            assert_eq!(
                engine
                    .get_function::<unsafe extern "C" fn(*mut i64) -> i64>("consume.entry")
                    .unwrap()
                    .call(&mut owned),
                43
            );
            assert_eq!(
                engine
                    .get_function::<unsafe extern "C" fn(*mut i64) -> i64>("owned.caller")
                    .unwrap()
                    .call(&mut owned),
                43
            );
        }
        assert_eq!(RELEASED.load(Ordering::SeqCst), 73);
        let mut invalid = program.clone();
        let caller = invalid
            .functions
            .get_mut(&MirFunctionId::Instance(InstanceId(91)))
            .unwrap();
        let Some(Terminator::Call { args, .. }) = &mut caller.basic_blocks[0].terminator else {
            unreachable!()
        };
        args[0] = Operand::Copy(place(3));
        assert!(super::CodeGen::new(&context, "invalid-owned-copy")
            .compile_program_from_mir(&invalid)
            .is_err());
    }
}

fn add_owned_consuming_caller(
    program: &mut MirProgram,
    scalar: crate::ids::TypeId,
    pointer: crate::ids::TypeId,
    object: crate::ids::TypeId,
    erased: crate::ids::TypeId,
) {
    let owner_id = definition(90);
    let owner = program.type_context.intern_type(&Type::Struct {
        id: owner_id,
        args: vec![],
    });
    let parts = program.type_context.intern_type(&Type::Tuple(vec![
        program.type_context.type_for(erased),
        Type::I64,
    ]));
    let unit = program.type_context.intern_type(&Type::Unit);
    let bytes = program
        .type_context
        .intern_type(&Type::Pointer(Box::new(Type::U8)));
    program.backend_contract.nominal_layouts.insert(
        owner_id,
        MirNominalLayout::Struct {
            id: owner_id,
            fields: vec![("handle".into(), erased), ("state".into(), scalar)],
            generic_params: vec![],
        },
    );
    let field = |index| Place {
        local: Local(1),
        projection: vec![Projection::Field {
            index,
            identity: None,
        }],
    };
    insert_body(
        program,
        MirFunction {
            id: MirFunctionId::Instance(InstanceId(90)),
            name: "split.owner".into(),
            arg_count: 1,
            ret_type: parts,
            local_decls: vec![
                local(parts, LocalSource::ReturnPlace),
                local(owner, LocalSource::Argument),
            ],
            closure_captures: vec![],
            ownership: Default::default(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    place(0),
                    Rvalue::Aggregate(
                        AggregateKind::Tuple,
                        vec![Operand::Copy(field(0)), Operand::Move(field(1))],
                    ),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
        },
        "split.owner",
        &[owner],
        false,
    );
    let release = MirCallableKey::Instance(InstanceId(92));
    program.backend_contract.callables.insert(
        release.clone(),
        MirCallableDecl {
            key: release.clone(),
            source_def_id: Some(definition(92)),
            kind: MirCallableKind::ObjectProvided,
            llvm_symbol: "release.owner".into(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(
                &[bytes, scalar],
                unit,
                MirPassMode::Direct,
            ),
        },
    );
    let key = MirOwnedObjectKey {
        owner,
        object,
        slot: MirOwnedObjectSlot::Concrete(0),
    };
    program.backend_contract.owned_object_calls.insert(
        key.clone(),
        MirOwnedObjectPlan {
            state: scalar,
            into_parts: MirCallableKey::Instance(InstanceId(90)),
            release,
            dictionaries: vec![],
        },
    );
    insert_body(
        program,
        MirFunction {
            id: MirFunctionId::Instance(InstanceId(91)),
            name: "owned.caller".into(),
            arg_count: 1,
            ret_type: scalar,
            local_decls: vec![
                local(scalar, LocalSource::ReturnPlace),
                local(pointer, LocalSource::Argument),
                local(erased, LocalSource::Temporary),
                local(owner, LocalSource::Temporary),
            ],
            closure_captures: vec![],
            ownership: Default::default(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![
                        StatementData::assign(
                            place(2),
                            Rvalue::Object(
                                Operand::Copy(place(1)),
                                MirObjectConversion::Concrete(MirVtableId(0)),
                            ),
                            None,
                        ),
                        StatementData::assign(
                            place(3),
                            Rvalue::Aggregate(
                                AggregateKind::Struct {
                                    id: owner_id,
                                    display_name: "Owner".into(),
                                },
                                vec![
                                    Operand::Copy(place(2)),
                                    Operand::Constant(Constant::Int(73)),
                                ],
                            ),
                            None,
                        ),
                    ],
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::OwnedObjectCall(key)),
                        args: vec![Operand::Move(place(3))],
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
        "owned.caller",
        &[pointer],
        false,
    );
}

#[test]
fn owned_erased_call_transfers_payload_and_initializes_result_before_release() {
    static RELEASED: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" fn release(_: *mut u8, state: i64) {
        RELEASED.fetch_add(state as u64, Ordering::SeqCst);
    }
    let mut tc = TypeContext::new();
    let scalar = tc.intern_type(&Type::I64);
    let pointer = tc.intern_type(&Type::Pointer(Box::new(Type::I64)));
    let object_ty = Type::Object(Box::new(ObjectType::new(TraitBound {
        trait_id: definition(60),
        type_args: vec![],
    })));
    let object = tc.intern_type(&object_ty);
    let raw = tc.intern_type(&Type::Pointer(Box::new(object_ty)));
    let parameter = MirErasedType::Parameter(1);
    let parameters = (0..2)
        .map(|index| MirErasedParameter {
            source: crate::types::GenericParamId {
                owner: definition(61),
                index,
            },
            kind: crate::type_services::kind::Kind::Type,
        })
        .collect();
    let signature = MirErasedSignature {
        parameters,
        params: vec![MirErasedType::Parameter(0), parameter.clone()],
        ret: parameter.clone(),
        dictionaries: vec![],
        layouts: vec![MirErasedType::Parameter(0), parameter.clone()],
        require_borrow_free: Default::default(),
    };
    let key = MirErasedKey {
        definition: definition(62),
        static_args: vec![scalar],
    };
    let mut entry_signature = signature.clone();
    entry_signature.params[0] = MirErasedType::Concrete(scalar);
    let entry = MirErasedFunction {
        key: key.clone(),
        symbol: "owned.generic.entry".into(),
        linkage: MirLinkage::Internal,
        signature: entry_signature,
        body: Some(MirErasedBody {
            locals: vec![MirErasedType::Concrete(scalar), parameter],
            mutable_locals: Default::default(),
            statements: vec![],
            result: MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(1))),
        }),
    };
    let mut program = MirProgram {
        functions: BTreeMap::new(),
        type_context: tc,
        backend_contract: MirBackendContract::default(),
    };
    program.backend_contract.object_schemas.insert(
        object,
        MirObjectSchema {
            object,
            slots: vec![],
            erased_slots: vec![MirErasedObjectSlot {
                trait_id: definition(60),
                member_id: definition(61),
                trait_args: vec![],
                receiver: ReceiverMode::Move,
                signature,
                result: MirObjectResult::Direct,
            }],
            views: vec![],
        },
    );
    program
        .backend_contract
        .erased
        .functions
        .insert(key.clone(), entry);
    let drop =
        payload_drop_plan(&program.backend_contract, &mut program.type_context, scalar).unwrap();
    program.backend_contract.erased.descriptors.insert(
        scalar,
        MirTypeDescriptor {
            ty: scalar,
            drop,
            fields: vec![],
        },
    );
    program.backend_contract.vtables.insert(
        MirVtableId(0),
        MirVtable {
            object,
            concrete: scalar,
            methods: vec![],
            erased_methods: vec![key],
            views: vec![],
            drop: None,
        },
    );
    add_owned_consuming_caller(&mut program, scalar, pointer, object, raw);
    let concrete = program
        .backend_contract
        .owned_object_calls
        .keys()
        .next()
        .unwrap()
        .clone();
    let plan = program
        .backend_contract
        .owned_object_calls
        .remove(&concrete)
        .unwrap();
    let mut owned = concrete;
    owned.slot = MirOwnedObjectSlot::Erased {
        slot: 0,
        type_args: vec![scalar],
    };
    program
        .backend_contract
        .owned_object_calls
        .insert(owned.clone(), plan);
    let caller = program
        .functions
        .get_mut(&MirFunctionId::Instance(InstanceId(91)))
        .unwrap();
    caller
        .local_decls
        .push(local(scalar, LocalSource::Temporary));
    caller.basic_blocks[0]
        .statements
        .push(StatementData::assign(
            place(4),
            Rvalue::Use(Operand::Constant(Constant::Int(42))),
            None,
        ));
    let Some(Terminator::Call { func, args, .. }) = &mut caller.basic_blocks[0].terminator else {
        unreachable!()
    };
    *func = Operand::Constant(Constant::OwnedObjectCall(owned.clone()));
    args.push(Operand::Copy(place(4)));
    assert!(
        validate_object_contract(&program).is_empty(),
        "{:?}",
        validate_object_contract(&program)
    );
    assert!(
        validate_erased_contract(&program).is_empty(),
        "{:?}",
        validate_erased_contract(&program)
    );
    let context = Context::create();
    let mut codegen = super::CodeGen::new(&context, "owned-erased");
    codegen.compile_program_from_mir(&program).unwrap();
    codegen.module.verify().unwrap();
    let engine = codegen
        .module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    engine.add_global_mapping(
        &codegen.module.get_function("release.owner").unwrap(),
        release as *const () as usize,
    );
    RELEASED.store(0, Ordering::SeqCst);
    let mut payload = 123i64;
    unsafe {
        assert_eq!(
            engine
                .get_function::<unsafe extern "C" fn(*mut i64) -> i64>("owned.caller")
                .unwrap()
                .call(&mut payload),
            42
        );
    }
    assert_eq!(RELEASED.load(Ordering::SeqCst), 73);
    for raw_selector in [false, true] {
        let mut invalid = program.clone();
        let caller = invalid
            .functions
            .get_mut(&MirFunctionId::Instance(InstanceId(91)))
            .unwrap();
        let Some(Terminator::Call { func, args, .. }) = &mut caller.basic_blocks[0].terminator
        else {
            unreachable!()
        };
        if raw_selector {
            *func = Operand::Constant(Constant::ErasedCall(MirErasedCall {
                target: MirErasedCallTarget::Virtual(MirVirtualTarget { object, slot: 0 }),
                type_args: vec![scalar],
                dictionaries: vec![],
            }));
            args[0] = Operand::Copy(place(2));
        } else {
            args[0] = Operand::Copy(place(3));
        }
        assert!(super::CodeGen::new(&context, "invalid-owned-erased")
            .compile_program_from_mir(&invalid)
            .is_err());
    }
    let mut invalid = program;
    let mut bad = owned;
    bad.slot = MirOwnedObjectSlot::Erased {
        slot: 0,
        type_args: vec![],
    };
    let plan = invalid
        .backend_contract
        .owned_object_calls
        .values()
        .next()
        .unwrap()
        .clone();
    invalid
        .backend_contract
        .owned_object_calls
        .insert(bad, plan);
    assert!(validate_object_contract(&invalid)
        .iter()
        .any(|error| matches!(error, MirObjectError::InvalidOwnedPlan(_))));
}
