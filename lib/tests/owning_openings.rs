use std::path::PathBuf;

use rock_lib::diagnostic::DiagnosticCode;
use rock_lib::hir::{HirExprKindFor, HirSelectedMethodTarget, HirStmtFor};
use rock_lib::types::Type;
use rock_lib::{Config, SourceProvider};

// Analysis-only protocol bodies: these fixtures assert selected authority,
// not allocation, destruction, or descriptor-backed runtime acceptance.
fn source(position: usize, consuming: bool) -> String {
    let arguments = |payload: &str| {
        let mut args = vec!["Allocator", "I64"];
        args.insert(position, payload);
        args.join(", ")
    };
    let mut parameters = vec!["A", "B"];
    parameters.insert(position, "(T: ?LayoutKnown)");
    let mut section = vec!["A", "I64"];
    section.insert(position, "_");
    format!(r#"
lang sized
< trait LayoutKnown
lang object_owner
< trait Transport State for F _
    lang owner_into_parts
    unsafe split: F T -> (*T, State) where T: ?LayoutKnown
    lang owner_from_parts
    unsafe rebuild: *T -> State -> F T where T: ?LayoutKnown
lang object_owner_unique
< trait Exclusive State for F _
    lang owner_release
    unsafe release: *U8 -> State -> ()
trait Read
    @read: I64
    ~@take: I64
struct Allocator
    < tag: I64
struct State A
    < allocator: A
struct Shell {parameters}
    < pointer: *T
    < state: State A
    < marker: B
impl Transport (State A) for Shell {section}
    unsafe split = owner -> (owner.pointer, owner.state)
    unsafe rebuild = pointer, state -> Shell
        pointer: pointer
        state: state
        marker: 0
impl Exclusive (State A) for Shell {section}
    unsafe release = _, _ !-> ()
inspect: Shell {owner} -> ()
inspect = object !->
    open object as Hidden, value
        {body}
main = !-> ()
"#,
        parameters = parameters.join(", "),
        section = section.join(", "),
        owner = arguments("Read"),
        body = if consuming { "value.take!" } else { "()" },
    )
}

fn config(source: &str) -> Config {
    let path = PathBuf::from("/virtual/owning_openings/main.rk");
    Config {
        entry_file: path.clone(),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual {
            path,
            text: source.into(),
        }],
        ..Config::default()
    }
}

#[test]
fn owning_opening_preserves_nonterminal_payload_constructor_and_state() {
    for position in 0..3 {
        let analysis = rock_lib::analyze(&config(&source(position, false)))
            .unwrap_or_else(|errors| panic!("position={position}: {errors:?}"));
        let program = &analysis.hir.program;
        let function = program.functions.values().find(|f| f.name == "inspect").unwrap();
        let HirStmtFor::Expr(expression) = &function.body.stmts[0] else {
            panic!("expected opening")
        };
        let HirExprKindFor::Open { source, binding, .. } = &expression.kind else {
            panic!("expected opening")
        };
        let Type::Struct { id, args } = &source.ty else { panic!("expected owner") };
        let mut opened_args = args.clone();
        opened_args[position] = Type::Witness(binding.witness);
        assert_eq!(binding.value.ty, Type::Struct { id: *id, args: opened_args });
        let owner = binding.owner.as_ref().unwrap();
        assert_eq!(owner.into_parts.owner_ty, owner.from_parts.owner_ty);
        let protocol = program.language_items.object_owner.as_ref().unwrap();
        for (operation, member) in [
            (&owner.into_parts, protocol.into_parts_id),
            (&owner.from_parts, protocol.from_parts_id),
        ] {
            let HirSelectedMethodTarget::ImplMethod { selected_trait: Some(selected), .. } =
                &operation.method.target else { panic!("expected selected operation") };
            assert_eq!(selected.trait_id, protocol.trait_id);
            assert_eq!(selected.member_id, member);
            assert_eq!(selected.trait_args, vec![owner.state_ty.clone()]);
        }
    }
}

#[test]
fn opened_consuming_call_keeps_the_same_constructor_and_state() {
    for position in 0..3 {
        let analysis = rock_lib::analyze(&config(&source(position, true)))
            .unwrap_or_else(|errors| panic!("position={position}: {errors:?}"));
        let program = &analysis.hir.program;
        let function = program.functions.values().find(|f| f.name == "inspect").unwrap();
        let HirStmtFor::Expr(expression) = &function.body.stmts[0] else { unreachable!() };
        let HirExprKindFor::Open { binding, body, .. } = &expression.kind else { unreachable!() };
        let HirStmtFor::Expr(expression) = &body.stmts[0] else { unreachable!() };
        let HirExprKindFor::OwnedObjectCall { owner, call, .. } = &expression.kind else {
            panic!("expected opened consuming call: {expression:?}")
        };
        let transport = binding.owner.as_ref().unwrap();
        assert_eq!(owner.ty, binding.value.ty);
        assert_eq!(call.state, transport.state_ty);
        assert_eq!(call.into_parts.owner_ty, transport.into_parts.owner_ty);
        assert_eq!(call.release.owner_ty, transport.into_parts.owner_ty);
    }
}

#[test]
fn owning_opening_retains_declared_generic_allocator_evidence() {
    for position in 0..3 {
        for consuming in [false, true] {
            let mut source = source(position, consuming);
            let start = source.find("inspect:").unwrap();
            let end = start + source[start..].find('\n').unwrap();
            let signature = source[start..end].replace("Allocator", "A");
            source.replace_range(start..end, &format!("{signature} where A: LayoutKnown"));
            let analysis = rock_lib::analyze(&config(&source))
                .unwrap_or_else(|errors| panic!("position={position}, consuming={consuming}: {errors:?}"));
            let function = analysis.hir.program.functions.values()
                .find(|f| f.name == "inspect").unwrap();
            let HirStmtFor::Expr(expression) = &function.body.stmts[0] else { unreachable!() };
            let HirExprKindFor::Open { binding, .. } = &expression.kind else { unreachable!() };
            let state = &binding.owner.as_ref().unwrap().state_ty;
            assert!(matches!(state, Type::Struct { args, .. }
                if args.as_slice() == [Type::Generic(function.generic_params[0].id)]));
        }
    }
}

#[test]
fn owning_opening_rejects_missing_transport_at_the_source_handle() {
    for position in 0..3 {
        let mut source = source(position, false);
        let start = source.find("impl Transport").unwrap();
        let end = source.find("impl Exclusive").unwrap();
        source.replace_range(start..end, "");
        let errors = rock_lib::analyze(&config(&source)).unwrap_err();
        assert!(errors.0.iter().any(|error| {
            error.message.contains("owner transport State is not uniquely determined")
                && error.primary.as_ref().is_some_and(|label| {
                    source.get(label.span.start..label.span.end) == Some("object")
                })
        }), "{errors:?}");
        assert!(errors.0.iter().all(|error| error.code != Some(DiagnosticCode::Internal)));
    }
}

#[test]
fn opened_consumption_requires_unique_release_for_the_exact_state() {
    for position in 0..3 {
        for missing in [false, true] {
            let mut source = source(position, true);
            if missing {
                let start = source.find("impl Exclusive").unwrap();
                let end = source.find("inspect:").unwrap();
                source.replace_range(start..end, "");
            } else {
                source = source.replace("impl Exclusive (State A)", "impl Exclusive I64");
            }
            let errors = rock_lib::analyze(&config(&source)).unwrap_err();
            assert!(errors.0.iter().any(|error| {
                error.message.contains("unique release for the same owner constructor and State")
                    && error.primary.as_ref().is_some_and(|label| {
                        source.get(label.span.start..label.span.end) == Some("value")
                    })
            }), "position={position}, missing={missing}: {errors:?}");
            assert!(errors.0.iter().all(|error| error.code != Some(DiagnosticCode::Internal)));
        }
    }
}
