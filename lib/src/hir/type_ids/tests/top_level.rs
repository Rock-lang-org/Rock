use super::*;

fn program_with_function_body(owner: DefId, body: HirBlock) -> HirProgram {
    let function = HirFunction {
        id: owner,
        name: "main".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: body.ty.clone(),
        body,
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    program(
        HashMap::from([(owner, function)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("main".to_string(), owner)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    )
}

#[test]
fn hir_type_ids_record_and_read_locations() {
    let function = def_id(1);
    let location = HirTypeLocation::FunctionReturn { function };
    let id = crate::ids::TypeId(7);
    let mut ids = HirTypeIds::new();

    assert!(ids.is_empty());
    ids.insert(location.clone(), id);

    assert_eq!(ids.get(&location), Some(id));
    assert_eq!(ids.len(), 1);
    assert!(!ids.is_empty());
    assert_eq!(ids.iter().collect::<Vec<_>>(), vec![(&location, id)]);
}

#[test]
fn hir_type_ids_collect_top_level_type_locations() {
    let function_id = def_id(1);
    let extern_id = def_id(2);
    let struct_id = def_id(3);
    let other_struct_id = def_id(4);
    let enum_id = def_id(5);
    let trait_id = def_id(6);
    let signature_id = def_id(7);
    let impl_id = def_id(8);
    let assoc_id = AssocTypeId(0);
    let generic_param = generic(impl_id, 0);
    let projection = Type::Projection {
        ty: Box::new(Type::Generic(generic_param)),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: assoc_id,
        },
        trait_args: vec![Type::I64],
    };

    let program = program(
        HashMap::from([(
            function_id,
            function(function_id, "id", Type::I64, Type::I64),
        )]),
        HashMap::from([
            (
                struct_id,
                HirStruct {
                    id: struct_id,
                    name: "Box".to_string(),
                    generic_params: Vec::new(),
                    fields: vec![HirField {
                        id: FieldId(0),
                        name: "value".to_string(),
                        ty: Type::Struct {
                            id: struct_id,
                            args: Vec::new(),
                        },
                        public: false,
                    }],
                },
            ),
            (
                other_struct_id,
                HirStruct {
                    id: other_struct_id,
                    name: "OtherBox".to_string(),
                    generic_params: Vec::new(),
                    fields: vec![HirField {
                        id: FieldId(0),
                        name: "value".to_string(),
                        ty: Type::Struct {
                            id: other_struct_id,
                            args: Vec::new(),
                        },
                        public: false,
                    }],
                },
            ),
        ]),
        HashMap::from([(
            enum_id,
            HirEnum {
                id: enum_id,
                name: "Maybe".to_string(),
                generic_params: Vec::new(),
                variants: vec![
                    HirVariant {
                        id: VariantId(0),
                        name: "Named".to_string(),
                        fields: HirVariantFields::Named(vec![HirField {
                            id: FieldId(0),
                            name: "payload".to_string(),
                            ty: Type::Bool,
                            public: false,
                        }]),
                    },
                    HirVariant {
                        id: VariantId(1),
                        name: "Tuple".to_string(),
                        fields: HirVariantFields::Positional(vec![Type::I64]),
                    },
                ],
            },
        )]),
        HashMap::from([(
            trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Iterable".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "next".to_string(),
                    HirFunctionSig {
                        id: signature_id,
                        name: "next".to_string(),
                        generic_params: Vec::new(),
                        params: vec![Type::I64],
                        ret: Type::Bool,
                        generic_bounds: HashMap::new().into(),
                        self_receiver: None,
                        is_unsafe: false,
                    },
                )]),
            },
        )]),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: GenericParamDecl::type_params(impl_id, ["T"]),
                receiver_pattern: vec![Type::Generic(generic_param)].into(),
                trait_name: Some("Iterable".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: vec![Type::I64],
                associated_types: vec![HirAssociatedTypeDef {
                    id: assoc_id,
                    name: "Item".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: projection.clone(),
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            },
        )]),
        HashMap::from([(
            extern_id,
            HirExtern {
                id: extern_id,
                name: "puts".to_string(),
                params: vec![Type::Pointer(Box::new(Type::U8))],
                ret: Type::I32,
                variadic: false,
                is_unsafe: false,
            },
        )]),
        HirNameTables {
            functions_by_name: HashMap::from([("id".to_string(), function_id)]),
            structs_by_name: HashMap::from([
                ("Box".to_string(), struct_id),
                ("OtherBox".to_string(), other_struct_id),
            ]),
            enums_by_name: HashMap::from([("Maybe".to_string(), enum_id)]),
            traits_by_name: HashMap::from([("Iterable".to_string(), trait_id)]),
            externs_by_name: HashMap::from([("puts".to_string(), extern_id)]),
            type_aliases_by_name: HashMap::new(),
        },
        HashMap::new(),
    );

    let mut context = TypeContext::new();
    let ids = collect_hir_type_ids(&program, &mut context);

    let function_ret = ids
        .get(&HirTypeLocation::FunctionReturn {
            function: function_id,
        })
        .unwrap();
    let function_param = ids
        .get(&HirTypeLocation::FunctionParam {
            function: function_id,
            index: 0,
        })
        .unwrap();
    assert_eq!(function_ret, function_param);
    assert_eq!(context.type_for(function_ret), Type::I64);
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ExternParam {
                extern_id,
                index: 0,
            })
            .unwrap()
        ),
        Type::Pointer(Box::new(Type::U8))
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ExternReturn { extern_id })
                .unwrap()
        ),
        Type::I32
    );
    let first_nominal = ids
        .get(&HirTypeLocation::StructField {
            owner: struct_id,
            field: FieldId(0),
        })
        .unwrap();
    let second_nominal = ids
        .get(&HirTypeLocation::StructField {
            owner: other_struct_id,
            field: FieldId(0),
        })
        .unwrap();
    assert_ne!(first_nominal, second_nominal);
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::EnumVariantNamedField {
                owner: enum_id,
                variant: VariantId(0),
                field: FieldId(0),
            })
            .unwrap()
        ),
        Type::Bool
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::EnumVariantPositionalField {
                owner: enum_id,
                variant: VariantId(1),
                index: 0,
            })
            .unwrap()
        ),
        Type::I64
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::TraitSignatureReturn {
                trait_id,
                signature: signature_id,
            })
            .unwrap()
        ),
        Type::Bool
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::TraitSignatureParam {
                trait_id,
                signature: signature_id,
                index: 0,
            })
            .unwrap()
        ),
        Type::I64
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ImplReceiverArg { impl_id, index: 0 })
                .unwrap()
        ),
        Type::Generic(generic_param)
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ImplTraitArg { impl_id, index: 0 })
                .unwrap()
        ),
        Type::I64
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::AssociatedTypeDef {
                impl_id,
                assoc_type: assoc_id,
            })
            .unwrap()
        ),
        projection
    );
    assert!(!ids.is_empty());
}

#[test]
fn hir_type_ids_collect_downstream_required_payload_types() {
    let owner = def_id(200);
    let trait_id = def_id(201);
    let struct_id = def_id(202);
    let method_id = def_id(203);
    let mut context = TypeContext::new();
    let expr = HirExpr {
        kind: HirExprKind::Cast(
            Box::new(HirExpr {
                kind: HirExprKind::MethodCall(
                    Box::new(HirExpr {
                        kind: HirExprKind::Var("receiver".to_string()),
                        ty: Type::Struct {
                            id: struct_id,
                            args: vec![Type::I64],
                        },
                        span: crate::lexer::Span::test(),
                    }),
                    "value".to_string(),
                    Vec::new(),
                    None,
                    Some(HirMethodCallTarget::impl_method(
                        owner,
                        method_id,
                        Some(crate::hir::HirSelectedTraitMember {
                            trait_id,
                            member_id: method_id,
                            trait_args: vec![Type::Bool],
                        }),
                    )),
                ),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }),
            Type::U64,
        ),
        ty: Type::U64,
        span: crate::lexer::Span::test(),
    };
    let pattern = HirPattern::Struct(
        "Box".to_string(),
        Some(struct_id),
        vec![Type::Bool],
        Vec::new(),
    );
    let program = program_with_function_body(
        owner,
        HirBlock {
            stmts: vec![HirStmt::Expr(expr)],
            ty: Type::Unit,
        },
    );
    let ids = collect_hir_type_ids(&program, &mut context);

    assert!(ids
        .get(&HirTypeLocation::CastTarget {
            owner,
            path: vec![0, 0],
        })
        .is_some());
    assert!(ids
        .get(&HirTypeLocation::MethodCallTraitArg {
            owner,
            path: vec![0, 0, 0],
            index: 0,
        })
        .is_some());

    let mut pattern_context = TypeContext::new();
    let mut pattern_ids = HirTypeIds::new();
    collect_pattern_type_ids(
        owner,
        &pattern,
        vec![9],
        &mut pattern_context,
        &mut pattern_ids,
    );
    assert!(pattern_ids
        .get(&HirTypeLocation::StructPatternArg {
            owner,
            path: vec![9],
            index: 0,
        })
        .is_some());
}
