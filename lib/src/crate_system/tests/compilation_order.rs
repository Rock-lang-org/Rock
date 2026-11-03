use super::*;

#[test]
fn test_compilation_order_no_deps() {
    let mut ctx = CrateContext::new();

    let manifest = CrateManifest {
        crate_: CrateConfig {
            name: "crate_a".to_string(),
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

    ctx.register_crate(manifest, PathBuf::from("/a"), module);
    let order = ctx.compilation_order().unwrap();
    assert_eq!(order, vec!["crate_a".to_string()]);
}

#[test]
fn test_compilation_order_with_deps() {
    let mut ctx = CrateContext::new();

    let manifest_a = CrateManifest {
        crate_: CrateConfig {
            name: "crate_a".to_string(),
            version: "0.1.0".to_string(),
            no_std: false,
        },
        targets: vec![CrateTarget::Lib {
            path: "lib.rk".to_string(),
        }],
        dependencies: None,
    };
    let module_a = Module {
        name: None,
        top_levels: vec![],
        is_inline: false,
        filepath: None,
    };
    ctx.register_crate(manifest_a, PathBuf::from("/a"), module_a);

    let manifest_b = CrateManifest {
        crate_: CrateConfig {
            name: "crate_b".to_string(),
            version: "0.1.0".to_string(),
            no_std: false,
        },
        targets: vec![CrateTarget::Lib {
            path: "lib.rk".to_string(),
        }],
        dependencies: {
            let mut deps = BTreeMap::new();
            deps.insert(
                "crate_a".to_string(),
                Dependency {
                    path: Some("../crate_a".to_string()),
                    version: None,
                },
            );
            Some(deps)
        },
    };
    let module_b = Module {
        name: None,
        top_levels: vec![],
        is_inline: false,
        filepath: None,
    };
    ctx.register_crate(manifest_b, PathBuf::from("/b"), module_b);

    let order = ctx.compilation_order().unwrap();
    assert_eq!(order, vec!["crate_a".to_string(), "crate_b".to_string()]);
}
