#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::ast::Module;
use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::{ArtifactCrateInterface, ArtifactCrossCrateHir, ArtifactExport};
use crate::hir::{
    function_is_object_provided_candidate, function_requires_downstream_specialization,
    hir_function_is_codegen_concrete, AcceptedHirFunction as HirFunction,
    AcceptedHirImpl as HirImpl, AcceptedHirTrait as HirTrait, HirLanguageItems,
};
use crate::ids::{CrateId, DefId};
use crate::macro_expansion::proc_macro::ProcMacroArtifact;
use crate::types::Type;

use super::{CrateManifest, ModuleTree};

#[derive(Debug, Clone)]
pub(crate) struct CurrentCrateSource {
    pub manifest: CrateManifest,
    pub root_dir: PathBuf,
    pub ast: Module,
    pub module_tree: Option<ModuleTree>,
    pub file_cache: HashMap<PathBuf, Module>,
    pub loaded_module_paths: Vec<(String, PathBuf)>,
}

impl CurrentCrateSource {
    pub(crate) fn new(manifest: CrateManifest, root_dir: PathBuf, ast: Module) -> Self {
        Self {
            manifest,
            root_dir,
            ast,
            module_tree: None,
            file_cache: HashMap::new(),
            loaded_module_paths: Vec::new(),
        }
    }

    pub(crate) fn root_module_path(&self) -> PathBuf {
        self.ast
            .filepath
            .clone()
            .unwrap_or_else(|| self.root_dir.join(self.manifest.primary_target().path()))
    }

    pub(crate) fn lib_path(&self) -> PathBuf {
        self.root_dir.join(self.manifest.primary_target().path())
    }

    pub(crate) fn build_module_tree(&self) -> Result<ModuleTree, String> {
        if let Some(module_tree) = &self.module_tree {
            return Ok(module_tree.clone());
        }

        crate::crate_system::module_tree::build_module_tree(&self.ast, &self.manifest.crate_.name)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ExternCrateMetadata {
    interface: ArtifactCrateInterface,
    resolver: ResolverTables,
    prelude_export_ids: BTreeMap<String, ArtifactExport>,
    proc_macros: Vec<ProcMacroArtifact>,
    language_items: HirLanguageItems,
}

impl ExternCrateMetadata {
    pub(crate) fn new(
        interface: ArtifactCrateInterface,
        resolver: ResolverTables,
        prelude_export_ids: BTreeMap<String, ArtifactExport>,
    ) -> Self {
        Self {
            interface,
            resolver,
            prelude_export_ids,
            proc_macros: Vec::new(),
            language_items: HirLanguageItems::default(),
        }
    }

    pub(crate) fn with_language_items(mut self, language_items: HirLanguageItems) -> Self {
        self.language_items = language_items;
        self
    }

    pub(crate) fn with_proc_macros(mut self, proc_macros: Vec<ProcMacroArtifact>) -> Self {
        self.proc_macros = proc_macros;
        self
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self::new(
            ArtifactCrateInterface::default(),
            ResolverTables::default(),
            BTreeMap::new(),
        )
    }

    pub(crate) fn interface(&self) -> &ArtifactCrateInterface {
        &self.interface
    }

    pub(crate) fn resolver(&self) -> &ResolverTables {
        &self.resolver
    }

    pub(crate) fn prelude_export_ids(&self) -> &BTreeMap<String, ArtifactExport> {
        &self.prelude_export_ids
    }

    pub(crate) fn proc_macros(&self) -> &[ProcMacroArtifact] {
        &self.proc_macros
    }

    pub(crate) fn language_items(&self) -> &HirLanguageItems {
        &self.language_items
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ExternCrateBodies {
    providers: ExternBodyProviders,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ExternBodyProviders {
    generic_functions_by_id: BTreeMap<DefId, HirFunction>,
    traits_with_defaults_by_id: BTreeMap<DefId, HirTrait>,
    generic_impls_by_id: BTreeMap<DefId, HirImpl>,
}

impl ExternCrateBodies {
    pub(crate) fn from_cross_crate_hir(bundle: ArtifactCrossCrateHir) -> Self {
        Self {
            providers: ExternBodyProviders::from_cross_crate_hir(&bundle),
        }
    }

    pub(crate) fn providers(&self) -> &ExternBodyProviders {
        &self.providers
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.providers.generic_functions_by_id.is_empty()
            && self.providers.traits_with_defaults_by_id.is_empty()
            && self.providers.generic_impls_by_id.is_empty()
    }
}

impl ExternBodyProviders {
    fn from_cross_crate_hir(bundle: &ArtifactCrossCrateHir) -> Self {
        let mut providers = Self::default();

        for (id, function) in &bundle.generic_functions {
            assert_eq!(
                *id, function.id,
                "generic body provider key must match payload ID"
            );
            providers
                .generic_functions_by_id
                .insert(*id, function.clone());
        }

        for (id, trait_def) in &bundle.traits_with_defaults {
            assert_eq!(
                *id, trait_def.id,
                "trait body provider key must match payload ID"
            );
            providers
                .traits_with_defaults_by_id
                .insert(*id, trait_def.clone());
        }

        for imp in &bundle.generic_impls {
            providers.generic_impls_by_id.insert(imp.id, imp.clone());
        }

        providers
    }

    pub(crate) fn generic_functions(&self) -> &BTreeMap<DefId, HirFunction> {
        &self.generic_functions_by_id
    }

    pub(crate) fn generic_function(&self, id: DefId) -> Option<&HirFunction> {
        self.generic_functions_by_id.get(&id)
    }

    pub(crate) fn traits_with_defaults(&self) -> &BTreeMap<DefId, HirTrait> {
        &self.traits_with_defaults_by_id
    }

    pub(crate) fn trait_with_defaults(&self, id: DefId) -> Option<&HirTrait> {
        self.traits_with_defaults_by_id.get(&id)
    }

    pub(crate) fn generic_impls(&self) -> &BTreeMap<DefId, HirImpl> {
        &self.generic_impls_by_id
    }

    pub(crate) fn generic_impl(&self, id: DefId) -> Option<&HirImpl> {
        self.generic_impls_by_id.get(&id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExternCrateLinkage {
    Object { object_path: PathBuf },
    MetadataOnly,
}

#[derive(Debug, Clone)]
pub(crate) struct ExternCrateLink {
    linkage: ExternCrateLinkage,
    backend_symbols: BTreeMap<DefId, String>,
}

impl ExternCrateLink {
    pub(crate) fn object(object_path: PathBuf, backend_symbols: BTreeMap<DefId, String>) -> Self {
        Self {
            linkage: ExternCrateLinkage::Object { object_path },
            backend_symbols,
        }
    }

    pub(crate) fn metadata_only(backend_symbols: BTreeMap<DefId, String>) -> Self {
        Self {
            linkage: ExternCrateLinkage::MetadataOnly,
            backend_symbols,
        }
    }

    pub(crate) fn is_object_backed(&self) -> bool {
        matches!(self.linkage, ExternCrateLinkage::Object { .. })
    }

    pub(crate) fn object_path(&self) -> Option<&PathBuf> {
        match &self.linkage {
            ExternCrateLinkage::Object { object_path } => Some(object_path),
            ExternCrateLinkage::MetadataOnly => None,
        }
    }

    pub(crate) fn backend_symbol(&self, id: DefId) -> Option<&str> {
        self.backend_symbols.get(&id).map(String::as_str)
    }

    fn has_backend_symbol(&self, id: DefId) -> bool {
        self.backend_symbols.contains_key(&id)
    }

    pub(crate) fn concrete_impl_body_is_object_provided(&self, imp: &HirImpl) -> bool {
        self.is_object_backed()
            && impl_link_shape_is_codegen_concrete(imp)
            && imp.methods.values().all(|method| {
                function_is_object_provided_candidate(method) && self.has_backend_symbol(method.id)
            })
    }

    pub(crate) fn impl_method_body_is_object_provided(
        &self,
        imp: &HirImpl,
        method: &HirFunction,
    ) -> bool {
        self.is_object_backed()
            && impl_link_shape_is_codegen_concrete(imp)
            && function_is_object_provided_candidate(method)
            && self.has_backend_symbol(method.id)
    }
}

fn impl_link_shape_is_codegen_concrete(imp: &HirImpl) -> bool {
    imp.type_generics.is_empty()
        && match &imp.receiver_pattern {
            crate::hir::HirImplReceiverPattern::Exact(ty)
            | crate::hir::HirImplReceiverPattern::Constructor(ty) => type_is_codegen_concrete(ty),
            crate::hir::HirImplReceiverPattern::SliceFamily { element } => {
                type_is_codegen_concrete(element)
            }
        }
        && imp.trait_arg_types.iter().all(type_is_codegen_concrete)
        && imp
            .associated_types
            .iter()
            .all(|associated| type_is_codegen_concrete(&associated.ty))
}

fn type_is_codegen_concrete(ty: &Type) -> bool {
    crate::type_services::facts::TypeFacts::is_codegen_concrete(ty)
}

#[derive(Debug, Clone)]
pub(crate) struct ExternCrateRecord {
    crate_id: CrateId,
    name: String,
    metadata: ExternCrateMetadata,
    bodies: ExternCrateBodies,
    link: ExternCrateLink,
}

impl ExternCrateRecord {
    pub(crate) fn new(
        crate_id: CrateId,
        name: String,
        metadata: ExternCrateMetadata,
        bodies: ExternCrateBodies,
        link: ExternCrateLink,
    ) -> Self {
        Self {
            crate_id,
            name,
            metadata,
            bodies,
            link,
        }
    }

    pub(crate) fn crate_id(&self) -> CrateId {
        self.crate_id
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn metadata(&self) -> &ExternCrateMetadata {
        &self.metadata
    }

    pub(crate) fn bodies(&self) -> &ExternCrateBodies {
        &self.bodies
    }

    pub(crate) fn link(&self) -> &ExternCrateLink {
        &self.link
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExternCrateRef<'a> {
    crate_id: CrateId,
    name: &'a str,
    record: &'a ExternCrateRecord,
}

impl<'a> ExternCrateRef<'a> {
    fn new(record: &'a ExternCrateRecord) -> Self {
        Self {
            crate_id: record.crate_id(),
            name: record.name(),
            record,
        }
    }

    pub(crate) fn crate_id(&self) -> CrateId {
        self.crate_id
    }

    pub(crate) fn name(&self) -> &'a str {
        self.name
    }

    pub(crate) fn metadata(&self) -> &'a ExternCrateMetadata {
        self.record.metadata()
    }

    pub(crate) fn root_exports(&self) -> &'a BTreeMap<String, ArtifactExport> {
        &self.record.metadata().interface().root_export_ids
    }

    pub(crate) fn prelude_exports(&self) -> &'a BTreeMap<String, ArtifactExport> {
        self.record.metadata().prelude_export_ids()
    }

    pub(crate) fn bodies(&self) -> &'a ExternCrateBodies {
        self.record.bodies()
    }

    pub(crate) fn body_providers(&self) -> &'a ExternBodyProviders {
        self.record.bodies().providers()
    }

    pub(crate) fn link(&self) -> &'a ExternCrateLink {
        self.record.link()
    }

    pub(crate) fn imported_function_symbol(
        &self,
        _interface_name: &str,
        func: &HirFunction,
    ) -> Option<String> {
        if !self.record.link.is_object_backed()
            || function_requires_downstream_specialization(func)
            || !hir_function_is_codegen_concrete(func)
        {
            return None;
        }

        self.record.link.backend_symbol(func.id).map(str::to_string)
    }

    pub(crate) fn provides_impl_body(&self, imp: &HirImpl) -> bool {
        self.record.metadata.interface().impls.contains_key(&imp.id)
            && self.record.link.concrete_impl_body_is_object_provided(imp)
    }

    pub(crate) fn provides_impl_method_body(&self, imp: &HirImpl, method: &HirFunction) -> bool {
        self.record.metadata.interface().impls.contains_key(&imp.id)
            && self
                .record
                .link
                .impl_method_body_is_object_provided(imp, method)
    }

    pub(crate) fn provides_any_impl_method_body(&self, imp: &HirImpl) -> bool {
        imp.methods
            .values()
            .any(|method| self.provides_impl_method_body(imp, method))
    }

    pub(crate) fn provides_dependency_impl_body(&self, imp: &HirImpl) -> bool {
        self.provides_impl_body(imp) || self.body_providers().generic_impl(imp.id).is_some()
    }

    pub(crate) fn imported_impl_method_symbol(
        &self,
        imp: &HirImpl,
        _method_name: &str,
        method: &HirFunction,
    ) -> Option<String> {
        if !self.provides_impl_method_body(imp, method) {
            return None;
        }

        self.record
            .link
            .backend_symbol(method.id)
            .map(str::to_string)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DependencyLinkInputs {
    pub object_crate_names: Vec<String>,
    pub object_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ExternCrateStore {
    records: BTreeMap<CrateId, ExternCrateRecord>,
    names: BTreeMap<String, CrateId>,
}

impl ExternCrateStore {
    pub(crate) fn insert(&mut self, record: ExternCrateRecord) -> Result<(), String> {
        if self.records.contains_key(&record.crate_id()) {
            return Err(format!(
                "duplicate external crate id {} for '{}'",
                record.crate_id().0,
                record.name()
            ));
        }
        if self.names.contains_key(record.name()) {
            return Err(format!("duplicate external crate name '{}'", record.name()));
        }

        let crate_id = record.crate_id();
        let name = record.name().to_string();
        self.records.insert(crate_id, record);
        self.names.insert(name, crate_id);
        Ok(())
    }

    pub(crate) fn by_name(&self, name: &str) -> Option<ExternCrateRef<'_>> {
        self.names
            .get(name)
            .and_then(|crate_id| self.records.get(crate_id))
            .map(ExternCrateRef::new)
    }

    pub(crate) fn by_crate_id(&self, crate_id: CrateId) -> Option<ExternCrateRef<'_>> {
        self.records.get(&crate_id).map(ExternCrateRef::new)
    }

    pub(crate) fn contains_name(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = ExternCrateRef<'_>> + '_ {
        self.names
            .values()
            .filter_map(|crate_id| self.records.get(crate_id))
            .map(ExternCrateRef::new)
    }

    pub(crate) fn len(&self) -> usize {
        self.records.len()
    }

    pub(crate) fn link_inputs(&self) -> DependencyLinkInputs {
        let mut object_crate_names = Vec::new();
        let mut object_paths = Vec::new();

        for dep in self.iter() {
            if let Some(path) = dep.link().object_path() {
                object_crate_names.push(dep.name().to_string());
                object_paths.push(path.clone());
            }
        }

        DependencyLinkInputs {
            object_crate_names,
            object_paths,
        }
    }
}
