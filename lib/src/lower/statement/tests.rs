use crate::ast::{
    Assignment, AssignmentLHS, Expression, Ident, IdentPattern, Literal, LiteralKind, Operand,
    Pattern, PatternKind, PrimaryExpr, UnaryExpr,
};
use crate::hir::{HirExprKind, HirStmt, HirVarRef, HirVarTarget};
use crate::ids::TypeVarId;
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::types::Type;

fn number_expr(value: u64) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Literal(Literal {
            kind: LiteralKind::Number(value),
            span: Span::test(),
        }),
        secondaries: None,
        type_annotation: None,
    }))
}

fn int_expr(value: u64) -> Expression {
    number_expr(value)
}

fn tuple_expr(elements: Vec<Expression>) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Tuple(crate::ast::Tuple { elements }),
        secondaries: None,
        type_annotation: None,
    }))
}

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: Span::test(),
    }
}

fn binding_pattern(name: &str) -> Pattern {
    Pattern {
        binding: Some(ident(name)),
        kind: PatternKind::Ident(IdentPattern {
            name: ident(name),
            mut_: false,
        }),
    }
}

#[test]
fn tuple_destructuring_temp_name_does_not_consume_type_var_id() {
    let mut lowerer = Lowerer::new_for_test();

    let _ = lowerer
        .with_test_body_context(|lowerer| lowerer.lower_tuple_destructuring(&[], &number_expr(1)));

    assert_eq!(lowerer.engine.fresh_type_var(), Type::TypeVar(TypeVarId(0)));
}

#[test]
fn tuple_destructuring_assigns_ids_to_temp_and_bindings() {
    let mut lowerer = Lowerer::new_for_test();
    let stmts = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_tuple_destructuring(
            &[binding_pattern("left"), binding_pattern("right")],
            &tuple_expr(vec![int_expr(1), int_expr(2)]),
        )
    });

    let HirStmt::Let {
        name: temp_name,
        local_id: temp_id,
        ..
    } = &stmts[0]
    else {
        panic!("expected tuple temp let");
    };
    assert!(temp_name.starts_with("__tuple_tmp_"));

    let HirStmt::Let {
        name: left_name,
        local_id: left_id,
        value,
        ..
    } = &stmts[1]
    else {
        panic!("expected left binding let");
    };
    assert_eq!(left_name, "left");
    assert_ne!(temp_id, left_id);
    match &value.kind {
        HirExprKind::TupleIndex(base, 0) => match &base.kind {
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Local(id),
                ..
            }) => {
                assert_eq!(id, temp_id);
            }
            other => panic!("expected tuple temp local ref, got {other:?}"),
        },
        other => panic!("expected tuple index, got {other:?}"),
    }
}

#[test]
fn reassignment_targets_shadowing_local_id() {
    let mut lowerer = Lowerer::new_for_test();
    let (inner, stmt) = lowerer.with_test_body_context(|lowerer| {
        let outer = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("x".to_string(), Type::I64, true, outer);
        lowerer.scope.push();
        let inner = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("x".to_string(), Type::I64, true, inner);
        let stmt = lowerer.lower_assignment(&Assignment {
            lhs: AssignmentLHS::Pattern {
                pattern: binding_pattern("x"),
                type_annotation: None,
            },
            rhs: number_expr(2),
        });
        (inner, stmt)
    });

    let HirStmt::Expr(expr) = stmt else {
        panic!("expected assignment expression statement");
    };
    let HirExprKind::Assign(lhs, _) = expr.kind else {
        panic!("expected assignment expression");
    };
    match lhs.kind {
        HirExprKind::ResolvedVar(HirVarRef {
            target: HirVarTarget::Local(id),
            name,
        }) => {
            assert_eq!(name, "x");
            assert_eq!(id, inner);
        }
        other => panic!("expected assignment lhs local ref, got {other:?}"),
    }
}
