use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clap::Parser;
use rock_lib::analysis::Analysis;
use rock_lib::diagnostic::{
    Diagnostic as RockDiagnostic, DiagnosticLocation, DiagnosticSeverity as RockSeverity,
    Diagnostics,
};
use rock_lib::{Config, SourceProvider, Span};
use tokio::sync::{Mutex, RwLock};
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

    /// Analyze for this target triple (defaults to the host), so cfg blocks
    /// and std match that platform.
    #[arg(long, value_parser = parse_target)]
    target: Option<String>,
}

fn parse_target(value: &str) -> std::result::Result<String, String> {
    rock_lib::target::Target::parse(value).map(|target| target.triple)
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
    target: Option<String>,
}

#[derive(Debug, Clone)]
struct Document {
    version: i32,
    text: String,
}

#[derive(Default)]
struct State {
    documents: HashMap<Url, Document>,
    analyses: HashMap<Url, (bool, Arc<Analysis>)>,
    pending: HashMap<Url, u64>,
    overlay_revision: u64,
    projects: HashMap<(PathBuf, PathBuf), rock::project::AnalysisProject>,
    diagnostic_entries: HashMap<Url, std::collections::BTreeMap<PathBuf, Vec<Diagnostic>>>,
    diagnostic_targets: HashMap<Url, PathBuf>,
}

impl State {
    fn invalidate_analyses(&mut self, changed: &Url, saved_root: Option<&Path>) -> u64 {
        self.overlay_revision += 1;
        let Ok(changed) = changed.to_file_path() else {
            return self.overlay_revision;
        };
        for uri in self.documents.keys() {
            let Ok(path) = uri.to_file_path() else {
                continue;
            };
            let root = rock::project::find_project_root(&path);
            let project = root
                .as_ref()
                .and_then(|root| self.projects.get(&(root.clone(), path.clone())));
            // Modules resolve beside the entry file or in its descendants.
            // Until target resolution succeeds, conservatively use the project root.
            let source_root = project
                .and_then(|project| project.entry_file.parent())
                .or(root.as_deref())
                .or_else(|| path.parent());
            if changed == path
                || source_root.is_some_and(|root| source_in_scope(&changed, root))
                || saved_root.is_some_and(|saved| root.as_deref() == Some(saved))
            {
                self.pending.insert(uri.clone(), self.overlay_revision);
                if let Some((current, _)) = self.analyses.get_mut(uri) {
                    *current = false;
                }
            }
        }
        self.overlay_revision
    }

    fn diagnostic_updates(
        &mut self,
        entry: &Path,
        trigger: &Url,
        diagnostics: &Diagnostics,
    ) -> Vec<(Url, i32, Vec<Diagnostic>)> {
        let previous = self
            .diagnostic_targets
            .insert(trigger.clone(), entry.to_path_buf());
        if let Some(previous) = previous.filter(|previous| previous != entry) {
            let retired = self.retire_diagnostic_target(&previous);
            let mut updates = self.update_target_diagnostics(entry, trigger, diagnostics);
            for update in retired {
                if !updates.iter().any(|current| current.0 == update.0) {
                    updates.push(update);
                }
            }
            return updates;
        }
        self.update_target_diagnostics(entry, trigger, diagnostics)
    }

    fn close_diagnostic_document(&mut self, uri: &Url) -> Vec<(Url, i32, Vec<Diagnostic>)> {
        self.diagnostic_entries.remove(uri);
        self.diagnostic_targets
            .remove(uri)
            .map_or_else(Vec::new, |entry| self.retire_diagnostic_target(&entry))
    }

    fn retire_diagnostic_target(&mut self, entry: &Path) -> Vec<(Url, i32, Vec<Diagnostic>)> {
        if self
            .diagnostic_targets
            .values()
            .any(|active| active == entry)
        {
            return Vec::new();
        }
        self.diagnostic_entries
            .iter_mut()
            .filter_map(|(uri, by_target)| {
                by_target.remove(entry)?;
                let document = self.documents.get(uri)?;
                Some((
                    uri.clone(),
                    document.version,
                    Self::merge_diagnostics(by_target),
                ))
            })
            .collect()
    }

    fn merge_diagnostics(
        by_target: &std::collections::BTreeMap<PathBuf, Vec<Diagnostic>>,
    ) -> Vec<Diagnostic> {
        let mut merged = Vec::new();
        for diagnostic in by_target.values().flatten() {
            if !merged.contains(diagnostic) {
                merged.push(diagnostic.clone());
            }
        }
        merged
    }

    fn update_target_diagnostics(
        &mut self,
        entry: &Path,
        trigger: &Url,
        diagnostics: &Diagnostics,
    ) -> Vec<(Url, i32, Vec<Diagnostic>)> {
        let mut updates = Vec::new();
        for (uri, document) in &self.documents {
            let converted = diagnostics
                .0
                .iter()
                .filter_map(|diagnostic| diagnostic_to_lsp(diagnostic, uri, &document.text))
                .collect::<Vec<_>>();
            let by_target = self.diagnostic_entries.entry(uri.clone()).or_default();
            if converted.is_empty() {
                if by_target.remove(entry).is_none() && uri != trigger {
                    continue;
                }
            } else {
                by_target.insert(entry.to_path_buf(), converted);
            }
            updates.push((
                uri.clone(),
                document.version,
                Self::merge_diagnostics(by_target),
            ));
        }
        updates
    }
}

struct Backend {
    client: Client,
    settings: Settings,
    state: Arc<RwLock<State>>,
    refresh_lock: Mutex<()>,
    inlay_hint_refresh: AtomicBool,
}

impl Backend {
    fn new(client: Client, settings: Settings) -> Self {
        Self {
            client,
            settings,
            state: Arc::new(RwLock::new(State::default())),
            refresh_lock: Mutex::new(()),
            inlay_hint_refresh: AtomicBool::new(false),
        }
    }

    async fn refresh(&self, revision: u64) {
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
        let lock = self.refresh_lock.lock().await;
        let (pending, providers) = {
            let state = self.state.read().await;
            if state.overlay_revision != revision {
                return;
            }
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
            (state.pending.clone(), providers)
        };
        let mut analyses = Vec::new();
        for (uri, dirty_revision) in pending {
            if self.state.read().await.overlay_revision != revision {
                break;
            }
            self.refresh_document(uri, dirty_revision, &providers, &mut analyses)
                .await;
        }
        drop(lock);
        if self.inlay_hint_refresh.load(Ordering::Relaxed)
            && self.state.read().await.overlay_revision == revision
        {
            let _ = self.client.inlay_hint_refresh().await;
        }
    }

    async fn refresh_document(
        &self,
        uri: Url,
        revision: u64,
        providers: &[SourceProvider],
        analyses: &mut Vec<(rock::project::AnalysisProject, AnalysisResult)>,
    ) {
        let Ok(path) = uri.to_file_path() else {
            return;
        };
        let project_root = rock::project::find_project_root(&path);
        let project_key = project_root
            .as_ref()
            .map(|root| (root.clone(), path.clone()));
        let cached_project = {
            let state = self.state.read().await;
            if state.pending.get(&uri) != Some(&revision) {
                return;
            }
            project_key
                .as_ref()
                .and_then(|key| state.projects.get(key))
                .cloned()
        };
        let project = match (cached_project, project_root) {
            (Some(project), _) => Ok(Some(project)),
            (None, Some(root)) => {
                let path = path.clone();
                let target = self.settings.target.clone();
                blocking(move || {
                    rock::project::resolve_analysis_project_for_source(
                        &root,
                        &path,
                        target.as_deref(),
                    )
                    .map(Some)
                    .map_err(|error| Diagnostics(vec![RockDiagnostic::for_project(error, root)]))
                })
                .await
            }
            (None, None) => Ok(None),
        };
        let (analysis, resolved_project) = match project {
            Ok(project) => (
                analyze_target(path, providers, &self.settings, project.as_ref(), analyses).await,
                project,
            ),
            Err(diagnostics) => (Err(diagnostics), None),
        };

        let mut state = self.state.write().await;
        if state.pending.get(&uri) != Some(&revision) {
            return;
        }
        state.pending.remove(&uri);

        let entry = resolved_project
            .as_ref()
            .map(|project| project.entry_file.clone())
            .or_else(|| uri.to_file_path().ok());
        if let (Some(key), Some(project)) = (project_key, resolved_project) {
            state.projects.insert(key, project);
        }

        let diagnostics = match analysis {
            Ok(analysis) => {
                state
                    .analyses
                    .insert(uri.clone(), (true, Arc::clone(&analysis)));
                analysis.warnings.clone()
            }
            Err(diagnostics) => {
                // Keep the last successful snapshot so a temporary edit error does not
                // disable hover and signature help across the whole document.
                diagnostics
            }
        };
        let Some(entry) = entry else {
            return;
        };
        let updates = state.diagnostic_updates(&entry, &uri, &diagnostics);
        drop(state);
        for (uri, version, diagnostics) in updates {
            self.client
                .publish_diagnostics(uri, diagnostics, Some(version))
                .await;
        }
    }
}

type AnalysisResult = std::result::Result<Arc<Analysis>, Diagnostics>;

fn source_in_scope(path: &Path, source_root: &Path) -> bool {
    path.starts_with(source_root)
        || dunce::canonicalize(path).is_ok_and(|path| path.starts_with(source_root))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> std::result::Result<T, Diagnostics> + Send + 'static,
) -> std::result::Result<T, Diagnostics> {
    tokio::task::spawn_blocking(work).await.map_err(|error| {
        Diagnostics(vec![RockDiagnostic::for_internal(format!(
            "language analysis task failed: {error}"
        ))])
    })?
}

async fn analyze_target(
    path: PathBuf,
    providers: &[SourceProvider],
    settings: &Settings,
    project: Option<&rock::project::AnalysisProject>,
    analyses: &mut Vec<(rock::project::AnalysisProject, AnalysisResult)>,
) -> AnalysisResult {
    if let Some(project) = project {
        if let Some((_, analysis)) = analyses.iter().find(|(target, _)| target == project) {
            return analysis.clone();
        }
    }
    let source_root = project
        .map_or(path.as_path(), |project| &project.entry_file)
        .parent();
    let providers = providers
        .iter()
        .filter(|provider| match provider {
            SourceProvider::Virtual { path, .. } => {
                source_root.is_none_or(|root| source_in_scope(path, root))
            }
            SourceProvider::Artifact { .. } => true,
        })
        .cloned()
        .collect();
    let config = analysis_config(path, providers, settings, project);
    let analysis = blocking(move || rock_lib::analyze(&config).map(Arc::new)).await;
    if let Some(project) = project {
        analyses.push((project.clone(), analysis.clone()));
    }
    analysis
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        self.inlay_hint_refresh.store(
            params
                .capabilities
                .workspace
                .and_then(|workspace| workspace.inlay_hint)
                .and_then(|hints| hints.refresh_support)
                .unwrap_or(false),
            Ordering::Relaxed,
        );
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
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
        let mut state = self.state.write().await;
        state.documents.insert(
            document.uri.clone(),
            Document {
                version: document.version,
                text: document.text,
            },
        );
        let revision = state.invalidate_analyses(&document.uri, None);
        drop(state);
        self.refresh(revision).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.into_iter().last() else {
            return;
        };
        let uri = params.text_document.uri;
        let mut state = self.state.write().await;
        state.documents.insert(
            uri.clone(),
            Document {
                version: params.text_document.version,
                text: change.text,
            },
        );
        let revision = state.invalidate_analyses(&uri, None);
        drop(state);
        self.refresh(revision).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        let root = uri
            .to_file_path()
            .ok()
            .and_then(|path| rock::project::find_project_root(&path));
        let mut state = self.state.write().await;
        let revision = state.invalidate_analyses(&uri, root.as_deref());
        if let Some(root) = root {
            state
                .projects
                .retain(|(project_root, _), _| project_root != &root);
        }
        drop(state);
        self.refresh(revision).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        let lock = self.refresh_lock.lock().await;
        let mut state = self.state.write().await;
        state.documents.remove(&uri);
        state.analyses.remove(&uri);
        state.pending.remove(&uri);
        let updates = state.close_diagnostic_document(&uri);
        let revision = state.invalidate_analyses(&uri, None);
        drop(state);
        self.client
            .publish_diagnostics(uri.clone(), Vec::new(), None)
            .await;
        for (uri, version, diagnostics) in updates {
            self.client
                .publish_diagnostics(uri, diagnostics, Some(version))
                .await;
        }
        drop(lock);
        self.refresh(revision).await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let position = params.text_document_position_params.position;
        let uri = params.text_document_position_params.text_document.uri;
        let state = self.state.read().await;
        let Some(document) = state.documents.get(&uri) else {
            return Ok(None);
        };
        let Some((_, analysis)) = state.analyses.get(&uri) else {
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

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        let state = self.state.read().await;
        let (Some(document), Some((true, analysis)), Ok(path)) = (
            state.documents.get(&uri),
            state.analyses.get(&uri),
            uri.to_file_path(),
        ) else {
            return Ok(None);
        };
        Ok(Some(type_hints_to_lsp(
            analysis,
            &path,
            &document.text,
            params.range,
        )))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let position = params.text_document_position_params.position;
        let uri = params.text_document_position_params.text_document.uri;
        let state = self.state.read().await;
        let Some(document) = state.documents.get(&uri) else {
            return Ok(None);
        };
        let Some((_, analysis)) = state.analyses.get(&uri) else {
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
        crate_kind: if project.is_some_and(|project| project.current_crate_name.is_some()) {
            rock_lib::CrateKind::Lib
        } else {
            rock_lib::CrateKind::Bin
        },
        no_prelude: settings.no_prelude
            || project
                .map(|project| project.no_prelude)
                .unwrap_or(!has_std),
        no_std: project.map(|project| project.no_std).unwrap_or(true),
        target: settings.target.clone(),
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
                    .get(label.span.file_path.as_path())
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

fn type_hints_to_lsp(analysis: &Analysis, path: &Path, text: &str, range: Range) -> Vec<InlayHint> {
    let (Some(start), Some(end)) = (byte_offset(text, range.start), byte_offset(text, range.end))
    else {
        return Vec::new();
    };
    analysis
        .type_hints(path, start..end)
        .into_iter()
        .map(|hint| InlayHint {
            position: position_at(text, hint.offset),
            label: InlayHintLabel::String(format!(": {}", hint.ty)),
            kind: Some(InlayHintKind::TYPE),
            text_edits: None,
            tooltip: None,
            padding_left: None,
            padding_right: None,
            data: None,
        })
        .collect()
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
        target: args.target,
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
mod tests;
