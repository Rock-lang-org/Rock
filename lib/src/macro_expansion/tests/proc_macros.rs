use super::*;

#[test]
fn function_like_proc_macro_expands_top_level_invocation() {
    let config = test_config();
    let generated_tokens = vec![
        Token::from(TokenType::Indent(0)),
        Token::from(TokenType::Ident("main".to_string())),
        Token::from(TokenType::Equal),
        Token::from(TokenType::Arrow),
        Token::from(TokenType::Number("0".to_string())),
        Token::from(TokenType::Eol),
        Token::from(TokenType::Indent(0)),
        Token::from(TokenType::Eol),
        Token::from(TokenType::Eof),
    ];
    let response = proc_macro::ProcMacroResponse::Expand {
        output: proc_macro::encode_tokens(&generated_tokens).unwrap(),
    };
    let (artifact, temp_dir) = proc_macro_artifact_with_response("make_main", response);
    let input_program = parse_string("%make_main", &config).unwrap();
    let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);

    let expanded = expand_macros_with_context(input_program, &context).unwrap();

    assert!(!expanded.module.has_macro_invoc());
    assert!(expanded.module.top_level_from_ident("main").is_some());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn proc_macro_diagnostics_use_invocation_span() {
    let config = test_config();
    let (artifact, temp_dir) = proc_macro_artifact_with_response(
        "fail_macro",
        proc_macro::ProcMacroResponse::Diagnostics {
            messages: vec!["boom".to_string()],
        },
    );
    let input_program = parse_string("%fail_macro", &config).unwrap();
    let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);

    let diagnostics = expand_macros_with_context(input_program, &context)
        .expect_err("proc macro diagnostics should fail expansion");

    let diagnostic = diagnostics.0.first().expect("expected diagnostic");
    assert!(diagnostic.message.contains("fail_macro"));
    assert_eq!(
        diagnostic.code,
        Some(crate::diagnostic::DiagnosticCode::Macro)
    );
    let crate::diagnostic::DiagnosticLocation::Source(span) = &diagnostic.location else {
        panic!("macro diagnostic should have a source location");
    };
    assert!(span.end > span.start);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn proc_macro_diagnostics_include_expansion_trace_label() {
    let config = test_config();
    let (artifact, temp_dir) = proc_macro_artifact_with_response(
        "fail_macro",
        proc_macro::ProcMacroResponse::Diagnostics {
            messages: vec!["boom".to_string()],
        },
    );
    let input_program = parse_string("%fail_macro", &config).unwrap();
    let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);

    let diagnostics = expand_macros_with_context(input_program, &context)
        .expect_err("proc macro diagnostics should fail expansion");

    let diagnostic = diagnostics.0.first().expect("expected diagnostic");
    assert!(diagnostic
        .primary
        .iter()
        .chain(diagnostic.secondary.iter())
        .any(|label| label.message.contains("expanded from macro 'fail_macro'")));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn proc_macro_expansion_appends_missing_eof_to_generated_tokens() {
    let config = test_config();
    let generated_tokens = vec![
        Token::from(TokenType::Indent(0)),
        Token::from(TokenType::Ident("main".to_string())),
        Token::from(TokenType::Equal),
        Token::from(TokenType::Arrow),
        Token::from(TokenType::Number("0".to_string())),
        Token::from(TokenType::Eol),
        Token::from(TokenType::Indent(0)),
        Token::from(TokenType::Eol),
    ];
    let (artifact, temp_dir) = proc_macro_artifact_with_response(
        "make_main",
        proc_macro::ProcMacroResponse::Expand {
            output: proc_macro::encode_tokens(&generated_tokens).unwrap(),
        },
    );
    let input_program = parse_string("%make_main", &config).unwrap();
    let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);

    let expanded = expand_macros_with_context(input_program, &context).unwrap();

    assert!(expanded.module.top_level_from_ident("main").is_some());

    let _ = fs::remove_dir_all(temp_dir);
}
