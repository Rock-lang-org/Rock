use super::*;

fn instance_callable_contract(
    id: InstanceId,
    callable_def_id: DefId,
    symbol: &str,
    is_method: bool,
    params: Vec<TypeId>,
    ret: TypeId,
    type_context: &TypeContext,
) -> crate::mir::MirBackendContract {
    let key = crate::mir::MirCallableKey::Instance(id);
    let mut contract = crate::mir::MirBackendContract::default();
    contract.callables.insert(
        key.clone(),
        crate::mir::MirCallableDecl {
            key,
            source_def_id: Some(callable_def_id),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: symbol.to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_instance_type_ids(
                &params,
                ret,
                is_method,
                type_context,
            ),
        },
    );
    contract
}

#[test]
fn builder_output_has_no_callable_unit_placeholders_for_simple_function_refs() {
    let callee_id = DefId::new(CrateId(0), LocalDefId(71));
    let caller_id = DefId::new(CrateId(0), LocalDefId(72));
    let callable_ty = Type::function(vec![], Type::Unit);
    let caller = HirFunction {
        body: HirBlock {
            stmts: vec![HirStmt::Expr(expr(
                HirExprKind::ResolvedVar(HirVarRef {
                    name: "callee".to_string(),
                    target: HirVarTarget::Function(callee_id),
                }),
                callable_ty.clone(),
            ))],
            ty: callable_ty.clone(),
        },
        ret_type: callable_ty,
        ..empty_function(caller_id, "caller")
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([
            (callee_id, empty_function(callee_id, "callee")),
            (caller_id, caller),
        ]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([
                ("callee".to_string(), callee_id),
                ("caller".to_string(), caller_id),
            ]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([
            (callee_id, "callee".to_string()),
            (caller_id, "caller".to_string()),
        ]),
    );

    let mir = MirBuilder::build(&program).unwrap();

    assert_mir_agreement_clean(&mir);
}

#[test]
fn resolved_function_var_lowers_to_callable_constant() {
    let id = DefId::new(CrateId(0), LocalDefId(11));
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(id, empty_function(id, "make"))]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([("make".to_string(), id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(id, "make".to_string())]),
    );
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::Unit),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });

    let resolved = expr(
        HirExprKind::ResolvedVar(HirVarRef {
            name: "make".to_string(),
            target: HirVarTarget::Function(id),
        }),
        Type::Unit,
    );

    builder.lower_expr(
        &resolved,
        Place {
            local: Local(0),
            projection: vec![],
        },
    );

    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(
            _,
            Rvalue::Use(Operand::Constant(crate::mir::Constant::Callable(
                crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Function(target))
            )))
        ) if *target == id
    )));
}

#[test]
fn resolved_instance_var_lowers_to_callable_constant_without_name_lookup() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::Unit),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let instance = InstanceId(3);
    let resolved = expr(
        HirExprKind::ResolvedVar(HirVarRef {
            name: "generic".to_string(),
            target: HirVarTarget::Instance(instance),
        }),
        Type::Unit,
    );

    builder.lower_expr(
        &resolved,
        Place {
            local: Local(0),
            projection: vec![],
        },
    );

    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(
            _,
            Rvalue::Use(Operand::Constant(crate::mir::Constant::Callable(
                crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Instance(target))
            )))
        ) if *target == instance
    )));
}

#[test]
fn named_callable_does_not_use_stale_function_name_table_owner() {
    let stale_id = DefId::new(CrateId(0), LocalDefId(42));
    let mut program = empty_program();
    program
        .names
        .functions_by_name
        .insert("ghost".to_string(), stale_id);
    let type_context = test_type_context(&program);
    let builder = MirBuilder::new(&program, &type_context);

    assert_eq!(
        builder.callable_for_expr(
            &expr(HirExprKind::Var("ghost".to_string()), Type::Unit),
            &Type::Unit,
        ),
        None
    );
}

#[test]
fn named_callable_does_not_resolve_function_from_display_name_table() {
    let id = DefId::new(CrateId(0), LocalDefId(43));
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(id, empty_function(id, "make"))]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([("make".to_string(), id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(id, "make".to_string())]),
    );
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::function(Vec::new(), Type::Unit)),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });

    builder.lower_expr(
        &expr(
            HirExprKind::Var("make".to_string()),
            Type::function(Vec::new(), Type::Unit),
        ),
        Place {
            local: Local(0),
            projection: vec![],
        },
    );

    assert!(builder.blocks[0].statements.iter().all(|stmt| !matches!(
        &stmt.kind,
        StatementKind::Assign(
            _,
            Rvalue::Use(Operand::Constant(crate::mir::Constant::Callable(
                crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Function(target))
            )))
        ) if *target == id
    )));
}

#[test]
fn named_callable_does_not_resolve_extern_from_display_name_table() {
    let id = DefId::new(CrateId(0), LocalDefId(44));
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(
            id,
            HirExtern {
                id,
                name: "puts".to_string(),
                params: vec![Type::Str],
                ret: Type::I64,
                variadic: false,
                is_unsafe: false,
            },
        )]),
        HirNameTables {
            externs_by_name: std::collections::HashMap::from([("puts".to_string(), id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::new(),
    );
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::function(vec![Type::Str], Type::I64)),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });

    builder.lower_expr(
        &expr(
            HirExprKind::Var("puts".to_string()),
            Type::function(vec![Type::Str], Type::I64),
        ),
        Place {
            local: Local(0),
            projection: vec![],
        },
    );

    assert!(builder.blocks[0].statements.iter().all(|stmt| !matches!(
        &stmt.kind,
        StatementKind::Assign(
            _,
            Rvalue::Use(Operand::Constant(crate::mir::Constant::Callable(
                crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Extern(target))
            )))
        ) if *target == id
    )));
}

#[test]
fn unresolved_impl_function_var_does_not_select_instance_by_name() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(50));
    let method_id = DefId::new(CrateId(0), LocalDefId(51));
    let instance = InstanceId(5);
    let method = HirFunction {
        id: method_id,
        name: "from_str".to_string(),
        generic_params: Vec::new(),
        generic_bounds: std::collections::HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::I64,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(expr(HirExprKind::IntLiteral(0), Type::I64))],
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("String".to_string()),
                type_name: "String".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: Vec::new().into(),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: std::collections::HashMap::from([("from_str".to_string(), method)]),
            },
        )]),
        std::collections::HashMap::new(),
        HirNameTables::default(),
        &std::collections::HashMap::new(),
    );
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::with_instance_metadata(
        &program,
        &type_context,
        std::collections::HashMap::new(),
        std::collections::HashSet::new(),
    );
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::function(Vec::new(), Type::I64)),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let unresolved = expr(
        HirExprKind::Var("String_from_str".to_string()),
        Type::function(Vec::new(), Type::I64),
    );

    builder.lower_expr(
        &unresolved,
        Place {
            local: Local(0),
            projection: vec![],
        },
    );

    assert!(builder.blocks[0].statements.iter().all(|stmt| !matches!(
        &stmt.kind,
        StatementKind::Assign(
            _,
            Rvalue::Use(Operand::Constant(crate::mir::Constant::Callable(
                crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Instance(target))
            )))
        ) if *target == instance
    )));
}

#[test]
#[should_panic(expected = "unmaterialized method call reached MIR lowering")]
fn method_call_terminator_uses_selected_method_callable() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "value", Type::I64);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let impl_id = DefId::new(CrateId(0), LocalDefId(21));
    let trait_id = DefId::new(CrateId(0), LocalDefId(22));
    let method_id = DefId::new(CrateId(0), LocalDefId(23));
    let call = expr(
        HirExprKind::MethodCall(
            Box::new(expr(HirExprKind::Var("value".to_string()), Type::I64)),
            "show".to_string(),
            vec![],
            Some(crate::types::ReceiverMode::Move),
            crate::hir::HirMethodCallTarget::impl_method(
                impl_id,
                method_id,
                Some(crate::hir::HirSelectedTraitMember {
                    trait_id,
                    member_id: method_id,
                    trait_args: vec![Type::I64],
                }),
            ),
        ),
        Type::I64,
    );

    builder.lower_expr(
        &call,
        Place {
            local: Local(1),
            projection: vec![],
        },
    );
}

#[test]
#[should_panic(expected = "unmaterialized method call reached MIR lowering")]
fn mir_rejects_method_call_with_selected_authority() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "value", Type::I64);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let impl_id = DefId::new(CrateId(0), LocalDefId(31));
    let method_id = DefId::new(CrateId(0), LocalDefId(32));
    let call = expr(
        HirExprKind::MethodCall(
            Box::new(expr(HirExprKind::Var("value".to_string()), Type::I64)),
            "show".to_string(),
            vec![],
            Some(crate::types::ReceiverMode::Move),
            crate::hir::HirMethodCallTarget::impl_method(impl_id, method_id, None),
        ),
        Type::I64,
    );

    builder.lower_expr(
        &call,
        Place {
            local: Local(1),
            projection: vec![],
        },
    );
}

#[test]
fn monomorphized_method_call_reuses_instance_callable_target() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let instance = crate::ids::InstanceId(9);
    let callee = expr(
        HirExprKind::ResolvedVar(HirVarRef {
            name: "Box::unwrap".to_string(),
            target: HirVarTarget::Instance(instance),
        }),
        Type::function(vec![], Type::I64),
    );
    let call = expr(HirExprKind::Call(Box::new(callee), vec![], None), Type::I64);

    builder.lower_expr(
        &call,
        Place {
            local: Local(0),
            projection: vec![],
        },
    );

    assert!(builder.blocks.iter().any(|block| matches!(
        &block.terminator,
        Some(crate::mir::Terminator::Call {
            func: Operand::Constant(crate::mir::Constant::Callable(
                crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Instance(target))
            )),
            ..
        }) if *target == instance
    )));

    let i64_ty = builder.type_id_for(&Type::I64);
    let backend_contract = {
        let type_context = type_context.borrow();
        instance_callable_contract(
            instance,
            DefId::new(CrateId(0), LocalDefId(9)),
            "Box::unwrap",
            true,
            vec![],
            i64_ty,
            &type_context,
        )
    };
    assert_builder_agreement_clean_with_contract(builder, Type::Unit, backend_contract);
}

#[test]
fn function_call_argument_temp_moves_noncopy_local() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    let value_ty = struct_ty(77);
    let function_ty = Type::function(vec![value_ty.clone()], Type::Unit);
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::Unit),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&function_ty),
        mutability: Mutability::Not,
        name: Some("f".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&value_ty),
        mutability: Mutability::Not,
        name: Some("value".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.var_map.insert("f".to_string(), Local(1));
    builder.var_map.insert("value".to_string(), Local(2));
    builder.blocks.push(BasicBlock {
        statements: Vec::new(),
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    let call = expr(
        HirExprKind::Call(
            Box::new(expr(HirExprKind::Var("f".to_string()), function_ty)),
            vec![expr(HirExprKind::Var("value".to_string()), value_ty)],
            None,
        ),
        Type::Unit,
    );

    builder.lower_expr(
        &call,
        Place {
            local: Local(0),
            projection: vec![],
        },
    );

    let Some(crate::mir::Terminator::Call { args, .. }) = &builder.blocks[0].terminator else {
        panic!("expected a call terminator");
    };
    let Operand::Move(argument_place) = &args[0] else {
        panic!("non-copy argument must be moved into the call");
    };
    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(
            destination,
            Rvalue::Use(Operand::Move(Place {
                local: Local(2),
                projection: source_projection,
            }))
        ) if destination == argument_place && source_projection.is_empty()
    )));
}

#[test]
fn method_receiver_mode_metadata_is_instance_keyed() {
    let struct_id = DefId::new(CrateId(0), LocalDefId(10));
    let impl_id = DefId::new(CrateId(0), LocalDefId(20));
    let method_id = DefId::new(CrateId(0), LocalDefId(21));
    let i64_instance = InstanceId(1);
    let bool_instance = InstanceId(2);
    let generic = Type::Generic(crate::types::GenericParamId {
        owner: impl_id,
        index: 0,
    });
    let method = HirFunction {
        id: method_id,
        name: "get".to_string(),
        generic_params: Vec::new(),
        generic_bounds: std::collections::HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: generic.clone(),
            mutable: false,
            is_ref: false,
        }],
        ret_type: generic.clone(),
        body: HirBlock {
            stmts: vec![HirStmt::Expr(expr(
                HirExprKind::Var("self".to_string()),
                generic.clone(),
            ))],
            ty: generic.clone(),
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(crate::types::ReceiverMode::Shared),
        is_unsafe: false,
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(
            struct_id,
            HirStruct {
                id: struct_id,
                name: "Box".to_string(),
                generic_params: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: struct_id,
                        index: 0,
                    },
                    "T",
                )],
                fields: Vec::new(),
            },
        )]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: impl_id,
                        index: 0,
                    },
                    "T",
                )],
                receiver_pattern: vec![generic].into(),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: std::collections::HashMap::from([("get".to_string(), method)]),
            },
        )]),
        std::collections::HashMap::new(),
        HirNameTables {
            structs_by_name: std::collections::HashMap::from([("Box".to_string(), struct_id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(struct_id, "Box".to_string())]),
    );
    let type_context = test_type_context(&program);
    let builder = MirBuilder::with_instance_metadata(
        &program,
        &type_context,
        std::collections::HashMap::from([
            (i64_instance, Some(crate::types::ReceiverMode::Shared)),
            (bool_instance, Some(crate::types::ReceiverMode::Mut)),
        ]),
        std::collections::HashSet::new(),
    );
    assert_eq!(
        builder.method_instance_receiver_modes.get(&i64_instance),
        Some(&Some(crate::types::ReceiverMode::Shared))
    );
}

#[test]
fn resolved_var_ignores_same_named_local_and_uses_canonical_target() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "make", Type::I64);
    builder.blocks.push(BasicBlock {
        statements: Vec::new(),
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let resolved = expr(
        HirExprKind::ResolvedVar(HirVarRef {
            name: "make".to_string(),
            target: HirVarTarget::Function(DefId::new(CrateId(0), LocalDefId(99))),
        }),
        Type::I64,
    );

    builder.lower_expr(
        &resolved,
        Place {
            local: Local(1),
            projection: vec![],
        },
    );

    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(
            _,
            Rvalue::Use(Operand::Constant(crate::mir::Constant::Callable(
                crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Function(target))
            )))
        ) if *target == DefId::new(CrateId(0), LocalDefId(99))
    )));
}

#[test]
fn resolved_local_moves_noncopy_in_borrow_context() {
    let program = program_with_struct(vec!["field"]);
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "self", struct_ty(0));
    builder.blocks.push(BasicBlock {
        statements: Vec::new(),
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&struct_ty(0)),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let resolved = expr(
        HirExprKind::ResolvedVar(HirVarRef {
            name: "self".to_string(),
            target: HirVarTarget::Local(crate::ids::HirLocalId(0)),
        }),
        struct_ty(0),
    );

    builder.lower_expr_with_context(
        &resolved,
        Place {
            local: Local(1),
            projection: vec![],
        },
        true,
    );

    assert!(matches!(
        &builder.blocks[0].statements[0].kind,
        StatementKind::Assign(
            dest,
            Rvalue::Use(Operand::Move(src))
        ) if dest.local == Local(1) && src.local == Local(0)
    ));
}

#[test]
fn reference_returning_call_destination_has_no_blanket_external_origin() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: Vec::new(),
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        }),
        mutability: Mutability::Mut,
        name: Some("destination".to_string()),
        span: None,
        source: LocalSource::Temporary,
    });

    builder.set_terminator(
        BasicBlockId(0),
        Terminator::Call {
            func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                MirCallableKey::Function(DefId::new(CrateId(0), LocalDefId(901))),
            ))),
            args: Vec::new(),
            destination: Place {
                local: Local(0),
                projection: Vec::new(),
            },
            target: BasicBlockId(1),
            span: Some(crate::lexer::Span::test()),
        },
    );

    assert!(!builder
        .ownership
        .reference_origins
        .contains(&(Local(0), ReferenceOrigin::UnknownExternal)));
}
