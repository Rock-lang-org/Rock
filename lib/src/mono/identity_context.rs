use std::collections::BTreeMap;

use crate::products::ProductCrateIdentity;

#[derive(Debug, Clone)]
pub(crate) struct CompilationIdentityContext {
    current_crate_id: crate::ids::CrateId,
    current_crate: ProductCrateIdentity,
    pub(super) dependencies: BTreeMap<crate::ids::CrateId, ProductCrateIdentity>,
}

impl CompilationIdentityContext {
    pub(crate) fn new(
        current_crate_id: crate::ids::CrateId,
        current_crate: ProductCrateIdentity,
        dependencies: BTreeMap<crate::ids::CrateId, ProductCrateIdentity>,
    ) -> Self {
        Self {
            current_crate_id,
            current_crate,
            dependencies,
        }
    }

    pub(super) fn local_default() -> Self {
        Self::new(
            crate::ids::CrateId(0),
            ProductCrateIdentity::local("local".to_string()),
            BTreeMap::new(),
        )
    }

    pub(super) fn crate_identity(&self, crate_id: crate::ids::CrateId) -> &ProductCrateIdentity {
        if crate_id == self.current_crate_id {
            return &self.current_crate;
        }
        self.dependencies
            .get(&crate_id)
            .unwrap_or_else(|| panic!("missing product identity for session crate {crate_id:?}"))
    }
}
