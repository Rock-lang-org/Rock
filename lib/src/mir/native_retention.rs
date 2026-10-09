//! Caller-local, nonserialized retention evidence from actual closure storage.
use std::collections::{HashMap, HashSet};

use crate::ids::TypeId;
use crate::mir::dataflow::{run_fixpoint, Analysis, Lattice};
use crate::mir::{
    Local, LocalSource, MirCallableKey, MirCallableKind, MirClosure, MirDictionaryMemberTarget,
    MirClosureCaptureKind, MirClosureId, MirFunction, MirFunctionId, MirProgram, Operand,
    Rvalue, StatementData, StatementKind, Terminator,
};
use crate::types::Type;

#[cfg(test)]
#[path = "native_retention_tests.rs"]
mod tests;

/// Only `check_closure` can construct this seal. Signature capture metadata,
/// including an empty capture list, never creates or forwards one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NoReferenceDestination {
    closure: MirClosureId,
    ty: TypeId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct State(Option<HashMap<Local, NoReferenceDestination>>);

impl Lattice for State {
    fn join(&mut self, other: &Self) -> bool {
        let Some(incoming) = &other.0 else { return false; };
        let Some(current) = &mut self.0 else {
            self.0 = other.0.clone();
            return true;
        };
        let old_len = current.len();
        // Must evidence: every reachable predecessor must carry the same seal.
        current.retain(|local, seal| incoming.get(local) == Some(seal));
        current.len() != old_len
    }
}

/// Exact storage proof for the currently supported visible capture leaves.
/// Opaque nominal/function/raw-pointer storage is deliberately not certified.
// lean-ctx: visible storage ceiling; upgrade with an exact nested environment/layout proof.
fn reference_free_storage(ty: &Type) -> bool {
    match ty {
        Type::I8 | Type::I16 | Type::I32 | Type::I64
        | Type::U8 | Type::U16 | Type::U32 | Type::U64
        | Type::F32 | Type::F64 | Type::Bool | Type::Char
        | Type::Str | Type::Unit | Type::Never => true,
        Type::Tuple(fields) => fields.iter().all(reference_free_storage),
        Type::Array(element, len) => *len == 0 || reference_free_storage(element),
        _ => false,
    }
}

fn check_closure(
    program: &MirProgram, owner: &MirFunction, closure: &MirClosure, ty: TypeId,
) -> Option<NoReferenceDestination> {
    let tc = &program.type_context;
    let contract = &program.backend_contract;
    if closure.id.owner != owner.id || !tc.type_id_tree_is_valid(ty) { return None; }
    let Type::Function { params, ret, .. } = tc.type_for(ty) else { return None; };
    let id = MirFunctionId::Closure(Box::new(closure.id.clone()));
    let body = program.functions.get(&id)?;
    let key = MirCallableKey::Closure(id.clone());
    let declaration = contract.callable(&key)?;
    if body.id != id || declaration.key != key || contract.function_bodies.get(&id) != Some(&key)
        || !matches!(&declaration.kind, MirCallableKind::LocalBody { function_id } if *function_id == id)
        || body.arg_count != params.len()
        || !tc.type_id_tree_is_valid(body.ret_type) || tc.type_for(body.ret_type) != *ret
        || declaration.signature.ret.semantic_ty != body.ret_type
        || declaration.signature.ret.abi_ty != body.ret_type
        || declaration.signature.params.len() != params.len()
        || body.local_decls.first()?.ty != body.ret_type
        || body.closure_captures.len() != closure.captures.len()
    { return None; }
    for (index, (parameter, expected)) in declaration.signature.params.iter().zip(&params).enumerate() {
        if !tc.type_id_tree_is_valid(parameter.semantic_ty)
            || tc.type_for(parameter.semantic_ty) != *expected
            || body.local_decls.get(index + 1)?.ty != parameter.semantic_ty
            || body.local_decls.get(index + 1)?.source != LocalSource::Argument
        { return None; }
    }
    let declared = body.local_decls.iter().enumerate()
        .filter_map(|(index, local)| (local.source == LocalSource::ClosureCapture).then_some(Local(index)))
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    for (supplied, stored) in closure.captures.iter().zip(&body.closure_captures) {
        // Borrowed captures are actual reference slots regardless of the type
        // of the local they refer to. Only checked by-value storage can seal.
        if supplied.kind != MirClosureCaptureKind::ByValue || stored.kind != supplied.kind
            || !declared.contains(&stored.local) || !seen.insert(stored.local)
        { return None; }
        let source = owner.local_decls.get(supplied.local.0)?;
        let destination = body.local_decls.get(stored.local.0)?;
        if source.ty != destination.ty || !tc.type_id_tree_is_valid(source.ty)
            || !reference_free_storage(&tc.type_for(source.ty))
        { return None; }
    }
    if seen != declared { return None; }
    Some(NoReferenceDestination { closure: closure.id.clone(), ty })
}

struct EnvironmentAnalysis<'a> {
    program: &'a MirProgram,
    function: &'a MirFunction,
}

/// Exact capture-free allocation and body agreement, not abstract Fn metadata.
pub(super) fn capture_free_closure_target(
    program: &MirProgram, owner: &MirFunction, closure: &MirClosure, ty: TypeId,
) -> Option<MirCallableKey> {
    if !closure.captures.is_empty() { return None; }
    let seal = check_closure(program, owner, closure, ty)?;
    Some(MirCallableKey::Closure(MirFunctionId::Closure(Box::new(seal.closure))))
}

impl Analysis for EnvironmentAnalysis<'_> {
    type Domain = State;

    fn initial_state(&self, _: &MirFunction) -> State { State(Some(HashMap::new())) }
    fn bottom_state(&self, _: &MirFunction) -> State { State::default() }

    fn apply_statement(&self, state: &mut State, statement: &StatementData) {
        let Some(facts) = &mut state.0 else { return; };
        match &statement.kind {
            StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                facts.remove(local);
            }
            StatementKind::Assign(destination, value) => {
                if !destination.projection.is_empty() {
                    // Without alias-qualified write authority, a projected write
                    // may overwrite an exposed callback. Do not retain a seal.
                    facts.clear();
                    return;
                }
                let ty = self.function.local_decls.get(destination.local.0).map(|local| local.ty);
                let seal = match (value, ty) {
                    (Rvalue::Closure(closure), Some(ty)) => check_closure(self.program, self.function, closure, ty),
                    (Rvalue::Use(Operand::Copy(source) | Operand::Move(source)), Some(ty))
                        if source.projection.is_empty() => facts.get(&source.local)
                            .filter(|seal| seal.ty == ty).cloned(),
                    _ => None,
                };
                if let Rvalue::Use(Operand::Move(source)) = value {
                    facts.remove(&source.local);
                }
                if let Rvalue::Ref(_, source) = value {
                    // Alias-exposed storage needs a stronger forwarding proof.
                    if let Some(exposed) = facts.get(&source.local).cloned() {
                        facts.retain(|_, seal| seal.closure != exposed.closure);
                    }
                }
                if let Rvalue::Closure(closure) = value {
                    for capture in &closure.captures { facts.remove(&capture.local); }
                }
                facts.remove(&destination.local);
                if let Some(seal) = seal { facts.insert(destination.local, seal); }
            }
            StatementKind::Assert(_) => {}
        }
    }

    fn apply_terminator(&self, state: &mut State, term: &Terminator) {
        let Some(facts) = &mut state.0 else { return; };
        match term {
            Terminator::Call { func, args, destination, .. } => {
                for operand in std::iter::once(func).chain(args) {
                    if let Operand::Copy(place) | Operand::Move(place) = operand { facts.remove(&place.local); }
                }
                facts.remove(&destination.local);
            }
            Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } => {
                facts.remove(&place.local);
            }
            _ => {}
        }
    }
}

/// Evidence is bound to one actual native call's argument ordinals and types.
/// It is neither persisted nor inferred from a dictionary's function signature.
pub(super) struct NativeRetentionProof {
    arguments: Vec<Option<NoReferenceDestination>>,
    call: Option<super::MirErasedCall>,
}

impl NativeRetentionProof {
    pub(super) fn for_call(program: &MirProgram, function: &MirFunction, block: usize) -> Self {
        let analysis = EnvironmentAnalysis { program, function };
        let results = run_fixpoint(&analysis, function);
        let mut state = results.entry_sets[block].clone();
        let block = &function.basic_blocks[block];
        for statement in &block.statements { analysis.apply_statement(&mut state, statement); }
        let args = match &block.terminator { Some(Terminator::Call { args, .. }) => args.as_slice(), _ => &[] };
        let call = match &block.terminator {
            Some(Terminator::Call { func: Operand::Constant(crate::mir::Constant::ErasedCall(call)), .. }) => Some(call.clone()),
            _ => None,
        };
        Self { call, arguments: args.iter().map(|argument| {
            let (Operand::Copy(place) | Operand::Move(place)) = argument else { return None; };
            if !place.projection.is_empty() { return None; }
            state.0.as_ref()?.get(&place.local).cloned()
        }).collect() }
    }

    pub(super) fn matches_call(&self, call: &super::MirErasedCall) -> bool {
        self.call.as_ref() == Some(call)
    }

    pub(super) fn covers_subject(
        &self, signature: &super::MirErasedSignature, dictionary: usize, member: usize,
        destination: &Type, contract: &crate::mir::MirBackendContract,
        tc: &crate::type_context::TypeContext,
    ) -> bool {
        if !matches!(destination, Type::Function { .. }) { return false; }
        let Some(required) = signature.dictionaries.get(dictionary) else { return false; };
        let Some(expected) = required.members.get(member) else { return false; };
        let Some(actual) = self.call.as_ref().and_then(|call| call.dictionaries.get(dictionary))
            .and_then(|id| contract.erased.dictionaries.get(id)) else { return false; };
        let Some(entry) = actual.members.get(member) else { return false; };
        let MirDictionaryMemberTarget::Native(MirCallableKey::ObjectAdapter(
            crate::mir::MirObjectAdapterKey::NativeCallable { concrete, kind },
        )) = &entry.target
            else { return false; };
        // The exact environment proof applies to native invocation, not a
        // custom trait implementation that can replace an abstract function value.
        if *concrete != actual.subject || !tc.type_id_tree_is_valid(*concrete)
            || tc.type_for(*concrete) != *destination || actual.trait_id != required.trait_id
            || entry.member_id != expected.member_id || entry.receiver != expected.receiver
            || entry.receiver != Some(match kind {
                crate::types::CallableKind::Fn => crate::types::ReceiverMode::Shared,
                crate::types::CallableKind::FnMut => crate::types::ReceiverMode::Mut,
                crate::types::CallableKind::FnOnce => crate::types::ReceiverMode::Move,
            })
        { return false; }
        let mut found = false;
        for (index, parameter) in signature.params.iter().enumerate() {
            if parameter != &required.subject { continue; }
            found = true;
            if !self.arguments.get(index).and_then(Option::as_ref)
                .is_some_and(|seal| tc.type_for(seal.ty) == *destination)
            { return false; }
        }
        found
    }
}
