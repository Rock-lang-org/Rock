pub(crate) const ROCKUP_HOME_ENV: &str = "ROCKUP_HOME";
/// Set to skip shell startup file and Windows user PATH changes, like
/// rustup's `--no-modify-path`. The `env` script is still written.
pub(crate) const ROCKUP_NO_MODIFY_PATH_ENV: &str = "ROCKUP_NO_MODIFY_PATH";
pub(crate) const ROCKUP_TOOLCHAIN_ENV: &str = "ROCKUP_TOOLCHAIN";
pub(crate) const ROCK_TOOLCHAIN_FILE: &str = "rock-toolchain.toml";
pub(crate) const DEFAULT_TOOLCHAIN_FILE: &str = "default-toolchain";
pub(crate) const ENV_FILE_NAME: &str = "env";
#[cfg_attr(windows, allow(dead_code))] // Shell startup files are Unix-only.
pub(crate) const SHELL_INIT_START_MARKER: &str = "# >>> rockup initialize >>>";
#[cfg_attr(windows, allow(dead_code))] // Shell startup files are Unix-only.
pub(crate) const SHELL_INIT_END_MARKER: &str = "# <<< rockup initialize <<<";
pub(crate) const TOOLCHAINS_DIR: &str = "toolchains";
pub(crate) const BIN_DIR: &str = "bin";
pub(crate) const LIB_DIR: &str = "lib";
pub(crate) const ROCK_BIN_NAME: &str = "rock";
pub(crate) const ROCKC_BIN_NAME: &str = "rockc";
pub(crate) const ROCK_LSP_BIN_NAME: &str = "rock-lsp";
pub(crate) const STD_ARTIFACT_NAME: &str = rock_shared::sysroot::STD_ARTIFACT_FILE_NAME;
pub(crate) const STD_OBJECT_NAME: &str = rock_shared::sysroot::STD_OBJECT_FILE_NAME;
pub(crate) const TOOLCHAIN_MANIFEST_NAME: &str = rock_shared::sysroot::TOOLCHAIN_MANIFEST_FILE_NAME;
pub(crate) const COMPONENTS_MANIFEST_NAME: &str =
    rock_shared::sysroot::COMPONENTS_MANIFEST_FILE_NAME;

/// `name` as an executable file name on the host (`rock.exe` on Windows).
pub(crate) fn exe_name(name: &str) -> String {
    format!("{}{}", name, std::env::consts::EXE_SUFFIX)
}
