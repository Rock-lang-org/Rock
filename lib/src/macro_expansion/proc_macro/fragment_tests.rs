use super::*;
use crate::lexer::lossless::{ConstructionContext, ConstructionKind};
use crate::macro_expansion::{expand_macros_with_context, GeneratedSourceId, MacroExpansionContext};

#[test]
fn macro_b_19_deferred_generated_source_uses_normal_item_parsing() {
    let config = crate::Config::default();
    let program = crate::parser::parse_string("%provider::make!\n", &config).unwrap();
    let invocation = crate::macro_expansion::tests::first_invocation(&program);
    let response = ProcMacroResponse { protocol_version: PROC_MACRO_PROTOCOL_VERSION, diagnostics: Vec::new(),
        output: Some(ProcMacroOutput { parts: vec![ProcMacroPart::Source {
            text: "main = -> 42\n".into(), context: ConstructionContext { invocation: invocation.id, kind: ConstructionKind::CallSite },
        }] }),
    };
    let (artifact, directory) = crate::macro_expansion::tests::proc_macro_artifact_with_response("make", response);
    let context = MacroExpansionContext::new(&config).with_named_proc_macro_artifact("provider".into(), artifact);
    let result = expand_macros_with_context(program, &context);
    std::fs::remove_dir_all(directory).unwrap();
    let expanded = result.unwrap();
    assert!(matches!(&expanded.module.top_levels[0], crate::ast::TopLevel::FunctionDecl(function) if function.name.name == "main"));
}

#[test]
fn macro_b_19_literal_holes_preserve_exact_caller_subspans_after_escapes() {
    let config = crate::Config::default();
    let source = "main = -> %provider::hole \"é\\nmissing\"\n";
    let program = crate::parser::parse_string(source, &config).unwrap();
    let response = ProcMacroResponse { protocol_version: PROC_MACRO_PROTOCOL_VERSION, diagnostics: Vec::new(),
        output: Some(ProcMacroOutput { parts: vec![ProcMacroPart::Literal { view: 0, range: 3..10 }] }),
    };
    let (artifact, directory) = crate::macro_expansion::tests::proc_macro_artifact_with_response("hole", response);
    let context = MacroExpansionContext::new(&config).with_named_proc_macro_artifact("provider".into(), artifact);
    let result = expand_macros_with_context(program, &context);
    std::fs::remove_dir_all(directory).unwrap();
    let expanded = result.unwrap();
    struct Names(Vec<crate::lexer::Span>);
    impl<'a> crate::ast::visit::Visitor<'a> for Names {
        fn visit_ident(&mut self, ident: &'a crate::ast::Ident) {
            if ident.name == "missing" { self.0.push(ident.span.clone()); }
        }
    }
    let mut names = Names(Vec::new());
    crate::ast::visit::walk_module(&mut names, &expanded.module);
    assert_eq!(names.0.len(), 1);
    let mapped = context.source_map().original_span(&names.0[0]);
    let start = source.find("missing").unwrap();
    assert_eq!(mapped.start..mapped.end, start..start + 7);
}

#[test]
fn macro_b_19_escape_scalars_and_multibyte_extents_map_without_linear_guessing() {
    let config = crate::Config::default();
    let source = "main = -> %provider::hole \"\\\"é\\n\\\"\"\n";
    let program = crate::parser::parse_string(source, &config).unwrap();
    let response = ProcMacroResponse { protocol_version: PROC_MACRO_PROTOCOL_VERSION, diagnostics: Vec::new(),
        output: Some(ProcMacroOutput { parts: vec![ProcMacroPart::Literal { view: 0, range: 0..5 }] }),
    };
    let (artifact, directory) = crate::macro_expansion::tests::proc_macro_artifact_with_response("hole", response);
    let context = MacroExpansionContext::new(&config).with_named_proc_macro_artifact("provider".into(), artifact);
    let result = expand_macros_with_context(program, &context);
    std::fs::remove_dir_all(directory).unwrap();
    let expanded = result.unwrap();
    struct Literals(Vec<crate::lexer::Span>);
    impl<'a> crate::ast::visit::Visitor<'a> for Literals {
        fn visit_literal(&mut self, literal: &'a crate::ast::Literal) { self.0.push(literal.span.clone()); }
    }
    let mut literals = Literals(Vec::new());
    crate::ast::visit::walk_module(&mut literals, &expanded.module);
    assert_eq!(literals.0.len(), 1);
    let map = context.source_map();
    let span = &literals.0[0];
    let start = source.find("\\\"").unwrap();
    assert_eq!(map.original_span(span).start..map.original_span(span).end, start..start + 8);
    let escape = crate::lexer::Span::new(span.file_path.clone(), span.start + 3, span.start + 4);
    let mapped = map.original_span(&escape);
    let newline = source.find("\\n").unwrap();
    assert_eq!(mapped.start..mapped.end, newline..newline + 2);
    let reports = map.map_diagnostics(crate::diagnostic::Diagnostics(vec![crate::diagnostic::Diagnostic::new("escape".into(), escape)]));
    assert_eq!(reports.0[0].primary.as_ref().unwrap().span, mapped);
}

#[test]
fn macro_b_19_fragments_reject_bad_authority_ranges_rock_and_excess_work() {
    for case in 0..7 {
        let config = crate::Config::default();
        let program = crate::parser::parse_string("main = -> %provider::hole \"é\"\n", &config).unwrap();
        let invocation = crate::macro_expansion::tests::first_invocation(&program);
        let mut construction = ConstructionContext { invocation: invocation.id, kind: ConstructionKind::CallSite };
        let part = match case {
            0 => ProcMacroPart::Literal { view: 99, range: 0..2 },
            1 => ProcMacroPart::Literal { view: 0, range: 1..2 },
            2 => { construction.invocation = crate::ast::InvocationId::fresh().unwrap(); ProcMacroPart::Source { text: "42".into(), context: construction } }
            3 => { construction.kind = ConstructionKind::Dependency(99); ProcMacroPart::Source { text: "42".into(), context: construction } }
            4 => ProcMacroPart::Source { text: "🦀".into(), context: construction },
            _ => ProcMacroPart::Source { text: "42".into(), context: construction },
        };
        let response = ProcMacroResponse { protocol_version: PROC_MACRO_PROTOCOL_VERSION, diagnostics: Vec::new(),
            output: Some(ProcMacroOutput { parts: vec![part] }),
        };
        let (artifact, directory) = crate::macro_expansion::tests::proc_macro_artifact_with_response("hole", response);
        let mut limits = crate::macro_expansion::ExpansionLimits::default();
        if case == 5 { limits.max_matching_work = 0; }
        if case == 6 { limits.max_output_bytes = 1; }
        let context = MacroExpansionContext::new(&config).with_named_proc_macro_artifact("provider".into(), artifact).with_limits(limits);
        let result = expand_macros_with_context(program, &context);
        std::fs::remove_dir_all(directory).unwrap();
        let errors = result.unwrap_err();
        assert!(!format!("{errors:?}").contains("not implemented"), "case {case}: {errors:?}");
        let expected = match case {
            0 => Some("invalid literal view handle"),
            1 => Some("invalid literal fragment range"),
            2 => Some("belongs to another invocation"),
            3 => Some("undeclared definition-side dependency handle"),
            5 => Some("matching work budget exceeded"),
            6 => Some("output byte budget exceeded"),
            _ => None,
        };
        if let Some(expected) = expected {
            assert!(errors.0.iter().any(|error| error.message.contains(expected)), "case {case}: {errors:?}");
        }
    }
}

#[test]
fn macro_b_19_source_fragment_boundaries_never_glue_identifiers() {
    let config = crate::Config::default();
    let program = crate::parser::parse_string("%provider::make!\n", &config).unwrap();
    let invocation = crate::macro_expansion::tests::first_invocation(&program);
    let construction = ConstructionContext { invocation: invocation.id, kind: ConstructionKind::CallSite };
    let response = ProcMacroResponse { protocol_version: PROC_MACRO_PROTOCOL_VERSION, diagnostics: Vec::new(),
        output: Some(ProcMacroOutput { parts: ["ma", "in = -> 42\n"].into_iter().map(|text|
            ProcMacroPart::Source { text: text.into(), context: construction }).collect() }),
    };
    let (artifact, directory) = crate::macro_expansion::tests::proc_macro_artifact_with_response("make", response);
    let context = MacroExpansionContext::new(&config).with_named_proc_macro_artifact("provider".into(), artifact);
    let result = expand_macros_with_context(program, &context);
    std::fs::remove_dir_all(directory).unwrap();
    if let Ok(program) = result {
        assert!(!program.module.top_levels.iter().any(|item| matches!(item,
            crate::ast::TopLevel::FunctionDecl(function) if function.name.name == "main")));
    }
}

#[test]
fn macro_b_19_copied_expression_captures_retain_snapshot_context_and_opacity() {
    let config = crate::Config::default();
    let program = crate::parser::parse_string("%provider::make (left + right)\n", &config).unwrap();
    let mut invocation = crate::macro_expansion::tests::first_invocation(&program);
    let limits = super::super::token_tree::MACRO_LIMITS;
    invocation.args = invocation.args.map_contexts(|_| Ok(crate::lexer::lossless::SyntaxContext::allocated(57))).unwrap()
        .expression_fragment(0, limits).unwrap();
    let request = ProcMacroRequest {
        protocol_version: PROC_MACRO_PROTOCOL_VERSION, compiler_version: env!("CARGO_PKG_VERSION").into(),
        macro_name: "make".into(), macro_identity: "test_macros::make".into(), source_module: None,
        expansion_id: Some(1), invocation_id: invocation.id, parent: None, output_category: invocation.output,
        kind: ProcMacroKind::FunctionLike, header_indent: 0, block_base: None,
        input: encode_stream(&invocation.args).unwrap(), definition_dependencies: Vec::new(), views: Default::default(),
    };
    let response = ProcMacroResponse { protocol_version: PROC_MACRO_PROTOCOL_VERSION, diagnostics: Vec::new(),
        output: Some(ProcMacroOutput { parts: vec![
            ProcMacroPart::Stream(request.input.clone()),
            ProcMacroPart::Source { text: " ".into(), context: request.construction_context(ConstructionKind::Fresh(0)) },
        ] }),
    };
    let context = MacroExpansionContext::new(&config);
    let stream = super::fragments::materialize(&request, &response, &invocation, &context, ExpansionId(1), GeneratedSourceId(1)).unwrap();
    assert_eq!(stream.expression_at(0, limits).unwrap().unwrap().0, invocation.args.len_bytes());
    assert_eq!(stream.runs()[0].context(), invocation.args.runs()[0].context());
    assert_eq!(stream.runs()[0].snapshot_identity(), invocation.args.runs()[0].snapshot_identity());
    assert_ne!(stream.runs().last().unwrap().context(), stream.runs()[0].context());
}

#[test]
fn macro_b_19_literal_subspans_survive_nested_declarative_forwarding() {
    let config = crate::Config::default();
    let source = "macro pass\n    $value:expr =>\n        $value\nmain = -> %provider::hole \"é\\nmissing\"\n";
    let program = crate::parser::parse_string(source, &config).unwrap();
    let invocation = crate::macro_expansion::tests::first_invocation(&program);
    let response = ProcMacroResponse { protocol_version: PROC_MACRO_PROTOCOL_VERSION, diagnostics: Vec::new(),
        output: Some(ProcMacroOutput { parts: vec![
            ProcMacroPart::Source { text: "%pass ".into(), context: ConstructionContext { invocation: invocation.id, kind: ConstructionKind::CallSite } },
            ProcMacroPart::Literal { view: 0, range: 3..10 },
        ] }),
    };
    let (artifact, directory) = crate::macro_expansion::tests::proc_macro_artifact_with_response("hole", response);
    let context = MacroExpansionContext::new(&config).with_named_proc_macro_artifact("provider".into(), artifact);
    let result = expand_macros_with_context(program, &context);
    std::fs::remove_dir_all(directory).unwrap();
    let expanded = result.unwrap();
    struct Names(Vec<crate::lexer::Span>);
    impl<'a> crate::ast::visit::Visitor<'a> for Names {
        fn visit_ident(&mut self, ident: &'a crate::ast::Ident) { if ident.name == "missing" { self.0.push(ident.span.clone()); } }
    }
    let mut names = Names(Vec::new());
    crate::ast::visit::walk_module(&mut names, &expanded.module);
    assert_eq!(names.0.len(), 1);
    let mapped = context.source_map().original_span(&names.0[0]);
    let start = source.find("missing").unwrap();
    assert_eq!(mapped.start..mapped.end, start..start + 7);
}
