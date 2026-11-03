use super::*;

#[test]
fn std_index_selection_persists_exact_impl_and_trait_authority() {
    let temp_dir = temp_test_dir("std_index_authority");
    let entry_file = temp_dir.join("main.rk");
    fs::write(
        &entry_file,
        r#"
to_slice: &[I64] -> &[I64]
to_slice = slice -> slice

struct Probe T
impl Probe T
    @probe = ->
        mut arr: [I64; 3] = [1, 2, 3]
        first = arr[0]
        slice = to_slice &arr
        second = slice[1]
        ptr: *I64 = (~ArrPtr slice) as *I64
        third = unsafe ptr[2]
        arr[0] = 9
        0

main = ->
    0
"#,
    )
    .unwrap();
    let artifact_path = shared_std_product_artifact();
    let output = crate::compile_with_products(&Config {
        entry_file,
        output_dir: temp_dir.join("out"),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![("std".to_string(), artifact_path)],
        source_providers: Vec::new(),
        current_crate_name: Some("app".to_string()),
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    })
    .expect("std-backed indexing should lower");
    let products = output.products.expect("products should be emitted");
    let std_products = CompilerProducts::read_artifact_from_path(&shared_std_product_artifact())
        .expect("std artifact should load for authority assertions");
    let probe = products
        .bodies
        .generic_impls
        .values()
        .flat_map(|imp| imp.methods.values())
        .find(|method| method.name == "probe")
        .expect("generic probe body should be present");
    let targets = collect_index_authorities_from_function(probe);
    let index = std_products
        .interface
        .language_items
        .index
        .as_ref()
        .expect("Index provider should be loaded");
    let index_mut = std_products
        .interface
        .language_items
        .index_mut
        .as_ref()
        .expect("IndexMut provider should be loaded");
    let remap_external = |crate_id: crate::ids::CrateId, id: ProductDefId| {
        DefId::new(crate_id, LocalDefId(id.local_id.0))
    };
    let remap_interface_def =
        |crate_id: crate::ids::CrateId, id: DefId| DefId::new(crate_id, id.local);

    let mut index_count = 0;
    let mut index_mut_count = 0;
    for target in targets {
        let expected_trait_id = match &target.target {
            HirSelectedMethodTarget::ImplMethod {
                method_id,
                selected_trait: Some(selected_trait),
                ..
            } => {
                assert_eq!(target.method_id(), Some(*method_id));
                selected_trait.trait_id
            }
            _ => panic!("index method must persist concrete impl authority"),
        };
        assert_eq!(target.trait_id(), Some(expected_trait_id));
        let HirSelectedMethodTarget::ImplMethod {
            impl_id,
            method_id,
            selected_trait: Some(selected_trait),
        } = target.target
        else {
            panic!("index method must persist concrete impl authority");
        };
        let external_crate_id = selected_trait.trait_id.crate_id;
        let impl_def = std_products
            .interface
            .impls
            .values()
            .find(|candidate| remap_interface_def(external_crate_id, candidate.id) == impl_id)
            .expect("selected impl must be present in the loaded product");
        assert_eq!(
            impl_def
                .trait_id
                .map(|id| remap_interface_def(external_crate_id, id)),
            Some(selected_trait.trait_id)
        );
        assert!(impl_def
            .methods
            .values()
            .any(|method| remap_interface_def(external_crate_id, method.id) == method_id));

        if selected_trait.trait_id == remap_external(external_crate_id, index.trait_id) {
            index_count += 1;
            assert_eq!(
                selected_trait.member_id,
                remap_external(external_crate_id, index.method_id)
            );
        } else if selected_trait.trait_id == remap_external(external_crate_id, index_mut.trait_id) {
            index_mut_count += 1;
            assert_eq!(
                selected_trait.member_id,
                remap_external(external_crate_id, index_mut.method_id)
            );
        } else {
            panic!("index authority selected an unrelated trait");
        }
    }

    assert_eq!((index_count, index_mut_count), (3, 1));
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn std_product_artifact_provides_index_mut_language_items() {
    let artifact_path = shared_std_product_artifact();
    let products = CompilerProducts::read_artifact_from_path(&artifact_path)
        .expect("shared std product artifact should load");
    let language_items = &products.interface.language_items;

    assert!(
        language_items.index.is_some(),
        "std artifact must provide Index"
    );
    assert!(
        language_items.index_mut.is_some(),
        "std artifact must provide IndexMut"
    );
}

#[test]
fn std_product_artifact_provides_complete_language_item_registry() {
    let artifact_path = shared_std_product_artifact();
    let products = CompilerProducts::read_artifact_from_path(&artifact_path)
        .expect("shared std product artifact should load");
    let product_sized = products
        .interface
        .language_items
        .sized
        .as_ref()
        .expect("std product must provide Sized");
    let product_drop = products
        .interface
        .language_items
        .drop
        .as_ref()
        .expect("std product must provide Drop");
    let product_index = products
        .interface
        .language_items
        .index
        .as_ref()
        .expect("std product must provide Index");
    let product_try = products
        .interface
        .language_items
        .try_protocol
        .as_ref()
        .expect("std product must provide Try");

    let mut crate_ctx = CrateContext::new();
    crate_ctx
        .load_product_artifact_from_path_as("std", artifact_path)
        .expect("std artifact should load into a fresh crate context");
    let runtime_std = crate_ctx
        .extern_crate("std")
        .expect("fresh context should contain std");
    let runtime_crate_id = runtime_std.crate_id();
    let runtime_language_items = runtime_std.metadata().language_items();
    let runtime_sized = runtime_language_items
        .sized
        .as_ref()
        .expect("loaded std must provide Sized");
    let runtime_drop = runtime_language_items
        .drop
        .as_ref()
        .expect("loaded std must provide Drop");
    let runtime_index = runtime_language_items
        .index
        .as_ref()
        .expect("loaded std must provide Index");
    let runtime_try = runtime_language_items
        .try_protocol
        .as_ref()
        .expect("loaded std must provide Try");
    let remapped = |id: ProductDefId| DefId::new(runtime_crate_id, LocalDefId(id.local_id.0));

    assert_eq!(runtime_sized.trait_id, remapped(product_sized.trait_id));
    assert_eq!(runtime_drop.trait_id, remapped(product_drop.trait_id));
    assert_eq!(runtime_drop.method_id, remapped(product_drop.method_id));
    assert_eq!(runtime_index.trait_id, remapped(product_index.trait_id));
    assert_eq!(runtime_index.method_id, remapped(product_index.method_id));
    assert_eq!(runtime_try.try_trait_id, remapped(product_try.try_trait_id));
    assert_eq!(
        runtime_try.branch_method_id,
        remapped(product_try.branch_method_id)
    );
    assert_eq!(
        runtime_try.from_residual_trait_id,
        remapped(product_try.from_residual_trait_id)
    );
    assert_eq!(
        runtime_try.from_residual_method_id,
        remapped(product_try.from_residual_method_id)
    );
    assert_eq!(
        runtime_try.control_flow_enum_id,
        remapped(product_try.control_flow_enum_id)
    );

    assert_eq!(runtime_index.output_id, product_index.output_id);
    assert_eq!(runtime_try.output_id, product_try.output_id);
    assert_eq!(runtime_try.residual_id, product_try.residual_id);
    assert_eq!(runtime_try.break_variant_id, product_try.break_variant_id);
    assert_eq!(
        runtime_try.continue_variant_id,
        product_try.continue_variant_id
    );

    let interface = runtime_std.metadata().interface();
    assert!(interface.traits.contains_key(&runtime_sized.trait_id));
    for (trait_id, assoc_type_ids, member_id) in [
        (runtime_drop.trait_id, &[][..], runtime_drop.method_id),
        (
            runtime_index.trait_id,
            std::slice::from_ref(&runtime_index.output_id),
            runtime_index.method_id,
        ),
        (
            runtime_try.try_trait_id,
            &[runtime_try.output_id, runtime_try.residual_id],
            runtime_try.branch_method_id,
        ),
        (
            runtime_try.from_residual_trait_id,
            &[][..],
            runtime_try.from_residual_method_id,
        ),
    ] {
        let trait_def = interface
            .traits
            .get(&trait_id)
            .expect("language-item trait must be declared by the loaded interface");
        assert!(assoc_type_ids.iter().all(|id| {
            trait_def
                .associated_types
                .iter()
                .any(|assoc| assoc.id == *id)
        }));
        assert!(
            trait_def
                .methods
                .values()
                .any(|method| method.id == member_id)
                || trait_def
                    .signatures
                    .values()
                    .any(|signature| signature.id == member_id)
        );
    }
    let control_flow = interface
        .enums
        .get(&runtime_try.control_flow_enum_id)
        .expect("ControlFlow must be declared by the loaded interface");
    assert!(control_flow
        .variants
        .iter()
        .any(|variant| variant.id == runtime_try.break_variant_id));
    assert!(control_flow
        .variants
        .iter()
        .any(|variant| variant.id == runtime_try.continue_variant_id));
}
