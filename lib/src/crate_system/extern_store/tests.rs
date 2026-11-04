use super::*;

impl ExternCrateStore {
    pub(crate) fn by_name(&self, name: &str) -> Option<ExternCrateRef<'_>> {
        self.names
            .get(name)
            .and_then(|crate_id| self.records.get(crate_id))
            .map(ExternCrateRef::new)
    }

    pub(crate) fn by_crate_id(&self, crate_id: CrateId) -> Option<ExternCrateRef<'_>> {
        self.records.get(&crate_id).map(ExternCrateRef::new)
    }
}

impl ExternBodyProviders {
    pub(crate) fn trait_with_defaults(&self, id: DefId) -> Option<&HirTrait> {
        self.traits_with_defaults_by_id.get(&id)
    }
}

impl ExternCrateMetadata {
    pub(crate) fn empty_for_test() -> Self {
        Self::new(
            ArtifactCrateInterface::default(),
            ResolverTables::default(),
            BTreeMap::new(),
        )
    }
}
