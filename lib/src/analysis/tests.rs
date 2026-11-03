use std::path::PathBuf;

use crate::{Config, SourceProvider};

fn analyze(source: &str) -> super::Analysis {
    let path = PathBuf::from("/virtual/main.rk");
    crate::analyze(&Config {
        entry_file: path.clone(),
        source_providers: vec![SourceProvider::Virtual {
            path,
            text: source.to_string(),
        }],
        no_prelude: true,
        no_std: true,
        ..Config::default()
    })
    .unwrap()
}

#[test]
fn analysis_reports_inferred_local_types_and_function_signatures() {
    let source = "id = value -> value\n\nmain = ->\n    answer = id 42\n    answer\n";
    let analysis = analyze(source);
    let path = PathBuf::from("/virtual/main.rk");

    let local_offset = source.rfind("answer").unwrap();
    let local = analysis.hover(&path, local_offset).unwrap();
    assert_eq!(local.contents, "answer: I64");

    let function_offset = source.find("id 42").unwrap();
    let function = analysis.hover(&path, function_offset).unwrap();
    assert!(function.contents.starts_with("id = value: "));

    let argument_offset = source.find("42").unwrap() + 1;
    let signature = analysis.signature(&path, argument_offset).unwrap();
    assert!(signature.label.starts_with("id = value: "));
    assert_eq!(signature.parameters.len(), 1);
    assert_eq!(signature.active_parameter, 0);
}

#[test]
fn analysis_reports_named_types_and_fields() {
    let source = "struct Holder\n    < value: I64\n\nread: Holder -> I64\nread = holder -> holder.value\n\nmain = -> 0\n";
    let analysis = analyze(source);
    let path = PathBuf::from("/virtual/main.rk");

    let structure = analysis
        .hover(&path, source.find("Holder").unwrap())
        .unwrap();
    assert_eq!(structure.contents, "struct Holder\n    value: I64");

    let declaration = analysis
        .hover(&path, source.find("value").unwrap())
        .unwrap();
    assert_eq!(declaration.contents, "Holder.value: I64");

    let reference = analysis
        .hover(&path, source.rfind("value").unwrap())
        .unwrap();
    assert_eq!(reference.contents, "Holder.value: I64");
}

#[test]
fn analysis_errors_keep_virtual_source_for_lsp_conversion() {
    let path = PathBuf::from("/virtual/error.rk");
    let source = "main = ->\n    `\n";
    let diagnostics = crate::analyze(&Config {
        entry_file: path.clone(),
        source_providers: vec![SourceProvider::Virtual {
            path,
            text: source.to_string(),
        }],
        no_prelude: true,
        no_std: true,
        ..Config::default()
    })
    .unwrap_err();

    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic
        .source
        .as_ref()
        .is_some_and(|source| { source.text == "main = ->\n    `\n" })));
}
