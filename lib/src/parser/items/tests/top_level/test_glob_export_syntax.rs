use crate::ast::*;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_glob_export_syntax() {
    let input = "< std::string::*\n";
    let tokens = lex_test_toplevel(input);
    let config = Config::default();

    let (rest, top_level) = top_level.process(ParseCtx::from(&tokens, &config)).unwrap();

    match top_level {
        TopLevel::GlobExport(path) => {
            assert_eq!(crate::collect::path_names(&path), vec!["std", "string"]);
        }
        _ => panic!("Expected glob export, got: {:?}", top_level),
    }

    assert_eq!(rest.len(), 0);
}
