//! The trait/impl item whose declarations are currently being lowered.
//!
//! Callers enter a scope with an `enter_*` operation and hand the returned frame back to
//! [`ItemScope::leave`], so nested trait/impl lowering restores exactly what it replaced.

use crate::ids::DefId;
use crate::lower::body_context::GenericLoweringContext;

#[derive(Default)]
pub(crate) struct ItemScope {
    trait_name: Option<String>,
    trait_id: Option<DefId>,
    generics: Option<GenericLoweringContext>,
    struct_impl: Option<String>,
}

/// Saved scope state returned by an `enter_*` operation.
#[must_use = "an entered item scope must be restored with ItemScope::leave"]
pub(crate) struct ItemFrame {
    trait_name: Option<String>,
    trait_id: Option<DefId>,
    generics: Option<GenericLoweringContext>,
    struct_impl: Option<String>,
}

impl ItemScope {
    /// Enters a trait: sets its name/id so method lookups on `Self` see sibling methods,
    /// and replaces the generic context with `params` owned by `generic_owner`.
    pub(crate) fn enter_trait(
        &mut self,
        name: String,
        id: DefId,
        generic_owner: DefId,
        params: Vec<String>,
    ) -> ItemFrame {
        let frame = self.frame();
        self.trait_name = Some(name);
        self.trait_id = Some(id);
        self.generics = Some(GenericLoweringContext::new(generic_owner, params));
        frame
    }

    /// Replaces only the generic context; trait and impl identity stay as they were.
    pub(crate) fn enter_generics(&mut self, owner: DefId, params: Vec<String>) -> ItemFrame {
        let frame = self.frame();
        self.generics = Some(GenericLoweringContext::new(owner, params));
        frame
    }

    /// Sets the struct whose private fields the lowered impl body may access.
    pub(crate) fn enter_struct_impl(&mut self, struct_impl: Option<String>) -> ItemFrame {
        let frame = self.frame();
        self.struct_impl = struct_impl;
        frame
    }

    pub(crate) fn leave(&mut self, frame: ItemFrame) {
        self.trait_name = frame.trait_name;
        self.trait_id = frame.trait_id;
        self.generics = frame.generics;
        self.struct_impl = frame.struct_impl;
    }

    pub(crate) fn trait_name(&self) -> Option<&str> {
        self.trait_name.as_deref()
    }

    pub(crate) fn trait_id(&self) -> Option<DefId> {
        self.trait_id
    }

    pub(crate) fn struct_impl(&self) -> Option<&str> {
        self.struct_impl.as_deref()
    }

    pub(crate) fn generics(&self) -> Option<&GenericLoweringContext> {
        self.generics.as_ref()
    }

    pub(crate) fn generics_mut(&mut self) -> Option<&mut GenericLoweringContext> {
        self.generics.as_mut()
    }

    fn frame(&self) -> ItemFrame {
        ItemFrame {
            trait_name: self.trait_name.clone(),
            trait_id: self.trait_id,
            generics: self.generics.clone(),
            struct_impl: self.struct_impl.clone(),
        }
    }
}
