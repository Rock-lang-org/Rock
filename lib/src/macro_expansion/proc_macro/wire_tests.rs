use super::*;
use crate::diagnostic::DiagnosticSeverity;
use crate::lexer::lossless::{golden_source, ConstructionKind};
use crate::products::ProductCrateIdentity;

fn request() -> ProcMacroRequest {
    ProcMacroRequest {
        protocol_version: PROC_MACRO_PROTOCOL_VERSION, compiler_version: env!("CARGO_PKG_VERSION").into(),
        macro_name: "format".into(), macro_identity: "macros::format".into(), source_module: Some("app".into()),
        expansion_id: Some(2), invocation_id: bincode::DefaultOptions::new().deserialize(&[1]).unwrap(),
        parent: Some(ExpansionId(1)), output_category: MacroOutputCategory::Expression, kind: ProcMacroKind::FunctionLike,
        header_indent: 0, block_base: None,
        input: encode_stream(&golden_source("(\"é\\n\", value)").expression_fragment(0, super::super::token_tree::MACRO_LIMITS).unwrap()).unwrap(),
        definition_dependencies: vec![ProductCrateIdentity::local("helper".into())],
        views: ProcMacroViews {
            arguments: Some(ProcMacroArguments { ranges: vec![1..7, 9..14] }),
            groups: vec![ProcMacroGroup { delimiter: crate::lexer::groups::Delimiter::Paren, range: 0..15 }],
            literals: vec![ProcMacroLiteral { source: 1..7, decoded: "é\n".into(), mapping: vec![
                LiteralMapping { decoded: 0..2, source: 2..4 }, LiteralMapping { decoded: 2..3, source: 4..6 },
            ] }],
        },
    }
}

fn response(request: &ProcMacroRequest) -> ProcMacroResponse {
    ProcMacroResponse { protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        output: Some(ProcMacroOutput { parts: vec![
            ProcMacroPart::Stream(request.input.clone()),
            ProcMacroPart::Source { text: "()".into(), context: request.construction_context(ConstructionKind::Fresh(3)) },
            ProcMacroPart::Source { text: "dependency".into(), context: request.construction_context(ConstructionKind::Dependency(0)) },
            ProcMacroPart::Source { text: "caller".into(), context: request.construction_context(ConstructionKind::CallSite) },
            ProcMacroPart::Literal { view: 0, range: 0..2 },
        ] }),
        diagnostics: vec![ProcMacroDiagnostic { severity: DiagnosticSeverity::Warning, message: "notice".into(),
            primary: ProcMacroLocation::Input(9..14), labels: vec![ProcMacroLabel { message: "emitted here".into(), location: ProcMacroLocation::Output(15..17) }],
            notes: vec!["note".into()], help: vec!["help".into()],
        }],
    }
}

fn hex(bytes: &[u8]) -> String { bytes.iter().map(|byte| format!("{byte:02x}")).collect() }

#[test]
fn macro_b_01_enriched_schema_roundtrips_with_exact_utf8_and_all_handles() {
    let request = request();
    let bytes = encode_request(&request).unwrap();
    let decoded = decode_request(&bytes).unwrap();
    assert_eq!(decoded, request);
    assert_eq!(encode_request(&decoded).unwrap(), bytes);
    let response = response(&request);
    response.validate_for(&request).unwrap();
    let bytes = encode_response(&response).unwrap();
    let decoded = decode_response(&bytes).unwrap();
    assert_eq!(decoded, response);
    assert_eq!(encode_response(&decoded).unwrap(), bytes);
    assert_eq!(hex(&encode_request(&request).unwrap()), include_str!("../../../tests/fixtures/macros/protocol-7/request.hex").trim());
    assert_eq!(hex(&bytes), include_str!("../../../tests/fixtures/macros/protocol-7/response.hex").trim());
    assert_eq!(hex(&encode_response(&ProcMacroResponse::failure(vec!["boom".into()])).unwrap()),
        include_str!("../../../tests/fixtures/macros/protocol-7/error.hex").trim());
}

#[test]
fn macro_b_01_rejects_every_truncation_trailing_data_versions_and_unknown_tags() {
    let request = request();
    let bytes = encode_request(&request).unwrap();
    for length in 0..bytes.len() { assert!(decode_request(&bytes[..length]).is_err(), "request prefix {length}"); }
    let mut trailing = bytes; trailing.push(0); assert!(decode_request(&trailing).is_err());
    let bytes = encode_response(&response(&request)).unwrap();
    for length in 0..bytes.len() { assert!(decode_response(&bytes[..length]).is_err(), "response prefix {length}"); }
    let mut trailing = bytes; trailing.push(0); assert!(decode_response(&trailing).is_err());
    assert!(decode_request(&[PROC_MACRO_PROTOCOL_VERSION as u8 - 1]).unwrap_err().to_string().contains("version"));
    assert!(decode_response(&[PROC_MACRO_PROTOCOL_VERSION as u8 - 1]).unwrap_err().to_string().contains("version"));
    // Protocol, Some(output), one part, invalid enum discriminant.
    assert!(decode_response(&[7, 1, 1, 99]).is_err());
    let mut raw = encode_request(&request).unwrap();
    let index = raw.windows(2).position(|bytes| bytes == [0xc3, 0xa9]).unwrap();
    raw[index] = 0xff; assert!(decode_request(&raw).is_err());
    assert!(decode_request(&vec![0; MAX_PROC_MACRO_MESSAGE_BYTES as usize + 1]).is_err());
    assert!(decode_response(&vec![0; MAX_PROC_MACRO_MESSAGE_BYTES as usize + 1]).is_err());
}

#[test]
fn macro_b_01_views_reject_invalid_utf8_ranges_nesting_and_mapping_coverage() {
    let valid = request();
    let rejects = |request: &ProcMacroRequest| {
        let bytes = bincode::DefaultOptions::new().serialize(request).unwrap();
        assert!(decode_request(&bytes).is_err(), "accepted {request:?}");
    };
    let mut bad = valid.clone(); bad.views.arguments.as_mut().unwrap().ranges[0] = 3..7; rejects(&bad);
    let mut bad = valid.clone(); bad.views.arguments.as_mut().unwrap().ranges.reverse(); rejects(&bad);
    let mut bad = valid.clone(); bad.views.groups[0].range.end = 14; rejects(&bad);
    let mut bad = valid.clone(); bad.views.groups = vec![bad.views.groups[0].clone(); 129]; rejects(&bad);
    let mut bad = valid.clone(); bad.views.literals[0].mapping[0].decoded.end = 1; rejects(&bad);
    let mut bad = valid.clone(); bad.views.literals[0].mapping[0].source.end = 3; rejects(&bad);
    let mut bad = valid.clone(); bad.views.literals[0].mapping[1].decoded.start = 1; rejects(&bad);
    let mut bad = valid.clone(); bad.views.literals[0].mapping.pop(); rejects(&bad);
    let mut bad = valid.clone(); bad.views.literals[0].mapping[0].source = 0..1; rejects(&bad);
    let mut bad = valid.clone(); bad.header_indent = 4; bad.block_base = Some(4); rejects(&bad);
    let mut bad = valid.clone(); bad.input = vec![0]; rejects(&bad);
    let mut bad = valid.clone(); bad.compiler_version = "unknown".into(); rejects(&bad);
    let mut bad = valid; bad.invocation_id = bincode::DefaultOptions::new().deserialize(&[0]).unwrap(); rejects(&bad);
}

#[test]
fn macro_b_01_response_rejects_unowned_handles_ranges_and_empty_failure() {
    let request = request();
    let valid = response(&request);
    let mut bad = valid.clone(); bad.output.as_mut().unwrap().parts.push(ProcMacroPart::Literal { view: 1, range: 0..1 });
    assert!(bad.validate_for(&request).is_err());
    let mut bad = valid.clone(); bad.output.as_mut().unwrap().parts.push(ProcMacroPart::Literal { view: 0, range: 0..1 });
    assert!(bad.validate_for(&request).is_err());
    let mut bad = valid.clone(); bad.output.as_mut().unwrap().parts.push(ProcMacroPart::Source { text: "x".into(), context: request.construction_context(ConstructionKind::Dependency(1)) });
    assert!(bad.validate_for(&request).is_err());
    let mut bad = valid.clone(); bad.output.as_mut().unwrap().parts.push(ProcMacroPart::Source { text: "x".into(), context: crate::lexer::lossless::ConstructionContext {
        invocation: bincode::DefaultOptions::new().deserialize(&[2]).unwrap(), kind: ConstructionKind::Fresh(0) } });
    assert!(bad.validate_for(&request).is_err());
    let mut bad = valid.clone(); bad.diagnostics[0].primary = ProcMacroLocation::Input(3..4);
    assert!(bad.validate_for(&request).is_err());
    let mut bad = valid.clone(); bad.diagnostics[0].primary = ProcMacroLocation::Output(0..1000);
    assert!(bad.validate_for(&request).is_err());
    let mut bad = valid.clone(); bad.output = None; bad.diagnostics[0].severity = DiagnosticSeverity::Error;
    assert!(bad.validate_for(&request).is_err());
    let mut bad = valid; bad.output = None; bad.diagnostics.clear();
    assert!(decode_response(&bincode::DefaultOptions::new().serialize(&bad).unwrap()).is_err());
}

#[test]
fn macro_b_01_unknown_source_characters_and_unmatched_dsl_quotes_are_data() {
    let mut request = request();
    request.input = encode_stream(&golden_source("雪 { \" '\0 @ | ü\r\n")).unwrap();
    request.block_base = Some(4); request.views = Default::default();
    let decoded = decode_request(&encode_request(&request).unwrap()).unwrap();
    let stream = decode_stream(&decoded.input).unwrap();
    assert_eq!(stream.runs().iter().map(|run| run.text()).collect::<String>(), "雪 { \" '\0 @ | ü\r\n");
}

#[test]
fn macro_b_01_declared_sequence_limits_reject_before_element_allocation() {
    // protocol, Some(output), over-limit parts count; no elements are provided.
    let mut bytes = vec![7, 1];
    bytes.extend(bincode::DefaultOptions::new().serialize(&(MAX_METADATA_ENTRIES as u64 + 1)).unwrap());
    assert!(decode_response(&bytes).unwrap_err().to_string().contains("sequence limit"));
    // protocol, Some(output), one Stream part with a forged byte-vector length.
    let mut bytes = vec![7, 1, 1, 0];
    bytes.extend(bincode::DefaultOptions::new().serialize(&u64::MAX).unwrap());
    assert!(decode_response(&bytes).unwrap_err().to_string().contains("sequence limit"));
}

#[test]
fn macro_b_01_structured_diagnostics_map_input_and_preserve_notes_severity() {
    let request = request();
    let response = response(&request);
    let output = golden_source("012345678901234567");
    let reports = response.diagnostics_at(&request, crate::lexer::Span::new("/invoke.rk".into(), 0, 1), Some(&output));
    assert_eq!(reports.0[0].severity, DiagnosticSeverity::Warning);
    assert_eq!(reports.0[0].primary.as_ref().unwrap().span.start, 9);
    assert_eq!(reports.0[0].secondary[0].span.start, 15);
    assert_eq!(reports.0[0].notes, vec!["note"]);
    assert_eq!(reports.0[0].help, vec!["help"]);
}

#[test]
fn macro_b_01_native_worker_success_retains_warning_reports() {
    let config = crate::Config::default();
    let program = crate::parser::parse_string("%provider::notice!", &config).unwrap();
    let invocation = crate::macro_expansion::tests::first_invocation(&program);
    let mut response = crate::macro_expansion::tests::constructed_response(&invocation, "main = -> 0");
    response.diagnostics.push(ProcMacroDiagnostic { severity: DiagnosticSeverity::Warning, message: "warning from worker".into(),
        primary: ProcMacroLocation::Invocation, labels: Vec::new(), notes: vec!["retained note".into()], help: Vec::new() });
    let (artifact, directory) = crate::macro_expansion::tests::proc_macro_artifact_with_response("notice", response);
    let context = crate::macro_expansion::MacroExpansionContext::new(&config).with_named_proc_macro_artifact("provider".into(), artifact);
    crate::macro_expansion::expand_macros_with_context(program, &context).unwrap();
    let reports = context.reports.borrow();
    assert_eq!(reports.0.len(), 1);
    assert_eq!(reports.0[0].severity, DiagnosticSeverity::Warning);
    assert_eq!(reports.0[0].notes, vec!["retained note"]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn macro_b_01_output_parts_do_not_glue_contiguous_forwarded_spelling() {
    let config = crate::Config::default();
    let program = crate::parser::parse_string("%provider::make main", &config).unwrap();
    let invocation = crate::macro_expansion::tests::first_invocation(&program);
    let text: String = invocation.args.runs().iter().map(|run| run.text()).collect();
    let offset = text.find("main").unwrap();
    let limits = super::super::token_tree::MACRO_LIMITS;
    let suffix = TokenStream::source_fragment(" = -> 0\n".into(), crate::lexer::lossless::ConstructionContext {
        invocation: invocation.id, kind: ConstructionKind::CallSite,
    }, limits).unwrap();
    let response = ProcMacroResponse { protocol_version: PROC_MACRO_PROTOCOL_VERSION, diagnostics: Vec::new(),
        output: Some(ProcMacroOutput { parts: vec![
            ProcMacroPart::Stream(encode_stream(&invocation.args.slice(offset..offset + 2, limits).unwrap()).unwrap()),
            ProcMacroPart::Stream(encode_stream(&invocation.args.slice(offset + 2..offset + 4, limits).unwrap()).unwrap()),
            ProcMacroPart::Stream(encode_stream(&suffix).unwrap()),
        ] }),
    };
    let (artifact, directory) = crate::macro_expansion::tests::proc_macro_artifact_with_response("make", response);
    let context = crate::macro_expansion::MacroExpansionContext::new(&config).with_named_proc_macro_artifact("provider".into(), artifact);
    let expanded = crate::macro_expansion::expand_macros_with_context(program, &context);
    // Parsed as distinct `ma in`, never silently joined into a `main` binding.
    if let Ok(program) = expanded {
        assert!(!program.module.top_levels.iter().any(|item| matches!(item,
            crate::ast::TopLevel::FunctionDecl(function) if function.name.name == "main")));
    }
    std::fs::remove_dir_all(directory).unwrap();
}
