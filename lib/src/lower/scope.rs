//! Scope management for name resolution

use std::collections::HashMap;

use crate::ids::HirLocalId;
use crate::lexer::lossless::SyntaxContext;
use crate::types::Type;

/// Local identity before lowering assigns a canonical `HirLocalId`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LocalName {
    pub spelling: String,
    pub context: SyntaxContext,
}

impl LocalName {
    pub(crate) fn new(spelling: String, context: SyntaxContext) -> Self {
        Self { spelling, context }
    }

    pub(crate) fn from_segment(segment: &crate::ast::IdentOrType) -> Option<Self> {
        match segment {
            crate::ast::IdentOrType::Ident(ident) => Some(ident.into()),
            crate::ast::IdentOrType::Type(crate::ast::ParseType::Type(ty)) => {
                Some(Self::new(ty.name.clone(), ty.context))
            }
            _ => None,
        }
    }
}

impl From<String> for LocalName {
    fn from(spelling: String) -> Self { Self::new(spelling, SyntaxContext::ROOT) }
}

impl From<&str> for LocalName {
    fn from(spelling: &str) -> Self { spelling.to_string().into() }
}

impl From<&String> for LocalName {
    fn from(spelling: &String) -> Self { spelling.as_str().into() }
}

impl From<&crate::ast::Ident> for LocalName {
    fn from(ident: &crate::ast::Ident) -> Self {
        Self::new(ident.name.clone(), ident.context)
    }
}

impl From<&LocalName> for LocalName {
    fn from(name: &LocalName) -> Self { name.clone() }
}

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
    vars: Vec<HashMap<LocalName, ScopeBinding>>,
    locals: Vec<HashMap<HirLocalId, (String, ScopeBinding)>>,
}

impl Scope {
    pub fn new() -> Self {
        Self {
            vars: vec![HashMap::new()],
            locals: vec![HashMap::new()],
        }
    }

    pub fn push(&mut self) {
        self.vars.push(HashMap::new());
        self.locals.push(HashMap::new());
    }

    pub fn pop(&mut self) {
        self.vars.pop();
        self.locals.pop();
    }

    pub fn define(&mut self, name: String, ty: Type, mutable: bool) {
        self.define_local_without_id(name, ty, mutable);
    }

    pub fn define_local_without_id(&mut self, name: String, ty: Type, mutable: bool) {
        self.insert(
            name.into(),
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
            name.into(),
            ScopeBinding {
                ty,
                mutable,
                local_id: None,
                is_alias: false,
                is_top_level: true,
            },
        );
    }

    pub(crate) fn define_local(&mut self, name: impl Into<LocalName>, ty: Type, mutable: bool, local_id: HirLocalId) {
        self.insert(
            name.into(),
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
            name.into(),
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
            name.into(),
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

    fn insert(&mut self, name: LocalName, binding: ScopeBinding) {
        if let (Some(id), Some(locals)) = (binding.local_id, self.locals.last_mut()) {
            locals.insert(id, (name.spelling.clone(), binding.clone()));
        }
        if let Some(scope) = self.vars.last_mut() {
            scope.insert(name, binding);
        }
    }

    pub(crate) fn lookup(&self, name: impl Into<LocalName>) -> Option<&ScopeBinding> {
        let name = name.into();
        for scope in self.vars.iter().rev() {
            if let Some(entry) = scope.get(&name) {
                return Some(entry);
            }
        }
        None
    }

    pub(crate) fn binding_scope_index(&self, name: impl Into<LocalName>) -> Option<usize> {
        let name = name.into();
        for (index, scope) in self.vars.iter().enumerate().rev() {
            if scope.contains_key(&name) {
                return Some(index);
            }
        }
        None
    }

    pub(crate) fn local_binding(&self, id: HirLocalId) -> Option<(&str, &ScopeBinding)> {
        self.locals.iter().rev().find_map(|locals| locals.get(&id))
            .map(|(name, binding)| (name.as_str(), binding))
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
mod tests {
    use super::*;

    use crate::ids::HirLocalId;
    use crate::types::Type;

    #[test]
    fn scope_distinguishes_local_bindings_from_aliases() {
        let mut scope = Scope::new();
        scope.define_local("value".to_string(), Type::I64, true, HirLocalId(3));
        scope.define_alias(
            "println".to_string(),
            Type::function(vec![Type::I64], Type::I64),
            false,
        );

        let value = scope.lookup("value").expect("local should be in scope");
        assert_eq!(value.ty, Type::I64);
        assert!(value.mutable);
        assert_eq!(value.local_id, Some(HirLocalId(3)));
        assert!(!value.is_alias);
        assert!(!value.is_top_level);

        let alias = scope.lookup("println").expect("alias should be in scope");
        assert!(alias.local_id.is_none());
        assert!(alias.is_alias);
        assert!(!alias.is_top_level);
    }

    #[test]
    fn scope_tracks_top_level_and_local_binding_origins() {
        let mut scope = Scope::new();
        scope.define_top_level("function".to_string(), Type::I64, false);
        scope.define("local".to_string(), Type::I64, false);
        scope.define_local_without_id("unidentified".to_string(), Type::I64, false);

        assert!(scope.lookup("function").unwrap().is_top_level);
        assert!(!scope.lookup("local").unwrap().is_top_level);
        assert!(!scope.lookup("unidentified").unwrap().is_top_level);
    }

    #[test]
    fn scope_shadowing_returns_innermost_local_id() {
        let mut scope = Scope::new();
        scope.define_local("x".to_string(), Type::I64, false, HirLocalId(1));
        scope.push();
        scope.define_local("x".to_string(), Type::Bool, false, HirLocalId(2));

        assert_eq!(scope.lookup("x").unwrap().local_id, Some(HirLocalId(2)));
        scope.pop();
        assert_eq!(scope.lookup("x").unwrap().local_id, Some(HirLocalId(1)));
    }

    #[test]
    fn scope_defines_short_alias_from_existing_binding() {
        let mut scope = Scope::new();
        scope.define_local(
            "demo::helper::answer".to_string(),
            Type::I64,
            true,
            HirLocalId(7),
        );

        assert!(scope.define_alias_to_existing("answer".to_string(), "demo::helper::answer"));
        assert!(!scope.define_alias_to_existing("missing".to_string(), "demo::helper::missing"));

        let alias = scope.lookup("answer").expect("alias should be in scope");
        assert_eq!(alias.ty, Type::I64);
        assert!(!alias.mutable);
        assert!(alias.local_id.is_none());
        assert!(alias.is_alias);
        assert!(!alias.is_top_level);
        assert!(scope.lookup("missing").is_none());
    }
}
