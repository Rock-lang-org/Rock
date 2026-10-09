//! Caller evidence must resolve dereference shape without guessing pointer kind.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::diagnostic::DiagnosticCode;
use rock_lib::types::Type;
use rock_lib::{Config, SourceProvider};

const DECLARATIONS: &str = r#"
lang sized
< trait LayoutKnown
struct Audit
    < releases: I64
"#;

fn config(body: &str) -> Config {
    let path = PathBuf::from("/virtual/raw_helper_inference/main.rk");
    Config {
        entry_file: path.clone(),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual {
            path,
            text: format!("{DECLARATIONS}{body}"),
        }],
        ..Config::default()
    }
}

#[test]
fn inferred_dereference_uses_actual_raw_shared_or_mutable_input_evidence() {
    for argument in ["((&mut audit) as *Audit)", "(&audit)", "(&mut audit)"] {
        for helper_first in [false, true] {
            let helper = "read = input -> unsafe (*input).releases\n";
            let caller = format!(
                "main = ->\n    mut audit = Audit\n        releases: 42\n    read {argument}\n"
            );
            let source = if helper_first {
                format!("{helper}{caller}")
            } else {
                format!("{caller}{helper}")
            };
            let program = rock_lib::analyze(&config(&source))
                .unwrap_or_else(|errors| {
                    panic!("{argument}, helper_first={helper_first}: {errors:?}")
                })
                .hir
                .program;
            let audit = program
                .structs
                .values()
                .find(|item| item.name == "Audit")
                .unwrap();
            let audit_ty = Type::Struct {
                id: audit.id,
                args: vec![],
            };
            let expected = match argument {
                "((&mut audit) as *Audit)" => Type::Pointer(Box::new(audit_ty)),
                _ => Type::Reference {
                    mutable: argument == "(&mut audit)",
                    inner: Box::new(audit_ty),
                },
            };
            let helper = program
                .functions
                .values()
                .find(|item| item.name == "read")
                .unwrap();
            assert_eq!(helper.params.len(), 1);
            assert_eq!(helper.params[0].ty, expected);
            assert_eq!(helper.ret_type, Type::I64);
        }
    }
}

fn reject(body: &str, messages: &[&str]) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let mut config = config(body);
    config.no_link = true;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures");
    std::fs::create_dir_all(&root).expect("project-local rejection fixture root");
    config.output_dir = root.join(format!(
        "rock_raw_helper_rejection_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    std::fs::create_dir(&config.output_dir).expect("fresh rejection output directory");
    let result = rock_lib::compile(&config);
    std::fs::remove_dir_all(&config.output_dir).expect("remove only this fixture's output");
    let errors = result.expect_err("unsupported dereference must reject");
    assert!(
        errors
            .0
            .iter()
            .all(|error| error.code != Some(DiagnosticCode::Internal)),
        "{errors:?}"
    );
    let error = errors
        .0
        .iter()
        .find(|error| {
            matches!(
                error.code,
                Some(DiagnosticCode::Type | DiagnosticCode::Selection | DiagnosticCode::Borrow)
            ) && messages
                .iter()
                .any(|message| error.message.contains(message))
        })
        .unwrap_or_else(|| panic!("expected {messages:?}: {errors:?}"));
    let span = &error.primary.as_ref().expect("real source primary").span;
    assert_eq!(span.file_path, config.entry_file);
    assert!(span.start < span.end && span.end <= DECLARATIONS.len() + body.len());
}

#[test]
fn known_scalar_dereference_is_not_admitted_by_unsafe() {
    reject(
        "main = -> unsafe *42\n",
        &["Cannot dereference non-pointer"],
    );
}

#[test]
fn inferred_raw_pointer_dereference_retains_unsafe_requirements() {
    reject(
        "read = input -> (*input).releases\nmain = ->\n    mut audit = Audit\n        releases: 42\n    read ((&mut audit) as *Audit)\n",
        &["unsafe"],
    );
}

#[test]
fn inferred_shared_input_does_not_grant_mutation() {
    reject(
        "write = input !-> unsafe (*input).releases = 42\nmain = !->\n    audit = Audit\n        releases: 0\n    write (&audit)\n",
        &["immutable", "shared", "mutable"],
    );
}

#[test]
fn unconstrained_helper_does_not_guess_raw_pointer_or_reference_kind() {
    reject(
        "read = input -> unsafe (*input).releases\nmain = !-> ()\n",
        &[
            "ambiguous",
            "cannot determine",
            "Cannot dereference non-pointer",
        ],
    );
}
