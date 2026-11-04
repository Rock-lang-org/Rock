use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::hir::{
    function_has_receiver, HirAssociatedTypeDef, HirExpr, HirFunction, HirGenericBounds, HirImpl,
    HirImplReceiverPattern, HirMethodCallTarget, HirSelectedTraitMember, HirTrait,
    HirTraitDispatchKind, HirTypeBinding,
};
use crate::ids::{AssocTypeId, DefId};
use crate::selection::matching::{
    constructor_target_from_applied_type, constructor_target_substitution,
    impl_receiver_pattern_substitution, type_pattern_matches,
};
use crate::selection::types::{
    ReceiverAdjustment, ReceiverCandidate, SelectedConstructorMember, SelectedMethod,
    SelectedOrigin, SelectionDiagnostic,
};
use crate::types::{GenericParamId, TraitBound, Type};

mod bound_methods;
mod bounds;
mod concrete_methods;
mod impl_matching;
mod instantiation;
mod self_adjustment;
mod trait_selection;
mod unresolved_methods;

pub struct SelectionService<'a> {
    traits: &'a HashMap<DefId, HirTrait>,
    impls: &'a HashMap<DefId, HirImpl>,
    impl_ids: Vec<DefId>,
    sized_trait_id: Option<DefId>,
    current_trait: Option<DefId>,
    current_impl_bounds: &'a HirGenericBounds,
    effective_trait_methods: &'a HashMap<(DefId, DefId), DefId>,
    active_obligations: RefCell<HashSet<(Type, TraitBound)>>,
}

struct EffectiveTraitMethod {
    function: HirFunction,
    inherited_default: bool,
}

fn empty_effective_trait_methods() -> &'static HashMap<(DefId, DefId), DefId> {
    static EMPTY: OnceLock<HashMap<(DefId, DefId), DefId>> = OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

impl<'a> SelectionService<'a> {
    pub fn new(
        traits: &'a HashMap<DefId, HirTrait>,
        impls: &'a HashMap<DefId, HirImpl>,
        sized_trait_id: Option<DefId>,
        current_trait: Option<DefId>,
        current_impl_bounds: &'a HirGenericBounds,
    ) -> Self {
        let mut impl_ids: Vec<_> = impls.keys().copied().collect();
        impl_ids.sort();
        Self {
            traits,
            impls,
            impl_ids,
            sized_trait_id,
            current_trait,
            current_impl_bounds,
            effective_trait_methods: empty_effective_trait_methods(),
            active_obligations: RefCell::new(HashSet::new()),
        }
    }

    pub fn with_effective_trait_methods(
        mut self,
        effective_trait_methods: &'a HashMap<(DefId, DefId), DefId>,
    ) -> Self {
        self.effective_trait_methods = effective_trait_methods;
        self
    }

    fn impl_ids_in_order(&self) -> impl Iterator<Item = DefId> + '_ {
        self.impl_ids.iter().copied()
    }

    pub fn trait_by_id(&self, id: DefId) -> Option<&HirTrait> {
        self.traits.get(&id)
    }

    pub fn trait_def_for_member(&self, trait_id: DefId, method_name: &str) -> Option<&HirTrait> {
        let trait_def = self.traits.get(&trait_id)?;
        (Self::trait_member_id_score(trait_def, method_name) > 0).then_some(trait_def)
    }

    pub fn trait_member_id(&self, trait_id: DefId, method_name: &str) -> Option<DefId> {
        let trait_def = self.trait_def_for_member(trait_id, method_name)?;
        trait_def
            .methods
            .get(method_name)
            .map(|method| method.id)
            .or_else(|| trait_def.signatures.get(method_name).map(|sig| sig.id))
    }

    pub(super) fn trait_member_name_by_id(
        &self,
        trait_id: DefId,
        member_id: DefId,
    ) -> Option<&str> {
        let trait_def = self.traits.get(&trait_id)?;
        trait_def
            .methods
            .iter()
            .find_map(|(name, method)| (method.id == member_id).then_some(name.as_str()))
            .or_else(|| {
                trait_def.signatures.iter().find_map(|(name, signature)| {
                    (signature.id == member_id).then_some(name.as_str())
                })
            })
    }

    fn trait_member_id_score(trait_def: &HirTrait, method_name: &str) -> usize {
        let method_score = trait_def
            .methods
            .get(method_name)
            .map(|method| Self::def_id_score(method.id))
            .unwrap_or(0);
        let signature_score = trait_def
            .signatures
            .get(method_name)
            .map(|sig| Self::def_id_score(sig.id))
            .unwrap_or(0);

        method_score.max(signature_score)
    }

    fn def_id_score(id: DefId) -> usize {
        if id.crate_id.0 == u32::MAX {
            1
        } else {
            2
        }
    }
}

fn inferred_rigid_receiver_type(pattern: &HirImplReceiverPattern) -> Option<Type> {
    let ty = match pattern {
        HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => ty,
        HirImplReceiverPattern::SliceFamily { element } => {
            return Some(Type::Reference {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(element.clone()))),
            });
        }
    };

    if !matches!(
        ty,
        Type::Struct { .. }
            | Type::Enum { .. }
            | Type::Reference { .. }
            | Type::Slice(_)
            | Type::Array(_, _)
            | Type::Pointer(_)
            | Type::Tuple(_)
    ) {
        return None;
    }

    (!crate::type_services::visit::type_any(ty, |nested| {
        matches!(
            nested,
            Type::TypeVar(_)
                | Type::Generic(_)
                | Type::Projection { .. }
                | Type::Apply { .. }
                | Type::Constructor { .. }
                | Type::Lambda { .. }
                | Type::BoundVar { .. }
                | Type::Error
        )
    }))
    .then(|| ty.clone())
}

#[cfg(test)]
mod tests;
