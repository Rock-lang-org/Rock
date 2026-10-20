//! Program-level lowering: entry points, module handling, and body lowering

use crate::ast;
use crate::crate_system::CrateContext;

use crate::lower::error::ResolveError;
use crate::lower::module_context::{ModuleLoweringContext, SourceModuleResolver};
use crate::lower::pipeline::LoweringPipeline;
use crate::lower::resolution::LowerResolutionContext;
use crate::lower::{collect_exports, path_names, Lowerer};

#[allow(dead_code)]
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

    /// Handle import statements: bring a qualified name into scope as an unqualified alias
    pub(crate) fn handle_import(&mut self, path: &ast::Path) {
        let names = match path {
            ast::Path::Ident(ip) => path_names(&ip.path),
            ast::Path::Type(tp) => path_names(&tp.path),
        };

        if names.is_empty() {
            return;
        }

        let qualified_name = LowerResolutionContext::new(self)
            .canonicalize_first_path_segment(&names)
            .join("::");
        let qualified_name = self
            .modules
            .current_module_prefix()
            .map(|prefix| qualify_module_local_import(self, &prefix, &qualified_name))
            .unwrap_or(qualified_name);
        let short_name = names.last().unwrap().clone();

        self.import_qualified_name(short_name, qualified_name);
    }

    fn import_qualified_name(&mut self, short_name: String, qualified_name: String) {
        use crate::types::Type;

        let resolved_id =
            LowerResolutionContext::new(self).resolve_item_id_for_path(&qualified_name);
        let resolved_qualified_name = resolved_id
            .and_then(|id| {
                LowerResolutionContext::new(self)
                    .canonical_name(id)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| qualified_name.clone());

        // Check if the qualified name maps to a known function
        if let Some(func) = resolved_id.and_then(|id| self.items.function(id)) {
            if resolved_id.is_some_and(|id| {
                LowerResolutionContext::new(self).current_function_owns_import_name(&short_name, id)
            }) {
                return;
            }

            let param_types: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
            let func_type = Type::function_with_safety(
                param_types,
                func.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            );
            self.scope
                .define_alias(short_name.clone(), func_type, false);
            if let Some(id) = resolved_id {
                self.resolver.insert_import_alias_with_name(
                    short_name,
                    resolved_qualified_name,
                    id,
                );
            }
            return;
        }

        if let Some(extern_def) = resolved_id.and_then(|id| self.items.extern_def(id)) {
            let func_type = Type::function_with_safety(
                extern_def.params.clone(),
                extern_def.ret.clone(),
                crate::types::FunctionSafety::from_is_unsafe(extern_def.is_unsafe),
            );
            self.scope
                .define_alias(short_name.clone(), func_type, false);
            if let Some(id) = resolved_id {
                self.resolver.insert_import_alias_with_name(
                    short_name,
                    resolved_qualified_name,
                    id,
                );
            }
            return;
        }

        // Check scope for the qualified name
        if let Some(binding) = self.scope.lookup(&resolved_qualified_name) {
            let resolved_id = LowerResolutionContext::new(self)
                .resolve_item_id_for_path(&resolved_qualified_name);
            if resolved_id.is_some_and(|id| {
                LowerResolutionContext::new(self).current_function_owns_import_name(&short_name, id)
            }) {
                return;
            }

            let ty = binding.ty.clone();
            self.scope.define_alias(short_name.clone(), ty, false);
            if let Some(id) = resolved_id {
                self.resolver.insert_import_alias_with_name(
                    short_name,
                    resolved_qualified_name,
                    id,
                );
            }
            return;
        }

        // Check structs
        if resolved_id.is_some_and(|id| self.items.structure(id).is_some()) {
            // Import just makes the short name usable as a type
            if let Some(id) = resolved_id {
                self.resolver.insert_import_alias_with_name(
                    short_name,
                    resolved_qualified_name,
                    id,
                );
            }
            return;
        }

        // Check enums
        if resolved_id.is_some_and(|id| self.items.enumeration(id).is_some()) {
            if let Some(id) = resolved_id {
                self.resolver.insert_import_alias_with_name(
                    short_name,
                    resolved_qualified_name,
                    id,
                );
            }
            return;
        }

        // Check traits
        if resolved_id.is_some_and(|id| self.items.trait_def(id).is_some()) {
            if let Some(id) = resolved_id {
                self.resolver.insert_import_alias_with_name(
                    short_name,
                    resolved_qualified_name,
                    id,
                );
            }
        }
    }

    pub(crate) fn handle_glob_import(&mut self, module_path: &[String]) {
        let module_label = format!("{}::*", module_path.join("::"));
        match self.glob_import_targets(module_path) {
            Ok(targets) => {
                let artifact_targets = LowerResolutionContext::new(self)
                    .resolve_glob_import_targets(module_path)
                    .unwrap_or_default();
                for target in &artifact_targets {
                    if let Some(id) = target.id {
                        self.resolver.insert_import_alias_with_name(
                            target.short_name.clone(),
                            target.source.clone(),
                            id,
                        );
                    }
                }

                for (short_name, qualified_name) in targets {
                    self.import_qualified_name(short_name, qualified_name);
                }
            }
            Err(err) => self
                .diagnostics
                .push_toolchain(format!("Failed to import {}: {}", module_label, err)),
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
mod tests {
    use super::*;

    use crate::ast::{Ident, Module};
    use crate::hir::{HirBlock, HirEnum, HirFunction, HirStruct, HirTrait};
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::lexer::Span;
    use crate::source_loader::LoadedModule;
    use crate::types::Type;

    fn test_function(id: DefId, name: &str) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: std::collections::HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::I64,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn test_struct(id: DefId, name: &str) -> HirStruct {
        HirStruct {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        }
    }

    fn test_enum(id: DefId, name: &str) -> HirEnum {
        HirEnum {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            variants: Vec::new(),
        }
    }

    fn test_trait(id: DefId, name: &str) -> HirTrait {
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: std::collections::HashMap::new(),
            signatures: std::collections::HashMap::new(),
        }
    }

    fn loaded_module(
        qualified_name: &str,
        path: std::path::PathBuf,
        module: Module,
    ) -> LoadedModule {
        LoadedModule {
            id: crate::source_loader::ModuleId(0),
            qualified_name: qualified_name.to_string(),
            path: path.clone(),
            canonical_path: path,
            module,
        }
    }

    #[test]
    fn source_module_resolver_requires_seeded_module_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_requires_seeded_module_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let root = temp_dir.join("main.rk");

        let mut lowerer = Lowerer::new_for_test();
        lowerer.modules.set_current_module_path(root);

        let error = SourceModuleResolver::new(&mut lowerer.modules)
            .load_local_module("util")
            .expect_err("module cache should not bypass graph-seeded module paths");

        assert!(error.contains("was not loaded by the source database"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn import_alias_does_not_remove_current_crate_function_with_same_short_name() {
        let mut lowerer = Lowerer::new_for_test();
        let local_id = DefId::new(CrateId(0), LocalDefId(1));
        let imported_id = DefId::new(CrateId(7), LocalDefId(1));
        lowerer.current_def_ids.insert(local_id);
        lowerer
            .resolver
            .item_paths
            .insert("answer".to_string(), local_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(local_id, "answer".to_string());
        lowerer
            .resolver
            .item_paths
            .insert("dep::answer".to_string(), imported_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(imported_id, "dep::answer".to_string());
        lowerer
            .items
            .insert_function(test_function(local_id, "answer"));
        lowerer
            .items
            .insert_function(test_function(imported_id, "answer"));

        lowerer.import_qualified_name("answer".to_string(), "dep::answer".to_string());

        assert_eq!(
            lowerer.items.function(local_id).map(|func| func.id),
            Some(local_id)
        );
        assert_eq!(
            lowerer.resolver.resolve_item_or_alias("answer"),
            Some(local_id)
        );
    }

    #[test]
    fn import_struct_registers_resolver_alias_metadata() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = DefId::new(CrateId(7), LocalDefId(2));
        lowerer
            .items
            .insert_structure(test_struct(struct_id, "dep::Thing"));
        lowerer
            .resolver
            .item_paths
            .insert("dep::Thing".to_string(), struct_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(struct_id, "dep::Thing".to_string());

        lowerer.import_qualified_name("Thing".to_string(), "dep::Thing".to_string());

        assert_eq!(
            lowerer.resolver.import_aliases.get("Thing").copied(),
            Some(struct_id)
        );
        assert_eq!(
            LowerResolutionContext::new(&lowerer)
                .resolve_struct_type("Thing")
                .map(|structure| structure.id),
            Some(struct_id)
        );
    }

    #[test]
    fn import_enum_and_trait_register_resolver_alias_metadata() {
        let mut lowerer = Lowerer::new_for_test();
        let enum_id = DefId::new(CrateId(7), LocalDefId(3));
        let trait_id = DefId::new(CrateId(7), LocalDefId(4));
        lowerer
            .items
            .insert_enumeration(test_enum(enum_id, "dep::Choice"));
        lowerer
            .items
            .insert_trait_def(test_trait(trait_id, "dep::Show"));
        lowerer
            .resolver
            .item_paths
            .insert("dep::Choice".to_string(), enum_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(enum_id, "dep::Choice".to_string());
        lowerer
            .resolver
            .item_paths
            .insert("dep::Show".to_string(), trait_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(trait_id, "dep::Show".to_string());

        lowerer.import_qualified_name("Choice".to_string(), "dep::Choice".to_string());
        lowerer.import_qualified_name("Show".to_string(), "dep::Show".to_string());

        assert_eq!(
            lowerer.resolver.import_aliases.get("Choice").copied(),
            Some(enum_id)
        );
        assert_eq!(
            lowerer.resolver.import_aliases.get("Show").copied(),
            Some(trait_id)
        );
        assert_eq!(
            LowerResolutionContext::new(&lowerer)
                .resolve_enum_type("Choice")
                .map(|enum_def| enum_def.id),
            Some(enum_id)
        );
        assert_eq!(
            LowerResolutionContext::new(&lowerer)
                .resolve_trait_type("Show")
                .map(|trait_def| trait_def.id),
            Some(trait_id)
        );
    }

    #[test]
    fn source_module_resolver_prefers_current_crate_prefixed_graph_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_prefers_current_crate_graph_path_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let alias_io = temp_dir.join("alias_io.rk");
        let graph_io = temp_dir.join("graph_io.rk");

        let alias_module = Module {
            name: Some(Ident { context: Default::default(),
                name: "io".to_string(),
                span: Span::test(),
            }),
            top_levels: vec![],
            is_inline: false,
            filepath: Some(alias_io.clone()),
        };
        let graph_module = Module {
            name: Some(Ident { context: Default::default(),
                name: "io".to_string(),
                span: Span::test(),
            }),
            top_levels: vec![],
            is_inline: false,
            filepath: Some(graph_io.clone()),
        };
        let mut lowerer = Lowerer::new_for_test();
        lowerer.modules = crate::lower::services::LowerModuleService::from_source_modules(vec![
            loaded_module("math::io", alias_io.clone(), alias_module),
            loaded_module("test::math::io", graph_io.clone(), graph_module),
        ]);
        lowerer
            .modules
            .set_current_crate_name(Some("test".to_string()));
        lowerer
            .modules
            .set_current_qualified_module_prefix(Some("math".to_string()));

        let module = SourceModuleResolver::new(&mut lowerer.modules)
            .load_local_module("io")
            .expect("current-crate-prefixed graph path should resolve");

        assert_eq!(module.filepath.as_ref(), Some(&graph_io));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn source_module_resolver_requires_seeded_external_child_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_requires_seeded_external_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let dep_root = temp_dir.join("lib.rk");
        let util = temp_dir.join("util.rk");

        let module = Module {
            name: Some(Ident { context: Default::default(),
                name: "util".to_string(),
                span: Span::test(),
            }),
            top_levels: vec![],
            is_inline: false,
            filepath: Some(util.clone()),
        };
        let mut lowerer = Lowerer::new_for_test();
        lowerer.modules =
            crate::lower::services::LowerModuleService::from_source_modules(vec![loaded_module(
                "dep", dep_root, module,
            )]);

        let error = SourceModuleResolver::new(&mut lowerer.modules)
            .load_external_crate_module("util", "dep")
            .expect_err("external module cache should not bypass graph-seeded module paths");

        assert!(error.contains("was not loaded by the source database"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn public_lower_entrypoint_uses_indexed_current_def_ids() {
        let parsed = crate::parser::parse_string(
            "trait Show\n    @show: I64\n\nimpl Show for I64\n    @show = -> 1\n\nmain = -> 0\n",
            &crate::Config::default(),
        )
        .expect("program should parse");

        let partial = lower(&parsed).expect("public lower entrypoint should lower through collect");
        let show_id = partial.resolver.item_paths["Show"];
        let trait_sig_id = partial
            .traits
            .get(&show_id)
            .and_then(|trait_def| trait_def.signatures.get("show"))
            .map(|sig| sig.id)
            .expect("trait signature should have a DefId");
        let impl_method_id = partial
            .impls
            .values()
            .find_map(|imp| imp.methods.get("show"))
            .map(|method| method.id)
            .expect("impl method should have a DefId");

        assert!(partial.current_def_ids.contains(&trait_sig_id));
        assert!(partial.current_def_ids.contains(&impl_method_id));
    }
}

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

fn qualify_module_local_import(lowerer: &Lowerer, prefix: &str, qualified: &str) -> String {
    if prefix.is_empty() || qualified == prefix || qualified.starts_with(&format!("{}::", prefix)) {
        return qualified.to_string();
    }

    let root_prefix = prefix.split("::").next().unwrap_or(prefix);
    if qualified == root_prefix || qualified.starts_with(&format!("{}::", root_prefix)) {
        return qualified.to_string();
    }

    let first_segment = qualified.split("::").next().unwrap_or(qualified);
    if lowerer.modules.has_loaded_root_name(first_segment) {
        qualified.to_string()
    } else {
        format!("{}::{}", prefix, qualified)
    }
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
