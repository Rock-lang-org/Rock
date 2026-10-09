//! Measured value transport, separate from invocation and effect authority.
//! A recipe forwards existing producer facts; an argument/header is never a fact.

use super::*;
use crate::mir::borrowck::ReferenceEscapeAnalysis;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Recipe {
    Argument(usize, TypeId),
    Opaque(TypeId),
    Callable(MirCallable, TypeId),
    Measured(Evidence),
    Project(Box<Recipe>, Vec<Projection>, TypeId),
    Reference(Box<Recipe>, TypeId),
    Aggregate(
        TypeId,
        Option<crate::ids::VariantId>,
        Vec<(Vec<Projection>, Recipe)>,
    ),
    Call(
        Option<MirCallable>,
        Option<Box<Recipe>>,
        Vec<Recipe>,
        TypeId,
    ),
    Alternatives(TypeId, Vec<Recipe>),
    /// Exact owned-field writes override value transport, not the producer
    /// authority of the remainder of the record.
    Overlay(Box<Recipe>, Vec<(Vec<Projection>, Recipe)>, TypeId),
}

impl Recipe {
    fn ty(&self) -> TypeId {
        match self {
            Self::Argument(_, ty)
            | Self::Opaque(ty)
            | Self::Callable(_, ty)
            | Self::Project(_, _, ty)
            | Self::Reference(_, ty)
            | Self::Aggregate(ty, _, _)
            | Self::Call(_, _, _, ty)
            | Self::Alternatives(ty, _)
            | Self::Overlay(_, _, ty) => *ty,
            Self::Measured(evidence) => evidence.ty,
        }
    }

    fn callable(&self, contract: &MirBackendContract) -> Option<MirCallable> {
        match self {
            Self::Callable(callable, _) => Some(callable.clone()),
            Self::Reference(value, _) => value.callable(contract),
            Self::Measured(evidence) => evidence
                .factory
                .as_ref()
                .and_then(|id| contract.function_bodies.get(id))
                .map(|key| MirCallable::Resolved(key.clone())),
            _ => None,
        }
    }

    fn overwrite(self, path: Vec<Projection>, value: Recipe) -> Self {
        let ty = self.ty();
        let (base, mut fields) = match self {
            Self::Overlay(base, fields, _) => (base, fields),
            base => (Box::new(base), Vec::new()),
        };
        fields.retain(|(field, _)| !field.starts_with(&path));
        fields.push((path, value));
        // Parent overrides precede their children; ordering is canonical so
        // the must-evidence lattice cannot depend on HashMap traversal order.
        fields.sort_by_key(|(path, _)| field_indices(path));
        Self::Overlay(base, fields, ty)
    }

    fn common(&self, other: &Self) -> Option<Self> {
        if self == other {
            return Some(self.clone());
        }
        let (base, fields) = match self {
            Self::Overlay(base, fields, _) => (base.as_ref(), fields.as_slice()),
            base => (base, &[][..]),
        };
        let (other_base, other_fields) = match other {
            Self::Overlay(base, fields, _) => (base.as_ref(), fields.as_slice()),
            base => (base, &[][..]),
        };
        if base != other_base || self.ty() != other.ty() {
            return None;
        }
        let mut result = base.clone();
        let mut paths = fields
            .iter()
            .chain(other_fields)
            .map(|(path, _)| path)
            .collect::<Vec<_>>();
        paths.sort_by_key(|path| field_indices(path));
        paths.dedup();
        for path in paths {
            let lhs = fields.iter().find(|(field, _)| field == path);
            let rhs = other_fields.iter().find(|(field, _)| field == path);
            let common = match (lhs, rhs) {
                (Some((_, lhs)), Some((_, rhs))) => {
                    lhs.common(rhs).unwrap_or(Recipe::Opaque(lhs.ty()))
                }
                (Some((_, value)), None) | (None, Some((_, value))) => Recipe::Opaque(value.ty()),
                (None, None) => unreachable!("joined path comes from an incoming write"),
            };
            result = result.overwrite(path.clone(), common);
        }
        Some(result)
    }
}

fn field_indices(path: &[Projection]) -> Vec<usize> {
    path.iter()
        .filter_map(|projection| match projection {
            Projection::Field { index, .. } => Some(*index),
            _ => None,
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Recipes(Option<HashMap<Place, Recipe>>);

impl Lattice for Recipes {
    fn join(&mut self, other: &Self) -> bool {
        let Some(incoming) = &other.0 else {
            return false;
        };
        let old = self.clone();
        if let Some(facts) = &mut self.0 {
            facts.retain(|place, recipe| {
                let Some(common) = incoming.get(place).and_then(|other| recipe.common(other))
                else {
                    return false;
                };
                *recipe = common;
                true
            });
        } else {
            self.0 = Some(incoming.clone());
        }
        *self != old
    }
}

struct Measure<'a> {
    bindings: Bindings<'a>,
}

impl Measure<'_> {
    fn invalidate_place(&self, facts: &mut HashMap<Place, Recipe>, place: &Place) {
        if !self.bindings.direct_field(place) {
            facts.retain(|key, _| key.local != place.local);
            return;
        }
        let root = Place {
            local: place.local,
            projection: vec![],
        };
        if let Some(value) = facts.remove(&root) {
            let normalized = ReferenceEscapeAnalysis::normalized_place(place.clone());
            let Some(ty) = self.bindings.place_type(place) else {
                return;
            };
            facts.insert(
                root,
                value.overwrite(normalized.projection, Recipe::Opaque(ty)),
            );
        }
    }
    fn place(&self, state: &Recipes, place: &Place) -> Option<Recipe> {
        let ty = self.bindings.place_type(place)?;
        let normalized =
            crate::mir::borrowck::ReferenceEscapeAnalysis::normalized_place(place.clone());
        let place = &normalized;
        let facts = state.0.as_ref()?;
        if let Some(value) = facts.get(place) {
            return Some(value.clone());
        }
        let (prefix, value) = facts
            .iter()
            .filter(|(prefix, _)| {
                prefix.local == place.local && place.projection.starts_with(&prefix.projection)
            })
            .max_by_key(|(prefix, _)| prefix.projection.len())?;
        let suffix = place.projection[prefix.projection.len()..].to_vec();
        if let (Recipe::Reference(value, _), [Projection::Deref]) = (value, suffix.as_slice()) {
            return (value.ty() == ty).then(|| (**value).clone());
        }
        Some(Recipe::Project(Box::new(value.clone()), suffix, ty))
    }

    fn operand(&self, state: &Recipes, operand: &Operand, ty: TypeId) -> Recipe {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                self.place(state, place).unwrap_or(Recipe::Opaque(ty))
            }
            Operand::Constant(Constant::Callable(callable)) => {
                Recipe::Callable(callable.clone(), ty)
            }
            _ => Recipe::Opaque(ty),
        }
    }
}

impl Analysis for Measure<'_> {
    type Domain = Recipes;

    fn initial_state(&self, _: &MirFunction) -> Recipes {
        Recipes(Some(
            (0..self.bindings.owner.arg_count)
                .map(|index| {
                    let local = Local(index + 1);
                    let ty = self.bindings.owner.local_decls[local.0].ty;
                    let value = if matches!(
                        self.bindings.program.type_context.try_ty(ty),
                        Some(Ty::Reference { mutable: true, .. } | Ty::Pointer(_))
                    ) {
                        Recipe::Opaque(ty)
                    } else {
                        Recipe::Argument(index, ty)
                    };
                    (
                        Place {
                            local,
                            projection: vec![],
                        },
                        value,
                    )
                })
                .collect(),
        ))
    }
    fn bottom_state(&self, _: &MirFunction) -> Recipes {
        Recipes::default()
    }

    fn apply_statement(&self, state: &mut Recipes, statement: &StatementData) {
        let candidate = match &statement.kind {
            StatementKind::Assign(destination, value)
                if destination.projection.is_empty() || self.bindings.direct_field(destination) =>
            {
                self.bindings
                    .place_type(destination)
                    .map(|ty| match value {
                        Rvalue::Use(operand) => self.operand(state, operand, ty),
                        Rvalue::Closure(closure) => {
                            // Exact construction identifies code even before its
                            // conditional return summary converges; it is not a
                            // producer certificate or mutation/effect permission.
                            let target = allocation_body(
                                self.bindings.program,
                                self.bindings.owner,
                                destination,
                                closure,
                            )
                            .filter(|body| body.closure_captures.is_empty())
                            .and_then(|body| {
                                self.bindings
                                    .program
                                    .backend_contract
                                    .function_bodies
                                    .get(&body.id)
                            });
                            target
                                .map(|key| Recipe::Callable(MirCallable::Resolved(key.clone()), ty))
                                .or_else(|| {
                                    self.bindings
                                        .assigned(&State::default(), destination, value)
                                        .map(Recipe::Measured)
                                })
                                .unwrap_or(Recipe::Opaque(ty))
                        }
                        Rvalue::Ref(Mutability::Not, source) => Recipe::Reference(
                            Box::new(self.place(state, source).unwrap_or(Recipe::Opaque(ty))),
                            ty,
                        ),
                        Rvalue::Aggregate(kind, operands) => {
                            let fields = operands
                                .iter()
                                .enumerate()
                                .map(|(index, operand)| {
                                    let field = crate::mir::borrowck::checked_aggregate_field(
                                        self.bindings.owner,
                                        &self.bindings.program.type_context,
                                        &self.bindings.program.backend_contract,
                                        destination,
                                        kind,
                                        index,
                                    )?;
                                    let field_ty = self.bindings.place_type(&field)?;
                                    let destination = ReferenceEscapeAnalysis::normalized_place(
                                        destination.clone(),
                                    );
                                    let projection = field
                                        .projection
                                        .strip_prefix(destination.projection.as_slice())?
                                        .to_vec();
                                    Some((projection, self.operand(state, operand, field_ty)))
                                })
                                .collect::<Option<Vec<_>>>();
                            let variant = match kind {
                                crate::mir::AggregateKind::EnumVariant { variant_id, .. } => {
                                    Some(*variant_id)
                                }
                                _ => None,
                            };
                            fields
                                .map(|fields| Recipe::Aggregate(ty, variant, fields))
                                .unwrap_or(Recipe::Opaque(ty))
                        }
                        _ => Recipe::Opaque(ty),
                    })
                    .map(|recipe| (destination.clone(), recipe))
            }
            _ => None,
        };
        let Some(facts) = &mut state.0 else {
            return;
        };
        match &statement.kind {
            StatementKind::Assign(destination, value) => {
                if matches!(value, Rvalue::Cast(_, _) | Rvalue::Ref(Mutability::Mut, _))
                    || (!destination.projection.is_empty()
                        && !self.bindings.direct_field(destination))
                {
                    facts.clear();
                } else {
                    if let Rvalue::Use(Operand::Move(source)) = value {
                        self.invalidate_place(facts, source);
                    }
                    if let Some((place, value)) = &candidate {
                        if self.bindings.direct_field(place) {
                            let root = Place {
                                local: place.local,
                                projection: vec![],
                            };
                            let root_ty = self.bindings.owner.local_decls[place.local.0].ty;
                            let base = facts.remove(&root).unwrap_or(Recipe::Opaque(root_ty));
                            let normalized =
                                ReferenceEscapeAnalysis::normalized_place(place.clone());
                            facts
                                .insert(root, base.overwrite(normalized.projection, value.clone()));
                            return;
                        }
                    }
                    facts.retain(|place, _| place.local != destination.local);
                }
                if let Some((place, value)) = candidate {
                    facts.insert(place, value);
                }
            }
            StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                facts.retain(|place, _| place.local != *local)
            }
            StatementKind::Assert(_) => {}
        }
    }

    fn apply_terminator(&self, state: &mut Recipes, term: &Terminator) {
        let candidate = if let Terminator::Call {
            func,
            args,
            destination,
            ..
        } = term
        {
            self.bindings.place_type(destination).and_then(|ty| {
                let params = self.bindings.call_parameters(func)?;
                if params.len() != args.len() {
                    return None;
                }
                let arguments = args
                    .iter()
                    .zip(params)
                    .map(|(operand, ty)| self.operand(state, operand, ty))
                    .collect();
                let (target, callee) = match func {
                    Operand::Constant(Constant::Callable(callable)) => {
                        crate::mir::borrowck::return_sources::checked_signature(
                            self.bindings.owner,
                            &self.bindings.program.type_context,
                            &self.bindings.program.backend_contract,
                            func,
                            args,
                            destination,
                        )?;
                        (Some(callable.clone()), None)
                    }
                    Operand::Copy(place) | Operand::Move(place) => {
                        (None, Some(Box::new(self.place(state, place)?)))
                    }
                    _ => return None,
                };
                Some((
                    destination.clone(),
                    Recipe::Call(target, callee, arguments, ty),
                ))
            })
        } else {
            None
        };
        let Some(facts) = &mut state.0 else {
            return;
        };
        match term {
            Terminator::Call {
                func,
                args,
                destination,
                ..
            } => {
                // No recipe grants mutation/retention permission. Discard values
                // whose storage is exposed by reference at an opaque call.
                if args.iter().any(|arg| matches!(arg, Operand::Copy(place) | Operand::Move(place)
                    if self.bindings.place_type(place).is_some_and(|ty| matches!(self.bindings.program.type_context.try_ty(ty), Some(Ty::Reference { .. } | Ty::Pointer(_)))))) {
                    facts.clear();
                } else {
                    for operand in args.iter().chain(std::iter::once(func)) {
                        if let Operand::Move(place) = operand { self.invalidate_place(facts, place); }
                    }
                    facts.retain(|place, _| place.local != destination.local);
                }
                if let Some((place, value)) = candidate {
                    facts.insert(place, value);
                }
            }
            Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } => {
                facts.retain(|key, _| key.local != place.local)
            }
            _ => {}
        }
    }
}

pub(super) fn measure(
    program: &MirProgram,
    lifetime: &HashMap<MirFunctionId, OriginSet>,
    values: &HashMap<MirFunctionId, OriginSet>,
    previous: &crate::mir::borrowck::return_sources::NativeReturnSources,
) -> HashMap<MirFunctionId, Recipe> {
    program
        .functions
        .values()
        .filter_map(|body| {
            let analysis = Measure {
                bindings: Bindings {
                    program,
                    owner: body,
                    lifetime,
                    values,
                    backend_valid: true,
                    calls: Some(previous),
                },
            };
            let flow = run_fixpoint(&analysis, body);
            let mut returns = Vec::new();
            for (index, block) in body.basic_blocks.iter().enumerate() {
                if !flow.reachable[index]
                    || !matches!(
                        block.terminator,
                        Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
                    )
                {
                    continue;
                }
                let mut state = flow.entry_sets[index].clone();
                for statement in &block.statements {
                    analysis.apply_statement(&mut state, statement);
                }
                let recipe = analysis
                    .place(
                        &state,
                        &Place {
                            local: Local(0),
                            projection: vec![],
                        },
                    )
                    .unwrap_or(Recipe::Opaque(body.ret_type));
                if !returns.contains(&recipe) {
                    returns.push(recipe);
                }
            }
            (!returns.is_empty()).then(|| {
                (
                    body.id.clone(),
                    Recipe::Alternatives(body.ret_type, returns),
                )
            })
        })
        .collect()
}

#[derive(Clone)]
pub(super) struct Value {
    ty: TypeId,
    pub(super) fields: HashMap<Vec<Projection>, Evidence>,
    variants: HashMap<Vec<Projection>, crate::ids::VariantId>,
}

/// Compose canonical target demands across reachable calls and local adapters.
/// Demand equality permits multiple sites; prior iterations compose nested
/// adapters. Escaping/capturing adapters still need portable capabilities and
/// are excluded; caller values and specialized headers never seed requirements.
pub(super) fn adapter_input_obligations(
    program: &MirProgram,
    lifetime: &HashMap<MirFunctionId, OriginSet>,
    values: &HashMap<MirFunctionId, OriginSet>,
    previous: &crate::mir::borrowck::return_sources::NativeReturnSources,
) -> HashMap<
    MirFunctionId,
    std::collections::BTreeMap<usize, crate::mir::backend_contract::MirGenericCallableInput>,
> {
    let mut eligible = HashMap::new();
    for owner in program.functions.values() {
        let bindings = Bindings {
            program,
            owner,
            lifetime,
            values,
            backend_valid: true,
            calls: Some(previous),
        };
        for statement in owner
            .basic_blocks
            .iter()
            .flat_map(|block| &block.statements)
        {
            let StatementKind::Assign(destination, Rvalue::Closure(closure)) = &statement.kind
            else {
                continue;
            };
            let id = MirFunctionId::Closure(Box::new(closure.id.clone()));
            let checked = closure.captures.is_empty()
                && allocation_body(program, owner, destination, closure).is_some()
                && bindings.isolated_storage(destination.local);
            // An unchecked/exposed construction cannot borrow a certificate
            // from another construction of the same body.
            eligible
                .entry(id)
                .and_modify(|old| *old &= checked)
                .or_insert(checked);
        }
    }
    let mut candidates = HashMap::new();
    for body in program.functions.values() {
        if eligible.get(&body.id) != Some(&true) {
            continue;
        }
        let analysis = Measure {
            bindings: Bindings {
                program,
                owner: body,
                lifetime,
                values,
                backend_valid: true,
                calls: Some(previous),
            },
        };
        let flow = run_fixpoint(&analysis, body);
        let mut demands = std::collections::BTreeMap::new();
        let mut supported = true;
        for (index, block) in body.basic_blocks.iter().enumerate() {
            if !flow.reachable[index] {
                continue;
            }
            let Some(Terminator::Call {
                func,
                args,
                destination: result,
                ..
            }) = &block.terminator
            else {
                continue;
            };
            let mut state = flow.entry_sets[index].clone();
            for statement in &block.statements {
                analysis.apply_statement(&mut state, statement);
            }
            let target = match func {
                Operand::Constant(Constant::Callable(callable)) => Some(callable.clone()),
                Operand::Copy(place) | Operand::Move(place) => analysis
                    .place(&state, place)
                    .and_then(|recipe| recipe.callable(&program.backend_contract)),
                _ => None,
            };
            let Some(MirCallable::Resolved(key)) = target else {
                supported = false;
                continue;
            };
            if crate::mir::borrowck::return_sources::checked_signature(
                body,
                &program.type_context,
                &program.backend_contract,
                &Operand::Constant(Constant::Callable(MirCallable::Resolved(key.clone()))),
                args,
                result,
            )
            .is_none()
            {
                supported = false;
                continue;
            }
            // Every identified call is checked, but only canonical input
            // obligations (or previously checked adapters) add demands.
            // Neither an incidental call nor its header certifies effects.
            let canonical = program
                .backend_contract
                .generic_callable_input_obligations
                .get(&key);
            let forwarded = MirCallable::Resolved(key.clone())
                .function_id()
                .and_then(|id| previous.callable_results.adapter_inputs.get(&id));
            for (index, requirement) in canonical
                .into_iter()
                .chain(forwarded)
                .flat_map(|requirements| requirements.iter())
            {
                let Some(argument) = args.get(*index) else {
                    supported = false;
                    continue;
                };
                let Recipe::Argument(parameter, ty) =
                    analysis.operand(&state, argument, requirement.subject)
                else {
                    supported = false;
                    continue;
                };
                if ty != requirement.subject
                    || parameter >= body.arg_count
                    || body.local_decls[parameter + 1].ty != ty
                {
                    supported = false;
                    continue;
                }
                if demands
                    .get(&parameter)
                    .is_some_and(|old| old != requirement)
                {
                    supported = false;
                } else {
                    demands.insert(parameter, requirement.clone());
                }
            }
        }
        if supported && !demands.is_empty() {
            candidates.insert(body.id.clone(), demands);
        }
    }
    candidates
}

pub(super) fn capture_inputs(
    program: &MirProgram,
    lifetime: &HashMap<MirFunctionId, OriginSet>,
    values: &HashMap<MirFunctionId, OriginSet>,
    previous: &crate::mir::borrowck::return_sources::NativeReturnSources,
) -> HashMap<MirFunctionId, HashMap<Place, Evidence>> {
    let mut captured: HashMap<MirFunctionId, HashMap<Place, Evidence>> = HashMap::new();
    for owner in program.functions.values() {
        if !owner.basic_blocks.iter().any(|block| {
            block.statements.iter().any(|statement| {
                matches!(statement.kind, StatementKind::Assign(_, Rvalue::Closure(_)))
            })
        }) {
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
        let flow = run_fixpoint(&analysis, owner);
        for (index, block) in owner.basic_blocks.iter().enumerate() {
            if !flow.reachable[index] {
                continue;
            }
            let mut state = flow.entry_sets[index].clone();
            for statement in &block.statements {
                if let StatementKind::Assign(destination, Rvalue::Closure(closure)) =
                    &statement.kind
                {
                    let mut facts = HashMap::new();
                    if let Some(body) = allocation_body(program, owner, destination, closure) {
                        for (source, stored) in closure.captures.iter().zip(&body.closure_captures)
                        {
                            if source.kind != crate::mir::MirClosureCaptureKind::ByValue {
                                continue;
                            }
                            let ty = owner.local_decls[source.local.0].ty;
                            let value = analysis.value(
                                &state,
                                &Operand::Copy(Place {
                                    local: source.local,
                                    projection: vec![],
                                }),
                                ty,
                            );
                            for (projection, mut evidence) in value.fields {
                                if !evidence.storage.is_empty()
                                    || evidence.sources != evidence.value_sources
                                {
                                    continue;
                                }
                                let place = Place {
                                    local: stored.local,
                                    projection,
                                };
                                if program.backend_contract.place_type_id(
                                    &program.type_context,
                                    body,
                                    &place,
                                ) != Some(evidence.ty)
                                {
                                    continue;
                                }
                                evidence.environment = Some(place.clone());
                                evidence.isolated = false;
                                facts.insert(place, evidence);
                            }
                        }
                    }
                    let id = MirFunctionId::Closure(Box::new(closure.id.clone()));
                    captured
                        .entry(id)
                        .and_modify(|old| {
                            old.retain(|place, evidence| facts.get(place) == Some(evidence))
                        })
                        .or_insert(facts);
                }
                analysis.apply_statement(&mut state, statement);
            }
        }
    }
    captured
}

impl Bindings<'_> {
    pub(super) fn forwarded_operand(
        &self,
        state: &State,
        operand: &Operand,
        destination: &Place,
    ) -> Vec<(Place, Evidence)> {
        let Some(ty) = self.place_type(destination) else {
            return vec![];
        };
        self.value(state, operand, ty)
            .fields
            .into_iter()
            .filter_map(|(path, mut evidence)| {
                let mut place = destination.clone();
                place.projection.extend(path);
                let expected = self.place_type(&place)?;
                if expected != evidence.ty {
                    if !crate::type_services::callable::input_shape_matches(
                        &self.program.type_context.type_for(expected),
                        &self.program.type_context.type_for(evidence.ty),
                    ) {
                        return None;
                    }
                    evidence.ty = expected;
                }
                Some((place, evidence))
            })
            .collect()
    }

    pub(super) fn value(&self, state: &State, operand: &Operand, ty: TypeId) -> Value {
        let mut fields = HashMap::new();
        if let Operand::Copy(source) | Operand::Move(source) = operand {
            let valid = self.place_type(source).is_some_and(|actual| {
                actual == ty
                    || crate::type_services::callable::input_shape_matches(
                        &self.program.type_context.type_for(ty),
                        &self.program.type_context.type_for(actual),
                    )
            });
            if !valid {
                return Value {
                    ty,
                    fields,
                    variants: HashMap::new(),
                };
            }
        }
        if let Some(evidence) = self.operand(state, operand, ty) {
            fields.insert(vec![], evidence);
        }
        if let Operand::Copy(source) | Operand::Move(source) = operand {
            if let Some(facts) = &state.0 {
                let source =
                    crate::mir::borrowck::ReferenceEscapeAnalysis::normalized_place(source.clone());
                for (place, evidence) in facts {
                    if place.local == source.local
                        && place.projection.starts_with(&source.projection)
                    {
                        fields.insert(
                            place.projection[source.projection.len()..].to_vec(),
                            evidence.clone(),
                        );
                    }
                }
            }
        }
        Value {
            ty,
            fields,
            variants: HashMap::new(),
        }
    }

    fn evaluate(
        &self,
        recipe: &Recipe,
        inputs: &[Value],
        active: &mut HashSet<MirFunctionId>,
    ) -> Option<Value> {
        let mut result = Value {
            ty: recipe.ty(),
            fields: HashMap::new(),
            variants: HashMap::new(),
        };
        match recipe {
            Recipe::Argument(index, ty) => {
                let value = inputs.get(*index)?;
                if value.ty != *ty
                    && !crate::type_services::callable::input_shape_matches(
                        &self.program.type_context.type_for(*ty),
                        &self.program.type_context.type_for(value.ty),
                    )
                {
                    return None;
                }
                result.fields = value.fields.clone();
                result.variants = value.variants.clone();
            }
            Recipe::Opaque(_) => {}
            Recipe::Callable(callable, ty) => {
                if let Some(evidence) = self.native(callable, *ty) {
                    result.fields.insert(vec![], evidence);
                }
            }
            Recipe::Measured(evidence) => {
                result.fields.insert(vec![], evidence.clone());
            }
            Recipe::Project(value, path, _) => {
                let value = self.evaluate(value, inputs, active)?;
                for (field, evidence) in value.fields {
                    if field.starts_with(path) {
                        result.fields.insert(field[path.len()..].to_vec(), evidence);
                    }
                }
                for (field, variant) in value.variants {
                    if field.starts_with(path) {
                        result
                            .variants
                            .insert(field[path.len()..].to_vec(), variant);
                    }
                }
            }
            Recipe::Reference(value, _) => {
                let value = self.evaluate(value, inputs, active)?;
                for (mut field, evidence) in value.fields {
                    field.insert(0, Projection::Deref);
                    result.fields.insert(field, evidence);
                }
            }
            Recipe::Aggregate(_, variant, fields) => {
                if let Some(variant) = variant {
                    result.variants.insert(vec![], *variant);
                }
                for (prefix, value) in fields {
                    let value = self.evaluate(value, inputs, active)?;
                    for (field, evidence) in value.fields {
                        let mut path = prefix.clone();
                        path.extend(field);
                        result.fields.insert(path, evidence);
                    }
                    for (field, variant) in value.variants {
                        let mut path = prefix.clone();
                        path.extend(field);
                        result.variants.insert(path, variant);
                    }
                }
            }
            Recipe::Call(target, callee, args, _) => {
                let factory = if let Some(target) = target {
                    target.function_id()?
                } else {
                    self.evaluate(callee.as_ref()?, inputs, active)?
                        .fields
                        .get(&Vec::new())?
                        .factory
                        .clone()?
                };
                if !active.insert(factory.clone()) {
                    return None;
                }
                let outcome = (|| {
                    let body = self.program.functions.get(&factory)?;
                    if body.ret_type != result.ty || body.arg_count != args.len() {
                        return None;
                    }
                    let recipe = self
                        .calls?
                        .callable_results
                        .forwarded_returns
                        .get(&factory)?;
                    let args = args
                        .iter()
                        .map(|arg| self.evaluate(arg, inputs, active))
                        .collect::<Option<Vec<_>>>()?;
                    self.evaluate(recipe, &args, active)
                })();
                active.remove(&factory);
                return outcome;
            }
            Recipe::Alternatives(_, alternatives) => {
                let alternatives = alternatives
                    .iter()
                    .map(|value| self.evaluate(value, inputs, active))
                    .collect::<Option<Vec<_>>>()?;
                for value in &alternatives {
                    for (path, evidence) in &value.fields {
                        let compatible = alternatives.iter().all(|other| {
                            let impossible = path.iter().enumerate().any(|(index, projection)| matches!(projection, Projection::Downcast(variant)
                                if other.variants.get(&path[..index]).is_some_and(|actual| actual != variant)));
                            impossible || other.fields.get(path) == Some(evidence)
                        });
                        if compatible {
                            result.fields.insert(path.clone(), evidence.clone());
                        }
                    }
                }
            }
            Recipe::Overlay(base, fields, _) => {
                let base = self.evaluate(base, inputs, active)?;
                result.fields = base.fields;
                result.variants = base.variants;
                for (path, recipe) in fields {
                    let written = Place {
                        local: Local(0),
                        projection: path.clone(),
                    };
                    let disjoint = |field: &Vec<Projection>| {
                        !crate::mir::borrowck::conflicts::places_conflict(
                            &written,
                            &ReferenceEscapeAnalysis::normalized_place(Place {
                                local: Local(0),
                                projection: field.clone(),
                            }),
                        )
                    };
                    result.fields.retain(|field, _| disjoint(field));
                    result.variants.retain(|field, _| disjoint(field));
                    let value = self.evaluate(recipe, inputs, active)?;
                    for (field, evidence) in value.fields {
                        let mut field_path = path.clone();
                        field_path.extend(field);
                        result.fields.insert(field_path, evidence);
                    }
                    for (field, variant) in value.variants {
                        let mut field_path = path.clone();
                        field_path.extend(field);
                        result.variants.insert(field_path, variant);
                    }
                }
            }
        }
        Some(result)
    }

    pub(super) fn forwarded_call(
        &self,
        state: &State,
        func: &Operand,
        args: &[Operand],
        destination: &Place,
    ) -> Vec<(Place, Evidence)> {
        let factory = match func {
            Operand::Constant(Constant::Callable(callable)) => {
                if crate::mir::borrowck::return_sources::checked_signature(
                    self.owner,
                    &self.program.type_context,
                    &self.program.backend_contract,
                    func,
                    args,
                    destination,
                )
                .is_none()
                {
                    return vec![];
                }
                callable.function_id()
            }
            Operand::Copy(place) | Operand::Move(place) => self
                .place_evidence(state, place)
                .and_then(|value| value.factory),
            _ => None,
        };
        let Some(factory) = factory else {
            return vec![];
        };
        let Some(body) = self.program.functions.get(&factory) else {
            return vec![];
        };
        if body.arg_count != args.len() || self.place_type(destination) != Some(body.ret_type) {
            return vec![];
        }
        let Some(recipe) = self
            .calls
            .and_then(|calls| calls.callable_results.forwarded_returns.get(&factory))
        else {
            return vec![];
        };
        let inputs = args
            .iter()
            .enumerate()
            .map(|(index, argument)| self.value(state, argument, body.local_decls[index + 1].ty))
            .collect::<Vec<_>>();
        let Some(value) = self.evaluate(recipe, &inputs, &mut HashSet::from([factory])) else {
            return vec![];
        };
        value
            .fields
            .into_iter()
            .filter_map(|(path, evidence)| {
                let mut place = destination.clone();
                place.projection.extend(path);
                (self.place_type(&place) == Some(evidence.ty)).then_some((place, evidence))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(index: usize) -> Projection {
        Projection::Field {
            index,
            identity: None,
        }
    }

    #[test]
    fn overlapping_record_write_joins_are_order_independent() {
        let ty = TypeId(0);
        let base = Recipe::Argument(0, ty);
        let child = base
            .clone()
            .overwrite(vec![field(0), field(1)], Recipe::Argument(1, ty));
        let parent = base
            .clone()
            .overwrite(vec![field(0)], Recipe::Argument(2, ty));
        let both = parent
            .clone()
            .overwrite(vec![field(0), field(1)], Recipe::Argument(1, ty));
        for lhs in [&base, &child, &parent, &both] {
            assert_eq!(lhs.common(lhs), Some(lhs.clone()));
            for rhs in [&base, &child, &parent, &both] {
                assert_eq!(lhs.common(rhs), rhs.common(lhs), "join must commute");
                for third in [&base, &child, &parent, &both] {
                    assert_eq!(
                        lhs.common(rhs).and_then(|value| value.common(third)),
                        rhs.common(third).and_then(|value| lhs.common(&value)),
                        "join must associate"
                    );
                }
            }
        }
    }
}
