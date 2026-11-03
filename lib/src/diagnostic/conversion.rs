use super::*;
use crate::lexer::LexerError;
use crate::parser::ParseError;

impl Diagnostic {
    /// Convert a resolution error while preserving its owning diagnostic family.
    pub fn from_resolve_error(error: &crate::lower::ResolveError) -> Self {
        let origin_valid = matches!(
            (error.kind(), error.span()),
            (crate::lower::ResolveErrorKind::Source, Some(_))
                | (crate::lower::ResolveErrorKind::NonSource, None)
        );
        if !origin_valid {
            return Self::for_internal("resolution error has inconsistent source origin")
                .with_code(DiagnosticCode::Internal);
        }
        let message = error.message.clone();
        let mut diagnostic = error
            .span()
            .map(|span| Self::from_source_parts(message.clone(), message.clone(), span, error.code))
            .unwrap_or_else(|| Self::internal(message));
        diagnostic.code = Some(error.code);
        if matches!(&diagnostic.location, DiagnosticLocation::Source(_)) {
            diagnostic.secondary = error.labels.clone();
        } else {
            diagnostic
                .notes
                .extend(error.labels.iter().map(|label| label.message.clone()));
        }
        diagnostic
    }
}

impl From<ParseError> for Diagnostic {
    fn from(err: ParseError) -> Self {
        match err {
            ParseError::UnexpectedToken(_expected_desc, got_token) => {
                let got_display = if got_token.token_type.to_string().is_empty() {
                    format!("end of file")
                } else {
                    format!("'{}'", got_token.token_type.to_string())
                };

                Diagnostic::from_source_parts(
                    format!("Unexpected token: {}", got_display),
                    format!("Unexpected {}", got_display),
                    got_token.span,
                    DiagnosticCode::Parser,
                )
            }
            /* ParseError::UnexpectedKeyword(token, expected) => Diagnostic {
                message: format!("Unexpected keyword: {:?}", token.token_type,),
                labels: vec![(
                    format!("Expected one of {:?}", expected),
                    token.span.clone(),
                )],
                span: token.span,
                kind: DiagnosticType::Error,
            }, */
            ParseError::Lexer(LexerError::UnknownToken(c, span)) => Diagnostic::from_source_parts(
                format!("Lexer: Unknown token: {:?}", c),
                "Unknown token".to_string(),
                span,
                DiagnosticCode::Lexer,
            ),
            ParseError::MacroNoCorrespondance {
                macro_name,
                invoc_name,
                invoc_arg,
            } => {
                let mut diagnostic = Diagnostic::from_source_parts(
                    "Macro: Nothing expected this token".to_string(),
                    "For this macro".to_string(),
                    macro_name,
                    DiagnosticCode::Macro,
                );
                diagnostic
                    .secondary
                    .push(DiagnosticLabel::new("In this macro invocation", invoc_name));
                if let Some(invoc_arg) = invoc_arg {
                    diagnostic
                        .secondary
                        .push(DiagnosticLabel::new("With this token", invoc_arg));
                }
                diagnostic
            }
            /* ParseError::InvalidPrecedence(precedence, token) => Diagnostic {
                message: format!("Invalid precedence: {:?}", precedence),
                labels: vec![(
                    format!("Precedence must be between 0 and 9"),
                    token.span.clone(),
                )],
                span: token.span,
                kind: DiagnosticType::Error,
            },
            ParseError::IndentMismatch(got, expected, span) => Diagnostic {
                message: format!("Indent mismatch: got {}, expected {}", got, expected),
                labels: vec![
                    (format!("Expected indent level: {}", expected), span.clone()),
                    (format!("Got indent level: {}", got), span.clone()),
                ],
                kind: DiagnosticType::Error,
            }, */
            ParseError::UnexpectedEOF(span) => Diagnostic::from_source_parts(
                "Unexpected end of file".to_string(),
                "Expected more input".to_string(),
                span,
                DiagnosticCode::Parser,
            ),
            /* ParseError::LeftoverTokens(tokens) => Diagnostic {
                message: format!("Leftover tokens"),
                labels: vec![(format!("Expected end of file"), tokens[0].span.clone())],
                span: tokens[0].span.clone(),
                kind: DiagnosticType::Error,
            }, */
            ParseError::UnknownFile(file) => Diagnostic::at_location_with_code(
                format!("Unknown file: {:?}", file),
                DiagnosticLocation::File(PathBuf::from(file)),
                DiagnosticCode::File,
            ),
            ParseError::UnexpectedIndent(level, span) => Diagnostic::from_source_parts(
                format!("Unexpected indent level: {level}"),
                "Unexpected indentation".to_string(),
                span,
                DiagnosticCode::Parser,
            ),
            ParseError::ExpectedOneOrMore(span) => Diagnostic::from_source_parts(
                "Expected one or more".to_string(),
                "Expected at least one item".to_string(),
                span,
                DiagnosticCode::Parser,
            ),
            ParseError::Fail => Diagnostic::internal("Parser control-flow error escaped: Fail"),
            ParseError::ShortCircuit => {
                Diagnostic::internal("Parser control-flow error escaped: ShortCircuit")
            }
            ParseError::AssertFailed => {
                Diagnostic::internal("Parser control-flow error escaped: AssertFailed")
            }
            ParseError::HardError(msg, span) => {
                Diagnostic::from_source_parts(msg.clone(), msg, span, DiagnosticCode::Parser)
            }
            ParseError::WithContext { context, error } => {
                let mut context_chain = vec![context];
                let mut current = error;
                while let ParseError::WithContext {
                    context: inner_context,
                    error: inner_error,
                } = current.as_ref()
                {
                    context_chain.push(inner_context.clone());
                    current = inner_error.clone();
                }

                let inner_error = *current;
                let mut diagnostic = Diagnostic::from(inner_error);
                diagnostic.message = format!(
                    "{}\n\nParsing context (innermost first):\n{}",
                    diagnostic.message,
                    context_chain
                        .iter()
                        .enumerate()
                        .map(|(index, context)| format!("  {}. {}", index + 1, context))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                diagnostic.notes.extend(context_chain);
                diagnostic
            }
        }
    }
}

impl From<crate::lower::ResolveError> for Diagnostic {
    fn from(err: crate::lower::ResolveError) -> Self {
        Diagnostic::from_resolve_error(&err)
    }
}

/* // Old
impl From<crate::parser::ParseError> for Diagnostics {
    fn from(err: crate::parser::ParseError) -> Self {
        let mut diagnostics = Diagnostics::default();

        diagnostics.push(Diagnostic::from(err));

        diagnostics
    }
}

impl From<crate::parser::ParseError> for Diagnostic {
    fn from(err: crate::parser::ParseError) -> Self {
        match err {
            crate::parser::ParseError::UnexpectedToken(got, expected) => Diagnostic {
                message: format!("Unexpected token: {:?}", got),
                labels: vec![(format!("Expected {:?}", expected), got.span.clone())],
                span: got.span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::UnexpectedKeyword(token, expected) => Diagnostic {
                message: format!("Unexpected keyword: {:?}", token.token_type,),
                labels: vec![(
                    format!("Expected one of {:?}", expected),
                    token.span.clone(),
                )],
                span: token.span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::Lexer(LexerError::UnknownToken(c, span)) => Diagnostic {
                message: format!("Lexer: Unknown token: {:?}", c),
                labels: vec![],
                span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::MacroNoCorrespondance {
                macro_name,
                invoc_name,
                invoc_arg,
            } => {
                let mut labels = vec![
                    (format!("For this macro"), macro_name.clone()),
                    (format!("In this macro invocation"), invoc_name.clone()),
                ];

                if let Some(invoc_arg) = invoc_arg {
                    labels.push((format!("With this token"), invoc_arg.clone()));
                }

                Diagnostic {
                    message: format!("Macro: Nothing expected this token"),
                    labels,
                    span: macro_name.clone(),
                    kind: DiagnosticType::Error,
                }
            }
            crate::parser::ParseError::InvalidPrecedence(precedence, token) => Diagnostic {
                message: format!("Invalid precedence: {:?}", precedence),
                labels: vec![(
                    format!("Precedence must be between 0 and 9"),
                    token.span.clone(),
                )],
                span: token.span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::IndentMismatch(got, expected, span) => Diagnostic {
                message: format!("Indent mismatch: got {}, expected {}", got, expected),
                labels: vec![
                    (format!("Expected indent level: {}", expected), span.clone()),
                    (format!("Got indent level: {}", got), span.clone()),
                ],
                span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::UnexpectedEof(_token) => Diagnostic {
                message: format!("Unexpected end of file"),
                labels: vec![],
                span: Span::test(),
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::LeftoverTokens(tokens) => Diagnostic {
                message: format!("Leftover tokens"),
                labels: vec![(format!("Expected end of file"), tokens[0].span.clone())],
                span: tokens[0].span.clone(),
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::UnknownFile(file) => Diagnostic {
                message: format!("Unknown file: {:?}", file),
                labels: vec![],
                span: Span::test(),
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::InternalError(message) => Diagnostic {
                message: format!("Internal error: {:?}", message),
                labels: vec![],
                span: Span::test(),
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::ShortCircuit => Diagnostic {
                message: format!("Short circuit, should never be printed !"),
                labels: vec![],
                span: Span::test(),
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::ExpectedType(span) => Diagnostic {
                message: format!("Expected a type"),
                labels: vec![(format!("But got"), span.clone())],
                span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::InvalidLHS(span) => Diagnostic {
                message: format!("Malformed assignment"),
                labels: vec![(format!("Invalid left-hand side"), span.clone())],
                span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::InvalidVariant(enum_name, variant) => Diagnostic {
                message: format!("Invalid variant"),
                labels: vec![
                    (format!("In this enum"), enum_name.clone()),
                    (format!("Expected a variant"), variant.clone()),
                ],
                span: enum_name,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::InvalidType(span) => Diagnostic {
                message: format!("Invalid type"),
                labels: vec![(format!("Expected a type"), span.clone())],
                span,
                kind: DiagnosticType::Error,
            },
        }
    }
} */
