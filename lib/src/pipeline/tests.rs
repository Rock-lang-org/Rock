use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::products::{
    CompilerProducts, ProductBodies, ProductCrateId, ProductCrateIdentity, ProductIdentityTable,
    ProductLinkData, ProductSourceFingerprint,
};
use crate::{compile_with_products, Config, SourceProvider};

#[test]
fn compile_with_products_extracts_ids_from_normal_pipeline() {
    let base = std::env::temp_dir().join(format!(
        "rock_compile_products_{}_{}",
        std::process::id(),
        "normal_pipeline"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    fs::write(&entry_file, "main = ->\n    0\n").unwrap();

    let output = compile_with_products(&Config {
        entry_file,
        output_dir: base.join("build"),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        ..Config::default()
    })
    .unwrap();

    let products = output.products.expect("products should be emitted");
    let main_id = products
        .identity_table
        .export_names
        .get("main")
        .copied()
        .expect("main should have a product ID");

    assert!(products.interface.functions.contains_key(&main_id));
    assert_eq!(
        products.identity_table.display_names.get(&main_id),
        Some(&"main".to_string())
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn compile_with_products_records_explicit_object_output_path() {
    let base = std::env::temp_dir().join(format!(
        "rock_compile_products_{}_{}",
        std::process::id(),
        "object_output"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let object_path = base.join("custom-main.o");
    fs::write(&entry_file, "main = ->\n    0\n").unwrap();

    let output = compile_with_products(&Config {
        entry_file,
        output_dir: base.join("build"),
        no_std: true,
        no_prelude: true,
        no_link: true,
        emit_object: Some(object_path.clone()),
        current_crate_name: Some("demo".to_string()),
        ..Config::default()
    })
    .unwrap();

    let products = output.products.expect("products should be emitted");
    assert!(object_path.exists());
    assert_eq!(products.link.object_path, Some(object_path));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn compile_with_products_records_object_backend_symbols() {
    let base = std::env::temp_dir().join(format!(
        "rock_compile_products_{}_{}",
        std::process::id(),
        "object_backend_symbols"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    fs::write(
        &entry_file,
        "helper = ->\n    1\n\nmain = ->\n    helper!\n",
    )
    .unwrap();

    let output = compile_with_products(&Config {
        entry_file,
        output_dir: base.join("build"),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        ..Config::default()
    })
    .unwrap();

    let products = output.products.expect("products should be emitted");
    let helper_id = products
        .identity_table
        .export_names
        .get("helper")
        .copied()
        .expect("helper should have a product ID");
    let record = products
        .link
        .records
        .get(&helper_id)
        .expect("helper should have a link record");
    assert!(record.backend_symbol.starts_with("__rock_"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn compile_with_products_records_canonical_extern_artifact_paths() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let base = workspace_root.join("target").join("tmp").join(format!(
        "rock_compile_products_{}_{}",
        std::process::id(),
        "canonical_extern_artifact"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&entry_file, "main = ->\n    0\n").unwrap();
    fs::write(&dep_object, []).unwrap();

    CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table: ProductIdentityTable {
            local_crate: Some(ProductCrateId(0)),
            ..ProductIdentityTable::default()
        },
        interface: crate::products::ProductInterface::default(),
        bodies: ProductBodies::default(),
        link: ProductLinkData {
            object_path: Some(dep_object.clone()),
            ..ProductLinkData::default()
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: BTreeMap::new(),
        proc_macros: Vec::new(),
    }
    .write_artifact_to_path(&dep_artifact)
    .unwrap();

    let relative_dep_artifact = relative_workspace_path_from_cwd(&dep_artifact, &workspace_root);
    assert!(!relative_dep_artifact.is_absolute());
    let output = compile_with_products(&Config {
        entry_file,
        output_dir: base.join("build"),
        extern_artifacts: vec![("dep".to_string(), relative_dep_artifact)],
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        ..Config::default()
    })
    .unwrap();

    let products = output.products.expect("products should be emitted");
    let dependency_path = &products.dependencies[0].artifact_path;
    assert!(dependency_path.is_absolute());
    assert_eq!(dependency_path, &dep_artifact.canonicalize().unwrap());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn compile_with_artifact_source_providers_loads_module_declarations_through_source_database() {
    let base = std::env::temp_dir().join(format!(
        "rock_compile_products_{}_{}",
        std::process::id(),
        "artifact_source_providers"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    let artifact_path = base.join("demo.rkca");
    let entry_file = PathBuf::from("/artifact/demo/main.rk");
    let util_file = PathBuf::from("/artifact/demo/util.rk");

    let output = compile_with_products(&Config {
        entry_file: entry_file.clone(),
        output_dir: base.join("build"),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        source_providers: vec![
            SourceProvider::Artifact {
                path: entry_file,
                artifact_path: artifact_path.clone(),
                text: "mod util\n> util::answer\nmain = -> answer!\n".to_string(),
            },
            SourceProvider::Artifact {
                path: util_file,
                artifact_path,
                text: "answer = -> 0\n< answer\n".to_string(),
            },
        ],
        ..Config::default()
    })
    .unwrap();

    let products = output.products.expect("products should be emitted");
    assert_eq!(
        products.crate_identity.source_fingerprint.loaded_files,
        vec![PathBuf::from("main.rk"), PathBuf::from("util.rk")]
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn compile_with_virtual_source_providers_loads_siblings_without_filesystem_probe() {
    let base = std::env::temp_dir().join(format!(
        "rock_compile_products_{}_{}",
        std::process::id(),
        "virtual_source_providers"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    let entry_file = base.join("main.rk");
    let util_file = base.join("util.rk");
    let output = compile_with_products(&Config {
        entry_file: entry_file.clone(),
        output_dir: base.join("build"),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        source_providers: vec![
            SourceProvider::Virtual {
                path: entry_file.clone(),
                text: "mod util\n> util::answer\nmain = -> answer!\n".to_string(),
            },
            SourceProvider::Virtual {
                path: util_file,
                text: "answer = -> 7\n< answer\n".to_string(),
            },
        ],
        ..Config::default()
    })
    .expect("virtual provider-backed compile should not require filesystem sources");

    let products = output.products.expect("products should be emitted");
    assert_eq!(
        products.crate_identity.source_fingerprint.loaded_files,
        vec![PathBuf::from("main.rk"), PathBuf::from("util.rk")]
    );
    assert!(
        !entry_file.exists(),
        "virtual source compile must not create or read an entry file"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn artifact_source_provider_parse_diagnostic_preserves_source_metadata() {
    let base = std::env::temp_dir().join(format!(
        "rock_compile_products_{}_{}",
        std::process::id(),
        "artifact_provider_diagnostic"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    let artifact_path = base.join("demo.rkca");
    let entry_file = PathBuf::from("/artifact/demo/main.rk");
    let source_text = "main = -> $\n";
    let diagnostics = compile_with_products(&Config {
        entry_file: entry_file.clone(),
        output_dir: base.join("build"),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        source_providers: vec![SourceProvider::Artifact {
            path: entry_file.clone(),
            artifact_path: artifact_path.clone(),
            text: source_text.to_string(),
        }],
        ..Config::default()
    })
    .unwrap_err();

    let diagnostic_source = diagnostics.0[0]
        .source
        .as_ref()
        .expect("provider parse diagnostic should retain source metadata");
    assert_eq!(diagnostic_source.display_path, entry_file);
    assert_eq!(diagnostic_source.text, source_text);
    assert_eq!(
        diagnostic_source.origin,
        crate::diagnostic::DiagnosticSourceOrigin::Artifact { artifact_path }
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn virtual_source_provider_parse_diagnostic_preserves_source_metadata() {
    let entry_file = PathBuf::from("/virtual/demo/main.rk");
    let source_text = "main = -> $\n";
    let diagnostics = compile_with_products(&Config {
        entry_file: entry_file.clone(),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        source_providers: vec![SourceProvider::Virtual {
            path: entry_file.clone(),
            text: source_text.to_string(),
        }],
        ..Config::default()
    })
    .unwrap_err();

    let diagnostic_source = diagnostics.0[0]
        .source
        .as_ref()
        .expect("virtual provider parse diagnostic should retain source metadata");
    assert_eq!(diagnostic_source.display_path, entry_file);
    assert_eq!(diagnostic_source.text, source_text);
    assert_eq!(
        diagnostic_source.origin,
        crate::diagnostic::DiagnosticSourceOrigin::Virtual
    );
}

fn relative_workspace_path_from_cwd(path: &Path, workspace_root: &Path) -> PathBuf {
    let cwd = std::env::current_dir().unwrap();
    if let Ok(relative) = path.strip_prefix(&cwd) {
        return relative.to_path_buf();
    }

    let Ok(cwd_relative) = cwd.strip_prefix(workspace_root) else {
        return path.to_path_buf();
    };
    let Ok(path_relative) = path.strip_prefix(workspace_root) else {
        return path.to_path_buf();
    };

    let mut relative = PathBuf::new();
    for component in cwd_relative.components() {
        if matches!(component, Component::Normal(_)) {
            relative.push("..");
        }
    }
    relative.push(path_relative);
    relative
}

#[test]
fn compile_rejects_extern_artifact_named_after_current_crate_before_load() {
    let config = Config {
        entry_file: PathBuf::from("missing-current-source.rk"),
        current_crate_name: Some("app".to_string()),
        extern_artifacts: vec![("app".to_string(), PathBuf::from("missing-self.rkca"))],
        ..Config::default()
    };

    let error = crate::compile(&config).expect_err("current-name artifact alias must be rejected");

    assert_eq!(error.0.len(), 1);
    assert_eq!(
        error.0[0].message,
        "External artifact crate name 'app' conflicts with current crate name 'app'",
    );
}

#[test]
fn compile_rejects_extern_artifact_named_after_entry_stem_before_load() {
    let config = Config {
        entry_file: PathBuf::from("app.rk"),
        extern_artifacts: vec![("app".to_string(), PathBuf::from("missing-self.rkca"))],
        ..Config::default()
    };

    let error = crate::compile(&config)
        .expect_err("entry-stem artifact alias must be rejected before artifact I/O");

    assert_eq!(error.0.len(), 1);
    assert_eq!(
        error.0[0].message,
        "External artifact crate name 'app' conflicts with current crate name 'app'",
    );
}

fn compile_virtual_sources(name: &str, sources: &[(&str, &str)]) -> Result<(), String> {
    let base = PathBuf::from(format!("/virtual/macro_child_modules/{name}"));
    compile_with_products(&Config {
        entry_file: base.join(sources[0].0),
        output_dir: std::env::temp_dir().join(format!(
            "rock_macro_child_modules_{}_{name}",
            std::process::id()
        )),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        source_providers: sources
            .iter()
            .map(|(path, text)| SourceProvider::Virtual {
                path: base.join(path),
                text: text.to_string(),
            })
            .collect(),
        ..Config::default()
    })
    .map(|_| ())
    .map_err(|diagnostics| {
        diagnostics
            .0
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

fn std_like_products(prelude: &str) -> CompilerProducts {
    let base = PathBuf::from("/virtual/macro_prelude/std");
    let sources = [
        ("lib.rk", "< mod fmt\n< mod text\nmod prelude\n"),
        (
            "fmt.rk",
            "< macro format\n    $x:expr =>\n        $x\n\nmacro hidden\n    =>\n        1\n",
        ),
        ("text.rk", "< macro shout\n    $x:expr =>\n        $x\n"),
        ("prelude.rk", prelude),
    ];
    compile_with_products(&Config {
        entry_file: base.join("lib.rk"),
        output_dir: std::env::temp_dir().join(format!("rock_macro_prelude_{}", std::process::id())),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("std".to_string()),
        source_providers: sources
            .iter()
            .map(|(path, text)| SourceProvider::Virtual {
                path: base.join(path),
                text: text.to_string(),
            })
            .collect(),
        ..Config::default()
    })
    .expect("std-like crate should compile")
    .products
    .expect("products should be emitted")
}

#[test]
fn std_prelude_records_reexported_macro_aliases() {
    let products = std_like_products("< std::fmt::format\n< std::text::*\n");

    let mut exported = products
        .interface
        .macros
        .iter()
        .map(|export| export.path.clone())
        .collect::<Vec<_>>();
    exported.sort();
    assert_eq!(exported, vec!["fmt::format", "text::shout"]);
    assert_eq!(
        products.interface.prelude_macros,
        BTreeMap::from([
            ("format".to_string(), "fmt::format".to_string()),
            ("shout".to_string(), "text::shout".to_string()),
        ])
    );
}

#[test]
fn std_prelude_ignores_macros_it_does_not_reexport() {
    let products = std_like_products("< std::fmt::format\n");

    assert_eq!(
        products.interface.prelude_macros,
        BTreeMap::from([("format".to_string(), "fmt::format".to_string())])
    );
}

#[test]
fn macros_expand_inside_child_file_modules() {
    compile_virtual_sources(
        "child",
        &[
            ("main.rk", "mod util\n> util::answer\nmain = -> answer!\n"),
            (
                "util.rk",
                "macro make_answer\n    $name:ident =>\n        $name = -> 42\n\n%make_answer answer\n< answer\n",
            ),
        ],
    )
    .expect("macro invocation in a child file should expand");
}

#[test]
fn macros_expand_inside_nested_child_file_modules() {
    compile_virtual_sources(
        "nested",
        &[
            ("main.rk", "mod util\n> util::deep::answer\nmain = -> answer!\n"),
            ("util.rk", "< mod deep\n"),
            (
                "deep.rk",
                "macro make_answer\n    $name:ident =>\n        $name = -> 42\n\n%make_answer answer\n< answer\n",
            ),
        ],
    )
    .expect("macro invocation in a nested child file should expand");
}

#[test]
fn macro_generated_mod_in_child_file_loads_its_module() {
    compile_virtual_sources(
        "generated_mod",
        &[
            (
                "main.rk",
                "mod util\n> util::deep::answer\nmain = -> answer!\n",
            ),
            (
                "util.rk",
                "macro include_deep\n    =>\n        < mod deep\n\n%include_deep\n",
            ),
            ("deep.rk", "< answer = -> 42\n"),
        ],
    )
    .expect("a macro-generated mod in a child file should be loaded");
}

#[test]
fn macro_errors_in_child_file_modules_are_reported() {
    let error = compile_virtual_sources(
        "child_error",
        &[
            ("main.rk", "mod util\nmain = -> 0\n"),
            (
                "util.rk",
                "macro make_answer\n    $name:ident =>\n        $name = -> 42\n\n%make_answer 1 2\n",
            ),
        ],
    )
    .expect_err("a non-matching macro call in a child file should fail");

    assert!(!error.is_empty());
}
