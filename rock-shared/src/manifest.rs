use std::{collections::BTreeMap, path::Path};

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub path: Option<String>,
    pub version: Option<String>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateManifest {
    pub crate_: CrateConfig,
    pub targets: Vec<CrateTarget>,
    pub dependencies: Option<BTreeMap<String, Dependency>>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateConfig {
    pub name: String,
    pub version: String,
    pub no_std: bool,
}

/// A library or named binary declared by a package manifest.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrateTarget {
    Bin { name: String, path: String },
    Lib { path: String },
    ProcMacro { path: String },
}

/// The product role is independent of the source path or package spelling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TargetRole {
    #[default]
    Binary,
    Library,
    ProcMacro,
}

impl CrateTarget {
    pub fn path(&self) -> &str {
        match self {
            Self::Bin { path, .. } | Self::Lib { path } | Self::ProcMacro { path } => path,
        }
    }

    pub fn role(&self) -> TargetRole {
        match self {
            Self::Bin { .. } => TargetRole::Binary,
            Self::Lib { .. } => TargetRole::Library,
            Self::ProcMacro { .. } => TargetRole::ProcMacro,
        }
    }
}

impl CrateManifest {
    pub fn library(&self) -> Option<&CrateTarget> {
        self.targets.iter().find(|target| matches!(target, CrateTarget::Lib { .. } | CrateTarget::ProcMacro { .. }))
    }

    pub fn binaries(&self) -> impl Iterator<Item = &CrateTarget> {
        self.targets.iter().filter(|target| matches!(target, CrateTarget::Bin { .. }))
    }

    pub fn primary_target(&self) -> &CrateTarget {
        // Manifest parsing guarantees at least one target; source-only compiler
        // fixtures must uphold the same invariant when constructing manifests.
        self.library().or_else(|| self.targets.first()).expect("manifest must have a target")
    }
}

pub fn load_manifest(manifest_path: &Path) -> Result<CrateManifest, String> {
    let content = std::fs::read_to_string(manifest_path)
        .map_err(|e| format!("Failed to read rock.toml: {}", e))?;

    parse_rock_toml(&content)
}

pub fn parse_rock_toml(content: &str) -> Result<CrateManifest, String> {
    let parsed: toml::Value = content
        .parse()
        .map_err(|e| format!("Failed to parse rock.toml: {}", e))?;

    let crate_table = parsed
        .get("crate")
        .ok_or("rock.toml missing [crate] section")?
        .as_table()
        .ok_or("[crate] section must be a table")?;

    let name = crate_table
        .get("name")
        .ok_or("rock.toml missing crate.name")?
        .as_str()
        .ok_or("crate.name must be a string")?
        .to_string();

    let version = crate_table
        .get("version")
        .ok_or("rock.toml missing crate.version")?
        .as_str()
        .ok_or("crate.version must be a string")?
        .to_string();

    let no_std = crate_table
        .get("no_std")
        .map(|value| value.as_bool().ok_or("crate.no_std must be a boolean"))
        .transpose()?
        .unwrap_or(false);

    let mut targets = Vec::new();
    if let Some(lib) = parsed.get("lib") {
        let table = lib.as_table().ok_or("[lib] section must be a table")?;
        let proc_macro = table.get("proc_macro")
            .map(|value| value.as_bool().ok_or("lib.proc_macro must be a boolean"))
            .transpose()?
            .unwrap_or(false);
        let path = target_string(table, "path", "src/lib.rk", "lib")?;
        targets.push(if proc_macro {
            CrateTarget::ProcMacro { path }
        } else {
            CrateTarget::Lib { path }
        });
    }
    if let Some(bins) = parsed.get("bin") {
        let bins = bins.as_array().ok_or("binary targets must use [[bin]] sections")?;
        let mut names = std::collections::HashSet::new();
        for bin in bins {
            let table = bin.as_table().ok_or("each [[bin]] entry must be a table")?;
            if table.contains_key("proc_macro") {
                return Err("proc_macro is only allowed in [lib], not [[bin]]".to_string());
            }
            let bin_name = target_string(table, "name", &name, "bin")?;
            if !bin_name.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')) {
                return Err("bin.name must contain only ASCII letters, digits, '-' or '_'".to_string());
            }
            if matches!(bin_name.as_str(), "artifacts" | "objects" | "deps" | "bins") {
                return Err(format!("binary target name '{}' is reserved for build output directories", bin_name));
            }
            if !names.insert(bin_name.clone()) {
                return Err(format!("duplicate binary target '{}'; give each [[bin]] a unique name", bin_name));
            }
            let default_path = if bin_name == name {
                "src/main.rk".to_string()
            } else {
                format!("src/bin/{}.rk", bin_name)
            };
            targets.push(CrateTarget::Bin {
                name: bin_name,
                path: target_string(table, "path", &default_path, "bin")?,
            });
        }
    }
    if targets.is_empty() {
        return Err("rock.toml requires a [lib] or at least one [[bin]] target".to_string());
    }
    if targets.iter().any(|target| target.role() == TargetRole::ProcMacro)
        && targets.iter().any(|target| target.role() == TargetRole::Binary)
    {
        return Err("a [lib] with proc_macro = true cannot be combined with [[bin]] targets".to_string());
    }

    let dependencies = if let Some(deps_table) = parsed.get("dependencies") {
        let deps = deps_table
            .as_table()
            .ok_or("[dependencies] must be a table")?;
        let mut dep_map = BTreeMap::new();
        for (dep_name, dep_value) in deps {
            let dep = dep_value
                .as_table()
                .ok_or_else(|| format!("Dependency '{}' must be a table", dep_name))?;
            let path = dep.get("path").and_then(|v| v.as_str()).map(str::to_string);
            let version = dep
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            if path.is_none() && version.is_none() {
                return Err(format!(
                    "Dependency '{}' must have at least 'path' or 'version'",
                    dep_name
                ));
            }
            dep_map.insert(dep_name.clone(), Dependency { path, version });
        }
        Some(dep_map)
    } else {
        None
    };

    Ok(CrateManifest {
        crate_: CrateConfig {
            name,
            version,
            no_std,
        },
        targets,
        dependencies,
    })
}

fn target_string(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    default: &str,
    section: &str,
) -> Result<String, String> {
    let value = table.get(key)
        .map(|value| value.as_str().ok_or_else(|| format!("{}.{} must be a string", section, key)))
        .transpose()?
        .unwrap_or(default);
    if value.is_empty() {
        return Err(format!("{}.{} must not be empty", section, key));
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CRATE_HEADER: &str = "[crate]\nname = \"example\"\nversion = \"0.1.0\"\n";

    #[test]
    fn proc_library_retains_ordinary_dependency_and_entry_metadata() {
        for (setting, path) in [("", "src/lib.rk"), ("path = \"macros.rk\"\n", "macros.rk")] {
            let manifest = parse_rock_toml(&format!(
                "{CRATE_HEADER}\n[lib]\nproc_macro = true\n{setting}\n[dependencies]\nsdk = {{ path = \"../sdk\" }}\n",
            )).unwrap();
            assert_eq!(manifest.targets, vec![CrateTarget::ProcMacro { path: path.to_string() }]);
            assert_eq!(manifest.primary_target().role(), TargetRole::ProcMacro);
            assert_eq!(manifest.library(), Some(manifest.primary_target()));
            assert_eq!(manifest.primary_target().path(), path);
            assert_eq!(manifest.binaries().count(), 0);
            assert_eq!(manifest.dependencies.unwrap()["sdk"].path.as_deref(), Some("../sdk"));
        }
    }

    #[test]
    fn omitted_and_false_proc_flags_preserve_ordinary_targets() {
        let ordinary = parse_rock_toml(&format!("{CRATE_HEADER}\n[lib]\n[[bin]]\n")).unwrap();
        let explicit = parse_rock_toml(&format!("{CRATE_HEADER}\n[lib]\nproc_macro = false\n[[bin]]\n")).unwrap();
        assert_eq!(ordinary, explicit);
        assert_eq!(ordinary.library().unwrap().role(), TargetRole::Library);
        assert_eq!(ordinary.binaries().next().unwrap().role(), TargetRole::Binary);
    }

    #[test]
    fn proc_flag_requires_a_boolean_in_the_library_table() {
        for value in ["\"true\"", "1", "0", "[]", "[true]", "{}", "{ enabled = true }", "1.0"] {
            let error = parse_rock_toml(&format!("{CRATE_HEADER}\n[lib]\nproc_macro = {value}\n")).unwrap_err();
            assert!(error.contains("lib.proc_macro must be a boolean"), "{value}: {error}");
        }
        for value in ["true", "false", "\"true\""] {
            let error = parse_rock_toml(&format!("{CRATE_HEADER}\n[[bin]]\nproc_macro = {value}\n")).unwrap_err();
            assert!(error.contains("only allowed in [lib]"), "{error}");
        }
    }

    #[test]
    fn proc_libraries_cannot_declare_binaries() {
        for targets in [
            "[lib]\nproc_macro = true\n[[bin]]\n",
            "[[bin]]\nname = \"helper\"\n[lib]\nproc_macro = true\n",
            "[lib]\nproc_macro = true\n[[bin]]\nname = \"one\"\n[[bin]]\nname = \"two\"\n",
        ] {
            let error = parse_rock_toml(&format!("{CRATE_HEADER}\n{targets}")).unwrap_err();
            assert!(error.contains("cannot be combined with [[bin]]"), "{error}");
        }
        let empty = parse_rock_toml(&format!("bin = []\n{CRATE_HEADER}\n[lib]\nproc_macro = true\n")).unwrap();
        assert_eq!(empty.primary_target().role(), TargetRole::ProcMacro);
    }

    #[test]
    fn test_binary_and_library_default_entries() {
        for (section, expected) in [
            ("[[bin]]", CrateTarget::Bin { name: "example".to_string(), path: "src/main.rk".to_string() }),
            ("[lib]", CrateTarget::Lib { path: "src/lib.rk".to_string() }),
        ] {
            let manifest = parse_rock_toml(&format!("{}\n{}\n", CRATE_HEADER, section)).unwrap();
            assert_eq!(manifest.targets, vec![expected]);
        }
    }

    #[test]
    fn test_binary_custom_entry() {
        let manifest = parse_rock_toml(&format!("{}\n[[bin]]\npath = \"main.rk\"\n", CRATE_HEADER)).unwrap();
        assert_eq!(manifest.targets, vec![CrateTarget::Bin { name: "example".to_string(), path: "main.rk".to_string() }]);
    }

    #[test]
    fn test_manifest_requires_at_least_one_target() {
        for suffix in ["", "\nbin = []\n"] {
            let error = parse_rock_toml(&format!("{}{}", suffix, CRATE_HEADER)).unwrap_err();
            assert!(error.contains("requires a [lib] or at least one [[bin]]"), "{error}");
        }
    }

    #[test]
    fn test_manifest_rejects_invalid_target_configuration() {
        for (section, header) in [("bin", "[[bin]]"), ("lib", "[lib]")] {
            for (suffix, message) in [
                (format!("{}\npath = 42\n", header), format!("{}.path must be a string", section)),
                (format!("{}\npath = \"\"\n", header), format!("{}.path must not be empty", section)),
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
        assert_eq!(manifest.targets, vec![
            CrateTarget::Lib { path: "src/lib.rk".to_string() },
            CrateTarget::Bin { name: "server".to_string(), path: "src/bin/server.rk".to_string() },
            CrateTarget::Bin { name: "client".to_string(), path: "tools/main.rk".to_string() },
        ]);
        assert_eq!(manifest.binaries().count(), 2);
    }

    #[test]
    fn test_invalid_or_duplicate_binary_names_are_rejected() {
        for (suffix, message) in [
            ("[[bin]]\nname = \"\"\n", "bin.name must not be empty"),
            ("[[bin]]\nname = 42\n", "bin.name must be a string"),
            ("[[bin]]\nname = \"../escape\"\n", "bin.name must contain only"),
            ("[[bin]]\nname = \"deps\"\n", "reserved for build output directories"),
            ("[[bin]]\n[[bin]]\n", "duplicate binary target"),
            ("[[bin]]\nname = \"same\"\n[[bin]]\nname = \"same\"\n", "duplicate binary target"),
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
        assert_eq!(manifest.targets, vec![CrateTarget::Lib { path: "lib.rk".to_string() }]);
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
}
