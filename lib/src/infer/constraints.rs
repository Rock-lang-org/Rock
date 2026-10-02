//! Constraint store for deferred type constraint verification.
//!
//! During lowering, constraints are accumulated here instead of being checked
//! immediately. A dedicated solver pass (Phase 4) discharges them, producing
//! errors for unsatisfied trait bounds or ambiguous integer literal types.

use std::collections::{HashMap, HashSet};

use crate::ids::TypeVarId;
use crate::lexer::Span;
use crate::types::{TraitBound, Type};

/// Owner used to keep obligations from unrelated inference bodies isolated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConstraintOwner {
    Global,
    Body(crate::ids::DefId),
}

impl Default for ConstraintOwner {
    fn default() -> Self {
        Self::Global
    }
}

/// Stable identity for one stored inference obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObligationId(u32);

impl ObligationId {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Monotonic lifecycle state for a stored obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObligationState {
    Pending,
    Solved,
    Ambiguous,
    Failed,
}

#[derive(Debug, Clone, Copy)]
struct ObligationMeta {
    id: ObligationId,
    owner: ConstraintOwner,
    state: ObligationState,
    last_generation: u64,
    attempts: u32,
}

/// A deferred type constraint accumulated during lowering.
#[derive(Debug, Clone)]
pub enum Constraint {
    /// The type must implement the given trait (from BinOp, method calls, etc.)
    Trait {
        ty: Type,
        bound: TraitBound,
        span: Span,
        context: String,
    },
    /// The type variable must resolve to some integer type (from integer literals)
    IntLiteral { var: TypeVarId, span: Span },
    /// The type variable must resolve to some float type (from float literals)
    FloatLiteral { var: TypeVarId, span: Span },
    /// Two types must unify once deferred call-site inference has resolved them.
    Equality {
        left: Type,
        right: Type,
        span: Span,
        context: String,
    },
    /// An argument type must either unify directly with the expected type or
    /// use a supported argument coercion once the expected shape is known.
    Coercion {
        actual: Type,
        expected: Type,
        span: Span,
        context: String,
    },
    /// Types linked by one `?` expression while canonical authorities are deferred.
    Try {
        carrier: Type,
        output: Type,
        residual: Type,
        return_ty: Type,
        span: Span,
    },
}

/// Accumulated type constraints produced during lowering.
#[derive(Debug, Clone, Default)]
pub struct ConstraintStore {
    constraints: Vec<Constraint>,
    obligations: Vec<ObligationMeta>,
    dependencies: HashMap<TypeVarId, Vec<ObligationId>>,
    owner: ConstraintOwner,
}

impl ConstraintStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn replace_owner(&mut self, owner: ConstraintOwner) -> ConstraintOwner {
        std::mem::replace(&mut self.owner, owner)
    }

    pub fn owner(&self, id: ObligationId) -> Option<ConstraintOwner> {
        self.obligations
            .get(id.raw() as usize)
            .filter(|obligation| obligation.id == id)
            .map(|obligation| obligation.owner)
    }

    pub(crate) fn includes_owner(
        &self,
        id: ObligationId,
        owners: Option<&HashSet<ConstraintOwner>>,
    ) -> bool {
        owners.is_none_or(|owners| self.owner(id).is_some_and(|owner| owners.contains(&owner)))
    }

    pub fn add_trait(
        &mut self,
        ty: Type,
        bound: TraitBound,
        span: Span,
        context: &str,
    ) -> ObligationId {
        self.constraints.push(Constraint::Trait {
            ty,
            bound,
            span,
            context: context.to_string(),
        });
        self.push_obligation()
    }

    pub fn add_int_literal(&mut self, var: TypeVarId, span: Span) -> ObligationId {
        self.constraints.push(Constraint::IntLiteral { var, span });
        self.push_obligation()
    }

    pub fn add_float_literal(&mut self, var: TypeVarId, span: Span) -> ObligationId {
        self.constraints
            .push(Constraint::FloatLiteral { var, span });
        self.push_obligation()
    }

    pub fn add_equality(
        &mut self,
        left: Type,
        right: Type,
        span: Span,
        context: &str,
    ) -> ObligationId {
        self.constraints.push(Constraint::Equality {
            left,
            right,
            span,
            context: context.to_string(),
        });
        self.push_obligation()
    }

    pub fn add_coercion(
        &mut self,
        actual: Type,
        expected: Type,
        span: Span,
        context: &str,
    ) -> ObligationId {
        self.constraints.push(Constraint::Coercion {
            actual,
            expected,
            span,
            context: context.to_string(),
        });
        self.push_obligation()
    }

    pub fn add_try(
        &mut self,
        carrier: Type,
        output: Type,
        residual: Type,
        return_ty: Type,
        span: Span,
    ) -> ObligationId {
        self.constraints.push(Constraint::Try {
            carrier,
            output,
            residual,
            return_ty,
            span,
        });
        self.push_obligation()
    }

    fn push_obligation(&mut self) -> ObligationId {
        let id = ObligationId::new(self.obligations.len() as u32);
        self.obligations.push(ObligationMeta {
            id,
            owner: self.owner,
            state: ObligationState::Pending,
            last_generation: u64::MAX,
            attempts: 0,
        });
        let mut vars = HashSet::new();
        collect_constraint_vars(
            self.constraints
                .last()
                .expect("an obligation is created with its constraint"),
            &mut vars,
        );
        for var in vars {
            self.dependencies.entry(var).or_default().push(id);
        }
        id
    }

    pub fn obligation_ids(&self) -> impl Iterator<Item = ObligationId> + '_ {
        self.obligations.iter().map(|obligation| obligation.id)
    }

    pub fn obligation_id_at(&self, index: usize) -> Option<ObligationId> {
        self.obligations.get(index).map(|obligation| obligation.id)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (ObligationId, &Constraint)> {
        self.obligations
            .iter()
            .zip(&self.constraints)
            .map(|(obligation, constraint)| (obligation.id, constraint))
    }

    pub(crate) fn constraint(&self, id: ObligationId) -> Option<&Constraint> {
        self.obligations
            .get(id.raw() as usize)
            .filter(|obligation| obligation.id == id)
            .and_then(|_| self.constraints.get(id.raw() as usize))
    }

    pub fn state(&self, id: ObligationId) -> Option<ObligationState> {
        self.obligations
            .get(id.raw() as usize)
            .filter(|obligation| obligation.id == id)
            .map(|obligation| obligation.state)
    }

    pub fn obligation_state(&self, id: ObligationId) -> Option<ObligationState> {
        self.state(id)
    }

    pub fn owners_have_unresolved_obligations(&self, owners: &HashSet<ConstraintOwner>) -> bool {
        self.obligations.iter().any(|obligation| {
            owners.contains(&obligation.owner)
                && matches!(
                    obligation.state,
                    ObligationState::Pending | ObligationState::Ambiguous
                )
        })
    }

    pub(crate) fn set_state(&mut self, id: ObligationId, state: ObligationState) {
        if let Some(obligation) = self.obligations.get_mut(id.raw() as usize) {
            debug_assert_eq!(obligation.id, id);
            if matches!(
                obligation.state,
                ObligationState::Pending | ObligationState::Ambiguous
            ) {
                obligation.state = state;
            }
        }
    }

    pub(crate) fn reopen_ambiguous(&mut self, id: ObligationId) {
        if let Some(obligation) = self.obligations.get_mut(id.raw() as usize) {
            debug_assert_eq!(obligation.id, id);
            if obligation.state == ObligationState::Ambiguous {
                obligation.state = ObligationState::Pending;
                obligation.last_generation = u64::MAX;
            }
        }
    }

    pub(crate) fn mark_attempt(&mut self, id: ObligationId, generation: u64) {
        if let Some(obligation) = self.obligations.get_mut(id.raw() as usize) {
            debug_assert_eq!(obligation.id, id);
            obligation.last_generation = generation;
            obligation.attempts = obligation.attempts.saturating_add(1);
        }
    }

    pub fn obligation_attempts(&self, id: ObligationId) -> Option<u32> {
        self.obligations
            .get(id.raw() as usize)
            .filter(|obligation| obligation.id == id)
            .map(|obligation| obligation.attempts)
    }

    pub(crate) fn last_generation(&self, id: ObligationId) -> Option<u64> {
        self.obligations
            .get(id.raw() as usize)
            .filter(|obligation| obligation.id == id)
            .map(|obligation| obligation.last_generation)
    }

    /// Refresh one obligation after it changes its representative class.
    pub(crate) fn refresh_dependencies(
        &mut self,
        id: ObligationId,
        engine: &crate::infer::InferenceEngine,
    ) {
        for dependents in self.dependencies.values_mut() {
            dependents.retain(|dependent| *dependent != id);
        }
        let Some(constraint) = self.constraint(id) else {
            return;
        };
        let mut vars = HashSet::new();
        collect_constraint_vars(constraint, &mut vars);
        for var in vars {
            let representative = engine
                .unresolved_type_var_representative_for_dependency(var)
                .unwrap_or(var);
            self.dependencies
                .entry(representative)
                .or_default()
                .push(id);
        }
    }

    pub(crate) fn dependents_for(&self, var: TypeVarId) -> &[ObligationId] {
        self.dependencies
            .get(&var)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn dependencies_for(&self, var: TypeVarId) -> &[ObligationId] {
        self.dependents_for(var)
    }

    pub fn literal_default_type(&self, var: TypeVarId) -> Option<Type> {
        self.literal_default_type_for_representative(var, Some)
    }

    /// Returns the literal default for one resolved type-variable class.
    /// Mixed integer and float evidence deliberately produces no default.
    pub fn literal_default_type_for_representative(
        &self,
        representative: TypeVarId,
        mut resolve_representative: impl FnMut(TypeVarId) -> Option<TypeVarId>,
    ) -> Option<Type> {
        let mut default = None;
        for constraint in &self.constraints {
            let candidate = match constraint {
                Constraint::IntLiteral {
                    var: constrained, ..
                } if resolve_representative(*constrained) == Some(representative) => Type::I64,
                Constraint::FloatLiteral {
                    var: constrained, ..
                } if resolve_representative(*constrained) == Some(representative) => Type::F64,
                _ => continue,
            };
            match &default {
                Some(existing) if *existing != candidate => return None,
                Some(_) => {}
                None => default = Some(candidate),
            }
        }
        default
    }

    pub fn has_literal_evidence_for_representative(
        &self,
        representative: TypeVarId,
        mut resolve_representative: impl FnMut(TypeVarId) -> Option<TypeVarId>,
    ) -> bool {
        self.constraints.iter().any(|constraint| match constraint {
            Constraint::IntLiteral {
                var: constrained, ..
            }
            | Constraint::FloatLiteral {
                var: constrained, ..
            } => resolve_representative(*constrained) == Some(representative),
            Constraint::Trait { .. }
            | Constraint::Equality { .. }
            | Constraint::Coercion { .. }
            | Constraint::Try { .. } => false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TypeVarId;

    #[test]
    fn obligations_have_stable_ids_and_pending_initial_state() {
        let mut store = ConstraintStore::new();
        let first = store.add_int_literal(TypeVarId(0), Span::test());
        let second = store.add_float_literal(TypeVarId(1), Span::test());

        assert_eq!(first.raw(), 0);
        assert_eq!(second.raw(), 1);
        assert_eq!(
            store.obligation_state(first),
            Some(ObligationState::Pending)
        );
        assert_eq!(
            store.obligation_state(second),
            Some(ObligationState::Pending)
        );
        assert_eq!(
            store.obligation_ids().collect::<Vec<_>>(),
            vec![first, second]
        );
    }

    #[test]
    fn dependency_index_tracks_all_type_variable_representatives() {
        let mut store = ConstraintStore::new();
        let id = store.add_equality(
            Type::Apply {
                constructor: Box::new(Type::TypeVar(TypeVarId(0))),
                args: vec![Type::TypeVar(TypeVarId(1))],
            },
            Type::TypeVar(TypeVarId(2)),
            Span::test(),
            "dependency test",
        );
        assert!(store.dependencies_for(TypeVarId(0)).contains(&id));
        assert!(store.dependencies_for(TypeVarId(1)).contains(&id));
        assert!(store.dependencies_for(TypeVarId(2)).contains(&id));
    }

    #[test]
    fn try_obligation_tracks_all_linked_type_variables() {
        let mut store = ConstraintStore::new();
        let id = store.add_try(
            Type::TypeVar(TypeVarId(0)),
            Type::TypeVar(TypeVarId(1)),
            Type::TypeVar(TypeVarId(2)),
            Type::TypeVar(TypeVarId(3)),
            Span::test(),
        );
        for var in 0..=3 {
            assert!(store.dependencies_for(TypeVarId(var)).contains(&id));
        }
    }

    #[test]
    fn obligation_state_transitions_are_stable() {
        let mut store = ConstraintStore::new();
        let id = store.add_int_literal(TypeVarId(0), Span::test());

        store.set_state(id, ObligationState::Solved);
        assert_eq!(store.obligation_state(id), Some(ObligationState::Solved));
        store.set_state(id, ObligationState::Failed);
        assert_eq!(store.obligation_state(id), Some(ObligationState::Solved));
        assert_eq!(store.obligation_state(id), Some(ObligationState::Solved));

        let failed = store.add_int_literal(TypeVarId(1), Span::test());
        store.set_state(failed, ObligationState::Failed);
        store.set_state(failed, ObligationState::Ambiguous);
        assert_eq!(
            store.obligation_state(failed),
            Some(ObligationState::Failed)
        );
    }

    #[test]
    fn obligations_record_body_owner_without_cross_owner_visibility() {
        let mut store = ConstraintStore::new();
        let first = store.add_int_literal(TypeVarId(0), Span::test());
        let previous = store.replace_owner(ConstraintOwner::Body(crate::ids::DefId::new(
            crate::ids::CrateId(0),
            crate::ids::LocalDefId(1),
        )));
        let second = store.add_int_literal(TypeVarId(1), Span::test());

        assert_eq!(previous, ConstraintOwner::Global);
        assert_eq!(store.owner(first), Some(ConstraintOwner::Global));
        assert_eq!(
            store.owner(second),
            Some(ConstraintOwner::Body(crate::ids::DefId::new(
                crate::ids::CrateId(0),
                crate::ids::LocalDefId(1),
            )))
        );
        let owners = HashSet::from([ConstraintOwner::Global]);
        assert!(store.includes_owner(first, Some(&owners)));
        assert!(!store.includes_owner(second, Some(&owners)));
    }
}

fn collect_constraint_vars(constraint: &Constraint, output: &mut HashSet<TypeVarId>) {
    match constraint {
        Constraint::Trait { ty, bound, .. } => {
            collect_type_vars(ty, output);
            for arg in &bound.type_args {
                collect_type_vars(arg, output);
            }
        }
        Constraint::IntLiteral { var, .. } | Constraint::FloatLiteral { var, .. } => {
            output.insert(*var);
        }
        Constraint::Equality { left, right, .. } => {
            collect_type_vars(left, output);
            collect_type_vars(right, output);
        }
        Constraint::Coercion {
            actual, expected, ..
        } => {
            collect_type_vars(actual, output);
            collect_type_vars(expected, output);
        }
        Constraint::Try {
            carrier,
            output: try_output,
            residual,
            return_ty,
            ..
        } => {
            collect_type_vars(carrier, output);
            collect_type_vars(try_output, output);
            collect_type_vars(residual, output);
            collect_type_vars(return_ty, output);
        }
    }
}

fn collect_type_vars(ty: &Type, output: &mut HashSet<TypeVarId>) {
    match ty {
        Type::Object(object) => {
            for child in object.children() {
                collect_type_vars(child, output);
            }
        }
        Type::TypeVar(id) => {
            output.insert(*id);
        }
        Type::Slice(inner) | Type::Pointer(inner) => collect_type_vars(inner, output),
        Type::Array(inner, _) | Type::Reference { inner, .. } => collect_type_vars(inner, output),
        Type::Tuple(items) | Type::Struct { args: items, .. } | Type::Enum { args: items, .. } => {
            for item in items {
                collect_type_vars(item, output);
            }
        }
        Type::Function {
            params,
            ret,
            captures,
            ..
        } => {
            for param in params {
                collect_type_vars(param, output);
            }
            collect_type_vars(ret, output);
            for capture in captures {
                collect_type_vars(&capture.ty, output);
            }
        }
        Type::Projection { ty, trait_args, .. } => {
            collect_type_vars(ty, output);
            for arg in trait_args {
                collect_type_vars(arg, output);
            }
        }
        Type::Apply { constructor, args } => {
            collect_type_vars(constructor, output);
            for arg in args {
                collect_type_vars(arg, output);
            }
        }
        Type::Lambda { body, .. } => collect_type_vars(body, output),
        Type::I8
        | Type::ObjectSelf { .. }
        | Type::Witness(_)
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
        | Type::Generic(_)
        | Type::Constructor { .. }
        | Type::BoundVar { .. }
        | Type::Error => {}
    }
}
