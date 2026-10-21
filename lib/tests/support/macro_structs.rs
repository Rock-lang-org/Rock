use std::path::Path;
use std::process::Command;

use rock_lib::ast::TopLevel;
use rock_lib::{parser, Config, SourceProvider};

use super::macro_types::shape;
#[path = "../fixtures/syntax/structs.rs"]
mod fixtures;
use fixtures::{StructCase, CASES};

pub fn declaration(text: &str) -> Option<(TopLevel, usize)> {
    let first = text.lines().find(|line| !line.trim().is_empty() && !line.trim().starts_with("/*"))?;
    let nested = first.starts_with(' ');
    let prefix = if nested { "mod nested\n" } else { "" };
    let source = format!("{prefix}{text}");
    let module = parser::parse_source("struct-fixture.rk".into(), &source, &Config::default()).ok()?;
    if module.top_levels.len() != 1 { return None; }
    let node = if nested {
        let TopLevel::Module(module) = &module.top_levels[0] else { return None };
        if module.0.top_levels.len() != 1 { return None; }
        module.0.top_levels[0].clone()
    } else { module.top_levels[0].clone() };
    Some((node, prefix.len()))
}

fn expected(case: &StructCase) -> String {
    let parsed = declaration(case.source);
    assert_eq!(parsed.is_some(), case.rust_accept, "{}", case.name);
    if let Some((code, start, end, related)) = case.error {
        return format!("{}\nE {code} {start} {end} {related}\n", case.name);
    }
    let (node, offset) = parsed.unwrap();
    let TopLevel::StructDecl(node) = node else { panic!("expected a struct fixture") };
    assert_eq!(node.fields.len(), case.fields.len(), "{}", case.name);
    let mut text = format!("{}\n{}|{}|{}|{}\n", case.name, node.name.name, node.exported,
        node.name.span.start - offset, node.name.span.end - offset);
    for param in &node.generic_params {
        let kind = match &param.kind {
            None => "none".into(),
            Some(kind) if kind.args.is_empty() => format!("explicit:{}", shape(&kind.constructor)),
            Some(kind) => format!("holes:{}", shape(&rock_lib::ast::ParseType::Application(kind.clone()))),
        };
        text.push_str(&format!("param:{}|{}|{}\n", param.name.name, kind, param.default.as_ref().map(|ty| shape(ty)).unwrap_or("none".into())));
    }
    for (field, (name, type_text, default)) in node.fields.iter().zip(case.fields) {
        assert_eq!(&field.name.name, name);
        assert_eq!(field.default.is_some(), default.is_some());
        let start = field.name.span.start - offset;
        let end = field.name.span.end - offset;
        let type_start = case.source[end..].find(type_text).unwrap() + end;
        let type_end = type_start + type_text.len();
        text.push_str(&format!("field:{}|{}|{}|{}|{}|{}|{}\n", name, field.public, start, end, type_start, type_end, shape(&field.ty)));
        text.push_str(&format!("type:{type_text}\n"));
        if let Some(default) = default {
            let start = type_end + case.source[type_end..].find(default).unwrap();
            text.push_str(&format!("default:{start}|{}|{default}\n", start + default.len()));
        } else { text.push_str("default:none\n"); }
    }
    text
}

pub fn verify_oracle() {
    for case in CASES { expected(case); }
}

pub fn verify(config: &Config) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let artifact = super::macro_syntax::build_syntax(config);
    let mut source = String::new();
    for file in ["client.rk", "controls.rk", "types-client.rk", "structs-client.rk"] {
        source.push_str(&std::fs::read_to_string(workspace.join("lib/tests/fixtures/syntax").join(file)).unwrap());
    }
    let readme = std::fs::read_to_string(workspace.join("rock_syntax/README.md")).unwrap();
    let section = readme.split("## C.06 struct derive input").nth(1).unwrap();
    source.push_str(section.split("```rock\n").nth(1).unwrap().split("```").next().unwrap());
    source.push_str("\nmain = !->\n");
    let mut wanted = String::new();
    for case in CASES {
        source.push_str(&format!("    run_struct {:?}, {:?}\n", case.name, case.source));
        wanted.push_str(&expected(case));
    }
    source.push_str("    struct_controls!\n");
    wanted.push_str("struct controls passed\n");
    let mut config = config.clone();
    config.extern_artifacts.push(("rock_syntax".into(), artifact));
    config.source_providers = vec![SourceProvider::Virtual { path: config.entry_file.clone(), text: source }];
    rock_lib::compile(&config).unwrap_or_else(|errors| panic!("struct syntax client: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), wanted);
}
