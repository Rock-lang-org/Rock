use std::collections::HashMap;

use crate::hir::{
    AcceptedHirProgram, HirAssociatedTypeDecl, HirAssociatedTypeDef, HirAssociatedTypeLocation,
    HirBlock, HirCallTarget, HirEnum, HirExpr, HirExprKind, HirExtern, HirField, HirFieldLocation,
    HirFunction, HirImpl, HirImplOwner, HirImplReceiverPattern, HirMethodCallTarget,
    HirMethodLocation, HirNameTables, HirParam, HirPattern, HirProgram, HirSelectedMethodTarget,
    HirSelectedTraitMember, HirStmt, HirStruct, HirStructPatternField, HirTrait,
    HirTraitDispatchKind, HirTypeBinding, HirVarRef, HirVarTarget, HirVariant, HirVariantFields,
    HirVariantLocation,
};
use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
use crate::types::{GenericParamDecl, GenericParamId, Type};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn field_id(index: u32) -> FieldId {
    FieldId(index)
}

fn variant_id(index: u32) -> VariantId {
    VariantId(index)
}

fn assoc_type_id(index: u32) -> AssocTypeId {
    AssocTypeId(index)
}

#[test]
fn hir_var_target_can_identify_local_bindings() {
    let local = crate::ids::HirLocalId(7);
    let target = HirVarTarget::Local(local);

    assert_eq!(target, HirVarTarget::Local(crate::ids::HirLocalId(7)));
}

#[test]
fn hir_call_target_can_identify_direct_function_calls() {
    let function_id = def_id(44);
    let target = HirCallTarget::Function(function_id);

    assert_eq!(target, HirCallTarget::Function(function_id));
}

#[test]
fn hir_var_target_can_reference_monomorphized_instance() {
    let target = HirVarTarget::Instance(crate::ids::InstanceId(9));

    assert_eq!(target, HirVarTarget::Instance(crate::ids::InstanceId(9)));
}

#[test]
fn method_selection_target_has_explicit_validated_variants() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(90));
    let method_id = DefId::new(CrateId(0), LocalDefId(91));
    let trait_id = DefId::new(CrateId(0), LocalDefId(92));
    let member_id = DefId::new(CrateId(0), LocalDefId(93));

    let inherent = HirMethodCallTarget {
        target: HirSelectedMethodTarget::ImplMethod {
            impl_id,
            method_id,
            selected_trait: None,
        },
        owner_substitution: Vec::new(),
        method_substitution: Vec::new(),
    };
    assert_eq!(inherent.impl_id(), Some(impl_id));
    assert_eq!(inherent.method_id(), Some(method_id));

    let trait_method = HirMethodCallTarget {
        target: HirSelectedMethodTarget::TraitMethod {
            trait_id,
            member_id,
            trait_args: vec![Type::I64],
            dispatch: HirTraitDispatchKind::TraitBound,
        },
        owner_substitution: vec![HirTypeBinding {
            param: GenericParamId {
                owner: impl_id,
                index: 0,
            },
            ty: Type::I64,
        }],
        method_substitution: Vec::new(),
    };
    assert_eq!(trait_method.trait_id(), Some(trait_id));
    assert_eq!(trait_method.method_id(), Some(member_id));
    assert_eq!(trait_method.trait_args(), &[Type::I64]);
}

#[test]
fn hir_reference_shapes_carry_resolved_targets() {
    let owner = def_id(10);
    let field = field_id(0);
    let variant = variant_id(0);

    let var = HirExprKind::ResolvedVar(HirVarRef {
        name: "make_box".to_string(),
        target: HirVarTarget::Function(def_id(11)),
    });
    let struct_literal = HirExprKind::StructLiteral("Box".to_string(), Some(owner), Vec::new());
    let struct_pattern = HirPattern::Struct(
        "Box".to_string(),
        Some(owner),
        Vec::new(),
        vec![HirStructPatternField {
            name: "value".to_string(),
            field: Some(HirFieldLocation {
                owner,
                field_id: field,
                name: "value".to_string(),
            }),
            pattern: HirPattern::Wildcard,
        }],
    );
    let enum_pattern = HirPattern::Enum(
        "Option".to_string(),
        "Some".to_string(),
        Some(HirVariantLocation {
            owner,
            variant_id: variant,
            name: "Some".to_string(),
        }),
        Vec::new(),
    );

    match var {
        HirExprKind::ResolvedVar(reference) => assert_eq!(reference.name, "make_box"),
        _ => panic!("expected resolved var"),
    }
    match struct_literal {
        HirExprKind::StructLiteral(_, Some(id), _) => assert_eq!(id, owner),
        _ => panic!("expected resolved struct literal"),
    }
    match struct_pattern {
        HirPattern::Struct(_, Some(id), _, fields) => {
            assert_eq!(id, owner);
            assert_eq!(fields[0].field.as_ref().unwrap().field_id, field);
        }
        _ => panic!("expected resolved struct pattern"),
    }
    match enum_pattern {
        HirPattern::Enum(_, _, Some(location), _) => assert_eq!(location.variant_id, variant),
        _ => panic!("expected resolved enum pattern"),
    }
}

fn empty_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::Unit,
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

fn test_function(id: DefId, name: &str) -> HirFunction {
    empty_function(id, name)
}

fn empty_struct(id: DefId, name: &str) -> HirStruct {
    HirStruct {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        fields: vec![HirField {
            id: field_id(0),
            name: "value".to_string(),
            ty: Type::I32,
            public: true,
        }],
    }
}

fn empty_enum(id: DefId, name: &str) -> HirEnum {
    HirEnum {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        variants: Vec::new(),
    }
}

fn trait_with_default_method(id: DefId, name: &str, method_id: DefId) -> HirTrait {
    HirTrait {
        target: None,
        predicates: Vec::new(),
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([(
            "default_method".to_string(),
            empty_function(method_id, "default_method"),
        )]),
        signatures: HashMap::new(),
    }
}

fn impl_with_method(id: DefId, method_id: DefId) -> HirImpl {
    HirImpl {
        id,
        owner: HirImplOwner::Named("Thing".to_string()),
        type_name: "Thing".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: vec![HirAssociatedTypeDef {
            id: assoc_type_id(0),
            name: "Item".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::I32,
        }],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "impl_method".to_string(),
            empty_function(method_id, "impl_method"),
        )]),
    }
}

fn program(
    functions: HashMap<DefId, HirFunction>,
    structs: HashMap<DefId, HirStruct>,
    enums: HashMap<DefId, HirEnum>,
    traits: HashMap<DefId, HirTrait>,
    impls: HashMap<DefId, HirImpl>,
    externs: HashMap<DefId, HirExtern>,
    names: HirNameTables,
    canonical_names: HashMap<DefId, String>,
) -> HirProgram {
    HirProgram::from_id_parts_with_names_and_canonical_names(
        functions,
        structs,
        enums,
        traits,
        impls,
        externs,
        names,
        crate::hir::HirLanguageItems::default(),
        &canonical_names,
    )
}

#[test]
fn accepted_hir_rejects_raw_method_callee_without_call_target() {
    let function_id = def_id(70);
    let impl_id = def_id(71);
    let method_id = def_id(72);
    let mut function = empty_function(function_id, "main");
    function.body.stmts.push(HirStmt::Expr(HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "method".to_string(),
                    target: HirVarTarget::Function(method_id),
                }),
                ty: Type::function(Vec::new(), Type::Unit),
                span: crate::lexer::Span::test(),
            }),
            Vec::new(),
            None,
        ),
        ty: Type::Unit,
        span: crate::lexer::Span::test(),
    }));
    let mut program = program(
        HashMap::from([(function_id, function)]),
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
    program.indexes.methods_by_id.insert(
        method_id,
        HirMethodLocation::ImplMethod {
            impl_id,
            method_id,
            method_name: "method".to_string(),
        },
    );

    let errors = program.validate_method_authorities();

    assert!(errors
        .iter()
        .any(|error| error.contains("raw method DefId")));
}

#[test]
fn constructor_trait_selection_accepted_hir_rejects_targetless_constructor_call() {
    let function_id = def_id(73);
    let impl_id = def_id(74);
    let method_id = def_id(75);
    let constructor_id = def_id(76);
    let mut function = empty_function(function_id, "main");
    function.body.stmts.push(HirStmt::Expr(HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "Option::pure".to_string(),
                    target: HirVarTarget::Function(method_id),
                }),
                ty: Type::function(Vec::new(), Type::Unit),
                span: crate::lexer::Span::test(),
            }),
            Vec::new(),
            Some(HirCallTarget::Function(method_id)),
        ),
        ty: Type::Unit,
        span: crate::lexer::Span::test(),
    }));
    let mut imp = impl_with_method(impl_id, method_id);
    imp.receiver_pattern = HirImplReceiverPattern::Constructor(Type::Constructor {
        id: constructor_id,
        flavor: crate::types::NominalTypeKind::Enum,
    });
    let program = program(
        HashMap::from([(function_id, function)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(impl_id, imp)]),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("main".to_string(), function_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let errors = program.validate_method_authorities();

    assert!(errors
        .iter()
        .any(|error| { error.contains("has no selected static-method authority") }));
}

#[test]
fn accepted_hir_rejects_missing_try_branch_receiver_authority() {
    let function_id = def_id(80);
    let trait_id = def_id(81);
    let member_id = def_id(82);
    let target = HirMethodCallTarget {
        target: HirSelectedMethodTarget::TraitMethod {
            trait_id,
            member_id,
            trait_args: Vec::new(),
            dispatch: HirTraitDispatchKind::TraitBound,
        },
        owner_substitution: Vec::new(),
        method_substitution: Vec::new(),
    };
    let mut function = empty_function(function_id, "main");
    function.body.stmts.push(HirStmt::Expr(HirExpr {
        kind: HirExprKind::Try {
            expr: Box::new(HirExpr {
                kind: HirExprKind::Unit,
                ty: Type::Unit,
                span: crate::lexer::Span::test(),
            }),
            branch_method: Some(target.clone()),
            branch_target: None,
            branch_self_receiver: None,
            from_residual_target: Some(HirCallTarget::StaticMethod(
                crate::hir::HirStaticMethodTarget {
                    owner_ty: Type::Unit,
                    method: target,
                },
            )),
            output_ty: Type::Unit,
            residual_ty: Type::Unit,
            return_ty: Type::Unit,
            control_flow_enum: def_id(83),
            break_variant: HirVariantLocation {
                owner: def_id(83),
                variant_id: variant_id(0),
                name: "Break".to_string(),
            },
            continue_variant: HirVariantLocation {
                owner: def_id(83),
                variant_id: variant_id(1),
                name: "Continue".to_string(),
            },
        },
        ty: Type::Unit,
        span: crate::lexer::Span::test(),
    }));
    let trait_def = HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Try".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "branch".to_string(),
            crate::hir::HirFunctionSig {
                id: member_id,
                name: "branch".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Unit],
                ret: Type::Unit,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(crate::types::ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    };
    let program = program(
        HashMap::from([(function_id, function)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(trait_id, trait_def)]),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("main".to_string(), function_id)]),
            traits_by_name: HashMap::from([("Try".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let errors = program.validate_method_authorities();

    assert!(errors
        .iter()
        .any(|error| error.contains("Try branch receiver mode mismatch")));
}

#[test]
fn accepted_hir_program_requires_validated_construction() {
    let program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        HashMap::new(),
    );

    let accepted = crate::hir::AcceptedHirProgram::try_from(program)
        .expect("empty program has valid method authority");

    assert!(accepted.program().functions.is_empty());
}

#[test]
fn hir_rejects_static_trait_member_mapped_to_receiver_impl_method() {
    let trait_id = def_id(84);
    let member_id = def_id(85);
    let impl_id = def_id(86);
    let method_id = def_id(87);
    let trait_def = trait_with_default_method(trait_id, "Protocol", member_id);
    let mut imp = impl_with_method(impl_id, method_id);
    imp.trait_id = Some(trait_id);
    imp.trait_name = Some("Protocol".to_string());
    let method = imp.methods.get_mut("impl_method").expect("fixture method");
    method.is_method = true;
    method.self_receiver = Some(crate::types::ReceiverMode::Move);
    let mut program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(trait_id, trait_def)]),
        HashMap::from([(impl_id, imp)]),
        HashMap::new(),
        HirNameTables::default(),
        HashMap::new(),
    );
    program
        .indexes
        .effective_trait_methods
        .insert((impl_id, member_id), method_id);

    let errors = program.validate_method_authorities();

    assert!(errors.iter().any(|error| {
        error.contains("effective trait member")
            && error.contains(&format!("{member_id:?}"))
            && error.contains(&format!("{method_id:?}"))
            && error.contains("receiver/static mismatch")
    }));
}

#[test]
fn accepted_hir_rejects_method_call_without_trait_or_impl_identity() {
    let trait_id = def_id(85);
    let method_id = def_id(86);
    let mut method = empty_function(method_id, "default");
    method.body.stmts.push(HirStmt::Expr(HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKind::Unit,
                ty: Type::Unit,
                span: crate::lexer::Span::test(),
            }),
            "missing".to_string(),
            Vec::new(),
            None,
            None,
        ),
        ty: Type::Unit,
        span: crate::lexer::Span::test(),
    }));
    let trait_def = HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Defaults".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("default".to_string(), method)]),
        signatures: HashMap::new(),
    };
    let program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(trait_id, trait_def)]),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([("Defaults".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let errors = program.validate_method_authorities();

    assert!(errors
        .iter()
        .any(|error| error.contains("method call has no selected authority")));
}

#[test]
fn hir_function_with_error_type_is_not_codegen_concrete() {
    let mut function = empty_function(def_id(87), "broken");
    function.ret_type = Type::Error;
    function.body.ty = Type::Error;

    assert!(!crate::hir::hir_function_is_codegen_concrete(&function));
    assert!(!crate::hir::function_is_object_provided_candidate(
        &function
    ));
    let program = program(
        HashMap::from([(function.id, function)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("broken".to_string(), def_id(87))]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );
    assert!(crate::hir::AcceptedHirProgram::try_from(program).is_err());
}

#[test]
fn accepted_hir_rejects_unresolved_type_locations() {
    let function_id = def_id(89);
    let unresolved = Type::TypeVar(crate::ids::TypeVarId(0));
    let mut function = empty_function(function_id, "broken");
    function.ret_type = unresolved.clone();
    function.body.ty = unresolved.clone();
    function.body.stmts.push(HirStmt::Expr(HirExpr {
        kind: HirExprKind::Unit,
        ty: unresolved,
        span: crate::lexer::Span::test(),
    }));
    let program = program(
        HashMap::from([(function_id, function)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("broken".to_string(), function_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let errors = crate::hir::AcceptedHirProgram::try_from(program)
        .expect_err("accepted HIR must reject every unresolved location");
    let unresolved_errors = errors
        .iter()
        .filter(|error| error.contains("accepted HIR contains unresolved type at"))
        .collect::<Vec<_>>();

    assert_eq!(unresolved_errors.len(), 3);
    assert!(unresolved_errors
        .iter()
        .any(|error| error.contains("FunctionReturn")));
    assert!(unresolved_errors
        .iter()
        .any(|error| error.contains("Block")));
    assert!(unresolved_errors.iter().any(|error| error.contains("Expr")));
}

#[test]
fn accepted_hir_rejects_unsaturated_constructor_in_return_type() {
    let function_id = def_id(190);
    let struct_id = def_id(191);
    let mut function = empty_function(function_id, "broken");
    function.ret_type = Type::Constructor {
        id: struct_id,
        flavor: crate::types::NominalTypeKind::Struct,
    };
    let structure = HirStruct {
        id: struct_id,
        name: "Box".to_string(),
        generic_params: vec![crate::types::GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: struct_id,
                index: 0,
            },
            "T",
        )],
        fields: Vec::new(),
    };
    let program = program(
        HashMap::from([(function_id, function)]),
        HashMap::from([(struct_id, structure)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        HashMap::new(),
    );

    let errors = crate::hir::AcceptedHirProgram::try_from(program).unwrap_err();

    assert!(errors.iter().any(|error| {
        error.contains("FunctionReturn") && error.contains("has kind Type -> Type, expected Type")
    }));
}

#[test]
fn kind_check_rejects_unsaturated_constructors_in_runtime_locations() {
    let function_id = def_id(194);
    let box_id = def_id(195);
    let holder_id = def_id(196);
    let constructor = Type::Constructor {
        id: box_id,
        flavor: crate::types::NominalTypeKind::Struct,
    };
    let box_struct = HirStruct {
        id: box_id,
        name: "Box".to_string(),
        generic_params: vec![crate::types::GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: box_id,
                index: 0,
            },
            "T",
        )],
        fields: Vec::new(),
    };
    let holder = HirStruct {
        id: holder_id,
        name: "Holder".to_string(),
        generic_params: Vec::new(),
        fields: vec![HirField {
            id: FieldId(0),
            name: "value".to_string(),
            ty: constructor.clone(),
            public: false,
        }],
    };
    let mut function = empty_function(function_id, "broken");
    function.params.push(HirParam {
        name: "value".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: constructor.clone(),
        mutable: false,
        is_ref: false,
    });
    function.ret_type = constructor.clone();
    function.body.stmts.push(HirStmt::Let {
        name: "local".to_string(),
        local_id: crate::ids::HirLocalId(1),
        ty: constructor,
        value: HirExpr {
            kind: HirExprKind::Unit,
            ty: Type::Unit,
            span: crate::lexer::Span::test(),
        },
        mutable: false,
    });
    let program = program(
        HashMap::from([(function_id, function)]),
        HashMap::from([(box_id, box_struct), (holder_id, holder)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        HashMap::new(),
    );

    let errors = crate::hir::AcceptedHirProgram::try_from(program).unwrap_err();

    for location in ["StructField", "FunctionParam", "FunctionReturn", "LetStmt"] {
        assert!(
            errors.iter().any(|error| error.contains(location)
                && error.contains("has kind Type -> Type, expected Type")),
            "missing {location} error in {errors:?}"
        );
    }
}

#[test]
fn accepted_hir_rejects_noncanonical_saturated_application() {
    let function_id = def_id(192);
    let struct_id = def_id(193);
    let mut function = empty_function(function_id, "broken");
    function.ret_type = Type::Apply {
        constructor: Box::new(Type::Constructor {
            id: struct_id,
            flavor: crate::types::NominalTypeKind::Struct,
        }),
        args: vec![Type::I64],
    };
    let structure = HirStruct {
        id: struct_id,
        name: "Box".to_string(),
        generic_params: vec![crate::types::GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: struct_id,
                index: 0,
            },
            "T",
        )],
        fields: Vec::new(),
    };
    let program = program(
        HashMap::from([(function_id, function)]),
        HashMap::from([(struct_id, structure)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        HashMap::new(),
    );

    let errors = crate::hir::AcceptedHirProgram::try_from(program).unwrap_err();

    assert!(errors
        .iter()
        .any(|error| { error.contains("FunctionReturn") && error.contains("noncanonical type") }));
}

#[test]
fn accepted_hir_rejects_unresolved_ordinary_field_access() {
    let function_id = def_id(88);
    let mut function = empty_function(function_id, "main");
    function.body.stmts.push(HirStmt::Expr(HirExpr {
        kind: HirExprKind::FieldAccess(
            Box::new(HirExpr {
                kind: HirExprKind::Unit,
                ty: Type::Unit,
                span: crate::lexer::Span::test(),
            }),
            "missing".to_string(),
            None,
        ),
        ty: Type::Unit,
        span: crate::lexer::Span::test(),
    }));
    let program = program(
        HashMap::from([(function_id, function)]),
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

    let errors = program.validate_method_authorities();

    assert!(errors
        .iter()
        .any(|error| error.contains("unresolved field access")));
}

#[test]
fn accepted_hir_rejects_pre_mono_instance_targets() {
    let function_id = def_id(89);
    let instance_id = crate::ids::InstanceId(3);
    let mut function = empty_function(function_id, "main");
    function.body.stmts.push(HirStmt::Expr(HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "specialized".to_string(),
                    target: HirVarTarget::Instance(instance_id),
                }),
                ty: Type::function(Vec::new(), Type::Unit),
                span: crate::lexer::Span::test(),
            }),
            Vec::new(),
            Some(HirCallTarget::Instance(instance_id)),
        ),
        ty: Type::Unit,
        span: crate::lexer::Span::test(),
    }));
    let program = program(
        HashMap::from([(function_id, function)]),
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

    let errors = program.validate_method_authorities();

    assert!(errors
        .iter()
        .any(|error| error.contains("pre-monomorphization instance")));
}

#[test]
fn accepted_hir_rejects_incomplete_impl_receiver_pattern_bindings() {
    let owner_id = def_id(90);
    let impl_id = def_id(91);
    let method_id = def_id(92);
    let mut imp = impl_with_method(impl_id, method_id);
    imp.type_generics = GenericParamDecl::type_params(impl_id, ["T"]);
    imp.receiver_pattern = crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
        id: owner_id,
        args: Vec::new(),
    });
    let program = program(
        HashMap::new(),
        HashMap::from([(owner_id, empty_struct(owner_id, "Thing"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(impl_id, imp)]),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([("Thing".to_string(), owner_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );
    let errors = program.validate_method_authorities();

    assert!(errors.iter().any(|error| {
        error.contains("receiver/trait argument generic bindings")
            && error.contains(&format!("{impl_id:?}"))
    }));
}

#[test]
fn accepted_hir_allows_impl_generic_bound_by_trait_argument() {
    let impl_id = def_id(193);
    let method_id = def_id(194);
    let receiver_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let trait_arg_param = GenericParamId {
        owner: impl_id,
        index: 1,
    };
    let mut imp = impl_with_method(impl_id, method_id);
    imp.type_generics = GenericParamDecl::type_params(impl_id, ["T", "U"]);
    imp.receiver_pattern = HirImplReceiverPattern::Exact(Type::Generic(receiver_param));
    imp.trait_arg_types = vec![Type::Generic(trait_arg_param)];

    assert_eq!(
        super::validate_impl_receiver_pattern_bindings(&imp, &imp.receiver_pattern),
        Ok(())
    );
}

#[test]
fn accepted_hir_rejects_ambiguous_concrete_projection_impls() {
    let owner_id = def_id(93);
    let trait_id = def_id(94);
    let generic_impl_id = def_id(95);
    let concrete_impl_id = def_id(96);
    let function_id = def_id(97);
    let assoc_type_id = AssocTypeId(0);
    let owner_ty = Type::Struct {
        id: owner_id,
        args: Vec::new(),
    };
    let projection_ty = Type::Projection {
        ty: Box::new(owner_ty.clone()),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: Vec::new(),
    };
    let trait_def = HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Project".to_string(),
        generic_params: Vec::new(),
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_type_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
    };
    let projection_impl = |id, receiver_pattern| HirImpl {
        id,
        owner: HirImplOwner::Named("Owner".to_string()),
        type_name: "Owner".to_string(),
        type_generics: (id == generic_impl_id)
            .then(|| GenericParamDecl::type_params(id, ["T"]))
            .unwrap_or_default(),
        receiver_pattern,
        trait_name: Some("Project".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: vec![HirAssociatedTypeDef {
            id: assoc_type_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::I64,
        }],
        bounds: HashMap::new().into(),
        methods: HashMap::new(),
    };
    let function = HirFunction {
        id: function_id,
        name: "project".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: projection_ty.clone(),
        body: HirBlock {
            stmts: Vec::new(),
            ty: projection_ty,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = program(
        HashMap::from([(function_id, function)]),
        HashMap::from([(owner_id, empty_struct(owner_id, "Owner"))]),
        HashMap::new(),
        HashMap::from([(trait_id, trait_def)]),
        HashMap::from([
            (
                generic_impl_id,
                projection_impl(
                    generic_impl_id,
                    HirImplReceiverPattern::Exact(Type::Generic(GenericParamId {
                        owner: generic_impl_id,
                        index: 0,
                    })),
                ),
            ),
            (
                concrete_impl_id,
                projection_impl(concrete_impl_id, HirImplReceiverPattern::Exact(owner_ty)),
            ),
        ]),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("project".to_string(), function_id)]),
            structs_by_name: HashMap::from([("Owner".to_string(), owner_id)]),
            traits_by_name: HashMap::from([("Project".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let errors = AcceptedHirProgram::try_from(program).unwrap_err();

    assert!(errors.iter().any(|error| {
        error.contains("multiple matching typed projection implementations")
            && error.contains(&format!("{generic_impl_id:?}"))
            && error.contains(&format!("{concrete_impl_id:?}"))
    }));
}

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
fn hir_method_locations_use_owner_ids_not_names_or_indexes() {
    let trait_id = def_id(20);
    let trait_method_id = def_id(21);
    let impl_id = def_id(22);
    let impl_method_id = def_id(23);

    let traits = HashMap::from([(
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Display".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("fmt".to_string(), test_function(trait_method_id, "fmt"))]),
            signatures: HashMap::new(),
        },
    )]);
    let impls = HashMap::from([(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Widget".to_string()),
            type_name: "Widget".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: Some("Display".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("fmt".to_string(), test_function(impl_method_id, "fmt"))]),
        },
    )]);

    let program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        traits,
        impls,
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([("Display".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    assert_eq!(
        program.indexes.methods_by_id.get(&trait_method_id),
        Some(&HirMethodLocation::TraitDefault {
            trait_id,
            method_id: trait_method_id,
            method_name: "fmt".to_string(),
        })
    );
    assert_eq!(
        program.indexes.methods_by_id.get(&impl_method_id),
        Some(&HirMethodLocation::ImplMethod {
            impl_id,
            method_id: impl_method_id,
            method_name: "fmt".to_string(),
        })
    );
    assert_eq!(
        program.effective_trait_method(impl_id, trait_method_id),
        None
    );
}

#[test]
fn hir_index_build_does_not_reconstruct_effective_trait_methods_by_name() {
    let trait_id = def_id(120);
    let trait_method_id = def_id(121);
    let impl_id = def_id(122);
    let impl_method_id = def_id(123);
    let traits = HashMap::from([(
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Display".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("fmt".to_string(), test_function(trait_method_id, "fmt"))]),
            signatures: HashMap::new(),
        },
    )]);
    let impls = HashMap::from([(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Widget".to_string()),
            type_name: "Widget".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: Some("Display".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("fmt".to_string(), test_function(impl_method_id, "fmt"))]),
        },
    )]);

    let mut program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        traits,
        impls,
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([("Display".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );
    program.rebuild_indexes();

    assert!(program.indexes.effective_trait_methods.is_empty());
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
fn hir_program_does_not_reconstruct_impl_owner_id_from_display_alias() {
    let struct_id = def_id(80);
    let impl_id = def_id(81);
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::from([(struct_id, empty_struct(struct_id, "Box"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("AliasBox".to_string()),
                type_name: "AliasBox".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Unit),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([
                ("module::Box".to_string(), struct_id),
                ("AliasBox".to_string(), struct_id),
            ]),
            ..HirNameTables::default()
        },
        crate::hir::HirLanguageItems::default(),
        &HashMap::from([(struct_id, "module::Box".to_string())]),
    );

    assert_eq!(program.impl_owner_id(impl_id), None);
    assert_eq!(
        program.impl_receiver_pattern(impl_id),
        Some(&HirImplReceiverPattern::Exact(Type::Unit))
    );
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
fn hir_program_externs_keep_owned_ids_after_serde_roundtrip_and_rebuild() {
    let extern_id = def_id(13);
    let default_id = DefId::new(CrateId(0), LocalDefId(0));
    let program = program(
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

    let bytes = bincode::serialize(&program).unwrap();
    let mut decoded: HirProgram = bincode::deserialize(&bytes).unwrap();
    decoded.rebuild_indexes();

    assert!(decoded.extern_by_id(extern_id).is_some());
    assert!(!decoded.indexes.externs_by_id.contains_key(&default_id));
}

#[test]
fn hir_program_indexes_same_named_fields_by_owner_and_field_id() {
    let left_owner = def_id(10);
    let right_owner = def_id(11);
    let program = program(
        HashMap::new(),
        HashMap::from([
            (
                left_owner,
                HirStruct {
                    id: left_owner,
                    name: "Left".to_string(),
                    generic_params: Vec::new(),
                    fields: vec![HirField {
                        id: field_id(0),
                        name: "value".to_string(),
                        ty: Type::I32,
                        public: true,
                    }],
                },
            ),
            (
                right_owner,
                HirStruct {
                    id: right_owner,
                    name: "Right".to_string(),
                    generic_params: Vec::new(),
                    fields: vec![HirField {
                        id: field_id(0),
                        name: "value".to_string(),
                        ty: Type::I64,
                        public: true,
                    }],
                },
            ),
        ]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([
                ("Left".to_string(), left_owner),
                ("Right".to_string(), right_owner),
            ]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    assert_eq!(
        program.indexes.fields_by_id[&(left_owner, field_id(0))],
        HirFieldLocation {
            owner: left_owner,
            field_id: field_id(0),
            name: "value".to_string(),
        }
    );
    assert_eq!(
        program.indexes.fields_by_id[&(right_owner, field_id(0))],
        HirFieldLocation {
            owner: right_owner,
            field_id: field_id(0),
            name: "value".to_string(),
        }
    );
    assert_eq!(program.indexes.fields_by_id.len(), 2);
}

#[test]
fn hir_program_indexes_same_named_variants_by_owner_and_variant_id() {
    let left_owner = def_id(20);
    let right_owner = def_id(21);
    let program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::from([
            (
                left_owner,
                HirEnum {
                    id: left_owner,
                    name: "LeftChoice".to_string(),
                    generic_params: Vec::new(),
                    variants: vec![HirVariant {
                        id: variant_id(0),
                        name: "Some".to_string(),
                        fields: HirVariantFields::Unit,
                    }],
                },
            ),
            (
                right_owner,
                HirEnum {
                    id: right_owner,
                    name: "RightChoice".to_string(),
                    generic_params: Vec::new(),
                    variants: vec![HirVariant {
                        id: variant_id(0),
                        name: "Some".to_string(),
                        fields: HirVariantFields::Unit,
                    }],
                },
            ),
        ]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            enums_by_name: HashMap::from([
                ("LeftChoice".to_string(), left_owner),
                ("RightChoice".to_string(), right_owner),
            ]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    assert_eq!(
        program.indexes.variants_by_id[&(left_owner, variant_id(0))],
        HirVariantLocation {
            owner: left_owner,
            variant_id: variant_id(0),
            name: "Some".to_string(),
        }
    );
    assert_eq!(
        program.indexes.variants_by_id[&(right_owner, variant_id(0))],
        HirVariantLocation {
            owner: right_owner,
            variant_id: variant_id(0),
            name: "Some".to_string(),
        }
    );
    assert_eq!(program.indexes.variants_by_id.len(), 2);
}

#[test]
fn hir_program_indexes_associated_type_decl_and_def_by_owner_and_assoc_type_id() {
    let trait_owner = def_id(30);
    let impl_owner = def_id(31);
    let program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            trait_owner,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_owner,
                name: "Iterable".to_string(),
                generic_params: Vec::new(),
                associated_types: vec![HirAssociatedTypeDecl {
                    id: assoc_type_id(0),
                    name: "Item".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                }],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        )]),
        HashMap::from([(
            impl_owner,
            HirImpl {
                id: impl_owner,
                owner: HirImplOwner::Named("List".to_string()),
                type_name: "List".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: Vec::new().into(),
                trait_name: Some("Iterable".to_string()),
                trait_id: Some(trait_owner),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: vec![HirAssociatedTypeDef {
                    id: assoc_type_id(0),
                    name: "Item".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: Type::I32,
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([("Iterable".to_string(), trait_owner)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    assert_eq!(
        program.indexes.associated_type_decls_by_id[&(trait_owner, assoc_type_id(0))],
        HirAssociatedTypeLocation {
            owner: trait_owner,
            assoc_type_id: assoc_type_id(0),
            name: "Item".to_string(),
        }
    );
    assert_eq!(
        program.indexes.associated_type_defs_by_id[&(impl_owner, assoc_type_id(0))],
        HirAssociatedTypeLocation {
            owner: impl_owner,
            assoc_type_id: assoc_type_id(0),
            name: "Item".to_string(),
        }
    );
    assert_eq!(program.indexes.associated_type_decls_by_id.len(), 1);
    assert_eq!(program.indexes.associated_type_defs_by_id.len(), 1);
}

#[test]
fn hir_extern_carries_canonical_def_id() {
    let extern_fn = HirExtern {
        id: DefId::new(CrateId(0), LocalDefId(9)),
        name: "puts".to_string(),
        params: vec![Type::Str],
        ret: Type::I32,
        variadic: false,
        is_unsafe: false,
    };

    assert_eq!(extern_fn.id, DefId::new(CrateId(0), LocalDefId(9)));
}

#[test]
fn hir_extern_serde_roundtrips_id() {
    let extern_fn = HirExtern {
        id: DefId::new(CrateId(1), LocalDefId(9)),
        name: "printf".to_string(),
        params: vec![Type::Pointer(Box::new(Type::I8))],
        ret: Type::I32,
        variadic: true,
        is_unsafe: false,
    };

    let bytes = bincode::serialize(&extern_fn).unwrap();
    let decoded: HirExtern = bincode::deserialize(&bytes).unwrap();

    assert_eq!(decoded.id, extern_fn.id);
    assert_eq!(decoded.name, extern_fn.name);
    assert_eq!(decoded.params, extern_fn.params);
    assert_eq!(decoded.ret, extern_fn.ret);
    assert_eq!(decoded.variadic, extern_fn.variadic);
}

#[test]
fn hir_program_serde_skips_derived_indexes() {
    let program = program(
        HashMap::from([(def_id(1), empty_function(def_id(1), "main"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            def_id(2),
            HirExtern {
                id: def_id(2),
                name: "puts".to_string(),
                params: vec![Type::Str],
                ret: Type::I32,
                variadic: false,
                is_unsafe: false,
            },
        )]),
        HirNameTables {
            functions_by_name: HashMap::from([("main".to_string(), def_id(1))]),
            externs_by_name: HashMap::from([("puts".to_string(), def_id(2))]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    assert!(!program.indexes.functions_by_id.is_empty());
    assert!(!program.indexes.externs_by_id.is_empty());

    let bytes = bincode::serialize(&program).unwrap();
    let decoded: HirProgram = bincode::deserialize(&bytes).unwrap();

    assert!(decoded.indexes.functions_by_id.is_empty());
    assert!(decoded.indexes.externs_by_id.is_empty());
    assert_eq!(decoded.externs[&def_id(2)].id, def_id(2));
}

#[test]
fn hir_program_indexes_definitions_by_def_id() {
    let program = program(
        HashMap::from([(def_id(1), empty_function(def_id(1), "main"))]),
        HashMap::from([(def_id(2), empty_struct(def_id(2), "Thing"))]),
        HashMap::from([(def_id(3), empty_enum(def_id(3), "Choice"))]),
        HashMap::from([(
            def_id(4),
            trait_with_default_method(def_id(4), "Show", def_id(8)),
        )]),
        HashMap::from([(def_id(5), impl_with_method(def_id(5), def_id(9)))]),
        HashMap::from([(
            def_id(6),
            HirExtern {
                id: def_id(6),
                name: "puts".to_string(),
                params: vec![Type::Str],
                ret: Type::I32,
                variadic: false,
                is_unsafe: false,
            },
        )]),
        HirNameTables {
            functions_by_name: HashMap::from([("main".to_string(), def_id(1))]),
            structs_by_name: HashMap::from([("Thing".to_string(), def_id(2))]),
            enums_by_name: HashMap::from([("Choice".to_string(), def_id(3))]),
            traits_by_name: HashMap::from([("Show".to_string(), def_id(4))]),
            externs_by_name: HashMap::from([("puts".to_string(), def_id(6))]),
            type_aliases_by_name: HashMap::new(),
        },
        HashMap::new(),
    );

    assert_eq!(program.indexes.functions_by_id[&def_id(1)], "main");
    assert_eq!(program.indexes.structs_by_id[&def_id(2)], "Thing");
    assert_eq!(program.indexes.enums_by_id[&def_id(3)], "Choice");
    assert_eq!(program.indexes.traits_by_id[&def_id(4)], "Show");
    assert_eq!(program.indexes.impls_by_id[&def_id(5)], 0);
    assert_eq!(program.indexes.externs_by_id[&def_id(6)], 0);
    assert_eq!(
        program.indexes.methods_by_id[&def_id(8)],
        HirMethodLocation::TraitDefault {
            trait_id: def_id(4),
            method_id: def_id(8),
            method_name: "default_method".to_string(),
        }
    );
    assert_eq!(
        program.indexes.methods_by_id[&def_id(9)],
        HirMethodLocation::ImplMethod {
            impl_id: def_id(5),
            method_id: def_id(9),
            method_name: "impl_method".to_string(),
        }
    );
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
fn hir_indexes_distinguish_multiple_inherited_default_method_ids() {
    let first_method_id = def_id(8);
    let second_method_id = def_id(9);
    let program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([
            (
                def_id(4),
                HirTrait {
                    target: None,
                    predicates: Vec::new(),
                    id: def_id(4),
                    name: "First".to_string(),
                    generic_params: Vec::new(),
                    associated_types: Vec::new(),
                    methods: HashMap::from([(
                        "value".to_string(),
                        empty_function(first_method_id, "value"),
                    )]),
                    signatures: HashMap::new(),
                },
            ),
            (
                def_id(5),
                HirTrait {
                    target: None,
                    predicates: Vec::new(),
                    id: def_id(5),
                    name: "Second".to_string(),
                    generic_params: Vec::new(),
                    associated_types: Vec::new(),
                    methods: HashMap::from([(
                        "value".to_string(),
                        empty_function(second_method_id, "value"),
                    )]),
                    signatures: HashMap::new(),
                },
            ),
        ]),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([
                ("First".to_string(), def_id(4)),
                ("Second".to_string(), def_id(5)),
            ]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    assert_eq!(
        program.indexes.methods_by_id[&first_method_id],
        HirMethodLocation::TraitDefault {
            trait_id: def_id(4),
            method_id: first_method_id,
            method_name: "value".to_string(),
        }
    );
    assert_eq!(
        program.indexes.methods_by_id[&second_method_id],
        HirMethodLocation::TraitDefault {
            trait_id: def_id(5),
            method_id: second_method_id,
            method_name: "value".to_string(),
        }
    );
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

#[test]
fn substitute_typevars_in_function_updates_method_target_trait_args() {
    let trait_id = def_id(100);
    let method_id = def_id(101);
    let impl_id = def_id(102);
    let generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let source_ty = Type::Generic(generic);
    let mut function = HirFunction {
        id: method_id,
        name: "call".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: source_ty.clone(),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::MethodCall(
                    Box::new(HirExpr {
                        kind: HirExprKind::Var("self".to_string()),
                        ty: source_ty.clone(),
                        span: crate::lexer::Span::test(),
                    }),
                    "target".to_string(),
                    Vec::new(),
                    None,
                    Some(HirMethodCallTarget::impl_method(
                        impl_id,
                        method_id,
                        Some(HirSelectedTraitMember {
                            trait_id,
                            member_id: method_id,
                            trait_args: vec![source_ty.clone()],
                        }),
                    )),
                ),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            })],
            ty: Type::I64,
        },
        is_curried: false,
        is_method: true,
        self_receiver: None,
        is_unsafe: false,
    };

    super::substitute_typevars_in_function(&mut function, &source_ty, &Type::I64);

    let HirStmt::Expr(expr) = &function.body.stmts[0] else {
        panic!("function body should contain method call");
    };
    let HirExprKind::MethodCall(_, _, _, _, Some(target)) = &expr.kind else {
        panic!("function body should contain method target");
    };
    assert_eq!(target.trait_args(), &[Type::I64]);
}
