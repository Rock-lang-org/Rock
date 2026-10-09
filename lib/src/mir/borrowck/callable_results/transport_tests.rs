use super::super::{borrows, liveness};
use super::*;
use crate::mir::dataflow::analyses::LoanTable;
use crate::mir::Operand;

const NATIVE: &str = r#"
identity: &I64 -> &I64
identity = value -> value
read: &I64 -> I64
read = value -> *value
main = !->
    value = 42
    input = &value
    callback = if true then identity else identity
    result = callback input
    read result
"#;

const CAPTURING: &str = r#"
struct Holder
    < value: &I64
impl Holder
    ~@extract: &I64
    ~@extract = -> self.value
read: &I64 -> I64
read = value -> *value
main = !->
    value = 42
    holder = Holder
        value: &value
    callback = -> holder.extract!
    result = callback!
    read result
"#;

fn inspect(name: &str, source: &str, environment: bool, contents: bool) {
    let (program, _) =
        super::super::allocator_trace_tests::mutation_source_program(name, source, false);
    let proof = BorrowChecker::native_return_loan_proof(&program);
    let body = program
        .functions
        .values()
        .find(|body| body.name == "main")
        .unwrap();
    let sites = body
        .basic_blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            let Terminator::Call {
                func,
                args,
                destination,
                target,
                ..
            } = block.terminator.as_ref()?
            else {
                return None;
            };
            let (inputs, summary) = proof.sources.callable_result_call(
                body,
                &program.type_context,
                &program.backend_contract,
                func,
                args,
                destination,
            )?;
            Some((index, func, args, destination, *target, inputs, summary))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        sites.len(),
        1,
        "a genuinely indirect call must have actual producer evidence"
    );
    let (index, func, args, destination, target, inputs, summary) = &sites[0];
    assert!(
        matches!(func, Operand::Copy(_) | Operand::Move(_)),
        "executable dispatch must remain indirect"
    );
    let source_local = Local(if environment { 1 } else { 2 });
    assert_eq!(
        *summary,
        OriginSet::from([if contents {
            crate::mir::ReferenceOrigin::ParamContents(source_local)
        } else {
            crate::mir::ReferenceOrigin::Param(source_local)
        }])
    );
    assert_eq!(inputs.len(), args.len() + 1);
    let callee = match func {
        Operand::Copy(place) | Operand::Move(place) => place,
        _ => unreachable!(),
    };
    let callable_ty = program
        .backend_contract
        .place_type_id(&program.type_context, body, callee)
        .unwrap();
    assert_eq!(
        body.local_decls[destination.local.0].ty,
        match program.type_context.try_ty(callable_ty).unwrap() {
            Ty::Function {
                ret,
                result_borrowing,
                ..
            } => {
                assert_eq!(
                    *result_borrowing,
                    CallableResultBorrowing::Unknown,
                    "a header is not the certificate"
                );
                *ret
            }
            _ => panic!(),
        }
    );
    let analysis = ReferenceEscapeAnalysis::new(
        body,
        &program.type_context,
        &program.backend_contract,
        &proof.lifetime,
        MissingSummaryMode::Conservative,
    )
    .with_return_sources(Some(&proof.sources));
    let results = run_fixpoint(&analysis, body);
    let mut before = results.entry_sets[*index].clone();
    use crate::mir::dataflow::Analysis;
    for statement in &body.basic_blocks[*index].statements {
        analysis.apply_statement(&mut before, statement);
    }
    let actual = analysis.call_return_origins(&before, func, args, destination);
    let payload_place = body
        .basic_blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| {
            let StatementKind::Assign(_, Rvalue::Ref(_, place)) = &statement.kind else {
                return None;
            };
            (program
                .backend_contract
                .place_type_id(&program.type_context, body, place)
                .is_some_and(|ty| program.type_context.try_ty(ty) == Some(&Ty::I64)))
            .then_some(place)
        })
        .unwrap();
    let expected = analysis.origins_for_borrowed_place(&before, payload_place);
    assert!(!expected.is_empty());
    assert_eq!(actual, expected);
    assert!(!actual.contains(&crate::mir::ReferenceOrigin::UnknownExternal));
    assert!(!actual.contains(&crate::mir::ReferenceOrigin::Local(callee.local)));

    let collected = borrows::collect_function_borrows_with_return_sources(
        body,
        &program.type_context,
        &program.backend_contract,
        &proof.summaries,
        Some(&proof.sources),
    );
    let table = LoanTable::from_borrows_with_paths(
        &collected,
        super::super::paths::PlacePathTable::for_function(body),
    );
    let locations = liveness::LoanLocationIndex::for_function(&table, body);
    let live = liveness::compute_reference_liveness_with_return_sources(
        body,
        &program.type_context,
        &program.backend_contract,
        &table,
        &[],
        Some(&proof.sources),
    );
    let entries = liveness::compute_active_loan_entries_with_return_sources(
        body,
        &program.type_context,
        &program.backend_contract,
        &table,
        &locations,
        &live,
        &proof.summaries,
        &[],
        Some(&proof.sources),
    );
    let payloads = table
        .iter()
        .filter(|loan| {
            table
                .place_paths()
                .place(loan.place_path)
                .is_some_and(|place| {
                    body.local_decls[place.local.0].name.as_deref() == Some("value")
                        && place.projection.is_empty()
                })
        })
        .collect::<Vec<_>>();
    assert!(!payloads.is_empty());
    assert!(
        payloads
            .iter()
            .any(|payload| entries[target.0].owner_contains(payload.id, destination.local)),
        "actual payload loan must survive the callable's consumed/dead storage"
    );
    assert!(table
        .iter()
        .any(|loan| loan.initial_owner == destination.local && table.is_returned_view(loan.id)));
    assert!(
        BorrowChecker::run(&program).is_ok(),
        "{:#?}",
        BorrowChecker::run(&program)
    );
}

#[test]
fn certified_native_indirect_result_transports_the_actual_argument_loan() {
    inspect("transport_native", NATIVE, false, false);
}

#[test]
fn bound_raw_getter_does_not_borrow_its_invocation_temporary() {
    let source = r#"
struct Buffer
    < raw: *I64
impl Buffer
    @ptr: *I64
    @ptr = -> self.raw
main = !->
    value = 42
    buffer = Buffer
        raw: unsafe (&value as *I64)
    pointer = buffer.ptr!
"#;
    let (program, _) = super::super::allocator_trace_tests::mutation_source_program(
        "transport_raw_getter",
        source,
        false,
    );
    BorrowChecker::run(&program).unwrap();
}

#[test]
fn certified_capturing_indirect_result_transports_payload_not_callback_storage() {
    inspect("transport_capturing", CAPTURING, true, false);
}

#[test]
fn certified_shared_capturing_result_loads_carried_contents_without_receiver_storage() {
    let source = CAPTURING.replace("~@extract", "@extract");
    inspect("transport_shared_capturing", &source, true, true);
}

#[test]
fn certified_shared_result_does_not_keep_the_holder_storage_borrow_alive() {
    let source = CAPTURING
        .replace("~@extract", "@extract")
        .replace(
            "read: &I64 -> I64",
            "impl Holder\n    ~@consume: &I64\n    ~@consume = -> self.value\nread: &I64 -> I64",
        )
        .replace("    read result", "    holder.consume!\n    read result");
    let (program, _) = super::super::allocator_trace_tests::mutation_source_program(
        "transport_shared_holder_release",
        &source,
        false,
    );
    BorrowChecker::run(&program).unwrap();
}

#[test]
fn measured_isolated_callable_survives_an_unrelated_call_without_empty_effects() {
    let source = CAPTURING.replace(
        "    result = callback!",
        "    observed = read &value\n    result = callback!",
    );
    inspect("transport_isolated_unrelated_call", &source, true, false);
}

#[test]
fn cast_exposure_vetoes_isolation_and_never_supplies_callable_authority() {
    let source = CAPTURING.replace(
        "    result = callback!",
        "    address = unsafe ((&callback) as *U8)\n    result = callback!",
    );
    let (program, _) = super::super::allocator_trace_tests::mutation_source_program(
        "transport_cast_veto",
        &source,
        false,
    );
    let proof = BorrowChecker::native_return_loan_proof(&program);
    let body = program
        .functions
        .values()
        .find(|body| body.name == "main")
        .unwrap();
    let sites = body
        .basic_blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            Some(Terminator::Call {
                func: func @ (Operand::Copy(_) | Operand::Move(_)),
                args,
                destination,
                ..
            }) => Some((func, args, destination)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(sites.len(), 1);
    for (func, args, destination) in sites {
        assert!(proof
            .sources
            .callable_result_call(
                body,
                &program.type_context,
                &program.backend_contract,
                func,
                args,
                destination
            )
            .is_none());
    }
}

#[test]
fn composed_nested_callable_result_keeps_the_environment_payload_loan() {
    let source = CAPTURING.replace("main = !->", "relay: Holder -> &I64\nrelay = holder ->\n    inner = -> holder.extract!\n    inner!\nmain = !->")
        .replace("callback = -> holder.extract!", "callback = -> relay holder")
        .replace("value = 42", "mut value = 42")
        .replace("    read result", "    value = 43\n    read result");
    let (program, _) = super::super::allocator_trace_tests::mutation_source_program(
        "transport_opaque_nested_payload",
        &source,
        false,
    );
    let diagnostics = BorrowChecker::run(&program)
        .expect_err("composition must not lose environment-carried loans");
    assert!(
        diagnostics.0.iter().any(|diagnostic| diagnostic.message
            == "borrow conflict on this value"
            && diagnostic.primary.is_some()),
        "{diagnostics:?}"
    );
}

#[test]
fn uncertified_callable_parameter_preserves_opaque_environment_loans() {
    let source = CAPTURING
        .replace("~@extract", "@extract")
        .replace(
            "main = !->",
            "invoke: (() -> &I64) -> &I64\ninvoke = callback -> callback!\nmain = !->",
        )
        .replace("result = callback!", "result = invoke callback")
        .replace("value = 42", "mut value = 42")
        .replace("    read result", "    value = 43\n    read result");
    let (program, _) = super::super::allocator_trace_tests::mutation_source_program(
        "transport_opaque_parameter",
        &source,
        false,
    );
    let proof = BorrowChecker::native_return_loan_proof(&program);
    let invoke = program
        .functions
        .values()
        .find(|body| {
            body.arg_count == 1
                && body.local_decls.get(1).is_some_and(|decl| {
                    matches!(
                        program.type_context.try_ty(decl.ty),
                        Some(Ty::Function { .. })
                    )
                })
        })
        .unwrap();
    let indirect = invoke
        .basic_blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            Some(Terminator::Call {
                func: func @ (Operand::Copy(_) | Operand::Move(_)),
                args,
                destination,
                ..
            }) => Some((func, args, destination)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(indirect.len(), 1);
    for (func, args, destination) in indirect {
        assert!(proof
            .sources
            .callable_result_call(
                invoke,
                &program.type_context,
                &program.backend_contract,
                func,
                args,
                destination
            )
            .is_none());
    }
    let diagnostics = BorrowChecker::run(&program).unwrap_err();
    assert!(
        diagnostics.0.iter().any(|diagnostic| diagnostic.message
            == "borrow conflict on this value"
            && diagnostic.primary.is_some()),
        "{diagnostics:?}"
    );
}

#[test]
fn concrete_returned_callable_has_measured_invocation_authority_without_a_promised_header() {
    let source = CAPTURING.replace("main = !->", "make: &I64 -> (() -> &I64)\nmake = value ->\n    holder = Holder\n        value: value\n    -> holder.extract!\nmain = !->")
        .replace("callback = -> holder.extract!", "callback = make &value");
    inspect("transport_returned", &source, true, false);
}

#[test]
fn callable_receiver_transport_preserves_owned_fold_early_returns() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/integration.rs");
    let source = super::super::allocator_trace_tests::integration_literal(
        &tests,
        "fn test_fold_loop_owned_elements_drop_on_break_and_return()",
    );
    let (program, _) = super::super::allocator_trace_tests::mutation_source_program(
        "transport_owned_fold_early_return",
        &source,
        true,
    );
    BorrowChecker::run(&program).expect("owned fold early return must preserve callback payload origins without temporary receiver-slot origins");
}

#[test]
fn certified_result_keeps_alias_and_escape_negatives_without_executing_them() {
    for (name, source) in [
        (
            "transport_native_alias",
            NATIVE
                .replace("value = 42", "mut value = 42")
                .replace("    read result", "    value = 43\n    read result"),
        ),
        (
            "transport_capturing_alias",
            CAPTURING
                .replace("value = 42", "mut value = 42")
                .replace("    read result", "    value = 43\n    read result"),
        ),
        (
            "transport_shared_capturing_alias",
            CAPTURING
                .replace("~@extract", "@extract")
                .replace("value = 42", "mut value = 42")
                .replace("    read result", "    value = 43\n    read result"),
        ),
    ] {
        let (program, _) =
            super::super::allocator_trace_tests::mutation_source_program(name, &source, false);
        let diagnostics = BorrowChecker::run(&program).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.message == "borrow conflict on this value"),
            "{diagnostics:?}"
        );
        assert!(diagnostics
            .0
            .iter()
            .filter(|diagnostic| diagnostic.message == "borrow conflict on this value")
            .all(|diagnostic| diagnostic.primary.is_some()));
    }
    let source = CAPTURING
        .replace("main = !->", "make: () -> &I64\nmake = ->")
        .replace(
            "    read result",
            "    result\nmain = !->\n    read (make!)",
        );
    let (program, _) = super::super::allocator_trace_tests::mutation_source_program(
        "transport_escape",
        &source,
        false,
    );
    assert!(BorrowChecker::run(&program)
        .unwrap_err()
        .0
        .iter()
        .any(|diagnostic| diagnostic.message
            == "Cannot return reference to local value because it does not live long enough"));
    let source = r#"
struct Holder
    < value: I64
impl Holder
    ~@consume: I64
    ~@consume = -> self.value
main = !->
    holder = Holder
        value: 42
    callback = ->
        result = &holder.value
        holder.consume!
        result
    result = callback!
"#;
    let (program, _) = super::super::allocator_trace_tests::mutation_source_program(
        "transport_storage_veto",
        source,
        false,
    );
    let proof = BorrowChecker::native_return_loan_proof(&program);
    for body in program.functions.values() {
        for block in &body.basic_blocks {
            if let Some(Terminator::Call {
                func,
                args,
                destination,
                ..
            }) = &block.terminator
            {
                assert!(proof
                    .sources
                    .callable_result_call(
                        body,
                        &program.type_context,
                        &program.backend_contract,
                        func,
                        args,
                        destination
                    )
                    .is_none());
            }
        }
    }
    assert!(BorrowChecker::run(&program)
        .unwrap_err()
        .0
        .iter()
        .any(|diagnostic| diagnostic.message
            == "Cannot return reference to local value because it does not live long enough"));
}
