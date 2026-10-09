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
    /// Optimization level (0-3)
    #[arg(short = 'O', long, default_value = "0")]
    opt_level: u8,
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
    /// Don't inject stdlib prelude
    #[arg(long)]
    no_prelude: bool,
    /// Don't auto-load bundled stdlib from the sysroot
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
            opt_level: config.opt_level,
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
        .map_err(rock_lib::source_load_errors_to_diagnostics)?;
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
mod tests {
    use super::{
        load_program_for_source_command, load_program_with_source_database, print_request_output,
        resolve_artifact_object_path, run_config, Config,
    };
    use clap::Parser;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use rock_lib::products::{
        CompilerProducts, ProductCrateId, ProductCrateIdentity, ProductDependencyIdentity,
        ProductIdentityTable, ProductLinkData,
    };

    #[test]
    fn test_print_target_triple_does_not_require_entry_file() {
        let config = Config::try_parse_from(["rockc", "--print", "target-triple"]).unwrap();

        assert_eq!(
            print_request_output(&config).unwrap(),
            Some(rock_lib::sysroot::host_target_triple())
        );
    }

    #[test]
    fn test_print_target_libdir_uses_sysroot_override() {
        let config = Config::try_parse_from([
            "rockc",
            "--sysroot",
            "/tmp/rock-toolchain",
            "--print",
            "target-libdir",
        ])
        .unwrap();
        let expected = rock_lib::sysroot::SysrootLayout::new(
            PathBuf::from("/tmp/rock-toolchain"),
            rock_lib::sysroot::host_target_triple(),
        )
        .target_libdir
        .display()
        .to_string();

        assert_eq!(print_request_output(&config).unwrap(), Some(expected));
    }

    #[test]
    fn test_print_artifact_deps_reports_transitive_dependency_capabilities() {
        let base = std::env::temp_dir().join(format!(
            "rockc_artifact_deps_{}_{}",
            std::process::id(),
            "transitive_capabilities"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let dep_a_artifact = base.join("dep_a.rkca");
        let dep_b_artifact = base.join("dep_b.rkca");

        product_artifact("dep_a", Vec::new(), ProductIdentityTable::default())
            .write_artifact_to_path(&dep_a_artifact)
            .unwrap();
        product_artifact(
            "dep_b",
            vec![ProductDependencyIdentity::artifact_object(
                "dep_a".to_string(),
                PathBuf::from("dep_a.rkca"),
            )],
            ProductIdentityTable {
                dependencies: BTreeMap::from([(
                    ProductCrateId(1),
                    ProductCrateIdentity::local("dep_a".to_string()),
                )]),
                ..ProductIdentityTable::default()
            },
        )
        .write_artifact_to_path(&dep_b_artifact)
        .unwrap();

        let extern_artifact = format!("dep_b={}", dep_b_artifact.display());
        let config = Config::try_parse_from([
            "rockc",
            "--print",
            "artifact-deps",
            "--extern-artifact",
            extern_artifact.as_str(),
        ])
        .unwrap();

        let output = print_request_output(&config).unwrap().unwrap();

        assert_eq!(
            output,
            format!(
                "dep_a={} metadata=true bodies=true link=object\n\
                 dep_b={} metadata=true bodies=true link=object",
                dep_a_artifact.display(),
                dep_b_artifact.display()
            )
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_print_artifact_deps_rejects_missing_dependency_artifact() {
        let base = artifact_deps_test_dir("missing_dependency_artifact");
        let dep_b_artifact = base.join("dep_b.rkca");
        product_artifact(
            "dep_b",
            vec![ProductDependencyIdentity::artifact_object(
                "missing".to_string(),
                PathBuf::from("missing.rkca"),
            )],
            identity_table_with_dependencies([("missing", ProductCrateId(1))]),
        )
        .write_artifact_to_path(&dep_b_artifact)
        .unwrap();

        let config = artifact_deps_config([("dep_b", &dep_b_artifact)]);
        let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

        assert!(messages.contains("dependency artifact 'missing' referenced by 'dep_b'"));
        assert!(messages.contains("missing.rkca"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_print_artifact_deps_rejects_corrupt_dependency_artifact() {
        let base = artifact_deps_test_dir("corrupt_dependency_artifact");
        let corrupt_artifact = base.join("corrupt.rkca");
        let dep_b_artifact = base.join("dep_b.rkca");
        std::fs::write(&corrupt_artifact, b"not a product artifact").unwrap();
        product_artifact(
            "dep_b",
            vec![ProductDependencyIdentity::artifact_object(
                "corrupt".to_string(),
                PathBuf::from("corrupt.rkca"),
            )],
            identity_table_with_dependencies([("corrupt", ProductCrateId(1))]),
        )
        .write_artifact_to_path(&dep_b_artifact)
        .unwrap();

        let config = artifact_deps_config([("dep_b", &dep_b_artifact)]);
        let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

        assert!(messages.contains("dependency artifact 'corrupt' referenced by 'dep_b'"));
        assert!(messages.contains("Product artifact is shorter than the 28-byte preamble"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_print_artifact_deps_rejects_crate_name_mismatch() {
        let base = artifact_deps_test_dir("crate_name_mismatch");
        let actual_artifact = base.join("actual.rkca");
        product_artifact("actual", Vec::new(), ProductIdentityTable::default())
            .write_artifact_to_path(&actual_artifact)
            .unwrap();

        let config = artifact_deps_config([("expected", &actual_artifact)]);
        let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

        assert!(messages.contains("Artifact dependency 'expected'"));
        assert!(messages.contains("contains crate 'actual'"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_print_artifact_deps_rejects_cycles() {
        let base = artifact_deps_test_dir("cycle");
        let dep_a_artifact = base.join("dep_a.rkca");
        let dep_b_artifact = base.join("dep_b.rkca");
        product_artifact(
            "dep_a",
            vec![ProductDependencyIdentity::artifact_object(
                "dep_b".to_string(),
                PathBuf::from("dep_b.rkca"),
            )],
            identity_table_with_dependencies([("dep_b", ProductCrateId(1))]),
        )
        .write_artifact_to_path(&dep_a_artifact)
        .unwrap();
        product_artifact(
            "dep_b",
            vec![ProductDependencyIdentity::artifact_object(
                "dep_a".to_string(),
                PathBuf::from("dep_a.rkca"),
            )],
            identity_table_with_dependencies([("dep_a", ProductCrateId(1))]),
        )
        .write_artifact_to_path(&dep_b_artifact)
        .unwrap();

        let config = artifact_deps_config([("dep_a", &dep_a_artifact)]);
        let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

        assert!(messages.contains("Cyclic artifact dependency involving 'dep_a'"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_print_artifact_deps_rejects_duplicate_root_names() {
        let config = Config::try_parse_from([
            "rockc",
            "--print",
            "artifact-deps",
            "--extern-artifact",
            "dep=/tmp/first.rkca",
            "--extern-artifact",
            "dep=/tmp/second.rkca",
        ])
        .unwrap();

        let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

        assert!(messages.contains("Duplicate external artifact crate name 'dep'"));
    }

    #[test]
    fn test_print_artifact_deps_rejects_conflicting_transitive_paths() {
        let base = artifact_deps_test_dir("conflicting_transitive_paths");
        let dep_a_artifact = base.join("dep_a.rkca");
        let dep_b_artifact = base.join("dep_b.rkca");
        let dep_c_artifact = base.join("dep_c.rkca");
        product_artifact("dep_a", Vec::new(), ProductIdentityTable::default())
            .write_artifact_to_path(&dep_a_artifact)
            .unwrap();
        product_artifact(
            "dep_b",
            vec![ProductDependencyIdentity::artifact_object(
                "dep_a".to_string(),
                PathBuf::from("dep_a.rkca"),
            )],
            identity_table_with_dependencies([("dep_a", ProductCrateId(1))]),
        )
        .write_artifact_to_path(&dep_b_artifact)
        .unwrap();
        product_artifact(
            "dep_c",
            vec![ProductDependencyIdentity::artifact_object(
                "dep_a".to_string(),
                PathBuf::from("other_dep_a.rkca"),
            )],
            identity_table_with_dependencies([("dep_a", ProductCrateId(1))]),
        )
        .write_artifact_to_path(&dep_c_artifact)
        .unwrap();

        let config = artifact_deps_config([("dep_b", &dep_b_artifact), ("dep_c", &dep_c_artifact)]);
        let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

        assert!(messages.contains("Conflicting artifact dependency paths for crate 'dep_a'"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_print_artifact_deps_rejects_inconsistent_dependency_identity_metadata() {
        let base = artifact_deps_test_dir("inconsistent_dependency_identity_metadata");
        let dep_a_artifact = base.join("dep_a.rkca");
        let dep_b_artifact = base.join("dep_b.rkca");
        let mut mismatched_identity = ProductCrateIdentity::local("dep_a".to_string());
        mismatched_identity.version = "2.0.0".to_string();
        product_artifact("dep_a", Vec::new(), ProductIdentityTable::default())
            .write_artifact_to_path(&dep_a_artifact)
            .unwrap();
        product_artifact(
            "dep_b",
            vec![ProductDependencyIdentity::artifact_object(
                "dep_a".to_string(),
                PathBuf::from("dep_a.rkca"),
            )],
            ProductIdentityTable {
                dependencies: BTreeMap::from([(ProductCrateId(1), mismatched_identity)]),
                ..ProductIdentityTable::default()
            },
        )
        .write_artifact_to_path(&dep_b_artifact)
        .unwrap();

        let config = artifact_deps_config([("dep_b", &dep_b_artifact)]);
        let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

        assert!(messages.contains("identity metadata that does not match"));

        let _ = std::fs::remove_dir_all(&base);
    }

    fn product_artifact(
        name: &str,
        dependencies: Vec<ProductDependencyIdentity>,
        identity_table: ProductIdentityTable,
    ) -> CompilerProducts {
        CompilerProducts {
            crate_identity: ProductCrateIdentity::local(name.to_string()),
            identity_table,
            interface: rock_lib::products::ProductInterface::default(),
            bodies: Default::default(),
            link: ProductLinkData {
                native_return_origins: Default::default(),
                runtime_support: Default::default(),
                object_path: Some(PathBuf::from(format!("{}.o", name))),
                records: Default::default(),
            },
            dependencies,
            source_fingerprint: Default::default(),
            proc_macros: Vec::new(),
            infix_precedence: Default::default(),
        }
    }

    fn artifact_deps_test_dir(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "rockc_artifact_deps_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn identity_table_with_dependencies<const N: usize>(
        dependencies: [(&str, ProductCrateId); N],
    ) -> ProductIdentityTable {
        ProductIdentityTable {
            dependencies: dependencies
                .into_iter()
                .map(|(name, id)| (id, ProductCrateIdentity::local(name.to_string())))
                .collect(),
            ..ProductIdentityTable::default()
        }
    }

    fn artifact_deps_config<const N: usize>(artifacts: [(&str, &std::path::Path); N]) -> Config {
        let mut args = vec![
            "rockc".to_string(),
            "--print".to_string(),
            "artifact-deps".to_string(),
        ];
        for (name, path) in artifacts {
            args.push("--extern-artifact".to_string());
            args.push(format!("{}={}", name, path.display()));
        }

        Config::try_parse_from(args).unwrap()
    }

    fn diagnostic_messages(diagnostics: rock_lib::diagnostic::Diagnostics) -> String {
        diagnostics
            .0
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_source_command_parse_errors_are_structured_diagnostics() {
        let base = std::env::temp_dir().join(format!(
            "rockc_source_command_diagnostics_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        std::fs::write(&entry_file, "main = ->\n    `\n").unwrap();

        let diagnostics = load_program_for_source_command(&rock_lib::Config {
            entry_file,
            ..rock_lib::Config::default()
        })
        .unwrap_err();
        let messages = diagnostics
            .0
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(messages.contains("Unexpected token") || messages.contains("Lexer"));
        assert!(!messages.contains("Failed to load source graph"));
        assert!(!messages.contains("Parse {"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_entry_file_is_required_for_compile_mode() {
        let config = Config::try_parse_from(["rockc"]).unwrap();

        assert!(config.into_compiler_config().is_err());
    }

    #[test]
    fn test_extern_artifact_parses() {
        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            "main.rk",
            "--extern-artifact",
            "dep=/tmp/dep.rkca",
        ])
        .unwrap();

        assert_eq!(
            config.extern_artifact,
            vec!["dep=/tmp/dep.rkca".to_string()]
        );
    }

    #[test]
    fn test_malformed_extern_artifact_is_rejected_without_panic() {
        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            "main.rk",
            "--extern-artifact",
            "dep",
        ])
        .unwrap();

        assert!(config.into_compiler_config().is_err());
    }

    #[test]
    fn test_extern_product_artifact_is_rejected() {
        let error = Config::try_parse_from([
            "rockc",
            "--entry-file",
            "main.rk",
            "--extern-product-artifact",
            "dep=build/dep.rkca",
        ])
        .unwrap_err();

        assert!(error.to_string().contains("unexpected argument"));
    }

    #[test]
    fn test_positional_artifacts_are_rejected() {
        let error =
            Config::try_parse_from(["rockc", "--entry-file", "main.rk", "dep=/tmp/dep.rkca"])
                .unwrap_err();

        assert!(error.to_string().contains("unexpected argument"));
    }

    #[test]
    fn test_emit_artifact_and_object_parse() {
        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            "main.rk",
            "--emit-artifact",
            "build/main.rkca",
            "--emit-object",
            "build/main.o",
            "--crate-name",
            "demo",
        ])
        .unwrap();

        assert_eq!(config.emit_artifact, Some(PathBuf::from("build/main.rkca")));
        assert_eq!(config.emit_object, Some(PathBuf::from("build/main.o")));
        assert_eq!(config.crate_name, Some("demo".to_string()));
    }

    #[test]
    fn test_run_config_formats_entry_file_in_place() {
        let base =
            std::env::temp_dir().join(format!("rockc_format_{}_{}", std::process::id(), "entry"));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let source = "// entry docs\n\nmain  =  ->\n    0\n";
        std::fs::write(&entry_file, source).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--format",
        ])
        .unwrap();

        run_config(config).unwrap();

        let formatted = std::fs::read_to_string(&entry_file).unwrap();
        assert_eq!(formatted, "// entry docs\n\nmain = -> 0\n");
        assert_ne!(formatted, source);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_format_preserves_collapsed_body_trivia() {
        let base = std::env::temp_dir().join(format!(
            "rockc_format_{}_{}",
            std::process::id(),
            "collapsed_body_trivia"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let source = "main = ->\n    // body comment\n    0\n\n";
        std::fs::write(&entry_file, source).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--format",
        ])
        .unwrap();

        run_config(config).unwrap();

        let formatted = std::fs::read_to_string(&entry_file).unwrap();
        assert_eq!(formatted, "main = -> 0\n    // body comment\n\n");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_format_preserves_standalone_multiline_block_comment() {
        let base = std::env::temp_dir().join(format!(
            "rockc_format_{}_{}",
            std::process::id(),
            "multiline_block_comment"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let source = "/*\nmodule docs\n*/\nmain  =  ->  0\n";
        std::fs::write(&entry_file, source).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--format",
        ])
        .unwrap();

        run_config(config).unwrap();

        let formatted = std::fs::read_to_string(&entry_file).unwrap();
        assert_eq!(formatted, "/*\nmodule docs\n*/\nmain = -> 0\n");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_format_anchors_trivia_to_next_top_level() {
        let base = std::env::temp_dir().join(format!(
            "rockc_format_{}_{}",
            std::process::id(),
            "next_top_level_trivia"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let source = "foo = ->\n    0\n\n// bar docs\nbar = -> 1\n";
        std::fs::write(&entry_file, source).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--format",
        ])
        .unwrap();

        run_config(config).unwrap();

        let formatted = std::fs::read_to_string(&entry_file).unwrap();
        assert_eq!(formatted, "foo = -> 0\n\n// bar docs\nbar = -> 1\n");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_format_keeps_operator_functions_as_code() {
        let base = std::env::temp_dir().join(format!(
            "rockc_format_{}_{}",
            std::process::id(),
            "operator_function"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let source = "*  =  ->  1\n";
        std::fs::write(&entry_file, source).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--format",
        ])
        .unwrap();

        run_config(config).unwrap();

        let formatted = std::fs::read_to_string(&entry_file).unwrap();
        assert_eq!(formatted, "* = -> 1\n");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_format_preserves_struct_field_docs_before_field() {
        let base = std::env::temp_dir().join(format!(
            "rockc_format_{}_{}",
            std::process::id(),
            "struct_field_docs"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let source = "struct Box\n    // field docs\n    value : ()\n";
        std::fs::write(&entry_file, source).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--format",
        ])
        .unwrap();

        run_config(config).unwrap();

        let formatted = std::fs::read_to_string(&entry_file).unwrap();
        assert_eq!(formatted, "struct Box\n    // field docs\n    value : ()\n");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_format_preserves_statement_docs_between_statements() {
        let base = std::env::temp_dir().join(format!(
            "rockc_format_{}_{}",
            std::process::id(),
            "statement_docs"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let source = "main = ->\n    a = 1\n    // keep between statements\n    b = 2\n";
        std::fs::write(&entry_file, source).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--format",
        ])
        .unwrap();

        run_config(config).unwrap();

        let formatted = std::fs::read_to_string(&entry_file).unwrap();
        assert_eq!(
            formatted,
            "main = ->\n    a = 1\n    // keep between statements\n    b = 2\n"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_format_preserves_same_line_block_comment_before_code() {
        let base = std::env::temp_dir().join(format!(
            "rockc_format_{}_{}",
            std::process::id(),
            "same_line_block_comment"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let source = "/* doc */ main  =  ->  0\n";
        std::fs::write(&entry_file, source).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--format",
        ])
        .unwrap();

        run_config(config).unwrap();

        let formatted = std::fs::read_to_string(&entry_file).unwrap();
        assert_eq!(formatted, "/* doc */\nmain = -> 0\n");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_format_preserves_block_comment_line_comment_remainder() {
        let base = std::env::temp_dir().join(format!(
            "rockc_format_{}_{}",
            std::process::id(),
            "block_comment_line_comment_remainder"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let source = "/* docs */ // extra\nmain  =  ->  0\n";
        std::fs::write(&entry_file, source).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--format",
        ])
        .unwrap();

        run_config(config).unwrap();

        let formatted = std::fs::read_to_string(&entry_file).unwrap();
        assert_eq!(formatted, "/* docs */ // extra\nmain = -> 0\n");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_format_preserves_multiline_block_comment_line_comment_remainder() {
        let base = std::env::temp_dir().join(format!(
            "rockc_format_{}_{}",
            std::process::id(),
            "multiline_block_comment_line_comment_remainder"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let source = "/*\ndocs\n*/ // extra\nmain  =  ->  0\n";
        std::fs::write(&entry_file, source).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--format",
        ])
        .unwrap();

        run_config(config).unwrap();

        let formatted = std::fs::read_to_string(&entry_file).unwrap();
        assert_eq!(formatted, "/*\ndocs\n*/ // extra\nmain = -> 0\n");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_format_preserves_multiline_block_comment_before_code() {
        let base = std::env::temp_dir().join(format!(
            "rockc_format_{}_{}",
            std::process::id(),
            "partial_multiline_block_comment"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let source = "/*\nmodule docs\n*/ main  =  ->  0\n";
        std::fs::write(&entry_file, source).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--format",
        ])
        .unwrap();

        run_config(config).unwrap();

        let formatted = std::fs::read_to_string(&entry_file).unwrap();
        assert_eq!(formatted, "/*\nmodule docs\n*/\nmain = -> 0\n");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_rock_lib_format_input_is_public_formatter_api() {
        let base = std::env::temp_dir().join(format!(
            "rockc_format_api_{}_{}",
            std::process::id(),
            "module"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        std::fs::write(&entry_file, "main  =  ->\n    0\n").unwrap();

        let program = load_program_for_source_command(&rock_lib::Config {
            entry_file,
            ..rock_lib::Config::default()
        })
        .unwrap();
        let source = std::fs::read_to_string(&program.module.filepath.clone().unwrap()).unwrap();
        let formatted = rock_lib::fmt::format(rock_lib::fmt::FormatInput::module_with_source(
            &program.module,
            &source,
        ));

        assert!(formatted.contains("main = ->"));
        assert_ne!(formatted, "main  =  ->\n    0\n");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_expands_entry_file_without_compiling() {
        let base =
            std::env::temp_dir().join(format!("rockc_expand_{}_{}", std::process::id(), "entry"));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let output_dir = base.join("build");
        std::fs::write(&entry_file, "main = -> 0\n").unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--output-dir",
            output_dir.to_str().unwrap(),
            "--expand",
        ])
        .unwrap();

        run_config(config).unwrap();

        assert!(!output_dir.join("main").exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn source_command_loader_keeps_virtual_source_for_diagnostic_attachment() {
        let path = PathBuf::from("/virtual/rockc/main.rk");
        let text = "main = -> 0\n";
        let config = rock_lib::Config {
            entry_file: path.clone(),
            no_std: true,
            no_prelude: true,
            source_providers: vec![rock_lib::SourceProvider::Virtual {
                path: path.clone(),
                text: text.to_string(),
            }],
            ..rock_lib::Config::default()
        };

        let (program, source_db) = load_program_with_source_database(&config).unwrap();
        assert_eq!(program.module.filepath, Some(path.clone()));
        let sources = rock_lib::diagnostic::DiagnosticSourceMap::from_source_database(&source_db);
        let mut diagnostic = rock_lib::diagnostic::Diagnostic::new(
            "source command failure".to_string(),
            rock_lib::Span::new(path.clone(), 0, 4),
        );
        sources.attach(&mut diagnostic);

        assert_eq!(diagnostic.source.as_ref().unwrap().text, text);
    }

    #[test]
    fn test_run_config_validate_artifact_rejects_corrupt_artifact() {
        let base = std::env::temp_dir().join(format!(
            "rockc_validate_artifact_{}_{}",
            std::process::id(),
            "corrupt"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let artifact_path = base.join("dep.rkca");
        std::fs::write(&artifact_path, b"not a product artifact").unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--validate-artifact",
            artifact_path.to_str().unwrap(),
        ])
        .unwrap();

        let diagnostics = run_config(config).expect_err("corrupt artifact should fail");
        assert_eq!(
            diagnostics.0[0].location,
            rock_lib::diagnostic::DiagnosticLocation::Artifact(artifact_path.clone())
        );
        assert_eq!(
            diagnostics.0[0].code,
            Some(rock_lib::diagnostic::DiagnosticCode::Artifact)
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_validate_artifact_reports_missing_object_as_file() {
        let base = std::env::temp_dir().join(format!(
            "rockc_validate_artifact_{}_{}",
            std::process::id(),
            "missing_object"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let artifact_path = base.join("dep.rkca");
        let object_path = base.join("missing.o");
        let products = rock_lib::products::CompilerProducts {
            crate_identity: ProductCrateIdentity::local("dep".to_string()),
            identity_table: Default::default(),
            interface: rock_lib::products::ProductInterface::default(),
            bodies: Default::default(),
            link: rock_lib::products::ProductLinkData {
                native_return_origins: Default::default(),
                runtime_support: Default::default(),
                object_path: Some(PathBuf::from(object_path.file_name().unwrap())),
                records: Default::default(),
            },
            dependencies: Vec::new(),
            source_fingerprint: Default::default(),
            proc_macros: Vec::new(),
            infix_precedence: Default::default(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--validate-artifact",
            artifact_path.to_str().unwrap(),
        ])
        .unwrap();
        let diagnostics = run_config(config).expect_err("missing object should fail");

        assert_eq!(
            diagnostics.0[0].location,
            rock_lib::diagnostic::DiagnosticLocation::File(object_path.clone())
        );
        assert_eq!(
            diagnostics.0[0].code,
            Some(rock_lib::diagnostic::DiagnosticCode::File)
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_validate_artifact_accepts_sibling_root_relative_object() {
        let base = std::env::temp_dir().join(format!(
            "rockc_validate_artifact_{}_{}",
            std::process::id(),
            "sibling_root_object"
        ));
        let _ = std::fs::remove_dir_all(&base);
        let artifact_dir = base.join("build").join("artifacts");
        let object_dir = base.join("build").join("objects");
        std::fs::create_dir_all(&artifact_dir).unwrap();
        std::fs::create_dir_all(&object_dir).unwrap();
        let artifact_path = artifact_dir.join("dep.rkca");
        std::fs::write(object_dir.join("dep.o"), []).unwrap();

        let products = rock_lib::products::CompilerProducts {
            crate_identity: rock_lib::products::ProductCrateIdentity::local("dep".to_string()),
            identity_table: Default::default(),
            interface: rock_lib::products::ProductInterface::default(),
            bodies: Default::default(),
            link: rock_lib::products::ProductLinkData {
                native_return_origins: Default::default(),
                runtime_support: Default::default(),
                object_path: Some(PathBuf::from("objects/dep.o")),
                records: Default::default(),
            },
            dependencies: Vec::new(),
            source_fingerprint: Default::default(),
            proc_macros: Vec::new(),
            infix_precedence: Default::default(),
        };
        products.write_artifact_to_path(&artifact_path).unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--validate-artifact",
            artifact_path.to_str().unwrap(),
        ])
        .unwrap();

        run_config(config).unwrap();

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_resolve_artifact_object_path_prefers_artifact_relative_over_cwd() {
        let test_name = format!(
            "rockc_validate_artifact_conflict_{}_{}",
            std::process::id(),
            "artifact_relative"
        );
        let base = std::env::temp_dir().join(&test_name);
        let _ = std::fs::remove_dir_all(&base);
        let artifact_dir = base.join("artifacts");
        std::fs::create_dir_all(&artifact_dir).unwrap();
        let artifact_path = artifact_dir.join("dep.rkca");

        let object_path = PathBuf::from("..")
            .join("target")
            .join(&test_name)
            .join("dep.o");
        let cwd_object_path = std::env::current_dir().unwrap().join(&object_path);
        let artifact_object_path = artifact_dir.join(&object_path);
        std::fs::create_dir_all(cwd_object_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(artifact_object_path.parent().unwrap()).unwrap();
        std::fs::write(&cwd_object_path, b"wrong object").unwrap();
        std::fs::write(&artifact_object_path, b"right object").unwrap();

        assert_eq!(
            resolve_artifact_object_path(&artifact_path, object_path),
            artifact_object_path
        );

        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(
            std::env::current_dir()
                .unwrap()
                .join("..")
                .join("target")
                .join(test_name),
        );
    }

    #[test]
    fn test_run_config_rejects_emit_artifact_without_no_link() {
        let base = std::env::temp_dir().join(format!(
            "rockc_product_artifact_{}_{}",
            std::process::id(),
            "requires_object"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let artifact_path = base.join("build").join("demo.rkca");
        std::fs::write(&entry_file, "main = ->\n    0\n").unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--output-dir",
            base.join("build").to_str().unwrap(),
            "--no-std",
            "--no-prelude",
            "--crate-name",
            "demo",
            "--emit-artifact",
            artifact_path.to_str().unwrap(),
        ])
        .unwrap();

        assert!(run_config(config).is_err());
        assert!(!artifact_path.exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_emits_product_artifact_and_object() {
        let base = std::env::temp_dir().join(format!(
            "rockc_product_artifact_{}_{}",
            std::process::id(),
            "emit"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let artifact_path = base.join("build").join("demo.rkca");
        let object_path = base.join("build").join("demo.o");
        std::fs::write(&entry_file, "main = ->\n    0\n").unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--output-dir",
            base.join("build").to_str().unwrap(),
            "--no-std",
            "--no-prelude",
            "--no-link",
            "--crate-name",
            "demo",
            "--emit-artifact",
            artifact_path.to_str().unwrap(),
            "--emit-object",
            object_path.to_str().unwrap(),
        ])
        .unwrap();

        run_config(config).unwrap();

        assert!(artifact_path.exists());
        assert!(object_path.exists());
        let products =
            rock_lib::products::CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();
        let main_id = products.identity_table.export_names["main"];
        assert!(products.interface.functions.contains_key(&main_id));
        assert_eq!(products.link.object_path, Some(PathBuf::from("demo.o")));

        let validate_config = Config::try_parse_from([
            "rockc",
            "--validate-artifact",
            artifact_path.to_str().unwrap(),
        ])
        .unwrap();
        run_config(validate_config).unwrap();

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_emits_colocated_product_artifact_with_relative_object_path() {
        let base = std::env::temp_dir().join(format!(
            "rockc_product_artifact_{}_{}",
            std::process::id(),
            "relative_object"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let build_dir = base.join("build");
        let artifact_path = build_dir.join("demo.rkca");
        let object_path = build_dir.join("demo.o");
        std::fs::write(&entry_file, "main = ->\n    0\n").unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--output-dir",
            build_dir.to_str().unwrap(),
            "--no-std",
            "--no-prelude",
            "--no-link",
            "--crate-name",
            "demo",
            "--emit-artifact",
            artifact_path.to_str().unwrap(),
            "--emit-object",
            object_path.to_str().unwrap(),
        ])
        .unwrap();

        run_config(config).unwrap();

        let products =
            rock_lib::products::CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();
        assert_eq!(products.link.object_path, Some(PathBuf::from("demo.o")));

        let validate_config = Config::try_parse_from([
            "rockc",
            "--validate-artifact",
            artifact_path.to_str().unwrap(),
        ])
        .unwrap();
        run_config(validate_config).unwrap();

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_emits_sibling_layout_product_artifact_with_relative_object_path() {
        let base = std::env::temp_dir().join(format!(
            "rockc_product_artifact_{}_{}",
            std::process::id(),
            "sibling_layout"
        ));
        let _ = std::fs::remove_dir_all(&base);
        let dep_dir = base.join("dep");
        let app_dir = base.join("app");
        let dep_artifacts = dep_dir.join("build").join("artifacts");
        let dep_objects = dep_dir.join("build").join("objects");
        let app_build = app_dir.join("build");
        std::fs::create_dir_all(&dep_artifacts).unwrap();
        std::fs::create_dir_all(&dep_objects).unwrap();
        std::fs::create_dir_all(&app_build).unwrap();
        let dep_entry = dep_dir.join("lib.rk");
        let app_entry = app_dir.join("main.rk");
        let dep_artifact = dep_artifacts.join("dep.rkca");
        let dep_object = dep_objects.join("dep.o");
        std::fs::write(&dep_entry, "answer = ->\n    5\n< answer\n").unwrap();
        std::fs::write(&app_entry, "> dep::answer\n\nmain = ->\n    answer!\n").unwrap();

        let dep_config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            dep_entry.to_str().unwrap(),
            "--output-dir",
            dep_objects.to_str().unwrap(),
            "--no-std",
            "--no-prelude",
            "--no-link",
            "--crate-name",
            "dep",
            "--emit-artifact",
            dep_artifact.to_str().unwrap(),
            "--emit-object",
            dep_object.to_str().unwrap(),
        ])
        .unwrap();
        run_config(dep_config).unwrap();

        let products =
            rock_lib::products::CompilerProducts::read_artifact_from_path(&dep_artifact).unwrap();
        assert_eq!(
            products.link.object_path,
            Some(PathBuf::from("objects").join("dep.o"))
        );

        let relocated_artifacts = base.join("relocated").join("artifacts");
        let relocated_objects = base.join("relocated").join("objects");
        std::fs::create_dir_all(&relocated_artifacts).unwrap();
        std::fs::create_dir_all(&relocated_objects).unwrap();
        let relocated_artifact = relocated_artifacts.join("dep.rkca");
        std::fs::copy(&dep_artifact, &relocated_artifact).unwrap();
        std::fs::copy(&dep_object, relocated_objects.join("dep.o")).unwrap();
        std::fs::remove_dir_all(dep_dir.join("build")).unwrap();

        let extern_artifact = format!("dep={}", relocated_artifact.display());
        let app_config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            app_entry.to_str().unwrap(),
            "--output-dir",
            app_build.to_str().unwrap(),
            "--no-std",
            "--no-prelude",
            "--extern-artifact",
            extern_artifact.as_str(),
        ])
        .unwrap();
        run_config(app_config).unwrap();

        let executable = app_build.join("main");
        assert!(executable.exists());
        let status = std::process::Command::new(&executable).status().unwrap();
        assert_eq!(status.code(), Some(5));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_emits_filename_only_product_artifact() {
        let base = std::env::temp_dir().join(format!(
            "rockc_product_artifact_{}_{}",
            std::process::id(),
            "filename_only"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let entry_file = base.join("main.rk");
        let build_dir = base.join("build");
        let object_path = build_dir.join("demo.o");
        let artifact_filename = format!("rockc_filename_only_{}.rkca", std::process::id());
        let artifact_path = PathBuf::from(&artifact_filename);
        let _ = std::fs::remove_file(&artifact_path);
        std::fs::write(&entry_file, "main = ->\n    0\n").unwrap();

        let config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            entry_file.to_str().unwrap(),
            "--output-dir",
            build_dir.to_str().unwrap(),
            "--no-std",
            "--no-prelude",
            "--no-link",
            "--crate-name",
            "demo",
            "--emit-artifact",
            artifact_filename.as_str(),
            "--emit-object",
            object_path.to_str().unwrap(),
        ])
        .unwrap();

        run_config(config).unwrap();
        assert!(artifact_path.exists());

        let validate_config =
            Config::try_parse_from(["rockc", "--validate-artifact", artifact_filename.as_str()])
                .unwrap();
        run_config(validate_config).unwrap();

        let _ = std::fs::remove_file(&artifact_path);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_consumes_product_artifact_dependency() {
        let base = std::env::temp_dir().join(format!(
            "rockc_product_dependency_{}_{}",
            std::process::id(),
            "smoke"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let dep_dir = base.join("dep");
        let app_dir = base.join("app");
        let dep_build = dep_dir.join("build");
        let app_build = app_dir.join("build");
        std::fs::create_dir_all(&dep_build).unwrap();
        std::fs::create_dir_all(&app_build).unwrap();
        let dep_entry = dep_dir.join("lib.rk");
        let app_entry = app_dir.join("main.rk");
        let dep_artifact = dep_build.join("dep.rkca");
        let dep_object = dep_build.join("dep.o");
        std::fs::write(&dep_entry, "answer = ->\n    5\n< answer\n").unwrap();
        std::fs::write(&app_entry, "> dep::answer\n\nmain = ->\n    answer!\n").unwrap();

        let dep_config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            dep_entry.to_str().unwrap(),
            "--output-dir",
            dep_build.to_str().unwrap(),
            "--no-std",
            "--no-prelude",
            "--no-link",
            "--crate-name",
            "dep",
            "--emit-artifact",
            dep_artifact.to_str().unwrap(),
            "--emit-object",
            dep_object.to_str().unwrap(),
        ])
        .unwrap();
        run_config(dep_config).unwrap();

        let extern_artifact = format!("dep={}", dep_artifact.display());
        let app_config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            app_entry.to_str().unwrap(),
            "--output-dir",
            app_build.to_str().unwrap(),
            "--no-std",
            "--no-prelude",
            "--extern-artifact",
            extern_artifact.as_str(),
        ])
        .unwrap();
        run_config(app_config).unwrap();

        let executable = app_build.join("main");
        assert!(executable.exists());
        let status = std::process::Command::new(&executable).status().unwrap();
        assert_eq!(status.code(), Some(5));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_rejects_extern_artifact_name_mismatch() {
        let base = std::env::temp_dir().join(format!(
            "rockc_product_dependency_{}_{}",
            std::process::id(),
            "name_mismatch"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let dep_dir = base.join("dep");
        let app_dir = base.join("app");
        let dep_build = dep_dir.join("build");
        let app_build = app_dir.join("build");
        std::fs::create_dir_all(&dep_build).unwrap();
        std::fs::create_dir_all(&app_build).unwrap();
        let dep_entry = dep_dir.join("lib.rk");
        let app_entry = app_dir.join("main.rk");
        let dep_artifact = dep_build.join("actual.rkca");
        let dep_object = dep_build.join("actual.o");
        std::fs::write(&dep_entry, "answer = ->\n    5\n< answer\n").unwrap();
        std::fs::write(&app_entry, "> actual::answer\n\nmain = ->\n    answer!\n").unwrap();

        let dep_config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            dep_entry.to_str().unwrap(),
            "--output-dir",
            dep_build.to_str().unwrap(),
            "--no-std",
            "--no-prelude",
            "--no-link",
            "--crate-name",
            "actual",
            "--emit-artifact",
            dep_artifact.to_str().unwrap(),
            "--emit-object",
            dep_object.to_str().unwrap(),
        ])
        .unwrap();
        run_config(dep_config).unwrap();

        let extern_artifact = format!("dep={}", dep_artifact.display());
        let app_config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            app_entry.to_str().unwrap(),
            "--output-dir",
            app_build.to_str().unwrap(),
            "--no-std",
            "--no-prelude",
            "--extern-artifact",
            extern_artifact.as_str(),
        ])
        .unwrap();

        assert!(run_config(app_config).is_err());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_run_config_consumes_product_artifact_generic_dependency() {
        let base = std::env::temp_dir().join(format!(
            "rockc_product_dependency_{}_{}",
            std::process::id(),
            "generic"
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let dep_dir = base.join("dep");
        let app_dir = base.join("app");
        let dep_build = dep_dir.join("build");
        let app_build = app_dir.join("build");
        std::fs::create_dir_all(&dep_build).unwrap();
        std::fs::create_dir_all(&app_build).unwrap();
        let dep_entry = dep_dir.join("lib.rk");
        let app_entry = app_dir.join("main.rk");
        let dep_artifact = dep_build.join("dep.rkca");
        let dep_object = dep_build.join("dep.o");
        std::fs::write(&dep_entry, "identity = x ->\n    x\n< identity\n").unwrap();
        std::fs::write(&app_entry, "> dep::identity\n\nmain = ->\n    identity 5\n").unwrap();

        let dep_config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            dep_entry.to_str().unwrap(),
            "--output-dir",
            dep_build.to_str().unwrap(),
            "--no-std",
            "--no-prelude",
            "--no-link",
            "--crate-name",
            "dep",
            "--emit-artifact",
            dep_artifact.to_str().unwrap(),
            "--emit-object",
            dep_object.to_str().unwrap(),
        ])
        .unwrap();
        run_config(dep_config).unwrap();

        let extern_artifact = format!("dep={}", dep_artifact.display());
        let app_config = Config::try_parse_from([
            "rockc",
            "--entry-file",
            app_entry.to_str().unwrap(),
            "--output-dir",
            app_build.to_str().unwrap(),
            "--no-std",
            "--no-prelude",
            "--extern-artifact",
            extern_artifact.as_str(),
        ])
        .unwrap();
        run_config(app_config).unwrap();

        let executable = app_build.join("main");
        assert!(executable.exists());
        let status = std::process::Command::new(&executable).status().unwrap();
        assert_eq!(status.code(), Some(5));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_removed_dependency_flags_are_rejected() {
        assert!(Config::try_parse_from([
            "rockc",
            "--entry-file",
            "main.rk",
            "--extern-crate",
            "dep=/tmp/dep.rk",
        ])
        .is_err());

        assert!(Config::try_parse_from([
            "rockc",
            "--entry-file",
            "main.rk",
            "--crate-path",
            "/tmp/dep",
        ])
        .is_err());
    }
}
