//! Method-call monomorphization: drop glue, standalone/static/trait method
//! calls, and impl-method specialization.

mod call_targets;
mod drop;
mod impl_specialization;
mod standalone_call;
mod static_call;
mod trait_call;

#[cfg(test)]
mod tests;
