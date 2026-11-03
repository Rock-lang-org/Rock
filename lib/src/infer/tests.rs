use super::*;

use crate::hir::{
    HirBlock, HirExpr, HirExprKind, HirImpl, HirImplOwner, HirImplReceiverPattern, HirParam,
    HirStmt, HirStruct, HirTrait, HirTypeLocation,
};
use crate::types::Type;

fn empty_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn partial_hir_with_traits(traits: HashMap<DefId, HirTrait>) -> PartialHir {
    PartialHir {
        functions: HashMap::new(),
        structs: HashMap::new(),
        enums: HashMap::new(),
        traits,
        impls: HashMap::new(),
        externs: HashMap::new(),
        type_aliases: HashMap::new(),
        engine: InferenceEngine::new(),
        function_type_vars: HashMap::new(),
        import_aliases: HashMap::new(),
        loaded_module_paths: vec![],
        constraint_store: ConstraintStore::default(),
        resolver: ResolverTables::default(),
        current_def_ids: BTreeSet::new(),
        root_crate_id: crate::ids::CrateId(0),
        local_def_ids: crate::ids::IdGen::<crate::ids::LocalDefId>::new(),
        language_items: HirLanguageItems::default(),
        imported_effective_trait_methods: HashMap::new(),
        inference_sccs: HashMap::new(),
        inference_scc_order: Vec::new(),
        source_map: Default::default(),
    }
}

#[test]
fn strict_materialization_rejects_targetless_renamed_method_call() {
    let trait_id = DefId::new(CrateId(0), LocalDefId(10));
    let member_id = DefId::new(CrateId(0), LocalDefId(11));
    let impl_id = DefId::new(CrateId(0), LocalDefId(12));
    let method_id = DefId::new(CrateId(0), LocalDefId(13));
    let receiver_id = DefId::new(CrateId(0), LocalDefId(14));

    let receiver_ty = Type::Struct {
        id: receiver_id,
        args: Vec::new(),
    };
    let method = HirFunction {
        id: method_id,
        name: "legacy_name".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: receiver_ty.clone(),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(crate::types::ReceiverMode::Move),
        is_unsafe: false,
    };
    let trait_def = HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "RenamedProtocol".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("source_name".to_string(), {
            let mut signature = method.clone();
            signature.id = member_id;
            signature.name = "source_name".to_string();
            signature
        })]),
        signatures: HashMap::new(),
    };
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Owner".to_string()),
        type_name: "Owner".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(receiver_ty.clone()),
        trait_name: Some("RenamedProtocol".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("legacy_name".to_string(), method)]),
    };
    let mut function = empty_function(DefId::new(CrateId(0), LocalDefId(15)), "caller");
    function.body.stmts.push(HirStmt::Expr(HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKind::Var("owner".to_string()),
                ty: receiver_ty.clone(),
                span: crate::lexer::Span::test(),
            }),
            "source_name".to_string(),
            Vec::new(),
            Some(crate::types::ReceiverMode::Move),
            None,
        ),
        ty: Type::Unit,
        span: crate::lexer::Span::test(),
    }));

    let mut partial = partial_hir_with_traits(HashMap::from([(trait_id, trait_def)]));
    let function_id = function.id;
    partial.functions.insert(function_id, function);
    partial.impls.insert(impl_id, imp);
    partial
        .imported_effective_trait_methods
        .insert((impl_id, member_id), method_id);

    let owners = HashSet::from([ConstraintOwner::Body(function_id)]);
    let mut obligations = authority::authority_obligations(&partial, &owners);
    let obligation = obligations
        .iter_mut()
        .find(|obligation| obligation.kind != authority::AuthorityObligationKind::Propagation)
        .expect("targetless method call must register an authority obligation");
    let errors = authority::run_authority_obligation(&mut partial, obligation, true, true)
        .expect_err("strict inference must reject targetless method dispatch");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("accepted HIR method call has no selected authority")
    }));
}

#[test]
fn finalize_rejects_duplicate_impl_method_ids_instead_of_repairing_them() {
    let duplicate_method_id = DefId::new(CrateId(0), LocalDefId(8));
    let mut partial = partial_hir_with_traits(HashMap::new());
    for (impl_id, type_name) in [
        (DefId::new(CrateId(0), LocalDefId(20)), "First"),
        (DefId::new(CrateId(0), LocalDefId(21)), "Second"),
    ] {
        partial.impls.insert(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named(type_name.to_string()),
                type_name: type_name.to_string(),
                type_generics: Vec::new(),
                receiver_pattern: Vec::new().into(),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "id".to_string(),
                    empty_function(duplicate_method_id, "id"),
                )]),
            },
        );
    }

    let errors = match finalize(partial) {
        Ok(_) => panic!("duplicate method IDs should not be repaired during inference"),
        Err(errors) => errors,
    };

    assert!(errors
        .iter()
        .any(|error| error.message.contains("duplicate method identity")));
}

#[test]
fn finalization_exempts_only_resolver_root_entrypoint_from_generalization() {
    let main_id = DefId::new(CrateId(0), LocalDefId(60));
    let nested_id = DefId::new(CrateId(0), LocalDefId(61));
    let mut engine = InferenceEngine::new();
    let Type::TypeVar(main_var) = engine.fresh_type_var_at(crate::lexer::Span::test()) else {
        panic!("fresh inference variable should be a TypeVar");
    };
    let Type::TypeVar(nested_var) = engine.fresh_type_var_at(crate::lexer::Span::test()) else {
        panic!("fresh inference variable should be a TypeVar");
    };
    let mut main = empty_function(main_id, "main");
    main.ret_type = Type::TypeVar(main_var);
    main.body.ty = Type::TypeVar(main_var);
    let mut nested = empty_function(nested_id, "main");
    nested.ret_type = Type::TypeVar(nested_var);
    nested.body.ty = Type::TypeVar(nested_var);
    let mut resolver = ResolverTables::default();
    resolver.item_paths.insert("main".to_string(), main_id);
    let mut partial = PartialHir {
        functions: HashMap::from([(main_id, main), (nested_id, nested)]),
        structs: HashMap::new(),
        enums: HashMap::new(),
        traits: HashMap::new(),
        impls: HashMap::new(),
        externs: HashMap::new(),
        type_aliases: HashMap::new(),
        engine,
        function_type_vars: HashMap::from([
            (main_id, HashSet::from([main_var])),
            (nested_id, HashSet::from([nested_var])),
        ]),
        import_aliases: HashMap::new(),
        loaded_module_paths: vec![],
        constraint_store: ConstraintStore::default(),
        resolver,
        current_def_ids: BTreeSet::from([main_id, nested_id]),
        root_crate_id: crate::ids::CrateId(0),
        local_def_ids: crate::ids::IdGen::<crate::ids::LocalDefId>::new(),
        language_items: HirLanguageItems::default(),
        imported_effective_trait_methods: HashMap::new(),
        inference_sccs: HashMap::new(),
        inference_scc_order: Vec::new(),
        source_map: Default::default(),
    };

    generalize::generalize_functions(&mut partial, &[main_id, nested_id], Some(main_id));

    assert!(partial.functions[&main_id].generic_params.is_empty());
    assert_eq!(
        partial.functions[&nested_id]
            .generic_params
            .iter()
            .map(|param| param.name.as_str())
            .collect::<Vec<_>>(),
        vec!["T"]
    );
    assert!(finalize(partial).is_err());
}

#[test]
fn finalization_item_id_errors_are_sorted() {
    fn partial(reverse_functions: bool) -> PartialHir {
        let first_key = DefId::new(CrateId(0), LocalDefId(1));
        let second_key = DefId::new(CrateId(0), LocalDefId(2));
        let mut functions = HashMap::new();
        let entries = [
            (
                first_key,
                empty_function(DefId::new(CrateId(0), LocalDefId(11)), "first"),
            ),
            (
                second_key,
                empty_function(DefId::new(CrateId(0), LocalDefId(12)), "second"),
            ),
        ];
        for (key, function) in if reverse_functions {
            entries.into_iter().rev().collect::<Vec<_>>()
        } else {
            entries.into_iter().collect()
        } {
            functions.insert(key, function);
        }
        let structure = HirStruct {
            id: DefId::new(CrateId(0), LocalDefId(13)),
            name: "structure".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        };

        let mut source_map = crate::source_map::SemanticSourceMap::default();
        source_map.insert_definition(first_key, crate::lexer::Span::test());
        source_map.insert_definition(second_key, crate::lexer::Span::test());
        PartialHir {
            functions,
            structs: HashMap::from([(first_key, structure)]),
            enums: HashMap::new(),
            traits: HashMap::new(),
            impls: HashMap::new(),
            externs: HashMap::new(),
            type_aliases: HashMap::new(),
            engine: InferenceEngine::new(),
            function_type_vars: HashMap::new(),
            import_aliases: HashMap::new(),
            loaded_module_paths: vec![],
            constraint_store: ConstraintStore::default(),
            resolver: ResolverTables::default(),
            current_def_ids: BTreeSet::new(),
            root_crate_id: crate::ids::CrateId(0),
            local_def_ids: crate::ids::IdGen::<crate::ids::LocalDefId>::new(),
            language_items: HirLanguageItems::default(),
            imported_effective_trait_methods: HashMap::new(),
            inference_sccs: HashMap::new(),
            inference_scc_order: Vec::new(),
            source_map,
        }
    }

    let strict = |reverse| {
        finalize(partial(reverse))
            .expect_err("strict finalization should reject item ID mismatches")
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
    };
    let expected = vec![
        "function declaration ID mismatch in compiler state".to_string(),
        "struct declaration ID mismatch in compiler state".to_string(),
        "function declaration ID mismatch in compiler state".to_string(),
    ];

    assert_eq!(strict(false), expected);
    assert_eq!(strict(true), expected);
}

#[test]
fn finalize_reports_impl_method_identity_errors_in_def_id_order() {
    fn diagnostics(reverse_insertion: bool) -> Vec<String> {
        let duplicate_method_id = DefId::new(CrateId(0), LocalDefId(8));
        let first_impl_id = DefId::new(CrateId(0), LocalDefId(20));
        let second_impl_id = DefId::new(CrateId(0), LocalDefId(21));
        let mut partial = partial_hir_with_traits(HashMap::new());
        let impls = [
            (
                first_impl_id,
                HirImpl {
                    id: first_impl_id,
                    owner: HirImplOwner::Named("First".to_string()),
                    type_name: "First".to_string(),
                    type_generics: Vec::new(),
                    receiver_pattern: Vec::new().into(),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: HashMap::new().into(),
                    methods: HashMap::from([
                        (
                            "later".to_string(),
                            empty_function(duplicate_method_id, "later"),
                        ),
                        (
                            "earlier".to_string(),
                            empty_function(DefId::new(CrateId(0), LocalDefId(7)), "earlier"),
                        ),
                    ]),
                },
            ),
            (
                second_impl_id,
                HirImpl {
                    id: second_impl_id,
                    owner: HirImplOwner::Named("Second".to_string()),
                    type_name: "Second".to_string(),
                    type_generics: Vec::new(),
                    receiver_pattern: Vec::new().into(),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: HashMap::new().into(),
                    methods: HashMap::from([(
                        "duplicate".to_string(),
                        empty_function(duplicate_method_id, "duplicate"),
                    )]),
                },
            ),
        ];
        for (id, imp) in if reverse_insertion {
            impls.into_iter().rev().collect::<Vec<_>>()
        } else {
            impls.into_iter().collect()
        } {
            partial.impls.insert(id, imp);
        }

        finalize(partial)
            .expect_err("duplicate method identity should be rejected")
            .into_iter()
            .map(|error| error.message)
            .collect()
    }

    assert_eq!(diagnostics(false), diagnostics(true));
}

#[test]
fn finalize_preserves_resolver_tables() {
    let partial = PartialHir {
        functions: HashMap::new(),
        structs: HashMap::new(),
        enums: HashMap::new(),
        traits: HashMap::new(),
        impls: HashMap::new(),
        externs: HashMap::new(),
        type_aliases: HashMap::new(),
        engine: InferenceEngine::new(),
        function_type_vars: HashMap::new(),
        import_aliases: HashMap::new(),
        loaded_module_paths: vec![],
        constraint_store: ConstraintStore::default(),
        resolver: ResolverTables::default(),
        current_def_ids: BTreeSet::new(),
        root_crate_id: crate::ids::CrateId(0),
        local_def_ids: crate::ids::IdGen::<crate::ids::LocalDefId>::new(),
        language_items: HirLanguageItems::default(),
        imported_effective_trait_methods: HashMap::new(),
        inference_sccs: HashMap::new(),
        inference_scc_order: Vec::new(),
        source_map: Default::default(),
    };

    let resolved = finalize(partial).unwrap();

    assert!(resolved.resolver.module_paths.is_empty());
    assert!(resolved.program.functions.is_empty());
}

#[test]
fn resolved_hir_reports_missing_required_type_id_location() {
    let resolved = finalize(partial_hir_with_traits(HashMap::new())).unwrap();
    let missing = HirTypeLocation::FunctionReturn {
        function: DefId::new(CrateId(0), LocalDefId(999)),
    };

    let err = resolved.require_type_id_at(missing).unwrap_err();

    assert!(err.message.contains("missing finalized HIR TypeId"));
}

#[test]
fn finalize_builds_resolved_hir_type_context_and_sidecar() {
    let function_id = DefId::new(CrateId(0), LocalDefId(1));
    let mut partial = PartialHir {
        functions: HashMap::from([(
            function_id,
            HirFunction {
                id: function_id,
                name: "identity".to_string(),
                generic_params: Vec::new(),
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "value".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::I64,
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: Type::I64,
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("value".to_string()),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::I64,
                },
                is_curried: false,
                is_method: false,
                self_receiver: None,
                is_unsafe: false,
            },
        )]),
        structs: HashMap::new(),
        enums: HashMap::new(),
        traits: HashMap::new(),
        impls: HashMap::new(),
        externs: HashMap::new(),
        type_aliases: HashMap::new(),
        engine: InferenceEngine::new(),
        function_type_vars: HashMap::new(),
        import_aliases: HashMap::new(),
        loaded_module_paths: vec![],
        constraint_store: ConstraintStore::default(),
        resolver: ResolverTables::default(),
        current_def_ids: BTreeSet::from([function_id]),
        root_crate_id: crate::ids::CrateId(0),
        local_def_ids: crate::ids::IdGen::<crate::ids::LocalDefId>::new(),
        language_items: HirLanguageItems::default(),
        imported_effective_trait_methods: HashMap::new(),
        inference_sccs: HashMap::new(),
        inference_scc_order: Vec::new(),
        source_map: Default::default(),
    };
    partial.local_def_ids.fresh();
    partial.local_def_ids.fresh();

    let resolved = finalize(partial).unwrap();
    let ret_id = resolved
        .type_id_at(&HirTypeLocation::FunctionReturn {
            function: function_id,
        })
        .unwrap();
    let param_id = resolved
        .type_id_at(&HirTypeLocation::FunctionParam {
            function: function_id,
            index: 0,
        })
        .unwrap();
    let expr_id = resolved
        .type_id_at(&HirTypeLocation::Expr {
            owner: function_id,
            path: vec![0, 0],
        })
        .unwrap();

    assert_eq!(ret_id, param_id);
    assert_eq!(param_id, expr_id);
    assert_eq!(resolved.type_at(ret_id), Type::I64);
}

#[test]
fn finalize_preserves_id_keyed_function_payload_and_resolver_aliases() {
    let id = DefId::new(CrateId(0), LocalDefId(50));
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(id, "package::canonical_answer".to_string());
    resolver
        .item_paths
        .insert("package::canonical_answer".to_string(), id);
    resolver
        .import_aliases
        .insert("imported_answer".to_string(), id);

    let partial = PartialHir {
        functions: HashMap::from([(id, empty_function(id, "payload_display"))]),
        structs: HashMap::new(),
        enums: HashMap::new(),
        traits: HashMap::new(),
        impls: HashMap::new(),
        externs: HashMap::new(),
        type_aliases: HashMap::new(),
        engine: InferenceEngine::new(),
        function_type_vars: HashMap::new(),
        import_aliases: HashMap::new(),
        loaded_module_paths: vec![],
        constraint_store: ConstraintStore::default(),
        resolver,
        current_def_ids: BTreeSet::new(),
        root_crate_id: crate::ids::CrateId(0),
        local_def_ids: crate::ids::IdGen::<crate::ids::LocalDefId>::new(),
        language_items: HirLanguageItems::default(),
        imported_effective_trait_methods: HashMap::new(),
        inference_sccs: HashMap::new(),
        inference_scc_order: Vec::new(),
        source_map: Default::default(),
    };

    let resolved = finalize(partial).expect("ID-keyed payload should finalize");

    assert_eq!(resolved.program.functions[&id].name, "payload_display");
    assert_eq!(
        resolved.program.names.functions_by_name["imported_answer"],
        id
    );
    assert_eq!(
        resolved.program.function_by_id(id).map(|(name, _)| name),
        Some("package::canonical_answer")
    );
}

#[test]
fn finalize_reports_function_key_payload_id_mismatch() {
    let key = DefId::new(CrateId(0), LocalDefId(51));
    let payload = DefId::new(CrateId(0), LocalDefId(52));
    let mut source_map = crate::source_map::SemanticSourceMap::default();
    source_map.insert_definition(key, crate::lexer::Span::test());
    let partial = PartialHir {
        functions: HashMap::from([(key, empty_function(payload, "payload_display"))]),
        structs: HashMap::new(),
        enums: HashMap::new(),
        traits: HashMap::new(),
        impls: HashMap::new(),
        externs: HashMap::new(),
        type_aliases: HashMap::new(),
        engine: InferenceEngine::new(),
        function_type_vars: HashMap::new(),
        import_aliases: HashMap::new(),
        loaded_module_paths: vec![],
        constraint_store: ConstraintStore::default(),
        resolver: ResolverTables::default(),
        current_def_ids: BTreeSet::new(),
        root_crate_id: crate::ids::CrateId(0),
        local_def_ids: crate::ids::IdGen::<crate::ids::LocalDefId>::new(),
        language_items: HirLanguageItems::default(),
        imported_effective_trait_methods: HashMap::new(),
        inference_sccs: HashMap::new(),
        inference_scc_order: Vec::new(),
        source_map,
    };

    let errors = finalize(partial).expect_err("mismatched function IDs should be rejected");

    assert!(errors
        .iter()
        .any(|error| error.message.contains("function declaration ID mismatch")));
}
