//! Expression lowering methods for Lowerer

mod entry;
mod function_instantiation;
mod literal;
mod operand;
mod operator_receiver_inference;
mod operator_selection;
mod precedence_tree;
mod primary;
mod unary;

pub(crate) use operand::ExprUse;

const APPLICATION_PRECEDENCE: u8 = 8;

#[cfg(test)]
mod tests;
