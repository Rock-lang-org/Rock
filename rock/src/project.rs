use std::path::{Path, PathBuf};

use rock_shared::manifest::CrateTarget;

use crate::{
    artifact::{collect_root_artifacts, ensure_artifact, ArtifactBuildState},
    bundled_sysroot::STD_CRATE_NAME,
    package::Package,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisProject {
    pub root_dir: PathBuf,
    pub entry_file: PathBuf,
    pub current_crate_name: Option<String>,
    pub extern_artifacts: Vec<(String, PathBuf)>,
    pub no_prelude: bool,
    pub no_std: bool,
}

pub fn find_project_root(source_path: &Path) -> Option<PathBuf> {
    let start = if source_path.is_dir() {
        source_path
    } else {
        source_path.parent()?
    };
    start
        .ancestors()
        .find(|ancestor| ancestor.join("rock.toml").is_file())
        .map(Path::to_path_buf)
}

pub fn resolve_analysis_project(project_root: &Path) -> Result<AnalysisProject, String> {
    resolve_project(project_root, None)
}

/// Select an explicitly declared entry when that entry is opened in the editor.
pub fn resolve_analysis_project_for_source(
    project_root: &Path,
    source_path: &Path,
) -> Result<AnalysisProject, String> {
    resolve_project(project_root, Some(source_path))
}

fn resolve_project(
    project_root: &Path,
    source_path: Option<&Path>,
) -> Result<AnalysisProject, String> {
    let mut package = Package::load(project_root.to_path_buf())?;
    if let Some(source_path) = source_path {
        let source_path = source_path
            .canonicalize()
            .unwrap_or_else(|_| source_path.to_path_buf());
        if let Some(target) = package.manifest.targets.iter().find(|target| {
            let entry = package.root_dir.join(target.path());
            entry.canonicalize().unwrap_or(entry) == source_path
        }) {
            package.target = target.clone();
        }
    }
    let mut state = ArtifactBuildState::new(&package);
    let mut artifacts = collect_root_artifacts(&package, &mut state)?;
    if matches!(package.target, CrateTarget::Bin { .. }) && package.manifest.library().is_some() {
        artifacts.push(crate::rockc::ExternArtifact {
            name: package.manifest.crate_.name.clone(),
            path: ensure_artifact(&package.root_dir, &mut state)?,
        });
    }
    let crate_name = package.manifest.crate_.name.clone();
    let no_std = package.manifest.crate_.no_std;
    // Match binary builds: the package name belongs to the reusable library,
    // not to the binary's local source crate, which may import that library.
    let current_crate_name = match package.target {
        CrateTarget::Lib { .. } => Some(crate_name.clone()),
        CrateTarget::Bin { .. } => None,
    };

    Ok(AnalysisProject {
        root_dir: package.root_dir.clone(),
        entry_file: package.entry_file(),
        no_prelude: no_std || crate_name == STD_CRATE_NAME,
        no_std,
        current_crate_name,
        extern_artifacts: artifacts
            .into_iter()
            .map(|artifact| (artifact.name, artifact.path))
            .collect(),
    })
}

#[cfg(test)]
mod tests;
