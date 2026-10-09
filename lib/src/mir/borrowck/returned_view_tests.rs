use super::{source_mir, BorrowChecker};
use crate::ids::LoanId;
use crate::mir::borrowck::accesses::AccessKind;
use crate::mir::borrowck::borrows::BorrowData;
use crate::mir::borrowck::location::{Location, StatementIndex};
use crate::mir::dataflow::analyses::{LoanAnalysis, LoanKind, LoanState, LoanTable};
use crate::mir::{BasicBlockId, Local, Place, Projection};

fn root(index: usize) -> Place {
    Place {
        local: Local(index),
        projection: Vec::new(),
    }
}

fn view(index: usize) -> Place {
    Place {
        local: Local(index),
        projection: vec![Projection::Deref],
    }
}

fn borrow(owner: usize, place: Place, kind: AccessKind) -> BorrowData {
    BorrowData {
        owner: Local(owner),
        place,
        kind,
        created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
        origin_span: None,
    }
}

fn state_with_view(extra: Vec<BorrowData>) -> (LoanTable, LoanState) {
    let mut borrows = vec![
        borrow(3, root(1), AccessKind::BorrowShared),
        borrow(4, root(2), AccessKind::BorrowMut),
    ];
    borrows.extend(extra);
    let table = LoanTable::from_borrows(&borrows);
    let mut state = LoanState::new(table.len());
    for loan in table.iter() {
        state.activate(loan.id, &table);
    }
    state.copy_owner_as_returned(Local(3), Local(5), false);
    state.copy_owner_as_returned(Local(4), Local(5), false);
    state.release_owner(Local(3));
    state.release_owner(Local(4));
    (table, state)
}

#[test]
fn exclusive_returned_view_keeps_shared_receiver_and_unique_input_leases() {
    let (table, mut state) = state_with_view(vec![borrow(5, view(5), AccessKind::ReturnedMut)]);
    LoanAnalysis::check_aliasing(&view(5), LoanKind::Mut, &table, &state).unwrap();
    assert_eq!(
        LoanAnalysis::check_aliasing(&root(1), LoanKind::Mut, &table, &state),
        Err(LoanId(0))
    );
    assert_eq!(
        LoanAnalysis::check_aliasing(&root(2), LoanKind::Shared, &table, &state),
        Err(LoanId(1))
    );
    state.release_owner(Local(5));
    LoanAnalysis::check_aliasing(&root(1), LoanKind::Mut, &table, &state).unwrap();
    LoanAnalysis::check_aliasing(&root(2), LoanKind::Mut, &table, &state).unwrap();
}

#[test]
fn lifetime_envelope_without_a_checked_exclusive_view_cannot_authorize_writes() {
    for extra in [
        Vec::new(),
        vec![borrow(5, view(5), AccessKind::ReturnedShared)],
    ] {
        let (table, state) = state_with_view(extra);
        assert!(LoanAnalysis::check_aliasing(&view(5), LoanKind::Mut, &table, &state).is_err());
    }
}

#[test]
fn origin_union_does_not_hide_an_independent_shared_alias() {
    for candidate in [1, 2] {
        let (table, state) = state_with_view(vec![
            borrow(5, view(5), AccessKind::ReturnedMut),
            borrow(6, root(candidate), AccessKind::BorrowShared),
        ]);
        assert_eq!(
            LoanAnalysis::check_aliasing(&view(5), LoanKind::Mut, &table, &state),
            Err(LoanId(3))
        );
    }
}

#[test]
fn moved_result_view_resolves_origins_from_its_current_carrier() {
    let (table, mut state) = state_with_view(vec![borrow(5, view(5), AccessKind::ReturnedMut)]);
    state.transfer_owner(Local(5), Local(7));
    state.release_owner(Local(5));
    LoanAnalysis::check_aliasing(&view(7), LoanKind::Mut, &table, &state).unwrap();
    assert!(LoanAnalysis::check_aliasing(&root(1), LoanKind::Mut, &table, &state).is_err());
    assert!(LoanAnalysis::check_aliasing(&root(2), LoanKind::Shared, &table, &state).is_err());
}

#[test]
fn an_ordinary_mutable_loan_does_not_certify_an_erased_result_envelope() {
    let (table, state) = state_with_view(vec![borrow(5, root(8), AccessKind::BorrowMut)]);
    assert_eq!(
        LoanAnalysis::check_aliasing(&view(5), LoanKind::Mut, &table, &state),
        Err(LoanId(0))
    );
}

#[test]
fn joined_safe_alias_remains_a_conflict_with_a_returned_exclusive_view() {
    let (table, mut state) = state_with_view(vec![borrow(5, view(5), AccessKind::ReturnedMut)]);
    let mut other_branch = state.clone();
    other_branch.add_owner(LoanId(0), Local(6));
    state.join(&other_branch);
    assert_eq!(
        LoanAnalysis::check_aliasing(&view(5), LoanKind::Mut, &table, &state),
        Err(LoanId(0))
    );
}

#[test]
fn a_copied_live_safe_alias_is_not_erased_into_a_lifetime_envelope() {
    let (table, mut state) = state_with_view(vec![borrow(5, view(5), AccessKind::ReturnedMut)]);
    state.add_owner(LoanId(0), Local(6));
    assert_eq!(
        LoanAnalysis::check_aliasing(&view(5), LoanKind::Mut, &table, &state),
        Err(LoanId(0))
    );
}

#[test]
fn separate_exclusive_results_with_the_same_envelope_still_conflict() {
    let (table, mut state) = state_with_view(vec![
        borrow(5, view(5), AccessKind::ReturnedMut),
        borrow(7, view(7), AccessKind::ReturnedMut),
    ]);
    state.copy_owner_as_returned(Local(5), Local(7), false);
    assert_eq!(
        LoanAnalysis::check_aliasing(&view(5), LoanKind::Mut, &table, &state),
        Err(LoanId(3))
    );
}

#[test]
fn contained_return_lease_does_not_retain_the_container_storage_loan() {
    let borrows = vec![
        borrow(3, root(1), AccessKind::BorrowShared),
        borrow(4, root(2), AccessKind::BorrowShared),
    ];
    let table = LoanTable::from_borrows(&borrows);
    let mut state = LoanState::new(table.len());
    for loan in table.iter() {
        state.activate(loan.id, &table);
    }
    state.copy_owner_as_contents(Local(4), Local(3));
    state.copy_owner_as_returned(Local(3), Local(5), true);
    state.release_owner(Local(3));
    state.release_owner(Local(4));
    assert!(!state.is_active(LoanId(0)));
    assert!(state.is_active(LoanId(1)));
}

const CALLBACKS: &str = r#"
lang fn_once
< trait Consume Args, Ret
    lang output
    type Output
    lang method
    ~@consume: Args -> Ret
lang fn_mut
< trait Change Args, Ret where Self: Consume Args, Ret
    lang output
    type Output
    lang method
    ^@change: Args -> Ret
lang fn
< trait Observe Args, Ret where Self: Change Args, Ret
    lang output
    type Output
    lang method
    @observe: Args -> Ret
< trait Callbacks
    @once: F -> T -> R where F: Consume T, R
< struct Service
    < marker: I64
impl Callbacks for Service
    @once = callback, value -> callback value
as_callbacks: &Service -> &Callbacks
< as_callbacks = service ->
    object: &Callbacks = service
    object
"#;

fn unique_source(before_write: &str) -> String {
    format!(
        r#"{CALLBACKS}
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    mut value = 0
    callback = reference -> reference
    result = object.once callback, &mut value
{before_write}    *result = 42
    value
"#
    )
}

#[test]
fn source_erased_unique_result_uses_its_checked_view_permission() {
    let mir = source_mir(&unique_source(""));
    BorrowChecker::run(&mir).expect("the receiver envelope is not the returned view's access kind");
}

#[test]
fn source_erased_unique_result_still_blocks_direct_input_mutation() {
    let mir = source_mir(&unique_source("    value = 1\n"));
    let diagnostics = BorrowChecker::run(&mir).expect_err("the input loan remains live");
    assert!(
        diagnostics.0.iter().any(|diagnostic| {
            diagnostic.code == Some(crate::diagnostic::DiagnosticCode::Borrow)
                && diagnostic.primary.is_some()
        }),
        "{diagnostics:?}"
    );
}

fn native_adapter_unique_source(before_write: &str) -> String {
    format!(
        r#"
invoke: F -> &mut I64 -> &mut I64 where F: FnOnce &mut I64, &mut I64
invoke = callback, value -> callback value
identity: &mut I64 -> &mut I64
identity = value -> value
main = !->
    mut value = 42
    adapter = inner, item, choose ->
        if choose then invoke inner, item
        else invoke inner, item
    selected = adapter identity, &mut value, true
    ()
{before_write}    *selected = 7
"#
    )
}

#[test]
fn source_native_unique_result_through_composed_adapter_keeps_view_permission() {
    let (mir, _) = super::super::allocator_trace_tests::mutation_source_program(
        "native_unique_adapter",
        &native_adapter_unique_source(""),
        true,
    );
    BorrowChecker::run(&mir)
        .expect("the composed returned unique view must authorize its pointee write");
}

#[test]
fn source_native_unique_adapter_result_still_blocks_direct_input_mutation() {
    let (mir, _) = super::super::allocator_trace_tests::mutation_source_program(
        "native_unique_adapter_conflict",
        &native_adapter_unique_source("    value = 1\n"),
        true,
    );
    let diagnostics = BorrowChecker::run(&mir).expect_err("the returned input loan remains live");
    assert!(
        diagnostics.0.iter().any(|diagnostic| {
            diagnostic.code == Some(crate::diagnostic::DiagnosticCode::Borrow)
                && diagnostic.message.contains("borrow conflict")
                && diagnostic.primary.is_some()
        }),
        "{diagnostics:?}"
    );
}

#[test]
fn source_erased_unique_result_cannot_escape_its_local_receiver_envelope() {
    let source = format!(
        r#"{CALLBACKS}
bad: &mut I64 -> &mut I64
bad = value ->
    service = Service
        marker: 0
    object = as_callbacks &service
    callback = reference -> reference
    object.once callback, value
main = ->
    mut value = 0
    result = bad &mut value
    *result = 42
    value
"#
    );
    let mir = source_mir(&source);
    let diagnostics =
        BorrowChecker::run(&mir).expect_err("receiver lifetime dependency must survive");
    assert!(
        diagnostics.0.iter().any(|diagnostic| {
            diagnostic.code == Some(crate::diagnostic::DiagnosticCode::Borrow)
                && diagnostic.message.contains("Cannot return reference")
                && diagnostic.primary.is_some()
        }),
        "{diagnostics:?}"
    );
}
