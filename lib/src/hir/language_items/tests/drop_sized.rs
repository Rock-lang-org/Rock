use super::*;

#[test]
fn language_item_validation_rejects_wrong_drop_receiver() {
    let mut hir = resolved_language_item_program(VALID_DROP_SOURCE);
    let program = hir.program.program_mut_for_test();
    let method_id = program
        .language_items
        .drop
        .as_ref()
        .expect("Drop bundle")
        .method_id;
    let method = program
        .traits
        .values_mut()
        .flat_map(|trait_def| trait_def.signatures.values_mut())
        .find(|signature| signature.id == method_id)
        .expect("marked Drop method signature");
    method.self_receiver = Some(ReceiverMode::Shared);

    assert!(validate_language_items(program)
        .iter()
        .any(|error| error == "drop.method must use a move receiver"));
}

#[test]
fn language_item_validation_rejects_cross_crate_drop_bundle_without_cascading() {
    let mut hir = resolved_language_item_program(VALID_DROP_SOURCE);
    let program = hir.program.program_mut_for_test();
    let method_id = program
        .language_items
        .drop
        .as_ref()
        .expect("Drop bundle")
        .method_id;
    let missing_root = crate::ids::DefId::new(crate::ids::CrateId(9), method_id.local);
    program
        .language_items
        .drop
        .as_mut()
        .expect("Drop bundle")
        .trait_id = missing_root;

    let errors = validate_language_items(program);
    assert!(errors
        .iter()
        .any(|error| error.contains("drop bundle field drop.method")));
    assert!(errors
        .iter()
        .any(|error| error.starts_with("drop.trait must identify a trait")));
    assert!(!errors
        .iter()
        .any(|error| error.starts_with("drop.method") && error.contains("owned by")));
}

#[test]
fn language_item_validation_rejects_sized_members_and_generics() {
    let mut hir = resolved_language_item_program(VALID_SIZED_SOURCE);
    let program = hir.program.program_mut_for_test();
    let trait_id = program
        .language_items
        .sized
        .as_ref()
        .expect("Sized bundle")
        .trait_id;
    let trait_def = program.traits.get_mut(&trait_id).expect("Sized trait");
    trait_def.generic_params.push(GenericParamDecl::type_param(
        GenericParamId {
            owner: trait_id,
            index: 0,
        },
        "T",
    ));
    trait_def
        .associated_types
        .push(crate::hir::HirAssociatedTypeDecl {
            id: crate::ids::AssocTypeId(0),
            name: "Extra".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        });
    trait_def.signatures.insert(
        "extra".to_string(),
        crate::hir::HirFunctionSig {
            id: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(999)),
            name: "extra".to_string(),
            generic_params: Vec::new(),
            params: Vec::new(),
            ret: Type::Unit,
            generic_bounds: Default::default(),
            self_receiver: None,
            is_unsafe: false,
        },
    );

    let errors = validate_language_items(program);
    assert!(errors
        .iter()
        .any(|error| error == "sized.trait must declare exactly 0 generic parameters; found 1"));
    assert!(errors
        .iter()
        .any(|error| error == "sized.trait must not declare associated types"));
    assert!(errors
        .iter()
        .any(|error| error == "sized.trait must not declare methods or signatures"));
}

#[test]
fn language_item_validation_rejects_drop_parameter_and_return_shapes() {
    let mut hir = resolved_language_item_program(VALID_DROP_SOURCE);
    let program = hir.program.program_mut_for_test();
    let method_id = program
        .language_items
        .drop
        .as_ref()
        .expect("Drop bundle")
        .method_id;
    let method = signature_mut(program, method_id);
    method.params.push(Type::I64);
    method.ret = Type::I64;

    let errors = validate_language_items(program);
    assert!(errors.iter().any(|error| {
        error == "drop.method must declare exactly 0 explicit parameters; found 1"
    }));
    assert!(errors
        .iter()
        .any(|error| error == "drop.method must return ()"));
}

#[test]
fn language_item_validation_allows_unmarked_drop_member() {
    let hir = resolved_language_item_program(
        r#"lang drop
< trait Release
    lang method
    ~@release: ()
    @extra: ()
"#,
    );

    assert!(validate_language_items(hir.program.program()).is_empty());
}

#[test]
fn language_item_validation_rejects_drop_generics() {
    let mut hir = resolved_language_item_program(VALID_DROP_SOURCE);
    let program = hir.program.program_mut_for_test();
    let trait_id = program
        .language_items
        .drop
        .as_ref()
        .expect("Drop bundle")
        .trait_id;
    program
        .traits
        .get_mut(&trait_id)
        .expect("Drop trait")
        .generic_params
        .push(GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "T",
        ));

    assert_eq!(
        validate_language_items(program),
        vec!["drop.trait must declare exactly 0 generic parameters; found 1".to_string()]
    );
}
