use std::path::Path;
use std::process::Command;

use rock_lib::ast::ParseType;
use rock_lib::lexer::interpretation::{PieceKind, RockView};
use rock_lib::lexer::lossless::{LosslessStream, ScanLimits, SyntaxContext};
use rock_lib::lexer::{Span, Token, TokenType};
use rock_lib::parser::{parse_type, ErrorScope, ParseCtx, Parser};
use rock_lib::{Config, SourceProvider};

#[path = "../fixtures/syntax/types.rs"]
mod fixtures;
use fixtures::{TypeCase, TYPE_CASES};

pub fn tokens(source: &str) -> Option<Vec<Token>> {
    let stream = LosslessStream::from_bytes("type-fixture.rk".into(), source.as_bytes().to_vec(), ScanLimits {
        max_source_bytes: 1048576, max_runs: 65536,
    }).unwrap();
    let mut view = RockView::new(&stream);
    let mut tokens = Vec::new();
    loop {
        match view.next_piece() {
            Ok(Some(piece)) => if let PieceKind::Token(token_type) = piece.kind {
                tokens.push(Token { token_type, context: piece.context.unwrap_or(SyntaxContext::ROOT), span: piece.span });
            },
            Ok(None) => break,
            Err(_) => return None,
        }
    }
    tokens.push(Token { token_type: TokenType::Eof, context: SyntaxContext::ROOT,
        span: Span::new("type-fixture.rk".into(), source.len(), source.len()) });
    Some(tokens)
}

fn parsed(case: &TypeCase) -> Option<ParseType> {
    let _scope = ErrorScope::new();
    let tokens = tokens(case.source)?;
    let config = Config::default();
    let (rest, ty) = parse_type.process(ParseCtx::from(&tokens, &config)).ok()?;
    matches!(rest.seek().ok()?.token_type, TokenType::Eof).then_some(ty)
}

fn joined(tag: &str, types: impl IntoIterator<Item = String>) -> String {
    format!("{tag}({})", types.into_iter().collect::<Vec<_>>().join(","))
}

pub fn shape(ty: &ParseType) -> String {
    match ty {
        ParseType::Type(name) => format!("name({})", name.name),
        ParseType::Associated { base, member } => format!("path(name({}),name({}))", base.name, member.name),
        ParseType::Application(application) => joined("apply", std::iter::once(shape(&application.constructor)).chain(application.args.iter().map(shape))),
        ParseType::Reference { is_mut, pointee } => joined(if *is_mut { "mut-ref" } else { "ref" }, [shape(pointee)]),
        ParseType::Pointer(pointee) => joined("pointer", [shape(pointee)]),
        ParseType::Unit(_) => "unit()".into(),
        ParseType::Tuple(types) => joined("tuple", types.iter().map(shape)),
        ParseType::Slice(inner) => joined("slice", [shape(inner)]),
        // Use the same bijective signed bit-pattern rendering as the test client.
        ParseType::Array { inner, len } => joined("array", [shape(inner), format!("length:{}", *len as i64)]),
        ParseType::Hole(_) => "hole()".into(),
        ParseType::Function(types) => joined("function", types.iter().map(shape)),
        ParseType::Lambda(_) => panic!("type lambdas are outside C.04's supported grammar"),
    }
}

fn expected(case: &TypeCase) -> String {
    let parsed = parsed(case);
    assert_eq!(parsed.is_some(), case.rust_accept, "{}", case.name);
    let result = match case.error {
        Some((code, start, end)) => {
            // Closing errors carry a second location; every such fixture opens at 0.
            format!("E {code} {start} {end} {}", if code == "type-close" { 0 } else { -1 })
        }
        None => shape(parsed.as_ref().unwrap()),
    };
    format!("{}\n{result}\n", case.name)
}

pub fn verify_oracle() {
    for case in TYPE_CASES { expected(case); }
}

pub fn verify(config: &Config) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let artifact = super::macro_syntax::build_syntax(config);
    let mut source = std::fs::read_to_string(workspace.join("lib/tests/fixtures/syntax/client.rk")).unwrap();
    source.push_str(&std::fs::read_to_string(workspace.join("lib/tests/fixtures/syntax/controls.rk")).unwrap());
    source.push_str(&std::fs::read_to_string(workspace.join("lib/tests/fixtures/syntax/types-client.rk")).unwrap());
    source.push_str("\nmain = !->\n");
    let mut wanted = String::new();
    for case in TYPE_CASES {
        source.push_str(&format!("    run_type {:?}, {:?}\n", case.name, case.source));
        wanted.push_str(&expected(case));
    }
    source.push_str("    type_controls!\n");
    wanted.push_str("type controls passed\n");
    let mut config = config.clone();
    config.extern_artifacts.push(("rock_syntax".into(), artifact));
    config.source_providers = vec![SourceProvider::Virtual { path: config.entry_file.clone(), text: source }];
    rock_lib::compile(&config).unwrap_or_else(|errors| panic!("type syntax client: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), wanted);
}
