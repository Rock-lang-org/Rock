use super::*;

#[test]
fn prune_unreachable_instances_keeps_main_instance() {
    let main_id = DefId::new(CrateId(0), LocalDefId(10));
    let main_instance = InstanceId(0);
    let mut program = monomorphized_with_instances(vec![instance_record(
        main_instance,
        InstanceOrigin::Function(main_id),
        "main",
        "main",
        Some(unit_return_function(main_id, "main")),
        false,
    )]);

    let report = prune_unreachable_instances(&mut program);

    assert_eq!(report.retained_instances, 1);
    assert_eq!(report.removed_instances, 0);
    assert!(program.instances.contains_key(&main_instance));
}

#[test]
fn prune_unreachable_instances_without_main_keeps_instances_for_artifacts() {
    let library_id = DefId::new(CrateId(0), LocalDefId(130));
    let library_instance = InstanceId(0);
    let mut program = monomorphized_with_instances(vec![instance_record(
        library_instance,
        InstanceOrigin::Function(library_id),
        "library_export",
        "library_export",
        Some(unit_return_function(library_id, "library_export")),
        false,
    )]);

    let report = prune_unreachable_instances(&mut program);

    assert_eq!(report.retained_instances, 1);
    assert_eq!(report.removed_instances, 0);
    assert!(program.instances.contains_key(&library_instance));
}

#[test]
fn prune_unreachable_instances_removes_unreachable_function_instance() {
    let main_id = DefId::new(CrateId(0), LocalDefId(10));
    let unused_id = DefId::new(CrateId(0), LocalDefId(11));
    let main_instance = InstanceId(0);
    let unused_instance = InstanceId(1);
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
            unused_instance,
            InstanceOrigin::Function(unused_id),
            "unused",
            "unused",
            Some(unit_return_function(unused_id, "unused")),
            false,
        ),
    ]);

    let report = prune_unreachable_instances(&mut program);

    assert_eq!(report.retained_instances, 1);
    assert_eq!(report.removed_instances, 1);
    assert!(program.instances.contains_key(&main_instance));
    assert!(!program.instances.contains_key(&unused_instance));
}

#[test]
fn prune_unreachable_instances_does_not_root_by_backend_symbol() {
    let main_id = DefId::new(CrateId(0), LocalDefId(12));
    let helper_id = DefId::new(CrateId(0), LocalDefId(13));
    let main_instance = InstanceId(0);
    let helper_instance = InstanceId(1);
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
            helper_instance,
            InstanceOrigin::Function(helper_id),
            "helper",
            "main",
            Some(unit_return_function(helper_id, "helper")),
            false,
        ),
    ]);

    prune_unreachable_instances(&mut program);

    assert!(program.instances.contains_key(&main_instance));
    assert!(!program.instances.contains_key(&helper_instance));
}

#[test]
fn prune_unreachable_instances_keeps_explicit_runtime_instance_root() {
    let main_id = DefId::new(CrateId(0), LocalDefId(65));
    let runtime_id = DefId::new(CrateId(0), LocalDefId(66));
    let main_instance = InstanceId(0);
    let runtime_instance = InstanceId(1);
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
            runtime_instance,
            InstanceOrigin::Function(runtime_id),
            "not_drop",
            "not_drop_backend",
            Some(unit_return_function(runtime_id, "not_drop")),
            false,
        ),
    ]);
    let mut bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut program);
    bodies.insert_runtime_instance_root(runtime_instance);

    super::super::prune_unreachable_instances(&mut program, &mut bodies);

    assert!(program.instances.contains_key(&runtime_instance));
}

#[test]
fn prune_unreachable_instances_keeps_reachable_object_backed_instance() {
    let main_id = DefId::new(CrateId(0), LocalDefId(70));
    let dep_id = DefId::new(CrateId(1), LocalDefId(71));
    let main_instance = InstanceId(0);
    let object_instance = InstanceId(1);
    let main_body = function_returning_expr(
        main_id,
        "main",
        HirExpr {
            kind: HirExprKind::Call(
                Box::new(instance_ref_expr("dep::print", object_instance)),
                Vec::new(),
                Some(HirCallTarget::Instance(object_instance)),
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
            object_instance,
            InstanceOrigin::Function(dep_id),
            "dep::print",
            "dep__print",
            Some(unit_return_function(dep_id, "dep::print")),
            true,
        ),
    ]);

    prune_unreachable_instances(&mut program);

    assert!(program.instances.contains_key(&object_instance));
}

#[test]
fn prune_unreachable_instances_removes_unreachable_object_backed_instance() {
    let main_id = DefId::new(CrateId(0), LocalDefId(80));
    let dep_id = DefId::new(CrateId(1), LocalDefId(81));
    let main_instance = InstanceId(0);
    let object_instance = InstanceId(1);
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
            object_instance,
            InstanceOrigin::Function(dep_id),
            "dep::print",
            "dep__print",
            Some(unit_return_function(dep_id, "dep::print")),
            true,
        ),
    ]);

    prune_unreachable_instances(&mut program);

    assert!(!program.instances.contains_key(&object_instance));
}

#[test]
fn prune_unreachable_instances_roots_main_without_display_name_table() {
    let main_id = DefId::new(CrateId(0), LocalDefId(105));
    let helper_id = DefId::new(CrateId(0), LocalDefId(106));
    let main_instance = InstanceId(0);
    let helper_instance = InstanceId(1);
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
            helper_instance,
            InstanceOrigin::Function(helper_id),
            "helper",
            "helper",
            Some(unit_return_function(helper_id, "helper")),
            false,
        ),
    ]);
    program.program.names.functions_by_name.clear();

    prune_unreachable_instances(&mut program);

    assert!(program.instances.contains_key(&main_instance));
    assert!(!program.instances.contains_key(&helper_instance));
}
