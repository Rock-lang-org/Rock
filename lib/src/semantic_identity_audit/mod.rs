use std::fs;
use std::path::{Path, PathBuf};

fn lib_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Files that replaced `path` when `X.rs` was split into `X/mod.rs` + `X/<topic>.rs`.
/// Returns `None` when `path` still exists as a single file.
fn split_module_files(path: &str) -> Option<Vec<String>> {
    if lib_src().join(path).is_file() {
        return None;
    }
    let dir = path.strip_suffix(".rs")?;
    if !lib_src().join(dir).is_dir() {
        return None;
    }
    Some(production_rust_source_files_under(dir))
}

fn production_source(path: &str) -> String {
    if let Some(files) = split_module_files(path) {
        return files
            .iter()
            .map(|file| production_source(file))
            .collect::<Vec<_>>()
            .join("");
    }
    let source = fs::read_to_string(lib_src().join(path)).expect("source file should be readable");
    strip_cfg_test_items(
        source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap_or(source.as_str()),
    )
}

/// Production source of every non-test `.rs` file under `dir`, in sorted order.
fn production_tree_source(dir: &str) -> String {
    production_rust_source_files_under(dir)
        .iter()
        .map(|file| production_source(file))
        .collect::<Vec<_>>()
        .join("")
}

/// Production source of the listed files, concatenated in the given order.
fn production_sources(paths: &[&str]) -> String {
    paths
        .iter()
        .map(|file| production_source(file))
        .collect::<Vec<_>>()
        .join("")
}

/// Files that together hold what `codegen/mod.rs` held before it was split.
const CODEGEN_MOD_FILES: &[&str] = &[
    "codegen/mod.rs",
    "codegen/callable_resolution.rs",
    "codegen/codegen_error.rs",
    "codegen/contract_validation.rs",
    "codegen/declarations.rs",
];

fn full_source(path: &str) -> String {
    if let Some(files) = split_module_files(path) {
        return files
            .iter()
            .map(|file| full_source(file))
            .collect::<Vec<_>>()
            .join("");
    }
    fs::read_to_string(lib_src().join(path)).expect("source file should be readable")
}

fn rust_source_files_under(path: &str) -> Vec<String> {
    fn collect_rust_sources(dir: &Path, root: &Path, files: &mut Vec<String>) {
        for entry in fs::read_dir(dir).expect("source directory should be readable") {
            let entry = entry.expect("source directory entry should be readable");
            let path = entry.path();
            if path.is_dir() {
                collect_rust_sources(&path, root, files);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let relative = path
                    .strip_prefix(root)
                    .expect("source file should be under lib src");
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }

    let root = lib_src();
    let mut files = Vec::new();
    collect_rust_sources(&root.join(path), &root, &mut files);
    files.sort();
    files
}

fn production_rust_source_files_under(path: &str) -> Vec<String> {
    rust_source_files_under(path)
        .into_iter()
        .filter(|path| {
            !path.split('/').any(|component| component == "tests") && !path.ends_with("/tests.rs")
        })
        .collect()
}

fn assert_source_file_missing(path: &str) {
    assert!(
        !lib_src().join(path).exists(),
        "semantic identity audit expects `{path}` to be deleted"
    );
}

fn strip_cfg_test_items(source: &str) -> String {
    let mut output = String::new();
    let mut lines = source.lines();

    while let Some(line) = lines.next() {
        if line.trim() == "#[cfg(test)]" {
            let mut item = String::new();
            let mut depth = 0usize;
            let mut saw_open = false;

            for item_line in lines.by_ref() {
                item.push_str(item_line);
                item.push('\n');
                for ch in item_line.chars() {
                    match ch {
                        '{' => {
                            saw_open = true;
                            depth += 1;
                        }
                        '}' => depth = depth.saturating_sub(1),
                        _ => {}
                    }
                }
                if saw_open && depth == 0 {
                    break;
                }
                if !saw_open && item_line.trim_end().ends_with(';') {
                    break;
                }
            }
            continue;
        }

        output.push_str(line);
        output.push('\n');
    }

    output
}

fn function_body(source: &str, signature: &str) -> String {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("missing function signature: {signature}"));
    let source = &source[start..];
    let open = source
        .find('{')
        .unwrap_or_else(|| panic!("missing function body: {signature}"));
    let mut depth = 0usize;
    let mut end = None;

    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(open + offset + ch.len_utf8());
                    break;
                }
            }
            _ => {}
        }
    }

    source[..end.unwrap_or_else(|| panic!("unterminated function body: {signature}"))].to_string()
}

fn assert_absent(source: &str, forbidden: &[&str]) {
    for pattern in forbidden {
        assert!(
            !source.contains(pattern),
            "semantic identity audit forbids production pattern `{pattern}`"
        );
    }
}

fn assert_present(source: &str, required: &[&str]) {
    for pattern in required {
        assert!(
            source.contains(pattern),
            "semantic identity audit requires production pattern `{pattern}`"
        );
    }
}

fn assert_production_file_absent(path: &str, forbidden: &[&str]) {
    let source = production_source(path);
    assert_absent(&source, forbidden);
}

fn assert_full_source_file_absent(path: &str, forbidden: &[&str]) {
    let source = full_source(path);
    assert_absent(&source, forbidden);
}

fn assert_production_tree_absent(path: &str, forbidden: &[&str]) {
    for source_path in production_rust_source_files_under(path) {
        assert_production_file_absent(&source_path, forbidden);
    }
}

fn is_audit_source(path: &str) -> bool {
    path == "semantic_identity_audit.rs" || path.starts_with("semantic_identity_audit/")
}

mod codegen_backend;
mod crate_boundaries;
mod display_name_identity;
mod mir_and_lowering_ownership;
mod product_identity;
mod selection_and_service_policy;
