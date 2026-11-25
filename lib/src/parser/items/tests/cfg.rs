use std::path::PathBuf;

use crate::ast::TopLevel;
use crate::parser::{parse_source, ParseError};
use crate::Config;

fn config_for(triple: &str) -> Config {
    Config {
        target: Some(triple.to_string()),
        ..Config::default()
    }
}

fn parse_for(triple: &str, source: &str) -> Result<Vec<TopLevel>, ParseError> {
    parse_source(PathBuf::from("cfg.rk"), source, &config_for(triple)).map(|m| m.top_levels)
}

fn item_names(items: &[TopLevel]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            TopLevel::Mod(ident, _) => format!("mod {}", ident.name),
            other => other.get_ident().cloned().unwrap_or_default(),
        })
        .collect()
}

fn hard_error(triple: &str, source: &str) -> String {
    match parse_for(triple, source) {
        Err(ParseError::HardError(message, _)) => message,
        other => panic!("expected a hard cfg error, got {other:?}"),
    }
}

const BACKENDS: &str = "\
before = -> 0
cfg(target_family = \"unix\")
    mod posix
    unix_only = -> 1
cfg(target_os = \"windows\")
    mod windows
after = -> 2
";

#[test]
fn cfg_block_splices_active_items_in_source_order() {
    let linux = parse_for("x86_64-unknown-linux-gnu", BACKENDS).unwrap();
    assert_eq!(
        item_names(&linux),
        ["before", "mod posix", "unix_only", "after"]
    );

    let windows = parse_for("x86_64-pc-windows-msvc", BACKENDS).unwrap();
    assert_eq!(item_names(&windows), ["before", "mod windows", "after"]);
}

#[test]
fn cfg_combinators_and_nesting_compose() {
    let source = "\
cfg(all(target_family = \"unix\", not(target_os = \"macos\")))
    linux_like = -> 1
    cfg(any(target_arch = \"aarch64\", target_env = \"gnu\"))
        nested = -> 2
";
    let linux = parse_for("x86_64-unknown-linux-gnu", source).unwrap();
    assert_eq!(item_names(&linux), ["linux_like", "nested"]);

    let mac = parse_for("aarch64-apple-darwin", source).unwrap();
    assert!(mac.is_empty());
}

#[test]
fn cfg_rejects_unknown_keys_and_values_on_every_target() {
    let source = "cfg(target_os = \"linx\")\n    f = -> 1\n";
    assert!(hard_error("x86_64-unknown-linux-gnu", source).contains("unknown value \"linx\""));

    let source = "cfg(target_oss = \"linux\")\n    f = -> 1\n";
    assert!(hard_error("x86_64-pc-windows-msvc", source).contains("unknown cfg key 'target_oss'"));
}

#[test]
fn cfg_rejects_bad_arity_and_empty_blocks() {
    let source = "cfg(not(target_os = \"linux\", target_os = \"macos\"))\n    f = -> 1\n";
    assert!(hard_error("x86_64-unknown-linux-gnu", source).contains("exactly one operand"));

    let source = "cfg(any())\n    f = -> 1\n";
    assert!(hard_error("x86_64-unknown-linux-gnu", source).contains("at least one operand"));

    let source = "cfg(target_os = \"linux\")\nf = -> 1\n";
    assert!(hard_error("x86_64-unknown-linux-gnu", source).contains("at least one indented"));
}

#[test]
fn cfg_name_stays_usable_as_an_identifier() {
    let items = parse_for("x86_64-unknown-linux-gnu", "cfg = -> 1\n").unwrap();
    assert_eq!(item_names(&items), ["cfg"]);
}
