use super::*;

#[test]
fn test_collect_impls_keeps_object_backed_concrete_trait_impls_available_for_dispatch() {
    let mut mono = Monomorphizer::new();
    let concrete_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::BuiltinSlice,
        type_name: "&[U8]".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![Type::U8].into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(def_id(500)),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("println".to_string(), byte_slice_println_method())]),
    };
    mono.collect_impls(&[concrete_impl]);

    assert_eq!(mono.trait_impls[&def_id(500)].len(), 1);
}

#[test]
fn collect_impls_keeps_same_named_traits_separate_by_def_id() {
    let mut mono = Monomorphizer::new();
    let first_trait_id = def_id(610);
    let second_trait_id = def_id(611);
    let first_method = option_println_method(def_id(614), def_id(612));
    let second_method = option_println_method(def_id(615), def_id(613));

    mono.collect_impls(&[
        HirImpl {
            id: def_id(614),
            owner: HirImplOwner::Named("Option".to_string()),
            type_name: "Option".to_string(),
            type_generics: type_generic(def_id(614), "T"),
            receiver_pattern: vec![Type::Generic(GenericParamId {
                owner: def_id(614),
                index: 0,
            })]
            .into(),
            trait_name: Some("Show".to_string()),
            trait_id: Some(first_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("println".to_string(), first_method)]),
        },
        HirImpl {
            id: def_id(615),
            owner: HirImplOwner::Named("Option".to_string()),
            type_name: "Option".to_string(),
            type_generics: type_generic(def_id(615), "T"),
            receiver_pattern: vec![Type::Generic(GenericParamId {
                owner: def_id(615),
                index: 0,
            })]
            .into(),
            trait_name: Some("Show".to_string()),
            trait_id: Some(second_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("println".to_string(), second_method)]),
        },
    ]);

    assert_eq!(mono.trait_impls.len(), 2);
}
