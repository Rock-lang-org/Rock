use super::*;

#[test]
fn reference_escape_validator_uses_ownership_metadata() {
    let mut type_context = TypeContext::new();

    let local_escape = reference_return_function(
        &mut type_context,
        MirOwnershipMetadata {
            reference_origins: vec![(Local(0), ReferenceOrigin::Local(Local(1)))],
            temporary_locals: Vec::new(),
            drop_obligations: Vec::new(),
        },
    );
    let mut diagnostics = Diagnostics::default();
    BorrowChecker::validate_reference_escapes(
        &local_escape,
        &type_context,
        &MirBackendContract::default(),
        &HashMap::new(),
        &mut diagnostics,
    );
    assert!(diagnostics
        .0
        .iter()
        .any(|diag| diag.message.contains("does not live long enough")));

    let temporary_escape = reference_return_function(
        &mut type_context,
        MirOwnershipMetadata {
            reference_origins: vec![(Local(0), ReferenceOrigin::Temporary(Local(2)))],
            temporary_locals: vec![Local(2)],
            drop_obligations: Vec::new(),
        },
    );
    let mut diagnostics = Diagnostics::default();
    BorrowChecker::validate_reference_escapes(
        &temporary_escape,
        &type_context,
        &MirBackendContract::default(),
        &HashMap::new(),
        &mut diagnostics,
    );
    assert!(diagnostics
        .0
        .iter()
        .any(|diag| diag.message.contains("temporary")));

    let param_escape = reference_return_function(
        &mut type_context,
        MirOwnershipMetadata {
            reference_origins: vec![(Local(0), ReferenceOrigin::Param(Local(1)))],
            temporary_locals: Vec::new(),
            drop_obligations: Vec::new(),
        },
    );
    let mut diagnostics = Diagnostics::default();
    BorrowChecker::validate_reference_escapes(
        &param_escape,
        &type_context,
        &MirBackendContract::default(),
        &HashMap::new(),
        &mut diagnostics,
    );
    assert!(diagnostics.0.is_empty());

    let static_override = reference_return_function(
        &mut type_context,
        MirOwnershipMetadata {
            reference_origins: vec![
                (Local(0), ReferenceOrigin::Temporary(Local(2))),
                (Local(0), ReferenceOrigin::Static),
            ],
            temporary_locals: vec![Local(2)],
            drop_obligations: Vec::new(),
        },
    );
    let mut diagnostics = Diagnostics::default();
    BorrowChecker::validate_reference_escapes(
        &static_override,
        &type_context,
        &MirBackendContract::default(),
        &HashMap::new(),
        &mut diagnostics,
    );
    assert!(diagnostics.0.is_empty());
}

#[test]
fn closure_assignment_rejects_captured_reference_to_temporary() {
    let mut type_context = TypeContext::new();
    let unit_id = test_type_id(&mut type_context, Type::Unit);
    let closure_id = test_type_id(&mut type_context, Type::function(Vec::new(), Type::Unit));
    let ref_i64 = test_type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let i64_id = test_type_id(&mut type_context, Type::I64);

    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(128))),
        name: "closure_captures_temporary_ref".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: Vec::new(),
                },
                Rvalue::Closure(MirClosure {
                    id: MirClosureId {
                        owner: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(128))),
                        local_index: 0,
                    },
                    display_name: "closure_captures_temporary_ref::lambda_0".to_string(),
                    captures: vec![MirClosureCapture {
                        name: "captured".to_string(),
                        local: Local(1),
                        kind: MirClosureCaptureKind::ByRef,
                        span: None,
                    }],
                }),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: closure_id,
                mutability: Mutability::Not,
                name: Some("closure".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("captured".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Not,
                name: Some("call_result".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit_id,
        ownership: MirOwnershipMetadata {
            reference_origins: vec![(Local(1), ReferenceOrigin::Temporary(Local(2)))],
            temporary_locals: vec![Local(1), Local(2)],
            drop_obligations: Vec::new(),
        },
    };
    let mut diagnostics = Diagnostics::default();

    BorrowChecker::validate_reference_escapes(
        &function,
        &type_context,
        &MirBackendContract::default(),
        &HashMap::new(),
        &mut diagnostics,
    );

    assert!(diagnostics.0.iter().any(|diag| {
        diag.message
            .contains("Cannot store reference to temporary value")
    }));
}

fn reference_return_function(
    type_context: &mut TypeContext,
    ownership: MirOwnershipMetadata,
) -> MirFunction {
    let ref_i64 = test_type_id(
        type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let i64_id = test_type_id(type_context, Type::I64);

    MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(121))),
        name: "reference_return".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Not,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Not,
                name: Some("temporary".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: ref_i64,
        ownership,
    }
}
