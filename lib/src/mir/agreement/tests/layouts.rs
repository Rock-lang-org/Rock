use super::*;

#[test]
fn agreement_requires_contract_nominal_layout_for_mir_used_struct_types() {
    let id = DefId::new(CrateId(0), LocalDefId(46));
    let function_id = MirFunctionId::Function(id);
    let struct_id = DefId::new(CrateId(0), LocalDefId(47));
    let mut type_context = TypeContext::new();
    let struct_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    let function = MirFunction {
        id: function_id.clone(),
        name: "returns_struct".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: struct_ty,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: struct_ty,
        ownership: Default::default(),
    };
    let key = MirCallableKey::Function(id);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(id),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "returns_struct".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], struct_ty, MirPassMode::Direct),
        },
    );
    backend_contract
        .function_bodies
        .insert(function_id.clone(), key);
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_layout_ids, 1);
    assert_eq!(report.invalid_backend_contract, 0);
    assert!(!report.is_clean());
}

#[test]
fn agreement_requires_contract_nominal_layouts_for_callable_signature_types() {
    let callable_id = DefId::new(CrateId(0), LocalDefId(48));
    let param_struct = DefId::new(CrateId(0), LocalDefId(49));
    let ret_struct = DefId::new(CrateId(0), LocalDefId(50));
    let abi_struct = DefId::new(CrateId(0), LocalDefId(51));
    let mut type_context = TypeContext::new();
    let param_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: param_struct,
            args: Vec::new(),
        },
    );
    let ret_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: ret_struct,
            args: Vec::new(),
        },
    );
    let abi_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: abi_struct,
            args: Vec::new(),
        },
    );
    let callable_key = MirCallableKey::Extern(callable_id);
    let mut signature =
        MirCallableSignature::from_type_ids(&[param_ty], ret_ty, MirPassMode::Direct);
    signature.ret.abi_ty = abi_ty;
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        callable_key.clone(),
        MirCallableDecl {
            key: callable_key,
            source_def_id: Some(callable_id),
            kind: MirCallableKind::Extern {
                link_name: "ffi".to_string(),
                variadic: false,
            },
            llvm_symbol: "ffi".to_string(),
            linkage: MirLinkage::External,
            signature,
        },
    );
    let program = MirProgram {
        functions: std::collections::BTreeMap::new(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_layout_ids, 3);
    assert_eq!(report.invalid_backend_contract, 0);
    assert!(!report.is_clean());
}

#[test]
fn agreement_validates_contract_layout_keys_and_nested_contract_type_layouts() {
    let callable_id = DefId::new(CrateId(0), LocalDefId(52));
    let holder_id = DefId::new(CrateId(0), LocalDefId(53));
    let enum_id = DefId::new(CrateId(0), LocalDefId(54));
    let nested_struct = DefId::new(CrateId(0), LocalDefId(55));
    let nested_enum = DefId::new(CrateId(0), LocalDefId(56));
    let mismatch_key = DefId::new(CrateId(0), LocalDefId(57));
    let mismatch_value = DefId::new(CrateId(0), LocalDefId(58));
    let projection_base_struct = DefId::new(CrateId(0), LocalDefId(59));
    let projection_arg_struct = DefId::new(CrateId(0), LocalDefId(60));
    let projection_output_struct = DefId::new(CrateId(0), LocalDefId(61));
    let drop_enum = DefId::new(CrateId(0), LocalDefId(62));
    let trait_id = DefId::new(CrateId(0), LocalDefId(63));
    let _trait_member_id = DefId::new(CrateId(0), LocalDefId(64));
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let nested_struct_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: nested_struct,
            args: Vec::new(),
        },
    );
    let nested_enum_ty = type_id(
        &mut type_context,
        Type::Enum {
            id: nested_enum,
            args: Vec::new(),
        },
    );
    let projection_base_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: projection_base_struct,
            args: Vec::new(),
        },
    );
    let projection_arg_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: projection_arg_struct,
            args: Vec::new(),
        },
    );
    let projection_output_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: projection_output_struct,
            args: Vec::new(),
        },
    );
    let drop_ty = type_id(
        &mut type_context,
        Type::Enum {
            id: drop_enum,
            args: Vec::new(),
        },
    );
    let callable_key = MirCallableKey::Extern(callable_id);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        callable_key.clone(),
        MirCallableDecl {
            key: callable_key.clone(),
            source_def_id: Some(callable_id),
            kind: MirCallableKind::Extern {
                link_name: "ffi".to_string(),
                variadic: false,
            },
            llvm_symbol: "ffi".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], unit, MirPassMode::Direct),
        },
    );
    backend_contract.nominal_layouts.insert(
        holder_id,
        MirNominalLayout::Struct {
            id: holder_id,
            fields: vec![("nested".to_string(), nested_struct_ty)],
            generic_params: Vec::new(),
        },
    );
    backend_contract.nominal_layouts.insert(
        enum_id,
        MirNominalLayout::Enum {
            id: enum_id,
            variants: vec![MirEnumVariantLayout {
                name: "Nested".to_string(),
                fields: crate::mir::MirVariantLayoutFields::Named(vec![(
                    "value".to_string(),
                    nested_enum_ty,
                )]),
            }],
            generic_params: Vec::new(),
        },
    );
    backend_contract.nominal_layouts.insert(
        mismatch_key,
        MirNominalLayout::Struct {
            id: mismatch_value,
            fields: Vec::new(),
            generic_params: Vec::new(),
        },
    );
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: projection_base_ty,
            trait_id,
            assoc_type_id: AssocTypeId(0),
            trait_args: vec![projection_arg_ty],
        },
        projection_output_ty,
    );
    backend_contract
        .drop_glue
        .insert(drop_ty, callable_key.clone());
    backend_contract.projection_traits.insert(trait_id);
    let program = MirProgram {
        functions: std::collections::BTreeMap::new(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_layout_ids, 7);
    assert_eq!(report.stale_projection_facts, 0);
    assert_eq!(report.invalid_backend_contract, 0);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_field_identity_without_nominal_layout_metadata() {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(900)));
    let struct_id = DefId::new(CrateId(0), LocalDefId(901));
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let function = MirFunction {
        id: function_id.clone(),
        name: "field_identity_without_layout".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![Projection::Field {
                        index: 0,
                        identity: Some(MirFieldIdentity::new(struct_id, FieldId(0))),
                    }],
                },
                Rvalue::Use(Operand::Constant(Constant::Unit)),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Mut,
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

    assert_eq!(report.invalid_layout_ids, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_layout_checks_use_contract_not_legacy_metadata() {
    let legacy_id = DefId::new(CrateId(0), LocalDefId(901));
    let contract_id = DefId::new(CrateId(0), LocalDefId(902));
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(903)));
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let legacy_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: legacy_id,
            args: Vec::new(),
        },
    );
    let function = MirFunction {
        id: function_id.clone(),
        name: "layout_contract_divergence".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![Projection::Field {
                        index: 0,
                        identity: Some(MirFieldIdentity::new(legacy_id, FieldId(0))),
                    }],
                },
                Rvalue::Aggregate(
                    AggregateKind::Struct {
                        id: legacy_id,
                        display_name: "Legacy".to_string(),
                    },
                    Vec::new(),
                ),
                None,
            )],
            terminator: None,
        }],
        local_decls: vec![LocalDecl {
            ty: legacy_ty,
            mutability: Mutability::Mut,
            name: Some("value".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        contract_id,
        MirNominalLayout::Struct {
            id: contract_id,
            fields: vec![("field".to_string(), unit)],
            generic_params: Vec::new(),
        },
    );
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_layout_ids, 3);
}

#[test]
fn agreement_does_not_request_layouts_for_closed_constructor_arguments() {
    let option = DefId::new(CrateId(0), LocalDefId(83));
    let compose = DefId::new(CrateId(0), LocalDefId(84));
    let unary = crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    );
    let mut env = crate::type_services::normalize::TypeNormalizationEnv::new();
    env.register_constructor(option, crate::types::NominalTypeKind::Enum, unary.clone());
    env.register_constructor(
        compose,
        crate::types::NominalTypeKind::Struct,
        crate::type_services::kind::Kind::arrow(
            unary,
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            ),
        ),
    );
    let mut type_context = TypeContext::new();
    let compose_id = type_context
        .intern_normalized_type(
            &Type::Apply {
                constructor: Box::new(Type::Constructor {
                    id: compose,
                    flavor: crate::types::NominalTypeKind::Struct,
                }),
                args: vec![
                    Type::Lambda {
                        params: vec![crate::type_services::kind::Kind::Type],
                        body: Box::new(Type::Tuple(vec![
                            Type::Enum {
                                id: option,
                                args: vec![Type::BoundVar {
                                    depth: 0,
                                    index: 0,
                                    kind: crate::type_services::kind::Kind::Type,
                                }],
                            },
                            Type::I64,
                        ])),
                    },
                    Type::I64,
                ],
            },
            &env,
        )
        .unwrap();
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        compose,
        MirNominalLayout::Struct {
            id: compose,
            fields: Vec::new(),
            generic_params: Vec::new(),
        },
    );
    let program = MirProgram {
        functions: Default::default(),
        type_context,
        backend_contract,
    };
    let mut report = MirAgreementReport::default();

    validate_type_id(&program, compose_id, &mut report);
    validate_type_nominal_layout_id(
        &program,
        &BackendContract::new(&program),
        compose_id,
        &mut HashSet::new(),
        &mut report,
    );

    assert_eq!(report.invalid_type_ids, 0);
    assert_eq!(report.invalid_layout_ids, 0);
}

#[test]
fn agreement_rejects_aggregate_with_missing_struct_layout_metadata() {
    let id = DefId::new(CrateId(0), LocalDefId(32));
    let missing_struct = DefId::new(CrateId(0), LocalDefId(33));
    let _other_struct = DefId::new(CrateId(0), LocalDefId(34));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![],
                },
                Rvalue::Aggregate(
                    AggregateKind::Struct {
                        id: missing_struct,
                        display_name: "Missing".to_string(),
                    },
                    vec![],
                ),
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

    assert_eq!(report.invalid_layout_ids, 1);
    assert!(!report.is_clean());
}
