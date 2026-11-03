use super::*;

const VALID_INDEX_MUT_SOURCE: &str = r#"lang index
< trait Lookup Key
    lang output
    type Value
    lang method
    @lookup: Key -> &Self::Value

lang index_mut
< trait MutLookup Key
    lang output
    type Value
    lang method
    ^@lookup_mut: Key -> &mut Self::Value
"#;

#[test]
fn language_item_validation_accepts_index_default_method_representation() {
    let mut hir = resolved_language_item_program(VALID_INDEX_SOURCE);
    let program = hir.program.program_mut_for_test();
    let items = program
        .language_items
        .index
        .as_ref()
        .expect("Index bundle")
        .clone();
    let key = Type::Generic(GenericParamId {
        owner: items.trait_id,
        index: 0,
    });
    let self_ty = Type::Generic(GenericParamId {
        owner: items.trait_id,
        index: 1,
    });
    let mut method = default_method_seed();
    method.id = items.method_id;
    method.self_receiver = Some(ReceiverMode::Shared);
    method.params[0].ty = Type::Reference {
        mutable: false,
        inner: Box::new(self_ty.clone()),
    };
    method.params.push(crate::hir::HirParam {
        name: "key".to_string(),
        local_id: crate::ids::HirLocalId(1),
        ty: key.clone(),
        mutable: false,
        is_ref: false,
    });
    method.ret_type = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Projection {
            ty: Box::new(self_ty),
            trait_id: items.trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: items.trait_id,
                assoc_type_id: items.output_id,
            },
            trait_args: vec![key],
        }),
    };
    let trait_def = program
        .traits
        .get_mut(&items.trait_id)
        .expect("Index trait");
    trait_def.signatures.clear();
    trait_def
        .methods
        .insert("default_index".to_string(), method);

    assert!(crate::hir::AcceptedHirProgram::revalidate_for_test(program.clone()).is_ok());
}

#[test]
fn language_item_validation_rejects_shared_index_mut_return() {
    let mut hir = resolved_language_item_program(VALID_INDEX_MUT_SOURCE);
    let program = hir.program.program_mut_for_test();
    let items = program
        .language_items
        .index_mut
        .as_ref()
        .expect("IndexMut bundle")
        .clone();
    let Type::Reference { mutable, .. } = &mut signature_mut(program, items.method_id).ret else {
        panic!("valid IndexMut return is a reference");
    };
    *mutable = false;

    assert_eq!(
        validate_language_items(program),
        vec![
            "index_mut.method must return a mutable reference to index_mut.output on Self"
                .to_string(),
        ],
    );
}

#[test]
fn language_item_validation_rejects_index_associated_type_from_another_trait() {
    let mut hir = resolved_language_item_program(
        r#"< trait Other
    type First
    type Second

lang index
< trait Lookup Key
    lang output
    type Value
    lang method
    @lookup: Key -> &Self::Value
"#,
    );
    let program = hir.program.program_mut_for_test();
    program
        .language_items
        .index
        .as_mut()
        .expect("Index bundle")
        .output_id = crate::ids::AssocTypeId(1);

    assert!(validate_language_items(program).iter().any(|error| {
        error.starts_with("index.output") && error.contains("is declared by trait")
    }));
}

#[test]
fn language_item_validation_rejects_index_generic_argument_and_projection_shapes() {
    let mut hir = resolved_language_item_program(VALID_INDEX_SOURCE);
    let program = hir.program.program_mut_for_test();
    let items = program
        .language_items
        .index
        .as_ref()
        .expect("Index bundle")
        .clone();
    program
        .traits
        .get_mut(&items.trait_id)
        .expect("Index trait")
        .generic_params
        .push(GenericParamDecl::type_param(
            GenericParamId {
                owner: items.trait_id,
                index: 1,
            },
            "Other",
        ));
    let method = signature_mut(program, items.method_id);
    method.params[1] = Type::Generic(GenericParamId {
        owner: items.trait_id,
        index: 1,
    });
    method.ret = Type::Unit;

    let errors = validate_language_items(program);
    assert!(errors
        .iter()
        .any(|error| error == "index.trait must declare exactly 1 generic parameters; found 2"));
    assert!(errors
        .iter()
        .any(|error| error == "index.method explicit parameter 0 has the wrong type"));
    assert!(errors.iter().any(|error| {
        error == "index.method must return a shared reference to index.output on Self"
    }));
}

macro_rules! index_shape_case {
    ($name:ident, $mutate:expr, $expected:expr) => {
        #[test]
        fn $name() {
            let mut hir = resolved_language_item_program(VALID_INDEX_SOURCE);
            let program = hir.program.program_mut_for_test();
            let items = program
                .language_items
                .index
                .as_ref()
                .expect("Index bundle")
                .clone();
            ($mutate)(program, &items);
            assert_eq!(
                validate_language_items(program),
                vec![$expected.to_string()]
            );
        }
    };
}

index_shape_case!(
    language_item_validation_rejects_index_non_shared_receiver,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &IndexLanguageItems<DefId>| {
        signature_mut(program, items.method_id).self_receiver = Some(ReceiverMode::Move);
    },
    "index.method must use a shared receiver"
);

index_shape_case!(
    language_item_validation_rejects_index_argument_wrong_owner,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &IndexLanguageItems<DefId>| {
        signature_mut(program, items.method_id).params[1] = Type::Generic(GenericParamId {
            owner: DefId::new(crate::ids::CrateId(44), crate::ids::LocalDefId(0)),
            index: 0,
        });
    },
    "index.method explicit parameter 0 has the wrong type"
);

index_shape_case!(
    language_item_validation_rejects_index_argument_wrong_index,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &IndexLanguageItems<DefId>| {
        signature_mut(program, items.method_id).params[1] = Type::Generic(GenericParamId {
            owner: items.trait_id,
            index: 1,
        });
    },
    "index.method explicit parameter 0 has the wrong type"
);

index_shape_case!(
    language_item_validation_rejects_index_receiver_not_shared_self,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &IndexLanguageItems<DefId>| {
        signature_mut(program, items.method_id).params[0] = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Generic(GenericParamId {
                owner: items.trait_id,
                index: 0,
            })),
        };
    },
    "index.method must use the trait Self receiver type"
);

index_shape_case!(
    language_item_validation_rejects_index_projection_trait_id,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &IndexLanguageItems<DefId>| {
        let Type::Reference { inner, .. } = &mut signature_mut(program, items.method_id).ret else {
            panic!("valid Index return is a reference");
        };
        let Type::Projection { trait_id, .. } = inner.as_mut() else {
            panic!("valid Index return is a projection");
        };
        *trait_id = DefId::new(crate::ids::CrateId(44), crate::ids::LocalDefId(1));
    },
    "index.method return projection must use index.trait"
);

index_shape_case!(
    language_item_validation_rejects_index_projection_associated_type,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &IndexLanguageItems<DefId>| {
        let Type::Reference { inner, .. } = &mut signature_mut(program, items.method_id).ret else {
            panic!("valid Index return is a reference");
        };
        let Type::Projection { assoc_type, .. } = inner.as_mut() else {
            panic!("valid Index return is a projection");
        };
        assoc_type.assoc_type_id = crate::ids::AssocTypeId(99);
    },
    "index.method return projection must use index.output"
);

index_shape_case!(
    language_item_validation_rejects_index_projection_non_self_base,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &IndexLanguageItems<DefId>| {
        let Type::Reference { inner, .. } = &mut signature_mut(program, items.method_id).ret else {
            panic!("valid Index return is a reference");
        };
        let Type::Projection { ty, .. } = inner.as_mut() else {
            panic!("valid Index return is a projection");
        };
        **ty = Type::Generic(GenericParamId {
            owner: items.trait_id,
            index: 0,
        });
    },
    "index.method return projection must project from index Self"
);

index_shape_case!(
    language_item_validation_rejects_index_projection_trait_arguments,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &IndexLanguageItems<DefId>| {
        let Type::Reference { inner, .. } = &mut signature_mut(program, items.method_id).ret else {
            panic!("valid Index return is a reference");
        };
        let Type::Projection { trait_args, .. } = inner.as_mut() else {
            panic!("valid Index return is a projection");
        };
        trait_args.clear();
    },
    "index.method return projection must use index generic 0"
);
