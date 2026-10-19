use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

use rock_shared::manifest::{self, CrateManifest, CrateTarget};

pub(crate) const BUILD_DIR: &str = "build";
pub(crate) const ARTIFACTS_DIR: &str = "artifacts";
pub(crate) const OBJECTS_DIR: &str = "objects";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum BuildProfile {
    #[default]
    Debug,
    Release,
}

impl BuildProfile {
    pub(crate) fn directory(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }

    pub(crate) fn flag(self) -> &'static str {
        match self {
            Self::Debug => "--debug",
            Self::Release => "--release",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Package {
    pub(crate) root_dir: PathBuf,
    pub(crate) manifest: CrateManifest,
    pub(crate) profile: BuildProfile,
    pub(crate) target: CrateTarget,
    build_root: PathBuf,
    dependency_key: Option<String>,
}

impl Package {
    pub(crate) fn load(root_dir: PathBuf) -> Result<Self, String> {
        let canonical_root = root_dir.canonicalize().map_err(|e| {
            format!(
                "Failed to resolve package root {}: {}",
                root_dir.display(),
                e
            )
        })?;
        let manifest_path = canonical_root.join("rock.toml");
        let manifest = manifest::load_manifest(&manifest_path)?;

        let target = manifest.primary_target().clone();
        Ok(Self {
            build_root: canonical_root.join(BUILD_DIR),
            dependency_key: None,
            root_dir: canonical_root,
            manifest,
            target,
            profile: BuildProfile::default(),
        })
    }

    pub(crate) fn use_project_build(&mut self, project_root: &Path) {
        self.build_root = project_root.join(BUILD_DIR);
        self.dependency_key = if self.root_dir == project_root {
            None
        } else {
            let mut hash = DefaultHasher::new();
            self.root_dir.hash(&mut hash);
            Some(format!(
                "{}-{}-{:016x}",
                filename_component(&self.manifest.crate_.name),
                filename_component(&self.manifest.crate_.version),
                hash.finish(),
            ))
        };
    }

    pub(crate) fn build_dir(&self) -> PathBuf {
        let directory = self.build_root.join(self.profile.directory());
        match &self.dependency_key {
            Some(key) => directory.join("deps").join(key),
            None => directory,
        }
    }

    pub(crate) fn artifact_path(&self) -> PathBuf {
        self.build_dir().join(ARTIFACTS_DIR).join(format!(
            "{}-{}.rkca",
            filename_component(&self.manifest.crate_.name),
            filename_component(&self.manifest.crate_.version),
        ))
    }

    pub(crate) fn object_dir(&self) -> PathBuf {
        self.build_dir().join(OBJECTS_DIR)
    }

    pub(crate) fn object_path(&self) -> PathBuf {
        let entry_file = self.entry_file();
        let stem = entry_file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("main");
        self.object_dir().join(format!("{}.o", stem))
    }

    pub(crate) fn entry_file(&self) -> PathBuf {
        self.root_dir.join(self.target.path())
    }

    pub(crate) fn binary_build_dir(&self) -> PathBuf {
        match &self.target {
            CrateTarget::Bin { name, .. } => self.build_dir().join("bins").join(name),
            CrateTarget::Lib { .. } => self.build_dir(),
        }
    }

    pub(crate) fn executable_path(&self, name: &str) -> PathBuf {
        let filename = if cfg!(target_os = "windows") { format!("{}.exe", name) } else { name.to_string() };
        self.build_dir().join(filename)
    }
}

fn filename_component(value: &str) -> String {
    value.chars().map(|ch| {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '+') {
            ch
        } else {
            '_'
        }
    }).collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::tests::support::{load_package, temp_test_dir, write_package};

    use super::BuildProfile;

    #[test]
    fn test_shared_build_layout_isolates_same_named_packages_and_profiles() {
        let root = temp_test_dir("shared_build_layout");
        let app = root.join("app");
        let first = root.join("first");
        let second = root.join("second");
        for (path, name) in [(&app, "app"), (&first, "dep"), (&second, "dep")] {
            write_package(path, name, "lib.rk", &[], "< answer = -> 42\n");
        }
        let project = load_package(app.clone());
        let mut a = load_package(first.clone());
        let mut b = load_package(second.clone());
        a.use_project_build(&project.root_dir);
        b.use_project_build(&project.root_dir);
        assert_ne!(a.artifact_path(), b.artifact_path());
        assert_ne!(a.object_path(), b.object_path());
        let debug = a.artifact_path();
        a.profile = BuildProfile::Release;
        assert_ne!(a.artifact_path(), debug);
        assert!(a.build_dir().starts_with(app.join("build/release/deps")));
        assert!(!first.join("build").exists());
        assert!(!second.join("build").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn test_manifest_text_cannot_escape_the_shared_build_directory() {
        let root = temp_test_dir("safe_build_names");
        write_package(&root, "app", "lib.rk", &[], "< answer = -> 42\n");
        let mut package = load_package(root.clone());
        package.manifest.crate_.name = "../../escape".to_string();
        package.manifest.crate_.version = "../../../escape".to_string();
        assert_eq!(package.artifact_path().parent().unwrap(), package.build_dir().join("artifacts"));
        fs::remove_dir_all(root).unwrap();
    }
}
