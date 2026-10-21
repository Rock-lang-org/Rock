use std::path::Path;
use std::process::Command;

use rock_lib::crate_system::TargetRole;
use rock_lib::diagnostic::DiagnosticCode;
use rock_lib::macro_expansion::host::ProcMacroSdk;
use rock_lib::{Config, SourceProvider};

const PRELUDE: &str = "> metric_derive::SumMetric\n> metric_derive::UnboundedMetric\ntrait Metric\n    @metric: I64\nimpl Metric for I64\n    @metric = -> *self\n";

pub fn verify(config: &Config) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixtures = workspace.join("lib/tests/fixtures/macros/derive_metric");
    let syntax = super::macro_syntax::build_syntax(config);
    let mut host = config.clone();
    host.current_crate_name = Some("metric_derive".into());
    host.target_role = TargetRole::ProcMacro;
    host.output_dir = config.output_dir.parent().unwrap().join("host");
    host.extern_artifacts.push(("rock_syntax".into(), syntax));
    host.source_providers = vec![SourceProvider::Virtual { path: host.entry_file.clone(), text: std::fs::read_to_string(fixtures.join("provider.rk")).unwrap() }];
    let built = rock_lib::compile_proc_macro_with_products(&host, &ProcMacroSdk { dependency: "rock_macro".into() })
        .unwrap_or_else(|errors| panic!("trait derive provider: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let artifact = host.output_dir.join("metric_derive.rkca");
    built.products.unwrap().write_artifact_to_path(&artifact).unwrap();
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
