use std::fs;
use std::path::{Path, PathBuf};

fn lib_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn production_source(path: &str) -> String {
    let source = fs::read_to_string(lib_src().join(path)).expect("source file should be readable");
    strip_cfg_test_items(
        source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap_or(source.as_str()),
    )
}

fn full_source(path: &str) -> String {
    fs::read_to_string(lib_src().join(path)).expect("source file should be readable")
}

fn rust_source_files_under(path: &str) -> Vec<String> {
    fn collect_rust_sources(dir: &Path, root: &Path, files: &mut Vec<String>) {
        for entry in fs::read_dir(dir).expect("source directory should be readable") {
            let entry = entry.expect("source directory entry should be readable");
            let path = entry.path();
            if path.is_dir() {
                collect_rust_sources(&path, root, files);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let relative = path
                    .strip_prefix(root)
                    .expect("source file should be under lib src");
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }

    let root = lib_src();
    let mut files = Vec::new();
    collect_rust_sources(&root.join(path), &root, &mut files);
    files.sort();
    files
}

fn production_rust_source_files_under(path: &str) -> Vec<String> {
    rust_source_files_under(path)
        .into_iter()
        .filter(|path| {
            !path.split('/').any(|component| component == "tests") && !path.ends_with("/tests.rs")
        })
        .collect()
}

fn assert_source_file_missing(path: &str) {
    assert!(
        !lib_src().join(path).exists(),
        "semantic identity audit expects `{path}` to be deleted"
    );
}

fn strip_cfg_test_items(source: &str) -> String {
    let mut output = String::new();
    let mut lines = source.lines();

    while let Some(line) = lines.next() {
        if line.trim() == "#[cfg(test)]" {
            let mut item = String::new();
            let mut depth = 0usize;
            let mut saw_open = false;

            for item_line in lines.by_ref() {
                item.push_str(item_line);
                item.push('\n');
                for ch in item_line.chars() {
                    match ch {
                        '{' => {
                            saw_open = true;
                            depth += 1;
                        }
                        '}' => depth = depth.saturating_sub(1),
                        _ => {}
                    }
                }
                if saw_open && depth == 0 {
                    break;
                }
                if !saw_open && item_line.trim_end().ends_with(';') {
                    break;
                }
            }
            continue;
        }

        output.push_str(line);
        output.push('\n');
    }

    output
}

fn function_body(source: &str, signature: &str) -> String {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("missing function signature: {signature}"));
    let source = &source[start..];
    let open = source
        .find('{')
        .unwrap_or_else(|| panic!("missing function body: {signature}"));
    let mut depth = 0usize;
    let mut end = None;

    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(open + offset + ch.len_utf8());
                    break;
                }
            }
            _ => {}
        }
    }

    source[..end.unwrap_or_else(|| panic!("unterminated function body: {signature}"))].to_string()
}

fn assert_absent(source: &str, forbidden: &[&str]) {
    for pattern in forbidden {
        assert!(
            !source.contains(pattern),
            "semantic identity audit forbids production pattern `{pattern}`"
        );
    }
}

fn assert_present(source: &str, required: &[&str]) {
    for pattern in required {
        assert!(
            source.contains(pattern),
            "semantic identity audit requires production pattern `{pattern}`"
        );
    }
}

fn assert_production_file_absent(path: &str, forbidden: &[&str]) {
    let source = production_source(path);
    assert_absent(&source, forbidden);
}

fn assert_full_source_file_absent(path: &str, forbidden: &[&str]) {
    let source = full_source(path);
    assert_absent(&source, forbidden);
}

fn assert_production_tree_absent(path: &str, forbidden: &[&str]) {
    for source_path in production_rust_source_files_under(path) {
        assert_production_file_absent(&source_path, forbidden);
    }
}

#[test]
fn legacy_backend_sidecar_and_adapter_tokens_are_absent() {
    for path in rust_source_files_under("") {
        if path == "semantic_identity_audit.rs" {
            continue;
        }
        assert_full_source_file_absent(&path, &["from_legacy_metadata"]);
    }
    for path in production_rust_source_files_under("") {
        if path == "semantic_identity_audit.rs" {
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
        if path == "semantic_identity_audit.rs" {
            continue;
        }
        assert_production_file_absent(&path, &["\"stdlib::sized::Sized\"", "\"Sized\""]);
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
    let source = production_source("mir/builder/mod.rs");
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
    let source = production_source("mir/builder/mod.rs");
    let body = function_body(&source, "fn callable_instances_by_def_id_for_program(");
    assert_absent(
        &body,
        &["InstanceOrigin::ImplMethod", "InstanceOrigin::TraitDefault"],
    );
}

#[test]
fn hir_name_tables_are_not_read_for_production_semantics() {
    let hir_source = production_source("hir/mod.rs");
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
        "lib.rs",
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
    let hir_source = production_source("hir/mod.rs");
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
                "stdlib_prelude_export_ids",
                "artifact_root_export_ids",
                "artifact_root_exports",
                "stdlib_prelude_exports",
                "capture_stdlib_exports",
                "inject_stdlib_prelude",
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

#[test]
fn mir_projection_resolution_metadata_uses_type_ids() {
    let mir_source = production_source("mir/mod.rs");
    assert_absent(
        &mir_source,
        &[
            "pub base: crate::types::Type",
            "pub trait_args: Vec<crate::types::Type>",
            "pub output: crate::types::Type",
        ],
    );

    let contract_source = production_source("mir/backend_contract.rs");
    assert_present(
        &contract_source,
        &[
            "pub struct MirProjectionKey",
            "pub projection_outputs: BTreeMap<MirProjectionKey, TypeId>",
        ],
    );
}

#[test]
fn mir_backend_contract_has_no_staging_or_partial_validation_path() {
    let mir = production_source("mir/mod.rs");
    assert_absent(
        &mir,
        &[
            "pub struct MirDropGlue",
            "pub struct MirProjectionResolution",
            "pub struct MirInstanceDeclaration",
            "pub struct MirProjectionImplMetadata",
            "pub struct MirExternDeclaration",
            "pub struct MirProductLinkCandidate",
            "pub struct MirStructLayout",
            "pub struct MirEnumLayout",
        ],
    );

    let builder = production_source("mir/builder/mod.rs");
    assert_absent(
        &builder,
        &[
            "struct_display_aliases",
            "enum_display_aliases",
            "canonical: false",
        ],
    );

    let builder = full_source("mir/builder/mod.rs");
    assert_absent(&builder, &["check_mir_runtime_agreement_for_mir_only"]);

    let agreement = full_source("mir/agreement.rs");
    assert_absent(
        &agreement,
        &[
            "check_mir_runtime_agreement_for_mir_only",
            "validate_backend_contract: bool",
            "nominal_layouts_required",
            "layout_checks_enabled",
        ],
    );
}

#[test]
fn alias_handling_does_not_clone_hir_semantic_owners() {
    let registration = production_source("lower/crates/registration.rs");
    assert_absent(
        &registration,
        &["fn sync_export_alias_functions", "source_func.body.clone()"],
    );

    let pipeline = production_source("lower/pipeline.rs");
    assert_absent(
        &pipeline,
        &[
            "sync_prelude_functions",
            "duplicate_aliases",
            "lowerer.items.functions.remove(&alias)",
        ],
    );

    let prelude = production_source("lower/prelude.rs");
    assert_absent(
        &prelude,
        &[
            "fn sync_prelude_functions",
            "items.functions.insert(short_name, func)",
            "items.structs.entry(short_name).or_insert(s)",
            "items.enums.entry(short_name).or_insert(e)",
            "items.traits.entry(short_name).or_insert(t)",
        ],
    );

    let collector = production_source("collect/collector.rs");
    let register_export_aliases = function_body(&collector, "fn register_export_aliases(");
    assert_absent(
        &register_export_aliases,
        &[
            ".get(&qualified_source).cloned()",
            ".functions\n                    .insert(qualified_export.clone(), func)",
            ".structs\n                    .insert(qualified_export.clone(), strukt)",
            "self.context.enums.insert(qualified_export.clone(), enum_)",
            "self.context.traits.insert(qualified_export, trait_)",
        ],
    );

    let context = production_source("collect/context.rs");
    for signature in [
        "fn import_artifact_root_export(",
        "fn inject_prelude_alias(",
        concat!("fn import_", "qualified_", "name("),
    ] {
        let body = function_body(&context, signature);
        assert_absent(
            &body,
            &[
                concat!(".functions.get(", "qualified_", "name).cloned()"),
                concat!(".functions.get(&", "qualified_", "name).cloned()"),
                concat!(".functions.get(&canonical_", "qualified_", "name).cloned()"),
                concat!(".structs.get(", "qualified_", "name).cloned()"),
                concat!(".structs.get(&", "qualified_", "name).cloned()"),
                concat!(".structs.get(&canonical_", "qualified_", "name).cloned()"),
                concat!(".enums.get(", "qualified_", "name).cloned()"),
                concat!(".enums.get(&", "qualified_", "name).cloned()"),
                concat!(".enums.get(&canonical_", "qualified_", "name).cloned()"),
                concat!(".traits.get(", "qualified_", "name).cloned()"),
                concat!(".traits.get(&", "qualified_", "name).cloned()"),
                concat!(".traits.get(&canonical_", "qualified_", "name).cloned()"),
                ".functions.insert(short_name, func)",
                ".structs.insert(short_name, strukt)",
                ".enums.insert(short_name, enum_)",
                ".traits.insert(short_name, trait_)",
                ".functions.insert(short_name.clone(), func)",
                ".structs.entry(short_name).or_insert(strukt)",
                ".enums.entry(short_name).or_insert(enum_)",
                ".traits.entry(short_name).or_insert(trait_)",
            ],
        );
    }
}

#[test]
fn body_lowering_does_not_mutate_standalone_resolver_output() {
    let crates_bodies = production_source("lower/crates/bodies.rs");
    let inject_aliases =
        function_body(&crates_bodies, "pub(crate) fn inject_module_local_aliases(");
    assert_absent(
        &inject_aliases,
        &[
            "insert_module_alias_with_name",
            "previous_module_alias_id",
            "resolve_module_local_alias_target",
        ],
    );

    let module_context = production_source("lower/module_context.rs");
    let with_aliases = function_body(&module_context, "pub(crate) fn with_module_local_aliases");
    assert_absent(
        &with_aliases,
        &[
            "resolver.module_aliases.insert",
            "resolver.module_aliases.remove",
        ],
    );
}

#[test]
fn lowering_does_not_perform_global_resolution_fallbacks() {
    let resolution = production_source("lower/resolution.rs");
    let resolve_item_id = function_body(&resolution, "pub(crate) fn resolve_item_id(");
    assert_absent(
        &resolve_item_id,
        &[
            "current_module_prefix().and_then",
            "format!(\"{}::{}\", prefix, name)",
            "resolve_dependency_item_path",
            "prelude.export_id",
        ],
    );

    let resolve_item_id_for_path =
        function_body(&resolution, "pub(crate) fn resolve_item_id_for_path(");
    assert_absent(
        &resolve_item_id_for_path,
        &[
            "current_crate_name",
            "format!(\"{}::{}\", crate_name, name)",
        ],
    );

    let try_canonical_owner_path =
        function_body(&resolution, "pub(crate) fn try_canonical_owner_path(");
    assert_absent(
        &try_canonical_owner_path,
        &[
            "current_crate_name",
            "strip_prefix(&crate_prefix)",
            "rsplit(\"::\")",
            "format!(\"{}::{}\", prefix, name)",
        ],
    );

    let resolve_owner_id = function_body(&resolution, "pub(crate) fn resolve_owner_id(");
    assert_absent(&resolve_owner_id, &["prelude.export_id"]);

    let conformance = production_source("lower/traits/conformance.rs");
    let conformance_resolve_item_id = function_body(&conformance, "fn resolve_item_id(&self,");
    assert_absent(
        &conformance_resolve_item_id,
        &[
            "current_module_prefix",
            "format!(\"{}::{}\", prefix, name)",
            "resolve_dependency_item_path",
            "prelude.export_id",
        ],
    );
    assert_absent(&conformance, &["fn resolve_owner_id(&self,"]);
}

#[test]
fn lowerer_core_phase_state_is_service_owned() {
    let lowerer_source = production_source("lower/mod.rs");
    let lowerer_struct = function_body(&lowerer_source, "pub struct Lowerer");
    assert_present(
        &lowerer_struct,
        &[
            "pub(crate) engine: LowerInferenceService",
            "pub(crate) scope: LowerScopeService",
            "pub(crate) items: LowerItemService",
            "pub(crate) diagnostics: LowerDiagnosticService",
            "pub(crate) modules: LowerModuleService",
            "pub(crate) prelude: LowerPreludeService",
            "pub(crate) resolver: LowerResolverService",
            "pub(crate) dependency_resolvers: LowerDependencyResolverService",
            "pub(crate) constraint_store: LowerConstraintService",
        ],
    );
    assert_absent(
        &lowerer_struct,
        &[
            "pub(crate) services: LowererServices",
            "pub(crate) engine: InferenceEngine",
            "pub(crate) scope: Scope",
            "pub(crate) items: LowerItems",
            "pub(crate) diagnostics: LowerDiagnosticSink",
            "pub(crate) modules: ModuleLoweringContext",
            "pub(crate) prelude: PreludeImports",
            "pub(crate) resolver: ResolverTables",
            "pub(crate) dependency_resolvers: HashMap<String, ResolverTables>",
            "pub(crate) constraint_store: ConstraintStore",
        ],
    );

    let services_source = production_source("lower/services.rs");
    assert_present(
        &services_source,
        &[
            "pub(crate) struct LowererServices",
            "pub(crate) engine: LowerInferenceService",
            "pub(crate) scope: LowerScopeService",
            "pub(crate) diagnostics: LowerDiagnosticService",
            "pub(crate) modules: LowerModuleService",
            "pub(crate) resolver: LowerResolverService",
            "lower_service_wrapper!(LowerInferenceService, InferenceEngine)",
            "lower_service_wrapper!(LowerScopeService, Scope)",
            "lower_service_wrapper!(LowerResolverService, ResolverTables)",
        ],
    );
}

#[test]
fn body_lowering_state_is_context_owned() {
    let lowerer_source = production_source("lower/mod.rs");
    let lowerer_struct = function_body(&lowerer_source, "pub struct Lowerer");
    assert_absent(
        &lowerer_struct,
        &[
            "pub(crate) current_function:",
            "pub(crate) current_body_owner:",
            "pub(crate) current_generic_owner:",
            "pub(crate) current_generic_params:",
            "pub(crate) current_impl_bounds:",
            "pub(crate) in_unsafe:",
            "pub(crate) local_ids:",
        ],
    );

    let body_context = production_source("lower/body_context.rs");
    assert_present(
        &body_context,
        &[
            "function_name: String",
            "owner: DefId",
            "generic_owner: Option<DefId>",
            "generic_params: Vec<String>",
            "impl_bounds: HirGenericBounds",
            "in_unsafe: bool",
            "local_ids: IdGen<HirLocalId>",
            "scope: Scope",
        ],
    );
    assert_absent(
        &lowerer_source,
        &[
            "self.current_function =",
            "self.current_body_owner =",
            "self.current_generic_owner =",
            "self.current_generic_params =",
            "self.current_impl_bounds =",
            "self.in_unsafe =",
            "self.local_ids =",
        ],
    );
}

#[test]
fn dependency_and_prelude_policy_are_session_owned() {
    let pipeline = production_source("lower/pipeline.rs");
    assert_present(
        &pipeline,
        &[
            "LoweringSessionServices::new",
            ".prepare_lowerer(&mut lowerer)",
            "session.lower_dependency_trait_bodies(lowerer)",
            "session.lower_dependency_module_bodies(lowerer)",
        ],
    );
    assert_absent(
        &pipeline,
        &[
            "lowerer.register_crate_resolvers",
            "lowerer.lower_crate_trait_bodies(self.crate_ctx)",
            "lowerer.lower_crate_module_bodies(self.crate_ctx)",
            "fn apply_prelude_imports",
            "lowerer.prelude.inject_loaded_prelude",
        ],
    );

    let session = production_source("lower/session.rs");
    assert_present(
        &session,
        &[
            "pub(crate) struct LoweringSessionServices",
            "fn register_crate_resolvers",
            "fn apply_loaded_prelude",
            "ctx.dependency_errors_for_phase(\"lowering\")",
            "ctx.has_extern_crate(\"stdlib\")",
            "inject_loaded_prelude",
        ],
    );

    let registration = production_source("lower/crates/registration.rs");
    assert_present(&registration, &["pub(crate) struct LowerCrateRegistration"]);
    assert_absent(
        &registration,
        &[
            "impl Lowerer",
            "fn register_crate_functions(&mut self",
            "fn register_extern_crate(&mut self",
        ],
    );
}

#[test]
fn module_source_cache_is_provider_owned() {
    let module_context = production_source("lower/module_context.rs");
    assert_present(
        &module_context,
        &[
            "pub(crate) struct ModuleSourceProvider",
            "pub(crate) struct ModuleLoweringContext",
        ],
    );
    assert_absent(
        &module_context,
        &[
            "source_modules: SourceModuleSet",
            "SourceModuleResolver<'a> {\n    modules: &'a mut ModuleLoweringContext",
        ],
    );

    let services = production_source("lower/services.rs");
    assert_present(
        &services,
        &[
            "pub(crate) struct LowerModuleService",
            "ModuleSourceProvider",
            "pub(crate) fn from_source_modules",
        ],
    );
    assert_absent(
        &services,
        &["lower_service_wrapper!(LowerModuleService, ModuleLoweringContext)"],
    );
}

#[test]
fn trait_conformance_policy_is_service_owned() {
    let pipeline = production_source("lower/pipeline.rs");
    assert_present(&pipeline, &["TraitConformancePhase::run(lowerer)"]);
    assert_absent(
        &pipeline,
        &[
            "lowerer.auto_impl_sized()",
            "lowerer.check_trait_conformance()",
        ],
    );

    let conformance = production_source("lower/traits/conformance.rs");
    assert_present(
        &conformance,
        &[
            "pub(crate) struct TraitConformancePhase",
            "pub(crate) struct TraitConformanceOutput",
            "fn prepare_missing_default_method",
            "pub(crate) fn run(lowerer: &mut Lowerer)",
            "output.diagnostics",
        ],
    );
    assert_absent(
        &conformance,
        &[
            "impl Lowerer",
            "imp.methods.insert(method_name.to_string(), func.clone())",
            "errors: &'a mut Vec<ResolveError>",
        ],
    );
}

#[test]
fn dispatch_selection_policy_is_service_owned() {
    let selection = production_source("selection/service.rs");
    assert_present(
        &selection,
        &[
            "pub fn select_concrete_method",
            "pub fn select_concrete_method_matching",
            "pub fn select_required_trait_method",
            "pub fn select_unary_operator_method",
            "pub fn select_current_trait_method",
            "pub fn select_bound_method_preferring_non_ref_receiver",
            "pub fn select_required_index_method",
            "pub fn select_unresolved_generic_method",
            "pub fn select_unresolved_generic_required_index_method",
        ],
    );

    for path in [
        "lower/expression.rs",
        "lower/control_flow/secondary.rs",
        "lower/types_helpers/helpers.rs",
    ] {
        let source = production_source(path);
        assert_absent(
            &source,
            &[
                "find_matching_trait_impl",
                "find_matching_trait_impl_by_id",
                "self.items.impls.iter().find",
                "self.items.impls.iter().find_map",
            ],
        );
    }
}

#[test]
fn lowering_consumes_selected_method_outputs_without_rediscovery() {
    let secondary = production_source("lower/control_flow/secondary.rs");
    assert_present(
        &secondary,
        &[
            "Option<crate::selection::SelectedMethod>",
            "fn selected_method_call_types",
            "selected.substituted_params",
            "selected.return_type",
        ],
    );
    assert_absent(
        &secondary,
        &[
            "Option<(\n        HirExpr",
            "selected.impl_def",
            "Self::seed_receiver_substitution_from_impl",
            "let raw_ret_ty = method_func.ret_type.clone()",
        ],
    );

    let expression = production_source("lower/expression.rs");
    assert_absent(
        &expression,
        &[
            "Some((selected.function?, selected.impl_def, selected.target))",
            "if let Some((method_func, _impl_def, target))",
        ],
    );
}

#[test]
fn static_method_values_do_not_reconstruct_selected_authority() {
    let secondary = production_source("lower/control_flow/secondary.rs");
    assert_absent(
        &secondary,
        &[
            "fn static_method_target_for_callee(",
            "imp.type_generics.get(param.index as usize)",
            "method.generic_params.iter().position",
        ],
    );
}

#[test]
fn mono_trait_dispatch_consumes_selected_target_identity() {
    let methods = production_source("mono/methods.rs");
    assert_present(&methods, &["method_for_selected_target"]);

    let process = production_source("mono/process.rs");
    assert_present(&process, &["fn register_effective_trait_methods"]);

    let trait_call = function_body(&methods, "pub(super) fn monomorphize_trait_method_call(");
    assert_absent(
        &trait_call,
        &[
            "let mut lookup_type_names",
            "lookup_type_names_for_receiver",
            "for (_trait_id, impls) in trait_impls",
            "imp.methods.get(method_name)",
        ],
    );
}

#[test]
fn selection_matching_does_not_branch_on_receiver_display_strings() {
    let matching = production_source("selection/matching.rs");
    assert_absent(
        &matching,
        &[
            "type_names_for_method_lookup",
            "type_name_for_method_lookup",
            "impl_matches_method_lookup_type",
            "trait_ref_type_name_matches",
            "fixed_array_type_name_lengths_match",
        ],
    );

    let service = production_source("selection/service.rs");
    assert_absent(
        &service,
        &[
            "type_names_for_method_lookup_in_context",
            "type_name_for_method_lookup_in_context",
            "impl_matches_method_lookup_type",
        ],
    );
}

#[test]
fn mir_identity_contains_only_materialized_method_calls() {
    let mir = production_source("mir/identity.rs");
    assert_absent(
        &mir,
        &[
            "pub struct MirSelectedMethodMetadata",
            "pub struct MirBuiltinIndexGuard",
            "SelfReceiverMode",
            "ReceiverMode",
            "selected: MirSelectedMethodMetadata",
            "MirCallable::Method",
        ],
    );
    assert_absent(
        &mir,
        &[
            concat!("pub backend_", "symbol: Option<String>"),
            concat!("backend_", "symbol: None"),
        ],
    );

    let builder = production_source("mir/builder/mod.rs");
    assert_absent(
        &builder,
        &[
            "fn selected_method_metadata(",
            "builtin_index_guard_for_target",
            "receiver_adjustment: self_receiver",
            "fn method_name_for_target(",
            "fn callable_for_field_method(",
        ],
    );
    assert_present(
        &mir,
        &["pub enum MirCallable {\n    Resolved(MirCallableKey),\n}"],
    );

    let contract = production_source("mir/backend_contract.rs");
    assert_absent(&contract, &["UnresolvedCallableOperand"]);
    assert_absent(&builder, &[concat!("backend_", "symbol: None")]);

    let codegen = production_source("codegen/mod.rs");
    assert_present(
        &codegen,
        &[
            "let MirCallable::Resolved(key) = callable",
            "self.callable_symbols_by_key",
        ],
    );
    assert_absent(
        &codegen,
        &[
            "fn find_projection_metadata_impl(",
            ".projection_impl(&lookup_names",
            "let lookup_names = self.get_type_names_for_method(&resolved_recv_ty)",
            "let lookup_names = self.get_type_names_for_method(recv_ty)",
            "selected.builtin_index_guard.is_some()",
            concat!("selected.backend_", "symbol.as_ref()"),
            "function_symbols_by_id",
            "extern_symbols_by_id",
            "instance_symbols_by_id",
            "method_functions",
            "method_receiver_modes",
            "mod metadata;",
            "BackendSelectionMetadata",
        ],
    );
    assert_source_file_missing("codegen/metadata.rs");

    for path in [
        "codegen/closures.rs",
        "codegen/control_flow.rs",
        "codegen/expr/access.rs",
        "codegen/expr/aggregates.rs",
        "codegen/expr/call.rs",
        "codegen/expr/cast_assign.rs",
        "codegen/expr/mod.rs",
        "codegen/operators.rs",
        "codegen/stmt.rs",
    ] {
        assert_source_file_missing(path);
    }
}

#[test]
fn backend_forbidden_types_are_rejected_without_codegen_repairs() {
    let agreement = production_source("mir/agreement.rs");
    assert_present(
        &agreement,
        &[
            "Ty::Generic(param) if generic_params.contains(param) => {}",
            "| Ty::Constructor { .. }",
            "| Ty::Apply { .. }",
            "| Ty::Lambda { .. }",
            "| Ty::BoundVar { .. }",
            "| Ty::Error => report.invalid_type_ids += 1",
        ],
    );

    let types = production_source("codegen/types.rs");
    assert_present(
        &types,
        &[
            "backend-forbidden type reached LLVM lowering",
            "backend-forbidden type reached LLVM default-value lowering",
        ],
    );

    let intrinsics = production_source("codegen/intrinsics.rs");
    assert_absent(
        &intrinsics,
        &["Type::TypeVar(_) | Type::Generic(_) => self.context.i8_type().into()"],
    );

    let codegen = production_source("codegen/mod.rs");
    assert_absent(
        &codegen,
        &["| Type::TypeVar(_)", "| Type::Generic(_)", "| Type::Error"],
    );
}

#[test]
fn legacy_hir_codegen_modules_are_deleted_even_in_tests() {
    for path in [
        "codegen/closures.rs",
        "codegen/control_flow.rs",
        "codegen/error.rs",
        "codegen/metadata.rs",
        "codegen/operators.rs",
        "codegen/expr/mod.rs",
        "codegen/expr/access.rs",
        "codegen/expr/aggregates.rs",
        "codegen/expr/call.rs",
        "codegen/expr/cast_assign.rs",
        "codegen/stmt.rs",
    ] {
        assert_source_file_missing(path);
    }

    assert_full_source_file_absent(
        "codegen/mod.rs",
        &[
            "mod closures;",
            "mod control_flow;",
            "mod expr;",
            "mod metadata;",
            "mod operators;",
            "mod stmt;",
            "pub fn compile_program(&mut self, program: &MonomorphizedProgram)",
            "prepare_program_declarations",
            "register_instances",
            "register_nominal_layouts",
            "register_index_trait_targets",
            "codegen_variant_fields_from_hir",
            "compile_function(&",
            "compile_block(&",
            "compile_stmt(&",
            "compile_expr(&",
        ],
    );
}

#[test]
fn codegen_backend_does_not_import_hir_executable_types() {
    for path in rust_source_files_under("codegen") {
        assert_full_source_file_absent(
            &path,
            &[
                "use crate::hir",
                "crate::hir::",
                "crate::{hir",
                "hir::*",
                "hir::{",
                "HirProgram",
                "HirExpr",
                "HirStmt",
                "HirBlock",
                "HirFunction",
                "HirExtern",
                "HirImpl",
                "HirTrait",
                "HirStruct",
                "HirEnum",
                "HirMethodCallTarget",
                "MonomorphizedProgram",
                "PreMirInstanceBodies",
                "ProjectionImplMetadata::from_hir_impl",
                "ProductInterface::from_metadata",
            ],
        );
    }
}

#[test]
fn production_codegen_setup_consumes_mir_backend_contract() {
    let codegen = production_source("codegen/mod.rs");
    assert_present(
        &codegen,
        &[
            "fn prepare_mir_program_declarations(",
            "self.drop_glue_callables_by_type =",
            "register_mir_nominal_layouts(&mir.backend_contract)",
            "self.validate_mir_backend_contract(mir)?",
            "self.register_mir_backend_contract_callables(mir)",
            ".map_err(CodegenError::internalize)?",
        ],
    );
    assert_absent(
        &codegen,
        &[
            "pub(crate) fn prepare_program_declarations(",
            "fn prepare_program_declarations_inner(",
            "program: &MonomorphizedProgram",
            "let hir_program = &program.program",
            "register_nominal_layouts(hir_program)",
            "register_instances(instances",
            "register_impl_method_aliases(hir_program)",
            "BackendSelectionMetadata",
            "mod metadata;",
        ],
    );
    assert_source_file_missing("codegen/metadata.rs");
}

#[test]
fn runtime_requirements_are_derived_from_mir_and_consumed_by_codegen() {
    let backend_contract = production_source("mir/backend_contract.rs");
    let runtime_requirements = function_body(
        &backend_contract,
        "pub fn runtime_requirements_for_functions<'a>(",
    );
    assert_present(
        &runtime_requirements,
        &[
            "assertion.kind == MirAssertKind::BoundsCheck",
            "assertion.operands.len() == 2",
            "!closure.captures.is_empty()",
            "MirRuntimeHelper::BoundsCheck",
            "MirRuntimeHelper::HeapAlloc",
        ],
    );

    let builder = production_source("mir/builder/mod.rs");
    let backend_requirements = function_body(
        &backend_contract,
        "pub fn runtime_requirements_for_backend<'a>(",
    );
    assert_present(
        &backend_requirements,
        &[
            "runtime_requirements_for_functions(functions)",
            "contract.callables.values()",
            "descriptor.drop.requires_heap_free()",
            "MirRuntimeHelper::HeapFree",
        ],
    );
    let builder_runtime_requirements = function_body(
        &builder,
        "fn populate_backend_contract_runtime_requirements(",
    );
    assert_present(
        &builder_runtime_requirements,
        &["runtime_requirements_for_backend(contract, functions.values())"],
    );

    let agreement = production_source("mir/agreement.rs");
    let agreement_runtime_requirements = function_body(
        &agreement,
        "fn validate_runtime_requirements(program: &MirProgram",
    );
    assert_present(
        &agreement_runtime_requirements,
        &[
            "runtime_requirements_for_backend(",
            ".difference(&program.backend_contract.runtime_requirements)",
            ".difference(&observed)",
        ],
    );

    let codegen = production_source("codegen/mod.rs");
    let declare_runtime = function_body(
        &codegen,
        "fn declare_runtime(&mut self, runtime_requirements: &BTreeSet<MirRuntimeHelper>)",
    );
    assert_present(
        &declare_runtime,
        &[
            "runtime_requirements.contains(&MirRuntimeHelper::BoundsCheck)",
            "runtime_requirements.contains(&MirRuntimeHelper::HeapAlloc)",
        ],
    );
    let declarations = function_body(&codegen, "fn prepare_mir_program_declarations(");
    assert_present(
        &declarations,
        &["self.declare_runtime(&mir.backend_contract.runtime_requirements)"],
    );
    let validation = function_body(&codegen, "fn validate_mir_backend_contract(");
    assert_present(
        &validation,
        &[
            "runtime_requirements_for_backend(",
            "runtime requirement mismatch",
        ],
    );
}

#[test]
fn instance_body_backend_boundary_is_mir_native() {
    let pipeline = production_source("lib.rs");
    assert_present(
        &pipeline,
        &[
            "let mut instance_bodies =\n        mir::builder::MirBuilder::take_mir_instance_bodies(&mut monomorphized);",
            "dce::prune_unreachable_instances(&mut monomorphized, &mut instance_bodies)",
        ],
    );
    assert_absent(
        &pipeline,
        &["dce::prune_unreachable_instances(&mut monomorphized);"],
    );

    let dce = production_source("dce.rs");
    assert_present(
        &dce,
        &[
            "bodies: &mut MirInstanceBodies",
            "collect_instance_edges_mir_function(",
            "bodies.retain(|id, _| reachable.contains(&id));",
        ],
    );
    assert_absent(&dce, &["record.pre_mir_body()"]);

    let mir = production_source("mir/mod.rs");
    assert_present(&mir, &["pub fn retain<F>(&mut self, mut keep: F)"]);
    assert_present(
        &mir,
        &[
            "pub enum MirBinOp",
            "pub enum MirUnaryOp",
            "pub enum MirClosureCaptureKind",
        ],
    );
    assert_absent(
        &mir,
        &[
            "use crate::hir::{BinOp, UnaryOp};",
            "BinaryOp(BinOp",
            "UnaryOp(UnaryOp",
            "pub kind: crate::hir::HirClosureCaptureKind",
            "SelfReceiverMode",
            "ReceiverMode",
        ],
    );

    let builder = production_source("mir/builder/mod.rs");
    assert_present(
        &builder,
        &[
            "MirFunctionId::Instance(record.id)",
            "MirInstanceBody {",
            "function: mir_func",
        ],
    );
    assert_absent(&builder, &["record.pre_mir_body()"]);
}

#[test]
fn instance_records_do_not_own_executable_hir_bodies() {
    let registry = production_source("mono/registry.rs");
    assert_absent(
        &registry,
        &[
            "pub(crate) pre_mir_body: Option<PreMirInstanceBody>",
            "pub fn pre_mir_body(&self)",
            "pub fn take_pre_mir_body(&mut self)",
            "pub fn has_pre_mir_body(&self)",
        ],
    );
    assert_present(&registry, &["pub struct PreMirInstanceBodies"]);
}

#[test]
fn production_runtime_codegen_is_mir_only() {
    let codegen = production_source("codegen/mod.rs");
    assert_present(
        &codegen,
        &[
            "pub(crate) mod mir_llvm;",
            "mod intrinsics;",
            "pub(crate) fn compile_program_from_mir(",
            "mir_llvm::compile_mir_program(self, mir)",
        ],
    );
    assert_absent(
        &codegen,
        &[
            "mod closures;",
            "mod control_flow;",
            "mod expr;",
            "mod operators;",
            "mod stmt;",
            "pub fn compile_program(&mut self, program: &MonomorphizedProgram)",
            "self.compile_function(&record.symbols.backend_symbol, body)?",
        ],
    );

    let mir_llvm = production_source("codegen/mir_llvm/mod.rs");
    assert_present(
        &mir_llvm,
        &[
            "fn compile_mir_statement(",
            "StatementKind::Assign(place, rvalue)",
            "StatementKind::Assert(assertion)",
            "self.compile_mir_terminator(mir_function, terminator, context)",
            "error.with_operation_span(",
        ],
    );

    for path in [
        "codegen/mir_llvm/assert.rs",
        "codegen/mir_llvm/rvalue.rs",
        "codegen/mir_llvm/place.rs",
        "codegen/mir_llvm/terminator.rs",
    ] {
        assert_present(&production_source(path), &["compile_mir"]);
    }
}

#[test]
fn production_codegen_is_mechanical_mir_lowering_only() {
    let codegen = production_source("codegen/mod.rs");
    assert_absent(
        &codegen,
        &[
            "use crate::hir::*;",
            "fn get_type_names_for_method(",
            "nominal_type_names_for_method(",
            "HirMethodCallTarget",
        ],
    );

    assert_source_file_missing("codegen/metadata.rs");

    let terminator = production_source("codegen/mir_llvm/terminator.rs");
    assert_absent(
        &terminator,
        &[
            "fn drop_impl_backend_symbol(&self, place_ty: &Type)",
            "self.get_type_names_for_method(place_ty)",
            ".projection_impl(",
            "entry.trait_name.as_deref() == Some(\"Drop\")",
            "metadata.method_backend_symbol(\"drop\")",
            "SelfReceiverMode",
            "ReceiverMode",
            "method_receiver_modes",
        ],
    );

    let runtime = production_source("codegen/runtime.rs");
    assert_absent(
        &runtime,
        &[
            "use crate::hir::{BinOp, UnaryOp};",
            "SelfReceiverMode",
            "ReceiverMode",
        ],
    );
}
