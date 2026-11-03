use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::{Ident, Module, ModuleDecl, TopLevel};
use crate::source_loader::{
    LoadedModule, ModuleGraph, ModuleId, SourceDatabase, SourceLoadError, SourceModuleSet,
    SourceOrigin,
};
use crate::Config;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rock_source_loader_{}_{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_std_config(entry_file: PathBuf) -> Config {
    Config {
        entry_file,
        no_std: true,
        no_prelude: true,
        ..Config::default()
    }
}

#[test]
fn virtual_sources_load_entry_and_sibling_modules_without_filesystem_paths() {
    let entry = PathBuf::from("/virtual/app/main.rk");
    let util = PathBuf::from("/virtual/app/util.rk");

    let mut db = SourceDatabase::new();
    db.add_virtual_source(entry.clone(), "mod util\nmain = -> util::answer!\n");
    db.add_virtual_source(util.clone(), "answer = -> 1\n< answer\n");

    let graph = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect("virtual entry should load without filesystem access");

    assert_eq!(graph.loaded_files(), &[entry.clone(), util.clone()]);
    assert!(graph.module_by_qualified_name("util").is_some());
    assert!(db.module_for_path(&util).is_some());
}

#[test]
fn module_graph_name_and_path_indexes_resolve_to_same_module_id() {
    let entry = PathBuf::from("/virtual/app/main.rk");
    let util = PathBuf::from("/virtual/app/util.rk");

    let mut db = SourceDatabase::new();
    db.add_virtual_source(entry.clone(), "mod util\nmain = -> util::answer!\n");
    db.add_virtual_source(util.clone(), "answer = -> 1\n< answer\n");

    let graph = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect("virtual entry should load without filesystem access");

    let by_name = graph
        .module_by_qualified_name("util")
        .expect("name lookup should find util");
    let by_path = graph
        .module_for_path(&util)
        .expect("path lookup should find util");

    assert_eq!(by_name.id, by_path.id);
    assert_eq!(
        graph
            .module(by_name.id)
            .map(|module| &module.qualified_name),
        Some(&"util".to_string())
    );
}

#[test]
fn module_graph_distinguishes_same_named_modules_under_different_parents() {
    let dir = temp_dir("same_named_nested_modules");
    let entry = dir.join("main.rk");
    fs::write(&entry, "mod left\nmod right\nmain = -> 0\n").unwrap();
    fs::create_dir_all(dir.join("left")).unwrap();
    fs::create_dir_all(dir.join("right")).unwrap();
    fs::write(dir.join("left").join("mod.rk"), "mod util\n").unwrap();
    fs::write(dir.join("left").join("util.rk"), "answer = -> 1\n").unwrap();
    fs::write(dir.join("right").join("mod.rk"), "mod util\n").unwrap();
    fs::write(dir.join("right").join("util.rk"), "answer = -> 2\n").unwrap();

    let mut db = SourceDatabase::new();
    let graph = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect("nested modules should load");

    let left_util = graph
        .module_by_qualified_name("left::util")
        .expect("left util should be indexed");
    let right_util = graph
        .module_by_qualified_name("right::util")
        .expect("right util should be indexed");
    assert_ne!(left_util.id, right_util.id);
    assert_eq!(
        graph
            .module_for_path(&dir.join("left").join("util.rk"))
            .map(|module| module.id),
        Some(left_util.id)
    );
    assert_eq!(
        graph
            .module_for_path(&dir.join("right").join("util.rk"))
            .map(|module| module.id),
        Some(right_util.id)
    );

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn module_graph_keeps_distinct_logical_modules_for_same_canonical_path() {
    let dir = temp_dir("same_canonical_logical_modules");
    let entry = dir.join("main.rk");
    let left_mod = dir.join("left").join("mod.rk");
    let right_mod = dir.join("right").join("mod.rk");
    fs::write(&entry, "mod left\nmod right\nmain = -> 0\n").unwrap();
    fs::create_dir_all(dir.join("left")).unwrap();
    fs::create_dir_all(dir.join("right")).unwrap();
    fs::write(&left_mod, "mod child\n").unwrap();
    fs::write(dir.join("left").join("child.rk"), "answer = -> 1\n").unwrap();
    fs::write(dir.join("right").join("child.rk"), "answer = -> 2\n").unwrap();
    std::os::unix::fs::symlink(&left_mod, &right_mod).unwrap();

    let mut db = SourceDatabase::new();
    let graph = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect("symlinked modules should load");

    let left_module = graph
        .module_by_qualified_name("left")
        .expect("left module should be indexed");
    let right_module = graph
        .module_by_qualified_name("right")
        .expect("right module should be indexed");
    assert_ne!(left_module.id, right_module.id);
    assert_eq!(left_module.qualified_name, "left");
    assert_eq!(right_module.qualified_name, "right");
    assert_eq!(
        graph.module_for_path(&right_mod).map(|module| module.id),
        Some(right_module.id)
    );
    assert_eq!(
        graph.module_for_path(&left_mod).map(|module| module.id),
        Some(left_module.id)
    );
    assert_eq!(
        graph
            .module_by_qualified_name("right::child")
            .map(|module| module.path.clone()),
        Some(dir.join("right").join("child.rk"))
    );
    assert_eq!(
        graph
            .module_file_cache()
            .get(&left_mod)
            .and_then(|module| module.filepath.clone()),
        Some(left_mod.clone())
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn artifact_backed_source_registration_preserves_origin_metadata() {
    let entry = PathBuf::from("/artifact/app/main.rk");
    let artifact = PathBuf::from("/deps/app.rkca");

    let mut db = SourceDatabase::new();
    db.add_artifact_source(entry.clone(), artifact.clone(), "main = -> 0\n");

    let graph = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect("artifact-backed source should load through explicit registration");
    let source = db
        .source_file_for_path(&entry)
        .expect("registered artifact-backed source should be cached");

    assert_eq!(graph.loaded_files(), &[entry.clone()]);
    assert_eq!(
        source.origin,
        SourceOrigin::Artifact {
            artifact_path: artifact
        }
    );
}

#[test]
fn load_entry_records_root_source_file_and_module() {
    let dir = temp_dir("entry");
    let entry = dir.join("main.rk");
    fs::write(&entry, "main = -> 0\n").unwrap();

    let mut db = SourceDatabase::new();
    let graph = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect("entry should load");

    assert_eq!(graph.root_path(), entry.as_path());
    assert_eq!(graph.root_module().filepath.as_ref(), Some(&entry));
    assert_eq!(graph.loaded_files(), &[entry.clone()]);
    assert!(db.module_for_path(&entry).is_some());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn sibling_resolution_prefers_flat_file_before_mod_rs_style_directory() {
    let dir = temp_dir("sibling_preference");
    let entry = dir.join("main.rk");
    fs::write(&entry, "mod util\nmain = -> util::answer!\n").unwrap();
    fs::write(dir.join("util.rk"), "answer = -> 1\n< answer\n").unwrap();
    fs::create_dir_all(dir.join("util")).unwrap();
    fs::write(dir.join("util").join("mod.rk"), "answer = -> 2\n< answer\n").unwrap();

    let mut db = SourceDatabase::new();
    let graph = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect("entry should load");

    let util = graph
        .module_by_qualified_name("util")
        .expect("util module should be loaded");
    assert_eq!(util.path, dir.join("util.rk"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn sibling_resolution_supports_directory_mod_file() {
    let dir = temp_dir("directory_mod");
    let entry = dir.join("main.rk");
    fs::write(&entry, "mod util\nmain = -> util::answer!\n").unwrap();
    fs::create_dir_all(dir.join("util")).unwrap();
    fs::write(dir.join("util").join("mod.rk"), "answer = -> 2\n< answer\n").unwrap();

    let mut db = SourceDatabase::new();
    let graph = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect("entry should load");

    assert!(graph
        .module_for_path(&dir.join("util").join("mod.rk"))
        .is_some());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn inline_module_nested_mod_resolves_relative_to_containing_source_file() {
    let dir = temp_dir("inline_nested_mod");
    let entry = dir.join("main.rk");
    fs::write(dir.join("util.rk"), "answer = -> 1\n< answer\n").unwrap();

    let root = Module {
        name: None,
        top_levels: vec![TopLevel::Module(ModuleDecl(Module {
            name: Some(Ident {
                name: "outer".to_string(),
                span: crate::lexer::Span::test(),
            }),
            top_levels: vec![TopLevel::Mod(
                Ident {
                    name: "util".to_string(),
                    span: crate::lexer::Span::test(),
                },
                false,
            )],
            is_inline: true,
            filepath: None,
        }))],
        is_inline: true,
        filepath: Some(entry.clone()),
    };
    let mut graph = ModuleGraph::new(entry.clone());
    graph.root_module = root.clone();
    let mut errors = Vec::new();
    let mut stack = Vec::new();

    let mut db = SourceDatabase::new();
    db.load_children(
        &root,
        None,
        &no_std_config(entry.clone()),
        &mut stack,
        &mut graph,
        &mut errors,
        None,
    );

    assert!(errors.is_empty(), "expected no load errors, got {errors:?}");

    let util = graph
        .module_by_qualified_name("outer::util")
        .expect("nested util module should be loaded");
    assert_eq!(util.path, dir.join("util.rk"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cached_module_hit_still_loads_children_on_reused_database() {
    let dir = temp_dir("cached_children");
    let entry = dir.join("main.rk");
    fs::write(&entry, "mod parent\nmain = -> 0\n").unwrap();
    fs::write(dir.join("parent.rk"), "mod child\n").unwrap();

    let mut db = SourceDatabase::new();
    db.load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect_err("missing child should fail first load");

    fs::write(dir.join("child.rk"), "answer = -> 1\n< answer\n").unwrap();
    let graph = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect("second load should discover child");

    let child = graph
        .module_by_qualified_name("parent::child")
        .expect("child module should be loaded from cached parent traversal");
    assert_eq!(child.path, dir.join("child.rk"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn load_entry_graph_loaded_files_are_isolated_per_load() {
    let dir = temp_dir("graph_loaded_files");
    let first = dir.join("first.rk");
    let second = dir.join("second.rk");
    fs::write(&first, "main = -> 0\n").unwrap();
    fs::write(&second, "main = -> 1\n").unwrap();

    let mut db = SourceDatabase::new();
    let first_graph = db
        .load_entry(first.clone(), &no_std_config(first.clone()))
        .expect("first entry should load");
    let second_graph = db
        .load_entry(second.clone(), &no_std_config(second.clone()))
        .expect("second entry should load");

    assert_eq!(first_graph.loaded_files(), &[first.clone()]);
    assert_eq!(second_graph.loaded_files(), &[second.clone()]);
    assert_eq!(db.loaded_files(), &[first.clone(), second.clone()]);

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn source_module_set_lookup_uses_only_preindexed_paths() {
    let dir = temp_dir("source_module_set_no_canonicalize");
    let real_path = dir.join("real.rk");
    let symlink_path = dir.join("link.rk");
    fs::write(&real_path, "main = -> 0\n").unwrap();
    std::os::unix::fs::symlink(&real_path, &symlink_path).unwrap();

    let module = Module {
        name: None,
        top_levels: Vec::new(),
        is_inline: false,
        filepath: Some(real_path.clone()),
    };
    let source_modules = SourceModuleSet::from_modules([LoadedModule {
        id: ModuleId(0),
        qualified_name: "demo".to_string(),
        path: real_path.clone(),
        canonical_path: real_path.canonicalize().unwrap(),
        module,
    }]);

    assert!(source_modules.module_for_path(&real_path).is_some());
    assert!(
        source_modules.module_for_path(&symlink_path).is_none(),
        "lower-facing source module lookup must not canonicalize miss paths"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn missing_entry_file_reports_io_error() {
    let dir = temp_dir("missing_entry");
    let entry = dir.join("main.rk");

    let mut db = SourceDatabase::new();
    let errors = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect_err("missing entry should report an IO error");

    match &errors[..] {
        [SourceLoadError::Io { path, message }] => {
            assert_eq!(path, &entry);
            assert!(!message.is_empty());
        }
        other => panic!("expected one IO error, got {other:?}"),
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn parse_error_preserves_source_path() {
    let dir = temp_dir("parse_error");
    let entry = dir.join("main.rk");
    fs::write(&entry, "main = ->\n").unwrap();

    let mut db = SourceDatabase::new();
    let errors = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect_err("invalid source should report a parse error");

    match &errors[..] {
        [SourceLoadError::Parse {
            path,
            source,
            error: _,
        }] => {
            assert_eq!(path, &entry);
            assert!(source.is_some());
        }
        other => panic!("expected one parse error, got {other:?}"),
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn missing_module_reports_both_searched_paths() {
    let dir = temp_dir("missing_module");
    let entry = dir.join("main.rk");
    fs::write(&entry, "mod missing\nmain = -> 0\n").unwrap();

    let mut db = SourceDatabase::new();
    let errors = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect_err("missing module should fail");

    match &errors[..] {
        [SourceLoadError::MissingModule {
            module,
            searched,
            span,
            parent_source,
        }] => {
            assert_eq!(module, "missing");
            assert_eq!(
                searched,
                &vec![dir.join("missing.rk"), dir.join("missing").join("mod.rk")]
            );
            assert!(span.is_some());
            assert_eq!(
                parent_source.as_ref().map(|source| source.text.as_str()),
                Some("mod missing\nmain = -> 0\n")
            );
        }
        other => panic!("expected one missing module error, got {other:?}"),
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn circular_module_load_reports_stack() {
    let dir = temp_dir("cycle");
    let entry = dir.join("main.rk");
    fs::write(&entry, "mod a\nmain = -> 0\n").unwrap();
    fs::write(dir.join("a.rk"), "mod b\n").unwrap();
    fs::write(dir.join("b.rk"), "mod a\n").unwrap();

    let mut db = SourceDatabase::new();
    let errors = db
        .load_entry(entry.clone(), &no_std_config(entry.clone()))
        .expect_err("cycle should fail");

    assert!(errors.iter().any(|error| match error {
        SourceLoadError::CircularModule { path, stack } => {
            path.ends_with(Path::new("a.rk"))
                && stack.iter().any(|entry| entry.ends_with(Path::new("b.rk")))
        }
        _ => false,
    }));

    let _ = fs::remove_dir_all(&dir);
}
