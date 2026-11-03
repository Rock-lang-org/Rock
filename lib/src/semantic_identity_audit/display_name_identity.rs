use super::*;

#[test]
fn legacy_backend_sidecar_and_adapter_tokens_are_absent() {
    for path in rust_source_files_under("") {
        if is_audit_source(&path) {
            continue;
        }
        assert_full_source_file_absent(&path, &["from_legacy_metadata"]);
    }
    for path in production_rust_source_files_under("") {
        if is_audit_source(&path) {
            continue;
        }
        assert_production_file_absent(&path, &["MirBackendMetadata", "backend_metadata"]);
    }
}

#[test]
fn downstream_phases_do_not_read_hir_display_aliases() {
    for path in ["mono", "mir", "codegen"] {
        assert_production_tree_absent(
            path,
            &[
                "function_display_aliases",
                "struct_display_aliases",
                "enum_display_aliases",
                "nominal_display_aliases",
            ],
        );
    }
}

#[test]
fn mono_and_mir_do_not_rediscover_methods_from_type_names() {
    for path in ["mono", "mir"] {
        assert_production_tree_absent(
            path,
            &[
                "type_names_for_method_lookup",
                "type_name_for_method_lookup",
                "impl_matches_method_lookup_type",
                "lookup_type_names_for_receiver",
                "method_name_for_target",
                "callable_for_field_method",
                "legacy_lookup",
            ],
        );
    }
}

#[test]
fn post_resolution_phases_do_not_resolve_item_ids_from_strings() {
    for path in [
        "hir",
        "infer",
        "selection",
        "mono",
        "mir",
        "codegen",
        "products",
        "crate_artifact",
        "crate_system",
    ] {
        assert_production_tree_absent(path, &["resolve_item_id(", "resolve_item_def_id("]);
    }
    for path in ["dce.rs", "products.rs"] {
        assert_production_file_absent(path, &["resolve_item_id(", "resolve_item_def_id("]);
    }
}

#[test]
fn compiler_recognized_sized_identity_is_not_string_based() {
    for path in production_rust_source_files_under("") {
        if is_audit_source(&path) {
            continue;
        }
        assert_production_file_absent(&path, &["\"std::sized::Sized\"", "\"Sized\""]);
    }
}

#[test]
fn ast_receiver_mode_stays_at_syntax_and_lowering_boundaries() {
    for path in [
        "hir",
        "infer",
        "selection",
        "mono",
        "mir",
        "codegen",
        "products",
        "crate_artifact",
        "crate_system",
    ] {
        assert_production_tree_absent(path, &["SelfReceiverMode"]);
    }
    for path in ["dce.rs", "products.rs"] {
        assert_production_file_absent(path, &["SelfReceiverMode"]);
    }
}

#[test]
fn dce_reachability_does_not_select_instances_by_display_or_backend_names() {
    let source = production_source("dce.rs");
    assert_absent(
        &source,
        &[
            ".source_name",
            ".backend_symbol",
            "function_by_name",
            "names.functions_by_name",
        ],
    );
}

#[test]
fn mir_unresolved_callables_do_not_use_hir_display_name_tables() {
    let source = production_tree_source("mir/builder");
    assert_absent(
        &source,
        &[
            "fn callable_for_named_value(&self, name: &str)",
            "fn impl_method_for_named_value(&self, name: &str)",
        ],
    );
    let callable_for_expr = function_body(
        &source,
        "fn callable_for_expr(&self, expr: &HirExpr, ret_ty: &Type)",
    );
    assert_absent(
        &callable_for_expr,
        &[
            "function_by_name",
            "externs_by_name",
            "names.",
            ".backend_symbol",
        ],
    );
}

#[test]
fn mir_def_id_callable_map_contains_only_function_origins() {
    let source = production_tree_source("mir/builder");
    let body = function_body(&source, "fn callable_instances_by_def_id_for_program(");
    assert_absent(
        &body,
        &["InstanceOrigin::ImplMethod", "InstanceOrigin::TraitDefault"],
    );
}

#[test]
fn hir_name_tables_are_not_read_for_production_semantics() {
    let hir_source = production_tree_source("hir");
    assert_absent(
        &hir_source,
        &[
            "pub fn function_by_name(&self, name: &str)",
            "pub fn struct_by_name(&self, name: &str)",
            "pub fn enum_by_name(&self, name: &str)",
            "pub fn trait_by_name(&self, name: &str)",
        ],
    );

    for path in [
        "mono/process.rs",
        "mono/external.rs",
        "mono/mod.rs",
        "mir/builder/mod.rs",
        "dce.rs",
        "products.rs",
        "codegen/mod.rs",
    ] {
        let source = production_source(path);
        assert_absent(
            &source,
            &[
                "function_by_name(",
                "struct_by_name(",
                "enum_by_name(",
                "trait_by_name(",
                "extern_by_name(",
                "names.functions_by_name.get",
                "names.structs_by_name.get",
                "names.enums_by_name.get",
                "names.traits_by_name.get",
                "names.externs_by_name.get",
                "names.functions_by_name[",
                "names.structs_by_name[",
                "names.enums_by_name[",
                "names.traits_by_name[",
                "names.externs_by_name[",
                "contains_key(\"main\")",
            ],
        );
    }
}

#[test]
fn mono_does_not_store_function_alias_views() {
    for path in ["mono/mod.rs", "mono/process.rs", "mono/external.rs"] {
        assert_production_file_absent(path, &["function_aliases", "register_function_aliases"]);
    }
}
