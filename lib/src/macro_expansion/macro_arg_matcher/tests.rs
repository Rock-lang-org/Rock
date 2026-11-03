use super::*;

use std::path::PathBuf;

use crate::lexer::{Token, TokenType};
use crate::macro_expansion::declarative::{CaptureKind, MatcherFragment};

fn test_config() -> crate::Config {
    crate::Config {
        entry_file: PathBuf::from("/test.rk"),
        output_dir: PathBuf::new(),
        debug_print: Vec::new(),
        meta_files: Vec::new(),
        extern_artifacts: Vec::new(),
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: false,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    }
}

#[test]
fn macro_arg_matcher_accepts_compiled_matcher_fragments() {
    let config = test_config();
    let context = MacroExpansionContext::new(&config);
    let args = vec![Token::from(TokenType::Ident("main".to_string()))];
    let matcher = vec![MatcherFragment::Capture {
        name: "name".to_string(),
        kind: CaptureKind::Ident,
    }];

    let correspondance = MacroArgMatcher::new(&args, matcher, Span::test(), Span::test(), &context)
        .run()
        .unwrap();

    assert!(correspondance.get("name", 0).is_some());
}
