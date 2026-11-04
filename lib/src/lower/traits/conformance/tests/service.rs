use super::*;

#[test]
fn trait_fixture_is_keyed_by_its_exact_id() {
    let expected_id = def_id(10);
    let wrong_id = def_id(11);
    let traits = HashMap::from([(
        wrong_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: wrong_id,
            name: "Debug".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    )]);

    assert!(traits.get(&expected_id).is_none());
    assert_eq!(traits.get(&wrong_id).unwrap().id, wrong_id);
}

#[test]
fn trait_conformance_service_uses_explicit_deps_for_selection_and_required_items() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(130);
    let impl_id = def_id(131);
    let assoc_id = AssocTypeId(0);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "module::Complete".to_string(),
        generic_params: vec![],
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "finish".to_string(),
            HirFunctionSig {
                id: def_id(132),
                name: "finish".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(trait_self)],
                ret: Type::I64,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });
    lowerer.resolver.insert_import_alias_with_name(
        "Complete".to_string(),
        "module::Complete".to_string(),
        trait_id,
    );
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Complete".to_string()),
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    let output = {
        let mut service = super::TraitConformanceService::new(super::TraitConformanceContext {
            items: &mut lowerer.items,
            engine: &mut lowerer.engine,
            resolver: &lowerer.resolver,
            dependency_resolvers: &lowerer.dependency_resolvers,
            root_crate_id: lowerer.root_crate_id,
            local_def_ids: &mut lowerer.local_def_ids,
            current_def_ids: &mut lowerer.current_def_ids,
            source_map: &lowerer.source_map,
            index_protocol_ids: None,
        });
        service.check_trait_conformance();
        service.finish()
    };

    assert_eq!(
        lowerer.items.impl_def(impl_id).unwrap().trait_id,
        Some(trait_id)
    );
    assert!(output
        .diagnostics
        .iter()
        .any(|error| error.message.contains("required associated type 'Output'")));
    assert!(output
        .diagnostics
        .iter()
        .any(|error| error.message.contains("required method 'finish'")));
}

#[test]
fn lowerer_conformance_delegate_reports_required_items() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(140);
    let impl_id = def_id(141);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Complete".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "finish".to_string(),
            HirFunctionSig {
                id: def_id(142),
                name: "finish".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(trait_self)],
                ret: Type::I64,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Complete".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert!(lowerer
        .errors()
        .iter()
        .any(|error| error.message.contains("required method 'finish'")));
}
