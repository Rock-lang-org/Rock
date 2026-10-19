use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_macro_decl() {
    let input = "macro mymacro\n    $a:ident, $b:ty =>\n        statement";
    let config = Config::default();

    let program = parse_string(input, &config).unwrap();
    let TopLevel::MacroDecl(macro_decl) = &program.module.top_levels[0] else { panic!("expected macro") };

    assert_eq!(macro_decl.name.name, "mymacro");
    assert_eq!(macro_decl.entries.len(), 1);
    assert_eq!(program.module.top_levels.len(), 1);
}
