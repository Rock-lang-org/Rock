use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use rock_lib::ast::MacroOutputCategory;
use rock_lib::macro_expansion::proc_macro::*;
use rock_lib::macro_expansion::TokenStream;
use rock_lib::Config;

fn stream(text: &str) -> Vec<u8> {
    encode_stream(&TokenStream::from_bytes("/worker-shape.rk".into(), text.as_bytes().to_vec(),
        rock_lib::macro_expansion::token_tree::MACRO_LIMITS).unwrap()).unwrap()
}

fn pid(stderr: &[u8], seen: &mut HashSet<u32>) {
    let text = std::str::from_utf8(stderr).unwrap();
    assert_eq!(text.lines().count(), 1, "entry executes exactly once: {text:?}");
    let pid = text.trim().parse::<u32>().unwrap();
    assert!(pid > 0 && seen.insert(pid), "one fresh process per invocation: {text:?}");
}

pub fn verify(path: &Path, golden: &ProcMacroRequest, rich: &ProcMacroRequest) {
    let mut seen = HashSet::new();
    let mut block = golden.clone();
    block.header_indent = 0; block.block_base = Some(4); block.output_category = MacroOutputCategory::Items;
    block.input = stream("    雪 { \" '\0 @ | ü\r\n"); block.views = Default::default();
    let mut empty = golden.clone(); empty.input = stream(""); empty.views = Default::default();
    for request in [golden, rich, &block, &empty, golden, golden] {
        let output = super::run_client_with_input(path, &encode_request(request).unwrap());
        assert!(output.status.success(), "identity {}: {output:?}", request.macro_name);
        pid(&output.stderr, &mut seen);
        let response = decode_response(&output.stdout).unwrap();
        response.validate_for(request).unwrap();
        assert_eq!(response, ProcMacroResponse::expanded(request.input.clone()));
    }
    let mut request = golden.clone(); request.macro_name = "context_copy".into();
    let output = super::run_client_with_input(path, &encode_request(&request).unwrap());
    assert!(output.status.success(), "author mutations changed retained request: {output:?}");
    pid(&output.stderr, &mut seen);
    assert_eq!(output.stdout, [7, 1, 0, 0]);
    decode_response(&output.stdout).unwrap().validate_for(&request).unwrap();

    request.macro_name = "reject".into();
    let output = super::run_client_with_input(path, &encode_request(&request).unwrap());
    assert!(output.status.success(), "structured entry failure is a valid final response: {output:?}");
    pid(&output.stderr, &mut seen);
    let response = decode_response(&output.stdout).unwrap(); response.validate_for(&request).unwrap();
    assert!(response.output.is_none()); assert_eq!(response.diagnostics[0].message, "entry rejected");
    request.macro_name = "invalid_result".into();
    let output = super::run_client_with_input(path, &encode_request(&request).unwrap());
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
    let error = b"macro worker: invalid result\n";
    assert!(output.stderr.ends_with(error), "{output:?}");
    pid(&output.stderr[..output.stderr.len() - error.len()], &mut seen);

    let bytes = encode_request(golden).unwrap();
    let mut twice = bytes.clone(); twice.extend_from_slice(&bytes);
    let mut wrong_version = bytes.clone(); wrong_version[0] = 6;
    for (bytes, message) in [
        (Vec::new(), "macro worker: invalid request\n"),
        (bytes[..bytes.len() / 2].to_vec(), "macro worker: invalid request\n"),
        (twice, "macro worker: invalid request\n"),
        (wrong_version, "macro worker: invalid request\n"),
        (vec![0; 1048577], "macro worker: request IO failed\n"),
    ] {
        let output = super::run_client_with_input(path, &bytes);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "malformed request emitted protocol data: {output:?}");
        assert_eq!(output.stderr, message.as_bytes(), "entry must not execute");
    }

    let artifact = ProcMacroArtifact {
        artifact_format_version: rock_lib::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "worker".into(), protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        host_triple: "x86_64-unknown-linux-gnu".into(), executable: path.into(),
        compiler_version: env!("CARGO_PKG_VERSION").into(), executable_identity: ProcMacroFileIdentity::read(path).unwrap(), required_files: Vec::new(),
        capabilities: vec![ProcMacroCapability::Stdio], definition_dependencies: Vec::new(),
        exports: vec![ProcMacroExport { name: "identity".into(), identity: "worker::identity".into(),
            kind: ProcMacroKind::FunctionLike, input_shape: ProcMacroInputShape::TokenStream }],
    };
    let mut request = golden.clone(); request.macro_name = "identity".into(); request.macro_identity = "worker::identity".into();
    request.definition_dependencies.clear();
    let response = run_proc_macro_process(&artifact, &request, Duration::from_secs(10), false, None).unwrap();
    assert_eq!(response, ProcMacroResponse::expanded(request.input.clone()));

    let config = Config { no_std: true, no_prelude: true, ..Default::default() };
    use rock_lib::ast::{Expression, Operand, Statement, TopLevel, UnaryExpr};
    // Exercise the exact invocation data produced by the parser through the
    // native compiler host, without adding future procedural-expression dispatch
    // or pretending preserved block margins are already normalized Rock output.
    for source in ["main = -> %worker::identity 42\n", "%worker::identity\n    main = -> 7\n"] {
        let program = rock_lib::parser::parse_string(source, &config).unwrap();
        let invocation = match &program.module.top_levels[0] {
            TopLevel::MacroInvoc(invocation) => invocation,
            TopLevel::FunctionDecl(function) => {
                let Statement::Expression(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary))) = &function.lambda.body.statements[0]
                    else { panic!("expected macro expression") };
                let Operand::MacroInvoc(invocation) = &primary.operand else { panic!("expected invocation") }; invocation
            }
            _ => panic!("expected parsed invocation"),
        };
        let mut parsed = request.clone();
        parsed.source_module = None; parsed.invocation_id = invocation.id; parsed.parent = invocation.parent;
        parsed.output_category = invocation.output; parsed.header_indent = invocation.header_indent;
        parsed.block_base = invocation.block.as_ref().map(|block| block.body_indent);
        parsed.input = encode_stream(&invocation.args).unwrap();
        parsed.views = ProcMacroViews { arguments: invocation.block.is_none().then(|| ProcMacroArguments {
            ranges: invocation.arguments.clone(),
        }), ..Default::default() };
        let response = run_proc_macro_process(&artifact, &parsed, Duration::from_secs(10), false, None).unwrap();
        response.validate_for(&parsed).unwrap();
        assert_eq!(response, ProcMacroResponse::expanded(parsed.input));
    }
    eprintln!("SDK acceptance: 14 one-shot controls and 3 compiler-host ABI controls passed");
}
