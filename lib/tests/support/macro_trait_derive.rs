use std::path::Path;
use std::process::Command;

use rock_lib::crate_system::TargetRole;
use rock_lib::diagnostic::DiagnosticCode;
use rock_lib::macro_expansion::host::ProcMacroSdk;
use rock_lib::{Config, SourceProvider};

const PRELUDE: &str = "> metric_derive::SumMetric\n> metric_derive::UnboundedMetric\ntrait Metric\n    @metric: I64\nimpl Metric for I64\n    @metric = -> *self\n";

fn build_provider(config: &Config) -> (Config, rock_lib::products::CompilerProducts) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixtures = workspace.join("lib/tests/fixtures/macros/derive_metric");
    let syntax = super::macro_syntax::build_syntax(config);
    eprintln!("trait derive: syntax artifact built");
    let mut host = config.clone();
    host.current_crate_name = Some("metric_derive".into());
    host.target_role = TargetRole::ProcMacro;
    host.output_dir = config.output_dir.parent().unwrap().join("host");
    host.extern_artifacts.push(("rock_syntax".into(), syntax));
    let text = std::fs::read_to_string(fixtures.join("provider.rk")).unwrap();
    host.source_providers = vec![SourceProvider::Virtual { path: host.entry_file.clone(), text }];
    let built = rock_lib::compile_proc_macro_with_products(&host, &ProcMacroSdk { dependency: "rock_macro".into() })
        .unwrap_or_else(|errors| panic!("trait derive provider: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    eprintln!("trait derive: provider artifact built");
    (host, built.products.unwrap())
}

pub fn verify(config: &Config) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixtures = workspace.join("lib/tests/fixtures/macros/derive_metric");
    let (host, products) = build_provider(config);
    let artifact = host.output_dir.join("metric_derive.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let mut consumer = config.clone();
    consumer.entry_file = config.output_dir.parent().unwrap().join("consumer.rk");
    consumer.output_dir = config.output_dir.parent().unwrap().join("consumer");
    consumer.current_crate_name = Some("metric_consumer".into());
    consumer.no_proc_execution = false;
    consumer.extern_artifacts.retain(|(name, _)| name == "std");
    consumer.extern_artifacts.push(("metric_derive".into(), artifact));
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: std::fs::read_to_string(fixtures.join("consumer.rk")).unwrap() }];
    rock_lib::compile(&consumer).unwrap_or_else(|errors| panic!("trait derive consumer: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let output = Command::new(consumer.output_dir.join("consumer")).output().unwrap();
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, b"0\n18\n18\n11\n19\n30\n");
    let expanded = rock_lib::expand(&consumer).unwrap();
    let packet = expanded.ast.module.top_levels.iter().find_map(|item| match item {
        rock_lib::ast::TopLevel::StructDecl(node) if node.name.name == "Packet" => Some(node), _ => None,
    }).unwrap();
    assert_eq!(packet.fields.iter().filter(|field| field.default.is_some()).count(), 2);
    for (name, body, expected) in [
        ("missing-concrete-impl", "struct Bare\n    < value: I64\n%derive SumMetric\nstruct Held\n    < value: Bare\nmain = !-> ()\n", vec!["metric"]),
        ("missing-generic-bound", "struct Bare\n    < value: I64\n%derive UnboundedMetric\nstruct Held T\n    < value: T\nmain = !->\n    held = Held\n        value: Bare\n            value: 7\n    held.metric!.println!\n", vec!["metric"]),
        ("private-construction", "%derive SumMetric\nstruct Hidden\n    value: I64\nmain = !->\n    hidden = Hidden\n        value: 7\n", vec!["private"]),
        ("private-access", "%derive SumMetric\nstruct Hidden\n    value: I64\nimpl Hidden\n    new = -> Hidden\n        value: 7\nmain = !->\n    hidden = Hidden::new!\n    hidden.value.println!\n", vec!["private"]),
        ("duplicate-impl", "%derive SumMetric\nstruct Held\n    < value: I64\nimpl Metric for Held\n    @metric = -> 99\nmain = !->\n    held = Held\n        value: 7\n    held.metric!.println!\n", vec!["duplicate", "conflict", "ambiguous", "overlap"]),
        ("overlapping-generic-impl", "%derive SumMetric\nstruct Held T\n    < value: T\nimpl Metric for Held I64\n    @metric = -> 99\nmain = !->\n    held = Held\n        value: 7\n    held.metric!.println!\n", vec!["duplicate", "conflict", "ambiguous", "overlap"]),
    ] {
        consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: format!("{PRELUDE}\n{body}") }];
        let errors = rock_lib::compile(&consumer).err().unwrap_or_else(|| panic!("{name}: unexpectedly compiled"));
        assert!(errors.0.iter().any(|error| expected.iter().any(|text| error.message.to_lowercase().contains(text))), "{name}: {errors:?}");
        assert!(errors.0.iter().all(|error| error.code != Some(DiagnosticCode::Macro)), "ordinary compiler error required for {name}: {errors:?}");
        eprintln!("struct derive: {name}: {:?}", errors.0.iter().map(|error| &error.message).collect::<Vec<_>>());
    }
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: format!("{PRELUDE}\n%derive SumMetric\nenum E\n    Empty\n") }];
    let errors = rock_lib::analyze(&consumer).unwrap_err();
    assert!(errors.0.iter().any(|error| error.code == Some(DiagnosticCode::Macro) && error.message.contains("supports structs only")), "{errors:?}");
}

pub fn verify_enums(config: &Config) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixtures = workspace.join("lib/tests/fixtures/macros/derive_metric");
    let (host, mut products) = build_provider(config);
    assert!(products.dependencies.is_empty() && products.bodies.functions.is_empty());
    let root = config.output_dir.parent().unwrap();
    let package = root.join("package");
    std::fs::create_dir_all(&package).unwrap();
    let descriptor = products.proc_macros[0].resolve_for_host(&host.output_dir.join("metric_derive.rkca")).unwrap();
    assert!(descriptor.required_files.is_empty());
    std::fs::copy(&descriptor.executable, package.join("worker")).unwrap();
    products.proc_macros[0].executable = "worker".into();
    products.write_artifact_to_path(&package.join("metric_derive.rkca")).unwrap();
    let relocated = root.join("relocated-package");
    std::fs::rename(&package, &relocated).unwrap();
    // Invalidate the exact build locations, without touching shared SDK/std
    // prerequisites or relying on a provider source tree in the consumer.
    std::fs::rename(&host.output_dir, root.join("retired-host")).unwrap();
    std::fs::rename(root.join("syntax"), root.join("retired-syntax")).unwrap();
    assert!(!descriptor.executable.exists() && !host.entry_file.exists());
    assert_eq!(std::fs::read_dir(&relocated).unwrap().count(), 2);
    let source = std::fs::read_to_string(fixtures.join("enum-consumer.rk")).unwrap().replace('\n', "\r\n");
    let mut consumer = config.clone();
    consumer.entry_file = root.join("enum-consumer.rk");
    consumer.output_dir = root.join("enum-consumer");
    consumer.current_crate_name = Some("enum_consumer".into());
    consumer.no_proc_execution = false;
    consumer.extern_artifacts.retain(|(name, _)| name == "std");
    consumer.extern_artifacts.push(("metric_derive".into(), relocated.join("metric_derive.rkca")));
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source.clone() }];
    rock_lib::compile(&consumer).unwrap_or_else(|errors| panic!("packaged enum consumer: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let output = Command::new(consumer.output_dir.join("enum-consumer")).output().unwrap();
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, b"0\n3\n12\n24\n0\n7\n");
    let expanded = rock_lib::expand(&consumer).unwrap();
    let declaration = expanded.ast.module.top_levels.iter().find_map(|item| match item {
        rock_lib::ast::TopLevel::EnumDecl(node) if node.name.name == "Event" => Some(node), _ => None,
    }).unwrap();
    let original: String = declaration.original.as_ref().unwrap().stream().unwrap().runs().iter().map(|run| run.text()).collect();
    assert!(original.contains("< enum Event T, {U = T}\r\n"));
    assert!(original.contains("< other: U = 99\r\n"));
    assert!(original.contains("// Retain Unicode trivia through the original CRLF transport: 雪\r\n"));
    let records = (1..=4).map(|index| expanded.source_map.record(rock_lib::macro_expansion::ExpansionId(index)).unwrap()).collect::<Vec<_>>();
    assert_eq!(records.iter().map(|record| record.macro_name.as_str()).collect::<Vec<_>>(), ["EnumMetric", "Cascade", "After", "EnumMetric"]);
    assert_eq!(records[3].parent, Some(rock_lib::macro_expansion::ExpansionId(2)));
    for (name, body, expected, macro_error) in [
        ("missing-enum-field-impl", "struct Bare\n    < value: I64\n%derive EnumMetric\nenum E\n    One Bare\nmain = !-> ()\n", "does not implement", false),
        ("duplicate-enum-impl", "%derive EnumMetric\nenum E\n    Empty\nimpl Metric for E\n    @metric = -> 1\nmain = !-> ()\n", "overlapping", false),
        ("wrong-target", "%derive EnumMetric\nstruct S\n", "supports enums only", true),
        ("wrong-category", "%derive WrongCategory\nstruct S\n", "", true),
        ("wrong-kind", "%derive ordinary\nstruct S\n", "DeriveLike", true),
        ("duplicate-attachment", "%derive EnumMetric, metric_derive::EnumMetric\nenum E\n    Empty\n", "duplicate canonical", true),
        ("recursive-limit", "%derive Loop\nstruct S\n", "depth exceeded", true),
    ] {
        let prefix = format!("{PRELUDE}> metric_derive::EnumMetric\n> metric_derive::WrongCategory\n> metric_derive::ordinary\n> metric_derive::Loop\n");
        consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: format!("{prefix}{body}") }];
        let errors = rock_lib::compile(&consumer).err().unwrap_or_else(|| panic!("{name}: unexpectedly compiled"));
        assert!(errors.0.iter().any(|error| error.message.contains(expected)), "{name}: {errors:?}");
        assert_eq!(errors.0.iter().any(|error| error.code == Some(DiagnosticCode::Macro)), macro_error, "{name}: {errors:?}");
        eprintln!("packaged enum derive: {name} rejected");
    }
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: "#[derive(EnumMetric)]\nenum E\n    Empty\n".into() }];
    assert!(rock_lib::expand(&consumer).is_err(), "attribute transforms are not supported");
}
