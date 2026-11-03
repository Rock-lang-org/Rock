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

    #[allow(dead_code)]
    pub(crate) fn functions(&self) -> impl Iterator<Item = (DefId, &HirFunction)> {
        self.declarations.function_defs()
    }

    pub(crate) fn function(&self, id: DefId) -> Option<&HirFunction> {
        self.declarations.function(id)
    }

    #[allow(dead_code)]
    pub(crate) fn function_mut(&mut self, id: DefId) -> Option<&mut HirFunction> {
        self.declarations.function_mut(id)
    }

    pub(crate) fn insert_function(&mut self, function: HirFunction) -> Option<HirFunction> {
        self.declarations.insert_function(function)
    }

    #[allow(dead_code)]
    pub(crate) fn remove_function(&mut self, id: DefId) -> Option<HirFunction> {
        self.declarations.remove_function(id)
    }

    #[allow(dead_code)]
    pub(crate) fn function_sigs(&self) -> impl Iterator<Item = (DefId, &HirFunctionSig)> {
        self.declarations.function_sig_defs()
    }

    pub(crate) fn function_sig(&self, id: DefId) -> Option<&HirFunctionSig> {
        self.declarations.function_sig(id)
    }

    #[allow(dead_code)]
    pub(crate) fn function_sig_mut(&mut self, id: DefId) -> Option<&mut HirFunctionSig> {
        self.declarations.function_sig_mut(id)
    }

    pub(crate) fn insert_function_sig(
        &mut self,
        signature: HirFunctionSig,
    ) -> Option<HirFunctionSig> {
        self.declarations.insert_function_sig(signature)
    }

    pub(crate) fn remove_function_sig(&mut self, id: DefId) -> Option<HirFunctionSig> {
        self.declarations.remove_function_sig(id)
    }

    pub(crate) fn impl_def(&self, id: DefId) -> Option<&HirImpl> {
        self.declarations.impl_def(id)
    }

    pub(crate) fn impl_def_mut(&mut self, id: DefId) -> Option<&mut HirImpl> {
        self.declarations.impl_def_mut(id)
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub(crate) fn remove_impl(&mut self, id: DefId) -> Option<HirImpl> {
        self.impl_order.retain(|stored_id| *stored_id != id);
        self.declarations.remove_impl(id)
    }

    #[allow(dead_code)]
    pub(crate) fn structure(&self, id: DefId) -> Option<&HirStruct> {
        self.declarations.structure(id)
    }

    #[allow(dead_code)]
    pub(crate) fn structure_mut(&mut self, id: DefId) -> Option<&mut HirStruct> {
        self.declarations.structure_mut(id)
    }

    pub(crate) fn structures(&self) -> impl Iterator<Item = (DefId, &HirStruct)> {
        self.declarations.structure_defs()
    }

    pub(crate) fn insert_structure(&mut self, structure: HirStruct) -> Option<HirStruct> {
        self.declarations.insert_structure(structure)
    }

    #[allow(dead_code)]
    pub(crate) fn remove_structure(&mut self, id: DefId) -> Option<HirStruct> {
        self.declarations.remove_structure(id)
    }

    #[allow(dead_code)]
    pub(crate) fn enumeration(&self, id: DefId) -> Option<&HirEnum> {
        self.declarations.enumeration(id)
    }

    #[allow(dead_code)]
    pub(crate) fn enumeration_mut(&mut self, id: DefId) -> Option<&mut HirEnum> {
        self.declarations.enumeration_mut(id)
    }

    pub(crate) fn enumerations(&self) -> impl Iterator<Item = (DefId, &HirEnum)> {
        self.declarations.enumeration_defs()
    }

    pub(crate) fn insert_enumeration(&mut self, enumeration: HirEnum) -> Option<HirEnum> {
        self.declarations.insert_enumeration(enumeration)
    }

    #[allow(dead_code)]
    pub(crate) fn remove_enumeration(&mut self, id: DefId) -> Option<HirEnum> {
        self.declarations.remove_enumeration(id)
    }

    pub(crate) fn type_alias(&self, id: DefId) -> Option<&HirTypeAlias> {
        self.declarations.type_alias(id)
    }

    pub(crate) fn type_aliases(&self) -> impl Iterator<Item = (DefId, &HirTypeAlias)> {
        self.declarations.type_alias_defs()
    }

    pub(crate) fn insert_type_alias(&mut self, alias: HirTypeAlias) -> Option<HirTypeAlias> {
        self.declarations.insert_type_alias(alias)
    }

    #[allow(dead_code)]
    pub(crate) fn trait_def(&self, id: DefId) -> Option<&HirTrait> {
        self.declarations.trait_def(id)
    }

    #[allow(dead_code)]
    pub(crate) fn trait_def_mut(&mut self, id: DefId) -> Option<&mut HirTrait> {
        self.declarations.trait_def_mut(id)
    }

    #[allow(dead_code)]
    pub(crate) fn trait_defs(&self) -> impl Iterator<Item = (DefId, &HirTrait)> {
        self.declarations.trait_defs()
    }

    pub(crate) fn traits_for_selection(&self) -> &HashMap<DefId, HirTrait> {
        self.declarations.traits()
    }

    pub(crate) fn insert_trait_def(&mut self, trait_def: HirTrait) -> Option<HirTrait> {
        self.declarations.insert_trait_def(trait_def)
    }

    #[allow(dead_code)]
    pub(crate) fn remove_trait_def(&mut self, id: DefId) -> Option<HirTrait> {
        self.declarations.remove_trait_def(id)
    }

    pub(crate) fn extern_def(&self, id: DefId) -> Option<&HirExtern> {
        self.declarations.extern_def(id)
    }

    #[allow(dead_code)]
    pub(crate) fn extern_mut(&mut self, id: DefId) -> Option<&mut HirExtern> {
        self.declarations.extern_mut(id)
    }

    #[allow(dead_code)]
    pub(crate) fn externs(&self) -> impl Iterator<Item = (DefId, &HirExtern)> {
        self.declarations.extern_defs()
    }

    pub(crate) fn insert_extern(&mut self, extern_def: HirExtern) -> Option<HirExtern> {
        self.declarations.insert_extern(extern_def)
    }

    #[allow(dead_code)]
    pub(crate) fn remove_extern(&mut self, id: DefId) -> Option<HirExtern> {
        self.declarations.remove_extern(id)
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
