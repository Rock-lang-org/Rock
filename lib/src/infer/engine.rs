//! Type inference engine implementation

use std::collections::{HashMap, HashSet};

use crate::ids::{IdGen, TypeVarId};
use crate::infer::constraints::{Constraint, ConstraintStore};
use crate::lexer::Span;
use crate::type_services::facts::TypeFacts;
use crate::type_services::kind::Kind;
use crate::type_services::normalize::{NormalizeError, TypeNormalizationEnv, TypeNormalizer};
use crate::type_services::visit::{fold_type, fold_type_children, TypeFolder};
use crate::types::{TraitBound, Type};

#[derive(Debug, Clone)]
pub enum UnifyError {
    Mismatch {
        expected: Type,
        found: Type,
    },
    InfiniteType {
        variable: TypeVarId,
        ty: Type,
    },
    KindMismatch {
        expected: Kind,
        found: Kind,
        ty: Option<Type>,
        context: &'static str,
    },
    ConstructorInference {
        pattern: Type,
        actual: Type,
    },
    AmbiguousConstructorHeads {
        left: Type,
        right: Type,
    },
    BinderEscape {
        ty: Type,
    },
    FunctionSafetyMismatch {
        expected: Type,
        found: Type,
    },
    ArityMismatch {
        kind: &'static str,
        expected: usize,
        found: usize,
        left: Type,
        right: Type,
    },
    MutabilityMismatch,
    Normalization {
        ty: Type,
        detail: NormalizeError,
    },
    AmbiguousType {
        kind: Kind,
    },
    NonCopyArrayRepeat {
        found: Type,
    },
    Message(String),
}

impl UnifyError {
    pub fn contains(&self, pattern: &str) -> bool {
        self.render_with(|ty| ty.to_string()).contains(pattern)
    }

    pub fn render(&self, engine: &InferenceEngine) -> String {
        self.render_with(|ty| engine.display_type(ty))
    }

    pub(crate) fn render_with(&self, display: impl Fn(&Type) -> String) -> String {
        match self {
            Self::Mismatch { expected, found } => {
                format!("Type mismatch: {} vs {}", display(expected), display(found))
            }
            Self::InfiniteType { ty, .. } => {
                format!("Infinite type: inferred type contains itself through {}", display(ty))
            }
            Self::KindMismatch {
                expected,
                found,
                ty,
                context,
            } => match ty {
                Some(ty) => format!(
                    "kind mismatch while {context} an inferred type to {}: {expected} vs {found}",
                    display(ty)
                ),
                None => format!("kind mismatch while {context}: {expected} vs {found}"),
            },
            Self::ConstructorInference { pattern, actual } => format!(
                "cannot infer a type constructor from {} = {}; add an explicit constructor section annotation",
                display(pattern),
                display(actual)
            ),
            Self::AmbiguousConstructorHeads { left, right } => format!(
                "ambiguous constructor heads: {} vs {}",
                display(left),
                display(right)
            ),
            Self::BinderEscape { ty } => {
                format!("type lambda binder escapes while inferring {}", display(ty))
            }
            Self::FunctionSafetyMismatch { expected, found } => format!(
                "Unsafe function cannot be used as safe function: {} vs {}",
                display(expected),
                display(found)
            ),
            Self::ArityMismatch {
                kind,
                expected,
                found,
                left,
                right,
            } => match *kind {
                "tuple" => format!("Tuple length mismatch: {expected} vs {found}"),
                "function" => format!("Function argument count mismatch: {expected} vs {found}"),
                "generic" => format!(
                    "Generic argument count mismatch for {}: {expected} vs {found}",
                    display(left)
                ),
                "application" => format!(
                    "Type application arity mismatch: {} vs {}",
                    display(left),
                    display(right)
                ),
                _ => format!("Type mismatch: {} vs {}", display(left), display(right)),
            },
            Self::MutabilityMismatch => "Mutability mismatch on references".to_string(),
            Self::Normalization { ty, detail } => format!(
                "failed to normalize finalized type {}: {}",
                display(ty),
                display_normalize_error(detail, &display)
            ),
            Self::AmbiguousType { kind } if kind == &Kind::Type => {
                "ambiguous type: cannot determine type of expression; add a type annotation (e.g., `: I64`)".to_string()
            }
            Self::AmbiguousType { kind } => format!(
                "ambiguous type constructor: cannot determine a value of kind {kind}; add an explicit constructor section annotation"
            ),
            Self::NonCopyArrayRepeat { found } => format!(
                "array repeat initializer must be Copy, found '{}'",
                display(found)
            ),
            Self::Message(message) => message.clone(),
        }
    }
}

fn display_normalize_error(error: &NormalizeError, display: &impl Fn(&Type) -> String) -> String {
    match error {
        NormalizeError::AliasCycle(_) => "type alias cycle".to_string(),
        NormalizeError::DepthLimit { limit } => {
            format!("type normalization depth limit exceeded ({limit})")
        }
        NormalizeError::NodeLimit { limit } => {
            format!("type normalization node limit exceeded ({limit})")
        }
        NormalizeError::UnknownConstructor(_) => "unknown type constructor".to_string(),
        NormalizeError::KindMismatch { expected, actual } => {
            format!("kind mismatch: expected {expected}, found {actual}")
        }
        NormalizeError::NotApplicable { kind } => {
            format!("type of kind {kind} is not applicable")
        }
        NormalizeError::NonCanonicalType { normalized } => format!(
            "type is not canonical; normalized form is {}",
            display(normalized)
        ),
    }
}

#[derive(Debug, Clone)]
pub struct InferenceError {
    pub error: UnifyError,
    pub span: Option<Span>,
}

impl InferenceError {
    pub fn contains(&self, pattern: &str) -> bool {
        self.error.contains(pattern)
    }
}

/// Type inference engine using unification
pub struct InferenceEngine {
    next_var: IdGen<TypeVarId>,
    substitutions: HashMap<TypeVarId, Type>,
    substitution_generation: u64,
    rigid_substitution_generation: u64,
    changed_type_vars: Vec<TypeVarId>,
    /// Trait bounds for type variables (var_id -> list of bounds)
    trait_bounds: HashMap<TypeVarId, Vec<TraitBound>>,
    /// Origin span for each fresh type variable (for error messages)
    pub var_spans: HashMap<TypeVarId, Span>,
    var_kinds: HashMap<TypeVarId, Kind>,
    normalization_env: TypeNormalizationEnv,
    type_display_context: crate::type_services::display::TypeDisplayContext,
    generic_defaults: Vec<(Type, Type)>,
}

struct ResolveFolder<'a> {
    substitutions: &'a HashMap<TypeVarId, Type>,
}

impl TypeFolder for ResolveFolder<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        match ty {
            Type::TypeVar(id) => self
                .substitutions
                .get(&id)
                .cloned()
                .map(|replacement| self.fold_type(replacement))
                .unwrap_or(Type::TypeVar(id)),
            other => {
                let resolved = fold_type_children(other, self);
                match resolved {
                    Type::Function {
                        params,
                        ret,
                        safety,
                        captures,
                        ..
                    } => Type::Function {
                        params,
                        ret,
                        safety,
                        callable_kind: crate::types::CallableKind::from_captures(&captures),
                        captures,
                    },
                    other => other,
                }
            }
        }
    }
}

struct StrictFinalizer<'a> {
    substitutions: &'a HashMap<TypeVarId, Type>,
    var_spans: &'a HashMap<TypeVarId, Span>,
    var_kinds: &'a HashMap<TypeVarId, Kind>,
    errors: &'a mut Vec<InferenceError>,
    reported: HashSet<TypeVarId>,
}

impl TypeFolder for StrictFinalizer<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        match ty {
            Type::TypeVar(id) => {
                if let Some(replacement) = self.substitutions.get(&id).cloned() {
                    return self.fold_type(replacement);
                }
                if self.reported.insert(id) {
                    let kind = self.var_kinds.get(&id).cloned().unwrap_or(Kind::Type);
                    self.errors.push(InferenceError {
                        error: UnifyError::AmbiguousType { kind },
                        span: self.var_spans.get(&id).cloned(),
                    });
                }
                Type::Error
            }
            other => fold_type_children(other, self),
        }
    }
}

#[cfg(test)]
mod tests;

impl InferenceEngine {
    pub fn new() -> Self {
        Self {
            next_var: IdGen::new(),
            substitutions: HashMap::new(),
            substitution_generation: 0,
            rigid_substitution_generation: 0,
            changed_type_vars: Vec::new(),
            trait_bounds: HashMap::new(),
            var_spans: HashMap::new(),
            var_kinds: HashMap::new(),
            normalization_env: TypeNormalizationEnv::new(),
            type_display_context: Default::default(),
            generic_defaults: Vec::new(),
        }
    }

    pub fn with_next_type_var(next: u32) -> Self {
        let mut var_kinds = HashMap::new();
        let mut normalization_env = TypeNormalizationEnv::new();
        for raw in 0..next {
            let id = TypeVarId(raw);
            var_kinds.insert(id, Kind::Type);
            normalization_env.register_inference_kind(id, Kind::Type);
        }
        Self {
            next_var: IdGen::with_next_raw(next),
            substitutions: HashMap::new(),
            substitution_generation: 0,
            rigid_substitution_generation: 0,
            changed_type_vars: Vec::new(),
            trait_bounds: HashMap::new(),
            var_spans: HashMap::new(),
            var_kinds,
            normalization_env,
            type_display_context: Default::default(),
            generic_defaults: Vec::new(),
        }
    }

    pub fn clone_for_probe(&self) -> Self {
        Self {
            next_var: IdGen::with_next_raw(self.next_var.next_raw()),
            substitutions: self.substitutions.clone(),
            substitution_generation: self.substitution_generation,
            rigid_substitution_generation: self.rigid_substitution_generation,
            changed_type_vars: Vec::new(),
            trait_bounds: self.trait_bounds.clone(),
            var_spans: self.var_spans.clone(),
            var_kinds: self.var_kinds.clone(),
            normalization_env: self.normalization_env.clone(),
            type_display_context: self.type_display_context.clone(),
            generic_defaults: self.generic_defaults.clone(),
        }
    }

    pub fn set_type_display_context(
        &mut self,
        context: crate::type_services::display::TypeDisplayContext,
    ) {
        self.type_display_context = context;
    }

    pub fn display_type(&self, ty: &Type) -> String {
        crate::type_services::display::display_type_with_context(ty, &self.type_display_context)
            .to_string()
    }

    pub(crate) fn clone_for_commit(&self) -> Self {
        let mut clone = self.clone_for_probe();
        clone.changed_type_vars = self.changed_type_vars.clone();
        clone
    }

    pub(crate) fn commit_probe(&mut self, mut probe: Self) {
        let changed = probe.take_changed_type_vars();
        let generation_delta = probe
            .substitution_generation
            .wrapping_sub(self.substitution_generation);
        let rigid_delta = probe
            .rigid_substitution_generation
            .wrapping_sub(self.rigid_substitution_generation);
        probe.substitution_generation = self.substitution_generation.wrapping_add(generation_delta);
        probe.rigid_substitution_generation =
            self.rigid_substitution_generation.wrapping_add(rigid_delta);
        probe.changed_type_vars = changed;
        *self = probe;
    }

    /// Add a trait bound to a type variable
    pub fn add_bound(&mut self, var_id: TypeVarId, bound: TraitBound) {
        self.trait_bounds.entry(var_id).or_default().push(bound);
    }

    /// Get all trait bounds for a type variable
    pub fn get_bounds(&self, var_id: TypeVarId) -> Vec<TraitBound> {
        self.trait_bounds.get(&var_id).cloned().unwrap_or_default()
    }

    pub(crate) fn substitution_generation(&self) -> u64 {
        self.substitution_generation
    }

    pub(crate) fn rigid_substitution_generation(&self) -> u64 {
        self.rigid_substitution_generation
    }

    pub(crate) fn take_changed_type_vars(&mut self) -> Vec<TypeVarId> {
        std::mem::take(&mut self.changed_type_vars)
    }

    pub(crate) fn unresolved_type_var_representative_for_dependency(
        &self,
        var: TypeVarId,
    ) -> Option<TypeVarId> {
        self.unresolved_type_var_representative(var)
    }

    /// Apply defaults only to unresolved variables with numeric literal evidence.
    pub fn apply_numeric_defaults(&mut self, constraints: &ConstraintStore) {
        self.apply_numeric_defaults_for_owners(constraints, None);
    }

    pub(crate) fn apply_numeric_defaults_for_owners(
        &mut self,
        constraints: &ConstraintStore,
        owners: Option<&HashSet<crate::infer::ConstraintOwner>>,
    ) {
        let representatives = constraints
            .iter()
            .filter(|(id, _)| constraints.includes_owner(*id, owners))
            .filter_map(|(_, constraint)| match constraint {
                Constraint::IntLiteral { var, .. } | Constraint::FloatLiteral { var, .. } => {
                    self.unresolved_type_var_representative(*var)
                }
                Constraint::Trait { .. }
                | Constraint::Equality { .. }
                | Constraint::Coercion { .. }
                | Constraint::Try { .. } => None,
            })
            .collect::<HashSet<_>>();
        for representative in representatives {
            if self.kind_of_type_var(representative) != Kind::Type {
                continue;
            }
            let Some(default) = constraints
                .literal_default_type_for_representative(representative, |var| {
                    self.unresolved_type_var_representative(var)
                })
            else {
                continue;
            };
            if self.unresolved_type_var_representative(representative) == Some(representative) {
                let _ = self.bind_type_var(representative, default);
            }
        }
    }

    fn unresolved_type_var_representative(&self, var: TypeVarId) -> Option<TypeVarId> {
        match self.resolve(&Type::TypeVar(var)) {
            Type::TypeVar(representative) => Some(representative),
            _ => None,
        }
    }

    pub(crate) fn set_normalization_env(&mut self, mut env: TypeNormalizationEnv) {
        for (id, kind) in &self.var_kinds {
            env.register_inference_kind(*id, kind.clone());
        }
        self.normalization_env = env;
    }

    pub fn kind_of_type_var(&self, id: TypeVarId) -> Kind {
        self.var_kinds.get(&id).cloned().unwrap_or(Kind::Type)
    }

    pub fn span_for_type(&self, ty: &Type) -> Option<Span> {
        let mut span = None;
        crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
            if span.is_none() {
                if let Type::TypeVar(id) = nested {
                    span = self.var_spans.get(id).cloned();
                }
            }
        });
        span
    }

    pub fn kind_of(&self, ty: &Type) -> Result<Kind, String> {
        TypeNormalizer::new(&self.normalization_env)
            .kind_of(&self.resolve(ty))
            .map_err(|error| error.to_string())
    }

    pub(crate) fn normalization_env(&self) -> TypeNormalizationEnv {
        self.normalization_env.clone()
    }

    pub fn fresh_type_var(&mut self) -> Type {
        self.fresh_type_var_of_kind(Kind::Type)
    }

    pub(crate) fn fresh_generic_args(&mut self, params: &[crate::types::GenericParamDecl], span: &Span) -> Vec<Type> {
        let args = params.iter().map(|param| self.fresh_type_var_at_kind(span.clone(), param.kind.clone())).collect::<Vec<_>>();
        self.register_generic_defaults(params, &args);
        args
    }

    pub(crate) fn register_generic_defaults(&mut self, params: &[crate::types::GenericParamDecl], args: &[Type]) {
        let substitutions = params.iter().zip(args).map(|(param, arg)| (param.id, arg.clone())).collect();
        for (param, arg) in params.iter().zip(args) {
            if let Some(default) = &param.default {
                self.generic_defaults.push((arg.clone(), default.substitute_generics(&substitutions)));
            }
        }
    }

    pub(crate) fn apply_generic_defaults(&mut self, parameter_vars: &HashSet<TypeVarId>) -> Result<(), InferenceError> {
        for (arg, default) in std::mem::take(&mut self.generic_defaults) {
            if matches!(self.resolve(&arg), Type::TypeVar(id) if !parameter_vars.contains(&id)) {
                self.unify(&arg, &default).map_err(|error| InferenceError { error, span: self.span_for_type(&arg) })?;
            }
        }
        Ok(())
    }

    pub fn fresh_type_var_of_kind(&mut self, kind: Kind) -> Type {
        let id = self.next_var.fresh();
        self.var_kinds.insert(id, kind.clone());
        self.normalization_env.register_inference_kind(id, kind);
        Type::TypeVar(id)
    }

    /// Create a fresh type variable with an associated source span (for error reporting)
    pub fn fresh_type_var_at(&mut self, span: Span) -> Type {
        self.fresh_type_var_at_kind(span, Kind::Type)
    }

    pub fn fresh_type_var_at_kind(&mut self, span: Span, kind: Kind) -> Type {
        let id = self.next_var.fresh();
        self.var_spans.insert(id, span);
        self.var_kinds.insert(id, kind.clone());
        self.normalization_env.register_inference_kind(id, kind);
        Type::TypeVar(id)
    }

    pub fn seed_var_spans(&mut self, spans: impl IntoIterator<Item = (TypeVarId, Span)>) {
        for (id, span) in spans {
            self.var_spans.insert(id, span);
            self.var_kinds.insert(id, Kind::Type);
            self.normalization_env
                .register_inference_kind(id, Kind::Type);
        }
    }

    /// Find the representative type for a type variable (path compression)
    pub fn resolve(&self, ty: &Type) -> Type {
        fold_type(
            ty.clone(),
            &mut ResolveFolder {
                substitutions: &self.substitutions,
            },
        )
    }

    /// Unify two types, producing substitutions
    pub fn unify(&mut self, a: &Type, b: &Type) -> Result<(), UnifyError> {
        let original_a = a.clone();
        let original_b = b.clone();
        let resolved_a = self.resolve(a);
        let a = TypeNormalizer::new(&self.normalization_env)
            .normalize(&resolved_a)
            .map_err(|error| UnifyError::Normalization {
                ty: resolved_a.clone(),
                detail: error,
            })?;
        let resolved_b = self.resolve(b);
        let b = TypeNormalizer::new(&self.normalization_env)
            .normalize(&resolved_b)
            .map_err(|error| UnifyError::Normalization {
                ty: resolved_b.clone(),
                detail: error,
            })?;

        if a == b {
            if let Type::TypeVar(id) = original_a {
                if self.substitutions.contains_key(&id) && self.substitutions.get(&id) != Some(&a) {
                    self.substitutions.insert(id, a.clone());
                    self.substitution_generation = self.substitution_generation.wrapping_add(1);
                    self.changed_type_vars.push(id);
                }
            }
            return Ok(());
        }

        let a_kind = TypeNormalizer::new(&self.normalization_env)
            .kind_of(&a)
            .map_err(|error| UnifyError::Normalization {
                ty: a.clone(),
                detail: error,
            })?;
        let b_kind = TypeNormalizer::new(&self.normalization_env)
            .kind_of(&b)
            .map_err(|error| UnifyError::Normalization {
                ty: b.clone(),
                detail: error,
            })?;
        if a_kind != b_kind {
            return Err(UnifyError::KindMismatch {
                expected: a_kind,
                found: b_kind,
                ty: None,
                context: "unifying",
            });
        }

        if let Some(result) = self.try_unify_constructor_spine(&a, &b) {
            if result.is_ok() {
                self.canonicalize_type_var_alias(&original_a, &b);
            }
            return result;
        }
        if let Some(result) = self.try_unify_constructor_spine(&b, &a) {
            if result.is_ok() {
                self.canonicalize_type_var_alias(&original_b, &a);
            }
            return result;
        }

        match (&a, &b) {
            (Type::TypeVar(id), _) => self.bind_type_var(*id, b),
            (_, Type::TypeVar(id)) => self.bind_type_var(*id, a),
            (Type::Never, _) | (_, Type::Never) => Ok(()),
            (Type::Error, _) | (_, Type::Error) => Ok(()),
            (Type::Tuple(elements), Type::Unit) | (Type::Unit, Type::Tuple(elements))
                if elements.is_empty() =>
            {
                Ok(())
            }
            (Type::Slice(a_inner), Type::Slice(b_inner)) => self.unify(a_inner, b_inner),
            (Type::Array(a_inner, a_len), Type::Array(b_inner, b_len)) => {
                if a_len != b_len {
                    return Err(UnifyError::Mismatch {
                        expected: a.clone(),
                        found: b.clone(),
                    });
                }
                self.unify(a_inner, b_inner)
            }
            (Type::Tuple(a_elems), Type::Tuple(b_elems)) => {
                if a_elems.len() != b_elems.len() {
                    return Err(UnifyError::ArityMismatch {
                        kind: "tuple",
                        expected: a_elems.len(),
                        found: b_elems.len(),
                        left: a.clone(),
                        right: b.clone(),
                    });
                }
                for (ae, be) in a_elems.iter().zip(b_elems.iter()) {
                    self.unify(ae, be)?;
                }
                Ok(())
            }
            (
                Type::Function {
                    params: a_args,
                    ret: a_ret,
                    safety: a_safety,
                    ..
                },
                Type::Function {
                    params: b_args,
                    ret: b_ret,
                    safety: b_safety,
                    ..
                },
            ) => {
                if matches!(a_safety, crate::types::FunctionSafety::Unsafe)
                    && matches!(b_safety, crate::types::FunctionSafety::Safe)
                {
                    return Err(UnifyError::FunctionSafetyMismatch {
                        expected: a.clone(),
                        found: b.clone(),
                    });
                }
                if a_args.len() != b_args.len() {
                    return Err(UnifyError::ArityMismatch {
                        kind: "function",
                        expected: a_args.len(),
                        found: b_args.len(),
                        left: a.clone(),
                        right: b.clone(),
                    });
                }
                for (aa, ba) in a_args.iter().zip(b_args.iter()) {
                    self.unify(ba, aa)?;
                }
                self.unify(a_ret, b_ret)?;
                self.preserve_function_metadata(&original_a, &original_b, &a, &b);
                Ok(())
            }
            (
                Type::Struct {
                    id: a_id,
                    args: a_gen,
                },
                Type::Struct {
                    id: b_id,
                    args: b_gen,
                },
            )
            | (
                Type::Enum {
                    id: a_id,
                    args: a_gen,
                },
                Type::Enum {
                    id: b_id,
                    args: b_gen,
                },
            ) => {
                if a_id != b_id {
                    return Err(UnifyError::Mismatch {
                        expected: a.clone(),
                        found: b.clone(),
                    });
                }
                if a_gen.len() != b_gen.len() {
                    return Err(UnifyError::ArityMismatch {
                        kind: "generic",
                        expected: a_gen.len(),
                        found: b_gen.len(),
                        left: a.clone(),
                        right: b.clone(),
                    });
                }
                for (ag, bg) in a_gen.iter().zip(b_gen.iter()) {
                    self.unify(ag, bg)?;
                }
                Ok(())
            }
            (
                Type::Reference {
                    mutable: am,
                    inner: ai,
                },
                Type::Reference {
                    mutable: bm,
                    inner: bi,
                },
            ) => {
                if am != bm {
                    return Err(UnifyError::MutabilityMismatch);
                }
                if *am {
                    self.unify_invariant(ai, bi)
                } else {
                    self.unify(ai, bi)
                }
            }
            (Type::Pointer(a_inner), Type::Pointer(b_inner)) => {
                self.unify_invariant(a_inner, b_inner)
            }
            (
                Type::Projection {
                    ty: a_ty,
                    trait_id: a_trait,
                    assoc_type: a_assoc,
                    trait_args: a_args,
                },
                Type::Projection {
                    ty: b_ty,
                    trait_id: b_trait,
                    assoc_type: b_assoc,
                    trait_args: b_args,
                },
            ) => {
                if a_trait != b_trait || a_assoc != b_assoc || a_args.len() != b_args.len() {
                    return Err(UnifyError::Mismatch {
                        expected: a.clone(),
                        found: b.clone(),
                    });
                }
                self.unify(a_ty, b_ty)?;
                for (a_arg, b_arg) in a_args.iter().zip(b_args.iter()) {
                    self.unify(a_arg, b_arg)?;
                }
                Ok(())
            }
            (
                Type::Constructor {
                    id: a_id,
                    flavor: a_flavor,
                },
                Type::Constructor {
                    id: b_id,
                    flavor: b_flavor,
                },
            ) if a_id == b_id && a_flavor == b_flavor => Ok(()),
            (
                Type::Apply {
                    constructor: a_constructor,
                    args: a_args,
                },
                Type::Apply {
                    constructor: b_constructor,
                    args: b_args,
                },
            ) => {
                if let (Type::TypeVar(a_id), Type::TypeVar(b_id)) =
                    (a_constructor.as_ref(), b_constructor.as_ref())
                {
                    if a_id != b_id {
                        return Err(UnifyError::AmbiguousConstructorHeads {
                            left: a.clone(),
                            right: b.clone(),
                        });
                    }
                }
                if a_args.len() != b_args.len() {
                    return Err(UnifyError::ArityMismatch {
                        kind: "application",
                        expected: a_args.len(),
                        found: b_args.len(),
                        left: a.clone(),
                        right: b.clone(),
                    });
                }
                self.unify(a_constructor, b_constructor)?;
                for (a_arg, b_arg) in a_args.iter().zip(b_args) {
                    self.unify(a_arg, b_arg)?;
                }
                Ok(())
            }
            (
                Type::Lambda {
                    params: a_params,
                    body: a_body,
                },
                Type::Lambda {
                    params: b_params,
                    body: b_body,
                },
            ) => {
                if a_params != b_params {
                    return Err(UnifyError::Mismatch {
                        expected: a.clone(),
                        found: b.clone(),
                    });
                }
                self.unify(a_body, b_body)
            }
            (
                Type::BoundVar {
                    depth: a_depth,
                    index: a_index,
                    kind: a_kind,
                },
                Type::BoundVar {
                    depth: b_depth,
                    index: b_index,
                    kind: b_kind,
                },
            ) if a_depth == b_depth && a_index == b_index && a_kind == b_kind => Ok(()),
            // Allow integer literal type coercion
            (a_ty, b_ty) if TypeFacts::is_integer(a_ty) && TypeFacts::is_integer(b_ty) => {
                // We'll allow implicit integer coercion for now
                Ok(())
            }
            _ => Err(UnifyError::Mismatch {
                expected: a.clone(),
                found: b.clone(),
            }),
        }
    }

    fn bind_type_var(&mut self, id: TypeVarId, ty: Type) -> Result<(), UnifyError> {
        if self.substitutions.get(&id) == Some(&ty) {
            return Ok(());
        }
        let variable_kind = self.kind_of_type_var(id);
        let type_kind = TypeNormalizer::new(&self.normalization_env)
            .kind_of(&ty)
            .map_err(|error| UnifyError::Normalization {
                ty: ty.clone(),
                detail: error,
            })?;
        if variable_kind != type_kind {
            return Err(UnifyError::KindMismatch {
                expected: variable_kind,
                found: type_kind,
                ty: Some(ty),
                context: "binding",
            });
        }
        if Self::has_escaping_bound_var(&ty, 0) {
            return Err(UnifyError::BinderEscape { ty });
        }
        if self.occurs_in(id, &ty) {
            return Err(UnifyError::InfiniteType { variable: id, ty });
        }
        if let (Type::TypeVar(target_id), Some(span)) = (&ty, self.var_spans.get(&id).cloned()) {
            self.var_spans.entry(*target_id).or_insert(span);
        }
        self.substitutions.insert(id, ty);
        self.substitution_generation = self.substitution_generation.wrapping_add(1);
        if !crate::type_services::visit::type_any(&self.substitutions[&id], |nested| {
            matches!(nested, Type::TypeVar(_))
        }) {
            self.rigid_substitution_generation = self.rigid_substitution_generation.wrapping_add(1);
        }
        self.changed_type_vars.push(id);
        if let Type::TypeVar(target_id) = self.substitutions[&id].clone() {
            self.changed_type_vars.push(target_id);
        }
        Ok(())
    }

    pub(crate) fn bind_pending_type_var(
        &mut self,
        id: TypeVarId,
        ty: &Type,
    ) -> Result<(), UnifyError> {
        match self.resolve(&Type::TypeVar(id)) {
            Type::TypeVar(current) => self.bind_type_var(current, ty.clone()),
            current => self.unify(&current, ty),
        }
    }

    pub(crate) fn normalize_resolved_type(&self, ty: &Type) -> Result<Type, String> {
        TypeNormalizer::new(&self.normalization_env)
            .normalize(&self.resolve(ty))
            .map_err(|error| error.to_string())
    }

    fn canonicalize_type_var_alias(&mut self, original: &Type, other: &Type) {
        let Type::TypeVar(id) = original else {
            return;
        };
        let Ok(normalized) =
            TypeNormalizer::new(&self.normalization_env).normalize(&self.resolve(other))
        else {
            return;
        };
        if matches!(normalized, Type::TypeVar(_)) || self.occurs_in(*id, &normalized) {
            return;
        }
        if self.substitutions.contains_key(id) && self.substitutions.get(id) != Some(&normalized) {
            self.substitutions.insert(*id, normalized);
            self.substitution_generation = self.substitution_generation.wrapping_add(1);
            self.changed_type_vars.push(*id);
        }
    }

    fn try_unify_constructor_spine(
        &mut self,
        pattern: &Type,
        actual: &Type,
    ) -> Option<Result<(), UnifyError>> {
        let Type::Apply {
            constructor,
            args: pattern_args,
        } = pattern
        else {
            return None;
        };
        let Type::TypeVar(constructor_var) = constructor.as_ref() else {
            return None;
        };
        if matches!(
            actual,
            Type::TypeVar(id) if id == constructor_var
        ) || matches!(
            actual,
            Type::Apply { constructor, .. }
                if matches!(constructor.as_ref(), Type::TypeVar(id) if id == constructor_var)
        ) {
            return None;
        }
        if let Type::TypeVar(actual_var) = actual {
            return self
                .occurs_in(*actual_var, pattern)
                .then_some(Err(Self::constructor_inference_error(pattern, actual)));
        }
        if matches!(
            actual,
            Type::Apply {
                constructor,
                ..
            } if matches!(constructor.as_ref(), Type::TypeVar(_))
        ) {
            return None;
        }
        let Some((actual_constructor, actual_args)) = Self::rigid_spine(actual) else {
            return Some(Err(Self::constructor_inference_error(pattern, actual)));
        };
        if actual_args.len() < pattern_args.len() {
            return Some(Err(Self::constructor_inference_error(pattern, actual)));
        }

        let mut probe = self.clone_for_probe();
        if actual_args.len() == pattern_args.len() {
            if probe
                .bind_type_var(*constructor_var, actual_constructor)
                .is_err()
            {
                return Some(Err(Self::constructor_inference_error(pattern, actual)));
            }
            for (pattern_arg, actual_arg) in pattern_args.iter().zip(actual_args.iter()) {
                if probe.unify(pattern_arg, actual_arg).is_err() {
                    return Some(Err(Self::constructor_inference_error(pattern, actual)));
                }
            }
            *self = probe;
            return Some(Ok(()));
        }
        let pattern_kinds = pattern_args
            .iter()
            .map(|arg| {
                TypeNormalizer::new(&probe.normalization_env)
                    .kind_of(&probe.resolve(arg))
                    .ok()
            })
            .collect::<Option<Vec<_>>>();
        let Some(pattern_kinds) = pattern_kinds else {
            return Some(Err(Self::constructor_inference_error(pattern, actual)));
        };
        let mut hole_indices = std::collections::HashMap::new();
        let mut section_params = Vec::new();
        let section_args = pattern_args
            .iter()
            .zip(pattern_kinds.iter())
            .map(|(pattern_arg, kind)| {
                let hole_index = if let Type::TypeVar(id) = pattern_arg {
                    if let Some(index) = hole_indices.get(id).copied() {
                        index
                    } else {
                        let index = section_params.len() as u32;
                        hole_indices.insert(*id, index);
                        section_params.push(kind.clone());
                        index
                    }
                } else {
                    let index = section_params.len() as u32;
                    section_params.push(kind.clone());
                    index
                };
                Type::BoundVar {
                    depth: 0,
                    index: hole_index,
                    kind: kind.clone(),
                }
            })
            .chain(actual_args[pattern_args.len()..].iter().cloned())
            .collect::<Vec<_>>();
        let section_body = if section_args.len() == actual_args.len() {
            match actual {
                Type::Struct { id, .. } => Type::Struct {
                    id: *id,
                    args: section_args,
                },
                Type::Enum { id, .. } => Type::Enum {
                    id: *id,
                    args: section_args,
                },
                _ => Type::Apply {
                    constructor: Box::new(actual_constructor),
                    args: section_args,
                },
            }
        } else {
            Type::Apply {
                constructor: Box::new(actual_constructor),
                args: section_args,
            }
        };
        let section = if pattern_args.is_empty() {
            section_body
        } else {
            Type::Lambda {
                params: section_params,
                body: Box::new(section_body),
            }
        };
        for (pattern_arg, actual_arg) in pattern_args.iter().zip(actual_args.iter()) {
            if probe.unify(pattern_arg, actual_arg).is_err() {
                return Some(Err(Self::constructor_inference_error(pattern, actual)));
            }
        }
        if probe.bind_type_var(*constructor_var, section).is_err() {
            return Some(Err(Self::constructor_inference_error(pattern, actual)));
        }
        for (pattern_arg, actual_arg) in pattern_args.iter().zip(actual_args.iter()) {
            if probe.unify(pattern_arg, actual_arg).is_err() {
                return Some(Err(Self::constructor_inference_error(pattern, actual)));
            }
        }
        *self = probe;
        Some(Ok(()))
    }

    fn rigid_spine(ty: &Type) -> Option<(Type, Vec<Type>)> {
        match ty {
            Type::Struct { id, args } => Some((
                Type::Constructor {
                    id: *id,
                    flavor: crate::types::NominalTypeKind::Struct,
                },
                args.clone(),
            )),
            Type::Enum { id, args } => Some((
                Type::Constructor {
                    id: *id,
                    flavor: crate::types::NominalTypeKind::Enum,
                },
                args.clone(),
            )),
            Type::Apply { constructor, args }
                if !matches!(
                    constructor.as_ref(),
                    Type::TypeVar(_) | Type::BoundVar { .. }
                ) =>
            {
                Some((constructor.as_ref().clone(), args.clone()))
            }
            Type::Constructor { .. } | Type::Generic(_) | Type::Projection { .. } => {
                Some((ty.clone(), Vec::new()))
            }
            _ => None,
        }
    }

    fn constructor_inference_error(pattern: &Type, actual: &Type) -> UnifyError {
        UnifyError::ConstructorInference {
            pattern: pattern.clone(),
            actual: actual.clone(),
        }
    }

    fn has_escaping_bound_var(ty: &Type, binder_depth: u32) -> bool {
        match ty {
            Type::BoundVar { depth, .. } => *depth >= binder_depth,
            Type::Slice(inner) | Type::Pointer(inner) => {
                Self::has_escaping_bound_var(inner, binder_depth)
            }
            Type::Array(inner, _) | Type::Reference { inner, .. } => {
                Self::has_escaping_bound_var(inner, binder_depth)
            }
            Type::Tuple(items)
            | Type::Struct { args: items, .. }
            | Type::Enum { args: items, .. } => items
                .iter()
                .any(|item| Self::has_escaping_bound_var(item, binder_depth)),
            Type::Function {
                params,
                ret,
                captures,
                ..
            } => {
                params
                    .iter()
                    .any(|param| Self::has_escaping_bound_var(param, binder_depth))
                    || Self::has_escaping_bound_var(ret, binder_depth)
                    || captures
                        .iter()
                        .any(|capture| Self::has_escaping_bound_var(&capture.ty, binder_depth))
            }
            Type::Projection { ty, trait_args, .. } => {
                Self::has_escaping_bound_var(ty, binder_depth)
                    || trait_args
                        .iter()
                        .any(|arg| Self::has_escaping_bound_var(arg, binder_depth))
            }
            Type::Apply { constructor, args } => {
                Self::has_escaping_bound_var(constructor, binder_depth)
                    || args
                        .iter()
                        .any(|arg| Self::has_escaping_bound_var(arg, binder_depth))
            }
            Type::Lambda { body, .. } => {
                Self::has_escaping_bound_var(body, binder_depth.saturating_add(1))
            }
            _ => false,
        }
    }

    pub(crate) fn unify_invariant(&mut self, a: &Type, b: &Type) -> Result<(), UnifyError> {
        let mut probe = self.clone_for_probe();
        probe.unify(a, b)?;
        probe.unify(b, a)?;
        let resolved_a = TypeNormalizer::new(&probe.normalization_env)
            .normalize(&probe.resolve(a))
            .map_err(|error| UnifyError::Normalization {
                ty: probe.resolve(a),
                detail: error,
            })?;
        let resolved_b = TypeNormalizer::new(&probe.normalization_env)
            .normalize(&probe.resolve(b))
            .map_err(|error| UnifyError::Normalization {
                ty: probe.resolve(b),
                detail: error,
            })?;
        if !Self::same_unification_shape(&resolved_a, &resolved_b) {
            return Err(UnifyError::Mismatch {
                expected: resolved_a,
                found: resolved_b,
            });
        }
        *self = probe;
        Ok(())
    }

    fn preserve_function_metadata(
        &mut self,
        original_a: &Type,
        original_b: &Type,
        a: &Type,
        b: &Type,
    ) {
        let Type::Function {
            callable_kind: a_kind,
            captures: a_captures,
            ..
        } = a
        else {
            return;
        };
        let Type::Function {
            callable_kind: b_kind,
            captures: b_captures,
            ..
        } = b
        else {
            return;
        };

        let a_is_default = a_kind.is_default(a_captures);
        let b_is_default = b_kind.is_default(b_captures);
        if !a_is_default && !b_is_default {
            return;
        }

        let metadata = if a_is_default {
            (*b_kind, b_captures.clone())
        } else {
            (*a_kind, a_captures.clone())
        };
        let replacement = |ty: &Type| match ty {
            Type::Function {
                params,
                ret,
                safety,
                ..
            } => Type::Function {
                params: params.clone(),
                ret: ret.clone(),
                safety: *safety,
                callable_kind: metadata.0,
                captures: metadata.1.clone(),
            },
            _ => ty.clone(),
        };

        for ty in [original_a, original_b] {
            let Type::TypeVar(id) = ty else {
                continue;
            };
            if self.substitutions.contains_key(id) {
                let current = self
                    .substitutions
                    .get(id)
                    .cloned()
                    .map(|ty| self.resolve(&ty))
                    .unwrap_or_else(|| ty.clone());
                let replacement = replacement(&current);
                if current != replacement {
                    self.substitutions.insert(*id, replacement);
                    self.substitution_generation = self.substitution_generation.wrapping_add(1);
                    self.rigid_substitution_generation =
                        self.rigid_substitution_generation.wrapping_add(1);
                    self.changed_type_vars.push(*id);
                }
            }
        }
    }

    fn same_unification_shape(a: &Type, b: &Type) -> bool {
        match (a, b) {
            (
                Type::Function {
                    params: a_params,
                    ret: a_ret,
                    safety: a_safety,
                    ..
                },
                Type::Function {
                    params: b_params,
                    ret: b_ret,
                    safety: b_safety,
                    ..
                },
            ) => {
                a_safety == b_safety
                    && a_params.len() == b_params.len()
                    && a_params
                        .iter()
                        .zip(b_params)
                        .all(|(a, b)| Self::same_unification_shape(a, b))
                    && Self::same_unification_shape(a_ret, b_ret)
            }
            (Type::Slice(a), Type::Slice(b)) | (Type::Pointer(a), Type::Pointer(b)) => {
                Self::same_unification_shape(a, b)
            }
            (Type::Array(a, a_len), Type::Array(b, b_len)) => {
                a_len == b_len && Self::same_unification_shape(a, b)
            }
            (Type::Tuple(a), Type::Tuple(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(a, b)| Self::same_unification_shape(a, b))
            }
            (
                Type::Reference {
                    mutable: a_mutable,
                    inner: a_inner,
                },
                Type::Reference {
                    mutable: b_mutable,
                    inner: b_inner,
                },
            ) => a_mutable == b_mutable && Self::same_unification_shape(a_inner, b_inner),
            (
                Type::Struct {
                    id: a_id,
                    args: a_args,
                },
                Type::Struct {
                    id: b_id,
                    args: b_args,
                },
            )
            | (
                Type::Enum {
                    id: a_id,
                    args: a_args,
                },
                Type::Enum {
                    id: b_id,
                    args: b_args,
                },
            ) => {
                a_id == b_id
                    && a_args.len() == b_args.len()
                    && a_args
                        .iter()
                        .zip(b_args)
                        .all(|(a, b)| Self::same_unification_shape(a, b))
            }
            (
                Type::Projection {
                    ty: a_ty,
                    trait_id: a_trait,
                    assoc_type: a_assoc,
                    trait_args: a_args,
                },
                Type::Projection {
                    ty: b_ty,
                    trait_id: b_trait,
                    assoc_type: b_assoc,
                    trait_args: b_args,
                },
            ) => {
                a_trait == b_trait
                    && a_assoc == b_assoc
                    && a_args.len() == b_args.len()
                    && Self::same_unification_shape(a_ty, b_ty)
                    && a_args
                        .iter()
                        .zip(b_args)
                        .all(|(a, b)| Self::same_unification_shape(a, b))
            }
            _ => a == b,
        }
    }

    /// Check if a type variable occurs in a type (for infinite type prevention)
    pub fn occurs_in(&self, var: TypeVarId, ty: &Type) -> bool {
        let ty = self.resolve(ty);
        match &ty {
            Type::TypeVar(id) => *id == var,
            Type::Slice(inner) | Type::Pointer(inner) => self.occurs_in(var, inner),
            Type::Array(inner, _) => self.occurs_in(var, inner),
            Type::Reference { inner, .. } => self.occurs_in(var, inner),
            Type::Tuple(elems) => elems.iter().any(|t| self.occurs_in(var, t)),
            Type::Function {
                params,
                ret,
                captures,
                ..
            } => {
                params.iter().any(|t| self.occurs_in(var, t))
                    || self.occurs_in(var, ret)
                    || captures
                        .iter()
                        .any(|capture| self.occurs_in(var, &capture.ty))
            }
            Type::Struct { args, .. } | Type::Enum { args, .. } => {
                args.iter().any(|t| self.occurs_in(var, t))
            }
            Type::Projection { ty, trait_args, .. } => {
                self.occurs_in(var, ty) || trait_args.iter().any(|arg| self.occurs_in(var, arg))
            }
            Type::Apply { constructor, args } => {
                self.occurs_in(var, constructor) || args.iter().any(|arg| self.occurs_in(var, arg))
            }
            Type::Lambda { body, .. } => self.occurs_in(var, body),
            _ => false,
        }
    }

    /// Finalize a type in strict mode: unresolved TypeVars produce an error entry.
    /// Returns `Type::Error` for ambiguous TypeVars so compilation can continue.
    pub fn finalize_strict(&self, ty: &Type, errors: &mut Vec<InferenceError>) -> Type {
        let finalized = fold_type(
            ty.clone(),
            &mut StrictFinalizer {
                substitutions: &self.substitutions,
                var_spans: &self.var_spans,
                var_kinds: &self.var_kinds,
                errors,
                reported: HashSet::new(),
            },
        );
        match TypeNormalizer::new(&self.normalization_env).normalize(&finalized) {
            Ok(normalized) => normalized,
            Err(error) => {
                let mut span = None;
                crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
                    if span.is_none() {
                        if let Type::TypeVar(id) = nested {
                            span = self.var_spans.get(id).cloned();
                        }
                    }
                });
                errors.push(InferenceError {
                    error: UnifyError::Normalization {
                        ty: finalized.clone(),
                        detail: error,
                    },
                    span,
                });
                Type::Error
            }
        }
    }
}

impl Default for InferenceEngine {
    fn default() -> Self {
        Self::new()
    }
}
