use rock_lib::macro_expansion::ConstructionKind;
use rock_lib::macro_expansion::proc_macro::*;

pub fn cases(golden: &ProcMacroRequest, rich: &ProcMacroRequest) -> Vec<(ProcMacroRequest, bool)> {
    let mut cases = Vec::new();
    for name in ["format", "error", "report_failure", "expand", "empty", "expand_empty", "severities", "constructed", "compact_windows", "wide_message",
        "missing_error", "warning_failure", "output_without_output", "input_cut", "invalid_label", "invalid_literal_handle",
        "invalid_literal_cut", "wrong_invocation", "wrong_dependency", "message_limit", "parts_limit", "spelling_limit", "invalid_stream", "forged_context"] {
        let mut request = golden.clone(); request.macro_name = name.into();
        let accepted = matches!(name, "format" | "error" | "report_failure" | "expand" | "empty" | "expand_empty" | "severities"
            | "constructed" | "compact_windows" | "wide_message" | "forged_context");
        cases.push((request, accepted));
    }
    cases.push((rich.clone(), true));
    let mut wide = rich.clone(); wide.macro_name = "wide_context".into(); cases.push((wide, true));
    for (_, bytes, accepted) in super::macro_requests::cases() {
        if !accepted { continue; }
        let mut request = decode_request(&bytes).unwrap();
        if matches!(request.macro_name.as_str(), "window" | "disjoint") {
            request.macro_name = "expand".into();
            cases.push((request, true));
        }
    }
    cases
}

fn unhex(hex: &str) -> Vec<u8> {
    let hex = hex.trim();
    (0..hex.len()).step_by(2).map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap()).collect()
}

pub fn verify(request: &ProcMacroRequest, bytes: &[u8]) {
    let response = decode_response(bytes).unwrap();
    assert_eq!(encode_response(&response).unwrap(), bytes, "canonical encoding");
    if request.macro_name == "forged_context" {
        assert!(response.validate_for(request).is_err(), "only the compiler can authorize forwarded handles");
        return;
    }
    response.validate_for(request).unwrap();
    match request.macro_name.as_str() {
        "format" => assert_eq!(bytes, unhex(include_str!("../fixtures/macros/protocol-9/response.hex"))),
        "error" | "report_failure" => assert_eq!(bytes, unhex(include_str!("../fixtures/macros/protocol-9/error.hex"))),
        "expand" | "rich" => assert_eq!(response, ProcMacroResponse::expanded(request.input.clone())),
        "empty" => {
            assert_eq!(bytes, [9, 1, 0, 0]);
            assert!(response.output.unwrap().parts.is_empty());
        }
        "expand_empty" => {
            let output = response.output.unwrap();
            assert_eq!(output.parts.len(), 1);
            let ProcMacroPart::Stream(bytes) = &output.parts[0] else { panic!("expected stream") };
            assert_eq!(decode_stream(bytes).unwrap().len_bytes(), 0);
        }
        "severities" => {
            use rock_lib::diagnostic::DiagnosticSeverity::{Error, Hint, Info, Warning};
            assert_eq!(response.diagnostics.iter().map(|report| report.severity).collect::<Vec<_>>(), [Error, Warning, Info, Hint]);
            assert_eq!(response.diagnostics[2].message, "é\0雪");
            assert_eq!(response.diagnostics[2].primary, ProcMacroLocation::Input(2..4));
            assert_eq!(response.diagnostics[3].primary, ProcMacroLocation::Output(0..0));
        }
        "constructed" => {
            let parts = response.output.unwrap().parts;
            assert_eq!(parts.len(), 3);
            for ((part, text), kind) in parts.iter().zip(["fresh", "caller", "helper::helper"])
                .zip([ConstructionKind::Fresh(0), ConstructionKind::CallSite, ConstructionKind::Dependency(0)]) {
                let ProcMacroPart::Stream(bytes) = part else { panic!("expected constructed stream") };
                let stream = decode_stream(bytes).unwrap();
                assert_eq!(stream.runs().iter().map(|run| run.text()).collect::<String>(), text);
                super::macro_requests::assert_construction(bytes, request.construction_context(kind));
            }
        }
        "compact_windows" => {
            assert!(bytes.len() < 512, "unused snapshot gap retained in SDK output");
            let output = response.output.unwrap();
            let ProcMacroPart::Stream(bytes) = &output.parts[0] else { panic!("expected stream") };
            let stream = decode_stream(bytes).unwrap();
            assert_eq!(stream.runs().iter().map(|run| run.text()).collect::<String>(), "AAA");
            assert_eq!(stream.runs().iter().map(|run| run.range()).collect::<Vec<_>>(), [1_099_999..1_100_000, 0..1, 0..1]);
            assert_eq!(encode_stream(&stream).unwrap(), *bytes);
        }
        "wide_context" => assert_eq!(response.output.unwrap().parts, [ProcMacroPart::Source {
            text: "x".into(), context: request.construction_context(ConstructionKind::Fresh(u32::MAX)),
        }]),
        "wide_message" => assert_eq!(response.diagnostics[0].message, "A".repeat(65536)),
        name => panic!("unverified result case {name}"),
    }
}
