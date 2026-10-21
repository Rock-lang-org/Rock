//! Compiler-owned host glue over an explicitly supplied ordinary SDK contract.
//! Provider implementations stay ASTs; only bounded bootstrap syntax is parsed.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ast::{
    Block, Else, Expression, FunctionSig, IdentOrType, Literal, LiteralKind, Module,
    NodeId, Operand, ParseType, Path, PatternKind, Program, SecondaryExpr, Statement,
    TopLevel, UnaryExpr,
};
use crate::crate_system::CrateContext;
use crate::diagnostic::{DiagnosticSourceMap, Diagnostics};
use crate::lexer::lossless::SyntaxContext;
use crate::lexer::Span;
use crate::source_loader::ModuleGraph;

use super::registry::{MacroRegistry, SourceProcMacroExport};
use super::{proc_macro_error, MacroExpansionContext};

/// Explicit artifact dependency supplying MacroContext, TokenStream, MacroResult
/// and the ordinary SDK bootstrap facade (run_worker/entry_kind_matches/unknown_entry).
/// Its package spelling is never inferred or automatically loaded.
#[derive(Debug, Clone)]
pub struct ProcMacroSdk {
    pub dependency: String,
}

const SDK_EXPORTS: [&str; 7] = ["MacroContext", "TokenStream", "MacroResult", "MacroKind", "run_worker", "entry_kind_matches", "unknown_entry"];

/// Inspect explicitly supplied ordinary artifact interfaces, without running workers.
pub fn discover_sdks(artifacts: &[(String, PathBuf)]) -> Result<Vec<String>, Diagnostics> {
    let mut crates = CrateContext::new();
    crate::load_extern_artifacts(&mut crates, artifacts, PathBuf::from("<sdk-discovery>"))
        .map_err(|error| Diagnostics(vec![error]))?;
    let mut candidates: Vec<_> = crates.extern_crates().filter(|provider| provider.metadata().proc_macros().is_empty()
        && SDK_EXPORTS.iter().all(|name| provider.root_exports().contains_key(*name)))
        .map(|provider| provider.name().to_string()).collect();
    candidates.sort();
    Ok(candidates)
}

struct Entry {
    id: NodeId,
    name: String,
    scope: Vec<(String, SyntaxContext)>,
    call_context: SyntaxContext,
}

fn collect(module: &Module, scope: &[(String, SyntaxContext)], entries: &mut Vec<Entry>) {
    for item in &module.top_levels {
        match item {
            TopLevel::ProcMacroDecl(declaration) => entries.push(Entry {
                id: declaration.id, name: format!("__rock_entry_{}", entries.len()),
                scope: scope.to_vec(), call_context: SyntaxContext::ROOT,
            }),
            TopLevel::Module(child) => {
                let mut path = scope.to_vec();
                if let Some(name) = &child.0.name { path.push((name.name.clone(), name.context)); }
                collect(&child.0, &path, entries);
            }
            _ => {}
        }
    }
}

fn module_scope(name: &str, context: &MacroExpansionContext<'_>, registry: &MacroRegistry) -> Vec<(String, SyntaxContext)> {
    let mut path: Vec<_> = name.split("::").map(str::to_string).collect();
    if path.first().map(String::as_str) == context.config.current_crate_name.as_deref() { path.remove(0); }
    registry.loaded_scope(&path)
}

fn template(count: usize, derives: &HashSet<usize>, span: &Span) -> Result<String, Diagnostics> {
    let mut source = String::from("mod __rock_proc_wrapper\n    > sdk::MacroContext\n    > sdk::TokenStream\n    > sdk::MacroResult\n    > sdk::MacroKind\n    > sdk::run_worker\n    > sdk::entry_kind_matches\n    > sdk::unknown_entry\n    __rock_dispatch: MacroContext -> TokenStream -> MacroResult\n    __rock_dispatch = context, input ->\n        match context.macro_identity.as_str!\n");
    for index in 0..count {
        let kind = if derives.contains(&index) { "DeriveLike" } else { "FunctionLike" };
        source.push_str(&format!("            \"__rock_export_{index}\" =>\n                if entry_kind_matches &context, MacroKind::{kind}\n                    __rock_selected_{index} context, input\n                else\n                    unknown_entry context, input\n"));
        if source.len() > super::token_tree::MACRO_LIMITS.max_source_bytes {
            return Err(proc_macro_error("generated host wrapper exceeds the source byte limit", span.clone()));
        }
    }
    source.push_str("            _ => unknown_entry context, input\n    __rock_boot = !->\n        run_worker \"__rock_compiler_version\", __rock_dispatch\nmain = !-> __rock_boot!\n");
    if source.len() > super::token_tree::MACRO_LIMITS.max_source_bytes {
        return Err(proc_macro_error("generated host wrapper exceeds the source byte limit", span.clone()));
    }
    Ok(source)
}

fn signature_types(ty: &mut ParseType, context: SyntaxContext) {
    match ty {
        ParseType::Function(types) => for ty in types { signature_types(ty, context); },
        ParseType::Type(inner) => inner.context = context,
        _ => {}
    }
}

fn add_implementations(module: &mut Module, entries: &HashMap<NodeId, &Entry>, signature: &FunctionSig,
    binding: SyntaxContext, types: SyntaxContext) {
    let mut implementations = Vec::new();
    for item in &mut module.top_levels {
        match item {
            TopLevel::ProcMacroDecl(declaration) => {
                let Some(entry) = entries.get(&declaration.id) else { continue; };
                let mut function = declaration.function.clone();
                function.name.name = entry.name.clone(); function.name.context = binding;
                function.exported = false;
                let mut signature = signature.clone(); signature.name = function.name.clone();
                signature_types(&mut signature.sig, types);
                implementations.push(TopLevel::FunctionSig(signature));
                implementations.push(TopLevel::FunctionDecl(function));
            }
            TopLevel::Module(child) => add_implementations(&mut child.0, entries, signature, binding, types),
            _ => {}
        }
    }
    module.top_levels.extend(implementations);
}

// This walk is restricted to compiler-owned template syntax. It never rewrites
// provider bodies, parameter bindings, user identifiers or captured expressions.
struct Rewrite<'a> {
    selected: &'a [(&'a Entry, &'a SourceProcMacroExport)],
    boot: SyntaxContext,
}

impl Rewrite<'_> {
    fn block(&self, block: &mut Block) {
        for statement in &mut block.statements {
            if let Statement::Expression(expression) = statement { self.expression(expression); }
        }
    }

    fn expression(&self, expression: &mut Expression) {
        let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else { return; };
        match &mut primary.operand {
            Operand::Ident(path) => for part in &mut path.path {
                let IdentOrType::Ident(name) = part else { continue; };
                if name.name == "__rock_boot" { name.context = self.boot; }
                if let Some(index) = name.name.strip_prefix("__rock_selected_").and_then(|value| value.parse::<usize>().ok()) {
                    if let Some((entry, _)) = self.selected.get(index) {
                        name.name = entry.name.clone(); name.context = entry.call_context;
                    }
                }
            },
            Operand::If(condition) => {
                self.expression(&mut condition.condition.expression); self.block(&mut condition.then);
                if let Some(Else::Block(block)) = &mut condition.else_ { self.block(block); }
            }
            Operand::Match(matched) => {
                self.expression(&mut matched.expr);
                for arm in &mut matched.arms {
                    if let PatternKind::Literal(Literal { kind: LiteralKind::String(value), .. }) = &mut arm.pattern.kind {
                        if let Some(index) = value.strip_prefix("__rock_export_").and_then(|value| value.parse::<usize>().ok()) {
                            if let Some((_, export)) = self.selected.get(index) { *value = export.export.identity.clone(); }
                        }
                    }
                    self.block(&mut arm.body);
                }
            }
            Operand::Literal(Literal { kind: LiteralKind::String(value), .. }) if value == "__rock_compiler_version" => {
                *value = env!("CARGO_PKG_VERSION").into();
            }
            _ => {}
        }
        for secondary in primary.secondaries.iter_mut().flatten() {
            if let SecondaryExpr::Arguments(arguments) = secondary {
                for argument in arguments { self.expression(&mut argument.arg); }
            }
        }
    }
}

pub(crate) fn install(ast: &mut Program, graph: &mut ModuleGraph, context: &MacroExpansionContext<'_>,
    crates: &CrateContext, registry: &MacroRegistry, sdk: &ProcMacroSdk, exports: &[SourceProcMacroExport],
    diagnostic_sources: &mut DiagnosticSourceMap) -> Result<(), Diagnostics> {
    let span = Span::new(context.config.entry_file.clone(), 0, 0);
    let failure = |message: String| proc_macro_error(message, span.clone());
    if exports.is_empty() { return Err(failure("procedural macro provider has no exported entries".into())); }
    if !context.config.extern_artifacts.iter().any(|(name, _)| name == &sdk.dependency) {
        return Err(failure(format!("host wrapper requires an explicitly supplied SDK artifact '{}'", sdk.dependency)));
    }
    let provider = crates.extern_crate(&sdk.dependency)
        .ok_or_else(|| failure(format!("SDK dependency '{}' is not an ordinary loaded library", sdk.dependency)))?;
    for required in SDK_EXPORTS {
        if !provider.root_exports().contains_key(required) {
            return Err(failure(format!("SDK '{}' has no public contract export '{required}'", sdk.dependency)));
        }
    }
    let identity = crates.product_crate_ids.iter().find_map(|(identity, id)| (*id == provider.crate_id()).then_some(identity))
        .ok_or_else(|| failure("SDK dependency has no supplied product identity".into()))?;
    let binding = context.fresh_private_context().map_err(failure)?;
    let sdk_context = context.fresh_dependency_context(identity).map_err(failure)?;
    let wrapper_path = vec![("__rock_proc_wrapper".into(), binding)];
    let types = context.fresh_source_context(wrapper_path.clone(), SyntaxContext::ROOT).map_err(failure)?;
    let boot = context.fresh_source_context(wrapper_path, SyntaxContext::ROOT).map_err(failure)?;
    let mut entries = Vec::new(); collect(&ast.module, &[], &mut entries);
    let mut modules: Vec<_> = graph.modules().filter(|module| Some(&module.path) != ast.module.filepath.as_ref()).collect();
    modules.sort_by(|left, right| left.qualified_name.cmp(&right.qualified_name));
    for module in modules { collect(&module.module, &module_scope(&module.qualified_name, context, registry), &mut entries); }
    for entry in &mut entries {
        entry.call_context = context.fresh_source_context(entry.scope.clone(), binding).map_err(failure)?;
    }
    let entries_by_id: HashMap<_, _> = entries.iter().map(|entry| (entry.id, entry)).collect();
    let selected = exports.iter().map(|export| entries_by_id.get(&export.definition).map(|entry| (*entry, export))
        .ok_or_else(|| failure("source export has no retained implementation".into()))).collect::<Result<Vec<_>, _>>()?;
    let derives = selected.iter().enumerate().filter_map(|(index, (_, export))|
        (export.export.kind == super::proc_macro::ProcMacroKind::DeriveLike).then_some(index)).collect();
    let source = template(selected.len(), &derives, &span)?;
    let bytes = selected.iter().try_fold(source.len(), |bytes, (_, export)| bytes.checked_add(export.export.identity.len()));
    if bytes.is_none_or(|bytes| bytes > super::token_tree::MACRO_LIMITS.max_source_bytes) {
        return Err(failure("generated host identities exceed the source byte limit".into()));
    }
    let path = PathBuf::from(format!("<proc-wrapper:{}>", context.config.entry_file.display()));
    diagnostic_sources.insert_virtual(path.clone(), source.clone());
    let mut generated = crate::parser::parse_source(path, &source, context.config).map_err(Diagnostics::from)?;
    let Some(TopLevel::Module(wrapper)) = generated.top_levels.first_mut() else {
        return Err(failure("invalid generated host module".into()));
    };
    let Some(name) = &mut wrapper.0.name else { return Err(failure("unnamed generated host module".into())); };
    name.context = binding;
    for item in &mut wrapper.0.top_levels {
        if let TopLevel::Import(path) = item {
            let parts = match path { Path::Ident(path) => &mut path.path, Path::Type(path) => &mut path.path };
            let Some(IdentOrType::Ident(head)) = parts.first_mut() else { return Err(failure("invalid SDK import head".into())); };
            head.name = sdk.dependency.clone(); head.context = sdk_context;
        }
    }
    let signature = wrapper.0.top_levels.iter().find_map(|item| match item {
        TopLevel::FunctionSig(signature) => Some(signature.clone()), _ => None,
    }).ok_or_else(|| failure("missing generated entry signature".into()))?;
    let rewrite = Rewrite { selected: &selected, boot };
    for item in &mut wrapper.0.top_levels {
        if let TopLevel::FunctionDecl(function) = item { rewrite.block(&mut function.lambda.body); }
    }
    for item in generated.top_levels.iter_mut().skip(1) {
        if let TopLevel::FunctionDecl(function) = item { rewrite.block(&mut function.lambda.body); }
    }
    add_implementations(&mut ast.module, &entries_by_id, &signature, binding, types);
    for module in graph.modules_mut() {
        if Some(&module.path) != ast.module.filepath.as_ref() {
            add_implementations(&mut module.module, &entries_by_id, &signature, binding, types);
        }
    }
    ast.module.top_levels.extend(generated.top_levels);
    *graph.root_module_mut() = ast.module.clone();
    for module in graph.modules_mut() {
        if Some(&module.path) == ast.module.filepath.as_ref() { module.module = ast.module.clone(); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrapper_signature(count: usize) -> FunctionSig {
        let source = template(count, &HashSet::new(), &Span::test()).unwrap();
        let parsed = crate::parser::parse_string(&source, &crate::Config::default()).unwrap();
        let TopLevel::Module(wrapper) = &parsed.module.top_levels[0] else { panic!("wrapper") };
        let signature = wrapper.0.top_levels.iter().find_map(|item| match item { TopLevel::FunctionSig(signature) => Some(signature), _ => None }).expect("signature");
        assert_eq!(parsed.module.top_levels.len(), 2, "only private module and main");
        signature.clone()
    }

    #[test]
    fn macro_b_13_bootstrap_is_ordinary_bounded_syntax() {
        for count in [0, 1, 2, 16] {
            let signature = wrapper_signature(count);
            let ParseType::Function(parts) = signature.sig else { panic!("ordinary arrow signature") };
            assert_eq!(parts.len(), 3);
        }
        let mixed = template(2, &HashSet::from([1]), &Span::test()).unwrap();
        assert!(mixed.contains("MacroKind::FunctionLike") && mixed.contains("MacroKind::DeriveLike"));
        crate::parser::parse_string(&mixed, &crate::Config::default()).unwrap();
        let errors = template(usize::MAX, &HashSet::new(), &Span::test()).unwrap_err();
        assert!(errors.0[0].message.contains("source byte limit"));
    }

    #[test]
    fn macro_b_13_retains_implementation_ast_and_parameter_contexts() {
        let config = crate::Config::default();
        let context = MacroExpansionContext::new(&config);
        let mut program = crate::parser::parse_string(
            "macro same = context, input ->\n    retained = input\n    retained\nsame = -> 42\n", &config).unwrap();
        let TopLevel::ProcMacroDecl(declaration) = &program.module.top_levels[0] else { panic!("proc entry") };
        let original = declaration.clone();
        let mut entries = Vec::new(); collect(&program.module, &[], &mut entries);
        let entries = entries.iter().map(|entry| (entry.id, entry)).collect();
        let binding = context.fresh_private_context().unwrap();
        let types = context.fresh_private_context().unwrap();
        add_implementations(&mut program.module, &entries, &wrapper_signature(1), binding, types);
        assert_eq!(program.module.top_levels[0], TopLevel::ProcMacroDecl(original.clone()));
        let TopLevel::FunctionDecl(function) = &program.module.top_levels[3] else { panic!("implementation") };
        assert_eq!(function.lambda, original.function.lambda);
        assert_eq!(function.name.span, original.function.name.span);
        assert_eq!(function.name.context, binding);
        assert!(!function.exported);
        let TopLevel::FunctionSig(signature) = &program.module.top_levels[2] else { panic!("SDK signature") };
        let ParseType::Function(parts) = &signature.sig else { panic!("arrow") };
        for part in parts {
            let ParseType::Type(ty) = part else { panic!("SDK nominal type") };
            assert_eq!(ty.context, types);
        }
    }

    #[test]
    fn macro_b_13_private_contexts_are_fresh_and_authority_is_explicit() {
        let config = crate::Config::default();
        let context = MacroExpansionContext::new(&config);
        let private = context.fresh_private_context().unwrap();
        let bridge = context.fresh_source_context(vec![("scope".into(), private)], private).unwrap();
        let dependency = context.fresh_dependency_context(&crate::products::ProductCrateIdentity::local("sdk".into())).unwrap();
        assert_ne!(private, SyntaxContext::ROOT);
        assert_ne!(private, bridge);
        assert_ne!(bridge, dependency);
        assert_ne!(private, dependency);
        assert_eq!(context.binding_environments.borrow().source_scope(private), None);
        assert_eq!(context.binding_environments.borrow().source_scope(bridge), Some((vec![("scope".into(), private)], private)));
        assert!(context.binding_environments.borrow().resolve(&CrateContext::new()).is_err(), "missing dependency cannot mint authority");
    }
}
