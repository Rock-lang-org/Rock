use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_function_decl_multiline() {
    let input = r#"myfn = a, b, c ->
    statement
    3 + 3
"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, function_decl) = function_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(function_decl.name.name, "myfn");
    assert_eq!(function_decl.lambda.parameters.len(), 3);
    assert_eq!(function_decl.lambda.body.statements.len(), 2);
    assert_eq!(rest.len(), 0);
}
