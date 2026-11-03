use super::*;

fn empty_manifest(name: &str) -> CrateManifest {
    CrateManifest {
        crate_: CrateConfig {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            no_std: false,
        },
        targets: vec![CrateTarget::Lib {
            path: "lib.rk".to_string(),
        }],
        dependencies: None,
    }
}

fn empty_module() -> Module {
    Module {
        name: None,
        top_levels: Vec::new(),
        is_inline: false,
        filepath: None,
    }
}

#[test]
fn test_crate_context_parse_rock_toml_delegates_to_shared_parser() {
    let content = r#"
[crate]
name = "test_crate"
version = "0.1.0"

[lib]
path = "src/lib.rk"
"#;

    let manifest = crate::crate_system::CrateContext::parse_rock_toml(content).unwrap();
    assert_eq!(manifest.crate_.name, "test_crate");
    assert_eq!(manifest.primary_target().path(), "src/lib.rk");
}

#[test]
fn test_crate_context_new() {
    let ctx = CrateContext::new();
    assert_eq!(ctx.crate_count(), 0);
}

#[test]
fn test_crate_context_register() {
    let mut ctx = CrateContext::new();

    let manifest = CrateManifest {
        crate_: CrateConfig {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            no_std: false,
        },
        targets: vec![CrateTarget::Lib {
            path: "lib.rk".to_string(),
        }],
        dependencies: None,
    };
    let module = Module {
        name: None,
        top_levels: vec![],
        is_inline: false,
        filepath: None,
    };

    ctx.register_crate(manifest, PathBuf::from("/test"), module);
    assert!(ctx.has_crate("test"));
}

#[test]
fn crate_context_registers_source_crates_outside_extern_store() {
    let mut ctx = CrateContext::new();
    ctx.register_crate(empty_manifest("dep"), PathBuf::from("/dep"), empty_module());

    assert!(ctx.has_crate("dep"));
    assert!(ctx.has_source_crate("dep"));
    assert!(!ctx.has_extern_crate("dep"));
    assert!(ctx.source_crate("dep").is_some());
    assert!(ctx.extern_crate("dep").is_none());
}
