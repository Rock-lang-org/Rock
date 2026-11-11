//! Mutable state of the function body currently being lowered.
//!
//! Everything here is per body: `enter` starts a body with its own context,
//! source-scope chain, no enclosing loops and a tuple temp counter at 0, and
//! `leave` restores the enclosing body's state. Temp names only need to be
//! unique within one body (lambdas share their body's counter); each temp also
//! gets its own local id. Fold-loop frames follow lexical loop nesting and are
//! suspended around lambdas.

use std::collections::{HashMap, HashSet};

use crate::lower::body_context::BodyLoweringContext;
use crate::lower::control_flow::fold_loop::FoldLoopContext;

#[derive(Default)]
pub(crate) struct BodyState {
    context: Option<BodyLoweringContext>,
    source_scopes: Vec<u32>,
    fold_loops: Vec<Option<FoldLoopContext>>,
    reusable_call_scopes: Vec<usize>,
    tuple_temp_counter: u32,
    pub(crate) capture_live_after: HashMap<usize, HashSet<String>>,
    pub(crate) borrowed_capture_sources: HashSet<crate::ids::HirLocalId>,
}

/// State of the enclosing body, restored by [`BodyState::leave`].
#[must_use = "an entered body must be restored with BodyState::leave"]
pub(crate) struct BodyFrame {
    context: Option<BodyLoweringContext>,
    source_scopes: Vec<u32>,
    fold_loops: Vec<Option<FoldLoopContext>>,
    reusable_call_scopes: Vec<usize>,
    tuple_temp_counter: u32,
    capture_live_after: HashMap<usize, HashSet<String>>,
    borrowed_capture_sources: HashSet<crate::ids::HirLocalId>,
}

impl BodyState {
    /// Starts a new body whose source-scope chain begins at `root_scope`.
    pub(crate) fn enter(&mut self, context: BodyLoweringContext, root_scope: u32) -> BodyFrame {
        BodyFrame {
            context: self.context.replace(context),
            source_scopes: std::mem::replace(&mut self.source_scopes, vec![root_scope]),
            fold_loops: std::mem::take(&mut self.fold_loops),
            reusable_call_scopes: std::mem::take(&mut self.reusable_call_scopes),
            tuple_temp_counter: std::mem::take(&mut self.tuple_temp_counter),
            capture_live_after: std::mem::take(&mut self.capture_live_after),
            borrowed_capture_sources: std::mem::take(&mut self.borrowed_capture_sources),
        }
    }

    pub(crate) fn leave(&mut self, frame: BodyFrame) {
        self.context = frame.context;
        self.source_scopes = frame.source_scopes;
        self.fold_loops = frame.fold_loops;
        self.reusable_call_scopes = frame.reusable_call_scopes;
        self.tuple_temp_counter = frame.tuple_temp_counter;
        self.capture_live_after = frame.capture_live_after;
        self.borrowed_capture_sources = frame.borrowed_capture_sources;
    }

    pub(crate) fn context(&self) -> Option<&BodyLoweringContext> {
        self.context.as_ref()
    }

    pub(crate) fn context_mut(&mut self) -> Option<&mut BodyLoweringContext> {
        self.context.as_mut()
    }

    pub(crate) fn push_source_scope(&mut self, scope: u32) {
        self.source_scopes.push(scope);
    }

    pub(crate) fn pop_source_scope(&mut self) {
        self.source_scopes.pop();
    }

    pub(crate) fn current_source_scope(&self) -> Option<u32> {
        self.source_scopes.last().copied()
    }

    /// Enters a loop; `None` marks a loop that is not lowered to a fold.
    pub(crate) fn enter_loop(&mut self, fold: Option<FoldLoopContext>, scope: usize) {
        self.fold_loops.push(fold);
        self.enter_callable_scope(scope);
    }

    pub(crate) fn leave_loop(&mut self) {
        self.fold_loops.pop();
        self.leave_callable_scope();
    }

    pub(crate) fn enter_callable_scope(&mut self, scope: usize) {
        self.reusable_call_scopes.push(scope);
    }

    pub(crate) fn leave_callable_scope(&mut self) {
        self.reusable_call_scopes.pop();
    }

    pub(crate) fn requires_reusable_callable(&self, binding_scope: usize) -> bool {
        self.reusable_call_scopes.iter().any(|scope| binding_scope < *scope)
    }

    /// Hides the enclosing loops (a lambda body cannot break out of them).
    pub(crate) fn suspend_loops(&mut self) -> Vec<Option<FoldLoopContext>> {
        std::mem::take(&mut self.fold_loops)
    }

    pub(crate) fn resume_loops(&mut self, loops: Vec<Option<FoldLoopContext>>) {
        self.fold_loops = loops;
    }

    /// Every enclosing fold loop, outermost first.
    pub(crate) fn fold_loops(&self) -> impl Iterator<Item = &FoldLoopContext> {
        self.fold_loops.iter().flatten()
    }

    /// The innermost loop's fold context, if the innermost loop is a fold.
    pub(crate) fn innermost_fold_loop(&self) -> Option<&FoldLoopContext> {
        self.fold_loops.last().and_then(Option::as_ref)
    }

    pub(crate) fn fresh_tuple_temp_id(&mut self) -> u32 {
        let id = self.tuple_temp_counter;
        self.tuple_temp_counter = id
            .checked_add(1)
            .expect("tuple destructuring temp counter exhausted");
        id
    }
}
