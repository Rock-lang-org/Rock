use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::ProductSourceFingerprint;
use crate::{source_loader, Config};

pub(crate) fn product_source_fingerprint(
    config: &Config,
    source_db: &source_loader::SourceDatabase,
    source_files: &[PathBuf],
) -> ProductSourceFingerprint {
    let root = config.entry_file.parent().unwrap_or_else(|| Path::new("."));
    let mut source_files = source_files.to_vec();
    if source_files.is_empty() {
        source_files.push(config.entry_file.clone());
    }

    let source_entries = fingerprint_source_entries(root, &source_files);
    let loaded_files = source_entries
        .iter()
        .map(|(relative_path, _)| relative_path.clone())
        .collect::<Vec<_>>();

    let source_hash = stable_loaded_source_fingerprint(root, &source_entries, Some(source_db));
    let manifest_path = config
        .entry_file
        .parent()
        .map(|parent| parent.join("rock.toml"))
        .filter(|path| path.exists());
    let manifest_hash = manifest_path.as_ref().and_then(|path| {
        let manifest_files = fingerprint_source_entries(root, std::slice::from_ref(path));
        stable_loaded_source_fingerprint(root, &manifest_files, None)
    });

    ProductSourceFingerprint {
        manifest_hash,
        source_hash,
        loaded_files,
    }
}

fn fingerprint_source_entries(root: &Path, paths: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    paths
        .iter()
        .fold(BTreeMap::new(), |mut entries, path| {
            let relative_path = path
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.clone());
            entries.entry(relative_path).or_insert_with(|| path.clone());
            entries
        })
        .into_iter()
        .collect()
}

fn stable_loaded_source_fingerprint(
    root: &Path,
    entries: &[(PathBuf, PathBuf)],
    source_db: Option<&source_loader::SourceDatabase>,
) -> Option<String> {
    if entries.is_empty() {
        return None;
    }

    let mut hash = 0xcbf29ce484222325u64;
    for (relative_path, original_path) in entries {
        hash_bytes(&mut hash, relative_path.to_string_lossy().as_bytes());
        hash_bytes(&mut hash, &[0]);

        if let Some(source_file) = source_db.and_then(|db| db.source_file_for_path(original_path)) {
            hash_bytes(&mut hash, source_file.text.as_bytes());
        } else {
            let read_path = if original_path.is_absolute() {
                original_path.clone()
            } else {
                root.join(relative_path)
            };
            let bytes = fs::read(read_path).ok()?;
            hash_bytes(&mut hash, &bytes);
        }
        hash_bytes(&mut hash, &[0xff]);
    }

    Some(format!("fnv1a64:{hash:016x}"))
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

#[cfg(test)]
mod tests;
