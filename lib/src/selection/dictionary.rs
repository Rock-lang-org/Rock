//! Phase-generic declaration evidence; never selects on a runtime payload.
use std::collections::{HashMap, HashSet};

use crate::hir::{HirGenericBounds, HirImplFor, HirPhase, HirTraitFor};
use crate::ids::DefId;
use crate::types::{GenericParamId, Predicate, TraitBound, Type};

use super::{impl_receiver_pattern_substitution, type_pattern_matches, SelectionDiagnostic, SelectionService};

impl SelectionService<'_> {
    pub(crate) fn select_declared_dictionary_impl<'a, P: HirPhase + 'a>(
        traits: &HashMap<DefId, HirTraitFor<P>>, impls: &'a HashMap<DefId, HirImplFor<P>>,
        assumed: &HirGenericBounds, subject: &Type, trait_id: DefId, args: &[Type],
    ) -> Result<(&'a HirImplFor<P>, HashMap<GenericParamId, Type>), SelectionDiagnostic> {
        fn select<'a, P: HirPhase + 'a>(traits: &HashMap<DefId, HirTraitFor<P>>, impls: &'a HashMap<DefId, HirImplFor<P>>, assumed: &HirGenericBounds,
            subject: &Type, bound: &TraitBound, active: &mut HashSet<(Type, TraitBound)>,
        ) -> Result<(&'a HirImplFor<P>, HashMap<GenericParamId, Type>), SelectionDiagnostic> {
            let error = || SelectionDiagnostic::NoImplementation { operation: "declaration-bound dictionary".into(), receiver: subject.clone() };
            if traits.get(&bound.trait_id).is_none_or(|declaration| declaration.generic_params.len() != bound.type_args.len()) { return Err(error()); }
            if crate::type_services::visit::type_any(subject, |ty| matches!(ty, Type::TypeVar(_) | Type::Witness(_)))
                || bound.type_args.iter().any(|arg| crate::type_services::visit::type_any(arg, |ty| matches!(ty, Type::TypeVar(_) | Type::Witness(_))))
                || active.len() >= 128 || !active.insert((subject.clone(), bound.clone())) { return Err(error()); }
            let mut candidates = Vec::new();
            for implementation in impls.values() {
                if implementation.trait_id != Some(bound.trait_id) || implementation.trait_arg_types.len() != bound.type_args.len() { continue; }
                // Identical to strict trait-obligation selection: method
                // autoderef/autoref must not manufacture trait evidence.
                if matches!(P::impl_receiver_pattern(&implementation.receiver_pattern), Some(crate::hir::HirImplReceiverPattern::Exact(pattern))
                    if !type_pattern_matches(pattern, subject, &mut HashMap::new())) { continue; }
                let Some(mut substitution) = impl_receiver_pattern_substitution(implementation, subject, impls.values()) else { continue; };
                if !implementation.trait_arg_types.iter().zip(&bound.type_args).all(|(a,b)| type_pattern_matches(a,b,&mut substitution))
                    || implementation.type_generics.iter().any(|p| !substitution.contains_key(&p.id)) { continue; }
                let obligations = if implementation.bounds.predicates.is_empty() {
                    implementation.bounds.iter().flat_map(|(param, roots)| roots.iter().map(|root| Predicate::Trait {
                        subject: Type::Generic(*param), trait_id: root.trait_id, args: root.type_args.clone(),
                    })).collect::<Vec<_>>()
                } else { implementation.bounds.predicates.clone() };
                let checked = obligations.iter().all(|predicate| {
                    let Predicate::Trait { subject, trait_id, args } = predicate.substitute_generics(&substitution);
                    let required = TraitBound { trait_id, type_args: args };
                    let declared = if let Type::Generic(param) = &subject {
                        assumed.get(param).is_some_and(|roots| crate::traits::evidence::implied_trait_bounds(traits, &subject, roots).contains(&required))
                    } else { false };
                    declared || select(traits, impls, assumed, &subject, &required, active).is_ok()
                });
                if checked { candidates.push((implementation, substitution)); }
            }
            active.remove(&(subject.clone(), bound.clone()));
            candidates.sort_by_key(|(implementation, _)| implementation.id);
            match candidates.len() {
                1 => Ok(candidates.remove(0)),
                0 => Err(error()),
                _ => Err(SelectionDiagnostic::AmbiguousCandidates { operation: "declaration-bound dictionary".into(), receiver: subject.clone(), candidates: candidates.iter().map(|(imp,_)| imp.id).collect() }),
            }
        }
        select(traits, impls, assumed, subject, &TraitBound { trait_id, type_args: args.to_vec() }, &mut HashSet::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{AcceptedHir, HirImplOwner, HirImplReceiverPattern};
    use crate::ids::{CrateId, LocalDefId};
    use crate::types::GenericParamDecl;

    fn id(n: u32) -> DefId { DefId::new(CrateId(0), LocalDefId(n)) }
    fn fixture() -> (HashMap<DefId, HirTraitFor<AcceptedHir>>, HashMap<DefId, HirImplFor<AcceptedHir>>, HirGenericBounds, Type) {
        let traits = [id(10), id(20)].into_iter().map(|id| (id, HirTraitFor {
            id, name: "display_only".into(), generic_params: vec![], target: Some(GenericParamDecl::type_param(GenericParamId { owner: id, index: 0 }, "Self")),
            predicates: vec![], associated_types: vec![], methods: HashMap::new(), signatures: HashMap::new(),
        })).collect();
        let parameter = GenericParamId { owner: id(30), index: 0 };
        let caller = GenericParamId { owner: id(40), index: 0 };
        let implementation = HirImplFor {
            id: id(30), owner: HirImplOwner::Named("not_authority".into()), type_name: "not_authority".into(),
            type_generics: vec![GenericParamDecl::type_param(parameter, "T")], receiver_pattern: HirImplReceiverPattern::Exact(Type::Generic(parameter)),
            trait_name: Some("not_authority".into()), trait_id: Some(id(20)), trait_generics: vec![], trait_arg_types: vec![], associated_types: vec![],
            bounds: HirGenericBounds::from_iter([(parameter, vec![TraitBound { trait_id: id(10), type_args: vec![] }])]), methods: HashMap::new(),
        };
        let assumed = HirGenericBounds::from_iter([(caller, vec![TraitBound { trait_id: id(10), type_args: vec![] }])]);
        (traits, HashMap::from([(id(30), implementation)]), assumed, Type::Generic(caller))
    }

    #[test]
    fn accepted_phase_selection_proves_impl_bounds_from_only_declared_read() {
        let (traits, impls, assumed, subject) = fixture();
        let (implementation, substitution) = SelectionService::select_declared_dictionary_impl(&traits, &impls, &assumed, &subject, id(20), &[]).unwrap();
        assert_eq!(implementation.id, id(30));
        assert_eq!(substitution[&implementation.type_generics[0].id], subject);
        assert!(SelectionService::select_declared_dictionary_impl(&traits, &impls, &HirGenericBounds::new(), &subject, id(20), &[]).is_err());
    }

    #[test]
    fn overlap_and_recursive_obligations_never_pick_the_first_impl() {
        let (traits, mut impls, assumed, subject) = fixture();
        let mut duplicate = impls[&id(30)].clone(); duplicate.id = id(32); impls.insert(duplicate.id, duplicate);
        assert!(matches!(SelectionService::select_declared_dictionary_impl(&traits, &impls, &assumed, &subject, id(20), &[]), Err(SelectionDiagnostic::AmbiguousCandidates { .. })));
        impls.remove(&id(32));
        let implementation = impls.get_mut(&id(30)).unwrap();
        implementation.bounds = HirGenericBounds::from_iter([(implementation.type_generics[0].id, vec![TraitBound { trait_id: id(20), type_args: vec![] }])]);
        assert!(SelectionService::select_declared_dictionary_impl(&traits, &impls, &assumed, &subject, id(20), &[]).is_err());
    }

    #[test]
    fn trait_evidence_does_not_apply_method_receiver_autoref_or_autoderef() {
        let (traits, mut impls, assumed, _) = fixture();
        let implementation = impls.get_mut(&id(30)).unwrap();
        implementation.type_generics.clear(); implementation.bounds = HirGenericBounds::new();
        implementation.receiver_pattern = HirImplReceiverPattern::Exact(Type::Reference { inner: Box::new(Type::I64), mutable: true });
        for subject in [Type::I64, Type::Reference { inner: Box::new(Type::I64), mutable: false }] {
            assert!(SelectionService::select_declared_dictionary_impl(&traits, &impls, &assumed, &subject, id(20), &[]).is_err());
        }
    }
}
