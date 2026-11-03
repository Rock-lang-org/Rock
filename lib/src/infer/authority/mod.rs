mod callable;
mod deferred_method;
mod instance_relation;
mod local_bindings;
mod materialize_expr;
mod materialize_function;
mod materialize_pending;
mod method_value;
mod obligations;
mod propagate;
mod propagate_call;
mod receivers;
mod sites;
mod status;
mod try_operator;
mod type_predicates;
mod type_vars;

pub(super) use materialize_pending::finalize_method_instantiations;
pub(super) use obligations::{
    authority_obligations, propagate_function_instances_for_owners_with_progress,
    run_authority_obligation,
};
pub(super) use status::{pending_authority_context, pending_authority_error};

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use crate::hir::{HirFunction, HirGenericBounds, HirTypeBinding};
use crate::ids::{DefId, HirLocalId};
use crate::infer::{ConstraintOwner, ConstraintStore, ObligationState};
use crate::language_items::TryLanguageItems;
use crate::types::Type;

fn native_call_type(ty: &Type) -> bool {
    match ty {
        Type::Function { .. } => true,
        Type::Reference { inner, .. } => native_call_type(inner),
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AuthorityPassResult {
    pub progress: bool,
    pub pending: bool,
    pub ambiguous: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct AuthorityObligationId(u32);

impl AuthorityObligationId {
    pub(super) const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum AuthorityObligationKind {
    Propagation,
    DeferredCall,
    CallableCall,
    MethodCall,
    Field,
    TryBranch,
    FromResidual,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AuthoritySiteId {
    kind: AuthorityObligationKind,
    file_path: std::path::PathBuf,
    start: usize,
    end: usize,
}

impl AuthoritySiteId {
    fn new(kind: AuthorityObligationKind, span: &crate::lexer::Span) -> Self {
        Self {
            kind,
            file_path: span.file_path.clone(),
            start: span.start,
            end: span.end,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct AuthorityObligation {
    pub id: AuthorityObligationId,
    pub owner: ConstraintOwner,
    pub kind: AuthorityObligationKind,
    pub state: ObligationState,
    pub last_generation: u64,
    pub attempts: u32,
    pub context: String,
    pub span: Option<crate::lexer::Span>,
    site: Option<AuthoritySiteId>,
    dependencies: HashSet<crate::ids::TypeVarId>,
}

impl AuthorityObligation {
    pub(super) fn depends_on_any(&self, changed: &HashSet<crate::ids::TypeVarId>) -> bool {
        self.dependencies.iter().any(|var| changed.contains(var))
    }

    pub(super) fn wake(&mut self) {
        if matches!(
            self.state,
            ObligationState::Pending | ObligationState::Ambiguous
        ) {
            self.state = ObligationState::Pending;
            self.last_generation = u64::MAX;
        }
    }
}

fn binding_sort_key(binding: &HirTypeBinding) -> (u32, u32, u32) {
    (
        binding.param.owner.crate_id.0,
        binding.param.owner.local.0,
        binding.param.index,
    )
}

struct MethodAuthorityContext<'a> {
    language_items: crate::hir::HirLanguageItems,
    functions: HashMap<DefId, HirFunction>,
    traits: &'a HashMap<DefId, crate::hir::HirTrait>,
    impls: &'a HashMap<DefId, crate::hir::HirImpl>,
    sized_trait_id: Option<DefId>,
    bounds: HirGenericBounds,
    constraint_store: &'a mut ConstraintStore,
    effective_trait_methods: &'a HashMap<(DefId, DefId), DefId>,
    resolver: &'a crate::collect::resolver::ResolverTables,
    structs: &'a HashMap<DefId, crate::hir::HirStruct>,
    engine: RefCell<&'a mut crate::infer::InferenceEngine>,
    try_protocol: Option<TryLanguageItems<DefId>>,
    unsafe_context: Cell<bool>,
    mutable_locals: HashSet<HirLocalId>,
    local_bindings: HashMap<HirLocalId, (String, Type, bool)>,
    next_local_id: u32,
    ambiguous: bool,
    selected_site: Option<AuthoritySiteId>,
    strict: bool,
    try_strict: bool,
}

#[cfg(test)]
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirStmt};
#[cfg(test)]
use crate::infer::PartialHir;
#[cfg(test)]
use instance_relation::propagate_instance_relation;
#[cfg(test)]
mod tests;
