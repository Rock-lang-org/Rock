use crate::ast::*;
use crate::ast::visit::{self, Visitor};
use crate::Config;

use super::{expand_macros_with_context, MacroExpansionContext};

const ID: &str = "macro id\n    $value:expr =>\n        $value\n";

fn parse(text: &str) -> Program {
    crate::parser::parse_string(text, &Config::default()).unwrap()
}

fn pending(module: &Module) -> usize {
    struct Pending(usize);
    impl<'a> Visitor<'a> for Pending {
        fn visit_macro_invoc(&mut self, _: &'a MacroInvoc) { self.0 += 1; }
    }
    let mut pending = Pending(0);
    visit::walk_module(&mut pending, module);
    pending.0
}

fn expand(text: &str) -> Program {
    let config = Config::default();
    expand_macros_with_context(parse(text), &MacroExpansionContext::new(&config)).unwrap()
}

#[test]
fn macro_a_08_declaration_defaults_and_trait_impl_methods_share_the_visitor() {
    let text = format!("{ID}struct Point\n    value: Int = %id 42\ntrait Show\n    show = -> %id 7\nimpl Show for Point\n    show = -> %id 9\n");
    assert_eq!(pending(&parse(&text).module), 3);
    let expanded = expand(&text);
    assert_eq!(pending(&expanded.module), 0);
}

#[test]
fn macro_a_08_inline_and_loaded_file_modules_use_their_own_macro_scopes() {
    let config = Config { current_crate_name: Some("app".into()), ..Config::default() };
    let mut sources = crate::source_loader::SourceDatabase::new();
    sources.add_virtual_source("/macro-traversal/main.rk".into(), "mod tools\nmod inline\n    macro local\n        =>\n            9\n    inner = -> %local!\nmain = -> %tools::local!\n");
    sources.add_virtual_source("/macro-traversal/tools.rk".into(), "< local\nmacro local\n    =>\n        42\nmacro item\n    =>\n        made = -> %local!\n%item\nwork = -> %local!\nmod nested\n    > app::tools::local\n    run = -> %local!\n");
    let mut graph = sources.load_entry("/macro-traversal/main.rk".into(), &config).unwrap();
    let expanded = super::expand_macros_in_graph(&mut graph, &mut sources, &MacroExpansionContext::new(&config)).unwrap();
    assert_eq!(pending(&expanded.module), 0);
    assert_eq!(pending(graph.root_module()), 0);
    let tools = graph.module_by_qualified_name("app::tools").unwrap();
    assert_eq!(pending(&tools.module), 0);
    assert!(tools.module.top_level_from_ident("made").is_some());
    assert_eq!(pending(&graph.module_file_cache()[&tools.path]), 0);
    let count = graph.modules().count();
    sources.load_module_declarations(&mut graph, &expanded.module, Some("app"), &config).unwrap();
    assert_eq!(graph.modules().count(), count);
    assert_eq!(pending(&graph.module_by_qualified_name("app::tools").unwrap().module), 0);
}

#[test]
fn macro_a_08_nested_inline_item_calls_resolve_without_root_injection() {
    let expanded = expand("mod inner\n    macro item\n        =>\n            work = -> 42\n    %item\n");
    let TopLevel::Module(inner) = &expanded.module.top_levels[0] else { panic!("module") };
    assert!(inner.0.top_level_from_ident("work").is_some());
    assert_eq!(pending(&expanded.module), 0);
}

#[test]
fn macro_a_08_every_ordinary_expression_container_is_visited() {
    let text = format!("{ID}main = ->\n    x = [(%id 1), (%id 2)]\n    y = ((%id 3), (%id 4))\n    callback = (-> %id 5)\n    z = Point value: (%id 6)\n    result = consume (%id 7), (%id 8)\n    x[(%id 0)] = %id 9\n    if (%id true)\n        %id 10\n    else if (%id false)\n        %id 11\n    else\n        %id 12\n    while (%id false)\n        break (%id 13)\n    for item in (%id x)\n        continue (%id 14)\n    unsafe\n        return (%id 15)\n");
    let input = parse(&text);
    assert!(pending(&input.module) >= 20);
    assert_eq!(pending(&expand(&text).module), 0);
}

#[test]
fn macro_a_08_outer_calls_discard_nested_unknown_invocations_in_every_field() {
    let text = "macro discard\n    $value:expr =>\n        42\nstruct Point\n    value: Int = %discard (%never!)\ntrait Show\n    show = -> %discard (%never!)\nimpl Show for Point\n    show = -> %discard (%never!)\nmain = ->\n    [%discard (%never!)]\n";
    assert_eq!(pending(&expand(text).module), 0);
}

#[test]
fn macro_a_08_unused_loaded_declaration_errors_are_not_skipped() {
    let config = Config { current_crate_name: Some("app".into()), ..Config::default() };
    let mut sources = crate::source_loader::SourceDatabase::new();
    sources.add_virtual_source("/macro-traversal-errors/main.rk".into(), "mod tools\nmain = -> 0\n");
    sources.add_virtual_source("/macro-traversal-errors/tools.rk".into(), "struct Point\n    value: Int = %missing!\n");
    let mut graph = sources.load_entry("/macro-traversal-errors/main.rk".into(), &config).unwrap();
    let diagnostics = super::expand_macros_in_graph(&mut graph, &mut sources, &MacroExpansionContext::new(&config)).unwrap_err();
    assert!(format!("{diagnostics:?}").contains("tools.rk"));
}

#[test]
fn macro_a_08_graph_expansion_reaches_normal_collection_without_raw_reload() {
    let config = Config {
        entry_file: "/macro-traversal-analyze/main.rk".into(),
        current_crate_name: Some("app".into()),
        no_prelude: true,
        source_providers: vec![
            crate::SourceProvider::Virtual {
                path: "/macro-traversal-analyze/main.rk".into(),
                text: "mod tools\nmain = -> tools::work!\n".into(),
            },
            crate::SourceProvider::Virtual {
                path: "/macro-traversal-analyze/tools.rk".into(),
                text: "macro number\n    =>\n        42\n< work = -> %number!\n".into(),
            },
        ],
        ..Config::default()
    };
    crate::analyze(&config).unwrap();
}

#[test]
fn macro_a_08_match_guards_ranges_casts_and_nested_control_flow_are_visited() {
    let text = format!("{ID}main = ->\n    range = (%id 0)..(%id 3)\n    cast = (%id 1) as Int\n    loop\n        break (%id 2)\n    match (%id 1)\n        1 if (%id true) => %id 42\n        _ => %id 0\n");
    assert_eq!(pending(&parse(&text).module), 7);
    assert_eq!(pending(&expand(&text).module), 0);
}

#[test]
fn macro_a_08_synthetic_pattern_literals_and_enum_defaults_have_no_unvisited_fields() {
    // These AST-owned fields admit expression-bearing literal containers even
    // where the current surface pattern grammar is deliberately more limited.
    let mut input = parse(&format!("{ID}main = -> %id 42\nstruct Point\n    value: Int = %id 7\nenum Choice\n    One\n"));
    let TopLevel::FunctionDecl(function) = &mut input.module.top_levels[1] else { panic!("function") };
    let value = || {
        let mut parsed = parse("fresh = -> %id 42\n");
        let TopLevel::FunctionDecl(mut function) = parsed.module.top_levels.remove(0) else { panic!("function") };
        let Statement::Expression(value) = function.lambda.body.statements.remove(0) else { panic!("expression") };
        value
    };
    let span = function.name.span.clone();
    let pattern = || Pattern {
        binding: None,
        kind: PatternKind::Literal(Literal { span: span.clone(), kind: LiteralKind::ArrayRepeat { value: Box::new(value()), len: 2 } }),
    };
    let nested = Pattern { binding: None, kind: PatternKind::Reference {
        mutable: false, pattern: Box::new(Pattern { binding: None, kind: PatternKind::Nested(Box::new(pattern())) }),
    } };
    function.lambda.parameters = vec![Pattern { binding: None, kind: PatternKind::Tuple(vec![nested]) }];
    function.lambda.body.statements.push(Statement::Assignment(Assignment {
        lhs: AssignmentLHS::Pattern { pattern: Pattern { binding: None, kind: PatternKind::Array(vec![ArrayPattern::Pattern(pattern())]) }, type_annotation: None },
        rhs: value(),
    }));
    let instance = Pattern { binding: None, kind: PatternKind::Instance(InstancePattern {
        name: TypePath { path: Vec::new() }, args: FieldsPatternOrArgumentsPattern::Fields(vec![FieldPattern {
            name: Ident { name: "value".into(), span: span.clone() }, pattern: pattern(),
        }]),
    }) };
    function.lambda.body.statements.push(Statement::Expression(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::If(Box::new(If {
            condition: Condition { pattern: Some(instance), expression: value() },
            then: Block { statements: vec![Statement::Expression(value())] }, else_: None,
        })), secondaries: None, type_annotation: None,
    }))));
    let TopLevel::StructDecl(point) = &input.module.top_levels[2] else { panic!("struct") };
    let mut fields = point.fields.clone();
    fields[0].default = Some(value());
    let TopLevel::EnumDecl(choice) = &mut input.module.top_levels[3] else { panic!("enum") };
    choice.variants[0].fields = NamedFieldsOrTypesList::NamedFields(fields);
    assert_eq!(pending(&input.module), 9);
    let config = Config::default();
    let output = expand_macros_with_context(input, &MacroExpansionContext::new(&config)).unwrap();
    assert_eq!(pending(&output.module), 0);
}

#[test]
fn macro_a_08_loaded_procedural_item_executes_once_and_discarded_inputs_never_run() {
    let config = Config { current_crate_name: Some("app".into()), ..Config::default() };
    let (provider, directory) = super::tests::proc_macro_artifact_with_response("make", super::proc_macro::ProcMacroResponse::Expand {
        output: super::proc_macro::encode_stream(&super::token_tree::source("made = -> 42\n")).unwrap(),
    });
    let context = MacroExpansionContext::new(&config).with_named_proc_macro_artifact("provider".into(), provider);
    let mut sources = crate::source_loader::SourceDatabase::new();
    sources.add_virtual_source("/macro-traversal-worker/main.rk".into(), "mod tools\nmain = -> 0\n");
    sources.add_virtual_source("/macro-traversal-worker/tools.rk".into(), "> provider::make\n%make\nmacro discard\n    $value:expr =>\n        7\nstruct Point\n    value: Int = %discard (%provider::make!)\n");
    let mut graph = sources.load_entry("/macro-traversal-worker/main.rk".into(), &config).unwrap();
    let result = super::expand_macros_in_graph(&mut graph, &mut sources, &context);
    std::fs::remove_dir_all(directory).unwrap();
    result.unwrap();
    let tools = &graph.module_by_qualified_name("app::tools").unwrap().module;
    assert!(tools.top_level_from_ident("made").is_some());
    assert_eq!(pending(tools), 0);
    assert_eq!(context.latest_expansion_id(), Some(super::ExpansionId(2)));
}
