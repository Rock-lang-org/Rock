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
}

impl CrateTarget {
    pub fn path(&self) -> &str {
        match self {
            Self::Bin { path, .. } | Self::Lib { path } => path,
        }
    }
}

impl CrateManifest {
    pub fn library(&self) -> Option<&CrateTarget> {
        self.targets
            .iter()
            .find(|target| matches!(target, CrateTarget::Lib { .. }))
    }

    pub fn binaries(&self) -> impl Iterator<Item = &CrateTarget> {
        self.targets
            .iter()
            .filter(|target| matches!(target, CrateTarget::Bin { .. }))
    }

    pub fn primary_target(&self) -> &CrateTarget {
        // Manifest parsing guarantees at least one target; source-only compiler
        // fixtures must uphold the same invariant when constructing manifests.
        self.library()
            .or_else(|| self.targets.first())
            .expect("manifest must have a target")
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
        targets.push(CrateTarget::Lib {
            path: target_string(table, "path", "src/lib.rk", "lib")?,
        });
    }
    if let Some(bins) = parsed.get("bin") {
        let bins = bins
            .as_array()
            .ok_or("binary targets must use [[bin]] sections")?;
        let mut names = std::collections::HashSet::new();
        for bin in bins {
            let table = bin.as_table().ok_or("each [[bin]] entry must be a table")?;
            let bin_name = target_string(table, "name", &name, "bin")?;
            if !bin_name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
            {
                return Err(
                    "bin.name must contain only ASCII letters, digits, '-' or '_'".to_string(),
                );
            }
            if matches!(bin_name.as_str(), "artifacts" | "objects" | "deps" | "bins") {
                return Err(format!(
                    "binary target name '{}' is reserved for build output directories",
                    bin_name
                ));
            }
            if !names.insert(bin_name.clone()) {
                return Err(format!(
                    "duplicate binary target '{}'; give each [[bin]] a unique name",
                    bin_name
                ));
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
    let value = table
        .get(key)
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| format!("{}.{} must be a string", section, key))
        })
        .transpose()?
        .unwrap_or(default);
    if value.is_empty() {
        return Err(format!("{}.{} must not be empty", section, key));
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests;
