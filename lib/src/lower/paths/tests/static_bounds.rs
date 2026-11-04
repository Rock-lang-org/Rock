use super::*;

#[test]
fn ambiguous_static_bound_method_is_an_error_without_internal_candidate_ids() {
    let mut lowerer = Lowerer::new_for_test();
    let generic_owner = def_id(53);
    let owner_param = GenericParamId {
        owner: generic_owner,
        index: 0,
    };
    let left_trait = def_id(54);
    let left_member = def_id(55);
    let right_trait = def_id(56);
    let right_member = def_id(57);
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: left_trait,
        name: "Left".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("build".to_string(), test_function(left_member, "build"))]),
        signatures: HashMap::new(),
    });
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: right_trait,
        name: "Right".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("build".to_string(), test_function(right_member, "build"))]),
        signatures: HashMap::new(),
    });
    let bounds = HashMap::from([(
        owner_param,
        vec![
            TraitBound {
                trait_id: right_trait,
                type_args: Vec::new(),
            },
            TraitBound {
                trait_id: left_trait,
                type_args: Vec::new(),
            },
        ],
    )]);

    let expr = lowerer.with_body_context(
        BodyLoweringContext::new(
            BodyOwner::Function(def_id(58)),
            Some(generic_owner),
            vec!["T".to_string()],
            bounds.into(),
            false,
        ),
        |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "build"])),
    );

    assert_eq!(expr.ty, Type::Error);
    assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "<error>"));
    let diagnostic = lowerer
        .diagnostics
        .errors()
        .iter()
        .find(|error| error.message.contains("ambiguous static bound method"))
        .expect("ambiguity diagnostic");
    assert!(diagnostic.message.contains("matching trait bounds"));
    assert!(!diagnostic.message.contains("DefId"));
    assert!(diagnostic.span().is_some());
}

#[test]
fn static_bound_method_lookup_expands_supertraits() {
    let mut lowerer = Lowerer::new_for_test();
    let generic_owner = def_id(59);
    let owner_param = GenericParamId {
        owner: generic_owner,
        index: 0,
    };
    let parent_trait = def_id(60);
    let parent_member = def_id(61);
    let child_trait = def_id(62);
    let child_target = GenericParamId {
        owner: child_trait,
        index: 0,
    };
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: parent_trait,
        name: "Parent".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("build".to_string(), test_function(parent_member, "build"))]),
        signatures: HashMap::new(),
    });
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: vec![Predicate::Trait {
            subject: Type::Generic(child_target),
            trait_id: parent_trait,
            args: Vec::new(),
        }],
        id: child_trait,
        name: "Child".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    });
    let bounds = HashMap::from([(
        owner_param,
        vec![TraitBound {
            trait_id: child_trait,
            type_args: Vec::new(),
        }],
    )]);

    let expr = lowerer.with_body_context(
        BodyLoweringContext::new(
            BodyOwner::Function(def_id(63)),
            Some(generic_owner),
            vec!["T".to_string()],
            bounds.into(),
            false,
        ),
        |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "build"])),
    );

    let (_, target) = static_method_value_parts(&expr);
    assert_eq!(target.method.trait_id(), Some(parent_trait));
    assert_eq!(target.method.method_id(), Some(parent_member));
}

#[test]
fn distinct_static_bound_trait_args_remain_ambiguous() {
    let mut lowerer = Lowerer::new_for_test();
    let generic_owner = def_id(64);
    let owner_param = GenericParamId {
        owner: generic_owner,
        index: 0,
    };
    let trait_id = def_id(65);
    let member_id = def_id(66);
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
            "Arg",
        )],
        associated_types: Vec::new(),
        methods: HashMap::from([("build".to_string(), test_function(member_id, "build"))]),
        signatures: HashMap::new(),
    });
    let bounds = HashMap::from([(
        owner_param,
        vec![
            TraitBound {
                trait_id,
                type_args: vec![Type::I64],
            },
            TraitBound {
                trait_id,
                type_args: vec![Type::Bool],
            },
        ],
    )]);

    let expr = lowerer.with_body_context(
        BodyLoweringContext::new(
            BodyOwner::Function(def_id(67)),
            Some(generic_owner),
            vec!["T".to_string()],
            bounds.into(),
            false,
        ),
        |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "build"])),
    );

    assert_eq!(expr.ty, Type::Error);
    assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "<error>"));
    let diagnostic = lowerer
        .diagnostics
        .errors()
        .iter()
        .find(|error| error.message.contains("ambiguous static bound method"))
        .expect("ambiguity diagnostic");
    assert!(diagnostic.message.contains("Bool"));
    assert!(diagnostic.message.contains("I64"));
    assert!(diagnostic.message.find("Bool") < diagnostic.message.find("I64"));
}

#[test]
fn generic_static_bound_method_instantiates_method_generics_without_replacing_owner() {
    let mut lowerer = Lowerer::new_for_test();
    let generic_owner = def_id(68);
    let owner_param = GenericParamId {
        owner: generic_owner,
        index: 0,
    };
    let trait_id = def_id(69);
    let method_id = def_id(70);
    let trait_param = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let mut method = test_function(method_id, "make");
    method.generic_params = vec![GenericParamDecl::type_param(method_param, "MethodDisplay")];
    method.params = vec![HirParam {
        name: "value".to_string(),
        local_id: HirLocalId(0),
        ty: Type::Generic(method_param),
        mutable: false,
        is_ref: false,
    }];
    method.ret_type = Type::Tuple(vec![
        Type::Generic(trait_param),
        Type::Generic(method_param),
    ]);
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
            "TraitDisplay",
        )],
        associated_types: Vec::new(),
        methods: HashMap::from([("make".to_string(), method)]),
        signatures: HashMap::new(),
    });
    let bounds = HashMap::from([(
        owner_param,
        vec![TraitBound {
            trait_id,
            type_args: vec![Type::Generic(owner_param)],
        }],
    )]);

    let expr = lowerer.with_body_context(
        BodyLoweringContext::new(
            BodyOwner::Function(def_id(71)),
            Some(generic_owner),
            vec!["T".to_string()],
            bounds.into(),
            false,
        ),
        |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "make"])),
    );

    let Type::Function { params, ret, .. } = &expr.ty else {
        panic!("expected generic static method value function type")
    };
    assert!(matches!(params.as_slice(), [Type::TypeVar(_)]));
    assert!(matches!(
        ret.as_ref(),
        Type::Tuple(elements)
            if matches!(elements.as_slice(), [Type::Generic(param), Type::TypeVar(_)] if *param == owner_param)
    ));
    let (_, target) = static_method_value_parts(&expr);
    assert_eq!(target.method.method_id(), Some(method_id));
    assert_eq!(target.method.method_substitution.len(), 1);
    assert_eq!(target.method.method_substitution[0].param, method_param);
    assert!(matches!(
        target.method.method_substitution[0].ty,
        Type::TypeVar(_)
    ));
    assert_eq!(target.method.trait_args(), &[Type::Generic(owner_param)]);
}

#[test]
fn identical_generic_static_bounds_select_one_fresh_method_target() {
    let mut lowerer = Lowerer::new_for_test();
    let generic_owner = def_id(72);
    let owner_param = GenericParamId {
        owner: generic_owner,
        index: 0,
    };
    let trait_id = def_id(73);
    let method_id = def_id(74);
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let mut method = test_function(method_id, "make");
    method.generic_params = vec![GenericParamDecl::type_param(method_param, "MethodDisplay")];
    method.params = vec![HirParam {
        name: "value".to_string(),
        local_id: HirLocalId(0),
        ty: Type::Generic(method_param),
        mutable: false,
        is_ref: false,
    }];
    method.ret_type = Type::Generic(method_param);
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
            "TraitDisplay",
        )],
        associated_types: Vec::new(),
        methods: HashMap::from([("make".to_string(), method)]),
        signatures: HashMap::new(),
    });
    let bound = TraitBound {
        trait_id,
        type_args: vec![Type::Generic(owner_param)],
    };
    let bounds = HashMap::from([(owner_param, vec![bound.clone(), bound])]);

    let expr = lowerer.with_body_context(
        BodyLoweringContext::new(
            BodyOwner::Function(def_id(75)),
            Some(generic_owner),
            vec!["T".to_string()],
            bounds.into(),
            false,
        ),
        |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "make"])),
    );

    assert!(matches!(expr.ty, Type::Function { .. }));
    assert!(lowerer.diagnostics.errors().is_empty());
    let (_, target) = static_method_value_parts(&expr);
    assert_eq!(target.method.method_id(), Some(method_id));
    assert_eq!(target.method.trait_args(), &[Type::Generic(owner_param)]);
    assert_eq!(target.method.method_substitution.len(), 1);
    assert_eq!(target.method.method_substitution[0].param, method_param);
    assert!(matches!(
        target.method.method_substitution[0].ty,
        Type::TypeVar(_)
    ));
}
