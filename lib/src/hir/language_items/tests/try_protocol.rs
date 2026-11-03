use super::*;

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
