use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_function_shorthand() {
    let input = "myfn = (+ 2)\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, function_decl) = function_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(function_decl.name.name, "myfn");
    assert_eq!(function_decl.lambda.parameters.len(), 1);
    assert_eq!(function_decl.lambda.body.statements.len(), 1);
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parse_unit_receiver_shorthand() {
    for body in [".println!", ".abs!.println!", ".method 1, 2", ".field"] {
        for (marker, arrow_kind) in [("", LambdaArrowKind::Normal), ("!", LambdaArrowKind::Unit)] {
            let input = format!("myfn = ({marker}{body})\n");
            let tokens = lex_test(&input);
            let config = Config::default();
            let (rest, declaration) = function_decl
                .process(ParseCtx::from(&tokens, &config))
                .unwrap();

            assert!(rest.is_empty(), "{input}");
            assert_eq!(declaration.lambda.arrow_kind, arrow_kind, "{input}");
            assert_eq!(declaration.lambda.parameters.len(), 1, "{input}");
            assert_eq!(declaration.lambda.body.statements.len(), 1, "{input}");
        }
    }
}

#[test]
fn test_parse_unit_receiver_shorthand_rejects_missing_member() {
    let tokens = lex_test("myfn = (!.)\n");
    let config = Config::default();
    assert!(function_decl
        .process(ParseCtx::from(&tokens, &config))
        .is_err());
}
