use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use ast::debug::debug_ast;
use diagnostic::Diagnostics;
use inkwell::context::Context;
use inkwell::OptimizationLevel;

use crate::ast::Program;
use crate::crate_system::CrateContext;
pub use crate::lexer::Span;
use crate::products::{
    CompilerProducts, ProductCrateId, ProductCrateIdentity, ProductDefId,
    ProductDependencyIdentity, ProductIdRemap, ProductLinkData, ProductLinkRecord,
    ProductSourceFingerprint,
};

pub mod analysis;
pub mod ast;
pub mod codegen;
pub mod collect;
pub mod crate_artifact;
pub mod crate_system;
pub mod dce;
pub mod diagnostic;
pub mod fmt;
pub mod hir;
pub mod ids;
pub mod infer;
pub mod language_items;
mod lexer;
pub mod lower;
pub mod macro_expansion;
pub mod mir;
pub mod mono;
pub mod parser;
pub mod products;
pub mod selection;
#[cfg(test)]
mod semantic_identity_audit;
pub mod source_loader;
pub mod source_map;
pub mod sysroot;
pub mod traits;
pub mod type_context;
pub(crate) mod type_lowering;
pub mod type_services;
pub mod types;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugPrint {
    Ast,
    AstFull,
    Expanded,
    Tokens,
    Hir,
    Mir,
    Llvm,
}

impl From<&str> for DebugPrint {
    fn from(s: &str) -> Self {
        match s {
            "ast" => DebugPrint::Ast,
            "ast-full" => DebugPrint::AstFull,
            "expanded" => DebugPrint::Expanded,
            "tokens" => DebugPrint::Tokens,
            "hir" => DebugPrint::Hir,
            "mir" => DebugPrint::Mir,
            "llvm" => DebugPrint::Llvm,
            _ => panic!(
                "Unknown debug print: {}\nValid options are: ast, ast-full, expanded, tokens, hir, mir, llvm",
                s
            ),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Config {
    pub entry_file: PathBuf,
    pub output_dir: PathBuf,
    pub debug_print: Vec<DebugPrint>,
    pub meta_files: Vec<(String, PathBuf)>, // crate name, path
    pub extern_artifacts: Vec<(String, PathBuf)>, // External product artifacts: name=path
    pub source_providers: Vec<SourceProvider>, // Explicit non-filesystem source inputs
    pub current_crate_name: Option<String>,
    pub opt_level: u8, // 0-3
    pub emit_llvm: bool,
    pub no_link: bool,
    pub emit_object: Option<PathBuf>,
    pub no_prelude: bool,         // Don't inject stdlib prelude
    pub no_std: bool,             // Don't auto-load bundled stdlib
    pub sysroot: Option<PathBuf>, // Toolchain sysroot override
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceProvider {
    Virtual {
        path: PathBuf,
        text: String,
    },
    Artifact {
        path: PathBuf,
        artifact_path: PathBuf,
        text: String,
    },
}

impl Config {
    pub fn has_debug_print(&self, name: DebugPrint) -> bool {
        self.debug_print.contains(&name)
    }
}

#[derive(Debug, Clone)]
pub struct CompileOutput {
    pub ast: Program,
    pub products: Option<CompilerProducts>,
}

pub fn compile(config: &Config) -> Result<Program, Diagnostics> {
    compile_impl(config, false).map(|output| output.ast)
}

pub fn compile_with_products(config: &Config) -> Result<CompileOutput, Diagnostics> {
    compile_impl(config, true)
}

/// Run the source frontend through finalized HIR without MIR or code generation.
pub fn analyze(config: &Config) -> Result<analysis::Analysis, Diagnostics> {
    compile_frontend(config).map(|frontend| frontend.analysis)
}

struct FrontendOutput {
    analysis: analysis::Analysis,
    crate_ctx: CrateContext,
    diagnostic_sources: diagnostic::DiagnosticSourceMap,
    loaded_prelude_export_ids:
        std::collections::HashMap<String, crate::crate_artifact::ArtifactExport>,
    infix_precedence: std::collections::HashMap<String, u8>,
    fingerprint_source_files: Vec<PathBuf>,
    source_db: source_loader::SourceDatabase,
    effective_current_crate_name: String,
}

fn compile_frontend(config: &Config) -> Result<FrontendOutput, Diagnostics> {
    let effective_current_crate_name = config
        .current_crate_name
        .clone()
        .unwrap_or_else(|| module_name_from_entry(&config.entry_file));

    let mut ctx = CrateContext::new();
    validate_extern_artifact_names(config, &effective_current_crate_name)?;
    if let Err(diagnostic) = load_extern_artifacts(
        &mut ctx,
        &config.extern_artifacts,
        config
            .entry_file
            .parent()
            .unwrap_or(&config.entry_file)
            .to_path_buf(),
    ) {
        let mut diagnostics = Diagnostics::default();
        diagnostics.push(diagnostic);
        return Err(diagnostics);
    }

    let mut source_db = source_loader::SourceDatabase::new();
    register_configured_sources(&mut source_db, config);
    let mut source_graph = match source_db.load_entry(config.entry_file.clone(), config) {
        Ok(graph) => graph,
        Err(errors) => return Err(source_load_errors_to_diagnostics(errors)),
    };
    let mut diagnostic_sources = diagnostic::DiagnosticSourceMap::from_source_database(&source_db);
    let ast = Program {
        module: source_graph.root_module().clone(),
    };

    if config.has_debug_print(DebugPrint::AstFull) {
        println!("{:#?}", ast);
    }
    if config.has_debug_print(DebugPrint::Ast) {
        debug_ast(&ast);
    }

    let mut macro_context = macro_expansion::MacroExpansionContext::new(config);
    for artifact in ctx.proc_macro_artifacts() {
        macro_context = macro_context.with_proc_macro_artifact(artifact.clone());
    }
    let ast = macro_expansion::expand_macros_with_context(ast, &macro_context)
        .map_err(|diagnostics| diagnostics.with_sources(&diagnostic_sources))?;

    if config.has_debug_print(DebugPrint::Expanded) {
        println!("{:#?}", ast);
    }

    if let Err(errors) = source_db.load_module_declarations(
        &mut source_graph,
        &ast.module,
        config.current_crate_name.as_deref(),
        config,
    ) {
        return Err(source_load_errors_to_diagnostics(errors));
    }
    diagnostic_sources = diagnostic::DiagnosticSourceMap::from_source_database(&source_db);

    let declarations = collect::collect_with_source_graph(
        &ast,
        &source_graph,
        &ctx,
        !config.no_prelude,
        config.current_crate_name.as_deref(),
    )
    .map_err(|errors| Diagnostics::from_resolve_errors(errors).with_sources(&diagnostic_sources))?;
    let loaded_prelude_export_ids = declarations.loaded_prelude_export_ids.clone();
    let infix_precedence = declarations.infix_precedence.clone();
    let fingerprint_source_files = source_graph.loaded_files().to_vec();
    let partial_hir = lower::program::lower_from_declarations(
        &ast,
        declarations,
        &ctx,
        config.current_crate_name.as_deref(),
    )
    .map_err(|errors| Diagnostics::from_resolve_errors(errors).with_sources(&diagnostic_sources))?;
    let hir = infer::finalize(partial_hir).map_err(|errors| {
        Diagnostics::from_resolve_errors(errors).with_sources(&diagnostic_sources)
    })?;

    if config.has_debug_print(DebugPrint::Hir) {
        println!("{:#?}", hir.program);
    }

    Ok(FrontendOutput {
        analysis: analysis::Analysis { ast, hir },
        crate_ctx: ctx,
        diagnostic_sources,
        loaded_prelude_export_ids,
        infix_precedence,
        fingerprint_source_files,
        source_db,
        effective_current_crate_name,
    })
}

fn compile_impl(config: &Config, emit_products: bool) -> Result<CompileOutput, Diagnostics> {
    let FrontendOutput {
        analysis: analysis::Analysis { ast, hir },
        crate_ctx: ctx,
        diagnostic_sources,
        loaded_prelude_export_ids,
        infix_precedence,
        fingerprint_source_files,
        source_db,
        effective_current_crate_name,
    } = compile_frontend(config)?;
    let crate_ctx = &ctx;

    let source_fingerprint =
        product_source_fingerprint(config, &source_db, &fingerprint_source_files);
    let current_crate_identity = ProductCrateIdentity::local(effective_current_crate_name.clone())
        .with_source_fingerprint(source_fingerprint.clone());
    let compilation_identity = mono::CompilationIdentityContext::new(
        hir.root_crate_id,
        current_crate_identity.clone(),
        crate_ctx
            .product_crate_ids
            .iter()
            .map(|(identity, crate_id)| (*crate_id, identity.clone()))
            .collect(),
    );

    let (mut products, product_id_remap) = if emit_products {
        let (mut products, product_id_remap) = CompilerProducts::from_resolved_hir_with_remap(
            current_crate_identity,
            &hir,
            product_dependencies_from_config(config),
            product_dependency_crate_identities_from_context(crate_ctx),
            source_fingerprint,
            ProductLinkData::default(),
        )
        .map_err(|error| {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(diagnostic::Diagnostic::for_internal(error));
            diagnostics
        })?;
        if config.current_crate_name.as_deref() == Some("stdlib") {
            products.record_prelude_export_ids(
                loaded_prelude_export_ids
                    .iter()
                    .map(|(alias, export)| (alias.clone(), export.clone())),
            );
        }
        products.infix_precedence = infix_precedence.into_iter().collect();
        (Some(products), Some(product_id_remap))
    } else {
        (None, None)
    };

    // Phase 5: Monomorphization
    let mut monomorphized = mono::monomorphize_with_crates(hir, crate_ctx, compilation_identity)
        .map_err(|diagnostics| diagnostics.with_sources(&diagnostic_sources))?;
    let mut instance_bodies =
        mir::builder::MirBuilder::take_mir_instance_bodies(&mut monomorphized);
    let _dce_report = dce::prune_unreachable_instances(&mut monomorphized, &mut instance_bodies);

    let mir_program = mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(
        &monomorphized,
        &instance_bodies,
    );
    if config.has_debug_print(DebugPrint::Mir) {
        println!("{:#?}", mir_program);
    }
    if let Err(diagnostics) = mir::borrowck::BorrowChecker::run(&mir_program) {
        return Err(diagnostics.with_sources(&diagnostic_sources));
    }
    let agreement = mir::agreement::check_mir_runtime_agreement(&mir_program);
    if !agreement.is_clean() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.push(diagnostic::Diagnostic::for_internal(format!(
            "MIR/codegen agreement failed: {:?}",
            agreement
        )));
        return Err(diagnostics);
    }

    // Phase 6: Code generation
    if let (Some(products), Some(remap)) = (&mut products, product_id_remap.as_ref()) {
        products
            .attach_object_schemas(&mir_program, remap)
            .map_err(|error| Diagnostics(vec![diagnostic::Diagnostic::for_internal(error)]))?;
    }
    let context = Context::create();
    let module_name = module_name_from_entry(&config.entry_file);
    let symbol_namespace = config
        .current_crate_name
        .as_deref()
        .unwrap_or(module_name.as_str());

    let mut codegen =
        codegen::CodeGen::new_with_symbol_namespace(&context, &module_name, symbol_namespace);
    codegen.set_type_context(mir_program.type_context.clone());
    let link_inputs = crate_ctx.dependency_link_inputs();

    if let Err(e) = codegen.compile_program_from_mir(&mir_program) {
        let mut diagnostics = Diagnostics::default();
        diagnostics.push(e.into_diagnostic());
        let diagnostics = diagnostics.with_sources(&diagnostic_sources);
        return Err(diagnostics);
    }
    attach_product_link_records(
        &mut products,
        &mir_program.backend_contract.artifact_exports,
        codegen.mir_contract_symbol_overrides(),
        product_id_remap.as_ref(),
    )
    .map_err(|error| {
        let mut diagnostics = Diagnostics::default();
        diagnostics.push(diagnostic::Diagnostic::for_internal(error));
        diagnostics
    })?;

    if config.has_debug_print(DebugPrint::Llvm) {
        println!("{}", codegen.get_ir());
    }

    // Create output directory
    let _ = std::fs::create_dir_all(&config.output_dir);

    // Write LLVM IR if requested
    if config.emit_llvm {
        let ir_path = config.output_dir.join(format!("{}.ll", module_name));
        if let Err(e) = codegen.write_ir(&ir_path) {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(diagnostic::Diagnostic::for_file(
                format!("Failed to write LLVM IR: {}", e),
                ir_path,
            ));
            return Err(diagnostics);
        }
    }

    // Write executable
    let opt = match config.opt_level {
        0 => OptimizationLevel::None,
        1 => OptimizationLevel::Less,
        2 => OptimizationLevel::Default,
        _ => OptimizationLevel::Aggressive,
    };

    let exe_name = if cfg!(target_os = "windows") {
        format!("{}.exe", module_name)
    } else {
        module_name.to_string()
    };
    let exe_path = config.output_dir.join(&exe_name);

    if config.no_link {
        let obj_path = config
            .emit_object
            .clone()
            .unwrap_or_else(|| config.output_dir.join(format!("{}.o", module_name)));
        if let Some(parent) = obj_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = codegen.write_object(&obj_path, opt) {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(e.into_diagnostic());
            return Err(diagnostics);
        }
        attach_product_object_path(&mut products, &obj_path);
    } else {
        if let Err(e) = codegen.write_executable(&exe_path, opt, &link_inputs.object_paths) {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(e.into_diagnostic());
            return Err(diagnostics);
        }
    }

    Ok(CompileOutput { ast, products })
}

fn register_configured_sources(source_db: &mut source_loader::SourceDatabase, config: &Config) {
    source_db.add_source_providers(&config.source_providers);
}

struct PendingExternArtifact {
    name: String,
    path: PathBuf,
    header: crate::products::ProductArtifactHeader,
    identity: ProductCrateIdentity,
    dependencies: Vec<ProductCrateIdentity>,
}

fn load_extern_artifacts(
    ctx: &mut CrateContext,
    extern_artifacts: &[(String, PathBuf)],
    project_path: PathBuf,
) -> Result<(), diagnostic::Diagnostic> {
    let mut pending = Vec::new();
    for (name, path) in extern_artifacts {
        let header =
            crate::products::ProductArtifactHeader::read_from_path_bounded(path).map_err(|e| {
                diagnostic::Diagnostic::for_artifact(
                    format!(
                        "Failed to read external artifact header '{}' from {}: {}",
                        name,
                        path.display(),
                        e
                    ),
                    path.clone(),
                )
            })?;
        pending.push(PendingExternArtifact {
            name: name.clone(),
            path: path.clone(),
            identity: header.crate_identity.clone(),
            dependencies: header.identity_dependencies.values().cloned().collect(),
            header,
        });
    }

    let mut loaded_identities: BTreeSet<ProductCrateIdentity> =
        ctx.product_crate_ids.keys().cloned().collect();
    let pending_identities: BTreeSet<ProductCrateIdentity> = pending
        .iter()
        .map(|artifact| artifact.identity.clone())
        .collect();
    for artifact in &pending {
        for dependency in &artifact.dependencies {
            if !loaded_identities.contains(dependency) && !pending_identities.contains(dependency) {
                return Err(diagnostic::Diagnostic::for_artifact(
                    format!(
                        "External artifact '{}' depends on missing external artifact '{}'",
                        artifact.name, dependency.name
                    ),
                    artifact.path.clone(),
                ));
            }
        }
    }

    while !pending.is_empty() {
        let Some(index) = pending.iter().position(|artifact| {
            artifact
                .dependencies
                .iter()
                .all(|dependency| loaded_identities.contains(dependency))
        }) else {
            let names = pending
                .iter()
                .map(|artifact| artifact.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(diagnostic::Diagnostic::for_project(
                format!(
                    "Failed to resolve external artifact load order for: {}",
                    names
                ),
                project_path,
            ));
        };

        let artifact = pending.remove(index);
        let mut type_context = crate::type_context::TypeContext::new();
        ctx.load_product_artifact_from_header_as_with_type_context(
            &artifact.name,
            artifact.path.clone(),
            &artifact.header,
            &mut type_context,
        )
        .map_err(|e| {
            diagnostic::Diagnostic::for_artifact(
                format!(
                    "Failed to load external artifact '{}' from {}: {}",
                    artifact.name,
                    artifact.path.display(),
                    e
                ),
                artifact.path.clone(),
            )
        })?;
        loaded_identities.insert(artifact.identity);
    }

    Ok(())
}

fn validate_extern_artifact_names(
    config: &Config,
    effective_current_crate_name: &str,
) -> Result<(), Diagnostics> {
    let mut seen = std::collections::BTreeSet::new();
    for (name, _) in &config.extern_artifacts {
        if effective_current_crate_name == name {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(diagnostic::Diagnostic::for_project(
                format!(
                    "External artifact crate name '{}' conflicts with current crate name '{}'",
                    name, name
                ),
                config.entry_file.clone(),
            ));
            return Err(diagnostics);
        }
        if !seen.insert(name.clone()) {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(diagnostic::Diagnostic::for_project(
                format!("Duplicate external artifact crate name '{}'", name),
                config.entry_file.clone(),
            ));
            return Err(diagnostics);
        }
    }

    Ok(())
}

pub fn source_load_errors_to_diagnostics(
    errors: Vec<source_loader::SourceLoadError>,
) -> Diagnostics {
    let mut diagnostics = Diagnostics::default();
    for error in errors {
        match error {
            source_loader::SourceLoadError::Io { path, message } => {
                diagnostics.push(diagnostic::Diagnostic::for_file(
                    format!("Failed to read source file {}: {}", path.display(), message),
                    path,
                ));
            }
            source_loader::SourceLoadError::MissingModule {
                module,
                searched,
                span,
                parent_source,
            } => {
                let searched_path = searched.first().cloned();
                let searched = searched
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let message = format!("Module '{}' not found; searched: {}", module, searched);
                diagnostics.push(match span {
                    Some(span) => {
                        let mut diagnostic = diagnostic::Diagnostic::new(message, span);
                        if let Some(source) = parent_source {
                            let origin = match source.origin {
                                source_loader::SourceOrigin::FileSystem => {
                                    diagnostic::DiagnosticSourceOrigin::FileSystem
                                }
                                source_loader::SourceOrigin::Virtual => {
                                    diagnostic::DiagnosticSourceOrigin::Virtual
                                }
                                source_loader::SourceOrigin::Artifact { artifact_path } => {
                                    diagnostic::DiagnosticSourceOrigin::Artifact { artifact_path }
                                }
                            };
                            diagnostic.source = Some(diagnostic::DiagnosticSource {
                                display_path: source.display_path,
                                text: source.text,
                                origin,
                                related: std::collections::BTreeMap::new(),
                            });
                        }
                        diagnostic
                    }
                    None => match searched_path {
                        Some(path) => diagnostic::Diagnostic::for_file(message, path),
                        None => diagnostic::Diagnostic::for_toolchain(message),
                    },
                });
            }
            source_loader::SourceLoadError::Parse { error, source, .. } => {
                let mut parse_diagnostics = Diagnostics::from(error);
                if let Some(source) = source {
                    let origin = match source.origin {
                        source_loader::SourceOrigin::FileSystem => {
                            diagnostic::DiagnosticSourceOrigin::FileSystem
                        }
                        source_loader::SourceOrigin::Virtual => {
                            diagnostic::DiagnosticSourceOrigin::Virtual
                        }
                        source_loader::SourceOrigin::Artifact { artifact_path } => {
                            diagnostic::DiagnosticSourceOrigin::Artifact { artifact_path }
                        }
                    };
                    let diagnostic_source = diagnostic::DiagnosticSource {
                        display_path: source.display_path,
                        text: source.text,
                        origin,
                        related: std::collections::BTreeMap::new(),
                    };
                    for diagnostic in &mut parse_diagnostics.0 {
                        if matches!(
                            &diagnostic.location,
                            diagnostic::DiagnosticLocation::Source(_)
                        ) {
                            diagnostic.source = Some(diagnostic_source.clone());
                        }
                    }
                }
                diagnostics.merge(parse_diagnostics);
            }
            source_loader::SourceLoadError::CircularModule { path, stack } => {
                let stack = stack
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ");
                diagnostics.push(diagnostic::Diagnostic::for_file(
                    format!(
                        "Circular module load detected at {} via {}",
                        path.display(),
                        stack
                    ),
                    path,
                ));
            }
        }
    }
    diagnostics
}

fn module_name_from_entry(entry_file: &std::path::Path) -> String {
    entry_file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main")
        .to_string()
}

fn product_dependencies_from_config(config: &Config) -> Vec<ProductDependencyIdentity> {
    config
        .extern_artifacts
        .iter()
        .map(|(name, artifact_path)| {
            ProductDependencyIdentity::artifact_object(
                name.clone(),
                canonical_product_dependency_artifact_path(artifact_path),
            )
        })
        .collect()
}

fn canonical_product_dependency_artifact_path(artifact_path: &Path) -> PathBuf {
    artifact_path
        .canonicalize()
        .unwrap_or_else(|_| absolute_product_dependency_artifact_path(artifact_path))
}

fn absolute_product_dependency_artifact_path(artifact_path: &Path) -> PathBuf {
    if artifact_path.is_absolute() {
        return artifact_path.to_path_buf();
    }

    std::env::current_dir()
        .map(|cwd| cwd.join(artifact_path))
        .unwrap_or_else(|_| artifact_path.to_path_buf())
}

fn product_source_fingerprint(
    config: &Config,
    source_db: &source_loader::SourceDatabase,
    source_files: &[PathBuf],
) -> ProductSourceFingerprint {
    let root = config.entry_file.parent().unwrap_or_else(|| Path::new("."));
    let mut source_files = source_files.to_vec();
    if source_files.is_empty() {
        source_files.push(config.entry_file.clone());
    }

    let source_entries = fingerprint_source_entries(root, &source_files);
    let loaded_files = source_entries
        .iter()
        .map(|(relative_path, _)| relative_path.clone())
        .collect::<Vec<_>>();

    let source_hash = stable_loaded_source_fingerprint(root, &source_entries, Some(source_db));
    let manifest_path = config
        .entry_file
        .parent()
        .map(|parent| parent.join("rock.toml"))
        .filter(|path| path.exists());
    let manifest_hash = manifest_path.as_ref().and_then(|path| {
        let manifest_files = fingerprint_source_entries(root, std::slice::from_ref(path));
        stable_loaded_source_fingerprint(root, &manifest_files, None)
    });

    ProductSourceFingerprint {
        manifest_hash,
        source_hash,
        loaded_files,
    }
}

fn fingerprint_source_entries(root: &Path, paths: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    paths
        .iter()
        .fold(BTreeMap::new(), |mut entries, path| {
            let relative_path = path
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.clone());
            entries.entry(relative_path).or_insert_with(|| path.clone());
            entries
        })
        .into_iter()
        .collect()
}

fn stable_loaded_source_fingerprint(
    root: &Path,
    entries: &[(PathBuf, PathBuf)],
    source_db: Option<&source_loader::SourceDatabase>,
) -> Option<String> {
    if entries.is_empty() {
        return None;
    }

    let mut hash = 0xcbf29ce484222325u64;
    for (relative_path, original_path) in entries {
        hash_bytes(&mut hash, relative_path.to_string_lossy().as_bytes());
        hash_bytes(&mut hash, &[0]);

        if let Some(source_file) = source_db.and_then(|db| db.source_file_for_path(original_path)) {
            hash_bytes(&mut hash, source_file.text.as_bytes());
        } else {
            let read_path = if original_path.is_absolute() {
                original_path.clone()
            } else {
                root.join(relative_path)
            };
            let bytes = fs::read(read_path).ok()?;
            hash_bytes(&mut hash, &bytes);
        }
        hash_bytes(&mut hash, &[0xff]);
    }

    Some(format!("fnv1a64:{hash:016x}"))
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn product_dependency_crate_identities_from_context(
    crate_ctx: &CrateContext,
) -> BTreeMap<ProductCrateId, ProductCrateIdentity> {
    crate_ctx
        .product_crate_ids
        .iter()
        .map(|(identity, crate_id)| (ProductCrateId::from(*crate_id), identity.clone()))
        .collect()
}

fn attach_product_object_path(
    products: &mut Option<CompilerProducts>,
    object_path: &std::path::Path,
) {
    if let Some(products) = products.as_mut() {
        products.link.object_path = Some(object_path.to_path_buf());
    }
}

fn attach_product_link_records(
    products: &mut Option<CompilerProducts>,
    candidates: &[crate::mir::MirArtifactExport],
    symbol_overrides: &std::collections::HashMap<crate::ids::DefId, String>,
    id_remap: Option<&ProductIdRemap>,
) -> Result<(), String> {
    let Some(products) = products.as_mut() else {
        return Ok(());
    };

    for candidate in candidates {
        if candidate.provided_by_object || !candidate.substitution_empty || !candidate.has_body {
            continue;
        }
        if candidate.is_specialization && !candidate.is_drop_glue {
            continue;
        }

        let def_id = candidate
            .origin_def_id
            .ok_or_else(|| "product link export has no origin definition ID".to_string())?;
        let id_remap = id_remap.ok_or_else(|| {
            "product ID remap is required to attach product link records".to_string()
        })?;
        let product_id = product_link_id_for_candidate(products, id_remap, def_id)?;
        let backend_symbol = symbol_overrides
            .get(&def_id)
            .cloned()
            .unwrap_or_else(|| candidate.backend_symbol.clone());
        products
            .link
            .records
            .insert(product_id, ProductLinkRecord { backend_symbol });
    }

    Ok(())
}

fn product_link_id_for_candidate(
    products: &CompilerProducts,
    id_remap: &ProductIdRemap,
    def_id: crate::ids::DefId,
) -> Result<ProductDefId, String> {
    let callable_ids = id_remap
        .get(&ProductDefId::from(def_id))
        .into_iter()
        .flatten()
        .copied()
        .filter(|id| product_id_is_callable(products, *id))
        .collect::<Vec<_>>();

    match callable_ids.as_slice() {
        [id] => Ok(*id),
        [] => Err(format!(
            "no explicit product callable identity for MIR export origin {:?}",
            def_id
        )),
        _ => Err(format!(
            "ambiguous product callable identities for MIR export origin {:?}",
            def_id
        )),
    }
}

fn product_id_is_callable(products: &CompilerProducts, id: ProductDefId) -> bool {
    products.interface.functions.contains_key(&id)
        || products.interface.traits.values().any(|trait_def| {
            trait_def
                .methods
                .values()
                .any(|method| ProductDefId::from(method.id) == id)
        })
        || products.interface.impls.values().any(|imp| {
            imp.methods
                .values()
                .any(|method| ProductDefId::from(method.id) == id)
        })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap},
        fs,
        path::{Component, Path, PathBuf},
    };

    use crate::crate_system::CrateContext;
    use crate::hir::{
        HirBlock, HirExpr, HirExprKind, HirFunction, HirNameTables, HirProgram, HirStmt,
    };
    use crate::ids::{CrateId, DefId, InstanceId, LocalDefId};
    use crate::mono::{InstanceOrigin, InstanceRecord, InstanceSymbols, MonomorphizedProgram};
    use crate::products::{
        CompilerProducts, ProductBodies, ProductCrateId, ProductCrateIdentity, ProductDefId,
        ProductIdRemap, ProductIdentityTable, ProductLinkData, ProductSourceFingerprint,
    };
    use crate::types::Type;
    use crate::{
        attach_product_link_records, compile_with_products,
        product_dependency_crate_identities_from_context, validate_extern_artifact_names, Config,
        SourceProvider,
    };

    fn link_test_function(id: DefId, name: &str) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: vec![HirStmt::Return(Some(HirExpr {
                    kind: HirExprKind::Unit,
                    ty: Type::Unit,
                    span: crate::lexer::Span::test(),
                }))],
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn program(
        functions: HashMap<DefId, HirFunction>,
        names: HirNameTables,
        canonical_names: HashMap<DefId, String>,
    ) -> HirProgram {
        HirProgram::from_id_parts_with_names_and_canonical_names(
            functions,
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            names,
            crate::hir::HirLanguageItems::default(),
            &canonical_names,
        )
    }

    fn link_test_instance<P: crate::hir::HirPhase>(
        id: InstanceId,
        function: &crate::hir::HirFunctionFor<P>,
        name: &str,
    ) -> InstanceRecord {
        InstanceRecord {
            id,
            origin: InstanceOrigin::Function(function.id),
            substitution: Vec::new(),
            symbols: InstanceSymbols::new(name, name),
            declared: None,
            provided_by_object: false,
            is_specialization: false,
        }
    }

    fn accepted_link_test_function(function: HirFunction) -> crate::hir::AcceptedHirFunction {
        let id = function.id;
        let name = function.name.clone();
        let accepted = crate::hir::AcceptedHirProgram::try_from(program(
            HashMap::from([(id, function)]),
            HirNameTables {
                functions_by_name: HashMap::from([(name.clone(), id)]),
                ..HirNameTables::default()
            },
            HashMap::from([(id, name)]),
        ))
        .expect("link fixture HIR is accepted");
        accepted.function_by_id(id).unwrap().1.clone()
    }

    fn link_candidate(
        record: &InstanceRecord,
        bodies: &crate::mir::MirInstanceBodies,
    ) -> crate::mir::MirArtifactExport {
        crate::mir::MirArtifactExport {
            origin_def_id: match &record.origin {
                InstanceOrigin::Function(def_id) => Some(*def_id),
                InstanceOrigin::ImplMethod { method, .. }
                | InstanceOrigin::TraitDefault { method, .. } => Some(*method),
            },
            source_name: record.symbols.source_name.clone(),
            backend_symbol: record.symbols.backend_symbol.clone(),
            substitution_empty: record.substitution.is_empty(),
            has_body: bodies.contains_key(record.id),
            provided_by_object: record.provided_by_object,
            is_specialization: record.is_specialization,
            is_drop_glue: false,
        }
    }

    fn link_test_products(
        functions: Vec<(&str, HirFunction)>,
    ) -> (CompilerProducts, ProductIdRemap) {
        let current_def_ids = functions
            .iter()
            .map(|(_, function)| function.id)
            .collect::<BTreeSet<_>>();
        let mut function_payloads = HashMap::new();
        let mut function_names = HashMap::new();
        let mut canonical_names = HashMap::new();
        for (name, function) in functions {
            function_names.insert(name.to_string(), function.id);
            canonical_names
                .entry(function.id)
                .or_insert_with(|| name.to_string());
            function_payloads.entry(function.id).or_insert(function);
        }
        let program = program(
            function_payloads,
            HirNameTables {
                functions_by_name: function_names,
                ..HirNameTables::default()
            },
            canonical_names,
        );
        let resolved = crate::infer::ResolvedHirProgram::new(
            program,
            crate::collect::resolver::ResolverTables::default(),
            current_def_ids,
            CrateId(0),
            crate::ids::IdGen::default(),
        );
        CompilerProducts::from_resolved_hir_with_remap(
            ProductCrateIdentity::local("app".to_string()),
            &resolved,
            Vec::new(),
            BTreeMap::new(),
            crate::products::ProductSourceFingerprint::default(),
            crate::products::ProductLinkData::default(),
        )
        .expect("test HIR has a valid product language-item registry")
    }

    #[test]
    fn product_link_records_use_callable_id_from_remap_despite_misleading_names() {
        let origin_id = DefId::new(CrateId(0), LocalDefId(40));
        let misleading_id = DefId::new(CrateId(0), LocalDefId(41));
        let non_callable_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(42)));
        let callable_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(43)));
        let (products, _) = link_test_products(vec![
            (
                "misleading",
                link_test_function(misleading_id, "misleading"),
            ),
            (
                "callable",
                link_test_function(DefId::new(CrateId(0), LocalDefId(43)), "callable"),
            ),
        ]);
        let mut products = Some(products);
        let remap: ProductIdRemap = BTreeMap::from([(
            ProductDefId::from(origin_id),
            BTreeSet::from([non_callable_id, callable_id]),
        )]);
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(origin_id),
            source_name: "misleading".to_string(),
            backend_symbol: "callable_symbol".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&remap))
            .expect("callable remap should attach the link record");

        let products = products.expect("products should remain present");
        assert!(products.link.records.contains_key(&callable_id));
        assert!(!products
            .link
            .records
            .contains_key(&ProductDefId::from(misleading_id)));
    }

    #[test]
    fn product_link_records_reject_empty_remap_despite_matching_names() {
        let function_id = DefId::new(CrateId(0), LocalDefId(44));
        let (products, _) = link_test_products(vec![(
            "matching",
            link_test_function(function_id, "matching"),
        )]);
        let mut products = Some(products);
        let remap = ProductIdRemap::default();
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(function_id),
            source_name: "matching".to_string(),
            backend_symbol: "matching".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let error =
            attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&remap))
                .expect_err("an empty remap must not fall back to matching names");

        assert!(error.contains("no explicit product callable identity"));
    }

    #[test]
    fn product_link_records_reject_missing_remap_despite_matching_names() {
        let function_id = DefId::new(CrateId(0), LocalDefId(45));
        let (products, _) = link_test_products(vec![(
            "matching",
            link_test_function(function_id, "matching"),
        )]);
        let mut products = Some(products);
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(function_id),
            source_name: "matching".to_string(),
            backend_symbol: "matching".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let error = attach_product_link_records(&mut products, &[export], &HashMap::new(), None)
            .expect_err("a missing remap must not fall back to matching names");

        assert!(error.contains("product ID remap is required"));
    }

    #[test]
    fn product_link_records_reject_ambiguous_callable_remap() {
        let origin_id = DefId::new(CrateId(0), LocalDefId(46));
        let first_id = DefId::new(CrateId(0), LocalDefId(47));
        let second_id = DefId::new(CrateId(0), LocalDefId(48));
        let (products, _) = link_test_products(vec![
            ("first", link_test_function(first_id, "first")),
            ("second", link_test_function(second_id, "second")),
        ]);
        let mut products = Some(products);
        let remap: ProductIdRemap = BTreeMap::from([(
            ProductDefId::from(origin_id),
            BTreeSet::from([ProductDefId::from(first_id), ProductDefId::from(second_id)]),
        )]);
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(origin_id),
            source_name: "first".to_string(),
            backend_symbol: "ambiguous".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let error =
            attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&remap))
                .expect_err("multiple callable remap targets must be rejected");

        assert!(error.contains("ambiguous product callable identities"));
    }

    #[test]
    fn product_link_records_follow_pruned_instance_set() {
        let main_id = DefId::new(CrateId(0), LocalDefId(1));
        let unused_id = DefId::new(CrateId(0), LocalDefId(2));
        let main_source = link_test_function(main_id, "main");
        let unused_source = link_test_function(unused_id, "unused");
        let main = accepted_link_test_function(main_source.clone());
        let unused = accepted_link_test_function(unused_source.clone());
        let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
        pre_mir_instance_bodies.insert(InstanceId(0), main.clone());
        pre_mir_instance_bodies.insert(InstanceId(1), unused.clone());
        let mut monomorphized = MonomorphizedProgram {
            erased: Default::default(),
            erased_invocations: Default::default(),
            owned_object_calls: Default::default(),
            object_schemas: Default::default(),
            vtables: Default::default(),
            program: crate::hir::HirProgramFor::<crate::hir::AcceptedHir>::from_accepted_id_parts_with_names_and_canonical_names(
                HashMap::from([
                    (main_id, main.clone()),
                    (unused_id, unused.clone()),
                ]),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HirNameTables {
                    functions_by_name: HashMap::from([
                        ("main".to_string(), main_id),
                        ("unused".to_string(), unused_id),
                    ]),
                    ..HirNameTables::default()
                },
                &HashMap::from([
                    (main_id, "main".to_string()),
                    (unused_id, "unused".to_string()),
                ]),
            ),
            instances: BTreeMap::from([
                (
                    InstanceId(0),
                    link_test_instance(InstanceId(0), &main, "main"),
                ),
                (
                    InstanceId(1),
                    link_test_instance(InstanceId(1), &unused, "unused"),
                ),
            ]),
            pre_mir_instance_bodies,
            generated_drop_instances: Default::default(),
            type_context: crate::type_context::TypeContext::new(),
            source_map: crate::source_map::SemanticSourceMap::default(),
        };
        let mut instance_bodies =
            crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut monomorphized);
        crate::dce::prune_unreachable_instances(&mut monomorphized, &mut instance_bodies);
        let (products, id_remap) =
            link_test_products(vec![("main", main_source), ("unused", unused_source)]);
        let mut products = Some(products);
        let candidates = monomorphized
            .instances
            .values()
            .map(|record| link_candidate(record, &instance_bodies))
            .collect::<Vec<_>>();

        attach_product_link_records(&mut products, &candidates, &HashMap::new(), Some(&id_remap))
            .expect("pruned candidates should retain one callable remap target");

        let products = products.expect("products should be present");
        assert!(products
            .link
            .records
            .contains_key(&ProductDefId::from(main_id)));
        assert!(!products
            .link
            .records
            .contains_key(&ProductDefId::from(unused_id)));
    }

    #[test]
    fn product_link_records_ignore_body_only_function_rows() {
        let function_id = DefId::new(CrateId(0), LocalDefId(7));
        let product_id = ProductDefId::from(function_id);
        let function = link_test_function(function_id, "ghost");
        let function = accepted_link_test_function(function);
        let mut bodies = ProductBodies::default();
        bodies.functions.insert(product_id, function);
        let mut products = Some(CompilerProducts {
            crate_identity: ProductCrateIdentity::local("app".to_string()),
            identity_table: ProductIdentityTable {
                local_crate: Some(ProductCrateId(0)),
                display_names: BTreeMap::from([(product_id, "ghost".to_string())]),
                export_names: BTreeMap::from([("ghost".to_string(), product_id)]),
                ..ProductIdentityTable::default()
            },
            interface: crate::products::ProductInterface::default(),
            bodies,
            link: ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        });
        let candidate = crate::mir::MirArtifactExport {
            origin_def_id: Some(function_id),
            source_name: "ghost".to_string(),
            backend_symbol: "ghost".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let id_remap = BTreeMap::from([(
            ProductDefId::from(function_id),
            BTreeSet::from([product_id]),
        )]);
        let error = attach_product_link_records(
            &mut products,
            &[candidate],
            &HashMap::new(),
            Some(&id_remap),
        )
        .expect_err("body-only rows must not become callable link records");

        assert!(error.contains("no explicit product callable identity"));

        let products = products.expect("products should remain present");
        assert!(!products.link.records.contains_key(&product_id));
    }

    #[test]
    fn product_link_records_ignore_body_only_trait_default_rows() {
        let method_id = DefId::new(CrateId(0), LocalDefId(8));
        let product_id = ProductDefId::from(method_id);
        let method = link_test_function(method_id, "show");
        let method = accepted_link_test_function(method);
        let mut bodies = ProductBodies::default();
        bodies.trait_default_methods.insert(product_id, method);
        let mut products = Some(CompilerProducts {
            crate_identity: ProductCrateIdentity::local("app".to_string()),
            identity_table: ProductIdentityTable {
                local_crate: Some(ProductCrateId(0)),
                display_names: BTreeMap::from([(product_id, "Show::show".to_string())]),
                export_names: BTreeMap::from([("Show::show".to_string(), product_id)]),
                ..ProductIdentityTable::default()
            },
            interface: crate::products::ProductInterface::default(),
            bodies,
            link: ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        });
        let candidate = crate::mir::MirArtifactExport {
            origin_def_id: Some(method_id),
            source_name: "Show::show".to_string(),
            backend_symbol: "Show_show".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let id_remap =
            BTreeMap::from([(ProductDefId::from(method_id), BTreeSet::from([product_id]))]);
        let error = attach_product_link_records(
            &mut products,
            &[candidate],
            &HashMap::new(),
            Some(&id_remap),
        )
        .expect_err("body-only trait default rows must not become callable link records");

        assert!(error.contains("no explicit product callable identity"));

        let products = products.expect("products should remain present");
        assert!(!products.link.records.contains_key(&product_id));
    }

    #[test]
    fn product_link_records_filter_non_drop_glue_specialization_artifact_exports() {
        let function_id = DefId::new(CrateId(0), LocalDefId(9));
        let product_id = ProductDefId::from(function_id);
        let function = link_test_function(function_id, "make");
        let (products, id_remap) = link_test_products(vec![("make", function)]);
        let mut products = Some(products);
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(function_id),
            source_name: "make".to_string(),
            backend_symbol: "make__i64".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: true,
            is_drop_glue: false,
        };

        attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&id_remap))
            .expect("non-drop-glue specializations are ineligible");

        let products = products.expect("products should remain present");
        assert!(!products.link.records.contains_key(&product_id));
    }

    #[test]
    fn product_link_records_export_drop_glue_specialization_artifact_exports() {
        let function_id = DefId::new(CrateId(0), LocalDefId(10));
        let product_id = ProductDefId::from(function_id);
        let function = link_test_function(function_id, "drop_box");
        let (products, id_remap) = link_test_products(vec![("drop_box", function)]);
        let mut products = Some(products);
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(function_id),
            source_name: "drop_box".to_string(),
            backend_symbol: "Box_drop".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: true,
            is_drop_glue: true,
        };

        attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&id_remap))
            .expect("drop-glue specializations should retain one callable remap target");

        let products = products.expect("products should remain present");
        assert_eq!(
            products
                .link
                .records
                .get(&product_id)
                .map(|record| record.backend_symbol.as_str()),
            Some("Box_drop")
        );
    }

    #[test]
    fn product_link_records_do_not_match_backend_symbols_from_display_names() {
        let function_id = DefId::new(CrateId(0), LocalDefId(20));
        let product_id = ProductDefId::from(function_id);
        let function = link_test_function(function_id, "answer");
        let (products, id_remap) = link_test_products(vec![("answer", function)]);
        let mut products = Some(products);
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(DefId::new(CrateId(0), LocalDefId(99))),
            source_name: "not_answer".to_string(),
            backend_symbol: "answer".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let error =
            attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&id_remap))
                .expect_err("unmapped origins must not use a display-name fallback");

        assert!(error.contains("no explicit product callable identity"));

        let products = products.expect("products should remain present");
        assert!(!products.link.records.contains_key(&product_id));
    }

    #[test]
    fn product_link_records_use_origin_id_for_impl_methods_without_name_match() {
        let impl_id = DefId::new(CrateId(0), LocalDefId(30));
        let method_id = DefId::new(CrateId(0), LocalDefId(31));
        let method_product_id = ProductDefId::from(method_id);
        let mut method = link_test_function(method_id, "get");
        method.is_method = true;

        let mut products = Some(CompilerProducts {
            crate_identity: ProductCrateIdentity::local("app".to_string()),
            identity_table: ProductIdentityTable {
                local_crate: Some(ProductCrateId(0)),
                display_names: BTreeMap::from([(
                    method_product_id,
                    "impl Value for Box::get".to_string(),
                )]),
                ..ProductIdentityTable::default()
            },
            interface: crate::products::ProductInterface {
                impls: BTreeMap::from([(
                    ProductDefId::from(impl_id),
                    crate::products::ProductImplInterface {
                        id: impl_id,
                        owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                        type_name: "Box".to_string(),
                        type_generics: Vec::new(),
                        receiver_pattern: Vec::new().into(),
                        trait_name: Some("Value".to_string()),
                        trait_id: None,
                        trait_generics: Vec::new(),
                        trait_arg_types: Vec::new(),
                        associated_types: Vec::new(),
                        bounds: HashMap::new().into(),
                        methods: BTreeMap::from([(
                            "get".to_string(),
                            crate::products::ProductFunctionInterface::from(&method),
                        )]),
                    },
                )]),
                ..crate::products::ProductInterface::default()
            },
            bodies: ProductBodies::default(),
            link: ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        });
        let export = crate::mir::MirArtifactExport {
            origin_def_id: Some(method_id),
            source_name: "Box::get".to_string(),
            backend_symbol: "Box_get".to_string(),
            substitution_empty: true,
            has_body: true,
            provided_by_object: false,
            is_specialization: false,
            is_drop_glue: false,
        };

        let id_remap = BTreeMap::from([(
            ProductDefId::from(method_id),
            BTreeSet::from([method_product_id]),
        )]);
        attach_product_link_records(&mut products, &[export], &HashMap::new(), Some(&id_remap))
            .expect("impl method remap should identify the callable method");

        let products = products.expect("products should remain present");
        assert_eq!(
            products
                .link
                .records
                .get(&method_product_id)
                .map(|record| record.backend_symbol.as_str()),
            Some("Box_get")
        );
    }

    #[test]
    fn compile_with_products_extracts_ids_from_normal_pipeline() {
        let base = std::env::temp_dir().join(format!(
            "rock_compile_products_{}_{}",
            std::process::id(),
            "normal_pipeline"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        fs::write(&entry_file, "main = ->\n    0\n").unwrap();

        let output = compile_with_products(&Config {
            entry_file,
            output_dir: base.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap();

        let products = output.products.expect("products should be emitted");
        let main_id = products
            .identity_table
            .export_names
            .get("main")
            .copied()
            .expect("main should have a product ID");

        assert!(products.interface.functions.contains_key(&main_id));
        assert_eq!(
            products.identity_table.display_names.get(&main_id),
            Some(&"main".to_string())
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn compile_with_products_records_explicit_object_output_path() {
        let base = std::env::temp_dir().join(format!(
            "rock_compile_products_{}_{}",
            std::process::id(),
            "object_output"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let object_path = base.join("custom-main.o");
        fs::write(&entry_file, "main = ->\n    0\n").unwrap();

        let output = compile_with_products(&Config {
            entry_file,
            output_dir: base.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            emit_object: Some(object_path.clone()),
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap();

        let products = output.products.expect("products should be emitted");
        assert!(object_path.exists());
        assert_eq!(products.link.object_path, Some(object_path));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn compile_with_products_records_object_backend_symbols() {
        let base = std::env::temp_dir().join(format!(
            "rock_compile_products_{}_{}",
            std::process::id(),
            "object_backend_symbols"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        fs::write(
            &entry_file,
            "helper = ->\n    1\n\nmain = ->\n    helper!\n",
        )
        .unwrap();

        let output = compile_with_products(&Config {
            entry_file,
            output_dir: base.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap();

        let products = output.products.expect("products should be emitted");
        let helper_id = products
            .identity_table
            .export_names
            .get("helper")
            .copied()
            .expect("helper should have a product ID");
        let record = products
            .link
            .records
            .get(&helper_id)
            .expect("helper should have a link record");
        assert!(record.backend_symbol.starts_with("__rock_"));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn compile_with_products_records_canonical_extern_artifact_paths() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let base = workspace_root.join("target").join("tmp").join(format!(
            "rock_compile_products_{}_{}",
            std::process::id(),
            "canonical_extern_artifact"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let dep_artifact = base.join("dep.rkca");
        let dep_object = base.join("dep.o");
        fs::write(&entry_file, "main = ->\n    0\n").unwrap();
        fs::write(&dep_object, []).unwrap();

        CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table: ProductIdentityTable {
                local_crate: Some(ProductCrateId(0)),
                ..ProductIdentityTable::default()
            },
            interface: crate::products::ProductInterface::default(),
            bodies: ProductBodies::default(),
            link: ProductLinkData {
                object_path: Some(dep_object.clone()),
                ..ProductLinkData::default()
            },
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        }
        .write_artifact_to_path(&dep_artifact)
        .unwrap();

        let relative_dep_artifact =
            relative_workspace_path_from_cwd(&dep_artifact, &workspace_root);
        assert!(!relative_dep_artifact.is_absolute());
        let output = compile_with_products(&Config {
            entry_file,
            output_dir: base.join("build"),
            extern_artifacts: vec![("dep".to_string(), relative_dep_artifact)],
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap();

        let products = output.products.expect("products should be emitted");
        let dependency_path = &products.dependencies[0].artifact_path;
        assert!(dependency_path.is_absolute());
        assert_eq!(dependency_path, &dep_artifact.canonicalize().unwrap());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn compile_with_artifact_source_providers_loads_module_declarations_through_source_database() {
        let base = std::env::temp_dir().join(format!(
            "rock_compile_products_{}_{}",
            std::process::id(),
            "artifact_source_providers"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let artifact_path = base.join("demo.rkca");
        let entry_file = PathBuf::from("/artifact/demo/main.rk");
        let util_file = PathBuf::from("/artifact/demo/util.rk");

        let output = compile_with_products(&Config {
            entry_file: entry_file.clone(),
            output_dir: base.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            source_providers: vec![
                SourceProvider::Artifact {
                    path: entry_file,
                    artifact_path: artifact_path.clone(),
                    text: "mod util\n> util::answer\nmain = -> answer!\n".to_string(),
                },
                SourceProvider::Artifact {
                    path: util_file,
                    artifact_path,
                    text: "answer = -> 0\n< answer\n".to_string(),
                },
            ],
            ..Config::default()
        })
        .unwrap();

        let products = output.products.expect("products should be emitted");
        assert_eq!(
            products.crate_identity.source_fingerprint.loaded_files,
            vec![PathBuf::from("main.rk"), PathBuf::from("util.rk")]
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn compile_with_virtual_source_providers_loads_siblings_without_filesystem_probe() {
        let base = std::env::temp_dir().join(format!(
            "rock_compile_products_{}_{}",
            std::process::id(),
            "virtual_source_providers"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let entry_file = base.join("main.rk");
        let util_file = base.join("util.rk");
        let output = compile_with_products(&Config {
            entry_file: entry_file.clone(),
            output_dir: base.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            source_providers: vec![
                SourceProvider::Virtual {
                    path: entry_file.clone(),
                    text: "mod util\n> util::answer\nmain = -> answer!\n".to_string(),
                },
                SourceProvider::Virtual {
                    path: util_file,
                    text: "answer = -> 7\n< answer\n".to_string(),
                },
            ],
            ..Config::default()
        })
        .expect("virtual provider-backed compile should not require filesystem sources");

        let products = output.products.expect("products should be emitted");
        assert_eq!(
            products.crate_identity.source_fingerprint.loaded_files,
            vec![PathBuf::from("main.rk"), PathBuf::from("util.rk")]
        );
        assert!(
            !entry_file.exists(),
            "virtual source compile must not create or read an entry file"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn artifact_source_provider_text_contributes_to_product_source_hash() {
        let base = std::env::temp_dir().join(format!(
            "rock_compile_products_{}_{}",
            std::process::id(),
            "artifact_provider_hash"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let first = compile_artifact_provider_fingerprint(&base, "1");
        let second = compile_artifact_provider_fingerprint(&base, "2");

        assert!(first.source_hash.is_some());
        assert!(second.source_hash.is_some());
        assert_ne!(first.source_hash, second.source_hash);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn artifact_source_provider_parse_diagnostic_preserves_source_metadata() {
        let base = std::env::temp_dir().join(format!(
            "rock_compile_products_{}_{}",
            std::process::id(),
            "artifact_provider_diagnostic"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let artifact_path = base.join("demo.rkca");
        let entry_file = PathBuf::from("/artifact/demo/main.rk");
        let source_text = "main = -> $\n";
        let diagnostics = compile_with_products(&Config {
            entry_file: entry_file.clone(),
            output_dir: base.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            source_providers: vec![SourceProvider::Artifact {
                path: entry_file.clone(),
                artifact_path: artifact_path.clone(),
                text: source_text.to_string(),
            }],
            ..Config::default()
        })
        .unwrap_err();

        let diagnostic_source = diagnostics.0[0]
            .source
            .as_ref()
            .expect("provider parse diagnostic should retain source metadata");
        assert_eq!(diagnostic_source.display_path, entry_file);
        assert_eq!(diagnostic_source.text, source_text);
        assert_eq!(
            diagnostic_source.origin,
            crate::diagnostic::DiagnosticSourceOrigin::Artifact { artifact_path }
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn virtual_source_provider_parse_diagnostic_preserves_source_metadata() {
        let entry_file = PathBuf::from("/virtual/demo/main.rk");
        let source_text = "main = -> $\n";
        let diagnostics = compile_with_products(&Config {
            entry_file: entry_file.clone(),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            source_providers: vec![SourceProvider::Virtual {
                path: entry_file.clone(),
                text: source_text.to_string(),
            }],
            ..Config::default()
        })
        .unwrap_err();

        let diagnostic_source = diagnostics.0[0]
            .source
            .as_ref()
            .expect("virtual provider parse diagnostic should retain source metadata");
        assert_eq!(diagnostic_source.display_path, entry_file);
        assert_eq!(diagnostic_source.text, source_text);
        assert_eq!(
            diagnostic_source.origin,
            crate::diagnostic::DiagnosticSourceOrigin::Virtual
        );
    }

    fn compile_artifact_provider_fingerprint(
        base: &Path,
        answer: &str,
    ) -> ProductSourceFingerprint {
        let artifact_path = base.join("demo.rkca");
        let entry_file = PathBuf::from("/artifact/demo/main.rk");
        let util_file = PathBuf::from("/artifact/demo/util.rk");

        compile_with_products(&Config {
            entry_file: entry_file.clone(),
            output_dir: base.join(format!("build-{answer}")),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            source_providers: vec![
                SourceProvider::Artifact {
                    path: entry_file,
                    artifact_path: artifact_path.clone(),
                    text: "mod util\nmain = -> util::answer!\n".to_string(),
                },
                SourceProvider::Artifact {
                    path: util_file,
                    artifact_path,
                    text: format!("answer = -> {answer}\n< answer\n"),
                },
            ],
            ..Config::default()
        })
        .unwrap()
        .products
        .unwrap()
        .crate_identity
        .source_fingerprint
    }

    fn relative_workspace_path_from_cwd(path: &Path, workspace_root: &Path) -> PathBuf {
        let cwd = std::env::current_dir().unwrap();
        if let Ok(relative) = path.strip_prefix(&cwd) {
            return relative.to_path_buf();
        }

        let Ok(cwd_relative) = cwd.strip_prefix(workspace_root) else {
            return path.to_path_buf();
        };
        let Ok(path_relative) = path.strip_prefix(workspace_root) else {
            return path.to_path_buf();
        };

        let mut relative = PathBuf::new();
        for component in cwd_relative.components() {
            if matches!(component, Component::Normal(_)) {
                relative.push("..");
            }
        }
        relative.push(path_relative);
        relative
    }

    #[test]
    fn compile_rejects_duplicate_extern_artifact_names() {
        let config = Config {
            entry_file: PathBuf::from("main.rk"),
            extern_artifacts: vec![
                ("dep".to_string(), "first.rkca".into()),
                ("dep".to_string(), "second.rkca".into()),
            ],
            ..Config::default()
        };

        let error = validate_extern_artifact_names(&config, "main").unwrap_err();
        assert!(error.0.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("Duplicate external artifact crate name 'dep'")
        }));
    }

    #[test]
    fn compile_rejects_extern_artifact_named_after_current_crate_before_load() {
        let config = Config {
            entry_file: PathBuf::from("missing-current-source.rk"),
            current_crate_name: Some("app".to_string()),
            extern_artifacts: vec![("app".to_string(), PathBuf::from("missing-self.rkca"))],
            ..Config::default()
        };

        let error =
            crate::compile(&config).expect_err("current-name artifact alias must be rejected");

        assert_eq!(error.0.len(), 1);
        assert_eq!(
            error.0[0].message,
            "External artifact crate name 'app' conflicts with current crate name 'app'",
        );
    }

    #[test]
    fn compile_rejects_extern_artifact_named_after_entry_stem_before_load() {
        let config = Config {
            entry_file: PathBuf::from("app.rk"),
            extern_artifacts: vec![("app".to_string(), PathBuf::from("missing-self.rkca"))],
            ..Config::default()
        };

        let error = crate::compile(&config)
            .expect_err("entry-stem artifact alias must be rejected before artifact I/O");

        assert_eq!(error.0.len(), 1);
        assert_eq!(
            error.0[0].message,
            "External artifact crate name 'app' conflicts with current crate name 'app'",
        );
    }

    #[test]
    fn validate_extern_artifact_names_accepts_nonmatching_entry_stem_alias() {
        let config = Config {
            entry_file: PathBuf::from("app.rk"),
            extern_artifacts: vec![("dep".to_string(), PathBuf::from("missing-dep.rkca"))],
            ..Config::default()
        };

        validate_extern_artifact_names(&config, "app")
            .expect("nonmatching artifact alias is valid");
    }

    #[test]
    fn load_extern_artifacts_rejects_missing_declared_dependency() {
        let base = std::env::temp_dir().join(format!(
            "rock_missing_extern_dependency_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let artifact_path = base.join("app.rkca");
        let object_path = base.join("app.o");
        fs::write(&object_path, b"").unwrap();

        let missing_identity = ProductCrateIdentity::local("missing".to_string());
        let products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("app".to_string()),
            identity_table: ProductIdentityTable {
                local_crate: Some(ProductCrateId(0)),
                dependencies: BTreeMap::from([(ProductCrateId(1), missing_identity)]),
                ..ProductIdentityTable::default()
            },
            interface: crate::products::ProductInterface::default(),
            bodies: ProductBodies::default(),
            link: ProductLinkData {
                object_path: Some(object_path),
                ..ProductLinkData::default()
            },
            dependencies: vec![],
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let error = super::load_extern_artifacts(
            &mut CrateContext::new(),
            &[("app".to_string(), artifact_path)],
            base.clone(),
        )
        .unwrap_err();

        assert!(
            error.message.contains("missing"),
            "unexpected error: {error:?}"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn product_dependency_crate_identities_from_context_preserves_real_artifact_identities() {
        let mut crate_ctx = CrateContext::new();
        let linux_identity = ProductCrateIdentity {
            name: "dep".to_string(),
            version: "1.0.0".to_string(),
            target_triple: Some("x86_64-unknown-linux-gnu".to_string()),
            format_version: 5,
            source_fingerprint: ProductSourceFingerprint::default(),
        };
        let wasm_identity = ProductCrateIdentity {
            name: "dep".to_string(),
            version: "2.0.0".to_string(),
            target_triple: Some("wasm32-unknown-unknown".to_string()),
            format_version: 6,
            source_fingerprint: ProductSourceFingerprint::default(),
        };
        crate_ctx
            .product_crate_ids
            .insert(linux_identity.clone(), CrateId(7));
        crate_ctx
            .product_crate_ids
            .insert(wasm_identity.clone(), CrateId(42));

        let identities = product_dependency_crate_identities_from_context(&crate_ctx);

        assert_eq!(
            identities,
            BTreeMap::from([
                (ProductCrateId(7), linux_identity),
                (ProductCrateId(42), wasm_identity),
            ])
        );
    }

    #[test]
    fn product_source_fingerprint_is_stable_across_checkout_paths() {
        let first =
            std::env::temp_dir().join(format!("rock_fingerprint_first_{}", std::process::id()));
        let second =
            std::env::temp_dir().join(format!("rock_fingerprint_second_{}", std::process::id()));
        let _ = fs::remove_dir_all(&first);
        let _ = fs::remove_dir_all(&second);
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("main.rk"), "main = ->\n    0\n").unwrap();
        fs::write(second.join("main.rk"), "main = ->\n    0\n").unwrap();

        let first_products = compile_with_products(&Config {
            entry_file: first.join("main.rk"),
            output_dir: first.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap()
        .products
        .unwrap();
        let second_products = compile_with_products(&Config {
            entry_file: second.join("main.rk"),
            output_dir: second.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap()
        .products
        .unwrap();

        assert_eq!(
            first_products.crate_identity.source_fingerprint,
            second_products.crate_identity.source_fingerprint
        );
        assert_eq!(
            first_products
                .crate_identity
                .source_fingerprint
                .loaded_files,
            vec![PathBuf::from("main.rk")]
        );

        let _ = fs::remove_dir_all(&first);
        let _ = fs::remove_dir_all(&second);
    }

    #[test]
    fn product_source_fingerprint_ignores_unloaded_rock_files() {
        let base =
            std::env::temp_dir().join(format!("rock_fingerprint_unloaded_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        fs::write(&entry_file, "main = ->\n    0\n").unwrap();

        let before = compile_with_products(&Config {
            entry_file: entry_file.clone(),
            output_dir: base.join("build-before"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap()
        .products
        .unwrap()
        .crate_identity
        .source_fingerprint;

        fs::write(base.join("unused.rk"), "unused = ->\n    1\n").unwrap();

        let after = compile_with_products(&Config {
            entry_file,
            output_dir: base.join("build-after"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap()
        .products
        .unwrap()
        .crate_identity
        .source_fingerprint;

        assert_eq!(before, after);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn product_source_fingerprint_uses_source_loader_loaded_files() {
        let base = std::env::temp_dir().join(format!(
            "rock_fingerprint_loaded_modules_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        fs::write(&entry_file, "mod util\n> util::answer\nmain = -> answer!\n").unwrap();
        fs::write(base.join("util.rk"), "answer = -> 0\n< answer\n").unwrap();
        fs::write(base.join("unloaded.rk"), "answer = -> 1\n").unwrap();

        let output = compile_with_products(&Config {
            entry_file,
            output_dir: base.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap();

        let products = output.products.expect("products should be emitted");
        assert_eq!(
            products.crate_identity.source_fingerprint.loaded_files,
            vec![PathBuf::from("main.rk"), PathBuf::from("util.rk")]
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn product_source_fingerprint_includes_macro_generated_source_modules() {
        let base = std::env::temp_dir().join(format!(
            "rock_fingerprint_macro_modules_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        fs::write(
            &entry_file,
            "macro include_util\n    x =>\n        mod util\n%include_util x\n\nmain = ->\n    0\n",
        )
        .unwrap();
        fs::write(base.join("util.rk"), "answer = -> 0\n< answer\n").unwrap();
        fs::write(base.join("unloaded.rk"), "answer = -> 1\n").unwrap();

        let output = compile_with_products(&Config {
            entry_file,
            output_dir: base.join("build"),
            no_std: true,
            no_prelude: true,
            no_link: true,
            current_crate_name: Some("demo".to_string()),
            ..Config::default()
        })
        .unwrap();

        let products = output.products.expect("products should be emitted");
        assert_eq!(
            products.crate_identity.source_fingerprint.loaded_files,
            vec![PathBuf::from("main.rk"), PathBuf::from("util.rk")]
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn source_load_conversion_attaches_text_only_to_source_diagnostics() {
        let path = PathBuf::from("/virtual/loader.rk");
        let source = crate::source_loader::SourceFile {
            original_path: path.clone(),
            canonical_path: path.clone(),
            display_path: path.clone(),
            text: "source".to_string(),
            origin: crate::source_loader::SourceOrigin::Virtual,
        };
        let diagnostics = super::source_load_errors_to_diagnostics(vec![
            crate::source_loader::SourceLoadError::Parse {
                path,
                source: Some(source),
                error: crate::parser::ParseError::Fail,
            },
        ]);

        assert_eq!(diagnostics.0.len(), 1);
        assert!(matches!(
            &diagnostics.0[0].location,
            crate::diagnostic::DiagnosticLocation::Toolchain
        ));
        assert!(diagnostics.0[0].source.is_none());
    }
}
