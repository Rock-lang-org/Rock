//! Compile-local must-evidence for copied primitive slice data, not an ABI proof.
//! A raw parameter is a conditional helper input, never borrowed backing by itself.

use std::collections::{HashMap, HashSet};

use crate::ids::TypeId;
use crate::mir::dataflow::{run_fixpoint, Analysis, Lattice};
use crate::mir::{
    AggregateKind, Constant, Local, LocalSource, MirBackendContract, MirCallable, MirCallableKey,
    MirCallableKind, MirFunction, MirFunctionId, MirIntrinsicId, MirProgram, Mutability, Operand,
    Place, Projection, Rvalue, StatementData, StatementKind, Terminator,
};
use crate::type_context::{Ty, TypeContext, TypeView};
use crate::types::Type;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Source {
    Slice {
        parameter: Local,
        load: Vec<Projection>,
        reference: TypeId,
    },
    PointerInput {
        parameter: Local,
    },
}

impl Source {
    pub(super) fn parameter(&self) -> Local {
        match self {
            Self::Slice { parameter, .. } | Self::PointerInput { parameter } => *parameter,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Slot,
    Slice,
    Pointer,
    View,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Value {
    stage: Stage,
    ty: TypeId,
    sources: HashSet<Source>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReturnFlow {
    body: MirFunctionId,
    value: Value,
}

#[derive(Default, PartialEq, Eq)]
pub(super) struct NativeReturnSources {
    flows: HashMap<MirCallableKey, ReturnFlow>,
    // Ancillary scalar helpers checked on every returning path. This is not
    // mutation knowledge: divergent paths and opaque callees remain unknown.
    scalar_returns: HashSet<MirCallableKey>,
    capture_free_calls: HashMap<(MirFunctionId, Place), super::call_targets::WitnessedCall>,
    // Callable producer value dependencies are not ordinary storage loans.
    pub(super) scalar_lifetime_dependencies: HashMap<MirFunctionId, super::OriginSet>,
    pub(super) scalar_value_dependencies: HashMap<MirFunctionId, super::OriginSet>,
    pub(super) callable_results: super::callable_results::CertifiedCalls,
}

impl NativeReturnSources {
    pub(super) fn witnessed_callable(
        &self,
        body: &MirFunction,
        function: &Operand,
        destination: &Place,
    ) -> Option<&MirCallable> {
        self.capture_free_calls
            .get(&(body.id.clone(), destination.clone()))?
            .target(function)
    }
    pub(super) fn scalar_dependencies(
        &self,
        purpose: super::OriginPurpose,
        id: &MirFunctionId,
    ) -> Option<&super::OriginSet> {
        match purpose {
            super::OriginPurpose::Lifetime => self.scalar_lifetime_dependencies.get(id),
            super::OriginPurpose::ValueLoans => self.scalar_value_dependencies.get(id),
        }
    }

    pub(super) fn certify_callable_results(
        &mut self,
        program: &MirProgram,
        lifetime: &HashMap<MirFunctionId, super::OriginSet>,
        values: &HashMap<MirFunctionId, super::OriginSet>,
    ) -> bool {
        let next =
            super::callable_results::CertifiedCalls::for_program(program, lifetime, values, self);
        let changed = next != self.callable_results;
        self.callable_results = next;
        changed
    }

    pub(super) fn callable_result_call(
        &self,
        body: &MirFunction,
        tc: &TypeContext,
        contract: &MirBackendContract,
        func: &Operand,
        args: &[Operand],
        destination: &Place,
    ) -> Option<(Vec<Operand>, super::OriginSet)> {
        self.callable_result_for_purpose(
            body,
            tc,
            contract,
            func,
            args,
            destination,
            super::OriginPurpose::Lifetime,
        )
    }

    pub(super) fn callable_result_loan_call(
        &self,
        body: &MirFunction,
        tc: &TypeContext,
        contract: &MirBackendContract,
        func: &Operand,
        args: &[Operand],
        destination: &Place,
    ) -> Option<(Vec<Operand>, super::OriginSet)> {
        self.callable_result_for_purpose(
            body,
            tc,
            contract,
            func,
            args,
            destination,
            super::OriginPurpose::ValueLoans,
        )
    }

    pub(super) fn callable_result_for_purpose(
        &self,
        body: &MirFunction,
        tc: &TypeContext,
        contract: &MirBackendContract,
        func: &Operand,
        args: &[Operand],
        destination: &Place,
        purpose: super::OriginPurpose,
    ) -> Option<(Vec<Operand>, super::OriginSet)> {
        self.callable_results
            .instantiate(body, tc, contract, func, args, destination, purpose)
    }

    pub(super) fn for_program(program: &MirProgram) -> Self {
        let tc = &program.type_context;
        let contract = &program.backend_contract;
        if !crate::mir::backend_contract::validate_backend_contract_against_mir(
            contract,
            program.functions.values(),
        )
        .is_empty()
        {
            return Self::default();
        }
        let mut table = Self::default();
        loop {
            let mut next = Self::default();
            for (id, body) in program.functions() {
                if body.basic_blocks.is_empty() || !body.closure_captures.is_empty() {
                    continue;
                }
                let Some(key) = contract.function_bodies.get(id) else {
                    continue;
                };
                let Some(header) = contract.callable(key) else {
                    continue;
                };
                if &body.id != id
                    || &header.key != key
                    || MirCallable::Resolved(key.clone()).function_id().as_ref() != Some(id)
                    || !matches!(&header.kind, MirCallableKind::LocalBody { function_id } if function_id == id)
                    || !body
                        .local_decls
                        .iter()
                        .all(|decl| tc.type_id_tree_is_valid(decl.ty))
                {
                    continue;
                }
                let analysis = SourceAnalysis {
                    body,
                    tc,
                    contract,
                    table: &table,
                };
                let results = run_fixpoint(&analysis, body);
                let mut returned: Option<Value> = None;
                let mut complete = true;
                let mut returning = false;
                let mut scalar_complete = true;
                for (block, data) in body.basic_blocks.iter().enumerate() {
                    if !results.reachable[block]
                        || !matches!(
                            data.terminator,
                            Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
                        )
                    {
                        continue;
                    }
                    let state = &results.exit_sets[block];
                    returning = true;
                    scalar_complete &= !state.blocked && !state.unknown_calls;
                    let value = state
                        .locals
                        .as_ref()
                        .and_then(|locals| locals.first())
                        .and_then(Option::as_ref);
                    let Some(value) = value.filter(|value| {
                        !state.blocked && value.ty == body.ret_type && !value.sources.is_empty()
                    }) else {
                        complete = false;
                        continue;
                    };
                    if let Some(previous) = &mut returned {
                        if previous.stage != value.stage || previous.ty != value.ty {
                            complete = false;
                            continue;
                        }
                        previous.sources.extend(value.sources.iter().cloned());
                    } else {
                        returned = Some(value.clone());
                    }
                }
                if returning && scalar_complete && scalar_type(tc, contract, body.ret_type)
                    && header.signature.params.iter().all(|parameter| {
                        scalar_type(tc, contract, parameter.semantic_ty)
                            || matches!(tc.try_ty(parameter.semantic_ty),
                                Some(Ty::Reference { mutable: false, inner }) if primitive(tc, *inner))
                    }) {
                    next.scalar_returns.insert(key.clone());
                }
                if complete {
                    if let Some(value) = returned {
                        next.flows.insert(
                            key.clone(),
                            ReturnFlow {
                                body: id.clone(),
                                value,
                            },
                        );
                    }
                }
            }
            if next == table {
                next.capture_free_calls = super::call_targets::witnessed_calls(program);
                return next;
            }
            table = next;
        }
    }

    // Only this admission is shared by Lifetime, loan collection and transport.
    // Conditional pointer/helper entries are inaccessible to caller precision.
    pub(super) fn borrowed_view_call<'a>(
        &'a self,
        body: &MirFunction,
        tc: &TypeContext,
        contract: &MirBackendContract,
        callee: &Operand,
        args: &[Operand],
        destination: &Place,
    ) -> Option<&'a HashSet<Source>> {
        let (key, ret) = checked_call(body, tc, contract, callee, args, destination)?;
        let flow = self.flows.get(key)?;
        if contract.function_bodies.get(&flow.body) != Some(key)
            || flow.value.ty != ret
            || flow.value.stage != Stage::View
            || !flow
                .value
                .sources
                .iter()
                .all(|source| matches!(source, Source::Slice { .. }))
        {
            return None;
        }
        for source in &flow.value.sources {
            let argument = args.get(source.parameter().0.checked_sub(1)?)?;
            let (Operand::Copy(place) | Operand::Move(place)) = argument else {
                return None;
            };
            // Flat carrier roles do not select fields of an owning aggregate.
            // Precision is limited to one whole, checked safe-reference value.
            if !place.projection.is_empty() {
                return None;
            }
        }
        Some(&flow.value.sources)
    }

    #[cfg(test)]
    pub(super) fn has_body_evidence(&self, key: &MirCallableKey) -> bool {
        self.flows.contains_key(key) || self.scalar_returns.contains(key)
    }
}

fn scalar_type(tc: &TypeContext, contract: &MirBackendContract, ty: TypeId) -> bool {
    tc.type_id_tree_is_valid(ty) && !super::type_id_tracks_origin(TypeView::new(tc), contract, ty)
}

fn primitive(tc: &TypeContext, ty: TypeId) -> bool {
    matches!(
        tc.try_ty(ty),
        Some(
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
                | Ty::Unit
        )
    )
}

fn slice_element(tc: &TypeContext, ty: TypeId) -> Option<TypeId> {
    let Ty::Reference {
        mutable: false,
        inner,
    } = tc.try_ty(ty)?
    else {
        return None;
    };
    let Ty::Slice(element) = tc.try_ty(*inner)? else {
        return None;
    };
    primitive(tc, *element).then_some(*element)
}

fn place_type(
    body: &MirFunction,
    tc: &TypeContext,
    contract: &MirBackendContract,
    place: &Place,
) -> Option<TypeId> {
    let ty = contract.place_type_id(tc, body, place)?;
    tc.type_id_tree_is_valid(ty).then_some(ty)
}

fn operand_type(
    body: &MirFunction,
    tc: &TypeContext,
    contract: &MirBackendContract,
    operand: &Operand,
) -> Option<TypeId> {
    let ty = match operand {
        Operand::Copy(place) | Operand::Move(place) => place_type(body, tc, contract, place),
        Operand::Constant(Constant::Int(_)) => tc.id_for_type(&Type::I64),
        Operand::Constant(Constant::Bool(_)) => tc.id_for_type(&Type::Bool),
        Operand::Constant(Constant::Float(_)) => tc.id_for_type(&Type::F64),
        Operand::Constant(Constant::Char(_)) => tc.id_for_type(&Type::Char),
        Operand::Constant(Constant::Unit) => tc.id_for_type(&Type::Unit),
        _ => None,
    }?;
    tc.type_id_tree_is_valid(ty).then_some(ty)
}

pub(super) fn checked_call<'a>(
    body: &MirFunction,
    tc: &TypeContext,
    contract: &'a MirBackendContract,
    callee: &'a Operand,
    args: &[Operand],
    destination: &Place,
) -> Option<(&'a MirCallableKey, TypeId)> {
    let (key, ret) = checked_signature(body, tc, contract, callee, args, destination)?;
    if !matches!(key, MirCallableKey::Intrinsic(_))
        && !matches!(
            &contract.callable(key)?.kind,
            MirCallableKind::LocalBody { .. }
        )
    {
        return None;
    }
    Some((key, ret))
}

// Opaque signatures can establish scalar value-flow independence, never a
// source-producing body or an effect summary. Source extraction uses checked_call.
pub(super) fn checked_signature<'a>(
    body: &MirFunction,
    tc: &TypeContext,
    contract: &'a MirBackendContract,
    callee: &'a Operand,
    args: &[Operand],
    destination: &Place,
) -> Option<(&'a MirCallableKey, TypeId)> {
    let Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) = callee else {
        return None;
    };
    let header = contract.callable(key)?;
    let ret = place_type(body, tc, contract, destination)?;
    if &header.key != key || !destination.projection.is_empty() {
        return None;
    }
    if let MirCallableKey::Intrinsic(intrinsic) = key {
        if !matches!(&header.kind, MirCallableKind::Intrinsic(actual) if actual == intrinsic)
            || header.signature.params.len() != args.len()
        {
            return None;
        }
        // Intrinsic headers are representative; the checked operands/result
        // below supply the real instantiation, never representative equality.
        for argument in args {
            operand_type(body, tc, contract, argument)?;
        }
    } else {
        match &header.kind {
            MirCallableKind::LocalBody { function_id } => {
                if contract.function_bodies.get(function_id) != Some(key)
                    || MirCallable::Resolved(key.clone()).function_id().as_ref()
                        != Some(function_id)
                {
                    return None;
                }
            }
            MirCallableKind::ObjectProvided | MirCallableKind::Extern { .. } => {}
            _ => return None,
        }
        if contract.callable_result_storage(header) != ret
            || header.signature.params.len() != args.len()
        {
            return None;
        }
        for (argument, parameter) in args.iter().zip(&header.signature.params) {
            if operand_type(body, tc, contract, argument)? != parameter.semantic_ty {
                return None;
            }
        }
    }
    Some((key, ret))
}

/// Body-local addresses, not return sources, loan permissions or effect evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ScalarAddress {
    ty: TypeId,
    paths: HashSet<Place>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FreshStorage {
    initialized: Vec<Option<TypeId>>,
    references: Vec<Option<ScalarAddress>>,
}

impl FreshStorage {
    fn new(count: usize) -> Self {
        Self {
            initialized: vec![None; count],
            references: vec![None; count],
        }
    }

    fn join(&mut self, other: &Self) {
        for (initialized, incoming) in self.initialized.iter_mut().zip(&other.initialized) {
            if *initialized != *incoming {
                *initialized = None;
            }
        }
        for (reference, incoming) in self.references.iter_mut().zip(&other.references) {
            match (reference.as_mut(), incoming) {
                (Some(address), Some(incoming)) if address.ty == incoming.ty => {
                    address.paths.extend(incoming.paths.iter().cloned());
                }
                _ => *reference = None,
            }
        }
    }

    fn clear(&mut self) {
        self.initialized.fill(None);
        self.references.fill(None);
    }

    fn kill(&mut self, local: Local) {
        self.initialized[local.0] = None;
        self.references[local.0] = None;
        for reference in &mut self.references {
            if reference
                .as_ref()
                .is_some_and(|address| address.paths.iter().any(|path| path.local == local))
            {
                *reference = None;
            }
        }
    }

    fn move_operand(&mut self, operand: &Operand) {
        if let Operand::Move(place) = operand {
            self.kill(place.local);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    // None is unreachable bottom. Per-local None is an uncertified alternative.
    locals: Option<Vec<Option<Value>>>,
    fresh: Option<FreshStorage>,
    blocked: bool,
    unknown_calls: bool,
}

impl Lattice for State {
    fn join(&mut self, other: &Self) -> bool {
        let Some(incoming) = &other.locals else {
            return false;
        };
        let old = self.clone();
        if let Some(locals) = &mut self.locals {
            for (local, incoming) in locals.iter_mut().zip(incoming) {
                match (local.as_mut(), incoming) {
                    (Some(value), Some(incoming))
                        if value.stage == incoming.stage && value.ty == incoming.ty =>
                    {
                        value.sources.extend(incoming.sources.iter().cloned());
                    }
                    _ => *local = None,
                }
            }
        } else {
            self.locals = Some(incoming.clone());
        }
        if let Some(incoming) = &other.fresh {
            if let Some(fresh) = &mut self.fresh {
                fresh.join(incoming);
            } else {
                self.fresh = Some(incoming.clone());
            }
        }
        self.blocked |= other.blocked;
        self.unknown_calls |= other.unknown_calls;
        *self != old
    }
}

struct SourceAnalysis<'a> {
    body: &'a MirFunction,
    tc: &'a TypeContext,
    contract: &'a MirBackendContract,
    table: &'a NativeReturnSources,
}

impl SourceAnalysis<'_> {
    // lean-ctx: primitive slots / one flat tuple field; upgrade only with
    // measured initialization and projection evidence for broader storage.
    fn storage_type(&self, ty: TypeId) -> bool {
        scalar_type(self.tc, self.contract, ty)
            && (primitive(self.tc, ty)
                || matches!(self.tc.try_ty(ty), Some(Ty::Tuple(fields))
                if fields.iter().all(|field| primitive(self.tc, *field))))
    }

    fn scalar_storage_path(&self, place: &Place) -> Option<TypeId> {
        let decl = self.body.local_decls.get(place.local.0)?;
        if !matches!(
            decl.source,
            LocalSource::UserBinding | LocalSource::Temporary
        ) || !self.storage_type(decl.ty)
        {
            return None;
        }
        let ty = match place.projection.as_slice() {
            [] if primitive(self.tc, decl.ty) => decl.ty,
            [Projection::Field {
                index,
                identity: None,
            }] => {
                let Ty::Tuple(fields) = self.tc.try_ty(decl.ty)? else {
                    return None;
                };
                *fields.get(*index)?
            }
            _ => return None,
        };
        (primitive(self.tc, ty) && place_type(self.body, self.tc, self.contract, place) == Some(ty))
            .then_some(ty)
    }

    fn scalar_reference<'a>(
        &self,
        state: &'a State,
        operand: &Operand,
    ) -> Option<&'a ScalarAddress> {
        let (Operand::Copy(place) | Operand::Move(place)) = operand else {
            return None;
        };
        if !place.projection.is_empty() {
            return None;
        }
        let address = state
            .fresh
            .as_ref()?
            .references
            .get(place.local.0)?
            .as_ref()?;
        if operand_type(self.body, self.tc, self.contract, operand) != Some(address.ty) {
            return None;
        }
        let Ty::Reference {
            mutable: false,
            inner,
        } = self.tc.try_ty(address.ty)?
        else {
            return None;
        };
        (!address.paths.is_empty()
            && address
                .paths
                .iter()
                .all(|path| self.scalar_storage_path(path) == Some(*inner)))
        .then_some(address)
    }

    fn scalar_value_initialized(&self, state: &State, operand: &Operand) -> bool {
        match operand {
            Operand::Constant(_) => operand_type(self.body, self.tc, self.contract, operand)
                .is_some_and(|ty| primitive(self.tc, ty)),
            Operand::Copy(place) | Operand::Move(place) => {
                let Some(fresh) = &state.fresh else {
                    return false;
                };
                let root = self.body.local_decls[place.local.0].ty;
                fresh.initialized[place.local.0] == Some(root)
                    && (place.projection.is_empty() || self.scalar_storage_path(place).is_some())
                    && operand_type(self.body, self.tc, self.contract, operand)
                        .is_some_and(|ty| self.storage_type(ty))
            }
        }
    }

    fn fresh_statement(&self, state: &mut State, destination: &Place, rvalue: &Rvalue) {
        if !destination.projection.is_empty() {
            if let Some(fresh) = &mut state.fresh {
                fresh.clear();
            }
            return;
        }
        let ty = place_type(self.body, self.tc, self.contract, destination);
        let eligible = matches!(
            self.body.local_decls[destination.local.0].source,
            LocalSource::UserBinding | LocalSource::Temporary
        );
        let initialized = ty
            .filter(|ty| eligible && self.storage_type(*ty))
            .filter(|ty| match rvalue {
                Rvalue::Use(operand) => {
                    operand_type(self.body, self.tc, self.contract, operand) == Some(*ty)
                        && self.scalar_value_initialized(state, operand)
                }
                Rvalue::Aggregate(AggregateKind::Tuple, operands) => {
                    let Some(Ty::Tuple(fields)) = self.tc.try_ty(*ty) else {
                        return false;
                    };
                    fields.len() == operands.len()
                        && fields.iter().zip(operands).all(|(field, operand)| {
                            operand_type(self.body, self.tc, self.contract, operand) == Some(*field)
                                && self.scalar_value_initialized(state, operand)
                        })
                }
                _ => false,
            });
        let reference = match rvalue {
            Rvalue::Use(operand) => self
                .scalar_reference(state, operand)
                .filter(|address| Some(address.ty) == ty)
                .cloned(),
            Rvalue::Ref(Mutability::Not, place) => {
                self.scalar_storage_path(place).and_then(|inner| {
                    let fresh = state.fresh.as_ref()?;
                    if fresh.initialized[place.local.0]
                        != Some(self.body.local_decls[place.local.0].ty)
                    {
                        return None;
                    }
                    let ty = ty?;
                    (self.tc.try_ty(ty)
                        == Some(&Ty::Reference {
                            mutable: false,
                            inner,
                        }))
                    .then(|| ScalarAddress {
                        ty,
                        paths: HashSet::from([place.clone()]),
                    })
                })
            }
            _ => None,
        };
        if let Some(fresh) = &mut state.fresh {
            match rvalue {
                Rvalue::Cast(_, _)
                | Rvalue::Ref(Mutability::Mut, _)
                | Rvalue::Closure(_)
                | Rvalue::Object(_, _)
                | Rvalue::Package(_, _) => fresh.clear(),
                Rvalue::Use(operand) | Rvalue::UnaryOp(_, operand) => fresh.move_operand(operand),
                Rvalue::BinaryOp(_, left, right) => {
                    fresh.move_operand(left);
                    fresh.move_operand(right);
                }
                Rvalue::Aggregate(_, operands) => {
                    for operand in operands {
                        fresh.move_operand(operand);
                    }
                }
                _ => {}
            }
            fresh.kill(destination.local);
            fresh.initialized[destination.local.0] = initialized;
            fresh.references[destination.local.0] = reference.filter(|address| {
                !address
                    .paths
                    .iter()
                    .any(|path| path.local == destination.local)
            });
        }
    }

    fn place(&self, state: &State, place: &Place) -> Option<Value> {
        let value = state.locals.as_ref()?.get(place.local.0)?.as_ref()?;
        let ty = place_type(self.body, self.tc, self.contract, place)?;
        if place.projection.is_empty() && value.ty == ty {
            return Some(value.clone());
        }
        if place.projection == [Projection::Deref]
            && value.stage == Stage::Slot
            && slice_element(self.tc, ty).is_some()
        {
            return Some(Value {
                stage: Stage::Slice,
                ty,
                sources: value.sources.clone(),
            });
        }
        None // Fields, raw dereferences, owner payloads and arrays are not slice loads.
    }

    fn operand(&self, state: &State, operand: &Operand) -> Option<Value> {
        let (Operand::Copy(place) | Operand::Move(place)) = operand else {
            return None;
        };
        if !place.projection.is_empty() && !matches!(operand, Operand::Copy(_)) {
            return None;
        }
        self.place(state, place)
    }

    fn call_value(
        &self,
        state: &State,
        callee: &Operand,
        args: &[Operand],
        destination: &Place,
    ) -> Option<Value> {
        let (key, ret) =
            checked_call(self.body, self.tc, self.contract, callee, args, destination)?;
        if let MirCallableKey::Intrinsic(intrinsic) = key {
            let first = self.operand(state, args.first()?)?;
            let stage = match intrinsic {
                MirIntrinsicId::ArrPtr
                    if args.len() == 1 && matches!(first.stage, Stage::Slice | Stage::View) =>
                {
                    let element = slice_element(self.tc, first.ty)?;
                    if self.tc.try_ty(ret) != Some(&Ty::Pointer(element)) {
                        return None;
                    }
                    Stage::Pointer
                }
                MirIntrinsicId::PtrOffset if args.len() == 2 && first.stage == Stage::Pointer => {
                    if ret != first.ty
                        || self.tc.try_ty(operand_type(
                            self.body,
                            self.tc,
                            self.contract,
                            &args[1],
                        )?) != Some(&Ty::I64)
                    {
                        return None;
                    }
                    Stage::Pointer
                }
                MirIntrinsicId::BorrowSlice if args.len() == 2 && first.stage == Stage::Pointer => {
                    let element = slice_element(self.tc, ret)?;
                    if self.tc.try_ty(first.ty) != Some(&Ty::Pointer(element))
                        || self.tc.try_ty(operand_type(
                            self.body,
                            self.tc,
                            self.contract,
                            &args[1],
                        )?) != Some(&Ty::I64)
                    {
                        return None;
                    }
                    Stage::View
                }
                _ => return None,
            };
            return Some(Value {
                stage,
                ty: ret,
                sources: first.sources,
            });
        }
        let flow = self.table.flows.get(key)?;
        if flow.value.ty != ret || self.contract.function_bodies.get(&flow.body) != Some(key) {
            return None;
        }
        let mut sources = HashSet::new();
        for source in &flow.value.sources {
            let argument = args.get(source.parameter().0.checked_sub(1)?)?;
            let incoming = match source {
                Source::Slice {
                    load, reference, ..
                } => {
                    let (Operand::Copy(place) | Operand::Move(place)) = argument else {
                        return None;
                    };
                    let mut loaded = place.clone();
                    loaded.projection.extend(load.iter().cloned());
                    let value = self.place(state, &loaded)?;
                    if !matches!(value.stage, Stage::Slice | Stage::View) || value.ty != *reference
                    {
                        return None;
                    }
                    value
                }
                Source::PointerInput { .. } => {
                    let value = self.operand(state, argument)?;
                    if value.stage != Stage::Pointer {
                        return None;
                    }
                    value
                }
            };
            sources.extend(incoming.sources);
        }
        Some(Value {
            stage: flow.value.stage,
            ty: ret,
            sources,
        })
    }
}

impl Analysis for SourceAnalysis<'_> {
    type Domain = State;

    fn bottom_state(&self, _: &MirFunction) -> State {
        State::default()
    }

    fn initial_state(&self, body: &MirFunction) -> State {
        let mut locals = vec![None; body.local_decls.len()];
        for (index, decl) in body
            .local_decls
            .iter()
            .enumerate()
            .skip(1)
            .take(body.arg_count)
        {
            if decl.source != LocalSource::Argument {
                continue;
            }
            let parameter = Local(index);
            let seed = if slice_element(self.tc, decl.ty).is_some() {
                Some((
                    Stage::Slice,
                    Source::Slice {
                        parameter,
                        load: Vec::new(),
                        reference: decl.ty,
                    },
                ))
            } else if let Some(Ty::Reference {
                mutable: false,
                inner,
            }) = self.tc.try_ty(decl.ty)
            {
                slice_element(self.tc, *inner).map(|_| {
                    (
                        Stage::Slot,
                        Source::Slice {
                            parameter,
                            load: vec![Projection::Deref],
                            reference: *inner,
                        },
                    )
                })
            } else if let Some(Ty::Pointer(element)) = self.tc.try_ty(decl.ty) {
                primitive(self.tc, *element)
                    .then_some((Stage::Pointer, Source::PointerInput { parameter }))
            } else {
                None
            };
            if let Some((stage, source)) = seed {
                locals[index] = Some(Value {
                    stage,
                    ty: decl.ty,
                    sources: HashSet::from([source]),
                });
            }
        }
        State {
            locals: Some(locals),
            fresh: Some(FreshStorage::new(body.local_decls.len())),
            blocked: false,
            unknown_calls: false,
        }
    }

    fn apply_statement(&self, state: &mut State, statement: &StatementData) {
        let StatementKind::Assign(destination, rvalue) = &statement.kind else {
            if let StatementKind::StorageDead(local) | StatementKind::StorageLive(local) =
                &statement.kind
            {
                if let Some(locals) = &mut state.locals {
                    locals[local.0] = None;
                }
                if let Some(fresh) = &mut state.fresh {
                    fresh.kill(*local);
                }
            }
            return;
        };
        self.fresh_statement(state, destination, rvalue);
        if !destination.projection.is_empty() {
            state.blocked = true;
            return;
        }
        let ty = place_type(self.body, self.tc, self.contract, destination);
        let value = match rvalue {
            Rvalue::Use(operand) => self
                .operand(state, operand)
                .filter(|value| Some(value.ty) == ty),
            Rvalue::Ref(Mutability::Not, place) if place.projection == [Projection::Deref] => {
                let pointer = Place {
                    local: place.local,
                    projection: Vec::new(),
                };
                self.place(state, &pointer).and_then(|value| {
                    let Ty::Pointer(element) = self.tc.try_ty(value.ty)? else {
                        return None;
                    };
                    let ty = ty?;
                    if value.stage != Stage::Pointer
                        || !primitive(self.tc, *element)
                        || self.tc.try_ty(ty)
                            != Some(&Ty::Reference {
                                mutable: false,
                                inner: *element,
                            })
                    {
                        return None;
                    }
                    Some(Value {
                        stage: Stage::View,
                        ty,
                        sources: value.sources,
                    })
                })
            }
            Rvalue::Cast(operand, target) => {
                if operand_type(self.body, self.tc, self.contract, operand).is_some_and(|ty| {
                    matches!(
                        self.tc.try_ty(ty),
                        Some(Ty::Reference { .. } | Ty::Pointer(_))
                    )
                }) || matches!(
                    self.tc.try_ty(*target),
                    Some(Ty::Reference { .. } | Ty::Pointer(_))
                ) {
                    state.blocked = true;
                }
                None // Even a same-pointee pointer cast discards the certificate.
            }
            Rvalue::Ref(Mutability::Mut, _)
            | Rvalue::Object(_, _)
            | Rvalue::Package(_, _)
            | Rvalue::Closure(_) => {
                state.blocked = true;
                None
            }
            _ => None,
        };
        if let Some(locals) = &mut state.locals {
            locals[destination.local.0] = value;
        }
    }

    fn apply_terminator(&self, state: &mut State, term: &Terminator) {
        match term {
            Terminator::Call {
                func,
                args,
                destination,
                ..
            } => {
                let value = self.call_value(state, func, args, destination);
                let ancillary =
                    checked_call(self.body, self.tc, self.contract, func, args, destination)
                        .is_some_and(|(key, ret)| match key {
                            MirCallableKey::Intrinsic(MirIntrinsicId::ArrayLen) => {
                                args.len() == 1
                                    && operand_type(self.body, self.tc, self.contract, &args[0])
                                        .is_some_and(|ty| slice_element(self.tc, ty).is_some())
                                    && self.tc.try_ty(ret) == Some(&Ty::I64)
                            }
                            MirCallableKey::Intrinsic(intrinsic)
                                if intrinsic.is_value_intrinsic()
                                    && !matches!(
                                        intrinsic,
                                        MirIntrinsicId::MakeArr
                                            | MirIntrinsicId::BorrowSlice
                                            | MirIntrinsicId::BorrowSliceMut
                                            | MirIntrinsicId::BorrowStr
                                            | MirIntrinsicId::AtomicU64Exchange
                                            | MirIntrinsicId::AtomicU64FetchAdd
                                            | MirIntrinsicId::AtomicU64FetchSub
                                    ) =>
                            {
                                primitive(self.tc, ret)
                                    && args.iter().all(|argument| {
                                        operand_type(self.body, self.tc, self.contract, argument)
                                            .is_some_and(|ty| primitive(self.tc, ty))
                                    })
                            }
                            key => self.table.scalar_returns.contains(key),
                        });
                // Checked scalar values and witnessed shared addresses of fresh
                // scalar storage cannot carry the slice backing witness. This
                // says nothing about effects or retention: unknown_calls stays
                // true for opaque callees, with no body/scalar-return evidence.
                let independent =
                    checked_signature(self.body, self.tc, self.contract, func, args, destination)
                        .is_some_and(|(key, ret)| {
                            !matches!(key, MirCallableKey::Intrinsic(_))
                                && scalar_type(self.tc, self.contract, ret)
                                && args.iter().all(|argument| {
                                    operand_type(self.body, self.tc, self.contract, argument)
                                        .is_some_and(|ty| scalar_type(self.tc, self.contract, ty))
                                        || self.scalar_reference(state, argument).is_some()
                                })
                        });
                if value.is_none() && !ancillary && !independent {
                    state.blocked = true;
                }
                if value.is_none() && !ancillary {
                    state.unknown_calls = true;
                }
                let initialized = place_type(self.body, self.tc, self.contract, destination)
                    .filter(|ty| {
                        destination.projection.is_empty()
                            && (ancillary || independent)
                            && self.storage_type(*ty)
                            && matches!(
                                self.body.local_decls[destination.local.0].source,
                                LocalSource::UserBinding | LocalSource::Temporary
                            )
                    });
                if let Some(fresh) = &mut state.fresh {
                    if !ancillary && !independent && value.is_none()
                        || !destination.projection.is_empty()
                    {
                        fresh.clear();
                    }
                    fresh.move_operand(func);
                    for argument in args {
                        fresh.move_operand(argument);
                    }
                    fresh.kill(destination.local);
                    fresh.initialized[destination.local.0] = initialized;
                }
                if destination.projection.is_empty() {
                    if let Some(locals) = &mut state.locals {
                        locals[destination.local.0] = value;
                    }
                } else {
                    state.blocked = true;
                }
            }
            Terminator::Drop { .. } | Terminator::DropWithOrigin { .. } => {
                state.blocked = true;
                if let Some(fresh) = &mut state.fresh {
                    fresh.clear();
                }
            }
            _ => {}
        }
    }
}

/// Observation only: shared across the selected producer probes in one control.
#[cfg(test)]
#[derive(Default)]
pub(super) struct SourceReceipt {
    functions: HashSet<MirFunctionId>,
    sites: HashSet<(MirFunctionId, usize, usize)>,
    expressions: HashSet<(MirFunctionId, Source)>,
    address_paths: HashSet<(MirFunctionId, Place)>,
    lines: usize,
    validated: bool,
    contract_failed: bool,
}

#[cfg(test)]
impl SourceReceipt {
    fn budget(&self) {
        assert!(self.functions.len() <= 4 && self.sites.len() <= 32
            && self.expressions.len() <= 16 && self.address_paths.len() <= 16 && self.lines <= 128,
            "SLICE_SOURCE_FIRST_LOSS HARNESS OVERFLOW functions={}/4 sites={}/32 sourceExpressions={}/16 addressPaths={}/16 lines={}/128; no truncation",
            self.functions.len(), self.sites.len(), self.expressions.len(), self.address_paths.len(), self.lines);
    }

    fn emit(&mut self, message: std::fmt::Arguments<'_>) {
        self.lines += 1;
        self.budget();
        eprintln!("SLICE_SOURCE_FIRST_LOSS {message}");
    }

    fn observe(&mut self, id: &MirFunctionId, state: &State) {
        for value in state.locals.iter().flatten().flatten() {
            for source in &value.sources {
                self.expressions.insert((id.clone(), source.clone()));
            }
        }
        if let Some(fresh) = &state.fresh {
            for address in fresh.references.iter().flatten() {
                for path in &address.paths {
                    self.address_paths.insert((id.clone(), path.clone()));
                }
            }
        }
        self.budget();
    }

    fn site(&mut self, id: &MirFunctionId, block: usize, index: usize) {
        self.sites.insert((id.clone(), block, index));
        self.budget();
    }

    fn value(state: &State, local: Local) -> Option<&Value> {
        state.locals.as_ref()?.get(local.0)?.as_ref()
    }

    fn fresh(state: &State, local: Local) -> (Option<TypeId>, Option<&ScalarAddress>) {
        state
            .fresh
            .as_ref()
            .map(|fresh| {
                (
                    fresh.initialized[local.0],
                    fresh.references[local.0].as_ref(),
                )
            })
            .unwrap_or((None, None))
    }
}

#[cfg(test)]
impl NativeReturnSources {
    /// Probe the actual selected caller, then replay only its canonical producer
    /// and source-helper/first-blocking dependencies using the production analysis.
    pub(super) fn trace_call(
        &self,
        program: &MirProgram,
        caller: &MirFunction,
        block: usize,
        receipt: &mut SourceReceipt,
    ) {
        let tc = &program.type_context;
        let contract = &program.backend_contract;
        if !receipt.validated {
            let errors = crate::mir::backend_contract::validate_backend_contract_against_mir(
                contract,
                program.functions.values(),
            );
            receipt.emit(format_args!("globalContractErrors={} caps=4functions/32sites/16sourceExpressions/128lines tableIsFinal=true execution=NONE", errors.len()));
            assert!(
                errors.len() <= 16,
                "SLICE_SOURCE_FIRST_LOSS HARNESS OVERFLOW contract errors={}/16; no truncation",
                errors.len()
            );
            for error in &errors {
                receipt.emit(format_args!("actualContractError={error:?}"));
            }
            receipt.validated = true;
            receipt.contract_failed = !errors.is_empty();
            if !errors.is_empty() {
                receipt.emit(format_args!(
                    "for_program admission=GLOBAL_CONTRACT_DEFAULT; no source proof inferred"
                ));
                return;
            }
        }
        if receipt.contract_failed {
            return;
        }
        let Some(Terminator::Call {
            func,
            args,
            destination,
            ..
        }) = &caller.basic_blocks[block].terminator
        else {
            return;
        };
        let Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) = func else {
            return;
        };
        receipt.site(
            &caller.id,
            block,
            caller.basic_blocks[block].statements.len(),
        );
        let header = contract.callable(key);
        let params = args
            .iter()
            .map(|arg| operand_type(caller, tc, contract, arg))
            .collect::<Vec<_>>();
        let ret = place_type(caller, tc, contract, destination);
        receipt.emit(format_args!("caller={:?} block={block} key={key:?} sourceDef={:?} header={header:?} actualParams={params:?} actualRet={ret:?} checkedCall={:?} tableEntry={:?} scalarEntry={} borrowedViewAdmission={:?}",
            caller.id, header.and_then(|header| header.source_def_id),
            checked_call(caller, tc, contract, func, args, destination), self.flows.get(key),
            self.scalar_returns.contains(key), self.borrowed_view_call(caller, tc, contract, func, args, destination)));
        if let Some(flow) = self.flows.get(key) {
            receipt.emit(format_args!("callerAdmissionPredicates bodyMapped={} stageView={} resultTypeEqual={} allSourcesSlice={} actualArgumentPlaces={args:?}; PointerInput is conditional helper evidence, NOT an admitted borrowed source",
                contract.function_bodies.get(&flow.body) == Some(key), flow.value.stage == Stage::View,
                Some(flow.value.ty) == ret, flow.value.sources.iter().all(|source| matches!(source, Source::Slice { .. }))));
        } else {
            receipt.emit(format_args!("callerFirstLoss=TABLE_ENTRY_ABSENT; inspect actual producer rather than guess from header"));
        }
        let Some(MirCallableKind::LocalBody { function_id }) = header.map(|header| &header.kind)
        else {
            return;
        };
        let mut pending = std::collections::VecDeque::from([function_id.clone()]);
        while let Some(id) = pending.pop_front() {
            if !receipt.functions.insert(id.clone()) {
                continue;
            }
            receipt.budget();
            let Some(body) = program.functions.get(&id) else {
                receipt.emit(format_args!(
                    "body={id:?} ABSENT; no replay/empty-summary assumption"
                ));
                continue;
            };
            let body_key = contract.function_bodies.get(&id);
            let body_header = body_key.and_then(|key| contract.callable(key));
            assert!(
                body.arg_count <= 8 && body.closure_captures.len() <= 16,
                "SLICE_SOURCE_FIRST_LOSS HARNESS OVERFLOW args8/captures16"
            );
            let invalid = body
                .local_decls
                .iter()
                .enumerate()
                .filter_map(|(index, decl)| {
                    (!tc.type_id_tree_is_valid(decl.ty)).then_some((Local(index), decl.ty))
                })
                .collect::<Vec<_>>();
            assert!(
                invalid.len() <= 16,
                "SLICE_SOURCE_FIRST_LOSS HARNESS OVERFLOW invalid types16"
            );
            receipt.emit(format_args!("body={id:?} actualId={:?} key={body_key:?} header={body_header:?} closureCaptures={:?} emptyBody={} invalidLocalTypes={invalid:?} tableEntry={:?} scalarEntry={}",
                body.id, body.closure_captures, body.basic_blocks.is_empty(), body_key.and_then(|key| self.flows.get(key)),
                body_key.is_some_and(|key| self.scalar_returns.contains(key))));
            receipt.emit(format_args!("body={id:?} for_programEntryPredicates bodyIdEqual={} headerKeyEqual={} callableBodyEqual={} localBodyKindEqual={} capturesEmpty={} validLocals={}; header alone does not certify return sources",
                body.id == id, body_header.is_some_and(|header| Some(&header.key) == body_key),
                body_key.is_some_and(|key| MirCallable::Resolved(key.clone()).function_id().as_ref() == Some(&id)),
                body_header.is_some_and(|header| matches!(&header.kind, MirCallableKind::LocalBody { function_id } if function_id == &id)),
                body.closure_captures.is_empty(), invalid.is_empty()));
            if body.basic_blocks.is_empty() {
                continue;
            }
            let analysis = SourceAnalysis {
                body,
                tc,
                contract,
                table: self,
            };
            let initial = analysis.initial_state(body);
            receipt.observe(&id, &initial);
            for (index, decl) in body
                .local_decls
                .iter()
                .enumerate()
                .skip(1)
                .take(body.arg_count)
            {
                receipt.emit(format_args!(
                    "body={id:?} argument={:?} declaration={decl:?} semantic={:?} actualSeed={:?}",
                    Local(index),
                    tc.try_ty(decl.ty),
                    SourceReceipt::value(&initial, Local(index))
                ));
            }
            // Even an excluded body is replayed for diagnosis only; this never
            // installs an entry in the immutable final production table.
            let results = run_fixpoint(&analysis, body);
            for (block, data) in body.basic_blocks.iter().enumerate() {
                if !results.reachable[block] {
                    continue;
                }
                let mut state = results.entry_sets[block].clone();
                receipt.observe(&id, &state);
                for (index, statement) in data.statements.iter().enumerate() {
                    let before = state.clone();
                    analysis.apply_statement(&mut state, statement);
                    receipt.observe(&id, &state);
                    let (local, input) = match &statement.kind {
                        StatementKind::Assign(
                            place,
                            Rvalue::Use(operand) | Rvalue::Cast(operand, _),
                        ) => (place.local, analysis.operand(&before, operand)),
                        StatementKind::Assign(place, Rvalue::Ref(_, target)) => (
                            place.local,
                            SourceReceipt::value(&before, target.local).cloned(),
                        ),
                        StatementKind::Assign(place, _) => (place.local, None),
                        StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                            (*local, None)
                        }
                        StatementKind::Assert(_) => continue,
                    };
                    if input.is_some()
                        || SourceReceipt::value(&before, local)
                            != SourceReceipt::value(&state, local)
                        || before.fresh != state.fresh
                        || before.blocked != state.blocked
                        || before.unknown_calls != state.unknown_calls
                    {
                        match &statement.kind {
                            StatementKind::Assign(_, Rvalue::Aggregate(_, operands)) => assert!(
                                operands.len() <= 16,
                                "SLICE_SOURCE_FIRST_LOSS HARNESS OVERFLOW aggregate operands16"
                            ),
                            StatementKind::Assign(_, Rvalue::Closure(closure)) => assert!(
                                closure.captures.len() <= 16,
                                "SLICE_SOURCE_FIRST_LOSS HARNESS OVERFLOW closure captures16"
                            ),
                            _ => {}
                        }
                        receipt.site(&id, block, index);
                        receipt.emit(format_args!("body={id:?} site=({block},{index}) step={statement:?} checkedDestination={:?} inputCertificate={input:?} before={:?} after={:?} freshBefore={:?} freshAfter={:?} blocked={}->{} unknownCalls={}->{} FIRST_BLOCKING={}",
                            match &statement.kind { StatementKind::Assign(place, _) => place_type(body, tc, contract, place), _ => None },
                            SourceReceipt::value(&before, local), SourceReceipt::value(&state, local),
                            SourceReceipt::fresh(&before, local), SourceReceipt::fresh(&state, local), before.blocked, state.blocked,
                            before.unknown_calls, state.unknown_calls, !before.blocked && state.blocked));
                    }
                }
                let Some(term) = &data.terminator else {
                    continue;
                };
                let before = state.clone();
                let call_value = if let Terminator::Call {
                    func,
                    args,
                    destination,
                    ..
                } = term
                {
                    analysis.call_value(&before, func, args, destination)
                } else {
                    None
                };
                analysis.apply_terminator(&mut state, term);
                receipt.observe(&id, &state);
                if let Terminator::Call {
                    func,
                    args,
                    destination,
                    ..
                } = term
                {
                    let Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) = func
                    else {
                        receipt.site(&id, block, data.statements.len());
                        receipt.emit(format_args!("body={id:?} block={block} unqualifiedCallee={func:?} blocked={}->{} unknownCalls={}->{}", before.blocked, state.blocked, before.unknown_calls, state.unknown_calls));
                        continue;
                    };
                    assert!(
                        args.len() <= 8,
                        "SLICE_SOURCE_FIRST_LOSS HARNESS OVERFLOW call args8"
                    );
                    let inputs = args
                        .iter()
                        .map(|arg| analysis.operand(&before, arg))
                        .collect::<Vec<_>>();
                    let native_header = contract.callable(key);
                    let params = args
                        .iter()
                        .map(|arg| operand_type(body, tc, contract, arg))
                        .collect::<Vec<_>>();
                    let ret = place_type(body, tc, contract, destination);
                    let first_blocking = !before.blocked && state.blocked;
                    let first_unknown = !before.unknown_calls && state.unknown_calls;
                    // Observe source/flag-relevant calls only, never all stdlib IR.
                    if inputs.iter().any(Option::is_some)
                        || call_value.is_some()
                        || first_blocking
                        || before.fresh != state.fresh
                        || before.unknown_calls != state.unknown_calls
                    {
                        receipt.site(&id, block, data.statements.len());
                        receipt.emit(format_args!("body={id:?} block={block} actualCall={term:?} sourceDef={:?} header={native_header:?} actualParams={params:?} paramSemantics={:?} paramTracksOrigin={:?} actualRet={ret:?} retSemantic={:?} retTracksOrigin={:?} checkedCall={:?} inputCertificates={inputs:?} sharedAddressInputs={:?} callValue={call_value:?} tableEntry={:?} scalarEntry={} beforeDestination={:?} afterDestination={:?} freshDestinationBefore={:?} freshDestinationAfter={:?} blocked={}->{} unknownCalls={}->{} FIRST_BLOCKING={first_blocking}",
                            native_header.and_then(|header| header.source_def_id), params.iter().map(|ty| ty.and_then(|ty| tc.try_ty(ty))).collect::<Vec<_>>(),
                            params.iter().map(|ty| ty.map(|ty| super::type_id_tracks_origin(TypeView::new(tc), contract, ty))).collect::<Vec<_>>(),
                            ret.and_then(|ty| tc.try_ty(ty)), ret.map(|ty| super::type_id_tracks_origin(TypeView::new(tc), contract, ty)),
                            checked_call(body, tc, contract, func, args, destination),
                            args.iter().map(|argument| analysis.scalar_reference(&before, argument)).collect::<Vec<_>>(),
                            self.flows.get(key), self.scalar_returns.contains(key),
                            SourceReceipt::value(&before, destination.local), SourceReceipt::value(&state, destination.local),
                            SourceReceipt::fresh(&before, destination.local), SourceReceipt::fresh(&state, destination.local),
                            before.blocked, state.blocked, before.unknown_calls, state.unknown_calls));
                    }
                    if first_blocking
                        || first_unknown && scalar_type(tc, contract, body.ret_type)
                        || inputs.iter().any(Option::is_some)
                            && ret.is_some_and(|ty| {
                                super::type_id_tracks_origin(TypeView::new(tc), contract, ty)
                            })
                    {
                        if let Some(MirCallableKind::LocalBody { function_id }) =
                            native_header.map(|header| &header.kind)
                        {
                            pending.push_back(function_id.clone());
                        }
                    }
                } else if !before.blocked && state.blocked {
                    receipt.site(&id, block, data.statements.len());
                    receipt.emit(format_args!("body={id:?} block={block} FIRST_BLOCKING=true actualTerm={term:?} blocked={}->{} unknownCalls={}->{}; whole-body effect boundary, not a missing source intrinsic",
                        before.blocked, state.blocked, before.unknown_calls, state.unknown_calls));
                }
                if matches!(
                    term,
                    Terminator::Return | Terminator::ReturnWithOrigin { .. }
                ) {
                    receipt.site(&id, block, data.statements.len());
                    let exit = &results.exit_sets[block];
                    receipt.emit(format_args!("body={id:?} reachableReturn={block} actualTerm={term:?} actualReturn={:?} expectedRet={:?}/{:?} retTypeEqual={} blocked={} unknownCalls={} replayEqualsFixpoint={} actualQualifiedTableEntry={:?}; blocked/uncertified alternative forbids whole-return admission",
                        SourceReceipt::value(exit, Local(0)), body.ret_type, tc.try_ty(body.ret_type),
                        SourceReceipt::value(exit, Local(0)).is_some_and(|value| value.ty == body.ret_type), exit.blocked, exit.unknown_calls,
                        &state == exit, body_key.and_then(|key| self.flows.get(key))));
                }
            }
        }
        receipt.emit(format_args!("probeComplete caller={:?} block={block} counts={}/{}/{}/{} caps=4/32/16/128 addressPaths={}/16; observation only, no source/ABI/loan repair", caller.id,
            receipt.functions.len(), receipt.sites.len(), receipt.expressions.len(), receipt.lines + 1, receipt.address_paths.len()));
    }
}
