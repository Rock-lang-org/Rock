//! Mutable state of the function body currently being lowered.
//!
//! Everything here is per body: `enter` starts a body with its own context,
//! source-scope chain, no enclosing loops and a tuple temp counter at 0, and
//! `leave` restores the enclosing body's state. Temp names only need to be
//! unique within one body (lambdas share their body's counter); each temp also
//! gets its own local id. Fold-loop frames follow lexical loop nesting and are
//! suspended around lambdas.

use std::collections::HashSet;

use crate::ast;
use crate::lower::body_context::BodyLoweringContext;
use crate::lower::control_flow::fold_loop::FoldLoopContext;

#[derive(Default)]
pub(crate) struct BodyState {
    context: Option<BodyLoweringContext>,
    source_scopes: Vec<u32>,
    fold_loops: Vec<Option<FoldLoopContext>>,
    tuple_temp_counter: u32,
    /// Addresses of AST lambdas returned or passed to consuming callable parameters.
    escaping_lambdas: HashSet<usize>,
}

/// State of the enclosing body, restored by [`BodyState::leave`].
#[must_use = "an entered body must be restored with BodyState::leave"]
pub(crate) struct BodyFrame {
    context: Option<BodyLoweringContext>,
    source_scopes: Vec<u32>,
    fold_loops: Vec<Option<FoldLoopContext>>,
    tuple_temp_counter: u32,
    escaping_lambdas: HashSet<usize>,
}

impl BodyState {
    /// Starts a new body whose source-scope chain begins at `root_scope`.
    pub(crate) fn enter(&mut self, context: BodyLoweringContext, root_scope: u32) -> BodyFrame {
        BodyFrame {
            context: self.context.replace(context),
            source_scopes: std::mem::replace(&mut self.source_scopes, vec![root_scope]),
            fold_loops: std::mem::take(&mut self.fold_loops),
            tuple_temp_counter: std::mem::take(&mut self.tuple_temp_counter),
            escaping_lambdas: std::mem::take(&mut self.escaping_lambdas),
        }
    }

    pub(crate) fn leave(&mut self, frame: BodyFrame) {
        self.context = frame.context;
        self.source_scopes = frame.source_scopes;
        self.fold_loops = frame.fold_loops;
        self.tuple_temp_counter = frame.tuple_temp_counter;
        self.escaping_lambdas = frame.escaping_lambdas;
    }

    pub(crate) fn mark_escaping_lambda(&mut self, lambda: &ast::LambdaDecl) {
        self.escaping_lambdas.insert(lambda as *const _ as usize);
    }

    pub(crate) fn is_escaping_lambda(&self, lambda: &ast::LambdaDecl) -> bool {
        self.escaping_lambdas.contains(&(lambda as *const _ as usize))
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
    pub(crate) fn enter_loop(&mut self, fold: Option<FoldLoopContext>) {
        self.fold_loops.push(fold);
    }

    pub(crate) fn leave_loop(&mut self) {
        self.fold_loops.pop();
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
