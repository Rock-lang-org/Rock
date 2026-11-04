use std::collections::HashMap;

use crate::collect::DeclarationItems;
use crate::hir::{
    HirEnum, HirExtern, HirFunction, HirFunctionSig, HirImpl, HirStruct, HirTrait, HirTypeAlias,
};
use crate::ids::DefId;

/// Owns the lowered item maps so the `Lowerer` shell does not directly carry
/// every mutable HIR collection.
#[derive(Debug)]
pub(crate) struct LowerItems {
    declarations: DeclarationItems,
    impl_order: Vec<DefId>,
}

impl LowerItems {
    pub(crate) fn new() -> Self {
        Self {
            declarations: DeclarationItems::default(),
            impl_order: Vec::new(),
        }
    }

    pub(crate) fn from_declarations(
        declarations: DeclarationItems,
    ) -> Result<Self, Vec<crate::lower::ResolveError>> {
        declarations.validate()?;
        let mut impl_order: Vec<_> = declarations.impl_defs().map(|(id, _)| id).collect();
        impl_order.sort();
        Ok(Self {
            declarations,
            impl_order,
        })
    }

    pub(crate) fn functions(&self) -> impl Iterator<Item = (DefId, &HirFunction)> {
        self.declarations.function_defs()
    }

    pub(crate) fn function(&self, id: DefId) -> Option<&HirFunction> {
        self.declarations.function(id)
    }

    pub(crate) fn insert_function(&mut self, function: HirFunction) -> Option<HirFunction> {
        self.declarations.insert_function(function)
    }

    pub(crate) fn function_sigs(&self) -> impl Iterator<Item = (DefId, &HirFunctionSig)> {
        self.declarations.function_sig_defs()
    }

    pub(crate) fn impl_def(&self, id: DefId) -> Option<&HirImpl> {
        self.declarations.impl_def(id)
    }

    pub(crate) fn impl_def_mut(&mut self, id: DefId) -> Option<&mut HirImpl> {
        self.declarations.impl_def_mut(id)
    }

    pub(crate) fn impl_defs(&self) -> impl Iterator<Item = (DefId, &HirImpl)> {
        self.declarations.impl_defs()
    }

    pub(crate) fn impls_for_selection(&self) -> &HashMap<DefId, HirImpl> {
        self.declarations.impls()
    }

    pub(crate) fn impl_defs_in_order(
        &self,
    ) -> impl DoubleEndedIterator<Item = (DefId, &HirImpl)> + '_ {
        self.impl_order.iter().filter_map(|id| {
            self.declarations
                .impl_def(*id)
                .map(|impl_def| (*id, impl_def))
        })
    }

    pub(crate) fn insert_impl(
        &mut self,
        impl_def: HirImpl,
    ) -> Result<(), crate::lower::ResolveError> {
        let id = impl_def.id;
        if self.declarations.impl_def(id).is_some() {
            return Err(crate::lower::ResolveError::non_source(format!(
                "duplicate impl declaration for DefId {id:?}"
            )));
        }
        self.declarations.insert_impl(impl_def);
        let position = self
            .impl_order
            .binary_search(&id)
            .expect_err("duplicate impl IDs are rejected before ordering");
        self.impl_order.insert(position, id);
        Ok(())
    }

    pub(crate) fn structure(&self, id: DefId) -> Option<&HirStruct> {
        self.declarations.structure(id)
    }

    pub(crate) fn structures(&self) -> impl Iterator<Item = (DefId, &HirStruct)> {
        self.declarations.structure_defs()
    }

    pub(crate) fn enumeration(&self, id: DefId) -> Option<&HirEnum> {
        self.declarations.enumeration(id)
    }

    pub(crate) fn enumerations(&self) -> impl Iterator<Item = (DefId, &HirEnum)> {
        self.declarations.enumeration_defs()
    }

    pub(crate) fn type_alias(&self, id: DefId) -> Option<&HirTypeAlias> {
        self.declarations.type_alias(id)
    }

    pub(crate) fn type_aliases(&self) -> impl Iterator<Item = (DefId, &HirTypeAlias)> {
        self.declarations.type_alias_defs()
    }

    pub(crate) fn trait_def(&self, id: DefId) -> Option<&HirTrait> {
        self.declarations.trait_def(id)
    }

    pub(crate) fn trait_def_mut(&mut self, id: DefId) -> Option<&mut HirTrait> {
        self.declarations.trait_def_mut(id)
    }

    pub(crate) fn trait_defs(&self) -> impl Iterator<Item = (DefId, &HirTrait)> {
        self.declarations.trait_defs()
    }

    pub(crate) fn traits_for_selection(&self) -> &HashMap<DefId, HirTrait> {
        self.declarations.traits()
    }

    pub(crate) fn extern_def(&self, id: DefId) -> Option<&HirExtern> {
        self.declarations.extern_def(id)
    }

    pub(crate) fn externs(&self) -> impl Iterator<Item = (DefId, &HirExtern)> {
        self.declarations.extern_defs()
    }

    pub(crate) fn into_validated_declarations(
        self,
    ) -> Result<DeclarationItems, Vec<crate::lower::ResolveError>> {
        self.declarations.validate()?;
        let mut errors = Vec::new();
        if self.impl_order.len() != self.declarations.impl_defs().count()
            || self.impl_order.windows(2).any(|ids| ids[0] >= ids[1])
            || self
                .impl_order
                .iter()
                .any(|id| self.declarations.impl_def(*id).is_none())
        {
            errors.push(crate::lower::ResolveError::non_source(
                "impl order does not match impl declaration IDs".to_string(),
            ));
        }
        if errors.is_empty() {
            Ok(self.declarations)
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests;
