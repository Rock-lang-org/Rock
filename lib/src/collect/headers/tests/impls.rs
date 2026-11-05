use super::*;

fn constructor_kind_syntax(name: &str, arity: usize) -> ast::TypeApplication {
    ast::TypeApplication {
        constructor: Box::new(named_type(name)),
        args: (0..arity)
            .map(|_| ast::ParseType::Hole(ast::TypeHole { span: Span::test() }))
            .collect(),
        span: Span::test(),
    }
}

#[test]
fn build_impl_resolves_trait_id_through_import_alias() {
    let mut context = CollectContext::new();
    let trait_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(10));
    let assoc_id = crate::ids::AssocTypeId(7);
    context
        .names
        .set_import_alias("Deref".to_string(), "std::deref::Deref".to_string());
    context.traits.insert(
        "std::deref::Deref".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Deref".to_string(),
            generic_params: Vec::new(),
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_id,
                name: "Target".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let imp = Impl {
        name: ParseType::Type(type_inner("Deref")),
        for_: Some(ParseType::Type(ParseTypeInner {
            name: "Vec".to_string(),
            generics: vec![generic_type("T")],
            span: Span::test(),
        })),
        associated_types: vec![AssociatedTypeDef {
            name: ident("Target"),
            kind: None,
            ty: ParseType::Reference {
                is_mut: false,
                pointee: Box::new(ParseType::Slice(Box::new(generic_type("T")))),
            },
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    let method_ids = impl_method_ids(&[]);
    let hir_impl = build_impl_with_id(&mut context, &imp, def_id(1), &method_ids);

    assert_eq!(hir_impl.trait_id, Some(trait_id));
    assert_eq!(hir_impl.associated_types[0].id, assoc_id);
}

#[test]
fn build_impl_resolves_trait_id_through_canonical_import_alias() {
    let mut context = CollectContext::new();
    let trait_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(10));
    context
        .names
        .record_canonical_alias("Clone".to_string(), trait_id, true);
    context.traits.insert(
        "std::clone::Clone".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Clone".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let imp = Impl {
        name: ParseType::Type(type_inner("Clone")),
        for_: Some(ParseType::Type(type_inner("String"))),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    let method_ids = impl_method_ids(&[]);
    let hir_impl = build_impl_with_id(&mut context, &imp, def_id(1), &method_ids);

    assert_eq!(hir_impl.trait_id, Some(trait_id));
}

#[test]
fn test_build_extern_registers_scope_entry() {
    let mut lowerer = CollectContext::new();
    let sig = function_sig(
        "puts",
        ParseType::Function(vec![
            ParseType::Pointer(Box::new(named_type("U8"))),
            named_type("I32"),
        ]),
        None,
        vec![],
    );

    let hir_extern = build_extern_with_id(&mut lowerer, &sig, sig.name.name.clone(), def_id(1));

    assert_eq!(hir_extern.name, "puts");
    assert_eq!(hir_extern.params, vec![Type::Pointer(Box::new(Type::U8))]);
    assert_eq!(hir_extern.ret, Type::I32);
    assert!(!hir_extern.variadic);
    let binding = lowerer.scope.lookup("puts").expect("extern is registered");
    assert_eq!(
        binding.ty,
        Type::function(vec![Type::Pointer(Box::new(Type::U8))], Type::I32)
    );
    assert!(!binding.mutable);
}

#[test]
fn test_build_impl_registers_methods_and_tracks_function_type_vars_by_id() {
    let mut lowerer = CollectContext::new();
    let imp = Impl {
        name: ParseType::Type(ParseTypeInner {
            name: "Show".to_string(),
            generics: vec![],
            span: Span::test(),
        }),
        for_: Some(named_type("Point")),
        associated_types: vec![],
        methods: HashMap::from([(
            ident("render"),
            function_decl(
                "render",
                &[],
                Some(SelfReceiverMode::Shared),
                LambdaArrowKind::Normal,
            ),
        )]),
        signatures: HashMap::new(),
        where_clauses: vec![],
    };

    let method_ids = impl_method_ids(&[("render", def_id(2))]);
    let hir_impl = build_impl_with_id(&mut lowerer, &imp, def_id(1), &method_ids);

    assert_eq!(hir_impl.type_name, "Point");
    assert_eq!(hir_impl.trait_name.as_deref(), Some("Show"));
    assert!(hir_impl.methods.contains_key("render"));
    assert!(lowerer
        .function_type_vars
        .contains_key(&hir_impl.methods["render"].id));
}

#[test]
fn build_impl_assigns_source_order_associated_type_def_ids_per_owner() {
    let mut context = CollectContext::new();
    let left = Impl {
        name: ParseType::Type(type_inner("Iterable")),
        for_: Some(named_type("List")),
        associated_types: vec![AssociatedTypeDef {
            name: ident("Item"),
            kind: None,
            ty: named_type("I32"),
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: vec![],
    };
    let right = Impl {
        name: ParseType::Type(type_inner("Iterable")),
        for_: Some(named_type("Tree")),
        associated_types: vec![
            AssociatedTypeDef {
                name: ident("Item"),
                kind: None,
                ty: named_type("I32"),
            },
            AssociatedTypeDef {
                name: ident("Error"),
                kind: None,
                ty: named_type("Bool"),
            },
        ],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: vec![],
    };

    let left_method_ids = impl_method_ids(&[]);
    let right_method_ids = impl_method_ids(&[]);
    let left = build_impl_with_id(&mut context, &left, def_id(1), &left_method_ids);
    let right = build_impl_with_id(&mut context, &right, def_id(2), &right_method_ids);

    assert_eq!(left.associated_types[0].id, AssocTypeId(0));
    assert_eq!(right.associated_types[0].id, AssocTypeId(0));
    assert_eq!(right.associated_types[1].id, AssocTypeId(1));
}

#[test]
fn build_impl_preserves_constructor_valued_associated_type_kind() {
    let mut context = CollectContext::new();
    let box_id = def_id(210);
    let trait_id = def_id(211);
    context.structs.insert(
        "Box".to_string(),
        HirStruct {
            id: box_id,
            name: "Box".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: box_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        },
    );
    context.traits.insert(
        "Families".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Families".to_string(),
            generic_params: Vec::new(),
            associated_types: vec![HirAssociatedTypeDecl {
                id: AssocTypeId(0),
                name: "Family".to_string(),
                kind: crate::type_services::kind::Kind::arrow(
                    crate::type_services::kind::Kind::Type,
                    crate::type_services::kind::Kind::Type,
                ),
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let imp = Impl {
        name: ParseType::Type(type_inner("Families")),
        for_: Some(ParseType::Type(ParseTypeInner {
            name: "Box".to_string(),
            generics: vec![named_type("I64")],
            span: Span::test(),
        })),
        associated_types: vec![AssociatedTypeDef {
            name: ident("Family"),
            kind: Some(constructor_kind_syntax("Family", 1)),
            ty: named_type("Box"),
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    let hir_impl = build_impl_with_id(&mut context, &imp, def_id(212), &HashMap::new());

    assert!(context.errors.is_empty(), "{:?}", context.errors);
    assert_eq!(hir_impl.associated_types[0].id, AssocTypeId(0));
    assert_eq!(
        hir_impl.associated_types[0].kind,
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        )
    );
    assert_eq!(
        hir_impl.associated_types[0].ty,
        Type::Constructor {
            id: box_id,
            flavor: crate::types::NominalTypeKind::Struct,
        }
    );
}

#[test]
fn build_impl_accepts_constructor_generic_in_receiver_pattern() {
    let impl_id = def_id(213);
    let receiver = ParseType::Application(ast::TypeApplication {
        constructor: Box::new(named_type("F")),
        args: vec![named_type("A")],
        span: Span::test(),
    });
    let imp = Impl {
        name: ParseType::Type(type_inner("Functor")),
        for_: Some(receiver),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: vec![ast::WhereClause {
            subject: ParseType::Application(constructor_kind_syntax("F", 1)),
            trait_bound: None,
        }],
    };
    let mut context = CollectContext::new();

    let hir_impl = build_impl_with_id(&mut context, &imp, impl_id, &HashMap::new());

    assert!(context.errors.is_empty(), "{:?}", context.errors);
    assert_eq!(
        hir_impl
            .type_generics
            .iter()
            .find(|param| param.name == "F")
            .unwrap()
            .kind,
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        )
    );
    assert!(matches!(
        hir_impl.receiver_pattern,
        HirImplReceiverPattern::Exact(Type::Apply { .. })
    ));
}

#[test]
fn build_impl_does_not_rekind_existing_ordinary_generic() {
    let trait_id = def_id(214);
    let impl_id = def_id(215);
    let mut context = CollectContext::new();
    context.traits.insert(
        "Holder".to_string(),
        HirTrait {
            id: trait_id,
            name: "Holder".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "F",
            )],
            target: None,
            predicates: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let imp = Impl {
        name: ParseType::Type(ParseTypeInner {
            name: "Holder".to_string(),
            generics: vec![generic_type("F")],
            span: Span::test(),
        }),
        for_: Some(named_type("I64")),
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: vec![WhereClause {
            subject: ParseType::Application(constructor_kind_syntax("F", 1)),
            trait_bound: None,
        }],
    };

    let hir_impl = build_impl_with_id(&mut context, &imp, impl_id, &HashMap::new());

    assert_eq!(
        hir_impl
            .type_generics
            .iter()
            .find(|param| param.name == "F")
            .expect("impl F generic")
            .kind,
        crate::type_services::kind::Kind::Type
    );
    assert!(context.errors.iter().any(|error| error.message.contains(
            "generic parameter 'F' was declared with kind Type, but a later binder requires Type -> Type"
        )));
}

#[test]
fn build_impl_records_constructor_trait_target_explicitly() {
    let trait_id = def_id(214);
    let option_id = def_id(215);
    let impl_id = def_id(216);
    let mut context = CollectContext::new();
    context.enums.insert(
        "Option".to_string(),
        HirEnum {
            id: option_id,
            name: "Option".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: option_id,
                    index: 0,
                },
                "T",
            )],
            variants: Vec::new(),
        },
    );
    context.traits.insert(
        "Functor".to_string(),
        HirTrait {
            id: trait_id,
            name: "Functor".to_string(),
            generic_params: Vec::new(),
            target: Some(GenericParamDecl::new(
                GenericParamId {
                    owner: trait_id,
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
    let imp = Impl {
        name: ParseType::Type(type_inner("Functor")),
        for_: Some(named_type("Option")),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    let hir_impl = build_impl_with_id(&mut context, &imp, impl_id, &HashMap::new());

    assert!(context.errors.is_empty(), "{:?}", context.errors);
    assert_eq!(
        hir_impl.receiver_pattern,
        HirImplReceiverPattern::Constructor(Type::Constructor {
            id: option_id,
            flavor: crate::types::NominalTypeKind::Enum,
        })
    );
}

#[test]
fn test_build_impl_tracks_receiver_and_trait_generics() {
    let mut lowerer = CollectContext::new();
    let vec_id = def_id(3);
    lowerer.structs.insert(
        "Vec".to_string(),
        HirStruct {
            id: vec_id,
            name: "Vec".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: vec_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        },
    );
    let imp = Impl {
        name: ParseType::Type(ParseTypeInner {
            name: "Show".to_string(),
            generics: vec![named_type("U8")],
            span: Span::test(),
        }),
        for_: Some(ParseType::Type(ParseTypeInner {
            name: "Vec".to_string(),
            generics: vec![generic_type("T")],
            span: Span::test(),
        })),
        associated_types: vec![],
        methods: HashMap::from([(
            ident("render"),
            function_decl(
                "render",
                &[],
                Some(SelfReceiverMode::Shared),
                LambdaArrowKind::Normal,
            ),
        )]),
        signatures: HashMap::new(),
        where_clauses: vec![],
    };

    let method_ids = impl_method_ids(&[("render", def_id(2))]);
    let hir_impl = build_impl_with_id(&mut lowerer, &imp, def_id(1), &method_ids);

    assert_eq!(hir_impl.type_name, "Vec");
    assert_eq!(
        hir_impl.receiver_pattern,
        HirImplReceiverPattern::Exact(Type::Struct {
            id: vec_id,
            args: vec![Type::Generic(crate::types::GenericParamId {
                owner: def_id(1),
                index: 0
            })],
        })
    );
    assert_eq!(hir_impl.trait_arg_types, vec![Type::U8]);
    assert!(lowerer
        .function_type_vars
        .contains_key(&hir_impl.methods["render"].id));
}

#[test]
fn build_impl_uses_marked_renamed_sized_trait_id() {
    let mut context = CollectContext::new();
    let sized_id = def_id(90);
    let vec_id = def_id(91);
    context.language_items.sized = Some(SizedLanguageItems { trait_id: sized_id });
    context.structs.insert(
        "Vec".to_string(),
        HirStruct {
            id: vec_id,
            name: "Vec".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: vec_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        },
    );
    context.traits.insert(
        "StaticLayout".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: sized_id,
            name: "StaticLayout".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let imp = Impl {
        name: ParseType::Type(type_inner("Vec")),
        for_: Some(ParseType::Type(ParseTypeInner {
            name: "Vec".to_string(),
            generics: vec![generic_type("T")],
            span: Span::test(),
        })),
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: vec![],
    };

    let hir_impl = build_impl_with_id(&mut context, &imp, def_id(92), &impl_method_ids(&[]));

    assert_eq!(
        hir_impl.bounds[&GenericParamId {
            owner: hir_impl.id,
            index: 0,
        }],
        vec![TraitBound {
            trait_id: sized_id,
            type_args: Vec::new(),
        }]
    );
}

#[test]
fn build_impl_does_not_inject_sized_bound_without_marked_bundle() {
    let mut context = CollectContext::new();
    let vec_id = def_id(93);
    context.structs.insert(
        "Vec".to_string(),
        HirStruct {
            id: vec_id,
            name: "Vec".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: vec_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        },
    );
    let imp = Impl {
        name: ParseType::Type(type_inner("Vec")),
        for_: Some(ParseType::Type(ParseTypeInner {
            name: "Vec".to_string(),
            generics: vec![generic_type("T")],
            span: Span::test(),
        })),
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: vec![],
    };

    let hir_impl = build_impl_with_id(&mut context, &imp, def_id(94), &impl_method_ids(&[]));

    assert!(hir_impl.bounds.is_empty());
}
