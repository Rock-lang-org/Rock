use super::*;

#[test]
fn direct_drop_obligation_without_glue_is_rejected() {
    let mut type_context = TypeContext::new();
    let tracked_id = test_type_id(&mut type_context, Type::I64);
    let function = drop_obligation_function(
        tracked_id,
        MirOwnershipMetadata {
            reference_origins: Vec::new(),
            temporary_locals: Vec::new(),
            drop_obligations: vec![MirDropObligation {
                place: Place {
                    local: Local(1),
                    projection: Vec::new(),
                },
                ty: tracked_id,
                kind: DropObligationKind::Direct,
            }],
        },
    );

    let err = BorrowChecker::check_function(&function, &type_context)
        .expect_err("direct drop without glue should be rejected");

    assert!(err
        .0
        .iter()
        .any(|diag| diag.message.contains("Missing drop glue")));
}

#[test]
fn direct_drop_obligation_uses_backend_contract_drop_glue() {
    let mut type_context = TypeContext::new();
    let tracked_id = test_type_id(&mut type_context, Type::I64);
    let function = drop_obligation_function(
        tracked_id,
        MirOwnershipMetadata {
            reference_origins: Vec::new(),
            temporary_locals: Vec::new(),
            drop_obligations: vec![MirDropObligation {
                place: Place {
                    local: Local(1),
                    projection: Vec::new(),
                },
                ty: tracked_id,
                kind: DropObligationKind::Direct,
            }],
        },
    );
    let mut backend_contract = MirBackendContract::default();
    backend_contract.drop_glue.insert(
        tracked_id,
        crate::mir::MirCallableKey::Function(DefId::new(CrateId(0), LocalDefId(124))),
    );
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function.id.clone(), function)]),
        type_context,
        backend_contract,
    };

    BorrowChecker::run(&program).expect("contract drop glue should satisfy direct drop obligation");
}

#[test]
fn projected_return_place_direct_drop_obligation_without_glue_is_rejected() {
    let mut type_context = TypeContext::new();
    let tracked_id = test_type_id(&mut type_context, Type::I64);
    let function = drop_obligation_function(
        tracked_id,
        MirOwnershipMetadata {
            reference_origins: Vec::new(),
            temporary_locals: Vec::new(),
            drop_obligations: vec![MirDropObligation {
                place: Place {
                    local: Local(0),
                    projection: vec![Projection::Field {
                        index: 0,
                        identity: None,
                    }],
                },
                ty: tracked_id,
                kind: DropObligationKind::Direct,
            }],
        },
    );

    let err = BorrowChecker::check_function(&function, &type_context)
        .expect_err("projected return-place direct drop without glue should be rejected");

    assert!(err
        .0
        .iter()
        .any(|diag| diag.message.contains("Missing drop glue")));
}

#[test]
fn structural_drop_obligation_without_direct_glue_is_allowed() {
    let mut type_context = TypeContext::new();
    let tracked_id = test_type_id(&mut type_context, Type::I64);
    let function = drop_obligation_function(
        tracked_id,
        MirOwnershipMetadata {
            reference_origins: Vec::new(),
            temporary_locals: Vec::new(),
            drop_obligations: vec![MirDropObligation {
                place: Place {
                    local: Local(1),
                    projection: Vec::new(),
                },
                ty: tracked_id,
                kind: DropObligationKind::Structural,
            }],
        },
    );

    BorrowChecker::check_function(&function, &type_context)
        .expect("structural cleanup should not require direct drop glue");
}

fn drop_obligation_function(
    tracked_id: crate::ids::TypeId,
    ownership: MirOwnershipMetadata,
) -> MirFunction {
    let unit_id = tracked_id;
    MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(123))),
        name: "drop_obligation".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit_id,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: tracked_id,
                mutability: Mutability::Mut,
                name: Some("tracked".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit_id,
        ownership,
    }
}
