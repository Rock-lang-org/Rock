use std::cell::RefCell;

use crate::lexer::Span;

use super::{IResult, Input, ParseError};

const MAX_DEPTH: usize = 512;
const FALLBACK_DEPTH: usize = 128;
const STACK_RESERVE: usize = 256 * 1024;
pub(crate) const MAX_PARSE_WORK: usize = 8 * 1024 * 1024;

// These targets have downward-growing native stacks. Query once per root parse;
// leave headroom for diagnostics, unwinding and the next unguarded parser frame.
#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
fn stack_bounds() -> Option<std::ops::Range<usize>> {
    // SAFETY: pthread_self names the live calling thread; only a successful
    // getattr initializes attr, which is destroyed after querying its bounds.
    unsafe {
        let mut attr = std::mem::MaybeUninit::<libc::pthread_attr_t>::uninit();
        if libc::pthread_getattr_np(libc::pthread_self(), attr.as_mut_ptr()) != 0 {
            return None;
        }
        let mut attr = attr.assume_init();
        let mut address = std::ptr::null_mut();
        let mut size = 0;
        let result = libc::pthread_attr_getstack(&attr, &mut address, &mut size);
        libc::pthread_attr_destroy(&mut attr);
        if result != 0 { return None; }
        let start = address as usize;
        Some(start..start.checked_add(size)?)
    }
}

#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
fn stack_bounds() -> Option<std::ops::Range<usize>> { None }

struct Budget {
    work: usize,
    limit: usize,
    depth: usize,
    stack: Option<std::ops::Range<usize>>,
    failure: Option<ParseError>,
}

thread_local! {
    // Scoped to one synchronous parse, including its nested fragment parsers.
    static BUDGET: RefCell<Option<Budget>> = const { RefCell::new(None) };
}

pub(crate) struct ParseWorkScope {
    root: bool,
    start: usize,
}

impl ParseWorkScope {
    pub(crate) fn new(limit: usize) -> Self {
        BUDGET.with(|slot| {
            let mut slot = slot.borrow_mut();
            let root = slot.is_none();
            let budget = slot.get_or_insert_with(|| Budget {
                work: 0, limit, depth: 0, stack: stack_bounds(), failure: None,
            });
            Self { root, start: budget.work }
        })
    }

    pub(crate) fn used(&self) -> usize {
        BUDGET.with(|slot| slot.borrow().as_ref().map_or(0, |budget| budget.work - self.start))
    }

    pub(crate) fn finish<T>(&self, result: Result<T, ParseError>) -> Result<T, ParseError> {
        match BUDGET.with(|slot| slot.borrow().as_ref().and_then(|budget| budget.failure.clone())) {
            Some(error) => Err(error),
            None => result,
        }
    }
}

impl Drop for ParseWorkScope {
    fn drop(&mut self) {
        if self.root { BUDGET.with(|slot| *slot.borrow_mut() = None); }
    }
}

pub(super) fn charge(span: impl FnOnce() -> Span) -> Result<(), ParseError> {
    BUDGET.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(budget) = slot.as_mut() else { return Ok(()); };
        if let Some(error) = &budget.failure { return Err(error.clone()); }
        let marker = 0u8;
        let position = &marker as *const u8 as usize;
        let stack_exhausted = match &budget.stack {
            Some(bounds) => !bounds.contains(&position) || position - bounds.start < STACK_RESERVE,
            None => budget.depth >= FALLBACK_DEPTH,
        };
        let resource = if stack_exhausted || budget.depth >= MAX_DEPTH { Some("depth") }
            else if budget.work >= budget.limit { Some("work") } else { None };
        if let Some(resource) = resource {
            let error = ParseError::HardError(format!("parser {resource} limit exceeded"), span());
            budget.failure = Some(error.clone());
            return Err(error);
        }
        budget.work += 1;
        Ok(())
    })
}

struct Depth;

impl Drop for Depth {
    fn drop(&mut self) {
        BUDGET.with(|slot| {
            if let Some(budget) = slot.borrow_mut().as_mut() { budget.depth -= 1; }
        });
    }
}

pub(crate) fn invoke<'a, T>(input: Input<'a>, parse: impl FnOnce(Input<'a>) -> IResult<'a, T>) -> IResult<'a, T> {
    let scope = ParseWorkScope::new(MAX_PARSE_WORK);
    charge(|| input.eof_span())?;
    BUDGET.with(|slot| slot.borrow_mut().as_mut().unwrap().depth += 1);
    let _depth = Depth;
    scope.finish(parse(input))
}
