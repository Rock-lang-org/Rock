use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::ast::debug::debug_ast;
use crate::ast::{self, Program};
use crate::crate_system::{load_extern_artifacts, validate_extern_artifact_names, CrateContext};
use crate::diagnostic::{self, Diagnostics};
use crate::{analysis, collect, infer, lower, macro_expansion, products, source_loader};

use super::{module_name_from_entry, Config, DebugPrint};

pub(super) struct FrontendOutput {
    pub(super) analysis: analysis::Analysis,
    pub(super) crate_ctx: CrateContext,
    pub(super) diagnostic_sources: diagnostic::DiagnosticSourceMap,
    pub(super) loaded_prelude_export_ids:
        std::collections::HashMap<String, crate::crate_artifact::ArtifactExport>,
    pub(super) infix_precedence: std::collections::HashMap<String, u8>,
    pub(super) infix_owners: std::collections::HashMap<String, String>,
    pub(super) fingerprint_source_files: Vec<PathBuf>,
    pub(super) source_db: source_loader::SourceDatabase,
    pub(super) effective_current_crate_name: String,
    pub(super) exported_macros: Vec<products::ProductMacroExport>,
    pub(super) prelude_macros: BTreeMap<String, String>,
}

pub(super) fn compile_frontend(config: &Config) -> Result<FrontendOutput, Diagnostics> {
    let effective_current_crate_name = config
        .current_crate_name
        .clone()
        .unwrap_or_else(|| module_name_from_entry(&config.entry_file));

    let mut ctx = CrateContext::new();
    validate_extern_artifact_names(config, &effective_current_crate_name)?;
    if let Err(diagnostic) = load_extern_artifacts(
        &mut ctx,
        &config.extern_artifacts,
        config
            .entry_file
            .parent()
            .unwrap_or(&config.entry_file)
            .to_path_buf(),
    ) {
        let mut diagnostics = Diagnostics::default();
        diagnostics.push(diagnostic);
        return Err(diagnostics);
    }

    let mut source_db = source_loader::SourceDatabase::new();
    source_db.add_source_providers(&config.source_providers);
    let mut source_graph = match source_db.load_entry(config.entry_file.clone(), config) {
        Ok(graph) => graph,
        Err(errors) => return Err(source_loader::source_load_errors_to_diagnostics(errors)),
    };
    let mut diagnostic_sources = diagnostic::DiagnosticSourceMap::from_source_database(&source_db);
    let ast = Program {
        module: source_graph.root_module().clone(),
    };

    if config.has_debug_print(DebugPrint::AstFull) {
        println!("{:#?}", ast);
    }
    if config.has_debug_print(DebugPrint::Ast) {
        debug_ast(&ast);
    }

    let mut macro_context = macro_expansion::MacroExpansionContext::new(config);
    for artifact in ctx.proc_macro_artifacts() {
        macro_context = macro_context.with_proc_macro_artifact(artifact.clone());
    }
    for (key, decl) in ctx.dependency_macros() {
        macro_context = macro_context.with_importable_macro(key, decl);
    }
    // Same-crate macros are importable with `>` from any module. The source
    // graph already holds every module unexpanded, so they are known before
    // expansion starts and import resolution does not depend on load order.
    let (own_macros, _) = crate_macro_exports(
        source_graph.root_module(),
        &source_graph,
        config.current_crate_name.as_deref(),
    );
    for export in own_macros {
        let key = macro_expansion::MacroKey {
            origin: macro_expansion::MacroOrigin::CurrentCrate,
            path: macro_expansion::exports::macro_path_segments(&export.path),
        };
        macro_context = macro_context.with_importable_macro(key, export.decl);
    }
    // Only the std prelude is injected, and only when std was passed explicitly.
    if !config.no_prelude {
        for (alias, decl) in ctx.prelude_macros("std") {
            macro_context = macro_context.with_prelude_macro(alias, decl);
        }
    }
    let ast = macro_expansion::expand_macros_with_context(ast, &macro_context)
        .map_err(|diagnostics| diagnostics.with_sources(&diagnostic_sources))?;

    reject_modules_named_like_dependencies(&ast.module, &ctx)
        .map_err(|diagnostics| diagnostics.with_sources(&diagnostic_sources))?;

    if config.has_debug_print(DebugPrint::Expanded) {
        println!("{:#?}", ast);
    }

    let mut expand_child_module = |module: ast::Module| {
        macro_expansion::expand_macros_with_context(Program { module }, &macro_context)
            .map(|program| program.module)
    };
    if let Err(errors) = source_db.load_module_declarations(
        &mut source_graph,
        &ast.module,
        config.current_crate_name.as_deref(),
        config,
        &mut expand_child_module,
    ) {
        let sources = diagnostic::DiagnosticSourceMap::from_source_database(&source_db);
        return Err(source_loader::source_load_errors_to_diagnostics(errors).with_sources(&sources));
    }
    diagnostic_sources = diagnostic::DiagnosticSourceMap::from_source_database(&source_db);
    let (exported_macros, prelude_macros) = crate_macro_exports(
        &ast.module,
        &source_graph,
        config.current_crate_name.as_deref(),
    );

    let declarations = collect::collect_with_source_graph(
        &ast,
        &source_graph,
        &ctx,
        !config.no_prelude,
        config.current_crate_name.as_deref(),
    )
    .map_err(|errors| Diagnostics::from_resolve_errors(errors).with_sources(&diagnostic_sources))?;
    let loaded_prelude_export_ids = declarations.loaded_prelude_export_ids.clone();
    let infix_precedence = declarations.infix_precedence.clone();
    let infix_owners = declarations.infix_owners.clone();
    let fingerprint_source_files = source_graph.loaded_files().to_vec();
    let partial_hir =
        lower::pipeline::LoweringPipeline::new(&ast, &ctx, config.current_crate_name.as_deref())
            .with_overflow_checks(config.overflow_checks.unwrap_or(config.opt_level == 0))
            .lower_from_declarations(declarations)
            .map_err(|errors| {
                Diagnostics::from_resolve_errors(errors).with_sources(&diagnostic_sources)
            })?;
    let hir = infer::finalize_with_crates(partial_hir, &ctx).map_err(|errors| {
        Diagnostics::from_resolve_errors(errors).with_sources(&diagnostic_sources)
    })?;

    if config.has_debug_print(DebugPrint::Hir) {
        println!("{:#?}", hir.program);
    }

    Ok(FrontendOutput {
        analysis: analysis::Analysis { ast, hir },
        crate_ctx: ctx,
        diagnostic_sources,
        loaded_prelude_export_ids,
        infix_precedence,
        infix_owners,
        fingerprint_source_files,
        source_db,
        effective_current_crate_name,
        exported_macros,
        prelude_macros,
    })
}

/// A top-level module sharing a dependency's name would make every path that
/// starts with that name ambiguous between the two, for items and macros alike.
fn reject_modules_named_like_dependencies(
    root: &ast::Module,
    ctx: &CrateContext,
) -> Result<(), Diagnostics> {
    let mut diagnostics = Diagnostics::default();
    for top_level in &root.top_levels {
        let name = match top_level {
            ast::TopLevel::Mod(ident, _) => ident,
            ast::TopLevel::Module(ast::ModuleDecl(inline)) => match &inline.name {
                Some(name) => name,
                None => continue,
            },
            _ => continue,
        };
        if ctx.has_extern_crate(&name.name) {
            diagnostics.push(
                diagnostic::Diagnostic::new(
                    format!(
                        "module `{0}` conflicts with dependency `{0}`",
                        name.name
                    ),
                    name.span.clone(),
                )
                .with_code(diagnostic::DiagnosticCode::Resolve),
            );
        }
    }

    if diagnostics.0.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

/// Collects this crate's `< macro` declarations with their crate-relative
/// paths, and, for std, the prelude aliases that re-export them.
fn crate_macro_exports(
    root: &ast::Module,
    source_graph: &source_loader::ModuleGraph,
    current_crate_name: Option<&str>,
) -> (Vec<products::ProductMacroExport>, BTreeMap<String, String>) {
    // The graph can list a file module twice (as parsed, then as expanded);
    // the later, expanded entry wins.
    let file_modules = source_graph
        .modules()
        .map(|loaded| (loaded.qualified_name.as_str(), &loaded.module))
        .collect::<BTreeMap<_, _>>();
    let crate_relative = |qualified: &str| -> String {
        current_crate_name
            .and_then(|name| qualified.strip_prefix(name)?.strip_prefix("::"))
            .unwrap_or(qualified)
            .to_string()
    };

    let mut exported = Vec::new();
    macro_expansion::exports::collect_exported_macros(root, "", &mut exported);
    for (qualified, module) in &file_modules {
        macro_expansion::exports::collect_exported_macros(
            module,
            &crate_relative(qualified),
            &mut exported,
        );
    }

    let prelude_macros = match current_crate_name {
        Some(name @ "std") => file_modules
            .get(format!("{name}::prelude").as_str())
            .map(|prelude| {
                macro_expansion::exports::prelude_macro_aliases(prelude, name, &exported)
            })
            .unwrap_or_default(),
        _ => BTreeMap::new(),
    };

    (exported, prelude_macros)
}
