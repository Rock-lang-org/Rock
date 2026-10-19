use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_macro_invoc() {
    let input = "%mymacro\n    a\n    b";
    let config = Config::default();

    let program = parse_string(input, &config).unwrap();
    let TopLevel::MacroInvoc(macro_invoc) = &program.module.top_levels[0] else { panic!("expected invocation") };

    assert_eq!(macro_invoc.path_name(), "mymacro");
    assert_eq!(macro_invoc.args.runs().iter().map(|run| run.text()).collect::<String>(), "    a\n    b");
    assert_eq!(program.module.top_levels.len(), 1);
}
