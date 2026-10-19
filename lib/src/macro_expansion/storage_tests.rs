use super::*;
use super::token_tree::{source, MACRO_LIMITS};
use crate::ast::MacroOutputCategory;
use crate::parser::parse_string;

fn text(input: &TokenStream) -> String {
    input.runs().iter().map(|run| run.text()).collect()
}

fn invocation(program: &Program) -> &MacroInvoc {
    program.module.top_levels.iter().find_map(|item| match item {
        TopLevel::MacroInvoc(invocation) => Some(invocation),
        _ => None,
    }).unwrap()
}

#[test]
fn macro_a_01_equal_spans_have_distinct_node_identities() {
    let config = crate::Config::default();
    let left = parse_string("%same input", &config).unwrap();
    let right = parse_string("%same input", &config).unwrap();
    let left = invocation(&left);
    let right = invocation(&right);
    assert_eq!(left.name.span, right.name.span);
    assert_ne!(left.id, right.id);
    assert_eq!(left.id, left.clone().id);
    assert_eq!(left.output, MacroOutputCategory::Items);
    assert_eq!(left.parent, None);
}

#[test]
fn macro_a_01_generated_equal_span_nodes_keep_explicit_parent_edges() {
    let config = crate::Config::default();
    let context = MacroExpansionContext::new(&config);
    let parent = |name: &str| context.record_expansion(MacroExpansionRecord {
        invocation: crate::ast::InvocationId::fresh().unwrap(),
        macro_name: name.into(), invocation_span: source(name).start_span(),
        definition_span: None, parent: None,
    });
    let first = parent("first");
    let second = parent("second");
    let output = source("%child");
    let left = parse_generated_top_levels(output.clone(), &context, Some(first)).unwrap();
    let right = parse_generated_top_levels(output, &context, Some(second)).unwrap();
    let (TopLevel::MacroInvoc(left), TopLevel::MacroInvoc(right)) = (&left[0], &right[0]) else { panic!("expected children") };
    assert_eq!(left.name.span, right.name.span);
    assert_ne!(left.id, right.id);
    assert_eq!(left.parent, Some(first));
    assert_eq!(right.parent, Some(second));
}

#[test]
fn macro_a_01_all_input_shapes_roundtrip_without_rock_interpretation() {
    for spelling in [
        "1, (f 2, 3)  /* inline trivia */",
        "struct Thing\r\n    field : I64\r\n",
        "\n    <h1>🦀</h1>\r\n\n    \"broken /* unmatched [\n",
        "",
    ] {
        let input = source(spelling);
        let encoded = proc_macro::encode_stream(&input).unwrap();
        let output = proc_macro::decode_stream(&encoded).unwrap();
        assert_eq!(text(&output), spelling);
        assert_eq!(output.start_span(), input.start_span());
        assert_eq!(output.end_span(), input.end_span());
        assert_eq!(proc_macro::encode_stream(&output).unwrap(), encoded);
    }
}

#[test]
fn macro_a_01_ast_invocation_and_template_keep_trivia_and_original_margins() {
    let config = crate::Config::default();
    let code = "macro keep\r\n    $v:expr =>\r\n        /* retained */ $v\r\n%keep  1 /* caller */ +  2\r\n";
    let program = parse_string(code, &config).unwrap();
    let TopLevel::MacroDecl(decl) = &program.module.top_levels[0] else { panic!("expected macro") };
    assert_eq!(text(&decl.entries[0].input), "    $v:expr =>\r\n        /* retained */ $v\r\n");
    assert_eq!(text(&invocation(&program).args), "  1 /* caller */ +  2\r\n");
    let (tokens, ranges) = macro_arg_matcher::rock_arguments(&invocation(&program).args).unwrap();
    let context = MacroExpansionContext::new(&config);
    let compiled = DeclarativeMacro::from_ast(decl);
    let captures = MacroArgMatcher::new(&tokens, &invocation(&program).args, &ranges,
        compiled.arms[0].matcher.fragments.clone(), decl.name.span.clone(), invocation(&program).name.span.clone(), &context,
    ).run().unwrap();
    assert_eq!(text(&captures.entries["v"][0]), "1 /* caller */ +  2");
}

#[test]
fn macro_a_01_old_token_payloads_and_requests_reject() {
    use bincode::Options;
    let tokens = vec![crate::lexer::Token::from(crate::lexer::TokenType::Ident("old".into()))];
    let bytes = bincode::DefaultOptions::new().serialize(&tokens).unwrap();
    assert!(proc_macro::decode_stream(&bytes).is_err());
    assert!(proc_macro::decode_stream(&[]).is_err());

    #[derive(serde::Serialize)]
    struct OldRequest {
        protocol_version: u32, compiler_version: String, macro_name: String,
        macro_identity: String, source_module: Option<String>, expansion_id: Option<u32>, input: Vec<u8>,
    }
    let bytes = bincode::DefaultOptions::new().serialize(&OldRequest {
        protocol_version: 2, compiler_version: "old".into(), macro_name: "old".into(),
        macro_identity: "old::old".into(), source_module: None, expansion_id: None, input: bytes,
    }).unwrap();
    assert!(proc_macro::decode_request(&bytes).is_err());
}

#[test]
fn macro_a_01_worker_receives_lossless_input_and_invocation_metadata() {
    let config = crate::Config::default();
    let program = parse_string("%native  1 /* trivia */ + 2", &config).unwrap();
    let invocation = invocation(&program).clone();
    let (artifact, directory) = super::tests::proc_macro_artifact_with_response("native", ProcMacroResponse::Expand {
        output: proc_macro::encode_stream(&source("main = -> 0")).unwrap(),
    });
    let request_file = artifact.executable.with_extension("request");
    let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);
    expand_macros_with_context(program, &context).unwrap();
    let request = proc_macro::decode_request(&std::fs::read(request_file).unwrap()).unwrap();
    assert_eq!(request.protocol_version, 3);
    assert_eq!(request.invocation_id, invocation.id);
    assert_eq!(request.parent, None);
    assert_eq!(request.output_category, MacroOutputCategory::Items);
    assert_eq!(text(&proc_macro::decode_stream(&request.input).unwrap()), "  1 /* trivia */ + 2");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn macro_a_01_origin_tagging_keeps_template_snapshot_adjacency() {
    let input = source("foo!");
    let left = input.slice(0..3, MACRO_LIMITS).unwrap().generated(ExpansionId(1), GeneratedSourceId(1));
    let right = input.slice(3..4, MACRO_LIMITS).unwrap().generated(ExpansionId(1), GeneratedSourceId(1));
    let joined = left.join(&right, crate::lexer::lossless::JoinBoundary::SourceAdjacent, MACRO_LIMITS).unwrap();
    assert_eq!(text(&joined), "foo!");
    let decoded = proc_macro::decode_stream(&proc_macro::encode_stream(&joined).unwrap()).unwrap();
    assert_eq!(decoded.runs()[0].origin(), joined.runs()[0].origin());
    assert_eq!(decoded.runs()[1].origin(), joined.runs()[1].origin());
}

#[test]
fn macro_a_01_recapturing_generated_input_retains_its_source_location() {
    let generated = source("prefix x").slice(7..8, MACRO_LIMITS).unwrap()
        .generated(ExpansionId(1), GeneratedSourceId(1));
    let invocation_span = source("%child").start_span();
    let captured = generated.captured(ExpansionId(2), &invocation_span);
    assert_eq!(captured.start_span(), generated.start_span());
    assert_eq!(captured.end_span(), generated.end_span());
    let empty = captured.slice(0..0, MACRO_LIMITS).unwrap();
    assert_eq!(empty.start_span(), generated.start_span());
    let decoded = proc_macro::decode_stream(&proc_macro::encode_stream(&captured).unwrap()).unwrap();
    assert_eq!(decoded.start_span(), generated.start_span());
}
