use std::collections::HashMap;

use crate::hir::{HirImpl, HirImplReceiverPattern, HirMethodCallTarget};
use crate::type_services::normalize::apply_type_lambda;
use crate::types::{FunctionSafety, GenericParamDecl, GenericParamId, Type};

mod receiver_patterns;
mod type_patterns;

use receiver_patterns::*;
pub use receiver_patterns::{
    constructor_target_from_applied_type, constructor_target_substitution,
    impl_receiver_pattern_substitution, receiver_pattern, receiver_pattern_substitution,
};
pub use type_patterns::{infer_generic_subst_from_types, type_pattern_matches};

pub fn seed_receiver_substitution_from_impl(
    imp: &HirImpl,
    recv_ty: &Type,
    subst: &mut HashMap<GenericParamId, Type>,
) {
    if let Some(receiver_substitution) =
        receiver_pattern_substitution(&imp.receiver_pattern, recv_ty)
    {
        subst.extend(receiver_substitution);
    }

    for (decl, concrete) in imp.type_generics.iter().zip(receiver_pattern(recv_ty)) {
        subst.entry(decl.id).or_insert(concrete);
    }
}

pub fn generic_substitution_for_owner(
    params: &[GenericParamDecl],
    args: &[Type],
) -> HashMap<GenericParamId, Type> {
    params
        .iter()
        .zip(args.iter())
        .map(|(decl, arg)| (decl.id, arg.clone()))
        .collect()
}

pub fn target_matches_impl<P: crate::hir::HirPhase>(
    imp: &crate::hir::HirImplFor<P>,
    target: Option<&HirMethodCallTarget>,
) -> bool {
    let Some(target) = target else {
        return true;
    };

    if let Some(impl_id) = target.impl_id() {
        if imp.id != impl_id {
            return false;
        }
    }

    if let Some(trait_id) = target.trait_id() {
        if imp.trait_id != Some(trait_id) {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests;
