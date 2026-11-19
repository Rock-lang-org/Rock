use super::*;

fn import_path() -> Path {
    let mut program =
        crate::parser::parse_string("> macros::make\n", &crate::Config::default()).unwrap();
    match program.module.top_levels.remove(0) {
        crate::ast::TopLevel::Import(path) => path,
        _ => panic!("expected an import"),
    }
}

fn scope(file: &str, inline_modules: Vec<usize>) -> MacroImportScope {
    MacroImportScope {
        file_path: file.into(),
        inline_modules,
    }
}

#[test]
fn rebuilding_a_registry_keeps_the_same_import_identity_and_usage() {
    let mut uses = MacroImportUses::default();
    let scope = scope("entry.rk", Vec::new());
    let path = import_path();
    let first = uses.register(&scope, &path, "macros::make".into()).unwrap();
    uses.mark_used(first);
    let second = uses.register(&scope, &path, "macros::make".into()).unwrap();
    assert_eq!(first, second);
    let sites = uses.snapshot();
    assert_eq!(sites.len(), 1);
    assert!(sites[0].used);
}

#[test]
fn identical_template_spans_in_different_scopes_are_distinct() {
    let path = import_path();
    for other in [scope("entry.rk", vec![1]), scope("other.rk", vec![0])] {
        let mut uses = MacroImportUses::default();
        let first = uses
            .register(&scope("entry.rk", vec![0]), &path, "macros::make".into())
            .unwrap();
        let second = uses.register(&other, &path, "macros::make".into()).unwrap();
        assert_ne!(first, second);
        uses.mark_used(second);
        let sites = uses.snapshot();
        assert!(!sites[0].used);
        assert!(sites[1].used);
    }
}

#[test]
fn allowed_import_lints_do_not_record_macro_provenance() {
    let mut config = crate::Config::default();
    config
        .lint_levels
        .set("unused_imports", crate::lint::LintLevel::Allow)
        .unwrap();
    let context = crate::macro_expansion::MacroExpansionContext::new(&config);
    let id = context.register_import(
        &scope("entry.rk", Vec::new()),
        &import_path(),
        &["macros".into(), "make".into()],
    );
    assert!(id.is_none());
    assert!(context.import_usage().is_empty());
}
