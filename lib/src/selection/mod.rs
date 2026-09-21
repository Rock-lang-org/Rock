mod matching;
mod service;
mod types;

pub use matching::{
    constructor_target_from_applied_type, constructor_target_substitution,
    generic_substitution_for_owner, impl_receiver_pattern_substitution,
    infer_generic_subst_from_types, receiver_pattern, receiver_pattern_substitution,
    seed_receiver_substitution_from_impl, target_matches_impl, type_pattern_matches,
};
pub use service::SelectionService;
pub use types::{
    ReceiverAdjustment, ReceiverCandidate, SelectedConstructorMember, SelectedMethod,
    SelectedOrigin, SelectionAuthority, SelectionDiagnostic,
};
