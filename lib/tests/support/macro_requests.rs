use std::ops::Range;

use bincode::Options;
use serde::Serialize;

use rock_lib::ast::{InvocationId, MacroOutputCategory};
use rock_lib::Span;
use rock_lib::macro_expansion::{ConstructionContext, ConstructionKind, ExpansionId, GeneratedSourceId, OutputProjection, TokenOrigin, TokenProvenance};
use rock_lib::macro_expansion::proc_macro::*;
use rock_lib::products::ProductCrateIdentity;

// Test-owned wire mirrors let malformed transport reach both real decoders;
// production code still has one authoritative shared lossless representation.
#[derive(Clone, Serialize)]
struct Snapshot { identity: u64, path: String, text: String }
#[derive(Clone, Serialize)]
struct Run {
    source: usize, range: Range<usize>, kind: u32, context: u32, boundary: u32,
    origin: Option<TokenOrigin>, construction: Option<ConstructionContext>,
}
#[derive(Clone, Serialize)]
struct Anchor { source: usize, offset: usize, context: u32, origin: Option<TokenOrigin> }
#[derive(Clone, Serialize)]
struct Stream {
    magic: [u8; 8], sources: Vec<Snapshot>, runs: Vec<Run>, anchor: Anchor,
    expressions: Vec<(Range<usize>, usize)>,
}

fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    bincode::DefaultOptions::new().with_limit(1048576).serialize(value).unwrap()
}

fn invocation(value: u64) -> InvocationId {
    bincode::DefaultOptions::new().deserialize(&encode(&value)).unwrap()
}

fn rich_stream() -> Stream {
    let span = Span::new("/origin.rk".into(), 0, 9);
    let origins = [
        Some(TokenProvenance::Source(span.clone()).into()),
        Some(TokenOrigin {
            provenance: TokenProvenance::Captured { expansion: ExpansionId(65536), invocation_span: span.clone(), capture_span: span.clone() },
            projection: Some(OutputProjection { namespace: u64::MAX, expansion: ExpansionId(u32::MAX),
                generated_source: GeneratedSourceId(251), source_start: 0, output_start: 65536 }),
        }),
        Some(TokenProvenance::Generated { expansion: ExpansionId(251), generated_source: GeneratedSourceId(65536), definition_span: span }.into()),
        None, None, None,
    ];
    Stream {
        magic: *b"RKLOSS04", sources: vec![Snapshot { identity: u64::MAX, path: "/rich.rk".into(), text: "a b\t雪\r\n".into() }],
        runs: [(0..1, 0, 251), (1..2, 1, 65536), (2..3, 0, u32::MAX), (3..4, 1, 0), (4..7, 0, 0), (7..9, 2, 0)]
            .into_iter().zip(origins).enumerate().map(|(index, ((range, kind, context), origin))| Run {
                source: 0, range, kind, context, boundary: u32::from(index != 0), origin, construction: None,
            }).collect(),
        anchor: Anchor { source: 0, offset: 0, context: 0, origin: None }, expressions: Vec::new(),
    }
}

pub fn cases() -> Vec<(String, Vec<u8>, bool)> {
    let hex = include_str!("../fixtures/macros/protocol-7/request.hex").trim();
    let bytes: Vec<_> = (0..hex.len()).step_by(2).map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap()).collect();
    let golden = decode_request(&bytes).unwrap();
    let mut cases = vec![("protocol-7 golden".into(), bytes.clone(), true)];
    for length in 0..bytes.len() {
        cases.push((format!("golden prefix {length}"), bytes[..length].to_vec(), false));
    }
    let mut trailing = bytes.clone(); trailing.push(0);
    cases.push(("trailing data".into(), trailing, false));
    let mut wide_version = vec![253]; wide_version.extend_from_slice(&7u64.to_le_bytes()); wide_version.extend_from_slice(&bytes[1..]);
    cases.push(("nonminimal unsigned encoding".into(), wide_version, true));
    let mut overflow_version = vec![253]; overflow_version.extend_from_slice(&(u32::MAX as u64 + 1).to_le_bytes());
    overflow_version.extend_from_slice(&bytes[1..]);
    cases.push(("u32 overflow".into(), overflow_version, false));
    for malformed in [vec![6], vec![254], vec![255], vec![7, 253, 255, 255, 255, 255, 255, 255, 255, 255]] {
        cases.push(("invalid version/tag/length".into(), malformed, false));
    }
    let mut utf8 = bytes.clone();
    let offset = utf8.windows(2).position(|window| window == [0xc3, 0xa9]).unwrap();
    utf8[offset] = 255;
    cases.push(("invalid UTF-8".into(), utf8, false));
    cases.push(("oversized envelope".into(), vec![0; 1048577], false));
    let module_offset = encode(&(golden.protocol_version, &golden.compiler_version, &golden.macro_name, &golden.macro_identity)).len();
    let category_offset = encode(&(golden.protocol_version, &golden.compiler_version, &golden.macro_name,
        &golden.macro_identity, &golden.source_module, golden.expansion_id, golden.invocation_id, golden.parent)).len();
    for (name, offset, value) in [("invalid option tag", module_offset, 2), ("invalid category tag", category_offset, 2),
        ("invalid macro kind tag", category_offset + 1, 3)] {
        let mut malformed = bytes.clone(); malformed[offset] = value;
        cases.push((name.into(), malformed, false));
    }

    let stream = rich_stream();
    let mut rich = golden.clone();
    rich.macro_name = "rich".into(); rich.macro_identity = "macros::rich".into();
    rich.source_module = None; rich.expansion_id = Some(u32::MAX); rich.invocation_id = invocation(u64::MAX);
    rich.parent = Some(ExpansionId(251)); rich.output_category = MacroOutputCategory::Items; rich.kind = ProcMacroKind::DeriveLike;
    rich.header_indent = 251; rich.block_base = Some(65536); rich.input = encode(&stream); rich.views = Default::default();
    let mut dependency = ProductCrateIdentity::local("helper".into());
    dependency.target_triple = Some("host".into()); dependency.format_version = 65536;
    dependency.source_fingerprint.manifest_hash = Some("manifest".into());
    dependency.source_fingerprint.source_hash = Some("source".into());
    dependency.source_fingerprint.loaded_files = vec!["main.rk".into(), "雪.rk".into()];
    rich.definition_dependencies = vec![dependency, ProductCrateIdentity::local("other".into())];
    cases.push(("all provenance and integer widths".into(), encode_request(&rich).unwrap(), true));

    let mut empty = rich.clone(); empty.macro_name = "empty".into(); empty.kind = ProcMacroKind::AttributeLike;
    empty.expansion_id = None; empty.parent = None; empty.header_indent = 0; empty.block_base = None;
    empty.definition_dependencies.clear();
    let mut empty_stream = stream.clone(); empty_stream.sources[0].text.clear(); empty_stream.runs.clear();
    empty.input = encode(&empty_stream);
    cases.push(("empty stream".into(), encode_request(&empty).unwrap(), true));

    let mut reject = |name: &str, request: ProcMacroRequest| {
        let bytes = encode(&request);
        assert!(decode_request(&bytes).is_err(), "invalid test control: {name}");
        cases.push((name.into(), bytes, false));
    };
    for (name, count) in [("oversized lossless table count", 1048577u64), ("table count exceeds remaining bytes", 1000)] {
        let mut bad = rich.clone(); bad.input = b"RKLOSS04".to_vec(); bad.input.extend(encode(&count)); reject(name, bad);
    }
    let mut bad = golden.clone(); bad.compiler_version = "other".into(); reject("compiler version", bad);
    let mut bad = golden.clone(); bad.macro_identity.clear(); reject("empty identity", bad);
    let mut bad = golden.clone(); bad.invocation_id = invocation(0); reject("zero invocation", bad);
    let mut bad = golden.clone(); bad.expansion_id = Some(0); reject("zero expansion", bad);
    let mut bad = golden.clone(); bad.parent = Some(ExpansionId(0)); reject("zero parent", bad);
    let mut bad = rich.clone(); bad.block_base = Some(251); reject("layout margin", bad);
    let mut bad = rich.clone(); bad.header_indent = 1048577; reject("layout limit", bad);
    let mut bad = rich.clone(); bad.definition_dependencies.push(bad.definition_dependencies[0].clone()); reject("duplicate dependencies", bad);
    let mut bad = golden.clone(); bad.views.arguments.as_mut().unwrap().ranges[0] = 3..7; reject("argument UTF-8 cut", bad);
    let mut bad = golden.clone(); bad.views.arguments.as_mut().unwrap().ranges.reverse(); reject("argument order", bad);
    let mut bad = golden.clone(); bad.views.groups[0].range.end = 14; reject("group endpoints", bad);
    let mut bad = golden.clone(); bad.views.groups = vec![bad.views.groups[0].clone(); 129]; reject("group depth", bad);
    let mut bad = golden.clone(); bad.views.groups = vec![bad.views.groups[0].clone(); 16385]; reject("metadata length", bad);
    let mut bad = golden.clone(); bad.views.literals[0].mapping[0].decoded.end = 1; reject("decoded UTF-8 cut", bad);
    let mut bad = golden.clone(); bad.views.literals[0].mapping[0].source.end = 3; reject("source UTF-8 cut", bad);
    let mut bad = golden.clone(); bad.views.literals[0].mapping.pop(); reject("mapping coverage", bad);
    for (name, wire) in [
        ("bad source", { let mut wire = stream.clone(); wire.runs[0].source = 99; wire }),
        ("bad range", { let mut wire = stream.clone(); wire.runs[0].range.end = 999; wire }),
        ("bad kind", { let mut wire = stream.clone(); wire.runs[0].kind = 1; wire }),
        ("invalid run kind tag", { let mut wire = stream.clone(); wire.runs[0].kind = 3; wire }),
        ("invalid boundary tag", { let mut wire = stream.clone(); wire.runs[0].boundary = 2; wire }),
        ("forged adjacency", { let mut wire = stream.clone(); wire.runs[1].range = 3..4; wire }),
        ("bad magic", { let mut wire = stream.clone(); wire.magic[0] = 0; wire }),
        ("duplicate snapshot", { let mut wire = stream.clone(); wire.sources.push(wire.sources[0].clone()); wire }),
        ("empty source path", { let mut wire = stream.clone(); wire.sources[0].path.clear(); wire }),
        ("anchor UTF-8 cut", { let mut wire = stream.clone(); wire.anchor.offset = 5; wire }),
        ("projection overflow", { let mut wire = stream.clone(); wire.runs[1].origin.as_mut().unwrap().projection.as_mut().unwrap().output_start = usize::MAX; wire }),
        ("crossing captures", { let mut wire = stream.clone(); wire.expressions = vec![(0..4, 0), (2..7, 0)]; wire }),
        ("duplicate captures", { let mut wire = stream.clone(); wire.expressions = vec![(0..7, 0), (0..7, 0)]; wire }),
        ("input construction", { let mut wire = stream.clone(); wire.runs[0].construction = Some(rich.construction_context(ConstructionKind::CallSite)); wire }),
    ] {
        let mut bad = rich.clone(); bad.input = encode(&wire); reject(name, bad);
    }
    for (name, bytes, accepted) in &cases {
        assert_eq!(decode_request(bytes).is_ok(), *accepted, "Rust control {name}");
    }
    cases
}
