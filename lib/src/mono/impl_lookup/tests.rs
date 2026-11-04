use super::*;

impl Monomorphizer {
    pub(crate) fn set_impl_receiver_pattern_for_test(
        &mut self,
        impl_id: DefId,
        pattern: HirImplReceiverPattern,
    ) {
        if let Some(imp) = self.generic_impls.get_mut(&impl_id) {
            imp.receiver_pattern = pattern;
            return;
        }
        if let Some(imp) = self
            .trait_impls
            .values_mut()
            .flat_map(|impls| impls.iter_mut())
            .find(|imp| imp.id == impl_id)
        {
            imp.receiver_pattern = pattern;
        }
    }
}
