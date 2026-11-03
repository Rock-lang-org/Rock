use super::*;

fn function_with_generic_owner(name: &str, function_id: DefId, owner: DefId) -> HirFunction {
    let generic = GenericParamId { owner, index: 0 };

    HirFunction {
        id: function_id,
        name: name.to_string(),
        generic_params: vec![GenericParamDecl::type_param(generic, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(generic),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(generic),
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

#[test]
fn rehome_static_impl_type_generics_keeps_function_owned_name_collision() {
    let function_id = def_id(10);
    let impl_id = def_id(20);
    let function_generic = GenericParamId {
        owner: function_id,
        index: 0,
    };
    let mut func = HirFunction {
        id: function_id,
        name: "id".to_string(),
        generic_params: vec![GenericParamDecl::type_param(function_generic, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(function_generic),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(function_generic),
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Generic(function_generic),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };

    rehome_static_impl_type_generics(
        &mut func,
        &[GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        impl_id,
    );

    assert_eq!(
        func.generic_params,
        vec![GenericParamDecl::type_param(function_generic, "T")]
    );
    assert_eq!(func.params[0].ty, Type::Generic(function_generic));
    assert_eq!(func.ret_type, Type::Generic(function_generic));
}

#[test]
fn rehome_static_impl_type_generics_keeps_existing_impl_owned_id() {
    let function_id = def_id(10);
    let impl_id = def_id(20);
    let function_generic_u = GenericParamId {
        owner: function_id,
        index: 0,
    };
    let impl_generic_t = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let mut func = HirFunction {
        id: function_id,
        name: "pair".to_string(),
        generic_params: vec![
            GenericParamDecl::type_param(function_generic_u, "U"),
            GenericParamDecl::type_param(impl_generic_t, "T"),
        ],
        generic_bounds: HashMap::new().into(),
        params: vec![
            HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(function_generic_u),
                mutable: false,
                is_ref: false,
            },
            HirParam {
                name: "boxed".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(impl_generic_t),
                mutable: false,
                is_ref: false,
            },
        ],
        ret_type: Type::Generic(impl_generic_t),
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Generic(impl_generic_t),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };

    rehome_static_impl_type_generics(
        &mut func,
        &[GenericParamDecl::type_param(impl_generic_t, "T")],
        impl_id,
    );

    assert_eq!(func.params[1].ty, Type::Generic(impl_generic_t));
    assert_eq!(func.ret_type, Type::Generic(impl_generic_t));
    assert_eq!(func.body.ty, Type::Generic(impl_generic_t));
}

#[test]
fn sync_static_impl_generic_params_keeps_ids_and_names_cardinality_equal() {
    let function_id = def_id(10);
    let impl_id = def_id(20);
    let function_generic = GenericParamId {
        owner: function_id,
        index: 0,
    };
    let impl_generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let mut func = HirFunction {
        id: function_id,
        name: "pair".to_string(),
        generic_params: vec![GenericParamDecl::type_param(function_generic, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![
            HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(function_generic),
                mutable: false,
                is_ref: false,
            },
            HirParam {
                name: "boxed".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(impl_generic),
                mutable: false,
                is_ref: false,
            },
        ],
        ret_type: Type::Generic(function_generic),
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Generic(function_generic),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };

    sync_static_impl_generic_params_to_ids(
        &mut func,
        &[GenericParamDecl::type_param(impl_generic, "T")],
        impl_id,
    );

    assert_eq!(
        func.generic_params,
        vec![
            GenericParamDecl::type_param(function_generic, "T"),
            GenericParamDecl::type_param(impl_generic, "T"),
        ]
    );
}

#[test]
fn is_known_nominal_type_name_recognizes_dependency_alias_id() {
    let mut lowerer = Lowerer::new_for_test();
    let dep_struct_id = DefId::new(CrateId(2), LocalDefId(10));
    lowerer.items.insert_structure(HirStruct {
        id: dep_struct_id,
        name: "dep::Widget".to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    });
    let mut dep_resolver = crate::collect::resolver::ResolverTables::default();
    dep_resolver
        .import_aliases
        .insert("Widget".to_string(), dep_struct_id);
    dep_resolver
        .item_names_by_id
        .insert(dep_struct_id, "dep::Widget".to_string());
    lowerer
        .dependency_resolvers
        .insert("dep".to_string(), dep_resolver);

    assert!(is_known_nominal_type_name(&lowerer, "Widget"));
}

#[test]
fn lower_function_body_reports_provisional_generic_owner_without_mutation() {
    let mut lowerer = Lowerer::new_for_test();
    let function_id = def_id(10);
    let provisional_owner = DefId::new(CrateId(u32::MAX), LocalDefId(99));
    lowerer.items.insert_function(function_with_generic_owner(
        "id",
        function_id,
        provisional_owner,
    ));
    lowerer
        .resolver
        .item_paths
        .insert("id".to_string(), function_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(function_id, "id".to_string());

    lowerer.lower_function_body_qualified(&empty_function_decl("id"), function_id, None);

    let function = lowerer.items.function(function_id).unwrap();
    assert_eq!(function.body.ty, Type::Unit);
    assert_eq!(function.generic_params[0].id.owner, provisional_owner);
    assert!(!lowerer.diagnostics.is_empty());
}

#[test]
fn lower_function_body_reports_missing_indexed_function_payload() {
    let mut lowerer = Lowerer::new_for_test();

    lowerer.lower_function_body_qualified(&empty_function_decl("missing"), def_id(90), None);

    assert!(lowerer.errors().iter().any(|error| error
        .message
        .contains("missing indexed function declaration while lowering body")));
}
