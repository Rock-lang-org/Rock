use super::*;

#[test]
fn prune_unreachable_instances_maps_direct_function_target_to_zero_substitution_instance() {
    let main_id = DefId::new(CrateId(0), LocalDefId(90));
    let helper_id = DefId::new(CrateId(0), LocalDefId(91));
    let main_instance = InstanceId(0);
    let helper_instance = InstanceId(1);
    let main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: "wrong_display_name".to_string(),
                target: HirVarTarget::Function(helper_id),
            }),
            ty: Type::function(Vec::new(), Type::Unit),
            span: Span::test(),
        },
    );
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(main_body),
            false,
        ),
        instance_record(
            helper_instance,
            InstanceOrigin::Function(helper_id),
            "helper",
            "helper",
            Some(unit_return_function(helper_id, "helper")),
            false,
        ),
    ]);

    prune_unreachable_instances(&mut program);

    assert!(program.instances.contains_key(&helper_instance));
}

#[test]
fn prune_unreachable_instances_does_not_map_source_name_var_to_zero_substitution_instance() {
    let main_id = DefId::new(CrateId(0), LocalDefId(100));
    let helper_id = DefId::new(CrateId(0), LocalDefId(101));
    let main_instance = InstanceId(0);
    let helper_instance = InstanceId(1);
    let main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Var("helper".to_string()),
            ty: Type::function(Vec::new(), Type::Unit),
            span: Span::test(),
        },
    );
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(main_body),
            false,
        ),
        instance_record(
            helper_instance,
            InstanceOrigin::Function(helper_id),
            "helper",
            "helper",
            Some(unit_return_function(helper_id, "helper")),
            false,
        ),
    ]);

    prune_unreachable_instances(&mut program);

    assert!(!program.instances.contains_key(&helper_instance));
}

#[test]
fn prune_unreachable_instances_does_not_map_non_callable_var_name() {
    let main_id = DefId::new(CrateId(0), LocalDefId(120));
    let helper_id = DefId::new(CrateId(0), LocalDefId(121));
    let main_instance = InstanceId(0);
    let helper_instance = InstanceId(1);
    let main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Var("helper".to_string()),
            ty: Type::I64,
            span: Span::test(),
        },
    );
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(main_body),
            false,
        ),
        instance_record(
            helper_instance,
            InstanceOrigin::Function(helper_id),
            "helper",
            "helper",
            Some(unit_return_function(helper_id, "helper")),
            false,
        ),
    ]);

    prune_unreachable_instances(&mut program);

    assert!(!program.instances.contains_key(&helper_instance));
}

#[test]
fn prune_unreachable_instances_does_not_map_callable_var_by_name() {
    let main_id = DefId::new(CrateId(0), LocalDefId(122));
    let helper_id = DefId::new(CrateId(0), LocalDefId(123));
    let main_instance = InstanceId(0);
    let helper_instance = InstanceId(1);
    let main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Var("helper".to_string()),
            ty: Type::function(Vec::new(), Type::Unit),
            span: Span::test(),
        },
    );
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(main_body),
            false,
        ),
        instance_record(
            helper_instance,
            InstanceOrigin::Function(helper_id),
            "helper",
            "helper",
            Some(unit_return_function(helper_id, "helper")),
            false,
        ),
    ]);

    prune_unreachable_instances(&mut program);

    assert!(!program.instances.contains_key(&helper_instance));
}

#[test]
fn prune_unreachable_instances_does_not_map_callable_var_by_backend_symbol() {
    let main_id = DefId::new(CrateId(0), LocalDefId(124));
    let helper_id = DefId::new(CrateId(0), LocalDefId(125));
    let main_instance = InstanceId(0);
    let helper_instance = InstanceId(1);
    let main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Var("helper_backend".to_string()),
            ty: Type::function(Vec::new(), Type::Unit),
            span: Span::test(),
        },
    );
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(main_body),
            false,
        ),
        instance_record(
            helper_instance,
            InstanceOrigin::Function(helper_id),
            "helper_source",
            "helper_backend",
            Some(unit_return_function(helper_id, "helper_source")),
            false,
        ),
    ]);

    prune_unreachable_instances(&mut program);

    assert!(!program.instances.contains_key(&helper_instance));
}

#[test]
fn prune_unreachable_instances_does_not_map_callable_var_to_declared_alias() {
    let main_id = DefId::new(CrateId(0), LocalDefId(140));
    let impl_id = DefId::new(CrateId(1), LocalDefId(141));
    let method_id = DefId::new(CrateId(1), LocalDefId(142));
    let main_instance = InstanceId(0);
    let method_instance = InstanceId(1);
    let main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::Var("String_from_str".to_string()),
                    ty: Type::function(Vec::new(), Type::Unit),
                    span: Span::test(),
                }),
                Vec::new(),
                None,
            ),
            ty: Type::Unit,
            span: Span::test(),
        },
    );
    let declared = unit_return_function(method_id, "std::from_str");
    let object_method = InstanceRecord {
        id: method_instance,
        origin: InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: method_id,
        },
        substitution: Vec::new(),
        symbols: InstanceSymbols::new("std__String_from_str", "std__String_from_str"),
        declared: Some(declared),
        provided_by_object: true,
        is_specialization: false,
    };
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(main_body),
            false,
        ),
        object_method,
    ]);

    prune_unreachable_instances(&mut program);

    assert!(!program.instances.contains_key(&method_instance));
}

#[test]
fn prune_unreachable_instances_does_not_map_callable_var_to_impl_alias() {
    let main_id = DefId::new(CrateId(0), LocalDefId(150));
    let impl_id = DefId::new(CrateId(1), LocalDefId(151));
    let method_id = DefId::new(CrateId(1), LocalDefId(152));
    let main_instance = InstanceId(0);
    let method_instance = InstanceId(1);
    let main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::Var("String_from_str".to_string()),
                    ty: Type::function(Vec::new(), Type::Unit),
                    span: Span::test(),
                }),
                Vec::new(),
                None,
            ),
            ty: Type::Unit,
            span: Span::test(),
        },
    );
    let object_method = InstanceRecord {
        id: method_instance,
        origin: InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: method_id,
        },
        substitution: Vec::new(),
        symbols: InstanceSymbols::new("std__String_from_str", "std__String_from_str"),
        declared: Some(unit_return_function(method_id, "std::from_str")),
        provided_by_object: true,
        is_specialization: false,
    };
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(main_body),
            false,
        ),
        object_method,
    ]);
    program.program.impls.insert(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("String".to_string()),
            type_name: "String".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "from_str".to_string(),
                unit_return_function(method_id, "std::from_str"),
            )]),
        },
    );

    prune_unreachable_instances(&mut program);

    assert!(!program.instances.contains_key(&method_instance));
}

#[test]
fn prune_unreachable_instances_does_not_map_field_access_to_receiver_method_alias() {
    let main_id = DefId::new(CrateId(0), LocalDefId(170));
    let struct_id = DefId::new(CrateId(0), LocalDefId(171));
    let impl_id = DefId::new(CrateId(1), LocalDefId(171));
    let method_id = DefId::new(CrateId(1), LocalDefId(172));
    let field_id = FieldId(0);
    let main_instance = InstanceId(0);
    let method_instance = InstanceId(1);
    let receiver_ty = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let mut main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::FieldAccess(
                        Box::new(HirExpr {
                            kind: HirExprKind::Var("foo".to_string()),
                            ty: receiver_ty.clone(),
                            span: Span::test(),
                        }),
                        "println".to_string(),
                        Some(callable_field_location(struct_id, field_id, "println")),
                    ),
                    ty: Type::function(Vec::new(), Type::Unit),
                    span: Span::test(),
                }),
                Vec::new(),
                None,
            ),
            ty: Type::Unit,
            span: Span::test(),
        },
    );
    add_param(&mut main_body, "foo", receiver_ty);
    let mut method = unit_return_function(method_id, "I64_println");
    method.is_method = true;
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(main_body),
            false,
        ),
        instance_record(
            method_instance,
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(impl_id),
                method: method_id,
            },
            "I64_println",
            "I64_println",
            Some(method),
            false,
        ),
    ]);
    program.program.structs.insert(
        struct_id,
        callable_field_struct(struct_id, "Printer", field_id, "println"),
    );

    prune_unreachable_instances(&mut program);

    assert!(!program.instances.contains_key(&method_instance));
}

#[test]
fn prune_unreachable_instances_does_not_map_field_access_to_method_alias_without_target() {
    let main_id = DefId::new(CrateId(0), LocalDefId(174));
    let struct_id = DefId::new(CrateId(0), LocalDefId(177));
    let impl_id = DefId::new(CrateId(1), LocalDefId(175));
    let method_id = DefId::new(CrateId(1), LocalDefId(176));
    let field_id = FieldId(0);
    let main_instance = InstanceId(0);
    let method_instance = InstanceId(1);
    let receiver_ty = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let mut main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::FieldAccess(
                        Box::new(HirExpr {
                            kind: HirExprKind::Var("foo".to_string()),
                            ty: receiver_ty.clone(),
                            span: Span::test(),
                        }),
                        "println".to_string(),
                        Some(callable_field_location(struct_id, field_id, "println")),
                    ),
                    ty: Type::function(Vec::new(), Type::Unit),
                    span: Span::test(),
                }),
                Vec::new(),
                None,
            ),
            ty: Type::Unit,
            span: Span::test(),
        },
    );
    add_param(&mut main_body, "foo", receiver_ty);
    let mut method = unit_return_function(method_id, "I64_println");
    method.is_method = true;
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(main_body),
            false,
        ),
        instance_record(
            method_instance,
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(impl_id),
                method: method_id,
            },
            "I64_println",
            "I64_println",
            Some(method),
            false,
        ),
    ]);
    program.program.structs.insert(
        struct_id,
        callable_field_struct(struct_id, "Printer", field_id, "println"),
    );

    prune_unreachable_instances(&mut program);

    assert!(!program.instances.contains_key(&method_instance));
}

#[test]
fn prune_unreachable_instances_does_not_map_field_access_to_short_nominal_method_alias() {
    let main_id = DefId::new(CrateId(0), LocalDefId(180));
    let struct_id = DefId::new(CrateId(0), LocalDefId(181));
    let method_id = DefId::new(CrateId(0), LocalDefId(182));
    let field_id = FieldId(0);
    let main_instance = InstanceId(0);
    let method_instance = InstanceId(1);
    let receiver_ty = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let mut main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::FieldAccess(
                        Box::new(HirExpr {
                            kind: HirExprKind::Var("foo".to_string()),
                            ty: receiver_ty.clone(),
                            span: Span::test(),
                        }),
                        "show".to_string(),
                        Some(callable_field_location(struct_id, field_id, "show")),
                    ),
                    ty: Type::function(Vec::new(), Type::Unit),
                    span: Span::test(),
                }),
                Vec::new(),
                None,
            ),
            ty: Type::Unit,
            span: Span::test(),
        },
    );
    add_param(&mut main_body, "foo", receiver_ty);
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(main_body),
            false,
        ),
        instance_record(
            method_instance,
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(DefId::new(CrateId(0), LocalDefId(183))),
                method: method_id,
            },
            "Foo_show",
            "Foo_show",
            Some(unit_return_function(method_id, "Foo_show")),
            false,
        ),
    ]);
    program.program.structs.insert(
        struct_id,
        callable_field_struct(struct_id, "Foo", field_id, "show"),
    );
    program
        .program
        .indexes
        .structs_by_id
        .insert(struct_id, "module::Foo".to_string());

    prune_unreachable_instances(&mut program);

    assert!(!program.instances.contains_key(&method_instance));
}

#[test]
fn prune_unreachable_instances_does_not_map_field_access_to_named_nominal_alias() {
    let main_id = DefId::new(CrateId(0), LocalDefId(190));
    let struct_id = DefId::new(CrateId(0), LocalDefId(191));
    let method_id = DefId::new(CrateId(0), LocalDefId(192));
    let field_id = FieldId(0);
    let main_instance = InstanceId(0);
    let method_instance = InstanceId(1);
    let receiver_ty = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let mut main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::FieldAccess(
                        Box::new(HirExpr {
                            kind: HirExprKind::Var("foo".to_string()),
                            ty: receiver_ty.clone(),
                            span: Span::test(),
                        }),
                        "show".to_string(),
                        Some(callable_field_location(struct_id, field_id, "show")),
                    ),
                    ty: Type::function(Vec::new(), Type::Unit),
                    span: Span::test(),
                }),
                Vec::new(),
                None,
            ),
            ty: Type::Unit,
            span: Span::test(),
        },
    );
    add_param(&mut main_body, "foo", receiver_ty);
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(main_body),
            false,
        ),
        instance_record(
            method_instance,
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(DefId::new(CrateId(0), LocalDefId(193))),
                method: method_id,
            },
            "AliasFoo_show",
            "AliasFoo_show",
            Some(unit_return_function(method_id, "AliasFoo_show")),
            false,
        ),
    ]);
    program.program.structs.insert(
        struct_id,
        callable_field_struct(struct_id, "Foo", field_id, "show"),
    );
    program
        .program
        .indexes
        .structs_by_id
        .insert(struct_id, "module::Foo".to_string());
    program
        .program
        .names
        .structs_by_name
        .insert("AliasFoo".to_string(), struct_id);

    prune_unreachable_instances(&mut program);

    assert!(!program.instances.contains_key(&method_instance));
}
