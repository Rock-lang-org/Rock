use crate::hir::{
    HirBlock, HirCallTarget, HirExpr, HirExprKind, HirMatchArm, HirMethodCallTarget, HirPattern,
    HirStaticMethodTarget, HirStmt, HirTypeBinding,
};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::lower::Lowerer;
use crate::types::{GenericParamId, Type};

#[test]
fn resolve_all_types_in_expr_resolves_match_arm_guards() {
    let mut lowerer = Lowerer::new_for_test();
    let guard_ty = lowerer.engine.fresh_type_var();
    lowerer.engine.unify(&guard_ty, &Type::Bool).unwrap();
    let mut expr = HirExpr {
        kind: HirExprKind::Match {
            scrutinee: Box::new(HirExpr {
                kind: HirExprKind::IntLiteral(0),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }),
            arms: vec![HirMatchArm {
                pattern: HirPattern::Wildcard,
                guard: Some(HirExpr {
                    kind: HirExprKind::BoolLiteral(true),
                    ty: guard_ty,
                    span: crate::lexer::Span::test(),
                }),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::IntLiteral(1),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::I64,
                },
            }],
        },
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    lowerer.resolve_all_types_in_expr(&mut expr);

    let HirExprKind::Match { arms, .. } = &expr.kind else {
        panic!("expected match expression");
    };
    assert_eq!(arms[0].guard.as_ref().unwrap().ty, Type::Bool);
}

#[test]
fn resolve_all_types_in_expr_resolves_method_target_trait_args() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_arg = lowerer.engine.fresh_type_var();
    lowerer.engine.unify(&trait_arg, &Type::I64).unwrap();
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKind::Var("value".to_string()),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }),
            "method".to_string(),
            Vec::new(),
            None,
            Some(HirMethodCallTarget::trait_method(
                DefId::new(CrateId(0), LocalDefId(1)),
                DefId::new(CrateId(0), LocalDefId(2)),
                vec![trait_arg],
                crate::hir::HirTraitDispatchKind::TraitBound,
            )),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    lowerer.resolve_all_types_in_expr(&mut expr);

    let HirExprKind::MethodCall(_, _, _, _, Some(target)) = &expr.kind else {
        panic!("expected method call target");
    };
    assert_eq!(target.trait_args(), &[Type::I64]);
}

#[test]
fn resolve_all_types_in_expr_resolves_static_method_substitution() {
    let mut lowerer = Lowerer::new_for_test();
    let method_id = DefId::new(CrateId(0), LocalDefId(3));
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let method_ty = lowerer.engine.fresh_type_var();
    lowerer.engine.unify(&method_ty, &Type::I64).unwrap();
    let mut method = HirMethodCallTarget::trait_method(
        DefId::new(CrateId(0), LocalDefId(4)),
        method_id,
        Vec::new(),
        crate::hir::HirTraitDispatchKind::TraitBound,
    );
    method.method_substitution = vec![HirTypeBinding {
        param: method_param,
        ty: method_ty,
    }];
    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::Var("generic_static".to_string()),
                ty: Type::function(Vec::new(), Type::I64),
                span: crate::lexer::Span::test(),
            }),
            Vec::new(),
            Some(HirCallTarget::StaticMethod(HirStaticMethodTarget {
                owner_ty: Type::I64,
                method,
            })),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    lowerer.resolve_all_types_in_expr(&mut expr);

    let HirExprKind::Call(_, _, Some(HirCallTarget::StaticMethod(target))) = &expr.kind else {
        panic!("expected static method target");
    };
    assert_eq!(target.method.method_substitution.len(), 1);
    assert_eq!(target.method.method_substitution[0].param, method_param);
    assert_eq!(target.method.method_substitution[0].ty, Type::I64);
}
