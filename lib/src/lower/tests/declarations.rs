use super::*;

fn declarations_with_items(resolver: ResolverTables, items: DeclarationItems) -> Declarations {
    Declarations {
        indexing_ids: IndexingIds::new_root(),
        item_index: ItemIndex::new(),
        resolver,
        current_def_ids: BTreeSet::new(),
        items,
        function_type_vars: HashMap::new(),
        infix_precedence: HashMap::new(),
        loaded_module_paths: Vec::new(),
        type_vars: DeclarationTypeVars::new(),
        inject_prelude: false,
        loaded_prelude_export_ids: HashMap::new(),
        module_file_cache: HashMap::new(),
        source_modules: crate::source_loader::SourceModuleSet::default(),
        dependency_root_export_ids: HashMap::new(),
        source_map: Default::default(),
        language_items: Default::default(),
    }
}

#[test]
fn lowerer_from_declarations_rejects_missing_canonical_item_name() {
    let function_id = DefId::new(CrateId(0), LocalDefId(1));
    let decls = declarations_with_items(
        ResolverTables::default(),
        declaration_items(
            HashMap::from([(
                function_id,
                test_function(function_id, "answer", Vec::new()),
            )]),
            HashMap::new(),
        ),
    );

    let errors = Lowerer::from_declarations(decls)
        .err()
        .expect("missing canonical names should be reported");

    assert!(errors[0]
        .message
        .contains("missing canonical function declaration name"));
}

#[test]
fn lowerer_from_declarations_reports_smallest_missing_canonical_id_regardless_of_insertion_order() {
    let first = DefId::new(CrateId(0), LocalDefId(1));
    let second = DefId::new(CrateId(0), LocalDefId(2));

    for ids in [[second, first], [first, second]] {
        let functions = ids
            .into_iter()
            .map(|id| (id, test_function(id, "missing", Vec::new())))
            .collect();
        let errors = Lowerer::from_declarations(declarations_with_items(
            ResolverTables::default(),
            declaration_items(functions, HashMap::new()),
        ))
        .err()
        .expect("missing canonical function names should be reported");

        assert_eq!(
            errors[0].message,
            format!("missing canonical function declaration name for DefId {first:?}")
        );
    }

    for ids in [[second, first], [first, second]] {
        let externs = ids.into_iter().map(|id| (id, test_extern(id))).collect();
        let errors = Lowerer::from_declarations(declarations_with_items(
            ResolverTables::default(),
            declaration_items(HashMap::new(), externs),
        ))
        .err()
        .expect("missing canonical extern names should be reported");

        assert_eq!(
            errors[0].message,
            format!("missing canonical extern declaration name for DefId {first:?}")
        );
    }
}

fn test_impl(id: DefId) -> HirImpl {
    HirImpl {
        id,
        owner: crate::hir::HirImplOwner::Named(format!("Type{}", id.local.0)),
        type_name: format!("Type{}", id.local.0),
        type_generics: vec![],
        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
        trait_name: None,
        trait_id: None,
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: HashMap::new().into(),
        methods: HashMap::new(),
    }
}

fn test_extern(id: DefId) -> HirExtern {
    HirExtern {
        id,
        name: format!("extern{}", id.local.0),
        params: vec![],
        ret: Type::Unit,
        variadic: false,
        is_unsafe: false,
    }
}

#[test]
fn lowerer_from_declarations_orders_impls_and_externs_by_def_id() {
    let first = DefId::new(CrateId(0), LocalDefId(1));
    let second = DefId::new(CrateId(0), LocalDefId(2));
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(first, "extern1".to_string());
    resolver
        .item_names_by_id
        .insert(second, "extern2".to_string());
    let items = DeclarationItems::from_id_maps(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(second, test_impl(second)), (first, test_impl(first))]),
        HashMap::from([(second, test_extern(second)), (first, test_extern(first))]),
    )
    .unwrap();

    let lowerer = Lowerer::from_declarations(declarations_with_items(resolver, items)).unwrap();

    assert_eq!(
        lowerer
            .items
            .impl_defs_in_order()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        vec![first, second]
    );
    assert_eq!(lowerer.items.extern_def(first).unwrap().id, first);
    assert_eq!(lowerer.items.extern_def(second).unwrap().id, second);
}

#[test]
fn lowerer_from_declarations_preserves_real_local_extern_id_zero() {
    let extern_id = DefId::new(CrateId(0), LocalDefId(0));
    let collision_id = DefId::new(CrateId(0), LocalDefId(1));
    let mut resolver = ResolverTables::default();
    resolver.item_paths.insert("puts".to_string(), collision_id);
    resolver
        .item_names_by_id
        .insert(extern_id, "demo::puts".to_string());

    let decls = Declarations {
        indexing_ids: IndexingIds::new_root(),
        item_index: ItemIndex::new(),
        resolver,
        current_def_ids: BTreeSet::from([extern_id]),
        items: declaration_items(
            HashMap::new(),
            HashMap::from([(
                extern_id,
                HirExtern {
                    id: extern_id,
                    name: "demo::puts".to_string(),
                    params: vec![Type::Pointer(Box::new(Type::U8))],
                    ret: Type::I32,
                    variadic: false,
                    is_unsafe: false,
                },
            )]),
        ),
        function_type_vars: HashMap::new(),
        infix_precedence: HashMap::new(),
        loaded_module_paths: Vec::new(),
        type_vars: DeclarationTypeVars::new(),
        inject_prelude: false,
        loaded_prelude_export_ids: HashMap::new(),
        module_file_cache: HashMap::new(),
        source_modules: crate::source_loader::SourceModuleSet::default(),
        dependency_root_export_ids: HashMap::new(),
        source_map: Default::default(),
        language_items: Default::default(),
    };

    let lowerer = Lowerer::from_declarations(decls).unwrap();

    assert_eq!(lowerer.items.extern_def(extern_id).unwrap().id, extern_id);
}

#[test]
fn lowerer_construction_uses_explicit_service_wrappers() {
    let services = crate::lower::services::LowererServices::new(true);
    let lowerer = Lowerer::from_services(services);

    assert!(lowerer.prelude.is_enabled());
    assert!(lowerer.diagnostics.is_empty());
    assert!(lowerer.items.functions().next().is_none());
}

#[test]
fn body_lowering_context_owns_body_local_ids_and_restores_lowerer_state() {
    let mut lowerer = Lowerer::new_for_test();
    let outer_owner = DefId::new(CrateId(0), LocalDefId(99));
    lowerer.generic_context = Some(crate::lower::body_context::GenericLoweringContext::new(
        outer_owner,
        vec!["Outer".to_string()],
    ));

    let body_owner = DefId::new(CrateId(0), LocalDefId(7));
    let context = crate::lower::body_context::BodyLoweringContext::new(
        BodyOwner::Function(body_owner),
        Some(body_owner),
        vec!["T".to_string()],
        HirGenericBounds::new(),
        true,
    );

    let ids = lowerer.with_body_context(context, |lowerer| {
        assert_eq!(
            lowerer.current_body_owner(),
            Some(BodyOwner::Function(body_owner))
        );
        assert_eq!(lowerer.current_generic_owner(), Some(body_owner));
        assert_eq!(lowerer.current_generic_params(), &["T".to_string()]);
        assert!(lowerer.is_in_unsafe());
        (lowerer.fresh_local_id(), lowerer.fresh_local_id())
    });

    assert_eq!(ids.0, HirLocalId(0));
    assert_eq!(ids.1, HirLocalId(1));
    assert_eq!(lowerer.current_generic_owner(), Some(outer_owner));
    assert_eq!(lowerer.current_generic_params(), &["Outer".to_string()]);
    assert!(!lowerer.is_in_unsafe());
    assert!(lowerer.current_body_owner().is_none());
}

#[test]
fn body_owner_mismatched_method_name_and_id_does_not_rediscover_method() {
    let mut lowerer = Lowerer::new_for_test();
    let impl_id = DefId::new(CrateId(0), LocalDefId(20));
    let first_method_id = DefId::new(CrateId(0), LocalDefId(21));
    let second_method_id = DefId::new(CrateId(0), LocalDefId(22));
    let mut first = test_function(first_method_id, "first", Vec::new());
    first.ret_type = Type::I32;
    let mut second = test_function(second_method_id, "second", Vec::new());
    second.ret_type = Type::Bool;
    let mut impl_def = test_impl(impl_id);
    impl_def.methods.insert("first".to_string(), first);
    impl_def.methods.insert("second".to_string(), second);
    lowerer.items.insert_impl(impl_def).unwrap();

    let context = BodyLoweringContext::new(
        BodyOwner::ImplMethod {
            impl_id,
            method_id: second_method_id,
            method_name: "first".to_string(),
        },
        Some(impl_id),
        Vec::new(),
        HirGenericBounds::new(),
        false,
    );

    let return_type =
        lowerer.with_body_context(context, |lowerer| lowerer.current_body_return_type());

    assert_eq!(return_type, None);
    assert!(!lowerer.diagnostics.is_empty());
}
