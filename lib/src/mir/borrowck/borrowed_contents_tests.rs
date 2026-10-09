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

#[path = "returned_view_tests.rs"]
mod returned_view_tests;

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

fn reject_local_view(body: &str, result: &str) -> (String, MirProgram, crate::diagnostic::Diagnostics) {
    let source = format!(
        "{STORAGE}\nbad: () -> {result}\nbad = ->\n    pointer = (malloc 8) as *I64\n    owner = Storage\n        pointer: pointer\n{body}\nmain = !->\n    bad!\n"
    );
    let mir = source_mir(&source);
    let agreement = crate::mir::agreement::check_mir_runtime_agreement(&mir);
    assert!(
        agreement.is_clean()
            && agreement.object_contract_errors.is_empty()
            && agreement.erased_contract_errors.is_empty(),
        "{agreement:?}"
    );
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
    (source, mir, diagnostics)
}

fn assert_source_cleanup_rejection(
    source: &str,
    mir: &MirProgram,
    diagnostics: &crate::diagnostic::Diagnostics,
    exit_operation: &str,
    explicit_return: bool,
) {
    use crate::diagnostic::DiagnosticLocation;
    use crate::lexer::Span;
    use crate::mir::{MirOrigin, MirSyntheticOrigin, StatementKind, Terminator};

    let path = PathBuf::from("/virtual/borrowed_contents/main.rk");
    let exit_start = source.rfind(exit_operation).unwrap();
    let exit_span = Span::new(path.clone(), exit_start, exit_start + exit_operation.len());
    let owner_start = source.find("    owner = Storage").unwrap() + 4;
    let owner_span = Span::new(path, owner_start, owner_start + "owner".len());
    assert_eq!(&source[exit_span.start..exit_span.end], exit_operation);
    assert_eq!(&source[owner_span.start..owner_span.end], "owner");
    assert!(diagnostics.0.iter().all(|diagnostic| {
        diagnostic.code == Some(DiagnosticCode::Borrow)
            && matches!(&diagnostic.location, DiagnosticLocation::Source(span)
                if diagnostic.primary.as_ref().is_some_and(|label| &label.span == span))
    }), "{diagnostics:?}");
    for (message, span) in [
        ("borrow conflict on this value", &exit_span),
        ("Cannot return reference to local value because it does not live long enough", &owner_span),
    ] {
        assert_eq!(diagnostics.0.iter().filter(|diagnostic| {
            diagnostic.message == message
                && diagnostic.location == DiagnosticLocation::Source(span.clone())
                && diagnostic.primary.as_ref().is_some_and(|label| &label.span == span)
        }).count(), 1, "{diagnostics:?}");
    }
    let (function, owner) = mir.functions().find_map(|(_, function)| {
        function.basic_blocks.iter().find_map(|block| match &block.terminator {
            Some(Terminator::DropWithOrigin {
                place,
                origin: MirOrigin::Synthetic { kind: MirSyntheticOrigin::Cleanup, origin: Some(span) },
                ..
            }) if span == &exit_span => Some((function, place.local)),
            _ => None,
        })
    }).expect("actual cleanup Drop must link the exiting operation");
    assert!(function.basic_blocks.iter().any(|block| block.statements.iter().any(|statement| {
        matches!(&statement.kind, StatementKind::StorageDead(local) if *local == owner)
            && statement.cleanup && statement.span.as_ref() == Some(&exit_span)
    })), "cleanup StorageDead must retain the same exit span");
    assert!(function.basic_blocks.iter().any(|block| match &block.terminator {
        Some(Terminator::ReturnWithOrigin { origin }) => match origin {
            MirOrigin::Source(span) => explicit_return && span == &exit_span,
            MirOrigin::Synthetic { kind: MirSyntheticOrigin::ControlFlow, origin: Some(span) } =>
                !explicit_return && span == &exit_span,
            _ => false,
        },
        _ => false,
    }), "actual return must retain its explicit/implicit exit origin");
}

#[test]
fn source_pointer_helper_view_cannot_outlive_local_owner() {
    let (source, mir, diagnostics) = reject_local_view("    owner.view!", "&I64");
    assert_source_cleanup_rejection(&source, &mir, &diagnostics, "owner.view!", false);
}

#[test]
fn source_raw_pointer_result_keeps_owner_until_later_dereference() {
    reject_local_view("    raw = owner.pointer_view!\n    unsafe &*raw", "&I64");
}

#[test]
fn source_nested_aggregate_view_cannot_outlive_local_owner() {
    let (source, mir, diagnostics) = reject_local_view("    owner.nested!", "((&I64, I64), I64)");
    assert_source_cleanup_rejection(&source, &mir, &diagnostics, "owner.nested!", false);
}

#[test]
fn source_explicit_return_view_cleanup_uses_return_operand_origin() {
    let (source, mir, diagnostics) = reject_local_view("    return owner.view!", "&I64");
    assert_source_cleanup_rejection(&source, &mir, &diagnostics, "owner.view!", true);
}

#[test]
#[ignore = "historical pre-repair Internal-use-origin reproduction; expected to fail after repair, not acceptance"]
fn capture_borrowed_contents_cleanup_use_origins() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use crate::diagnostic::{DiagnosticLocation, Diagnostics};
    use crate::ids::{Idx, LoanId, TypeId};
    use crate::lexer::Span;
    use crate::mir::borrowck::{accesses, borrows, liveness, paths, provenance};
    use crate::mir::dataflow::analyses::{InitializationAnalysis, LoanAnalysis, LoanKind, LoanState, LoanTable};
    use crate::mir::dataflow::{run_fixpoint, Analysis};
    use crate::mir::{BasicBlockId, Operand, Rvalue, StatementKind, Terminator};

    fn fixture(body: &str, result: &str) -> String {
        // Byte-for-byte the existing reject_local_view input, not a corrected fixture.
        format!("{STORAGE}\nbad: () -> {result}\nbad = ->\n    pointer = (malloc 8) as *I64\n    owner = Storage\n        pointer: pointer\n{body}\nmain = !->\n    bad!\n")
    }

    fn digest(bytes: &[u8]) -> String {
        let mut child = Command::new("sha256sum").stdin(Stdio::piped())
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "source digest failed: {:?}", output.stderr);
        let digest = String::from_utf8(output.stdout).unwrap().split_whitespace().next().unwrap().to_owned();
        assert!(digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        digest
    }

    fn source_span(source: &str, span: Option<&Span>, seen: &mut HashSet<(usize, usize)>) {
        if let Some(span) = span {
            assert_eq!(span.file_path, PathBuf::from("/virtual/borrowed_contents/main.rk"));
            let bytes = source.get(span.start..span.end).expect("actual source-owned UTF-8 range");
            if seen.insert((span.start, span.end)) {
                eprintln!("CLEANUP_SOURCE span={span:?} bytes={bytes:?}");
            }
        }
    }

    fn observe_type(case: usize, tc: &TypeContext, id: TypeId, types: &mut HashSet<(usize, TypeId)>) {
        assert!(tc.type_id_tree_is_valid(id), "invalid observed TypeId {id:?}");
        types.insert((case, id));
        crate::type_services::visit::visit_type(&tc.type_for(id), &mut |ty: &Type| {
            if let Some(id) = tc.id_for_type(ty) { types.insert((case, id)); }
        });
        assert!(types.len() <= 96, "cleanup observer TypeID cap96 exceeded");
    }

    fn roles(state: &LoanState, id: LoanId) -> BTreeMap<&'static str, Vec<Local>> {
        // Display-only projection of existing Debug fields: no role getter is public.
        // This text never supplies alias, liveness, origin or oracle authority.
        let debug = format!("{state:?}");
        let record = debug.split("LoanOwners {").nth(id.index() + 1).expect("actual indexed loan roles");
        let mut result = BTreeMap::new();
        for role in ["storage", "contents", "raw", "raw_storage", "borrowed_raw", "returned"] {
            let marker = format!("{role}: LocalSet {{ bits: [");
            let bits = record.split_once(&marker).expect("existing role Debug shape").1
                .split_once(']').expect("bounded role bits").0;
            let mut owners = Vec::new();
            if !bits.is_empty() {
                for (index, bit) in bits.split(", ").enumerate() {
                    match bit {
                        "true" => {
                            let owner = Local(index);
                            assert!(state.owner_contains(id, owner), "Debug owner must agree with indexed API");
                            owners.push(owner);
                        }
                        "false" => {}
                        _ => panic!("unexpected role Debug bit {bit:?}"),
                    }
                }
            }
            result.insert(role, owners);
        }
        let displayed = result.values().flatten().copied().collect::<HashSet<_>>();
        assert_eq!(displayed, state.owners(id).iter().collect::<HashSet<_>>());
        result
    }

    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy();
        assert!(!["ROCK_OWNER_", "ROCK_ALLOCATOR_", "ROCK_INTEGRATION_BORROW_",
            "ROCK_FRESH_STDLIB_RETURN_", "ROCK_STATIC_CAPTURE_"].iter().any(|prefix| name.starts_with(prefix)),
            "broad trace/debug flag must be unset: {name}");
    }
    let mut functions = HashSet::new();
    let mut locations = HashSet::new();
    let mut loan_ids = HashSet::new();
    let mut types = HashSet::new();
    let mut alias_alternatives = 0;
    let mut total_conflicts = 0;
    let mut observations = Vec::new();
    for (case, (body, result_type, expected_start, expected_end)) in [
        ("    owner.nested!", "((&I64, I64), I64)", 653, 658),
        ("    owner.view!", "&I64", 639, 644),
    ].into_iter().enumerate() {
        let source = fixture(body, result_type);
        let before = source.as_bytes().to_vec();
        let before_hash = digest(&before);
        let mut spans = HashSet::new();
        eprintln!("CLEANUP_INPUT case={case} bytes={} SHA256={before_hash} source={source:?}", before.len());
        let mir = source_mir(&source);
        let agreement = crate::mir::agreement::check_mir_runtime_agreement(&mir);
        eprintln!("CLEANUP_AGREEMENT case={case} report={agreement:?}");
        assert!(agreement.is_clean() && agreement.object_contract_errors.is_empty()
            && agreement.erased_contract_errors.is_empty(), "actual unmodified source MIR must agree before Borrow");
        let production = BorrowChecker::run(&mir);
        if let Err(diagnostics) = &production {
            assert!(diagnostics.0.len() <= 32, "diagnostic cap32 exceeded");
            for diagnostic in &diagnostics.0 {
                eprintln!("CLEANUP_DIAGNOSTIC case={case} actual={diagnostic:?}");
                source_span(&source, diagnostic.primary.as_ref().map(|label| &label.span), &mut spans);
                for label in &diagnostic.secondary { source_span(&source, Some(&label.span), &mut spans); }
            }
        } else { eprintln!("CLEANUP_DIAGNOSTIC case={case} actual=Ok"); }

        let values = BorrowChecker::native_return_loan_proof(&mir);
        let mutations = super::NativeMutationTable::for_program(&mir);
        let tc = &mir.type_context;
        let contract = &mir.backend_contract;
        let mut case_conflicts = 0;
        for (function_id, function) in mir.functions() {
            let initialization = InitializationAnalysis::new(function, tc);
            let initialized = run_fixpoint(&initialization, function);
            let borrows = borrows::collect_function_borrows_with_contract(function, tc, contract, &values.summaries);
            let table = LoanTable::from_borrows_with_paths(&borrows, paths::PlacePathTable::for_function(function));
            let loan_locations = liveness::LoanLocationIndex::for_function(&table, function);
            let candidates = function.basic_blocks.iter().map(|block| block.terminator.as_ref()
                .map(|term| super::native_mutation_candidates(function, tc, contract, &mutations, term))
                .unwrap_or_default()).collect::<Vec<_>>();
            let live = liveness::compute_reference_liveness_with_mutations(function, tc, contract, &table, &candidates);
            let entries = liveness::compute_active_loan_entries_with_mutations(
                function, tc, contract, &table, &loan_locations, &live, &values.summaries, &candidates,
            );
            let mut selected = BTreeMap::new();
            let mut conflict_locals = HashSet::new();
            for (block_index, block) in function.basic_blocks.iter().enumerate() {
                if !initialized.reachable[block_index] { continue; }
                let mut init = initialized.entry_sets[block_index].clone();
                let mut loans = entries[block_index].clone();
                for index in 0..=block.statements.len() {
                    let location = super::location::Location::new(BasicBlockId(block_index), super::location::StatementIndex(index));
                    let mut replay = Diagnostics::default();
                    let (origin, events, access_state) = if let Some(statement) = block.statements.get(index) {
                        let mut before_use = loans.clone();
                        if let Some(set) = live.before_statement_sets.get(block_index).and_then(|sets| sets.get(index)) {
                            before_use.retain_owners(|owner| set.contains(owner));
                        }
                        BorrowChecker::check_statement_loans(statement, tc, contract, &table, &before_use, function, &mut replay);
                        let mut events = accesses::classify_statement(statement, tc);
                        if let StatementKind::Assign(destination, value) = &statement.kind {
                            if !matches!(value, Rvalue::Ref(_, _)) {
                                events.push(accesses::AccessEvent { kind: accesses::AccessKind::BorrowMut, place: destination.clone() });
                            }
                        }
                        (statement.origin(), events, before_use)
                    } else if let Some(term) = &block.terminator {
                        BorrowChecker::check_terminator(term, tc, contract, &initialization, &init, &table, &loans, function, &mut replay);
                        (term.origin(), accesses::classify_terminator(term), loans.clone())
                    } else { break; };
                    if !replay.0.is_empty() {
                        eprintln!("CLEANUP_REPLAY case={case} function={function_id:?} location={location:?} diagnostics={:?}", replay.0);
                        for event in events {
                            use accesses::AccessKind;
                            let kind = match event.kind {
                                AccessKind::Read | AccessKind::BorrowShared | AccessKind::ReturnedShared | AccessKind::CaptureShared => LoanKind::Shared,
                                AccessKind::Move => BorrowChecker::move_access_kind(tc, contract, function, &event.place),
                                AccessKind::Drop if index == block.statements.len() => LoanKind::Mut,
                                AccessKind::BorrowMut | AccessKind::ReturnedMut | AccessKind::CaptureMut => LoanKind::Mut,
                                AccessKind::RawPointerCast => {
                                    let StatementKind::Assign(_, Rvalue::Cast(operand, _)) = &block.statements[index].kind else { continue; };
                                    if matches!(operand, Operand::Move(_)) { LoanKind::Mut } else { LoanKind::Shared }
                                }
                                _ => continue,
                            };
                            let Err(id) = LoanAnalysis::check_aliasing(&event.place, kind, &table, &access_state) else { continue; };
                            case_conflicts += 1;
                            total_conflicts += 1;
                            assert!(total_conflicts <= 2, "more than the two approved actual conflicts");
                            selected.insert((block_index, index), "conflicting-use");
                            let loan = table.get(id).expect("actual conflicting LoanId");
                            loan_ids.insert((case, function_id.clone(), id));
                            assert!(loan_ids.len() <= 16, "LoanID cap16 exceeded");
                            conflict_locals.insert(event.place.local);
                            selected.entry((loan.created_at.block.0, loan.created_at.statement.0)).or_insert("loan-creation");
                            let aliases = provenance::resolve_places(&event.place, &table, &access_state);
                            let loan_aliases = provenance::resolve_loan_places(id, &table, &access_state);
                            alias_alternatives += aliases.len() + loan_aliases.len();
                            assert!(alias_alternatives <= 32, "resolved alias alternatives cap32 exceeded");
                            eprintln!("CLEANUP_CONFLICT case={case} function={function_id:?} location={location:?} access={event:?} kind={kind:?} use_origin={origin:?} source_only={:?} linked_synthetic={:?} path={:?} resolved_path={:?} aliases={aliases:?} loan_aliases={loan_aliases:?} loan={loan:?} current_roles={:?} returned_view={}",
                                origin.source_span(), origin.synthetic_origin(), table.place_paths().path_id(&event.place),
                                provenance::resolve_place_path(&event.place, &table, &access_state), roles(&access_state, id), table.is_returned_view(id));
                            eprintln!("CLEANUP_RESULT_AT_USE Local0_initialization={:?} Local0_active_carrier={}",
                                init.get_local(Local(0)), access_state.owner_contains(id, Local(0)));
                            source_span(&source, origin.source_span(), &mut spans);
                            source_span(&source, origin.synthetic_origin(), &mut spans);
                            source_span(&source, loan.origin_span.as_ref(), &mut spans);
                            for owner in access_state.owners(id).iter() {
                                conflict_locals.insert(owner);
                                observe_type(case, tc, function.local_decls[owner.0].ty, &mut types);
                            }
                            for place in std::iter::once(&event.place).chain(&aliases).chain(&loan_aliases) {
                                observe_type(case, tc, function.local_decls[place.local.0].ty, &mut types);
                                if let Some(ty) = contract.place_type_id(tc, function, place) { observe_type(case, tc, ty, &mut types); }
                            }
                        }
                    }
                    if let Some(statement) = block.statements.get(index) {
                        initialization.apply_statement(&mut init, statement);
                        liveness::apply_active_loan_statement_transfer(&mut loans, function, tc, contract, statement);
                        for id in loan_locations.loan_ids_at(location) { loans.activate(id, &table); }
                        if let Some(set) = live.after_statement_sets.get(block_index).and_then(|sets| sets.get(index)) {
                            loans.retain_owners(|owner| set.contains(owner));
                        }
                        if let StatementKind::StorageDead(local) = &statement.kind { loans.release_owner(*local); }
                    }
                }
            }
            if selected.is_empty() { continue; }
            functions.insert((case, function_id.clone()));
            assert!(functions.len() <= 8, "function cap8 exceeded");
            let key = contract.function_bodies.get(function_id).expect("actual native body key");
            let header = contract.callable(key).expect("actual canonical header");
            assert_eq!(&header.key, key);
            eprintln!("CLEANUP_FUNCTION case={case} function={function_id:?} actual_body_id={:?} key={key:?} source_definition={:?} kind={:?} signature_ABI={:?} mutation_knowledge={:?} ReturnLocal0={:?} ret_type={:?}",
                function.id, header.source_def_id, header.kind, header.signature, mutations.knowledge(key), function.local_decls[0], tc.type_for(function.ret_type));
            observe_type(case, tc, function.ret_type, &mut types);
            for param in &header.signature.params { observe_type(case, tc, param.semantic_ty, &mut types); }
            observe_type(case, tc, header.signature.ret.abi_ty, &mut types);
            for (block_index, block) in function.basic_blocks.iter().enumerate() {
                if !initialized.reachable[block_index] { continue; }
                for (index, statement) in block.statements.iter().enumerate() {
                    if matches!(&statement.kind, StatementKind::Assign(place, _) if place.local == Local(0)) {
                        selected.entry((block_index, index)).or_insert("return-initialization");
                    }
                    if matches!(&statement.kind, StatementKind::StorageDead(local) if conflict_locals.contains(local)) {
                        selected.entry((block_index, index)).or_insert("carrier-cleanup");
                    }
                }
                if let Some(term) = &block.terminator {
                    if matches!(term, Terminator::Return | Terminator::ReturnWithOrigin { .. })
                        || matches!(term, Terminator::Call { destination, .. } if destination.local == Local(0))
                        || matches!(term, Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } if conflict_locals.contains(&place.local))
                    {
                        selected.entry((block_index, block.statements.len())).or_insert("return-or-cleanup");
                    }
                }
            }
            for ((block_index, index), reason) in selected {
                locations.insert((case, function_id.clone(), block_index, index));
                assert!(locations.len() <= 32, "selected location cap32 exceeded");
                let block = &function.basic_blocks[block_index];
                let origin = if let Some(statement) = block.statements.get(index) {
                    for event in accesses::classify_statement(statement, tc) {
                        observe_type(case, tc, function.local_decls[event.place.local.0].ty, &mut types);
                        if let Some(ty) = contract.place_type_id(tc, function, &event.place) { observe_type(case, tc, ty, &mut types); }
                    }
                    if let StatementKind::Assign(_, Rvalue::Cast(_, target)) = &statement.kind {
                        observe_type(case, tc, *target, &mut types);
                    }
                    eprintln!("CLEANUP_OPERATION case={case} function={function_id:?} block={block_index} index={index} reason={reason} statement={statement:?}");
                    statement.origin()
                } else {
                    let term = block.terminator.as_ref().expect("actual selected terminator");
                    for event in accesses::classify_terminator(term) {
                        observe_type(case, tc, function.local_decls[event.place.local.0].ty, &mut types);
                        if let Some(ty) = contract.place_type_id(tc, function, &event.place) { observe_type(case, tc, ty, &mut types); }
                    }
                    if let Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } = term {
                        let ty = contract.place_type_id(tc, function, place).expect("actual typed drop place");
                        let key = contract.drop_glue.get(&ty);
                        eprintln!("CLEANUP_DROP_TYPE place={place:?} type_id={ty:?} type={:?} actual_glue_key={key:?} header={:?}",
                            tc.type_for(ty), key.and_then(|key| contract.callable(key)));
                        if let Some(header) = key.and_then(|key| contract.callable(key)) {
                            for param in &header.signature.params { observe_type(case, tc, param.semantic_ty, &mut types); }
                            observe_type(case, tc, header.signature.ret.semantic_ty, &mut types);
                            observe_type(case, tc, header.signature.ret.abi_ty, &mut types);
                        }
                    }
                    eprintln!("CLEANUP_OPERATION case={case} function={function_id:?} block={block_index} index={index} reason={reason} terminator={term:?} candidates={:?}", candidates[block_index]);
                    eprintln!("CLEANUP_RESULT_BLOCK_EXIT initialization={:?} live_at_entry={} live_at_exit={}",
                        initialized.exit_sets[block_index].get_local(Local(0)),
                        live.entry_sets[block_index].contains(Local(0)), live.exit_sets[block_index].contains(Local(0)));
                    term.origin()
                };
                eprintln!("CLEANUP_ORIGIN full={origin:?} source_only={:?} linked_synthetic={:?}", origin.source_span(), origin.synthetic_origin());
                source_span(&source, origin.source_span(), &mut spans);
                source_span(&source, origin.synthetic_origin(), &mut spans);
            }
        }
        assert_eq!(source.as_bytes(), before.as_slice(), "original virtual bytes changed");
        assert_eq!(fixture(body, result_type).as_bytes(), before.as_slice(), "fixture snapshot changed");
        let after_hash = digest(source.as_bytes());
        assert_eq!(after_hash, before_hash);
        eprintln!("CLEANUP_BYTES_AFTER case={case} bytes={} SHA256={after_hash}", source.len());
        eprintln!("CLEANUP_CAPS functions={} locations={} loans={} aliases={alias_alternatives} types={}", functions.len(), locations.len(), loan_ids.len(), types.len());
        observations.push((production, expected_start, expected_end, case_conflicts));
    }
    // Both inputs' actual diagnostics and byte receipts precede either oracle.
    for (production, expected_start, expected_end, case_conflicts) in observations {
        // Exact reproduction, not an AnyError oracle or permission to execute.
        let diagnostics = production.expect_err("current preservation rejection must reproduce");
        assert_eq!(diagnostics.0.len(), 2, "exact current diagnostic multiset");
        assert_eq!(diagnostics.0.iter().filter(|diagnostic| diagnostic.code == Some(DiagnosticCode::Internal)
            && diagnostic.message == "borrow conflict is missing its use origin"
            && diagnostic.location == DiagnosticLocation::Toolchain && diagnostic.primary.is_none()).count(), 1);
        let expected = Span::new(PathBuf::from("/virtual/borrowed_contents/main.rk"), expected_start, expected_end);
        assert_eq!(diagnostics.0.iter().filter(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
            && diagnostic.message == "Cannot return reference to local value because it does not live long enough"
            && diagnostic.location == DiagnosticLocation::Source(expected.clone())
            && diagnostic.primary.as_ref().is_some_and(|label| label.span == expected)).count(), 1);
        assert_eq!(case_conflicts, 1, "exactly one actual missing-use conflict per unchanged input");
    }
    assert_eq!(total_conflicts, 2);
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

#[test]
fn capture_and_reference_slot_storage_are_not_contained_reference_origins() {
    use crate::mir::dataflow::Analysis;

    let mut tc = TypeContext::new();
    let scalar = tc.intern_type(&Type::I64);
    let reference = tc.intern_type(&Type::Reference { inner: Box::new(Type::I64), mutable: false });
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1))),
        name: "capture_origins".into(), basic_blocks: Vec::new(),
        local_decls: [reference, scalar, reference].into_iter().enumerate().map(|(index, ty)| LocalDecl {
            ty, source: if index == 0 { LocalSource::ReturnPlace } else { LocalSource::ClosureCapture },
            mutability: Mutability::Not, name: None, span: None,
        }).collect(),
        closure_captures: Vec::new(), arg_count: 0, ret_type: reference, ownership: Default::default(),
    };
    let contract = MirBackendContract::default();
    let summaries = HashMap::new();
    let analysis = ReferenceEscapeAnalysis::new(&function, &tc, &contract, &summaries, MissingSummaryMode::Conservative);
    let state = analysis.initial_state(&function);
    for local in [Local(1), Local(2)] {
        let place = Place { local, projection: Vec::new() };
        assert_eq!(analysis.origins_for_borrowed_place(&state, &place), HashSet::from([ReferenceOrigin::Local(local)]));
    }
    assert_eq!(analysis.origins_for_place(&state, &Place { local: Local(2), projection: Vec::new() }), HashSet::from([ReferenceOrigin::Param(Local(2))]));
}

#[test]
fn unit_producer_cannot_hide_a_local_reference_in_borrowed_storage() {
    let mir = source_mir(r#"
struct Slot
    < value: &I64
store_bad: &mut Slot -> ()
store_bad = slot ->
    local = 7
    slot.value = &local
    ()
main = !->
    initial = 1
    mut slot = Slot
        value: &initial
    store_bad &mut slot
"#);
    let diagnostics = BorrowChecker::run(&mir).expect_err("unit return cannot conceal retained local storage");
    assert!(diagnostics.0.iter().any(|diagnostic| {
        diagnostic.code == Some(DiagnosticCode::Borrow)
            && diagnostic.message.contains("Cannot retain") && diagnostic.primary.is_some()
    }), "{diagnostics:?}");
}

#[test]
fn indexed_pointer_view_preserves_carrier_origin_but_slot_address_stays_local() {
    let mut tc = TypeContext::new();
    let scalar = tc.intern_type(&Type::I64);
    let pointer = tc.intern_type(&Type::Pointer(Box::new(Type::I64)));
    let reference = tc.intern_type(&Type::Reference { inner: Box::new(Type::I64), mutable: false });
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1))),
        name: "indexed_pointer_origins".into(), basic_blocks: Vec::new(),
        local_decls: [reference, pointer, pointer, scalar].into_iter().enumerate().map(|(index, ty)| LocalDecl {
            ty, source: match index {
                0 => LocalSource::ReturnPlace,
                1 => LocalSource::Argument,
                _ => LocalSource::Temporary,
            },
            mutability: Mutability::Not, name: None, span: None,
        }).collect(),
        closure_captures: Vec::new(), arg_count: 1, ret_type: reference, ownership: Default::default(),
    };
    let contract = MirBackendContract::default();
    let summaries = HashMap::new();
    let analysis = ReferenceEscapeAnalysis::new(&function, &tc, &contract, &summaries, MissingSummaryMode::Conservative);
    let slot = Place { local: Local(2), projection: Vec::new() };
    let indexed = Place { local: Local(2), projection: vec![Projection::Index(Local(3))] };
    let mut state = ReferenceOriginState::default();
    state.origins.insert(slot.clone(), HashSet::from([ReferenceOrigin::Param(Local(1))]));
    assert_eq!(analysis.origins_for_borrowed_place(&state, &indexed), HashSet::from([ReferenceOrigin::Param(Local(1))]));
    assert_eq!(analysis.origins_for_borrowed_place(&state, &slot), HashSet::from([ReferenceOrigin::Temporary(Local(2))]));
    state.origins.insert(slot, HashSet::from([ReferenceOrigin::ParamContents(Local(1))]));
    assert_eq!(analysis.origins_for_borrowed_place(&state, &indexed), HashSet::from([ReferenceOrigin::ParamBorrowedContents(Local(1))]));
}

#[test]
fn native_raw_getter_value_proof_keeps_distinct_lifetime_envelope() {
    let mir = source_mir(&format!("{STORAGE}\nmain = !->\n    pointer = (malloc 8) as *I64\n    owner = Storage\n        pointer: pointer\n    owner.pointer_view!\n"));
    let lifetime = BorrowChecker::reference_return_summaries(&mir);
    let values = BorrowChecker::native_return_loan_proof(&mir);
    assert!(values.summaries.iter().any(|(id, origins)| {
        origins == &HashSet::from([ReferenceOrigin::ParamContents(Local(1))])
            && lifetime.get(id) == Some(&HashSet::from([
                ReferenceOrigin::ParamBorrowedContents(Local(1)),
            ]))
    }), "actual copied-field body must not turn its lifetime envelope into storage access authority");
}

#[test]
fn native_external_only_storage_result_has_known_empty_input_effect() {
    let mir = source_mir(r#"
extern malloc: I64 -> *U8
struct Allocator
    < value: I64
impl Allocator
    @allocate = -> malloc 8
main = !->
    allocator = Allocator
        value: 7
    allocator.allocate!
"#);
    let lifetime = BorrowChecker::reference_return_summaries(&mir);
    let values = BorrowChecker::native_return_loan_proof(&mir);
    assert!(lifetime.iter().any(|(id, origins)| {
        origins == &HashSet::from([ReferenceOrigin::UnknownExternal])
            && values.summaries.get(id).is_some_and(HashSet::is_empty)
    }), "actual external storage with no input origins is not a missing native body summary");
}

#[test]
fn native_opaque_reference_leaf_keeps_parameter_effect() {
    let mir = source_mir(r#"
struct Holder
    < value: I64
extern opaque: &Holder -> *I64
forward = holder -> opaque holder
main = !->
    holder = Holder
        value: 7
    forward &holder
"#);
    let lifetime = BorrowChecker::reference_return_summaries(&mir);
    let values = BorrowChecker::native_return_loan_proof(&mir);
    assert!(values.summaries.iter().any(|(id, origins)| {
        origins.contains(&ReferenceOrigin::Param(Local(1)))
            && lifetime.get(id).is_some_and(|origins| {
                origins.contains(&ReferenceOrigin::UnknownExternal)
                    && origins.contains(&ReferenceOrigin::ParamBorrowedContents(Local(1)))
            })
    }), "bodyless reference inputs stay conservative in both distinct authorities");
}

#[test]
fn native_return_loan_proof_requires_exact_body_signature() {
    let mir = source_mir(r#"
extern malloc: I64 -> *U8
struct Allocator
    < value: I64
impl Allocator
    @allocate = -> malloc 8
main = !->
    allocator = Allocator
        value: 7
    allocator.allocate!
"#);
    let proof = BorrowChecker::native_return_loan_proof(&mir);
    let id = mir.functions().find(|(id, function)| {
        function.arg_count == 1 && proof.summaries.get(*id).is_some_and(HashSet::is_empty)
    }).expect("actual accepted native allocation body").0.clone();
    let key = mir.backend_contract.function_bodies[&id].clone();
    let mut changed = mir.clone();
    let different = changed.type_context.intern_type(&Type::Bool);
    for corruption in 0..5 {
        let mut invalid = changed.clone();
        match corruption {
            0 => invalid.backend_contract.callables.get_mut(&key).unwrap()
                .signature.params[0].semantic_ty = different,
            1 => invalid.backend_contract.callables.get_mut(&key).unwrap()
                .signature.ret.semantic_ty = different,
            2 => invalid.backend_contract.callables.get_mut(&key).unwrap()
                .signature.ret.abi_ty = different,
            3 => invalid.functions.get_mut(&id).unwrap().local_decls[0].ty = different,
            4 => invalid.functions.get_mut(&id).unwrap().arg_count = 0,
            _ => unreachable!(),
        }
        assert!(!BorrowChecker::native_return_loan_proof(&invalid).summaries.contains_key(&id),
            "native body/header disagreement must not grant input-loan proof: {corruption}");
    }
}
