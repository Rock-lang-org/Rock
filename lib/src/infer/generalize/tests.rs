use super::*;

use crate::hir::{HirBlock, HirExpr, HirExprKind, HirParam, HirStmt};
use crate::ids::{CrateId, DefId, LocalDefId, TypeVarId};

fn def_id(local: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(local))
}

fn function_with_type_vars(id: DefId, var_order: &[TypeVarId]) -> HirFunction {
    HirFunction {
        id,
        name: "pair".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: var_order
            .iter()
            .enumerate()
            .map(|(index, var_id)| HirParam {
                name: format!("param{index}"),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::TypeVar(*var_id),
                mutable: false,
                is_ref: false,
            })
            .collect(),
        ret_type: Type::TypeVar(var_order[0]),
        body: HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::Var("param0".to_string()),
                ty: Type::TypeVar(var_order[0]),
                span: crate::lexer::Span::test(),
            })],
            ty: Type::TypeVar(var_order[0]),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

#[test]
fn generalize_assigns_generic_ids_in_signature_order() {
    let engine = InferenceEngine::new();
    let id = def_id(40);
    let var_order = [
        TypeVarId(6),
        TypeVarId(5),
        TypeVarId(4),
        TypeVarId(3),
        TypeVarId(2),
        TypeVarId(1),
        TypeVarId(0),
    ];
    let func = function_with_type_vars(id, &var_order);
    let function_type_vars = HashMap::from([(id, var_order.iter().copied().collect())]);

    let generalized = generalize_single_function(
        &engine,
        &ConstraintStore::default(),
        &function_type_vars,
        func,
    );

    assert_eq!(
        generalized
            .generic_params
            .iter()
            .map(|param| param.name.as_str())
            .collect::<Vec<_>>(),
        vec!["T", "U", "V", "W", "X", "Y", "Z"]
    );
    for (expected_index, param) in generalized.params.iter().enumerate() {
        assert_eq!(
            param.ty,
            Type::Generic(GenericParamId {
                owner: id,
                index: expected_index as u32,
            })
        );
    }
    assert_eq!(
        generalized.ret_type,
        Type::Generic(GenericParamId {
            owner: id,
            index: 0,
        })
    );
}

#[test]
fn generalize_discovers_type_vars_nested_in_resolved_signature_types() {
    let id = def_id(41);
    let header_var = TypeVarId(0);
    let nested_var = TypeVarId(1);
    let body_alias = TypeVarId(2);
    let option_id = def_id(42);
    let option_ty = Type::Enum {
        id: option_id,
        args: vec![Type::TypeVar(nested_var)],
    };
    let mut engine = InferenceEngine::new();
    engine
        .unify(&Type::TypeVar(header_var), &option_ty)
        .unwrap();
    engine
        .unify(&Type::TypeVar(body_alias), &Type::TypeVar(nested_var))
        .unwrap();
    let mut function = function_with_type_vars(id, &[header_var]);
    function.params.clear();
    function.ret_type = Type::TypeVar(header_var);
    function.body.ty = Type::Enum {
        id: option_id,
        args: vec![Type::TypeVar(body_alias)],
    };
    let function_type_vars = HashMap::from([(id, HashSet::from([header_var]))]);

    let generalized = generalize_single_function(
        &engine,
        &ConstraintStore::default(),
        &function_type_vars,
        function,
    );
    let generic = Type::Generic(GenericParamId {
        owner: id,
        index: 0,
    });

    assert_eq!(
        generalized
            .generic_params
            .iter()
            .map(|param| param.name.as_str())
            .collect::<Vec<_>>(),
        vec!["T"]
    );
    assert_eq!(
        generalized.ret_type,
        Type::Enum {
            id: option_id,
            args: vec![generic.clone()],
        }
    );
    assert_eq!(
        generalized.body.ty,
        Type::Enum {
            id: option_id,
            args: vec![generic],
        }
    );
}

#[test]
fn generalize_transports_inferred_trait_bounds_to_generic_parameters() {
    let id = def_id(43);
    let trait_id = def_id(44);
    let var = TypeVarId(0);
    let mut engine = InferenceEngine::new();
    engine.add_bound(
        var,
        crate::types::TraitBound {
            trait_id,
            type_args: vec![Type::I64],
        },
    );
    let function = function_with_type_vars(id, &[var]);
    let function_type_vars = HashMap::from([(id, HashSet::from([var]))]);

    let generalized = generalize_single_function(
        &engine,
        &ConstraintStore::default(),
        &function_type_vars,
        function,
    );
    let generic = GenericParamId {
        owner: id,
        index: 0,
    };

    assert_eq!(
        generalized.generic_bounds.get(&generic),
        Some(&vec![crate::types::TraitBound {
            trait_id,
            type_args: vec![Type::I64],
        }])
    );
}

#[test]
fn generalize_preserves_constructor_variable_kinds() {
    let id = def_id(44);
    let mut engine = InferenceEngine::new();
    let variable = engine.fresh_type_var_of_kind(crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    ));
    let Type::TypeVar(var_id) = variable.clone() else {
        unreachable!();
    };
    let applied = Type::Apply {
        constructor: Box::new(variable),
        args: vec![Type::I64],
    };
    let mut function = function_with_type_vars(id, &[var_id]);
    function.params[0].ty = applied.clone();
    function.ret_type = applied;
    let function_type_vars = HashMap::from([(id, HashSet::from([var_id]))]);

    let generalized = generalize_single_function(
        &engine,
        &ConstraintStore::default(),
        &function_type_vars,
        function,
    );

    assert_eq!(
        generalized.generic_params[0].kind,
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        )
    );
    assert_eq!(
        generalized.params[0].ty,
        Type::Apply {
            constructor: Box::new(Type::Generic(GenericParamId {
                owner: id,
                index: 0,
            })),
            args: vec![Type::I64],
        }
    );
}

#[test]
fn generalize_preserves_literal_backed_signature_vars_for_defaulting() {
    let id = def_id(43);
    let var = TypeVarId(0);
    let engine = InferenceEngine::new();
    let function = function_with_type_vars(id, &[var]);
    let function_type_vars = HashMap::from([(id, HashSet::from([var]))]);
    let mut constraints = ConstraintStore::default();
    constraints.add_int_literal(var, crate::lexer::Span::test());

    let generalized =
        generalize_single_function(&engine, &constraints, &function_type_vars, function);

    assert!(generalized.generic_params.is_empty());
    assert_eq!(generalized.ret_type, Type::TypeVar(var));
    assert_eq!(generalized.body.ty, Type::TypeVar(var));
}
