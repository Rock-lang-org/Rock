use std::collections::{HashMap, HashSet};

use crate::mir::dataflow::{Analysis, Lattice};
use crate::mir::{
    Constant, Local, LocalSource, MirBackendContract, MirCallable, MirCallableKey, MirFunction,
    MirFunctionId, MirIntrinsicId, Operand, Place, Projection, ReferenceOrigin, Rvalue,
    StatementData, StatementKind, Terminator,
};
use crate::type_context::{Ty, TypeView};

use super::*;

pub(super) type OriginSet = HashSet<ReferenceOrigin>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ReferenceOriginState {
    pub(super) origins: HashMap<Place, OriginSet>,
}

impl Lattice for ReferenceOriginState {
    fn join(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (place, origins) in &other.origins {
            let entry = self.origins.entry(place.clone()).or_default();
            for origin in origins {
                if entry.insert(*origin) {
                    changed = true;
                }
            }
        }
        changed
    }
}

pub(super) struct ReferenceEscapeAnalysis<'a> {
    pub(super) func: &'a MirFunction,
    pub(super) type_context: &'a crate::type_context::TypeContext,
    pub(super) backend_contract: &'a MirBackendContract,
    return_summaries: &'a HashMap<MirFunctionId, OriginSet>,
    missing_summary_mode: MissingSummaryMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MissingSummaryMode {
    Defer,
    Conservative,
}

impl<'a> ReferenceEscapeAnalysis<'a> {
    pub(super) fn new(
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
        }
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

    pub(super) fn local_type_is_reference(&self, local: Local) -> bool {
        self.local_decl(local).is_some_and(|decl| {
            matches!(
                TypeView::new(self.type_context).ty(decl.ty),
                Ty::Reference { .. }
            )
        })
    }

    pub(super) fn local_type_is_pointer(&self, local: Local) -> bool {
        self.local_decl(local).is_some_and(|decl| {
            matches!(TypeView::new(self.type_context).ty(decl.ty), Ty::Pointer(_))
        })
    }

    pub(super) fn place_type_is_pointer(&self, place: &Place) -> bool {
        self.backend_contract
            .place_type_id(self.type_context, self.func, place)
            .is_some_and(|ty| type_id_is_pointer(TypeView::new(self.type_context), ty))
    }

    pub(super) fn raw_pointer_source(&self, place: &Place) -> Option<Place> {
        let mut source = Place {
            local: place.local,
            projection: Vec::new(),
        };
        for projection in &place.projection {
            if matches!(projection, Projection::Deref) && self.place_type_is_pointer(&source) {
                return Some(source);
            }
            source.projection.push(projection.clone());
        }
        None
    }

    pub(super) fn call_return_argument_indices(
        &self,
        callee: &Operand,
        args: &[Operand],
    ) -> Vec<usize> {
        let summary = match callee {
            Operand::Constant(Constant::Callable(callable)) => callable
                .function_id()
                .and_then(|id| self.return_summaries.get(&id)),
            _ => None,
        };
        let Some(summary) = summary else {
            return (0..args.len()).collect();
        };
        let mut indices = summary
            .iter()
            .filter_map(|origin| match origin {
                ReferenceOrigin::Param(local) => local.0.checked_sub(1),
                _ => None,
            })
            .collect::<Vec<_>>();
        indices.sort_unstable();
        indices.dedup();
        indices
    }

    fn local_is_argument(&self, local: Local) -> bool {
        self.local_decl(local)
            .is_some_and(|decl| decl.source == LocalSource::Argument)
    }

    pub(super) fn local_type_tracks_origin(&self, local: Local) -> bool {
        self.local_decl(local).is_some_and(|decl| {
            type_id_tracks_origin(
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
                if self.local_type_is_reference(local)
                    || self.local_type_is_pointer(local)
                    || self.local_type_tracks_origin(local)
                {
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

    pub(super) fn origins_for_place(
        &self,
        state: &ReferenceOriginState,
        place: &Place,
    ) -> OriginSet {
        let normalized = Self::normalized_place(place.clone());
        if let Some(origins) = state.origins.get(&normalized) {
            return origins.clone();
        }

        let child_origins = self.child_origins_for_place(state, &normalized);
        if !child_origins.is_empty() {
            return child_origins;
        }

        if matches!(normalized.projection.first(), Some(Projection::Deref)) {
            let base = Place {
                local: normalized.local,
                projection: Vec::new(),
            };
            if let Some(origins) = state.origins.get(&base) {
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
        if let Some(source) = self.raw_pointer_source(place) {
            return self.origins_for_place(state, &source);
        }
        let normalized = Self::normalized_place(place.clone());
        if self.local_decl(place.local).is_some_and(|decl| {
            matches!(
                TypeView::new(self.type_context).ty(decl.ty),
                Ty::Slice(_) | Ty::Str
            )
        }) {
            let origins = self.origins_for_place(state, place);
            if !origins.is_empty() {
                return origins;
            }
        }
        if normalized.projection.is_empty()
            && !self.local_type_is_reference(normalized.local)
            && !self.local_type_is_pointer(normalized.local)
        {
            return self
                .local_origin(normalized.local)
                .map(|origin| HashSet::from([origin]))
                .unwrap_or_default();
        }

        let origins = self.origins_for_place(state, place);
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

    fn origins_for_operand(&self, state: &ReferenceOriginState, operand: &Operand) -> OriginSet {
        match operand {
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

    pub(super) fn temporary_origins_for_assignment(
        &self,
        state: &ReferenceOriginState,
        dest: &Place,
        rvalue: &Rvalue,
    ) -> HashSet<Local> {
        if !self.destination_can_outlive_statement(dest) {
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
            Rvalue::Use(operand) | Rvalue::Cast(operand, _) => {
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

    pub(super) fn temporary_origins_for_call(
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

    pub(super) fn place_is_prefix(prefix: &Place, place: &Place) -> bool {
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

        let Operand::Constant(Constant::Callable(callable)) = func else {
            return self.opaque_call_return_origins(state, args);
        };

        match callable {
            MirCallable::Resolved(MirCallableKey::Intrinsic(MirIntrinsicId::ArrPtr)) => {
                return match args.first() {
                    Some(Operand::Copy(place) | Operand::Move(place)) => {
                        self.origins_for_borrowed_place(state, place)
                    }
                    _ => args
                        .first()
                        .map(|arg| self.origins_for_operand(state, arg))
                        .unwrap_or_default(),
                };
            }
            MirCallable::Resolved(MirCallableKey::Intrinsic(intrinsic))
                if matches!(
                    intrinsic,
                    MirIntrinsicId::PtrFromParts | MirIntrinsicId::PtrOffset
                ) =>
            {
                return args
                    .first()
                    .map(|arg| self.origins_for_operand(state, arg))
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
        let Some(summary) = self.return_summaries.get(&function_id) else {
            return match self.missing_summary_mode {
                MissingSummaryMode::Defer => OriginSet::new(),
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
            if decl.source == LocalSource::Argument && self.local_type_is_reference(local) {
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

        match rvalue {
            Rvalue::Ref(_, place) => {
                let origins = self.origins_for_borrowed_place(state, place);
                self.assign_origins(state, dest, origins);
            }
            Rvalue::Use(operand) | Rvalue::Cast(operand, _) => {
                let origins = self.origins_for_operand(state, operand);
                self.assign_origins(state, dest, origins);
            }
            Rvalue::Aggregate(_, operands) => {
                self.assign_origins(state, dest, OriginSet::new());
                for (index, operand) in operands.iter().enumerate() {
                    let origins = self.origins_for_operand(state, operand);
                    if !origins.is_empty() {
                        let field_place = Self::field_place(dest, index);
                        state.origins.insert(field_place, origins);
                    }
                }
            }
            Rvalue::Closure(_)
            | Rvalue::BinaryOp(_, _, _)
            | Rvalue::UnaryOp(_, _)
            | Rvalue::Discriminant(_) => {
                self.assign_origins(state, dest, OriginSet::new());
            }
        }
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
            self.assign_origins(state, destination, origins);
        }
    }
}
