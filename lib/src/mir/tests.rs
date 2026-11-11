use super::*;

fn span(start: usize, end: usize) -> Span {
    Span::new("/virtual/main.rk".into(), start, end)
}

#[test]
fn mir_origin_accessors_distinguish_source_and_synthetic_operations() {
    let source_span = span(2, 6);
    let source_statement = StatementData::assign(
        Place {
            local: Local(0),
            projection: vec![],
        },
        Rvalue::Use(Operand::Constant(Constant::Unit)),
        Some(source_span.clone()),
    );
    assert_eq!(
        source_statement.origin(),
        MirOrigin::Source(source_span.clone())
    );
    assert_eq!(
        StatementData::cleanup(StatementKind::StorageDead(Local(0)), None).origin(),
        MirOrigin::synthetic(MirSyntheticOrigin::Cleanup, None)
    );
    let linked = MirOrigin::synthetic(MirSyntheticOrigin::ControlFlow, Some(span(8, 9)));
    assert_eq!(linked.synthetic_origin(), Some(&span(8, 9)));

    let local = LocalDecl {
        ty: TypeContext::new().intern_type(&crate::types::Type::Unit),
        mutability: Mutability::Not,
        name: None,
        span: None,
        source: LocalSource::Temporary,
    };
    assert_eq!(
        local.origin(),
        MirOrigin::synthetic(MirSyntheticOrigin::Temporary, None)
    );
    let capture = MirClosureCapture {
        name: "value".to_string(),
        local: Local(0),
        kind: MirClosureCaptureKind::ByRef,
        span: Some(source_span),
    };
    assert!(matches!(capture.origin(), MirOrigin::Source(_)));

    let mut type_context = TypeContext::new();
    let unit = type_context.intern_type(&crate::types::Type::Unit);
    let function = MirFunction {
        id: MirFunctionId::Function(crate::ids::DefId::new(
            crate::ids::CrateId(0),
            crate::ids::LocalDefId(0),
        )),
        name: "main".to_string(),
        basic_blocks: vec![],
        local_decls: vec![
            local,
            LocalDecl {
                ty: unit,
                mutability: Mutability::Not,
                name: None,
                span: Some(span(20, 24)),
                source: LocalSource::Temporary,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: MirOwnershipMetadata::default(),
    };
    let call = Terminator::Call {
        func: Operand::Constant(Constant::Unit),
        args: vec![],
        destination: Place {
            local: Local(1),
            projection: vec![],
        },
        target: BasicBlockId(0),
        span: Some(span(2, 6)),
    };
    assert_eq!(
        function.terminator_origin(&call),
        MirOrigin::Source(span(2, 6))
    );

    let source = MirOrigin::Source(span(30, 35));
    assert_eq!(
        Terminator::switch_int(
            Operand::Constant(Constant::Bool(true)),
            vec![(1, BasicBlockId(1))],
            BasicBlockId(2),
            source.clone(),
        )
        .origin(),
        source
    );
    assert!(matches!(
        Terminator::goto(
            BasicBlockId(1),
            MirOrigin::synthetic(MirSyntheticOrigin::Cleanup, Some(span(40, 41))),
        )
        .origin(),
        MirOrigin::Synthetic {
            kind: MirSyntheticOrigin::Cleanup,
            origin: Some(_),
        }
    ));
}

#[test]
fn loop_owned_array_move_shape_requires_move_from_indexed_element() {
    let local = |index| Place {
        local: Local(index),
        projection: Vec::new(),
    };
    let element = Place {
        local: Local(1),
        projection: vec![Projection::Index(Local(3))],
    };
    let valid = StatementData::loop_owned_array_move(local(2), Operand::Move(element.clone()), None);
    assert!(valid.has_loop_owned_array_move_shape());

    let mut flagged_copy = StatementData::assign(local(2), Rvalue::Use(Operand::Copy(element)), None);
    flagged_copy.loop_owned_array_move = true;
    assert!(!flagged_copy.has_loop_owned_array_move_shape());

    let mut flagged_whole = StatementData::assign(local(2), Rvalue::Use(Operand::Move(local(1))), None);
    flagged_whole.loop_owned_array_move = true;
    assert!(!flagged_whole.has_loop_owned_array_move_shape());

    let plain = StatementData::assign(local(2), Rvalue::Use(Operand::Move(local(1))), None);
    assert!(plain.has_loop_owned_array_move_shape());
}
