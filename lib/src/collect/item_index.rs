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
mod tests;
