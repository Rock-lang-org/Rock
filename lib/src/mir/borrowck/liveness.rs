use std::collections::{HashMap, HashSet};

use crate::mir::{
    BasicBlock, BasicBlockId, Local, MirBackendContract, MirFunction, MirProjectionKey, Operand,
    Place, Projection, Rvalue, StatementData, StatementKind, Terminator,
};

use crate::ids::{AssocTypeId, DefId, LoanId, TypeId};
use crate::mir::dataflow::analyses::{LoanState, LoanTable};
use crate::mir::dataflow::{Analysis, BitSet, Lattice, LocalSet};
use crate::type_context::{Ty, TypeContext, TypeView};
use crate::types::{GenericParamId, Type};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoanLocationIndex {
    block_offsets: Vec<usize>,
    slots: Vec<BitSet<LoanId>>,
}

impl LoanLocationIndex {
    pub fn for_function(table: &LoanTable, func: &MirFunction) -> Self {
        let statement_counts: Vec<_> = func
            .basic_blocks
            .iter()
            .map(|block| block.statements.len() + 1)
            .collect();
        Self::from_table(table, func.basic_blocks.len(), &statement_counts)
    }

    fn from_table(table: &LoanTable, block_count: usize, statement_counts: &[usize]) -> Self {
        let mut block_offsets = Vec::with_capacity(block_count + 1);
        let mut next_offset = 0;
        for block_idx in 0..block_count {
            block_offsets.push(next_offset);
            next_offset += statement_counts.get(block_idx).copied().unwrap_or(0);
        }
        block_offsets.push(next_offset);

        let slots = (0..next_offset)
            .map(|_| BitSet::with_capacity(table.len()))
            .collect::<Vec<_>>();
        let mut index = Self {
            block_offsets,
            slots,
        };

        for loan in table.iter() {
            if let Some(slot) = index.slot_mut(loan.created_at) {
                slot.insert(loan.id);
            }
        }

        index
    }

    pub fn loan_ids_at(
        &self,
        location: crate::mir::borrowck::location::Location,
    ) -> impl Iterator<Item = LoanId> + '_ {
        self.slot(location).into_iter().flat_map(|slot| slot.iter())
    }

    fn slot(&self, location: crate::mir::borrowck::location::Location) -> Option<&BitSet<LoanId>> {
        let index = self.slot_index(location)?;
        self.slots.get(index)
    }

    fn slot_mut(
        &mut self,
        location: crate::mir::borrowck::location::Location,
    ) -> Option<&mut BitSet<LoanId>> {
        let index = self.slot_index(location)?;
        self.slots.get_mut(index)
    }

    fn slot_index(&self, location: crate::mir::borrowck::location::Location) -> Option<usize> {
        let block = location.block.0;
        let start = *self.block_offsets.get(block)?;
        let end = *self.block_offsets.get(block + 1)?;
        let index = start + location.statement.0;
        (index < end).then_some(index)
    }
}

#[derive(Clone, Debug)]
pub struct ReferenceLiveness {
    pub entry_sets: Vec<LocalSet>,
    pub exit_sets: Vec<LocalSet>,
    pub before_statement_sets: Vec<Vec<LocalSet>>,
    pub after_statement_sets: Vec<Vec<LocalSet>>,
}

pub fn compute_reference_liveness(
    func: &MirFunction,
    type_context: &TypeContext,
) -> ReferenceLiveness {
    let backend_contract = MirBackendContract::default();
    compute_reference_liveness_with_contract(func, type_context, &backend_contract)
}

pub fn compute_reference_liveness_with_contract(
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
) -> ReferenceLiveness {
    let reference_locals = reference_locals(func, type_context, backend_contract);
    compute_reference_liveness_for(func, &reference_locals)
}

pub fn compute_active_loan_entries(
    func: &MirFunction,
    type_context: &TypeContext,
    table: &LoanTable,
    loan_at_location: &LoanLocationIndex,
    reference_liveness: &ReferenceLiveness,
) -> Vec<LoanState> {
    let backend_contract = MirBackendContract::default();
    compute_active_loan_entries_with_contract(
        func,
        type_context,
        &backend_contract,
        table,
        loan_at_location,
        reference_liveness,
    )
}

pub fn compute_active_loan_entries_with_contract(
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    table: &LoanTable,
    loan_at_location: &LoanLocationIndex,
    reference_liveness: &ReferenceLiveness,
) -> Vec<LoanState> {
    let analysis = ActiveLoanAnalysis::new(
        type_context,
        backend_contract,
        table,
        loan_at_location,
        reference_liveness,
    );
    compute_active_loan_entries_fixpoint(&analysis, func)
}

fn compute_active_loan_entries_fixpoint(
    analysis: &ActiveLoanAnalysis<'_>,
    func: &MirFunction,
) -> Vec<LoanState> {
    let predecessors = compute_predecessors(func);
    let loan_count = analysis.table.len();
    let mut entry_sets = vec![LoanState::new(loan_count); func.basic_blocks.len()];
    let mut exit_sets = vec![LoanState::new(loan_count); func.basic_blocks.len()];

    loop {
        let mut changed = false;
        for block_idx in 0..func.basic_blocks.len() {
            let mut entry = LoanState::new(loan_count);
            if block_idx == 0 {
                entry = analysis.initial_state(func);
            }
            for &predecessor in &predecessors[block_idx] {
                entry.join(&exit_sets[predecessor]);
            }

            let mut exit = entry.clone();
            analysis.apply_block(&mut exit, func, BasicBlockId(block_idx));

            if entry_sets[block_idx] != entry {
                entry_sets[block_idx] = entry;
                changed = true;
            }
            if exit_sets[block_idx] != exit {
                exit_sets[block_idx] = exit;
                changed = true;
            }
        }

        if !changed {
            return entry_sets;
        }
    }
}

pub(super) struct ActiveLoanAnalysis<'a> {
    type_context: &'a TypeContext,
    backend_contract: &'a MirBackendContract,
    table: &'a LoanTable,
    loan_at_location: &'a LoanLocationIndex,
    reference_liveness: &'a ReferenceLiveness,
}

impl<'a> ActiveLoanAnalysis<'a> {
    pub fn new(
        type_context: &'a TypeContext,
        backend_contract: &'a MirBackendContract,
        table: &'a LoanTable,
        loan_at_location: &'a LoanLocationIndex,
        reference_liveness: &'a ReferenceLiveness,
    ) -> Self {
        Self {
            type_context,
            backend_contract,
            table,
            loan_at_location,
            reference_liveness,
        }
    }
}

impl Analysis for ActiveLoanAnalysis<'_> {
    type Domain = LoanState;

    fn initial_state(&self, _func: &MirFunction) -> Self::Domain {
        LoanState::new(self.table.len())
    }

    fn apply_statement(&self, _state: &mut Self::Domain, _stmt: &StatementData) {
        // Active-loan precision needs the function and statement index; apply_block owns it.
    }

    fn apply_terminator(&self, _state: &mut Self::Domain, _term: &Terminator) {
        // Active-loan precision needs the function and block index; apply_block owns it.
    }

    fn apply_block(&self, state: &mut Self::Domain, func: &MirFunction, block_id: BasicBlockId) {
        let block = &func.basic_blocks[block_id.0];

        for (stmt_idx, stmt) in block.statements.iter().enumerate() {
            apply_active_loan_statement_transfer(
                state,
                func,
                self.type_context,
                self.backend_contract,
                stmt,
            );

            let location = crate::mir::borrowck::location::Location::new(
                block_id,
                crate::mir::borrowck::location::StatementIndex(stmt_idx),
            );
            for loan_id in self.loan_at_location.loan_ids_at(location) {
                state.activate(loan_id, self.table);
            }

            if let Some(live_locals) = self
                .reference_liveness
                .after_statement_sets
                .get(block_id.0)
                .and_then(|stmt_sets| stmt_sets.get(stmt_idx))
            {
                state.retain_owners(|owner| live_locals.contains(owner));
            }

            if let StatementKind::StorageDead(local) = &stmt.kind {
                state.release_owner(*local);
            }
        }

        let terminator_location = crate::mir::borrowck::location::Location::new(
            block_id,
            crate::mir::borrowck::location::StatementIndex(block.statements.len()),
        );
        if let Some(term) = &block.terminator {
            apply_active_loan_terminator_transfer(
                state,
                func,
                self.type_context,
                self.backend_contract,
                term,
            );
        }
        for loan_id in self.loan_at_location.loan_ids_at(terminator_location) {
            state.activate(loan_id, self.table);
        }

        if let Some(live_locals) = self.reference_liveness.exit_sets.get(block_id.0) {
            state.retain_owners(|owner| live_locals.contains(owner));
        }
    }
}

pub(super) fn apply_active_loan_statement_transfer(
    state: &mut LoanState,
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    stmt: &StatementData,
) {
    match &stmt.kind {
        StatementKind::Assign(
            dest,
            Rvalue::Use(Operand::Move(src)) | Rvalue::Object(Operand::Move(src), _),
        ) => {
            state.transfer_owner(src.local, dest.local);
        }
        StatementKind::Assign(
            dest,
            Rvalue::Use(Operand::Copy(src)) | Rvalue::Object(Operand::Copy(src), _),
        ) => {
            if tracks_local_reference_liveness(func, type_context, backend_contract, src.local) {
                state.copy_owner(src.local, dest.local);
            }
        }
        StatementKind::Assign(dest, Rvalue::Ref(_, place))
            if matches!(
                place.projection.first(),
                Some(crate::mir::Projection::Deref)
            ) && (local_is_reference(func, type_context, place.local)
                || func
                    .local_decls
                    .get(place.local.0)
                    .is_some_and(|decl| target_is_pointer(type_context, decl.ty))) =>
        {
            state.copy_owner(place.local, dest.local);
        }
        StatementKind::Assign(dest, Rvalue::Aggregate(_, operands)) => {
            for operand in operands {
                let (Operand::Copy(place) | Operand::Move(place)) = operand else {
                    continue;
                };
                if !tracks_local_reference_liveness(
                    func,
                    type_context,
                    backend_contract,
                    place.local,
                ) {
                    continue;
                }
                match operand {
                    Operand::Copy(_) => {
                        state.copy_owner(place.local, dest.local);
                    }
                    Operand::Move(_) => {
                        state.transfer_owner(place.local, dest.local);
                    }
                    Operand::Constant(_) => unreachable!(),
                }
            }
        }
        StatementKind::Assign(dest, Rvalue::Cast(op, target)) => {
            if let Operand::Copy(place) | Operand::Move(place) = op {
                if local_is_mutable_reference(func, type_context, place.local)
                    && target_is_pointer(type_context, *target)
                {
                    state.transfer_owner(place.local, dest.local);
                } else if local_is_reference(func, type_context, place.local)
                    && target_is_pointer(type_context, *target)
                {
                    state.copy_owner(place.local, dest.local);
                }
            }
        }
        _ => {}
    }
}

fn apply_active_loan_terminator_transfer(
    state: &mut LoanState,
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    term: &Terminator,
) {
    let Terminator::Call {
        func: callee,
        args,
        destination,
        ..
    } = term
    else {
        return;
    };

    if matches!(
        callee,
        Operand::Constant(
            crate::mir::Constant::VirtualTarget(_) | crate::mir::Constant::OwnedObjectCall(_)
        )
    ) {
        // Unknown implementations may return any input borrow and store it through
        // any mutable input. Metadata itself never creates an origin.
        for arg in args {
            let (Operand::Copy(source) | Operand::Move(source)) = arg else {
                continue;
            };
            if local_contains_reference(func, type_context, backend_contract, destination.local) {
                state.copy_owner(source.local, destination.local);
            }
            for target in args {
                let (Operand::Copy(target) | Operand::Move(target)) = target else {
                    continue;
                };
                if local_is_mutable_reference(func, type_context, target.local) {
                    state.copy_owner(source.local, target.local);
                }
            }
        }
        return;
    }
    if !local_contains_reference(func, type_context, backend_contract, destination.local) {
        return;
    }

    // Aggregate/raw handles can carry payload loans through ordinary protocol
    // split/rebuild calls; their physical pointer fields do not end those loans.
    for arg in args {
        let (Operand::Copy(source) | Operand::Move(source)) = arg else {
            continue;
        };
        if local_contains_reference(func, type_context, backend_contract, source.local) {
            state.copy_owner(source.local, destination.local);
        }
    }
}

fn local_contains_reference(
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    local: Local,
) -> bool {
    let type_view = TypeView::new(type_context);
    func.local_decls.get(local.0).is_some_and(|decl| {
        type_id_contains_reference_with_contract(type_view, backend_contract, decl.ty)
    })
}

fn local_is_reference(func: &MirFunction, type_context: &TypeContext, local: Local) -> bool {
    let type_view = TypeView::new(type_context);
    matches!(
        func.local_decls
            .get(local.0)
            .map(|decl| type_view.ty(decl.ty)),
        Some(Ty::Reference { .. })
    )
}

fn local_is_mutable_reference(
    func: &MirFunction,
    type_context: &TypeContext,
    local: Local,
) -> bool {
    let type_view = TypeView::new(type_context);
    func.local_decls
        .get(local.0)
        .is_some_and(|decl| super::type_id_is_mut_reference(type_view, decl.ty))
}

fn target_is_pointer(type_context: &TypeContext, target: crate::ids::TypeId) -> bool {
    super::type_id_is_pointer(TypeView::new(type_context), target)
}

fn compute_successors(func: &MirFunction) -> Vec<Vec<usize>> {
    let mut successors = vec![Vec::new(); func.basic_blocks.len()];

    for (block_idx, block) in func.basic_blocks.iter().enumerate() {
        if let Some(term) = &block.terminator {
            match term {
                crate::mir::Terminator::Goto(target)
                | crate::mir::Terminator::GotoWithOrigin { target, .. } => {
                    successors[block_idx].push(target.0);
                }
                crate::mir::Terminator::SwitchInt {
                    targets, otherwise, ..
                }
                | crate::mir::Terminator::SwitchIntWithOrigin {
                    targets, otherwise, ..
                } => {
                    for (_, target) in targets {
                        successors[block_idx].push(target.0);
                    }
                    successors[block_idx].push(otherwise.0);
                }
                crate::mir::Terminator::Call { target, .. } => {
                    successors[block_idx].push(target.0);
                }
                crate::mir::Terminator::Drop { target, .. }
                | crate::mir::Terminator::DropWithOrigin { target, .. } => {
                    successors[block_idx].push(target.0);
                }
                crate::mir::Terminator::Return
                | crate::mir::Terminator::Unreachable { .. }
                | crate::mir::Terminator::ReturnWithOrigin { .. } => {}
            }
        }
    }

    successors
}

fn compute_predecessors(func: &MirFunction) -> Vec<Vec<usize>> {
    let successors = compute_successors(func);
    let mut predecessors = vec![Vec::new(); func.basic_blocks.len()];
    for (block_idx, block_successors) in successors.iter().enumerate() {
        for &successor in block_successors {
            predecessors[successor].push(block_idx);
        }
    }
    predecessors
}

fn reference_locals(
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
) -> LocalSet {
    let type_view = TypeView::new(type_context);
    let mut locals = LocalSet::with_capacity(func.local_decls.len());
    for (idx, decl) in func.local_decls.iter().enumerate() {
        if tracks_reference_liveness(type_view, backend_contract, decl.ty) {
            locals.insert(Local(idx));
        }
    }

    locals
}

fn tracks_reference_liveness(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: TypeId,
) -> bool {
    type_id_contains_reference_with_contract(view, backend_contract, ty)
        || matches!(view.ty(ty), Ty::Function { .. } | Ty::Pointer(_))
}

#[cfg(test)]
fn type_id_contains_reference(view: TypeView<'_>, ty: TypeId) -> bool {
    let backend_contract = MirBackendContract::default();
    type_id_contains_reference_with_contract(view, &backend_contract, ty)
}

pub(crate) fn type_id_contains_reference_with_contract(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: TypeId,
) -> bool {
    let mut seen = HashSet::new();
    type_id_contains_reference_inner(view, backend_contract, ty, &mut seen)
}

fn type_id_contains_reference_inner(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: TypeId,
    seen: &mut HashSet<TypeId>,
) -> bool {
    if !seen.insert(ty) {
        return false;
    }

    match view.ty(ty) {
        Ty::Object(_) => true,
        Ty::ObjectSelf { .. } | Ty::Witness(_) => true,
        Ty::Reference { .. } => true,
        Ty::Tuple(elems) => elems
            .iter()
            .any(|elem| type_id_contains_reference_inner(view, backend_contract, *elem, seen)),
        Ty::Array { inner, .. } | Ty::Slice(inner) | Ty::Pointer(inner) => {
            type_id_contains_reference_inner(view, backend_contract, *inner, seen)
        }
        Ty::Function {
            params,
            ret,
            captures,
            ..
        } => {
            params
                .iter()
                .any(|param| type_id_contains_reference_inner(view, backend_contract, *param, seen))
                || type_id_contains_reference_inner(view, backend_contract, *ret, seen)
                || captures.iter().any(|capture| {
                    type_id_contains_reference_inner(view, backend_contract, capture.ty, seen)
                })
        }
        Ty::Struct { id, args } => {
            args.iter()
                .any(|arg| type_id_contains_reference_inner(view, backend_contract, *arg, seen))
                || contract_struct_fields(backend_contract, *id).is_some_and(|layout| {
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
                    fields.into_iter().any(|field_ty| {
                        type_contains_reference_for_structural_type(
                            view,
                            backend_contract,
                            &field_ty.substitute_generics(&subst),
                            seen,
                        )
                    })
                })
        }
        Ty::Enum { id, args } => {
            args.iter()
                .any(|arg| type_id_contains_reference_inner(view, backend_contract, *arg, seen))
                || contract_enum_variants(backend_contract, *id).is_some_and(|variants| {
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
                    fields.into_iter().any(|field_ty| {
                        type_contains_reference_for_structural_type(
                            view,
                            backend_contract,
                            &field_ty.substitute_generics(&subst),
                            seen,
                        )
                    })
                })
        }
        Ty::Projection {
            ty,
            trait_id,
            assoc_type,
            trait_args,
        } => {
            let output_contains = projection_output_type_id(
                backend_contract,
                *ty,
                *trait_id,
                assoc_type.assoc_type_id,
                trait_args,
            )
            .is_some_and(|output| {
                let mut output_seen = seen.clone();
                type_id_contains_reference_inner(view, backend_contract, output, &mut output_seen)
            });

            output_contains
                || type_id_contains_reference_inner(view, backend_contract, *ty, seen)
                || trait_args
                    .iter()
                    .any(|arg| type_id_contains_reference_inner(view, backend_contract, *arg, seen))
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
        | Ty::Unit
        | Ty::Str
        | Ty::TypeVar(_)
        | Ty::Generic(_)
        | Ty::Constructor { .. }
        | Ty::Apply { .. }
        | Ty::Lambda { .. }
        | Ty::BoundVar { .. }
        | Ty::Error
        | Ty::Never => false,
    }
}

fn contract_struct_fields(
    backend_contract: &MirBackendContract,
    id: DefId,
) -> Option<&[(String, TypeId)]> {
    match backend_contract.nominal_layouts.get(&id) {
        Some(crate::mir::MirNominalLayout::Struct { fields, .. }) => Some(fields.as_slice()),
        _ => None,
    }
}

fn contract_enum_variants(
    backend_contract: &MirBackendContract,
    id: DefId,
) -> Option<&[crate::mir::MirEnumVariantLayout]> {
    match backend_contract.nominal_layouts.get(&id) {
        Some(crate::mir::MirNominalLayout::Enum { variants, .. }) => Some(variants.as_slice()),
        _ => None,
    }
}

fn type_contains_reference_for_structural_type(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: &Type,
    seen: &mut HashSet<TypeId>,
) -> bool {
    if let Some(id) = view.id_for_type(ty) {
        return type_id_contains_reference_inner(view, backend_contract, id, seen);
    }

    match ty {
        Type::Reference { .. } => true,
        Type::Object(_) => true,
        Type::ObjectSelf { .. } | Type::Witness(_) => true,
        Type::Tuple(elems) => elems.iter().any(|elem| {
            type_contains_reference_for_structural_type(view, backend_contract, elem, seen)
        }),
        Type::Array(inner, _) | Type::Slice(inner) => {
            type_contains_reference_for_structural_type(view, backend_contract, inner, seen)
        }
        Type::Function {
            params,
            ret,
            captures,
            ..
        } => {
            params.iter().any(|param| {
                type_contains_reference_for_structural_type(view, backend_contract, param, seen)
            }) || type_contains_reference_for_structural_type(view, backend_contract, ret, seen)
                || captures.iter().any(|capture| {
                    type_contains_reference_for_structural_type(
                        view,
                        backend_contract,
                        &capture.ty,
                        seen,
                    )
                })
        }
        Type::Struct { id, args } => {
            args.iter().any(|arg| {
                type_contains_reference_for_structural_type(view, backend_contract, arg, seen)
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
                    type_contains_reference_for_structural_type(
                        view,
                        backend_contract,
                        &field_ty.substitute_generics(&subst),
                        seen,
                    )
                })
            })
        }
        Type::Enum { id, args } => {
            args.iter().any(|arg| {
                type_contains_reference_for_structural_type(view, backend_contract, arg, seen)
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
                    type_contains_reference_for_structural_type(
                        view,
                        backend_contract,
                        &field_ty.substitute_generics(&subst),
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
            let output_contains = projection_output_for_structural_type(
                view,
                backend_contract,
                ty,
                *trait_id,
                assoc_type.assoc_type_id,
                trait_args,
            )
            .is_some_and(|output| {
                let mut output_seen = seen.clone();
                type_id_contains_reference_inner(view, backend_contract, output, &mut output_seen)
            });

            output_contains
                || type_contains_reference_for_structural_type(view, backend_contract, ty, seen)
                || trait_args.iter().any(|arg| {
                    type_contains_reference_for_structural_type(view, backend_contract, arg, seen)
                })
        }
        Type::Apply { constructor, args } => {
            type_contains_reference_for_structural_type(view, backend_contract, constructor, seen)
                || args.iter().any(|arg| {
                    type_contains_reference_for_structural_type(view, backend_contract, arg, seen)
                })
        }
        Type::Pointer(body) | Type::Lambda { body, .. } => {
            type_contains_reference_for_structural_type(view, backend_contract, body, seen)
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
        | Type::TypeVar(_)
        | Type::Generic(_)
        | Type::Constructor { .. }
        | Type::BoundVar { .. }
        | Type::Error => false,
    }
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

fn tracks_local_reference_liveness(
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    local: Local,
) -> bool {
    let type_view = TypeView::new(type_context);
    func.local_decls
        .get(local.0)
        .is_some_and(|decl| tracks_reference_liveness(type_view, backend_contract, decl.ty))
}

fn compute_reference_liveness_for(
    func: &MirFunction,
    reference_locals: &LocalSet,
) -> ReferenceLiveness {
    let successors = compute_successors(func);
    let mut entry_sets = vec![LocalSet::new(); func.basic_blocks.len()];
    let mut exit_sets = vec![LocalSet::new(); func.basic_blocks.len()];

    loop {
        let mut changed = false;

        for block_idx in (0..func.basic_blocks.len()).rev() {
            let mut live_out = LocalSet::new();
            for &succ in &successors[block_idx] {
                live_out.join(&entry_sets[succ]);
            }

            let live_in = transfer_block(
                func,
                &func.basic_blocks[block_idx],
                live_out.clone(),
                reference_locals,
            );

            if live_in != entry_sets[block_idx] {
                entry_sets[block_idx] = live_in;
                changed = true;
            }

            if live_out != exit_sets[block_idx] {
                exit_sets[block_idx] = live_out;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    let mut before_statement_sets = Vec::with_capacity(func.basic_blocks.len());
    let mut after_statement_sets = Vec::with_capacity(func.basic_blocks.len());
    for (block_idx, block) in func.basic_blocks.iter().enumerate() {
        let mut live_after = exit_sets[block_idx].clone();
        let mut block_before = vec![LocalSet::new(); block.statements.len()];
        let mut block_after = vec![LocalSet::new(); block.statements.len()];

        if let Some(term) = &block.terminator {
            apply_terminator_liveness(term, &mut live_after, reference_locals);
        }

        for stmt_idx in (0..block.statements.len()).rev() {
            if let StatementKind::Assign(dest, Rvalue::Ref(_, _)) = &block.statements[stmt_idx].kind
            {
                if reference_locals.contains(dest.local)
                    && local_preserves_reference_liveness(func, dest.local)
                {
                    live_after.insert(dest.local);
                }
            }

            block_after[stmt_idx] = live_after.clone();
            apply_statement_liveness(
                &block.statements[stmt_idx],
                &mut live_after,
                reference_locals,
            );
            block_before[stmt_idx] = live_after.clone();
        }

        before_statement_sets.push(block_before);
        after_statement_sets.push(block_after);
    }

    ReferenceLiveness {
        entry_sets,
        exit_sets,
        before_statement_sets,
        after_statement_sets,
    }
}

fn transfer_block(
    func: &MirFunction,
    block: &BasicBlock,
    mut live: LocalSet,
    reference_locals: &LocalSet,
) -> LocalSet {
    if let Some(term) = &block.terminator {
        apply_terminator_liveness(term, &mut live, reference_locals);
    }

    for stmt in block.statements.iter().rev() {
        if let StatementKind::Assign(dest, Rvalue::Ref(_, _)) = &stmt.kind {
            if reference_locals.contains(dest.local)
                && local_preserves_reference_liveness(func, dest.local)
            {
                live.insert(dest.local);
            }
        }

        apply_statement_liveness(stmt, &mut live, reference_locals);
    }

    live
}

fn local_preserves_reference_liveness(func: &MirFunction, local: Local) -> bool {
    func.local_decls
        .get(local.0)
        .is_some_and(|decl| decl.source.preserves_reference_liveness())
}

fn apply_statement_liveness(
    stmt: &StatementData,
    live: &mut LocalSet,
    reference_locals: &LocalSet,
) {
    match &stmt.kind {
        StatementKind::Assign(dest, rvalue) => {
            if dest.projection.is_empty() && reference_locals.contains(dest.local) {
                live.remove(dest.local);
            }

            for local in rvalue_reference_uses(rvalue, reference_locals) {
                live.insert(local);
            }

            if !dest.projection.is_empty() {
                let mut locals = Vec::new();
                place_reference_uses_vec(dest, reference_locals, &mut locals);
                insert_locals(live, locals);
            }

            if let Rvalue::Closure(closure) = rvalue {
                for capture in &closure.captures {
                    if reference_locals.contains(capture.local) {
                        live.insert(capture.local);
                    }
                }
            }
        }
        StatementKind::Assert(assertion) => {
            let mut locals = Vec::new();
            for operand in &assertion.operands {
                operand_reference_uses(operand, reference_locals, &mut locals);
            }
            insert_locals(live, locals);
        }
        StatementKind::StorageLive(local) => {
            live.remove(*local);
        }
        StatementKind::StorageDead(local) => {
            live.remove(*local);
        }
    }
}

fn apply_terminator_liveness(term: &Terminator, live: &mut LocalSet, reference_locals: &LocalSet) {
    match term {
        Terminator::SwitchInt { discr, .. } | Terminator::SwitchIntWithOrigin { discr, .. } => {
            let mut locals = Vec::new();
            operand_reference_uses(discr, reference_locals, &mut locals);
            insert_locals(live, locals);
        }
        Terminator::Call {
            func,
            args,
            destination,
            ..
        } => {
            if reference_locals.contains(destination.local) {
                live.remove(destination.local);
            }

            let mut locals = Vec::new();
            operand_reference_uses(func, reference_locals, &mut locals);
            for arg in args {
                operand_reference_uses(arg, reference_locals, &mut locals);
            }
            insert_locals(live, locals);
        }
        Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } => {
            let mut locals = Vec::new();
            place_reference_uses_vec(place, reference_locals, &mut locals);
            insert_locals(live, locals);
        }
        Terminator::Goto(_)
        | Terminator::Unreachable { .. }
        | Terminator::GotoWithOrigin { .. }
        | Terminator::Return
        | Terminator::ReturnWithOrigin { .. } => {}
    }
}

fn rvalue_reference_uses(rvalue: &Rvalue, reference_locals: &LocalSet) -> Vec<Local> {
    let mut locals = Vec::new();

    match rvalue {
        Rvalue::Use(op) => operand_reference_uses(op, reference_locals, &mut locals),
        Rvalue::Ref(_, place) => place_reference_uses_vec(place, reference_locals, &mut locals),
        Rvalue::Object(op, _) | Rvalue::Cast(op, _) => {
            operand_reference_uses(op, reference_locals, &mut locals)
        }
        Rvalue::BinaryOp(_, a, b) => {
            operand_reference_uses(a, reference_locals, &mut locals);
            operand_reference_uses(b, reference_locals, &mut locals);
        }
        Rvalue::UnaryOp(_, a) => operand_reference_uses(a, reference_locals, &mut locals),
        Rvalue::Discriminant(place) => {
            place_reference_uses_vec(place, reference_locals, &mut locals)
        }
        Rvalue::Aggregate(_, ops) => {
            for op in ops {
                operand_reference_uses(op, reference_locals, &mut locals);
            }
        }
        Rvalue::Closure(closure) => {
            for capture in &closure.captures {
                if reference_locals.contains(capture.local) {
                    locals.push(capture.local);
                }
            }
        }
    }

    locals
}

fn operand_reference_uses(op: &Operand, reference_locals: &LocalSet, out: &mut Vec<Local>) {
    match op {
        Operand::Copy(place) | Operand::Move(place) => {
            place_reference_uses_vec(place, reference_locals, out)
        }
        Operand::Constant(_) => {}
    }
}

fn place_reference_uses_vec(place: &Place, reference_locals: &LocalSet, out: &mut Vec<Local>) {
    if reference_locals.contains(place.local) {
        out.push(place.local);
    }

    for projection in &place.projection {
        if let Projection::Index(local) = projection {
            if reference_locals.contains(*local) {
                out.push(*local);
            }
        }
    }
}

fn insert_locals(live: &mut LocalSet, locals: Vec<Local>) {
    for local in locals {
        live.insert(local);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_active_loan_statement_transfer, compute_active_loan_entries,
        compute_active_loan_entries_with_contract, compute_reference_liveness,
        compute_reference_liveness_with_contract, type_id_contains_reference, ActiveLoanAnalysis,
        LoanLocationIndex,
    };
    use crate::ids::{AssocTypeId, CrateId, DefId, LoanId, LocalDefId};
    use crate::mir::borrowck::location::{Location, StatementIndex};
    use crate::mir::dataflow::analyses::{LoanState, LoanTable};
    use crate::mir::{
        AggregateKind, BasicBlock, BasicBlockId, Constant, Local, LocalDecl, MirBackendContract,
        MirCallable, MirCallableKey, MirFunction, MirFunctionId, MirIntrinsicId, MirNominalLayout,
        MirProjectionKey, MirVariantLayoutFields, Mutability, Operand, Place, Rvalue,
        StatementData, StatementKind, Terminator,
    };
    use crate::type_context::TypeContext;
    use crate::type_services::facts::TypeFacts;
    use crate::types::{AssociatedTypeKey, GenericParamId, Type};

    use super::super::accesses::AccessKind;
    use super::super::borrows::BorrowData;

    fn type_id(type_context: &mut TypeContext, ty: Type) -> crate::ids::TypeId {
        type_context.intern_type(&ty)
    }

    fn assoc_type(owner: DefId) -> AssociatedTypeKey {
        AssociatedTypeKey {
            owner,
            assoc_type_id: AssocTypeId(0),
        }
    }

    #[test]
    fn type_id_contains_reference_tracks_pointer_payload_and_projection_borrows() {
        let trait_id = DefId::new(CrateId(0), LocalDefId(10));
        let reference = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };
        let cases = vec![
            Type::Pointer(Box::new(reference.clone())),
            Type::Projection {
                ty: Box::new(reference.clone()),
                trait_id,
                assoc_type: assoc_type(trait_id),
                trait_args: vec![],
            },
            Type::Projection {
                ty: Box::new(Type::I64),
                trait_id,
                assoc_type: assoc_type(trait_id),
                trait_args: vec![reference],
            },
        ];

        let mut context = TypeContext::new();
        for ty in cases {
            let id = context.intern_type(&ty);
            let view = crate::type_context::TypeView::new(&context);

            // Borrowck also tracks loans carried by raw payload transport;
            // structural reference presence deliberately excludes raw pointers.
            let expected = match &ty {
                Type::Pointer(payload) => TypeFacts::contains_reference(payload),
                _ => TypeFacts::contains_reference(&ty),
            };
            assert_eq!(type_id_contains_reference(view, id), expected);
        }
    }

    fn test_function(type_context: &mut TypeContext) -> MirFunction {
        MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
            name: "main".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::storage_live(Local(1), None),
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
                    StatementData::storage_dead(Local(2), None),
                ],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: type_id(type_context, Type::Unit),
                    mutability: Mutability::Mut,
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: type_id(type_context, Type::I64),
                    mutability: Mutability::Mut,
                    name: Some("x".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: type_id(
                        type_context,
                        Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I64),
                        },
                    ),
                    mutability: Mutability::Not,
                    name: Some("r".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: type_id(type_context, Type::Unit),
            ownership: Default::default(),
        }
    }

    fn active_states_after_each_statement(
        func: &MirFunction,
        type_context: &TypeContext,
    ) -> Vec<LoanState> {
        let backend_contract = MirBackendContract::default();
        active_states_after_each_statement_with_contract(func, type_context, &backend_contract)
    }

    fn active_states_after_each_statement_with_contract(
        func: &MirFunction,
        type_context: &TypeContext,
        backend_contract: &MirBackendContract,
    ) -> Vec<LoanState> {
        let borrows = crate::mir::borrowck::borrows::collect_function_borrows(func);
        let table = LoanTable::from_borrows(&borrows);
        let by_location = LoanLocationIndex::for_function(&table, func);
        let reference_liveness =
            compute_reference_liveness_with_contract(func, type_context, backend_contract);
        let entries = compute_active_loan_entries_with_contract(
            func,
            type_context,
            backend_contract,
            &table,
            &by_location,
            &reference_liveness,
        );
        let block = &func.basic_blocks[0];
        let mut state = entries[0].clone();
        let mut states = Vec::with_capacity(block.statements.len());

        for (stmt_idx, stmt) in block.statements.iter().enumerate() {
            apply_active_loan_statement_transfer(
                &mut state,
                func,
                type_context,
                backend_contract,
                stmt,
            );
            let location = Location::new(BasicBlockId(0), StatementIndex(stmt_idx));
            for loan_id in by_location.loan_ids_at(location) {
                state.activate(loan_id, &table);
            }
            if let Some(live_locals) = reference_liveness
                .after_statement_sets
                .first()
                .and_then(|stmt_sets| stmt_sets.get(stmt_idx))
            {
                state.retain_owners(|owner| live_locals.contains(owner));
            }
            if let StatementKind::StorageDead(local) = &stmt.kind {
                state.release_owner(*local);
            }
            states.push(state.clone());
        }

        states
    }

    fn nominal_owner_transfer_function(
        type_context: &mut TypeContext,
        owner_ty: crate::ids::TypeId,
    ) -> MirFunction {
        let shared_ref_i64 = type_id(
            type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let i64 = type_id(type_context, Type::I64);
        MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
            name: "main".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::assign(
                        Place {
                            local: Local(1),
                            projection: vec![],
                        },
                        Rvalue::Ref(
                            Mutability::Not,
                            Place {
                                local: Local(3),
                                projection: vec![],
                            },
                        ),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(2),
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Move(Place {
                            local: Local(1),
                            projection: vec![],
                        })),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(0),
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
            local_decls: vec![
                LocalDecl {
                    ty: owner_ty,
                    mutability: Mutability::Mut,
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("r".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: owner_ty,
                    mutability: Mutability::Not,
                    name: Some("owner".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64,
                    mutability: Mutability::Mut,
                    name: Some("x".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: owner_ty,
            ownership: Default::default(),
        }
    }

    fn test_function_with_reference_copy(type_context: &mut TypeContext) -> MirFunction {
        MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
            name: "main".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::assign(
                        Place {
                            local: Local(1),
                            projection: vec![],
                        },
                        Rvalue::Ref(
                            Mutability::Not,
                            Place {
                                local: Local(3),
                                projection: vec![],
                            },
                        ),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(2),
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(1),
                            projection: vec![],
                        })),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(0),
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
            local_decls: vec![
                LocalDecl {
                    ty: type_id(
                        type_context,
                        Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I64),
                        },
                    ),
                    mutability: Mutability::Not,
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: type_id(
                        type_context,
                        Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I64),
                        },
                    ),
                    mutability: Mutability::Not,
                    name: Some("r1".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: type_id(
                        type_context,
                        Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I64),
                        },
                    ),
                    mutability: Mutability::Not,
                    name: Some("r2".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: type_id(type_context, Type::I64),
                    mutability: Mutability::Not,
                    name: Some("x".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: type_id(
                type_context,
                Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::I64),
                },
            ),
            ownership: Default::default(),
        }
    }

    fn branch_merge_borrow_function(
        type_context: &mut TypeContext,
    ) -> (MirFunction, BasicBlockId, Local, Local) {
        let owner_a = Local(2);
        let owner_b = Local(3);
        let merge_block = BasicBlockId(3);

        let func = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
            name: "main".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![StatementData::assign(
                        Place {
                            local: Local(1),
                            projection: vec![],
                        },
                        Rvalue::Ref(
                            Mutability::Not,
                            Place {
                                local: Local(4),
                                projection: vec![],
                            },
                        ),
                        None,
                    )],
                    terminator: Some(Terminator::SwitchInt {
                        discr: Operand::Copy(Place {
                            local: Local(5),
                            projection: vec![],
                        }),
                        targets: vec![(0, BasicBlockId(1))],
                        otherwise: BasicBlockId(2),
                    }),
                },
                BasicBlock {
                    statements: vec![StatementData::assign(
                        Place {
                            local: owner_a,
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(1),
                            projection: vec![],
                        })),
                        None,
                    )],
                    terminator: Some(Terminator::Goto(merge_block)),
                },
                BasicBlock {
                    statements: vec![StatementData::assign(
                        Place {
                            local: owner_b,
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(1),
                            projection: vec![],
                        })),
                        None,
                    )],
                    terminator: Some(Terminator::Goto(merge_block)),
                },
                BasicBlock {
                    statements: vec![
                        StatementData::assign(
                            Place {
                                local: Local(0),
                                projection: vec![],
                            },
                            Rvalue::Use(Operand::Copy(Place {
                                local: owner_a,
                                projection: vec![],
                            })),
                            None,
                        ),
                        StatementData::assign(
                            Place {
                                local: Local(0),
                                projection: vec![],
                            },
                            Rvalue::Use(Operand::Copy(Place {
                                local: owner_b,
                                projection: vec![],
                            })),
                            None,
                        ),
                    ],
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: type_id(
                        type_context,
                        Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I64),
                        },
                    ),
                    mutability: Mutability::Not,
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: type_id(
                        type_context,
                        Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I64),
                        },
                    ),
                    mutability: Mutability::Not,
                    name: Some("r".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: type_id(
                        type_context,
                        Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I64),
                        },
                    ),
                    mutability: Mutability::Not,
                    name: Some("owner_a".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: type_id(
                        type_context,
                        Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I64),
                        },
                    ),
                    mutability: Mutability::Not,
                    name: Some("owner_b".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: type_id(type_context, Type::I64),
                    mutability: Mutability::Not,
                    name: Some("x".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: type_id(type_context, Type::I64),
                    mutability: Mutability::Not,
                    name: Some("cond".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: type_id(
                type_context,
                Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::I64),
                },
            ),
            ownership: Default::default(),
        };

        (func, merge_block, owner_a, owner_b)
    }

    #[test]
    fn active_loan_entries_release_owner_after_storage_dead() {
        let mut type_context = TypeContext::new();
        let func = test_function(&mut type_context);
        let reference_liveness = compute_reference_liveness(&func, &type_context);
        let borrows = vec![BorrowData {
            owner: Local(2),
            place: Place {
                local: Local(1),
                projection: vec![],
            },
            kind: AccessKind::BorrowShared,
            created_at: Location::new(BasicBlockId(0), StatementIndex(1)),
            origin_span: None,
        }];
        let table = LoanTable::from_borrows(&borrows);
        let grouped = LoanLocationIndex::for_function(&table, &func);
        let entries = compute_active_loan_entries(
            &func,
            &type_context,
            &table,
            &grouped,
            &reference_liveness,
        );

        let block = &func.basic_blocks[0];
        let mut state = entries[0].clone();
        for (stmt_idx, stmt) in block.statements.iter().enumerate() {
            apply_active_loan_statement_transfer(
                &mut state,
                &func,
                &type_context,
                &MirBackendContract::default(),
                stmt,
            );

            let location = Location::new(BasicBlockId(0), StatementIndex(stmt_idx));
            for loan_id in grouped.loan_ids_at(location) {
                state.activate(loan_id, &table);
            }

            if let Some(live_locals) = reference_liveness
                .after_statement_sets
                .get(0)
                .and_then(|stmt_sets| stmt_sets.get(stmt_idx))
            {
                state.retain_owners(|owner| live_locals.contains(owner));
            }

            if let StatementKind::StorageDead(local) = &stmt.kind {
                state.release_owner(*local);
            }

            if stmt_idx == 1 {
                assert!(state.is_active(LoanId(0)));
                assert!(state.owner_contains(LoanId(0), Local(2)));
            }
            if stmt_idx == 2 {
                assert!(!state.is_active(LoanId(0)));
            }
        }
    }

    #[test]
    fn active_loan_statement_precision_releases_drop_in_same_block() {
        let mut type_context = TypeContext::new();
        let func = test_function(&mut type_context);
        let states = active_states_after_each_statement(&func, &type_context);

        assert!(!states[0].is_active(LoanId(0)));
        assert!(states[1].is_active(LoanId(0)));
        assert!(states[1].owner_contains(LoanId(0), Local(2)));
        assert!(!states[2].is_active(LoanId(0)));
    }

    #[test]
    fn active_loan_statement_precision_moves_owner_before_next_statement() {
        let mut type_context = TypeContext::new();
        let shared_ref_i64 = type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let i64 = type_id(&mut type_context, Type::I64);
        let func = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
            name: "main".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::assign(
                        Place {
                            local: Local(1),
                            projection: vec![],
                        },
                        Rvalue::Ref(
                            Mutability::Not,
                            Place {
                                local: Local(3),
                                projection: vec![],
                            },
                        ),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(2),
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Move(Place {
                            local: Local(1),
                            projection: vec![],
                        })),
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
                    StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(4),
                            projection: vec![],
                        })),
                        None,
                    ),
                    StatementData::storage_dead(Local(4), None),
                ],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Mut,
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("r".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("moved".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64,
                    mutability: Mutability::Mut,
                    name: Some("x".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("used".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: shared_ref_i64,
            ownership: Default::default(),
        };
        let states = active_states_after_each_statement(&func, &type_context);

        assert!(states[0].owner_contains(LoanId(0), Local(1)));
        assert!(!states[1].owner_contains(LoanId(0), Local(1)));
        assert!(states[1].owner_contains(LoanId(0), Local(2)));
        assert!(states[2].owner_contains(LoanId(0), Local(4)));
    }

    #[test]
    fn active_loan_entries_preserve_owner_for_contract_struct_field_reference() {
        let mut type_context = TypeContext::new();
        let struct_id = DefId::new(CrateId(0), LocalDefId(60));
        let generic = GenericParamId {
            owner: struct_id,
            index: 0,
        };
        let field_ty = type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Generic(generic)),
            },
        );
        let owner_ty = type_id(
            &mut type_context,
            Type::Struct {
                id: struct_id,
                args: vec![Type::I64],
            },
        );
        let func = nominal_owner_transfer_function(&mut type_context, owner_ty);
        let mut backend_contract = MirBackendContract::default();
        backend_contract.nominal_layouts.insert(
            struct_id,
            MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("value".to_string(), field_ty)],
                generic_params: vec![generic],
            },
        );

        let states = active_states_after_each_statement_with_contract(
            &func,
            &type_context,
            &backend_contract,
        );

        assert!(states[1].owner_contains(LoanId(0), Local(2)));
    }

    #[test]
    fn active_loan_entries_preserve_owner_for_contract_enum_field_reference() {
        let mut type_context = TypeContext::new();
        let enum_id = DefId::new(CrateId(0), LocalDefId(61));
        let generic = GenericParamId {
            owner: enum_id,
            index: 0,
        };
        let field_ty = type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Generic(generic)),
            },
        );
        let owner_ty = type_id(
            &mut type_context,
            Type::Enum {
                id: enum_id,
                args: vec![Type::I64],
            },
        );
        let func = nominal_owner_transfer_function(&mut type_context, owner_ty);
        let mut backend_contract = MirBackendContract::default();
        backend_contract.nominal_layouts.insert(
            enum_id,
            MirNominalLayout::Enum {
                id: enum_id,
                variants: vec![crate::mir::MirEnumVariantLayout {
                    name: "Some".to_string(),
                    fields: MirVariantLayoutFields::Positional(vec![field_ty]),
                }],
                generic_params: vec![generic],
            },
        );

        let states = active_states_after_each_statement_with_contract(
            &func,
            &type_context,
            &backend_contract,
        );

        assert!(states[1].owner_contains(LoanId(0), Local(2)));
    }

    #[test]
    fn active_loan_entries_preserve_owner_for_projection_output_nominal_reference_field() {
        let mut type_context = TypeContext::new();
        let struct_id = DefId::new(CrateId(0), LocalDefId(62));
        let trait_id = DefId::new(CrateId(0), LocalDefId(63));
        let generic = GenericParamId {
            owner: struct_id,
            index: 0,
        };
        let base_ty = type_id(&mut type_context, Type::I64);
        let field_ty = type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Generic(generic)),
            },
        );
        let output_ty = type_id(
            &mut type_context,
            Type::Struct {
                id: struct_id,
                args: vec![Type::I64],
            },
        );
        let owner_ty = type_id(
            &mut type_context,
            Type::Projection {
                ty: Box::new(Type::I64),
                trait_id,
                assoc_type: assoc_type(trait_id),
                trait_args: vec![],
            },
        );
        let func = nominal_owner_transfer_function(&mut type_context, owner_ty);
        let mut backend_contract = MirBackendContract::default();
        backend_contract.nominal_layouts.insert(
            struct_id,
            MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("value".to_string(), field_ty)],
                generic_params: vec![generic],
            },
        );
        backend_contract.projection_outputs.insert(
            MirProjectionKey {
                base: base_ty,
                trait_id,
                assoc_type_id: AssocTypeId(0),
                trait_args: vec![],
            },
            output_ty,
        );

        let states = active_states_after_each_statement_with_contract(
            &func,
            &type_context,
            &backend_contract,
        );

        assert!(states[1].owner_contains(LoanId(0), Local(2)));
    }

    #[test]
    fn active_loan_loop_backedge_preserves_statement_owner_copies() {
        let mut type_context = TypeContext::new();
        let shared_ref_i64 = type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let i64 = type_id(&mut type_context, Type::I64);
        let bool_ty = type_id(&mut type_context, Type::Bool);
        let func = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
            name: "main".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![StatementData::assign(
                        Place {
                            local: Local(1),
                            projection: vec![],
                        },
                        Rvalue::Ref(
                            Mutability::Not,
                            Place {
                                local: Local(3),
                                projection: vec![],
                            },
                        ),
                        None,
                    )],
                    terminator: Some(Terminator::Goto(BasicBlockId(1))),
                },
                BasicBlock {
                    statements: vec![
                        StatementData::assign(
                            Place {
                                local: Local(2),
                                projection: vec![],
                            },
                            Rvalue::Use(Operand::Copy(Place {
                                local: Local(1),
                                projection: vec![],
                            })),
                            None,
                        ),
                        StatementData::assign(
                            Place {
                                local: Local(5),
                                projection: vec![],
                            },
                            Rvalue::Use(Operand::Copy(Place {
                                local: Local(2),
                                projection: vec![],
                            })),
                            None,
                        ),
                    ],
                    terminator: Some(Terminator::SwitchInt {
                        discr: Operand::Copy(Place {
                            local: Local(4),
                            projection: vec![],
                        }),
                        targets: vec![(0, BasicBlockId(1))],
                        otherwise: BasicBlockId(2),
                    }),
                },
                BasicBlock {
                    statements: vec![StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(5),
                            projection: vec![],
                        })),
                        None,
                    )],
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Mut,
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("r".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("copy".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64,
                    mutability: Mutability::Mut,
                    name: Some("x".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: bool_ty,
                    mutability: Mutability::Not,
                    name: Some("again".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("loop_carried".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: shared_ref_i64,
            ownership: Default::default(),
        };
        let borrows = crate::mir::borrowck::borrows::collect_function_borrows(&func);
        let table = LoanTable::from_borrows(&borrows);
        let by_location = LoanLocationIndex::for_function(&table, &func);
        let reference_liveness = compute_reference_liveness(&func, &type_context);
        let entries = compute_active_loan_entries(
            &func,
            &type_context,
            &table,
            &by_location,
            &reference_liveness,
        );

        assert!(entries[1].owner_contains(LoanId(0), Local(1)));
        assert!(entries[1].owner_contains(LoanId(0), Local(5)));
        assert!(entries[2].owner_contains(LoanId(0), Local(5)));
    }

    #[test]
    fn active_loan_dataflow_releases_dead_owner_before_successor_entry() {
        let mut type_context = TypeContext::new();
        let unit = type_id(&mut type_context, Type::Unit);
        let i64 = type_id(&mut type_context, Type::I64);
        let shared_ref_i64 = type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let func = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
            name: "main".to_string(),
            basic_blocks: vec![
                BasicBlock {
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
                        StatementData::storage_dead(Local(2), None),
                    ],
                    terminator: Some(Terminator::Goto(BasicBlockId(1))),
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
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: i64,
                    mutability: Mutability::Mut,
                    name: Some("x".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("r".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: unit,
            ownership: Default::default(),
        };
        let borrows = crate::mir::borrowck::borrows::collect_function_borrows(&func);
        let table = LoanTable::from_borrows(&borrows);
        let grouped = LoanLocationIndex::for_function(&table, &func);
        let reference_liveness = compute_reference_liveness(&func, &type_context);
        let backend_contract = MirBackendContract::default();
        let analysis = ActiveLoanAnalysis::new(
            &type_context,
            &backend_contract,
            &table,
            &grouped,
            &reference_liveness,
        );

        let results = crate::mir::dataflow::run_fixpoint(&analysis, &func);

        assert!(!results.entry_sets[1].is_active(LoanId(0)));
    }

    #[test]
    fn reference_liveness_ignores_local_display_name_presence() {
        let mut type_context = TypeContext::new();
        let named_func = test_function(&mut type_context);
        let mut unnamed_func = named_func.clone();
        unnamed_func.local_decls[2].name = None;

        let named = compute_reference_liveness(&named_func, &type_context);
        let unnamed = compute_reference_liveness(&unnamed_func, &type_context);

        assert_eq!(named.entry_sets, unnamed.entry_sets);
        assert_eq!(named.exit_sets, unnamed.exit_sets);
        assert_eq!(named.before_statement_sets, unnamed.before_statement_sets);
        assert_eq!(named.after_statement_sets, unnamed.after_statement_sets);
    }

    #[test]
    fn test_reference_liveness_keeps_pointer_owner_live_for_projected_write() {
        let mut type_context = TypeContext::new();
        let pointer_i64 = type_id(&mut type_context, Type::Pointer(Box::new(Type::I64)));
        let i64 = type_id(&mut type_context, Type::I64);
        let mut_ref_i64 = type_id(
            &mut type_context,
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::I64),
            },
        );
        let unit = type_id(&mut type_context, Type::Unit);
        let func = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
            name: "main".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::storage_live(Local(1), None),
                    StatementData::assign(
                        Place {
                            local: Local(1),
                            projection: vec![crate::mir::Projection::Deref],
                        },
                        Rvalue::Use(crate::mir::Operand::Copy(Place {
                            local: Local(4),
                            projection: vec![],
                        })),
                        None,
                    ),
                    StatementData::assign(
                        Place {
                            local: Local(1),
                            projection: vec![crate::mir::Projection::Deref],
                        },
                        Rvalue::Use(crate::mir::Operand::Copy(Place {
                            local: Local(4),
                            projection: vec![],
                        })),
                        None,
                    ),
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
                    StatementData::storage_live(Local(3), None),
                    StatementData::assign(
                        Place {
                            local: Local(3),
                            projection: vec![],
                        },
                        Rvalue::Cast(
                            crate::mir::Operand::Move(Place {
                                local: Local(2),
                                projection: vec![],
                            }),
                            pointer_i64,
                        ),
                        None,
                    ),
                    StatementData::storage_live(Local(4), None),
                    StatementData::assign(
                        Place {
                            local: Local(4),
                            projection: vec![],
                        },
                        Rvalue::Use(crate::mir::Operand::Copy(Place {
                            local: Local(1),
                            projection: vec![],
                        })),
                        None,
                    ),
                    StatementData::storage_live(Local(5), None),
                    StatementData::assign(
                        Place {
                            local: Local(3),
                            projection: vec![crate::mir::Projection::Deref],
                        },
                        Rvalue::Use(crate::mir::Operand::Copy(Place {
                            local: Local(4),
                            projection: vec![],
                        })),
                        None,
                    ),
                    StatementData::storage_dead(Local(5), None),
                    StatementData::storage_dead(Local(4), None),
                    StatementData::storage_dead(Local(3), None),
                    StatementData::storage_dead(Local(2), None),
                    StatementData::storage_dead(Local(1), None),
                ],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: i64,
                    mutability: Mutability::Mut,
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: pointer_i64,
                    mutability: Mutability::Mut,
                    name: Some("ptr_owner".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: mut_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("r".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: pointer_i64,
                    mutability: Mutability::Not,
                    name: Some("ptr".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64,
                    mutability: Mutability::Not,
                    name: Some("y".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: unit,
                    mutability: Mutability::Not,
                    name: Some("tmp".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: i64,
            ownership: Default::default(),
        };

        let liveness = compute_reference_liveness(&func, &type_context);

        assert!(liveness.before_statement_sets[0][1].contains(Local(1)));
        assert!(liveness.after_statement_sets[0][1].contains(Local(1)));
        assert!(liveness.before_statement_sets[0][7].contains(Local(3)));
    }

    #[test]
    fn reference_liveness_uses_local_set_at_statement_boundaries() {
        let mut type_context = TypeContext::new();
        let func = test_function_with_reference_copy(&mut type_context);
        let liveness = compute_reference_liveness(&func, &type_context);

        assert!(liveness.entry_sets[0].is_empty());
        assert!(liveness.before_statement_sets[0]
            .iter()
            .any(|set| set.iter().any(|local| local == Local(1))));
    }

    #[test]
    fn active_loan_entries_use_indexed_state_and_merge_owners() {
        let mut type_context = TypeContext::new();
        let (func, merge_block, owner_a, owner_b) = branch_merge_borrow_function(&mut type_context);
        let borrows = crate::mir::borrowck::borrows::collect_function_borrows(&func);
        let table = crate::mir::dataflow::analyses::LoanTable::from_borrows(&borrows);
        let by_location = LoanLocationIndex::for_function(&table, &func);
        let reference_liveness = compute_reference_liveness(&func, &type_context);

        let entries = compute_active_loan_entries(
            &func,
            &type_context,
            &table,
            &by_location,
            &reference_liveness,
        );
        let loan_id = table.iter().next().expect("loan").id;
        let merged_owners = entries[merge_block.0].owners(loan_id);

        assert!(merged_owners.contains(owner_a));
        assert!(merged_owners.contains(owner_b));
    }

    #[test]
    fn loan_location_index_uses_dense_slots_and_deduplicates_loan_ids() {
        let mut table = LoanTable::new();
        let created_at = Location::new(BasicBlockId(0), StatementIndex(1));
        let first = table.push(crate::mir::dataflow::analyses::LoanData {
            id: LoanId(99),
            diagnostic_place: Place {
                local: Local(1),
                projection: vec![],
            },
            place_path: crate::ids::PlacePathId(0),
            initial_owner: Local(2),
            kind: crate::mir::dataflow::analyses::LoanKind::Shared,
            origin_span: None,
            created_at,
        });
        let second = table.push(crate::mir::dataflow::analyses::LoanData {
            id: LoanId(99),
            diagnostic_place: Place {
                local: Local(3),
                projection: vec![],
            },
            place_path: crate::ids::PlacePathId(0),
            initial_owner: Local(4),
            kind: crate::mir::dataflow::analyses::LoanKind::Mut,
            origin_span: None,
            created_at,
        });

        let index = super::LoanLocationIndex::from_table(&table, 1, &[2]);

        assert_eq!(
            index.loan_ids_at(created_at).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert_eq!(
            index.loan_ids_at(created_at).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert!(index
            .loan_ids_at(Location::new(BasicBlockId(0), StatementIndex(0)))
            .next()
            .is_none());
    }

    #[test]
    fn active_loan_entries_copy_receiver_loan_to_returned_reference() {
        let mut type_context = TypeContext::new();
        let i64 = type_id(&mut type_context, Type::I64);
        let shared_ref_i64 = type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let callee_ty = type_id(
            &mut type_context,
            Type::function(
                vec![Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::I64),
                }],
                Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::I64),
                },
            ),
        );
        let func = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
            name: "main".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![StatementData::assign(
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
                    )],
                    terminator: Some(Terminator::Call {
                        func: Operand::Copy(Place {
                            local: Local(4),
                            projection: vec![],
                        }),
                        args: vec![Operand::Copy(Place {
                            local: Local(2),
                            projection: vec![],
                        })],
                        destination: Place {
                            local: Local(3),
                            projection: vec![],
                        },
                        target: BasicBlockId(1),
                        span: Some(crate::lexer::Span::test()),
                    }),
                },
                BasicBlock {
                    statements: vec![StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(3),
                            projection: vec![],
                        })),
                        None,
                    )],
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64,
                    mutability: Mutability::Mut,
                    name: Some("x".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("receiver_ref".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("returned_ref".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: callee_ty,
                    mutability: Mutability::Not,
                    name: Some("callee".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: shared_ref_i64,
            ownership: Default::default(),
        };
        let borrows = crate::mir::borrowck::borrows::collect_function_borrows(&func);
        let table = crate::mir::dataflow::analyses::LoanTable::from_borrows(&borrows);
        let by_location = LoanLocationIndex::for_function(&table, &func);
        let reference_liveness = compute_reference_liveness(&func, &type_context);

        let entries = compute_active_loan_entries(
            &func,
            &type_context,
            &table,
            &by_location,
            &reference_liveness,
        );
        let loan_id = table.iter().next().expect("loan").id;

        assert!(entries[1].owners(loan_id).contains(Local(3)));
    }

    #[test]
    fn active_loan_entries_move_reference_owner_to_call_destination() {
        let mut type_context = TypeContext::new();
        let i64 = type_id(&mut type_context, Type::I64);
        let shared_ref_i64 = type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let slice_ref_i64 = type_id(
            &mut type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(Type::I64))),
            },
        );
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(141))),
            name: "move_reference_call_destination".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![StatementData::assign(
                        Place {
                            local: Local(1),
                            projection: Vec::new(),
                        },
                        Rvalue::Ref(
                            Mutability::Not,
                            Place {
                                local: Local(2),
                                projection: Vec::new(),
                            },
                        ),
                        None,
                    )],
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                            MirCallableKey::Intrinsic(MirIntrinsicId::ArrayRefToSlice),
                        ))),
                        args: vec![Operand::Move(Place {
                            local: Local(1),
                            projection: Vec::new(),
                        })],
                        destination: Place {
                            local: Local(3),
                            projection: Vec::new(),
                        },
                        target: BasicBlockId(1),
                        span: Some(crate::lexer::Span::test()),
                    }),
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
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: slice_ref_i64,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: shared_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("source".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
                LocalDecl {
                    ty: i64,
                    mutability: Mutability::Mut,
                    name: Some("value".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: slice_ref_i64,
                    mutability: Mutability::Not,
                    name: Some("destination".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: slice_ref_i64,
            ownership: Default::default(),
        };
        let borrows = crate::mir::borrowck::borrows::collect_function_borrows(&function);
        let table = crate::mir::dataflow::analyses::LoanTable::from_borrows(&borrows);
        let by_location = LoanLocationIndex::for_function(&table, &function);
        let reference_liveness = compute_reference_liveness(&function, &type_context);
        let entries = compute_active_loan_entries(
            &function,
            &type_context,
            &table,
            &by_location,
            &reference_liveness,
        );
        let loan_id = table.iter().next().expect("loan").id;

        assert!(entries[1].owners(loan_id).contains(Local(3)));
        assert!(!entries[1].owners(loan_id).contains(Local(1)));
    }

    #[test]
    fn active_loan_transfer_preserves_aggregate_reference_owner() {
        let mut type_context = TypeContext::new();
        let unit = type_id(&mut type_context, Type::Unit);
        let i64 = type_id(&mut type_context, Type::I64);
        let mut_ref = type_id(
            &mut type_context,
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::I64),
            },
        );
        let holder = type_id(
            &mut type_context,
            Type::Tuple(vec![Type::Reference {
                mutable: true,
                inner: Box::new(Type::I64),
            }]),
        );
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(140))),
            name: "aggregate_reference".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(3),
                        projection: Vec::new(),
                    },
                    Rvalue::Aggregate(
                        AggregateKind::Tuple,
                        vec![Operand::Move(Place {
                            local: Local(2),
                            projection: Vec::new(),
                        })],
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
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: i64,
                    mutability: Mutability::Mut,
                    name: Some("value".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: mut_ref,
                    mutability: Mutability::Not,
                    name: Some("reference".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
                LocalDecl {
                    ty: holder,
                    mutability: Mutability::Not,
                    name: Some("holder".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: unit,
            ownership: Default::default(),
        };
        let statement = &function.basic_blocks[0].statements[0];
        let mut state = LoanState::new(1);
        state.add_owner(LoanId(0), Local(2));

        apply_active_loan_statement_transfer(
            &mut state,
            &function,
            &type_context,
            &MirBackendContract::default(),
            statement,
        );

        assert!(!state.owner_contains(LoanId(0), Local(2)));
        assert!(state.owner_contains(LoanId(0), Local(3)));
    }

    #[test]
    fn active_loan_transfer_ignores_mut_reference_cast_to_non_pointer() {
        let mut type_context = TypeContext::new();
        let unit = type_context.intern_type(&Type::Unit);
        let i64 = type_context.intern_type(&Type::I64);
        let mut_i64_ref = type_context.intern_type(&Type::Reference {
            mutable: true,
            inner: Box::new(Type::I64),
        });
        let func = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
            name: "main".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: unit,
                    mutability: Mutability::Mut,
                    name: Some("ret".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: mut_i64_ref,
                    mutability: Mutability::Not,
                    name: Some("r".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64,
                    mutability: Mutability::Not,
                    name: Some("bits".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: unit,
            ownership: Default::default(),
        };
        let mut state = LoanState::new(1);
        state.add_owner(crate::ids::LoanId(0), Local(1));
        let stmt = StatementData::assign(
            Place {
                local: Local(2),
                projection: vec![],
            },
            Rvalue::Cast(
                Operand::Copy(Place {
                    local: Local(1),
                    projection: vec![],
                }),
                i64,
            ),
            None,
        );

        apply_active_loan_statement_transfer(
            &mut state,
            &func,
            &type_context,
            &MirBackendContract::default(),
            &stmt,
        );

        let owners = state.owners(crate::ids::LoanId(0));
        assert!(owners.contains(Local(1)));
        assert!(!owners.contains(Local(2)));
    }
}
