use super::*;
use crate::{Config, SourceProvider};

#[test]
fn callable_body_local_sources_survive_source_free_instantiation_and_missing_location() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(format!(
            ".agent-work/fixtures/callable-body-source-{}",
            std::process::id()
        ));
    std::fs::create_dir_all(&root).unwrap();
    let entry = root.join("absent.rk");
    let source = "borrowed: T -> &T\n< borrowed = value ->\n    alias = value\n    match alias\n        selected => &selected\n";
    let mut products = crate::compile_with_products(&Config {
        entry_file: entry.clone(),
        output_dir: root.clone(),
        current_crate_name: Some("stored".into()),
        no_std: true,
        no_prelude: true,
        no_link: true,
        emit_object: Some(root.join("stored.o")),
        source_providers: vec![SourceProvider::Virtual {
            path: entry.clone(),
            text: source.into(),
        }],
        ..Default::default()
    })
    .unwrap()
    .products
    .unwrap();
    products.link.object_path = Some("stored.o".into());
    assert!(!entry.exists());
    let (&id, row) = products
        .bodies
        .callable_sources
        .callables
        .iter()
        .next()
        .unwrap();
    assert_eq!(row.locals.len(), 3);
    let (&local, span) = row
        .locals
        .iter()
        .find(|(_, span)| &source[span.start..span.end] == "selected")
        .unwrap();
    let span = span.clone();
    let artifact = root.join("stored.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let frozen = std::fs::read(&artifact).unwrap();
    let object = std::fs::read(root.join("stored.o")).unwrap();
    let mut missing = products.clone();
    missing
        .bodies
        .callable_sources
        .callables
        .get_mut(&id)
        .unwrap()
        .locals
        .remove(&local);
    let missing_artifact = root.join("missing.rkca");
    missing.write_artifact_to_path(&missing_artifact).unwrap();

    for level in [0, 3] {
        for (path, has_location) in [(&artifact, true), (&missing_artifact, false)] {
            let caller = root.join(format!("consumer{level}_{has_location}.rk"));
            let diagnostics = crate::compile(&Config {
                entry_file: caller.clone(),
                output_dir: root.clone(),
                no_std: true,
                no_prelude: true,
                no_link: true,
                opt_level: level,
                extern_artifacts: vec![("stored".into(), path.clone())],
                source_providers: vec![SourceProvider::Virtual {
                    path: caller,
                    text: "> stored::borrowed\nmain = !->\n    borrowed 42\n".into(),
                }],
                ..Default::default()
            })
            .expect_err("returning a reference to the pattern local must remain rejected");
            let errors = diagnostics.0.iter().filter(|diagnostic| diagnostic.message == "Cannot return reference to local value because it does not live long enough").collect::<Vec<_>>();
            assert!(!errors.is_empty(), "{diagnostics:?}");
            if has_location {
                assert!(
                    diagnostics.0.iter().all(|diagnostic| diagnostic.code
                        != Some(crate::diagnostic::DiagnosticCode::Internal)),
                    "{diagnostics:?}"
                );
                for error in errors {
                    assert_eq!(error.code, Some(crate::diagnostic::DiagnosticCode::Borrow));
                    assert_eq!(error.primary.as_ref().unwrap().span, span);
                    let source = error.source.as_ref().unwrap();
                    assert!(
                        matches!(&source.origin, crate::diagnostic::DiagnosticSourceOrigin::Artifact { artifact_path } if artifact_path == &artifact)
                    );
                }
            } else {
                assert!(
                    errors.iter().any(|error| error.code
                        == Some(crate::diagnostic::DiagnosticCode::Internal)
                        && error.primary.is_none()),
                    "no fallback location may replace missing producer metadata: {diagnostics:?}"
                );
            }
            assert!(!entry.exists());
            assert_eq!(std::fs::read(&artifact).unwrap(), frozen);
            assert_eq!(std::fs::read(root.join("stored.o")).unwrap(), object);
        }
    }
}

#[test]
fn callable_source_transport_is_exact_remapped_source_free_and_diagnostic_only() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(format!(
            ".agent-work/fixtures/callable-source-{}",
            std::process::id()
        ));
    std::fs::create_dir_all(&root).unwrap();
    let entry = root.join("absent.rk");
    let source = "keep: T -> T\n< keep = value -> value\n";
    let mut products = crate::compile_with_products(&Config {
        entry_file: entry.clone(),
        output_dir: root.clone(),
        current_crate_name: Some("stored".into()),
        no_std: true,
        no_prelude: true,
        no_link: true,
        emit_object: Some(root.join("stored.o")),
        source_providers: vec![SourceProvider::Virtual {
            path: entry.clone(),
            text: source.into(),
        }],
        ..Default::default()
    })
    .unwrap()
    .products
    .unwrap();
    products.link.object_path = Some("stored.o".into());
    assert!(!entry.exists());
    let (id, row) = products
        .bodies
        .callable_sources
        .callables
        .iter()
        .next()
        .unwrap();
    let id = *id;
    assert_eq!(row.locals.len(), 1);
    let (local, span) = row.locals.iter().next().unwrap();
    let local = *local;
    let span = span.clone();
    assert_eq!(&source[span.start..span.end], "value");
    assert_eq!(span.file_path, entry);
    let artifact = root.join("stored.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let frozen = std::fs::read(&artifact).unwrap();
    let decoded = CompilerProducts::read_artifact_from_path(&artifact).unwrap();
    assert_eq!(
        decoded.bodies.callable_sources.callables[&id].locals[&local],
        span
    );
    let mut context = crate::crate_system::CrateContext::new();
    context
        .load_product_artifact_from_path(artifact.clone())
        .unwrap();
    let dependency = context.extern_crates().next().unwrap();
    let imported_id = dependency.metadata().source_map().references();
    assert!(imported_id.is_empty(), "no fabricated semantic references");
    let function = dependency
        .bodies()
        .providers()
        .generic_functions()
        .values()
        .next()
        .unwrap();
    assert_ne!(function.id.crate_id.0, id.crate_id.0);
    assert_eq!(
        dependency
            .metadata()
            .source_map()
            .local_span(function.id, local),
        Some(&span)
    );
    let mut database = SourceDatabase::new();
    for (path, text) in dependency.metadata().source_files() {
        database
            .add_artifact_diagnostic_source(path.clone(), artifact.clone(), text.clone())
            .unwrap();
    }
    assert!(database.loaded_files().is_empty());
    let sources = crate::diagnostic::DiagnosticSourceMap::from_source_database(&database);
    assert_eq!(sources.source_for_path(&entry).unwrap().text, source);
    assert!(
        matches!(&sources.source_for_path(&entry).unwrap().origin, crate::diagnostic::DiagnosticSourceOrigin::Artifact { artifact_path } if artifact_path == &artifact)
    );
    for mutation in 0..8 {
        let mut changed = products.clone();
        let sources = &mut changed.bodies.callable_sources;
        match mutation {
            0 => {
                let row = sources.callables.remove(&id).unwrap();
                sources.callables.insert(
                    super::super::ProductDefId {
                        local_id: super::super::ProductLocalDefId(u32::MAX),
                        ..id
                    },
                    row,
                );
            }
            1 => {
                let row = sources.callables.get_mut(&id).unwrap();
                let span = row.locals.remove(&local).unwrap();
                row.locals.insert(crate::ids::HirLocalId(u32::MAX), span);
            }
            2 => {
                sources.files.clear();
            }
            3 => {
                sources
                    .callables
                    .get_mut(&id)
                    .unwrap()
                    .locals
                    .get_mut(&local)
                    .unwrap()
                    .end = source.len() + 1
            }
            4 => {
                sources.files.insert(entry.clone(), "é".into());
            }
            5 => {
                sources
                    .files
                    .insert(entry.clone(), format!("é{}", &source[2..]));
                sources
                    .callables
                    .get_mut(&id)
                    .unwrap()
                    .locals
                    .get_mut(&local)
                    .unwrap()
                    .start = 1;
            }
            6 => {
                sources
                    .callables
                    .get_mut(&id)
                    .unwrap()
                    .locals
                    .get_mut(&local)
                    .unwrap()
                    .start = source.len()
            }
            7 => {
                sources
                    .files
                    .insert(root.join("unregistered.rk"), "unreferenced".into());
            }
            _ => unreachable!(),
        }
        assert!(
            changed.to_artifact_bytes().is_err(),
            "source emission mutation {mutation}"
        );
        let payload = super::super::type_table::products_to_artifact(
            &changed,
            super::super::PRODUCT_ARTIFACT_FORMAT_VERSION,
        )
        .unwrap();
        let bytes = super::super::encode_product_artifact(
            &super::super::ProductArtifactHeader::from_products(&changed),
            &payload,
        )
        .unwrap();
        assert!(
            CompilerProducts::from_artifact_bytes(&bytes).is_err(),
            "source decoding mutation {mutation}"
        );
    }
    assert!(frozen == std::fs::read(&artifact).unwrap());

    // Rendering transport unit control, not a newly claimed compiler rejection.
    let diagnostic = crate::diagnostic::Diagnostic::new("transport control".into(), span.clone());
    let rendered = crate::diagnostic::Diagnostics(vec![diagnostic]).with_sources(&sources);
    assert_eq!(rendered.0[0].primary.as_ref().unwrap().span, span);
    assert_eq!(rendered.0[0].source.as_ref().unwrap().text, source);
    assert!(matches!(&rendered.0[0].source.as_ref().unwrap().origin,
        crate::diagnostic::DiagnosticSourceOrigin::Artifact { artifact_path } if artifact_path == &artifact));
    assert!(!entry.exists());
    assert!(frozen == std::fs::read(&artifact).unwrap());
}
