use super::*;
use crate::ast::{
    EnumDecl, Ident, Module, ModuleDecl, ParseTypeInner, StructDecl, TopLevel, TraitDecl,
};
use crate::lexer::Span;

fn type_inner(name: &str) -> ParseTypeInner {
    ParseTypeInner {
        name: name.to_string(),
        generics: vec![],
        span: Span::test(),
    }
}

fn struct_item(name: &str) -> TopLevel {
    TopLevel::StructDecl(StructDecl {
        name: type_inner(name),
        generic_params: vec![],
        fields: vec![],
        exported: false,
    })
}

fn enum_item(name: &str) -> TopLevel {
    TopLevel::EnumDecl(EnumDecl {
        generic_params: Vec::new(),
        name: type_inner(name),
        variants: vec![],
        exported: false,
        language_items: Default::default(),
    })
}

fn trait_item(name: &str) -> TopLevel {
    TopLevel::TraitDecl(TraitDecl {
        where_clauses: Vec::new(),
        name: type_inner(name),
        generic_params: vec![],
        for_: None,
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        exported: false,
        language_items: Default::default(),
    })
}

fn module_with(top_levels: Vec<TopLevel>) -> Module {
    Module {
        name: None,
        top_levels,
        is_inline: false,
        filepath: None,
    }
}

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: Span::test(),
    }
}

fn inline_module(name: &str, top_levels: Vec<TopLevel>) -> TopLevel {
    TopLevel::Module(ModuleDecl(Module {
        name: Some(ident(name)),
        top_levels,
        is_inline: true,
        filepath: None,
    }))
}

fn source_mod(name: &str) -> TopLevel {
    TopLevel::Mod(ident(name), false)
}

#[test]
fn source_module_map_derives_nested_inline_source_backed_paths_from_cached_asts() {
    let root_module = Module {
        name: None,
        top_levels: vec![inline_module("math", vec![source_mod("io")])],
        is_inline: false,
        filepath: Some(PathBuf::from("/virtual/test/main.rk")),
    };
    let loaded_module_paths = vec![(
        "test::math::io".to_string(),
        PathBuf::from("/virtual/test/io.rk"),
    )];
    let mut module_file_cache = HashMap::new();
    module_file_cache.insert(
        PathBuf::from("/cache-key/io.rk"),
        Module {
            name: Some(ident("io")),
            top_levels: vec![struct_item("Writer")],
            is_inline: false,
            filepath: Some(PathBuf::from("/virtual/test/io.rk")),
        },
    );

    let source_modules = source_module_map_from_loaded_modules(
        &root_module,
        Some("test"),
        &loaded_module_paths,
        &module_file_cache,
    );

    let io_module = source_modules
        .get(&vec![
            "test".to_string(),
            "math".to_string(),
            "io".to_string(),
        ])
        .expect("nested source-backed module should be indexed under inline module path");
    assert_eq!(
        io_module.filepath,
        Some(PathBuf::from("/virtual/test/io.rk"))
    );
    assert_eq!(io_module.top_levels.len(), 1);
}

#[test]
fn source_module_map_does_not_reinterpret_top_level_module_as_nested_inline_child() {
    let root_module = Module {
        name: None,
        top_levels: vec![inline_module("math", vec![source_mod("io")])],
        is_inline: false,
        filepath: Some(PathBuf::from("/virtual/test/main.rk")),
    };
    let loaded_module_paths = vec![("test::io".to_string(), PathBuf::from("/virtual/test/io.rk"))];
    let mut module_file_cache = HashMap::new();
    module_file_cache.insert(
        PathBuf::from("/cache-key/io.rk"),
        Module {
            name: Some(ident("io")),
            top_levels: vec![struct_item("Writer")],
            is_inline: false,
            filepath: Some(PathBuf::from("/virtual/test/io.rk")),
        },
    );

    let source_modules = source_module_map_from_loaded_modules(
        &root_module,
        Some("test"),
        &loaded_module_paths,
        &module_file_cache,
    );

    assert!(!source_modules.contains_key(&vec![
        "test".to_string(),
        "math".to_string(),
        "io".to_string(),
    ]));
}

#[test]
fn indexing_ids_allocates_root_crate_and_module_ids() {
    let ids = IndexingIds::new_root();

    assert_eq!(ids.root_crate_id(), CrateId(0));
    assert_eq!(ids.root_module_id(), ModuleId(0));
}

#[test]
fn indexing_ids_allocates_modules_and_def_ids_monotonically() {
    let mut ids = IndexingIds::new_root();

    assert_eq!(ids.root_crate_id(), CrateId(0));
    assert_eq!(ids.root_module_id(), ModuleId(0));
    assert_eq!(ids.fresh_module_id(), ModuleId(1));
    assert_eq!(ids.fresh_module_id(), ModuleId(2));
    assert_eq!(ids.fresh_def_id(), DefId::new(CrateId(0), LocalDefId(0)));
    assert_eq!(ids.fresh_def_id(), DefId::new(CrateId(0), LocalDefId(1)));
}

#[test]
fn indexes_root_module_items_from_indexing_ids() {
    let mut ids = IndexingIds::new_root();
    let module = module_with(vec![struct_item("Point")]);

    let index = index_root_module_items(&mut ids, &module);
    let point_id = index.defs_named("Point")[0];
    let point = index.get(point_id).unwrap();

    assert_eq!(point.def_id.crate_id, ids.root_crate_id());
    assert_eq!(point.module_id, ids.root_module_id());
}

#[test]
fn indexes_root_module_record() {
    let mut ids = IndexingIds::new_root();
    let module = module_with(vec![struct_item("Point")]);

    let index = index_root_module_items(&mut ids, &module);
    let root = index
        .get_module(ids.root_module_id())
        .expect("root module should be indexed");

    assert_eq!(index.modules().len(), 1);
    assert_eq!(root.module_id, ids.root_module_id());
    assert_eq!(root.parent, None);
    assert_eq!(root.name, None);
    assert_eq!(root.kind, ModuleKind::Root);
}

#[test]
fn indexes_inline_module_records_and_body_items() {
    let mut ids = IndexingIds::new_root();
    let module = module_with(vec![
        struct_item("RootThing"),
        inline_module("math", vec![struct_item("Vector")]),
    ]);

    let index = index_root_module_items(&mut ids, &module);

    assert_eq!(index.modules().len(), 2);

    let math_module = index.get_module(ModuleId(1)).unwrap();
    assert_eq!(math_module.parent, Some(ModuleId(0)));
    assert_eq!(math_module.name.as_deref(), Some("math"));
    assert_eq!(math_module.kind, ModuleKind::Inline);

    let math_item = index.get(index.defs_named("math")[0]).unwrap();
    assert_eq!(math_item.kind, ItemKind::Module);
    assert_eq!(math_item.module_id, ModuleId(0));
    assert_eq!(math_item.def_id, DefId::new(CrateId(0), LocalDefId(1)));

    let vector = index.get(index.defs_named("Vector")[0]).unwrap();
    assert_eq!(vector.kind, ItemKind::Struct);
    assert_eq!(vector.module_id, ModuleId(1));
    assert_eq!(vector.def_id, DefId::new(CrateId(0), LocalDefId(2)));
}

#[test]
fn finds_inline_item_by_original_source_ordinal() {
    let mut ids = IndexingIds::new_root();
    let module = module_with(vec![inline_module(
        "math",
        vec![
            TopLevel::InfixOperator(5, crate::ast::Operator { value: "+".to_string(), span: crate::lexer::Span::new("test.rk".into(), 0, 0) }),
            struct_item("Vector"),
        ],
    )]);

    let index = index_root_module_items(&mut ids, &module);
    let inline_module_id = index
        .module_id_by_path(&["math".to_string()])
        .expect("inline module should be indexed");

    assert_eq!(index.item_at_source(inline_module_id, 0), None);
    let vector = index
        .item_at_source(inline_module_id, 1)
        .expect("source item should be indexed");
    assert_eq!(vector.name, "Vector");
    assert_eq!(vector.source.module_id, inline_module_id);
    assert_eq!(vector.source.top_level_index, 1);
}

#[test]
fn indexes_source_backed_module_shell_records() {
    let mut ids = IndexingIds::new_root();
    let module = module_with(vec![source_mod("io")]);

    let index = index_root_module_items(&mut ids, &module);

    assert_eq!(index.modules().len(), 2);

    let io_module = index.get_module(ModuleId(1)).unwrap();
    assert_eq!(io_module.parent, Some(ModuleId(0)));
    assert_eq!(io_module.name.as_deref(), Some("io"));
    assert_eq!(io_module.kind, ModuleKind::SourceBacked);

    let io_item = index.get(index.defs_named("io")[0]).unwrap();
    assert_eq!(io_item.kind, ItemKind::Module);
    assert_eq!(io_item.module_id, ModuleId(0));
    assert_eq!(io_item.def_id, DefId::new(CrateId(0), LocalDefId(0)));
}

#[test]
fn indexes_source_backed_body_when_ast_is_provided() {
    let mut ids = IndexingIds::new_root();
    let module = module_with(vec![source_mod("io")]);
    let mut sources = SourceModuleMap::new();
    sources.insert(
        vec!["io".to_string()],
        module_with(vec![struct_item("Writer")]),
    );

    let index = index_root_module_items_with_sources(&mut ids, &module, None, &sources);

    let io_module = index.get_module(ModuleId(1)).unwrap();
    assert_eq!(io_module.kind, ModuleKind::SourceBacked);

    let writer = index.get(index.defs_named("Writer")[0]).unwrap();
    assert_eq!(writer.kind, ItemKind::Struct);
    assert_eq!(writer.module_id, ModuleId(1));
    assert_eq!(writer.def_id, DefId::new(CrateId(0), LocalDefId(1)));
}

#[test]
fn finds_root_item_by_original_source_ordinal() {
    let mut ids = IndexingIds::new_root();
    let module = module_with(vec![
        TopLevel::InfixOperator(5, crate::ast::Operator { value: "+".to_string(), span: crate::lexer::Span::new("test.rk".into(), 0, 0) }),
        struct_item("Answer"),
    ]);

    let index = index_root_module_items(&mut ids, &module);

    let answer = index
        .item_at_source(ids.root_module_id(), 1)
        .expect("source item should be indexed");
    assert_eq!(answer.source.module_id, ids.root_module_id());
    assert_eq!(answer.source.top_level_index, 1);
    assert_eq!(answer.name, "Answer");
}

#[test]
fn finds_source_backed_item_by_original_source_ordinal() {
    let mut ids = IndexingIds::new_root();
    let module = module_with(vec![source_mod("io")]);
    let mut sources = SourceModuleMap::new();
    sources.insert(
        vec!["io".to_string()],
        module_with(vec![
            TopLevel::InfixOperator(5, crate::ast::Operator { value: "+".to_string(), span: crate::lexer::Span::new("test.rk".into(), 0, 0) }),
            struct_item("Writer"),
        ]),
    );

    let index = index_root_module_items_with_sources(&mut ids, &module, None, &sources);

    let writer = index
        .item_at_source(ModuleId(1), 1)
        .expect("source item should be indexed");
    assert_eq!(writer.source.module_id, ModuleId(1));
    assert_eq!(writer.source.top_level_index, 1);
    assert_eq!(writer.name, "Writer");
}

#[test]
fn finds_modules_by_exact_structural_path() {
    let mut ids = IndexingIds::new_root();
    let module = module_with(vec![source_mod("same")]);
    let mut sources = SourceModuleMap::new();
    sources.insert(
        vec!["same".to_string()],
        module_with(vec![source_mod("same")]),
    );
    sources.insert(
        vec!["same".to_string(), "same".to_string()],
        module_with(vec![struct_item("Leaf")]),
    );

    let index = index_root_module_items_with_sources(&mut ids, &module, None, &sources);

    assert_eq!(index.module_id_by_path(&[]), Some(ids.root_module_id()));
    assert_eq!(
        index.module_id_by_path(&["same".to_string()]),
        Some(ModuleId(1))
    );
    assert_eq!(
        index.module_id_by_path(&["same".to_string(), "same".to_string()]),
        Some(ModuleId(2)),
    );
    assert_eq!(
        index.module_id_by_path(&["same".to_string(), "missing".to_string()]),
        None,
    );
    assert_eq!(index.module_id_by_path(&["missing".to_string()]), None);
}

#[test]
fn item_kind_matches_existing_top_level_classification() {
    let structure = struct_item("Point");
    let operator = TopLevel::InfixOperator(5, crate::ast::Operator { value: "+".to_string(), span: crate::lexer::Span::new("test.rk".into(), 0, 0) });

    assert!(ItemKind::Struct.matches_top_level(&structure));
    assert!(!ItemKind::Function.matches_top_level(&structure));
    assert!(!ItemKind::Struct.matches_top_level(&operator));
}

#[test]
fn source_module_map_uses_cached_module_filepaths_without_filesystem_io() {
    let loaded_module_paths = vec![("test::io".to_string(), PathBuf::from("/virtual/test/io.rk"))];
    let mut module_file_cache = HashMap::new();
    module_file_cache.insert(
        PathBuf::from("/cache-key/io.rk"),
        Module {
            name: Some(ident("io")),
            top_levels: vec![struct_item("Writer")],
            is_inline: false,
            filepath: Some(PathBuf::from("/virtual/test/io.rk")),
        },
    );

    let source_modules = source_module_map_from_loaded_modules(
        &module_with(vec![]),
        None,
        &loaded_module_paths,
        &module_file_cache,
    );

    let io_module = source_modules
        .get(&vec!["test".to_string(), "io".to_string()])
        .unwrap();
    assert_eq!(
        io_module.filepath,
        Some(PathBuf::from("/virtual/test/io.rk"))
    );
    assert_eq!(io_module.top_levels.len(), 1);
}

#[test]
fn source_module_map_rejects_ambiguous_cached_filepaths() {
    let loaded_module_paths = vec![("test::io".to_string(), PathBuf::from("/virtual/test/io.rk"))];
    let mut module_file_cache = HashMap::new();
    module_file_cache.insert(
        PathBuf::from("/cache-key/first.rk"),
        Module {
            name: Some(ident("io")),
            top_levels: vec![struct_item("Writer")],
            is_inline: false,
            filepath: Some(PathBuf::from("/virtual/test/io.rk")),
        },
    );
    module_file_cache.insert(
        PathBuf::from("/cache-key/second.rk"),
        Module {
            name: Some(ident("io")),
            top_levels: vec![struct_item("Reader")],
            is_inline: false,
            filepath: Some(PathBuf::from("/virtual/test/io.rk")),
        },
    );

    let source_modules = source_module_map_from_loaded_modules(
        &module_with(vec![]),
        None,
        &loaded_module_paths,
        &module_file_cache,
    );

    assert!(!source_modules.contains_key(&vec!["test".to_string(), "io".to_string()]));
}

#[test]
fn indexes_named_top_level_items_with_def_ids() {
    let module = module_with(vec![
        struct_item("Point"),
        enum_item("Choice"),
        trait_item("Show"),
    ]);

    let index = index_module_items(CrateId(7), ModuleId(3), &module);

    assert_eq!(index.len(), 3);
    assert_eq!(index.defs_named("Point").len(), 1);

    let point_id = index.defs_named("Point")[0];
    let point = index.get(point_id).expect("Point should be indexed");
    assert_eq!(point.def_id, DefId::new(CrateId(7), LocalDefId(0)));
    assert_eq!(point.module_id, ModuleId(3));
    assert_eq!(point.name, "Point");
    assert_eq!(point.kind, ItemKind::Struct);

    assert_eq!(index.defs_named("Choice")[0].local, LocalDefId(1));
    assert_eq!(index.defs_named("Show")[0].local, LocalDefId(2));
}

#[test]
fn keeps_multiple_defs_with_the_same_source_name() {
    let module = module_with(vec![struct_item("Thing"), enum_item("Thing")]);

    let index = index_module_items(CrateId(0), ModuleId(0), &module);

    let defs = index.defs_named("Thing");

    assert_eq!(defs.len(), 2);
    assert_eq!(index.get(defs[0]).unwrap().kind, ItemKind::Struct);
    assert_eq!(index.get(defs[1]).unwrap().kind, ItemKind::Enum);
}

#[test]
fn ignores_non_defining_top_level_items() {
    let module = module_with(vec![TopLevel::InfixOperator(5, crate::ast::Operator { value: "+".to_string(), span: crate::lexer::Span::new("test.rk".into(), 0, 0) })]);

    let index = index_module_items(CrateId(0), ModuleId(0), &module);

    assert!(index.is_empty());
    assert!(index.defs_named("+").is_empty());
}
