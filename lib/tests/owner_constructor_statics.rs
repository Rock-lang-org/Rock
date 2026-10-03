use std::path::PathBuf;

use rock_lib::hir::{
    AcceptedHir, HirBlockFor, HirCallTarget, HirExprFor, HirExprKindFor, HirSelectedMethodTarget,
    HirStaticMethodTarget, HirStmtFor,
};
use rock_lib::types::{NominalTypeKind, Type};
use rock_lib::{Config, SourceProvider};

// Analysis-only provider: bodies do not implement a runtime allocator.
const PROVIDER: &str = r#"
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
struct Token
    < tag: I64
struct Carrier (T: ?LayoutKnown)
    < pointer: *T
impl Transport Token for Carrier
    unsafe split = owner ->
        state = Token
            tag: 0
        (owner.pointer, state)
    unsafe rebuild = pointer, _ -> Carrier
        pointer: pointer
impl Reservation Token for Carrier
    unsafe reserve = bytes, alignment ->
        state = Token
            tag: alignment
        (bytes as *U8, state)
    unsafe abandon = _, _ !-> ()
struct Other (T: ?LayoutKnown)
    < pointer: *T
struct NonGeneric
    < value: I64
"#;

fn config(body: &str) -> Config {
    let path = PathBuf::from("/virtual/owner_constructor_statics/main.rk");
    Config {
        entry_file: path.clone(),
        current_crate_name: Some("reservation_calls".into()),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual {
            path,
            text: format!("{PROVIDER}{body}"),
        }],
        ..Config::default()
    }
}

fn static_targets(block: &HirBlockFor<AcceptedHir>) -> Vec<&HirStaticMethodTarget> {
    fn expression<'a>(
        value: &'a HirExprFor<AcceptedHir>,
        out: &mut Vec<&'a HirStaticMethodTarget>,
    ) {
        match &value.kind {
            HirExprKindFor::Lambda { body, .. }
            | HirExprKindFor::Block(body)
            | HirExprKindFor::UnsafeBlock(body) => visit(body, out),
            HirExprKindFor::Call(callee, arguments, target) => {
                if let Some(HirCallTarget::StaticMethod(target)) = target {
                    out.push(target);
                }
                expression(callee, out);
                for argument in arguments {
                    expression(argument, out);
                }
            }
            _ => {}
        }
    }
    fn visit<'a>(block: &'a HirBlockFor<AcceptedHir>, out: &mut Vec<&'a HirStaticMethodTarget>) {
        for statement in &block.stmts {
            match statement {
                HirStmtFor::Let { value, .. } | HirStmtFor::Expr(value) => expression(value, out),
                _ => {}
            }
        }
    }
    let mut result = Vec::new();
    visit(block, &mut result);
    result
}

#[test]
fn renamed_constructor_statics_retain_marked_member_and_exact_state_authority() {
    for owner in ["Carrier", "Carrier::Reservation"] {
        let body = format!(
            "main = !->\n    allocation = unsafe {owner}::reserve 0, 64\n    unsafe {owner}::abandon allocation.0, allocation.1\n"
        );
        let program = rock_lib::analyze(&config(&body)).unwrap().hir.program;
        let allocation = program
            .language_items
            .object_owner_allocate
            .as_ref()
            .unwrap();
        let carrier = program
            .structs
            .values()
            .find(|definition| definition.name == "Carrier")
            .unwrap();
        let state = program
            .structs
            .values()
            .find(|definition| definition.name == "Token")
            .unwrap();
        let function = program
            .functions
            .values()
            .find(|function| function.name == "main")
            .unwrap();
        let targets = static_targets(&function.body);
        assert_eq!(targets.len(), 2, "{owner}");
        for (target, member) in targets
            .into_iter()
            .zip([allocation.method_id, allocation.cancel_id])
        {
            assert_eq!(
                target.owner_ty,
                Type::Constructor {
                    id: carrier.id,
                    flavor: NominalTypeKind::Struct
                }
            );
            let HirSelectedMethodTarget::ImplMethod {
                impl_id,
                selected_trait: Some(selected),
                ..
            } = &target.method.target
            else {
                panic!("expected exact constructor implementation")
            };
            assert_eq!(selected.trait_id, allocation.trait_id);
            assert_eq!(selected.member_id, member);
            assert_eq!(
                selected.trait_args,
                vec![Type::Struct {
                    id: state.id,
                    args: Vec::new()
                }]
            );
            assert_eq!(program.impls[impl_id].trait_id, Some(allocation.trait_id));
        }
        assert!(program.language_items.object_owner_unique.is_none());
    }
}

#[test]
fn allocation_statics_reject_an_unimplemented_owner() {
    let diagnostics =
        rock_lib::analyze(&config("main = !->\n    unsafe Other::reserve 0, 64\n")).unwrap_err();
    assert!(
        diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Other::reserve")
                && diagnostic
                    .message
                    .contains("no matching static implementation")),
        "{diagnostics:?}"
    );
}

#[test]
fn cancellation_statics_do_not_coerce_the_independent_state() {
    let diagnostics = rock_lib::analyze(&config("main = !->\n    allocation = unsafe Carrier::reserve 0, 64\n    unsafe Carrier::abandon allocation.0, 0\n")).unwrap_err();
    assert!(
        diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Type mismatch: I64 vs Token")),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics.0.iter().all(|diagnostic| !diagnostic
            .message
            .contains("no matching static implementation")),
        "{diagnostics:?}"
    );
}

#[test]
fn allocation_statics_require_the_declared_constructor_kind() {
    let diagnostics = rock_lib::analyze(&config(
        "main = !->\n    unsafe NonGeneric::Reservation::reserve 0, 64\n",
    ))
    .unwrap_err();
    assert!(
        diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("kind")
                && diagnostic.message.contains("Reservation")
                && diagnostic.message.contains("requires")),
        "{diagnostics:?}"
    );
}

#[test]
fn allocation_implementation_state_must_have_value_kind() {
    let source = PROVIDER.replace(
        "impl Reservation Token for Carrier",
        "impl Reservation Carrier for Carrier",
    );
    let mut configuration = config("main = !-> ()\n");
    configuration.source_providers = vec![SourceProvider::Virtual {
        path: configuration.entry_file.clone(),
        text: format!("{source}\nmain = !-> ()\n"),
    }];
    let diagnostics = rock_lib::analyze(&configuration).unwrap_err();
    assert!(
        diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("kind")),
        "{diagnostics:?}"
    );
}
