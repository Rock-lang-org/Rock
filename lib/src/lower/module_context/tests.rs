use super::*;

use crate::ast::{Ident, IdentOrType, IdentifierPath, Module, Path, TopLevel};
use crate::collect::item_index::{index_root_module_items, IndexingIds};
use crate::ids::{CrateId, DefId, LocalDefId, ModuleId};
use crate::lexer::Span;
use crate::lower::services::LowerModuleService;
use crate::lower::Lowerer;
use crate::source_loader::LoadedModule;
use crate::types::Type;

fn empty_module(path: &std::path::Path) -> Module {
    Module {
        name: Some(Ident {
            name: "io".to_string(),
            span: Span::test(),
        }),
        top_levels: Vec::new(),
        is_inline: false,
        filepath: Some(path.to_path_buf()),
    }
}

fn import_path(names: &[&str]) -> TopLevel {
    TopLevel::Import(Path::Ident(IdentifierPath {
        path: names
            .iter()
            .map(|name| {
                IdentOrType::Ident(Ident {
                    name: name.to_string(),
                    span: Span::test(),
                })
            })
            .collect(),
    }))
}

fn loaded_module(qualified_name: &str, path: PathBuf) -> LoadedModule {
    LoadedModule {
        id: crate::source_loader::ModuleId(0),
        qualified_name: qualified_name.to_string(),
        path: path.clone(),
        canonical_path: path.clone(),
        module: empty_module(&path),
    }
}

#[test]
fn source_module_resolver_loads_modules_from_loaded_module_entries() {
    let root_path = PathBuf::from("/virtual/demo/main.rk");
    let local_path = PathBuf::from("/virtual/demo/foo.rk");
    let dep_root_path = PathBuf::from("/virtual/dep/lib.rk");
    let dep_util_path = PathBuf::from("/virtual/dep/util.rk");

    let mut modules = LowerModuleService::from_source_modules(vec![
        loaded_module("demo", root_path.clone()),
        loaded_module("demo::foo", local_path.clone()),
        loaded_module("dep", dep_root_path),
        loaded_module("dep::util", dep_util_path.clone()),
    ]);
    modules.configure_current_crate(Some("demo"), Some(&root_path));

    let (local, local_prefix) = SourceModuleResolver::new(&mut modules)
        .load_module_by_path(&["foo".to_string()])
        .expect("loaded local module entry should resolve");
    assert_eq!(local.filepath.as_ref(), Some(&local_path));
    assert_eq!(local_prefix, "demo::foo");

    let (external, external_prefix) = SourceModuleResolver::new(&mut modules)
        .load_module_by_path(&["dep".to_string(), "util".to_string()])
        .expect("loaded external module entry should resolve");
    assert_eq!(external.filepath.as_ref(), Some(&dep_util_path));
    assert_eq!(external_prefix, "dep::util");
}

#[test]
fn source_module_resolver_loads_cached_local_and_external_modules_from_context() {
    let root_path = PathBuf::from("/virtual/demo/main.rk");
    let local_path = PathBuf::from("/virtual/demo/foo.rk");
    let dep_root_path = PathBuf::from("/virtual/dep/lib.rk");
    let dep_util_path = PathBuf::from("/virtual/dep/util.rk");

    let mut modules = LowerModuleService::from_source_modules(vec![
        loaded_module("demo", root_path.clone()),
        loaded_module("demo::foo", local_path.clone()),
        loaded_module("dep", dep_root_path),
        loaded_module("dep::util", dep_util_path.clone()),
    ]);
    modules.configure_current_crate(Some("demo"), Some(&root_path));

    let (local, local_prefix) = SourceModuleResolver::new(&mut modules)
        .load_module_by_path(&["foo".to_string()])
        .expect("cached local module should resolve");
    assert_eq!(local.filepath.as_ref(), Some(&local_path));
    assert_eq!(local_prefix, "demo::foo");

    let (external, external_prefix) = SourceModuleResolver::new(&mut modules)
        .load_module_by_path(&["dep".to_string(), "util".to_string()])
        .expect("cached external module should resolve from seeded paths");
    assert_eq!(external.filepath.as_ref(), Some(&dep_util_path));
    assert_eq!(external_prefix, "dep::util");
}

#[test]
fn lowerer_exposes_module_backing_state_through_module_service() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_module_context_service_state_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let root_path = temp_dir.join("main.rk");
    let util_path = temp_dir.join("util.rk");

    let mut lowerer = Lowerer::new_for_test();
    lowerer.modules = LowerModuleService::from_source_modules(vec![loaded_module(
        "demo::util",
        util_path.clone(),
    )]);
    lowerer
        .modules
        .configure_current_crate(Some("demo"), Some(&root_path));

    assert_eq!(lowerer.modules.current_crate_name(), Some("demo"));
    assert_eq!(lowerer.modules.root_file_path(), Some(root_path.as_path()));
    assert_eq!(lowerer.modules.current_module_path(), root_path.as_path());
    assert_eq!(
        lowerer.modules.current_module_prefix(),
        Some("demo".to_string())
    );
    assert_eq!(
        lowerer.modules.source_path_for_module_name("util"),
        Some(util_path.clone())
    );
    assert!(lowerer
        .modules
        .source_module_for_qualified_name("util")
        .is_some());

    lowerer
        .modules
        .with_current_module_path(util_path.clone(), |modules| {
            assert_eq!(modules.current_module_path(), util_path.as_path());
        });

    assert_eq!(lowerer.modules.current_module_path(), root_path.as_path());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn module_context_prefers_current_crate_prefixed_graph_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_module_context_prefers_graph_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let alias_path = temp_dir.join("alias.rk");
    let graph_path = temp_dir.join("graph.rk");

    let mut lowerer = Lowerer::new_for_test();
    lowerer.modules = LowerModuleService::from_source_modules(vec![
        loaded_module("math::io", alias_path.clone()),
        loaded_module("test::math::io", graph_path.clone()),
    ]);
    lowerer
        .modules
        .set_current_crate_name(Some("test".to_string()));

    let path = lowerer
        .modules
        .source_path_for_module_name("math::io")
        .expect("graph path should resolve");

    assert_eq!(path, graph_path);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn module_context_does_not_use_cache_without_loaded_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_module_context_requires_graph_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let cached_path = temp_dir.join("util.rk");

    let mut lowerer = Lowerer::new_for_test();
    lowerer.modules = LowerModuleService::from_source_modules(vec![loaded_module(
        "other::util",
        cached_path.clone(),
    )]);

    assert!(lowerer
        .modules
        .source_path_for_module_name("util")
        .is_none());
    assert!(lowerer
        .modules
        .source_module_for_qualified_name("util")
        .is_none());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn module_context_skips_only_root_file_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_module_context_skips_root_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let root_path = temp_dir.join("main.rk");
    let module_path = temp_dir.join("util.rk");

    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .modules
        .configure_current_crate(Some("demo"), Some(&root_path));

    assert!(lowerer.modules.should_skip_loaded_module_path(&root_path));
    assert!(!lowerer.modules.should_skip_loaded_module_path(&module_path));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn module_context_iterates_loaded_modules_from_cache_and_skips_root() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_module_context_iter_loaded_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let root_path = temp_dir.join("main.rk");
    let child_path = temp_dir.join("child.rk");
    let child = empty_module(&child_path);

    let root = Module {
        name: None,
        top_levels: vec![TopLevel::Mod(
            Ident {
                name: "child".to_string(),
                span: Span::test(),
            },
            false,
        )],
        is_inline: false,
        filepath: Some(root_path.clone()),
    };
    let mut ids = IndexingIds::new_root();
    let mut lowerer = Lowerer::new_for_test();
    lowerer.item_index = index_root_module_items(&mut ids, &root);
    lowerer.modules = LowerModuleService::from_source_modules(vec![
        loaded_module("demo", root_path.clone()),
        LoadedModule {
            module: child,
            ..loaded_module("demo::child", child_path.clone())
        },
    ]);
    lowerer
        .modules
        .configure_current_crate(Some("demo"), Some(&root_path));
    lowerer
        .modules
        .associate_current_crate_module_ids(&lowerer.item_index);

    let mut seen = Vec::new();
    ModuleLoweringContext::for_each_loaded_module(
        &mut lowerer,
        |_, module_id, module_name, module| {
            seen.push((module_id, module_name.to_string(), module.filepath.clone()));
        },
    );

    assert_eq!(
        seen,
        vec![(ModuleId(1), "demo::child".to_string(), Some(child_path),)]
    );
    assert!(
        lowerer.errors().is_empty(),
        "unexpected errors: {:?}",
        lowerer.errors()
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn module_context_owns_loaded_root_name_lookup() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_module_context_loaded_root_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let root_path = temp_dir.join("lib.rk");

    let mut lowerer = Lowerer::new_for_test();
    lowerer.modules =
        LowerModuleService::from_source_modules(vec![loaded_module("dep", root_path.clone())]);

    assert!(lowerer.modules.has_loaded_root_name("dep"));
    assert_eq!(lowerer.modules.source_root_path("dep"), Some(root_path));
    assert!(!lowerer.modules.has_loaded_root_name("missing"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn module_context_scopes_module_local_aliases_and_prefix() {
    let parsed =
        crate::parser::parse_string("answer: I64\nanswer = -> 1\n", &crate::Config::default())
            .unwrap();
    let module = parsed.module;

    let mut lowerer = Lowerer::new_for_test();
    let answer_id = DefId::new(CrateId(0), LocalDefId(1));
    lowerer
        .resolver
        .item_paths
        .insert("demo::helper::answer".to_string(), answer_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(answer_id, "demo::helper::answer".to_string());
    lowerer
        .resolver
        .scoped_module_aliases
        .entry("demo::helper".to_string())
        .or_default()
        .insert("answer".to_string(), answer_id);
    lowerer.scope.define(
        "demo::helper::answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );
    lowerer
        .modules
        .set_current_qualified_module_prefix(Some("old".to_string()));

    ModuleLoweringContext::with_qualified_module_context(
        &mut lowerer,
        &module,
        Some("demo::helper"),
        |lowerer| {
            assert_eq!(
                lowerer.modules.current_module_prefix(),
                Some("demo::helper".to_string())
            );
            assert_eq!(
                crate::lower::resolution::LowerResolutionContext::new(lowerer)
                    .resolve_module_alias_or_item_id("answer"),
                Some(answer_id)
            );
            assert!(!lowerer.resolver.module_aliases.contains_key("answer"));
            assert!(lowerer.scope.lookup("answer").is_some());
        },
    );

    assert_eq!(
        lowerer.modules.current_module_prefix(),
        Some("old".to_string())
    );
    assert!(!lowerer.resolver.module_aliases.contains_key("answer"));
    assert!(lowerer.scope.lookup("answer").is_none());
}

#[test]
fn module_context_duplicate_module_aliases_use_scoped_resolver_output() {
    let module = Module {
        name: Some(Ident {
            name: "helper".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![
            import_path(&["demo", "helper", "answer"]),
            import_path(&["demo", "helper", "answer"]),
        ],
        is_inline: false,
        filepath: None,
    };

    let previous_id = DefId::new(CrateId(0), LocalDefId(1));
    let answer_id = DefId::new(CrateId(0), LocalDefId(2));
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .resolver
        .module_aliases
        .insert("answer".to_string(), previous_id);
    lowerer
        .resolver
        .item_paths
        .insert("demo::helper::answer".to_string(), answer_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(answer_id, "demo::helper::answer".to_string());
    lowerer
        .resolver
        .scoped_module_aliases
        .entry("demo::helper".to_string())
        .or_default()
        .insert("answer".to_string(), answer_id);
    lowerer.scope.define(
        "demo::helper::answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );
    lowerer
        .modules
        .set_current_qualified_module_prefix(Some("demo::helper".to_string()));

    ModuleLoweringContext::with_module_local_aliases(
        &mut lowerer,
        &module,
        "demo::helper",
        true,
        true,
        |lowerer| {
            assert_eq!(
                crate::lower::resolution::LowerResolutionContext::new(lowerer)
                    .resolve_module_alias_or_item_id("answer"),
                Some(answer_id)
            );
            assert_eq!(
                lowerer.resolver.module_aliases.get("answer").copied(),
                Some(previous_id)
            );
        },
    );

    assert_eq!(
        lowerer.resolver.module_aliases.get("answer").copied(),
        Some(previous_id)
    );
}

impl ModuleLoweringContext {
    pub(crate) fn set_current_qualified_module_prefix(&mut self, prefix: Option<String>) {
        self.current_qualified_module_prefix = prefix;
    }

    pub(crate) fn set_current_crate_name(&mut self, crate_name: Option<String>) {
        self.current_crate_name = crate_name;
    }

    pub(crate) fn current_module_path(&self) -> &std::path::Path {
        self.current_module_path.as_path()
    }

    pub(crate) fn with_current_module_path<F, R>(&mut self, path: PathBuf, visit: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        let previous_path = self.replace_current_module_path(path);
        let result = visit(self);
        self.current_module_path = previous_path;
        result
    }
}
