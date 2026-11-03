use super::*;

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

    let codegen = production_sources(CODEGEN_MOD_FILES);
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
    let codegen = production_sources(CODEGEN_MOD_FILES);
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

    let builder = production_tree_source("mir/builder");
    let builder_runtime_requirements = function_body(
        &builder,
        "fn populate_backend_contract_runtime_requirements(",
    );
    assert_present(
        &builder_runtime_requirements,
        &["runtime_requirements_for_functions(functions.values())"],
    );

    let agreement = production_source("mir/agreement.rs");
    let agreement_runtime_requirements = function_body(
        &agreement,
        "fn validate_runtime_requirements(program: &MirProgram",
    );
    assert_present(
        &agreement_runtime_requirements,
        &[
            "runtime_requirements_for_functions(",
            ".difference(&program.backend_contract.runtime_requirements)",
            ".difference(&observed)",
        ],
    );

    let codegen = production_sources(CODEGEN_MOD_FILES);
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
            "runtime_requirements_for_functions(",
            "runtime requirement mismatch",
        ],
    );
}

#[test]
fn instance_body_backend_boundary_is_mir_native() {
    let pipeline = production_source("pipeline/backend.rs");
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

    let builder = production_tree_source("mir/builder");
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
    let codegen = production_sources(CODEGEN_MOD_FILES);
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
    let codegen = production_sources(CODEGEN_MOD_FILES);
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
