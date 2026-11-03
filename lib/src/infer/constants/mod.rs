//! Bounded constant arithmetic checking through selected method bodies.
//!
//! Local variables and ordinary function calls remain runtime values. A method
//! application is constant only when its operands and the selected pure body are
//! known; operator spellings and dependency profiles supply no arithmetic meaning.

mod evaluate;
mod mechanisms;
mod summarize;
mod summary;
mod validate;

pub(crate) use summarize::method_summaries;
pub use summary::{ArithmeticSummary, ArithmeticType};
pub(super) use validate::validate;

use std::collections::HashMap;

use crate::ids::HirLocalId;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Constant {
    Integer(i128),
    Bool(bool),
    Ref(Box<Constant>),
    Tuple(Vec<Constant>),
}

type Environment = HashMap<HirLocalId, Constant>;
type Evaluation = Result<Option<Constant>, &'static str>;

#[cfg(test)]
use crate::mir::integer::{IntegerIntrinsic, IntegerMode, IntegerOp};
#[cfg(test)]
use crate::types::Type;
#[cfg(test)]
use mechanisms::{integer, mechanism, width};
#[cfg(test)]
mod tests;
