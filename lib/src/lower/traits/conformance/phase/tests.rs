use super::*;

impl Lowerer {
    pub(crate) fn auto_impl_sized(&mut self) {
        TraitConformancePhase::auto_impl_sized(self);
    }

    pub(crate) fn check_trait_conformance(&mut self) {
        TraitConformancePhase::run(self);
    }
}
