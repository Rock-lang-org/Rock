//! Trait conformance checking

mod checks;
mod default_method_calls;
mod default_method_injector;
mod default_method_types;
mod generic_owner_remap;
mod index_alpha_equivalence;
mod phase;
mod service;
mod trait_impl_substitution;

pub(crate) use service::TraitConformancePhase;
#[cfg(test)]
pub(crate) use service::{IndexProtocolIds, TraitConformanceContext, TraitConformanceService};

#[cfg(test)]
mod tests;
