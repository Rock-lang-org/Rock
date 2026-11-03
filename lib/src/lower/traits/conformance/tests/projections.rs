use super::*;

#[test]
fn trait_conformance_display_context_renders_generic_projection_names() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(80);
    let generic = GenericParamDecl::type_param(
        GenericParamId {
            owner: trait_id,
            index: 0,
        },
        "T",
    );
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "Render".to_string());
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Render".to_string(),
        generic_params: vec![generic],
        associated_types: vec![HirAssociatedTypeDecl {
            id: AssocTypeId(0),
            name: "Item".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
    });

    let service =
        super::super::TraitConformanceService::new(super::super::TraitConformanceContext {
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
    let ty = Type::Projection {
        ty: Box::new(Type::Generic(GenericParamId {
            owner: trait_id,
            index: 0,
        })),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![Type::Generic(GenericParamId {
            owner: trait_id,
            index: 0,
        })],
    };

    assert_eq!(service.display_type(&ty), "<T as Render T>::Item");
}

#[test]
fn conformance_uses_resolved_trait_id_for_projection_substitution() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(30);
    let impl_id = def_id(31);
    let method_id = def_id(32);
    let assoc_id = AssocTypeId(0);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let ret_ty = lowerer.engine.fresh_type_var();

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Deref".to_string(),
        generic_params: vec![],
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_id,
            name: "Target".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "deref".to_string(),
            HirFunctionSig {
                id: def_id(45),
                name: "deref".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(trait_self)],
                ret: Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Projection {
                        ty: Box::new(Type::Generic(trait_self)),
                        trait_id,
                        assoc_type: AssociatedTypeKey {
                            owner: trait_id,
                            assoc_type_id: assoc_id,
                        },
                        trait_args: Vec::new(),
                    }),
                },
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });
    lowerer
        .resolver
        .item_paths
        .insert("Deref".to_string(), trait_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "Deref".to_string());

    let method = HirFunction {
        id: method_id,
        name: "deref".to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        }],
        ret_type: ret_ty,
        body: HirBlock {
            stmts: vec![],
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Deref".to_string()),
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_id,
                name: "Target".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("deref".to_string(), method)]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let imp = lowerer.items.impl_def(impl_id).unwrap();
    assert_eq!(imp.trait_id, Some(trait_id));
    assert_eq!(
        imp.methods["deref"].ret_type,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        }
    );
}

#[test]
fn conformance_does_not_substitute_projection_owned_by_different_trait() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(50);
    let other_trait_id = def_id(51);
    let impl_id = def_id(52);
    let method_id = def_id(53);
    let assoc_id = AssocTypeId(0);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let ret_ty = lowerer.engine.fresh_type_var();

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Deref".to_string(),
        generic_params: vec![],
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_id,
            name: "Target".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "deref".to_string(),
            HirFunctionSig {
                id: def_id(54),
                name: "deref".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(trait_self)],
                ret: Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Projection {
                        ty: Box::new(Type::Generic(trait_self)),
                        trait_id,
                        assoc_type: AssociatedTypeKey {
                            owner: other_trait_id,
                            assoc_type_id: assoc_id,
                        },
                        trait_args: Vec::new(),
                    }),
                },
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
            trait_name: Some("Deref".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_id,
                name: "Target".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "deref".to_string(),
                HirFunction {
                    id: method_id,
                    name: "deref".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I64,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: ret_ty,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I64),
                        },
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

    assert!(matches!(
        &lowerer.items.impl_def(impl_id).unwrap().methods["deref"].ret_type,
        Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::Projection { .. })
    ));
}

#[test]
fn conformance_does_not_substitute_projection_with_different_trait_args() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(55);
    let impl_id = def_id(56);
    let method_id = def_id(57);
    let assoc_id = AssocTypeId(0);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let ret_ty = lowerer.engine.fresh_type_var();

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Project".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "T",
        )],
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "project".to_string(),
            HirFunctionSig {
                id: def_id(58),
                name: "project".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(trait_self)],
                ret: Type::Projection {
                    ty: Box::new(Type::Generic(trait_self)),
                    trait_id,
                    assoc_type: AssociatedTypeKey {
                        owner: trait_id,
                        assoc_type_id: assoc_id,
                    },
                    trait_args: vec![Type::Bool],
                },
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
            trait_name: Some("Project".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "project".to_string(),
                HirFunction {
                    id: method_id,
                    name: "project".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I64,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: ret_ty,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Projection {
                            ty: Box::new(Type::I64),
                            trait_id,
                            assoc_type: AssociatedTypeKey {
                                owner: trait_id,
                                assoc_type_id: assoc_id,
                            },
                            trait_args: vec![Type::Bool],
                        },
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

    assert!(matches!(
        &lowerer.items.impl_def(impl_id).unwrap().methods["project"].ret_type,
        Type::Projection { trait_args, .. } if trait_args.as_slice() == [Type::Bool]
    ));
}

#[test]
fn conformance_does_not_substitute_projection_on_non_self_generic_base() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(70);
    let impl_id = def_id(71);
    let method_id = def_id(72);
    let assoc_id = AssocTypeId(0);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let non_self_projection = Type::Projection {
        ty: Box::new(Type::Generic(trait_generic)),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: assoc_id,
        },
        trait_args: vec![Type::Generic(trait_generic)],
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Project".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "T",
        )],
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::from([(
            "value".to_string(),
            HirFunction {
                id: method_id,
                name: "value".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![
                    HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    },
                    HirParam {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: non_self_projection.clone(),
                        mutable: false,
                        is_ref: false,
                    },
                ],
                ret_type: non_self_projection.clone(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("value".to_string()),
                        ty: non_self_projection.clone(),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: non_self_projection.clone(),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Project".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::Bool,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["value"];
    assert!(matches!(method.params[1].ty, Type::Projection { .. }));
    assert!(matches!(method.ret_type, Type::Projection { .. }));
}
