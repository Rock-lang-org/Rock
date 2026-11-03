//! Tests live in `tests.rs` / `tests/` files next to the code they test,
//! never inline in business source files.

use std::path::Path;

fn is_test_file(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "tests.rs")
        || path.components().any(|part| part.as_os_str() == "tests")
}

fn visit(dir: &Path, offenders: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("readable source dir") {
        let path = entry.expect("readable dir entry").path();
        if path.is_dir() {
            visit(&path, offenders);
        } else if path.extension().is_some_and(|ext| ext == "rs") && !is_test_file(&path) {
            let source = std::fs::read_to_string(&path).expect("readable source file");
            for (index, line) in source.lines().enumerate() {
                let line = line.trim_start();
                if line.starts_with("mod ") && line.trim_end().ends_with('{') && line.contains("tests") {
                    offenders.push(format!("{}:{}", path.display(), index + 1));
                }
            }
        }
    }
}

#[test]
fn source_files_have_no_inline_test_modules() {
    let mut offenders = Vec::new();
    visit(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut offenders);
    assert!(
        offenders.is_empty(),
        "move these test modules into a sibling `tests.rs` (declare `#[cfg(test)] mod tests;`):\n{}",
        offenders.join("\n")
    );
}
