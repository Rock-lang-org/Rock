//! `unused_variables`: locals, parameters and pattern bindings never read.
//!
//! One walk per current-crate body after inference. Assigning to a local is
//! not a read. Bindings without a source span are compiler-generated and
//! never reported.

use std::collections::{BTreeMap, HashSet};

use crate::diagnostic::Diagnostic;
use crate::hir::{
    AcceptedHirExpr, AcceptedHirFunction, HirCallTarget, HirExprKindFor, HirVarTarget,
};
use crate::ids::{DefId, HirLocalId};
use crate::infer::ResolvedHirProgram;

use super::walk::{walk_expr, walk_function, HirVisitor};
use super::{is_intentionally_unused, Lint, LintSink};

pub(crate) fn check_unused_variables(hir: &ResolvedHirProgram, sink: &mut LintSink<'_>) {
    if !sink.is_enabled(Lint::UnusedVariables) {
        return;
    }
    let program = &hir.program;
    let bodies = program
        .functions
        .values()
        .chain(program.impls.values().flat_map(|imp| imp.methods.values()))
        .chain(program.traits.values().flat_map(|tr| tr.methods.values()));
    for function in bodies {
        if hir.current_def_ids.contains(&function.id) {
            check_function(hir, function, sink);
        }
    }
}

fn check_function(
    hir: &ResolvedHirProgram,
    function: &AcceptedHirFunction,
    sink: &mut LintSink<'_>,
) {
    let mut locals = LocalUses::default();
    walk_function(&mut locals, function);
    for (local, name) in locals.declared {
        if locals.read.contains(&local) || name == "self" || is_intentionally_unused(&name) {
            continue;
        }
        let Some(span) = local_span(hir, function.id, local) else {
            continue;
        };
        sink.emit(
            Lint::UnusedVariables,
            Diagnostic::new(format!("unused variable: `{name}`"), span).with_help(format!(
                "if this is intentional, prefix it with an underscore: `_{name}`"
            )),
        );
    }
}

fn local_span(hir: &ResolvedHirProgram, owner: DefId, local: HirLocalId) -> Option<crate::Span> {
    hir.source_map.local_span(owner, local).cloned()
}

#[derive(Default)]
struct LocalUses {
    declared: BTreeMap<HirLocalId, String>,
    read: HashSet<HirLocalId>,
}

impl HirVisitor for LocalUses {
    fn enter_expr(&mut self, expr: &AcceptedHirExpr) -> bool {
        match &expr.kind {
            HirExprKindFor::ResolvedVar(var) => {
                if let HirVarTarget::Local(local) = var.target {
                    self.read.insert(local);
                }
            }
            HirExprKindFor::Call(_, _, Some(HirCallTarget::Local(local))) => {
                self.read.insert(*local);
            }
            HirExprKindFor::Lambda { captures, .. } => {
                self.read
                    .extend(captures.iter().map(|capture| capture.local_id));
            }
            // Writing a local is not reading it: only the assigned value is walked.
            HirExprKindFor::Assign(target, value) if is_local(target) => {
                walk_expr(self, value);
                return false;
            }
            _ => {}
        }
        true
    }

    fn bind_local(&mut self, name: &str, local: HirLocalId) {
        self.declared.insert(local, name.to_string());
    }
}

fn is_local(expr: &AcceptedHirExpr) -> bool {
    matches!(
        &expr.kind,
        HirExprKindFor::ResolvedVar(var) if matches!(var.target, HirVarTarget::Local(_))
    )
}
