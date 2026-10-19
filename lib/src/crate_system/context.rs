use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::ast::Module;
use crate::ast::Program;
use crate::collect::collect_with_source_graph;
use crate::diagnostic::{Diagnostic, Diagnostics};
use crate::hir::AcceptedHirImpl as HirImpl;
use crate::macro_expansion::proc_macro::ProcMacroArtifact;
use crate::source_loader::SourceDatabase;
use crate::{Config, SourceProvider};

use super::{CrateContext, CrateManifest, CurrentCrateSource, ExternCrateRecord, ExternCrateRef};

impl Default for CrateContext {
    fn default() -> Self {
        Self::new()
    }
}

fn toolchain_diagnostics(message: impl Into<String>) -> Diagnostics {
    let mut diagnostics = Diagnostics::default();
    diagnostics.push(Diagnostic::for_toolchain(message.into()));
    diagnostics
}

fn annotate_dependency_diagnostics(
    mut diagnostics: Diagnostics,
    dependency: &str,
    crate_name: &str,
) -> Diagnostics {
    let note = format!(
        "while loading dependency '{}' of crate '{}'",
        dependency, crate_name
    );
    for diagnostic in &mut diagnostics.0 {
        diagnostic.notes.push(note.clone());
    }
    diagnostics
}

fn register_source_providers(source_db: &mut SourceDatabase, source_providers: &[SourceProvider]) {
    source_db.add_source_providers(source_providers);
}

impl CrateContext {
    pub fn new() -> Self {
        Self {
            source_crates: BTreeMap::new(),
            extern_crates: Default::default(),
            product_crate_ids: BTreeMap::new(),
            next_product_crate_id: 1,
        }
    }

    pub(crate) fn consumer_crate_id_for_product_identity(
        &mut self,
        identity: &crate::products::ProductCrateIdentity,
    ) -> crate::ids::CrateId {
        if let Some(crate_id) = self.product_crate_ids.get(identity).copied() {
            return crate_id;
        }

        let crate_id = crate::ids::CrateId(self.next_product_crate_id);
        self.next_product_crate_id = self
            .next_product_crate_id
            .checked_add(1)
            .expect("consumer product crate ID generator exhausted u32 ID space");
        self.product_crate_ids.insert(identity.clone(), crate_id);
        crate_id
    }

    pub fn register_crate(&mut self, manifest: CrateManifest, source_dir: PathBuf, ast: Module) {
        let name = manifest.crate_.name.clone();
        self.source_crates
            .insert(name, CurrentCrateSource::new(manifest, source_dir, ast));
    }

    pub fn load_crate_from_dir(&mut self, crate_dir: PathBuf) -> Result<(), Diagnostics> {
        self.load_crate_from_dir_with_source_providers(crate_dir, Vec::new())
    }

    pub fn load_crate_from_dir_with_source_providers(
        &mut self,
        crate_dir: PathBuf,
        source_providers: Vec<SourceProvider>,
    ) -> Result<(), Diagnostics> {
        let manifest_path = crate_dir.join("rock.toml");
        let manifest = Self::load_manifest(&manifest_path).map_err(toolchain_diagnostics)?;

        let lib_path = crate_dir.join(manifest.primary_target().path());
        let source_config = Config {
            entry_file: lib_path.clone(),
            no_prelude: true,
            no_std: true,
            current_crate_name: Some(manifest.crate_.name.clone()),
            ..Config::default()
        };
        let mut source_db = SourceDatabase::new();
        register_source_providers(&mut source_db, &source_providers);
        let graph = match source_db.load_source_crate(
            lib_path.clone(),
            &manifest.crate_.name,
            &source_config,
        ) {
            Ok(graph) => graph,
            Err(errors) => {
                let sources =
                    crate::diagnostic::DiagnosticSourceMap::from_source_database(&source_db);
                return Err(crate::source_load_errors_to_diagnostics(errors).with_sources(&sources));
            }
        };
        let ast = graph.root_module().clone();
        let file_cache = crate::crate_system::module_tree::module_file_cache_from_graph(&graph);
        let loaded_module_paths = graph.loaded_module_paths();
        let module_tree = crate::crate_system::module_tree::build_module_tree_from_graph(
            &graph,
            &manifest.crate_.name,
        )
        .map_err(toolchain_diagnostics)?;

        collect_with_source_graph(
            &Program {
                module: ast.clone(),
            },
            &graph,
            self,
            false,
            Some(&manifest.crate_.name),
        )
        .map_err(|errors| {
            let sources = crate::diagnostic::DiagnosticSourceMap::from_source_database(&source_db);
            crate::diagnostic::Diagnostics::from_resolve_errors(errors).with_sources(&sources)
        })?;

        let name = manifest.crate_.name.clone();
        let mut source = CurrentCrateSource::new(manifest, crate_dir, ast);
        source.file_cache = file_cache;
        source.loaded_module_paths = loaded_module_paths;
        source.module_tree = Some(module_tree);
        self.source_crates.insert(name, source);
        Ok(())
    }

    pub fn get_crate_lib_path(&self, name: &str) -> Option<PathBuf> {
        self.source_crate(name).map(CurrentCrateSource::lib_path)
    }

    pub fn has_crate(&self, name: &str) -> bool {
        self.has_source_crate(name) || self.has_extern_crate(name)
    }

    pub fn crate_count(&self) -> usize {
        let mut names = std::collections::BTreeSet::new();
        names.extend(self.source_crates.keys().cloned());
        names.extend(self.extern_crates().map(|dep| dep.name().to_string()));
        names.len()
    }

    #[allow(dead_code)]
    pub(crate) fn add_extern_crate(&mut self, record: ExternCrateRecord) -> Result<(), String> {
        self.extern_crates.insert(record)
    }

    #[allow(dead_code)]
    pub(crate) fn extern_crate(&self, name: &str) -> Option<ExternCrateRef<'_>> {
        self.extern_crates.by_name(name)
    }

    pub(crate) fn extern_crates(&self) -> impl Iterator<Item = ExternCrateRef<'_>> + '_ {
        self.extern_crates.iter()
    }

    pub(crate) fn is_current_source_self_artifact(
        &self,
        current_crate_name: Option<&str>,
        extern_crate_name: &str,
    ) -> bool {
        let Some(current_crate_name) = current_crate_name else {
            return false;
        };

        current_crate_name == extern_crate_name
            && self.source_crates.contains_key(current_crate_name)
    }

    pub(crate) fn impl_body_provider(&self, imp: &HirImpl) -> Option<ExternCrateRef<'_>> {
        self.extern_crates().find(|dep| dep.provides_impl_body(imp))
    }

    pub(crate) fn provides_impl_body(&self, imp: &HirImpl) -> bool {
        self.impl_body_provider(imp).is_some()
    }

    pub(crate) fn provides_dependency_impl_body(&self, imp: &HirImpl) -> bool {
        self.extern_crates()
            .any(|dep| dep.provides_dependency_impl_body(imp))
    }

    pub(crate) fn proc_macro_artifacts(&self) -> impl Iterator<Item = &ProcMacroArtifact> + '_ {
        self.extern_crates()
            .flat_map(|dep| dep.metadata().proc_macros().iter())
    }

    pub(crate) fn has_extern_crate(&self, name: &str) -> bool {
        self.extern_crates.contains_name(name)
    }

    #[allow(dead_code)]
    pub(crate) fn source_crate(&self, name: &str) -> Option<&CurrentCrateSource> {
        self.source_crates.get(name)
    }

    #[allow(dead_code)]
    pub(crate) fn source_crate_mut(&mut self, name: &str) -> Option<&mut CurrentCrateSource> {
        self.source_crates.get_mut(name)
    }

    pub(crate) fn has_source_crate(&self, name: &str) -> bool {
        self.source_crates.contains_key(name)
    }

    pub(crate) fn dependency_errors_for_phase(&self, phase: &str) -> Vec<String> {
        self.source_crates
            .keys()
            .filter_map(|crate_name| {
                if self.has_extern_crate(crate_name) {
                    return None;
                }

                Some(format!(
                    "source-backed external dependency '{}' is not supported during {}; build it as a product artifact and pass it with --extern-artifact {}=<path>",
                    crate_name, phase, crate_name
                ))
            })
            .collect()
    }

    pub(crate) fn dependency_link_inputs(&self) -> super::DependencyLinkInputs {
        self.extern_crates.link_inputs()
    }

    pub fn load_crate_with_dependencies(
        &mut self,
        crate_dir: PathBuf,
        loading_stack: &mut Vec<String>,
    ) -> Result<String, Diagnostics> {
        self.load_crate_with_dependencies_inner(crate_dir, loading_stack, &[])
    }

    pub fn load_crate_with_dependencies_with_source_providers(
        &mut self,
        crate_dir: PathBuf,
        loading_stack: &mut Vec<String>,
        source_providers: Vec<SourceProvider>,
    ) -> Result<String, Diagnostics> {
        self.load_crate_with_dependencies_inner(crate_dir, loading_stack, &source_providers)
    }

    fn load_crate_with_dependencies_inner(
        &mut self,
        crate_dir: PathBuf,
        loading_stack: &mut Vec<String>,
        source_providers: &[SourceProvider],
    ) -> Result<String, Diagnostics> {
        let manifest_path = crate_dir.join("rock.toml");
        let manifest = Self::load_manifest(&manifest_path).map_err(toolchain_diagnostics)?;

        let crate_name = manifest.crate_.name.clone();

        if let Some(pos) = loading_stack.iter().position(|n| n == &crate_name) {
            let cycle = loading_stack[pos..].join(" -> ");
            return Err(toolchain_diagnostics(format!(
                "Circular dependency detected: {} -> {}",
                cycle, crate_name
            )));
        }

        if self.has_crate(&crate_name) {
            return Ok(crate_name);
        }

        loading_stack.push(crate_name.clone());

        if let Some(ref deps) = manifest.dependencies {
            for (dep_name, dep) in deps {
                let dep_path = if let Some(ref path) = dep.path {
                    if path.starts_with("..") || path.starts_with(".") {
                        crate_dir.join(path).canonicalize().map_err(|e| {
                            toolchain_diagnostics(format!(
                                "Failed to resolve path '{}': {}",
                                path, e
                            ))
                        })?
                    } else {
                        PathBuf::from(path)
                    }
                } else if let Some(ref version) = dep.version {
                    return Err(toolchain_diagnostics(format!(
                        "Registry-based dependencies not yet supported ({} version {})",
                        dep_name, version
                    )));
                } else {
                    return Err(toolchain_diagnostics(format!(
                        "Dependency '{}' must have either 'path' or 'version'",
                        dep_name
                    )));
                };

                self.load_crate_with_dependencies_inner(dep_path, loading_stack, source_providers)
                    .map_err(|diagnostics| {
                        annotate_dependency_diagnostics(diagnostics, dep_name, &crate_name)
                    })?;
            }
        }

        self.load_crate_from_dir_with_source_providers(crate_dir, source_providers.to_vec())?;

        if let Some(source_crate) = self.source_crate_mut(&crate_name) {
            match source_crate.build_module_tree() {
                Ok(tree) => {
                    source_crate.module_tree = Some(tree);
                }
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to build module tree for '{}': {}",
                        crate_name, e
                    );
                }
            }
        }

        loading_stack.pop();

        Ok(crate_name)
    }

    pub fn compilation_order(&self) -> Result<Vec<String>, String> {
        use std::collections::{HashMap, HashSet, VecDeque};

        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adj_list: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_crates: HashSet<String> = HashSet::new();

        for name in self.source_crates.keys() {
            all_crates.insert(name.clone());
            in_degree.insert(name.clone(), 0);
            adj_list.insert(name.clone(), Vec::new());
        }

        for dep in self.extern_crates() {
            all_crates.insert(dep.name().to_string());
            in_degree.entry(dep.name().to_string()).or_insert(0);
            adj_list.entry(dep.name().to_string()).or_default();
        }

        for (name, source_crate) in &self.source_crates {
            if let Some(ref deps) = source_crate.manifest.dependencies {
                for dep_name in deps.keys() {
                    if all_crates.contains(dep_name) {
                        adj_list.get_mut(dep_name).unwrap().push(name.clone());
                        *in_degree.get_mut(name).unwrap() += 1;
                    }
                }
            }
        }

        let mut queue: VecDeque<String> = VecDeque::new();
        for (name, degree) in &in_degree {
            if *degree == 0 {
                queue.push_back(name.clone());
            }
        }

        let mut result = Vec::new();
        while let Some(node) = queue.pop_front() {
            result.push(node.clone());

            if let Some(neighbors) = adj_list.get(&node) {
                for neighbor in neighbors {
                    if let Some(degree) = in_degree.get_mut(neighbor) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(neighbor.clone());
                        }
                    }
                }
            }
        }

        if result.len() != all_crates.len() {
            return Err("Circular dependency detected in crate graph".to_string());
        }

        Ok(result)
    }
}
