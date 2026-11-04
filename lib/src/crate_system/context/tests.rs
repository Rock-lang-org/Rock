use super::*;

impl CrateContext {
    pub(crate) fn extern_crate(&self, name: &str) -> Option<ExternCrateRef<'_>> {
        self.extern_crates.by_name(name)
    }
}
