use super::*;

#[test]
fn test_run_config_formats_entry_file_in_place() {
    let base =
        std::env::temp_dir().join(format!("rockc_format_{}_{}", std::process::id(), "entry"));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let source = "// entry docs\n\nmain  =  ->\n    0\n";
    std::fs::write(&entry_file, source).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--format",
    ])
    .unwrap();

    run_config(config).unwrap();

    let formatted = std::fs::read_to_string(&entry_file).unwrap();
    assert_eq!(formatted, "// entry docs\n\nmain = -> 0\n");
    assert_ne!(formatted, source);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_format_preserves_collapsed_body_trivia() {
    let base = std::env::temp_dir().join(format!(
        "rockc_format_{}_{}",
        std::process::id(),
        "collapsed_body_trivia"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let source = "main = ->\n    // body comment\n    0\n\n";
    std::fs::write(&entry_file, source).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--format",
    ])
    .unwrap();

    run_config(config).unwrap();

    let formatted = std::fs::read_to_string(&entry_file).unwrap();
    assert_eq!(formatted, "main = -> 0\n    // body comment\n\n");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_format_preserves_standalone_multiline_block_comment() {
    let base = std::env::temp_dir().join(format!(
        "rockc_format_{}_{}",
        std::process::id(),
        "multiline_block_comment"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let source = "/*\nmodule docs\n*/\nmain  =  ->  0\n";
    std::fs::write(&entry_file, source).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--format",
    ])
    .unwrap();

    run_config(config).unwrap();

    let formatted = std::fs::read_to_string(&entry_file).unwrap();
    assert_eq!(formatted, "/*\nmodule docs\n*/\nmain = -> 0\n");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_format_anchors_trivia_to_next_top_level() {
    let base = std::env::temp_dir().join(format!(
        "rockc_format_{}_{}",
        std::process::id(),
        "next_top_level_trivia"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let source = "foo = ->\n    0\n\n// bar docs\nbar = -> 1\n";
    std::fs::write(&entry_file, source).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--format",
    ])
    .unwrap();

    run_config(config).unwrap();

    let formatted = std::fs::read_to_string(&entry_file).unwrap();
    assert_eq!(formatted, "foo = -> 0\n\n// bar docs\nbar = -> 1\n");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_format_keeps_operator_functions_as_code() {
    let base = std::env::temp_dir().join(format!(
        "rockc_format_{}_{}",
        std::process::id(),
        "operator_function"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let source = "*  =  ->  1\n";
    std::fs::write(&entry_file, source).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--format",
    ])
    .unwrap();

    run_config(config).unwrap();

    let formatted = std::fs::read_to_string(&entry_file).unwrap();
    assert_eq!(formatted, "* = -> 1\n");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_format_preserves_struct_field_docs_before_field() {
    let base = std::env::temp_dir().join(format!(
        "rockc_format_{}_{}",
        std::process::id(),
        "struct_field_docs"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let source = "struct Box\n    // field docs\n    value : ()\n";
    std::fs::write(&entry_file, source).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--format",
    ])
    .unwrap();

    run_config(config).unwrap();

    let formatted = std::fs::read_to_string(&entry_file).unwrap();
    assert_eq!(formatted, "struct Box\n    // field docs\n    value : ()\n");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_format_preserves_statement_docs_between_statements() {
    let base = std::env::temp_dir().join(format!(
        "rockc_format_{}_{}",
        std::process::id(),
        "statement_docs"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let source = "main = ->\n    a = 1\n    // keep between statements\n    b = 2\n";
    std::fs::write(&entry_file, source).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--format",
    ])
    .unwrap();

    run_config(config).unwrap();

    let formatted = std::fs::read_to_string(&entry_file).unwrap();
    assert_eq!(
        formatted,
        "main = ->\n    a = 1\n    // keep between statements\n    b = 2\n"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_format_preserves_same_line_block_comment_before_code() {
    let base = std::env::temp_dir().join(format!(
        "rockc_format_{}_{}",
        std::process::id(),
        "same_line_block_comment"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let source = "/* doc */ main  =  ->  0\n";
    std::fs::write(&entry_file, source).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--format",
    ])
    .unwrap();

    run_config(config).unwrap();

    let formatted = std::fs::read_to_string(&entry_file).unwrap();
    assert_eq!(formatted, "/* doc */\nmain = -> 0\n");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_format_preserves_block_comment_line_comment_remainder() {
    let base = std::env::temp_dir().join(format!(
        "rockc_format_{}_{}",
        std::process::id(),
        "block_comment_line_comment_remainder"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let source = "/* docs */ // extra\nmain  =  ->  0\n";
    std::fs::write(&entry_file, source).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--format",
    ])
    .unwrap();

    run_config(config).unwrap();

    let formatted = std::fs::read_to_string(&entry_file).unwrap();
    assert_eq!(formatted, "/* docs */ // extra\nmain = -> 0\n");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_format_preserves_multiline_block_comment_line_comment_remainder() {
    let base = std::env::temp_dir().join(format!(
        "rockc_format_{}_{}",
        std::process::id(),
        "multiline_block_comment_line_comment_remainder"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let source = "/*\ndocs\n*/ // extra\nmain  =  ->  0\n";
    std::fs::write(&entry_file, source).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--format",
    ])
    .unwrap();

    run_config(config).unwrap();

    let formatted = std::fs::read_to_string(&entry_file).unwrap();
    assert_eq!(formatted, "/*\ndocs\n*/ // extra\nmain = -> 0\n");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_format_preserves_multiline_block_comment_before_code() {
    let base = std::env::temp_dir().join(format!(
        "rockc_format_{}_{}",
        std::process::id(),
        "partial_multiline_block_comment"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    let source = "/*\nmodule docs\n*/ main  =  ->  0\n";
    std::fs::write(&entry_file, source).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        entry_file.to_str().unwrap(),
        "--format",
    ])
    .unwrap();

    run_config(config).unwrap();

    let formatted = std::fs::read_to_string(&entry_file).unwrap();
    assert_eq!(formatted, "/*\nmodule docs\n*/\nmain = -> 0\n");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_rock_lib_format_input_is_public_formatter_api() {
    let base = std::env::temp_dir().join(format!(
        "rockc_format_api_{}_{}",
        std::process::id(),
        "module"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let entry_file = base.join("main.rk");
    std::fs::write(&entry_file, "main  =  ->\n    0\n").unwrap();

    let program = load_program_for_source_command(&rock_lib::Config {
        entry_file,
        ..rock_lib::Config::default()
    })
    .unwrap();
    let source = std::fs::read_to_string(&program.module.filepath.clone().unwrap()).unwrap();
    let formatted = rock_lib::fmt::format(rock_lib::fmt::FormatInput::module_with_source(
        &program.module,
        &source,
    ));

    assert!(formatted.contains("main = ->"));
    assert_ne!(formatted, "main  =  ->\n    0\n");

    let _ = std::fs::remove_dir_all(&base);
}
