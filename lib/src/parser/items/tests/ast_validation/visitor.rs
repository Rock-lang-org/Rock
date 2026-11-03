use super::*;
use crate::ast::visit::Visitor;

// ===========================================================================
// Test: Visitor walks the full AST without panicking
// ===========================================================================

/// A visitor that simply counts every node type it encounters.
struct NodeCounter {
    total: usize,
    functions: usize,
    expressions: usize,
    statements: usize,
    literals: usize,
    patterns: usize,
    identifiers: usize,
    blocks: usize,
}

impl NodeCounter {
    fn new() -> Self {
        Self {
            total: 0,
            functions: 0,
            expressions: 0,
            statements: 0,
            literals: 0,
            patterns: 0,
            identifiers: 0,
            blocks: 0,
        }
    }
}

impl<'ast> Visitor<'ast> for NodeCounter {
    fn visit_function_decl(&mut self, node: &'ast FunctionDecl) {
        self.total += 1;
        self.functions += 1;
        crate::ast::visit::walk_function_decl(self, node);
    }

    fn visit_expression(&mut self, node: &'ast Expression) {
        self.total += 1;
        self.expressions += 1;
        crate::ast::visit::walk_expression(self, node);
    }

    fn visit_statement(&mut self, node: &'ast Statement) {
        self.total += 1;
        self.statements += 1;
        crate::ast::visit::walk_statement(self, node);
    }

    fn visit_literal(&mut self, node: &'ast Literal) {
        self.total += 1;
        self.literals += 1;
        crate::ast::visit::walk_literal(self, node);
    }

    fn visit_pattern(&mut self, node: &'ast Pattern) {
        self.total += 1;
        self.patterns += 1;
        crate::ast::visit::walk_pattern(self, node);
    }

    fn visit_ident(&mut self, node: &'ast Ident) {
        self.total += 1;
        self.identifiers += 1;
        crate::ast::visit::walk_ident(self, node);
    }

    fn visit_block(&mut self, node: &'ast Block) {
        self.total += 1;
        self.blocks += 1;
        crate::ast::visit::walk_block(self, node);
    }
}

#[test]
fn visitor_counts_realistic_program() {
    let input = r#"struct Point
    x: Int
    y: Int

impl Point
    add = other ->
        Point
            x: @x + other.x
            y: @y + other.y

main = ->
    a = Point x: 1, y: 2
    b = Point x: 3, y: 4
    c = a.add b
    result = match c.x
        0 => "zero"
        n if n > 0 => "positive"
        _ => "negative"
    result
"#;
    let p = parse(input);
    let mut counter = NodeCounter::new();
    p.visit(&mut counter);

    assert!(counter.total > 0, "Visitor should have visited nodes");
    assert!(
        counter.functions >= 2,
        "Should have at least 2 function decls (add + main)"
    );
    assert!(counter.expressions > 0, "Should have expressions");
    assert!(counter.statements > 0, "Should have statements");
    assert!(counter.identifiers > 0, "Should have identifiers");
    assert!(counter.blocks > 0, "Should have blocks");
    assert!(
        counter.patterns > 0,
        "Should have patterns (in match + function params)"
    );
}

#[test]
fn visitor_walks_all_control_flow() {
    let input = r#"main = ->
    x = if true then 1 else 2
    y = match x
        1 => "one"
        _ => "other"
    z = 0
    for i in [1, 2, 3]
        z = z + i
    w = 0
    while w < 10
        w = w + 1
    result = loop
        break 42
    result
"#;
    let p = parse(input);
    let mut counter = NodeCounter::new();
    p.visit(&mut counter);

    // Sanity check that we traversed deeply
    assert!(
        counter.total > 20,
        "Should visit many nodes, got {}",
        counter.total
    );
    assert!(
        counter.blocks >= 5,
        "Should have multiple blocks (main body + if/match/for/while/loop), got {}",
        counter.blocks
    );
}
