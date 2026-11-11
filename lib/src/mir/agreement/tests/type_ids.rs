use super::*;

#[test]
fn agreement_validates_all_backend_contract_type_ids() {
    let mut type_context = TypeContext::new();
    let valid_ty = type_id(&mut type_context, Type::I64);
    let callable_key = MirCallableKey::Extern(DefId::new(CrateId(0), LocalDefId(42)));
    let mut signature =
        MirCallableSignature::from_type_ids(&[TypeId(900)], TypeId(901), MirPassMode::Direct);
    signature.ret.abi_ty = TypeId(902);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        callable_key.clone(),
        MirCallableDecl {
            key: callable_key,
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(42))),
            kind: MirCallableKind::Extern {
                link_name: "ffi".to_string(),
                variadic: false,
            },
            llvm_symbol: "ffi".to_string(),
            linkage: MirLinkage::External,
            signature,
        },
    );
    backend_contract.nominal_layouts.insert(
        DefId::new(CrateId(0), LocalDefId(43)),
        MirNominalLayout::Struct {
            id: DefId::new(CrateId(0), LocalDefId(43)),
            fields: vec![("field".to_string(), TypeId(903))],
            generic_params: Vec::new(),
        },
    );
    backend_contract.nominal_layouts.insert(
        DefId::new(CrateId(0), LocalDefId(44)),
        MirNominalLayout::Enum {
            id: DefId::new(CrateId(0), LocalDefId(44)),
            variants: vec![
                MirEnumVariantLayout {
                    name: "Positional".to_string(),
                    fields: crate::mir::MirVariantLayoutFields::Positional(vec![TypeId(904)]),
                },
                MirEnumVariantLayout {
                    name: "Named".to_string(),
                    fields: crate::mir::MirVariantLayoutFields::Named(vec![(
                        "field".to_string(),
                        TypeId(905),
                    )]),
                },
            ],
            generic_params: Vec::new(),
        },
    );
    backend_contract.drop_glue.insert(
        TypeId(906),
        MirCallableKey::Extern(DefId::new(CrateId(0), LocalDefId(42))),
    );
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: valid_ty,
            trait_id: DefId::new(CrateId(0), LocalDefId(45)),
            assoc_type_id: AssocTypeId(0),
            trait_args: vec![TypeId(907)],
        },
        TypeId(908),
    );
    let program = MirProgram {
        functions: std::collections::BTreeMap::new(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 9);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_invalid_type_ids_in_mir_and_backend_contract() {
    let id = DefId::new(CrateId(0), LocalDefId(8));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let invalid = TypeId(999);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: invalid,
            mutability: Mutability::Not,
            name: Some("bad".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: invalid,
        ownership: Default::default(),
    };
    let mut backend_contract = MirBackendContract::default();
    let extern_key = MirCallableKey::Extern(id);
    backend_contract.callables.insert(
        extern_key.clone(),
        MirCallableDecl {
            key: extern_key,
            source_def_id: Some(id),
            kind: MirCallableKind::Extern {
                link_name: "external".to_string(),
                variadic: false,
            },
            llvm_symbol: "external".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[invalid], unit, MirPassMode::Direct),
        },
    );
    backend_contract.nominal_layouts.insert(
        id,
        MirNominalLayout::Struct {
            id,
            fields: vec![("field".to_string(), invalid)],
            generic_params: Vec::new(),
        },
    );
    backend_contract
        .drop_glue
        .insert(invalid, MirCallableKey::Function(id));
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 5);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_backend_forbidden_type_states() {
    let owner = DefId::new(CrateId(0), LocalDefId(81));
    let mut type_context = TypeContext::new();
    let type_var = type_id(&mut type_context, Type::TypeVar(TypeVarId(0)));
    let generic = type_id(
        &mut type_context,
        Type::Generic(GenericParamId { owner, index: 0 }),
    );
    let error = type_id(&mut type_context, Type::Error);
    let option = DefId::new(CrateId(0), LocalDefId(82));
    let constructor_generic = GenericParamId { owner, index: 1 };
    let unary = crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    );
    let mut normalization_env = crate::type_services::normalize::TypeNormalizationEnv::new();
    normalization_env.register_constructor(
        option,
        crate::types::NominalTypeKind::Enum,
        unary.clone(),
    );
    normalization_env.register_generic_kind(constructor_generic, unary);
    let constructor = type_context
        .intern_normalized_type(
            &Type::Constructor {
                id: option,
                flavor: crate::types::NominalTypeKind::Enum,
            },
            &normalization_env,
        )
        .unwrap();
    let application = type_context
        .intern_normalized_type(
            &Type::Apply {
                constructor: Box::new(Type::Generic(constructor_generic)),
                args: vec![Type::I64],
            },
            &normalization_env,
        )
        .unwrap();
    let lambda = type_context
        .intern_normalized_type(
            &Type::Lambda {
                params: vec![crate::type_services::kind::Kind::Type],
                body: Box::new(Type::Tuple(vec![
                    Type::BoundVar {
                        depth: 0,
                        index: 0,
                        kind: crate::type_services::kind::Kind::Type,
                    },
                    Type::I64,
                ])),
            },
            &normalization_env,
        )
        .unwrap();
    let bound = type_context
        .intern_normalized_type(
            &Type::BoundVar {
                depth: 0,
                index: 0,
                kind: crate::type_services::kind::Kind::Type,
            },
            &normalization_env,
        )
        .unwrap();
    let unit = type_id(&mut type_context, Type::Unit);
    let key = MirCallableKey::Extern(owner);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key,
            source_def_id: Some(owner),
            kind: MirCallableKind::Extern {
                link_name: "forbidden_types".to_string(),
                variadic: false,
            },
            llvm_symbol: "forbidden_types".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(
                &[
                    type_var,
                    generic,
                    error,
                    constructor,
                    application,
                    lambda,
                    bound,
                ],
                unit,
                MirPassMode::Direct,
            ),
        },
    );
    let program = MirProgram {
        functions: std::collections::BTreeMap::new(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 7);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_invalid_cast_target_type_id() {
    let id = DefId::new(CrateId(0), LocalDefId(17));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let invalid = TypeId(999);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![],
                },
                Rvalue::Cast(Operand::Constant(Constant::Int(1)), invalid),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_invalid_nested_type_id() {
    let id = DefId::new(CrateId(0), LocalDefId(18));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let invalid = TypeId(999);
    let function_ty = type_context.intern_ty(Ty::Function {
        captures_known: true,
        params: vec![invalid],
        ret: unit,
        safety: FunctionSafety::Safe,
        callable_kind: crate::types::CallableKind::Fn,
        captures: Vec::new(),
    });
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![],
                },
                Rvalue::Use(Operand::Constant(Constant::Callable(
                    MirCallable::Resolved(MirCallableKey::Intrinsic(
                        crate::mir::MirIntrinsicId::SizeOf,
                    )),
                ))),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: function_ty,
            mutability: Mutability::Not,
            name: Some("callable".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_accepts_type_id_metadata_for_type_only_intrinsic() {
    let program = intrinsic_call_program(
        crate::mir::MirIntrinsicId::SizeOf,
        vec![Operand::Constant(Constant::TypeId(TypeId(0)))],
    );

    let report = check_mir_runtime_agreement(&program);

    assert!(report.is_clean(), "unexpected report: {report:?}");
}

#[test]
fn agreement_rejects_misplaced_and_malformed_type_id_metadata() {
    let runtime_intrinsic = intrinsic_call_program(
        crate::mir::MirIntrinsicId::I64Add,
        vec![Operand::Constant(Constant::TypeId(TypeId(0)))],
    );
    let malformed_type_only = intrinsic_call_program(
        crate::mir::MirIntrinsicId::SizeOf,
        vec![Operand::Constant(Constant::Int(1))],
    );
    let unknown_type = intrinsic_call_program(
        crate::mir::MirIntrinsicId::SizeOf,
        vec![Operand::Constant(Constant::TypeId(TypeId(999)))],
    );
    let misplaced_callable = intrinsic_call_program(
        crate::mir::MirIntrinsicId::I64Add,
        vec![Operand::Constant(Constant::Callable(
            MirCallable::Resolved(MirCallableKey::Intrinsic(
                crate::mir::MirIntrinsicId::SizeOf,
            )),
        ))],
    );
    let empty_array_len = intrinsic_call_program(crate::mir::MirIntrinsicId::ArrayLen, vec![]);
    let scalar_array_len_metadata = intrinsic_call_program(
        crate::mir::MirIntrinsicId::ArrayLen,
        vec![Operand::Constant(Constant::TypeId(TypeId(0)))],
    );
    let extra_array_len_args = intrinsic_call_program(
        crate::mir::MirIntrinsicId::ArrayLen,
        vec![
            Operand::Constant(Constant::Int(1)),
            Operand::Constant(Constant::Int(2)),
        ],
    );

    assert_eq!(
        check_mir_runtime_agreement(&runtime_intrinsic).invalid_constant_operands,
        1
    );
    assert_eq!(
        check_mir_runtime_agreement(&malformed_type_only).invalid_constant_operands,
        1
    );
    assert!(check_mir_runtime_agreement(&unknown_type).invalid_type_ids > 0);
    assert_eq!(
        check_mir_runtime_agreement(&misplaced_callable).invalid_constant_operands,
        1
    );
    assert_eq!(
        check_mir_runtime_agreement(&empty_array_len).invalid_constant_operands,
        1
    );
    assert_eq!(
        check_mir_runtime_agreement(&scalar_array_len_metadata).invalid_constant_operands,
        1
    );
    assert_eq!(
        check_mir_runtime_agreement(&extra_array_len_args).invalid_constant_operands,
        1
    );
}

#[test]
fn agreement_rejects_non_concrete_array_length() {
    let mut type_context = TypeContext::new();
    let param = crate::types::GenericParamId {
        owner: DefId::new(CrateId(0), LocalDefId(60)),
        index: 0,
    };
    let unit = type_id(&mut type_context, Type::Unit);
    let symbolic = type_id(
        &mut type_context,
        Type::Array(
            Box::new(Type::I64),
            crate::types::ArrayLen::Param(param),
        ),
    );
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(61)));
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: symbolic,
                mutability: Mutability::Not,
                name: Some("arr".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert!(report.invalid_type_ids >= 1);
    assert!(!report.is_clean());
}
