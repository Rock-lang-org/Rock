use inkwell::context::Context;
use inkwell::OptimizationLevel;

use crate::diagnostic::{self, Diagnostics};
use crate::products::fingerprint::product_source_fingerprint;
use crate::products::identity::{
    product_dependencies_from_config, product_dependency_crate_identities_from_context,
};
use crate::products::link::{attach_product_link_records, attach_product_object_path};
use crate::products::{CompilerProducts, ProductCrateIdentity, ProductLinkData};
use crate::{analysis, codegen, dce, mir, mono};

use super::frontend::{compile_frontend, FrontendOutput};
use super::{module_name_from_entry, CompileOutput, Config, DebugPrint};

pub(super) fn compile_impl(
    config: &Config,
    emit_products: bool,
) -> Result<CompileOutput, Diagnostics> {
    let FrontendOutput {
        analysis: analysis::Analysis { ast, hir },
        crate_ctx: ctx,
        diagnostic_sources,
        loaded_prelude_export_ids,
        infix_precedence,
        infix_owners,
        fingerprint_source_files,
        source_db,
        effective_current_crate_name,
        exported_macros,
        prelude_macros,
    } = compile_frontend(config)?;
    let crate_ctx = &ctx;

    let source_fingerprint =
        product_source_fingerprint(config, &source_db, &fingerprint_source_files);
    let current_crate_identity = ProductCrateIdentity::local(effective_current_crate_name.clone())
        .with_source_fingerprint(source_fingerprint.clone());
    let compilation_identity = mono::CompilationIdentityContext::new(
        hir.root_crate_id,
        current_crate_identity.clone(),
        crate_ctx
            .product_crate_ids
            .iter()
            .map(|(identity, crate_id)| (*crate_id, identity.clone()))
            .collect(),
    );

    let (mut products, product_id_remap) = if emit_products {
        let (mut products, product_id_remap) = CompilerProducts::from_resolved_hir_with_remap(
            current_crate_identity,
            &hir,
            product_dependencies_from_config(config),
            product_dependency_crate_identities_from_context(crate_ctx),
            source_fingerprint,
            ProductLinkData::default(),
        )
        .map_err(|error| {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(diagnostic::Diagnostic::for_internal(error));
            diagnostics
        })?;
        if config.current_crate_name.as_deref() == Some("std") {
            products.record_prelude_export_ids(
                loaded_prelude_export_ids
                    .iter()
                    .map(|(alias, export)| (alias.clone(), export.clone())),
            );
        }
        products.infix_precedence = infix_precedence
            .into_iter()
            .map(|(name, precedence)| {
                let declaring_crate = infix_owners
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| effective_current_crate_name.clone());
                let operator = crate::products::ProductInfixOperator {
                    precedence,
                    declaring_crate,
                };
                (name, operator)
            })
            .collect();
        products.interface.macros = exported_macros;
        products.interface.prelude_macros = prelude_macros;
        (Some(products), Some(product_id_remap))
    } else {
        (None, None)
    };

    // Phase 5: Monomorphization
    let mut monomorphized = mono::monomorphize_with_crates(hir, crate_ctx, compilation_identity)
        .map_err(|diagnostics| diagnostics.with_sources(&diagnostic_sources))?;
    let mut instance_bodies =
        mir::builder::MirBuilder::take_mir_instance_bodies(&mut monomorphized);
    let _dce_report = dce::prune_unreachable_instances(&mut monomorphized, &mut instance_bodies);

    let mir_program = mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(
        &monomorphized,
        &instance_bodies,
    );
    if config.has_debug_print(DebugPrint::Mir) {
        println!("{:#?}", mir_program);
    }
    let mut type_names = crate::type_services::display::TypeDisplayContext::default();
    type_names.extend_nominal_names(&monomorphized.program);
    if let Err(diagnostics) = mir::borrowck::BorrowChecker::run(&mir_program, &type_names) {
        return Err(diagnostics.with_sources(&diagnostic_sources));
    }
    let agreement = mir::agreement::check_mir_runtime_agreement(&mir_program);
    if !agreement.is_clean() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.push(diagnostic::Diagnostic::for_internal(format!(
            "MIR/codegen agreement failed: {:?}",
            agreement
        )));
        return Err(diagnostics);
    }

    // Phase 6: Code generation
    let context = Context::create();
    let module_name = module_name_from_entry(&config.entry_file);
    let symbol_namespace = config
        .current_crate_name
        .as_deref()
        .unwrap_or(module_name.as_str());

    let mut codegen =
        codegen::CodeGen::new_with_symbol_namespace(&context, &module_name, symbol_namespace);
    codegen.set_type_context(mir_program.type_context.clone());
    let link_inputs = crate_ctx.dependency_link_inputs();

    if let Err(e) = codegen.compile_program_from_mir(&mir_program) {
        let mut diagnostics = Diagnostics::default();
        diagnostics.push(e.into_diagnostic());
        let diagnostics = diagnostics.with_sources(&diagnostic_sources);
        return Err(diagnostics);
    }
    attach_product_link_records(
        &mut products,
        &mir_program.backend_contract.artifact_exports,
        codegen.mir_contract_symbol_overrides(),
        product_id_remap.as_ref(),
    )
    .map_err(|error| {
        let mut diagnostics = Diagnostics::default();
        diagnostics.push(diagnostic::Diagnostic::for_internal(error));
        diagnostics
    })?;

    if config.has_debug_print(DebugPrint::Llvm) {
        println!("{}", codegen.get_ir());
    }

    // Create output directory
    let _ = std::fs::create_dir_all(&config.output_dir);

    // Write LLVM IR if requested
    if config.emit_llvm {
        let ir_path = config.output_dir.join(format!("{}.ll", module_name));
        if let Err(e) = codegen.write_ir(&ir_path) {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(diagnostic::Diagnostic::for_file(
                format!("Failed to write LLVM IR: {}", e),
                ir_path,
            ));
            return Err(diagnostics);
        }
    }

    // Write executable
    let opt = match config.opt_level {
        0 => OptimizationLevel::None,
        1 => OptimizationLevel::Less,
        2 => OptimizationLevel::Default,
        _ => OptimizationLevel::Aggressive,
    };

    let exe_name = if cfg!(target_os = "windows") {
        format!("{}.exe", module_name)
    } else {
        module_name.to_string()
    };
    let exe_path = config.output_dir.join(&exe_name);

    if config.no_link {
        let obj_path = config
            .emit_object
            .clone()
            .unwrap_or_else(|| config.output_dir.join(format!("{}.o", module_name)));
        if let Some(parent) = obj_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = codegen.write_object(&obj_path, opt) {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(e.into_diagnostic());
            return Err(diagnostics);
        }
        attach_product_object_path(&mut products, &obj_path);
    } else {
        if let Err(e) = codegen.write_executable(&exe_path, opt, &link_inputs.object_paths) {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(e.into_diagnostic());
            return Err(diagnostics);
        }
    }

    Ok(CompileOutput { ast, products })
}
