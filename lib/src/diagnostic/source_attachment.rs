use super::*;

impl DiagnosticSourceMap {
    pub fn from_source_database(database: &crate::source_loader::SourceDatabase) -> Self {
        let mut sources = Self::default();
        for file in database.source_files() {
            sources.insert_source_file(file);
        }
        sources
    }

    pub fn source_for_path(&self, path: &Path) -> Option<&DiagnosticSource> {
        self.by_path.get(path)
    }

    pub fn attach(&self, diagnostic: &mut Diagnostic) {
        let DiagnosticLocation::Source(primary_span) = &diagnostic.location else {
            return;
        };
        let source = diagnostic
            .source
            .clone()
            .or_else(|| self.source_for_path(&primary_span.file_path).cloned());
        if let Some(mut source) = source {
            for label in diagnostic.primary.iter().chain(diagnostic.secondary.iter()) {
                let span = &label.span;
                if span.file_path != primary_span.file_path {
                    if let Some(related) = self.source_for_path(&span.file_path) {
                        source.related.insert(
                            span.file_path.clone(),
                            DiagnosticRelatedSource {
                                display_path: related.display_path.clone(),
                                text: related.text.clone(),
                            },
                        );
                    }
                }
            }
            diagnostic.source = Some(source);
            if diagnostic.primary.is_none() {
                diagnostic.primary = Some(DiagnosticLabel::new(
                    diagnostic.message.clone(),
                    primary_span.clone(),
                ));
            }
        }
    }

    fn insert_source_file(&mut self, file: &crate::source_loader::SourceFile) {
        let origin = match &file.origin {
            crate::source_loader::SourceOrigin::FileSystem => DiagnosticSourceOrigin::FileSystem,
            crate::source_loader::SourceOrigin::Virtual => DiagnosticSourceOrigin::Virtual,
            crate::source_loader::SourceOrigin::Artifact { artifact_path } => {
                DiagnosticSourceOrigin::Artifact {
                    artifact_path: artifact_path.clone(),
                }
            }
        };
        let source = DiagnosticSource {
            display_path: file.display_path.clone(),
            text: file.text.clone(),
            origin,
            related: BTreeMap::new(),
        };
        for path in [
            &file.original_path,
            &file.canonical_path,
            &file.display_path,
        ] {
            self.by_path.insert(path.clone(), source.clone());
        }
    }
}
