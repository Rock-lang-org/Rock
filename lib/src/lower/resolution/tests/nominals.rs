use super::*;

fn test_struct(id: DefId, name: &str) -> HirStruct {
    HirStruct {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    }
}

fn test_enum(id: DefId, name: &str) -> HirEnum {
    HirEnum {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        variants: Vec::new(),
    }
}

fn test_trait(id: DefId, name: &str) -> HirTrait {
    HirTrait {
        target: None,
        predicates: Vec::new(),
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    }
}

#[test]
fn resolution_context_resolves_nominals_by_module_alias_first() {
    let root_id = def_id(0, 40);
    let module_id = def_id(0, 41);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(root_id, "Thing"));
    lowerer
        .items
        .insert_structure(test_struct(module_id, "demo::helper::Thing"));
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

    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution.resolve_struct_type("Thing").unwrap();

    assert_eq!(resolved.id, module_id);
}

#[test]
fn resolution_context_resolves_enum_and_trait_by_id() {
    let enum_id = def_id(0, 50);
    let trait_id = def_id(0, 51);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_enumeration(test_enum(enum_id, "demo::Choice"));
    lowerer
        .items
        .insert_trait_def(test_trait(trait_id, "demo::Show"));
    lowerer
        .resolver
        .item_paths
        .insert("Choice".to_string(), enum_id);
    lowerer
        .resolver
        .item_paths
        .insert("Show".to_string(), trait_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(enum_id, "demo::Choice".to_string());
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "demo::Show".to_string());

    let resolution = LowerResolutionContext::new(&lowerer);

    assert_eq!(resolution.resolve_enum_type("Choice").unwrap().id, enum_id);
    assert_eq!(resolution.resolve_trait_type("Show").unwrap().id, trait_id);
}

#[test]
fn resolution_context_requires_resolver_ids_for_nominal_types() {
    let struct_id = def_id(0, 52);
    let enum_id = def_id(0, 53);
    let trait_id = def_id(0, 54);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(struct_id, "Widget"));
    lowerer
        .items
        .insert_enumeration(test_enum(enum_id, "Choice"));
    lowerer.items.insert_trait_def(test_trait(trait_id, "Show"));

    let resolution = LowerResolutionContext::new(&lowerer);

    assert!(resolution.resolve_struct_type("Widget").is_none());
    assert!(resolution.resolve_enum_type("Choice").is_none());
    assert!(resolution.resolve_trait_type("Show").is_none());
}

#[test]
fn resolution_context_requires_canonical_names_for_nominal_types() {
    let struct_id = def_id(0, 55);
    let enum_id = def_id(0, 56);
    let trait_id = def_id(0, 57);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(struct_id, "pkg::Widget"));
    lowerer
        .items
        .insert_enumeration(test_enum(enum_id, "pkg::Choice"));
    lowerer
        .items
        .insert_trait_def(test_trait(trait_id, "pkg::Show"));
    lowerer
        .resolver
        .item_paths
        .insert("Widget".to_string(), struct_id);
    lowerer
        .resolver
        .item_paths
        .insert("Choice".to_string(), enum_id);
    lowerer
        .resolver
        .item_paths
        .insert("Show".to_string(), trait_id);

    let resolution = LowerResolutionContext::new(&lowerer);

    assert!(resolution.resolve_struct_type("Widget").is_none());
    assert!(resolution.resolve_enum_type("Choice").is_none());
    assert!(resolution.resolve_trait_type("Show").is_none());
}

#[test]
fn resolution_context_resolves_static_method_target_by_canonical_owner() {
    let struct_id = def_id(0, 74);
    let method_id = def_id(0, 75);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(struct_id, "String"));
    lowerer.resolver.insert_import_alias_with_name(
        "String".to_string(),
        "std::string::String".to_string(),
        struct_id,
    );
    let impl_id = def_id(0, 76);
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("std::string::String".to_string()),
            type_name: "std::string::String".to_string(),
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
            bounds: HashMap::new().into(),
            methods: HashMap::from([("from".to_string(), test_function(method_id, "from"))]),
        })
        .unwrap();
    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution
        .resolve_static_method_path(&["String".to_string(), "from".to_string()])
        .unwrap()
        .unwrap();

    assert_eq!(resolved.value.name, "std::string::String::from");
    assert_eq!(
        resolved.value.target,
        Some(HirVarTarget::Function(method_id))
    );
    assert_eq!(resolved.target.impl_id(), Some(impl_id));
    assert_eq!(resolved.target.method_id(), Some(method_id));
}

#[test]
fn resolution_context_rejects_receiver_method_without_explicit_receiver_mode_as_static() {
    let struct_id = def_id(0, 174);
    let method_id = def_id(0, 175);
    let impl_id = def_id(0, 176);
    let mut method = test_function(method_id, "value");
    method.is_method = true;
    method.self_receiver = None;
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(struct_id, "Box"));
    lowerer
        .resolver
        .item_paths
        .insert("Box".to_string(), struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "Box".to_string());
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
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
            bounds: HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        })
        .unwrap();

    assert!(LowerResolutionContext::new(&lowerer)
        .resolve_static_method_path(&["Box".to_string(), "value".to_string()])
        .unwrap()
        .is_none());
}

#[test]
fn resolution_context_resolves_struct_literal_target_by_module_alias() {
    let root_id = def_id(0, 76);
    let module_id = def_id(0, 77);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(root_id, "Thing"));
    lowerer
        .items
        .insert_structure(test_struct(module_id, "demo::helper::Thing"));
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

    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution.resolve_struct_literal_path(&["Thing".to_string()]);

    assert_eq!(resolved.name, "demo::helper::Thing");
    assert_eq!(resolved.structure.unwrap().id, module_id);
}

#[test]
fn resolution_context_resolves_owner_path_by_id() {
    let owner_id = def_id(0, 81);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .resolver
        .item_paths
        .insert("demo::Box".to_string(), owner_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(owner_id, "demo::Box".to_string());

    let resolution = LowerResolutionContext::new(&lowerer);

    assert_eq!(
        resolution.try_canonical_owner_path("demo::Box"),
        Some("demo::Box".to_string())
    );
}

#[test]
fn resolution_context_does_not_mark_import_alias_struct_unexported() {
    let string_id = def_id(0, 94);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(string_id, "std::string::String"));
    lowerer.resolver.insert_import_alias_with_name(
        "String".to_string(),
        "std::string::String".to_string(),
        string_id,
    );

    let resolution = LowerResolutionContext::new(&lowerer);

    assert!(!resolution.struct_segment_looks_unexported("String"));
}

#[test]
fn resolution_context_marks_qualified_only_struct_unexported() {
    let hidden_id = def_id(0, 95);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(hidden_id, "pkg::Hidden"));
    lowerer
        .resolver
        .item_names_by_id
        .insert(hidden_id, "pkg::Hidden".to_string());

    let resolution = LowerResolutionContext::new(&lowerer);

    assert!(resolution.struct_segment_looks_unexported("Hidden"));
}
