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
        let method_ids = self.lowerer.items.impl_defs()
            .flat_map(|(_, implementation)| implementation.methods.values())
            .filter_map(|method| self.lowerer.current_def_ids.contains(&method.id).then_some(method.id))
            .collect();
        self.lowerer.inference_scc_order = crate::lower::inference_scc::dependency_order(
            &self.lowerer.inference_sccs, &edges, &method_ids,
        );

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

pub(crate) fn body_dependency_rank(lowerer: &Lowerer, id: DefId) -> usize {
    let component = lowerer.inference_sccs.get(&id).copied().unwrap_or(id);
    lowerer
        .inference_scc_order
        .iter()
        .position(|&candidate| candidate == component)
        .unwrap_or(usize::MAX)
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
    lowerer: &mut Lowerer,
    module: &ast::Module,
    module_prefix: Option<&str>,
    module_id: ModuleId,
    edges: &mut HashMap<DefId, BTreeSet<DefId>>,
) {
    ModuleLoweringContext::with_qualified_module_context(
        lowerer,
        module,
        module_prefix,
        |lowerer| collect_module_edges_in_context(lowerer, module, module_prefix, module_id, edges),
    );
}

fn resolve_body_reference(
    lowerer: &Lowerer,
    path: &[String],
    module_prefix: Option<&str>,
) -> Option<DefId> {
    // Associated functions live in their canonical impl, not the top-level
    // function map. Resolve exactly the same owner/member as body lowering.
    match crate::lower::resolution::LowerResolutionContext::new(lowerer)
        .resolve_static_method_path(path)
    {
        Ok(Some(method)) => return method.target.method_id(),
        Err(_) => return None, // Body lowering reports invalid/ambiguous owners.
        Ok(None) => {}
    }
    let path = path.join("::");
    let target = module_prefix
        .and_then(|prefix| lowerer.resolver.resolve_item_or_alias(&format!("{prefix}::{path}")))
        .or_else(|| lowerer.resolver.resolve_item_or_alias(&path))?;
    lowerer.items.function(target).map(|_| target)
}

fn collect_module_edges_in_context(
    lowerer: &mut Lowerer,
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
            if let Some(target) = resolve_body_reference(lowerer, &path, module_prefix) {
                function_edges.insert(target);
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
                if let Some(target) = resolve_body_reference(lowerer, &path, module_prefix) {
                    method_edges.insert(target);
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

    fn owner_scheduling_source(payload_first: bool, forward: bool, separate_impl: bool) -> String {
        let args = if payload_first { "T, A" } else { "A, T" };
        let header = r#"
lang sized
< trait LayoutKnown
trait Memory
    @key: I64
struct Small
    < tag: I64
impl Memory for Small
    @key = -> self.tag
struct Large
    < padding: (I64, I64)
    < tag: I64
impl Memory for Large
    @key = -> self.tag
struct State A
    < allocator: A
    < token: I64
struct Shell $ARGS
    < value: T
    < state: State A
"#;
        let wrapper = "    new = allocator, value -> Shell::new_aligned allocator, value\n";
        let construction = r#"    new_aligned = allocator, value ->
        state = State
            allocator: allocator
            token: allocator.key!
        Shell
            value: value
            state: state
"#;
        let (first, second) = if forward {
            (wrapper, construction)
        } else {
            (construction, wrapper)
        };
        let implementation = format!("impl Shell {args} where A: Memory\n");
        let separator = if separate_impl { implementation.as_str() } else { "" };
        format!(
            "{}{implementation}{first}{separator}{second}\nmain = ->\n    small = Small\n        tag: 1\n    large = Large\n        padding: (2, 3)\n        tag: 4\n    left = Shell::new small, 42\n    right = Shell::new large, (7 as U8)\n    (~I64Add left.value, (right.value as I64))\n",
            header.replace("$ARGS", args),
        )
    }

    fn owner_scheduling_config(source: &str) -> crate::Config {
        let path = std::path::PathBuf::from("/virtual/owner_scheduling/main.rk");
        crate::Config {
            entry_file: path.clone(),
            current_crate_name: Some("owner_scheduling".into()),
            no_std: true,
            no_prelude: true,
            source_providers: vec![crate::SourceProvider::Virtual {
                path,
                text: source.into(),
            }],
            ..crate::Config::default()
        }
    }

    fn scheduled_static_targets(
        block: &crate::hir::HirBlockFor<crate::hir::AcceptedHir>,
    ) -> Vec<&crate::hir::HirStaticMethodTarget> {
        fn expression<'a>(
            expr: &'a crate::hir::HirExprFor<crate::hir::AcceptedHir>,
            targets: &mut Vec<&'a crate::hir::HirStaticMethodTarget>,
        ) {
            match &expr.kind {
                crate::hir::HirExprKindFor::Call(callee, args, target) => {
                    if let Some(crate::hir::HirCallTarget::StaticMethod(target)) = target {
                        targets.push(target);
                    }
                    expression(callee, targets);
                    for arg in args {
                        expression(arg, targets);
                    }
                }
                crate::hir::HirExprKindFor::Lambda { body, .. }
                | crate::hir::HirExprKindFor::Block(body)
                | crate::hir::HirExprKindFor::UnsafeBlock(body) => {
                    targets.extend(scheduled_static_targets(body));
                }
                _ => {}
            }
        }
        let mut targets = Vec::new();
        for statement in &block.stmts {
            if let crate::hir::HirStmtFor::Let { value, .. }
                | crate::hir::HirStmtFor::Expr(value) = statement
            {
                expression(value, &mut targets);
            }
        }
        targets
    }

    fn assert_allocator_scheme_bound(
        method: &crate::hir::HirFunctionFor<crate::hir::AcceptedHir>,
        memory: &crate::hir::HirTraitFor<crate::hir::AcceptedHir>,
    ) -> crate::types::GenericParamId {
        let crate::types::Type::Generic(allocator) = method.params[0].ty else {
            panic!("allocator input must remain generalized: {method:?}")
        };
        assert!(method.generic_bounds.get(&allocator).is_some_and(|bounds| {
            bounds.iter().any(|bound| bound.trait_id == memory.id && bound.type_args.is_empty())
        }), "the method operation must constrain the actual allocator slot: {method:?}");
        assert!(method.generic_params.iter().any(|parameter| parameter.id == allocator));
        allocator
    }

    fn assert_parametric_allocator_method(
        method: &crate::hir::HirFunctionFor<crate::hir::AcceptedHir>,
        memory: &crate::hir::HirTraitFor<crate::hir::AcceptedHir>,
    ) {
        let allocator = assert_allocator_scheme_bound(method, memory);
        let state = method.body.stmts.iter().find_map(|statement| {
            if let crate::hir::HirStmtFor::Let { name, value, .. } = statement {
                (name == "state").then_some(value)
            } else {
                None
            }
        }).expect("actual State initializer");
        let crate::hir::HirExprKindFor::StructLiteral(_, Some(state_id), fields) = &state.kind else {
            panic!("expected actual State construction: {state:?}")
        };
        assert_eq!(state.ty, crate::types::Type::Struct {
            id: *state_id,
            args: vec![crate::types::Type::Generic(allocator)],
        });
        let key = &fields.iter().find(|field| field.name == "token").unwrap().value;
        let crate::hir::HirExprKindFor::MethodCall(receiver, _, args, receiver_mode, authority) = &key.kind else {
            panic!("expected a declaration-selected allocator operation: {key:?}")
        };
        assert!(args.is_empty());
        assert_eq!(authority.impl_id(), None, "no Small/Large implementation in the producer");
        assert_eq!(authority.target, crate::hir::HirSelectedMethodTarget::TraitMethod {
            trait_id: memory.id,
            member_id: memory.signatures["key"].id,
            trait_args: Vec::new(),
            dispatch: crate::hir::HirTraitDispatchKind::TraitBound,
        });
        // Ordinary HIR keeps the value place plus receiver mode; only fat/scoped
        // handles materialize AutorefShared as an explicit Ref at this stage.
        assert_eq!(*receiver_mode, Some(crate::types::ReceiverMode::Shared));
        assert_eq!(receiver.ty, crate::types::Type::Generic(allocator));
        let traits = HashMap::from([(memory.id, crate::hir::HirTrait {
            id: memory.id,
            name: memory.name.clone(),
            generic_params: memory.generic_params.clone(),
            target: memory.target.clone(),
            predicates: memory.predicates.clone(),
            associated_types: memory.associated_types.clone(),
            methods: HashMap::new(),
            signatures: memory.signatures.clone(),
        })]);
        let impls = HashMap::new();
        let selection = crate::selection::SelectionService::new(
            &traits, &impls, None, None, &method.generic_bounds,
        ).select_bound_method(
            &[crate::selection::ReceiverCandidate {
                expr: crate::hir::HirExpr {
                    kind: crate::hir::HirExprKind::Var("allocator".into()),
                    ty: receiver.ty.clone(),
                    span: receiver.span.clone(),
                },
                adjustment: crate::selection::ReceiverAdjustment::None,
                can_autoref_mut: false,
            }],
            &method.generic_bounds[&allocator], "key", receiver.ty.clone(),
        ).expect("canonical shared receiver selection");
        assert_eq!(selection.receiver_adjustment, crate::selection::ReceiverAdjustment::AutorefShared);
        assert_eq!(selection.target.target, authority.target);
    }

    #[test]
    fn nominal_method_outputs_are_ready_before_unrelated_function_bodies() {
        let source = r#"
lang sized
< trait LayoutKnown
struct Storage
    < pointer: *I64
consumer = ->
    value = 7
    pointer = (&value) as *I64
    owner = Storage
        pointer: pointer
    raw = owner.pointer_view!
    view = owner.slice!
    (~I64Add (unsafe *raw), (~ArrayLen view))
impl Storage
    @pointer_view: *I64
    @pointer_view = -> self.pointer
    @slice: &[I64]
    @slice = -> unsafe (~BorrowSlice self.pointer, 1)
main = -> consumer!
"#;
        let analysis = crate::analyze(&owner_scheduling_config(source))
            .unwrap_or_else(|errors| panic!("nominal view results must reach accepted HIR: {errors:?}"));
        let program = &analysis.hir.program;
        let consumer = program.functions.values().find(|function| function.name == "consumer").unwrap();
        assert_eq!(consumer.ret_type, crate::types::Type::I64);
        let implementation = program.impls.values().find(|implementation| {
            implementation.methods.contains_key("pointer_view")
        }).unwrap();
        assert_eq!(implementation.methods["pointer_view"].ret_type,
            crate::types::Type::Pointer(Box::new(crate::types::Type::I64)));
        assert_eq!(implementation.methods["slice"].ret_type, crate::types::Type::Reference {
            mutable: false,
            inner: Box::new(crate::types::Type::Slice(Box::new(crate::types::Type::I64))),
        });
    }

    #[test]
    fn forward_static_owner_bodies_preserve_constructor_and_state_arguments() {
        for payload_first in [false, true] {
            for forward in [false, true] {
                for separate_impl in [false, true] {
                    let source = owner_scheduling_source(payload_first, forward, separate_impl);
                    let analysis = crate::analyze(&owner_scheduling_config(&source))
                        .unwrap_or_else(|errors| panic!("{payload_first}/{forward}/{separate_impl}: {errors:?}"));
                    let program = &analysis.hir.program;
                    let shell = program.structs.values().find(|item| item.name == "Shell").unwrap();
                    let state = program.structs.values().find(|item| item.name == "State").unwrap();
                    let state_field = shell.fields.iter().find(|field| field.name == "state").unwrap();
                    let allocator_slot = usize::from(payload_first);
                    assert_eq!(state_field.ty, crate::types::Type::Struct {
                        id: state.id,
                        args: vec![crate::types::Type::Generic(shell.generic_params[allocator_slot].id)],
                    });
                    let constructors = program.impls.values().flat_map(|implementation| {
                        implementation.methods.values().filter(|method| {
                            matches!(method.name.as_str(), "new" | "new_aligned")
                        })
                    }).collect::<Vec<_>>();
                    assert_eq!(constructors.len(), 2);
                    let aligned = *constructors.iter().find(|method| method.name == "new_aligned").unwrap();
                    let memory = program.traits.values().find(|item| item.name == "Memory").unwrap();
                    assert_parametric_allocator_method(aligned, memory);
                    for method in constructors {
                        assert_allocator_scheme_bound(method, memory);
                        let crate::types::Type::Struct { id, args } = &method.ret_type else {
                            panic!("constructor must retain its actual nominal result: {method:?}")
                        };
                        assert_eq!(*id, shell.id);
                        assert_eq!(args.len(), 2);
                        assert_eq!(args[allocator_slot], method.params[0].ty);
                        assert_eq!(args[1 - allocator_slot], method.params[1].ty);
                        assert!(args.iter().all(|ty| matches!(ty, crate::types::Type::Generic(_))),
                            "later callers must not specialize the producer's constructor: {method:?}");
                        if method.name == "new" {
                            let targets = scheduled_static_targets(&method.body);
                            assert_eq!(targets.len(), 1);
                            assert_eq!(targets[0].owner_ty, method.ret_type);
                            assert_eq!(targets[0].method.method_id(), Some(aligned.id));
                            let impl_id = targets[0].method.impl_id().expect("exact impl authority");
                            assert_eq!(program.impls[&impl_id].methods["new_aligned"].id, aligned.id);
                        }
                    }
                }
            }
        }
    }

    fn reject_owner_scheduling(source: &str, message: &str, offending: &str) {
        let config = owner_scheduling_config(source);
        let errors = crate::analyze(&config).expect_err("invalid static owner authority must reject");
        assert!(errors.0.iter().all(|error| error.code != Some(crate::diagnostic::DiagnosticCode::Internal)),
            "{errors:?}");
        let error = errors.0.iter().find(|error| error.message.contains(message))
            .unwrap_or_else(|| panic!("missing {message:?}: {errors:?}"));
        let span = &error.primary.as_ref().expect("source-backed static rejection").span;
        assert_eq!(span.file_path, config.entry_file);
        assert!(span.start < span.end && span.end <= source.len());
        assert!(source[span.start..span.end].contains(offending), "{span:?}");
    }

    #[test]
    fn static_constructor_context_is_independent_of_later_allocator_implementations() {
        for payload_first in [false, true] {
            let source = owner_scheduling_source(payload_first, true, false);
            let allocator_start = source.find("struct Small\n").unwrap();
            let allocator_end = source.find("struct State A\n").unwrap();
            let producer_end = source.find("\nmain = ->").unwrap();
            let source = format!("{}{}\nmain = !-> ()\n",
                &source[..allocator_start], &source[allocator_end..producer_end]);
            let analysis = crate::analyze(&owner_scheduling_config(&source))
                .unwrap_or_else(|errors| panic!("{payload_first}: {errors:?}"));
            let program = &analysis.hir.program;
            let memory = program.traits.values().find(|item| item.name == "Memory").unwrap();
            assert!(program.impls.values().all(|implementation| implementation.trait_id != Some(memory.id)));
            let aligned = program.impls.values().find_map(|implementation| {
                implementation.methods.get("new_aligned")
            }).unwrap();
            assert_parametric_allocator_method(aligned, memory);
            let wrapper = program.impls.values().find_map(|implementation| {
                implementation.methods.get("new")
            }).unwrap();
            for method in [wrapper, aligned] {
                assert_allocator_scheme_bound(method, memory);
                let crate::types::Type::Struct { args, .. } = &method.ret_type else {
                    panic!("actual generic constructor result: {method:?}")
                };
                assert_eq!(args.len(), 2);
                let allocator_slot = usize::from(payload_first);
                assert_eq!(args[allocator_slot], method.params[0].ty);
                assert_eq!(args[1 - allocator_slot], method.params[1].ty);
                assert!(args.iter().all(|ty| matches!(ty, crate::types::Type::Generic(_))));
            }
            let targets = scheduled_static_targets(&wrapper.body);
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].owner_ty, wrapper.ret_type);
            assert_eq!(targets[0].method.method_id(), Some(aligned.id));
        }
    }

    #[test]
    fn static_constructor_context_rejects_ambiguous_declared_protocols() {
        let source = owner_scheduling_source(false, true, false)
            .replace("struct Small\n", "trait OtherMemory\n    @key: I64\nstruct Small\n")
            .replace("where A: Memory\n", "where A: Memory, A: OtherMemory\n");
        reject_owner_scheduling(&source, "Ambiguous", "allocator");
    }

    #[test]
    fn static_constructor_context_does_not_discharge_a_nonimplementing_allocator() {
        for payload_first in [false, true] {
            for direct in [false, true] {
                let mut source = owner_scheduling_source(payload_first, true, false)
                    .replace("struct Small\n", "struct Invalid\n    < tag: I64\nstruct Small\n")
                    .replace("    small = Small\n", "    small = Invalid\n");
                if direct {
                    source = source.replace("Shell::new small", "Shell::new_aligned small");
                }
                let config = owner_scheduling_config(&source);
                let errors = crate::analyze(&config).expect_err("inferred Memory is still a checked obligation");
                assert!(errors.0.iter().all(|error| error.code != Some(crate::diagnostic::DiagnosticCode::Internal)),
                    "{errors:?}");
                assert!(errors.0.iter().any(|error| {
                    matches!(error.code, Some(crate::diagnostic::DiagnosticCode::Type
                        | crate::diagnostic::DiagnosticCode::Selection | crate::diagnostic::DiagnosticCode::Borrow))
                        && error.message.contains("Memory") && error.message.contains("Invalid")
                        && error.primary.as_ref().is_some_and(|label| {
                            label.span.file_path == config.entry_file && label.span.start < label.span.end
                                && label.span.end <= source.len()
                                && source[label.span.start..label.span.end].contains("Shell::new")
                        })
                }), "missing source-backed Invalid Memory rejection: {errors:?}");
            }
        }
    }

    #[test]
    fn forward_static_owner_rejects_a_different_result_constructor() {
        let source = owner_scheduling_source(false, true, false).replace(
            "struct Shell A, T\n",
            "struct Other A, T\n    < value: T\n    < state: State A\nstruct Shell A, T\n",
        ).replace("        Shell\n", "        Other\n");
        reject_owner_scheduling(&source, "could not infer an owner generic parameter", "Shell::new_aligned");
    }

    #[test]
    fn forward_static_owner_rejects_a_signature_without_a_body() {
        let source = r#"
lang sized
< trait LayoutKnown
struct Shell A, T
    < allocator: A
    < value: T
impl Shell A, T
    new = allocator, value -> Shell::missing allocator, value
    missing: A -> T -> Shell A, T
main = !-> ()
"#;
        reject_owner_scheduling(source, "no matching static implementation", "Shell::missing");
    }

    #[test]
    fn static_owner_without_signature_evidence_remains_an_error() {
        let source = r#"
lang sized
< trait LayoutKnown
struct Shell A, T
    < allocator: A
    < value: T
impl Shell A, T
    phantom = -> 42
main = -> Shell::phantom!
"#;
        reject_owner_scheduling(source, "could not infer an owner generic parameter", "Shell::phantom");
    }

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
                name: Some(ast::Ident {
                    name: "left_inline".to_string(),
                    span: crate::lexer::Span::test(),
                }),
                top_levels: vec![ast::TopLevel::Module(ast::ModuleDecl(ast::Module {
                    name: Some(ast::Ident {
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
                name: Some(ast::Ident {
                    name: "right_inline".to_string(),
                    span: crate::lexer::Span::test(),
                }),
                top_levels: vec![ast::TopLevel::Module(ast::ModuleDecl(ast::Module {
                    name: Some(ast::Ident {
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
