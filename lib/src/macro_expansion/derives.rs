use std::collections::HashSet;

use crate::ast::{DerivedDecl, Module, TopLevel};
use crate::diagnostic::Diagnostics;

use super::proc_macro::ProcMacroKind;
use super::registry::{BindingName, MacroRegistry};
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
    let mut seen = HashSet::new();
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
            Ok(())
        })();
        result.map_err(|diagnostics| attachment.parent.map(|parent| with_expansion_trace(diagnostics.clone(), context, parent)).unwrap_or(diagnostics))?;
    }
    Ok(())
}

/// C.02 validates metadata only. Never silently collect an unexecuted attachment;
/// C.09 will consume these nodes through the ordinary bounded expansion engine.
pub(crate) fn reject_pending(module: &Module) -> Result<(), Diagnostics> {
    for item in &module.top_levels {
        match item {
            TopLevel::Module(child) => reject_pending(&child.0)?,
            TopLevel::DerivedDecl(declaration) => {
                let span = declaration.derives.first().map(|attachment| attachment.span.clone())
                    .unwrap_or_else(|| declaration.target.name().span.clone());
                return Err(proc_macro_error("derive attachment has not been expanded; derive execution is not implemented", span));
            }
            _ => {}
        }
    }
    Ok(())
}
