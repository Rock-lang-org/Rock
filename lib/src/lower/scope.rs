//! Scope management for name resolution

use std::collections::HashMap;

use crate::ids::HirLocalId;
use crate::types::Type;

#[derive(Clone, Debug, PartialEq)]
pub struct ScopeBinding {
    pub ty: Type,
    pub mutable: bool,
    pub local_id: Option<HirLocalId>,
    pub is_alias: bool,
    pub is_top_level: bool,
}

/// Scope for name resolution - tracks variable bindings
#[derive(Clone)]
pub struct Scope {
    /// Stack of variable bindings.
    vars: Vec<HashMap<String, ScopeBinding>>,
}

impl Scope {
    pub fn new() -> Self {
        Self {
            vars: vec![HashMap::new()],
        }
    }

    pub fn push(&mut self) {
        self.vars.push(HashMap::new());
    }

    pub fn pop(&mut self) {
        self.vars.pop();
    }

    pub fn define(&mut self, name: String, ty: Type, mutable: bool) {
        self.define_local_without_id(name, ty, mutable);
    }

    pub fn define_local_without_id(&mut self, name: String, ty: Type, mutable: bool) {
        self.insert(
            name,
            ScopeBinding {
                ty,
                mutable,
                local_id: None,
                is_alias: false,
                is_top_level: false,
            },
        );
    }

    pub fn define_top_level(&mut self, name: String, ty: Type, mutable: bool) {
        self.insert(
            name,
            ScopeBinding {
                ty,
                mutable,
                local_id: None,
                is_alias: false,
                is_top_level: true,
            },
        );
    }

    pub fn define_local(&mut self, name: String, ty: Type, mutable: bool, local_id: HirLocalId) {
        self.insert(
            name,
            ScopeBinding {
                ty,
                mutable,
                local_id: Some(local_id),
                is_alias: false,
                is_top_level: false,
            },
        );
    }

    pub fn define_alias(&mut self, name: String, ty: Type, mutable: bool) {
        self.insert(
            name,
            ScopeBinding {
                ty,
                mutable,
                local_id: None,
                is_alias: true,
                is_top_level: false,
            },
        );
    }

    pub fn define_alias_to_existing(&mut self, name: String, existing_name: &str) -> bool {
        let Some(existing) = self.lookup(existing_name).cloned() else {
            return false;
        };

        self.insert(
            name,
            ScopeBinding {
                ty: existing.ty,
                mutable: false,
                local_id: None,
                is_alias: true,
                is_top_level: false,
            },
        );
        true
    }

    fn insert(&mut self, name: String, binding: ScopeBinding) {
        if let Some(scope) = self.vars.last_mut() {
            scope.insert(name, binding);
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&ScopeBinding> {
        for scope in self.vars.iter().rev() {
            if let Some(entry) = scope.get(name) {
                return Some(entry);
            }
        }
        None
    }

    pub fn binding_scope_index(&self, name: &str) -> Option<usize> {
        for (index, scope) in self.vars.iter().enumerate().rev() {
            if scope.contains_key(name) {
                return Some(index);
            }
        }
        None
    }

    pub fn binding_is_alias(&self, name: &str) -> bool {
        self.lookup(name).is_some_and(|binding| binding.is_alias)
    }
}

impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
