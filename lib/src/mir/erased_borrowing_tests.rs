use super::*;
use crate::hir::borrowing::{BorrowProjection, BorrowSourceKind};
use crate::ids::{CrateId, LocalDefId};
use crate::mir::{MirBackendContract, MirLinkage};
use crate::type_context::TypeContext;
use crate::types::{CallableKind, CaptureKind, FunctionCapture, FunctionSafety};

fn id(index: u32) -> DefId {
    DefId::new(CrateId(1), LocalDefId(index))
}

fn callback_program(
    mutable_result: bool,
    borrowed_receiver: bool,
) -> (MirErasedFunction, MirBackendContract, TypeContext) {
    let mut tc = TypeContext::new();
    let scalar = tc.intern_type(&Type::I64);
    let reference = tc.intern_type(&Type::Reference {
        inner: Box::new(Type::I64),
        mutable: mutable_result,
    });
    let mut contract = MirBackendContract::default();
    for ty in [scalar, reference] {
        let drop = super::super::payload_drop_plan(&contract, &mut tc, ty).unwrap();
        contract.erased.descriptors.insert(
            ty,
            MirTypeDescriptor {
                ty,
                drop,
                fields: Vec::new(),
            },
        );
    }
    let callback = MirErasedType::Parameter(0);
    let input = MirErasedType::Concrete(reference);
    let receiver = if borrowed_receiver {
        MirErasedType::Reference {
            inner: Box::new(callback.clone()),
            mutable: false,
        }
    } else {
        callback.clone()
    };
    let member_params = vec![callback.clone(), input.clone()];
    let member = MirErasedDictionaryMember {
        binder: Default::default(),
        member_id: id(2),
        receiver: Some(ReceiverMode::Shared),
        borrowing: erased_borrow_contract(&member_params, &input, Some(ReceiverMode::Shared), &tc)
            .unwrap(),
        params: member_params,
        ret: input.clone(),
    };
    let input_operand = if mutable_result {
        MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(1)))
    } else {
        MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(1)))
    };
    let result = if mutable_result {
        MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(2)))
    } else {
        MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(2)))
    };
    let function = MirErasedFunction {
        key: MirErasedKey {
            definition: id(1),
            static_args: Vec::new(),
        },
        symbol: "borrow.callback".into(),
        linkage: MirLinkage::Internal,
        signature: MirErasedSignature {
            parameters: vec![MirErasedParameter {
                source: GenericParamId {
                    owner: id(1),
                    index: 0,
                },
                kind: Kind::Type,
            }],
            params: vec![receiver.clone(), input.clone()],
            ret: input.clone(),
            dictionaries: vec![MirErasedDictionarySchema {
                subject: callback.clone(),
                trait_id: id(3),
                trait_args: vec![input.clone(), input.clone()],
                members: vec![member],
            }],
            layouts: vec![callback, receiver.clone(), input.clone()],
            require_borrow_free: BTreeSet::new(),
        },
        body: Some(MirErasedBody {
            locals: vec![receiver, input.clone(), input],
            mutable_locals: BTreeSet::new(),
            scopes: Vec::new(),
            blocks: vec![MirErasedBasicBlock {
                statements: vec![MirErasedAssignment {
                    destination: MirErasedLocal(2),
                    value: MirErasedRvalue::DictionaryCall {
                        type_args: vec![],
                        dictionaries: vec![],
                        dictionary: 0,
                        member: 0,
                        receiver: Some(MirErasedPlace {
                            local: MirErasedLocal(0),
                            projection: if borrowed_receiver {
                                vec![MirErasedProjection::Deref]
                            } else {
                                Vec::new()
                            },
                        }),
                        args: vec![input_operand],
                    },
                }],
                terminator: MirErasedTerminator::Return(result),
            }],
        }),
    };
    (function, contract, tc)
}

#[test]
fn shared_callback_transports_shared_and_mutable_input_loans() {
    for mutable in [false, true] {
        let (function, contract, tc) = callback_program(mutable, true);
        validate_erased_function(&function, &contract, &tc).unwrap();
    }
}

#[test]
fn callback_result_cannot_borrow_an_owned_local_environment() {
    let (function, contract, tc) = callback_program(false, false);
    assert_eq!(
        validate_erased_function(&function, &contract, &tc),
        Err(MirErasedError::BorrowEscape)
    );
}

#[test]
fn portable_paths_preserve_nested_references_and_storage_contents_distinction() {
    let reference = Type::Reference {
        inner: Box::new(Type::I64),
        mutable: false,
    };
    let input = MirErasedType::Concrete(Type::Tuple(vec![reference.clone(), Type::I64]));
    let result = MirErasedType::Tuple(vec![
        MirErasedType::Concrete(reference),
        MirErasedType::Tuple(vec![MirErasedType::Parameter(0)]),
    ]);
    let borrowing = erased_borrow_contract_types(&[input], &result, None);
    assert_eq!(borrowing.returns[0].path, vec![BorrowProjection::Field(0)]);
    assert_eq!(
        borrowing.returns[1].path,
        vec![BorrowProjection::Field(1), BorrowProjection::Field(0)]
    );
    assert!(borrowing.returns[0].sources.iter().any(|source| {
        source.path == vec![BorrowProjection::Field(0)] && source.kind == BorrowSourceKind::Contents
    }));
    let bytes = bincode::serialize(&borrowing).unwrap();
    assert_eq!(
        bincode::deserialize::<crate::hir::borrowing::BorrowContract>(&bytes).unwrap(),
        borrowing
    );
}

#[test]
fn malformed_dictionary_origin_contract_is_rejected() {
    let (mut function, contract, tc) = callback_program(false, true);
    function.signature.dictionaries[0].members[0]
        .borrowing
        .returns[0]
        .sources[0]
        .argument = 99;
    assert_eq!(
        validate_erased_function(&function, &contract, &tc),
        Err(MirErasedError::InvalidDictionary)
    );
}

#[test]
fn unit_callback_retention_is_checked_independently_of_the_result() {
    let (mut function, contract, mut tc) = callback_program(false, false);
    let member = &mut function.signature.dictionaries[0].members[0];
    member.receiver = Some(ReceiverMode::Mut);
    member.ret = MirErasedType::Concrete(tc.intern_type(&Type::Unit));
    member.borrowing =
        erased_borrow_contract(&member.params, &member.ret, member.receiver, &tc).unwrap();
    let reference = Type::Reference {
        inner: Box::new(Type::I64),
        mutable: false,
    };
    let plain = Type::function(vec![reference.clone()], Type::Unit);
    tc.intern_type(&plain);
    assert_eq!(
        validate_concrete_borrow_effects(
            &function.signature,
            &[Some(plain.clone())],
            &contract,
            &tc
        ),
        Err(MirErasedError::BorrowEscape)
    );
    let retaining = Type::function_with_metadata(
        vec![reference.clone()],
        Type::Unit,
        FunctionSafety::Safe,
        CallableKind::FnMut,
        vec![FunctionCapture {
            kind: CaptureKind::Move,
            ty: reference,
        }],
    );
    tc.intern_type(&retaining);
    assert_eq!(
        validate_concrete_borrow_effects(&function.signature, &[Some(retaining)], &contract, &tc),
        Err(MirErasedError::BorrowEscape)
    );
    let member = &mut function.signature.dictionaries[0].members[0];
    member.params[1] = MirErasedType::Concrete(tc.intern_type(&Type::I64));
    member.borrowing =
        erased_borrow_contract(&member.params, &member.ret, member.receiver, &tc).unwrap();
    validate_concrete_borrow_effects(&function.signature, &[Some(plain)], &contract, &tc).unwrap();
}

#[test]
fn owned_callback_storage_obligation_is_not_a_blanket_input_or_output_ban() {
    let (borrowed, _, _) = callback_program(false, true);
    assert!(erased_callback_storage_requirements(
        &borrowed.signature.params,
        &borrowed.signature.dictionaries
    )
    .is_empty());
    let (mut owned, _, _) = callback_program(false, false);
    assert_eq!(
        erased_callback_storage_requirements(
            &owned.signature.params,
            &owned.signature.dictionaries
        ),
        BTreeSet::from([owned.signature.ret.clone()])
    );
    owned.signature.dictionaries[0].members[0].receiver = Some(ReceiverMode::Move);
    assert!(erased_callback_storage_requirements(
        &owned.signature.params,
        &owned.signature.dictionaries
    )
    .is_empty());
}

fn dictionary_with_live_receiver_loan(
    receiver_mode: ReceiverMode,
    mutable_loan: bool,
) -> (MirErasedFunction, MirBackendContract, TypeContext) {
    let (mut function, contract, mut tc) = callback_program(false, false);
    let unit = MirErasedType::Concrete(tc.intern_type(&Type::Unit));
    let loan = MirErasedType::Reference {
        inner: Box::new(function.signature.params[0].clone()),
        mutable: mutable_loan,
    };
    function.signature.ret = unit.clone();
    function
        .signature
        .layouts
        .extend([unit.clone(), loan.clone()]);
    let member = &mut function.signature.dictionaries[0].members[0];
    member.receiver = Some(receiver_mode);
    member.ret = unit.clone();
    member.borrowing = erased_borrow_contract(&member.params, &unit, member.receiver, &tc).unwrap();
    let body = function.body.as_mut().unwrap();
    body.locals[2] = unit;
    body.locals.extend([loan.clone(), loan]);
    body.mutable_locals.insert(MirErasedLocal(0));
    body.blocks[0].statements.insert(
        0,
        MirErasedAssignment {
            destination: MirErasedLocal(3),
            value: MirErasedRvalue::Borrow {
                place: MirErasedPlace::local(MirErasedLocal(0)),
                mutable: mutable_loan,
            },
        },
    );
    // The reference is used after the call: this is not an expired loan.
    body.blocks[0].statements.push(MirErasedAssignment {
        destination: MirErasedLocal(4),
        value: MirErasedRvalue::Use(if mutable_loan {
            MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(3)))
        } else {
            MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(3)))
        }),
    });
    body.blocks[0].terminator = MirErasedTerminator::Return(MirErasedOperand::Copy(
        MirErasedPlace::local(MirErasedLocal(2)),
    ));
    (function, contract, tc)
}

#[test]
fn shared_dictionary_receiver_rejects_a_live_mutable_loan() {
    let (function, contract, tc) = dictionary_with_live_receiver_loan(ReceiverMode::Shared, true);
    assert_eq!(
        validate_erased_function(&function, &contract, &tc),
        Err(MirErasedError::BorrowConflict)
    );
}

#[test]
fn dictionary_receiver_loan_conflicts_follow_the_access_mode() {
    for receiver in [ReceiverMode::Shared, ReceiverMode::Mut, ReceiverMode::Move] {
        for mutable in [false, true] {
            let (function, contract, tc) = dictionary_with_live_receiver_loan(receiver, mutable);
            let expected = if receiver == ReceiverMode::Shared && !mutable {
                Ok(())
            } else {
                Err(MirErasedError::BorrowConflict)
            };
            assert_eq!(
                validate_erased_function(&function, &contract, &tc),
                expected,
                "{receiver:?}, mutable loan: {mutable}"
            );
        }
    }
}

#[test]
fn operand_reads_obey_live_loan_permissions() {
    for mutable in [false, true] {
        for through_owner in [false, true] {
            let (mut function, contract, mut tc) = callback_program(false, true);
            let scalar = MirErasedType::Concrete(tc.intern_type(&Type::I64));
            let reference = MirErasedType::Reference {
                inner: Box::new(scalar.clone()),
                mutable,
            };
            function.signature.parameters.clear();
            function.signature.dictionaries.clear();
            function.signature.params = vec![scalar.clone()];
            function.signature.ret = scalar.clone();
            function.signature.layouts = vec![scalar.clone(), reference.clone()];
            function.body = Some(MirErasedBody {
                locals: vec![scalar.clone(), reference.clone(), scalar, reference],
                mutable_locals: BTreeSet::from([MirErasedLocal(0)]),
                scopes: Vec::new(),
                blocks: vec![MirErasedBasicBlock {
                    statements: vec![
                        MirErasedAssignment {
                            destination: MirErasedLocal(1),
                            value: MirErasedRvalue::Borrow {
                                place: MirErasedPlace::local(MirErasedLocal(0)),
                                mutable,
                            },
                        },
                        MirErasedAssignment {
                            destination: MirErasedLocal(2),
                            value: MirErasedRvalue::Use(MirErasedOperand::Copy(MirErasedPlace {
                                local: MirErasedLocal(u32::from(through_owner)),
                                projection: if through_owner {
                                    vec![MirErasedProjection::Deref]
                                } else {
                                    Vec::new()
                                },
                            })),
                        },
                        // Keep the borrow live across the read.
                        MirErasedAssignment {
                            destination: MirErasedLocal(3),
                            value: MirErasedRvalue::Use(MirErasedOperand::Move(
                                MirErasedPlace::local(MirErasedLocal(1)),
                            )),
                        },
                    ],
                    terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(
                        MirErasedPlace::local(MirErasedLocal(2)),
                    )),
                }],
            });
            let expected = if mutable && !through_owner {
                Err(MirErasedError::BorrowConflict)
            } else {
                Ok(())
            };
            assert_eq!(
                validate_erased_function(&function, &contract, &tc),
                expected,
                "mutable={mutable}, through owner={through_owner}",
            );
        }
    }
}

#[test]
fn cleanup_releases_loans_but_rejects_their_subsequent_use() {
    for use_after_cleanup in [false, true] {
        let (mut function, contract, tc) =
            dictionary_with_live_receiver_loan(ReceiverMode::Shared, true);
        let body = function.body.as_mut().unwrap();
        let mut statements = std::mem::take(&mut body.blocks[0].statements);
        let borrow = statements.remove(0);
        if !use_after_cleanup {
            statements.pop();
        }
        let terminator = body.blocks[0].terminator.clone();
        body.blocks = vec![
            MirErasedBasicBlock {
                statements: vec![borrow],
                terminator: MirErasedTerminator::Cleanup {
                    locals: vec![MirErasedLocal(3)],
                    target: MirErasedBlock(1),
                },
            },
            MirErasedBasicBlock {
                statements,
                terminator,
            },
        ];
        let expected = if use_after_cleanup {
            Err(MirErasedError::UninitializedOrMoved)
        } else {
            Ok(())
        };
        assert_eq!(
            validate_erased_function(&function, &contract, &tc),
            expected
        );
    }
}

#[test]
fn cleanup_does_not_destroy_storage_with_a_live_borrow() {
    let (mut function, contract, tc) =
        dictionary_with_live_receiver_loan(ReceiverMode::Shared, false);
    let body = function.body.as_mut().unwrap();
    body.blocks[0].statements.truncate(1);
    body.blocks[0].terminator = MirErasedTerminator::Cleanup {
        locals: vec![MirErasedLocal(0)],
        target: MirErasedBlock(0),
    };
    assert_eq!(
        validate_erased_function(&function, &contract, &tc),
        Err(MirErasedError::BorrowConflict)
    );
}

#[test]
fn cleanup_of_reference_storage_preserves_a_reborrow_of_parameter_contents() {
    let (mut function, contract, tc) = callback_program(false, true);
    let reference = function.signature.params[1].clone();
    function.signature.parameters.clear();
    function.signature.dictionaries.clear();
    function.signature.params = vec![reference.clone()];
    function.signature.layouts = vec![reference.clone()];
    function.body = Some(MirErasedBody {
        locals: vec![reference.clone(), reference],
        mutable_locals: BTreeSet::new(),
        scopes: Vec::new(),
        blocks: vec![
            MirErasedBasicBlock {
                statements: vec![MirErasedAssignment {
                    destination: MirErasedLocal(1),
                    value: MirErasedRvalue::Borrow {
                        place: MirErasedPlace {
                            local: MirErasedLocal(0),
                            projection: vec![MirErasedProjection::Deref],
                        },
                        mutable: false,
                    },
                }],
                terminator: MirErasedTerminator::Cleanup {
                    locals: vec![MirErasedLocal(0)],
                    target: MirErasedBlock(1),
                },
            },
            MirErasedBasicBlock {
                statements: Vec::new(),
                terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(
                    MirErasedPlace::local(MirErasedLocal(1)),
                )),
            },
        ],
    });
    assert_eq!(validate_erased_function(&function, &contract, &tc), Ok(()));
}

#[test]
fn cleanup_rejects_duplicate_locals_and_invalid_edges() {
    for (locals, target) in [
        (
            vec![MirErasedLocal(3), MirErasedLocal(3)],
            MirErasedBlock(1),
        ),
        (vec![MirErasedLocal(99)], MirErasedBlock(1)),
        (vec![MirErasedLocal(3)], MirErasedBlock(99)),
    ] {
        let (mut function, contract, tc) =
            dictionary_with_live_receiver_loan(ReceiverMode::Shared, true);
        let body = function.body.as_mut().unwrap();
        body.blocks[0].statements.truncate(1);
        body.blocks[0].terminator = MirErasedTerminator::Cleanup { locals, target };
        body.blocks.push(MirErasedBasicBlock {
            statements: Vec::new(),
            terminator: MirErasedTerminator::Unreachable,
        });
        assert_eq!(
            validate_erased_function(&function, &contract, &tc),
            Err(MirErasedError::InvalidLocal)
        );
    }
}

#[test]
fn cleanup_of_conditionally_initialized_storage_ends_its_lifetime_on_all_paths() {
    for use_after_cleanup in [false, true] {
        let (mut function, mut contract, mut tc) = callback_program(false, true);
        let boolean = tc.intern_type(&Type::Bool);
        let scalar = tc.intern_type(&Type::I64);
        let unit = tc.intern_type(&Type::Unit);
        for ty in [boolean, unit] {
            let drop = super::super::payload_drop_plan(&contract, &mut tc, ty).unwrap();
            contract.erased.descriptors.insert(
                ty,
                MirTypeDescriptor {
                    ty,
                    drop,
                    fields: Vec::new(),
                },
            );
        }
        function.signature.parameters.clear();
        function.signature.dictionaries.clear();
        function.signature.params = vec![MirErasedType::Concrete(boolean)];
        function.signature.ret = MirErasedType::Concrete(unit);
        function.signature.layouts.clear();
        let mut last = vec![MirErasedAssignment {
            destination: MirErasedLocal(2),
            value: MirErasedRvalue::Literal(MirErasedLiteral::Unit),
        }];
        if use_after_cleanup {
            last.push(MirErasedAssignment {
                destination: MirErasedLocal(3),
                value: MirErasedRvalue::Use(MirErasedOperand::Copy(MirErasedPlace::local(
                    MirErasedLocal(1),
                ))),
            });
        }
        function.body = Some(MirErasedBody {
            locals: vec![boolean, scalar, unit, scalar]
                .into_iter()
                .map(MirErasedType::Concrete)
                .collect(),
            mutable_locals: BTreeSet::new(),
            scopes: Vec::new(),
            blocks: vec![
                MirErasedBasicBlock {
                    statements: Vec::new(),
                    terminator: MirErasedTerminator::Branch {
                        condition: MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(0))),
                        then_block: MirErasedBlock(1),
                        else_block: MirErasedBlock(2),
                    },
                },
                MirErasedBasicBlock {
                    statements: vec![MirErasedAssignment {
                        destination: MirErasedLocal(1),
                        value: MirErasedRvalue::Literal(MirErasedLiteral::Int(42)),
                    }],
                    terminator: MirErasedTerminator::Goto(MirErasedBlock(3)),
                },
                MirErasedBasicBlock {
                    statements: Vec::new(),
                    terminator: MirErasedTerminator::Goto(MirErasedBlock(3)),
                },
                MirErasedBasicBlock {
                    statements: Vec::new(),
                    terminator: MirErasedTerminator::Cleanup {
                        locals: vec![MirErasedLocal(1)],
                        target: MirErasedBlock(4),
                    },
                },
                MirErasedBasicBlock {
                    statements: last,
                    terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(
                        MirErasedPlace::local(MirErasedLocal(2)),
                    )),
                },
            ],
        });
        let expected = if use_after_cleanup {
            Err(MirErasedError::UninitializedOrMoved)
        } else {
            Ok(())
        };
        assert_eq!(
            validate_erased_function(&function, &contract, &tc),
            expected
        );
    }
}
