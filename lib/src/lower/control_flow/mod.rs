//! Control flow lowering: if, match, loop, and postfix operations

mod callable;
pub(crate) mod fold_loop;
mod if_match;
mod loops;
pub(crate) mod pattern;
mod pattern_arrays;
mod secondary;
