use std::path::PathBuf;

use rock_lib::hir::AcceptedHirProgram;
use rock_lib::{Config, SourceProvider};

const PROTOCOL: &str = r#"
lang sized
< trait LayoutKnown
lang object_owner
< trait Transport State for F _
    lang owner_into_parts
    unsafe split: F T -> (*T, State) where T: ?LayoutKnown
    lang owner_from_parts
    unsafe rebuild: *T -> State -> F T where T: ?LayoutKnown
lang object_owner_allocate
< trait Reservation State for F _
    lang owner_allocate
    unsafe reserve: I64 -> I64 -> (*U8, State)
    lang owner_cancel
    unsafe abandon: *U8 -> State -> ()
main = !-> ()
"#;

fn config(source: &str) -> Config {
    let path = PathBuf::from("/virtual/owner_cancellation/main.rk");
    Config {
        entry_file: path.clone(),
        current_crate_name: Some("reservation_provider".into()),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual {
            path,
            text: source.into(),
        }],
        ..Config::default()
    }
}

fn program() -> AcceptedHirProgram {
    rock_lib::analyze(&config(PROTOCOL)).unwrap().hir.program
}

#[test]
fn renamed_allocation_protocol_retains_separate_canonical_cancel_authority() {
    let program = program();
    let items = program
        .language_items
        .object_owner_allocate
        .as_ref()
        .unwrap();
    let declaration = &program.traits[&items.trait_id];
    assert_eq!(declaration.name, "Reservation");
    assert_eq!(items.method_id, declaration.signatures["reserve"].id);
    assert_eq!(items.cancel_id, declaration.signatures["abandon"].id);
    assert_ne!(items.method_id, items.cancel_id);
    assert!(program.language_items.object_owner_unique.is_none());
}

#[test]
fn allocation_protocol_requires_a_marked_cancel_member() {
    for source in [
        PROTOCOL.replace(
            "    lang owner_cancel\n    unsafe abandon: *U8 -> State -> ()\n",
            "",
        ),
        PROTOCOL.replace("    lang owner_cancel\n", ""),
    ] {
        let diagnostics = rock_lib::analyze(&config(&source)).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|error| error.message.contains("object_owner_allocate")
                    && error.message.contains("missing a required operation")),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn cancellation_requires_unsafe_static_payload_independent_exact_state_transport() {
    for (signature, expected) in [
        ("abandon: *U8 -> State -> ()", "must be unsafe"),
        ("unsafe @abandon: *U8 -> State -> ()", "receiver"),
        ("unsafe ^@abandon: *U8 -> State -> ()", "receiver"),
        ("unsafe ~@abandon: *U8 -> State -> ()", "receiver"),
        ("unsafe abandon: *T -> State -> ()", "payload-dependent"),
        (
            "unsafe abandon: *U8 -> I64 -> ()",
            "signature *U8 -> State -> ()",
        ),
        (
            "unsafe abandon: *U8 -> State -> I64",
            "signature *U8 -> State -> ()",
        ),
        (
            "unsafe abandon: *U8 -> State -> State",
            "signature *U8 -> State -> ()",
        ),
    ] {
        let source = PROTOCOL.replace("unsafe abandon: *U8 -> State -> ()", signature);
        let diagnostics = rock_lib::analyze(&config(&source)).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|error| error.message.contains("owner_cancel")
                    && error.message.contains(expected)),
            "{signature}: {diagnostics:?}"
        );
    }
}

#[test]
fn cancellation_receiver_diagnostic_uses_the_marked_members_source_span() {
    for receiver in ["@", "^@", "~@"] {
        for declaration in [
            format!("unsafe {receiver}abandon: *U8 -> State -> ()"),
            format!(
                "unsafe abandon: *U8 -> State -> ()\n    unsafe {receiver}abandon = pointer, state !-> ()"
            ),
        ] {
            let source = PROTOCOL.replace("unsafe abandon: *U8 -> State -> ()", &declaration);
            let diagnostics = rock_lib::analyze(&config(&source)).unwrap_err();
            let diagnostic = diagnostics
                .0
                .iter()
                .find(|diagnostic| {
                    diagnostic.message.contains("object_owner_allocate.owner_cancel")
                        && diagnostic.message.contains("abandon")
                        && diagnostic.message.contains("static")
                        && diagnostic.message.contains("receiver")
                })
                .unwrap_or_else(|| panic!("missing precise receiver diagnostic: {diagnostics:?}"));
            let span = diagnostic.primary.as_ref().unwrap().span.clone();
            assert_eq!(span.file_path, config(&source).entry_file);
            assert_eq!(&source[span.start..span.end], "abandon");
            assert!(diagnostics
                .0
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("ambiguous type")));
        }
    }
}

#[test]
fn cancellation_rejects_wrong_member_kind_or_protocol_parent() {
    for source in [
        PROTOCOL.replace("unsafe abandon: *U8 -> State -> ()", "type Abandoned"),
        PROTOCOL.replace("lang owner_cancel", "lang owner_release"),
    ] {
        let diagnostics = rock_lib::analyze(&config(&source)).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|error| error.message.contains("language item")),
            "{diagnostics:?}"
        );
    }
}
