use super::*;

use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mono::hir_types::{HirBlock, HirFunction};
use crate::types::Type;

fn i64_type_id() -> crate::ids::TypeId {
    let mut context = crate::type_context::TypeContext::new();
    context.intern_type(&Type::I64)
}

fn empty_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: std::collections::HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

#[test]
fn mono_reexports_canonical_instance_id() {
    let id: crate::ids::InstanceId = crate::mono::InstanceId(7);

    assert_eq!(id.0, 7);
}

#[test]
fn instance_registry_uses_type_ids_for_substitution_identity() {
    let mut context = crate::type_context::TypeContext::new();
    let first_ty = context.intern_type(&Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(10)),
        args: Vec::new(),
    });
    let second_ty = context.intern_type(&Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(11)),
        args: Vec::new(),
    });
    let mut registry = InstanceRegistry::new();
    let function = DefId::new(CrateId(0), LocalDefId(1));
    let first_key = InstanceKey::new(InstanceOrigin::Function(function), vec![first_ty]);
    let second_key = InstanceKey::new(InstanceOrigin::Function(function), vec![second_ty]);

    let first = registry.intern(first_key.clone(), |id| InstanceRecord {
        id,
        origin: first_key.origin.clone(),
        substitution: first_key.substitution.clone(),
        symbols: InstanceSymbols::new("id", "id_first"),
        declared: None,
        provided_by_object: false,
        is_specialization: true,
    });
    let second = registry.intern(second_key.clone(), |id| InstanceRecord {
        id,
        origin: second_key.origin.clone(),
        substitution: second_key.substitution.clone(),
        symbols: InstanceSymbols::new("id", "id_second"),
        declared: None,
        provided_by_object: false,
        is_specialization: true,
    });

    assert_ne!(first, second);
    assert_eq!(registry.len(), 2);
}

#[test]
fn instance_registry_reuses_same_function_instance() {
    let mut registry = InstanceRegistry::new();
    let def_id = DefId::new(CrateId(0), LocalDefId(1));
    let key = InstanceKey::new(InstanceOrigin::Function(def_id), vec![i64_type_id()]);

    let first = registry.intern(key.clone(), |id| InstanceRecord {
        id,
        origin: key.origin.clone(),
        substitution: key.substitution.clone(),
        symbols: InstanceSymbols::new("std::parse::identity", "std::parse::identity_mono_0"),
        declared: None,
        provided_by_object: false,
        is_specialization: true,
    });

    let second = registry.intern(key, |_| {
        panic!("duplicate instance should reuse the existing id");
    });

    assert_eq!(first, second);
    assert_eq!(registry.len(), 1);
}

fn impl_method_key(owner: DefId, method: DefId) -> InstanceKey {
    InstanceKey::new(
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(owner),
            method,
        },
        vec![i64_type_id()],
    )
}

fn impl_method_record(
    id: InstanceId,
    key: &InstanceKey,
    source_name: &str,
    backend_symbol: &str,
) -> InstanceRecord {
    InstanceRecord {
        id,
        origin: key.origin.clone(),
        substitution: key.substitution.clone(),
        symbols: InstanceSymbols::new(source_name, backend_symbol),
        declared: None,
        provided_by_object: false,
        is_specialization: false,
    }
}

fn trait_default_key(trait_id: DefId, method: DefId) -> InstanceKey {
    InstanceKey::new(
        InstanceOrigin::TraitDefault { trait_id, method },
        Vec::new(),
    )
}

#[test]
fn instance_registry_distinguishes_impl_methods_by_method_id() {
    let mut registry = InstanceRegistry::new();
    let owner = DefId::new(CrateId(0), LocalDefId(2));
    let map_method = DefId::new(CrateId(0), LocalDefId(3));
    let println_method = DefId::new(CrateId(0), LocalDefId(4));

    let map_key = impl_method_key(owner, map_method);
    let println_key = impl_method_key(owner, println_method);

    let map_id = registry.intern(map_key.clone(), |id| {
        impl_method_record(id, &map_key, "Option::map", "Option_I64_map")
    });
    let println_id = registry.intern(println_key.clone(), |id| {
        impl_method_record(id, &println_key, "Option::println", "Option_I64_println")
    });

    assert_ne!(map_id, println_id);
    assert_eq!(registry.len(), 2);
}

#[test]
fn instance_registry_distinguishes_impl_methods_by_owner_and_method_id() {
    let mut registry = InstanceRegistry::new();
    let option_owner = DefId::new(CrateId(0), LocalDefId(2));
    let result_owner = DefId::new(CrateId(0), LocalDefId(3));
    let map_method = DefId::new(CrateId(0), LocalDefId(4));

    let option_key = impl_method_key(option_owner, map_method);
    let result_key = impl_method_key(result_owner, map_method);

    let option_id = registry.intern(option_key.clone(), |id| {
        impl_method_record(id, &option_key, "Option::map", "Option_I64_map")
    });
    let result_id = registry.intern(result_key.clone(), |id| {
        impl_method_record(id, &result_key, "Result::map", "Result_I64_map")
    });

    assert_ne!(option_id, result_id);
    assert_eq!(registry.len(), 2);
}

#[test]
fn instance_registry_ignores_backend_symbol_for_identity() {
    let mut registry = InstanceRegistry::new();
    let owner = DefId::new(CrateId(0), LocalDefId(2));
    let method = DefId::new(CrateId(0), LocalDefId(3));
    let key = impl_method_key(owner, method);

    let first = registry.intern(key.clone(), |id| {
        impl_method_record(id, &key, "Option::map", "Option_I64_map")
    });
    let second = registry.intern(key, |_| {
        panic!("duplicate instance should reuse existing id even if backend symbol would differ");
    });

    assert_eq!(first, second);
    assert_eq!(registry.len(), 1);
    assert_eq!(
        registry
            .record(first)
            .map(|record| record.symbols.backend_symbol.as_str()),
        Some("Option_I64_map")
    );
}

#[test]
fn instance_registry_reuses_zero_substitution_function_instance() {
    let mut registry = InstanceRegistry::new();
    let function_id = DefId::new(CrateId(0), LocalDefId(10));
    let key = InstanceKey::new(InstanceOrigin::Function(function_id), Vec::new());

    let first = registry.intern(key.clone(), |id| InstanceRecord {
        id,
        origin: key.origin.clone(),
        substitution: key.substitution.clone(),
        symbols: InstanceSymbols::new("main", "main"),
        declared: None,
        provided_by_object: false,
        is_specialization: false,
    });

    let second = registry.intern(key, |_| {
        panic!("zero-substitution function should reuse existing instance id");
    });

    assert_eq!(first, second);
    assert_eq!(registry.len(), 1);
}

#[test]
fn instance_registry_distinguishes_trait_default_methods_by_trait_and_method_id() {
    let mut registry = InstanceRegistry::new();
    let show_trait = DefId::new(CrateId(0), LocalDefId(20));
    let debug_trait = DefId::new(CrateId(0), LocalDefId(21));
    let fmt_method = DefId::new(CrateId(0), LocalDefId(22));
    let display_method = DefId::new(CrateId(0), LocalDefId(23));

    let show_fmt = trait_default_key(show_trait, fmt_method);
    let debug_fmt = trait_default_key(debug_trait, fmt_method);
    let show_display = trait_default_key(show_trait, display_method);

    let show_fmt_id = registry.intern(show_fmt.clone(), |id| {
        impl_method_record(id, &show_fmt, "Show::fmt", "Show_fmt")
    });
    let debug_fmt_id = registry.intern(debug_fmt.clone(), |id| {
        impl_method_record(id, &debug_fmt, "Debug::fmt", "Debug_fmt")
    });
    let show_display_id = registry.intern(show_display.clone(), |id| {
        impl_method_record(id, &show_display, "Show::display", "Show_display")
    });

    assert_ne!(show_fmt_id, debug_fmt_id);
    assert_ne!(show_fmt_id, show_display_id);
    assert_eq!(registry.len(), 3);
}
