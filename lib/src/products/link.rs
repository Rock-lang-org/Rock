use super::{CompilerProducts, ProductDefId, ProductIdRemap, ProductLinkRecord};

pub(crate) fn attach_product_object_path(
    products: &mut Option<CompilerProducts>,
    object_path: &std::path::Path,
) {
    if let Some(products) = products.as_mut() {
        products.link.object_path = Some(object_path.to_path_buf());
    }
}

pub(crate) fn attach_product_link_records(
    products: &mut Option<CompilerProducts>,
    candidates: &[crate::mir::MirArtifactExport],
    symbol_overrides: &std::collections::HashMap<crate::ids::DefId, String>,
    id_remap: Option<&ProductIdRemap>,
) -> Result<(), String> {
    let Some(products) = products.as_mut() else {
        return Ok(());
    };

    for candidate in candidates {
        if candidate.provided_by_object || !candidate.substitution_empty || !candidate.has_body {
            continue;
        }
        if candidate.is_specialization && !candidate.is_drop_glue {
            continue;
        }

        let def_id = candidate
            .origin_def_id
            .ok_or_else(|| "product link export has no origin definition ID".to_string())?;
        let id_remap = id_remap.ok_or_else(|| {
            "product ID remap is required to attach product link records".to_string()
        })?;
        let product_id = product_link_id_for_candidate(products, id_remap, def_id)?;
        let backend_symbol = symbol_overrides
            .get(&def_id)
            .cloned()
            .unwrap_or_else(|| candidate.backend_symbol.clone());
        products
            .link
            .records
            .insert(product_id, ProductLinkRecord { backend_symbol });
    }

    Ok(())
}

fn product_link_id_for_candidate(
    products: &CompilerProducts,
    id_remap: &ProductIdRemap,
    def_id: crate::ids::DefId,
) -> Result<ProductDefId, String> {
    let callable_ids = id_remap
        .get(&ProductDefId::from(def_id))
        .into_iter()
        .flatten()
        .copied()
        .filter(|id| product_id_is_callable(products, *id))
        .collect::<Vec<_>>();

    match callable_ids.as_slice() {
        [id] => Ok(*id),
        [] => Err(format!(
            "no explicit product callable identity for MIR export origin {:?}",
            def_id
        )),
        _ => Err(format!(
            "ambiguous product callable identities for MIR export origin {:?}",
            def_id
        )),
    }
}

fn product_id_is_callable(products: &CompilerProducts, id: ProductDefId) -> bool {
    products.interface.functions.contains_key(&id)
        || products.interface.traits.values().any(|trait_def| {
            trait_def
                .methods
                .values()
                .any(|method| ProductDefId::from(method.id) == id)
        })
        || products.interface.impls.values().any(|imp| {
            imp.methods
                .values()
                .any(|method| ProductDefId::from(method.id) == id)
        })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap};

    use super::attach_product_link_records;
    use crate::hir::{
        HirBlock, HirExpr, HirExprKind, HirFunction, HirNameTables, HirProgram, HirStmt,
    };
    use crate::ids::{CrateId, DefId, InstanceId, LocalDefId};
    use crate::mono::{InstanceOrigin, InstanceRecord, InstanceSymbols, MonomorphizedProgram};
    use crate::products::{
        CompilerProducts, ProductBodies, ProductCrateId, ProductCrateIdentity, ProductDefId,
        ProductIdRemap, ProductIdentityTable, ProductLinkData, ProductSourceFingerprint,
    };
    use crate::types::Type;

    fn link_test_function(id: DefId, name: &str) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: vec![HirStmt::Return(Some(HirExpr {
                    kind: HirExprKind::Unit,
                    ty: Type::Unit,
                    span: crate::lexer::Span::test(),
                }))],
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn program(
        functions: HashMap<DefId, HirFunction>,
        names: HirNameTables,
        canonical_names: HashMap<DefId, String>,
    ) -> HirProgram {
        HirProgram::from_id_parts_with_names_and_canonical_names(
            functions,
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            names,
            crate::hir::HirLanguageItems::default(),
            &canonical_names,
        )
    }

    fn link_test_instance<P: crate::hir::HirPhase>(
        id: InstanceId,
        function: &crate::hir::HirFunctionFor<P>,
        name: &str,
    ) -> InstanceRecord {
        InstanceRecord {
            id,
            origin: InstanceOrigin::Function(function.id),
            substitution: Vec::new(),
            symbols: InstanceSymbols::new(name, name),
            declared: None,
            provided_by_object: false,
            is_specialization: false,
        }
    }

    fn accepted_link_test_function(function: HirFunction) -> crate::hir::AcceptedHirFunction {
        let id = function.id;
        let name = function.name.clone();
        let accepted = crate::hir::AcceptedHirProgram::try_from(program(
            HashMap::from([(id, function)]),
            HirNameTables {
                functions_by_name: HashMap::from([(name.clone(), id)]),
                ..HirNameTables::default()
            },
            HashMap::from([(id, name)]),
        ))
        .expect("link fixture HIR is accepted");
        accepted.function_by_id(id).unwrap().1.clone()
    }

    fn link_candidate(
        record: &InstanceRecord,
        bodies: &crate::mir::MirInstanceBodies,
    ) -> crate::mir::MirArtifactExport {
        crate::mir::MirArtifactExport {
            origin_def_id: match &record.origin {
                InstanceOrigin::Function(def_id) => Some(*def_id),
                InstanceOrigin::ImplMethod { method, .. }
                | InstanceOrigin::TraitDefault { method, .. } => Some(*method),
            },
            source_name: record.symbols.source_name.clone(),
            backend_symbol: record.symbols.backend_symbol.clone(),
            substitution_empty: record.substitution.is_empty(),
            has_body: bodies.contains_key(record.id),
            provided_by_object: record.provided_by_object,
            is_specialization: record.is_specialization,
            is_drop_glue: false,
        }
    }

    fn link_test_products(
        functions: Vec<(&str, HirFunction)>,
    ) -> (CompilerProducts, ProductIdRemap) {
        let current_def_ids = functions
            .iter()
            .map(|(_, function)| function.id)
            .collect::<BTreeSet<_>>();
        let mut function_payloads = HashMap::new();
        let mut function_names = HashMap::new();
        let mut canonical_names = HashMap::new();
        for (name, function) in functions {
            function_names.insert(name.to_string(), function.id);
            canonical_names
                .entry(function.id)
                .or_insert_with(|| name.to_string());
            function_payloads.entry(function.id).or_insert(function);
        }
        let program = program(
            function_payloads,
            HirNameTables {
                functions_by_name: function_names,
                ..HirNameTables::default()
            },
            canonical_names,
        );
        let resolved = crate::infer::ResolvedHirProgram::new(
            program,
            crate::collect::resolver::ResolverTables::default(),
            current_def_ids,
            CrateId(0),
            crate::ids::IdGen::default(),
        );
        CompilerProducts::from_resolved_hir_with_remap(
            ProductCrateIdentity::local("app".to_string()),
            &resolved,
            Vec::new(),
            BTreeMap::new(),
            crate::products::ProductSourceFingerprint::default(),
            crate::products::ProductLinkData::default(),
        )
        .expect("test HIR has a valid product language-item registry")
    }

    #[test]
    fn product_link_records_use_callable_id_from_remap_despite_misleading_names() {
        let origin_id = DefId::new(CrateId(0), LocalDefId(40));
        let misleading_id = DefId::new(CrateId(0), LocalDefId(41));
        let non_callable_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(42)));
        let callable_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(43)));
        let (products, _) = link_test_products(vec![
            (
                "misleading",
                link_test_function(misleading_id, "misleading"),
            ),
            (
                "callable",
                link_test_function(DefId::new(CrateId(0), LocalDefId(43)), "callable"),
            ),
        ]);
        let mut products = Some(products);
        let remap: ProductIdRemap = BTreeMap::from([(
            ProductDefId::from(origin_id),
            BTreeSet::from([non_callable_id, callable_id]),
        )]);
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(origin_id),
            source_name: "misleading".to_string(),
            backend_symbol: "callable_symbol".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&remap))
            .expect("callable remap should attach the link record");

        let products = products.expect("products should remain present");
        assert!(products.link.records.contains_key(&callable_id));
        assert!(!products
            .link
            .records
            .contains_key(&ProductDefId::from(misleading_id)));
    }

    #[test]
    fn product_link_records_reject_empty_remap_despite_matching_names() {
        let function_id = DefId::new(CrateId(0), LocalDefId(44));
        let (products, _) = link_test_products(vec![(
            "matching",
            link_test_function(function_id, "matching"),
        )]);
        let mut products = Some(products);
        let remap = ProductIdRemap::default();
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(function_id),
            source_name: "matching".to_string(),
            backend_symbol: "matching".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let error =
            attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&remap))
                .expect_err("an empty remap must not fall back to matching names");

        assert!(error.contains("no explicit product callable identity"));
    }

    #[test]
    fn product_link_records_reject_missing_remap_despite_matching_names() {
        let function_id = DefId::new(CrateId(0), LocalDefId(45));
        let (products, _) = link_test_products(vec![(
            "matching",
            link_test_function(function_id, "matching"),
        )]);
        let mut products = Some(products);
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(function_id),
            source_name: "matching".to_string(),
            backend_symbol: "matching".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let error = attach_product_link_records(&mut products, &[export], &HashMap::new(), None)
            .expect_err("a missing remap must not fall back to matching names");

        assert!(error.contains("product ID remap is required"));
    }

    #[test]
    fn product_link_records_reject_ambiguous_callable_remap() {
        let origin_id = DefId::new(CrateId(0), LocalDefId(46));
        let first_id = DefId::new(CrateId(0), LocalDefId(47));
        let second_id = DefId::new(CrateId(0), LocalDefId(48));
        let (products, _) = link_test_products(vec![
            ("first", link_test_function(first_id, "first")),
            ("second", link_test_function(second_id, "second")),
        ]);
        let mut products = Some(products);
        let remap: ProductIdRemap = BTreeMap::from([(
            ProductDefId::from(origin_id),
            BTreeSet::from([ProductDefId::from(first_id), ProductDefId::from(second_id)]),
        )]);
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(origin_id),
            source_name: "first".to_string(),
            backend_symbol: "ambiguous".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let error =
            attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&remap))
                .expect_err("multiple callable remap targets must be rejected");

        assert!(error.contains("ambiguous product callable identities"));
    }

    #[test]
    fn product_link_records_follow_pruned_instance_set() {
        let main_id = DefId::new(CrateId(0), LocalDefId(1));
        let unused_id = DefId::new(CrateId(0), LocalDefId(2));
        let main_source = link_test_function(main_id, "main");
        let unused_source = link_test_function(unused_id, "unused");
        let main = accepted_link_test_function(main_source.clone());
        let unused = accepted_link_test_function(unused_source.clone());
        let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
        pre_mir_instance_bodies.insert(InstanceId(0), main.clone());
        pre_mir_instance_bodies.insert(InstanceId(1), unused.clone());
        let mut monomorphized = MonomorphizedProgram {
            program: crate::hir::HirProgramFor::<crate::hir::AcceptedHir>::from_accepted_id_parts_with_names_and_canonical_names(
                HashMap::from([
                    (main_id, main.clone()),
                    (unused_id, unused.clone()),
                ]),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HirNameTables {
                    functions_by_name: HashMap::from([
                        ("main".to_string(), main_id),
                        ("unused".to_string(), unused_id),
                    ]),
                    ..HirNameTables::default()
                },
                &HashMap::from([
                    (main_id, "main".to_string()),
                    (unused_id, "unused".to_string()),
                ]),
            ),
            instances: BTreeMap::from([
                (
                    InstanceId(0),
                    link_test_instance(InstanceId(0), &main, "main"),
                ),
                (
                    InstanceId(1),
                    link_test_instance(InstanceId(1), &unused, "unused"),
                ),
            ]),
            pre_mir_instance_bodies,
            generated_drop_instances: Default::default(),
            type_context: crate::type_context::TypeContext::new(),
            source_map: crate::source_map::SemanticSourceMap::default(),
        };
        let mut instance_bodies =
            crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut monomorphized);
        crate::dce::prune_unreachable_instances(&mut monomorphized, &mut instance_bodies);
        let (products, id_remap) =
            link_test_products(vec![("main", main_source), ("unused", unused_source)]);
        let mut products = Some(products);
        let candidates = monomorphized
            .instances
            .values()
            .map(|record| link_candidate(record, &instance_bodies))
            .collect::<Vec<_>>();

        attach_product_link_records(&mut products, &candidates, &HashMap::new(), Some(&id_remap))
            .expect("pruned candidates should retain one callable remap target");

        let products = products.expect("products should be present");
        assert!(products
            .link
            .records
            .contains_key(&ProductDefId::from(main_id)));
        assert!(!products
            .link
            .records
            .contains_key(&ProductDefId::from(unused_id)));
    }

    #[test]
    fn product_link_records_ignore_body_only_function_rows() {
        let function_id = DefId::new(CrateId(0), LocalDefId(7));
        let product_id = ProductDefId::from(function_id);
        let function = link_test_function(function_id, "ghost");
        let function = accepted_link_test_function(function);
        let mut bodies = ProductBodies::default();
        bodies.functions.insert(product_id, function);
        let mut products = Some(CompilerProducts {
            crate_identity: ProductCrateIdentity::local("app".to_string()),
            identity_table: ProductIdentityTable {
                local_crate: Some(ProductCrateId(0)),
                display_names: BTreeMap::from([(product_id, "ghost".to_string())]),
                export_names: BTreeMap::from([("ghost".to_string(), product_id)]),
                ..ProductIdentityTable::default()
            },
            interface: crate::products::ProductInterface::default(),
            bodies,
            link: ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        });
        let candidate = crate::mir::MirArtifactExport {
            origin_def_id: Some(function_id),
            source_name: "ghost".to_string(),
            backend_symbol: "ghost".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let id_remap = BTreeMap::from([(
            ProductDefId::from(function_id),
            BTreeSet::from([product_id]),
        )]);
        let error = attach_product_link_records(
            &mut products,
            &[candidate],
            &HashMap::new(),
            Some(&id_remap),
        )
        .expect_err("body-only rows must not become callable link records");

        assert!(error.contains("no explicit product callable identity"));

        let products = products.expect("products should remain present");
        assert!(!products.link.records.contains_key(&product_id));
    }

    #[test]
    fn product_link_records_ignore_body_only_trait_default_rows() {
        let method_id = DefId::new(CrateId(0), LocalDefId(8));
        let product_id = ProductDefId::from(method_id);
        let method = link_test_function(method_id, "show");
        let method = accepted_link_test_function(method);
        let mut bodies = ProductBodies::default();
        bodies.trait_default_methods.insert(product_id, method);
        let mut products = Some(CompilerProducts {
            crate_identity: ProductCrateIdentity::local("app".to_string()),
            identity_table: ProductIdentityTable {
                local_crate: Some(ProductCrateId(0)),
                display_names: BTreeMap::from([(product_id, "Show::show".to_string())]),
                export_names: BTreeMap::from([("Show::show".to_string(), product_id)]),
                ..ProductIdentityTable::default()
            },
            interface: crate::products::ProductInterface::default(),
            bodies,
            link: ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        });
        let candidate = crate::mir::MirArtifactExport {
            origin_def_id: Some(method_id),
            source_name: "Show::show".to_string(),
            backend_symbol: "Show_show".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let id_remap =
            BTreeMap::from([(ProductDefId::from(method_id), BTreeSet::from([product_id]))]);
        let error = attach_product_link_records(
            &mut products,
            &[candidate],
            &HashMap::new(),
            Some(&id_remap),
        )
        .expect_err("body-only trait default rows must not become callable link records");

        assert!(error.contains("no explicit product callable identity"));

        let products = products.expect("products should remain present");
        assert!(!products.link.records.contains_key(&product_id));
    }

    #[test]
    fn product_link_records_filter_non_drop_glue_specialization_artifact_exports() {
        let function_id = DefId::new(CrateId(0), LocalDefId(9));
        let product_id = ProductDefId::from(function_id);
        let function = link_test_function(function_id, "make");
        let (products, id_remap) = link_test_products(vec![("make", function)]);
        let mut products = Some(products);
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(function_id),
            source_name: "make".to_string(),
            backend_symbol: "make__i64".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: true,
            is_drop_glue: false,
        };

        attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&id_remap))
            .expect("non-drop-glue specializations are ineligible");

        let products = products.expect("products should remain present");
        assert!(!products.link.records.contains_key(&product_id));
    }

    #[test]
    fn product_link_records_export_drop_glue_specialization_artifact_exports() {
        let function_id = DefId::new(CrateId(0), LocalDefId(10));
        let product_id = ProductDefId::from(function_id);
        let function = link_test_function(function_id, "drop_box");
        let (products, id_remap) = link_test_products(vec![("drop_box", function)]);
        let mut products = Some(products);
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(function_id),
            source_name: "drop_box".to_string(),
            backend_symbol: "Box_drop".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: true,
            is_drop_glue: true,
        };

        attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&id_remap))
            .expect("drop-glue specializations should retain one callable remap target");

        let products = products.expect("products should remain present");
        assert_eq!(
            products
                .link
                .records
                .get(&product_id)
                .map(|record| record.backend_symbol.as_str()),
            Some("Box_drop")
        );
    }

    #[test]
    fn product_link_records_do_not_match_backend_symbols_from_display_names() {
        let function_id = DefId::new(CrateId(0), LocalDefId(20));
        let product_id = ProductDefId::from(function_id);
        let function = link_test_function(function_id, "answer");
        let (products, id_remap) = link_test_products(vec![("answer", function)]);
        let mut products = Some(products);
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(DefId::new(CrateId(0), LocalDefId(99))),
            source_name: "not_answer".to_string(),
            backend_symbol: "answer".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let error =
            attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&id_remap))
                .expect_err("unmapped origins must not use a display-name fallback");

        assert!(error.contains("no explicit product callable identity"));

        let products = products.expect("products should remain present");
        assert!(!products.link.records.contains_key(&product_id));
    }

    #[test]
    fn product_link_records_use_origin_id_for_impl_methods_without_name_match() {
        let impl_id = DefId::new(CrateId(0), LocalDefId(30));
        let method_id = DefId::new(CrateId(0), LocalDefId(31));
        let method_product_id = ProductDefId::from(method_id);
        let mut method = link_test_function(method_id, "get");
        method.is_method = true;

        let mut products = Some(CompilerProducts {
            crate_identity: ProductCrateIdentity::local("app".to_string()),
            identity_table: ProductIdentityTable {
                local_crate: Some(ProductCrateId(0)),
                display_names: BTreeMap::from([(
                    method_product_id,
                    "impl Value for Box::get".to_string(),
                )]),
                ..ProductIdentityTable::default()
            },
            interface: crate::products::ProductInterface {
                impls: BTreeMap::from([(
                    ProductDefId::from(impl_id),
                    crate::products::ProductImplInterface {
                        id: impl_id,
                        owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                        type_name: "Box".to_string(),
                        type_generics: Vec::new(),
                        receiver_pattern: Vec::new().into(),
                        trait_name: Some("Value".to_string()),
                        trait_id: None,
                        trait_generics: Vec::new(),
                        trait_arg_types: Vec::new(),
                        associated_types: Vec::new(),
                        bounds: HashMap::new().into(),
                        methods: BTreeMap::from([(
                            "get".to_string(),
                            crate::products::ProductFunctionInterface::from(&method),
                        )]),
                    },
                )]),
                ..crate::products::ProductInterface::default()
            },
            bodies: ProductBodies::default(),
            link: ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        });
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(method_id),
            source_name: "Box::get".to_string(),
            backend_symbol: "Box_get".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let id_remap = BTreeMap::from([(
            ProductDefId::from(method_id),
            BTreeSet::from([method_product_id]),
        )]);
        attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&id_remap))
            .expect("impl method remap should identify the callable method");

        let products = products.expect("products should remain present");
        assert_eq!(
            products
                .link
                .records
                .get(&method_product_id)
                .map(|record| record.backend_symbol.as_str()),
            Some("Box_get")
        );
    }
}
