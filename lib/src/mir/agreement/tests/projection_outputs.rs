use super::*;

#[test]
fn agreement_validates_contract_projection_output_type_ids() {
    let mut type_context = TypeContext::new();
    let valid_ty = type_id(&mut type_context, Type::I64);
    let invalid_ty = TypeId(999);
    let trait_id = DefId::new(CrateId(0), LocalDefId(41));
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: invalid_ty,
            trait_id,
            assoc_type_id: AssocTypeId(0),
            trait_args: vec![valid_ty, invalid_ty],
        },
        invalid_ty,
    );
    let program = MirProgram {
        functions: std::collections::BTreeMap::new(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 3);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_invalid_projection_output_without_panicking() {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(41)));
    let trait_id = DefId::new(CrateId(0), LocalDefId(42));
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let projection_ty = type_id(
        &mut type_context,
        Type::Projection {
            ty: Box::new(Type::Unit),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![],
        },
    );
    let function = MirFunction {
        id: function_id.clone(),
        name: "invalid_projection_output".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: projection_ty,
                mutability: Mutability::Not,
                name: Some("projected".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: unit,
            trait_id,
            assoc_type_id: AssocTypeId(0),
            trait_args: vec![],
        },
        TypeId(999),
    );
    backend_contract.projection_traits.insert(trait_id);
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert!(report.invalid_type_ids >= 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_function_type_with_missing_projection_output() {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(35)));
    let trait_id = DefId::new(CrateId(0), LocalDefId(36));
    let mut type_context = TypeContext::new();
    let projection = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };
    let projection_id = type_id(&mut type_context, projection);
    let mut function = empty_function(function_id.clone(), &mut type_context);
    function.local_decls.push(LocalDecl {
        ty: projection_id,
        mutability: Mutability::Not,
        name: Some("projected".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_projection_facts, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_missing_projection_outputs_in_cast_and_type_metadata() {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(37)));
    let trait_id = DefId::new(CrateId(0), LocalDefId(38));
    let mut type_context = TypeContext::new();
    let projection = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };
    let projection_id = type_id(&mut type_context, projection);
    let mut function = empty_function(function_id.clone(), &mut type_context);
    function.basic_blocks[0].statements = vec![
        StatementData::assign(
            Place {
                local: Local(0),
                projection: Vec::new(),
            },
            Rvalue::Cast(Operand::Constant(Constant::Unit), projection_id),
            None,
        ),
        StatementData::assert(
            MirAssert {
                kind: MirAssertKind::BoundsCheck,
                operands: vec![
                    Operand::Constant(Constant::TypeId(projection_id)),
                    Operand::Constant(Constant::Int(0)),
                ],
            },
            None,
        ),
    ];
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_projection_facts, 2);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_projection_output_without_contract_trait() {
    let stale_trait = DefId::new(CrateId(0), LocalDefId(35));
    let assoc_type = AssocTypeId(0);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: unit,
            trait_id: stale_trait,
            assoc_type_id: assoc_type,
            trait_args: vec![],
        },
        unit,
    );
    let program = MirProgram {
        functions: Default::default(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.stale_projection_facts, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_stale_contract_projection_output_without_projection_trait() {
    let stale_trait = DefId::new(CrateId(0), LocalDefId(37));
    let assoc_type = AssocTypeId(0);
    let mut type_context = TypeContext::new();
    let valid = type_id(&mut type_context, Type::Unit);
    let invalid = TypeId(999);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: invalid,
            trait_id: stale_trait,
            assoc_type_id: assoc_type,
            trait_args: vec![valid, invalid],
        },
        invalid,
    );
    let program = MirProgram {
        functions: Default::default(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 3);
    assert_eq!(report.stale_projection_facts, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_accepts_contract_projection_output_with_projection_trait() {
    let known_trait = DefId::new(CrateId(0), LocalDefId(39));
    let assoc_type = AssocTypeId(0);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_traits.insert(known_trait);
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: unit,
            trait_id: known_trait,
            assoc_type_id: assoc_type,
            trait_args: vec![],
        },
        unit,
    );
    let program = MirProgram {
        functions: Default::default(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.stale_projection_facts, 0);
    assert!(
        report.is_clean(),
        "unexpected MIR agreement report: {report:?}"
    );
}
