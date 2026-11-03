use super::*;

#[test]
fn generic_box_static_constructor_value_lowers() {
    let resolved = compile_source_to_resolved_hir_for_test(
        r#"
struct Box T
    value: T

impl Box T
    new = value ->
        Box
            value: value

main = ->
    Box::new 21
"#,
    );

    assert!(resolved.program.test_function_by_name("main").is_some());
}

fn resolved_generic_static_method(
    _lowerer: &mut Lowerer,
    impl_id: DefId,
    method_id: DefId,
    struct_id: DefId,
    owner_param: GenericParamId,
    method_params: Vec<GenericParamId>,
    params: Vec<Type>,
    ret: Type,
) -> crate::lower::resolution::LowerResolvedStaticMethod {
    let mut method = test_function(method_id, "new");
    method.generic_params = method_params
        .iter()
        .enumerate()
        .map(|(index, param)| GenericParamDecl::type_param(*param, format!("Method{index}")))
        .collect();
    method.params = params
        .iter()
        .enumerate()
        .map(|(index, ty)| HirParam {
            name: format!("arg{index}"),
            local_id: HirLocalId(index as u32),
            ty: ty.clone(),
            mutable: false,
            is_ref: false,
        })
        .collect();
    method.ret_type = ret.clone();
    crate::lower::resolution::LowerResolvedStaticMethod {
        value: crate::lower::resolution::LowerResolvedValue {
            name: "Box::new".to_string(),
            ty: Type::function(params, ret),
            target: Some(HirVarTarget::Function(method_id)),
            is_alias: false,
            scope_index: None,
            should_instantiate: true,
        },
        target: HirMethodCallTarget::impl_method(impl_id, method_id, None),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: struct_id,
            args: vec![Type::Generic(owner_param)],
        }),
        owner_generic_params: vec![owner_param],
        method_generic_params: method_params,
    }
}

#[test]
fn trait_backed_structural_static_owner_substitutes_trait_args_for_artifact_validation() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(160);
    let trait_id = def_id(159);
    let trait_member_id = def_id(1600);
    let impl_id = def_id(161);
    let method_id = def_id(162);
    let owner_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let mut resolved = resolved_generic_static_method(
        &mut lowerer,
        impl_id,
        method_id,
        struct_id,
        owner_param,
        vec![method_param],
        vec![Type::Struct {
            id: struct_id,
            args: vec![Type::Generic(method_param)],
        }],
        Type::Unit,
    );
    resolved.target = HirMethodCallTarget::impl_method(
        impl_id,
        method_id,
        Some(HirSelectedTraitMember {
            trait_id,
            member_id: trait_member_id,
            trait_args: vec![Type::Generic(owner_param)],
        }),
    );

    let (_, target) = lowerer
        .with_test_body_context(|lowerer| {
            lowerer.instantiate_static_method_value(resolved, crate::lexer::Span::test())
        })
        .expect("structural owner inference");

    assert_eq!(target.method.owner_substitution[0].param, owner_param);
    assert_eq!(target.method.method_substitution[0].param, method_param);
    assert_eq!(
        target.method.owner_substitution[0].ty,
        target.method.method_substitution[0].ty
    );
    assert!(matches!(
        target.method.owner_substitution[0].ty,
        Type::TypeVar(_)
    ));
    let HirSelectedMethodTarget::ImplMethod {
        selected_trait: Some(selected_trait),
        ..
    } = target.method.target
    else {
        panic!("expected trait-backed static method target")
    };
    assert_eq!(
        selected_trait.trait_args,
        vec![target.method.owner_substitution[0].ty.clone()]
    );
}

#[test]
fn structural_static_owner_fallback_preserves_nested_method_generic_shape() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(163);
    let impl_id = def_id(164);
    let method_id = def_id(165);
    let owner_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let resolved = resolved_generic_static_method(
        &mut lowerer,
        impl_id,
        method_id,
        struct_id,
        owner_param,
        vec![method_param],
        vec![Type::Struct {
            id: struct_id,
            args: vec![Type::Tuple(vec![Type::Generic(method_param)])],
        }],
        Type::Unit,
    );

    let (_, target) = lowerer
        .with_test_body_context(|lowerer| {
            lowerer.instantiate_static_method_value(resolved, crate::lexer::Span::test())
        })
        .expect("nested structural owner inference");

    assert_eq!(target.method.owner_substitution[0].param, owner_param);
    assert_eq!(target.method.method_substitution[0].param, method_param);
    assert_eq!(
        target.method.owner_substitution[0].ty,
        Type::Tuple(vec![target.method.method_substitution[0].ty.clone()])
    );
}

#[test]
fn structural_static_owner_fallback_visits_projection_nominal_types() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(169);
    let impl_id = def_id(170);
    let method_id = def_id(171);
    let trait_id = def_id(172);
    let owner_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let resolved = resolved_generic_static_method(
        &mut lowerer,
        impl_id,
        method_id,
        struct_id,
        owner_param,
        vec![method_param],
        vec![Type::Projection {
            ty: Box::new(Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(method_param)],
            }),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: crate::ids::AssocTypeId(0),
            },
            trait_args: Vec::new(),
        }],
        Type::Unit,
    );

    let (_, target) = lowerer
        .with_test_body_context(|lowerer| {
            lowerer.instantiate_static_method_value(resolved, crate::lexer::Span::test())
        })
        .expect("projection structural owner inference");

    assert_eq!(
        target.method.owner_substitution[0].ty,
        target.method.method_substitution[0].ty
    );
    assert!(matches!(
        target.method.owner_substitution[0].ty,
        Type::TypeVar(_)
    ));
}

#[test]
fn conflicting_structural_static_owner_occurrences_are_an_error() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(166);
    let impl_id = def_id(167);
    let method_id = def_id(168);
    let owner_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let left_method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let right_method_param = GenericParamId {
        owner: method_id,
        index: 1,
    };
    lowerer
        .items
        .insert_structure(empty_struct(struct_id, "Box"));
    register_item_path(&mut lowerer, "Box", struct_id);
    let mut method = test_function(method_id, "new");
    method.generic_params = vec![
        GenericParamDecl::type_param(left_method_param, "Left"),
        GenericParamDecl::type_param(right_method_param, "Right"),
    ];
    method.params = vec![
        HirParam {
            name: "left".to_string(),
            local_id: HirLocalId(0),
            ty: Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(left_method_param)],
            },
            mutable: false,
            is_ref: false,
        },
        HirParam {
            name: "right".to_string(),
            local_id: HirLocalId(1),
            ty: Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(right_method_param)],
            },
            mutable: false,
            is_ref: false,
        },
    ];
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: vec![GenericParamDecl::type_param(owner_param, "Owner")],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(owner_param)],
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("new".to_string(), method)]),
        })
        .unwrap();

    let expr = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_identifier_path(&identifier_path(&["Box", "new"]))
    });

    assert_eq!(expr.ty, Type::Error);
    assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "<error>"));
    let diagnostic = lowerer
        .diagnostics
        .errors()
        .iter()
        .find(|error| error.message.contains("conflicting owner inference"))
        .expect("conflicting owner inference diagnostic");
    assert!(diagnostic.message.contains("_ vs _"));
    assert!(!diagnostic.message.contains("GenericParamId"));
}

fn register_static_impl(
    lowerer: &mut Lowerer,
    impl_id: DefId,
    owner_id: DefId,
    owner_name: &str,
    method_name: &str,
    method: HirFunction,
) {
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named(owner_name.to_string()),
            type_name: owner_name.to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner_id,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([(method_name.to_string(), method)]),
        })
        .unwrap();
}

#[test]
fn lower_static_impl_reference_records_method_id() {
    let mut lowerer = Lowerer::new_for_test();
    let box_id = def_id(40);
    lowerer.items.insert_structure(HirStruct {
        id: box_id,
        name: "Box".to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    });
    register_item_path(&mut lowerer, "Box", box_id);
    let method_id = def_id(41);
    register_static_impl(
        &mut lowerer,
        def_id(42),
        box_id,
        "Box",
        "make",
        test_function(method_id, "make"),
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["Box", "make"]));

    let (reference, target) = static_method_value_parts(&expr);
    assert_eq!(reference.name, "Box::make");
    assert_eq!(reference.target, HirVarTarget::Function(method_id));
    assert_eq!(target.method.impl_id(), Some(def_id(42)));
    assert_eq!(target.method.method_id(), Some(method_id));
}

#[test]
fn generic_static_method_value_preserves_owner_generic_param_id() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(43);
    let impl_id = def_id(44);
    let method_id = def_id(45);
    let owner_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    lowerer.items.insert_structure(HirStruct {
        id: struct_id,
        name: "Widget".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: struct_id,
                index: 0,
            },
            "StructDisplay",
        )],
        fields: Vec::new(),
    });
    register_item_path(&mut lowerer, "Widget", struct_id);

    let mut method = test_function(method_id, "create");
    method.generic_params = vec![GenericParamDecl::type_param(method_param, "MethodDisplay")];
    method.params = vec![
        HirParam {
            name: "owner".to_string(),
            local_id: HirLocalId(0),
            ty: Type::Generic(owner_param),
            mutable: false,
            is_ref: false,
        },
        HirParam {
            name: "value".to_string(),
            local_id: HirLocalId(1),
            ty: Type::Generic(method_param),
            mutable: false,
            is_ref: false,
        },
    ];
    method.ret_type = Type::Tuple(vec![
        Type::Generic(owner_param),
        Type::Generic(method_param),
    ]);
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Widget".to_string()),
            type_name: "Widget".to_string(),
            type_generics: vec![GenericParamDecl::type_param(owner_param, "ImplDisplay")],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(owner_param)],
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("create".to_string(), method)]),
        })
        .unwrap();

    let expr = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_identifier_path(&identifier_path(&["Widget", "create"]))
    });

    let (reference, target) = static_method_value_parts(&expr);
    assert_eq!(reference.target, HirVarTarget::Function(method_id));
    assert_eq!(target.method.impl_id(), Some(impl_id));
    assert_eq!(target.method.method_id(), Some(method_id));
    assert_eq!(target.method.owner_substitution.len(), 1);
    assert_eq!(target.method.owner_substitution[0].param, owner_param);
    assert_eq!(target.method.method_substitution.len(), 1);
    assert_eq!(target.method.method_substitution[0].param, method_param);
}

#[test]
fn enum_static_method_value_preserves_exact_impl_and_method_authority() {
    let mut lowerer = Lowerer::new_for_test();
    let enum_id = def_id(50);
    let impl_id = def_id(51);
    let method_id = def_id(52);
    lowerer.items.insert_enumeration(HirEnum {
        id: enum_id,
        name: "Choice".to_string(),
        generic_params: Vec::new(),
        variants: Vec::new(),
    });
    register_item_path(&mut lowerer, "Choice", enum_id);
    let method = test_function(method_id, "default");
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Choice".to_string()),
            type_name: "Choice".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Enum {
                id: enum_id,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("default".to_string(), method)]),
        })
        .unwrap();

    let expr = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_identifier_path(&identifier_path(&["Choice", "default"]))
    });

    let (reference, target) = static_method_value_parts(&expr);
    assert_eq!(reference.target, HirVarTarget::Function(method_id));
    assert_eq!(target.method.impl_id(), Some(impl_id));
    assert_eq!(target.method.method_id(), Some(method_id));
}

#[test]
fn canonical_static_owner_without_method_is_an_error_not_a_raw_var() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(53);
    lowerer
        .items
        .insert_structure(empty_struct(struct_id, "Widget"));
    register_item_path(&mut lowerer, "Widget", struct_id);

    let expr = lowerer.lower_identifier_path(&identifier_path(&["Widget", "missing"]));

    assert_eq!(expr.ty, Type::Error);
    assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "<error>"));
    assert!(lowerer
        .diagnostics
        .errors()
        .iter()
        .any(|error| { error.message.contains("Widget::missing") && error.span().is_some() }));
}

#[test]
fn trait_backed_generic_static_method_preserves_member_and_trait_args() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(59);
    let trait_id = def_id(60);
    let trait_member_id = def_id(61);
    let impl_id = def_id(62);
    let method_id = def_id(63);
    let owner_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    lowerer.items.insert_structure(HirStruct {
        id: struct_id,
        name: "Tagged".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: struct_id,
                index: 0,
            },
            "NominalDisplay",
        )],
        fields: Vec::new(),
    });
    register_item_path(&mut lowerer, "Tagged", struct_id);
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Factory".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "TraitArgumentDisplay",
        )],
        associated_types: Vec::new(),
        methods: HashMap::from([(
            "create".to_string(),
            test_function(trait_member_id, "create"),
        )]),
        signatures: HashMap::new(),
    });
    let mut method = test_function(method_id, "create");
    method.generic_params = vec![GenericParamDecl::type_param(method_param, "MethodDisplay")];
    method.params = vec![HirParam {
        name: "value".to_string(),
        local_id: HirLocalId(0),
        ty: Type::Struct {
            id: struct_id,
            args: vec![Type::Generic(method_param)],
        },
        mutable: false,
        is_ref: false,
    }];
    method.ret_type = Type::Struct {
        id: struct_id,
        args: vec![Type::Generic(method_param)],
    };
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Tagged".to_string()),
            type_name: "Tagged".to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                owner_param,
                "ImplOwnerDisplay",
            )],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(owner_param)],
            }),
            trait_name: Some("Factory".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(owner_param, "TraitDisplay")],
            trait_arg_types: vec![Type::Generic(owner_param)],
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("create".to_string(), method)]),
        })
        .unwrap();

    let expr = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_identifier_path(&identifier_path(&["Tagged", "create"]))
    });

    let (_, target) = static_method_value_parts(&expr);
    assert_eq!(target.method.impl_id(), Some(impl_id));
    assert_eq!(target.method.method_id(), Some(method_id));
    assert_eq!(target.method.owner_substitution.len(), 1);
    assert_eq!(target.method.owner_substitution[0].param, owner_param);
    assert_eq!(target.method.method_substitution.len(), 1);
    assert_eq!(target.method.method_substitution[0].param, method_param);
    assert!(matches!(
        target.method.owner_substitution[0].ty,
        Type::TypeVar(_)
    ));
    assert_eq!(
        target.method.owner_substitution[0].ty,
        target.method.method_substitution[0].ty
    );
    let HirSelectedMethodTarget::ImplMethod {
        selected_trait: Some(selected_trait),
        ..
    } = &target.method.target
    else {
        panic!("expected trait-backed impl method authority")
    };
    assert_eq!(selected_trait.trait_id, trait_id);
    assert_eq!(selected_trait.member_id, trait_member_id);
    assert_eq!(selected_trait.trait_args.len(), 1);
    assert_eq!(
        selected_trait.trait_args[0],
        target.method.owner_substitution[0].ty
    );
}

#[test]
fn trait_backed_static_method_with_unknown_trait_authority_is_an_error() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(46);
    let impl_id = def_id(47);
    let method_id = def_id(48);
    let missing_trait_id = def_id(49);
    lowerer
        .items
        .insert_structure(empty_struct(struct_id, "Widget"));
    register_item_path(&mut lowerer, "Widget", struct_id);
    let method = test_function(method_id, "create");
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Widget".to_string()),
            type_name: "Widget".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: Vec::new(),
            }),
            trait_name: Some("MissingTrait".to_string()),
            trait_id: Some(missing_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("create".to_string(), method)]),
        })
        .unwrap();

    let expr = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_identifier_path(&identifier_path(&["Widget", "create"]))
    });

    assert_eq!(expr.ty, Type::Error);
    assert!(!matches!(
        expr.kind,
        HirExprKind::ResolvedVar(HirVarRef {
            target: HirVarTarget::Function(id),
            ..
        }) if id == method_id
    ));
    assert!(lowerer
        .diagnostics
        .errors()
        .iter()
        .any(|error| { error.message.contains("unknown trait") && error.span().is_some() }));
}

#[test]
fn module_local_struct_alias_shadows_root_struct_for_static_method() {
    let mut lowerer = Lowerer::new_for_test();
    let root_struct_id = def_id(44);
    let module_struct_id = def_id(45);
    let root_method_id = def_id(46);
    let module_method_id = def_id(47);
    lowerer
        .items
        .insert_structure(empty_struct(root_struct_id, "Thing"));
    lowerer
        .items
        .insert_structure(empty_struct(module_struct_id, "demo::helper::Thing"));
    register_static_impl(
        &mut lowerer,
        def_id(146),
        root_struct_id,
        "Thing",
        "make",
        test_function(root_method_id, "make"),
    );
    register_static_impl(
        &mut lowerer,
        def_id(147),
        module_struct_id,
        "demo::helper::Thing",
        "make",
        test_function(module_method_id, "make"),
    );
    lowerer
        .resolver
        .item_paths
        .insert("Thing".to_string(), root_struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_struct_id, "Thing".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "Thing".to_string(),
        "demo::helper::Thing".to_string(),
        module_struct_id,
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["Thing", "make"]));

    let (reference, target) = static_method_value_parts(&expr);
    assert_eq!(reference.name, "demo::helper::Thing::make");
    assert_eq!(reference.target, HirVarTarget::Function(module_method_id));
    assert_eq!(target.method.impl_id(), Some(def_id(147)));
    assert_eq!(target.method.method_id(), Some(module_method_id));
}

#[test]
fn import_alias_struct_resolves_static_impl_method_by_canonical_owner() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(48);
    let method_id = def_id(49);
    lowerer
        .items
        .insert_structure(empty_struct(struct_id, "std::string::String"));
    lowerer.resolver.insert_import_alias_with_name(
        "String".to_string(),
        "std::string::String".to_string(),
        struct_id,
    );
    lowerer
        .items
        .insert_impl(HirImpl {
            id: def_id(50),
            owner: HirImplOwner::Named("std::string::String".to_string()),
            type_name: "String".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("from".to_string(), test_function(method_id, "from"))]),
        })
        .unwrap();
    let expr = lowerer.lower_identifier_path(&identifier_path(&["String", "from"]));

    let (reference, target) = static_method_value_parts(&expr);
    assert_eq!(reference.name, "std::string::String::from");
    assert_eq!(reference.target, HirVarTarget::Function(method_id));
    assert_eq!(target.method.impl_id(), Some(def_id(50)));
    assert_eq!(target.method.method_id(), Some(method_id));
}

#[test]
fn import_alias_struct_without_canonical_name_does_not_resolve_static_impl_method_by_id() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(51);
    lowerer
        .items
        .insert_structure(empty_struct(struct_id, "String"));
    lowerer
        .resolver
        .import_aliases
        .insert("String".to_string(), struct_id);
    let expr = lowerer.lower_identifier_path(&identifier_path(&["String", "from"]));

    match expr.kind {
        HirExprKind::Var(name) => assert_eq!(name, "String::from"),
        other => panic!("expected unresolved static method path, got {other:?}"),
    }
}

#[test]
fn import_alias_struct_uses_canonical_static_impl_owner() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(53);
    let canonical_method_id = def_id(55);
    lowerer
        .items
        .insert_structure(empty_struct(struct_id, "String"));
    lowerer.resolver.insert_import_alias_with_name(
        "String".to_string(),
        "std::string::String".to_string(),
        struct_id,
    );
    register_static_impl(
        &mut lowerer,
        def_id(155),
        struct_id,
        "std::string::String",
        "from",
        test_function(canonical_method_id, "from"),
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["String", "from"]));

    let (reference, target) = static_method_value_parts(&expr);
    assert_eq!(reference.name, "std::string::String::from");
    assert_eq!(
        reference.target,
        HirVarTarget::Function(canonical_method_id)
    );
    assert_eq!(target.method.impl_id(), Some(def_id(155)));
    assert_eq!(target.method.method_id(), Some(canonical_method_id));
}
