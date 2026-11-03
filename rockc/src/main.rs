use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use clap::{Parser, ValueEnum};
use rock_lib::{
    ast::{
        visit::{walk_module, Visitor},
        Module, Program,
    },
    diagnostic::Diagnostics,
    products::{
        PortableProductArtifact, ProductArtifactHeader, ProductCrateIdentity,
        ProductDependencyCapabilities, ProductDependencyIdentity, ProductDependencyLinkCapability,
    },
    DebugPrint,
};

fn main() {
    if let Err(e) = run() {
        e.report();

        std::process::exit(1);
    }
}

fn run() -> Result<(), Diagnostics> {
    run_config(Config::parse())
}

fn run_config(config: Config) -> Result<(), Diagnostics> {
    if let Some(output) = print_request_output(&config)? {
        println!("{}", output);
        return Ok(());
    }

    if config.format {
        return format_config(config);
    }

    if config.expand {
        return expand_config(config);
    }

    if let Some(path) = &config.validate_artifact {
        return validate_artifact(path);
    }

    let emit_artifact = config.emit_artifact.clone();
    let compiler_config = config.into_compiler_config()?;
    if emit_artifact.is_some() && !compiler_config.no_link {
        return Err(diagnostics_from_message(
            "--emit-artifact requires --no-link so the product artifact records an object file",
        ));
    }

    if let Some(path) = emit_artifact {
        let output = rock_lib::compile_with_products(&compiler_config)?;
        let mut products = output.products.ok_or_else(|| {
            diagnostics_from_message("Compiler did not produce product data for artifact emission")
        })?;
        normalize_product_object_path(&path, &mut products);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|err| {
                diagnostics_from_message(format!(
                    "Failed to create artifact directory {}: {}",
                    parent.display(),
                    err
                ))
            })?;
        }
        products
            .write_artifact_to_path(&path)
            .map_err(diagnostics_from_message)?;
    } else {
        rock_lib::compile(&compiler_config)?;
    }

    Ok(())
}

fn normalize_product_object_path(
    artifact_path: &std::path::Path,
    products: &mut rock_lib::products::CompilerProducts,
) {
    let Some(object_path) = products.link.object_path.as_ref() else {
        return;
    };
    let Some(object_file_name) = object_path.file_name() else {
        return;
    };
    let Some(artifact_parent) = artifact_path.parent() else {
        return;
    };
    let Some(object_parent) = object_path.parent() else {
        return;
    };

    if same_directory(artifact_parent, object_parent) {
        products.link.object_path = Some(PathBuf::from(object_file_name));
        return;
    }

    if let Some(sibling_root) = artifact_parent.parent() {
        if let Some(relative_path) = relative_to_root(object_path, sibling_root) {
            products.link.object_path = Some(relative_path);
        }
    }
}

fn relative_to_root(path: &std::path::Path, root: &std::path::Path) -> Option<PathBuf> {
    if let Ok(relative) = path.strip_prefix(root) {
        if !relative.as_os_str().is_empty() {
            return Some(relative.to_path_buf());
        }
    }

    let path = path.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    path.strip_prefix(root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
}

fn same_directory(left: &std::path::Path, right: &std::path::Path) -> bool {
    if left == right {
        return true;
    }

    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum PrintValue {
    ArtifactDeps,
    Sysroot,
    TargetLibdir,
    TargetTriple,
}

#[derive(Parser, Debug)]
#[command(version, about = "Rock compiler", long_about = None)]
pub struct Config {
    #[arg(long)]
    entry_file: Option<PathBuf>,
    #[arg(long, default_value = "build")]
    output_dir: PathBuf,
    #[arg(long, default_value = None)]
    debug_print: Option<String>,
    #[arg(long, value_enum)]
    print: Option<PrintValue>,
    /// Crate name for product identity and codegen symbol qualification
    #[arg(long)]
    crate_name: Option<String>,
    /// Build without optimization (default)
    #[arg(long, conflicts_with = "release")]
    debug: bool,
    /// Build with full optimization
    #[arg(long, conflicts_with = "debug")]
    release: bool,
    /// Override integer overflow checks (true/false), independently of optimization.
    #[arg(long, action = clap::ArgAction::Set)]
    overflow_checks: Option<bool>,
    /// Emit LLVM IR file
    #[arg(long)]
    emit_llvm: bool,
    /// Don't link, just produce object file
    #[arg(long)]
    no_link: bool,
    /// Emit product-backed compiler artifact to this path
    #[arg(long)]
    emit_artifact: Option<PathBuf>,
    /// Emit object file to this exact path
    #[arg(long)]
    emit_object: Option<PathBuf>,
    /// Don't inject std prelude
    #[arg(long)]
    no_prelude: bool,
    /// Don't auto-load bundled std from the sysroot
    #[arg(long)]
    no_std: bool,
    /// Override the compiler sysroot
    #[arg(long)]
    sysroot: Option<PathBuf>,
    /// External product artifacts in name=path format
    #[arg(long)]
    extern_artifact: Vec<String>,
    /// Format parsed modules in place and print them
    #[arg(long)]
    format: bool,
    /// Expand macros, print modules, and stop before lowering/compilation
    #[arg(long)]
    expand: bool,
    /// Validate a product artifact can be loaded and its object path exists
    #[arg(long)]
    validate_artifact: Option<PathBuf>,
}

impl Config {
    fn into_compiler_config(self) -> Result<rock_lib::Config, Diagnostics> {
        let config = self;
        let entry_file = config.entry_file.ok_or_else(|| {
            diagnostics_from_message("Missing required --entry-file unless --print is used")
        })?;

        let extern_artifacts = config
            .extern_artifact
            .iter()
            .map(|s| parse_name_path(s, "extern-artifact"))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rock_lib::Config {
            entry_file,
            output_dir: config.output_dir,
            debug_print: config
                .debug_print
                .map(|s| s.split(',').map(DebugPrint::from).collect())
                .unwrap_or_default(),
            meta_files: Vec::new(),
            extern_artifacts,
            source_providers: Vec::new(),
            current_crate_name: config.crate_name,
            opt_level: if config.release { 3 } else { 0 },
            overflow_checks: config.overflow_checks,
            emit_llvm: config.emit_llvm,
            no_link: config.no_link,
            emit_object: config.emit_object,
            no_prelude: config.no_prelude,
            no_std: config.no_std,
            sysroot: config.sysroot,
        })
    }
}

fn print_request_output(config: &Config) -> Result<Option<String>, Diagnostics> {
    let Some(print_value) = config.print else {
        return Ok(None);
    };

    let output = match print_value {
        PrintValue::ArtifactDeps => print_artifact_deps(config)?,
        PrintValue::Sysroot => resolved_sysroot_layout(config)?
            .sysroot
            .display()
            .to_string(),
        PrintValue::TargetLibdir => resolved_sysroot_layout(config)?
            .target_libdir
            .display()
            .to_string(),
        PrintValue::TargetTriple => rock_lib::sysroot::host_target_triple(),
    };

    Ok(Some(output))
}

fn resolved_sysroot_layout(
    config: &Config,
) -> Result<rock_lib::sysroot::SysrootLayout, Diagnostics> {
    rock_lib::sysroot::resolve_sysroot(config.sysroot.as_deref())
        .map(|resolution| resolution.into_layout())
        .map_err(diagnostics_from_message)
}

#[derive(Debug, Clone)]
struct ArtifactDependencyRequest {
    name: String,
    path: PathBuf,
    capabilities: ProductDependencyCapabilities,
    expected_identity: Option<ProductCrateIdentity>,
    referenced_by: Option<String>,
}

#[derive(Debug, Clone)]
struct ArtifactDependencyRecord {
    name: String,
    path: PathBuf,
    capabilities: ProductDependencyCapabilities,
    identity: ProductCrateIdentity,
}

#[derive(Default)]
struct ArtifactDependencyDiscovery {
    records: BTreeMap<String, ArtifactDependencyRecord>,
    visiting: BTreeSet<String>,
    order: Vec<String>,
}

fn print_artifact_deps(config: &Config) -> Result<String, Diagnostics> {
    let requests = artifact_dependency_root_requests(config)?;
    let mut discovery = ArtifactDependencyDiscovery::default();
    for request in requests {
        discovery.visit(request)?;
    }

    Ok(discovery.output())
}

fn artifact_dependency_root_requests(
    config: &Config,
) -> Result<Vec<ArtifactDependencyRequest>, Diagnostics> {
    if config.extern_artifact.is_empty() {
        return Err(diagnostics_from_message(
            "--print artifact-deps requires at least one --extern-artifact",
        ));
    }

    let mut seen = BTreeSet::new();
    let mut requests = Vec::new();
    for raw in &config.extern_artifact {
        let (name, path) = parse_name_path(raw, "extern-artifact")?;
        if !seen.insert(name.clone()) {
            return Err(diagnostics_from_artifact_message(
                &path,
                format!(
                    "Duplicate external artifact crate name '{}' for --print artifact-deps",
                    name
                ),
            ));
        }
        requests.push(ArtifactDependencyRequest {
            name,
            path,
            capabilities: ProductDependencyCapabilities::artifact_object(),
            expected_identity: None,
            referenced_by: None,
        });
    }

    requests.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(requests)
}

impl ArtifactDependencyDiscovery {
    fn visit(&mut self, request: ArtifactDependencyRequest) -> Result<(), Diagnostics> {
        if let Some(existing) = self.records.get(&request.name) {
            validate_existing_artifact_dependency(&request, existing)?;
            return Ok(());
        }

        if !self.visiting.insert(request.name.clone()) {
            return Err(diagnostics_from_artifact_message(
                &request.path,
                format!("Cyclic artifact dependency involving '{}'", request.name),
            ));
        }

        let header = read_artifact_dependency_header(&request)?;
        validate_artifact_dependency_identity(&request, &header.crate_identity)?;
        let dependencies =
            artifact_dependency_requests_from_header(&request.name, &request.path, &header)?;
        for dependency in dependencies {
            self.visit(dependency)?;
        }

        self.visiting.remove(&request.name);
        self.order.push(request.name.clone());
        self.records.insert(
            request.name.clone(),
            ArtifactDependencyRecord {
                name: request.name,
                path: request.path,
                capabilities: request.capabilities,
                identity: header.crate_identity,
            },
        );

        Ok(())
    }

    fn output(&self) -> String {
        self.order
            .iter()
            .filter_map(|name| self.records.get(name))
            .map(format_artifact_dependency_record)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn read_artifact_dependency_header(
    request: &ArtifactDependencyRequest,
) -> Result<ProductArtifactHeader, Diagnostics> {
    ProductArtifactHeader::read_from_path_bounded(&request.path).map_err(|err| {
        let context = match &request.referenced_by {
            Some(parent) => format!(
                "dependency artifact '{}' referenced by '{}'",
                request.name, parent
            ),
            None => format!("external artifact '{}'", request.name),
        };
        diagnostics_from_artifact_message(
            &request.path,
            format!(
                "Failed to read {} from {}: {}",
                context,
                request.path.display(),
                err
            ),
        )
    })
}

fn validate_artifact_dependency_identity(
    request: &ArtifactDependencyRequest,
    artifact_identity: &ProductCrateIdentity,
) -> Result<(), Diagnostics> {
    if artifact_identity.name != request.name {
        return Err(diagnostics_from_artifact_message(
            &request.path,
            format!(
                "Artifact dependency '{}' at {} contains crate '{}'",
                request.name,
                request.path.display(),
                artifact_identity.name
            ),
        ));
    }

    if let Some(expected_identity) = &request.expected_identity {
        if expected_identity != artifact_identity {
            return Err(diagnostics_from_artifact_message(&request.path, format!(
                "Artifact dependency '{}' at {} has identity metadata that does not match its dependent artifact",
                request.name,
                request.path.display()
            )));
        }
    }

    Ok(())
}

fn artifact_dependency_requests_from_header(
    artifact_name: &str,
    artifact_path: &Path,
    header: &ProductArtifactHeader,
) -> Result<Vec<ArtifactDependencyRequest>, Diagnostics> {
    let expected_identities =
        artifact_dependency_identity_table_by_name(artifact_name, artifact_path, header)?;
    let dependencies = artifact_product_dependencies_by_name(artifact_name, artifact_path, header)?;
    for dependency_name in expected_identities.keys() {
        if !dependencies.contains_key(dependency_name) {
            return Err(diagnostics_from_artifact_message(artifact_path, format!(
                "Artifact '{}' records dependency '{}' in its identity table without an artifact path",
                artifact_name, dependency_name
            )));
        }
    }

    let mut requests = Vec::new();
    for (dependency_name, dependency) in dependencies {
        let Some(expected_identity) = expected_identities.get(&dependency_name) else {
            return Err(diagnostics_from_artifact_message(
                artifact_path,
                format!(
                    "Artifact '{}' records dependency '{}' without matching identity metadata",
                    artifact_name, dependency_name
                ),
            ));
        };

        requests.push(ArtifactDependencyRequest {
            name: dependency.name,
            path: resolve_artifact_dependency_path(artifact_path, dependency.artifact_path),
            capabilities: dependency.capabilities,
            expected_identity: Some(expected_identity.clone()),
            referenced_by: Some(artifact_name.to_string()),
        });
    }

    Ok(requests)
}

fn artifact_dependency_identity_table_by_name(
    artifact_name: &str,
    artifact_path: &Path,
    header: &ProductArtifactHeader,
) -> Result<BTreeMap<String, ProductCrateIdentity>, Diagnostics> {
    let mut identities = BTreeMap::new();
    for identity in header.identity_dependencies.values() {
        if identities
            .insert(identity.name.clone(), identity.clone())
            .is_some()
        {
            return Err(diagnostics_from_artifact_message(
                artifact_path,
                format!(
                    "Artifact '{}' records duplicate dependency identity for crate '{}'",
                    artifact_name, identity.name
                ),
            ));
        }
    }

    Ok(identities)
}

fn artifact_product_dependencies_by_name(
    artifact_name: &str,
    artifact_path: &Path,
    header: &ProductArtifactHeader,
) -> Result<BTreeMap<String, ProductDependencyIdentity>, Diagnostics> {
    let mut dependencies = BTreeMap::new();
    for dependency in &header.dependencies {
        if dependencies
            .insert(dependency.name.clone(), dependency.clone())
            .is_some()
        {
            return Err(diagnostics_from_artifact_message(
                artifact_path,
                format!(
                    "Artifact '{}' records duplicate dependency artifact path for crate '{}'",
                    artifact_name, dependency.name
                ),
            ));
        }
    }

    Ok(dependencies)
}

fn resolve_artifact_dependency_path(artifact_path: &Path, dependency_path: PathBuf) -> PathBuf {
    if dependency_path.is_absolute() {
        return dependency_path;
    }

    artifact_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(dependency_path)
}

fn validate_existing_artifact_dependency(
    request: &ArtifactDependencyRequest,
    existing: &ArtifactDependencyRecord,
) -> Result<(), Diagnostics> {
    if !artifact_paths_match(&existing.path, &request.path) {
        return Err(diagnostics_from_artifact_message(
            &request.path,
            format!(
                "Conflicting artifact dependency paths for crate '{}': {} and {}",
                request.name,
                existing.path.display(),
                request.path.display()
            ),
        ));
    }

    if existing.capabilities != request.capabilities {
        return Err(diagnostics_from_artifact_message(
            &request.path,
            format!(
                "Conflicting artifact dependency capabilities for crate '{}' at {}",
                request.name,
                request.path.display()
            ),
        ));
    }

    if let Some(expected_identity) = &request.expected_identity {
        if expected_identity != &existing.identity {
            return Err(diagnostics_from_artifact_message(
                &request.path,
                format!(
                    "Conflicting artifact dependency identity metadata for crate '{}' at {}",
                    request.name,
                    request.path.display()
                ),
            ));
        }
    }

    Ok(())
}

fn artifact_paths_match(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn format_artifact_dependency_record(record: &ArtifactDependencyRecord) -> String {
    format!(
        "{}={} metadata={} bodies={} link={}",
        record.name,
        record.path.display(),
        record.capabilities.metadata,
        record.capabilities.bodies,
        format_artifact_dependency_link_capability(&record.capabilities.link)
    )
}

fn format_artifact_dependency_link_capability(
    capability: &ProductDependencyLinkCapability,
) -> &'static str {
    match capability {
        ProductDependencyLinkCapability::Object => "object",
        ProductDependencyLinkCapability::MetadataOnly => "metadata-only",
    }
}

fn load_program_for_source_command(
    rockc_config: &rock_lib::Config,
) -> Result<Program, Diagnostics> {
    load_program_with_source_database(rockc_config).map(|(program, _)| program)
}

fn load_program_with_source_database(
    rockc_config: &rock_lib::Config,
) -> Result<(Program, rock_lib::source_loader::SourceDatabase), Diagnostics> {
    let mut source_db = rock_lib::source_loader::SourceDatabase::new();
    for provider in &rockc_config.source_providers {
        source_db.add_source_provider(provider);
    }
    let graph = source_db
        .load_entry(rockc_config.entry_file.clone(), rockc_config)
        .map_err(rock_lib::source_loader::source_load_errors_to_diagnostics)?;
    Ok((
        Program {
            module: graph.root_module().clone(),
        },
        source_db,
    ))
}

fn format_config(config: Config) -> Result<(), Diagnostics> {
    let entry_file = config
        .entry_file
        .ok_or_else(|| diagnostics_from_message("Missing required --entry-file for --format"))?;
    let rockc_config = rock_lib::Config {
        entry_file,
        ..rock_lib::Config::default()
    };

    let program = load_program_for_source_command(&rockc_config)?;
    let mut formatter = AstFormatter { result: Ok(()) };
    program.visit(&mut formatter);
    formatter.result
}

fn expand_config(config: Config) -> Result<(), Diagnostics> {
    let entry_file = config
        .entry_file
        .ok_or_else(|| diagnostics_from_message("Missing required --entry-file for --expand"))?;
    let rockc_config = rock_lib::Config {
        entry_file,
        ..rock_lib::Config::default()
    };

    let (program, source_db) = load_program_with_source_database(&rockc_config)?;
    let diagnostic_sources =
        rock_lib::diagnostic::DiagnosticSourceMap::from_source_database(&source_db);
    let macro_context = rock_lib::macro_expansion::MacroExpansionContext::new(&rockc_config);
    let expanded = rock_lib::macro_expansion::expand_macros_with_context(program, &macro_context)
        .map_err(|diagnostics| diagnostics.with_sources(&diagnostic_sources))?;
    expanded.visit(&mut ModulePrinter);
    Ok(())
}

fn validate_artifact(path: &std::path::Path) -> Result<(), Diagnostics> {
    let header = ProductArtifactHeader::read_from_path_bounded(path).map_err(|error| {
        diagnostics_from_artifact_message(path, format!("Failed to read artifact: {error}"))
    })?;
    let products = PortableProductArtifact::read_payload_from_path_bounded(path, &header)
        .and_then(PortableProductArtifact::into_products)
        .map_err(|error| {
            diagnostics_from_artifact_message(
                path,
                format!("Failed to read artifact payload: {error}"),
            )
        })?;
    let Some(object_path) = products.link.object_path else {
        return Err(diagnostics_from_artifact_message(
            path,
            format!(
                "Product artifact {} does not reference an object file",
                path.display()
            ),
        ));
    };
    let object_path = resolve_artifact_object_path(path, object_path);
    if !object_path.exists() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.push(rock_lib::diagnostic::Diagnostic::for_file(
            format!(
                "Product artifact {} references missing object file {}",
                path.display(),
                object_path.display()
            ),
            object_path,
        ));
        return Err(diagnostics);
    }

    Ok(())
}

fn diagnostics_from_artifact_message(path: &Path, message: impl Into<String>) -> Diagnostics {
    let mut diagnostics = Diagnostics::default();
    diagnostics.push(rock_lib::diagnostic::Diagnostic::for_artifact(
        message.into(),
        path.to_path_buf(),
    ));
    diagnostics
}

fn resolve_artifact_object_path(artifact_path: &std::path::Path, object_path: PathBuf) -> PathBuf {
    if object_path.is_absolute() {
        return object_path;
    }

    let artifact_parent = artifact_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let artifact_relative = artifact_parent.join(&object_path);
    if artifact_relative.exists() {
        return artifact_relative;
    }

    if let Some(parent) = artifact_parent.parent() {
        let sibling_relative = parent.join(&object_path);
        if sibling_relative.exists() {
            return sibling_relative;
        }
    }

    if object_path.exists() {
        return object_path;
    }

    artifact_relative
}

struct AstFormatter {
    result: Result<(), Diagnostics>,
}

impl<'a> Visitor<'a> for AstFormatter {
    fn visit_module(&mut self, module: &'a Module) {
        if self.result.is_err() {
            return;
        }

        let formatted = if let Some(path) = &module.filepath {
            let source = match fs::read_to_string(path) {
                Ok(source) => source,
                Err(err) => {
                    self.result = Err(diagnostics_from_message(format!(
                        "Failed to read source module {} for formatting: {}",
                        path.display(),
                        err
                    )));
                    return;
                }
            };

            rock_lib::fmt::format(rock_lib::fmt::FormatInput::module_with_source(
                module, &source,
            ))
        } else {
            rock_lib::fmt::format(rock_lib::fmt::FormatInput::module(module))
        };

        if let Some(path) = &module.filepath {
            if let Err(err) = fs::write(path, &formatted) {
                self.result = Err(diagnostics_from_message(format!(
                    "Failed to write formatted module {}: {}",
                    path.display(),
                    err
                )));
                return;
            }
        }

        println!("{}", formatted);
        walk_module(self, module);
    }
}

struct ModulePrinter;

impl<'a> Visitor<'a> for ModulePrinter {
    fn visit_module(&mut self, module: &'a Module) {
        if let Some(name) = &module.name {
            println!("### {}: ###\n", name.name);
        }

        println!(
            "{}",
            rock_lib::fmt::format(rock_lib::fmt::FormatInput::module(module))
        );
        walk_module(self, module);
    }
}

fn diagnostics_from_message(message: impl Into<String>) -> Diagnostics {
    let mut diagnostics = Diagnostics::default();
    diagnostics.push(rock_lib::diagnostic::Diagnostic::for_toolchain(
        message.into(),
    ));
    diagnostics
}

fn parse_name_path(input: &str, flag: &str) -> Result<(String, PathBuf), Diagnostics> {
    let Some((name, path)) = input.split_once('=') else {
        return Err(diagnostics_from_message(format!(
            "Invalid --{} format: '{}'. Expected name=path",
            flag, input
        )));
    };
    if name.is_empty() || path.is_empty() {
        return Err(diagnostics_from_message(format!(
            "Invalid --{} format: '{}'. Expected name=path",
            flag, input
        )));
    }

    Ok((name.to_string(), PathBuf::from(path)))
}

#[cfg(test)]
mod tests;
