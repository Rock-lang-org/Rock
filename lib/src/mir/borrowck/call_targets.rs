//! Analysis-only resolution of witnessed capture-free native closure calls.
//! No runtime dispatch replacement and no effect, dictionary or ABI authority.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    key: MirCallableKey,
    ty: TypeId,
    depth: usize,
    storage: HashSet<Local>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WitnessedCall {
    function: Place,
    moved: bool,
    callable: MirCallable,
}

impl WitnessedCall {
    pub(super) fn target(&self, function: &Operand) -> Option<&MirCallable> {
        match function {
            Operand::Copy(place) if !self.moved && place == &self.function => Some(&self.callable),
            Operand::Move(place) if self.moved && place == &self.function => Some(&self.callable),
            _ => None,
        }
    }
}

/// Only a destination occurring at exactly one call site can index this
/// analysis-local witness; repeated destinations stay opaque, including joins.
pub(super) fn witnessed_calls(
    program: &MirProgram,
) -> HashMap<(MirFunctionId, Place), WitnessedCall> {
    let resolved = resolved_bodies(program);
    let mut calls = HashMap::new();
    for (id, original) in program.functions() {
        let Some(body) = resolved.get(id) else {
            continue;
        };
        let mut seen = HashSet::new();
        let mut repeated = HashSet::new();
        for (block, original_block) in original.basic_blocks.iter().enumerate() {
            let Some(Terminator::Call {
                func, destination, ..
            }) = &original_block.terminator
            else {
                continue;
            };
            if !seen.insert(destination.clone()) {
                repeated.insert(destination.clone());
                continue;
            }
            let Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(callable)),
                ..
            }) = &body.basic_blocks[block].terminator
            else {
                continue;
            };
            let (Operand::Copy(function) | Operand::Move(function)) = func else {
                continue;
            };
            calls.insert(
                (id.clone(), destination.clone()),
                WitnessedCall {
                    function: function.clone(),
                    moved: matches!(func, Operand::Move(_)),
                    callable: callable.clone(),
                },
            );
        }
        for destination in repeated {
            calls.remove(&(id.clone(), destination));
        }
    }
    calls
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State(Option<HashMap<Local, Target>>);

impl Lattice for State {
    fn join(&mut self, other: &Self) -> bool {
        let Some(incoming) = &other.0 else {
            return false;
        };
        let Some(current) = &mut self.0 else {
            self.0 = other.0.clone();
            return true;
        };
        let previous = current.clone();
        current.retain(|local, target| incoming.get(local) == Some(target));
        *current != previous
    }
}

struct Targets<'a> {
    program: &'a MirProgram,
    body: &'a MirFunction,
}

impl Targets<'_> {
    fn place(&self, state: &State, place: &Place) -> Option<Target> {
        let mut target = state.0.as_ref()?.get(&place.local)?.clone();
        for projection in &place.projection {
            if *projection != Projection::Deref || target.depth == 0 {
                return None;
            }
            let Ty::Reference {
                mutable: false,
                inner,
            } = self.program.type_context.try_ty(target.ty)?
            else {
                return None;
            };
            target.depth -= 1;
            target.ty = *inner;
        }
        Some(target)
    }

    fn invalidate(facts: &mut HashMap<Local, Target>, local: Local) {
        facts.retain(|key, target| *key != local && !target.storage.contains(&local));
    }
}

impl Analysis for Targets<'_> {
    type Domain = State;
    fn initial_state(&self, _: &MirFunction) -> State {
        State(Some(HashMap::new()))
    }
    fn bottom_state(&self, _: &MirFunction) -> State {
        State::default()
    }

    fn apply_statement(&self, state: &mut State, statement: &StatementData) {
        let candidate = match &statement.kind {
            StatementKind::Assign(destination, Rvalue::Closure(closure))
                if destination.projection.is_empty() =>
            {
                self.body
                    .local_decls
                    .get(destination.local.0)
                    .and_then(|local| {
                        crate::mir::erased::capture_free_closure_target(
                            self.program,
                            self.body,
                            closure,
                            local.ty,
                        )
                        .map(|key| Target {
                            key,
                            ty: local.ty,
                            depth: 0,
                            storage: HashSet::new(),
                        })
                    })
            }
            StatementKind::Assign(_, Rvalue::Use(Operand::Copy(place) | Operand::Move(place))) => {
                self.place(state, place)
            }
            StatementKind::Assign(destination, Rvalue::Ref(Mutability::Not, place)) => {
                self.place(state, place).and_then(|mut target| {
                    let ty = self.body.local_decls.get(destination.local.0)?.ty;
                    if self.program.type_context.try_ty(ty)
                        != Some(&Ty::Reference {
                            mutable: false,
                            inner: target.ty,
                        })
                    {
                        return None;
                    }
                    target.depth += 1;
                    target.ty = ty;
                    target.storage.insert(place.local);
                    Some(target)
                })
            }
            _ => None,
        };
        let Some(facts) = &mut state.0 else {
            return;
        };
        match &statement.kind {
            StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                Self::invalidate(facts, *local)
            }
            StatementKind::Assign(destination, value) => {
                if !destination.projection.is_empty()
                    || matches!(value, Rvalue::Ref(Mutability::Mut, _) | Rvalue::Cast(_, _))
                {
                    facts.clear();
                    return;
                }
                if let Rvalue::Use(Operand::Move(place)) = value {
                    Self::invalidate(facts, place.local);
                }
                if let Rvalue::Closure(closure) = value {
                    for capture in &closure.captures {
                        Self::invalidate(facts, capture.local);
                    }
                }
                Self::invalidate(facts, destination.local);
                if let Some(target) = candidate.filter(|target| {
                    self.body
                        .local_decls
                        .get(destination.local.0)
                        .is_some_and(|local| local.ty == target.ty)
                }) {
                    facts.insert(destination.local, target);
                }
            }
            StatementKind::Assert(_) => {}
        }
    }

    fn apply_terminator(&self, state: &mut State, term: &Terminator) {
        let Some(facts) = &mut state.0 else {
            return;
        };
        match term {
            Terminator::Call {
                func:
                    Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Intrinsic(intrinsic),
                    ))),
                args,
                destination,
                ..
            } if intrinsic.is_type_only()
                && matches!(args.as_slice(), [Operand::Constant(Constant::TypeId(ty))]
                    if self.program.type_context.type_id_tree_is_valid(*ty)) =>
            {
                // A native type-metadata instruction evaluates no value or
                // callable storage. Its destination still overwrites facts.
                Self::invalidate(facts, destination.local);
            }
            // Opaque effects may change exposed callable storage. Return-source
            // resolution itself supplies no empty-effect permission.
            Terminator::Call { .. } => facts.clear(),
            Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } => {
                Self::invalidate(facts, place.local)
            }
            _ => {}
        }
    }
}

pub(super) fn resolved_bodies(program: &MirProgram) -> HashMap<MirFunctionId, MirFunction> {
    if !crate::mir::backend_contract::validate_backend_contract_against_mir(
        &program.backend_contract,
        program.functions.values(),
    )
    .is_empty()
    {
        return HashMap::new();
    }
    let mut resolved = HashMap::new();
    for (id, body) in program.functions() {
        let analysis = Targets { program, body };
        let results = run_fixpoint(&analysis, body);
        let mut changed = None;
        for (index, block) in body.basic_blocks.iter().enumerate() {
            if !results.reachable[index] {
                continue;
            }
            let Some(Terminator::Call {
                func: Operand::Copy(place) | Operand::Move(place),
                args,
                destination,
                ..
            }) = &block.terminator
            else {
                continue;
            };
            let mut state = results.entry_sets[index].clone();
            for statement in &block.statements {
                analysis.apply_statement(&mut state, statement);
            }
            let Some(target) = analysis
                .place(&state, place)
                .filter(|target| target.depth == 0)
            else {
                continue;
            };
            let callee = Operand::Constant(Constant::Callable(MirCallable::Resolved(target.key)));
            if super::return_sources::checked_call(
                body,
                &program.type_context,
                &program.backend_contract,
                &callee,
                args,
                destination,
            )
            .is_none()
            {
                continue;
            }
            // Clone only for origin analysis; the original allocation/body IDs
            // were checked above and executable MIR remains genuinely indirect.
            let copy = changed.get_or_insert_with(|| body.clone());
            if let Some(Terminator::Call { func, .. }) = &mut copy.basic_blocks[index].terminator {
                *func = callee;
            }
        }
        if let Some(body) = changed {
            resolved.insert(id.clone(), body);
        }
    }
    resolved
}
