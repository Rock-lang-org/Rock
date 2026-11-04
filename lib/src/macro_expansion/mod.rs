use std::{collections::HashMap, time::Duration};

use crate::{
    ast::{MacroDecl, Module, ModuleDecl, Program, TopLevel},
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
pub mod exports;
mod expression;
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
pub use token_tree::{Delimiter, TokenStream, TokenTree};

pub fn expand_macros_with_context(
    mut program: Program,
    context: &MacroExpansionContext<'_>,
) -> Result<Program, Diagnostics> {
    let mut depth = 0;
    let mut module = program.module;

    while module.has_macro_invoc() {
        let mut decls = HashMap::new();
        let mut proc_macros = HashMap::new();
        let registry = module_registry(&module, context);

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

    let registry = module_registry(&module, context);
    expression::expand_module_expressions(&mut module, &registry, context)?;

    // Imports of dependency macros are fully consumed by expansion; later passes
    // would otherwise see them as imports of items that do not exist.
    module.top_levels.retain(|top_level| match top_level {
        TopLevel::Import(path) => !context
            .dependency_macros
            .contains_key(&exports::path_segments(path).join("::")),
        _ => true,
    });

    program.module = expand_inline_modules(module, context)?;

    Ok(program)
}

/// Macros visible in `module`, from lowest to highest priority: std prelude
/// macros, dependency macros imported by name with `>`, and the module's own
/// macros.
fn module_registry(
    module: &Module,
    context: &MacroExpansionContext<'_>,
) -> registry::MacroRegistry {
    let mut registry = registry::MacroRegistry::new();

    for (alias, decl) in &context.prelude_macros {
        registry.insert_declaration(alias.clone(), decl.clone());
    }

    for top_level in &module.top_levels {
        if let TopLevel::Import(path) = top_level {
            let segments = exports::path_segments(path);
            if let (Some(decl), Some(name)) = (
                context.dependency_macros.get(&segments.join("::")),
                segments.last(),
            ) {
                registry.insert_declaration(name.clone(), decl.clone());
            }
        }
    }

    registry.add_module_declarations(module);
    for artifact in &context.proc_macro_artifacts {
        registry.add_proc_macro_artifact(artifact.clone());
    }

    registry
}

/// Expands each inline `mod name` block with the macros declared inside it.
fn expand_inline_modules(
    mut module: Module,
    context: &MacroExpansionContext<'_>,
) -> Result<Module, Diagnostics> {
    let top_levels = std::mem::take(&mut module.top_levels);
    module.top_levels = top_levels
        .into_iter()
        .map(|top_level| match top_level {
            TopLevel::Module(ModuleDecl(inner)) => {
                let expanded = expand_macros_with_context(Program { module: inner }, context)?;
                Ok(TopLevel::Module(ModuleDecl(expanded.module)))
            }
            other => Ok(other),
        })
        .collect::<Result<_, Diagnostics>>()?;

    Ok(module)
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
    let (body, expansion_id) =
        expand_declarative_tokens(macro_decl, &args, invoc_span, parent, context)?;

    let top_levels = parse_generated_top_levels(body, context, Some(expansion_id))?;
    if top_levels.is_empty() {
        return Err(Diagnostics::default());
    }

    Ok(top_levels)
}

/// Records one expansion of `macro_decl`, matches `args` against its arms in
/// order, and fills the template of the first arm that matches.
fn expand_declarative_tokens(
    macro_decl: &MacroDecl,
    args: &[Token],
    invoc_span: Span,
    parent: Option<ExpansionId>,
    context: &MacroExpansionContext<'_>,
) -> Result<(TokenStream, ExpansionId), Diagnostics> {
    let expansion_id = context.record_expansion(MacroExpansionRecord {
        macro_name: macro_decl.name.name.clone(),
        invocation_span: invoc_span.clone(),
        definition_span: Some(macro_decl.name.span.clone()),
        parent,
    });
    let declarative_macro = DeclarativeMacro::from_ast(macro_decl);
    let mut diagnostics = Diagnostics::default();

    for arm in &declarative_macro.arms {
        let mut macro_matcher = MacroArgMatcher::new(
            args,
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
        return match arm
            .template
            .expand_with_origins(&captures, expansion_id, generated_source)
        {
            Ok(body) => Ok((body, expansion_id)),
            Err(message) => {
                diagnostics.push(ParseError::HardError(message, invoc_span.clone()));
                Err(with_expansion_trace(diagnostics, context, expansion_id))
            }
        };
    }

    Err(diagnostics)
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
mod tests;
