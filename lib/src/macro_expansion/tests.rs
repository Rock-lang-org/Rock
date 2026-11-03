use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{language_items::LanguageItemRole, parser::parse_string, Config};

use super::*;

fn proc_macro_artifact_with_response(
    macro_name: &str,
    response: proc_macro::ProcMacroResponse,
) -> (proc_macro::ProcMacroArtifact, PathBuf) {
    let response = proc_macro::encode_response(&response).unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "rock-proc-macro-test-{}-{}",
        std::process::id(),
        nonce
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let host = temp_dir.join("proc_macro_host");
    fs::write(host.with_extension("response"), response).unwrap();
    fs::write(&host, "#!/bin/sh\ncat >/dev/null\ncat \"$0.response\"\n").unwrap();
    let mut permissions = fs::metadata(&host).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&host, permissions).unwrap();

    let artifact = proc_macro::ProcMacroArtifact {
        artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "test_macros".to_string(),
        protocol_version: proc_macro::PROC_MACRO_PROTOCOL_VERSION,
        host_triple: "x86_64-unknown-linux-gnu".to_string(),
        executable: host,
        capabilities: vec![proc_macro::ProcMacroCapability::Stdio],
        exports: vec![proc_macro::ProcMacroExport {
            name: macro_name.to_string(),
            identity: format!("test_macros::{macro_name}"),
            kind: proc_macro::ProcMacroKind::FunctionLike,
            input_shape: proc_macro::ProcMacroInputShape::TokenStream,
        }],
    };

    (artifact, temp_dir)
}

fn test_config() -> Config {
    Config {
        entry_file: PathBuf::from("/test.rk"),
        output_dir: PathBuf::new(),
        debug_print: Vec::new(),
        meta_files: Vec::new(),
        extern_artifacts: Vec::new(),
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: false,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    }
}

fn assert_program_semantically_eq(actual: &Program, expected: &Program) {
    assert_eq!(
        crate::fmt::format(crate::fmt::FormatInput::program(actual)),
        crate::fmt::format(crate::fmt::FormatInput::program(expected)),
    );
}

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
fn macro_arg_matcher_uses_context_config_for_expr_parsing() {
    let input = r#"macro id
    $value:expr =>
        main = -> $value
%id 1 + 2"#;
    let config = Config {
        debug_print: vec![crate::DebugPrint::Tokens],
        ..test_config()
    };
    let input_program = parse_string(input, &config).unwrap();
    let context = MacroExpansionContext::new(&config);

    let expanded = expand_macros_with_context(input_program, &context).unwrap();

    assert!(!expanded.module.has_macro_invoc());
}

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
fn declarative_repetition_expands_each_capture_group() {
    let input = r#"macro defs
    $( $name:ident )* =>
        $( $name = -> 1 )*
%defs one two three"#;
    let expected = r#"macro defs
    $( $name:ident )* =>
        $( $name = -> 1 )*
one = -> 1
two = -> 1
three = -> 1"#;
    let config = test_config();
    let context = MacroExpansionContext::new(&config);

    let expanded =
        expand_macros_with_context(parse_string(input, &config).unwrap(), &context).unwrap();
    let expected = parse_string(expected, &config).unwrap();

    assert_program_semantically_eq(&expanded, &expected);
}

#[test]
fn declarative_repetition_can_use_direct_outer_capture() {
    let input = r#"macro defs
    $ret:ident $( $name:ident )* =>
        $( $name = -> $ret )*
%defs value one two"#;
    let expected = r#"macro defs
    $ret:ident $( $name:ident )* =>
        $( $name = -> $ret )*
one = -> value
two = -> value"#;
    let config = test_config();
    let context = MacroExpansionContext::new(&config);

    let expanded =
        expand_macros_with_context(parse_string(input, &config).unwrap(), &context).unwrap();
    let expected = parse_string(expected, &config).unwrap();

    assert_program_semantically_eq(&expanded, &expected);
}

fn assert_expands_to(input: &str, expected: &str) {
    let config = test_config();
    let context = MacroExpansionContext::new(&config);

    let expanded =
        expand_macros_with_context(parse_string(input, &config).unwrap(), &context).unwrap();
    let expected = parse_string(expected, &config).unwrap();

    assert_program_semantically_eq(&expanded, &expected);
}

const INLINE_JOIN_MACRO: &str = r#"macro join
    $name:ident $first:expr $( , $rest:expr )* =>
        $name = -> ($first).show! $( .concat (($rest).show!) )*
"#;

#[test]
fn inline_repetition_ending_with_paren_keeps_its_paren() {
    assert_expands_to(
        &format!("{INLINE_JOIN_MACRO}%join joined a, b, c"),
        &format!(
            "{INLINE_JOIN_MACRO}joined = -> (a).show! .concat ((b).show!) .concat ((c).show!)"
        ),
    );
}

#[test]
fn inline_repetition_with_one_copy_stays_on_its_line() {
    assert_expands_to(
        &format!("{INLINE_JOIN_MACRO}%join joined a, b"),
        &format!("{INLINE_JOIN_MACRO}joined = -> (a).show! .concat ((b).show!)"),
    );
}

#[test]
fn inline_repetition_with_no_copy_keeps_the_rest_of_its_line() {
    assert_expands_to(
        &format!("{INLINE_JOIN_MACRO}%join single a\n%join other b"),
        &format!("{INLINE_JOIN_MACRO}single = -> (a).show!\nother = -> (b).show!"),
    );
}

#[test]
fn whole_line_repetition_with_no_copy_keeps_following_lines() {
    let declaration = r#"macro defs
    $( $name:ident )* =>
        $( $name = -> 1 )*
        last = -> 2
"#;
    assert_expands_to(
        &format!("{declaration}%defs"),
        &format!("{declaration}last = -> 2"),
    );
    assert_expands_to(
        &format!("{declaration}%defs one two"),
        &format!("{declaration}one = -> 1\ntwo = -> 1\nlast = -> 2"),
    );
}

const TWICE_MACRO: &str = r#"macro twice
    $x:expr =>
        ($x) + ($x)
"#;

#[test]
fn expression_macro_expands_inside_function_body() {
    assert_expands_to(
        &format!("{TWICE_MACRO}main = ->\n    a = %twice 1\n    a\n"),
        &format!("{TWICE_MACRO}main = ->\n    a = ((1) + (1))\n    a\n"),
    );
}

#[test]
fn expression_macro_expands_inside_call_arguments_and_nested_blocks() {
    assert_expands_to(
            &format!(
                "{TWICE_MACRO}main = ->\n    if true\n        f (%twice 1), (%twice 2)\n    else\n        0\n"
            ),
            &format!(
                "{TWICE_MACRO}main = ->\n    if true\n        f (((1) + (1))), (((2) + (2)))\n    else\n        0\n"
            ),
        );
}

#[test]
fn expression_macro_expands_calls_produced_by_its_expansion() {
    let declarations =
        format!("{TWICE_MACRO}macro quad\n    $x:expr =>\n        %twice (%twice $x)\n");
    assert_expands_to(
        &format!("{declarations}main = -> %quad 3\n"),
        &format!("{declarations}main = -> ((((((3) + (3)))) + ((((3) + (3))))))\n"),
    );
}

#[test]
fn expression_macro_inline_repetition_joins_arguments() {
    let declaration = r#"macro join
    $first:expr $( , $rest:expr )* =>
        ($first).show! $( .concat (($rest).show!) )*
"#;
    assert_expands_to(
            &format!("{declaration}main = ->\n    s = %join \"a\", b, 1\n    s\n"),
            &format!(
                "{declaration}main = ->\n    s = ((\"a\").show! .concat ((b).show!) .concat ((1).show!))\n    s\n"
            ),
        );
}

#[test]
fn expression_macro_errors_name_unknown_macros() {
    let config = test_config();
    let context = MacroExpansionContext::new(&config);
    let program = parse_string("main = -> %missing 1\n", &config).unwrap();

    let diagnostics = expand_macros_with_context(program, &context)
        .expect_err("an unknown expression macro should be reported");

    assert!(diagnostics.contains("Unknown macro '%missing'"));
}

#[test]
fn expression_macro_errors_when_expansion_is_not_an_expression() {
    let config = test_config();
    let context = MacroExpansionContext::new(&config);
    let input = "macro assign\n    $x:expr =>\n        y = $x\nmain = -> %assign 1\n";
    let program = parse_string(input, &config).unwrap();

    let diagnostics = expand_macros_with_context(program, &context)
        .expect_err("a non-expression expansion should be reported");

    assert!(diagnostics.contains("single expression"));
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

#[test]
fn matcher_repetition_can_contain_parenthesized_tokens() {
    let declaration = r#"macro defs
    $( ( $name:ident ) )* =>
        $( $name = -> 1 )*
"#;
    assert_expands_to(
        &format!("{declaration}%defs (one) (two)"),
        &format!("{declaration}one = -> 1\ntwo = -> 1"),
    );
}

#[test]
fn capture_set_from_correspondance_preserves_repeated_direct_streams() {
    let mut correspondance = Correspondance::new();
    correspondance.insert_direct(
        "name".to_string(),
        vec![Token::from(TokenType::Ident("first".to_string()))],
    );
    correspondance.insert_direct(
        "name".to_string(),
        vec![Token::from(TokenType::Ident("second".to_string()))],
    );
    let captures = capture_set_from_correspondance(&correspondance, ExpansionId(1), &Span::test());
    let template = declarative::MacroTemplate {
        fragments: vec![declarative::TemplateFragment::Capture {
            name: "name".to_string(),
        }],
    };

    let expanded = template.expand(&captures).unwrap().to_tokens();

    assert_eq!(expanded.len(), 2);
    assert!(matches!(
        expanded[0].token_type,
        TokenType::Ident(ref name) if name == "first"
    ));
    assert!(matches!(
        expanded[1].token_type,
        TokenType::Ident(ref name) if name == "second"
    ));
}

#[test]
fn simple_macro_expand() {
    let input = r#"macro mymacro
    a b c =>
        main = -> 1
%mymacro a b c"#;

    let expected = r#"macro mymacro
    a b c =>
        main = -> 1
main = -> 1"#;

    let config = test_config();

    let input_program = parse_string(input, &config).unwrap();
    let context = MacroExpansionContext::new(&config);
    let expanded = expand_macros_with_context(input_program, &context).unwrap();

    let expected_program = parse_string(expected, &config).unwrap();

    assert_program_semantically_eq(&expanded, &expected_program);
}

#[test]
fn macro_expansion_preserves_language_item_annotations() {
    let input = r#"macro make_try
    =>
        lang try
        trait Carrier
            lang output
            type Value
            lang branch
            split : I64
%make_try"#;
    let config = test_config();
    let context = MacroExpansionContext::new(&config);

    let expanded = expand_macros_with_context(parse_string(input, &config).unwrap(), &context)
        .expect("language item declaration generated by a macro should expand");
    let TopLevel::TraitDecl(decl) = &expanded.module.top_levels[1] else {
        panic!("expected the generated trait declaration");
    };

    assert_eq!(
        decl.language_items.root.as_ref().unwrap().role,
        LanguageItemRole::Try
    );
    assert_eq!(decl.language_items.members.len(), 2);
    assert_eq!(decl.language_items.members[0].member_name, "Value");
    assert_eq!(decl.language_items.members[1].member_name, "split");
}

#[test]
fn simple_macro_expand_fail() {
    let input = r#"macro mymacro
    a b c =>
        main = -> 1
%mymacro a c b"#;

    let config = test_config();

    let input_program = parse_string(input, &config).unwrap();
    let context = MacroExpansionContext::new(&config);
    let expanded = expand_macros_with_context(input_program, &context);

    assert!(expanded.is_err());
}

#[test]
fn argument_matching() {
    let input = r#"macro mymacro
    $a:ident $b:ident $c:ident =>
        $a = $b -> $c
%mymacro x y z "#;

    let expected = r#"macro mymacro
    $a:ident $b:ident $c:ident =>
        $a = $b -> $c
x = y -> z"#;

    let config = test_config();

    let input_program = parse_string(input, &config).unwrap();

    let context = MacroExpansionContext::new(&config);
    let expanded = expand_macros_with_context(input_program, &context).unwrap();

    let expected_program = parse_string(expected, &config).unwrap();

    assert_program_semantically_eq(&expanded, &expected_program);
}

#[test]
fn argument_repetition() {
    let input = r#"macro mymacro
    $a:ident $($b:ident)* $c:ident =>
        $a = $($b,)* -> $c
%mymacro a b c d e"#;

    let expected = r#"macro mymacro
    $a:ident $($b:ident)* $c:ident =>
        $a = $($b,)* -> $c
a = b, c, d, -> e"#;

    let config = test_config();

    let input_program = parse_string(input, &config).unwrap();
    let context = MacroExpansionContext::new(&config);
    let expanded = expand_macros_with_context(input_program, &context).unwrap();

    let expected_program = parse_string(expected, &config).unwrap();

    assert_program_semantically_eq(&expanded, &expected_program);
}

#[test]
fn multi_entries_macro() {
    let input = r#"macro mymacro
    $a:ident $b:ident $c:ident =>
        $a = $b -> $c
    $a:ident =>
        $a = -> 1
%mymacro x y z
%mymacro x"#;

    let expected = r#"macro mymacro
    $a:ident $b:ident $c:ident =>
        $a = $b -> $c
    $a:ident =>
        $a = -> 1
x = y -> z
x = -> 1"#;

    let config = test_config();

    let input_program = parse_string(input, &config).unwrap();
    let context = MacroExpansionContext::new(&config);
    let expanded = expand_macros_with_context(input_program, &context).unwrap();

    let expected_program = parse_string(expected, &config).unwrap();

    assert_program_semantically_eq(&expanded, &expected_program);
}

#[test]
fn no_repetition() {
    let input = r#"macro mymacro
    $a:ident $($b:ident)* $c:ident =>
        $a = $($b,)* -> $c
%mymacro a c"#;
    let expected = r#"macro mymacro
    $a:ident $($b:ident)* $c:ident =>
        $a = $($b,)* -> $c
a = -> c"#;

    let config = test_config();

    let input_program = parse_string(input, &config).unwrap();
    let context = MacroExpansionContext::new(&config);
    let expanded = expand_macros_with_context(input_program, &context).unwrap();

    let expected_program = parse_string(expected, &config).unwrap();

    assert_program_semantically_eq(&expanded, &expected_program);
}

#[test]
fn empty_repetition_matches_less_args() {
    let input = r#"macro mymacro
    $name:ident $($args:ident)* =>
        $name = $($args,)* -> 1
%mymacro a"#;

    let expected = r#"macro mymacro
    $name:ident $($args:ident)* =>
        $name = $($args,)* -> 1
a = -> 1"#;

    let config = test_config();

    let input_program = parse_string(input, &config).unwrap();
    let context = MacroExpansionContext::new(&config);
    let expanded = expand_macros_with_context(input_program, &context).unwrap();

    let expected_program = parse_string(expected, &config).unwrap();

    assert_program_semantically_eq(&expanded, &expected_program);
}

#[test]
fn parse_macro_expr() {
    let input = r#"macro mymacro
    $a:expr =>
        main = -> $a
%mymacro 1 + 2"#;

    let expected = r#"macro mymacro
    $a:expr =>
        main = -> $a
main = -> 1 + 2"#;

    let config = test_config();

    let input_program = parse_string(input, &config).unwrap();

    let context = MacroExpansionContext::new(&config);
    let expanded = expand_macros_with_context(input_program, &context).unwrap();

    let expected_program = parse_string(expected, &config).unwrap();

    assert_program_semantically_eq(&expanded, &expected_program);
}
