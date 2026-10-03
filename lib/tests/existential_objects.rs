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
fn unused_opening_value_has_a_source_backed_declaration_hover() {
    let config = config(
        r#"
inspect: &Rebuild -> ()
inspect = object !->
    open object as Hidden, value
        ()
main = !-> ()
"#,
    );
    let SourceProvider::Virtual { path, text } = &config.source_providers[0] else {
        panic!("expected virtual source");
    };
    let offset = text.find("as Hidden, value").unwrap() + "as Hidden, ".len();
    let analysis = rock_lib::analyze(&config).expect("opening declaration must remain inspectable");
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "inspect")
        .unwrap();
    let HirStmtFor::Expr(expression) = &function.body.stmts[0] else {
        panic!("expected opening");
    };
    let HirExprKindFor::Open { binding, .. } = &expression.kind else {
        panic!("expected opening");
    };
    let (symbol, span) = analysis.hir.source_map.symbol_at(path, offset).unwrap();
    assert_eq!(
        symbol,
        rock_lib::source_map::SourceSymbol::Local {
            owner: function.id,
            local: binding.value.local_id
        }
    );
    assert_eq!(&text[span.start..span.end], "value");
    let hover = analysis.hover(path, offset).unwrap();
    assert!(
        hover.contents.contains("scoped witness of Rebuild"),
        "{hover:?}"
    );
    assert!(!hover.contents.contains("Token"), "{hover:?}");
}

#[test]
fn invalid_opening_source_reports_its_own_span_and_supported_handles() {
    let config = config(
        r#"
main = !->
    open 42 as Hidden, value
        ()
"#,
    );
    let SourceProvider::Virtual { text, .. } = &config.source_providers[0] else {
        panic!("expected virtual source");
    };
    let diagnostics = rock_lib::analyze(&config).expect_err("integer is not an object handle");
    let diagnostic = diagnostics
        .0
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .contains("open requires a borrowed object or a protocol-backed object owner")
        })
        .unwrap();
    let primary = diagnostic.primary.as_ref().unwrap();
    assert_eq!(&text[primary.span.start..primary.span.end], "42");
}

#[test]
fn a_witness_capturing_closure_can_be_called_inside_its_opening() {
    rock_lib::analyze(&config(
        r#"
inspect: &Rebuild -> ()
inspect = object !->
    open object as Hidden, value
        callback = !-> value.accept value
        callback!
main = !-> ()
"#,
    ))
    .expect("a closure retains the witness and dictionary authority while its opening dominates");
}

#[test]
fn mutable_opened_receivers_keep_explicit_reborrows_and_original_local_access() {
    let analysis = rock_lib::analyze(&config(
        r#"
trait Cell
    @read: I64
    ^@set: I64 -> ()
inspect: &mut Cell -> I64
inspect = object ->
    open object as Hidden, value
        value.set 42
        value.read!
main = !-> ()
"#,
    ))
    .expect(
        "exclusive opening permits exclusive and shared reborrows without relabeling its local",
    );
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "inspect")
        .unwrap();
    let HirStmtFor::Expr(expression) = &function.body.stmts[0] else {
        panic!("expected opening")
    };
    let HirExprKindFor::Open { binding, body, .. } = &expression.kind else {
        panic!("expected opening")
    };
    for (statement, mutable) in body.stmts.iter().zip([true, false]) {
        let HirStmtFor::Expr(expression) = statement else {
            panic!("expected call")
        };
        let HirExprKindFor::MethodCall(receiver, _, _, mode, _) = &expression.kind else {
            panic!("expected method call")
        };
        assert_eq!(
            *mode,
            Some(if mutable {
                rock_lib::types::ReceiverMode::Mut
            } else {
                rock_lib::types::ReceiverMode::Shared
            })
        );
        assert_eq!(
            receiver.ty,
            Type::Reference {
                mutable,
                inner: Box::new(Type::Witness(binding.witness))
            }
        );
        let HirExprKindFor::Ref(access, place) = &receiver.kind else {
            panic!("witness call must retain its explicit reborrow")
        };
        assert_eq!(*access, mutable);
        let HirExprKindFor::Deref(original) = &place.kind else {
            panic!("expected borrowed witness place")
        };
        assert_eq!(original.ty, binding.value.ty);
    }
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
fn scoped_open_cannot_escape_through_an_outward_break() {
    assert_witness_rejected(
        r#"
escape: &Rebuild -> ()
escape = object !->
    loop
        open object as Hidden, value
            break value.rebuild!
main = !-> ()
"#,
    );
}

#[test]
fn outward_break_escape_has_the_inferred_values_exact_source_span() {
    for loop_header in ["loop", "while true", "for item in [1, 2]"] {
        let body = format!(
            "escape: &Rebuild -> ()\nescape = object !->\n    {loop_header}\n        open object as Hidden, value\n            break value.rebuild!\nmain = !-> ()\n"
        );
        let configuration = config(&body);
        let source = format!("{REBUILD}\n{body}");
        let diagnostics = rock_lib::analyze(&configuration).unwrap_err();
        let diagnostic = diagnostics
            .0
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("opened witness escapes through a break")
            })
            .unwrap_or_else(|| {
                panic!("missing source escape diagnostic for {loop_header}: {diagnostics:?}")
            });
        let span = &diagnostic
            .primary
            .as_ref()
            .expect("escape value source span")
            .span;
        assert_eq!(span.file_path, configuration.entry_file);
        assert_eq!(&source[span.start..span.end], "value.rebuild!");
        assert!(
            diagnostics.0.iter().all(|diagnostic| diagnostic.code
                != Some(rock_lib::diagnostic::DiagnosticCode::Internal)),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn scoped_break_keeps_its_exact_loop_result_type() {
    let body = "exercise: &Rebuild -> ()\nexercise = object !->\n    open object as Hidden, value\n        loop\n            break value.rebuild!\nmain = !-> ()\n";
    let configuration = config(body);
    let source = format!("{REBUILD}\n{body}");
    let diagnostics = rock_lib::analyze(&configuration).unwrap_err();
    let diagnostic = diagnostics
        .0
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .contains("opened witness break result disagrees with its loop target")
        })
        .unwrap_or_else(|| panic!("missing exact loop-target diagnostic: {diagnostics:?}"));
    let span = &diagnostic.primary.as_ref().unwrap().span;
    assert_eq!(span.file_path, configuration.entry_file);
    assert_eq!(&source[span.start..span.end], "value.rebuild!");
    assert!(
        diagnostics
            .0
            .iter()
            .all(|diagnostic| diagnostic.code
                != Some(rock_lib::diagnostic::DiagnosticCode::Internal)),
        "{diagnostics:?}"
    );
}

#[test]
fn independent_and_valueless_breaks_do_not_export_opened_witnesses() {
    for control in ["break", "break 42"] {
        let body = format!(
            "exercise: &Rebuild -> ()\nexercise = object !->\n    loop\n        open object as Hidden, value\n            rebuilt = value.rebuild!\n            value.accept &rebuilt\n            {control}\nmain = !-> ()\n"
        );
        let analysis = rock_lib::analyze(&config(&body))
            .expect("independent break must retain local witness authority");
        let function = analysis
            .hir
            .program
            .functions
            .values()
            .find(|function| function.name == "exercise")
            .unwrap();
        let HirStmtFor::Expr(expression) = &function.body.stmts[0] else {
            panic!("expected loop")
        };
        let HirExprKindFor::Loop(loop_body) = &expression.kind else {
            panic!("expected loop")
        };
        let HirStmtFor::Expr(open) = &loop_body.stmts[0] else {
            panic!("expected opening")
        };
        let HirExprKindFor::Open { body, .. } = &open.kind else {
            panic!("expected opening")
        };
        assert_eq!(open.ty, Type::Never);
        assert_eq!(body.ty, Type::Never);
        assert!(matches!(body.stmts.last(), Some(HirStmtFor::Break(_))));
    }
}

#[test]
fn scoped_open_cannot_return_a_closure_capturing_its_witness() {
    assert_witness_rejected(
        r#"
escape = object ->
    open (object as &Rebuild) as Hidden, value
        -> value.accept value
main = !-> ()
"#,
    );
}

#[test]
fn witness_forwarded_through_an_inferred_helper_keeps_opened_method_authority() {
    let analysis = rock_lib::analyze(&config(
        r#"
forward = item -> item
exercise: &Rebuild -> ()
exercise = object !->
    open object as Hidden, value
        rebuilt = forward (value.rebuild!)
        rebuilt.accept &rebuilt
main = !-> ()
"#,
    ))
    .expect("deferred selection must use the dominating witness schema");
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
    let HirExprKindFor::Open { binding, body, .. } = &expression.kind else {
        panic!("expected opening");
    };
    let HirStmtFor::Expr(expression) = body.stmts.last().unwrap() else {
        panic!("expected method call");
    };
    let HirExprKindFor::MethodCall(receiver, _, _, _, target) = &expression.kind else {
        panic!("expected opened dispatch");
    };
    assert_eq!(
        receiver.ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Witness(binding.witness))
        }
    );
    assert!(
        matches!(target.target, rock_lib::hir::HirSelectedMethodTarget::TraitMethod { dispatch: HirTraitDispatchKind::Opened(witness), .. } if witness == binding.witness)
    );
}

#[test]
fn injected_default_method_rebinds_opening_and_dispatch_to_its_body_owner() {
    let analysis = rock_lib::analyze(&config(
        r#"
trait Runner
    @exercise: &Rebuild -> ()
    @exercise = object !->
        open object as Hidden, value
            rebuilt = value.rebuild!
            value.accept &rebuilt
impl Runner for Token
main = !-> ()
"#,
    ))
    .expect("injected default body must remap its scoped witness authority");
    let method = analysis
        .hir
        .program
        .impls
        .values()
        .flat_map(|implementation| implementation.methods.values())
        .find(|method| method.name == "exercise")
        .unwrap();
    let HirStmtFor::Expr(expression) = &method.body.stmts[0] else {
        panic!("expected opening");
    };
    let HirExprKindFor::Open { binding, body, .. } = &expression.kind else {
        panic!("expected opening");
    };
    assert_eq!(binding.witness.owner, method.id);
    let HirStmtFor::Expr(expression) = body.stmts.last().unwrap() else {
        panic!("expected method call");
    };
    let HirExprKindFor::MethodCall(_, _, _, _, target) = &expression.kind else {
        panic!("expected opened dispatch");
    };
    assert!(
        matches!(target.target, rock_lib::hir::HirSelectedMethodTarget::TraitMethod { dispatch: HirTraitDispatchKind::Opened(witness), .. } if witness == binding.witness)
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
