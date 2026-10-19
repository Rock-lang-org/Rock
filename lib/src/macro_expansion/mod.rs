use std::{collections::HashMap, time::Duration};

use crate::{
    ast::{MacroDecl, Module, Program, TopLevel},
    diagnostic::{Diagnostic, Diagnostics},
    lexer::{Span, Token, TokenType},
    parser::ParseError,
};

use self::{
    correspondances::Correspondance,
    declarative::DeclarativeMacro,
    macro_arg_matcher::MacroArgMatcher,
    proc_macro::{ProcMacroKind, ProcMacroRequest, ProcMacroResponse},
    registry::ProcMacroRegistryEntry,
};

mod context;
mod correspondances;
pub mod declarative;
pub mod generated_parser;
mod macro_arg_matcher;
pub mod proc_macro;
pub mod registry;
pub mod source_map;
pub mod token_tree;

pub use context::MacroExpansionContext;
pub use source_map::{
    ExpansionId, GeneratedSourceId, MacroExpansionRecord, MacroSourceMap, TokenOrigin,
};
pub use crate::lexer::groups::Delimiter;
pub use token_tree::{TokenStream, TokenTree};

pub fn expand_macros_with_context(
    mut program: Program,
    context: &MacroExpansionContext<'_>,
) -> Result<Program, Diagnostics> {
    let mut depth = 0;
    let mut module = program.module;

    while module.has_macro_invoc() {
        let mut decls = HashMap::new();
        let mut proc_macros = HashMap::new();
        let mut registry = registry::MacroRegistry::from_module(&module);

        for artifact in &context.proc_macro_artifacts {
            registry.add_proc_macro_artifact(artifact.clone());
        }

        for (i, top_level) in module.top_levels.iter().enumerate() {
            if let TopLevel::MacroInvoc(invocation) = &top_level {
                if let Some(decl) = registry.declaration(&invocation.name.name) {
                    decls.insert(
                        i,
                        (
                            decl.clone(),
                            invocation.args.clone(),
                            invocation.name.span.clone(),
                        ),
                    );
                    continue;
                }

                if let Some(entry) = registry.proc_macro(&invocation.name.name) {
                    proc_macros.insert(
                        i,
                        (
                            entry.clone(),
                            invocation.args.clone(),
                            invocation.name.span.clone(),
                        ),
                    );
                }
            }
        }
        module = expand_macros_once(module, &decls, &proc_macros, context)?;

        depth += 1;

        if depth > context.max_depth {
            let mut diagnostics = Diagnostics::default();
            let span = first_macro_invocation_span(&module)
                .expect("remaining macro invocation must retain its source span");
            let trace_expansion = context.peek_parent_expansion_for_invocation(&span);
            let mut diagnostic = context.depth_exceeded_diagnostic(span);
            if let Some(expansion_id) = trace_expansion {
                diagnostic = diagnostic.with_labels(context.trace_labels(expansion_id));
            }
            diagnostics.push(diagnostic);
            return Err(diagnostics);
        }
    }

    program.module = module;

    Ok(program)
}

fn expand_macros_once(
    mut module: Module,
    decls: &HashMap<usize, (MacroDecl, Vec<Token>, Span)>,
    proc_macros: &HashMap<usize, (ProcMacroRegistryEntry, Vec<Token>, Span)>,
    context: &MacroExpansionContext<'_>,
) -> Result<Module, Diagnostics> {
    let results = module
        .top_levels
        .into_iter()
        .enumerate()
        .map(|(i, top_level)| match top_level {
            TopLevel::MacroInvoc(_) => {
                if let Some((decl, args, invoc_span)) = decls.get(&i) {
                    return expand_top_level(decl, args.clone(), invoc_span.clone(), context);
                }

                let Some((entry, args, invoc_span)) = proc_macros.get(&i) else {
                    return Ok(vec![top_level]);
                };

                expand_proc_macro_top_level(entry, args, invoc_span.clone(), context)
            }
            _ => Ok(vec![top_level]),
        })
        .collect::<Vec<_>>();

    module.top_levels = vec![];

    for result in results {
        module.top_levels.extend(result?);
    }

    Ok(module)
}

fn expand_top_level(
    macro_decl: &MacroDecl,
    args: Vec<Token>,
    invoc_span: Span,
    context: &MacroExpansionContext<'_>,
) -> Result<Vec<TopLevel>, Diagnostics> {
    let parent = context.take_parent_expansion_for_invocation(&invoc_span);
    let expansion_id = context.record_expansion(MacroExpansionRecord {
        macro_name: macro_decl.name.name.clone(),
        invocation_span: invoc_span.clone(),
        definition_span: Some(macro_decl.name.span.clone()),
        parent,
    });
    let declarative_macro = DeclarativeMacro::from_ast(macro_decl);
    let mut top_levels = vec![];
    let mut diagnostics = Diagnostics::default();

    for arm in &declarative_macro.arms {
        let mut macro_matcher = MacroArgMatcher::new(
            &args,
            arm.matcher.fragments.clone(),
            macro_decl.name.span.clone(),
            invoc_span.clone(),
            context,
        );
        let correspondances = match macro_matcher.run() {
            Ok(correspondances) => correspondances,
            Err(diags) => {
                diagnostics.merge(with_expansion_trace(diags, context, expansion_id));
                continue;
            }
        };

        let generated_source = context
            .generated_source_id(expansion_id)
            .unwrap_or(GeneratedSourceId(0));
        let captures = capture_set_from_correspondance(&correspondances, expansion_id, &invoc_span);
        let body = match arm
            .template
            .expand_with_origins(&captures, expansion_id, generated_source)
        {
            Ok(expanded) => expanded,
            Err(message) => {
                diagnostics.push(ParseError::HardError(message, invoc_span.clone()));
                return Err(with_expansion_trace(diagnostics, context, expansion_id));
            }
        };

        top_levels.extend(parse_generated_top_levels(
            body,
            context,
            Some(expansion_id),
        )?);

        break;
    }

    if top_levels.is_empty() {
        return Err(diagnostics);
    }

    Ok(top_levels)
}

fn expand_proc_macro_top_level(
    entry: &ProcMacroRegistryEntry,
    args: &[Token],
    invoc_span: Span,
    context: &MacroExpansionContext<'_>,
) -> Result<Vec<TopLevel>, Diagnostics> {
    let parent = context.take_parent_expansion_for_invocation(&invoc_span);
    let expansion_id = context.record_expansion(MacroExpansionRecord {
        macro_name: entry.export.name.clone(),
        invocation_span: invoc_span.clone(),
        definition_span: None,
        parent,
    });

    if entry.export.kind != ProcMacroKind::FunctionLike {
        return Err(with_expansion_trace(
            proc_macro_error(
                format!("Proc macro '{}' is not function-like", entry.export.name),
                invoc_span,
            ),
            context,
            expansion_id,
        ));
    }

    let input = proc_macro::encode_tokens(args).map_err(|err| {
        with_expansion_trace(
            proc_macro_error(
                format!("Failed to encode proc macro input: {}", err),
                invoc_span.clone(),
            ),
            context,
            expansion_id,
        )
    })?;
    let request = ProcMacroRequest {
        protocol_version: proc_macro::PROC_MACRO_PROTOCOL_VERSION,
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        macro_name: entry.export.name.clone(),
        macro_identity: entry.export.identity.clone(),
        source_module: None,
        expansion_id: Some(expansion_id.0),
        input,
    };

    let response =
        proc_macro::run_proc_macro_process(&entry.artifact, &request, Duration::from_secs(10))
            .map_err(|err| {
                with_expansion_trace(
                    proc_macro_error(err, invoc_span.clone()),
                    context,
                    expansion_id,
                )
            })?;

    let output = match response {
        ProcMacroResponse::Expand { output } => output,
        diagnostics @ ProcMacroResponse::Diagnostics { .. } => {
            return Err(with_expansion_trace(
                diagnostics.into_diagnostics_at(&entry.export.name, invoc_span),
                context,
                expansion_id,
            ));
        }
    };

    let tokens = proc_macro::decode_tokens(&output).map_err(|err| {
        with_expansion_trace(
            proc_macro_error(
                format!("Failed to decode proc macro output: {}", err),
                invoc_span,
            ),
            context,
            expansion_id,
        )
    })?;
    let generated_source = context
        .generated_source_id(expansion_id)
        .unwrap_or(GeneratedSourceId(0));
    let stream = TokenStream::generated_from_tokens(tokens, expansion_id, generated_source);
    parse_generated_top_levels(stream, context, Some(expansion_id))
}

fn parse_generated_top_levels(
    mut stream: TokenStream,
    context: &MacroExpansionContext<'_>,
    expansion_id: Option<ExpansionId>,
) -> Result<Vec<TopLevel>, Diagnostics> {
    let mut tokens = stream.to_tokens();
    if matches!(
        tokens.last().map(|token| &token.token_type),
        Some(TokenType::Eof)
    ) {
        tokens.pop();
        stream.trees.pop();
    }

    let last_content_token = tokens
        .iter()
        .rev()
        .find(|token| !matches!(token.token_type, TokenType::Eol | TokenType::Indent(_)));

    if matches!(
        last_content_token.map(|token| &token.token_type),
        Some(TokenType::MacroInvoc(_))
    ) {
        append_generated_parse_token(&mut stream, TokenType::Eol, context, expansion_id, &tokens);
        append_generated_parse_token(
            &mut stream,
            TokenType::Indent(0),
            context,
            expansion_id,
            &tokens,
        );
        append_generated_parse_token(&mut stream, TokenType::Eol, context, expansion_id, &tokens);
    }

    append_generated_parse_token(&mut stream, TokenType::Eof, context, expansion_id, &tokens);

    let module = generated_parser::parse_generated_module(&stream, context.config).map_err(
        |mut diags| {
            if let Some(expansion_id) = expansion_id {
                context.map_generated_diagnostics(&mut diags, expansion_id);
                with_expansion_trace(diags, context, expansion_id)
            } else {
                diags
            }
        },
    )?;

    if let Some(expansion_id) = expansion_id {
        for top_level in &module.top_levels {
            if let TopLevel::MacroInvoc(invocation) = top_level {
                context.record_generated_invocation_parent(&invocation.name.span, expansion_id);
            }
        }
    }

    Ok(module.top_levels)
}

fn append_generated_parse_token(
    stream: &mut TokenStream,
    token_type: TokenType,
    context: &MacroExpansionContext<'_>,
    expansion_id: Option<ExpansionId>,
    existing_tokens: &[Token],
) {
    let expansion_id = expansion_id.expect("generated parser tokens require an expansion identity");
    let span = existing_tokens
        .last()
        .filter(|token| !token.span.file_path.as_os_str().is_empty())
        .map(|token| Span::new(token.span.file_path.clone(), token.span.end, token.span.end))
        .unwrap_or_else(|| {
            Span::new(
                std::path::PathBuf::from(format!("<macro-expansion:{}>", expansion_id.0)),
                0,
                0,
            )
        });
    let token = Token {
        token_type,
        span: span.clone(),
    };
    let origin = context
        .generated_source_id(expansion_id)
        .map(|source| (expansion_id, source))
        .map_or_else(
            || TokenOrigin::Source(span.clone()),
            |(expansion, generated_source)| TokenOrigin::Generated {
                expansion,
                generated_source,
                definition_span: span.clone(),
            },
        );

    stream.trees.push(TokenTree::Leaf { token, origin });
}

fn with_expansion_trace(
    mut diagnostics: Diagnostics,
    context: &MacroExpansionContext<'_>,
    expansion_id: ExpansionId,
) -> Diagnostics {
    let labels = context.trace_labels(expansion_id);

    diagnostics.0 = diagnostics
        .0
        .into_iter()
        .map(|mut diagnostic| {
            diagnostic.code = Some(crate::diagnostic::DiagnosticCode::Macro);
            if matches!(
                &diagnostic.location,
                crate::diagnostic::DiagnosticLocation::Source(_)
            ) {
                diagnostic.with_labels(labels.clone())
            } else {
                diagnostic
            }
        })
        .collect();

    diagnostics
}

fn proc_macro_error(message: impl Into<String>, span: Span) -> Diagnostics {
    let mut diagnostics = Diagnostics::default();
    diagnostics.push(
        Diagnostic::new(message.into(), span).with_code(crate::diagnostic::DiagnosticCode::Macro),
    );
    diagnostics
}

fn first_macro_invocation_span(module: &Module) -> Option<Span> {
    module
        .top_levels
        .iter()
        .find_map(|top_level| match top_level {
            TopLevel::MacroInvoc(invocation) => Some(invocation.name.span.clone()),
            _ => None,
        })
}

fn capture_set_from_correspondance(
    correspondances: &Correspondance,
    expansion_id: ExpansionId,
    invoc_span: &Span,
) -> declarative::CaptureSet {
    let mut captures = declarative::CaptureSet::new();
    for name in correspondances.entries.keys() {
        let Some(streams) = correspondances.get(name, 0) else {
            continue;
        };
        for tokens in streams {
            captures.insert_direct(
                name.clone(),
                TokenStream::captured_from_tokens(tokens, expansion_id, invoc_span.clone()),
            );
        }
    }
    for nested in &correspondances.nested_corresp {
        for (name, streams) in &nested.entries {
            captures.insert_repeated(
                name.clone(),
                streams
                    .iter()
                    .map(|tokens| {
                        TokenStream::captured_from_tokens(
                            tokens.clone(),
                            expansion_id,
                            invoc_span.clone(),
                        )
                    })
                    .collect(),
            );
        }
    }
    captures
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::{language_items::LanguageItemRole, parser::parse_string, Config};

    use super::*;

    fn proc_macro_artifact_with_response(
        macro_name: &str,
        response: proc_macro::ProcMacroResponse,
    ) -> (proc_macro::ProcMacroArtifact, PathBuf) {
        let response = proc_macro::encode_response(&response).unwrap();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "rock-proc-macro-test-{}-{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        let host = temp_dir.join("proc_macro_host");
        fs::write(host.with_extension("response"), response).unwrap();
        fs::write(&host, "#!/bin/sh\ncat >/dev/null\ncat \"$0.response\"\n").unwrap();
        let mut permissions = fs::metadata(&host).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&host, permissions).unwrap();

        let artifact = proc_macro::ProcMacroArtifact {
            artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
            crate_identity: "test_macros".to_string(),
            protocol_version: proc_macro::PROC_MACRO_PROTOCOL_VERSION,
            host_triple: "x86_64-unknown-linux-gnu".to_string(),
            executable: host,
            capabilities: vec![proc_macro::ProcMacroCapability::Stdio],
            exports: vec![proc_macro::ProcMacroExport {
                name: macro_name.to_string(),
                identity: format!("test_macros::{macro_name}"),
                kind: proc_macro::ProcMacroKind::FunctionLike,
                input_shape: proc_macro::ProcMacroInputShape::TokenStream,
            }],
        };

        (artifact, temp_dir)
    }

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

    fn assert_program_semantically_eq(actual: &Program, expected: &Program) {
        assert_eq!(
            crate::fmt::format(crate::fmt::FormatInput::program(actual)),
            crate::fmt::format(crate::fmt::FormatInput::program(expected)),
        );
    }

    #[test]
    fn macro_expansion_uses_context_depth_limit_instead_of_panicking() {
        let input = r#"macro repeat
    =>
        %repeat
%repeat"#;
        let config = test_config();
        let input_program = parse_string(input, &config).unwrap();
        let context = MacroExpansionContext::new(&config).with_max_depth(2);

        let result = expand_macros_with_context(input_program, &context);

        let diagnostics = result.expect_err("recursive macro should produce diagnostics");
        let diagnostic = diagnostics
            .0
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("Macro expansion depth exceeded")
            })
            .expect("expected depth diagnostic");
        assert!(diagnostic
            .primary
            .iter()
            .chain(diagnostic.secondary.iter())
            .any(|label| label.message.contains("expanded from macro 'repeat'")));
    }

    #[test]
    fn macro_expansion_depth_trace_matches_first_remaining_invocation() {
        let input = r#"macro repeat_a
    =>
        %repeat_a
macro repeat_b
    =>
        %repeat_b
%repeat_a
%repeat_b"#;
        let config = test_config();
        let input_program = parse_string(input, &config).unwrap();
        let context = MacroExpansionContext::new(&config).with_max_depth(1);

        let result = expand_macros_with_context(input_program, &context);

        let diagnostics = result.expect_err("recursive macros should produce diagnostics");
        let diagnostic = diagnostics
            .0
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("Macro expansion depth exceeded")
            })
            .expect("expected depth diagnostic");

        assert!(diagnostic
            .primary
            .iter()
            .chain(diagnostic.secondary.iter())
            .any(|label| label.message.contains("expanded from macro 'repeat_a'")));
        assert!(!diagnostic
            .primary
            .iter()
            .chain(diagnostic.secondary.iter())
            .any(|label| label.message.contains("expanded from macro 'repeat_b'")));
    }

    #[test]
    fn macro_expansion_depth_trace_ignores_unrelated_latest_expansion() {
        let input = r#"macro known
    =>
        main = -> 0
%unknown
%known"#;
        let config = test_config();
        let input_program = parse_string(input, &config).unwrap();
        let context = MacroExpansionContext::new(&config).with_max_depth(0);

        let result = expand_macros_with_context(input_program, &context);

        let diagnostics = result.expect_err("unknown macro should remain past depth limit");
        let diagnostic = diagnostics
            .0
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("Macro expansion depth exceeded")
            })
            .expect("expected depth diagnostic");

        assert!(!diagnostic
            .primary
            .iter()
            .chain(diagnostic.secondary.iter())
            .any(|label| label.message.contains("expanded from macro 'known'")));
    }

    #[test]
    fn macro_arg_matcher_uses_context_config_for_expr_parsing() {
        let input = r#"macro id
    $value:expr =>
        main = -> $value
%id 1 + 2"#;
        let config = Config {
            debug_print: vec![crate::DebugPrint::Tokens],
            ..test_config()
        };
        let input_program = parse_string(input, &config).unwrap();
        let context = MacroExpansionContext::new(&config);

        let expanded = expand_macros_with_context(input_program, &context).unwrap();

        assert!(!expanded.module.has_macro_invoc());
    }

    #[test]
    fn function_like_proc_macro_expands_top_level_invocation() {
        let config = test_config();
        let generated_tokens = vec![
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
        let response = proc_macro::ProcMacroResponse::Expand {
            output: proc_macro::encode_tokens(&generated_tokens).unwrap(),
        };
        let (artifact, temp_dir) = proc_macro_artifact_with_response("make_main", response);
        let input_program = parse_string("%make_main", &config).unwrap();
        let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);

        let expanded = expand_macros_with_context(input_program, &context).unwrap();

        assert!(!expanded.module.has_macro_invoc());
        assert!(expanded.module.top_level_from_ident("main").is_some());

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn generated_parse_errors_point_back_to_macro_invocation_source() {
        let config = test_config();
        let generated_tokens = vec![
            Token::from(TokenType::Indent(0)),
            Token::from(TokenType::Ident("main".to_string())),
            Token::from(TokenType::Equal),
            Token::from(TokenType::Arrow),
            Token::from(TokenType::Eof),
        ];
        let response = proc_macro::ProcMacroResponse::Expand {
            output: proc_macro::encode_tokens(&generated_tokens).unwrap(),
        };
        let (artifact, temp_dir) = proc_macro_artifact_with_response("broken", response);
        let input_program = parse_string("%broken", &config).unwrap();
        let invocation_span = match &input_program.module.top_levels[0] {
            TopLevel::MacroInvoc(invocation) => invocation.name.span.clone(),
            other => panic!("expected macro invocation, got {other:?}"),
        };
        let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);

        let diagnostics = expand_macros_with_context(input_program, &context)
            .expect_err("invalid generated source should fail parsing");
        let diagnostic = diagnostics
            .0
            .first()
            .expect("expected generated parse diagnostic");
        assert_eq!(
            diagnostic.location,
            crate::diagnostic::DiagnosticLocation::Source(invocation_span)
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn proc_macro_diagnostics_use_invocation_span() {
        let config = test_config();
        let (artifact, temp_dir) = proc_macro_artifact_with_response(
            "fail_macro",
            proc_macro::ProcMacroResponse::Diagnostics {
                messages: vec!["boom".to_string()],
            },
        );
        let input_program = parse_string("%fail_macro", &config).unwrap();
        let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);

        let diagnostics = expand_macros_with_context(input_program, &context)
            .expect_err("proc macro diagnostics should fail expansion");

        let diagnostic = diagnostics.0.first().expect("expected diagnostic");
        assert!(diagnostic.message.contains("fail_macro"));
        assert_eq!(
            diagnostic.code,
            Some(crate::diagnostic::DiagnosticCode::Macro)
        );
        let crate::diagnostic::DiagnosticLocation::Source(span) = &diagnostic.location else {
            panic!("macro diagnostic should have a source location");
        };
        assert!(span.end > span.start);

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn proc_macro_diagnostics_include_expansion_trace_label() {
        let config = test_config();
        let (artifact, temp_dir) = proc_macro_artifact_with_response(
            "fail_macro",
            proc_macro::ProcMacroResponse::Diagnostics {
                messages: vec!["boom".to_string()],
            },
        );
        let input_program = parse_string("%fail_macro", &config).unwrap();
        let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);

        let diagnostics = expand_macros_with_context(input_program, &context)
            .expect_err("proc macro diagnostics should fail expansion");

        let diagnostic = diagnostics.0.first().expect("expected diagnostic");
        assert!(diagnostic
            .primary
            .iter()
            .chain(diagnostic.secondary.iter())
            .any(|label| label.message.contains("expanded from macro 'fail_macro'")));

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn nested_macro_diagnostics_include_parent_expansion_trace_label() {
        let input = r#"macro inner
    =>
        main =
macro outer
    =>
        %inner
%outer"#;
        let config = test_config();
        let input_program = parse_string(input, &config).unwrap();
        let context = MacroExpansionContext::new(&config);

        let diagnostics = expand_macros_with_context(input_program, &context)
            .expect_err("inner macro output should fail parsing");

        let labels = diagnostics
            .0
            .iter()
            .flat_map(|diagnostic| diagnostic.primary.iter().chain(diagnostic.secondary.iter()))
            .map(|label| label.message.as_str())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label.contains("expanded from macro 'inner'")),
            "labels: {labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("expanded from macro 'outer'")),
            "labels: {labels:?}"
        );
    }

    #[test]
    fn repeated_generated_macro_invocations_keep_parent_trace_order() {
        let input = r#"macro inner
    =>
        main =
macro outer
    =>
        %inner
%outer
%outer"#;
        let config = test_config();
        let input_program = parse_string(input, &config).unwrap();
        let outer_spans = input_program
            .module
            .top_levels
            .iter()
            .filter_map(|top_level| match top_level {
                TopLevel::MacroInvoc(invocation) if invocation.name.name == "outer" => {
                    Some(invocation.name.span.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(outer_spans.len(), 2);
        let context = MacroExpansionContext::new(&config);

        let diagnostics = expand_macros_with_context(input_program, &context)
            .expect_err("first inner macro output should fail parsing");

        let outer_trace_spans = diagnostics
            .0
            .iter()
            .flat_map(|diagnostic| diagnostic.primary.iter().chain(diagnostic.secondary.iter()))
            .filter_map(|label| {
                label
                    .message
                    .contains("expanded from macro 'outer'")
                    .then_some(label.span.clone())
            })
            .collect::<Vec<_>>();
        assert!(outer_trace_spans
            .iter()
            .any(|span| span.start == outer_spans[0].start && span.end == outer_spans[0].end));
        assert!(!outer_trace_spans
            .iter()
            .any(|span| span.start == outer_spans[1].start && span.end == outer_spans[1].end));
    }

    #[test]
    fn proc_macro_expansion_appends_missing_eof_to_generated_tokens() {
        let config = test_config();
        let generated_tokens = vec![
            Token::from(TokenType::Indent(0)),
            Token::from(TokenType::Ident("main".to_string())),
            Token::from(TokenType::Equal),
            Token::from(TokenType::Arrow),
            Token::from(TokenType::Number("0".to_string())),
            Token::from(TokenType::Eol),
            Token::from(TokenType::Indent(0)),
            Token::from(TokenType::Eol),
        ];
        let (artifact, temp_dir) = proc_macro_artifact_with_response(
            "make_main",
            proc_macro::ProcMacroResponse::Expand {
                output: proc_macro::encode_tokens(&generated_tokens).unwrap(),
            },
        );
        let input_program = parse_string("%make_main", &config).unwrap();
        let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);

        let expanded = expand_macros_with_context(input_program, &context).unwrap();

        assert!(expanded.module.top_level_from_ident("main").is_some());

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn selected_arm_template_errors_do_not_fall_through_to_later_arms() {
        let input = r#"macro mymacro
    $a:ident =>
        $missing = -> 1
    $a:ident =>
        main = -> 0
%mymacro main"#;
        let config = test_config();
        let input_program = parse_string(input, &config).unwrap();
        let context = MacroExpansionContext::new(&config);

        let result = expand_macros_with_context(input_program, &context);

        assert!(result.is_err());
    }

    #[test]
    fn declarative_repetition_expands_each_capture_group() {
        let input = r#"macro defs
    $( $name:ident ) =>
        $( $name = -> 1 )
%defs one two three"#;
        let expected = r#"macro defs
    $( $name:ident ) =>
        $( $name = -> 1 )
one = -> 1
two = -> 1
three = -> 1"#;
        let config = test_config();
        let context = MacroExpansionContext::new(&config);

        let expanded =
            expand_macros_with_context(parse_string(input, &config).unwrap(), &context).unwrap();
        let expected = parse_string(expected, &config).unwrap();

        assert_program_semantically_eq(&expanded, &expected);
    }

    #[test]
    fn declarative_repetition_can_use_direct_outer_capture() {
        let input = r#"macro defs
    $ret:ident $( $name:ident ) =>
        $( $name = -> $ret )
%defs value one two"#;
        let expected = r#"macro defs
    $ret:ident $( $name:ident ) =>
        $( $name = -> $ret )
one = -> value
two = -> value"#;
        let config = test_config();
        let context = MacroExpansionContext::new(&config);

        let expanded =
            expand_macros_with_context(parse_string(input, &config).unwrap(), &context).unwrap();
        let expected = parse_string(expected, &config).unwrap();

        assert_program_semantically_eq(&expanded, &expected);
    }

    #[test]
    fn capture_set_from_correspondance_preserves_repeated_direct_streams() {
        let mut correspondance = Correspondance::new();
        correspondance.insert_direct(
            "name".to_string(),
            vec![Token::from(TokenType::Ident("first".to_string()))],
        );
        correspondance.insert_direct(
            "name".to_string(),
            vec![Token::from(TokenType::Ident("second".to_string()))],
        );
        let captures =
            capture_set_from_correspondance(&correspondance, ExpansionId(1), &Span::test());
        let template = declarative::MacroTemplate {
            fragments: vec![declarative::TemplateFragment::Capture {
                name: "name".to_string(),
            }],
        };

        let expanded = template.expand(&captures).unwrap().to_tokens();

        assert_eq!(expanded.len(), 2);
        assert!(matches!(
            expanded[0].token_type,
            TokenType::Ident(ref name) if name == "first"
        ));
        assert!(matches!(
            expanded[1].token_type,
            TokenType::Ident(ref name) if name == "second"
        ));
    }

    #[test]
    fn simple_macro_expand() {
        let input = r#"macro mymacro
    a b c =>
        main = -> 1
%mymacro a b c"#;

        let expected = r#"macro mymacro
    a b c =>
        main = -> 1
main = -> 1"#;

        let config = test_config();

        let input_program = parse_string(input, &config).unwrap();
        let context = MacroExpansionContext::new(&config);
        let expanded = expand_macros_with_context(input_program, &context).unwrap();

        let expected_program = parse_string(expected, &config).unwrap();

        assert_program_semantically_eq(&expanded, &expected_program);
    }

    #[test]
    fn macro_expansion_preserves_language_item_annotations() {
        let input = r#"macro make_try
    =>
        lang try
        trait Carrier
            lang output
            type Value
            lang branch
            split : I64
%make_try"#;
        let config = test_config();
        let context = MacroExpansionContext::new(&config);

        let expanded = expand_macros_with_context(parse_string(input, &config).unwrap(), &context)
            .expect("language item declaration generated by a macro should expand");
        let TopLevel::TraitDecl(decl) = &expanded.module.top_levels[1] else {
            panic!("expected the generated trait declaration");
        };

        assert_eq!(
            decl.language_items.root.as_ref().unwrap().role,
            LanguageItemRole::Try
        );
        assert_eq!(decl.language_items.members.len(), 2);
        assert_eq!(decl.language_items.members[0].member_name, "Value");
        assert_eq!(decl.language_items.members[1].member_name, "split");
    }

    #[test]
    fn simple_macro_expand_fail() {
        let input = r#"macro mymacro
    a b c =>
        main = -> 1
%mymacro a c b"#;

        let config = test_config();

        let input_program = parse_string(input, &config).unwrap();
        let context = MacroExpansionContext::new(&config);
        let expanded = expand_macros_with_context(input_program, &context);

        assert!(expanded.is_err());
    }

    #[test]
    fn argument_matching() {
        let input = r#"macro mymacro
    $a:ident $b:ident $c:ident =>
        $a = $b -> $c
%mymacro x y z "#;

        let expected = r#"macro mymacro
    $a:ident $b:ident $c:ident =>
        $a = $b -> $c
x = y -> z"#;

        let config = test_config();

        let input_program = parse_string(input, &config).unwrap();

        let context = MacroExpansionContext::new(&config);
        let expanded = expand_macros_with_context(input_program, &context).unwrap();

        let expected_program = parse_string(expected, &config).unwrap();

        assert_program_semantically_eq(&expanded, &expected_program);
    }

    #[test]
    fn argument_repetition() {
        let input = r#"macro mymacro
    $a:ident $($b:ident)* $c:ident =>
        $a = $($b,)* -> $c
%mymacro a b c d e"#;

        let expected = r#"macro mymacro
    $a:ident $($b:ident)* $c:ident =>
        $a = $($b,)* -> $c
a = b, c, d, -> e"#;

        let config = test_config();

        let input_program = parse_string(input, &config).unwrap();
        let context = MacroExpansionContext::new(&config);
        let expanded = expand_macros_with_context(input_program, &context).unwrap();

        let expected_program = parse_string(expected, &config).unwrap();

        assert_program_semantically_eq(&expanded, &expected_program);
    }

    #[test]
    fn multi_entries_macro() {
        let input = r#"macro mymacro
    $a:ident $b:ident $c:ident =>
        $a = $b -> $c
    $a:ident =>
        $a = -> 1
%mymacro x y z
%mymacro x"#;

        let expected = r#"macro mymacro
    $a:ident $b:ident $c:ident =>
        $a = $b -> $c
    $a:ident =>
        $a = -> 1
x = y -> z
x = -> 1"#;

        let config = test_config();

        let input_program = parse_string(input, &config).unwrap();
        let context = MacroExpansionContext::new(&config);
        let expanded = expand_macros_with_context(input_program, &context).unwrap();

        let expected_program = parse_string(expected, &config).unwrap();

        assert_program_semantically_eq(&expanded, &expected_program);
    }

    #[test]
    fn no_repetition() {
        let input = r#"macro mymacro
    $a:ident $($b:ident)* $c:ident =>
        $a = $($b,)* -> $c
%mymacro a c"#;
        let expected = r#"macro mymacro
    $a:ident $($b:ident)* $c:ident =>
        $a = $($b,)* -> $c
a = -> c"#;

        let config = test_config();

        let input_program = parse_string(input, &config).unwrap();
        let context = MacroExpansionContext::new(&config);
        let expanded = expand_macros_with_context(input_program, &context).unwrap();

        let expected_program = parse_string(expected, &config).unwrap();

        assert_program_semantically_eq(&expanded, &expected_program);
    }

    #[test]
    fn empty_repetition_matches_less_args() {
        let input = r#"macro mymacro
    $name:ident $($args:ident)* =>
        $name = $($args,)* -> 1
%mymacro a"#;

        let expected = r#"macro mymacro
    $name:ident $($args:ident)* =>
        $name = $($args,)* -> 1
a = -> 1"#;

        let config = test_config();

        let input_program = parse_string(input, &config).unwrap();
        let context = MacroExpansionContext::new(&config);
        let expanded = expand_macros_with_context(input_program, &context).unwrap();

        let expected_program = parse_string(expected, &config).unwrap();

        assert_program_semantically_eq(&expanded, &expected_program);
    }

    #[test]
    fn parse_macro_expr() {
        let input = r#"macro mymacro
    $a:expr =>
        main = -> $a
%mymacro 1 + 2"#;

        let expected = r#"macro mymacro
    $a:expr =>
        main = -> $a
main = -> 1 + 2"#;

        let config = test_config();

        let input_program = parse_string(input, &config).unwrap();

        let context = MacroExpansionContext::new(&config);
        let expanded = expand_macros_with_context(input_program, &context).unwrap();

        let expected_program = parse_string(expected, &config).unwrap();

        assert_program_semantically_eq(&expanded, &expected_program);
    }
}
