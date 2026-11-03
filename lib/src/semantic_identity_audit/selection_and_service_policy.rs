use super::*;

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
            "ctx.has_extern_crate(\"std\")",
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

    let trait_call = function_body(&methods, "fn monomorphize_trait_method_call(");
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

    let builder = production_tree_source("mir/builder");
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

    let codegen = production_sources(CODEGEN_MOD_FILES);
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
