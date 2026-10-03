use std::path::PathBuf;

use crate::diagnostic::{DiagnosticCode, DiagnosticLocation, Diagnostics};
use crate::lexer::Span;

fn config(source: &str) -> crate::Config {
    let path = PathBuf::from("/tmp/opencode/rock-package-diagnostics/main.rk");
    crate::Config {
        entry_file: path.clone(),
        no_std: true,
        no_prelude: true,
        source_providers: vec![crate::SourceProvider::Virtual {
            path,
            text: source.into(),
        }],
        ..Default::default()
    }
}

fn reject_at(
    source: &str,
    code: DiagnosticCode,
    message: &str,
    start: usize,
    fragment: &str,
) -> Diagnostics {
    let config = config(source);
    let diagnostics = match crate::analyze(&config) {
        Err(diagnostics) => diagnostics,
        Ok(_) => panic!("expected source rejection: {message}"),
    };
    let diagnostic = diagnostics
        .0
        .iter()
        .find(|diagnostic| diagnostic.message == message)
        .unwrap_or_else(|| panic!("missing exact diagnostic {message:?}: {diagnostics:?}"));
    assert_eq!(diagnostic.code, Some(code));
    let expected = Span::new(config.entry_file, start, start + fragment.len());
    assert_eq!(
        diagnostic.location,
        DiagnosticLocation::Source(expected.clone())
    );
    assert_eq!(diagnostic.primary.as_ref().unwrap().span, expected);
    assert_eq!(&source[expected.start..expected.end], fragment);
    assert!(
        diagnostic.source.is_some(),
        "diagnostic must retain its virtual source"
    );
    assert!(diagnostics
        .0
        .iter()
        .all(|diagnostic| diagnostic.code != Some(DiagnosticCode::Internal)));
    diagnostics
}

#[test]
fn shared_package_mutable_member_reports_type_at_member_without_field_cascade() {
    for (bounds, declarations) in [
        ("Change", "trait Change\n    ^@revise: I64 -> ()\n"),
        ("Derived", "trait Change\n    ^@revise: I64 -> ()\ntrait Derived where Self: Change\n    @read: I64\n"),
        ("Read { Change }", "trait Change\n    ^@revise: I64 -> ()\ntrait Read\n    @read: I64\n"),
    ] {
        let source = format!("{declarations}\
type Shared = exists T where T: {bounds} in &T
exercise: Shared -> ()
exercise = package !->
    open package as Hidden, value
        value.revise 42
main = !-> ()
");
        let start = source.rfind("revise 42").unwrap();
        let diagnostics = reject_at(&source, DiagnosticCode::Type,
            "Cannot call mutable receiver method 'revise' through shared package access", start, "revise");
        assert!(diagnostics.0.iter().all(|diagnostic| {
            let message = diagnostic.message.to_lowercase();
            !message.contains("unknown field") && !message.contains("no implementation")
        }), "access rejection must not fall through to unrelated member selection: {diagnostics:?}");
    }
}

#[test]
fn shared_package_member_probe_preserves_shared_and_mutable_positive_candidates() {
    for access in ["&T", "&mut T"] {
        let source = format!(
            r#"
trait Read
    @read: I64
trait Change
    ^@revise: I64 -> ()
type View = exists T where T: Read {{ Change }} in {access}
exercise: View -> ()
exercise = package !->
    open package as Hidden, value
        value.read!
{}
main = !-> ()
"#,
            if access == "&mut T" {
                "        value.revise 42"
            } else {
                ""
            }
        );
        if let Err(diagnostics) = crate::analyze(&config(&source)) {
            panic!("existing canonical access must remain selectable: {diagnostics:?}");
        }
    }
}

#[test]
fn unrelated_package_member_is_not_mislabeled_as_mutable_access() {
    let source = r#"
trait Change
    ^@revise: I64 -> ()
type Shared = exists T where T: Change in &T
exercise: Shared -> ()
exercise = package !->
    open package as Hidden, value
        value.missing!
main = !-> ()
"#;
    let diagnostics = match crate::analyze(&config(source)) {
        Err(diagnostics) => diagnostics,
        Ok(_) => panic!("unknown member must be rejected"),
    };
    assert!(diagnostics
        .0
        .iter()
        .all(|diagnostic| !diagnostic.message.contains("through shared package access")));
}

#[test]
fn borrowed_package_opening_has_explicit_source_type_rejection() {
    let source = r#"
trait Read
    @read: I64
type Shared = exists T where T: Read in &T
exercise: &Shared -> ()
exercise = package !->
    open package as Hidden, value
        ()
main = !-> ()
"#;
    reject_at(
        source,
        DiagnosticCode::Type,
        "borrowed-package opening is not implemented; open an existing package by value",
        source.rfind("package as Hidden").unwrap(),
        "package",
    );
}

#[test]
fn constructor_kind_package_admission_rejects_descriptor_application_at_binder() {
    let source = r#"
trait Read
    @read: I64
type Higher = exists (F _) where F: Read in (&(F I64), &(F Bool))
main = !-> ()
"#;
    reject_at(
        source,
        DiagnosticCode::Resolve,
        "constructor-kind existential descriptor application is not implemented",
        source.find("F _").unwrap(),
        "F _",
    );
}

#[test]
fn relaxed_unsized_package_admission_rejects_at_exact_existential_type() {
    for target in [
        "exists (T: ?Read) where T: Read in &T",
        "exists T where T: ?Read in &T",
    ] {
        let source =
            format!("trait Read\n    @read: I64\ntype Unsized = {target}\nmain = !-> ()\n");
        reject_at(
            &source,
            DiagnosticCode::Resolve,
            "existential packages currently require a Sized witness",
            source.find(target).unwrap(),
            target,
        );
    }
}

#[test]
fn owner_target_pack_reports_explicit_unsupported_target_not_an_ordinary_call() {
    let source = r#"
trait Read
    @read: I64
type Shared = exists T where T: Read in &T
struct Holder T
    < pointer: *T
exercise: I64 -> ()
exercise = number !->
    pack (Holder Shared) (&number)
main = !-> ()
"#;
    let diagnostics = reject_at(
        source,
        DiagnosticCode::Type,
        "owner-target pack/destination construction is not implemented",
        source.find("Holder Shared").unwrap(),
        "Holder Shared",
    );
    assert!(diagnostics
        .0
        .iter()
        .all(|diagnostic| !diagnostic.message.contains("Unknown identifier 'pack'")));
}
