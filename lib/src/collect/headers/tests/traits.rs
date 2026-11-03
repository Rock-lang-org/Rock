use super::*;

fn trait_member_ids(
    methods: &[(&str, DefId)],
    signatures: &[(&str, DefId)],
) -> CollectedTraitMemberIds {
    CollectedTraitMemberIds {
        methods: methods
            .iter()
            .map(|(name, id)| ((*name).to_string(), *id))
            .collect(),
        signatures: signatures
            .iter()
            .map(|(name, id)| ((*name).to_string(), *id))
            .collect(),
    }
}

#[test]
fn test_build_trait_uses_temporary_trait_context_and_restores_it() {
    let mut lowerer = CollectContext::new();
    lowerer.current_trait = Some("Outer".to_string());
    lowerer.current_trait_generics = vec!["X".to_string()];

    let decl = TraitDecl {
        where_clauses: Vec::new(),
        name: ParseTypeInner {
            name: "Iterable".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![generic_param("T")],
        for_: None,
        associated_types: vec![AssociatedTypeDecl {
            name: ident("Item"),
            kind: None,
        }],
        methods: HashMap::from([(
            ident("next"),
            function_decl(
                "next",
                &[],
                Some(SelfReceiverMode::Shared),
                LambdaArrowKind::Normal,
            ),
        )]),
        signatures: HashMap::from([(
            ident("collect"),
            function_sig(
                "collect",
                ParseType::Associated {
                    base: type_inner("Self"),
                    member: ident("Item"),
                },
                None,
                vec![],
            ),
        )]),
        exported: false,
        language_items: Default::default(),
    };

    let member_ids = trait_member_ids(&[("next", def_id(2))], &[("collect", def_id(3))]);
    let hir_trait = build_trait_with_id(&mut lowerer, &decl, def_id(1), &member_ids);

    assert_eq!(hir_trait.name, "Iterable");
    assert!(hir_trait.methods.contains_key("next"));
    assert!(lowerer
        .function_type_vars
        .contains_key(&hir_trait.methods["next"].id));
    assert_eq!(lowerer.current_trait.as_deref(), Some("Outer"));
    assert_eq!(lowerer.current_trait_generics, vec!["X".to_string()]);
    assert_eq!(
        hir_trait.signatures["collect"].ret,
        Type::Projection {
            ty: Box::new(Type::Generic(crate::types::GenericParamId {
                owner: def_id(1),
                index: 1
            })),
            trait_id: def_id(1),
            assoc_type: crate::types::AssociatedTypeKey {
                owner: def_id(1),
                assoc_type_id: crate::ids::AssocTypeId(0),
            },
            trait_args: vec![Type::Generic(crate::types::GenericParamId {
                owner: def_id(1),
                index: 0
            })],
        }
    );
}

#[test]
fn build_trait_lowers_constructor_target_and_typed_supertrait_predicate() {
    let functor_id = def_id(70);
    let applicative_id = def_id(71);
    let mut context = CollectContext::new();
    context.traits.insert(
        "Functor".to_string(),
        HirTrait {
            id: functor_id,
            name: "Functor".to_string(),
            generic_params: Vec::new(),
            target: Some(GenericParamDecl::new(
                GenericParamId {
                    owner: functor_id,
                    index: 0,
                },
                "F",
                crate::type_services::kind::Kind::arrow(
                    crate::type_services::kind::Kind::Type,
                    crate::type_services::kind::Kind::Type,
                ),
            )),
            predicates: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let target = ast::GenericParamDecl {
        default: None,
        name: ident("F"),
        kind: Some(ast::TypeApplication {
            constructor: Box::new(named_type("F")),
            args: vec![ast::ParseType::Hole(ast::TypeHole { span: Span::test() })],
            span: Span::test(),
        }),
        span: Span::test(),
    };
    let declaration = TraitDecl {
        name: type_inner("Applicative"),
        generic_params: Vec::new(),
        for_: Some(target),
        where_clauses: vec![ast::WhereClause {
            subject: named_type("F"),
            trait_bound: Some(named_type("Functor")),
        }],
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
        exported: false,
        language_items: Default::default(),
    };

    let trait_def = build_trait_with_id(
        &mut context,
        &declaration,
        applicative_id,
        &trait_member_ids(&[], &[]),
    );

    let target = trait_def.target.expect("constructor target");
    assert_eq!(
        target.kind,
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        )
    );
    assert_eq!(
        trait_def.predicates,
        vec![crate::types::Predicate::Trait {
            subject: Type::Generic(target.id),
            trait_id: functor_id,
            args: Vec::new(),
        }]
    );
}

#[test]
fn typed_predicate_preserves_arbitrary_subjects() {
    let owner = def_id(72);
    let trait_id = def_id(73);
    let mut context = CollectContext::new();
    context.traits.insert(
        "Show".to_string(),
        HirTrait {
            id: trait_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            target: None,
            predicates: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let previous = context.push_generic_context(owner, vec!["T".to_string()]);
    let clauses = vec![ast::WhereClause {
        subject: ast::ParseType::Tuple(vec![named_type("T"), named_type("I64")]),
        trait_bound: Some(named_type("Show")),
    }];

    let bounds = lower_where_clauses(&mut context, &clauses);
    context.pop_generic_context(previous.0, previous.1);

    assert!(bounds.is_empty());
    assert_eq!(
        bounds.predicates,
        vec![crate::types::Predicate::Trait {
            subject: Type::Tuple(vec![
                Type::Generic(GenericParamId { owner, index: 0 }),
                Type::I64,
            ]),
            trait_id,
            args: Vec::new(),
        }]
    );
}

#[test]
fn build_trait_method_reuses_trait_generic_without_redeclaring_it() {
    let mut context = CollectContext::new();
    let decl = TraitDecl {
        where_clauses: Vec::new(),
        name: ParseTypeInner {
            name: "Mapper".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![generic_param("T")],
        for_: None,
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            ident("map"),
            function_sig(
                "map",
                ParseType::Function(vec![generic_type("T"), generic_type("T")]),
                None,
                vec![],
            ),
        )]),
        exported: false,
        language_items: Default::default(),
    };

    let member_ids = trait_member_ids(&[], &[("map", def_id(2))]);
    let hir_trait = build_trait_with_id(&mut context, &decl, def_id(1), &member_ids);
    let expected_generic = crate::types::GenericParamId {
        owner: hir_trait.id,
        index: 0,
    };
    let sig = &hir_trait.signatures["map"];

    assert!(sig.generic_params.is_empty());
    assert_eq!(sig.params[0], Type::Generic(expected_generic));
    assert_eq!(sig.ret, Type::Generic(expected_generic));
}

#[test]
fn trait_member_owned_generics_do_not_collide_across_members() {
    let mut context = CollectContext::new();
    let trait_id = def_id(1);
    let first_id = def_id(2);
    let second_id = def_id(3);
    let signature = |name| {
        function_sig(
            name,
            ParseType::Function(vec![generic_type("T"), generic_type("U")]),
            None,
            vec![],
        )
    };
    let decl = TraitDecl {
        where_clauses: Vec::new(),
        name: ParseTypeInner {
            name: "Mapper".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![generic_param("T")],
        for_: None,
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([
            (ident("first"), signature("first")),
            (ident("second"), signature("second")),
        ]),
        exported: false,
        language_items: Default::default(),
    };
    let member_ids = trait_member_ids(&[], &[("first", first_id), ("second", second_id)]);

    let hir_trait = build_trait_with_id(&mut context, &decl, trait_id, &member_ids);
    let inherited = Type::Generic(GenericParamId {
        owner: trait_id,
        index: 0,
    });
    for (name, member_id) in [("first", first_id), ("second", second_id)] {
        let signature = &hir_trait.signatures[name];
        let member_generic = GenericParamId {
            owner: member_id,
            index: 0,
        };
        assert_eq!(
            signature.generic_params,
            vec![GenericParamDecl::type_param(member_generic, "U")]
        );
        assert_eq!(signature.params, vec![inherited.clone()]);
        assert_eq!(signature.ret, Type::Generic(member_generic));
    }
}

#[test]
fn trait_member_bound_only_generic_uses_member_owner() {
    let mut context = CollectContext::new();
    let trait_id = def_id(1);
    let member_id = def_id(2);
    let marker_id = def_id(3);
    context.traits.insert(
        "Marker".to_string(),
        HirTrait {
            id: marker_id,
            name: "Marker".to_string(),
            generic_params: vec![],
            target: None,
            predicates: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let decl = TraitDecl {
        where_clauses: vec![],
        name: type_inner("Bounded"),
        generic_params: vec![generic_param("T")],
        for_: None,
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            ident("check"),
            function_sig(
                "check",
                ParseType::Function(vec![generic_type("T"), generic_type("T")]),
                None,
                vec![WhereClause {
                    subject: generic_type("U"),
                    trait_bound: Some(named_type("Marker")),
                }],
            ),
        )]),
        exported: false,
        language_items: Default::default(),
    };
    let member_ids = trait_member_ids(&[], &[("check", member_id)]);

    let hir_trait = build_trait_with_id(&mut context, &decl, trait_id, &member_ids);
    let signature = &hir_trait.signatures["check"];
    let member_generic = GenericParamId {
        owner: member_id,
        index: 0,
    };
    assert_eq!(
        signature.generic_params,
        vec![GenericParamDecl::type_param(member_generic, "U")]
    );
    assert_eq!(
        signature.generic_bounds[&member_generic][0].trait_id,
        marker_id
    );
}

#[test]
fn constructor_kind_discovery_requires_explicit_holes() {
    let application = |arg| {
        ParseType::Application(ast::TypeApplication {
            constructor: Box::new(generic_type("F")),
            args: vec![arg],
            span: Span::test(),
        })
    };
    let mut kinds = HashMap::new();
    collect_constructor_generic_kinds(&application(generic_type("A")), &mut kinds, &|_| false);
    assert!(!kinds.contains_key("F"));

    collect_constructor_generic_kinds(
        &application(ParseType::Hole(ast::TypeHole { span: Span::test() })),
        &mut kinds,
        &|_| false,
    );
    assert_eq!(
        kinds["F"],
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        )
    );
}

#[test]
fn constructor_binder_does_not_rekind_existing_ordinary_generic() {
    let mut context = CollectContext::new();
    let marker_id = def_id(3);
    context.traits.insert(
        "Marker".to_string(),
        HirTrait {
            id: marker_id,
            name: "Marker".to_string(),
            generic_params: vec![],
            target: None,
            predicates: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let constructor_subject = ParseType::Application(ast::TypeApplication {
        constructor: Box::new(generic_type("F")),
        args: vec![ParseType::Hole(ast::TypeHole { span: Span::test() })],
        span: Span::test(),
    });
    let decl = TraitDecl {
        where_clauses: vec![],
        name: type_inner("Ordinary"),
        generic_params: vec![generic_param("F")],
        for_: None,
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            ident("check"),
            function_sig(
                "check",
                ParseType::Function(vec![generic_type("T"), generic_type("T")]),
                None,
                vec![WhereClause {
                    subject: constructor_subject,
                    trait_bound: Some(named_type("Marker")),
                }],
            ),
        )]),
        exported: false,
        language_items: Default::default(),
    };

    let hir_trait = build_trait_with_id(
        &mut context,
        &decl,
        def_id(1),
        &trait_member_ids(&[], &[("check", def_id(2))]),
    );

    assert_eq!(
        hir_trait.generic_params[0].kind,
        crate::type_services::kind::Kind::Type
    );
    assert!(context.errors.iter().any(|error| error.message.contains(
            "generic parameter 'F' was declared with kind Type, but its constructor binder requires Type -> Type"
        )));
}

#[test]
fn build_trait_assigns_source_order_associated_type_ids_per_owner() {
    let mut context = CollectContext::new();
    let left = TraitDecl {
        where_clauses: Vec::new(),
        name: type_inner("Left"),
        generic_params: vec![],
        for_: None,
        associated_types: vec![AssociatedTypeDecl {
            name: ident("Item"),
            kind: None,
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        exported: false,
        language_items: Default::default(),
    };
    let right = TraitDecl {
        where_clauses: Vec::new(),
        name: type_inner("Right"),
        generic_params: vec![],
        for_: None,
        associated_types: vec![
            AssociatedTypeDecl {
                name: ident("Item"),
                kind: None,
            },
            AssociatedTypeDecl {
                name: ident("Error"),
                kind: None,
            },
        ],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        exported: false,
        language_items: Default::default(),
    };

    let left_member_ids = trait_member_ids(&[], &[]);
    let right_member_ids = trait_member_ids(&[], &[]);
    let left = build_trait_with_id(&mut context, &left, def_id(1), &left_member_ids);
    let right = build_trait_with_id(&mut context, &right, def_id(2), &right_member_ids);

    assert_eq!(left.associated_types[0].id, AssocTypeId(0));
    assert_eq!(right.associated_types[0].id, AssocTypeId(0));
    assert_eq!(right.associated_types[1].id, AssocTypeId(1));
}
