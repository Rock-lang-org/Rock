use super::*;

fn find_first_struct_literal(
    block: &crate::hir::AcceptedHirBlock,
) -> Option<(
    &String,
    &Option<crate::ids::DefId>,
    &Vec<crate::hir::HirStructLiteralFieldFor<crate::hir::AcceptedHir>>,
)> {
    block.stmts.iter().find_map(|stmt| match stmt {
        crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
            find_first_struct_literal_expr(expr)
        }
        crate::hir::HirStmtFor::Let { value, .. } => find_first_struct_literal_expr(value),
        _ => None,
    })
}

fn find_first_struct_literal_expr(
    expr: &crate::hir::AcceptedHirExpr,
) -> Option<(
    &String,
    &Option<crate::ids::DefId>,
    &Vec<crate::hir::HirStructLiteralFieldFor<crate::hir::AcceptedHir>>,
)> {
    match &expr.kind {
        crate::hir::HirExprKindFor::StructLiteral(name, id, fields) => Some((name, id, fields)),
        crate::hir::HirExprKindFor::Call(callee, args, _) => find_first_struct_literal_expr(callee)
            .or_else(|| args.iter().find_map(find_first_struct_literal_expr)),
        crate::hir::HirExprKindFor::Block(block)
        | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_struct_literal(block),
        _ => None,
    }
}

fn find_first_enum_variant(
    block: &crate::hir::AcceptedHirBlock,
) -> Option<(
    &String,
    &String,
    &Vec<crate::hir::AcceptedHirExpr>,
    &Option<crate::hir::HirVariantLocation>,
)> {
    block.stmts.iter().find_map(|stmt| match stmt {
        crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
            find_first_enum_variant_expr(expr)
        }
        crate::hir::HirStmtFor::Let { value, .. } => find_first_enum_variant_expr(value),
        _ => None,
    })
}

fn find_first_enum_variant_expr(
    expr: &crate::hir::AcceptedHirExpr,
) -> Option<(
    &String,
    &String,
    &Vec<crate::hir::AcceptedHirExpr>,
    &Option<crate::hir::HirVariantLocation>,
)> {
    match &expr.kind {
        crate::hir::HirExprKindFor::EnumVariant(enum_name, variant_name, args, location) => {
            Some((enum_name, variant_name, args, location))
        }
        crate::hir::HirExprKindFor::Call(callee, args, _) => find_first_enum_variant_expr(callee)
            .or_else(|| args.iter().find_map(find_first_enum_variant_expr)),
        crate::hir::HirExprKindFor::Block(block)
        | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_enum_variant(block),
        _ => None,
    }
}

fn int_expr(value: &str) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Literal(crate::ast::Literal {
            kind: crate::ast::LiteralKind::Number(value.parse().unwrap()),
            span: crate::lexer::Span::test(),
        }),
        secondaries: None,
        type_annotation: None,
    }))
}

fn point_struct() -> HirStruct {
    HirStruct {
        id: def_id(10),
        name: "Point".to_string(),
        generic_params: Vec::new(),
        fields: vec![
            HirField {
                id: FieldId(0),
                name: "x".to_string(),
                ty: Type::I64,
                public: true,
            },
            HirField {
                id: FieldId(1),
                name: "y".to_string(),
                ty: Type::I64,
                public: true,
            },
        ],
    }
}

fn option_enum() -> HirEnum {
    HirEnum {
        id: def_id(20),
        name: "Maybe".to_string(),
        generic_params: Vec::new(),
        variants: vec![HirVariant {
            id: VariantId(0),
            name: "Some".to_string(),
            fields: HirVariantFields::Positional(vec![Type::I64]),
        }],
    }
}

fn record_enum() -> HirEnum {
    HirEnum {
        id: def_id(21),
        name: "Record".to_string(),
        generic_params: Vec::new(),
        variants: vec![HirVariant {
            id: VariantId(0),
            name: "Pair".to_string(),
            fields: HirVariantFields::Named(vec![
                HirField {
                    id: FieldId(0),
                    name: "first".to_string(),
                    ty: Type::I64,
                    public: true,
                },
                HirField {
                    id: FieldId(1),
                    name: "second".to_string(),
                    ty: Type::I64,
                    public: true,
                },
                HirField {
                    id: FieldId(2),
                    name: "third".to_string(),
                    ty: Type::I64,
                    public: true,
                },
                HirField {
                    id: FieldId(3),
                    name: "fourth".to_string(),
                    ty: Type::I64,
                    public: true,
                },
            ]),
        }],
    }
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

#[test]
fn lower_struct_literal_records_struct_id() {
    let source = r#"
struct Box
    < value: I64

main = ->
    box = Box
        value: 7
    box.value
"#;

    let resolved = compile_source_to_resolved_hir_for_test(source);
    let (_, main) = resolved.program.test_function_by_name("main").unwrap();
    let struct_id = resolved.program.names.structs_by_name["Box"];

    let literal = find_first_struct_literal(&main.body).unwrap();
    assert_eq!(literal.1, &Some(struct_id));
}

#[test]
fn lower_enum_variant_records_enum_and_variant_ids() {
    let source = r#"
enum Maybe
    Some I64
    None

main = ->
    Maybe::Some 7
"#;

    let resolved = compile_source_to_resolved_hir_for_test(source);
    let (_, main) = resolved.program.test_function_by_name("main").unwrap();
    let (enum_id, maybe_enum) = resolved.program.test_enum_by_name("Maybe").unwrap();
    let expected_variant_id = maybe_enum
        .variants
        .iter()
        .find(|variant| variant.name == "Some")
        .expect("Maybe::Some exists")
        .id;

    let variant = find_first_enum_variant(&main.body).unwrap();
    let location = variant.3.as_ref().expect("enum variant must be resolved");
    assert_eq!(location.owner, enum_id);
    assert_eq!(location.variant_id, expected_variant_id);
    assert_eq!(location.name, "Some");
}

#[test]
fn lowers_known_field_access_with_resolved_field_location() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_structure(point_struct());
    register_item_path(&mut lowerer, "Point", def_id(10));

    let base = HirExpr {
        kind: HirExprKind::Var("point".to_string()),
        ty: Type::Struct {
            id: def_id(10),
            args: Vec::new(),
        },
        span: crate::lexer::Span::test(),
    };

    let field = lowerer.apply_secondary(
        base,
        &ast::SecondaryExpr::Dot(ast::IdentOrNumber::Ident(ident("y"))),
        crate::lower::expression::ExprUse::Value,
        false,
    );

    match field.kind {
        HirExprKind::FieldAccess(_, name, Some(location)) => {
            assert_eq!(name, "y");
            assert_eq!(location.owner, def_id(10));
            assert_eq!(location.field_id, FieldId(1));
            assert_eq!(location.name, "y");
        }
        other => panic!("expected resolved field access, got {other:?}"),
    }
}

#[test]
fn lowers_known_struct_literal_fields_with_resolved_field_locations() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_structure(point_struct());
    register_item_path(&mut lowerer, "Point", def_id(10));

    let literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![IdentOrType::Ident(ident("Point"))],
        },
        fields: HashMap::from([(ident("x"), int_expr("1")), (ident("y"), int_expr("2"))]),
    });

    match literal.kind {
        HirExprKind::StructLiteral(_, _, fields) => {
            let y = fields.iter().find(|field| field.name == "y").unwrap();
            let location = y.field.as_ref().expect("known field should be resolved");
            assert_eq!(location.owner, def_id(10));
            assert_eq!(location.field_id, FieldId(1));
            assert_eq!(location.name, "y");
        }
        other => panic!("expected struct literal, got {other:?}"),
    }
}

#[test]
fn struct_literal_resolved_fields_all_have_locations() {
    let point_id = def_id(10);
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_structure(point_struct());
    register_item_path(&mut lowerer, "Point", point_id);

    let literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![IdentOrType::Ident(ident("Point"))],
        },
        fields: HashMap::from([(ident("x"), int_expr("1")), (ident("y"), int_expr("2"))]),
    });

    match literal.kind {
        HirExprKind::StructLiteral(struct_name, Some(id), fields) => {
            assert_eq!(struct_name, "Point");
            assert_eq!(id, point_id);
            let x = fields.iter().find(|field| field.name == "x").unwrap();
            let location = x
                .field
                .as_ref()
                .expect("resolved field should have location");
            assert_eq!(location.owner, point_id);
            assert_eq!(location.field_id, FieldId(0));
            assert_eq!(location.name, "x");
            assert!(
                fields.iter().all(|field| field.field.is_some()),
                "all resolved fields should have locations"
            );
        }
        other => panic!("expected resolved Point struct literal, got {other:?}"),
    }
}

#[test]
fn module_local_struct_alias_shadows_root_struct_for_instance_construction() {
    let mut lowerer = Lowerer::new_for_test();
    let root_id = def_id(48);
    let module_id = def_id(49);
    lowerer
        .items
        .insert_structure(empty_struct(root_id, "Thing"));
    lowerer
        .items
        .insert_structure(empty_struct(module_id, "demo::helper::Thing"));
    lowerer
        .resolver
        .item_paths
        .insert("Thing".to_string(), root_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_id, "Thing".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "Thing".to_string(),
        "demo::helper::Thing".to_string(),
        module_id,
    );

    let literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![IdentOrType::Ident(ident("Thing"))],
        },
        fields: HashMap::new(),
    });

    match literal.kind {
        HirExprKind::StructLiteral(name, Some(id), _) => {
            assert_eq!(name, "demo::helper::Thing");
            assert_eq!(id, module_id);
        }
        other => panic!("expected module alias struct literal, got {other:?}"),
    }
}

#[test]
fn struct_literal_private_field_check_matches_current_impl_by_struct_id() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(56);
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "String"));
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "std::string_type::String"));
    lowerer
        .resolver
        .item_paths
        .insert("String".to_string(), struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "std::string_type::String".to_string());
    lowerer.current_struct_impl = Some("String".to_string());

    let _literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![IdentOrType::Ident(ident("String"))],
        },
        fields: HashMap::from([(ident("ptr"), int_expr("1"))]),
    });

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
fn struct_literal_private_field_check_matches_current_impl_by_pre_resolved_id() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(58);
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "std::string_type::String"));
    lowerer
        .resolver
        .item_paths
        .insert("String".to_string(), struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "std::string_type::String".to_string());
    lowerer.current_struct_impl = Some("std::string_type::String".to_string());

    let _literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![IdentOrType::Ident(ident("String"))],
        },
        fields: HashMap::from([(ident("ptr"), int_expr("1"))]),
    });

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
fn struct_literal_private_field_check_requires_resolved_current_impl_id() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(57);
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "String"));
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "std::string_type::String"));
    register_item_path(&mut lowerer, "std::string_type::String", struct_id);
    lowerer.current_struct_impl = Some("String".to_string());

    let _literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![
                IdentOrType::Ident(ident("std")),
                IdentOrType::Ident(ident("string_type")),
                IdentOrType::Ident(ident("String")),
            ],
        },
        fields: HashMap::from([(ident("ptr"), int_expr("1"))]),
    });

    assert!(
        lowerer
            .errors()
            .iter()
            .any(|error| error.message.contains("private fields")),
        "unresolved current impl name should not grant private construction: {:?}",
        lowerer.errors()
    );
}

#[test]
fn lowers_known_enum_variant_constructor_with_resolved_variant_location() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_enumeration(option_enum());
    register_item_path(&mut lowerer, "Maybe", def_id(20));

    let variant = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![
                IdentOrType::Ident(ident("Maybe")),
                IdentOrType::Ident(ident("Some")),
            ],
        },
        fields: HashMap::from([(ident("value"), int_expr("1"))]),
    });

    match variant.kind {
        HirExprKind::EnumVariant(enum_name, variant_name, _, Some(location)) => {
            assert_eq!(enum_name, "Maybe");
            assert_eq!(variant_name, "Some");
            assert_eq!(location.owner, def_id(20));
            assert_eq!(location.variant_id, VariantId(0));
            assert_eq!(location.name, "Some");
        }
        other => panic!("expected resolved enum variant, got {other:?}"),
    }
}

#[test]
fn lowers_named_enum_variant_fields_in_variant_definition_order() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_enumeration(record_enum());
    register_item_path(&mut lowerer, "Record", def_id(21));

    let variant = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![
                IdentOrType::Ident(ident("Record")),
                IdentOrType::Ident(ident("Pair")),
            ],
        },
        fields: HashMap::from([
            (ident("fourth"), int_expr("4")),
            (ident("third"), int_expr("3")),
            (ident("second"), int_expr("2")),
            (ident("first"), int_expr("1")),
        ]),
    });

    match variant.kind {
        HirExprKind::EnumVariant(_, _, args, Some(location)) => {
            assert_eq!(location.owner, def_id(21));
            assert_eq!(location.variant_id, VariantId(0));
            assert!(matches!(args[0].kind, HirExprKind::IntLiteral(1)));
            assert!(matches!(args[1].kind, HirExprKind::IntLiteral(2)));
            assert!(matches!(args[2].kind, HirExprKind::IntLiteral(3)));
            assert!(matches!(args[3].kind, HirExprKind::IntLiteral(4)));
        }
        other => panic!("expected resolved enum variant, got {other:?}"),
    }
}

#[test]
fn leaves_unknown_struct_literal_fields_unresolved() {
    let mut lowerer = Lowerer::new_for_test();

    let literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![IdentOrType::Ident(ident("Unknown"))],
        },
        fields: HashMap::from([(ident("value"), int_expr("1"))]),
    });

    match literal.kind {
        HirExprKind::StructLiteral(_, _, fields) => {
            assert_eq!(fields[0].name, "value");
            assert!(fields[0].field.is_none());
        }
        other => panic!("expected fallback struct literal, got {other:?}"),
    }
}
