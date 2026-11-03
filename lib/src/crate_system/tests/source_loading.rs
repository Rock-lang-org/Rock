use super::*;

#[test]
fn load_crate_from_dir_seeds_source_module_cache() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_crate_context_source_cache_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(temp_dir.join("src")).unwrap();
    std::fs::write(
        temp_dir.join("rock.toml"),
        r#"[crate]
name = "dep"
version = "0.1.0"

[lib]
path = "src/lib.rk"
"#,
    )
    .unwrap();
    std::fs::write(temp_dir.join("src/lib.rk"), "mod util\n< util::*\n").unwrap();
    let util_path = temp_dir.join("src/util.rk");
    std::fs::write(&util_path, "answer = -> 42\n< answer\n").unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_crate_from_dir(temp_dir.clone())
        .expect("source crate with module should load");

    let source = ctx
        .source_crate("dep")
        .expect("dep source crate should load");
    assert!(source.file_cache.contains_key(&util_path));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn load_crate_from_dir_uses_source_database_module_graph() {
    let dir = std::env::temp_dir().join(format!("rock_crate_source_graph_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("rock.toml"),
        "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rk"), "mod util\n< util::*\n").unwrap();
    std::fs::write(
        dir.join("src").join("util.rk"),
        "answer = -> 42\n< answer\n",
    )
    .unwrap();

    let mut ctx = crate::crate_system::CrateContext::new();
    ctx.load_crate_from_dir(dir.clone())
        .expect("source crate should load");
    let source = ctx.source_crate("dep").expect("dep should be registered");

    assert!(source.file_cache.values().any(|module| {
        module
            .filepath
            .as_ref()
            .is_some_and(|path| path.ends_with("util.rk"))
    }));
    let tree = source
        .build_module_tree()
        .expect("module tree should build");
    assert!(tree.modules.contains_key("dep::util"));
    assert_eq!(tree.modules["dep"].submodules, vec!["util".to_string()]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_crate_from_dir_with_source_providers_loads_virtual_modules() {
    let dir = std::env::temp_dir().join(format!(
        "rock_crate_virtual_source_provider_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("rock.toml"),
        "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rk"), "mod util\n< util::*\n").unwrap();
    let util_path = dir.join("src").join("util.rk");

    let mut ctx = crate::crate_system::CrateContext::new();
    ctx.load_crate_from_dir_with_source_providers(
        dir.clone(),
        vec![SourceProvider::Virtual {
            path: util_path.clone(),
            text: "answer = -> 42\n< answer\n".to_string(),
        }],
    )
    .expect("source crate should load virtual child module through SourceDatabase");

    let source = ctx.source_crate("dep").expect("dep should be registered");
    assert!(source.file_cache.contains_key(&util_path));
    let tree = source
        .build_module_tree()
        .expect("module tree should include virtual child");
    assert!(tree.modules.contains_key("dep::util"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_crate_from_dir_with_source_providers_loads_artifact_modules() {
    let dir = std::env::temp_dir().join(format!(
        "rock_crate_artifact_source_provider_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("rock.toml"),
        "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rk"), "mod util\n< util::*\n").unwrap();
    let util_path = dir.join("src").join("util.rk");
    let artifact_path = dir.join("dep.rkca");

    let mut ctx = crate::crate_system::CrateContext::new();
    ctx.load_crate_from_dir_with_source_providers(
        dir.clone(),
        vec![SourceProvider::Artifact {
            path: util_path.clone(),
            artifact_path,
            text: "answer = -> 42\n< answer\n".to_string(),
        }],
    )
    .expect("source crate should load artifact-backed child module through SourceDatabase");

    let source = ctx.source_crate("dep").expect("dep should be registered");
    assert!(source.file_cache.contains_key(&util_path));
    let tree = source
        .build_module_tree()
        .expect("module tree should include artifact-backed child");
    assert!(tree.modules.contains_key("dep::util"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_crate_with_dependencies_with_source_providers_loads_nested_virtual_modules() {
    let root = std::env::temp_dir().join(format!(
        "rock_crate_dependency_virtual_provider_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let app = root.join("app");
    let dep = root.join("dep");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::create_dir_all(dep.join("src")).unwrap();
    std::fs::write(
        app.join("rock.toml"),
        "[crate]\nname = \"app\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n\n[dependencies.dep]\npath = \"../dep\"\n",
    )
    .unwrap();
    std::fs::write(app.join("src").join("lib.rk"), "main = -> 0\n").unwrap();
    std::fs::write(
        dep.join("rock.toml"),
        "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
    )
    .unwrap();
    std::fs::write(dep.join("src").join("lib.rk"), "mod util\n< util::*\n").unwrap();
    let util_path = dep.join("src").join("util.rk");

    let mut ctx = crate::crate_system::CrateContext::new();
    let loaded = ctx
        .load_crate_with_dependencies_with_source_providers(
            app.clone(),
            &mut Vec::new(),
            vec![SourceProvider::Virtual {
                path: util_path.clone(),
                text: "answer = -> 42\n< answer\n".to_string(),
            }],
        )
        .expect("recursive dependency load should use configured source providers");

    assert_eq!(loaded, "app");
    let dep_source = ctx.source_crate("dep").expect("dep should be registered");
    assert!(dep_source.file_cache.contains_key(&util_path));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn load_crate_from_dir_reports_missing_lib_through_source_database() {
    let dir = std::env::temp_dir().join(format!("rock_crate_missing_lib_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("rock.toml"),
        "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/missing.rk\"\n",
    )
    .unwrap();

    let mut ctx = crate::crate_system::CrateContext::new();
    let error = ctx
        .load_crate_from_dir(dir.clone())
        .expect_err("missing source crate lib should fail through source loading");

    assert!(error.contains("Failed to read source file"), "{error}");
    assert!(error.contains("src/missing.rk"), "{error}");
    assert!(!error.contains("Lib path"), "{error}");

    let _ = std::fs::remove_dir_all(&dir);
}
