use super::*;

#[test]
fn test_run_config_consumes_product_artifact_dependency() {
    let base = std::env::temp_dir().join(format!(
        "rockc_product_dependency_{}_{}",
        std::process::id(),
        "smoke"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let dep_dir = base.join("dep");
    let app_dir = base.join("app");
    let dep_build = dep_dir.join("build");
    let app_build = app_dir.join("build");
    std::fs::create_dir_all(&dep_build).unwrap();
    std::fs::create_dir_all(&app_build).unwrap();
    let dep_entry = dep_dir.join("lib.rk");
    let app_entry = app_dir.join("main.rk");
    let dep_artifact = dep_build.join("dep.rkca");
    let dep_object = dep_build.join("dep.o");
    std::fs::write(&dep_entry, "answer = ->\n    5\n< answer\n").unwrap();
    std::fs::write(&app_entry, "> dep::answer\n\nmain = ->\n    answer!\n").unwrap();

    let dep_config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        dep_entry.to_str().unwrap(),
        "--output-dir",
        dep_build.to_str().unwrap(),
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

    let extern_artifact = format!("dep={}", dep_artifact.display());
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
fn test_run_config_rejects_extern_artifact_name_mismatch() {
    let base = std::env::temp_dir().join(format!(
        "rockc_product_dependency_{}_{}",
        std::process::id(),
        "name_mismatch"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let dep_dir = base.join("dep");
    let app_dir = base.join("app");
    let dep_build = dep_dir.join("build");
    let app_build = app_dir.join("build");
    std::fs::create_dir_all(&dep_build).unwrap();
    std::fs::create_dir_all(&app_build).unwrap();
    let dep_entry = dep_dir.join("lib.rk");
    let app_entry = app_dir.join("main.rk");
    let dep_artifact = dep_build.join("actual.rkca");
    let dep_object = dep_build.join("actual.o");
    std::fs::write(&dep_entry, "answer = ->\n    5\n< answer\n").unwrap();
    std::fs::write(&app_entry, "> actual::answer\n\nmain = ->\n    answer!\n").unwrap();

    let dep_config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        dep_entry.to_str().unwrap(),
        "--output-dir",
        dep_build.to_str().unwrap(),
        "--no-std",
        "--no-prelude",
        "--no-link",
        "--crate-name",
        "actual",
        "--emit-artifact",
        dep_artifact.to_str().unwrap(),
        "--emit-object",
        dep_object.to_str().unwrap(),
    ])
    .unwrap();
    run_config(dep_config).unwrap();

    let extern_artifact = format!("dep={}", dep_artifact.display());
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

    assert!(run_config(app_config).is_err());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_consumes_product_artifact_generic_dependency() {
    let base = std::env::temp_dir().join(format!(
        "rockc_product_dependency_{}_{}",
        std::process::id(),
        "generic"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let dep_dir = base.join("dep");
    let app_dir = base.join("app");
    let dep_build = dep_dir.join("build");
    let app_build = app_dir.join("build");
    std::fs::create_dir_all(&dep_build).unwrap();
    std::fs::create_dir_all(&app_build).unwrap();
    let dep_entry = dep_dir.join("lib.rk");
    let app_entry = app_dir.join("main.rk");
    let dep_artifact = dep_build.join("dep.rkca");
    let dep_object = dep_build.join("dep.o");
    std::fs::write(&dep_entry, "identity = x ->\n    x\n< identity\n").unwrap();
    std::fs::write(&app_entry, "> dep::identity\n\nmain = ->\n    identity 5\n").unwrap();

    let dep_config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        dep_entry.to_str().unwrap(),
        "--output-dir",
        dep_build.to_str().unwrap(),
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

    let extern_artifact = format!("dep={}", dep_artifact.display());
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
