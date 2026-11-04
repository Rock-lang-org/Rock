use crate::ast::*;
use crate::parser::items::tests::common::lex_test_toplevel;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_multiple_imports() {
    let input = "> std::fs::File\n> std::io::Write\n";
    let tokens = lex_test_toplevel(input);
    let config = Config::default();

    // Parse first import
    let (rest, top_level1) = top_level.process(ParseCtx::from(&tokens, &config)).unwrap();
    match top_level1 {
        TopLevel::Import(_) => {
            // Test passes for first import
        }
        _ => panic!("Expected first import, got: {:?}", top_level1),
    }

    // Parse second import
    let (rest, top_level2) = top_level.process(rest).unwrap();
    match top_level2 {
        TopLevel::Import(_) => {
            // Test passes for second import
        }
        _ => panic!("Expected second import, got: {:?}", top_level2),
    }

    assert_eq!(rest.len(), 0);
}
