use crate::ast;
use crate::collect::item_index::ModuleKind;
use crate::ids::{DefId, ModuleId};
use crate::lower::module_context::ModuleLoweringContext;
use crate::lower::Lowerer;
use std::collections::{BTreeSet, HashMap, HashSet};

pub(crate) struct BodyLowerer<'a> {
    lowerer: &'a mut Lowerer,
}

impl<'a> BodyLowerer<'a> {
    pub(crate) fn new(lowerer: &'a mut Lowerer) -> Self {
        Self { lowerer }
    }

    pub(crate) fn lower_root_and_loaded_modules(&mut self, root_module: &ast::Module) {
        let Some(root_module_id) = self
            .lowerer
            .item_index
            .modules()
            .iter()
            .find(|module| module.kind == ModuleKind::Root)
            .map(|module| module.module_id)
        else {
            self.lowerer
                .diagnostics
                .push_toolchain("missing indexed root module while lowering bodies".to_string());
            return;
        };
        let mut edges = HashMap::new();
        collect_module_edges(self.lowerer, root_module, None, root_module_id, &mut edges);
        ModuleLoweringContext::for_each_loaded_module(
            self.lowerer,
            |lowerer, module_id, module_name, loaded_module| {
                collect_module_edges(
                    lowerer,
                    loaded_module,
                    Some(module_name),
                    module_id,
                    &mut edges,
                );
            },
        );
        let mut nodes = self
            .lowerer
            .items
            .functions()
            .filter_map(|(id, _)| self.lowerer.current_def_ids.contains(&id).then_some(id))
            .collect::<BTreeSet<_>>();
        nodes.extend(
            self.lowerer
                .items
                .trait_defs()
                .flat_map(|(_, trait_def)| trait_def.methods.values())
                .filter_map(|method| {
                    self.lowerer
                        .current_def_ids
                        .contains(&method.id)
                        .then_some(method.id)
                }),
        );
        nodes.extend(
            self.lowerer
                .items
                .impl_defs()
                .flat_map(|(_, impl_def)| impl_def.methods.values())
                .filter_map(|method| {
                    self.lowerer
                        .current_def_ids
                        .contains(&method.id)
                        .then_some(method.id)
                }),
        );
        self.lowerer.inference_sccs = crate::lower::inference_scc::components(&nodes, &edges);
        for (_, trait_def) in self.lowerer.items.trait_defs() {
            for method in trait_def.methods.values() {
                if !self.lowerer.current_def_ids.contains(&method.id) {
                    continue;
                }
                self.lowerer
                    .inference_sccs
                    .entry(method.id)
                    .or_insert(method.id);
            }
        }
        for (_, impl_def) in self.lowerer.items.impl_defs() {
            for method in impl_def.methods.values() {
                if !self.lowerer.current_def_ids.contains(&method.id) {
                    continue;
                }
                self.lowerer
                    .inference_sccs
                    .entry(method.id)
                    .or_insert(method.id);
            }
        }
        self.lowerer.inference_scc_order =
            crate::lower::inference_scc::dependency_order(&self.lowerer.inference_sccs, &edges);

        self.lower_module(root_module, root_module_id);
        self.lower_loaded_modules();
    }

    pub(crate) fn lower_module(&mut self, module: &ast::Module, module_id: ModuleId) {
        self.lower_module_qualified(module, None, module_id);
    }

    pub(crate) fn lower_module_qualified(
        &mut self,
        module: &ast::Module,
        module_prefix: Option<&str>,
        module_id: ModuleId,
    ) {
        self.lowerer
            .lower_module_bodies_qualified_impl(module, module_prefix, module_id);
    }

    pub(crate) fn lower_loaded_modules(&mut self) {
        ModuleLoweringContext::for_each_loaded_module(
            self.lowerer,
            |lowerer, module_id, module_name, loaded_module| {
                lowerer.lower_module_bodies_qualified_impl(
                    loaded_module,
                    Some(module_name),
                    module_id,
                );
            },
        );
    }
}

struct FunctionReferenceCollector {
    paths: Vec<Vec<String>>,
    scopes: Vec<HashSet<String>>,
    top_level_functions: HashSet<String>,
}

impl<'ast> ast::visit::Visitor<'ast> for FunctionReferenceCollector {
    fn visit_identifier_path(&mut self, path: &'ast ast::IdentifierPath) {
        let names = path
            .path
            .iter()
            .filter_map(|segment| match segment {
                ast::IdentOrType::Ident(ident) => Some(ident.name.clone()),
                ast::IdentOrType::Type(ast::ParseType::Type(ty)) => Some(ty.name.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let shadowed = names
            .first()
            .is_some_and(|name| self.scopes.iter().rev().any(|scope| scope.contains(name)));
        let is_qualified = names.len() > 1;
        let known_top_level = names
            .first()
            .is_some_and(|name| self.top_level_functions.contains(name));
        if !names.is_empty() && !shadowed && (is_qualified || known_top_level) {
            self.paths.push(names);
        }
    }

    fn visit_lambda_decl(&mut self, lambda: &'ast ast::LambdaDecl) {
        let mut bindings = HashSet::new();
        for parameter in &lambda.parameters {
            collect_pattern_bindings(parameter, &mut bindings);
        }
        self.scopes.push(bindings);
        ast::visit::walk_block(self, &lambda.body);
        self.scopes.pop();
    }

    fn visit_operator(&mut self, operator: &'ast ast::Operator) {
        if !self
            .scopes
            .iter()
            .rev()
            .any(|scope| scope.contains(&operator.value))
        {
            self.paths.push(vec![operator.value.clone()]);
        }
    }

    fn visit_block(&mut self, block: &'ast ast::Block) {
        self.scopes.push(HashSet::new());
        ast::visit::walk_block(self, block);
        self.scopes.pop();
    }

    fn visit_statement(&mut self, statement: &'ast ast::Statement) {
        match statement {
            ast::Statement::Assignment(assignment) => {
                self.visit_expression(&assignment.rhs);
                match &assignment.lhs {
                    ast::AssignmentLHS::Expression(lhs) => self.visit_unary_expr(lhs),
                    ast::AssignmentLHS::Pattern {
                        pattern,
                        type_annotation,
                    } => {
                        if let Some(type_annotation) = type_annotation {
                            self.visit_parse_type(type_annotation);
                        }
                        let mut bindings = HashSet::new();
                        collect_pattern_bindings(pattern, &mut bindings);
                        if let Some(scope) = self.scopes.last_mut() {
                            scope.extend(bindings);
                        }
                    }
                }
            }
            ast::Statement::Expression(expression) => self.visit_expression(expression),
            ast::Statement::Return(expression)
            | ast::Statement::Continue(expression)
            | ast::Statement::Break(expression) => {
                if let Some(expression) = expression {
                    self.visit_expression(expression);
                }
            }
        }
    }

    fn visit_match_arm(&mut self, arm: &'ast ast::MatchArm) {
        let mut bindings = HashSet::new();
        collect_pattern_bindings(&arm.pattern, &mut bindings);
        self.visit_pattern(&arm.pattern);
        self.scopes.push(bindings);
        if let Some(condition) = &arm.condition {
            self.visit_expression(condition);
        }
        self.visit_block(&arm.body);
        self.scopes.pop();
    }

    fn visit_loop(&mut self, loop_: &'ast ast::Loop) {
        match loop_ {
            ast::Loop::For(pattern, condition, body, _) => {
                self.visit_expression(condition);
                let mut bindings = HashSet::new();
                collect_pattern_bindings(pattern, &mut bindings);
                self.scopes.push(bindings);
                self.visit_pattern(pattern);
                self.visit_block(body);
                self.scopes.pop();
            }
            ast::Loop::While(condition, body, _) => {
                self.visit_condition(condition);
                self.visit_block(body);
            }
            ast::Loop::Loop(body, _) => self.visit_block(body),
        }
    }
}

fn collect_pattern_bindings(pattern: &ast::Pattern, bindings: &mut HashSet<String>) {
    if let Some(binding) = &pattern.binding {
        bindings.insert(binding.name.clone());
    }
    match &pattern.kind {
        ast::PatternKind::Ident(ident) => {
            bindings.insert(ident.name.name.clone());
        }
        ast::PatternKind::Tuple(patterns) => {
            for pattern in patterns {
                collect_pattern_bindings(pattern, bindings);
            }
        }
        ast::PatternKind::Array(patterns) => {
            for pattern in patterns {
                match pattern {
                    ast::ArrayPattern::Pattern(pattern) => {
                        collect_pattern_bindings(pattern, bindings)
                    }
                    ast::ArrayPattern::Rest(ident) => {
                        bindings.insert(ident.name.name.clone());
                    }
                }
            }
        }
        ast::PatternKind::Instance(instance) => match &instance.args {
            ast::FieldsPatternOrArgumentsPattern::Fields(fields) => {
                for field in fields {
                    collect_pattern_bindings(&field.pattern, bindings);
                }
            }
            ast::FieldsPatternOrArgumentsPattern::Arguments(patterns) => {
                for pattern in patterns {
                    collect_pattern_bindings(pattern, bindings);
                }
            }
        },
        ast::PatternKind::Nested(pattern) | ast::PatternKind::Reference { pattern, .. } => {
            collect_pattern_bindings(pattern, bindings)
        }
        ast::PatternKind::Literal(_) | ast::PatternKind::Wildcard => {}
    }
}

fn collect_module_edges(
    lowerer: &Lowerer,
    module: &ast::Module,
    module_prefix: Option<&str>,
    module_id: ModuleId,
    edges: &mut HashMap<DefId, BTreeSet<DefId>>,
) {
    for (ordinal, top_level) in module.top_levels.iter().enumerate() {
        let ast::TopLevel::FunctionDecl(function) = top_level else {
            continue;
        };
        let Some(record) = lowerer.item_index.item_at_source(module_id, ordinal) else {
            continue;
        };
        let function_id = record.def_id;
        let top_level_functions = module
            .top_levels
            .iter()
            .filter_map(|top_level| match top_level {
                ast::TopLevel::FunctionDecl(function) => Some(function.name.name.clone()),
                _ => None,
            })
            .collect();
        let mut references = FunctionReferenceCollector {
            paths: Vec::new(),
            scopes: Vec::new(),
            top_level_functions,
        };
        function.lambda.visit(&mut references);
        let function_edges = edges.entry(function_id).or_default();
        for path in references.paths {
            let path = path.join("::");
            let mut candidates = Vec::new();
            if let Some(prefix) = module_prefix {
                candidates.push(format!("{prefix}::{path}"));
            }
            candidates.push(path);
            if let Some(target) = lowerer
                .resolver
                .resolve_item_or_alias(&candidates[0])
                .or_else(|| {
                    candidates
                        .iter()
                        .skip(1)
                        .find_map(|candidate| lowerer.resolver.resolve_item_or_alias(candidate))
                })
            {
                if lowerer.items.function(target).is_some() {
                    function_edges.insert(target);
                }
            }
        }
    }

    for (ordinal, top_level) in module.top_levels.iter().enumerate() {
        let ast::TopLevel::Impl(implementation) = top_level else {
            continue;
        };
        let Some(record) = lowerer.item_index.item_at_source(module_id, ordinal) else {
            continue;
        };
        let Some(impl_def) = lowerer.items.impl_def(record.def_id) else {
            continue;
        };
        let top_level_functions = module
            .top_levels
            .iter()
            .filter_map(|top_level| match top_level {
                ast::TopLevel::FunctionDecl(function) => Some(function.name.name.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        for (method_name, method) in &implementation.methods {
            let Some(hir_method) = impl_def.methods.get(&method_name.name) else {
                continue;
            };
            let mut references = FunctionReferenceCollector {
                paths: Vec::new(),
                scopes: Vec::new(),
                top_level_functions: top_level_functions.clone(),
            };
            method.lambda.visit(&mut references);
            let method_edges = edges.entry(hir_method.id).or_default();
            for path in references.paths {
                let path = path.join("::");
                let mut candidates = Vec::new();
                if let Some(prefix) = module_prefix {
                    candidates.push(format!("{prefix}::{path}"));
                }
                candidates.push(path);
                if let Some(target) = lowerer
                    .resolver
                    .resolve_item_or_alias(&candidates[0])
                    .or_else(|| {
                        candidates
                            .iter()
                            .skip(1)
                            .find_map(|candidate| lowerer.resolver.resolve_item_or_alias(candidate))
                    })
                {
                    if lowerer.items.function(target).is_some() {
                        method_edges.insert(target);
                    }
                }
            }
        }
    }

    for (ordinal, top_level) in module.top_levels.iter().enumerate() {
        let ast::TopLevel::Module(module_decl) = top_level else {
            continue;
        };
        let Some(name) = module_decl.0.name.as_ref() else {
            continue;
        };
        let Some(child_module_id) = lowerer.item_index.child_module_id(module_id, &name.name)
        else {
            let _ = ordinal;
            continue;
        };
        let prefix = match module_prefix {
            Some(prefix) => format!("{prefix}::{}", name.name),
            None => name.name.clone(),
        };
        collect_module_edges(
            lowerer,
            &module_decl.0,
            Some(&prefix),
            child_module_id,
            edges,
        );
    }
}

#[cfg(test)]
mod tests;
