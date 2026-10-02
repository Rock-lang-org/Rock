use std::collections::BTreeSet;

use crate::ids::{AssocTypeId, DefId, TypeVarId};
use crate::type_services::normalize::TypeNormalizer;
use crate::types::{ObjectType, Type};

use super::{InferenceEngine, UnifyError};

#[cfg(test)]
#[path = "object_unify_tests.rs"]
mod tests;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Row {
    trait_id: DefId,
    member: Option<AssocTypeId>,
    terms: Vec<Type>,
}

fn rows(object: &ObjectType) -> Vec<Row> {
    std::iter::once(&object.principal)
        .chain(&object.guarantees)
        .map(|bound| Row {
            trait_id: bound.trait_id,
            member: None,
            terms: bound.type_args.clone(),
        })
        .chain(object.bindings.iter().map(|binding| {
            let mut terms = binding.key.trait_ref.type_args.clone();
            terms.push(binding.ty.clone());
            Row {
                trait_id: binding.key.trait_ref.trait_id,
                member: Some(binding.key.member),
                terms,
            }
        }))
        .collect()
}

struct Obligation {
    row: Row,
    candidates: Vec<Row>,
}
struct Solution {
    substitutions: Vec<Type>,
    engine: InferenceEngine,
}

fn normalized(engine: &InferenceEngine, ty: &Type) -> Result<Type, UnifyError> {
    let ty = engine.resolve(ty);
    TypeNormalizer::new(&engine.normalization_env())
        .normalize(&ty)
        .map_err(|detail| UnifyError::Normalization { ty, detail })
}

struct Search<'a> {
    obligations: Vec<Obligation>,
    left: &'a Type,
    right: &'a Type,
    variables: Vec<TypeVarId>,
    solutions: Vec<Solution>,
    remaining: usize,
    deferred: bool,
}

impl Search<'_> {
    fn solve(&mut self, engine: InferenceEngine) -> Result<(), UnifyError> {
        let mut pending = vec![(engine, 0)];
        while let Some((engine, index)) = pending.pop() {
            if self.solutions.len() >= 2 {
                break;
            }
            if self.remaining == 0 {
                return Err(UnifyError::ObjectMatchLimit {
                    left: self.left.clone(),
                    right: self.right.clone(),
                });
            }
            self.remaining -= 1;
            let Some(obligation) = self.obligations.get(index) else {
                if normalized(&engine, self.left)? != normalized(&engine, self.right)? {
                    continue;
                }
                let substitutions = self
                    .variables
                    .iter()
                    .map(|id| normalized(&engine, &Type::TypeVar(*id)))
                    .collect::<Result<Vec<_>, _>>()?;
                if self
                    .solutions
                    .iter()
                    .all(|solution| solution.substitutions != substitutions)
                {
                    self.solutions.push(Solution {
                        substitutions,
                        engine,
                    });
                }
                continue;
            };
            let row = obligation.row.clone();
            let candidates = obligation
                .candidates
                .iter()
                .map(|candidate| {
                    Ok(Row {
                        trait_id: candidate.trait_id,
                        member: candidate.member,
                        terms: candidate
                            .terms
                            .iter()
                            .map(|ty| normalized(&engine, ty))
                            .collect::<Result<_, _>>()?,
                    })
                })
                .collect::<Result<BTreeSet<_>, UnifyError>>()?;
            if candidates.len().saturating_add(pending.len()) > self.remaining {
                return Err(UnifyError::ObjectMatchLimit {
                    left: self.left.clone(),
                    right: self.right.clone(),
                });
            }
            for candidate in candidates {
                let mut probe = engine.clone_for_probe();
                let mut matched = true;
                for (left, right) in row.terms.iter().zip(&candidate.terms) {
                    if let Err(error) = probe.unify_invariant(left, right) {
                        match error {
                            error @ UnifyError::ObjectMatchLimit { .. } => return Err(error),
                            UnifyError::ObjectMatchPending { .. }
                            | UnifyError::ConstructorInference { .. }
                            | UnifyError::AmbiguousConstructorHeads { .. } => self.deferred = true,
                            _ => {}
                        }
                        matched = false;
                        break;
                    }
                }
                if matched {
                    pending.push((probe, index + 1));
                }
            }
        }
        Ok(())
    }
}

/// Match sets of guarantees, not a positional list or a multiset. The reverse
/// obligations prevent dropping guarantees; reuse permits equivalent rows to
/// collapse after inference. Branch substitutions are isolated in probes.
pub(super) fn unify_objects(
    engine: &mut InferenceEngine,
    left: &ObjectType,
    right: &ObjectType,
) -> Result<(), UnifyError> {
    let left_ty = Type::Object(Box::new(left.clone()));
    let right_ty = Type::Object(Box::new(right.clone()));
    let mismatch = || UnifyError::Mismatch {
        expected: left_ty.clone(),
        found: right_ty.clone(),
    };
    if left.principal.trait_id != right.principal.trait_id
        || left.principal.type_args.len() != right.principal.type_args.len()
    {
        return Err(mismatch());
    }
    let mut principal = engine.clone_for_probe();
    for (left, right) in left
        .principal
        .type_args
        .iter()
        .zip(&right.principal.type_args)
    {
        principal.unify_invariant(left, right)?;
    }
    let left_rows = rows(left);
    let right_rows = rows(right);
    let mut obligations = Vec::new();
    for (source, target) in [(&left_rows, &right_rows), (&right_rows, &left_rows)] {
        for row in source {
            let candidates = target
                .iter()
                .filter(|candidate| {
                    candidate.trait_id == row.trait_id
                        && candidate.member == row.member
                        && candidate.terms.len() == row.terms.len()
                })
                .cloned()
                .collect::<Vec<_>>();
            if candidates.is_empty() {
                return Err(mismatch());
            }
            obligations.push(Obligation {
                row: row.clone(),
                candidates,
            });
        }
    }
    obligations.sort_by_key(|obligation| obligation.candidates.len());
    let mut variables = BTreeSet::new();
    for ty in [&left_ty, &right_ty] {
        crate::type_services::visit::visit_type(ty, &mut |ty: &Type| {
            if let Type::TypeVar(id) = ty {
                variables.insert(*id);
            }
        });
    }
    let mut search = Search {
        obligations,
        left: &left_ty,
        right: &right_ty,
        variables: variables.into_iter().collect(),
        solutions: Vec::new(),
        remaining: 4096,
        deferred: false,
    };
    search.solve(principal.clone_for_probe())?;
    match search.solutions.len() {
        0 if !search.deferred => Err(mismatch()),
        1 if !search.deferred => {
            *engine = search.solutions.pop().expect("one solution").engine;
            Ok(())
        }
        _ => {
            // Only principal equations are common to every branch. Their
            // progress can wake other constraints without guessing a match.
            *engine = principal;
            Err(UnifyError::ObjectMatchPending {
                left: left_ty,
                right: right_ty,
            })
        }
    }
}
