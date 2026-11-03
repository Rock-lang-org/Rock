use crate::collect;
use crate::crate_system::CrateContext;
use crate::ids::DefId;
use crate::infer;
use crate::language_items::IndexLanguageItems;
use crate::lower;
use crate::types::ReceiverMode;
use crate::types::{GenericParamDecl, GenericParamId, Type};

use super::validate_language_items;

const VALID_DROP_SOURCE: &str = r#"lang drop
< trait Release
    lang method
    ~@release: ()
"#;

const VALID_SIZED_SOURCE: &str = r#"lang sized
< trait StaticLayout
"#;

const VALID_INDEX_SOURCE: &str = r#"lang index
< trait Lookup Key
    lang output
    type Value
    lang method
    @lookup: Key -> &Self::Value
"#;

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

const VALID_TRY_SOURCE: &str = r#"lang control_flow
< enum Flow B, C
    lang break
    Stop B
    lang continue
    Next C

lang try
< trait Carrier
    lang output
    type Value
    lang residual
    type Remainder
    lang branch
    ~@split: Flow Self::Remainder, Self::Value

lang from_residual
< trait Recover R
    lang method
    recover: R -> Self
"#;

const VALID_CALLABLE_SOURCE: &str = r#"lang fn_once
< trait Once Args, Ret
    lang output
    type Output
    lang method
    ~@call_once: Args -> Ret

lang fn_mut
< trait Mutable Args, Ret
    lang output
    type Output
    lang method
    ^@call_mut: Args -> Ret

lang fn
< trait Shared Args, Ret
    lang output
    type Output
    lang method
    @call: Args -> Ret

lang send
< trait Transferable

lang sync
< trait Shareable
"#;

fn resolved_language_item_program(source: &str) -> crate::infer::ResolvedHirProgram {
    let program = crate::parser::parse_string(source, &crate::Config::default())
        .expect("language-item source should parse");
    let context = CrateContext::new();
    let declarations = collect::collect(&program, &context, false, Some("core"))
        .expect("language-item source should collect");
    let partial =
        lower::program::lower_from_declarations(&program, declarations, &context, Some("core"))
            .expect("language-item source should lower");
    infer::finalize(partial).expect("language-item source should finalize")
}

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
fn language_item_validation_accepts_renamed_finalized_protocols() {
    for source in [
        VALID_SIZED_SOURCE,
        VALID_DROP_SOURCE,
        VALID_INDEX_SOURCE,
        VALID_TRY_SOURCE,
        VALID_CALLABLE_SOURCE,
    ] {
        let hir = resolved_language_item_program(source);
        assert!(
            validate_language_items(hir.program.program()).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn language_item_validation_accepts_marked_default_method_by_id() {
    let hir = resolved_language_item_program(
        r#"lang drop
< trait Release
    lang method
    ~@release: ()
    ~@release = -> return
"#,
    );

    assert!(validate_language_items(hir.program.program()).is_empty());
    let items = hir
        .program
        .program()
        .language_items
        .drop
        .as_ref()
        .expect("Drop bundle");
    assert!(hir
        .program
        .program()
        .traits
        .get(&items.trait_id)
        .expect("Drop trait")
        .methods
        .values()
        .any(|method| method.id == items.method_id));
}

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
fn language_item_validation_accepts_try_branch_default_method_representation() {
    let mut hir = resolved_language_item_program(VALID_TRY_SOURCE);
    let program = hir.program.program_mut_for_test();
    let items = program
        .language_items
        .try_protocol
        .as_ref()
        .expect("Try bundle")
        .clone();
    let self_ty = Type::Generic(GenericParamId {
        owner: items.try_trait_id,
        index: 0,
    });
    let mut method = default_method_seed();
    method.id = items.branch_method_id;
    method.self_receiver = Some(ReceiverMode::Move);
    method.params[0].ty = self_ty.clone();
    method.ret_type = Type::Enum {
        id: items.control_flow_enum_id,
        args: vec![
            try_projection(self_ty.clone(), items.try_trait_id, items.residual_id),
            try_projection(self_ty, items.try_trait_id, items.output_id),
        ],
    };
    let trait_def = program
        .traits
        .get_mut(&items.try_trait_id)
        .expect("Try trait");
    trait_def.signatures.clear();
    trait_def
        .methods
        .insert("default_branch".to_string(), method);

    assert!(crate::hir::AcceptedHirProgram::revalidate_for_test(program.clone()).is_ok());
}

#[test]
fn language_item_validation_accepts_from_residual_default_method_representation() {
    let mut hir = resolved_language_item_program(VALID_TRY_SOURCE);
    let program = hir.program.program_mut_for_test();
    let items = program
        .language_items
        .try_protocol
        .as_ref()
        .expect("Try bundle")
        .clone();
    let mut method = default_method_seed();
    method.id = items.from_residual_method_id;
    method.is_method = false;
    method.self_receiver = None;
    method.params = vec![crate::hir::HirParam {
        name: "residual".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Generic(GenericParamId {
            owner: items.from_residual_trait_id,
            index: 0,
        }),
        mutable: false,
        is_ref: false,
    }];
    method.ret_type = Type::Generic(GenericParamId {
        owner: items.from_residual_trait_id,
        index: 1,
    });
    let trait_def = program
        .traits
        .get_mut(&items.from_residual_trait_id)
        .expect("FromResidual trait");
    trait_def.signatures.clear();
    trait_def
        .methods
        .insert("default_from_residual".to_string(), method);

    assert!(crate::hir::AcceptedHirProgram::revalidate_for_test(program.clone()).is_ok());
}

#[test]
fn language_item_validation_rejects_member_owned_by_another_trait() {
    let mut hir = resolved_language_item_program(
        r#"< trait Other
    @other: ()

lang drop
< trait Release
    lang method
    ~@release: ()
"#,
    );
    let program = hir.program.program_mut_for_test();
    let drop_id = program
        .language_items
        .drop
        .as_ref()
        .expect("Drop bundle")
        .trait_id;
    let other_method = program
        .traits
        .iter()
        .find(|(id, _)| **id != drop_id)
        .and_then(|(_, trait_def)| trait_def.signatures.values().next())
        .expect("other method")
        .id;
    program
        .language_items
        .drop
        .as_mut()
        .expect("Drop bundle")
        .method_id = other_method;

    assert!(validate_language_items(program)
        .iter()
        .any(|error| error.starts_with("drop.method") && error.contains("is owned by trait")));
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
fn language_item_validation_rejects_try_control_flow_variant_from_another_enum() {
    let mut hir = resolved_language_item_program(
        r#"< enum Other
    First I64
    Second I64
    Third I64

lang control_flow
< enum Flow B, C
    lang break
    Stop B
    lang continue
    Next C

lang try
< trait Carrier
    lang output
    type Value
    lang residual
    type Remainder
    lang branch
    ~@split: Flow Self::Remainder, Self::Value

lang from_residual
< trait Recover R
    lang method
    recover: R -> Self
"#,
    );
    let program = hir.program.program_mut_for_test();
    program
        .language_items
        .try_protocol
        .as_mut()
        .expect("Try bundle")
        .break_variant_id = crate::ids::VariantId(2);

    assert!(validate_language_items(program).iter().any(|error| {
        error.starts_with("control_flow.break") && error.contains("is declared by enum")
    }));
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

#[test]
fn language_item_validation_rejects_try_and_control_flow_shapes() {
    let mut hir = resolved_language_item_program(VALID_TRY_SOURCE);
    let program = hir.program.program_mut_for_test();
    let items = program
        .language_items
        .try_protocol
        .as_ref()
        .expect("Try bundle")
        .clone();
    signature_mut(program, items.branch_method_id).ret = Type::Unit;
    let from_residual = signature_mut(program, items.from_residual_method_id);
    from_residual.params.push(Type::I64);
    from_residual.ret = Type::Unit;
    let control_flow = program
        .enums
        .get_mut(&items.control_flow_enum_id)
        .expect("ControlFlow enum");
    control_flow
        .generic_params
        .push(GenericParamDecl::type_param(
            GenericParamId {
                owner: items.control_flow_enum_id,
                index: 2,
            },
            "Extra",
        ));
    control_flow.variants.push(crate::hir::HirVariant {
        id: crate::ids::VariantId(2),
        name: "Extra".to_string(),
        fields: crate::hir::HirVariantFields::Unit,
    });
    let break_variant = control_flow
        .variants
        .iter_mut()
        .find(|variant| variant.id == items.break_variant_id)
        .expect("Break variant");
    break_variant.fields = crate::hir::HirVariantFields::Unit;

    let errors = validate_language_items(program);
    assert!(errors
        .iter()
        .any(|error| error == "try.branch must return control_flow<Residual, Output>"));
    assert!(errors.iter().any(|error| {
        error == "from_residual.method must declare exactly 1 explicit parameters; found 2"
    }));
    assert!(errors
        .iter()
        .any(|error| error == "from_residual.method must return Self"));
    assert!(errors.iter().any(|error| {
        error == "control_flow.enum must declare exactly 2 generic parameters; found 3"
    }));
    assert!(errors
        .iter()
        .any(|error| { error == "control_flow.enum must declare exactly 2 variants; found 3" }));
    assert!(errors
        .iter()
        .any(|error| error == "control_flow.break must not be unit"));
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

macro_rules! try_shape_case {
    ($name:ident, $mutate:expr, $expected:expr) => {
        #[test]
        fn $name() {
            let mut hir = resolved_language_item_program(VALID_TRY_SOURCE);
            let program = hir.program.program_mut_for_test();
            let items = program
                .language_items
                .try_protocol
                .as_ref()
                .expect("Try bundle")
                .clone();
            ($mutate)(program, &items);
            assert_eq!(
                validate_language_items(program),
                vec![$expected.to_string()]
            );
        }
    };
}

try_shape_case!(
    language_item_validation_rejects_try_generics,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        program
            .traits
            .get_mut(&items.try_trait_id)
            .expect("Try trait")
            .generic_params
            .push(GenericParamDecl::type_param(
                GenericParamId {
                    owner: items.try_trait_id,
                    index: 0,
                },
                "T",
            ));
    },
    "try.trait must declare exactly 0 generic parameters; found 1"
);

try_shape_case!(
    language_item_validation_rejects_try_branch_non_move_receiver,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        signature_mut(program, items.branch_method_id).self_receiver = Some(ReceiverMode::Shared);
    },
    "try.branch must use a move receiver"
);

try_shape_case!(
    language_item_validation_rejects_try_branch_explicit_argument,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        signature_mut(program, items.branch_method_id)
            .params
            .push(Type::I64);
    },
    "try.branch must declare exactly 0 explicit parameters; found 1"
);

try_shape_case!(
    language_item_validation_rejects_try_branch_wrong_control_flow_enum,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        let Type::Enum { id, .. } = &mut signature_mut(program, items.branch_method_id).ret else {
            panic!("valid branch return is an enum");
        };
        *id = DefId::new(crate::ids::CrateId(44), crate::ids::LocalDefId(1));
    },
    "try.branch must return the marked control_flow enum"
);

try_shape_case!(
    language_item_validation_rejects_try_residual_projection_wrong_trait,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        let Type::Enum { args, .. } = &mut signature_mut(program, items.branch_method_id).ret
        else {
            panic!("valid branch return is an enum");
        };
        let Type::Projection { trait_id, .. } = &mut args[0] else {
            panic!("valid residual is a projection");
        };
        *trait_id = DefId::new(crate::ids::CrateId(44), crate::ids::LocalDefId(2));
    },
    "try.branch residual projection must use try.trait"
);

try_shape_case!(
    language_item_validation_rejects_try_output_projection_wrong_associated_type,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        let Type::Enum { args, .. } = &mut signature_mut(program, items.branch_method_id).ret
        else {
            panic!("valid branch return is an enum");
        };
        let Type::Projection { assoc_type, .. } = &mut args[1] else {
            panic!("valid output is a projection");
        };
        assoc_type.assoc_type_id = items.residual_id;
    },
    "try.branch output projection must use its marked associated type"
);

try_shape_case!(
    language_item_validation_rejects_try_projection_non_self_base,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        let Type::Enum { args, .. } = &mut signature_mut(program, items.branch_method_id).ret
        else {
            panic!("valid branch return is an enum");
        };
        let Type::Projection { ty, .. } = &mut args[0] else {
            panic!("valid residual is a projection");
        };
        **ty = Type::I64;
    },
    "try.branch residual projection must project from try Self"
);

try_shape_case!(
    language_item_validation_rejects_try_projection_trait_arguments,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        let Type::Enum { args, .. } = &mut signature_mut(program, items.branch_method_id).ret
        else {
            panic!("valid branch return is an enum");
        };
        let Type::Projection { trait_args, .. } = &mut args[0] else {
            panic!("valid residual is a projection");
        };
        trait_args.push(Type::I64);
    },
    "try.branch residual projection must not have trait arguments"
);

try_shape_case!(
    language_item_validation_rejects_from_residual_non_static_method,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        let method = signature_mut(program, items.from_residual_method_id);
        method.self_receiver = Some(ReceiverMode::Shared);
        method.params.insert(
            0,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Generic(GenericParamId {
                    owner: items.from_residual_trait_id,
                    index: 1,
                })),
            },
        );
    },
    "from_residual.method must use a static method"
);

try_shape_case!(
    language_item_validation_rejects_from_residual_argument_wrong_owner,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        signature_mut(program, items.from_residual_method_id).params[0] =
            Type::Generic(GenericParamId {
                owner: DefId::new(crate::ids::CrateId(44), crate::ids::LocalDefId(3)),
                index: 0,
            });
    },
    "from_residual.method explicit parameter 0 has the wrong type"
);

try_shape_case!(
    language_item_validation_rejects_from_residual_argument_wrong_index,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        signature_mut(program, items.from_residual_method_id).params[0] =
            Type::Generic(GenericParamId {
                owner: items.from_residual_trait_id,
                index: 1,
            });
    },
    "from_residual.method explicit parameter 0 has the wrong type"
);

try_shape_case!(
    language_item_validation_rejects_from_residual_generics,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        program
            .traits
            .get_mut(&items.from_residual_trait_id)
            .expect("FromResidual trait")
            .generic_params
            .push(GenericParamDecl::type_param(
                GenericParamId {
                    owner: items.from_residual_trait_id,
                    index: 1,
                },
                "Extra",
            ));
    },
    "from_residual.trait must declare exactly 1 generic parameters; found 2"
);

macro_rules! control_flow_shape_case {
    ($name:ident, $mutate:expr, $expected:expr) => {
        #[test]
        fn $name() {
            let mut hir = resolved_language_item_program(VALID_TRY_SOURCE);
            let program = hir.program.program_mut_for_test();
            let items = program
                .language_items
                .try_protocol
                .as_ref()
                .expect("Try bundle")
                .clone();
            ($mutate)(program, &items);
            assert_eq!(
                validate_language_items(program),
                vec![$expected.to_string()]
            );
        }
    };
}

control_flow_shape_case!(
    language_item_validation_rejects_continue_wrong_generic_payload,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        let variant = program
            .enums
            .get_mut(&items.control_flow_enum_id)
            .expect("ControlFlow enum")
            .variants
            .iter_mut()
            .find(|variant| variant.id == items.continue_variant_id)
            .expect("Continue variant");
        variant.fields =
            crate::hir::HirVariantFields::Positional(vec![Type::Generic(GenericParamId {
                owner: items.control_flow_enum_id,
                index: 0,
            })]);
    },
    "control_flow.continue must carry generic 1"
);

control_flow_shape_case!(
    language_item_validation_rejects_break_named_payload,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        let variant = program
            .enums
            .get_mut(&items.control_flow_enum_id)
            .expect("ControlFlow enum")
            .variants
            .iter_mut()
            .find(|variant| variant.id == items.break_variant_id)
            .expect("Break variant");
        variant.fields = crate::hir::HirVariantFields::Named(Vec::new());
    },
    "control_flow.break must use a positional payload"
);

control_flow_shape_case!(
    language_item_validation_rejects_break_multiple_payloads,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        let variant = program
            .enums
            .get_mut(&items.control_flow_enum_id)
            .expect("ControlFlow enum")
            .variants
            .iter_mut()
            .find(|variant| variant.id == items.break_variant_id)
            .expect("Break variant");
        variant.fields = crate::hir::HirVariantFields::Positional(vec![Type::I64, Type::I64]);
    },
    "control_flow.break must carry exactly one positional payload; found 2"
);

control_flow_shape_case!(
    language_item_validation_rejects_break_unit_payload,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        let variant = program
            .enums
            .get_mut(&items.control_flow_enum_id)
            .expect("ControlFlow enum")
            .variants
            .iter_mut()
            .find(|variant| variant.id == items.break_variant_id)
            .expect("Break variant");
        variant.fields = crate::hir::HirVariantFields::Unit;
    },
    "control_flow.break must not be unit"
);

control_flow_shape_case!(
    language_item_validation_rejects_control_flow_third_variant,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        program
            .enums
            .get_mut(&items.control_flow_enum_id)
            .expect("ControlFlow enum")
            .variants
            .push(crate::hir::HirVariant {
                id: crate::ids::VariantId(2),
                name: "Third".to_string(),
                fields: crate::hir::HirVariantFields::Unit,
            });
    },
    "control_flow.enum must declare exactly 2 variants; found 3"
);

control_flow_shape_case!(
    language_item_validation_rejects_control_flow_wrong_generic_count,
    |program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
     items: &crate::language_items::TryLanguageItems<DefId>| {
        program
            .enums
            .get_mut(&items.control_flow_enum_id)
            .expect("ControlFlow enum")
            .generic_params
            .pop();
    },
    "control_flow.enum must declare exactly 2 generic parameters; found 1"
);

#[test]
fn language_item_validation_rejects_identical_control_flow_markers() {
    let mut hir = resolved_language_item_program(VALID_TRY_SOURCE);
    let program = hir.program.program_mut_for_test();
    let items = program
        .language_items
        .try_protocol
        .as_mut()
        .expect("Try bundle");
    items.continue_variant_id = items.break_variant_id;

    assert_eq!(
        validate_language_items(program),
        vec![
            "control_flow.break and control_flow.continue must identify distinct variants"
                .to_string(),
            "control_flow.continue must carry generic 1".to_string(),
        ]
    );
}

macro_rules! try_provider_crate_case {
    ($name:ident, $mutate:expr, $expected:expr) => {
        #[test]
        fn $name() {
            let mut hir = resolved_language_item_program(VALID_TRY_SOURCE);
            let program = hir.program.program_mut_for_test();
            ($mutate)(
                program
                    .language_items
                    .try_protocol
                    .as_mut()
                    .expect("Try bundle"),
            );
            assert!(validate_language_items(program).contains(&$expected.to_string()));
        }
    };
}

try_provider_crate_case!(
    language_item_validation_rejects_try_provider_crate_for_try_root,
    |items: &mut crate::language_items::TryLanguageItems<DefId>| {
        items.try_trait_id.crate_id = crate::ids::CrateId(44);
    },
    "try bundle field try.branch must use crate CrateId(44); found CrateId(0)"
);
try_provider_crate_case!(
    language_item_validation_rejects_try_provider_crate_for_branch,
    |items: &mut crate::language_items::TryLanguageItems<DefId>| {
        items.branch_method_id.crate_id = crate::ids::CrateId(44);
    },
    "try bundle field try.branch must use crate CrateId(0); found CrateId(44)"
);
try_provider_crate_case!(
    language_item_validation_rejects_try_provider_crate_for_from_residual_root,
    |items: &mut crate::language_items::TryLanguageItems<DefId>| {
        items.from_residual_trait_id.crate_id = crate::ids::CrateId(44);
    },
    "try bundle field from_residual.trait must use crate CrateId(0); found CrateId(44)"
);
try_provider_crate_case!(
    language_item_validation_rejects_try_provider_crate_for_from_residual_method,
    |items: &mut crate::language_items::TryLanguageItems<DefId>| {
        items.from_residual_method_id.crate_id = crate::ids::CrateId(44);
    },
    "try bundle field from_residual.method must use crate CrateId(0); found CrateId(44)"
);
try_provider_crate_case!(
    language_item_validation_rejects_try_provider_crate_for_control_flow_root,
    |items: &mut crate::language_items::TryLanguageItems<DefId>| {
        items.control_flow_enum_id.crate_id = crate::ids::CrateId(44);
    },
    "try bundle field control_flow.enum must use crate CrateId(0); found CrateId(44)"
);

#[test]
fn accepted_hir_revalidation_rejects_malformed_language_items() {
    let mut hir = resolved_language_item_program(VALID_DROP_SOURCE);
    let program = hir.program.program_mut_for_test();
    let method_id = program
        .language_items
        .drop
        .as_ref()
        .expect("Drop bundle")
        .method_id;
    signature_mut(program, method_id).ret = Type::I64;

    let errors = crate::hir::AcceptedHirProgram::revalidate_for_test(program.clone())
        .expect_err("accepted conversion must reject malformed Drop");
    assert_eq!(errors, vec!["drop.method must return ()".to_string()]);
}

#[test]
fn strict_finalization_reports_malformed_language_items() {
    let program = crate::parser::parse_string(VALID_DROP_SOURCE, &crate::Config::default())
        .expect("language-item source should parse");
    let context = CrateContext::new();
    let declarations = collect::collect(&program, &context, false, Some("core"))
        .expect("language-item source should collect");
    let mut partial =
        lower::program::lower_from_declarations(&program, declarations, &context, Some("core"))
            .expect("language-item source should lower");
    partial
        .language_items
        .drop
        .as_mut()
        .expect("Drop bundle")
        .method_id = DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(999));

    let errors =
        infer::finalize(partial).expect_err("strict finalization must report malformed providers");
    assert_eq!(
            errors.into_iter().map(|error| error.message).collect::<Vec<_>>(),
            vec!["drop.method DefId { crate_id: CrateId(0), local: LocalDefId(999) } is not declared by any trait".to_string()]
        );
}

fn default_method_seed() -> crate::hir::HirFunctionFor<crate::hir::AcceptedHir> {
    let hir = resolved_language_item_program(
        r#"lang drop
< trait Release
    lang method
    ~@release: ()
    ~@release = -> return
"#,
    );
    hir.program
        .program()
        .traits
        .values()
        .flat_map(|trait_def| trait_def.methods.values())
        .next()
        .expect("Drop default method")
        .clone()
}

fn try_projection(ty: Type, trait_id: DefId, assoc_type_id: crate::ids::AssocTypeId) -> Type {
    Type::Projection {
        ty: Box::new(ty),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: Vec::new(),
    }
}

fn signature_mut(
    program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
    method_id: DefId,
) -> &mut crate::hir::HirFunctionSig {
    program
        .traits
        .values_mut()
        .flat_map(|trait_def| trait_def.signatures.values_mut())
        .find(|signature| signature.id == method_id)
        .expect("marked method signature")
}
