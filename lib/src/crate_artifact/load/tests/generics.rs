use super::*;

#[test]
fn product_artifact_rejects_receiver_pattern_missing_impl_generic() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let impl_id = product_def_id(0, 7);
    let imp = test_generic_impl(product_hir_def_id(impl_id), HashMap::new());
    insert_interface_impl(&mut products, impl_id, imp);
    products
        .interface
        .impls
        .get_mut(&impl_id)
        .unwrap()
        .receiver_pattern = crate::hir::HirImplReceiverPattern::Exact(Type::Unit);

    let error = super::validate_product_method_authority_maps(
        &products,
        &super::ProductDependencyDefinitions::default(),
    )
    .expect_err("receiver pattern must bind every impl generic");

    assert!(
        error.contains("receiver/trait argument generic bindings") && error.contains("0::7"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_allows_impl_generic_bound_by_trait_argument() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let impl_id = product_def_id(0, 8);
    let hir_impl_id = product_hir_def_id(impl_id);
    let mut imp = test_generic_impl(hir_impl_id, HashMap::new());
    imp.type_generics = generic_params(hir_impl_id, &["T", "U"]);
    imp.receiver_pattern =
        crate::hir::HirImplReceiverPattern::Exact(Type::Generic(crate::types::GenericParamId {
            owner: hir_impl_id,
            index: 0,
        }));
    imp.trait_arg_types = vec![Type::Generic(crate::types::GenericParamId {
        owner: hir_impl_id,
        index: 1,
    })];
    insert_interface_impl(&mut products, impl_id, imp);

    super::validate_product_method_authority_maps(
        &products,
        &super::ProductDependencyDefinitions::default(),
    )
    .expect("trait arguments may bind impl generics absent from the receiver");
}

#[test]
fn load_product_artifact_rejects_body_only_generic_impl_without_interface_declaration() {
    let (base, _cleanup) = temp_test_dir("body_only_generic_impl_without_interface");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let impl_id = product_def_id(0, 7);
    products.bodies.generic_impls.insert(
        impl_id,
        HirImpl {
            id: product_hir_def_id(impl_id),
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: generic_params(product_hir_def_id(impl_id), &["T"]),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        },
    );
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("body-only generic impl rows should be rejected");

    assert!(
        err.contains("generic impl body") && err.contains("interface declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_generic_impl_body_extra_method_without_interface_declaration() {
    let (base, _cleanup) = temp_test_dir("generic_impl_body_extra_method");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let impl_id = product_def_id(0, 7);
    let method_id = product_def_id(0, 8);
    let extra_method_id = product_def_id(0, 9);

    let mut interface_method = test_function(product_hir_def_id(method_id), "value");
    interface_method.is_method = true;
    let interface_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([("value".to_string(), interface_method.clone())]),
    );
    insert_interface_impl(&mut products, impl_id, interface_impl);

    let mut extra_method = test_function(product_hir_def_id(extra_method_id), "hidden");
    extra_method.is_method = true;
    let body_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([
            ("value".to_string(), interface_method),
            ("hidden".to_string(), extra_method),
        ]),
    );
    products.bodies.generic_impls.insert(impl_id, body_impl);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("extra generic impl body methods should be rejected");

    assert!(
        err.contains("generic impl body")
            && err.contains("method 'hidden'")
            && err.contains("interface declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_generic_impl_body_method_id_mismatch() {
    let (base, _cleanup) = temp_test_dir("generic_impl_body_method_id_mismatch");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let impl_id = product_def_id(0, 7);
    let method_id = product_def_id(0, 8);
    let mismatched_method_id = product_def_id(0, 9);

    let mut interface_method = test_function(product_hir_def_id(method_id), "value");
    interface_method.is_method = true;
    let interface_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([("value".to_string(), interface_method)]),
    );
    insert_interface_impl(&mut products, impl_id, interface_impl);

    let mut body_method = test_function(product_hir_def_id(mismatched_method_id), "value");
    body_method.is_method = true;
    let body_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([("value".to_string(), body_method)]),
    );
    products.bodies.generic_impls.insert(impl_id, body_impl);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("generic impl body method ID mismatches should be rejected");

    assert!(
        err.contains("generic impl body")
            && err.contains("method 'value'")
            && err.contains("interface declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_generic_impl_body_omitted_interface_method() {
    let (base, _cleanup) = temp_test_dir("generic_impl_body_omitted_interface_method");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let impl_id = product_def_id(0, 7);
    let method_id = product_def_id(0, 8);

    let mut interface_method = test_function(product_hir_def_id(method_id), "value");
    interface_method.is_method = true;
    let interface_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([("value".to_string(), interface_method)]),
    );
    insert_interface_impl(&mut products, impl_id, interface_impl);

    let body_impl = test_generic_impl(product_hir_def_id(impl_id), HashMap::new());
    products.bodies.generic_impls.insert(impl_id, body_impl);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("generic impl bodies must include every interface method body");

    assert!(
        err.contains("generic impl body")
            && err.contains("method 'value'")
            && err.contains("serialized body"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_generic_function_interface_without_body() {
    let (base, _cleanup) = temp_test_dir("generic_function_interface_without_body");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let function_id = product_def_id(0, 0);
    let function_def = product_hir_def_id(function_id);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let generic = crate::types::GenericParamId {
        owner: function_def,
        index: 0,
    };
    let function = products.interface.functions.get_mut(&function_id).unwrap();
    function
        .generic_params
        .push(GenericParamDecl::type_param(generic, "T"));
    products.bodies.functions.remove(&function_id);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("generic function interface declarations need serialized bodies");

    assert!(
        err.contains("generic function") && err.contains("serialized body"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_generic_impl_interface_without_body() {
    let (base, _cleanup) = temp_test_dir("generic_impl_interface_without_body");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let impl_id = product_def_id(0, 7);
    let method_id = product_def_id(0, 8);
    let mut method = test_function(product_hir_def_id(method_id), "value");
    method.is_method = true;
    let interface_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([("value".to_string(), method)]),
    );
    insert_interface_impl(&mut products, impl_id, interface_impl);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("generic impl interface declarations need serialized bodies");

    assert!(
        err.contains("generic impl") && err.contains("serialized body"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_invalid_generic_defaults() {
    let id = product_def_id(0, 7);
    let owner = product_hir_def_id(id);
    for default in [
        Type::Error,
        Type::TypeVar(crate::ids::TypeVarId(0)),
        Type::Generic(GenericParamId { owner, index: 0 }),
        Type::Generic(GenericParamId {
            owner: product_hir_def_id(product_def_id(0, 0)),
            index: 0,
        }),
        Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::I64),
        },
        Type::BoundVar {
            depth: 0,
            index: 0,
            kind: Kind::Type,
        },
    ] {
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let mut param = GenericParamDecl::type_param(GenericParamId { owner, index: 0 }, "T");
        param.default = Some(default);
        products.interface.structs.insert(
            id,
            ProductStructInterface {
                id: owner,
                name: "dep::Private".into(),
                generic_params: vec![param],
                fields: vec![],
            },
        );
        let remap = super::super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        assert!(
            super::validate_product_type_fields(&products, &remap, &CrateContext::new()).is_err()
        );
    }
}

#[test]
fn load_product_artifact_remaps_generic_body_ids_with_interface_ids() {
    let (base, _cleanup) = temp_test_dir("generic_body_ids");
    let artifact_path = write_product_fixture(&base, "dep");

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let interface_id = dep
        .metadata()
        .interface()
        .function_by_canonical_name("dep::answer")
        .expect("function interface should exist")
        .id;
    let body_id = dep
        .body_providers()
        .generic_function(interface_id)
        .expect("generic function body should exist")
        .id;

    assert_eq!(interface_id, body_id);
    assert_ne!(body_id.crate_id, CrateId(0));
}

#[test]
fn product_artifact_generic_param_descriptor_round_trip_remaps_owners() {
    let (base, _cleanup) = temp_test_dir("generic_param_owner_remap");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let function_def = DefId::new(CrateId(0), LocalDefId(0));
    let generic = crate::types::GenericParamId {
        owner: function_def,
        index: 0,
    };
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let mut function = products.bodies.functions[&function_id].clone();
    function.generic_params = vec![GenericParamDecl::type_param(generic, "T")];
    function.params = vec![HirParam {
        name: "value".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Generic(generic),
        mutable: false,
        is_ref: false,
    }];
    function.ret_type = Type::Generic(generic);
    function.body.ty = Type::Generic(generic);
    insert_function_body(&mut products, function_id, function);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let interface_function = dep
        .metadata()
        .interface()
        .function_by_canonical_name("dep::answer")
        .expect("function interface should exist");
    let expected_generic = GenericParamDecl::new(
        GenericParamId {
            owner: interface_function.id,
            index: 0,
        },
        "T",
        Kind::Type,
    );
    assert_eq!(
        interface_function.generic_params,
        vec![expected_generic.clone()]
    );
    let body_function = dep
        .body_providers()
        .generic_function(interface_function.id)
        .expect("generic function body should exist");
    assert_eq!(body_function.generic_params, vec![expected_generic]);
    let Type::Generic(param) = body_function.params[0].ty else {
        panic!("expected remapped generic param in artifact body");
    };

    assert_eq!(param.owner, interface_function.id);
    assert_ne!(param.owner.crate_id, CrateId(0));
}

#[test]
fn product_artifact_remaps_function_generic_bounds() {
    let (base, _cleanup) = temp_test_dir("function_generic_bounds_remap");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let trait_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(1),
    };
    let function_def = DefId::new(CrateId(0), LocalDefId(0));
    let trait_def = DefId::new(CrateId(0), LocalDefId(1));
    let generic = crate::types::GenericParamId {
        owner: function_def,
        index: 0,
    };
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products
        .identity_table
        .display_names
        .insert(trait_id, "dep::Bound".to_string());
    insert_interface_trait(
        &mut products,
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_def,
            name: "Bound".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let mut function = products.bodies.functions[&function_id].clone();
    function.generic_params = vec![GenericParamDecl::type_param(generic, "T")];
    function.params = vec![HirParam {
        name: "value".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Generic(generic),
        mutable: false,
        is_ref: false,
    }];
    function.generic_bounds.insert(
        generic,
        vec![crate::types::TraitBound {
            trait_id: trait_def,
            type_args: vec![Type::Generic(generic)],
        }],
    );
    insert_function_body(&mut products, function_id, function);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let interface_function = dep
        .metadata()
        .interface()
        .function_by_canonical_name("dep::answer")
        .expect("function interface should exist");
    let body_function = dep
        .body_providers()
        .generic_function(interface_function.id)
        .expect("generic function body should exist");
    let remapped_generic = crate::types::GenericParamId {
        owner: interface_function.id,
        index: 0,
    };
    let bounds = body_function
        .generic_bounds
        .get(&remapped_generic)
        .expect("generic bound key should be remapped to interface function ID");

    assert_eq!(bounds[0].trait_id.local, LocalDefId(1));
    assert_ne!(bounds[0].trait_id.crate_id, CrateId(0));
    assert_eq!(bounds[0].trait_id.crate_id, interface_function.id.crate_id);
    assert_eq!(bounds[0].type_args, vec![Type::Generic(remapped_generic)]);
}

#[test]
fn product_artifact_remaps_signature_generic_bounds() {
    let (base, _cleanup) = temp_test_dir("signature_generic_bounds_remap");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let trait_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(1),
    };
    let trait_def = DefId::new(CrateId(0), LocalDefId(1));
    let generic = crate::types::GenericParamId {
        owner: trait_def,
        index: 0,
    };
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products
        .identity_table
        .display_names
        .insert(trait_id, "dep::Bound".to_string());

    let mut signature_bounds = HashMap::new();
    signature_bounds.insert(
        generic,
        vec![crate::types::TraitBound {
            trait_id: trait_def,
            type_args: vec![Type::Generic(generic)],
        }],
    );
    insert_interface_trait(
        &mut products,
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_def,
            name: "Bound".to_string(),
            generic_params: generic_params(trait_def, &["T"]),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "value".to_string(),
                HirFunctionSig {
                    id: DefId::new(CrateId(0), LocalDefId(31)),
                    name: "value".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(generic, "T")],
                    params: vec![Type::Generic(generic)],
                    ret: Type::Generic(generic),
                    generic_bounds: signature_bounds.into(),
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        },
    );
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let loaded_trait = dep
        .metadata()
        .interface()
        .trait_by_canonical_name("dep::Bound")
        .expect("trait interface should exist");
    let signature = &loaded_trait.signatures["value"];
    let remapped_generic = crate::types::GenericParamId {
        owner: loaded_trait.id,
        index: 0,
    };
    let bounds = signature
        .generic_bounds
        .get(&remapped_generic)
        .expect("signature generic bound key should be remapped to trait ID");

    assert_eq!(bounds[0].trait_id, loaded_trait.id);
    assert_eq!(bounds[0].type_args, vec![Type::Generic(remapped_generic)]);
}

#[test]
fn load_product_artifact_keeps_generic_body_without_display_metadata() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "generic_names"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let function_id = DefId::new(CrateId(1), LocalDefId(7));
    let product_id = ProductDefId::from(function_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    let mut identity = test_function(function_id, "identity");
    identity.generic_params = vec![GenericParamDecl::type_param(
        crate::types::GenericParamId {
            owner: function_id,
            index: 0,
        },
        "T",
    )];
    let mut interface = ProductInterface::default();
    interface
        .functions
        .insert(product_id, ProductFunctionInterface::from(&identity));
    let mut bodies = crate::products::ProductBodies::default();
    bodies.functions.insert(product_id, identity);
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies,
        link: ProductLinkData {
            object_path: Some(object_path.clone()),
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    assert!(dep.metadata().interface().canonical_names.is_empty());
    let loaded_id = *dep
        .metadata()
        .interface()
        .functions
        .keys()
        .next()
        .expect("function interface should survive without display metadata");
    assert!(dep.body_providers().generic_function(loaded_id).is_some());

    let _ = fs::remove_dir_all(&base);
}
