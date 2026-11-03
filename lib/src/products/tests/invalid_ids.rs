use super::*;

#[test]
#[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
fn compiler_products_reject_function_generic_param_decls_with_invalid_current_crate_ids() {
    let original_id = DefId::new(CrateId(u32::MAX), LocalDefId(17));
    let original_generic = crate::types::GenericParamId {
        owner: original_id,
        index: 0,
    };
    let mut function = test_function(original_id, "identity", generic_params(original_id, &["T"]));
    function.generic_params[0].id = original_generic;
    function.params[0].ty = Type::Generic(original_generic);
    function.ret_type = Type::Generic(original_generic);
    function.body = empty_body(Type::Generic(original_generic));

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &ResolvedHirProgram::new(
            HirProgram::from_id_parts_with_names_and_canonical_names(
                HashMap::from([(original_id, function)]),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HirNameTables {
                    functions_by_name: HashMap::from([("identity".to_string(), original_id)]),
                    ..HirNameTables::default()
                },
                HirLanguageItems::default(),
                &HashMap::from([(original_id, "identity".to_string())]),
            ),
            ResolverTables::default(),
            BTreeSet::from([original_id]),
            CrateId(0),
            IdGen::new(),
        ),
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let (product_id, function) = products
        .interface
        .functions
        .iter()
        .next()
        .expect("product should contain remapped generic function metadata");
    let expected_owner = DefId::new(
        CrateId(product_id.crate_id.0),
        LocalDefId(product_id.local_id.0),
    );

    assert_ne!(*product_id, ProductDefId::from(original_id));
    assert_eq!(function.generic_params[0].id.owner, expected_owner);
    assert_eq!(
        function.params[0],
        Type::Generic(crate::types::GenericParamId {
            owner: expected_owner,
            index: 0,
        })
    );
}

#[test]
#[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
fn compiler_products_reject_function_body_generic_owners_with_invalid_current_crate_ids() {
    let function_id = DefId::new(CrateId(0), LocalDefId(17));
    let invalid_owner = DefId::new(CrateId(u32::MAX), LocalDefId(18));
    let invalid_generic = crate::types::GenericParamId {
        owner: invalid_owner,
        index: 0,
    };
    let mut function = test_function(function_id, "identity", generic_params(function_id, &["T"]));
    function.generic_params[0].id = invalid_generic;
    function.params[0].ty = Type::Generic(invalid_generic);
    function.ret_type = Type::Generic(invalid_generic);
    function.body = empty_body(Type::Generic(invalid_generic));

    let _ = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &ResolvedHirProgram::new(
            HirProgram::from_id_parts_with_names_and_canonical_names(
                HashMap::from([(function_id, function)]),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HirNameTables {
                    functions_by_name: HashMap::from([("identity".to_string(), function_id)]),
                    ..HirNameTables::default()
                },
                HirLanguageItems::default(),
                &HashMap::from([(function_id, "identity".to_string())]),
            ),
            ResolverTables::default(),
            BTreeSet::from([function_id]),
            CrateId(0),
            IdGen::new(),
        ),
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
}

#[test]
#[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
fn compiler_products_reject_trait_default_body_generic_owners_with_invalid_current_crate_ids() {
    let trait_id = DefId::new(CrateId(u32::MAX), LocalDefId(17));
    let method_id = DefId::new(CrateId(u32::MAX), LocalDefId(18));
    let self_generic = crate::types::GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let method = HirFunction {
        id: method_id,
        name: "identity".to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(self_generic),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(self_generic),
        body: HirBlock {
            stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
                kind: crate::hir::HirExprKindFor::Var("self".to_string()),
                ty: Type::Generic(self_generic),
                span: Span::test(),
            })],
            ty: Type::Generic(self_generic),
        },
        is_curried: false,
        is_method: true,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Identity".to_string(),
                generic_params: vec![],
                associated_types: vec![],
                methods: HashMap::from([("identity".to_string(), method)]),
                signatures: HashMap::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([("Identity".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([(trait_id, "Identity".to_string())]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([trait_id, method_id]),
        CrateId(0),
        IdGen::new(),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let trait_product_id = products
        .interface
        .traits
        .keys()
        .next()
        .copied()
        .expect("product should contain trait interface");
    let expected_owner = DefId::new(
        CrateId(trait_product_id.crate_id.0),
        LocalDefId(trait_product_id.local_id.0),
    );
    let method = products
        .bodies
        .trait_default_methods
        .values()
        .next()
        .expect("product should contain trait default body");
    let crate::hir::HirStmtFor::Expr(expr) = &method.body.stmts[0] else {
        panic!("expected expression statement in default method body");
    };

    assert_eq!(
        expr.ty,
        Type::Generic(crate::types::GenericParamId {
            owner: expected_owner,
            index: 0,
        })
    );
}

#[test]
#[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
fn compiler_products_reject_generic_impl_method_body_generic_owners_with_invalid_current_crate_ids()
{
    let impl_id = DefId::new(CrateId(u32::MAX), LocalDefId(19));
    let method_id = DefId::new(CrateId(u32::MAX), LocalDefId(20));
    let impl_generic = crate::types::GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_generic = crate::types::GenericParamId {
        owner: method_id,
        index: 0,
    };
    let method = HirFunction {
        id: method_id,
        name: "get".to_string(),
        generic_params: vec![GenericParamDecl::type_param(method_generic, "U")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(impl_generic),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(method_generic),
        body: HirBlock {
            stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
                kind: crate::hir::HirExprKindFor::Var("self".to_string()),
                ty: Type::Generic(impl_generic),
                span: Span::test(),
            })],
            ty: Type::Generic(impl_generic),
        },
        is_curried: false,
        is_method: true,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: generic_params(impl_id, &["T"]),
                receiver_pattern: vec![Type::Generic(impl_generic)].into(),
                trait_name: None,
                trait_id: None,
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("get".to_string(), method)]),
            },
        )]),
        HashMap::new(),
        HirNameTables::default(),
        HirLanguageItems::default(),
        &HashMap::new(),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([impl_id, method_id]),
        CrateId(0),
        IdGen::new(),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let impl_product_id = products
        .interface
        .impls
        .keys()
        .next()
        .copied()
        .expect("product should contain impl interface");
    let expected_owner = DefId::new(
        CrateId(impl_product_id.crate_id.0),
        LocalDefId(impl_product_id.local_id.0),
    );
    let imp = products
        .bodies
        .generic_impls
        .values()
        .next()
        .expect("product should contain generic impl body");
    let method = &imp.methods["get"];
    assert_ne!(method.id, method_id);
    assert_ne!(method.id.crate_id, CrateId(u32::MAX));
    assert_eq!(
        method.generic_params[0].id.owner, method.id,
        "method-owned generic params should be rehomed to the product method ID"
    );
    assert_eq!(
        method.ret_type,
        Type::Generic(crate::types::GenericParamId {
            owner: method.id,
            index: 0,
        })
    );
    let crate::hir::HirStmtFor::Expr(expr) = &method.body.stmts[0] else {
        panic!("expected expression statement in impl method body");
    };

    assert_eq!(
        expr.ty,
        Type::Generic(crate::types::GenericParamId {
            owner: expected_owner,
            index: 0,
        })
    );
}

#[test]
#[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
fn compiler_products_reject_body_field_and_variant_location_owners_with_invalid_current_crate_ids()
{
    let struct_id = DefId::new(CrateId(u32::MAX), LocalDefId(21));
    let enum_id = DefId::new(CrateId(u32::MAX), LocalDefId(22));
    let impl_id = DefId::new(CrateId(u32::MAX), LocalDefId(23));
    let method_id = DefId::new(CrateId(u32::MAX), LocalDefId(24));
    let method = HirFunction {
        id: method_id,
        name: "locations".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        generic_bounds: HashMap::new().into(),
        params: vec![],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: vec![
                crate::hir::HirStmtFor::Expr(HirExpr {
                    kind: crate::hir::HirExprKindFor::FieldAccess(
                        Box::new(HirExpr {
                            kind: crate::hir::HirExprKindFor::Var("boxed".to_string()),
                            ty: Type::Struct {
                                id: struct_id,
                                args: vec![],
                            },
                            span: Span::test(),
                        }),
                        "value".to_string(),
                        Some(crate::hir::HirFieldLocation {
                            owner: struct_id,
                            field_id: FieldId(0),
                            name: "value".to_string(),
                        }),
                    ),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                crate::hir::HirStmtFor::Expr(HirExpr {
                    kind: crate::hir::HirExprKindFor::EnumVariant(
                        "Maybe".to_string(),
                        "Some".to_string(),
                        vec![],
                        Some(crate::hir::HirVariantLocation {
                            owner: enum_id,
                            variant_id: VariantId(0),
                            name: "Some".to_string(),
                        }),
                    ),
                    ty: Type::Enum {
                        id: enum_id,
                        args: vec![],
                    },
                    span: Span::test(),
                }),
            ],
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::from([(
            struct_id,
            HirStruct {
                id: struct_id,
                name: "Box".to_string(),
                generic_params: vec![],
                fields: Vec::new(),
            },
        )]),
        HashMap::from([(
            enum_id,
            HirEnum {
                id: enum_id,
                name: "Maybe".to_string(),
                generic_params: vec![],
                variants: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: generic_params(impl_id, &["T"]),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: vec![Type::Generic(crate::types::GenericParamId {
                        owner: impl_id,
                        index: 0,
                    })],
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("locations".to_string(), method)]),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([("Box".to_string(), struct_id)]),
            enums_by_name: HashMap::from([("Maybe".to_string(), enum_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([
            (struct_id, "Box".to_string()),
            (enum_id, "Maybe".to_string()),
        ]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([struct_id, enum_id, impl_id, method_id]),
        CrateId(0),
        IdGen::new(),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let expected_struct_owner = products
        .interface
        .structs
        .keys()
        .next()
        .copied()
        .map(|id| DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0)))
        .expect("product should contain struct interface");
    let expected_enum_owner = products
        .interface
        .enums
        .keys()
        .next()
        .copied()
        .map(|id| DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0)))
        .expect("product should contain enum interface");
    let imp = products
        .bodies
        .generic_impls
        .values()
        .next()
        .expect("product should contain generic impl body");
    let method = &imp.methods["locations"];
    let crate::hir::HirStmtFor::Expr(field_expr) = &method.body.stmts[0] else {
        panic!("expected field access expression statement");
    };
    let crate::hir::HirExprKindFor::FieldAccess(_, _, Some(field_location)) = &field_expr.kind
    else {
        panic!("expected field access location");
    };
    let crate::hir::HirStmtFor::Expr(variant_expr) = &method.body.stmts[1] else {
        panic!("expected enum variant expression statement");
    };
    let crate::hir::HirExprKindFor::EnumVariant(_, _, _, Some(variant_location)) =
        &variant_expr.kind
    else {
        panic!("expected enum variant location");
    };

    assert_eq!(field_location.owner, expected_struct_owner);
    assert_eq!(variant_location.owner, expected_enum_owner);
}

#[test]
#[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
fn compiler_products_reject_call_targets_with_invalid_current_crate_ids() {
    let function_id = DefId::new(CrateId(0), LocalDefId(0));
    let invalid_target = DefId::new(CrateId(u32::MAX), LocalDefId(99));
    let mut function = test_function(function_id, "caller", Vec::new());
    function.body = HirBlock {
        stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
            kind: crate::hir::HirExprKindFor::Call(
                Box::new(HirExpr {
                    kind: crate::hir::HirExprKindFor::Var("target".to_string()),
                    ty: Type::function(Vec::new(), Type::I64),
                    span: Span::test(),
                }),
                Vec::new(),
                Some(HirCallTarget::Function(invalid_target)),
            ),
            ty: Type::I64,
            span: Span::test(),
        })],
        ty: Type::I64,
    };
    let hir = ResolvedHirProgram::new(
        HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::from([(function_id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("caller".to_string(), function_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([(function_id, "caller".to_string())]),
        ),
        ResolverTables::default(),
        BTreeSet::from([function_id]),
        CrateId(0),
        local_def_ids_after(1),
    );

    CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
}
