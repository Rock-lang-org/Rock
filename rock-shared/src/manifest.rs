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
    pub lib: LibConfig,
    pub dependencies: Option<BTreeMap<String, Dependency>>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateConfig {
    pub name: String,
    pub version: String,
    pub no_std: bool,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibConfig {
    pub path: String,
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

    let lib_table = parsed
        .get("lib")
        .ok_or("rock.toml missing [lib] section")?
        .as_table()
        .ok_or("[lib] section must be a table")?;

    let lib_path = lib_table
        .get("path")
        .ok_or("rock.toml missing lib.path")?
        .as_str()
        .ok_or("lib.path must be a string")?
        .to_string();

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
        lib: LibConfig { path: lib_path },
        dependencies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(manifest.lib.path, "lib.rk");
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
