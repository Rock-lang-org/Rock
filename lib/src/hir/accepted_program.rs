use super::*;

impl HirProgramFor<AcceptedHir> {
    pub(crate) fn from_accepted_id_parts_with_names_and_canonical_names(
        functions: HashMap<DefId, HirFunctionFor<AcceptedHir>>,
        structs: HashMap<DefId, HirStruct>,
        enums: HashMap<DefId, HirEnum>,
        traits: HashMap<DefId, HirTraitFor<AcceptedHir>>,
        impls: HashMap<DefId, HirImplFor<AcceptedHir>>,
        externs: HashMap<DefId, HirExtern>,
        names: HirNameTables,
        canonical_names_by_id: &HashMap<DefId, String>,
    ) -> Self {
        let impl_order = sorted_definition_ids(&impls);
        let extern_order = sorted_definition_ids(&externs);
        let mut program = Self {
            functions,
            structs,
            enums,
            traits,
            impls,
            externs,
            type_aliases: HashMap::new(),
            names,
            order: HirProgramOrder {
                impls: impl_order,
                externs: extern_order,
            },
            language_items: HirLanguageItems::default(),
            indexes: HirDefinitionIndexes::default(),
        };
        program.rebuild_indexes_with_canonical_names(canonical_names_by_id);
        program
    }

    pub fn struct_by_id(&self, id: DefId) -> Option<(&str, &HirStruct)> {
        let structure = self.structs.get(&id)?;
        let name = self
            .indexes
            .structs_by_id
            .get(&id)
            .map(String::as_str)
            .unwrap_or(structure.name.as_str());
        Some((name, structure))
    }

    pub fn enum_by_id(&self, id: DefId) -> Option<(&str, &HirEnum)> {
        let enumeration = self.enums.get(&id)?;
        let name = self
            .indexes
            .enums_by_id
            .get(&id)
            .map(String::as_str)
            .unwrap_or(enumeration.name.as_str());
        Some((name, enumeration))
    }

    pub fn rebuild_indexes_with_canonical_names(
        &mut self,
        canonical_names_by_id: &HashMap<DefId, String>,
    ) {
        let effective_trait_methods = std::mem::take(&mut self.indexes.effective_trait_methods);
        self.indexes = HirDefinitionIndexes::from_parts(
            &self.functions,
            &self.structs,
            &self.enums,
            &self.traits,
            &self.impls,
            &self.externs,
            &self.type_aliases,
            &self.names,
            &self.order,
            canonical_names_by_id,
        );
        self.indexes.effective_trait_methods = effective_trait_methods;
    }

    pub fn function_by_id(&self, id: DefId) -> Option<(&str, &HirFunctionFor<AcceptedHir>)> {
        self.functions.get(&id).map(|function| {
            let name = self
                .indexes
                .functions_by_id
                .get(&id)
                .map(String::as_str)
                .unwrap_or(function.name.as_str());
            (name, function)
        })
    }

    pub fn functions_by_id(
        &self,
    ) -> impl Iterator<Item = (DefId, &str, &HirFunctionFor<AcceptedHir>)> {
        self.functions.iter().map(|(id, function)| {
            let name = self
                .indexes
                .functions_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(function.name.as_str());
            (*id, name, function)
        })
    }

    pub fn function_display_aliases(&self, id: DefId) -> Vec<&str> {
        let Some((primary_name, _)) = self.function_by_id(id) else {
            return Vec::new();
        };
        display_aliases_for_id(&self.names.functions_by_name, id, Some(primary_name))
    }

    pub fn struct_display_aliases(&self, id: DefId) -> Vec<&str> {
        let Some((primary_name, _)) = self.struct_by_id(id) else {
            return Vec::new();
        };
        display_aliases_for_id(&self.names.structs_by_name, id, Some(primary_name))
    }

    pub fn enum_display_aliases(&self, id: DefId) -> Vec<&str> {
        let Some((primary_name, _)) = self.enum_by_id(id) else {
            return Vec::new();
        };
        display_aliases_for_id(&self.names.enums_by_name, id, Some(primary_name))
    }

    pub fn structs_by_id(&self) -> impl Iterator<Item = (DefId, &str, &HirStruct)> {
        self.structs.iter().map(|(id, structure)| {
            let name = self
                .indexes
                .structs_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(structure.name.as_str());
            (*id, name, structure)
        })
    }

    pub fn enums_by_id(&self) -> impl Iterator<Item = (DefId, &str, &HirEnum)> {
        self.enums.iter().map(|(id, enumeration)| {
            let name = self
                .indexes
                .enums_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(enumeration.name.as_str());
            (*id, name, enumeration)
        })
    }

    pub fn traits_by_id(&self) -> impl Iterator<Item = (DefId, &str, &HirTraitFor<AcceptedHir>)> {
        self.traits.iter().map(|(id, trait_def)| {
            let name = self
                .indexes
                .traits_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(trait_def.name.as_str());
            (*id, name, trait_def)
        })
    }

    pub fn impls_by_id(&self) -> impl Iterator<Item = (DefId, usize, &HirImplFor<AcceptedHir>)> {
        self.indexes
            .impls_by_id
            .iter()
            .filter_map(|(id, index)| self.impls.get(id).map(|imp| (*id, *index, imp)))
    }

    pub fn externs_by_id(&self) -> impl Iterator<Item = (DefId, usize, &HirExtern)> {
        self.indexes
            .externs_by_id
            .iter()
            .filter_map(|(id, index)| self.externs.get(id).map(|ext| (*id, *index, ext)))
    }

    pub fn impl_receiver_pattern(&self, impl_id: DefId) -> Option<&HirImplReceiverPattern> {
        self.impls.get(&impl_id).map(|imp| &imp.receiver_pattern)
    }

    pub fn effective_trait_method(&self, impl_id: DefId, member_id: DefId) -> Option<DefId> {
        self.indexes
            .effective_trait_methods
            .get(&(impl_id, member_id))
            .copied()
    }

    pub fn impls_in_order(&self) -> impl Iterator<Item = (DefId, &HirImplFor<AcceptedHir>)> {
        ordered_owned_ids(&self.impls, &self.order.impls)
            .into_iter()
            .filter_map(|id| self.impls.get(&id).map(|imp| (id, imp)))
    }

    pub fn externs_in_order(&self) -> impl Iterator<Item = (DefId, &HirExtern)> {
        ordered_owned_ids(&self.externs, &self.order.externs)
            .into_iter()
            .filter_map(|id| self.externs.get(&id).map(|ext| (id, ext)))
    }
}

#[cfg(test)]
mod tests;
