use super::*;

#[test]
fn build_uses_canonical_function_indexes_once_per_def_id() {
    let id = DefId::new(CrateId(0), LocalDefId(0));
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(id, empty_function(id, "main"))]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        crate::hir::HirNameTables {
            functions_by_name: std::collections::HashMap::from([
                ("alias_main".to_string(), id),
                ("main".to_string(), id),
            ]),
            ..crate::hir::HirNameTables::default()
        },
        &std::collections::HashMap::from([(id, "main".to_string())]),
    );

    let mir = MirBuilder::build(&program).unwrap();
    let mir_id = crate::mir::MirFunctionId::Function(id);

    assert_eq!(mir.functions.len(), 1);
    assert!(mir.functions.contains_key(&mir_id));
    assert_eq!(mir.functions[&mir_id].name, "main");
}

#[test]
fn build_function_uses_session_source_map_for_parameter_and_return_origins() {
    let program = empty_program();
    let owner = DefId::new(CrateId(0), LocalDefId(300));
    let parameter_local = crate::ids::HirLocalId(7);
    let parameter_span = Span::new("/virtual/main.rk".into(), 10, 13);
    let operation_span = Span::new("/virtual/main.rk".into(), 0, 20);
    let function = HirFunction {
        id: owner,
        name: "main".to_string(),
        generic_params: vec![],
        generic_bounds: std::collections::HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: parameter_local,
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: vec![],
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let mut source_map = crate::source_map::SemanticSourceMap::default();
    source_map.insert_local(owner, parameter_local, parameter_span.clone());
    source_map.insert_definition(owner, operation_span.clone());

    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context).with_source_map(&source_map);
    let mir_func = builder.build_function(
        MirFunctionId::Function(owner),
        &function.name,
        &function,
        false,
        Some(owner),
        Some(operation_span.clone()),
    ).unwrap();

    assert_eq!(mir_func.local_decls[0].span, Some(operation_span.clone()));
    assert_eq!(mir_func.local_decls[1].span, Some(parameter_span.clone()));
    assert_eq!(
        mir_func.local_decls[0].origin(),
        crate::mir::MirOrigin::Source(operation_span)
    );
    assert_eq!(
        mir_func.local_decls[1].origin(),
        crate::mir::MirOrigin::Source(parameter_span)
    );
}

#[test]
fn build_function_terminates_blocks_after_if_without_else() {
    let program = empty_program();
    let func = HirFunction {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        name: "if_without_else".to_string(),
        generic_params: vec![],
        generic_bounds: std::collections::HashMap::new().into(),
        params: vec![],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(expr(
                HirExprKind::If {
                    condition: Box::new(expr(HirExprKind::BoolLiteral(true), Type::Bool)),
                    then_branch: HirBlock {
                        stmts: vec![HirStmt::Expr(expr(
                            HirExprKind::BoolLiteral(false),
                            Type::Bool,
                        ))],
                        ty: Type::Unit,
                    },
                    else_branch: None,
                },
                Type::Unit,
            ))],
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };

    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    let mir_func = builder.build_function(
        MirFunctionId::Function(func.id),
        &func.name,
        &func,
        false,
        Some(func.id),
        None,
    ).unwrap();

    assert!(mir_func
        .basic_blocks
        .iter()
        .enumerate()
        .all(|(_, block)| block.terminator.is_some()));
}
