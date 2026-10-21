use std::path::Path;
use std::process::Command;

use rock_lib::lexer::groups::{BalancedGroups, Delimiter, GroupError, GroupLimits};
use rock_lib::lexer::interpretation::{PieceKind, RockError, RockView};
use rock_lib::lexer::lossless::{LosslessStream, ScanLimits};
use rock_lib::lexer::{LexerError, TokenType};
use rock_lib::{Config, SourceProvider};

#[path = "../fixtures/syntax/lexical.rs"]
mod fixtures;
use fixtures::{Case, CASES};

fn cases() -> &'static [Case] { CASES }

fn stream(case: &Case) -> LosslessStream {
    LosslessStream::from_bytes("fixture.rk".into(), case.source.as_bytes().to_vec(), ScanLimits {
        max_source_bytes: 1048576, max_runs: 65536,
    }).unwrap()
}

fn tag(kind: &PieceKind) -> &'static str {
    match kind {
        PieceKind::Whitespace => "whitespace",
        PieceKind::LineEnding => "line-ending",
        PieceKind::LineComment => "line-comment",
        PieceKind::BlockComment => "block-comment",
        PieceKind::Token(token) => match token {
            TokenType::Ident(_) => "ident", TokenType::Type(_) => "type", TokenType::Keyword(_) => "keyword",
            TokenType::Number(_) => "number", TokenType::Float(_) => "float", TokenType::String(_) => "string", TokenType::Char(_) => "char",
            TokenType::MacroVar(_) => "macro-var", TokenType::MacroInvoc(_) => "macro-invocation", TokenType::NativeOperator(_) => "native",
            TokenType::Operator(_) => "operator", TokenType::StuckOperator(_) => "stuck-operator",
            TokenType::ExpressionFragment(_) => "expression", _ => "symbol",
        },
    }
}

fn error_line(error: RockError) -> String {
    let (code, span) = match error {
        RockError::Lexer(LexerError::UnknownToken(ch, span)) => (if ch == '\'' || ch == '"' { "quote" } else { "character" }, span),
        RockError::UnterminatedBlockComment(span) => ("comment", span),
        other => panic!("unexpected fixture error: {other:?}"),
    };
    format!("E {code} {} {} -1\n", span.start, span.end)
}

fn oracle(case: &Case) -> String {
    let stream = stream(case);
    let mut output = format!("{}\n", case.name);
    let mut view = RockView::new(&stream);
    let mut lex_ok = true;
    loop {
        match view.next_piece() {
            Ok(Some(piece)) => output.push_str(&format!("T {} {} {} {}\n", tag(&piece.kind), piece.range.start, piece.range.end,
                if piece.context.is_some() { 7 } else { -1 })),
            Ok(None) => break,
            Err(error) => { lex_ok = false; output.push_str(&error_line(error)); break; }
        }
    }
    assert_eq!(lex_ok, case.lex, "{}", case.name);
    match BalancedGroups::parse(&stream, GroupLimits { max_depth: 128, max_groups: 16384 }) {
        Ok(groups) => {
            assert!(case.groups, "{}", case.name);
            for group in groups.records() {
                let delimiter = match group.delimiter { Delimiter::Paren => "paren", Delimiter::Bracket => "bracket", Delimiter::Brace => "brace" };
                output.push_str(&format!("G {delimiter} {} {} {} {} {}\n", group.open.start, group.open.end, group.close.start, group.close.end,
                    group.parent.map(|index| index as i64).unwrap_or(-1)));
            }
        }
        Err(error) => {
            assert!(!case.groups, "{}: {error:?}", case.name);
            output.push_str(&match error {
                GroupError::Rock(error) => error_line(error),
                GroupError::UnexpectedClose { span, .. } => format!("E unexpected-close {} {} -1\n", span.start, span.end),
                GroupError::MismatchedClose { open, close, .. } => format!("E mismatch {} {} {}\n", close.start, close.end, open.start),
                GroupError::Unclosed { open, .. } => format!("E unclosed {} {} -1\n", open.start, open.end),
                other => panic!("unexpected fixture group error: {other:?}"),
            });
        }
    }
    output.push_str("done\n");
    output
}

pub fn verify_oracle() {
    for case in cases() { oracle(case); }
}

pub fn build_syntax(config: &Config) -> std::path::PathBuf {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let directory = config.output_dir.parent().unwrap().join("syntax");
    std::fs::create_dir_all(&directory).unwrap();
    let built = rock_lib::compile_with_products(&Config {
        entry_file: workspace.join("rock_syntax/src/lib.rk"), output_dir: directory.clone(),
        current_crate_name: Some("rock_syntax".into()), extern_artifacts: config.extern_artifacts.clone(),
        no_std: true, no_prelude: true, no_proc_execution: true, no_link: true,
        emit_object: Some(directory.join("rock_syntax.o")), ..Default::default()
    }).unwrap_or_else(|errors| panic!("syntax library: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let artifact = directory.join("rock_syntax.rkca");
    let products = built.products.unwrap();
    assert!(products.proc_macros.is_empty());
    products.write_artifact_to_path(&artifact).unwrap();
    artifact
}

pub fn verify(config: &Config) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let artifact = build_syntax(config);
    let cases = cases();
    let mut source = std::fs::read_to_string(workspace.join("lib/tests/fixtures/syntax/client.rk")).unwrap();
    source.push_str(&std::fs::read_to_string(workspace.join("lib/tests/fixtures/syntax/controls.rk")).unwrap());
    source.push_str("\nmain = !->\n");
    let mut expected = String::new();
    for case in cases {
        source.push_str(&format!("    run_case {:?}, (input {:?})\n", case.name, case.source));
        expected.push_str(&oracle(case));
    }
    source.push_str("    controls!\n");
    expected.push_str("controls passed\n");
    let mut config = config.clone();
    config.extern_artifacts.push(("rock_syntax".into(), artifact));
    config.source_providers = vec![SourceProvider::Virtual { path: config.entry_file.clone(), text: source }];
    rock_lib::compile(&config).unwrap_or_else(|errors| panic!("syntax client: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);

    let readme = std::fs::read_to_string(workspace.join("rock_syntax/README.md")).unwrap();
    let example = readme.split("```rock\n").nth(1).unwrap().split("```").next().unwrap();
    config.source_providers = vec![SourceProvider::Virtual { path: config.entry_file.clone(), text: example.into() }];
    rock_lib::compile(&config).unwrap_or_else(|errors| panic!("syntax documentation: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, b"type\n1\napply\nPacket(T)\n");
}
