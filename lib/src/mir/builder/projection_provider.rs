use std::collections::HashMap;

use crate::ids::{AssocTypeId, DefId};
use crate::type_services::projection::{
    ProjectionAssociatedType, ProjectionImpl, ProjectionProvider,
};
use crate::types::Type;

use super::*;

pub(super) struct MirProjectionResolutionProvider<'a> {
    pub(super) program: &'a HirProgram,
}

impl MirProjectionResolutionProvider<'_> {
    fn matching_impls(
        &self,
        base_ty: &Type,
        trait_id: DefId,
        trait_args: &[Type],
        assoc_type_id: Option<AssocTypeId>,
    ) -> Vec<(&HirImpl, HashMap<crate::types::GenericParamId, Type>)> {
        let mut matches = self
            .program
            .impls_in_order()
            .filter_map(|(_, imp)| {
                if imp.trait_id != Some(trait_id)
                    || assoc_type_id
                        .is_some_and(|id| !imp.associated_types.iter().any(|assoc| assoc.id == id))
                    || imp.trait_arg_types.len() != trait_args.len()
                {
                    return None;
                }

                let mut subst = crate::selection::receiver_pattern_substitution(
                    &imp.receiver_pattern,
                    base_ty,
                )?;
                if imp
                    .trait_arg_types
                    .iter()
                    .zip(trait_args)
                    .all(|(expected, actual)| {
                        crate::selection::type_pattern_matches(expected, actual, &mut subst)
                    })
                {
                    Some((imp, subst))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|(imp, _)| imp.id);
        matches.dedup_by_key(|(imp, _)| imp.id);
        matches
    }

    fn projection_impl(imp: &HirImpl) -> ProjectionImpl {
        ProjectionImpl {
            impl_id: imp.id,
            receiver_pattern: imp.receiver_pattern.clone(),
            trait_arg_types: imp.trait_arg_types.clone(),
            associated_types: imp
                .associated_types
                .iter()
                .map(|assoc| ProjectionAssociatedType {
                    id: assoc.id,
                    name: assoc.name.clone(),
                    ty: assoc.ty.clone(),
                })
                .collect(),
        }
    }
}

impl ProjectionProvider for MirProjectionResolutionProvider<'_> {
    fn resolve_projection_output(
        &self,
        base_ty: &Type,
        trait_id: DefId,
        assoc_type_id: AssocTypeId,
        trait_args: &[Type],
    ) -> Option<Type> {
        let mut matches = self
            .matching_impls(base_ty, trait_id, trait_args, Some(assoc_type_id))
            .into_iter()
            .filter_map(|(imp, subst)| {
                imp.associated_types
                    .iter()
                    .find(|assoc| assoc.id == assoc_type_id)
                    .map(|assoc| (imp.id, assoc.ty.substitute_generics(&subst)))
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|(impl_id, _)| *impl_id);
        matches.dedup_by_key(|(impl_id, _)| *impl_id);
        match matches.as_slice() {
            [(_, output)] => Some(output.clone()),
            _ => None,
        }
    }

    fn find_projection_impl(
        &self,
        base_ty: &Type,
        trait_id: DefId,
        trait_args: &[Type],
    ) -> Option<ProjectionImpl> {
        match self
            .matching_impls(base_ty, trait_id, trait_args, None)
            .as_slice()
        {
            [(imp, _)] => Some(Self::projection_impl(imp)),
            _ => None,
        }
    }
}
