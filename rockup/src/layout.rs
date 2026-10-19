use std::path::{Path, PathBuf};

use crate::constants::{
    BIN_DIR, COMPONENTS_MANIFEST_NAME, LIB_DIR, ROCKC_BIN_NAME, ROCK_BIN_NAME, ROCK_LSP_BIN_NAME,
    STD_ARTIFACT_NAME, STD_OBJECT_NAME, TOOLCHAIN_MANIFEST_NAME,
};

pub(crate) use rock_shared::sysroot::host_target_triple;

#[derive(Debug, Clone)]
pub(crate) struct ToolchainLayout {
    pub(crate) sysroot: PathBuf,
    pub(crate) target_triple: String,
    pub(crate) bin_dir: PathBuf,
    pub(crate) target_component_dir: PathBuf,
    pub(crate) rock_bin: PathBuf,
    pub(crate) rockc_bin: PathBuf,
    pub(crate) rock_lsp_bin: PathBuf,
    pub(crate) std_artifact: PathBuf,
    pub(crate) std_object: PathBuf,
    pub(crate) manifest_path: PathBuf,
    pub(crate) components_path: PathBuf,
}

impl ToolchainLayout {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self::new_for_target(root, host_target_triple())
    }

    pub(crate) fn new_for_target(root: PathBuf, target_triple: String) -> Self {
        let bin_dir = root.join(BIN_DIR);
        let target_libdir = root.join(LIB_DIR).join("rocklib").join(&target_triple);

        Self {
            sysroot: root,
            target_triple,
            target_component_dir: target_libdir.clone(),
            rock_bin: bin_dir.join(ROCK_BIN_NAME),
            rockc_bin: bin_dir.join(ROCKC_BIN_NAME),
            rock_lsp_bin: bin_dir.join(ROCK_LSP_BIN_NAME),
            std_artifact: target_libdir.join(STD_ARTIFACT_NAME),
            std_object: target_libdir.join(STD_OBJECT_NAME),
            manifest_path: target_libdir.join(TOOLCHAIN_MANIFEST_NAME),
            components_path: target_libdir.join(COMPONENTS_MANIFEST_NAME),
            bin_dir,
        }
    }
}

pub(crate) struct CargoBuildDirLayout {
    pub(crate) rock_bin: PathBuf,
    pub(crate) rockc_bin: PathBuf,
    pub(crate) rock_lsp_bin: PathBuf,
    pub(crate) target_libdir: PathBuf,
}

pub(crate) fn cargo_build_dir_layout(root: PathBuf) -> CargoBuildDirLayout {
    let target_libdir = root
        .join(LIB_DIR)
        .join("rocklib")
        .join(host_target_triple());

    CargoBuildDirLayout {
        rock_bin: root.join(ROCK_BIN_NAME),
        rockc_bin: root.join(ROCKC_BIN_NAME),
        rock_lsp_bin: root.join(ROCK_LSP_BIN_NAME),
        target_libdir,
    }
}

pub(crate) fn target_component_dir(toolchain_root: &Path, triple: &str) -> PathBuf {
    toolchain_root.join(LIB_DIR).join("rocklib").join(triple)
}
