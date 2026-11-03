use std::collections::{BTreeMap, HashMap, HashSet};

use super::{
    type_id_contains_reference, type_id_is_mut_reference, type_id_is_pointer, type_id_tracks_origin,
};
use crate::diagnostic::{DiagnosticCode, Diagnostics};
use crate::ids::{AssocTypeId, CrateId, DefId, InstanceId, LocalDefId};
use crate::mir::borrowck::BorrowChecker;
use crate::mir::{
    BasicBlock, Constant, DropObligationKind, Local, LocalDecl, MirAssert, MirAssertKind,
    MirBackendContract, MirCallable, MirCallableKey, MirClosure, MirClosureCapture,
    MirClosureCaptureKind, MirClosureId, MirDropObligation, MirEnumVariantLayout, MirFunction,
    MirFunctionId, MirNominalLayout, MirOwnershipMetadata, MirProgram, MirProjectionKey,
    MirVariantLayoutFields, Mutability, Operand, Place, Projection, ReferenceOrigin, Rvalue,
    StatementData, Terminator,
};
use crate::type_context::{TypeContext, TypeView};
use crate::types::{GenericParamId, Type};

fn test_type_id(type_context: &mut TypeContext, ty: Type) -> crate::ids::TypeId {
    type_context.intern_type(&ty)
}

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
    )
    .expect_err("projection output reference should not allow a local to escape");

    assert!(err.0.iter().any(|diag| diag
        .message
        .contains("Cannot return reference to local value")));
}

#[test]
fn borrowck_uses_indexed_loan_state_for_active_conflicts() {
    let mut type_context = TypeContext::new();
    let func = mut_borrow_then_shared_read_function(&mut type_context);
    let err = BorrowChecker::check_function(&func, &type_context).expect_err("borrow conflict");

    assert!(err
        .0
        .iter()
        .any(|diag| { diag.message.contains("Cannot borrow") || diag.message.contains("borrow") }));
}

#[test]
fn immutable_binding_mut_borrow_diagnostic_ignores_display_name_presence() {
    let mut type_context = TypeContext::new();
    let named = immutable_binding_mut_borrow_function(&mut type_context, Some("value"));
    let unnamed = immutable_binding_mut_borrow_function(&mut type_context, None);

    for function in [&named, &unnamed] {
        let err = BorrowChecker::check_function(function, &type_context)
            .expect_err("mutable borrow of immutable binding should be rejected");
        assert!(err.0.iter().any(|diag| diag
            .message
            .contains("Cannot take a mutable reference to immutable binding")));
    }
}

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

#[test]
fn borrowck_rejects_spanless_non_cleanup_discriminant_read_after_move() {
    let mut type_context = TypeContext::new();
    let enum_id = DefId::new(CrateId(0), LocalDefId(700));
    let enum_ty = Type::Enum {
        id: enum_id,
        args: Vec::new(),
    };
    let enum_ty_id = test_type_id(&mut type_context, enum_ty);
    let i64_id = test_type_id(&mut type_context, Type::I64);
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(701))),
        name: "spanless_discriminant_after_move".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Move(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    })),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(3),
                        projection: Vec::new(),
                    },
                    Rvalue::Discriminant(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    }),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(3),
                        projection: Vec::new(),
                    })),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: enum_ty_id,
                mutability: Mutability::Not,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            LocalDecl {
                ty: enum_ty_id,
                mutability: Mutability::Not,
                name: Some("moved".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Not,
                name: Some("discriminant".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_id,
        ownership: MirOwnershipMetadata::default(),
    };

    let err = BorrowChecker::check_function(&function, &type_context)
        .expect_err("spanless non-cleanup discriminant read after move should be rejected");
    assert!(err
        .0
        .iter()
        .any(|diag| diag.message.contains("borrow of moved value")));
    let diagnostic = err
        .0
        .iter()
        .find(|diag| diag.message.contains("borrow of moved value"))
        .expect("move diagnostic should be present");
    assert_eq!(diagnostic.code, Some(DiagnosticCode::Internal));
    assert_eq!(
        diagnostic.location,
        crate::diagnostic::DiagnosticLocation::Toolchain
    );
    assert!(diagnostic.primary.is_none());
    assert!(diagnostic.secondary.is_empty());
}

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

#[test]
fn reference_return_summary_joins_branch_return_origins() {
    let mut type_context = TypeContext::new();
    let ref_i64 = test_type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let bool_id = test_type_id(&mut type_context, Type::Bool);
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(122)));
    let function = MirFunction {
        id: function_id.clone(),
        name: "choose_ref".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::SwitchInt {
                    discr: Operand::Copy(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    }),
                    targets: vec![(1, crate::mir::BasicBlockId(1))],
                    otherwise: crate::mir::BasicBlockId(2),
                }),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(2),
                        projection: Vec::new(),
                    })),
                    None,
                )],
                terminator: Some(Terminator::Goto(crate::mir::BasicBlockId(3))),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(3),
                        projection: Vec::new(),
                    })),
                    None,
                )],
                terminator: Some(Terminator::Goto(crate::mir::BasicBlockId(3))),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: bool_id,
                mutability: Mutability::Not,
                name: Some("flag".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("left".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("right".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 3,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };
    let program = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let summaries = BorrowChecker::reference_return_summaries(&program);
    let summary = summaries
        .get(&function_id)
        .expect("choose_ref should have a reference return summary");

    assert!(summary.contains(&ReferenceOrigin::Param(Local(2))));
    assert!(summary.contains(&ReferenceOrigin::Param(Local(3))));
}

#[test]
fn instance_reference_return_summary_maps_receiver_origin() {
    let mut type_context = TypeContext::new();
    let ref_i64 = test_type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let instance_id = InstanceId(902);
    let callee_id = MirFunctionId::Instance(instance_id);
    let caller_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(903)));
    let callee = MirFunction {
        id: callee_id.clone(),
        name: "instance_ref".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: Vec::new(),
                },
                Rvalue::Use(Operand::Copy(Place {
                    local: Local(1),
                    projection: Vec::new(),
                })),
                None,
            )],
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
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("receiver".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };
    let caller = MirFunction {
        id: caller_id.clone(),
        name: "caller".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Instance(instance_id),
                    ))),
                    args: vec![Operand::Copy(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    })],
                    destination: Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: Some(crate::lexer::Span::test()),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("receiver".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };
    let program = MirProgram {
        functions: BTreeMap::from([(callee_id, callee), (caller_id.clone(), caller)]),
        type_context,
        backend_contract: Default::default(),
    };

    let summaries = BorrowChecker::reference_return_summaries(&program);
    assert_eq!(
        summaries.get(&caller_id),
        Some(&HashSet::from([ReferenceOrigin::Param(Local(1))]))
    );
}

#[test]
fn temporary_reference_return_call_destination_is_rejected() {
    let mut type_context = TypeContext::new();
    let unit = test_type_id(&mut type_context, Type::Unit);
    let i64_id = test_type_id(&mut type_context, Type::I64);
    let ref_i64 = test_type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let callee_id = DefId::new(CrateId(0), LocalDefId(904));
    let caller_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(905)));
    let callee = MirFunction {
        id: MirFunctionId::Function(callee_id),
        name: "returns_temporary_reference".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
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
            )],
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
                name: Some("temporary".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };
    let caller = MirFunction {
        id: caller_id.clone(),
        name: "stores_call_result".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Function(callee_id),
                    ))),
                    args: Vec::new(),
                    destination: Place {
                        local: Local(1),
                        projection: Vec::new(),
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: Some(crate::lexer::Span::test()),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("stored".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit,
        ownership: MirOwnershipMetadata::default(),
    };
    let program = MirProgram {
        functions: BTreeMap::from([
            (MirFunctionId::Function(callee_id), callee),
            (caller_id.clone(), caller.clone()),
        ]),
        type_context,
        backend_contract: Default::default(),
    };
    let summaries = BorrowChecker::reference_return_summaries(&program);
    let mut diagnostics = Diagnostics::default();

    BorrowChecker::validate_reference_escapes(
        &caller,
        &program.type_context,
        &MirBackendContract::default(),
        &summaries,
        &mut diagnostics,
    );

    assert!(diagnostics
        .0
        .iter()
        .any(|diagnostic| diagnostic.message.contains("temporary")));
}

#[test]
fn reference_return_summary_preserves_param_origin_through_pointer_projection() {
    let mut type_context = TypeContext::new();
    let i64_id = type_context.intern_type(&Type::I64);
    let ptr_i64 = type_context.intern_ty(crate::type_context::Ty::Pointer(i64_id));
    let ref_i64 = type_context.intern_ty(crate::type_context::Ty::Reference {
        mutable: false,
        inner: i64_id,
    });
    let index_i64 = type_context.intern_type(&Type::I64);
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(127))),
        name: "project_pointer_ref".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    })),
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
                            local: Local(2),
                            projection: vec![Projection::Index(Local(3))],
                        },
                    ),
                    None,
                ),
            ],
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
                ty: ptr_i64,
                mutability: Mutability::Not,
                name: Some("ptr_arg".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            LocalDecl {
                ty: ptr_i64,
                mutability: Mutability::Not,
                name: Some("ptr_temp".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
            LocalDecl {
                ty: index_i64,
                mutability: Mutability::Not,
                name: Some("idx".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };

    let origins = BorrowChecker::reference_return_origins(
        &function,
        &type_context,
        &MirBackendContract::default(),
        &HashMap::new(),
    );

    assert!(origins.contains(&ReferenceOrigin::Param(Local(1))));
    assert!(!origins.contains(&ReferenceOrigin::Param(Local(2))));
}

#[test]
fn opaque_reference_return_maps_reference_argument_origins() {
    let mut type_context = TypeContext::new();
    let ref_i64 = test_type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let extern_id = DefId::new(CrateId(0), LocalDefId(123));
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(124))),
        name: "extern_passthrough".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Extern(extern_id),
                    ))),
                    args: vec![Operand::Copy(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    })],
                    destination: Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: Some(crate::lexer::Span::test()),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("input".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };
    let mut diagnostics = Diagnostics::default();

    BorrowChecker::validate_reference_escapes(
        &function,
        &type_context,
        &MirBackendContract::default(),
        &HashMap::new(),
        &mut diagnostics,
    );

    assert!(diagnostics.0.is_empty());
}

#[test]
fn reference_return_summary_defers_missing_known_call_summary() {
    let mut type_context = TypeContext::new();
    let ref_i64 = test_type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let callee_id = DefId::new(CrateId(0), LocalDefId(125));
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(126)));
    let function = MirFunction {
        id: function_id.clone(),
        name: "calls_later_summarized".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Function(callee_id),
                    ))),
                    args: Vec::new(),
                    destination: Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: Some(crate::lexer::Span::test()),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![LocalDecl {
            ty: ref_i64,
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };
    let origins = BorrowChecker::reference_return_origins(
        &function,
        &type_context,
        &MirBackendContract::default(),
        &HashMap::new(),
    );

    assert!(origins.is_empty());
}

#[test]
fn borrowck_reports_parent_use_after_field_move() {
    let mut type_context = TypeContext::new();
    let i64_id = type_context.intern_type(&Type::I64);
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(99))),
        name: "field_move".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
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
                ),
                StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    })),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("tmp".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_id,
        ownership: Default::default(),
    };

    let err = BorrowChecker::check_function(&function, &type_context)
        .expect_err("parent use after field move should be rejected");
    assert!(err.0.iter().any(|diag| diag.message.contains("moved")));
}

#[test]
fn borrowck_reports_drop_after_field_move() {
    let mut type_context = TypeContext::new();
    let i64_id = type_context.intern_type(&Type::I64);
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(101))),
        name: "drop_after_field_move".to_string(),
        basic_blocks: vec![
            BasicBlock {
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
                terminator: Some(Terminator::Drop {
                    place: Place {
                        local: Local(1),
                        projection: Vec::new(),
                    },
                    target: crate::mir::BasicBlockId(1),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("tmp".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_id,
        ownership: Default::default(),
    };

    let err = BorrowChecker::check_function(&function, &type_context)
        .expect_err("drop after field move should be rejected");
    assert!(err.0.iter().any(|diag| diag.message.contains("moved")));
}

#[test]
fn borrowck_allows_sibling_use_after_field_move() {
    let mut type_context = TypeContext::new();
    let i64_id = type_context.intern_type(&Type::I64);
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(100))),
        name: "field_move_sibling".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
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
                ),
                StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(1),
                        projection: vec![Projection::Field {
                            index: 1,
                            identity: None,
                        }],
                    })),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("tmp".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_id,
        ownership: Default::default(),
    };

    BorrowChecker::check_function(&function, &type_context)
        .expect("sibling field remains usable after moving another field");
}

#[test]
fn borrowck_rejects_non_copy_move_out_of_shared_reference() {
    let mut type_context = TypeContext::new();
    let string_def = DefId::new(CrateId(0), LocalDefId(130));
    let string_ty = Type::Struct {
        id: string_def,
        args: Vec::new(),
    };
    let string_id = type_context.intern_type(&string_ty);
    let ref_string_id = type_context.intern_type(&Type::Reference {
        mutable: false,
        inner: Box::new(string_ty),
    });
    let function = move_or_copy_out_of_reference_function(
        string_id,
        ref_string_id,
        Operand::Move(Place {
            local: Local(1),
            projection: vec![Projection::Deref],
        }),
    );
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        string_def,
        MirNominalLayout::Struct {
            id: string_def,
            fields: Vec::new(),
            generic_params: Vec::new(),
        },
    );

    let err = BorrowChecker::check_function_with_contract(
        &function,
        &type_context,
        &backend_contract,
        &HashMap::new(),
    )
    .expect_err("moving a non-copy value out of a shared reference should be rejected");

    assert!(err
        .0
        .iter()
        .any(|diag| diag.message.contains("Cannot move non-copy value of type")));
}

#[test]
fn borrowck_allows_copy_out_of_shared_reference() {
    let mut type_context = TypeContext::new();
    let i64_id = type_context.intern_type(&Type::I64);
    let ref_i64_id = type_context.intern_type(&Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    });
    let function = move_or_copy_out_of_reference_function(
        i64_id,
        ref_i64_id,
        Operand::Copy(Place {
            local: Local(1),
            projection: vec![Projection::Deref],
        }),
    );

    BorrowChecker::check_function(&function, &type_context)
        .expect("copying I64 out of a shared reference should be allowed");
}

#[test]
fn borrowck_rejects_non_copy_copy_out_of_shared_reference() {
    let mut type_context = TypeContext::new();
    let pair_def = DefId::new(CrateId(0), LocalDefId(132));
    let i64_id = type_context.intern_type(&Type::I64);
    let pair_ty = Type::Struct {
        id: pair_def,
        args: Vec::new(),
    };
    let pair_id = type_context.intern_type(&pair_ty);
    let ref_pair_id = type_context.intern_type(&Type::Reference {
        mutable: false,
        inner: Box::new(pair_ty),
    });
    let function = move_or_copy_out_of_reference_function(
        pair_id,
        ref_pair_id,
        Operand::Copy(Place {
            local: Local(1),
            projection: vec![Projection::Deref],
        }),
    );
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        pair_def,
        MirNominalLayout::Struct {
            id: pair_def,
            fields: vec![("x".to_string(), i64_id)],
            generic_params: Vec::new(),
        },
    );

    let err = BorrowChecker::check_function_with_contract(
        &function,
        &type_context,
        &backend_contract,
        &HashMap::new(),
    )
    .expect_err("copy operand of non-copy value out of shared reference should be rejected");

    assert!(err
        .0
        .iter()
        .any(|diag| diag.message.contains("Cannot move non-copy value of type")));
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
    let array_id = type_context.intern_type(&Type::Array(Box::new(drop_ty), 2));
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

fn immutable_binding_mut_borrow_function(
    type_context: &mut TypeContext,
    binding_name: Option<&str>,
) -> MirFunction {
    let unit = test_type_id(type_context, Type::Unit);
    let i64_id = test_type_id(type_context, Type::I64);
    let mut_ref_i64 = test_type_id(
        type_context,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::I64),
        },
    );

    MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(120))),
        name: "mut_borrow_immutable".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(2),
                    projection: Vec::new(),
                },
                Rvalue::Ref(
                    Mutability::Mut,
                    Place {
                        local: Local(1),
                        projection: Vec::new(),
                    },
                ),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Not,
                name: binding_name.map(str::to_string),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: mut_ref_i64,
                mutability: Mutability::Not,
                name: Some("borrow".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit,
        ownership: MirOwnershipMetadata::default(),
    }
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

fn move_or_copy_out_of_reference_function(
    value_type: crate::ids::TypeId,
    reference_type: crate::ids::TypeId,
    source: Operand,
) -> MirFunction {
    MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(131))),
        name: "move_or_copy_out_of_reference".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(2),
                    projection: Vec::new(),
                },
                Rvalue::Use(source),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: value_type,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: reference_type,
                mutability: Mutability::Not,
                name: Some("reference".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            LocalDecl {
                ty: value_type,
                mutability: Mutability::Mut,
                name: Some("destination".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: value_type,
        ownership: MirOwnershipMetadata::default(),
    }
}

fn mut_borrow_then_shared_read_function(type_context: &mut TypeContext) -> MirFunction {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0)));
    let local_decls = vec![
        LocalDecl {
            ty: test_type_id(type_context, Type::Unit),
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(type_context, Type::I64),
            mutability: Mutability::Mut,
            name: Some("x".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: true,
                },
            ),
            mutability: Mutability::Not,
            name: Some("r".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: false,
                },
            ),
            mutability: Mutability::Not,
            name: Some("s".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: true,
                },
            ),
            mutability: Mutability::Not,
            name: Some("keep_r_live".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
    ];

    MirFunction {
        id: function_id,
        name: "mut_borrow_then_shared_read".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: vec![],
                    },
                    Rvalue::Ref(
                        Mutability::Mut,
                        Place {
                            local: Local(1),
                            projection: vec![],
                        },
                    ),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(3),
                        projection: vec![],
                    },
                    Rvalue::Ref(
                        Mutability::Not,
                        Place {
                            local: Local(1),
                            projection: vec![],
                        },
                    ),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(4),
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(2),
                        projection: vec![],
                    })),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls,
        closure_captures: vec![],
        arg_count: 1,
        ret_type: test_type_id(type_context, Type::Unit),
        ownership: Default::default(),
    }
}

#[test]
fn assert_operand_read_conflicts_with_active_mut_borrow() {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0)));
    let mut type_context = TypeContext::new();
    let local_decls = vec![
        LocalDecl {
            ty: test_type_id(&mut type_context, Type::Unit),
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(&mut type_context, Type::I64),
            mutability: Mutability::Mut,
            name: Some("x".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                &mut type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: true,
                },
            ),
            mutability: Mutability::Not,
            name: Some("r".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                &mut type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: true,
                },
            ),
            mutability: Mutability::Not,
            name: Some("keep_r_live".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
    ];
    let block = BasicBlock {
        statements: vec![
            StatementData::assign(
                Place {
                    local: Local(2),
                    projection: vec![],
                },
                Rvalue::Ref(
                    Mutability::Mut,
                    Place {
                        local: Local(1),
                        projection: vec![],
                    },
                ),
                None,
            ),
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![Operand::Copy(Place {
                        local: Local(1),
                        projection: vec![],
                    })],
                },
                None,
            ),
            StatementData::assign(
                Place {
                    local: Local(3),
                    projection: vec![],
                },
                Rvalue::Use(Operand::Copy(Place {
                    local: Local(2),
                    projection: vec![],
                })),
                None,
            ),
        ],
        terminator: Some(Terminator::Return),
    };
    let program = MirProgram {
        functions: BTreeMap::from([(
            function_id.clone(),
            MirFunction {
                id: function_id,
                name: "assert_read_conflict".to_string(),
                basic_blocks: vec![block],
                local_decls,
                closure_captures: vec![],
                arg_count: 1,
                ret_type: test_type_id(&mut type_context, Type::Unit),
                ownership: Default::default(),
            },
        )]),
        type_context,
        backend_contract: Default::default(),
    };

    let diagnostics = BorrowChecker::run(&program).expect_err("assert read should conflict");

    assert!(diagnostics
        .0
        .iter()
        .any(|diagnostic| diagnostic.message.contains("borrow")));
}

#[test]
fn assert_move_operand_read_does_not_conflict_with_active_shared_borrow() {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0)));
    let mut type_context = TypeContext::new();
    let local_decls = vec![
        LocalDecl {
            ty: test_type_id(&mut type_context, Type::Unit),
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(&mut type_context, Type::I64),
            mutability: Mutability::Not,
            name: Some("x".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                &mut type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: false,
                },
            ),
            mutability: Mutability::Not,
            name: Some("r".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                &mut type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: false,
                },
            ),
            mutability: Mutability::Not,
            name: Some("keep_r_live".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
    ];
    let block = BasicBlock {
        statements: vec![
            StatementData::assign(
                Place {
                    local: Local(2),
                    projection: vec![],
                },
                Rvalue::Ref(
                    Mutability::Not,
                    Place {
                        local: Local(1),
                        projection: vec![],
                    },
                ),
                None,
            ),
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![Operand::Move(Place {
                        local: Local(1),
                        projection: vec![],
                    })],
                },
                None,
            ),
            StatementData::assign(
                Place {
                    local: Local(3),
                    projection: vec![],
                },
                Rvalue::Use(Operand::Copy(Place {
                    local: Local(2),
                    projection: vec![],
                })),
                None,
            ),
        ],
        terminator: Some(Terminator::Return),
    };
    let program = MirProgram {
        functions: BTreeMap::from([(
            function_id.clone(),
            MirFunction {
                id: function_id,
                name: "assert_move_read_shared_borrow".to_string(),
                basic_blocks: vec![block],
                local_decls,
                closure_captures: vec![],
                arg_count: 1,
                ret_type: test_type_id(&mut type_context, Type::Unit),
                ownership: Default::default(),
            },
        )]),
        type_context,
        backend_contract: Default::default(),
    };

    BorrowChecker::run(&program).expect("assert move operand should be read-only");
}
