use super::*;

fn resolved_hir_with_traits_in_order(names: &[&str]) -> ResolvedHirProgram {
    let mut traits = HashMap::new();

    for (index, name) in names.iter().enumerate() {
        let method_id = match *name {
            "Show" => DefId::new(CrateId(0), LocalDefId(100)),
            "Debug" => DefId::new(CrateId(0), LocalDefId(101)),
            _ => DefId::new(CrateId(0), LocalDefId(200 + index as u32)),
        };
        let mut methods = HashMap::new();
        methods.insert(
            "show".to_string(),
            test_function(method_id, "show", Vec::new()),
        );
        traits.insert(
            DefId::new(CrateId(0), LocalDefId(index as u32)),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: DefId::new(CrateId(0), LocalDefId(index as u32)),
                name: (*name).to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods,
                signatures: HashMap::new(),
            },
        );
    }

    ResolvedHirProgram::new(
        HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            traits,
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                traits_by_name: names
                    .iter()
                    .enumerate()
                    .map(|(index, name)| {
                        (
                            (*name).to_string(),
                            DefId::new(CrateId(0), LocalDefId(index as u32)),
                        )
                    })
                    .collect(),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    (
                        DefId::new(CrateId(0), LocalDefId(index as u32)),
                        (*name).to_string(),
                    )
                })
                .collect(),
        ),
        ResolverTables::default(),
        names
            .iter()
            .enumerate()
            .map(|(index, _)| DefId::new(CrateId(0), LocalDefId(index as u32)))
            .chain(names.iter().filter_map(|name| match *name {
                "Show" => Some(DefId::new(CrateId(0), LocalDefId(100))),
                "Debug" => Some(DefId::new(CrateId(0), LocalDefId(101))),
                _ => None,
            }))
            .collect(),
        CrateId(0),
        IdGen::new(),
    )
}

#[test]
fn compiler_products_keep_empty_trait_default_bodies_by_product_def_id() {
    let mut hir = resolved_hir_for_products();
    let empty_method_id = DefId::new(CrateId(0), LocalDefId(10));
    hir.current_def_ids.insert(empty_method_id);
    let empty_method = HirFunction {
        body: empty_body(Type::Unit),
        ret_type: Type::Unit,
        ..test_function(empty_method_id, "noop", Vec::new())
    };
    let effective_method_id = DefId::new(CrateId(0), LocalDefId(11));
    hir.current_def_ids.insert(effective_method_id);
    rebuild_program(
        &mut hir.program,
        |_, _, _, traits, impls, _, _, effective_methods| {
            traits
                .get_mut(&DefId::new(CrateId(0), LocalDefId(4)))
                .unwrap()
                .methods
                .insert("noop".to_string(), accept_function(empty_method.clone()));
            let mut effective_method = empty_method;
            effective_method.id = effective_method_id;
            impls
                .get_mut(&DefId::new(CrateId(0), LocalDefId(5)))
                .unwrap()
                .methods
                .insert("noop".to_string(), accept_function(effective_method));
            effective_methods.insert(
                (DefId::new(CrateId(0), LocalDefId(5)), empty_method_id),
                effective_method_id,
            );
        },
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

    let empty_method_id = ProductDefId::from(empty_method_id);

    assert!(products
        .bodies
        .trait_default_methods
        .contains_key(&empty_method_id));
}

#[test]
fn compiler_products_preserve_trait_defaults_with_distinct_method_ids() {
    let mut hir = resolved_hir_for_products();
    let trait_id = DefId::new(CrateId(0), LocalDefId(8));
    let method_id = DefId::new(CrateId(0), LocalDefId(9));
    hir.current_def_ids.insert(trait_id);
    hir.current_def_ids.insert(method_id);
    let mut methods = HashMap::new();
    methods.insert(
        "show".to_string(),
        accept_function(test_function(method_id, "show", Vec::new())),
    );
    rebuild_program(&mut hir.program, |_, _, _, traits, _, _, _, _| {
        traits.insert(
            trait_id,
            HirTraitFor::<AcceptedHir> {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Debug".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods,
                signatures: HashMap::new(),
            },
        );
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

    assert_eq!(products.bodies.trait_default_methods.len(), 2);
}

#[test]
fn compiler_products_share_signature_and_default_member_identity() {
    let mut hir = resolved_hir_for_products();
    let trait_id = DefId::new(CrateId(0), LocalDefId(4));
    let member_id = DefId::new(CrateId(0), LocalDefId(7));
    let impl_id = DefId::new(CrateId(0), LocalDefId(5));
    let effective_id = DefId::new(CrateId(0), LocalDefId(8));
    rebuild_program(&mut hir.program, |_, _, _, traits, _, _, _, _| {
        let trait_def = traits.get_mut(&trait_id).unwrap();
        let method = &trait_def.methods["show"];
        trait_def.signatures.insert(
            "show".to_string(),
            HirFunctionSig {
                id: member_id,
                name: "show".to_string(),
                generic_params: method.generic_params.clone(),
                params: method.params.iter().map(|param| param.ty.clone()).collect(),
                ret: method.ret_type.clone(),
                generic_bounds: method.generic_bounds.clone(),
                self_receiver: method.self_receiver,
                is_unsafe: method.is_unsafe,
            },
        );
    });
    let (products, remap) = CompilerProducts::from_resolved_hir_with_remap(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let trait_def = &products.interface.traits[&ProductDefId::from(trait_id)];
    assert_eq!(
        trait_def.methods["show"].id,
        trait_def.signatures["show"].id
    );
    assert_eq!(trait_def.methods["show"].id, member_id);
    assert_eq!(remap[&ProductDefId::from(member_id)].len(), 1);
    assert_eq!(
        products.interface.effective_trait_methods
            [&(ProductDefId::from(impl_id), ProductDefId::from(member_id))],
        ProductDefId::from(effective_id),
    );
    assert!(products
        .bodies
        .trait_default_methods
        .contains_key(&ProductDefId::from(member_id)));
}

#[test]
fn compiler_products_record_trait_default_method_ids_in_metadata() {
    let mut hir = resolved_hir_for_products();
    let trait_id = DefId::new(CrateId(0), LocalDefId(8));
    let method_id = DefId::new(CrateId(0), LocalDefId(0));
    hir.current_def_ids.insert(trait_id);
    let mut methods = HashMap::new();
    methods.insert(
        "show".to_string(),
        accept_function(test_function(method_id, "show", Vec::new())),
    );
    rebuild_program(&mut hir.program, |_, _, _, traits, _, _, _, _| {
        traits.insert(
            trait_id,
            HirTraitFor::<AcceptedHir> {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Debug".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods,
                signatures: HashMap::new(),
            },
        );
    });
    hir.local_def_ids = local_def_ids_after(9);
    let next_raw = hir.local_def_ids.next_raw();

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let debug_trait_id = ProductDefId::from(trait_id);
    let interface_method_id = products.interface.traits[&debug_trait_id].methods["show"].id;
    let debug_method_id = ProductDefId::from(interface_method_id);

    assert_ne!(debug_method_id, ProductDefId::from(method_id));
    assert!(debug_method_id.local_id.0 >= next_raw);

    assert_eq!(
        products.bodies.trait_default_methods[&debug_method_id].id,
        interface_method_id
    );
    assert_eq!(
        products.interface.traits[&debug_trait_id].methods["show"].id,
        interface_method_id
    );
}

#[test]
fn compiler_products_prefer_exact_trait_default_method_identity() {
    let shared_id = DefId::new(CrateId(0), LocalDefId(7));
    let exact_method = test_function(shared_id, "right", Vec::new());
    let accepted = crate::hir::AcceptedHirProgram::try_from(
        HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::from([(shared_id, exact_method)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("right".to_string(), shared_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([(shared_id, "right".to_string())]),
        ),
    )
    .expect("fixture HIR is accepted");
    let exact_method = accepted.function_by_id(shared_id).unwrap().1.clone();
    let preferred = super::PreferredTraitDefaultMethods {
        by_id: HashMap::from([(shared_id, exact_method)]),
    };

    let selected = preferred.get(shared_id, "Debug", "show");

    assert_eq!(selected.name, "right");
}

#[test]
fn compiler_products_do_not_export_trait_default_methods() {
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

    assert!(products
        .identity_table
        .display_names
        .values()
        .any(|name| name == "Show::show"));
    assert!(!products
        .identity_table
        .export_names
        .contains_key("Show::show"));
}

#[test]
fn compiler_products_keep_trait_default_ids_stable_across_trait_order() {
    let first = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved_hir_with_traits_in_order(&["Show", "Debug"]),
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let second = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved_hir_with_traits_in_order(&["Debug", "Show"]),
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    for display_name in ["Debug::show", "Show::show"] {
        let first_id = first
            .identity_table
            .display_names
            .iter()
            .find_map(|(id, name)| (name == display_name).then_some(*id));
        let second_id = second
            .identity_table
            .display_names
            .iter()
            .find_map(|(id, name)| (name == display_name).then_some(*id));

        assert_eq!(first_id, second_id, "{display_name}");
    }
}
