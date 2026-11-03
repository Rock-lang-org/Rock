use super::*;

#[test]
fn macro_expansion_uses_context_depth_limit_instead_of_panicking() {
    let input = r#"macro repeat
    =>
        %repeat
%repeat"#;
    let config = test_config();
    let input_program = parse_string(input, &config).unwrap();
    let context = MacroExpansionContext::new(&config).with_max_depth(2);

    let result = expand_macros_with_context(input_program, &context);

    let diagnostics = result.expect_err("recursive macro should produce diagnostics");
    let diagnostic = diagnostics
        .0
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .contains("Macro expansion depth exceeded")
        })
        .expect("expected depth diagnostic");
    assert!(diagnostic
        .primary
        .iter()
        .chain(diagnostic.secondary.iter())
        .any(|label| label.message.contains("expanded from macro 'repeat'")));
}

#[test]
fn macro_expansion_depth_trace_matches_first_remaining_invocation() {
    let input = r#"macro repeat_a
    =>
        %repeat_a
macro repeat_b
    =>
        %repeat_b
%repeat_a
%repeat_b"#;
    let config = test_config();
    let input_program = parse_string(input, &config).unwrap();
    let context = MacroExpansionContext::new(&config).with_max_depth(1);

    let result = expand_macros_with_context(input_program, &context);

    let diagnostics = result.expect_err("recursive macros should produce diagnostics");
    let diagnostic = diagnostics
        .0
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .contains("Macro expansion depth exceeded")
        })
        .expect("expected depth diagnostic");

    assert!(diagnostic
        .primary
        .iter()
        .chain(diagnostic.secondary.iter())
        .any(|label| label.message.contains("expanded from macro 'repeat_a'")));
    assert!(!diagnostic
        .primary
        .iter()
        .chain(diagnostic.secondary.iter())
        .any(|label| label.message.contains("expanded from macro 'repeat_b'")));
}

#[test]
fn macro_expansion_depth_trace_ignores_unrelated_latest_expansion() {
    let input = r#"macro known
    =>
        main = -> 0
%unknown
%known"#;
    let config = test_config();
    let input_program = parse_string(input, &config).unwrap();
    let context = MacroExpansionContext::new(&config).with_max_depth(0);

    let result = expand_macros_with_context(input_program, &context);

    let diagnostics = result.expect_err("unknown macro should remain past depth limit");
    let diagnostic = diagnostics
        .0
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .contains("Macro expansion depth exceeded")
        })
        .expect("expected depth diagnostic");

    assert!(!diagnostic
        .primary
        .iter()
        .chain(diagnostic.secondary.iter())
        .any(|label| label.message.contains("expanded from macro 'known'")));
}

#[test]
fn generated_parse_errors_point_back_to_macro_invocation_source() {
    let config = test_config();
    let generated_tokens = vec![
        Token::from(TokenType::Indent(0)),
        Token::from(TokenType::Ident("main".to_string())),
        Token::from(TokenType::Equal),
        Token::from(TokenType::Arrow),
        Token::from(TokenType::Eof),
    ];
    let response = proc_macro::ProcMacroResponse::Expand {
        output: proc_macro::encode_tokens(&generated_tokens).unwrap(),
    };
    let (artifact, temp_dir) = proc_macro_artifact_with_response("broken", response);
    let input_program = parse_string("%broken", &config).unwrap();
    let invocation_span = match &input_program.module.top_levels[0] {
        TopLevel::MacroInvoc(invocation) => invocation.name.span.clone(),
        other => panic!("expected macro invocation, got {other:?}"),
    };
    let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);

    let diagnostics = expand_macros_with_context(input_program, &context)
        .expect_err("invalid generated source should fail parsing");
    let diagnostic = diagnostics
        .0
        .first()
        .expect("expected generated parse diagnostic");
    assert_eq!(
        diagnostic.location,
        crate::diagnostic::DiagnosticLocation::Source(invocation_span)
    );

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn nested_macro_diagnostics_include_parent_expansion_trace_label() {
    let input = r#"macro inner
    =>
        main =
macro outer
    =>
        %inner
%outer"#;
    let config = test_config();
    let input_program = parse_string(input, &config).unwrap();
    let context = MacroExpansionContext::new(&config);

    let diagnostics = expand_macros_with_context(input_program, &context)
        .expect_err("inner macro output should fail parsing");

    let labels = diagnostics
        .0
        .iter()
        .flat_map(|diagnostic| diagnostic.primary.iter().chain(diagnostic.secondary.iter()))
        .map(|label| label.message.as_str())
        .collect::<Vec<_>>();
    assert!(
        labels
            .iter()
            .any(|label| label.contains("expanded from macro 'inner'")),
        "labels: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|label| label.contains("expanded from macro 'outer'")),
        "labels: {labels:?}"
    );
}

#[test]
fn repeated_generated_macro_invocations_keep_parent_trace_order() {
    let input = r#"macro inner
    =>
        main =
macro outer
    =>
        %inner
%outer
%outer"#;
    let config = test_config();
    let input_program = parse_string(input, &config).unwrap();
    let outer_spans = input_program
        .module
        .top_levels
        .iter()
        .filter_map(|top_level| match top_level {
            TopLevel::MacroInvoc(invocation) if invocation.name.name == "outer" => {
                Some(invocation.name.span.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(outer_spans.len(), 2);
    let context = MacroExpansionContext::new(&config);

    let diagnostics = expand_macros_with_context(input_program, &context)
        .expect_err("first inner macro output should fail parsing");

    let outer_trace_spans = diagnostics
        .0
        .iter()
        .flat_map(|diagnostic| diagnostic.primary.iter().chain(diagnostic.secondary.iter()))
        .filter_map(|label| {
            label
                .message
                .contains("expanded from macro 'outer'")
                .then_some(label.span.clone())
        })
        .collect::<Vec<_>>();
    assert!(outer_trace_spans
        .iter()
        .any(|span| span.start == outer_spans[0].start && span.end == outer_spans[0].end));
    assert!(!outer_trace_spans
        .iter()
        .any(|span| span.start == outer_spans[1].start && span.end == outer_spans[1].end));
}

#[test]
fn selected_arm_template_errors_do_not_fall_through_to_later_arms() {
    let input = r#"macro mymacro
    $a:ident =>
        $missing = -> 1
    $a:ident =>
        main = -> 0
%mymacro main"#;
    let config = test_config();
    let input_program = parse_string(input, &config).unwrap();
    let context = MacroExpansionContext::new(&config);

    let result = expand_macros_with_context(input_program, &context);

    assert!(result.is_err());
}

#[test]
fn expression_macro_depth_limit_stops_runaway_expansion() {
    let config = test_config();
    let context = MacroExpansionContext::new(&config).with_max_depth(5);
    let input = "macro forever\n    $x:expr =>\n        %forever $x\nmain = -> %forever 1\n";
    let program = parse_string(input, &config).unwrap();

    let diagnostics = expand_macros_with_context(program, &context)
        .expect_err("runaway expression expansion should stop");

    assert!(diagnostics.contains("depth exceeded"));
}
