use super::*;

#[test]
fn hir_program_owns_functions_by_def_id_and_names_are_views() {
    let canonical_id = def_id(10);
    let canonical_names = HashMap::from([(canonical_id, "main".to_string())]);

    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::from([(canonical_id, test_function(canonical_id, "main"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([
                ("main".to_string(), canonical_id),
                ("alias_main".to_string(), canonical_id),
            ]),
            ..HirNameTables::default()
        },
        crate::hir::HirLanguageItems::default(),
        &canonical_names,
    );

    assert_eq!(program.functions.len(), 1);
    assert!(program.functions.contains_key(&canonical_id));
    assert_eq!(program.names.functions_by_name["main"], canonical_id);
    assert_eq!(program.names.functions_by_name["alias_main"], canonical_id);
    assert_eq!(
        program.names.functions_by_name.get("alias_main"),
        Some(&canonical_id)
    );
}

#[test]
fn hir_program_can_be_built_from_id_keyed_declarations() {
    let function_id = def_id(11);
    let function = test_function(function_id, "main");
    let function_names = HashMap::from([
        ("main".to_string(), function_id),
        ("alias_main".to_string(), function_id),
    ]);

    let program = HirProgram::from_id_parts_with_names(
        HashMap::from([(function_id, function)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        crate::hir::HirNameTables {
            functions_by_name: function_names,
            ..crate::hir::HirNameTables::default()
        },
        crate::hir::HirLanguageItems::default(),
    );

    assert_eq!(program.functions.len(), 1);
    assert!(program.functions.contains_key(&function_id));
    assert_eq!(program.names.functions_by_name["alias_main"], function_id);
}

#[test]
fn hir_program_lists_display_aliases_by_def_id() {
    let function_id = def_id(40);
    let struct_id = def_id(41);
    let enum_id = def_id(42);
    let canonical_names = HashMap::from([
        (function_id, "main".to_string()),
        (struct_id, "module::Foo".to_string()),
        (enum_id, "module::Choice".to_string()),
    ]);

    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::from([(function_id, test_function(function_id, "main"))]),
        HashMap::from([(struct_id, empty_struct(struct_id, "Foo"))]),
        HashMap::from([(enum_id, empty_enum(enum_id, "Choice"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([
                ("main".to_string(), function_id),
                ("alias_main".to_string(), function_id),
            ]),
            structs_by_name: HashMap::from([
                ("module::Foo".to_string(), struct_id),
                ("AliasFoo".to_string(), struct_id),
            ]),
            enums_by_name: HashMap::from([
                ("module::Choice".to_string(), enum_id),
                ("AliasChoice".to_string(), enum_id),
            ]),
            ..HirNameTables::default()
        },
        crate::hir::HirLanguageItems::default(),
        &canonical_names,
    );

    assert_eq!(
        program.function_display_aliases(function_id),
        vec!["main", "alias_main"]
    );
    assert_eq!(
        program.struct_display_aliases(struct_id),
        vec!["module::Foo", "AliasFoo"]
    );
    assert_eq!(
        program.enum_display_aliases(enum_id),
        vec!["module::Choice", "AliasChoice"]
    );
    assert_eq!(
        program.nominal_display_aliases(struct_id),
        vec!["module::Foo", "AliasFoo"]
    );
    assert_eq!(
        program.nominal_display_aliases(enum_id),
        vec!["module::Choice", "AliasChoice"]
    );
}

#[test]
fn hir_program_resolves_nominal_display_aliases_only_to_existing_owners() {
    let struct_id = def_id(43);
    let enum_id = def_id(44);
    let stale_id = def_id(99);
    let mut program = program(
        HashMap::new(),
        HashMap::from([(struct_id, empty_struct(struct_id, "Foo"))]),
        HashMap::from([(enum_id, empty_enum(enum_id, "Choice"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([("Foo".to_string(), struct_id)]),
            enums_by_name: HashMap::from([("Choice".to_string(), enum_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );
    program
        .names
        .structs_by_name
        .insert("AliasFoo".to_string(), struct_id);
    program
        .names
        .enums_by_name
        .insert("AliasChoice".to_string(), enum_id);
    program
        .names
        .structs_by_name
        .insert("Stale".to_string(), stale_id);

    assert_eq!(
        program.nominal_owner_id_for_display_alias("AliasFoo"),
        Some(struct_id)
    );
    assert_eq!(
        program.nominal_owner_id_for_display_alias("AliasChoice"),
        Some(enum_id)
    );
    assert_eq!(program.nominal_owner_id_for_display_alias("Stale"), None);
    assert!(program.nominal_display_aliases(stale_id).is_empty());
}

#[test]
fn hir_program_display_aliases_ignore_stale_name_table_ids() {
    let stale_id = def_id(100);
    let mut program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        HashMap::new(),
    );
    program
        .names
        .functions_by_name
        .insert("stale_function".to_string(), stale_id);
    program
        .names
        .structs_by_name
        .insert("stale_struct".to_string(), stale_id);
    program
        .names
        .enums_by_name
        .insert("stale_enum".to_string(), stale_id);

    assert!(program.function_display_aliases(stale_id).is_empty());
    assert!(program.struct_display_aliases(stale_id).is_empty());
    assert!(program.enum_display_aliases(stale_id).is_empty());
}

#[test]
fn hir_name_tables_do_not_own_semantic_bodies() {
    let id = def_id(30);
    let program = program(
        HashMap::from([(id, test_function(id, "answer"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("answer".to_string(), id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let by_name_id = program.names.functions_by_name["answer"];
    let by_name = &program.functions[&by_name_id];
    let (_, by_id) = program.function_by_id(id).unwrap();

    assert_eq!(by_name_id, id);
    assert!(std::ptr::eq(by_name, by_id));
}

#[test]
fn hir_program_keeps_one_function_payload_for_multiple_aliases() {
    let id = def_id(11);
    let program = program(
        HashMap::from([(id, test_function(id, "canonical"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([
                ("canonical".to_string(), id),
                ("alias".to_string(), id),
            ]),
            ..HirNameTables::default()
        },
        HashMap::from([(id, "canonical".to_string())]),
    );

    assert_eq!(program.functions.len(), 1);
    assert_eq!(program.names.functions_by_name["alias"], id);
}

#[test]
fn hir_program_indexes_trait_default_aliases_by_canonical_trait_name() {
    let trait_id = def_id(4);
    let method_id = def_id(8);
    let trait_def = HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Show".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("show".to_string(), empty_function(method_id, "show"))]),
        signatures: HashMap::new(),
    };
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(trait_id, trait_def)]),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([
                ("Alias".to_string(), trait_id),
                ("module::Show".to_string(), trait_id),
            ]),
            ..HirNameTables::default()
        },
        crate::hir::HirLanguageItems::default(),
        &HashMap::from([(trait_id, "module::Show".to_string())]),
    );

    assert_eq!(
        program.indexes.methods_by_id[&method_id],
        HirMethodLocation::TraitDefault {
            trait_id,
            method_id,
            method_name: "show".to_string(),
        }
    );
}

#[test]
fn hir_program_rebuild_indexes_with_canonical_names() {
    let mut program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::from([(def_id(1), empty_function(def_id(1), "canonical"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([
                ("canonical".to_string(), def_id(1)),
                ("alias".to_string(), def_id(1)),
            ]),
            ..HirNameTables::default()
        },
        crate::hir::HirLanguageItems::default(),
        &HashMap::from([(def_id(1), "canonical".to_string())]),
    );

    program
        .indexes
        .functions_by_id
        .insert(def_id(1), "alias".to_string());
    assert_eq!(program.indexes.functions_by_id[&def_id(1)], "alias");

    program.rebuild_indexes_with_canonical_names(&HashMap::from([(
        def_id(1),
        "canonical".to_string(),
    )]));

    assert_eq!(program.indexes.functions_by_id[&def_id(1)], "canonical");
}

#[test]
fn hir_program_uses_canonical_names_with_aliases() {
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::from([(def_id(1), empty_function(def_id(1), "canonical"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("alias".to_string(), def_id(1))]),
            ..HirNameTables::default()
        },
        crate::hir::HirLanguageItems::default(),
        &HashMap::from([(def_id(1), "canonical".to_string())]),
    );

    assert_eq!(program.indexes.functions_by_id[&def_id(1)], "canonical");
}

#[test]
fn hir_program_resolves_aliased_function_by_def_id() {
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::from([(def_id(1), empty_function(def_id(1), "dep::canonical"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("alias".to_string(), def_id(1))]),
            ..HirNameTables::default()
        },
        crate::hir::HirLanguageItems::default(),
        &HashMap::from([(def_id(1), "dep::canonical".to_string())]),
    );

    let (name, function) = program.function_by_id(def_id(1)).unwrap();

    assert_eq!(name, "dep::canonical");
    assert_eq!(function.id, def_id(1));
}

#[test]
fn hir_program_iterates_canonical_functions_once_per_def_id() {
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::from([
            (def_id(1), empty_function(def_id(1), "dep::canonical")),
            (def_id(2), empty_function(def_id(2), "other")),
        ]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("alias".to_string(), def_id(1))]),
            ..HirNameTables::default()
        },
        crate::hir::HirLanguageItems::default(),
        &HashMap::from([(def_id(1), "dep::canonical".to_string())]),
    );

    let mut functions = program.functions_by_id().collect::<Vec<_>>();
    functions.sort_by_key(|(_, name, _)| (*name).to_string());

    assert_eq!(functions.len(), 2);
    assert_eq!(functions[0].0, def_id(1));
    assert_eq!(functions[0].1, "dep::canonical");
    assert_eq!(functions[1].0, def_id(2));
    assert_eq!(functions[1].1, "other");
}

#[test]
fn hir_program_uses_table_candidates_when_canonical_name_is_for_another_kind() {
    let trait_def = HirTrait {
        target: None,
        predicates: Vec::new(),
        id: def_id(1),
        name: "Animal".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    };
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(def_id(1), trait_def)]),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([
                ("Animal".to_string(), def_id(1)),
                ("dep::Animal".to_string(), def_id(1)),
            ]),
            ..HirNameTables::default()
        },
        crate::hir::HirLanguageItems::default(),
        &HashMap::from([(def_id(1), "Dog".to_string())]),
    );

    assert_eq!(program.indexes.traits_by_id[&def_id(1)], "dep::Animal");
}
