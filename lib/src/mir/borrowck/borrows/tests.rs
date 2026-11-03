use super::{collect_statement_borrows, collect_terminator_borrows, AccessKind};
use crate::mir::borrowck::location::{Location, StatementIndex};
use crate::mir::{
    BasicBlockId, Constant, Local, MirCallable, MirCallableKey, MirClosure, MirClosureCaptureKind,
    MirClosureId, MirFunctionId, MirIntrinsicId, Mutability, Operand, Place, Rvalue, StatementData,
    Terminator,
};

#[test]
fn test_collect_borrow_from_ref_assignment() {
    let stmt = StatementData::assign(
        Place {
            local: Local(0),
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
    );

    let borrows =
        collect_statement_borrows(&stmt, Location::new(BasicBlockId(0), StatementIndex(0)));
    assert_eq!(borrows.len(), 1);
    assert_eq!(borrows[0].owner, Local(0));
    assert_eq!(borrows[0].place.local, Local(1));
    assert_eq!(borrows[0].kind, AccessKind::BorrowShared);
    assert_eq!(
        borrows[0].created_at,
        Location::new(BasicBlockId(0), StatementIndex(0))
    );
}

#[test]
fn test_collect_borrow_from_shared_closure_capture() {
    let stmt = StatementData::assign(
        Place {
            local: Local(2),
            projection: vec![],
        },
        Rvalue::Closure(MirClosure {
            id: MirClosureId {
                owner: MirFunctionId::Function(crate::ids::DefId::new(
                    crate::ids::CrateId(0),
                    crate::ids::LocalDefId(0),
                )),
                local_index: 0,
            },
            display_name: "lambda_0".to_string(),
            captures: vec![crate::mir::MirClosureCapture {
                name: "x".to_string(),
                local: Local(1),
                kind: MirClosureCaptureKind::ByRef,
                span: None,
            }],
        }),
        None,
    );

    let borrows =
        collect_statement_borrows(&stmt, Location::new(BasicBlockId(0), StatementIndex(0)));
    assert_eq!(borrows.len(), 1);
    assert_eq!(borrows[0].owner, Local(2));
    assert_eq!(borrows[0].place.local, Local(1));
    assert_eq!(borrows[0].kind, AccessKind::BorrowShared);
}

#[test]
fn raw_wide_pointer_construction_does_not_create_a_loan() {
    let terminator = Terminator::Call {
        func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
            MirCallableKey::Intrinsic(MirIntrinsicId::PtrFromParts),
        ))),
        args: vec![Operand::Copy(Place {
            local: Local(1),
            projection: vec![],
        })],
        destination: Place {
            local: Local(2),
            projection: vec![],
        },
        target: BasicBlockId(1),
        span: Some(crate::lexer::Span::test()),
    };

    let borrows = collect_terminator_borrows(
        &terminator,
        Location::new(BasicBlockId(0), StatementIndex(0)),
    );

    assert!(
        borrows.is_empty(),
        "raw pointer construction must not establish a loan"
    );
}
