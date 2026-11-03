//! Comprehensive AST validation tests.
//!
//! These tests parse complete programs and walk the resulting AST to verify
//! logical invariants rather than exact structural equality. This catches
//! classes of bugs where the parser produces an AST that "looks OK" in a
//! unit test but is nonsensical when you inspect the relationships between
//! nodes (empty bodies, missing identifiers, wrong nesting, etc.).

use crate::ast::tree::*;
use crate::parser::parse_string;
use crate::Config;

mod support;
use support::*;

// ===========================================================================
// ===========================================================================
//
//   DEEP STRUCTURAL VALIDATION
//
//   The tests below don't just check "does it parse + pass invariants",
//   they crack open the AST and verify the *shape* is sensible: operator
//   chains, nesting depths, argument binding, expression-in-expression, etc.
//
// ===========================================================================
// ===========================================================================

// ---- helpers for digging into the AST ------------------------------------

/// Unwrap a statement that is a bare expression.
fn as_expr(stmt: &Statement) -> &Expression {
    match stmt {
        Statement::Expression(e) => e,
        other => panic!("expected Expression statement, got {:?}", other),
    }
}

/// Unwrap a statement that is an assignment.
fn as_assign(stmt: &Statement) -> &Assignment {
    match stmt {
        Statement::Assignment(a) => a,
        other => panic!("expected Assignment statement, got {:?}", other),
    }
}

/// Unwrap Expression::UnaryExpr -> UnaryExpr::PrimaryExpr -> PrimaryExpr.
fn as_primary(expr: &Expression) -> &PrimaryExpr {
    match expr {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(pe)) => pe,
        other => panic!("expected simple primary expression, got {:?}", other),
    }
}

/// Unwrap to Operand from a simple (no-secondary) expression.
fn as_operand(expr: &Expression) -> &Operand {
    &as_primary(expr).operand
}

/// Unwrap a BinopExpr; return (lhs unary, operator string, rhs expression).
fn as_binop(expr: &Expression) -> (&UnaryExpr, &str, &Expression) {
    match expr {
        Expression::BinopExpr(lhs, op, rhs) => (lhs, &op.value, rhs),
        other => panic!("expected BinopExpr, got {:?}", other),
    }
}

/// Get the identifier name from a simple ident operand.
fn ident_name(expr: &Expression) -> &str {
    match as_operand(expr) {
        Operand::Ident(path) => match &path.path[0] {
            IdentOrType::Ident(id) => &id.name,
            other => panic!("expected Ident in path, got {:?}", other),
        },
        other => panic!("expected Ident operand, got {:?}", other),
    }
}

/// Get the integer value from a literal number operand.
fn lit_number(expr: &Expression) -> u64 {
    match as_operand(expr) {
        Operand::Literal(lit) => match &lit.kind {
            LiteralKind::Number(n) => *n,
            other => panic!("expected Number literal, got {:?}", other),
        },
        other => panic!("expected Literal operand, got {:?}", other),
    }
}

/// Get the integer value from a UnaryExpr that wraps a simple literal.
fn unary_lit_number(u: &UnaryExpr) -> u64 {
    match u {
        UnaryExpr::PrimaryExpr(pe) => match &pe.operand {
            Operand::Literal(lit) => match &lit.kind {
                LiteralKind::Number(n) => *n,
                other => panic!("expected Number literal, got {:?}", other),
            },
            other => panic!("expected Literal operand, got {:?}", other),
        },
        other => panic!("expected PrimaryExpr, got {:?}", other),
    }
}

/// Get the ident name from a UnaryExpr that wraps a simple ident.
fn unary_ident_name(u: &UnaryExpr) -> &str {
    match u {
        UnaryExpr::PrimaryExpr(pe) => match &pe.operand {
            Operand::Ident(path) => match &path.path[0] {
                IdentOrType::Ident(id) => &id.name,
                other => panic!("expected Ident in path, got {:?}", other),
            },
            other => panic!("expected Ident operand, got {:?}", other),
        },
        other => panic!("expected PrimaryExpr, got {:?}", other),
    }
}

/// Get the secondaries list from a primary expression.
fn get_secondaries(expr: &Expression) -> &[SecondaryExpr] {
    let pe = as_primary(expr);
    pe.secondaries.as_deref().unwrap_or(&[])
}

mod control_flow;
mod declarations;
mod edge_cases;
mod expressions;
mod impls;
mod lambdas_and_instances;
mod nested_structure;
mod operator_structure;
mod realistic_programs;
mod value_structure;
mod visitor;
