use crate::ast;
use crate::collect::item_index::ModuleKind;
use crate::collect::Declarations;
use crate::crate_system::CrateContext;
use crate::infer::PartialHir;
use crate::lower::body_lowerer::BodyLowerer;
use crate::lower::module_context::ModuleLoweringContext;
use crate::lower::session::LoweringSessionServices;
use crate::lower::traits::conformance::TraitConformancePhase;
use crate::lower::{Lowerer, ResolveError};
use crate::traits::coherence::CoherencePhase;

pub(crate) struct LoweringPipeline<'a> {
    program: &'a ast::Program,
    crate_ctx: &'a CrateContext,
    current_crate_name: Option<&'a str>,
    overflow_checks: bool,
}

impl<'a> LoweringPipeline<'a> {
    pub(crate) fn new(
        program: &'a ast::Program,
        crate_ctx: &'a CrateContext,
        current_crate_name: Option<&'a str>,
    ) -> Self {
        Self {
            program,
            crate_ctx,
            current_crate_name,
            overflow_checks: true,
        }
    }

    pub(crate) fn with_overflow_checks(mut self, enabled: bool) -> Self {
        self.overflow_checks = enabled;
        self
    }

    pub(crate) fn lower_from_declarations(
        self,
        decls: Declarations,
    ) -> Result<PartialHir, Vec<ResolveError>> {
        let mut lowerer = Lowerer::from_declarations(decls)?;
        lowerer.overflow_checks = self.overflow_checks;
        let session = LoweringSessionServices::new(
            self.crate_ctx,
            self.current_crate_name,
            self.program.module.filepath.as_deref(),
        );
        session.prepare_lowerer(&mut lowerer);
        lowerer.refresh_type_display_context();
        self.prepare_traits(&session, &mut lowerer);
        self.lower_dependency_bodies(&session, &mut lowerer);
        self.lower_current_crate_bodies(&mut lowerer);
        self.finish(lowerer)
    }

    fn prepare_traits(&self, session: &LoweringSessionServices<'_>, lowerer: &mut Lowerer) {
        TraitConformancePhase::prepare_explicit_method_selection(lowerer);
        session.lower_dependency_trait_bodies(lowerer);
        let Some(root_module_id) = lowerer
            .item_index
            .modules()
            .iter()
            .find(|module| module.kind == ModuleKind::Root)
            .map(|module| module.module_id)
        else {
            lowerer.diagnostics.push_toolchain(
                "missing indexed root module while lowering trait default bodies".to_string(),
            );
            return;
        };
        ModuleLoweringContext::with_qualified_module_context(
            lowerer,
            &self.program.module,
            None,
            |lowerer| lowerer.lower_trait_default_bodies(&self.program.module, root_module_id),
        );
        lowerer.lower_loaded_module_trait_defaults();
        TraitConformancePhase::run(lowerer);
        CoherencePhase::run(lowerer);
    }

    fn lower_dependency_bodies(
        &self,
        session: &LoweringSessionServices<'_>,
        lowerer: &mut Lowerer,
    ) {
        session.lower_dependency_module_bodies(lowerer);
    }

    fn lower_current_crate_bodies(&self, lowerer: &mut Lowerer) {
        BodyLowerer::new(lowerer).lower_root_and_loaded_modules(&self.program.module);
    }

    fn finish(self, lowerer: Lowerer) -> Result<PartialHir, Vec<ResolveError>> {
        if lowerer.has_errors() {
            return Err(lowerer.diagnostics.into_inner().into_errors());
        }
        let import_aliases = lowerer
            .resolver
            .import_aliases
            .iter()
            .filter_map(|(alias, id)| {
                lowerer
                    .canonical_name_for_def_id(*id)
                    .map(|canonical| (alias.clone(), canonical.to_string()))
            })
            .collect();
        let loaded_module_paths = lowerer.modules.into_loaded_module_paths();
        let items = lowerer.items.into_inner().into_validated_declarations()?;
        let (functions, _, structs, enums, traits, impls, externs, type_aliases) =
            items.into_id_maps();

        Ok(PartialHir {
            functions,
            structs,
            enums,
            traits,
            impls,
            externs,
            type_aliases,
            engine: lowerer.engine.into_inner(),
            function_type_vars: lowerer.function_type_vars,
            import_aliases,
            loaded_module_paths,
            constraint_store: lowerer.constraint_store.into_inner(),
            resolver: lowerer.resolver.into_inner(),
            current_def_ids: lowerer.current_def_ids,
            root_crate_id: lowerer.root_crate_id,
            local_def_ids: lowerer.local_def_ids,
            language_items: lowerer.language_items,
            imported_effective_trait_methods: lowerer.imported_effective_trait_methods,
            inference_sccs: lowerer.inference_sccs.clone(),
            inference_scc_order: lowerer.inference_scc_order,
            source_map: lowerer.source_map,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{BTreeMap, HashMap};

    use crate::ast::Program;
    use crate::collect::resolver::ResolverTables;
    use crate::crate_artifact::{ArtifactCrateInterface, ArtifactExport};
    use crate::crate_system::CrateContext;
    use crate::crate_system::{
        ExternCrateBodies, ExternCrateLink, ExternCrateMetadata, ExternCrateRecord,
    };
    use crate::hir::{
        HirBlock, HirCallTarget, HirExpr, HirExprKind, HirExtern, HirFunction, HirImpl,
        HirImplOwner, HirLanguageItems, HirMethodCallTarget, HirStmt, HirTrait,
    };
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::language_items::SizedLanguageItems;
    use crate::source_loader::SourceDatabase;
    use crate::types::Type;

    fn test_function(id: DefId, name: &str) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
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

    fn test_extern(id: DefId) -> HirExtern {
        HirExtern {
            id,
            name: format!("extern{}", id.local.0),
            params: Vec::new(),
            ret: Type::Unit,
            variadic: false,
            is_unsafe: false,
        }
    }

    fn test_impl(id: DefId) -> HirImpl {
        HirImpl {
            id,
            owner: HirImplOwner::Named(format!("Impl{}", id.local.0)),
            type_name: format!("Impl{}", id.local.0),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        }
    }

    fn static_method_target(expr: &HirExpr) -> Option<&HirMethodCallTarget> {
        match &expr.kind {
            HirExprKind::Call(_, _, Some(HirCallTarget::StaticMethod(target))) => {
                Some(&target.method)
            }
            HirExprKind::Call(callee, _, _) => static_method_target(callee),
            HirExprKind::Lambda { body, .. } => {
                body.stmts.iter().find_map(|statement| match statement {
                    HirStmt::Expr(expression) => static_method_target(expression),
                    _ => None,
                })
            }
            _ => None,
        }
    }

    #[test]
    fn lowering_pipeline_preserves_deferred_impl_and_extern_ids() {
        let first = DefId::new(CrateId(0), LocalDefId(1));
        let second = DefId::new(CrateId(0), LocalDefId(2));
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_impl(test_impl(second)).unwrap();
        lowerer.items.insert_impl(test_impl(first)).unwrap();
        lowerer.items.insert_extern(test_extern(second));
        lowerer.items.insert_extern(test_extern(first));
        let parsed = crate::parser::parse_string("main = -> 0\n", &crate::Config::default())
            .expect("test program should parse");

        let lowered = LoweringPipeline::new(&parsed, &CrateContext::new(), None)
            .finish(lowerer)
            .expect("pipeline should finish deferred externs");

        assert!(lowered.impls.contains_key(&first));
        assert!(lowered.impls.contains_key(&second));
        assert!(lowered.externs.contains_key(&first));
        assert!(lowered.externs.contains_key(&second));
    }

    #[test]
    fn lowering_pipeline_lowers_from_declarations() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_pipeline_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let entry = temp_dir.join("main.rk");
        std::fs::write(
            &entry,
            "answer: I64\nanswer = -> 42\ncaller = ->\n    local = -> 1\n    local!\nmain: I64\nmain = -> answer!\n",
        )
        .unwrap();

        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = SourceDatabase::new();
        let graph = db.load_entry(entry, &config).unwrap();
        let program = Program {
            module: graph.root_module().clone(),
        };
        let crate_ctx = CrateContext::new();
        let decls = crate::collect::collect_with_source_graph(
            &program,
            &graph,
            &crate_ctx,
            false,
            Some("demo"),
        )
        .unwrap();

        let lowered = LoweringPipeline::new(&program, &crate_ctx, Some("demo"))
            .lower_from_declarations(decls)
            .expect("pipeline should lower function bodies");

        assert!(
            lowered.functions[&lowered.resolver.item_paths["main"]]
                .body
                .stmts
                .len()
                > 0,
            "pipeline should lower the root main body"
        );
        let answer_id = lowered
            .resolver
            .item_paths
            .get("demo::answer")
            .or_else(|| lowered.resolver.item_paths.get("answer"))
            .copied()
            .expect("answer function should be indexed");
        assert!(lowered
            .source_map
            .definition_declaration_span(answer_id)
            .is_some());
        assert!(lowered
            .source_map
            .references()
            .iter()
            .any(|reference| reference.target
                == crate::source_map::SourceSymbol::Definition(answer_id)));
        assert!(lowered
            .source_map
            .scopes()
            .iter()
            .any(|scope| scope.parent.is_some()));
        let caller_id = lowered
            .resolver
            .item_paths
            .get("caller")
            .or_else(|| lowered.resolver.item_paths.get("demo::caller"))
            .copied()
            .expect("caller function should be indexed");
        let caller_body = &lowered.functions[&caller_id].body;
        let local_id = match &caller_body.stmts[0] {
            crate::hir::HirStmt::Let { local_id, .. } => *local_id,
            statement => panic!("expected shadowing local binding, got {statement:?}"),
        };
        assert!(lowered.source_map.references().iter().any(|reference| {
            reference.target
                == crate::source_map::SourceSymbol::Local {
                    owner: caller_id,
                    local: local_id,
                }
                && reference.scope_id.is_some()
        }));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lowering_pipeline_preserves_renamed_sized_item_through_strict_finalization() {
        let parsed = crate::parser::parse_string(
            "lang sized\n< trait StaticLayout\n",
            &crate::Config {
                no_std: true,
                no_prelude: true,
                ..crate::Config::default()
            },
        )
        .expect("language item source should parse");
        let program = Program {
            module: parsed.module,
        };
        let crate_ctx = CrateContext::new();
        let declarations = crate::collect::collect(&program, &crate_ctx, false, Some("app"))
            .expect("renamed language item should collect");
        let trait_id = declarations
            .language_items
            .sized
            .as_ref()
            .expect("collection should bind the renamed Sized trait")
            .trait_id;

        let partial = LoweringPipeline::new(&program, &crate_ctx, Some("app"))
            .lower_from_declarations(declarations)
            .expect("renamed language item should lower");
        assert_eq!(
            partial
                .language_items
                .sized
                .as_ref()
                .map(|items| items.trait_id),
            Some(trait_id),
        );

        let resolved = crate::infer::finalize(partial)
            .expect("renamed language item should finalize strictly");
        assert_eq!(
            resolved
                .program
                .language_items
                .sized
                .as_ref()
                .map(|items| items.trait_id),
            Some(trait_id),
        );
    }

    #[test]
    fn lowering_pipeline_preserves_dependency_language_items_without_prelude_injection() {
        let sized_id = DefId::new(CrateId(7), LocalDefId(11));
        let mut interface = ArtifactCrateInterface::default();
        interface.insert_trait(
            "provider::StaticLayout".to_string(),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: sized_id,
                name: "StaticLayout".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        interface.root_export_ids.insert(
            "StaticLayout".to_string(),
            ArtifactExport {
                source: "StaticLayout".to_string(),
                id: sized_id,
            },
        );
        let resolver = ResolverTables {
            item_paths: HashMap::from([("provider::StaticLayout".to_string(), sized_id)]),
            item_names_by_id: HashMap::from([(sized_id, "provider::StaticLayout".to_string())]),
            ..ResolverTables::default()
        };
        let mut crate_ctx = CrateContext::new();
        crate_ctx
            .add_extern_crate(ExternCrateRecord::new(
                CrateId(7),
                "provider".to_string(),
                ExternCrateMetadata::new(interface, resolver, BTreeMap::new()).with_language_items(
                    HirLanguageItems {
                        sized: Some(SizedLanguageItems { trait_id: sized_id }),
                        ..HirLanguageItems::default()
                    },
                ),
                ExternCrateBodies::default(),
                ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .expect("dependency language item provider should register");
        let parsed = crate::parser::parse_string(
            "struct App\n",
            &crate::Config {
                no_std: true,
                no_prelude: true,
                ..crate::Config::default()
            },
        )
        .expect("application source should parse");
        let program = Program {
            module: parsed.module,
        };
        let declarations = crate::collect::collect(&program, &crate_ctx, false, Some("app"))
            .expect("dependency language item should collect without prelude injection");
        assert_eq!(
            declarations
                .language_items
                .sized
                .as_ref()
                .map(|items| items.trait_id),
            Some(sized_id),
        );

        let partial = LoweringPipeline::new(&program, &crate_ctx, Some("app"))
            .lower_from_declarations(declarations)
            .expect("dependency language item should lower");
        assert_eq!(
            partial
                .language_items
                .sized
                .as_ref()
                .map(|items| items.trait_id),
            Some(sized_id),
        );
        assert!(partial.traits.contains_key(&sized_id));

        let resolved = crate::infer::finalize(partial)
            .expect("dependency language item should finalize strictly");
        assert_eq!(
            resolved
                .program
                .language_items
                .sized
                .as_ref()
                .map(|items| items.trait_id),
            Some(sized_id),
        );
        assert!(resolved.program.program().traits.contains_key(&sized_id));
    }

    #[test]
    fn lowering_pipeline_keeps_static_impl_method_payload_only_under_its_impl() {
        let parsed = crate::parser::parse_string(
            "struct Math\n\nimpl Math\n    identity = value -> value\n\nmain = ->\n    direct = Math::identity 6\n    operation = Math::identity\n    operation direct\n",
            &crate::Config {
                no_std: true,
                no_prelude: true,
                ..crate::Config::default()
            },
        )
        .expect("static impl source should parse");
        let program = Program {
            module: parsed.module,
        };
        let crate_ctx = CrateContext::new();
        let declarations = crate::collect::collect(&program, &crate_ctx, false, None)
            .expect("static impl declarations should collect");

        let lowered = LoweringPipeline::new(&program, &crate_ctx, None)
            .lower_from_declarations(declarations)
            .expect("static impl bodies should lower");
        let (impl_id, impl_def) = lowered
            .impls
            .iter()
            .find(|(_, impl_def)| impl_def.type_name == "Math")
            .expect("Math impl should be lowered");
        let impl_id = *impl_id;
        let method = &impl_def.methods["identity"];
        let method_id = method.id;
        let main_id = lowered.resolver.item_paths["main"];

        assert_eq!(lowered.resolver.item_paths["Math::identity"], method_id);
        assert!(
            !lowered.functions.contains_key(&method_id),
            "static impl method payload must not be duplicated in PartialHir.functions"
        );
        assert_eq!(
            lowered
                .functions
                .values()
                .filter(|function| function.id == method_id)
                .count(),
            0,
            "no top-level function payload may carry the impl method DefId"
        );

        let static_targets = lowered.functions[&main_id]
            .body
            .stmts
            .iter()
            .filter_map(|statement| match statement {
                HirStmt::Let { value, .. } => static_method_target(value),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(static_targets.len(), 2);
        for target in static_targets {
            assert_eq!(target.impl_id(), Some(impl_id));
            assert_eq!(target.method_id(), Some(method_id));
        }

        let resolved = crate::infer::finalize(lowered)
            .expect("static impl method authority should finalize without a top-level payload");
        let accepted = resolved.program.program();
        assert!(!accepted.functions.contains_key(&method_id));
        assert_eq!(accepted.impls[&impl_id].methods["identity"].id, method_id);
    }

    #[test]
    fn lowering_pipeline_selects_concrete_operator_in_trait_default() {
        let parsed = crate::parser::parse_string(
            "trait Flag\n    @invert: Bool\n    @invert = -> !true\n\ntrait Flip\n    @!: Bool\n\nimpl Flip for Bool\n    @! = -> ~BoolNot *self\n\nimpl Flag for I64\n\nmain = -> 0\n",
            &crate::Config {
                no_std: true,
                no_prelude: true,
                ..crate::Config::default()
            },
        )
        .expect("concrete operator in trait default should parse");
        let program = Program {
            module: parsed.module,
        };
        let crate_ctx = CrateContext::new();
        let declarations = crate::collect::collect(&program, &crate_ctx, false, None)
            .expect("concrete operator declarations should collect");
        let lowered = LoweringPipeline::new(&program, &crate_ctx, None)
            .lower_from_declarations(declarations)
            .expect("trait default should select the explicit concrete operator");
        let flip_id = lowered.resolver.item_paths["Flip"];
        let flip_impl = lowered.impls.values().find(|imp| imp.trait_id == Some(flip_id)).unwrap();
        let flag_id = lowered.resolver.item_paths["Flag"];
        let flag_impl = lowered.impls.values().find(|imp| imp.trait_id == Some(flag_id)).unwrap();
        let HirStmt::Expr(expr) = &flag_impl.methods["invert"].body.stmts[0] else {
            panic!("default method should contain the operator call");
        };
        let HirExprKind::MethodCall(_, _, _, _, Some(target)) = &expr.kind else {
            panic!("operator call should have an exact selected target");
        };
        assert_eq!(target.impl_id(), Some(flip_impl.id));
        assert_eq!(target.method_id(), Some(flip_impl.methods["!"].id));
        crate::infer::finalize(lowered)
            .expect("concrete operator selected in trait default should finalize");
    }

    #[test]
    fn lowering_pipeline_checks_conformance_after_selecting_operator_in_trait_default() {
        let parsed = crate::parser::parse_string(
            "trait Flag\n    @invert: Bool\n    @invert = -> !true\n\ntrait Flip\n    @!: I64\n\nimpl Flip for Bool\n    @! = -> ~BoolNot *self\n\nmain = -> 0\n",
            &crate::Config {
                no_std: true,
                no_prelude: true,
                ..crate::Config::default()
            },
        )
        .expect("mismatched concrete operator source should parse");
        let program = Program {
            module: parsed.module,
        };
        let crate_ctx = CrateContext::new();
        let declarations = crate::collect::collect(&program, &crate_ctx, false, None)
            .expect("mismatched concrete operator declarations should collect");
        let errors = LoweringPipeline::new(&program, &crate_ctx, None)
            .lower_from_declarations(declarations)
            .err()
            .expect("early operator selection must not bypass conformance");
        assert!(errors.iter().any(|error| error.message.contains("return type mismatch")));
        assert!(!errors.iter().any(|error| error.message.contains("No implementation found")));
    }

    #[test]
    fn lowering_pipeline_lowers_root_trait_defaults_with_glob_imports() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_root_trait_glob_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let entry = temp_dir.join("main.rk");
        std::fs::write(
            &entry,
            "mod helper\n> helper::*\ntrait HasAnswer\n    @value = -> answer!\nmain: I64\nmain = -> 0\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("helper.rk"),
            "answer: I64\nanswer = -> 42\n< answer\n",
        )
        .unwrap();

        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = SourceDatabase::new();
        let graph = db.load_entry(entry, &config).unwrap();
        let program = Program {
            module: graph.root_module().clone(),
        };
        let crate_ctx = CrateContext::new();
        let decls = crate::collect::collect_with_source_graph(
            &program,
            &graph,
            &crate_ctx,
            false,
            Some("demo"),
        )
        .unwrap();

        let lowered = LoweringPipeline::new(&program, &crate_ctx, Some("demo"))
            .lower_from_declarations(decls)
            .expect("root trait default should resolve glob-imported function");

        assert!(
            lowered.traits[&lowered.resolver.item_paths["HasAnswer"]].methods["value"]
                .body
                .stmts
                .len()
                > 0,
            "trait default body should be lowered"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lowering_pipeline_seeds_dependency_resolvers_for_root_glob_imports() {
        let answer_id = DefId::new(CrateId(7), LocalDefId(3));
        let mut interface = ArtifactCrateInterface::default();
        interface.insert_function(
            "dep::answer".to_string(),
            test_function(answer_id, "dep::answer"),
        );
        interface.root_export_ids.insert(
            "answer".to_string(),
            ArtifactExport {
                source: "dep::answer".to_string(),
                id: answer_id,
            },
        );
        let mut dep_resolver = ResolverTables::default();
        dep_resolver
            .item_paths
            .insert("dep::answer".to_string(), answer_id);
        dep_resolver.insert_export_alias_with_name(
            "answer".to_string(),
            "dep::answer".to_string(),
            answer_id,
        );

        let mut crate_ctx = CrateContext::new();
        crate_ctx
            .add_extern_crate(ExternCrateRecord::new(
                CrateId(7),
                "dep".to_string(),
                ExternCrateMetadata::new(interface, dep_resolver, BTreeMap::new()),
                ExternCrateBodies::default(),
                ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .unwrap();
        let parsed = crate::parser::parse_string(
            "> dep::*\nmain: I64\nmain = -> answer!\n",
            &crate::Config::default(),
        )
        .unwrap();
        let program = Program {
            module: parsed.module,
        };
        let mut decls = crate::collect::collect(&program, &crate_ctx, false, None).unwrap();
        // Collection already tests root artifact aliases; this keeps the assertion on
        // lower-stage glob handling from dependency resolver metadata.
        decls.resolver.import_aliases.remove("answer");

        let lowered = LoweringPipeline::new(&program, &crate_ctx, None)
            .lower_from_declarations(decls)
            .expect("pipeline should seed dependency resolvers before body lowering");

        assert!(
            lowered.functions[&lowered.resolver.item_paths["main"]]
                .body
                .stmts
                .len()
                > 0,
            "root glob-imported dependency function should lower"
        );
    }

    #[test]
    fn root_glob_import_keeps_current_crate_callable_targets_canonical() {
        let dependency_id = DefId::new(CrateId(7), LocalDefId(3));
        let mut interface = ArtifactCrateInterface::default();
        interface.insert_function(
            "dep::unused".to_string(),
            test_function(dependency_id, "dep::unused"),
        );
        interface.root_export_ids.insert(
            "unused".to_string(),
            ArtifactExport {
                source: "dep::unused".to_string(),
                id: dependency_id,
            },
        );
        let mut dependency_resolver = ResolverTables::default();
        dependency_resolver
            .item_paths
            .insert("dep::unused".to_string(), dependency_id);
        dependency_resolver.insert_export_alias_with_name(
            "unused".to_string(),
            "dep::unused".to_string(),
            dependency_id,
        );

        let mut crate_ctx = CrateContext::new();
        crate_ctx
            .add_extern_crate(ExternCrateRecord::new(
                CrateId(7),
                "dep".to_string(),
                ExternCrateMetadata::new(interface, dependency_resolver, BTreeMap::new()),
                ExternCrateBodies::default(),
                ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .unwrap();
        let parsed = crate::parser::parse_string(
            "> dep::*\nlocal: I64\nlocal = -> 83\nidentity: T -> T\nidentity = value -> value\nmain: I64\nmain = -> identity (local!)\n",
            &crate::Config {
                no_std: true,
                no_prelude: true,
                ..crate::Config::default()
            },
        )
        .unwrap();
        let program = Program {
            module: parsed.module,
        };
        let declarations = crate::collect::collect(&program, &crate_ctx, false, Some("app"))
            .expect("artifact root glob import should collect");
        let lowered = LoweringPipeline::new(&program, &crate_ctx, Some("app"))
            .lower_from_declarations(declarations)
            .expect("artifact root glob import should lower current-crate calls");

        let main = lowered
            .functions
            .values()
            .find(|function| function.name.ends_with("::main") || function.name == "main")
            .unwrap_or_else(|| {
                panic!(
                    "main function not found: {:?}",
                    lowered
                        .functions
                        .values()
                        .map(|function| &function.name)
                        .collect::<Vec<_>>()
                )
            });
        let HirStmt::Expr(HirExpr {
            kind: HirExprKind::Call(_, args, Some(HirCallTarget::Function(identity_id))),
            ..
        }) = &main.body.stmts[0]
        else {
            panic!(
                "expected canonical direct call to identity: {:?}",
                main.body.stmts
            );
        };
        assert_eq!(
            lowered.functions[identity_id].name.rsplit("::").next(),
            Some("identity"),
            "generic current-crate call must retain its canonical function target"
        );
        let HirExprKind::Call(_, _, Some(HirCallTarget::Function(local_id))) = &args[0].kind else {
            panic!("expected canonical direct call to local: {:?}", args[0]);
        };
        assert_eq!(
            lowered.functions[local_id].name.rsplit("::").next(),
            Some("local"),
            "non-generic current-crate call must retain its canonical function target"
        );
    }

    #[test]
    fn lowering_pipeline_does_not_make_dependency_exports_unqualified_without_import() {
        let answer_id = DefId::new(CrateId(7), LocalDefId(3));
        let mut interface = ArtifactCrateInterface::default();
        interface.insert_function(
            "dep::answer".to_string(),
            test_function(answer_id, "dep::answer"),
        );
        interface.root_export_ids.insert(
            "answer".to_string(),
            ArtifactExport {
                source: "dep::answer".to_string(),
                id: answer_id,
            },
        );
        let mut dep_resolver = ResolverTables::default();
        dep_resolver
            .item_paths
            .insert("dep::answer".to_string(), answer_id);
        dep_resolver.insert_export_alias_with_name(
            "answer".to_string(),
            "dep::answer".to_string(),
            answer_id,
        );

        let mut crate_ctx = CrateContext::new();
        crate_ctx
            .add_extern_crate(ExternCrateRecord::new(
                CrateId(7),
                "dep".to_string(),
                ExternCrateMetadata::new(interface, dep_resolver, BTreeMap::new()),
                ExternCrateBodies::default(),
                ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .unwrap();
        let parsed = crate::parser::parse_string(
            "main: I64\nmain = -> answer!\n",
            &crate::Config::default(),
        )
        .unwrap();
        let program = Program {
            module: parsed.module,
        };
        let decls = crate::collect::collect(&program, &crate_ctx, false, None).unwrap();

        let lowered =
            LoweringPipeline::new(&program, &crate_ctx, None).lower_from_declarations(decls);

        assert!(
            lowered.is_err(),
            "dependency root exports should require an explicit import for unqualified lookup"
        );
    }
}
