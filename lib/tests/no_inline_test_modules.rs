//! Source layout rules for every crate in the workspace:
//! - tests live in `tests.rs` / `tests/` files next to the code they test,
//!   never inline in business source files;
//! - business source files stay small and focused.

use std::path::{Path, PathBuf};

/// Largest allowed business source file, in lines.
const MAX_SOURCE_LINES: usize = 800;

fn is_test_file(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "tests.rs")
        || path.components().any(|part| part.as_os_str() == "tests")
}

/// All non-test `.rs` files under every workspace crate's `src/`.
fn business_sources() -> Vec<PathBuf> {
    fn visit(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("readable source dir") {
            let path = entry.expect("readable dir entry").path();
            if path.is_dir() {
                visit(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "rs") && !is_test_file(&path) {
                files.push(path);
            }
        }
    }

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(workspace).expect("readable workspace") {
        let crate_dir = entry.expect("readable dir entry").path();
        if crate_dir.join("Cargo.toml").is_file() && crate_dir.join("src").is_dir() {
            visit(&crate_dir.join("src"), &mut files);
        }
    }
    files.sort();
    files
}

#[test]
fn source_files_have_no_inline_test_modules() {
    let mut offenders = Vec::new();
    for path in business_sources() {
        let source = std::fs::read_to_string(&path).expect("readable source file");
        for (index, line) in source.lines().enumerate() {
            let line = line.trim_start();
            if line.starts_with("mod ") && line.trim_end().ends_with('{') && line.contains("tests")
            {
                offenders.push(format!("{}:{}", path.display(), index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "move these test modules into a sibling `tests.rs` (declare `#[cfg(test)] mod tests;`):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn source_files_stay_small() {
    let offenders: Vec<String> = business_sources()
        .into_iter()
        .filter_map(|path| {
            let lines = std::fs::read_to_string(&path)
                .expect("readable source file")
                .lines()
                .count();
            (lines > MAX_SOURCE_LINES).then(|| format!("{} ({lines} lines)", path.display()))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "split these files into focused submodules (max {MAX_SOURCE_LINES} lines):\n{}",
        offenders.join("\n")
    );
}
