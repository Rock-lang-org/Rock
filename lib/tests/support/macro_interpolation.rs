use std::path::Path;

use rock_lib::crate_system::TargetRole;
use rock_lib::macro_expansion::host::ProcMacroSdk;
use rock_lib::{Config, SourceProvider};

pub fn verify(config: &Config) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let directory = config.output_dir.parent().unwrap().join("interpolation");
    let std = config.extern_artifacts.iter().find(|(name, _)| name == "std").unwrap().1.clone();
    let runtime = super::build_library(workspace.join("lib/tests/fixtures/macros/interpolation/runtime/src/lib.rk"),
        "ink_runtime", &directory.join("runtime"), vec![("std".into(), std.clone())]);
    let mut host = config.clone();
    host.current_crate_name = Some("weave".into());
    host.target_role = TargetRole::ProcMacro;
    host.no_prelude = false;
    host.output_dir = directory.join("provider");
    host.extern_artifacts.push(("ink_runtime".into(), runtime.clone()));
    host.source_providers = vec![SourceProvider::Virtual { path: host.entry_file.clone(),
        text: include_str!("../fixtures/macros/interpolation/provider/src/lib.rk").into() }];
    let output = rock_lib::compile_proc_macro_with_products(&host, &ProcMacroSdk { dependency: "rock_macro".into() })
        .unwrap_or_else(|errors| panic!("interpolation provider: {errors:?}"));
    let mut products = output.products.unwrap();
    products.proc_macros[0].executable = "main".into();
    let artifact = host.output_dir.join("weave.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let mut consumer = Config {
        entry_file: directory.join("consumer.rk"), output_dir: directory.join("consumer"), no_std: true,
        extern_artifacts: vec![("std".into(), std), ("ink_runtime".into(), runtime), ("weave".into(), artifact)], ..Default::default()
    };
    let source = include_str!("../fixtures/macros/interpolation/consumer.rk").to_string();
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source }];
    rock_lib::compile(&consumer).unwrap_or_else(|errors| panic!("interpolation caller: {errors:?}"));
    let output = super::run_client(&consumer.output_dir.join("consumer"));
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"Hello Ada: 3\n");
    let source = "> weave::weave\nmain = !-> (%weave \"é\\n{missing}\").println!\n";
    let offset = source.find("missing").unwrap();
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source.into() }];
    let errors = rock_lib::analyze(&consumer).unwrap_err();
    assert!(errors.0.iter().any(|error| error.message.contains("Unknown variable: missing") && error.primary.as_ref().is_some_and(|label|
        label.span.file_path == consumer.entry_file && label.span.start == offset && label.span.end == offset + 7)), "{errors:?}");
    for (literal, message, marker) in [
        ("é\\n}", "unmatched interpolation closing brace", "}"),
        ("é\\n{}", "empty interpolation hole", "{}"),
        ("é\\n{missing", "unclosed interpolation hole", "{"),
        ("é\\n{a{b}", "nested braces are outside the fixture grammar", "{b"),
    ] {
        let source = format!("> weave::weave\nmain = !-> (%weave \"{literal}\").println!\n");
        let offset = source.find(marker).unwrap();
        let width = if marker == "{}" { 2 } else { 1 };
        consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source }];
        let errors = rock_lib::analyze(&consumer).unwrap_err();
        assert!(errors.0.iter().any(|error| error.message.contains(message) && error.primary.as_ref().is_some_and(|label|
            label.span.file_path == consumer.entry_file && label.span.start == offset && label.span.end == offset + width)), "{errors:?}");
    }
    let source = "> weave::weave\nmain = !->\n    name = String::from \"Ada\"\n    (%weave \"é\\n\\\"{name}\\\"\").println!\n";
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source.into() }];
    rock_lib::compile(&consumer).unwrap_or_else(|errors| panic!("interpolation literal quoting: {errors:?}"));
    let output = super::run_client(&consumer.output_dir.join("consumer"));
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, "é\n\"Ada\"\n".as_bytes());
    let source = "> weave::weave\nstruct Unsupported\nmain = !->\n    item = Unsupported\n    (%weave \"{item}\").println!\n";
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source.into() }];
    let errors = rock_lib::analyze(&consumer).unwrap_err();
    let report = format!("{errors:?}");
    assert!(report.contains("Paint") && report.contains("Unsupported"), "{report}");
    let source = include_str!("../fixtures/macros/interpolation/consumer.rk").to_string();
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source }];
    consumer.extern_artifacts.retain(|(name, _)| name != "ink_runtime");
    let errors = rock_lib::analyze(&consumer).unwrap_err();
    assert!(format!("{errors:?}").contains("required runtime dependency ink_runtime@0.1.0"), "{errors:?}");
    eprintln!("SDK acceptance: B.21 caller holes/exact Unicode-escape errors/library grammar/Paint/runtime dependency controls passed");
}
