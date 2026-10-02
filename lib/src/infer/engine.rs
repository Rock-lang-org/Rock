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
    ObjectMatchPending {
        left: Type,
        right: Type,
    },
    ObjectMatchLimit {
        left: Type,
        right: Type,
    },
    ObjectBinderEscape {
        ty: Type,
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
            Self::ObjectMatchPending { left, right } => format!("ambiguous object trait arguments: {} vs {}", display(left), display(right)),
            Self::ObjectMatchLimit { .. } => "object signature matching resource limit exceeded".to_string(),
            Self::ObjectBinderEscape { ty } => format!("hidden Self escapes its object signature through {}", display(ty)),
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
        NormalizeError::Substitution(error) => format!("type substitution failed: {error}"),
        NormalizeError::ConflictingObjectBinding(_) => {
            "conflicting associated-type bindings in trait object".to_string()
        }
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
    witness_scopes: HashMap<TypeVarId, Vec<crate::types::WitnessId>>,
    active_witnesses: Vec<crate::types::WitnessId>,
    normalization_env: TypeNormalizationEnv,
    type_display_context: crate::type_services::display::TypeDisplayContext,
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
mod tests {
    use super::*;
    use crate::ids::{CrateId, DefId, LocalDefId, TypeVarId};

    fn test_def(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn constructor(id: DefId, flavor: crate::types::NominalTypeKind) -> Type {
        Type::Constructor { id, flavor }
    }

    fn apply(constructor: Type, args: Vec<Type>) -> Type {
        Type::Apply {
            constructor: Box::new(constructor),
            args,
        }
    }

    #[test]
    fn constructor_variable_unifies_with_rigid_nominal_spine() {
        let option = test_def(100);
        let mut env = TypeNormalizationEnv::new();
        env.register_nominal(option, crate::types::NominalTypeKind::Enum, 1);
        let mut engine = InferenceEngine::new();
        engine.set_normalization_env(env);
        let constructor_var = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));

        engine
            .unify(
                &apply(constructor_var.clone(), vec![Type::I64]),
                &Type::Enum {
                    id: option,
                    args: vec![Type::I64],
                },
            )
            .unwrap();

        assert_eq!(
            engine.resolve(&constructor_var),
            constructor(option, crate::types::NominalTypeKind::Enum)
        );
    }

    #[test]
    fn unification_keeps_mismatch_types_until_rendering() {
        let mut engine = InferenceEngine::new();
        let error = engine.unify(&Type::Bool, &Type::Char).unwrap_err();

        match &error {
            UnifyError::Mismatch { expected, found } => {
                assert_eq!(expected, &Type::Bool);
                assert_eq!(found, &Type::Char);
            }
            other => panic!("expected structured mismatch, got {other:?}"),
        }
        assert!(error.render(&engine).contains("Bool"));
        assert!(error.render(&engine).contains("Char"));
    }

    #[test]
    fn unification_keeps_ambiguous_constructor_heads_structured() {
        let mut engine = InferenceEngine::new();
        let kind = Kind::arrow(Kind::Type, Kind::Type);
        let left = apply(engine.fresh_type_var_of_kind(kind.clone()), vec![Type::I64]);
        let right = apply(engine.fresh_type_var_of_kind(kind), vec![Type::I64]);

        let error = engine.unify(&left, &right).unwrap_err();

        assert!(matches!(
            error,
            UnifyError::AmbiguousConstructorHeads { .. }
        ));
    }

    #[test]
    fn equal_unresolved_constructor_heads_unify_their_arguments_structurally() {
        let mut engine = InferenceEngine::new();
        let constructor_var = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));
        let argument = engine.fresh_type_var();

        engine
            .unify(
                &apply(constructor_var.clone(), vec![argument.clone()]),
                &apply(constructor_var.clone(), vec![Type::I64]),
            )
            .unwrap();

        assert_eq!(engine.resolve(&constructor_var), constructor_var);
        assert_eq!(engine.resolve(&argument), Type::I64);
    }

    #[test]
    fn nested_constructor_variables_unify_from_aligned_rigid_spines() {
        let option = test_def(101);
        let vector = test_def(102);
        let mut env = TypeNormalizationEnv::new();
        env.register_nominal(option, crate::types::NominalTypeKind::Enum, 1);
        env.register_nominal(vector, crate::types::NominalTypeKind::Struct, 1);
        let mut engine = InferenceEngine::new();
        engine.set_normalization_env(env);
        let unary = Kind::arrow(Kind::Type, Kind::Type);
        let f = engine.fresh_type_var_of_kind(unary.clone());
        let g = engine.fresh_type_var_of_kind(unary);
        let a = engine.fresh_type_var();

        engine
            .unify(
                &apply(f.clone(), vec![apply(g.clone(), vec![a.clone()])]),
                &Type::Struct {
                    id: vector,
                    args: vec![Type::Enum {
                        id: option,
                        args: vec![Type::I64],
                    }],
                },
            )
            .unwrap();

        assert_eq!(
            engine.resolve(&f),
            constructor(vector, crate::types::NominalTypeKind::Struct)
        );
        assert_eq!(
            engine.resolve(&g),
            constructor(option, crate::types::NominalTypeKind::Enum)
        );
        assert_eq!(engine.resolve(&a), Type::I64);
    }

    #[test]
    fn constructor_variable_infers_a_rigid_partial_application() {
        let result = test_def(103);
        let mut env = TypeNormalizationEnv::new();
        env.register_nominal(result, crate::types::NominalTypeKind::Enum, 2);
        let mut engine = InferenceEngine::new();
        engine.set_normalization_env(env);
        let f = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));
        let a = engine.fresh_type_var();

        engine
            .unify(
                &apply(f.clone(), vec![a.clone()]),
                &Type::Enum {
                    id: result,
                    args: vec![Type::Bool, Type::I64],
                },
            )
            .unwrap();

        assert_eq!(
            engine.resolve(&f),
            Type::Lambda {
                params: vec![Kind::Type],
                body: Box::new(Type::Enum {
                    id: result,
                    args: vec![
                        Type::BoundVar {
                            depth: 0,
                            index: 0,
                            kind: Kind::Type,
                        },
                        Type::I64
                    ],
                }),
            }
        );
        assert_eq!(engine.resolve(&a), Type::Bool);
    }

    #[test]
    fn constructor_variable_matches_a_unique_rigid_section() {
        let result = test_def(104);
        let mut env = TypeNormalizationEnv::new();
        env.register_nominal(result, crate::types::NominalTypeKind::Enum, 2);
        let mut engine = InferenceEngine::new();
        engine.set_normalization_env(env);
        let f = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));
        let a = engine.fresh_type_var();

        engine
            .unify(
                &apply(f.clone(), vec![a.clone()]),
                &Type::Enum {
                    id: result,
                    args: vec![Type::I64, Type::Bool],
                },
            )
            .unwrap();

        assert!(matches!(engine.resolve(&f), Type::Lambda { .. }));
        assert_eq!(engine.resolve(&a), Type::I64);
    }

    #[test]
    fn repeated_constructor_hole_mismatch_is_rejected() {
        let result = test_def(106);
        let mut env = TypeNormalizationEnv::new();
        env.register_nominal(result, crate::types::NominalTypeKind::Enum, 2);
        let mut engine = InferenceEngine::new();
        engine.set_normalization_env(env);
        let constructor = engine
            .fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::arrow(Kind::Type, Kind::Type)));
        let hole = engine.fresh_type_var();

        let error = engine
            .unify(
                &apply(constructor.clone(), vec![hole.clone(), hole]),
                &Type::Enum {
                    id: result,
                    args: vec![Type::I64, Type::Bool],
                },
            )
            .unwrap_err();

        assert!(error.contains("cannot infer a type constructor"));
        assert!(matches!(engine.resolve(&constructor), Type::TypeVar(_)));
    }

    #[test]
    fn constructor_application_arity_mismatch_is_rejected() {
        let result = test_def(107);
        let mut env = TypeNormalizationEnv::new();
        env.register_nominal(result, crate::types::NominalTypeKind::Enum, 0);
        let mut engine = InferenceEngine::new();
        engine.set_normalization_env(env);
        let constructor = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));

        let error = engine
            .unify(
                &apply(constructor.clone(), vec![Type::I64]),
                &Type::Enum {
                    id: result,
                    args: Vec::new(),
                },
            )
            .unwrap_err();

        assert!(error.contains("cannot infer a type constructor"));
        assert!(matches!(engine.resolve(&constructor), Type::TypeVar(_)));
    }

    #[test]
    fn explicit_constructor_lambda_beta_reduces_before_unification() {
        let result = test_def(105);
        let mut env = TypeNormalizationEnv::new();
        env.register_nominal(result, crate::types::NominalTypeKind::Enum, 2);
        let mut engine = InferenceEngine::new();
        engine.set_normalization_env(env);
        let f = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));
        let a = engine.fresh_type_var();
        let e = engine.fresh_type_var();
        let section = Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::Enum {
                id: result,
                args: vec![
                    Type::BoundVar {
                        depth: 0,
                        index: 0,
                        kind: Kind::Type,
                    },
                    e.clone(),
                ],
            }),
        };

        engine.unify(&f, &section).unwrap();
        engine
            .unify(
                &apply(f, vec![a.clone()]),
                &Type::Enum {
                    id: result,
                    args: vec![a, e],
                },
            )
            .unwrap();
    }

    #[test]
    fn constructor_unification_checks_kinds_occurs_and_binder_escape() {
        let mut engine = InferenceEngine::new();
        let unary = Kind::arrow(Kind::Type, Kind::Type);
        let f = engine.fresh_type_var_of_kind(unary.clone());
        assert!(engine
            .unify(&f, &Type::I64)
            .unwrap_err()
            .contains("kind mismatch"));

        let recursive = engine.fresh_type_var_of_kind(unary);
        let recursive_lambda = Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::Tuple(vec![apply(
                recursive.clone(),
                vec![Type::BoundVar {
                    depth: 0,
                    index: 0,
                    kind: Kind::Type,
                }],
            )])),
        };
        assert!(engine
            .unify(&recursive, &recursive_lambda)
            .unwrap_err()
            .contains("Infinite type"));

        let ordinary = engine.fresh_type_var();
        assert!(engine
            .unify(
                &ordinary,
                &Type::BoundVar {
                    depth: 0,
                    index: 0,
                    kind: Kind::Type,
                }
            )
            .unwrap_err()
            .contains("binder escapes"));
    }

    #[test]
    fn constructor_variables_are_not_numeric_defaulted_and_finalize_strictly() {
        let mut engine = InferenceEngine::new();
        let constructor_var = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));
        let Type::TypeVar(id) = constructor_var else {
            unreachable!();
        };
        let mut constraints = ConstraintStore::new();
        constraints.add_int_literal(id, Span::test());

        engine.apply_numeric_defaults(&constraints);
        assert_eq!(engine.resolve(&Type::TypeVar(id)), Type::TypeVar(id));

        let mut errors = Vec::new();
        assert_eq!(
            engine.finalize_strict(&Type::TypeVar(id), &mut errors),
            Type::Error
        );
        assert!(errors[0].contains("ambiguous type constructor"));
    }

    #[test]
    fn probe_progress_does_not_mutate_production_generation() {
        let mut engine = InferenceEngine::new();
        let variable = engine.fresh_type_var();
        let before = engine.substitution_generation();
        let mut probe = engine.clone_for_probe();
        probe.unify(&variable, &Type::I64).unwrap();

        assert_eq!(engine.substitution_generation(), before);
        assert!(matches!(engine.resolve(&variable), Type::TypeVar(_)));
        engine.commit_probe(probe);
        assert!(engine.substitution_generation() > before);
        assert_eq!(engine.resolve(&variable), Type::I64);
    }

    #[test]
    fn committing_probes_preserves_unobserved_worklist_notifications() {
        let mut engine = InferenceEngine::new();
        let first = engine.fresh_type_var();
        let second = engine.fresh_type_var();
        engine.unify(&first, &Type::I64).unwrap();
        let mut probe = engine.clone_for_probe();
        probe.unify(&second, &Type::Bool).unwrap();
        engine.commit_probe(probe);
        // Callable propagation can commit an unchanged probe after completing a
        // nested coercion, before the authority worklist drains its events.
        engine.commit_probe(engine.clone_for_probe());
        assert_eq!(
            engine.take_changed_type_vars(),
            vec![TypeVarId(0), TypeVarId(1)]
        );
        assert_eq!(engine.resolve(&first), Type::I64);
        assert_eq!(engine.resolve(&second), Type::Bool);
    }

    #[test]
    fn fresh_type_var_returns_typed_type_var_id() {
        let mut engine = InferenceEngine::new();

        assert_eq!(engine.fresh_type_var(), Type::TypeVar(TypeVarId(0)));
        assert_eq!(engine.fresh_type_var(), Type::TypeVar(TypeVarId(1)));
    }

    #[test]
    fn fresh_type_var_at_records_span_by_typed_type_var_id() {
        let mut engine = InferenceEngine::new();
        let span = Span::test();

        assert_eq!(
            engine.fresh_type_var_at(span.clone()),
            Type::TypeVar(TypeVarId(0))
        );
        assert_eq!(engine.var_spans.get(&TypeVarId(0)), Some(&span));
    }

    #[test]
    fn numeric_defaults_do_not_default_marker_bound_type_variables() {
        let mut engine = InferenceEngine::new();
        let marker_bound = TraitBound {
            trait_id: DefId::new(CrateId(0), LocalDefId(1)),
            type_args: Vec::new(),
        };
        let ty = engine.fresh_type_var();
        let Type::TypeVar(var_id) = ty else {
            panic!("fresh type variable must be a type variable");
        };
        engine.add_bound(var_id, marker_bound);

        engine.apply_numeric_defaults(&crate::infer::constraints::ConstraintStore::new());

        assert_eq!(engine.resolve(&ty), ty);
    }

    #[test]
    fn numeric_defaults_apply_integer_literal_evidence() {
        let mut engine = InferenceEngine::new();
        let ty = engine.fresh_type_var();
        let Type::TypeVar(var_id) = ty else {
            panic!("fresh type variable must be a type variable");
        };
        let mut constraints = crate::infer::constraints::ConstraintStore::new();
        constraints.add_int_literal(var_id, Span::test());

        engine.apply_numeric_defaults(&constraints);

        assert_eq!(engine.resolve(&ty), Type::I64);
    }

    #[test]
    fn numeric_defaults_apply_integer_evidence_to_the_entire_alias_class() {
        for unify_literal_with_alias in [true, false] {
            let mut engine = InferenceEngine::new();
            let literal = engine.fresh_type_var();
            let alias = engine.fresh_type_var();
            let (Type::TypeVar(literal_id), Type::TypeVar(_)) = (&literal, &alias) else {
                panic!("fresh type variables must be type variables");
            };
            let mut constraints = crate::infer::constraints::ConstraintStore::new();
            constraints.add_int_literal(*literal_id, Span::test());

            if unify_literal_with_alias {
                engine.unify(&literal, &alias).unwrap();
            } else {
                engine.unify(&alias, &literal).unwrap();
            }
            engine.apply_numeric_defaults(&constraints);

            assert_eq!(engine.resolve(&literal), Type::I64);
            assert_eq!(engine.resolve(&alias), Type::I64);
        }
    }

    #[test]
    fn numeric_defaults_apply_float_evidence_to_the_entire_alias_class() {
        for unify_literal_with_alias in [true, false] {
            let mut engine = InferenceEngine::new();
            let literal = engine.fresh_type_var();
            let alias = engine.fresh_type_var();
            let (Type::TypeVar(literal_id), Type::TypeVar(_)) = (&literal, &alias) else {
                panic!("fresh type variables must be type variables");
            };
            let mut constraints = crate::infer::constraints::ConstraintStore::new();
            constraints.add_float_literal(*literal_id, Span::test());

            if unify_literal_with_alias {
                engine.unify(&literal, &alias).unwrap();
            } else {
                engine.unify(&alias, &literal).unwrap();
            }
            engine.apply_numeric_defaults(&constraints);

            assert_eq!(engine.resolve(&literal), Type::F64);
            assert_eq!(engine.resolve(&alias), Type::F64);
        }
    }

    #[test]
    fn numeric_defaults_are_stable_for_same_kind_evidence_across_aliases() {
        for unify_first_with_second in [true, false] {
            let mut engine = InferenceEngine::new();
            let first = engine.fresh_type_var();
            let second = engine.fresh_type_var();
            let (Type::TypeVar(first_id), Type::TypeVar(second_id)) = (&first, &second) else {
                panic!("fresh type variables must be type variables");
            };
            let mut constraints = crate::infer::constraints::ConstraintStore::new();
            constraints.add_int_literal(*first_id, Span::test());
            constraints.add_int_literal(*second_id, Span::test());

            if unify_first_with_second {
                engine.unify(&first, &second).unwrap();
            } else {
                engine.unify(&second, &first).unwrap();
            }
            engine.apply_numeric_defaults(&constraints);

            assert_eq!(engine.resolve(&first), Type::I64);
            assert_eq!(engine.resolve(&second), Type::I64);
        }
    }

    #[test]
    fn numeric_defaults_leave_mixed_literal_evidence_across_aliases_unresolved() {
        for unify_integer_with_float in [true, false] {
            let mut engine = InferenceEngine::new();
            let integer = engine.fresh_type_var();
            let float = engine.fresh_type_var();
            let (Type::TypeVar(integer_id), Type::TypeVar(float_id)) = (&integer, &float) else {
                panic!("fresh type variables must be type variables");
            };
            let mut constraints = crate::infer::constraints::ConstraintStore::new();
            constraints.add_int_literal(*integer_id, Span::test());
            constraints.add_float_literal(*float_id, Span::test());

            if unify_integer_with_float {
                engine.unify(&integer, &float).unwrap();
            } else {
                engine.unify(&float, &integer).unwrap();
            }
            engine.apply_numeric_defaults(&constraints);

            let integer_resolved = engine.resolve(&integer);
            let float_resolved = engine.resolve(&float);
            assert!(matches!(integer_resolved, Type::TypeVar(_)));
            assert_eq!(integer_resolved, float_resolved);
        }
    }

    #[test]
    fn numeric_defaults_do_not_use_marker_bounds_from_aliases() {
        let mut engine = InferenceEngine::new();
        let bounded = engine.fresh_type_var();
        let alias = engine.fresh_type_var();
        let (Type::TypeVar(bounded_id), Type::TypeVar(_)) = (&bounded, &alias) else {
            panic!("fresh type variables must be type variables");
        };
        engine.add_bound(
            *bounded_id,
            TraitBound {
                trait_id: DefId::new(CrateId(0), LocalDefId(2)),
                type_args: Vec::new(),
            },
        );
        engine.unify(&bounded, &alias).unwrap();

        engine.apply_numeric_defaults(&crate::infer::constraints::ConstraintStore::new());

        let bounded_resolved = engine.resolve(&bounded);
        assert!(matches!(bounded_resolved, Type::TypeVar(_)));
        assert_eq!(bounded_resolved, engine.resolve(&alias));
    }

    #[test]
    fn strict_finalization_rejects_unspanned_type_var() {
        let mut engine = InferenceEngine::new();
        let unspanned = engine.fresh_type_var();
        let mut errors = Vec::new();

        assert_eq!(engine.finalize_strict(&unspanned, &mut errors), Type::Error);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("ambiguous type"));
    }

    #[test]
    fn strict_finalization_rejects_unresolved_alias_class() {
        let mut engine = InferenceEngine::new();
        let origin = Span {
            file_path: "main.rk".into(),
            start: 12,
            end: 17,
        };
        let spanned = engine.fresh_type_var_at(origin.clone());
        let unspanned = engine.fresh_type_var();
        engine.unify(&spanned, &unspanned).unwrap();

        let mut errors = Vec::new();
        assert_eq!(engine.finalize_strict(&spanned, &mut errors), Type::Error);
        assert_eq!(errors.len(), 1);
        let span = errors[0].span.as_ref().expect("origin span");
        assert_eq!(span.file_path, origin.file_path);
        assert_eq!((span.start, span.end), (origin.start, origin.end));

        let mut engine = InferenceEngine::new();
        let spanned = engine.fresh_type_var_at(origin.clone());
        let unspanned = engine.fresh_type_var();
        engine.unify(&unspanned, &spanned).unwrap();

        let mut errors = Vec::new();
        assert_eq!(engine.finalize_strict(&unspanned, &mut errors), Type::Error);
        assert_eq!(errors.len(), 1);
        let span = errors[0].span.as_ref().expect("origin span");
        assert_eq!(span.file_path, origin.file_path);
        assert_eq!((span.start, span.end), (origin.start, origin.end));
    }

    #[test]
    fn same_crate_nominal_struct_ids_do_not_unify() {
        let mut engine = InferenceEngine::new();
        let first = Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(1)),
            args: vec![Type::I64],
        };
        let second = Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(2)),
            args: vec![Type::I64],
        };

        assert!(engine.unify(&first, &second).is_err());
    }

    #[test]
    fn function_unification_uses_contravariant_params_for_safety() {
        let mut engine = InferenceEngine::new();
        let accepts_safe = Type::function(vec![Type::function(Vec::new(), Type::I64)], Type::I64);
        let accepts_unsafe = Type::function(
            vec![Type::unsafe_function(Vec::new(), Type::I64)],
            Type::I64,
        );

        assert!(engine.unify(&accepts_safe, &accepts_unsafe).is_err());

        let mut engine = InferenceEngine::new();
        assert!(engine.unify(&accepts_unsafe, &accepts_safe).is_ok());
    }

    #[test]
    fn function_unification_preserves_closure_metadata_through_type_variables() {
        let mut engine = InferenceEngine::new();
        let variable = engine.fresh_type_var();
        let default_signature = Type::function(vec![Type::I64], Type::Bool);
        let closure = Type::function_with_metadata(
            vec![Type::I64],
            Type::Bool,
            crate::types::FunctionSafety::Safe,
            crate::types::CallableKind::FnMut,
            vec![crate::types::FunctionCapture::new(
                crate::types::CaptureKind::MutableBorrow,
                Type::I64,
            )],
        );

        engine.unify(&variable, &default_signature).unwrap();
        engine.unify(&variable, &closure).unwrap();

        assert_eq!(engine.resolve(&variable), closure);
    }

    #[test]
    fn function_signature_unification_treats_default_metadata_as_wildcard() {
        let default_signature = Type::function(Vec::new(), Type::I64);
        let closure = Type::function_with_metadata(
            Vec::new(),
            Type::I64,
            crate::types::FunctionSafety::Safe,
            crate::types::CallableKind::FnOnce,
            vec![crate::types::FunctionCapture::new(
                crate::types::CaptureKind::Move,
                Type::I64,
            )],
        );
        let mut engine = InferenceEngine::new();

        assert!(engine.unify(&default_signature, &closure).is_ok());

        let mut engine = InferenceEngine::new();
        assert!(engine
            .unify(
                &Type::Reference {
                    mutable: true,
                    inner: Box::new(default_signature),
                },
                &Type::Reference {
                    mutable: true,
                    inner: Box::new(closure),
                },
            )
            .is_ok());
    }

    #[test]
    fn mutable_ref_and_pointer_unification_are_invariant_for_safety() {
        let safe_fn = Type::function(Vec::new(), Type::I64);
        let unsafe_fn = Type::unsafe_function(Vec::new(), Type::I64);

        let mut engine = InferenceEngine::new();
        assert!(engine
            .unify(
                &Type::Reference {
                    mutable: true,
                    inner: Box::new(safe_fn.clone()),
                },
                &Type::Reference {
                    mutable: true,
                    inner: Box::new(unsafe_fn.clone()),
                },
            )
            .is_err());

        let mut engine = InferenceEngine::new();
        assert!(engine
            .unify(
                &Type::Pointer(Box::new(safe_fn)),
                &Type::Pointer(Box::new(unsafe_fn)),
            )
            .is_err());
    }

    #[test]
    fn mutable_ref_and_pointer_unification_are_invariant_for_integer_types() {
        let mut engine = InferenceEngine::new();
        assert!(engine
            .unify(
                &Type::Reference {
                    mutable: true,
                    inner: Box::new(Type::I8),
                },
                &Type::Reference {
                    mutable: true,
                    inner: Box::new(Type::I64),
                },
            )
            .is_err());

        let mut engine = InferenceEngine::new();
        assert!(engine
            .unify(
                &Type::Pointer(Box::new(Type::I8)),
                &Type::Pointer(Box::new(Type::I64))
            )
            .is_err());
    }
}

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
            witness_scopes: HashMap::new(),
            active_witnesses: Vec::new(),
            normalization_env: TypeNormalizationEnv::new(),
            type_display_context: Default::default(),
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
            witness_scopes: HashMap::new(),
            active_witnesses: Vec::new(),
            normalization_env,
            type_display_context: Default::default(),
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
            witness_scopes: self.witness_scopes.clone(),
            active_witnesses: self.active_witnesses.clone(),
            normalization_env: self.normalization_env.clone(),
            type_display_context: self.type_display_context.clone(),
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
        // A probe owns only its new events. Committing it must not discard
        // earlier unifications that the enclosing worklist has not observed.
        let mut changed = self.take_changed_type_vars();
        changed.extend(probe.take_changed_type_vars());
        changed.sort();
        changed.dedup();
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

    pub(crate) fn enter_witness_scope(&mut self, witness: crate::types::WitnessId) {
        self.active_witnesses.push(witness);
    }

    pub(crate) fn exit_witness_scope(&mut self, witness: crate::types::WitnessId) {
        debug_assert_eq!(self.active_witnesses.pop(), Some(witness));
    }

    pub fn fresh_type_var_of_kind(&mut self, kind: Kind) -> Type {
        let id = self.next_var.fresh();
        self.witness_scopes
            .insert(id, self.active_witnesses.clone());
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
        self.witness_scopes
            .insert(id, self.active_witnesses.clone());
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
            (Type::Object(left), Type::Object(right)) => {
                super::object_unify::unify_objects(self, left, right)
            }
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
        let allowed = self.witness_scopes.get(&id).cloned().unwrap_or_default();
        if crate::type_services::visit::type_any(
            &ty,
            |nested| matches!(nested, Type::Witness(witness) if !allowed.contains(witness)),
        ) {
            return Err(UnifyError::Message(
                "opened witness escapes its inference scope; repackage the value".into(),
            ));
        }
        if crate::type_services::substitution::has_free_object_self(&ty) {
            return Err(UnifyError::ObjectBinderEscape { ty });
        }
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
        // Restrict nested unknowns too: an outer variable bound to (inner,)
        // must not acquire a witness later through the inner variable.
        let mut nested_vars = Vec::new();
        crate::type_services::visit::visit_type(&ty, &mut |nested: &Type| {
            if let Type::TypeVar(variable) = nested {
                nested_vars.push(*variable);
            }
        });
        for variable in nested_vars {
            self.witness_scopes
                .entry(variable)
                .or_default()
                .retain(|witness| allowed.contains(witness));
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
            Type::Object(object) => object
                .children()
                .any(|child| Self::has_escaping_bound_var(child, binder_depth)),
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
            Type::Object(object) => object.children().any(|child| self.occurs_in(var, child)),
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

#[cfg(test)]
mod witness_scope_tests {
    use super::InferenceEngine;
    use crate::ids::{CrateId, DefId, HirLocalId, LocalDefId};
    use crate::types::{Type, WitnessId};

    fn witness(local: u32) -> WitnessId {
        WitnessId {
            owner: DefId::new(CrateId(0), LocalDefId(0)),
            local: HirLocalId(local),
        }
    }

    #[test]
    fn opened_witness_inference_is_local_and_rigid() {
        let mut engine = InferenceEngine::new();
        let outer = engine.fresh_type_var();
        let witness = witness(0);
        engine.enter_witness_scope(witness);
        let inner = engine.fresh_type_var();
        assert!(engine.unify(&outer, &Type::Witness(witness)).is_err());
        engine.unify(&inner, &Type::Witness(witness)).unwrap();
        assert_eq!(engine.resolve(&inner), Type::Witness(witness));
        assert!(engine.unify(&Type::Witness(witness), &Type::I64).is_err());
        engine.exit_witness_scope(witness);
    }

    #[test]
    fn witness_cannot_escape_later_through_a_nested_inference_alias() {
        let mut engine = InferenceEngine::new();
        let outer = engine.fresh_type_var();
        let witness = witness(0);
        engine.enter_witness_scope(witness);
        let inner = engine.fresh_type_var();
        engine
            .unify(&outer, &Type::Tuple(vec![inner.clone()]))
            .unwrap();
        assert!(engine.unify(&inner, &Type::Witness(witness)).is_err());
        engine.exit_witness_scope(witness);
    }

    #[test]
    fn different_openings_have_independent_rigid_type_identity() {
        let mut engine = InferenceEngine::new();
        assert!(engine
            .unify(&Type::Witness(witness(0)), &Type::Witness(witness(1)))
            .is_err());
    }
}
