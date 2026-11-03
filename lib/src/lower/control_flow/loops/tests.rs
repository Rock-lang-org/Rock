use crate::ast::{
    Block, Expression, Ident, IdentOrType, IdentifierPath, Literal, LiteralKind, Operand, Pattern,
    PatternKind, PrimaryExpr, Statement, UnaryExpr,
};
use crate::hir::{HirExprKind, HirStmt, HirVarRef, HirVarTarget};
use crate::lexer::Span;
use crate::lower::Lowerer;

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: Span::test(),
    }
}

fn binding_pattern(name: &str) -> Pattern {
    Pattern {
        binding: Some(ident(name)),
        kind: PatternKind::Ident(crate::ast::IdentPattern {
            name: ident(name),
            mut_: false,
        }),
    }
}

fn array_expr(start: u64, end: u64) -> Expression {
    let endpoint = |value| {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Literal(Literal {
                kind: LiteralKind::Number(value),
                span: Span::test(),
            }),
            secondaries: None,
            type_annotation: None,
        }))
    };
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Literal(Literal {
            kind: LiteralKind::Array(crate::ast::Array {
                elements: vec![endpoint(start), endpoint(end)],
            }),
            span: Span::test(),
        }),
        secondaries: None,
        type_annotation: None,
    }))
}

fn var_expr(name: &str) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Ident(IdentifierPath {
            path: vec![IdentOrType::Ident(ident(name))],
        }),
        secondaries: None,
        type_annotation: None,
    }))
}

#[test]
fn for_loop_variable_read_uses_loop_local_id() {
    let mut lowerer = Lowerer::new_for_test();
    let loop_expr = crate::ast::Loop::For(
        binding_pattern("item"),
        array_expr(0, 2),
        Block {
            statements: vec![Statement::Expression(var_expr("item"))],
        },
        Span::test(),
    );

    let hir = lowerer.with_test_body_context(|lowerer| lowerer.lower_loop(&loop_expr));

    let HirExprKind::For { local_id, body, .. } = hir.kind else {
        panic!("expected for loop");
    };
    let HirStmt::Expr(expr) = &body.stmts[0] else {
        panic!("expected loop body expression");
    };
    match &expr.kind {
        HirExprKind::ResolvedVar(HirVarRef {
            target: HirVarTarget::Local(id),
            name,
        }) => {
            assert_eq!(name, "item");
            assert_eq!(*id, local_id);
        }
        other => panic!("expected loop local read, got {other:?}"),
    }
}

#[test]
fn loop_control_statements_do_not_require_a_value_span() {
    let mut lowerer = Lowerer::new_for_test();
    for statement in [
        Statement::Continue(None),
        Statement::Return(None),
        Statement::Break(None),
    ] {
        let loop_expr = crate::ast::Loop::Loop(
            Block {
                statements: vec![statement],
            },
            Span::test(),
        );
        let hir = lowerer.with_test_body_context(|lowerer| lowerer.lower_loop(&loop_expr));
        assert!(matches!(hir.kind, HirExprKind::Loop(_)));
        assert_eq!(hir.span, Span::test());
    }
}
