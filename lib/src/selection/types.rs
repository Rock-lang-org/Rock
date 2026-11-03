use std::collections::HashMap;

use crate::hir::{
    HirAssociatedTypeDef, HirExpr, HirFunction, HirImpl, HirMethodCallTarget, HirParam,
    HirSelectedMethodTarget, HirTypeBinding,
};
use crate::ids::DefId;
use crate::types::{GenericParamId, TraitBound, Type};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverAdjustment {
    None,
    AutorefShared,
    AutorefMut,
    MutToSharedRef,
    BuiltinDeref,
    TraitDeref,
    ArrayRefToSliceRef,
    ArrayValueToMutSliceRef,
    ArrayValueToSliceRef,
    ArrayValueToSliceValue,
}

#[derive(Debug, Clone)]
pub struct ReceiverCandidate {
    pub expr: HirExpr,
    pub adjustment: ReceiverAdjustment,
    pub can_autoref_mut: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectedOrigin {
    InherentImpl { impl_id: DefId },
    TraitImpl { impl_id: DefId, trait_id: DefId },
    TraitBound { trait_id: DefId },
    CurrentTrait { trait_id: DefId },
    UnresolvedGeneric { trait_id: DefId },
}

#[derive(Debug, Clone)]
pub struct SelectedMethod {
    pub receiver: HirExpr,
    pub function: Option<HirFunction>,
    pub impl_def: Option<HirImpl>,
    pub target: HirMethodCallTarget,
    pub origin: SelectedOrigin,
    pub receiver_adjustment: ReceiverAdjustment,
    pub substituted_params: Vec<HirParam>,
    pub return_type: Type,
    pub pending_impl_bounds: Vec<(Type, TraitBound)>,
    pub associated_types: Vec<HirAssociatedTypeDef>,
    pub owner_substitution: HashMap<GenericParamId, Type>,
    pub owner_generic_params: Vec<GenericParamId>,
}

#[derive(Debug, Clone)]
pub struct SelectedConstructorMember {
    pub function: HirFunction,
    pub impl_def: HirImpl,
    pub target: HirMethodCallTarget,
    pub substituted_params: Vec<HirParam>,
    pub return_type: Type,
    pub pending_impl_bounds: Vec<(Type, TraitBound)>,
    pub owner_substitution: HashMap<GenericParamId, Type>,
    pub owner_generic_params: Vec<GenericParamId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionAuthority {
    pub origin: SelectedOrigin,
    pub target: HirSelectedMethodTarget,
    pub receiver_adjustment: ReceiverAdjustment,
    pub return_type: Type,
}

impl SelectionAuthority {
    pub fn impl_id(&self) -> Option<DefId> {
        match self.target {
            HirSelectedMethodTarget::ImplMethod { impl_id, .. } => Some(impl_id),
            HirSelectedMethodTarget::TraitMethod { .. } => None,
        }
    }

    pub fn trait_id(&self) -> Option<DefId> {
        match &self.target {
            HirSelectedMethodTarget::ImplMethod { selected_trait, .. } => {
                selected_trait.as_ref().map(|selected| selected.trait_id)
            }
            HirSelectedMethodTarget::TraitMethod { trait_id, .. } => Some(*trait_id),
        }
    }

    pub fn method_id(&self) -> Option<DefId> {
        match self.target {
            HirSelectedMethodTarget::ImplMethod { method_id, .. } => Some(method_id),
            HirSelectedMethodTarget::TraitMethod { member_id, .. } => Some(member_id),
        }
    }

    pub fn trait_args(&self) -> &[Type] {
        match &self.target {
            HirSelectedMethodTarget::ImplMethod { selected_trait, .. } => selected_trait
                .as_ref()
                .map(|selected| selected.trait_args.as_slice())
                .unwrap_or_default(),
            HirSelectedMethodTarget::TraitMethod { trait_args, .. } => trait_args,
        }
    }
}

impl SelectedMethod {
    pub fn authority(&self) -> SelectionAuthority {
        SelectionAuthority {
            origin: self.origin.clone(),
            target: self.target.target.clone(),
            receiver_adjustment: self.receiver_adjustment,
            return_type: self.return_type.clone(),
        }
    }

    /// Build the canonical call target after method generic inference.
    ///
    /// Selection owns the target shape; callers only provide the inferred
    /// substitutions and their normalizer.
    pub fn target_with_substitution<F>(
        &self,
        substitution: &HashMap<GenericParamId, Type>,
        mut normalize: F,
    ) -> HirMethodCallTarget
    where
        F: FnMut(&Type) -> Type,
    {
        let mut target = self.target.clone();
        if let Some(trait_args) = target.trait_args_mut() {
            *trait_args = trait_args
                .iter()
                .map(|arg| normalize(&arg.substitute_generics(substitution)))
                .collect();
        }

        for param in &self.owner_generic_params {
            if target
                .owner_substitution
                .iter()
                .any(|binding| binding.param == *param)
            {
                continue;
            }
            if let Some(ty) = substitution
                .get(param)
                .or_else(|| self.owner_substitution.get(param))
            {
                target.owner_substitution.push(HirTypeBinding {
                    param: *param,
                    ty: normalize(ty),
                });
            }
        }
        target.owner_substitution.sort_by_key(binding_sort_key);

        let owner_params = target
            .owner_substitution
            .iter()
            .map(|binding| binding.param)
            .collect::<std::collections::HashSet<_>>();
        let method_params = self
            .function
            .as_ref()
            .map(|function| {
                function
                    .generic_params
                    .iter()
                    .map(|param| param.id)
                    .filter(|param| !owner_params.contains(param))
                    .filter(|param| Some(param.owner) != target.trait_id())
                    .collect::<std::collections::HashSet<_>>()
            })
            .unwrap_or_default();
        target.method_substitution = substitution
            .iter()
            .filter(|(param, _)| method_params.contains(param))
            .map(|(&param, ty)| HirTypeBinding {
                param,
                ty: normalize(ty),
            })
            .collect();
        target.method_substitution.sort_by_key(binding_sort_key);
        target
    }
}

fn binding_sort_key(binding: &HirTypeBinding) -> (u32, u32, u32) {
    (
        binding.param.owner.crate_id.0,
        binding.param.owner.local.0,
        binding.param.index,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionDiagnostic {
    NoImplementation {
        operation: String,
        receiver: Type,
    },
    ReceiverMismatch {
        operation: String,
        receiver: Type,
    },
    AmbiguousCandidates {
        operation: String,
        receiver: Type,
        candidates: Vec<DefId>,
    },
    SelectedTargetMissing {
        method_name: String,
        target: HirMethodCallTarget,
    },
    TraitMemberMissing {
        trait_id: DefId,
        method_name: String,
    },
    TraitMemberIdMissing {
        trait_id: DefId,
        member_id: DefId,
    },
}

impl SelectionDiagnostic {
    pub fn message(&self) -> String {
        let context = crate::type_services::display::TypeDisplayContext::default();
        self.message_with_names(
            |ty| crate::type_services::display::display_type_with_context(ty, &context).to_string(),
            |_| None,
        )
    }

    pub fn message_with(&self, display_type: impl Fn(&Type) -> String) -> String {
        self.message_with_names(display_type, |_| None)
    }

    pub fn message_with_names(
        &self,
        display_type: impl Fn(&Type) -> String,
        display_definition: impl Fn(DefId) -> Option<String>,
    ) -> String {
        match self {
            SelectionDiagnostic::NoImplementation {
                operation,
                receiver,
            } => {
                format!(
                    "No implementation found for operator '{}' on type {}",
                    operation,
                    display_type(receiver)
                )
            }
            SelectionDiagnostic::ReceiverMismatch {
                operation,
                receiver,
            } => {
                format!(
                    "No receiver adjustment for '{}' on type {}",
                    operation,
                    display_type(receiver)
                )
            }
            SelectionDiagnostic::AmbiguousCandidates {
                operation,
                receiver,
                candidates,
            } => {
                let candidates = candidates
                    .iter()
                    .filter_map(|id| display_definition(*id))
                    .collect::<Vec<_>>();
                let candidates = if candidates.is_empty() {
                    "multiple matching implementations".to_string()
                } else {
                    candidates.join(", ")
                };
                format!(
                    "Ambiguous selection for '{}' on type {}: {}",
                    operation,
                    display_type(receiver),
                    candidates
                )
            }
            SelectionDiagnostic::SelectedTargetMissing { method_name, .. } => format!(
                "Selected method target for '{}' could not be resolved by identity",
                method_name
            ),
            SelectionDiagnostic::TraitMemberMissing {
                trait_id,
                method_name,
            } => format!(
                "Trait '{}' does not declare selected member '{}'",
                display_definition(*trait_id).unwrap_or_else(|| "<unknown trait>".to_string()),
                method_name
            ),
            SelectionDiagnostic::TraitMemberIdMissing { trait_id, .. } => format!(
                "Trait '{}' does not declare the selected member",
                display_definition(*trait_id).unwrap_or_else(|| "<unknown trait>".to_string())
            ),
        }
    }

    pub fn target(&self) -> Option<&HirMethodCallTarget> {
        match self {
            SelectionDiagnostic::SelectedTargetMissing { target, .. } => Some(target),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
