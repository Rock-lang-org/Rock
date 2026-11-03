use super::*;

impl HirProgram {
    pub fn from_id_parts_with_names(
        functions: HashMap<DefId, HirFunction>,
        structs: HashMap<DefId, HirStruct>,
        enums: HashMap<DefId, HirEnum>,
        traits: HashMap<DefId, HirTrait>,
        impls: HashMap<DefId, HirImpl>,
        externs: HashMap<DefId, HirExtern>,
        names: HirNameTables,
        language_items: HirLanguageItems,
    ) -> Self {
        let mut canonical_names_by_id = HashMap::new();
        canonical_names_by_id.extend(
            functions
                .iter()
                .map(|(id, function)| (*id, function.name.clone())),
        );
        canonical_names_by_id.extend(
            structs
                .iter()
                .map(|(id, structure)| (*id, structure.name.clone())),
        );
        canonical_names_by_id.extend(
            enums
                .iter()
                .map(|(id, enum_def)| (*id, enum_def.name.clone())),
        );
        canonical_names_by_id.extend(
            traits
                .iter()
                .map(|(id, trait_def)| (*id, trait_def.name.clone())),
        );

        Self::from_id_parts_with_names_and_canonical_names(
            functions,
            structs,
            enums,
            traits,
            impls,
            externs,
            names,
            language_items,
            &canonical_names_by_id,
        )
    }

    pub fn from_id_parts_with_names_and_canonical_names(
        functions: HashMap<DefId, HirFunction>,
        structs: HashMap<DefId, HirStruct>,
        enums: HashMap<DefId, HirEnum>,
        traits: HashMap<DefId, HirTrait>,
        impls: HashMap<DefId, HirImpl>,
        externs: HashMap<DefId, HirExtern>,
        names: HirNameTables,
        language_items: HirLanguageItems,
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
            language_items,
            indexes: HirDefinitionIndexes::default(),
        };
        program.rebuild_indexes_with_canonical_names(&canonical_names_by_id);
        program
    }

    pub fn rebuild_indexes(&mut self) {
        self.rebuild_indexes_with_canonical_names(&HashMap::new());
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
        self.indexes
            .effective_trait_methods
            .extend(effective_trait_methods);
    }

    pub fn function_by_id(&self, id: DefId) -> Option<(&str, &HirFunction)> {
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

    pub fn struct_by_id(&self, id: DefId) -> Option<(&str, &HirStruct)> {
        self.structs.get(&id).map(|structure| {
            let name = self
                .indexes
                .structs_by_id
                .get(&id)
                .map(String::as_str)
                .unwrap_or(structure.name.as_str());
            (name, structure)
        })
    }

    pub fn enum_by_id(&self, id: DefId) -> Option<(&str, &HirEnum)> {
        self.enums.get(&id).map(|enumeration| {
            let name = self
                .indexes
                .enums_by_id
                .get(&id)
                .map(String::as_str)
                .unwrap_or(enumeration.name.as_str());
            (name, enumeration)
        })
    }

    pub fn trait_by_id(&self, id: DefId) -> Option<(&str, &HirTrait)> {
        self.traits.get(&id).map(|trt| {
            let name = self
                .indexes
                .traits_by_id
                .get(&id)
                .map(String::as_str)
                .unwrap_or(trt.name.as_str());
            (name, trt)
        })
    }

    pub fn impl_by_id(&self, id: DefId) -> Option<(usize, &HirImpl)> {
        let index = *self.indexes.impls_by_id.get(&id)?;
        self.impls.get(&id).map(|imp| (index, imp))
    }

    pub fn extern_by_id(&self, id: DefId) -> Option<(usize, &HirExtern)> {
        let index = *self.indexes.externs_by_id.get(&id)?;
        self.externs.get(&id).map(|ext| (index, ext))
    }

    pub fn functions_by_id(&self) -> impl Iterator<Item = (DefId, &str, &HirFunction)> {
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

    pub fn traits_by_id(&self) -> impl Iterator<Item = (DefId, &str, &HirTrait)> {
        self.traits.iter().map(|(id, trt)| {
            let name = self
                .indexes
                .traits_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(trt.name.as_str());
            (*id, name, trt)
        })
    }

    pub fn impls_by_id(&self) -> impl Iterator<Item = (DefId, usize, &HirImpl)> {
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

    /// Display/compatibility aliases for a canonical function owner.
    ///
    /// Semantic consumers should use `function_by_id` when they already have a
    /// `DefId`; this helper is only for display, artifact compatibility, and
    /// alias metadata derived from a known owner.
    pub fn function_display_aliases(&self, id: DefId) -> Vec<&str> {
        let Some((primary_name, _)) = self.function_by_id(id) else {
            return Vec::new();
        };

        display_aliases_for_id(&self.names.functions_by_name, id, Some(primary_name))
    }

    /// Display/compatibility aliases for a canonical struct owner.
    pub fn struct_display_aliases(&self, id: DefId) -> Vec<&str> {
        let Some((primary_name, _)) = self.struct_by_id(id) else {
            return Vec::new();
        };

        display_aliases_for_id(&self.names.structs_by_name, id, Some(primary_name))
    }

    /// Display/compatibility aliases for a canonical enum owner.
    pub fn enum_display_aliases(&self, id: DefId) -> Vec<&str> {
        let Some((primary_name, _)) = self.enum_by_id(id) else {
            return Vec::new();
        };

        display_aliases_for_id(&self.names.enums_by_name, id, Some(primary_name))
    }

    /// Display/compatibility aliases for a canonical nominal owner.
    pub fn nominal_display_aliases(&self, id: DefId) -> Vec<&str> {
        if self.structs.contains_key(&id) {
            self.struct_display_aliases(id)
        } else if self.enums.contains_key(&id) {
            self.enum_display_aliases(id)
        } else {
            Vec::new()
        }
    }

    /// Resolve a display alias to an existing nominal owner.
    ///
    /// This is a compatibility helper for HIR metadata that still stores a
    /// display owner string. Prefer `Type::Struct { id, .. }`,
    /// `Type::Enum { id, .. }`, or explicit owner sidecars when available.
    pub fn nominal_owner_id_for_display_alias(&self, name: &str) -> Option<DefId> {
        self.names
            .structs_by_name
            .get(name)
            .copied()
            .filter(|id| self.structs.contains_key(id))
            .or_else(|| {
                self.names
                    .enums_by_name
                    .get(name)
                    .copied()
                    .filter(|id| self.enums.contains_key(id))
            })
    }

    pub fn impl_owner_id(&self, impl_id: DefId) -> Option<DefId> {
        self.indexes.impl_owner_ids_by_id.get(&impl_id).copied()
    }

    pub fn impl_receiver_pattern(&self, impl_id: DefId) -> Option<&HirImplReceiverPattern> {
        self.impls.get(&impl_id).map(|imp| &imp.receiver_pattern)
    }

    pub fn effective_trait_method(&self, impl_id: DefId, trait_method_id: DefId) -> Option<DefId> {
        self.indexes
            .effective_trait_methods
            .get(&(impl_id, trait_method_id))
            .copied()
    }

    pub fn impls_in_order(&self) -> impl Iterator<Item = (DefId, &HirImpl)> {
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
