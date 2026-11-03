use super::*;
use crate::ids::{CrateId, LocalDefId};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn constructor(id: DefId, flavor: NominalTypeKind) -> Type {
    Type::Constructor { id, flavor }
}

fn bound(index: u32) -> Type {
    Type::BoundVar {
        depth: 0,
        index,
        kind: Kind::Type,
    }
}

fn unary_lambda(body: Type) -> Type {
    Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(body),
    }
}

#[test]
fn nominal_application_preserves_kind_and_saturates() {
    let id = def_id(1);
    let unit_struct = def_id(13);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(id, NominalTypeKind::Struct, 2);
    env.register_nominal(unit_struct, NominalTypeKind::Struct, 0);
    let partial = TypeNormalizer::new(&env)
        .normalize(&Type::Apply {
            constructor: Box::new(constructor(id, NominalTypeKind::Struct)),
            args: vec![Type::I64],
        })
        .unwrap();
    assert_eq!(
        TypeNormalizer::new(&env).kind_of(&partial).unwrap(),
        Kind::arrow(Kind::Type, Kind::Type)
    );
    let saturated = TypeNormalizer::new(&env)
        .normalize(&Type::Apply {
            constructor: Box::new(partial),
            args: vec![Type::Bool],
        })
        .unwrap();
    assert_eq!(
        saturated,
        Type::Struct {
            id,
            args: vec![Type::I64, Type::Bool]
        }
    );
    assert_eq!(
        TypeNormalizer::new(&env)
            .normalize(&constructor(unit_struct, NominalTypeKind::Struct))
            .unwrap(),
        Type::Struct {
            id: unit_struct,
            args: Vec::new(),
        }
    );
}

#[test]
fn sections_bind_holes_left_to_right_and_eta_reduce() {
    let result = def_id(2);
    let option = def_id(3);
    let ternary = def_id(10);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(result, NominalTypeKind::Enum, 2);
    env.register_nominal(option, NominalTypeKind::Enum, 1);
    env.register_nominal(ternary, NominalTypeKind::Struct, 3);

    let section = TypeNormalizer::new(&env)
        .desugar_section(
            constructor(result, NominalTypeKind::Enum),
            vec![SectionArg::Hole(Kind::Type), SectionArg::Type(Type::I64)],
        )
        .unwrap();
    assert_eq!(
        section,
        unary_lambda(Type::Enum {
            id: result,
            args: vec![bound(0), Type::I64],
        })
    );

    let eta = TypeNormalizer::new(&env)
        .desugar_section(
            constructor(option, NominalTypeKind::Enum),
            vec![SectionArg::Hole(Kind::Type)],
        )
        .unwrap();
    assert_eq!(eta, constructor(option, NominalTypeKind::Enum));

    let multi_hole = TypeNormalizer::new(&env)
        .desugar_section(
            constructor(ternary, NominalTypeKind::Struct),
            vec![
                SectionArg::Hole(Kind::Type),
                SectionArg::Type(Type::I64),
                SectionArg::Hole(Kind::Type),
            ],
        )
        .unwrap();
    assert_eq!(
        multi_hole,
        Type::Lambda {
            params: vec![Kind::Type, Kind::Type],
            body: Box::new(Type::Struct {
                id: ternary,
                args: vec![bound(0), Type::I64, bound(1)],
            }),
        }
    );
}

#[test]
fn beta_reduction_handles_partial_exact_and_extra_application() {
    let option = def_id(4);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(option, NominalTypeKind::Enum, 1);
    let lambda = Type::Lambda {
        params: vec![Kind::Type, Kind::Type],
        body: Box::new(Type::Tuple(vec![bound(0), bound(1)])),
    };
    let partial = TypeNormalizer::new(&env)
        .normalize(&Type::Apply {
            constructor: Box::new(lambda.clone()),
            args: vec![Type::I64],
        })
        .unwrap();
    assert_eq!(
        partial,
        unary_lambda(Type::Tuple(vec![Type::I64, bound(0)]))
    );
    let exact = TypeNormalizer::new(&env)
        .normalize(&Type::Apply {
            constructor: Box::new(lambda),
            args: vec![Type::I64, Type::Bool],
        })
        .unwrap();
    assert_eq!(exact, Type::Tuple(vec![Type::I64, Type::Bool]));

    let returning_constructor = unary_lambda(constructor(option, NominalTypeKind::Enum));
    let extra = TypeNormalizer::new(&env)
        .normalize(&Type::Apply {
            constructor: Box::new(returning_constructor),
            args: vec![Type::Unit, Type::I64],
        })
        .unwrap();
    assert_eq!(
        extra,
        Type::Enum {
            id: option,
            args: vec![Type::I64]
        }
    );
}

#[test]
fn substitution_shifts_bound_variables_to_avoid_capture() {
    let outer = Type::BoundVar {
        depth: 0,
        index: 0,
        kind: Kind::Type,
    };
    let lambda = unary_lambda(Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(Type::BoundVar {
            depth: 1,
            index: 0,
            kind: Kind::Type,
        }),
    });
    let env = TypeNormalizationEnv::new();
    let normalized = TypeNormalizer::new(&env)
        .normalize(&Type::Apply {
            constructor: Box::new(lambda),
            args: vec![outer],
        })
        .unwrap();
    assert_eq!(
        normalized,
        Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::BoundVar {
                depth: 1,
                index: 0,
                kind: Kind::Type,
            }),
        }
    );
}

#[test]
fn alpha_equivalent_lambdas_are_structurally_equal_and_eta_reduce() {
    let option = def_id(5);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(option, NominalTypeKind::Enum, 1);
    let first = unary_lambda(Type::Apply {
        constructor: Box::new(constructor(option, NominalTypeKind::Enum)),
        args: vec![bound(0)],
    });
    let second = first.clone();
    assert_eq!(first, second);
    assert_eq!(
        TypeNormalizer::new(&env).normalize(&first).unwrap(),
        constructor(option, NominalTypeKind::Enum)
    );
}

#[test]
fn nested_applications_and_constructor_composition_normalize_once() {
    let outer = def_id(6);
    let inner = def_id(7);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(outer, NominalTypeKind::Struct, 1);
    env.register_nominal(inner, NominalTypeKind::Enum, 1);
    let nested = Type::Apply {
        constructor: Box::new(constructor(outer, NominalTypeKind::Struct)),
        args: vec![Type::Apply {
            constructor: Box::new(constructor(inner, NominalTypeKind::Enum)),
            args: vec![Type::I64],
        }],
    };
    let normalized = TypeNormalizer::new(&env).normalize(&nested).unwrap();
    assert_eq!(
        normalized,
        Type::Struct {
            id: outer,
            args: vec![Type::Enum {
                id: inner,
                args: vec![Type::I64]
            }]
        }
    );
    assert_eq!(
        TypeNormalizer::new(&env).normalize(&normalized).unwrap(),
        normalized
    );
}

#[test]
fn alias_cycles_and_limits_are_fatal() {
    let a = def_id(8);
    let b = def_id(9);
    let mut env = TypeNormalizationEnv::new();
    env.register_alias(a, Kind::Type, constructor(b, NominalTypeKind::Alias));
    env.register_alias(b, Kind::Type, constructor(a, NominalTypeKind::Alias));
    assert!(matches!(
        TypeNormalizer::new(&env).normalize(&constructor(a, NominalTypeKind::Alias)),
        Err(NormalizeError::AliasCycle(_))
    ));

    let limits = NormalizationLimits {
        max_depth: 1,
        max_nodes: 100,
    };
    assert!(matches!(
        TypeNormalizer::with_limits(&TypeNormalizationEnv::new(), limits)
            .normalize(&Type::Slice(Box::new(Type::Slice(Box::new(Type::I64))))),
        Err(NormalizeError::DepthLimit { .. })
    ));
    let limits = NormalizationLimits {
        max_depth: 100,
        max_nodes: 1,
    };
    assert!(matches!(
        TypeNormalizer::with_limits(&TypeNormalizationEnv::new(), limits)
            .normalize(&Type::Tuple(vec![Type::I64])),
        Err(NormalizeError::NodeLimit { .. })
    ));
}

#[test]
fn aliases_expand_and_application_kind_mismatches_are_rejected() {
    let option = def_id(11);
    let alias = def_id(12);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(option, NominalTypeKind::Enum, 1);
    env.register_alias(
        alias,
        Kind::arrow(Kind::Type, Kind::Type),
        constructor(option, NominalTypeKind::Enum),
    );

    let expanded = TypeNormalizer::new(&env)
        .normalize(&Type::Apply {
            constructor: Box::new(constructor(alias, NominalTypeKind::Alias)),
            args: vec![Type::Bool],
        })
        .unwrap();
    assert_eq!(
        expanded,
        Type::Enum {
            id: option,
            args: vec![Type::Bool],
        }
    );

    let higher_kind = Kind::arrow(Kind::Type, Kind::Type);
    let mismatch = TypeNormalizer::new(&env).normalize(&Type::Apply {
        constructor: Box::new(constructor(option, NominalTypeKind::Enum)),
        args: vec![Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            }),
        }],
    });
    assert!(matches!(
        mismatch,
        Err(NormalizeError::KindMismatch {
            expected: Kind::Type,
            actual,
        }) if actual == higher_kind
    ));
}
