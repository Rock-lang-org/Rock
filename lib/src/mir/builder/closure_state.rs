use crate::hir::HirClosureCapture;
use crate::mir::{MirClosureCapture, MirFunction};

/// Closure bookkeeping for one builder: the lambda numbering, the nested
/// functions built for closure bodies, and the captures that connect a closure
/// body builder to its enclosing function.
///
/// Closure bodies are built by a fresh `MirBuilder`, so none of this state is
/// saved and restored around nested bodies; the child receives its captures via
/// `set_pending_body_captures` and hands its nested functions back to the parent.
#[derive(Default)]
pub(super) struct ClosureState {
    captures: Vec<MirClosureCapture>,
    lambda_counter: usize,
    nested_functions: Vec<MirFunction>,
    pending_body_captures: Vec<HirClosureCapture>,
    pub(super) borrows_owned_environment: bool,
}

impl ClosureState {
    /// Allocates the next lambda index and its `lambda_N` display name.
    pub(super) fn fresh_lambda(&mut self) -> (u32, String) {
        let index = self.lambda_counter;
        self.lambda_counter += 1;
        (index as u32, format!("lambda_{index}"))
    }

    /// Captures the closure body about to be built must declare as locals.
    pub(super) fn set_pending_body_captures(&mut self, captures: Vec<HirClosureCapture>) {
        self.pending_body_captures = captures;
    }

    pub(super) fn take_pending_body_captures(&mut self) -> Vec<HirClosureCapture> {
        std::mem::take(&mut self.pending_body_captures)
    }

    pub(super) fn record_capture(&mut self, capture: MirClosureCapture) {
        self.captures.push(capture);
    }

    pub(super) fn take_captures(&mut self) -> Vec<MirClosureCapture> {
        std::mem::take(&mut self.captures)
    }

    pub(super) fn push_nested(&mut self, function: MirFunction) {
        self.nested_functions.push(function);
    }

    pub(super) fn extend_nested(&mut self, functions: Vec<MirFunction>) {
        self.nested_functions.extend(functions);
    }

    pub(super) fn take_nested(&mut self) -> Vec<MirFunction> {
        std::mem::take(&mut self.nested_functions)
    }
}
