use super::*;

fn build_struct_with_id(context: &mut CollectContext, decl: &StructDecl, id: DefId) -> HirStruct {
    let params = build_nominal_generic_params(context, &decl.generic_params, id);
    super::super::build_struct_with_id(context, decl, id, params)
}

fn build_enum_with_id(context: &mut CollectContext, decl: &EnumDecl, id: DefId) -> HirEnum {
    let params = build_nominal_generic_params(context, &decl.generic_params, id);
    super::super::build_enum_with_id(context, decl, id, params)
}

fn generic_type_inner(name: &str) -> ParseTypeInner {
    ParseTypeInner {
        name: name.to_string(),
        generics: vec![],
        span: Span::test(),
    }
}

fn field(name: &str, ty: ParseType, public: bool) -> StructDeclField {
    StructDeclField {
        name: ident(name),
        ty,
        public,
        default: None,
    }
}

#[test]
fn test_build_struct_lowers_struct_field_types() {
    let mut context = CollectContext::new();
    let decl = StructDecl {
        name: ParseTypeInner {
            name: "Buffer".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![generic_param("T")],
        fields: vec![
            field(
                "bytes",
                ParseType::Reference {
                    is_mut: false,
                    pointee: Box::new(ParseType::Slice(Box::new(named_type("U8")))),
                },
                true,
            ),
            field(
                "cursor",
                ParseType::Reference {
                    is_mut: true,
                    pointee: Box::new(named_type("I64")),
                },
                false,
            ),
        ],
        exported: false,
    };

    let hir_struct = build_struct_with_id(&mut context, &decl, def_id(1));

    assert_eq!(hir_struct.name, "Buffer");
    assert_eq!(
        hir_struct.generic_params,
        vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(1),
                index: 0,
            },
            "T",
        )]
    );
    assert_eq!(hir_struct.fields.len(), 2);
    assert_eq!(hir_struct.fields[0].name, "bytes");
    assert_eq!(
        hir_struct.fields[0].ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Slice(Box::new(Type::U8))),
        }
    );
    assert!(hir_struct.fields[0].public);
    assert_eq!(hir_struct.fields[1].name, "cursor");
    assert_eq!(
        hir_struct.fields[1].ty,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::I64),
        }
    );
    assert!(!hir_struct.fields[1].public);
}

#[test]
fn build_struct_preserves_constructor_generic_kind_and_application() {
    let owner = def_id(90);
    let constructor_param = ast::GenericParamDecl {
        default: None,
        name: ident("F"),
        kind: Some(ast::TypeApplication {
            constructor: Box::new(named_type("F")),
            args: vec![ast::ParseType::Hole(ast::TypeHole { span: Span::test() })],
            span: Span::test(),
        }),
        span: Span::test(),
    };
    let decl = StructDecl {
        name: type_inner("Wrapper"),
        generic_params: vec![constructor_param, generic_param("A")],
        fields: vec![field(
            "value",
            ast::ParseType::Application(ast::TypeApplication {
                constructor: Box::new(named_type("F")),
                args: vec![named_type("A")],
                span: Span::test(),
            }),
            true,
        )],
        exported: false,
    };

    let strukt = build_struct_with_id(&mut CollectContext::new(), &decl, owner);

    assert_eq!(
        strukt.generic_params[0].kind,
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        )
    );
    assert_eq!(
        strukt.fields[0].ty,
        Type::Apply {
            constructor: Box::new(Type::Generic(GenericParamId { owner, index: 0 })),
            args: vec![Type::Generic(GenericParamId { owner, index: 1 })],
        }
    );
}

#[test]
fn build_struct_assigns_source_order_field_ids_per_owner() {
    let mut context = CollectContext::new();
    let left = StructDecl {
        name: type_inner("Left"),
        generic_params: vec![],
        fields: vec![field("value", named_type("I32"), true)],
        exported: false,
    };
    let right = StructDecl {
        name: type_inner("Right"),
        generic_params: vec![],
        fields: vec![
            field("value", named_type("I32"), true),
            field("other", named_type("I64"), true),
        ],
        exported: false,
    };

    let left = build_struct_with_id(&mut context, &left, def_id(1));
    let right = build_struct_with_id(&mut context, &right, def_id(2));

    assert_eq!(left.fields[0].id, FieldId(0));
    assert_eq!(right.fields[0].id, FieldId(0));
    assert_eq!(right.fields[1].id, FieldId(1));
}

#[test]
fn test_build_struct_rejects_bare_str_field_type() {
    let mut context = CollectContext::new();
    let decl = StructDecl {
        name: ParseTypeInner {
            name: "TextHolder".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![],
        fields: vec![field("text", named_type("Str"), true)],
        exported: false,
    };

    let hir_struct = build_struct_with_id(&mut context, &decl, def_id(1));

    assert_eq!(hir_struct.fields[0].ty, Type::Error);
    assert!(context.errors.iter().any(|err| err
        .message
        .contains("bare string slice type Str must be written behind a reference")));
}

#[test]
fn test_build_struct_accepts_borrowed_str_field_type() {
    let mut context = CollectContext::new();
    let decl = StructDecl {
        name: ParseTypeInner {
            name: "TextHolder".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![],
        fields: vec![field(
            "text",
            ParseType::Reference {
                is_mut: false,
                pointee: Box::new(named_type("Str")),
            },
            true,
        )],
        exported: false,
    };

    let hir_struct = build_struct_with_id(&mut context, &decl, def_id(1));

    assert_eq!(
        hir_struct.fields[0].ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        }
    );
    assert!(context.errors.is_empty());
}

#[test]
fn test_build_enum_mirrors_variant_header_construction() {
    let mut lowerer = CollectContext::new();
    let decl = EnumDecl {
        generic_params: vec![generic_param("T")],
        name: ParseTypeInner {
            name: "Message".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        variants: vec![
            EnumVariant {
                name: generic_type_inner("Data"),
                fields: NamedFieldsOrTypesList::NamedFields(vec![field(
                    "payload",
                    ParseType::Pointer(Box::new(named_type("U8"))),
                    true,
                )]),
            },
            EnumVariant {
                name: generic_type_inner("Pair"),
                fields: NamedFieldsOrTypesList::TypesList(vec![
                    named_type("I32"),
                    ParseType::Array {
                        inner: Box::new(named_type("Bool")),
                        len: ast::ParseArrayLen::Literal(2),
                    },
                ]),
            },
            EnumVariant {
                name: generic_type_inner("Empty"),
                fields: NamedFieldsOrTypesList::TypesList(vec![]),
            },
        ],
        exported: false,
        language_items: Default::default(),
    };

    let hir_enum = build_enum_with_id(&mut lowerer, &decl, def_id(1));

    assert_eq!(hir_enum.name, "Message");
    assert_eq!(
        hir_enum.generic_params,
        vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(1),
                index: 0,
            },
            "T",
        )]
    );
    assert_eq!(hir_enum.variants.len(), 3);
    match &hir_enum.variants[0].fields {
        HirVariantFields::Named(fields) => {
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].name, "payload");
            assert_eq!(fields[0].ty, Type::Pointer(Box::new(Type::U8)));
            assert!(fields[0].public);
        }
        other => panic!("expected named fields, got {other:?}"),
    }
    match &hir_enum.variants[1].fields {
        HirVariantFields::Positional(types) => {
            assert_eq!(
                types,
                &vec![Type::I32, Type::Array(Box::new(Type::Bool), crate::types::ArrayLen::Value(2))]
            );
        }
        other => panic!("expected positional fields, got {other:?}"),
    }
    assert!(matches!(
        hir_enum.variants[2].fields,
        HirVariantFields::Unit
    ));
}

#[test]
fn build_enum_assigns_variant_ids_and_owner_unique_named_field_ids() {
    let mut context = CollectContext::new();
    let decl = EnumDecl {
        generic_params: Vec::new(),
        name: type_inner("Message"),
        variants: vec![
            EnumVariant {
                name: generic_type_inner("First"),
                fields: NamedFieldsOrTypesList::NamedFields(vec![
                    field("value", named_type("I32"), true),
                    field("shared", named_type("I64"), true),
                ]),
            },
            EnumVariant {
                name: generic_type_inner("Second"),
                fields: NamedFieldsOrTypesList::NamedFields(vec![field(
                    "value",
                    named_type("Bool"),
                    true,
                )]),
            },
        ],
        exported: false,
        language_items: Default::default(),
    };

    let hir_enum = build_enum_with_id(&mut context, &decl, def_id(1));

    assert_eq!(hir_enum.variants[0].id, VariantId(0));
    assert_eq!(hir_enum.variants[1].id, VariantId(1));
    let HirVariantFields::Named(first_fields) = &hir_enum.variants[0].fields else {
        panic!("expected first variant to have named fields");
    };
    let HirVariantFields::Named(second_fields) = &hir_enum.variants[1].fields else {
        panic!("expected second variant to have named fields");
    };
    assert_eq!(first_fields[0].id, FieldId(0));
    assert_eq!(first_fields[1].id, FieldId(1));
    assert_eq!(second_fields[0].id, FieldId(2));
}
