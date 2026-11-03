use super::*;

#[test]
fn collect_structural_inherent_method_has_declared_return_and_receiver() {
    let mut context = CollectContext::new();
    let impl_id = def_id(100);
    let method_id = def_id(101);
    let implementation = Impl {
        name: ParseType::Reference {
            is_mut: false,
            pointee: Box::new(ParseType::Slice(Box::new(named_type("T")))),
        },
        for_: None,
        associated_types: vec![],
        methods: HashMap::from([(
            ident("pointer"),
            function_decl(
                "pointer",
                &[],
                Some(SelfReceiverMode::Shared),
                LambdaArrowKind::Normal,
            ),
        )]),
        signatures: HashMap::from([(
            ident("pointer"),
            function_sig(
                "pointer",
                ParseType::Pointer(Box::new(named_type("T"))),
                Some(SelfReceiverMode::Shared),
                vec![],
            ),
        )]),
        where_clauses: vec![],
    };
    let collected = build_impl_with_id(
        &mut context,
        &implementation,
        impl_id,
        &impl_method_ids(&[("pointer", method_id)]),
    );
    assert!(context.errors.is_empty(), "{:?}", context.errors);
    let element = Type::Generic(GenericParamId {
        owner: impl_id,
        index: 0,
    });
    let target = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Slice(Box::new(element.clone()))),
    };
    assert_eq!(collected.type_generics.len(), 1);
    assert_eq!(
        collected.receiver_pattern,
        crate::hir::HirImplReceiverPattern::Exact(target.clone())
    );
    let method = &collected.methods["pointer"];
    assert_eq!(method.ret_type, Type::Pointer(Box::new(element)));
    assert_eq!(
        method.params[0].ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(target)
        }
    );
    assert_eq!(method.id, method_id);
}

#[test]
fn collect_type_var_ids_visits_projection_base_and_trait_args() {
    let base_var = TypeVarId(10);
    let arg_var = TypeVarId(11);
    let ty = Type::Projection {
        ty: Box::new(Type::TypeVar(base_var)),
        trait_id: DefId::new(CrateId(0), LocalDefId(20)),
        assoc_type: AssociatedTypeKey {
            owner: DefId::new(CrateId(0), LocalDefId(20)),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![Type::TypeVar(arg_var)],
    };
    let mut type_vars = HashSet::new();

    collect_type_var_ids(&ty, &mut type_vars);

    assert!(type_vars.contains(&base_var));
    assert!(type_vars.contains(&arg_var));
}

#[test]
fn test_build_function_sig_handles_curried_types_and_where_bounds() {
    let mut lowerer = CollectContext::new();
    let show_id = DefId::new(CrateId(0), LocalDefId(11));
    let eq_id = DefId::new(CrateId(0), LocalDefId(12));
    lowerer.traits.insert(
        "Show".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: show_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    lowerer.traits.insert(
        "Eq".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: eq_id,
            name: "Eq".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let sig = function_sig(
        "compose",
        ParseType::Function(vec![
            generic_type("T"),
            named_type("I32"),
            named_type("Bool"),
            named_type("U8"),
        ]),
        Some(SelfReceiverMode::Shared),
        vec![
            WhereClause {
                subject: generic_type("T"),
                trait_bound: Some(named_type("Show")),
            },
            WhereClause {
                subject: generic_type("T"),
                trait_bound: Some(named_type("Eq")),
            },
        ],
    );

    let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(20));

    assert_eq!(hir_sig.name, "compose");
    assert_eq!(
        hir_sig.generic_params,
        vec![
            GenericParamDecl::type_param(
                GenericParamId {
                    owner: def_id(20),
                    index: 0,
                },
                "T",
            ),
            GenericParamDecl::type_param(
                GenericParamId {
                    owner: def_id(20),
                    index: 1,
                },
                "Self",
            ),
        ]
    );
    let t_id = hir_sig.generic_params[0].id;
    let self_id = hir_sig.generic_params[1].id;
    assert_eq!(
        hir_sig.params,
        vec![
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Generic(self_id)),
            },
            Type::Generic(t_id),
            Type::I32,
            Type::Bool
        ]
    );
    assert_eq!(hir_sig.ret, Type::U8);
    assert_eq!(
        hir_sig.self_receiver,
        Some(crate::types::ReceiverMode::Shared)
    );
    assert_eq!(
        hir_sig.generic_bounds.get(&t_id),
        Some(&vec![
            TraitBound {
                trait_id: show_id,
                type_args: Vec::new(),
            },
            TraitBound {
                trait_id: eq_id,
                type_args: Vec::new(),
            },
        ])
    );
}

#[test]
fn test_build_function_sig_preserves_returned_function_types() {
    let mut lowerer = CollectContext::new();
    let sig = function_sig(
        "make",
        ParseType::Function(vec![
            named_type("I64"),
            ParseType::Function(vec![
                ParseType::Unit(crate::lexer::Span::test()),
                named_type("I64"),
            ]),
        ]),
        None,
        Vec::new(),
    );

    let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(21));

    assert_eq!(hir_sig.params, vec![Type::I64]);
    assert_eq!(hir_sig.ret, Type::function(Vec::new(), Type::I64));
}

#[test]
fn test_build_function_sig_omits_unknown_where_clause_trait_bounds() {
    let mut lowerer = CollectContext::new();
    let sig = function_sig(
        "identity",
        ParseType::Function(vec![generic_type("T"), generic_type("T")]),
        None,
        vec![WhereClause {
            subject: generic_type("T"),
            trait_bound: Some(named_type("Missing")),
        }],
    );

    let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(22));

    assert!(hir_sig.generic_bounds.is_empty());
    assert!(lowerer
        .errors
        .iter()
        .any(|error| error.message == "unknown trait 'Missing' in where clause"));
    assert!(lowerer.errors.iter().any(|error| {
        error.message == "unknown trait 'Missing' in where clause"
            && error.span() == Some(Span::test())
    }));
}

#[test]
fn test_build_function_sig_preserves_self_receiver_for_signature_backed_method() {
    let mut lowerer = CollectContext::new();
    let sig = function_sig(
        "render",
        ParseType::Function(vec![named_type("I64"), named_type("Bool")]),
        Some(SelfReceiverMode::Mut),
        vec![],
    );

    let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(23));

    assert_eq!(hir_sig.self_receiver, Some(crate::types::ReceiverMode::Mut));
    assert!(matches!(
        hir_sig.params[0],
        Type::Reference { mutable: true, .. }
    ));
    assert_eq!(
        hir_sig.generic_params,
        vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(23),
                index: 0,
            },
            "Self",
        )]
    );
    assert_eq!(hir_sig.params[1], Type::I64);
    assert_eq!(hir_sig.ret, Type::Bool);
}

#[test]
fn test_build_function_sig_preserves_explicit_generic_names() {
    let mut lowerer = CollectContext::new();
    let sig = function_sig(
        "identity",
        ParseType::Function(vec![named_type("A"), named_type("A")]),
        None,
        vec![],
    );

    let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(24));

    assert_eq!(
        hir_sig.generic_params,
        vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(24),
                index: 0,
            },
            "A",
        )]
    );
}

#[test]
fn test_build_function_header_tracks_function_type_vars() {
    let mut lowerer = CollectContext::new();
    let decl = function_decl("map", &["f", "x"], None, LambdaArrowKind::Curried);

    let function = build_function_header_with_id(&mut lowerer, &decl, def_id(40));

    assert_eq!(function.name, "map");
    assert!(function.is_curried);
    assert_eq!(function.params.len(), 1);
    let mut expected = HashSet::new();
    collect_type_var_ids(&function.params[0].ty, &mut expected);
    collect_type_var_ids(&function.ret_type, &mut expected);
    assert_eq!(
        lowerer.function_type_vars.get(&function.id),
        Some(&expected)
    );
}

#[test]
fn test_build_function_header_uses_provided_function_id() {
    let mut lowerer = CollectContext::new();
    let decl = function_decl("map", &["f", "x"], None, LambdaArrowKind::Curried);
    let function_id = def_id(44);

    let function = build_function_header_with_id(&mut lowerer, &decl, function_id);

    assert_eq!(function.id, function_id);
}

#[test]
fn test_build_function_header_with_sig_uses_provided_function_id_for_generic_owner() {
    let mut lowerer = CollectContext::new();
    let decl = function_decl("id", &["value"], None, LambdaArrowKind::Normal);
    let signature_id = def_id(11);
    let function_id = def_id(45);
    let sig_generic = GenericParamId {
        owner: signature_id,
        index: 0,
    };
    let sig = HirFunctionSig {
        id: signature_id,
        name: "id".to_string(),
        generic_params: vec![GenericParamDecl::type_param(sig_generic, "T")],
        params: vec![Type::Generic(sig_generic)],
        ret: Type::Generic(sig_generic),
        generic_bounds: HashMap::new().into(),
        self_receiver: None,
        is_unsafe: false,
    };

    let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, function_id);

    assert_eq!(function.id, function_id);
    assert_eq!(
        function.generic_params,
        vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: function_id,
                index: 0,
            },
            "T",
        )]
    );
    assert_eq!(
        function.params[0].ty,
        Type::Generic(GenericParamId {
            owner: function_id,
            index: 0,
        })
    );
    assert_eq!(
        function.ret_type,
        Type::Generic(GenericParamId {
            owner: function_id,
            index: 0,
        })
    );
}

#[test]
fn test_build_function_header_with_sig_uses_signature_types_and_tracks_type_vars() {
    let mut lowerer = CollectContext::new();
    let decl = function_decl("id", &["value"], None, LambdaArrowKind::Normal);
    let sig = HirFunctionSig {
        id: DefId::new(CrateId(0), LocalDefId(1)),
        name: "id".to_string(),
        generic_params: Vec::new(),
        params: vec![Type::TypeVar(TypeVarId(41))],
        ret: Type::TypeVar(TypeVarId(99)),
        generic_bounds: HashMap::new().into(),
        self_receiver: None,
        is_unsafe: false,
    };

    let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, def_id(41));

    assert_eq!(function.params[0].ty, Type::TypeVar(TypeVarId(41)));
    assert_eq!(function.ret_type, Type::TypeVar(TypeVarId(99)));
    assert_eq!(
        lowerer.function_type_vars.get(&function.id),
        Some(&HashSet::from([TypeVarId(41), TypeVarId(99)]))
    );
}

#[test]
fn test_build_function_header_with_sig_keeps_self_receiver_for_signature_backed_method() {
    let mut lowerer = CollectContext::new();
    let decl = function_decl(
        "render",
        &["value"],
        Some(SelfReceiverMode::Shared),
        LambdaArrowKind::Normal,
    );
    let sig = HirFunctionSig {
        id: DefId::new(CrateId(0), LocalDefId(2)),
        name: "render".to_string(),
        generic_params: Vec::new(),
        params: vec![Type::TypeVar(TypeVarId(7)), Type::I64],
        ret: Type::Bool,
        generic_bounds: HashMap::new().into(),
        self_receiver: Some(crate::types::ReceiverMode::Shared),
        is_unsafe: false,
    };

    let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, def_id(42));

    assert!(function.is_method);
    assert_eq!(
        function.self_receiver,
        Some(crate::types::ReceiverMode::Shared)
    );
    assert_eq!(function.params.len(), 2);
    assert_eq!(function.params[0].ty, Type::TypeVar(TypeVarId(7)));
    assert_eq!(function.params[1].name, "value");
    assert_eq!(function.params[1].ty, Type::I64);
    assert_eq!(function.ret_type, Type::Bool);
    assert_eq!(
        lowerer.function_type_vars.get(&function.id),
        Some(&HashSet::from([TypeVarId(7)]))
    );
}

#[test]
fn test_build_function_header_with_sig_remaps_hidden_mut_self_receiver_generic() {
    let mut lowerer = CollectContext::new();
    let signature_id = def_id(46);
    let function_id = def_id(47);
    let signature_self = GenericParamId {
        owner: signature_id,
        index: 0,
    };
    let decl = function_decl(
        "set",
        &["value"],
        Some(SelfReceiverMode::Mut),
        LambdaArrowKind::Normal,
    );
    let sig = HirFunctionSig {
        id: signature_id,
        name: "set".to_string(),
        generic_params: vec![GenericParamDecl::type_param(signature_self, "Self")],
        params: vec![
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::Generic(signature_self)),
            },
            Type::I64,
        ],
        ret: Type::Bool,
        generic_bounds: HashMap::new().into(),
        self_receiver: Some(crate::types::ReceiverMode::Mut),
        is_unsafe: false,
    };

    let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, function_id);

    let expected_self = GenericParamId {
        owner: function_id,
        index: 0,
    };
    assert_eq!(
        function.generic_params,
        vec![GenericParamDecl::type_param(expected_self, "Self")]
    );
    assert_eq!(function.params[0].local_id, crate::ids::HirLocalId(0));
    assert!(function.params[0].mutable);
    assert_eq!(
        function.params[0].ty,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::Generic(expected_self)),
        }
    );
    assert_eq!(function.params[1].ty, Type::I64);
}

#[test]
fn test_build_function_header_with_sig_keeps_public_generic_name_paired_after_hidden_self() {
    let mut lowerer = CollectContext::new();
    let signature_id = def_id(48);
    let function_id = def_id(49);
    let signature_self = GenericParamId {
        owner: signature_id,
        index: 0,
    };
    let signature_t = GenericParamId {
        owner: signature_id,
        index: 1,
    };
    let decl = function_decl(
        "set",
        &["value"],
        Some(SelfReceiverMode::Mut),
        LambdaArrowKind::Normal,
    );
    let sig = HirFunctionSig {
        id: signature_id,
        name: "set".to_string(),
        generic_params: vec![
            GenericParamDecl::type_param(signature_self, "Self"),
            GenericParamDecl::type_param(signature_t, "T"),
        ],
        params: vec![
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::Generic(signature_self)),
            },
            Type::Generic(signature_t),
        ],
        ret: Type::Generic(signature_t),
        generic_bounds: HashMap::new().into(),
        self_receiver: Some(crate::types::ReceiverMode::Mut),
        is_unsafe: false,
    };

    let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, function_id);

    let expected_t = GenericParamId {
        owner: function_id,
        index: 0,
    };
    let expected_self = GenericParamId {
        owner: function_id,
        index: 1,
    };
    assert_eq!(
        function.generic_params,
        vec![
            GenericParamDecl::type_param(expected_t, "T"),
            GenericParamDecl::type_param(expected_self, "Self"),
        ]
    );
    assert_eq!(
        function.params[0].ty,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::Generic(expected_self)),
        }
    );
    assert_eq!(function.params[1].ty, Type::Generic(expected_t));
    assert_eq!(function.ret_type, Type::Generic(expected_t));
}

#[test]
fn test_build_function_header_with_sig_keeps_curried_signature_shape() {
    let mut lowerer = CollectContext::new();
    let decl = function_decl("compose", &["f", "x"], None, LambdaArrowKind::Curried);
    let sig = HirFunctionSig {
        id: DefId::new(CrateId(0), LocalDefId(3)),
        name: "compose".to_string(),
        generic_params: Vec::new(),
        params: vec![Type::I32, Type::Bool],
        ret: Type::U8,
        generic_bounds: HashMap::new().into(),
        self_receiver: None,
        is_unsafe: false,
    };

    let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, def_id(43));

    assert!(function.is_curried);
    assert_eq!(function.params.len(), 1);
    assert_eq!(function.params[0].name, "f");
    assert_eq!(function.params[0].ty, Type::I32);
    assert_eq!(
        function.ret_type,
        Type::function(vec![Type::Bool], Type::U8)
    );
}
