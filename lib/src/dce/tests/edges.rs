use super::*;

#[test]
fn prune_unreachable_instances_follows_direct_instance_call_edge() {
    let main_id = DefId::new(CrateId(0), LocalDefId(20));
    let helper_id = DefId::new(CrateId(0), LocalDefId(21));
    let main_instance = InstanceId(0);
    let helper_instance = InstanceId(1);
    let main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Call(
                Box::new(instance_ref_expr("helper", helper_instance)),
                Vec::new(),
                Some(HirCallTarget::Instance(helper_instance)),
            ),
            ty: Type::Unit,
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

    let report = prune_unreachable_instances(&mut program);

    assert_eq!(report.retained_instances, 2);
    assert!(program.instances.contains_key(&main_instance));
    assert!(program.instances.contains_key(&helper_instance));
}

#[test]
fn prune_unreachable_instances_follows_function_value_instance_edge() {
    let main_id = DefId::new(CrateId(0), LocalDefId(30));
    let helper_id = DefId::new(CrateId(0), LocalDefId(31));
    let main_instance = InstanceId(0);
    let helper_instance = InstanceId(1);
    let main_body = function_returning_expr(
        main_id,
        "main",
        instance_ref_expr("helper", helper_instance),
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
fn prune_unreachable_instances_records_missing_instance_edges_without_name_fallback() {
    let main_id = DefId::new(CrateId(0), LocalDefId(40));
    let real_helper_id = DefId::new(CrateId(0), LocalDefId(41));
    let main_instance = InstanceId(0);
    let real_helper_instance = InstanceId(1);
    let missing_instance = InstanceId(99);
    let main_body = function_returning_expr(
        main_id,
        "main",
        instance_ref_expr("real_helper", missing_instance),
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
            real_helper_instance,
            InstanceOrigin::Function(real_helper_id),
            "real_helper",
            "real_helper",
            Some(unit_return_function(real_helper_id, "real_helper")),
            false,
        ),
    ]);

    let report = prune_unreachable_instances(&mut program);

    assert_eq!(report.missing_instance_edges, vec![missing_instance]);
    assert!(!program.instances.contains_key(&real_helper_instance));
}

#[test]
fn prune_unreachable_instances_keeps_reachable_generic_specialization() {
    let main_id = DefId::new(CrateId(0), LocalDefId(50));
    let identity_id = DefId::new(CrateId(0), LocalDefId(51));
    let main_instance = InstanceId(0);
    let specialization = InstanceId(1);
    let main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Call(
                Box::new(instance_ref_expr("identity", specialization)),
                vec![HirExpr {
                    kind: HirExprKind::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                }],
                Some(HirCallTarget::Instance(specialization)),
            ),
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
        specialized_instance_record(
            specialization,
            InstanceOrigin::Function(identity_id),
            "identity",
            "identity_mono_0",
            Some(unit_return_function(identity_id, "identity")),
        ),
    ]);

    prune_unreachable_instances(&mut program);

    assert!(program.instances.contains_key(&specialization));
}

#[test]
fn prune_unreachable_instances_removes_unused_generic_specialization() {
    let main_id = DefId::new(CrateId(0), LocalDefId(60));
    let identity_id = DefId::new(CrateId(0), LocalDefId(61));
    let main_instance = InstanceId(0);
    let specialization = InstanceId(1);
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(unit_return_function(main_id, "main")),
            false,
        ),
        specialized_instance_record(
            specialization,
            InstanceOrigin::Function(identity_id),
            "identity",
            "identity_mono_0",
            Some(unit_return_function(identity_id, "identity")),
        ),
    ]);

    prune_unreachable_instances(&mut program);

    assert!(!program.instances.contains_key(&specialization));
}

#[test]
fn prune_unreachable_instances_does_not_keep_closure_edges_from_unreachable_owner() {
    let main_id = DefId::new(CrateId(0), LocalDefId(62));
    let owner_id = DefId::new(CrateId(0), LocalDefId(63));
    let callee_id = DefId::new(CrateId(0), LocalDefId(64));
    let main_instance = InstanceId(0);
    let owner_instance = InstanceId(1);
    let callee_instance = InstanceId(2);
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(unit_return_function(main_id, "main")),
            false,
        ),
        instance_record(
            owner_instance,
            InstanceOrigin::Function(owner_id),
            "unused_owner",
            "unused_owner",
            Some(unit_return_function(owner_id, "unused_owner")),
            false,
        ),
        instance_record(
            callee_instance,
            InstanceOrigin::Function(callee_id),
            "closure_callee",
            "closure_callee",
            Some(unit_return_function(callee_id, "closure_callee")),
            false,
        ),
    ]);
    let unit_id = program.type_context.intern_type(&Type::Unit);
    let mut bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut program);
    let closure_id = MirFunctionId::Closure(Box::new(MirClosureId {
        owner: MirFunctionId::Instance(owner_instance),
        local_index: 0,
    }));
    bodies.push_nested(MirFunction {
        id: closure_id.clone(),
        name: "unused_owner_closure".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Instance(callee_instance),
                    ))),
                    args: Vec::new(),
                    destination: Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    target: BasicBlockId(1),
                    span: Some(crate::lexer::Span::test()),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![LocalDecl {
            ty: unit_id,
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit_id,
        ownership: Default::default(),
    });

    super::super::prune_unreachable_instances(&mut program, &mut bodies);

    assert!(program.instances.contains_key(&main_instance));
    assert!(!program.instances.contains_key(&owner_instance));
    assert!(!program.instances.contains_key(&callee_instance));
    assert!(!bodies
        .nested_functions()
        .iter()
        .any(|function| function.id == closure_id));

    let mir = crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(
        &program, &bodies,
    );
    assert!(!mir.functions.contains_key(&closure_id));
    assert!(!mir
        .functions
        .contains_key(&MirFunctionId::Instance(callee_instance)));
}

#[test]
fn prune_unreachable_instances_keeps_drop_impl_for_automatic_drop() {
    let main_id = DefId::new(CrateId(0), LocalDefId(170));
    let drop_trait_id = DefId::new(CrateId(0), LocalDefId(171));
    let drop_trait_method_id = DefId::new(CrateId(0), LocalDefId(172));
    let impl_id = DefId::new(CrateId(0), LocalDefId(173));
    let drop_method_id = DefId::new(CrateId(0), LocalDefId(174));
    let main_instance = InstanceId(0);
    let drop_instance = InstanceId(1);
    let drop_method = unit_return_function(drop_method_id, "Box_drop");
    let mut program = monomorphized_with_instances(vec![
        instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(unit_return_function(main_id, "main")),
            false,
        ),
        instance_record(
            drop_instance,
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(impl_id),
                method: drop_method_id,
            },
            "Box_drop",
            "Box_drop",
            Some(drop_method.clone()),
            false,
        ),
    ]);
    program.program.traits.insert(
        drop_trait_id,
        crate::hir::HirTraitFor::<AcceptedHir> {
            target: None,
            predicates: Vec::new(),
            id: drop_trait_id,
            name: "Drop".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "drop".to_string(),
                crate::hir::HirFunctionSig {
                    id: drop_trait_method_id,
                    name: "drop".to_string(),
                    generic_params: Vec::new(),
                    params: Vec::new(),
                    ret: Type::Unit,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(crate::types::ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        },
    );
    program.program.impls.insert(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: Some("Drop".to_string()),
            trait_id: Some(drop_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("drop".to_string(), drop_method)]),
        },
    );
    program
        .program
        .rebuild_indexes_with_canonical_names(&HashMap::from([
            (main_id, "main".to_string()),
            (drop_trait_id, "std::drop::Drop".to_string()),
        ]));

    let mut bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut program);
    bodies.insert_runtime_instance_root(drop_instance);
    super::super::prune_unreachable_instances(&mut program, &mut bodies);

    assert!(program.instances.contains_key(&drop_instance));
}
