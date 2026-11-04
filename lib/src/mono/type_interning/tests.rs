use super::*;

impl Monomorphizer {
    pub(crate) fn intern_types(&mut self, types: &[Type]) -> Vec<crate::ids::TypeId> {
        types.iter().map(|ty| self.intern_type(ty)).collect()
    }
}
