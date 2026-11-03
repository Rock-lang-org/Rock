use super::*;

#[test]
fn compile_mir_program_binds_shared_scalar_method_receiver_by_pointer() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let self_ref = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };
    let self_ref_type = type_id(&mut type_context, self_ref.clone());
    let function_id = DefId::new(CrateId(0), LocalDefId(80));
    let instance_id = InstanceId(0);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let self_place = crate::mir::Place {
        local: crate::mir::Local(1),
        projection: vec![],
    };
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "I64_identity".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Copy(crate::mir::Place {
                    local: self_place.local,
                    projection: vec![crate::mir::Projection::Deref],
                })),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            crate::mir::LocalDecl {
                ty: self_ref_type,
                mutability: crate::mir::Mutability::Not,
                name: Some("self".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "I64_identity",
        vec![self_ref_type],
        i64_type,
        true,
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(
        ir.contains("define i64 @I64_identity(ptr %0)"),
        "IR was:\n{}",
        ir
    );
    assert!(ir.contains("%self = alloca ptr"), "IR was:\n{}", ir);
    assert!(ir.contains("store ptr %0, ptr %self"), "IR was:\n{}", ir);
    assert!(ir.contains("load i64"), "IR was:\n{}", ir);
}

#[test]
fn compile_mir_program_uses_contract_pass_mode_for_method_receiver_without_legacy_metadata() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let function_id = DefId::new(CrateId(0), LocalDefId(81));
    let instance_id = InstanceId(81);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "contract_only_shared_scalar_identity".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Copy(crate::mir::Place {
                    local: crate::mir::Local(1),
                    projection: vec![],
                })),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Not,
                name: Some("self".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let key = crate::mir::MirCallableKey::Instance(instance_id);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        key.clone(),
        crate::mir::MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "contract_only_shared_scalar_identity".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature {
                params: vec![crate::mir::MirParamAbi {
                    semantic_ty: i64_type,
                    pass_mode: crate::mir::MirPassMode::Pointer,
                }],
                ret: crate::mir::MirReturnAbi {
                    semantic_ty: i64_type,
                    abi_ty: i64_type,
                },
            },
        },
    );
    backend_contract.function_bodies.insert(mir_id.clone(), key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(
        ir.contains("define i64 @contract_only_shared_scalar_identity(ptr %0)"),
        "IR was:\n{}",
        ir,
    );
    assert!(ir.contains("load i64, ptr %0"), "IR was:\n{}", ir);
    assert!(ir.contains("store i64"), "IR was:\n{}", ir);
}
