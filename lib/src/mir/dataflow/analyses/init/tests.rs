use super::*;

use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mir::{
    AggregateKind, BasicBlock, BasicBlockId, Constant, LocalDecl, MirAssert, MirAssertKind,
    MirClosureCapture, MirClosureCaptureKind, MirClosureId, MirFunctionId, Mutability, Projection,
};
use crate::types::Type;

fn test_type_id(
    type_context: &mut crate::type_context::TypeContext,
    ty: Type,
) -> crate::ids::TypeId {
    type_context.intern_type(&ty)
}

fn field_place(local: Local, index: usize) -> Place {
    Place {
        local,
        projection: vec![Projection::Field {
            index,
            identity: None,
        }],
    }
}

fn test_function_with_block(
    type_context: &mut crate::type_context::TypeContext,
    block: BasicBlock,
) -> MirFunction {
    let i64_ty = test_type_id(type_context, Type::I64);

    MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1))),
        name: "test".to_string(),
        basic_blocks: vec![block],
        local_decls: vec![
            LocalDecl {
                ty: i64_ty,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_ty,
                mutability: Mutability::Mut,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_ty,
                mutability: Mutability::Mut,
                name: Some("dest".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_ty,
        ownership: Default::default(),
    }
}

#[test]
fn initialization_fixpoint_successor_entry_is_not_poisoned_by_unvisited_state() {
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_ty = test_type_id(&mut type_context, Type::I64);
    let local = Place {
        local: Local(1),
        projection: Vec::new(),
    };
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1))),
        name: "test".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: vec![StatementData::assign(
                    local.clone(),
                    Rvalue::Use(Operand::Constant(Constant::Int(1))),
                    None,
                )],
                terminator: Some(Terminator::Goto(BasicBlockId(1))),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: i64_ty,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_ty,
                mutability: Mutability::Mut,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_ty,
        ownership: Default::default(),
    };

    let analysis = InitializationAnalysis::new(&function, &type_context);
    let results = crate::mir::dataflow::run_fixpoint(&analysis, &function);

    assert!(analysis
        .check_operand_at_path(&Operand::Copy(local), &results.entry_sets[1])
        .is_ok());
}

#[test]
fn initialization_binary_op_move_marks_field_moved_and_preserves_sibling() {
    let mut type_context = crate::type_context::TypeContext::new();
    let field0 = field_place(Local(1), 0);
    let field1 = field_place(Local(1), 1);
    let stmt = StatementData::assign(
        Place {
            local: Local(2),
            projection: Vec::new(),
        },
        Rvalue::BinaryOp(
            crate::mir::MirBinOp::Add,
            Operand::Move(field0.clone()),
            Operand::Copy(field1.clone()),
        ),
        None,
    );
    let function = test_function_with_block(
        &mut type_context,
        BasicBlock {
            statements: vec![stmt.clone()],
            terminator: None,
        },
    );
    let analysis = InitializationAnalysis::new(&function, &type_context);
    let mut state = analysis.initial_state(&function);

    analysis.apply_statement(&mut state, &stmt);

    let field0_error = analysis
        .check_operand_at_path(&Operand::Move(field0), &state)
        .unwrap_err();
    assert!(field0_error.message.contains("use of moved value"));
    assert!(analysis.check_place_at_path(&field1, &state).is_ok());
}

#[test]
fn initialization_move_through_raw_pointer_index_preserves_pointer_local() {
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_ty = test_type_id(&mut type_context, Type::I64);
    let ptr_ty = test_type_id(&mut type_context, Type::Pointer(Box::new(Type::I64)));
    let pointer = Place {
        local: Local(1),
        projection: Vec::new(),
    };
    let pointee = Place {
        local: Local(1),
        projection: vec![Projection::Index(Local(2))],
    };
    let stmt = StatementData::assign(
        Place {
            local: Local(3),
            projection: Vec::new(),
        },
        Rvalue::Use(Operand::Move(pointee)),
        None,
    );
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1))),
        name: "test".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![stmt.clone()],
            terminator: None,
        }],
        local_decls: vec![
            LocalDecl {
                ty: i64_ty,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: ptr_ty,
                mutability: Mutability::Mut,
                name: Some("ptr".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_ty,
                mutability: Mutability::Mut,
                name: Some("index".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_ty,
                mutability: Mutability::Mut,
                name: Some("dest".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_ty,
        ownership: Default::default(),
    };
    let analysis = InitializationAnalysis::new(&function, &type_context);
    let mut state = analysis.initial_state(&function);

    analysis.apply_statement(&mut state, &stmt);

    assert!(analysis
        .check_operand_at_path(&Operand::Copy(pointer), &state)
        .is_ok());
}

#[test]
fn initialization_assert_move_marks_field_moved() {
    let mut type_context = crate::type_context::TypeContext::new();
    let field0 = field_place(Local(1), 0);
    let stmt = StatementData::assert(
        MirAssert {
            kind: MirAssertKind::BoundsCheck,
            operands: vec![Operand::Move(field0.clone())],
        },
        None,
    );
    let function = test_function_with_block(
        &mut type_context,
        BasicBlock {
            statements: vec![stmt.clone()],
            terminator: None,
        },
    );
    let analysis = InitializationAnalysis::new(&function, &type_context);
    let mut state = analysis.initial_state(&function);

    analysis.apply_statement(&mut state, &stmt);

    let field0_error = analysis
        .check_operand_at_path(&Operand::Move(field0), &state)
        .unwrap_err();
    assert!(field0_error.message.contains("use of moved value"));
}

#[test]
fn initialization_call_move_argument_marks_field_moved() {
    let mut type_context = crate::type_context::TypeContext::new();
    let field0 = field_place(Local(1), 0);
    let terminator = Terminator::Call {
        func: Operand::Constant(Constant::Unit),
        args: vec![Operand::Move(field0.clone())],
        destination: Place {
            local: Local(2),
            projection: Vec::new(),
        },
        target: BasicBlockId(0),
        span: Some(crate::lexer::Span::test()),
    };
    let function = test_function_with_block(
        &mut type_context,
        BasicBlock {
            statements: Vec::new(),
            terminator: Some(terminator.clone()),
        },
    );
    let analysis = InitializationAnalysis::new(&function, &type_context);
    let mut state = analysis.initial_state(&function);

    analysis.apply_terminator(&mut state, &terminator);

    let field0_error = analysis
        .check_operand_at_path(&Operand::Move(field0), &state)
        .unwrap_err();
    assert!(field0_error.message.contains("use of moved value"));
}

#[test]
fn initialization_call_destination_remains_init_when_also_moved_arg() {
    let mut type_context = crate::type_context::TypeContext::new();
    let destination = Place {
        local: Local(2),
        projection: Vec::new(),
    };
    let terminator = Terminator::Call {
        func: Operand::Constant(Constant::Unit),
        args: vec![Operand::Move(destination.clone())],
        destination: destination.clone(),
        target: BasicBlockId(0),
        span: Some(crate::lexer::Span::test()),
    };
    let function = test_function_with_block(
        &mut type_context,
        BasicBlock {
            statements: Vec::new(),
            terminator: Some(terminator.clone()),
        },
    );
    let analysis = InitializationAnalysis::new(&function, &type_context);
    let mut state = analysis.initial_state(&function);

    analysis.apply_terminator(&mut state, &terminator);

    assert!(analysis
        .check_operand_at_path(&Operand::Copy(destination), &state)
        .is_ok());
}

#[test]
fn initialization_assignment_destination_remains_init_when_rhs_moves_part() {
    let mut type_context = crate::type_context::TypeContext::new();
    let destination = Place {
        local: Local(1),
        projection: Vec::new(),
    };
    let field0 = field_place(Local(1), 0);
    let field1 = field_place(Local(1), 1);
    let stmt = StatementData::assign(
        destination.clone(),
        Rvalue::Aggregate(
            AggregateKind::Tuple,
            vec![Operand::Move(field0), Operand::Copy(field1)],
        ),
        None,
    );
    let function = test_function_with_block(
        &mut type_context,
        BasicBlock {
            statements: vec![stmt.clone()],
            terminator: None,
        },
    );
    let analysis = InitializationAnalysis::new(&function, &type_context);
    let mut state = analysis.initial_state(&function);

    analysis.apply_statement(&mut state, &stmt);

    assert!(analysis.check_place_at_path(&destination, &state).is_ok());
}

#[test]
fn initialization_treats_closure_capture_locals_as_entry_initialized() {
    let owner = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1)));
    let capture_local = Local(1);
    let mut type_context = crate::type_context::TypeContext::new();
    let function = MirFunction {
        id: MirFunctionId::Closure(Box::new(MirClosureId {
            owner,
            local_index: 0,
        })),
        name: "lambda_0".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: Vec::new(),
            terminator: None,
        }],
        local_decls: vec![
            LocalDecl {
                ty: test_type_id(&mut type_context, Type::I64),
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: test_type_id(&mut type_context, Type::I64),
                mutability: Mutability::Not,
                name: Some("captured".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![MirClosureCapture {
            name: "captured".to_string(),
            local: capture_local,
            kind: MirClosureCaptureKind::ByRef,
            span: None,
        }],
        arg_count: 0,
        ret_type: test_type_id(&mut type_context, Type::I64),
        ownership: Default::default(),
    };

    let analysis = InitializationAnalysis::new(&function, &type_context);
    let state = analysis.initial_state(&function);

    assert_eq!(state.get_local(capture_local), Some(&InitState::Init));
}

#[test]
fn initialization_tracks_move_path_for_field_move() {
    use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
    use crate::mir::{Local, Place, Projection};

    let mut places = PlacePathTable::new();
    let root_place = places.intern(Place {
        local: Local(1),
        projection: Vec::new(),
    });
    let field_place = places.intern(Place {
        local: Local(1),
        projection: vec![Projection::Field {
            index: 0,
            identity: None,
        }],
    });
    let moves = MovePathTable::from_place_paths(&places);
    let mut state = InitMap::new_for_move_paths(&moves, InitState::Init);

    state.set_moved(
        field_place,
        &moves,
        MoveInfo {
            span: Some(crate::lexer::Span::test()),
        },
    );

    assert!(state.check_place_path(field_place, &moves).is_err());
    assert!(state.check_place_path(root_place, &moves).is_err());
}

#[test]
fn initialization_drop_allows_fully_moved_root() {
    use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
    use crate::mir::{Local, Place, Projection};

    let mut places = PlacePathTable::new();
    let root_place = places.intern(Place {
        local: Local(1),
        projection: Vec::new(),
    });
    places.intern(Place {
        local: Local(1),
        projection: vec![Projection::Field {
            index: 0,
            identity: None,
        }],
    });
    places.intern(Place {
        local: Local(1),
        projection: vec![Projection::Field {
            index: 1,
            identity: None,
        }],
    });
    let moves = MovePathTable::from_place_paths(&places);
    let mut state = InitMap::new_for_move_paths(&moves, InitState::Init);

    state.set_moved(
        root_place,
        &moves,
        MoveInfo {
            span: Some(crate::lexer::Span::test()),
        },
    );

    assert!(state.check_drop_place_path(root_place, &moves).is_ok());
}

#[test]
fn initialization_drop_rejects_partially_moved_root() {
    use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
    use crate::mir::{Local, Place, Projection};

    let mut places = PlacePathTable::new();
    let root_place = places.intern(Place {
        local: Local(1),
        projection: Vec::new(),
    });
    let field_place = places.intern(Place {
        local: Local(1),
        projection: vec![Projection::Field {
            index: 0,
            identity: None,
        }],
    });
    places.intern(Place {
        local: Local(1),
        projection: vec![Projection::Field {
            index: 1,
            identity: None,
        }],
    });
    let moves = MovePathTable::from_place_paths(&places);
    let mut state = InitMap::new_for_move_paths(&moves, InitState::Init);

    state.set_moved(
        field_place,
        &moves,
        MoveInfo {
            span: Some(crate::lexer::Span::test()),
        },
    );

    let error = state.check_drop_place_path(root_place, &moves).unwrap_err();

    assert_eq!(error.message, "drop of partially moved value");
    assert!(error.move_span.is_some());
}

#[test]
fn init_map_join_preserves_moved_over_init() {
    use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
    use crate::mir::{Local, Place, Projection};

    let mut places = PlacePathTable::new();
    let root_place = places.intern(Place {
        local: Local(1),
        projection: Vec::new(),
    });
    let field_place = places.intern(Place {
        local: Local(1),
        projection: vec![Projection::Field {
            index: 0,
            identity: None,
        }],
    });
    let moves = MovePathTable::from_place_paths(&places);
    let mut moved_state = InitMap::new_for_move_paths(&moves, InitState::Init);
    let init_state = InitMap::new_for_move_paths(&moves, InitState::Init);

    moved_state.set_moved(
        field_place,
        &moves,
        MoveInfo {
            span: Some(crate::lexer::Span::test()),
        },
    );

    moved_state.join(&init_state);

    let field_error = moved_state
        .check_place_path(field_place, &moves)
        .unwrap_err();
    let root_error = moved_state
        .check_place_path(root_place, &moves)
        .unwrap_err();

    assert_eq!(field_error.message, "use of moved value");
    assert_eq!(root_error.message, "use of partially moved value");
}

#[test]
fn init_map_join_requires_init_on_all_paths() {
    use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
    use crate::mir::{Local, Place, Projection};

    let mut places = PlacePathTable::new();
    let root_place = places.intern(Place {
        local: Local(1),
        projection: Vec::new(),
    });
    let field_place = places.intern(Place {
        local: Local(1),
        projection: vec![Projection::Field {
            index: 0,
            identity: None,
        }],
    });
    let moves = MovePathTable::from_place_paths(&places);
    let mut init_state = InitMap::new_for_move_paths(&moves, InitState::Uninit);
    let uninit_state = InitMap::new_for_move_paths(&moves, InitState::Uninit);

    init_state.set_place_path(root_place, &moves, InitState::Init);
    init_state.join(&uninit_state);

    let error = init_state
        .check_place_path(field_place, &moves)
        .unwrap_err();

    assert_eq!(error.message, "use of uninitialized place");
}

#[test]
fn initialization_errors_do_not_render_mir_local_ids() {
    use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
    use crate::mir::{Local, Operand, Place};

    let mut places = PlacePathTable::new();
    places.intern(Place {
        local: Local(9),
        projection: Vec::new(),
    });
    let moves = MovePathTable::from_place_paths(&places);
    let state = InitMap::new_for_move_paths(&moves, InitState::Uninit);
    let error = InitializationAnalysis::check_operand(
        &Operand::Move(Place {
            local: Local(9),
            projection: Vec::new(),
        }),
        &state,
    )
    .unwrap_err();

    assert_eq!(error.message, "use of uninitialized variable");
    assert!(!error.message.contains("Local("));
}
