use super::*;

fn resolved_hir_with_non_generic_impl_generic_method() -> ResolvedHirProgram {
    let struct_id = DefId::new(CrateId(0), LocalDefId(41));
    let impl_id = DefId::new(CrateId(0), LocalDefId(42));
    let method_id = DefId::new(CrateId(0), LocalDefId(43));
    let method_generic = crate::types::GenericParamId {
        owner: method_id,
        index: 0,
    };

    let method = HirFunction {
        id: method_id,
        name: "id".to_string(),
        generic_params: vec![GenericParamDecl::type_param(method_generic, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(method_generic),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(method_generic),
        body: HirBlock {
            stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
                kind: crate::hir::HirExprKindFor::Var("value".to_string()),
                ty: Type::Generic(method_generic),
                span: Span::test(),
            })],
            ty: Type::Generic(method_generic),
        },
        is_curried: false,
        is_method: true,
        self_receiver: None,
        is_unsafe: false,
    };

    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::from([(
            struct_id,
            HirStruct {
                id: struct_id,
                name: "Foo".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: crate::hir::HirImplOwner::Named("Foo".to_string()),
                type_name: "Foo".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("id".to_string(), method)]),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([("Foo".to_string(), struct_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([(struct_id, "Foo".to_string())]),
    );
    ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([struct_id, impl_id, method_id]),
        CrateId(0),
        local_def_ids_after(44),
    )
}

#[test]
fn compiler_products_keep_static_impl_methods_out_of_top_level_function_payloads() {
    let struct_id = DefId::new(CrateId(0), LocalDefId(100));
    let impl_id = DefId::new(CrateId(0), LocalDefId(101));
    let method_id = DefId::new(CrateId(0), LocalDefId(102));
    let mut method = test_function(method_id, "new", Vec::new());
    method.is_method = true;
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::from([(
            struct_id,
            HirStruct {
                id: struct_id,
                name: "Box".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("new".to_string(), method)]),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([("Box".to_string(), struct_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([(struct_id, "Box".to_string())]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([struct_id, impl_id, method_id]),
        CrateId(0),
        local_def_ids_after(103),
    );
    let method_product_id = ProductDefId::from(method_id);
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData {
            object_path: None,
            records: BTreeMap::from([(
                method_product_id,
                ProductLinkRecord {
                    backend_symbol: "box_new".to_string(),
                },
            )]),
        },
    )
    .expect("test HIR has valid product language items");

    assert!(!products
        .interface
        .functions
        .contains_key(&method_product_id));
    assert!(!products.bodies.functions.contains_key(&method_product_id));
    assert_eq!(
        products.interface.impls[&ProductDefId::from(impl_id)].methods["new"].id,
        method_id
    );
    assert_eq!(
        products.link.records[&method_product_id].backend_symbol,
        "box_new"
    );
}

#[test]
fn product_bodies_only_include_downstream_specialized_functions() {
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved_hir_for_products(),
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let plain_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let generic_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(1),
    };

    let plain_decl = products
        .interface
        .functions
        .get(&plain_id)
        .expect("plain function interface row");
    assert_eq!(plain_decl.id, DefId::new(CrateId(0), LocalDefId(0)));
    assert_eq!(plain_decl.name, "plain");
    assert_eq!(plain_decl.params, vec![Type::I64]);
    assert_eq!(plain_decl.ret_type, Type::I64);
    assert!(!products.bodies.functions.contains_key(&plain_id));

    assert!(products.interface.functions.contains_key(&generic_id));
    assert!(products.bodies.functions.contains_key(&generic_id));
}

#[test]
fn compiler_products_key_interface_by_product_def_id() {
    let hir = resolved_hir_for_products();

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));
    let extern_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(6)));

    assert!(products.interface.functions.contains_key(&plain_id));
    assert!(products.interface.externs.contains_key(&extern_id));
    assert_eq!(
        products.identity_table.export_names.get("plain"),
        Some(&plain_id)
    );
    assert_eq!(
        products.identity_table.display_names.get(&plain_id),
        Some(&"plain".to_string())
    );
}

#[test]
fn product_emission_reads_id_owned_hir_definitions() {
    let function_id = DefId::new(CrateId(0), LocalDefId(0));
    let struct_id = DefId::new(CrateId(0), LocalDefId(1));
    let function = test_function(function_id, "answer", Vec::new());
    let structure = HirStruct {
        id: struct_id,
        name: "Box".to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    };

    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::from([(function_id, function)]),
        HashMap::from([(struct_id, structure)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("answer".to_string(), function_id)]),
            structs_by_name: HashMap::from([("Box".to_string(), struct_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([
            (function_id, "answer".to_string()),
            (struct_id, "Box".to_string()),
        ]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([function_id, struct_id]),
        CrateId(0),
        IdGen::new(),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("test".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    assert!(products
        .interface
        .functions
        .contains_key(&ProductDefId::from(function_id)));
    assert!(products
        .interface
        .structs
        .contains_key(&ProductDefId::from(struct_id)));
}

#[test]
fn compiler_products_emit_named_interface_when_derived_indexes_are_stale() {
    let mut hir = resolved_hir_for_products();
    let plain_id = DefId::new(CrateId(0), LocalDefId(0));
    let struct_id = DefId::new(CrateId(0), LocalDefId(2));
    let enum_id = DefId::new(CrateId(0), LocalDefId(3));
    let trait_id = DefId::new(CrateId(0), LocalDefId(4));
    mutate_program(&mut hir.program, |program| {
        program.indexes.functions_by_id.remove(&plain_id);
        program.indexes.structs_by_id.remove(&struct_id);
        program.indexes.enums_by_id.remove(&enum_id);
        program.indexes.traits_by_id.remove(&trait_id);
    });

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let plain_id = ProductDefId::from(plain_id);
    let struct_id = ProductDefId::from(struct_id);
    let enum_id = ProductDefId::from(enum_id);
    let trait_id = ProductDefId::from(trait_id);
    assert_eq!(products.interface.functions[&plain_id].name, "plain");
    assert_eq!(products.interface.structs[&struct_id].name, "Box");
    assert_eq!(products.interface.enums[&enum_id].name, "Maybe");
    assert_eq!(products.interface.traits[&trait_id].name, "demo::Show");
    assert_eq!(products.identity_table.export_names["plain"], plain_id);
    assert_eq!(products.identity_table.export_names["Box"], struct_id);
    assert_eq!(products.identity_table.export_names["Maybe"], enum_id);
    assert_eq!(products.identity_table.export_names["Show"], trait_id);
}

#[test]
fn compiler_products_emit_alias_duplicate_function_once_by_def_id() {
    let mut hir = resolved_hir_for_products();
    let generic_id = DefId::new(CrateId(0), LocalDefId(1));
    rebuild_program(&mut hir.program, |_, _, _, _, _, _, names, _| {
        names
            .functions_by_name
            .insert("alias_identity".to_string(), generic_id);
    });
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let generic_product_id = ProductDefId::from(generic_id);
    assert_eq!(
        products
            .interface
            .functions
            .values()
            .filter(|function| function.id == generic_id)
            .count(),
        1
    );
    assert_eq!(
        products.identity_table.export_names.get("identity"),
        Some(&generic_product_id)
    );
    assert!(!products
        .identity_table
        .export_names
        .contains_key("alias_identity"));
}

#[test]
fn compiler_products_select_generic_bodies_by_product_def_id() {
    let hir = resolved_hir_for_products();

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));
    let generic_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(1)));
    let impl_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(5)));
    let default_method_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(7)));

    assert!(!products.bodies.functions.contains_key(&plain_id));
    assert!(products.bodies.functions.contains_key(&generic_id));
    assert!(products.bodies.generic_impls.contains_key(&impl_id));
    assert!(products
        .bodies
        .trait_default_methods
        .contains_key(&default_method_id));
}

#[test]
fn compiler_products_preserve_non_generic_impl_with_generic_method_body() {
    let hir = resolved_hir_with_non_generic_impl_generic_method();
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let impl_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(42)));
    assert!(products.bodies.generic_impls.contains_key(&impl_id));
}

#[test]
fn compiler_products_emit_only_current_crate_hir() {
    let mut hir = resolved_hir_for_products();
    let foreign_function_id = DefId::new(CrateId(0), LocalDefId(50));
    let foreign_impl_id = DefId::new(CrateId(0), LocalDefId(51));
    hir.current_def_ids = (0..8)
        .map(|local| DefId::new(CrateId(0), LocalDefId(local)))
        .collect();
    rebuild_program(&mut hir.program, |functions, _, _, _, impls, _, _, _| {
        functions.insert(
            foreign_function_id,
            accept_function(test_function(
                foreign_function_id,
                "std::identity",
                generic_params(foreign_function_id, &["T"]),
            )),
        );
        impls.insert(
            foreign_impl_id,
            HirImplFor::<AcceptedHir> {
                id: foreign_impl_id,
                owner: crate::hir::HirImplOwner::Named("std::Box".to_string()),
                type_name: "std::Box".to_string(),
                type_generics: generic_params(foreign_impl_id, &["T"]),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: DefId::new(CrateId(0), LocalDefId(2)),
                    args: vec![Type::Generic(crate::types::GenericParamId {
                        owner: foreign_impl_id,
                        index: 0,
                    })],
                }),
                trait_name: Some("std::Show".to_string()),
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            },
        );
    });

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        vec![ProductDependencyIdentity::artifact_object(
            "std".to_string(),
            PathBuf::from("build/std.rkca"),
        )],
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let foreign_function_id = ProductDefId::from(foreign_function_id);
    let foreign_impl_id = ProductDefId::from(foreign_impl_id);

    assert!(!products
        .interface
        .functions
        .contains_key(&foreign_function_id));
    assert!(!products.bodies.functions.contains_key(&foreign_function_id));
    assert!(!products.interface.impls.contains_key(&foreign_impl_id));
    assert!(!products.bodies.generic_impls.contains_key(&foreign_impl_id));
    assert_eq!(products.interface.impls.len(), 1);
    assert_eq!(products.bodies.generic_impls.len(), 1);
}

#[test]
fn compiler_products_do_not_treat_empty_current_ids_as_root_crate_ownership() {
    let mut hir = resolved_hir_for_products();
    let foreign_function_id = DefId::new(CrateId(0), LocalDefId(50));
    hir.current_def_ids = BTreeSet::new();
    rebuild_program(
        &mut hir.program,
        |functions, structs, enums, traits, impls, externs, _, effective_methods| {
            functions.clear();
            structs.clear();
            enums.clear();
            traits.clear();
            impls.clear();
            externs.clear();
            effective_methods.clear();
            functions.insert(
                foreign_function_id,
                accept_function(test_function(
                    foreign_function_id,
                    "std::identity",
                    generic_params(foreign_function_id, &["T"]),
                )),
            );
        },
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        vec![ProductDependencyIdentity::artifact_object(
            "std".to_string(),
            PathBuf::from("build/std.rkca"),
        )],
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    assert!(products.interface.functions.is_empty());
    assert!(products.bodies.functions.is_empty());
}
