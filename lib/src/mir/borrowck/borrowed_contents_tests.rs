use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use super::{BorrowChecker, MissingSummaryMode, ReferenceEscapeAnalysis, ReferenceOriginState};
use crate::diagnostic::DiagnosticCode;
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mir::builder::MirBuilder;
use crate::mir::{
    Local, LocalDecl, LocalSource, MirBackendContract, MirFunction, MirFunctionId, MirProgram,
    Mutability, Place, Projection, ReferenceOrigin,
};
use crate::mono::CompilationIdentityContext;
use crate::products::ProductCrateIdentity;
use crate::type_context::TypeContext;
use crate::types::Type;
use crate::{Config, SourceProvider};

const STORAGE: &str = r#"
extern malloc: I64 -> *U8
extern free: *U8 -> ()
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
struct Storage
    < pointer: *I64
unsafe dereference: *I64 -> &I64
unsafe dereference = pointer -> unsafe &*pointer
impl Storage
    @pointer_view: *I64
    @pointer_view = -> self.pointer
    @view: &I64
    @view = -> unsafe dereference self.pointer
    @slice: &[I64]
    @slice = -> unsafe ~BorrowSlice self.pointer, 1
    @nested: ((&I64, I64), I64)
    @nested = -> ((self.view!, 0), 0)
impl Dispose for Storage
    ~@dispose = !-> free (self.pointer as *U8)
"#;

fn source_mir(source: &str) -> MirProgram {
    let path = PathBuf::from("/virtual/borrowed_contents/main.rk");
    let frontend = crate::compile_frontend(&Config {
        entry_file: path.clone(),
        current_crate_name: Some("borrowed_contents".into()),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual {
            path,
            text: source.into(),
        }],
        ..Config::default()
    })
    .expect("source fixture must reach accepted HIR");
    let identity = CompilationIdentityContext::new(
        frontend.analysis.hir.root_crate_id,
        ProductCrateIdentity::local("borrowed_contents".into()),
        BTreeMap::new(),
    );
    let mut mono =
        crate::mono::monomorphize_with_crates(frontend.analysis.hir, &frontend.crate_ctx, identity)
            .expect("source fixture must monomorphize");
    let mut bodies = MirBuilder::take_mir_instance_bodies(&mut mono);
    crate::dce::prune_unreachable_instances(&mut mono, &mut bodies);
    MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies)
}

fn reject_local_view(body: &str, result: &str) {
    let source = format!(
        "{STORAGE}\nbad: () -> {result}\nbad = ->\n    pointer = (malloc 8) as *I64\n    owner = Storage\n        pointer: pointer\n{body}\nmain = !->\n    bad!\n"
    );
    let mir = source_mir(&source);
    assert!(
        BorrowChecker::reference_return_summaries(&mir)
            .values()
            .any(|origins| { origins.contains(&ReferenceOrigin::ParamBorrowedContents(Local(1))) }),
        "memory-view authority must come from the actual producer body"
    );
    let diagnostics = BorrowChecker::run(&mir).expect_err("view cannot outlive local storage");
    assert!(
        diagnostics.0.iter().any(|diagnostic| {
            diagnostic.code == Some(DiagnosticCode::Borrow)
                && diagnostic.message.contains("Cannot return reference")
                && diagnostic.primary.is_some()
        }),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .0
            .iter()
            .all(|diagnostic| { diagnostic.code != Some(DiagnosticCode::Internal) }),
        "{diagnostics:?}"
    );
}

#[test]
fn source_pointer_helper_view_cannot_outlive_local_owner() {
    reject_local_view("    owner.view!", "&I64");
}

#[test]
fn source_raw_pointer_result_keeps_owner_until_later_dereference() {
    reject_local_view("    raw = owner.pointer_view!\n    unsafe &*raw", "&I64");
}

#[test]
fn source_nested_aggregate_view_cannot_outlive_local_owner() {
    reject_local_view("    owner.nested!", "((&I64, I64), I64)");
}

#[test]
fn source_slice_view_keeps_owner_storage_loan_live() {
    let mir = source_mir(&format!("{STORAGE}\nmain = !->\n    pointer = (malloc 8) as *I64\n    mut owner = Storage\n        pointer: pointer\n    view = owner.slice!\n    owner.pointer = pointer\n    ~ArrayLen view\n"));
    let diagnostics = BorrowChecker::run(&mir).expect_err("live slice must prevent owner mutation");
    assert!(
        diagnostics.0.iter().any(|diagnostic| {
            diagnostic.code == Some(DiagnosticCode::Borrow)
                && diagnostic.message.contains("borrow")
                && diagnostic.primary.is_some()
        }),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .0
            .iter()
            .all(|diagnostic| { diagnostic.code != Some(DiagnosticCode::Internal) }),
        "{diagnostics:?}"
    );
    assert!(BorrowChecker::reference_return_summaries(&mir)
        .values()
        .any(|origins| { origins.contains(&ReferenceOrigin::ParamBorrowedContents(Local(1))) }));
}

#[test]
fn source_copied_reference_contents_remain_independent_of_holder_storage() {
    let mir = source_mir(
        r#"
struct Links
    < value: &I64
impl Links
    @copied: &I64
    @copied = -> self.value
copy: &I64 -> &I64
copy = input ->
    holder = Links
        value: input
    holder.copied!
main = !->
    value = 7
    result = copy &value
    *result
"#,
    );
    BorrowChecker::run(&mir).expect("copying an existing reference must not borrow holder storage");
    assert!(BorrowChecker::reference_return_summaries(&mir)
        .values()
        .any(|origins| { origins.contains(&ReferenceOrigin::ParamContents(Local(1))) }));
}

#[test]
fn borrowed_pointer_place_does_not_invent_missing_storage_provenance() {
    let mut tc = TypeContext::new();
    let pointer_ty = tc.intern_type(&Type::Pointer(Box::new(Type::I64)));
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1))),
        name: "untracked_pointer".into(),
        basic_blocks: vec![],
        local_decls: (0..2)
            .map(|_| LocalDecl {
                ty: pointer_ty,
                source: LocalSource::UserBinding,
                mutability: Mutability::Not,
                name: None,
                span: None,
            })
            .collect(),
        closure_captures: vec![],
        arg_count: 0,
        ret_type: pointer_ty,
        ownership: Default::default(),
    };
    let contract = MirBackendContract::default();
    let summaries = HashMap::new();
    let analysis = ReferenceEscapeAnalysis::new(
        &function,
        &tc,
        &contract,
        &summaries,
        MissingSummaryMode::Conservative,
    );
    let pointer = Place {
        local: Local(1),
        projection: vec![Projection::Deref],
    };
    let mut state = ReferenceOriginState::default();
    assert!(analysis
        .origins_for_borrowed_place(&state, &pointer)
        .is_empty());
    state.origins.insert(
        Place {
            local: Local(1),
            projection: vec![],
        },
        HashSet::from([ReferenceOrigin::ParamContents(Local(2))]),
    );
    assert_eq!(
        analysis.origins_for_borrowed_place(&state, &pointer),
        HashSet::from([ReferenceOrigin::ParamBorrowedContents(Local(2))])
    );
}

#[test]
fn source_tracked_raw_pointer_allows_unsafe_store_without_shared_access_rights() {
    for source in [
        "main = !->\n    value = 7\n    pointer = (&value) as *I64\n    unsafe *pointer = 9\n    unsafe *pointer\n".into(),
        format!("{STORAGE}\nmain = !->\n    pointer = (malloc 8) as *I64\n    owner = Storage\n        pointer: pointer\n    raw = owner.pointer_view!\n    unsafe *raw = 9\n    unsafe *raw\n"),
    ] {
        BorrowChecker::run(&source_mir(&source))
            .expect("raw provenance must not grant safe shared-reference access restrictions");
    }
}

#[test]
fn source_reference_reconstructed_from_raw_pointer_blocks_later_raw_write() {
    let mir = source_mir("main = !->\n    value = 7\n    pointer = (&value) as *I64\n    view = unsafe &*pointer\n    unsafe *pointer = 9\n    *view\n");
    let diagnostics =
        BorrowChecker::run(&mir).expect_err("live reconstructed reference is a real loan");
    assert!(
        diagnostics.0.iter().any(|diagnostic| {
            diagnostic.code == Some(DiagnosticCode::Borrow) && diagnostic.message.contains("borrow")
        }),
        "{diagnostics:?}"
    );
}

#[test]
fn source_safe_pointer_index_protocol_reconstructs_exclusive_access() {
    let mir = source_mir(
        r#"
lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue
lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue
impl ReadAt I64 for *I64
    type ReadValue = I64
    @read_at = index -> unsafe &*(~PtrOffset *self, index)
impl WriteAt I64 for *I64
    type WriteValue = I64
    ^@write_at = index -> unsafe &mut *(~PtrOffset *self, index)
main = ->
    mut value = 1
    mut pointer: *I64 = (&value) as *I64
    pointer[0] = 9
    pointer[0]
"#,
    );
    BorrowChecker::run(&mir)
        .expect("selected pointer index bodies grant actual reconstructed access");
}

#[test]
fn raw_lease_tracks_root_and_nested_views_without_safe_shared_access_permissions() {
    use super::accesses::AccessKind;
    use super::borrows::BorrowData;
    use super::location::{Location, StatementIndex};
    use crate::mir::dataflow::analyses::{LoanAnalysis, LoanKind, LoanState, LoanTable};
    use crate::mir::BasicBlockId;

    let place = |local, projection| Place {
        local: Local(local),
        projection,
    };
    let root = place(0, vec![]);
    let table = LoanTable::from_borrows(&[BorrowData {
        owner: Local(1),
        place: root.clone(),
        kind: AccessKind::BorrowShared,
        created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
        origin_span: None,
    }]);
    let loan = table.iter().next().unwrap().id;
    let mut state = LoanState::new(table.len());
    state.activate(loan, &table);
    state.copy_owner_as_raw(Local(1), Local(2));
    let raw_access = place(2, vec![Projection::Deref]);
    assert!(
        LoanAnalysis::check_aliasing(&raw_access, LoanKind::Mut, &table, &state).is_err(),
        "a still-live safe shared reference is not erased by a raw cast"
    );
    state.release_owner(Local(1));
    assert!(
        state.is_active(loan),
        "the raw carrier keeps the real storage lease"
    );
    assert!(LoanAnalysis::check_aliasing(&root, LoanKind::Mut, &table, &state).is_err());
    assert!(LoanAnalysis::check_aliasing(&raw_access, LoanKind::Mut, &table, &state).is_ok());

    state.copy_owner_for_borrow(Local(2), Local(3));
    state.copy_owner_as_contents(Local(3), Local(4));
    state.release_owner(Local(3));
    assert!(
        LoanAnalysis::check_aliasing(&raw_access, LoanKind::Mut, &table, &state).is_err(),
        "a view stored in an aggregate still blocks unsafe alias writes"
    );
    assert!(LoanAnalysis::check_aliasing(&root, LoanKind::Mut, &table, &state).is_err());
    state.release_owner(Local(4));
    assert!(LoanAnalysis::check_aliasing(&raw_access, LoanKind::Mut, &table, &state).is_ok());
}
