use std::path::PathBuf;

use rock_lib::diagnostic::Diagnostic as RockDiagnostic;
use rock_lib::Span;
use tower_lsp::lsp_types::{DiagnosticSeverity, Position, Url};

use super::{analysis_config, byte_offset, diagnostic_to_lsp, position_at, Settings};

#[test]
fn mixed_package_binary_entries_analyze_with_their_own_library() {
    let root = std::env::temp_dir().join(format!("rock_lsp_multi_target_{}", std::process::id()));
    std::fs::create_dir_all(root.join("src/bin")).unwrap();
    std::fs::write(
        root.join("rock.toml"),
        concat!(
            "[crate]\nname = \"mixed\"\nversion = \"0.1.0\"\nno_std = true\n\n",
            "[lib]\n\n[[bin]]\n\n[[bin]]\nname = \"other\"\n",
        ),
    )
    .unwrap();
    std::fs::write(root.join("src/lib.rk"), "< answer = -> 42\n").unwrap();
    let settings = Settings {
        extern_artifacts: Vec::new(),
        no_prelude: false,
    };
    for entry in ["src/main.rk", "src/bin/other.rk"] {
        let path = root.join(entry);
        std::fs::write(&path, "> mixed::answer\nmain = -> answer!\n").unwrap();
        let project = rock::project::resolve_analysis_project_for_source(&root, &path).unwrap();
        assert_eq!(project.entry_file, path);
        assert!(project
            .extern_artifacts
            .iter()
            .any(|(name, _)| name == "mixed"));
        let config = analysis_config(path, Vec::new(), &settings, Some(&project));
        let result = rock_lib::analyze(&config);
        assert!(result.is_ok(), "{:?}", result.err());
    }
    let project = rock::project::resolve_analysis_project(&root).unwrap();
    assert_eq!(project.entry_file, root.join("src/lib.rk"));
    assert!(!project
        .extern_artifacts
        .iter()
        .any(|(name, _)| name == "mixed"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn runtime_analyzes_nested_source_on_blocking_worker() {
    super::runtime().unwrap().block_on(async {
        tokio::task::spawn_blocking(|| {
            let path = PathBuf::from("/virtual/nested.rk");
            let source = format!(
                "main = ->\n    value = {}0{}\n    0\n",
                "[".repeat(32),
                "]".repeat(32),
            );
            let config = analysis_config(
                path.clone(),
                vec![rock_lib::SourceProvider::Virtual { path, text: source }],
                &Settings {
                    extern_artifacts: Vec::new(),
                    no_prelude: true,
                },
                None,
            );
            assert!(rock_lib::analyze(&config).is_ok());
        })
        .await
        .unwrap();
    });
}

#[test]
fn positions_use_utf16_code_units() {
    let text = "a😀b\nrock";
    assert_eq!(byte_offset(text, Position::new(0, 3)), Some(5));
    assert_eq!(position_at(text, 5), Position::new(0, 3));
    assert_eq!(byte_offset(text, Position::new(1, 2)), Some(9));
}

#[test]
fn diagnostics_convert_byte_ranges_to_utf16_ranges() {
    let path = PathBuf::from("/virtual/main.rk");
    let uri = Url::from_file_path(&path).unwrap();
    let diagnostic = RockDiagnostic::new("bad expression".to_string(), Span::new(path, 1, 5));

    let converted = diagnostic_to_lsp(&diagnostic, &uri, "a😀b").unwrap();
    assert_eq!(converted.range.start, Position::new(0, 1));
    assert_eq!(converted.range.end, Position::new(0, 3));
    assert_eq!(converted.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(converted.source.as_deref(), Some("rock"));
}

#[test]
fn project_configuration_discovers_artifacts_and_applies_explicit_overrides() {
    let project = rock::project::AnalysisProject {
        root_dir: PathBuf::from("/project"),
        entry_file: PathBuf::from("/project/src/main.rk"),
        current_crate_name: Some("app".to_string()),
        extern_artifacts: vec![
            ("dep".to_string(), PathBuf::from("/project/build/dep.rkca")),
            ("std".to_string(), PathBuf::from("/toolchain/std.rkca")),
        ],
        no_prelude: false,
        no_std: false,
    };
    let settings = Settings {
        extern_artifacts: vec![
            ("dep".to_string(), PathBuf::from("/override/dep.rkca")),
            ("extra".to_string(), PathBuf::from("/override/extra.rkca")),
        ],
        no_prelude: false,
    };

    let config = analysis_config(
        PathBuf::from("/project/src/module.rk"),
        Vec::new(),
        &settings,
        Some(&project),
    );

    assert_eq!(config.entry_file, PathBuf::from("/project/src/main.rk"));
    assert_eq!(config.current_crate_name.as_deref(), Some("app"));
    assert!(!config.no_prelude);
    assert!(!config.no_std);
    assert_eq!(
        config.extern_artifacts,
        vec![
            ("dep".to_string(), PathBuf::from("/override/dep.rkca")),
            ("extra".to_string(), PathBuf::from("/override/extra.rkca")),
            ("std".to_string(), PathBuf::from("/toolchain/std.rkca")),
        ]
    );
}
