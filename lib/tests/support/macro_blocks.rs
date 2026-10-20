use rock_lib::crate_system::TargetRole;
use rock_lib::macro_expansion::host::ProcMacroSdk;
use rock_lib::{Config, SourceProvider};

pub fn verify(config: &Config) {
    let mut host = config.clone();
    host.current_crate_name = Some("block_dsl".into());
    host.target_role = TargetRole::ProcMacro;
    host.no_prelude = false;
    host.output_dir = config.output_dir.parent().unwrap().join("block-host");
    host.source_providers = vec![SourceProvider::Virtual { path: host.entry_file.clone(),
        text: include_str!("../fixtures/macros/block_dsl/src/lib.rk").into() }];
    let output = rock_lib::compile_proc_macro_with_products(&host, &ProcMacroSdk { dependency: "rock_macro".into() })
        .unwrap_or_else(|errors| panic!("block DSL provider: {errors:?}"));
    let mut products = output.products.unwrap();
    products.proc_macros[0].executable = "main".into();
    let artifact = host.output_dir.join("block_dsl.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let entry_file = host.output_dir.join("consumer.rk");
    let mut consumer = Config {
        entry_file, output_dir: host.output_dir.join("consumer"), no_std: true, no_prelude: true,
        extern_artifacts: vec![("block_dsl".into(), artifact)], ..Default::default()
    };
    for (index, source) in [
        include_str!("../fixtures/macros/block_dsl/expression.rk"),
        include_str!("../fixtures/macros/block_dsl/items.rk"),
        include_str!("../fixtures/macros/block_dsl/forward.rk"),
    ].into_iter().enumerate() {
        let parsed = rock_lib::parser::parse_string(source, &consumer).unwrap();
        assert!(parsed.module.top_levels.iter().any(|item| matches!(item,
            rock_lib::ast::TopLevel::FunctionDecl(function) if function.name.name == "after")), "following dedented binding was captured");
        if index < 2 {
            struct Calls(Vec<rock_lib::ast::MacroInvoc>);
            impl<'a> rock_lib::ast::visit::Visitor<'a> for Calls {
                fn visit_macro_invoc(&mut self, invocation: &'a rock_lib::ast::MacroInvoc) { self.0.push(invocation.clone()); }
            }
            let mut calls = Calls(Vec::new());
            rock_lib::ast::visit::walk_module(&mut calls, &parsed.module);
            let invocation = &calls.0[0];
            let marker = if index == 0 { "%render\n" } else { "%declare\n" };
            let expected = source.split_once(marker).unwrap().1.split_once("after =").unwrap().0;
            let retained: String = invocation.args.runs().iter().map(|run| run.text()).collect();
            assert_eq!(retained, expected);
            assert!(invocation.literal_ranges.is_empty() && invocation.block.is_some());
        }
        consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source.into() }];
        rock_lib::compile(&consumer).unwrap_or_else(|errors| panic!("block DSL case {index}: {errors:?}"));
        let result = super::run_client(&consumer.output_dir.join("consumer"));
        assert_eq!(result.status.code(), Some(42), "case {index}: {result:?}");
        assert!(result.stdout.is_empty() && result.stderr.is_empty());
    }
    let source = include_str!("../fixtures/macros/block_dsl/expression.rk").replace("value=42", "value=17");
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source }];
    rock_lib::compile(&consumer).unwrap_or_else(|errors| panic!("alternate library value: {errors:?}"));
    assert_eq!(super::run_client(&consumer.output_dir.join("consumer")).status.code(), Some(17));
    for (attribute, expected) in [
        ("value=123", "fixture value has at most two digits"),
        ("value=no", "expected decimal digits after '='"),
        ("no-value", "block fixture requires a decimal value attribute"),
    ] {
        let source = include_str!("../fixtures/macros/block_dsl/expression.rk").replace("value=42", attribute);
        consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source }];
        let errors = rock_lib::analyze(&consumer).unwrap_err();
        assert!(errors.0.iter().any(|error| error.message == format!("Proc macro 'render': {expected}")), "{errors:?}");
    }
    let source = include_str!("../fixtures/macros/block_dsl/error.rk");
    let offset = source.find('@').unwrap();
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source.into() }];
    let errors = rock_lib::analyze(&consumer).unwrap_err();
    assert!(errors.0.iter().any(|error| error.message.contains("block fixture forbids '@'") && error.primary.as_ref().is_some_and(|label|
        label.span.file_path == consumer.entry_file && label.span.start == offset && label.span.end == offset + 1)), "{errors:?}");
    let raw = include_str!("../fixtures/macros/block_dsl/expression.rk").replace("render", "raw");
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: raw }];
    let errors = rock_lib::analyze(&consumer).unwrap_err();
    assert!(errors.0.iter().any(|error| error.code == Some(rock_lib::diagnostic::DiagnosticCode::Macro)), "{errors:?}");
    assert!(errors.0.iter().any(|error| error.message.contains("Unexpected")), "{errors:?}");
    assert!(!format!("{errors:?}").contains("not implemented"));
    eprintln!("SDK acceptance: B.20 external block DSL expression/item/forwarding/diagnostic/raw-output controls passed");
}
