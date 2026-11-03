use std::fs;
use std::path::{Path, PathBuf};

use crate::products::ProductSourceFingerprint;
use crate::{compile_with_products, Config, SourceProvider};

#[test]
fn artifact_source_provider_text_contributes_to_product_source_hash() {
    let base = std::env::temp_dir().join(format!(
        "rock_compile_products_{}_{}",
        std::process::id(),
        "artifact_provider_hash"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    let first = compile_artifact_provider_fingerprint(&base, "1");
    let second = compile_artifact_provider_fingerprint(&base, "2");

    assert!(first.source_hash.is_some());
    assert!(second.source_hash.is_some());
    assert_ne!(first.source_hash, second.source_hash);

    let _ = fs::remove_dir_all(&base);
}

fn compile_artifact_provider_fingerprint(base: &Path, answer: &str) -> ProductSourceFingerprint {
    let artifact_path = base.join("demo.rkca");
    let entry_file = PathBuf::from("/artifact/demo/main.rk");
    let util_file = PathBuf::from("/artifact/demo/util.rk");

    compile_with_products(&Config {
        entry_file: entry_file.clone(),
        output_dir: base.join(format!("build-{answer}")),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        source_providers: vec![
            SourceProvider::Artifact {
                path: entry_file,
                artifact_path: artifact_path.clone(),
                text: "mod util\nmain = -> util::answer!\n".to_string(),
            },
            SourceProvider::Artifact {
                path: util_file,
                artifact_path,
                text: format!("answer = -> {answer}\n< answer\n"),
            },
        ],
        ..Config::default()
    })
    .unwrap()
    .products
    .unwrap()
    .crate_identity
    .source_fingerprint
}

#[test]
fn product_source_fingerprint_is_stable_across_checkout_paths() {
    let first = std::env::temp_dir().join(format!("rock_fingerprint_first_{}", std::process::id()));
    let second =
        std::env::temp_dir().join(format!("rock_fingerprint_second_{}", std::process::id()));
    let _ = fs::remove_dir_all(&first);
    let _ = fs::remove_dir_all(&second);
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(first.join("main.rk"), "main = ->\n    0\n").unwrap();
    fs::write(second.join("main.rk"), "main = ->\n    0\n").unwrap();

    let first_products = compile_with_products(&Config {
        entry_file: first.join("main.rk"),
        output_dir: first.join("build"),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        ..Config::default()
    })
    .unwrap()
    .products
    .unwrap();
    let second_products = compile_with_products(&Config {
        entry_file: second.join("main.rk"),
        output_dir: second.join("build"),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        ..Config::default()
    })
    .unwrap()
    .products
    .unwrap();

    assert_eq!(
        first_products.crate_identity.source_fingerprint,
        second_products.crate_identity.source_fingerprint
    );
    assert_eq!(
        first_products
            .crate_identity
            .source_fingerprint
            .loaded_files,
        vec![PathBuf::from("main.rk")]
    );

    let _ = fs::remove_dir_all(&first);
    let _ = fs::remove_dir_all(&second);
}

#[test]
fn product_source_fingerprint_ignores_unloaded_rock_files() {
    let base =
        std::env::temp_dir().join(format!("rock_fingerprint_unloaded_{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    fs::write(&entry_file, "main = ->\n    0\n").unwrap();

    let before = compile_with_products(&Config {
        entry_file: entry_file.clone(),
        output_dir: base.join("build-before"),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        ..Config::default()
    })
    .unwrap()
    .products
    .unwrap()
    .crate_identity
    .source_fingerprint;

    fs::write(base.join("unused.rk"), "unused = ->\n    1\n").unwrap();

    let after = compile_with_products(&Config {
        entry_file,
        output_dir: base.join("build-after"),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("demo".to_string()),
        ..Config::default()
    })
    .unwrap()
    .products
    .unwrap()
    .crate_identity
    .source_fingerprint;

    assert_eq!(before, after);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn product_source_fingerprint_uses_source_loader_loaded_files() {
    let base = std::env::temp_dir().join(format!(
        "rock_fingerprint_loaded_modules_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    fs::write(&entry_file, "mod util\n> util::answer\nmain = -> answer!\n").unwrap();
    fs::write(base.join("util.rk"), "answer = -> 0\n< answer\n").unwrap();
    fs::write(base.join("unloaded.rk"), "answer = -> 1\n").unwrap();

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
    assert_eq!(
        products.crate_identity.source_fingerprint.loaded_files,
        vec![PathBuf::from("main.rk"), PathBuf::from("util.rk")]
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn product_source_fingerprint_includes_macro_generated_source_modules() {
    let base = std::env::temp_dir().join(format!(
        "rock_fingerprint_macro_modules_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    fs::write(
        &entry_file,
        "macro include_util\n    x =>\n        mod util\n%include_util x\n\nmain = ->\n    0\n",
    )
    .unwrap();
    fs::write(base.join("util.rk"), "answer = -> 0\n< answer\n").unwrap();
    fs::write(base.join("unloaded.rk"), "answer = -> 1\n").unwrap();

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
    assert_eq!(
        products.crate_identity.source_fingerprint.loaded_files,
        vec![PathBuf::from("main.rk"), PathBuf::from("util.rk")]
    );

    let _ = fs::remove_dir_all(&base);
}
