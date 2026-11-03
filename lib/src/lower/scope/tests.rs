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
