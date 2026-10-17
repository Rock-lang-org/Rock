use std::collections::HashMap;
use std::path::PathBuf;

use crate::ast;
use crate::ids::{CrateId, DefId, IdGen, Idx, LocalDefId, ModuleId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Module,
    Function,
    FunctionSignature,
    ExternFunction,
    Struct,
    Enum,
    Trait,
    Impl,
    NewType,
}

impl ItemKind {
    pub fn matches_top_level(self, top_level: &ast::TopLevel) -> bool {
        item_name_and_kind(top_level).is_some_and(|(_, kind, _)| kind == self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemSourceId {
    pub module_id: ModuleId,
    pub top_level_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRecord {
    pub def_id: DefId,
    pub module_id: ModuleId,
    pub source: ItemSourceId,
    pub name: String,
    pub kind: ItemKind,
    pub name_span: crate::lexer::Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleKind {
    Root,
    Inline,
    SourceBacked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRecord {
    pub module_id: ModuleId,
    pub parent: Option<ModuleId>,
    pub name: Option<String>,
    pub kind: ModuleKind,
}

pub type SourceModuleMap = HashMap<Vec<String>, ast::Module>;

#[derive(Debug)]
pub struct IndexingIds {
    root_crate_id: CrateId,
    root_module_id: ModuleId,
    module_ids: IdGen<ModuleId>,
    local_def_ids: IdGen<LocalDefId>,
}

impl IndexingIds {
    pub fn new_root() -> Self {
        let mut crate_ids = IdGen::<CrateId>::new();
        let mut module_ids = IdGen::<ModuleId>::new();
        let root_crate_id = crate_ids.fresh();
        let root_module_id = module_ids.fresh();

        Self {
            root_crate_id,
            root_module_id,
            module_ids,
            local_def_ids: IdGen::<LocalDefId>::new(),
        }
    }

    pub fn root_crate_id(&self) -> CrateId {
        self.root_crate_id
    }

    pub fn root_module_id(&self) -> ModuleId {
        self.root_module_id
    }

    pub fn fresh_module_id(&mut self) -> ModuleId {
        self.module_ids.fresh()
    }

    pub fn fresh_def_id(&mut self) -> DefId {
        DefId::new(self.root_crate_id, self.local_def_ids.fresh())
    }

    pub fn into_local_def_ids(self) -> IdGen<LocalDefId> {
        self.local_def_ids
    }
}

#[derive(Debug, Default, Clone)]
pub struct ItemIndex {
    items: Vec<ItemRecord>,
    modules: Vec<ModuleRecord>,
    by_name: HashMap<String, Vec<DefId>>,
}

impl ItemIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> &[ItemRecord] {
        &self.items
    }

    pub fn modules(&self) -> &[ModuleRecord] {
        &self.modules
    }

    pub fn get_module(&self, module_id: ModuleId) -> Option<&ModuleRecord> {
        let module = self.modules.get(module_id.index())?;

        if module.module_id == module_id {
            Some(module)
        } else {
            None
        }
    }

    pub fn get(&self, def_id: DefId) -> Option<&ItemRecord> {
        let item = self.items.get(def_id.local.index())?;

        if item.def_id == def_id {
            Some(item)
        } else {
            None
        }
    }

    pub fn defs_named(&self, name: &str) -> &[DefId] {
        self.by_name.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn item_at_source(
        &self,
        module_id: ModuleId,
        top_level_index: usize,
    ) -> Option<&ItemRecord> {
        let source = ItemSourceId {
            module_id,
            top_level_index: top_level_index.try_into().ok()?,
        };
        self.items.iter().find(|item| item.source == source)
    }

    pub fn module_id_by_path(&self, path: &[String]) -> Option<ModuleId> {
        let mut module_id = self
            .modules
            .iter()
            .find(|module| module.kind == ModuleKind::Root)?
            .module_id;

        for name in path {
            module_id = self
                .modules
                .iter()
                .find(|module| {
                    module.parent == Some(module_id) && module.name.as_deref() == Some(name)
                })?
                .module_id;
        }

        Some(module_id)
    }

    pub fn child_module_id(&self, parent_id: ModuleId, name: &str) -> Option<ModuleId> {
        self.modules
            .iter()
            .find(|module| module.parent == Some(parent_id) && module.name.as_deref() == Some(name))
            .map(|module| module.module_id)
    }

    fn push(&mut self, item: ItemRecord) {
        self.by_name
            .entry(item.name.clone())
            .or_default()
            .push(item.def_id);
        self.items.push(item);
    }

    fn push_module(&mut self, module: ModuleRecord) {
        self.modules.push(module);
    }
}

pub fn index_module_items(
    crate_id: CrateId,
    module_id: ModuleId,
    module: &ast::Module,
) -> ItemIndex {
    let mut index = ItemIndex::new();
    let mut local_ids = IdGen::<LocalDefId>::new();

    for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
        let Some((name, kind, name_span)) = item_name_and_kind(top_level) else {
            continue;
        };

        let def_id = DefId::new(crate_id, local_ids.fresh());
        index.push(ItemRecord {
            def_id,
            module_id,
            source: ItemSourceId {
                module_id,
                top_level_index: top_level_index as u32,
            },
            name,
            kind,
            name_span,
        });
    }

    index
}

pub fn index_root_module_items(ids: &mut IndexingIds, module: &ast::Module) -> ItemIndex {
    index_root_module_items_with_sources(ids, module, None, &SourceModuleMap::new())
}

pub fn index_root_module_items_with_sources(
    ids: &mut IndexingIds,
    module: &ast::Module,
    current_crate_name: Option<&str>,
    source_modules: &SourceModuleMap,
) -> ItemIndex {
    let mut index = ItemIndex::new();
    let root_module_id = ids.root_module_id();
    index.push_module(ModuleRecord {
        module_id: root_module_id,
        parent: None,
        name: module.name.as_ref().map(|name| name.name.clone()),
        kind: ModuleKind::Root,
    });

    let mut module_path = current_crate_name
        .map(|name| vec![name.to_string()])
        .unwrap_or_default();
    index_module_contents(
        ids,
        &mut index,
        root_module_id,
        module,
        &mut module_path,
        source_modules,
    );

    index
}

pub fn source_module_map_from_loaded_modules(
    root_module: &ast::Module,
    current_crate_name: Option<&str>,
    loaded_module_paths: &[(String, PathBuf)],
    module_file_cache: &HashMap<PathBuf, ast::Module>,
) -> SourceModuleMap {
    let mut sources = SourceModuleMap::new();

    for (qualified_name, path) in loaded_module_paths {
        let module = module_file_cache.get(path).cloned().or_else(|| {
            let mut matches = module_file_cache
                .values()
                .filter(|module| module.filepath.as_ref() == Some(path));
            let module = matches.next()?;

            if matches.next().is_some() {
                None
            } else {
                Some(module.clone())
            }
        });

        if let Some(module) = module {
            let module_path = qualified_name
                .split("::")
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            sources.insert(module_path, module);
        }
    }

    let mut root_path = current_crate_name
        .map(|name| vec![name.to_string()])
        .unwrap_or_default();
    populate_nested_source_module_paths(
        root_module,
        current_crate_name,
        &mut root_path,
        &mut sources,
    );

    sources
}

pub fn source_module_map_from_graph(graph: &crate::source_loader::ModuleGraph) -> SourceModuleMap {
    graph
        .modules()
        .map(|loaded| {
            (
                loaded
                    .qualified_name
                    .split("::")
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
                loaded.module.clone(),
            )
        })
        .collect()
}

fn populate_nested_source_module_paths(
    module: &ast::Module,
    current_crate_name: Option<&str>,
    module_path: &mut Vec<String>,
    sources: &mut SourceModuleMap,
) {
    for top_level in &module.top_levels {
        match top_level {
            ast::TopLevel::Module(module_decl) => {
                if let Some(module_name) = &module_decl.0.name {
                    module_path.push(module_name.name.clone());
                    populate_nested_source_module_paths(
                        &module_decl.0,
                        current_crate_name,
                        module_path,
                        sources,
                    );
                    module_path.pop();
                }
            }
            ast::TopLevel::Mod(name, _) => {
                module_path.push(name.name.clone());

                if let Some(module) =
                    source_module_for_path(sources, module_path, current_crate_name)
                {
                    sources
                        .entry(module_path.clone())
                        .or_insert_with(|| module.clone());
                    populate_nested_source_module_paths(
                        &module,
                        current_crate_name,
                        module_path,
                        sources,
                    );
                }

                module_path.pop();
            }
            _ => {}
        }
    }
}

fn source_module_for_path(
    sources: &SourceModuleMap,
    module_path: &[String],
    current_crate_name: Option<&str>,
) -> Option<ast::Module> {
    if let Some(module) = sources.get(module_path) {
        return Some(module.clone());
    }

    let crate_name = current_crate_name?;
    if module_path.first().map(String::as_str) == Some(crate_name) {
        return sources.get(&module_path[1..]).cloned();
    }

    let mut crate_qualified = Vec::with_capacity(module_path.len() + 1);
    crate_qualified.push(crate_name.to_string());
    crate_qualified.extend(module_path.iter().cloned());
    sources.get(&crate_qualified).cloned()
}

fn index_module_contents(
    ids: &mut IndexingIds,
    index: &mut ItemIndex,
    module_id: ModuleId,
    module: &ast::Module,
    module_path: &mut Vec<String>,
    source_modules: &SourceModuleMap,
) {
    for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
        match top_level {
            ast::TopLevel::Module(module_decl) => {
                let Some((name, kind, name_span)) = item_name_and_kind(top_level) else {
                    continue;
                };
                let child_module_id = ids.fresh_module_id();

                index.push(ItemRecord {
                    def_id: ids.fresh_def_id(),
                    module_id,
                    source: ItemSourceId {
                        module_id,
                        top_level_index: top_level_index as u32,
                    },
                    name,
                    kind,
                    name_span,
                });
                index.push_module(ModuleRecord {
                    module_id: child_module_id,
                    parent: Some(module_id),
                    name: module_decl.0.name.as_ref().map(|name| name.name.clone()),
                    kind: ModuleKind::Inline,
                });
                if let Some(module_name) = &module_decl.0.name {
                    module_path.push(module_name.name.clone());
                }
                index_module_contents(
                    ids,
                    index,
                    child_module_id,
                    &module_decl.0,
                    module_path,
                    source_modules,
                );
                if module_decl.0.name.is_some() {
                    module_path.pop();
                }
            }
            ast::TopLevel::Mod(name, _) => {
                let child_module_id = ids.fresh_module_id();

                index.push(ItemRecord {
                    def_id: ids.fresh_def_id(),
                    module_id,
                    source: ItemSourceId {
                        module_id,
                        top_level_index: top_level_index as u32,
                    },
                    name: name.name.clone(),
                    kind: ItemKind::Module,
                    name_span: name.span.clone(),
                });
                index.push_module(ModuleRecord {
                    module_id: child_module_id,
                    parent: Some(module_id),
                    name: Some(name.name.clone()),
                    kind: ModuleKind::SourceBacked,
                });

                module_path.push(name.name.clone());
                if let Some(source_module) = source_modules.get(&*module_path) {
                    index_module_contents(
                        ids,
                        index,
                        child_module_id,
                        source_module,
                        module_path,
                        source_modules,
                    );
                }
                module_path.pop();
            }
            _ => {
                let Some((name, kind, name_span)) = item_name_and_kind(top_level) else {
                    continue;
                };

                index.push(ItemRecord {
                    def_id: ids.fresh_def_id(),
                    module_id,
                    source: ItemSourceId {
                        module_id,
                        top_level_index: top_level_index as u32,
                    },
                    name,
                    kind,
                    name_span,
                });
            }
        }
    }
}

fn item_name_and_kind(top_level: &ast::TopLevel) -> Option<(String, ItemKind, crate::lexer::Span)> {
    match top_level {
        ast::TopLevel::Module(module_decl) => module_decl
            .0
            .name
            .as_ref()
            .map(|name| (name.name.clone(), ItemKind::Module, name.span.clone())),
        ast::TopLevel::Mod(name, _) => {
            Some((name.name.clone(), ItemKind::Module, name.span.clone()))
        }
        ast::TopLevel::Extern(sig) => Some((
            sig.name.name.clone(),
            ItemKind::ExternFunction,
            sig.name.span.clone(),
        )),
        ast::TopLevel::FunctionSig(sig) => Some((
            sig.name.name.clone(),
            ItemKind::FunctionSignature,
            sig.name.span.clone(),
        )),
        ast::TopLevel::FunctionDecl(func) => Some((
            func.name.name.clone(),
            ItemKind::Function,
            func.name.span.clone(),
        )),
        ast::TopLevel::StructDecl(strukt) => Some((
            strukt.name.name.clone(),
            ItemKind::Struct,
            strukt.name.span.clone(),
        )),
        ast::TopLevel::TraitDecl(trait_) => Some((
            trait_.name.name.clone(),
            ItemKind::Trait,
            trait_.name.span.clone(),
        )),
        ast::TopLevel::EnumDecl(enum_) => Some((
            enum_.name.name.clone(),
            ItemKind::Enum,
            enum_.name.span.clone(),
        )),
        ast::TopLevel::Impl(impl_) => Some((
            impl_.name.type_name(),
            ItemKind::Impl,
            impl_.name.span(),
        )),
        ast::TopLevel::NewType(name, _, _) => {
            Some((name.name.clone(), ItemKind::NewType, name.span.clone()))
        }
        ast::TopLevel::Import(_)
        | ast::TopLevel::GlobImport(_)
        | ast::TopLevel::Export(_)
        | ast::TopLevel::GlobExport(_)
        | ast::TopLevel::InfixOperator(_, _)
        | ast::TopLevel::MacroDecl(_)
        | ast::TopLevel::MacroInvoc(_) => None,
    }
}

#[cfg(test)]
mod tests {
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
        let loaded_module_paths =
            vec![("test::io".to_string(), PathBuf::from("/virtual/test/io.rk"))];
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
                TopLevel::InfixOperator(5, "+".to_string()),
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
            TopLevel::InfixOperator(5, "+".to_string()),
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
                TopLevel::InfixOperator(5, "+".to_string()),
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
        let operator = TopLevel::InfixOperator(5, "+".to_string());

        assert!(ItemKind::Struct.matches_top_level(&structure));
        assert!(!ItemKind::Function.matches_top_level(&structure));
        assert!(!ItemKind::Struct.matches_top_level(&operator));
    }

    #[test]
    fn source_module_map_uses_cached_module_filepaths_without_filesystem_io() {
        let loaded_module_paths =
            vec![("test::io".to_string(), PathBuf::from("/virtual/test/io.rk"))];
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
        let loaded_module_paths =
            vec![("test::io".to_string(), PathBuf::from("/virtual/test/io.rk"))];
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
        let module = module_with(vec![TopLevel::InfixOperator(5, "+".to_string())]);

        let index = index_module_items(CrateId(0), ModuleId(0), &module);

        assert!(index.is_empty());
        assert!(index.defs_named("+").is_empty());
    }
}
