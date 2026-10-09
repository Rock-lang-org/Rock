//! Source-produced guards for the native primitive-view authority frontier.
//! Flat origins and a &&slice header cannot certify data rather than slot memory.
//! These observations grant no permissions and never reach LLVM or execution.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::allocator_trace_tests::{
    agent_work_path, integration_literal, integration_sha, mutation_source_program,
    GUARDED_STDLIB_SHA,
};
use super::*;
use crate::diagnostic::DiagnosticLocation;
use crate::ids::{Idx, LoanId};
use crate::mir::borrowck::location::{Location, StatementIndex};
use crate::mir::{BasicBlockId, MirCallableKind};

// Adjacent && is one operator token; nested reference types need separate & tokens.
// Signature parameters use chained arrows; lambda/call arguments use commas.
const VIEWS: &str = r#"
as_slice: &[I64] -> &[I64]
as_slice = value -> value
data_view: & &[I64] -> I64 -> &I64
data_view = metadata, index -> unsafe &*(~PtrOffset (~ArrPtr *metadata), index)
slice_view: & &[I64] -> &[I64]
slice_view = metadata -> unsafe ~BorrowSlice (~ArrPtr *metadata), 2
"#;

const SLOT: &str = r#"
slot_view: & &[I64] -> I64 -> &I64
slot_view = metadata, index -> unsafe &*(metadata as *I64)
"#;

const TEMP_ERROR: &str =
    "Cannot store reference to temporary value because it does not live long enough";
const LOCAL_ERROR: &str =
    "Cannot return reference to local value because it does not live long enough";

#[derive(Clone, Debug)]
struct LoanReceipt {
    id: LoanId,
    indexed: Place,
    aliases: Vec<Place>,
    roles: Vec<(&'static str, Vec<Local>)>,
    active: bool,
}

#[derive(Debug)]
struct Row {
    function: MirFunctionId,
    block: usize,
    index: usize,
    phase: &'static str,
    span: Option<Span>,
    place: Place,
    origins: OriginSet,
    aliases: Vec<Place>,
    loans: Vec<LoanReceipt>,
}

#[derive(Debug)]
struct Conflict {
    function: MirFunctionId,
    span: Span,
    loan: LoanId,
    borrow_span: Span,
}

#[derive(Default)]
struct Proof {
    functions: HashSet<MirFunctionId>,
    origins: HashSet<(MirFunctionId, ReferenceOrigin)>,
    loans: HashSet<(MirFunctionId, LoanId)>,
    paths: HashSet<(MirFunctionId, Place)>,
    rows: Vec<Row>,
    conflicts: Vec<Conflict>,
    writes: Vec<(MirFunctionId, Span, Place, bool)>,
    casts: Vec<(MirFunctionId, TypeId, TypeId)>,
    intrinsics: Vec<(MirFunctionId, MirIntrinsicId)>,
    absent: Vec<MirCallableKey>,
    released: HashSet<(MirFunctionId, Local)>,
    drops: usize,
}

impl Proof {
    fn budget(&self) {
        assert!(self.functions.len() <= 4 && self.origins.len() <= 16
            && self.loans.len() <= 16 && self.paths.len() <= 48 && self.rows.len() <= 512,
            "SLICE_SOURCE HARNESS OVERFLOW fn={}/4 origins={}/16 loans={}/16 paths={}/48 rows={}/512; no truncation or guessed proof",
            self.functions.len(), self.origins.len(), self.loans.len(), self.paths.len(), self.rows.len());
    }

    fn path(&mut self, function: &MirFunction, table: &LoanTable, place: &Place) {
        self.paths.insert((function.id.clone(), place.clone()));
        self.budget();
        eprintln!(
            "SLICE_SOURCE function={:?} actualPath={place:?} indexedId={:?}",
            function.id,
            table.place_paths().path_id(place)
        );
    }

    fn origin_set(&mut self, function: &MirFunction, origins: &OriginSet) {
        for origin in origins {
            self.origins.insert((function.id.clone(), *origin));
        }
        self.budget();
    }

    fn snapshot(
        &mut self,
        analysis: &ReferenceEscapeAnalysis<'_>,
        state: &ReferenceOriginState,
        table: &LoanTable,
        active: &LoanState,
        places: &[Place],
        block: usize,
        index: usize,
        phase: &'static str,
        span: Option<&Span>,
    ) {
        assert!(places.len() <= 16, "SLICE_SOURCE HARNESS site-place cap16");
        let function = analysis.func;
        let tc = analysis.type_context;
        let contract = analysis.backend_contract;
        let mut selected = HashSet::new();
        for place in places {
            for loan in table.iter() {
                if active.owner_contains(loan.id, place.local)
                    || loan.initial_owner == place.local
                    || table
                        .place_paths()
                        .place(loan.place_path)
                        .is_some_and(|path| path.local == place.local)
                {
                    selected.insert(loan.id);
                }
            }
        }
        // A result can stop carrying a slot loan while independent aliases retain it.
        selected.extend(
            self.loans
                .iter()
                .filter_map(|(owner, id)| (owner == &function.id).then_some(*id)),
        );
        assert!(
            selected.len() <= 16,
            "SLICE_SOURCE HARNESS complete selected-loan array cap16"
        );
        let debug = format!("{active:?}");
        assert!(
            debug.len() <= 2_097_152,
            "SLICE_SOURCE HARNESS Debug bytes cap2097152"
        );
        let owner_rows = debug
            .split_once("owners: [")
            .expect("actual LoanState Debug schema")
            .1;
        let mut loan_rows = Vec::new();
        for id in selected {
            self.loans.insert((function.id.clone(), id));
            self.budget();
            let loan = table.get(id).expect("actual selected LoanId");
            let indexed = table
                .place_paths()
                .place(loan.place_path)
                .expect("actual indexed loan path");
            self.path(function, table, indexed);
            self.path(function, table, &loan.diagnostic_place);
            let row = owner_rows
                .split("LoanOwners {")
                .nth(id.index() + 1)
                .expect("actual indexed role row");
            let mut roles = Vec::new();
            let mut union = HashSet::new();
            // Debug observation only: no production accessor or role mutation.
            for role in [
                "storage",
                "contents",
                "raw",
                "raw_storage",
                "borrowed_raw",
                "returned",
            ] {
                let marker = format!("{role}: LocalSet {{ bits: [");
                let bits = row
                    .split_once(marker.as_str())
                    .expect("actual role schema")
                    .1
                    .split_once(']')
                    .unwrap()
                    .0;
                let mut owners = Vec::new();
                if !bits.is_empty() {
                    for (index, bit) in bits.split(", ").enumerate() {
                        assert!(
                            matches!(bit, "true" | "false"),
                            "SLICE_SOURCE HARNESS unknown role bit"
                        );
                        if bit == "true" {
                            assert!(index < function.local_decls.len());
                            let local = Local(index);
                            owners.push(local);
                            union.insert(local);
                        }
                    }
                }
                assert!(
                    owners.len() <= 16,
                    "SLICE_SOURCE HARNESS ALL role carriers cap16"
                );
                if role == "storage" {
                    for index in 0..function.local_decls.len() {
                        assert_eq!(
                            owners.contains(&Local(index)),
                            active.storage_owner_contains(id, Local(index))
                        );
                    }
                }
                roles.push((role, owners));
            }
            assert!(union.len() <= 16, "SLICE_SOURCE HARNESS owner union cap16");
            assert_eq!(union, active.owners(id).iter().collect::<HashSet<_>>());
            for owner in union {
                let root = Place {
                    local: owner,
                    projection: Vec::new(),
                };
                self.path(function, table, &root);
                let mut query = root;
                if contract
                    .place_type_id(tc, function, &query)
                    .is_some_and(|ty| {
                        matches!(tc.try_ty(ty), Some(Ty::Reference { .. } | Ty::Pointer(_)))
                    })
                {
                    query.projection.push(Projection::Deref);
                }
                self.path(function, table, &query);
                let aliases = provenance::resolve_places(&query, table, active);
                assert!(
                    aliases.len() <= 32,
                    "SLICE_SOURCE HARNESS ALL owner aliases cap32"
                );
                for alias in &aliases {
                    self.path(function, table, alias);
                }
                eprintln!("SLICE_SOURCE function={:?} location=({block},{index}) phase={phase} loan={id:?} roleCarrier={owner:?} ALL_aliases={aliases:?}", function.id);
            }
            let aliases = provenance::resolve_loan_places(id, table, active);
            assert!(
                aliases.len() <= 32,
                "SLICE_SOURCE HARNESS ALL loan aliases cap32"
            );
            for alias in &aliases {
                self.path(function, table, alias);
            }
            eprintln!("SLICE_SOURCE function={:?} location=({block},{index}) phase={phase} actualLoan={loan:?} indexed={indexed:?} active={} returnedView={} ALL_roles={roles:?} ALL_loanAliases={aliases:?}",
                function.id, active.is_active(id), table.is_returned_view(id));
            loan_rows.push(LoanReceipt {
                id,
                indexed: indexed.clone(),
                aliases,
                roles,
                active: active.is_active(id),
            });
        }
        for place in places {
            let checked = contract
                .place_type_id(tc, function, place)
                .expect("actual selected projected type");
            assert!(tc.type_id_tree_is_valid(checked));
            let origins = analysis.origins_for_place(state, place);
            self.origin_set(function, &origins);
            let mut queries = vec![place.clone()];
            if matches!(
                tc.try_ty(checked),
                Some(Ty::Reference { .. } | Ty::Pointer(_))
            ) {
                let mut deref = place.clone();
                deref.projection.push(Projection::Deref);
                queries.push(deref);
            }
            for query in queries {
                self.path(function, table, &query);
                let aliases = provenance::resolve_places(&query, table, active);
                assert!(
                    aliases.len() <= 32,
                    "SLICE_SOURCE HARNESS ALL carrier aliases cap32"
                );
                for alias in &aliases {
                    self.path(function, table, alias);
                }
                eprintln!("SLICE_SOURCE function={:?} location=({block},{index}) phase={phase} span={span:?} actualPlace={query:?} checkedType={:?} semantic={:?} declaration={:?} origins={origins:?} ALL_aliases={aliases:?}; no access permission inferred",
                    function.id, contract.place_type_id(tc, function, &query),
                    contract.place_type_id(tc, function, &query).map(|ty| tc.type_for(ty)), function.local_decls[place.local.0]);
                self.rows.push(Row {
                    function: function.id.clone(),
                    block,
                    index,
                    phase,
                    span: span.cloned(),
                    place: query,
                    origins: origins.clone(),
                    aliases,
                    loans: loan_rows.clone(),
                });
                self.budget();
            }
        }
    }
}

fn interesting(program: &MirProgram, function: &MirFunction, place: &Place) -> bool {
    program
        .backend_contract
        .place_type_id(&program.type_context, function, place)
        .is_some_and(|ty| {
            matches!(program.type_context.try_ty(ty), Some(Ty::Array { .. }))
                || type_id_tracks_origin(
                    TypeView::new(&program.type_context),
                    &program.backend_contract,
                    ty,
                )
        })
}

fn operand_places(value: &Rvalue) -> Vec<Place> {
    let mut places = Vec::new();
    let mut add = |operand: &Operand| {
        if let Operand::Copy(place) | Operand::Move(place) = operand {
            places.push(place.clone());
        }
    };
    match value {
        Rvalue::Use(op)
        | Rvalue::Cast(op, _)
        | Rvalue::Object(op, _)
        | Rvalue::Package(op, _)
        | Rvalue::UnaryOp(_, op) => add(op),
        Rvalue::BinaryOp(_, left, right) => {
            add(left);
            add(right);
        }
        Rvalue::Aggregate(_, operands) => {
            for op in operands {
                add(op);
            }
        }
        Rvalue::Ref(_, place) | Rvalue::Discriminant(place) => places.push(place.clone()),
        Rvalue::Closure(closure) => {
            for capture in &closure.captures {
                places.push(capture.place());
            }
        }
    }
    assert!(
        places.len() <= 8,
        "SLICE_SOURCE HARNESS operand-place array cap8"
    );
    places
}

fn operand_type(program: &MirProgram, function: &MirFunction, operand: &Operand) -> TypeId {
    let tc = &program.type_context;
    let checked = match operand {
        Operand::Copy(place) | Operand::Move(place) => {
            program.backend_contract.place_type_id(tc, function, place)
        }
        Operand::Constant(Constant::Int(_)) => tc.id_for_type(&Type::I64),
        Operand::Constant(Constant::Bool(_)) => tc.id_for_type(&Type::Bool),
        Operand::Constant(Constant::Unit) => tc.id_for_type(&Type::Unit),
        Operand::Constant(Constant::TypeId(id)) => Some(*id),
        _ => None,
    }
    .expect(
        "SLICE_SOURCE HARNESS actual operand type unavailable; no guessed representative signature",
    );
    assert!(tc.type_id_tree_is_valid(checked));
    checked
}

fn line_range(source: &str, marker: &str) -> (usize, usize) {
    let start = source.find(marker).expect("fixed source marker");
    assert!(
        source[start + marker.len()..].find(marker).is_none(),
        "SLICE_SOURCE HARNESS ambiguous source marker"
    );
    (start, start + marker.len())
}

fn within(span: &Span, entry: &Path, range: (usize, usize)) -> bool {
    span.file_path == entry && span.start >= range.0 && span.end <= range.1
}

fn binding(
    program: &MirProgram,
    entry: &Path,
    source: &str,
    marker: &str,
    name: &str,
) -> (MirFunctionId, Local) {
    let range = line_range(source, marker);
    let found = program
        .functions()
        .flat_map(|(id, function)| {
            function
                .local_decls
                .iter()
                .enumerate()
                .filter_map(move |(index, decl)| {
                    (decl.source == LocalSource::UserBinding
                        && decl.name.as_deref() == Some(name)
                        && decl
                            .span
                            .as_ref()
                            .is_some_and(|span| within(span, entry, range)))
                    .then_some((id.clone(), Local(index)))
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(found.len(), 1, "SLICE_SOURCE HARNESS actual source binding {marker:?}; names only select fixture spans, never authority");
    found[0].clone()
}

fn inspect(program: &MirProgram, root: MirFunctionId) -> Proof {
    let tc = &program.type_context;
    let contract = &program.backend_contract;
    let agreement = crate::mir::backend_contract::validate_backend_contract_against_mir(
        contract,
        program.functions.values(),
    );
    assert!(
        agreement.is_empty(),
        "SLICE_SOURCE HARNESS canonical ABI agreement: {agreement:?}"
    );
    let lifetime = BorrowChecker::reference_return_summaries(program);
    let values = BorrowChecker::native_return_loan_proof(program);
    let mutations = NativeMutationTable::for_program(program);
    let mut proof = Proof::default();
    let mut queue = VecDeque::from([root]);
    while let Some(id) = queue.pop_front() {
        if !proof.functions.insert(id.clone()) {
            continue;
        }
        proof.budget();
        let function = program
            .functions
            .get(&id)
            .expect("selected canonical native body PRESENT");
        let key = contract
            .function_bodies
            .get(&id)
            .expect("actual body mapping");
        let header = contract.callable(key).expect("actual header");
        assert_eq!(&header.key, key);
        assert_eq!(
            MirCallable::Resolved(key.clone()).function_id().as_ref(),
            Some(&id)
        );
        assert_eq!(function.id, id);
        assert!(
            matches!(&header.kind, MirCallableKind::LocalBody { function_id } if function_id == &id)
        );
        assert_eq!(header.signature.params.len(), function.arg_count);
        assert_eq!(header.signature.ret.semantic_ty, function.ret_type);
        // The canonical validator owns permitted semantic/ABI distinctions.
        assert!(tc.type_id_tree_is_valid(header.signature.ret.abi_ty));
        assert_eq!(function.local_decls[0].ty, function.ret_type);
        assert!(
            function.arg_count <= 8,
            "SLICE_SOURCE HARNESS parameter cap8"
        );
        for (index, param) in header.signature.params.iter().enumerate() {
            assert_eq!(param.semantic_ty, function.local_decls[index + 1].ty);
            assert!(tc.type_id_tree_is_valid(param.semantic_ty));
        }
        eprintln!("SLICE_SOURCE actualFunction={id:?} key={key:?} declaration={header:?} body=PRESENT Lifetime={:?} ValueLoans={:?} mutationKnowledge={:?}; None=UNKNOWN, header/roles alone cannot prove primitive data source",
            lifetime.get(&id), values.summaries.get(&id), mutations.knowledge(key));
        for summary in [lifetime.get(&id), values.summaries.get(&id)]
            .into_iter()
            .flatten()
        {
            proof.origin_set(function, summary);
        }
        let candidates = function
            .basic_blocks
            .iter()
            .map(|block| {
                block
                    .terminator
                    .as_ref()
                    .map(|term| {
                        native_mutation_candidates(function, tc, contract, &mutations, term)
                    })
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        let (_, mutation_origins) = BorrowChecker::validate_moves_with_return_sources(
            function,
            tc,
            contract,
            &values.summaries,
            &mutations,
            Some(&values.sources),
        );
        let mut analysis = ReferenceEscapeAnalysis::new(
            function,
            tc,
            contract,
            &lifetime,
            MissingSummaryMode::Conservative,
        )
        .with_return_sources(Some(&values.sources));
        analysis.mutation_origins = Some(&mutation_origins);
        let results = run_fixpoint(&analysis, function);
        let borrows = borrows::collect_function_borrows_with_return_sources(
            function,
            tc,
            contract,
            &values.summaries,
            Some(&values.sources),
        );
        let table = LoanTable::from_borrows_with_paths(
            &borrows,
            paths::PlacePathTable::for_function(function),
        );
        let locations = LoanLocationIndex::for_function(&table, function);
        let live = liveness::compute_reference_liveness_with_mutations(
            function,
            tc,
            contract,
            &table,
            &candidates,
        );
        let entries = liveness::compute_active_loan_entries_with_return_sources(
            function,
            tc,
            contract,
            &table,
            &locations,
            &live,
            &values.summaries,
            &candidates,
            Some(&values.sources),
        );
        let active_analysis = liveness::ActiveLoanAnalysis::new(
            tc,
            contract,
            &table,
            &locations,
            &live,
            &values.summaries,
        )
        .with_mutations(&candidates)
        .with_return_sources(Some(&values.sources));
        for (block, data) in function.basic_blocks.iter().enumerate() {
            if !results.reachable[block] {
                continue;
            }
            let mut state = results.entry_sets[block].clone();
            let mut active = entries[block].clone();
            for (index, stmt) in data.statements.iter().enumerate() {
                let mut live_before = active.clone();
                if let Some(set) = live
                    .before_statement_sets
                    .get(block)
                    .and_then(|sets| sets.get(index))
                {
                    live_before.retain_owners(|owner| set.contains(owner));
                }
                let mut places = Vec::new();
                if let StatementKind::Assign(dest, value) = &stmt.kind {
                    if interesting(program, function, dest) || !dest.projection.is_empty() {
                        places.push(dest.clone());
                        let operands = operand_places(value);
                        places.extend(operands.into_iter().filter(|place| {
                            interesting(program, function, place)
                                || matches!(value, Rvalue::Ref(_, _))
                        }));
                        eprintln!("SLICE_SOURCE function={id:?} location=({block},{index}) statement={stmt:?} temporaryClassification={:?}", analysis.temporary_origins_for_assignment(&state, dest, value));
                        if let Rvalue::Cast(operand, target) = value {
                            let source = operand_type(program, function, operand);
                            eprintln!("SLICE_SOURCE actualCast source={source:?}/{:?} target={target:?}/{:?}; cast is NOT pointee/backing-owner equality", tc.type_for(source), tc.type_for(*target));
                            proof.casts.push((id.clone(), source, *target));
                        }
                        if !matches!(value, Rvalue::Ref(_, _)) {
                            let conflict = LoanAnalysis::check_aliasing(
                                dest,
                                LoanKind::Mut,
                                &table,
                                &live_before,
                            );
                            if let Some(span) = &stmt.span {
                                proof.writes.push((
                                    id.clone(),
                                    span.clone(),
                                    dest.clone(),
                                    conflict.is_ok(),
                                ));
                                if let Err(loan_id) = conflict {
                                    let loan = table.get(loan_id).unwrap();
                                    proof.conflicts.push(Conflict {
                                        function: id.clone(),
                                        span: span.clone(),
                                        loan: loan_id,
                                        borrow_span: loan
                                            .origin_span
                                            .clone()
                                            .expect("actual conflicting borrow origin"),
                                    });
                                }
                            }
                        }
                    }
                }
                if !places.is_empty() {
                    proof.snapshot(
                        &analysis,
                        &state,
                        &table,
                        &live_before,
                        &places,
                        block,
                        index,
                        "BEFORE_STATEMENT_FROM_JOINED_ENTRY",
                        stmt.span.as_ref(),
                    );
                }
                analysis.apply_statement(&mut state, stmt);
                apply_active_loan_statement_transfer(&mut active, function, tc, contract, stmt);
                for loan in
                    locations.loan_ids_at(Location::new(BasicBlockId(block), StatementIndex(index)))
                {
                    active.activate(loan, &table);
                }
                if let Some(set) = live
                    .after_statement_sets
                    .get(block)
                    .and_then(|sets| sets.get(index))
                {
                    active.retain_owners(|owner| set.contains(owner));
                }
                if let StatementKind::StorageDead(local) = &stmt.kind {
                    active.release_owner(*local);
                    if interesting(
                        program,
                        function,
                        &Place {
                            local: *local,
                            projection: Vec::new(),
                        },
                    ) {
                        assert!(!table
                            .iter()
                            .any(|loan| active.owner_contains(loan.id, *local)));
                        proof.released.insert((id.clone(), *local));
                        eprintln!("SLICE_SOURCE function={id:?} location=({block},{index}) actualStorageDead={local:?} carrierReleased=true; other LoanId roles untouched");
                    }
                }
                if !places.is_empty() {
                    proof.snapshot(
                        &analysis,
                        &state,
                        &table,
                        &active,
                        &places,
                        block,
                        index,
                        "AFTER_EXACT_STATEMENT",
                        stmt.span.as_ref(),
                    );
                }
            }
            let Some(term) = &data.terminator else {
                continue;
            };
            let mut places = Vec::new();
            match term {
                Terminator::Call {
                    func,
                    args,
                    destination,
                    ..
                } => {
                    assert!(
                        args.len() <= 8 && candidates[block].len() <= 16,
                        "SLICE_SOURCE HARNESS args8/mutation16"
                    );
                    if interesting(program, function, destination) {
                        places.push(destination.clone());
                    }
                    for arg in args {
                        if let Operand::Copy(place) | Operand::Move(place) = arg {
                            if interesting(program, function, place) {
                                places.push(place.clone());
                            }
                        }
                    }
                    if !places.is_empty() {
                        let Operand::Constant(Constant::Callable(MirCallable::Resolved(call_key))) =
                            func
                        else {
                            panic!(
                                "SLICE_SOURCE HARNESS unhandled actual callable operand {func:?}"
                            );
                        };
                        let declaration =
                            contract.callable(call_key).expect("actual selected header");
                        assert_eq!(&declaration.key, call_key);
                        let params = args
                            .iter()
                            .map(|arg| operand_type(program, function, arg))
                            .collect::<Vec<_>>();
                        let ret = contract
                            .place_type_id(tc, function, destination)
                            .expect("actual checked result");
                        eprintln!("SLICE_SOURCE function={id:?} location=({block},terminator) actualCall={term:?} canonicalHeader={declaration:?} actualParams={params:?}/{:?} projectedResult={ret:?}/{:?} safeReferenceEligible={} mutationCandidates={:?} call_return_origins={:?} temporaryClassification={:?}",
                            params.iter().map(|ty| tc.type_for(*ty)).collect::<Vec<_>>(), tc.type_for(ret), return_type_needs_escape_check(tc, contract, ret), candidates[block],
                            analysis.call_return_origins(&state, func, args, destination), analysis.temporary_origins_for_call(&state, func, args, destination));
                        if let MirCallableKey::Intrinsic(intrinsic) = call_key {
                            assert!(
                                matches!(&declaration.kind, MirCallableKind::Intrinsic(actual) if actual == intrinsic)
                            );
                            proof.intrinsics.push((id.clone(), intrinsic.clone()));
                            eprintln!("SLICE_SOURCE typedIntrinsic={intrinsic:?} representativeHeaderOnly=true actualPerCallTypesAbove; no signature equivalence or new source certificate");
                        } else {
                            assert_eq!(params.len(), declaration.signature.params.len());
                            for (actual, expected) in
                                params.iter().zip(&declaration.signature.params)
                            {
                                assert_eq!(*actual, expected.semantic_ty);
                            }
                            assert_eq!(ret, declaration.signature.ret.semantic_ty);
                            assert_eq!(ret, declaration.signature.ret.abi_ty);
                        }
                        if let MirCallableKind::LocalBody { function_id } = &declaration.kind {
                            assert_eq!(contract.function_bodies.get(function_id), Some(call_key));
                            queue.push_back(function_id.clone());
                        } else if !matches!(declaration.kind, MirCallableKind::Intrinsic(_)) {
                            assert!(matches!(declaration.kind, MirCallableKind::Extern { .. }));
                            if let Some(function_id) =
                                MirCallable::Resolved(call_key.clone()).function_id()
                            {
                                assert!(
                                    lifetime.get(&function_id).is_none()
                                        && values.summaries.get(&function_id).is_none()
                                );
                                assert!(!program.functions.contains_key(&function_id));
                            }
                            proof.absent.push(call_key.clone());
                            eprintln!("SLICE_SOURCE key={call_key:?} body=ABSENT Lifetime=None ValueLoans=None mutation={:?}; UNKNOWN, not known-empty", mutations.knowledge(call_key));
                            assert_eq!(
                                mutations.knowledge(call_key),
                                NativeMutationKnowledge::Unknown
                            );
                        }
                    }
                }
                Terminator::Return | Terminator::ReturnWithOrigin { .. } => {
                    if interesting(
                        program,
                        function,
                        &Place {
                            local: Local(0),
                            projection: Vec::new(),
                        },
                    ) {
                        places.push(Place {
                            local: Local(0),
                            projection: Vec::new(),
                        });
                    }
                }
                Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } => {
                    places.push(place.clone());
                    let ty = contract
                        .place_type_id(tc, function, place)
                        .expect("actual drop type");
                    let key = contract.drop_glue.get(&ty).expect("actual owner drop glue");
                    let declaration = contract.callable(key).expect("actual drop declaration");
                    assert_eq!(&declaration.key, key);
                    eprintln!("SLICE_SOURCE ownerDrop={term:?} checkedType={ty:?} canonicalGlue={declaration:?} leaseConflict={:?}", LoanAnalysis::check_aliasing(place, LoanKind::Mut, &table, &active));
                    proof.drops += 1;
                    if let MirCallableKind::LocalBody { function_id } = &declaration.kind {
                        queue.push_back(function_id.clone());
                    }
                }
                _ => {}
            }
            let origin = function.terminator_origin(term);
            if !places.is_empty() {
                for (boundary, set) in [
                    ("BLOCK_ENTRY", &live.entry_sets[block]),
                    ("BLOCK_EXIT", &live.exit_sets[block]),
                ] {
                    let members = set.iter().collect::<Vec<_>>();
                    assert!(
                        members.len() <= 128,
                        "SLICE_SOURCE HARNESS liveness-member cap128"
                    );
                    eprintln!("SLICE_SOURCE function={id:?} block={block} actualLivenessBoundary={boundary} members={members:?}; entry is not a pre-call snapshot");
                }
            }
            if !places.is_empty() {
                proof.snapshot(
                    &analysis,
                    &state,
                    &table,
                    &active,
                    &places,
                    block,
                    data.statements.len(),
                    "BEFORE_TERMINATOR_FROM_JOINED_ENTRY",
                    origin.source_span(),
                );
            }
            if let Some(bindings) = mutation_origins.get(&block) {
                analysis.apply_mutation_origins(&mut state, bindings);
            }
            analysis.apply_terminator(&mut state, term);
            let mut exact = entries[block].clone();
            active_analysis.apply_block(&mut exact, function, BasicBlockId(block));
            assert_eq!(state, results.exit_sets[block]);
            if !places.is_empty() {
                proof.snapshot(
                    &analysis,
                    &state,
                    &table,
                    &exact,
                    &places,
                    block,
                    data.statements.len(),
                    "AFTER_EXACT_BLOCK_FROM_JOINED_ENTRY",
                    origin.source_span(),
                );
            }
        }
    }
    proof.budget();
    proof
}

struct Control {
    source: String,
    entry: PathBuf,
    program: MirProgram,
    proof: Proof,
    outcome: Result<(), Diagnostics>,
}

fn control(case: &str, source: String, root_marker: &str, root_name: &str) -> Control {
    for flag in [
        "ROCK_INTEGRATION_BORROW_TRACE",
        "ROCK_OWNER_LOAN_TRACE",
        "ROCK_OWNER_RETURN_TRACE",
        "ROCK_FRESH_STDLIB_RETURN_TRACE",
    ] {
        assert!(
            std::env::var_os(flag).is_none(),
            "SLICE_SOURCE HARNESS unset observer flag {flag}"
        );
    }
    let artifact = agent_work_path("artifacts/stdlib58/contracts-1/stdlib.rkca");
    let bytes = std::fs::read(&artifact).unwrap();
    assert_eq!(bytes.len(), 927978);
    assert_eq!(integration_sha(&artifact), GUARDED_STDLIB_SHA);
    let receipt = agent_work_path("fixtures").join(format!(
        "rock_slice_source_{}_{case}.rk",
        std::process::id()
    ));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&receipt)
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let hash = integration_sha(&receipt);
    eprintln!("SLICE_SOURCE case={case} sourceBytes={} SHA256_BEFORE={hash} exactSource={source:?} stdlib927978SHA={GUARDED_STDLIB_SHA} compilerStdlibDependency=NONE caps=4functions/16origins/16loans/48paths execution=NONE; frontend/mono/agreement/budget failures=HARNESS, never Borrow RED", source.len());
    let (program, entry) = mutation_source_program(&format!("slice_source_{case}"), &source, false);
    let agreement = crate::mir::agreement::check_mir_runtime_agreement(&program);
    eprintln!("SLICE_SOURCE case={case} RuntimeAgreement={agreement:?}");
    assert!(
        agreement.is_clean()
            && agreement.object_contract_errors.is_empty()
            && agreement.erased_contract_errors.is_empty()
    );
    let (root, _) = binding(&program, &entry, &source, root_marker, root_name);
    let proof = inspect(&program, root);
    let outcome = BorrowChecker::run(&program);
    if let Err(diagnostics) = &outcome {
        assert!(
            diagnostics.0.len() <= 32,
            "SLICE_SOURCE HARNESS diagnostic cap32"
        );
        for diagnostic in &diagnostics.0 {
            eprintln!("SLICE_SOURCE case={case} actualDiagnostic={diagnostic:?}");
        }
    }
    for path in [&entry, &receipt] {
        assert_eq!(std::fs::read(path).unwrap(), source.as_bytes());
        assert_eq!(integration_sha(path), hash);
    }
    assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
    assert_eq!(integration_sha(&artifact), GUARDED_STDLIB_SHA);
    eprintln!("SLICE_SOURCE case={case} SHA256_AFTER={hash} source/artifactBytesUnchanged=true functions={} origins={} loans={} paths={} outcome={} execution=NONE ceiling=flat origins/header/roles NOT source proof",
        proof.functions.len(), proof.origins.len(), proof.loans.len(), proof.paths.len(), if outcome.is_ok() { "ACCEPTED" } else { "REJECTED" });
    Control {
        source,
        entry,
        program,
        proof,
        outcome,
    }
}

fn diagnostics(control: &Control) -> &Diagnostics {
    let diagnostics = control.outcome.as_ref().expect_err(
        "SLICE_SOURCE SECURITY RED: forbidden reference accepted; NEVER execute this source",
    );
    assert!(!diagnostics.0.is_empty());
    assert!(diagnostics.0.iter().all(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
        && matches!(&diagnostic.location, DiagnosticLocation::Source(span) if span.file_path == control.entry)
        && diagnostic.primary.is_some()), "SLICE_SOURCE HARNESS incidental/non-source Borrow diagnostic: {diagnostics:?}");
    diagnostics
}

fn exact_error(control: &Control, message: &str, span: &Span) {
    assert!(
        diagnostics(control)
            .0
            .iter()
            .any(|diagnostic| diagnostic.message == message
                && diagnostic
                    .primary
                    .as_ref()
                    .is_some_and(|label| &label.span == span)
                && diagnostic.location == DiagnosticLocation::Source(span.clone())),
        "SLICE_SOURCE SECURITY RED: missing exact {message:?} at actual span {span:?}"
    );
}

fn conflict(control: &Control, marker: &str) {
    let range = line_range(&control.source, marker);
    let conflicts = control
        .proof
        .conflicts
        .iter()
        .filter(|event| within(&event.span, &control.entry, range))
        .collect::<Vec<_>>();
    assert!(
        !conflicts.is_empty(),
        "SLICE_SOURCE SECURITY RED: no actual loan conflict at fixed mutation site {marker:?}"
    );
    for event in conflicts {
        assert!(control
            .proof
            .loans
            .contains(&(event.function.clone(), event.loan)));
        exact_error(control, "borrow conflict on this value", &event.span);
        assert!(
            diagnostics(control)
                .0
                .iter()
                .any(
                    |diagnostic| diagnostic.message == "borrow conflict on this value"
                        && diagnostic
                            .primary
                            .as_ref()
                            .is_some_and(|label| label.span == event.span)
                        && diagnostic
                            .secondary
                            .iter()
                            .any(|label| label.span == event.borrow_span)
                ),
            "actual conflicting LoanId must retain its source origin"
        );
    }
}

fn local_escape(control: &Control, marker: &str, name: &str) {
    let (id, local) = binding(
        &control.program,
        &control.entry,
        &control.source,
        marker,
        name,
    );
    let function = &control.program.functions[&id];
    assert!(
        control.proof.rows.iter().any(|row| row.function == id
            && row.place.local == Local(0)
            && row.origins.contains(&ReferenceOrigin::Local(local))),
        "SLICE_SOURCE SECURITY RED: actual local backing lost at ReturnPlace"
    );
    exact_error(
        control,
        LOCAL_ERROR,
        function.local_decls[local.0]
            .span
            .as_ref()
            .expect("real local backing span"),
    );
}

fn slot_cast(control: &Control) {
    let tc = &control.program.type_context;
    let view = TypeView::new(tc);
    assert!(control.proof.casts.iter().any(|(id, source, target)| {
        let Ty::Reference { inner: slot, .. } = view.ty(*source) else { return false; };
        let Ty::Reference { inner: slice, .. } = view.ty(*slot) else { return false; };
        let Ty::Slice(element) = view.ty(*slice) else { return false; };
        if view.ty(*element) != &Ty::I64 || view.ty(*target) != &Ty::Pointer(*element) { return false; }
        assert_ne!(slot, element, "slot referent and raw pointee are NOT equal");
        let header = control.program.backend_contract.callable(&control.program.backend_contract.function_bodies[id]).unwrap();
        assert_eq!(header.signature.params[0].semantic_ty, *source);
        assert_eq!(view.ty(header.signature.ret.semantic_ty), &Ty::Reference { mutable: false, inner: *element });
        assert!(control.proof.intrinsics.iter().any(|(body, intrinsic)| {
            if *intrinsic != MirIntrinsicId::ArrPtr { return false; }
            let checked = control.program.backend_contract.callable(&control.program.backend_contract.function_bodies[body]).unwrap();
            checked.signature == header.signature
        }), "SLICE_SOURCE HARNESS cast and checked extraction must have identical actual native headers; equality does NOT certify their bodies");
        true
    }), "SLICE_SOURCE HARNESS missing actual same-header metadata-address reinterpretation");
}

fn temporary_slot(control: &Control, marker: &str) {
    let range = line_range(&control.source, marker);
    let mut found = false;
    for row in &control.proof.rows {
        if row.phase != "AFTER_EXACT_BLOCK_FROM_JOINED_ENTRY"
            || !row
                .span
                .as_ref()
                .is_some_and(|span| within(span, &control.entry, range))
        {
            continue;
        }
        for origin in &row.origins {
            let ReferenceOrigin::Temporary(local) = origin else {
                continue;
            };
            let function = &control.program.functions[&row.function];
            let decl = &function.local_decls[local.0];
            if !matches!(control.program.type_context.type_for(decl.ty), Type::Reference { inner, .. } if matches!(*inner, Type::Slice(_)))
            {
                continue;
            }
            let leases = row
                .loans
                .iter()
                .filter(|loan| {
                    loan.active
                        && loan.indexed.local == *local
                        && loan.indexed.projection.is_empty()
                        && loan.aliases.iter().any(|place| place.local == *local)
                        && loan
                            .roles
                            .iter()
                            .any(|(role, owners)| *role == "storage" && !owners.is_empty())
                })
                .collect::<Vec<_>>();
            if leases.is_empty() {
                continue;
            }
            eprintln!("SLICE_SOURCE actualMetadataTemporary={local:?} function={:?} leases={leases:?} location=({},{})", row.function, row.block, row.index);
            exact_error(
                control,
                TEMP_ERROR,
                decl.span.as_ref().expect("real temporary slot origin"),
            );
            found = true;
        }
    }
    assert!(
        found,
        "SLICE_SOURCE SECURITY RED: actual Temporary metadata storage lease lost"
    );
}

#[test]
fn slice_return_source_authority_metadata_address_cast_rejects() {
    let source = format!("{VIEWS}{SLOT}\nmain = !->\n    values = [1, 2]\n    metadata: &[I64] = as_slice &values\n    checked = data_view &metadata, 0\n    result = slot_view &(as_slice &values), 0\n    *checked\n    *result\n");
    let control = control("slot_cast", source, "    values = [1, 2]", "values");
    slot_cast(&control);
    temporary_slot(&control, "    result = slot_view &(as_slice &values), 0");
}

#[test]
fn slice_return_source_authority_held_element_and_subslice_block_mutation() {
    let source = format!("{VIEWS}\nmain = !->\n    mut values: [I64; 2] = [1, 2]\n    metadata: &[I64] = as_slice &values\n    element = data_view &metadata, 0\n    subslice = slice_view &metadata\n    values = [3, 4]\n    *element\n    ~ArrayLen subslice\n");
    let control = control(
        "held",
        source,
        "    mut values: [I64; 2] = [1, 2]",
        "values",
    );
    conflict(&control, "    values = [3, 4]");
}

#[test]
fn slice_return_source_authority_dead_views_allow_mutation() {
    let source = format!("{VIEWS}\nmain = !->\n    mut values: [I64; 2] = [1, 2]\n    if true\n        metadata: &[I64] = as_slice &values\n        element = data_view &metadata, 0\n        subslice = slice_view &metadata\n        *element\n        ~ArrayLen subslice\n        ()\n    values = [3, 4]\n");
    let released = control(
        "dead",
        source,
        "    mut values: [I64; 2] = [1, 2]",
        "values",
    );
    for (marker, name) in [
        ("        metadata: &[I64] = as_slice &values", "metadata"),
        ("        element = data_view &metadata, 0", "element"),
        ("        subslice = slice_view &metadata", "subslice"),
    ] {
        let actual = binding(
            &released.program,
            &released.entry,
            &released.source,
            marker,
            name,
        );
        assert!(
            released.proof.released.contains(&actual),
            "SLICE_SOURCE HARNESS missing actual scoped StorageDead {actual:?}"
        );
    }
    let range = line_range(&released.source, "    values = [3, 4]");
    let writes = released
        .proof
        .writes
        .iter()
        .filter(|(_, span, _, _)| within(span, &released.entry, range))
        .collect::<Vec<_>>();
    assert!(!writes.is_empty());
    assert!(
        writes.iter().all(|(_, _, _, allowed)| *allowed),
        "SLICE_SOURCE CLASSIFICATION RED: dead views still retain a conflicting backing loan"
    );
    if released.outcome.is_err() {
        diagnostics(&released);
    }
    released.outcome.expect(
        "SLICE_SOURCE CLASSIFICATION RED: valid released array-backed views must pass Borrow",
    );
    // Deliberately positive: current flat summaries may report the known metadata
    // Temporary false positive. This is classification RED, not a security pass.
    let source = format!("{VIEWS}\nmain = !->\n    values = [1, 2]\n    result = data_view &(as_slice &values), 0\n    *result\n");
    let positive = control("checked_data", source, "    values = [1, 2]", "values");
    assert!(positive
        .proof
        .intrinsics
        .iter()
        .any(|(_, id)| *id == MirIntrinsicId::ArrPtr));
    assert!(positive
        .proof
        .intrinsics
        .iter()
        .any(|(_, id)| *id == MirIntrinsicId::PtrOffset));
    if positive.outcome.is_err() {
        diagnostics(&positive);
    }
    positive.outcome.expect("SLICE_SOURCE CLASSIFICATION RED: checked data source must retain backing provenance without retaining only supplying metadata storage");
}

#[test]
fn slice_return_source_authority_local_array_views_cannot_escape() {
    let source = format!("{VIEWS}\nescape: () -> (&I64, &[I64])\nescape = ->\n    values = [1, 2]\n    metadata: &[I64] = as_slice &values\n    ((data_view &metadata, 0), (slice_view &metadata))\nmain = !->\n    escape!\n");
    let control = control("escape", source, "    values = [1, 2]", "values");
    local_escape(&control, "    values = [1, 2]", "values");
}

#[test]
fn slice_return_source_authority_raw_owner_storage_and_drop_leases() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/mir/borrowck/borrowed_contents_tests.rs");
    let before = std::fs::read(&fixture).unwrap();
    let before_hash = integration_sha(&fixture);
    let storage = integration_literal(&fixture, "const STORAGE:");
    let source = format!("{storage}\nescape: () -> &[I64]\nescape = ->\n    pointer = (malloc 16) as *I64\n    mut owner = Storage\n        pointer: pointer\n    view = owner.slice!\n    owner.pointer = pointer\n    view\nmain = !->\n    escape!\n");
    let control = control("owner", source, "    mut owner = Storage", "owner");
    assert_eq!(std::fs::read(&fixture).unwrap(), before);
    assert_eq!(integration_sha(&fixture), before_hash);
    conflict(&control, "    owner.pointer = pointer");
    assert!(
        control.proof.drops > 0,
        "actual checked owner cleanup must remain present"
    );
    assert!(control
        .proof
        .intrinsics
        .iter()
        .any(|(_, id)| *id == MirIntrinsicId::BorrowSlice));
    assert!(control.proof.rows.iter().any(|row| row.place.projection.iter().any(|projection| matches!(projection, Projection::Field { .. }))
        && matches!(control.program.type_context.try_ty(control.program.functions[&row.function].local_decls[row.place.local.0].ty), Some(Ty::Reference { inner, .. })
            if matches!(control.program.type_context.try_ty(*inner), Some(Ty::Struct { .. })))), "actual owner field extraction is NOT primitive slice data extraction");
    let (id, owner) = binding(
        &control.program,
        &control.entry,
        &control.source,
        "    mut owner = Storage",
        "owner",
    );
    assert!(
        control.proof.rows.iter().any(|row| row.function == id
            && row.phase == "BEFORE_TERMINATOR_FROM_JOINED_ENTRY"
            && row.place.local == owner
            && row.loans.iter().any(|loan| loan.active
                && loan.indexed.local == owner
                && loan.roles.iter().any(|(_, carriers)| !carriers.is_empty()))),
        "SLICE_SOURCE SECURITY RED: owner cleanup lost its actual live lease"
    );
    assert!(
        diagnostics(&control)
            .0
            .iter()
            .any(|diagnostic| diagnostic.message == LOCAL_ERROR),
        "owner-backed local view escape remains forbidden"
    );
}

#[test]
fn slice_return_source_authority_join_reassignment_and_mixed_slot_branch() {
    let source = format!("{VIEWS}\nextern choice: () -> Bool\nmain = !->\n    mut first: [I64; 2] = [1, 2]\n    mut second: [I64; 2] = [3, 4]\n    mut metadata: &[I64] = as_slice &first\n    metadata = if choice!\n        as_slice &first\n    else\n        as_slice &second\n    result = data_view &metadata, 0\n    first = [5, 6]\n    second = [7, 8]\n    *result\n");
    let joined = control(
        "joined",
        source,
        "    mut first: [I64; 2] = [1, 2]",
        "first",
    );
    let (id, first) = binding(
        &joined.program,
        &joined.entry,
        &joined.source,
        "    mut first: [I64; 2] = [1, 2]",
        "first",
    );
    let (_, second) = binding(
        &joined.program,
        &joined.entry,
        &joined.source,
        "    mut second: [I64; 2] = [3, 4]",
        "second",
    );
    let (_, result) = binding(
        &joined.program,
        &joined.entry,
        &joined.source,
        "    result = data_view &metadata, 0",
        "result",
    );
    assert!(
        joined.proof.rows.iter().any(|row| row.function == id
            && row.place.local == result
            && row.place.projection == vec![Projection::Deref]
            && row.aliases.iter().any(|place| place.local == first)
            && row.aliases.iter().any(|place| place.local == second)),
        "SLICE_SOURCE SECURITY RED: joined backing alternatives lost"
    );
    conflict(&joined, "    first = [5, 6]");
    conflict(&joined, "    second = [7, 8]");
    let source = format!("{VIEWS}\nextern choice: () -> Bool\nmixed: & &[I64] -> Bool -> &I64\nmixed = metadata, flag ->\n    if flag\n        unsafe &*(~ArrPtr *metadata)\n    else\n        unsafe &*(metadata as *I64)\nmain = !->\n    values = [1, 2]\n    result = mixed &(as_slice &values), choice!\n    *result\n");
    let mixed = control("mixed", source, "    values = [1, 2]", "values");
    slot_cast(&mixed);
    assert!(mixed.proof.casts.iter().any(|(id, _, _)| mixed.proof.intrinsics.iter().any(|(body, intrinsic)| body == id && *intrinsic == MirIntrinsicId::ArrPtr)),
        "both actual data extraction and metadata-address branches must be present in the same selected body");
    temporary_slot(&mixed, "    result = mixed &(as_slice &values), choice!");
}

#[test]
fn slice_return_source_authority_exact_loan_id_independent_metadata_alias() {
    let source = format!("{VIEWS}\nmain = !->\n    values = [1, 2]\n    mut metadata: &[I64] = as_slice &values\n    alias = &metadata\n    holder = (alias, 0)\n    result = data_view alias, 0\n    metadata = *alias\n    *result\n    ~ArrayLen *alias\n    ~ArrayLen *(holder.0)\n");
    let control = control(
        "independent",
        source,
        "    mut metadata: &[I64] = as_slice &values",
        "metadata",
    );
    let (id, metadata) = binding(
        &control.program,
        &control.entry,
        &control.source,
        "    mut metadata: &[I64] = as_slice &values",
        "metadata",
    );
    let (_, alias) = binding(
        &control.program,
        &control.entry,
        &control.source,
        "    alias = &metadata",
        "alias",
    );
    let (_, holder) = binding(
        &control.program,
        &control.entry,
        &control.source,
        "    holder = (alias, 0)",
        "holder",
    );
    let range = line_range(&control.source, "    result = data_view alias, 0");
    let mut ids = HashSet::new();
    for row in &control.proof.rows {
        if row.function != id
            || row.phase != "BEFORE_TERMINATOR_FROM_JOINED_ENTRY"
            || !row
                .span
                .as_ref()
                .is_some_and(|span| within(span, &control.entry, range))
        {
            continue;
        }
        for loan in &row.loans {
            if loan.active
                && loan.indexed
                    == (Place {
                        local: metadata,
                        projection: Vec::new(),
                    })
                && loan
                    .roles
                    .iter()
                    .any(|(role, owners)| *role == "storage" && owners.contains(&alias))
                && loan
                    .roles
                    .iter()
                    .any(|(role, owners)| *role == "contents" && owners.contains(&holder))
            {
                ids.insert(loan.id);
            }
        }
    }
    assert!(
        !ids.is_empty(),
        "SLICE_SOURCE HARNESS missing actual metadata loan with independent storage+contents roles"
    );
    for loan_id in ids {
        assert!(control.proof.rows.iter().any(|row| row.function == id && row.phase == "AFTER_EXACT_BLOCK_FROM_JOINED_ENTRY"
            && row.span.as_ref().is_some_and(|span| within(span, &control.entry, range))
            && row.loans.iter().any(|loan| loan.id == loan_id && loan.active
                && loan.roles.iter().any(|(role, owners)| *role == "storage" && owners.contains(&alias))
                && loan.roles.iter().any(|(role, owners)| *role == "contents" && owners.contains(&holder)))),
            "SLICE_SOURCE SECURITY RED: exact receiver-slot LoanId globally deactivated or independent roles dropped");
    }
    conflict(&control, "    metadata = *alias");
    let write = line_range(&control.source, "    metadata = *alias");
    assert!(
        control
            .proof
            .writes
            .iter()
            .any(|(owner, span, destination, permitted)| owner == &id
                && within(span, &control.entry, write)
                && *destination
                    == (Place {
                        local: metadata,
                        projection: Vec::new()
                    })
                && !permitted),
        "SLICE_SOURCE SECURITY RED: actual metadata write must conflict, not its shared RHS read"
    );
}

#[test]
fn slice_return_source_authority_bodyless_reference_result_stays_unknown() {
    let source = format!("{VIEWS}\nextern opaque: & &[I64] -> I64 -> &I64\nmain = !->\n    values = [1, 2]\n    result = opaque &(as_slice &values), 0\n    *result\n");
    let control = control("bodyless", source, "    values = [1, 2]", "values");
    assert!(
        !control.proof.absent.is_empty(),
        "actual declared bodyless reference-producing callable required"
    );
    temporary_slot(&control, "    result = opaque &(as_slice &values), 0");
}

#[test]
fn slice_return_source_authority_by_value_array_and_pointer_casts_cannot_escape() {
    // Direct ArrPtr(array-value) is frontend-forbidden today. This valid source
    // borrows a locally materialized by-value parameter; it is not external backing.
    let source = format!("{VIEWS}\nmaterialize: [I64; 2] -> &[I64]\nmaterialize = values ->\n    owned: [I64; 2] = values\n    unsafe\n        pointer: *I64 = ~ArrPtr (as_slice &owned)\n        bytes: *U8 = pointer as *U8\n        recast: *I64 = bytes as *I64\n        ~BorrowSlice recast, 2\nmain = !->\n    materialize [1, 2]\n");
    let control = control("by_value", source, "    owned: [I64; 2] = values", "owned");
    let (id, owned) = binding(
        &control.program,
        &control.entry,
        &control.source,
        "    owned: [I64; 2] = values",
        "owned",
    );
    let function = &control.program.functions[&id];
    let header = control
        .program
        .backend_contract
        .callable(&control.program.backend_contract.function_bodies[&id])
        .unwrap();
    assert_eq!(header.signature.params.len(), 1);
    assert_eq!(
        header.signature.params[0].semantic_ty,
        function.local_decls[owned.0].ty
    );
    assert!(matches!(
        control
            .program
            .type_context
            .try_ty(header.signature.params[0].semantic_ty),
        Some(Ty::Array { .. })
    ));
    assert!(function.basic_blocks.iter().flat_map(|block| &block.statements).any(|stmt|
        matches!(&stmt.kind, StatementKind::Assign(dest, Rvalue::Use(Operand::Copy(source) | Operand::Move(source)))
            if dest.local == owned && function.local_decls[source.local.0].source == LocalSource::Argument)), "actual local array materialization must come from by-value argument");
    assert!(
        control
            .proof
            .casts
            .iter()
            .filter(|(body, source, target)| body == &id
                && matches!(
                    control.program.type_context.try_ty(*source),
                    Some(Ty::Pointer(_))
                )
                && matches!(
                    control.program.type_context.try_ty(*target),
                    Some(Ty::Pointer(_))
                ))
            .count()
            >= 2
    );
    assert!(control
        .proof
        .intrinsics
        .iter()
        .any(|(body, intrinsic)| body == &id && *intrinsic == MirIntrinsicId::ArrPtr));
    assert!(control
        .proof
        .intrinsics
        .iter()
        .any(|(body, intrinsic)| body == &id && *intrinsic == MirIntrinsicId::BorrowSlice));
    local_escape(&control, "    owned: [I64; 2] = values", "owned");
}

// The real source declaration produces Extern authority, not an invented
// ObjectProvided header. Original artifact-backed slice controls cover that kind.
// Neither this positive nor the paired negatives ever link or execute opaque code.
fn opaque_scalar_call(control: &Control, marker: &str, scalar_only: bool) {
    let program = &control.program;
    let tc = &program.type_context;
    let contract = &program.backend_contract;
    let range = line_range(&control.source, marker);
    let selected = program
        .functions()
        .flat_map(|(_, function)| {
            function
                .basic_blocks
                .iter()
                .enumerate()
                .filter_map(move |(block, data)| {
                    let term @ Terminator::Call { .. } = data.terminator.as_ref()? else {
                        return None;
                    };
                    function
                        .terminator_origin(term)
                        .source_span()
                        .is_some_and(|span| within(span, &control.entry, range))
                        .then_some((function, block, term))
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        selected.len(),
        1,
        "SLICE_SOURCE HARNESS exact source-selected opaque call required"
    );
    let (producer, block, term) = selected[0];
    let Terminator::Call {
        func,
        args,
        destination,
        ..
    } = term
    else {
        unreachable!();
    };
    let Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) = func else {
        panic!("actual opaque key required");
    };
    let header = contract.callable(key).expect("actual opaque declaration");
    assert_eq!(&header.key, key);
    assert!(
        header.source_def_id.is_some(),
        "actual source declaration identity required"
    );
    assert!(matches!(
        &header.kind,
        MirCallableKind::Extern {
            variadic: false,
            ..
        }
    ));
    assert!(args.len() <= 8);
    let params = args
        .iter()
        .map(|arg| operand_type(program, producer, arg))
        .collect::<Vec<_>>();
    assert_eq!(params.len(), header.signature.params.len());
    for (actual, parameter) in params.iter().zip(&header.signature.params) {
        assert_eq!(*actual, parameter.semantic_ty);
    }
    let ret = contract
        .place_type_id(tc, producer, destination)
        .expect("actual opaque result type");
    assert!(tc.type_id_tree_is_valid(ret));
    assert_eq!(ret, header.signature.ret.semantic_ty);
    assert_eq!(tc.try_ty(ret), Some(&Ty::I64));
    let tracks = params
        .iter()
        .map(|ty| type_id_tracks_origin(TypeView::new(tc), contract, *ty))
        .collect::<Vec<_>>();
    assert_eq!(tracks.iter().all(|tracks| !tracks), scalar_only);
    assert!(!type_id_tracks_origin(TypeView::new(tc), contract, ret));
    let opaque_id = MirCallable::Resolved(key.clone())
        .function_id()
        .expect("actual extern identity");
    assert!(!program.functions.contains_key(&opaque_id));
    assert!(contract.function_bodies.get(&opaque_id).is_none());
    let lifetime = BorrowChecker::reference_return_summaries(program);
    let values = BorrowChecker::native_return_loan_proof(program);
    let mutations = NativeMutationTable::for_program(program);
    assert!(lifetime.get(&opaque_id).is_none() && values.summaries.get(&opaque_id).is_none());
    assert_eq!(mutations.knowledge(key), NativeMutationKnowledge::Unknown);
    assert!(
        !values.sources.has_body_evidence(key),
        "SLICE_SOURCE SECURITY RED: opaque header acquired body/source/scalar-return evidence"
    );
    assert!(values
        .sources
        .borrowed_view_call(producer, tc, contract, func, args, destination)
        .is_none());
    eprintln!("SLICE_SOURCE opaqueCall producer={:?} block={block} actualTerm={term:?} actualHeader={header:?} params={params:?} paramTracksOrigin={tracks:?} ret={ret:?} retTracksOrigin=false body=ABSENT Lifetime=None ValueLoans=None mutationKnowledge={:?} bodyEvidence=false scalarOnly={scalar_only}; value-flow independence is NOT no-effects knowledge execution=NONE",
        producer.id, mutations.knowledge(key));

    let caller_range = line_range(
        &control.source,
        "    result = opaque_data_view &(as_slice &values), 0",
    );
    let callers = program
        .functions()
        .flat_map(|(_, function)| {
            function.basic_blocks.iter().filter_map(move |data| {
                let term @ Terminator::Call {
                    func,
                    args,
                    destination,
                    ..
                } = data.terminator.as_ref()?
                else {
                    return None;
                };
                let Operand::Constant(Constant::Callable(callable)) = func else {
                    return None;
                };
                (callable.function_id().as_ref() == Some(&producer.id)
                    && function
                        .terminator_origin(term)
                        .source_span()
                        .is_some_and(|span| within(span, &control.entry, caller_range)))
                .then_some((function, func, args, destination))
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        callers.len(),
        1,
        "SLICE_SOURCE HARNESS actual native data-view caller required"
    );
    let (caller, func, args, destination) = callers[0];
    let sources = values
        .sources
        .borrowed_view_call(caller, tc, contract, func, args, destination);
    assert_eq!(sources.is_some(), scalar_only,
        "SLICE_SOURCE SECURITY/CLASSIFICATION RED: opaque reference/raw inputs must block certification; scalar-only calls must preserve actual local-body data proof");
    if let Some(sources) = sources {
        assert_eq!(sources.len(), 1);
        for source in sources {
            let super::return_sources::Source::Slice {
                parameter,
                load,
                reference,
            } = source
            else {
                panic!("SLICE_SOURCE SECURITY RED: conditional pointer input admitted as borrowed source");
            };
            let declaration = &producer.local_decls[parameter.0];
            assert_eq!(declaration.source, LocalSource::Argument);
            assert_eq!(load, &[Projection::Deref]);
            assert!(
                matches!(tc.try_ty(declaration.ty), Some(Ty::Reference { mutable: false, inner }) if inner == reference)
            );
            assert!(
                matches!(tc.try_ty(*reference), Some(Ty::Reference { mutable: false, inner })
                if matches!(tc.try_ty(*inner), Some(Ty::Slice(element)) if tc.try_ty(*element) == Some(&Ty::I64)))
            );
        }
    }
}

#[test]
fn slice_return_source_authority_opaque_scalar_call_preserves_checked_data_only() {
    let source = format!("{VIEWS}\nextern opaque_scalar: I64 -> I64\nopaque_data_view: & &[I64] -> I64 -> &I64\nopaque_data_view = metadata, index ->\n    data: &[I64] = *metadata\n    offset = opaque_scalar index\n    unsafe &*(~PtrOffset (~ArrPtr data), offset)\nmain = !->\n    values = [1, 2]\n    result = opaque_data_view &(as_slice &values), 0\n    *result\n");
    let control = control("opaque_scalar", source, "    values = [1, 2]", "values");
    opaque_scalar_call(&control, "    offset = opaque_scalar index", true);
    for intrinsic in [MirIntrinsicId::ArrPtr, MirIntrinsicId::PtrOffset] {
        assert!(control
            .proof
            .intrinsics
            .iter()
            .any(|(_, actual)| *actual == intrinsic));
    }
    if control.outcome.is_err() {
        diagnostics(&control);
    }
    control.outcome.expect("SLICE_SOURCE CLASSIFICATION RED: witnessed primitive backing with opaque scalar-only call must pass Borrow; NOT an effect summary or runtime claim");
}

#[test]
fn slice_return_source_authority_opaque_reference_input_remains_conservative() {
    let source = format!("{VIEWS}\nextern opaque_reference: & &[I64] -> I64\nopaque_data_view: & &[I64] -> I64 -> &I64\nopaque_data_view = metadata, index ->\n    data: &[I64] = *metadata\n    offset = opaque_reference metadata\n    unsafe &*(~PtrOffset (~ArrPtr data), offset)\nmain = !->\n    values = [1, 2]\n    result = opaque_data_view &(as_slice &values), 0\n    *result\n");
    let control = control(
        "opaque_reference_input",
        source,
        "    values = [1, 2]",
        "values",
    );
    opaque_scalar_call(&control, "    offset = opaque_reference metadata", false);
    temporary_slot(
        &control,
        "    result = opaque_data_view &(as_slice &values), 0",
    );
}

#[test]
fn slice_return_source_authority_opaque_raw_input_remains_conservative() {
    let source = format!("{VIEWS}\nextern opaque_raw: *I64 -> I64\nopaque_data_view: & &[I64] -> I64 -> &I64\nopaque_data_view = metadata, index ->\n    data: &[I64] = *metadata\n    unsafe\n        pointer: *I64 = ~ArrPtr data\n        offset = opaque_raw pointer\n        &*(~PtrOffset pointer, offset)\nmain = !->\n    values = [1, 2]\n    result = opaque_data_view &(as_slice &values), 0\n    *result\n");
    let control = control("opaque_raw_input", source, "    values = [1, 2]", "values");
    opaque_scalar_call(&control, "        offset = opaque_raw pointer", false);
    temporary_slot(
        &control,
        "    result = opaque_data_view &(as_slice &values), 0",
    );
}

const OPAQUE_SHARED: &str = "\nextern opaque_shared: &I64 -> I64\n";
const SHARED_MAIN: &str = "\nmain = !->\n    values = [1, 2]\n    result = shared_data_view &(as_slice &values)\n    *result\n";
const SHARED_RESULT: &str = "    result = shared_data_view &(as_slice &values)";

// New guard-local observation only; existing observers and source analysis stay
// unchanged. NONE is absence, never fresh-storage or no-effects evidence.
fn opaque_shared_receipt<'a>(control: &'a Control, marker: &str, negative: bool) -> &'a Row {
    let program = &control.program;
    let tc = &program.type_context;
    let contract = &program.backend_contract;
    let range = line_range(&control.source, marker);
    let calls = program
        .functions()
        .flat_map(|(_, body)| {
            body.basic_blocks
                .iter()
                .enumerate()
                .filter_map(move |(block, data)| {
                    let term @ Terminator::Call { .. } = data.terminator.as_ref()? else {
                        return None;
                    };
                    body.terminator_origin(term)
                        .source_span()
                        .is_some_and(|span| within(span, &control.entry, range))
                        .then_some((body, block, term))
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        calls.len(),
        1,
        "SLICE_SOURCE HARNESS exact shared opaque call required"
    );
    let (body, block, term) = calls[0];
    let Terminator::Call {
        func,
        args,
        destination,
        ..
    } = term
    else {
        unreachable!();
    };
    let Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) = func else {
        panic!("actual shared opaque key required");
    };
    let header = contract
        .callable(key)
        .expect("actual shared opaque declaration");
    assert_eq!(&header.key, key);
    assert!(header.source_def_id.is_some());
    assert!(matches!(
        &header.kind,
        MirCallableKind::Extern {
            variadic: false,
            ..
        }
    ));
    assert_eq!(args.len(), 1);
    assert_eq!(header.signature.params.len(), 1);
    let input = operand_type(program, body, &args[0]);
    assert_eq!(input, header.signature.params[0].semantic_ty);
    assert!(
        matches!(tc.try_ty(input), Some(Ty::Reference { mutable: false, inner }) if tc.try_ty(*inner) == Some(&Ty::I64))
    );
    let ret = contract
        .place_type_id(tc, body, destination)
        .expect("actual opaque scalar result");
    assert!(tc.type_id_tree_is_valid(ret));
    assert_eq!(ret, header.signature.ret.semantic_ty);
    assert_eq!(tc.try_ty(ret), Some(&Ty::I64));
    assert!(type_id_tracks_origin(TypeView::new(tc), contract, input));
    assert!(!type_id_tracks_origin(TypeView::new(tc), contract, ret));
    let opaque_id = MirCallable::Resolved(key.clone())
        .function_id()
        .expect("actual extern identity");
    let lifetime = BorrowChecker::reference_return_summaries(program);
    let values = BorrowChecker::native_return_loan_proof(program);
    let mutations = NativeMutationTable::for_program(program);
    assert!(
        !program.functions.contains_key(&opaque_id)
            && contract.function_bodies.get(&opaque_id).is_none()
    );
    assert!(lifetime.get(&opaque_id).is_none() && values.summaries.get(&opaque_id).is_none());
    assert_eq!(mutations.knowledge(key), NativeMutationKnowledge::Unknown);
    assert!(!values.sources.has_body_evidence(key));
    assert!(values
        .sources
        .borrowed_view_call(body, tc, contract, func, args, destination)
        .is_none());
    let body_key = &contract.function_bodies[&body.id];
    if negative {
        diagnostics(control); // Accepted unsafe source is SECURITY RED, never executed.
        assert!(!values.sources.has_body_evidence(body_key), "SLICE_SOURCE SECURITY RED: forbidden shared input acquired native source certification");
    }
    let caller_range = line_range(&control.source, SHARED_RESULT);
    let callers = program
        .functions()
        .flat_map(|(_, caller)| {
            caller.basic_blocks.iter().filter_map(move |data| {
                let term @ Terminator::Call {
                    func,
                    args,
                    destination,
                    ..
                } = data.terminator.as_ref()?
                else {
                    return None;
                };
                let Operand::Constant(Constant::Callable(callable)) = func else {
                    return None;
                };
                (callable.function_id().as_ref() == Some(&body.id)
                    && caller
                        .terminator_origin(term)
                        .source_span()
                        .is_some_and(|span| within(span, &control.entry, caller_range)))
                .then_some((caller, func, args, destination))
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(callers.len(), 1);
    let (caller, func, actual_args, result) = callers[0];
    let source = values
        .sources
        .borrowed_view_call(caller, tc, contract, func, actual_args, result);
    if negative {
        assert!(
            source.is_none(),
            "SLICE_SOURCE SECURITY RED: forbidden shared input granted caller precision"
        );
    }
    let (Operand::Copy(argument) | Operand::Move(argument)) = &args[0] else {
        panic!("actual shared reference carrier required");
    };
    assert!(argument.projection.is_empty());
    let mut deref = argument.clone();
    deref.projection.push(Projection::Deref);
    let rows = control
        .proof
        .rows
        .iter()
        .filter(|row| {
            row.function == body.id
                && row.block == block
                && row.phase == "BEFORE_TERMINATOR_FROM_JOINED_ENTRY"
                && row.place == deref
                && row
                    .span
                    .as_ref()
                    .is_some_and(|span| within(span, &control.entry, range))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rows.len(),
        1,
        "SLICE_SOURCE HARNESS complete shared-call alias receipt required"
    );
    let row = rows[0];
    assert!(!row.aliases.is_empty());
    assert!(
        row.loans.iter().any(|loan| loan.active
            && loan
                .roles
                .iter()
                .any(|(role, owners)| *role == "storage" && owners.contains(&argument.local))),
        "SLICE_SOURCE HARNESS copied shared argument must retain an actual live storage loan"
    );
    eprintln!("SLICE_SOURCE sharedOpaque body={:?} block={block} actualCall={term:?} header={header:?} actualInput={input:?} ret={ret:?} body=ABSENT Lifetime=None ValueLoans=None mutationKnowledge=UNKNOWN opaqueSourceAndScalarEvidence=ABSENT nativeEvidence={} callerSources={source:?} actualCarrier={argument:?} ALL_aliases={:?} ALL_observedLoans={:?}; absence is NOT independence/no-effects evidence execution=NONE",
        body.id, values.sources.has_body_evidence(body_key), row.aliases, row.loans);
    row
}

fn shared_ref_lease(control: &Control, row: &Row, initial: Local, target: &Place) -> Vec<Place> {
    let program = &control.program;
    let body = &program.functions[&row.function];
    let definitions = body
        .basic_blocks
        .iter()
        .enumerate()
        .flat_map(|(block, data)| {
            data.statements
                .iter()
                .enumerate()
                .filter_map(move |(index, statement)| {
                    let StatementKind::Assign(
                        dest,
                        Rvalue::Ref(crate::mir::Mutability::Not, place),
                    ) = &statement.kind
                    else {
                        return None;
                    };
                    (dest.projection.is_empty() && place == target)
                        .then_some((block, index, dest.local))
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(definitions.len(), 1, "SLICE_SOURCE HARNESS actual shared Ref definition required, not a header/type-based address guess");
    let (block, index, owner) = definitions[0];
    let values = BorrowChecker::native_return_loan_proof(program);
    let borrows = borrows::collect_function_borrows_with_return_sources(
        body,
        &program.type_context,
        &program.backend_contract,
        &values.summaries,
        Some(&values.sources),
    );
    let table =
        LoanTable::from_borrows_with_paths(&borrows, paths::PlacePathTable::for_function(body));
    let loans = table
        .iter()
        .filter(|loan| {
            loan.initial_owner == owner
                && loan.kind == LoanKind::Shared
                && loan.created_at
                    == (Location {
                        block: BasicBlockId(block),
                        statement: StatementIndex(index),
                    })
                && table.place_paths().place(loan.place_path) == Some(target)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        loans.len(),
        1,
        "SLICE_SOURCE HARNESS exact Ref-created indexed shared loan required"
    );
    let loan = loans[0];
    assert!(control.proof.loans.contains(&(body.id.clone(), loan.id)));
    assert!(control.proof.rows.iter().any(|observed| observed.function == body.id
        && observed.phase == "AFTER_EXACT_STATEMENT" && observed.place.local == initial
        && observed.loans.iter().any(|receipt| receipt.id == loan.id && receipt.active
            && receipt.roles.iter().any(|(role, owners)| *role == "storage" && owners.contains(&initial)))),
        "SLICE_SOURCE HARNESS named reference must have carried the same Ref-created loan, including compiler copy temporaries");
    let receipt = row
        .loans
        .iter()
        .find(|receipt| receipt.id == loan.id)
        .expect("same actual LoanId at opaque call");
    assert_eq!(&receipt.indexed, target);
    assert!(
        receipt.active
            && receipt
                .roles
                .iter()
                .any(|(role, owners)| *role == "storage" && owners.contains(&row.place.local)),
        "SLICE_SOURCE HARNESS supplying shared lease missing from copied live carrier"
    );
    assert!(!receipt.aliases.is_empty());
    eprintln!("SLICE_SOURCE sharedInputRef actualLoan={loan:?} indexedTarget={target:?} ALL_roles={:?} ALL_loanAliases={:?}; IDs are function-scoped, no fabricated path or permission", receipt.roles, receipt.aliases);
    receipt.aliases.clone()
}

fn shared_data_pointer(
    control: &Control,
    row: &Row,
    pointer_marker: &str,
    reference_marker: &str,
    reference_name: &str,
) -> Vec<Place> {
    let (id, pointer) = binding(
        &control.program,
        &control.entry,
        &control.source,
        pointer_marker,
        "pointer",
    );
    let (reference_id, reference) = binding(
        &control.program,
        &control.entry,
        &control.source,
        reference_marker,
        reference_name,
    );
    assert_eq!(id, row.function);
    assert_eq!(reference_id, id);
    let body = &control.program.functions[&id];
    let contract = &control.program.backend_contract;
    let tc = &control.program.type_context;
    let definitions = body
        .basic_blocks
        .iter()
        .filter_map(|block| {
            let Terminator::Call {
                func,
                args,
                destination,
                ..
            } = block.terminator.as_ref()?
            else {
                return None;
            };
            (destination.local == pointer && destination.projection.is_empty())
                .then_some((func, args))
        })
        .collect::<Vec<_>>();
    assert_eq!(definitions.len(), 1);
    let (func, args) = definitions[0];
    assert!(matches!(
        func,
        Operand::Constant(Constant::Callable(MirCallable::Resolved(
            MirCallableKey::Intrinsic(MirIntrinsicId::ArrPtr)
        )))
    ));
    assert_eq!(args.len(), 1);
    let Operand::Copy(data) = &args[0] else {
        panic!("actual copied inner slice required");
    };
    assert!(data.projection.is_empty());
    // Observe compiler argument-copy temporaries, not just the named binding.
    // These fixtures deliberately have a single load/copy chain, not a second
    // source-analysis algorithm or permission for unknown reaching definitions.
    let mut current = data.local;
    let mut visited = HashSet::new();
    loop {
        assert!(
            visited.insert(current),
            "SLICE_SOURCE HARNESS cyclic slice argument copies"
        );
        assert!(
            visited.len() <= 8,
            "SLICE_SOURCE HARNESS OVERFLOW slice copy chain8; no truncation"
        );
        let definitions = body
            .basic_blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| {
                let StatementKind::Assign(dest, value) = &statement.kind else {
                    return None;
                };
                (dest.local == current && dest.projection.is_empty()).then_some((statement, value))
            })
            .collect::<Vec<_>>();
        assert_eq!(definitions.len(), 1, "SLICE_SOURCE HARNESS all slice-copy definitions must be observed, not an arbitrary first source");
        let (statement, Rvalue::Use(Operand::Copy(source))) = definitions[0] else {
            panic!("actual copied slice load/forward required");
        };
        let loaded = contract
            .place_type_id(tc, body, source)
            .expect("actual copied slice type");
        assert_eq!(loaded, body.local_decls[current.0].ty);
        assert!(
            matches!(tc.try_ty(loaded), Some(Ty::Reference { mutable: false, inner })
            if matches!(tc.try_ty(*inner), Some(Ty::Slice(element)) if tc.try_ty(*element) == Some(&Ty::I64)))
        );
        eprintln!("SLICE_SOURCE sharedDataInput actualCopy={statement:?} checkedType={loaded:?}; actual load/copy chain, not source-state NONE authority");
        if source.projection == [Projection::Deref] {
            assert_eq!(
                body.local_decls[source.local.0].source,
                LocalSource::Argument
            );
            break;
        }
        assert!(
            source.projection.is_empty(),
            "SLICE_SOURCE HARNESS unexpected slice extraction path"
        );
        current = source.local;
    }
    let target = Place {
        local: pointer,
        projection: vec![Projection::Deref],
    };
    let ty = contract
        .place_type_id(tc, body, &target)
        .expect("actual data pointee type");
    assert_eq!(tc.try_ty(ty), Some(&Ty::I64));
    shared_ref_lease(control, row, reference, &target)
}

#[test]
fn slice_return_source_authority_opaque_shared_fresh_scalar_and_tuple_storage() {
    let scalar_source = format!("{VIEWS}{OPAQUE_SHARED}\nshared_data_view: & &[I64] -> &I64\nshared_data_view = metadata ->\n    data: &[I64] = *metadata\n    fresh: I64 = 0\n    first: &I64 = &fresh\n    alias: &I64 = first\n    offset = opaque_shared alias\n    unsafe &*(~PtrOffset (~ArrPtr data), offset)\n{SHARED_MAIN}");
    let tuple_source = format!("{VIEWS}{OPAQUE_SHARED}\nextern opaque_bounds: I64 -> (I64, I64)\nshared_data_view: & &[I64] -> &I64\nshared_data_view = metadata ->\n    data: &[I64] = *metadata\n    bounds: (I64, I64) = opaque_bounds 2\n    alias: &I64 = &bounds.1\n    offset = opaque_shared alias\n    unsafe &*(~PtrOffset (~ArrPtr data), offset)\n{SHARED_MAIN}");
    // Both complete source/proof/hash receipts precede any acceptance assertion.
    let scalar = control(
        "shared_fresh_scalar",
        scalar_source,
        "    values = [1, 2]",
        "values",
    );
    let tuple = control(
        "shared_tuple_field",
        tuple_source,
        "    values = [1, 2]",
        "values",
    );
    let scalar_row = opaque_shared_receipt(&scalar, "    offset = opaque_shared alias", false);
    let (id, fresh) = binding(
        &scalar.program,
        &scalar.entry,
        &scalar.source,
        "    fresh: I64 = 0",
        "fresh",
    );
    let (_, first) = binding(
        &scalar.program,
        &scalar.entry,
        &scalar.source,
        "    first: &I64 = &fresh",
        "first",
    );
    let (_, alias) = binding(
        &scalar.program,
        &scalar.entry,
        &scalar.source,
        "    alias: &I64 = first",
        "alias",
    );
    assert_eq!(id, scalar_row.function);
    let root = Place {
        local: fresh,
        projection: Vec::new(),
    };
    let body = &scalar.program.functions[&id];
    let initialized = body.basic_blocks.iter().flat_map(|block| &block.statements).filter(|statement|
        matches!(&statement.kind, StatementKind::Assign(dest, Rvalue::Use(Operand::Constant(Constant::Int(0)))) if dest == &root)).collect::<Vec<_>>();
    assert_eq!(
        initialized.len(),
        1,
        "SLICE_SOURCE HARNESS direct initialized scalar storage required"
    );
    assert_eq!(
        scalar
            .program
            .type_context
            .try_ty(body.local_decls[fresh.0].ty),
        Some(&Ty::I64)
    );
    assert!(!type_id_tracks_origin(
        TypeView::new(&scalar.program.type_context),
        &scalar.program.backend_contract,
        body.local_decls[fresh.0].ty
    ));
    eprintln!("SLICE_SOURCE sharedScalarRoot actualDefinition={:?} actualRoot={root:?} declaration={:?}; storage initialization witnessed, not inferred from source-state NONE", initialized[0], body.local_decls[fresh.0]);
    assert!(body.basic_blocks.iter().flat_map(|block| &block.statements).any(|statement|
        matches!(&statement.kind, StatementKind::Assign(dest, Rvalue::Use(Operand::Copy(source)))
            if dest.local == alias && dest.projection.is_empty() && source.local == first && source.projection.is_empty())));
    assert_eq!(
        shared_ref_lease(&scalar, scalar_row, first, &root),
        vec![root.clone()]
    );
    assert_eq!(scalar_row.aliases, vec![root]);

    let tuple_row = opaque_shared_receipt(&tuple, "    offset = opaque_shared alias", false);
    let (id, bounds) = binding(
        &tuple.program,
        &tuple.entry,
        &tuple.source,
        "    bounds: (I64, I64) = opaque_bounds 2",
        "bounds",
    );
    let (_, alias) = binding(
        &tuple.program,
        &tuple.entry,
        &tuple.source,
        "    alias: &I64 = &bounds.1",
        "alias",
    );
    assert_eq!(id, tuple_row.function);
    let body = &tuple.program.functions[&id];
    let tc = &tuple.program.type_context;
    let contract = &tuple.program.backend_contract;
    let root_ty = body.local_decls[bounds.0].ty;
    assert!(!type_id_tracks_origin(TypeView::new(tc), contract, root_ty));
    assert!(
        matches!(tc.try_ty(root_ty), Some(Ty::Tuple(fields)) if fields.len() == 2 && fields.iter().all(|ty| tc.try_ty(*ty) == Some(&Ty::I64)))
    );
    let definitions = body
        .basic_blocks
        .iter()
        .filter_map(|block| {
            let term @ Terminator::Call { destination, .. } = block.terminator.as_ref()? else {
                return None;
            };
            (destination.local == bounds && destination.projection.is_empty()).then_some(term)
        })
        .collect::<Vec<_>>();
    assert_eq!(definitions.len(), 1);
    let Terminator::Call { func, args, .. } = definitions[0] else {
        unreachable!();
    };
    let Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) = func else {
        panic!("actual opaque tuple producer required");
    };
    let header = contract.callable(key).unwrap();
    assert_eq!(&header.key, key);
    assert!(matches!(
        &header.kind,
        MirCallableKind::Extern {
            variadic: false,
            ..
        }
    ));
    assert_eq!(header.signature.ret.semantic_ty, root_ty);
    assert_eq!(args.len(), header.signature.params.len());
    for (arg, parameter) in args.iter().zip(&header.signature.params) {
        let actual = operand_type(&tuple.program, body, arg);
        assert_eq!(actual, parameter.semantic_ty);
        assert!(!type_id_tracks_origin(TypeView::new(tc), contract, actual));
    }
    let values = BorrowChecker::native_return_loan_proof(&tuple.program);
    let lifetime = BorrowChecker::reference_return_summaries(&tuple.program);
    let opaque_id = MirCallable::Resolved(key.clone()).function_id().unwrap();
    assert!(
        !tuple.program.functions.contains_key(&opaque_id)
            && contract.function_bodies.get(&opaque_id).is_none()
    );
    assert!(lifetime.get(&opaque_id).is_none() && values.summaries.get(&opaque_id).is_none());
    assert!(!values.sources.has_body_evidence(key));
    assert_eq!(
        NativeMutationTable::for_program(&tuple.program).knowledge(key),
        NativeMutationKnowledge::Unknown
    );
    eprintln!("SLICE_SOURCE sharedTupleRoot actualDefinition={:?} actualHeader={header:?} root={bounds:?} checkedTuple={root_ty:?} Lifetime=None ValueLoans=None mutationKnowledge=UNKNOWN body/source/scalarEvidence=ABSENT; direct destination storage, not inferred pointee/effect authority", definitions[0]);
    let field = Place {
        local: bounds,
        projection: vec![Projection::Field {
            index: 1,
            identity: None,
        }],
    };
    let field_ty = contract
        .place_type_id(tc, body, &field)
        .expect("canonical tuple field path");
    assert_eq!(tc.try_ty(field_ty), Some(&Ty::I64));
    assert_eq!(
        shared_ref_lease(&tuple, tuple_row, alias, &field),
        vec![field.clone()]
    );
    assert_eq!(tuple_row.aliases, vec![field]);
    for case in [&scalar, &tuple] {
        if case.outcome.is_err() {
            diagnostics(case);
        }
    }
    assert!(scalar.outcome.is_ok() && tuple.outcome.is_ok(),
        "SLICE_SOURCE CLASSIFICATION RED: witnessed fresh scalar and tuple-field shared inputs must preserve checked slice data; scalarAccepted={} tupleAccepted={}; BOTH receipts complete",
        scalar.outcome.is_ok(), tuple.outcome.is_ok());
}

#[test]
fn slice_return_source_authority_opaque_shared_data_pointee_stays_conservative() {
    let source = format!("{VIEWS}{OPAQUE_SHARED}\nshared_data_view: & &[I64] -> &I64\nshared_data_view = metadata ->\n    data: &[I64] = *metadata\n    unsafe\n        pointer: *I64 = ~ArrPtr data\n        pointed: &I64 = &*pointer\n        alias: &I64 = pointed\n        offset = opaque_shared alias\n        &*(~PtrOffset pointer, offset)\n{SHARED_MAIN}");
    let control = control(
        "shared_data_pointee",
        source,
        "    values = [1, 2]",
        "values",
    );
    let row = opaque_shared_receipt(&control, "        offset = opaque_shared alias", true);
    let alternatives = shared_data_pointer(
        &control,
        row,
        "        pointer: *I64 = ~ArrPtr data",
        "        pointed: &I64 = &*pointer",
        "pointed",
    );
    assert!(alternatives.iter().all(|alternative| row.aliases.contains(alternative)),
        "SLICE_SOURCE SECURITY RED: copied data-pointee carrier lost a witnessed loan alternative; additional conservative aliases are retained");
    temporary_slot(&control, SHARED_RESULT);
}

#[test]
fn slice_return_source_authority_opaque_shared_metadata_cast_stays_conservative() {
    let source = format!("{VIEWS}{OPAQUE_SHARED}\nshared_data_view: & &[I64] -> &I64\nshared_data_view = metadata ->\n    data: &[I64] = *metadata\n    unsafe\n        pointer: *I64 = metadata as *I64\n        pointed: &I64 = &*pointer\n        alias: &I64 = pointed\n        offset = opaque_shared alias\n        &*(~PtrOffset (~ArrPtr data), offset)\n{SHARED_MAIN}");
    let control = control(
        "shared_metadata_cast",
        source,
        "    values = [1, 2]",
        "values",
    );
    let row = opaque_shared_receipt(&control, "        offset = opaque_shared alias", true);
    let (id, pointer) = binding(
        &control.program,
        &control.entry,
        &control.source,
        "        pointer: *I64 = metadata as *I64",
        "pointer",
    );
    let (_, pointed) = binding(
        &control.program,
        &control.entry,
        &control.source,
        "        pointed: &I64 = &*pointer",
        "pointed",
    );
    assert_eq!(id, row.function);
    let body = &control.program.functions[&id];
    let tc = &control.program.type_context;
    let contract = &control.program.backend_contract;
    let arguments = body
        .local_decls
        .iter()
        .enumerate()
        .filter(|(_, decl)| decl.source == LocalSource::Argument)
        .collect::<Vec<_>>();
    assert_eq!(body.arg_count, 1);
    assert_eq!(
        arguments.len(),
        1,
        "SLICE_SOURCE HARNESS exact metadata argument required"
    );
    let (index, metadata_decl) = arguments[0];
    let metadata = Place {
        local: Local(index),
        projection: Vec::new(),
    };
    let header = contract.callable(&contract.function_bodies[&id]).unwrap();
    assert_eq!(header.signature.params.len(), 1);
    assert_eq!(header.signature.params[0].semantic_ty, metadata_decl.ty);
    assert!(
        matches!(tc.try_ty(metadata_decl.ty), Some(Ty::Reference { mutable: false, inner })
        if matches!(tc.try_ty(*inner), Some(Ty::Reference { mutable: false, inner })
            if matches!(tc.try_ty(*inner), Some(Ty::Slice(element)) if tc.try_ty(*element) == Some(&Ty::I64))))
    );
    let definitions = body
        .basic_blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| {
            let StatementKind::Assign(dest, value) = &statement.kind else {
                return None;
            };
            (dest.local == pointer).then_some((statement, dest, value))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        definitions.len(),
        1,
        "SLICE_SOURCE HARNESS all pointer definitions required, not any unrelated cast"
    );
    let (cast, dest, Rvalue::Cast(Operand::Copy(source), target)) = definitions[0] else {
        panic!("actual metadata reference-to-pointer cast required");
    };
    assert!(dest.projection.is_empty() && source.projection.is_empty());
    let source_ty = operand_type(&control.program, body, &Operand::Copy(source.clone()));
    assert_eq!(source_ty, metadata_decl.ty);
    assert_eq!(body.local_decls[pointer.0].ty, *target);
    assert!(tc.type_id_tree_is_valid(*target));
    assert!(
        matches!(tc.try_ty(*target), Some(Ty::Pointer(element)) if tc.try_ty(*element) == Some(&Ty::I64))
    );
    assert!(control
        .proof
        .casts
        .iter()
        .any(|(owner, from, to)| owner == &id && *from == source_ty && to == target));
    // Gate1 measures one whole-reference copy before this cast. Require every
    // definition of that actual operand to be this copy of the exact argument;
    // do not admit a same-typed reference from an unrelated slot or reborrow.
    let copies = body
        .basic_blocks
        .iter()
        .enumerate()
        .flat_map(|(block, data)| {
            data.statements
                .iter()
                .enumerate()
                .filter_map(move |(index, statement)| {
                    let StatementKind::Assign(dest, value) = &statement.kind else {
                        return None;
                    };
                    (dest.local == source.local).then_some((block, index, statement, dest, value))
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        copies.len(),
        1,
        "SLICE_SOURCE HARNESS complete measured metadata-copy ancestry required"
    );
    let (block, index, copy, dest, Rvalue::Use(Operand::Copy(root))) = copies[0] else {
        panic!("actual metadata argument copy required, no type-only alias guess");
    };
    assert_eq!(dest, source);
    assert_eq!(root, &metadata);
    let aliases = control
        .proof
        .rows
        .iter()
        .filter(|observed| {
            observed.function == id
                && observed.block == block
                && observed.index == index
                && observed.phase == "AFTER_EXACT_STATEMENT"
                && (observed.place.local == source.local || observed.place.local == metadata.local)
        })
        .collect::<Vec<_>>();
    assert!(
        !aliases.is_empty() && aliases.len() <= 16,
        "SLICE_SOURCE HARNESS complete measured copy-alias receipts cap16"
    );
    assert!(aliases.iter().all(|observed| observed
        .origins
        .contains(&ReferenceOrigin::Param(metadata.local))));
    eprintln!("SLICE_SOURCE sharedMetadataCast actualMetadata={metadata:?} declaration={metadata_decl:?} actualCopy={copy:?} actualCast={cast:?} sourceType={source_ty:?} targetType={target:?} ALL_copyAliasRows={aliases:?}; same metadata reference value, NOT pointee equality/backing/independence evidence");
    shared_ref_lease(
        &control,
        row,
        pointed,
        &Place {
            local: pointer,
            projection: vec![Projection::Deref],
        },
    );
    temporary_slot(&control, SHARED_RESULT);
}

#[test]
fn slice_return_source_authority_opaque_shared_mixed_copied_alias_keeps_all_alternatives() {
    let source = format!("{VIEWS}{OPAQUE_SHARED}\nextern opaque_choice: () -> Bool\nshared_data_view: & &[I64] -> &I64\nshared_data_view = metadata ->\n    data: &[I64] = *metadata\n    fresh: I64 = 0\n    local_ref: &I64 = &fresh\n    unsafe\n        pointer: *I64 = ~ArrPtr data\n        data_ref: &I64 = &*pointer\n        chosen: &I64 = if opaque_choice!\n            local_ref\n        else\n            data_ref\n        alias: &I64 = chosen\n        offset = opaque_shared alias\n        *local_ref\n        *data_ref\n        &*(~PtrOffset pointer, offset)\n{SHARED_MAIN}");
    let control = control(
        "shared_mixed_alias",
        source,
        "    values = [1, 2]",
        "values",
    );
    let row = opaque_shared_receipt(&control, "        offset = opaque_shared alias", true);
    let (id, fresh) = binding(
        &control.program,
        &control.entry,
        &control.source,
        "    fresh: I64 = 0",
        "fresh",
    );
    let (_, local_ref) = binding(
        &control.program,
        &control.entry,
        &control.source,
        "    local_ref: &I64 = &fresh",
        "local_ref",
    );
    let (_, chosen) = binding(
        &control.program,
        &control.entry,
        &control.source,
        "        chosen: &I64 = if opaque_choice!",
        "chosen",
    );
    let (_, alias) = binding(
        &control.program,
        &control.entry,
        &control.source,
        "        alias: &I64 = chosen",
        "alias",
    );
    assert_eq!(id, row.function);
    let fresh_path = Place {
        local: fresh,
        projection: Vec::new(),
    };
    let mut alternatives = shared_ref_lease(&control, row, local_ref, &fresh_path)
        .into_iter()
        .collect::<HashSet<_>>();
    let data = shared_data_pointer(
        &control,
        row,
        "        pointer: *I64 = ~ArrPtr data",
        "        data_ref: &I64 = &*pointer",
        "data_ref",
    );
    assert!(
        !data.contains(&fresh_path),
        "SLICE_SOURCE HARNESS mixed branch targets must be distinct"
    );
    alternatives.extend(data);
    assert!(alternatives.len() >= 2);
    assert!(alternatives.iter().all(|alternative| row.aliases.contains(alternative)),
        "SLICE_SOURCE SECURITY RED: joined shared argument lost a reachable address alternative; additional conservative aliases are retained");
    let body = &control.program.functions[&id];
    assert!(body.basic_blocks.iter().flat_map(|block| &block.statements).any(|statement|
        matches!(&statement.kind, StatementKind::Assign(dest, Rvalue::Use(Operand::Copy(source)))
            if dest.local == alias && dest.projection.is_empty() && source.local == chosen && source.projection.is_empty())));
    let choices = body
        .basic_blocks
        .iter()
        .filter_map(|block| {
            let term @ Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))),
                destination,
                ..
            } = block.terminator.as_ref()?
            else {
                return None;
            };
            (matches!(
                &control.program.backend_contract.callable(key)?.kind,
                MirCallableKind::Extern { .. }
            ) && control
                .program
                .type_context
                .try_ty(body.local_decls[destination.local.0].ty)
                == Some(&Ty::Bool))
            .then_some((term, key))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        choices.len(),
        1,
        "SLICE_SOURCE HARNESS opaque Boolean choice required, not a constant-folded branch"
    );
    let (choice, key) = choices[0];
    let choice_id = MirCallable::Resolved(key.clone()).function_id().unwrap();
    let values = BorrowChecker::native_return_loan_proof(&control.program);
    assert!(!values.sources.has_body_evidence(key));
    assert!(!control.program.functions.contains_key(&choice_id));
    assert!(
        BorrowChecker::reference_return_summaries(&control.program)
            .get(&choice_id)
            .is_none()
            && values.summaries.get(&choice_id).is_none()
    );
    assert_eq!(
        NativeMutationTable::for_program(&control.program).knowledge(key),
        NativeMutationKnowledge::Unknown
    );
    eprintln!("SLICE_SOURCE sharedMixedChoice actualCall={choice:?} actualHeader={:?} body=ABSENT Lifetime=None ValueLoans=None mutationKnowledge=UNKNOWN source/scalarEvidence=ABSENT; both live address alternatives required", control.program.backend_contract.callable(key));
    temporary_slot(&control, SHARED_RESULT);
}
