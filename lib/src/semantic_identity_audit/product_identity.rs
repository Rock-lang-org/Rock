use super::*;

#[test]
fn product_identity_remapping_does_not_fabricate_current_crate_def_ids() {
    let source = production_source("products.rs");
    for signature in [
        "pub fn from_resolved_hir(",
        "fn product_def_id_to_def_id(id: ProductDefId)",
        "fn remap_product_type_def_ids(",
        "fn remap_expr_location_product_ids<P: HirPhase>(",
        "fn remap_pattern_location_product_ids(",
    ] {
        let body = function_body(&source, signature);
        assert_absent(&body, &["CrateId(0)", "LocalDefId(0)"]);
    }
}

#[test]
fn product_artifacts_do_not_serialize_full_hir_metadata() {
    let products = production_source("products.rs");
    assert_absent(
        &products,
        &[
            "pub metadata: ProductMetadata",
            "pub struct ProductMetadata",
            "ProductInterface::from_metadata",
            "metadata.functions",
        ],
    );

    let type_table = production_source("products/type_table.rs");
    assert_absent(
        &type_table,
        &[
            "SerializedProductMetadata",
            "metadata: SerializedProductMetadata",
            "ProductInterface::from_metadata(&metadata)",
            "SerializedHirFunction::encode(&products.metadata",
        ],
    );
    assert_present(
        &type_table,
        &[
            "interface: SerializedProductInterface",
            "SerializedProductInterface::encode(&products.interface",
        ],
    );
}

#[test]
fn product_backend_symbols_are_link_record_owned() {
    assert_production_file_absent(
        "products.rs",
        &[
            "pub backend_symbols: BTreeMap<ProductDefId, String>",
            "identity_table.backend_symbols",
            "backend_symbols_from_link",
        ],
    );
    assert_production_file_absent(
        "pipeline/backend.rs",
        &[
            "identity_table.backend_symbols",
            "product_id_for_exported_backend_symbol",
            "exported_product_symbol",
        ],
    );
    assert_production_file_absent(
        "crate_artifact/load.rs",
        &["identity_table.backend_symbols", "identity backend symbol"],
    );
}

#[test]
fn external_object_symbols_do_not_fall_back_to_source_names() {
    assert_production_file_absent(
        "crate_system/extern_store.rs",
        &[
            concat!(".or_else(|| func.", "qualified_", "name.clone())"),
            concat!(".or_else(|| method.", "qualified_", "name.clone())"),
            "unwrap_or_else(|| interface_name.to_string())",
            "unwrap_or_else(|| format!(\"{}_{}\", imp.type_name, method_name))",
        ],
    );
    assert_production_file_absent(
        "mono/external.rs",
        &[
            concat!("method.", "qualified_", "name.clone()"),
            "unwrap_or_else(|| format!(\"{}_{}\", imp.type_name, method_name))",
        ],
    );
}

#[test]
fn hir_functions_do_not_own_qualified_backend_names() {
    let hir_source = production_tree_source("hir");
    let hir_function = function_body(&hir_source, "pub struct HirFunction");
    assert_absent(
        &hir_function,
        &[concat!("pub qualified_", "name: Option<String>")],
    );

    assert_production_file_absent(
        "products.rs",
        &[
            concat!("pub qualified_", "name: Option<String>"),
            concat!(
                "qualified_",
                "name: function.",
                "qualified_",
                "name.clone()",
            ),
        ],
    );
    assert_production_file_absent(
        "products/type_table.rs",
        &[
            concat!("pub qualified_", "name: Option<String>"),
            concat!("qualified_", "name: value.", "qualified_", "name.clone()",),
            concat!("qualified_", "name: self.", "qualified_", "name"),
        ],
    );
}

#[test]
fn collect_and_lower_do_not_construct_impl_backend_names() {
    for path in [
        "collect/context.rs",
        "collect/headers.rs",
        "collect/collector.rs",
        "lower/crates/bodies.rs",
        "lower/crates/registration.rs",
        "lower/resolution.rs",
        "lower/traits/conformance.rs",
        "lower/bodies.rs",
    ] {
        assert_production_file_absent(
            path,
            &[
                concat!("format_impl_", "backend_name"),
                "TypeName_methodName",
                concat!(".qualified_", "name"),
            ],
        );
    }

    assert_production_file_absent(
        "lower/bodies.rs",
        &["let mangled = format!(\"{}_{}\", type_name, method_name);"],
    );
    assert_production_file_absent(
        "lower/crates/bodies.rs",
        &[".insert(format!(\"{}_{}\", type_name, method_name), func.clone());"],
    );
    assert_production_file_absent(
        "lower/resolution.rs",
        &["name: format!(\"{}_{}\", type_name, method_name),"],
    );
}

#[test]
fn mono_and_artifacts_do_not_use_source_name_fields_for_identity_or_symbols() {
    for path in [
        "mono/mod.rs",
        "mono/methods.rs",
        "mono/specialize.rs",
        "mono/external.rs",
        "crate_artifact/types.rs",
        "crate_artifact/load.rs",
    ] {
        assert_production_file_absent(path, &[concat!(".qualified_", "name")]);
    }

    assert_production_file_absent(
        "mono/mod.rs",
        &[
            "return format!(\"{}_{}\", imp.type_name, method_name);",
            "unwrap_or_else(|| format!(\"{}_{}\", imp.type_name, method_name))",
        ],
    );

    assert_production_file_absent(
        "crate_artifact/load.rs",
        &[
            "format!(\"{}_{}\", imp.type_name, method_name)",
            "format!(\"{}_{}\", owner, method_name)",
            "format!(\"{}_{}\", short, method_name)",
        ],
    );
    assert_production_file_absent(
        "mir/builder/mod.rs",
        &["backend_symbol: format!(\"{}_{}\", imp.type_name, method_name)"],
    );
}
