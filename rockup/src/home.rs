use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::constants::{
    BIN_DIR, DEFAULT_TOOLCHAIN_FILE, ENV_FILE_NAME, ROCKUP_HOME_ENV, TOOLCHAINS_DIR,
};

#[derive(Debug, Clone)]
pub(crate) struct RockupHome {
    pub(crate) root: PathBuf,
}

impl RockupHome {
    pub(crate) fn resolve() -> Result<Self, String> {
        let root = match env::var_os(ROCKUP_HOME_ENV) {
            Some(path) => PathBuf::from(path),
            None => default_rockup_home(&home_dir()?),
        };

        Ok(Self { root })
    }

    pub(crate) fn toolchains_dir(&self) -> PathBuf {
        self.root.join(TOOLCHAINS_DIR)
    }

    pub(crate) fn bin_dir(&self) -> PathBuf {
        self.root.join(BIN_DIR)
    }

    pub(crate) fn default_toolchain_file(&self) -> PathBuf {
        self.root.join(DEFAULT_TOOLCHAIN_FILE)
    }

    pub(crate) fn env_file(&self) -> PathBuf {
        self.root.join(ENV_FILE_NAME)
    }

    pub(crate) fn toolchain_dir(&self, name: &str) -> Result<PathBuf, String> {
        validate_name(name)?;
        let path = self.toolchains_dir().join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!("Toolchain '{}' must not be a symlink", name));
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("Failed to inspect {}: {}", path.display(), error));
            }
            _ => {}
        }
        Ok(path)
    }
}

pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    if name.len() > 128
        || !name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-+".contains(&c))
    {
        return Err(format!("Invalid toolchain name '{}'", name));
    }
    Ok(())
}

pub(crate) fn home_dir() -> Result<PathBuf, String> {
    // PowerShell and cmd set USERPROFILE; Git Bash and MSYS also set HOME.
    env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .or_else(|| {
            cfg!(windows)
                .then(|| env::var_os("USERPROFILE"))
                .flatten()
                .filter(|home| !home.is_empty())
        })
        .map(PathBuf::from)
        .ok_or_else(|| "Failed to resolve HOME for rockup".to_string())
}

pub(crate) fn default_rockup_home(user_home: &Path) -> PathBuf {
    user_home.join(".rockup")
}

pub(crate) fn installed_toolchain_names(home: &RockupHome) -> Result<Vec<String>, String> {
    let toolchains_dir = home.toolchains_dir();

    if !toolchains_dir.exists() {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();
    for entry in fs::read_dir(&toolchains_dir).map_err(|e| {
        format!(
            "Failed to read toolchains directory {}: {}",
            toolchains_dir.display(),
            e
        )
    })? {
        let entry = entry.map_err(|e| format!("Failed to read toolchain entry: {}", e))?;
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Failed to inspect toolchain entry: {}", e))?;
        if !file_type.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if validate_name(&name).is_ok() {
            names.push(name);
        }
    }

    names.sort();
    Ok(names)
}
