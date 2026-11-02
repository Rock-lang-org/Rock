use crate::diagnostic::{self, Diagnostics};
use crate::source_loader;

fn diagnostic_source(source: source_loader::SourceFile) -> diagnostic::DiagnosticSource {
    let origin = match source.origin {
        source_loader::SourceOrigin::FileSystem => diagnostic::DiagnosticSourceOrigin::FileSystem,
        source_loader::SourceOrigin::Virtual => diagnostic::DiagnosticSourceOrigin::Virtual,
        source_loader::SourceOrigin::Artifact { artifact_path } => {
            diagnostic::DiagnosticSourceOrigin::Artifact { artifact_path }
        }
    };
    diagnostic::DiagnosticSource {
        display_path: source.display_path,
        text: source.text,
        origin,
        related: std::collections::BTreeMap::new(),
    }
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
                            diagnostic.source = Some(diagnostic_source(source));
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
                    let diagnostic_source = diagnostic_source(source);
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
            source_loader::SourceLoadError::Expansion {
                diagnostics: expansion_diagnostics,
                ..
            } => {
                diagnostics.merge(expansion_diagnostics);
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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
