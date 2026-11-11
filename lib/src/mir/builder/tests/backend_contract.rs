use super::*;

#[test]
fn build_monomorphized_backend_contract_type_ids_use_final_mir_context() {
    let struct_id = DefId::new(CrateId(0), LocalDefId(0));
    let function_id = DefId::new(CrateId(0), LocalDefId(10));
    let instance_id = InstanceId(0);
    let mut program = program_with_struct(vec!["value"]);
    let function = HirFunction {
        id: function_id,
        name: "main".to_string(),
        generic_params: Vec::new(),
        generic_bounds: std::collections::HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::I32,
        body: HirBlock {
            stmts: vec![HirStmt::Return(Some(expr(
                HirExprKind::IntLiteral(1),
                Type::I32,
            )))],
            ty: Type::I32,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    program.functions.insert(function_id, function.clone());
    program
        .names
        .functions_by_name
        .insert("main".to_string(), function_id);
    program.rebuild_indexes();
    let mut type_context = crate::type_context::TypeContext::new();
    let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
    let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
    pre_mir_instance_bodies.insert(instance_id, function);
    let mono = crate::mono::MonomorphizedProgram {
        program,
        instances: std::collections::BTreeMap::from([(
            instance_id,
            crate::mono::InstanceRecord {
                id: instance_id,
                origin: crate::mono::InstanceOrigin::Function(function_id),
                substitution: Vec::new(),
                symbols: crate::mono::InstanceSymbols::new("main".to_string(), "main".to_string()),
                declared: None,
                provided_by_object: false,
                is_specialization: false,
            },
        )]),
        pre_mir_instance_bodies,
        generated_drop_instances: Default::default(),
        type_context,
        source_map: crate::source_map::SemanticSourceMap::default(),
    };

    let mir = MirBuilder::build_monomorphized(&mono).unwrap();

    let layout = mir
        .backend_contract
        .nominal_layouts
        .get(&struct_id)
        .expect("struct layout should be present");
    let crate::mir::MirNominalLayout::Struct { fields, .. } = layout else {
        panic!("expected struct layout, got {layout:?}");
    };
    assert_eq!(mir.type_context.type_for(fields[0].1), Type::I64);
}

#[test]
#[should_panic(expected = "unmaterialized method DefId reached MIR lowering")]
fn build_monomorphized_rejects_direct_static_method_function_target() {
    let caller_id = DefId::new(CrateId(0), LocalDefId(10));
    let impl_id = DefId::new(CrateId(0), LocalDefId(20));
    let method_id = DefId::new(CrateId(0), LocalDefId(21));
    let caller_instance = InstanceId(0);
    let method_instance = InstanceId(1);
    let function_instance = InstanceId(2);
    let callee = expr(
        HirExprKind::ResolvedVar(HirVarRef {
            name: "Global::dealloc".to_string(),
            target: HirVarTarget::Function(method_id),
        }),
        Type::function(Vec::new(), Type::I64),
    );
    let call = expr(
        HirExprKind::Call(
            Box::new(callee),
            Vec::new(),
            Some(crate::hir::HirCallTarget::Function(method_id)),
        ),
        Type::I64,
    );
    let caller = HirFunction {
        id: caller_id,
        name: "caller".to_string(),
        generic_params: Vec::new(),
        generic_bounds: std::collections::HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::I64,
        body: HirBlock {
            stmts: vec![HirStmt::Return(Some(call))],
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let method = HirFunction {
        id: method_id,
        name: "dealloc".to_string(),
        generic_params: Vec::new(),
        generic_bounds: std::collections::HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::I64,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let mut program = empty_program();
    program.functions.insert(caller_id, caller.clone());
    program
        .names
        .functions_by_name
        .insert("caller".to_string(), caller_id);
    program.rebuild_indexes();
    let mut type_context = crate::type_context::TypeContext::new();
    let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
    let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
    pre_mir_instance_bodies.insert(caller_instance, caller);
    let mono = crate::mono::MonomorphizedProgram {
        program,
        instances: std::collections::BTreeMap::from([
            (
                caller_instance,
                crate::mono::InstanceRecord {
                    id: caller_instance,
                    origin: crate::mono::InstanceOrigin::Function(caller_id),
                    substitution: Vec::new(),
                    symbols: crate::mono::InstanceSymbols::new("caller", "caller"),
                    declared: None,
                    provided_by_object: false,
                    is_specialization: false,
                },
            ),
            (
                method_instance,
                crate::mono::InstanceRecord {
                    id: method_instance,
                    origin: crate::mono::InstanceOrigin::ImplMethod {
                        owner: crate::mono::InstanceImplOwner::Named(impl_id),
                        method: method_id,
                    },
                    substitution: Vec::new(),
                    symbols: crate::mono::InstanceSymbols::new(
                        "__rock_Global_dealloc",
                        "__rock_Global_dealloc",
                    ),
                    declared: Some(method.clone()),
                    provided_by_object: true,
                    is_specialization: false,
                },
            ),
            (
                function_instance,
                crate::mono::InstanceRecord {
                    id: function_instance,
                    origin: crate::mono::InstanceOrigin::Function(method_id),
                    substitution: Vec::new(),
                    symbols: crate::mono::InstanceSymbols::new(
                        "__rock_Global_dealloc",
                        "__rock_Global_dealloc",
                    ),
                    declared: Some(method.clone()),
                    provided_by_object: false,
                    is_specialization: false,
                },
            ),
        ]),
        pre_mir_instance_bodies,
        generated_drop_instances: Default::default(),
        type_context,
        source_map: crate::source_map::SemanticSourceMap::default(),
    };

    let mir = MirBuilder::build_monomorphized(&mono).unwrap();
    let caller = mir
        .functions
        .get(&MirFunctionId::Instance(caller_instance))
        .expect("caller MIR function");

    assert!(
        caller.basic_blocks.iter().any(|block| matches!(
            &block.terminator,
            Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(crate::mir::MirCallable::Resolved(
                    crate::mir::MirCallableKey::Instance(found),
                ))),
                ..
            }) if *found == function_instance
        )),
        "caller MIR was: {caller:#?}"
    );
}

#[test]
fn backend_contract_records_one_canonical_layout_without_visiting_display_aliases() {
    let struct_id = DefId::new(CrateId(0), LocalDefId(0));
    let enum_id = DefId::new(CrateId(0), LocalDefId(1));
    let mut program = program_with_struct(vec!["value"]);
    program
        .names
        .structs_by_name
        .insert("AliasFoo".to_string(), struct_id);
    program.enums.insert(
        enum_id,
        crate::hir::HirEnum {
            id: enum_id,
            name: "Result".to_string(),
            generic_params: Vec::new(),
            variants: Vec::new(),
        },
    );
    program
        .names
        .enums_by_name
        .insert("AliasResult".to_string(), enum_id);
    let canonical_names = std::collections::HashMap::from([
        (struct_id, "Foo".to_string()),
        (enum_id, "Result".to_string()),
    ]);
    program.rebuild_indexes_with_canonical_names(&canonical_names);

    let mut type_context = crate::type_context::TypeContext::new();
    let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
    let mono = crate::mono::MonomorphizedProgram {
        program,
        instances: std::collections::BTreeMap::new(),
        pre_mir_instance_bodies: crate::mono::PreMirInstanceBodies::new(),
        generated_drop_instances: Default::default(),
        type_context,
        source_map: crate::source_map::SemanticSourceMap::default(),
    };
    let functions = std::collections::BTreeMap::new();
    let mut contract_type_context = crate::type_context::TypeContext::new();
    let bodies = MirInstanceBodies::new();

    let contract = MirBuilder::backend_contract_for_program(
        &mono,
        &functions,
        &mut contract_type_context,
        &bodies,
    );

    assert_eq!(contract.nominal_layouts.len(), 2);
    assert!(matches!(
        contract.nominal_layouts.get(&struct_id),
        Some(crate::mir::MirNominalLayout::Struct { id, .. }) if *id == struct_id
    ));
    assert!(matches!(
        contract.nominal_layouts.get(&enum_id),
        Some(crate::mir::MirNominalLayout::Enum { id, .. }) if *id == enum_id
    ));
}

#[test]
#[should_panic(expected = "has neither a MIR body nor a declared signature")]
fn backend_contract_rejects_instance_without_body_or_declared_signature() {
    let instance_id = InstanceId(90);
    let function_id = DefId::new(CrateId(0), LocalDefId(90));
    let mono = crate::mono::MonomorphizedProgram {
        program: empty_program(),
        instances: std::collections::BTreeMap::from([(
            instance_id,
            crate::mono::InstanceRecord {
                id: instance_id,
                origin: crate::mono::InstanceOrigin::Function(function_id),
                substitution: Vec::new(),
                symbols: crate::mono::InstanceSymbols::new("missing", "missing"),
                declared: None,
                provided_by_object: true,
                is_specialization: false,
            },
        )]),
        pre_mir_instance_bodies: crate::mono::PreMirInstanceBodies::new(),
        generated_drop_instances: Default::default(),
        type_context: TypeContext::new(),
        source_map: crate::source_map::SemanticSourceMap::default(),
    };
    let mut type_context = TypeContext::new();

    MirBuilder::backend_contract_for_program(
        &mono,
        &std::collections::BTreeMap::new(),
        &mut type_context,
        &MirInstanceBodies::new(),
    );
}

#[test]
fn backend_contract_records_runtime_requirements_from_final_mir() {
    let bounds_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1000)));
    let captured_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1001)));
    let empty_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1002)));
    let closure_owner = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1003)));
    let closure_id = MirClosureId {
        owner: closure_owner,
        local_index: 0,
    };
    let mut type_context = TypeContext::new();
    let i64_ty = type_context.intern_type(&Type::I64);
    let function = |id: MirFunctionId, statements: Vec<StatementData>| MirFunction {
        id,
        name: "runtime_requirement_fixture".to_string(),
        basic_blocks: vec![BasicBlock {
            statements,
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: i64_ty,
                mutability: Mutability::Not,
                name: Some("base".to_string()),
                span: None,
                source: LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_ty,
                mutability: Mutability::Not,
                name: Some("index".to_string()),
                span: None,
                source: LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: TypeId(0),
        ownership: Default::default(),
    };
    let functions = std::collections::BTreeMap::from([
        (
            bounds_id.clone(),
            function(
                bounds_id,
                vec![StatementData::assert(
                    crate::mir::MirAssert {
                        kind: crate::mir::MirAssertKind::BoundsCheck,
                        operands: vec![
                            Operand::Copy(Place {
                                local: Local(0),
                                projection: vec![],
                            }),
                            Operand::Copy(Place {
                                local: Local(1),
                                projection: vec![],
                            }),
                        ],
                    },
                    None,
                )],
            ),
        ),
        (
            captured_id.clone(),
            function(
                captured_id,
                vec![StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: vec![],
                    },
                    Rvalue::Closure(crate::mir::MirClosure {
                        id: closure_id.clone(),
                        display_name: "captured".to_string(),
                        captures: vec![crate::mir::MirClosureCapture {
                            name: "capture".to_string(),
                            local: Local(1),
                            kind: crate::mir::MirClosureCaptureKind::ByValue,
                            span: None,
                        }],
                    }),
                    None,
                )],
            ),
        ),
        (
            empty_id.clone(),
            function(
                empty_id,
                vec![StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: vec![],
                    },
                    Rvalue::Closure(crate::mir::MirClosure {
                        id: closure_id,
                        display_name: "empty".to_string(),
                        captures: vec![],
                    }),
                    None,
                )],
            ),
        ),
    ]);
    let mut contract = crate::mir::MirBackendContract::default();

    MirBuilder::populate_backend_contract_runtime_requirements(&mut contract, &functions);

    assert_eq!(
        contract.runtime_requirements,
        std::collections::BTreeSet::from([
            crate::mir::MirRuntimeHelper::BoundsCheck,
            crate::mir::MirRuntimeHelper::HeapAlloc,
        ])
    );
}

#[test]
fn fallback_contract_closure_symbols_include_owner_identity() {
    let mut type_context = TypeContext::new();
    let unit_id = type_context.intern_type(&Type::Unit);
    let left_parent = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(10)));
    let right_parent = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(11)));
    let left_id = MirFunctionId::Closure(Box::new(MirClosureId {
        owner: left_parent,
        local_index: 0,
    }));
    let right_id = MirFunctionId::Closure(Box::new(MirClosureId {
        owner: right_parent,
        local_index: 0,
    }));
    let closure_function = |id: MirFunctionId| MirFunction {
        id,
        name: "lambda_0".to_string(),
        basic_blocks: vec![],
        local_decls: vec![LocalDecl {
            ty: unit_id,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit_id,
        ownership: Default::default(),
    };
    let functions = std::collections::BTreeMap::from([
        (left_id.clone(), closure_function(left_id.clone())),
        (right_id.clone(), closure_function(right_id.clone())),
    ]);
    let mut contract = crate::mir::MirBackendContract::default();

    MirBuilder::populate_backend_contract_function_bodies(&mut contract, &functions, &type_context);

    let left_symbol = &contract
        .callable(&crate::mir::MirCallableKey::Closure(left_id))
        .expect("left closure callable")
        .llvm_symbol;
    let right_symbol = &contract
        .callable(&crate::mir::MirCallableKey::Closure(right_id))
        .expect("right closure callable")
        .llvm_symbol;
    assert_ne!(left_symbol, right_symbol);
    assert_ne!(left_symbol, "lambda_0");
    assert_ne!(right_symbol, "lambda_0");
}

#[test]
fn drop_glue_collection_consumes_generated_mono_instance() {
    let mut type_context = TypeContext::new();
    let struct_id = DefId::new(CrateId(0), LocalDefId(50));
    let drop_trait_id = DefId::new(CrateId(0), LocalDefId(51));
    let drop_member_id = DefId::new(CrateId(0), LocalDefId(52));
    let instance_id = InstanceId(55);
    let ty = Type::Struct {
        id: struct_id,
        args: vec![],
    };
    let ty_id = type_context.intern_type(&ty);
    let generated = std::collections::BTreeMap::from([(
        ty_id,
        crate::mono::GeneratedMethodInstance {
            receiver_ty: ty_id,
            trait_id: drop_trait_id,
            member_id: drop_member_id,
            instance_id,
            origin_span: None,
        },
    )]);

    let mut contract = crate::mir::MirBackendContract::default();
    MirBuilder::populate_backend_contract_drop_glue(&mut contract, &generated);

    assert_eq!(
        contract.drop_glue.get(&ty_id),
        Some(&crate::mir::MirCallableKey::Instance(instance_id)),
    );
}

#[test]
fn intrinsic_contract_signature_uses_projected_place_types() {
    let struct_id = DefId::new(CrateId(0), LocalDefId(41));
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(42)));
    let mut type_context = TypeContext::new();
    let unit_id = type_context.intern_type(&Type::Unit);
    let ptr_id = type_context.intern_type(&Type::Pointer(Box::new(Type::U8)));
    let len_id = type_context.intern_type(&Type::I64);
    let slice_id =
        type_context.intern_type(&Type::Pointer(Box::new(Type::Slice(Box::new(Type::U8)))));
    let struct_id_ty = type_context.intern_type(&Type::Struct {
        id: struct_id,
        args: vec![],
    });
    let function = MirFunction {
        id: function_id.clone(),
        name: "uses_raw_wide_pointer".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    crate::mir::MirCallableKey::Intrinsic(MirIntrinsicId::PtrFromParts),
                ))),
                args: vec![
                    Operand::Copy(Place {
                        local: Local(1),
                        projection: vec![Projection::Field {
                            index: 0,
                            identity: None,
                        }],
                    }),
                    Operand::Copy(Place {
                        local: Local(2),
                        projection: vec![],
                    }),
                ],
                destination: Place {
                    local: Local(3),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit_id,
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: struct_id_ty,
                mutability: Mutability::Not,
                name: Some("buffer".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: len_id,
                mutability: Mutability::Not,
                name: Some("len".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: slice_id,
                mutability: Mutability::Not,
                name: Some("slice".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit_id,
        ownership: Default::default(),
    };
    let functions = std::collections::BTreeMap::from([(function_id, function)]);
    let mut contract = crate::mir::MirBackendContract::default();
    contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("ptr".to_string(), ptr_id), ("len".to_string(), len_id)],
            generic_params: vec![],
        },
    );

    MirBuilder::populate_backend_contract_function_bodies(&mut contract, &functions, &type_context);

    let declaration = contract
        .callable(&crate::mir::MirCallableKey::Intrinsic(
            MirIntrinsicId::PtrFromParts,
        ))
        .expect("PtrFromParts intrinsic callable should be registered");
    assert_eq!(declaration.signature.params[0].semantic_ty, ptr_id);
    assert_eq!(declaration.signature.params[1].semantic_ty, len_id);
    assert_eq!(declaration.signature.ret.semantic_ty, slice_id);
}

#[test]
fn intrinsic_contract_signature_uses_projected_aggregate_field_types() {
    let trait_id = DefId::new(CrateId(0), LocalDefId(43));
    let assoc_type_id = AssocTypeId(0);
    let struct_id = DefId::new(CrateId(0), LocalDefId(44));
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(45)));
    let mut type_context = TypeContext::new();
    let unit_id = type_context.intern_type(&Type::Unit);
    let base_id = type_context.intern_type(&Type::I64);
    let ptr_id = type_context.intern_type(&Type::Pointer(Box::new(Type::U8)));
    let len_id = type_context.intern_type(&Type::I64);
    let slice_id =
        type_context.intern_type(&Type::Pointer(Box::new(Type::Slice(Box::new(Type::U8)))));
    let struct_ty = Type::Struct {
        id: struct_id,
        args: vec![],
    };
    let struct_type_id = type_context.intern_type(&struct_ty);
    let projection_id = type_context.intern_type(&Type::Projection {
        ty: Box::new(Type::I64),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: Vec::new(),
    });
    let function = MirFunction {
        id: function_id.clone(),
        name: "uses_projected_borrow_slice".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    crate::mir::MirCallableKey::Intrinsic(MirIntrinsicId::PtrFromParts),
                ))),
                args: vec![
                    Operand::Copy(Place {
                        local: Local(1),
                        projection: vec![Projection::Field {
                            index: 0,
                            identity: None,
                        }],
                    }),
                    Operand::Copy(Place {
                        local: Local(2),
                        projection: vec![],
                    }),
                ],
                destination: Place {
                    local: Local(3),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit_id,
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: projection_id,
                mutability: Mutability::Not,
                name: Some("projected_buffer".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: len_id,
                mutability: Mutability::Not,
                name: Some("len".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: slice_id,
                mutability: Mutability::Not,
                name: Some("slice".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit_id,
        ownership: Default::default(),
    };
    let functions = std::collections::BTreeMap::from([(function_id, function)]);
    let mut contract = crate::mir::MirBackendContract::default();
    contract.projection_outputs.insert(
        crate::mir::MirProjectionKey {
            base: base_id,
            trait_id,
            assoc_type_id,
            trait_args: Vec::new(),
        },
        struct_type_id,
    );
    contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("ptr".to_string(), ptr_id), ("len".to_string(), len_id)],
            generic_params: Vec::new(),
        },
    );

    MirBuilder::populate_backend_contract_function_bodies(&mut contract, &functions, &type_context);

    let declaration = contract
        .callable(&crate::mir::MirCallableKey::Intrinsic(
            MirIntrinsicId::PtrFromParts,
        ))
        .expect("PtrFromParts intrinsic callable should be registered for projected field");
    assert_eq!(declaration.signature.params[0].semantic_ty, ptr_id);
    assert_eq!(declaration.signature.params[1].semantic_ty, len_id);
    assert_eq!(declaration.signature.ret.semantic_ty, slice_id);
}
