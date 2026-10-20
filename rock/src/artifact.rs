use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use rock_shared::fs::{collect_package_source_inputs, file_modified};

use crate::{
    bundled_sysroot::{
        ensure_sysroot_std_available, package_requires_sysroot_std, STD_CRATE_NAME,
    },
    dependency_plan::{BuildPlan, BuildTriples, BuildUnitId},
    package::{BuildProfile, Package},
    progress,
    rockc::{
        build_dependency_artifact_invocation, build_proc_artifact_invocation, discover_proc_macro_sdk,
        resolve_rockc_path, run_invocation, ExternArtifact,
    },
};

pub(crate) struct ArtifactBuildState {
    profile: BuildProfile,
    no_proc_execution: bool,
    project_root: PathBuf,
    triples: BuildTriples,
    artifacts: HashMap<BuildUnitId, PathBuf>,
    packages: HashMap<PathBuf, Package>,
    uses_std: HashMap<PathBuf, bool>,
    pub(crate) progress: progress::BuildProgress,
}

impl ArtifactBuildState {
    pub(crate) fn new(package: &Package) -> Self {
        Self {
            profile: package.profile,
            no_proc_execution: package.no_proc_execution,
            project_root: package.root_dir.clone(),
            artifacts: HashMap::new(),
            packages: HashMap::new(),
            triples: BuildTriples::native(),
            uses_std: HashMap::new(),
            progress: progress::BuildProgress::default(),
        }
    }

    pub(crate) fn for_targets(package: &Package, root_targets: usize) -> Result<Self, String> {
        let plan = BuildPlan::new(package, BuildTriples::native())?;
        Ok(Self {
            progress: progress::BuildProgress::new(plan.units.len() - 1 + root_targets),
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

pub(crate) fn collect_dependency_artifacts(
    package: &Package,
    state: &mut ArtifactBuildState,
) -> Result<Vec<(String, PathBuf)>, String> {
    let mut artifacts = BTreeMap::new();
    let plan = BuildPlan::with_known_packages(package, state.triples.clone(), &state.project_root, &state.packages)?;
    for id in &plan.units[&plan.root].compiler_inputs {
        let path = ensure_planned_artifact(id, &plan, state)?;
        let name = plan.units[id].package.manifest.crate_.name.clone();
        if artifacts.insert(name.clone(), path.clone()).is_some_and(|previous| previous != path) {
            return Err(format!("conflicting dependency build units for crate '{name}'"));
        }
    }

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

    if package_requires_sysroot_std(package) && !artifacts.iter().any(|artifact| artifact.name == STD_CRATE_NAME) {
        let layout = ensure_sysroot_std_available(package.no_proc_execution)?;
        artifacts.push(ExternArtifact {
            name: STD_CRATE_NAME.to_string(),
            path: layout.std_artifact,
        });
    }
    if state.dependency_artifacts_use_std(&dependency_artifacts) && !artifacts.iter().any(|artifact| artifact.name == STD_CRATE_NAME) {
        let std_artifact = ensure_sysroot_std_available(package.no_proc_execution)?.std_artifact;
        artifacts.push(ExternArtifact {
            name: STD_CRATE_NAME.to_string(),
            path: std_artifact,
        });
    }

    Ok(artifacts)
}

pub(crate) fn ensure_artifact(
    crate_root: &Path,
    state: &mut ArtifactBuildState,
) -> Result<PathBuf, String> {
    let canonical_root = crate_root.canonicalize().map_err(|error| format!("Failed to resolve crate root {}: {error}", crate_root.display()))?;
    let mut package = match state.packages.get(&canonical_root) {
        Some(package) => package.clone(),
        None => Package::load(canonical_root)?,
    };
    package.target = package.manifest.library().cloned().ok_or_else(|| format!(
        "Package '{}' has no library target and cannot be used as a dependency; add a [lib] target", package.manifest.crate_.name))?;
    package.profile = state.profile; package.no_proc_execution = state.no_proc_execution;
    package.platform = crate::dependency_plan::BuildPlatform::Target;
    let plan = BuildPlan::with_known_packages(&package, state.triples.clone(), &state.project_root, &state.packages)?;
    ensure_planned_artifact(&plan.root, &plan, state)
}

fn ensure_planned_artifact(id: &BuildUnitId, plan: &BuildPlan, state: &mut ArtifactBuildState) -> Result<PathBuf, String> {
    if let Some(existing) = state.artifacts.get(id) {
        return Ok(existing.clone());
    }
    (|| {
        let package = plan.units[id].package.clone();
        debug_assert_eq!(*id, plan.units[id].id);
        if plan.units[id].runtime_inputs.iter().any(|input| input.platform != id.platform) {
            return Err("runtime inputs cross the host/target boundary".into());
        }
        let procedural = id.role() == rock_shared::manifest::TargetRole::ProcMacro;
        if id.triple != rock_shared::sysroot::host_target_triple() {
            return Err(format!("native compiler cannot build requested platform triple '{}'", id.triple));
        }
        let supplied_std = plan.units[id].compiler_inputs.iter().any(|dependency|
            plan.units[dependency].package.manifest.crate_.name == STD_CRATE_NAME);
        let mut implicit_artifact_inputs = if package_requires_sysroot_std(&package) && !supplied_std {
            vec![ensure_sysroot_std_available(package.no_proc_execution)?.std_artifact.clone()]
        } else {
            Vec::new()
        };
        let mut dependency_artifacts = Vec::new();
        let mut dependencies_use_std = false;
        for dependency_id in &plan.units[id].compiler_inputs {
                let artifact = ensure_planned_artifact(dependency_id, plan, state)?;
                let dep_name = plan.units[dependency_id].package.manifest.crate_.name.clone();
                if dep_name != STD_CRATE_NAME {
                    dependencies_use_std |=
                        state.uses_std.get(&artifact).copied().unwrap_or(false);
                }
                dependency_artifacts.push((dep_name, artifact));
        }
        dependency_artifacts.sort();
        dependency_artifacts.dedup();

        if dependencies_use_std && !supplied_std {
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
            let executable = resolve_rockc_path()?;
            let invocation = if procedural {
                let sdk = discover_proc_macro_sdk(&executable, &artifacts)?;
                build_proc_artifact_invocation(executable, &package, &artifacts, &sdk)
            } else { build_dependency_artifact_invocation(executable, &package, &artifacts) };
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
            .insert(id.clone(), artifact_path.clone());
        state.packages.insert(package.root_dir.clone(), package.clone());
        state.uses_std.insert(
            artifact_path.clone(),
            !procedural && (package_requires_sysroot_std(&package) || dependencies_use_std),
        );
        state.progress.complete();
        Ok(artifact_path)
    })()
}

fn artifact_is_fresh(
    package: &Package,
    artifact_path: &Path,
    dependency_artifacts: &[(String, PathBuf)],
    implicit_artifact_inputs: &[PathBuf],
) -> Result<bool, String> {
    let object_path = if package.target.role() == rock_shared::manifest::TargetRole::ProcMacro { package.worker_path() } else { package.object_path() };
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
