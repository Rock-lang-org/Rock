use super::*;

const CRATE_HEADER: &str = "[crate]\nname = \"example\"\nversion = \"0.1.0\"\n";

#[test]
fn test_binary_and_library_default_entries() {
    for (section, expected) in [
        (
            "[[bin]]",
            CrateTarget::Bin {
                name: "example".to_string(),
                path: "src/main.rk".to_string(),
            },
        ),
        (
            "[lib]",
            CrateTarget::Lib {
                path: "src/lib.rk".to_string(),
            },
        ),
    ] {
        let manifest = parse_rock_toml(&format!("{}\n{}\n", CRATE_HEADER, section)).unwrap();
        assert_eq!(manifest.targets, vec![expected]);
    }
}

#[test]
fn test_binary_custom_entry() {
    let manifest =
        parse_rock_toml(&format!("{}\n[[bin]]\npath = \"main.rk\"\n", CRATE_HEADER)).unwrap();
    assert_eq!(
        manifest.targets,
        vec![CrateTarget::Bin {
            name: "example".to_string(),
            path: "main.rk".to_string()
        }]
    );
}

#[test]
fn test_manifest_requires_at_least_one_target() {
    for suffix in ["", "\nbin = []\n"] {
        let error = parse_rock_toml(&format!("{}{}", suffix, CRATE_HEADER)).unwrap_err();
        assert!(
            error.contains("requires a [lib] or at least one [[bin]]"),
            "{error}"
        );
    }
}

#[test]
fn test_manifest_rejects_invalid_target_configuration() {
    for (section, header) in [("bin", "[[bin]]"), ("lib", "[lib]")] {
        for (suffix, message) in [
            (
                format!("{}\npath = 42\n", header),
                format!("{}.path must be a string", section),
            ),
            (
                format!("{}\npath = \"\"\n", header),
                format!("{}.path must not be empty", section),
            ),
        ] {
            let error = parse_rock_toml(&format!("{}\n{}", suffix, CRATE_HEADER)).unwrap_err();
            assert!(error.contains(&message), "{error}");
        }
    }
}

#[test]
fn test_library_and_multiple_named_binaries() {
    let manifest = parse_rock_toml(&format!(
            "{}\n[lib]\n[[bin]]\nname = \"server\"\n[[bin]]\nname = \"client\"\npath = \"tools/main.rk\"\n", CRATE_HEADER,
        )).unwrap();
    assert_eq!(
        manifest.targets,
        vec![
            CrateTarget::Lib {
                path: "src/lib.rk".to_string()
            },
            CrateTarget::Bin {
                name: "server".to_string(),
                path: "src/bin/server.rk".to_string()
            },
            CrateTarget::Bin {
                name: "client".to_string(),
                path: "tools/main.rk".to_string()
            },
        ]
    );
    assert_eq!(manifest.binaries().count(), 2);
}

#[test]
fn test_invalid_or_duplicate_binary_names_are_rejected() {
    for (suffix, message) in [
        ("[[bin]]\nname = \"\"\n", "bin.name must not be empty"),
        ("[[bin]]\nname = 42\n", "bin.name must be a string"),
        (
            "[[bin]]\nname = \"../escape\"\n",
            "bin.name must contain only",
        ),
        (
            "[[bin]]\nname = \"deps\"\n",
            "reserved for build output directories",
        ),
        ("[[bin]]\n[[bin]]\n", "duplicate binary target"),
        (
            "[[bin]]\nname = \"same\"\n[[bin]]\nname = \"same\"\n",
            "duplicate binary target",
        ),
        ("[bin]\n", "must use [[bin]]"),
        ("[[lib]]\n", "[lib] section must be a table"),
        ("[lib]\n[lib]\n", "Failed to parse"),
    ] {
        let error = parse_rock_toml(&format!("{}\n{}", CRATE_HEADER, suffix)).unwrap_err();
        assert!(error.contains(message), "{error}");
    }
}

#[test]
fn test_parse_rock_toml() {
    let content = r#"
[crate]
name = "std"
version = "0.1.0"

[lib]
path = "lib.rk"
"#;

    let manifest = parse_rock_toml(content).unwrap();
    assert_eq!(manifest.crate_.name, "std");
    assert_eq!(manifest.crate_.version, "0.1.0");
    assert!(!manifest.crate_.no_std);
    assert_eq!(
        manifest.targets,
        vec![CrateTarget::Lib {
            path: "lib.rk".to_string()
        }]
    );
}

#[test]
fn test_parse_rock_toml_no_std() {
    let content = r#"
[crate]
name = "nostd"
version = "0.1.0"
no_std = true

[lib]
path = "lib.rk"
"#;

    let manifest = parse_rock_toml(content).unwrap();
    assert!(manifest.crate_.no_std);
}

#[test]
fn test_parse_rock_toml_missing_section() {
    let content = r#"
[crate]
name = "std"
"#;

    let result = parse_rock_toml(content);
    assert!(result.is_err());
}

#[test]
fn test_parse_dependencies() {
    let content = r#"
[crate]
name = "myapp"
version = "0.1.0"

[lib]
path = "lib.rk"

[dependencies]
regex = { path = "../regex" }
serde = { version = "1.0" }
"#;

    let manifest = parse_rock_toml(content).unwrap();
    assert_eq!(manifest.crate_.name, "myapp");
    assert!(manifest.dependencies.is_some());
    let deps = manifest.dependencies.unwrap();
    assert_eq!(deps.len(), 2);
    assert_eq!(deps["regex"].path, Some("../regex".to_string()));
    assert_eq!(deps["serde"].version, Some("1.0".to_string()));
}

#[test]
fn test_parse_no_dependencies() {
    let content = r#"
[crate]
name = "noproj"
version = "0.1.0"

[lib]
path = "lib.rk"
"#;

    let manifest = parse_rock_toml(content).unwrap();
    assert_eq!(manifest.crate_.name, "noproj");
    assert!(manifest.dependencies.is_none());
}
