use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use rock_lib::{
    diagnostic::Diagnostics,
    products::{
        ProductArtifactHeader, ProductCrateIdentity, ProductDependencyCapabilities,
        ProductDependencyIdentity, ProductDependencyLinkCapability,
    },
};

use super::artifact_validation::diagnostics_from_artifact_message;
use super::{diagnostics_from_message, parse_name_path, Config};

#[derive(Debug, Clone)]
struct ArtifactDependencyRequest {
    name: String,
    path: PathBuf,
    capabilities: ProductDependencyCapabilities,
    expected_identity: Option<ProductCrateIdentity>,
    referenced_by: Option<String>,
}

#[derive(Debug, Clone)]
struct ArtifactDependencyRecord {
    name: String,
    path: PathBuf,
    capabilities: ProductDependencyCapabilities,
    identity: ProductCrateIdentity,
}

#[derive(Default)]
struct ArtifactDependencyDiscovery {
    records: BTreeMap<String, ArtifactDependencyRecord>,
    visiting: BTreeSet<String>,
    order: Vec<String>,
}

pub(super) fn print_artifact_deps(config: &Config) -> Result<String, Diagnostics> {
    let requests = artifact_dependency_root_requests(config)?;
    let mut discovery = ArtifactDependencyDiscovery::default();
    for request in requests {
        discovery.visit(request)?;
    }

    Ok(discovery.output())
}

fn artifact_dependency_root_requests(
    config: &Config,
) -> Result<Vec<ArtifactDependencyRequest>, Diagnostics> {
    if config.extern_artifact.is_empty() {
        return Err(diagnostics_from_message(
            "--print artifact-deps requires at least one --extern-artifact",
        ));
    }

    let mut seen = BTreeSet::new();
    let mut requests = Vec::new();
    for raw in &config.extern_artifact {
        let (name, path) = parse_name_path(raw, "extern-artifact")?;
        if !seen.insert(name.clone()) {
            return Err(diagnostics_from_artifact_message(
                &path,
                format!(
                    "Duplicate external artifact crate name '{}' for --print artifact-deps",
                    name
                ),
            ));
        }
        requests.push(ArtifactDependencyRequest {
            name,
            path,
            capabilities: ProductDependencyCapabilities::artifact_object(),
            expected_identity: None,
            referenced_by: None,
        });
    }

    requests.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(requests)
}

impl ArtifactDependencyDiscovery {
    fn visit(&mut self, request: ArtifactDependencyRequest) -> Result<(), Diagnostics> {
        if let Some(existing) = self.records.get(&request.name) {
            validate_existing_artifact_dependency(&request, existing)?;
            return Ok(());
        }

        if !self.visiting.insert(request.name.clone()) {
            return Err(diagnostics_from_artifact_message(
                &request.path,
                format!("Cyclic artifact dependency involving '{}'", request.name),
            ));
        }

        let header = read_artifact_dependency_header(&request)?;
        validate_artifact_dependency_identity(&request, &header.crate_identity)?;
        let dependencies =
            artifact_dependency_requests_from_header(&request.name, &request.path, &header)?;
        for dependency in dependencies {
            self.visit(dependency)?;
        }

        self.visiting.remove(&request.name);
        self.order.push(request.name.clone());
        self.records.insert(
            request.name.clone(),
            ArtifactDependencyRecord {
                name: request.name,
                path: request.path,
                capabilities: request.capabilities,
                identity: header.crate_identity,
            },
        );

        Ok(())
    }

    fn output(&self) -> String {
        self.order
            .iter()
            .filter_map(|name| self.records.get(name))
            .map(format_artifact_dependency_record)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn read_artifact_dependency_header(
    request: &ArtifactDependencyRequest,
) -> Result<ProductArtifactHeader, Diagnostics> {
    ProductArtifactHeader::read_from_path_bounded(&request.path).map_err(|err| {
        let context = match &request.referenced_by {
            Some(parent) => format!(
                "dependency artifact '{}' referenced by '{}'",
                request.name, parent
            ),
            None => format!("external artifact '{}'", request.name),
        };
        diagnostics_from_artifact_message(
            &request.path,
            format!(
                "Failed to read {} from {}: {}",
                context,
                request.path.display(),
                err
            ),
        )
    })
}

fn validate_artifact_dependency_identity(
    request: &ArtifactDependencyRequest,
    artifact_identity: &ProductCrateIdentity,
) -> Result<(), Diagnostics> {
    if artifact_identity.name != request.name {
        return Err(diagnostics_from_artifact_message(
            &request.path,
            format!(
                "Artifact dependency '{}' at {} contains crate '{}'",
                request.name,
                request.path.display(),
                artifact_identity.name
            ),
        ));
    }

    if let Some(expected_identity) = &request.expected_identity {
        if expected_identity != artifact_identity {
            return Err(diagnostics_from_artifact_message(&request.path, format!(
                "Artifact dependency '{}' at {} has identity metadata that does not match its dependent artifact",
                request.name,
                request.path.display()
            )));
        }
    }

    Ok(())
}

fn artifact_dependency_requests_from_header(
    artifact_name: &str,
    artifact_path: &Path,
    header: &ProductArtifactHeader,
) -> Result<Vec<ArtifactDependencyRequest>, Diagnostics> {
    let expected_identities =
        artifact_dependency_identity_table_by_name(artifact_name, artifact_path, header)?;
    let dependencies = artifact_product_dependencies_by_name(artifact_name, artifact_path, header)?;
    for dependency_name in expected_identities.keys() {
        if !dependencies.contains_key(dependency_name) {
            return Err(diagnostics_from_artifact_message(artifact_path, format!(
                "Artifact '{}' records dependency '{}' in its identity table without an artifact path",
                artifact_name, dependency_name
            )));
        }
    }

    let mut requests = Vec::new();
    for (dependency_name, dependency) in dependencies {
        let Some(expected_identity) = expected_identities.get(&dependency_name) else {
            return Err(diagnostics_from_artifact_message(
                artifact_path,
                format!(
                    "Artifact '{}' records dependency '{}' without matching identity metadata",
                    artifact_name, dependency_name
                ),
            ));
        };

        requests.push(ArtifactDependencyRequest {
            name: dependency.name,
            path: resolve_artifact_dependency_path(artifact_path, dependency.artifact_path),
            capabilities: dependency.capabilities,
            expected_identity: Some(expected_identity.clone()),
            referenced_by: Some(artifact_name.to_string()),
        });
    }

    Ok(requests)
}

fn artifact_dependency_identity_table_by_name(
    artifact_name: &str,
    artifact_path: &Path,
    header: &ProductArtifactHeader,
) -> Result<BTreeMap<String, ProductCrateIdentity>, Diagnostics> {
    let mut identities = BTreeMap::new();
    for identity in header.identity_dependencies.values() {
        if identities
            .insert(identity.name.clone(), identity.clone())
            .is_some()
        {
            return Err(diagnostics_from_artifact_message(
                artifact_path,
                format!(
                    "Artifact '{}' records duplicate dependency identity for crate '{}'",
                    artifact_name, identity.name
                ),
            ));
        }
    }

    Ok(identities)
}

fn artifact_product_dependencies_by_name(
    artifact_name: &str,
    artifact_path: &Path,
    header: &ProductArtifactHeader,
) -> Result<BTreeMap<String, ProductDependencyIdentity>, Diagnostics> {
    let mut dependencies = BTreeMap::new();
    for dependency in &header.dependencies {
        if dependencies
            .insert(dependency.name.clone(), dependency.clone())
            .is_some()
        {
            return Err(diagnostics_from_artifact_message(
                artifact_path,
                format!(
                    "Artifact '{}' records duplicate dependency artifact path for crate '{}'",
                    artifact_name, dependency.name
                ),
            ));
        }
    }

    Ok(dependencies)
}

fn resolve_artifact_dependency_path(artifact_path: &Path, dependency_path: PathBuf) -> PathBuf {
    if dependency_path.is_absolute() {
        return dependency_path;
    }

    artifact_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(dependency_path)
}

fn validate_existing_artifact_dependency(
    request: &ArtifactDependencyRequest,
    existing: &ArtifactDependencyRecord,
) -> Result<(), Diagnostics> {
    if !artifact_paths_match(&existing.path, &request.path) {
        return Err(diagnostics_from_artifact_message(
            &request.path,
            format!(
                "Conflicting artifact dependency paths for crate '{}': {} and {}",
                request.name,
                existing.path.display(),
                request.path.display()
            ),
        ));
    }

    if existing.capabilities != request.capabilities {
        return Err(diagnostics_from_artifact_message(
            &request.path,
            format!(
                "Conflicting artifact dependency capabilities for crate '{}' at {}",
                request.name,
                request.path.display()
            ),
        ));
    }

    if let Some(expected_identity) = &request.expected_identity {
        if expected_identity != &existing.identity {
            return Err(diagnostics_from_artifact_message(
                &request.path,
                format!(
                    "Conflicting artifact dependency identity metadata for crate '{}' at {}",
                    request.name,
                    request.path.display()
                ),
            ));
        }
    }

    Ok(())
}

fn artifact_paths_match(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn format_artifact_dependency_record(record: &ArtifactDependencyRecord) -> String {
    format!(
        "{}={} metadata={} bodies={} link={}",
        record.name,
        record.path.display(),
        record.capabilities.metadata,
        record.capabilities.bodies,
        format_artifact_dependency_link_capability(&record.capabilities.link)
    )
}

fn format_artifact_dependency_link_capability(
    capability: &ProductDependencyLinkCapability,
) -> &'static str {
    match capability {
        ProductDependencyLinkCapability::Object => "object",
        ProductDependencyLinkCapability::MetadataOnly => "metadata-only",
    }
}
