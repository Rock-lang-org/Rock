use super::*;

impl HirProgramFor<AcceptedHir> {
    pub(crate) fn rebuild_indexes(&mut self) {
        self.rebuild_indexes_with_canonical_names(&HashMap::new());
    }

    pub(crate) fn test_function_by_name(
        &self,
        name: &str,
    ) -> Option<(DefId, &HirFunctionFor<AcceptedHir>)> {
        let id = *self.names.functions_by_name.get(name)?;
        self.functions.get(&id).map(|function| (id, function))
    }

    pub(crate) fn test_struct_by_name(&self, name: &str) -> Option<(DefId, &HirStruct)> {
        let id = *self.names.structs_by_name.get(name)?;
        self.structs.get(&id).map(|structure| (id, structure))
    }

    pub(crate) fn test_enum_by_name(&self, name: &str) -> Option<(DefId, &HirEnum)> {
        let id = *self.names.enums_by_name.get(name)?;
        self.enums.get(&id).map(|enumeration| (id, enumeration))
    }
}
