use super::kinds::role_order;
use super::BindingState;
use crate::ids::DefId;
use crate::language_items::{
    DropLanguageItems, FnLanguageItems, FnMutLanguageItems, FnOnceLanguageItems, FoldLanguageItems,
    IndexLanguageItems, IndexMutLanguageItems, LanguageItemRole, LanguageItems, RangeLanguageItems,
    SendLanguageItems, SizedLanguageItems, SyncLanguageItems, TryLanguageItems,
};
use crate::lower::ResolveError;

impl BindingState {
    pub(super) fn finish(mut self) -> Result<LanguageItems<DefId>, Vec<ResolveError>> {
        self.complete_sized();
        self.complete_drop();
        if let Some(root) = &self.fold.root {
            if self.fold.method.is_none() {
                self.error(
                    LanguageItemRole::Fold,
                    Some(LanguageItemRole::Method),
                    "language item fold.method is missing".to_string(),
                    root.span.clone(),
                );
            }
        }
        self.complete_index();
        self.complete_index_mut();
        self.complete_fn(LanguageItemRole::FnOnce);
        self.complete_fn(LanguageItemRole::FnMut);
        self.complete_fn(LanguageItemRole::Fn);
        self.require_index_for_index_mut();
        self.complete_try_bundle();
        self.complete_range();
        self.errors.sort_by_key(|error| {
            (
                role_order(error.protocol),
                error.role.map(role_order).unwrap_or_default(),
            )
        });
        if !self.errors.is_empty() {
            return Err(self.errors.into_iter().map(|error| error.error).collect());
        }

        let sized = self
            .sized
            .map(|root| SizedLanguageItems { trait_id: root.id });
        let drop = match (self.drop.root, self.drop.method) {
            (Some(root), Some(method)) if method.owner_id == root.id => Some(DropLanguageItems {
                trait_id: root.id,
                method_id: method.id,
            }),
            _ => None,
        };
        let fold = match (self.fold.root, self.fold.method) {
            (Some(root), Some(method)) if method.owner_id == root.id => Some(FoldLanguageItems {
                trait_id: root.id,
                method_id: method.id,
            }),
            _ => None,
        };
        let index = match (self.index.root, self.index.output, self.index.method) {
            (Some(root), Some(output), Some(method))
                if output.owner_id == root.id && method.owner_id == root.id =>
            {
                Some(IndexLanguageItems {
                    trait_id: root.id,
                    output_id: output.id,
                    method_id: method.id,
                })
            }
            _ => None,
        };
        let index_mut = match (
            self.index_mut.root,
            self.index_mut.output,
            self.index_mut.method,
        ) {
            (Some(root), Some(output), Some(method))
                if output.owner_id == root.id && method.owner_id == root.id =>
            {
                Some(IndexMutLanguageItems {
                    trait_id: root.id,
                    output_id: output.id,
                    method_id: method.id,
                })
            }
            _ => None,
        };
        let range = match (
            self.range.root,
            self.range.full_variant,
            self.range.from_variant,
            self.range.to_variant,
            self.range.to_inclusive_variant,
            self.range.exclusive_variant,
            self.range.inclusive_variant,
        ) {
            (
                Some(root),
                Some(full),
                Some(from),
                Some(to),
                Some(to_inclusive),
                Some(exclusive),
                Some(inclusive),
            ) if [
                full.owner_id,
                from.owner_id,
                to.owner_id,
                to_inclusive.owner_id,
                exclusive.owner_id,
                inclusive.owner_id,
            ]
            .iter()
            .all(|owner| *owner == root.id) =>
            {
                Some(RangeLanguageItems {
                    enum_id: root.id,
                    full_variant_id: full.id,
                    from_variant_id: from.id,
                    to_variant_id: to.id,
                    to_inclusive_variant_id: to_inclusive.id,
                    exclusive_variant_id: exclusive.id,
                    inclusive_variant_id: inclusive.id,
                })
            }
            _ => None,
        };
        let fn_once = match (self.fn_once.root, self.fn_once.output, self.fn_once.method) {
            (Some(root), Some(output), Some(method))
                if output.owner_id == root.id && method.owner_id == root.id =>
            {
                Some(FnOnceLanguageItems {
                    trait_id: root.id,
                    output_id: output.id,
                    method_id: method.id,
                })
            }
            _ => None,
        };
        let fn_mut = match (self.fn_mut.root, self.fn_mut.output, self.fn_mut.method) {
            (Some(root), Some(output), Some(method))
                if output.owner_id == root.id && method.owner_id == root.id =>
            {
                Some(FnMutLanguageItems {
                    trait_id: root.id,
                    output_id: output.id,
                    method_id: method.id,
                })
            }
            _ => None,
        };
        let fn_trait = match (
            self.fn_trait.root,
            self.fn_trait.output,
            self.fn_trait.method,
        ) {
            (Some(root), Some(output), Some(method))
                if output.owner_id == root.id && method.owner_id == root.id =>
            {
                Some(FnLanguageItems {
                    trait_id: root.id,
                    output_id: output.id,
                    method_id: method.id,
                })
            }
            _ => None,
        };
        let send = self
            .send
            .map(|root| SendLanguageItems { trait_id: root.id });
        let sync = self
            .sync
            .map(|root| SyncLanguageItems { trait_id: root.id });
        let try_protocol = match (
            self.try_protocol.root,
            self.try_protocol.output,
            self.try_protocol.residual,
            self.try_protocol.branch,
            self.from_residual.root,
            self.from_residual.method,
            self.control_flow.root,
            self.control_flow.break_variant,
            self.control_flow.continue_variant,
        ) {
            (
                Some(try_root),
                Some(output),
                Some(residual),
                Some(branch),
                Some(from_residual_root),
                Some(from_residual_method),
                Some(control_flow_root),
                Some(break_variant),
                Some(continue_variant),
            ) if output.owner_id == try_root.id
                && residual.owner_id == try_root.id
                && branch.owner_id == try_root.id
                && from_residual_method.owner_id == from_residual_root.id
                && break_variant.owner_id == control_flow_root.id
                && continue_variant.owner_id == control_flow_root.id =>
            {
                Some(TryLanguageItems {
                    try_trait_id: try_root.id,
                    output_id: output.id,
                    residual_id: residual.id,
                    branch_method_id: branch.id,
                    from_residual_trait_id: from_residual_root.id,
                    from_residual_method_id: from_residual_method.id,
                    control_flow_enum_id: control_flow_root.id,
                    break_variant_id: break_variant.id,
                    continue_variant_id: continue_variant.id,
                })
            }
            _ => None,
        };
        Ok(LanguageItems {
            sized,
            drop,
            fold,
            index,
            index_mut,
            fn_once,
            fn_mut,
            fn_trait,
            send,
            sync,
            try_protocol,
            range,
        })
    }
}
