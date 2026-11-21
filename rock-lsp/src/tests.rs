use std::path::PathBuf;

use rock_lib::diagnostic::Diagnostic as RockDiagnostic;
use rock_lib::Span;
use tower_lsp::lsp_types::{DiagnosticSeverity, InlayHintKind, InlayHintLabel, Position, Range, Url};

use super::{
    analysis_config, byte_offset, diagnostic_to_lsp, position_at, type_hints_to_lsp, Settings,
};

#[test]
fn regression_clears_cross_file_diagnostics_only_for_the_publishing_target() {
    let entry = PathBuf::from("/virtual/main.rk");
    let other_entry = PathBuf::from("/virtual/other.rk");
    let trigger = Url::from_file_path(&entry).unwrap();
    let helper_path = PathBuf::from("/virtual/helpers.rk");
    let helper = Url::from_file_path(&helper_path).unwrap();
    let mut state = super::State::default();
    state.documents.insert(
        helper.clone(),
        super::Document {
            version: 7,
            text: "helper = -> 1\n".into(),
        },
    );
    let mut warnings = rock_lib::diagnostic::Diagnostics::default();
    warnings.push(RockDiagnostic::new(
        "unused helper".into(),
        Span::new(helper_path, 0, 6),
    ));

    let updates = state.diagnostic_updates(&entry, &trigger, &warnings);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].2.len(), 1);
    let empty = Default::default();
    assert!(state
        .diagnostic_updates(&other_entry, &trigger, &empty)
        .is_empty());
    assert_eq!(
        state.diagnostic_updates(&entry, &trigger, &empty),
        vec![(helper, 7, vec![])]
    );
    assert!(state
        .diagnostic_updates(&entry, &trigger, &empty)
        .is_empty());
}

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
fn inlay_type_hints_only_label_unannotated_bindings_in_range() {
    let path = PathBuf::from("/virtual/main.rk");
    let text = "main = ->\n    first = 1\n    typed: I64 = first\n    last = typed\n    last\n";
    let config = analysis_config(
        path.clone(),
        vec![rock_lib::SourceProvider::Virtual {
            path: path.clone(),
            text: text.to_string(),
        }],
        &Settings {
            extern_artifacts: Vec::new(),
            no_prelude: true,
        },
        None,
    );
    let analysis = rock_lib::analyze(&config).unwrap();
    let range = Range::new(Position::new(2, 0), Position::new(4, 8));
    let hints = type_hints_to_lsp(&analysis, &path, text, range);

    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].position, Position::new(3, 8));
    assert!(matches!(&hints[0].label, InlayHintLabel::String(label) if label == ": I64"));
    assert_eq!(hints[0].kind, Some(InlayHintKind::TYPE));
    assert!(type_hints_to_lsp(&analysis, &path.with_file_name("other.rk"), text, range).is_empty());
    let before_hint = Range::new(Position::new(3, 0), Position::new(3, 8));
    assert!(type_hints_to_lsp(&analysis, &path, text, before_hint).is_empty());
    let after_hint = Range::new(Position::new(3, 8), Position::new(4, 0));
    assert_eq!(type_hints_to_lsp(&analysis, &path, text, after_hint).len(), 1);
}

#[test]
fn editing_an_open_module_updates_hints_in_an_unchanged_buffer() {
    use tower_lsp::lsp_types::{
        DidChangeTextDocumentParams, DidOpenTextDocumentParams, InlayHintParams,
        TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
        VersionedTextDocumentIdentifier,
    };
    use tower_lsp::{LanguageServer, LspService};

    super::runtime().unwrap().block_on(async {
        let (service, _socket) = LspService::new(|client| {
            super::Backend::new(
                client,
                Settings {
                    extern_artifacts: Vec::new(),
                    no_prelude: true,
                },
            )
        });
        let backend = service.inner();
        let main = Url::parse("file:///virtual/main.rk").unwrap();
        let module = Url::parse("file:///virtual/values.rk").unwrap();
        let unrelated = Url::parse("file:///other/main.rk").unwrap();
        for (uri, text) in [
            (unrelated.clone(), "main = -> 0\n"),
            (module.clone(), "< answer = -> 42\n"),
            (
                main.clone(),
                "mod values\n> values::answer\nmain = !->\n    value = answer!\n",
            ),
        ] {
            backend
                .did_open(DidOpenTextDocumentParams {
                    text_document: TextDocumentItem::new(uri, "rock".into(), 1, text.into()),
                })
                .await;
        }
        let request = || InlayHintParams {
            work_done_progress_params: Default::default(),
            text_document: TextDocumentIdentifier::new(main.clone()),
            range: Range::new(Position::new(3, 0), Position::new(3, 18)),
        };
        let hints = backend.inlay_hint(request()).await.unwrap().unwrap();
        assert!(matches!(&hints[0].label, InlayHintLabel::String(label) if label == ": I64"));
        let unrelated_analysis = backend.state.read().await.analyses[&unrelated].1.clone();

        backend
            .did_change(DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier::new(module, 2),
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: "< answer = -> true\n".into(),
                }],
            })
            .await;
        let hints = backend.inlay_hint(request()).await.unwrap().unwrap();
        assert!(matches!(&hints[0].label, InlayHintLabel::String(label) if label == ": Bool"));
        let state = backend.state.read().await;
        let (current, analysis) = &state.analyses[&unrelated];
        assert!(*current);
        assert!(std::sync::Arc::ptr_eq(analysis, &unrelated_analysis));
    });
}

#[test]
fn open_buffers_for_the_same_target_share_analysis() {
    use tower_lsp::lsp_types::{DidOpenTextDocumentParams, TextDocumentItem};
    use tower_lsp::{LanguageServer, LspService};

    let root = std::env::temp_dir().join(format!("rock_lsp_shared_target_{}", std::process::id()));
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        root.join("rock.toml"),
        "[crate]\nname = \"shared\"\nversion = \"0.1.0\"\nno_std = true\n\n[[bin]]\n",
    )
    .unwrap();
    let main = Url::from_file_path(src.join("main.rk")).unwrap();
    let helper = Url::from_file_path(src.join("helper.rk")).unwrap();

    super::runtime().unwrap().block_on(async {
        let (service, _socket) = LspService::new(|client| {
            super::Backend::new(
                client,
                Settings {
                    extern_artifacts: Vec::new(),
                    no_prelude: true,
                },
            )
        });
        let backend = service.inner();
        let open = |uri, text: &str| DidOpenTextDocumentParams {
            text_document: TextDocumentItem::new(uri, "rock".into(), 1, text.into()),
        };
        backend
            .did_open(open(
                main.clone(),
                "mod helper\n> helper::value\nmain = -> value!\n",
            ))
            .await;
        backend
            .did_open(open(helper.clone(), "< value = -> 42\n"))
            .await;
        let state = backend.state.read().await;
        let (main_current, main_analysis) = &state.analyses[&main];
        let (helper_current, helper_analysis) = &state.analyses[&helper];
        assert!(*main_current && *helper_current);
        assert!(std::sync::Arc::ptr_eq(main_analysis, helper_analysis));
    });
    std::fs::remove_dir_all(root).unwrap();
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
