pub mod accesses;
pub mod borrows;
mod call_targets;
mod callable_results;
pub mod closures;
pub mod conflicts;
pub mod diagnostics;
pub mod liveness;
pub mod location;
pub mod paths;
pub mod provenance;
mod return_sources;

#[cfg(test)]
mod borrowed_contents_tests;

#[cfg(test)]
mod allocator_trace_tests;

#[cfg(test)]
pub(crate) mod hkt_return_origin_tests;

#[cfg(test)]
mod slice_source_authority_tests;

#[cfg(test)]
mod utf8_return_origin_tests;

#[cfg(test)]
mod native_return_contract_tests;

use std::collections::{HashMap, HashSet};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::ids::{AssocTypeId, DefId, TypeId};
use crate::lexer::Span;
use crate::mir::borrowck::accesses::AccessKind;
use crate::mir::borrowck::borrows::collect_function_borrows_with_contract;
use crate::mir::borrowck::closures::CaptureKind;
use crate::mir::borrowck::diagnostics::borrow_error;
use crate::mir::borrowck::liveness::{apply_active_loan_statement_transfer, LoanLocationIndex};
#[cfg(test)]
use crate::mir::borrowck::liveness::{
    compute_active_loan_entries_with_contract, compute_reference_liveness_with_contract,
};
use crate::mir::dataflow::analyses::{
    InitError, InitMap, InitializationAnalysis, LoanAnalysis, LoanKind, LoanState, LoanTable,
};
use crate::mir::dataflow::{run_fixpoint, Analysis, Lattice};
use crate::mir::origins::{OriginSet, ReferenceOriginState};
use crate::mir::{
    Constant, DropObligationKind, Local, LocalSource, MirBackendContract, MirCallable,
    MirCallableKey, MirFunction, MirFunctionId, MirIntrinsicId, MirNominalLayout, MirProgram,
    MirProjectionKey, Mutability, Operand, Place, Projection, ReferenceOrigin, Rvalue,
    StatementData, StatementKind, Terminator,
};
use crate::type_context::{Ty, TypeView};
use crate::types::{GenericParamId, Type};

fn mir_diagnostic(message: String, span: Option<Span>) -> Diagnostic {
    match span {
        Some(span) => Diagnostic::new(message, span).with_code(DiagnosticCode::Borrow),
        None => Diagnostic::for_internal(message),
    }
}

pub struct BorrowChecker;

struct MoveValidationContext<'a> {
    type_context: &'a crate::type_context::TypeContext,
    backend_contract: &'a MirBackendContract,
    direct_drop_types: HashSet<TypeId>,
    cleanup_cache: HashMap<Type, bool>,
}

/// Compile-local mutation knowledge, deliberately separate from return proofs.
/// Raw stores, indirect calls and recursion remain Unknown; no backing seal is
/// inferred from ParamBorrowedContents or a pointer producer's return envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeMutationKnowledge {
    Unknown,
    CompletedNoEffects,
}

#[derive(Default)]
struct NativeMutationTable(HashMap<MirCallableKey, NativeMutationKnowledge>);

impl NativeMutationTable {
    fn for_program(program: &MirProgram) -> Self {
        let mut table = Self::default();
        let contract = &program.backend_contract;
        let tc = &program.type_context;
        let headers_valid = crate::mir::backend_contract::validate_backend_contract_against_mir(
            contract,
            program.functions().map(|(_, body)| body),
        )
        .is_empty();
        for (key, header) in &contract.callables {
            table
                .0
                .insert(key.clone(), NativeMutationKnowledge::Unknown);
            if !header.signature.params.iter().any(|param| {
                tc.type_id_tree_is_valid(param.semantic_ty)
                    && matches!(
                        contract.normalize_storage_type(tc, &tc.type_for(param.semantic_ty)),
                        Type::Reference { mutable: true, .. }
                    )
            }) {
                continue;
            } // Only candidate mutable-storage bodies need completion.
            let crate::mir::MirCallableKind::LocalBody { function_id } = &header.kind else {
                continue;
            };
            let Some((_, body)) = program.functions().find(|(id, _)| *id == function_id) else {
                continue;
            };
            if !headers_valid
                || &header.key != key
                || contract.function_bodies.get(function_id) != Some(key)
                || MirCallable::Resolved(key.clone()).function_id().as_ref() != Some(function_id)
                || &body.id != function_id
                || body.basic_blocks.is_empty()
                || header.signature.params.len() != body.arg_count
                || !header
                    .signature
                    .params
                    .iter()
                    .enumerate()
                    .all(|(i, param)| {
                        body.local_decls
                            .get(i + 1)
                            .is_some_and(|local| local.ty == param.semantic_ty)
                    })
                || header.signature.ret.semantic_ty != body.ret_type
                || header.signature.ret.abi_ty != body.ret_type
                || !body
                    .local_decls
                    .first()
                    .is_some_and(|local| local.ty == body.ret_type)
                || !body
                    .local_decls
                    .iter()
                    .all(|local| tc.type_id_tree_is_valid(local.ty))
            {
                continue;
            }
            // lean-ctx: no-call/local-write ceiling; upgrade only with checked
            // direct storage effects. All blocks are checked, not just one path.
            let no_effects = body.basic_blocks.iter().all(|block| {
                block
                    .statements
                    .iter()
                    .all(|statement| match &statement.kind {
                        StatementKind::Assign(place, _) => {
                            place.projection.is_empty()
                                && contract.place_type_id(tc, body, place).is_some()
                                && body.local_decls.get(place.local.0).is_some_and(|local| {
                                    matches!(
                                        local.source,
                                        LocalSource::ReturnPlace
                                            | LocalSource::UserBinding
                                            | LocalSource::Temporary
                                    )
                                })
                        }
                        StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                            body.local_decls.get(local.0).is_some()
                        }
                        StatementKind::Assert(_) => true,
                    })
                    && block.statements.iter().all(|statement| {
                        accesses::classify_statement(statement, tc)
                            .iter()
                            .all(|event| {
                                checked_place_storage_type(tc, contract, body, &event.place)
                                    .is_some()
                            })
                    })
                    && matches!(
                        block.terminator,
                        Some(
                            Terminator::Return
                                | Terminator::ReturnWithOrigin { .. }
                                | Terminator::Goto(_)
                                | Terminator::GotoWithOrigin { .. }
                                | Terminator::SwitchInt { .. }
                                | Terminator::SwitchIntWithOrigin { .. }
                                | Terminator::Unreachable { .. }
                        )
                    )
            });
            if no_effects {
                table
                    .0
                    .insert(key.clone(), NativeMutationKnowledge::CompletedNoEffects);
            }
        }
        table
    }

    fn knowledge(&self, key: &MirCallableKey) -> NativeMutationKnowledge {
        self.0
            .get(key)
            .copied()
            .unwrap_or(NativeMutationKnowledge::Unknown)
    }
}

#[cfg(test)]
fn checked_place_type(
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    function: &MirFunction,
    place: &Place,
) -> Option<Type> {
    checked_place_storage_type(tc, contract, function, place)
        .map(|ty| contract.normalize_type(tc, &ty))
}

fn checked_place_storage_type(
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    function: &MirFunction,
    place: &Place,
) -> Option<Type> {
    let root = function.local_decls.get(place.local.0)?.ty;
    if !tc.type_id_tree_is_valid(root) {
        return None;
    }
    let ty = contract.place_type_id(tc, function, place)?;
    if !tc.type_id_tree_is_valid(ty) {
        return None;
    }
    Some(contract.normalize_storage_type(tc, &tc.type_for(ty)))
}

/// Type containment is only a conservative destination screen, not memory
/// identity, write permission, or a positive no-borrow proof.
fn mutation_storage_may_carry(
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    storage: &Type,
    payload: &Type,
    seen: &mut HashSet<Type>,
) -> bool {
    let storage = contract.normalize_storage_type(tc, storage);
    // A measured layout cannot disprove semantic containment. This is only
    // conservative retention screening, never storage or effect authority.
    if crate::type_services::callable::value_representation(&storage, payload).is_some() {
        return true;
    }
    if let Type::Struct { id, .. }
    | Type::StructRepresentation { id, .. }
    | Type::Enum { id, .. }
    | Type::EnumRepresentation { id, .. } = &storage
    {
        if seen.iter().any(|ty| matches!(ty, Type::Struct { id: other, .. } | Type::StructRepresentation { id: other, .. } | Type::Enum { id: other, .. } | Type::EnumRepresentation { id: other, .. } if other == id)) {
            return true; // Recursive/expanding nominal layouts cannot disprove containment.
        }
    }
    if !seen.insert(storage.clone()) {
        return false;
    }
    match &storage {
        Type::StructRepresentation { args, fields, .. } => args.iter().chain(fields).any(|field| {
            mutation_storage_may_carry(tc, contract, field, payload, &mut seen.clone())
        }),
        Type::EnumRepresentation { args, variants, .. } => {
            args.iter().chain(variants.iter().flatten()).any(|field| {
                mutation_storage_may_carry(tc, contract, field, payload, &mut seen.clone())
            })
        }
        Type::Struct { id, args } | Type::Enum { id, args } => {
            let (parameters, fields) = match contract.nominal_layouts.get(id) {
                Some(MirNominalLayout::Struct {
                    id: owner,
                    generic_params,
                    fields,
                }) if owner == id => (
                    generic_params,
                    fields.iter().map(|(_, ty)| *ty).collect::<Vec<_>>(),
                ),
                Some(MirNominalLayout::Enum {
                    id: owner,
                    generic_params,
                    variants,
                }) if owner == id => (
                    generic_params,
                    variants
                        .iter()
                        .flat_map(|variant| match &variant.fields {
                            crate::mir::MirVariantLayoutFields::Unit => Vec::new(),
                            crate::mir::MirVariantLayoutFields::Positional(fields) => {
                                fields.clone()
                            }
                            crate::mir::MirVariantLayoutFields::Named(fields) => {
                                fields.iter().map(|(_, ty)| *ty).collect()
                            }
                        })
                        .collect(),
                ),
                _ => return true, // Missing layout cannot establish non-containment.
            };
            if parameters.len() != args.len() {
                return true;
            }
            // Only ancestors are recursive; an argument and a physical field
            // can be the same ordinary type without containing each other.
            if args.iter().any(|arg| {
                mutation_storage_may_carry(tc, contract, arg, payload, &mut seen.clone())
            }) {
                return true; // Logical/phantom slots may conservatively over-retain.
            }
            let substitution = parameters
                .iter()
                .copied()
                .zip(args.iter().cloned())
                .collect::<HashMap<_, _>>();
            fields.into_iter().any(|field| {
                !tc.type_id_tree_is_valid(field)
                    || mutation_storage_may_carry(
                        tc,
                        contract,
                        &tc.type_for(field).substitute_generics(&substitution),
                        payload,
                        &mut seen.clone(),
                    )
            })
        }
        Type::Tuple(elements) => elements.iter().any(|element| {
            mutation_storage_may_carry(tc, contract, element, payload, &mut seen.clone())
        }),
        Type::Array(inner, _) | Type::Slice(inner) => {
            mutation_storage_may_carry(tc, contract, inner, payload, seen)
        }
        Type::Exists(_)
        | Type::Object(_)
        | Type::ObjectSelf { .. }
        | Type::Witness(_)
        | Type::Generic(_)
        | Type::TypeVar(_)
        | Type::Projection { .. }
        | Type::Constructor { .. }
        | Type::Apply { .. }
        | Type::Lambda { .. }
        | Type::BoundVar { .. }
        | Type::Error => true,
        // A raw pointee does not establish checked backing-owner storage.
        _ => false,
    }
}

fn native_mutation_candidates(
    function: &MirFunction,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    mutations: &NativeMutationTable,
    term: &Terminator,
) -> Vec<crate::hir::borrowing::BorrowRetention> {
    use crate::hir::borrowing::{BorrowRetention, BorrowSource, BorrowSourceKind};
    let Terminator::Call {
        func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))),
        args,
        destination,
        ..
    } = term
    else {
        return Vec::new();
    };
    let Some(header) = contract.callable(key) else {
        return Vec::new();
    };
    if &header.key != key
        || header.signature.params.len() != args.len()
        || !tc.type_id_tree_is_valid(header.signature.ret.semantic_ty)
        || !tc.type_id_tree_is_valid(header.signature.ret.abi_ty)
        || checked_place_storage_type(tc, contract, function, destination).as_ref()
            != Some(
                &contract.normalize_storage_type(
                    tc,
                    &tc.type_for(contract.callable_result_storage(header)),
                ),
            )
    {
        return Vec::new();
    }
    let actual = args
        .iter()
        .map(|operand| match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                checked_place_storage_type(tc, contract, function, place)
            }
            Operand::Constant(_) => None,
        })
        .collect::<Vec<_>>();
    if !header
        .signature
        .params
        .iter()
        .zip(&actual)
        .zip(args)
        .all(|((param, actual), operand)| {
            tc.type_id_tree_is_valid(param.semantic_ty)
                && match operand {
                    Operand::Copy(_) | Operand::Move(_) => actual.as_ref().is_some_and(|actual| {
                        *actual
                            == contract.normalize_storage_type(tc, &tc.type_for(param.semantic_ty))
                    }),
                    Operand::Constant(_) => true, // Constants supply no source rows or destination aliases.
                }
        })
    {
        return Vec::new();
    }
    if mutations.knowledge(key) == NativeMutationKnowledge::CompletedNoEffects {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for (target, ty) in actual.iter().enumerate() {
        let Some(Type::Reference {
            mutable: true,
            inner: storage,
        }) = ty
        else {
            continue;
        };
        for (source, ty) in actual.iter().enumerate() {
            if source == target {
                continue;
            }
            let Some(ty) = ty else {
                continue;
            };
            for (kind, payload) in std::iter::once((BorrowSourceKind::Value, ty)).chain(match ty {
                Type::Reference { inner, .. } => Some((BorrowSourceKind::Contents, inner.as_ref())),
                _ => None,
            }) {
                let may_borrow = tc.id_for_type(payload).is_none_or(|id| {
                    type_id_tracks_origin(TypeView::new(tc), contract, id)
                        || return_type_needs_escape_check(tc, contract, id)
                });
                if may_borrow
                    && mutation_storage_may_carry(
                        tc,
                        contract,
                        storage,
                        payload,
                        &mut HashSet::new(),
                    )
                {
                    rows.push(BorrowRetention {
                        destination: BorrowSource {
                            argument: target as u32,
                            path: Vec::new(),
                            kind: BorrowSourceKind::Contents,
                        },
                        sources: vec![BorrowSource {
                            argument: source as u32,
                            path: Vec::new(),
                            kind,
                        }],
                    });
                }
            }
        }
    }
    rows
}

#[derive(Clone)]
struct NativeMutationBinding {
    owner: Place,
    source: crate::hir::borrowing::BorrowSource,
    value: Place,
    excluded: HashSet<crate::ids::LoanId>,
}

fn native_mutation_bindings(
    function: &MirFunction,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    table: &LoanTable,
    state: &LoanState,
    term: &Terminator,
    rows: &[crate::hir::borrowing::BorrowRetention],
) -> Vec<NativeMutationBinding> {
    let Terminator::Call { args, .. } = term else {
        return Vec::new();
    };
    let mut bindings = Vec::new();
    let aliases = state.storage_alias_state();
    for row in rows {
        let Some(Operand::Copy(receiver) | Operand::Move(receiver)) =
            args.get(row.destination.argument as usize)
        else {
            continue;
        };
        let Some(Type::Reference {
            mutable: true,
            inner,
        }) = checked_place_storage_type(tc, contract, function, receiver)
        else {
            continue;
        };
        if !receiver.projection.is_empty() {
            continue;
        } // No invented projected-carrier loan.
        let reborrows = state
            .active_ids()
            .filter(|id| {
                state.storage_owner_contains(*id, receiver.local)
                    && table
                        .get(*id)
                        .is_some_and(|loan| loan.kind == LoanKind::Mut)
            })
            .collect::<HashSet<_>>();
        for id in &reborrows {
            // Inspect every alternative; raw/contents leases never bind destinations.
            for owner in provenance::resolve_loan_places(*id, table, &aliases) {
                if checked_place_storage_type(tc, contract, function, &owner).as_ref()
                    != Some(inner.as_ref())
                {
                    continue;
                }
                if owner
                    .projection
                    .iter()
                    .any(|projection| matches!(projection, Projection::Deref))
                {
                    continue;
                }
                for source in &row.sources {
                    let Some(Operand::Copy(value) | Operand::Move(value)) =
                        args.get(source.argument as usize)
                    else {
                        continue;
                    };
                    let values = if source.kind == crate::hir::borrowing::BorrowSourceKind::Contents
                    {
                        let Some(Type::Reference { inner, .. }) =
                            checked_place_storage_type(tc, contract, function, value)
                        else {
                            continue;
                        };
                        let mut contents = value.clone();
                        contents.projection.push(Projection::Deref);
                        provenance::resolve_places(&contents, table, &aliases)
                            .into_iter()
                            .filter(|place| {
                                checked_place_storage_type(tc, contract, function, place).as_ref()
                                    == Some(inner.as_ref())
                                    && (!place
                                        .projection
                                        .iter()
                                        .any(|p| matches!(p, Projection::Deref))
                                        || function.local_decls.get(place.local.0).is_some_and(
                                            |decl| decl.source == LocalSource::Argument,
                                        ))
                            })
                            .collect::<Vec<_>>()
                    } else {
                        vec![value.clone()]
                    };
                    for value in values {
                        bindings.push(NativeMutationBinding {
                            owner: owner.clone(),
                            source: source.clone(),
                            value,
                            excluded: reborrows.clone(),
                        });
                    }
                }
            }
        }
    }
    bindings
}

type NativeMutationOrigins = HashMap<usize, Vec<NativeMutationBinding>>;

/// Instantiate portable interface ordinals, never a runtime-selected native body.
pub(crate) fn erased_return_summary(
    contract: &MirBackendContract,
    tc: &crate::type_context::TypeContext,
    call: &crate::mir::MirErasedCall,
) -> Option<OriginSet> {
    use crate::hir::borrowing::BorrowSourceKind;
    let signature = call.target.signature(contract).ok()?;
    let borrowing =
        crate::mir::erased_borrow_contract(&signature.params, &signature.ret, None, tc).ok()?;
    let mut summary = HashSet::from([ReferenceOrigin::Static]);
    for source in borrowing.returns.iter().flat_map(|result| &result.sources) {
        let local = Local(source.argument as usize + 1);
        summary.insert(match source.kind {
            BorrowSourceKind::Value => ReferenceOrigin::Param(local),
            BorrowSourceKind::Contents => ReferenceOrigin::ParamContents(local),
        });
    }
    Some(summary)
}

struct ReferenceEscapeAnalysis<'a> {
    func: &'a MirFunction,
    type_context: &'a crate::type_context::TypeContext,
    backend_contract: &'a MirBackendContract,
    return_summaries: &'a HashMap<MirFunctionId, OriginSet>,
    missing_summary_mode: MissingSummaryMode,
    mutation_origins: Option<&'a NativeMutationOrigins>,
    return_sources: Option<&'a return_sources::NativeReturnSources>,
    purpose: OriginPurpose,
    // Producer measurement can retain dependencies even through scalar values;
    // ordinary lifetime/loan consumers remain restricted to origin carriers.
    track_value_dependencies: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OriginPurpose {
    Lifetime,
    ValueLoans,
}

/// Local native-body value-flow authority, not a portable/erased call contract.
struct NativeReturnLoanProof {
    summaries: HashMap<MirFunctionId, OriginSet>,
    measured: HashMap<MirFunctionId, OriginSet>,
    lifetime: HashMap<MirFunctionId, OriginSet>,
    sources: return_sources::NativeReturnSources,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingSummaryMode {
    Defer,
    Conservative,
}

impl<'a> ReferenceEscapeAnalysis<'a> {
    fn new(
        func: &'a MirFunction,
        type_context: &'a crate::type_context::TypeContext,
        backend_contract: &'a MirBackendContract,
        return_summaries: &'a HashMap<MirFunctionId, OriginSet>,
        missing_summary_mode: MissingSummaryMode,
    ) -> Self {
        Self {
            func,
            type_context,
            backend_contract,
            return_summaries,
            missing_summary_mode,
            mutation_origins: None,
            return_sources: None,
            purpose: OriginPurpose::Lifetime,
            track_value_dependencies: false,
        }
    }

    fn with_return_sources(
        mut self,
        sources: Option<&'a return_sources::NativeReturnSources>,
    ) -> Self {
        self.return_sources = sources;
        self
    }

    fn normalized_place(mut place: Place) -> Place {
        for projection in &mut place.projection {
            if let Projection::Field { identity, .. } = projection {
                *identity = None;
            }
        }
        place
    }

    fn field_place(dest: &Place, index: usize) -> Place {
        let mut place = dest.clone();
        place.projection.push(Projection::Field {
            index,
            identity: None,
        });
        Self::normalized_place(place)
    }

    fn local_decl(&self, local: Local) -> Option<&crate::mir::LocalDecl> {
        self.func.local_decls.get(local.0)
    }

    fn local_type_is_reference(&self, local: Local) -> bool {
        self.local_decl(local).is_some_and(|decl| {
            matches!(
                TypeView::new(self.type_context).ty(decl.ty),
                Ty::Reference { .. }
            )
        })
    }

    fn local_type_is_pointer(&self, local: Local) -> bool {
        self.local_decl(local).is_some_and(|decl| {
            matches!(TypeView::new(self.type_context).ty(decl.ty), Ty::Pointer(_))
        })
    }

    fn local_is_argument(&self, local: Local) -> bool {
        self.local_decl(local)
            .is_some_and(|decl| decl.source == LocalSource::Argument)
    }

    fn local_type_tracks_origin(&self, local: Local) -> bool {
        self.local_decl(local).is_some_and(|decl| {
            self.track_value_dependencies
                || type_id_tracks_origin(
                    TypeView::new(self.type_context),
                    self.backend_contract,
                    decl.ty,
                )
        })
    }

    fn local_type_contains_reference(&self, local: Local) -> bool {
        self.local_decl(local).is_some_and(|decl| {
            type_id_contains_reference(
                TypeView::new(self.type_context),
                self.backend_contract,
                decl.ty,
            )
        })
    }

    fn local_origin(&self, local: Local) -> Option<ReferenceOrigin> {
        self.local_decl(local).map(|decl| match decl.source {
            LocalSource::Argument => {
                // Contained raw handles carry incoming payload origins too.
                // This is value transport, not a borrow of the argument's own
                // storage (which origins_for_borrowed_place keeps local).
                if self.local_type_tracks_origin(local) {
                    ReferenceOrigin::Param(local)
                } else {
                    ReferenceOrigin::Local(local)
                }
            }
            LocalSource::ClosureCapture => ReferenceOrigin::Param(local),
            LocalSource::ReturnPlace | LocalSource::UserBinding => ReferenceOrigin::Local(local),
            LocalSource::Temporary => ReferenceOrigin::Temporary(local),
        })
    }

    fn origins_for_place(&self, state: &ReferenceOriginState, place: &Place) -> OriginSet {
        let normalized = Self::normalized_place(place.clone());
        let mut child_origins = self.child_origins_for_place(state, &normalized);
        if let Some(origins) = state.origins.get(&normalized) {
            // Projected writes can add dependencies after a root was forwarded.
            child_origins.extend(origins.iter().copied());
            return child_origins;
        }

        if !child_origins.is_empty() {
            return child_origins;
        }

        if matches!(normalized.projection.first(), Some(Projection::Deref)) {
            let base = Place {
                local: normalized.local,
                projection: Vec::new(),
            };
            if let Some(origins) = state.origins.get(&base) {
                if self.local_type_is_reference(normalized.local) {
                    // Loading a value through &Aggregate transports its
                    // contents; taking its address uses the separate borrow path.
                    return origins
                        .iter()
                        .map(|origin| origin.loaded_contents())
                        .collect();
                }
                return origins.clone();
            }

            if self.local_type_is_pointer(normalized.local) {
                if !self.local_is_argument(normalized.local) {
                    return OriginSet::new();
                }
                return self
                    .local_origin(normalized.local)
                    .map(|origin| HashSet::from([origin]))
                    .unwrap_or_else(|| HashSet::from([ReferenceOrigin::UnknownExternal]));
            }

            if let Some(origin) = self.local_origin(normalized.local) {
                return HashSet::from([origin]);
            }
        }

        if !normalized.projection.is_empty() {
            let base = Place {
                local: normalized.local,
                projection: Vec::new(),
            };
            if let Some(origins) = state.origins.get(&base) {
                return origins.clone();
            }

            if self.local_is_argument(normalized.local)
                && self.local_type_tracks_origin(normalized.local)
            {
                return self
                    .local_origin(normalized.local)
                    .map(|origin| HashSet::from([origin]))
                    .unwrap_or_default();
            }

            if self.local_type_is_reference(normalized.local)
                || self.local_type_is_pointer(normalized.local)
            {
                return if self.local_is_argument(normalized.local) {
                    self.local_origin(normalized.local)
                        .map(|origin| HashSet::from([origin]))
                        .unwrap_or_default()
                } else {
                    OriginSet::new()
                };
            }

            return OriginSet::new();
        }

        if normalized.projection.is_empty()
            && self.local_type_tracks_origin(normalized.local)
            && !self.local_type_is_reference(normalized.local)
            && !self.local_type_is_pointer(normalized.local)
        {
            return match self.local_origin(normalized.local) {
                Some(ReferenceOrigin::Param(local)) => {
                    HashSet::from([ReferenceOrigin::Param(local)])
                }
                _ => OriginSet::new(),
            };
        }

        if normalized.projection.is_empty()
            && (self.local_type_is_reference(normalized.local)
                || self.local_type_is_pointer(normalized.local))
            && !self.local_is_argument(normalized.local)
        {
            return OriginSet::new();
        }

        if normalized.projection.is_empty() && !self.local_type_tracks_origin(normalized.local) {
            return OriginSet::new();
        }

        self.local_origin(normalized.local)
            .map(|origin| HashSet::from([origin]))
            .unwrap_or_default()
    }

    fn origins_for_borrowed_place(&self, state: &ReferenceOriginState, place: &Place) -> OriginSet {
        let normalized = Self::normalized_place(place.clone());
        let mut prefix = Place {
            local: normalized.local,
            projection: Vec::new(),
        };
        let mut raw_memory_view = false;
        let mut indexed_storage_view = false;
        let mut raw_indexed_view = false;
        for projection in &normalized.projection {
            let prefix_type =
                self.backend_contract
                    .place_type_id(self.type_context, self.func, &prefix);
            let pointer = prefix_type
                .is_some_and(|ty| type_id_is_pointer(TypeView::new(self.type_context), ty));
            raw_memory_view |= *projection == Projection::Deref && pointer;
            if matches!(projection, Projection::Index(_)) {
                raw_indexed_view |= pointer;
                indexed_storage_view |= pointer
                    || prefix_type.is_some_and(|ty| {
                        matches!(self.type_context.try_ty(ty), Some(Ty::Reference { .. }))
                    });
            }
            prefix.projection.push(projection.clone());
        }
        if !indexed_storage_view
            && !normalized
                .projection
                .iter()
                .any(|projection| matches!(projection, Projection::Deref))
        {
            // Pointer/reference indexing addresses incoming storage without an
            // explicit Deref. Without either view, this is the local slot itself.
            return HashSet::from([
                match self.local_decl(normalized.local).map(|decl| &decl.source) {
                    Some(LocalSource::Temporary) => ReferenceOrigin::Temporary(normalized.local),
                    _ => ReferenceOrigin::Local(normalized.local),
                },
            ]);
        }

        let origins = self
            .origins_for_place(state, place)
            .into_iter()
            .map(|origin| origin.borrowed_contents(raw_memory_view, raw_indexed_view))
            .collect::<OriginSet>();
        if !origins.is_empty() {
            return origins;
        }

        if (self.local_type_is_reference(normalized.local)
            || self.local_type_is_pointer(normalized.local))
            && !self.local_is_argument(normalized.local)
        {
            return OriginSet::new();
        }

        self.local_origin(normalized.local)
            .map(|origin| HashSet::from([origin]))
            .unwrap_or_default()
    }

    fn child_origins_for_place(&self, state: &ReferenceOriginState, place: &Place) -> OriginSet {
        let mut origins = OriginSet::new();
        for (tracked_place, tracked_origins) in &state.origins {
            if tracked_place != place && Self::place_is_prefix(place, tracked_place) {
                origins.extend(tracked_origins.iter().copied());
            }
        }
        origins
    }

    fn borrowed_contents_origins(origins: OriginSet) -> OriginSet {
        origins
            .into_iter()
            .map(|origin| match origin {
                ReferenceOrigin::Param(local) | ReferenceOrigin::ParamContents(local) => {
                    ReferenceOrigin::ParamBorrowedContents(local)
                }
                origin => origin,
            })
            .collect()
    }

    fn origins_for_operand(&self, state: &ReferenceOriginState, operand: &Operand) -> OriginSet {
        match operand {
            Operand::Copy(place) | Operand::Move(place)
                if matches!(place.projection.first(), Some(Projection::Deref))
                    && self.local_type_is_pointer(place.local) =>
            {
                // A value read through a raw address transports the pointee's
                // contents. Address-taking keeps the separate storage envelope.
                let mut contents = OriginSet::new();
                for origin in self.origins_for_place(state, place) {
                    match origin {
                        ReferenceOrigin::Local(local) | ReferenceOrigin::Temporary(local) => {
                            contents.extend(self.origins_for_place(
                                state,
                                &Place {
                                    local,
                                    projection: Vec::new(),
                                },
                            ));
                        }
                        ReferenceOrigin::Param(local) => {
                            contents.insert(ReferenceOrigin::ParamContents(local));
                        }
                        origin => {
                            contents.insert(origin);
                        }
                    }
                }
                contents
            }
            Operand::Copy(place) | Operand::Move(place) => self.origins_for_place(state, place),
            Operand::Constant(Constant::String(_)) => HashSet::from([ReferenceOrigin::Static]),
            Operand::Constant(_) => OriginSet::new(),
        }
    }

    fn assign_origins(&self, state: &mut ReferenceOriginState, dest: &Place, origins: OriginSet) {
        let dest = Self::normalized_place(dest.clone());
        state
            .origins
            .retain(|place, _| !Self::place_is_prefix(&dest, place));
        if dest.projection.is_empty() && !self.local_type_tracks_origin(dest.local) {
            return;
        }
        if !origins.is_empty() {
            state.origins.insert(dest, origins);
        }
    }

    fn forward_structured_origins(
        &self,
        state: &mut ReferenceOriginState,
        dest: &Place,
        operand: &Operand,
    ) -> bool {
        let (Operand::Copy(source) | Operand::Move(source)) = operand else {
            return false;
        };
        let Some(ty) =
            checked_place_storage_type(self.type_context, self.backend_contract, self.func, source)
        else {
            return false;
        };
        if !matches!(
            ty,
            Type::Struct { .. }
                | Type::StructRepresentation { .. }
                | Type::Enum { .. }
                | Type::EnumRepresentation { .. }
                | Type::Tuple(_)
                | Type::Array(_, _)
                | Type::Slice(_)
        ) || checked_place_storage_type(
            self.type_context,
            self.backend_contract,
            self.func,
            dest,
        ) != Some(ty)
        {
            return false;
        }
        let envelope = self.origins_for_operand(state, operand);
        let source = Self::normalized_place(source.clone());
        let dest = Self::normalized_place(dest.clone());
        // Snapshot before clearing, including self/overlapping forwarding.
        // Rebase actual child paths, not their union onto an untyped root.
        let rows = state
            .origins
            .iter()
            .filter(|(place, origins)| !origins.is_empty() && Self::place_is_prefix(&source, place))
            .map(|(place, origins)| {
                let mut target = dest.clone();
                target.projection.extend(
                    place
                        .projection
                        .iter()
                        .skip(source.projection.len())
                        .cloned(),
                );
                let ty = checked_place_storage_type(
                    self.type_context,
                    self.backend_contract,
                    self.func,
                    place,
                )?;
                (checked_place_storage_type(
                    self.type_context,
                    self.backend_contract,
                    self.func,
                    &target,
                ) == Some(ty))
                .then(|| (target, origins.clone()))
            })
            .collect::<Option<Vec<_>>>();
        let Some(rows) = rows.filter(|rows| !rows.is_empty()) else {
            return false;
        };
        self.assign_origins(state, &dest, OriginSet::new());
        state.origins.extend(rows);
        // Keep the whole-value envelope as well as its checked child paths.
        state.origins.entry(dest).or_default().extend(envelope);
        true
    }

    fn apply_mutation_origins(
        &self,
        state: &mut ReferenceOriginState,
        bindings: &[NativeMutationBinding],
    ) {
        let inputs = bindings
            .iter()
            .map(|binding| {
                let origins = self.origins_for_place(state, &binding.value);
                (Self::normalized_place(binding.owner.clone()), origins)
            })
            .collect::<Vec<_>>();
        for (owner, origins) in inputs {
            if origins.is_empty() {
                continue;
            }
            // A local-granularity may write must preserve all existing contents
            // and remain visible to a later whole-owner move, including fields.
            let root = Place {
                local: owner.local,
                projection: Vec::new(),
            };
            let previous = state
                .origins
                .iter()
                .filter(|(place, _)| place.local == owner.local)
                .flat_map(|(_, origins)| origins.iter().copied())
                .collect::<OriginSet>();
            state
                .origins
                .entry(root)
                .or_default()
                .extend(previous.into_iter().chain(origins.iter().copied()));
            state.origins.entry(owner).or_default().extend(origins);
        }
    }

    fn destination_can_outlive_statement(&self, dest: &Place) -> bool {
        self.local_decl(dest.local).is_some_and(|decl| {
            matches!(
                decl.source,
                LocalSource::Argument
                    | LocalSource::ClosureCapture
                    | LocalSource::ReturnPlace
                    | LocalSource::UserBinding
            )
        })
    }

    fn temporary_origins_for_assignment(
        &self,
        state: &ReferenceOriginState,
        dest: &Place,
        rvalue: &Rvalue,
    ) -> HashSet<Local> {
        if !self.destination_can_outlive_statement(dest) {
            return HashSet::new();
        }

        // lean-ctx: numeric diagnostic-only ceiling; widen after typed-storage controls.
        // Keep raw leases, callable environments and unknown storage conservative.
        if self
            .backend_contract
            .place_type_id(self.type_context, self.func, dest)
            .is_some_and(|ty| {
                self.type_context.type_id_tree_is_valid(ty)
                    && self.type_context.type_for(ty).is_numeric()
                    && !return_type_needs_escape_check(self.type_context, self.backend_contract, ty)
            })
        {
            return HashSet::new();
        }

        let mut temporaries = HashSet::new();
        match rvalue {
            Rvalue::Ref(_, place) => {
                Self::collect_temporary_origins(
                    self.origins_for_borrowed_place(state, place),
                    &mut temporaries,
                );
            }
            Rvalue::Package(operand, _)
            | Rvalue::Object(operand, _)
            | Rvalue::Use(operand)
            | Rvalue::Cast(operand, _) => {
                Self::collect_temporary_origins(
                    self.origins_for_operand(state, operand),
                    &mut temporaries,
                );
            }
            Rvalue::Aggregate(_, operands) => {
                for operand in operands {
                    Self::collect_temporary_origins(
                        self.origins_for_operand(state, operand),
                        &mut temporaries,
                    );
                }
            }
            Rvalue::Closure(closure) => {
                for capture in &closure.captures {
                    Self::collect_temporary_origins(
                        self.origins_for_place(state, &capture.place()),
                        &mut temporaries,
                    );
                }
            }
            Rvalue::BinaryOp(_, _, _) | Rvalue::UnaryOp(_, _) | Rvalue::Discriminant(_) => {}
        }
        temporaries
    }

    fn temporary_origins_for_call(
        &self,
        state: &ReferenceOriginState,
        func: &Operand,
        args: &[Operand],
        destination: &Place,
    ) -> HashSet<Local> {
        if !self.destination_can_outlive_statement(destination)
            || !self.local_type_contains_reference(destination.local)
        {
            return HashSet::new();
        }

        let mut temporaries = HashSet::new();
        Self::collect_temporary_origins(
            self.call_return_origins(state, func, args, destination),
            &mut temporaries,
        );
        temporaries
    }

    fn collect_temporary_origins(origins: OriginSet, temporaries: &mut HashSet<Local>) {
        for origin in origins {
            if let ReferenceOrigin::Temporary(local) = origin {
                temporaries.insert(local);
            }
        }
    }

    fn place_is_prefix(prefix: &Place, place: &Place) -> bool {
        prefix.local == place.local
            && prefix.projection.len() <= place.projection.len()
            && prefix
                .projection
                .iter()
                .zip(place.projection.iter())
                .all(|(left, right)| left == right)
    }

    fn callable_function_id(callable: &MirCallable) -> Option<MirFunctionId> {
        callable.function_id()
    }

    fn call_return_origins(
        &self,
        state: &ReferenceOriginState,
        func: &Operand,
        args: &[Operand],
        destination: &Place,
    ) -> OriginSet {
        if !destination.projection.is_empty() || !self.local_type_tracks_origin(destination.local) {
            return OriginSet::new();
        }

        if let Operand::Constant(Constant::ErasedCall(call)) = func {
            return erased_return_summary(self.backend_contract, self.type_context, call)
                .map(|summary| self.instantiate_return_origins(state, args, destination, &summary))
                .unwrap_or_else(|| self.opaque_call_return_origins(state, args));
        }

        let Operand::Constant(Constant::Callable(callable)) = func else {
            if let Some(callable) = self
                .return_sources
                .and_then(|table| table.witnessed_callable(self.func, func, destination))
            {
                // The exact checked allocation/body supplies the target, not a
                // result/effect certificate. Keep its ordinary origin summary
                // (including local/unknown dependencies) and executable MIR.
                let target = Operand::Constant(Constant::Callable(callable.clone()));
                if return_sources::checked_call(
                    self.func,
                    self.type_context,
                    self.backend_contract,
                    &target,
                    args,
                    destination,
                )
                .is_some()
                {
                    return self.call_return_origins(state, &target, args, destination);
                }
            }
            if let Some((inputs, summary)) = self.return_sources.and_then(|table| {
                table.callable_result_for_purpose(
                    self.func,
                    self.type_context,
                    self.backend_contract,
                    func,
                    args,
                    destination,
                    self.purpose,
                )
            }) {
                return self.instantiate_return_origins(state, &inputs, destination, &summary);
            }
            let mut origins = self.opaque_call_return_origins(state, args);
            // Unknown callable implementations may transport their environment,
            // including a borrowed receiver's storage. Absence is not emptiness.
            origins.extend(self.origins_for_operand(state, func));
            return origins;
        };

        match callable {
            MirCallable::Resolved(key @ MirCallableKey::Intrinsic(intrinsic))
                if self.track_value_dependencies && intrinsic.is_scalar_operation() =>
            {
                // Measure the operands of an explicit instruction, not an
                // ordinary/foreign function with a similarly shaped result.
                let valid = self.backend_contract.callable(key).is_some_and(|header| {
                    matches!(&header.kind, crate::mir::MirCallableKind::Intrinsic(id) if id == intrinsic)
                        && header.signature.params.len() == args.len()
                        && self.backend_contract.place_type_id(self.type_context, self.func, destination)
                            == Some(header.signature.ret.semantic_ty)
                });
                return if valid {
                    args.iter()
                        .flat_map(|argument| self.origins_for_operand(state, argument))
                        .collect()
                } else {
                    self.opaque_call_return_origins(state, args)
                };
            }
            MirCallable::Resolved(MirCallableKey::Intrinsic(intrinsic))
                if matches!(
                    intrinsic,
                    MirIntrinsicId::BorrowSlice | MirIntrinsicId::BorrowSliceMut
                ) =>
            {
                return args
                    .first()
                    .map(|arg| {
                        Self::borrowed_contents_origins(self.origins_for_operand(state, arg))
                    })
                    .unwrap_or_else(|| HashSet::from([ReferenceOrigin::UnknownExternal]));
            }
            MirCallable::Resolved(
                MirCallableKey::Extern(_)
                | MirCallableKey::Intrinsic(_)
                | MirCallableKey::RuntimeHelper(_),
            ) => {
                return self.opaque_call_return_origins(state, args);
            }
            _ => {}
        }

        let Some(function_id) = Self::callable_function_id(callable) else {
            return HashSet::from([ReferenceOrigin::Local(destination.local)]);
        };
        let summary = self.return_summaries.get(&function_id).or_else(|| {
            self.track_value_dependencies
                .then(|| {
                    self.return_sources
                        .and_then(|table| table.scalar_dependencies(self.purpose, &function_id))
                })
                .flatten()
        });
        let Some(summary) = summary else {
            return match self.missing_summary_mode {
                // Local bodies are seeded in the fixpoint map. A missing entry
                // is an opaque/bodyless callable, not a recursive pending body.
                MissingSummaryMode::Defer => self.opaque_call_return_origins(state, args),
                MissingSummaryMode::Conservative
                    if self.local_type_tracks_origin(destination.local) =>
                {
                    self.opaque_call_return_origins(state, args)
                }
                MissingSummaryMode::Conservative => {
                    HashSet::from([ReferenceOrigin::Local(destination.local)])
                }
            };
        };

        if let Some(sources) = self.return_sources.and_then(|table| {
            table.borrowed_view_call(
                self.func,
                self.type_context,
                self.backend_contract,
                func,
                args,
                destination,
            )
        }) {
            let remainder = summary
                .iter()
                .copied()
                .filter(|origin| match origin {
                    ReferenceOrigin::Param(local)
                    | ReferenceOrigin::ParamContents(local)
                    | ReferenceOrigin::ParamBorrowedContents(local) => {
                        !sources.iter().any(|source| source.parameter() == *local)
                    }
                    _ => true, // Preserve unknown, local, temporary and root-history envelopes.
                })
                .collect();
            let mut origins = self.instantiate_return_origins(state, args, destination, &remainder);
            for source in sources {
                let return_sources::Source::Slice {
                    parameter,
                    load,
                    reference,
                } = source
                else {
                    continue;
                };
                let Some(arg) = parameter.0.checked_sub(1).and_then(|index| args.get(index)) else {
                    return self.instantiate_return_origins(state, args, destination, summary);
                };
                for origin in self.origins_for_operand(state, arg) {
                    if load.is_empty() {
                        origins.insert(origin);
                        continue;
                    }
                    match origin {
                        ReferenceOrigin::Local(local) | ReferenceOrigin::Temporary(local)
                            if self
                                .local_decl(local)
                                .is_some_and(|decl| decl.ty == *reference) =>
                        {
                            // Exactly one witnessed safe slice-reference load:
                            // retain its whole value/history, not the slot address.
                            let contents = self.origins_for_place(
                                state,
                                &Place {
                                    local,
                                    projection: Vec::new(),
                                },
                            );
                            if contents.is_empty() {
                                origins.insert(origin);
                            } else {
                                origins.extend(contents);
                            }
                        }
                        ReferenceOrigin::Param(local)
                            if self.local_decl(local).is_some_and(|decl| {
                                self.type_context.try_ty(decl.ty)
                                    == Some(&Ty::Reference {
                                        mutable: false,
                                        inner: *reference,
                                    })
                            }) =>
                        {
                            origins.insert(ReferenceOrigin::ParamContents(local));
                        }
                        origin => {
                            origins.insert(origin);
                        }
                    }
                }
            }
            return origins;
        }
        self.instantiate_return_origins(state, args, destination, summary)
    }

    fn instantiate_return_origins(
        &self,
        state: &ReferenceOriginState,
        args: &[Operand],
        destination: &Place,
        summary: &OriginSet,
    ) -> OriginSet {
        let mut origins = OriginSet::new();
        for origin in summary {
            match origin {
                ReferenceOrigin::Param(local) => {
                    if let Some(arg_index) = local.0.checked_sub(1) {
                        if let Some(arg) = args.get(arg_index) {
                            origins.extend(self.origins_for_operand(state, arg));
                        } else {
                            origins.insert(ReferenceOrigin::Local(destination.local));
                        }
                    } else {
                        origins.insert(ReferenceOrigin::Local(destination.local));
                    }
                }
                ReferenceOrigin::ParamContents(local) => {
                    if let Some(arg) = local.0.checked_sub(1).and_then(|index| args.get(index)) {
                        for origin in self.origins_for_operand(state, arg) {
                            match origin {
                                ReferenceOrigin::Local(local)
                                | ReferenceOrigin::Temporary(local) => {
                                    origins.extend(self.origins_for_place(
                                        state,
                                        &Place {
                                            local,
                                            projection: Vec::new(),
                                        },
                                    ));
                                }
                                ReferenceOrigin::Param(local)
                                    if !self.local_type_is_reference(local)
                                        && !self.local_type_is_pointer(local) =>
                                {
                                    // Owned aggregates already carry the origins
                                    // of their fields. A callable projected from
                                    // one is not an address to argument storage.
                                    origins.insert(ReferenceOrigin::Param(local));
                                }
                                ReferenceOrigin::Param(local)
                                | ReferenceOrigin::ParamContents(local) => {
                                    origins.insert(ReferenceOrigin::ParamContents(local));
                                }
                                origin => {
                                    origins.insert(origin);
                                }
                            }
                        }
                    } else {
                        origins.insert(ReferenceOrigin::Local(destination.local));
                    }
                }
                ReferenceOrigin::ParamBorrowedContents(local) => {
                    if let Some(arg) = local.0.checked_sub(1).and_then(|index| args.get(index)) {
                        // Unlike copied contents, a memory view still depends on
                        // the actual owner storage. Empty pointer provenance stays
                        // empty; the summary cannot manufacture an owner lease.
                        for origin in
                            Self::borrowed_contents_origins(self.origins_for_operand(state, arg))
                        {
                            origins.insert(origin);
                            if let ReferenceOrigin::Local(local)
                            | ReferenceOrigin::Temporary(local) = origin
                            {
                                origins.extend(self.origins_for_place(
                                    state,
                                    &Place {
                                        local,
                                        projection: Vec::new(),
                                    },
                                ));
                            }
                        }
                    } else {
                        origins.insert(ReferenceOrigin::Local(destination.local));
                    }
                }
                ReferenceOrigin::Static => {
                    origins.insert(ReferenceOrigin::Static);
                }
                ReferenceOrigin::UnknownExternal => {
                    origins.insert(ReferenceOrigin::UnknownExternal);
                }
                ReferenceOrigin::Local(_) => {
                    origins.insert(ReferenceOrigin::Local(destination.local));
                }
                ReferenceOrigin::Temporary(_) => {
                    origins.insert(ReferenceOrigin::Temporary(destination.local));
                }
            }
        }
        origins
    }

    fn opaque_call_return_origins(
        &self,
        state: &ReferenceOriginState,
        args: &[Operand],
    ) -> OriginSet {
        let mut origins = HashSet::from([ReferenceOrigin::UnknownExternal]);
        for arg in args {
            origins.extend(self.origins_for_operand(state, arg));
        }
        origins
    }

    fn measured_return_origins(&self) -> Option<OriginSet> {
        let results = run_fixpoint(self, self.func);
        let return_place = Place {
            local: Local(0),
            projection: Vec::new(),
        };
        let mut origins = OriginSet::new();
        let mut returns = false;
        for (index, block) in self.func.basic_blocks.iter().enumerate() {
            if !results.reachable[index]
                || !matches!(
                    block.terminator,
                    Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
                )
            {
                continue;
            }
            returns = true;
            for (place, dependencies) in &results.exit_sets[index].origins {
                if Self::place_is_prefix(&return_place, place) {
                    origins.extend(dependencies.iter().copied());
                }
            }
        }
        returns.then_some(origins)
    }
}

impl Analysis for ReferenceEscapeAnalysis<'_> {
    type Domain = ReferenceOriginState;

    fn initial_state(&self, _func: &MirFunction) -> Self::Domain {
        let mut state = ReferenceOriginState::default();
        for (local, origin) in &self.func.ownership.reference_origins {
            if *local == Local(0) && !self.local_type_is_reference(*local) {
                continue;
            }
            let place = Place {
                local: *local,
                projection: Vec::new(),
            };
            state.origins.insert(place, HashSet::from([*origin]));
        }
        for (index, decl) in self.func.local_decls.iter().enumerate() {
            let local = Local(index);
            if matches!(
                decl.source,
                LocalSource::Argument | LocalSource::ClosureCapture
            ) && (self.local_type_is_reference(local) || self.track_value_dependencies)
            {
                state.origins.insert(
                    Place {
                        local,
                        projection: Vec::new(),
                    },
                    HashSet::from([ReferenceOrigin::Param(local)]),
                );
            }
        }
        state
    }

    fn bottom_state(&self, _func: &MirFunction) -> Self::Domain {
        ReferenceOriginState::default()
    }

    fn apply_statement(&self, state: &mut Self::Domain, stmt: &StatementData) {
        let StatementKind::Assign(dest, rvalue) = &stmt.kind else {
            return;
        };
        #[cfg(test)]
        let fresh_transfer = allocator_trace_tests::fresh_transfer_before(self, state, stmt);

        match rvalue {
            Rvalue::Ref(_, place) => {
                let origins = self.origins_for_borrowed_place(state, place);
                self.assign_origins(state, dest, origins);
            }
            Rvalue::Package(operand, _)
            | Rvalue::Object(operand, _)
            | Rvalue::Use(operand)
            | Rvalue::Cast(operand, _) => {
                let origins = self.origins_for_operand(state, operand);
                let forwarding = callable_forwarding_rvalue(
                    self.func,
                    self.type_context,
                    self.backend_contract,
                    dest,
                    rvalue,
                );
                if matches!(rvalue, Rvalue::Use(_))
                    && self.forward_structured_origins(state, dest, operand)
                {
                    // Existing canonical storage paths were transported intact.
                } else if forwarding && !origins.is_empty() {
                    let dest = Self::normalized_place(dest.clone());
                    state
                        .origins
                        .retain(|place, _| !Self::place_is_prefix(&dest, place));
                    state.origins.insert(dest, origins);
                } else {
                    self.assign_origins(state, dest, origins);
                }
            }
            Rvalue::Aggregate(kind, operands) => {
                self.assign_origins(state, dest, OriginSet::new());
                let mut payload = dest.clone();
                if let crate::mir::AggregateKind::EnumVariant { variant_id, .. } = kind {
                    // Match payload reads and checked typing retain this variant.
                    payload.projection.push(Projection::Downcast(*variant_id));
                }
                for (index, operand) in operands.iter().enumerate() {
                    if let Some(field) = checked_aggregate_field(
                        self.func,
                        self.type_context,
                        self.backend_contract,
                        dest,
                        kind,
                        index,
                    ) {
                        if self.forward_structured_origins(state, &field, operand) {
                            continue;
                        }
                    }
                    let origins = self.origins_for_operand(state, operand);
                    if !origins.is_empty() {
                        let field_place = Self::field_place(&payload, index);
                        state.origins.insert(field_place, origins);
                    }
                }
            }
            Rvalue::Closure(closure) => {
                let origins = closure
                    .captures
                    .iter()
                    .flat_map(|capture| {
                        let place = Place {
                            local: capture.local,
                            projection: Vec::new(),
                        };
                        match capture.kind {
                            crate::mir::MirClosureCaptureKind::ByValue => {
                                self.origins_for_place(state, &place)
                            }
                            crate::mir::MirClosureCaptureKind::ByRef
                            | crate::mir::MirClosureCaptureKind::ByMutRef => {
                                self.origins_for_borrowed_place(state, &place)
                            }
                        }
                    })
                    .collect();
                self.assign_origins(state, dest, origins);
            }
            Rvalue::BinaryOp(_, left, right) if self.track_value_dependencies => {
                let mut origins = self.origins_for_operand(state, left);
                origins.extend(self.origins_for_operand(state, right));
                self.assign_origins(state, dest, origins);
            }
            Rvalue::UnaryOp(_, operand) if self.track_value_dependencies => {
                self.assign_origins(state, dest, self.origins_for_operand(state, operand));
            }
            Rvalue::Discriminant(place) if self.track_value_dependencies => {
                self.assign_origins(state, dest, self.origins_for_place(state, place));
            }
            Rvalue::BinaryOp(_, _, _) | Rvalue::UnaryOp(_, _) | Rvalue::Discriminant(_) => {
                self.assign_origins(state, dest, OriginSet::new());
            }
        }
        #[cfg(test)]
        allocator_trace_tests::fresh_transfer_after(self, state, fresh_transfer);
    }

    fn apply_terminator(&self, state: &mut Self::Domain, term: &Terminator) {
        if let Terminator::Call {
            func,
            args,
            destination,
            ..
        } = term
        {
            let origins = self.call_return_origins(state, func, args, destination);
            #[cfg(test)]
            allocator_trace_tests::call_origins(self, state, term, &origins);
            self.assign_origins(state, destination, origins);
        }
    }

    fn apply_block(
        &self,
        state: &mut Self::Domain,
        func: &MirFunction,
        block: crate::mir::BasicBlockId,
    ) {
        let data = &func.basic_blocks[block.0];
        for statement in &data.statements {
            self.apply_statement(state, statement);
        }
        if let Some(term) = &data.terminator {
            if let (Some(bindings), Terminator::Call { .. }) = (
                self.mutation_origins
                    .and_then(|origins| origins.get(&block.0)),
                term,
            ) {
                self.apply_mutation_origins(state, bindings);
            }
            self.apply_terminator(state, term);
        }
    }
}

fn callable_forwarding_types(
    function: &MirFunction,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    destination: &Place,
    source: &Operand,
) -> bool {
    let (Operand::Copy(source) | Operand::Move(source)) = source else {
        return false;
    };
    match (
        checked_place_storage_type(tc, contract, function, destination),
        checked_place_storage_type(tc, contract, function, source),
    ) {
        (
            Some(Type::Function {
                params: a,
                ret: b,
                safety: c,
                callable_kind: d,
                ..
            }),
            Some(Type::Function {
                params: e,
                ret: f,
                safety: g,
                callable_kind: h,
                ..
            }),
        ) => a == e && b == f && c == g && h <= d,
        _ => false,
    }
}

fn checked_aggregate_field(
    function: &MirFunction,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    destination: &Place,
    kind: &crate::mir::AggregateKind,
    index: usize,
) -> Option<Place> {
    let mut payload = destination.clone();
    match (
        kind,
        checked_place_storage_type(tc, contract, function, destination)?,
    ) {
        (
            crate::mir::AggregateKind::Struct { id, .. },
            Type::Struct { id: owner, .. } | Type::StructRepresentation { id: owner, .. },
        ) if *id == owner => {}
        (
            crate::mir::AggregateKind::EnumVariant {
                enum_id,
                variant_id,
                ..
            },
            Type::Enum { id, .. } | Type::EnumRepresentation { id, .. },
        ) if *enum_id == id => payload.projection.push(Projection::Downcast(*variant_id)),
        (crate::mir::AggregateKind::Tuple, Type::Tuple(_)) => {}
        _ => return None,
    }
    let field = ReferenceEscapeAnalysis::field_place(&payload, index);
    checked_place_storage_type(tc, contract, function, &field)?;
    Some(field)
}

fn aggregate_operand_forwarding(
    function: &MirFunction,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    destination: &Place,
    kind: &crate::mir::AggregateKind,
    index: usize,
    operand: &Operand,
) -> bool {
    let (Operand::Copy(source) | Operand::Move(source)) = operand else {
        return false;
    };
    let Some(field) = checked_aggregate_field(function, tc, contract, destination, kind, index)
    else {
        return false;
    };
    checked_place_storage_type(tc, contract, function, &field)
        == checked_place_storage_type(tc, contract, function, source)
        || callable_forwarding_types(function, tc, contract, &field, operand)
}

fn callable_forwarding_rvalue(
    function: &MirFunction,
    tc: &crate::type_context::TypeContext,
    contract: &MirBackendContract,
    destination: &Place,
    rvalue: &Rvalue,
) -> bool {
    let operand = match rvalue {
        Rvalue::Use(operand) => operand,
        Rvalue::Cast(operand, target)
            if tc.type_id_tree_is_valid(*target)
                && checked_place_storage_type(tc, contract, function, destination)
                    == Some(contract.normalize_storage_type(tc, &tc.type_for(*target))) =>
        {
            operand
        }
        _ => return false,
    };
    callable_forwarding_types(function, tc, contract, destination, operand)
}

impl<'a> MoveValidationContext<'a> {
    fn new(
        type_context: &'a crate::type_context::TypeContext,
        backend_contract: &'a MirBackendContract,
    ) -> Self {
        Self {
            type_context,
            backend_contract,
            direct_drop_types: backend_contract.drop_glue.keys().copied().collect(),
            cleanup_cache: HashMap::new(),
        }
    }

    fn validate_move(
        &mut self,
        place: &Place,
        func: &MirFunction,
        span: Option<Span>,
        diagnostics: &mut Diagnostics,
    ) {
        self.validate_place(place, func, span, diagnostics, true);
    }

    fn validate_copy(
        &mut self,
        place: &Place,
        func: &MirFunction,
        span: Option<Span>,
        diagnostics: &mut Diagnostics,
    ) {
        self.validate_place(place, func, span, diagnostics, false);
    }

    fn validate_place(
        &mut self,
        place: &Place,
        func: &MirFunction,
        span: Option<Span>,
        diagnostics: &mut Diagnostics,
        validate_move_only_rules: bool,
    ) {
        let Some(local) = func.local_decls.get(place.local.0) else {
            return;
        };
        let mut current_ty = self
            .backend_contract
            .normalize_storage_type(self.type_context, &self.type_context.type_for(local.ty));
        let mut reference_owned_path = false;

        for projection in &place.projection {
            current_ty = self
                .backend_contract
                .normalize_storage_type(self.type_context, &current_ty);
            match projection {
                Projection::Deref => match &current_ty {
                    Type::Reference { .. } => reference_owned_path = true,
                    Type::Pointer(_) => reference_owned_path = false,
                    _ => {}
                },
                Projection::Field { .. }
                    if validate_move_only_rules && self.has_direct_drop(&current_ty) =>
                {
                    #[cfg(test)]
                    if allocator_trace_tests::enabled_at(span.as_ref()) {
                        eprintln!(
                            "OWNER_LOAN_TRACE actual-drop-field-rejection function={:?} span={:?} move_place={:?} projection={:?} normalized_owner_type={:?} type_id={:?} direct_drop={:?}",
                            func.id, span, place, projection, current_ty,
                            self.type_context.id_for_type(&current_ty),
                            self.type_context.id_for_type(&current_ty)
                                .and_then(|id| self.backend_contract.drop_glue.get(&id)),
                        );
                    }
                    diagnostics.push(mir_diagnostic(
                        format!(
                            "Cannot move field out of type '{}' because it implements Drop",
                            self.type_display_name(&current_ty)
                        ),
                        span.clone(),
                    ));
                    return;
                }
                Projection::Index(_) if validate_move_only_rules => {
                    if let Type::Array(elem_ty, _) = &current_ty {
                        let elem_ty = elem_ty.as_ref().clone();
                        if self.type_needs_cleanup(&elem_ty) {
                            diagnostics.push(mir_diagnostic(
                                "Cannot move cleanup value out of array element".to_string(),
                                span.clone(),
                            ));
                            return;
                        }
                    }
                }
                _ => {}
            }

            let Some(next_ty) = self.project_type(&current_ty, projection) else {
                return;
            };
            current_ty = self
                .backend_contract
                .normalize_storage_type(self.type_context, &next_ty);
        }

        if reference_owned_path && !current_ty.is_copy() {
            diagnostics.push(mir_diagnostic(
                format!(
                    "Cannot move non-copy value of type '{}' out of a reference",
                    self.type_display_name(&current_ty)
                ),
                span,
            ));
        }
    }

    fn has_direct_drop(&self, ty: &Type) -> bool {
        let ty = self
            .backend_contract
            .normalize_storage_type(self.type_context, ty);
        self.type_context
            .id_for_type(&ty)
            .is_some_and(|id| self.direct_drop_types.contains(&id))
    }

    fn type_needs_cleanup(&mut self, ty: &Type) -> bool {
        let ty = self
            .backend_contract
            .normalize_storage_type(self.type_context, ty);
        if let Some(needs_cleanup) = self.cleanup_cache.get(&ty) {
            return *needs_cleanup;
        }

        let needs_cleanup = if self.has_direct_drop(&ty) {
            true
        } else {
            match &ty {
                Type::Struct { id, args } => contract_struct_fields(self.backend_contract, *id)
                    .is_some_and(|layout_fields| {
                        let fields = layout_fields
                            .iter()
                            .map(|(_, field_ty)| self.type_context.type_for(*field_ty))
                            .collect::<Vec<_>>();
                        let subst = generic_substitution_for_fields(
                            &fields,
                            &Type::Struct {
                                id: *id,
                                args: args.clone(),
                            },
                        );
                        fields
                            .into_iter()
                            .map(|field_ty| field_ty.substitute_generics(&subst))
                            .any(|field_ty| self.type_needs_cleanup(&field_ty))
                    }),
                Type::Enum { id, args } => contract_enum_variants(self.backend_contract, *id)
                    .is_some_and(|variants| {
                        let fields = variants
                            .iter()
                            .flat_map(|variant| match &variant.fields {
                                crate::mir::MirVariantLayoutFields::Unit => Vec::new(),
                                crate::mir::MirVariantLayoutFields::Positional(fields) => fields
                                    .iter()
                                    .map(|field_ty| self.type_context.type_for(*field_ty))
                                    .collect(),
                                crate::mir::MirVariantLayoutFields::Named(fields) => fields
                                    .iter()
                                    .map(|(_, field_ty)| self.type_context.type_for(*field_ty))
                                    .collect(),
                            })
                            .collect::<Vec<_>>();
                        let subst = generic_substitution_for_fields(
                            &fields,
                            &Type::Enum {
                                id: *id,
                                args: args.clone(),
                            },
                        );
                        fields
                            .into_iter()
                            .map(|field_ty| field_ty.substitute_generics(&subst))
                            .any(|field_ty| self.type_needs_cleanup(&field_ty))
                    }),
                Type::Array(elem, _) => self.type_needs_cleanup(elem),
                Type::Tuple(elems) | Type::StructRepresentation { fields: elems, .. } => {
                    elems.iter().any(|elem| self.type_needs_cleanup(elem))
                }
                Type::EnumRepresentation { variants, .. } => variants
                    .iter()
                    .flatten()
                    .any(|field| self.type_needs_cleanup(field)),
                Type::Function { .. } => true,
                _ => false,
            }
        };

        self.cleanup_cache.insert(ty, needs_cleanup);
        needs_cleanup
    }

    fn project_type(&self, ty: &Type, projection: &Projection) -> Option<Type> {
        let ty = self
            .backend_contract
            .normalize_storage_type(self.type_context, ty);
        let projected = match projection {
            Projection::Deref => match &ty {
                Type::Reference { inner, .. } | Type::Pointer(inner) => {
                    Some(inner.as_ref().clone())
                }
                _ => None,
            },
            Projection::Field { index, .. } => match &ty {
                Type::Tuple(elems) | Type::StructRepresentation { fields: elems, .. } => {
                    elems.get(*index).cloned()
                }
                Type::Struct { id, args } => {
                    let layout_fields = contract_struct_fields(self.backend_contract, *id)?;
                    let field_ty = layout_fields.get(*index).map(|(_, ty)| *ty)?;
                    let fields = layout_fields
                        .iter()
                        .map(|(_, field_ty)| self.type_context.type_for(*field_ty))
                        .collect::<Vec<_>>();
                    let subst = generic_substitution_for_fields(
                        &fields,
                        &Type::Struct {
                            id: *id,
                            args: args.clone(),
                        },
                    );
                    Some(
                        self.type_context
                            .type_for(field_ty)
                            .substitute_generics(&subst),
                    )
                }
                _ => None,
            },
            Projection::Index(_) => match &ty {
                Type::Array(elem, _) | Type::Slice(elem) => Some(elem.as_ref().clone()),
                Type::Str => Some(Type::U8),
                _ => None,
            },
            Projection::Downcast(_) => Some(ty.clone()),
        }?;
        Some(
            self.backend_contract
                .normalize_storage_type(self.type_context, &projected),
        )
    }

    fn type_display_name(&self, ty: &Type) -> String {
        let ty = self
            .backend_contract
            .normalize_storage_type(self.type_context, ty);
        let context = crate::type_services::display::TypeDisplayContext::default();
        crate::type_services::display::display_type_with_context(&ty, &context).to_string()
    }
}

fn contract_struct_fields(
    backend_contract: &MirBackendContract,
    id: DefId,
) -> Option<&[(String, TypeId)]> {
    match backend_contract.nominal_layouts.get(&id) {
        Some(MirNominalLayout::Struct { fields, .. }) => Some(fields.as_slice()),
        _ => None,
    }
}

fn contract_enum_variants(
    backend_contract: &MirBackendContract,
    id: DefId,
) -> Option<&[crate::mir::MirEnumVariantLayout]> {
    match backend_contract.nominal_layouts.get(&id) {
        Some(MirNominalLayout::Enum { variants, .. }) => Some(variants.as_slice()),
        _ => None,
    }
}

fn type_id_is_pointer(view: TypeView<'_>, ty: crate::ids::TypeId) -> bool {
    matches!(view.ty(ty), Ty::Pointer(_))
}

fn type_id_contains_reference(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: crate::ids::TypeId,
) -> bool {
    let mut seen = HashSet::new();
    type_id_tracks_origin_inner(view, backend_contract, ty, false, &mut seen)
}

// Escape eligibility only: raw Lifetime dependencies are not safe-reference
// payloads, while an abstract Fn can still store an actually borrowed environment.
pub(crate) fn return_type_needs_escape_check(
    type_context: &crate::type_context::TypeContext,
    backend_contract: &MirBackendContract,
    ty: TypeId,
) -> bool {
    use crate::type_services::visit::{visit_type_children, TypeVisitor};

    if !type_context.type_id_tree_is_valid(ty) {
        return true;
    }

    // Canonical nominal arguments may describe logical contents hidden behind
    // raw storage; eligibility is not ownership or no-borrow authority.
    match TypeView::new(type_context).ty(ty) {
        Ty::Struct { .. }
        | Ty::StructRepresentation { .. }
        | Ty::Enum { .. }
        | Ty::EnumRepresentation { .. }
        | Ty::Tuple(_)
        | Ty::Array { .. }
        | Ty::Slice(_) => {}
        _ if type_id_contains_reference(TypeView::new(type_context), backend_contract, ty) => {
            return true;
        }
        _ => {}
    }

    struct StoredCallables<'a> {
        context: &'a crate::type_context::TypeContext,
        contract: &'a MirBackendContract,
        seen: HashSet<Type>,
        found: bool,
    }

    impl TypeVisitor for StoredCallables<'_> {
        fn visit_type(&mut self, ty: &Type) {
            let context = self.context;
            let contract = self.contract;
            let ty = contract.normalize_storage_type(context, ty);
            if self.found || !self.seen.insert(ty.clone()) {
                return;
            }
            match &ty {
                Type::Pointer(_) => {}
                Type::Function { .. } | Type::Reference { .. } => self.found = true,
                Type::Struct { id, args } | Type::Enum { id, args } => {
                    let (fields, parameters) = match (&ty, contract.nominal_layouts.get(id)) {
                        (
                            Type::Struct { .. },
                            Some(MirNominalLayout::Struct {
                                id: owner,
                                fields,
                                generic_params,
                            }),
                        ) if owner == id => (
                            fields.iter().map(|(_, ty)| *ty).collect::<Vec<_>>(),
                            generic_params,
                        ),
                        (
                            Type::Enum { .. },
                            Some(MirNominalLayout::Enum {
                                id: owner,
                                variants,
                                generic_params,
                            }),
                        ) if owner == id => {
                            let fields = variants
                                .iter()
                                .flat_map(|variant| match &variant.fields {
                                    crate::mir::MirVariantLayoutFields::Unit => Vec::new(),
                                    crate::mir::MirVariantLayoutFields::Positional(fields) => {
                                        fields.clone()
                                    }
                                    crate::mir::MirVariantLayoutFields::Named(fields) => {
                                        fields.iter().map(|(_, ty)| *ty).collect()
                                    }
                                })
                                .collect::<Vec<_>>();
                            (fields, generic_params)
                        }
                        _ => {
                            self.found = true; // Missing layout is not borrow-free authority.
                            return;
                        }
                    };
                    if parameters.len() != args.len() {
                        self.found = true;
                        return;
                    }
                    for argument in args {
                        self.visit_type(argument);
                    }
                    let substitution = parameters
                        .iter()
                        .copied()
                        .zip(args.iter().cloned())
                        .collect::<HashMap<_, _>>();
                    for field in fields {
                        if !context.type_id_tree_is_valid(field) {
                            self.found = true;
                            return;
                        }
                        self.visit_type(
                            &context.type_for(field).substitute_generics(&substitution),
                        );
                    }
                }
                Type::Exists(_)
                | Type::Object(_)
                | Type::ObjectSelf { .. }
                | Type::Witness(_)
                | Type::Generic(_)
                | Type::TypeVar(_)
                | Type::Projection { .. }
                | Type::Constructor { .. }
                | Type::Apply { .. }
                | Type::Lambda { .. }
                | Type::BoundVar { .. }
                | Type::Error => self.found = true,
                _ => visit_type_children(&ty, self),
            }
        }
    }

    let mut storage = StoredCallables {
        context: type_context,
        contract: backend_contract,
        seen: HashSet::new(),
        found: false,
    };
    storage.visit_type(&type_context.type_for(ty));
    storage.found
}

pub(crate) fn type_id_tracks_origin(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: crate::ids::TypeId,
) -> bool {
    let mut seen = HashSet::new();
    type_id_tracks_origin_inner(view, backend_contract, ty, true, &mut seen)
}

fn type_id_tracks_origin_inner(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: crate::ids::TypeId,
    include_pointers: bool,
    seen: &mut HashSet<crate::ids::TypeId>,
) -> bool {
    if !seen.insert(ty) {
        return false;
    }

    match view.ty(ty) {
        Ty::Exists(_) => true,
        Ty::Object(_) => true,
        Ty::ObjectSelf { .. } => true,
        Ty::Reference { .. } => true,
        Ty::Pointer(_) if include_pointers => true,
        Ty::Tuple(elems) | Ty::StructRepresentation { fields: elems, .. } => {
            elems.iter().any(|elem| {
                type_id_tracks_origin_inner(view, backend_contract, *elem, include_pointers, seen)
            })
        }
        Ty::EnumRepresentation { variants, .. } => variants.iter().flatten().any(|elem| {
            type_id_tracks_origin_inner(view, backend_contract, *elem, include_pointers, seen)
        }),
        Ty::Array { inner, .. } | Ty::Slice(inner) => {
            type_id_tracks_origin_inner(view, backend_contract, *inner, include_pointers, seen)
        }
        // Abstract callable signatures do not describe the carried environment.
        // Track its possible loans even when Args/Ret/capture headers are scalar.
        // This grants no invocation, source-independence or effect authority.
        Ty::Function {
            params,
            ret,
            captures,
            ..
        } => {
            include_pointers
                || params
                    .iter()
                    .chain(std::iter::once(ret))
                    .any(|ty| type_id_tracks_origin_inner(view, backend_contract, *ty, false, seen))
                || captures.iter().any(|capture| {
                    capture.kind != crate::types::CaptureKind::Move
                        || type_id_tracks_origin_inner(
                            view,
                            backend_contract,
                            capture.ty,
                            false,
                            seen,
                        )
                })
        }
        Ty::Struct { id, args } => {
            args.iter().any(|arg| {
                type_id_tracks_origin_inner(view, backend_contract, *arg, include_pointers, seen)
            }) || contract_struct_fields(backend_contract, *id).is_some_and(|layout| {
                let fields = layout
                    .iter()
                    .map(|(_, field_ty)| view.type_for(*field_ty))
                    .collect::<Vec<_>>();
                let subst = generic_substitution_for_fields(
                    &fields,
                    &Type::Struct {
                        id: *id,
                        args: args.iter().map(|arg| view.type_for(*arg)).collect(),
                    },
                );
                layout.iter().any(|(_, field_ty)| {
                    type_id_tracks_origin_for_structural_type(
                        view,
                        backend_contract,
                        &view.type_for(*field_ty).substitute_generics(&subst),
                        include_pointers,
                        seen,
                    )
                })
            })
        }
        Ty::Enum { id, args } => {
            args.iter().any(|arg| {
                type_id_tracks_origin_inner(view, backend_contract, *arg, include_pointers, seen)
            }) || contract_enum_variants(backend_contract, *id).is_some_and(|variants| {
                let fields = variants
                    .iter()
                    .flat_map(|variant| match &variant.fields {
                        crate::mir::MirVariantLayoutFields::Unit => Vec::new(),
                        crate::mir::MirVariantLayoutFields::Positional(fields) => fields
                            .iter()
                            .map(|field_ty| view.type_for(*field_ty))
                            .collect(),
                        crate::mir::MirVariantLayoutFields::Named(fields) => fields
                            .iter()
                            .map(|(_, field_ty)| view.type_for(*field_ty))
                            .collect(),
                    })
                    .collect::<Vec<_>>();
                let subst = generic_substitution_for_fields(
                    &fields,
                    &Type::Enum {
                        id: *id,
                        args: args.iter().map(|arg| view.type_for(*arg)).collect(),
                    },
                );
                variants.iter().any(|variant| match &variant.fields {
                    crate::mir::MirVariantLayoutFields::Unit => false,
                    crate::mir::MirVariantLayoutFields::Positional(fields) => {
                        fields.iter().any(|field_ty| {
                            type_id_tracks_origin_for_structural_type(
                                view,
                                backend_contract,
                                &view.type_for(*field_ty).substitute_generics(&subst),
                                include_pointers,
                                seen,
                            )
                        })
                    }
                    crate::mir::MirVariantLayoutFields::Named(fields) => {
                        fields.iter().any(|(_, field_ty)| {
                            type_id_tracks_origin_for_structural_type(
                                view,
                                backend_contract,
                                &view.type_for(*field_ty).substitute_generics(&subst),
                                include_pointers,
                                seen,
                            )
                        })
                    }
                })
            })
        }
        Ty::Projection {
            ty,
            trait_id,
            assoc_type,
            trait_args,
        } => {
            let output_tracks = projection_output_type_id(
                backend_contract,
                *ty,
                *trait_id,
                assoc_type.assoc_type_id,
                trait_args,
            )
            .is_some_and(|output| {
                let mut output_seen = seen.clone();
                type_id_tracks_origin_inner(
                    view,
                    backend_contract,
                    output,
                    include_pointers,
                    &mut output_seen,
                )
            });

            output_tracks
                || type_id_tracks_origin_inner(view, backend_contract, *ty, include_pointers, seen)
                || trait_args.iter().any(|arg| {
                    type_id_tracks_origin_inner(
                        view,
                        backend_contract,
                        *arg,
                        include_pointers,
                        seen,
                    )
                })
        }
        Ty::I8
        | Ty::I16
        | Ty::I32
        | Ty::I64
        | Ty::U8
        | Ty::U16
        | Ty::U32
        | Ty::U64
        | Ty::F32
        | Ty::F64
        | Ty::Bool
        | Ty::Char
        | Ty::Str
        | Ty::Unit
        | Ty::Never
        | Ty::Pointer(_)
        | Ty::Error => false,
        Ty::Witness(_)
        | Ty::Generic(_)
        | Ty::TypeVar(_)
        | Ty::Constructor { .. }
        | Ty::Apply { .. }
        | Ty::Lambda { .. }
        | Ty::BoundVar { .. } => true,
    }
}

fn type_id_tracks_origin_for_structural_type(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: &Type,
    include_pointers: bool,
    seen: &mut HashSet<crate::ids::TypeId>,
) -> bool {
    if let Some(id) = view.id_for_type(ty) {
        return type_id_tracks_origin_inner(view, backend_contract, id, include_pointers, seen);
    }

    match ty {
        Type::Exists(_) => true,
        Type::Reference { .. } => true,
        Type::Object(_) => true,
        Type::ObjectSelf { .. } => true,
        Type::Pointer(_) if include_pointers => true,
        Type::Tuple(elems) | Type::StructRepresentation { fields: elems, .. } => {
            elems.iter().any(|elem| {
                type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    elem,
                    include_pointers,
                    seen,
                )
            })
        }
        Type::EnumRepresentation { variants, .. } => variants.iter().flatten().any(|elem| {
            type_id_tracks_origin_for_structural_type(
                view,
                backend_contract,
                elem,
                include_pointers,
                seen,
            )
        }),
        Type::Array(inner, _) | Type::Slice(inner) => type_id_tracks_origin_for_structural_type(
            view,
            backend_contract,
            inner,
            include_pointers,
            seen,
        ),
        Type::Function {
            params,
            ret,
            captures,
            ..
        } => {
            include_pointers
                || params
                    .iter()
                    .chain(std::iter::once(ret.as_ref()))
                    .any(|ty| {
                        type_id_tracks_origin_for_structural_type(
                            view,
                            backend_contract,
                            ty,
                            false,
                            seen,
                        )
                    })
                || captures.iter().any(|capture| {
                    capture.kind != crate::types::CaptureKind::Move
                        || type_id_tracks_origin_for_structural_type(
                            view,
                            backend_contract,
                            &capture.ty,
                            false,
                            seen,
                        )
                })
        }
        Type::Struct { id, args } => {
            args.iter().any(|arg| {
                type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    arg,
                    include_pointers,
                    seen,
                )
            }) || contract_struct_fields(backend_contract, *id).is_some_and(|layout| {
                let fields = layout
                    .iter()
                    .map(|(_, field_ty)| view.type_for(*field_ty))
                    .collect::<Vec<_>>();
                let subst = generic_substitution_for_fields(
                    &fields,
                    &Type::Struct {
                        id: *id,
                        args: args.clone(),
                    },
                );

                fields.into_iter().any(|field_ty| {
                    type_id_tracks_origin_for_structural_type(
                        view,
                        backend_contract,
                        &field_ty.substitute_generics(&subst),
                        include_pointers,
                        seen,
                    )
                })
            })
        }
        Type::Enum { id, args } => {
            args.iter().any(|arg| {
                type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    arg,
                    include_pointers,
                    seen,
                )
            }) || contract_enum_variants(backend_contract, *id).is_some_and(|variants| {
                let fields = variants
                    .iter()
                    .flat_map(|variant| match &variant.fields {
                        crate::mir::MirVariantLayoutFields::Unit => Vec::new(),
                        crate::mir::MirVariantLayoutFields::Positional(fields) => fields
                            .iter()
                            .map(|field_ty| view.type_for(*field_ty))
                            .collect(),
                        crate::mir::MirVariantLayoutFields::Named(fields) => fields
                            .iter()
                            .map(|(_, field_ty)| view.type_for(*field_ty))
                            .collect(),
                    })
                    .collect::<Vec<_>>();
                let subst = generic_substitution_for_fields(
                    &fields,
                    &Type::Enum {
                        id: *id,
                        args: args.clone(),
                    },
                );

                fields.into_iter().any(|field_ty| {
                    type_id_tracks_origin_for_structural_type(
                        view,
                        backend_contract,
                        &field_ty.substitute_generics(&subst),
                        include_pointers,
                        seen,
                    )
                })
            })
        }
        Type::Projection {
            ty,
            trait_id,
            assoc_type,
            trait_args,
        } => {
            let output_tracks = projection_output_for_structural_type(
                view,
                backend_contract,
                ty,
                *trait_id,
                assoc_type.assoc_type_id,
                trait_args,
            )
            .is_some_and(|output| {
                let mut output_seen = seen.clone();
                type_id_tracks_origin_inner(
                    view,
                    backend_contract,
                    output,
                    include_pointers,
                    &mut output_seen,
                )
            });

            output_tracks
                || type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    ty,
                    include_pointers,
                    seen,
                )
                || trait_args.iter().any(|arg| {
                    type_id_tracks_origin_for_structural_type(
                        view,
                        backend_contract,
                        arg,
                        include_pointers,
                        seen,
                    )
                })
        }
        Type::Apply { constructor, args } => {
            type_id_tracks_origin_for_structural_type(
                view,
                backend_contract,
                constructor,
                include_pointers,
                seen,
            ) || args.iter().any(|arg| {
                type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    arg,
                    include_pointers,
                    seen,
                )
            })
        }
        Type::Lambda { body, .. } => type_id_tracks_origin_for_structural_type(
            view,
            backend_contract,
            body,
            include_pointers,
            seen,
        ),
        Type::Witness(_)
        | Type::Generic(_)
        | Type::TypeVar(_)
        | Type::Constructor { .. }
        | Type::BoundVar { .. } => true,
        Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::F32
        | Type::F64
        | Type::Bool
        | Type::Str
        | Type::Char
        | Type::Unit
        | Type::Never
        | Type::Pointer(_)
        | Type::Error => false,
    }
}

fn projection_output_type_id(
    backend_contract: &MirBackendContract,
    base: TypeId,
    trait_id: DefId,
    assoc_type_id: AssocTypeId,
    trait_args: &[TypeId],
) -> Option<TypeId> {
    backend_contract
        .projection_outputs
        .get(&MirProjectionKey {
            base,
            trait_id,
            assoc_type_id,
            trait_args: trait_args.to_vec(),
        })
        .copied()
}

fn projection_output_for_structural_type(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    base: &Type,
    trait_id: DefId,
    assoc_type_id: AssocTypeId,
    trait_args: &[Type],
) -> Option<TypeId> {
    let base = view.id_for_type(base)?;
    let trait_args = trait_args
        .iter()
        .map(|arg| view.id_for_type(arg))
        .collect::<Option<Vec<_>>>()?;
    projection_output_type_id(backend_contract, base, trait_id, assoc_type_id, &trait_args)
}

fn generic_substitution_for_fields(
    fields: &[Type],
    source_ty: &Type,
) -> HashMap<GenericParamId, Type> {
    let args = match source_ty {
        Type::Enum { args, .. } | Type::Struct { args, .. } => args,
        _ => return HashMap::new(),
    };
    let mut generic_params = HashSet::new();
    for field_ty in fields {
        field_ty.collect_generic_params(&mut generic_params);
    }
    generic_params
        .into_iter()
        .filter_map(|param| {
            args.get(param.index as usize)
                .cloned()
                .map(|arg| (param, arg))
        })
        .collect()
}

fn type_id_is_mut_reference(view: TypeView<'_>, ty: crate::ids::TypeId) -> bool {
    matches!(view.ty(ty), Ty::Reference { mutable: true, .. })
}

impl BorrowChecker {
    pub fn run(program: &MirProgram) -> Result<(), Diagnostics> {
        let mut diagnostics = crate::mir::erased::native_retention_diagnostics(program);
        let value_loan_proof = Self::native_return_loan_proof(program);
        let return_summaries = &value_loan_proof.lifetime;
        callable_results::validate_declarations(
            program,
            return_summaries,
            &value_loan_proof.measured,
            Some(&value_loan_proof.sources),
            &mut diagnostics,
        );
        #[cfg(test)]
        hkt_return_origin_tests::pre_callable_result_origins(
            program,
            return_summaries,
            &value_loan_proof.summaries,
        );
        let mutations = NativeMutationTable::for_program(program);
        #[cfg(test)]
        allocator_trace_tests::program(program, return_summaries, &value_loan_proof.summaries);
        #[cfg(test)]
        allocator_trace_tests::return_program(
            program,
            return_summaries,
            &value_loan_proof.summaries,
        );
        #[cfg(test)]
        allocator_trace_tests::integration_begin(program);

        for (_, func) in program.functions() {
            if let Err(func_diagnostics) = Self::validate_with_mutation_authority(
                func,
                &program.type_context,
                &program.backend_contract,
                return_summaries,
                &value_loan_proof.summaries,
                &mutations,
                Some(&value_loan_proof.sources),
            ) {
                for diag in func_diagnostics.0 {
                    diagnostics.push(diag);
                }
            }
        }

        #[cfg(test)]
        allocator_trace_tests::fresh_missing_program(
            program,
            &return_summaries,
            &value_loan_proof.summaries,
        );
        #[cfg(test)]
        allocator_trace_tests::integration_program(
            program,
            &return_summaries,
            &value_loan_proof.summaries,
            &diagnostics,
        );

        if diagnostics.0.is_empty() {
            Ok(())
        } else {
            Err(diagnostics)
        }
    }

    #[cfg(test)]
    fn check_function(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
    ) -> Result<(), Diagnostics> {
        Self::check_function_with_contract(
            func,
            type_context,
            &MirBackendContract::default(),
            &HashMap::new(),
        )
    }

    #[cfg(test)]
    fn check_function_with_contract(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
    ) -> Result<(), Diagnostics> {
        let mut checker = BorrowChecker;
        checker.validate_ownership_core(func, type_context, backend_contract, return_summaries)
    }

    pub fn validate_ownership_core(
        &mut self,
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
    ) -> Result<(), Diagnostics> {
        Self::validate_with_origin_authority(
            func,
            type_context,
            backend_contract,
            return_summaries,
            return_summaries,
        )
    }

    fn validate_with_origin_authority(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        value_loan_summaries: &HashMap<MirFunctionId, OriginSet>,
    ) -> Result<(), Diagnostics> {
        Self::validate_with_mutation_authority(
            func,
            type_context,
            backend_contract,
            return_summaries,
            value_loan_summaries,
            &NativeMutationTable::default(),
            None,
        )
    }

    fn validate_with_mutation_authority(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        value_loan_summaries: &HashMap<MirFunctionId, OriginSet>,
        mutations: &NativeMutationTable,
        sources: Option<&return_sources::NativeReturnSources>,
    ) -> Result<(), Diagnostics> {
        let (mut diagnostics, mutation_origins) = Self::validate_moves_with_return_sources(
            func,
            type_context,
            backend_contract,
            value_loan_summaries,
            mutations,
            sources,
        );
        Self::validate_reference_escapes_with_return_sources(
            func,
            type_context,
            backend_contract,
            return_summaries,
            &mut diagnostics,
            &mutation_origins,
            sources,
        );
        Self::validate_drop_obligations(func, backend_contract, &mut diagnostics);

        if diagnostics.0.is_empty() {
            Ok(())
        } else {
            Err(diagnostics)
        }
    }

    fn reference_return_summaries(program: &MirProgram) -> HashMap<MirFunctionId, OriginSet> {
        Self::return_summaries_for_purpose(program, OriginPurpose::Lifetime)
    }

    fn native_return_loan_proof(program: &MirProgram) -> NativeReturnLoanProof {
        let mut sources = return_sources::NativeReturnSources::for_program(program);
        // Never flatten qualified loads into globally precise ParamContents.
        // Unqualified calls still instantiate the original conservative summary.
        let mut measured = Self::return_summaries_for_purpose(program, OriginPurpose::ValueLoans);
        let mut lifetime = Self::reference_return_summaries(program);
        // Removing UnknownExternal for loan transport is not certification of
        // an empty dependency envelope. Both raw measurements discharge proof.
        loop {
            // Only already measured, signature-checked producers enter this
            // layer. Recursive unknown callbacks cannot seed their own proof.
            let scalar_lifetime = Self::measured_return_summaries(
                program,
                OriginPurpose::Lifetime,
                Some(&sources),
                true,
            );
            let scalar_values = Self::measured_return_summaries(
                program,
                OriginPurpose::ValueLoans,
                Some(&sources),
                true,
            );
            let scalar_changed = scalar_lifetime != sources.scalar_lifetime_dependencies
                || scalar_values != sources.scalar_value_dependencies;
            sources.scalar_lifetime_dependencies = scalar_lifetime;
            sources.scalar_value_dependencies = scalar_values;
            let changed = sources.certify_callable_results(program, &lifetime, &measured);
            let next_lifetime = Self::return_summaries_with_sources(
                program,
                OriginPurpose::Lifetime,
                Some(&sources),
            );
            let next_values = Self::return_summaries_with_sources(
                program,
                OriginPurpose::ValueLoans,
                Some(&sources),
            );
            if !changed && !scalar_changed && next_lifetime == lifetime && next_values == measured {
                break;
            }
            lifetime = next_lifetime;
            measured = next_values;
        }
        let mut summaries = measured.clone();
        for origins in summaries.values_mut() {
            // An opaque leaf's own external storage is not an input loan. Its
            // actual input origins were unioned separately and remain intact.
            origins.remove(&ReferenceOrigin::UnknownExternal);
        }
        NativeReturnLoanProof {
            summaries,
            measured,
            lifetime,
            sources,
        }
    }

    pub(crate) fn checked_native_return_origins(
        program: &MirProgram,
    ) -> HashMap<MirFunctionId, crate::hir::borrowing::NativeReturnOrigins<TypeId>> {
        use crate::hir::borrowing::{NativeReturnOrigins, NativeReturnSource};
        let summaries = Self::return_summaries_for_purpose(program, OriginPurpose::ValueLoans);
        let proof = Self::native_return_loan_proof(program);
        let resolved = call_targets::resolved_bodies(program);
        let tc = &program.type_context;
        let contract = &program.backend_contract;
        if !crate::mir::backend_contract::validate_backend_contract_against_mir(
            contract,
            program.functions.values(),
        )
        .is_empty()
        {
            return HashMap::new();
        }
        let mut storage_report = crate::mir::agreement::MirAgreementReport::default();
        crate::mir::agreement::validate_native_result_storage(program, &mut storage_report);
        if !storage_report.is_clean() {
            return HashMap::new();
        }
        let mut exported = HashMap::new();
        for (id, body) in program.functions() {
            let body = resolved.get(id).unwrap_or(body);
            if !Self::body_available_for_purpose(program, id, body, OriginPurpose::ValueLoans)
                || !body.closure_captures.is_empty()
            {
                continue;
            }
            let measured;
            let storage = contract.function_result_storage(body);
            let tracked = type_id_tracks_origin(TypeView::new(tc), contract, storage);
            let summary = if tracked {
                let Some(summary) = summaries.get(id) else {
                    continue;
                };
                summary
            } else {
                // Value dependencies of an untracked owning/aggregate result
                // are not portable storage loans. This extension publishes
                // only actual scalar producer measurements, not owner layouts.
                if !tc.type_for(body.ret_type).is_numeric()
                    && !matches!(tc.try_ty(body.ret_type), Some(Ty::Bool | Ty::Char))
                {
                    continue;
                }
                // A scalar result header is not producer authority. Retain a
                // separately measured dependency envelope from this checked
                // body, and export only role-agreeing portable sources.
                let lifetime = callable_results::body_sources_with_calls(
                    program,
                    body,
                    &proof.lifetime,
                    &proof.measured,
                    Some(&proof.sources),
                );
                let values = callable_results::body_sources_with_calls(
                    program,
                    body,
                    &proof.measured,
                    &proof.measured,
                    Some(&proof.sources),
                );
                if lifetime != values {
                    continue;
                }
                let Some(crate::types::CallableResultBorrowing::Sources(sources)) = lifetime else {
                    continue;
                };
                let Some(origins) = sources
                    .iter()
                    .map(|source| {
                        use crate::types::CallableResultSource as S;
                        Some(match *source {
                            S::ArgumentValue(index) => {
                                ReferenceOrigin::Param(Local(index as usize + 1))
                            }
                            S::ArgumentContents(index) => {
                                ReferenceOrigin::ParamContents(Local(index as usize + 1))
                            }
                            S::ArgumentBorrowedContents(index) => {
                                ReferenceOrigin::ParamBorrowedContents(Local(index as usize + 1))
                            }
                            S::Static => ReferenceOrigin::Static,
                            S::Unknown => ReferenceOrigin::UnknownExternal,
                            _ => return None,
                        })
                    })
                    .collect::<Option<OriginSet>>()
                else {
                    continue;
                };
                measured = origins;
                &measured
            };
            let analysis = ReferenceEscapeAnalysis::new(
                body,
                tc,
                contract,
                &summaries,
                MissingSummaryMode::Defer,
            );
            let results = run_fixpoint(&analysis, body);
            if !body.basic_blocks.iter().enumerate().any(|(block, data)| {
                results.reachable[block]
                    && matches!(
                        data.terminator,
                        Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
                    )
            }) {
                continue;
            }
            let sources = summary
                .iter()
                .map(|origin| {
                    let argument = |local: Local| {
                        let index = local.0.checked_sub(1)?;
                        (index < body.arg_count
                            && body.local_decls.get(local.0)?.source == LocalSource::Argument)
                            .then(|| u32::try_from(index).ok())
                            .flatten()
                    };
                    Some(match origin {
                        ReferenceOrigin::Param(local) => {
                            NativeReturnSource::ArgumentValue(argument(*local)?)
                        }
                        ReferenceOrigin::ParamContents(local) => {
                            NativeReturnSource::ArgumentContents(argument(*local)?)
                        }
                        ReferenceOrigin::ParamBorrowedContents(local) => {
                            NativeReturnSource::ArgumentBorrowedContents(argument(*local)?)
                        }
                        ReferenceOrigin::Static => NativeReturnSource::Static,
                        ReferenceOrigin::UnknownExternal if !tracked => {
                            NativeReturnSource::UnknownExternal
                        }
                        ReferenceOrigin::Local(_)
                        | ReferenceOrigin::Temporary(_)
                        | ReferenceOrigin::UnknownExternal => return None,
                    })
                })
                .collect::<Option<std::collections::BTreeSet<_>>>();
            let Some(sources) = sources else {
                continue;
            };
            let entry = NativeReturnOrigins {
                params: body
                    .local_decls
                    .iter()
                    .skip(1)
                    .take(body.arg_count)
                    .map(|decl| decl.ty)
                    .collect(),
                ret: storage,
                sources: sources.into_iter().collect(),
            };
            if entry.sources_are_valid(|ty| {
                matches!(tc.try_ty(*ty), Some(Ty::Reference { .. } | Ty::Pointer(_)))
            }) {
                exported.insert(id.clone(), entry);
            }
        }
        exported
    }

    pub(crate) fn checked_returned_callable_borrowing(
        program: &MirProgram,
    ) -> HashMap<MirFunctionId, crate::types::CallableResultBorrowing> {
        let proof = Self::native_return_loan_proof(program);
        program
            .functions()
            .filter_map(|(id, _)| {
                Some((
                    id.clone(),
                    proof.sources.callable_results.portable_return(id)?,
                ))
            })
            .collect()
    }

    fn return_summaries_for_purpose(
        program: &MirProgram,
        purpose: OriginPurpose,
    ) -> HashMap<MirFunctionId, OriginSet> {
        Self::return_summaries_with_sources(program, purpose, None)
    }

    fn return_summaries_with_sources(
        program: &MirProgram,
        purpose: OriginPurpose,
        sources: Option<&return_sources::NativeReturnSources>,
    ) -> HashMap<MirFunctionId, OriginSet> {
        Self::measured_return_summaries(program, purpose, sources, false)
    }

    fn measured_return_summaries(
        program: &MirProgram,
        purpose: OriginPurpose,
        sources: Option<&return_sources::NativeReturnSources>,
        measure_scalars: bool,
    ) -> HashMap<MirFunctionId, OriginSet> {
        // Presence includes proven empty bodies; absence means opaque authority.
        let resolved = call_targets::resolved_bodies(program);
        let measured_result = |function: &MirFunction| {
            let tc = &program.type_context;
            type_id_tracks_origin(
                TypeView::new(tc),
                &program.backend_contract,
                program.backend_contract.function_result_storage(function),
            ) || measure_scalars
                && (tc.type_for(function.ret_type).is_numeric()
                    || matches!(tc.try_ty(function.ret_type), Some(Ty::Bool | Ty::Char)))
        };
        let mut summaries = program
            .functions()
            .filter_map(|(id, func)| {
                let func = resolved.get(id).unwrap_or(func);
                (Self::body_available_for_purpose(program, id, func, purpose)
                    && measured_result(func))
                .then(|| (id.clone(), OriginSet::new()))
            })
            .collect::<HashMap<_, _>>();

        // Bodyless return authority is signature-bound producer evidence, never
        // effect knowledge or an inferred empty summary.
        let imported = Self::imported_native_return_summaries(program, purpose, measure_scalars);
        summaries.extend(imported.clone());

        loop {
            let previous = summaries.clone();
            let mut next = imported.clone();
            for (id, func) in program.functions() {
                let func = resolved.get(id).unwrap_or(func);
                if !Self::body_available_for_purpose(program, id, func, purpose)
                    || !measured_result(func)
                {
                    continue;
                }

                let origins = Self::return_origins_with_sources(
                    func,
                    &program.type_context,
                    &program.backend_contract,
                    &previous,
                    purpose,
                    sources,
                );
                next.insert(id.clone(), origins);
            }

            if next == previous {
                return next;
            }
            summaries = next;
        }
    }

    fn imported_native_return_summaries(
        program: &MirProgram,
        purpose: OriginPurpose,
        measure_scalars: bool,
    ) -> HashMap<MirFunctionId, OriginSet> {
        use crate::hir::borrowing::NativeReturnSource;
        let tc = &program.type_context;
        let contract = &program.backend_contract;
        if !crate::mir::backend_contract::validate_backend_contract(contract).is_empty() {
            return HashMap::new();
        }
        contract
            .native_return_origins
            .iter()
            .filter_map(|(key, origins)| {
                if !origins
                    .params
                    .iter()
                    .chain(std::iter::once(&origins.ret))
                    .all(|ty| tc.type_id_tree_is_valid(*ty))
                    || !origins.sources_are_valid(|ty| {
                        matches!(tc.try_ty(*ty), Some(Ty::Reference { .. } | Ty::Pointer(_)))
                    })
                    || origins
                        .sources
                        .contains(&NativeReturnSource::UnknownExternal)
                        && !tc.type_for(origins.ret).is_numeric()
                        && !matches!(tc.try_ty(origins.ret), Some(Ty::Bool | Ty::Char))
                    || !measure_scalars
                        && !type_id_tracks_origin(TypeView::new(tc), contract, origins.ret)
                {
                    return None;
                }
                let id = MirCallable::Resolved(key.clone()).function_id()?;
                if program.functions().any(|(body_id, _)| body_id == &id) {
                    return None;
                }
                let summary = origins
                    .sources
                    .iter()
                    .map(|source| match *source {
                        NativeReturnSource::ArgumentValue(index) => {
                            ReferenceOrigin::Param(Local(index as usize + 1))
                        }
                        NativeReturnSource::ArgumentContents(index) => {
                            ReferenceOrigin::ParamContents(Local(index as usize + 1))
                        }
                        NativeReturnSource::ArgumentBorrowedContents(index) => {
                            ReferenceOrigin::ParamBorrowedContents(Local(index as usize + 1))
                        }
                        NativeReturnSource::Static => ReferenceOrigin::Static,
                        NativeReturnSource::UnknownExternal => ReferenceOrigin::UnknownExternal,
                    })
                    .collect();
                let summary = if purpose == OriginPurpose::Lifetime
                    && matches!(tc.try_ty(origins.ret), Some(Ty::Pointer(_)))
                {
                    ReferenceEscapeAnalysis::borrowed_contents_origins(summary)
                } else {
                    summary
                };
                Some((id, summary))
            })
            .collect()
    }

    fn body_available_for_purpose(
        program: &MirProgram,
        id: &MirFunctionId,
        func: &MirFunction,
        purpose: OriginPurpose,
    ) -> bool {
        if purpose == OriginPurpose::Lifetime {
            return !func.basic_blocks.is_empty();
        }
        if func.basic_blocks.is_empty() || &func.id != id {
            return false;
        }
        let Some(key) = program.backend_contract.function_bodies.get(id) else {
            return false;
        };
        let Some(decl) = program.backend_contract.callable(key) else {
            return false;
        };
        // Opaque calls contribute UnknownExternal AND every argument/callable
        // dependency. Measuring that conservative body is not certification of
        // its incoming callback, independence, or effects. Refusing the whole
        // body instead loses the adapter's checked contents-versus-storage roles.
        &decl.key == key
            && matches!(&decl.kind, crate::mir::MirCallableKind::LocalBody { function_id } if function_id == id)
            && decl.signature.params.len() == func.arg_count
            && decl
                .signature
                .params
                .iter()
                .enumerate()
                .all(|(index, param)| {
                    func.local_decls
                        .get(index + 1)
                        .is_some_and(|local| local.ty == param.semantic_ty)
                })
            && decl.signature.ret.semantic_ty == func.ret_type
            && decl.signature.ret.abi_ty == func.ret_type
            && func
                .local_decls
                .first()
                .is_some_and(|local| local.ty == func.ret_type)
    }

    #[cfg(test)]
    fn reference_return_origins(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
    ) -> OriginSet {
        Self::return_origins_for_purpose(
            func,
            type_context,
            backend_contract,
            return_summaries,
            OriginPurpose::Lifetime,
        )
    }

    fn return_origins_for_purpose(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        purpose: OriginPurpose,
    ) -> OriginSet {
        Self::return_origins_with_sources(
            func,
            type_context,
            backend_contract,
            return_summaries,
            purpose,
            None,
        )
    }

    fn return_origins_with_sources(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        purpose: OriginPurpose,
        sources: Option<&return_sources::NativeReturnSources>,
    ) -> OriginSet {
        let mut analysis = ReferenceEscapeAnalysis::new(
            func,
            type_context,
            backend_contract,
            return_summaries,
            MissingSummaryMode::Defer,
        )
        .with_return_sources(sources);
        analysis.purpose = purpose;
        // Checked scalar bodies participate in dependency composition; normal
        // loan/lifetime consumers still track only their actual carrier types.
        analysis.track_value_dependencies =
            !type_id_tracks_origin(TypeView::new(type_context), backend_contract, func.ret_type)
                && (type_context.type_for(func.ret_type).is_numeric()
                    || matches!(
                        type_context.try_ty(func.ret_type),
                        Some(Ty::Bool | Ty::Char)
                    ));
        let origins = analysis.measured_return_origins().unwrap_or_default();

        // A raw-pointer result is a view carrier. Keep its receiver provenance
        // until a later dereference, without treating copies of reference fields
        // or rebuilt owning aggregates as borrows of the receiver's storage.
        if purpose == OriginPurpose::Lifetime
            && matches!(
                TypeView::new(type_context).ty(func.ret_type),
                Ty::Pointer(_)
            )
        {
            ReferenceEscapeAnalysis::borrowed_contents_origins(origins)
        } else {
            origins
        }
    }

    fn validate_moves_and_initialization(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        mutations: &NativeMutationTable,
    ) -> (Diagnostics, NativeMutationOrigins) {
        Self::validate_moves_with_return_sources(
            func,
            type_context,
            backend_contract,
            return_summaries,
            mutations,
            None,
        )
    }

    fn validate_moves_with_return_sources(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        mutations: &NativeMutationTable,
        sources: Option<&return_sources::NativeReturnSources>,
    ) -> (Diagnostics, NativeMutationOrigins) {
        let mut diagnostics = Diagnostics::default();
        let mut mutation_origins = NativeMutationOrigins::new();
        let mut move_validation = MoveValidationContext::new(type_context, backend_contract);

        let init_analysis = InitializationAnalysis::new(func, type_context);
        let init_results = run_fixpoint(&init_analysis, func);

        let borrows = borrows::collect_function_borrows_with_return_sources(
            func,
            type_context,
            backend_contract,
            return_summaries,
            sources,
        );
        let place_paths = crate::mir::borrowck::paths::PlacePathTable::for_function(func);
        let loan_table = LoanTable::from_borrows_with_paths(&borrows, place_paths);
        let loans_by_location = LoanLocationIndex::for_function(&loan_table, func);
        let candidates = func
            .basic_blocks
            .iter()
            .map(|block| {
                block
                    .terminator
                    .as_ref()
                    .map(|term| {
                        native_mutation_candidates(
                            func,
                            type_context,
                            backend_contract,
                            mutations,
                            term,
                        )
                    })
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        let reference_liveness = liveness::compute_reference_liveness_with_return_sources(
            func,
            type_context,
            backend_contract,
            &loan_table,
            &candidates,
            sources,
        );
        let active_loan_entries = liveness::compute_active_loan_entries_with_return_sources(
            func,
            type_context,
            backend_contract,
            &loan_table,
            &loans_by_location,
            &reference_liveness,
            return_summaries,
            &candidates,
            sources,
        );

        #[cfg(test)]
        allocator_trace_tests::function(
            func,
            type_context,
            backend_contract,
            &init_results,
            &loan_table,
            &loans_by_location,
            &reference_liveness,
            &active_loan_entries,
        );

        for (block_idx, block) in func.basic_blocks.iter().enumerate() {
            if !init_results.reachable[block_idx] {
                continue;
            }
            let mut state = init_results.entry_sets[block_idx].clone();
            let mut active_loans = active_loan_entries[block_idx].clone();

            for (stmt_idx, stmt) in block.statements.iter().enumerate() {
                #[cfg(test)]
                allocator_trace_tests::state(
                    "before-retain",
                    func,
                    stmt.span.as_ref(),
                    &state,
                    &active_loans,
                );
                let mut live_loans = active_loans.clone();
                if let Some(live_locals) = reference_liveness
                    .before_statement_sets
                    .get(block_idx)
                    .and_then(|stmt_sets| stmt_sets.get(stmt_idx))
                {
                    live_loans.retain_owners(|owner| live_locals.contains(owner));
                }

                #[cfg(test)]
                allocator_trace_tests::statement(
                    func,
                    type_context,
                    backend_contract,
                    &loan_table,
                    block_idx,
                    stmt_idx,
                    stmt,
                    &state,
                    &live_loans,
                );

                Self::check_statement_loans(
                    stmt,
                    type_context,
                    backend_contract,
                    &loan_table,
                    &live_loans,
                    func,
                    &mut diagnostics,
                );
                Self::check_statement(
                    stmt,
                    &init_analysis,
                    &state,
                    func,
                    &mut move_validation,
                    &mut diagnostics,
                );

                init_analysis.apply_statement(&mut state, stmt);

                // Replay exactly the transfer used to compute block-entry loans.
                // Keeping a second operand-specific path here loses object and
                // reborrow owners before their next use in the same block.
                apply_active_loan_statement_transfer(
                    &mut active_loans,
                    func,
                    type_context,
                    backend_contract,
                    stmt,
                );

                let location = crate::mir::borrowck::location::Location::new(
                    crate::mir::BasicBlockId(block_idx),
                    crate::mir::borrowck::location::StatementIndex(stmt_idx),
                );
                for loan_id in loans_by_location.loan_ids_at(location) {
                    active_loans.activate(loan_id, &loan_table);
                }

                if let Some(live_locals) = reference_liveness
                    .after_statement_sets
                    .get(block_idx)
                    .and_then(|stmt_sets| stmt_sets.get(stmt_idx))
                {
                    active_loans.retain_owners(|owner| live_locals.contains(owner));
                }

                if let StatementKind::StorageDead(local) = &stmt.kind {
                    active_loans.release_owner(*local);
                }
            }

            if let Some(term) = &block.terminator {
                #[cfg(test)]
                allocator_trace_tests::terminator(
                    func,
                    type_context,
                    backend_contract,
                    &loan_table,
                    block_idx,
                    block.statements.len(),
                    term,
                    &state,
                    &active_loans,
                );
                Self::check_terminator(
                    term,
                    type_context,
                    backend_contract,
                    &init_analysis,
                    &state,
                    &loan_table,
                    &active_loans,
                    func,
                    &mut diagnostics,
                );
                mutation_origins.insert(
                    block_idx,
                    native_mutation_bindings(
                        func,
                        type_context,
                        backend_contract,
                        &loan_table,
                        &active_loans,
                        term,
                        &candidates[block_idx],
                    ),
                );
                liveness::apply_active_loan_terminator_transfer_with_return_sources(
                    &mut active_loans,
                    func,
                    type_context,
                    backend_contract,
                    term,
                    return_summaries,
                    Some((&loan_table, &candidates[block_idx])),
                    sources,
                );
            }
        }

        (diagnostics, mutation_origins)
    }

    #[cfg(test)]
    fn validate_reference_escapes(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        diagnostics: &mut Diagnostics,
    ) {
        Self::validate_reference_escapes_with_mutations(
            func,
            type_context,
            backend_contract,
            return_summaries,
            diagnostics,
            &NativeMutationOrigins::new(),
        );
    }

    fn validate_reference_escapes_with_mutations(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        diagnostics: &mut Diagnostics,
        mutation_origins: &NativeMutationOrigins,
    ) {
        Self::validate_reference_escapes_with_return_sources(
            func,
            type_context,
            backend_contract,
            return_summaries,
            diagnostics,
            mutation_origins,
            None,
        );
    }

    fn validate_reference_escapes_with_return_sources(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        diagnostics: &mut Diagnostics,
        mutation_origins: &NativeMutationOrigins,
        sources: Option<&return_sources::NativeReturnSources>,
    ) {
        #[cfg(test)]
        allocator_trace_tests::integration_return_gate(func, type_context, backend_contract);
        if !type_id_contains_reference(
            TypeView::new(type_context),
            backend_contract,
            backend_contract.function_result_storage(func),
        ) && !func.local_decls.iter().any(|decl| {
            type_id_tracks_origin(TypeView::new(type_context), backend_contract, decl.ty)
        }) {
            return;
        }

        let mut analysis = ReferenceEscapeAnalysis::new(
            func,
            type_context,
            backend_contract,
            return_summaries,
            MissingSummaryMode::Conservative,
        );
        analysis.return_sources = sources;
        analysis.mutation_origins = Some(mutation_origins);
        let results = run_fixpoint(&analysis, func);

        #[cfg(test)]
        allocator_trace_tests::return_exits(&analysis, &results);
        #[cfg(test)]
        allocator_trace_tests::integration_return_exits(&analysis, &results);

        Self::validate_temporary_reference_storage(func, &analysis, &results, diagnostics);

        // An abstract Fn field can omit captures from its declared type while
        // the actual returned value still borrows local closure storage.
        let return_place = Place {
            local: Local(0),
            projection: Vec::new(),
        };
        for (block_idx, block) in func.basic_blocks.iter().enumerate() {
            if !matches!(
                block.terminator,
                Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
            ) {
                continue;
            }
            for (place, origins) in &results.exit_sets[block_idx].origins {
                if !ReferenceEscapeAnalysis::place_is_prefix(&return_place, place) {
                    continue;
                }
                #[cfg(test)]
                allocator_trace_tests::integration_return_entry(
                    &analysis, &results, block_idx, place, origins,
                );

                let Some(ty) = backend_contract.place_type_id(type_context, func, place) else {
                    #[cfg(test)]
                    allocator_trace_tests::fresh_missing_return(
                        &analysis, &results, block_idx, place, origins,
                    );
                    diagnostics.push(
                        Diagnostic::for_internal(format!(
                            "Missing checked return-place type for {:?} in {:?}",
                            place, func.id,
                        ))
                        .with_code(DiagnosticCode::Borrow),
                    );
                    continue;
                };
                if !return_type_needs_escape_check(type_context, backend_contract, ty) {
                    continue;
                }

                for origin in origins {
                    match origin {
                        ReferenceOrigin::Temporary(local) => {
                            #[cfg(test)]
                            allocator_trace_tests::integration_return_reject(
                                &analysis, block_idx, place, *origin,
                            );
                            diagnostics.push(mir_diagnostic(
                                "Cannot return reference to temporary value".to_string(),
                                Self::origin_span(func, *local),
                            ));
                        }
                        ReferenceOrigin::Local(local) => {
                            #[cfg(test)]
                            allocator_trace_tests::integration_return_reject(
                                &analysis, block_idx, place, *origin,
                            );
                            #[cfg(test)]
                            hkt_return_origin_tests::local_return_reject(
                                &analysis, block_idx, place, origins, *local, ty,
                            );
                            diagnostics.push(mir_diagnostic(
                                "Cannot return reference to local value because it does not live long enough"
                                    .to_string(),
                                Self::origin_span(func, *local),
                            ));
                        }
                        ReferenceOrigin::Param(_)
                        | ReferenceOrigin::ParamContents(_)
                        | ReferenceOrigin::ParamBorrowedContents(_)
                        | ReferenceOrigin::Static
                        | ReferenceOrigin::UnknownExternal => {}
                    }
                }
            }
        }
    }

    fn validate_temporary_reference_storage(
        func: &MirFunction,
        analysis: &ReferenceEscapeAnalysis<'_>,
        results: &crate::mir::dataflow::Results<ReferenceOriginState>,
        diagnostics: &mut Diagnostics,
    ) {
        for (block_idx, block) in func.basic_blocks.iter().enumerate() {
            if !results.reachable[block_idx] {
                continue;
            }
            let mut state = results.entry_sets[block_idx].clone();
            for stmt in &block.statements {
                #[cfg(test)]
                let integration_before = allocator_trace_tests::integration_statement_before(
                    &analysis,
                    &state,
                    stmt,
                    block_idx,
                    diagnostics.0.len(),
                );
                let external_destination = if let StatementKind::Assign(dest, _) = &stmt.kind {
                    let mut prefix = Place {
                        local: dest.local,
                        projection: Vec::new(),
                    };
                    let through_reference = dest.projection.iter().any(|projection| {
                        let reference = *projection == Projection::Deref
                            && analysis
                                .backend_contract
                                .place_type_id(analysis.type_context, func, &prefix)
                                .is_some_and(|ty| {
                                    matches!(
                                        analysis.type_context.try_ty(ty),
                                        Some(Ty::Reference { .. })
                                    )
                                });
                        prefix.projection.push(projection.clone());
                        reference
                    });
                    through_reference
                        && analysis
                            .origins_for_borrowed_place(&state, dest)
                            .iter()
                            .any(|origin| {
                                matches!(
                                    origin,
                                    ReferenceOrigin::Param(_)
                                        | ReferenceOrigin::ParamContents(_)
                                        | ReferenceOrigin::ParamBorrowedContents(_)
                                )
                            })
                } else {
                    false
                };
                if let StatementKind::Assign(dest, rvalue) = &stmt.kind {
                    Self::report_temporary_reference_origins(
                        func,
                        diagnostics,
                        analysis.temporary_origins_for_assignment(&state, dest, rvalue),
                    );
                }
                analysis.apply_statement(&mut state, stmt);
                #[cfg(test)]
                allocator_trace_tests::integration_statement_after(
                    &analysis,
                    &state,
                    integration_before,
                    diagnostics,
                );
                if external_destination {
                    if let StatementKind::Assign(dest, _) = &stmt.kind {
                        if analysis
                            .backend_contract
                            .place_type_id(analysis.type_context, func, dest)
                            .is_some_and(|ty| {
                                type_id_contains_reference(
                                    TypeView::new(analysis.type_context),
                                    analysis.backend_contract,
                                    ty,
                                )
                            })
                            && analysis
                                .origins_for_place(&state, dest)
                                .iter()
                                .any(|origin| {
                                    matches!(
                                        origin,
                                        ReferenceOrigin::Local(_) | ReferenceOrigin::Temporary(_)
                                    )
                                })
                        {
                            diagnostics.push(mir_diagnostic(
                                "Cannot retain a reference to local storage in borrowed external storage".into(),
                                stmt.span.clone(),
                            ));
                        }
                    }
                }
            }

            if let Some(Terminator::Call {
                func: callable,
                args,
                destination,
                ..
            }) = &block.terminator
            {
                #[cfg(test)]
                let integration_diagnostic_start = diagnostics.0.len();
                Self::report_temporary_reference_origins(
                    func,
                    diagnostics,
                    analysis.temporary_origins_for_call(&state, callable, args, destination),
                );
                #[cfg(test)]
                allocator_trace_tests::integration_call_reject(
                    &analysis,
                    &state,
                    block_idx,
                    &diagnostics.0[integration_diagnostic_start..],
                );
            }
        }
    }

    fn report_temporary_reference_origins(
        func: &MirFunction,
        diagnostics: &mut Diagnostics,
        temporaries: impl IntoIterator<Item = Local>,
    ) {
        for temporary in temporaries {
            diagnostics.push(mir_diagnostic(
                "Cannot store reference to temporary value because it does not live long enough"
                    .to_string(),
                Self::origin_span(func, temporary),
            ));
        }
    }

    fn origin_span(func: &MirFunction, local: Local) -> Option<Span> {
        func.local_decls
            .get(local.0)
            .and_then(|decl| decl.span.clone())
    }

    fn validate_drop_obligations(
        func: &MirFunction,
        backend_contract: &MirBackendContract,
        diagnostics: &mut Diagnostics,
    ) {
        for obligation in &func.ownership.drop_obligations {
            if obligation.kind != DropObligationKind::Direct {
                continue;
            }
            if func
                .local_decls
                .get(obligation.place.local.0)
                .is_some_and(|local| {
                    obligation.place.projection.is_empty()
                        && local.source == LocalSource::ReturnPlace
                })
            {
                continue;
            }
            if backend_contract.drop_glue.contains_key(&obligation.ty) {
                continue;
            }

            diagnostics.push(mir_diagnostic(
                format!(
                    "Missing drop glue for a MIR value in function '{}'",
                    func.name
                ),
                func.local_decls
                    .get(obligation.place.local.0)
                    .and_then(|local| local.span.clone()),
            ));
        }
    }

    fn check_statement(
        stmt: &StatementData,
        init_analysis: &InitializationAnalysis,
        state: &InitMap,
        func: &MirFunction,
        move_validation: &mut MoveValidationContext<'_>,
        diagnostics: &mut Diagnostics,
    ) {
        let stmt_span = stmt.span.clone();

        match &stmt.kind {
            StatementKind::Assign(_dest, rvalue) => match rvalue {
                Rvalue::Package(operand, _) | Rvalue::Object(operand, _) | Rvalue::Use(operand) => {
                    match operand {
                        Operand::Move(place) => move_validation.validate_move(
                            place,
                            func,
                            stmt_span.clone(),
                            diagnostics,
                        ),
                        Operand::Copy(place) => move_validation.validate_copy(
                            place,
                            func,
                            stmt_span.clone(),
                            diagnostics,
                        ),
                        Operand::Constant(_) => {}
                    }
                    if let Err(e) = init_analysis.check_operand_at_path(operand, state) {
                        diagnostics.push(Self::make_error(e, func, operand, stmt_span));
                    }
                }
                Rvalue::Ref(_mutability, place) => {
                    if let Err(e) = init_analysis.check_place_at_path(place, state) {
                        diagnostics.push(Self::make_error(
                            e,
                            func,
                            &Operand::Copy(place.clone()),
                            stmt_span,
                        ));
                    }
                }
                Rvalue::Cast(op, _) => {
                    if let Err(e) = init_analysis.check_operand_at_path(op, state) {
                        diagnostics.push(Self::make_error(e, func, op, stmt_span));
                    }
                }
                Rvalue::BinaryOp(_, a, b) => {
                    if let Err(e) = init_analysis.check_operand_at_path(a, state) {
                        diagnostics.push(Self::make_error(e, func, a, stmt_span.clone()));
                    }
                    if let Err(e) = init_analysis.check_operand_at_path(b, state) {
                        diagnostics.push(Self::make_error(e, func, b, stmt_span));
                    }
                }
                Rvalue::UnaryOp(_, a) => {
                    if let Err(e) = init_analysis.check_operand_at_path(a, state) {
                        diagnostics.push(Self::make_error(e, func, a, stmt_span));
                    }
                }
                Rvalue::Discriminant(place) => {
                    // Structural enum cleanup reads discriminants behind drop flags.
                    // Initialization state alone cannot prove those drop-flag branches
                    // unreachable after a move, so only skip explicitly marked cleanup reads.
                    if !stmt.cleanup {
                        if let Err(e) = init_analysis.check_discriminant_at_path(place, state) {
                            diagnostics.push(Self::make_error(
                                e,
                                func,
                                &Operand::Copy(place.clone()),
                                stmt_span,
                            ));
                        }
                    }
                }
                Rvalue::Aggregate(_, operands) => {
                    for op in operands {
                        if let Err(e) = init_analysis.check_operand_at_path(op, state) {
                            diagnostics.push(Self::make_error(e, func, op, stmt_span.clone()));
                        }
                    }
                }
                Rvalue::Closure(closure) => {
                    for capture in &closure.captures {
                        if matches!(CaptureKind::from(capture.kind), CaptureKind::Move) {
                            continue;
                        }
                        let capture_operand = Operand::Copy(capture.place());
                        if let Err(e) = init_analysis.check_operand_at_path(&capture_operand, state)
                        {
                            diagnostics.push(Self::make_error(
                                e,
                                func,
                                &capture_operand,
                                stmt_span.clone(),
                            ));
                        }
                    }
                }
            },
            StatementKind::Assert(assertion) => {
                for operand in &assertion.operands {
                    if let Err(e) = init_analysis.check_operand_at_path(operand, state) {
                        diagnostics.push(Self::make_error(e, func, operand, stmt_span.clone()));
                    }
                }
            }
            _ => {}
        }
    }

    fn check_statement_loans(
        stmt: &StatementData,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        table: &LoanTable,
        active_loans: &LoanState,
        func: &MirFunction,
        diagnostics: &mut Diagnostics,
    ) {
        let assign_rvalue = if let StatementKind::Assign(dest, rvalue) = &stmt.kind {
            if !matches!(rvalue, Rvalue::Ref(_, _)) {
                Self::check_place_loan_access(
                    dest,
                    LoanKind::Mut,
                    table,
                    active_loans,
                    func,
                    diagnostics,
                    stmt.span.clone(),
                );
            }

            Some(rvalue)
        } else {
            None
        };

        for event in accesses::classify_statement(stmt, type_context) {
            match event.kind {
                AccessKind::BorrowShared | AccessKind::ReturnedShared => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Shared,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        stmt.span.clone(),
                    );
                }
                AccessKind::BorrowMut | AccessKind::ReturnedMut => {
                    if !event
                        .place
                        .projection
                        .iter()
                        .any(|projection| matches!(projection, crate::mir::Projection::Deref))
                        && event.place.local.0 < func.local_decls.len()
                        && func.local_decls[event.place.local.0]
                            .source
                            .reports_immutable_mut_borrow()
                        && func.local_decls[event.place.local.0].mutability == Mutability::Not
                    {
                        let name = func.local_decls[event.place.local.0]
                            .name
                            .clone()
                            .unwrap_or_else(|| "unnamed binding".to_string());
                        diagnostics.push(mir_diagnostic(
                            format!(
                                "Cannot take a mutable reference to immutable binding '{}' for mutable receiver",
                                name
                            ),
                            stmt.span.clone(),
                        ));
                    }

                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Mut,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        stmt.span.clone(),
                    );
                }
                AccessKind::Read => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Shared,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        stmt.span.clone(),
                    );
                }
                AccessKind::Move => {
                    Self::check_place_loan_access(
                        &event.place,
                        Self::move_access_kind(type_context, backend_contract, func, &event.place),
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        stmt.span.clone(),
                    );
                }
                AccessKind::CaptureShared => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Shared,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        stmt.span.clone(),
                    );
                }
                AccessKind::CaptureMut => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Mut,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        stmt.span.clone(),
                    );
                }
                AccessKind::RawPointerCast => {
                    if let Some(Rvalue::Cast(op, _)) = assign_rvalue {
                        match op {
                            Operand::Copy(place) => Self::check_place_loan_access(
                                place,
                                LoanKind::Shared,
                                table,
                                active_loans,
                                func,
                                diagnostics,
                                stmt.span.clone(),
                            ),
                            Operand::Move(place) => Self::check_place_loan_access(
                                place,
                                LoanKind::Mut,
                                table,
                                active_loans,
                                func,
                                diagnostics,
                                stmt.span.clone(),
                            ),
                            Operand::Constant(_) => {}
                        }
                    }
                }
                AccessKind::CaptureMove | AccessKind::Write | AccessKind::Drop => {}
            }
        }
    }

    fn check_terminator(
        term: &Terminator,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        init_analysis: &InitializationAnalysis,
        state: &InitMap,
        table: &LoanTable,
        active_loans: &LoanState,
        func: &MirFunction,
        diagnostics: &mut Diagnostics,
    ) {
        let term_span = term.origin().source_span().cloned();
        // Cleanup links the actual exiting operation, not an owner declaration.
        // Other synthetic origins still cannot authorize a diagnostic use span.
        let loan_use_span = match term.origin() {
            crate::mir::MirOrigin::Source(span) => Some(span),
            crate::mir::MirOrigin::Synthetic {
                kind: crate::mir::MirSyntheticOrigin::Cleanup,
                origin,
            } => origin,
            _ => None,
        };
        for event in accesses::classify_terminator(term) {
            match event.kind {
                AccessKind::Read => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Shared,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        loan_use_span.clone(),
                    );
                }
                AccessKind::Move | AccessKind::Drop => {
                    Self::check_place_loan_access(
                        &event.place,
                        if event.kind == AccessKind::Move {
                            Self::move_access_kind(
                                type_context,
                                backend_contract,
                                func,
                                &event.place,
                            )
                        } else {
                            LoanKind::Mut
                        },
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        loan_use_span.clone(),
                    );
                }
                AccessKind::Write
                | AccessKind::BorrowShared
                | AccessKind::BorrowMut
                | AccessKind::ReturnedShared
                | AccessKind::ReturnedMut
                | AccessKind::CaptureShared
                | AccessKind::CaptureMut
                | AccessKind::CaptureMove
                | AccessKind::RawPointerCast => {}
            }
        }

        match term {
            Terminator::SwitchInt { discr, .. } | Terminator::SwitchIntWithOrigin { discr, .. } => {
                if let Err(e) = init_analysis.check_operand_at_path(discr, state) {
                    diagnostics.push(Self::make_error(e, func, discr, term_span.clone()));
                }
            }
            Terminator::Call {
                func: op_func,
                args,
                ..
            } => {
                if let Err(e) = init_analysis.check_operand_at_path(op_func, state) {
                    diagnostics.push(Self::make_error(e, func, op_func, term_span.clone()));
                }
                for arg in args {
                    if let Err(e) = init_analysis.check_operand_at_path(arg, state) {
                        diagnostics.push(Self::make_error(e, func, arg, term_span.clone()));
                    }
                }
            }
            Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } => {
                if let Err(e) = init_analysis.check_drop_at_path(place, state) {
                    diagnostics.push(Self::make_error(
                        e,
                        func,
                        &Operand::Copy(place.clone()),
                        term_span,
                    ));
                }
            }
            _ => {}
        }
    }

    fn move_access_kind(
        type_context: &crate::type_context::TypeContext,
        contract: &MirBackendContract,
        func: &MirFunction,
        place: &Place,
    ) -> LoanKind {
        let mut prefix = Place {
            local: place.local,
            projection: Vec::new(),
        };
        let mut raw_read = false;
        for projection in &place.projection {
            if *projection == Projection::Deref {
                raw_read = contract
                    .place_type_id(type_context, func, &prefix)
                    .is_some_and(|id| matches!(type_context.try_ty(id), Some(Ty::Pointer(_))));
            }
            prefix.projection.push(projection.clone());
        }
        // A raw-pointer read transfers the loaded value, not the initialization
        // of an aliased safe place. Writes/drops and safe-reference moves retain
        // their exclusive-access checks; unsafe admission remains frontend-owned.
        if raw_read {
            LoanKind::Shared
        } else {
            LoanKind::Mut
        }
    }

    fn check_place_loan_access(
        place: &crate::mir::Place,
        kind: LoanKind,
        table: &LoanTable,
        active_loans: &LoanState,
        _func: &MirFunction,
        diagnostics: &mut Diagnostics,
        stmt_span: Option<Span>,
    ) {
        #[cfg(test)]
        allocator_trace_tests::access(_func, place, kind, table, active_loans, stmt_span.as_ref());
        if let Err(conflicting_loan_id) =
            LoanAnalysis::check_aliasing(place, kind, table, active_loans)
        {
            #[cfg(test)]
            if allocator_trace_tests::enabled_at(stmt_span.as_ref()) {
                eprintln!(
                    "OWNER_LOAN_TRACE actual-conflict function={:?} span={:?} place={:?} kind={:?} loan={:?} loan_data={:?}",
                    _func.id, stmt_span, place, kind, conflicting_loan_id,
                    table.get(conflicting_loan_id),
                );
            }
            let use_span = stmt_span;
            let borrow_span = table
                .get(conflicting_loan_id)
                .and_then(|loan| loan.origin_span.clone());
            diagnostics.push(crate::mir::borrowck::diagnostics::borrow_conflict(
                place,
                use_span,
                borrow_span,
            ));
        }
    }

    fn make_error(
        error: InitError,
        func: &MirFunction,
        operand: &Operand,
        stmt_span: Option<Span>,
    ) -> Diagnostic {
        let span = stmt_span;
        let is_move_error = error.message.contains("moved");
        let move_span = error.move_span.clone();

        let msg = match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                if place.local.0 < func.local_decls.len() {
                    if let Some(name) = &func.local_decls[place.local.0].name {
                        format!("{} `{name}`", error.message)
                    } else {
                        error.message.clone()
                    }
                } else {
                    error.message.clone()
                }
            }
            _ => error.message.clone(),
        };

        if span.is_none() || (is_move_error && move_span.is_none()) {
            return mir_diagnostic(msg, None);
        }

        let label_msg = match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                if place.local.0 < func.local_decls.len() {
                    if let Some(name) = &func.local_decls[place.local.0].name {
                        if move_span.is_some() {
                            format!("`{}` used here after move", name)
                        } else {
                            format!("`{}` used here", name)
                        }
                    } else {
                        "used here".to_string()
                    }
                } else {
                    "used here".to_string()
                }
            }
            _ => "here".to_string(),
        };

        let mut diagnostic = match span {
            Some(span) => borrow_error(msg, span.clone()).with_label(label_msg, span),
            None => mir_diagnostic(msg, None),
        };

        if let Some(move_span) = move_span {
            let move_label_msg = match operand {
                Operand::Copy(place) | Operand::Move(place) => {
                    if place.local.0 < func.local_decls.len() {
                        if let Some(name) = &func.local_decls[place.local.0].name {
                            format!("value moved from `{}` here", name)
                        } else {
                            "value moved here".to_string()
                        }
                    } else {
                        "value moved here".to_string()
                    }
                }
                _ => "value moved here".to_string(),
            };
            diagnostic = diagnostic.with_label(move_label_msg, move_span);
        }

        diagnostic
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};

    use super::{
        contract_struct_fields, return_type_needs_escape_check, type_id_contains_reference,
        type_id_is_mut_reference, type_id_is_pointer, type_id_tracks_origin, MissingSummaryMode,
        ReferenceEscapeAnalysis, ReferenceOriginState,
    };
    use crate::diagnostic::{DiagnosticCode, Diagnostics};
    use crate::ids::{AssocTypeId, CrateId, DefId, InstanceId, LocalDefId};
    use crate::lexer::Span;
    use crate::mir::borrowck::BorrowChecker;
    use crate::mir::dataflow::run_fixpoint;
    use crate::mir::{
        BasicBlock, Constant, DropObligationKind, Local, LocalDecl, LocalSource, MirAssert,
        MirAssertKind, MirBackendContract, MirCallable, MirCallableKey, MirClosure,
        MirClosureCapture, MirClosureCaptureKind, MirClosureId, MirDropObligation,
        MirEnumVariantLayout, MirFunction, MirFunctionId, MirNominalLayout, MirOwnershipMetadata,
        MirProgram, MirProjectionKey, MirVariantLayoutFields, Mutability, Operand, Place,
        Projection, ReferenceOrigin, Rvalue, StatementData, Terminator,
    };
    use crate::type_context::{TypeContext, TypeView};
    use crate::types::{GenericParamId, Type};

    fn test_type_id(type_context: &mut TypeContext, ty: Type) -> crate::ids::TypeId {
        type_context.intern_type(&ty)
    }

    #[test]
    fn nominal_result_storage_is_separate_from_identity_and_checked_against_its_producer() {
        let mut program = enum_origin_source_program(
            r#"
struct Holder
    < value: () -> I64
< trait CopyValue
    ~@into_value: I64
impl CopyValue for I64
    ~@into_value = -> self
make: I64 -> Holder
make = seed ->
    Holder
        value: -> seed.into_value!
main = !->
    held = make 42
"#,
        );
        BorrowChecker::run(&program).unwrap();
        let (key, storage) = program
            .backend_contract
            .native_result_storage
            .iter()
            .find(|(_, storage)| {
                matches!(
                    program.type_context.type_for(**storage),
                    Type::StructRepresentation { .. }
                )
            })
            .map(|(key, storage)| (key.clone(), *storage))
            .unwrap();
        let declaration = &program.backend_contract.callables[&key];
        let crate::mir::MirCallableKind::LocalBody { function_id } = &declaration.kind else {
            panic!("storage requires its actual local body");
        };
        let function = &program.functions[function_id];
        assert_eq!(declaration.signature.ret.semantic_ty, function.ret_type);
        assert_eq!(function.local_decls[0].ty, function.ret_type);
        assert!(matches!(
            program.type_context.type_for(function.ret_type),
            Type::Struct { .. }
        ));
        assert_eq!(
            program.backend_contract.place_type_id(
                &program.type_context,
                function,
                &Place {
                    local: Local(0),
                    projection: vec![]
                }
            ),
            Some(storage)
        );
        let mut forged = program.type_context.type_for(storage);
        let Type::StructRepresentation { fields, .. } = &mut forged else {
            unreachable!()
        };
        let Type::Function { captures, .. } = &mut fields[0] else {
            unreachable!()
        };
        assert!(!captures.is_empty());
        captures[0].ty = Type::Tuple(vec![Type::I64, Type::I64]);
        let forged = program.type_context.intern_type(&forged);
        program
            .backend_contract
            .native_result_storage
            .insert(key.clone(), forged);
        let report = crate::mir::agreement::check_mir_runtime_agreement(&program);
        assert!(report.invalid_backend_contract > 0, "{report:?}");
        program.backend_contract.native_result_storage.remove(&key);
        let report = crate::mir::agreement::check_mir_runtime_agreement(&program);
        assert!(
            !report.is_clean(),
            "forgetting measured storage must not erase the obligation: {report:?}"
        );
        program
            .backend_contract
            .native_result_storage
            .insert(key, crate::ids::TypeId(999_999));
        let report = crate::mir::agreement::check_mir_runtime_agreement(&program);
        assert!(report.invalid_type_ids > 0, "{report:?}");
    }

    #[test]
    fn enum_result_storage_requires_exact_producer_and_variant_schema() {
        let original = enum_origin_source_program(
            r#"
struct Holder
    < value: () -> I64
trait CopyValue
    ~@into_value: I64
impl CopyValue for I64
    ~@into_value = -> self
enum Envelope
    Held Holder
make: I64 -> Envelope
make = seed ->
    held = Holder
        value: -> seed.into_value!
    Envelope::Held held
main = !->
    wrapped = make 42
"#,
        );
        BorrowChecker::run(&original).unwrap();
        let (key, storage) = original
            .backend_contract
            .native_result_storage
            .iter()
            .find(|(_, storage)| {
                matches!(
                    original.type_context.type_for(**storage),
                    Type::EnumRepresentation { .. }
                )
            })
            .map(|(key, storage)| (key.clone(), *storage))
            .unwrap();
        let declaration = &original.backend_contract.callables[&key];
        let crate::mir::MirCallableKind::LocalBody { function_id } = &declaration.kind else {
            panic!("measured local producer");
        };
        let function = &original.functions[function_id];
        assert!(matches!(
            original.type_context.type_for(function.ret_type),
            Type::Enum { .. }
        ));
        assert_eq!(declaration.signature.ret.semantic_ty, function.ret_type);
        assert_eq!(function.local_decls[0].ty, function.ret_type);
        for corruption in 0..5 {
            let mut program = original.clone();
            let mut forged = program.type_context.type_for(storage);
            let Type::EnumRepresentation { variants, .. } = &mut forged else {
                unreachable!()
            };
            match corruption {
                0 => {
                    let Type::StructRepresentation { fields, .. } = &mut variants[0][0] else {
                        panic!("measured Holder");
                    };
                    let Type::Function { captures, .. } = &mut fields[0] else {
                        panic!("measured callable");
                    };
                    assert!(!captures.is_empty());
                    captures[0].ty = Type::Tuple(vec![Type::I64, Type::I64]);
                }
                1 => variants.push(vec![]),
                2 => variants[0].clear(),
                _ => {}
            }
            if corruption < 3 {
                let forged = program.type_context.intern_type(&forged);
                program
                    .backend_contract
                    .native_result_storage
                    .insert(key.clone(), forged);
            } else if corruption == 3 {
                program.backend_contract.native_result_storage.remove(&key);
            } else {
                program
                    .backend_contract
                    .native_result_storage
                    .insert(key.clone(), crate::ids::TypeId(999_999));
            }
            let report = crate::mir::agreement::check_mir_runtime_agreement(&program);
            assert!(!report.is_clean(), "corruption {corruption}: {report:?}");
        }
    }

    #[test]
    fn measured_enum_payload_does_not_grant_borrowed_capture_escape() {
        let program = enum_origin_source_program(
            r#"
struct Holder
    < value: () -> I64
enum Envelope
    Held Holder
make = ->
    local = 42
    held = Holder
        value: -> local
    Envelope::Held held
main = !->
    wrapped = make!
"#,
        );
        assert!(program
            .backend_contract
            .native_result_storage
            .values()
            .any(|storage| matches!(
                program.type_context.type_for(*storage),
                Type::EnumRepresentation { .. }
            )));
        let diagnostics = BorrowChecker::run(&program)
            .expect_err("measured payload is not a borrowing certificate");
        assert!(
            diagnostics.0.iter().any(
                |diagnostic| diagnostic.code == Some(crate::diagnostic::DiagnosticCode::Borrow)
            ),
            "{diagnostics:?}"
        );
        assert!(
            diagnostics
                .0
                .iter()
                .all(|diagnostic| diagnostic.code
                    != Some(crate::diagnostic::DiagnosticCode::Internal)),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn measured_borrowed_enum_pattern_does_not_authorize_moving_its_nominal_payload() {
        let program = enum_origin_source_program(
            r#"
trait CopyValue
    ~@into_value: I64
impl CopyValue for I64
    ~@into_value = -> self
struct Holder
    < value: () -> I64
enum Envelope
    Held Holder
make = seed ->
    held = Holder
        value: -> seed.into_value!
    Envelope::Held held
take: &Envelope -> Holder
take = wrapped ->
    match *wrapped
        Envelope::Held value => value
main = !->
    wrapped = make 42
    stolen = take &wrapped
"#,
        );
        let diagnostics = BorrowChecker::run(&program)
            .expect_err("borrowed measured payload is not an owned Holder");
        assert!(
            diagnostics.0.iter().any(|diagnostic| diagnostic.code
                == Some(crate::diagnostic::DiagnosticCode::Borrow)
                && diagnostic.message.contains("out of a reference")),
            "{diagnostics:?}"
        );
        assert!(
            diagnostics
                .0
                .iter()
                .all(|diagnostic| diagnostic.code
                    != Some(crate::diagnostic::DiagnosticCode::Internal)),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn static_nominal_factory_requires_exact_measured_result_storage_with_canonical_dictionary() {
        let source = format!(
            "{}{}",
            crate::mir::static_dictionary::tests::PRODUCER,
            r#"
< trait CopyValue
    ~@into_value: I64
impl CopyValue for I64
    ~@into_value = -> self
struct Holder
    < value: () -> I64
impl Factory for Holder
    make = seed ->
        Holder
            value: -> seed.into_value!
main = !->
    service = Service
        marker: 0
    object = as_dispatch &service
    held: Holder = object.build 42
"#
        );
        let mut program = crate::mir::static_dictionary::tests::accepted_program(&source);
        let (owner, block, call, destination) = program
            .functions
            .iter()
            .find_map(|(owner, body)| {
                body.basic_blocks
                    .iter()
                    .enumerate()
                    .find_map(|(index, block)| {
                        let Some(Terminator::Call {
                            func: Operand::Constant(Constant::ErasedCall(call)),
                            destination,
                            ..
                        }) = &block.terminator
                        else {
                            return None;
                        };
                        if !matches!(call.target, crate::mir::MirErasedCallTarget::Virtual(_)) {
                            return None;
                        }
                        Some((owner.clone(), index, call.clone(), destination.clone()))
                    })
            })
            .unwrap();
        let dictionary = &program.backend_contract.erased.dictionaries[&call.dictionaries[0]];
        let canonical = dictionary.subject;
        let represented = call.type_args[0];
        assert!(matches!(
            program.type_context.type_for(canonical),
            Type::Struct { .. }
        ));
        assert!(matches!(
            program.type_context.type_for(represented),
            Type::StructRepresentation { .. }
        ));
        assert_eq!(
            program.backend_contract.normalize_type(
                &program.type_context,
                &program.type_context.type_for(represented)
            ),
            program.type_context.type_for(canonical)
        );
        assert!(crate::mir::native_dictionary_keys(
            &program.backend_contract,
            &program.type_context,
            call.dictionaries[0]
        )
        .is_ok());
        let body = program.functions.get_mut(&owner).unwrap();
        let Some(Terminator::Call {
            func: Operand::Constant(Constant::ErasedCall(actual)),
            ..
        }) = &mut body.basic_blocks[block].terminator
        else {
            unreachable!()
        };
        actual.type_args[0] = canonical;
        body.local_decls[destination.local.0].ty = canonical;
        let report = crate::mir::agreement::check_mir_runtime_agreement(&program);
        assert!(
            report
                .erased_contract_errors
                .iter()
                .any(|(_, error)| *error == crate::mir::MirErasedError::InvalidDictionary),
            "{report:?}"
        );
    }

    #[test]
    fn nominal_mutation_retains_borrowed_callable_fields_after_layout_specialization() {
        for borrowed in [false, true] {
            let signature = if borrowed { "&Holder" } else { "Holder" };
            let argument = if borrowed { "&holder" } else { "holder" };
            let source = format!(
                r#"
struct Holder
    < callback: () -> I64
struct Cell T
    < payload: T
store: &mut (Cell Holder) -> {signature} -> ()
store = dest, source ->
    dest.payload = Holder
        callback: source.callback
    ()
escape: () -> Cell Holder
escape = ->
    local = 42
    view = &local
    callback = -> *view
    holder = Holder
        callback: callback
    mut cell = Cell
        payload: Holder
            callback: -> 0
    store &mut cell, {argument}
    cell
main = !->
    result = escape!
"#
            );
            let program = enum_origin_source_program(&source);
            let diagnostics = BorrowChecker::run(&program)
                .expect_err("a physical field layout cannot erase its retained local loan");
            assert!(diagnostics.0.iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message
                        == "Cannot return reference to local value because it does not live long enough"
                    && diagnostic.primary.is_some()
            }), "{diagnostics:?}");
        }
    }

    #[test]
    fn borrowed_field_value_and_field_address_have_distinct_symbolic_origins() {
        let mut tc = TypeContext::new();
        let reference = Type::Reference {
            inner: Box::new(Type::I64),
            mutable: false,
        };
        let container = Type::Tuple(vec![reference.clone()]);
        let borrowed_container = tc.intern_type(&Type::Reference {
            inner: Box::new(container),
            mutable: false,
        });
        let mut function = reference_return_function(&mut tc, MirOwnershipMetadata::default());
        function.arg_count = 1;
        function.local_decls[1].ty = borrowed_container;
        function.local_decls[1].source = LocalSource::Argument;
        let field = Place {
            local: Local(1),
            projection: vec![
                Projection::Deref,
                Projection::Field {
                    index: 0,
                    identity: None,
                },
            ],
        };
        let destination = Place {
            local: Local(0),
            projection: vec![],
        };
        function.basic_blocks[0].statements = vec![StatementData::assign(
            destination.clone(),
            Rvalue::Use(Operand::Copy(field.clone())),
            None,
        )];
        assert_eq!(
            BorrowChecker::reference_return_origins(
                &function,
                &tc,
                &MirBackendContract::default(),
                &HashMap::new(),
            ),
            HashSet::from([ReferenceOrigin::ParamContents(Local(1))]),
        );
        let address = tc.intern_type(&Type::Reference {
            inner: Box::new(reference),
            mutable: false,
        });
        function.ret_type = address;
        function.local_decls[0].ty = address;
        function.basic_blocks[0].statements = vec![StatementData::assign(
            destination,
            Rvalue::Ref(Mutability::Not, field),
            None,
        )];
        assert_eq!(
            BorrowChecker::reference_return_origins(
                &function,
                &tc,
                &MirBackendContract::default(),
                &HashMap::new(),
            ),
            HashSet::from([ReferenceOrigin::Param(Local(1))]),
        );
    }

    #[test]
    fn consuming_aggregate_return_distinguishes_contained_borrow_from_owned_storage() {
        let mut tc = TypeContext::new();
        let reference = Type::Reference {
            inner: Box::new(Type::I64),
            mutable: false,
        };
        let aggregate = tc.intern_type(&Type::Tuple(vec![reference.clone()]));
        let mut function = reference_return_function(&mut tc, MirOwnershipMetadata::default());
        function.arg_count = 1;
        function.local_decls[1].ty = aggregate;
        function.local_decls[1].source = crate::mir::LocalSource::Argument;
        let field = Place {
            local: Local(1),
            projection: vec![Projection::Field {
                index: 0,
                identity: None,
            }],
        };
        let destination = Place {
            local: Local(0),
            projection: vec![],
        };
        function.basic_blocks[0].statements = vec![StatementData::assign(
            destination.clone(),
            Rvalue::Use(Operand::Copy(field.clone())),
            None,
        )];
        let origins = BorrowChecker::reference_return_origins(
            &function,
            &tc,
            &MirBackendContract::default(),
            &HashMap::new(),
        );
        assert_eq!(origins, HashSet::from([ReferenceOrigin::Param(Local(1))]));

        let borrowed_storage = tc.intern_type(&Type::Reference {
            inner: Box::new(reference),
            mutable: false,
        });
        function.ret_type = borrowed_storage;
        function.local_decls[0].ty = borrowed_storage;
        function.basic_blocks[0].statements = vec![StatementData::assign(
            destination,
            Rvalue::Ref(Mutability::Not, field),
            None,
        )];
        let origins = BorrowChecker::reference_return_origins(
            &function,
            &tc,
            &MirBackendContract::default(),
            &HashMap::new(),
        );
        assert_eq!(origins, HashSet::from([ReferenceOrigin::Local(Local(1))]));
    }

    #[test]
    fn raw_owner_parts_transport_parameter_origins_without_borrowing_owner_storage() {
        let mut tc = TypeContext::new();
        let pointer_ty = Type::Pointer(Box::new(Type::I64));
        let owner = tc.intern_type(&Type::Tuple(vec![pointer_ty.clone()]));
        let parts = tc.intern_type(&Type::Tuple(vec![pointer_ty.clone(), Type::Unit]));
        let pointer = tc.intern_type(&pointer_ty);
        let unit = tc.intern_type(&Type::Unit);
        let mut split = reference_return_function(&mut tc, MirOwnershipMetadata::default());
        split.arg_count = 1;
        split.ret_type = parts;
        split.local_decls[0].ty = parts;
        split.local_decls[1].ty = owner;
        split.local_decls[1].source = LocalSource::Argument;
        split.local_decls.truncate(2);
        let local = |ty, source| LocalDecl {
            ty,
            source,
            mutability: Mutability::Not,
            name: None,
            span: None,
        };
        split
            .local_decls
            .push(local(parts, LocalSource::UserBinding));
        split
            .local_decls
            .push(local(pointer, LocalSource::Temporary));
        split.local_decls.push(local(unit, LocalSource::Temporary));
        let place = |index| Place {
            local: Local(index),
            projection: vec![],
        };
        let field = Place {
            local: Local(1),
            projection: vec![Projection::Field {
                index: 0,
                identity: None,
            }],
        };
        split.basic_blocks[0].statements = vec![
            StatementData::assign(place(3), Rvalue::Use(Operand::Copy(field.clone())), None),
            StatementData::assign(
                place(4),
                Rvalue::Use(Operand::Constant(Constant::Unit)),
                None,
            ),
            StatementData::assign(
                place(2),
                Rvalue::Aggregate(
                    crate::mir::AggregateKind::Tuple,
                    vec![Operand::Copy(place(3)), Operand::Copy(place(4))],
                ),
                None,
            ),
            StatementData::assign(place(0), Rvalue::Use(Operand::Move(place(2))), None),
        ];
        let contract = MirBackendContract::default();
        let origins =
            BorrowChecker::reference_return_origins(&split, &tc, &contract, &HashMap::new());
        assert_eq!(origins, HashSet::from([ReferenceOrigin::Param(Local(1))]));

        // A caller's hidden stack origin must survive substitution of that
        // parameter summary; the transferred raw handle is not borrow-free.
        let summaries = HashMap::from([(split.id.clone(), origins)]);
        let mut caller = split.clone();
        caller.arg_count = 0;
        caller.local_decls[1].source = LocalSource::UserBinding;
        let analysis = ReferenceEscapeAnalysis::new(
            &caller,
            &tc,
            &contract,
            &summaries,
            MissingSummaryMode::Conservative,
        );
        let mut state = ReferenceOriginState::default();
        state.origins.insert(
            field.clone(),
            HashSet::from([ReferenceOrigin::Local(Local(1))]),
        );
        let MirFunctionId::Function(split_id) = &split.id else {
            unreachable!()
        };
        let callee = Operand::Constant(Constant::Callable(MirCallable::Resolved(
            MirCallableKey::Function(*split_id),
        )));
        assert_eq!(
            analysis.call_return_origins(&state, &callee, &[Operand::Move(place(1))], &place(0)),
            HashSet::from([ReferenceOrigin::Local(Local(1))])
        );

        let borrowed_storage = tc.intern_type(&Type::Reference {
            inner: Box::new(pointer_ty),
            mutable: false,
        });
        split.ret_type = borrowed_storage;
        split.local_decls[0].ty = borrowed_storage;
        split.basic_blocks[0].statements = vec![StatementData::assign(
            place(0),
            Rvalue::Ref(Mutability::Not, field),
            None,
        )];
        assert_eq!(
            BorrowChecker::reference_return_origins(&split, &tc, &contract, &HashMap::new()),
            HashSet::from([ReferenceOrigin::Local(Local(1))])
        );
    }

    #[test]
    fn borrowck_type_queries_use_type_view() {
        let mut context = crate::type_context::TypeContext::new();
        let i64_id = context.intern_type(&Type::I64);
        let mut_ref = context.intern_ty(crate::type_context::Ty::Reference {
            mutable: true,
            inner: i64_id,
        });
        let pointer = context.intern_ty(crate::type_context::Ty::Pointer(i64_id));
        let view = crate::type_context::TypeView::new(&context);

        assert!(type_id_is_mut_reference(view, mut_ref));
        assert!(type_id_is_pointer(view, pointer));
        assert!(!type_id_is_pointer(view, mut_ref));
    }

    #[test]
    fn concrete_generic_enum_fields_do_not_inherit_generic_reference_facts() {
        let mut type_context = TypeContext::new();
        let result_id = DefId::new(CrateId(0), LocalDefId(601));
        let t_param = GenericParamId {
            owner: result_id,
            index: 0,
        };
        let e_param = GenericParamId {
            owner: result_id,
            index: 1,
        };
        let generic_t = test_type_id(&mut type_context, Type::Generic(t_param));
        let generic_e = test_type_id(&mut type_context, Type::Generic(e_param));
        let concrete_result = test_type_id(
            &mut type_context,
            Type::Enum {
                id: result_id,
                args: vec![Type::I64, Type::I32],
            },
        );
        let reference_result = test_type_id(
            &mut type_context,
            Type::Enum {
                id: result_id,
                args: vec![
                    Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::I64),
                    },
                    Type::I32,
                ],
            },
        );
        let mut backend_contract = MirBackendContract::default();
        backend_contract.nominal_layouts.insert(
            result_id,
            MirNominalLayout::Enum {
                id: result_id,
                variants: vec![
                    MirEnumVariantLayout {
                        name: "Ok".to_string(),
                        fields: MirVariantLayoutFields::Positional(vec![generic_t]),
                    },
                    MirEnumVariantLayout {
                        name: "Err".to_string(),
                        fields: MirVariantLayoutFields::Positional(vec![generic_e]),
                    },
                ],
                generic_params: vec![t_param, e_param],
            },
        );
        let view = TypeView::new(&type_context);

        assert!(!type_id_contains_reference(
            view,
            &backend_contract,
            concrete_result
        ));
        assert!(!type_id_tracks_origin(
            view,
            &backend_contract,
            concrete_result
        ));
        assert!(type_id_contains_reference(
            view,
            &backend_contract,
            reference_result
        ));
        assert!(type_id_tracks_origin(
            view,
            &backend_contract,
            reference_result
        ));
    }

    #[test]
    fn borrowck_rejects_local_escape_through_contract_projection_output() {
        let mut type_context = TypeContext::new();
        let trait_id = DefId::new(CrateId(0), LocalDefId(701));
        let assoc_type_id = AssocTypeId(0);
        let base_id = test_type_id(&mut type_context, Type::I64);
        let ref_i64 = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };
        let ref_i64_id = test_type_id(&mut type_context, ref_i64);
        let projection_id = test_type_id(
            &mut type_context,
            Type::Projection {
                ty: Box::new(Type::I64),
                trait_id,
                assoc_type: crate::types::AssociatedTypeKey {
                    owner: trait_id,
                    assoc_type_id,
                },
                trait_args: Vec::new(),
            },
        );
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(702))),
            name: "projection_reference_return".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::assign(
                        Place {
                            local: Local(1),
                            projection: Vec::new(),
                        },
                        Rvalue::Use(Operand::Constant(Constant::Int(1))),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: Vec::new(),
                        },
                        Rvalue::Ref(
                            Mutability::Not,
                            Place {
                                local: Local(1),
                                projection: Vec::new(),
                            },
                        ),
                        None,
                    ),
                ],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: projection_id,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: base_id,
                    mutability: Mutability::Not,
                    name: Some("local".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: projection_id,
            ownership: MirOwnershipMetadata::default(),
        };
        let mut backend_contract = MirBackendContract::default();
        backend_contract.projection_outputs.insert(
            MirProjectionKey {
                base: base_id,
                trait_id,
                assoc_type_id,
                trait_args: Vec::new(),
            },
            ref_i64_id,
        );

        let err = BorrowChecker::check_function_with_contract(
            &function,
            &type_context,
            &backend_contract,
            &HashMap::new(),
        )
        .expect_err("projection output reference should not allow a local to escape");

        assert!(err.0.iter().any(|diag| diag
            .message
            .contains("Cannot return reference to local value")));
    }

    #[test]
    fn borrowck_uses_indexed_loan_state_for_active_conflicts() {
        let mut type_context = TypeContext::new();
        let func = mut_borrow_then_shared_read_function(&mut type_context);
        let err = BorrowChecker::check_function(&func, &type_context).expect_err("borrow conflict");

        assert!(err.0.iter().any(|diag| {
            diag.message.contains("Cannot borrow") || diag.message.contains("borrow")
        }));
    }

    #[test]
    fn immutable_binding_mut_borrow_diagnostic_ignores_display_name_presence() {
        let mut type_context = TypeContext::new();
        let named = immutable_binding_mut_borrow_function(&mut type_context, Some("value"));
        let unnamed = immutable_binding_mut_borrow_function(&mut type_context, None);

        for function in [&named, &unnamed] {
            let err = BorrowChecker::check_function(function, &type_context)
                .expect_err("mutable borrow of immutable binding should be rejected");
            assert!(err.0.iter().any(|diag| diag
                .message
                .contains("Cannot take a mutable reference to immutable binding")));
        }
    }

    #[test]
    fn direct_drop_obligation_without_glue_is_rejected() {
        let mut type_context = TypeContext::new();
        let tracked_id = test_type_id(&mut type_context, Type::I64);
        let function = drop_obligation_function(
            tracked_id,
            MirOwnershipMetadata {
                reference_origins: Vec::new(),
                temporary_locals: Vec::new(),
                drop_obligations: vec![MirDropObligation {
                    place: Place {
                        local: Local(1),
                        projection: Vec::new(),
                    },
                    ty: tracked_id,
                    kind: DropObligationKind::Direct,
                }],
            },
        );

        let err = BorrowChecker::check_function(&function, &type_context)
            .expect_err("direct drop without glue should be rejected");

        assert!(err
            .0
            .iter()
            .any(|diag| diag.message.contains("Missing drop glue")));
    }

    #[test]
    fn direct_drop_obligation_uses_backend_contract_drop_glue() {
        let mut type_context = TypeContext::new();
        let tracked_id = test_type_id(&mut type_context, Type::I64);
        let function = drop_obligation_function(
            tracked_id,
            MirOwnershipMetadata {
                reference_origins: Vec::new(),
                temporary_locals: Vec::new(),
                drop_obligations: vec![MirDropObligation {
                    place: Place {
                        local: Local(1),
                        projection: Vec::new(),
                    },
                    ty: tracked_id,
                    kind: DropObligationKind::Direct,
                }],
            },
        );
        let mut backend_contract = MirBackendContract::default();
        backend_contract.drop_glue.insert(
            tracked_id,
            crate::mir::MirCallableKey::Function(DefId::new(CrateId(0), LocalDefId(124))),
        );
        let program = MirProgram {
            functions: std::collections::BTreeMap::from([(function.id.clone(), function)]),
            type_context,
            backend_contract,
        };

        BorrowChecker::run(&program)
            .expect("contract drop glue should satisfy direct drop obligation");
    }

    #[test]
    fn projected_return_place_direct_drop_obligation_without_glue_is_rejected() {
        let mut type_context = TypeContext::new();
        let tracked_id = test_type_id(&mut type_context, Type::I64);
        let function = drop_obligation_function(
            tracked_id,
            MirOwnershipMetadata {
                reference_origins: Vec::new(),
                temporary_locals: Vec::new(),
                drop_obligations: vec![MirDropObligation {
                    place: Place {
                        local: Local(0),
                        projection: vec![Projection::Field {
                            index: 0,
                            identity: None,
                        }],
                    },
                    ty: tracked_id,
                    kind: DropObligationKind::Direct,
                }],
            },
        );

        let err = BorrowChecker::check_function(&function, &type_context)
            .expect_err("projected return-place direct drop without glue should be rejected");

        assert!(err
            .0
            .iter()
            .any(|diag| diag.message.contains("Missing drop glue")));
    }

    #[test]
    fn structural_drop_obligation_without_direct_glue_is_allowed() {
        let mut type_context = TypeContext::new();
        let tracked_id = test_type_id(&mut type_context, Type::I64);
        let function = drop_obligation_function(
            tracked_id,
            MirOwnershipMetadata {
                reference_origins: Vec::new(),
                temporary_locals: Vec::new(),
                drop_obligations: vec![MirDropObligation {
                    place: Place {
                        local: Local(1),
                        projection: Vec::new(),
                    },
                    ty: tracked_id,
                    kind: DropObligationKind::Structural,
                }],
            },
        );

        BorrowChecker::check_function(&function, &type_context)
            .expect("structural cleanup should not require direct drop glue");
    }

    #[test]
    fn borrowck_rejects_spanless_non_cleanup_discriminant_read_after_move() {
        let mut type_context = TypeContext::new();
        let enum_id = DefId::new(CrateId(0), LocalDefId(700));
        let enum_ty = Type::Enum {
            id: enum_id,
            args: Vec::new(),
        };
        let enum_ty_id = test_type_id(&mut type_context, enum_ty);
        let i64_id = test_type_id(&mut type_context, Type::I64);
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(701))),
            name: "spanless_discriminant_after_move".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::assign(
                        Place {
                            local: Local(2),
                            projection: Vec::new(),
                        },
                        Rvalue::Use(Operand::Move(Place {
                            local: Local(1),
                            projection: Vec::new(),
                        })),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(3),
                            projection: Vec::new(),
                        },
                        Rvalue::Discriminant(Place {
                            local: Local(1),
                            projection: Vec::new(),
                        }),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: Vec::new(),
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(3),
                            projection: Vec::new(),
                        })),
                        None,
                    ),
                ],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: enum_ty_id,
                    mutability: Mutability::Not,
                    name: Some("value".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
                LocalDecl {
                    ty: enum_ty_id,
                    mutability: Mutability::Not,
                    name: Some("moved".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Not,
                    name: Some("discriminant".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: i64_id,
            ownership: MirOwnershipMetadata::default(),
        };

        let err = BorrowChecker::check_function(&function, &type_context)
            .expect_err("spanless non-cleanup discriminant read after move should be rejected");
        assert!(err
            .0
            .iter()
            .any(|diag| diag.message.contains("borrow of moved value")));
        let diagnostic = err
            .0
            .iter()
            .find(|diag| diag.message.contains("borrow of moved value"))
            .expect("move diagnostic should be present");
        assert_eq!(diagnostic.code, Some(DiagnosticCode::Internal));
        assert_eq!(
            diagnostic.location,
            crate::diagnostic::DiagnosticLocation::Toolchain
        );
        assert!(diagnostic.primary.is_none());
        assert!(diagnostic.secondary.is_empty());
    }

    #[test]
    fn reference_escape_validator_uses_ownership_metadata() {
        let mut type_context = TypeContext::new();

        let local_escape = reference_return_function(
            &mut type_context,
            MirOwnershipMetadata {
                reference_origins: vec![(Local(0), ReferenceOrigin::Local(Local(1)))],
                temporary_locals: Vec::new(),
                drop_obligations: Vec::new(),
            },
        );
        let mut diagnostics = Diagnostics::default();
        BorrowChecker::validate_reference_escapes(
            &local_escape,
            &type_context,
            &MirBackendContract::default(),
            &HashMap::new(),
            &mut diagnostics,
        );
        assert!(diagnostics
            .0
            .iter()
            .any(|diag| diag.message.contains("does not live long enough")));

        let temporary_escape = reference_return_function(
            &mut type_context,
            MirOwnershipMetadata {
                reference_origins: vec![(Local(0), ReferenceOrigin::Temporary(Local(2)))],
                temporary_locals: vec![Local(2)],
                drop_obligations: Vec::new(),
            },
        );
        let mut diagnostics = Diagnostics::default();
        BorrowChecker::validate_reference_escapes(
            &temporary_escape,
            &type_context,
            &MirBackendContract::default(),
            &HashMap::new(),
            &mut diagnostics,
        );
        assert!(diagnostics
            .0
            .iter()
            .any(|diag| diag.message.contains("temporary")));

        let param_escape = reference_return_function(
            &mut type_context,
            MirOwnershipMetadata {
                reference_origins: vec![(Local(0), ReferenceOrigin::Param(Local(1)))],
                temporary_locals: Vec::new(),
                drop_obligations: Vec::new(),
            },
        );
        let mut diagnostics = Diagnostics::default();
        BorrowChecker::validate_reference_escapes(
            &param_escape,
            &type_context,
            &MirBackendContract::default(),
            &HashMap::new(),
            &mut diagnostics,
        );
        assert!(diagnostics.0.is_empty());

        let static_override = reference_return_function(
            &mut type_context,
            MirOwnershipMetadata {
                reference_origins: vec![
                    (Local(0), ReferenceOrigin::Temporary(Local(2))),
                    (Local(0), ReferenceOrigin::Static),
                ],
                temporary_locals: vec![Local(2)],
                drop_obligations: Vec::new(),
            },
        );
        let mut diagnostics = Diagnostics::default();
        BorrowChecker::validate_reference_escapes(
            &static_override,
            &type_context,
            &MirBackendContract::default(),
            &HashMap::new(),
            &mut diagnostics,
        );
        assert!(diagnostics.0.is_empty());
    }

    fn enum_origin_source_program(source: &str) -> MirProgram {
        let path = std::path::PathBuf::from("/virtual/enum_return_origins/main.rk");
        let config = crate::Config {
            entry_file: path.clone(),
            current_crate_name: Some("enum_return_origins".into()),
            no_std: true,
            no_prelude: true,
            no_link: true,
            source_providers: vec![crate::SourceProvider::Virtual {
                path,
                text: source.into(),
            }],
            ..Default::default()
        };
        let frontend =
            crate::compile_frontend(&config).expect("actual enum source must reach accepted HIR");
        let fingerprint = crate::product_source_fingerprint(
            &config,
            &frontend.source_db,
            &frontend.fingerprint_source_files,
        );
        let identity = crate::mono::CompilationIdentityContext::new(
            frontend.analysis.hir.root_crate_id,
            crate::products::ProductCrateIdentity::local(
                frontend.effective_current_crate_name.clone(),
            )
            .with_source_fingerprint(fingerprint),
            frontend
                .crate_ctx
                .product_crate_ids
                .iter()
                .map(|(identity, id)| (*id, identity.clone()))
                .collect(),
        );
        let mut mono = crate::mono::monomorphize_with_crates(
            frontend.analysis.hir,
            &frontend.crate_ctx,
            identity,
        )
        .expect("actual enum source must monomorphize");
        let mut bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut mono);
        crate::dce::prune_unreachable_instances(&mut mono, &mut bodies);
        let program = crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(
            &mono, &bodies,
        );
        let agreement = crate::mir::agreement::check_mir_runtime_agreement(&program);
        assert!(agreement.is_clean(), "{agreement:?}");
        program
    }

    fn enum_origin_native<'a>(
        program: &'a MirProgram,
        source: &str,
        marker: &str,
        argument: &str,
    ) -> &'a MirFunction {
        let start = source.find(marker).unwrap() + marker.find(argument).unwrap();
        let span = Span::new(
            "/virtual/enum_return_origins/main.rk".into(),
            start,
            start + argument.len(),
        );
        let found = program
            .functions()
            .filter_map(|(_, function)| {
                (function.arg_count == 1
                    && function.local_decls.get(1).is_some_and(|decl| {
                        decl.source == LocalSource::Argument && decl.span.as_ref() == Some(&span)
                    }))
                .then_some(function)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            found.len(),
            1,
            "one actual native body at the real source argument declaration",
        );
        let function = found[0];
        let key = &program.backend_contract.function_bodies[&function.id];
        let declaration = &program.backend_contract.callables[key];
        assert_eq!(&declaration.key, key);
        assert!(
            matches!(&declaration.kind, crate::mir::MirCallableKind::LocalBody { function_id } if function_id == &function.id)
        );
        assert!(declaration.source_def_id.is_some());
        assert_eq!(declaration.signature.params.len(), function.arg_count);
        assert_eq!(
            declaration.signature.params[0].semantic_ty,
            function.local_decls[1].ty,
        );
        assert_eq!(
            declaration.signature.params[0].pass_mode,
            crate::mir::MirPassMode::Direct,
        );
        assert_eq!(declaration.signature.ret.semantic_ty, function.ret_type);
        assert_eq!(declaration.signature.ret.abi_ty, function.ret_type);
        assert_eq!(function.local_decls[0].ty, function.ret_type);
        assert_eq!(function.local_decls[0].source, LocalSource::ReturnPlace);
        assert_eq!(
            program.backend_contract.place_type_id(
                &program.type_context,
                function,
                &Place {
                    local: Local(0),
                    projection: Vec::new(),
                },
            ),
            Some(function.ret_type),
        );
        function
    }

    fn enum_origin_binding(
        function: &MirFunction,
        source: &str,
        marker: &str,
        binding: &str,
    ) -> (Local, Span) {
        let body = source.find(marker).unwrap();
        let start = body + source[body..].find(&format!("    {binding} =")).unwrap() + 4;
        let span = Span::new(
            "/virtual/enum_return_origins/main.rk".into(),
            start,
            start + binding.len(),
        );
        let local = function
            .local_decls
            .iter()
            .enumerate()
            .find(|(_, decl)| {
                decl.source == LocalSource::UserBinding && decl.span.as_ref() == Some(&span)
            })
            .map(|(index, _)| Local(index))
            .expect("actual local binding source span");
        (local, span)
    }

    fn enum_origin_payload(
        program: &MirProgram,
        function: &MirFunction,
        statement: &StatementData,
    ) -> (String, Place, crate::ids::TypeId) {
        let crate::mir::StatementKind::Assign(
            destination,
            Rvalue::Aggregate(
                crate::mir::AggregateKind::EnumVariant {
                    enum_id,
                    variant_id,
                    variant_name,
                    ..
                },
                operands,
            ),
        ) = &statement.kind
        else {
            panic!("actual source enum aggregate required");
        };
        assert_eq!(operands.len(), 1);
        let root = program
            .backend_contract
            .place_type_id(&program.type_context, function, destination)
            .unwrap();
        let Type::Enum { id: owner, args } = program.type_context.type_for(root) else {
            panic!("actual aggregate destination must be the canonical enum");
        };
        assert_eq!(owner, *enum_id);
        let MirNominalLayout::Enum {
            id,
            generic_params,
            variants,
        } = &program.backend_contract.nominal_layouts[&owner]
        else {
            panic!("actual canonical enum layout required");
        };
        assert_eq!(*id, owner);
        assert_eq!(generic_params.len(), args.len());
        assert_eq!(args.len(), 1);
        assert_eq!(generic_params[0].owner, owner);
        let layout = &variants[variant_id.0 as usize];
        assert_eq!(&layout.name, variant_name);
        let MirVariantLayoutFields::Positional(fields) = &layout.fields else {
            panic!("actual positional payload layout required");
        };
        assert_eq!(fields.len(), 1);
        let substitution = generic_params
            .iter()
            .copied()
            .zip(args)
            .collect::<HashMap<_, _>>();
        let actual = program.backend_contract.normalize_type(
            &program.type_context,
            &program
                .type_context
                .type_for(fields[0])
                .substitute_generics(&substitution),
        );
        let payload_ty = program
            .type_context
            .id_for_type(&actual)
            .expect("actual instantiated payload TypeId");
        let mut payload = destination.clone();
        payload.projection.push(Projection::Downcast(*variant_id));
        payload = ReferenceEscapeAnalysis::field_place(&payload, 0);
        assert_eq!(
            program
                .backend_contract
                .place_type_id(&program.type_context, function, &payload),
            Some(payload_ty),
        );
        (variant_name.clone(), payload, payload_ty)
    }

    #[test]
    fn enum_origin_return_preserves_raw_handle_payload_and_input_envelope() {
        let source = r#"
extern acquire: &I64 -> *I64
struct Handle
    < pointer: *I64
enum Choice A
    Present A
    Alternate A
raw_make: &I64 -> Choice Handle
raw_make = pointer ->
    raw = acquire pointer
    handle = Handle
        pointer: raw
    Choice::Present handle
main = !->
    value = 7
    raw_make &value
"#;
        let program = enum_origin_source_program(source);
        let function = enum_origin_native(&program, source, "raw_make = pointer ->", "pointer");
        assert_eq!(
            program.type_context.type_for(function.local_decls[1].ty),
            Type::Reference {
                inner: Box::new(Type::I64),
                mutable: false,
            },
        );
        let aggregates = function
            .basic_blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter(|statement| {
                matches!(&statement.kind, crate::mir::StatementKind::Assign(destination,
                    Rvalue::Aggregate(crate::mir::AggregateKind::EnumVariant { .. }, _))
                    if destination.local == Local(0) && destination.projection.is_empty())
            })
            .collect::<Vec<_>>();
        assert_eq!(aggregates.len(), 1);
        let (variant, payload, payload_ty) = enum_origin_payload(&program, function, aggregates[0]);
        assert_eq!(variant, "Present");
        assert!(
            matches!(program.type_context.type_for(payload_ty), Type::Struct { args, .. } if args.is_empty())
        );
        assert!(!return_type_needs_escape_check(
            &program.type_context,
            &program.backend_contract,
            payload_ty,
        ));
        let expected = HashSet::from([
            ReferenceOrigin::Param(Local(1)),
            ReferenceOrigin::UnknownExternal,
        ]);
        let lifetime = BorrowChecker::reference_return_summaries(&program);
        assert_eq!(
            lifetime.get(&function.id),
            Some(&expected),
            "the raw Lifetime envelope must not be replaced by a value-loan/empty proof",
        );
        let analysis = ReferenceEscapeAnalysis::new(
            function,
            &program.type_context,
            &program.backend_contract,
            &lifetime,
            MissingSummaryMode::Conservative,
        );
        let results = run_fixpoint(&analysis, function);
        let mut transfers = 0;
        for (block, data) in function.basic_blocks.iter().enumerate() {
            if !results.reachable[block] {
                continue;
            }
            let mut state = results.entry_sets[block].clone();
            for statement in &data.statements {
                if std::ptr::eq(statement, aggregates[0]) {
                    let crate::mir::StatementKind::Assign(_, Rvalue::Aggregate(_, operands)) =
                        &statement.kind
                    else {
                        unreachable!();
                    };
                    assert_eq!(analysis.origins_for_operand(&state, &operands[0]), expected);
                    crate::mir::dataflow::Analysis::apply_statement(
                        &analysis, &mut state, statement,
                    );
                    assert_eq!(state.origins.get(&payload), Some(&expected));
                    transfers += 1;
                } else {
                    crate::mir::dataflow::Analysis::apply_statement(
                        &analysis, &mut state, statement,
                    );
                }
            }
        }
        assert_eq!(
            transfers, 1,
            "the actual enum transfer preserves the operand's full envelope",
        );
        let returns = function
            .basic_blocks
            .iter()
            .enumerate()
            .filter_map(|(block, data)| {
                (results.reachable[block]
                    && matches!(
                        data.terminator,
                        Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
                    ))
                .then_some(&results.exit_sets[block])
            })
            .collect::<Vec<_>>();
        assert!(!returns.is_empty());
        for state in returns {
            assert_eq!(state.origins.get(&payload), Some(&expected));
            let root = Place {
                local: Local(0),
                projection: Vec::new(),
            };
            assert_eq!(analysis.origins_for_place(state, &root), expected);
            assert!(
                !state
                    .origins
                    .contains_key(&ReferenceEscapeAnalysis::field_place(&root, 0)),
                "no variant-agnostic payload key",
            );
        }
        BorrowChecker::run(&program)
            .expect("checked generic enum payload is raw-only owning storage");
    }

    #[test]
    fn enum_origin_return_and_match_reject_both_local_reference_variants() {
        for (declarations, arms) in [
            ("    First A\n    Second A", "        Choice::First payload => payload\n        Choice::Second payload => payload"),
            ("    Second A\n    First A", "        Choice::Second payload => payload\n        Choice::First payload => payload"),
        ] {
            let source = format!(
                r#"
enum Choice A
{declarations}
bad: Bool -> Choice (&I64)
bad = flag ->
    left = 1
    right = 2
    if flag then Choice::First &left else Choice::Second &right
extract: Bool -> &I64
extract = flag ->
    left = 3
    right = 4
    match (if flag then Choice::First &left else Choice::Second &right)
{arms}
main = !->
    bad true
    bad false
    extract true
    extract false
"#
            );
            let program = enum_origin_source_program(&source);
            let lifetime = BorrowChecker::reference_return_summaries(&program);
            let mut expected_spans = Vec::new();
            for marker in ["bad = flag ->", "extract = flag ->"] {
                let function = enum_origin_native(&program, &source, marker, "flag");
                assert_eq!(
                    program.type_context.type_for(function.local_decls[1].ty),
                    Type::Bool,
                );
                let (left, left_span) = enum_origin_binding(function, &source, marker, "left");
                let (right, right_span) = enum_origin_binding(function, &source, marker, "right");
                expected_spans.extend([left_span, right_span]);
                let analysis = ReferenceEscapeAnalysis::new(
                    function,
                    &program.type_context,
                    &program.backend_contract,
                    &lifetime,
                    MissingSummaryMode::Conservative,
                );
                let results = run_fixpoint(&analysis, function);
                let aggregates = function
                    .basic_blocks
                    .iter()
                    .flat_map(|block| &block.statements)
                    .filter(|statement| {
                        matches!(
                            &statement.kind,
                            crate::mir::StatementKind::Assign(
                                _,
                                Rvalue::Aggregate(crate::mir::AggregateKind::EnumVariant { .. }, _)
                            )
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(aggregates.len(), 2);
                let mut payloads = HashMap::new();
                for statement in aggregates {
                    let (variant, place, ty) = enum_origin_payload(&program, function, statement);
                    assert_eq!(
                        program.type_context.type_for(ty),
                        Type::Reference {
                            inner: Box::new(Type::I64),
                            mutable: false,
                        },
                    );
                    assert!(return_type_needs_escape_check(
                        &program.type_context,
                        &program.backend_contract,
                        ty,
                    ));
                    let origin = match variant.as_str() {
                        "First" => ReferenceOrigin::Local(left),
                        "Second" => ReferenceOrigin::Local(right),
                        _ => panic!("unexpected source fixture variant"),
                    };
                    assert!(payloads.insert(place, HashSet::from([origin])).is_none());
                }
                assert_eq!(
                    payloads.len(),
                    2,
                    "different actual VariantIds remain different origin keys",
                );
                let whole = HashSet::from([
                    ReferenceOrigin::Local(left),
                    ReferenceOrigin::Local(right),
                ]);
                let root = Place {
                    local: payloads.keys().next().unwrap().local,
                    projection: Vec::new(),
                };
                assert!(payloads.keys().all(|place| place.local == root.local));
                assert!(
                    results.exit_sets.iter().any(|state| {
                        if !payloads
                            .iter()
                            .all(|(place, origins)| state.origins.get(place) == Some(origins))
                        {
                            return false;
                        }
                        for (place, origins) in &payloads {
                            assert_eq!(
                                &analysis.origins_for_place(state, place),
                                origins,
                                "no cross-variant field0 contamination",
                            );
                        }
                        assert_eq!(
                            analysis.origins_for_place(state, &root),
                            whole,
                            "whole enum retains both origins across joins",
                        );
                        true
                    }),
                    "actual source CFG must join both canonical variant payload paths",
                );
                if marker == "extract = flag ->" {
                    let mut reads = HashSet::new();
                    for (block, data) in function.basic_blocks.iter().enumerate() {
                        if !results.reachable[block] {
                            continue;
                        }
                        let mut state = results.entry_sets[block].clone();
                        for statement in &data.statements {
                            if let crate::mir::StatementKind::Assign(
                                destination,
                                Rvalue::Use(operand),
                            ) = &statement.kind
                            {
                                if let Operand::Copy(place) | Operand::Move(place) = operand {
                                    if let Some(expected) = payloads.get(place) {
                                        assert_eq!(
                                            program.backend_contract.place_type_id(
                                                &program.type_context,
                                                function,
                                                place,
                                            ),
                                            Some(function.ret_type),
                                        );
                                        assert_eq!(
                                            &analysis.origins_for_operand(&state, operand),
                                            expected,
                                        );
                                        reads.insert(place.clone());
                                        crate::mir::dataflow::Analysis::apply_statement(
                                            &analysis, &mut state, statement,
                                        );
                                        assert_eq!(
                                            state.origins.get(
                                                &ReferenceEscapeAnalysis::normalized_place(
                                                    destination.clone(),
                                                ),
                                            ),
                                            Some(expected),
                                        );
                                        continue;
                                    }
                                }
                            }
                            crate::mir::dataflow::Analysis::apply_statement(
                                &analysis, &mut state, statement,
                            );
                        }
                    }
                    assert_eq!(
                        reads.len(),
                        2,
                        "both actual match payload reads must use the canonical variant paths and retain their own borrow",
                    );
                }
            }
            let diagnostics = BorrowChecker::run(&program)
                .expect_err("both variant orders and match reads must reject escaping local references");
            assert!(
                diagnostics.0.iter().all(|diagnostic| {
                    diagnostic.code == Some(DiagnosticCode::Borrow)
                        && !diagnostic.message.contains("Missing checked return-place type")
                }),
                "a typing/authority gap is not a successful safety rejection: {diagnostics:?}",
            );
            for span in expected_spans {
                assert!(
                    diagnostics.0.iter().any(|diagnostic| {
                        diagnostic.message == "Cannot return reference to local value because it does not live long enough"
                            && diagnostic.primary.as_ref().map(|label| &label.span) == Some(&span)
                    }),
                    "each actual source-local borrow in both enum variants must be diagnosed: {span:?} {diagnostics:?}",
                );
            }
        }
    }

    const RAW_RETURN_ELIGIBILITY: &str = r#"
extern malloc: I64 -> *U8
struct Storage
    < pointer: *I64
impl Storage
    @raw: *I64
    @raw = -> self.pointer
struct State A
    < owner: A
    < audit: *I64
struct LaterRaw
    < pointer: *I64
    < state: State Storage
impl Factory for LaterRaw
    make = seed ->
        pointer = (malloc 8) as *I64
        storage = Storage
            pointer: pointer
        audit = storage.raw!
        state = State
            owner: storage
            audit: audit
        LaterRaw
            pointer: pointer
            state: state
read: LaterRaw -> I64
read = product -> 0
main = !->
    service = Service
        marker: 0
    object = as_dispatch &service
    read (object.build 42)
"#;

    fn return_eligibility_program(source: &str) -> (MirProgram, MirFunctionId) {
        let program = crate::mir::static_dictionary::tests::accepted_program(source);
        let report = crate::mir::agreement::check_mir_runtime_agreement(&program);
        assert!(report.is_clean(), "{report:?}");
        let errors = crate::mir::validate_erased_contract(&program);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(program.backend_contract.erased.dictionaries.len(), 1);
        let dictionary = program
            .backend_contract
            .erased
            .dictionaries
            .values()
            .next()
            .unwrap();
        assert_eq!(dictionary.members.len(), 1);
        let member = &dictionary.members[0];
        assert_eq!(member.receiver, None);
        let key = member
            .target
            .native()
            .expect("actual canonical Factory target");
        let declaration = &program.backend_contract.callables[key];
        let crate::mir::MirCallableKind::LocalBody { function_id } = &declaration.kind else {
            panic!("actual native Factory body required");
        };
        let id = function_id.clone();
        let function = &program.functions[&id];
        assert_eq!(&function.id, function_id);
        assert_eq!(&program.backend_contract.function_bodies[function_id], key);
        assert_eq!(&declaration.key, key);
        assert!(declaration.source_def_id.is_some());
        let scalar = program.type_context.id_for_type(&Type::I64).unwrap();
        assert_eq!(function.arg_count, 1);
        assert_eq!(function.local_decls[1].ty, scalar);
        assert_eq!(function.local_decls[0].ty, function.ret_type);
        assert_eq!(dictionary.subject, function.ret_type);
        assert_eq!(
            declaration.signature,
            crate::mir::MirCallableSignature::from_type_ids(
                &[scalar],
                function.ret_type,
                crate::mir::MirPassMode::Direct,
            )
        );
        let Type::Struct { id: owner, args } = program.type_context.type_for(function.ret_type)
        else {
            panic!("actual nominal product result required");
        };
        assert!(args.is_empty());
        let fields = contract_struct_fields(&program.backend_contract, owner).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[1].0, "state");
        assert_eq!(
            program.backend_contract.nominal_field_ids[&owner].len(),
            fields.len()
        );
        (program, id)
    }

    #[test]
    fn return_origin_eligibility_allows_raw_handle_only_owning_state() {
        let source = format!(
            "{}{RAW_RETURN_ELIGIBILITY}",
            crate::mir::static_dictionary::tests::PRODUCER
        );
        let (program, id) = return_eligibility_program(&source);
        let function = &program.functions[&id];
        let state_place = ReferenceEscapeAnalysis::field_place(
            &Place {
                local: Local(0),
                projection: Vec::new(),
            },
            1,
        );
        let state_ty = program
            .backend_contract
            .place_type_id(&program.type_context, function, &state_place)
            .unwrap();
        let Type::Struct { args, .. } = program.type_context.type_for(state_ty) else {
            panic!("actual generic State field type required");
        };
        assert_eq!(args.len(), 1);
        assert!(matches!(&args[0], Type::Struct { args, .. } if args.is_empty()));
        assert!(!return_type_needs_escape_check(
            &program.type_context,
            &program.backend_contract,
            state_ty
        ));
        let start = source.find("        storage =").unwrap() + 8;
        let expected = Span::new(
            "/virtual/static_dictionary/producer.rk".into(),
            start,
            start + "storage".len(),
        );
        let storage = function
            .local_decls
            .iter()
            .enumerate()
            .find(|(_, decl)| {
                decl.source == LocalSource::UserBinding && decl.span.as_ref() == Some(&expected)
            })
            .map(|(index, decl)| (Local(index), decl))
            .expect("actual source storage binding");
        assert_eq!(program.type_context.type_for(storage.1.ty), args[0]);
        let lifetime = BorrowChecker::reference_return_summaries(&program);
        let values = BorrowChecker::native_return_loan_proof(&program);
        assert!(
            lifetime[&id].contains(&ReferenceOrigin::Local(storage.0)),
            "the raw Lifetime storage envelope must remain intact"
        );
        assert!(
            values.summaries.get(&id).is_some_and(HashSet::is_empty),
            "actual checked native value flow is independently known empty"
        );
        let analysis = ReferenceEscapeAnalysis::new(
            function,
            &program.type_context,
            &program.backend_contract,
            &lifetime,
            MissingSummaryMode::Conservative,
        );
        let results = run_fixpoint(&analysis, function);
        assert!(
            results.exit_sets.iter().enumerate().any(|(block, state)| {
                results.reachable[block]
                    && matches!(
                        function.basic_blocks[block].terminator,
                        Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
                    )
                    && state
                        .origins
                        .get(&state_place)
                        .is_some_and(|origins| origins.contains(&ReferenceOrigin::Local(storage.0)))
            }),
            "the actual raw-only returned State must retain the real Local origin",
        );
        BorrowChecker::run(&program).unwrap();
    }

    #[test]
    fn return_origin_eligibility_rejects_safe_reference_in_generic_state() {
        let source = format!(
            "{}{}",
            crate::mir::static_dictionary::tests::PRODUCER,
            RAW_RETURN_ELIGIBILITY
                .replace("< state: State Storage", "< state: State (&I64)")
                .replace("owner: storage", "owner: &seed")
        );
        let (program, id) = return_eligibility_program(&source);
        let function = &program.functions[&id];
        let state_place = ReferenceEscapeAnalysis::field_place(
            &Place {
                local: Local(0),
                projection: Vec::new(),
            },
            1,
        );
        let state_ty = program
            .backend_contract
            .place_type_id(&program.type_context, function, &state_place)
            .unwrap();
        let Type::Struct { args, .. } = program.type_context.type_for(state_ty) else {
            panic!("actual generic State field type required");
        };
        assert_eq!(
            args,
            vec![Type::Reference {
                inner: Box::new(Type::I64),
                mutable: false
            }]
        );
        assert!(return_type_needs_escape_check(
            &program.type_context,
            &program.backend_contract,
            state_ty
        ));
        let start = source.rfind("    make = seed ->").unwrap() + "    make = ".len();
        let expected = Span::new(
            "/virtual/static_dictionary/producer.rk".into(),
            start,
            start + "seed".len(),
        );
        assert_eq!(function.local_decls[1].source, LocalSource::Argument);
        assert_eq!(function.local_decls[1].span.as_ref(), Some(&expected));
        let lifetime = BorrowChecker::reference_return_summaries(&program);
        let analysis = ReferenceEscapeAnalysis::new(
            function,
            &program.type_context,
            &program.backend_contract,
            &lifetime,
            MissingSummaryMode::Conservative,
        );
        let results = run_fixpoint(&analysis, function);
        assert!(
            results.exit_sets.iter().enumerate().any(|(block, state)| {
                results.reachable[block]
                    && matches!(
                        function.basic_blocks[block].terminator,
                        Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
                    )
                    && state
                        .origins
                        .get(&state_place)
                        .is_some_and(|origins| origins.contains(&ReferenceOrigin::Local(Local(1))))
            }),
            "the generic reference field must retain the real callee-local seed storage",
        );
        let diagnostics = BorrowChecker::run(&program)
            .expect_err("generic State<&I64> cannot return a reference to its seed slot");
        assert!(diagnostics.0.iter().any(|diagnostic| {
            diagnostic.code == Some(DiagnosticCode::Borrow)
                && diagnostic.message == "Cannot return reference to local value because it does not live long enough"
                && diagnostic.primary.as_ref().map(|label| &label.span) == Some(&expected)
        }), "actual safe-reference field must reject at the real seed declaration: {diagnostics:?}");
    }

    const ABSTRACT_FUNCTION_FIELD: &str = r#"
struct LaterReference
    < value: () -> I64
impl Factory for LaterReference
    make = seed ->
        local = seed
        borrowed = &local
        LaterReference
            value: -> *borrowed
read: LaterReference -> I64
read = product -> product.value!
main = ->
    service = Service
        marker: 0
    object = as_dispatch &service
    read (object.build 42)
"#;

    fn abstract_function_field_native(program: &MirProgram) -> (&MirFunction, &Place, &MirClosure) {
        let report = crate::mir::agreement::check_mir_runtime_agreement(program);
        assert!(report.is_clean(), "{report:?}");
        let errors = crate::mir::validate_erased_contract(program);
        assert!(errors.is_empty(), "{errors:?}");

        let dictionaries = &program.backend_contract.erased.dictionaries;
        assert_eq!(dictionaries.len(), 1);
        let dictionary = dictionaries.values().next().unwrap();
        assert_eq!(dictionary.members.len(), 1);
        let member = &dictionary.members[0];
        assert_eq!(member.receiver, None);
        let key = member.target.native().expect("actual static native target");
        let declaration = &program.backend_contract.callables[key];
        let crate::mir::MirCallableKind::LocalBody { function_id } = &declaration.kind else {
            panic!("static native target must have its actual local body");
        };
        let function = &program.functions[function_id];
        assert_eq!(&function.id, function_id);
        assert_eq!(&program.backend_contract.function_bodies[function_id], key);
        assert_eq!(&declaration.key, key);
        assert!(declaration.source_def_id.is_some());
        assert_eq!(function.arg_count, 1);
        let scalar = program.type_context.id_for_type(&Type::I64).unwrap();
        assert_eq!(function.local_decls[1].ty, scalar);
        assert_eq!(function.local_decls[0].ty, function.ret_type);
        assert_eq!(dictionary.subject, function.ret_type);
        assert_eq!(
            declaration.signature,
            crate::mir::MirCallableSignature::from_type_ids(
                &[scalar],
                function.ret_type,
                crate::mir::MirPassMode::Direct,
            ),
        );

        let Type::Struct { id, args } = program.type_context.type_for(function.ret_type) else {
            panic!("actual native result must be the nominal field owner");
        };
        assert!(args.is_empty());
        let fields = super::contract_struct_fields(&program.backend_contract, id).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(program.backend_contract.nominal_field_ids[&id].len(), 1);
        assert_eq!(
            program.type_context.type_for(fields[0].1),
            Type::function(Vec::new(), Type::I64),
            "declared field is abstract, not proof of a capture-free environment",
        );
        assert!(!type_id_contains_reference(
            TypeView::new(&program.type_context),
            &program.backend_contract,
            function.ret_type,
        ));

        let closures = function
            .basic_blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.kind {
                crate::mir::StatementKind::Assign(dest, Rvalue::Closure(closure)) => {
                    Some((dest, closure))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(closures.len(), 1);
        let (destination, closure) = closures[0];
        assert_eq!(closure.id.owner, function.id);
        assert!(destination.projection.is_empty());
        assert_eq!(
            function.local_decls[destination.local.0].source,
            LocalSource::Temporary
        );
        assert_eq!(closure.captures.len(), 1);
        assert!(function.basic_blocks.iter().flat_map(|block| &block.statements).any(|statement| {
            matches!(&statement.kind,
                crate::mir::StatementKind::Assign(dest, Rvalue::Aggregate(kind, operands))
                    if dest.local == Local(0) && dest.projection.is_empty()
                        && matches!(kind, crate::mir::AggregateKind::Struct { id: owner, .. } if *owner == id)
                        && matches!(operands.as_slice(), [Operand::Move(place)] if place == destination))
        }), "actual captured temporary must be packed directly into the returned field");
        (function, destination, closure)
    }

    #[test]
    fn abstract_function_field_return_rejects_borrowed_local_capture() {
        let source = format!(
            "{}{ABSTRACT_FUNCTION_FIELD}",
            crate::mir::static_dictionary::tests::PRODUCER,
        );
        let program = crate::mir::static_dictionary::tests::accepted_program(&source);
        let (function, destination, closure) = abstract_function_field_native(&program);
        let capture = &closure.captures[0];
        assert_eq!(capture.kind, MirClosureCaptureKind::ByRef);
        let borrowed = &function.local_decls[capture.local.0];
        let reference = Type::Reference {
            inner: Box::new(Type::I64),
            mutable: false,
        };
        assert_eq!(program.type_context.type_for(borrowed.ty), reference);
        assert_eq!(borrowed.source, LocalSource::UserBinding);
        let start = source.find("        borrowed =").unwrap() + 8;
        let expected_span = crate::lexer::Span::new(
            "/virtual/static_dictionary/producer.rk".into(),
            start,
            start + "borrowed".len(),
        );
        assert_eq!(borrowed.span.as_ref(), Some(&expected_span));
        assert_eq!(capture.span.as_ref(), Some(&expected_span));
        let Type::Function { captures, .. } = program
            .type_context
            .type_for(function.local_decls[destination.local.0].ty)
        else {
            panic!("actual closure temporary type");
        };
        assert_eq!(
            captures.as_ref(),
            &[crate::types::FunctionCapture::new(
                crate::types::CaptureKind::SharedBorrow,
                reference.clone(),
            )]
        );
        let body = &program.functions[&MirFunctionId::Closure(Box::new(closure.id.clone()))];
        assert_eq!(body.closure_captures.len(), 1);
        let slot = &body.closure_captures[0];
        assert_eq!(slot.kind, MirClosureCaptureKind::ByRef);
        assert_eq!(
            body.local_decls[slot.local.0].source,
            LocalSource::ClosureCapture
        );
        assert_eq!(
            program
                .type_context
                .type_for(body.local_decls[slot.local.0].ty),
            Type::Reference {
                inner: Box::new(reference),
                mutable: false
            },
        );

        let summaries = BorrowChecker::reference_return_summaries(&program);
        let analysis = ReferenceEscapeAnalysis::new(
            function,
            &program.type_context,
            &program.backend_contract,
            &summaries,
            MissingSummaryMode::Conservative,
        );
        let results = crate::mir::dataflow::run_fixpoint(&analysis, function);
        let field = ReferenceEscapeAnalysis::field_place(
            &Place {
                local: Local(0),
                projection: Vec::new(),
            },
            0,
        );
        assert!(results
            .exit_sets
            .iter()
            .any(|state| state.origins.get(&field)
                == Some(&HashSet::from([ReferenceOrigin::Local(capture.local)]))));
        let diagnostics = BorrowChecker::run(&program)
            .expect_err("actual borrowed local environment must not escape");
        assert!(
            diagnostics.0.iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic
                        .message
                        .contains("Cannot return reference to local value")
                    && diagnostic.primary.as_ref().map(|label| &label.span) == Some(&expected_span)
            }),
            "actual capture storage origin and source Borrow diagnostic: {diagnostics:?}"
        );
    }

    #[test]
    fn abstract_function_field_return_allows_by_value_copy_scalar_capture() {
        // The explicit move receiver requests value capture; Copy I64 alone
        // would not prove that the default closure capture is by value.
        let copying = r#"
< trait CopyValue
    ~@into_value: I64
impl CopyValue for I64
    ~@into_value = -> self
"#;
        let source = format!(
            "{}{copying}{}",
            crate::mir::static_dictionary::tests::PRODUCER,
            ABSTRACT_FUNCTION_FIELD
                .replace("        borrowed = &local\n", "")
                .replace("value: -> *borrowed", "value: -> local.into_value!"),
        );
        let program = crate::mir::static_dictionary::tests::accepted_program(&source);
        let (function, destination, closure) = abstract_function_field_native(&program);
        let capture = &closure.captures[0];
        assert_eq!(
            capture.kind,
            MirClosureCaptureKind::ByValue,
            "no stack-slot capture permitted in this positive"
        );
        let captured = &function.local_decls[capture.local.0];
        assert_eq!(captured.source, LocalSource::UserBinding);
        assert_eq!(program.type_context.type_for(captured.ty), Type::I64);
        assert!(crate::type_services::facts::TypeFacts::is_copy(&Type::I64));
        let Type::Function {
            callable_kind,
            captures,
            ..
        } = program
            .type_context
            .type_for(function.local_decls[destination.local.0].ty)
        else {
            panic!("actual closure temporary type");
        };
        assert_eq!(callable_kind, crate::types::CallableKind::Fn);
        assert_eq!(
            captures.as_ref(),
            &[crate::types::FunctionCapture::new(
                crate::types::CaptureKind::Move,
                Type::I64,
            )]
        );
        let body = &program.functions[&MirFunctionId::Closure(Box::new(closure.id.clone()))];
        assert_eq!(body.closure_captures.len(), 1);
        let slot = &body.closure_captures[0];
        assert_eq!(slot.kind, MirClosureCaptureKind::ByValue);
        assert_eq!(
            body.local_decls[slot.local.0].source,
            LocalSource::ClosureCapture
        );
        assert_eq!(
            program
                .type_context
                .type_for(body.local_decls[slot.local.0].ty),
            Type::I64
        );
        let summaries = BorrowChecker::reference_return_summaries(&program);
        assert!(
            BorrowChecker::reference_return_origins(
                function,
                &program.type_context,
                &program.backend_contract,
                &summaries,
            )
            .is_empty(),
            "actual value-captured scalar has no borrowed storage origin"
        );
        BorrowChecker::run(&program).unwrap();
    }

    #[test]
    fn closure_assignment_rejects_captured_reference_to_temporary() {
        let mut type_context = TypeContext::new();
        let unit_id = test_type_id(&mut type_context, Type::Unit);
        let closure_id = test_type_id(&mut type_context, Type::function(Vec::new(), Type::Unit));
        let ref_i64 = test_type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let i64_id = test_type_id(&mut type_context, Type::I64);

        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(128))),
            name: "closure_captures_temporary_ref".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Closure(MirClosure {
                        id: MirClosureId {
                            owner: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(128))),
                            local_index: 0,
                        },
                        display_name: "closure_captures_temporary_ref::lambda_0".to_string(),
                        captures: vec![MirClosureCapture {
                            name: "captured".to_string(),
                            local: Local(1),
                            kind: MirClosureCaptureKind::ByRef,
                            span: None,
                        }],
                    }),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: closure_id,
                    mutability: Mutability::Not,
                    name: Some("closure".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Not,
                    name: Some("captured".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Not,
                    name: Some("call_result".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: unit_id,
            ownership: MirOwnershipMetadata {
                reference_origins: vec![(Local(1), ReferenceOrigin::Temporary(Local(2)))],
                temporary_locals: vec![Local(1), Local(2)],
                drop_obligations: Vec::new(),
            },
        };
        let mut diagnostics = Diagnostics::default();

        BorrowChecker::validate_reference_escapes(
            &function,
            &type_context,
            &MirBackendContract::default(),
            &HashMap::new(),
            &mut diagnostics,
        );

        assert!(diagnostics.0.iter().any(|diag| {
            diag.message
                .contains("Cannot store reference to temporary value")
        }));
    }

    #[test]
    fn temporary_assignment_screen_preserves_non_numeric_storage_guards() {
        let mut tc = TypeContext::new();
        let mut function = reference_return_function(&mut tc, MirOwnershipMetadata::default());
        let reference = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };
        let pointer = Type::Pointer(Box::new(Type::I64));
        let callback = Type::function(Vec::new(), Type::I64);
        assert!(matches!(&callback, Type::Function { captures, .. } if captures.is_empty()));
        let owner_id = DefId::new(CrateId(0), LocalDefId(501));
        let parameter = GenericParamId {
            owner: owner_id,
            index: 0,
        };
        let logical_owner = Type::Struct {
            id: owner_id,
            args: vec![reference.clone()],
        };
        let mut contract = MirBackendContract::default();
        contract.nominal_layouts.insert(
            owner_id,
            MirNominalLayout::Struct {
                id: owner_id,
                generic_params: vec![parameter],
                fields: vec![("raw".into(), tc.intern_type(&pointer))],
            },
        );
        let input = Place {
            local: Local(1),
            projection: Vec::new(),
        };
        let dest = Place {
            local: Local(0),
            projection: Vec::new(),
        };
        let rvalue = Rvalue::Use(Operand::Copy(input.clone()));
        let mut state = ReferenceOriginState::default();
        state.origins.insert(
            input.clone(),
            HashSet::from([
                ReferenceOrigin::Temporary(Local(2)),
                ReferenceOrigin::Param(Local(1)),
                ReferenceOrigin::ParamContents(Local(1)),
                ReferenceOrigin::ParamBorrowedContents(Local(1)),
                ReferenceOrigin::UnknownExternal,
            ]),
        );
        let original = state.origins.clone();
        let summaries = HashMap::new();
        // Classification-only guards, not accepted-source or LoanId evidence.
        for (ty, numeric) in [
            (Type::I64, true),
            (Type::F64, true),
            (Type::Bool, false),
            (Type::Unit, false),
            (pointer, false),
            (reference, false),
            (callback, false),
            (logical_owner, false),
            (Type::Generic(parameter), false),
            (Type::Error, false),
        ] {
            let ty_id = tc.intern_type(&ty);
            function.local_decls[0].ty = ty_id;
            function.local_decls[1].ty = ty_id;
            function.ret_type = ty_id;
            if matches!(
                ty,
                Type::Function { .. } | Type::Reference { .. } | Type::Struct { .. }
            ) {
                assert!(
                    return_type_needs_escape_check(&tc, &contract, ty_id),
                    "{ty:?}"
                );
            }
            let analysis = ReferenceEscapeAnalysis::new(
                &function,
                &tc,
                &contract,
                &summaries,
                MissingSummaryMode::Conservative,
            );
            assert_eq!(
                analysis.temporary_origins_for_assignment(&state, &dest, &rvalue),
                if numeric {
                    HashSet::new()
                } else {
                    HashSet::from([Local(2)])
                },
                "{ty:?}"
            );
            assert_eq!(
                analysis.origins_for_operand(&state, &Operand::Copy(input.clone())),
                original[&input]
            );
            assert_eq!(
                state.origins, original,
                "diagnostic screening must not mutate any origin envelope"
            );
        }
    }

    #[test]
    fn temporary_assignment_screen_checks_projected_type_and_keeps_missing_types_conservative() {
        let mut tc = TypeContext::new();
        let mut function = reference_return_function(&mut tc, MirOwnershipMetadata::default());
        let tuple = tc.intern_type(&Type::Tuple(vec![
            Type::I64,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        ]));
        let scalar = tc.intern_type(&Type::I64);
        let contract = MirBackendContract::default();
        let summaries = HashMap::new();
        let input = Place {
            local: Local(1),
            projection: Vec::new(),
        };
        let rvalue = Rvalue::Use(Operand::Copy(input.clone()));
        let mut state = ReferenceOriginState::default();
        state
            .origins
            .insert(input, HashSet::from([ReferenceOrigin::Temporary(Local(2))]));
        let original = state.origins.clone();
        // Malformed projection/type cases exercise conservative fallback only;
        // they are never admitted as valid MIR or executed.
        for (root_type, projection, screened) in [
            (
                tuple,
                vec![Projection::Field {
                    index: 0,
                    identity: None,
                }],
                true,
            ),
            (
                tuple,
                vec![Projection::Field {
                    index: 1,
                    identity: None,
                }],
                false,
            ),
            (
                tuple,
                vec![Projection::Field {
                    index: 2,
                    identity: None,
                }],
                false,
            ),
            (scalar, vec![Projection::Deref], false),
            (crate::ids::TypeId(999_999), Vec::new(), false),
        ] {
            function.local_decls[0].ty = root_type;
            let dest = Place {
                local: Local(0),
                projection,
            };
            let analysis = ReferenceEscapeAnalysis::new(
                &function,
                &tc,
                &contract,
                &summaries,
                MissingSummaryMode::Conservative,
            );
            let checked = contract.place_type_id(&tc, &function, &dest);
            if screened {
                assert_eq!(checked, Some(scalar));
            }
            assert_eq!(
                analysis.temporary_origins_for_assignment(&state, &dest, &rvalue),
                if screened {
                    HashSet::new()
                } else {
                    HashSet::from([Local(2)])
                },
                "{dest:?} checked={checked:?}"
            );
            assert_eq!(state.origins, original);
        }
    }

    #[test]
    fn reference_return_summary_joins_branch_return_origins() {
        let mut type_context = TypeContext::new();
        let ref_i64 = test_type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let bool_id = test_type_id(&mut type_context, Type::Bool);
        let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(122)));
        let function = MirFunction {
            id: function_id.clone(),
            name: "choose_ref".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::SwitchInt {
                        discr: Operand::Copy(Place {
                            local: Local(1),
                            projection: Vec::new(),
                        }),
                        targets: vec![(1, crate::mir::BasicBlockId(1))],
                        otherwise: crate::mir::BasicBlockId(2),
                    }),
                },
                BasicBlock {
                    statements: vec![StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: Vec::new(),
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(2),
                            projection: Vec::new(),
                        })),
                        None,
                    )],
                    terminator: Some(Terminator::Goto(crate::mir::BasicBlockId(3))),
                },
                BasicBlock {
                    statements: vec![StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: Vec::new(),
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(3),
                            projection: Vec::new(),
                        })),
                        None,
                    )],
                    terminator: Some(Terminator::Goto(crate::mir::BasicBlockId(3))),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: bool_id,
                    mutability: Mutability::Not,
                    name: Some("flag".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Not,
                    name: Some("left".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Not,
                    name: Some("right".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 3,
            ret_type: ref_i64,
            ownership: MirOwnershipMetadata::default(),
        };
        let program = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context,
            backend_contract: Default::default(),
        };

        let summaries = BorrowChecker::reference_return_summaries(&program);
        let summary = summaries
            .get(&function_id)
            .expect("choose_ref should have a reference return summary");

        assert!(summary.contains(&ReferenceOrigin::Param(Local(2))));
        assert!(summary.contains(&ReferenceOrigin::Param(Local(3))));
    }

    #[test]
    fn instance_reference_return_summary_maps_receiver_origin() {
        let mut type_context = TypeContext::new();
        let ref_i64 = test_type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let instance_id = InstanceId(902);
        let callee_id = MirFunctionId::Instance(instance_id);
        let caller_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(903)));
        let callee = MirFunction {
            id: callee_id.clone(),
            name: "instance_ref".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    })),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Not,
                    name: Some("receiver".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: ref_i64,
            ownership: MirOwnershipMetadata::default(),
        };
        let caller = MirFunction {
            id: caller_id.clone(),
            name: "caller".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                            MirCallableKey::Instance(instance_id),
                        ))),
                        args: vec![Operand::Copy(Place {
                            local: Local(1),
                            projection: Vec::new(),
                        })],
                        destination: Place {
                            local: Local(0),
                            projection: Vec::new(),
                        },
                        target: crate::mir::BasicBlockId(1),
                        span: Some(crate::lexer::Span::test()),
                    }),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Not,
                    name: Some("receiver".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: ref_i64,
            ownership: MirOwnershipMetadata::default(),
        };
        let program = MirProgram {
            functions: BTreeMap::from([(callee_id, callee), (caller_id.clone(), caller)]),
            type_context,
            backend_contract: Default::default(),
        };

        let summaries = BorrowChecker::reference_return_summaries(&program);
        assert_eq!(
            summaries.get(&caller_id),
            Some(&HashSet::from([ReferenceOrigin::Param(Local(1))]))
        );
    }

    #[test]
    fn temporary_reference_return_call_destination_is_rejected() {
        let mut type_context = TypeContext::new();
        let unit = test_type_id(&mut type_context, Type::Unit);
        let i64_id = test_type_id(&mut type_context, Type::I64);
        let ref_i64 = test_type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let callee_id = DefId::new(CrateId(0), LocalDefId(904));
        let caller_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(905)));
        let callee = MirFunction {
            id: MirFunctionId::Function(callee_id),
            name: "returns_temporary_reference".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Ref(
                        Mutability::Not,
                        Place {
                            local: Local(1),
                            projection: Vec::new(),
                        },
                    ),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Not,
                    name: Some("temporary".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: ref_i64,
            ownership: MirOwnershipMetadata::default(),
        };
        let caller = MirFunction {
            id: caller_id.clone(),
            name: "stores_call_result".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                            MirCallableKey::Function(callee_id),
                        ))),
                        args: Vec::new(),
                        destination: Place {
                            local: Local(1),
                            projection: Vec::new(),
                        },
                        target: crate::mir::BasicBlockId(1),
                        span: Some(crate::lexer::Span::test()),
                    }),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: unit,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Not,
                    name: Some("stored".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: unit,
            ownership: MirOwnershipMetadata::default(),
        };
        let program = MirProgram {
            functions: BTreeMap::from([
                (MirFunctionId::Function(callee_id), callee),
                (caller_id.clone(), caller.clone()),
            ]),
            type_context,
            backend_contract: Default::default(),
        };
        let summaries = BorrowChecker::reference_return_summaries(&program);
        let mut diagnostics = Diagnostics::default();

        BorrowChecker::validate_reference_escapes(
            &caller,
            &program.type_context,
            &MirBackendContract::default(),
            &summaries,
            &mut diagnostics,
        );

        assert!(diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("temporary")));
    }

    #[test]
    fn reference_return_summary_preserves_param_origin_through_pointer_projection() {
        let mut type_context = TypeContext::new();
        let i64_id = type_context.intern_type(&Type::I64);
        let ptr_i64 = type_context.intern_ty(crate::type_context::Ty::Pointer(i64_id));
        let ref_i64 = type_context.intern_ty(crate::type_context::Ty::Reference {
            mutable: false,
            inner: i64_id,
        });
        let index_i64 = type_context.intern_type(&Type::I64);
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(127))),
            name: "project_pointer_ref".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::assign(
                        Place {
                            local: Local(2),
                            projection: Vec::new(),
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(1),
                            projection: Vec::new(),
                        })),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: Vec::new(),
                        },
                        Rvalue::Ref(
                            Mutability::Not,
                            Place {
                                local: Local(2),
                                projection: vec![Projection::Index(Local(3))],
                            },
                        ),
                        None,
                    ),
                ],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: ptr_i64,
                    mutability: Mutability::Not,
                    name: Some("ptr_arg".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
                LocalDecl {
                    ty: ptr_i64,
                    mutability: Mutability::Not,
                    name: Some("ptr_temp".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
                LocalDecl {
                    ty: index_i64,
                    mutability: Mutability::Not,
                    name: Some("idx".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: ref_i64,
            ownership: MirOwnershipMetadata::default(),
        };

        let origins = BorrowChecker::reference_return_origins(
            &function,
            &type_context,
            &MirBackendContract::default(),
            &HashMap::new(),
        );

        assert!(origins.contains(&ReferenceOrigin::Param(Local(1))));
        assert!(!origins.contains(&ReferenceOrigin::Param(Local(2))));
    }

    #[test]
    fn opaque_reference_return_maps_reference_argument_origins() {
        let mut type_context = TypeContext::new();
        let ref_i64 = test_type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let extern_id = DefId::new(CrateId(0), LocalDefId(123));
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(124))),
            name: "extern_passthrough".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                            MirCallableKey::Extern(extern_id),
                        ))),
                        args: vec![Operand::Copy(Place {
                            local: Local(1),
                            projection: Vec::new(),
                        })],
                        destination: Place {
                            local: Local(0),
                            projection: Vec::new(),
                        },
                        target: crate::mir::BasicBlockId(1),
                        span: Some(crate::lexer::Span::test()),
                    }),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Not,
                    name: Some("input".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: ref_i64,
            ownership: MirOwnershipMetadata::default(),
        };
        let mut diagnostics = Diagnostics::default();

        BorrowChecker::validate_reference_escapes(
            &function,
            &type_context,
            &MirBackendContract::default(),
            &HashMap::new(),
            &mut diagnostics,
        );

        assert!(diagnostics.0.is_empty());
    }

    #[test]
    fn reference_return_summary_defers_missing_known_call_summary() {
        let mut type_context = TypeContext::new();
        let ref_i64 = test_type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let callee_id = DefId::new(CrateId(0), LocalDefId(125));
        let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(126)));
        let function = MirFunction {
            id: function_id.clone(),
            name: "calls_later_summarized".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                            MirCallableKey::Function(callee_id),
                        ))),
                        args: vec![Operand::Copy(Place {
                            local: Local(1),
                            projection: Vec::new(),
                        })],
                        destination: Place {
                            local: Local(0),
                            projection: Vec::new(),
                        },
                        target: crate::mir::BasicBlockId(1),
                        span: Some(crate::lexer::Span::test()),
                    }),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Not,
                    name: Some("input".to_string()),
                    span: None,
                    source: LocalSource::Argument,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: ref_i64,
            ownership: MirOwnershipMetadata::default(),
        };
        let key = MirCallableKey::Function(callee_id);
        let callee_function = MirFunctionId::Function(callee_id);
        let mut contract = MirBackendContract::default();
        contract
            .function_bodies
            .insert(callee_function.clone(), key.clone());
        contract.callables.insert(
            key.clone(),
            crate::mir::MirCallableDecl {
                key: key.clone(),
                source_def_id: Some(callee_id),
                kind: crate::mir::MirCallableKind::LocalBody {
                    function_id: callee_function.clone(),
                },
                llvm_symbol: "pending_reference_producer".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[ref_i64],
                    ref_i64,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        // The production fixpoint seeds declared local bodies, including pending
        // empty entries; an absent entry is not that pending-body authority.
        let pending = HashMap::from([(callee_function.clone(), HashSet::new())]);
        let origins =
            BorrowChecker::reference_return_origins(&function, &type_context, &contract, &pending);

        assert!(origins.is_empty());
        let completed = HashMap::from([(
            callee_function.clone(),
            HashSet::from([ReferenceOrigin::Param(Local(1))]),
        )]);
        assert_eq!(
            BorrowChecker::reference_return_origins(
                &function,
                &type_context,
                &contract,
                &completed,
            ),
            HashSet::from([ReferenceOrigin::Param(Local(1))])
        );
        contract.function_bodies.remove(&callee_function);
        contract.callables.get_mut(&key).unwrap().kind =
            crate::mir::MirCallableKind::ObjectProvided;
        assert_eq!(
            BorrowChecker::reference_return_origins(
                &function,
                &type_context,
                &contract,
                &HashMap::new(),
            ),
            HashSet::from([
                ReferenceOrigin::UnknownExternal,
                ReferenceOrigin::Param(Local(1))
            ])
        );
    }

    #[test]
    fn borrowck_reports_parent_use_after_field_move() {
        let mut type_context = TypeContext::new();
        let i64_id = type_context.intern_type(&Type::I64);
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(99))),
            name: "field_move".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::assign(
                        Place {
                            local: Local(2),
                            projection: Vec::new(),
                        },
                        Rvalue::Use(Operand::Move(Place {
                            local: Local(1),
                            projection: vec![Projection::Field {
                                index: 0,
                                identity: None,
                            }],
                        })),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: Vec::new(),
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(1),
                            projection: Vec::new(),
                        })),
                        None,
                    ),
                ],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Mut,
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Mut,
                    name: Some("value".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Mut,
                    name: Some("tmp".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: i64_id,
            ownership: Default::default(),
        };

        let err = BorrowChecker::check_function(&function, &type_context)
            .expect_err("parent use after field move should be rejected");
        assert!(err.0.iter().any(|diag| diag.message.contains("moved")));
    }

    #[test]
    fn borrowck_reports_drop_after_field_move() {
        let mut type_context = TypeContext::new();
        let i64_id = type_context.intern_type(&Type::I64);
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(101))),
            name: "drop_after_field_move".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![StatementData::assign(
                        Place {
                            local: Local(2),
                            projection: Vec::new(),
                        },
                        Rvalue::Use(Operand::Move(Place {
                            local: Local(1),
                            projection: vec![Projection::Field {
                                index: 0,
                                identity: None,
                            }],
                        })),
                        None,
                    )],
                    terminator: Some(Terminator::Drop {
                        place: Place {
                            local: Local(1),
                            projection: Vec::new(),
                        },
                        target: crate::mir::BasicBlockId(1),
                    }),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Mut,
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Mut,
                    name: Some("value".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Mut,
                    name: Some("tmp".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: i64_id,
            ownership: Default::default(),
        };

        let err = BorrowChecker::check_function(&function, &type_context)
            .expect_err("drop after field move should be rejected");
        assert!(err.0.iter().any(|diag| diag.message.contains("moved")));
    }

    #[test]
    fn borrowck_allows_sibling_use_after_field_move() {
        let mut type_context = TypeContext::new();
        let i64_id = type_context.intern_type(&Type::I64);
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(100))),
            name: "field_move_sibling".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::assign(
                        Place {
                            local: Local(2),
                            projection: Vec::new(),
                        },
                        Rvalue::Use(Operand::Move(Place {
                            local: Local(1),
                            projection: vec![Projection::Field {
                                index: 0,
                                identity: None,
                            }],
                        })),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: Vec::new(),
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(1),
                            projection: vec![Projection::Field {
                                index: 1,
                                identity: None,
                            }],
                        })),
                        None,
                    ),
                ],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Mut,
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Mut,
                    name: Some("value".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Mut,
                    name: Some("tmp".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: i64_id,
            ownership: Default::default(),
        };

        BorrowChecker::check_function(&function, &type_context)
            .expect("sibling field remains usable after moving another field");
    }

    #[test]
    fn borrowck_rejects_non_copy_move_out_of_shared_reference() {
        let mut type_context = TypeContext::new();
        let string_def = DefId::new(CrateId(0), LocalDefId(130));
        let string_ty = Type::Struct {
            id: string_def,
            args: Vec::new(),
        };
        let string_id = type_context.intern_type(&string_ty);
        let ref_string_id = type_context.intern_type(&Type::Reference {
            mutable: false,
            inner: Box::new(string_ty),
        });
        let function = move_or_copy_out_of_reference_function(
            string_id,
            ref_string_id,
            Operand::Move(Place {
                local: Local(1),
                projection: vec![Projection::Deref],
            }),
        );
        let mut backend_contract = MirBackendContract::default();
        backend_contract.nominal_layouts.insert(
            string_def,
            MirNominalLayout::Struct {
                id: string_def,
                fields: Vec::new(),
                generic_params: Vec::new(),
            },
        );

        let err = BorrowChecker::check_function_with_contract(
            &function,
            &type_context,
            &backend_contract,
            &HashMap::new(),
        )
        .expect_err("moving a non-copy value out of a shared reference should be rejected");

        assert!(err
            .0
            .iter()
            .any(|diag| diag.message.contains("Cannot move non-copy value of type")));
    }

    #[test]
    fn borrowck_allows_copy_out_of_shared_reference() {
        let mut type_context = TypeContext::new();
        let i64_id = type_context.intern_type(&Type::I64);
        let ref_i64_id = type_context.intern_type(&Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        });
        let function = move_or_copy_out_of_reference_function(
            i64_id,
            ref_i64_id,
            Operand::Copy(Place {
                local: Local(1),
                projection: vec![Projection::Deref],
            }),
        );

        BorrowChecker::check_function(&function, &type_context)
            .expect("copying I64 out of a shared reference should be allowed");
    }

    #[test]
    fn borrowck_rejects_non_copy_copy_out_of_shared_reference() {
        let mut type_context = TypeContext::new();
        let pair_def = DefId::new(CrateId(0), LocalDefId(132));
        let i64_id = type_context.intern_type(&Type::I64);
        let pair_ty = Type::Struct {
            id: pair_def,
            args: Vec::new(),
        };
        let pair_id = type_context.intern_type(&pair_ty);
        let ref_pair_id = type_context.intern_type(&Type::Reference {
            mutable: false,
            inner: Box::new(pair_ty),
        });
        let function = move_or_copy_out_of_reference_function(
            pair_id,
            ref_pair_id,
            Operand::Copy(Place {
                local: Local(1),
                projection: vec![Projection::Deref],
            }),
        );
        let mut backend_contract = MirBackendContract::default();
        backend_contract.nominal_layouts.insert(
            pair_def,
            MirNominalLayout::Struct {
                id: pair_def,
                fields: vec![("x".to_string(), i64_id)],
                generic_params: Vec::new(),
            },
        );

        let err = BorrowChecker::check_function_with_contract(
            &function,
            &type_context,
            &backend_contract,
            &HashMap::new(),
        )
        .expect_err("copy operand of non-copy value out of shared reference should be rejected");

        assert!(err
            .0
            .iter()
            .any(|diag| diag.message.contains("Cannot move non-copy value of type")));
    }

    #[test]
    fn borrowck_field_projection_uses_contract_not_legacy_metadata() {
        let mut type_context = TypeContext::new();
        let outer_def = DefId::new(CrateId(0), LocalDefId(901));
        let inner_def = DefId::new(CrateId(0), LocalDefId(902));
        let outer_ty = Type::Struct {
            id: outer_def,
            args: Vec::new(),
        };
        let inner_ty = Type::Struct {
            id: inner_def,
            args: Vec::new(),
        };
        let inner_id = type_context.intern_type(&inner_ty);
        let outer_ref_id = type_context.intern_type(&Type::Reference {
            mutable: false,
            inner: Box::new(outer_ty.clone()),
        });
        let function = move_or_copy_out_of_reference_function(
            inner_id,
            outer_ref_id,
            Operand::Move(Place {
                local: Local(1),
                projection: vec![
                    Projection::Deref,
                    Projection::Field {
                        index: 0,
                        identity: None,
                    },
                ],
            }),
        );
        let mut backend_contract = MirBackendContract::default();
        backend_contract.nominal_layouts.insert(
            outer_def,
            MirNominalLayout::Struct {
                id: outer_def,
                fields: vec![("field".to_string(), inner_id)],
                generic_params: Vec::new(),
            },
        );
        backend_contract.nominal_layouts.insert(
            inner_def,
            MirNominalLayout::Struct {
                id: inner_def,
                fields: Vec::new(),
                generic_params: Vec::new(),
            },
        );

        let err = BorrowChecker::check_function_with_contract(
            &function,
            &type_context,
            &backend_contract,
            &HashMap::new(),
        )
        .expect_err("contract field type should make the projected move non-copy");

        assert!(err
            .0
            .iter()
            .any(|diag| diag.message.contains("Cannot move non-copy value")));
    }

    #[test]
    fn borrowck_rejects_field_move_from_projected_drop_struct_output() {
        let mut type_context = TypeContext::new();
        let trait_id = DefId::new(CrateId(0), LocalDefId(920));
        let assoc_type_id = AssocTypeId(0);
        let drop_def = DefId::new(CrateId(0), LocalDefId(921));
        let base_id = type_context.intern_type(&Type::I64);
        let field_id = type_context.intern_type(&Type::I64);
        let drop_ty = Type::Struct {
            id: drop_def,
            args: Vec::new(),
        };
        let drop_id = type_context.intern_type(&drop_ty);
        let projection_id = type_context.intern_type(&Type::Projection {
            ty: Box::new(Type::I64),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: Vec::new(),
        });
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(922))),
            name: "projected_drop_struct_field_move".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Move(Place {
                        local: Local(1),
                        projection: vec![Projection::Field {
                            index: 0,
                            identity: None,
                        }],
                    })),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: field_id,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: projection_id,
                    mutability: Mutability::Not,
                    name: Some("projected".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
                LocalDecl {
                    ty: field_id,
                    mutability: Mutability::Mut,
                    name: Some("field".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: field_id,
            ownership: MirOwnershipMetadata::default(),
        };
        let mut backend_contract = MirBackendContract::default();
        backend_contract.projection_outputs.insert(
            MirProjectionKey {
                base: base_id,
                trait_id,
                assoc_type_id,
                trait_args: Vec::new(),
            },
            drop_id,
        );
        backend_contract.drop_glue.insert(
            drop_id,
            crate::mir::MirCallableKey::Function(DefId::new(CrateId(0), LocalDefId(923))),
        );
        backend_contract.nominal_layouts.insert(
            drop_def,
            MirNominalLayout::Struct {
                id: drop_def,
                fields: vec![("value".to_string(), field_id)],
                generic_params: Vec::new(),
            },
        );

        let err = BorrowChecker::check_function_with_contract(
            &function,
            &type_context,
            &backend_contract,
            &HashMap::new(),
        )
        .expect_err("moving a field out of projected Drop output should be rejected");

        assert!(err
            .0
            .iter()
            .any(|diag| diag.message.contains("because it implements Drop")));
    }

    #[test]
    fn borrowck_rejects_array_element_move_from_projected_drop_array_output() {
        let mut type_context = TypeContext::new();
        let trait_id = DefId::new(CrateId(0), LocalDefId(924));
        let assoc_type_id = AssocTypeId(0);
        let drop_def = DefId::new(CrateId(0), LocalDefId(925));
        let base_id = type_context.intern_type(&Type::I64);
        let i64_id = type_context.intern_type(&Type::I64);
        let drop_ty = Type::Struct {
            id: drop_def,
            args: Vec::new(),
        };
        let drop_id = type_context.intern_type(&drop_ty);
        let array_id = type_context.intern_type(&Type::Array(Box::new(drop_ty), 2));
        let projection_id = type_context.intern_type(&Type::Projection {
            ty: Box::new(Type::I64),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: Vec::new(),
        });
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(926))),
            name: "projected_drop_array_element_move".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Move(Place {
                        local: Local(1),
                        projection: vec![Projection::Index(Local(3))],
                    })),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: drop_id,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: projection_id,
                    mutability: Mutability::Not,
                    name: Some("array".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
                LocalDecl {
                    ty: drop_id,
                    mutability: Mutability::Mut,
                    name: Some("element".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Not,
                    name: Some("index".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: drop_id,
            ownership: MirOwnershipMetadata::default(),
        };
        let mut backend_contract = MirBackendContract::default();
        backend_contract.projection_outputs.insert(
            MirProjectionKey {
                base: base_id,
                trait_id,
                assoc_type_id,
                trait_args: Vec::new(),
            },
            array_id,
        );
        backend_contract.drop_glue.insert(
            drop_id,
            crate::mir::MirCallableKey::Function(DefId::new(CrateId(0), LocalDefId(927))),
        );
        backend_contract.nominal_layouts.insert(
            drop_def,
            MirNominalLayout::Struct {
                id: drop_def,
                fields: Vec::new(),
                generic_params: Vec::new(),
            },
        );

        let err = BorrowChecker::check_function_with_contract(
            &function,
            &type_context,
            &backend_contract,
            &HashMap::new(),
        )
        .expect_err("moving projected Drop array element should be rejected");

        assert!(err.0.iter().any(|diag| diag
            .message
            .contains("Cannot move cleanup value out of array element")));
    }

    #[test]
    fn borrowck_rejects_move_out_of_projected_reference_output() {
        let mut type_context = TypeContext::new();
        let trait_id = DefId::new(CrateId(0), LocalDefId(928));
        let assoc_type_id = AssocTypeId(0);
        let payload_def = DefId::new(CrateId(0), LocalDefId(929));
        let base_id = type_context.intern_type(&Type::I64);
        let payload_ty = Type::Struct {
            id: payload_def,
            args: Vec::new(),
        };
        let payload_id = type_context.intern_type(&payload_ty);
        let projection_output_id = type_context.intern_type(&Type::Reference {
            mutable: false,
            inner: Box::new(payload_ty.clone()),
        });
        let projection_id = type_context.intern_type(&Type::Projection {
            ty: Box::new(Type::I64),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: Vec::new(),
        });
        let function = move_or_copy_out_of_reference_function(
            payload_id,
            projection_id,
            Operand::Move(Place {
                local: Local(1),
                projection: vec![Projection::Deref],
            }),
        );
        let mut backend_contract = MirBackendContract::default();
        backend_contract.projection_outputs.insert(
            MirProjectionKey {
                base: base_id,
                trait_id,
                assoc_type_id,
                trait_args: Vec::new(),
            },
            projection_output_id,
        );
        backend_contract.nominal_layouts.insert(
            payload_def,
            MirNominalLayout::Struct {
                id: payload_def,
                fields: vec![("value".to_string(), base_id)],
                generic_params: Vec::new(),
            },
        );

        let err = BorrowChecker::check_function_with_contract(
            &function,
            &type_context,
            &backend_contract,
            &HashMap::new(),
        )
        .expect_err("moving non-copy projected reference output should be rejected");

        assert!(err
            .0
            .iter()
            .any(|diag| diag.message.contains("Cannot move non-copy value of type")));
    }

    #[test]
    fn borrowck_allows_raw_pointer_move_after_reading_pointer_through_reference() {
        let mut type_context = TypeContext::new();
        let pair_def = DefId::new(CrateId(0), LocalDefId(133));
        let pair_ty = Type::Struct {
            id: pair_def,
            args: Vec::new(),
        };
        let pair_id = type_context.intern_type(&pair_ty);
        let ref_ptr_pair_id = type_context.intern_type(&Type::Reference {
            mutable: false,
            inner: Box::new(Type::Pointer(Box::new(pair_ty))),
        });
        let function = move_or_copy_out_of_reference_function(
            pair_id,
            ref_ptr_pair_id,
            Operand::Move(Place {
                local: Local(1),
                projection: vec![Projection::Deref, Projection::Deref],
            }),
        );

        BorrowChecker::check_function(&function, &type_context)
            .expect("raw-pointer pointee moves are not moves out of the reference");
    }

    #[test]
    fn raw_reinterpret_read_keeps_shared_loan_but_not_exclusive_move_access() {
        let mut tc = TypeContext::new();
        let i64 = tc.intern_type(&Type::I64);
        let reference = tc.intern_type(&Type::Reference {
            inner: Box::new(Type::I64),
            mutable: false,
        });
        let pointer = tc.intern_type(&Type::Pointer(Box::new(Type::I64)));
        let mut function = reference_return_function(&mut tc, MirOwnershipMetadata::default());
        function.arg_count = 1;
        function.ret_type = i64;
        function.local_decls.truncate(2);
        function.local_decls[0].ty = i64;
        function.local_decls[1].ty = i64;
        function.local_decls[1].source = LocalSource::Argument;
        function.local_decls.push(LocalDecl {
            ty: reference,
            source: LocalSource::Temporary,
            mutability: Mutability::Not,
            name: None,
            span: None,
        });
        function.local_decls.push(LocalDecl {
            ty: pointer,
            source: LocalSource::Temporary,
            mutability: Mutability::Not,
            name: None,
            span: None,
        });
        let place = |index| Place {
            local: Local(index),
            projection: Vec::new(),
        };
        function.basic_blocks[0].statements = vec![
            StatementData::assign(place(2), Rvalue::Ref(Mutability::Not, place(1)), None),
            StatementData::assign(
                place(3),
                Rvalue::Cast(Operand::Copy(place(2)), pointer),
                None,
            ),
            StatementData::assign(
                place(0),
                Rvalue::Use(Operand::Move(Place {
                    local: Local(3),
                    projection: vec![Projection::Deref],
                })),
                None,
            ),
        ];
        BorrowChecker::check_function(&function, &tc)
            .expect("raw reinterpret load is a read of the shared source");
        assert_eq!(
            BorrowChecker::move_access_kind(
                &tc,
                &MirBackendContract::default(),
                &function,
                &Place {
                    local: Local(3),
                    projection: vec![Projection::Deref]
                }
            ),
            super::LoanKind::Shared
        );
        assert_eq!(
            BorrowChecker::move_access_kind(
                &tc,
                &MirBackendContract::default(),
                &function,
                &Place {
                    local: Local(2),
                    projection: vec![Projection::Deref]
                }
            ),
            super::LoanKind::Mut
        );
        assert_eq!(
            BorrowChecker::move_access_kind(
                &tc,
                &MirBackendContract::default(),
                &function,
                &place(1)
            ),
            super::LoanKind::Mut
        );
    }

    fn immutable_binding_mut_borrow_function(
        type_context: &mut TypeContext,
        binding_name: Option<&str>,
    ) -> MirFunction {
        let unit = test_type_id(type_context, Type::Unit);
        let i64_id = test_type_id(type_context, Type::I64);
        let mut_ref_i64 = test_type_id(
            type_context,
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::I64),
            },
        );

        MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(120))),
            name: "mut_borrow_immutable".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: Vec::new(),
                    },
                    Rvalue::Ref(
                        Mutability::Mut,
                        Place {
                            local: Local(1),
                            projection: Vec::new(),
                        },
                    ),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: unit,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Not,
                    name: binding_name.map(str::to_string),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: mut_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("borrow".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: unit,
            ownership: MirOwnershipMetadata::default(),
        }
    }

    fn drop_obligation_function(
        tracked_id: crate::ids::TypeId,
        ownership: MirOwnershipMetadata,
    ) -> MirFunction {
        let unit_id = tracked_id;
        MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(123))),
            name: "drop_obligation".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: unit_id,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: tracked_id,
                    mutability: Mutability::Mut,
                    name: Some("tracked".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: unit_id,
            ownership,
        }
    }

    fn reference_return_function(
        type_context: &mut TypeContext,
        ownership: MirOwnershipMetadata,
    ) -> MirFunction {
        let ref_i64 = test_type_id(
            type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let i64_id = test_type_id(type_context, Type::I64);

        MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(121))),
            name: "reference_return".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: ref_i64,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Not,
                    name: Some("value".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Not,
                    name: Some("temporary".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: ref_i64,
            ownership,
        }
    }

    fn move_or_copy_out_of_reference_function(
        value_type: crate::ids::TypeId,
        reference_type: crate::ids::TypeId,
        source: Operand,
    ) -> MirFunction {
        MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(131))),
            name: "move_or_copy_out_of_reference".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(source),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: value_type,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: reference_type,
                    mutability: Mutability::Not,
                    name: Some("reference".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
                LocalDecl {
                    ty: value_type,
                    mutability: Mutability::Mut,
                    name: Some("destination".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: value_type,
            ownership: MirOwnershipMetadata::default(),
        }
    }

    fn mut_borrow_then_shared_read_function(type_context: &mut TypeContext) -> MirFunction {
        let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0)));
        let local_decls = vec![
            LocalDecl {
                ty: test_type_id(type_context, Type::Unit),
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: test_type_id(type_context, Type::I64),
                mutability: Mutability::Mut,
                name: Some("x".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: test_type_id(
                    type_context,
                    Type::Reference {
                        inner: Box::new(Type::I64),
                        mutable: true,
                    },
                ),
                mutability: Mutability::Not,
                name: Some("r".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: test_type_id(
                    type_context,
                    Type::Reference {
                        inner: Box::new(Type::I64),
                        mutable: false,
                    },
                ),
                mutability: Mutability::Not,
                name: Some("s".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: test_type_id(
                    type_context,
                    Type::Reference {
                        inner: Box::new(Type::I64),
                        mutable: true,
                    },
                ),
                mutability: Mutability::Not,
                name: Some("keep_r_live".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ];

        MirFunction {
            id: function_id,
            name: "mut_borrow_then_shared_read".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::assign(
                        Place {
                            local: Local(2),
                            projection: vec![],
                        },
                        Rvalue::Ref(
                            Mutability::Mut,
                            Place {
                                local: Local(1),
                                projection: vec![],
                            },
                        ),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(3),
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
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(4),
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(2),
                            projection: vec![],
                        })),
                        None,
                    ),
                ],
                terminator: Some(Terminator::Return),
            }],
            local_decls,
            closure_captures: vec![],
            arg_count: 1,
            ret_type: test_type_id(type_context, Type::Unit),
            ownership: Default::default(),
        }
    }

    #[test]
    fn assert_operand_read_conflicts_with_active_mut_borrow() {
        let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0)));
        let mut type_context = TypeContext::new();
        let local_decls = vec![
            LocalDecl {
                ty: test_type_id(&mut type_context, Type::Unit),
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: test_type_id(&mut type_context, Type::I64),
                mutability: Mutability::Mut,
                name: Some("x".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: test_type_id(
                    &mut type_context,
                    Type::Reference {
                        inner: Box::new(Type::I64),
                        mutable: true,
                    },
                ),
                mutability: Mutability::Not,
                name: Some("r".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: test_type_id(
                    &mut type_context,
                    Type::Reference {
                        inner: Box::new(Type::I64),
                        mutable: true,
                    },
                ),
                mutability: Mutability::Not,
                name: Some("keep_r_live".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ];
        let block = BasicBlock {
            statements: vec![
                StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: vec![],
                    },
                    Rvalue::Ref(
                        Mutability::Mut,
                        Place {
                            local: Local(1),
                            projection: vec![],
                        },
                    ),
                    None,
                ),
                StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![Operand::Copy(Place {
                            local: Local(1),
                            projection: vec![],
                        })],
                    },
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(3),
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(2),
                        projection: vec![],
                    })),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        };
        let program = MirProgram {
            functions: BTreeMap::from([(
                function_id.clone(),
                MirFunction {
                    id: function_id,
                    name: "assert_read_conflict".to_string(),
                    basic_blocks: vec![block],
                    local_decls,
                    closure_captures: vec![],
                    arg_count: 1,
                    ret_type: test_type_id(&mut type_context, Type::Unit),
                    ownership: Default::default(),
                },
            )]),
            type_context,
            backend_contract: Default::default(),
        };

        let diagnostics = BorrowChecker::run(&program).expect_err("assert read should conflict");

        assert!(diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("borrow")));
    }

    #[test]
    fn assert_move_operand_read_does_not_conflict_with_active_shared_borrow() {
        let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0)));
        let mut type_context = TypeContext::new();
        let local_decls = vec![
            LocalDecl {
                ty: test_type_id(&mut type_context, Type::Unit),
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: test_type_id(&mut type_context, Type::I64),
                mutability: Mutability::Not,
                name: Some("x".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: test_type_id(
                    &mut type_context,
                    Type::Reference {
                        inner: Box::new(Type::I64),
                        mutable: false,
                    },
                ),
                mutability: Mutability::Not,
                name: Some("r".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: test_type_id(
                    &mut type_context,
                    Type::Reference {
                        inner: Box::new(Type::I64),
                        mutable: false,
                    },
                ),
                mutability: Mutability::Not,
                name: Some("keep_r_live".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ];
        let block = BasicBlock {
            statements: vec![
                StatementData::assign(
                    Place {
                        local: Local(2),
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
                ),
                StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![Operand::Move(Place {
                            local: Local(1),
                            projection: vec![],
                        })],
                    },
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(3),
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(2),
                        projection: vec![],
                    })),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        };
        let program = MirProgram {
            functions: BTreeMap::from([(
                function_id.clone(),
                MirFunction {
                    id: function_id,
                    name: "assert_move_read_shared_borrow".to_string(),
                    basic_blocks: vec![block],
                    local_decls,
                    closure_captures: vec![],
                    arg_count: 1,
                    ret_type: test_type_id(&mut type_context, Type::Unit),
                    ownership: Default::default(),
                },
            )]),
            type_context,
            backend_contract: Default::default(),
        };

        BorrowChecker::run(&program).expect("assert move operand should be read-only");
    }
}
