use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub fn std_cache_key(std_dir: &Path, compiler_stamp: &str) -> String {
    let mut hash = 0xcbf29ce484222325;
    update_cache_hash(&mut hash, compiler_stamp.as_bytes());
    let canonical_std_dir = std_dir.canonicalize().unwrap();
    update_cache_hash(&mut hash, canonical_std_dir.to_string_lossy().as_bytes());

    let mut files = Vec::new();
    collect_rock_source_files(std_dir, &mut files);
    let manifest = std_dir.join("rock.toml");
    if manifest.is_file() {
        files.push(manifest);
    }
    files.sort();
    for path in files {
        let relative = path.strip_prefix(std_dir).unwrap();
        update_cache_hash(&mut hash, relative.to_string_lossy().as_bytes());
        update_cache_hash(&mut hash, &std::fs::read(&path).unwrap());
    }

    format!("{hash:016x}")
}

fn update_cache_hash(hash: &mut u64, bytes: &[u8]) {
    // Include lengths so path/content boundaries cannot alias.
    for byte in (bytes.len() as u64).to_le_bytes().iter().chain(bytes) {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn collect_rock_source_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            collect_rock_source_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rk") {
            files.push(path);
        }
    }
}

pub fn std_cache_compiler_stamp() -> String {
    let exe = std::env::current_exe().unwrap();
    let metadata = std::fs::metadata(&exe).unwrap();
    let modified = metadata
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap();
    // The test executable includes the compiler and artifact schema; rebuilding
    // it invalidates the fixture even if the package version did not change.
    format!(
        "{}:{}:{}",
        exe.display(),
        metadata.len(),
        modified.as_nanos()
    )
}
