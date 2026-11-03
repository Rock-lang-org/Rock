use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoherenceError {
    pub impl_id: DefId,
    pub kind: CoherenceErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoherenceErrorKind {
    InvalidTarget {
        trait_id: DefId,
        target: HirImplReceiverPattern,
        error: NormalizeError,
    },
    TargetKindMismatch {
        trait_id: DefId,
        expected: Kind,
        actual: Kind,
    },
    TargetCategoryMismatch {
        trait_id: DefId,
        expected: Kind,
    },
    InvalidConstructorTarget {
        trait_id: DefId,
        target: Type,
        reason: String,
    },
    Orphan {
        trait_id: DefId,
        target: CanonicalTarget,
    },
    InvalidTraitArguments {
        trait_id: DefId,
        error: NormalizeError,
    },
    Overlapping {
        left_impl_id: DefId,
        right_impl_id: DefId,
        trait_id: DefId,
        left_target: CanonicalTarget,
        right_target: CanonicalTarget,
        witness: Vec<CoherenceWitnessBinding>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoherenceWitnessBinding {
    pub parameter: GenericParamId,
    pub value: Type,
}

impl CoherenceError {
    pub(super) fn render_with(
        &self,
        display_type: impl Fn(&Type) -> String,
        display_name: impl Fn(DefId) -> String,
    ) -> String {
        let trait_name = |id| display_name(id);
        match &self.kind {
            CoherenceErrorKind::InvalidTarget {
                trait_id,
                target,
                error,
            } => format!(
                "invalid impl target for trait `{}`: `{}` cannot be normalized ({})",
                trait_name(*trait_id),
                render_receiver_pattern(target, &display_type),
                render_normalize_error(error, &display_type),
            ),
            CoherenceErrorKind::TargetKindMismatch {
                trait_id,
                expected,
                actual,
            } => format!(
                "constructor impl target for trait `{}` has kind {actual}, but the trait requires {expected}",
                trait_name(*trait_id),
            ),
            CoherenceErrorKind::TargetCategoryMismatch {
                trait_id,
                expected,
            } => format!(
                "impl target for trait `{}` does not match the required target kind `{expected}`",
                trait_name(*trait_id),
            ),
            CoherenceErrorKind::InvalidConstructorTarget {
                trait_id,
                target,
                reason,
            } => format!(
                "invalid constructor impl target `{}` for trait `{}`: {reason}",
                display_type(target),
                trait_name(*trait_id),
            ),
            CoherenceErrorKind::Orphan { trait_id, target } => format!(
                "orphan impl for trait `{}` is illegal: trait and normalized target `{}` are both foreign",
                trait_name(*trait_id),
                render_target(target, &display_type),
            ),
            CoherenceErrorKind::InvalidTraitArguments { trait_id, error } => format!(
                "invalid trait arguments for trait `{}`: {}",
                trait_name(*trait_id),
                render_normalize_error(error, &display_type),
            ),
            CoherenceErrorKind::Overlapping {
                trait_id,
                left_target,
                right_target,
                witness,
                ..
            } => format!(
                "overlapping impls for trait `{}`: normalized targets `{}` and `{}`; witness {}",
                trait_name(*trait_id),
                render_target(left_target, &display_type),
                render_target(right_target, &display_type),
                render_witness(witness, &display_type),
            ),
        }
    }

    pub(super) fn render_toolchain(&self) -> String {
        format!(
            "invalid coherence metadata in an external implementation: {}",
            self.neutral_reason()
        )
    }

    fn neutral_reason(&self) -> &'static str {
        match &self.kind {
            CoherenceErrorKind::InvalidTarget { .. } => "invalid target",
            CoherenceErrorKind::TargetKindMismatch { .. } => "target kind mismatch",
            CoherenceErrorKind::TargetCategoryMismatch { .. } => "target category mismatch",
            CoherenceErrorKind::InvalidConstructorTarget { .. } => "invalid constructor target",
            CoherenceErrorKind::Orphan { .. } => "orphan implementation",
            CoherenceErrorKind::InvalidTraitArguments { .. } => "invalid trait arguments",
            CoherenceErrorKind::Overlapping { .. } => "overlapping implementations",
        }
    }

    pub(super) fn current_impl_id(&self, root_crate_id: crate::ids::CrateId) -> Option<DefId> {
        if self.impl_id.crate_id == root_crate_id {
            return Some(self.impl_id);
        }
        match &self.kind {
            CoherenceErrorKind::Overlapping {
                left_impl_id,
                right_impl_id,
                ..
            } => [*left_impl_id, *right_impl_id]
                .into_iter()
                .find(|id| id.crate_id == root_crate_id),
            _ => None,
        }
    }

    pub(super) fn overlapping_other_impl_id(&self, primary_impl_id: DefId) -> Option<DefId> {
        match &self.kind {
            CoherenceErrorKind::Overlapping {
                left_impl_id,
                right_impl_id,
                ..
            } if *left_impl_id == primary_impl_id => Some(*right_impl_id),
            CoherenceErrorKind::Overlapping {
                left_impl_id,
                right_impl_id,
                ..
            } if *right_impl_id == primary_impl_id => Some(*left_impl_id),
            _ => None,
        }
    }
}
