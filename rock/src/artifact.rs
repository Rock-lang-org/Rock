use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use rock_shared::fs::{collect_package_source_inputs, file_modified};

use crate::{
    bundled_sysroot::{
        ensure_sysroot_std_available, package_requires_sysroot_std, STD_CRATE_NAME,
    },
    deps::resolve_dependency_root,
    package::{BuildProfile, Package},
    progress,
    rockc::{
        build_dependency_artifact_invocation, resolve_rockc_path, run_invocation, ExternArtifact,
    },
};

pub(crate) struct ArtifactBuildState {
    profile: BuildProfile,
    no_proc_execution: bool,
    project_root: PathBuf,
    artifacts: HashMap<PathBuf, PathBuf>,
    artifact_dependencies: HashMap<PathBuf, Vec<(String, PathBuf)>>,
    uses_std: HashMap<PathBuf, bool>,
    stack: Vec<PathBuf>,
    pub(crate) progress: progress::BuildProgress,
}

impl ArtifactBuildState {
    pub(crate) fn new(package: &Package) -> Self {
        Self {
            profile: package.profile,
            no_proc_execution: package.no_proc_execution,
            project_root: package.root_dir.clone(),
            artifacts: HashMap::new(),
            artifact_dependencies: HashMap::new(),
            uses_std: HashMap::new(),
            stack: Vec::new(),
            progress: progress::BuildProgress::default(),
        }
    }

    pub(crate) fn for_targets(package: &Package, root_targets: usize) -> Result<Self, String> {
        let mut roots = HashSet::new();
        collect_package_roots(package, &mut roots)?;
        Ok(Self {
            progress: progress::BuildProgress::new(roots.len() - 1 + root_targets),
            ..Self::new(package)
        })
    }

    pub(crate) fn dependency_artifacts_use_std(
        &self,
        dependency_artifacts: &[(String, PathBuf)],
    ) -> bool {
        dependency_artifacts.iter().any(|(name, artifact_path)| {
            name != STD_CRATE_NAME
                && self
                    .uses_std
                    .get(artifact_path)
                    .copied()
                    .unwrap_or(false)
        })
    }
}

fn collect_package_roots(package: &Package, roots: &mut HashSet<PathBuf>) -> Result<(), String> {
    roots.insert(package.root_dir.clone());
    if let Some(dependencies) = &package.manifest.dependencies {
        for (name, dependency) in dependencies {
            let root = resolve_dependency_root(&package.root_dir, name, dependency)?;
            if !roots.contains(&root) {
                collect_package_roots(&Package::load(root)?, roots)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn collect_dependency_artifacts(
    package: &Package,
    state: &mut ArtifactBuildState,
) -> Result<Vec<(String, PathBuf)>, String> {
    let mut artifacts = BTreeMap::new();
    let mut visited = HashSet::new();
    collect_dependency_artifacts_into(package, state, &mut visited, &mut artifacts)?;

    Ok(artifacts.into_iter().collect())
}

pub(crate) fn collect_root_artifacts(
    package: &Package,
    state: &mut ArtifactBuildState,
) -> Result<Vec<ExternArtifact>, String> {
    let dependency_artifacts = collect_dependency_artifacts(package, state)?;
    let mut artifacts = dependency_artifacts
        .iter()
        .map(|(name, path)| ExternArtifact {
            name: name.clone(),
            path: path.clone(),
        })
        .collect::<Vec<_>>();

    if package_requires_sysroot_std(package) {
        let layout = ensure_sysroot_std_available(package.no_proc_execution)?;
        artifacts.push(ExternArtifact {
            name: STD_CRATE_NAME.to_string(),
            path: layout.std_artifact,
        });
    }
    if state.dependency_artifacts_use_std(&dependency_artifacts) {
        let std_artifact = ensure_sysroot_std_available(package.no_proc_execution)?.std_artifact;
        if !artifacts
            .iter()
            .any(|artifact| artifact.path == std_artifact)
        {
            artifacts.push(ExternArtifact {
                name: STD_CRATE_NAME.to_string(),
                path: std_artifact,
            });
        }
    }

    Ok(artifacts)
}

fn collect_dependency_artifacts_into(
    package: &Package,
    state: &mut ArtifactBuildState,
    visited: &mut HashSet<PathBuf>,
    artifacts: &mut BTreeMap<String, PathBuf>,
) -> Result<(), String> {
    let Some(dependencies) = &package.manifest.dependencies else {
        return Ok(());
    };

    for (dep_name, dependency) in dependencies {
        let dep_root = resolve_dependency_root(&package.root_dir, dep_name, dependency)?;
        let dep_root = dep_root
            .canonicalize()
            .map_err(|e| format!("Failed to resolve crate root {}: {}", dep_root.display(), e))?;
        if !visited.insert(dep_root.clone()) {
            continue;
        }

        let dep_package = Package::load(dep_root.clone())?;
        let artifact_path = ensure_artifact(&dep_root, state)?;
        artifacts.insert(dep_package.manifest.crate_.name.clone(), artifact_path);
        collect_dependency_artifacts_into(&dep_package, state, visited, artifacts)?;
    }

    Ok(())
}

pub(crate) fn ensure_artifact(
    crate_root: &Path,
    state: &mut ArtifactBuildState,
) -> Result<PathBuf, String> {
    let canonical_root = crate_root.canonicalize().map_err(|e| {
        format!(
            "Failed to resolve crate root {}: {}",
            crate_root.display(),
            e
        )
    })?;

    if let Some(existing) = state.artifacts.get(&canonical_root) {
        return Ok(existing.clone());
    }

    if let Some(position) = state.stack.iter().position(|path| path == &canonical_root) {
        let cycle = state.stack[position..]
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(format!("Circular dependency detected: {}", cycle));
    }

    state.stack.push(canonical_root.clone());

    let result = (|| {
        let mut package = Package::load(canonical_root.clone())?;
        let Some(library) = package.manifest.library() else {
            return Err(format!(
                "Package '{}' has no library target and cannot be used as a dependency; add a [lib] target",
                package.manifest.crate_.name
            ));
        };
        package.target = library.clone();
        package.require_runtime_target()?;
        package.profile = state.profile;
        package.no_proc_execution = state.no_proc_execution;
        package.use_project_build(&state.project_root);
        let mut implicit_artifact_inputs = if package_requires_sysroot_std(&package) {
            vec![ensure_sysroot_std_available(package.no_proc_execution)?.std_artifact.clone()]
        } else {
            Vec::new()
        };
        let mut dependency_artifacts = Vec::new();
        let mut dependencies_use_std = false;
        if let Some(dependencies) = &package.manifest.dependencies {
            for (dep_name, dependency) in dependencies {
                let dep_root = resolve_dependency_root(&package.root_dir, dep_name, dependency)?;
                let artifact = ensure_artifact(&dep_root, state)?;
                if dep_name != STD_CRATE_NAME {
                    dependencies_use_std |=
                        state.uses_std.get(&artifact).copied().unwrap_or(false);
                }
                // Loaded dependencies also require their complete artifact closure.
                // Retain it in the build state so cached sources need not be re-read.
                if let Some(transitive) = state.artifact_dependencies.get(&artifact) {
                    dependency_artifacts.extend(transitive.iter().cloned());
                }
                dependency_artifacts.push((dep_name.clone(), artifact));
            }
        }
        dependency_artifacts.sort();
        dependency_artifacts.dedup();

        if dependencies_use_std {
            let std_artifact = ensure_sysroot_std_available(package.no_proc_execution)?.std_artifact;
            if !implicit_artifact_inputs.contains(&std_artifact) {
                implicit_artifact_inputs.push(std_artifact);
            }
        }

        let artifact_path = package.artifact_path();
        if !artifact_is_fresh(
            &package,
            &artifact_path,
            &dependency_artifacts,
            &implicit_artifact_inputs,
        )? {
            if let Some(parent) = artifact_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create artifact directory {}: {}",
                        parent.display(),
                        e
                    )
                })?;
            }

            if let Some(parent) = package.object_path().parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create object directory {}: {}",
                        parent.display(),
                        e
                    )
                })?;
            }

            let mut artifacts = dependency_artifacts
                .iter()
                .map(|(name, path)| ExternArtifact {
                    name: name.clone(),
                    path: path.clone(),
                })
                .collect::<Vec<_>>();
            artifacts.extend(implicit_artifact_inputs.iter().map(|path| ExternArtifact {
                name: STD_CRATE_NAME.to_string(),
                path: path.clone(),
            }));
            let invocation =
                build_dependency_artifact_invocation(resolve_rockc_path()?, &package, &artifacts);
            progress::compiling(&package);
            run_invocation(
                invocation,
                &package,
                Some(&state.progress),
                &format!(
                    "library artifact for crate '{}'",
                    package.manifest.crate_.name
                ),
            )?;
        }

        state
            .artifacts
            .insert(canonical_root.clone(), artifact_path.clone());
        state.artifact_dependencies.insert(artifact_path.clone(), dependency_artifacts);
        state.uses_std.insert(
            artifact_path.clone(),
            package_requires_sysroot_std(&package) || dependencies_use_std,
        );
        state.progress.complete();
        Ok(artifact_path)
    })();

    state.stack.pop();
    result
}

fn artifact_is_fresh(
    package: &Package,
    artifact_path: &Path,
    dependency_artifacts: &[(String, PathBuf)],
    implicit_artifact_inputs: &[PathBuf],
) -> Result<bool, String> {
    let object_path = package.object_path();
    if !artifact_path.exists() || !object_path.exists() {
        return Ok(false);
    }

    let artifact_modified = file_modified(artifact_path)?;
    let object_modified = file_modified(&object_path)?;

    if !validate_artifact_with_rockc(artifact_path)? {
        return Ok(false);
    }

    for source_path in collect_package_source_inputs(&package.root_dir, crate::package::BUILD_DIR)?
    {
        let source_modified = file_modified(&source_path)?;
        if source_modified > artifact_modified || source_modified > object_modified {
            return Ok(false);
        }
    }

    for (_, dependency_artifact_path) in dependency_artifacts {
        if !dependency_artifact_path.exists() {
            return Ok(false);
        }
        let dependency_modified = file_modified(dependency_artifact_path)?;
        if dependency_modified > artifact_modified || dependency_modified > object_modified {
            return Ok(false);
        }
    }

    for implicit_artifact_path in implicit_artifact_inputs {
        if !implicit_artifact_path.exists() {
            return Ok(false);
        }

        let implicit_modified = file_modified(implicit_artifact_path)?;
        if implicit_modified > artifact_modified || implicit_modified > object_modified {
            return Ok(false);
        }
    }

    Ok(true)
}

fn validate_artifact_with_rockc(artifact_path: &Path) -> Result<bool, String> {
    let rockc = resolve_rockc_path()?;
    let output = Command::new(&rockc)
        .arg("--validate-artifact")
        .arg(artifact_path)
        .output()
        .map_err(|e| {
            format!(
                "Failed to spawn rockc for product artifact validation using {}: {}",
                rockc.display(),
                e
            )
        })?;

    Ok(output.status.success())
}
