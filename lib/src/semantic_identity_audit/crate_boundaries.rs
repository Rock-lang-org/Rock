use super::*;

#[test]
fn compiler_phases_do_not_branch_on_dependency_storage_mode() {
    for path in [
        "collect/context.rs",
        "collect/mod.rs",
        "lower/crates/registration.rs",
        "lower/crates/bodies.rs",
        "mono/mod.rs",
        "mono/external.rs",
    ] {
        assert_production_file_absent(
            path,
            &[
                ".link()",
                "is_object_backed",
                "object_backed",
                "concrete_impl_body_is_object_provided",
                "provides_impl_object",
                "object_backed_impls",
                "set_object_backed_impls",
                "impl_body_is_provided_by_object",
                "source_backed_dependency_errors",
                "dep.bodies()",
            ],
        );
    }
}

#[test]
fn prelude_and_root_exports_use_id_backed_capabilities() {
    for path in [
        "collect/context.rs",
        "collect/collector.rs",
        "collect/mod.rs",
        "lower/prelude.rs",
        "lower/session.rs",
    ] {
        assert_production_file_absent(
            path,
            &[
                "std_prelude_export_ids",
                "artifact_root_export_ids",
                "artifact_root_exports",
                "std_prelude_exports",
                "capture_std_exports",
                "inject_std_prelude",
            ],
        );
    }
}

#[test]
fn artifact_crate_interfaces_are_id_keyed_and_hir_free() {
    let source = production_source("crate_artifact/types.rs");
    assert_absent(
        &source,
        &[
            "use crate::hir::{HirEnum, HirExtern, HirFunction, HirImpl, HirStruct, HirTrait};",
            "pub functions: BTreeMap<String, HirFunction>",
            "pub structs: BTreeMap<String, HirStruct>",
            "pub enums: BTreeMap<String, HirEnum>",
            "pub traits: BTreeMap<String, HirTrait>",
            "pub impls: Vec<HirImpl>",
            "pub externs: Vec<HirExtern>",
        ],
    );
    assert_present(
        &source,
        &[
            "pub functions: BTreeMap<DefId, ProductFunctionInterface>",
            "pub structs: BTreeMap<DefId, ProductStructInterface>",
            "pub enums: BTreeMap<DefId, ProductEnumInterface>",
            "pub traits: BTreeMap<DefId, ProductTraitInterface>",
            "pub impls: BTreeMap<DefId, ProductImplInterface>",
            "pub externs: BTreeMap<DefId, ProductExternInterface>",
            "pub generic_functions: BTreeMap<DefId, AcceptedHirFunction>",
            "pub traits_with_defaults: BTreeMap<DefId, AcceptedHirTrait>",
        ],
    );

    let load = production_source("crate_artifact/load.rs");
    assert_absent(
        &load,
        &[
            "product_display_name(products, *id).map(|_| (*id, function.clone()))",
            "product_display_name(products, *id).map(|name| (*id, name, value.clone()))",
            "product_display_name(products, *id).map(|name| (*id, name, function.clone()))",
            "let Some(name) = product_display_name(products, *id) else",
        ],
    );
}

#[test]
fn cross_crate_bodies_are_provider_capabilities() {
    let store_source = production_source("crate_system/extern_store.rs");
    assert_absent(
        &store_source,
        &[
            "generic_functions: BTreeMap<String, HirFunction>",
            "traits_with_defaults: BTreeMap<String, HirTrait>",
            "generic_impls: Vec<HirImpl>",
            "pub(crate) fn generic_functions(&self) -> &BTreeMap<String, HirFunction>",
            "pub(crate) fn traits_with_defaults(&self) -> &BTreeMap<String, HirTrait>",
            "pub(crate) fn generic_impls(&self) -> &[HirImpl]",
        ],
    );
    assert_present(
        &store_source,
        &[
            "providers: ExternBodyProviders",
            "pub(crate) fn body_providers(&self)",
        ],
    );

    let load_source = production_source("crate_artifact/load.rs");
    assert_absent(
        &load_source,
        &[
            "dep.bodies().generic_functions()",
            "dep.bodies().generic_impls()",
            "dep.body_providers()",
            "bodies.generic_functions()",
            "bodies.traits_with_defaults()",
            "bodies.generic_impls()",
        ],
    );
    assert_present(&load_source, &["record_dependency_impl(&mut defs"]);
}
