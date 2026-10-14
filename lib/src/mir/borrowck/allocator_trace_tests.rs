//! Opt-in observation of the unchanged allocator source through production MIR ownership.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use super::*;

// Independent integration observer. Selection and budgets never participate in
// compiler authority; all recorded states come from the unchanged analyses.
static INTEGRATION_TRACE: OnceLock<Mutex<IntegrationTrace>> = OnceLock::new();

#[derive(Default)]
struct IntegrationTrace {
    case: String,
    entry: PathBuf,
    negative_range: Option<(usize, usize)>,
    inputs: std::collections::BTreeMap<PathBuf, Vec<u8>>,
    selected: std::collections::BTreeSet<MirFunctionId>,
    targets: HashMap<MirFunctionId, HashSet<Local>>,
    functions: HashSet<String>,
    records: std::collections::BTreeMap<String, String>,
    origins: HashSet<String>,
    types: HashSet<String>,
    edges: HashSet<String>,
    source_spans: HashSet<String>,
    loan_rows: HashSet<String>,
    gates: HashSet<MirFunctionId>,
    checker_seen: bool,
    detail_bodies: std::collections::BTreeMap<MirFunctionId, IntegrationBodyRequest>,
    detail_body_keys: HashSet<String>,
    detail_statements: HashSet<String>,
    detail_calls: HashSet<String>,
}

#[derive(Clone)]
struct IntegrationBodyRequest {
    caller: MirFunctionId,
    block: usize,
    key: MirCallableKey,
    receiver_aliases: Vec<String>,
}

fn integration_enabled() -> bool {
    std::env::var("ROCK_INTEGRATION_BORROW_TRACE").as_deref() == Ok("1")
}

fn integration_effects_detail() -> bool {
    std::env::var("ROCK_INTEGRATION_BORROW_DETAIL").as_deref() == Ok("effects")
}

fn integration_recording() -> bool {
    if !integration_enabled() { return false; }
    if !integration_effects_detail() { return true; }
    // Fixture filtering is test-only; native selection below uses actual calls.
    matches!(integration_trace().lock().unwrap().case.as_str(),
        "custom_owner_cannot_return_an_erased_payload_borrowing_a_local"
        | "vec_reference_payload_cannot_escape_local"
        | "vec_borrowed_callable_payload_cannot_escape_local")
}

fn integration_trace() -> &'static Mutex<IntegrationTrace> {
    INTEGRATION_TRACE.get_or_init(|| Mutex::new(IntegrationTrace::default()))
}

fn integration_budget(trace: &IntegrationTrace) {
    let origin_cap = integration_origin_cap();
    assert!(trace.functions.len() <= 24 && trace.records.len() <= 128
        && trace.origins.len() <= origin_cap && trace.types.len() <= 256 && trace.edges.len() <= 512
        && trace.source_spans.len() <= 128 && trace.loan_rows.len() <= 128,
        "INTEGRATION_BORROW_TRACE slice budget overflow before truncation: functions={} records={} origins={} types={} edges={} spans={} loans={} caps=24/128/{origin_cap}/256/512 extras=128/128; request bounded scope/budget approval",
        trace.functions.len(), trace.records.len(), trace.origins.len(), trace.types.len(), trace.edges.len(), trace.source_spans.len(), trace.loan_rows.len());
    assert!(trace.detail_body_keys.len() <= 2 && trace.detail_statements.len() <= 64 && trace.detail_calls.len() <= 32,
        "INTEGRATION_BORROW_TRACE detail body budget overflow before truncation: bodies={} statements={} calls={} caps=2/64/32; functions={} records={} origins={} types={} edges={}; request measured cap delta, no partial-body proof",
        trace.detail_body_keys.len(), trace.detail_statements.len(), trace.detail_calls.len(),
        trace.functions.len(), trace.records.len(), trace.origins.len(), trace.types.len(), trace.edges.len());
}

fn integration_origin_cap() -> usize {
    if integration_effects_detail() { 192 } else { 128 }
}

fn integration_select(trace: &mut IntegrationTrace, function: &MirFunction) {
    trace.selected.insert(function.id.clone());
    trace.functions.insert(format!("{}|{:?}", trace.case, function.id));
    integration_budget(trace);
}

fn integration_record(trace: &mut IntegrationTrace, key: String, row: String) {
    let key = format!("{}|{key}", trace.case);
    // Static location/stage keys preserve the first actual decision; replay has
    // separate keys and cannot overwrite that snapshot with a later iteration.
    trace.records.entry(key).or_insert(row);
    if trace.records.len() > 128 {
        for (key, row) in &trace.records { eprintln!("INTEGRATION_BORROW_TRACE budget-frontier key={key:?} {row}"); }
        panic!("INTEGRATION_BORROW_TRACE slice budget overflow before truncation: functions={} records={} origins={} types={} edges={} spans={} loans={} caps=24/128/{}/256/512 extras=128/128; request bounded scope/budget approval", trace.functions.len(), trace.records.len(), trace.origins.len(), trace.types.len(), trace.edges.len(), trace.source_spans.len(), trace.loan_rows.len(), integration_origin_cap());
    }
}

fn integration_source_span(trace: &mut IntegrationTrace, label: &str, span: Option<&Span>) {
    let Some(span) = span else {
        eprintln!("INTEGRATION_BORROW_TRACE {label}=None; no fabricated source fallback");
        return;
    };
    if !trace.source_spans.insert(format!("{}|{span:?}", trace.case)) { return; }
    integration_budget(trace);
    let input = trace.inputs.get(&span.file_path).expect("actual source ownership must be snapshotted");
    assert_eq!(&std::fs::read(&span.file_path).unwrap(), input, "source bytes unchanged at actual observer callback");
    assert!(span.end >= span.start && span.end - span.start <= 4096, "INTEGRATION_BORROW_TRACE source excerpt budget overflow");
    let text = std::str::from_utf8(input.get(span.start..span.end).expect("actual source span byte range")).unwrap();
    eprintln!("INTEGRATION_BORROW_TRACE {label}={span:?} exactText={text:?}");
}

fn integration_origins(trace: &mut IntegrationTrace, function: &MirFunction, place: &Place, origins: &OriginSet) -> String {
    let mut rows = Vec::new();
    for origin in origins {
        trace.origins.insert(format!("{}|{:?}|{place:?}|{origin:?}", trace.case, function.id));
        integration_budget(trace);
        let local = match origin {
            ReferenceOrigin::Local(local) | ReferenceOrigin::Temporary(local)
            | ReferenceOrigin::Param(local) | ReferenceOrigin::ParamContents(local)
            | ReferenceOrigin::ParamBorrowedContents(local) => Some(*local),
            ReferenceOrigin::Static | ReferenceOrigin::UnknownExternal => None,
        };
        if let Some(local) = local {
            trace.targets.entry(function.id.clone()).or_default().insert(local);
        }
        let declaration = local.and_then(|local| function.local_decls.get(local.0));
        integration_source_span(trace, "sourceOriginSpan", declaration.and_then(|decl| decl.span.as_ref()));
        rows.push(format!("{origin:?} origin-declaration={declaration:?} sourceOriginSpan={:?}", declaration.and_then(|decl| decl.span.as_ref())));
    }
    rows.sort();
    format!("{rows:?}")
}

fn integration_path(trace: &mut IntegrationTrace, analysis: &ReferenceEscapeAnalysis<'_>, place: &Place) -> String {
    assert!(place.projection.len() <= 32, "INTEGRATION_BORROW_TRACE checked-prefix budget overflow");
    let mut rows = Vec::new();
    for length in 0..=place.projection.len() {
        let prefix = Place { local: place.local, projection: place.projection[..length].to_vec() };
        let checked = analysis.backend_contract.place_type_id(analysis.type_context, analysis.func, &prefix);
        rows.push(format!("prefix={prefix:?} checkedProjectedType={checked:?}"));
        if let Some(ty) = checked { integration_type(trace, analysis.type_context, analysis.backend_contract, ty); }
    }
    format!("localSourceAndType={:?} checkedPrefixes={rows:?}", analysis.func.local_decls.get(place.local.0))
}

fn integration_type(trace: &mut IntegrationTrace, tc: &crate::type_context::TypeContext, contract: &MirBackendContract, id: TypeId) {
    if !trace.types.insert(format!("{}|{id:?}", trace.case)) { return; }
    integration_budget(trace);
    if integration_effects_detail() {
        if let Some(Ty::Function { params, captures, .. }) = tc.try_ty(id) {
            assert!(params.len() <= 16 && captures.len() <= 16, "bounded detail Fn metadata, not an environment proof");
        }
    }
    eprintln!("INTEGRATION_BORROW_TRACE type case={:?} id={id:?} node={:?} eligible={} normalized={:?}",
        trace.case, tc.try_ty(id), return_type_needs_escape_check(tc, contract, id), contract.normalize_type_id(tc, id));
    if !tc.type_id_tree_is_valid(id) { return; }
    let ty = contract.normalize_type(tc, &tc.type_for(id));
    let mut children = Vec::new();
    match &ty {
        Type::Struct { id: owner, args } | Type::Enum { id: owner, args } => {
            for (index, arg) in args.iter().enumerate() {
                children.push((format!("generic-payload[{index}]-NOT-ownership-proof"), tc.id_for_type(arg)));
            }
            let Some(layout) = contract.nominal_layouts.get(owner) else {
                eprintln!("INTEGRATION_BORROW_TRACE missing-layout owner={owner:?}; no empty authority inferred");
                return;
            };
            let (parameters, fields) = match layout {
                MirNominalLayout::Struct { generic_params, fields, .. } => (generic_params, fields.iter().enumerate()
                    .map(|(index, (name, field))| (format!("physical-field[{index}] name={name:?} identity={:?}", contract.nominal_field_ids.get(owner).and_then(|ids| ids.get(index))), *field)).collect::<Vec<_>>()),
                MirNominalLayout::Enum { generic_params, variants, .. } => {
                    let mut fields = Vec::new();
                    for (ordinal, variant) in variants.iter().enumerate() {
                        match &variant.fields {
                            crate::mir::MirVariantLayoutFields::Unit => {}
                            crate::mir::MirVariantLayoutFields::Positional(types) => {
                                for (index, field) in types.iter().enumerate() {
                                    fields.push((format!("physical-enum-layout-ordinal[{ordinal}] field[{index}]"), *field));
                                }
                            }
                            crate::mir::MirVariantLayoutFields::Named(types) => {
                                for (index, (name, field)) in types.iter().enumerate() {
                                    fields.push((format!("physical-enum-layout-ordinal[{ordinal}] field[{index}] name={name:?}"), *field));
                                }
                            }
                        }
                    }
                    (generic_params, fields)
                }
            };
            assert_eq!(parameters.len(), args.len(), "INTEGRATION_BORROW_TRACE canonical nominal binder arity");
            let substitution = parameters.iter().copied().zip(args.iter().cloned()).collect::<HashMap<_, _>>();
            eprintln!("INTEGRATION_BORROW_TRACE nominal case={:?} id={id:?} owner={owner:?} binders={parameters:?} args={args:?} dropAuthority={:?}; physical fields/raw spelling are NOT contained-value proofs", trace.case, contract.nominal_drop_authority.get(owner));
            for (relation, declared) in fields {
                let actual = contract.normalize_type(tc, &tc.type_for(declared).substitute_generics(&substitution));
                children.push((format!("{relation} declared={declared:?} substituted={actual:?}"), tc.id_for_type(&actual)));
            }
        }
        _ => {
            use crate::type_services::visit::TypeVisitor;
            struct Children(Vec<Type>);
            impl TypeVisitor for Children {
                fn visit_type(&mut self, ty: &Type) { self.0.push(ty.clone()); }
            }
            let mut immediate = Children(Vec::new());
            crate::type_services::visit::visit_type_children(&ty, &mut immediate);
            for child in immediate.0 {
                let relation = match &ty {
                    Type::Pointer(_) => "raw-pointee-NOT-logical-ownership-proof",
                    Type::Function { .. } => "signature-child-NOT-actual-capture-storage",
                    _ => "structural-child",
                };
                children.push((format!("{relation} {child:?}"), tc.id_for_type(&child)));
            }
        }
    }
    children.sort_by_key(|child| child.0.clone());
    for (relation, child) in children {
        trace.edges.insert(format!("{}|{id:?}|{relation}|{child:?}", trace.case));
        integration_budget(trace);
        eprintln!("INTEGRATION_BORROW_TRACE edge case={:?} parent={id:?} relation={relation:?} child={child:?}", trace.case);
        if let Some(child) = child { integration_type(trace, tc, contract, child); }
    }
}

fn integration_state(trace: &mut IntegrationTrace, analysis: &ReferenceEscapeAnalysis<'_>, state: &ReferenceOriginState, locals: &HashSet<Local>) -> Vec<String> {
    let mut rows = Vec::new();
    for (place, origins) in &state.origins {
        if locals.contains(&place.local) {
            rows.push(format!("{place:?}={} {}", integration_origins(trace, analysis.func, place, origins), integration_path(trace, analysis, place)));
        }
    }
    if integration_effects_detail() { assert!(rows.len() <= 128, "bounded detail materialized state array"); }
    rows.sort();
    rows
}

fn integration_operands(rvalue: &Rvalue) -> Vec<Place> {
    let operands = match rvalue {
        Rvalue::Use(operand) | Rvalue::Cast(operand, _) | Rvalue::Object(operand, _)
        | Rvalue::Package(operand, _) | Rvalue::UnaryOp(_, operand) => vec![operand],
        Rvalue::BinaryOp(_, left, right) => vec![left, right],
        Rvalue::Aggregate(_, operands) => operands.iter().collect(),
        Rvalue::Ref(_, place) | Rvalue::Discriminant(place) => return vec![place.clone()],
        Rvalue::Closure(closure) => return closure.captures.iter().map(|capture| capture.place()).collect(),
    };
    operands.into_iter().filter_map(|operand| match operand {
        Operand::Copy(place) | Operand::Move(place) => Some(place.clone()),
        Operand::Constant(_) => None,
    }).collect()
}

fn integration_statement_row(trace: &mut IntegrationTrace, analysis: &ReferenceEscapeAnalysis<'_>, state: &ReferenceOriginState, statement: &StatementData) -> String {
    let StatementKind::Assign(destination, rvalue) = &statement.kind else { return format!("{statement:?}"); };
    integration_source_span(trace, "sourceStatementSpan", statement.span.as_ref());
    let places = std::iter::once(destination.clone()).chain(integration_operands(rvalue)).collect::<Vec<_>>();
    let locals = places.iter().map(|place| place.local).collect::<HashSet<_>>();
    let paths = places.iter().map(|place| {
        let origins = analysis.origins_for_place(state, place);
        format!("place={place:?} {} queryOrigins={}", integration_path(trace, analysis, place), integration_origins(trace, analysis.func, place, &origins))
    }).collect::<Vec<_>>();
    let mut temporary_query = analysis.temporary_origins_for_assignment(state, destination, rvalue).into_iter().collect::<Vec<_>>();
    temporary_query.sort_by_key(|local| local.0);
    let borrowed_queries = match rvalue {
        Rvalue::Ref(_, place) => vec![(place.clone(), analysis.origins_for_borrowed_place(state, place))],
        Rvalue::Closure(closure) => closure.captures.iter().map(|capture| {
            let place = capture.place();
            let origins = match capture.kind {
                crate::mir::MirClosureCaptureKind::ByValue => analysis.origins_for_place(state, &place),
                crate::mir::MirClosureCaptureKind::ByRef | crate::mir::MirClosureCaptureKind::ByMutRef => analysis.origins_for_borrowed_place(state, &place),
            };
            (place, origins)
        }).collect(),
        _ => Vec::new(),
    }.into_iter().map(|(place, origins)| format!("{place:?}={}", integration_origins(trace, analysis.func, &place, &origins))).collect::<Vec<_>>();
    format!("sourceStatementSpan={:?} statement={:?} paths={paths:?} actualBorrowOrCaptureQueries={borrowed_queries:?} temporaryAssignmentQuery={temporary_query:?} materializedState={:?}", statement.span, statement.kind, integration_state(trace, analysis, state, &locals))
}

pub(super) fn integration_begin(program: &MirProgram) {
    if !integration_enabled() { return; }
    let mut trace = integration_trace().lock().unwrap();
    trace.checker_seen = true;
    if let Some(range) = trace.negative_range {
        for (_, function) in program.functions() {
            if function.basic_blocks.iter().any(|block| block.statements.iter().any(|stmt| stmt.span.as_ref()
                .is_some_and(|span| span.file_path == trace.entry && overlaps(span, range)))) {
                integration_select(&mut trace, function);
                trace.targets.entry(function.id.clone()).or_default().insert(Local(0));
            }
        }
    }
}

pub(super) struct IntegrationStatement {
    key: String,
    before: String,
    statement: StatementData,
    diagnostic_start: usize,
}

pub(super) fn integration_statement_before(analysis: &ReferenceEscapeAnalysis<'_>, state: &ReferenceOriginState, statement: &StatementData, block: usize, diagnostic_start: usize) -> Option<IntegrationStatement> {
    if !integration_recording() { return None; }
    let StatementKind::Assign(destination, rvalue) = &statement.kind else { return None; };
    if analysis.temporary_origins_for_assignment(state, destination, rvalue).is_empty() { return None; }
    let mut trace = integration_trace().lock().unwrap();
    integration_select(&mut trace, analysis.func);
    trace.targets.entry(analysis.func.id.clone()).or_default().insert(destination.local);
    Some(IntegrationStatement {
        key: format!("{:?}|decision-statement|{block}|{statement:?}", analysis.func.id),
        before: integration_statement_row(&mut trace, analysis, state, statement),
        statement: statement.clone(), diagnostic_start,
    })
}

pub(super) fn integration_statement_after(analysis: &ReferenceEscapeAnalysis<'_>, state: &ReferenceOriginState, before: Option<IntegrationStatement>, diagnostics: &Diagnostics) {
    let Some(before) = before else { return; };
    let actual = &diagnostics.0[before.diagnostic_start..];
    assert!(!actual.is_empty(), "INTEGRATION_BORROW_TRACE candidate must match actual rejecting decision");
    let mut trace = integration_trace().lock().unwrap();
    let after = integration_statement_row(&mut trace, analysis, state, &before.statement);
    let rows = actual.iter().map(|diag| format!("code={:?} message={:?} sourceOriginSpan={:?}", diag.code, diag.message, diag.primary.as_ref().map(|label| &label.span))).collect::<Vec<_>>();
    integration_record(&mut trace, before.key, format!("ACTUAL-REJECT assignment BEFORE={} AFTER={after} actualDiagnostics={rows:?}", before.before));
}

pub(super) fn integration_call_reject(analysis: &ReferenceEscapeAnalysis<'_>, state: &ReferenceOriginState, block: usize, diagnostics: &[Diagnostic]) {
    if !integration_recording() || diagnostics.is_empty() { return; }
    let term = analysis.func.basic_blocks[block].terminator.as_ref().unwrap();
    let Terminator::Call { destination, args, .. } = term else { return; };
    let mut trace = integration_trace().lock().unwrap();
    integration_select(&mut trace, analysis.func);
    let mut locals = HashSet::from([destination.local]);
    for arg in args { if let Operand::Copy(place) | Operand::Move(place) = arg { locals.insert(place.local); } }
    trace.targets.entry(analysis.func.id.clone()).or_default().extend(locals.iter().copied());
    let before = integration_state(&mut trace, analysis, state, &locals);
    let path = integration_path(&mut trace, analysis, destination);
    let diagnostic_rows = diagnostics.iter().map(|diag| format!("code={:?} message={:?} sourceOriginSpan={:?}", diag.code, diag.message, diag.primary.as_ref().map(|label| &label.span))).collect::<Vec<_>>();
    integration_record(&mut trace, format!("{:?}|decision-call|{block}", analysis.func.id), format!("ACTUAL-REJECT call sourceStatementSpan={:?} term={term:?} destination={path} BEFORE={before:?} actualDiagnostics={diagnostic_rows:?}", analysis.func.terminator_origin(term).source_span()));
}

pub(super) fn integration_return_gate(function: &MirFunction, tc: &crate::type_context::TypeContext, contract: &MirBackendContract) {
    if !integration_recording() { return; }
    let mut trace = integration_trace().lock().unwrap();
    if !trace.selected.contains(&function.id) { return; }
    trace.gates.insert(function.id.clone());
    eprintln!("INTEGRATION_BORROW_TRACE return-validation-reached case={:?} function={:?} declaredReturn={:?} predicate={} outerGate={} local0={:?}", trace.case, function.id, function.ret_type,
        return_type_needs_escape_check(tc, contract, function.ret_type),
        type_id_contains_reference(TypeView::new(tc), contract, function.ret_type) || function.local_decls.iter().any(|decl| type_id_tracks_origin(TypeView::new(tc), contract, decl.ty)), function.local_decls.first());
}

pub(super) fn integration_return_exits(analysis: &ReferenceEscapeAnalysis<'_>, results: &crate::mir::dataflow::Results<ReferenceOriginState>) {
    if !integration_recording() { return; }
    let selected = integration_trace().lock().unwrap().selected.contains(&analysis.func.id);
    if !selected { return; }
    for (block, data) in analysis.func.basic_blocks.iter().enumerate() {
        if matches!(data.terminator, Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })) {
            let mut trace = integration_trace().lock().unwrap();
            let locals = HashSet::from([Local(0)]);
            let rows = integration_state(&mut trace, analysis, &results.exit_sets[block], &locals);
            integration_record(&mut trace, format!("{:?}|return-exit|{block}", analysis.func.id), format!("actual-return-exit reachable={} sourceStatementSpan={:?} originPaths={rows:?}", results.reachable[block], data.terminator.as_ref().and_then(|term| analysis.func.terminator_origin(term).source_span().cloned())));
        }
    }
}

pub(super) fn integration_return_entry(analysis: &ReferenceEscapeAnalysis<'_>, results: &crate::mir::dataflow::Results<ReferenceOriginState>, block: usize, place: &Place, origins: &OriginSet) {
    if !integration_recording() { return; }
    let mut trace = integration_trace().lock().unwrap();
    let checked = analysis.backend_contract.place_type_id(analysis.type_context, analysis.func, place);
    let rejecting = checked.is_none() || (checked.is_some_and(|ty| return_type_needs_escape_check(analysis.type_context, analysis.backend_contract, ty))
        && origins.iter().any(|origin| matches!(origin, ReferenceOrigin::Local(_) | ReferenceOrigin::Temporary(_))));
    if !trace.selected.contains(&analysis.func.id) && !rejecting { return; }
    integration_select(&mut trace, analysis.func);
    trace.gates.insert(analysis.func.id.clone());
    trace.targets.entry(analysis.func.id.clone()).or_default().insert(place.local);
    let path = integration_path(&mut trace, analysis, place);
    let rows = integration_origins(&mut trace, analysis.func, place, origins);
    integration_record(&mut trace, format!("{:?}|return-entry|{block}|{place:?}", analysis.func.id), format!("actual-return-path reachable={} place={place:?} checked={checked:?} predicate={:?} path={path} origins={rows}", results.reachable[block], checked.map(|ty| return_type_needs_escape_check(analysis.type_context, analysis.backend_contract, ty))));
}

pub(super) fn integration_return_reject(analysis: &ReferenceEscapeAnalysis<'_>, block: usize, place: &Place, origin: ReferenceOrigin) {
    if !integration_recording() { return; }
    let mut trace = integration_trace().lock().unwrap();
    integration_select(&mut trace, analysis.func);
    let rows = integration_origins(&mut trace, analysis.func, place, &HashSet::from([origin]));
    let span = analysis.func.basic_blocks[block].terminator.as_ref().and_then(|term| analysis.func.terminator_origin(term).source_span().cloned());
    integration_source_span(&mut trace, "sourceStatementSpan", span.as_ref());
    integration_record(&mut trace, format!("{:?}|decision-return|{block}|{place:?}|{origin:?}", analysis.func.id), format!("ACTUAL-REJECT return sourceStatementSpan={span:?} place={place:?} origins={rows}"));
}

fn integration_header(trace: &mut IntegrationTrace, program: &MirProgram, id: &MirFunctionId, lifetime: &HashMap<MirFunctionId, OriginSet>, values: &HashMap<MirFunctionId, OriginSet>) {
    let contract = &program.backend_contract;
    let function = program.functions.get(id).expect("INTEGRATION_BORROW_TRACE real selected body");
    integration_select(trace, function);
    let key = contract.function_bodies.get(id).expect("INTEGRATION_BORROW_TRACE canonical body key");
    let declaration = contract.callable(key).expect("INTEGRATION_BORROW_TRACE canonical declaration");
    assert_eq!(&declaration.key, key);
    assert!(matches!(&declaration.kind, crate::mir::MirCallableKind::LocalBody { function_id } if function_id == id));
    assert_eq!(function.arg_count, declaration.signature.params.len());
    assert_eq!(function.ret_type, declaration.signature.ret.semantic_ty);
    if function.name == "main" {
        assert_eq!(program.type_context.id_for_type(&Type::I32), Some(declaration.signature.ret.abi_ty));
    } else {
        assert_eq!(function.ret_type, declaration.signature.ret.abi_ty);
    }
    assert_eq!(function.local_decls[0].ty, function.ret_type);
    assert_eq!(function.local_decls[0].source, LocalSource::ReturnPlace);
    assert!(function.arg_count <= 16, "INTEGRATION_BORROW_TRACE header parameter budget overflow");
    if integration_effects_detail() { assert!(function.closure_captures.len() <= 16, "bounded detail native capture array"); }
    let definition = declaration.source_def_id;
    assert!(definition.is_some() || matches!(id, MirFunctionId::Closure(_)), "non-closure native source DefId authority missing");
    eprintln!("INTEGRATION_BORROW_TRACE native case={:?} function={id:?} display={:?} key={key:?} DefId={definition:?} product={:?} header={declaration:?} local0={:?} Lifetime={:?} ValueLoans={:?} LifetimeBody={} ValueLoansBody={}",
        trace.case, function.name, definition.and_then(|definition| contract.descriptor_origins.get(&definition.crate_id)), function.local_decls[0],
        return_summary(lifetime, Some(id)), return_summary(values, Some(id)),
        BorrowChecker::body_available_for_purpose(program, id, function, OriginPurpose::Lifetime),
        BorrowChecker::body_available_for_purpose(program, id, function, OriginPurpose::ValueLoans));
    for (index, param) in declaration.signature.params.iter().enumerate() {
        assert_eq!(function.local_decls[index + 1].ty, param.semantic_ty);
        assert_eq!(function.local_decls[index + 1].source, LocalSource::Argument);
        eprintln!("INTEGRATION_BORROW_TRACE native-param function={id:?} ordinal={index} ABI={param:?} local={:?}", function.local_decls[index + 1]);
    }
    integration_type(trace, &program.type_context, contract, function.ret_type);
    for capture in &function.closure_captures {
        let stored = function.local_decls.get(capture.local.0).expect("actual closure-body storage local");
        assert_eq!(stored.source, LocalSource::ClosureCapture);
        eprintln!("INTEGRATION_BORROW_TRACE closure-body function={id:?} actualCapture={capture:?} storedSlot={stored:?}");
        integration_type(trace, &program.type_context, contract, stored.ty);
    }
}

fn integration_call_authority(trace: &mut IntegrationTrace, program: &MirProgram, operand: &Operand, lifetime: &HashMap<MirFunctionId, OriginSet>, values: &HashMap<MirFunctionId, OriginSet>) {
    let Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) = operand else {
        eprintln!("INTEGRATION_BORROW_TRACE actual indirect callable={operand:?}; TypeOpaque until actual construction/body binding, no fabricated native target or empty authority");
        return;
    };
    let id = ReferenceEscapeAnalysis::callable_function_id(&MirCallable::Resolved(key.clone()));
    let declaration = program.backend_contract.callable(key);
    let body = id.as_ref().and_then(|id| program.functions.get(id));
    if integration_effects_detail() {
        if let Some(declaration) = declaration {
            assert!(declaration.signature.params.len() <= 16, "bounded detail call header");
            for parameter in &declaration.signature.params {
                integration_type(trace, &program.type_context, &program.backend_contract, parameter.semantic_ty);
            }
            integration_type(trace, &program.type_context, &program.backend_contract, declaration.signature.ret.semantic_ty);
            integration_type(trace, &program.type_context, &program.backend_contract, declaration.signature.ret.abi_ty);
        }
        let mutation = NativeMutationTable::for_program(program).knowledge(key);
        eprintln!("INTEGRATION_BORROW_TRACE detail-call-lookup actualKey={key:?} declarationPresent={} currentMirProgramBodyPresent={} LifetimeStatus={} ValueLoansStatus={}; compileLocalMutationKnowledge={mutation:?}; UNKNOWN typed may-screen is not retention permission, backing-owner seal or closureSeal", declaration.is_some(), body.is_some(),
            id.as_ref().map(|id| integration_summary_status(lifetime, id)).unwrap_or_else(|| "NO_NATIVE_FUNCTION_BINDING/UNKNOWN".into()),
            id.as_ref().map(|id| integration_summary_status(values, id)).unwrap_or_else(|| "NO_NATIVE_FUNCTION_BINDING/UNKNOWN".into()));
    }
    eprintln!("INTEGRATION_BORROW_TRACE call-authority case={:?} key={key:?} declaration={declaration:?} Lifetime={:?} ValueLoans={:?} LifetimeBody={:?} ValueLoansBody={:?}", trace.case, return_summary(lifetime, id.as_ref()), return_summary(values, id.as_ref()),
        body.zip(id.as_ref()).map(|(body, id)| BorrowChecker::body_available_for_purpose(program, id, body, OriginPurpose::Lifetime)),
        body.zip(id.as_ref()).map(|(body, id)| BorrowChecker::body_available_for_purpose(program, id, body, OriginPurpose::ValueLoans)));
    if let Some(id) = id.filter(|id| program.functions.contains_key(id)) {
        integration_header(trace, program, &id, lifetime, values);
        // A real callee-local escape requires its producing body, not merely a
        // declared Fn signature or a display-derived callback identity.
        let locals = lifetime.get(&id).into_iter().flatten().filter_map(|origin| {
            match origin {
                ReferenceOrigin::Local(local) | ReferenceOrigin::Temporary(local) => Some(*local),
                _ => None,
            }
        }).collect::<Vec<_>>();
        if !locals.is_empty() {
            trace.targets.entry(id).or_default().extend(std::iter::once(Local(0)).chain(locals));
        }
    }
}

fn integration_returned_owner_locals(function: &MirFunction, contract: &MirBackendContract, tc: &crate::type_context::TypeContext) -> HashSet<Local> {
    let root = Place { local: Local(0), projection: Vec::new() };
    let Some(ty) = contract.place_type_id(tc, function, &root) else { return HashSet::new(); };
    if !matches!(tc.try_ty(ty), Some(Ty::Struct { .. } | Ty::Enum { .. })) { return HashSet::new(); }
    let mut owners = HashSet::from([Local(0)]);
    loop {
        let before = owners.len();
        for block in &function.basic_blocks {
            for statement in &block.statements {
                if let StatementKind::Assign(destination, Rvalue::Use(Operand::Copy(source) | Operand::Move(source))) = &statement.kind {
                    if owners.contains(&destination.local) && destination.projection.is_empty()
                        && source.projection.is_empty() && contract.place_type_id(tc, function, source) == Some(ty)
                    { owners.insert(source.local); }
                }
            }
        }
        if owners.len() == before { return owners; }
    }
}

// Actual operand/capture binding only. This is observer selection, never a
// mutation summary or permission to retain/discharge a loan.
fn integration_receiver_aliases(function: &MirFunction, tc: &crate::type_context::TypeContext, contract: &MirBackendContract,
    func: &Operand, args: &[Operand], owners: &HashSet<Local>, table: &LoanTable, active: &LoanState) -> Vec<String> {
    let mut candidates = args.iter().filter_map(|operand| match operand {
        Operand::Copy(place) | Operand::Move(place) => Some((place.clone(), "explicit-argument")),
        _ => None,
    }).collect::<Vec<_>>();
    if let Operand::Copy(place) | Operand::Move(place) = func {
        candidates.push((place.clone(), "actual-callable-environment"));
        let mut bindings = HashSet::from([place.local]);
        loop {
            let before = bindings.len();
            for block in &function.basic_blocks {
                for statement in &block.statements {
                    let StatementKind::Assign(destination, rvalue) = &statement.kind else { continue; };
                    if !bindings.contains(&destination.local) { continue; }
                    match rvalue {
                        Rvalue::Use(Operand::Copy(source) | Operand::Move(source))
                        | Rvalue::Cast(Operand::Copy(source) | Operand::Move(source), _) => { bindings.insert(source.local); }
                        Rvalue::Closure(closure) => {
                            for capture in &closure.captures {
                                let binding = if capture.kind == crate::mir::MirClosureCaptureKind::ByMutRef {
                                    "actual-bound-mut-capture"
                                } else { "actual-bound-capture" };
                                candidates.push((capture.place(), binding));
                            }
                            candidates.push((destination.clone(), "actual-closure-storage"));
                        }
                        _ => {}
                    }
                }
            }
            if bindings.len() == before { break; }
        }
    }
    let mut rows = std::collections::BTreeSet::new();
    for (place, binding) in candidates {
        let checked = contract.place_type_id(tc, function, &place);
        let mutable_reference = checked.is_some_and(|ty| matches!(tc.try_ty(ty), Some(Ty::Reference { mutable: true, .. })));
        if !mutable_reference && !matches!(binding, "actual-callable-environment" | "actual-closure-storage" | "actual-bound-mut-capture") {
            continue;
        }
        for loan in table.iter().filter(|loan| loan.kind == LoanKind::Mut
            && (active.owner_contains(loan.id, place.local) || loan.initial_owner == place.local)) {
            // Include actual creation-site aliases even when the current active
            // carrier has already been lost; label that absence explicitly.
            let Some(source) = table.place_paths().place(loan.place_path) else { continue; };
            for root in provenance::resolve_places(source, table, active) {
                if !owners.contains(&root.local) { continue; }
                rows.insert(format!("LoanTableAlias binding={binding} carrier={place:?} checkedType={checked:?} loan={loan:?} activeOwner={} resolvedOwner={root:?}", active.owner_contains(loan.id, place.local)));
            }
        }
    }
    rows.into_iter().collect()
}

fn integration_loan_after_statement(active: &mut LoanState, function: &MirFunction, program: &MirProgram,
    statement: &StatementData, block: usize, index: usize, locations: &LoanLocationIndex, table: &LoanTable,
    liveness: &liveness::ReferenceLiveness) {
    apply_active_loan_statement_transfer(active, function, &program.type_context, &program.backend_contract, statement);
    let location = location::Location::new(crate::mir::BasicBlockId(block), location::StatementIndex(index));
    for loan in locations.loan_ids_at(location) { active.activate(loan, table); }
    if let Some(live) = liveness.after_statement_sets.get(block).and_then(|sets| sets.get(index)) {
        active.retain_owners(|owner| live.contains(owner));
    }
    if let StatementKind::StorageDead(local) = &statement.kind { active.release_owner(*local); }
}

fn integration_detail_place(trace: &mut IntegrationTrace, analysis: &ReferenceEscapeAnalysis<'_>,
    state: &ReferenceOriginState, place: &Place, table: &LoanTable, active: &LoanState) -> String {
    let tc = analysis.type_context;
    let checked = analysis.backend_contract.place_type_id(tc, analysis.func, place);
    let normalized = checked.and_then(|ty| analysis.backend_contract.normalize_type_id(tc, ty));
    let gates = checked.map(|ty| (tc.try_ty(ty),
        type_id_tracks_origin(TypeView::new(tc), analysis.backend_contract, ty),
        type_id_contains_reference(TypeView::new(tc), analysis.backend_contract, ty)));
    let origins = analysis.origins_for_place(state, place);
    let loans = table.iter().filter(|loan| active.owner_contains(loan.id, place.local)
        || loan.initial_owner == place.local).map(|loan| {
        trace.loan_rows.insert(format!("{}|{:?}|{:?}", trace.case, analysis.func.id, loan.id));
        integration_budget(trace);
        let resolved = provenance::resolve_loan_places(loan.id, table, active);
        assert!(resolved.len() <= 128, "bounded detail resolved loan paths");
        format!("loan={loan:?} activeCarrier={} indexedPlace={:?} actualResolvedPlaces={resolved:?}",
            active.owner_contains(loan.id, place.local), table.place_paths().place(loan.place_path),
        )
    }).collect::<Vec<_>>();
    assert!(loans.len() <= 128, "bounded detail loan array");
    let indexed = table.place_paths().path_id(place);
    let resolved = provenance::resolve_places(place, table, active);
    assert!(resolved.len() <= 128, "bounded detail resolved place paths");
    let resolved_paths = resolved.iter().map(|resolved| {
        format!("resolvedPlace={resolved:?} actualPathId={:?} {}", table.place_paths().path_id(resolved), integration_path(trace, analysis, resolved))
    }).collect::<Vec<_>>();
    let root = Place { local: place.local, projection: Vec::new() };
    let materialized = state.origins.get(&ReferenceEscapeAnalysis::normalized_place(place.clone()));
    let root_materialized = state.origins.get(&root);
    format!("place={place:?} {} normalizedType={normalized:?} TypeView_tracksOrigin_containsReference={gates:?} root_tracksOrigin_containsReference_rawPointer_safeReference={:?} materializedExact={materialized:?} materializedRoot={root_materialized:?} queryOrigins={} actualPathId={indexed:?} resolvedCheckedPaths={resolved_paths:?} actualLoans={loans:?}; no parameter-entry loan synthesis; raw resolution is provenance only, never a checked backing-owner seal",
        integration_path(trace, analysis, place),
        (analysis.local_type_tracks_origin(place.local), analysis.local_type_contains_reference(place.local), analysis.local_type_is_pointer(place.local), analysis.local_type_is_reference(place.local)),
        integration_origins(trace, analysis.func, place, &origins))
}

fn integration_detail_statement_row(trace: &mut IntegrationTrace, analysis: &ReferenceEscapeAnalysis<'_>,
    state: &ReferenceOriginState, statement: &StatementData, table: &LoanTable, active: &LoanState) -> String {
    let StatementKind::Assign(destination, value) = &statement.kind else { return format!("{statement:?}"); };
    let operands = integration_operands(value);
    assert!(operands.len() <= 32, "bounded detail statement operand array");
    if let Rvalue::Closure(closure) = value { assert!(closure.captures.len() <= 16, "bounded detail capture array"); }
    let paths = std::iter::once(destination).chain(operands.iter()).map(|place| {
        integration_detail_place(trace, analysis, state, place, table, active)
    }).collect::<Vec<_>>();
    let normalized = ReferenceEscapeAnalysis::normalized_place(destination.clone());
    let discarded_by_root_gate = normalized.projection.is_empty() && !analysis.local_type_tracks_origin(normalized.local)
        && !callable_forwarding_rvalue(analysis.func, analysis.type_context, analysis.backend_contract, destination, value);
    let input = match value {
        Rvalue::Use(operand) | Rvalue::Cast(operand, _) | Rvalue::Object(operand, _) | Rvalue::Package(operand, _) => {
            integration_origins(trace, analysis.func, destination, &analysis.origins_for_operand(state, operand))
        }
        _ => "NOT_OPERAND_FORWARDING; actual borrow/capture queries below".into(),
    };
    format!("{} actualTypedOperandGatesAndLoanOwners={paths:?} existingOperandInputOrigins={input} normalizedAssignmentDestination={normalized:?} assign_origins_discarded_by_root_gate={discarded_by_root_gate}; Lifetime replay only; actual move/copy LoanState replay is separate; Fn signature emptiness is NOT environment proof",
        integration_statement_row(trace, analysis, state, statement))
}

fn integration_request_body(trace: &mut IntegrationTrace, analysis: &ReferenceEscapeAnalysis<'_>,
    block: usize, term: &Terminator, aliases: &[String]) {
    if !integration_effects_detail() { return; }
    let Terminator::Call { func, destination, args, .. } = term else { return; };
    let tc = analysis.type_context;
    let contract = analysis.backend_contract;
    if !args.iter().any(|arg| matches!(arg, Operand::Copy(place) | Operand::Move(place)
        if contract.place_type_id(tc, analysis.func, place)
            .is_some_and(|ty| matches!(tc.try_ty(ty), Some(Ty::Reference { mutable: false, .. } | Ty::Function { .. }))))) { return; }
    if !contract.place_type_id(tc, analysis.func, destination)
        .is_some_and(|ty| matches!(tc.try_ty(ty), Some(Ty::Unit))) { return; }
    if !args.iter().any(|arg| matches!(arg, Operand::Copy(place) | Operand::Move(place)
        if contract.place_type_id(tc, analysis.func, place)
            .is_some_and(|ty| matches!(tc.try_ty(ty), Some(Ty::Reference { mutable: true, .. }))))) { return; }
    let Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) = func else {
        eprintln!("INTEGRATION_BORROW_TRACE detail Unit receiver call has unknown native body binding: caller={:?} block={block} func={func:?}; no callee guessed", analysis.func.id);
        return;
    };
    let declaration = contract.callable(key);
    let id = declaration.and_then(|declaration| match &declaration.kind {
        crate::mir::MirCallableKind::LocalBody { function_id } => Some(function_id.clone()),
        _ => None,
    });
    eprintln!("INTEGRATION_BORROW_TRACE detail body-request caller={:?} block={block} actualKey={key:?} header={declaration:?} bodyBinding={id:?}; absent/ObjectProvided is unknown mutation, never known-empty",
        analysis.func.id);
    if let Some(id) = id {
        assert!(aliases.len() <= 128, "bounded detail receiver alias array");
        if let Some(declaration) = declaration {
            assert!(declaration.signature.params.len() <= 16, "bounded detail selected-native header");
        }
        trace.detail_body_keys.insert(format!("{}|{id:?}|{key:?}", trace.case));
        integration_budget(trace);
        trace.detail_bodies.entry(id).or_insert_with(|| IntegrationBodyRequest {
            caller: analysis.func.id.clone(), block, key: key.clone(), receiver_aliases: aliases.to_vec(),
        });
    }
}

fn integration_detail_locals(analysis: &ReferenceEscapeAnalysis<'_>, places: impl IntoIterator<Item = Place>) -> Vec<Local> {
    places.into_iter().filter_map(|place| {
        analysis.backend_contract.place_type_id(analysis.type_context, analysis.func, &place)
            .filter(|ty| type_id_tracks_origin(TypeView::new(analysis.type_context), analysis.backend_contract, *ty)
                || matches!(analysis.type_context.try_ty(*ty), Some(Ty::Function { .. })))
            .map(|_| place.local)
    }).collect()
}

fn integration_detail_body(trace: &mut IntegrationTrace, program: &MirProgram, id: &MirFunctionId,
    request: &IntegrationBodyRequest, lifetime: &HashMap<MirFunctionId, OriginSet>, values: &HashMap<MirFunctionId, OriginSet>) {
    let Some(function) = program.functions.get(id) else {
        integration_record(trace, format!("{id:?}|detail-body-absent"), format!("actualRequestedKey={:?} bodyAbsent=true mutationAuthority=Unknown; no empty summary substituted", request.key));
        return;
    };
    assert_eq!(program.backend_contract.function_bodies.get(id), Some(&request.key));
    integration_header(trace, program, id, lifetime, values);
    let analysis = ReferenceEscapeAnalysis::new(function, &program.type_context, &program.backend_contract,
        lifetime, MissingSummaryMode::Conservative);
    let results = run_fixpoint(&analysis, function);
    let borrows = collect_function_borrows_with_contract(function, &program.type_context, &program.backend_contract, values);
    let table = LoanTable::from_borrows_with_paths(&borrows, paths::PlacePathTable::for_function(function));
    assert!(table.len() <= 128, "bounded detail native body loan table");
    let locations = LoanLocationIndex::for_function(&table, function);
    for loan in table.iter() {
        trace.loan_rows.insert(format!("{}|{id:?}|{:?}", trace.case, loan.id));
        integration_budget(trace);
    }
    let liveness = compute_reference_liveness_with_contract(function, &program.type_context, &program.backend_contract);
    let entries = compute_active_loan_entries_with_contract(function, &program.type_context, &program.backend_contract,
        &table, &locations, &liveness, values);
    let mut statements = std::collections::BTreeSet::new();
    let mut calls = std::collections::BTreeSet::new();
    let mut locals = HashSet::new();
    let mut first_ceiling = None;
    let mut ceiling_count = 0;
    // Seed actual projected writes and pointer producers. Follow definitions,
    // not spelling, nominal names, imported source reparsing or guessed owners.
    for (block_index, block) in function.basic_blocks.iter().enumerate() {
        for (index, statement) in block.statements.iter().enumerate() {
            if let StatementKind::Assign(destination, value) = &statement.kind {
                let raw_cast = matches!(value, Rvalue::Cast(_, _))
                    && std::iter::once(destination.clone()).chain(integration_operands(value)).any(|place| {
                        program.backend_contract.place_type_id(&program.type_context, function, &place)
                            .is_some_and(|ty| matches!(program.type_context.try_ty(ty), Some(Ty::Pointer(_))))
                    });
                if !destination.projection.is_empty() || raw_cast {
                    statements.insert((block_index, index));
                    locals.insert(destination.local);
                    locals.extend(integration_detail_locals(&analysis, integration_operands(value)));
                }
            }
        }
        if let Some(Terminator::Call { func, args, destination, .. }) = &block.terminator {
            let pointer = program.backend_contract.place_type_id(&program.type_context, function, destination)
                .is_some_and(|ty| matches!(program.type_context.try_ty(ty), Some(Ty::Pointer(_))));
            let mutable = args.iter().any(|arg| matches!(arg, Operand::Copy(place) | Operand::Move(place)
                if program.backend_contract.place_type_id(&program.type_context, function, place)
                    .is_some_and(|ty| matches!(program.type_context.try_ty(ty), Some(Ty::Reference { mutable: true, .. })))));
            let raw_unit_call = program.backend_contract.place_type_id(&program.type_context, function, destination)
                .is_some_and(|ty| matches!(program.type_context.try_ty(ty), Some(Ty::Unit)))
                && args.iter().any(|arg| matches!(arg, Operand::Copy(place) | Operand::Move(place)
                    if program.backend_contract.place_type_id(&program.type_context, function, place)
                        .is_some_and(|ty| matches!(program.type_context.try_ty(ty), Some(Ty::Pointer(_))))));
            let ceiling = !matches!(func, Operand::Constant(Constant::Callable(_)));
            if ceiling { ceiling_count += 1; }
            let first = ceiling && first_ceiling.is_none();
            if first { first_ceiling = Some(block_index); }
            if pointer || mutable || raw_unit_call || first {
                calls.insert(block_index);
                locals.insert(destination.local);
                if let Operand::Copy(place) | Operand::Move(place) = func { locals.insert(place.local); }
                locals.extend(integration_detail_locals(&analysis, args.iter().filter_map(|arg| match arg {
                    Operand::Copy(place) | Operand::Move(place) => Some(place.clone()), _ => None,
                })));
            }
        }
    }
    loop {
        let before = (locals.len(), statements.len(), calls.len());
        for (block_index, block) in function.basic_blocks.iter().enumerate() {
            for (index, statement) in block.statements.iter().enumerate() {
                if let StatementKind::Assign(destination, value) = &statement.kind {
                    if locals.contains(&destination.local) {
                        statements.insert((block_index, index));
                        locals.extend(integration_detail_locals(&analysis, integration_operands(value)));
                    }
                }
            }
            if let Some(Terminator::Call { func, args, destination, .. }) = &block.terminator {
                if locals.contains(&destination.local) {
                    calls.insert(block_index);
                    if let Operand::Copy(place) | Operand::Move(place) = func { locals.insert(place.local); }
                    locals.extend(integration_detail_locals(&analysis, args.iter().filter_map(|arg| match arg {
                        Operand::Copy(place) | Operand::Move(place) => Some(place.clone()), _ => None,
                    })));
                }
            }
        }
        if before == (locals.len(), statements.len(), calls.len()) { break; }
    }
    for (block, index) in &statements { trace.detail_statements.insert(format!("{}|{id:?}|{block}|{index}", trace.case)); }
    for block in &calls { trace.detail_calls.insert(format!("{}|{id:?}|{block}", trace.case)); }
    eprintln!("INTEGRATION_BORROW_TRACE detail-slice-count case={:?} actualNative={id:?} selectedStatements={} selectedCalls={} cumulativeBodies={} cumulativeStatements={} cumulativeCalls={} bodyCaps=2/64/32; no MIR truncation", trace.case, statements.len(), calls.len(), trace.detail_body_keys.len(), trace.detail_statements.len(), trace.detail_calls.len());
    integration_budget(trace); // Measure the complete selected slice before formatting any body arrays.
    integration_record(trace, format!("{id:?}|detail-body-binding"), format!(
        "actualCaller={:?} block={} actualKey={:?} receiverAliasEvidence={:?} header={:?} bodyId={:?} Lifetime={} ValueLoans={} LifetimeBody={} ValueLoansBody={}; actualNonstaticCallCount={ceiling_count} firstStaticProofCeilingBlock={first_ceiling:?}; mutationSummary=ABSENT (not known-empty); slice includes first actual static ceiling, pointer producers, mutable-reference calls, raw-pointer Unit calls, projected stores/casts and typed ancestry, NOT all body MIR; transitive native backing-store bodies outside this two-body slice remain UNKNOWN; Lifetime uses unchanged ReferenceEscapeAnalysis with conservative missing summaries; LoanState uses unchanged ValueLoans/default collection authority, never substituted for Lifetime",
        request.caller, request.block, request.key, request.receiver_aliases, program.backend_contract.callable(&request.key), function.id,
        integration_summary_status(lifetime, id), integration_summary_status(values, id),
        BorrowChecker::body_available_for_purpose(program, id, function, OriginPurpose::Lifetime),
        BorrowChecker::body_available_for_purpose(program, id, function, OriginPurpose::ValueLoans)));
    for (block_index, block) in function.basic_blocks.iter().enumerate() {
        let mut state = results.entry_sets[block_index].clone();
        let mut active = entries[block_index].clone();
        for (index, statement) in block.statements.iter().enumerate() {
            let before = statements.contains(&(block_index, index)).then(|| {
                integration_detail_statement_row(trace, &analysis, &state, statement, &table, &active)
            });
            analysis.apply_statement(&mut state, statement);
            integration_loan_after_statement(&mut active, function, program, statement, block_index, index, &locations, &table, &liveness);
            if let Some(before) = before {
                let after = integration_detail_statement_row(trace, &analysis, &state, statement, &table, &active);
                integration_record(trace, format!("{id:?}|detail-body-statement|{block_index}|{index}"),
                    format!("location=({block_index},{index}) reachable={} actual-transfer BEFORE={before} AFTER={after}; projected installation is this checked destination only, not heap ownership inferred from a pointer", results.reachable[block_index]));
            }
        }
        if calls.contains(&block_index) {
            let Some(term @ Terminator::Call { func, args, destination, target, .. }) = &block.terminator else { continue; };
            assert!(args.len() <= 16, "bounded detail call argument array");
            integration_source_span(trace, "sourceStatementSpan", function.terminator_origin(term).source_span());
            integration_call_authority(trace, program, func, lifetime, values);
            integration_bound_call_authority(trace, program, function, func, lifetime, values);
            let mut places = vec![destination.clone()];
            if let Operand::Copy(place) | Operand::Move(place) = func { places.push(place.clone()); }
            for arg in args { if let Operand::Copy(place) | Operand::Move(place) = arg { places.push(place.clone()); } }
            let before = places.iter().map(|place| integration_detail_place(trace, &analysis, &state, place, &table, &active)).collect::<Vec<_>>();
            let envelope = analysis.call_return_origins(&state, func, args, destination);
            analysis.apply_terminator(&mut state, term);
            let after = places.iter().map(|place| integration_detail_place(trace, &analysis, &state, place, &table, &active)).collect::<Vec<_>>();
            let envelope = integration_origins(trace, function, destination, &envelope);
            integration_record(trace, format!("{id:?}|detail-body-call|{block_index}"), format!(
                "location=({block_index},terminator) reachable={} actualCall={term:?} staticValueLoansCeiling={} actualResultEnvelope={} LifetimeBEFORE={before:?} LifetimeAFTER={after:?} actualLoansBeforeTerminator={:?} actualSuccessorJoinedLoans={:?}; AFTER paths still show pre-terminator loans, successor is joined production LoanState; ParamBorrowedContents describes a storage dependency, NOT a heap ownership/write permission seal",
                results.reachable[block_index], !matches!(func, Operand::Constant(Constant::Callable(_))),
                envelope, loan_summary(&active), loan_summary(&entries[target.0])));
        }
    }
}

fn integration_summary_status(map: &HashMap<MirFunctionId, OriginSet>, id: &MirFunctionId) -> String {
    match map.get(id) {
        None => "ABSENT/UNKNOWN".into(),
        Some(origins) if origins.is_empty() => "PRESENT_EMPTY_RETURN_ONLY".into(),
        Some(_) => format!("PRESENT_RETURN_ONLY {:?}", return_summary(map, Some(id))),
    }
}

fn integration_bound_call_authority(trace: &mut IntegrationTrace, program: &MirProgram, function: &MirFunction,
    operand: &Operand, lifetime: &HashMap<MirFunctionId, OriginSet>, values: &HashMap<MirFunctionId, OriginSet>) {
    let (Operand::Copy(place) | Operand::Move(place)) = operand else { return; };
    let mut bindings = HashSet::from([place.local]);
    let mut seen = HashSet::new();
    loop {
        let before = bindings.len();
        for block in &function.basic_blocks {
            for statement in &block.statements {
                let StatementKind::Assign(destination, rvalue) = &statement.kind else { continue; };
                if !bindings.contains(&destination.local) { continue; }
                match rvalue {
                    Rvalue::Use(Operand::Copy(source) | Operand::Move(source))
                    | Rvalue::Cast(Operand::Copy(source) | Operand::Move(source), _) => { bindings.insert(source.local); }
                    Rvalue::Use(callable @ Operand::Constant(Constant::Callable(_))) => {
                        integration_call_authority(trace, program, callable, lifetime, values);
                    }
                    Rvalue::Closure(closure) if seen.insert(closure.id.clone()) => {
                        if integration_effects_detail() {
                            assert!(closure.captures.len() <= 16, "bounded detail bound-callable capture array");
                            eprintln!("INTEGRATION_BORROW_TRACE detail indirect binding is an actual lexical MIR construction candidate, not must-evidence of callee/environment independence across joins");
                        }
                        let id = MirFunctionId::Closure(Box::new(closure.id.clone()));
                        let key = program.backend_contract.function_bodies.get(&id);
                        eprintln!("INTEGRATION_BORROW_TRACE actual-bound-callable operand={operand:?} construction={:?} closureId={:?} canonicalKey={key:?} header={:?} actualCaptures={:?}", statement.span, closure.id, key.and_then(|key| program.backend_contract.callable(key)), closure.captures);
                        let Some(body) = program.functions.get(&id) else {
                            eprintln!("INTEGRATION_BORROW_TRACE actual-bound-callable bodyAbsent={id:?}; TypeOpaque, not empty/error-skip");
                            continue;
                        };
                        integration_header(trace, program, &id, lifetime, values);
                        // Inspect the actual bound-method body, not its display
                        // spelling. These are source/capture bindings, not effects.
                        for (block_index, block) in body.basic_blocks.iter().enumerate() {
                            if let Some(term @ Terminator::Call { func, .. }) = &block.terminator {
                                if integration_effects_detail() {
                                    trace.detail_calls.insert(format!("{}|{id:?}|{block_index}", trace.case));
                                    integration_budget(trace);
                                }
                                integration_call_authority(trace, program, func, lifetime, values);
                                integration_record(trace, format!("{id:?}|bound-body-call|{block_index}"),
                                    format!("actual-bound-body caller={:?} constructedAt={:?} captureStorage={:?} term={term:?}; body binding only, no invented retention", function.id, statement.span, body.closure_captures));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if bindings.len() == before { break; }
    }
}

pub(super) fn integration_program(program: &MirProgram, lifetime: &HashMap<MirFunctionId, OriginSet>, values: &HashMap<MirFunctionId, OriginSet>, diagnostics: &Diagnostics) {
    if !integration_recording() { return; }
    assert!(diagnostics.0.len() <= 128, "INTEGRATION_BORROW_TRACE actual diagnostic budget overflow");
    for diagnostic in &diagnostics.0 {
        eprintln!("INTEGRATION_BORROW_TRACE checker-actual-diagnostic code={:?} severity={:?} location={:?} primary={:?} message={:?}", diagnostic.code, diagnostic.severity, diagnostic.location, diagnostic.primary.as_ref().map(|label| &label.span), diagnostic.message);
    }
    {
        let mut trace = integration_trace().lock().unwrap();
        if trace.negative_range.is_some() {
            assert!(program.backend_contract.owned_object_calls.len() <= 24, "INTEGRATION_BORROW_TRACE owner protocol budget overflow");
            for (key, plan) in &program.backend_contract.owned_object_calls {
                eprintln!("INTEGRATION_BORROW_TRACE actual-selected-owner key={key:?} plan={plan:?} intoPartsHeader={:?} releaseHeader={:?}", program.backend_contract.callable(&plan.into_parts), program.backend_contract.callable(&plan.release));
                integration_type(&mut trace, &program.type_context, &program.backend_contract, key.owner);
                integration_type(&mut trace, &program.type_context, &program.backend_contract, key.object);
            }
        }
    }
    let mut replayed: HashMap<MirFunctionId, HashSet<Local>> = HashMap::new();
    let mutations = NativeMutationTable::for_program(program);
    loop {
        let id = {
            let trace = integration_trace().lock().unwrap();
            trace.selected.iter().find(|id| trace.targets.get(*id).is_some_and(|targets| {
                !replayed.get(*id).is_some_and(|done| targets.is_subset(done))
            })).cloned()
        };
        let Some(id) = id else { break; };
        let function = program.functions.get(&id).unwrap();
        let candidates = function.basic_blocks.iter().map(|block| block.terminator.as_ref()
            .map(|term| native_mutation_candidates(function, &program.type_context, &program.backend_contract, &mutations, term))
            .unwrap_or_default()).collect::<Vec<_>>();
        let borrows = collect_function_borrows_with_contract(function, &program.type_context, &program.backend_contract, values);
        let table = LoanTable::from_borrows_with_paths(&borrows, crate::mir::borrowck::paths::PlacePathTable::for_function(function));
        let locations = LoanLocationIndex::for_function(&table, function);
        let liveness = liveness::compute_reference_liveness_with_mutations(function, &program.type_context, &program.backend_contract, &table, &candidates);
        let entries = liveness::compute_active_loan_entries_with_mutations(function, &program.type_context, &program.backend_contract, &table, &locations, &liveness, values, &candidates);
        let loans = liveness::ActiveLoanAnalysis::new(&program.type_context, &program.backend_contract, &table, &locations, &liveness, values).with_mutations(&candidates);
        let mut bindings = NativeMutationOrigins::new();
        for (block, data) in function.basic_blocks.iter().enumerate() {
            if let Some(term) = &data.terminator {
                let mut state = entries[block].clone();
                loans.apply_statements(&mut state, function, crate::mir::BasicBlockId(block));
                bindings.insert(block, native_mutation_bindings(function, &program.type_context,
                    &program.backend_contract, &table, &state, term, &candidates[block]));
            }
        }
        let mut analysis = ReferenceEscapeAnalysis::new(function, &program.type_context, &program.backend_contract, lifetime, MissingSummaryMode::Conservative);
        analysis.mutation_origins = Some(&bindings);
        // Replay is immutable and uses exactly the checker maps and transfer
        // functions; it neither repairs nor replaces any production authority.
        let results = run_fixpoint(&analysis, function);
        let mut trace = integration_trace().lock().unwrap();
        integration_header(&mut trace, program, &id, lifetime, values);
        let mut locals = trace.targets.get(&id).cloned().unwrap_or_default();
        // Exact closure-owner ancestry exposes the callback producer and its
        // real construction/capture slots (including constant native wrappers).
        if let MirFunctionId::Closure(closure_id) = &id {
            if let Some(parent) = program.functions.get(&closure_id.owner) {
                for block in &parent.basic_blocks {
                    for statement in &block.statements {
                        if let StatementKind::Assign(destination, Rvalue::Closure(closure)) = &statement.kind {
                            if &closure.id == closure_id.as_ref() {
                                integration_select(&mut trace, parent);
                                trace.targets.entry(parent.id.clone()).or_default().extend(
                                    std::iter::once(destination.local).chain(closure.captures.iter().map(|capture| capture.local)));
                                eprintln!("INTEGRATION_BORROW_TRACE actual-producer-ancestor parent={:?} closureId={:?} destination={destination:?} sourceStatementSpan={:?} captures={:?}", parent.id, closure.id, statement.span, closure.captures);
                            }
                        }
                    }
                }
            } else {
                eprintln!("INTEGRATION_BORROW_TRACE actual-producer-ancestor bodyAbsent owner={:?}; opaque, not empty or error-skip", closure_id.owner);
            }
        }
        let owners = integration_returned_owner_locals(function, &program.backend_contract, &program.type_context);
        let mut receiver_calls = HashMap::new();
        for (block_index, block) in function.basic_blocks.iter().enumerate() {
            if !results.reachable[block_index] { continue; }
            let mut active = entries[block_index].clone();
            for (index, statement) in block.statements.iter().enumerate() {
                integration_loan_after_statement(&mut active, function, program, statement, block_index, index, &locations, &table, &liveness);
            }
            if let Some(Terminator::Call { func, args, .. }) = &block.terminator {
                let aliases = integration_receiver_aliases(function, &program.type_context, &program.backend_contract,
                    func, args, &owners, &table, &active);
                if !aliases.is_empty() { receiver_calls.insert(block_index, aliases); }
            }
        }
        let mut payload_staging = HashSet::new();
        if integration_effects_detail() {
            for (block_index, aliases) in &receiver_calls {
                let term = function.basic_blocks[*block_index].terminator.as_ref().unwrap();
                integration_request_body(&mut trace, &analysis, *block_index, term, aliases);
                if let Terminator::Call { args, .. } = term {
                    for arg in args {
                        if let Operand::Copy(place) | Operand::Move(place) = arg {
                            if program.backend_contract.place_type_id(&program.type_context, function, place)
                                .is_some_and(|ty| matches!(program.type_context.try_ty(ty), Some(Ty::Function { .. })))
                            { payload_staging.insert(place.local); }
                        }
                    }
                }
            }
            // Force unchanged forwarding operations on the actual payload's
            // definition ancestry, not a global Fn-storage selection rule.
            loop {
                let before = payload_staging.len();
                for block in &function.basic_blocks {
                    for statement in &block.statements {
                        if let StatementKind::Assign(destination, value @ (Rvalue::Use(_)
                            | Rvalue::Cast(..) | Rvalue::Object(..) | Rvalue::Package(..) | Rvalue::Ref(..))) = &statement.kind {
                            if payload_staging.contains(&destination.local) {
                                payload_staging.extend(integration_operands(value).iter().map(|place| place.local));
                            }
                        }
                    }
                }
                if payload_staging.len() == before { break; }
            }
            locals.extend(payload_staging.iter().copied());
        }
        loop {
            let before = locals.len();
            for (block_index, block) in function.basic_blocks.iter().enumerate() {
                for stmt in &block.statements {
                    if let StatementKind::Assign(dest, rvalue) = &stmt.kind {
                        if locals.contains(&dest.local) { locals.extend(integration_operands(rvalue).iter().map(|place| place.local)); }
                    }
                }
                if let Some(Terminator::Call { destination, func, args, .. }) = &block.terminator {
                    if locals.contains(&destination.local) || receiver_calls.contains_key(&block_index) {
                        locals.insert(destination.local);
                        if let Operand::Copy(place) | Operand::Move(place) = func { locals.insert(place.local); }
                        for arg in args { if let Operand::Copy(place) | Operand::Move(place) = arg { locals.insert(place.local); } }
                    }
                }
            }
            if before == locals.len() { break; }
        }
        for loan in table.iter().filter(|loan| locals.contains(&loan.initial_owner) || locals.contains(&loan.diagnostic_place.local)) {
            let loan_key = format!("{}|{id:?}|{:?}", trace.case, loan.id);
            trace.loan_rows.insert(loan_key);
            integration_budget(&trace);
            eprintln!("INTEGRATION_BORROW_TRACE actual-loan function={id:?} loan={loan:?} checkedPlace={:?}", table.place_paths().place(loan.place_path));
        }
        for (block_index, block) in function.basic_blocks.iter().enumerate() {
            if !results.reachable[block_index] { continue; }
            let mut state = results.entry_sets[block_index].clone();
            let mut active_loans = entries[block_index].clone();
            let entry_rows = integration_state(&mut trace, &analysis, &state, &locals);
            let exit_rows = integration_state(&mut trace, &analysis, &results.exit_sets[block_index], &locals);
            if entry_rows != exit_rows {
                integration_record(&mut trace, format!("{id:?}|join|{block_index}"), format!("actual-reachable-block={block_index} control={:?} ENTRY={entry_rows:?} EXIT={exit_rows:?} activeLoanEntry={:?}", block.terminator, loan_summary(&entries[block_index])));
            }
            for (index, statement) in block.statements.iter().enumerate() {
                let relevant = match &statement.kind {
                    StatementKind::Assign(dest, _) => locals.contains(&dest.local),
                    _ => false,
                };
                let staging = integration_effects_detail() && matches!(&statement.kind,
                    StatementKind::Assign(destination, value) if payload_staging.contains(&destination.local)
                        || matches!(value, Rvalue::Closure(_)));
                let before = relevant.then(|| if staging {
                    integration_detail_statement_row(&mut trace, &analysis, &state, statement, &table, &active_loans)
                } else { integration_statement_row(&mut trace, &analysis, &state, statement) });
                let mut live_loans = active_loans.clone();
                if let Some(live) = liveness.before_statement_sets.get(block_index).and_then(|sets| sets.get(index)) {
                    live_loans.retain_owners(|owner| live.contains(owner));
                }
                let before_loans = relevant.then(|| loan_summary(&live_loans));
                analysis.apply_statement(&mut state, statement);
                integration_loan_after_statement(&mut active_loans, function, program, statement, block_index, index, &locations, &table, &liveness);
                if let Some(before) = before {
                    let after = if staging {
                        integration_detail_statement_row(&mut trace, &analysis, &state, statement, &table, &active_loans)
                    } else { integration_statement_row(&mut trace, &analysis, &state, statement) };
                    if staging || before != after || matches!(&statement.kind, StatementKind::Assign(_, Rvalue::Cast(..) | Rvalue::Ref(..) | Rvalue::Closure(..) | Rvalue::Object(..) | Rvalue::Package(..))) {
                        let detail = if staging {
                            format!("actualPayloadStaging=true location=({block_index},{index}) callerNativeKey={:?} callerHeader={:?} ", program.backend_contract.function_bodies.get(&id), program.backend_contract.function_bodies.get(&id).and_then(|key| program.backend_contract.callable(key)))
                        } else { String::new() };
                        integration_record(&mut trace, format!("{id:?}|replay-statement|{block_index}|{index}"), format!("{detail}actual-transfer BEFORE={before} AFTER={after} activeLoansBeforeRetained={before_loans:?} activeLoansAfterRetained={:?}", loan_summary(&active_loans)));
                    }
                    if let StatementKind::Assign(_, Rvalue::Closure(closure)) = &statement.kind {
                        let body_id = MirFunctionId::Closure(Box::new(closure.id.clone()));
                        let Some(body) = program.functions.get(&body_id) else {
                            eprintln!("INTEGRATION_BORROW_TRACE actualClosureSource closureId={:?} bodyAbsent={body_id:?} statementSpan={:?}; opaque, not empty or error-skip", closure.id, statement.span);
                            continue;
                        };
                        assert_eq!(&closure.id.owner, &function.id);
                        assert_eq!(&body.id, &body_id);
                        assert_eq!(closure.captures.len(), body.closure_captures.len());
                        eprintln!("INTEGRATION_BORROW_TRACE actualClosureSource statementSpan={:?} closureId={:?} body={body_id:?}", statement.span, closure.id);
                        for (source, stored_capture) in closure.captures.iter().zip(&body.closure_captures) {
                            assert_eq!(source.name, stored_capture.name, "actual source/body capture binding");
                            let stored = &body.local_decls[stored_capture.local.0];
                            let source_decl = &function.local_decls[source.local.0];
                            eprintln!("INTEGRATION_BORROW_TRACE capture source={source:?} sourceDecl={source_decl:?} storedCapture={stored_capture:?} storedSlot={stored:?}; kind is real storage plan, not declared Fn metadata");
                            integration_type(&mut trace, &program.type_context, &program.backend_contract, stored.ty);
                            let storage_events = function.basic_blocks.iter().enumerate().flat_map(|(block, data)| data.statements.iter().enumerate().map(move |(index, stmt)| (block, index, stmt)))
                                .filter(|(_, _, stmt)| matches!(stmt.kind, StatementKind::StorageLive(local) | StatementKind::StorageDead(local) if local == source.local))
                                .map(|(block, index, stmt)| format!("block={block} index={index} kind={:?} span={:?}", stmt.kind, stmt.span)).collect::<Vec<_>>();
                            assert!(storage_events.len() <= 128, "INTEGRATION_BORROW_TRACE stack lifetime event budget overflow");
                            eprintln!("INTEGRATION_BORROW_TRACE capture-stack-lifetime sourceLocal={:?} events={storage_events:?}", source.local);
                        }
                        integration_header(&mut trace, program, &body_id, lifetime, values);
                    }
                }
            }
            if matches!(block.terminator, Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })) {
                let root = Place { local: Local(0), projection: Vec::new() };
                let path = integration_path(&mut trace, &analysis, &root);
                let origins = integration_origins(&mut trace, function, &root, &analysis.origins_for_place(&results.exit_sets[block_index], &root));
                integration_record(&mut trace, format!("{id:?}|replay-return|{block_index}"), format!("actual-return root={path} queryOrigins={origins} materializedPaths={exit_rows:?} activeLoansAtReturn={:?}; empty map is NOT a contained-value proof", loan_summary(&active_loans)));
            }
            if let Some(term @ Terminator::Call { destination, func, args, target, .. }) = &block.terminator {
                if locals.contains(&destination.local) {
                    let result = analysis.call_return_origins(&state, func, args, destination);
                    let receiver_aliases = receiver_calls.get(&block_index);
                    // Unit/empty-result mutation candidates remain observable:
                    // result absence says nothing about receiver/payload storage.
                    if result.is_empty() && receiver_aliases.is_none() { continue; }
                    integration_source_span(&mut trace, "sourceStatementSpan", function.terminator_origin(term).source_span());
                    integration_call_authority(&mut trace, program, func, lifetime, values);
                    if receiver_aliases.is_some() {
                        integration_bound_call_authority(&mut trace, program, function, func, lifetime, values);
                    }
                    let mut places = vec![destination.clone()];
                    if let Operand::Copy(place) | Operand::Move(place) = func { places.push(place.clone()); }
                    for arg in args { if let Operand::Copy(place) | Operand::Move(place) = arg { places.push(place.clone()); } }
                    let paths = places.iter().map(|place| {
                        let origins = analysis.origins_for_place(&state, place);
                        let loans = table.iter().filter(|loan| active_loans.owner_contains(loan.id, place.local)
                            || loan.initial_owner == place.local).map(|loan| format!("loan={loan:?} activeOwner={} indexedSource={:?}", active_loans.owner_contains(loan.id, place.local), table.place_paths().place(loan.place_path))).collect::<Vec<_>>();
                        format!("place={place:?} {} actualOrigins={} actualCarrierLoans={loans:?}", integration_path(&mut trace, &analysis, place), integration_origins(&mut trace, function, place, &origins))
                    }).collect::<Vec<_>>();
                    let before = integration_state(&mut trace, &analysis, &state, &locals);
                    let envelope = integration_origins(&mut trace, function, destination, &result);
                    analysis.apply_mutation_origins(&mut state, bindings.get(&block_index).map(Vec::as_slice).unwrap_or_default());
                    analysis.apply_terminator(&mut state, term);
                    let after = integration_state(&mut trace, &analysis, &state, &locals);
                    let successor_loans = loan_summary(&entries[target.0]);
                    let location = location::Location::new(crate::mir::BasicBlockId(block_index), location::StatementIndex(block.statements.len()));
                    let created = locations.loan_ids_at(location).collect::<Vec<_>>();
                    integration_record(&mut trace, format!("{id:?}|replay-call|{block_index}"), format!("sourceStatementSpan={:?} call={term:?} receiverAliasEvidence={receiver_aliases:?} checkedPaths={paths:?} actualResultEnvelope={envelope} BEFORE={before:?} AFTER={after:?} actualLoansBeforeTerminator={:?} actualTerminatorCreatedLoans={created:?} actualSuccessorJoinedLoans={successor_loans:?}; successor is CFG must/may analysis, not a fabricated receiver effect", function.terminator_origin(term).source_span(), loan_summary(&active_loans)));
                }
            }
        }
        replayed.insert(id, locals);
    }
    if integration_effects_detail() {
        let mut trace = integration_trace().lock().unwrap();
        // Requests own only bounded identities/alias strings; bodies come from
        // this exact MirProgram, synchronously, without saved borrow lifetimes.
        let requests = std::mem::take(&mut trace.detail_bodies);
        if matches!(trace.case.as_str(), "vec_reference_payload_cannot_escape_local"
            | "vec_borrowed_callable_payload_cannot_escape_local") {
            assert_eq!(requests.len(), 1, "detail observation must find the actual single typed Unit receiver/payload native; never guess a missing target");
        }
        for (id, request) in requests {
            integration_detail_body(&mut trace, program, &id, &request, lifetime, values);
        }
    }
}

// Python's standard JSON decoder recovers the log's escaped DiagnosticSource
// string without manually replacing escapes (notably UTF-8 cursor bytes).
fn integration_log_source(log: &std::path::Path, name: &str) -> String {
    let script = r#"import json,re,sys
s=open(sys.argv[1],encoding='utf-8').read()
b=s.split('---- '+sys.argv[2]+' stdout ----',1)[1].split('---- ',1)[0]
m=re.search(r'DiagnosticSource \{ display_path: .*?, text: ',b)
assert m, 'missing archived DiagnosticSource'
sys.stdout.buffer.write(json.JSONDecoder().raw_decode(b[m.end():])[0].encode('utf-8'))
"#;
    let output = std::process::Command::new("python3").args(["-c", script]).arg(log).arg(name).output().unwrap();
    assert!(output.status.success(), "faithful archived JSON source recovery failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

pub(super) fn integration_literal(file: &std::path::Path, marker: &str) -> String {
    let input = std::fs::read_to_string(file).unwrap();
    let tail = input.split_once(marker).expect("actual fixture marker").1;
    let literal = tail.split_once("r#\"").expect("actual raw source literal").1;
    literal.split_once("\"#").expect("actual raw source terminator").0.to_string()
}

pub(super) fn integration_sha(path: &std::path::Path) -> String {
    let output = std::process::Command::new("sha256sum").arg(path).output().unwrap();
    assert!(output.status.success(), "input hash command failed");
    String::from_utf8(output.stdout).unwrap().split_whitespace().next().unwrap().to_string()
}

fn integration_pipeline(config: &crate::Config) -> Result<(), Diagnostics> {
    // This is compile_impl's actual frontend -> identity -> mono -> DCE -> MIR
    // -> Borrow -> RuntimeAgreement boundary; intentionally stop before LLVM.
    let frontend = crate::compile_frontend(config)?;
    let fingerprint = crate::product_source_fingerprint(config, &frontend.source_db, &frontend.fingerprint_source_files);
    let identity = crate::products::ProductCrateIdentity::local(frontend.effective_current_crate_name.clone())
        .with_source_fingerprint(fingerprint.clone());
    let context = crate::mono::CompilationIdentityContext::new(frontend.analysis.hir.root_crate_id, identity,
        frontend.crate_ctx.product_crate_ids.iter().map(|(identity, crate_id)| (*crate_id, identity.clone())).collect());
    if integration_effects_detail() && !integration_recording() {
        eprintln!("INTEGRATION_BORROW_TRACE actual-product-fingerprint={fingerprint:?} loadedPreludeExportCount={}; detail-positive source/diagnostic-only observation", frontend.loaded_prelude_export_ids.len());
    } else {
        eprintln!("INTEGRATION_BORROW_TRACE actual-product-fingerprint={fingerprint:?} imported-product-identities={:?} loadedPreludeExportCount={}", frontend.crate_ctx.product_crate_ids, frontend.loaded_prelude_export_ids.len());
    }
    let mut mono = crate::mono::monomorphize_with_crates(frontend.analysis.hir, &frontend.crate_ctx, context)
        .map_err(|diagnostics| diagnostics.with_sources(&frontend.diagnostic_sources))?;
    let mut bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut mono);
    let _report = crate::dce::prune_unreachable_instances(&mut mono, &mut bodies);
    let program = crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies);
    let outcome = BorrowChecker::run(&program).map_err(|diagnostics| diagnostics.with_sources(&frontend.diagnostic_sources));
    let agreement = crate::mir::agreement::check_mir_runtime_agreement(&program);
    if integration_effects_detail() && !integration_recording() {
        eprintln!("INTEGRATION_BORROW_TRACE actual-RuntimeAgreement clean={}; detail-positive source/diagnostic-only observation; no LLVM/codegen/link/native execution", agreement.is_clean());
    } else {
        eprintln!("INTEGRATION_BORROW_TRACE actual-RuntimeAgreement clean={} evidence={agreement:?}; no LLVM/codegen/link/native execution", agreement.is_clean());
    }
    // Keep the actual Borrow diagnostics even if agreement independently fails.
    if !agreement.is_clean() {
        let mut diagnostics = outcome.err().unwrap_or_default();
        diagnostics.push(Diagnostic::for_internal(format!("Observer actual MIR RuntimeAgreement failed: {agreement:?}")));
        return Err(diagnostics);
    }
    outcome
}

#[test]
fn custom_owner_return_eligibility_rejects_actual_local_answer() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixture = workspace.join("lib/tests/owned_object_calls.rs");
    let original = std::fs::read(&fixture).unwrap();
    let source = format!("{}{}{}\n{}",
        integration_literal(&fixture, "const OWNER:"),
        integration_literal(&fixture, "const UNIQUE:"),
        integration_literal(&fixture, "const BORROWED_PAYLOAD:"),
        integration_literal(&fixture, "fn custom_owner_cannot_return_an_erased_payload_borrowing_a_local()"));
    let entry = agent_work_path("fixtures/custom_owner_return_eligibility/main.rk");
    let config = crate::Config {
        entry_file: entry.clone(), output_dir: entry.parent().unwrap().into(),
        current_crate_name: Some("owned_calls".into()),
        no_std: true, no_prelude: true, no_link: true,
        source_providers: vec![crate::SourceProvider::Virtual { path: entry.clone(), text: source.clone() }],
        ..Default::default()
    };
    let frontend = crate::compile_frontend(&config).expect("unchanged owner source passes frontend authority");
    let fingerprint = crate::product_source_fingerprint(&config, &frontend.source_db, &frontend.fingerprint_source_files);
    let identity = crate::products::ProductCrateIdentity::local(frontend.effective_current_crate_name.clone())
        .with_source_fingerprint(fingerprint);
    let context = crate::mono::CompilationIdentityContext::new(frontend.analysis.hir.root_crate_id, identity,
        frontend.crate_ctx.product_crate_ids.iter().map(|(identity, crate_id)| (*crate_id, identity.clone())).collect());
    let mut mono = crate::mono::monomorphize_with_crates(frontend.analysis.hir, &frontend.crate_ctx, context).unwrap();
    let mut bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut mono);
    crate::dce::prune_unreachable_instances(&mut mono, &mut bodies);
    let program = crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies);
    let agreement = crate::mir::agreement::check_mir_runtime_agreement(&program);
    assert!(agreement.is_clean(), "{agreement:?}");
    assert!(crate::mir::validate_erased_contract(&program).is_empty());
    let id = MirFunctionId::Instance(crate::ids::InstanceId(10));
    let function = &program.functions[&id];
    let key = &program.backend_contract.function_bodies[&id];
    let header = program.backend_contract.callable(key).unwrap();
    assert_eq!(&header.key, key);
    assert_eq!(header.source_def_id, Some(DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(19))));
    assert!(matches!(&header.kind, crate::mir::MirCallableKind::LocalBody { function_id } if function_id == &id));
    let root = Place { local: Local(0), projection: Vec::new() };
    let ty = program.backend_contract.place_type_id(&program.type_context, function, &root).unwrap();
    assert_eq!(ty, TypeId(1));
    assert_eq!(ty, function.local_decls[0].ty);
    assert_eq!(header.signature.ret.semantic_ty, ty);
    assert_eq!(header.signature.ret.abi_ty, ty);
    assert!(matches!(program.type_context.type_for(ty), Type::Struct { id, .. }
        if id == DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(7))));
    assert!(return_type_needs_escape_check(&program.type_context, &program.backend_contract, ty));
    assert_eq!(&source[1928..1934], "answer");
    let answer_span = Span::new(entry, 1928, 1934);
    assert_eq!(function.local_decls[2].span.as_ref(), Some(&answer_span));
    assert_eq!(function.local_decls[2].source, LocalSource::UserBinding);
    let lifetime = BorrowChecker::reference_return_summaries(&program);
    assert!(lifetime[&id].contains(&ReferenceOrigin::Local(Local(2))));
    let analysis = ReferenceEscapeAnalysis::new(function, &program.type_context, &program.backend_contract,
        &lifetime, MissingSummaryMode::Conservative);
    let results = run_fixpoint(&analysis, function);
    assert!(function.basic_blocks.iter().enumerate().any(|(block, data)| {
        results.reachable[block]
            && matches!(data.terminator, Some(Terminator::Return | Terminator::ReturnWithOrigin { .. }))
            && results.exit_sets[block].origins.get(&root)
                .is_some_and(|origins| origins.contains(&ReferenceOrigin::Local(Local(2))))
    }));
    // Preserve already-selected consuming/native authority; no new admission.
    assert!(!program.backend_contract.owned_object_calls.is_empty());
    for plan in program.backend_contract.owned_object_calls.values() {
        assert!(program.backend_contract.callable(&plan.into_parts).is_some());
        assert!(program.backend_contract.callable(&plan.release).is_some());
    }
    let diagnostics = BorrowChecker::run(&program).unwrap_err();
    assert!(diagnostics.0.iter().all(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)),
        "no other-stage error may substitute for the owning-return negative: {diagnostics:?}");
    assert!(diagnostics.0.iter().any(|diagnostic| {
        diagnostic.code == Some(DiagnosticCode::Borrow)
            && diagnostic.message == "Cannot return reference to local value because it does not live long enough"
            && diagnostic.primary.as_ref().is_some_and(|label| label.span == answer_span)
    }), "must reject the actual returned Local(answer), not an incidental Borrow: {diagnostics:?}");
    assert_eq!(std::fs::read(&fixture).unwrap(), original);
}

pub(super) fn agent_work_path(relative: &str) -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().join(".agent-work").join(relative)
}

pub(super) fn current_stdlib_artifact() -> PathBuf {
    // This fixture exercises only the current artifact format.
    let artifact = agent_work_path("artifacts/stdlib60/contracts-2/stdlib.rkca");
    let object = artifact.with_file_name("stdlib.o");
    assert_eq!(std::fs::metadata(&artifact).unwrap().len(), 967266);
    assert_eq!(integration_sha(&artifact), "b808e0c547714ac13a492e1b9a2c0dec86bcc8c30f639971eadfb6e5ff4f0de4");
    assert_eq!(std::fs::metadata(&object).unwrap().len(), 174424);
    assert_eq!(integration_sha(&object), "08fb9f976055f987ee9c639981685c56d90ecb1245ac5732e0e1dd17f142b128");
    eprintln!("CURRENT_STDLIB artifact={artifact:?} schema=60 bytes=967266 sha=b808e0c547714ac13a492e1b9a2c0dec86bcc8c30f639971eadfb6e5ff4f0de4 producerPaths=absolute");
    artifact
}

pub(super) fn mutation_source_program(name: &str, source: &str, stdlib: bool) -> (MirProgram, PathBuf) {
    let (program, entry, ()) = mutation_source_program_with_callback(name, source, stdlib, |_, _, _| ());
    (program, entry)
}

pub(super) fn mutation_source_program_with_callback<R>(
    name: &str,
    source: &str,
    stdlib: bool,
    callback: impl FnOnce(
        &crate::mono::MonomorphizedProgram,
        &crate::mir::MirInstanceBodies,
        &MirProgram,
    ) -> R,
) -> (MirProgram, PathBuf, R) {
    mutation_source_program_with_phase_callbacks(name, source, stdlib, |_| (), callback)
}

pub(super) fn mutation_source_program_with_phase_callbacks<R>(
    name: &str,
    source: &str,
    stdlib: bool,
    before_mir: impl FnOnce(&crate::mono::MonomorphizedProgram),
    callback: impl FnOnce(
        &crate::mono::MonomorphizedProgram,
        &crate::mir::MirInstanceBodies,
        &MirProgram,
    ) -> R,
) -> (MirProgram, PathBuf, R) {
    let directory = agent_work_path("fixtures").join(format!("rock_mutation_{}_{}", std::process::id(), name));
    std::fs::create_dir_all(directory.parent().unwrap()).unwrap();
    std::fs::create_dir(&directory).unwrap();
    let entry = directory.join("main.rk");
    std::fs::write(&entry, source).unwrap();
    let artifact = stdlib.then(current_stdlib_artifact);
    let bytes = artifact.as_ref().map(|path| std::fs::read(path).unwrap());
    let config = crate::Config {
        entry_file: entry.clone(), output_dir: directory,
        no_std: !stdlib, no_prelude: !stdlib, no_link: true,
        extern_artifacts: artifact.as_ref().map(|path| vec![("stdlib".into(), path.clone())]).unwrap_or_default(),
        ..Default::default()
    };
    let frontend = crate::compile_frontend(&config).expect("source control must pass frontend, not reject incidentally");
    let fingerprint = crate::product_source_fingerprint(&config, &frontend.source_db, &frontend.fingerprint_source_files);
    let identity = crate::products::ProductCrateIdentity::local(frontend.effective_current_crate_name.clone())
        .with_source_fingerprint(fingerprint);
    let context = crate::mono::CompilationIdentityContext::new(frontend.analysis.hir.root_crate_id, identity,
        frontend.crate_ctx.product_crate_ids.iter().map(|(identity, crate_id)| (*crate_id, identity.clone())).collect());
    let mut mono = crate::mono::monomorphize_with_crates(frontend.analysis.hir, &frontend.crate_ctx, context).unwrap();
    before_mir(&mono);
    let mut bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut mono);
    crate::dce::prune_unreachable_instances(&mut mono, &mut bodies);
    let program = crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies);
    let agreement = crate::mir::agreement::check_mir_runtime_agreement(&program);
    assert!(agreement.is_clean(), "RuntimeAgreement must be clean BEFORE Borrow: {agreement:?}");
    assert!(crate::mir::validate_erased_contract(&program).is_empty());
    let outcome = callback(&mono, &bodies, &program);
    assert_eq!(std::fs::read(&entry).unwrap(), source.as_bytes());
    if let Some(bytes) = bytes { assert_eq!(std::fs::read(artifact.unwrap()).unwrap(), bytes); }
    (program, entry, outcome)
}

fn strict_vector_mutation_return(case: &str, hash: &str, range: (usize, usize)) {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/mir/borrowck/allocator_trace_tests.rs");
    let source = integration_literal(&fixture, &format!("        (\"{case}\","));
    let (program, entry) = mutation_source_program(case, &source, true);
    assert_eq!(integration_sha(&entry), hash, "same unchanged observer input, no annotations/workaround");
    let origin_span = Span::new(entry.clone(), range.0, range.1);
    let (_, function) = program.functions().find(|(_, function)| function.local_decls.iter()
        .any(|local| local.source == LocalSource::UserBinding && local.span.as_ref() == Some(&origin_span))).expect("actual original source owner");
    let origin = Local(function.local_decls.iter().position(|local|
        local.source == LocalSource::UserBinding && local.span.as_ref() == Some(&origin_span)).unwrap());
    let lifetime = BorrowChecker::reference_return_summaries(&program);
    let values = BorrowChecker::native_return_loan_proof(&program);
    let mutations = NativeMutationTable::for_program(&program);
    let tc = &program.type_context;
    let contract = &program.backend_contract;
    let candidates = function.basic_blocks.iter().map(|block| block.terminator.as_ref()
        .map(|term| native_mutation_candidates(function, tc, contract, &mutations, term)).unwrap_or_default()).collect::<Vec<_>>();
    let selected = function.basic_blocks.iter().zip(&candidates).enumerate().find(|(_, (_, rows))|
        rows.iter().any(|row| row.destination.argument == 0 && row.sources.iter().any(|source|
            source.argument == 1 && source.kind == crate::hir::borrowing::BorrowSourceKind::Value)))
        .expect("actual typed mutable receiver/payload call").0;
    let term = function.basic_blocks[selected].terminator.as_ref().unwrap();
    let Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))), .. } = term else { panic!("native binding"); };
    assert_eq!(mutations.knowledge(key), NativeMutationKnowledge::Unknown,
        "observed raw backing stores and indirect allocation remain Unknown, not a fabricated backing seal");
    let header = contract.callable(key).unwrap();
    let crate::mir::MirCallableKind::LocalBody { function_id } = &header.kind else { panic!("selected current body"); };
    assert_eq!(contract.function_bodies.get(function_id), Some(key));
    assert!(program.functions().any(|(id, _)| id == function_id));
    let borrows = collect_function_borrows_with_contract(function, tc, contract, &values.summaries);
    let table = LoanTable::from_borrows_with_paths(&borrows, paths::PlacePathTable::for_function(function));
    let locations = LoanLocationIndex::for_function(&table, function);
    let live = liveness::compute_reference_liveness_with_mutations(function, tc, contract, &table, &candidates);
    let entries = liveness::compute_active_loan_entries_with_mutations(function, tc, contract, &table, &locations, &live, &values.summaries, &candidates);
    let analysis = liveness::ActiveLoanAnalysis::new(tc, contract, &table, &locations, &live, &values.summaries).with_mutations(&candidates);
    let mut call_state = entries[selected].clone();
    analysis.apply_statements(&mut call_state, function, crate::mir::BasicBlockId(selected));
    let bindings = native_mutation_bindings(function, tc, contract, &table, &call_state, term, &candidates[selected]);
    assert!(!bindings.is_empty());
    let source_ids = table.iter().filter(|loan| table.place_paths().place(loan.place_path)
        .is_some_and(|place| place.local == origin)).map(|loan| loan.id).collect::<HashSet<_>>();
    assert!(!source_ids.is_empty(), "real source/capture loans, not invented IDs");
    liveness::apply_active_loan_terminator_transfer(&mut call_state, function, tc, contract, term,
        &values.summaries, Some((&table, &candidates[selected])));
    for binding in &bindings {
        assert!(source_ids.iter().any(|id| call_state.owner_contains(*id, binding.owner.local)));
        assert!(binding.excluded.iter().all(|id| !call_state.owner_contains(*id, binding.owner.local)),
            "receiver storage reborrow must not be installed into its owner");
    }
    let (_, origin_bindings) = BorrowChecker::validate_moves_and_initialization(function, tc, contract, &values.summaries, &mutations);
    let mut escape = ReferenceEscapeAnalysis::new(function, tc, contract, &lifetime, MissingSummaryMode::Conservative);
    escape.mutation_origins = Some(&origin_bindings);
    let origins = run_fixpoint(&escape, function);
    let root = Place { local: Local(0), projection: Vec::new() };
    let mut saw_return = false;
    for (block, data) in function.basic_blocks.iter().enumerate() {
        if origins.reachable[block] && matches!(data.terminator, Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })) {
            saw_return = true;
            assert!(origins.exit_sets[block].origins.get(&root).is_some_and(|set| set.contains(&ReferenceOrigin::Local(origin))),
                "original source origin must survive actual owner -> return move");
            let mut state = entries[block].clone();
            analysis.apply_statements(&mut state, function, crate::mir::BasicBlockId(block));
            assert!(source_ids.iter().any(|id| state.owner_contains(*id, Local(0))),
                "payload/capture loan must survive the real return move, not just call staging");
        }
    }
    assert!(saw_return);
    let diagnostics = BorrowChecker::run(&program).expect_err("owning return must reject without executing native code");
    assert!(diagnostics.0.iter().all(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)));
    assert!(diagnostics.0.iter().any(|diagnostic|
        diagnostic.message == "Cannot return reference to local value because it does not live long enough"
            && diagnostic.primary.as_ref().is_some_and(|label| label.span == origin_span)),
        "not an incidental assignment, capture seal or any-error oracle: {diagnostics:?}");
}

#[test]
fn native_mutation_vec_reference_rejects_actual_owning_return() {
    strict_vector_mutation_return("vec_reference_payload_cannot_escape_local",
        "d9c06a955035df92d0f7ddfa4230ab58411be5fdb0c7ed312e8e76312103f174", (40, 46));
}

#[test]
fn native_mutation_vec_callable_rejects_actual_owning_return() {
    strict_vector_mutation_return("vec_borrowed_callable_payload_cannot_escape_local",
        "a8fe3a5aa3aa37a4786f06aa99cba9d595828769e1c95290e7694eabf2290ab7", (63, 67));
}

const MUTATION_SLOT: &str = r#"
struct Slot
    < value: &I64
store: &mut Slot -> &I64 -> ()
store = slot, value ->
    slot.value = value
    ()
"#;

#[test]
fn native_mutation_caller_write_rejects_live_retained_borrow() {
    let source = format!("{MUTATION_SLOT}{}", r#"
main = !->
    initial = 0
    mut answer = 42
    mut slot = Slot
        value: &initial
    store &mut slot, &answer
    answer = 7
    *slot.value
"#);
    let (program, entry) = mutation_source_program("live_write", &source, false);
    let diagnostics = BorrowChecker::run(&program).expect_err("Unit call must retain the actual live payload loan");
    let start = source.find("answer = 7").unwrap();
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
        && diagnostic.message == "borrow conflict on this value"
        && diagnostic.primary.as_ref().is_some_and(|label| label.span.file_path == entry
            && label.span.start >= start && label.span.end <= start + "answer = 7".len())), "{diagnostics:?}");
}

#[test]
fn native_mutation_owner_move_and_drop_release_payload_borrow() {
    let source = r#"
release: Vec &I64 -> ()
release = values -> ()
main = !->
    mut answer = 42
    mut values: Vec &I64 = Vec::new!
    values.push &answer
    moved = values
    release moved
    answer = 7
    answer.println!
"#;
    let (program, _) = mutation_source_program("move_drop", source, true);
    BorrowChecker::run(&program).expect("real moved/dropped owner releases its contained borrow");
}

#[test]
fn native_mutation_safe_by_value_callable_has_no_fabricated_origin() {
    let source = r#"
escape: () -> Vec (() -> I64)
escape = ->
    callback = -> 42
    mut values: Vec (() -> I64) = Vec::new!
    values.push callback
    values
main = !->
    values = escape!
    values.len!.println!
"#;
    let (program, _) = mutation_source_program("by_value_fn", source, true);
    BorrowChecker::run(&program).expect("empty actual dependency evidence must not fabricate a Local or capture seal");
}

#[test]
fn native_mutation_scalar_storage_screen_is_not_empty_body_proof() {
    let source = r#"
touch: &mut I64 -> &I64 -> ()
touch = target, source ->
    *target = 7
    ()
main = !->
    mut target = 0
    source = 1
    touch &mut target, &source
    target = 8
    target
"#;
    let (program, _) = mutation_source_program("scalar_screen", source, false);
    let table = NativeMutationTable::for_program(&program);
    let (_, main) = program.functions().find(|(_, function)| function.name == "main").unwrap();
    let term = main.basic_blocks.iter().filter_map(|block| block.terminator.as_ref()).find(|term|
        matches!(term, Terminator::Call { args, .. } if args.len() == 2)).unwrap();
    let Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))), .. } = term else { panic!("native key"); };
    assert_eq!(table.knowledge(key), NativeMutationKnowledge::Unknown);
    assert!(native_mutation_candidates(main, &program.type_context, &program.backend_contract, &table, term).is_empty());
    BorrowChecker::run(&program).expect("scalar pointee cannot contain the reference value; no no-effects proof claimed");
}

#[test]
fn native_mutation_completed_no_call_local_write_body_is_nonretaining() {
    let source = r#"
struct Slot
    < value: &I64
read_only: &mut Slot -> &I64 -> ()
read_only = slot, source -> ()
main = !->
    initial = 0
    mut answer = 42
    mut slot = Slot
        value: &initial
    read_only &mut slot, &answer
    answer = 7
    *slot.value
"#;
    let (program, _) = mutation_source_program("completed_empty", source, false);
    let table = NativeMutationTable::for_program(&program);
    let (_, main) = program.functions().find(|(_, function)| function.name == "main").unwrap();
    let term = main.basic_blocks.iter().filter_map(|block| block.terminator.as_ref()).find(|term|
        matches!(term, Terminator::Call { args, .. } if args.len() == 2)).unwrap();
    let Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))), .. } = term else { panic!("native key"); };
    assert_eq!(table.knowledge(key), NativeMutationKnowledge::CompletedNoEffects);
    assert!(!native_mutation_candidates(main, &program.type_context, &program.backend_contract, &NativeMutationTable::default(), term).is_empty(),
        "Unknown would conservatively retain here; completion, not screening, discharges it");
    assert!(native_mutation_candidates(main, &program.type_context, &program.backend_contract, &table, term).is_empty());
    BorrowChecker::run(&program).expect("actual complete no-call/local-write body does not retain source");
}

#[test]
fn native_mutation_alias_join_resolves_both_storage_alternatives() {
    let source = format!("{MUTATION_SLOT}{}", r#"
main = !->
    initial = 0
    mut answer = 42
    mut left = Slot
        value: &initial
    mut right = Slot
        value: &initial
    chosen = if ~I64Eq 0, 1
        &mut left
    else
        &mut right
    store chosen, &answer
    answer = 7
    *left.value
"#);
    let (program, entry) = mutation_source_program("alias_join", &source, false);
    let (_, main) = program.functions().find(|(_, function)| function.name == "main").unwrap();
    let values = BorrowChecker::native_return_loan_proof(&program);
    let (_, bindings) = BorrowChecker::validate_moves_and_initialization(main, &program.type_context,
        &program.backend_contract, &values.summaries, &NativeMutationTable::for_program(&program));
    let owners = bindings.values().flatten().map(|binding|
        main.local_decls[binding.owner.local.0].name.as_deref()).collect::<HashSet<_>>();
    assert!(owners.contains(&Some("left")) && owners.contains(&Some("right")), "must retain all alias alternatives: {owners:?}");
    let diagnostics = BorrowChecker::run(&program).expect_err("possible retention into left keeps the source loan live");
    let start = source.find("answer = 7").unwrap();
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
        && diagnostic.message == "borrow conflict on this value"
        && diagnostic.primary.as_ref().is_some_and(|label| label.span.file_path == entry
            && label.span.start >= start && label.span.end <= start + "answer = 7".len())), "{diagnostics:?}");
}

// Six ordinary source controls; no new observer mode or production authority.
const STRUCTURED_CALLABLE_STORAGE: &str = r#"
struct Holder
    < callback: () -> I64
struct Cell T
    < payload: T
"#;

fn structured_callable_source(
    producer: &str,
    transport: Option<crate::hir::borrowing::BorrowSourceKind>,
    release_address: bool,
) -> String {
    use crate::hir::borrowing::BorrowSourceKind;
    let (store, result, tail) = match transport {
        None => ("", "Holder", "    staged\n".to_string()),
        Some(BorrowSourceKind::Value) => (
            "store: &mut (Cell Holder) -> Holder -> ()\nstore = dest, source ->\n    dest.payload = source\n    ()\n",
            "Cell Holder",
            "    mut cell = Cell\n        payload: Holder\n            callback: -> 0\n    store &mut cell, staged\n    cell\n".to_string(),
        ),
        Some(BorrowSourceKind::Contents) => (
            // Holder itself is non-Copy. Copy only the checked Function field,
            // not a whole nominal value out of shared-reference storage.
            "store: &mut (Cell Holder) -> &Holder -> ()\nstore = dest, source ->\n    dest.payload = Holder\n        callback: source.callback\n    ()\n",
            "Cell Holder",
            format!("    mut cell = Cell\n        payload: Holder\n            callback: -> 0\n    source_view = &staged\n    store &mut cell, source_view\n{}    cell\n",
                if release_address { "    staged = Holder\n        callback: -> 1\n" } else { "" }),
        ),
    };
    let scalar_method = if producer.contains("into_value") {
        "< trait ScalarValue\n    ~@into_value: I64\nimpl ScalarValue for I64\n    ~@into_value = -> self\n"
    } else { "" };
    format!("{STRUCTURED_CALLABLE_STORAGE}{scalar_method}{store}escape: () -> {result}\nescape = ->\n{producer}    holder = Holder\n        callback: callback\n    mut staged = holder\n{tail}main = !->\n    result = escape!\n    ()\n")
}

fn structured_callable_control(
    case: &str,
    producer: &str,
    transport: Option<crate::hir::borrowing::BorrowSourceKind>,
    capture_kind: Option<crate::mir::MirClosureCaptureKind>,
) {
    use crate::hir::borrowing::BorrowSourceKind;
    use crate::mir::MirClosureCaptureKind;

    let negative = capture_kind == Some(MirClosureCaptureKind::ByRef);
    let source = structured_callable_source(producer, transport,
        transport == Some(BorrowSourceKind::Contents) && !negative);
    let (program, entry) = mutation_source_program(case, &source, false);
    // Print actual admission before any additional capture/dataflow assertions.
    let outcome = BorrowChecker::run(&program);
    eprintln!("STRUCTURED_CALLABLE case={case} BorrowOutcome={} execution=NONE",
        if outcome.is_ok() { "ACCEPTED" } else { "REJECTED" });
    if let Err(diagnostics) = &outcome {
        assert!(diagnostics.0.len() <= 32, "structured diagnostic cap32 exceeded");
        for diagnostic in &diagnostics.0 {
            eprintln!("STRUCTURED_CALLABLE case={case} code={:?} message={} primary={:?}",
                diagnostic.code, diagnostic.message, diagnostic.primary.as_ref().map(|label| &label.span));
        }
    }
    assert!(program.functions().count() <= 8, "structured body-function cap8 exceeded");
    let holder_start = source.find("    holder = Holder").unwrap() + 4;
    let holder_span = Span::new(entry.clone(), holder_start, holder_start + "holder".len());
    let (_, function) = program.functions().find(|(_, function)| function.local_decls.iter()
        .any(|local| local.span.as_ref() == Some(&holder_span))).expect("actual source holder owner");
    let holder = Local(function.local_decls.iter().position(|local|
        local.span.as_ref() == Some(&holder_span)).unwrap());
    let callback_start = source.find("    callback =").unwrap() + 4;
    let callback_span = Span::new(entry.clone(), callback_start, callback_start + "callback".len());
    let callback = Local(function.local_decls.iter().position(|local|
        local.span.as_ref() == Some(&callback_span)).expect("actual source callback binding"));
    let staged_start = source.find("    mut staged = holder").unwrap() + "    mut ".len();
    let staged_span = Span::new(entry.clone(), staged_start, staged_start + "staged".len());
    let staged = Local(function.local_decls.iter().position(|local|
        local.span.as_ref() == Some(&staged_span)).expect("actual source staging local"));
    let tc = &program.type_context;
    let contract = &program.backend_contract;
    let holder_type = contract.normalize_type(tc, &tc.type_for(function.local_decls[holder.0].ty));
    let Type::Struct { id: holder_id, .. } = &holder_type else { panic!("declared Holder storage"); };
    let fields = contract_struct_fields(contract, *holder_id).unwrap();
    assert_eq!(fields.len(), 1);
    let Type::Function { params, ret, captures, .. } = tc.type_for(fields[0].1) else { panic!("actual declared Function field"); };
    assert!(params.is_empty() && ret.as_ref() == &Type::I64 && captures.is_empty(),
        "abstract field shape is intentional, not a capture-free environment proof");
    let (closure_destination, closure) = function.basic_blocks.iter().flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            StatementKind::Assign(dest, Rvalue::Closure(closure)) => Some((dest, closure)),
            _ => None,
        }).expect("first source callback producer, before holder/initial-cell construction");
    let body = &program.functions[&MirFunctionId::Closure(Box::new(closure.id.clone()))];
    assert_eq!(closure.id.owner, function.id);
    assert_eq!(closure.captures.len(), usize::from(capture_kind.is_some()));
    assert_eq!(body.closure_captures.len(), closure.captures.len());
    let expected_span = capture_kind.map(|kind| {
        let name = if kind == MirClosureCaptureKind::ByRef { "view" } else { "scalar" };
        let start = source.find(&format!("    {name} =")).unwrap() + 4;
        Span::new(entry.clone(), start, start + name.len())
    });
    let origin = closure.captures.first().map(|capture| {
        assert_eq!(Some(capture.kind), capture_kind);
        assert_eq!(function.local_decls[capture.local.0].span.as_ref(), expected_span.as_ref());
        assert_eq!(capture.span.as_ref(), expected_span.as_ref());
        let stored = &body.closure_captures[0];
        assert_eq!(stored.kind, capture.kind);
        let ty = tc.type_for(function.local_decls[capture.local.0].ty);
        let stored_ty = tc.type_for(body.local_decls[stored.local.0].ty);
        if negative {
            assert_eq!(ty, Type::Reference { mutable: false, inner: Box::new(Type::I64) });
            assert_eq!(stored_ty, Type::Reference { mutable: false, inner: Box::new(ty) });
        } else {
            assert_eq!(ty, Type::I64);
            assert_eq!(stored_ty, Type::I64);
        }
        capture.local
    });
    eprintln!("STRUCTURED_CALLABLE case={case} holder={holder:?} staged={staged:?} closure={:?} producer={closure_destination:?} actualCapture={capture_kind:?} span={expected_span:?} declaredFieldCaptures=0",
        closure.id);

    let lifetime = BorrowChecker::reference_return_summaries(&program);
    let values = BorrowChecker::native_return_loan_proof(&program);
    let mutations = NativeMutationTable::for_program(&program);
    let candidates = function.basic_blocks.iter().map(|block| block.terminator.as_ref()
        .map(|term| native_mutation_candidates(function, tc, contract, &mutations, term)).unwrap_or_default()).collect::<Vec<_>>();
    let borrows = collect_function_borrows_with_contract(function, tc, contract, &values.summaries);
    let table = LoanTable::from_borrows_with_paths(&borrows, paths::PlacePathTable::for_function(function));
    assert!(table.place_paths().iter_ids().count() <= 32, "structured place-path cap32 exceeded");
    let source_ids = table.iter().filter(|loan| negative && table.place_paths().place(loan.place_path)
        .is_some_and(|place| Some(place.local) == origin && place.projection.is_empty()))
        .map(|loan| loan.id).collect::<HashSet<_>>();
    assert!(source_ids.len() <= 8, "structured capture-loan cap8 exceeded");
    assert!(!negative || !source_ids.is_empty(), "actual source/capture LoanIds required");
    let address_ids = table.iter().filter(|loan| transport == Some(BorrowSourceKind::Contents)
        && loan.kind == LoanKind::Shared && table.place_paths().place(loan.place_path)
            .is_some_and(|place| place.local == staged && place.projection.is_empty()))
        .map(|loan| loan.id).collect::<HashSet<_>>();
    assert!(address_ids.len() <= 8, "structured address-loan cap8 exceeded");
    assert!(transport != Some(BorrowSourceKind::Contents) || !address_ids.is_empty());
    let locations = LoanLocationIndex::for_function(&table, function);
    let live = liveness::compute_reference_liveness_with_mutations(function, tc, contract, &table, &candidates);
    let entries = liveness::compute_active_loan_entries_with_mutations(function, tc, contract, &table, &locations, &live, &values.summaries, &candidates);
    let loans = liveness::ActiveLoanAnalysis::new(tc, contract, &table, &locations, &live, &values.summaries).with_mutations(&candidates);
    let mut transport_ok = true;
    if let Some(kind) = transport {
        let selected = candidates.iter().position(|rows| rows.iter().any(|row|
            row.destination.argument == 0 && row.destination.kind == BorrowSourceKind::Contents
                && row.sources.iter().any(|source| source.argument == 1 && source.kind == kind)))
            .expect("actual typed native destination0 Contents/source1 role");
        let term = function.basic_blocks[selected].terminator.as_ref().unwrap();
        let Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))), .. } = term else { panic!("actual native key"); };
        let header = contract.callable(key).unwrap();
        let crate::mir::MirCallableKind::LocalBody { function_id } = &header.kind else { panic!("actual native store body"); };
        assert_eq!(contract.function_bodies.get(function_id), Some(key));
        assert_eq!(header.signature.ret.semantic_ty, tc.id_for_type(&Type::Unit).unwrap());
        assert_eq!(mutations.knowledge(key), NativeMutationKnowledge::Unknown);
        assert!(program.functions[function_id].basic_blocks.iter().flat_map(|block| &block.statements)
            .any(|statement| matches!(&statement.kind, StatementKind::Assign(place, _) if !place.projection.is_empty())));
        if kind == BorrowSourceKind::Contents {
            assert!(!candidates[selected].iter().flat_map(|row| &row.sources)
                .any(|source| source.argument == 1 && source.kind == BorrowSourceKind::Value),
                "Cell<Holder> contents do not store the &Holder address");
        }
        let mut state = entries[selected].clone();
        loans.apply_statements(&mut state, function, crate::mir::BasicBlockId(selected));
        if kind == BorrowSourceKind::Contents {
            let Terminator::Call { args, .. } = term else { unreachable!(); };
            let (Operand::Copy(source) | Operand::Move(source)) = &args[1] else { panic!("actual borrowed Holder carrier"); };
            assert!(address_ids.iter().all(|id| state.is_active(*id)
                && state.storage_owner_contains(*id, source.local)),
                "actual source address loans must be live before Contents transport, not vacuously absent");
        }
        let bindings = native_mutation_bindings(function, tc, contract, &table, &state, term, &candidates[selected]);
        assert!(!bindings.is_empty() && bindings.len() <= 8, "bounded actual native storage bindings");
        liveness::apply_active_loan_terminator_transfer(&mut state, function, tc, contract, term,
            &values.summaries, Some((&table, &candidates[selected])));
        for binding in &bindings {
            let captured = source_ids.iter().all(|id| state.owner_contains(*id, binding.owner.local));
            let address = address_ids.iter().any(|id| state.owner_contains(*id, binding.owner.local));
            let receiver = binding.excluded.iter().any(|id| state.owner_contains(*id, binding.owner.local));
            eprintln!("STRUCTURED_CALLABLE case={case} native={key:?} role={kind:?} owner={:?} sourceLoans={source_ids:?} addressLoans={address_ids:?} captureRetained={captured} addressRetained={address} receiverRetained={receiver}", binding.owner);
            transport_ok &= captured && !address && !receiver;
        }
    }
    let (_, origin_bindings) = BorrowChecker::validate_moves_and_initialization(function, tc, contract, &values.summaries, &mutations);
    let mut escape = ReferenceEscapeAnalysis::new(function, tc, contract, &lifetime, MissingSummaryMode::Conservative);
    escape.mutation_origins = Some(&origin_bindings);
    let origins = run_fixpoint(&escape, function);
    let root = Place { local: Local(0), projection: Vec::new() };
    let mut saw_aggregate = false;
    let mut saw_use = false;
    let mut producer_ok = false;
    let mut forwarding_ok = true;
    let mut return_ok = true;
    let mut saw_return = false;
    let mut rows = 0;
    for (block, data) in function.basic_blocks.iter().enumerate().filter(|(block, _)| origins.reachable[*block]) {
        let mut state = origins.entry_sets[block].clone();
        for (index, statement) in data.statements.iter().enumerate() {
            let transfer = match &statement.kind {
                StatementKind::Assign(dest, value) if checked_place_type(tc, contract, function, dest).as_ref() == Some(&holder_type) => {
                    match value {
                        Rvalue::Aggregate(_, operands) if dest.local == holder || operands.iter().any(|operand|
                            matches!(operand, Operand::Copy(place) | Operand::Move(place) if place.local == callback)) => {
                            let input = operands.iter().flat_map(|operand| escape.origins_for_operand(&state, operand)).collect::<OriginSet>();
                            Some((dest, "Aggregate", input))
                        }
                        Rvalue::Use(operand @ (Operand::Copy(place) | Operand::Move(place)))
                            if place.projection.is_empty() && (place.local == holder || place.local == staged) =>
                                Some((dest, "Use", escape.origins_for_operand(&state, operand))),
                        _ => None,
                    }
                }
                _ => None,
            };
            escape.apply_statement(&mut state, statement);
            if matches!(&statement.kind, StatementKind::Assign(_, Rvalue::Closure(actual)) if actual.id == closure.id) {
                let envelope = escape.origins_for_place(&state, closure_destination);
                let retained = origin.is_some_and(|local| envelope.contains(&ReferenceOrigin::Local(local)));
                producer_ok = if negative { retained }
                    else { !envelope.iter().any(|origin| matches!(origin, ReferenceOrigin::Local(_))) };
                eprintln!("STRUCTURED_CALLABLE case={case} block={block} statement={index} transfer=Closure producer={closure_destination:?} sourceOrigin={retained} actualSpan={expected_span:?}");
            }
            if let Some((dest, label, input)) = transfer {
                rows += 1;
                assert!(rows <= 32, "structured transfer-row cap32 exceeded");
                let output = escape.origins_for_place(&state, dest);
                let input_local = input.iter().any(|origin| matches!(origin, ReferenceOrigin::Local(_)));
                let output_local = output.iter().any(|origin| matches!(origin, ReferenceOrigin::Local(_)));
                let expected = origin.map(ReferenceOrigin::Local);
                let incoming = expected.is_some_and(|origin| input.contains(&origin));
                let outgoing = expected.is_some_and(|origin| output.contains(&origin));
                eprintln!("STRUCTURED_CALLABLE case={case} block={block} statement={index} transfer={label} dest={dest:?} sourceOriginIn={incoming} sourceOriginOut={outgoing} localIn={input_local} localOut={output_local}");
                if label == "Aggregate" { saw_aggregate = true; }
                if label == "Use" { saw_use = true; }
                forwarding_ok &= if negative { incoming && outgoing } else { !input_local && !output_local };
            }
        }
        if matches!(data.terminator, Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })) {
            saw_return = true;
            let envelope = escape.origins_for_place(&origins.exit_sets[block], &root);
            let retained = origin.is_some_and(|local| envelope.contains(&ReferenceOrigin::Local(local)));
            let mut state = entries[block].clone();
            loans.apply_statements(&mut state, function, crate::mir::BasicBlockId(block));
            let capture_retained = source_ids.iter().all(|id| state.owner_contains(*id, Local(0)));
            let address_retained = address_ids.iter().any(|id| state.owner_contains(*id, Local(0)));
            eprintln!("STRUCTURED_CALLABLE case={case} returnBlock={block} sourceOrigin={retained} sourceLoans={source_ids:?} captureRetained={capture_retained} addressRetained={address_retained}");
            return_ok &= if negative { retained && capture_retained && !address_retained }
                else { !envelope.iter().any(|origin| matches!(origin, ReferenceOrigin::Local(_))) && !address_retained };
        }
    }
    assert_eq!(std::fs::read(&entry).unwrap(), source.as_bytes());
    if negative {
        let diagnostics = outcome.expect_err("structured owning return was UNSAFE-ACCEPTED; see BorrowOutcome before trace assertions");
        assert!(diagnostics.0.iter().all(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)));
        assert!(diagnostics.0.iter().any(|diagnostic|
            diagnostic.message == "Cannot return reference to local value because it does not live long enough"
                && diagnostic.primary.as_ref().is_some_and(|label| Some(&label.span) == expected_span.as_ref())),
            "must reject owning return at the actual source capture span, not any-error/incidental Borrow");
    } else {
        outcome.expect("legitimate shape-matched positive must pass Borrow, not an unsupported earlier-stage substitute");
    }
    assert!(saw_aggregate && saw_use && saw_return, "actual Holder aggregate/whole Use/owning Return required");
    assert!(producer_ok, "actual closure producer must supply the existing dependency envelope, not a signature-derived proof");
    assert!(forwarding_ok, "existing source envelope lost at Holder aggregate/whole Use; no fabricated replacement allowed");
    assert!(transport_ok, "native source Contents/Value transport lost capture loans or retained address/receiver loans");
    assert!(return_ok, "source origin/capture LoanIds must survive into Local0; positives must not acquire Local/address origins");
}

#[test]
fn structured_callable_whole_holder_borrowed_capture_rejects_owning_return() {
    structured_callable_control("whole_holder_borrow", "    answer = 42\n    view = &answer\n    callback = -> *view\n",
        None, Some(crate::mir::MirClosureCaptureKind::ByRef));
}

#[test]
fn structured_callable_whole_holder_capture_free_returns_without_local_origin() {
    structured_callable_control("whole_holder_free", "    callback = -> 42\n", None, None);
}

#[test]
fn structured_callable_native_payload_borrowed_capture_rejects_owning_return() {
    structured_callable_control("structured_payload_borrow", "    answer = 42\n    view = &answer\n    callback = -> *view\n",
        Some(crate::hir::borrowing::BorrowSourceKind::Value), Some(crate::mir::MirClosureCaptureKind::ByRef));
}

#[test]
fn structured_callable_native_payload_by_value_scalar_returns_without_local_origin() {
    structured_callable_control("structured_payload_scalar", "    scalar = 42\n    callback = -> scalar.into_value!\n",
        Some(crate::hir::borrowing::BorrowSourceKind::Value), Some(crate::mir::MirClosureCaptureKind::ByValue));
}

#[test]
fn structured_callable_contents_borrowed_capture_rejects_owning_return() {
    structured_callable_control("structured_contents_borrow", "    answer = 42\n    view = &answer\n    callback = -> *view\n",
        Some(crate::hir::borrowing::BorrowSourceKind::Contents), Some(crate::mir::MirClosureCaptureKind::ByRef));
}

#[test]
fn structured_callable_contents_capture_free_does_not_retain_holder_address() {
    structured_callable_control("structured_contents_free", "    callback = -> 42\n",
        Some(crate::hir::borrowing::BorrowSourceKind::Contents), None);
}

fn structured_updated_child_control(negative: bool) {
    use crate::hir::borrowing::BorrowSourceKind;
    use crate::mir::MirClosureCaptureKind;

    let case = if negative { "updated_child_borrow" } else { "updated_child_free" };
    let producer = if negative { "-> *view" } else { "-> 42" };
    let source = format!(r#"struct Holder
    < anchor: &I64
    < callback: () -> I64
struct Cell T
    < payload: T
store: &mut (Cell Holder) -> &Holder -> ()
store = dest, source ->
    dest.payload = Holder
        anchor: source.anchor
        callback: source.callback
    ()
escape: &I64 -> Cell Holder
escape = input ->
    holder = Holder
        anchor: input
        callback: -> 0
    mut staged = holder
    answer = 42
    view = &answer
    updated = {producer}
    staged.callback = updated
    mut cell = Cell
        payload: Holder
            anchor: input
            callback: -> 0
    source_view = &staged
    store &mut cell, source_view
    cell
main = !->
    anchor = 7
    result = escape &anchor
    ()
"#);
    eprintln!("UPDATED_CHILD case={case} phase=fixture bytes={} source={source:?} execution=NONE", source.len());
    let (program, entry) = mutation_source_program(case, &source, false);
    let source_bytes = std::fs::read(&entry).unwrap();
    let hash = integration_sha(&entry);
    let agreement = crate::mir::agreement::check_mir_runtime_agreement(&program);
    eprintln!("UPDATED_CHILD case={case} phase=agreement SHA256={hash} report={agreement:?}");
    assert!(agreement.is_clean() && agreement.object_contract_errors.is_empty()
        && agreement.erased_contract_errors.is_empty(), "fixture must agree before Borrow");
    let outcome = BorrowChecker::run(&program);
    eprintln!("UPDATED_CHILD case={case} BorrowOutcome={} execution=NONE", if outcome.is_ok() { "ACCEPTED" } else { "REJECTED" });
    if let Err(diagnostics) = &outcome {
        assert!(diagnostics.0.len() <= 32, "updated-child diagnostic cap32 exceeded");
        for diagnostic in &diagnostics.0 { eprintln!("UPDATED_CHILD case={case} actual={diagnostic:?}"); }
    }
    assert!(program.functions().count() <= 8, "updated-child function cap8 exceeded");
    let span_for = |line: &str, offset: usize, len: usize| {
        let start = source.find(line).expect("unchanged source binding") + offset;
        Span::new(entry.clone(), start, start + len)
    };
    let holder_span = span_for("    holder = Holder", 4, "holder".len());
    let staged_span = span_for("    mut staged = holder", 8, "staged".len());
    let view_span = span_for("    view = &answer", 4, "view".len());
    let cell_span = span_for("    mut cell = Cell", 8, "cell".len());
    let (_, function) = program.functions().find(|(_, function)| function.local_decls.iter()
        .any(|local| local.span.as_ref() == Some(&holder_span))).expect("actual source escape body");
    let local_at = |span: &Span| Local(function.local_decls.iter().position(|local|
        local.span.as_ref() == Some(span)).expect("actual source local declaration"));
    let holder = local_at(&holder_span);
    let staged = local_at(&staged_span);
    let view = local_at(&view_span);
    let cell = local_at(&cell_span);
    let tc = &program.type_context;
    let contract = &program.backend_contract;
    let holder_type = checked_place_type(tc, contract, function, &Place { local: holder, projection: Vec::new() }).unwrap();
    assert!(!holder_type.is_copy(), "fixture must not copy a whole non-Copy Holder through &Holder");
    let Type::Struct { id: holder_id, .. } = &holder_type else { panic!("actual Holder type"); };
    let field_ids = &contract.nominal_field_ids[holder_id];
    let fields = contract_struct_fields(contract, *holder_id).unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(field_ids.len(), 2);
    assert_eq!(tc.type_for(fields[0].1), Type::Reference { mutable: false, inner: Box::new(Type::I64) });
    let Type::Function { params, ret, .. } = tc.type_for(fields[1].1) else { panic!("checked callback field"); };
    assert!(params.is_empty() && ret.as_ref() == &Type::I64 && tc.type_for(fields[1].1).is_copy());
    assert_eq!(function.arg_count, 1);
    let input = Local(1);
    assert_eq!(tc.type_for(function.local_decls[input.0].ty), tc.type_for(fields[0].1));
    let input_span = function.local_decls[input.0].span.as_ref().expect("actual incoming parameter span");
    assert_eq!(input_span.file_path, entry);
    assert_eq!(&source[input_span.start..input_span.end], "input");
    let escape_key = contract.function_bodies.get(&function.id).expect("canonical source body key");
    let escape_header = contract.callable(escape_key).expect("canonical source body header");
    assert_eq!(&escape_header.key, escape_key);
    eprintln!("UPDATED_CHILD case={case} function={:?} canonicalKey={escape_key:?} header={escape_header:?} holderType={holder_type:?} fieldIDs={field_ids:?} input={input:?} inputSpan={input_span:?} view={view:?} viewSpan={view_span:?}", function.id);

    let updated_start = source.find("    updated =").unwrap();
    let updated_end = updated_start + source[updated_start..].find('\n').unwrap();
    let (capture_block, capture_index, capture_dest, closure) = function.basic_blocks.iter().enumerate()
        .flat_map(|(block, data)| data.statements.iter().enumerate().map(move |(index, statement)| (block, index, statement)))
        .find_map(|(block, index, statement)| match &statement.kind {
            StatementKind::Assign(dest, Rvalue::Closure(closure)) if statement.span.as_ref().is_some_and(|span|
                span.file_path == entry && span.start >= updated_start && span.end <= updated_end) => Some((block, index, dest, closure)),
            _ => None,
        }).expect("actual projected-write callback producer in its own source line");
    assert_eq!(closure.id.owner, function.id);
    assert_eq!(closure.captures.len(), usize::from(negative));
    if negative {
        let capture = &closure.captures[0];
        assert_eq!(capture.local, view);
        assert_eq!(capture.kind, MirClosureCaptureKind::ByRef);
        assert_eq!(capture.span.as_ref(), Some(&view_span));
    }
    let closure_body = &program.functions[&MirFunctionId::Closure(Box::new(closure.id.clone()))];
    assert_eq!(closure_body.closure_captures.len(), closure.captures.len());
    if negative { assert_eq!(closure_body.closure_captures[0].kind, MirClosureCaptureKind::ByRef); }
    eprintln!("UPDATED_CHILD case={case} actualClosure={:?} creationBlock={capture_block} creationStatement={capture_index} destination={capture_dest:?} captures={:?} bodyCaptures={:?}",
        closure.id, closure.captures, closure_body.closure_captures);
    let lifetime = BorrowChecker::reference_return_summaries(&program);
    let values = BorrowChecker::native_return_loan_proof(&program);
    let mutations = NativeMutationTable::for_program(&program);
    let candidates = function.basic_blocks.iter().map(|block| block.terminator.as_ref()
        .map(|term| native_mutation_candidates(function, tc, contract, &mutations, term)).unwrap_or_default()).collect::<Vec<_>>();
    let borrows = collect_function_borrows_with_contract(function, tc, contract, &values.summaries);
    let table = LoanTable::from_borrows_with_paths(&borrows, paths::PlacePathTable::for_function(function));
    let capture_ids = table.iter().filter(|loan| loan.initial_owner == capture_dest.local
        && loan.kind == LoanKind::Shared && loan.created_at.block.0 == capture_block
        && loan.created_at.statement.0 == capture_index && loan.diagnostic_place == (Place { local: view, projection: Vec::new() }))
        .map(|loan| loan.id).collect::<HashSet<_>>();
    assert_eq!(capture_ids.len(), usize::from(negative), "actual selected ByRef capture LoanId, not an Fn signature proof");
    let address_ids = table.iter().filter(|loan| loan.kind == LoanKind::Shared
        && loan.diagnostic_place == (Place { local: staged, projection: Vec::new() }))
        .map(|loan| loan.id).collect::<HashSet<_>>();
    assert!(!address_ids.is_empty() && capture_ids.is_disjoint(&address_ids));
    let mut selected_ids = capture_ids.union(&address_ids).copied().map(|id| (function.id.clone(), id)).collect::<HashSet<_>>();
    let mut selected_paths = table.iter().filter(|loan| capture_ids.contains(&loan.id) || address_ids.contains(&loan.id))
        .map(|loan| (function.id.clone(), loan.place_path)).collect::<HashSet<_>>();
    assert!(selected_ids.len() <= 8 && selected_paths.len() <= 32, "updated-child selected loan/path cap exceeded");
    for id in capture_ids.union(&address_ids) {
        let loan = table.get(*id).unwrap();
        eprintln!("UPDATED_CHILD case={case} selectedLoan={loan:?} capture={} holderAddress={}", capture_ids.contains(id), address_ids.contains(id));
        let span = loan.origin_span.as_ref().expect("actual loan creation operation span");
        assert_eq!(span.file_path, entry);
        eprintln!("UPDATED_CHILD case={case} loanSource={:?}", &source[span.start..span.end]);
    }
    let locations = LoanLocationIndex::for_function(&table, function);
    let live = liveness::compute_reference_liveness_with_mutations(function, tc, contract, &table, &candidates);
    let entries = liveness::compute_active_loan_entries_with_mutations(function, tc, contract, &table, &locations, &live, &values.summaries, &candidates);
    let loans = liveness::ActiveLoanAnalysis::new(tc, contract, &table, &locations, &live, &values.summaries).with_mutations(&candidates);
    let (_, origin_bindings) = BorrowChecker::validate_moves_and_initialization(function, tc, contract, &values.summaries, &mutations);
    let mut escape = ReferenceEscapeAnalysis::new(function, tc, contract, &lifetime, MissingSummaryMode::Conservative);
    escape.mutation_origins = Some(&origin_bindings);
    let origins = run_fixpoint(&escape, function);
    let root = Place { local: staged, projection: Vec::new() };
    let child = ReferenceEscapeAnalysis::field_place(&root, 1);
    let incoming = ReferenceOrigin::Param(input);
    let local_origin = ReferenceOrigin::Local(view);
    let mut saw_stage = false;
    let mut saw_write = false;
    let mut saw_call = false;
    let mut saw_return = false;
    let mut retained_origin = true;
    let mut retained_loans = true;
    let mut selected_rows = 0;
    for (block, data) in function.basic_blocks.iter().enumerate().filter(|(block, _)| origins.reachable[*block]) {
        let mut state = origins.entry_sets[block].clone();
        for (index, statement) in data.statements.iter().enumerate() {
            escape.apply_statement(&mut state, statement);
            let phase = match &statement.kind {
                StatementKind::Assign(dest, Rvalue::Use(Operand::Move(source))) if dest == &root
                    && source.local == holder && source.projection.is_empty() => { saw_stage = true; Some("stage") }
                StatementKind::Assign(dest, _) if ReferenceEscapeAnalysis::normalized_place(dest.clone()) == child => {
                    saw_write = true; Some("projected-write")
                }
                _ => None,
            };
            if let Some(phase) = phase {
                selected_rows += 1;
                assert!(selected_rows <= 32, "updated-child phase-row cap32 exceeded");
                let StatementKind::Assign(dest, _) = &statement.kind else { unreachable!(); };
                selected_paths.insert((function.id.clone(), table.place_paths().path_id(dest).expect("actual selected path")));
                assert!(selected_paths.len() <= 32, "updated-child selected-path cap32 exceeded");
                let exact = state.origins.get(&root).cloned().unwrap_or_default();
                let child_row = state.origins.get(&child).cloned().unwrap_or_default();
                let children = escape.child_origins_for_place(&state, &root);
                let query = escape.origins_for_place(&state, &root);
                let union = exact.union(&children).copied().collect::<OriginSet>();
                eprintln!("UPDATED_CHILD case={case} phase={phase} block={block} statement={index} actual={statement:?} canonicalCallbackField={:?} exactRoot={exact:?} callbackChild={child_row:?} productionCombinedQuery={query:?} existingRowsUnion={union:?}", field_ids[1]);
                assert!(exact.contains(&incoming), "fixture must establish and preserve the legitimate incoming root summary");
                if phase == "stage" { assert!(!union.contains(&local_origin), "callback update must occur after whole staging"); }
                if phase == "projected-write" {
                    assert_eq!(child_row.contains(&local_origin), negative, "actual projected child must reflect the chosen closure");
                }
            }
        }
        if let Some(term @ Terminator::Call { .. }) = &data.terminator {
            let rows = &candidates[block];
            if rows.iter().any(|row| row.destination.argument == 0 && row.destination.kind == BorrowSourceKind::Contents
                && row.sources.iter().any(|source| source.argument == 1 && source.kind == BorrowSourceKind::Contents)) {
                saw_call = true;
                selected_rows += 1;
                assert!(selected_rows <= 32, "updated-child phase-row cap32 exceeded");
                let Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))), args, .. } = term else { panic!("canonical native Unit call"); };
                let header = contract.callable(key).unwrap();
                let crate::mir::MirCallableKind::LocalBody { function_id } = &header.kind else { panic!("actual native body"); };
                assert_eq!(contract.function_bodies.get(function_id), Some(key));
                assert_eq!(header.signature.ret.semantic_ty, tc.id_for_type(&Type::Unit).unwrap());
                assert_eq!(header.signature.ret.abi_ty, header.signature.ret.semantic_ty);
                assert_eq!(mutations.knowledge(key), NativeMutationKnowledge::Unknown);
                assert!(!rows.iter().flat_map(|row| &row.sources).any(|source| source.argument == 1 && source.kind == BorrowSourceKind::Value));
                // The real body copies fields, never a whole Holder from shared storage.
                let mut copied_fields = HashSet::new();
                for statement in program.functions[function_id].basic_blocks.iter().flat_map(|block| &block.statements) {
                    if let StatementKind::Assign(_, Rvalue::Use(operand @ (Operand::Copy(place) | Operand::Move(place)))) = &statement.kind {
                        if place.local == Local(2) && place.projection.first() == Some(&Projection::Deref) {
                            let ty = checked_place_type(tc, contract, &program.functions[function_id], place).unwrap();
                            assert!(ty.is_copy() && ty != holder_type, "only checked Copy fields may be read from &Holder");
                            assert!(matches!(operand, Operand::Copy(_)), "source fields are real Copy reads");
                            let Some(Projection::Field { index, .. }) = place.projection.get(1) else { panic!("actual source field projection"); };
                            assert!(*index < field_ids.len());
                            copied_fields.insert(*index);
                            eprintln!("UPDATED_CHILD case={case} actualStoreFieldCopy={operand:?} type={ty:?}");
                        }
                    }
                }
                assert_eq!(copied_fields, HashSet::from([0usize, 1]), "actual anchor and callback field copies required");
                let mut active = entries[block].clone();
                loans.apply_statements(&mut active, function, crate::mir::BasicBlockId(block));
                let (Operand::Copy(source_place) | Operand::Move(source_place)) = &args[1] else { panic!("safe &Holder actual source"); };
                assert!(address_ids.iter().all(|id| active.is_active(*id) && active.storage_owner_contains(*id, source_place.local)));
                let bindings = native_mutation_bindings(function, tc, contract, &table, &active, term, rows);
                assert!(!bindings.is_empty() && bindings.len() <= 8, "actual bounded Contents owner alternatives");
                for binding in &bindings {
                    assert_eq!(binding.owner.local, cell);
                    assert_eq!(checked_place_type(tc, contract, function, &binding.value), Some(holder_type.clone()));
                    selected_ids.extend(binding.excluded.iter().map(|id| (function.id.clone(), *id)));
                    selected_paths.insert((function.id.clone(), table.place_paths().path_id(&binding.value).unwrap()));
                    assert!(selected_ids.len() <= 8 && selected_paths.len() <= 32, "updated-child selected loan/path cap exceeded");
                    eprintln!("UPDATED_CHILD case={case} phase=unit-contents native={key:?} header={header:?} rowRoles={rows:?} owner={:?} value={:?} exactSourceRoot={:?} updatedChildren={:?} productionCombinedQuery={:?}",
                        binding.owner, binding.value, state.origins.get(&ReferenceEscapeAnalysis::normalized_place(binding.value.clone())),
                        escape.child_origins_for_place(&state, &binding.value), escape.origins_for_place(&state, &binding.value));
                }
                escape.apply_mutation_origins(&mut state, &bindings);
                liveness::apply_active_loan_terminator_transfer(&mut active, function, tc, contract, term, &values.summaries, Some((&table, rows)));
                for binding in &bindings {
                    let envelope = escape.origins_for_place(&state, &binding.owner);
                    let captured = capture_ids.iter().all(|id| active.owner_contains(*id, cell));
                    let address = address_ids.iter().any(|id| active.owner_contains(*id, cell));
                    let receiver = binding.excluded.iter().any(|id| active.owner_contains(*id, cell));
                    eprintln!("UPDATED_CHILD case={case} phase=unit-result envelope={envelope:?} captureLoanIDs={capture_ids:?} captureRetained={captured} holderAddressIDs={address_ids:?} addressRetained={address} receiverRetained={receiver}");
                    for id in capture_ids.union(&address_ids) {
                        eprintln!("UPDATED_CHILD case={case} function={:?} loan={id:?} roleAtCell storage={} anyCarrier={}",
                            function.id, active.storage_owner_contains(*id, cell), active.owner_contains(*id, cell));
                    }
                    assert!(envelope.contains(&incoming), "legitimate incoming anchor envelope must survive Contents transfer");
                    retained_origin &= envelope.contains(&local_origin) == negative;
                    retained_loans &= captured && !address && !receiver;
                }
            }
        }
        if matches!(data.terminator, Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })) {
            saw_return = true;
            selected_rows += 1;
            assert!(selected_rows <= 32, "updated-child phase-row cap32 exceeded");
            let result = Place { local: Local(0), projection: Vec::new() };
            let envelope = escape.origins_for_place(&origins.exit_sets[block], &result);
            let mut active = entries[block].clone();
            loans.apply_statements(&mut active, function, crate::mir::BasicBlockId(block));
            let captured = capture_ids.iter().all(|id| active.owner_contains(*id, Local(0)));
            let address = address_ids.iter().any(|id| active.owner_contains(*id, Local(0)));
            eprintln!("UPDATED_CHILD case={case} phase=return block={block} envelope={envelope:?} captureLoanIDs={capture_ids:?} captureIntoLocal0={captured} sourceAddressIntoLocal0={address}");
            for id in capture_ids.union(&address_ids) {
                eprintln!("UPDATED_CHILD case={case} function={:?} loan={id:?} roleAtReturn storage={} anyCarrier={}",
                    function.id, active.storage_owner_contains(*id, Local(0)), active.owner_contains(*id, Local(0)));
            }
            assert!(envelope.contains(&incoming), "incoming parameter remains legitimate, not a local-capture proof");
            retained_origin &= envelope.contains(&local_origin) == negative;
            retained_loans &= captured && !address;
            if !negative { assert!(!envelope.iter().any(|origin| matches!(origin, ReferenceOrigin::Local(_) | ReferenceOrigin::Temporary(_)))); }
        }
    }

    // Caller anchor loan is a separate function-scoped ID, not the Holder-address loan.
    let caller_span = span_for("    anchor = 7", 4, "anchor".len());
    let (_, caller) = program.functions().find(|(_, function)| function.local_decls.iter()
        .any(|local| local.span.as_ref() == Some(&caller_span))).expect("actual caller anchor owner");
    let anchor = Local(caller.local_decls.iter().position(|local| local.span.as_ref() == Some(&caller_span)).unwrap());
    let caller_borrows = collect_function_borrows_with_contract(caller, tc, contract, &values.summaries);
    let caller_table = LoanTable::from_borrows_with_paths(&caller_borrows, paths::PlacePathTable::for_function(caller));
    let anchor_ids = caller_table.iter().filter(|loan| loan.kind == LoanKind::Shared
        && loan.diagnostic_place == (Place { local: anchor, projection: Vec::new() })).map(|loan| loan.id).collect::<HashSet<_>>();
    assert!(!anchor_ids.is_empty(), "real incoming scalar-reference loan, not fabricated Param provenance");
    selected_ids.extend(anchor_ids.iter().map(|id| (caller.id.clone(), *id)));
    assert!(selected_ids.len() <= 8, "updated-child selected-loan cap8 exceeded");
    let caller_candidates = caller.basic_blocks.iter().map(|block| block.terminator.as_ref()
        .map(|term| native_mutation_candidates(caller, tc, contract, &mutations, term)).unwrap_or_default()).collect::<Vec<_>>();
    let caller_locations = LoanLocationIndex::for_function(&caller_table, caller);
    let caller_live = liveness::compute_reference_liveness_with_mutations(caller, tc, contract, &caller_table, &caller_candidates);
    let caller_entries = liveness::compute_active_loan_entries_with_mutations(caller, tc, contract, &caller_table, &caller_locations, &caller_live, &values.summaries, &caller_candidates);
    let caller_loans = liveness::ActiveLoanAnalysis::new(tc, contract, &caller_table, &caller_locations, &caller_live, &values.summaries).with_mutations(&caller_candidates);
    let mut caller_seen = false;
    for (block, data) in caller.basic_blocks.iter().enumerate() {
        let Some(term @ Terminator::Call { func: Operand::Constant(Constant::Callable(callable)), destination, .. }) = &data.terminator else { continue; };
        if callable.function_id().as_ref() != Some(&function.id) { continue; }
        caller_seen = true;
        let mut active = caller_entries[block].clone();
        caller_loans.apply_statements(&mut active, caller, crate::mir::BasicBlockId(block));
        assert!(anchor_ids.iter().all(|id| active.is_active(*id)), "actual caller anchor loan must be live before call");
        liveness::apply_active_loan_terminator_transfer(&mut active, caller, tc, contract, term, &values.summaries, Some((&caller_table, &caller_candidates[block])));
        for id in &anchor_ids {
            let loan = caller_table.get(*id).unwrap();
            selected_paths.insert((caller.id.clone(), loan.place_path));
            assert!(selected_paths.len() <= 32, "updated-child selected-path cap32 exceeded");
            eprintln!("UPDATED_CHILD case={case} callerAnchorLoan={loan:?} scalarSpan={caller_span:?} destination={destination:?} retained={}", active.owner_contains(*id, destination.local));
            assert!(active.owner_contains(*id, destination.local), "valid incoming reference loan must survive the native result");
        }
    }
    assert_eq!(std::fs::read(&entry).unwrap(), source_bytes);
    assert_eq!(source_bytes, source.as_bytes());
    assert_eq!(integration_sha(&entry), hash);
    eprintln!("UPDATED_CHILD case={case} SHA256_AFTER={hash} selectedLoans={} selectedPaths={} phaseRows={selected_rows}", selected_ids.len(), selected_paths.len());
    assert!(selected_ids.len() <= 8 && selected_paths.len() <= 32 && selected_rows <= 32, "updated-child observation caps exceeded");
    assert!(saw_stage && saw_write && saw_call && saw_return && caller_seen, "fixture must reach all four actual transport phases and the caller");
    if negative {
        let diagnostics = outcome.expect_err("SECURITY RED: valid updated-child source was UNSAFE-ACCEPTED; see actual phases and BorrowOutcome");
        assert!(diagnostics.0.iter().all(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
            && matches!(&diagnostic.location, crate::diagnostic::DiagnosticLocation::Source(_)) && diagnostic.primary.is_some()), "{diagnostics:?}");
        assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message == "Cannot return reference to local value because it does not live long enough"
            && diagnostic.primary.as_ref().is_some_and(|label| label.span == view_span)), "must reject actual new local view dependency, not another Borrow: {diagnostics:?}");
    } else { outcome.expect("capture-free matching updated child must preserve valid incoming anchor without fabricated local/address origins"); }
    assert!(retained_origin, "existing projected child origin was lost at native Contents transfer/owning return");
    assert!(retained_loans, "actual capture LoanIds must reach Local0 without Holder-address or receiver-storage loans");
}

#[test]
fn structured_callable_updated_child_borrowed_capture_rejects_owning_return() {
    structured_updated_child_control(true);
}

#[test]
fn structured_callable_updated_child_capture_free_preserves_incoming_anchor() {
    structured_updated_child_control(false);
}

fn scalar_index_temporary_control(case: &str) {
    let (fixture, origin_range) = match case {
        "cast" => ("test_fixed_array_borrow_coercion_preserves_original_storage_alias", (127, 128)),
        "first" => ("test_explicit_fixed_array_type_syntax_lowers_successfully", (52, 55)),
        "temp" => ("test_bubble_sort", (164, 167)),
        _ => panic!("unknown scalar/index control"),
    };
    for flag in ["ROCK_INTEGRATION_BORROW_TRACE", "ROCK_INTEGRATION_BORROW_DETAIL", "ROCK_OWNER_LOAN_TRACE",
        "ROCK_OWNER_RETURN_TRACE", "ROCK_FRESH_STDLIB_RETURN_TRACE", "ROCK_ALLOCATOR_EXECUTION_DEBUG", "ROCK_ALLOCATOR_EXECUTION_MIR"] {
        assert!(std::env::var_os(flag).is_none(), "unset unrelated observer flag {flag}");
    }
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixture_path = workspace.join("lib/tests/integration.rs");
    let archive = agent_work_path("recovery/rock-completion-integration-6.log");
    let fixture_bytes = std::fs::read(&fixture_path).unwrap();
    let archive_bytes = std::fs::read(&archive).unwrap();
    let source = integration_log_source(&archive, fixture);
    assert_eq!(source, integration_literal(&fixture_path, &format!("fn {fixture}()")), "exact historical source, no annotations or replacement fixture");
    assert_eq!(&source[origin_range.0..origin_range.1], if case == "cast" { "s" } else { "arr" });
    eprintln!("SCALAR_INDEX case={case} fixture={fixture} phase=fixture-harness bytes={} exactSource={source:?} explicitDependency=stdlib execution=NONE; frontend/mono/agreement failures here are HARNESS, not Borrow RED", source.len());
    let (program, entry) = mutation_source_program(&format!("scalar_index_{case}"), &source, true);
    let hash = integration_sha(&entry);
    if case == "cast" {
        assert_eq!(source.len(), 315);
        assert_eq!(hash, "8e64a23fe8f1e0d060a36d25bfb19997736364e787723b9c071235699a0fee09");
    }
    let agreement = crate::mir::agreement::check_mir_runtime_agreement(&program);
    eprintln!("SCALAR_INDEX case={case} SHA256_BEFORE={hash} RuntimeAgreement={agreement:?}");
    assert!(agreement.is_clean() && agreement.object_contract_errors.is_empty() && agreement.erased_contract_errors.is_empty(), "HARNESS: clean agreement required before Borrow");
    let outcome = BorrowChecker::run(&program);
    eprintln!("SCALAR_INDEX case={case} BorrowOutcome={} execution=NONE", if outcome.is_ok() { "ACCEPTED" } else { "REJECTED" });
    if let Err(diagnostics) = &outcome {
        assert!(diagnostics.0.len() <= 24, "scalar/index diagnostic cap24 exceeded");
        for diagnostic in &diagnostics.0 { eprintln!("SCALAR_INDEX case={case} actualDiagnostic={diagnostic:?}"); }
    }

    let tc = &program.type_context;
    let contract = &program.backend_contract;
    let scalar = tc.id_for_type(&Type::I64).expect("HARNESS: actual I64 TypeId");
    let first_arg = source.find("get_first = arr ->").map(|start| start + "get_first = ".len());
    let temp_binding = source.find("temp = arr[i]");
    let selected = program.functions().flat_map(|(_, function)| function.basic_blocks.iter().enumerate()
        .flat_map(move |(block, data)| data.statements.iter().enumerate().map(move |(index, statement)| (function, block, index, statement))))
        .filter(|(function, _, _, statement)| {
            let StatementKind::Assign(dest, _) = &statement.kind else { return false; };
            if contract.place_type_id(tc, function, dest) != Some(scalar) { return false; }
            match case {
                "cast" => dest.local == Local(0) && dest.projection.is_empty()
                    && statement.span.as_ref().is_some_and(|span| span.file_path == entry && (span.start, span.end) == (146, 149)),
                "first" => dest.local == Local(0) && dest.projection.is_empty()
                    && function.local_decls.iter().any(|decl| decl.source == LocalSource::Argument && decl.span.as_ref().is_some_and(|span|
                        span.file_path == entry && Some(span.start) == first_arg && span.end == span.start + 3)),
                "temp" => dest.projection.is_empty() && function.local_decls[dest.local.0].source == LocalSource::UserBinding
                    && function.local_decls[dest.local.0].span.as_ref().is_some_and(|span|
                        span.file_path == entry && Some(span.start) == temp_binding && span.end == span.start + 4),
                _ => false,
            }
        }).collect::<Vec<_>>();
    assert_eq!(selected.len(), 1, "HARNESS: exactly one actual historical scalar assignment, not a guessed rejecting call");
    let (function, block, index, statement) = selected[0];
    let StatementKind::Assign(dest, rvalue) = &statement.kind else { unreachable!(); };
    let operand = match (case, rvalue) {
        ("cast", Rvalue::Cast(operand, target)) => { assert_eq!(*target, scalar); operand }
        ("first" | "temp", Rvalue::Use(operand)) => operand,
        _ => panic!("HARNESS: selected scalar assignment has unexpected actual Rvalue {rvalue:?}"),
    };
    let (Operand::Copy(input) | Operand::Move(input)) = operand else { panic!("HARNESS: actual place operand required"); };
    let dest_ty = contract.place_type_id(tc, function, dest).unwrap();
    let input_ty = contract.place_type_id(tc, function, input).expect("HARNESS: checked scalar assignment input");
    assert_eq!(tc.type_for(dest_ty), Type::I64);
    assert!(!return_type_needs_escape_check(tc, contract, dest_ty));
    if case == "cast" {
        assert_eq!(dest_ty, TypeId(0));
        assert_eq!(input_ty, TypeId(3));
        assert_eq!(tc.type_for(input_ty), Type::Pointer(Box::new(Type::I64)));
        assert_eq!(&source[146..149], "I64");
    } else {
        assert_eq!(tc.type_for(input_ty), Type::I64);
        assert!(matches!(operand, Operand::Copy(_)) && input.projection.contains(&Projection::Deref),
            "HARNESS: actual Copy scalar load through the returned reference, not Fn/native call as scalar assignment");
    }
    let key = contract.function_bodies.get(&function.id).expect("canonical source body key");
    let header = contract.callable(key).expect("canonical source body header");
    assert_eq!(&header.key, key);
    assert!(matches!(&header.kind, crate::mir::MirCallableKind::LocalBody { function_id } if function_id == &function.id));
    assert!(header.source_def_id.is_some());
    let lifetime = BorrowChecker::reference_return_summaries(&program);
    let values = BorrowChecker::native_return_loan_proof(&program);
    let mutations = NativeMutationTable::for_program(&program);
    let candidates = function.basic_blocks.iter().map(|data| data.terminator.as_ref()
        .map(|term| native_mutation_candidates(function, tc, contract, &mutations, term)).unwrap_or_default()).collect::<Vec<_>>();
    let (_, mutation_origins) = BorrowChecker::validate_moves_and_initialization(function, tc, contract, &values.summaries, &mutations);
    let mut analysis = ReferenceEscapeAnalysis::new(function, tc, contract, &lifetime, MissingSummaryMode::Conservative);
    analysis.mutation_origins = Some(&mutation_origins);
    let results = run_fixpoint(&analysis, function);
    assert!(results.reachable[block], "HARNESS: selected scalar load must be reachable");
    let borrows = collect_function_borrows_with_contract(function, tc, contract, &values.summaries);
    let table = LoanTable::from_borrows_with_paths(&borrows, paths::PlacePathTable::for_function(function));
    let locations = LoanLocationIndex::for_function(&table, function);
    let live = liveness::compute_reference_liveness_with_mutations(function, tc, contract, &table, &candidates);
    let entries = liveness::compute_active_loan_entries_with_mutations(function, tc, contract, &table, &locations, &live, &values.summaries, &candidates);
    let mut state = results.entry_sets[block].clone();
    let mut active = entries[block].clone();
    for (preceding, stmt) in function.basic_blocks[block].statements.iter().enumerate().take(index) {
        analysis.apply_statement(&mut state, stmt);
        integration_loan_after_statement(&mut active, function, &program, stmt, block, preceding, &locations, &table, &live);
    }
    let input_origins = analysis.origins_for_operand(&state, operand);
    let temporaries = input_origins.iter().filter_map(|origin| if let ReferenceOrigin::Temporary(local) = origin { Some(*local) } else { None }).collect::<HashSet<_>>();
    let expected_origin = Span::new(entry.clone(), origin_range.0, origin_range.1);
    assert!(temporaries.iter().any(|local| function.local_decls[local.0].span.as_ref() == Some(&expected_origin)),
        "HARNESS: actual incoming Temporary dependency must match the original rejection span: {input_origins:?}");
    if case == "cast" {
        assert!(temporaries.contains(&Local(5)) && input_origins.contains(&ReferenceOrigin::Param(Local(1))));
        assert_eq!(function.local_decls[5].span.as_ref(), Some(&expected_origin));
    }
    let classification = analysis.temporary_origins_for_assignment(&state, dest, rvalue);
    let mut functions = HashSet::from([function.id.clone()]);
    let mut types = HashSet::from([dest_ty, input_ty, function.local_decls[input.local.0].ty, function.ret_type]);
    types.extend(header.signature.params.iter().map(|param| param.semantic_ty));
    types.extend([header.signature.ret.semantic_ty, header.signature.ret.abi_ty]);
    assert!(types.len() <= 64, "scalar/index TypeId cap64 exceeded");
    let mut records = 1;
    eprintln!("SCALAR_INDEX case={case} record=assignment location=({block},{index}) function={:?} key={key:?} header={header:?} sourceSpan={:?} checkedDestType={dest_ty:?} destType=I64 safeStorageEligible=false actualRvalue={rvalue:?} checkedInputType={input_ty:?} inputType={:?} inputLifetime={input_origins:?} temporaryStorageClassification={classification:?} actualFunctionLifetime={:?} actualFunctionValueLoans={:?}; missing summaries stay absent, not empty", function.id, statement.span, tc.type_for(input_ty), lifetime.get(&function.id), values.summaries.get(&function.id));
    for temporary in &temporaries {
        records += 1;
        assert!(records <= 24, "scalar/index record cap24 exceeded");
        let decl = &function.local_decls[temporary.0];
        types.insert(decl.ty);
        eprintln!("SCALAR_INDEX case={case} record=origin actualTemporary={temporary:?} declaration={decl:?} exactText={:?}",
            decl.span.as_ref().map(|span| source.get(span.start..span.end)).flatten());
    }

    // Walk only the selected scalar input's actual assignment dependencies.
    let mut dependencies = HashSet::from([input.local]);
    loop {
        let before = dependencies.len();
        for stmt in function.basic_blocks.iter().flat_map(|data| &data.statements) {
            if let StatementKind::Assign(target, value) = &stmt.kind {
                if target.projection.is_empty() && dependencies.contains(&target.local) {
                    dependencies.extend(integration_operands(value).iter().map(|place| place.local));
                }
            }
        }
        if dependencies.len() == before { break; }
    }
    let mut native_reference_producer = false;
    for data in &function.basic_blocks {
        let Some(Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(callee_key))), args, destination, span, .. }) = &data.terminator else { continue; };
        if !dependencies.contains(&destination.local) { continue; }
        let callee_header = contract.callable(callee_key).expect("actual selected producer header");
        assert_eq!(&callee_header.key, callee_key);
        types.insert(callee_header.signature.ret.semantic_ty);
        types.insert(callee_header.signature.ret.abi_ty);
        types.extend(callee_header.signature.params.iter().map(|param| param.semantic_ty));
        assert!(types.len() <= 64, "scalar/index TypeId cap64 exceeded");
        let body_id = match &callee_header.kind {
            crate::mir::MirCallableKind::LocalBody { function_id } => {
                assert_eq!(contract.function_bodies.get(function_id), Some(callee_key));
                assert!(program.functions.contains_key(function_id));
                functions.insert(function_id.clone());
                assert!(functions.len() <= 8, "scalar/index selected-function cap8 exceeded");
                Some(function_id)
            }
            _ => None,
        };
        if tc.type_for(callee_header.signature.ret.semantic_ty) == (Type::Reference { mutable: false, inner: Box::new(Type::I64) }) {
            assert!(return_type_needs_escape_check(tc, contract, callee_header.signature.ret.semantic_ty), "actual reference result remains protected");
            let body_id = body_id.expect("HARNESS: actual native reference producer");
            assert_eq!(args.len(), 2);
            assert_eq!(callee_header.signature.params[1].semantic_ty, scalar);
            assert!(lifetime.get(body_id).is_some_and(|origins| origins.iter().any(|origin| matches!(origin, ReferenceOrigin::ParamBorrowedContents(_)))),
                "HARNESS: preserve real native input borrowed-contents Lifetime authority");
            native_reference_producer = true;
        }
        records += 1;
        assert!(records <= 24, "scalar/index record cap24 exceeded");
        eprintln!("SCALAR_INDEX case={case} record=producer actualKey={callee_key:?} header={callee_header:?} args={args:?} destination={destination:?} sourceSpan={span:?} Lifetime={:?} ValueLoans={:?}; None is UNKNOWN, not known-none", body_id.and_then(|id| lifetime.get(id)), body_id.and_then(|id| values.summaries.get(id)));
    }
    assert!(native_reference_producer, "HARNESS: scalar load must descend from the actual canonical native reference result");
    let aliases = provenance::resolve_places(input, &table, &active);
    assert!(aliases.len() <= 16, "scalar/index alias-alternative cap16 exceeded");
    for alias in &aliases {
        let alias_ty = contract.place_type_id(tc, function, alias).expect("HARNESS: actual checked alias alternative");
        types.insert(alias_ty);
    }
    let selected_loans = table.iter().filter(|loan| (case == "cast") || active.owner_contains(loan.id, input.local)
        || loan.origin_span.as_ref() == Some(&expected_origin)).collect::<Vec<_>>();
    assert!(selected_loans.len() <= 16, "scalar/index selected-LoanId cap16 exceeded");
    for loan in &selected_loans {
        records += 1;
        assert!(records <= 24, "scalar/index record cap24 exceeded");
        types.insert(function.local_decls[loan.initial_owner.0].ty);
        let owners = active.owners(loan.id).iter().collect::<Vec<_>>();
        assert!(owners.len() <= 64, "scalar/index carrier-local cap64 exceeded");
        let storage = owners.iter().copied().filter(|owner| active.storage_owner_contains(loan.id, *owner)).collect::<Vec<_>>();
        let non_storage = owners.iter().copied().filter(|owner| !active.storage_owner_contains(loan.id, *owner)).collect::<Vec<_>>();
        eprintln!("SCALAR_INDEX case={case} record=loan actual={loan:?} active={} returnedView={} storageCarriers={storage:?} nonStorageProvenanceCarriers={non_storage:?} inputCarrier={} scalarDestCarrier={}; no origin-to-LoanId synthesis or permission inference",
            active.is_active(loan.id), table.is_returned_view(loan.id), active.owner_contains(loan.id, input.local), active.owner_contains(loan.id, dest.local));
    }
    if case == "cast" {
        // This actual two-loan source also exposes the raw/borrowed_raw/returned
        // carrier sets; never dump an unbounded table for the loop fixture.
        assert!(table.len() <= 16, "scalar Cast raw-role table cap16 exceeded");
        records += 1;
        assert!(records <= 24, "scalar/index record cap24 exceeded");
        eprintln!("SCALAR_INDEX case={case} record=raw-roles actualLoanState={active:?}");
    }
    analysis.apply_statement(&mut state, statement);
    assert_eq!(analysis.origins_for_operand(&state, operand), input_origins,
        "diagnostic classification must not erase the conservative input Lifetime envelope");
    integration_loan_after_statement(&mut active, function, &program, statement, block, index, &locations, &table, &live);
    records += 1;
    assert!(records <= 24 && types.len() <= 64, "scalar/index record/TypeId caps exceeded");
    eprintln!("SCALAR_INDEX case={case} record=assignment-after actualDestinationEnvelope={:?} inputEnvelopeAfter={:?} allActualAliasAlternatives={aliases:?}; eligibility is diagnostic classification only, not read/access authority", analysis.origins_for_place(&state, dest), analysis.origins_for_operand(&state, operand));
    if case == "cast" {
        records += 1;
        assert!(records <= 24, "scalar/index record cap24 exceeded");
        eprintln!("SCALAR_INDEX case={case} record=raw-roles-after actualLoanState={active:?}; liveness may release dead carriers, not delete lease authority");
    }
    for (return_block, data) in function.basic_blocks.iter().enumerate() {
        if !results.reachable[return_block] || !matches!(data.terminator, Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })) { continue; }
        let result = Place { local: Local(0), projection: Vec::new() };
        let result_ty = contract.place_type_id(tc, function, &result).unwrap();
        types.insert(result_ty);
        records += 1;
        assert!(records <= 24 && types.len() <= 64, "scalar/index record/TypeId caps exceeded");
        eprintln!("SCALAR_INDEX case={case} record=return block={return_block} checkedType={result_ty:?} semanticType={:?} safeStorageEligible={} envelope={:?}", tc.type_for(result_ty), return_type_needs_escape_check(tc, contract, result_ty), analysis.origins_for_place(&results.exit_sets[return_block], &result));
    }
    assert_eq!(std::fs::read(&entry).unwrap(), source.as_bytes());
    assert_eq!(integration_sha(&entry), hash);
    assert_eq!(std::fs::read(&fixture_path).unwrap(), fixture_bytes);
    assert_eq!(std::fs::read(&archive).unwrap(), archive_bytes);
    eprintln!("SCALAR_INDEX case={case} SHA256_AFTER={hash} source/archive bytes unchanged functions={} records={records} actualLoanIds={} actualTypeIds={} caps=8/24/16/64 execution=NONE", functions.len(), selected_loans.len(), types.len());
    outcome.expect("SCALAR CLASSIFICATION RED: unchanged valid source storing/returning I64 must pass Borrow; inspect actual typed assignment and diagnostics, not unsafe execution");
    assert!(classification.is_empty(), "ineligible scalar storage must not classify its conservative input Temporary envelope as a stored reference");
}

#[test]
fn scalar_index_temporary_scalar_cast_preserves_input_envelope() {
    scalar_index_temporary_control("cast");
}

#[test]
fn scalar_index_temporary_get_first_returns_loaded_i64() {
    scalar_index_temporary_control("first");
}

#[test]
fn scalar_index_temporary_sort_binding_stores_loaded_i64() {
    scalar_index_temporary_control("temp");
}

mod slice_reference_controls {
    use std::collections::{BTreeSet, VecDeque};
    use std::io::Write;

    use super::*;
    use crate::ids::{Idx, LoanId};

    #[derive(Default)]
    struct Receipt {
        functions: HashSet<MirFunctionId>,
        origins: HashSet<(MirFunctionId, ReferenceOrigin)>,
        loans: HashSet<(MirFunctionId, LoanId)>,
        paths: HashSet<(MirFunctionId, Place)>,
        sites: HashSet<(MirFunctionId, usize, usize)>,
        conversions: usize,
        metadata_borrows: usize,
        array_bindings: HashSet<(MirFunctionId, Local)>,
        view_bindings: HashSet<(MirFunctionId, Local)>,
    }

    impl Receipt {
        fn budget(&self) {
            assert!(self.functions.len() <= 8 && self.origins.len() <= 16
                && self.loans.len() <= 16 && self.paths.len() <= 64 && self.sites.len() <= 64,
                "SLICE_REFERENCE HARNESS OVERFLOW functions={}/8 origins={}/16 loans={}/16 paths={}/64 sites={}/64; no truncated proof",
                self.functions.len(), self.origins.len(), self.loans.len(), self.paths.len(), self.sites.len());
        }

        fn origins(&mut self, function: &MirFunction, label: &str, origins: &OriginSet) {
            for origin in origins { self.origins.insert((function.id.clone(), *origin)); }
            self.budget();
            eprintln!("SLICE_REFERENCE function={:?} {label}={origins:?}", function.id);
        }

        fn path(&mut self, analysis: &ReferenceEscapeAnalysis<'_>, table: &LoanTable, place: &Place) {
            self.paths.insert((analysis.func.id.clone(), place.clone()));
            self.budget();
            let checked = analysis.backend_contract.place_type_id(analysis.type_context, analysis.func, place);
            eprintln!("SLICE_REFERENCE function={:?} actualPlace={place:?} indexedPath={:?} checkedType={checked:?} semanticType={:?}",
                analysis.func.id, table.place_paths().path_id(place), checked.map(|ty| analysis.type_context.type_for(ty)));
        }

        fn snapshot(&mut self, phase: &str, analysis: &ReferenceEscapeAnalysis<'_>, state: &ReferenceOriginState,
            table: &LoanTable, active: &LoanState, places: &[Place]) {
            assert!(places.len() <= 16, "SLICE_REFERENCE HARNESS site-place array cap16");
            let mut selected = BTreeSet::new();
            let mut carriers = HashSet::new();
            for place in places {
                carriers.insert(place.local);
                self.path(analysis, table, place);
                self.origins(analysis.func, phase, &analysis.origins_for_place(state, place));
                let decl = analysis.func.local_decls.get(place.local.0).expect("actual selected local");
                eprintln!("SLICE_REFERENCE phase={phase} function={:?} local={:?} declaration={decl:?}", analysis.func.id, place.local);
                let checked = analysis.backend_contract.place_type_id(analysis.type_context, analysis.func, place);
                if decl.source == LocalSource::UserBinding {
                    if matches!(analysis.type_context.try_ty(decl.ty), Some(Ty::Array { .. })) {
                        self.array_bindings.insert((analysis.func.id.clone(), place.local));
                    }
                    if checked.is_some_and(|ty| matches!(analysis.type_context.try_ty(ty), Some(Ty::Reference { .. }))) {
                        self.view_bindings.insert((analysis.func.id.clone(), place.local));
                    }
                }
                let mut queries = vec![place.clone()];
                if checked.is_some_and(|ty| matches!(analysis.type_context.try_ty(ty), Some(Ty::Reference { .. } | Ty::Pointer(_)))) {
                    let mut view = place.clone();
                    view.projection.push(Projection::Deref);
                    queries.push(view);
                }
                for query in queries {
                    self.path(analysis, table, &query);
                    let alternatives = provenance::resolve_places(&query, table, active);
                    assert!(alternatives.len() <= 32, "SLICE_REFERENCE HARNESS alias array cap32");
                    for alternative in &alternatives { self.path(analysis, table, alternative); }
                    eprintln!("SLICE_REFERENCE phase={phase} function={:?} carrier={query:?} ALL_aliases={alternatives:?}; provenance only, not access permission", analysis.func.id);
                }
                for loan in table.iter() {
                    if active.owner_contains(loan.id, place.local) || loan.initial_owner == place.local
                        || loan.diagnostic_place.local == place.local
                        || table.place_paths().place(loan.place_path).is_some_and(|path| path.local == place.local) {
                        selected.insert(loan.id);
                    }
                }
            }
            assert!(carriers.len() <= 16 && selected.len() <= 16, "SLICE_REFERENCE HARNESS carrier/loan array cap16");
            // Debug-only decoding exposes private role sets without a production
            // accessor. Cross-check the decoded union/storage against real APIs;
            // none of this observation feeds transfer, selection or permissions.
            let debug = format!("{active:?}");
            assert!(debug.len() <= 2_097_152, "SLICE_REFERENCE HARNESS private-role Debug byte cap2097152");
            let owner_rows = debug.split_once("owners: [").expect("actual LoanState Debug schema").1;
            for id in selected {
                self.loans.insert((analysis.func.id.clone(), id));
                self.budget();
                let loan = table.get(id).expect("actual LoanId");
                let row = owner_rows.split("LoanOwners {").nth(id.index() + 1).expect("actual indexed LoanOwners row");
                let mut union = HashSet::new();
                let mut roles = Vec::new();
                for role in ["storage", "contents", "raw", "raw_storage", "borrowed_raw", "returned"] {
                    let marker = format!("{role}: LocalSet {{ bits: [");
                    let bits = row.split_once(marker.as_str()).expect("actual role Debug schema").1.split_once(']').unwrap().0;
                    let mut owners = Vec::new();
                    if !bits.is_empty() {
                        for (index, bit) in bits.split(", ").enumerate() {
                            assert!(matches!(bit, "true" | "false"), "SLICE_REFERENCE HARNESS unknown Debug role bit");
                            if bit == "true" {
                                let local = Local(index);
                                assert!(index < analysis.func.local_decls.len(), "actual carrier local must exist");
                                owners.push(local);
                                union.insert(local);
                            }
                        }
                    }
                    assert!(owners.len() <= 16, "SLICE_REFERENCE HARNESS per-role owner array cap16");
                    if role == "storage" {
                        for index in 0..analysis.func.local_decls.len() {
                            assert_eq!(owners.contains(&Local(index)), active.storage_owner_contains(id, Local(index)));
                        }
                    }
                    roles.push((role, owners));
                }
                assert!(union.len() <= 16, "SLICE_REFERENCE HARNESS complete owner union cap16");
                assert_eq!(union, active.owners(id).iter().collect::<HashSet<_>>());
                for owner in &union {
                    assert!(active.owner_contains(id, *owner));
                    let root = Place { local: *owner, projection: Vec::new() };
                    self.path(analysis, table, &root);
                    let mut query = root;
                    if analysis.backend_contract.place_type_id(analysis.type_context, analysis.func, &query)
                        .is_some_and(|ty| matches!(analysis.type_context.try_ty(ty), Some(Ty::Reference { .. } | Ty::Pointer(_)))) {
                        query.projection.push(Projection::Deref);
                    }
                    let alternatives = provenance::resolve_places(&query, table, active);
                    assert!(alternatives.len() <= 32, "SLICE_REFERENCE HARNESS role-carrier alias array cap32");
                    for alternative in &alternatives { self.path(analysis, table, alternative); }
                    eprintln!("SLICE_REFERENCE phase={phase} function={:?} loan={id:?} roleCarrier={owner:?} ALL_aliases={alternatives:?}", analysis.func.id);
                }
                self.path(analysis, table, &loan.diagnostic_place);
                if let Some(indexed) = table.place_paths().place(loan.place_path) { self.path(analysis, table, indexed); }
                let alternatives = provenance::resolve_loan_places(id, table, active);
                assert!(alternatives.len() <= 32, "SLICE_REFERENCE HARNESS loan alias array cap32");
                for alternative in &alternatives { self.path(analysis, table, alternative); }
                eprintln!("SLICE_REFERENCE phase={phase} function={:?} actualLoan={loan:?} active={} returnedView={} ALL_roles={roles:?} ALL_loanAliases={alternatives:?}",
                    analysis.func.id, active.is_active(id), table.is_returned_view(id));
            }
        }
    }

    fn actual_operand_type(program: &MirProgram, function: &MirFunction, operand: &Operand) -> TypeId {
        let tc = &program.type_context;
        let checked = match operand {
            Operand::Copy(place) | Operand::Move(place) => program.backend_contract.place_type_id(tc, function, place),
            Operand::Constant(constant) => match constant {
                Constant::Int(_) => tc.id_for_type(&Type::I64),
                Constant::Float(_) => tc.id_for_type(&Type::F64),
                Constant::Bool(_) => tc.id_for_type(&Type::Bool),
                Constant::Char(_) => tc.id_for_type(&Type::Char),
                Constant::String(_) => tc.id_for_type(&Type::Str),
                Constant::Unit => tc.id_for_type(&Type::Unit),
                Constant::TypeId(id) => Some(*id),
                Constant::Callable(_) | Constant::VirtualTarget(_) | Constant::ErasedCall(_)
                | Constant::OwnedObjectCall(_) => None,
            },
        }.expect("SLICE_REFERENCE HARNESS actual operand type authority ABSENT; cannot substitute representative header");
        assert!(tc.type_id_tree_is_valid(checked), "SLICE_REFERENCE HARNESS invalid actual operand TypeId");
        checked
    }

    fn ancestry(function: &MirFunction, mut locals: HashSet<Local>) -> (BTreeSet<(usize, usize)>, BTreeSet<usize>) {
        let mut statements = BTreeSet::new();
        let mut calls = BTreeSet::new();
        loop {
            let before = (locals.len(), statements.len(), calls.len());
            for (block, data) in function.basic_blocks.iter().enumerate() {
                for (index, stmt) in data.statements.iter().enumerate() {
                    if let StatementKind::Assign(dest, value) = &stmt.kind {
                        if locals.contains(&dest.local) {
                            statements.insert((block, index));
                            let operands = integration_operands(value);
                            assert!(operands.len() <= 8, "SLICE_REFERENCE HARNESS assignment operand cap8");
                            locals.extend(operands.iter().map(|place| place.local));
                        }
                    }
                }
                if let Some(Terminator::Call { func, args, destination, .. }) = &data.terminator {
                    if locals.contains(&destination.local) {
                        calls.insert(block);
                        assert!(args.len() <= 8, "SLICE_REFERENCE HARNESS ancestry call argument cap8");
                        if let Operand::Copy(place) | Operand::Move(place) = func { locals.insert(place.local); }
                        locals.extend(args.iter().filter_map(|arg| match arg {
                            Operand::Copy(place) | Operand::Move(place) => Some(place.local), _ => None,
                        }));
                    }
                }
            }
            assert!(locals.len() <= 32 && statements.len() + calls.len() <= 64,
                "SLICE_REFERENCE HARNESS complete ancestry cap32 locals/64 sites; cannot omit required alternatives");
            if before == (locals.len(), statements.len(), calls.len()) { return (statements, calls); }
        }
    }

    fn control(fixture: &str, range: (usize, usize)) {
        for flag in ["ROCK_INTEGRATION_BORROW_TRACE", "ROCK_INTEGRATION_BORROW_DETAIL", "ROCK_OWNER_LOAN_TRACE",
            "ROCK_OWNER_RETURN_TRACE", "ROCK_FRESH_STDLIB_RETURN_TRACE", "ROCK_ALLOCATOR_EXECUTION_DEBUG", "ROCK_ALLOCATOR_EXECUTION_MIR"] {
            assert!(std::env::var_os(flag).is_none(), "SLICE_REFERENCE HARNESS unset observer flag {flag}");
        }
        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/integration.rs");
        let archive = agent_work_path("recovery/rock-completion-integration-8.log");
        let inputs = [&fixture_path, &archive].into_iter()
            .map(|path| (path.clone(), std::fs::read(path).unwrap(), integration_sha(path))).collect::<Vec<_>>();
        let source = integration_log_source(&archive, fixture);
        assert_eq!(source, integration_literal(&fixture_path, &format!("fn {fixture}()")), "unchanged original literal required");
        assert_eq!(&source[range.0..range.1], if fixture == "shared_index_reference_uses_index_authority" { "arr" } else { "values" });
        let source_receipt = agent_work_path("fixtures").join(format!("rock_slice_reference_source_{}_{fixture}.rk", std::process::id()));
        std::fs::OpenOptions::new().write(true).create_new(true).open(&source_receipt).unwrap().write_all(source.as_bytes()).unwrap();
        let hash = integration_sha(&source_receipt);
        if fixture == "shared_index_reference_uses_index_authority" {
            assert_eq!(hash, "d014593f9034a1c79aa7c3c3dbc92b2bc5ae831c4b1d1aa0e22ab0b00afd7c84");
        }
        eprintln!("SLICE_REFERENCE fixture={fixture} bytes={} SHA256_BEFORE={hash} exactSource={source:?} inputsBefore={:?} caps=8functions/16origins/16loans/64paths extras=64sites/32ancestryLocals/8arguments/16owners execution=NONE; frontend/mono/agreement failures are HARNESS, not Borrow RED",
            source.len(), inputs.iter().map(|(path, bytes, hash)| (path, bytes.len(), hash)).collect::<Vec<_>>());
        let (program, entry) = mutation_source_program(&format!("slice_reference_{fixture}"), &source, true);
        assert_eq!(integration_sha(&entry), hash);
        let agreement = crate::mir::agreement::check_mir_runtime_agreement(&program);
        eprintln!("SLICE_REFERENCE fixture={fixture} RuntimeAgreement={agreement:?}");
        assert!(agreement.is_clean() && agreement.object_contract_errors.is_empty() && agreement.erased_contract_errors.is_empty(), "SLICE_REFERENCE HARNESS agreement before Borrow");
        let outcome = BorrowChecker::run(&program);
        eprintln!("SLICE_REFERENCE fixture={fixture} BorrowOutcome={} execution=NONE", if outcome.is_ok() { "ACCEPTED" } else { "REJECTED" });
        if let Err(diagnostics) = &outcome {
            assert!(diagnostics.0.len() <= 16, "SLICE_REFERENCE HARNESS diagnostic cap16");
            for diagnostic in &diagnostics.0 { eprintln!("SLICE_REFERENCE actualDiagnostic={diagnostic:?}"); }
        }
        let tc = &program.type_context;
        let contract = &program.backend_contract;
        let origin_span = Span::new(entry.clone(), range.0, range.1);
        let roots = program.functions().filter_map(|(_, function)| {
            function.local_decls.iter().any(|decl| decl.source == LocalSource::Temporary
                && decl.span.as_ref() == Some(&origin_span)
                && matches!(tc.try_ty(decl.ty), Some(Ty::Reference { .. }))).then_some(function)
        }).collect::<Vec<_>>();
        assert_eq!(roots.len(), 1, "SLICE_REFERENCE HARNESS one actual diagnostic-slot function, not a guessed main");
        let root = roots[0];
        let mut seeds = root.local_decls.iter().enumerate().filter_map(|(index, decl)| {
            (decl.span.as_ref() == Some(&origin_span) && matches!(tc.try_ty(decl.ty), Some(Ty::Reference { .. }))).then_some(Local(index))
        }).collect::<HashSet<_>>();
        // Include the actual held view, including a move/copy from a source
        // temporary. Do not follow its later for_each/println consumers.
        loop {
            let before = seeds.len();
            for data in &root.basic_blocks {
                for stmt in &data.statements {
                    if let StatementKind::Assign(dest, value) = &stmt.kind {
                        if root.local_decls[dest.local.0].source == LocalSource::UserBinding
                            && contract.place_type_id(tc, root, dest).is_some_and(|ty| matches!(tc.try_ty(ty), Some(Ty::Reference { .. })))
                            && integration_operands(value).iter().any(|place| seeds.contains(&place.local)) { seeds.insert(dest.local); }
                    }
                }
                if let Some(term @ Terminator::Call { destination, .. }) = &data.terminator {
                    if root.terminator_origin(term).source_span() == Some(&origin_span)
                        && contract.place_type_id(tc, root, destination).is_some_and(|ty| matches!(tc.try_ty(ty), Some(Ty::Reference { .. }))) {
                        seeds.insert(destination.local);
                    }
                }
            }
            if seeds.len() == before { break; }
        }
        let lifetime = BorrowChecker::reference_return_summaries(&program);
        let values = BorrowChecker::native_return_loan_proof(&program);
        let mutations = NativeMutationTable::for_program(&program);
        let mut receipt = Receipt::default();
        let mut pending = VecDeque::from([(root.id.clone(), seeds)]);
        let mut source_trace = crate::mir::borrowck::return_sources::SourceReceipt::default();
        let mut replayed = HashSet::new();
        while let Some((id, seeds)) = pending.pop_front() {
            if !replayed.insert(id.clone()) { continue; }
            receipt.functions.insert(id.clone());
            receipt.budget();
            let function = program.functions.get(&id).expect("SLICE_REFERENCE HARNESS selected body ABSENT");
            let key = contract.function_bodies.get(&id).expect("SLICE_REFERENCE HARNESS canonical body key ABSENT");
            let header = contract.callable(key).expect("SLICE_REFERENCE HARNESS selected header ABSENT");
            assert_eq!(&header.key, key);
            assert!(matches!(&header.kind, crate::mir::MirCallableKind::LocalBody { function_id } if function_id == &id));
            assert!(header.signature.params.len() <= 8, "SLICE_REFERENCE HARNESS body header parameter cap8");
            eprintln!("SLICE_REFERENCE function={id:?} canonicalKey={key:?} declaration={header:?} body=PRESENT Lifetime={:?} ValueLoans={:?} LifetimeBody={} ValueLoansBody={}; None=UNKNOWN, not known-empty",
                lifetime.get(&id), values.summaries.get(&id), BorrowChecker::body_available_for_purpose(&program, &id, function, OriginPurpose::Lifetime),
                BorrowChecker::body_available_for_purpose(&program, &id, function, OriginPurpose::ValueLoans));
            for (label, summary) in [("Lifetime", lifetime.get(&id)), ("ValueLoans", values.summaries.get(&id))] {
                if let Some(summary) = summary { receipt.origins(function, label, summary); }
            }
            let (statements, calls) = ancestry(function, seeds);
            for (block, index) in &statements { receipt.sites.insert((id.clone(), *block, *index)); }
            for block in &calls { receipt.sites.insert((id.clone(), *block, function.basic_blocks[*block].statements.len())); }
            receipt.budget();
            let candidates = function.basic_blocks.iter().map(|data| data.terminator.as_ref()
                .map(|term| native_mutation_candidates(function, tc, contract, &mutations, term)).unwrap_or_default()).collect::<Vec<_>>();
            let (_, mutation_origins) = BorrowChecker::validate_moves_with_return_sources(function, tc, contract, &values.summaries, &mutations, Some(&values.sources));
            let mut analysis = ReferenceEscapeAnalysis::new(function, tc, contract, &lifetime, MissingSummaryMode::Conservative).with_return_sources(Some(&values.sources));
            analysis.mutation_origins = Some(&mutation_origins);
            let results = run_fixpoint(&analysis, function);
            let borrows = borrows::collect_function_borrows_with_return_sources(function, tc, contract, &values.summaries, Some(&values.sources));
            let table = LoanTable::from_borrows_with_paths(&borrows, paths::PlacePathTable::for_function(function));
            let locations = LoanLocationIndex::for_function(&table, function);
            let live = liveness::compute_reference_liveness_with_mutations(function, tc, contract, &table, &candidates);
            let entries = liveness::compute_active_loan_entries_with_return_sources(function, tc, contract, &table, &locations, &live, &values.summaries, &candidates, Some(&values.sources));
            let active_analysis = liveness::ActiveLoanAnalysis::new(tc, contract, &table, &locations, &live, &values.summaries).with_mutations(&candidates).with_return_sources(Some(&values.sources));
            for (block, data) in function.basic_blocks.iter().enumerate() {
                if !results.reachable[block] { continue; }
                let mut state = results.entry_sets[block].clone();
                let mut active = entries[block].clone();
                for (index, stmt) in data.statements.iter().enumerate() {
                    let selected = statements.contains(&(block, index));
                    let mut places = Vec::new();
                    if selected {
                        let StatementKind::Assign(dest, value) = &stmt.kind else { unreachable!(); };
                        places.push(dest.clone());
                        places.extend(integration_operands(value));
                        let checked = contract.place_type_id(tc, function, dest);
                        eprintln!("SLICE_REFERENCE function={id:?} location=({block},{index}) JOINED_BLOCK_ENTRY_replayed=true statement={stmt:?} checkedDestination={checked:?} safeStorageEligible={:?} temporaryClassification={:?}",
                            checked.map(|ty| return_type_needs_escape_check(tc, contract, ty)), analysis.temporary_origins_for_assignment(&state, dest, value));
                        if let Rvalue::Ref(_, place) = value {
                            receipt.origins(function, "borrowedInput", &analysis.origins_for_borrowed_place(&state, place));
                            if place.projection.is_empty() && checked_place_type(tc, contract, function, place)
                                .is_some_and(|ty| matches!(ty, Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::Slice(_)))) {
                                receipt.metadata_borrows += 1;
                            }
                        } else {
                            let operands = match value {
                                Rvalue::Use(operand) | Rvalue::Cast(operand, _) | Rvalue::Object(operand, _)
                                | Rvalue::Package(operand, _) | Rvalue::UnaryOp(_, operand) => vec![operand],
                                Rvalue::Aggregate(_, operands) => operands.iter().collect(),
                                Rvalue::BinaryOp(_, left, right) => vec![left, right],
                                Rvalue::Discriminant(_) | Rvalue::Closure(_) => Vec::new(),
                                Rvalue::Ref(_, _) => unreachable!(),
                            };
                            assert!(operands.len() <= 8, "SLICE_REFERENCE HARNESS exact operand array cap8");
                            for operand in operands {
                                receipt.origins(function, "actualOperandInput", &analysis.origins_for_operand(&state, operand));
                            }
                        }
                        receipt.snapshot("BEFORE_STATEMENT", &analysis, &state, &table, &active, &places);
                    }
                    analysis.apply_statement(&mut state, stmt);
                    integration_loan_after_statement(&mut active, function, &program, stmt, block, index, &locations, &table, &live);
                    if selected { receipt.snapshot("AFTER_EXACT_STATEMENT_TRANSFER", &analysis, &state, &table, &active, &places); }
                }
                if id != root.id && matches!(data.terminator, Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })) {
                    receipt.sites.insert((id.clone(), block, data.statements.len()));
                    receipt.budget();
                    let returned = Place { local: Local(0), projection: Vec::new() };
                    let projected = contract.place_type_id(tc, function, &returned).expect("actual checked native ReturnPlace");
                    assert_eq!(projected, function.ret_type);
                    assert!(return_type_needs_escape_check(tc, contract, projected), "native reference return remains protected");
                    eprintln!("SLICE_REFERENCE function={id:?} nativeReturnBlock={block} actualReturn={:?} projectedReturnType={projected:?} semanticReturn={:?} safeStorageEligible=true", data.terminator, tc.type_for(projected));
                    receipt.snapshot("NATIVE_RETURN_AFTER_EXACT_STATEMENTS_FROM_JOINED_ENTRY", &analysis, &state, &table, &active, &[returned]);
                }
                if !calls.contains(&block) { continue; }
                let Some(term @ Terminator::Call { func, args, destination, .. }) = &data.terminator else { unreachable!(); };
                assert!(args.len() <= 8 && candidates[block].len() <= 16, "SLICE_REFERENCE HARNESS call args8/mutation candidates16");
                let mut places = vec![destination.clone()];
                if let Operand::Copy(place) | Operand::Move(place) = func { places.push(place.clone()); }
                for arg in args {
                    receipt.origins(function, "actualOperandInput", &analysis.origins_for_operand(&state, arg));
                    if let Operand::Copy(place) | Operand::Move(place) = arg { places.push(place.clone()); }
                }
                let checked = contract.place_type_id(tc, function, destination);
                eprintln!("SLICE_REFERENCE function={id:?} location=({block},terminator) actualCall={term:?} projectedResult={checked:?} safeStorageEligible={:?} temporaryClassificationBeforeNativeMutation={:?} actualMutationCandidates={:?}",
                    checked.map(|ty| return_type_needs_escape_check(tc, contract, ty)), analysis.temporary_origins_for_call(&state, func, args, destination), candidates[block]);
                receipt.origins(function, "callReturnBeforeNativeMutation", &analysis.call_return_origins(&state, func, args, destination));
                receipt.snapshot("BEFORE_TERMINATOR", &analysis, &state, &table, &active, &places);
                if let Operand::Constant(Constant::Callable(MirCallable::Resolved(call_key))) = func {
                    let declaration = contract.callable(call_key);
                    eprintln!("SLICE_REFERENCE function={id:?} selectedKey={call_key:?} exactHeader={declaration:?}; None=ABSENT, never inferred from spelling");
                    let declaration = declaration.expect("SLICE_REFERENCE HARNESS producing header ABSENT");
                    assert_eq!(&declaration.key, call_key);
                    assert!(declaration.signature.params.len() <= 8, "SLICE_REFERENCE HARNESS header parameter cap8");
                    assert_eq!(declaration.signature.params.len(), args.len(), "actual canonical arity");
                    let actual_params = args.iter().map(|arg| actual_operand_type(&program, function, arg)).collect::<Vec<_>>();
                    let actual_ret = checked.expect("SLICE_REFERENCE HARNESS actual projected result type ABSENT");
                    assert!(tc.type_id_tree_is_valid(actual_ret), "SLICE_REFERENCE HARNESS invalid actual result TypeId");
                    if id == root.id
                        && matches!(&declaration.kind, crate::mir::MirCallableKind::LocalBody { .. })
                        && matches!(tc.try_ty(actual_ret), Some(Ty::Reference { .. })) {
                    values.sources.trace_call(&program, function, block, &mut source_trace);
                    }
                    if let MirCallableKey::Intrinsic(intrinsic) = call_key {
                        assert!(matches!(&declaration.kind, crate::mir::MirCallableKind::Intrinsic(id) if id == intrinsic),
                            "exact typed intrinsic key/kind authority");
                        // Builder stores the first signature for each intrinsic key;
                        // MIR intrinsic dispatch uses actual projected operand/result
                        // IDs instead. Keep both receipts, never equate instantiations.
                        eprintln!("SLICE_REFERENCE function={id:?} actualIntrinsicId={intrinsic:?} representativeHeaderRet={:?} representativeHeaderRetSemantic={:?} representativeHeaderParams={:?} representativeHeaderParamSemantics={:?} actualDestination={actual_ret:?} actualDestinationSemantic={:?} actualParams={actual_params:?} actualParamSemantics={:?} RuntimeAgreement=CLEAN authority=backend_contract.place_type_id/TypeView; operand/result types only, no synthetic ABI signature or header/destination unification",
                            declaration.signature.ret.semantic_ty, tc.type_for(declaration.signature.ret.semantic_ty),
                            declaration.signature.params, declaration.signature.params.iter().map(|param| tc.type_for(param.semantic_ty)).collect::<Vec<_>>(),
                            tc.type_for(actual_ret), actual_params.iter().map(|ty| tc.type_for(*ty)).collect::<Vec<_>>());
                        // These are intrinsic-ID/type-shape observations, not native
                        // callable exceptions or access/ownership permissions.
                        let view = TypeView::new(tc);
                        match intrinsic {
                            MirIntrinsicId::ArrayRefToSlice => {
                                assert_eq!(actual_params.len(), 1);
                                let Ty::Reference { mutable: input_mutable, inner: array } = view.ty(actual_params[0]) else { panic!("checked array borrow required"); };
                                let Ty::Array { inner: element, .. } = view.ty(*array) else { panic!("checked fixed-array backing required"); };
                                let Ty::Reference { mutable: output_mutable, inner: slice } = view.ty(actual_ret) else { panic!("checked slice reference result required"); };
                                let Ty::Slice(result_element) = view.ty(*slice) else { panic!("checked slice result required"); };
                                assert_eq!(element, result_element, "exact actual element TypeIds; no raw-cast equality");
                                assert_eq!(input_mutable, output_mutable, "actual conversion preserves reference mutability");
                                receipt.conversions += 1;
                            }
                            MirIntrinsicId::ArrPtr | MirIntrinsicId::ArrayLen => {
                                assert_eq!(actual_params.len(), 1);
                                let input = match view.ty(actual_params[0]) {
                                    Ty::Reference { inner, .. } => view.ty(*inner), input => input,
                                };
                                let element = match input {
                                    Ty::Slice(element) | Ty::Array { inner: element, .. } => *element,
                                    Ty::Str => view.id_for_type(&Type::U8).expect("checked string byte TypeId"),
                                    _ => panic!("SLICE_REFERENCE HARNESS intrinsic array/slice input authority lost"),
                                };
                                if *intrinsic == MirIntrinsicId::ArrPtr {
                                    assert_eq!(view.ty(actual_ret), &Ty::Pointer(element), "actual ArrPtr element/result authority");
                                } else {
                                    assert_eq!(view.ty(actual_ret), &Ty::I64, "actual ArrayLen result authority");
                                }
                            }
                            MirIntrinsicId::PtrOffset => {
                                assert_eq!(actual_params.len(), 2);
                                assert!(matches!(view.ty(actual_params[0]), Ty::Pointer(_)), "actual PtrOffset pointer input");
                                assert_eq!(view.ty(actual_params[1]), &Ty::I64, "actual PtrOffset offset type");
                                assert_eq!(actual_ret, actual_params[0], "exact actual PtrOffset result TypeId");
                            }
                            MirIntrinsicId::BorrowSlice | MirIntrinsicId::BorrowSliceMut => {
                                assert_eq!(actual_params.len(), 2);
                                let Ty::Pointer(element) = view.ty(actual_params[0]) else { panic!("checked BorrowSlice pointer input"); };
                                assert_eq!(view.ty(actual_params[1]), &Ty::I64, "actual BorrowSlice length type");
                                let Ty::Reference { mutable, inner } = view.ty(actual_ret) else { panic!("checked BorrowSlice reference result"); };
                                assert_eq!(view.ty(*inner), &Ty::Slice(*element), "actual BorrowSlice element/result authority");
                                assert_eq!(*mutable, *intrinsic == MirIntrinsicId::BorrowSliceMut, "typed intrinsic result mutability");
                            }
                            _ => {
                                // Unhandled intrinsic instantiations stay strict;
                                // observing IDs is not proof of semantic agreement.
                                assert_eq!(actual_ret, declaration.signature.ret.semantic_ty, "SLICE_REFERENCE HARNESS unhandled intrinsic result authority; request scope rather than guess");
                                for (actual, param) in actual_params.iter().zip(&declaration.signature.params) {
                                    assert_eq!(*actual, param.semantic_ty, "SLICE_REFERENCE HARNESS unhandled intrinsic parameter authority; request scope rather than guess");
                                }
                            }
                        }
                    } else {
                        assert_eq!(actual_ret, declaration.signature.ret.semantic_ty, "actual checked producing result");
                        for (actual, param) in actual_params.iter().zip(&declaration.signature.params) {
                            assert_eq!(*actual, param.semantic_ty, "exact actual native/header parameter TypeId");
                        }
                    }
                    if let crate::mir::MirCallableKind::LocalBody { function_id } = &declaration.kind {
                        receipt.functions.insert(function_id.clone());
                        receipt.budget();
                        let body = program.functions.get(function_id);
                        eprintln!("SLICE_REFERENCE function={id:?} selectedBodyId={function_id:?} bodyPresent={} Lifetime={:?} ValueLoans={:?}; absent body/summary=UNKNOWN", body.is_some(), lifetime.get(function_id), values.summaries.get(function_id));
                        if let Some(body) = body {
                            assert_eq!(contract.function_bodies.get(function_id), Some(call_key));
                            for (label, summary) in [("Lifetime", lifetime.get(function_id)), ("ValueLoans", values.summaries.get(function_id))] {
                                if let Some(summary) = summary { receipt.origins(body, label, summary); }
                            }
                            if matches!(tc.try_ty(declaration.signature.ret.semantic_ty), Some(Ty::Reference { .. })) {
                                pending.push_back((function_id.clone(), HashSet::from([Local(0)])));
                            }
                        }
                    } else {
                        eprintln!("SLICE_REFERENCE function={id:?} selectedBody=ABSENT(non-native-body callable) Lifetime=UNKNOWN ValueLoans=UNKNOWN intrinsicIdentity={call_key:?}");
                    }
                } else {
                    eprintln!("SLICE_REFERENCE function={id:?} staticHeader=ABSENT nonstaticOperand={func:?}; no fabricated callable/body");
                }
                if let Some(bindings) = mutation_origins.get(&block) {
                    assert!(bindings.len() <= 16, "SLICE_REFERENCE HARNESS mutation origin binding cap16");
                    analysis.apply_mutation_origins(&mut state, bindings);
                }
                receipt.origins(function, "callReturnAfterNativeMutation", &analysis.call_return_origins(&state, func, args, destination));
                analysis.apply_terminator(&mut state, term);
                // Invoke production block transfer from its original joined entry,
                // not the already replayed pre-call state (no double statement transfer).
                let mut exact = entries[block].clone();
                active_analysis.apply_block(&mut exact, function, crate::mir::BasicBlockId(block));
                receipt.snapshot("AFTER_EXACT_BLOCK_TRANSFER_FROM_JOINED_ENTRY", &analysis, &state, &table, &exact, &places);
                assert_eq!(state, results.exit_sets[block], "actual Lifetime block transfer must agree with fixpoint exit");
            }
        }
        receipt.budget();
        for (path, bytes, before_hash) in &inputs {
            assert_eq!(&std::fs::read(path).unwrap(), bytes, "original input bytes unchanged");
            assert_eq!(&integration_sha(path), before_hash, "original input SHA unchanged");
        }
        for path in [&entry, &source_receipt] {
            assert_eq!(std::fs::read(path).unwrap(), source.as_bytes());
            assert_eq!(integration_sha(path), hash);
        }
        eprintln!("SLICE_REFERENCE fixture={fixture} SHA256_AFTER={hash} ALL_original_source_archive_fixture_bytes_preserved=true functions={} origins={} loans={} paths={} sites={} actualConversions={} actualMetadataBorrows={} actualArrayBindings={:?} actualViewBindings={:?} execution=NONE",
            receipt.functions.len(), receipt.origins.len(), receipt.loans.len(), receipt.paths.len(), receipt.sites.len(), receipt.conversions, receipt.metadata_borrows, receipt.array_bindings, receipt.view_bindings);
        assert!(receipt.conversions > 0 && receipt.metadata_borrows > 0 && !receipt.array_bindings.is_empty() && !receipt.view_bindings.is_empty(),
            "SLICE_REFERENCE HARNESS required primitive-view ancestry missing; do not generalize shared-index evidence or claim security RED");
        if let Err(diagnostics) = &outcome {
            assert_eq!(diagnostics.0.len(), 1, "SLICE_REFERENCE HARNESS expected only the original Borrow frontier, not incidental errors");
            let diagnostic = &diagnostics.0[0];
            assert_eq!(diagnostic.code, Some(DiagnosticCode::Borrow));
            assert_eq!(diagnostic.message, "Cannot store reference to temporary value because it does not live long enough");
            assert_eq!(diagnostic.primary.as_ref().map(|label| &label.span), Some(&origin_span));
        }
        outcome.expect("SLICE_REFERENCE CLASSIFICATION RED: original array-backed held reference storage must pass Borrow; range bounds trap remains a later coordinator runtime assertion, never executed here");
    }

    #[test]
    fn slice_reference_temporary_shared_index() {
        control("shared_index_reference_uses_index_authority", (56, 59));
    }

    #[test]
    fn slice_reference_temporary_native_range_bounds_trap_source() {
        control("test_native_range_out_of_bounds_traps", (50, 56));
    }

    #[test]
    fn slice_reference_temporary_full_for_each_source() {
        control("test_stdlib_for_each_owned_and_borrowed_collections", (258, 264));
    }
}

#[test]
#[ignore = "historical seven-source rejection oracle; expected to fail after scalar diagnostic repair; never executes"]
fn capture_integration_borrow_storage_and_owner_authority() {
    assert!(integration_enabled(), "set ROCK_INTEGRATION_BORROW_TRACE=1");
    assert!(std::env::var_os("ROCK_INTEGRATION_BORROW_DETAIL").is_none() || integration_effects_detail(),
        "detail mode must be unset or exactly effects");
    let detail_archives = if integration_effects_detail() {
        eprintln!("INTEGRATION_BORROW_TRACE detail=effects: detailed selection ONLY three original negatives; four positive sources retain exact pipeline/diagnostic oracles; caps=24/128/192/256/512 bodyCaps=2/64/32 (total, not per body)");
        ["recovery/integration-borrow-storage-owner-authority-1.log",
            "recovery/integration-borrow-storage-owner-authority-2.log"].into_iter()
            .map(|path| { let path = agent_work_path(path); let bytes = std::fs::read(&path).unwrap(); (path, bytes) }).collect::<Vec<_>>()
    } else { Vec::new() };
    for flag in [
        "ROCK_OWNER_LOAN_TRACE", "ROCK_OWNER_RETURN_TRACE", "ROCK_OWNER_TRACE_SOURCE", "ROCK_OWNER_TRACE_STDLIB",
        "ROCK_FRESH_STDLIB_RETURN_TRACE", "ROCK_ALLOCATOR_EXECUTION_DEBUG", "ROCK_ALLOCATOR_EXECUTION_MIR",
        "ROCK_ALLOCATOR_FRONTEND_ONLY", "ROCK_ALLOCATOR_TRACE_STDLIB", "ROCK_STATIC_CAPTURE_TRACE",
    ] {
        assert!(std::env::var_os(flag).is_none(), "unset other observer/debug/artifact flag {flag}");
    }
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let integration_file = workspace.join("lib/tests/integration.rs");
    let owner_file = workspace.join("lib/tests/owned_object_calls.rs");
    let log = agent_work_path("recovery/rock-completion-integration-6.log");
    let artifact = agent_work_path("artifacts/stdlib/stdlib.rkca");
    let artifact_bytes = std::fs::read(&artifact).unwrap();
    assert_eq!(artifact_bytes.len(), 803478, "approved current primary7 artifact length");
    let stdlib_root = workspace.join("stdlib");
    let stdlib_inputs = fresh_stdlib_inputs(&stdlib_root);
    eprintln!("INTEGRATION_BORROW_TRACE approved-artifact={artifact:?} bytes={} SHA256={} stdlib-source-snapshot-files={}; before/after complete filename+byte equality enforced", artifact_bytes.len(), integration_sha(&artifact), stdlib_inputs.len());
    let integration_bytes = std::fs::read(&integration_file).unwrap();
    let owner_bytes = std::fs::read(&owner_file).unwrap();
    let log_bytes = std::fs::read(&log).unwrap();
    let directory = agent_work_path("fixtures").join(format!("rock_integration_borrow_authority_{}", std::process::id()));
    std::fs::create_dir(&directory).expect("new isolated observer directory, never overwrite an archive");
    let cases = [
        ("shared_index_reference_uses_index_authority", integration_log_source(&log, "shared_index_reference_uses_index_authority"), false),
        ("test_fixed_array_borrow_coercion_preserves_original_storage_alias", integration_log_source(&log, "test_fixed_array_borrow_coercion_preserves_original_storage_alias"), false),
        ("test_fold_loop_utf8_cursor_resume_and_empty", integration_log_source(&log, "test_fold_loop_utf8_cursor_resume_and_empty"), false),
        ("test_hkt_standalone_and_receiver_operations", integration_literal(&integration_file, "fn test_hkt_standalone_and_receiver_operations()"), false),
        ("custom_owner_cannot_return_an_erased_payload_borrowing_a_local", format!("{}{}{}\n{}",
            integration_literal(&owner_file, "const OWNER:"), integration_literal(&owner_file, "const UNIQUE:"),
            integration_literal(&owner_file, "const BORROWED_PAYLOAD:"), integration_literal(&owner_file, "fn custom_owner_cannot_return_an_erased_payload_borrowing_a_local()")), true),
        ("vec_reference_payload_cannot_escape_local", r#"
escape: () -> Vec &I64
escape = ->
    answer = 42
    mut values: Vec &I64 = Vec::new!
    values.push &answer
    values
main = !->
    values = escape!
    values.len!.println!
"#.to_string(), true),
        ("vec_borrowed_callable_payload_cannot_escape_local", r#"
escape: () -> Vec (() -> I64)
escape = ->
    answer = 42
    view = &answer
    callback = -> *view
    mut values: Vec (() -> I64) = Vec::new!
    values.push callback
    values
main = !->
    values = escape!
    values.len!.println!
"#.to_string(), true),
    ];
    *integration_trace().lock().unwrap() = IntegrationTrace::default();
    let mut failures = Vec::new();
    for (name, source, negative) in cases {
        let output = directory.join(name);
        std::fs::create_dir(&output).unwrap();
        let entry = output.join("main.rk");
        std::fs::write(&entry, &source).unwrap();
        match name {
            "shared_index_reference_uses_index_authority" => {
                assert_eq!(source, integration_literal(&integration_file, "fn shared_index_reference_uses_index_authority()"));
                assert_eq!(integration_sha(&entry), "d014593f9034a1c79aa7c3c3dbc92b2bc5ae831c4b1d1aa0e22ab0b00afd7c84");
            }
            "test_fixed_array_borrow_coercion_preserves_original_storage_alias" => {
                assert_eq!(source, integration_literal(&integration_file, "fn test_fixed_array_borrow_coercion_preserves_original_storage_alias()"));
                assert_eq!(integration_sha(&entry), "8e64a23fe8f1e0d060a36d25bfb19997736364e787723b9c071235699a0fee09");
            }
            "test_fold_loop_utf8_cursor_resume_and_empty" => {
                assert_eq!(source, integration_literal(&integration_file, "fn test_fold_loop_utf8_cursor_resume_and_empty()"));
                assert_eq!(integration_sha(&entry), "78c14a2a854a08bd0a76f80c93af2ffe3284f3d1b52c2ee4753fecd8c5300d76");
            }
            "test_hkt_standalone_and_receiver_operations" => {
                assert_eq!(integration_sha(&entry), "8b0c3e611ea091aacccb091132b65835cd9af9c34bf632d08419c8bf42b450a6");
            }
            "custom_owner_cannot_return_an_erased_payload_borrowing_a_local" => {
                assert_eq!(integration_sha(&entry), "a0daeb712f1e1a2b766c728ba75f5666de04151f52b5217002ec58602d770a73");
            }
            "vec_reference_payload_cannot_escape_local" => {
                assert_eq!(integration_sha(&entry), "d9c06a955035df92d0f7ddfa4230ab58411be5fdb0c7ed312e8e76312103f174");
            }
            "vec_borrowed_callable_payload_cannot_escape_local" => {
                assert_eq!(integration_sha(&entry), "a8fe3a5aa3aa37a4786f06aa99cba9d595828769e1c95290e7694eabf2290ab7");
            }
            _ => {}
        }
        let custom_owner = name == "custom_owner_cannot_return_an_erased_payload_borrowing_a_local";
        let config = crate::Config {
            entry_file: entry.clone(), output_dir: output.clone(),
            current_crate_name: custom_owner.then(|| "owned_calls".into()),
            no_std: custom_owner, no_prelude: custom_owner, no_link: true,
            extern_artifacts: if custom_owner { Vec::new() } else { vec![("stdlib".into(), artifact.clone())] },
            ..Default::default()
        };
        assert!(config.debug_print.is_empty() && config.source_providers.is_empty());
        eprintln!("INTEGRATION_BORROW_TRACE input case={name:?} bytes={} SHA256={} exactSource={source:?} actualConfig={config:?}", source.len(), integration_sha(&entry));
        std::fs::write(output.join("observer-config.txt"), format!("{config:#?}\n")).unwrap();
        {
            let mut trace = integration_trace().lock().unwrap();
            trace.case = name.into(); trace.entry = entry.clone();
            trace.inputs = stdlib_inputs.clone(); trace.inputs.insert(entry.clone(), source.as_bytes().to_vec());
            trace.negative_range = negative.then(|| (source.find("escape =").unwrap(), source.find("main =").unwrap()));
            trace.selected.clear(); trace.targets.clear(); trace.gates.clear(); trace.checker_seen = false;
            trace.detail_bodies.clear();
        }
        let outcome = integration_pipeline(&config);
        assert_eq!(std::fs::read(&entry).unwrap(), source.as_bytes(), "fixture bytes unchanged");
        assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes, "approved artifact unchanged");
        assert_eq!(fresh_stdlib_inputs(&stdlib_root), stdlib_inputs, "all stdlib source filenames and bytes unchanged before/after each case");
        for (path, bytes) in &detail_archives { assert_eq!(&std::fs::read(path).unwrap(), bytes, "original observer FAIL archive unchanged"); }
        let diagnostics = outcome.err();
        if let Some(diagnostics) = &diagnostics {
            assert!(diagnostics.0.len() <= 128, "INTEGRATION_BORROW_TRACE diagnostic budget overflow");
            for diagnostic in &diagnostics.0 {
                eprintln!("INTEGRATION_BORROW_TRACE actual-diagnostic case={name:?} code={:?} severity={:?} location={:?} primary={:?} message={:?}", diagnostic.code, diagnostic.severity, diagnostic.location, diagnostic.primary.as_ref().map(|label| &label.span), diagnostic.message);
            }
        } else {
            eprintln!("INTEGRATION_BORROW_TRACE actual-diagnostics case={name:?} [] accepted=true unsafeNegative={negative}; ACTUAL ACCEPTED RETURN/PATH/LOAN AUTHORITY recorded, NOT a green escape fixture");
        }
        let mut trace = integration_trace().lock().unwrap();
        for (key, row) in trace.records.iter().filter(|(key, _)| key.starts_with(&format!("{name}|"))) {
            eprintln!("INTEGRATION_BORROW_TRACE record key={key:?} {row}");
        }
        if !trace.checker_seen { failures.push(format!("{name}: did not reach actual BorrowChecker (invalid source or earlier stage)")); }
        if negative {
            if trace.gates.is_empty() { failures.push(format!("{name}: actual owner/container return validation not observed")); }
            if !diagnostics.as_ref().is_some_and(|diagnostics| diagnostics.0.iter().any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow) && diagnostic.primary.is_some())) {
                failures.push(format!("{name}: UNSAFE-ACCEPTED or non-Borrow rejection; inspect actual return eligibility/origin paths/native summaries/loans above"));
            }
            if diagnostics.as_ref().is_some_and(|diagnostics| diagnostics.0.iter().any(|diagnostic| diagnostic.code != Some(DiagnosticCode::Borrow))) {
                failures.push(format!("{name}: unexpected non-Borrow stage/diagnostic, not faithful ownership reproduction"));
            }
            let origin_range = match name {
                "custom_owner_cannot_return_an_erased_payload_borrowing_a_local" => (1928, 1934),
                "vec_reference_payload_cannot_escape_local" => (40, 46),
                "vec_borrowed_callable_payload_cannot_escape_local" => (63, 67),
                _ => unreachable!(),
            };
            if !diagnostics.as_ref().is_some_and(|diagnostics| diagnostics.0.iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "Cannot return reference to local value because it does not live long enough"
                    && diagnostic.primary.as_ref().is_some_and(|label| label.span.file_path == entry
                        && (label.span.start, label.span.end) == origin_range)
            })) {
                failures.push(format!("{name}: expected actual owning-return rejection at original Local origin {origin_range:?}, not incidental Borrow"));
            }
        } else {
            let temporary = "Cannot store reference to temporary value because it does not live long enough";
            let return_local = "Cannot return reference to local value because it does not live long enough";
            let mut actual = diagnostics.as_ref().map(|diagnostics| diagnostics.0.iter().map(|diagnostic| {
                let span = diagnostic.primary.as_ref().map(|label| &label.span);
                let location = span.map(|span| (if span.file_path == entry { "entry" } else if span.file_path == stdlib_root.join("vec.rk") { "stdlib/vec.rk" } else { "unexpected-source" }, span.start, span.end));
                (format!("{:?}", diagnostic.code), format!("{:?}", diagnostic.severity), diagnostic.message.clone(), location)
            }).collect::<Vec<_>>()).unwrap_or_default();
            let mut expected = match name {
                "shared_index_reference_uses_index_authority" => vec![("Some(Borrow)", temporary, Some(("entry", 56, 59)))],
                "test_fixed_array_borrow_coercion_preserves_original_storage_alias" => vec![("Some(Borrow)", temporary, Some(("entry", 127, 128)))],
                "test_fold_loop_utf8_cursor_resume_and_empty" => vec![("Some(Borrow)", temporary, Some(("entry", 28, 36))), ("Some(Borrow)", temporary, Some(("entry", 127, 135))), ("Some(Borrow)", temporary, Some(("entry", 392, 394)))],
                "test_hkt_standalone_and_receiver_operations" => vec![("Some(Internal)", return_local, None), ("Some(Borrow)", return_local, Some(("stdlib/vec.rk", 11676, 11683)))],
                _ => unreachable!(),
            }.into_iter().map(|(code, message, location)| (code.to_string(), "Error".to_string(), message.to_string(), location)).collect::<Vec<_>>();
            actual.sort(); expected.sort();
            if actual != expected { failures.push(format!("{name}: exact integration6 Borrow oracle changed actual={actual:?} expected={expected:?}")); }
            if diagnostics.as_ref().is_some_and(|diagnostics| diagnostics.0.iter().any(|diagnostic| diagnostic.primary.is_none() && !matches!(diagnostic.location, crate::diagnostic::DiagnosticLocation::Toolchain))) {
                failures.push(format!("{name}: unexpected missing-source location"));
            }
        }
        eprintln!("INTEGRATION_BORROW_TRACE case-end={name:?} functions={} records={} originEntries={} types={} layoutEdges={} sourceSpans={} actualLoanRows={} caps=24/128/{}/256/512 extras=128/128; stdlib/entry/artifact byte equality confirmed, no execution", trace.functions.len(), trace.records.len(), trace.origins.len(), trace.types.len(), trace.edges.len(), trace.source_spans.len(), trace.loan_rows.len(), integration_origin_cap());
        if integration_effects_detail() {
            eprintln!("INTEGRATION_BORROW_TRACE detail-body-accounting case={name:?} cumulativeNativeBodies={} cumulativeBodyStatements={} cumulativeBodyCalls={} caps=2/64/32; arrays individually bounded and records/types/origins charged to displayed mode caps", trace.detail_body_keys.len(), trace.detail_statements.len(), trace.detail_calls.len());
        }
        // Keep cumulative counters/records across cases, but release source bytes.
        trace.inputs.clear();
    }
    assert_eq!(std::fs::read(&integration_file).unwrap(), integration_bytes);
    assert_eq!(std::fs::read(&owner_file).unwrap(), owner_bytes);
    assert_eq!(std::fs::read(&log).unwrap(), log_bytes);
    for (path, bytes) in &detail_archives { assert_eq!(&std::fs::read(path).unwrap(), bytes, "original observer FAIL archive unchanged"); }
    if integration_effects_detail() {
        assert_eq!(integration_trace().lock().unwrap().detail_body_keys.len(), 2,
            "detail observation requires both actual current-compilation native specializations");
    }
    assert!(failures.is_empty(), "INTEGRATION_BORROW_TRACE strict per-case failures (all available actual diagnostics printed first): {failures:?}");
}

static CHECKER_SEEN: AtomicBool = AtomicBool::new(false);
static SELECTION: OnceLock<TraceSelection> = OnceLock::new();
static CALL_OBSERVATIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

// Original source diagnostic ranges only; never function/loan/type authority.
const PRIMARY_SPANS: [(usize, usize); 6] = [
    (2747, 2754), (2808, 2817), (2985, 3000),
    (3048, 3059), (3453, 3458), (3476, 3481),
];
const DEFAULT_SPANS: [(usize, usize); 8] = [
    (2567, 2583), (2667, 2701), (2747, 2754), (2808, 2817),
    (2985, 3000), (3048, 3059), (3453, 3458), (3476, 3481),
];

struct TraceSelection {
    path: PathBuf,
    spans: Vec<(usize, usize)>,
}

fn selection() -> &'static TraceSelection {
    SELECTION.get_or_init(|| {
        let path = PathBuf::from(
            std::env::var_os("ROCK_OWNER_TRACE_SOURCE").expect("explicit trace source path"),
        );
        let spans = match std::env::var("ROCK_OWNER_LOAN_TRACE_SPANS") {
            Ok(value) => value.split(',').map(|range| {
                let (start, end) = range.trim().split_once("..").expect("trace range start..end");
                let start = start.parse::<usize>().expect("trace range start");
                let end = end.parse::<usize>().expect("trace range end");
                assert!(start < end, "nonempty diagnostic trace range");
                (start, end)
            }).collect(),
            Err(std::env::VarError::NotPresent) => DEFAULT_SPANS.to_vec(),
            Err(error) => panic!("invalid trace ranges: {error}"),
        };
        TraceSelection { path, spans }
    })
}

fn overlaps(span: &Span, range: (usize, usize)) -> bool {
    span.start < range.1 && range.0 < span.end
}

pub(super) fn enabled_at(span: Option<&Span>) -> bool {
    if !enabled() {
        return false;
    }
    let selection = selection();
    span.is_some_and(|span| {
        span.file_path == selection.path
            && selection.spans.iter().any(|range| overlaps(span, *range))
    })
}

fn enabled() -> bool {
    std::env::var("ROCK_OWNER_LOAN_TRACE").as_deref() == Ok("1")
}

pub(super) fn enabled_for(func: &MirFunction) -> bool {
    if !enabled() {
        return false;
    }
    func.basic_blocks.iter().any(|block| {
        block.statements.iter().any(|stmt| enabled_at(stmt.span.as_ref()))
            || block.terminator.as_ref().is_some_and(|term| {
                enabled_at(func.terminator_origin(term).source_span())
            })
    })
}

fn loan_summary(active: &LoanState) -> Vec<(crate::ids::LoanId, Vec<Local>)> {
    active.active_ids().map(|id| {
        (id, active.owners(id).iter().collect())
    }).collect()
}

pub(super) fn state(stage: &str, func: &MirFunction, span: Option<&Span>, init: &InitMap, active: &LoanState) {
    if !enabled_at(span) {
        return;
    }
    eprintln!(
        "OWNER_LOAN_TRACE {} function={:?} span={:?} init={:?} active_ids_and_owner_union={:?}",
        stage, func.id, span, init, loan_summary(active),
    );
    if span.is_some_and(|span| PRIMARY_SPANS.iter().any(|range| overlaps(span, *range))) {
        // Existing Debug retains the actual private owner categories, without guessing them.
        eprintln!("OWNER_LOAN_TRACE focused-owner-categories stage={} function={:?} span={:?} state={:?}", stage, func.id, span, active);
    }
}

pub(super) fn program(
    program: &MirProgram,
    summaries: &HashMap<MirFunctionId, OriginSet>,
    native_loan_summaries: &HashMap<MirFunctionId, OriginSet>,
) {
    if !enabled() || !program.functions().any(|(_, func)| enabled_for(func)) {
        return;
    }
    CHECKER_SEEN.store(true, Ordering::Relaxed);
    eprintln!("OWNER_LOAN_TRACE final-return-summaries={summaries:#?}");
    eprintln!("OWNER_LOAN_TRACE actual-native-value-loan-summaries={native_loan_summaries:#?}");
    eprintln!(
        "OWNER_LOAN_TRACE canonical-drop-glue={:#?}",
        program.backend_contract.drop_glue,
    );
    for (id, func) in program.functions() {
        let key = program.backend_contract.function_bodies.get(id);
        let declaration = key.and_then(|key| program.backend_contract.callable(key));
        eprintln!(
            "OWNER_LOAN_TRACE function-summary id={:?} display_name={:?} ret={:?} ret_type={:?} callable={:?} declaration={:?} summary={:?}",
            id, func.name, func.ret_type, program.type_context.type_for(func.ret_type),
            key, declaration, summaries.get(id),
        );
    }
}

pub(super) fn function(
    func: &MirFunction,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    init: &crate::mir::dataflow::Results<InitMap>,
    table: &LoanTable,
    locations: &LoanLocationIndex,
    liveness: &liveness::ReferenceLiveness,
    entries: &[LoanState],
) {
    if !enabled_for(func) {
        return;
    }
    eprintln!("OWNER_LOAN_TRACE focused-function={:?} ownership={:?}", func.id, func.ownership);
    for loan in table.iter() {
        eprintln!(
            "OWNER_LOAN_TRACE function-loan function={:?} loan={:?} indexed_place={:?} returned_view={}",
            func.id, loan, table.place_paths().place(loan.place_path), table.is_returned_view(loan.id),
        );
    }
    for (block_index, block) in func.basic_blocks.iter().enumerate() {
        let selected = block.statements.iter().any(|stmt| enabled_at(stmt.span.as_ref()))
            || block.terminator.as_ref().is_some_and(|term| enabled_at(func.terminator_origin(term).source_span()));
        if !selected {
            continue;
        }
        eprintln!(
            "OWNER_LOAN_TRACE focused-block function={:?} block={} reachable={} init_entry={:?} init_exit={:?} active_entry={:?} live_entry={:?} live_exit={:?}",
            func.id, block_index, init.reachable[block_index], init.entry_sets[block_index],
            init.exit_sets[block_index], loan_summary(&entries[block_index]),
            liveness.entry_sets[block_index].iter().collect::<Vec<_>>(),
            liveness.exit_sets[block_index].iter().collect::<Vec<_>>(),
        );
        for (index, stmt) in block.statements.iter().enumerate() {
            if !enabled_at(stmt.span.as_ref()) {
                continue;
            }
            let location = location::Location::new(crate::mir::BasicBlockId(block_index), location::StatementIndex(index));
            eprintln!(
                "OWNER_LOAN_TRACE focused-location function={:?} location={:?} created_loans={:?} live_before={:?} live_after={:?}",
                func.id, location, locations.loan_ids_at(location).collect::<Vec<_>>(),
                liveness.before_statement_sets[block_index][index].iter().collect::<Vec<_>>(),
                liveness.after_statement_sets[block_index][index].iter().collect::<Vec<_>>(),
            );
        }
    }
    for (index, decl) in func.local_decls.iter().enumerate() {
        let ty = contract.normalize_type(tc, &tc.type_for(decl.ty));
        let normalized_id = tc.id_for_type(&ty);
        eprintln!(
            "OWNER_LOAN_TRACE local function={:?} local={:?} declaration={:?} normalized_type={:?} normalized_id={:?} direct_drop={:?}",
            func.id, Local(index), decl, ty, normalized_id,
            normalized_id.and_then(|id| contract.drop_glue.get(&id)),
        );
    }
}

fn place(
    func: &MirFunction,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    table: &LoanTable,
    place: &Place,
) {
    let mut prefix = Place {
        local: place.local,
        projection: Vec::new(),
    };
    for end in 0..=place.projection.len() {
        let ty = contract.place_type_id(tc, func, &prefix);
        eprintln!(
            "OWNER_LOAN_TRACE typed-place function={:?} place={:?} path={:?} type_id={:?} type={:?} direct_drop={:?}",
            func.id, prefix, table.place_paths().path_id(&prefix), ty,
            ty.map(|id| contract.normalize_type(tc, &tc.type_for(id))),
            ty.and_then(|id| contract.normalize_type_id(tc, id))
                .and_then(|id| contract.drop_glue.get(&id)),
        );
        if let Some(projection) = place.projection.get(end) {
            prefix.projection.push(projection.clone());
        }
    }
}

pub(super) fn statement(
    func: &MirFunction,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    table: &LoanTable,
    block: usize,
    index: usize,
    stmt: &StatementData,
    init: &InitMap,
    live: &LoanState,
) {
    if !enabled_at(stmt.span.as_ref()) {
        return;
    }
    let events = accesses::classify_statement(stmt, tc);
    eprintln!(
        "OWNER_LOAN_TRACE before-statement function={:?} block={} statement={} stmt={:?} accesses={:?}",
        func.id, block, index, stmt, events,
    );
    state("after-retain-before-statement", func, stmt.span.as_ref(), init, live);
    for event in events {
        place(func, tc, contract, table, &event.place);
    }
}

pub(super) fn terminator(
    func: &MirFunction,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    table: &LoanTable,
    block: usize,
    index: usize,
    term: &Terminator,
    init: &InitMap,
    active: &LoanState,
) {
    if !enabled_at(func.terminator_origin(term).source_span()) {
        return;
    }
    let events = accesses::classify_terminator(term);
    eprintln!(
        "OWNER_LOAN_TRACE before-terminator function={:?} block={} statement={} term={:?} origin={:?} accesses={:?}",
        func.id, block, index, term, func.terminator_origin(term), events,
    );
    state("before-terminator", func, func.terminator_origin(term).source_span(), init, active);
    for event in events {
        place(func, tc, contract, table, &event.place);
    }
}

pub(super) fn access(
    func: &MirFunction,
    place: &Place,
    kind: LoanKind,
    table: &LoanTable,
    active: &LoanState,
    span: Option<&Span>,
) {
    if !enabled_at(span) {
        return;
    }
    eprintln!(
        "OWNER_LOAN_TRACE checked-access function={:?} span={:?} kind={:?} place={:?} path={:?} resolved={:?} active_ids_and_owner_union={:?}",
        func.id, span, kind, place, table.place_paths().path_id(place),
        provenance::resolve_places(place, table, active), loan_summary(active),
    );
    for id in active.active_ids() {
        let resolved = provenance::resolve_loan_places(id, table, active);
        eprintln!(
            "OWNER_LOAN_TRACE active-loan id={:?} data={:?} owners={:?} resolved={:?} resolved_paths={:?}",
            id, table.get(id), active.owners(id).iter().collect::<Vec<_>>(), resolved,
            resolved.iter().map(|place| table.place_paths().path_id(place)).collect::<Vec<_>>(),
        );
    }
}

pub(super) fn call_origins(
    analysis: &ReferenceEscapeAnalysis<'_>,
    state: &ReferenceOriginState,
    term: &Terminator,
    result: &OriginSet,
) {
    return_call_origins(analysis, state, term, result);
    if !enabled_at(analysis.func.terminator_origin(term).source_span())
        || analysis.missing_summary_mode != MissingSummaryMode::Conservative
    {
        return;
    }
    let Terminator::Call {
        func, args, destination, ..
    } = term
    else {
        return;
    };
    let callable = match func {
        Operand::Constant(Constant::Callable(callable)) => Some(callable),
        _ => None,
    };
    let id = callable.and_then(MirCallable::function_id);
    let declaration = match callable {
        Some(MirCallable::Resolved(key)) => analysis.backend_contract.callable(key),
        _ => None,
    };
    let sorted_origins = |origins: &OriginSet| {
        let mut values = origins.iter().map(|origin| format!("{origin:?}")).collect::<Vec<_>>();
        values.sort();
        values
    };
    let argument_origins = args.iter().map(|arg| sorted_origins(&analysis.origins_for_operand(state, arg))).collect::<Vec<_>>();
    let mapped_result = sorted_origins(result);
    let observation = format!("{:?}|{:?}|{:?}|{:?}|{:?}", analysis.func.id, term, analysis.missing_summary_mode, argument_origins, mapped_result);
    if !CALL_OBSERVATIONS.get_or_init(|| Mutex::new(HashSet::new()))
        .lock().expect("diagnostic observation lock").insert(observation)
    {
        return;
    }
    eprintln!(
        "OWNER_LOAN_TRACE call-origins function={:?} mode={:?} term={:?} origin={:?} callee_id={:?} signature={:?} summary={:?} destination_tracks_origin={} argument_origins={:?} mapped_result={:?} callable_lifetime={:?} callable_values={:?}",
        analysis.func.id, analysis.missing_summary_mode, term,
        analysis.func.terminator_origin(term), id, declaration.map(|decl| &decl.signature),
        id.as_ref().and_then(|id| analysis.return_summaries.get(id)),
        analysis.local_type_tracks_origin(destination.local), argument_origins, result,
        analysis.return_sources.and_then(|table| table.callable_result_call(analysis.func, analysis.type_context, analysis.backend_contract, func, args, destination)),
        analysis.return_sources.and_then(|table| table.callable_result_loan_call(analysis.func, analysis.type_context, analysis.backend_contract, func, args, destination)),
    );
}

#[test]
#[ignore = "historical source10 inspection: explicit baseline or remaining-drop mode; not allocator acceptance"]
fn capture_original_allocator_source10_loan_authority() {
    assert!(
        enabled(),
        "set ROCK_OWNER_LOAN_TRACE=1 for actual checker evidence",
    );
    let source_path = PathBuf::from(
        std::env::var_os("ROCK_OWNER_TRACE_SOURCE").expect("explicit unchanged source10 path"),
    );
    let artifact = PathBuf::from(
        std::env::var_os("ROCK_OWNER_TRACE_STDLIB").expect("explicit real stdlib artifact"),
    );
    assert!(source_path.is_absolute() && source_path.is_file());
    assert!(artifact.is_absolute() && artifact.is_file());
    let remaining_drop = match std::env::var("ROCK_OWNER_LOAN_TRACE_EXPECT") {
        Err(std::env::VarError::NotPresent) => false,
        Ok(value) if value == "baseline" => false,
        Ok(value) if value == "remaining-drop" => true,
        value => panic!("expected baseline or remaining-drop inspection mode: {value:?}"),
    };
    assert_eq!(selection().path, source_path);
    for (start, end) in DEFAULT_SPANS {
        assert!(enabled_at(Some(&Span::new(source_path.clone(), start, end))), "trace ranges must cover every original site and loan origin");
    }
    let source = std::fs::read(&source_path).unwrap();
    let artifact_bytes = std::fs::read(&artifact).unwrap();
    let config = crate::Config {
        entry_file: source_path.clone(),
        current_crate_name: Some("allocator_state_caller".into()),
        extern_artifacts: vec![("stdlib".into(), artifact.clone())],
        debug_print: vec![crate::DebugPrint::Mir],
        no_std: true,
        no_prelude: true,
        no_link: true,
        ..Default::default()
    };
    CHECKER_SEEN.store(false, Ordering::Relaxed);
    CALL_OBSERVATIONS.get_or_init(|| Mutex::new(HashSet::new()))
        .lock().expect("diagnostic observation lock").clear();
    let diagnostics = crate::compile(&config)
        .expect_err("unchanged source10 must reach its actual Borrow rejection");
    eprintln!("OWNER_LOAN_TRACE actual-compile-diagnostics={diagnostics:#?}");
    assert!(
        CHECKER_SEEN.load(Ordering::Relaxed),
        "production BorrowChecker::run was not reached",
    );
    assert!(
        diagnostics.0.iter().all(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)),
        "unexpected non-Borrow failure: {diagnostics:#?}",
    );
    // These are original source ranges, not invented MIR/loan identities.
    for (start, end, text, message, related) in [
        (2747, 2754, "pointer", "borrow conflict", Some((2567, 2583, "borrow introduced here"))),
        (2808, 2817, "allocator", "borrow conflict", Some((2567, 2583, "borrow introduced here"))),
        (2985, 3000, "allocator_bytes", "borrow conflict", Some((2667, 2701, "borrow introduced here"))),
        (3048, 3059, "state_bytes", "borrow conflict", Some((2667, 2701, "borrow introduced here"))),
        (3453, 3458, "state", "because it implements Drop", None),
        (3476, 3481, "owner", "partially moved", Some((3453, 3458, "value moved from"))),
    ] {
        assert_eq!(&source[start..end], text.as_bytes(), "requires unchanged source10");
        if remaining_drop && start < 3453 {
            assert!(!diagnostics.0.iter().any(|diagnostic| {
                diagnostic.primary.as_ref().is_some_and(|primary| {
                    primary.span.file_path == source_path
                        && primary.span.start == start && primary.span.end == end
                })
            }), "repaired raw-provenance site still rejects: {diagnostics:#?}");
            continue;
        }
        assert!(diagnostics.0.iter().any(|diagnostic| {
            diagnostic.message.contains(message) && diagnostic.primary.as_ref().is_some_and(|primary| {
                primary.span.file_path == source_path && primary.span.start == start && primary.span.end == end
            }) && related.map_or(true, |(start, end, message)| {
                diagnostic.secondary.iter().any(|label| {
                    label.message.contains(message) && label.span.file_path == source_path
                        && label.span.start == start && label.span.end == end
                })
            })
        }), "missing original source10 Borrow site {start}..{end}: {diagnostics:#?}");
    }
    if remaining_drop {
        assert!(diagnostics.0.iter().all(|diagnostic| {
            diagnostic.primary.as_ref().is_some_and(|primary| {
                primary.span.file_path == source_path
                    && [(3453, 3458), (3476, 3481)].contains(&(primary.span.start, primary.span.end))
            })
        }), "unexpected rejection beyond ordinary Drop field movement: {diagnostics:#?}");
    }
    assert_eq!(std::fs::read(&source_path).unwrap(), source);
    assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
}

// Isolated source12 inspection. These counters and snapshots are observation
// only; none of them participate in origin, loan, or native-proof authority.
const RETURN_ARGUMENT: (usize, usize) = (2450, 2459);
const RETURN_CALLS: [(usize, usize); 2] = [(2567, 2583), (2667, 2701)];
static RETURN_TRACE: OnceLock<Mutex<ReturnTrace>> = OnceLock::new();
static RETURN_SOURCE: OnceLock<PathBuf> = OnceLock::new();

#[derive(Default)]
struct ReturnTrace {
    checker_seen: bool,
    exits_seen: usize,
    local_storage_seen: bool,
    functions: HashSet<MirFunctionId>,
    call_authorities: usize,
    calls: HashSet<String>,
    call_sites: HashSet<(usize, usize)>,
    entries: usize,
    origins: usize,
    types: HashSet<TypeId>,
    edges: usize,
    path_steps: usize,
    captures: usize,
}

fn return_trace() -> &'static Mutex<ReturnTrace> {
    RETURN_TRACE.get_or_init(|| Mutex::new(ReturnTrace::default()))
}

fn return_enabled() -> bool {
    std::env::var("ROCK_OWNER_RETURN_TRACE").as_deref() == Ok("1")
}

fn return_source() -> &'static PathBuf {
    RETURN_SOURCE.get_or_init(|| PathBuf::from(
        std::env::var_os("ROCK_OWNER_TRACE_SOURCE").expect("explicit source12 observer path"),
    ))
}

fn return_at(span: Option<&Span>, range: (usize, usize)) -> bool {
    span.is_some_and(|span| {
        span.file_path == *return_source() && span.start == range.0 && span.end == range.1
    })
}

fn return_function(func: &MirFunction) -> bool {
    return_enabled() && func.local_decls.iter().any(|decl| {
        decl.source == LocalSource::Argument && return_at(decl.span.as_ref(), RETURN_ARGUMENT)
    })
}

fn return_budget(count: &mut usize, maximum: usize, label: &str) {
    *count += 1;
    assert!(*count <= maximum, "RETURN_TRACE budget overflow: {label} {count}/{maximum}");
}

fn return_gap<T>(value: Option<T>, label: &str) -> T {
    value.unwrap_or_else(|| {
        eprintln!("RETURN_TRACE authority-gap={label}");
        panic!("source12 observer lacks checked {label}");
    })
}

fn return_sorted(origins: &OriginSet) -> Vec<String> {
    assert!(origins.len() <= 64, "RETURN_TRACE origin-set budget overflow");
    let mut sorted = origins.iter().map(|origin| format!("{origin:?}")).collect::<Vec<_>>();
    sorted.sort();
    sorted
}

fn return_summary(map: &HashMap<MirFunctionId, OriginSet>, id: Option<&MirFunctionId>) -> Option<Vec<String>> {
    id.and_then(|id| map.get(id)).map(return_sorted)
}

fn return_type(
    trace: &mut ReturnTrace,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    id: TypeId,
) {
    let id = return_gap(contract.normalize_type_id(tc, id), "normalized type ID");
    if !trace.types.insert(id) {
        return;
    }
    assert!(trace.types.len() <= 512, "RETURN_TRACE type budget overflow");
    // Ty uses child TypeIds, not recursively expanded type-context dumps.
    eprintln!("RETURN_TRACE type={id:?} node={:?}", TypeView::new(tc).ty(id));
    let ty = tc.type_for(id);
    let mut children = Vec::new();
    match &ty {
        Type::Struct { id: owner, args } => {
            let layout = return_gap(contract.nominal_layouts.get(owner), "nominal layout");
            let MirNominalLayout::Struct { fields, generic_params, .. } = layout else {
                panic!("RETURN_TRACE authority-gap=struct layout kind");
            };
            assert_eq!(generic_params.len(), args.len(), "RETURN_TRACE nominal binder/argument agreement");
            let field_ids = return_gap(contract.nominal_field_ids.get(owner), "canonical nominal field IDs");
            assert_eq!(field_ids.len(), fields.len(), "RETURN_TRACE canonical field-table agreement");
            let substitution = generic_params.iter().copied().zip(args.iter().cloned())
                .collect::<HashMap<_, _>>();
            let bindings = generic_params.iter().zip(args).map(|(param, arg)| {
                (*param, return_gap(tc.id_for_type(arg), "nominal argument type ID"))
            }).collect::<Vec<_>>();
            assert!(fields.len() <= 512, "RETURN_TRACE field budget overflow");
            eprintln!("RETURN_TRACE nominal type={id:?} owner={owner:?} declared-bindings={bindings:?}");
            for (index, (name, declared)) in fields.iter().enumerate() {
                return_budget(&mut trace.edges, 1024, "type edges");
                let actual = contract.normalize_type(tc, &tc.type_for(*declared).substitute_generics(&substitution));
                let child = return_gap(tc.id_for_type(&actual), "instantiated nominal field type ID");
                eprintln!("RETURN_TRACE owned-field type={id:?} index={index} owner={owner:?} field_id={:?} name={name:?} declared={declared:?} actual={child:?}", field_ids[index]);
                children.push(("owned-field", child));
            }
        }
        Type::Tuple(fields) => {
            assert!(fields.len() <= 512, "RETURN_TRACE tuple budget overflow");
            for field in fields {
                children.push(("owned-tuple-element", return_gap(tc.id_for_type(field), "tuple element type ID")));
            }
        }
        Type::Array(inner, _) | Type::Slice(inner) => {
            children.push(("array-or-slice-element", return_gap(tc.id_for_type(inner), "element type ID")));
        }
        Type::Pointer(inner) => {
            children.push(("raw-pointee-NOT-stored-environment", return_gap(tc.id_for_type(inner), "raw pointee type ID")));
        }
        Type::Reference { inner, .. } => {
            children.push(("safe-reference-pointee", return_gap(tc.id_for_type(inner), "reference pointee type ID")));
        }
        Type::Function { params, ret, captures, .. } => {
            assert!(params.len() + captures.len() <= 64, "RETURN_TRACE callable type budget overflow");
            // These are actual signature children, not a capture-free seal.
            for param in params {
                children.push(("callable-parameter-NOT-environment", return_gap(tc.id_for_type(param), "callable parameter type ID")));
            }
            children.push(("callable-result-NOT-environment", return_gap(tc.id_for_type(ret), "callable result type ID")));
            for capture in captures {
                children.push(("actual-signature-capture-metadata", return_gap(tc.id_for_type(&capture.ty), "capture metadata type ID")));
            }
        }
        Type::Enum { .. } => {
            // This bounded source12 observer does not fabricate enum FieldIds.
            panic!("RETURN_TRACE authority-gap=unexpected enum payload; request explicit enum-path observation");
        }
        _ => {}
    }
    for (relation, child) in children {
        return_budget(&mut trace.edges, 1024, "type edges");
        eprintln!("RETURN_TRACE edge parent={id:?} relation={relation} child={child:?}");
        return_type(trace, tc, contract, child);
    }
}

fn return_place_type(
    trace: &mut ReturnTrace,
    func: &MirFunction,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    place: &Place,
) -> TypeId {
    assert!(place.projection.len() <= 32, "RETURN_TRACE projection budget overflow");
    let mut prefix = Place { local: place.local, projection: Vec::new() };
    let mut ty = return_gap(contract.place_type_id(tc, func, &prefix), "root place type ID");
    for projection in &place.projection {
        return_budget(&mut trace.path_steps, 2048, "checked path steps");
        if let Projection::Field { index, identity } = projection {
            if let Type::Struct { id: owner, .. } = contract.normalize_type(tc, &tc.type_for(ty)) {
                let fields = return_gap(contract.nominal_field_ids.get(&owner), "path canonical field table");
                let field_id = *return_gap(fields.get(*index), "path canonical field ID");
                let canonical = crate::mir::MirFieldIdentity::new(owner, field_id);
                if let Some(identity) = identity {
                    assert_eq!(*identity, canonical, "RETURN_TRACE path identity agreement");
                }
                eprintln!("RETURN_TRACE path-field function={:?} prefix={prefix:?} index={index} canonical={canonical:?}", func.id);
            }
        }
        prefix.projection.push(projection.clone());
        ty = return_gap(contract.place_type_id(tc, func, &prefix), "projected place type ID");
        eprintln!("RETURN_TRACE typed-path function={:?} place={prefix:?} type={ty:?}", func.id);
    }
    return_type(trace, tc, contract, ty);
    ty
}

pub(super) fn return_program(
    program: &MirProgram,
    lifetime: &HashMap<MirFunctionId, OriginSet>,
    values: &HashMap<MirFunctionId, OriginSet>,
) {
    if !return_enabled() {
        return;
    }
    assert!(!enabled(), "RETURN_TRACE must not enable broad OWNER_LOAN_TRACE");
    let mut trace = return_trace().lock().expect("source12 observation lock");
    trace.checker_seen = true;
    for (id, func) in program.functions().filter(|(_, func)| return_function(func)) {
        trace.functions.insert(id.clone());
        assert!(trace.functions.len() <= 32, "RETURN_TRACE function budget overflow");
        let contract = &program.backend_contract;
        let tc = &program.type_context;
        let key = return_gap(contract.function_bodies.get(id), "actual native function-body key");
        let declaration = return_gap(contract.callable(key), "actual native declaration");
        assert_eq!(&declaration.key, key, "RETURN_TRACE callable key agreement");
        assert!(matches!(&declaration.kind, crate::mir::MirCallableKind::LocalBody { function_id } if function_id == id));
        assert_eq!(&func.id, id, "RETURN_TRACE actual body ID agreement");
        assert!(declaration.source_def_id.is_some(), "RETURN_TRACE actual source DefId missing");
        assert_eq!(declaration.signature.params.len(), func.arg_count);
        assert_eq!(declaration.signature.ret.semantic_ty, func.ret_type);
        assert_eq!(declaration.signature.ret.abi_ty, func.ret_type);
        assert_eq!(func.local_decls[0].ty, func.ret_type);
        assert!(func.arg_count <= 16, "RETURN_TRACE parameter budget overflow");
        eprintln!("RETURN_TRACE function={id:?} key={key:?} declaration={declaration:?} local0={:?} lifetime={:?} value_loans={:?} value_body_available={}",
            func.local_decls[0], return_summary(lifetime, Some(id)), return_summary(values, Some(id)),
            BorrowChecker::body_available_for_purpose(program, id, func, OriginPurpose::ValueLoans));
        for (index, param) in declaration.signature.params.iter().enumerate() {
            let local = &func.local_decls[index + 1];
            assert_eq!(local.ty, param.semantic_ty, "RETURN_TRACE parameter/body type agreement");
            eprintln!("RETURN_TRACE argument function={id:?} local={:?} declaration={local:?} abi={param:?}", Local(index + 1));
            return_type(&mut trace, tc, contract, local.ty);
        }
        return_type(&mut trace, tc, contract, func.ret_type);
        for block in &func.basic_blocks {
            let Some(term @ Terminator::Call { func: operand, destination, .. }) = &block.terminator else {
                continue;
            };
            let span = func.terminator_origin(term).source_span().cloned();
            if !RETURN_CALLS.iter().any(|range| return_at(span.as_ref(), *range)) {
                continue;
            }
            return_budget(&mut trace.call_authorities, 64, "related callable headers");
            let Operand::Constant(Constant::Callable(MirCallable::Resolved(callee_key))) = operand else {
                panic!("RETURN_TRACE authority-gap=related call is not a canonical resolved callable");
            };
            let callee = return_gap(contract.callable(callee_key), "related callable header");
            assert_eq!(&callee.key, callee_key);
            let callee_id = match &callee.kind {
                crate::mir::MirCallableKind::LocalBody { function_id } => Some(function_id),
                _ => None,
            };
            let body_available = callee_id.and_then(|id| program.functions.get(id).map(|body| (id, body)))
                .is_some_and(|(id, body)| BorrowChecker::body_available_for_purpose(program, id, body, OriginPurpose::ValueLoans));
            eprintln!("RETURN_TRACE call-authority caller={id:?} span={span:?} callee_key={callee_key:?} declaration={callee:?} lifetime={:?} value_loans={:?} value_body_available={body_available}",
                return_summary(lifetime, callee_id), return_summary(values, callee_id));
            if let Some(body) = callee_id.and_then(|id| program.functions.get(id)) {
                assert!(body.arg_count <= 16, "RETURN_TRACE related body argument budget overflow");
                eprintln!("RETURN_TRACE callee-body key={callee_key:?} id={:?} ret={:?} local0={:?} arg_count={}",
                    body.id, body.ret_type, body.local_decls.first(), body.arg_count);
                for (index, local) in body.local_decls.iter().skip(1).take(body.arg_count).enumerate() {
                    eprintln!("RETURN_TRACE callee-argument key={callee_key:?} local={:?} declaration={local:?} header={:?}",
                        Local(index + 1), callee.signature.params.get(index));
                }
            }
            return_place_type(&mut trace, func, tc, contract, destination);
        }
    }
}

fn return_call_origins(
    analysis: &ReferenceEscapeAnalysis<'_>,
    state: &ReferenceOriginState,
    term: &Terminator,
    result: &OriginSet,
) {
    if !return_function(analysis.func) || analysis.missing_summary_mode != MissingSummaryMode::Conservative {
        return;
    }
    let origin = analysis.func.terminator_origin(term);
    let span = origin.source_span();
    if !RETURN_CALLS.iter().any(|range| return_at(span, *range)) {
        return;
    }
    let Terminator::Call { args, destination, .. } = term else { return; };
    assert!(args.len() <= 16, "RETURN_TRACE call argument budget overflow");
    let arguments = args.iter().map(|arg| return_sorted(&analysis.origins_for_operand(state, arg)))
        .collect::<Vec<_>>();
    let result = return_sorted(result);
    let observation = format!("function={:?} origin={origin:?} term={term:?} argument_origins={arguments:?} mapped_result={result:?}", analysis.func.id);
    let mut trace = return_trace().lock().expect("source12 observation lock");
    if !trace.calls.insert(observation.clone()) {
        return;
    }
    assert!(trace.calls.len() <= 64, "RETURN_TRACE related-call snapshot budget overflow");
    let span = return_gap(span, "related call source span");
    trace.call_sites.insert((span.start, span.end));
    // Conservative analysis is the actual escape validator's Lifetime map;
    // native ValueLoans summaries are printed separately by return_program.
    eprintln!("RETURN_TRACE actual-lifetime-call {observation}");
    for (index, argument) in args.iter().enumerate() {
        if let Operand::Copy(place) | Operand::Move(place) = argument {
            let declaration = return_gap(analysis.func.local_decls.get(place.local.0), "actual call argument declaration");
            eprintln!("RETURN_TRACE actual-call-argument function={:?} argument={index} place={place:?} declaration={declaration:?}", analysis.func.id);
            return_place_type(&mut trace, analysis.func, analysis.type_context, analysis.backend_contract, place);
        }
    }
    return_place_type(&mut trace, analysis.func, analysis.type_context, analysis.backend_contract, destination);
}

pub(super) fn return_exits(
    analysis: &ReferenceEscapeAnalysis<'_>,
    results: &crate::mir::dataflow::Results<ReferenceOriginState>,
) {
    let func = analysis.func;
    if !return_function(func) {
        return;
    }
    let mut trace = return_trace().lock().expect("source12 observation lock");
    assert!(trace.functions.contains(&func.id), "RETURN_TRACE map/body observation must precede return validation");
    trace.exits_seen += 1;
    let root = Place { local: Local(0), projection: Vec::new() };
    for (block_index, block) in func.basic_blocks.iter().enumerate() {
        if !matches!(block.terminator, Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })) {
            continue;
        }
        let mut entries = results.exit_sets[block_index].origins.iter()
            .filter(|(place, _)| ReferenceEscapeAnalysis::place_is_prefix(&root, place))
            .collect::<Vec<_>>();
        entries.sort_by_key(|(place, _)| format!("{place:?}"));
        for (place, origins) in entries {
            return_budget(&mut trace.entries, 256, "actual return entries");
            let ty = return_place_type(&mut trace, func, analysis.type_context, analysis.backend_contract, place);
            eprintln!("RETURN_TRACE actual-return function={:?} block={block_index} reachable={} place={place:?} type={ty:?} lifetime_origins={:?}",
                func.id, results.reachable[block_index], return_sorted(origins));
            let mut sorted = origins.iter().collect::<Vec<_>>();
            sorted.sort_by_key(|origin| format!("{origin:?}"));
            for origin in sorted {
                return_budget(&mut trace.origins, 1024, "return origin rows");
                let local = match origin {
                    ReferenceOrigin::Local(local) | ReferenceOrigin::Temporary(local)
                    | ReferenceOrigin::Param(local) | ReferenceOrigin::ParamContents(local)
                    | ReferenceOrigin::ParamBorrowedContents(local) => Some(*local),
                    ReferenceOrigin::Static | ReferenceOrigin::UnknownExternal => None,
                };
                let declaration = local.map(|local| return_gap(func.local_decls.get(local.0), "actual origin local declaration"));
                eprintln!("RETURN_TRACE return-origin function={:?} place={place:?} kind={origin:?} declaration={declaration:?}", func.id);
                if let Some(declaration) = declaration {
                    return_type(&mut trace, analysis.type_context, analysis.backend_contract, declaration.ty);
                    if matches!(origin, ReferenceOrigin::Local(_) | ReferenceOrigin::Temporary(_))
                        && return_at(declaration.span.as_ref(), RETURN_ARGUMENT)
                    {
                        trace.local_storage_seen = true;
                    }
                }
                if matches!(TypeView::new(analysis.type_context).ty(ty), Ty::Function { .. }) {
                    // Only actual storage operations in this body are reported;
                    // signature metadata is not upgraded to environment proof.
                    for statement in func.basic_blocks.iter().flat_map(|block| &block.statements) {
                        if let StatementKind::Assign(dest, Rvalue::Closure(closure)) = &statement.kind {
                            return_budget(&mut trace.captures, 64, "actual closure storage rows");
                            eprintln!("RETURN_TRACE actual-closure-storage function={:?} destination={dest:?} id={:?}", func.id, closure.id);
                            assert!(closure.captures.len() <= 64, "RETURN_TRACE closure capture budget overflow");
                            for capture in &closure.captures {
                                let declaration = return_gap(func.local_decls.get(capture.local.0), "actual capture storage declaration");
                                eprintln!("RETURN_TRACE actual-capture capture={capture:?} declaration={declaration:?}");
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "historical pre-eligibility source12 oracle: exact ten Borrows; explicit artifact, never executes"]
fn capture_allocator_source12_return_origin_authority() {
    assert!(return_enabled(), "set ROCK_OWNER_RETURN_TRACE=1");
    for flag in [
        "ROCK_OWNER_LOAN_TRACE", "ROCK_ALLOCATOR_EXECUTION_DEBUG",
        "ROCK_ALLOCATOR_EXECUTION_MIR", "ROCK_STATIC_CAPTURE_TRACE",
        "ROCK_ALLOCATOR_FRONTEND_ONLY",
    ] {
        assert!(std::env::var_os(flag).is_none(), "unset broad diagnostic flag {flag}");
    }
    let source_path = return_source().clone();
    let artifact = PathBuf::from(std::env::var_os("ROCK_OWNER_TRACE_STDLIB")
        .expect("explicit completed-primary source12 stdlib artifact"));
    assert_eq!(source_path, agent_work_path("recovery/rock-completion-allocator-state-source-12.rk"));
    assert_eq!(artifact, agent_work_path("recovery/rock_allocator_state_785145_1/stdlib.rkca"));
    let source = std::fs::read(&source_path).unwrap();
    let artifact_bytes = std::fs::read(&artifact).unwrap();
    assert_eq!(source.len(), 14272, "unchanged source12 byte length");
    assert_eq!(artifact_bytes.len(), 803478, "current primary stdlib byte length");
    assert_eq!(source, std::fs::read(agent_work_path("recovery/rock-source12-return-diagnostic-source.txt")).unwrap(),
        "source12 must exactly match the decoded completed-primary diagnostic source");
    for (start, end, text) in [
        (2450, 2459, "allocator"), (2567, 2583, "allocator.audit!"),
        (2667, 2701, "allocator.reserve bytes, alignment"),
        (2378, 2396, "Shell::new_aligned"),
        (11007, 11025, "Shell::new_aligned"),
        (13401, 13419, "Shell::new_aligned"),
    ] {
        assert_eq!(&source[start..end], text.as_bytes(), "unchanged source12 observer anchor");
    }
    eprintln!("RETURN_TRACE recorded-lineage source_sha256=4c27fe022a76fc12e5a3b6044ef25ebbd40d5413e46fafa68eabb13ccd4dc796 artifact_sha256=1379082206f7865ff405d8674cfad35813469925d6395fcb3864bf391caa5f58; byte snapshots checked independently, hashes are not MIR authority");
    *return_trace().lock().expect("source12 observation lock") = ReturnTrace::default();
    let config = crate::Config {
        entry_file: source_path.clone(),
        current_crate_name: Some("allocator_state_caller".into()),
        extern_artifacts: vec![("stdlib".into(), artifact.clone())],
        no_std: true,
        no_prelude: true,
        no_link: true,
        ..Default::default()
    };
    let outcome = crate::compile(&config);
    assert_eq!(std::fs::read(&source_path).unwrap(), source, "source12 bytes unchanged after real compile");
    assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes, "primary artifact bytes unchanged after real compile");
    let diagnostics = outcome.expect_err("unchanged source12 must reproduce its actual owning-return Borrow rejection");
    assert!(diagnostics.0.len() <= 32, "source12 diagnostic observation budget exceeded");
    for diagnostic in &diagnostics.0 {
        eprintln!("RETURN_TRACE diagnostic code={:?} primary={:?} message={:?}", diagnostic.code,
            diagnostic.primary.as_ref().map(|primary| &primary.span), diagnostic.message);
    }
    assert_eq!(diagnostics.0.len(), 10, "source12 observer requires all ten historical Borrow diagnostics");
    let mut spans = std::collections::BTreeMap::new();
    for diagnostic in &diagnostics.0 {
        assert_eq!(diagnostic.code, Some(DiagnosticCode::Borrow));
        assert_eq!(diagnostic.message, "Cannot return reference to local value because it does not live long enough");
        let primary = diagnostic.primary.as_ref().expect("actual source12 primary span");
        assert_eq!(primary.span.file_path, source_path);
        *spans.entry((primary.span.start, primary.span.end)).or_insert(0usize) += 1;
    }
    assert_eq!(spans, std::collections::BTreeMap::from([
        ((2450, 2459), 4), ((2378, 2396), 4),
        ((11007, 11025), 1), ((13401, 13419), 1),
    ]), "exact source12 diagnostic multiset; no alternate error accepted");
    let trace = return_trace().lock().expect("source12 observation lock");
    assert!(trace.checker_seen && !trace.functions.is_empty(), "actual production BorrowChecker::run maps not observed");
    assert!(trace.exits_seen > 0 && trace.entries > 0 && trace.local_storage_seen,
        "actual return fixpoint must contain the offending local-storage origin at source2450..2459");
    assert!(RETURN_CALLS.iter().all(|range| trace.call_sites.contains(range)),
        "both actual audit/reserve Lifetime call states must be observed");
    eprintln!("RETURN_TRACE reproduced-historical-negative exact-Borrow-multiset={spans:?}");
    eprintln!("RETURN_TRACE counts functions={} return_entries={} origin_rows={} calls={} call_authorities={} types={} edges={} path_steps={} capture_rows={} exits={} budgets=32/256/1024/64/64/512/1024/2048/64; allocator execution NOT RUN",
        trace.functions.len(), trace.entries, trace.origins, trace.calls.len(), trace.call_authorities, trace.types.len(),
        trace.edges, trace.path_steps, trace.captures, trace.exits_seen);
}

// Independent fresh-stdlib observer. All snapshots are test-only and immutable
// with respect to the compiler's types, origins, summaries and native proofs.
static FRESH_STDLIB_TRACE: OnceLock<Mutex<FreshStdlibTrace>> = OnceLock::new();

#[derive(Default)]
struct FreshStdlibTrace {
    inputs: std::collections::BTreeMap<PathBuf, Vec<u8>>,
    transfers: std::collections::BTreeMap<String, FreshTransfer>,
    missing: Vec<FreshMissing>,
    types: HashSet<TypeId>,
    edges: usize,
    source_spans: HashSet<(PathBuf, usize, usize)>,
    program_seen: bool,
    headers: usize,
}

pub(super) struct FreshTransfer {
    function: MirFunctionId,
    statement: StatementData,
    unmatched: Vec<Place>,
    before: Vec<String>,
    after: Vec<String>,
}

struct FreshMissing {
    function: MirFunctionId,
    block: usize,
    reachable: bool,
    place: Place,
    origins: OriginSet,
}

fn fresh_enabled() -> bool {
    std::env::var("ROCK_FRESH_STDLIB_RETURN_TRACE").as_deref() == Ok("1")
}

fn fresh_trace() -> &'static Mutex<FreshStdlibTrace> {
    FRESH_STDLIB_TRACE.get_or_init(|| Mutex::new(FreshStdlibTrace::default()))
}

fn fresh_state_rows(
    analysis: &ReferenceEscapeAnalysis<'_>,
    state: &ReferenceOriginState,
    statement: &StatementData,
) -> Vec<String> {
    let StatementKind::Assign(destination, Rvalue::Aggregate(_, operands)) = &statement.kind else {
        panic!("FRESH_STDLIB_TRACE requires an actual aggregate assignment");
    };
    let locals = std::iter::once(destination.local).chain(operands.iter().filter_map(|operand| {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => Some(place.local),
            Operand::Constant(_) => None,
        }
    })).collect::<HashSet<_>>();
    let mut rows = Vec::new();
    for (place, origins) in &state.origins {
        if locals.contains(&place.local) {
            assert!(rows.len() < 64, "FRESH_STDLIB_TRACE state-row budget overflow");
            rows.push(format!("{place:?}={:?}", return_sorted(origins)));
        }
    }
    rows.sort();
    // Exact queries against this pre/post state include the analysis's existing
    // parameter fallback, not just already materialized map rows. Post queries
    // are labeled separately from the actual destination map after insertion.
    for (index, operand) in operands.iter().enumerate() {
        let checked = match operand {
            Operand::Copy(place) | Operand::Move(place) => analysis.backend_contract
                .place_type_id(analysis.type_context, analysis.func, place),
            Operand::Constant(_) => None,
        };
        rows.push(format!("operand-query index={index} checked-place-type={checked:?} origins={:?}",
            return_sorted(&analysis.origins_for_operand(state, operand))));
    }
    rows
}

pub(super) fn fresh_transfer_before(
    analysis: &ReferenceEscapeAnalysis<'_>,
    state: &ReferenceOriginState,
    statement: &StatementData,
) -> Option<FreshTransfer> {
    if !fresh_enabled() || analysis.missing_summary_mode != MissingSummaryMode::Conservative {
        return None;
    }
    let StatementKind::Assign(destination, Rvalue::Aggregate(_, operands)) = &statement.kind else {
        return None;
    };
    let mut unmatched = Vec::new();
    for (index, operand) in operands.iter().enumerate() {
        let field = ReferenceEscapeAnalysis::field_place(destination, index);
        if analysis.backend_contract.place_type_id(analysis.type_context, analysis.func, &field).is_none()
            && !analysis.origins_for_operand(state, operand).is_empty()
        {
            assert!(unmatched.len() < 8, "FRESH_STDLIB_TRACE unmatched-field budget overflow");
            unmatched.push(field);
        }
    }
    if unmatched.is_empty() {
        return None;
    }
    assert!(operands.len() <= 64, "FRESH_STDLIB_TRACE aggregate operand budget overflow");
    Some(FreshTransfer {
        function: analysis.func.id.clone(),
        statement: statement.clone(),
        unmatched,
        before: fresh_state_rows(analysis, state, statement),
        after: Vec::new(),
    })
}

pub(super) fn fresh_transfer_after(
    analysis: &ReferenceEscapeAnalysis<'_>,
    state: &ReferenceOriginState,
    transfer: Option<FreshTransfer>,
) {
    let Some(mut transfer) = transfer else { return; };
    assert_eq!(transfer.function, analysis.func.id);
    transfer.after = fresh_state_rows(analysis, state, &transfer.statement);
    let key = format!("{:?}|{:?}|{:?}|{:?}|{:?}", transfer.function,
        transfer.statement, transfer.unmatched, transfer.before, transfer.after);
    let mut trace = fresh_trace().lock().expect("fresh stdlib observer lock");
    if !trace.transfers.contains_key(&key) {
        assert!(trace.transfers.len() < 64, "FRESH_STDLIB_TRACE transfer budget overflow");
        trace.transfers.insert(key, transfer);
    }
}

pub(super) fn fresh_missing_return(
    analysis: &ReferenceEscapeAnalysis<'_>,
    results: &crate::mir::dataflow::Results<ReferenceOriginState>,
    block: usize,
    place: &Place,
    origins: &OriginSet,
) {
    if !fresh_enabled() {
        return;
    }
    assert!(analysis.backend_contract.place_type_id(analysis.type_context, analysis.func, place).is_none());
    let mut trace = fresh_trace().lock().expect("fresh stdlib observer lock");
    assert!(trace.missing.len() < 8, "FRESH_STDLIB_TRACE missing-entry budget overflow");
    trace.missing.push(FreshMissing {
        function: analysis.func.id.clone(), block, reachable: results.reachable[block],
        place: place.clone(), origins: origins.clone(),
    });
}

fn fresh_source_span(trace: &mut FreshStdlibTrace, span: Option<&Span>) {
    let Some(span) = span else {
        eprintln!("FRESH_STDLIB_TRACE source-span=None (no fabricated fallback)");
        return;
    };
    let key = (span.file_path.clone(), span.start, span.end);
    if !trace.source_spans.insert(key) {
        return;
    }
    assert!(trace.source_spans.len() <= 64, "FRESH_STDLIB_TRACE source-span budget overflow");
    assert!(span.end >= span.start && span.end - span.start <= 4096,
        "FRESH_STDLIB_TRACE source-excerpt budget overflow");
    let input = trace.inputs.get(&span.file_path).expect("actual body source must be snapshotted before compilation");
    assert_eq!(&std::fs::read(&span.file_path).unwrap(), input, "actual source unchanged at observer callback");
    let bytes = input.get(span.start..span.end).expect("actual source span byte range");
    eprintln!("FRESH_STDLIB_TRACE source-span={span:?} exact-text={:?}", std::str::from_utf8(bytes).unwrap());
}

fn fresh_type(
    trace: &mut FreshStdlibTrace,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    id: TypeId,
) {
    if !trace.types.insert(id) {
        return;
    }
    assert!(trace.types.len() <= 128, "FRESH_STDLIB_TRACE visited-TypeId budget overflow");
    eprintln!("FRESH_STDLIB_TRACE type={id:?} valid={} node={:?} normalized_id={:?}",
        tc.type_id_tree_is_valid(id), tc.try_ty(id), contract.normalize_type_id(tc, id));
    if !tc.type_id_tree_is_valid(id) {
        return;
    }
    let ty = contract.normalize_type(tc, &tc.type_for(id));
    let mut children = Vec::new();
    match &ty {
        Type::Struct { id: owner, args } | Type::Enum { id: owner, args } => {
            let Some(layout) = contract.nominal_layouts.get(owner) else {
                eprintln!("FRESH_STDLIB_TRACE authority-gap=MissingNominalLayout owner={owner:?}");
                return;
            };
            let (parameters, fields) = match layout {
                MirNominalLayout::Struct { id: layout_owner, generic_params, fields } => {
                    assert_eq!(layout_owner, owner);
                    assert!(matches!(&ty, Type::Struct { .. }));
                    let ids = contract.nominal_field_ids.get(owner);
                    eprintln!("FRESH_STDLIB_TRACE struct owner={owner:?} canonical-field-ids={ids:?}");
                    let fields = fields.iter().enumerate().map(|(index, (name, field))| {
                        (format!("struct-field index={index} name={name:?} field_id={:?}", ids.and_then(|ids| ids.get(index))), *field)
                    }).collect::<Vec<_>>();
                    (generic_params, fields)
                }
                MirNominalLayout::Enum { id: layout_owner, generic_params, variants } => {
                    assert_eq!(layout_owner, owner);
                    assert!(matches!(&ty, Type::Enum { .. }));
                    assert!(variants.len() <= 64, "FRESH_STDLIB_TRACE variant layout budget overflow");
                    let mut fields = Vec::new();
                    for (ordinal, variant) in variants.iter().enumerate() {
                        // Layout ordinal is labeled as such; actual canonical
                        // VariantId is supplied separately by the MIR aggregate.
                        match &variant.fields {
                            crate::mir::MirVariantLayoutFields::Unit => {}
                            crate::mir::MirVariantLayoutFields::Positional(types) => {
                                for (index, field) in types.iter().enumerate() {
                                    fields.push((format!("enum-layout-ordinal={ordinal} field-index={index} variant-name={:?}", variant.name), *field));
                                }
                            }
                            crate::mir::MirVariantLayoutFields::Named(types) => {
                                for (index, (name, field)) in types.iter().enumerate() {
                                    fields.push((format!("enum-layout-ordinal={ordinal} field-index={index} field-name={name:?} variant-name={:?}", variant.name), *field));
                                }
                            }
                        }
                    }
                    (generic_params, fields)
                }
            };
            assert!(fields.len() <= 256, "FRESH_STDLIB_TRACE field layout budget overflow");
            eprintln!("FRESH_STDLIB_TRACE nominal parent={id:?} owner={owner:?} declared-parameters={parameters:?} actual-args={args:?} actual-arg-ids={:?}",
                args.iter().map(|arg| tc.id_for_type(arg)).collect::<Vec<_>>());
            if parameters.len() != args.len() {
                eprintln!("FRESH_STDLIB_TRACE authority-gap=NominalBinderArityMismatch");
                return;
            }
            let substitution = parameters.iter().copied().zip(args.iter().cloned()).collect::<HashMap<_, _>>();
            for (relation, declared) in fields {
                return_budget(&mut trace.edges, 256, "fresh stdlib layout edges");
                if !tc.type_id_tree_is_valid(declared) {
                    eprintln!("FRESH_STDLIB_TRACE invalid-declared-field parent={id:?} {relation} type={declared:?}");
                    continue;
                }
                let actual = contract.normalize_type(tc, &tc.type_for(declared).substitute_generics(&substitution));
                let child = tc.id_for_type(&actual);
                eprintln!("FRESH_STDLIB_TRACE field parent={id:?} {relation} declared={declared:?} actual={actual:?} instantiated_id={child:?}");
                if let Some(child) = child {
                    children.push(child);
                }
            }
        }
        _ => {
            use crate::type_services::visit::TypeVisitor;
            struct Children(Vec<Type>);
            impl TypeVisitor for Children {
                fn visit_type(&mut self, ty: &Type) { self.0.push(ty.clone()); }
            }
            let mut immediate = Children(Vec::new());
            crate::type_services::visit::visit_type_children(&ty, &mut immediate);
            assert!(immediate.0.len() <= 64, "FRESH_STDLIB_TRACE child type budget overflow");
            for child in immediate.0 {
                return_budget(&mut trace.edges, 256, "fresh stdlib layout edges");
                let child_id = tc.id_for_type(&child);
                eprintln!("FRESH_STDLIB_TRACE structural-child parent={id:?} node={child:?} id={child_id:?}; signature/pointee metadata is NOT stored-environment authority");
                if let Some(child_id) = child_id { children.push(child_id); }
            }
        }
    }
    for child in children { fresh_type(trace, tc, contract, child); }
}

fn fresh_path(
    trace: &mut FreshStdlibTrace,
    program: &MirProgram,
    function: &MirFunction,
    place: &Place,
) {
    assert!(place.projection.len() <= 32, "FRESH_STDLIB_TRACE path budget overflow");
    let tc = &program.type_context;
    let contract = &program.backend_contract;
    let mut downcast = None;
    for length in 0..=place.projection.len() {
        let prefix = Place { local: place.local, projection: place.projection[..length].to_vec() };
        let checked = contract.place_type_id(tc, function, &prefix);
        eprintln!("FRESH_STDLIB_TRACE checked-prefix function={:?} path={prefix:?} checked_type={checked:?}", function.id);
        if let Some(id) = checked { fresh_type(trace, tc, contract, id); }
        if length > 0 && checked.is_none() {
            let parent = Place { local: place.local, projection: place.projection[..length - 1].to_vec() };
            let parent_ty = contract.place_type_id(tc, function, &parent);
            let projection = &place.projection[length - 1];
            let owner = parent_ty.map(|id| contract.normalize_type(tc, &tc.type_for(id)));
            let reason = match (&owner, projection) {
                (Some(Type::Enum { .. }), Projection::Field { .. }) if downcast.is_none() => "EnumMissingDowncast",
                (Some(Type::Function { .. }), Projection::Field { .. }) => "UnsupportedFunctionField",
                (Some(Type::Array(_, _)), Projection::Field { .. }) => "UnsupportedArrayField",
                (Some(Type::Struct { id, .. }), Projection::Field { .. })
                    if !contract.nominal_layouts.contains_key(id) => "MissingNominalLayout",
                (Some(Type::Struct { .. } | Type::Enum { .. } | Type::Tuple(_)), Projection::Field { .. }) => "InspectDeclaredFieldAndInstantiatedIdAbove",
                (None, _) => "ParentCheckedTypeUnavailable",
                _ => "UnsupportedOrUnavailableProjectionAuthority",
            };
            eprintln!("FRESH_STDLIB_TRACE unresolved-step projection={projection:?} parent_type={parent_ty:?} owner={owner:?} reason={reason}; NO guessed Downcast/field/capture/type inserted");
        }
        if length > 0 {
            match &place.projection[length - 1] {
                Projection::Downcast(variant) => downcast = Some(*variant),
                Projection::Field { .. } => downcast = None,
                _ => {}
            }
        }
    }
}

pub(super) fn fresh_missing_program(
    program: &MirProgram,
    lifetime: &HashMap<MirFunctionId, OriginSet>,
    values: &HashMap<MirFunctionId, OriginSet>,
) {
    if !fresh_enabled() { return; }
    let mut trace = fresh_trace().lock().expect("fresh stdlib observer lock");
    trace.program_seen = true;
    assert!(!trace.inputs.is_empty(), "fresh stdlib host must snapshot inputs before compilation");
    let ids = trace.missing.iter().map(|case| case.function.clone()).collect::<std::collections::BTreeSet<_>>();
    for id in ids {
        let function = program.functions.get(&id).expect("actual failing native body");
        let contract = &program.backend_contract;
        let key = contract.function_bodies.get(&id).expect("actual failing native key");
        let declaration = contract.callable(key).expect("actual failing native declaration");
        assert_eq!(&declaration.key, key);
        assert!(matches!(&declaration.kind, crate::mir::MirCallableKind::LocalBody { function_id } if function_id == &id));
        assert_eq!(&function.id, &id);
        assert_eq!(function.arg_count, declaration.signature.params.len());
        assert_eq!(function.ret_type, declaration.signature.ret.semantic_ty);
        assert_eq!(function.ret_type, declaration.signature.ret.abi_ty);
        assert_eq!(function.local_decls[0].ty, function.ret_type);
        assert_eq!(function.local_decls[0].source, LocalSource::ReturnPlace);
        let definition = declaration.source_def_id.expect("actual native source definition");
        assert!(function.arg_count <= 16, "FRESH_STDLIB_TRACE header argument budget overflow");
        trace.headers += 1;
        eprintln!("FRESH_STDLIB_TRACE native function={id:?} display_name={:?} key={key:?} declaration={declaration:?} defining_product={:?} local0={:?} Lifetime={:?} ValueLoans={:?} value_body_available={}",
            function.name, contract.descriptor_origins.get(&definition.crate_id), function.local_decls[0],
            return_summary(lifetime, Some(&id)), return_summary(values, Some(&id)),
            BorrowChecker::body_available_for_purpose(program, &id, function, OriginPurpose::ValueLoans));
        fresh_source_span(&mut trace, function.local_decls[0].span.as_ref());
        fresh_path(&mut trace, program, function, &Place { local: Local(0), projection: Vec::new() });
        assert!(contract.place_type_id(&program.type_context, function,
            &Place { local: Local(0), projection: Vec::new() }).is_some(),
            "actual failing native return root must have checked canonical type authority");
        for (index, param) in declaration.signature.params.iter().enumerate() {
            let local = &function.local_decls[index + 1];
            assert_eq!(local.ty, param.semantic_ty);
            assert_eq!(local.source, LocalSource::Argument);
            eprintln!("FRESH_STDLIB_TRACE argument local={:?} declaration={local:?} ABI={param:?}", Local(index + 1));
            fresh_source_span(&mut trace, local.span.as_ref());
            fresh_type(&mut trace, &program.type_context, contract, local.ty);
        }
        let cases = trace.missing.iter().filter(|case| case.function == id)
            .map(|case| (case.block, case.reachable, case.place.clone(), case.origins.clone())).collect::<Vec<_>>();
        for (block, reachable, place, origins) in cases {
            eprintln!("FRESH_STDLIB_TRACE actual-return-exit function={id:?} block={block} reachable={reachable} path={place:?} origins={:?}", return_sorted(&origins));
            let term = function.basic_blocks[block].terminator.as_ref().expect("actual return terminator");
            fresh_source_span(&mut trace, function.terminator_origin(term).source_span());
            fresh_path(&mut trace, program, function, &place);
            let mut sorted = origins.into_iter().collect::<Vec<_>>();
            sorted.sort_by_key(|origin| format!("{origin:?}"));
            for origin in sorted {
                let local = match origin {
                    ReferenceOrigin::Local(local) | ReferenceOrigin::Temporary(local)
                    | ReferenceOrigin::Param(local) | ReferenceOrigin::ParamContents(local)
                    | ReferenceOrigin::ParamBorrowedContents(local) => Some(local),
                    ReferenceOrigin::Static | ReferenceOrigin::UnknownExternal => None,
                };
                let declaration = local.map(|local| &function.local_decls[local.0]);
                eprintln!("FRESH_STDLIB_TRACE actual-exit-origin kind={origin:?} declaration={declaration:?}");
                if let Some(declaration) = declaration {
                    fresh_source_span(&mut trace, declaration.span.as_ref());
                    fresh_type(&mut trace, &program.type_context, contract, declaration.ty);
                }
            }
        }
        // Only the native cases that really produced a missing-return diagnostic
        // are printed. Candidate collection was based solely on checked None.
        let transfers = trace.transfers.values().filter(|transfer| transfer.function == id)
            .map(|transfer| (transfer.statement.clone(), transfer.unmatched.clone(), transfer.before.clone(), transfer.after.clone())).collect::<Vec<_>>();
        for (statement, unmatched, before, after) in transfers {
            eprintln!("FRESH_STDLIB_TRACE actual-transfer function={id:?} statement={statement:?} unmatched={unmatched:?} BEFORE={before:?} AFTER={after:?}");
            fresh_source_span(&mut trace, statement.span.as_ref());
            let StatementKind::Assign(destination, Rvalue::Aggregate(kind, operands)) = &statement.kind else { unreachable!(); };
            fresh_path(&mut trace, program, function, destination);
            if let crate::mir::AggregateKind::EnumVariant { enum_id, variant_id, .. } = kind {
                match contract.nominal_layouts.get(enum_id) {
                    Some(MirNominalLayout::Enum { id: owner, variants, .. }) => {
                        assert_eq!(owner, enum_id);
                        let variant = variants.get(variant_id.0 as usize);
                        eprintln!("FRESH_STDLIB_TRACE actual-aggregate enum_owner={enum_id:?} canonical_variant={variant_id:?} variant_layout={variant:?}; origin path is unchanged");
                    }
                    other => eprintln!("FRESH_STDLIB_TRACE actual-aggregate enum_owner={enum_id:?} canonical_variant={variant_id:?} authority-gap=EnumLayoutUnavailable observed-layout={other:?}"),
                }
            }
            for (index, operand) in operands.iter().enumerate() {
                if let Operand::Copy(place) | Operand::Move(place) = operand {
                    let local = &function.local_decls[place.local.0];
                    eprintln!("FRESH_STDLIB_TRACE operand index={index} operand={operand:?} local-declaration={local:?}");
                    fresh_source_span(&mut trace, local.span.as_ref());
                    fresh_path(&mut trace, program, function, place);
                } else {
                    eprintln!("FRESH_STDLIB_TRACE operand index={index} constant={operand:?}");
                }
            }
            for place in unmatched { fresh_path(&mut trace, program, function, &place); }
        }
    }
}

fn fresh_stdlib_inputs(directory: &std::path::Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut inputs = std::collections::BTreeMap::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else if entry.file_type().unwrap().is_file() {
                inputs.insert(entry.path(), std::fs::read(entry.path()).unwrap());
            }
        }
    }
    inputs
}

#[test]
#[ignore = "fresh stdlib missing return-place type authority; reproduces one Borrow, never executes"]
fn capture_fresh_stdlib_return_place_type_authority() {
    assert!(fresh_enabled(), "set ROCK_FRESH_STDLIB_RETURN_TRACE=1");
    for flag in [
        "ROCK_OWNER_LOAN_TRACE", "ROCK_OWNER_RETURN_TRACE", "ROCK_OWNER_TRACE_SOURCE",
        "ROCK_OWNER_TRACE_STDLIB", "ROCK_ALLOCATOR_EXECUTION_DEBUG", "ROCK_ALLOCATOR_EXECUTION_MIR",
        "ROCK_ALLOCATOR_FRONTEND_ONLY", "ROCK_ALLOCATOR_TRACE_STDLIB", "ROCK_STATIC_CAPTURE_TRACE",
    ] {
        assert!(std::env::var_os(flag).is_none(), "unset other observer/debug/artifact flag {flag}");
    }
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let source_root = workspace.join("stdlib");
    let entry = source_root.join("lib.rk");
    let inputs = fresh_stdlib_inputs(&source_root);
    assert!(inputs.contains_key(&entry));
    let output = agent_work_path("fixtures").join(format!("rock_fresh_stdlib_return_authority_{}", std::process::id()));
    std::fs::create_dir(&output).expect("new isolated output directory; never reuse old stdlib artifacts");
    let config = crate::Config {
        entry_file: entry,
        output_dir: output.clone(),
        current_crate_name: Some("stdlib".into()),
        no_std: true,
        no_prelude: false,
        no_link: true,
        extern_artifacts: Vec::new(),
        emit_object: Some(output.join("stdlib.o")),
        ..Default::default()
    };
    assert!(config.debug_print.is_empty());
    std::fs::write(output.join("observer-config.txt"), format!("{config:#?}\n")).unwrap();
    eprintln!("FRESH_STDLIB_TRACE actual-primary-equivalent-config={config:?} source_snapshot_files={} output={output:?}; ONE compile_with_products, no cached artifact/app/native execution", inputs.len());
    *fresh_trace().lock().expect("fresh stdlib observer lock") = FreshStdlibTrace {
        inputs: inputs.clone(), ..Default::default()
    };
    let outcome = crate::compile_with_products(&config);
    assert_eq!(fresh_stdlib_inputs(&source_root), inputs, "all actual stdlib input bytes and filenames unchanged after compilation");
    let diagnostics = match outcome {
        Err(diagnostics) => diagnostics,
        Ok(_) => panic!("fresh stdlib must reproduce its current missing return-place type rejection; no execution permitted"),
    };
    assert!(diagnostics.0.len() <= 8, "FRESH_STDLIB_TRACE diagnostic budget overflow");
    for diagnostic in &diagnostics.0 {
        eprintln!("FRESH_STDLIB_TRACE diagnostic code={:?} location={:?} primary={:?} message={:?}",
            diagnostic.code, diagnostic.location, diagnostic.primary.as_ref().map(|label| &label.span), diagnostic.message);
    }
    let trace = fresh_trace().lock().expect("fresh stdlib observer lock");
    assert!(trace.program_seen && trace.headers > 0, "actual checker and checked native header observation required");
    assert_eq!(diagnostics.0.len(), 1, "exact current single fresh-stdlib missing-type diagnostic; no alternate errors accepted");
    assert_eq!(trace.missing.len(), 1, "exact actual missing return entry must match the one diagnostic");
    let case = &trace.missing[0];
    let diagnostic = &diagnostics.0[0];
    assert_eq!(diagnostic.code, Some(DiagnosticCode::Borrow));
    assert_eq!(diagnostic.severity, crate::diagnostic::DiagnosticSeverity::Error);
    assert!(matches!(diagnostic.location, crate::diagnostic::DiagnosticLocation::Toolchain));
    assert!(diagnostic.primary.is_none());
    assert_eq!(diagnostic.message, format!("Missing checked return-place type for {:?} in {:?}", case.place, case.function));
    assert_eq!(case.place, Place { local: Local(0), projection: vec![Projection::Field { index: 0, identity: None }] });
    assert!(case.reachable && !case.origins.is_empty());
    assert!(trace.transfers.values().any(|transfer| {
        transfer.function == case.function && transfer.unmatched.contains(&case.place)
    }), "actual failing return path must have a recorded unmatched aggregate BEFORE/AFTER transfer");
    assert!(!trace.source_spans.is_empty(), "actual failing body source text must be observed without synthetic fallbacks");
    eprintln!("FRESH_STDLIB_TRACE reproduced-single-negative actual_function={:?} path={:?} counts missing={} transfers={} visited_types={} layout_edges={} source_spans={} native_headers={} budgets=8/64/128/256; stdlib compile/runtime acceptance NOT established",
        case.function, case.place, trace.missing.len(), trace.transfers.len(), trace.types.len(), trace.edges, trace.source_spans.len(), trace.headers);
}
