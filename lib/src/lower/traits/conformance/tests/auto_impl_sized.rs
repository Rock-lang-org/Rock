use super::*;

#[test]
fn auto_impl_sized_ignores_local_trait_named_sized() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(10);
    let point_id = def_id(11);
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "Sized"));
    register_point(&mut lowerer, point_id);

    lowerer.auto_impl_sized();

    assert!(lowerer
        .items
        .impl_defs()
        .all(|(_, imp)| imp.trait_id != Some(sized_id)));
}

#[test]
fn auto_impl_sized_uses_marked_trait_id() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(20);
    let point_id = def_id(21);
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "Sized"));
    lowerer.language_items.sized =
        Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
    register_point(&mut lowerer, point_id);

    lowerer.auto_impl_sized();

    assert!(lowerer
        .items
        .impl_defs()
        .any(|(_, imp)| imp.type_name == "Point" && imp.trait_id == Some(sized_id)));
}

#[test]
fn auto_impl_sized_uses_marked_renamed_trait_id() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(22);
    let point_id = def_id(23);
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "StaticLayout"));
    lowerer.language_items.sized =
        Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
    register_point(&mut lowerer, point_id);

    lowerer.auto_impl_sized();

    let generated = lowerer
        .items
        .impl_defs()
        .find_map(|(_, imp)| (imp.type_name == "Point").then_some(imp))
        .expect("marked trait should receive automatic implementations");
    assert_eq!(generated.trait_id, Some(sized_id));
    assert_eq!(generated.trait_name.as_deref(), Some("StaticLayout"));
}

#[test]
fn auto_impl_sized_does_not_run_without_marked_bundle() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(24);
    let point_id = def_id(25);
    lowerer
        .items
        .insert_trait_def(empty_trait(trait_id, "StaticLayout"));
    register_point(&mut lowerer, point_id);

    lowerer.auto_impl_sized();

    assert!(lowerer.items.impl_defs().next().is_none());
}

#[test]
fn auto_impl_sized_preserves_generic_struct_shape() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(5);
    let struct_id = def_id(6);
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "Sized"));
    lowerer.language_items.sized =
        Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
    lowerer.items.insert_structure(HirStruct {
        id: struct_id,
        name: "Box".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: struct_id,
                index: 0,
            },
            "T",
        )],
        fields: vec![],
    });
    lowerer
        .resolver
        .item_paths
        .insert("Box".to_string(), struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "Box".to_string());
    lowerer.current_def_ids.insert(struct_id);

    lowerer.auto_impl_sized();

    let impl_def = lowerer
        .items
        .impl_defs()
        .find_map(|(_, imp)| (imp.type_name == "Box").then_some(imp))
        .expect("generic Box should receive Sized");
    assert_eq!(impl_def.type_generics[0].name, "T");
    assert_eq!(impl_def.type_generics[0].id.owner, impl_def.id);
    assert_eq!(
        impl_def.receiver_pattern,
        HirImplReceiverPattern::Exact(Type::Struct {
            id: struct_id,
            args: vec![Type::Generic(GenericParamId {
                owner: impl_def.id,
                index: 0,
            })],
        })
    );
}

#[test]
fn auto_impl_sized_preserves_generic_enum_shape() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(7);
    let enum_id = def_id(8);
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "Sized"));
    lowerer.language_items.sized =
        Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
    lowerer.items.insert_enumeration(HirEnum {
        id: enum_id,
        name: "Option".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: enum_id,
                index: 0,
            },
            "T",
        )],
        variants: vec![],
    });
    lowerer
        .resolver
        .item_paths
        .insert("Option".to_string(), enum_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(enum_id, "Option".to_string());
    lowerer.current_def_ids.insert(enum_id);

    lowerer.auto_impl_sized();

    let impl_def = lowerer
        .items
        .impl_defs()
        .find_map(|(_, imp)| (imp.type_name == "Option").then_some(imp))
        .expect("generic Option should receive Sized");
    assert_eq!(impl_def.type_generics[0].name, "T");
    assert_eq!(impl_def.type_generics[0].id.owner, impl_def.id);
    assert_eq!(
        impl_def.receiver_pattern,
        HirImplReceiverPattern::Exact(Type::Enum {
            id: enum_id,
            args: vec![Type::Generic(GenericParamId {
                owner: impl_def.id,
                index: 0,
            })],
        })
    );
}

#[test]
fn auto_impl_sized_generates_impls_in_owner_def_id_order() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(5);
    let high_id = def_id(30);
    let low_id = def_id(10);
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "Sized"));
    lowerer.language_items.sized =
        Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
    lowerer.items.insert_structure(HirStruct {
        id: high_id,
        name: "High".to_string(),
        generic_params: vec![],
        fields: vec![],
    });
    lowerer.items.insert_enumeration(HirEnum {
        id: low_id,
        name: "Low".to_string(),
        generic_params: vec![],
        variants: vec![],
    });
    for (id, name) in [(high_id, "High"), (low_id, "Low")] {
        lowerer.resolver.item_paths.insert(name.to_string(), id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(id, name.to_string());
        lowerer.current_def_ids.insert(id);
    }

    lowerer.auto_impl_sized();

    assert_eq!(
        lowerer
            .items
            .impl_defs_in_order()
            .map(|(_, imp)| imp.type_name.as_str())
            .collect::<Vec<_>>(),
        vec!["Low", "High"]
    );
}

#[test]
fn auto_impl_sized_skips_foreign_structs_without_current_provenance() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(20);
    let foreign_point_id = DefId::new(CrateId(7), LocalDefId(21));
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "Sized"));
    lowerer.language_items.sized =
        Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
    lowerer
        .items
        .insert_structure(point_struct(foreign_point_id));
    lowerer
        .resolver
        .item_paths
        .insert("dep::Point".to_string(), foreign_point_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(foreign_point_id, "dep::Point".to_string());

    lowerer.auto_impl_sized();

    assert!(lowerer
        .items
        .impl_defs()
        .all(|(_, imp)| !(imp.type_name == "dep::Point" && imp.trait_id == Some(sized_id))));
}
