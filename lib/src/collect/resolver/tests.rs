use super::*;

use crate::ids::{CrateId, DefId, LocalDefId};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

#[test]
fn resolver_tables_resolve_items_imports_and_exports_by_id() {
    let item_id = def_id(1);
    let import_id = def_id(2);
    let export_id = def_id(3);
    let mut resolver = ResolverTables::default();
    resolver
        .item_paths
        .insert("crate::item".to_string(), item_id);
    resolver
        .import_aliases
        .insert("imported".to_string(), import_id);
    resolver
        .export_aliases
        .insert("crate::alias".to_string(), export_id);

    assert_eq!(resolver.resolve_item_or_alias("crate::item"), Some(item_id));
    assert_eq!(resolver.resolve_item_or_alias("imported"), Some(import_id));
    assert_eq!(
        resolver.resolve_item_or_alias("crate::alias"),
        Some(export_id)
    );
    assert_eq!(resolver.resolve_item_or_alias("missing"), None);
}

#[test]
fn resolver_tables_insert_aliases_with_reverse_names() {
    let import_id = def_id(10);
    let export_id = def_id(11);
    let mut resolver = ResolverTables::default();

    resolver.insert_import_alias_with_name("short".to_string(), "dep::long".to_string(), import_id);
    resolver.insert_export_alias_with_name(
        "crate::public".to_string(),
        "crate::private".to_string(),
        export_id,
    );

    assert_eq!(resolver.import_aliases.get("short"), Some(&import_id));
    assert_eq!(
        resolver.export_aliases.get("crate::public"),
        Some(&export_id)
    );
    assert_eq!(resolver.canonical_name(import_id), Some("dep::long"));
    assert_eq!(resolver.canonical_name(export_id), Some("crate::private"));
}

#[test]
fn resolver_tables_resolve_distinct_alias_categories_by_id() {
    let import_id = def_id(21);
    let export_id = def_id(22);
    let module_id = def_id(23);
    let mut resolver = ResolverTables::default();

    resolver.insert_import_alias_with_name(
        "imported".to_string(),
        "dep::internal::value".to_string(),
        import_id,
    );
    resolver.insert_export_alias_with_name(
        "demo::public".to_string(),
        "demo::internal::value".to_string(),
        export_id,
    );
    resolver.insert_module_alias_with_name(
        "local".to_string(),
        "demo::module::local".to_string(),
        module_id,
    );

    assert_eq!(resolver.resolve_item_or_alias("imported"), Some(import_id));
    assert_eq!(
        resolver.resolve_item_or_alias("demo::public"),
        Some(export_id)
    );
    assert_eq!(resolver.resolve_item_or_alias("local"), Some(module_id));
    assert_eq!(
        resolver.canonical_name(module_id),
        Some("demo::module::local")
    );
}

#[test]
fn resolver_tables_expose_scoped_module_local_aliases() {
    let first_id = def_id(31);
    let second_id = def_id(32);
    let mut resolver = ResolverTables::default();

    resolver
        .scoped_module_aliases
        .entry("demo::first".to_string())
        .or_default()
        .insert("answer".to_string(), first_id);
    resolver
        .scoped_module_aliases
        .entry("demo::second".to_string())
        .or_default()
        .insert("answer".to_string(), second_id);

    assert_eq!(
        resolver.resolve_module_local_alias("demo::first", "answer"),
        Some(first_id)
    );
    assert_eq!(
        resolver.resolve_module_local_alias("demo::second", "answer"),
        Some(second_id)
    );
}
