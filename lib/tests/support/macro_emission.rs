use std::path::Path;
use std::process::Command;

use rock_lib::crate_system::TargetRole;
use rock_lib::macro_expansion::host::ProcMacroSdk;
use rock_lib::parser::{parse_type, ErrorScope, ParseCtx, Parser};
use rock_lib::lexer::TokenType;
use rock_lib::{Config, SourceProvider};

const TYPES: &[(&str, &[&str], &str)] = &[
    ("Name", &["T /* retained */"], "name(T)"),
    ("Associated", &["T", "Item"], "path(name(T),name(Item))"),
    ("Application", &["F", "A -> B", "G C, D"], "apply(name(F),function(name(A),name(B)),apply(name(G),name(C),name(D)))"),
    ("SharedReference", &["A -> B"], "ref(function(name(A),name(B)))"),
    ("MutableReference", &["F A, B"], "mut-ref(apply(name(F),name(A),name(B)))"),
    ("Pointer", &["A -> B"], "pointer(function(name(A),name(B)))"),
    ("Unit", &[], "unit()"),
    ("Grouped", &["F A, B"], "apply(name(F),name(A),name(B))"),
    ("Tuple", &["A"], "tuple(name(A))"),
    ("Tuple", &["F A, B", "C -> D"], "tuple(apply(name(F),name(A),name(B)),function(name(C),name(D)))"),
    ("Slice", &["A -> B"], "slice(function(name(A),name(B)))"),
    ("Array", &["F A, B", "18446744073709551615"], "array(apply(name(F),name(A),name(B)),length:-1)"),
    ("Hole", &[], "hole()"),
    ("Function", &["A -> B", "F C, D"], "function(function(name(A),name(B)),apply(name(F),name(C),name(D)))"),
];

fn type_shape(text: &str) -> String {
    let _scope = ErrorScope::new();
    let tokens = super::macro_types::tokens(text).unwrap();
    let config = Config::default();
    let (rest, ty) = parse_type.process(ParseCtx::from(&tokens, &config)).unwrap();
    assert!(matches!(rest.seek().unwrap().token_type, TokenType::Eof), "{text}");
    super::macro_types::shape(&ty)
}

pub fn verify_oracle() {
    assert_eq!(type_shape("((F) (A -> B), (G C, D))"), TYPES[2].2);
    assert_eq!(type_shape("[(F A, B); 18446744073709551615]"), TYPES[11].2);
}

pub fn verify(config: &Config) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let artifact = super::macro_syntax::build_syntax(config);
    let fixture = |name| std::fs::read_to_string(workspace.join("lib/tests/fixtures/syntax").join(name)).unwrap();
    let mut source = format!("{}\n{}\n{}\n{}", fixture("client.rk"), fixture("controls.rk"), fixture("types-client.rk"), fixture("emission-client.rk"));
    let readme = std::fs::read_to_string(workspace.join("rock_syntax/README.md")).unwrap();
    let section = readme.split("## C.08 token edits and output construction").nth(1).unwrap();
    source.push_str(section.split("```rock\n").nth(1).unwrap().split("```").next().unwrap());
    source.push_str("\nmain = !->\n    context = emission_context!\n");
    for (kind, children, _) in TYPES {
        source.push_str("    mut children: Vec MacroOutput = Vec::new!\n");
        for child in *children { source.push_str(&format!("    children.push (test_tokens &context, {child:?})\n")); }
        source.push_str(&format!("    run_emission &context, TypeKind::{kind}, (children.as_slice!)\n"));
    }
    source.push_str("    emission_controls &context\n");
    let mut client = config.clone();
    client.extern_artifacts.push(("rock_syntax".into(), artifact.clone()));
    client.source_providers = vec![SourceProvider::Virtual { path: client.entry_file.clone(), text: source }];
    rock_lib::compile(&client).unwrap_or_else(|errors| panic!("emission client: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let output = Command::new(client.output_dir.join("main")).output().unwrap();
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    let mut lines = text.lines();
    for (_, _, expected) in TYPES { assert_eq!(&type_shape(lines.next().unwrap()), expected); }
    assert_eq!(lines.collect::<Vec<_>>(), ["emission controls passed"]);
    verify_proc(config, artifact, fixture("emission-provider.rk"), fixture("emission-consumer.rk"));
}

fn verify_proc(config: &Config, syntax: std::path::PathBuf, provider: String, consumer_source: String) {
    let mut host = config.clone();
    host.current_crate_name = Some("emission_provider".into());
    host.target_role = TargetRole::ProcMacro;
    host.output_dir = config.output_dir.parent().unwrap().join("host");
    host.extern_artifacts.push(("rock_syntax".into(), syntax));
    host.source_providers = vec![SourceProvider::Virtual { path: host.entry_file.clone(), text: provider }];
    let output = rock_lib::compile_proc_macro_with_products(&host, &ProcMacroSdk { dependency: "rock_macro".into() })
        .unwrap_or_else(|errors| panic!("emission provider: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let artifact = host.output_dir.join("emission_provider.rkca");
    output.products.unwrap().write_artifact_to_path(&artifact).unwrap();
    let mut consumer = config.clone();
    consumer.entry_file = config.output_dir.parent().unwrap().join("consumer.rk");
    consumer.output_dir = config.output_dir.parent().unwrap().join("consumer");
    consumer.current_crate_name = Some("emission_consumer".into());
    consumer.no_proc_execution = false;
    consumer.extern_artifacts.retain(|(name, _)| name == "std");
    consumer.extern_artifacts.push(("emission_provider".into(), artifact));
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: consumer_source }];
    rock_lib::compile(&consumer).unwrap_or_else(|errors| panic!("emission consumer: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let output = Command::new(consumer.output_dir.join("consumer")).output().unwrap();
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, b"17\n53\n23\n31\n41\n");
    for (name, expected) in [("bad_edit", "edit ranges must be ordered"), ("bad_arity", "invalid type emission arity")] {
        consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(),
            text: format!("> emission_provider::{name}\nmain = !->\n    result = %{name} hello\n") }];
        let errors = rock_lib::analyze(&consumer).unwrap_err();
        assert!(errors.0.iter().any(|error| error.message.contains(expected)), "{name}: {errors:?}");
    }
}
