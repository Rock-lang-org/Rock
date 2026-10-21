use std::path::Path;
use std::process::Command;

use rock_lib::ast::{GenericParamDecl, ParseType, WhereClause};
use rock_lib::lexer::TokenType;
use rock_lib::parser::{function_sig, parse_generic_param_list, parse_type, ErrorScope, ParseCtx, Parser};
use rock_lib::{Config, SourceProvider};

use super::macro_types::{shape, tokens};
#[path = "../fixtures/syntax/generics.rs"]
mod fixtures;
use fixtures::{GenericCase, BINDERS, BOUNDS};

fn binders(source: &str) -> Option<Vec<GenericParamDecl>> {
    let _scope = ErrorScope::new();
    let tokens = tokens(source)?;
    let config = Config::default();
    let (rest, params) = parse_generic_param_list.process(ParseCtx::from(&tokens, &config)).ok()?;
    matches!(rest.seek().ok()?.token_type, TokenType::Eof).then_some(params)
}

fn bounds(source: &str) -> Option<Vec<WhereClause>> {
    let _scope = ErrorScope::new();
    let mut tokens = tokens(&format!("f: () {source}"))?;
    let mut eol = tokens.last()?.clone();
    eol.token_type = TokenType::Eol;
    tokens.insert(tokens.len() - 1, eol);
    let config = Config::default();
    let (rest, signature) = function_sig.process(ParseCtx::from(&tokens, &config)).ok()?;
    (matches!(rest.seek().ok()?.token_type, TokenType::Eof) && !signature.where_clauses.is_empty()).then_some(signature.where_clauses)
}

fn kind_shape(param: &GenericParamDecl) -> String {
    match &param.kind {
        None => "none".into(),
        Some(kind) if kind.args.is_empty() => format!("explicit:{}", shape(&kind.constructor)),
        Some(kind) => format!("holes:{}", shape(&ParseType::Application(kind.clone()))),
    }
}

fn error(case: &GenericCase) -> Option<String> {
    case.error.map(|(code, start, end, related)| format!("{}\nE {code} {start} {end} {related}\n", case.name))
}

fn binder_expected(case: &GenericCase) -> String {
    let parsed = binders(case.source);
    assert_eq!(parsed.is_some(), case.rust_accept, "binder {}", case.name);
    if let Some(error) = error(case) { return error; }
    let params = parsed.unwrap();
    let mut result = format!("{}\n", case.name);
    for param in &params {
        result.push_str(&format!("{}|{}|{}\n", param.name.name, kind_shape(param), param.default.as_ref().map(|ty| shape(ty)).unwrap_or("none".into())));
    }
    if let Some(param) = params.iter().find(|param| param.kind.as_ref().is_some_and(|kind| kind.args.is_empty() && !matches!(kind.constructor.as_ref(), ParseType::Type(_)))) {
        let start = if case.source.as_bytes()[param.span.start] == b'{' { param.span.start } else { param.span.start - 1 };
        result.push_str(&format!("emit-error {start} {}\n", case.source.len()));
        return result;
    }
    let impl_binders = params.iter().map(|param| match &param.kind {
        Some(kind) if !kind.args.is_empty() => format!("({})", &case.source[kind.span.start..kind.span.end]),
        _ => param.name.name.clone(),
    }).collect::<Vec<_>>().join(",");
    let arguments = impl_binders.clone();
    if !params.is_empty() {
        let clauses = bounds(&format!("where {impl_binders}")).expect("emitted impl subjects parse");
        assert_eq!(clauses.len(), params.len(), "{}", case.name);
        for (clause, param) in clauses.iter().zip(&params) {
            let expected = match &param.kind {
                Some(kind) if !kind.args.is_empty() => shape(&ParseType::Application(kind.clone())),
                _ => format!("name({})", param.name.name),
            };
            assert_eq!(shape(&clause.subject), expected);
            assert!(clause.trait_bound.is_none());
        }
        let tokens = tokens(&format!("Witness {arguments}")).unwrap();
        let config = Config::default();
        let (rest, ty) = parse_type.process(ParseCtx::from(&tokens, &config)).unwrap();
        assert!(matches!(rest.seek().unwrap().token_type, TokenType::Eof));
        let ParseType::Application(application) = ty else { panic!("emitted type arguments") };
        assert_eq!(application.args.len(), params.len());
        for (argument, param) in application.args.iter().zip(&params) {
            let expected = match &param.kind {
                Some(kind) if !kind.args.is_empty() => shape(&ParseType::Application(kind.clone())),
                _ => format!("name({})", param.name.name),
            };
            assert_eq!(shape(argument), expected);
        }
    }
    result.push_str(&format!("args:{arguments}\nimpl:{impl_binders}\n"));
    result
}

fn bounds_expected(case: &GenericCase) -> String {
    let parsed = bounds(case.source);
    assert_eq!(parsed.is_some(), case.rust_accept, "bound {}", case.name);
    if let Some(error) = error(case) { return error; }
    let mut result = format!("{}\n", case.name);
    for clause in parsed.unwrap() {
        result.push_str(&format!("{}|{}\n", shape(&clause.subject), clause.trait_bound.as_ref().map(shape).unwrap_or("none".into())));
    }
    result
}

pub fn verify_oracle() {
    for case in BINDERS { binder_expected(case); }
    for case in BOUNDS { bounds_expected(case); }
}

pub fn verify(config: &Config) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let artifact = super::macro_syntax::build_syntax(config);
    let mut source = String::new();
    for file in ["client.rk", "controls.rk", "types-client.rk", "generics-client.rk"] {
        source.push_str(&std::fs::read_to_string(workspace.join("lib/tests/fixtures/syntax").join(file)).unwrap());
    }
    source.push_str("\nmain = !->\n");
    let mut wanted = String::new();
    for case in BINDERS {
        source.push_str(&format!("    run_binders {:?}, {:?}\n", case.name, case.source));
        wanted.push_str(&binder_expected(case));
    }
    for case in BOUNDS {
        source.push_str(&format!("    run_bounds {:?}, {:?}\n", case.name, case.source));
        wanted.push_str(&bounds_expected(case));
    }
    source.push_str("    generic_controls!\n");
    wanted.push_str("generic controls passed\n");
    let mut config = config.clone();
    config.extern_artifacts.push(("rock_syntax".into(), artifact));
    config.source_providers = vec![SourceProvider::Virtual { path: config.entry_file.clone(), text: source }];
    rock_lib::compile(&config).unwrap_or_else(|errors| panic!("generic syntax client: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    let output = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output, wanted);
    verify_emitted_program(&config, &output);
}

// Spelling is consumed ONLY by this test's second ordinary compiler check.
// Library emission above remains SDK token slices/construction, never strings.
fn verify_emitted_program(config: &Config, output: &str) {
    fn fragments<'a>(output: &'a str, name: &str) -> (&'a str, &'a str) {
        let section = output.split_once(&format!("{name}\n")).unwrap().1;
        let mut lines = section.lines();
        let arguments = lines.find_map(|line| line.strip_prefix("args:")).unwrap();
        let binders = lines.next().unwrap().strip_prefix("impl:").unwrap();
        (arguments, binders)
    }
    let (pair_args, pair_binders) = fragments(output, "earlier-default");
    let (holder_args, holder_binders) = fragments(output, "default-hole-constructor");
    let (higher_args, higher_binders) = fragments(output, "explicit-type");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let source = std::fs::read_to_string(workspace.join("lib/tests/fixtures/syntax/generic-emission.rk.in")).unwrap()
        .replace("{{PAIR_ARGS}}", pair_args).replace("{{PAIR_BINDERS}}", pair_binders)
        .replace("{{HOLDER_ARGS}}", holder_args).replace("{{HOLDER_BINDERS}}", holder_binders)
        .replace("{{HIGHER_ARGS}}", higher_args).replace("{{HIGHER_BINDERS}}", higher_binders);
    let mut config = config.clone();
    config.source_providers = vec![SourceProvider::Virtual { path: config.entry_file.clone(), text: source }];
    rock_lib::compile(&config).unwrap_or_else(|errors| panic!("emitted generic impl program: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, b"11\n17\n");
}
