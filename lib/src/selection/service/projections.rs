//! Associated-type projection lookup through impl selection.

use crate::ids::DefId;
use crate::type_services::projection::{
    ProjectionAssociatedType, ProjectionImpl, ProjectionProvider,
};
use crate::types::Type;

use super::SelectionService;

impl ProjectionProvider for SelectionService<'_> {
    fn find_projection_impl(
        &self,
        base_ty: &Type,
        trait_id: DefId,
        trait_args: &[Type],
    ) -> Option<ProjectionImpl> {
        self.select_trait_impl_strict(base_ty, trait_id, trait_args)
            .ok()
            .map(|imp| ProjectionImpl {
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
            })
    }
}
