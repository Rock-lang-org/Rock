use crate::ast;
use crate::collect::item_index::ModuleKind;
use crate::ids::{DefId, ModuleId};
use crate::lower::module_context::ModuleLoweringContext;
use crate::lower::Lowerer;
use crate::lower::scope::LocalName;
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
    scopes: Vec<HashSet<LocalName>>,
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
        let shadowed = path.path
            .first()
            .and_then(LocalName::from_segment)
            .is_some_and(|name| self.scopes.iter().rev().any(|scope| scope.contains(&name)));
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
            .any(|scope| scope.contains(&LocalName::new(operator.value.clone(), operator.context)))
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

fn collect_pattern_bindings(pattern: &ast::Pattern, bindings: &mut HashSet<LocalName>) {
    if let Some(binding) = &pattern.binding {
        bindings.insert(binding.into());
    }
    match &pattern.kind {
        ast::PatternKind::Ident(ident) => {
            bindings.insert((&ident.name).into());
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
                        bindings.insert((&ident.name).into());
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
mod tests {
    use super::*;

    use crate::ast::Program;
    use crate::crate_system::CrateContext;
    use crate::lower::Lowerer;
    use crate::source_loader::SourceDatabase;

    #[test]
    fn body_lowerer_attaches_bodies_by_source_id_despite_resolver_name_changes() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_body_lowerer_modules_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let entry = temp_dir.join("main.rk");
        let left = temp_dir.join("left.rk");
        let right = temp_dir.join("right.rk");
        std::fs::write(&entry, "mod left\nmod right\nmain: I64\nmain = ->\n    0\n").unwrap();
        std::fs::write(&left, "answer: I64\nanswer = ->\n    7\n< answer\n").unwrap();
        std::fs::write(&right, "answer: I64\nanswer = ->\n    8\n< answer\n").unwrap();

        let config = crate::Config {
            entry_file: entry,
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = SourceDatabase::new();
        let graph = db.load_entry(config.entry_file.clone(), &config).unwrap();
        let left_inline_body = crate::parser::parse_string(
            "answer: I64\nanswer = ->\n    3\n",
            &crate::Config::default(),
        )
        .unwrap()
        .module;
        let right_inline_body = crate::parser::parse_string(
            "answer: I64\nanswer = ->\n    4\n",
            &crate::Config::default(),
        )
        .unwrap()
        .module;
        let mut root_module = graph.root_module().clone();
        root_module
            .top_levels
            .insert(0, ast::TopLevel::InfixOperator(5, "+".to_string()));
        root_module.top_levels.insert(
            1,
            ast::TopLevel::Module(ast::ModuleDecl(ast::Module {
                name: Some(ast::Ident { context: Default::default(),
                    name: "left_inline".to_string(),
                    span: crate::lexer::Span::test(),
                }),
                top_levels: vec![ast::TopLevel::Module(ast::ModuleDecl(ast::Module {
                    name: Some(ast::Ident { context: Default::default(),
                        name: "shared".to_string(),
                        span: crate::lexer::Span::test(),
                    }),
                    top_levels: left_inline_body.top_levels,
                    is_inline: true,
                    filepath: None,
                }))],
                is_inline: true,
                filepath: None,
            })),
        );
        root_module.top_levels.insert(
            2,
            ast::TopLevel::Module(ast::ModuleDecl(ast::Module {
                name: Some(ast::Ident { context: Default::default(),
                    name: "right_inline".to_string(),
                    span: crate::lexer::Span::test(),
                }),
                top_levels: vec![ast::TopLevel::Module(ast::ModuleDecl(ast::Module {
                    name: Some(ast::Ident { context: Default::default(),
                        name: "shared".to_string(),
                        span: crate::lexer::Span::test(),
                    }),
                    top_levels: right_inline_body.top_levels,
                    is_inline: true,
                    filepath: None,
                }))],
                is_inline: true,
                filepath: None,
            })),
        );
        let program = Program {
            module: root_module,
        };
        let crate_ctx = CrateContext::new();
        let decls = crate::collect::collect_with_source_graph(
            &program,
            &graph,
            &crate_ctx,
            false,
            Some("demo"),
        )
        .unwrap();
        let mut lowerer = Lowerer::from_declarations(decls).unwrap();
        lowerer
            .modules
            .configure_current_crate(Some("demo"), Some(&config.entry_file));
        lowerer
            .modules
            .associate_current_crate_module_ids(&lowerer.item_index);

        let main_id = lowerer.resolve_item_def_id("main").unwrap();
        let left_inline_answer_id = lowerer
            .resolve_item_def_id("left_inline::shared::answer")
            .unwrap();
        let right_inline_answer_id = lowerer
            .resolve_item_def_id("right_inline::shared::answer")
            .unwrap();
        let left_answer_id = lowerer.resolve_item_def_id("demo::left::answer").unwrap();
        let right_answer_id = lowerer.resolve_item_def_id("demo::right::answer").unwrap();
        let root_module_id = lowerer
            .item_index
            .modules()
            .iter()
            .find(|module| module.kind == ModuleKind::Root)
            .unwrap()
            .module_id;
        let left_inline_id = lowerer
            .item_index
            .child_module_id(root_module_id, "left_inline")
            .unwrap();
        let right_inline_id = lowerer
            .item_index
            .child_module_id(root_module_id, "right_inline")
            .unwrap();
        assert_ne!(
            lowerer.item_index.child_module_id(left_inline_id, "shared"),
            lowerer
                .item_index
                .child_module_id(right_inline_id, "shared"),
            "same child name under different inline parents must retain distinct ModuleIds"
        );
        assert_eq!(
            lowerer
                .item_index
                .item_at_source(
                    root_module_id,
                    program
                        .module
                        .top_levels
                        .iter()
                        .position(|top_level| {
                            matches!(top_level, ast::TopLevel::FunctionDecl(function) if function.name.name == "main")
                        })
                        .unwrap(),
                )
                .unwrap()
                .def_id,
            main_id,
            "root function should retain its source-indexed DefId"
        );
        for id in [
            main_id,
            left_inline_answer_id,
            right_inline_answer_id,
            left_answer_id,
            right_answer_id,
        ] {
            let body = &mut lowerer.items.function_mut(id).unwrap().body;
            body.stmts = vec![crate::hir::HirStmt::Continue];
            body.ty = crate::types::Type::Str;
        }
        lowerer.resolver.item_paths.clear();
        lowerer.resolver.item_names_by_id.clear();
        lowerer.resolver.import_aliases.clear();
        lowerer.resolver.export_aliases.clear();
        lowerer.resolver.scoped_module_aliases.clear();
        lowerer.resolver.module_aliases.clear();

        BodyLowerer::new(&mut lowerer).lower_root_and_loaded_modules(&program.module);

        assert!(
            lowerer
                .items
                .function(main_id)
                .unwrap()
                .body
                .stmts
                .iter()
                .all(|stmt| !matches!(stmt, crate::hir::HirStmt::Continue),),
            "root body should be lowered: {:?}",
            lowerer.errors()
        );
        assert!(
            lowerer
                .items
                .function(left_inline_answer_id)
                .unwrap()
                .body
                .stmts
                .iter()
                .all(|stmt| !matches!(stmt, crate::hir::HirStmt::Continue)),
            "left nested inline module body should be lowered"
        );
        assert!(
            lowerer
                .items
                .function(right_inline_answer_id)
                .unwrap()
                .body
                .stmts
                .iter()
                .all(|stmt| !matches!(stmt, crate::hir::HirStmt::Continue)),
            "right nested inline module body should be lowered"
        );
        assert!(
            lowerer
                .items
                .function(left_answer_id)
                .unwrap()
                .body
                .stmts
                .iter()
                .all(|stmt| !matches!(stmt, crate::hir::HirStmt::Continue)),
            "left source-backed body should be lowered"
        );
        assert!(
            lowerer
                .items
                .function(right_answer_id)
                .unwrap()
                .body
                .stmts
                .iter()
                .all(|stmt| !matches!(stmt, crate::hir::HirStmt::Continue)),
            "right source-backed body should be lowered"
        );
        let left_span = lowerer
            .source_map
            .definition_declaration_span(left_answer_id)
            .expect("left module definition should retain its declaration span");
        let right_span = lowerer
            .source_map
            .definition_declaration_span(right_answer_id)
            .expect("right module definition should retain its declaration span");
        assert_ne!(left_span.file_path, right_span.file_path);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn function_reference_collector_ignores_local_shadowing() {
        let program = crate::parser::parse_string(
            "helper = -> 1\ncaller = ->\n    helper = -> 2\n    helper!\n",
            &crate::Config::default(),
        )
        .unwrap();
        let top_level_functions = program
            .module
            .top_levels
            .iter()
            .filter_map(|top_level| match top_level {
                ast::TopLevel::FunctionDecl(function) => Some(function.name.name.clone()),
                _ => None,
            })
            .collect();
        let caller = program
            .module
            .top_levels
            .iter()
            .find_map(|top_level| match top_level {
                ast::TopLevel::FunctionDecl(function) if function.name.name == "caller" => {
                    Some(function)
                }
                _ => None,
            })
            .expect("caller function");
        let mut references = FunctionReferenceCollector {
            paths: Vec::new(),
            scopes: Vec::new(),
            top_level_functions,
        };
        caller.lambda.visit(&mut references);
        assert!(references.paths.is_empty());
    }

    #[test]
    fn body_lowerer_lowers_source_backed_body_diagnostic_once() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_body_lowerer_duplicate_diagnostic_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let entry = temp_dir.join("main.rk");
        let helper = temp_dir.join("helper.rk");
        std::fs::write(&entry, "mod helper\nmain: I64\nmain = ->\n    0\n").unwrap();
        std::fs::write(&helper, "bad: I64\nbad = ->\n    \"not an integer\"\n").unwrap();

        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = SourceDatabase::new();
        let graph = db.load_entry(entry.clone(), &config).unwrap();
        let program = Program {
            module: graph.root_module().clone(),
        };
        let crate_ctx = CrateContext::new();
        let decls = crate::collect::collect_with_source_graph(
            &program,
            &graph,
            &crate_ctx,
            false,
            Some("demo"),
        )
        .unwrap();
        let mut lowerer = Lowerer::from_declarations(decls).unwrap();
        lowerer
            .modules
            .configure_current_crate(Some("demo"), Some(&entry));
        lowerer
            .modules
            .associate_current_crate_module_ids(&lowerer.item_index);

        BodyLowerer::new(&mut lowerer).lower_root_and_loaded_modules(&program.module);

        let diagnostics = lowerer
            .errors()
            .iter()
            .filter(|error| {
                error
                    .message
                    .contains("In function 'demo::helper::bad': return type mismatch")
            })
            .count();
        assert_eq!(
            diagnostics, 1,
            "source-backed function body should be lowered exactly once"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
