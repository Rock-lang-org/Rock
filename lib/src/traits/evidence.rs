//! Instantiated trait evidence shared by inference and member selection.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::hir::{HirPhase, HirTraitFor};
use crate::ids::DefId;
use crate::types::{GenericParamId, Predicate, TraitBound, Type};

/// Expand assumptions about `subject` through their instantiated predicates.
///
/// Declaration cycles must already have been rejected by trait-graph validation.
/// An unavailable declaration preserves the explicit assumption but supplies no
/// additional evidence. This query does not validate declarations or normalize
/// type arguments; those responsibilities remain with its callers.
///
/// Results follow root/predicate discovery order, independent of map iteration
/// and display names. This order is not a vtable slot or artifact ABI order.
pub fn implied_trait_bounds<P: HirPhase>(
    traits: &HashMap<DefId, HirTraitFor<P>>,
    subject: &Type,
    roots: &[TraitBound],
) -> Vec<TraitBound> {
    let mut pending = VecDeque::from(roots.to_vec());
    let mut visited = HashSet::new();
    let mut output = Vec::new();

    while let Some(bound) = pending.pop_front() {
        if !visited.insert(bound.clone()) {
            continue;
        }
        output.push(bound.clone());

        let Some(definition) = traits.get(&bound.trait_id).filter(|definition| {
            definition.id == bound.trait_id
                && definition.generic_params.len() == bound.type_args.len()
        }) else {
            continue;
        };

        let mut substitution: HashMap<_, _> = definition
            .generic_params
            .iter()
            .zip(&bound.type_args)
            .map(|(param, ty)| (param.id, ty.clone()))
            .collect();
        let target = definition
            .target
            .as_ref()
            .map(|target| target.id)
            .unwrap_or(GenericParamId {
                owner: definition.id,
                index: definition.generic_params.len() as u32,
            });
        substitution.insert(target, subject.clone());

        for predicate in &definition.predicates {
            let Predicate::Trait {
                subject: implied_subject,
                trait_id,
                args,
            } = predicate.substitute_generics(&substitution);
            if implied_subject == *subject {
                pending.push_back(TraitBound {
                    trait_id,
                    type_args: args,
                });
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use crate::hir::HirTrait;
    use crate::ids::{CrateId, LocalDefId};
    use crate::types::{GenericParamDecl, NominalTypeKind};

    use super::*;

    fn id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn generic(owner: DefId, index: u32) -> GenericParamId {
        GenericParamId { owner, index }
    }

    fn bound(trait_id: DefId, type_args: Vec<Type>) -> TraitBound {
        TraitBound {
            trait_id,
            type_args,
        }
    }

    fn declaration(trait_id: DefId, arity: u32, parents: Vec<TraitBound>) -> HirTrait {
        HirTrait {
            id: trait_id,
            name: "SameDisplayName".to_string(),
            generic_params: (0..arity)
                .map(|index| GenericParamDecl::type_param(generic(trait_id, index), "T"))
                .collect(),
            target: None,
            predicates: parents
                .into_iter()
                .map(|parent| Predicate::Trait {
                    subject: Type::Generic(generic(trait_id, arity)),
                    trait_id: parent.trait_id,
                    args: parent.type_args,
                })
                .collect(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        }
    }

    #[test]
    fn diamond_deduplicates_the_same_instantiated_ancestor() {
        let root = bound(id(1), vec![Type::I64]);
        let traits = HashMap::from([
            (
                id(1),
                declaration(
                    id(1),
                    1,
                    vec![
                        bound(id(2), vec![Type::Generic(generic(id(1), 0))]),
                        bound(id(3), vec![Type::Generic(generic(id(1), 0))]),
                    ],
                ),
            ),
            (
                id(2),
                declaration(
                    id(2),
                    1,
                    vec![bound(id(4), vec![Type::Generic(generic(id(2), 0))])],
                ),
            ),
            (
                id(3),
                declaration(
                    id(3),
                    1,
                    vec![bound(id(4), vec![Type::Generic(generic(id(3), 0))])],
                ),
            ),
            (id(4), declaration(id(4), 1, vec![])),
        ]);
        let roots = [root.clone(), root.clone()];
        assert_eq!(
            implied_trait_bounds(&traits, &Type::Bool, &roots),
            vec![
                root,
                bound(id(2), vec![Type::I64]),
                bound(id(3), vec![Type::I64]),
                bound(id(4), vec![Type::I64]),
            ]
        );
    }

    #[test]
    fn distinct_instantiations_are_not_merged_by_trait_id() {
        let root = bound(id(1), vec![]);
        let parents = vec![
            bound(id(2), vec![Type::I64]),
            bound(id(2), vec![Type::Bool]),
        ];
        let traits = HashMap::from([
            (id(1), declaration(id(1), 0, parents.clone())),
            (id(2), declaration(id(2), 1, vec![])),
        ]);
        let mut expected = vec![root.clone()];
        expected.extend(parents);
        assert_eq!(
            implied_trait_bounds(&traits, &Type::Unit, &[root]),
            expected
        );
    }

    #[test]
    fn explicit_constructor_target_is_substituted_without_capturing_method_generics() {
        let root = bound(id(1), vec![Type::I64]);
        let target = generic(id(1), 7);
        let other_owner = Type::Generic(generic(id(99), 0));
        let mut child = declaration(id(1), 1, vec![]);
        child.target = Some(GenericParamDecl::new(
            target,
            "F",
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            ),
        ));
        child.predicates.push(Predicate::Trait {
            subject: Type::Generic(target),
            trait_id: id(2),
            args: vec![Type::Generic(generic(id(1), 0)), other_owner.clone()],
        });
        let traits = HashMap::from([(id(1), child), (id(2), declaration(id(2), 2, vec![]))]);
        let constructor = Type::Constructor {
            id: id(50),
            flavor: NominalTypeKind::Enum,
        };
        assert_eq!(
            implied_trait_bounds(&traits, &constructor, std::slice::from_ref(&root)),
            vec![root, bound(id(2), vec![Type::I64, other_owner])]
        );
    }

    #[test]
    fn predicates_on_other_subjects_do_not_become_receiver_evidence() {
        let root = bound(id(1), vec![Type::I64]);
        let mut child = declaration(id(1), 1, vec![bound(id(2), vec![])]);
        child.predicates.push(Predicate::Trait {
            subject: Type::Generic(generic(id(1), 0)),
            trait_id: id(3),
            args: vec![],
        });
        let traits = HashMap::from([(id(1), child)]);
        assert_eq!(
            implied_trait_bounds(&traits, &Type::Bool, std::slice::from_ref(&root)),
            vec![root, bound(id(2), vec![])]
        );
    }

    #[test]
    fn evidence_order_does_not_depend_on_trait_map_or_display_names() {
        let root = bound(id(1), vec![]);
        let child = declaration(id(1), 0, vec![bound(id(3), vec![]), bound(id(2), vec![])]);
        let parent = declaration(id(2), 0, vec![]);
        let first = HashMap::from([(id(1), child.clone()), (id(2), parent.clone())]);
        let mut renamed_child = child;
        renamed_child.name = "Renamed".to_string();
        let second = HashMap::from([(id(2), parent), (id(1), renamed_child)]);
        let expected = vec![root.clone(), bound(id(3), vec![]), bound(id(2), vec![])];
        assert_eq!(
            implied_trait_bounds(&first, &Type::Unit, std::slice::from_ref(&root)),
            expected
        );
        assert_eq!(
            implied_trait_bounds(&second, &Type::Unit, &[root]),
            expected
        );
    }

    #[test]
    fn missing_or_mismatched_headers_do_not_invent_derived_evidence() {
        let root = bound(id(1), vec![]);
        let wrong_identity =
            HashMap::from([(id(1), declaration(id(2), 0, vec![bound(id(3), vec![])]))]);
        let wrong_arity =
            HashMap::from([(id(1), declaration(id(1), 1, vec![bound(id(3), vec![])]))]);
        for traits in [HashMap::new(), wrong_identity, wrong_arity] {
            assert_eq!(
                implied_trait_bounds(&traits, &Type::Unit, std::slice::from_ref(&root)),
                vec![root.clone()]
            );
        }
    }
}
