use crate::ast::*;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_type_alias() {
    let input = "type MyInt = Int32\n";
    let tokens = lex_test_toplevel(input);
    let config = Config::default();

    let (rest, top_level) = top_level.process(ParseCtx::from(&tokens, &config)).unwrap();

    match top_level {
        TopLevel::NewType(name, _, _ty) => {
            assert_eq!(name.name, "MyInt");
            // Test passes if we get a type alias
        }
        _ => panic!("Expected type alias, got: {:?}", top_level),
    }

    assert_eq!(rest.len(), 0);
}
