use super::*;

#[test]
fn index_mut_pair_diagnostic_uses_index_mut_method_span() {
    let mut lowerer = Lowerer::new_for_test();
    let index_trait_id = def_id(40);
    let index_mut_trait_id = def_id(41);
    let index_mut_method_id = def_id(42);
    let impl_id = def_id(43);
    let point_id = def_id(44);
    let span = Span {
        file_path: PathBuf::from("cell.rk"),
        start: 12,
        end: 24,
    };

    lowerer.language_items.index = Some(crate::language_items::IndexLanguageItems {
        trait_id: index_trait_id,
        output_id: AssocTypeId(1),
        method_id: def_id(45),
    });
    lowerer.language_items.index_mut = Some(crate::language_items::IndexMutLanguageItems {
        trait_id: index_mut_trait_id,
        output_id: AssocTypeId(2),
        method_id: index_mut_method_id,
    });
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Point".to_string()),
            type_name: "Point".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: point_id,
                args: Vec::new(),
            }),
            trait_name: Some("IndexMut".to_string()),
            trait_id: Some(index_mut_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::I64],
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([(
                "write_at".to_string(),
                HirFunction {
                    id: index_mut_method_id,
                    name: "write_at".to_string(),
                    generic_params: Vec::new(),
                    generic_bounds: HashMap::new().into(),
                    params: Vec::new(),
                    ret_type: Type::Reference {
                        mutable: true,
                        inner: Box::new(Type::I64),
                    },
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::Unit,
                            ty: Type::Unit,
                            span: span.clone(),
                        })],
                        ty: Type::Unit,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();
    lowerer.source_map.insert_symbol(
        crate::source_map::SourceSymbol::Definition(impl_id),
        span.clone(),
        Some(span.clone()),
        None,
    );

    let output = {
        let mut service =
            super::super::TraitConformanceService::new(super::super::TraitConformanceContext {
                items: &mut lowerer.items,
                engine: &mut lowerer.engine,
                resolver: &lowerer.resolver,
                dependency_resolvers: &lowerer.dependency_resolvers,
                root_crate_id: lowerer.root_crate_id,
                local_def_ids: &mut lowerer.local_def_ids,
                current_def_ids: &mut lowerer.current_def_ids,
                source_map: &lowerer.source_map,
                index_protocol_ids: Some(super::super::IndexProtocolIds {
                    index_trait: index_trait_id,
                    index_output: AssocTypeId(1),
                    index_mut_trait: index_mut_trait_id,
                    index_mut_output: AssocTypeId(2),
                }),
            });
        service.validate_index_mut_pairs();
        service.finish()
    };

    let diagnostic = output
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.message
                    == "IndexMut implementation for Point with key I64 requires a matching Index implementation"
            })
            .expect("missing IndexMut pairing diagnostic");
    assert!(!diagnostic.message.contains("DefId"));
    assert!(!diagnostic.message.contains("TypeVarId"));
    assert_eq!(diagnostic.code, crate::diagnostic::DiagnosticCode::Resolve);
    let diagnostic_span = diagnostic.span().expect("diagnostic should have a span");
    assert_eq!(diagnostic_span.start, span.start);
    assert_eq!(diagnostic_span.end, span.end);
}

#[test]
fn conformance_does_not_resolve_dependency_export_alias_without_import() {
    let mut lowerer = Lowerer::new_for_test();
    let point_id = def_id(30);
    let trait_id = DefId::new(CrateId(7), LocalDefId(31));
    register_point(&mut lowerer, point_id);
    lowerer
        .items
        .insert_trait_def(empty_trait(trait_id, "dep::Show"));

    let mut dep_resolver = ResolverTables::default();
    dep_resolver.item_paths.insert("Show".to_string(), trait_id);
    dep_resolver
        .item_paths
        .insert("dep::Show".to_string(), trait_id);
    dep_resolver
        .item_names_by_id
        .insert(trait_id, "dep::Show".to_string());
    dep_resolver.insert_export_alias_with_name(
        "Show".to_string(),
        "dep::Show".to_string(),
        trait_id,
    );
    lowerer
        .dependency_resolvers
        .insert("dep".to_string(), dep_resolver);
    lowerer
        .items
        .insert_impl(HirImpl {
            id: def_id(32),
            owner: HirImplOwner::Named("Point".to_string()),
            type_name: "Point".to_string(),
            type_generics: vec![],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: point_id,
                args: Vec::new(),
            }),
            trait_name: Some("Show".to_string()),
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert_eq!(lowerer.items.impl_def(def_id(32)).unwrap().trait_id, None);
}

#[test]
fn conformance_requires_resolver_id_for_trait_name() {
    let mut lowerer = Lowerer::new_for_test();
    let point_id = def_id(33);
    let trait_id = def_id(34);
    register_point(&mut lowerer, point_id);
    lowerer
        .items
        .insert_trait_def(empty_trait(trait_id, "Show"));
    lowerer
        .items
        .insert_impl(HirImpl {
            id: def_id(35),
            owner: HirImplOwner::Named("Point".to_string()),
            type_name: "Point".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Show".to_string()),
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert_eq!(lowerer.items.impl_def(def_id(35)).unwrap().trait_id, None);
}

#[test]
fn conformance_rejects_impl_method_missing_explicit_signature_arg() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(13);
    let impl_id = def_id(14);
    let method_id = def_id(15);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Sink".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "take".to_string(),
            HirFunctionSig {
                id: def_id(16),
                name: "take".to_string(),
                generic_params: Vec::new(),
                params: vec![
                    Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::Generic(trait_self)),
                    },
                    Type::I64,
                ],
                ret: Type::Bool,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::I32),
            trait_name: Some("Sink".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "take".to_string(),
                HirFunction {
                    id: method_id,
                    name: "take".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I32),
                        },
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::Bool,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Bool,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert!(lowerer.errors().iter().any(|error| {
        error
            .message
            .contains("method 'take' parameter count mismatch")
            && error.message.contains("expected 2")
            && error.message.contains("found 1")
    }));
}

#[test]
fn conformance_rejects_impl_method_receiver_mode_mismatch() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(420);
    let impl_id = def_id(421);
    let method_id = def_id(422);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "DropLike".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "drop".to_string(),
            HirFunctionSig {
                id: def_id(423),
                name: "drop".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Generic(trait_self)),
                }],
                ret: Type::Unit,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("DropLike".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "drop".to_string(),
                HirFunction {
                    id: method_id,
                    name: "drop".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I32,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::Unit,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Unit,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert!(lowerer.errors().iter().any(|error| {
        error
            .message
            .contains("method 'drop' parameter 0 type mismatch")
            && error.message.contains("trait 'DropLike'")
    }));
}

#[test]
fn conformance_rejects_impl_method_return_type_mismatch() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(424);
    let impl_id = def_id(425);
    let method_id = def_id(426);

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Flag".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "flag".to_string(),
            HirFunctionSig {
                id: def_id(427),
                name: "flag".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::I32],
                ret: Type::Bool,
                generic_bounds: HashMap::new().into(),
                self_receiver: None,
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Flag".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "flag".to_string(),
                HirFunction {
                    id: method_id,
                    name: "flag".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I32,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::I64,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::I64,
                    },
                    is_curried: false,
                    is_method: false,
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert!(lowerer.errors().iter().any(|error| {
        error.message.contains("method 'flag' return type mismatch")
            && error.message.contains("trait 'Flag'")
    }));
}

#[test]
fn conformance_does_not_recheck_dependency_impl_signatures() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(428);
    let impl_id = DefId::new(CrateId(7), LocalDefId(429));
    let method_id = DefId::new(CrateId(7), LocalDefId(430));

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Flag".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "flag".to_string(),
            HirFunctionSig {
                id: def_id(431),
                name: "flag".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::I32],
                ret: Type::Bool,
                generic_bounds: HashMap::new().into(),
                self_receiver: None,
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Flag".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "flag".to_string(),
                HirFunction {
                    id: method_id,
                    name: "flag".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I32,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::I64,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::I64,
                    },
                    is_curried: false,
                    is_method: false,
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert!(lowerer.errors().is_empty());
}

#[test]
fn conformance_rejects_missing_supertrait_implementation() {
    let mut lowerer = Lowerer::new_for_test();
    let parent_id = def_id(700);
    let child_id = def_id(701);
    let point_id = def_id(702);
    let impl_id = def_id(703);
    register_point(&mut lowerer, point_id);
    lowerer
        .items
        .insert_trait_def(empty_trait(parent_id, "Parent"));
    lowerer.items.insert_trait_def(HirTrait {
        id: child_id,
        name: "Child".to_string(),
        generic_params: Vec::new(),
        target: None,
        predicates: vec![crate::types::Predicate::Trait {
            subject: Type::Generic(GenericParamId {
                owner: child_id,
                index: 0,
            }),
            trait_id: parent_id,
            args: Vec::new(),
        }],
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    });
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Point".to_string()),
            type_name: "Point".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: point_id,
                args: Vec::new(),
            }),
            trait_name: Some("Child".to_string()),
            trait_id: Some(child_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: crate::hir::HirGenericBounds::new(),
            methods: HashMap::new(),
        })
        .unwrap();
    lowerer.current_def_ids.insert(impl_id);

    lowerer.check_trait_conformance();

    assert!(
        lowerer.errors().iter().any(|error| {
            error
                .message
                .contains("does not satisfy supertrait obligation")
                && error.message.contains("Parent")
        }),
        "unexpected diagnostics: {:?}",
        lowerer.errors()
    );
}
