use super::*;

#[test]
fn conformance_resolves_generic_impl_self_type_from_canonical_owner() {
    let mut lowerer = Lowerer::new_for_test();

    let trait_id = def_id(60);
    let impl_id = def_id(61);
    let default_method_id = def_id(62);
    let local_box_id = def_id(63);
    let local_enum_box_id = def_id(64);
    let dependency_box_id = DefId::new(CrateId(2), LocalDefId(10));
    let trait_self = Type::Generic(GenericParamId {
        owner: trait_id,
        index: 0,
    });

    lowerer.items.insert_structure(HirStruct {
        id: local_box_id,
        name: "Box".to_string(),
        generic_params: vec![],
        fields: vec![],
    });
    lowerer.items.insert_enumeration(HirEnum {
        id: local_enum_box_id,
        name: "Box".to_string(),
        generic_params: vec![],
        variants: vec![],
    });
    lowerer.items.insert_structure(HirStruct {
        id: dependency_box_id,
        name: "Box".to_string(),
        generic_params: vec![],
        fields: vec![],
    });
    lowerer
        .resolver
        .item_paths
        .insert("Box".to_string(), local_box_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(local_box_id, "Box".to_string());
    let mut dep_resolver = ResolverTables::default();
    dep_resolver
        .item_paths
        .insert("dep_b::Box".to_string(), dependency_box_id);
    dep_resolver
        .item_names_by_id
        .insert(dependency_box_id, "dep_b::Box".to_string());
    lowerer
        .dependency_resolvers
        .insert("dep_b".to_string(), dep_resolver);

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "CloneSelf".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::from([(
            "clone_self".to_string(),
            HirFunction {
                id: default_method_id,
                name: "clone_self".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: trait_self.clone(),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: trait_self.clone(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("self".to_string()),
                        ty: trait_self.clone(),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: trait_self.clone(),
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
            owner: HirImplOwner::Named("dep_b::Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: vec![],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: dependency_box_id,
                args: Vec::new(),
            }),
            trait_name: Some("CloneSelf".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["clone_self"];
    let expected_self = Type::Struct {
        id: dependency_box_id,
        args: vec![],
    };
    assert_eq!(method.params[0].ty, expected_self);
    assert_eq!(method.ret_type, expected_self);
    assert!(
        lowerer.errors().is_empty(),
        "unexpected errors: {:?}",
        lowerer.errors()
    );
}

#[test]
fn test_conformance_preserves_generic_borrowed_slice_self_param() {
    let mut lowerer = Lowerer::new_for_test();
    let self_ty = lowerer.engine.fresh_type_var();

    let trait_id = def_id(1);
    let impl_id = def_id(2);

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Show".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "show".to_string(),
            HirFunctionSig {
                id: def_id(42),
                name: "show".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(GenericParamId {
                    owner: trait_id,
                    index: 0,
                })],
                ret: Type::I32,
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
            owner: HirImplOwner::Named("&[T]".to_string()),
            type_name: "&[T]".to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                "T",
            )],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Reference {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })))),
            }),
            trait_name: Some("Show".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "show".to_string(),
                HirFunction {
                    id: def_id(3),
                    name: "show".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: self_ty,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::I32,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::I32,
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

    assert_eq!(
        lowerer.items.impl_def(impl_id).unwrap().methods["show"].params[0].ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Slice(Box::new(Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0
            })))),
        }
    );
}

#[test]
fn conformance_substitutes_zero_based_declared_trait_generics() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(10);
    let impl_id = def_id(11);
    let method_id = def_id(12);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let value_ty = lowerer.engine.fresh_type_var();
    let ret_ty = lowerer.engine.fresh_type_var();

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Sink".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "take".to_string(),
            HirFunctionSig {
                id: def_id(43),
                name: "take".to_string(),
                generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
                params: vec![Type::Generic(trait_self), Type::Generic(trait_generic)],
                ret: Type::Generic(trait_generic),
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
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::I32),
            trait_name: Some("Sink".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "Bool",
            )],
            trait_arg_types: vec![Type::Bool],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "take".to_string(),
                HirFunction {
                    id: method_id,
                    name: "take".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![
                        HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I32,
                            mutable: false,
                            is_ref: false,
                        },
                        HirParam {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: value_ty,
                            mutable: false,
                            is_ref: false,
                        },
                    ],
                    ret_type: ret_ty,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Bool,
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

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["take"];
    assert_eq!(method.params[1].ty, Type::Bool);
    assert_eq!(method.ret_type, Type::Bool);
}

#[test]
fn conformance_substitutes_trait_arg_types_not_display_names() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(20);
    let impl_id = def_id(21);
    let method_id = def_id(22);
    let box_id = def_id(23);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let boxed_i64 = Type::Struct {
        id: box_id,
        args: vec![Type::I64],
    };
    let value_ty = lowerer.engine.fresh_type_var();
    let ret_ty = lowerer.engine.fresh_type_var();

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Sink".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "take".to_string(),
            HirFunctionSig {
                id: def_id(44),
                name: "take".to_string(),
                generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
                params: vec![Type::Generic(trait_self), Type::Generic(trait_generic)],
                ret: Type::Generic(trait_generic),
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
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Sink".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "Box",
            )],
            trait_arg_types: vec![boxed_i64.clone()],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "take".to_string(),
                HirFunction {
                    id: method_id,
                    name: "take".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![
                        HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I32,
                            mutable: false,
                            is_ref: false,
                        },
                        HirParam {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: value_ty,
                            mutable: false,
                            is_ref: false,
                        },
                    ],
                    ret_type: ret_ty,
                    body: HirBlock {
                        stmts: vec![],
                        ty: boxed_i64.clone(),
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

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["take"];
    assert_eq!(method.params[1].ty, boxed_i64);
    assert_eq!(method.ret_type, boxed_i64);
}
