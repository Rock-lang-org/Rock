use super::*;

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

    let builder = production_tree_source("mir/builder");
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
