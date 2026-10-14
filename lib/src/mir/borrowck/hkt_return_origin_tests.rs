//! Bounded receipts for the original HKT integration sources; never borrow authority.
//! Return receipts inspect Mono after rejection; pre-binding observations borrow
//! specialized bodies before consumption without running the FULL receipt graph.
//! absent source/HIR rows remain UNKNOWN, never a diagnostic span fallback.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use super::allocator_trace_tests::{
    agent_work_path, integration_literal, integration_sha, mutation_source_program_with_callback,
    mutation_source_program_with_phase_callbacks,
};
use super::*;
use crate::hir::{
    AcceptedHir, HirBlockFor, HirExprFor, HirExprKindFor, HirFunctionFor, HirStmtFor,
};
use crate::ids::{HirLocalId, InstanceId};
use crate::mir::MirInstanceBodies;
use crate::mono::{InstanceOrigin, MonomorphizedProgram};

const MESSAGE: &str = "Cannot return reference to local value because it does not live long enough";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BindingKind {
    Construction,
    Alias,
    Argument(usize),
    Capture(usize),
}

// Immediate observed edges only, not a new target/provenance analysis domain.
// The site owner is also the source-place owner; argument edges retain the
// originating invocation rather than pairing function-local unions as proof.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BindingEdge {
    site: (MirFunctionId, usize, usize),
    source: Option<Place>,
    destination: (MirFunctionId, Local),
    candidate: MirFunctionId,
    kind: BindingKind,
}

#[derive(Debug, Clone)]
enum SelectionReason {
    RejectedReturn {
        site: (MirFunctionId, usize, usize),
    },
    RequiredReturn {
        site: Option<(MirFunctionId, usize, usize)>,
    },
    ClosureOwner {
        child: MirFunctionId,
    },
    Construction {
        site: Option<(MirFunctionId, usize, usize)>,
    },
    CallableOperand {
        site: Option<(MirFunctionId, usize, usize)>,
    },
    Binding(BindingEdge),
    ObservedCall {
        site: (MirFunctionId, usize, usize),
    },
}

impl SelectionReason {
    fn role(&self) -> &'static str {
        match self {
            Self::RejectedReturn { .. } => "actual-rejected-return",
            Self::RequiredReturn { .. } => "required-return-request",
            Self::ClosureOwner { .. } => "construction-owner-observation",
            Self::Construction { .. } => "construction-observation",
            Self::CallableOperand { .. } => "literal-callable-observation",
            Self::Binding(_) => "binding-observation",
            Self::ObservedCall { .. } => "callback-use-observation",
        }
    }

    fn report(&self) {
        match self {
            Self::RejectedReturn { site } | Self::ObservedCall { site } =>
                eprintln!("HKT_RETURN_ORIGIN overflowCause role={} site={site:?}", self.role()),
            Self::RequiredReturn { site } | Self::Construction { site } | Self::CallableOperand { site } =>
                eprintln!("HKT_RETURN_ORIGIN overflowCause role={} site={site:?}; None means function-level root, not a fabricated site", self.role()),
            Self::ClosureOwner { child } =>
                eprintln!("HKT_RETURN_ORIGIN overflowCause role={} exactChild={child:?}; owner follows canonical closure ID", self.role()),
            Self::Binding(edge) =>
                eprintln!("HKT_RETURN_ORIGIN overflowCause role={} actualBindingEdge={edge:?}", self.role()),
        }
    }
}

#[derive(Default)]
struct Receipt {
    // FULL body selection: dependency traversal and every per-body replay.
    selected: BTreeSet<MirFunctionId>,
    // Canonical symbol/header/construction observations, including FULL IDs.
    symbols: BTreeSet<MirFunctionId>,
    absent_body_requests: BTreeSet<MirFunctionId>,
    callable_keys: BTreeSet<MirCallableKey>,
    rejected: BTreeSet<MirFunctionId>,
    locals: HashSet<(MirFunctionId, Local)>,
    // Header records are not dependency seeds: visiting an owner must not
    // select its return value or unrelated sibling closure definitions.
    dependencies: HashSet<(MirFunctionId, Local)>,
    // May-flow observation only: all construction/alias/argument alternatives
    // survive; this never resolves an indirect production call target.
    callable_bindings: HashMap<(MirFunctionId, Local), BTreeSet<MirFunctionId>>,
    binding_edges: HashSet<BindingEdge>,
    result_calls: HashSet<(MirFunctionId, usize)>,
    // Error context only; none of these rows drive semantic selection.
    selection_site: Option<(MirFunctionId, usize, usize)>,
    selection_roles: HashMap<MirFunctionId, BTreeSet<&'static str>>,
    last_new_selection: Option<(MirFunctionId, SelectionReason)>,
    last_new_symbol: Option<(MirFunctionId, SelectionReason)>,
    last_binding_edge: Option<BindingEdge>,
    rejection_rows: Vec<(MirFunctionId, usize, Place, OriginSet)>,
    emitter_summaries: HashMap<MirFunctionId, Option<OriginSet>>,
    source_locals: HashSet<(DefId, HirLocalId)>,
    origins: HashSet<(MirFunctionId, ReferenceOrigin)>,
    sites: HashSet<(MirFunctionId, usize, usize)>,
    source_sites: HashSet<(MirFunctionId, PathBuf, usize, usize, u8, usize)>,
    types: BTreeSet<TypeId>,
    rejections: usize,
}

thread_local! {
    static ACTIVE: RefCell<Option<Receipt>> = const { RefCell::new(None) };
}

struct Scope;

impl Scope {
    fn enter() -> Self {
        ACTIVE.with(|active| {
            assert!(active.borrow().is_none(), "HKT receipt scope must not nest");
            *active.borrow_mut() = Some(Receipt::default());
        });
        Self
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            active.borrow_mut().take();
        });
    }
}

impl Receipt {
    fn budget(&self) {
        if self.selected.len() > 16 || self.symbols.len() > 32 {
            assert!(self.selected.len() <= 17 && self.symbols.len() <= 33,
                "HKT overflow identity diagnostic exceeds cap+1: FULL={} symbols={}; no IDs listed or truncated", self.selected.len(), self.symbols.len());
            eprintln!("HKT_RETURN_ORIGIN overflowSelection FULLBodies={} symbols={} lastNewFULL={:?} lastNewSymbol={:?} lastObservedBindingEdge={:?}; diagnostic bounds FULL17/symbol33, admitted caps FULL16/symbol32; roles are requests/observations, NOT completed body/capture/return proof",
                self.selected.len(), self.symbols.len(), self.last_new_selection, self.last_new_symbol, self.last_binding_edge);
            if let Some((_, reason)) = &self.last_new_selection {
                reason.report();
            }
            if self.symbols.len() > 32 {
                if let Some((_, reason)) = &self.last_new_symbol {
                    reason.report();
                }
            }
            for id in &self.symbols {
                eprintln!("HKT_RETURN_ORIGIN overflowSelected id={id:?} FULL={} bodyRequestAbsent={} roles={:?} recordedRejected={} existingLocal0Dependency={}; dependency presence is not return-effect proof",
                    self.selected.contains(id), self.absent_body_requests.contains(id), self.selection_roles.get(id), self.rejected.contains(id), self.dependencies.contains(&(id.clone(), Local(0))));
            }
        }
        assert!(self.selected.len() <= 16 && self.symbols.len() <= 32 && self.callable_keys.len() <= 16
            && self.locals.len() + self.source_locals.len() <= 128
            && self.origins.len() <= 16 && self.sites.len() + self.source_sites.len() <= 192 && self.types.len() <= 64,
            "HKT_RETURN_ORIGIN overflow before formatting: FULLBodies={} symbols={} callableKeys={} locals={} (MIR={} sourceHIR={}) origins={} sites={} (MIR={} sourceHIR={}) types={} caps=FULL16/symbol32/keys16/locals128/origins16/sites192/types64; request measured scope/cap approval",
            self.selected.len(), self.symbols.len(), self.callable_keys.len(), self.locals.len() + self.source_locals.len(),
            self.locals.len(), self.source_locals.len(), self.origins.len(),
            self.sites.len() + self.source_sites.len(), self.sites.len(), self.source_sites.len(), self.types.len());
    }

    fn observe(&mut self, id: &MirFunctionId, reason: SelectionReason) {
        self.selection_roles
            .entry(id.clone())
            .or_default()
            .insert(reason.role());
        if self.symbols.insert(id.clone()) {
            self.last_new_symbol = Some((id.clone(), reason));
        }
        self.budget();
    }

    // Callers must have an actual MIR body, never merely a compatible header.
    fn select(&mut self, id: &MirFunctionId, reason: SelectionReason) {
        self.observe(id, reason.clone());
        if self.selected.insert(id.clone()) {
            self.last_new_selection = Some((id.clone(), reason));
        }
        self.budget();
    }

    fn local(&mut self, function: &MirFunction, local: Local) {
        self.locals.insert((function.id.clone(), local));
        if let Some(declaration) = function.local_decls.get(local.0) {
            self.types.insert(declaration.ty);
        }
        self.budget();
    }

    fn site(&mut self, id: &MirFunctionId, block: usize, index: usize) {
        assert!(
            self.selected.contains(id),
            "MIR state-site admission requires FULL body, not symbol-only observation"
        );
        self.selection_site = Some((id.clone(), block, index));
        self.sites.insert((id.clone(), block, index));
        self.budget();
    }

    fn need(&mut self, function: &MirFunction, local: Local) {
        assert!(
            self.selected.contains(&function.id),
            "definition dependency admission requires FULL body, not a capture header"
        );
        self.dependencies.insert((function.id.clone(), local));
        self.local(function, local);
    }

    fn needs(&self, id: &MirFunctionId, local: Local) -> bool {
        self.dependencies.contains(&(id.clone(), local))
    }

    fn bind(
        &mut self,
        function: &MirFunction,
        local: Local,
        candidates: &BTreeSet<MirFunctionId>,
        site: &(MirFunctionId, usize, usize),
        source: Option<Place>,
        kind: BindingKind,
    ) {
        if candidates.is_empty() {
            return;
        }
        if let BindingKind::Argument(ordinal) | BindingKind::Capture(ordinal) = kind {
            assert!(
                ordinal < 8,
                "HKT binding edge ordinal overflow before formatting"
            );
        }
        self.site(&site.0, site.1, site.2);
        if self.selected.contains(&function.id) {
            self.need(function, local);
        } else {
            self.local(function, local);
        }
        for candidate in candidates {
            let edge = BindingEdge {
                site: site.clone(),
                source: source.clone(),
                destination: (function.id.clone(), local),
                candidate: candidate.clone(),
                kind,
            };
            self.last_binding_edge = Some(edge.clone());
            self.observe(candidate, SelectionReason::Binding(edge.clone()));
            self.binding_edges.insert(edge);
        }
        self.callable_bindings
            .entry((function.id.clone(), local))
            .or_default()
            .extend(candidates.iter().cloned());
        self.budget();
    }

    fn origin_row(&mut self, id: &MirFunctionId, origins: &OriginSet) {
        for origin in origins {
            self.origins.insert((id.clone(), *origin));
        }
        self.budget();
    }

    fn header(&mut self, function: &MirFunction, contract: &MirBackendContract) {
        assert!(function.arg_count <= 8 && function.closure_captures.len() <= 8,
            "HKT_RETURN_ORIGIN header overflow before formatting: arguments={} captures={} cap=8 each",
            function.arg_count, function.closure_captures.len());
        self.types.insert(function.ret_type);
        for index in 0..=function.arg_count {
            self.local(function, Local(index));
        }
        for capture in &function.closure_captures {
            self.local(function, capture.local);
        }
        let key = contract.function_bodies.get(&function.id);
        if self.selected.contains(&function.id) {
            if let Some(key) = key {
                self.callable_keys.insert(key.clone());
            }
        }
        let declaration = key.and_then(|key| contract.callable(key));
        if let Some(declaration) = declaration {
            assert!(
                declaration.signature.params.len() <= 8,
                "HKT callable signature overflow"
            );
            for param in &declaration.signature.params {
                self.types.insert(param.semantic_ty);
            }
            self.types.insert(declaration.signature.ret.semantic_ty);
            self.types.insert(declaration.signature.ret.abi_ty);
        }
        self.budget();
        eprintln!("HKT_RETURN_ORIGIN header function={:?} key={key:?} sourceDef={:?} signature={:?} argCount={} returnType={:?} captures={:?} monoSourceCorrelation=PENDING_POSTRUN",
            function.id, declaration.and_then(|decl| decl.source_def_id),
            declaration.map(|decl| &decl.signature), function.arg_count, function.ret_type,
            function.closure_captures);
        for index in 0..=function.arg_count {
            eprintln!(
                "HKT_RETURN_ORIGIN headerLocal function={:?} local={index} declaration={:?}",
                function.id,
                function.local_decls.get(index)
            );
        }
        for capture in &function.closure_captures {
            eprintln!("HKT_RETURN_ORIGIN captureBinding function={:?} capture={capture:?} declaration={:?}",
                function.id, function.local_decls.get(capture.local.0));
        }
    }
}

pub(super) fn local_return_reject(
    analysis: &ReferenceEscapeAnalysis<'_>,
    block: usize,
    place: &Place,
    origins: &OriginSet,
    local: Local,
    checked_type: TypeId,
) {
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        let Some(receipt) = active.as_mut() else { return; };
        let function = analysis.func;
        receipt.select(&function.id, SelectionReason::RejectedReturn {
            site: (function.id.clone(), block, usize::MAX),
        });
        let first = receipt.rejected.insert(function.id.clone());
        receipt.need(function, local);
        need_place(receipt, function, place);
        receipt.need(function, Local(0));
        receipt.types.insert(checked_type);
        receipt.site(&function.id, block, usize::MAX);
        assert!(origins.len() <= 16, "HKT return origin row overflow before formatting");
        for origin in origins { receipt.origins.insert((function.id.clone(), *origin)); }
        if let Some(summary) = analysis.return_summaries.get(&function.id) {
            assert!(summary.len() <= 16, "HKT Lifetime summary overflow before formatting");
            for origin in summary { receipt.origins.insert((function.id.clone(), *origin)); }
        }
        receipt.rejections += 1;
        assert!(receipt.rejections <= 16, "HKT actual rejection-site overflow before formatting");
        receipt.rejection_rows.push((function.id.clone(), block, place.clone(), origins.clone()));
        receipt.emitter_summaries.insert(function.id.clone(), analysis.return_summaries.get(&function.id).cloned());
        receipt.budget();
        if first { receipt.header(function, analysis.backend_contract); }
        let declaration = function.local_decls.get(local.0);
        eprintln!("HKT_RETURN_ORIGIN actualReject function={:?} block={block} place={place:?} checkedType={checked_type:?} semanticType={:?} rejectedLocal={local:?} declarationExists={} declaration={declaration:?} local0={} missingSpan={} exitOrigin={:?} origins={origins:?} LifetimeSummary={:?} ValueLoans=UNKNOWN(not-supplied-to-emitter) sourceExpectation=PENDING_POSTRUN",
            function.id, analysis.type_context.type_for(checked_type), declaration.is_some(), local.0 == 0,
            declaration.is_some_and(|decl| decl.span.is_none()),
            function.basic_blocks[block].terminator.as_ref().map(Terminator::origin),
            analysis.return_summaries.get(&function.id));
    });
}

fn operand_type(program: &MirProgram, function: &MirFunction, operand: &Operand) -> Option<TypeId> {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => {
            program
                .backend_contract
                .place_type_id(&program.type_context, function, place)
        }
        // Constant payloads (including TypeId metadata) do not establish their
        // operand's semantic type; retain UNKNOWN instead of guessing it.
        _ => None,
    }
}

fn need_place(receipt: &mut Receipt, function: &MirFunction, place: &Place) {
    // Base-local closure includes every write/branch alternative, not a
    // projected-value or dereference alias proof; preserve that UNKNOWN.
    receipt.need(function, place.local);
    for projection in &place.projection {
        if let Projection::Index(index) = projection {
            receipt.need(function, *index);
        }
    }
}

fn need_operand(
    receipt: &mut Receipt,
    program: &MirProgram,
    function: &MirFunction,
    operand: &Operand,
) {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => need_place(receipt, function, place),
        Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) => {
            receipt.callable_keys.insert(key.clone());
            receipt.budget();
            // A literal closure ID is actual binding authority, not an
            // inferred target for any indirect call reading this local.
            if let MirCallableKey::Closure(id) = key {
                receipt.observe(
                    id,
                    SelectionReason::CallableOperand {
                        site: receipt.selection_site.clone(),
                    },
                );
                if let Some(body) = program.function(id.clone()) {
                    for capture in &body.closure_captures {
                        receipt.local(body, capture.local);
                    }
                }
            }
        }
        _ => {}
    }
}

fn request_body(
    receipt: &mut Receipt,
    program: &MirProgram,
    id: &MirFunctionId,
    reason: SelectionReason,
) {
    let Some(function) = program.function(id.clone()) else {
        receipt.absent_body_requests.insert(id.clone());
        receipt.observe(id, reason);
        return; // Actual body ABSENT: no empty-borrow or header return proof.
    };
    receipt.select(id, reason);
    // Promote previously observed incoming/capture bindings with the body;
    // retaining all their alternatives must not depend on discovery order.
    let locals = receipt
        .callable_bindings
        .keys()
        .filter_map(|(owner, local)| (owner == id).then_some(*local))
        .collect::<Vec<_>>();
    for local in locals {
        receipt.need(function, local);
    }
}

fn need_return(
    receipt: &mut Receipt,
    program: &MirProgram,
    id: &MirFunctionId,
    site: Option<(MirFunctionId, usize, usize)>,
) {
    request_body(
        receipt,
        program,
        id,
        SelectionReason::RequiredReturn { site },
    );
    if let Some(function) = program.function(id.clone()) {
        receipt.need(function, Local(0));
        for capture in &function.closure_captures {
            receipt.need(function, capture.local);
        }
    }
}

fn need_rvalue(
    receipt: &mut Receipt,
    program: &MirProgram,
    function: &MirFunction,
    value: &Rvalue,
) {
    match value {
        Rvalue::Use(operand)
        | Rvalue::Cast(operand, _)
        | Rvalue::Object(operand, _)
        | Rvalue::Package(operand, _)
        | Rvalue::UnaryOp(_, operand) => need_operand(receipt, program, function, operand),
        Rvalue::Ref(_, place) | Rvalue::Discriminant(place) => need_place(receipt, function, place),
        Rvalue::BinaryOp(_, left, right) => {
            need_operand(receipt, program, function, left);
            need_operand(receipt, program, function, right);
        }
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                need_operand(receipt, program, function, operand);
            }
        }
        Rvalue::Closure(closure) => {
            need_closure(receipt, program, function, closure);
        }
    }
}

fn need_closure(
    receipt: &mut Receipt,
    program: &MirProgram,
    function: &MirFunction,
    closure: &crate::mir::MirClosure,
) {
    assert!(
        closure.captures.len() <= 8,
        "HKT construction capture overflow before formatting"
    );
    let id = MirFunctionId::Closure(Box::new(closure.id.clone()));
    receipt.observe(
        &id,
        SelectionReason::Construction {
            site: receipt.selection_site.clone(),
        },
    );
    // Construction/capture ancestry needs the environment, not the result
    // of hypothetically invoking this closure. Do not seed its Local0 here.
    if let Some(body) = program.function(id) {
        for capture in &body.closure_captures {
            receipt.local(body, capture.local);
        }
    }
    for capture in &closure.captures {
        receipt.need(function, capture.local);
    }
}

fn callable_storage_local(
    program: &MirProgram,
    function: &MirFunction,
    place: &Place,
) -> Option<Local> {
    let mut prefix = Place {
        local: place.local,
        projection: Vec::new(),
    };
    for projection in &place.projection {
        if *projection != Projection::Deref {
            return None;
        }
        let ty =
            program
                .backend_contract
                .place_type_id(&program.type_context, function, &prefix)?;
        if !matches!(
            program
                .backend_contract
                .normalize_type(&program.type_context, &program.type_context.type_for(ty)),
            Type::Reference { .. }
        ) {
            return None;
        }
        prefix.projection.push(projection.clone());
    }
    let ty = program
        .backend_contract
        .place_type_id(&program.type_context, function, place)?;
    let ty = program
        .backend_contract
        .normalize_type(&program.type_context, &program.type_context.type_for(ty));
    let mut view = &ty;
    while let Type::Reference { inner, .. } = view {
        view = inner;
    }
    matches!(view, Type::Function { .. }).then_some(place.local)
}

fn binding_candidates(
    receipt: &Receipt,
    program: &MirProgram,
    function: &MirFunction,
    operand: &Operand,
) -> BTreeSet<MirFunctionId> {
    match operand {
        Operand::Constant(Constant::Callable(callable)) => {
            callable.function_id().into_iter().collect()
        }
        Operand::Copy(place) | Operand::Move(place) => {
            callable_storage_local(program, function, place)
                .and_then(|local| receipt.callable_bindings.get(&(function.id.clone(), local)))
                .cloned()
                .unwrap_or_default()
        }
        // Only checked reference reborrows preserve whole-slot observations;
        // raw/field/index projections are not callable reconstruction. Absence is
        // UNKNOWN, not proof that an operand has no callable environment.
        _ => BTreeSet::new(),
    }
}

fn value_bindings(
    receipt: &Receipt,
    program: &MirProgram,
    function: &MirFunction,
    value: &Rvalue,
) -> BTreeSet<MirFunctionId> {
    match value {
        Rvalue::Closure(closure) => {
            BTreeSet::from([MirFunctionId::Closure(Box::new(closure.id.clone()))])
        }
        Rvalue::Use(operand)
        | Rvalue::Cast(operand, _)
        | Rvalue::Object(operand, _)
        | Rvalue::Package(operand, _) => binding_candidates(receipt, program, function, operand),
        Rvalue::Ref(_, place) => callable_storage_local(program, function, place)
            .and_then(|local| receipt.callable_bindings.get(&(function.id.clone(), local)))
            .cloned()
            .unwrap_or_default(),
        _ => BTreeSet::new(),
    }
}

fn binding_source(operand: &Operand) -> Option<Place> {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => Some(place.clone()),
        _ => None,
    }
}

fn value_binding_source(value: &Rvalue) -> Option<Place> {
    match value {
        Rvalue::Use(operand)
        | Rvalue::Cast(operand, _)
        | Rvalue::Object(operand, _)
        | Rvalue::Package(operand, _) => binding_source(operand),
        Rvalue::Ref(_, place) => Some(place.clone()),
        _ => None,
    }
}

fn argument_binding_agrees(
    program: &MirProgram,
    caller: &MirFunction,
    target: &MirFunction,
    args: &[Operand],
) -> bool {
    let Some(key) = program.backend_contract.function_bodies.get(&target.id) else {
        return false;
    };
    let Some(header) = program.backend_contract.callable(key) else {
        return false;
    };
    args.len() == target.arg_count
        && header.signature.params.len() == args.len()
        && header
            .signature
            .params
            .iter()
            .zip(args)
            .enumerate()
            .all(|(index, (param, arg))| {
                let Some(actual) = operand_type(program, caller, arg) else {
                    return false;
                };
                target
                    .local_decls
                    .get(index + 1)
                    .is_some_and(|decl| decl.ty == param.semantic_ty)
                    && program.backend_contract.normalize_type(
                        &program.type_context,
                        &program.type_context.type_for(actual),
                    ) == program.backend_contract.normalize_type(
                        &program.type_context,
                        &program.type_context.type_for(param.semantic_ty),
                    )
            })
}

fn construction_bindings(
    receipt: &mut Receipt,
    program: &MirProgram,
    parent: &MirFunction,
    destination: &Place,
    closure: &crate::mir::MirClosure,
    site: &(MirFunctionId, usize, usize),
) {
    let id = MirFunctionId::Closure(Box::new(closure.id.clone()));
    if destination.projection.is_empty() {
        receipt.bind(
            parent,
            destination.local,
            &BTreeSet::from([id.clone()]),
            site,
            None,
            BindingKind::Construction,
        );
    }
    let Some(body) = program.function(id) else {
        return;
    };
    assert!(
        closure.captures.len() <= 8 && body.closure_captures.len() <= 8,
        "HKT capture binding overflow before formatting"
    );
    if closure.captures.len() != body.closure_captures.len() {
        return;
    } // UNKNOWN, rendered with both actual headers.
    for (ordinal, (capture, binding)) in closure
        .captures
        .iter()
        .zip(&body.closure_captures)
        .enumerate()
    {
        let candidates = receipt
            .callable_bindings
            .get(&(parent.id.clone(), capture.local))
            .cloned()
            .unwrap_or_default();
        let (Some(source), Some(destination)) = (
            parent.local_decls.get(capture.local.0),
            body.local_decls.get(binding.local.0),
        ) else {
            continue;
        };
        let ty = program.backend_contract.normalize_type(
            &program.type_context,
            &program.type_context.type_for(source.ty),
        );
        let expected = match capture.kind {
            crate::mir::MirClosureCaptureKind::ByValue => ty,
            crate::mir::MirClosureCaptureKind::ByRef => Type::Reference {
                mutable: false,
                inner: Box::new(ty),
            },
            crate::mir::MirClosureCaptureKind::ByMutRef => Type::Reference {
                mutable: true,
                inner: Box::new(ty),
            },
        };
        if program.backend_contract.normalize_type(
            &program.type_context,
            &program.type_context.type_for(destination.ty),
        ) == expected
        {
            receipt.bind(
                body,
                binding.local,
                &candidates,
                site,
                Some(capture.place()),
                BindingKind::Capture(ordinal),
            );
        }
    }
}

fn observed_bind(span: Option<&Span>) -> bool {
    let expected = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("stdlib/vec.rk");
    span.is_some_and(|span| span.start == 11676 && span.end == 11683 && span.file_path == expected)
}

fn select_dependencies(receipt: &mut Receipt, program: &MirProgram) {
    for id in receipt.rejected.clone() {
        need_return(receipt, program, &id, None);
    }
    loop {
        let before = (
            receipt.selected.len(),
            receipt.symbols.len(),
            receipt.dependencies.len(),
            receipt
                .callable_bindings
                .values()
                .map(BTreeSet::len)
                .sum::<usize>(),
        );
        for id in receipt.selected.clone() {
            // Follow every exact construction of this selected child, never
            // enumerate its siblings just because their parent is selected.
            if let MirFunctionId::Closure(closure_id) = &id {
                // The parent is FULL because its construction state is replayed.
                request_body(
                    receipt,
                    program,
                    &closure_id.owner,
                    SelectionReason::ClosureOwner { child: id.clone() },
                );
                if let Some(body) = program.function(id.clone()) {
                    for capture in &body.closure_captures {
                        receipt.need(body, capture.local);
                    }
                }
                if let Some(parent) = program.function(closure_id.owner.clone()) {
                    for (block_index, block) in parent.basic_blocks.iter().enumerate() {
                        for (index, statement) in block.statements.iter().enumerate() {
                            let StatementKind::Assign(destination, Rvalue::Closure(closure)) =
                                &statement.kind
                            else {
                                continue;
                            };
                            if &closure.id != closure_id.as_ref() {
                                continue;
                            }
                            receipt.site(&parent.id, block_index, index);
                            need_place(receipt, parent, destination);
                            need_closure(receipt, program, parent, closure);
                            construction_bindings(
                                receipt,
                                program,
                                parent,
                                destination,
                                closure,
                                &(parent.id.clone(), block_index, index),
                            );
                        }
                    }
                }
            }
            let Some(function) = program.function(id.clone()) else {
                continue;
            };
            for (block_index, block) in function.basic_blocks.iter().enumerate() {
                for (index, statement) in block.statements.iter().enumerate() {
                    if let StatementKind::Assign(destination, value) = &statement.kind {
                        let candidates = value_bindings(receipt, program, function, value);
                        // Only already-observed callable aliases flow forward;
                        // unrelated sibling constructions do not seed a family.
                        let forward = !candidates.is_empty()
                            && match value {
                                Rvalue::Use(operand)
                                | Rvalue::Cast(operand, _)
                                | Rvalue::Object(operand, _)
                                | Rvalue::Package(operand, _) => {
                                    matches!(operand, Operand::Copy(_) | Operand::Move(_))
                                }
                                Rvalue::Ref(_, _) => true,
                                _ => false,
                            };
                        if !receipt.needs(&id, destination.local) && !forward {
                            continue;
                        }
                        receipt.site(&id, block_index, index);
                        need_place(receipt, function, destination);
                        need_rvalue(receipt, program, function, value);
                        if let Rvalue::Closure(closure) = value {
                            construction_bindings(
                                receipt,
                                program,
                                function,
                                destination,
                                closure,
                                &(id.clone(), block_index, index),
                            );
                        } else if destination.projection.is_empty() {
                            receipt.bind(
                                function,
                                destination.local,
                                &candidates,
                                &(id.clone(), block_index, index),
                                value_binding_source(value),
                                BindingKind::Alias,
                            );
                        }
                    }
                }
                match &block.terminator {
                    Some(Terminator::Call {
                        func,
                        args,
                        destination,
                        span,
                        ..
                    }) => {
                        // Decide before observing this use: callback flow alone
                        // must not turn its result into a dependency seed.
                        let result_needed = receipt.needs(&id, destination.local);
                        let targets = binding_candidates(receipt, program, function, func);
                        let func_flow = matches!(func, Operand::Copy(_) | Operand::Move(_))
                            && !targets.is_empty();
                        let argument_flow = args.iter().any(|arg| {
                            matches!(arg, Operand::Copy(_) | Operand::Move(_))
                                && !binding_candidates(receipt, program, function, arg).is_empty()
                        });
                        if !result_needed
                            && !observed_bind(span.as_ref())
                            && !func_flow
                            && !argument_flow
                        {
                            continue;
                        }
                        assert!(
                            args.len() <= 8,
                            "HKT call argument overflow before formatting"
                        );
                        receipt.site(&id, block_index, usize::MAX);
                        if result_needed {
                            receipt.result_calls.insert((id.clone(), block_index));
                            need_place(receipt, function, destination);
                        } else {
                            receipt.local(function, destination.local);
                        }
                        need_operand(receipt, program, function, func);
                        for arg in args {
                            need_operand(receipt, program, function, arg);
                        }
                        // Preserve every observed body/binding alternative, but
                        // inspect returns only for an actually needed result.
                        // No observation installs an indirect semantic target.
                        for target in targets {
                            if result_needed {
                                need_return(
                                    receipt,
                                    program,
                                    &target,
                                    Some((id.clone(), block_index, usize::MAX)),
                                );
                            } else {
                                receipt.observe(
                                    &target,
                                    SelectionReason::ObservedCall {
                                        site: (id.clone(), block_index, usize::MAX),
                                    },
                                );
                            }
                            if let Some(body) = program.function(target.clone()) {
                                // Checked argument/header ordinals only. These
                                // are may-bound values, not installed call targets.
                                if argument_binding_agrees(program, function, body, args) {
                                    for (index, arg) in args.iter().enumerate() {
                                        let candidates =
                                            binding_candidates(receipt, program, function, arg);
                                        if !candidates.is_empty() {
                                            // Following incoming bindings enters this observed
                                            // body alternative: charge FULL, not a header.
                                            // It does not resolve an indirect call target.
                                            request_body(
                                                receipt,
                                                program,
                                                &target,
                                                SelectionReason::ObservedCall {
                                                    site: (id.clone(), block_index, usize::MAX),
                                                },
                                            );
                                        }
                                        receipt.bind(
                                            body,
                                            Local(index + 1),
                                            &candidates,
                                            &(id.clone(), block_index, usize::MAX),
                                            binding_source(arg),
                                            BindingKind::Argument(index),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
                        if receipt.needs(&id, Local(0)) =>
                    {
                        receipt.site(&id, block_index, usize::MAX)
                    }
                    Some(
                        Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. },
                    ) if receipt.needs(&id, place.local) => {
                        receipt.site(&id, block_index, usize::MAX)
                    }
                    _ => {}
                }
            }
        }
        if before
            == (
                receipt.selected.len(),
                receipt.symbols.len(),
                receipt.dependencies.len(),
                receipt
                    .callable_bindings
                    .values()
                    .map(BTreeSet::len)
                    .sum::<usize>(),
            )
        {
            break;
        }
    }
}

fn post_run(program: &MirProgram) {
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        let receipt = active.as_mut().unwrap();
        select_dependencies(receipt, program);
        assert!(receipt.rejected.is_subset(&receipt.selected), "all actual emitter rejections require FULL body receipts");
        assert!(receipt.sites.iter().all(|(owner, _, _)| receipt.selected.contains(owner)),
            "every actual state-replay site owner must be FULL, including construction parents");
        eprintln!("HKT_RETURN_ORIGIN selectionBoundary rejected={} FULLBodies={} symbols={} absentBodyRequests={} dependencyLocals={} requiredResultCalls={} bindingEdges={}; FULL16/symbol32 are separate observer domains; constructor/header-only children do not enter definitions/replay; parents and incoming-binding traversal count FULL; cross-invocation pairing and unresolved projected/erased/indirect/virtual targets UNKNOWN",
            receipt.rejected.len(), receipt.selected.len(), receipt.symbols.len(), receipt.absent_body_requests.len(), receipt.dependencies.len(), receipt.result_calls.len(), receipt.binding_edges.len());
        // Every edge is charged to the existing site/local/callable bounds;
        // argument/capture ordinals are checked before insertion, never cut off.
        receipt.budget();
        for edge in &receipt.binding_edges {
            eprintln!("HKT_RETURN_ORIGIN bindingEdge site={:?} kind={:?} sourcePlace={:?} destination={:?} canonicalCandidate={:?}; immediate observed transfer, source owner=site owner; originating invocation retained, function-local union is NOT call-site pairing/target/retention proof",
                edge.site, edge.kind, edge.source, edge.destination, edge.candidate);
        }
        for id in receipt.symbols.clone() {
            eprintln!("HKT_RETURN_ORIGIN workset function={id:?} FULL={} symbolObserved=true actualMIRBodyPresent={} requestedBodyAbsent={}; header/construction observation is NOT runtime result/return/capture-independence proof",
                receipt.selected.contains(&id), program.function(id.clone()).is_some(), receipt.absent_body_requests.contains(&id));
            let Some(function) = program.function(id.clone()) else {
                eprintln!("HKT_RETURN_ORIGIN function={id:?} actualMIRBody=ABSENT effects=UNKNOWN; symbol metadata cannot supply empty borrow authority");
                continue;
            };
            receipt.header(function, &program.backend_contract);
            for ((owner, local), candidates) in &receipt.callable_bindings {
                if owner == &id {
                    eprintln!("HKT_RETURN_ORIGIN callableStorage function={id:?} local={local:?} constructionOrBindingAlternatives={candidates:?}; union may-flow observation, originating sites in bindingEdge rows; cross-invocation pairing UNKNOWN, NOT resolved call/retention authority");
                }
            }
            if let MirFunctionId::Closure(closure) = &id {
                let constructions = receipt.sites.iter().filter(|(owner, block, index)| {
                    owner == &closure.owner && program.function(owner.clone())
                        .and_then(|parent| parent.basic_blocks.get(*block))
                        .and_then(|block| block.statements.get(*index))
                        .is_some_and(|statement| matches!(&statement.kind,
                            StatementKind::Assign(_, Rvalue::Closure(value)) if &value.id == closure.as_ref()))
                }).count();
                eprintln!("HKT_RETURN_ORIGIN ancestry child={id:?} actualOwner={:?} recordedConstructions={constructions}; absent construction is UNKNOWN", closure.owner);
            }
            for (owner, local) in &receipt.dependencies {
                if owner == &id {
                    eprintln!("HKT_RETURN_ORIGIN dependencyLocal function={id:?} local={local:?} declaration={:?} sourceHIRLink=UNKNOWN(unless-header-or-capture-binding-recorded)", function.local_decls.get(local.0));
                }
            }
            if !receipt.selected.contains(&id) { continue; }
            for (block_index, block) in function.basic_blocks.iter().enumerate() {
                for (index, statement) in block.statements.iter().enumerate() {
                    if !receipt.sites.contains(&(id.clone(), block_index, index)) { continue; }
                    if let StatementKind::Assign(destination, Rvalue::Closure(closure)) = &statement.kind {
                        assert!(closure.captures.len() <= 8, "HKT construction capture overflow before formatting");
                        receipt.site(&id, block_index, index);
                        receipt.local(function, destination.local);
                        for capture in &closure.captures { receipt.local(function, capture.local); }
                        eprintln!("HKT_RETURN_ORIGIN construction function={id:?} block={block_index} index={index} destination={destination:?} closure={:?} origin={:?} captures={:?}",
                            closure.id, statement.origin(), closure.captures);
                    } else {
                        eprintln!("HKT_RETURN_ORIGIN dependencyDefinition function={id:?} block={block_index} index={index} statement={:?} origin={:?}; base-local dependency only, projected/alias/callable flow UNKNOWN", statement.kind, statement.origin());
                    }
                }
                if !receipt.sites.contains(&(id.clone(), block_index, usize::MAX)) { continue; }
                let Some(terminator) = &block.terminator else { continue; };
                if let Terminator::Call { func, args, destination, span, .. } = terminator {
                    assert!(args.len() <= 8, "HKT call argument overflow before formatting");
                    receipt.site(&id, block_index, usize::MAX);
                    let result = program.backend_contract.place_type_id(&program.type_context, function, destination);
                    let arguments = args.iter().map(|arg| operand_type(program, function, arg)).collect::<Vec<_>>();
                    receipt.types.extend(result);
                    receipt.types.extend(arguments.iter().flatten().copied());
                    receipt.budget();
                    let target = match func {
                        Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) => Some(key),
                        _ => None,
                    };
                    if let Some(key) = target { receipt.callable_keys.insert(key.clone()); }
                    receipt.budget();
                    assert!(!matches!(target, Some(MirCallableKey::Intrinsic(
                        MirIntrinsicId::BorrowSlice | MirIntrinsicId::BorrowSliceMut
                        | MirIntrinsicId::ArrayRefToSlice))),
                        "HKT_RETURN_ORIGIN overlap with primitive slice frontier at actual function={id:?} block={block_index}; stop, loan roles UNMEASURED and owned by slice lane");
                    let header = target.and_then(|key| program.backend_contract.callable(key));
                    if let Some(header) = header {
                        assert!(header.signature.params.len() <= 8, "HKT target signature overflow");
                        receipt.types.extend(header.signature.params.iter().map(|param| param.semantic_ty));
                        receipt.types.insert(header.signature.ret.semantic_ty);
                        receipt.types.insert(header.signature.ret.abi_ty);
                    }
                    receipt.budget();
                    let flow_targets = binding_candidates(receipt, program, function, func);
                    eprintln!("HKT_RETURN_ORIGIN callUse function={id:?} block={block_index} requiredResultAncestry={} actualFunc={func:?} callableBindingAlternatives={flow_targets:?}; checked whole-slot/reference-reborrow observations only, absent/other projected/erased bindings and cross-invocation pairing UNKNOWN; stop at absent binding, no compatible-type candidate sweep or indirect-target promotion",
                        receipt.result_calls.contains(&(id.clone(), block_index)));
                    if flow_targets.is_empty() && matches!(func, Operand::Copy(_) | Operand::Move(_)) {
                        eprintln!("HKT_RETURN_ORIGIN bindingGap function={id:?} block={block_index} role=callee actualOperand={func:?} authority=UNKNOWN; stop target expansion, request only this call's bounded pre-consumption typedHIR/capture-binding snapshot if needed; no snapshot fabricated here");
                    }
                    for (ordinal, argument) in args.iter().enumerate() {
                        let candidates = binding_candidates(receipt, program, function, argument);
                        eprintln!("HKT_RETURN_ORIGIN argumentStorage function={id:?} block={block_index} argument={ordinal} operand={argument:?} callableBindingAlternatives={candidates:?}");
                        if candidates.is_empty() && arguments[ordinal].is_some_and(|ty| {
                            let ty = program.backend_contract.normalize_type(&program.type_context, &program.type_context.type_for(ty));
                            let mut view = &ty;
                            while let Type::Reference { inner, .. } = view { view = inner; }
                            matches!(view, Type::Function { .. })
                        }) {
                            eprintln!("HKT_RETURN_ORIGIN bindingGap function={id:?} block={block_index} role=arg{ordinal} actualOperand={argument:?} authority=UNKNOWN; Fn type identifies only an observation gap, never a target; request only this call's bounded pre-consumption typedHIR/capture-binding snapshot if needed");
                        }
                    }
                    for candidate in flow_targets {
                        eprintln!("HKT_RETURN_ORIGIN argumentHeaderLink caller={id:?} block={block_index} bodyCandidate={candidate:?} checkedOrdinalAgreement={}; binding candidates remain alternatives, not installed targets",
                            program.function(candidate.clone()).is_some_and(|body| argument_binding_agrees(program, function, body, args)));
                    }
                    eprintln!("HKT_RETURN_ORIGIN call function={id:?} block={block_index} target={target:?} targetSourceDef={:?} span={span:?} args={args:?} argumentTypes={arguments:?} destination={destination:?} resultType={result:?} targetSignature={:?} substitutions=PENDING_POSTRUN; call-input dependency is observational, result/argument lifetime relation and indirect target UNKNOWN",
                        header.and_then(|header| header.source_def_id), header.map(|header| &header.signature));
                } else if matches!(terminator, Terminator::Return | Terminator::ReturnWithOrigin { .. }
                    | Terminator::Drop { .. } | Terminator::DropWithOrigin { .. }) {
                    receipt.site(&id, block_index, usize::MAX);
                    eprintln!("HKT_RETURN_ORIGIN exitOrCleanup function={id:?} block={block_index} operation={terminator:?} origin={:?}", terminator.origin());
                }
            }
        }
        for ty in &receipt.types {
            eprintln!("HKT_RETURN_ORIGIN type={ty:?} semantic={:?}", program.type_context.type_for(*ty));
        }
        eprintln!("HKT_RETURN_ORIGIN MIRTotals rejectedCallables={} FULLBodies={} symbols={} callableKeys={} locals={} origins={} sites={} types={} rejections={} caps=FULL16/symbol32/keys16/locals128/origins16/sites192/types64 argumentsAndCaptures=8; monoSourceCorrelation=PENDING_POSTRUN",
            receipt.rejected.len(), receipt.selected.len(), receipt.symbols.len(), receipt.callable_keys.len(), receipt.locals.len(), receipt.origins.len(),
            receipt.sites.len(), receipt.types.len(), receipt.rejections);
    });
}

fn operand_origins_row(
    receipt: &mut Receipt,
    program: &MirProgram,
    analysis: &ReferenceEscapeAnalysis<'_>,
    state: &ReferenceOriginState,
    block: usize,
    role: &str,
    operand: &Operand,
) {
    let origins = analysis.origins_for_operand(state, operand);
    let ty = operand_type(program, analysis.func, operand);
    receipt.types.extend(ty);
    receipt.origin_row(&analysis.func.id, &origins);
    let declaration = match operand {
        Operand::Copy(place) | Operand::Move(place) => analysis.func.local_decls.get(place.local.0),
        _ => None,
    };
    eprintln!("HKT_RETURN_ORIGIN callInput function={:?} block={block} role={role} operand={operand:?} type={ty:?} semanticType={:?} LifetimeOrigins={origins:?} source={:?} declarationTemporary={:?} TemporaryOrigins={:?}; callable absence/retention UNKNOWN unless separately recorded",
        analysis.func.id, ty.map(|ty| program.type_context.type_for(ty)), declaration.map(|decl| decl.source),
        declaration.map(|decl| decl.source.is_temporary()),
        origins.iter().filter(|origin| matches!(origin, ReferenceOrigin::Temporary(_))).collect::<Vec<_>>());
}

fn replay_origins(program: &MirProgram) {
    // Recompute through the same phase-owned functions, never checker.run or
    // a substitute summary. Compare against captured emitter authority below.
    let lifetime = BorrowChecker::reference_return_summaries(program);
    let values = BorrowChecker::native_return_loan_proof(program);
    let mutations = NativeMutationTable::for_program(program);
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        let receipt = active.as_mut().unwrap();
        for (id, recorded) in &receipt.emitter_summaries {
            assert_eq!(lifetime.get(id), recorded.as_ref(), "HKT replay Lifetime differs from actual Local-reject emitter authority");
        }
        for id in receipt.selected.clone() {
            let function = program.function(id.clone()).expect("FULL replay admission requires an actual MIR body");
            // Reuse the checked ValueLoans/mutation phase, including its exact
            // current liveness/alias authority; no duplicated loan algorithm.
            // Replay diagnostics never replace the original Borrow verdict.
            let (replay_diagnostics, mutation_origins) = BorrowChecker::validate_moves_with_return_sources(
                function, &program.type_context, &program.backend_contract, &values.summaries, &mutations, Some(&values.sources),
            );
            let mut analysis = ReferenceEscapeAnalysis::new(function, &program.type_context,
                &program.backend_contract, &lifetime, MissingSummaryMode::Conservative).with_return_sources(Some(&values.sources));
            analysis.mutation_origins = Some(&mutation_origins);
            let results = run_fixpoint(&analysis, function);
            for (owner, block, place, origins) in &receipt.rejection_rows {
                if owner == &id {
                    assert!(results.reachable[*block], "actual rejected return must be replay-reachable");
                    assert_eq!(results.exit_sets[*block].origins.get(place), Some(origins),
                        "HKT replay checked return envelope differs from actual Local-reject emitter");
                }
            }
            for origins in [lifetime.get(&id), values.summaries.get(&id)].into_iter().flatten() {
                receipt.origin_row(&id, origins);
            }
            eprintln!("HKT_RETURN_ORIGIN replayHeader function={id:?} LifetimeSummary={:?} ValueLoansSummary={:?} replayMoveDiagnosticCount={} emitterEqualityChecked={}; ValueLoans remains separate from Lifetime, absent summary UNKNOWN",
                lifetime.get(&id), values.summaries.get(&id), replay_diagnostics.0.len(), receipt.rejected.contains(&id));
            for (block_index, block) in function.basic_blocks.iter().enumerate() {
                let recorded = receipt.sites.iter().any(|(owner, selected_block, _)| owner == &id && *selected_block == block_index);
                if !recorded { continue; }
                if !results.reachable[block_index] {
                    eprintln!("HKT_RETURN_ORIGIN replaySite function={id:?} block={block_index} reachable=false; syntactic alternatives retained, no reachable origin fabricated");
                    continue;
                }
                let mut state = results.entry_sets[block_index].clone();
                for (index, statement) in block.statements.iter().enumerate() {
                    let observe = receipt.sites.contains(&(id.clone(), block_index, index));
                    if observe {
                        if let StatementKind::Assign(_, Rvalue::Closure(closure)) = &statement.kind {
                            assert!(closure.captures.len() <= 8, "HKT replay capture row overflow");
                            for (ordinal, capture) in closure.captures.iter().enumerate() {
                                let origins = match capture.kind {
                                    crate::mir::MirClosureCaptureKind::ByValue => analysis.origins_for_place(&state, &capture.place()),
                                    crate::mir::MirClosureCaptureKind::ByRef | crate::mir::MirClosureCaptureKind::ByMutRef =>
                                        analysis.origins_for_borrowed_place(&state, &capture.place()),
                                };
                                receipt.origin_row(&id, &origins);
                                let child = program.function(MirFunctionId::Closure(Box::new(closure.id.clone())));
                                let binding = child.and_then(|body| body.closure_captures.get(ordinal));
                                eprintln!("HKT_RETURN_ORIGIN captureStorage function={id:?} block={block_index} index={index} closure={:?} ordinal={ordinal} constructorCapture={capture:?} actualBodyBinding={binding:?} captureLifetimeOrigins={origins:?} sourceDeclaration={:?}; genuine storage dependencies preserved, no result-retention inference",
                                    closure.id, function.local_decls.get(capture.local.0));
                            }
                        }
                    }
                    let before = if observe {
                        if let StatementKind::Assign(destination, _) = &statement.kind {
                            Some(analysis.origins_for_place(&state, destination))
                        } else { None }
                    } else { None };
                    analysis.apply_statement(&mut state, statement);
                    if let (Some(before), StatementKind::Assign(destination, _)) = (before, &statement.kind) {
                        let after = analysis.origins_for_place(&state, destination);
                        receipt.origin_row(&id, &before);
                        receipt.origin_row(&id, &after);
                        eprintln!("HKT_RETURN_ORIGIN statementOrigins function={id:?} block={block_index} index={index} destination={destination:?} LifetimeBefore={before:?} LifetimeAfter={after:?} operationOrigin={:?}", statement.origin());
                    }
                }
                let Some(term) = &block.terminator else { continue; };
                if let (Some(bindings), Terminator::Call { .. }) = (mutation_origins.get(&block_index), term) {
                    analysis.apply_mutation_origins(&mut state, bindings);
                }
                if receipt.sites.contains(&(id.clone(), block_index, usize::MAX)) {
                    if let Terminator::Call { func, args, destination, .. } = term {
                        assert!(args.len() <= 8, "HKT replay call arguments overflow");
                        operand_origins_row(receipt, program, &analysis, &state, block_index, "callee", func);
                        for (ordinal, arg) in args.iter().enumerate() {
                            operand_origins_row(receipt, program, &analysis, &state, block_index, &format!("arg{ordinal}"), arg);
                        }
                        let target = match func {
                            Operand::Constant(Constant::Callable(callable)) => callable.function_id(),
                            _ => None,
                        };
                        let native = match func {
                            Operand::Constant(Constant::Callable(MirCallable::Resolved(
                                MirCallableKey::Function(_) | MirCallableKey::Instance(_) | MirCallableKey::Closure(_)))) => true,
                            _ => false,
                        };
                        let summary = if native { target.as_ref().and_then(|target| lifetime.get(target)) } else { None };
                        if let (Some(target), Some(summary)) = (target.as_ref(), summary) {
                            receipt.origin_row(target, summary);
                        }
                        let eligible = destination.projection.is_empty() && analysis.local_type_tracks_origin(destination.local);
                        let computed = analysis.call_return_origins(&state, func, args, destination);
                        receipt.origin_row(&id, &computed);
                        if eligible && matches!(func, Operand::Copy(_) | Operand::Move(_)) {
                            let fallback = analysis.opaque_call_return_origins(&state, args);
                            assert_eq!(computed, fallback, "actual indirect call must retain its production opaque-Fn policy, not observed body candidates");
                            eprintln!("HKT_RETURN_ORIGIN opaqueFnFallback function={id:?} block={block_index} actualFunc={func:?} argumentUnionAndUnknownExternal={fallback:?}; actual callable-result retention remains UNKNOWN");
                        }
                        if let Some(summary) = summary.filter(|_| eligible) {
                            let instantiated = analysis.instantiate_return_origins(&state, args, destination, summary);
                            assert_eq!(computed, instantiated, "selected native Lifetime instantiation must match production transfer");
                            for origin in summary {
                                let mapped = analysis.instantiate_return_origins(&state, args, destination, &OriginSet::from([*origin]));
                                receipt.origin_row(&id, &mapped);
                                eprintln!("HKT_RETURN_ORIGIN returnInstantiation function={id:?} block={block_index} actualTarget={target:?} summaryOrigin={origin:?} callerDestination={destination:?} mappedLifetimeOrigins={mapped:?}; Local-to-destination mapping is observed, not deleted");
                            }
                        }
                        if let Some((target, value_summary)) = target.as_ref().and_then(|target|
                            values.summaries.get(target).map(|summary| (target, summary))) {
                            receipt.origin_row(target, value_summary);
                        }
                        let route = match func {
                            _ if !eligible => "untracked-or-projected-destination-production-policy",
                            Operand::Constant(Constant::ErasedCall(_)) => "erased-authority-or-opaque-fallback",
                            _ if summary.is_some() => "selected-native-Lifetime-summary",
                            _ if native => "missing-native-summary-opaque-argument-union",
                            Operand::Constant(Constant::Callable(_)) => "non-native-callable-policy",
                            Operand::Copy(_) | Operand::Move(_) => "opaque-indirect-Fn-argument-union-UnknownExternal",
                            _ => "opaque-other-selector-argument-union-UnknownExternal",
                        };
                        eprintln!("HKT_RETURN_ORIGIN callResult function={id:?} block={block_index} route={route} actualFunc={func:?} actualTarget={target:?} selectedLifetimeSummary={summary:?} selectedValueLoansSummary={:?} destination={destination:?} computedLifetimeReturn={computed:?} operationOrigin={:?}; observed bindings never substitute a summary for an indirect Fn",
                            target.as_ref().and_then(|target| values.summaries.get(target)), term.origin());
                    }
                }
                analysis.apply_terminator(&mut state, term);
                assert_eq!(state, results.exit_sets[block_index], "HKT replay BEFORE/AFTER sequence must equal phase-owned fixpoint exit");
                if receipt.sites.contains(&(id.clone(), block_index, usize::MAX)) {
                    let destination = match term {
                        Terminator::Call { destination, .. } => Some(destination.clone()),
                        Terminator::Return | Terminator::ReturnWithOrigin { .. } => Some(Place { local: Local(0), projection: Vec::new() }),
                        _ => None,
                    };
                    if let Some(destination) = destination {
                        let after = analysis.origins_for_place(&state, &destination);
                        receipt.origin_row(&id, &after);
                        eprintln!("HKT_RETURN_ORIGIN terminatorAfter function={id:?} block={block_index} place={destination:?} LifetimeAfter={after:?}");
                    }
                }
            }
        }
        receipt.budget();
        for ty in &receipt.types { eprintln!("HKT_RETURN_ORIGIN finalType id={ty:?} semantic={:?}", program.type_context.type_for(*ty)); }
        eprintln!("HKT_RETURN_ORIGIN fullTotals FULLBodies={} symbols={} callableKeys={} locals={} MIRLocals={} sourceHIRLocals={} origins={} sites={} MIRSites={} sourceHIRSites={} types={} caps=FULL16/symbol32/keys16/locals128/origins16/sites192/types64; ProducerBeforeArtifactVsAfterLoad=UNMEASURED; no missing source authority reconstructed",
            receipt.selected.len(), receipt.symbols.len(), receipt.callable_keys.len(), receipt.locals.len() + receipt.source_locals.len(),
            receipt.locals.len(), receipt.source_locals.len(), receipt.origins.len(),
            receipt.sites.len() + receipt.source_sites.len(), receipt.sites.len(), receipt.source_sites.len(), receipt.types.len());
    });
}

type SourceFunction = HirFunctionFor<AcceptedHir>;
type SourceBlock = HirBlockFor<AcceptedHir>;
type SourceExpr = HirExprFor<AcceptedHir>;

fn root_callable(id: &MirFunctionId) -> &MirFunctionId {
    match id {
        MirFunctionId::Closure(closure) => root_callable(&closure.owner),
        _ => id,
    }
}

fn origin_definition(origin: &InstanceOrigin) -> DefId {
    match origin {
        InstanceOrigin::Function(id) => *id,
        InstanceOrigin::ImplMethod { method, .. } | InstanceOrigin::TraitDefault { method, .. } => {
            *method
        }
    }
}

fn retained_definition(mono: &MonomorphizedProgram, owner: DefId) -> Option<&SourceFunction> {
    mono.program
        .functions
        .get(&owner)
        .or_else(|| {
            mono.program
                .impls
                .values()
                .flat_map(|imp| imp.methods.values())
                .find(|method| method.id == owner)
        })
        .or_else(|| {
            mono.program
                .traits
                .values()
                .flat_map(|tr| tr.methods.values())
                .find(|method| method.id == owner)
        })
}

// Walk only a canonically selected defining body, without copying or dumping
// it. Matching source sites retain all alternatives, never pick by local_index.
fn walk_block<'a>(block: &'a SourceBlock, visit: &mut impl FnMut(&'a SourceExpr)) {
    for statement in &block.stmts {
        match statement {
            HirStmtFor::Let { value, .. } | HirStmtFor::Expr(value) => walk_expr(value, visit),
            HirStmtFor::Return(Some(value)) | HirStmtFor::Break(Some(value)) => {
                walk_expr(value, visit)
            }
            _ => {}
        }
    }
}

fn walk_expr<'a>(expression: &'a SourceExpr, visit: &mut impl FnMut(&'a SourceExpr)) {
    use HirExprKindFor as K;
    visit(expression);
    match &expression.kind {
        K::PackagePack { payload, .. } => walk_expr(payload, visit),
        K::PackageOpen { source, body, .. } | K::Open { source, body, .. } => {
            walk_expr(source, visit);
            walk_block(body, visit);
        }
        K::ArrayLiteral(values)
        | K::TupleLiteral(values)
        | K::EnumVariant(_, _, values, _)
        | K::Intrinsic { args: values, .. } => {
            for value in values {
                walk_expr(value, visit);
            }
        }
        K::ArrayRepeat(value, _)
        | K::FieldAccess(value, _, _)
        | K::TupleIndex(value, _)
        | K::UnaryOp(_, value)
        | K::Ref(_, value)
        | K::Deref(value)
        | K::Cast(value, _)
        | K::ObjectCoercion(value, _)
        | K::Try { expr: value, .. } => walk_expr(value, visit),
        K::BinOp(_, left, right) | K::Assign(left, right) => {
            walk_expr(left, visit);
            walk_expr(right, visit);
        }
        K::Call(callee, args, _)
        | K::MethodCall(callee, _, args, _, _)
        | K::OwnedObjectCall {
            owner: callee,
            args,
            ..
        } => {
            walk_expr(callee, visit);
            for arg in args {
                walk_expr(arg, visit);
            }
        }
        K::StructLiteral(_, _, fields) => {
            for field in fields {
                walk_expr(&field.value, visit);
            }
        }
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            walk_expr(condition, visit);
            walk_block(then_branch, visit);
            if let Some(block) = else_branch {
                walk_block(block, visit);
            }
        }
        K::Match { scrutinee, arms } => {
            walk_expr(scrutinee, visit);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    walk_expr(guard, visit);
                }
                walk_block(&arm.body, visit);
            }
        }
        K::While { condition, body } => {
            walk_expr(condition, visit);
            walk_block(body, visit);
        }
        K::For { iter, body, .. } => {
            walk_expr(iter, visit);
            walk_block(body, visit);
        }
        K::Loop(body) | K::Block(body) | K::UnsafeBlock(body) | K::Lambda { body, .. } => {
            walk_block(body, visit)
        }
        K::IntLiteral(_)
        | K::FloatLiteral(_)
        | K::BoolLiteral(_)
        | K::StringLiteral(_)
        | K::CharLiteral(_)
        | K::Unit
        | K::Var(_)
        | K::ResolvedVar(_) => {}
    }
}

fn source_local(
    receipt: &mut Receipt,
    mono: &MonomorphizedProgram,
    program: &MirProgram,
    owner: DefId,
    local: HirLocalId,
    ty: &Type,
    mir: Option<&crate::mir::LocalDecl>,
) {
    receipt.source_locals.insert((owner, local));
    let semantic_id = program.type_context.id_for_type(ty);
    receipt.types.extend(semantic_id);
    receipt.budget();
    let registration = mono.source_map.local_span(owner, local);
    eprintln!("HKT_RETURN_ORIGIN sourceLocal owner={owner:?} HirLocalId={local:?} registered={} sourceSpan={registration:?} sourceType={ty:?} semanticTypeId={semantic_id:?} pairedMIRDeclaration={mir:?}; absent registration is UNKNOWN, not a span fallback",
        registration.is_some());
}

fn source_header(
    receipt: &mut Receipt,
    mono: &MonomorphizedProgram,
    program: &MirProgram,
    owner: DefId,
    params: &[crate::hir::HirParam],
    ret: &Type,
    mir: &MirFunction,
) {
    assert!(
        params.len() <= 8,
        "HKT source parameter overflow before formatting"
    );
    receipt.types.extend(program.type_context.id_for_type(ret));
    receipt.budget();
    let same_arity = params.len() == mir.arg_count;
    eprintln!("HKT_RETURN_ORIGIN sourceExpectation function={:?} sourceOwner={owner:?} definingReturn={ret:?} MIRReturn={:?} parameterOrdinalLinkAvailable={same_arity}; defining types are not assumed to equal specialized types",
        mir.id, mir.ret_type);
    for (index, param) in params.iter().enumerate() {
        // build_function binds its input params in this exact ordinal order.
        let declaration = same_arity.then(|| mir.local_decls.get(index + 1)).flatten();
        source_local(
            receipt,
            mono,
            program,
            owner,
            param.local_id,
            &param.ty,
            declaration,
        );
    }
}

fn selected_method_types(
    receipt: &mut Receipt,
    program: &MirProgram,
    target: &crate::hir::HirMethodCallTarget,
) {
    for ty in target.trait_args() {
        receipt.types.extend(program.type_context.id_for_type(ty));
    }
    for binding in target
        .owner_substitution
        .iter()
        .chain(&target.method_substitution)
    {
        receipt
            .types
            .extend(program.type_context.id_for_type(&binding.ty));
    }
    receipt.budget();
}

fn correlate(mono: &MonomorphizedProgram, bodies: &MirInstanceBodies, program: &MirProgram) {
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        let receipt = active.as_mut().unwrap();
        let selected = receipt.symbols.iter().cloned().collect::<Vec<_>>();
        receipt.budget();
        for id in selected {
            let root = root_callable(&id);
            let record = match root {
                MirFunctionId::Instance(instance) => mono.instances.get(instance),
                _ => None,
            };
            let source_owner = record.map(|record| origin_definition(&record.origin)).or_else(|| match root {
                MirFunctionId::Function(owner) | MirFunctionId::Extern(owner) => Some(*owner),
                _ => None,
            });
            if let Some(header_source) = program.backend_contract.function_bodies.get(root)
                .and_then(|key| program.backend_contract.callable(key))
                .and_then(|header| header.source_def_id)
            {
                assert_eq!(source_owner, Some(header_source), "canonical Mono/header defining owner must agree");
            }
            if let Some(record) = record {
                assert!(!matches!(record.origin, InstanceOrigin::ImplMethod {
                    owner: crate::mono::InstanceImplOwner::BuiltinSlice, ..
                }), "HKT source correlation overlaps primitive slice owner; stop, roles UNMEASURED");
                receipt.types.extend(record.substitution.iter().copied());
                receipt.budget();
                eprintln!("HKT_RETURN_ORIGIN mono function={id:?} root={root:?} instance={:?} actualOrigin={:?} actualSubstitutionVector={:?} declaredPresent={} providedByObject={} specialization={} preMirHIRPresent={} MIRInstanceBodyPresent={}",
                    record.id, record.origin, record.substitution, record.declared.is_some(),
                    record.provided_by_object, record.is_specialization,
                    mono.pre_mir_instance_bodies.contains(record.id), bodies.contains_key(record.id));
            } else {
                eprintln!("HKT_RETURN_ORIGIN mono function={id:?} root={root:?} instanceRecord=UNKNOWN");
            }
            let instance_body = match &id {
                MirFunctionId::Instance(instance) => bodies.get(*instance).map(|body| &body.function),
                MirFunctionId::Closure(_) => bodies.nested_functions().iter().find(|function| function.id == id),
                _ => program.function(id.clone()),
            };
            eprintln!("HKT_RETURN_ORIGIN typedInstanceBody function={id:?} actualBodyId={:?} sourceOwner={source_owner:?} definitionSourceRegistration={:?}",
                instance_body.map(|body| &body.id), source_owner.and_then(|owner| mono.source_map.definition_span(owner)));
            if let Some(body) = instance_body { assert_eq!(body.id, id, "actual typed instance body key"); }
            let Some(owner) = source_owner else { continue; };
            let source = record.and_then(|record| record.declared.as_ref())
                .or_else(|| retained_definition(mono, owner));
            let Some(source) = source else {
                eprintln!("HKT_RETURN_ORIGIN definingHIR function={id:?} owner={owner:?} UNKNOWN(no-retained-canonical-definition; specialized-HIR-consumed-before-callback)");
                continue;
            };
            if source.id != owner {
                eprintln!("HKT_RETURN_ORIGIN definingHIR function={id:?} canonicalOwner={owner:?} retainedId={:?} correspondence=UNKNOWN(id-mismatch)", source.id);
                continue;
            }
            assert!(source.generic_params.len() <= 8, "HKT defining generic header overflow before formatting");
            eprintln!("HKT_RETURN_ORIGIN definingHIR function={id:?} owner={owner:?} retainedDefinitionId={:?} genericParameters={:?} declaredView={} specializedHIR=UNKNOWN(consumed-by-take_mir_instance_bodies)",
                source.id, source.generic_params, record.is_some_and(|record| record.declared.is_some()));
            let Some(mir) = instance_body else { continue; };
            if !matches!(id, MirFunctionId::Closure(_)) {
                source_header(receipt, mono, program, owner, &source.params, &source.ret_type, mir);
            }

            // Correlate a closure only through its actual construction in its
            // canonical parent and exact operation span. Retain every HIR
            // alternative; do not pretend an ambiguous source site is unique.
            if let MirFunctionId::Closure(closure_id) = &id {
                if let Some(parent) = program.function(closure_id.owner.clone()) {
                    for (block_index, block) in parent.basic_blocks.iter().enumerate() {
                        for (index, statement) in block.statements.iter().enumerate() {
                            let StatementKind::Assign(destination, Rvalue::Closure(closure)) = &statement.kind else { continue; };
                            if &closure.id != closure_id.as_ref() { continue; }
                            if !receipt.sites.contains(&(parent.id.clone(), block_index, index)) { continue; }
                            receipt.site(&parent.id, block_index, index);
                            let Some(span) = &statement.span else {
                                eprintln!("HKT_RETURN_ORIGIN HIRClosure function={id:?} constructionSpan=UNKNOWN");
                                continue;
                            };
                            let mut alternatives = Vec::new();
                            walk_block(&source.body, &mut |expression| {
                                if expression.span == *span && matches!(expression.kind, HirExprKindFor::Lambda { .. }) {
                                    assert!(alternatives.len() < 32, "HKT source-site alternative overflow before formatting cap=32");
                                    alternatives.push(expression);
                                }
                            });
                            eprintln!("HKT_RETURN_ORIGIN HIRClosure function={id:?} parent={:?} construction=({block_index},{index}) destination={destination:?} span={span:?} alternatives={} uniqueSourceSite={}; source site is not a reconstructed closure ID",
                                parent.id, alternatives.len(), alternatives.len() == 1);
                            for (alternative_index, expression) in alternatives.iter().enumerate() {
                                receipt.source_sites.insert((id.clone(), expression.span.file_path.clone(), expression.span.start, expression.span.end, 0, alternative_index));
                                receipt.budget();
                                let HirExprKindFor::Lambda { params, body, captures } = &expression.kind else { unreachable!() };
                                assert!(captures.len() <= 8 && params.len() <= 8, "HKT source closure header overflow before formatting");
                                receipt.types.extend(program.type_context.id_for_type(&expression.ty));
                                receipt.budget();
                                eprintln!("HKT_RETURN_ORIGIN HIRClosureAlternative function={id:?} owner={owner:?} closureType={:?} captures={captures:?}", expression.ty);
                                // Only a unique exact source construction has a
                                // parameter/capture ordinal link to this header.
                                if alternatives.len() == 1 {
                                    source_header(receipt, mono, program, owner, params, &body.ty, mir);
                                } else {
                                    for param in params { source_local(receipt, mono, program, owner, param.local_id, &param.ty, None); }
                                }
                                for (ordinal, capture) in captures.iter().enumerate() {
                                    let binding = (alternatives.len() == 1 && captures.len() == mir.closure_captures.len())
                                        .then(|| mir.closure_captures.get(ordinal)).flatten();
                                    let declaration = binding.and_then(|binding| mir.local_decls.get(binding.local.0));
                                    source_local(receipt, mono, program, owner, capture.local_id, &capture.ty, declaration);
                                    eprintln!("HKT_RETURN_ORIGIN captureDefinition function={id:?} sourceCapture={capture:?} actualHeaderBinding={binding:?} actualConstructorStorage={:?}", closure.captures.get(ordinal));
                                }
                                eprintln!("HKT_RETURN_ORIGIN sourceBodyLocal function={id:?} MIRLocalCorrespondence=UNKNOWN(no-retained-HIR-to-MIR-binding-map); unanchored HIR lets are not selected by body membership");
                            }
                        }
                    }
                }
            }
            if !receipt.selected.contains(&id) { continue; }
            // Recover checked target bindings at only the actual MIR call
            // sites already retained, within the canonical defining HIR body.
            for (block_index, block) in mir.basic_blocks.iter().enumerate() {
                if !receipt.sites.contains(&(id.clone(), block_index, usize::MAX)) { continue; }
                let Some(Terminator::Call { span: Some(span), .. }) = &block.terminator else { continue; };
                let mut alternatives = Vec::new();
                walk_block(&source.body, &mut |expression| {
                    if expression.span == *span && matches!(expression.kind,
                        HirExprKindFor::Call(..) | HirExprKindFor::MethodCall(..)) {
                        assert!(alternatives.len() < 32, "HKT call source alternatives overflow before formatting");
                        alternatives.push(expression);
                    }
                });
                receipt.site(&id, block_index, usize::MAX);
                eprintln!("HKT_RETURN_ORIGIN sourceCallSite function={id:?} owner={owner:?} block={block_index} span={span:?} alternatives={}", alternatives.len());
                for (alternative_index, expression) in alternatives.into_iter().enumerate() {
                    receipt.source_sites.insert((id.clone(), expression.span.file_path.clone(), expression.span.start, expression.span.end, 1, alternative_index));
                    receipt.budget();
                    match &expression.kind {
                        HirExprKindFor::Call(_, args, target) => {
                            assert!(args.len() <= 8, "HKT HIR call args overflow");
                            if let Some(crate::hir::HirCallTarget::StaticMethod(target)) = target {
                                assert!(target.method.owner_substitution.len() <= 8 && target.method.method_substitution.len() <= 8,
                                    "HKT selected binding row overflow before formatting");
                                receipt.types.extend(program.type_context.id_for_type(&target.owner_ty));
                                selected_method_types(receipt, program, &target.method);
                            }
                            receipt.types.extend(args.iter().filter_map(|arg| program.type_context.id_for_type(&arg.ty)));
                            receipt.types.extend(program.type_context.id_for_type(&expression.ty));
                            receipt.budget();
                            eprintln!("HKT_RETURN_ORIGIN selectedHIRCall function={id:?} owner={owner:?} span={span:?} target={target:?} argumentTypes={:?} result={:?}; defining-view substitutions are not reinterpreted as concrete instance substitutions",
                                args.iter().map(|arg| &arg.ty).collect::<Vec<_>>(), expression.ty);
                        }
                        HirExprKindFor::MethodCall(_, _, args, _, target) => {
                            assert!(args.len() <= 8 && target.owner_substitution.len() <= 8 && target.method_substitution.len() <= 8,
                                "HKT selected method row overflow before formatting");
                            selected_method_types(receipt, program, target);
                            receipt.types.extend(args.iter().filter_map(|arg| program.type_context.id_for_type(&arg.ty)));
                            receipt.types.extend(program.type_context.id_for_type(&expression.ty));
                            receipt.budget();
                            eprintln!("HKT_RETURN_ORIGIN selectedHIRMethod function={id:?} owner={owner:?} span={span:?} target={target:?} result={:?}", expression.ty);
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        for key in &receipt.callable_keys {
            if let MirCallableKey::Instance(instance) = key {
                let record = mono.instances.get(instance);
                if let Some(record) = record { receipt.types.extend(record.substitution.iter().copied()); }
                // This loop does not select additional bodies or infer missing
                // bindings: it reads only already-recorded canonical call keys.
                assert!(receipt.types.len() <= 64, "HKT linked substitution TypeId overflow cap=64 actual={}", receipt.types.len());
                eprintln!("HKT_RETURN_ORIGIN linkedInstance key={key:?} origin={:?} substitutions={:?}",
                    record.map(|record| &record.origin), record.map(|record| &record.substitution));
            }
        }
        receipt.budget();
        eprintln!("HKT_RETURN_ORIGIN correlationTotals FULLBodies={} symbols={} callableKeys={} locals={} MIRLocals={} sourceHIRLocals={} origins={} sites={} MIRSites={} sourceHIRSites={} types={} caps=FULL16/symbol32/keys16/locals128/origins16/sites192/types64; absent registrations/bodies and ambiguous sites remain UNKNOWN; symbol-only source rows are NOT runtime result proof",
            receipt.selected.len(), receipt.symbols.len(), receipt.callable_keys.len(), receipt.locals.len() + receipt.source_locals.len(),
            receipt.locals.len(), receipt.source_locals.len(), receipt.origins.len(),
            receipt.sites.len() + receipt.source_sites.len(), receipt.sites.len(), receipt.source_sites.len(), receipt.types.len());
    });
}

fn historical_diagnostic_receipt(case: &str, internal_count: usize, borrow_count: usize) {
    // Frozen integration8 RED evidence is a historical reference, not an
    // expected error count for a positive checked by the current compiler.
    eprintln!("HKT_RETURN_ORIGIN historicalDiagnosticMultiset case={case} integration8=({internal_count},{borrow_count}) source=immutable-integration8-receipt; NOT current acceptance or borrow authority");
}

#[test]
fn hkt_historical_integration8_diagnostics_remain_frozen() {
    let path = agent_work_path("recovery/rock-completion-integration-8.log");
    assert_eq!(
        integration_sha(&path),
        "737b4283d95bbc6b87ab48adc4658e9b9ab80c680054cd8306d22573ea6925dc",
        "historical rejection evidence must not be rewritten as acceptance",
    );
    let log = std::fs::read_to_string(path).unwrap();
    for (case, internal, borrowed) in [
        ("test_hkt_standalone_and_receiver_operations", 1, 1),
        ("test_hkt_receiver_failure_paths_and_constructor_sections", 2, 1),
        ("test_hkt_foldable_and_traversable", 2, 2),
    ] {
        let section = log.split(&format!("---- {case} stdout ----")).nth(1).unwrap();
        let diagnostic = section.lines().find(|line| line.starts_with("Compilation failed: Diagnostics(")).unwrap();
        assert_eq!(diagnostic.matches("code: Some(Internal)").count(), internal);
        assert_eq!(diagnostic.matches("location: Toolchain, primary: None").count(), internal);
        assert_eq!(diagnostic.matches("code: Some(Borrow)").count(), borrowed);
        assert_eq!(diagnostic.matches("start: 11676, end: 11683").count(), borrowed * 2);
        assert_eq!(diagnostic.matches(MESSAGE).count(), internal + borrowed * 2);
        historical_diagnostic_receipt(case, internal, borrowed);
    }
}

fn control(case: &str, bytes: usize, sha: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let fixture = root.join("lib/tests/integration.rs");
    let source = integration_literal(&fixture, &format!("fn {case}()"));
    assert_eq!(source.len(), bytes, "unchanged original HKT fixture length");
    let vec_path = root.join("stdlib/vec.rk");
    let vec_bytes = std::fs::read(&vec_path).unwrap();
    assert_eq!(vec_bytes.len(), 11927);
    assert_eq!(
        integration_sha(&vec_path),
        "1cd5d65d0bb35535af3cd1426066a3ca41ea46375bda08fe800e416827c73e1b"
    );
    assert_eq!(&vec_bytes[11676..11683], b"G::bind");
    let _scope = Scope::enter();
    let (program, entry, verdict) =
        mutation_source_program_with_callback(case, &source, true, |mono, bodies, program| {
            let verdict = BorrowChecker::run(program);
            post_run(program);
            correlate(mono, bodies, program);
            replay_origins(program);
            verdict
        });
    assert_eq!(std::fs::read(&entry).unwrap(), source.as_bytes());
    assert_eq!(
        integration_sha(&entry),
        sha,
        "actual compiled source SHA guard"
    );
    eprintln!("HKT_RETURN_ORIGIN fixture={case} actualEntry={entry:?} bytes={bytes} sha={sha} integration8SourceArchive=UNAVAILABLE(log-has-diagnostics-only) RuntimeAgreement=clean-before-Borrow currentStdlibSchema=59(independently-guarded)");
    assert!(crate::mir::agreement::check_mir_runtime_agreement(&program).is_clean());
    let diagnostics = verdict
        .as_ref()
        .err()
        .map(|diagnostics| diagnostics.0.as_slice())
        .unwrap_or(&[]);
    for diagnostic in diagnostics {
        eprintln!("HKT_RETURN_ORIGIN diagnostic case={case} code={:?} location={:?} primary={:?} message={:?}",
            diagnostic.code, diagnostic.location, diagnostic.primary, diagnostic.message);
    }
    ACTIVE.with(|active| {
        assert_eq!(
            active.borrow().as_ref().unwrap().rejections,
            diagnostics.len()
        )
    });
    assert_eq!(std::fs::read(&entry).unwrap(), source.as_bytes());
    assert_eq!(integration_sha(&entry), sha);
    assert_eq!(std::fs::read(&vec_path).unwrap(), vec_bytes);
    // The capped observer and original source guards remain live. A
    // historical rejection cannot satisfy this independent positive gate.
    assert!(verdict.is_ok(), "owned HKT traversal must pass Borrow: {verdict:?}");
}

#[test]
fn hkt_return_origin_authority_foldable_and_traversable() {
    historical_diagnostic_receipt("test_hkt_foldable_and_traversable", 2, 2);
    control(
        "test_hkt_foldable_and_traversable",
        9941,
        "35273432f8febd03f6cef234bf85e09d80f3f2b5ef4c516cd8d0a6022e370446",
    );
}

#[test]
fn hkt_return_origin_authority_receiver_failure_paths_and_constructor_sections() {
    historical_diagnostic_receipt("test_hkt_receiver_failure_paths_and_constructor_sections", 2, 1);
    control(
        "test_hkt_receiver_failure_paths_and_constructor_sections",
        1306,
        "a35b7dd3b9c523b65b5a93cffdef8c89d945384086bf3581b45fda5beb350572",
    );
}

#[test]
fn hkt_return_origin_authority_standalone_and_receiver_operations() {
    historical_diagnostic_receipt("test_hkt_standalone_and_receiver_operations", 1, 1);
    control(
        "test_hkt_standalone_and_receiver_operations",
        1317,
        "8b0c3e611ea091aacccb091132b65835cd9af9c34bf632d08419c8bf42b450a6",
    );
}

// Pre-consumption observations are separate from FULL return-state replay.
// The retained specialized map uses AcceptedHir; canonical targets are
// observed where present, never reconstructed from its display metadata.
type PreExpr = crate::hir::HirExprFor<crate::hir::AcceptedHir>;
type PreBlock = crate::hir::HirBlockFor<crate::hir::AcceptedHir>;
type PreStmt = crate::hir::HirStmtFor<crate::hir::AcceptedHir>;

#[derive(Default)]
struct PreBindingSnapshot {
    roots: BTreeSet<InstanceId>,
    constructors: usize,
    bindings: usize,
    invocations: usize,
    relay_invocations: usize,
    incoming_bindings: usize,
    anchors: usize,
    unknown: usize,
    rows: Vec<String>,
    selected_constructors: Vec<PreSelectedConstructor>,
}

#[derive(Clone)]
struct PreSelectedConstructor {
    owner: InstanceId,
    occurrence: usize,
    params: Vec<HirLocalId>,
    captures: Vec<(HirLocalId, crate::hir::HirClosureCaptureKind)>,
}

struct PreEmission {
    selected: PreSelectedConstructor,
    closure: crate::mir::MirClosureId,
    destination: Place,
    captures: Vec<crate::mir::MirClosureCapture>,
}

#[derive(Default)]
struct PreAllocations {
    selected: Vec<PreSelectedConstructor>,
    cloned: Vec<CloneSelectedConstructor>,
    emitted: Vec<PreEmission>,
}

struct CloneSelectedConstructor {
    selected: PreSelectedConstructor,
    actual_owner: MirFunctionId,
    cloned_occurrence: usize,
}

thread_local! {
    static PRE_ALLOCATIONS: RefCell<Option<PreAllocations>> = const { RefCell::new(None) };
}

struct PreAllocationScope;

impl PreAllocationScope {
    fn enter() -> Self {
        PRE_ALLOCATIONS.with(|active| {
            assert!(active.borrow().is_none());
            *active.borrow_mut() = Some(PreAllocations::default());
        });
        Self
    }
}

impl Drop for PreAllocationScope {
    fn drop(&mut self) {
        PRE_ALLOCATIONS.with(|active| *active.borrow_mut() = None);
    }
}

pub(crate) fn closure_body_clone(
    original: &PreExpr,
    cloned_body: &PreBlock,
    closure: &crate::mir::MirClosureId,
) {
    PRE_ALLOCATIONS.with(|active| {
        let mut active = active.borrow_mut();
        let Some(active) = active.as_mut() else { return; };
        let Some(selected) = active.selected.iter().find(|selected|
            closure.owner == MirFunctionId::Instance(selected.owner)
                && selected.occurrence == original as *const PreExpr as usize).cloned() else { return; };
        let crate::hir::HirExprKindFor::Lambda { body, .. } = &original.kind else { panic!("selected constructor is no longer a Lambda"); };
        assert_eq!(body.stmts.len(), cloned_body.stmts.len(), "exact compiler Clone boundary");
        // Only immediate lambda-valued lexical bindings inside an already
        // invocation-rooted constructor; no unrelated bodies or global scan.
        for (original, cloned) in body.stmts.iter().zip(&cloned_body.stmts) {
            let (PreStmt::Let { local_id: original_local, value: original, .. },
                PreStmt::Let { local_id: cloned_local, value: cloned, .. }) = (original, cloned) else { continue; };
            let crate::hir::HirExprKindFor::Lambda { params, captures, .. } = &original.kind else { continue; };
            let crate::hir::HirExprKindFor::Lambda { params: cloned_params, captures: cloned_captures, .. } = &cloned.kind else { panic!("Clone changed lambda syntax"); };
            assert_eq!(original_local, cloned_local);
            assert_eq!(params.iter().map(|param| param.local_id).collect::<Vec<_>>(), cloned_params.iter().map(|param| param.local_id).collect::<Vec<_>>());
            assert_eq!(captures.iter().map(|capture| (capture.local_id, capture.kind)).collect::<Vec<_>>(), cloned_captures.iter().map(|capture| (capture.local_id, capture.kind)).collect::<Vec<_>>());
            assert!(active.cloned.len() < 8 && params.len() <= 8 && captures.len() <= 8, "bounded Clone-coupled constructor/capture receipt cap8; no FULL proof/scope growth");
            active.cloned.push(CloneSelectedConstructor {
                selected: PreSelectedConstructor {
                    owner: selected.owner,
                    occurrence: original as *const PreExpr as usize,
                    params: params.iter().map(|param| param.local_id).collect(),
                    captures: captures.iter().map(|capture| (capture.local_id, capture.kind)).collect(),
                },
                actual_owner: MirFunctionId::Closure(Box::new(closure.clone())),
                cloned_occurrence: cloned as *const PreExpr as usize,
            });
            eprintln!("HKT_CLONE actualParentAllocation={closure:?} originalBinding={original_local:?} immediateConstructor=true exactBodyCloneCoupled=true captures={}; no pre-clone pointer reused to identify the cloned allocation", captures.len());
        }
    });
}

pub(crate) fn closure_allocation(
    expr: &PreExpr,
    closure: &crate::mir::MirClosureId,
    destination: &Place,
    captures: &[crate::mir::MirClosureCapture],
) {
    PRE_ALLOCATIONS.with(|active| {
        let mut active = active.borrow_mut();
        let Some(active) = active.as_mut() else {
            return;
        };
        let selected = active.selected.iter().find(|selected| {
            closure.owner == MirFunctionId::Instance(selected.owner)
                && selected.occurrence == expr as *const PreExpr as usize
        }).cloned().or_else(|| active.cloned.iter().find(|selected|
            closure.owner == selected.actual_owner && selected.cloned_occurrence == expr as *const PreExpr as usize)
            .map(|selected| selected.selected.clone()));
        let Some(selected) = selected else {
            return;
        };
        assert!(
            active.emitted.len() < 8 && captures.len() <= 8,
            "selected allocation/capture cap8; no global collection"
        );
        // No dereference of saved addresses. Nested occurrences are identified
        // only by the new occurrence recorded at the exact compiler Clone event.
        active.emitted.push(PreEmission {
            selected,
            closure: closure.clone(),
            destination: destination.clone(),
            captures: captures.to_vec(),
        });
    });
}

fn verify_pre_allocations(program: &MirProgram) {
    PRE_ALLOCATIONS.with(|active| {
        let active = active.borrow();
        let active = active.as_ref().unwrap();
        assert_eq!(active.selected.len(), 2, "only invocation-rooted relay and actual step constructors");
        assert_eq!(active.cloned.len(), 1, "one immediate finish constructor inside the selected actual step allocation");
        assert_eq!(active.emitted.len(), active.selected.len() + active.cloned.len(), "every selected original or Clone-coupled occurrence must reach an actual allocation; no ID reconstruction");
        let mut ids = HashSet::new();
        for emission in &active.emitted {
            assert!(ids.insert(emission.closure.clone()), "one emission per selected constructor");
            let owner = emission.closure.owner.clone();
            let (_, function) = program.functions().find(|(id, _)| **id == owner).unwrap();
            let statements = function.basic_blocks.iter().flat_map(|block| &block.statements).filter_map(|statement| {
                let StatementKind::Assign(place, Rvalue::Closure(closure)) = &statement.kind else { return None; };
                (closure.id == emission.closure).then_some((place, closure))
            }).collect::<Vec<_>>();
            assert_eq!(statements.len(), 1, "allocation event must match the real emitted Closure assignment");
            let (destination, closure) = statements[0];
            assert_eq!(destination, &emission.destination);
            assert_eq!(closure.captures.iter().map(|capture| (capture.local, capture.kind)).collect::<Vec<_>>(),
                emission.captures.iter().map(|capture| (capture.local, capture.kind)).collect::<Vec<_>>());
            assert_eq!(emission.captures.len(), emission.selected.captures.len(),
                "missing filter_map captures are UNKNOWN; cannot pair partial arrays");
            let body_id = MirFunctionId::Closure(Box::new(emission.closure.clone()));
            let (_, body) = program.functions().find(|(id, _)| **id == body_id).unwrap();
            assert_eq!(body.local_decls.iter().filter(|local| local.source == LocalSource::Argument).count(), emission.selected.params.len());
            eprintln!("HKT_ALLOCATION actualClosure={:?} actualDestination={destination:?} parameterLocalIds={:?} captureCount={} occurrenceMatched=true emittedAssignmentMatched=true; no return/source-independent authority",
                emission.closure, emission.selected.params, emission.captures.len());
            for (ordinal, ((hir_local, hir_kind), capture)) in emission.selected.captures.iter().zip(&emission.captures).enumerate() {
                let decl = &function.local_decls[capture.local.0];
                eprintln!("HKT_ALLOCATION actualClosure={:?} ordinal={ordinal} hirLocal={hir_local:?} hirKind={hir_kind:?} emittedLocal={:?} emittedKind={:?} actualType={:?} source={:?} span={:?}; emitter-order coupling only, NOT callback-result proof",
                    emission.closure, capture.local, capture.kind, decl.ty, decl.source, decl.span);
            }
        }
    });
}

/// Replay only the immediate Clone-coupled finish, within the original FULL
/// budget; this receipt neither resolves a parameter nor installs permission.
pub(super) fn pre_callable_result_origins(
    program: &MirProgram,
    lifetime: &HashMap<MirFunctionId, OriginSet>,
    values: &HashMap<MirFunctionId, OriginSet>,
) {
    PRE_ALLOCATIONS.with(|active| {
        let active = active.borrow();
        let Some(active) = active.as_ref() else {
            return;
        };
        let mut receipt = Receipt::default();
        let resolved = call_targets::resolved_bodies(program);
        for emission in &active.emitted {
            let id = MirFunctionId::Closure(Box::new(emission.closure.clone()));
            let emitted_body = program
                .functions
                .get(&id)
                .expect("actual emitted closure body");
            let body = resolved.get(&id).unwrap_or(emitted_body);
            let replay = active
                .cloned
                .iter()
                .any(|selected| selected.actual_owner == emission.closure.owner);
            if replay {
                receipt.select(&id, SelectionReason::Construction { site: None });
                for index in 0..body.local_decls.len() {
                    receipt.local(body, Local(index));
                }
            } else {
                receipt.observe(&id, SelectionReason::Construction { site: None });
            }
            receipt.header(body, &program.backend_contract);
            assert_eq!(body.closure_captures.len(), emission.captures.len());
            let key = program.backend_contract.function_bodies.get(&id).unwrap();
            let declaration = program.backend_contract.callable(key).unwrap();
            assert_eq!(key, &MirCallableKey::Closure(id.clone()));
            assert!(matches!(&declaration.kind, crate::mir::MirCallableKind::LocalBody { function_id } if function_id == &id));
            assert_eq!(declaration.signature.ret.semantic_ty, body.ret_type);
            for (argument, parameter) in declaration.signature.params.iter().enumerate() {
                assert_eq!(body.local_decls[argument + 1].source, LocalSource::Argument);
                assert_eq!(body.local_decls[argument + 1].ty, parameter.semantic_ty);
            }
            let owner = &program.functions[&emission.closure.owner];
            assert!(emission.destination.projection.is_empty());
            let constructor_ty = owner.local_decls[emission.destination.local.0].ty;
            assert!(program.type_context.type_id_tree_is_valid(constructor_ty));
            let Type::Function { params, ret, callable_kind, .. } = program.type_context.type_for(constructor_ty) else {
                panic!("actual allocated callable lacks a canonical function type");
            };
            assert_eq!(params.len(), body.arg_count);
            assert_eq!(*ret, program.type_context.type_for(body.ret_type));
            for (parameter, expected) in declaration.signature.params.iter().zip(&params) {
                assert_eq!(program.type_context.type_for(parameter.semantic_ty), *expected);
            }
            eprintln!("HKT_CALLABLE_RESULT actualAllocation={:?} actualConstructorType={constructor_ty:?} actualCallableKind={callable_kind:?} constructorBodySignatureAgreement=true abstractCaptureMetadataUnused=true", emission.closure);
            for (ordinal, (supplied, stored)) in emission.captures.iter().zip(&body.closure_captures).enumerate() {
                assert_eq!(supplied.kind, stored.kind);
                assert_eq!(body.local_decls[stored.local.0].source, LocalSource::ClosureCapture);
                assert_eq!(owner.local_decls[supplied.local.0].ty, body.local_decls[stored.local.0].ty);
                eprintln!("HKT_CALLABLE_RESULT actualAllocation={:?} ordinal={ordinal} allocationLocal={:?} bodyCaptureLocal={:?} captureKind={:?}; exact stored binding, not return proof", emission.closure, supplied.local, stored.local, stored.kind);
            }
            for (purpose, summaries) in [(OriginPurpose::Lifetime, lifetime), (OriginPurpose::ValueLoans, values)] {
                if !replay {
                    if let Some(origins) = summaries.get(&id) { receipt.origin_row(&id, origins); }
                    let mut sorted = summaries.get(&id).map(|origins| origins.iter().map(|origin| format!("{origin:?}")).collect::<Vec<_>>());
                    if let Some(sorted) = &mut sorted { sorted.sort(); }
                    eprintln!("HKT_CALLABLE_RESULT actualAllocation={:?} purpose={purpose:?} producerSummaryHeader={sorted:?} FULL=false replay=NOT_REQUESTED unresolvedProducer=true; parent summary is not result independence or parameter authority", emission.closure);
                    continue;
                }
                let analysis = ReferenceEscapeAnalysis::new(body, &program.type_context,
                    &program.backend_contract, summaries, MissingSummaryMode::Defer);
                let results = run_fixpoint(&analysis, body);
                let mut actual = OriginSet::new();
                let mut returns = 0;
                for (index, block) in body.basic_blocks.iter().enumerate() {
                    if !results.reachable[index] { continue; }
                    for state in [&results.entry_sets[index], &results.exit_sets[index]] {
                        for origins in state.origins.values() {
                            receipt.origin_row(&id, origins);
                        }
                    }
                    for statement_index in 0..block.statements.len() {
                        receipt.site(&id, index, statement_index);
                    }
                    receipt.site(&id, index, block.statements.len());
                    if !matches!(block.terminator, Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })) { continue; }
                    returns += 1;
                    let return_place = Place { local: Local(0), projection: Vec::new() };
                    for (place, origins) in &results.exit_sets[index].origins {
                        if ReferenceEscapeAnalysis::place_is_prefix(&return_place, place) {
                            receipt.local(body, place.local);
                            receipt.origin_row(&id, origins);
                            actual.extend(origins.iter().copied());
                        }
                    }
                }
                assert!(returns > 0, "nonreturning constructor cannot prove result independence");
                let mut producer_row = actual.clone();
                if purpose == OriginPurpose::ValueLoans {
                    // The production value-loan table omits an opaque leaf's
                    // external storage, not its actual argument dependencies.
                    producer_row.remove(&ReferenceOrigin::UnknownExternal);
                }
                assert_eq!(summaries.get(&id), Some(&producer_row), "receipt must match actual producer summary normalization");
                assert_eq!(actual, HashSet::from([ReferenceOrigin::Param(body.closure_captures[0].local)]),
                    "actual finish returns its checked captured value dependency, not a local environment address; no header/ByValue shortcut");
                let mut sorted = actual.iter().map(|origin| format!("{origin:?}")).collect::<Vec<_>>();
                sorted.sort();
                eprintln!("HKT_CALLABLE_RESULT actualAllocation={:?} purpose={purpose:?} reachableReturns={returns} actualRawSummary={sorted:?} bodyReplayEqualsProducerAfterExistingNormalization=true captureFreeAnalysisResolution={} executableMIRUnchanged=true; body/capture evidence only, no callable-type contract or parameter authority", emission.closure, resolved.contains_key(&id));
            }
        }
        receipt.budget();
        assert_eq!(receipt.selected.len(), 1, "only exact immediate Clone-coupled finish is replayed; original FULL control remains separate");
        eprintln!("HKT_CALLABLE_RESULT boundedReplay FULL={} symbols={} locals={} origins={} sites={} types={} originalCapsUnchanged=true acceptance=UNPROVED",
            receipt.selected.len(), receipt.symbols.len(), receipt.locals.len(), receipt.origins.len(), receipt.sites.len(), receipt.types.len());
    });
}

#[test]
fn hkt_callable_result_body_distinguishes_by_value_capture_from_storage_borrow() {
    use crate::mir::MirClosureCaptureKind;
    for (case, source, borrowed_storage) in [
        (
            "captured_result_value",
            r#"
struct Holder
    < value: &I64
impl Holder
    ~@extract: &I64
    ~@extract = -> self.value
make: &I64 -> (() -> &I64)
make = value ->
    holder = Holder
        value: value
    -> holder.extract!
main = !->
    value = 42
    callback = make &value
    result = callback!
"#,
            false,
        ),
        (
            "captured_result_storage",
            r#"
struct Holder
    < value: I64
impl Holder
    ~@consume: I64
    ~@consume = -> self.value
make: I64 -> (() -> &I64)
make = value ->
    holder = Holder
        value: value
    ->
        result = &holder.value
        holder.consume!
        result
main = !->
    callback = make 42
    result = callback!
"#,
            true,
        ),
    ] {
        let (program, entry) =
            super::allocator_trace_tests::mutation_source_program(case, source, false);
        assert!(
            program.functions.len() <= 8,
            "focused source-backed body cap8"
        );
        let allocations = program
            .functions
            .values()
            .flat_map(|owner| {
                owner
                    .basic_blocks
                    .iter()
                    .flat_map(|block| &block.statements)
                    .filter_map(move |statement| {
                        let StatementKind::Assign(_, Rvalue::Closure(closure)) = &statement.kind
                        else {
                            return None;
                        };
                        (!closure.captures.is_empty()).then_some((owner, closure))
                    })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            allocations.len(),
            1,
            "one actual captured constructor, no guessed closure ID"
        );
        let (owner, allocation) = allocations[0];
        let id = MirFunctionId::Closure(Box::new(allocation.id.clone()));
        let body = &program.functions[&id];
        assert_eq!(allocation.id.owner, owner.id);
        assert_eq!(allocation.captures.len(), 1);
        assert_eq!(body.closure_captures.len(), 1);
        let capture = &body.closure_captures[0];
        assert_eq!(allocation.captures[0].kind, MirClosureCaptureKind::ByValue);
        assert_eq!(capture.kind, MirClosureCaptureKind::ByValue);
        assert_eq!(
            owner.local_decls[allocation.captures[0].local.0].ty,
            body.local_decls[capture.local.0].ty
        );
        let expected = HashSet::from([if borrowed_storage {
            ReferenceOrigin::Local(capture.local)
        } else {
            ReferenceOrigin::Param(capture.local)
        }]);
        let lifetime = BorrowChecker::reference_return_summaries(&program);
        let values =
            BorrowChecker::return_summaries_for_purpose(&program, OriginPurpose::ValueLoans);
        assert_eq!(
            lifetime.get(&id),
            Some(&expected),
            "actual captured producer, not type-derived inference"
        );
        assert_eq!(values.get(&id), Some(&expected));
        let verdict = BorrowChecker::run(&program);
        if borrowed_storage {
            let diagnostics = verdict
                .expect_err("ByValue capture alone cannot authorize borrowing its own storage");
            let capture_start = source.find("    holder = Holder").unwrap() + 4;
            let capture_span =
                Span::new(entry.clone(), capture_start, capture_start + "holder".len());
            assert_eq!(
                body.local_decls[capture.local.0].span.as_ref(),
                Some(&capture_span)
            );
            assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                && diagnostic.message == MESSAGE && diagnostic.primary.as_ref().is_some_and(|primary| primary.span == capture_span)),
                "reject actual captured-storage return at its transported source origin: {diagnostics:?}");
            assert_eq!(
                diagnostics.0.len(),
                2,
                "independent consume-vs-borrow conflict must also survive"
            );
            assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.code
                == Some(DiagnosticCode::Borrow)
                && diagnostic.message == "borrow conflict on this value"
                && diagnostic
                    .primary
                    .as_ref()
                    .is_some_and(|primary| primary.span.file_path == entry
                        && &source[primary.span.start..primary.span.end] == "holder.consume!")));
        } else {
            verdict.expect("returning a captured reference value retains the original dependency without borrowing its environment");
        }
        assert_eq!(std::fs::read(&entry).unwrap(), source.as_bytes());
        eprintln!("HKT_CALLABLE_RESULT_GUARD case={case} actualAllocation={:?} bodyCapture={:?} kind=ByValue checkedSummary={expected:?} rejectsStorage={borrowed_storage} execution=NONE callableTypeAuthority=ABSENT", allocation.id, capture.local);
    }
}

impl PreBindingSnapshot {
    fn budget(&self) {
        assert!(self.roots.len() <= 6 && self.constructors <= 8
            && self.bindings <= 16 && self.invocations <= 4,
            "HKT_PRE_MIR scope overflow before allocation dump: roots={} constructors={} bindings={} invocations={} caps=6/8/16/4; no truncation or scope growth",
            self.roots.len(), self.constructors, self.bindings, self.invocations);
    }

    fn unknown(&mut self, reason: &str) {
        self.unknown += 1;
        self.rows.push(format!("UNKNOWN {reason}"));
    }

    fn root(&mut self, mono: &MonomorphizedProgram, id: InstanceId) {
        if !self.roots.insert(id) {
            return;
        }
        self.budget();
        let Some(record) = mono.instances.get(&id) else {
            self.unknown("canonical Instance target has no Mono record");
            return;
        };
        let body = mono.pre_mir_instance_bodies.get(id);
        self.rows.push(format!("nativeRoot={id:?} origin={:?} sourceOwner={:?} substitution={:?} retainedBody={} retainedBodyDef={:?} typeParams={:?}; metadata/source observation, NOT invoked-target or return authority",
            record.origin, origin_definition(&record.origin), record.substitution,
            body.is_some(), body.map(|body| body.id), body.map(|body| &body.generic_params)));
    }

    fn constructor(&mut self, owner: InstanceId, expr: &PreExpr) {
        let HirExprKindFor::Lambda {
            params, captures, ..
        } = &expr.kind
        else {
            return;
        };
        self.constructors += 1;
        self.selected_constructors.push(PreSelectedConstructor {
            owner,
            occurrence: expr as *const PreExpr as usize,
            params: params.iter().map(|param| param.local_id).collect(),
            captures: captures
                .iter()
                .map(|capture| (capture.local_id, capture.kind))
                .collect(),
        });
        self.bindings += captures.len();
        self.budget();
        assert!(
            params.len() <= 8 && captures.len() <= 8,
            "pre HIR lambda parameter/capture cap8 exceeded"
        );
        self.rows.push(format!("constructor nativeOwner={owner:?} span={:?} type={:?} params={:?} captures={:?}; occurrence only, NO HirExprId or MirClosureId correlation; captures are NOT return-independence proof",
            expr.span, expr.ty,
            params.iter().map(|p| (p.local_id, &p.ty, p.mutable, p.is_ref)).collect::<Vec<_>>(),
            captures.iter().map(|c| (c.local_id, &c.ty, c.kind, c.mutable)).collect::<Vec<_>>()));
    }

    fn definition_records(&mut self, mono: &MonomorphizedProgram, definition: DefId) {
        let mut present = false;
        // Metadata relationship only: retain every exact defining-ID match.
        // Neither a definition union nor substitutions resolve an invocation.
        for record in mono.instances.values() {
            if origin_definition(&record.origin) != definition {
                continue;
            }
            present = true;
            self.root(mono, record.id);
            self.rows.push(format!("definitionObservation definition={definition:?} instance={:?}; metadata alternative ONLY, NOT invoked target; its body is not entered", record.id));
        }
        if !present {
            self.unknown("canonical target definition has no retained Mono InstanceOrigin record");
        }
    }
}

// Borrowed index of ONE lexical scope, including every write alternative.
// Lambda bodies are separate scopes; parameters stop lookup, captures cross
// only their actual canonical LocalId edge to the immediately enclosing scope.
struct PreScope<'a> {
    defs: HashMap<HirLocalId, Vec<&'a PreExpr>>,
    uncertain_writes: bool,
    params: &'a [crate::hir::HirParam],
    captures: &'a [crate::hir::HirClosureCapture],
    incoming: Option<PreIncoming<'a>>,
}

#[derive(Clone, Copy)]
struct PreIncoming<'a> {
    call: &'a PreExpr,
    args: &'a [PreExpr],
    caller_depth: usize,
}

// Borrowed occurrence identity is valid only inside this pre-consumption walk;
// it is never exported as a HIR/MIR closure ID or used by production authority.
// (contains, exact) retains every LocalId alternative before admitting an edge.
fn pre_constructor_binding(
    value: &PreExpr,
    constructor: &PreExpr,
    scopes: &[PreScope<'_>],
    depth: usize,
    active: &mut HashSet<(usize, HirLocalId)>,
) -> (bool, bool) {
    use crate::hir::{HirExprKindFor as K, HirVarTarget as V};
    match &value.kind {
        K::Lambda { .. } => {
            let selected = std::ptr::eq(value, constructor);
            (selected, selected)
        }
        K::Ref(_, value) => pre_constructor_binding(value, constructor, scopes, depth, active),
        K::ResolvedVar(var) => {
            let V::Local(local) = var.target else {
                return (false, false);
            };
            if !active.insert((depth, local)) {
                return (false, false);
            }
            let scope = &scopes[depth];
            let result = if let Some(defs) = scope.defs.get(&local) {
                let mut contains = false;
                let mut exact = !scope.uncertain_writes
                    && !defs.is_empty()
                    && !scope.params.iter().any(|param| param.local_id == local)
                    && !scope
                        .captures
                        .iter()
                        .any(|capture| capture.local_id == local);
                for value in defs {
                    let alternative =
                        pre_constructor_binding(value, constructor, scopes, depth, active);
                    contains |= alternative.0;
                    exact &= alternative.1;
                }
                (contains, exact)
            } else if depth > 0
                && scope
                    .captures
                    .iter()
                    .any(|capture| capture.local_id == local)
            {
                pre_constructor_binding(value, constructor, scopes, depth - 1, active)
            } else {
                (false, false)
            };
            active.remove(&(depth, local));
            result
        }
        _ => (false, false),
    }
}

fn pre_walk_block<'a>(
    block: &'a PreBlock,
    stmt: &mut impl FnMut(&'a PreStmt),
    expr: &mut impl FnMut(&'a PreExpr),
) {
    for statement in &block.stmts {
        stmt(statement);
        match statement {
            PreStmt::Let { value, .. }
            | PreStmt::Expr(value)
            | PreStmt::Return(Some(value))
            | PreStmt::Break(Some(value)) => pre_walk_expr(value, stmt, expr),
            _ => {}
        }
    }
}

fn pre_walk_expr<'a>(
    value: &'a PreExpr,
    stmt: &mut impl FnMut(&'a PreStmt),
    expr: &mut impl FnMut(&'a PreExpr),
) {
    use crate::hir::HirExprKindFor as K;
    expr(value);
    match &value.kind {
        K::PackagePack { payload, .. } => pre_walk_expr(payload, stmt, expr),
        K::PackageOpen { source, body, .. } | K::Open { source, body, .. } => {
            pre_walk_expr(source, stmt, expr);
            pre_walk_block(body, stmt, expr);
        }
        K::ArrayLiteral(values)
        | K::TupleLiteral(values)
        | K::EnumVariant(_, _, values, _)
        | K::Intrinsic { args: values, .. } => {
            for value in values {
                pre_walk_expr(value, stmt, expr);
            }
        }
        K::ArrayRepeat(value, _)
        | K::FieldAccess(value, _, _)
        | K::TupleIndex(value, _)
        | K::UnaryOp(_, value)
        | K::Ref(_, value)
        | K::Deref(value)
        | K::Cast(value, _)
        | K::ObjectCoercion(value, _)
        | K::Try { expr: value, .. } => pre_walk_expr(value, stmt, expr),
        K::BinOp(_, left, right) | K::Assign(left, right) => {
            pre_walk_expr(left, stmt, expr);
            pre_walk_expr(right, stmt, expr);
        }
        K::Call(callee, args, _)
        | K::MethodCall(callee, _, args, _, _)
        | K::OwnedObjectCall {
            owner: callee,
            args,
            ..
        } => {
            pre_walk_expr(callee, stmt, expr);
            for arg in args {
                pre_walk_expr(arg, stmt, expr);
            }
        }
        K::StructLiteral(_, _, fields) => {
            for field in fields {
                pre_walk_expr(&field.value, stmt, expr);
            }
        }
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            pre_walk_expr(condition, stmt, expr);
            pre_walk_block(then_branch, stmt, expr);
            if let Some(block) = else_branch {
                pre_walk_block(block, stmt, expr);
            }
        }
        K::Match { scrutinee, arms } => {
            pre_walk_expr(scrutinee, stmt, expr);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    pre_walk_expr(guard, stmt, expr);
                }
                pre_walk_block(&arm.body, stmt, expr);
            }
        }
        K::While { condition, body } => {
            pre_walk_expr(condition, stmt, expr);
            pre_walk_block(body, stmt, expr);
        }
        K::For { iter, body, .. } => {
            pre_walk_expr(iter, stmt, expr);
            pre_walk_block(body, stmt, expr);
        }
        K::Loop(body) | K::Block(body) | K::UnsafeBlock(body) => pre_walk_block(body, stmt, expr),
        // Never enter an unrelated lambda while indexing local definitions.
        K::Lambda { .. }
        | K::IntLiteral(_)
        | K::FloatLiteral(_)
        | K::BoolLiteral(_)
        | K::StringLiteral(_)
        | K::CharLiteral(_)
        | K::Unit
        | K::Var(_)
        | K::ResolvedVar(_) => {}
    }
}

fn pre_anchor(expr: &PreExpr, path: &std::path::Path) -> bool {
    expr.span.file_path == path && (expr.span.start, expr.span.end) == (11676, 11683)
}

fn pre_contains_anchor(block: &PreBlock, path: &std::path::Path) -> bool {
    let mut found = false;
    pre_walk_block(block, &mut |_| {}, &mut |expr| {
        found |= pre_anchor(expr, path);
        if let HirExprKindFor::Lambda { body, .. } = &expr.kind {
            found |= pre_contains_anchor(body, path);
        }
    });
    found // Visited metadata is not admitted proof or a saved global collector.
}

fn pre_local(
    snapshot: &mut PreBindingSnapshot,
    mono: &MonomorphizedProgram,
    owner: InstanceId,
    scopes: &[PreScope<'_>],
    depth: usize,
    local: HirLocalId,
    active: &mut HashSet<(usize, HirLocalId)>,
) {
    snapshot.bindings += 1;
    snapshot.budget();
    snapshot.rows.push(format!("binding nativeOwner={owner:?} lexicalDepth={depth} local={local:?}; all definition/write alternatives, no cross-invocation pairing"));
    if !active.insert((depth, local)) {
        snapshot.unknown("cyclic LocalId binding; no constructor inferred");
        return;
    }
    let scope = &scopes[depth];
    if scope.uncertain_writes {
        snapshot.unknown("scope contains projected/unresolved assignment; indexed direct writes are not a complete binding proof");
    }
    if let Some(defs) = scope.defs.get(&local) {
        if scope.params.iter().any(|param| param.local_id == local)
            || scope
                .captures
                .iter()
                .any(|capture| capture.local_id == local)
        {
            snapshot.unknown("writes to an incoming parameter/capture do not account for its initial binding; indexed writes remain observations only");
        }
        for value in defs {
            pre_value(snapshot, mono, owner, scopes, depth, value, active);
        }
    } else if let Some((ordinal, param)) = scope
        .params
        .iter()
        .enumerate()
        .find(|(_, param)| param.local_id == local)
    {
        snapshot.rows.push(format!(
            "parameterBoundary local={local:?} type={:?} mutable={} isRef={}",
            param.ty, param.mutable, param.is_ref
        ));
        if let Some(incoming) = scope.incoming {
            snapshot.incoming_bindings += 1;
            assert_eq!(
                incoming.args.len(),
                scope.params.len(),
                "actual relay invocation arity must match its constructor"
            );
            snapshot.rows.push(format!("incomingInvocationBinding nativeOwner={owner:?} callerDepth={} callSpan={:?} parameter={local:?} argumentOrdinal={ordinal}; exact borrowed constructor occurrence, NOT result authority",
                incoming.caller_depth, incoming.call.span));
            pre_value(
                snapshot,
                mono,
                owner,
                scopes,
                incoming.caller_depth,
                &incoming.args[ordinal],
                active,
            );
        } else {
            snapshot.unknown("actual invocation parameter has no retained incoming binding in this lexical scope; request invocation-specific input snapshot");
        }
    } else if let Some(capture) = scope
        .captures
        .iter()
        .find(|capture| capture.local_id == local)
    {
        snapshot.rows.push(format!(
            "lexicalCaptureBinding local={local:?} type={:?} kind={:?}",
            capture.ty, capture.kind
        ));
        if depth == 0 {
            snapshot.unknown("capture has no enclosing retained lexical scope");
            active.remove(&(depth, local));
            return;
        }
        pre_local(snapshot, mono, owner, scopes, depth - 1, local, active);
    } else {
        snapshot
            .unknown("canonical LocalId definition/capture/input absent; no name or span fallback");
    }
    active.remove(&(depth, local));
}

fn pre_value(
    snapshot: &mut PreBindingSnapshot,
    mono: &MonomorphizedProgram,
    owner: InstanceId,
    scopes: &[PreScope<'_>],
    depth: usize,
    value: &PreExpr,
    active: &mut HashSet<(usize, HirLocalId)>,
) {
    use crate::hir::{HirExprKindFor as K, HirVarTarget as V};
    // Each selected value-source alternative is a binding row too. This bounds
    // repeated alternatives/wrapper chains before formatting, without deduping.
    snapshot.bindings += 1;
    snapshot.budget();
    snapshot.rows.push(format!(
        "valueSource nativeOwner={owner:?} lexicalDepth={depth} span={:?} type={:?}",
        value.span, value.ty
    ));
    match &value.kind {
        K::ResolvedVar(var) => match &var.target {
            V::Local(local) => pre_local(snapshot, mono, owner, scopes, depth, *local, active),
            V::Instance(id) => snapshot.root(mono, *id),
            V::Function(id) | V::Extern(id) => snapshot.rows.push(format!("canonicalVarTarget={:?} definition={id:?}; no instance/substitution guessed", var.target)),
        },
        K::Lambda { captures, .. } => {
            snapshot.constructor(owner, value);
            for (ordinal, capture) in captures.iter().enumerate() {
                snapshot.rows.push(format!("capture ordinal={ordinal} local={:?} kind={:?} type={:?}; actual HIR edge only, MIR emission UNKNOWN", capture.local_id, capture.kind, capture.ty));
                pre_local(snapshot, mono, owner, scopes, depth, capture.local_id, active);
            }
        }
        K::Ref(mutable, inner) => {
            snapshot.rows.push(format!("reference mutable={mutable}; storage borrow not callable-result authority"));
            pre_value(snapshot, mono, owner, scopes, depth, inner, active);
        }
        K::FieldAccess(_, _, field) => {
            snapshot.rows.push(format!("projection field={field:?}"));
            snapshot.unknown("projected callable binding unavailable; stop target expansion at this input");
        }
        K::TupleIndex(_, index) => {
            snapshot.rows.push(format!("projection tupleIndex={index}"));
            snapshot.unknown("projected input requires exact field binding; stop target expansion");
        }
        K::Cast(_, _) | K::ObjectCoercion(_, _) | K::Deref(_) => snapshot.unknown("cast/erased/dereferenced input binding unavailable; stop target expansion"),
        K::Call(_, _, target) => {
            snapshot.rows.push(format!("returnedCallValue target={target:?}"));
            snapshot.unknown("call-produced callable value has no retained constructor binding; do not walk its return graph");
            // An incoming call-produced value is a boundary, not another
            // invocation root; expanding its inputs loses the bounded selector.
        }
        K::If { then_branch, else_branch, .. } => {
            pre_outputs(snapshot, mono, owner, scopes, depth, then_branch, active);
            if let Some(branch) = else_branch { pre_outputs(snapshot, mono, owner, scopes, depth, branch, active); }
            else { snapshot.unknown("selected conditional has no else value binding"); }
        }
        K::Match { arms, .. } => {
            for arm in arms { pre_outputs(snapshot, mono, owner, scopes, depth, &arm.body, active); }
        }
        K::Block(block) | K::UnsafeBlock(block) => pre_outputs(snapshot, mono, owner, scopes, depth, block, active),
        K::Var(_) => snapshot.unknown("unresolved display variable has no canonical LocalId"),
        _ => snapshot.unknown("value-source form lacks exact callable binding in this observer; no alternatives fabricated"),
    }
}

fn pre_outputs(
    snapshot: &mut PreBindingSnapshot,
    mono: &MonomorphizedProgram,
    owner: InstanceId,
    scopes: &[PreScope<'_>],
    depth: usize,
    block: &PreBlock,
    active: &mut HashSet<(usize, HirLocalId)>,
) {
    // Explicit return alternatives and the real lexical tail only; no statement
    // consumer/backwards whole-function result analysis.
    pre_walk_block(
        block,
        &mut |statement| {
            if let PreStmt::Return(Some(value)) = statement {
                pre_value(snapshot, mono, owner, scopes, depth, value, active);
            }
        },
        &mut |_| {},
    );
    if let Some(PreStmt::Expr(value)) = block.stmts.last() {
        pre_value(snapshot, mono, owner, scopes, depth, value, active);
    }
}

fn pre_invocation(
    snapshot: &mut PreBindingSnapshot,
    mono: &MonomorphizedProgram,
    owner: InstanceId,
    scopes: &[PreScope<'_>],
    depth: usize,
    expr: &PreExpr,
    callee: &PreExpr,
    args: &[PreExpr],
    target: Option<&crate::hir::HirCallTarget>,
    active: &mut HashSet<(usize, HirLocalId)>,
) {
    use crate::hir::HirCallTarget as T;
    snapshot.invocations += 1;
    snapshot.budget();
    assert!(args.len() <= 8, "pre invocation argument cap8 exceeded");
    snapshot.rows.push(format!("invocation nativeOwner={owner:?} lexicalDepth={depth} sourceOwner={:?} span={:?} canonicalTarget={target:?}",
        mono.instances.get(&owner).map(|record| origin_definition(&record.origin)), expr.span));
    match target {
        Some(T::Instance(id)) => snapshot.root(mono, *id),
        Some(T::StaticMethod(method)) => {
            snapshot.rows.push(format!("actualStaticTarget={method:?}; definition authority only, no inferred Mono specialization"));
            let definition = match &method.method.target {
                crate::hir::HirSelectedMethodTarget::ImplMethod { method_id, .. } => *method_id,
                crate::hir::HirSelectedMethodTarget::TraitMethod { member_id, .. } => *member_id,
            };
            snapshot.definition_records(mono, definition);
        }
        Some(T::Function(id)) | Some(T::Extern(id)) => {
            snapshot.rows.push(format!(
                "actualDefinitionTarget={id:?}; no guessed instance"
            ));
            snapshot.definition_records(mono, *id);
        }
        Some(T::Local(local)) => snapshot.rows.push(format!(
            "actualLocalCallTarget={local:?}; observations do NOT resolve indirect invocation"
        )),
        _ => snapshot.unknown("invocation lacks retained native/local/static method target"),
    }
    snapshot.rows.push("callee binding path".into());
    pre_value(snapshot, mono, owner, scopes, depth, callee, active);
    for (ordinal, arg) in args.iter().enumerate() {
        snapshot.rows.push(format!(
            "invocationArgument ordinal={ordinal} target={target:?}"
        ));
        if let HirExprKindFor::Lambda { params, .. } = &callee.kind {
            snapshot.rows.push(format!("directLambdaArgument ordinal={ordinal} actualParameterLocal={:?}; no lambda-body/return traversal",
                params.get(ordinal).map(|param| param.local_id)));
        }
        pre_value(snapshot, mono, owner, scopes, depth, arg, active);
    }
}

fn pre_scope<'a>(
    snapshot: &mut PreBindingSnapshot,
    mono: &MonomorphizedProgram,
    owner: InstanceId,
    block: &'a PreBlock,
    params: &'a [crate::hir::HirParam],
    captures: &'a [crate::hir::HirClosureCapture],
    incoming: Option<PreIncoming<'a>>,
    scopes: &mut Vec<PreScope<'a>>,
    path: &std::path::Path,
) {
    use crate::hir::{HirExprKindFor as K, HirVarTarget as V};
    let mut defs = HashMap::<HirLocalId, Vec<&PreExpr>>::new();
    let mut writes = Vec::new();
    let mut anchors = Vec::new();
    let mut nested = Vec::new();
    let mut calls = Vec::new();
    let mut uncertain_writes = false;
    pre_walk_block(
        block,
        &mut |statement| {
            if let PreStmt::Let {
                local_id, value, ..
            } = statement
            {
                defs.entry(*local_id).or_default().push(value);
            }
        },
        &mut |expr| {
            if let K::Call(callee, args, target) = &expr.kind {
                calls.push((expr, callee.as_ref(), args.as_slice(), target.as_ref()));
            }
            if pre_anchor(expr, path) {
                anchors.push(expr);
            }
            if let K::Assign(left, right) = &expr.kind {
                match &left.kind {
                    K::ResolvedVar(var) => match var.target {
                        V::Local(local) => writes.push((local, right.as_ref())),
                        _ => uncertain_writes = true,
                    },
                    _ => uncertain_writes = true,
                }
            }
            if let K::Lambda { body, .. } = &expr.kind {
                if pre_contains_anchor(body, path) {
                    nested.push(expr);
                }
            }
        },
    );
    for (local, value) in writes {
        defs.entry(local).or_default().push(value);
    }
    scopes.push(PreScope {
        defs,
        uncertain_writes,
        params,
        captures,
        incoming,
    });
    let depth = scopes.len() - 1;
    for expr in anchors {
        snapshot.anchors += 1;
        // Span is solely the pinned source-observation anchor, never target proof.
        if let K::Call(callee, args, target) = &expr.kind {
            let mut active = HashSet::new();
            pre_invocation(
                snapshot,
                mono,
                owner,
                scopes,
                depth,
                expr,
                callee,
                args,
                target.as_ref(),
                &mut active,
            );
        } else if matches!(expr.kind, K::MethodCall(..) | K::OwnedObjectCall { .. }) {
            snapshot.invocations += 1;
            snapshot.budget();
            snapshot.unknown(
                "anchored operation is not a retained static Call; no receiver target/body guessed",
            );
        }
    }
    for lambda in nested {
        let K::Lambda {
            params,
            captures,
            body,
        } = &lambda.kind
        else {
            unreachable!()
        };
        snapshot.constructor(owner, lambda);
        for (ordinal, capture) in captures.iter().enumerate() {
            snapshot.rows.push(format!(
                "enclosingConstructorCapture ordinal={ordinal} local={:?} kind={:?} type={:?}",
                capture.local_id, capture.kind, capture.ty
            ));
        }
        let mut incoming_calls = Vec::new();
        for &(call, callee, args, target) in &calls {
            let (contains, exact) =
                pre_constructor_binding(callee, lambda, scopes, depth, &mut HashSet::new());
            if !contains {
                continue;
            }
            if !exact {
                snapshot.unknown("relay callee LocalId alternatives include unsupported or distinct constructors; no invocation parameter edge admitted");
                continue;
            }
            // Only an indirect/local call can invoke this actual constructor;
            // a contradictory canonical native target cannot be ignored.
            if !matches!(target, None | Some(crate::hir::HirCallTarget::Local(_))) {
                snapshot.unknown(
                    "relay occurrence disagrees with retained canonical native call target",
                );
                continue;
            }
            assert!(
                args.len() <= 8 && args.len() == params.len(),
                "relay invocation argument/parameter cap8 and exact arity"
            );
            snapshot.invocations += 1;
            snapshot.relay_invocations += 1;
            snapshot.budget();
            snapshot.rows.push(format!("relayInvocation nativeOwner={owner:?} callerDepth={depth} callSpan={:?} canonicalTarget={target:?} actualParameterLocals={:?}; all callee alternatives identify the same borrowed constructor, NO MirClosureId",
                call.span, params.iter().map(|param| param.local_id).collect::<Vec<_>>()));
            incoming_calls.push(PreIncoming {
                call,
                args,
                caller_depth: depth,
            });
        }
        if incoming_calls.is_empty() {
            pre_scope(
                snapshot, mono, owner, body, params, captures, None, scopes, path,
            );
        } else {
            // Keep each caller separate; never union operands across invocations.
            for incoming in incoming_calls {
                pre_scope(
                    snapshot,
                    mono,
                    owner,
                    body,
                    params,
                    captures,
                    Some(incoming),
                    scopes,
                    path,
                );
            }
        }
    }
    scopes.pop();
}

fn pre_snapshot(mono: &MonomorphizedProgram, path: &std::path::Path) -> PreBindingSnapshot {
    let mut snapshot = PreBindingSnapshot::default();
    for record in mono.instances.values() {
        let Some(body) = mono.pre_mir_instance_bodies.get(record.id) else {
            continue;
        };
        if !pre_contains_anchor(&body.body, path) {
            continue;
        }
        snapshot.root(mono, record.id);
        pre_scope(
            &mut snapshot,
            mono,
            record.id,
            &body.body,
            &body.params,
            &[],
            None,
            &mut Vec::new(),
            path,
        );
    }
    snapshot.budget();
    eprintln!("HKT_PRE_MIR totals roots={} constructors={} bindings={} invocations={} relayInvocations={} incomingBindings={} sourceAnchorOccurrences={} unknown={} caps=6/8/16/4; specialized AcceptedHir representation, no MIR closure identity/return authority; original FULL17 observer NOT run",
        snapshot.roots.len(), snapshot.constructors, snapshot.bindings, snapshot.invocations, snapshot.relay_invocations, snapshot.incoming_bindings, snapshot.anchors, snapshot.unknown);
    for row in &snapshot.rows {
        eprintln!("HKT_PRE_MIR {row}");
    }
    assert!(
        snapshot.anchors > 0 && !snapshot.roots.is_empty(),
        "HKT_PRE_MIR STOP: no actual canonical native-owner body retains pinned source anchor"
    );
    if snapshot.invocations == 0 {
        eprintln!("HKT_PRE_MIR UNKNOWN: source operation retained but canonical invocation absent; request anchored invocation's retained call-target binding, no body/name sweep");
    }
    snapshot
}

#[test]
fn hkt_historical_pre_mir_receipt_remains_frozen() {
    let path = agent_work_path("logs/hkt-callable-result-observation-6.log");
    assert_eq!(integration_sha(&path),
        "282e7af10886cb7946402e94895b3d5ea1dd918d75582cefa26183a7ee117eb5");
    let log = std::fs::read_to_string(path).unwrap();
    assert!(log.contains("HKT_PRE_MIR observationOnly=true BorrowRuns=1 diagnosticMultiset=2Borrow exactSourceRegistration=transported acceptance=UNPROVED"));
    assert!(log.contains("test mir::borrowck::hkt_return_origin_tests::hkt_pre_mir_binding_authority_standalone_observation ... ok"));
}

#[test]
fn hkt_pre_mir_binding_authority_standalone_observation() {
    let case = "test_hkt_standalone_and_receiver_operations";
    let sha = "8b0c3e611ea091aacccb091132b65835cd9af9c34bf632d08419c8bf42b450a6";
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let source = integration_literal(
        &root.join("lib/tests/integration.rs"),
        &format!("fn {case}()"),
    );
    assert_eq!(source.len(), 1317);
    let vec_path = root.join("stdlib/vec.rk");
    let vec_bytes = std::fs::read(&vec_path).unwrap();
    assert_eq!(vec_bytes.len(), 11927);
    assert_eq!(
        integration_sha(&vec_path),
        "1cd5d65d0bb35535af3cd1426066a3ca41ea46375bda08fe800e416827c73e1b"
    );
    assert_eq!(&vec_bytes[11676..11683], b"G::bind");
    ACTIVE.with(|active| {
        assert!(
            active.borrow().is_none(),
            "pre observation must not enable FULL return observer"
        )
    });
    let snapshot = RefCell::new(None);
    let _allocation_scope = PreAllocationScope::enter();
    let (program, entry, verdict) = mutation_source_program_with_phase_callbacks(
        "hkt_pre_mir_standalone",
        &source,
        true,
        |mono| {
            let observed = pre_snapshot(mono, &vec_path);
            for root in &observed.roots {
                let record = mono.instances.get(root).unwrap();
                let owner = origin_definition(&record.origin);
                let body = mono.pre_mir_instance_bodies.get(*root).unwrap();
                assert!(body.params.len() <= 8);
                eprintln!("HKT_SOURCE_REGISTRATION importedInstance={root:?} actualOwner={owner:?} importedDefinitionSpan={:?} retainedBodyDef={:?}", mono.source_map.definition_span(owner), body.id);
                for parameter in &body.params {
                    assert!(mono.source_map.local_span(owner, parameter.local_id).is_some(), "exact producer parameter registration survives import");
                    eprintln!("HKT_SOURCE_REGISTRATION importedInstance={root:?} actualOwner={owner:?} actualParameter={:?} importedLocalSpan={:?}; missing is not a fallback span", parameter.local_id, mono.source_map.local_span(owner, parameter.local_id));
                }
            }
            PRE_ALLOCATIONS.with(|active| {
                active.borrow_mut().as_mut().unwrap().selected =
                    observed.selected_constructors.clone()
            });
            *snapshot.borrow_mut() = Some(observed);
        },
        |_, _, program| {
            verify_pre_allocations(program);
            BorrowChecker::run(program)
        },
    );
    assert!(
        snapshot.borrow().is_some(),
        "pre-consumption callback must run"
    );
    assert_eq!(
        snapshot
            .borrow()
            .as_ref()
            .map(|snapshot| (snapshot.relay_invocations, snapshot.incoming_bindings)),
        Some((1, 2)),
        "one actual relay invocation must couple both immediate parameters to its own operands"
    );
    assert!(crate::mir::agreement::check_mir_runtime_agreement(&program).is_clean());
    if let Err(diagnostics) = &verdict {
        for diagnostic in &diagnostics.0 {
            eprintln!("HKT_PRE_MIR currentDiagnostic={diagnostic:?}; historical2Borrow is not current acceptance");
        }
    }
    assert_eq!(std::fs::read(&entry).unwrap(), source.as_bytes());
    assert_eq!(integration_sha(&entry), sha);
    assert_eq!(std::fs::read(&vec_path).unwrap(), vec_bytes);
    eprintln!("HKT_PRE_MIR observationOnly=true BorrowRuns=1 exactSourceRegistration=transported historicalDiagnosticMultiset=(0,2) historicalSites=option1395..1403/vec11676..11683 historicalReceipt=hkt-callable-result-observation-6.log; observation is not callable authority");
    assert!(verdict.is_ok(), "original HKT positive must pass Borrow after bounded pre-MIR observation: {verdict:?}");
}

#[test]
fn hkt_pre_mir_constructor_selector_preserves_local_alternatives() {
    use crate::hir::{HirExprKindFor as K, HirVarRef, HirVarTarget as V};
    let constructor = PreExpr {
        kind: K::Lambda {
            params: Vec::new(),
            captures: Vec::new(),
            body: PreBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
        },
        ty: Type::Unit,
        span: Span::new(agent_work_path("fixtures/selector.rk"), 0, 1),
    };
    let same_metadata = constructor.clone();
    let local = HirLocalId(0);
    let alias = PreExpr {
        kind: K::ResolvedVar(HirVarRef {
            name: "not_authority".into(),
            target: V::Local(local),
        }),
        ..constructor.clone()
    };
    let projected = PreExpr {
        kind: K::TupleIndex(Box::new(alias.clone()), 0),
        ..constructor.clone()
    };
    let mut scopes = vec![PreScope {
        defs: HashMap::new(),
        uncertain_writes: false,
        params: &[],
        captures: &[],
        incoming: None,
    }];
    let selected = |value: &PreExpr, scopes: &[PreScope<'_>]| {
        pre_constructor_binding(value, &constructor, scopes, 0, &mut HashSet::new())
    };
    assert_eq!(selected(&constructor, &scopes), (true, true));
    assert_eq!(
        selected(&same_metadata, &scopes),
        (false, false),
        "equal spans/types/bodies cannot invent occurrence identity"
    );
    scopes[0].defs.insert(local, vec![&constructor]);
    assert_eq!(selected(&alias, &scopes), (true, true));
    scopes[0]
        .defs
        .insert(local, vec![&constructor, &same_metadata]);
    assert_eq!(
        selected(&alias, &scopes),
        (true, false),
        "distinct constructor alternatives veto a unique incoming edge"
    );
    scopes[0].defs.insert(local, vec![&constructor, &projected]);
    assert_eq!(
        selected(&alias, &scopes),
        (true, false),
        "unsupported projection cannot be dropped from alternatives"
    );
    scopes[0].defs.insert(local, vec![&alias]);
    assert_eq!(
        selected(&alias, &scopes),
        (false, false),
        "cyclic local is unresolved"
    );
    scopes[0].defs.insert(local, vec![&constructor]);
    scopes[0].uncertain_writes = true;
    assert_eq!(
        selected(&alias, &scopes),
        (true, false),
        "unindexed writes veto complete local binding"
    );
    scopes[0].uncertain_writes = false;
    let params = [crate::hir::HirParam {
        name: "not_authority".into(),
        local_id: local,
        ty: Type::Unit,
        mutable: true,
        is_ref: false,
    }];
    scopes[0].params = &params;
    assert_eq!(
        selected(&alias, &scopes),
        (true, false),
        "parameter writes cannot erase its unaccounted incoming alternative"
    );
}
