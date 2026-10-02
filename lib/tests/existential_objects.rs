use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::hir::{HirExprKindFor, HirStmtFor, HirTraitDispatchKind};
use rock_lib::types::Type;
use rock_lib::{Config, SourceProvider};

static NEXT: AtomicUsize = AtomicUsize::new(0);

const REBUILD: &str = r#"
trait Rebuild
    @rebuild: Self
    @accept: &Self -> ()
    @same: &Self
struct Token
    < value: I64
impl Rebuild for Token
    @rebuild = -> Token
        value: self.value
    @accept = other !-> ()
    @same = -> self
"#;

fn config(body: &str) -> Config {
    let directory = std::env::temp_dir().join(format!(
        "rock_existential_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let entry = directory.join("main.rk");
    Config {
        entry_file: entry.clone(),
        output_dir: directory,
        current_crate_name: Some("existential_objects".into()),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual {
            path: entry,
            text: format!("{REBUILD}\n{body}"),
        }],
        ..Default::default()
    }
}

#[test]
fn scoped_open_relates_self_results_and_arguments_with_rigid_authority() {
    let analysis = rock_lib::analyze(&config(
        r#"
exercise: &Rebuild -> ()
exercise = object !->
    open object as Hidden, value
        rebuilt = value.rebuild!
        value.accept &rebuilt
main = !-> ()
"#,
    ))
    .expect("opening must allow descriptor-backed Self storage without annotations");
    let exercise = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "exercise")
        .unwrap();
    let HirStmtFor::Expr(expression) = &exercise.body.stmts[0] else {
        panic!("expected opening");
    };
    let HirExprKindFor::Open {
        binding,
        body,
        source,
    } = &expression.kind
    else {
        panic!("expected typed opening");
    };
    assert_eq!(binding.source_ty, source.ty);
    assert_eq!(binding.witness.owner, exercise.id);
    assert_eq!(binding.witness.local, binding.value.local_id);
    assert_eq!(
        binding.value.ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Witness(binding.witness))
        }
    );
    let HirStmtFor::Let { ty, value, .. } = &body.stmts[0] else {
        panic!("expected hidden local");
    };
    assert_eq!(ty, &Type::Witness(binding.witness));
    let HirExprKindFor::MethodCall(_, _, _, _, target) = &value.kind else {
        panic!("expected opened dispatch");
    };
    assert!(
        matches!(target.target, rock_lib::hir::HirSelectedMethodTarget::TraitMethod { dispatch: HirTraitDispatchKind::Opened(witness), .. } if witness == binding.witness)
    );
}

#[test]
fn opened_self_reference_can_reclose_to_its_proven_object_view() {
    let analysis = rock_lib::analyze(&config(
        r#"
relay: &Rebuild -> &Rebuild
relay = object ->
    open object as Hidden, value
        value.same!
main = !-> ()
"#,
    ))
    .expect("expected object result must reclose the scoped witness");
    let relay = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "relay")
        .unwrap();
    let HirStmtFor::Expr(expression) = &relay.body.stmts[0] else {
        panic!("expected opening");
    };
    let HirExprKindFor::Open { binding, body, .. } = &expression.kind else {
        panic!("expected opening");
    };
    let HirStmtFor::Expr(expression) = body.stmts.last().unwrap() else {
        panic!("expected result");
    };
    let HirExprKindFor::ObjectCoercion(_, coercion) = &expression.kind else {
        panic!("expected reclosing coercion");
    };
    let Some(rock_lib::hir::HirObjectEvidence::Concrete { source, views }) = &coercion.evidence
    else {
        panic!("expected scoped dictionary evidence");
    };
    assert_eq!(source, &Type::Witness(binding.witness));
    assert!(views
        .iter()
        .all(|view| matches!(view.origin, rock_lib::hir::HirObjectWitnessOrigin::Bound)));
    assert_eq!(coercion.target, relay.ret_type);
}

fn assert_witness_rejected(body: &str) {
    let diagnostics =
        rock_lib::analyze(&config(body)).expect_err("witness escape must be rejected by frontend");
    assert!(
        diagnostics.0.iter().any(|diagnostic| {
            diagnostic.primary.is_some() && diagnostic.message.to_lowercase().contains("witness")
        }),
        "expected source-backed witness error, got {diagnostics:?}"
    );
}

#[test]
fn scoped_open_cannot_return_its_unbound_self_result() {
    assert_witness_rejected(
        r#"
escape: &Rebuild -> ()
escape = object ->
    open object as Hidden, value
        value.rebuild!
main = !-> ()
"#,
    );
}

#[test]
fn scoped_open_cannot_escape_through_an_early_return() {
    assert_witness_rejected(
        r#"
escape: &Rebuild -> ()
escape = object !->
    open object as Hidden, value
        return value.rebuild!
main = !-> ()
"#,
    );
}

#[test]
fn independently_opened_objects_do_not_share_self_identity() {
    let diagnostics = rock_lib::analyze(&config(
        r#"
exercise: &Rebuild -> &Rebuild -> ()
exercise = first, second !->
    open first as A, left
        open second as B, right
            rebuilt = right.rebuild!
            left.accept &rebuilt
main = !-> ()
"#,
    ))
    .expect_err("independent witnesses must not unify");
    assert!(
        diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.primary.is_some()
                && diagnostic.message.to_lowercase().contains("witness")),
        "{diagnostics:?}"
    );
}
