use super::*;

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
