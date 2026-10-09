//! Measured producer evidence at callable-value bindings, never type-header grants.

use std::collections::{HashMap, HashSet};

use super::{allocation_body, body_sources_with_calls};
mod forwarding;
use crate::diagnostic::Diagnostics;
use crate::ids::TypeId;
use crate::mir::borrowck::{mir_diagnostic, OriginPurpose, OriginSet};
use crate::mir::dataflow::{run_fixpoint, Analysis, Lattice};
use crate::mir::{
    Constant, Local, MirBackendContract, MirCallable, MirCallableKind, MirFunction, MirFunctionId,
    MirProgram, Mutability, Operand, Place, Projection, ReferenceOrigin, Rvalue, StatementData,
    StatementKind, Terminator,
};
use crate::type_context::{Ty, TypeContext};
use crate::types::{CallableResultBorrowing, CallableResultSource};

/// All reachable occurrences sharing this binding must be certified. Keeping the
/// callable value as input zero transports its carried loans, not its storage.
#[derive(Default, PartialEq, Eq)]
pub(in crate::mir::borrowck) struct CertifiedCalls {
    // Conditional entry hypotheses are scoped to one definition and discharged
    // by validate() at all actual bindings; they are not measured independence.
    inputs: HashMap<MirFunctionId, HashMap<Place, Evidence>>,
    adapter_inputs: HashMap<
        MirFunctionId,
        std::collections::BTreeMap<usize, crate::mir::backend_contract::MirGenericCallableInput>,
    >,
    calls: HashMap<(MirFunctionId, Place, Local), Evidence>,
    returns: HashMap<MirFunctionId, Evidence>,
    forwarded_returns: HashMap<MirFunctionId, forwarding::Recipe>,
}

impl CertifiedCalls {
    pub(in crate::mir::borrowck) fn portable_return(
        &self,
        function: &MirFunctionId,
    ) -> Option<CallableResultBorrowing> {
        let evidence = self.returns.get(function)?;
        let CallableResultBorrowing::Sources(sources) = &evidence.sources else {
            return None;
        };
        // One portable envelope can serve both consumers only when their
        // independently measured roles agree. Raw lease/storage distinctions
        // need separate portable authority, not a union used as a loan map.
        (evidence.sources == evidence.value_sources
            && sources
                .iter()
                .all(|source| *source != CallableResultSource::EnvironmentStorage))
        .then(|| evidence.sources.clone())
    }

    pub(in crate::mir::borrowck) fn for_program(
        program: &MirProgram,
        lifetime: &HashMap<MirFunctionId, OriginSet>,
        values: &HashMap<MirFunctionId, OriginSet>,
        previous: &crate::mir::borrowck::return_sources::NativeReturnSources,
    ) -> Self {
        if !crate::mir::backend_contract::validate_backend_contract_against_mir(
            &program.backend_contract,
            program.functions.values(),
        )
        .is_empty()
        {
            return Self::default();
        }
        let mut calls: HashMap<_, Option<Evidence>> = HashMap::new();
        let mut returns: HashMap<_, Option<Evidence>> = HashMap::new();
        let mut inputs = HashMap::new();
        let adapter_inputs =
            forwarding::adapter_input_obligations(program, lifetime, values, previous);
        for (key, obligations) in &program.backend_contract.callable_input_obligations {
            let Some(id) = MirCallable::Resolved(key.clone()).function_id() else {
                continue;
            };
            let Some(body) = program.functions.get(&id) else {
                continue;
            };
            let mut entry = HashMap::new();
            for (index, ty) in obligations {
                let local = Local(index + 1);
                if *index >= body.arg_count
                    || !program.type_context.type_id_tree_is_valid(*ty)
                    || !body.local_decls.get(local.0).is_some_and(|decl| {
                        decl.ty == *ty && decl.source == crate::mir::LocalSource::Argument
                    })
                {
                    continue;
                }
                let Some(Ty::Function {
                    result_borrowing: CallableResultBorrowing::Sources(sources),
                    ..
                }) = program.type_context.try_ty(*ty)
                else {
                    continue;
                };
                // lean-ctx: role-agreeing input hypotheses; raw lifetime leases
                // require distinct canonical ValueLoans obligations first.
                if sources.iter().any(|source| {
                    matches!(
                        source,
                        CallableResultSource::ArgumentBorrowedContents(_)
                            | CallableResultSource::EnvironmentBorrowedContents
                            | CallableResultSource::EnvironmentStorage
                    )
                }) {
                    continue;
                }
                let required = CallableResultBorrowing::Sources(sources.clone());
                entry.insert(
                    Place {
                        local,
                        projection: Vec::new(),
                    },
                    Evidence {
                        ty: *ty,
                        sources: required.clone(),
                        value_sources: required,
                        environment: Some(Place {
                            local,
                            projection: Vec::new(),
                        }),
                        storage: HashSet::new(),
                        isolated: false,
                        factory: None,
                    },
                );
            }
            if !entry.is_empty() {
                inputs.insert(id, entry);
            }
        }
        for (_, owner) in program.functions() {
            if let Some(key) = program.backend_contract.function_bodies.get(&owner.id) {
                if let Some(requirements) = program
                    .backend_contract
                    .generic_callable_input_obligations
                    .get(key)
                {
                    let entry = inputs.entry(owner.id.clone()).or_insert_with(HashMap::new);
                    for (index, input) in requirements {
                        let local = Local(index + 1);
                        if *index >= owner.arg_count
                            || !owner.local_decls.get(local.0).is_some_and(|decl| {
                                decl.ty == input.subject
                                    && decl.source == crate::mir::LocalSource::Argument
                            })
                            || !program.type_context.type_id_tree_is_valid(input.subject)
                            || !program.type_context.type_id_tree_is_valid(input.invocation)
                        {
                            continue;
                        }
                        for payload in crate::mir::backend_contract::callable_payload_requirements(
                            &program.type_context,
                            &program.backend_contract,
                            input,
                        ) {
                            let place = Place {
                                local,
                                projection: payload.projection,
                            };
                            if program.backend_contract.place_type_id(
                                &program.type_context,
                                owner,
                                &place,
                            ) != Some(payload.ty)
                            {
                                continue;
                            }
                            entry.insert(
                                place.clone(),
                                Evidence {
                                    ty: payload.ty,
                                    sources: payload.borrowing.clone(),
                                    value_sources: payload.borrowing,
                                    environment: Some(place),
                                    storage: HashSet::new(),
                                    isolated: false,
                                    factory: None,
                                },
                            );
                        }
                    }
                }
            }
        }
        // Exact by-value construction transports a checked input's conditional
        // demand into its capture slot, not into caller-selected parameters.
        for (id, captured) in forwarding::capture_inputs(program, lifetime, values, previous) {
            inputs.entry(id).or_default().extend(captured);
        }
        for (id, requirements) in &adapter_inputs {
            let Some(body) = program.functions.get(id) else {
                continue;
            };
            let entry = inputs.entry(id.clone()).or_default();
            for (index, requirement) in requirements {
                for payload in crate::mir::backend_contract::callable_payload_requirements(
                    &program.type_context,
                    &program.backend_contract,
                    requirement,
                ) {
                    let place = Place {
                        local: Local(index + 1),
                        projection: payload.projection,
                    };
                    if program
                        .backend_contract
                        .place_type_id(&program.type_context, body, &place)
                        != Some(payload.ty)
                    {
                        continue;
                    }
                    entry.insert(
                        place.clone(),
                        Evidence {
                            ty: payload.ty,
                            sources: payload.borrowing.clone(),
                            value_sources: payload.borrowing,
                            environment: Some(place),
                            storage: HashSet::new(),
                            isolated: false,
                            factory: None,
                        },
                    );
                }
            }
        }
        for (_, owner) in program.functions() {
            let returns_callable = matches!(
                program.type_context.try_ty(owner.ret_type),
                Some(Ty::Function { .. })
            );
            if !returns_callable
                && !owner.basic_blocks.iter().any(|block| {
                    matches!(
                        &block.terminator,
                        Some(Terminator::Call {
                            func: Operand::Copy(_) | Operand::Move(_),
                            ..
                        })
                    )
                })
            {
                continue;
            }
            let analysis = Bindings {
                program,
                owner,
                lifetime,
                values,
                backend_valid: true,
                calls: Some(previous),
            };
            let results = run_fixpoint(&analysis, owner);
            for (index, block) in owner.basic_blocks.iter().enumerate() {
                if !results.reachable[index] {
                    continue;
                }
                if returns_callable
                    && matches!(
                        block.terminator,
                        Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
                    )
                {
                    let mut state = results.entry_sets[index].clone();
                    for statement in &block.statements {
                        analysis.apply_statement(&mut state, statement);
                    }
                    let evidence = analysis
                        .place_evidence(
                            &state,
                            &Place {
                                local: Local(0),
                                projection: Vec::new(),
                            },
                        )
                        .filter(|value| value.ty == owner.ret_type && value.storage.is_empty());
                    returns
                        .entry(owner.id.clone())
                        .and_modify(|old| match (old.as_mut(), &evidence) {
                            (Some(old), Some(new)) if old.ty == new.ty => {
                                if old.factory != new.factory {
                                    old.factory = None;
                                }
                                old.sources = old.sources.merge(&new.sources, true);
                                old.value_sources =
                                    old.value_sources.merge(&new.value_sources, true);
                            }
                            _ => *old = None,
                        })
                        .or_insert(evidence);
                }
                let Some(Terminator::Call {
                    func: Operand::Copy(callee) | Operand::Move(callee),
                    destination,
                    ..
                }) = &block.terminator
                else {
                    continue;
                };
                if !destination.projection.is_empty() {
                    continue;
                }
                let mut state = results.entry_sets[index].clone();
                for statement in &block.statements {
                    analysis.apply_statement(&mut state, statement);
                }
                let evidence = analysis.place_evidence(&state, callee).filter(|evidence| {
                    match &evidence.sources {
                        CallableResultBorrowing::Sources(sources) => sources.iter().all(|source| {
                            !matches!(source, CallableResultSource::EnvironmentStorage)
                        }),
                        CallableResultBorrowing::Unknown => false,
                    }
                });
                let key = (owner.id.clone(), callee.clone(), destination.local);
                calls
                    .entry(key)
                    .and_modify(|old| match (old.as_mut(), &evidence) {
                        (Some(old), Some(new))
                            if old.ty == new.ty && old.environment == new.environment =>
                        {
                            if old.factory != new.factory {
                                old.factory = None;
                            }
                            old.sources = old.sources.merge(&new.sources, true);
                            old.value_sources = old.value_sources.merge(&new.value_sources, true);
                            old.storage.extend(new.storage.iter().copied());
                        }
                        _ => *old = None,
                    })
                    .or_insert(evidence);
            }
        }
        Self {
            forwarded_returns: forwarding::measure(program, lifetime, values, previous),
            adapter_inputs,
            inputs,
            calls: calls
                .into_iter()
                .filter_map(|(key, value)| Some((key, value?)))
                .collect(),
            returns: returns
                .into_iter()
                .filter_map(|(key, value)| {
                    let mut value = value?;
                    // Exported construction evidence describes invocation obligations,
                    // not an address into this function's return slot.
                    value.environment = None;
                    Some((key, value))
                })
                .collect(),
        }
    }

    fn returned_value(
        &self,
        owner: &MirFunction,
        tc: &TypeContext,
        contract: &MirBackendContract,
        func: &Operand,
        args: &[Operand],
        destination: &Place,
    ) -> Option<Evidence> {
        let (key, ret) = crate::mir::borrowck::return_sources::checked_signature(
            owner,
            tc,
            contract,
            func,
            args,
            destination,
        )?;
        let id = MirCallable::Resolved(key.clone()).function_id()?;
        let mut evidence = self.returns.get(&id).cloned().or_else(|| {
            // A header is an obligation, not a certificate. Only the validated
            // closed producer record, bound to this exact native signature and
            // object link, can carry a checked returned-callable envelope.
            let header = contract.callable(key)?;
            let origins = contract.native_return_origins.get(key)?;
            if !matches!(header.kind, MirCallableKind::ObjectProvided)
                // This branch transports returned callables, not the separate
                // scalar dependency protocol. A matching header is no escape hatch.
                || origins.sources.contains(
                    &crate::hir::borrowing::NativeReturnSource::UnknownExternal,
                )
                || origins.ret != ret
                || origins.ret != header.signature.ret.abi_ty
                || origins.params
                    != header
                        .signature
                        .params
                        .iter()
                        .map(|parameter| parameter.semantic_ty)
                        .collect::<Vec<_>>()
                || !origins.sources_are_valid(|ty| {
                    matches!(tc.try_ty(*ty), Some(Ty::Reference { .. } | Ty::Pointer(_)))
                })
            {
                return None;
            }
            let Ty::Function {
                result_borrowing, ..
            } = tc.try_ty(ret)?
            else {
                return None;
            };
            let CallableResultBorrowing::Sources(sources) = result_borrowing else {
                return None;
            };
            if sources.contains(&CallableResultSource::EnvironmentStorage) {
                return None;
            }
            Some(Evidence {
                ty: ret,
                sources: result_borrowing.clone(),
                value_sources: result_borrowing.clone(),
                environment: None,
                storage: HashSet::new(),
                isolated: false,
                factory: None,
            })
        })?;
        if evidence.ty != ret {
            return None;
        }
        evidence.environment = Some(destination.clone());
        Some(evidence)
    }

    pub(in crate::mir::borrowck) fn instantiate(
        &self,
        body: &MirFunction,
        tc: &TypeContext,
        contract: &MirBackendContract,
        func: &Operand,
        args: &[Operand],
        destination: &Place,
        purpose: crate::mir::borrowck::OriginPurpose,
    ) -> Option<(Vec<Operand>, OriginSet)> {
        let (Operand::Copy(callee) | Operand::Move(callee)) = func else {
            return None;
        };
        if !destination.projection.is_empty() {
            return None;
        }
        let evidence = self
            .calls
            .get(&(body.id.clone(), callee.clone(), destination.local))?;
        if contract.place_type_id(tc, body, callee) != Some(evidence.ty)
            || !tc.type_id_tree_is_valid(evidence.ty)
        {
            return None;
        }
        let Ty::Function { params, ret, .. } = tc.try_ty(evidence.ty)? else {
            return None;
        };
        if params.len() != args.len() || contract.place_type_id(tc, body, destination) != Some(*ret)
        {
            return None;
        }
        for (argument, parameter) in args.iter().zip(params) {
            let actual = match argument {
                Operand::Copy(place) | Operand::Move(place) if place.projection.is_empty() => {
                    contract.place_type_id(tc, body, place)
                }
                Operand::Constant(Constant::Int(_)) => tc.id_for_type(&crate::types::Type::I64),
                Operand::Constant(Constant::Float(_)) => tc.id_for_type(&crate::types::Type::F64),
                Operand::Constant(Constant::Bool(_)) => tc.id_for_type(&crate::types::Type::Bool),
                Operand::Constant(Constant::Char(_)) => tc.id_for_type(&crate::types::Type::Char),
                Operand::Constant(Constant::String(_)) => tc.id_for_type(&crate::types::Type::Str),
                Operand::Constant(Constant::Unit) => tc.id_for_type(&crate::types::Type::Unit),
                _ => None,
            };
            if actual != Some(*parameter) {
                return None;
            }
        }
        let selected = match purpose {
            crate::mir::borrowck::OriginPurpose::Lifetime => &evidence.sources,
            crate::mir::borrowck::OriginPurpose::ValueLoans => &evidence.value_sources,
        };
        let CallableResultBorrowing::Sources(sources) = selected else {
            return None;
        };
        let origins = sources
            .iter()
            .map(|source| {
                use CallableResultSource as S;
                Some(match *source {
                    S::ArgumentValue(index) => ReferenceOrigin::Param(Local(index as usize + 2)),
                    S::ArgumentContents(index) => {
                        ReferenceOrigin::ParamContents(Local(index as usize + 2))
                    }
                    S::ArgumentBorrowedContents(index) => {
                        ReferenceOrigin::ParamBorrowedContents(Local(index as usize + 2))
                    }
                    S::EnvironmentValue => ReferenceOrigin::Param(Local(1)),
                    S::EnvironmentContents => ReferenceOrigin::ParamContents(Local(1)),
                    S::EnvironmentBorrowedContents => {
                        ReferenceOrigin::ParamBorrowedContents(Local(1))
                    }
                    S::Static => ReferenceOrigin::Static,
                    S::Unknown => ReferenceOrigin::UnknownExternal,
                    S::EnvironmentStorage => return None,
                })
            })
            .collect::<Option<OriginSet>>()?;
        let environment = evidence.environment.clone()?;
        if contract.place_type_id(tc, body, &environment) != Some(evidence.ty) {
            return None;
        }
        // A checked shared receiver alias points at this actual environment value.
        // Its temporary &Fn slot/storage loan is not an environment-carried loan.
        Some((
            std::iter::once(Operand::Copy(environment))
                .chain(args.iter().cloned())
                .collect(),
            origins,
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Evidence {
    ty: TypeId,
    sources: CallableResultBorrowing,
    // The obligation union is not a loan-role envelope: raw lifetimes may keep
    // borrowed storage while the measured value flow transports contents only.
    value_sources: CallableResultBorrowing,
    environment: Option<Place>,
    storage: HashSet<Local>,
    isolated: bool,
    // A measured return-value recipe uses the producer body, never a replacement
    // executable target or an effect certificate.
    factory: Option<MirFunctionId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State(Option<HashMap<Place, Evidence>>);

impl Lattice for State {
    fn join(&mut self, other: &Self) -> bool {
        let Some(incoming) = &other.0 else {
            return false;
        };
        let old = self.clone();
        if let Some(current) = &mut self.0 {
            current.retain(|local, evidence| {
                let Some(other) = incoming.get(local).filter(|other| {
                    other.ty == evidence.ty && other.environment == evidence.environment
                }) else {
                    return false;
                };
                evidence.sources = evidence.sources.merge(&other.sources, true);
                if evidence.factory != other.factory {
                    evidence.factory = None;
                }
                evidence.value_sources = evidence.value_sources.merge(&other.value_sources, true);
                evidence.storage.extend(other.storage.iter().copied());
                evidence.isolated &= other.isolated;
                true
            });
        } else {
            self.0 = Some(incoming.clone());
        }
        *self != old
    }
}

struct Bindings<'a> {
    program: &'a MirProgram,
    owner: &'a MirFunction,
    lifetime: &'a HashMap<MirFunctionId, OriginSet>,
    values: &'a HashMap<MirFunctionId, OriginSet>,
    backend_valid: bool,
    calls: Option<&'a crate::mir::borrowck::return_sources::NativeReturnSources>,
}

impl Bindings<'_> {
    fn local_invocation_alias(&self, alias: Local) -> bool {
        use crate::mir::borrowck::accesses::{classify_statement, classify_terminator};
        if !self
            .owner
            .local_decls
            .get(alias.0)
            .is_some_and(|decl| decl.source == crate::mir::LocalSource::Temporary)
        {
            return false;
        }
        let mut definitions = 0;
        let mut invocations = 0;
        for block in &self.owner.basic_blocks {
            for statement in &block.statements {
                if !classify_statement(statement, &self.program.type_context)
                    .iter()
                    .any(|event| event.place.local == alias)
                {
                    continue;
                }
                match &statement.kind {
                    StatementKind::Assign(destination, Rvalue::Ref(_, _))
                        if destination.local == alias && destination.projection.is_empty() =>
                    {
                        definitions += 1
                    }
                    StatementKind::StorageLive(local) | StatementKind::StorageDead(local)
                        if *local == alias => {}
                    _ => return false,
                }
            }
            if let Some(term) = &block.terminator {
                if !classify_terminator(term)
                    .iter()
                    .any(|event| event.place.local == alias)
                {
                    continue;
                }
                match term {
                    Terminator::Call { func: Operand::Copy(place) | Operand::Move(place), args, destination, .. }
                        if place.local == alias && !place.projection.is_empty() && place.projection.iter().all(|projection| *projection == Projection::Deref)
                            && destination.local != alias && !args.iter().any(|arg| matches!(arg, Operand::Copy(place) | Operand::Move(place) if place.local == alias)) => invocations += 1,
                    _ => return false,
                }
            }
        }
        definitions == 1 && invocations == 1
    }

    /// Prove only that a single allocation's callable storage cannot be rebound
    /// by another call. Body/result authority is still independently measured.
    fn isolated_storage(&self, root: Local) -> bool {
        use crate::mir::borrowck::accesses::{classify_statement, classify_terminator};
        if root == Local(0) {
            return false;
        }
        let Some(root_ty) = self.owner.local_decls.get(root.0).map(|decl| decl.ty) else {
            return false;
        };
        let mut aliases = HashSet::from([root]);
        for statement in self
            .owner
            .basic_blocks
            .iter()
            .flat_map(|block| &block.statements)
        {
            if let StatementKind::Assign(destination, Rvalue::Ref(Mutability::Not, source)) =
                &statement.kind
            {
                if source.local == root
                    && source.projection.is_empty()
                    && destination.projection.is_empty()
                    && destination.local != Local(0)
                    && self
                        .place_type(destination)
                        .and_then(|ty| self.program.type_context.try_ty(ty))
                        == Some(&Ty::Reference {
                            mutable: false,
                            inner: root_ty,
                        })
                {
                    aliases.insert(destination.local);
                }
            }
        }
        let mut definitions = HashMap::<Local, usize>::new();
        for block in &self.owner.basic_blocks {
            for statement in &block.statements {
                if !classify_statement(statement, &self.program.type_context)
                    .iter()
                    .any(|event| aliases.contains(&event.place.local))
                {
                    continue;
                }
                match &statement.kind {
                    StatementKind::StorageDead(_) => {}
                    StatementKind::Assign(destination, value)
                        if destination.projection.is_empty() =>
                    {
                        let allowed = if destination.local == root {
                            matches!(
                                value,
                                Rvalue::Closure(_)
                                    | Rvalue::Use(Operand::Constant(Constant::Callable(_)))
                            )
                        } else if aliases.contains(&destination.local) {
                            matches!(value, Rvalue::Ref(Mutability::Not, source)
                                if source.local == root && source.projection.is_empty())
                        } else {
                            false
                        };
                        if !allowed {
                            return false;
                        }
                        *definitions.entry(destination.local).or_default() += 1;
                    }
                    _ => return false,
                }
            }
            if let Some(term) = &block.terminator {
                if !classify_terminator(term)
                    .iter()
                    .any(|event| aliases.contains(&event.place.local))
                {
                    continue;
                }
                match term {
                    Terminator::Call {
                        func,
                        args,
                        destination,
                        ..
                    } => {
                        if aliases.contains(&destination.local) || args.iter().any(|arg|
                            matches!(arg, Operand::Copy(place) | Operand::Move(place) if aliases.contains(&place.local))) {
                            return false;
                        }
                        if !matches!(func, Operand::Copy(place) | Operand::Move(place)
                            if (place.local == root && place.projection.is_empty())
                                || (aliases.contains(&place.local) && place.local != root && place.projection == [Projection::Deref]))
                        {
                            return false;
                        }
                    }
                    Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. }
                        if place.projection.is_empty() => {}
                    _ => return false,
                }
            }
        }
        aliases.iter().all(|local| {
            if *local == root
                && matches!(
                    self.owner.local_decls[root.0].source,
                    crate::mir::LocalSource::Argument | crate::mir::LocalSource::ClosureCapture
                )
            {
                !definitions.contains_key(local)
            } else {
                definitions.get(local) == Some(&1)
            }
        })
    }

    fn invalidate(facts: &mut HashMap<Place, Evidence>, local: Local) {
        facts.retain(|key, value| key.local != local && !value.storage.contains(&local));
    }

    fn direct_field(&self, place: &Place) -> bool {
        !place.projection.is_empty()
            && place
                .projection
                .iter()
                .all(|projection| matches!(projection, Projection::Field { .. }))
            && self.place_type(place).is_some()
    }

    fn invalidate_place(facts: &mut HashMap<Place, Evidence>, place: &Place) {
        let written =
            crate::mir::borrowck::ReferenceEscapeAnalysis::normalized_place(place.clone());
        facts.retain(|key, evidence| {
            !crate::mir::borrowck::conflicts::places_conflict(
                &crate::mir::borrowck::ReferenceEscapeAnalysis::normalized_place(key.clone()),
                &written,
            ) && !evidence.storage.contains(&place.local)
        });
    }

    fn place_evidence(&self, state: &State, place: &Place) -> Option<Evidence> {
        let facts = state.0.as_ref()?;
        let normalized =
            crate::mir::borrowck::ReferenceEscapeAnalysis::normalized_place(place.clone());
        if let Some(evidence) = facts.get(&normalized) {
            return (self.place_type(place) == Some(evidence.ty)).then(|| evidence.clone());
        }
        let mut evidence = facts
            .get(&Place {
                local: place.local,
                projection: Vec::new(),
            })?
            .clone();
        for projection in &place.projection {
            let Ty::Reference { inner, .. } = self.program.type_context.try_ty(evidence.ty)? else {
                return None;
            };
            if *projection != Projection::Deref {
                return None;
            }
            evidence.ty = *inner;
        }
        (self.place_type(place) == Some(evidence.ty)).then_some(evidence)
    }
    fn obligation(&self, ty: TypeId) -> Option<&CallableResultBorrowing> {
        let tc = &self.program.type_context;
        let Ty::Function {
            result_borrowing, ..
        } = tc.try_ty(ty)?
        else {
            return None;
        };
        (*result_borrowing != CallableResultBorrowing::Unknown).then_some(result_borrowing)
    }

    fn place_type(&self, place: &Place) -> Option<TypeId> {
        self.program
            .backend_contract
            .place_type_id(&self.program.type_context, self.owner, place)
            .or_else(|| {
                // Keep malformed root declarations visible to obligation diagnostics;
                // native()/check() still require a valid type tree for any evidence.
                place
                    .projection
                    .is_empty()
                    .then(|| {
                        self.owner
                            .local_decls
                            .get(place.local.0)
                            .map(|local| local.ty)
                    })
                    .flatten()
            })
    }

    fn call_parameters(&self, func: &Operand) -> Option<Vec<TypeId>> {
        match func {
            Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) => {
                self.program.backend_contract.callable(key).map(|header| {
                    header
                        .signature
                        .params
                        .iter()
                        .map(|parameter| parameter.semantic_ty)
                        .collect()
                })
            }
            Operand::Copy(place) | Operand::Move(place) => {
                let tc = &self.program.type_context;
                let mut ty = self.place_type(place)?;
                if !tc.type_id_tree_is_valid(ty) {
                    return None;
                }
                loop {
                    match tc.try_ty(ty)? {
                        Ty::Function { params, .. } => return Some(params.clone()),
                        Ty::Reference { inner, .. } => ty = *inner,
                        _ => return None,
                    }
                }
            }
            _ => None,
        }
    }

    fn native(&self, callable: &MirCallable, ty: TypeId) -> Option<Evidence> {
        if !self.backend_valid || !self.program.type_context.type_id_tree_is_valid(ty) {
            return None;
        }
        let tc = &self.program.type_context;
        let Ty::Function { params, ret, .. } = tc.try_ty(ty)? else {
            return None;
        };
        let MirCallable::Resolved(key) = callable;
        let contract = &self.program.backend_contract;
        let header = contract.callable(key)?;
        if &header.key != key
            || header.signature.ret.semantic_ty != *ret
            || header.signature.ret.abi_ty != *ret
            || header.signature.params.len() != params.len()
            || !params
                .iter()
                .zip(&header.signature.params)
                .all(|(ty, parameter)| *ty == parameter.semantic_ty)
        {
            return None;
        }
        let id = callable.function_id()?;
        let (sources, value_sources) = if let Some(body) = self.program.functions.get(&id) {
            if contract.function_bodies.get(&id) != Some(key)
                || !matches!(&header.kind, MirCallableKind::LocalBody { function_id } if function_id == &id)
                || !body.closure_captures.is_empty()
            {
                return None;
            }
            (
                body_sources_with_calls(
                    self.program,
                    body,
                    self.lifetime,
                    self.values,
                    self.calls,
                )?,
                body_sources_with_calls(self.program, body, self.values, self.values, self.calls)?,
            )
        } else {
            // Reuse validated, signature-bound native producer evidence. A
            // portable function row alone cannot certify an executable value.
            let origins = contract.native_return_origins.get(key)?;
            if origins.params != *params
                || origins.ret != *ret
                || !(self.lifetime.contains_key(&id)
                    || self.calls.is_some_and(|table| {
                        table
                            .scalar_dependencies(OriginPurpose::Lifetime, &id)
                            .is_some()
                    }))
                || !(self.values.contains_key(&id)
                    || self.calls.is_some_and(|table| {
                        table
                            .scalar_dependencies(OriginPurpose::ValueLoans, &id)
                            .is_some()
                    }))
            {
                return None;
            }
            use crate::hir::borrowing::NativeReturnSource as N;
            use CallableResultSource as S;
            let sources = CallableResultBorrowing::Sources(Box::new(
                origins
                    .sources
                    .iter()
                    .map(|source| match *source {
                        N::ArgumentValue(index) => S::ArgumentValue(index),
                        N::ArgumentContents(index) => S::ArgumentContents(index),
                        N::ArgumentBorrowedContents(index) => S::ArgumentBorrowedContents(index),
                        N::Static => S::Static,
                        N::UnknownExternal => S::Unknown,
                    })
                    .collect(),
            ));
            (sources.clone(), sources)
        };
        Some(Evidence {
            ty,
            sources,
            value_sources,
            environment: None,
            storage: HashSet::new(),
            isolated: false,
            factory: self.program.functions.contains_key(&id).then_some(id),
        })
    }

    fn operand(&self, state: &State, operand: &Operand, expected: TypeId) -> Option<Evidence> {
        match operand {
            Operand::Constant(Constant::Callable(callable)) => self.native(callable, expected),
            Operand::Copy(place) | Operand::Move(place) => {
                let mut evidence = self.place_evidence(state, place)?;
                if evidence.ty == expected {
                    return Some(evidence);
                }
                // A canonical input may abstract over the actual environment;
                // keep its measured sources/root/storage, never manufacture
                // capture or independence evidence from that abstraction.
                let actual = self.program.type_context.type_for(evidence.ty);
                let required = self.program.type_context.type_for(expected);
                if !crate::type_services::callable::input_shape_matches(&required, &actual) {
                    return None;
                }
                evidence.ty = expected;
                Some(evidence)
            }
            _ => None,
        }
    }

    fn assigned(&self, state: &State, destination: &Place, value: &Rvalue) -> Option<Evidence> {
        let ty = self.place_type(destination)?;
        match value {
            Rvalue::Closure(closure) => {
                let body = allocation_body(self.program, self.owner, destination, closure)?;
                Some(Evidence {
                    ty,
                    sources: body_sources_with_calls(
                        self.program,
                        body,
                        self.lifetime,
                        self.values,
                        self.calls,
                    )?,
                    value_sources: body_sources_with_calls(
                        self.program,
                        body,
                        self.values,
                        self.values,
                        self.calls,
                    )?,
                    environment: None,
                    storage: HashSet::new(),
                    isolated: false,
                    factory: Some(body.id.clone()),
                })
            }
            Rvalue::Use(operand) => self.operand(state, operand, ty),
            Rvalue::Ref(mutability, source) if destination.projection.is_empty() => {
                let mut evidence = self.place_evidence(state, source)?;
                if self.program.type_context.try_ty(ty)
                    != Some(&Ty::Reference {
                        mutable: *mutability == Mutability::Mut,
                        inner: evidence.ty,
                    })
                {
                    return None;
                }
                evidence.storage.insert(source.local);
                evidence.ty = ty;
                Some(evidence)
            }
            _ => None,
        }
    }

    fn check(
        &self,
        ty: TypeId,
        actual: Option<Evidence>,
        span: Option<crate::lexer::Span>,
        diagnostics: &mut Diagnostics,
    ) {
        let Some(required) = self.obligation(ty) else {
            return;
        };
        let actual = actual.map(|evidence| evidence.sources).unwrap_or_default();
        if !self.program.type_context.type_id_tree_is_valid(ty) || !required.accepts(&actual) {
            diagnostics.push(mir_diagnostic(
                "Callable result borrowing contract is not satisfied".into(),
                span,
            ));
        }
    }

    fn check_input(
        &self,
        ty: TypeId,
        actual: Option<Evidence>,
        span: Option<crate::lexer::Span>,
        diagnostics: &mut Diagnostics,
    ) {
        // An entry hypothesis uses one canonical envelope for both consumers.
        // Require independent measured roles to agree before discharging it;
        // ordinary binding declarations still preserve separate envelopes.
        if self.obligation(ty).is_some()
            && actual
                .as_ref()
                .is_some_and(|evidence| evidence.sources != evidence.value_sources)
        {
            diagnostics.push(mir_diagnostic(
                "Callable result borrowing contract is not satisfied".into(),
                span,
            ));
            return;
        }
        self.check(ty, actual, span, diagnostics);
    }

    fn check_generic_input(
        &self,
        required: &crate::mir::backend_contract::MirGenericCallableInput,
        argument: Option<&Operand>,
        state: &State,
        span: Option<crate::lexer::Span>,
        diagnostics: &mut Diagnostics,
    ) {
        let requirements = crate::mir::backend_contract::callable_payload_requirements(
            &self.program.type_context,
            &self.program.backend_contract,
            required,
        );
        let actual = argument.map(|argument| self.value(state, argument, required.subject));
        let accepted = !requirements.is_empty()
            && requirements.iter().all(|required| {
                actual.as_ref().is_some_and(|actual| {
                    actual
                        .fields
                        .get(&required.projection)
                        .is_some_and(|evidence| {
                            evidence.ty == required.ty
                                && evidence.sources == evidence.value_sources
                                && required.borrowing.accepts(&evidence.sources)
                        })
                })
            });
        if !accepted {
            diagnostics.push(mir_diagnostic(
                "Callable result borrowing contract is not satisfied".into(),
                span,
            ));
        }
    }
}

impl Analysis for Bindings<'_> {
    type Domain = State;

    fn initial_state(&self, _: &MirFunction) -> State {
        // Only definition-owned conditional requirements enter the body. The
        // binding validator independently discharges them at every call; an
        // ordinary parameter header or a measured caller cannot add entries.
        let mut inputs = self
            .calls
            .and_then(|table| table.callable_results.inputs.get(&self.owner.id))
            .cloned()
            .unwrap_or_default();
        for (local, evidence) in &mut inputs {
            evidence.isolated = self.isolated_storage(local.local);
        }
        State(Some(inputs))
    }

    fn bottom_state(&self, _: &MirFunction) -> State {
        State::default()
    }

    fn apply_statement(&self, state: &mut State, statement: &StatementData) {
        let structured = match &statement.kind {
            StatementKind::Assign(destination, Rvalue::Aggregate(kind, operands)) => operands
                .iter()
                .enumerate()
                .flat_map(|(index, operand)| {
                    let Some(field) = crate::mir::borrowck::checked_aggregate_field(
                        self.owner,
                        &self.program.type_context,
                        &self.program.backend_contract,
                        destination,
                        kind,
                        index,
                    ) else {
                        return Vec::new();
                    };
                    self.forwarded_operand(state, operand, &field)
                })
                .collect::<Vec<_>>(),
            StatementKind::Assign(destination, Rvalue::Use(operand)) => {
                self.forwarded_operand(state, operand, destination)
            }
            _ => Vec::new(),
        };
        let mut candidate = match &statement.kind {
            StatementKind::Assign(destination, value) => self.assigned(state, destination, value),
            _ => None,
        };
        let borrowed_code = match (&statement.kind, &candidate) {
            (StatementKind::Assign(destination, Rvalue::Ref(_, source)), Some(_))
                if self.local_invocation_alias(destination.local) =>
            {
                self.place_evidence(state, source)
                    .filter(|evidence| {
                        matches!(
                            self.program.type_context.try_ty(evidence.ty),
                            Some(Ty::Function { .. })
                        )
                    })
                    .map(|evidence| {
                        (
                            crate::mir::borrowck::ReferenceEscapeAnalysis::normalized_place(
                                source.clone(),
                            ),
                            evidence,
                        )
                    })
            }
            _ => None,
        };
        let Some(facts) = &mut state.0 else {
            return;
        };
        match &statement.kind {
            StatementKind::Assign(destination, value) => {
                // Owned field writes only replace overlapping value evidence;
                // alias/index writes still supply no mutation or layout authority.
                if (!destination.projection.is_empty() && !self.direct_field(destination))
                    || matches!(value, Rvalue::Cast(_, _))
                {
                    facts.retain(|_, evidence| evidence.isolated);
                    return;
                }
                if let Rvalue::Ref(_, source) = value {
                    if !source.projection.is_empty() {
                        facts.retain(|_, evidence| evidence.isolated);
                    } else {
                        // Taking either loan of an owned root exposes that root,
                        // not unrelated callable storage. Subsequent aliased
                        // writes/calls still require their own effect authority.
                        Self::invalidate(facts, source.local);
                        Self::invalidate(facts, destination.local);
                    }
                    if let Some(evidence) = candidate {
                        facts.insert(
                            crate::mir::borrowck::ReferenceEscapeAnalysis::normalized_place(
                                destination.clone(),
                            ),
                            evidence,
                        );
                    }
                    if let Some((place, evidence)) = borrowed_code {
                        facts.insert(place, evidence);
                    }
                    return;
                }
                if let Rvalue::Use(Operand::Move(place)) = value {
                    if self.direct_field(place) {
                        Self::invalidate_place(facts, place);
                    } else {
                        Self::invalidate(facts, place.local);
                    }
                }
                if let Rvalue::Closure(closure) = value {
                    for capture in &closure.captures {
                        Self::invalidate(facts, capture.local);
                    }
                }
                Self::invalidate_place(facts, destination);
                if let Some(evidence) = &mut candidate {
                    if matches!(
                        self.program.type_context.try_ty(evidence.ty),
                        Some(Ty::Function { .. })
                    ) {
                        evidence.environment = Some(destination.clone());
                        evidence.isolated = self.isolated_storage(destination.local);
                    }
                }
                if let Some(evidence) = candidate {
                    facts.insert(
                        crate::mir::borrowck::ReferenceEscapeAnalysis::normalized_place(
                            destination.clone(),
                        ),
                        evidence,
                    );
                }
                for (place, mut evidence) in structured {
                    evidence.environment = Some(place.clone());
                    evidence.isolated = self.isolated_storage(place.local);
                    facts.insert(
                        crate::mir::borrowck::ReferenceEscapeAnalysis::normalized_place(place),
                        evidence,
                    );
                }
            }
            StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                Self::invalidate(facts, *local);
            }
            StatementKind::Assert(_) => {}
        }
    }

    fn apply_terminator(&self, state: &mut State, term: &Terminator) {
        let invoked_code = match term {
            Terminator::Call { func: Operand::Copy(place) | Operand::Move(place), args, .. } => self.place_evidence(state, place).and_then(|mut evidence| {
                let environment = evidence.environment.clone()?;
                let ty = self.place_type(&environment)?;
                if !matches!(self.program.type_context.try_ty(ty), Some(Ty::Function { .. }))
                    || args.iter().any(|argument| matches!(argument, Operand::Copy(place) | Operand::Move(place) if place.local == environment.local)) { return None; }
                // Invocation preserves the callable's code, not empty effects:
                // mutable environment loans remain in the ordinary analyses.
                evidence.ty = ty; evidence.storage.clear(); evidence.isolated = false;
                Some((crate::mir::borrowck::ReferenceEscapeAnalysis::normalized_place(environment), evidence))
            }),
            _ => None,
        };
        let forwarded = match term {
            Terminator::Call {
                func,
                args,
                destination,
                ..
            } => self.forwarded_call(state, func, args, destination),
            _ => Vec::new(),
        };
        let returned = match term {
            Terminator::Call {
                func,
                args,
                destination,
                ..
            } => self.calls.and_then(|table| {
                table.callable_results.returned_value(
                    self.owner,
                    &self.program.type_context,
                    &self.program.backend_contract,
                    func,
                    args,
                    destination,
                )
            }),
            _ => None,
        };
        let Some(facts) = &mut state.0 else {
            return;
        };
        match term {
            // Result obligations never supply mutation or retention authority.
            Terminator::Call { destination, .. } => {
                let Terminator::Call { func, .. } = term else {
                    unreachable!()
                };
                facts.retain(|_, evidence| evidence.isolated);
                if let Operand::Copy(place) | Operand::Move(place) = func {
                    Self::invalidate(facts, place.local);
                }
                Self::invalidate(facts, destination.local);
                if let Some((place, evidence)) = invoked_code {
                    if place.local != destination.local {
                        facts.insert(place, evidence);
                    }
                }
                if let Some(evidence) = returned {
                    facts.insert(destination.clone(), evidence);
                }
                for (place, mut evidence) in forwarded {
                    evidence.environment = Some(place.clone());
                    evidence.isolated = self.isolated_storage(place.local);
                    facts.insert(place, evidence);
                }
            }
            Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } => {
                Self::invalidate(facts, place.local);
            }
            _ => {}
        }
    }
}

pub(super) fn validate(
    program: &MirProgram,
    lifetime: &HashMap<MirFunctionId, OriginSet>,
    values: &HashMap<MirFunctionId, OriginSet>,
    calls: Option<&crate::mir::borrowck::return_sources::NativeReturnSources>,
    diagnostics: &mut Diagnostics,
) {
    let backend_valid = crate::mir::backend_contract::validate_backend_contract_against_mir(
        &program.backend_contract,
        program.functions.values(),
    )
    .is_empty();
    for (_, owner) in program.functions() {
        let analysis = Bindings {
            program,
            owner,
            lifetime,
            values,
            backend_valid,
            calls,
        };
        let has_obligations = owner
            .local_decls
            .iter()
            .any(|local| analysis.obligation(local.ty).is_some())
            || owner
                .basic_blocks
                .iter()
                .any(|block| match &block.terminator {
                    Some(Terminator::Call { func, .. }) => {
                        analysis.call_parameters(func).is_some_and(|params| {
                            params.iter().any(|ty| analysis.obligation(*ty).is_some())
                        })
                    }
                    _ => false,
                });
        let has_obligations = has_obligations || owner.basic_blocks.iter().any(|block| matches!(
            &block.terminator,
            Some(Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))), .. })
                if program.backend_contract.generic_callable_input_obligations.contains_key(key)
        ));
        let has_obligations = has_obligations
            || calls.is_some_and(|table| !table.callable_results.adapter_inputs.is_empty());
        if !has_obligations {
            continue;
        }
        let results = run_fixpoint(&analysis, owner);
        for (index, block) in owner.basic_blocks.iter().enumerate() {
            if !results.reachable[index] {
                continue;
            }
            let mut state = results.entry_sets[index].clone();
            for statement in &block.statements {
                if let StatementKind::Assign(destination, value) = &statement.kind {
                    // Closure allocations are checked by the parent validator.
                    if !matches!(value, Rvalue::Closure(_)) {
                        if let Some(ty) = analysis.place_type(destination) {
                            analysis.check(
                                ty,
                                analysis.assigned(&state, destination, value),
                                statement.span.clone(),
                                diagnostics,
                            );
                        }
                    }
                }
                analysis.apply_statement(&mut state, statement);
            }
            if let Some(Terminator::Call {
                func,
                args,
                destination,
                span,
                ..
            }) = &block.terminator
            {
                if let Some(params) = analysis
                    .call_parameters(func)
                    .filter(|params| params.len() == args.len())
                {
                    for (ty, argument) in params.into_iter().zip(args) {
                        analysis.check_input(
                            ty,
                            analysis.operand(&state, argument, ty),
                            span.clone(),
                            diagnostics,
                        );
                    }
                }
                if let Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) = func {
                    if let Some(requirements) = program
                        .backend_contract
                        .generic_callable_input_obligations
                        .get(key)
                    {
                        for (index, required) in requirements {
                            analysis.check_generic_input(
                                required,
                                args.get(*index),
                                &state,
                                span.clone(),
                                diagnostics,
                            );
                        }
                    }
                }
                if let Some(table) = calls {
                    let target = match func {
                        Operand::Copy(place) | Operand::Move(place) => analysis
                            .place_evidence(&state, place)
                            .and_then(|evidence| evidence.factory)
                            .or_else(|| {
                                table
                                    .witnessed_callable(owner, func, destination)
                                    .and_then(MirCallable::function_id)
                            }),
                        Operand::Constant(Constant::Callable(callable)) => callable.function_id(),
                        _ => None,
                    };
                    if let Some(requirements) = target
                        .as_ref()
                        .and_then(|target| table.callable_results.adapter_inputs.get(target))
                    {
                        for (index, requirement) in requirements {
                            analysis.check_generic_input(
                                requirement,
                                args.get(*index),
                                &state,
                                span.clone(),
                                diagnostics,
                            );
                        }
                    }
                }
                // Reuse actual construction evidence; never manufacture a
                // callable promise from the destination's type header.
                if let Some(ty) = analysis.place_type(destination) {
                    let evidence = calls.and_then(|table| {
                        table.callable_results.returned_value(
                            owner,
                            &program.type_context,
                            &program.backend_contract,
                            func,
                            args,
                            destination,
                        )
                    });
                    analysis.check(ty, evidence, span.clone(), diagnostics);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
