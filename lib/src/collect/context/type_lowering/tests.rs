use super::*;

impl CollectContext {
    pub(crate) fn push_generic_context(
        &mut self,
        owner: DefId,
        params: Vec<String>,
    ) -> (Option<DefId>, Vec<String>) {
        let kinds = vec![crate::type_services::kind::Kind::Type; params.len()];
        let (owner, params, _) = self.push_generic_context_with_kinds(owner, params, kinds);
        (owner, params)
    }

    pub(crate) fn push_generic_context_with_kinds(
        &mut self,
        owner: DefId,
        params: Vec<String>,
        kinds: Vec<crate::type_services::kind::Kind>,
    ) -> (
        Option<DefId>,
        Vec<String>,
        Vec<crate::type_services::kind::Kind>,
    ) {
        self.push_generic_context_with_kinds_at(owner, params, kinds, &[])
    }

    pub(crate) fn pop_generic_context(
        &mut self,
        prev_owner: Option<DefId>,
        prev_params: Vec<String>,
    ) {
        self.current_generic_params = prev_params;
        self.current_generic_kinds =
            vec![crate::type_services::kind::Kind::Type; self.current_generic_params.len()];
        self.current_generic_owner = prev_owner;
    }
}
