use super::*;

impl Monomorphizer {
    pub(crate) fn register_test_dependency_identity(&mut self, crate_id: crate::ids::CrateId) {
        self.identity_context.dependencies.insert(
            crate_id,
            ProductCrateIdentity::local("test-dependency".to_string()),
        );
    }
}
