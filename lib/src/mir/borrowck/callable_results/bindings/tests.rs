use super::*;
use crate::lexer::Span;
use crate::mir::borrowck::{BorrowChecker, OriginPurpose};
use crate::mir::{BasicBlock, BasicBlockId, LocalDecl, LocalSource, MirCallableKey, Mutability};
use crate::type_context::Ty;
use crate::types::Type;

#[test]
fn retained_arrow_forwarding_discharges_raw_result_demands_without_protocol_widening() {
    const SOURCE: &str = r#"
bind: String -> Result I64, String
bind = value -> Result::Ok (value.len!)
main = !->
    value: Result String, String = Result::Ok (String::from "hello")
    mapped = value >>= bind
    mapped.unwrap_or 0 .println!
"#;
    let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "retained_arrow_raw_result_forwarding",
        SOURCE,
        true,
    );
    let verdict = BorrowChecker::run(&program);
    assert!(verdict.is_ok(), "{verdict:?}");
}

#[test]
fn stored_generic_callable_bound_uses_the_physical_tuple_pack() {
    const SOURCE: &str = r#"
lang fn_once
< trait Consume Args, Ret
    lang output
    type Output
    lang method
    ~@consume: Args -> Ret
struct Envelope F
    < callback: F
first: (&I64, &I64) -> &I64
first = pair -> pair.0
apply: Envelope F -> &I64 -> &I64 -> &I64 where F: Consume (&I64, &I64), &I64
apply = envelope, left, right -> envelope.callback (left, right)
main = !->
    left = 42
    right = 7
    envelope = Envelope
        callback: first
    result = apply envelope, &left, &right
"#;
    let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "stored_generic_physical_tuple_pack",
        SOURCE,
        false,
    );
    let outcome = BorrowChecker::run(&program);
    assert!(
        outcome.is_ok(),
        "canonical Args packs must adapt at stored field boundaries: {outcome:?}"
    );
}

#[test]
fn retained_closure_record_inputs_are_checked_at_the_actual_binding() {
    const SOURCE: &str = r#"
lang fn_once
< trait Consume Args, Ret
    lang output
    type Output
    lang method
    ~@consume: Args -> Ret
struct Envelope F
    < callback: F
identity: &I64 -> &I64
identity = value -> value
relay: Envelope F -> &I64 -> &I64 where F: Consume &I64, &I64
relay = envelope, value ->
    adapter: Envelope F -> &I64 = current -> current.callback value
    adapter envelope
main = !->
    value = 42
    envelope = Envelope
        callback: identity
    result = relay envelope, &value
"#;
    let (mut program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "retained_closure_record_inputs",
        SOURCE,
        false,
    );
    let outcome = BorrowChecker::run(&program);
    assert!(
        outcome.is_ok(),
        "canonical lambda inputs must survive capture merging: {outcome:?}"
    );
    assert!(program
        .backend_contract
        .generic_callable_input_obligations
        .keys()
        .any(|key| matches!(key, MirCallableKey::Closure(_))));
    let mut missing = program.clone();
    let identity = missing
        .functions
        .values()
        .find(|body| {
            body.arg_count == 1
                && body.local_decls[1]
                    .span
                    .as_ref()
                    .is_some_and(|span| SOURCE.get(span.start..span.end) == Some("value"))
        })
        .unwrap()
        .id
        .clone();
    let key = missing
        .backend_contract
        .function_bodies
        .remove(&identity)
        .unwrap();
    missing.functions.remove(&identity);
    missing.backend_contract.native_return_origins.remove(&key);
    missing
        .backend_contract
        .callables
        .get_mut(&key)
        .unwrap()
        .kind = MirCallableKind::ObjectProvided;
    assert!(crate::mir::agreement::check_mir_runtime_agreement(&missing).is_clean());
    assert!(
        BorrowChecker::run(&missing).is_err(),
        "lambda requirements cannot replace an incoming producer record"
    );
    program
        .backend_contract
        .generic_callable_input_obligations
        .retain(|key, _| !matches!(key, MirCallableKey::Closure(_)));
    assert!(crate::mir::agreement::check_mir_runtime_agreement(&program).is_clean());
    assert!(
        BorrowChecker::run(&program).is_err(),
        "a specialized field layout alone cannot supply a lambda input contract"
    );
}

#[test]
fn portable_factory_result_through_a_definition_owned_record_field() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(format!(
            ".agent-work/fixtures/portable_record_{}",
            std::process::id()
        ));
    std::fs::create_dir_all(&root).unwrap();
    let provider = root.join("provider.rk");
    std::fs::write(
        &provider,
        r#"
< struct Envelope F
    < callback: F
    < tag: I64
retag: Envelope F -> Envelope F
< retag = mut envelope ->
    envelope.tag = 7
    envelope
struct Holder
    < value: &I64
impl Holder
    ~@extract: &I64
    ~@extract = -> self.value
make: &I64 -> (() -> &I64)
< make = value ->
    holder = Holder
        value: value
    -> holder.extract!
"#,
    )
    .unwrap();
    let mut config = crate::Config {
        entry_file: provider.clone(),
        output_dir: root.clone(),
        no_std: true,
        no_prelude: true,
        no_link: true,
        current_crate_name: Some("provider".into()),
        emit_object: Some(root.join("provider.o")),
        ..Default::default()
    };
    let mut products = crate::compile_with_products(&config)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    std::fs::remove_file(&provider).unwrap();
    const SOURCE: &str = r#"
> provider::Envelope
> provider::make
> provider::retag
lang fn_once
< trait Consume Args, Ret
    lang output
    type Output
    lang method
    ~@consume: Args -> Ret
apply: Envelope F -> &I64 where F: Consume (), &I64
apply = envelope -> envelope.callback!
main = !->
    value = 42
    callback = make &value
    envelope = Envelope
        callback: callback
        tag: 0
    changed = retag envelope
    result = apply changed
"#;
    let entry = root.join("main.rk");
    std::fs::write(&entry, SOURCE).unwrap();
    config.entry_file = entry;
    config.current_crate_name = None;
    config.emit_object = None;
    config.extern_artifacts = vec![("provider".into(), artifact)];
    let frontend = crate::compile_frontend(&config).unwrap();
    let fingerprint = crate::product_source_fingerprint(
        &config,
        &frontend.source_db,
        &frontend.fingerprint_source_files,
    );
    let identity =
        crate::products::ProductCrateIdentity::local(frontend.effective_current_crate_name.clone())
            .with_source_fingerprint(fingerprint);
    let context = crate::mono::CompilationIdentityContext::new(
        frontend.analysis.hir.root_crate_id,
        identity,
        frontend
            .crate_ctx
            .product_crate_ids
            .iter()
            .map(|(identity, id)| (*id, identity.clone()))
            .collect(),
    );
    let mut mono =
        crate::mono::monomorphize_with_crates(frontend.analysis.hir, &frontend.crate_ctx, context)
            .unwrap();
    let mut bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut mono);
    crate::dce::prune_unreachable_instances(&mut mono, &mut bodies);
    let mut program =
        crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies);
    assert!(crate::mir::agreement::check_mir_runtime_agreement(&program).is_clean());
    let outcome = BorrowChecker::run(&program);
    assert!(
        outcome.is_ok(),
        "portable field input must discharge its actual producer: {outcome:?}"
    );
    let factory = program
        .backend_contract
        .native_return_origins
        .iter()
        .find(|(_, origins)| {
            matches!(
                program.type_context.try_ty(origins.ret),
                Some(Ty::Function { .. })
            )
        })
        .map(|(key, _)| key.clone())
        .unwrap();
    program
        .backend_contract
        .native_return_origins
        .remove(&factory);
    assert!(crate::mir::agreement::check_mir_runtime_agreement(&program).is_clean());
    let diagnostics = BorrowChecker::run(&program).unwrap_err();
    assert!(
        diagnostics.0.iter().any(|diagnostic| diagnostic.message
            == "Callable result borrowing contract is not satisfied"),
        "a portable header must not replace missing factory authority"
    );
}

#[test]
fn disjoint_record_write_preserves_the_callable_producer() {
    const SOURCE: &str = r#"
struct Holder F
    < callback: F
    < tag: I64
retag: Holder F -> I64 -> Holder F
retag = mut holder, tag ->
    holder.tag = tag
    holder
apply: Holder (&I64 -> &I64) -> &I64 -> &I64
apply = holder, value -> holder.callback value
identity: &I64 -> &I64
identity = value -> value
main = !->
    value = 42
    holder = Holder
        callback: identity
        tag: 0
    changed = retag holder, 7
    selected = apply changed, &value
    (*selected).println!
"#;
    let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "binding_disjoint_record_write",
        SOURCE,
        true,
    );
    let outcome = BorrowChecker::run(&program);
    assert!(
        outcome.is_ok(),
        "a disjoint scalar field write cannot erase callable code evidence: {outcome:?}"
    );
}

#[test]
fn disjoint_record_writes_preserve_producers_across_branches_and_loops() {
    const SOURCE: &str = r#"
struct Holder F
    < callback: F
    < tag: I64
retag: Holder F -> Bool -> Holder F
retag = mut holder, choose ->
    if choose then holder.tag = 7
    else holder.tag = 8
    mut repeat = choose
    while repeat
        holder.tag = 9
        repeat = false
    holder
apply: Holder (&I64 -> &I64) -> &I64 -> &I64
apply = holder, value -> holder.callback value
identity: &I64 -> &I64
identity = value -> value
main = !->
    value = 42
    holder = Holder
        callback: identity
        tag: 0
    changed = retag holder, true
    selected = apply changed, &value
"#;
    let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "binding_record_write_join",
        SOURCE,
        false,
    );
    let outcome = BorrowChecker::run(&program);
    assert!(
        outcome.is_ok(),
        "branch/loop joins must retain disjoint producer evidence: {outcome:?}"
    );
}

#[test]
fn nested_record_overwrite_transports_only_the_replacement_producer() {
    const SOURCE: &str = r#"
struct Holder F
    < callback: F
    < tag: I64
struct Outer F
    < holder: Holder F
replace: Outer F -> Holder F -> Outer F
replace = mut outer, holder ->
    outer.holder = holder
    outer.holder.tag = 7
    outer
apply: Outer (&I64 -> &I64) -> &I64 -> &I64
apply = outer, value -> outer.holder.callback value
identity: &I64 -> &I64
identity = value -> value
replacement: &I64 -> &I64
replacement = replacement_value -> replacement_value
main = !->
    value = 42
    original = Holder
        callback: identity
        tag: 0
    outer = Outer
        holder: original
    fresh = Holder
        callback: replacement
        tag: 1
    changed = replace outer, fresh
    selected = apply changed, &value
"#;
    let (mut program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "binding_nested_record_write",
        SOURCE,
        false,
    );
    let outcome = BorrowChecker::run(&program);
    assert!(
        outcome.is_ok(),
        "nested writes must retain the actual replacement: {outcome:?}"
    );
    let replacement = program
        .functions
        .values()
        .find(|body| {
            body.arg_count == 1
                && body.local_decls[1].span.as_ref().is_some_and(|span| {
                    SOURCE.get(span.start..span.end) == Some("replacement_value")
                })
        })
        .unwrap()
        .id
        .clone();
    let key = program.backend_contract.function_bodies[&replacement].clone();
    let mut direct = program.clone();
    let body = direct
        .functions
        .values_mut()
        .find(|body| {
            body.arg_count == 2
                && matches!(
                    direct.type_context.try_ty(body.ret_type),
                    Some(Ty::Struct { .. })
                )
        })
        .unwrap();
    let (block, statement, destination) = body
        .basic_blocks
        .iter()
        .enumerate()
        .find_map(|(block, data)| {
            data.statements
                .iter()
                .enumerate()
                .find_map(|(statement, data)| match &data.kind {
                    StatementKind::Assign(destination, Rvalue::Use(_))
                        if destination.projection.len() == 1 =>
                    {
                        Some((block, statement, destination.clone()))
                    }
                    _ => None,
                })
        })
        .unwrap();
    let ty = direct
        .backend_contract
        .place_type_id(&direct.type_context, body, &destination)
        .unwrap();
    let Ty::Struct { id, .. } = direct.type_context.ty(ty) else {
        panic!("record field")
    };
    let StatementKind::Assign(_, Rvalue::Use(Operand::Copy(source) | Operand::Move(source))) =
        &body.basic_blocks[block].statements[statement].kind
    else {
        panic!("record input")
    };
    let mut callback = source.clone();
    callback.projection.push(Projection::Field {
        index: 0,
        identity: None,
    });
    body.basic_blocks[block].statements[statement].kind = StatementKind::Assign(
        destination,
        Rvalue::Aggregate(
            crate::mir::AggregateKind::Struct {
                id: *id,
                display_name: "Holder".into(),
            },
            vec![Operand::Move(callback), Operand::Constant(Constant::Int(1))],
        ),
    );
    let agreement = crate::mir::agreement::check_mir_runtime_agreement(&direct);
    assert!(agreement.is_clean(), "{agreement:?}");
    let outcome = BorrowChecker::run(&direct);
    assert!(
        outcome.is_ok(),
        "direct nested aggregates require relative, normalized field paths: {outcome:?}"
    );
    for program in [&mut program, &mut direct] {
        program
            .backend_contract
            .function_bodies
            .remove(&replacement);
        program.functions.remove(&replacement);
        program.backend_contract.native_return_origins.remove(&key);
        program
            .backend_contract
            .callables
            .get_mut(&key)
            .unwrap()
            .kind = MirCallableKind::ObjectProvided;
        assert!(crate::mir::agreement::check_mir_runtime_agreement(program).is_clean());
        let diagnostics = BorrowChecker::run(program).unwrap_err();
        assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message
            == "Callable result borrowing contract is not satisfied"),
            "the overwritten producer and specialized field layout cannot certify a missing replacement");
    }
}

#[test]
fn record_field_write_transports_a_measured_capturing_callable() {
    const SOURCE: &str = r#"
struct Holder
    < value: &I64
impl Holder
    ~@extract: &I64
    ~@extract = -> self.value
struct Envelope F
    < callback: F
    < tag: I64
lang fn_once
< trait Consume Args, Ret
    lang output
    type Output
    lang method
    ~@consume: Args -> Ret
retag: Envelope F -> Envelope F
retag = mut envelope ->
    envelope.tag = 7
    envelope
apply: Envelope F -> &I64 where F: Consume (), &I64
apply = envelope -> envelope.callback!
main = !->
    value = 42
    holder = Holder
        value: &value
    callback = -> holder.extract!
    envelope = Envelope
        callback: callback
        tag: 0
    changed = retag envelope
    result = apply changed
"#;
    let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "binding_capturing_record_write",
        SOURCE,
        false,
    );
    let outcome = BorrowChecker::run(&program);
    assert!(
        outcome.is_ok(),
        "field transport must retain measured capture evidence, not invent it: {outcome:?}"
    );
    let mut no_contract = program.clone();
    let apply = program
        .backend_contract
        .generic_callable_input_obligations
        .iter()
        .find(|(_, requirements)| {
            requirements.values().any(|input| {
                matches!(
                    program.type_context.try_ty(input.subject),
                    Some(Ty::Struct { .. })
                )
            })
        })
        .and_then(|(key, _)| MirCallable::Resolved(key.clone()).function_id())
        .unwrap();
    no_contract
        .backend_contract
        .generic_callable_input_obligations
        .clear();
    assert!(crate::mir::agreement::check_mir_runtime_agreement(&no_contract).is_clean());
    let proof = BorrowChecker::native_return_loan_proof(&no_contract);
    assert!(
        !proof.sources.callable_results.inputs.contains_key(&apply),
        "specialized callable-field layout cannot seed a definition-owned input hypothesis"
    );
}

#[test]
fn adapter_composes_repeated_definition_owned_requirements() {
    const SOURCE: &str = r#"
invoke: F -> &I64 -> &I64 where F: FnMut &I64, &I64
invoke = mut callback, value -> callback value
identity: &I64 -> &I64
identity = value -> value
main = !->
    value = 42
    adapter = inner, item, choose ->
        if choose then invoke inner, item
        else invoke inner, item
    result = adapter identity, &value, true
"#;
    let (mut program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "binding_composed_adapter",
        SOURCE,
        true,
    );
    let outcome = BorrowChecker::run(&program);
    assert!(
        outcome.is_ok(),
        "all forwarded requirements must be discharged: {outcome:?}"
    );
    let mut exposed = program.clone();
    let main = exposed
        .functions
        .values()
        .find(|body| body.name == "main")
        .unwrap()
        .id
        .clone();
    let (block, ty, closure) = exposed.functions[&main]
        .basic_blocks
        .iter()
        .enumerate()
        .find_map(|(index, block)| {
            block.statements.iter().find_map(|statement| {
                let StatementKind::Assign(destination, Rvalue::Closure(closure)) = &statement.kind
                else {
                    return None;
                };
                (exposed.functions[&main].local_decls[destination.local.0]
                    .name
                    .as_deref()
                    == Some("adapter"))
                .then(|| {
                    (
                        index,
                        exposed.functions[&main].local_decls[destination.local.0].ty,
                        closure.clone(),
                    )
                })
            })
        })
        .unwrap();
    let reference = exposed.type_context.intern_type(&Type::Reference {
        inner: Box::new(exposed.type_context.type_for(ty)),
        mutable: true,
    });
    let body = exposed.functions.get_mut(&main).unwrap();
    let duplicate = Local(body.local_decls.len());
    let alias = Local(duplicate.0 + 1);
    for ty in [ty, reference] {
        body.local_decls.push(LocalDecl {
            ty,
            mutability: Mutability::Mut,
            name: None,
            span: Some(Span::test()),
            source: LocalSource::Temporary,
        });
    }
    let place = Place {
        local: duplicate,
        projection: vec![],
    };
    body.basic_blocks[block].statements.extend([
        StatementData::assign(place.clone(), Rvalue::Closure(closure), Some(Span::test())),
        StatementData::assign(
            Place {
                local: alias,
                projection: vec![],
            },
            Rvalue::Ref(Mutability::Mut, place),
            Some(Span::test()),
        ),
    ]);
    assert!(crate::mir::agreement::check_mir_runtime_agreement(&exposed).is_clean());
    let diagnostics = BorrowChecker::run(&exposed).unwrap_err();
    assert!(
        diagnostics.0.iter().any(|diagnostic| diagnostic.message
            == "Callable result borrowing contract is not satisfied"),
        "an exposed construction cannot reuse another construction's adapter certificate"
    );
    let identity = program
        .functions
        .values()
        .find(|body| {
            body.arg_count == 1
                && body.local_decls[1]
                    .span
                    .as_ref()
                    .is_some_and(|span| SOURCE.get(span.start..span.end) == Some("value"))
        })
        .unwrap()
        .id
        .clone();
    let key = program
        .backend_contract
        .function_bodies
        .remove(&identity)
        .unwrap();
    program.functions.remove(&identity);
    program.backend_contract.native_return_origins.remove(&key);
    program
        .backend_contract
        .callables
        .get_mut(&key)
        .unwrap()
        .kind = MirCallableKind::ObjectProvided;
    let negative = BorrowChecker::run(&program).unwrap_err();
    assert!(
        negative.0.iter().any(|diagnostic| diagnostic.message
            == "Callable result borrowing contract is not satisfied"),
        "a composed requirement never certifies a missing producer"
    );
}

#[test]
fn adapter_requirements_compose_through_nested_relays_and_incidental_calls() {
    const SOURCE: &str = r#"
invoke: F -> &I64 -> &I64 where F: FnMut &I64, &I64
invoke = mut callback, value -> callback value
identity: &I64 -> &I64
identity = value -> value
unrelated: () -> ()
unrelated = !-> ()
main = !->
    value = 42
    outer = callback, value ->
        adapter = inner, item, choose ->
            result = if choose then invoke inner, item
            else invoke inner, item
            unrelated!
            result
        adapter callback, value, true
    result = outer identity, &value
"#;
    let (mut program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "binding_transitive_adapters",
        SOURCE,
        true,
    );
    let outcome = BorrowChecker::run(&program);
    assert!(
        outcome.is_ok(),
        "requirements must compose across all relay boundaries: {outcome:?}"
    );
    let identity = program
        .functions
        .values()
        .find(|body| {
            body.arg_count == 1
                && body.local_decls[1]
                    .span
                    .as_ref()
                    .is_some_and(|span| SOURCE.get(span.start..span.end) == Some("value"))
        })
        .unwrap()
        .id
        .clone();
    let key = program
        .backend_contract
        .function_bodies
        .remove(&identity)
        .unwrap();
    program.functions.remove(&identity);
    program.backend_contract.native_return_origins.remove(&key);
    program
        .backend_contract
        .callables
        .get_mut(&key)
        .unwrap()
        .kind = MirCallableKind::ObjectProvided;
    let negative = BorrowChecker::run(&program).unwrap_err();
    assert!(
        negative.0.iter().any(|diagnostic| diagnostic.message
            == "Callable result borrowing contract is not satisfied"),
        "every transitive binding still needs an independently measured producer"
    );
}

#[test]
fn composed_adapter_keeps_the_actual_shared_argument_loan() {
    const SOURCE: &str = r#"
invoke: F -> &I64 -> &I64 where F: FnMut &I64, &I64
invoke = mut callback, value -> callback value
identity: &I64 -> &I64
identity = value -> value
main = !->
    mut value = 42
    adapter = inner, item, choose ->
        if choose then invoke inner, item
        else invoke inner, item
    selected = adapter identity, &value, true
    OBSERVE
    (*selected).println!
"#;
    for conflict in [false, true] {
        let source = SOURCE.replace("OBSERVE", if conflict { "value = 7" } else { "()" });
        let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
            if conflict {
                "binding_composed_shared_loan_negative"
            } else {
                "binding_composed_shared_loan_positive"
            },
            &source,
            true,
        );
        let outcome = BorrowChecker::run(&program);
        if conflict {
            let diagnostics = outcome.unwrap_err();
            assert!(
                diagnostics
                    .0
                    .iter()
                    .any(|diagnostic| diagnostic.message == "borrow conflict on this value"),
                "composed callable evidence must retain the shared input loan: {diagnostics:?}"
            );
        } else {
            assert!(
                outcome.is_ok(),
                "branch-exclusive input must remain valid: {outcome:?}"
            );
        }
    }
}

#[test]
fn composed_adapter_checks_every_branch_input_not_just_the_selected_one() {
    const SOURCE: &str = r#"
invoke: F -> &I64 -> &I64 where F: FnMut &I64, &I64
invoke = mut callback, value -> callback value
left: &I64 -> &I64
left = left_value -> left_value
right: &I64 -> &I64
right = right_value -> right_value
main = !->
    value = 42
    adapter = first, second, item, choose ->
        if choose then invoke first, item
        else invoke second, item
    result = adapter left, right, &value, true
"#;
    let (mut program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "binding_all_branch_inputs",
        SOURCE,
        true,
    );
    let outcome = BorrowChecker::run(&program);
    assert!(
        outcome.is_ok(),
        "both independent inputs have producers: {outcome:?}"
    );
    let right = program
        .functions
        .values()
        .find(|body| {
            body.arg_count == 1
                && body.local_decls[1]
                    .span
                    .as_ref()
                    .is_some_and(|span| SOURCE.get(span.start..span.end) == Some("right_value"))
        })
        .unwrap()
        .id
        .clone();
    let key = program
        .backend_contract
        .function_bodies
        .remove(&right)
        .unwrap();
    program.functions.remove(&right);
    program.backend_contract.native_return_origins.remove(&key);
    program
        .backend_contract
        .callables
        .get_mut(&key)
        .unwrap()
        .kind = MirCallableKind::ObjectProvided;
    let diagnostics = BorrowChecker::run(&program).unwrap_err();
    assert!(
        diagnostics.0.iter().any(|diagnostic| diagnostic.message
            == "Callable result borrowing contract is not satisfied"),
        "the unselected branch is not permission to omit its input's producer"
    );
}

#[test]
fn vec_traversal_transports_measured_callback_producers() {
    const SOURCE: &str = r#"
some: I64 -> Option I64
some = value -> pure value
main = !->
    mut values = Vec::new!
    values.push 1
    result = values.traverse some
"#;
    let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "binding_vec_traversal",
        SOURCE,
        true,
    );
    let outcome = BorrowChecker::run(&program);
    if let Err(errors) = &outcome {
        for diagnostic in &errors.0 {
            eprintln!(
                "VEC_TRAVERSAL diagnostic={} location={:?}",
                diagnostic.message, diagnostic.location
            );
        }
    }
    assert!(
        outcome.is_ok(),
        "Vec traversal must discharge its wrapped callable inputs"
    );
}

#[test]
fn vec_fold_transports_measured_callback_producers() {
    const SOURCE: &str = r#"
main = !->
    mut values = Vec::new!
    values.push 1
    result = values.foldl (pair -> pair.0 + pair.1), 0
"#;
    let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "binding_vec_fold",
        SOURCE,
        true,
    );
    let outcome = BorrowChecker::run(&program);
    if let Err(errors) = &outcome {
        for diagnostic in &errors.0 {
            eprintln!(
                "VEC_FOLD diagnostic={} location={:?}",
                diagnostic.message, diagnostic.location
            );
        }
    }
    assert!(
        outcome.is_ok(),
        "Vec fold must transport its callback environment without receiver-slot loans"
    );
}

#[test]
fn generic_fn_mut_repeated_calls_keep_the_code_obligation_not_empty_effects() {
    const SOURCE: &str = r#"
select: F -> &I64 -> &I64 -> &I64 where F: FnMut &I64, &I64
select = mut callback, first, second ->
    callback first
    callback second
identity: &I64 -> &I64
identity = value -> value
main = !->
    first = 1
    second = 2
    selected = select identity, &first, &second
"#;
    let (mut program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "binding_repeated_fn_mut",
        SOURCE,
        true,
    );
    let positive = BorrowChecker::run(&program);
    assert!(
        positive.is_ok(),
        "both native FnMut invocations must use the checked definition requirement: {positive:?}"
    );
    let identity = program
        .functions
        .values()
        .find_map(|body| {
            (body
                .local_decls
                .get(1)
                .and_then(|decl| decl.span.as_ref())
                .is_some_and(|span| SOURCE.get(span.start..span.end) == Some("value"))
                && body.arg_count == 1
                && body.closure_captures.is_empty())
            .then(|| body.id.clone())
        })
        .unwrap();
    let key = program
        .backend_contract
        .function_bodies
        .remove(&identity)
        .unwrap();
    program.functions.remove(&identity);
    program.backend_contract.native_return_origins.remove(&key);
    program
        .backend_contract
        .callables
        .get_mut(&key)
        .unwrap()
        .kind = MirCallableKind::ObjectProvided;
    let negative = BorrowChecker::run(&program).unwrap_err();
    assert!(
        negative.0.iter().any(|diagnostic| diagnostic.message
            == "Callable result borrowing contract is not satisfied"),
        "every actual generic input binding still needs its producer, not a matching header"
    );
}

#[test]
fn partial_constructor_callable_demand_is_definition_owned() {
    const SOURCE: &str = r#"
enum Wrapped T, E
    Some T
    Error E
trait Applied for F _
    ap: F (A -> B) -> F A -> F B
impl Applied for Wrapped _, E
    ap = functions, values ->
        match functions
            Wrapped::Some callback =>
                match values
                    Wrapped::Some value => Wrapped::Some (callback value)
                    Wrapped::Error error => Wrapped::Error error
            Wrapped::Error error => Wrapped::Error error
identity: &I64 -> &I64
identity = value -> value
apply: F (A -> B) -> F A -> F B where F _: Applied
apply = functions, values -> F::Applied::ap functions, values
main = !->
    value = 42
    functions: Wrapped (&I64 -> &I64), () = Wrapped::Some identity
    values: Wrapped &I64, () = Wrapped::Some &value
    result: Wrapped &I64, () = apply functions, values
"#;
    let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "binding_partial_constructor",
        SOURCE,
        false,
    );
    let outcome = BorrowChecker::run(&program);
    assert!(
        outcome.is_ok(),
        "canonical partial constructor and all callable bindings must agree: {outcome:?}"
    );
    assert!(
        program
            .backend_contract
            .generic_callable_input_obligations
            .len()
            >= 2,
        "both the retained generic relay and implementation require actual discharge"
    );
}

#[test]
fn wrapped_callable_transport_keeps_only_actual_producer_facts() {
    const SOURCE: &str = r#"
enum Wrapped T
    Some T
    None
identity: &I64 -> &I64
identity = value -> value
pure: T -> Wrapped T
pure = value -> Wrapped::Some value
apply: Wrapped (&I64 -> &I64) -> &I64 -> &I64
apply = wrapped, value ->
    match wrapped
        Wrapped::Some callback => callback value
        Wrapped::None => value
main = !->
    value = 42
    wrapped = pure identity
    result = apply wrapped, &value
"#;
    let (mut program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "binding_wrapped_transport",
        SOURCE,
        false,
    );
    let owner = program
        .functions
        .values()
        .find(|body| body.name == "main")
        .unwrap()
        .id
        .clone();
    assert!(program.functions.len() <= 8);
    let (block, argument) = program.functions[&owner]
        .basic_blocks
        .iter()
        .enumerate()
        .find_map(|(index, block)| {
            let Some(Terminator::Call {
                args,
                span: Some(span),
                ..
            }) = &block.terminator
            else {
                return None;
            };
            (SOURCE.get(span.start..span.end) == Some("apply wrapped, &value"))
                .then(|| (index, args[0].clone()))
        })
        .unwrap();
    let (pure, identity) = program.functions[&owner]
        .basic_blocks
        .iter()
        .find_map(|block| {
            let Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(pure)),
                args,
                span: Some(span),
                ..
            }) = &block.terminator
            else {
                return None;
            };
            if SOURCE.get(span.start..span.end) != Some("pure identity") {
                return None;
            }
            let [argument] = args.as_slice() else {
                panic!("actual one-argument factory");
            };
            let identity = match argument {
                Operand::Constant(Constant::Callable(identity)) => identity.clone(),
                Operand::Copy(source) | Operand::Move(source) => {
                    let definitions = program.functions[&owner]
                        .basic_blocks
                        .iter()
                        .flat_map(|block| &block.statements)
                        .filter_map(|statement| match &statement.kind {
                            StatementKind::Assign(
                                destination,
                                Rvalue::Use(Operand::Constant(Constant::Callable(callable))),
                            ) if destination == source => Some(callable.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(definitions.len(), 1, "actual typed function binding");
                    definitions[0].clone()
                }
                _ => panic!("actual callable producer"),
            };
            Some((pure.function_id().unwrap(), identity.function_id().unwrap()))
        })
        .unwrap();
    let identity_key = program.backend_contract.function_bodies[&identity].clone();
    for present in [true, false] {
        if !present {
            program.functions.remove(&identity);
            program.backend_contract.function_bodies.remove(&identity);
            program
                .backend_contract
                .callables
                .get_mut(&identity_key)
                .unwrap()
                .kind = MirCallableKind::ObjectProvided;
            program
                .backend_contract
                .native_return_origins
                .remove(&identity_key);
        }
        let proof = BorrowChecker::native_return_loan_proof(&program);
        let outcome = BorrowChecker::run(&program);
        if present {
            assert!(
                outcome.is_ok(),
                "wrapped reference callback must pass its conditional input contract: {outcome:?}"
            );
        } else {
            assert!(
                outcome
                    .unwrap_err()
                    .0
                    .iter()
                    .any(|diagnostic| diagnostic.message
                        == "Callable result borrowing contract is not satisfied"),
                "missing producer must reject without execution"
            );
        }
        assert!(
            !proof.sources.callable_results.inputs.contains_key(&pure),
            "a generic factory input is not an invocation hypothesis"
        );
        let body = &program.functions[&owner];
        let analysis = Bindings {
            program: &program,
            owner: body,
            lifetime: &proof.lifetime,
            values: &proof.measured,
            backend_valid: true,
            calls: Some(&proof.sources),
        };
        let flow = run_fixpoint(&analysis, body);
        let mut state = flow.entry_sets[block].clone();
        for statement in &body.basic_blocks[block].statements {
            analysis.apply_statement(&mut state, statement);
        }
        let (Operand::Copy(destination) | Operand::Move(destination)) = &argument else {
            panic!("actual wrapper binding");
        };
        let facts = analysis.forwarded_operand(&state, &argument, destination);
        let evidence = facts.iter().find_map(|(place, evidence)| matches!(place.projection.as_slice(), [Projection::Downcast(variant), Projection::Field { index: 0, .. }] if variant.0 == 0).then_some(evidence));
        assert_eq!(
            evidence.is_some(),
            present,
            "a matching wrapper/header cannot replace a missing producer record"
        );
        if let Some(evidence) = evidence {
            assert_eq!(
                evidence.sources,
                CallableResultBorrowing::Sources(Box::new(vec![
                    CallableResultSource::ArgumentValue(0)
                ]))
            );
            assert_eq!(evidence.factory, Some(identity.clone()));
        }
    }
}

const SOURCE: &str = r#"
identity: &I64 -> &I64
identity = value -> value
bad: &I64 -> &I64
bad = value ->
    storage = 42
    &storage
invoke: (&I64 -> &I64) -> &I64 -> &I64
invoke = callback, value -> callback value
main = !->
    value = 42
    first = identity &value
    second = bad &value
    third = invoke identity, &value
"#;

struct Fixture {
    program: MirProgram,
    owner: MirFunctionId,
    identity: MirCallableKey,
    bad: MirCallableKey,
    invoke: MirCallableKey,
    unknown: TypeId,
    checked: TypeId,
    span: Span,
}

fn fixture(name: &str) -> Fixture {
    let (mut program, _) =
        super::super::super::allocator_trace_tests::mutation_source_program(name, SOURCE, false);
    let owner = program
        .functions
        .values()
        .find(|body| body.name == "main")
        .unwrap()
        .id
        .clone();
    let mut calls = program.functions[&owner]
        .basic_blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))),
                span: Some(span),
                ..
            }) => Some((key.clone(), span.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    calls.sort_by_key(|(_, span)| span.start);
    assert_eq!(calls.len(), 3);
    for ((_, span), source) in
        calls
            .iter()
            .zip(["identity &value", "bad &value", "invoke identity, &value"])
    {
        assert_eq!(&SOURCE[span.start..span.end], source);
    }
    let identity = calls[0].0.clone();
    let bad = calls[1].0.clone();
    let invoke = calls[2].0.clone();
    let header = program.backend_contract.callable(&identity).unwrap();
    let unknown = program.type_context.intern_type(&Type::function(
        vec![program
            .type_context
            .type_for(header.signature.params[0].semantic_ty)],
        program
            .type_context
            .type_for(header.signature.ret.semantic_ty),
    ));
    let mut ty = program.type_context.type_for(unknown);
    let Type::Function {
        result_borrowing, ..
    } = &mut ty
    else {
        panic!();
    };
    *result_borrowing =
        CallableResultBorrowing::Sources(Box::new(vec![CallableResultSource::ArgumentValue(0)]));
    let checked = program.type_context.intern_type(&ty);
    let span = program.functions[&owner]
        .basic_blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| statement.span.clone())
        .unwrap();
    Fixture {
        program,
        owner,
        identity,
        bad,
        invoke,
        unknown,
        checked,
        span,
    }
}

impl Fixture {
    fn local(&mut self, ty: TypeId) -> Local {
        let body = self.program.functions.get_mut(&self.owner).unwrap();
        let local = Local(body.local_decls.len());
        body.local_decls.push(LocalDecl {
            ty,
            mutability: Mutability::Mut,
            name: None,
            span: Some(self.span.clone()),
            source: LocalSource::UserBinding,
        });
        local
    }

    fn assign(&self, local: Local, value: Operand) -> StatementData {
        StatementData::assign(
            Place {
                local,
                projection: vec![],
            },
            Rvalue::Use(value),
            Some(self.span.clone()),
        )
    }

    fn constant(key: &MirCallableKey) -> Operand {
        Operand::Constant(Constant::Callable(MirCallable::Resolved(key.clone())))
    }

    fn validate(&self) -> Diagnostics {
        let lifetime = BorrowChecker::reference_return_summaries(&self.program);
        let values =
            BorrowChecker::return_summaries_for_purpose(&self.program, OriginPurpose::ValueLoans);
        let mut diagnostics = Diagnostics::default();
        validate(&self.program, &lifetime, &values, None, &mut diagnostics);
        diagnostics
    }

    fn assert_rejected(&self, diagnostics: &Diagnostics) {
        assert_eq!(diagnostics.0.len(), 1, "{diagnostics:?}");
        assert_eq!(
            diagnostics.0[0].message,
            "Callable result borrowing contract is not satisfied"
        );
        assert_eq!(diagnostics.0[0].primary.as_ref().unwrap().span, self.span);
    }
}

#[test]
fn owned_root_loans_preserve_only_disjoint_callable_binding_evidence() {
    let mut f = fixture("binding_disjoint_root_loan");
    let callback = f.local(f.unknown);
    let scalar = f.program.type_context.intern_type(&Type::I64);
    let value = f.local(scalar);
    let reference = f.program.type_context.intern_type(&Type::Reference {
        mutable: true,
        inner: Box::new(Type::I64),
    });
    let alias = f.local(reference);
    let callable_reference = f.program.type_context.intern_type(&Type::Reference {
        mutable: true,
        inner: Box::new(f.program.type_context.type_for(f.unknown)),
    });
    let exposed = f.local(callable_reference);
    let lifetime = BorrowChecker::reference_return_summaries(&f.program);
    let values = BorrowChecker::return_summaries_for_purpose(&f.program, OriginPurpose::ValueLoans);
    let owner = &f.program.functions[&f.owner];
    let analysis = Bindings {
        program: &f.program,
        owner,
        lifetime: &lifetime,
        values: &values,
        backend_valid: true,
        calls: None,
    };
    let root = |local| Place {
        local,
        projection: vec![],
    };
    let evidence = analysis
        .native(&MirCallable::Resolved(f.identity.clone()), f.unknown)
        .unwrap();
    assert!(
        !evidence.isolated,
        "the test must not rely on isolated-storage exemption"
    );
    let initial = State(Some(HashMap::from([(root(callback), evidence.clone())])));
    let mut state = initial.clone();
    analysis.apply_statement(
        &mut state,
        &StatementData::assign(
            root(alias),
            Rvalue::Ref(Mutability::Mut, root(value)),
            Some(f.span.clone()),
        ),
    );
    assert_eq!(
        state.0.as_ref().unwrap().get(&root(callback)),
        Some(&evidence)
    );
    analysis.apply_statement(
        &mut state,
        &StatementData::assign(
            Place {
                local: alias,
                projection: vec![Projection::Deref],
            },
            Rvalue::Use(Operand::Constant(Constant::Int(7))),
            Some(f.span.clone()),
        ),
    );
    assert!(
        state.0.as_ref().unwrap().is_empty(),
        "an aliased write still supplies no effects authority"
    );
    let mut state = initial;
    analysis.apply_statement(
        &mut state,
        &StatementData::assign(
            root(exposed),
            Rvalue::Ref(Mutability::Mut, root(callback)),
            Some(f.span.clone()),
        ),
    );
    assert!(
        !state.0.as_ref().unwrap().contains_key(&root(callback)),
        "the exposed callable root must lose its binding fact"
    );
}

#[test]
fn native_function_value_uses_measured_body_not_its_destination_header() {
    let mut f = fixture("binding_native");
    let local = f.local(f.checked);
    for (key, accepted) in [(f.identity.clone(), true), (f.bad.clone(), false)] {
        f.program.functions.get_mut(&f.owner).unwrap().basic_blocks = vec![BasicBlock {
            statements: vec![f.assign(local, Fixture::constant(&key))],
            terminator: Some(Terminator::Return),
        }];
        let diagnostics = f.validate();
        assert_eq!(diagnostics.0.is_empty(), accepted, "{diagnostics:?}");
        if !accepted {
            f.assert_rejected(&diagnostics);
        }
    }
    let id = MirCallable::Resolved(f.identity.clone())
        .function_id()
        .unwrap();
    let lifetime = BorrowChecker::reference_return_summaries(&f.program);
    let mut values =
        BorrowChecker::return_summaries_for_purpose(&f.program, OriginPurpose::ValueLoans);
    values.remove(&id);
    f.program.functions.get_mut(&f.owner).unwrap().basic_blocks[0].statements =
        vec![f.assign(local, Fixture::constant(&f.identity))];
    let mut diagnostics = Diagnostics::default();
    validate(&f.program, &lifetime, &values, None, &mut diagnostics);
    f.assert_rejected(&diagnostics);
}

#[test]
fn local_consuming_capture_discharges_definition_owned_callable_input() {
    let source = "struct Holder\n    < value: &I64\nimpl Holder\n    ~@extract: &I64\n    ~@extract = -> self.value\nread: &I64 -> I64\nread = value -> *value\ninvoke = callback -> callback!\nmain = ->\n    value = 42\n    holder = Holder\n        value: &value\n    callback = -> holder.extract!\n    result = invoke callback\n    read result\n";
    let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "local_consuming_input",
        source,
        false,
    );
    let proof = BorrowChecker::native_return_loan_proof(&program);
    let owner = program
        .functions
        .values()
        .find(|body| body.name == "main")
        .unwrap();
    let analysis = Bindings {
        program: &program,
        owner,
        lifetime: &proof.lifetime,
        values: &proof.measured,
        backend_valid: true,
        calls: Some(&proof.sources),
    };
    let results = run_fixpoint(&analysis, owner);
    let mut checked = false;
    for (index, block) in owner.basic_blocks.iter().enumerate() {
        if !results.reachable[index] {
            continue;
        }
        let mut state = results.entry_sets[index].clone();
        for statement in &block.statements {
            analysis.apply_statement(&mut state, statement);
        }
        if let Some(Terminator::Call { func, args, .. }) = &block.terminator {
            if let Some(params) = analysis.call_parameters(func) {
                for (ty, argument) in params.into_iter().zip(args) {
                    if analysis.obligation(ty).is_some() {
                        let evidence = analysis.operand(&state, argument, ty).unwrap();
                        assert_eq!(
                            evidence.sources,
                            CallableResultBorrowing::Sources(Box::new(vec![
                                CallableResultSource::EnvironmentValue
                            ]))
                        );
                        assert_eq!(evidence.sources, evidence.value_sources);
                        assert!(
                            matches!(program.type_context.try_ty(ty), Some(Ty::Function { callable_kind: crate::types::CallableKind::FnOnce, captures, .. }) if captures.is_empty())
                        );
                        checked = true;
                    }
                }
            }
        }
    }
    assert!(
        checked,
        "actual measured binding, not just a successful opaque scalar call"
    );
    BorrowChecker::run(&program).unwrap();
}

#[test]
fn scalar_bodies_measure_dependencies_without_shape_independence_or_local_escape() {
    for (name, source, expected) in [
        ("scalar_identity_measure", "main = ->\n    callback = value -> value\n    callback 42\n",
            Some(vec![CallableResultSource::ArgumentValue(0)])),
        ("scalar_instruction_measure", "main = ->\n    callback = value -> ~I64Add value, 1\n    callback 41\n",
            Some(vec![CallableResultSource::ArgumentValue(0)])),
        ("scalar_opaque_measure", "extern external: I64 -> I64\nmain = ->\n    callback = value -> external value\n    callback 42\n",
            Some(vec![CallableResultSource::ArgumentValue(0), CallableResultSource::Unknown])),
        ("scalar_local_measure", "extern external: &I64 -> I64\nmain = ->\n    callback = value -> external &value\n    callback 42\n", None),
    ] {
        let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(name, source, false);
        let body = program.functions.values().find(|body| matches!(body.id, MirFunctionId::Closure(_))).unwrap();
        assert!(matches!(program.type_context.try_ty(body.ret_type), Some(Ty::I64)));
        let proof = BorrowChecker::native_return_loan_proof(&program);
        let sources = body_sources_with_calls(&program, body, &proof.lifetime, &proof.measured, Some(&proof.sources));
        assert_eq!(sources, expected.map(|sources| CallableResultBorrowing::Sources(Box::new(sources))),
            "scalar shape cannot establish independence or convert invocation-local storage into external authority");
        assert_eq!(sources, body_sources_with_calls(&program, body, &proof.measured, &proof.measured, Some(&proof.sources)));
    }
}

#[test]
fn autoderef_section_scalar_measurement() {
    let source = "infix 1 |>>\n|>>: T -> (T -> U) -> U\n|>> = value, callback -> callback value\nstruct Inner\n    < value: I64\nimpl Inner\n    @read = -> self.value\nstruct Box T\n    < value: T\nimpl Deref for Box T\n    type Target = T\n    @* = -> &@value\nmake: () -> Box Inner\nmake = -> Box\n    value: Inner\n        value: 42\nmain = -> make! |>> (.read!)\n";
    let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "autoderef_scalar",
        source,
        true,
    );
    let proof = BorrowChecker::native_return_loan_proof(&program);
    let mut checked = 0;
    for body in program
        .functions
        .values()
        .filter(|body| matches!(body.id, MirFunctionId::Closure(_)))
    {
        let sources = body_sources_with_calls(
            &program,
            body,
            &proof.lifetime,
            &proof.measured,
            Some(&proof.sources),
        );
        assert_eq!(sources, Some(CallableResultBorrowing::Sources(Box::new(vec![CallableResultSource::ArgumentValue(0)]))),
            "checked scalar field reads compose as value dependencies, not physical receiver storage");
        assert_eq!(
            sources,
            body_sources_with_calls(
                &program,
                body,
                &proof.measured,
                &proof.measured,
                Some(&proof.sources)
            )
        );
        checked += 1;
    }
    assert_eq!(checked, 1);
    BorrowChecker::run(&program).unwrap();
}

#[test]
fn generic_operator_keeps_a_foreign_scalar_callback_uncertified() {
    use crate::diagnostic::DiagnosticCode;
    let source = "extern external: I64 -> I64\ninfix 1 |>>\n|>>: T -> (T -> U) -> U\n|>> = x, f -> f x\nmain = -> 42 |>> external\n";
    let (program, _) = super::super::super::allocator_trace_tests::mutation_source_program(
        "generic_operator_producer",
        source,
        false,
    );
    let diagnostics = BorrowChecker::run(&program).unwrap_err();
    assert!(diagnostics
        .0
        .iter()
        .all(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)));
    assert!(
        diagnostics.0.iter().any(|diagnostic| diagnostic.message
            == "Callable result borrowing contract is not satisfied"
            && diagnostic
                .primary
                .as_ref()
                .is_some_and(|label| &source[label.span.start..label.span.end] == "external")),
        "a foreign scalar header has no measured producer certificate: {diagnostics:?}"
    );
}

#[test]
fn copy_binding_joins_every_incoming_producer_and_missing_alternatives() {
    let mut f = fixture("binding_join");
    let incoming = f.local(f.unknown);
    let result = f.local(f.checked);
    for alternative in [Some(f.identity.clone()), Some(f.bad.clone()), None] {
        let alternative_assignment = alternative
            .as_ref()
            .map(|key| f.assign(incoming, Fixture::constant(key)))
            .into_iter()
            .collect();
        f.program.functions.get_mut(&f.owner).unwrap().basic_blocks = vec![
            BasicBlock {
                statements: vec![],
                terminator: Some(Terminator::SwitchInt {
                    discr: Operand::Constant(Constant::Bool(true)),
                    targets: vec![(1, BasicBlockId(1))],
                    otherwise: BasicBlockId(2),
                }),
            },
            BasicBlock {
                statements: vec![f.assign(incoming, Fixture::constant(&f.identity))],
                terminator: Some(Terminator::Goto(BasicBlockId(3))),
            },
            BasicBlock {
                statements: alternative_assignment,
                terminator: Some(Terminator::Goto(BasicBlockId(3))),
            },
            BasicBlock {
                statements: vec![f.assign(
                    result,
                    Operand::Copy(Place {
                        local: incoming,
                        projection: vec![],
                    }),
                )],
                terminator: Some(Terminator::Return),
            },
        ];
        let diagnostics = f.validate();
        if alternative.as_ref() == Some(&f.identity) {
            assert!(diagnostics.0.is_empty(), "{diagnostics:?}");
        } else {
            f.assert_rejected(&diagnostics);
        }
    }
}

#[test]
fn scalar_external_record_cannot_certify_a_bodyless_callable_factory() {
    use crate::hir::borrowing::{NativeReturnOrigins, NativeReturnSource};

    // Deliberately malformed, signature-agreeing evidence: an external scalar
    // source is not a producer record for a returned callable. No code runs.
    let mut f = fixture("binding_factory_bad_scalar_record");
    let id = MirCallable::Resolved(f.identity.clone())
        .function_id()
        .unwrap();
    let parameter = f
        .program
        .backend_contract
        .callable(&f.identity)
        .unwrap()
        .signature
        .params[0]
        .semantic_ty;
    f.program.functions.remove(&id);
    f.program.backend_contract.function_bodies.remove(&id);
    let header = f
        .program
        .backend_contract
        .callables
        .get_mut(&f.identity)
        .unwrap();
    header.kind = MirCallableKind::ObjectProvided;
    header.signature.ret.semantic_ty = f.checked;
    header.signature.ret.abi_ty = f.checked;
    f.program.backend_contract.native_return_origins.insert(
        f.identity.clone(),
        NativeReturnOrigins {
            params: vec![parameter],
            ret: f.checked,
            sources: vec![NativeReturnSource::UnknownExternal],
        },
    );
    let input = f.local(parameter);
    let destination = Place {
        local: f.local(f.checked),
        projection: vec![],
    };
    let argument = Operand::Copy(Place {
        local: input,
        projection: vec![],
    });
    assert!(
        CertifiedCalls::default()
            .returned_value(
                &f.program.functions[&f.owner],
                &f.program.type_context,
                &f.program.backend_contract,
                &Fixture::constant(&f.identity),
                &[argument],
                &destination,
            )
            .is_none(),
        "scalar external dependencies cannot establish non-scalar factory authority"
    );
}

#[test]
fn source_free_native_binding_requires_the_checked_producer_record() {
    let mut f = fixture("binding_imported");
    let local = f.local(f.checked);
    let id = MirCallable::Resolved(f.identity.clone())
        .function_id()
        .unwrap();
    let origins = BorrowChecker::checked_native_return_origins(&f.program)
        .remove(&id)
        .unwrap();
    f.program.functions.remove(&id);
    f.program.backend_contract.function_bodies.remove(&id);
    f.program
        .backend_contract
        .callables
        .get_mut(&f.identity)
        .unwrap()
        .kind = MirCallableKind::ObjectProvided;
    f.program.functions.get_mut(&f.owner).unwrap().basic_blocks = vec![BasicBlock {
        statements: vec![f.assign(local, Fixture::constant(&f.identity))],
        terminator: Some(Terminator::Return),
    }];
    f.assert_rejected(&f.validate());
    f.program
        .backend_contract
        .native_return_origins
        .insert(f.identity.clone(), origins.clone());
    assert!(f.validate().0.is_empty());
    f.program
        .backend_contract
        .native_return_origins
        .get_mut(&f.identity)
        .unwrap()
        .sources
        .push(origins.sources[0]);
    f.assert_rejected(&f.validate());
}

#[test]
fn callable_parameter_binding_checks_actual_argument_even_with_a_claimed_header() {
    let mut f = fixture("binding_argument");
    let invoke_id = MirCallable::Resolved(f.invoke.clone())
        .function_id()
        .unwrap();
    let header = f
        .program
        .backend_contract
        .callables
        .get_mut(&f.invoke)
        .unwrap();
    header.signature.params[0].semantic_ty = f.checked;
    f.program.functions.get_mut(&invoke_id).unwrap().local_decls[1].ty = f.checked;
    let call = f.program.functions[&f.owner].basic_blocks.iter().filter_map(|block| block.terminator.as_ref()).find(|term|
        matches!(term, Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))), .. } if key == &f.invoke)
    ).unwrap().clone();
    for (key, accepted) in [(f.identity.clone(), true), (f.bad.clone(), false)] {
        let mut call = call.clone();
        let Terminator::Call {
            args, target, span, ..
        } = &mut call
        else {
            panic!();
        };
        args[0] = Fixture::constant(&key);
        *target = BasicBlockId(1);
        *span = Some(f.span.clone());
        f.program.functions.get_mut(&f.owner).unwrap().basic_blocks = vec![
            BasicBlock {
                statements: vec![],
                terminator: Some(call),
            },
            BasicBlock {
                statements: vec![],
                terminator: Some(Terminator::Return),
            },
        ];
        let diagnostics = f.validate();
        assert_eq!(diagnostics.0.is_empty(), accepted, "{diagnostics:?}");
        if !accepted {
            f.assert_rejected(&diagnostics);
        }
    }
}

#[test]
fn callable_parameter_promise_is_not_itself_an_incoming_producer_proof() {
    let mut f = fixture("binding_parameter_header");
    let invoke_id = MirCallable::Resolved(f.invoke.clone())
        .function_id()
        .unwrap();
    f.program
        .backend_contract
        .callables
        .get_mut(&f.invoke)
        .unwrap()
        .signature
        .params[0]
        .semantic_ty = f.checked;
    let local = Local(f.program.functions[&invoke_id].local_decls.len());
    let statement = f.assign(
        local,
        Operand::Copy(Place {
            local: Local(1),
            projection: vec![],
        }),
    );
    let body = f.program.functions.get_mut(&invoke_id).unwrap();
    body.local_decls[1].ty = f.checked;
    body.local_decls.push(LocalDecl {
        ty: f.checked,
        mutability: Mutability::Not,
        name: None,
        span: Some(f.span.clone()),
        source: LocalSource::UserBinding,
    });
    body.basic_blocks = vec![BasicBlock {
        statements: vec![statement],
        terminator: Some(Terminator::Return),
    }];
    // Check only this universally declared body, rather than unrelated callers.
    let lifetime = BorrowChecker::reference_return_summaries(&f.program);
    let values = BorrowChecker::return_summaries_for_purpose(&f.program, OriginPurpose::ValueLoans);
    let analysis = Bindings {
        program: &f.program,
        owner: &f.program.functions[&invoke_id],
        lifetime: &lifetime,
        values: &values,
        backend_valid: true,
        calls: None,
    };
    let mut state = analysis.initial_state(analysis.owner);
    let statement = &analysis.owner.basic_blocks[0].statements[0];
    let StatementKind::Assign(destination, value) = &statement.kind else {
        panic!();
    };
    let mut diagnostics = Diagnostics::default();
    analysis.check(
        f.checked,
        analysis.assigned(&state, destination, value),
        statement.span.clone(),
        &mut diagnostics,
    );
    f.assert_rejected(&diagnostics);
    analysis.apply_statement(&mut state, statement);
    assert!(state.0.unwrap().keys().all(|place| place.local != local));
}

#[test]
fn definition_owned_callable_input_is_checked_conditionally_and_every_caller_must_discharge_it() {
    let mut f = fixture("binding_universal_input");
    let invoke_id = MirCallable::Resolved(f.invoke.clone())
        .function_id()
        .unwrap();
    // A declaration requirement is not measured producer evidence. This test
    // supplies that requirement separately, then checks the real generic body
    // and actual producer bindings rather than replacing its indirect call.
    f.program
        .backend_contract
        .callables
        .get_mut(&f.invoke)
        .unwrap()
        .signature
        .params[0]
        .semantic_ty = f.checked;
    let shared = f.program.type_context.intern_type(&Type::Reference {
        mutable: false,
        inner: Box::new(f.program.type_context.type_for(f.checked)),
    });
    let body = f.program.functions.get_mut(&invoke_id).unwrap();
    for local in &mut body.local_decls {
        if local.ty == f.unknown {
            local.ty = f.checked;
        } else if matches!(f.program.type_context.try_ty(local.ty),
            Some(Ty::Reference { mutable: false, inner }) if *inner == f.unknown)
        {
            local.ty = shared;
        }
    }
    f.program
        .backend_contract
        .callable_input_obligations
        .insert(
            f.invoke.clone(),
            std::collections::BTreeMap::from([(0, f.checked)]),
        );
    let proof = BorrowChecker::native_return_loan_proof(&f.program);
    let invoke = &f.program.functions[&invoke_id];
    let summary = &proof.lifetime[&invoke_id];
    assert_eq!(
        summary,
        &OriginSet::from([ReferenceOrigin::Param(Local(2))])
    );
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
    for purpose in [OriginPurpose::Lifetime, OriginPurpose::ValueLoans] {
        assert!(proof
            .sources
            .callable_result_for_purpose(
                invoke,
                &f.program.type_context,
                &f.program.backend_contract,
                indirect[0].0,
                indirect[0].1,
                indirect[0].2,
                purpose
            )
            .is_some());
    }
    let call = f.program.functions[&f.owner].basic_blocks.iter().filter_map(|block|
        block.terminator.as_ref()).find(|term| matches!(term,
            Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))), .. }
            if key == &f.invoke)).unwrap().clone();
    for (key, accepted) in [(f.identity.clone(), true), (f.bad.clone(), false)] {
        let mut call = call.clone();
        let Terminator::Call {
            args, target, span, ..
        } = &mut call
        else {
            unreachable!()
        };
        args[0] = Fixture::constant(&key);
        *target = BasicBlockId(1);
        *span = Some(f.span.clone());
        f.program.functions.get_mut(&f.owner).unwrap().basic_blocks = vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(call),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ];
        let mut diagnostics = Diagnostics::default();
        let proof = BorrowChecker::native_return_loan_proof(&f.program);
        validate(
            &f.program,
            &proof.lifetime,
            &proof.measured,
            Some(&proof.sources),
            &mut diagnostics,
        );
        assert_eq!(diagnostics.0.is_empty(), accepted, "{diagnostics:?}");
        if !accepted {
            f.assert_rejected(&diagnostics);
        }
    }
    // Removing the separate definition requirement keeps an identical type
    // header but restores the opaque path; no caller can certify this input.
    f.program
        .backend_contract
        .callable_input_obligations
        .clear();
    let proof = BorrowChecker::native_return_loan_proof(&f.program);
    assert!(!proof
        .sources
        .callable_results
        .inputs
        .contains_key(&invoke_id));
    assert!(proof.lifetime[&invoke_id].iter().any(|origin| matches!(
        origin,
        ReferenceOrigin::Local(_) | ReferenceOrigin::Temporary(_)
    )));
}

#[test]
fn callable_input_obligations_do_not_flatten_distinct_measured_loan_roles() {
    let f = fixture("binding_input_roles");
    let lifetime = BorrowChecker::reference_return_summaries(&f.program);
    let values = BorrowChecker::return_summaries_for_purpose(&f.program, OriginPurpose::ValueLoans);
    let analysis = Bindings {
        program: &f.program,
        owner: &f.program.functions[&f.owner],
        lifetime: &lifetime,
        values: &values,
        backend_valid: true,
        calls: None,
    };
    let actual = Evidence {
        ty: f.checked,
        sources: CallableResultBorrowing::Sources(Box::new(vec![
            CallableResultSource::ArgumentValue(0),
        ])),
        value_sources: CallableResultBorrowing::Sources(Box::default()),
        environment: None,
        storage: HashSet::new(),
        isolated: false,
        factory: None,
    };
    let mut diagnostics = Diagnostics::default();
    analysis.check(
        f.checked,
        Some(actual.clone()),
        Some(f.span.clone()),
        &mut diagnostics,
    );
    assert!(diagnostics.0.is_empty());
    analysis.check_input(
        f.checked,
        Some(actual),
        Some(f.span.clone()),
        &mut diagnostics,
    );
    f.assert_rejected(&diagnostics);
}

#[test]
fn indirect_call_argument_obligations_are_checked_without_a_result_contract_on_the_callee() {
    let mut f = fixture("binding_indirect_argument");
    let header = f.program.backend_contract.callable(&f.invoke).unwrap();
    let mut params = header
        .signature
        .params
        .iter()
        .map(|parameter| f.program.type_context.type_for(parameter.semantic_ty))
        .collect::<Vec<_>>();
    params[0] = f.program.type_context.type_for(f.checked);
    let callee_ty = f.program.type_context.intern_type(&Type::function(
        params,
        f.program
            .type_context
            .type_for(header.signature.ret.semantic_ty),
    ));
    let callee = f.local(callee_ty);
    let incoming = f.local(f.checked);
    let mut call = f.program.functions[&f.owner].basic_blocks.iter().filter_map(|block| block.terminator.as_ref()).find(|term|
        matches!(term, Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))), .. } if key == &f.invoke)
    ).unwrap().clone();
    let Terminator::Call {
        func,
        args,
        target,
        span,
        ..
    } = &mut call
    else {
        panic!();
    };
    *func = Operand::Copy(Place {
        local: callee,
        projection: vec![],
    });
    // The argument's header has the exact promise, but no incoming producer.
    args[0] = Operand::Copy(Place {
        local: incoming,
        projection: vec![],
    });
    *target = BasicBlockId(1);
    *span = Some(f.span.clone());
    f.program.functions.get_mut(&f.owner).unwrap().basic_blocks = vec![
        BasicBlock {
            statements: vec![],
            terminator: Some(call),
        },
        BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Return),
        },
    ];
    f.assert_rejected(&f.validate());
    // Remove the incoming local's own promise: the indirect-call parameter
    // obligation must still trigger validation, rather than the fast path skip.
    f.program.functions.get_mut(&f.owner).unwrap().local_decls[incoming.0].ty = f.unknown;
    f.assert_rejected(&f.validate());
}

#[test]
fn missing_signature_agreement_and_malformed_declared_envelopes_reject() {
    let mut f = fixture("binding_signature");
    for malformed in [false, true] {
        let mut ty = f.program.type_context.type_for(f.checked);
        let Type::Function {
            params,
            result_borrowing,
            ..
        } = &mut ty
        else {
            panic!();
        };
        if malformed {
            *result_borrowing = CallableResultBorrowing::Sources(Box::new(vec![
                    CallableResultSource::ArgumentValue(
                        0
                    );
                    2
                ]));
        } else {
            params.push(params[0].clone());
        }
        let ty = f.program.type_context.intern_type(&ty);
        let local = f.local(ty);
        f.program.functions.get_mut(&f.owner).unwrap().basic_blocks = vec![BasicBlock {
            statements: vec![f.assign(local, Fixture::constant(&f.identity))],
            terminator: Some(Terminator::Return),
        }];
        f.assert_rejected(&f.validate());
    }
}

#[test]
fn exposing_callable_storage_or_crossing_a_call_does_not_supply_empty_effects() {
    let mut f = fixture("binding_effects");
    let incoming = f.local(f.unknown);
    let result = f.local(f.checked);
    let reference_ty = f.program.type_context.intern_ty(Ty::Reference {
        mutable: false,
        inner: f.unknown,
    });
    let reference = f.local(reference_ty);
    let lifetime = BorrowChecker::reference_return_summaries(&f.program);
    let values = BorrowChecker::return_summaries_for_purpose(&f.program, OriginPurpose::ValueLoans);
    let analysis = Bindings {
        program: &f.program,
        owner: &f.program.functions[&f.owner],
        lifetime: &lifetime,
        values: &values,
        backend_valid: true,
        calls: None,
    };
    for through_call in [false, true] {
        let mut state = analysis.initial_state(analysis.owner);
        analysis.apply_statement(
            &mut state,
            &f.assign(incoming, Fixture::constant(&f.identity)),
        );
        assert!(state.0.as_ref().unwrap().contains_key(&Place {
            local: incoming,
            projection: vec![]
        }));
        if through_call {
            let call = analysis
                .owner
                .basic_blocks
                .iter()
                .find_map(|block| {
                    block
                        .terminator
                        .as_ref()
                        .filter(|term| matches!(term, Terminator::Call { .. }))
                })
                .unwrap();
            analysis.apply_terminator(&mut state, call);
        } else {
            analysis.apply_statement(
                &mut state,
                &StatementData::assign(
                    Place {
                        local: reference,
                        projection: vec![],
                    },
                    Rvalue::Ref(
                        Mutability::Not,
                        Place {
                            local: incoming,
                            projection: vec![],
                        },
                    ),
                    Some(f.span.clone()),
                ),
            );
        }
        let statement = f.assign(
            result,
            Operand::Copy(Place {
                local: incoming,
                projection: vec![],
            }),
        );
        let StatementKind::Assign(destination, value) = &statement.kind else {
            panic!();
        };
        let mut diagnostics = Diagnostics::default();
        analysis.check(
            f.checked,
            analysis.assigned(&state, destination, value),
            statement.span.clone(),
            &mut diagnostics,
        );
        f.assert_rejected(&diagnostics);
    }
}

#[test]
fn production_borrow_run_checks_the_original_native_argument_binding_and_span() {
    let mut f = fixture("binding_production_run");
    let invoke_id = MirCallable::Resolved(f.invoke.clone())
        .function_id()
        .unwrap();
    f.program
        .backend_contract
        .callables
        .get_mut(&f.invoke)
        .unwrap()
        .signature
        .params[0]
        .semantic_ty = f.checked;
    f.program.functions.get_mut(&invoke_id).unwrap().local_decls[1].ty = f.checked;
    for (key, accepted) in [(f.identity.clone(), true), (f.bad.clone(), false)] {
        let body = f.program.functions.get_mut(&f.owner).unwrap();
        let call = body.basic_blocks.iter_mut().filter_map(|block| block.terminator.as_mut()).find(|term|
            matches!(term, Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(key))), .. } if key == &f.invoke)
        ).unwrap();
        let Terminator::Call { args, span, .. } = call else {
            panic!();
        };
        args[0] = Fixture::constant(&key);
        let span = span.clone().unwrap();
        let diagnostics = BorrowChecker::run(&f.program).unwrap_err();
        // The original bad body remains independently rejected; it is never
        // executed or removed to make the binding checker look successful.
        assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message
            == "Cannot return reference to local value because it does not live long enough"));
        let bindings = diagnostics
            .0
            .iter()
            .filter(|diagnostic| {
                diagnostic.message == "Callable result borrowing contract is not satisfied"
            })
            .collect::<Vec<_>>();
        assert_eq!(bindings.len(), usize::from(!accepted), "{diagnostics:?}");
        if !accepted {
            assert_eq!(bindings[0].primary.as_ref().unwrap().span, span);
        }
    }
}
