use super::*;

#[test]
fn load_product_artifact_reinterns_structural_types_for_consumer_context() {
    let (base, _cleanup) = temp_test_dir("reinterns_structural_types");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();
    let bytes = std::fs::read(&artifact_path).unwrap();
    assert!(
        crate::products::type_table::tests::artifact_type_table_len_for_test(&bytes).unwrap() > 0
    );

    let mut type_context = crate::type_context::TypeContext::new();
    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_with_type_context(artifact_path, &mut type_context)
        .unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let function = dep
        .metadata()
        .interface()
        .function_by_canonical_name("dep::answer")
        .unwrap();
    let ret_id = type_context.id_for_type(&function.ret_type).unwrap();

    assert_eq!(type_context.type_for(ret_id), function.ret_type);
}

#[test]
fn load_product_artifact_as_reinterns_structural_types_for_consumer_context() {
    let (base, _cleanup) = temp_test_dir("as_reinterns_structural_types");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();
    let bytes = std::fs::read(&artifact_path).unwrap();
    assert!(
        crate::products::type_table::tests::artifact_type_table_len_for_test(&bytes).unwrap() > 0
    );

    let mut type_context = crate::type_context::TypeContext::new();
    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_as_with_type_context(
        "dep",
        artifact_path,
        &mut type_context,
    )
    .unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let function = dep
        .metadata()
        .interface()
        .function_by_canonical_name("dep::answer")
        .unwrap();
    let ret_id = type_context.id_for_type(&function.ret_type).unwrap();

    assert_eq!(type_context.type_for(ret_id), function.ret_type);
}

#[test]
fn load_product_artifact_does_not_mutate_type_context_when_add_fails() {
    let (base, _cleanup) = temp_test_dir("failed_add_does_not_mutate_context");
    let first_object_path = base.join("dep.o");
    fs::write(&first_object_path, []).unwrap();
    let first_products = product_with_function("dep", ProductCrateId(0), 0, first_object_path);
    let first_artifact_path = base.join("dep.rkca");
    first_products
        .write_artifact_to_path(&first_artifact_path)
        .unwrap();

    let duplicate_object_path = base.join("dep_duplicate.o");
    fs::write(&duplicate_object_path, []).unwrap();
    let mut duplicate_products =
        product_with_function("dep", ProductCrateId(0), 0, duplicate_object_path);
    let product_id = product_def_id(0, 0);
    if let Some(function) = duplicate_products.interface.functions.get_mut(&product_id) {
        function.params.push(Type::Bool);
        function.ret_type = Type::Bool;
    }
    if let Some(function) = duplicate_products.bodies.functions.get_mut(&product_id) {
        function.params.push(HirParam {
            name: "flag".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Bool,
            mutable: false,
            is_ref: false,
        });
        function.ret_type = Type::Bool;
        function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::BoolLiteral(true),
                ty: Type::Bool,
                span: Span::test(),
            })],
            ty: Type::Bool,
        };
    }
    let duplicate_artifact_path = base.join("dep_duplicate.rkca");
    duplicate_products
        .write_artifact_to_path(&duplicate_artifact_path)
        .unwrap();

    let mut type_context = crate::type_context::TypeContext::new();
    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_with_type_context(first_artifact_path, &mut type_context)
        .unwrap();

    let err = ctx
        .load_product_artifact_from_path_with_type_context(
            duplicate_artifact_path,
            &mut type_context,
        )
        .unwrap_err();

    assert!(err.contains("duplicate external crate"));
    assert!(type_context.id_for_type(&Type::Bool).is_none());
}

#[test]
fn load_product_artifact_failure_does_not_mutate_crate_context_identity_state() {
    let (base, _cleanup) = temp_test_dir("failed_load_does_not_mutate_crate_context");
    let bad_object_path = base.join("bad.o");
    fs::write(&bad_object_path, []).unwrap();
    let mut bad_products = product_with_function("bad", ProductCrateId(0), 0, bad_object_path);
    bad_products.identity_table.dependencies.insert(
        ProductCrateId(0),
        ProductCrateIdentity::local("invalid_dep".to_string()),
    );
    let bad_artifact_path = base.join("bad.rkca");
    bad_products
        .write_artifact_to_path(&bad_artifact_path)
        .unwrap();

    let mut type_context = crate::type_context::TypeContext::new();
    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path_with_type_context(bad_artifact_path, &mut type_context)
        .unwrap_err();

    assert!(err.contains("maps dependency 'invalid_dep' to local product crate ID 0"));
    assert!(ctx.product_crate_ids.is_empty());

    let good_artifact_path = write_product_fixture(&base, "good");
    ctx.load_product_artifact_from_path_with_type_context(good_artifact_path, &mut type_context)
        .unwrap();

    assert_eq!(ctx.extern_crate("good").unwrap().crate_id(), CrateId(1));
}

#[test]
fn load_product_artifact_rejects_previous_format_before_context_mutation() {
    let (base, _cleanup) = temp_test_dir("rejects_previous_format_before_mutation");
    let artifact_path = base.join("old.rkca");
    let mut preamble = Vec::new();
    preamble.extend_from_slice(&PRODUCT_ARTIFACT_MAGIC);
    preamble.extend_from_slice(&43u32.to_le_bytes());
    preamble.extend_from_slice(&0u64.to_le_bytes());
    preamble.extend_from_slice(&0u64.to_le_bytes());
    fs::write(&artifact_path, preamble).unwrap();

    let mut ctx = CrateContext::new();
    let mut type_context = crate::type_context::TypeContext::new();
    let error = ctx
        .load_product_artifact_from_path_with_type_context(artifact_path, &mut type_context)
        .unwrap_err();

    assert!(error.contains("Unsupported product artifact format 43"));
    assert!(ctx.product_crate_ids.is_empty());
    assert!(type_context.id_for_type(&Type::I64).is_none());
}

#[test]
fn load_product_artifact_remap_failure_is_transactional_for_contexts() {
    let (base, _cleanup) = temp_test_dir("remap_failure_is_transactional");
    let good_object_path = base.join("good.o");
    fs::write(&good_object_path, []).unwrap();
    let good_artifact_path = base.join("good.rkca");
    product_with_function("good", ProductCrateId(0), 0, good_object_path)
        .write_artifact_to_path(&good_artifact_path)
        .unwrap();

    let bad_object_path = base.join("bad.o");
    fs::write(&bad_object_path, []).unwrap();
    let bad_artifact_path = base.join("bad.rkca");
    let bad_id = product_def_id(0, 0);
    let mut bad_products = product_with_function("bad", ProductCrateId(0), 0, bad_object_path);
    let mut bad_function = bad_products.bodies.functions[&bad_id].clone();
    bad_function.params.push(HirParam {
        name: "ghost".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(999)),
            args: Vec::new(),
        },
        mutable: false,
        is_ref: false,
    });
    insert_function_body(&mut bad_products, bad_id, bad_function);
    bad_products
        .write_artifact_to_path(&bad_artifact_path)
        .unwrap();

    let mut ctx = CrateContext::new();
    let mut type_context = crate::type_context::TypeContext::new();
    ctx.load_product_artifact_from_path_with_type_context(good_artifact_path, &mut type_context)
        .unwrap();
    let product_ids_before = ctx.product_crate_ids.clone();
    let i64_id_before = type_context.id_for_type(&Type::I64);

    let error = ctx
        .load_product_artifact_from_path_with_type_context(bad_artifact_path, &mut type_context)
        .unwrap_err();

    assert!(error.contains("unknown nominal type definition"));
    assert_eq!(ctx.product_crate_ids, product_ids_before);
    assert_eq!(type_context.id_for_type(&Type::I64), i64_id_before);
    assert!(ctx.extern_crate("good").is_some());
    assert!(ctx.extern_crate("bad").is_none());
}
