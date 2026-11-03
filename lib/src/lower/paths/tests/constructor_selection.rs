use super::*;

fn find_static_method_target(expr: &crate::hir::AcceptedHirExpr) -> Option<&HirStaticMethodTarget> {
    match &expr.kind {
        crate::hir::HirExprKindFor::Call(
            _,
            _,
            Some(crate::hir::HirCallTarget::StaticMethod(target)),
        ) => Some(target),
        crate::hir::HirExprKindFor::Call(callee, args, _) => find_static_method_target(callee)
            .or_else(|| args.iter().find_map(find_static_method_target)),
        crate::hir::HirExprKindFor::Lambda { body, .. }
        | crate::hir::HirExprKindFor::Block(body)
        | crate::hir::HirExprKindFor::UnsafeBlock(body) => {
            body.stmts.iter().find_map(|stmt| match stmt {
                crate::hir::HirStmtFor::Let { value, .. }
                | crate::hir::HirStmtFor::Expr(value)
                | crate::hir::HirStmtFor::Return(Some(value))
                | crate::hir::HirStmtFor::Break(Some(value)) => find_static_method_target(value),
                _ => None,
            })
        }
        _ => None,
    }
}

#[test]
fn constructor_trait_selection_concrete_call_records_exact_impl_authority() {
    let hir = compile_source_to_resolved_hir_for_test(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Maybe T
    None
    Some T

impl Applicative for Maybe
    pure = value -> Maybe::Some value

main = ->
    value = Maybe::pure 42
    0
"#,
    );
    let main = hir
        .program
        .functions
        .values()
        .find(|function| function.name == "main")
        .expect("main function");
    let value = main
        .body
        .stmts
        .iter()
        .find_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Let { value, .. } => Some(value),
            _ => None,
        })
        .expect("lowered constructor call");
    let crate::hir::HirExprKindFor::Call(callee, _, _) = &value.kind else {
        panic!("expected call of static method value, got {:?}", value.kind);
    };
    let crate::hir::HirExprKindFor::Lambda { body, .. } = &callee.kind else {
        panic!("expected static method lambda, got {:?}", callee.kind);
    };
    let target = body
        .stmts
        .iter()
        .find_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Expr(crate::hir::HirExprFor {
                kind:
                    crate::hir::HirExprKindFor::Call(
                        _,
                        _,
                        Some(crate::hir::HirCallTarget::StaticMethod(target)),
                    ),
                ..
            }) => Some(target),
            _ => None,
        })
        .expect("static method authority");

    assert!(target.method.impl_id().is_some());
    assert!(target.method.trait_id().is_some());
    assert!(target.method.method_id().is_some());
    assert!(matches!(target.owner_ty, Type::Constructor { .. }));
}

#[test]
fn constructor_trait_selection_generic_calls_record_deferred_bound_authority() {
    let hir = compile_source_to_resolved_hir_for_test(
        r#"
trait Applicative for F _
    pure: A -> F A

make: A -> F A where F _: Applicative
make = value -> F::pure value

make_qualified: A -> F A where F _: Applicative
make_qualified = value -> F::Applicative::pure value
"#,
    );

    for function_name in ["make", "make_qualified"] {
        let function = hir
            .program
            .functions
            .values()
            .find(|function| function.name == function_name)
            .unwrap_or_else(|| panic!("{function_name} function"));
        let target = function
            .body
            .stmts
            .iter()
            .find_map(|stmt| match stmt {
                crate::hir::HirStmtFor::Expr(expr)
                | crate::hir::HirStmtFor::Return(Some(expr))
                | crate::hir::HirStmtFor::Let { value: expr, .. } => {
                    find_static_method_target(expr)
                }
                _ => None,
            })
            .expect("deferred constructor authority");

        assert!(target.method.impl_id().is_none());
        assert!(matches!(
            target.method.target,
            HirSelectedMethodTarget::TraitMethod {
                dispatch: HirTraitDispatchKind::TraitBound,
                ..
            }
        ));
        assert!(matches!(target.owner_ty, Type::Generic(_)));
        assert!(target.method.trait_id().is_some());
        assert!(target.method.method_id().is_some());
        assert!(!target.method.owner_substitution.is_empty());
    }
}

#[test]
fn constructor_trait_selection_result_section_preserves_fixed_error() {
    let hir = compile_source_to_resolved_hir_for_test(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Outcome T, E
    Ok T
    Err E

struct IoError

impl Applicative for Outcome _, E
    pure = value -> Outcome::Ok value

main = ->
    value = (Outcome _, IoError)::pure 42
    0
"#,
    );
    let io_error_id = hir
        .program
        .structs
        .values()
        .find(|structure| structure.name == "IoError")
        .expect("IoError structure")
        .id;
    let main = hir
        .program
        .functions
        .values()
        .find(|function| function.name == "main")
        .expect("main function");
    let target = main
        .body
        .stmts
        .iter()
        .find_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Let { value, .. } => find_static_method_target(value),
            _ => None,
        })
        .expect("selected Outcome section authority");

    assert!(target.method.impl_id().is_some());
    assert!(target.method.owner_substitution.iter().any(|binding| {
        binding.ty
            == Type::Struct {
                id: io_error_id,
                args: Vec::new(),
            }
    }));
}

#[test]
fn constructor_trait_selection_preserves_inherent_precedence_and_qualification() {
    let hir = compile_source_to_resolved_hir_for_test(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Maybe T
    None
    Some T

impl Maybe T
    pure = value -> Maybe::None

impl Applicative for Maybe
    pure = value -> Maybe::Some value

main = ->
    inherent: Maybe I64 = Maybe::pure 1
    qualified: Maybe I64 = Maybe::Applicative::pure 2
    0
"#,
    );
    let main = hir
        .program
        .functions
        .values()
        .find(|function| function.name == "main")
        .expect("main function");
    let targets = main
        .body
        .stmts
        .iter()
        .filter_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Let { value, .. } => find_static_method_target(value),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(targets.len(), 2);
    assert!(matches!(
        targets[0].method.target,
        HirSelectedMethodTarget::ImplMethod {
            selected_trait: None,
            ..
        }
    ));
    assert!(matches!(
        targets[1].method.target,
        HirSelectedMethodTarget::ImplMethod {
            selected_trait: Some(_),
            ..
        }
    ));
}
