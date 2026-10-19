use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use rock_lib::analysis::Analysis;
use rock_lib::diagnostic::{
    Diagnostic as RockDiagnostic, DiagnosticLocation, DiagnosticSeverity as RockSeverity,
    Diagnostics,
};
use rock_lib::{Config, SourceProvider, Span};
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Debug, Parser)]
#[command(name = "rock-lsp", about = "Language server for Rock")]
struct Args {
    /// Override or add a product artifact, as name=path.
    #[arg(long = "extern-artifact", value_parser = parse_named_path)]
    extern_artifacts: Vec<(String, PathBuf)>,

    /// Do not inject a dependency-provided prelude.
    #[arg(long)]
    no_prelude: bool,
}

fn parse_named_path(value: &str) -> std::result::Result<(String, PathBuf), String> {
    let (name, path) = value
        .split_once('=')
        .ok_or_else(|| "expected name=path".to_string())?;
    if name.is_empty() || path.is_empty() {
        return Err("expected non-empty name=path".to_string());
    }
    Ok((name.to_string(), PathBuf::from(path)))
}

#[derive(Debug, Clone)]
struct Settings {
    extern_artifacts: Vec<(String, PathBuf)>,
    no_prelude: bool,
}

#[derive(Debug, Clone)]
struct Document {
    version: i32,
    text: String,
}

#[derive(Default)]
struct State {
    documents: HashMap<Url, Document>,
    analyses: HashMap<Url, Arc<Analysis>>,
    projects: HashMap<(PathBuf, PathBuf), rock::project::AnalysisProject>,
}

struct Backend {
    client: Client,
    settings: Settings,
    state: Arc<RwLock<State>>,
}

impl Backend {
    fn new(client: Client, settings: Settings) -> Self {
        Self {
            client,
            settings,
            state: Arc::new(RwLock::new(State::default())),
        }
    }

    async fn refresh(&self, uri: Url) {
        let (document, providers) = {
            let state = self.state.read().await;
            let Some(document) = state.documents.get(&uri).cloned() else {
                return;
            };
            let providers = state
                .documents
                .iter()
                .filter_map(|(uri, document)| {
                    uri.to_file_path().ok().map(|path| SourceProvider::Virtual {
                        path,
                        text: document.text.clone(),
                    })
                })
                .collect::<Vec<_>>();
            (document, providers)
        };
        let Ok(path) = uri.to_file_path() else {
            return;
        };
        let project_root = rock::project::find_project_root(&path);
        let project_key = project_root.as_ref().map(|root| (root.clone(), path.clone()));
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
        let current = self
            .state
            .read()
            .await
            .documents
            .get(&uri)
            .is_some_and(|current| current.version == document.version);
        if !current {
            return;
        }

        let settings = self.settings.clone();
        let cached_project = {
            let state = self.state.read().await;
            project_key
                .as_ref()
                .and_then(|key| state.projects.get(key))
                .cloned()
        };
        let task_project_root = project_root.clone();
        let task = tokio::task::spawn_blocking(move || {
            let project = match (cached_project, task_project_root.as_deref()) {
                (Some(project), _) => Some(project),
                (None, Some(root)) => match rock::project::resolve_analysis_project_for_source(root, &path) {
                    Ok(project) => Some(project),
                    Err(error) => {
                        let mut diagnostics = Diagnostics::default();
                        diagnostics.push(RockDiagnostic::for_project(error, root.to_path_buf()));
                        return (Err(diagnostics), None);
                    }
                },
                (None, None) => None,
            };
            let config = analysis_config(path, providers, &settings, project.as_ref());
            (rock_lib::analyze(&config), project)
        })
        .await;

        let (analysis, resolved_project) = match task {
            Ok(result) => result,
            Err(error) => {
                let mut diagnostics = Diagnostics::default();
                diagnostics.push(RockDiagnostic::for_internal(format!(
                    "language analysis task failed: {error}"
                )));
                (Err(diagnostics), None)
            }
        };

        let current = self
            .state
            .read()
            .await
            .documents
            .get(&uri)
            .is_some_and(|current| current.version == document.version);
        if !current {
            return;
        }

        if let (Some(key), Some(project)) = (project_key, resolved_project) {
            self.state.write().await.projects.insert(key, project);
        }

        match analysis {
            Ok(analysis) => {
                self.state
                    .write()
                    .await
                    .analyses
                    .insert(uri.clone(), Arc::new(analysis));
                self.client
                    .publish_diagnostics(uri, Vec::new(), Some(document.version))
                    .await;
            }
            Err(diagnostics) => {
                // Keep the last successful snapshot so a temporary edit error does not
                // disable hover and signature help across the whole document.
                let documents = self.state.read().await.documents.clone();
                for (diagnostic_uri, diagnostic_document) in documents {
                    let converted = diagnostics
                        .0
                        .iter()
                        .filter_map(|diagnostic| {
                            diagnostic_to_lsp(
                                diagnostic,
                                &diagnostic_uri,
                                &diagnostic_document.text,
                            )
                        })
                        .collect::<Vec<_>>();
                    if diagnostic_uri == uri || !converted.is_empty() {
                        self.client
                            .publish_diagnostics(
                                diagnostic_uri,
                                converted,
                                Some(diagnostic_document.version),
                            )
                            .await;
                    }
                }
            }
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec![",".to_string()]),
                    retrigger_characters: None,
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "rock-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Rock language server initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let document = params.text_document;
        self.state.write().await.documents.insert(
            document.uri.clone(),
            Document {
                version: document.version,
                text: document.text,
            },
        );
        self.refresh(document.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.into_iter().last() else {
            return;
        };
        let uri = params.text_document.uri;
        self.state.write().await.documents.insert(
            uri.clone(),
            Document {
                version: params.text_document.version,
                text: change.text,
            },
        );
        self.refresh(uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Ok(path) = uri.to_file_path() {
            if let Some(root) = rock::project::find_project_root(&path) {
            self.state.write().await.projects.retain(|(project_root, _), _| project_root != &root);
            }
        }
        self.refresh(uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        let mut state = self.state.write().await;
        state.documents.remove(&uri);
        state.analyses.remove(&uri);
        drop(state);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let position = params.text_document_position_params.position;
        let uri = params.text_document_position_params.text_document.uri;
        let state = self.state.read().await;
        let Some(document) = state.documents.get(&uri) else {
            return Ok(None);
        };
        let Some(analysis) = state.analyses.get(&uri) else {
            return Ok(None);
        };
        let Ok(path) = uri.to_file_path() else {
            return Ok(None);
        };
        let Some(offset) = byte_offset(&document.text, position) else {
            return Ok(None);
        };
        let Some(info) = analysis.hover(&path, offset) else {
            return Ok(None);
        };

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!("```rock\n{}\n```", info.contents),
            }),
            range: Some(span_range(&info.span, &document.text)),
        }))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let position = params.text_document_position_params.position;
        let uri = params.text_document_position_params.text_document.uri;
        let state = self.state.read().await;
        let Some(document) = state.documents.get(&uri) else {
            return Ok(None);
        };
        let Some(analysis) = state.analyses.get(&uri) else {
            return Ok(None);
        };
        let Ok(path) = uri.to_file_path() else {
            return Ok(None);
        };
        let Some(offset) = byte_offset(&document.text, position) else {
            return Ok(None);
        };
        let Some(info) = analysis.signature(&path, offset) else {
            return Ok(None);
        };

        Ok(Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label: info.label,
                documentation: None,
                parameters: Some(
                    info.parameters
                        .into_iter()
                        .map(|parameter| ParameterInformation {
                            label: ParameterLabel::Simple(parameter),
                            documentation: None,
                        })
                        .collect(),
                ),
                active_parameter: Some(info.active_parameter as u32),
            }],
            active_signature: Some(0),
            active_parameter: Some(info.active_parameter as u32),
        }))
    }
}

fn analysis_config(
    source_path: PathBuf,
    providers: Vec<SourceProvider>,
    settings: &Settings,
    project: Option<&rock::project::AnalysisProject>,
) -> Config {
    let mut artifacts = BTreeMap::new();
    if let Some(project) = project {
        artifacts.extend(project.extern_artifacts.iter().cloned());
    }
    artifacts.extend(settings.extern_artifacts.iter().cloned());
    let has_std = artifacts.contains_key("std");

    Config {
        entry_file: project
            .map(|project| project.entry_file.clone())
            .unwrap_or(source_path),
        source_providers: providers,
        extern_artifacts: artifacts.into_iter().collect(),
        current_crate_name: project.and_then(|project| project.current_crate_name.clone()),
        no_prelude: settings.no_prelude
            || project
                .map(|project| project.no_prelude)
                .unwrap_or(!has_std),
        no_std: project.map(|project| project.no_std).unwrap_or(true),
        ..Config::default()
    }
}

fn diagnostic_to_lsp(diagnostic: &RockDiagnostic, uri: &Url, text: &str) -> Option<Diagnostic> {
    let path = uri.to_file_path().ok()?;
    match &diagnostic.location {
        DiagnosticLocation::Project(project) if !path.starts_with(project) => return None,
        DiagnosticLocation::File(file) | DiagnosticLocation::Artifact(file) if file != &path => {
            return None;
        }
        _ => {}
    }
    let span = diagnostic
        .primary
        .as_ref()
        .map(|label| &label.span)
        .or_else(|| match &diagnostic.location {
            DiagnosticLocation::Source(span) => Some(span),
            _ => None,
        });
    if span.is_some_and(|span| span.file_path != path) {
        return None;
    }

    let mut message = diagnostic.message.clone();
    if let Some(primary) = &diagnostic.primary {
        if !primary.message.is_empty() && primary.message != diagnostic.message {
            message.push_str("\n");
            message.push_str(&primary.message);
        }
    }
    for note in &diagnostic.notes {
        message.push_str("\nnote: ");
        message.push_str(note);
    }
    for help in &diagnostic.help {
        message.push_str("\nhelp: ");
        message.push_str(help);
    }

    let related_information = diagnostic
        .secondary
        .iter()
        .filter_map(|label| {
            let (related_uri, related_text) = if label.span.file_path == path {
                (uri.clone(), text)
            } else {
                let source = diagnostic.source.as_ref()?;
                let related_text = source
                    .related
                    .get(&label.span.file_path)
                    .map(|related| related.text.as_str())
                    .or_else(|| {
                        (source.display_path == label.span.file_path)
                            .then_some(source.text.as_str())
                    })?;
                (
                    Url::from_file_path(&label.span.file_path).ok()?,
                    related_text,
                )
            };
            Some(DiagnosticRelatedInformation {
                location: Location::new(related_uri, span_range(&label.span, related_text)),
                message: label.message.clone(),
            })
        })
        .collect::<Vec<_>>();

    Some(Diagnostic {
        range: span
            .map(|span| span_range(span, text))
            .unwrap_or_else(|| Range::new(Position::new(0, 0), Position::new(0, 0))),
        severity: Some(match diagnostic.severity {
            RockSeverity::Error => DiagnosticSeverity::ERROR,
            RockSeverity::Warning => DiagnosticSeverity::WARNING,
            RockSeverity::Info => DiagnosticSeverity::INFORMATION,
            RockSeverity::Hint => DiagnosticSeverity::HINT,
        }),
        code: diagnostic
            .code
            .map(|code| NumberOrString::String(code.as_str().to_string())),
        code_description: None,
        source: Some("rock".to_string()),
        message,
        related_information: (!related_information.is_empty()).then_some(related_information),
        tags: None,
        data: None,
    })
}

fn span_range(span: &Span, text: &str) -> Range {
    Range::new(position_at(text, span.start), position_at(text, span.end))
}

fn byte_offset(text: &str, position: Position) -> Option<usize> {
    let mut line = 0_u32;
    let mut line_start = 0;
    for (index, ch) in text.char_indices() {
        if line == position.line {
            line_start = index;
            break;
        }
        if ch == '\n' {
            line += 1;
            line_start = index + 1;
        }
    }
    if line != position.line {
        if position.line == line && line_start == text.len() {
            return Some(text.len());
        }
        return None;
    }

    let mut utf16_column = 0_u32;
    for (relative, ch) in text[line_start..].char_indices() {
        if ch == '\n' || utf16_column >= position.character {
            return Some(line_start + relative);
        }
        utf16_column += ch.len_utf16() as u32;
        if utf16_column > position.character {
            return Some(line_start + relative);
        }
    }
    (utf16_column <= position.character).then_some(text.len())
}

fn position_at(text: &str, offset: usize) -> Position {
    let offset = offset.min(text.len());
    let offset = (0..=offset)
        .rev()
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(0);
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    let character = prefix[line_start..].encode_utf16().count() as u32;
    Position::new(line, character)
}

fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        // Recursive compiler passes exceed Tokio's default stack on ordinary projects.
        // This also covers spawn_blocking analysis and snapshot drops on async workers.
        .thread_stack_size(32 * 1024 * 1024)
        .enable_all()
        .build()
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let settings = Settings {
        no_prelude: args.no_prelude,
        extern_artifacts: args.extern_artifacts,
    };
    runtime()?.block_on(async move {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let (service, socket) = LspService::new(|client| Backend::new(client, settings));
        Server::new(stdin, stdout, socket).serve(service).await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rock_lib::diagnostic::Diagnostic as RockDiagnostic;
    use rock_lib::Span;
    use tower_lsp::lsp_types::{DiagnosticSeverity, Position, Url};

    use super::{analysis_config, byte_offset, diagnostic_to_lsp, position_at, Settings};

    #[test]
    fn mixed_package_binary_entries_analyze_with_their_own_library() {
        let root = std::env::temp_dir().join(format!("rock_lsp_multi_target_{}", std::process::id()));
        std::fs::create_dir_all(root.join("src/bin")).unwrap();
        std::fs::write(root.join("rock.toml"), concat!(
            "[crate]\nname = \"mixed\"\nversion = \"0.1.0\"\nno_std = true\n\n",
            "[lib]\n\n[[bin]]\n\n[[bin]]\nname = \"other\"\n",
        )).unwrap();
        std::fs::write(root.join("src/lib.rk"), "< answer = -> 42\n").unwrap();
        let settings = Settings { extern_artifacts: Vec::new(), no_prelude: false };
        for entry in ["src/main.rk", "src/bin/other.rk"] {
            let path = root.join(entry);
            std::fs::write(&path, "> mixed::answer\nmain = -> answer!\n").unwrap();
            let project = rock::project::resolve_analysis_project_for_source(&root, &path).unwrap();
            assert_eq!(project.entry_file, path);
            assert!(project.extern_artifacts.iter().any(|(name, _)| name == "mixed"));
            let config = analysis_config(path, Vec::new(), &settings, Some(&project));
            let result = rock_lib::analyze(&config);
            assert!(result.is_ok(), "{:?}", result.err());
        }
        let project = rock::project::resolve_analysis_project(&root).unwrap();
        assert_eq!(project.entry_file, root.join("src/lib.rk"));
        assert!(!project.extern_artifacts.iter().any(|(name, _)| name == "mixed"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_analyzes_nested_source_on_blocking_worker() {
        super::runtime().unwrap().block_on(async {
            tokio::task::spawn_blocking(|| {
                let path = PathBuf::from("/virtual/nested.rk");
                let source = format!(
                    "main = ->\n    value = {}0{}\n    0\n",
                    "[".repeat(32),
                    "]".repeat(32),
                );
                let config = analysis_config(
                    path.clone(),
                    vec![rock_lib::SourceProvider::Virtual { path, text: source }],
                    &Settings {
                        extern_artifacts: Vec::new(),
                        no_prelude: true,
                    },
                    None,
                );
                assert!(rock_lib::analyze(&config).is_ok());
            })
            .await
            .unwrap();
        });
    }

    #[test]
    fn positions_use_utf16_code_units() {
        let text = "a😀b\nrock";
        assert_eq!(byte_offset(text, Position::new(0, 3)), Some(5));
        assert_eq!(position_at(text, 5), Position::new(0, 3));
        assert_eq!(byte_offset(text, Position::new(1, 2)), Some(9));
    }

    #[test]
    fn diagnostics_convert_byte_ranges_to_utf16_ranges() {
        let path = PathBuf::from("/virtual/main.rk");
        let uri = Url::from_file_path(&path).unwrap();
        let diagnostic = RockDiagnostic::new("bad expression".to_string(), Span::new(path, 1, 5));

        let converted = diagnostic_to_lsp(&diagnostic, &uri, "a😀b").unwrap();
        assert_eq!(converted.range.start, Position::new(0, 1));
        assert_eq!(converted.range.end, Position::new(0, 3));
        assert_eq!(converted.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(converted.source.as_deref(), Some("rock"));
    }

    #[test]
    fn project_configuration_discovers_artifacts_and_applies_explicit_overrides() {
        let project = rock::project::AnalysisProject {
            root_dir: PathBuf::from("/project"),
            entry_file: PathBuf::from("/project/src/main.rk"),
            current_crate_name: Some("app".to_string()),
            extern_artifacts: vec![
                ("dep".to_string(), PathBuf::from("/project/build/dep.rkca")),
                (
                    "std".to_string(),
                    PathBuf::from("/toolchain/std.rkca"),
                ),
            ],
            no_prelude: false,
            no_std: false,
        };
        let settings = Settings {
            extern_artifacts: vec![
                ("dep".to_string(), PathBuf::from("/override/dep.rkca")),
                ("extra".to_string(), PathBuf::from("/override/extra.rkca")),
            ],
            no_prelude: false,
        };

        let config = analysis_config(
            PathBuf::from("/project/src/module.rk"),
            Vec::new(),
            &settings,
            Some(&project),
        );

        assert_eq!(config.entry_file, PathBuf::from("/project/src/main.rk"));
        assert_eq!(config.current_crate_name.as_deref(), Some("app"));
        assert!(!config.no_prelude);
        assert!(!config.no_std);
        assert_eq!(
            config.extern_artifacts,
            vec![
                ("dep".to_string(), PathBuf::from("/override/dep.rkca")),
                ("extra".to_string(), PathBuf::from("/override/extra.rkca")),
                (
                    "std".to_string(),
                    PathBuf::from("/toolchain/std.rkca")
                ),
            ]
        );
    }
}
