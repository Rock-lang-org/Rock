use super::*;
use crate::ids::{CrateId, DefId, HirLocalId, LocalDefId, TypeVarId};
use crate::mono::hir_types::{HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt};
use crate::types::{ArrayLen, GenericParamDecl, GenericParamId};

fn id() -> DefId {
    DefId::new(CrateId(0), LocalDefId(1))
}

fn expr(kind: HirExprKind, ty: Type) -> HirExpr {
    HirExpr { kind, ty, span: crate::lexer::Span::test() }
}

fn unit() -> HirExpr {
    expr(HirExprKind::Unit, Type::Unit)
}

fn function(body: HirBlock) -> HirFunction {
    HirFunction {
        id: id(),
        name: "body_only".to_string(),
        generic_params: Vec::new(),
        generic_bounds: Default::default(),
        params: Vec::new(),
        ret_type: Type::Unit,
        body,
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn invalid_body_cases(len: ArrayLen) -> Vec<HirBlock> {
    let array = Type::Array(Box::new(Type::I64), len);
    let block = |stmt| HirBlock { stmts: vec![stmt], ty: Type::Unit };
    let mut cases = vec![
        HirBlock { stmts: Vec::new(), ty: array.clone() },
        block(HirStmt::Let {
            name: "local".to_string(), local_id: HirLocalId(0),
            ty: array.clone(), value: unit(), mutable: false,
        }),
        block(HirStmt::Expr(expr(HirExprKind::Unit, array.clone()))),
        block(HirStmt::Expr(expr(HirExprKind::Cast(Box::new(unit()), array.clone()), Type::Unit))),
        block(HirStmt::Expr(expr(HirExprKind::ArrayRepeat(Box::new(unit()), len), Type::Unit))),
        block(HirStmt::Expr(expr(HirExprKind::Lambda {
            params: vec![crate::hir::HirParam {
                name: "arg".to_string(), local_id: HirLocalId(1), ty: array.clone(),
                mutable: false, is_ref: false,
            }],
            body: HirBlock { stmts: Vec::new(), ty: Type::Unit },
            captures: Vec::new(),
        }, Type::Unit))),
        block(HirStmt::Expr(expr(HirExprKind::Lambda {
            params: Vec::new(),
            body: HirBlock { stmts: Vec::new(), ty: Type::Unit },
            captures: vec![crate::hir::HirClosureCapture {
                name: "capture".to_string(), local_id: HirLocalId(2),
                kind: crate::hir::HirClosureCaptureKind::Move, mutable: false, ty: array.clone(),
            }],
        }, Type::Unit))),
        block(HirStmt::Expr(expr(HirExprKind::Loop(HirBlock {
            stmts: vec![HirStmt::Break(Some(expr(HirExprKind::Unit, Type::Struct {
                id: id(), args: vec![Type::Tuple(vec![array.clone()])],
            })))],
            ty: Type::Unit,
        }), Type::Unit))),
    ];
    for location in 0..4 {
        let mut target = crate::hir::HirMethodCallTarget::trait_method(
            id(), id(), Vec::new(), crate::hir::HirTraitDispatchKind::TraitBound,
        );
        let binding = crate::hir::HirTypeBinding {
            param: GenericParamId { owner: id(), index: 0 }, ty: array.clone(),
        };
        match location {
            0 => { *target.trait_args_mut().unwrap() = vec![array.clone()]; }
            1 => target.owner_substitution.push(binding),
            2 => target.method_substitution.push(binding),
            _ => {}
        }
        if location < 3 {
            cases.push(block(HirStmt::Expr(expr(HirExprKind::MethodCall(
                Box::new(unit()), "method".to_string(), Vec::new(), None, target.clone(),
            ), Type::Unit))));
        }
        cases.push(block(HirStmt::Expr(expr(HirExprKind::Call(
            Box::new(unit()), Vec::new(), Some(crate::hir::HirCallTarget::StaticMethod(
                crate::hir::HirStaticMethodTarget {
                    owner_ty: if location == 3 { array.clone() } else { Type::Unit }, method: target,
                },
            )),
        ), Type::Unit))));
    }
    cases.push(block(HirStmt::Expr(expr(HirExprKind::Match {
        scrutinee: Box::new(unit()),
        arms: vec![crate::hir::HirMatchArmFor {
            pattern: crate::hir::HirPattern::Struct("Pattern".to_string(), Some(id()), vec![array], Vec::new()),
            guard: None,
            body: HirBlock { stmts: Vec::new(), ty: Type::Unit },
        }],
    }, Type::Unit))));
    cases
}

#[test]
fn rejects_all_symbolic_length_forms_in_body_only_types() {
    for len in [
        ArrayLen::Param(GenericParamId { owner: id(), index: 0 }),
        ArrayLen::Var(TypeVarId(7)),
        ArrayLen::Bound { depth: 0, index: 0 },
        ArrayLen::Error,
    ] {
        for (case, body) in invalid_body_cases(len).into_iter().enumerate() {
            let mut mono = Monomorphizer::new();
            mono.register_function_instance("body_only", &function(body));
            mono.check_instance_array_lengths();
            assert!(mono.diagnostics.return_if_err().is_err(), "{len:?}, case {case}");
        }
    }
}

#[test]
fn permits_concrete_body_lengths_including_zero() {
    for len in [0, 3] {
        for body in invalid_body_cases(ArrayLen::Value(len)) {
            let mut mono = Monomorphizer::new();
            mono.register_function_instance("body_only", &function(body));
            mono.check_instance_array_lengths();
            assert!(mono.diagnostics.return_if_err().is_ok());
        }
    }
}

#[test]
fn preserves_symbolic_generic_templates_outside_instance_registry() {
    let param = GenericParamId { owner: id(), index: 0 };
    let mut template = function(invalid_body_cases(ArrayLen::Param(param)).remove(0));
    template.generic_params.push(GenericParamDecl::new(param, "N", crate::type_services::kind::Kind::Usize));
    let mut mono = Monomorphizer::new();
    mono.register_generic_function("template".to_string(), template);
    mono.check_instance_array_lengths();
    assert!(mono.diagnostics.return_if_err().is_ok());
    assert!(matches!(mono.generic_functions[&id()].body.ty, Type::Array(_, ArrayLen::Param(_))));
}

#[test]
fn diagnoses_symbol_substitutions_before_using_recovery_identity() {
    let mut mono = Monomorphizer::new();
    let origin = crate::mono::InstanceOrigin::Function(id());
    let a = mono.intern_type(&Type::Array(Box::new(Type::I64), ArrayLen::Var(TypeVarId(1))));
    let b = mono.intern_type(&Type::Array(Box::new(Type::I64), ArrayLen::Var(TypeVarId(2))));
    let first = mono.backend_symbol_for_origin(&origin, &[a]);
    let second = mono.backend_symbol_for_origin(&origin, &[b]);
    assert_ne!(first, second);
    assert!(mono.diagnostics.return_if_err().is_err());
}

#[test]
fn validates_substituted_nominal_fields_reached_only_from_body() {
    for args in [Vec::new(), vec![Type::ConstValue(3)]] {
        let mut mono = Monomorphizer::new();
        mono.nominal_field_types.insert(id(), vec![Type::Array(
            Box::new(Type::I64), ArrayLen::Param(GenericParamId { owner: id(), index: 0 }),
        )]);
        let valid = !args.is_empty();
        let body = HirBlock { stmts: vec![HirStmt::Expr(expr(
            HirExprKind::Unit, Type::Struct { id: id(), args },
        ))], ty: Type::Unit };
        mono.register_function_instance("body_only", &function(body));
        mono.check_instance_array_lengths();
        assert_eq!(mono.diagnostics.return_if_err().is_ok(), valid);
    }
}

#[test]
fn constructor_binders_remain_symbolic_but_free_bound_lengths_are_rejected() {
    let array = Type::Array(Box::new(Type::I64), ArrayLen::Bound { depth: 0, index: 0 });
    assert!(unresolved_array_length(&array));
    assert!(!unresolved_array_length(&Type::Lambda { params: vec![crate::type_services::kind::Kind::Usize], body: Box::new(array) }));
}

#[test]
fn distinct_bound_constructor_lengths_have_distinct_symbol_identities() {
    let mut mono = Monomorphizer::new();
    let types = (0..2).map(|index| Type::Lambda {
        params: vec![crate::type_services::kind::Kind::Usize; 2],
        body: Box::new(Type::Array(Box::new(Type::I64), ArrayLen::Bound { depth: 0, index })),
    }).collect::<Vec<_>>();
    let ids = types.iter().map(|ty| mono.intern_type(ty)).collect::<Vec<_>>();
    let origin = crate::mono::InstanceOrigin::Function(id());
    let first = mono.backend_symbol_for_origin(&origin, &ids[..1]);
    let second = mono.backend_symbol_for_origin(&origin, &ids[1..]);
    assert_ne!(first, second);
    assert!(mono.diagnostics.return_if_err().is_ok());
}

#[test]
fn rejects_invalid_substitution_on_object_provided_instance_without_body() {
    let mut mono = Monomorphizer::new();
    let ty = mono.intern_type(&Type::Array(Box::new(Type::I64), ArrayLen::Error));
    let origin = crate::mono::InstanceOrigin::Function(id());
    mono.instances.intern(crate::mono::InstanceKey::new(origin.clone(), vec![ty]), |instance| {
        crate::mono::InstanceRecord::new(
            instance, origin, vec![ty], crate::mono::InstanceSymbols::new("external", "external"),
            None, true, false,
        )
    });
    mono.check_instance_array_lengths();
    assert!(mono.diagnostics.return_if_err().is_err());
}
