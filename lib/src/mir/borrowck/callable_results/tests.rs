use super::*;
use crate::mir::StatementData;
use crate::types::Type;

const VALUE: &str = r#"
struct Holder
    < value: &I64
impl Holder
    ~@extract: &I64
    ~@extract = -> self.value
make: &I64 -> (() -> &I64)
make = value ->
    holder = Holder
        value: value
    -> holder.extract!
main = !->
    value = 42
    callback = make &value
    result = callback!
"#;

const STORAGE: &str = r#"
struct Holder
    < value: I64
impl Holder
    ~@consume: I64
    ~@consume = -> self.value
make: I64 -> (() -> &I64)
make = value ->
    holder = Holder
        value: value
    ->
        result = &holder.value
        holder.consume!
        result
main = !->
    callback = make 42
    result = callback!
"#;

fn allocation(
    program: &MirProgram,
) -> (
    &MirFunction,
    &Place,
    &crate::mir::MirClosure,
    &StatementData,
) {
    let rows = program
        .functions
        .values()
        .flat_map(|owner| {
            owner
                .basic_blocks
                .iter()
                .flat_map(|block| &block.statements)
                .filter_map(move |statement| {
                    let StatementKind::Assign(destination, Rvalue::Closure(closure)) =
                        &statement.kind
                    else {
                        return None;
                    };
                    (!closure.captures.is_empty()).then_some((
                        owner,
                        destination,
                        closure,
                        statement,
                    ))
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1);
    rows[0]
}

#[test]
fn checked_callable_result_sources_distinguish_carried_values_and_environment_storage() {
    for (name, source, dependency, accepted) in [
        (
            "contract_value",
            VALUE,
            CallableResultSource::EnvironmentValue,
            true,
        ),
        (
            "contract_storage",
            STORAGE,
            CallableResultSource::EnvironmentStorage,
            false,
        ),
    ] {
        let (program, _) =
            super::super::allocator_trace_tests::mutation_source_program(name, source, false);
        let (owner, destination, closure, _) = allocation(&program);
        let body = allocation_body(&program, owner, destination, closure).unwrap();
        let lifetime = BorrowChecker::reference_return_summaries(&program);
        let values =
            BorrowChecker::return_summaries_for_purpose(&program, OriginPurpose::ValueLoans);
        assert_eq!(
            body_sources(&program, body, &lifetime, &values),
            Some(CallableResultBorrowing::Sources(Box::new(vec![dependency])))
        );
        assert_eq!(
            closure.captures[0].kind,
            crate::mir::MirClosureCaptureKind::ByValue
        );
        assert_eq!(BorrowChecker::run(&program).is_ok(), accepted);
        if !accepted {
            let diagnostics = BorrowChecker::run(&program).unwrap_err();
            assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message
                == "Cannot return reference to local value because it does not live long enough"));
            assert!(diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.message == "borrow conflict on this value"));
        }
    }
}

#[test]
fn checked_callable_result_declaration_must_cover_actual_body() {
    let (mut program, _) = super::super::allocator_trace_tests::mutation_source_program(
        "contract_declared",
        VALUE,
        false,
    );
    let (owner, destination, _, statement) = allocation(&program);
    let owner_id = owner.id.clone();
    let destination = destination.clone();
    let span = statement.span.clone();
    let original = owner.local_decls[destination.local.0].ty;
    let lifetime = BorrowChecker::reference_return_summaries(&program);
    let values = BorrowChecker::return_summaries_for_purpose(&program, OriginPurpose::ValueLoans);
    for (envelope, accepted) in [
        (vec![], false),
        (vec![CallableResultSource::EnvironmentStorage], false),
        (vec![CallableResultSource::EnvironmentValue], true),
    ] {
        let mut ty = program.type_context.type_for(original);
        let Type::Function {
            result_borrowing, ..
        } = &mut ty
        else {
            panic!("actual allocation type");
        };
        *result_borrowing = CallableResultBorrowing::Sources(Box::new(envelope));
        let ty = program.type_context.intern_type(&ty);
        program.functions.get_mut(&owner_id).unwrap().local_decls[destination.local.0].ty = ty;
        let mut diagnostics = Diagnostics::default();
        validate_declarations(&program, &lifetime, &values, None, &mut diagnostics);
        assert_eq!(diagnostics.0.is_empty(), accepted);
        if !accepted {
            assert_eq!(diagnostics.0.len(), 1);
            assert_eq!(
                diagnostics.0[0].message,
                "Callable result borrowing contract is not satisfied"
            );
            assert_eq!(
                diagnostics.0[0].primary.as_ref().unwrap().span,
                span.clone().unwrap()
            );
        }
    }
}

#[test]
fn checked_callable_result_missing_body_summary_or_capture_binding_stays_unknown() {
    let (mut program, _) = super::super::allocator_trace_tests::mutation_source_program(
        "contract_missing",
        VALUE,
        false,
    );
    let (owner, destination, closure, _) = allocation(&program);
    let body = allocation_body(&program, owner, destination, closure).unwrap();
    let id = body.id.clone();
    let lifetime = BorrowChecker::reference_return_summaries(&program);
    let values = BorrowChecker::return_summaries_for_purpose(&program, OriginPurpose::ValueLoans);
    assert!(body_sources(&program, body, &lifetime, &values).is_some());
    assert_eq!(body_sources(&program, body, &HashMap::new(), &values), None);
    assert_eq!(
        body_sources(&program, body, &lifetime, &HashMap::new()),
        None
    );
    program
        .functions
        .get_mut(&id)
        .unwrap()
        .closure_captures
        .clear();
    assert_eq!(
        body_sources(&program, &program.functions[&id], &lifetime, &values),
        None
    );
    program.functions.get_mut(&id).unwrap().basic_blocks.clear();
    assert_eq!(
        body_sources(&program, &program.functions[&id], &lifetime, &values),
        None
    );
}

#[test]
fn checked_callable_result_includes_every_reachable_return_alternative() {
    let source = r#"
struct Holder
    < value: &I64
    < storage: I64
impl Holder
    ~@extract: &I64
    ~@extract = -> self.value
make: &I64 -> Bool -> (() -> &I64)
make = value, flag ->
    holder = Holder
        value: value
        storage: 42
    ->
        if flag
            holder.extract!
        else
            result = &holder.storage
            holder.extract!
            result
main = !->
    value = 42
    callback = make &value, true
    result = callback!
"#;
    let (program, _) = super::super::allocator_trace_tests::mutation_source_program(
        "contract_alternatives",
        source,
        false,
    );
    let (owner, destination, closure, _) = allocation(&program);
    let body = allocation_body(&program, owner, destination, closure).unwrap();
    let lifetime = BorrowChecker::reference_return_summaries(&program);
    let values = BorrowChecker::return_summaries_for_purpose(&program, OriginPurpose::ValueLoans);
    assert_eq!(
        body_sources(&program, body, &lifetime, &values),
        Some(CallableResultBorrowing::Sources(Box::new(vec![
            CallableResultSource::EnvironmentValue,
            CallableResultSource::EnvironmentStorage,
        ])))
    );
    assert!(
        BorrowChecker::run(&program).is_err(),
        "unsafe alternative stays rejected, even with constant true at the caller"
    );
}
