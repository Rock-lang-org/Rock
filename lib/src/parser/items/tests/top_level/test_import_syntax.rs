use crate::ast::*;
use crate::parser::items::tests::common::lex_test_toplevel;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_import_syntax() {
    let input = "> std::collections::HashMap\n";
    let tokens = lex_test_toplevel(input);
    let config = Config::default();

    let (rest, top_level) = top_level.process(ParseCtx::from(&tokens, &config)).unwrap();

    match top_level {
        TopLevel::Import(path) => {
            // Test passes if we get an import
            match path {
                Path::Type(type_path) => {
                    assert!(type_path.path.len() > 0);
                }
                Path::Ident(ident_path) => {
                    assert!(ident_path.path.len() > 0);
                }
            }
        }
        _ => panic!("Expected import, got: {:?}", top_level),
    }

    assert_eq!(rest.len(), 0);
}
