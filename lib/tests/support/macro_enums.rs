use std::path::Path;
use std::process::Command;

use rock_lib::ast::{NamedFieldsOrTypesList, ParseType, TopLevel};
use rock_lib::{Config, SourceProvider};

use super::macro_structs::declaration;
use super::macro_types::shape;
#[path = "../fixtures/syntax/enums.rs"]
mod fixtures;
use fixtures::{EnumCase, Fields, CASES};

fn expected(case: &EnumCase) -> String {
    let parsed = declaration(case.source);
    assert_eq!(parsed.is_some(), case.rust_accept, "{}", case.name);
    if let Some((code, highlight, related)) = case.error {
        let start = if highlight.is_empty() { case.source.trim_end_matches(['\r', '\n']).len() } else { case.source.rfind(highlight).unwrap() };
        let related = related.or(if code == "type-close" { Some("(") } else { None });
        return format!("{}\nE {code} {start} {} {}\n", case.name, start + highlight.len(), related.map(|text| case.source.find(text).unwrap() as i64).unwrap_or(-1));
    }
    let (node, offset) = parsed.unwrap();
    let TopLevel::EnumDecl(node) = node else { panic!("expected enum fixture") };
    assert_eq!(node.variants.len(), case.variants.len(), "{}", case.name);
    let mut text = format!("{}\n{}|{}|{}|{}\n", case.name, node.name.name, node.exported,
        node.name.span.start - offset, node.name.span.end - offset);
    for param in &node.generic_params {
        let kind = match &param.kind {
            None => "none".into(),
            Some(kind) if kind.args.is_empty() => format!("explicit:{}", shape(&kind.constructor)),
            Some(kind) => format!("holes:{}", shape(&ParseType::Application(kind.clone()))),
        };
        text.push_str(&format!("param:{}|{}|{}\n", param.name.name, kind, param.default.as_ref().map(|ty| shape(ty)).unwrap_or("none".into())));
    }
    for (variant, wanted) in node.variants.iter().zip(case.variants) {
        assert_eq!(variant.name.name, wanted.name);
        let start = variant.name.span.start - offset;
        let end = start + variant.name.name.len();
        let tag = match wanted.fields { Fields::Nullary => "nullary", Fields::Named(_) => "named", Fields::Positional(_) => "positional" };
        text.push_str(&format!("variant:{}|{tag}|{start}|{end}\n", wanted.name));
        match (&variant.fields, &wanted.fields) {
            (NamedFieldsOrTypesList::NamedFields(fields), Fields::Nullary) => assert!(fields.is_empty()),
            (NamedFieldsOrTypesList::TypesList(types), Fields::Positional(spellings)) => {
                assert_eq!(types.len(), spellings.len(), "{} {}", case.name, wanted.name);
                let mut after = end;
                for (ty, spelling) in types.iter().zip(*spellings) {
                    let start = after + case.source[after..].find(spelling).unwrap();
                    after = start + spelling.len();
                    text.push_str(&format!("position:{start}|{after}|{}\ntype:{spelling}\n", shape(ty)));
                }
            }
            (NamedFieldsOrTypesList::NamedFields(fields), Fields::Named(wanted)) => {
                assert_eq!(fields.len(), wanted.len());
                for (field, (name, spelling, default)) in fields.iter().zip(*wanted) {
                    assert_eq!(&field.name.name, name);
                    assert_eq!(field.default.is_some(), default.is_some());
                    let start = field.name.span.start - offset;
                    let end = field.name.span.end - offset;
                    let type_start = end + case.source[end..].find(spelling).unwrap();
                    let type_end = type_start + spelling.len();
                    text.push_str(&format!("field:{name}|{}|{start}|{end}|{type_start}|{type_end}|{}\ntype:{spelling}\n", field.public, shape(&field.ty)));
                    if let Some(default) = default {
                        let start = type_end + case.source[type_end..].find(default).unwrap();
                        text.push_str(&format!("default:{start}|{}|{default}\n", start + default.len()));
                    } else { text.push_str("default:none\n"); }
                }
            }
            _ => panic!("variant shape: {} {}", case.name, wanted.name),
        }
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
    for file in ["client.rk", "controls.rk", "types-client.rk", "structs-client.rk", "enums-client.rk"] {
        source.push_str(&std::fs::read_to_string(workspace.join("lib/tests/fixtures/syntax").join(file)).unwrap());
    }
    let readme = std::fs::read_to_string(workspace.join("rock_syntax/README.md")).unwrap();
    let section = readme.split("## C.07 enum derive input").nth(1).unwrap();
    source.push_str(section.split("```rock\n").nth(1).unwrap().split("```").next().unwrap());
    source.push_str("\nmain = !->\n");
    let mut wanted = String::new();
    for case in CASES {
        source.push_str(&format!("    run_enum {:?}, {:?}\n", case.name, case.source));
        wanted.push_str(&expected(case));
    }
    source.push_str("    enum_controls!\n");
    wanted.push_str("enum controls passed\n");
    let mut config = config.clone();
    config.extern_artifacts.push(("rock_syntax".into(), artifact));
    config.source_providers = vec![SourceProvider::Virtual { path: config.entry_file.clone(), text: source }];
    rock_lib::compile(&config).unwrap_or_else(|errors| panic!("enum syntax client: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), wanted);
}
