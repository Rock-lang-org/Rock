pub mod accesses;
pub mod borrows;
pub mod closures;
pub mod conflicts;
pub mod diagnostics;
pub mod liveness;
pub mod location;
pub mod paths;
pub mod provenance;

use std::collections::{HashMap, HashSet};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::ids::{AssocTypeId, DefId, TypeId};
use crate::lexer::Span;
use crate::mir::borrowck::accesses::AccessKind;
use crate::mir::borrowck::borrows::collect_function_borrows_with_contract;
use crate::mir::borrowck::closures::CaptureKind;
use crate::mir::borrowck::diagnostics::borrow_error;
use crate::mir::borrowck::liveness::{
    apply_active_loan_statement_transfer, compute_active_loan_entries_with_contract,
    compute_reference_liveness_with_contract, LoanLocationIndex,
};
use crate::mir::dataflow::analyses::{
    InitError, InitMap, InitializationAnalysis, LoanAnalysis, LoanKind, LoanState, LoanTable,
};
use crate::mir::dataflow::{run_fixpoint, Analysis, Lattice};
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

type OriginSet = HashSet<ReferenceOrigin>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ReferenceOriginState {
    origins: HashMap<Place, OriginSet>,
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

struct ReferenceEscapeAnalysis<'a> {
    func: &'a MirFunction,
    type_context: &'a crate::type_context::TypeContext,
    backend_contract: &'a MirBackendContract,
    return_summaries: &'a HashMap<MirFunctionId, OriginSet>,
    missing_summary_mode: MissingSummaryMode,
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
                    || self.local_type_contains_reference(local)
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

    fn origins_for_place(&self, state: &ReferenceOriginState, place: &Place) -> OriginSet {
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
        let normalized = Self::normalized_place(place.clone());
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

    fn temporary_origins_for_assignment(
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

        let Operand::Constant(Constant::Callable(callable)) = func else {
            return self.opaque_call_return_origins(state, args);
        };

        match callable {
            MirCallable::Resolved(MirCallableKey::Intrinsic(intrinsic))
                if matches!(
                    intrinsic,
                    MirIntrinsicId::BorrowSlice | MirIntrinsicId::BorrowSliceMut
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
            .normalize_type(self.type_context, &self.type_context.type_for(local.ty));
        let mut reference_owned_path = false;

        for projection in &place.projection {
            current_ty = self
                .backend_contract
                .normalize_type(self.type_context, &current_ty);
            match projection {
                Projection::Deref => match &current_ty {
                    Type::Reference { .. } => reference_owned_path = true,
                    Type::Pointer(_) => reference_owned_path = false,
                    _ => {}
                },
                Projection::Field { .. }
                    if validate_move_only_rules && self.has_direct_drop(&current_ty) =>
                {
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
                .normalize_type(self.type_context, &next_ty);
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
        let ty = self.backend_contract.normalize_type(self.type_context, ty);
        self.type_context
            .id_for_type(&ty)
            .is_some_and(|id| self.direct_drop_types.contains(&id))
    }

    fn type_needs_cleanup(&mut self, ty: &Type) -> bool {
        let ty = self.backend_contract.normalize_type(self.type_context, ty);
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
                Type::Tuple(elems) => elems.iter().any(|elem| self.type_needs_cleanup(elem)),
                Type::Function { .. } => true,
                _ => false,
            }
        };

        self.cleanup_cache.insert(ty, needs_cleanup);
        needs_cleanup
    }

    fn project_type(&self, ty: &Type, projection: &Projection) -> Option<Type> {
        let ty = self.backend_contract.normalize_type(self.type_context, ty);
        let projected = match projection {
            Projection::Deref => match &ty {
                Type::Reference { inner, .. } | Type::Pointer(inner) => {
                    Some(inner.as_ref().clone())
                }
                _ => None,
            },
            Projection::Field { index, .. } => match &ty {
                Type::Tuple(elems) => elems.get(*index).cloned(),
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
                .normalize_type(self.type_context, &projected),
        )
    }

    fn type_display_name(&self, ty: &Type) -> String {
        let ty = self.backend_contract.normalize_type(self.type_context, ty);
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

fn type_id_tracks_origin(
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
        Ty::Reference { .. } => true,
        Ty::Pointer(_) if include_pointers => true,
        Ty::Tuple(elems) => elems.iter().any(|elem| {
            type_id_tracks_origin_inner(view, backend_contract, *elem, include_pointers, seen)
        }),
        Ty::Array { inner, .. } | Ty::Slice(inner) => {
            type_id_tracks_origin_inner(view, backend_contract, *inner, include_pointers, seen)
        }
        Ty::Function {
            params,
            ret,
            captures,
            ..
        } => {
            params.iter().any(|param| {
                type_id_tracks_origin_inner(view, backend_contract, *param, include_pointers, seen)
            }) || type_id_tracks_origin_inner(view, backend_contract, *ret, include_pointers, seen)
                || captures.iter().any(|capture| {
                    type_id_tracks_origin_inner(
                        view,
                        backend_contract,
                        capture.ty,
                        include_pointers,
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
        Ty::Generic(_)
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
        Type::Reference { .. } => true,
        Type::Pointer(_) if include_pointers => true,
        Type::Tuple(elems) => elems.iter().any(|elem| {
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
            params.iter().any(|param| {
                type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    param,
                    include_pointers,
                    seen,
                )
            }) || type_id_tracks_origin_for_structural_type(
                view,
                backend_contract,
                ret,
                include_pointers,
                seen,
            ) || captures.iter().any(|capture| {
                type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    &capture.ty,
                    include_pointers,
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
        Type::Generic(_) | Type::TypeVar(_) | Type::Constructor { .. } | Type::BoundVar { .. } => {
            true
        }
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
        let mut diagnostics = Diagnostics::default();
        let return_summaries = Self::reference_return_summaries(program);

        for (_, func) in program.functions() {
            if let Err(func_diagnostics) = Self::check_function_with_contract(
                func,
                &program.type_context,
                &program.backend_contract,
                &return_summaries,
            ) {
                for diag in func_diagnostics.0 {
                    diagnostics.push(diag);
                }
            }
        }

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
        let mut diagnostics = Self::validate_moves_and_initialization(
            func,
            type_context,
            backend_contract,
            return_summaries,
        );
        Self::validate_reference_escapes(
            func,
            type_context,
            backend_contract,
            return_summaries,
            &mut diagnostics,
        );
        Self::validate_drop_obligations(func, backend_contract, &mut diagnostics);

        if diagnostics.0.is_empty() {
            Ok(())
        } else {
            Err(diagnostics)
        }
    }

    fn reference_return_summaries(program: &MirProgram) -> HashMap<MirFunctionId, OriginSet> {
        let mut summaries = HashMap::new();

        loop {
            let previous = summaries.clone();
            let mut next = HashMap::new();
            for (id, func) in program.functions() {
                if !type_id_tracks_origin(
                    TypeView::new(&program.type_context),
                    &program.backend_contract,
                    func.ret_type,
                ) {
                    continue;
                }

                let origins = Self::reference_return_origins(
                    func,
                    &program.type_context,
                    &program.backend_contract,
                    &previous,
                );
                if !origins.is_empty() {
                    next.insert(id.clone(), origins);
                }
            }

            if next == previous {
                return next;
            }
            summaries = next;
        }
    }

    fn reference_return_origins(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
    ) -> OriginSet {
        let analysis = ReferenceEscapeAnalysis::new(
            func,
            type_context,
            backend_contract,
            return_summaries,
            MissingSummaryMode::Defer,
        );
        let results = run_fixpoint(&analysis, func);
        let return_place = Place {
            local: Local(0),
            projection: Vec::new(),
        };
        let mut origins = OriginSet::new();

        for (block_idx, block) in func.basic_blocks.iter().enumerate() {
            if !matches!(
                block.terminator,
                Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
            ) {
                continue;
            }
            for (place, block_origins) in &results.exit_sets[block_idx].origins {
                if ReferenceEscapeAnalysis::place_is_prefix(&return_place, place) {
                    origins.extend(block_origins.iter().copied());
                }
            }
        }

        origins
    }

    fn validate_moves_and_initialization(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
    ) -> Diagnostics {
        let mut diagnostics = Diagnostics::default();
        let type_view = TypeView::new(type_context);
        let mut move_validation = MoveValidationContext::new(type_context, backend_contract);

        let init_analysis = InitializationAnalysis::new(func, type_context);
        let init_results = run_fixpoint(&init_analysis, func);

        let borrows = collect_function_borrows_with_contract(
            func,
            type_context,
            backend_contract,
            return_summaries,
        );
        let place_paths = crate::mir::borrowck::paths::PlacePathTable::for_function(func);
        let loan_table = LoanTable::from_borrows_with_paths(&borrows, place_paths);
        let loans_by_location = LoanLocationIndex::for_function(&loan_table, func);
        let reference_liveness =
            compute_reference_liveness_with_contract(func, type_context, backend_contract);
        let active_loan_entries = compute_active_loan_entries_with_contract(
            func,
            type_context,
            backend_contract,
            &loan_table,
            &loans_by_location,
            &reference_liveness,
        );

        for (block_idx, block) in func.basic_blocks.iter().enumerate() {
            let mut state = init_results.entry_sets[block_idx].clone();
            let mut active_loans = active_loan_entries[block_idx].clone();

            for (stmt_idx, stmt) in block.statements.iter().enumerate() {
                let mut live_loans = active_loans.clone();
                if let Some(live_locals) = reference_liveness
                    .before_statement_sets
                    .get(block_idx)
                    .and_then(|stmt_sets| stmt_sets.get(stmt_idx))
                {
                    live_loans.retain_owners(|owner| live_locals.contains(owner));
                }

                Self::check_statement_loans(
                    stmt,
                    type_context,
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

                if let StatementKind::Assign(dest, Rvalue::Use(Operand::Move(src))) = &stmt.kind {
                    active_loans.transfer_owner(src.local, dest.local);
                }

                if let StatementKind::Assign(dest, Rvalue::Use(Operand::Copy(src))) = &stmt.kind {
                    active_loans.copy_owner(src.local, dest.local);
                }

                if let StatementKind::Assign(dest, Rvalue::Cast(op, target)) = &stmt.kind {
                    if let Operand::Copy(place) | Operand::Move(place) = op {
                        if func
                            .local_decls
                            .get(place.local.0)
                            .is_some_and(|decl| type_id_is_mut_reference(type_view, decl.ty))
                            && type_id_is_pointer(type_view, *target)
                        {
                            active_loans.transfer_owner(place.local, dest.local);
                        }
                    }
                }

                if matches!(stmt.kind, StatementKind::Assign(_, Rvalue::Aggregate(_, _))) {
                    apply_active_loan_statement_transfer(
                        &mut active_loans,
                        func,
                        type_context,
                        backend_contract,
                        stmt,
                    );
                }

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
                Self::check_terminator(
                    term,
                    &init_analysis,
                    &state,
                    &loan_table,
                    &active_loans,
                    func,
                    &mut diagnostics,
                );
            }
        }

        diagnostics
    }

    fn validate_reference_escapes(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        diagnostics: &mut Diagnostics,
    ) {
        if !type_id_contains_reference(TypeView::new(type_context), backend_contract, func.ret_type)
            && !func.local_decls.iter().any(|decl| {
                type_id_tracks_origin(TypeView::new(type_context), backend_contract, decl.ty)
            })
        {
            return;
        }

        let analysis = ReferenceEscapeAnalysis::new(
            func,
            type_context,
            backend_contract,
            return_summaries,
            MissingSummaryMode::Conservative,
        );
        let results = run_fixpoint(&analysis, func);

        Self::validate_temporary_reference_storage(func, &analysis, &results, diagnostics);

        if !type_id_contains_reference(TypeView::new(type_context), backend_contract, func.ret_type)
        {
            return;
        }

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

                for origin in origins {
                    match origin {
                        ReferenceOrigin::Temporary(local) => {
                            diagnostics.push(mir_diagnostic(
                                "Cannot return reference to temporary value".to_string(),
                                Self::origin_span(func, *local),
                            ));
                        }
                        ReferenceOrigin::Local(local) => {
                            diagnostics.push(mir_diagnostic(
                                "Cannot return reference to local value because it does not live long enough"
                                    .to_string(),
                                Self::origin_span(func, *local),
                            ));
                        }
                        ReferenceOrigin::Param(_)
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
            let mut state = results.entry_sets[block_idx].clone();
            for stmt in &block.statements {
                if let StatementKind::Assign(dest, rvalue) = &stmt.kind {
                    Self::report_temporary_reference_origins(
                        func,
                        diagnostics,
                        analysis.temporary_origins_for_assignment(&state, dest, rvalue),
                    );
                }
                analysis.apply_statement(&mut state, stmt);
            }

            if let Some(Terminator::Call {
                func: callable,
                args,
                destination,
                ..
            }) = &block.terminator
            {
                Self::report_temporary_reference_origins(
                    func,
                    diagnostics,
                    analysis.temporary_origins_for_call(&state, callable, args, destination),
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
                Rvalue::Use(operand) => {
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
                AccessKind::BorrowShared => {
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
                AccessKind::BorrowMut => {
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
                        LoanKind::Mut,
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
        init_analysis: &InitializationAnalysis,
        state: &InitMap,
        table: &LoanTable,
        active_loans: &LoanState,
        func: &MirFunction,
        diagnostics: &mut Diagnostics,
    ) {
        let term_span = term.origin().source_span().cloned();
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
                        term_span.clone(),
                    );
                }
                AccessKind::Move | AccessKind::Drop => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Mut,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        term_span.clone(),
                    );
                }
                AccessKind::Write
                | AccessKind::BorrowShared
                | AccessKind::BorrowMut
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

    fn check_place_loan_access(
        place: &crate::mir::Place,
        kind: LoanKind,
        table: &LoanTable,
        active_loans: &LoanState,
        _func: &MirFunction,
        diagnostics: &mut Diagnostics,
        stmt_span: Option<Span>,
    ) {
        if let Err(conflicting_loan_id) =
            LoanAnalysis::check_aliasing(place, kind, table, active_loans)
        {
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
        type_id_contains_reference, type_id_is_mut_reference, type_id_is_pointer,
        type_id_tracks_origin,
    };
    use crate::diagnostic::{DiagnosticCode, Diagnostics};
    use crate::ids::{AssocTypeId, CrateId, DefId, InstanceId, LocalDefId};
    use crate::mir::borrowck::BorrowChecker;
    use crate::mir::{
        BasicBlock, Constant, DropObligationKind, Local, LocalDecl, MirAssert, MirAssertKind,
        MirBackendContract, MirCallable, MirCallableKey, MirClosure, MirClosureCapture,
        MirClosureCaptureKind, MirClosureId, MirDropObligation, MirEnumVariantLayout, MirFunction,
        MirFunctionId, MirNominalLayout, MirOwnershipMetadata, MirProgram, MirProjectionKey,
        MirVariantLayoutFields, Mutability, Operand, Place, Projection, ReferenceOrigin, Rvalue,
        StatementData, Terminator,
    };
    use crate::type_context::{TypeContext, TypeView};
    use crate::types::{GenericParamId, Type};

    fn test_type_id(type_context: &mut TypeContext, ty: Type) -> crate::ids::TypeId {
        type_context.intern_type(&ty)
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
                        args: Vec::new(),
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
            local_decls: vec![LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: ref_i64,
            ownership: MirOwnershipMetadata::default(),
        };
        let origins = BorrowChecker::reference_return_origins(
            &function,
            &type_context,
            &MirBackendContract::default(),
            &HashMap::new(),
        );

        assert!(origins.is_empty());
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
