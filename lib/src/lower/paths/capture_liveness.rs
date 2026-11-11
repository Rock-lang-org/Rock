//! Lexical binding resolution followed by backward liveness at closure creation.
//! Assignments kill the old value; declarations introduce distinct bindings.

use std::collections::{HashMap, HashSet};

use crate::ast::{self, visit::Visitor};

type Binding = usize;
type Live = HashSet<Binding>;

fn key<T>(node: &T) -> usize {
    node as *const T as usize
}

fn pattern_names(pattern: &ast::Pattern) -> Vec<String> {
    #[derive(Default)]
    struct Names(Vec<String>);
    impl<'a> Visitor<'a> for Names {
        fn visit_ident_pattern(&mut self, pattern: &'a ast::IdentPattern) {
            self.0.push(pattern.name.name.clone());
        }
        fn visit_pattern(&mut self, pattern: &'a ast::Pattern) {
            if let Some(name) = &pattern.binding {
                self.0.push(name.name.clone());
            }
            ast::visit::walk_pattern(self, pattern);
        }
    }
    let mut names = Names::default();
    pattern.visit(&mut names);
    names.0
}

#[derive(Default)]
struct Bindings {
    scopes: Vec<HashMap<String, Binding>>,
    next: Binding,
    uses: HashMap<usize, Binding>,
    writes: HashMap<usize, Vec<Binding>>,
    patterns: HashMap<usize, Vec<Binding>>,
    closures: HashMap<usize, HashMap<String, Binding>>,
}

impl Bindings {
    fn lookup(&self, name: &str) -> Option<Binding> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name).copied())
    }

    fn define(&mut self, name: String) -> Binding {
        let id = self.next;
        self.next += 1;
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, id);
        }
        id
    }

    fn define_pattern(&mut self, pattern: &ast::Pattern) {
        let ids = pattern_names(pattern).into_iter().map(|name| self.define(name)).collect();
        self.patterns.insert(key(pattern), ids);
    }
}

impl<'a> Visitor<'a> for Bindings {
    fn visit_block(&mut self, block: &'a ast::Block) {
        self.scopes.push(HashMap::new());
        ast::visit::walk_block(self, block);
        self.scopes.pop();
    }

    fn visit_identifier_path(&mut self, path: &'a ast::IdentifierPath) {
        if let Some(ast::IdentOrType::Ident(name)) = path.path.first() {
            if let Some(id) = self.lookup(&name.name) {
                self.uses.insert(key(path), id);
            }
        }
    }

    fn visit_operand(&mut self, operand: &'a ast::Operand) {
        if matches!(operand, ast::Operand::SelfIdent(_)) {
            if let Some(id) = self.lookup("self") {
                self.uses.insert(key(operand), id);
            }
        } else {
            ast::visit::walk_operand(self, operand);
        }
    }

    fn visit_assignment(&mut self, assignment: &'a ast::Assignment) {
        assignment.rhs.visit(self);
        match &assignment.lhs {
            ast::AssignmentLHS::Pattern { pattern, type_annotation } => {
                let ids = pattern_names(pattern).into_iter().map(|name| {
                    // Match lower_assignment: unannotated existing locals are
                    // reassigned; an annotation always introduces a binding.
                    if type_annotation.is_none() {
                        if let Some(id) = self.lookup(&name) {
                            return id;
                        }
                    }
                    self.define(name)
                }).collect();
                self.writes.insert(key(assignment), ids);
            }
            ast::AssignmentLHS::Expression(value) => value.visit(self),
        }
    }

    fn visit_lambda_decl(&mut self, lambda: &'a ast::LambdaDecl) {
        let visible = self.scopes.iter().flat_map(|scope| scope.clone()).collect();
        self.closures.insert(key(lambda), visible);
        self.scopes.push(HashMap::new());
        for pattern in &lambda.parameters {
            self.define_pattern(pattern);
        }
        lambda.body.visit(self);
        self.scopes.pop();
    }

    fn visit_if(&mut self, value: &'a ast::If) {
        value.condition.expression.visit(self);
        self.scopes.push(HashMap::new());
        if let Some(pattern) = &value.condition.pattern {
            self.define_pattern(pattern);
        }
        value.then.visit(self);
        self.scopes.pop();
        if let Some(otherwise) = &value.else_ {
            otherwise.visit(self);
        }
    }

    fn visit_match(&mut self, value: &'a ast::Match) {
        value.expr.visit(self);
        for arm in &value.arms {
            self.scopes.push(HashMap::new());
            self.define_pattern(&arm.pattern);
            if let Some(condition) = &arm.condition {
                condition.visit(self);
            }
            arm.body.visit(self);
            self.scopes.pop();
        }
    }

    fn visit_loop(&mut self, value: &'a ast::Loop) {
        self.scopes.push(HashMap::new());
        match value {
            ast::Loop::For(pattern, iter, body, _) => {
                iter.visit(self);
                self.define_pattern(pattern);
                body.visit(self);
            }
            ast::Loop::While(condition, body, _) => {
                condition.expression.visit(self);
                if let Some(pattern) = &condition.pattern {
                    self.define_pattern(pattern);
                }
                body.visit(self);
            }
            ast::Loop::Loop(body, _) => body.visit(self),
        }
        self.scopes.pop();
    }
}

struct Liveness<'b> {
    bindings: &'b Bindings,
    live: Live,
    after: HashMap<usize, HashSet<String>>,
    loops: Vec<(Live, Live)>,
}

impl Liveness<'_> {
    fn kill_pattern(&mut self, pattern: &ast::Pattern) {
        if let Some(ids) = self.bindings.patterns.get(&key(pattern)) {
            self.live.retain(|id| !ids.contains(id));
        }
    }
}

impl<'a> Visitor<'a> for Liveness<'_> {
    fn visit_block(&mut self, block: &'a ast::Block) {
        for statement in block.statements.iter().rev() {
            statement.visit(self);
        }
    }

    fn visit_statement(&mut self, statement: &'a ast::Statement) {
        match statement {
            ast::Statement::Return(value) => {
                self.live.clear();
                if let Some(value) = value { value.visit(self); }
            }
            ast::Statement::Continue(value) | ast::Statement::Break(value) => {
                if let Some((head, after)) = self.loops.last() {
                    self.live = if matches!(statement, ast::Statement::Continue(_)) {
                        head.clone()
                    } else {
                        after.clone()
                    };
                }
                if let Some(value) = value { value.visit(self); }
            }
            _ => ast::visit::walk_statement(self, statement),
        }
    }

    fn visit_assignment(&mut self, assignment: &'a ast::Assignment) {
        match &assignment.lhs {
            ast::AssignmentLHS::Pattern { .. } => {
                if let Some(ids) = self.bindings.writes.get(&key(assignment)) {
                    self.live.retain(|id| !ids.contains(id));
                }
            }
            ast::AssignmentLHS::Expression(value) => value.visit(self),
        }
        assignment.rhs.visit(self);
    }

    fn visit_identifier_path(&mut self, path: &'a ast::IdentifierPath) {
        if let Some(id) = self.bindings.uses.get(&key(path)) {
            self.live.insert(*id);
        }
    }

    fn visit_operand(&mut self, operand: &'a ast::Operand) {
        if matches!(operand, ast::Operand::SelfIdent(_)) {
            if let Some(id) = self.bindings.uses.get(&key(operand)) { self.live.insert(*id); }
        } else {
            ast::visit::walk_operand(self, operand);
        }
    }

    fn visit_lambda_decl(&mut self, lambda: &'a ast::LambdaDecl) {
        let after = std::mem::take(&mut self.live);
        if let Some(visible) = self.bindings.closures.get(&key(lambda)) {
            self.after.insert(key(lambda), visible.iter()
                .filter(|(_, id)| after.contains(id)).map(|(name, _)| name.clone()).collect());
        }
        // Invoking a closure is a different control-flow graph.
        let loops = std::mem::take(&mut self.loops);
        lambda.body.visit(self);
        self.loops = loops;
        for pattern in &lambda.parameters { self.kill_pattern(pattern); }
        self.live.extend(after);
    }

    fn visit_primary_expr(&mut self, primary: &'a ast::PrimaryExpr) {
        if primary.secondaries.as_ref().is_some_and(|items| {
            items.iter().any(|item| matches!(item, ast::SecondaryExpr::Arguments(_)))
        }) {
            // The callee borrow lives through argument evaluation and the call.
            // A callback argument capturing that callee must not move it out
            // from under its own invocation (e.g. nested local `bind` calls).
            if let ast::Operand::Ident(path) = &primary.operand {
                if let Some(id) = self.bindings.uses.get(&key(path)) {
                    self.live.insert(*id);
                }
            }
        }
        if let Some(secondaries) = &primary.secondaries {
            for secondary in secondaries.iter().rev() { secondary.visit(self); }
        }
        primary.operand.visit(self);
    }

    fn visit_secondary_expr(&mut self, secondary: &'a ast::SecondaryExpr) {
        match secondary {
            ast::SecondaryExpr::Arguments(args) => {
                for arg in args.iter().rev() { arg.arg.visit(self); }
            }
            ast::SecondaryExpr::Indice(index) => index.visit(self),
            ast::SecondaryExpr::Dot(_) | ast::SecondaryExpr::Interogation => {}
        }
    }

    fn visit_tuple(&mut self, tuple: &'a ast::Tuple) {
        for element in tuple.elements.iter().rev() { element.visit(self); }
    }

    fn visit_array(&mut self, array: &'a ast::Array) {
        for element in array.elements.iter().rev() { element.visit(self); }
    }

    fn visit_instance(&mut self, instance: &'a ast::Instance) {
        for value in instance.fields.values().collect::<Vec<_>>().into_iter().rev() { value.visit(self); }
    }

    fn visit_expression(&mut self, expression: &'a ast::Expression) {
        match expression {
            ast::Expression::BinopExpr(left, _, right) => { right.visit(self); left.visit(self); }
            ast::Expression::Range(range) => {
                if let Some(end) = &range.end { end.visit(self); }
                if let Some(start) = &range.start { start.visit(self); }
            }
            _ => ast::visit::walk_expression(self, expression),
        }
    }

    fn visit_if(&mut self, value: &'a ast::If) {
        let after = self.live.clone();
        value.then.visit(self);
        if let Some(pattern) = &value.condition.pattern { self.kill_pattern(pattern); }
        let mut before = self.live.clone();
        self.live = after;
        if let Some(otherwise) = &value.else_ { otherwise.visit(self); }
        before.extend(self.live.drain());
        self.live = before;
        value.condition.expression.visit(self);
    }

    fn visit_match(&mut self, value: &'a ast::Match) {
        let after = self.live.clone();
        let mut before = Live::new();
        for arm in &value.arms {
            self.live = after.clone();
            arm.body.visit(self);
            if let Some(condition) = &arm.condition { condition.visit(self); }
            self.kill_pattern(&arm.pattern);
            before.extend(self.live.drain());
        }
        self.live = before;
        value.expr.visit(self);
    }

    fn visit_loop(&mut self, value: &'a ast::Loop) {
        let (body, pattern) = match value {
            ast::Loop::For(pattern, _, body, _) => (body, Some(pattern)),
            ast::Loop::While(condition, body, _) => (body, condition.pattern.as_ref()),
            ast::Loop::Loop(body, _) => (body, None),
        };
        let after = self.live.clone();
        let mut head = after.clone();
        loop {
            self.loops.push((head.clone(), after.clone()));
            self.live = head.clone();
            body.visit(self);
            self.loops.pop();
            if let Some(pattern) = pattern { self.kill_pattern(pattern); }
            if let ast::Loop::While(condition, _, _) = value { condition.expression.visit(self); }
            let previous = head.len();
            head.extend(self.live.drain());
            if head.len() == previous { break; }
        }
        self.live = head;
        if let ast::Loop::For(_, iter, _, _) = value { iter.visit(self); }
    }
}

impl crate::lower::Lowerer {
    pub(crate) fn prepare_capture_liveness(&mut self, block: &ast::Block) {
        // Seed only locals already in scope (parameters and enclosing captures),
        // never global function or type names.
        #[derive(Default)]
        struct Names(HashSet<String>);
        impl<'a> Visitor<'a> for Names {
            fn visit_ident(&mut self, name: &'a ast::Ident) { self.0.insert(name.name.clone()); }
        }
        let mut names = Names::default();
        block.visit(&mut names);
        names.0.insert("self".to_string());
        let mut bindings = Bindings::default();
        bindings.scopes.push(HashMap::new());
        for name in names.0 {
            if self.scope.lookup(&name).is_some_and(|binding| binding.local_id.is_some()) {
                bindings.define(name);
            }
        }
        block.visit(&mut bindings);
        let mut liveness = Liveness {
            bindings: &bindings, live: Live::new(), after: HashMap::new(), loops: Vec::new(),
        };
        block.visit(&mut liveness);
        self.body.capture_live_after.extend(liveness.after);
    }
}
