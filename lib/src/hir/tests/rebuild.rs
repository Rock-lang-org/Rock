use super::*;

#[test]
fn hir_program_rebuild_indexes_uses_owned_functions_when_name_table_is_stale() {
    let function_id = def_id(12);
    let stale_id = def_id(99);
    let mut program = program(
        HashMap::from([(function_id, test_function(function_id, "main"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("main".to_string(), function_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    program.names.functions_by_name.clear();
    program
        .names
        .functions_by_name
        .insert("stale".to_string(), stale_id);

    program.rebuild_indexes();

    assert_eq!(program.indexes.functions_by_id[&function_id], "main");
    assert!(program.function_by_id(function_id).is_some());
    assert_eq!(program.functions_by_id().count(), 1);
    assert!(!program.indexes.functions_by_id.contains_key(&stale_id));
}

#[test]
fn hir_program_rebuild_indexes_uses_owned_traits_for_default_methods_without_name_table() {
    let trait_id = def_id(14);
    let method_id = def_id(15);
    let mut program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            trait_id,
            trait_with_default_method(trait_id, "Show", method_id),
        )]),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([("Show".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    program.names.traits_by_name.clear();
    program.rebuild_indexes();

    assert_eq!(
        program.indexes.methods_by_id[&method_id],
        HirMethodLocation::TraitDefault {
            trait_id,
            method_id,
            method_name: "default_method".to_string(),
        }
    );
}

#[test]
fn hir_program_rebuild_indexes_ignores_stale_trait_names_for_default_methods() {
    let trait_id = def_id(16);
    let method_id = def_id(17);
    let stale_id = def_id(99);
    let mut program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            trait_id,
            trait_with_default_method(trait_id, "Show", method_id),
        )]),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([("Show".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    program.names.traits_by_name.clear();
    program
        .names
        .traits_by_name
        .insert("stale".to_string(), stale_id);
    program.rebuild_indexes();

    assert!(program.indexes.methods_by_id.contains_key(&method_id));
    assert!(!program.indexes.traits_by_id.contains_key(&stale_id));
}

#[test]
fn hir_program_rebuild_indexes_uses_owned_impls_when_order_is_stale() {
    let impl_id = def_id(18);
    let method_id = def_id(19);
    let stale_id = def_id(99);
    let mut program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(impl_id, impl_with_method(impl_id, method_id))]),
        HashMap::new(),
        HirNameTables::default(),
        HashMap::new(),
    );

    program.order.impls = vec![stale_id];
    program.rebuild_indexes();

    assert_eq!(program.indexes.impls_by_id[&impl_id], 0);
    assert!(program.impl_by_id(impl_id).is_some());
    assert!(!program.indexes.impls_by_id.contains_key(&stale_id));
    assert_eq!(
        program.indexes.methods_by_id[&method_id],
        HirMethodLocation::ImplMethod {
            impl_id,
            method_id,
            method_name: "impl_method".to_string(),
        }
    );
}

#[test]
fn hir_program_impls_in_order_uses_owned_impls_when_order_is_stale() {
    let preferred_id = def_id(18);
    let missing_id = def_id(19);
    let stale_id = def_id(99);
    let mut program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([
            (missing_id, impl_with_method(missing_id, def_id(20))),
            (preferred_id, impl_with_method(preferred_id, def_id(21))),
        ]),
        HashMap::new(),
        HirNameTables::default(),
        HashMap::new(),
    );

    program.order.impls = vec![preferred_id, stale_id];
    let ids = program
        .impls_in_order()
        .map(|(id, _)| id)
        .collect::<Vec<_>>();

    assert_eq!(ids, vec![preferred_id, missing_id]);
}

#[test]
fn hir_program_rebuild_indexes_uses_owned_externs_when_order_is_stale() {
    let extern_id = def_id(20);
    let stale_id = def_id(99);
    let mut program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            extern_id,
            HirExtern {
                id: extern_id,
                name: "puts".to_string(),
                params: vec![Type::Str],
                ret: Type::I32,
                variadic: false,
                is_unsafe: false,
            },
        )]),
        HirNameTables {
            externs_by_name: HashMap::from([("puts".to_string(), extern_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    program.order.externs = vec![stale_id];
    program.rebuild_indexes();

    assert_eq!(program.indexes.externs_by_id[&extern_id], 0);
    assert!(program.extern_by_id(extern_id).is_some());
    assert!(!program.indexes.externs_by_id.contains_key(&stale_id));
}

#[test]
fn hir_program_externs_in_order_uses_owned_externs_when_order_is_stale() {
    let preferred_id = def_id(20);
    let missing_id = def_id(21);
    let stale_id = def_id(99);
    let mut program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([
            (
                missing_id,
                HirExtern {
                    id: missing_id,
                    name: "missing".to_string(),
                    params: vec![Type::Str],
                    ret: Type::I32,
                    variadic: false,
                    is_unsafe: false,
                },
            ),
            (
                preferred_id,
                HirExtern {
                    id: preferred_id,
                    name: "preferred".to_string(),
                    params: vec![Type::Str],
                    ret: Type::I32,
                    variadic: false,
                    is_unsafe: false,
                },
            ),
        ]),
        HirNameTables {
            externs_by_name: HashMap::from([
                ("missing".to_string(), missing_id),
                ("preferred".to_string(), preferred_id),
            ]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    program.order.externs = vec![preferred_id, stale_id];
    let ids = program
        .externs_in_order()
        .map(|(id, _)| id)
        .collect::<Vec<_>>();

    assert_eq!(ids, vec![preferred_id, missing_id]);
}

#[test]
fn hir_program_rebuild_indexes() {
    let mut program = program(
        HashMap::from([
            (def_id(1), empty_function(def_id(1), "main")),
            (def_id(2), empty_function(def_id(2), "helper")),
        ]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([
                ("main".to_string(), def_id(1)),
                ("helper".to_string(), def_id(2)),
            ]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    program.indexes.functions_by_id.remove(&def_id(2));
    assert!(!program.indexes.functions_by_id.contains_key(&def_id(2)));

    program.rebuild_indexes();

    assert_eq!(program.indexes.functions_by_id[&def_id(2)], "helper");
}
