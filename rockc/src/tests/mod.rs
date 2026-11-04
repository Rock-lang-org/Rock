use super::artifact_validation::resolve_artifact_object_path;
use super::source_commands::{load_program_for_source_command, load_program_with_source_database};
use super::{print_request_output, run_config, Config};
use clap::Parser;
use std::collections::BTreeMap;
use std::path::PathBuf;

use rock_lib::products::{
    CompilerProducts, ProductCrateId, ProductCrateIdentity, ProductDependencyIdentity,
    ProductIdentityTable, ProductLinkData,
};

mod artifact_consume;
mod artifact_deps;
mod artifact_emit;
mod artifact_validation;
mod cli_args;
mod format;
mod source_commands;

fn product_artifact(
    name: &str,
    dependencies: Vec<ProductDependencyIdentity>,
    identity_table: ProductIdentityTable,
) -> CompilerProducts {
    CompilerProducts {
        crate_identity: ProductCrateIdentity::local(name.to_string()),
        identity_table,
        interface: rock_lib::products::ProductInterface::default(),
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(PathBuf::from(format!("{}.o", name))),
            records: Default::default(),
        },
        dependencies,
        source_fingerprint: Default::default(),
        proc_macros: Vec::new(),
        infix_precedence: Default::default(),
    }
}

fn artifact_deps_test_dir(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "rockc_artifact_deps_{}_{}",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn identity_table_with_dependencies<const N: usize>(
    dependencies: [(&str, ProductCrateId); N],
) -> ProductIdentityTable {
    ProductIdentityTable {
        dependencies: dependencies
            .into_iter()
            .map(|(name, id)| (id, ProductCrateIdentity::local(name.to_string())))
            .collect(),
        ..ProductIdentityTable::default()
    }
}

fn artifact_deps_config<const N: usize>(artifacts: [(&str, &std::path::Path); N]) -> Config {
    let mut args = vec![
        "rockc".to_string(),
        "--print".to_string(),
        "artifact-deps".to_string(),
    ];
    for (name, path) in artifacts {
        args.push("--extern-artifact".to_string());
        args.push(format!("{}={}", name, path.display()));
    }

    Config::try_parse_from(args).unwrap()
}

fn diagnostic_messages(diagnostics: rock_lib::diagnostic::Diagnostics) -> String {
    diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
