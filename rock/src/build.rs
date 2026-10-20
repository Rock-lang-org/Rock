use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use rock_shared::manifest::CrateTarget;

use crate::{
    artifact::{collect_root_artifacts, ensure_artifact, ArtifactBuildState},
    compile::output_executable_path,
    package::{BuildProfile, Package},
    progress,
    rockc::{build_root_invocation, resolve_rockc_path, run_invocation},
};

#[derive(Debug, Default)]
pub(crate) struct BuildProducts {
    pub(crate) library: Option<PathBuf>,
    pub(crate) binaries: Vec<BinaryProduct>,
}

#[derive(Debug)]
pub(crate) struct BinaryProduct {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
}

impl BuildProducts {
    pub(crate) fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.library.iter().chain(self.binaries.iter().map(|binary| &binary.path))
    }
}

pub(crate) fn build_project(project_root: &Path, profile: BuildProfile, no_proc_execution: bool) -> Result<BuildProducts, String> {
    let mut package = Package::load(project_root.to_path_buf())?;
    package.profile = profile;
    package.no_proc_execution = no_proc_execution;
    let binaries = package.manifest.binaries().cloned().collect::<Vec<_>>();
    build_targets(package, binaries)
}

fn build_targets(mut package: Package, binaries: Vec<CrateTarget>) -> Result<BuildProducts, String> {
    let started = Instant::now();
    let has_library = package.manifest.library().is_some();
    let mut state = ArtifactBuildState::for_targets(&package, usize::from(has_library) + binaries.len())?;
    let mut products = BuildProducts::default();
    if has_library {
        products.library = Some(ensure_artifact(&package.root_dir, &mut state)?);
    }
    if !binaries.is_empty() {
        let mut artifacts = collect_root_artifacts(&package, &mut state)?;
        if let Some(library) = &products.library {
            artifacts.push(crate::rockc::ExternArtifact {
                name: package.manifest.crate_.name.clone(),
                path: library.clone(),
            });
        }
        for target in binaries {
            let CrateTarget::Bin { name, .. } = &target else { continue };
            let name = name.clone();
            package.target = target;
            let output_dir = package.binary_build_dir();
            fs::create_dir_all(&output_dir).map_err(|e| {
                format!("Failed to create build directory {}: {}", output_dir.display(), e)
            })?;
            let invocation = build_root_invocation(resolve_rockc_path()?, &package, &artifacts);
            progress::compiling(&package);
            run_invocation(
                invocation,
                &package,
                Some(&state.progress),
                &format!("root executable for crate '{}' (binary '{}')", package.manifest.crate_.name, name),
            )?;
            // rockc names its executable after the entry file. Isolate intermediate
            // products per target, then publish under the manifest's binary name.
            let emitted = output_executable_path(&output_dir, &package.entry_file());
            let path = package.executable_path(&name);
            fs::rename(&emitted, &path).map_err(|e| {
                format!("Failed to publish binary '{}' at {}: {}", name, path.display(), e)
            })?;
            products.binaries.push(BinaryProduct { name, path });
            state.progress.complete();
        }
    }
    progress::finished(package.profile, started.elapsed(), &state.progress);
    Ok(products)
}

pub(crate) fn run_project(project_root: &Path, profile: BuildProfile, args: &[String], binary: Option<&str>, no_proc_execution: bool) -> Result<i32, String> {
    let mut package = Package::load(project_root.to_path_buf())?;
    package.profile = profile;
    package.no_proc_execution = no_proc_execution;
    let binaries = package.manifest.binaries().cloned().collect::<Vec<_>>();
    let selected = if let Some(name) = binary {
        let target = binaries.iter().find(|target| matches!(target, CrateTarget::Bin { name: candidate, .. } if candidate == name))
            .ok_or_else(|| format!("No binary named '{}' in package '{}'", name, package.manifest.crate_.name))?;
        Some(target.clone())
    } else if binaries.len() > 1 {
        let names = binaries.iter().filter_map(|target| match target {
            CrateTarget::Bin { name, .. } => Some(name.as_str()),
            _ => None,
        }).collect::<Vec<_>>().join(", ");
        return Err(format!("Multiple binaries are available: {}. Select one with `rock run --bin <name>`", names));
    } else {
        binaries.into_iter().next()
    };
    let crate_name = package.manifest.crate_.name.clone();
    let products = build_targets(package, selected.into_iter().collect())?;
    let Some(executable) = products.binaries.first() else {
        println!("Crate '{}' is a library; there is no binary to execute.", crate_name);
        return Ok(0);
    };
    let status = std::process::Command::new(&executable.path)
        .args(args)
        .status()
        .map_err(|e| format!("Failed to run binary '{}' at {}: {}", executable.name, executable.path.display(), e))?;

    Ok(status.code().unwrap_or(1))
}
