use super::*;

use crate::ast::{Ident, Module};
use crate::lexer::Span;
use crate::source_loader::LoadedModule;

fn loaded_module(qualified_name: &str, path: std::path::PathBuf, module: Module) -> LoadedModule {
    LoadedModule {
        id: crate::source_loader::ModuleId(0),
        qualified_name: qualified_name.to_string(),
        path: path.clone(),
        canonical_path: path,
        module,
    }
}

#[test]
fn source_module_resolver_requires_seeded_module_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_requires_seeded_module_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let root = temp_dir.join("main.rk");

    let mut lowerer = Lowerer::new_for_test();
    lowerer.modules.set_current_module_path(root);

    let error = SourceModuleResolver::new(&mut lowerer.modules)
        .load_local_module("util")
        .expect_err("module cache should not bypass graph-seeded module paths");

    assert!(error.contains("was not loaded by the source database"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn source_module_resolver_prefers_current_crate_prefixed_graph_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_prefers_current_crate_graph_path_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let alias_io = temp_dir.join("alias_io.rk");
    let graph_io = temp_dir.join("graph_io.rk");

    let alias_module = Module {
        name: Some(Ident {
            name: "io".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![],
        is_inline: false,
        filepath: Some(alias_io.clone()),
    };
    let graph_module = Module {
        name: Some(Ident {
            name: "io".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![],
        is_inline: false,
        filepath: Some(graph_io.clone()),
    };
    let mut lowerer = Lowerer::new_for_test();
    lowerer.modules = crate::lower::services::LowerModuleService::from_source_modules(vec![
        loaded_module("math::io", alias_io.clone(), alias_module),
        loaded_module("test::math::io", graph_io.clone(), graph_module),
    ]);
    lowerer
        .modules
        .set_current_crate_name(Some("test".to_string()));
    lowerer
        .modules
        .set_current_qualified_module_prefix(Some("math".to_string()));

    let module = SourceModuleResolver::new(&mut lowerer.modules)
        .load_local_module("io")
        .expect("current-crate-prefixed graph path should resolve");

    assert_eq!(module.filepath.as_ref(), Some(&graph_io));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn source_module_resolver_requires_seeded_external_child_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_requires_seeded_external_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let dep_root = temp_dir.join("lib.rk");
    let util = temp_dir.join("util.rk");

    let module = Module {
        name: Some(Ident {
            name: "util".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![],
        is_inline: false,
        filepath: Some(util.clone()),
    };
    let mut lowerer = Lowerer::new_for_test();
    lowerer.modules =
        crate::lower::services::LowerModuleService::from_source_modules(vec![loaded_module(
            "dep", dep_root, module,
        )]);

    let error = SourceModuleResolver::new(&mut lowerer.modules)
        .load_external_crate_module("util", "dep")
        .expect_err("external module cache should not bypass graph-seeded module paths");

    assert!(error.contains("was not loaded by the source database"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn public_lower_entrypoint_uses_indexed_current_def_ids() {
    let parsed = crate::parser::parse_string(
        "trait Show\n    @show: I64\n\nimpl Show for I64\n    @show = -> 1\n\nmain = -> 0\n",
        &crate::Config::default(),
    )
    .expect("program should parse");

    let partial = lower(&parsed).expect("public lower entrypoint should lower through collect");
    let show_id = partial.resolver.item_paths["Show"];
    let trait_sig_id = partial
        .traits
        .get(&show_id)
        .and_then(|trait_def| trait_def.signatures.get("show"))
        .map(|sig| sig.id)
        .expect("trait signature should have a DefId");
    let impl_method_id = partial
        .impls
        .values()
        .find_map(|imp| imp.methods.get("show"))
        .map(|method| method.id)
        .expect("impl method should have a DefId");

    assert!(partial.current_def_ids.contains(&trait_sig_id));
    assert!(partial.current_def_ids.contains(&impl_method_id));
}
