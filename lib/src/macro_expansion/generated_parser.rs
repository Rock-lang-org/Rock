use crate::ast::Module;
use crate::diagnostic::Diagnostics;
use crate::macro_expansion::TokenStream;
use crate::{parser, Config};

pub fn parse_generated_source(
    stream: &crate::lexer::lossless::LosslessStream,
    config: &Config,
) -> Result<Module, Diagnostics> {
    parser::parse_lossless_source(stream, config).map_err(Diagnostics::from)
}

pub fn parse_generated_module(
    stream: &TokenStream,
    config: &Config,
) -> Result<Module, Diagnostics> {
    parser::parse_module_tokens(&stream.to_tokens(), config).map_err(Diagnostics::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::lexer::{Token, TokenType};
    use crate::macro_expansion::TokenStream;
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
        let tokens = vec![
            Token::from(TokenType::Indent(0)),
            Token::from(TokenType::Ident("main".to_string())),
            Token::from(TokenType::Equal),
            Token::from(TokenType::Arrow),
            Token::from(TokenType::Number("0".to_string())),
            Token::from(TokenType::Eol),
            Token::from(TokenType::Indent(0)),
            Token::from(TokenType::Eol),
            Token::from(TokenType::Eof),
        ];
        let stream = TokenStream::from_tokens(tokens);
        let config = test_config();

        let module = parse_generated_module(&stream, &config).unwrap();

        assert_eq!(module.top_levels.len(), 1);
    }

    #[test]
    fn generated_parser_reports_incomplete_macro_head() {
        let tokens = vec![
            Token::from(TokenType::Indent(0)),
            Token::from(TokenType::Keyword("macro".to_string())),
            Token::from(TokenType::Ident("make".to_string())),
            Token::from(TokenType::Eol),
            Token::from(TokenType::Indent(4)),
            Token::from(TokenType::MacroVar("name".to_string())),
            Token::from(TokenType::Eof),
        ];
        let stream = TokenStream::from_tokens(tokens);
        let config = test_config();

        let parse_error = parser::parse_module_tokens(&stream.to_tokens(), &config)
            .expect_err("incomplete macro head should fail before diagnostics conversion");
        match parse_error {
            ParseError::HardError(message, _) => {
                assert!(message.contains("Incomplete macro head"));
            }
            other => panic!("expected incomplete macro head error, got {other:?}"),
        }

        let diagnostics = parse_generated_module(&stream, &config).unwrap_err();
        assert!(diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Incomplete macro head")));
    }
}
