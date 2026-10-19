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
pub fn resolve_analysis_project_for_source(project_root: &Path, source_path: &Path) -> Result<AnalysisProject, String> {
    resolve_project(project_root, Some(source_path))
}

fn resolve_project(project_root: &Path, source_path: Option<&Path>) -> Result<AnalysisProject, String> {
    let mut package = Package::load(project_root.to_path_buf())?;
    if let Some(source_path) = source_path {
        let source_path = source_path.canonicalize().unwrap_or_else(|_| source_path.to_path_buf());
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
mod tests {
    use std::fs;

    use crate::tests::support::{temp_test_dir, write_package, write_package_with_options};

    use super::{find_project_root, resolve_analysis_project};

    #[test]
    fn resolves_library_entry_for_project_analysis() {
        let root = temp_test_dir("analysis_library");
        write_package_with_options(&root, "library", "src/lib.rk", &[], "< answer = -> 42\n", true);
        let project = resolve_analysis_project(&root).unwrap();
        assert_eq!(project.entry_file, root.join("src/lib.rk"));
        assert!(project.no_std);
        assert!(project.extern_artifacts.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_nearest_manifest_and_resolves_no_std_project() {
        let root = std::env::temp_dir().join(format!(
            "rock_project_analysis_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let source_dir = root.join("src/nested");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(
            root.join("rock.toml"),
            "[crate]\nname = \"app\"\nversion = \"0.1.0\"\nno_std = true\n\n[[bin]]\npath = \"src/main.rk\"\n",
        )
        .unwrap();
        fs::write(root.join("src/main.rk"), "main = -> 0\n").unwrap();
        let nested_source = source_dir.join("module.rk");
        fs::write(&nested_source, "value = 1\n").unwrap();

        assert_eq!(find_project_root(&nested_source), Some(root.clone()));
        let project = resolve_analysis_project(&root).unwrap();
        assert_eq!(project.root_dir, root.canonicalize().unwrap());
        assert_eq!(project.current_crate_name, None);
        assert!(project.no_std);
        assert!(project.no_prelude);
        assert!(project.extern_artifacts.is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_toolchain_std_for_project_analysis() {
        let root = temp_test_dir("analysis_std");
        write_package(&root, "app", "src/main.rk", &[], "main = -> 42.println!\n");

        let project = resolve_analysis_project(&root).unwrap();

        assert!(!project.no_std);
        assert!(!project.no_prelude);
        let std = project
            .extern_artifacts
            .iter()
            .find(|(name, _)| name == "std")
            .expect("project analysis should include the toolchain std");
        assert!(std.1.is_file());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_analysis_uses_the_project_dependency_cache() {
        let root = temp_test_dir("analysis_dependency_cache");
        let dep = root.join("dep");
        let app = root.join("app");
        write_package_with_options(&dep, "dep", "lib.rk", &[], "< answer = -> 42\n", true);
        write_package_with_options(
            &app, "app", "src/main.rk", &[("dep", dep.display().to_string())],
            "> dep::answer\nmain = -> answer!\n", true,
        );
        let project = resolve_analysis_project(&app).unwrap();
        assert_eq!(project.extern_artifacts.len(), 1);
        assert!(project.extern_artifacts[0].1.starts_with(app.join("build/debug/deps")));
        assert!(project.extern_artifacts[0].1.exists());
        assert!(!dep.join("build").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
