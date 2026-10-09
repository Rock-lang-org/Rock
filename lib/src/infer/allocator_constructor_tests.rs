//! Analysis reductions retain the real reserve/cast/store/value construction flow.

use std::path::PathBuf;

use crate::hir::{HirExprKind, HirStmt};
use crate::infer::{ConstraintOwner, PartialHir};
use crate::types::Type;

fn source(payload_first: bool, caller_first: bool, early_return: bool) -> String {
    let args = if payload_first { "T, A" } else { "A, T" };
    let declarations = r#"
lang sized
< trait LayoutKnown
extern malloc: I64 -> *U8
extern exit: I32 -> ()
trait Memory
    unsafe @reserve: I64 -> I64 -> *U8
struct State A
    < allocator: A
    < bytes: I64
    < alignment: I64
struct Shell $ARGS
    < pointer: *T
    < state: State A
struct Payload
    < value: I64
struct Zero
"#
    .replace("$ARGS", args);
    let providers = r#"
struct Small
    < tag: I64
impl Memory for Small
    unsafe @reserve = bytes, alignment -> malloc bytes
struct Large
    < padding: (I64, I64)
    < tag: I64
impl Memory for Large
    unsafe @reserve = bytes, alignment -> malloc bytes
"#;
    let constructor = format!(
        r#"
impl Shell {args} where A: Memory
    new = allocator, value -> Shell::new_aligned allocator, value, (~AlignOf value)
    new_aligned = allocator, value, alignment ->
        if (~I64Lt alignment, (~AlignOf value)) then exit (69 as I32)
        bytes = ~SizeOf value
        base = unsafe allocator.reserve bytes, alignment
        pointer = base as *T
        unsafe *pointer = value
        state = State
            allocator: allocator
            bytes: bytes
            alignment: alignment
{early}        Shell
            pointer: pointer
            state: state
"#,
        early = if early_return {
            "        if (~I64Eq alignment, 0) then return Shell\n            pointer: pointer\n            state: state\n"
        } else {
            ""
        }
    );
    let mut callers = String::new();
    for (allocator, suffix) in [("Small", "small"), ("Large", "large")] {
        for (case, payload, method, alignment) in [
            ("normal", "Payload\n        value: 42", "new", ""),
            ("zero", "Zero", "new", ""),
            (
                "aligned",
                "Payload\n        value: 42",
                "new_aligned",
                ", 4096",
            ),
        ] {
            let allocator_fields = if allocator == "Small" {
                "        tag: 1\n"
            } else {
                "        padding: (2, 3)\n        tag: 4\n"
            };
            callers.push_str(&format!("{suffix}_{case} = ->\n    allocator = {allocator}\n{allocator_fields}    value = {payload}\n    Shell::{method} allocator, value{alignment}\n"));
        }
    }
    if caller_first {
        format!("{declarations}{providers}{callers}{constructor}\nmain = !-> ()\n")
    } else {
        format!("{declarations}{constructor}{providers}{callers}\nmain = !-> ()\n")
    }
}

fn config(source: &str) -> crate::Config {
    let path = PathBuf::from("/virtual/allocator_constructor_authority.rk");
    crate::Config {
        entry_file: path.clone(),
        no_std: true,
        no_prelude: true,
        source_providers: vec![crate::SourceProvider::Virtual {
            path,
            text: source.into(),
        }],
        ..crate::Config::default()
    }
}

fn partial(source: &str) -> PartialHir {
    let config = config(source);
    let module = crate::parser::parse_source(config.entry_file.clone(), source, &config).unwrap();
    crate::lower::Lowerer::with_options(false)
        .lower_program(&crate::ast::Program { module })
        .unwrap()
}

fn trace(hir: &PartialHir, stage: &str) {
    for implementation in hir.impls.values() {
        if let Some(method) = implementation.methods.get("new_aligned") {
            eprintln!("allocator constructor {stage}: impl={:?}, receiver={:?}, impl_params={:?}, impl_bounds={:?}, header_slots={:?}, method={method:#?}, resolved_params={:?}, resolved_result={:?}",
                implementation.id, implementation.receiver_pattern, implementation.type_generics,
                implementation.bounds, hir.function_type_vars.get(&method.id),
                method.params.iter().map(|param| hir.engine.resolve(&param.ty)).collect::<Vec<_>>(),
                hir.engine.resolve(&method.ret_type));
            for (id, constraint) in hir.constraint_store.iter() {
                if hir.constraint_store.owner(id) == Some(ConstraintOwner::Body(method.id)) {
                    eprintln!(
                        "allocator constructor constraint={id:?}, owner={:?}, {constraint:?}",
                        hir.constraint_store.owner(id)
                    );
                }
            }
        }
    }
}

#[test]
fn reserve_store_constructor_generalizes_only_actual_signature_slots_after_constraints() {
    let source = source(false, false, true);
    let mut hir = partial(&source);
    trace(&hir, "lowered");
    crate::infer::drive_inference_to_quiescence(&mut hir).unwrap();
    trace(&hir, "constraints discharged, before method generalization");
    let implementation = hir
        .impls
        .values()
        .find(|implementation| implementation.methods.contains_key("new_aligned"))
        .unwrap();
    let method = &implementation.methods["new_aligned"];
    let payload = implementation
        .type_generics
        .iter()
        .find(|param| param.name == "T")
        .unwrap()
        .id;
    assert_eq!(
        hir.engine.resolve(&method.params[1].ty),
        Type::Generic(payload)
    );
    assert!(method
        .generic_params
        .iter()
        .any(|param| param.id == payload));
    let Type::TypeVar(allocator_slot) = hir.engine.resolve(&method.params[0].ty) else {
        panic!("actual allocator scheme must remain independent of concrete callers: {method:?}")
    };
    let memory = hir
        .traits
        .values()
        .find(|definition| definition.name == "Memory")
        .unwrap()
        .id;
    assert!(hir
        .engine
        .get_bounds(allocator_slot)
        .iter()
        .any(|bound| bound.trait_id == memory));
    let Type::Struct { args, .. } = hir.engine.resolve(&method.ret_type) else {
        unreachable!()
    };
    assert_eq!(
        args,
        vec![Type::TypeVar(allocator_slot), Type::Generic(payload)]
    );
    let pointer = method
        .body
        .stmts
        .iter()
        .find_map(|stmt| match stmt {
            HirStmt::Let { name, value, .. } if name == "pointer" => Some(value),
            _ => None,
        })
        .unwrap();
    let HirExprKind::Cast(_, target) = &pointer.kind else {
        panic!("{pointer:?}")
    };
    assert_eq!(
        hir.engine.resolve(target),
        Type::Pointer(Box::new(Type::Generic(payload)))
    );
    let store = method
        .body
        .stmts
        .iter()
        .find_map(|stmt| {
            let HirStmt::Expr(expr) = stmt else {
                return None;
            };
            let HirExprKind::UnsafeBlock(body) = &expr.kind else {
                return None;
            };
            body.stmts.iter().find_map(|stmt| match stmt {
                HirStmt::Expr(expr) if matches!(expr.kind, HirExprKind::Assign(_, _)) => Some(expr),
                _ => None,
            })
        })
        .unwrap();
    let HirExprKind::Assign(place, value) = &store.kind else {
        unreachable!()
    };
    assert!(matches!(place.kind, HirExprKind::Deref(_)));
    assert_eq!(hir.engine.resolve(&place.ty), Type::Generic(payload));
    assert_eq!(hir.engine.resolve(&value.ty), Type::Generic(payload));
    let HirStmt::Expr(literal) = method.body.stmts.last().unwrap() else {
        unreachable!()
    };
    let HirExprKind::StructLiteral(_, Some(shell), fields) = &literal.kind else {
        panic!("{literal:?}")
    };
    assert_eq!(&source[literal.span.start..literal.span.end], "Shell");
    assert_eq!(
        hir.engine.resolve(&literal.ty),
        Type::Struct {
            id: *shell,
            args: args.clone()
        }
    );
    let pointer_field = fields.iter().find(|field| field.name == "pointer").unwrap();
    assert_eq!(
        hir.engine.resolve(&pointer_field.value.ty),
        hir.engine.resolve(target)
    );
    let state_field = fields.iter().find(|field| field.name == "state").unwrap();
    let Type::Struct {
        args: state_args, ..
    } = hir.engine.resolve(&state_field.value.ty)
    else {
        unreachable!()
    };
    assert_eq!(state_args, vec![Type::TypeVar(allocator_slot)]);
    let method_id = method.id;
    let impl_id = implementation.id;
    super::generalize_methods_only(&mut hir);
    trace(&hir, "method generalized");
    let method = &hir.impls[&impl_id].methods["new_aligned"];
    let Type::Generic(allocator) = method.params[0].ty else {
        panic!("{method:?}")
    };
    assert_eq!(allocator.owner, method_id);
    assert_ne!(
        allocator,
        hir.impls[&impl_id]
            .type_generics
            .iter()
            .find(|param| param.name == "A")
            .unwrap()
            .id
    );
    assert_eq!(method.params[1].ty, Type::Generic(payload));
    assert!(method.generic_bounds[&allocator]
        .iter()
        .any(|bound| bound.trait_id == memory));
    crate::infer::authority::finalize_method_instantiations(&mut hir).unwrap();
    let errors = crate::infer::finalize::apply_finalization(&mut hir);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn actual_allocation_constructor_preserves_two_layouts_zero_alignment_and_source_orders() {
    for payload_first in [false, true] {
        for caller_first in [false, true] {
            for early_return in [false, true] {
                let source = source(payload_first, caller_first, early_return);
                let analysis = crate::analyze(&config(&source)).unwrap_or_else(|errors| {
                    panic!("{payload_first}/{caller_first}/{early_return}: {errors:?}")
                });
                let program = &analysis.hir.program;
                let shell = program
                    .structs
                    .values()
                    .find(|definition| definition.name == "Shell")
                    .unwrap();
                let state = program
                    .structs
                    .values()
                    .find(|definition| definition.name == "State")
                    .unwrap();
                let memory = program
                    .traits
                    .values()
                    .find(|definition| definition.name == "Memory")
                    .unwrap();
                let implementation = program
                    .impls
                    .values()
                    .find(|implementation| implementation.methods.contains_key("new_aligned"))
                    .unwrap();
                let payload = implementation
                    .type_generics
                    .iter()
                    .find(|param| param.name == "T")
                    .unwrap()
                    .id;
                let aligned = &implementation.methods["new_aligned"];
                let Type::Generic(allocator) = aligned.params[0].ty else {
                    panic!("{aligned:?}")
                };
                assert_eq!(allocator.owner, aligned.id);
                assert_eq!(aligned.params[1].ty, Type::Generic(payload));
                assert_eq!(aligned.params[2].ty, Type::I64);
                assert!(aligned.generic_bounds[&allocator]
                    .iter()
                    .any(|bound| bound.trait_id == memory.id));
                let base = aligned
                    .body
                    .stmts
                    .iter()
                    .find_map(|stmt| match stmt {
                        crate::hir::HirStmtFor::Let { name, value, .. } if name == "base" => {
                            Some(value)
                        }
                        _ => None,
                    })
                    .unwrap();
                let crate::hir::HirExprKindFor::Block(body) = &base.kind else {
                    panic!("{base:?}")
                };
                let Some(crate::hir::HirStmtFor::Expr(reserve)) = body.stmts.last() else {
                    unreachable!()
                };
                let crate::hir::HirExprKindFor::MethodCall(receiver, _, args, _, target) =
                    &reserve.kind
                else {
                    panic!("{reserve:?}")
                };
                assert_eq!(
                    target.impl_id(),
                    None,
                    "producer cannot choose Small or Large"
                );
                assert_eq!(
                    target.target,
                    crate::hir::HirSelectedMethodTarget::TraitMethod {
                        trait_id: memory.id,
                        member_id: memory.signatures["reserve"].id,
                        trait_args: Vec::new(),
                        dispatch: crate::hir::HirTraitDispatchKind::TraitBound,
                    }
                );
                assert_eq!(receiver.ty, Type::Generic(allocator));
                assert_eq!(args.len(), 2);
                assert!(args.iter().all(|arg| arg.ty == Type::I64));
                assert_eq!(reserve.ty, Type::Pointer(Box::new(Type::U8)));
                let pointer = aligned
                    .body
                    .stmts
                    .iter()
                    .find_map(|stmt| match stmt {
                        crate::hir::HirStmtFor::Let { name, value, .. } if name == "pointer" => {
                            Some(value)
                        }
                        _ => None,
                    })
                    .unwrap();
                let crate::hir::HirExprKindFor::Cast(_, target) = &pointer.kind else {
                    panic!("{pointer:?}")
                };
                assert_eq!(target, &Type::Pointer(Box::new(Type::Generic(payload))));
                let store = aligned
                    .body
                    .stmts
                    .iter()
                    .find_map(|stmt| {
                        let crate::hir::HirStmtFor::Expr(expr) = stmt else {
                            return None;
                        };
                        let crate::hir::HirExprKindFor::Block(body) = &expr.kind else {
                            return None;
                        };
                        body.stmts.iter().find_map(|stmt| match stmt {
                            crate::hir::HirStmtFor::Expr(expr)
                                if matches!(
                                    expr.kind,
                                    crate::hir::HirExprKindFor::Assign(_, _)
                                ) =>
                            {
                                Some(expr)
                            }
                            _ => None,
                        })
                    })
                    .unwrap();
                let crate::hir::HirExprKindFor::Assign(place, value) = &store.kind else {
                    unreachable!()
                };
                assert!(matches!(place.kind, crate::hir::HirExprKindFor::Deref(_)));
                assert_eq!(place.ty, Type::Generic(payload));
                assert_eq!(value.ty, Type::Generic(payload));
                let state_value = aligned
                    .body
                    .stmts
                    .iter()
                    .find_map(|stmt| match stmt {
                        crate::hir::HirStmtFor::Let { name, value, .. } if name == "state" => {
                            Some(value)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(
                    state_value.ty,
                    Type::Struct {
                        id: state.id,
                        args: vec![Type::Generic(allocator)]
                    }
                );
                if early_return {
                    let branch = aligned
                        .body
                        .stmts
                        .iter()
                        .find_map(|stmt| {
                            let crate::hir::HirStmtFor::Expr(expr) = stmt else {
                                return None;
                            };
                            let crate::hir::HirExprKindFor::If { then_branch, .. } = &expr.kind
                            else {
                                return None;
                            };
                            matches!(
                                then_branch.stmts.last(),
                                Some(crate::hir::HirStmtFor::Return(Some(_)))
                            )
                            .then_some(then_branch)
                        })
                        .unwrap();
                    assert_eq!(branch.ty, Type::Never);
                    let Some(crate::hir::HirStmtFor::Return(Some(value))) = branch.stmts.last()
                    else {
                        unreachable!()
                    };
                    assert_eq!(value.ty, aligned.ret_type);
                }
                for method in [&implementation.methods["new"], aligned] {
                    let Type::Struct { id, args } = &method.ret_type else {
                        panic!("{method:?}")
                    };
                    assert_eq!(*id, shell.id);
                    assert_eq!(args[usize::from(payload_first)], method.params[0].ty);
                    assert_eq!(args[usize::from(!payload_first)], method.params[1].ty);
                    assert!(args.iter().all(|ty| matches!(ty, Type::Generic(_))));
                }
                for (name, allocator_name) in [("small", "Small"), ("large", "Large")] {
                    for (case, payload_name) in [
                        ("normal", "Payload"),
                        ("zero", "Zero"),
                        ("aligned", "Payload"),
                    ] {
                        let function = program
                            .functions
                            .values()
                            .find(|function| function.name == format!("{name}_{case}"))
                            .unwrap();
                        let nominal = |name: &str| Type::Struct {
                            id: program
                                .structs
                                .values()
                                .find(|definition| definition.name == name)
                                .unwrap()
                                .id,
                            args: Vec::new(),
                        };
                        let mut args = vec![nominal(allocator_name), nominal(payload_name)];
                        if payload_first {
                            args.swap(0, 1);
                        }
                        assert_eq!(function.ret_type, Type::Struct { id: shell.id, args });
                    }
                }
            }
        }
    }
}

#[test]
fn allocation_constructor_keeps_source_safety_and_real_memory_obligations_strict() {
    let original = source(false, false, false);
    for (source, message, offending) in [
        (
            original.replace("unsafe *pointer = value", "*pointer = value"),
            "requires an unsafe block",
            "*",
        ),
        (
            original.replace("pointer = base as *T", "pointer = alignment"),
            "Cannot dereference non-pointer",
            "*",
        ),
        (
            original.replace("pointer = base as *T", "pointer = &value"),
            "Cannot assign through a shared reference",
            "pointer",
        ),
    ] {
        let errors = crate::analyze(&config(&source))
            .expect_err("invalid source store cannot be generalized away");
        let error = errors
            .0
            .iter()
            .find(|error| error.message.contains(message))
            .unwrap_or_else(|| panic!("{errors:?}"));
        let span = &error.primary.as_ref().unwrap().span;
        assert_eq!(span.file_path, config(&source).entry_file);
        assert!(source[span.start..span.end].contains(offending), "{span:?}");
    }
    for payload_first in [false, true] {
        for direct in [false, true] {
            let mut source = source(payload_first, true, false)
                .replace(
                    "struct Small\n",
                    "struct Invalid\n    < tag: I64\nstruct Small\n",
                )
                .replace("    allocator = Small\n", "    allocator = Invalid\n");
            if direct {
                source = source.replace(
                    "Shell::new allocator, value\n",
                    "Shell::new_aligned allocator, value, 4096\n",
                );
            }
            let errors = crate::analyze(&config(&source))
                .expect_err("all actual invalid allocator calls must reject");
            assert!(
                errors
                    .0
                    .iter()
                    .any(|error| error.message.contains("Memory")),
                "{errors:?}"
            );
            assert!(
                errors
                    .0
                    .iter()
                    .all(|error| error.code != Some(crate::diagnostic::DiagnosticCode::Internal)),
                "{errors:?}"
            );
        }
    }
}

#[test]
fn uncalled_allocation_constructor_has_no_concrete_allocator_authority() {
    let source = source(false, false, false);
    let provider_start = source.find("\nstruct Small\n").unwrap();
    let source = format!("{}\nmain = !-> ()\n", &source[..provider_start]);
    let analysis = crate::analyze(&config(&source)).unwrap_or_else(|errors| panic!("{errors:?}"));
    let program = &analysis.hir.program;
    let memory = program
        .traits
        .values()
        .find(|definition| definition.name == "Memory")
        .unwrap();
    assert!(program
        .impls
        .values()
        .all(|implementation| implementation.trait_id != Some(memory.id)));
    let aligned = program
        .impls
        .values()
        .find_map(|implementation| implementation.methods.get("new_aligned"))
        .unwrap();
    let Type::Generic(allocator) = aligned.params[0].ty else {
        panic!("{aligned:?}")
    };
    assert_eq!(allocator.owner, aligned.id);
    assert!(aligned.generic_bounds[&allocator]
        .iter()
        .any(|bound| bound.trait_id == memory.id));
}

#[test]
fn allocation_context_does_not_guess_an_unrelated_dereference_kind() {
    let source = source(false, false, false).replace(
        "main = !-> ()",
        "unknown = input, value !-> unsafe *input = value\nmain = !-> ()",
    );
    let errors = crate::analyze(&config(&source))
        .expect_err("allocation context cannot choose another helper's handle kind");
    assert!(
        errors
            .0
            .iter()
            .all(|error| error.code != Some(crate::diagnostic::DiagnosticCode::Internal)),
        "{errors:?}"
    );
    let error = errors
        .0
        .iter()
        .find(|error| {
            error.message.contains("cannot determine") || error.message.contains("ambiguous")
        })
        .unwrap_or_else(|| panic!("{errors:?}"));
    let span = &error.primary.as_ref().unwrap().span;
    assert_eq!(span.file_path, config(&source).entry_file);
    assert!(source[span.start..span.end].contains('*'), "{span:?}");
}

#[test]
#[ignore = "coordinator supplies the unchanged extracted source and real prebuilt stdlib artifact"]
fn capture_original_allocator_constructor_authority() {
    let source_path = PathBuf::from(
        std::env::var_os("ROCK_OWNER_TRACE_SOURCE").expect("explicit original source path"),
    );
    let artifact = PathBuf::from(
        std::env::var_os("ROCK_OWNER_TRACE_STDLIB").expect("explicit real stdlib artifact"),
    );
    let source = std::fs::read_to_string(&source_path).unwrap();
    let mut config = config(&source);
    config.entry_file = source_path.clone();
    config.current_crate_name = Some("allocator_state_caller".into());
    config.extern_artifacts = vec![("stdlib".into(), artifact)];
    let mut context = crate::crate_system::CrateContext::new();
    crate::load_extern_artifacts(
        &mut context,
        &config.extern_artifacts,
        source_path.parent().unwrap().to_path_buf(),
    )
    .unwrap();
    let module = crate::parser::parse_source(source_path, &source, &config).unwrap();
    let program = crate::ast::Program { module };
    let declarations = crate::collect::collect(
        &program,
        &context,
        false,
        config.current_crate_name.as_deref(),
    )
    .unwrap();
    let mut hir = crate::lower::program::lower_from_declarations(
        &program,
        declarations,
        &context,
        config.current_crate_name.as_deref(),
    )
    .unwrap();
    trace(&hir, "original source lowered");
    crate::infer::drive_inference_to_quiescence(&mut hir).unwrap();
    trace(
        &hir,
        "original source constraints discharged, before method generalization",
    );
    super::generalize_methods_only(&mut hir);
    trace(&hir, "original source method generalized");
    crate::infer::authority::finalize_method_instantiations(&mut hir).unwrap();
    trace(&hir, "original source authority finalized");
    let errors = crate::infer::finalize::apply_finalization(&mut hir);
    assert!(errors.is_empty(), "{errors:?}");
}
