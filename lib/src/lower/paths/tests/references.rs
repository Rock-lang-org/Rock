use super::*;

fn find_first_resolved_var(block: &crate::hir::AcceptedHirBlock) -> Option<&HirVarRef> {
    block.stmts.iter().find_map(|stmt| match stmt {
        crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
            find_first_resolved_var_expr(expr)
        }
        _ => None,
    })
}

fn find_first_resolved_var_expr(expr: &crate::hir::AcceptedHirExpr) -> Option<&HirVarRef> {
    match &expr.kind {
        crate::hir::HirExprKindFor::ResolvedVar(reference) => Some(reference),
        crate::hir::HirExprKindFor::Call(callee, args, _) => find_first_resolved_var_expr(callee)
            .or_else(|| args.iter().find_map(find_first_resolved_var_expr)),
        crate::hir::HirExprKindFor::Block(block)
        | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_resolved_var(block),
        _ => None,
    }
}

#[test]
fn local_reads_resolve_to_shadowing_local_ids() {
    let mut lowerer = Lowerer::new_for_test();
    let (inner, expr) = lowerer.with_test_body_context(|lowerer| {
        let outer = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("x".to_string(), Type::I64, false, outer);
        lowerer.scope.push();
        let inner = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("x".to_string(), Type::Bool, false, inner);
        (
            inner,
            lowerer.lower_identifier_path(&identifier_path(&["x"])),
        )
    });

    match expr.kind {
        HirExprKind::ResolvedVar(HirVarRef {
            target: HirVarTarget::Local(id),
            name,
        }) => {
            assert_eq!(name, "x");
            assert_eq!(id, inner);
        }
        other => panic!("expected resolved local var, got {other:?}"),
    }
}

#[test]
fn parameter_reads_resolve_to_parameter_local_ids() {
    let resolved = compile_source_to_resolved_hir_for_test(
        r#"
identity = x -> x
"#,
    );
    let (_, function) = resolved.program.test_function_by_name("identity").unwrap();
    let param_id = function.params[0].local_id;

    let var = find_first_resolved_var(&function.body).unwrap();
    assert_eq!(var.name, "x");
    assert_eq!(var.target, HirVarTarget::Local(param_id));
}

fn single_unit_variant_enum(id: DefId, name: &str, variant_name: &str) -> HirEnum {
    HirEnum {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        variants: vec![HirVariant {
            id: VariantId(0),
            name: variant_name.to_string(),
            fields: HirVariantFields::Unit,
        }],
    }
}

#[test]
fn lower_direct_function_reference_records_function_id() {
    let source = r#"
answer = -> 7

main = ->
    answer!
"#;

    let resolved = compile_source_to_resolved_hir_for_test(source);
    let (answer_id, _) = resolved.program.test_function_by_name("answer").unwrap();
    let (_, main) = resolved.program.test_function_by_name("main").unwrap();

    let var = find_first_resolved_var(&main.body).unwrap();
    assert_eq!(var.name, "answer");
    assert_eq!(var.target, HirVarTarget::Function(answer_id));
}

#[test]
fn lower_direct_extern_reference_records_extern_id() {
    let source = r#"
extern puts: I32 -> I32

main = ->
    puts 0
"#;

    let resolved = compile_source_to_resolved_hir_for_test(source);
    let extern_id = resolved.program.names.externs_by_name["puts"];
    let (_, main) = resolved.program.test_function_by_name("main").unwrap();

    let var = find_first_resolved_var(&main.body).unwrap();
    assert_eq!(var.name, "puts");
    assert_eq!(var.target, HirVarTarget::Extern(extern_id));
}

#[test]
fn lower_qualified_function_reference_records_function_id() {
    let mut lowerer = Lowerer::new_for_test();
    let answer_id = def_id(30);
    lowerer
        .items
        .insert_function(test_function(answer_id, "answer"));
    register_item_path(&mut lowerer, "pkg::answer", answer_id);

    let expr = lowerer.lower_identifier_path(&identifier_path(&["pkg", "answer"]));

    match expr.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "pkg::answer");
            assert_eq!(reference.target, HirVarTarget::Function(answer_id));
        }
        other => panic!("expected resolved function reference, got {other:?}"),
    }
}

#[test]
fn module_local_enum_alias_shadows_root_enum_for_qualified_variant() {
    let mut lowerer = Lowerer::new_for_test();
    let root_id = def_id(42);
    let module_id = def_id(43);
    lowerer
        .items
        .insert_enumeration(single_unit_variant_enum(root_id, "Thing", "Variant"));
    lowerer.items.insert_enumeration(single_unit_variant_enum(
        module_id,
        "demo::helper::Thing",
        "Variant",
    ));
    lowerer
        .resolver
        .item_paths
        .insert("Thing".to_string(), root_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_id, "Thing".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "Thing".to_string(),
        "demo::helper::Thing".to_string(),
        module_id,
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["Thing", "Variant"]));

    match expr.kind {
        HirExprKind::EnumVariant(enum_name, variant_name, _, Some(location)) => {
            assert_eq!(enum_name, "demo::helper::Thing");
            assert_eq!(variant_name, "Variant");
            assert_eq!(location.owner, module_id);
        }
        other => panic!("expected module alias enum variant, got {other:?}"),
    }
}

#[test]
fn lower_import_alias_function_reference_records_function_id() {
    let mut lowerer = Lowerer::new_for_test();
    let answer_id = def_id(31);
    lowerer
        .items
        .insert_function(test_function(answer_id, "answer"));
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );
    lowerer.resolver.insert_import_alias_with_name(
        "answer".to_string(),
        "pkg::answer".to_string(),
        answer_id,
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "pkg::answer");
            assert_eq!(reference.target, HirVarTarget::Function(answer_id));
        }
        other => panic!("expected resolved function alias, got {other:?}"),
    }
}

#[test]
fn module_local_alias_resolves_through_resolver_without_lowerer_string_map() {
    let mut lowerer = Lowerer::new_for_test();
    let function_id = def_id(32);
    lowerer
        .items
        .insert_function(test_function(function_id, "answer"));
    lowerer.resolver.insert_module_alias_with_name(
        "answer".to_string(),
        "demo::math::answer".to_string(),
        function_id,
    );
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::ResolvedVar(HirVarRef {
            name,
            target: HirVarTarget::Function(id),
        }) => {
            assert_eq!(name, "demo::math::answer");
            assert_eq!(id, function_id);
        }
        other => panic!("expected resolver-backed module alias, got {other:?}"),
    }
}

#[test]
fn lower_identifier_path_requires_resolver_id_for_top_level_function() {
    let mut lowerer = Lowerer::new_for_test();
    let function_id = def_id(38);
    lowerer
        .items
        .insert_function(test_function(function_id, "answer"));

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::Var(name) => assert_eq!(name, "answer"),
        other => panic!("expected unresolved var without resolver id, got {other:?}"),
    }
}

#[test]
fn module_local_alias_shadows_root_item_with_same_short_name() {
    let mut lowerer = Lowerer::new_for_test();
    let root_id = def_id(36);
    let module_id = def_id(37);
    lowerer
        .items
        .insert_function(test_function(root_id, "answer"));
    lowerer
        .items
        .insert_function(test_function(module_id, "answer"));
    lowerer
        .resolver
        .item_paths
        .insert("answer".to_string(), root_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_id, "answer".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "answer".to_string(),
        "demo::helper::answer".to_string(),
        module_id,
    );
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::ResolvedVar(HirVarRef {
            name,
            target: HirVarTarget::Function(id),
        }) => {
            assert_eq!(name, "demo::helper::answer");
            assert_eq!(id, module_id);
        }
        other => panic!("expected module alias to shadow root item, got {other:?}"),
    }
}

#[test]
fn lower_nested_scope_module_local_alias_function_reference_records_function_id() {
    let mut lowerer = Lowerer::new_for_test();
    let answer_id = def_id(34);
    lowerer
        .items
        .insert_function(test_function(answer_id, "answer"));
    lowerer.scope.push();
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );
    lowerer.resolver.insert_module_alias_with_name(
        "answer".to_string(),
        "pkg::answer".to_string(),
        answer_id,
    );
    lowerer.scope.push();

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "pkg::answer");
            assert_eq!(reference.target, HirVarTarget::Function(answer_id));
        }
        other => panic!("expected resolved nested module-local function alias, got {other:?}"),
    }
}

#[test]
fn lower_local_shadowing_module_local_alias_stays_var() {
    let mut lowerer = Lowerer::new_for_test();
    let answer_id = def_id(35);
    lowerer
        .items
        .insert_function(test_function(answer_id, "answer"));
    lowerer.scope.push();
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );
    lowerer.resolver.insert_module_alias_with_name(
        "answer".to_string(),
        "pkg::answer".to_string(),
        answer_id,
    );
    lowerer.scope.push();
    lowerer.scope.define("answer".to_string(), Type::I64, false);

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::Var(name) => assert_eq!(name, "answer"),
        other => panic!("expected local shadow to stay a var, got {other:?}"),
    }
}

#[test]
fn lower_import_alias_extern_reference_records_extern_id() {
    let mut lowerer = Lowerer::new_for_test();
    let extern_id = def_id(33);
    lowerer.items.insert_extern(HirExtern {
        id: extern_id,
        name: "pkg::puts".to_string(),
        params: vec![Type::I32],
        ret: Type::I32,
        variadic: false,
        is_unsafe: false,
    });
    lowerer.scope.define_alias(
        "puts".to_string(),
        Type::function(vec![Type::I32], Type::I32),
        false,
    );
    lowerer.resolver.insert_import_alias_with_name(
        "puts".to_string(),
        "pkg::puts".to_string(),
        extern_id,
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["puts"]));

    match expr.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "pkg::puts");
            assert_eq!(reference.target, HirVarTarget::Extern(extern_id));
        }
        other => panic!("expected resolved extern alias, got {other:?}"),
    }
}
