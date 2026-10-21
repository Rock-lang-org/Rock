use std::time::Duration;

use crate::{
    ast::{MacroDecl, MacroInvoc, Module, Program, TopLevel},
    diagnostic::{Diagnostic, Diagnostics},
    lexer::Span,
    parser::ParseError,
};

use self::{
    correspondances::Correspondance,
    declarative::DeclarativeMacro,
    macro_arg_matcher::MacroArgMatcher,
    proc_macro::{ProcMacroKind, ProcMacroRequest},
    registry::ProcMacroRegistryEntry,
};

mod context;
mod hygiene;
#[cfg(test)]
mod proc_hygiene_tests;
#[cfg(test)]
mod diagnostic_tests;
pub(crate) mod binding_environment;
mod correspondances;
pub mod declarative;
pub mod generated_parser;
mod macro_arg_matcher;
mod expression;
pub mod proc_macro;
pub mod registry;
pub mod host;
pub mod source_map;
pub mod token_tree;

#[cfg(test)]
mod storage_tests;

#[cfg(test)]
mod matcher_contract_tests;

#[cfg(test)]
mod expression_tests;

#[cfg(test)]
mod resolution_tests;

#[cfg(test)]
mod source_proc_tests;

mod derives;
#[cfg(test)]
mod derive_tests;

#[cfg(test)]
mod traversal_tests;

#[cfg(test)]
mod scheduling_tests;

#[cfg(test)]
mod budget_tests;

mod budget;
pub use budget::ExpansionLimits;

pub use context::MacroExpansionContext;
pub(crate) use derives::reject_pending as reject_pending_derives;
pub use source_map::{
    ExpansionId, GeneratedSourceId, MacroExpansionRecord, MacroSourceMap, OutputProjection, TokenOrigin, TokenProvenance,
};
pub use crate::lexer::groups::Delimiter;
pub use crate::lexer::lossless::{ConstructionContext, ConstructionKind, ScanLimits};
pub use token_tree::TokenStream;

pub fn expand_macros_with_context(
    mut program: Program,
    context: &MacroExpansionContext<'_>,
) -> Result<Program, Diagnostics> {
    let mut modules = context.source_modules.modules().cloned().collect::<Vec<_>>();
    expand_forest(&mut program.module, &mut modules, context, &mut |_, _| Ok(false))?;
    Ok(program)
}

/// Expand the loaded forest and publish the same trees to collection's graph.
/// Source discovery is interleaved through the ordinary source loader only.
pub fn expand_macros_in_graph(
    graph: &mut crate::source_loader::ModuleGraph,
    source_db: &mut crate::source_loader::SourceDatabase,
    context: &MacroExpansionContext<'_>,
) -> Result<Program, Diagnostics> {
    let mut program = Program { module: graph.root_module().clone() };
    let mut modules = graph.modules().cloned().collect::<Vec<_>>();
    expand_forest(&mut program.module, &mut modules, context, &mut |root, modules| {
        publish_forest(graph, root, modules, context);
        let count = graph.modules().count();
        source_db.load_module_declarations_with_budget(graph, root, context.config.current_crate_name.as_deref(), context.config,
            &mut |path, qualified, bytes| context.charge_source(bytes)
                .map_err(|message| context.source_budget_diagnostic(path, qualified, message)))
            .map_err(crate::source_load_errors_to_diagnostics)?;
        *modules = graph.modules().cloned().collect();
        Ok(graph.modules().count() != count)
    })?;
    publish_forest(graph, &program.module, &modules, context);
    Ok(program)
}

fn is_forest_root(root: &Module, module: &crate::source_loader::LoadedModule, context: &MacroExpansionContext<'_>) -> bool {
    context.config.current_crate_name.as_deref() == Some(module.qualified_name.as_str())
        && root.filepath.as_ref() == Some(&module.path)
}

fn publish_forest(graph: &mut crate::source_loader::ModuleGraph, root: &Module, modules: &[crate::source_loader::LoadedModule], context: &MacroExpansionContext<'_>) {
    *graph.root_module_mut() = root.clone();
    let expanded: std::collections::HashMap<_, _> = modules.iter().map(|module| (&module.qualified_name, &module.module)).collect();
    for target in graph.modules_mut() {
        if is_forest_root(root, target, context) { target.module = root.clone(); continue; }
        if let Some(module) = expanded.get(&target.qualified_name) { target.module = (*module).clone(); }
    }
}

fn expand_forest(
    root: &mut Module,
    modules: &mut Vec<crate::source_loader::LoadedModule>,
    context: &MacroExpansionContext<'_>,
    discover: &mut impl FnMut(&Module, &mut Vec<crate::source_loader::LoadedModule>) -> Result<bool, Diagnostics>,
) -> Result<(), Diagnostics> {
    loop {
        let discovered = discover(root, modules)?;
        // A named source graph also indexes the root as a loaded module. It is
        // one AST identity, not a second executable copy of the entry program.
        modules.retain(|module| !is_forest_root(root, module, context));
        modules.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));
        validate_macro_definitions(root, context.config)?;
        for loaded in modules.iter() { validate_macro_definitions(&loaded.module, context.config)?; }
        let sources = crate::source_loader::SourceModuleSet::from_modules(modules.iter().cloned());
        let registry = registry::MacroRegistry::build_for_expansion(root, &sources, context)?;
        if first_forest_invocation(root, modules).is_none() {
            let registry = registry::MacroRegistry::build_with_modules(root, &sources, context)?;
            derives::validate(root, &[], &registry, context)?;
            for loaded in modules.iter() {
                let mut path: Vec<_> = loaded.qualified_name.split("::").map(String::from).collect();
                if path.first().map(String::as_str) == context.config.current_crate_name.as_deref() { path.remove(0); }
                derives::validate(&loaded.module, &registry.loaded_scope(&path), &registry, context)?;
            }
            let mut visitor = expression::ExpansionVisitor::new(&registry, context, expression::Phase::Expressions);
            visitor.module(root)?;
            for loaded in modules.iter_mut() {
                visitor.set_scope(&loaded.qualified_name);
                visitor.module(&mut loaded.module)?;
            }
            validate_no_invocations(root)?;
            for loaded in modules.iter() { validate_no_invocations(&loaded.module)?; }
            return Ok(());
        }
        let mut visitor = expression::ExpansionVisitor::new(&registry, context, expression::Phase::Items);
        visitor.module(root)?;
        for loaded in modules.iter_mut() {
            visitor.set_scope(&loaded.qualified_name);
            visitor.module(&mut loaded.module)?;
        }
        if visitor.executed == 0 && !discovered {
            // Reject only at a stable frontier: another item's output may add
            // the definition, import, or export needed by this invocation.
            registry::MacroRegistry::build_with_modules(root, &sources, context)?;
            if let Some(diagnostics) = visitor.deferred { return Err(diagnostics); }
            if let Some(invocation) = first_forest_invocation(root, modules) {
                let diagnostics = proc_macro_error(format!("macro resolution made no progress: unresolved macro path '{}'", invocation.path_name()), invocation.span.clone());
                return Err(invocation.parent.map(|parent| with_expansion_trace(diagnostics.clone(), context, parent)).unwrap_or(diagnostics));
            }
        }
    }
}

fn validate_no_invocations(module: &Module) -> Result<(), Diagnostics> {
    struct Pending(Diagnostics);
    impl<'ast> crate::ast::visit::Visitor<'ast> for Pending {
        fn visit_macro_invoc(&mut self, invocation: &'ast MacroInvoc) {
            self.0.merge(proc_macro_error("unexpanded macro at expansion/collection boundary", invocation.span.clone()));
        }
    }
    let mut pending = Pending(Diagnostics::default());
    crate::ast::visit::walk_module(&mut pending, module);
    if pending.0.0.is_empty() { Ok(()) } else { Err(pending.0) }
}

fn validate_macro_definitions(module: &Module, config: &crate::Config) -> Result<(), Diagnostics> {
    struct Definitions { diagnostics: Diagnostics, proc_target: bool }
    impl<'ast> crate::ast::visit::Visitor<'ast> for Definitions {
        fn visit_macro_decl(&mut self, declaration: &'ast MacroDecl) {
            if let Err(message) = DeclarativeMacro::from_ast(declaration).validate() {
                self.diagnostics.push(Diagnostic::new(message, declaration.name.span.clone())
                    .with_code(crate::diagnostic::DiagnosticCode::Macro));
            }
        }
        fn visit_proc_macro_decl(&mut self, declaration: &'ast crate::ast::ProcMacroDecl) {
            if !self.proc_target {
                self.diagnostics.push(Diagnostic::new(
                    "source procedural macro declarations require a [lib] with proc_macro = true".into(),
                    declaration.function.name.span.clone(),
                ).with_code(crate::diagnostic::DiagnosticCode::Macro));
            }
            if !matches!(declaration.kind, ProcMacroKind::FunctionLike | ProcMacroKind::DeriveLike) {
                self.diagnostics.push(Diagnostic::new("unsupported source procedural macro kind".into(), declaration.function.name.span.clone())
                    .with_code(crate::diagnostic::DiagnosticCode::Macro));
            }
        }
    }
    let mut definitions = Definitions { diagnostics: Diagnostics::default(),
        proc_target: config.target_role == crate::crate_system::TargetRole::ProcMacro };
    crate::ast::visit::walk_module(&mut definitions, module);
    if definitions.diagnostics.0.is_empty() { Ok(()) } else { Err(definitions.diagnostics) }
}

fn expand_top_level(
    macro_decl: &MacroDecl,
    invocation: &MacroInvoc,
    context: &MacroExpansionContext<'_>,
) -> Result<Vec<TopLevel>, Diagnostics> {
    match expand_declarative(macro_decl, invocation, context)? {
        generated_parser::GeneratedSyntax::Items(module) => Ok(module.top_levels),
        generated_parser::GeneratedSyntax::Expression(_) => Err(proc_macro_error("expected generated items", invocation.span.clone())),
    }
}

fn expand_declarative(
    macro_decl: &MacroDecl,
    invocation: &MacroInvoc,
    context: &MacroExpansionContext<'_>,
) -> Result<generated_parser::GeneratedSyntax, Diagnostics> {
    let invoc_span = invocation.span.clone();
    let parent = invocation.parent;
    let expansion_id = context.begin_expansion(MacroExpansionRecord {
        invocation: invocation.id,
        macro_name: macro_decl.name.name.clone(),
        invocation_span: invoc_span.clone(),
        definition_span: Some(macro_decl.name.span.clone()),
        parent,
    })?;
    context.charge_matching(invocation.args.len_bytes(), &invoc_span)
        .map_err(|diagnostics| with_expansion_trace(diagnostics, context, expansion_id))?;
    let (args, ranges) = macro_arg_matcher::rock_arguments(&invocation.args, context.config, invocation.header_indent)
        .map_err(|diagnostics| with_expansion_trace(diagnostics, context, expansion_id))?;
    let declarative_macro = DeclarativeMacro::from_ast(macro_decl);
    let mut diagnostics = Diagnostics::default();

    for arm in &declarative_macro.arms {
        let mut macro_matcher = MacroArgMatcher::new(
            &args,
            &invocation.args,
            invocation.header_indent,
            &ranges,
            arm.matcher.fragments.clone(),
            macro_decl.name.span.clone(),
            invoc_span.clone(),
            context,
        );
        let correspondances = match macro_matcher.run() {
            Ok(correspondances) => correspondances,
            Err(macro_arg_matcher::MatchError::NoMatch(diags)) => {
                diagnostics.merge(with_expansion_trace(diags, context, expansion_id));
                continue;
            }
            Err(macro_arg_matcher::MatchError::Invalid(diags)) => {
                return Err(with_expansion_trace(diags, context, expansion_id));
            }
        };

        let generated_source = context
            .generated_source_id(expansion_id)
            .unwrap_or(GeneratedSourceId(0));
        let captures = capture_set_from_correspondance(&correspondances, expansion_id, &invoc_span);
        let body = match arm
            .template
            .expand_with_origins(&captures, expansion_id, generated_source, |original| context.template_context(macro_decl.id, expansion_id, original))
        {
            Ok(expanded) => expanded,
            Err(message) => {
                diagnostics.push(ParseError::HardError(message, invoc_span.clone()));
                return Err(with_expansion_trace(diagnostics, context, expansion_id));
            }
        };

        return parse_generated_result(
            body,
            context,
            invocation.output,
            Some(expansion_id),
        );
    }

    Err(diagnostics)
}

fn expand_proc_macro_top_level(
    entry: &ProcMacroRegistryEntry,
    invocation: &MacroInvoc,
    context: &MacroExpansionContext<'_>,
) -> Result<Vec<TopLevel>, Diagnostics> {
    match expand_procedural(entry, invocation, context)? {
        generated_parser::GeneratedSyntax::Items(module) => Ok(module.top_levels),
        generated_parser::GeneratedSyntax::Expression(_) => Err(proc_macro_error("expected generated items", invocation.span.clone())),
    }
}

fn expand_procedural(
    entry: &ProcMacroRegistryEntry,
    invocation: &MacroInvoc,
    context: &MacroExpansionContext<'_>,
) -> Result<generated_parser::GeneratedSyntax, Diagnostics> {
    let invoc_span = invocation.span.clone();
    let args = &invocation.args;
    let parent = invocation.parent;
    let expansion_id = context.begin_expansion(MacroExpansionRecord {
        invocation: invocation.id,
        macro_name: entry.export.name.clone(),
        invocation_span: invoc_span.clone(),
        definition_span: None,
        parent,
    })?;

    if context.config.no_proc_execution {
        return Err(with_expansion_trace(proc_macro_error(
            format!("Proc macro '{}': procedural macro execution is disabled (--no-proc-execution)", entry.export.identity),
            invoc_span,
        ), context, expansion_id));
    }
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

    let input = proc_macro::encode_stream(args).map_err(|err| {
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
        invocation_id: invocation.id,
        parent: invocation.parent,
        output_category: invocation.output,
        kind: entry.export.kind.clone(),
        header_indent: invocation.header_indent,
        block_base: invocation.block.as_ref().map(|block| block.body_indent),
        input,
        definition_dependencies: entry.artifact.definition_dependencies.clone(),
        views: proc_macro::ProcMacroViews::for_invocation(invocation).map_err(|error|
            with_expansion_trace(proc_macro_error(format!("Failed to prepare proc macro literal views: {error}"), invoc_span.clone()), context, expansion_id))?,
    };

    let response =
        proc_macro::run_proc_macro_process(&entry.artifact, &request, Duration::from_secs(10), context.config.no_proc_execution, context.cancellation.as_deref())
            .map_err(|err| {
                with_expansion_trace(
                    proc_macro_error(err, invoc_span.clone()),
                    context,
                    expansion_id,
                )
            })?;

    let generated_source = context
        .generated_source_id(expansion_id)
        .unwrap_or(GeneratedSourceId(0));
    let stream = proc_macro::materialize(&request, &response, invocation, context, expansion_id, generated_source)
        .map_err(|diagnostics| with_expansion_trace(diagnostics, context, expansion_id))?;
    let projected = context.project_output(&stream, expansion_id);
    let diagnostics = response.diagnostics_at(&request, invoc_span, Some(&projected));
    if diagnostics.0.iter().any(|diagnostic| diagnostic.severity == crate::diagnostic::DiagnosticSeverity::Error) {
        return Err(with_expansion_trace(diagnostics, context, expansion_id));
    }
    context.reports.borrow_mut().merge(diagnostics);
    parse_generated_result(stream, context, invocation.output, Some(expansion_id))
}

#[cfg(test)]
fn parse_generated_top_levels(
    stream: TokenStream,
    context: &MacroExpansionContext<'_>,
    expansion_id: Option<ExpansionId>,
) -> Result<Vec<TopLevel>, Diagnostics> {
    let anchor = stream.start_span();
    match parse_generated_result(stream, context, crate::ast::MacroOutputCategory::Items, expansion_id)? {
        generated_parser::GeneratedSyntax::Items(module) => Ok(module.top_levels),
        generated_parser::GeneratedSyntax::Expression(_) => Err(proc_macro_error("expected generated items", anchor)),
    }
}

fn parse_generated_result(
    stream: TokenStream,
    context: &MacroExpansionContext<'_>,
    category: crate::ast::MacroOutputCategory,
    expansion_id: Option<ExpansionId>,
) -> Result<generated_parser::GeneratedSyntax, Diagnostics> {
    context.charge_output(&stream).map_err(|diagnostics| expansion_id
        .map(|id| with_expansion_trace(diagnostics.clone(), context, id)).unwrap_or(diagnostics))?;
    let stream = expansion_id.map(|id| context.project_output(&stream, id)).unwrap_or(stream);
    generated_parser::parse_generated_source(&stream, context.config, category, expansion_id).map_err(
        |mut diags| {
            if let Some(expansion_id) = expansion_id {
                context.map_generated_diagnostics(&mut diags, expansion_id);
                with_expansion_trace(diags, context, expansion_id)
            } else {
                diags
            }
        },
    )
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

fn first_macro_invocation(module: &Module) -> Option<&MacroInvoc> {
    module
        .top_levels
        .iter()
        .find_map(|top_level| match top_level {
            TopLevel::MacroInvoc(invocation) => Some(invocation),
            TopLevel::Module(child) => first_macro_invocation(&child.0),
            _ => None,
        })
}

fn first_forest_invocation<'a>(root: &'a Module, modules: &'a [crate::source_loader::LoadedModule]) -> Option<&'a MacroInvoc> {
    first_macro_invocation(root).or_else(|| modules.iter().find_map(|loaded| first_macro_invocation(&loaded.module)))
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
                tokens.captured(expansion_id, invoc_span),
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
                        tokens.captured(expansion_id, invoc_span)
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
    use super::token_tree::source;

    pub(super) fn first_invocation(program: &Program) -> MacroInvoc {
        struct Calls(Vec<MacroInvoc>);
        impl<'a> crate::ast::visit::Visitor<'a> for Calls {
            fn visit_macro_invoc(&mut self, invocation: &'a MacroInvoc) { self.0.push(invocation.clone()); }
        }
        let mut calls = Calls(Vec::new());
        crate::ast::visit::walk_module(&mut calls, &program.module);
        calls.0.into_iter().next().unwrap()
    }

    pub(super) fn constructed_response(invocation: &MacroInvoc, text: &str) -> proc_macro::ProcMacroResponse {
        let stream = TokenStream::source_fragment(text.into(), crate::lexer::lossless::ConstructionContext {
            invocation: invocation.id, kind: crate::lexer::lossless::ConstructionKind::CallSite,
        }, token_tree::MACRO_LIMITS).unwrap();
        proc_macro::ProcMacroResponse::expanded(proc_macro::encode_stream(&stream).unwrap())
    }

    pub(super) fn proc_macro_artifact_with_response(
        macro_name: &str,
        response: proc_macro::ProcMacroResponse,
    ) -> (proc_macro::ProcMacroArtifact, PathBuf) {
        let response = proc_macro::encode_response(&response).unwrap();
        static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let fixture = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "rock-proc-macro-test-{}-{}-{}",
            std::process::id(),
            nonce,
            fixture,
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        let host = temp_dir.join("proc_macro_host");
        fs::write(host.with_extension("response"), response).unwrap();
        fs::write(&host, "#!/bin/sh\ncat >\"$0.request\"\ncat \"$0.response\"\n").unwrap();
        let mut permissions = fs::metadata(&host).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&host, permissions).unwrap();

        let artifact = proc_macro::ProcMacroArtifact {
            artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
            crate_identity: "test_macros".to_string(),
            protocol_version: proc_macro::PROC_MACRO_PROTOCOL_VERSION,
            host_triple: "x86_64-unknown-linux-gnu".to_string(),
            executable_identity: proc_macro::ProcMacroFileIdentity::read(&host).unwrap(), executable: host,
            compiler_version: env!("CARGO_PKG_VERSION").into(), required_files: Vec::new(),
            capabilities: vec![proc_macro::ProcMacroCapability::Stdio],
            definition_dependencies: Vec::new(),
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
            target_role: crate::crate_system::TargetRole::Binary,
            opt_level: 0,
            overflow_checks: None,
            emit_llvm: false,
            no_link: false,
            emit_object: None,
            no_prelude: false,
            no_std: false,
            sysroot: None,
            no_proc_execution: false,
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
        let input_program = parse_string("%test_macros::make_main", &config).unwrap();
        let response = constructed_response(&first_invocation(&input_program), "main = -> 0\n");
        let (artifact, temp_dir) = proc_macro_artifact_with_response("make_main", response);
        let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);

        let expanded = expand_macros_with_context(input_program, &context).unwrap();

        assert!(!expanded.module.has_macro_invoc());
        assert!(expanded.module.top_level_from_ident("main").is_some());

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn generated_parse_errors_point_back_to_macro_invocation_source() {
        let config = test_config();
        let input_program = parse_string("%test_macros::broken", &config).unwrap();
        let response = constructed_response(&first_invocation(&input_program), "main = ->");
        let (artifact, temp_dir) = proc_macro_artifact_with_response("broken", response);
        let invocation_span = match &input_program.module.top_levels[0] {
            TopLevel::MacroInvoc(invocation) => invocation.span.clone(),
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
            proc_macro::ProcMacroResponse::failure(vec!["boom".to_string()]),
        );
        let input_program = parse_string("%test_macros::fail_macro", &config).unwrap();
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
            proc_macro::ProcMacroResponse::failure(vec!["boom".to_string()]),
        );
        let input_program = parse_string("%test_macros::fail_macro", &config).unwrap();
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
                TopLevel::MacroInvoc(invocation) if invocation.path_name() == "outer" => {
                    Some(invocation.span.clone())
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
    fn proc_macro_expansion_supplies_terminal_layout_for_generated_source() {
        let config = test_config();
        let input_program = parse_string("%test_macros::make_main", &config).unwrap();
        let (artifact, temp_dir) = proc_macro_artifact_with_response(
            "make_main",
            constructed_response(&first_invocation(&input_program), "main = -> 0"),
        );
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
            source("first"),
        );
        correspondance.insert_direct(
            "name".to_string(),
            source("second"),
        );
        let captures =
            capture_set_from_correspondance(&correspondance, ExpansionId(1), &Span::test());
        let template = declarative::MacroTemplate {
            input: source(""),
            fragments: vec![declarative::TemplateFragment::Capture {
                name: "name".to_string(),
            }],
        };

        let expanded = template.expand(&captures).unwrap();
        let (tokens, _) = macro_arg_matcher::rock_arguments(&expanded, &crate::Config::default(), 0).unwrap();
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].token_type, crate::lexer::TokenType::Ident("first".into()));
        assert_eq!(tokens[1].token_type, crate::lexer::TokenType::Ident("second".into()));
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
main = -> (1 + 2)"#;

        let config = test_config();

        let input_program = parse_string(input, &config).unwrap();

        let context = MacroExpansionContext::new(&config);
        let expanded = expand_macros_with_context(input_program, &context).unwrap();

        let expected_program = parse_string(expected, &config).unwrap();

        assert_program_semantically_eq(&expanded, &expected_program);
    }
}
