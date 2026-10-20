use std::collections::HashMap;

use crate::hir::HirGenericBounds;
use crate::ids::{DefId, HirLocalId, IdGen, Idx};
use crate::lexer::lossless::SyntaxContext;
use crate::lower::scope::{LocalName, Scope};
use crate::types::{GenericParamId, Type};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BodyOwner {
    Function(DefId),
    ImplMethod {
        impl_id: DefId,
        method_id: DefId,
        method_name: String,
    },
    TraitMethod {
        trait_id: DefId,
        method_id: DefId,
        method_name: String,
    },
}

#[derive(Clone)]
pub(crate) struct GenericLoweringContext {
    owner: DefId,
    params: Vec<String>,
}

impl GenericLoweringContext {
    pub(crate) fn new(owner: DefId, params: Vec<String>) -> Self {
        Self { owner, params }
    }

    pub(crate) fn owner(&self) -> DefId {
        self.owner
    }

    pub(crate) fn params(&self) -> &[String] {
        &self.params
    }

    pub(crate) fn params_mut(&mut self) -> &mut Vec<String> {
        &mut self.params
    }
}

pub(crate) struct BodyLoweringContext {
    #[allow(dead_code)]
    function_name: String,
    #[allow(dead_code)]
    owner: BodyOwner,
    generic_owner: Option<DefId>,
    generic_params: Vec<String>,
    generic_param_ids: HashMap<LocalName, GenericParamId>,
    impl_bounds: HirGenericBounds,
    return_type_override: Option<Type>,
    in_unsafe: bool,
    local_ids: IdGen<HirLocalId>,
    scope: Scope,
}

impl BodyLoweringContext {
    pub(crate) fn new(
        function_name: String,
        owner: BodyOwner,
        generic_owner: Option<DefId>,
        generic_params: Vec<String>,
        impl_bounds: HirGenericBounds,
        in_unsafe: bool,
    ) -> Self {
        Self {
            function_name,
            owner,
            generic_owner,
            generic_params,
            generic_param_ids: HashMap::new(),
            impl_bounds,
            return_type_override: None,
            in_unsafe,
            local_ids: IdGen::<HirLocalId>::new(),
            scope: Scope::new(),
        }
    }

    pub(crate) fn function_name(&self) -> &str {
        &self.function_name
    }

    pub(crate) fn owner(&self) -> &BodyOwner {
        &self.owner
    }

    pub(crate) fn generic_owner(&self) -> Option<DefId> {
        self.generic_owner
    }

    pub(crate) fn generic_params(&self) -> &[String] {
        &self.generic_params
    }

    pub(crate) fn generic_params_mut(&mut self) -> &mut Vec<String> {
        &mut self.generic_params
    }

    pub(crate) fn generic_param_id(&self, name: &str, context: SyntaxContext) -> Option<GenericParamId> {
        self.generic_param_ids.get(&LocalName::new(name.to_string(), context)).copied()
    }

    pub(crate) fn register_generic_param(&mut self, name: String, context: SyntaxContext, id: GenericParamId) {
        self.generic_param_ids.insert(LocalName::new(name, context), id);
    }

    pub(crate) fn replace_generic_context(
        &mut self,
        owner: Option<DefId>,
        params: Vec<String>,
    ) -> (Option<DefId>, Vec<String>) {
        let previous = (
            self.generic_owner,
            std::mem::replace(&mut self.generic_params, params),
        );
        self.generic_owner = owner;
        previous
    }

    pub(crate) fn impl_bounds(&self) -> &HirGenericBounds {
        &self.impl_bounds
    }

    pub(crate) fn return_type_override(&self) -> Option<&Type> {
        self.return_type_override.as_ref()
    }

    pub(crate) fn replace_return_type_override(&mut self, ty: Option<Type>) -> Option<Type> {
        std::mem::replace(&mut self.return_type_override, ty)
    }

    pub(crate) fn in_unsafe(&self) -> bool {
        self.in_unsafe
    }

    pub(crate) fn replace_in_unsafe(&mut self, in_unsafe: bool) -> bool {
        std::mem::replace(&mut self.in_unsafe, in_unsafe)
    }

    pub(crate) fn fresh_local_id(&mut self) -> HirLocalId {
        self.local_ids.fresh()
    }

    pub(crate) fn seed_after_existing_locals(
        &mut self,
        locals: impl IntoIterator<Item = HirLocalId>,
    ) {
        let next = locals
            .into_iter()
            .map(|local| local.raw().saturating_add(1))
            .max()
            .unwrap_or(0);
        self.local_ids = IdGen::<HirLocalId>::with_next_raw(next);
    }

    pub(crate) fn replace_scope(&mut self, scope: Scope) -> Scope {
        std::mem::replace(&mut self.scope, scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ids::{CrateId, LocalDefId};

    #[test]
    fn body_context_can_continue_after_existing_parameter_ids() {
        let mut context = BodyLoweringContext::new(
            "body".to_string(),
            BodyOwner::Function(DefId::new(CrateId(0), LocalDefId(1))),
            None,
            Vec::new(),
            HirGenericBounds::new(),
            false,
        );

        context.seed_after_existing_locals([HirLocalId(0), HirLocalId(2)]);

        assert_eq!(context.fresh_local_id(), HirLocalId(3));
        assert_eq!(context.fresh_local_id(), HirLocalId(4));
    }
}
