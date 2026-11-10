use super::*;

#[test]
fn borrowck_type_queries_use_type_view() {
    let mut context = crate::type_context::TypeContext::new();
    let i64_id = context.intern_type(&Type::I64);
    let mut_ref = context.intern_ty(crate::type_context::Ty::Reference {
        mutable: true,
        inner: i64_id,
    });
    let pointer = context.intern_ty(crate::type_context::Ty::Pointer(i64_id));
    let view = crate::type_context::TypeView::new(&context);

    assert!(type_id_is_mut_reference(view, mut_ref));
    assert!(type_id_is_pointer(view, pointer));
    assert!(!type_id_is_pointer(view, mut_ref));
}

#[test]
fn concrete_generic_enum_fields_do_not_inherit_generic_reference_facts() {
    let mut type_context = TypeContext::new();
    let result_id = DefId::new(CrateId(0), LocalDefId(601));
    let t_param = GenericParamId {
        owner: result_id,
        index: 0,
    };
    let e_param = GenericParamId {
        owner: result_id,
        index: 1,
    };
    let generic_t = test_type_id(&mut type_context, Type::Generic(t_param));
    let generic_e = test_type_id(&mut type_context, Type::Generic(e_param));
    let concrete_result = test_type_id(
        &mut type_context,
        Type::Enum {
            id: result_id,
            args: vec![Type::I64, Type::I32],
        },
    );
    let reference_result = test_type_id(
        &mut type_context,
        Type::Enum {
            id: result_id,
            args: vec![
                Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::I64),
                },
                Type::I32,
            ],
        },
    );
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        result_id,
        MirNominalLayout::Enum {
            id: result_id,
            variants: vec![
                MirEnumVariantLayout {
                    name: "Ok".to_string(),
                    fields: MirVariantLayoutFields::Positional(vec![generic_t]),
                },
                MirEnumVariantLayout {
                    name: "Err".to_string(),
                    fields: MirVariantLayoutFields::Positional(vec![generic_e]),
                },
            ],
            generic_params: vec![t_param, e_param],
        },
    );
    let view = TypeView::new(&type_context);

    assert!(!type_id_contains_reference(
        view,
        &backend_contract,
        concrete_result
    ));
    assert!(!type_id_tracks_origin(
        view,
        &backend_contract,
        concrete_result
    ));
    assert!(type_id_contains_reference(
        view,
        &backend_contract,
        reference_result
    ));
    assert!(type_id_tracks_origin(
        view,
        &backend_contract,
        reference_result
    ));
}

#[test]
fn borrowck_rejects_local_escape_through_contract_projection_output() {
    let mut type_context = TypeContext::new();
    let trait_id = DefId::new(CrateId(0), LocalDefId(701));
    let assoc_type_id = AssocTypeId(0);
    let base_id = test_type_id(&mut type_context, Type::I64);
    let ref_i64 = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };
    let ref_i64_id = test_type_id(&mut type_context, ref_i64);
    let projection_id = test_type_id(
        &mut type_context,
        Type::Projection {
            ty: Box::new(Type::I64),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: Vec::new(),
        },
    );
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(702))),
        name: "projection_reference_return".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
                    Place {
                        local: Local(1),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Constant(Constant::Int(1))),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Ref(
                        Mutability::Not,
                        Place {
                            local: Local(1),
                            projection: Vec::new(),
                        },
                    ),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: projection_id,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: base_id,
                mutability: Mutability::Not,
                name: Some("local".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: projection_id,
        ownership: MirOwnershipMetadata::default(),
    };
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: base_id,
            trait_id,
            assoc_type_id,
            trait_args: Vec::new(),
        },
        ref_i64_id,
    );

    let err = BorrowChecker::check_function_with_contract(
        &function,
        &type_context,
        &backend_contract,
        &HashMap::new(),
        &Default::default(),
    )
    .expect_err("projection output reference should not allow a local to escape");

    assert!(err.0.iter().any(|diag| diag
        .message
        .contains("Cannot return reference to local value")));
}

#[test]
fn borrowck_field_projection_uses_contract_not_legacy_metadata() {
    let mut type_context = TypeContext::new();
    let outer_def = DefId::new(CrateId(0), LocalDefId(901));
    let inner_def = DefId::new(CrateId(0), LocalDefId(902));
    let outer_ty = Type::Struct {
        id: outer_def,
        args: Vec::new(),
    };
    let inner_ty = Type::Struct {
        id: inner_def,
        args: Vec::new(),
    };
    let inner_id = type_context.intern_type(&inner_ty);
    let outer_ref_id = type_context.intern_type(&Type::Reference {
        mutable: false,
        inner: Box::new(outer_ty.clone()),
    });
    let function = move_or_copy_out_of_reference_function(
        inner_id,
        outer_ref_id,
        Operand::Move(Place {
            local: Local(1),
            projection: vec![
                Projection::Deref,
                Projection::Field {
                    index: 0,
                    identity: None,
                },
            ],
        }),
    );
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        outer_def,
        MirNominalLayout::Struct {
            id: outer_def,
            fields: vec![("field".to_string(), inner_id)],
            generic_params: Vec::new(),
        },
    );
    backend_contract.nominal_layouts.insert(
        inner_def,
        MirNominalLayout::Struct {
            id: inner_def,
            fields: Vec::new(),
            generic_params: Vec::new(),
        },
    );

    let err = BorrowChecker::check_function_with_contract(
        &function,
        &type_context,
        &backend_contract,
        &HashMap::new(),
        &Default::default(),
    )
    .expect_err("contract field type should make the projected move non-copy");

    assert!(err
        .0
        .iter()
        .any(|diag| diag.message.contains("Cannot move non-copy value")));
}

#[test]
fn borrowck_rejects_field_move_from_projected_drop_struct_output() {
    let mut type_context = TypeContext::new();
    let trait_id = DefId::new(CrateId(0), LocalDefId(920));
    let assoc_type_id = AssocTypeId(0);
    let drop_def = DefId::new(CrateId(0), LocalDefId(921));
    let base_id = type_context.intern_type(&Type::I64);
    let field_id = type_context.intern_type(&Type::I64);
    let drop_ty = Type::Struct {
        id: drop_def,
        args: Vec::new(),
    };
    let drop_id = type_context.intern_type(&drop_ty);
    let projection_id = type_context.intern_type(&Type::Projection {
        ty: Box::new(Type::I64),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: Vec::new(),
    });
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(922))),
        name: "projected_drop_struct_field_move".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(2),
                    projection: Vec::new(),
                },
                Rvalue::Use(Operand::Move(Place {
                    local: Local(1),
                    projection: vec![Projection::Field {
                        index: 0,
                        identity: None,
                    }],
                })),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: field_id,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: projection_id,
                mutability: Mutability::Not,
                name: Some("projected".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            LocalDecl {
                ty: field_id,
                mutability: Mutability::Mut,
                name: Some("field".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: field_id,
        ownership: MirOwnershipMetadata::default(),
    };
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: base_id,
            trait_id,
            assoc_type_id,
            trait_args: Vec::new(),
        },
        drop_id,
    );
    backend_contract.drop_glue.insert(
        drop_id,
        crate::mir::MirCallableKey::Function(DefId::new(CrateId(0), LocalDefId(923))),
    );
    backend_contract.nominal_layouts.insert(
        drop_def,
        MirNominalLayout::Struct {
            id: drop_def,
            fields: vec![("value".to_string(), field_id)],
            generic_params: Vec::new(),
        },
    );

    let err = BorrowChecker::check_function_with_contract(
        &function,
        &type_context,
        &backend_contract,
        &HashMap::new(),
        &Default::default(),
    )
    .expect_err("moving a field out of projected Drop output should be rejected");

    assert!(err
        .0
        .iter()
        .any(|diag| diag.message.contains("because it implements Drop")));
}

#[test]
fn borrowck_rejects_array_element_move_from_projected_drop_array_output() {
    let mut type_context = TypeContext::new();
    let trait_id = DefId::new(CrateId(0), LocalDefId(924));
    let assoc_type_id = AssocTypeId(0);
    let drop_def = DefId::new(CrateId(0), LocalDefId(925));
    let base_id = type_context.intern_type(&Type::I64);
    let i64_id = type_context.intern_type(&Type::I64);
    let drop_ty = Type::Struct {
        id: drop_def,
        args: Vec::new(),
    };
    let drop_id = type_context.intern_type(&drop_ty);
    let array_id = type_context.intern_type(&Type::Array(Box::new(drop_ty), crate::types::ArrayLen::Value(2)));
    let projection_id = type_context.intern_type(&Type::Projection {
        ty: Box::new(Type::I64),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: Vec::new(),
    });
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(926))),
        name: "projected_drop_array_element_move".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(2),
                    projection: Vec::new(),
                },
                Rvalue::Use(Operand::Move(Place {
                    local: Local(1),
                    projection: vec![Projection::Index(Local(3))],
                })),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: drop_id,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: projection_id,
                mutability: Mutability::Not,
                name: Some("array".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            LocalDecl {
                ty: drop_id,
                mutability: Mutability::Mut,
                name: Some("element".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Not,
                name: Some("index".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: drop_id,
        ownership: MirOwnershipMetadata::default(),
    };
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: base_id,
            trait_id,
            assoc_type_id,
            trait_args: Vec::new(),
        },
        array_id,
    );
    backend_contract.drop_glue.insert(
        drop_id,
        crate::mir::MirCallableKey::Function(DefId::new(CrateId(0), LocalDefId(927))),
    );
    backend_contract.nominal_layouts.insert(
        drop_def,
        MirNominalLayout::Struct {
            id: drop_def,
            fields: Vec::new(),
            generic_params: Vec::new(),
        },
    );

    let err = BorrowChecker::check_function_with_contract(
        &function,
        &type_context,
        &backend_contract,
        &HashMap::new(),
        &Default::default(),
    )
    .expect_err("moving projected Drop array element should be rejected");

    assert!(err.0.iter().any(|diag| diag
        .message
        .contains("Cannot move cleanup value out of array element")));
}

#[test]
fn borrowck_rejects_move_out_of_projected_reference_output() {
    let mut type_context = TypeContext::new();
    let trait_id = DefId::new(CrateId(0), LocalDefId(928));
    let assoc_type_id = AssocTypeId(0);
    let payload_def = DefId::new(CrateId(0), LocalDefId(929));
    let base_id = type_context.intern_type(&Type::I64);
    let payload_ty = Type::Struct {
        id: payload_def,
        args: Vec::new(),
    };
    let payload_id = type_context.intern_type(&payload_ty);
    let projection_output_id = type_context.intern_type(&Type::Reference {
        mutable: false,
        inner: Box::new(payload_ty.clone()),
    });
    let projection_id = type_context.intern_type(&Type::Projection {
        ty: Box::new(Type::I64),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: Vec::new(),
    });
    let function = move_or_copy_out_of_reference_function(
        payload_id,
        projection_id,
        Operand::Move(Place {
            local: Local(1),
            projection: vec![Projection::Deref],
        }),
    );
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: base_id,
            trait_id,
            assoc_type_id,
            trait_args: Vec::new(),
        },
        projection_output_id,
    );
    backend_contract.nominal_layouts.insert(
        payload_def,
        MirNominalLayout::Struct {
            id: payload_def,
            fields: vec![("value".to_string(), base_id)],
            generic_params: Vec::new(),
        },
    );

    let err = BorrowChecker::check_function_with_contract(
        &function,
        &type_context,
        &backend_contract,
        &HashMap::new(),
        &Default::default(),
    )
    .expect_err("moving non-copy projected reference output should be rejected");

    assert!(err
        .0
        .iter()
        .any(|diag| diag.message.contains("Cannot move non-copy value of type")));
}

#[test]
fn borrowck_allows_raw_pointer_move_after_reading_pointer_through_reference() {
    let mut type_context = TypeContext::new();
    let pair_def = DefId::new(CrateId(0), LocalDefId(133));
    let pair_ty = Type::Struct {
        id: pair_def,
        args: Vec::new(),
    };
    let pair_id = type_context.intern_type(&pair_ty);
    let ref_ptr_pair_id = type_context.intern_type(&Type::Reference {
        mutable: false,
        inner: Box::new(Type::Pointer(Box::new(pair_ty))),
    });
    let function = move_or_copy_out_of_reference_function(
        pair_id,
        ref_ptr_pair_id,
        Operand::Move(Place {
            local: Local(1),
            projection: vec![Projection::Deref, Projection::Deref],
        }),
    );

    BorrowChecker::check_function(&function, &type_context)
        .expect("raw-pointer pointee moves are not moves out of the reference");
}
