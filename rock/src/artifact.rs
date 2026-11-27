use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use rock_shared::fs::{collect_package_source_inputs, file_modified};

use crate::{
    bundled_sysroot::{ensure_sysroot_std_available, package_requires_sysroot_std, STD_CRATE_NAME},
    deps::resolve_dependency_root,
    package::{BuildProfile, Package},
    progress,
    rockc::{
        build_dependency_artifact_invocation, resolve_rockc_path, run_invocation, ExternArtifact,
    },
};

pub(crate) struct ArtifactBuildState {
    profile: BuildProfile,
    target_triple: Option<String>,
    lint_args: Vec<String>,
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
            target_triple: package.target_triple.clone(),
            lint_args: package.lint_args.clone(),
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
        collect_package_roots(package, &package.root_dir, &mut roots)?;
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
            name != STD_CRATE_NAME && self.uses_std.get(artifact_path).copied().unwrap_or(false)
        })
    }
}

fn collect_package_roots(
    package: &Package,
    project_root: &Path,
    roots: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    roots.insert(package.root_dir.clone());
    if let Some(dependencies) = &package.manifest.dependencies {
        for (name, dependency) in dependencies {
            let root = resolve_dependency_root(&package.root_dir, project_root, name, dependency)?;
            if !roots.contains(&root) {
                collect_package_roots(&Package::load(root)?, project_root, roots)?;
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
        let layout = ensure_sysroot_std_available(package.target_triple.as_deref())?;
        artifacts.push(ExternArtifact {
            name: STD_CRATE_NAME.to_string(),
            path: layout.std_artifact,
        });
    }
    if state.dependency_artifacts_use_std(&dependency_artifacts) {
        let std_artifact =
            ensure_sysroot_std_available(package.target_triple.as_deref())?.std_artifact;
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
        let dep_root =
            resolve_dependency_root(&package.root_dir, &state.project_root, dep_name, dependency)?;
        let dep_root = dunce::canonicalize(&dep_root)
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
    let canonical_root = dunce::canonicalize(&crate_root).map_err(|e| {
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
        package.profile = state.profile;
        package.target_triple = state.target_triple.clone();
        package.use_project_build(&state.project_root);
        // Like cargo, lints are reported for the project's own crate only.
        package.lint_args = if package.is_project_root() {
            state.lint_args.clone()
        } else {
            vec!["-A".to_string(), "unused".to_string()]
        };
        let mut implicit_artifact_inputs = if package_requires_sysroot_std(&package) {
            vec![
                ensure_sysroot_std_available(package.target_triple.as_deref())?
                    .std_artifact
                    .clone(),
            ]
        } else {
            Vec::new()
        };
        let mut dependency_artifacts = Vec::new();
        let mut dependencies_use_std = false;
        if let Some(dependencies) = &package.manifest.dependencies {
            for (dep_name, dependency) in dependencies {
                let dep_root = resolve_dependency_root(
                    &package.root_dir,
                    &state.project_root,
                    dep_name,
                    dependency,
                )?;
                let artifact = ensure_artifact(&dep_root, state)?;
                if dep_name != STD_CRATE_NAME {
                    dependencies_use_std |= state.uses_std.get(&artifact).copied().unwrap_or(false);
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
            let std_artifact =
                ensure_sysroot_std_available(package.target_triple.as_deref())?.std_artifact;
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
            // The rebuilt artifact invalidates any previous validation stamp.
            let _ = fs::remove_file(validation_stamp_path(&artifact_path));
            let stamp = lint_stamp_path(&artifact_path);
            fs::write(&stamp, lint_stamp(&package))
                .map_err(|e| format!("Failed to write lint stamp {}: {}", stamp.display(), e))?;
        }

        state
            .artifacts
            .insert(canonical_root.clone(), artifact_path.clone());
        state
            .artifact_dependencies
            .insert(artifact_path.clone(), dependency_artifacts);
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

    // Lint levels change the outcome (denied lints fail the build), so an
    // artifact built under different levels is stale.
    if fs::read_to_string(lint_stamp_path(artifact_path)).ok() != Some(lint_stamp(package)) {
        return Ok(false);
    }

    let artifact_modified = file_modified(artifact_path)?;
    let object_modified = file_modified(&object_path)?;

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

    // Validation spawns rockc and decodes the payload; it runs last, and only
    // when the artifact, its object or the compiler changed since it passed.
    let rockc = resolve_rockc_path()?;
    let stamp = validation_stamp(&rockc, artifact_path, &object_path)?;
    let stamp_path = validation_stamp_path(artifact_path);
    if stamp.is_some() && fs::read_to_string(&stamp_path).ok() == stamp {
        return Ok(true);
    }
    if !validate_artifact_with_rockc(&rockc, artifact_path)? {
        let _ = fs::remove_file(&stamp_path);
        return Ok(false);
    }
    match stamp {
        Some(stamp) => {
            let _ = fs::write(stamp_path, stamp);
        }
        None => {
            let _ = fs::remove_file(&stamp_path);
        }
    }
    Ok(true)
}

/// The file a `ROCKC` override runs: a bare name is looked up on `PATH`,
/// as `Command` does.
fn rockc_executable_file(rockc: &Path) -> Option<PathBuf> {
    if rockc.components().count() > 1 {
        return rockc.is_file().then(|| rockc.to_path_buf());
    }
    // Like Command on Windows, an extensionless name also matches `name.exe`.
    let suffixed = (rockc.extension().is_none() && !std::env::consts::EXE_SUFFIX.is_empty())
        .then(|| {
            let mut name = rockc.as_os_str().to_owned();
            name.push(std::env::consts::EXE_SUFFIX);
            PathBuf::from(name)
        });
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|dir| {
            let suffixed = suffixed.as_ref().map(|name| dir.join(name));
            std::iter::once(dir.join(rockc)).chain(suffixed)
        })
        .find(|candidate| candidate.is_file())
}

fn validation_stamp_path(artifact_path: &Path) -> PathBuf {
    artifact_path.with_extension("validated")
}

/// Identifies the exact compiler, artifact and object a validation covered.
/// The artifact is keyed by content: a same-size replacement with preserved
/// timestamps (e.g. a cache restore) must be validated again. `None` when the
/// compiler file cannot be located; validation then always runs.
pub(crate) fn validation_stamp(
    rockc: &Path,
    artifact_path: &Path,
    object_path: &Path,
) -> Result<Option<String>, String> {
    use std::hash::{Hash, Hasher};

    let Some(rockc) = rockc_executable_file(rockc) else {
        return Ok(None);
    };
    let bytes = fs::read(artifact_path)
        .map_err(|e| format!("Failed to read {}: {}", artifact_path.display(), e))?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let mut stamp = format!(
        "{}\tcontent\t{:016x}\n",
        artifact_path.display(),
        hasher.finish()
    );
    for path in [rockc.as_path(), object_path] {
        let metadata = fs::metadata(path)
            .map_err(|e| format!("Failed to inspect {}: {}", path.display(), e))?;
        let modified = file_modified(path)?
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_nanos())
            .unwrap_or_default();
        stamp.push_str(&format!(
            "{}\t{}\t{}\n",
            path.display(),
            metadata.len(),
            modified
        ));
    }
    Ok(Some(stamp))
}

fn lint_stamp_path(artifact_path: &Path) -> PathBuf {
    artifact_path.with_extension("lints")
}

fn lint_stamp(package: &Package) -> String {
    package.lint_args.join("\n")
}

fn validate_artifact_with_rockc(rockc: &Path, artifact_path: &Path) -> Result<bool, String> {
    let output = Command::new(rockc)
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
