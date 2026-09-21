use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn multiline_if() {
    let input = "if true\n    1";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, _if_) = parse_if.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(rest.len(), 0);
}

#[test]
fn multiline_if_with_negative_branch() {
    let tokens = lex_test("if value <= 0\n    -1\nelse\n    1");
    let config = Config::default();
    let (rest, parsed) = parse_if.process(ParseCtx::from(&tokens, &config)).unwrap();
    assert!(rest.is_empty());
    assert!(parsed.else_.is_some());
    assert!(matches!(
        &parsed.then.statements[0],
        Statement::Expression(Expression::UnaryExpr(UnaryExpr::UnaryExpr(op, _)))
            if op.value == "-"
    ));
}
