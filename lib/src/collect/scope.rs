use std::collections::HashMap;

use crate::ids::{IdGen, TypeVarId};
use crate::types::Type;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeclarationBinding {
    pub(crate) ty: Type,
    pub(crate) mutable: bool,
}

#[derive(Clone, Default)]
pub(crate) struct DeclarationScope {
    bindings: HashMap<String, DeclarationBinding>,
}

impl DeclarationScope {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn define(&mut self, name: String, ty: Type, mutable: bool) {
        self.bindings
            .insert(name, DeclarationBinding { ty, mutable });
    }

    pub(crate) fn lookup(&self, name: &str) -> Option<&DeclarationBinding> {
        self.bindings.get(name)
    }
}

pub struct DeclarationTypeVars {
    next_var: IdGen<TypeVarId>,
    var_spans: HashMap<TypeVarId, crate::lexer::Span>,
}

impl DeclarationTypeVars {
    pub(crate) fn new() -> Self {
        Self {
            next_var: IdGen::new(),
            var_spans: HashMap::new(),
        }
    }

    pub(crate) fn fresh_type_var_at(&mut self, span: crate::lexer::Span) -> Type {
        let id = self.next_var.fresh();
        self.var_spans.insert(id, span);
        Type::TypeVar(id)
    }

    pub(crate) fn var_spans(&self) -> &HashMap<TypeVarId, crate::lexer::Span> {
        &self.var_spans
    }

    pub(crate) fn next_raw(&self) -> u32 {
        self.next_var.next_raw()
    }
}

impl Default for DeclarationTypeVars {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
