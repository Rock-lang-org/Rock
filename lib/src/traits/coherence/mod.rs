use std::collections::{HashMap, HashSet};

use crate::hir::{HirEnum, HirImpl, HirImplReceiverPattern, HirStruct, HirTrait, HirTypeAlias};
use crate::ids::DefId;
use crate::lower::Lowerer;
use crate::type_services::kind::Kind;
use crate::type_services::normalize::{NormalizeError, TypeNormalizationEnv, TypeNormalizer};
use crate::types::{ArrayLen, AssociatedTypeKey, GenericParamId, NominalTypeKind, Type};

mod canonical_targets;
mod errors;
mod rendering;
mod unifier;
mod validation;

pub use canonical_targets::CanonicalTarget;
use canonical_targets::*;
pub use errors::{CoherenceError, CoherenceErrorKind, CoherenceWitnessBinding};
use rendering::*;
use unifier::*;
pub use validation::validate_coherence;
use validation::*;

pub struct CoherencePhase;

impl CoherencePhase {
    pub(crate) fn run(lowerer: &mut Lowerer) {
        let errors = validate_coherence_with_ids(
            lowerer.items.trait_defs().map(|(_, trait_def)| trait_def),
            lowerer.items.impl_defs().map(|(_, imp)| imp),
            lowerer.items.structures().map(|(_, structure)| structure),
            lowerer
                .items
                .enumerations()
                .map(|(_, enumeration)| enumeration),
            lowerer.items.type_aliases().map(|(_, alias)| alias),
            lowerer
                .language_items
                .sized
                .as_ref()
                .map(|items| items.trait_id),
        );
        for error in errors {
            let Some(primary_impl_id) = error.current_impl_id(lowerer.root_crate_id) else {
                lowerer
                    .diagnostics
                    .push_toolchain_once(error.render_toolchain());
                continue;
            };
            let span = lowerer
                .source_map
                .definition_span(primary_impl_id)
                .cloned()
                .expect("current-crate coherence error requires its impl source span");
            let message = error.render_with(
                |ty| lowerer.display_type(ty),
                |id| {
                    lowerer
                        .canonical_name_for_def_id(id)
                        .unwrap_or("<unknown item>")
                        .to_string()
                },
            );
            let labels = error
                .overlapping_other_impl_id(primary_impl_id)
                .filter(|id| id.crate_id == lowerer.root_crate_id)
                .and_then(|id| lowerer.source_map.definition_span(id).cloned())
                .into_iter()
                .map(|span| {
                    crate::diagnostic::DiagnosticLabel::new("overlapping implementation", span)
                })
                .collect();
            lowerer
                .diagnostics
                .push_with_span_and_labels(message, span, labels);
        }
    }
}

#[cfg(test)]
mod tests;
