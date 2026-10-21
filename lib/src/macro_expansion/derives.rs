use std::collections::HashSet;

use crate::ast::{DerivedDecl, Module, TopLevel};
use crate::diagnostic::Diagnostics;

use super::proc_macro::ProcMacroKind;
use super::registry::{BindingName, MacroDefinition, MacroEntry, MacroRegistry};
use super::{proc_macro_error, with_expansion_trace, MacroExpansionContext};

pub(super) fn validate(module: &Module, scope: &[BindingName], registry: &MacroRegistry, context: &MacroExpansionContext<'_>) -> Result<(), Diagnostics> {
    for item in &module.top_levels {
        match item {
            TopLevel::Module(child) => {
                let mut path = scope.to_vec();
                if let Some(name) = &child.0.name { path.push((name.name.clone(), name.context)); }
                validate(&child.0, &path, registry, context)?;
            }
            TopLevel::DerivedDecl(declaration) => validate_declaration(declaration, scope, registry, context)?,
            _ => {}
        }
    }
    Ok(())
}

fn validate_declaration(declaration: &DerivedDecl, scope: &[BindingName], registry: &MacroRegistry, context: &MacroExpansionContext<'_>) -> Result<(), Diagnostics> {
    resolve(declaration, scope, registry, context).map(|_| ())
}

pub(super) fn resolve<'a>(declaration: &DerivedDecl, scope: &[BindingName], registry: &'a MacroRegistry, context: &MacroExpansionContext<'_>) -> Result<Vec<&'a MacroEntry>, Diagnostics> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    if declaration.derives.is_empty() || declaration.target.original().is_none() {
        return Err(proc_macro_error("derive attachment has no original declaration input", declaration.target.name().span.clone()));
    }
    for attachment in &declaration.derives {
        let result = (|| {
            let path = crate::collect::resolver::binding_path(&attachment.path.path)
                .ok_or_else(|| proc_macro_error("invalid derive path", attachment.span.clone()))?;
            let entry = registry.resolve_contextual(scope, &path, &attachment.span, context)?
                .ok_or_else(|| proc_macro_error("unresolved derive macro path", attachment.span.clone()))?;
            if entry.proc_kind() != Some(&ProcMacroKind::DeriveLike) {
                return Err(proc_macro_error("%derive requires a DeriveLike procedural macro", attachment.span.clone()));
            }
            if !seen.insert(entry.identity.clone()) {
                return Err(proc_macro_error("duplicate canonical derive on one declaration", attachment.span.clone()));
            }
            entries.push(entry);
            Ok(())
        })();
        result.map_err(|diagnostics| attachment.parent.map(|parent| with_expansion_trace(diagnostics.clone(), context, parent)).unwrap_or(diagnostics))?;
    }
    Ok(entries)
}

pub(super) fn expand(declaration: DerivedDecl, entries: &[&MacroEntry], context: &MacroExpansionContext<'_>, scope: &[BindingName]) -> Result<Vec<TopLevel>, Diagnostics> {
    let target = match &declaration.target {
        crate::ast::DeriveTarget::Struct(node) => TopLevel::StructDecl(node.clone()),
        crate::ast::DeriveTarget::Enum(node) => TopLevel::EnumDecl(node.clone()),
    };
    let mut items = vec![target];
    let consumed = context.consume_derive(declaration.id, &declaration.target.name().span).map_err(|errors|
        declaration.derives.first().and_then(|attachment| attachment.parent)
            .map(|parent| with_expansion_trace(errors.clone(), context, parent)).unwrap_or(errors))?;
    // A second representation of the same node is not a second declaration or
    // another set of worker calls. Distinct nodes with identical text still run.
    if !consumed { return Ok(Vec::new()); }
    let original = declaration.target.original().ok_or_else(||
        proc_macro_error("derive attachment has no original declaration input", declaration.target.name().span.clone()))?;
    let input = original.stream().map_err(|error| proc_macro_error(format!("invalid derive declaration input: {error:?}"), declaration.target.name().span.clone()))?;
    for (attachment, entry) in declaration.derives.iter().zip(entries) {
        let invocation = crate::ast::MacroInvoc {
            header_indent: original.header_indent(), id: attachment.id, parent: attachment.parent,
            output: crate::ast::MacroOutputCategory::Items, path: attachment.path.clone(), span: attachment.span.clone(),
            args: input.clone(), arguments: Vec::new(), literal_ranges: Vec::new(), block: None,
        };
        let emitted = match &entry.definition {
            MacroDefinition::Procedural(entry) => match super::expand_procedural_kind(entry, &invocation, context, ProcMacroKind::DeriveLike)? {
                super::generated_parser::GeneratedSyntax::Items(module) => module.top_levels,
                super::generated_parser::GeneratedSyntax::Expression(_) => return Err(proc_macro_error("expected generated derive items", attachment.span.clone())),
            },
            MacroDefinition::SourceProcedural(_) => {
                let errors = proc_macro_error("a source procedural macro cannot execute while building its own provider; use a built dependency", attachment.span.clone());
                return Err(attachment.parent.map(|parent| with_expansion_trace(errors.clone(), context, parent)).unwrap_or(errors));
            }
            MacroDefinition::Declarative(_) => return Err(proc_macro_error("%derive requires a DeriveLike procedural macro", attachment.span.clone())),
        };
        context.note_source_modules(&emitted, &scope.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>(), attachment.id);
        items.extend(emitted);
    }
    Ok(items)
}

/// Never silently collect an unexecuted attachment, including inspection trees.
pub(crate) fn reject_pending(module: &Module) -> Result<(), Diagnostics> {
    for item in &module.top_levels {
        match item {
            TopLevel::Module(child) => reject_pending(&child.0)?,
            TopLevel::DerivedDecl(declaration) => {
                let span = declaration.derives.first().map(|attachment| attachment.span.clone())
                    .unwrap_or_else(|| declaration.target.name().span.clone());
                return Err(proc_macro_error("unexpanded derive attachment at expansion/collection boundary", span));
            }
            _ => {}
        }
    }
    Ok(())
}
