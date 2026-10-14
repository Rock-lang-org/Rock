use std::path::PathBuf;

use rock_shared::manifest::{self, CrateManifest};

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

        Ok(Self {
            root_dir: canonical_root,
            manifest,
            profile: BuildProfile::default(),
        })
    }

    pub(crate) fn build_dir(&self) -> PathBuf {
        self.root_dir.join(BUILD_DIR).join(self.profile.directory())
    }

    pub(crate) fn artifact_path(&self) -> PathBuf {
        self.build_dir().join(ARTIFACTS_DIR).join(format!(
            "{}-{}.rkca",
            self.manifest.crate_.name, self.manifest.crate_.version
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
        self.root_dir.join(&self.manifest.lib.path)
    }
}
