use crate::ast::Module;
use crate::diagnostic::Diagnostics;
use crate::{parser, Config};

pub fn parse_generated_source(
    stream: &crate::lexer::lossless::LosslessStream,
    config: &Config,
) -> Result<Module, Diagnostics> {
    parser::parse_lossless_source(stream, config).map_err(Diagnostics::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::macro_expansion::token_tree::source;
    use crate::parser::ParseError;
    use crate::Config;

    fn test_config() -> Config {
        Config {
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
    fn generated_parser_parses_top_level_tokens_with_explicit_config() {
        let stream = source("main = -> 0");
        let config = test_config();

        let module = parse_generated_source(&stream, &config).unwrap();

        assert_eq!(module.top_levels.len(), 1);
    }

    #[test]
    fn generated_parser_reports_incomplete_macro_head() {
        let stream = source("macro make\n    $name");
        let config = test_config();

        let parse_error = parser::parse_lossless_source(&stream, &config)
            .expect_err("incomplete macro head should fail before diagnostics conversion");
        match parse_error {
            ParseError::HardError(message, _) => {
                assert!(message.contains("Incomplete macro head"));
            }
            other => panic!("expected incomplete macro head error, got {other:?}"),
        }

        let diagnostics = parse_generated_source(&stream, &config).unwrap_err();
        assert!(diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Incomplete macro head")));
    }
}
