use super::*;

#[test]
fn test_source_command_parse_errors_are_structured_diagnostics() {
    let base = std::env::temp_dir().join(format!(
        "rockc_source_command_diagnostics_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    std::fs::write(&entry_file, "main = ->\n    `\n").unwrap();

    let diagnostics = load_program_for_source_command(&rock_lib::Config {
        entry_file,
        ..rock_lib::Config::default()
    })
    .unwrap_err();
    let messages = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(messages.contains("Unexpected token") || messages.contains("Lexer"));
    assert!(!messages.contains("Failed to load source graph"));
    assert!(!messages.contains("Parse {"));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_expands_entry_file_without_compiling() {
    let base =
        std::env::temp_dir().join(format!("rockc_expand_{}_{}", std::process::id(), "entry"));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let output_dir = base.join("build");
    std::fs::write(&entry_file, "main = -> 0\n").unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--output-dir",
        output_dir.to_str().unwrap(),
        "--expand",
    ])
    .unwrap();

    run_config(config).unwrap();

    assert!(!output_dir.join("main").exists());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn source_command_loader_keeps_virtual_source_for_diagnostic_attachment() {
    let path = PathBuf::from("/virtual/rockc/main.rk");
    let text = "main = -> 0\n";
    let config = rock_lib::Config {
        entry_file: path.clone(),
        no_std: true,
        no_prelude: true,
        source_providers: vec![rock_lib::SourceProvider::Virtual {
            path: path.clone(),
            text: text.to_string(),
        }],
        ..rock_lib::Config::default()
    };

    let (program, source_db) = load_program_with_source_database(&config).unwrap();
    assert_eq!(program.module.filepath, Some(path.clone()));
    let sources = rock_lib::diagnostic::DiagnosticSourceMap::from_source_database(&source_db);
    let mut diagnostic = rock_lib::diagnostic::Diagnostic::new(
        "source command failure".to_string(),
        rock_lib::Span::new(path.clone(), 0, 4),
    );
    sources.attach(&mut diagnostic);

    assert_eq!(diagnostic.source.as_ref().unwrap().text, text);
}
