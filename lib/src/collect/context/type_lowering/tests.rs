use super::*;

impl CollectContext {
    pub(crate) fn push_generic_context(
        &mut self,
        owner: DefId,
        params: Vec<String>,
    ) -> GenericFrame {
        let kinds = vec![crate::type_services::kind::Kind::Type; params.len()];
        self.push_generic_context_with_kinds(owner, params, kinds)
    }

    pub(crate) fn push_generic_context_with_kinds(
        &mut self,
        owner: DefId,
        params: Vec<String>,
        kinds: Vec<crate::type_services::kind::Kind>,
    ) -> GenericFrame {
        self.push_generic_context_with_kinds_at(owner, params, kinds, &[])
    }

    pub(crate) fn pop_generic_context(&mut self, previous: GenericFrame) {
        self.pop_generic_context_with_kinds(previous);
    }
}
