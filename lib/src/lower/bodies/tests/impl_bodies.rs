use super::*;

fn parse_type(name: &str, generics: Vec<ast::ParseType>) -> ast::ParseType {
    ast::ParseType::Type(ParseTypeInner {
        name: name.to_string(),
        generics,
        span: Span::test(),
    })
}

fn int_expr(value: u64) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Literal(Literal {
            kind: LiteralKind::Number(value),
            span: Span::test(),
        }),
        secondaries: None,
        type_annotation: None,
    }))
}

fn string_instance_expr() -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Instance(crate::ast::Instance {
            name: TypePath {
                path: vec![IdentOrType::Ident(ident("String"))],
            },
            fields: HashMap::from([(ident("ptr"), int_expr(1))]),
        }),
        secondaries: None,
        type_annotation: None,
    }))
}

fn self_field_expr(field: &str) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Ident(IdentifierPath {
            path: vec![IdentOrType::Ident(ident("self"))],
        }),
        secondaries: Some(vec![SecondaryExpr::Dot(IdentOrNumber::Ident(ident(field)))]),
        type_annotation: None,
    }))
}

fn private_field_struct(id: DefId, name: &str) -> HirStruct {
    HirStruct {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        fields: vec![HirField {
            id: FieldId(0),
            name: "ptr".to_string(),
            ty: Type::I64,
            public: false,
        }],
    }
}

fn unit_enum(id: DefId, name: &str, generic_params: Vec<GenericParamDecl>) -> HirEnum {
    HirEnum {
        id,
        name: name.to_string(),
        generic_params,
        variants: vec![crate::hir::HirVariant {
            id: VariantId(0),
            name: "Some".to_string(),
            fields: crate::hir::HirVariantFields::Unit,
        }],
    }
}

fn empty_impl_function(name: &str, id: DefId, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn shared_method_placeholder(name: &str, id: DefId, self_ty: Type, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: self_ty,
            mutable: false,
            is_ref: false,
        }],
        ret_type,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(crate::types::ReceiverMode::Shared),
        is_unsafe: false,
    }
}

#[test]
fn lower_impl_bodies_resolves_current_struct_impl_by_prepared_id() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(40);
    let impl_id = def_id(41);
    let function_id = def_id(42);
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "std::string_type::String"));
    lowerer
        .resolver
        .scoped_module_aliases
        .entry("std::string_type".to_string())
        .or_default()
        .insert("String".to_string(), struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "std::string_type::String".to_string());
    lowerer
        .modules
        .replace_qualified_module_prefix(Some("std::string_type"));
    let function = empty_impl_function(
        "new",
        function_id,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("String".to_string()),
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
            methods: HashMap::from([("new".to_string(), function.clone())]),
        })
        .unwrap();
    let ast_impl = crate::ast::Impl {
        name: ast::ParseType::Type(crate::ast::ParseTypeInner {
            name: "String".to_string(),
            generics: Vec::new(),
            span: Span::test(),
        }),
        for_: None,
        associated_types: Vec::new(),
        methods: HashMap::from([(
            ident("new"),
            FunctionDecl {
                name: ident("new"),
                lambda: LambdaDecl {
                    parameters: Vec::new(),
                    body: Block {
                        statements: vec![Statement::Expression(string_instance_expr())],
                    },
                    arrow_kind: LambdaArrowKind::Normal,
                    span: crate::lexer::Span::test(),
                },
                self_receiver: None,
                is_unsafe: false,
                exported: false,
            },
        )]),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    lowerer.lower_impl_bodies(&ast_impl, impl_id);

    assert!(
        lowerer
            .errors()
            .iter()
            .all(|error| !error.message.contains("private fields")),
        "same-struct construction should not report private fields: {:?}",
        lowerer.errors()
    );
}

#[test]
fn lower_impl_bodies_inherent_impl_does_not_match_trait_impl_slot() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(70);
    let inherent_impl_id = def_id(71);
    let trait_id = def_id(72);
    let trait_impl_id = def_id(73);
    let inherent_function_id = def_id(74);
    let trait_function_id = def_id(75);
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "Thing"));
    lowerer
        .resolver
        .item_paths
        .insert("Thing".to_string(), struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "Thing".to_string());
    let inherent_function = empty_impl_function("value", inherent_function_id, Type::I64);
    let trait_function = empty_impl_function("value", trait_function_id, Type::Bool);
    lowerer
        .items
        .insert_impl(HirImpl {
            id: inherent_impl_id,
            owner: HirImplOwner::Named("Thing".to_string()),
            type_name: "Thing".to_string(),
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
            methods: HashMap::from([("value".to_string(), inherent_function)]),
        })
        .unwrap();
    lowerer
        .items
        .insert_impl(HirImpl {
            id: trait_impl_id,
            owner: HirImplOwner::Named("Thing".to_string()),
            type_name: "Thing".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: Some("Readable".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), trait_function)]),
        })
        .unwrap();
    let ast_impl = crate::ast::Impl {
        name: ast::ParseType::Type(ParseTypeInner {
            name: "Thing".to_string(),
            generics: Vec::new(),
            span: Span::test(),
        }),
        for_: None,
        associated_types: Vec::new(),
        methods: HashMap::from([(
            ident("value"),
            FunctionDecl {
                name: ident("value"),
                lambda: LambdaDecl {
                    parameters: Vec::new(),
                    body: Block {
                        statements: vec![Statement::Expression(int_expr(1))],
                    },
                    arrow_kind: LambdaArrowKind::Normal,
                    span: crate::lexer::Span::test(),
                },
                self_receiver: None,
                is_unsafe: false,
                exported: false,
            },
        )]),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    lowerer.lower_impl_bodies(&ast_impl, inherent_impl_id);

    assert_eq!(
        lowerer.items.impl_def(inherent_impl_id).unwrap().methods["value"]
            .body
            .ty,
        Type::I64
    );
    assert_eq!(
        lowerer.items.impl_def(trait_impl_id).unwrap().methods["value"]
            .body
            .ty,
        Type::Unit
    );
}

#[test]
fn lower_impl_bodies_context_owner_matches_trait_args() {
    let mut lowerer = Lowerer::new_for_test();
    let box_id = def_id(80);
    let trait_id = def_id(81);
    let matching_impl_id = def_id(82);
    let other_impl_id = def_id(83);
    let function_id = def_id(84);
    let matching_generic = GenericParamId {
        owner: matching_impl_id,
        index: 0,
    };
    let other_generic = GenericParamId {
        owner: other_impl_id,
        index: 0,
    };
    lowerer.items.insert_structure(HirStruct {
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
    });
    lowerer
        .resolver
        .item_paths
        .insert("Box".to_string(), box_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(box_id, "Box".to_string());
    lowerer
        .resolver
        .item_paths
        .insert("Convert".to_string(), trait_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "Convert".to_string());
    lowerer
        .items
        .insert_impl(HirImpl {
            id: matching_impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: vec![GenericParamDecl::type_param(matching_generic, "T")],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: box_id,
                args: vec![Type::Generic(matching_generic)],
            }),
            trait_name: Some("Convert".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::Bool],
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "take".to_string(),
                empty_impl_function("take", function_id, Type::Unit),
            )]),
        })
        .unwrap();
    lowerer
        .items
        .insert_impl(HirImpl {
            id: other_impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: vec![GenericParamDecl::type_param(other_generic, "T")],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: box_id,
                args: vec![Type::Generic(other_generic)],
            }),
            trait_name: Some("Convert".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::I64],
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();
    let ast_impl = crate::ast::Impl {
        name: ast::ParseType::Type(ParseTypeInner {
            name: "Convert".to_string(),
            generics: vec![parse_type("Bool", Vec::new())],
            span: Span::test(),
        }),
        for_: Some(parse_type("Box", vec![parse_type("T", Vec::new())])),
        associated_types: Vec::new(),
        methods: HashMap::from([(ident("take"), empty_function_decl("take"))]),
        signatures: HashMap::from([(
            ident("take"),
            crate::ast::FunctionSig {
                name: ident("take"),
                sig: ast::ParseType::Function(vec![parse_type("T", Vec::new())]),
                where_clauses: Vec::new(),
                self_receiver: None,
                is_unsafe: false,
                exported: false,
            },
        )]),
        where_clauses: Vec::new(),
    };

    lowerer.lower_impl_bodies(&ast_impl, matching_impl_id);

    assert_eq!(
        lowerer.items.impl_def(matching_impl_id).unwrap().methods["take"].ret_type,
        Type::Generic(matching_generic)
    );
    assert!(!lowerer
        .items
        .impl_def(other_impl_id)
        .unwrap()
        .methods
        .contains_key("take"));
}

#[test]
fn lower_impl_bodies_rewraps_shared_signature_self_as_concrete_receiver() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(50);
    let impl_id = def_id(51);
    let function_id = def_id(52);
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "Point"));
    lowerer
        .resolver
        .item_paths
        .insert("Point".to_string(), struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "Point".to_string());
    let signature_self = GenericParamId {
        owner: function_id,
        index: 0,
    };
    let function = shared_method_placeholder(
        "get",
        function_id,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Generic(signature_self)),
        },
        Type::I64,
    );
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Point".to_string()),
            type_name: "Point".to_string(),
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
            methods: HashMap::from([("get".to_string(), function.clone())]),
        })
        .unwrap();
    let ast_impl = crate::ast::Impl {
        name: ast::ParseType::Type(ParseTypeInner {
            name: "Point".to_string(),
            generics: Vec::new(),
            span: Span::test(),
        }),
        for_: None,
        associated_types: Vec::new(),
        methods: HashMap::from([(
            ident("get"),
            FunctionDecl {
                name: ident("get"),
                lambda: LambdaDecl {
                    parameters: Vec::new(),
                    body: Block {
                        statements: vec![Statement::Expression(self_field_expr("ptr"))],
                    },
                    arrow_kind: LambdaArrowKind::Normal,
                    span: crate::lexer::Span::test(),
                },
                self_receiver: Some(SelfReceiverMode::Shared),
                is_unsafe: false,
                exported: false,
            },
        )]),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    lowerer.lower_impl_bodies(&ast_impl, impl_id);

    let lowered = &lowerer.items.impl_def(impl_id).unwrap().methods["get"];
    assert_eq!(
        lowered.params[0].ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Struct {
                id: struct_id,
                args: Vec::new(),
            }),
        }
    );
    assert_eq!(lowered.body.ty, Type::I64);
    let Some(crate::hir::HirStmt::Expr(expr)) = lowered.body.stmts.first() else {
        panic!("expected lowered field expression");
    };
    match &expr.kind {
        HirExprKind::FieldAccess(_, _, Some(location)) => {
            assert_eq!(location.owner, struct_id);
        }
        other => panic!("expected located field access, got {other:?}"),
    }
}

#[test]
fn lower_impl_bodies_rewraps_enum_self_as_enum_receiver() {
    let mut lowerer = Lowerer::new_for_test();
    let enum_id = def_id(60);
    let impl_id = def_id(61);
    let function_id = def_id(62);
    lowerer
        .items
        .insert_enumeration(unit_enum(enum_id, "Option", Vec::new()));
    lowerer
        .resolver
        .item_paths
        .insert("Option".to_string(), enum_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(enum_id, "Option".to_string());
    let signature_self = GenericParamId {
        owner: function_id,
        index: 0,
    };
    let function = shared_method_placeholder(
        "is_some",
        function_id,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Generic(signature_self)),
        },
        Type::Bool,
    );
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Option".to_string()),
            type_name: "Option".to_string(),
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
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("is_some".to_string(), function.clone())]),
        })
        .unwrap();
    let ast_impl = crate::ast::Impl {
        name: ast::ParseType::Type(ParseTypeInner {
            name: "Option".to_string(),
            generics: Vec::new(),
            span: Span::test(),
        }),
        for_: None,
        associated_types: Vec::new(),
        methods: HashMap::from([(
            ident("is_some"),
            FunctionDecl {
                name: ident("is_some"),
                lambda: LambdaDecl {
                    parameters: Vec::new(),
                    body: Block {
                        statements: vec![Statement::Expression(Expression::UnaryExpr(
                            UnaryExpr::PrimaryExpr(PrimaryExpr {
                                operand: Operand::Literal(Literal {
                                    kind: LiteralKind::Bool(true),
                                    span: Span::test(),
                                }),
                                secondaries: None,
                                type_annotation: None,
                            }),
                        ))],
                    },
                    arrow_kind: LambdaArrowKind::Normal,
                    span: crate::lexer::Span::test(),
                },
                self_receiver: Some(SelfReceiverMode::Shared),
                is_unsafe: false,
                exported: false,
            },
        )]),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    lowerer.lower_impl_bodies(&ast_impl, impl_id);

    let lowered = &lowerer.items.impl_def(impl_id).unwrap().methods["is_some"];
    assert_eq!(
        lowered.params[0].ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Enum {
                id: enum_id,
                args: Vec::new(),
            }),
        }
    );
}

#[test]
fn lower_impl_bodies_reports_missing_indexed_method_payload() {
    let mut lowerer = Lowerer::new_for_test();
    let impl_id = def_id(91);
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Missing".to_string()),
            type_name: "Missing".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::I64),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();
    let ast_impl = ast::Impl {
        name: ast::ParseType::Type(ParseTypeInner {
            name: "Missing".to_string(),
            generics: Vec::new(),
            span: Span::test(),
        }),
        for_: None,
        associated_types: Vec::new(),
        methods: HashMap::from([(ident("value"), empty_function_decl("value"))]),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    lowerer.lower_impl_bodies(&ast_impl, impl_id);

    assert!(lowerer.errors().iter().any(|error| error
        .message
        .contains("missing indexed impl method 'value' while lowering bodies")));
}
