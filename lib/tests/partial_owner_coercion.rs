use std::path::PathBuf;

use rock_lib::hir::{
    AcceptedHir, HirBlockFor, HirCallTarget, HirExprFor, HirExprKindFor, HirSelectedMethodTarget,
    HirStaticMethodTarget, HirStmtFor,
};
use rock_lib::types::Type;
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
"#;

const CARRIER: &str = r#"
trait Value
    @value: I64
struct Number
    < inner: I64
impl Value for Number
    @value = -> self.inner
struct Allocator
    < tag: I64
struct OtherAllocator
    < tag: I64
struct State A
    < allocator: A
struct Shell A, (T: ?LayoutKnown)
    < pointer: *T
    < state: State A
impl Transport (State A) for Shell A, _
    unsafe split = owner -> (owner.pointer, owner.state)
    unsafe rebuild = pointer, state -> Shell
        pointer: pointer
        state: state
erase: Shell Allocator, Number -> Shell Allocator, Value
erase = owner -> owner
main = !-> ()
"#;

fn config(carrier: &str) -> Config {
    let path = PathBuf::from("/virtual/partial_owner_coercion/main.rk");
    Config {
        entry_file: path.clone(),
        current_crate_name: Some("carrier".into()),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual {
            path,
            text: format!("{PROTOCOL}{carrier}"),
        }],
        ..Config::default()
    }
}

fn operations(block: &HirBlockFor<AcceptedHir>) -> Vec<&HirStaticMethodTarget> {
    fn expression<'a>(
        value: &'a HirExprFor<AcceptedHir>,
        out: &mut Vec<&'a HirStaticMethodTarget>,
    ) {
        match &value.kind {
            HirExprKindFor::Block(body) | HirExprKindFor::UnsafeBlock(body) => visit(body, out),
            HirExprKindFor::Call(_, _, Some(HirCallTarget::StaticMethod(target))) => {
                out.push(target)
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
fn partial_owner_coercion_retains_canonical_allocator_and_state() {
    let analysis = rock_lib::analyze(&config(CARRIER)).unwrap();
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "erase")
        .unwrap();
    let targets = operations(&function.body);
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].owner_ty, targets[1].owner_ty);
    let Type::Lambda { params, body } = &targets[0].owner_ty else {
        panic!("expected partial constructor authority")
    };
    assert_eq!(params.len(), 1);
    let Type::Struct { args, .. } = body.as_ref() else {
        panic!("expected nominal section")
    };
    assert_eq!(
        args[0],
        match &function.params[0].ty {
            Type::Struct { args, .. } => args[0].clone(),
            _ => panic!("expected nominal source"),
        }
    );
    assert!(matches!(
        args[1],
        Type::BoundVar {
            depth: 0,
            index: 0,
            ..
        }
    ));
    let protocol = analysis
        .hir
        .program
        .language_items
        .object_owner
        .as_ref()
        .unwrap();
    let state_id = analysis
        .hir
        .program
        .structs
        .values()
        .find(|definition| definition.name == "State")
        .unwrap()
        .id;
    let expected_state = Type::Struct {
        id: state_id,
        args: vec![args[0].clone()],
    };
    for (target, member_id) in targets
        .into_iter()
        .zip([protocol.into_parts_id, protocol.from_parts_id])
    {
        assert!(!target.method.owner_substitution.is_empty());
        assert!(!target.method.method_substitution.is_empty());
        let HirSelectedMethodTarget::ImplMethod {
            selected_trait: Some(selected),
            ..
        } = &target.method.target
        else {
            panic!("expected canonical transport implementation")
        };
        assert_eq!(selected.trait_id, protocol.trait_id);
        assert_eq!(selected.member_id, member_id);
        assert_eq!(selected.trait_args, vec![expected_state.clone()]);
    }
}

#[test]
fn partial_owner_coercion_uses_the_declared_first_payload_position() {
    let source = CARRIER
        .replace(
            "struct Shell A, (T: ?LayoutKnown)",
            "struct Shell (T: ?LayoutKnown), A",
        )
        .replace("for Shell A, _", "for Shell _, A")
        .replace(
            "Shell Allocator, Number -> Shell Allocator, Value",
            "Shell Number, Allocator -> Shell Value, Allocator",
        );
    let analysis = rock_lib::analyze(&config(&source)).unwrap();
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "erase")
        .unwrap();
    let targets = operations(&function.body);
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].owner_ty, targets[1].owner_ty);
    let Type::Lambda { body, .. } = &targets[0].owner_ty else {
        panic!("expected section")
    };
    let Type::Struct { args, .. } = body.as_ref() else {
        panic!("expected nominal section")
    };
    assert!(matches!(
        args[0],
        Type::BoundVar {
            depth: 0,
            index: 0,
            ..
        }
    ));
    assert_eq!(
        args[1],
        match &function.params[0].ty {
            Type::Struct { args, .. } => args[1].clone(),
            _ => panic!("expected nominal source"),
        }
    );
}

#[test]
fn partial_owner_coercion_accepts_contextual_allocator_state_with_actual_bounds() {
    let source = CARRIER.replace(
        "Shell Allocator, Number -> Shell Allocator, Value",
        "Shell A, Number -> Shell A, Value",
    );
    let analysis = rock_lib::analyze(&config(&source)).unwrap();
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "erase")
        .unwrap();
    assert!(!function.generic_params.is_empty());
    let Type::Struct { args, .. } = &function.params[0].ty else {
        panic!("expected nominal source")
    };
    let Type::Generic(allocator) = &args[0] else {
        panic!("allocator must remain contextual, not forced to a closed type")
    };
    let declaration = function
        .generic_params
        .iter()
        .find(|parameter| parameter.id == *allocator)
        .unwrap();
    assert!(!declaration.maybe_unsized);
    let targets = operations(&function.body);
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].owner_ty, targets[1].owner_ty);
    for target in targets {
        let HirSelectedMethodTarget::ImplMethod {
            selected_trait: Some(selected),
            ..
        } = &target.method.target
        else {
            panic!("expected canonical transport implementation")
        };
        let [Type::Struct { args, .. }] = selected.trait_args.as_slice() else {
            panic!("expected independent allocator State")
        };
        assert_eq!(args.as_slice(), [Type::Generic(*allocator)]);
    }
}

#[test]
fn partial_owner_coercion_rejects_a_different_fixed_allocator() {
    let source = CARRIER.replace(
        "-> Shell Allocator, Value",
        "-> Shell OtherAllocator, Value",
    );
    let diagnostics = rock_lib::analyze(&config(&source)).unwrap_err();
    assert!(
        diagnostics.0.iter().any(|diagnostic| diagnostic
            .message
            .contains("exact owner constructor and State")),
        "{diagnostics:?}"
    );
}

#[test]
fn partial_owner_coercion_rejects_erasure_at_a_nonpayload_position() {
    let source = CARRIER
        .replace(
            "struct State A\n    < allocator: A",
            "struct State (A: ?LayoutKnown)\n    < marker: *A",
        )
        .replace(
            "struct Shell A, (T: ?LayoutKnown)",
            "struct Shell (A: ?LayoutKnown), (T: ?LayoutKnown)",
        )
        .replace(
            "impl Transport (State A) for Shell A, _",
            "impl Transport (State A) for Shell A, _ where A: ?LayoutKnown",
        )
        .replace(
            "Shell Allocator, Number -> Shell Allocator, Value",
            "Shell Number, Number -> Shell Value, Number",
        );
    let diagnostics = rock_lib::analyze(&config(&source)).unwrap_err();
    assert!(
        diagnostics.0.iter().any(|diagnostic| diagnostic
            .message
            .contains("exact owner constructor and State")),
        "{diagnostics:?}"
    );
}

#[test]
fn partial_owner_coercion_does_not_assume_unproven_allocator_bounds() {
    let constrained = CARRIER.replace(
        "impl Transport (State A) for Shell A, _",
        "trait AllocatorAccess\nimpl Transport (State A) for Shell A, _ where A: AllocatorAccess",
    );
    for source in [
        constrained.clone(),
        constrained.replace(
            "Shell Allocator, Number -> Shell Allocator, Value",
            "Shell A, Number -> Shell A, Value",
        ),
    ] {
        let diagnostics = rock_lib::analyze(&config(&source)).unwrap_err();
        assert!(
            diagnostics.0.iter().any(|diagnostic| diagnostic
                .message
                .contains("one proven constructor implementation")),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn partial_owner_coercion_proves_explicit_contextual_allocator_bounds() {
    let source = CARRIER
        .replace(
            "impl Transport (State A) for Shell A, _",
            "trait AllocatorAccess\nimpl Transport (State A) for Shell A, _ where A: AllocatorAccess",
        )
        .replace(
            "Shell Allocator, Number -> Shell Allocator, Value",
            "Shell A, Number -> Shell A, Value where A: AllocatorAccess",
        );
    let analysis = rock_lib::analyze(&config(&source)).unwrap();
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "erase")
        .unwrap();
    assert_eq!(operations(&function.body).len(), 2);
}

#[test]
fn partial_owner_coercion_does_not_restore_a_relaxed_allocator_sized_bound() {
    let source = CARRIER
        .replace(
            "struct State A\n    < allocator: A",
            "struct State (A: ?LayoutKnown)\n    < marker: *A",
        )
        .replace(
            "struct Shell A, (T: ?LayoutKnown)",
            "struct Shell (A: ?LayoutKnown), (T: ?LayoutKnown)",
        )
        .replace(
            "Shell Allocator, Number -> Shell Allocator, Value",
            "Shell A, Number -> Shell A, Value where A: ?LayoutKnown",
        );
    let diagnostics = rock_lib::analyze(&config(&source)).unwrap_err();
    assert!(
        diagnostics.0.iter().any(|diagnostic| diagnostic
            .message
            .contains("one proven constructor implementation")),
        "{diagnostics:?}"
    );
}
