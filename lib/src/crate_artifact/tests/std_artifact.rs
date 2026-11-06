use super::*;

fn build_std_product_artifact(std_dir: &PathBuf, artifact_path: &PathBuf) -> PathBuf {
    build_product_artifact(std_dir, "std", artifact_path, true, false)
}

fn write_sysroot_std_product_bundle(std_dir: &PathBuf, sysroot: &PathBuf) -> PathBuf {
    let layout =
        crate::sysroot::SysrootLayout::new(sysroot.clone(), crate::sysroot::host_target_triple());
    fs::create_dir_all(&layout.target_libdir).unwrap();

    let artifact_path = layout.std_artifact.clone();
    let object_path = build_std_product_artifact(std_dir, &artifact_path);
    fs::copy(object_path, &layout.std_object).unwrap();

    artifact_path
}

#[test]
fn test_compile_with_std_artifact() {
    let hello_path = workspace_root().join("examples").join("hello.rk");
    let temp_dir = temp_test_dir("std_artifact_compile");
    let artifact_path = shared_std_product_artifact();
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    crate::compile(&Config {
        entry_file: hello_path,
        output_dir: temp_dir.join("out"),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![("std".to_string(), artifact_path.clone())],
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    })
    .unwrap();

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_run_with_std_product_artifact_links() {
    let temp_dir = temp_test_dir("std_artifact_methods");
    let artifact_path = shared_std_product_artifact();
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app(
        "main = -> 40 + 2\n",
        vec![("std".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_std_artifact_preserves_string_method_abi_and_prelude_exports() {
    let artifact_path = shared_std_product_artifact();
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    assert_eq!(products.crate_identity.name, "std");
    let sqrt_id = products.identity_table.prelude_export_names["sqrt"];
    let deref_id = products.identity_table.prelude_export_names["Deref"];
    assert_eq!(
        products
            .identity_table
            .display_names
            .get(&sqrt_id)
            .map(String::as_str),
        Some("std::ffi::libc::sqrt")
    );
    assert_eq!(
        products
            .identity_table
            .display_names
            .get(&deref_id)
            .map(String::as_str),
        Some("std::ops::deref::Deref")
    );
    for (operator, precedence) in [("+", 9), ("*", 10)] {
        assert_eq!(
            products.infix_precedence.get(operator),
            Some(&crate::products::ProductInfixOperator {
                precedence,
                declaring_crate: "std".to_string(),
            })
        );
    }

    let string_impls = products
        .interface
        .impls
        .values()
        .filter(|imp| imp.type_name == "String")
        .collect::<Vec<_>>();
    let from = string_impls
        .iter()
        .filter_map(|imp| imp.methods.get("from"))
        .find(|method| {
            method.params
                == vec![Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Str),
                }]
        })
        .unwrap();
    let concat = string_impls
        .iter()
        .find_map(|imp| imp.methods.get("concat"))
        .unwrap();
    let expected_string_ty = from.ret_type.clone();

    assert!(matches!(expected_string_ty, Type::Struct { .. }));
    assert_eq!(concat.ret_type, expected_string_ty);
    assert_eq!(
        from.params[0],
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        }
    );
    assert_eq!(concat.params[1], from.ret_type);
    assert!(products
        .interface
        .traits
        .values()
        .any(|trait_| trait_.name == "std::ops::deref::Deref"));
}

#[test]
fn test_product_std_artifact_records_static_impl_method_link_symbol_by_product_id() {
    let artifact_path = shared_std_product_artifact();
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let from = products
        .interface
        .impls
        .values()
        .filter(|imp| imp.type_name == "String")
        .filter_map(|imp| imp.methods.get("from"))
        .find(|method| {
            method.params
                == vec![Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Str),
                }]
        })
        .unwrap();
    let from_id = ProductDefId::from(from.id);
    assert!(!products.interface.functions.contains_key(&from_id));
    assert!(!products.bodies.functions.contains_key(&from_id));
    let record = products
        .link
        .records
        .get(&from_id)
        .unwrap_or_else(|| panic!("missing from link record for {from_id:?}"));
    assert!(record.backend_symbol.starts_with("__rock_"));
}

#[test]
fn test_std_product_artifact_links_static_impl_method() {
    let artifact_path = shared_std_product_artifact();

    let exit_code = write_and_run_artifact_app(
        r#"
main = ->
    s = String::from "hello"
    s.len!
"#,
        vec![("std".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 5);
}

#[test]
fn test_std_product_artifact_links_arithmetic_operator_impl_methods() {
    let artifact_path = shared_std_product_artifact();
    let arithmetic_path = workspace_root().join("examples").join("arithmetic.rk");
    let temp_dir = temp_test_dir("std_artifact_arithmetic");

    crate::compile(&Config {
        entry_file: arithmetic_path,
        output_dir: temp_dir.join("out"),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![("std".to_string(), artifact_path)],
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    })
    .unwrap();

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_std_product_artifact_specializes_generic_function_used_by_generic_impl() {
    let artifact_path = shared_std_product_artifact();

    let exit_code = write_and_run_artifact_app(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    v.len!
"#,
        vec![("std".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 1);
}

#[test]
fn test_compile_requires_explicit_std_artifact_even_with_sysroot() {
    let temp_dir = temp_test_dir("sysroot_std_compile");
    let sysroot = temp_dir.join("toolchain");
    let std_dir = temp_dir.join("std");
    let entry_file = temp_dir.join("main.rk");
    let output_dir = temp_dir.join("out");

    write_answer_std(&std_dir, 7);
    write_sysroot_std_product_bundle(&std_dir, &sysroot);
    fs::write(&entry_file, "main = -> answer!\n").unwrap();

    let config = Config {
        entry_file,
        output_dir: output_dir.clone(),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![],
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: Some(sysroot),
    };

    assert!(crate::compile(&config).is_err());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_explicit_std_artifact_works_when_sysroot_is_set() {
    let temp_dir = temp_test_dir("sysroot_std_override");
    let sysroot = temp_dir.join("toolchain");
    let sysroot_std = temp_dir.join("sysroot_std");
    let explicit_std = temp_dir.join("explicit_std");
    let explicit_artifact_path = temp_dir.join("explicit_std.rkca");

    write_answer_std(&sysroot_std, 7);
    write_answer_std(&explicit_std, 5);

    write_sysroot_std_product_bundle(&sysroot_std, &sysroot);

    build_std_product_artifact(&explicit_std, &explicit_artifact_path);
    CompilerProducts::read_artifact_from_path(&explicit_artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app_with_options(
        "> std::math::answer\n\nmain = -> answer!\n",
        vec![("std".to_string(), explicit_artifact_path)],
        Some(sysroot),
        false,
        false,
    );
    assert_eq!(exit_code, 5);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_without_explicit_std_artifact_fails_with_no_std() {
    let temp_dir = temp_test_dir("sysroot_no_std");
    let sysroot = temp_dir.join("toolchain");
    let std_dir = temp_dir.join("std");
    let entry_file = temp_dir.join("main.rk");

    write_answer_std(&std_dir, 7);
    write_sysroot_std_product_bundle(&std_dir, &sysroot);
    fs::write(&entry_file, "main = -> answer!\n").unwrap();

    let config = Config {
        entry_file,
        output_dir: temp_dir.join("out"),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![],
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: false,
        no_std: true,
        sysroot: Some(sysroot),
    };

    assert!(crate::compile(&config).is_err());

    let _ = fs::remove_dir_all(temp_dir);
}
