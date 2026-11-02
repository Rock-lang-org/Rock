use std::collections::BTreeSet;
use std::path::PathBuf;

use super::CrateContext;
use crate::diagnostic::{self, Diagnostics};
use crate::products::ProductCrateIdentity;
use crate::Config;

struct PendingExternArtifact {
    name: String,
    path: PathBuf,
    header: crate::products::ProductArtifactHeader,
    identity: ProductCrateIdentity,
    dependencies: Vec<ProductCrateIdentity>,
}

pub(crate) fn load_extern_artifacts(
    ctx: &mut CrateContext,
    extern_artifacts: &[(String, PathBuf)],
    project_path: PathBuf,
) -> Result<(), diagnostic::Diagnostic> {
    let mut pending = Vec::new();
    for (name, path) in extern_artifacts {
        let header =
            crate::products::ProductArtifactHeader::read_from_path_bounded(path).map_err(|e| {
                diagnostic::Diagnostic::for_artifact(
                    format!(
                        "Failed to read external artifact header '{}' from {}: {}",
                        name,
                        path.display(),
                        e
                    ),
                    path.clone(),
                )
            })?;
        pending.push(PendingExternArtifact {
            name: name.clone(),
            path: path.clone(),
            identity: header.crate_identity.clone(),
            dependencies: header.identity_dependencies.values().cloned().collect(),
            header,
        });
    }

    let mut loaded_identities: BTreeSet<ProductCrateIdentity> =
        ctx.product_crate_ids.keys().cloned().collect();
    let pending_identities: BTreeSet<ProductCrateIdentity> = pending
        .iter()
        .map(|artifact| artifact.identity.clone())
        .collect();
    for artifact in &pending {
        for dependency in &artifact.dependencies {
            if !loaded_identities.contains(dependency) && !pending_identities.contains(dependency) {
                return Err(diagnostic::Diagnostic::for_artifact(
                    format!(
                        "External artifact '{}' depends on missing external artifact '{}'",
                        artifact.name, dependency.name
                    ),
                    artifact.path.clone(),
                ));
            }
        }
    }

    while !pending.is_empty() {
        let Some(index) = pending.iter().position(|artifact| {
            artifact
                .dependencies
                .iter()
                .all(|dependency| loaded_identities.contains(dependency))
        }) else {
            let names = pending
                .iter()
                .map(|artifact| artifact.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(diagnostic::Diagnostic::for_project(
                format!(
                    "Failed to resolve external artifact load order for: {}",
                    names
                ),
                project_path,
            ));
        };

        let artifact = pending.remove(index);
        let mut type_context = crate::type_context::TypeContext::new();
        ctx.load_product_artifact_from_header_as_with_type_context(
            &artifact.name,
            artifact.path.clone(),
            &artifact.header,
            &mut type_context,
        )
        .map_err(|e| {
            diagnostic::Diagnostic::for_artifact(
                format!(
                    "Failed to load external artifact '{}' from {}: {}",
                    artifact.name,
                    artifact.path.display(),
                    e
                ),
                artifact.path.clone(),
            )
        })?;
        loaded_identities.insert(artifact.identity);
    }

    Ok(())
}

pub(crate) fn validate_extern_artifact_names(
    config: &Config,
    effective_current_crate_name: &str,
) -> Result<(), Diagnostics> {
    let mut seen = std::collections::BTreeSet::new();
    for (name, _) in &config.extern_artifacts {
        if effective_current_crate_name == name {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(diagnostic::Diagnostic::for_project(
                format!(
                    "External artifact crate name '{}' conflicts with current crate name '{}'",
                    name, name
                ),
                config.entry_file.clone(),
            ));
            return Err(diagnostics);
        }
        if !seen.insert(name.clone()) {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(diagnostic::Diagnostic::for_project(
                format!("Duplicate external artifact crate name '{}'", name),
                config.entry_file.clone(),
            ));
            return Err(diagnostics);
        }
    }

    Ok(())
}
