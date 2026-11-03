use super::*;
use crate::ids::{CrateId, DefId, InstanceId, LocalDefId, TypeId};
use crate::mir::{
    BasicBlock, BasicBlockId, Constant, Local, LocalDecl, MirAssert, MirAssertKind, MirCallable,
    MirFunction, MirFunctionId, Mutability, Operand, Place, StatementData, Terminator,
};

fn def_id(local: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(local))
}

#[test]
fn contract_declares_callable_by_exact_key() {
    let key = MirCallableKey::Function(def_id(1));
    let declaration = MirCallableDecl {
        key: key.clone(),
        source_def_id: Some(def_id(1)),
        kind: MirCallableKind::LocalBody {
            function_id: MirFunctionId::Function(def_id(1)),
        },
        llvm_symbol: "main".to_string(),
        linkage: MirLinkage::External,
        signature: MirCallableSignature::from_type_ids(
            &[TypeId(1)],
            TypeId(2),
            MirPassMode::Direct,
        ),
    };
    let mut contract = MirBackendContract::default();
    contract.callables.insert(key.clone(), declaration);

    assert_eq!(contract.callable(&key).map(|decl| &decl.key), Some(&key));
    assert!(contract
        .callable(&MirCallableKey::Function(def_id(2)))
        .is_none());
}

#[test]
fn runtime_requirements_ignore_malformed_bounds_checks() {
    let function = MirFunction {
        id: MirFunctionId::Function(def_id(100)),
        name: "malformed_bounds".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![Operand::Constant(Constant::Int(0))],
                },
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: TypeId(0),
        ownership: Default::default(),
    };

    assert!(runtime_requirements_for_functions([&function]).is_empty());
}

#[test]
fn runtime_requirements_scan_later_blocks() {
    let function = MirFunction {
        id: MirFunctionId::Function(def_id(101)),
        name: "later_bounds".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: vec![],
                terminator: Some(Terminator::Goto(BasicBlockId(1))),
            },
            BasicBlock {
                statements: vec![StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![
                            Operand::Constant(Constant::Int(0)),
                            Operand::Constant(Constant::Int(0)),
                        ],
                    },
                    None,
                )],
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: TypeId(0),
        ownership: Default::default(),
    };

    assert_eq!(
        runtime_requirements_for_functions([&function]),
        BTreeSet::from([MirRuntimeHelper::BoundsCheck])
    );
}

#[test]
fn validation_reports_callable_map_key_mismatch() {
    let map_key = MirCallableKey::Function(def_id(1));
    let declaration_key = MirCallableKey::Function(def_id(2));
    let mut contract = MirBackendContract::default();
    contract.callables.insert(
        map_key.clone(),
        MirCallableDecl {
            key: declaration_key.clone(),
            source_def_id: Some(def_id(2)),
            kind: MirCallableKind::Extern {
                link_name: "ffi".to_string(),
                variadic: false,
            },
            llvm_symbol: "ffi".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
        },
    );

    assert_eq!(
        validate_backend_contract(&contract),
        vec![MirBackendContractError::CallableKeyMismatch {
            map_key,
            declaration_key,
        }],
    );
}

#[test]
fn validation_reports_local_callable_without_body() {
    let key = MirCallableKey::Function(def_id(1));
    let function = MirFunctionId::Function(def_id(1));
    let declaration = MirCallableDecl {
        key: key.clone(),
        source_def_id: Some(def_id(1)),
        kind: MirCallableKind::LocalBody {
            function_id: function.clone(),
        },
        llvm_symbol: "main".to_string(),
        linkage: MirLinkage::External,
        signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
    };
    let mut contract = MirBackendContract::default();
    contract.callables.insert(key.clone(), declaration);
    contract
        .function_bodies
        .insert(function.clone(), key.clone());
    contract.function_bodies.remove(&function);

    assert_eq!(
        validate_backend_contract(&contract),
        vec![MirBackendContractError::CallableWithoutBody { key, function }],
    );
}

#[test]
fn method_receiver_abi_is_part_of_callable_signature() {
    let mut type_context = TypeContext::new();
    let receiver = type_context.intern_type(&Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    });
    let ret = type_context.intern_type(&Type::Unit);
    let signature =
        MirCallableSignature::from_instance_type_ids(&[receiver], ret, true, &type_context);

    assert_eq!(signature.params[0].semantic_ty, receiver);
    assert_eq!(signature.params[0].pass_mode, MirPassMode::Pointer);
}

#[test]
fn contract_intrinsic_identity_is_typed_not_string_backed() {
    fn callable_key_intrinsic_ctor(_: fn(MirIntrinsicId) -> MirCallableKey) {}
    fn callable_kind_intrinsic_ctor(_: fn(MirIntrinsicId) -> MirCallableKind) {}

    callable_key_intrinsic_ctor(MirCallableKey::Intrinsic);
    callable_kind_intrinsic_ctor(MirCallableKind::Intrinsic);
}

#[test]
fn validation_reports_local_body_missing_from_program_functions() {
    let key = MirCallableKey::Function(def_id(1));
    let function = MirFunctionId::Function(def_id(1));
    let mut contract = MirBackendContract::default();
    contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(def_id(1)),
            kind: MirCallableKind::LocalBody {
                function_id: function.clone(),
            },
            llvm_symbol: "main".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
        },
    );
    contract
        .function_bodies
        .insert(function.clone(), key.clone());

    assert_eq!(
        validate_backend_contract_against_functions(&contract, std::iter::empty()),
        vec![MirBackendContractError::CallableBodyMissingFunction { key, function }],
    );
}

#[test]
fn validation_reports_local_body_signature_mismatch() {
    let key = MirCallableKey::Function(def_id(1));
    let function_id = MirFunctionId::Function(def_id(1));
    let mut contract = MirBackendContract::default();
    contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(def_id(1)),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "body".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(
                &[TypeId(1)],
                TypeId(3),
                MirPassMode::Direct,
            ),
        },
    );
    contract
        .function_bodies
        .insert(function_id.clone(), key.clone());
    let function = MirFunction {
        id: function_id,
        name: "body".to_string(),
        basic_blocks: vec![],
        local_decls: vec![
            LocalDecl {
                ty: TypeId(2),
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: TypeId(2),
                mutability: Mutability::Not,
                name: Some("arg".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
        ],
        closure_captures: vec![],
        arg_count: 1,
        ret_type: TypeId(2),
        ownership: Default::default(),
    };

    let messages = validate_backend_contract_against_mir(&contract, [&function])
        .into_iter()
        .map(|error| format!("{:?}", error))
        .collect::<Vec<_>>();

    assert!(
        messages.iter().any(|message| {
            message.contains("LocalBodyParamTypeMismatch")
                && message.contains("TypeId(1)")
                && message.contains("TypeId(2)")
        }),
        "expected parameter type mismatch, got {messages:?}"
    );
    assert!(
        messages.iter().any(|message| {
            message.contains("LocalBodyReturnTypeMismatch")
                && message.contains("TypeId(3)")
                && message.contains("TypeId(2)")
        }),
        "expected return type mismatch, got {messages:?}"
    );
}

#[test]
fn validation_reports_local_body_return_abi_mismatch() {
    let key = MirCallableKey::Function(def_id(1));
    let function_id = MirFunctionId::Function(def_id(1));
    let mut contract = MirBackendContract::default();
    contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(def_id(1)),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "body".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature {
                params: vec![],
                ret: MirReturnAbi {
                    semantic_ty: TypeId(2),
                    abi_ty: TypeId(3),
                },
            },
        },
    );
    contract
        .function_bodies
        .insert(function_id.clone(), key.clone());
    let function = MirFunction {
        id: function_id,
        name: "body".to_string(),
        basic_blocks: vec![],
        local_decls: vec![LocalDecl {
            ty: TypeId(2),
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: TypeId(2),
        ownership: Default::default(),
    };

    let messages = validate_backend_contract_against_mir(&contract, [&function])
        .into_iter()
        .map(|error| format!("{:?}", error))
        .collect::<Vec<_>>();

    assert!(
        messages.iter().any(|message| {
            message.contains("LocalBodyReturnAbiTypeMismatch")
                && message.contains("TypeId(3)")
                && message.contains("TypeId(2)")
        }),
        "expected return ABI mismatch, got {messages:?}"
    );
}

#[test]
fn validation_reports_local_body_return_local_mismatch() {
    let key = MirCallableKey::Function(def_id(1));
    let function_id = MirFunctionId::Function(def_id(1));
    let mut contract = MirBackendContract::default();
    contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(def_id(1)),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "body".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], TypeId(2), MirPassMode::Direct),
        },
    );
    contract
        .function_bodies
        .insert(function_id.clone(), key.clone());
    let function = MirFunction {
        id: function_id,
        name: "body".to_string(),
        basic_blocks: vec![],
        local_decls: vec![LocalDecl {
            ty: TypeId(3),
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: TypeId(2),
        ownership: Default::default(),
    };

    let messages = validate_backend_contract_against_mir(&contract, [&function])
        .into_iter()
        .map(|error| format!("{:?}", error))
        .collect::<Vec<_>>();

    assert!(
        messages.iter().any(|message| {
            message.contains("LocalBodyReturnLocalTypeMismatch")
                && message.contains("TypeId(3)")
                && message.contains("TypeId(2)")
        }),
        "expected return local type mismatch, got {messages:?}"
    );
}

#[test]
fn validation_reports_drop_glue_callable_that_codegen_will_not_declare() {
    let dropped_ty = TypeId(1);
    let key = MirCallableKey::Instance(InstanceId(9));
    let mut contract = MirBackendContract::default();
    contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: None,
            kind: MirCallableKind::RuntimeHelper(MirRuntimeHelper::DropGlue),
            llvm_symbol: "Box_drop".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(
                &[dropped_ty],
                TypeId(0),
                MirPassMode::Direct,
            ),
        },
    );
    contract.drop_glue.insert(dropped_ty, key);

    let messages = validate_backend_contract(&contract)
        .into_iter()
        .map(|error| format!("{:?}", error))
        .collect::<Vec<_>>();

    assert!(
        messages
            .iter()
            .any(|message| message.contains("DropGlueCallableKindMismatch")),
        "expected drop-glue kind mismatch, got {messages:?}"
    );
}

#[test]
fn validation_reports_drop_glue_missing_callable_key() {
    let dropped_ty = TypeId(1);
    let key = MirCallableKey::Instance(InstanceId(404));
    let mut contract = MirBackendContract::default();
    contract.drop_glue.insert(dropped_ty, key);

    let messages = validate_backend_contract(&contract)
        .into_iter()
        .map(|error| format!("{:?}", error))
        .collect::<Vec<_>>();

    assert!(
        messages
            .iter()
            .any(|message| message.contains("DropGlueCallableMissing")),
        "expected missing drop-glue callable, got {messages:?}"
    );
}

#[test]
fn validation_reports_program_function_without_callable_mapping_when_contract_is_non_empty() {
    let function = MirFunctionId::Function(def_id(1));
    let mut contract = MirBackendContract::default();
    let ext_key = MirCallableKey::Extern(def_id(2));
    contract.callables.insert(
        ext_key.clone(),
        MirCallableDecl {
            key: ext_key,
            source_def_id: Some(def_id(2)),
            kind: MirCallableKind::Extern {
                link_name: "ffi".to_string(),
                variadic: false,
            },
            llvm_symbol: "ffi".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
        },
    );

    assert_eq!(
        validate_backend_contract_against_functions(&contract, [&function]),
        vec![MirBackendContractError::FunctionWithoutCallable { function }],
    );
}

#[test]
fn validation_reports_resolved_callable_key_used_by_body_without_contract_entry() {
    let caller = def_id(3);
    let function_id = MirFunctionId::Function(caller);
    let missing_key = MirCallableKey::Instance(InstanceId(404));
    let mut contract = MirBackendContract::default();
    let caller_key = MirCallableKey::Function(caller);
    contract.callables.insert(
        caller_key.clone(),
        MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(caller),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "main".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
        },
    );
    contract
        .function_bodies
        .insert(function_id.clone(), caller_key);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    missing_key.clone(),
                ))),
                args: vec![],
                destination: Place {
                    local: Local(0),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![LocalDecl {
            ty: TypeId(0),
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: TypeId(0),
        ownership: Default::default(),
    };

    assert_eq!(
        validate_backend_contract_against_mir(&contract, [&function]),
        vec![MirBackendContractError::ResolvedCallableWithoutContract {
            function: function_id,
            key: missing_key,
        }],
    );
}

#[test]
fn validation_rejects_callable_key_not_in_contract_even_when_source_alias_matches() {
    let caller = def_id(4);
    let callee = def_id(5);
    let function_id = MirFunctionId::Function(caller);
    let caller_key = MirCallableKey::Function(caller);
    let callee_key = MirCallableKey::Instance(InstanceId(405));
    let mut contract = MirBackendContract::default();
    contract.callables.insert(
        caller_key.clone(),
        MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(caller),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "main".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
        },
    );
    contract
        .function_bodies
        .insert(function_id.clone(), caller_key);
    contract.callables.insert(
        callee_key.clone(),
        MirCallableDecl {
            key: callee_key,
            source_def_id: Some(callee),
            kind: MirCallableKind::ObjectProvided,
            llvm_symbol: "callee_alias".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
        },
    );
    let function = MirFunction {
        id: function_id,
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    MirCallableKey::Function(callee),
                ))),
                args: vec![],
                destination: Place {
                    local: Local(0),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![LocalDecl {
            ty: TypeId(0),
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: TypeId(0),
        ownership: Default::default(),
    };

    let errors = validate_backend_contract_against_mir(&contract, [&function]);
    let messages = errors
        .iter()
        .map(|error| format!("{:?}", error))
        .collect::<Vec<_>>();

    assert!(
        messages
            .iter()
            .any(|message| message.contains("ResolvedCallableWithoutContract")),
        "expected missing canonical callable error, got {messages:?}"
    );
}

#[test]
fn backend_validation_reports_program_function_without_callable_mapping_when_contract_is_empty() {
    let function = MirFunctionId::Function(def_id(1));
    let contract = MirBackendContract::default();

    assert_eq!(
        validate_backend_contract_against_functions(&contract, [&function]),
        vec![MirBackendContractError::FunctionWithoutCallable { function }],
    );
}
