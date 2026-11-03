use super::*;

#[test]
fn test_run_config_rejects_emit_artifact_without_no_link() {
    let base = std::env::temp_dir().join(format!(
        "rockc_product_artifact_{}_{}",
        std::process::id(),
        "requires_object"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let artifact_path = base.join("build").join("demo.rkca");
    std::fs::write(&entry_file, "main = ->\n    0\n").unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--output-dir",
        base.join("build").to_str().unwrap(),
        "--no-std",
        "--no-prelude",
        "--crate-name",
        "demo",
        "--emit-artifact",
        artifact_path.to_str().unwrap(),
    ])
    .unwrap();

    assert!(run_config(config).is_err());
    assert!(!artifact_path.exists());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_emits_product_artifact_and_object() {
    let base = std::env::temp_dir().join(format!(
        "rockc_product_artifact_{}_{}",
        std::process::id(),
        "emit"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let artifact_path = base.join("build").join("demo.rkca");
    let object_path = base.join("build").join("demo.o");
    std::fs::write(&entry_file, "main = ->\n    0\n").unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--output-dir",
        base.join("build").to_str().unwrap(),
        "--no-std",
        "--no-prelude",
        "--no-link",
        "--crate-name",
        "demo",
        "--emit-artifact",
        artifact_path.to_str().unwrap(),
        "--emit-object",
        object_path.to_str().unwrap(),
    ])
    .unwrap();

    run_config(config).unwrap();

    assert!(artifact_path.exists());
    assert!(object_path.exists());
    let products =
        rock_lib::products::CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();
    let main_id = products.identity_table.export_names["main"];
    assert!(products.interface.functions.contains_key(&main_id));
    assert_eq!(products.link.object_path, Some(PathBuf::from("demo.o")));

    let validate_config = Config::try_parse_from([
        "rockc",
        "--validate-artifact",
        artifact_path.to_str().unwrap(),
    ])
    .unwrap();
    run_config(validate_config).unwrap();

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_emits_colocated_product_artifact_with_relative_object_path() {
    let base = std::env::temp_dir().join(format!(
        "rockc_product_artifact_{}_{}",
        std::process::id(),
        "relative_object"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let build_dir = base.join("build");
    let artifact_path = build_dir.join("demo.rkca");
    let object_path = build_dir.join("demo.o");
    std::fs::write(&entry_file, "main = ->\n    0\n").unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--output-dir",
        build_dir.to_str().unwrap(),
        "--no-std",
        "--no-prelude",
        "--no-link",
        "--crate-name",
        "demo",
        "--emit-artifact",
        artifact_path.to_str().unwrap(),
        "--emit-object",
        object_path.to_str().unwrap(),
    ])
    .unwrap();

    run_config(config).unwrap();

    let products =
        rock_lib::products::CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();
    assert_eq!(products.link.object_path, Some(PathBuf::from("demo.o")));

    let validate_config = Config::try_parse_from([
        "rockc",
        "--validate-artifact",
        artifact_path.to_str().unwrap(),
    ])
    .unwrap();
    run_config(validate_config).unwrap();

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_emits_sibling_layout_product_artifact_with_relative_object_path() {
    let base = std::env::temp_dir().join(format!(
        "rockc_product_artifact_{}_{}",
        std::process::id(),
        "sibling_layout"
    ));
    let _ = std::fs::remove_dir_all(&base);
    let dep_dir = base.join("dep");
    let app_dir = base.join("app");
    let dep_artifacts = dep_dir.join("build").join("artifacts");
    let dep_objects = dep_dir.join("build").join("objects");
    let app_build = app_dir.join("build");
    std::fs::create_dir_all(&dep_artifacts).unwrap();
    std::fs::create_dir_all(&dep_objects).unwrap();
    std::fs::create_dir_all(&app_build).unwrap();
    let dep_entry = dep_dir.join("lib.rk");
    let app_entry = app_dir.join("main.rk");
    let dep_artifact = dep_artifacts.join("dep.rkca");
    let dep_object = dep_objects.join("dep.o");
    std::fs::write(&dep_entry, "answer = ->\n    5\n< answer\n").unwrap();
    std::fs::write(&app_entry, "> dep::answer\n\nmain = ->\n    answer!\n").unwrap();

    let dep_config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        dep_entry.to_str().unwrap(),
        "--output-dir",
        dep_objects.to_str().unwrap(),
        "--no-std",
        "--no-prelude",
        "--no-link",
        "--crate-name",
        "dep",
        "--emit-artifact",
        dep_artifact.to_str().unwrap(),
        "--emit-object",
        dep_object.to_str().unwrap(),
    ])
    .unwrap();
    run_config(dep_config).unwrap();

    let products =
        rock_lib::products::CompilerProducts::read_artifact_from_path(&dep_artifact).unwrap();
    assert_eq!(
        products.link.object_path,
        Some(PathBuf::from("objects").join("dep.o"))
    );

    let relocated_artifacts = base.join("relocated").join("artifacts");
    let relocated_objects = base.join("relocated").join("objects");
    std::fs::create_dir_all(&relocated_artifacts).unwrap();
    std::fs::create_dir_all(&relocated_objects).unwrap();
    let relocated_artifact = relocated_artifacts.join("dep.rkca");
    std::fs::copy(&dep_artifact, &relocated_artifact).unwrap();
    std::fs::copy(&dep_object, relocated_objects.join("dep.o")).unwrap();
    std::fs::remove_dir_all(dep_dir.join("build")).unwrap();

    let extern_artifact = format!("dep={}", relocated_artifact.display());
    let app_config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        app_entry.to_str().unwrap(),
        "--output-dir",
        app_build.to_str().unwrap(),
        "--no-std",
        "--no-prelude",
        "--extern-artifact",
        extern_artifact.as_str(),
    ])
    .unwrap();
    run_config(app_config).unwrap();

    let executable = app_build.join("main");
    assert!(executable.exists());
    let status = std::process::Command::new(&executable).status().unwrap();
    assert_eq!(status.code(), Some(5));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_emits_filename_only_product_artifact() {
    let base = std::env::temp_dir().join(format!(
        "rockc_product_artifact_{}_{}",
        std::process::id(),
        "filename_only"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let build_dir = base.join("build");
    let object_path = build_dir.join("demo.o");
    let artifact_filename = format!("rockc_filename_only_{}.rkca", std::process::id());
    let artifact_path = PathBuf::from(&artifact_filename);
    let _ = std::fs::remove_file(&artifact_path);
    std::fs::write(&entry_file, "main = ->\n    0\n").unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--output-dir",
        build_dir.to_str().unwrap(),
        "--no-std",
        "--no-prelude",
        "--no-link",
        "--crate-name",
        "demo",
        "--emit-artifact",
        artifact_filename.as_str(),
        "--emit-object",
        object_path.to_str().unwrap(),
    ])
    .unwrap();

    run_config(config).unwrap();
    assert!(artifact_path.exists());

    let validate_config =
        Config::try_parse_from(["rockc", "--validate-artifact", artifact_filename.as_str()])
            .unwrap();
    run_config(validate_config).unwrap();

    let _ = std::fs::remove_file(&artifact_path);
    let _ = std::fs::remove_dir_all(&base);
}
