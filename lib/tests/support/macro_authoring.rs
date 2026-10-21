use rock_lib::crate_system::TargetRole;
use rock_lib::macro_expansion::host::ProcMacroSdk;
use rock_lib::{Config, SourceProvider};

pub fn verify(config: &Config) {
    let mut host = config.clone();
    host.current_crate_name = Some("authoring_provider".into());
    host.target_role = TargetRole::ProcMacro;
    host.output_dir = config.output_dir.parent().unwrap().join("host");
    host.source_providers = vec![SourceProvider::Virtual { path: host.entry_file.clone(), text: format!("{}\n{}",
        include_str!("../fixtures/macros/sdk-format.rk"), include_str!("../fixtures/macros/sdk-authoring.rk")) }];
    let output = rock_lib::compile_proc_macro_with_products(&host, &ProcMacroSdk { dependency: "rock_macro".into() })
        .unwrap_or_else(|errors| panic!("authoring provider: {errors:?}"));
    let artifact = host.output_dir.join("authoring_provider.rkca");
    output.products.unwrap().write_artifact_to_path(&artifact).unwrap();
    let mut consumer = config.clone();
    consumer.entry_file = config.output_dir.parent().unwrap().join("consumer.rk");
    consumer.output_dir = config.output_dir.parent().unwrap().join("consumer");
    consumer.current_crate_name = Some("authoring_consumer".into());
    consumer.no_proc_execution = false;
    consumer.extern_artifacts.retain(|(name, _)| name == "std");
    consumer.extern_artifacts.push(("authoring_provider".into(), artifact));
    let source = r#"
> authoring_provider::format
> authoring_provider::string
> authoring_provider::checks
> authoring_provider::forward
> authoring_provider::twice
> authoring_provider::scoped
> authoring_provider::greeting
> authoring_provider::mixed_scope
make = ->
    "once".println!
    7
main = !->
    name = String::from "Ada 雪"
    count = 3
    (%format "Hello {name}! {count} items; {name}.").println!
    (%format "雪🦀 {name} / {count}").println!
    name.println!
    (%greeting name).println!
    (%format "plain").println!
    (%format "").println!
    (%format "{make!}").println!
    (%string "a\"b\\c\n\r\t\0雪🦀").println!
    (%checks "雪").println!
    value = 21
    (%forward value).println!
    (%twice value).println!
    (%scoped value).println!
    (%mixed_scope!).println!
"#;
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source.into() }];
    rock_lib::compile(&consumer).unwrap_or_else(|errors| panic!("authoring consumer: {errors:?}"));
    let output = super::run_client(&consumer.output_dir.join("consumer"));
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, "Hello Ada 雪! 3 items; Ada 雪.\n雪🦀 Ada 雪 / 3\nAda 雪\nHello Ada 雪\nplain\n\nonce\n7\na\"b\\c\n\r\t\0雪🦀\n42\n21\n42\n21\n1\n".as_bytes());
    for (call, expected) in [
        ("%format!", "expected one string literal"),
        ("%format 'a'", "not a character literal"),
        ("%format 42", "expected one string literal"),
        ("%format \"x\", \"y\"", "expected one string literal"),
        ("%format \"x\", 42", "expected only a string literal"),
        ("%format \"{}\"", "needs a name"),
        ("%format \"{name\"", "unclosed"),
        ("%format \"name}\"", "unmatched"),
        ("%format \"{{name}}\"", "nested"),
        ("%format \"雪🦀{}\"", "needs a name"),
        ("%format \"雪🦀{name\"", "unclosed"),
        ("%format \"雪🦀}\"", "unmatched"),
        ("%format \"雪🦀{{name}}\"", "nested"),
        ("%propagation!", "invalid literal range"),
    ] {
        consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(),
            text: format!("> authoring_provider::format\n> authoring_provider::propagation\nmain = !->\n    result = {call}\n") }];
        let errors = rock_lib::analyze(&consumer).unwrap_err();
        assert!(errors.0.iter().any(|error| error.message.contains(expected)), "{call}: {errors:?}");
    }
    let source = "> authoring_provider::format\nmain = !->\n    result = %format \"é\\n{missing}\"\n";
    let missing = source.find("missing").unwrap();
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source.into() }];
    let errors = rock_lib::analyze(&consumer).unwrap_err();
    assert!(errors.0.iter().any(|error| error.primary.as_ref().is_some_and(|label|
        label.span.file_path == consumer.entry_file && label.span.start == missing && label.span.end == missing + 7)), "{errors:?}");
}
