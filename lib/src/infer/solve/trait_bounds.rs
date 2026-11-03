use std::collections::HashMap;

use crate::ids::DefId;
use crate::types::{TraitBound, Type};

pub(super) fn trait_bounds_with_supertraits(
    subject: &Type,
    initial: &TraitBound,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
) -> Vec<TraitBound> {
    fn expand(
        subject: &Type,
        bound: TraitBound,
        traits: &HashMap<DefId, crate::hir::HirTrait>,
        visiting: &mut std::collections::HashSet<crate::types::Predicate>,
        output: &mut Vec<TraitBound>,
    ) {
        let key = crate::types::Predicate::Trait {
            subject: subject.clone(),
            trait_id: bound.trait_id,
            args: bound.type_args.clone(),
        };
        if !visiting.insert(key.clone()) {
            return;
        }
        if !output.contains(&bound) {
            output.push(bound.clone());
        }
        if let Some(trait_def) = traits.get(&bound.trait_id) {
            let mut subst = crate::selection::generic_substitution_for_owner(
                &trait_def.generic_params,
                &bound.type_args,
            );
            let target_id = trait_def.target.as_ref().map(|target| target.id).unwrap_or(
                crate::types::GenericParamId {
                    owner: trait_def.id,
                    index: trait_def.generic_params.len() as u32,
                },
            );
            subst.insert(target_id, subject.clone());
            for predicate in &trait_def.predicates {
                let crate::types::Predicate::Trait {
                    subject: implied_subject,
                    trait_id,
                    args,
                } = predicate.substitute_generics(&subst);
                if implied_subject == *subject {
                    expand(
                        subject,
                        TraitBound {
                            trait_id,
                            type_args: args,
                        },
                        traits,
                        visiting,
                        output,
                    );
                }
            }
        }
        visiting.remove(&key);
    }

    let mut output = Vec::new();
    expand(
        subject,
        initial.clone(),
        traits,
        &mut std::collections::HashSet::new(),
        &mut output,
    );
    output
}
