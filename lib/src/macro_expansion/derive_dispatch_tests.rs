use crate::ast::{DeriveTarget, DerivedDecl, MacroInvoc, MacroOutputCategory, Program, TopLevel};
use crate::macro_expansion::proc_macro::*;
use crate::{parser, Config};

use super::{expand_macros_with_context, ExpansionId, MacroExpansionContext};

fn parse(text: &str) -> Program { parser::parse_string(text, &Config::default()).unwrap() }

fn declaration(program: &Program) -> &DerivedDecl {
    program.module.top_levels.iter().find_map(|item| match item { TopLevel::DerivedDecl(node) => Some(node), _ => None }).unwrap()
}

fn invocation(node: &DerivedDecl, index: usize) -> MacroInvoc {
    let attachment = &node.derives[index];
    MacroInvoc { id: attachment.id, parent: attachment.parent, path: attachment.path.clone(), span: attachment.span.clone(),
        args: node.target.original().unwrap().stream().unwrap(), header_indent: node.target.original().unwrap().header_indent(),
        output: MacroOutputCategory::Items, arguments: Vec::new(), literal_ranges: Vec::new(), block: None }
}

fn worker(name: &str, response: ProcMacroResponse) -> (ProcMacroArtifact, std::path::PathBuf) {
    let (mut artifact, root) = super::tests::proc_macro_artifact_with_response(name, response);
    artifact.crate_identity = format!("provider_{name}");
    artifact.exports[0].identity = format!("provider_{name}::{name}");
    artifact.exports[0].kind = ProcMacroKind::DeriveLike;
    (artifact, root)
}

fn empty() -> ProcMacroResponse {
    ProcMacroResponse { protocol_version: PROC_MACRO_PROTOCOL_VERSION, output: Some(ProcMacroOutput { parts: Vec::new() }), diagnostics: Vec::new() }
}

fn request(artifact: &ProcMacroArtifact) -> ProcMacroRequest {
    decode_request(&std::fs::read(artifact.executable.with_extension("request")).unwrap()).unwrap()
}

#[test]
fn macro_c_09_derives_share_exact_original_input_and_append_in_order_before_expressions() {
    let cfg = Config::default();
    let source = "%derive one::First, two::Second\n< struct S T, {U = T}\r\n    // 雪\r\n    < value: T = %later!\r\n\r\nafter = -> 0\n";
    let program = parse(source);
    let node = declaration(&program);
    let original = node.target.original().unwrap().clone();
    let first = invocation(node, 0);
    let second = invocation(node, 1);
    let (one, root_one) = worker("First", super::tests::constructed_response(&first, "macro later\n    =>\n        7\nfirst = -> 1\n"));
    let (two, root_two) = worker("Second", super::tests::constructed_response(&second, "second = -> 2\n"));
    let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("one".into(), one.clone()).with_named_proc_macro_artifact("two".into(), two.clone());
    let output = expand_macros_with_context(program, &context).unwrap();
    assert_eq!(output.module.top_levels.iter().filter_map(TopLevel::get_ident).collect::<Vec<_>>(), ["S", "later", "first", "second", "after"]);
    let TopLevel::StructDecl(retained) = &output.module.top_levels[0] else { panic!("original struct") };
    assert_eq!(retained.original.as_ref().unwrap(), &original);
    assert_eq!(request(&one).input, request(&two).input);
    assert_eq!(request(&one).input, encode_stream(&original.stream().unwrap()).unwrap());
    for (artifact, call) in [(&one, &first), (&two, &second)] {
        let request = request(artifact);
        assert_eq!(request.kind, ProcMacroKind::DeriveLike);
        assert_eq!(request.output_category, MacroOutputCategory::Items);
        assert_eq!(request.invocation_id, call.id);
        assert_eq!(request.block_base, None);
        assert_eq!(request.views, ProcMacroViews::default());
    }
    assert_eq!(context.latest_expansion_id(), Some(ExpansionId(3)), "two derives then the default expression macro");
    std::fs::remove_dir_all(root_one).unwrap(); std::fs::remove_dir_all(root_two).unwrap();
}

#[test]
fn macro_c_09_empty_results_keep_structs_enums_and_consume_by_node_identity() {
    let cfg = Config::default();
    let mut program = parse("%derive dep::Empty\nstruct Same\n");
    let original = program.module.top_levels[0].clone();
    program.module.top_levels.push(original.clone());
    program.module.top_levels.extend(parse("%derive dep::Empty\nstruct Same\n%derive dep::Empty\nenum E\n    A\n").module.top_levels);
    let (artifact, root) = worker("Empty", empty());
    let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("dep".into(), artifact);
    let output = expand_macros_with_context(program, &context).unwrap();
    assert_eq!(output.module.top_levels.len(), 3, "one retained declaration per node ID");
    assert!(output.module.top_levels[..2].iter().all(|item| matches!(item, TopLevel::StructDecl(_))));
    assert!(matches!(output.module.top_levels[2], TopLevel::EnumDecl(_)));
    assert_eq!(context.latest_expansion_id(), Some(ExpansionId(3)), "cloned node once, distinct same-spelling node independently");
    let TopLevel::DerivedDecl(node) = original else { panic!("derived") };
    assert!(matches!(node.target, DeriveTarget::Struct(_)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn macro_c_09_generated_attachments_reenter_discovery_with_parent_and_shared_limits() {
    for limited in [false, true] {
        let cfg = Config::default();
        let program = parse("%derive make::Make\nstruct Original\n");
        let call = invocation(declaration(&program), 0);
        let (make, root_make) = worker("Make", super::tests::constructed_response(&call, "%derive dep::Empty\nstruct Generated\n"));
        let (empty_worker, root_empty) = worker("Empty", empty());
        let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("make".into(), make).with_named_proc_macro_artifact("dep".into(), empty_worker.clone())
            .with_max_depth(if limited { 1 } else { 10 });
        let output = expand_macros_with_context(program, &context);
        if limited {
            let errors = output.unwrap_err();
            assert!(format!("{errors:?}").contains("depth exceeded"));
            assert!(!empty_worker.executable.with_extension("request").exists());
        } else {
            let output = output.unwrap();
            assert_eq!(output.module.top_levels.iter().filter_map(TopLevel::get_ident).collect::<Vec<_>>(), ["Original", "Generated"]);
            assert_eq!(context.source_map().record(ExpansionId(2)).unwrap().parent, Some(ExpansionId(1)));
        }
        std::fs::remove_dir_all(root_make).unwrap(); std::fs::remove_dir_all(root_empty).unwrap();
    }
}

#[test]
fn macro_c_09_original_cannot_be_replaced_and_non_item_output_rejects() {
    for text in ["struct Replacement\n", "42"] {
        let cfg = Config::default();
        let program = parse("%derive dep::Make\nstruct Original\n");
        let call = invocation(declaration(&program), 0);
        let (artifact, root) = worker("Make", super::tests::constructed_response(&call, text));
        let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("dep".into(), artifact);
        let result = expand_macros_with_context(program, &context);
        if text == "42" { assert!(result.is_err()); }
        else { assert_eq!(result.unwrap().module.top_levels.iter().filter_map(TopLevel::get_ident).collect::<Vec<_>>(), ["Original", "Replacement"]); }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn macro_c_09_no_execution_flag_rejects_selected_derives_without_starting_a_worker() {
    let cfg = Config { no_proc_execution: true, ..Config::default() };
    let program = parse("%derive dep::Empty\nstruct Original\n");
    let (artifact, root) = worker("Empty", empty());
    let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("dep".into(), artifact.clone());
    let errors = expand_macros_with_context(program, &context).unwrap_err();
    assert!(format!("{errors:?}").contains("execution is disabled"));
    assert!(!artifact.executable.with_extension("request").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn macro_c_09_duplicates_and_wrong_kinds_reject_before_worker_execution() {
    for duplicate in [false, true] {
        let cfg = Config::default();
        let (mut artifact, root) = worker("Empty", empty());
        let source = if duplicate { "> dep::Empty\n%derive Empty, dep::Empty\nstruct S\n" } else {
            artifact.exports[0].kind = ProcMacroKind::FunctionLike;
            "%derive dep::Empty\nstruct S\n"
        };
        let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("dep".into(), artifact.clone());
        let errors = expand_macros_with_context(parse(source), &context).unwrap_err();
        assert!(format!("{errors:?}").contains(if duplicate { "duplicate canonical" } else { "DeriveLike" }));
        assert!(!artifact.executable.with_extension("request").exists());
        assert_eq!(context.latest_expansion_id(), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn macro_c_09_generated_imports_can_resolve_deferred_item_and_derive_paths() {
    let cfg = Config::default();
    let program = parse("%finish\n%derive Finish\nstruct Waiting\n%derive dep::Publish\nstruct Publisher\nmod tools\n    < finish\n    macro finish\n        =>\n            completed = -> 7\n\n");
    let publishing = program.module.top_levels.iter().find_map(|item| match item {
        TopLevel::DerivedDecl(node) if node.target.name().name == "Publisher" => Some(node), _ => None,
    }).unwrap();
    let call = invocation(publishing, 0);
    let (artifact, root) = worker("Publish", super::tests::constructed_response(&call, "> tools::finish\n> quiet::Finish\n"));
    let (quiet, quiet_root) = worker("Finish", empty());
    let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("dep".into(), artifact).with_named_proc_macro_artifact("quiet".into(), quiet);
    let output = expand_macros_with_context(program, &context).unwrap();
    assert!(output.module.top_level_from_ident("completed").is_some());
    assert!(output.module.top_levels.iter().filter(|item| matches!(item, TopLevel::StructDecl(_))).count() == 2);
    assert_eq!(context.latest_expansion_id(), Some(ExpansionId(3)));
    std::fs::remove_dir_all(root).unwrap(); std::fs::remove_dir_all(quiet_root).unwrap();
}

#[test]
fn macro_c_09_derived_file_modules_reenter_source_discovery() {
    let cfg = Config { entry_file: "/macro-c09/main.rk".into(), current_crate_name: Some("app".into()), ..Config::default() };
    let mut sources = crate::source_loader::SourceDatabase::new();
    sources.add_virtual_source(cfg.entry_file.clone(), "%derive dep::Make\nstruct Root\n");
    sources.add_virtual_source("/macro-c09/child.rk".into(), "%derive quiet::Empty\nenum E\n    A\n");
    let mut graph = sources.load_entry(cfg.entry_file.clone(), &cfg).unwrap();
    let call = invocation(declaration(&Program { module: graph.root_module().clone() }), 0);
    let (make, root_make) = worker("Make", super::tests::constructed_response(&call, "mod child\n"));
    let (quiet, root_quiet) = worker("Empty", empty());
    let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("dep".into(), make).with_named_proc_macro_artifact("quiet".into(), quiet.clone());
    super::expand_macros_in_graph(&mut graph, &mut sources, &context).unwrap();
    assert!(graph.modules().any(|module| module.module.top_levels.iter().any(|item| matches!(item, TopLevel::EnumDecl(_)))));
    assert!(matches!(graph.root_module().top_levels[0], TopLevel::StructDecl(_)));
    assert_eq!(request(&quiet).kind, ProcMacroKind::DeriveLike);
    std::fs::remove_dir_all(root_make).unwrap(); std::fs::remove_dir_all(root_quiet).unwrap();
}

#[test]
fn macro_c_09_invocation_and_output_budgets_cover_derive_workers() {
    for invocation_limit in [false, true] {
        let cfg = Config::default();
        let program = parse("%derive dep::Make\nstruct S\n");
        let call = invocation(declaration(&program), 0);
        let (artifact, root) = worker("Make", super::tests::constructed_response(&call, "struct Generated\n"));
        let limits = if invocation_limit { super::ExpansionLimits { max_invocations: 0, ..Default::default() } }
            else { super::ExpansionLimits { max_output_bytes: 1, ..Default::default() } };
        let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("dep".into(), artifact.clone()).with_limits(limits);
        let errors = expand_macros_with_context(program, &context).unwrap_err();
        assert!(format!("{errors:?}").contains(if invocation_limit { "invocation budget" } else { "output byte budget" }));
        assert_eq!(artifact.executable.with_extension("request").exists(), !invocation_limit);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn macro_c_09_ready_derives_and_item_invocations_keep_one_source_order() {
    let cfg = Config::default();
    let program = parse("%derive one::First\nstruct A\n%middle::Emit\n%derive two::Last\nenum B\n    Empty\n");
    let calls = program.module.top_levels.iter().filter_map(|item| match item {
        TopLevel::DerivedDecl(node) => Some(invocation(node, 0)), TopLevel::MacroInvoc(call) => Some(call.clone()), _ => None,
    }).collect::<Vec<_>>();
    let (one, root_one) = worker("First", super::tests::constructed_response(&calls[0], "first = -> 1\n"));
    let (mut middle, root_middle) = worker("Emit", super::tests::constructed_response(&calls[1], "middle = -> 2\n"));
    middle.exports[0].kind = ProcMacroKind::FunctionLike;
    let (two, root_two) = worker("Last", super::tests::constructed_response(&calls[2], "last = -> 3\n"));
    let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("one".into(), one)
        .with_named_proc_macro_artifact("middle".into(), middle).with_named_proc_macro_artifact("two".into(), two);
    let output = expand_macros_with_context(program, &context).unwrap();
    assert_eq!(output.module.top_levels.iter().filter_map(TopLevel::get_ident).collect::<Vec<_>>(), ["A", "first", "middle", "B", "last"]);
    let map = context.source_map();
    assert_eq!((1..=3).map(|index| map.record(ExpansionId(index)).unwrap().macro_name.as_str()).collect::<Vec<_>>(), ["First", "Emit", "Last"]);
    for root in [root_one, root_middle, root_two] { std::fs::remove_dir_all(root).unwrap(); }
}
