use super::*;

#[test]
fn needs_drop_includes_enums_and_closures() {
    let enum_ty = Type::Enum {
        id: DefId::new(CrateId(0), LocalDefId(60)),
        args: vec![],
    };
    assert!(MirBuilder::needs_drop(&enum_ty));
    assert!(MirBuilder::needs_drop(&Type::function(vec![], Type::Unit)));
}

#[test]
fn drop_in_place_lowers_to_mir_drop_terminator() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let pointee_ty = struct_ty(0);
    let ptr_ty = Type::Pointer(Box::new(pointee_ty.clone()));
    let mut builder = builder_with_var(&program, &type_context, "ptr", ptr_ty.clone());
    builder.direct_drop_types.insert(pointee_ty.clone());
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::Unit),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });

    let intrinsic_expr = expr(
        HirExprKind::Intrinsic {
            name: "DropInPlace".to_string(),
            args: vec![expr(HirExprKind::Var("ptr".to_string()), ptr_ty)],
        },
        Type::Unit,
    );

    builder.lower_expr_with_context(
        &intrinsic_expr,
        Place {
            local: Local(1),
            projection: vec![],
        },
        false,
    );

    assert!(matches!(
        &builder.blocks[0].terminator,
        Some(Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. })
            if place.local == Local(0) && place.projection == vec![Projection::Deref]
    ));

    assert_builder_agreement_clean(builder, Type::Unit);
}

#[test]
fn cleanup_drops_before_storage_dead_for_drop_needed_local() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.direct_drop_types.insert(struct_ty(0));
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&struct_ty(0)),
        mutability: Mutability::Not,
        name: Some("value".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });

    builder.finish_scope_locals(vec![Local(0)]);

    let drop_target = match &builder.blocks[0].terminator {
        Some(
            Terminator::Drop { place, target, .. }
            | Terminator::DropWithOrigin { place, target, .. },
        ) if place.local == Local(0) => *target,
        other => panic!("expected drop terminator before storage dead, got {other:?}"),
    };
    assert!(!builder.blocks[0]
        .statements
        .iter()
        .any(|stmt| matches!(&stmt.kind, StatementKind::StorageDead(Local(0)))));
    assert!(builder.blocks[drop_target.0]
        .statements
        .iter()
        .any(|stmt| matches!(&stmt.kind, StatementKind::StorageDead(Local(0)))));
}

#[test]
fn recursive_field_drop_emits_root_drop_before_field_drops() {
    let inner_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(1));
    let outer_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0));
    let drop_trait_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(2));
    let drop_trait_method_id =
        crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(3));
    let outer_drop_impl_id =
        crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(4));
    let outer_drop_method_id =
        crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(5));
    let inner_drop_impl_id =
        crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(6));
    let inner_drop_method_id =
        crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(7));
    let outer_ty = Type::Struct {
        id: outer_id,
        args: vec![],
    };
    let inner_ty = Type::Struct {
        id: inner_id,
        args: vec![],
    };
    let drop_function = |id, name: &str, ty: Type| {
        let mut function = empty_function(id, name);
        function.params.push(HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty,
            mutable: false,
            is_ref: false,
        });
        function.is_method = true;
        function.self_receiver = Some(crate::types::ReceiverMode::Move);
        function
    };

    let mut structs = std::collections::HashMap::new();
    structs.insert(
        inner_id,
        HirStruct {
            id: inner_id,
            name: "Inner".to_string(),
            generic_params: vec![],
            fields: vec![HirField {
                id: crate::ids::FieldId(0),
                name: "x".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );
    structs.insert(
        outer_id,
        HirStruct {
            id: outer_id,
            name: "Outer".to_string(),
            generic_params: vec![],
            fields: vec![HirField {
                id: crate::ids::FieldId(0),
                name: "inner".to_string(),
                ty: inner_ty.clone(),
                public: true,
            }],
        },
    );
    let mut program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        structs,
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(
            drop_trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: drop_trait_id,
                name: "Drop".to_string(),
                generic_params: vec![],
                associated_types: vec![],
                methods: std::collections::HashMap::from([(
                    "drop".to_string(),
                    empty_function(drop_trait_method_id, "drop"),
                )]),
                signatures: std::collections::HashMap::new(),
            },
        )]),
        std::collections::HashMap::from([
            (
                outer_drop_impl_id,
                HirImpl {
                    id: outer_drop_impl_id,
                    owner: HirImplOwner::Named("Outer".to_string()),
                    type_name: "Outer".to_string(),
                    type_generics: vec![],
                    receiver_pattern: vec![].into(),
                    trait_name: Some("Drop".to_string()),
                    trait_id: Some(drop_trait_id),
                    trait_generics: vec![],
                    trait_arg_types: vec![],
                    associated_types: vec![],
                    bounds: std::collections::HashMap::new().into(),
                    methods: std::collections::HashMap::from([(
                        "drop".to_string(),
                        drop_function(outer_drop_method_id, "drop", outer_ty.clone()),
                    )]),
                },
            ),
            (
                inner_drop_impl_id,
                HirImpl {
                    id: inner_drop_impl_id,
                    owner: HirImplOwner::Named("Inner".to_string()),
                    type_name: "Inner".to_string(),
                    type_generics: vec![],
                    receiver_pattern: vec![].into(),
                    trait_name: Some("Drop".to_string()),
                    trait_id: Some(drop_trait_id),
                    trait_generics: vec![],
                    trait_arg_types: vec![],
                    associated_types: vec![],
                    bounds: std::collections::HashMap::new().into(),
                    methods: std::collections::HashMap::from([(
                        "drop".to_string(),
                        drop_function(inner_drop_method_id, "drop", inner_ty.clone()),
                    )]),
                },
            ),
        ]),
        std::collections::HashMap::new(),
        HirNameTables {
            structs_by_name: std::collections::HashMap::from([
                ("Inner".to_string(), inner_id),
                ("Outer".to_string(), outer_id),
            ]),
            traits_by_name: std::collections::HashMap::from([(
                "std::drop::Drop".to_string(),
                drop_trait_id,
            )]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([
            (inner_id, "Inner".to_string()),
            (outer_id, "Outer".to_string()),
            (drop_trait_id, "Drop".to_string()),
        ]),
    );
    program.language_items.drop = Some(crate::language_items::DropLanguageItems {
        trait_id: drop_trait_id,
        method_id: drop_trait_method_id,
    });

    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::with_instance_metadata(
        &program,
        &type_context,
        HashMap::new(),
        HashSet::from([outer_ty.clone(), inner_ty.clone()]),
    );
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&outer_ty),
        mutability: Mutability::Not,
        name: Some("value".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });

    builder.finish_scope_locals(vec![Local(0)]);

    let drops: Vec<_> = builder
        .blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            Some(Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. }) => {
                Some(place.clone())
            }
            _ => None,
        })
        .collect();

    assert!(
        drops.len() >= 2,
        "expected at least 2 Drop terminators (field + root), got {}",
        drops.len()
    );

    let inner_place = Place {
        local: Local(0),
        projection: vec![crate::mir::Projection::Field {
            index: 0,
            identity: None,
        }],
    };
    let root_place = Place {
        local: Local(0),
        projection: vec![],
    };

    let inner_drop_index = drops.iter().position(|p| *p == inner_place);
    let root_drop_index = drops.iter().position(|p| *p == root_place);
    assert!(
        inner_drop_index.is_some(),
        "expected Drop for inner field (projection [Field(0)]), found drops: {drops:?}"
    );
    assert!(
        root_drop_index.is_some(),
        "expected Drop for root, found drops: {drops:?}"
    );

    let inner_idx = inner_drop_index.unwrap();
    let root_idx = root_drop_index.unwrap();
    assert!(
        root_idx < inner_idx,
        "expected root Drop ({root_idx}) before field Drop ({inner_idx})"
    );

    assert_builder_agreement_clean(builder, Type::Unit);
}

#[test]
fn test_lower_expr_ref_non_place_base_registers_temp_for_cleanup() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.scope_locals.push(Vec::new());
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        }),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let dest = Place {
        local: Local(0),
        projection: vec![],
    };
    let expr = expr(
        HirExprKind::Ref(false, Box::new(expr(HirExprKind::IntLiteral(1), Type::I64))),
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );

    builder.lower_expr(&expr, dest);
    assert_eq!(builder.scope_locals.last().map(|s| s.len()), Some(1));
}
