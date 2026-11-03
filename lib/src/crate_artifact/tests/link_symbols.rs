use super::*;

#[test]
fn test_product_artifacts_mangle_duplicate_exported_backend_symbols_by_crate() {
    let temp_dir = temp_test_dir("duplicate_exported_backend_symbols");
    let first_dir = temp_dir.join("first");
    let second_dir = temp_dir.join("second");
    let first_artifact = temp_dir.join("first.rkca");
    let second_artifact = temp_dir.join("second.rkca");

    write_crate(&first_dir, "first", "", "< helper = -> 11\n< helper\n");
    write_crate(&second_dir, "second", "", "< helper = -> 22\n< helper\n");

    build_product_artifact(&first_dir, "first", &first_artifact, true, true);
    build_product_artifact(&second_dir, "second", &second_artifact, true, true);

    let first_products = CompilerProducts::read_artifact_from_path(&first_artifact).unwrap();
    let second_products = CompilerProducts::read_artifact_from_path(&second_artifact).unwrap();
    let first_helper_id = first_products.identity_table.export_names["helper"];
    let second_helper_id = second_products.identity_table.export_names["helper"];
    let first_symbol = first_products.link.records[&first_helper_id]
        .backend_symbol
        .as_str();
    let second_symbol = second_products.link.records[&second_helper_id]
        .backend_symbol
        .as_str();
    assert!(first_symbol.starts_with("__rock_"));
    assert!(second_symbol.starts_with("__rock_"));
    assert_ne!(first_symbol, second_symbol);

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_as("first", first_artifact)
        .unwrap();
    ctx.load_product_artifact_from_path_as("second", second_artifact)
        .unwrap();
    let first = ctx.extern_crate("first").unwrap();
    let second = ctx.extern_crate("second").unwrap();
    let first_link_symbol = first
        .link()
        .backend_symbol(first.root_exports()["helper"].id)
        .unwrap()
        .to_string();
    let second_link_symbol = second
        .link()
        .backend_symbol(second.root_exports()["helper"].id)
        .unwrap()
        .to_string();

    assert_eq!(first_link_symbol, first_symbol);
    assert_eq!(second_link_symbol, second_symbol);
    assert_ne!(first_link_symbol, second_link_symbol);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifacts_distinguish_separator_underscore_exported_backend_symbols() {
    let temp_dir = temp_test_dir("separator_underscore_exported_backend_symbols");
    let first_dir = temp_dir.join("foo");
    let second_dir = temp_dir.join("foo__bar");
    let first_artifact = temp_dir.join("foo.rkca");
    let second_artifact = temp_dir.join("foo__bar.rkca");

    write_crate(&first_dir, "foo", "", "< bar__baz = -> 11\n< bar__baz\n");
    write_crate(&second_dir, "foo__bar", "", "< baz = -> 22\n< baz\n");

    build_product_artifact(&first_dir, "foo", &first_artifact, true, true);
    build_product_artifact(&second_dir, "foo__bar", &second_artifact, true, true);

    let first_products = CompilerProducts::read_artifact_from_path(&first_artifact).unwrap();
    let second_products = CompilerProducts::read_artifact_from_path(&second_artifact).unwrap();
    let first_id = first_products.identity_table.export_names["bar__baz"];
    let second_id = second_products.identity_table.export_names["baz"];
    let first_symbol = first_products.link.records[&first_id]
        .backend_symbol
        .as_str();
    let second_symbol = second_products.link.records[&second_id]
        .backend_symbol
        .as_str();

    assert_ne!(first_symbol, second_symbol);

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_as("foo", first_artifact)
        .unwrap();
    ctx.load_product_artifact_from_path_as("foo__bar", second_artifact)
        .unwrap();
    let first = ctx.extern_crate("foo").unwrap();
    let second = ctx.extern_crate("foo__bar").unwrap();
    let first_link_symbol = first
        .link()
        .backend_symbol(first.root_exports()["bar__baz"].id)
        .unwrap()
        .to_string();
    let second_link_symbol = second
        .link()
        .backend_symbol(second.root_exports()["baz"].id)
        .unwrap()
        .to_string();

    assert_eq!(first_link_symbol, first_symbol);
    assert_eq!(second_link_symbol, second_symbol);
    assert_ne!(first_link_symbol, second_link_symbol);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_links_concrete_trait_impl_method() {
    let temp_dir = temp_test_dir("concrete_trait_impl_method_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        r#"
< trait Value
    @get: I64
< Value

< struct Box
    < value: I64
< Box

impl Value for Box
    @get = -> @value

< new_box = -> Box
    value: 37
"#,
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    delete_crate_source_files(&dep_dir);

    let exit_code = write_and_run_artifact_app_with_options(
        r#"
> dep::Value
> dep::Box
> dep::new_box

main = ->
    box = new_box!
    box.get!
"#,
        vec![("dep".to_string(), artifact_path)],
        None,
        true,
        false,
    );
    assert_eq!(exit_code, 37);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_links_concrete_function_with_mono_in_name() {
    let temp_dir = temp_test_dir("mono_named_concrete_function_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        r#"< foo_mono_bar = -> 9
< foo_mono_bar
"#,
    );
    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        r#"
> dep::foo_mono_bar

main = -> foo_mono_bar!
"#,
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 9);
}

#[test]
fn test_product_artifact_interface_externs_use_resolved_def_ids() {
    let temp_dir = temp_test_dir("product_artifact_extern_def_ids");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "< marker = -> 0\nextern puts: *U8 -> I32\n",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let (product_id, extern_fn) = products
        .interface
        .externs
        .iter()
        .find(|(_, ext)| ext.name == "dep::puts")
        .unwrap_or_else(|| {
            panic!(
                "available externs: {:?}",
                products
                    .interface
                    .externs
                    .values()
                    .map(|ext| ext.name.clone())
                    .collect::<Vec<_>>()
            )
        });
    let placeholder = DefId::new(CrateId(0), LocalDefId(0));
    assert_ne!(extern_fn.id, placeholder);
    assert_eq!(
        extern_fn.id,
        DefId::new(
            CrateId(product_id.crate_id.0),
            LocalDefId(product_id.local_id.0),
        )
    );

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_imported_extern_call_links_by_def_id() {
    let temp_dir = temp_test_dir("product_artifact_imported_extern");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "extern tolower: I32 -> I32\n");
    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        "> dep::tolower\n\nmain = -> tolower 65\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 97);

    let _ = fs::remove_dir_all(temp_dir);
}
