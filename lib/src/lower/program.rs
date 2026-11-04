//! Program-level lowering: entry points, module handling, and body lowering

use crate::ast;
use crate::crate_system::CrateContext;

use crate::lower::error::ResolveError;
use crate::lower::module_context::{ModuleLoweringContext, SourceModuleResolver};
use crate::lower::pipeline::LoweringPipeline;
use crate::lower::resolution::LowerResolutionContext;
use crate::lower::{collect_exports, Lowerer};

impl Lowerer {
    pub fn lower_program(
        self,
        program: &ast::Program,
    ) -> Result<crate::infer::PartialHir, Vec<ResolveError>> {
        self.lower_program_with_crates(program, None)
    }

    /// Lower a program with optional crate context
    pub fn lower_program_with_crates(
        self,
        program: &ast::Program,
        crate_ctx: Option<&CrateContext>,
    ) -> Result<crate::infer::PartialHir, Vec<ResolveError>> {
        let current_crate_name = self.modules.current_crate_name().map(ToString::to_string);
        if let Some(crate_ctx) = crate_ctx {
            lower_program_through_collection(
                program,
                crate_ctx,
                self.prelude.is_enabled(),
                current_crate_name.as_deref(),
            )
        } else {
            let crate_ctx = CrateContext::new();
            lower_program_through_collection(
                program,
                &crate_ctx,
                self.prelude.is_enabled(),
                current_crate_name.as_deref(),
            )
        }
    }

    pub(crate) fn glob_import_targets(
        &mut self,
        module_path: &[String],
    ) -> Result<Vec<(String, String)>, String> {
        if let Some(targets) =
            LowerResolutionContext::new(self).resolve_glob_import_targets(module_path)
        {
            return Ok(targets
                .into_iter()
                .map(|target| (target.short_name, target.source))
                .collect());
        }

        let (module, resolved_prefix) =
            SourceModuleResolver::new(&mut self.modules).load_module_by_path(module_path)?;
        let exports =
            self.expand_glob_exports_with_prefix(&resolved_prefix, collect_exports(&module));
        let mut targets = Vec::new();

        for (name, source) in exports {
            if name.ends_with("::*") {
                continue;
            }

            let qualified_name = source.map_or_else(
                || format!("{}::{}", resolved_prefix, name),
                |source| {
                    LowerResolutionContext::new(self)
                        .qualify_export_source(&resolved_prefix, &source)
                },
            );
            targets.push((name, qualified_name));
        }

        Ok(targets)
    }

    pub(crate) fn push_missing_cached_module_error(
        &mut self,
        module_name: &str,
        file_path: &std::path::Path,
        span: Option<crate::lexer::Span>,
    ) {
        let message = format!(
            "Module '{}' was not loaded by the source database; expected {}",
            module_name,
            file_path.display()
        );
        if self.diagnostics.has_message(&message) {
            return;
        }

        match span {
            Some(span) => self.diagnostics.push_with_span(message, span),
            None => self.diagnostics.push_toolchain(message),
        }
    }

    pub(crate) fn lower_module_bodies_qualified_impl(
        &mut self,
        module: &ast::Module,
        module_prefix: Option<&str>,
        module_id: crate::ids::ModuleId,
    ) {
        ModuleLoweringContext::with_qualified_module_context(
            self,
            module,
            module_prefix,
            |lowerer| {
                // Lower impl method bodies FIRST so their return types are resolved
                // before function bodies reference them (e.g. `x.println!`)
                for (ordinal, top_level) in module.top_levels.iter().enumerate() {
                    let ast::TopLevel::Impl(imp) = top_level else {
                        continue;
                    };
                    let Some(record) = lowerer.item_index.item_at_source(module_id, ordinal) else {
                        lowerer.diagnostics.push_toolchain(
                            "missing indexed impl declaration while lowering bodies".to_string(),
                        );
                        continue;
                    };
                    if !record.kind.matches_top_level(top_level) {
                        lowerer.diagnostics.push_toolchain(
                            "indexed declaration kind does not match impl syntax".to_string(),
                        );
                        continue;
                    }
                    lowerer.lower_impl_bodies(imp, record.def_id);
                }

                // Now lower function bodies (after impl methods are resolved)
                for (ordinal, top_level) in module.top_levels.iter().enumerate() {
                    let ast::TopLevel::FunctionDecl(fd) = top_level else {
                        continue;
                    };
                    let Some(record) = lowerer.item_index.item_at_source(module_id, ordinal) else {
                        lowerer.diagnostics.push_toolchain(
                            "missing indexed function declaration while lowering bodies"
                                .to_string(),
                        );
                        continue;
                    };
                    if !record.kind.matches_top_level(top_level) {
                        lowerer.diagnostics.push_toolchain(
                            "indexed declaration kind does not match function syntax".to_string(),
                        );
                        continue;
                    }
                    lowerer.lower_function_body_qualified(fd, record.def_id, module_prefix);
                }

                // Recurse into sub-modules (inline Module declarations and `mod name` files)
                for tl in &module.top_levels {
                    match tl {
                        ast::TopLevel::Module(module_decl) => {
                            let new_prefix = match (module_prefix, &module_decl.0.name) {
                                (Some(prefix), Some(name)) => {
                                    Some(format!("{}::{}", prefix, name.name))
                                }
                                (None, Some(name)) => Some(name.name.clone()),
                                (Some(prefix), None) => Some(prefix.to_string()),
                                (None, None) => None,
                            };

                            let new_prefix = match new_prefix {
                                Some(prefix) => prefix,
                                None => continue,
                            };
                            let Some(module_name) = module_decl.0.name.as_ref() else {
                                continue;
                            };
                            let Some(child_module_id) = lowerer
                                .item_index
                                .child_module_id(module_id, &module_name.name)
                            else {
                                lowerer.diagnostics.push_toolchain(format!(
                                    "missing indexed module '{}' while lowering bodies",
                                    new_prefix
                                ));
                                continue;
                            };
                            lowerer.lower_module_bodies_qualified_impl(
                                &module_decl.0,
                                Some(&new_prefix),
                                child_module_id,
                            );
                        }
                        ast::TopLevel::Mod(ident, _) => {
                            let Some(_child_module_id) =
                                lowerer.item_index.child_module_id(module_id, &ident.name)
                            else {
                                lowerer.diagnostics.push_toolchain(format!(
                                    "missing indexed module '{}' while lowering bodies",
                                    ident.name
                                ));
                                continue;
                            };
                            let new_prefix = match module_prefix {
                                Some(prefix) => format!("{}::{}", prefix, ident.name),
                                None => lowerer
                                    .modules
                                    .current_module_prefix()
                                    .map(|prefix| format!("{}::{}", prefix, ident.name))
                                    .unwrap_or_else(|| ident.name.clone()),
                            };
                            let Some(file_path) =
                                lowerer.modules.source_path_for_module_name(&new_prefix)
                            else {
                                lowerer.diagnostics.push_with_span(
                                    format!(
                                        "Module '{}' was not loaded by the source database",
                                        new_prefix
                                    ),
                                    ident.span.clone(),
                                );
                                continue;
                            };

                            match lowerer.modules.source_module_for_path(&file_path) {
                                // Source-backed modules are lowered once by the provider pass.
                                // The structural lookup above still validates this shell's exact ID.
                                Some(_) => {}
                                None => lowerer.push_missing_cached_module_error(
                                    &new_prefix,
                                    &file_path,
                                    Some(ident.span.clone()),
                                ),
                            }
                        }
                        _ => {}
                    }
                }
            },
        );
    }

    pub(crate) fn lower_loaded_module_trait_defaults(&mut self) {
        ModuleLoweringContext::for_each_loaded_module(
            self,
            |lowerer, module_id, module_name, loaded_module| {
                ModuleLoweringContext::with_module_local_aliases(
                    lowerer,
                    loaded_module,
                    module_name,
                    true,
                    true,
                    |lowerer| lowerer.lower_trait_default_bodies(loaded_module, module_id),
                );
            },
        );
    }
}

#[cfg(test)]
mod tests;

/// Main entry point for lowering
pub fn lower(program: &ast::Program) -> Result<crate::infer::PartialHir, Vec<ResolveError>> {
    let crate_ctx = CrateContext::new();
    lower_program_through_collection(program, &crate_ctx, true, None)
}

/// Lower a program with crate context
pub fn lower_with_crates(
    program: &ast::Program,
    crate_ctx: &CrateContext,
) -> Result<crate::infer::PartialHir, Vec<ResolveError>> {
    lower_with_crates_and_options(program, crate_ctx, true)
}

pub fn lower_with_crates_and_options(
    program: &ast::Program,
    crate_ctx: &CrateContext,
    inject_prelude: bool,
) -> Result<crate::infer::PartialHir, Vec<ResolveError>> {
    lower_program_through_collection(program, crate_ctx, inject_prelude, None)
}

fn lower_program_through_collection(
    program: &ast::Program,
    crate_ctx: &CrateContext,
    inject_prelude: bool,
    current_crate_name: Option<&str>,
) -> Result<crate::infer::PartialHir, Vec<ResolveError>> {
    let decls = crate::collect::collect(program, crate_ctx, inject_prelude, current_crate_name)?;
    lower_from_declarations(program, decls, crate_ctx, current_crate_name)
}

/// Lower a program using pre-collected declarations (new pipeline).
///
/// Takes `Declarations` from the collect stage and lowers only function bodies,
/// producing `PartialHir` for the infer stage.
pub fn lower_from_declarations(
    program: &ast::Program,
    decls: crate::collect::Declarations,
    crate_ctx: &CrateContext,
    current_crate_name: Option<&str>,
) -> Result<crate::infer::PartialHir, Vec<ResolveError>> {
    LoweringPipeline::new(program, crate_ctx, current_crate_name).lower_from_declarations(decls)
}
