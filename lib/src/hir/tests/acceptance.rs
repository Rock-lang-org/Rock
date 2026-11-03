use super::*;

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
        super::super::validate_impl_receiver_pattern_bindings(&imp, &imp.receiver_pattern),
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
