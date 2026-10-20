//! Proc-only host metadata and bounded content validation; inspection never spawns.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ProcMacroArtifact, ProcMacroCapability, ProcMacroExport, ProcMacroKind, ProcMacroRequest, PROC_MACRO_PROTOCOL_VERSION};

const MAX_HOST_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_HOST_FILES: usize = 128;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroFileIdentity {
    pub bytes: u64,
    pub sha256: [u8; 32],
}

impl ProcMacroFileIdentity {
    pub fn read(path: &Path) -> Result<Self, String> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = options.open(path).map_err(|error| format!("failed to open host file {}: {error}", path.display()))?;
        let metadata = file.metadata().map_err(|error| format!("failed to inspect host file {}: {error}", path.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_HOST_FILE_BYTES {
            return Err(format!("host file {} is not a bounded regular file", path.display()));
        }
        let mut reader = file.take(MAX_HOST_FILE_BYTES + 1);
        let mut hash = Sha256::new();
        let mut bytes = 0;
        let mut buffer = [0; 65536];
        loop {
            let count = reader.read(&mut buffer).map_err(|error| format!("failed to read host file {}: {error}", path.display()))?;
            if count == 0 { break; }
            bytes += count as u64;
            if bytes > MAX_HOST_FILE_BYTES { return Err("host file byte limit exceeded".into()); }
            hash.update(&buffer[..count]);
        }
        Ok(Self { bytes, sha256: hash.finalize().into() })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroHostFile {
    pub path: PathBuf,
    pub identity: ProcMacroFileIdentity,
}

impl ProcMacroArtifact {
    pub fn for_executable(crate_identity: String, executable: PathBuf, exports: Vec<ProcMacroExport>) -> Result<Self, String> {
        let executable = executable.canonicalize().map_err(|error| format!("failed to locate host executable: {error}"))?;
        let artifact = Self {
            artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
            crate_identity, protocol_version: PROC_MACRO_PROTOCOL_VERSION,
            compiler_version: env!("CARGO_PKG_VERSION").into(), host_triple: crate::sysroot::host_target_triple(),
            executable_identity: ProcMacroFileIdentity::read(&executable)?, executable,
            required_files: Vec::new(), capabilities: vec![ProcMacroCapability::Stdio],
            definition_dependencies: Vec::new(), exports,
        };
        artifact.validate_metadata()?;
        Ok(artifact)
    }

    pub fn validate_metadata(&self) -> Result<(), String> {
        if self.artifact_format_version != crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION {
            return Err("unsupported proc artifact format".into());
        }
        if self.protocol_version != PROC_MACRO_PROTOCOL_VERSION || self.compiler_version != env!("CARGO_PKG_VERSION") {
            return Err("incompatible proc artifact protocol/compiler version".into());
        }
        if self.crate_identity.is_empty() || self.host_triple.is_empty() || self.executable.as_os_str().is_empty()
            || self.executable_identity.bytes == 0 || self.executable_identity.bytes > MAX_HOST_FILE_BYTES {
            return Err("invalid proc host executable identity".into());
        }
        if self.required_files.len() > MAX_HOST_FILES || self.exports.is_empty() || self.exports.len() > super::MAX_METADATA_ENTRIES
            || self.definition_dependencies.len() > super::MAX_METADATA_ENTRIES {
            return Err("proc artifact metadata count limit exceeded".into());
        }
        if self.capabilities != [ProcMacroCapability::Stdio] { return Err("unsupported proc artifact capabilities".into()); }
        let mut names = BTreeSet::new();
        let mut identities = BTreeSet::new();
        for export in &self.exports {
            if export.kind != ProcMacroKind::FunctionLike || export.name.split("::").any(str::is_empty)
                || export.identity.is_empty() || !identities.insert(&export.identity) || !names.insert(&export.name) {
                return Err("invalid or unsupported proc export identity/kind".into());
            }
        }
        let mut paths = BTreeSet::new(); paths.insert(&self.executable);
        let mut total_bytes = self.executable_identity.bytes;
        for file in &self.required_files {
            if file.path.as_os_str().is_empty() || file.identity.bytes > MAX_HOST_FILE_BYTES || !paths.insert(&file.path) {
                return Err("invalid or duplicate required proc host file".into());
            }
            total_bytes = total_bytes.checked_add(file.identity.bytes).ok_or("required proc host file byte count overflow")?;
            if total_bytes > MAX_HOST_FILE_BYTES { return Err("total proc host file byte limit exceeded".into()); }
        }
        Ok(())
    }

    /// Resolve only against the supplying artifact, never cwd or sibling guesses.
    pub fn resolve_for_host(&self, artifact_path: &Path) -> Result<Self, String> {
        self.validate_metadata()?;
        let artifact_path = if artifact_path.is_absolute() { artifact_path.to_path_buf() } else {
            std::env::current_dir().map_err(|error| format!("failed to resolve artifact location: {error}"))?.join(artifact_path)
        };
        let parent = artifact_path.parent().unwrap_or_else(|| Path::new("/"));
        let resolve = |path: &Path| if path.is_absolute() { path.to_path_buf() } else { parent.join(path) };
        let mut resolved = self.clone();
        resolved.executable = resolve(&self.executable);
        for file in &mut resolved.required_files { file.path = resolve(&file.path); }
        resolved.validate_host_files()?;
        Ok(resolved)
    }

    pub fn validate_host_files(&self) -> Result<(), String> {
        self.validate_metadata()?;
        if self.host_triple != crate::sysroot::host_target_triple() { return Err("proc artifact execution host mismatch".into()); }
        for (path, identity) in std::iter::once((&self.executable, &self.executable_identity))
            .chain(self.required_files.iter().map(|file| (&file.path, &file.identity))) {
            if !path.is_absolute() { return Err("proc host file has no artifact-relative resolution".into()); }
            if ProcMacroFileIdentity::read(path)? != *identity { return Err(format!("proc host file content identity mismatch: {}", path.display())); }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&self.executable).map_err(|error| format!("failed to inspect host executable: {error}"))?;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(format!("proc host executable lacks execution permission: {}", std::io::Error::from_raw_os_error(libc::EACCES)));
            }
        }
        Ok(())
    }

    pub(super) fn validate_invocation(&self, request: &ProcMacroRequest) -> Result<(), String> {
        self.validate_metadata()?;
        if !self.exports.iter().any(|export| export.identity == request.macro_identity && export.kind == request.kind) {
            return Err("unknown proc export identity/kind".into());
        }
        self.validate_host_files()
    }
}
