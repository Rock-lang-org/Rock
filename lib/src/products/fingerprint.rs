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
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::products::ProductSourceFingerprint;
    use crate::{compile_with_products, Config, SourceProvider};

    #[test]
    fn artifact_source_provider_text_contributes_to_product_source_hash() {
        let base = std::env::temp_dir().join(format!(
            "rock_compile_products_{}_{}",
            std::process::id(),
            "artifact_provider_hash"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let first = compile_artifact_provider_fingerprint(&base, "1");
        let second = compile_artifact_provider_fingerprint(&base, "2");

        assert!(first.source_hash.is_some());
        assert!(second.source_hash.is_some());
        assert_ne!(first.source_hash, second.source_hash);

        let _ = fs::remove_dir_all(&base);
    }

    fn compile_artifact_provider_fingerprint(
        base: &Path,
        answer: &str,
    ) -> ProductSourceFingerprint {
        let artifact_path = base.join("demo.rkca");
        let entry_file = PathBuf::from("/artifact/demo/main.rk");
        let util_file = PathBuf::from("/artifact/demo/util.rk");

        compile_with_products(&Config {
            entry_file: entry_file.clone(),
            output_dir: base.join(format!("build-{answer}")),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            source_providers: vec![
                SourceProvider::Artifact {
                    path: entry_file,
                    artifact_path: artifact_path.clone(),
                    text: "mod util\nmain = -> util::answer!\n".to_string(),
                },
                SourceProvider::Artifact {
                    path: util_file,
                    artifact_path,
                    text: format!("answer = -> {answer}\n< answer\n"),
                },
            ],
            ..Config::default()
        })
        .unwrap()
        .products
        .unwrap()
        .crate_identity
        .source_fingerprint
    }

    #[test]
    fn product_source_fingerprint_is_stable_across_checkout_paths() {
        let first =
            std::env::temp_dir().join(format!("rock_fingerprint_first_{}", std::process::id()));
        let second =
            std::env::temp_dir().join(format!("rock_fingerprint_second_{}", std::process::id()));
        let _ = fs::remove_dir_all(&first);
        let _ = fs::remove_dir_all(&second);
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("main.rk"), "main = ->\n    0\n").unwrap();
        fs::write(second.join("main.rk"), "main = ->\n    0\n").unwrap();

        let first_products = compile_with_products(&Config {
            entry_file: first.join("main.rk"),
            output_dir: first.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap()
        .products
        .unwrap();
        let second_products = compile_with_products(&Config {
            entry_file: second.join("main.rk"),
            output_dir: second.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap()
        .products
        .unwrap();

        assert_eq!(
            first_products.crate_identity.source_fingerprint,
            second_products.crate_identity.source_fingerprint
        );
        assert_eq!(
            first_products
                .crate_identity
                .source_fingerprint
                .loaded_files,
            vec![PathBuf::from("main.rk")]
        );

        let _ = fs::remove_dir_all(&first);
        let _ = fs::remove_dir_all(&second);
    }

    #[test]
    fn product_source_fingerprint_ignores_unloaded_rock_files() {
        let base =
            std::env::temp_dir().join(format!("rock_fingerprint_unloaded_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        fs::write(&entry_file, "main = ->\n    0\n").unwrap();

        let before = compile_with_products(&Config {
            entry_file: entry_file.clone(),
            output_dir: base.join("build-before"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap()
        .products
        .unwrap()
        .crate_identity
        .source_fingerprint;

        fs::write(base.join("unused.rk"), "unused = ->\n    1\n").unwrap();

        let after = compile_with_products(&Config {
            entry_file,
            output_dir: base.join("build-after"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap()
        .products
        .unwrap()
        .crate_identity
        .source_fingerprint;

        assert_eq!(before, after);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn product_source_fingerprint_uses_source_loader_loaded_files() {
        let base = std::env::temp_dir().join(format!(
            "rock_fingerprint_loaded_modules_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        fs::write(&entry_file, "mod util\n> util::answer\nmain = -> answer!\n").unwrap();
        fs::write(base.join("util.rk"), "answer = -> 0\n< answer\n").unwrap();
        fs::write(base.join("unloaded.rk"), "answer = -> 1\n").unwrap();

        let output = compile_with_products(&Config {
            entry_file,
            output_dir: base.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap();

        let products = output.products.expect("products should be emitted");
        assert_eq!(
            products.crate_identity.source_fingerprint.loaded_files,
            vec![PathBuf::from("main.rk"), PathBuf::from("util.rk")]
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn product_source_fingerprint_includes_macro_generated_source_modules() {
        let base = std::env::temp_dir().join(format!(
            "rock_fingerprint_macro_modules_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        fs::write(
            &entry_file,
            "macro include_util\n    x =>\n        mod util\n%include_util x\n\nmain = ->\n    0\n",
        )
        .unwrap();
        fs::write(base.join("util.rk"), "answer = -> 0\n< answer\n").unwrap();
        fs::write(base.join("unloaded.rk"), "answer = -> 1\n").unwrap();

        let output = compile_with_products(&Config {
            entry_file,
            output_dir: base.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap();

        let products = output.products.expect("products should be emitted");
        assert_eq!(
            products.crate_identity.source_fingerprint.loaded_files,
            vec![PathBuf::from("main.rk"), PathBuf::from("util.rk")]
        );

        let _ = fs::remove_dir_all(&base);
    }
}
