use super::allocator_trace_tests::mutation_source_program;
use super::*;
use crate::hir::borrowing::{NativeReturnOrigins, NativeReturnSource};

#[test]
fn measured_scalar_external_dependencies_stay_separate_from_storage_loan_summaries() {
    let source = "extern abs: I32 -> I32\nchecked: I32 -> I32\nchecked = value -> abs value\nmain = -> checked 42\n";
    let (program, _) = mutation_source_program("native_scalar_external", source, false);
    BorrowChecker::run(&program).unwrap();
    let portable = BorrowChecker::checked_native_return_origins(&program);
    let (id, origins) = portable
        .iter()
        .find(|(_, origins)| {
            origins.params.len() == 1
                && origins
                    .sources
                    .contains(&NativeReturnSource::UnknownExternal)
        })
        .unwrap();
    assert_eq!(
        origins.sources,
        vec![
            NativeReturnSource::ArgumentValue(0),
            NativeReturnSource::UnknownExternal
        ]
    );
    let proof = BorrowChecker::native_return_loan_proof(&program);
    let expected = OriginSet::from([
        ReferenceOrigin::Param(Local(1)),
        ReferenceOrigin::UnknownExternal,
    ]);
    assert_eq!(
        proof
            .sources
            .scalar_dependencies(OriginPurpose::Lifetime, id),
        Some(&expected)
    );
    assert_eq!(
        proof
            .sources
            .scalar_dependencies(OriginPurpose::ValueLoans, id),
        Some(&expected)
    );
    assert!(!proof.lifetime.contains_key(id));
    assert!(!proof.measured.contains_key(id));
    assert!(!proof.summaries.contains_key(id));
}

#[test]
fn checked_native_return_contract_distinguishes_copied_contents_from_slot_borrow() {
    let source = r#"
struct Cursor
    < text: &Str
    < offset: I64
new: &Str -> Cursor
new = text -> Cursor
    text: text
    offset: 0
copied: & &Str -> Cursor
copied = metadata -> new *metadata
slot: & &Str -> & &Str
slot = metadata -> metadata
main = !->
    text = "é🦀"
    cursor = copied &text
    held = slot &text
"#;
    let (program, entry) = mutation_source_program("native_return_contract_sources", source, false);
    BorrowChecker::run(&program).unwrap();
    let contracts = BorrowChecker::checked_native_return_origins(&program);
    for (name, expected) in [
        ("copied", NativeReturnSource::ArgumentContents(0)),
        ("slot", NativeReturnSource::ArgumentValue(0)),
    ] {
        let start = source.find(&format!("{name} =")).unwrap();
        let functions = program
            .functions()
            .filter(|(_, body)| {
                body.local_decls.iter().any(|local| {
                    local.span.as_ref().is_some_and(|span| {
                        span.file_path == entry
                            && span.start >= start
                            && span.start
                                < source[start..].find('\n').map(|end| start + end).unwrap()
                    })
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(functions.len(), 1);
        let (id, body) = functions[0];
        let contract = contracts
            .get(id)
            .expect("completed exact native body return proof");
        assert_eq!(contract.sources, vec![expected]);
        assert_eq!(contract.params.len(), body.arg_count);
        assert_eq!(contract.ret, body.ret_type);
    }
}

#[test]
fn native_return_contract_rejects_malformed_sources_without_empty_proof() {
    let mut contract = NativeReturnOrigins {
        params: vec![Type::I64],
        ret: Type::I64,
        sources: vec![NativeReturnSource::ArgumentValue(0)],
    };
    let reference = |ty: &Type| matches!(ty, Type::Reference { .. } | Type::Pointer(_));
    assert!(contract.sources_are_valid(reference));
    for sources in [
        vec![],
        vec![NativeReturnSource::ArgumentValue(1)],
        vec![NativeReturnSource::ArgumentContents(0)],
        vec![
            NativeReturnSource::ArgumentValue(0),
            NativeReturnSource::ArgumentValue(0),
        ],
    ] {
        contract.sources = sources;
        assert!(!contract.sources_are_valid(reference));
    }
}

#[test]
fn native_return_contract_utf8_producer_observation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let config = crate::Config {
        entry_file: root.join("stdlib/lib.rk"),
        output_dir: root.join(".agent-work/fixtures/native-utf8-producer"),
        current_crate_name: Some("stdlib".into()),
        no_std: true,
        no_link: true,
        ..Default::default()
    };
    let frontend = crate::compile_frontend(&config).unwrap();
    let current = super::allocator_trace_tests::current_stdlib_artifact();
    let products = crate::products::CompilerProducts::read_artifact_from_path(&current).unwrap();
    let mut registration_roots = 0;
    let mut registration_locals = 0;
    for method in frontend
        .analysis
        .hir
        .program
        .impls
        .values()
        .flat_map(|implementation| implementation.methods.values())
    {
        let Some(definition) = frontend.analysis.hir.source_map.definition_span(method.id) else {
            continue;
        };
        if !((definition.file_path == root.join("stdlib/option.rk") && method.name == "bind")
            || (definition.file_path == root.join("stdlib/vec.rk") && method.name == "traverse_m"))
        {
            continue;
        }
        registration_roots += 1;
        assert!(registration_roots <= 2 && method.params.len() <= 8);
        let retained = products
            .bodies
            .generic_impls
            .values()
            .flat_map(|implementation| implementation.methods.values())
            .find(|retained| retained.id == method.id)
            .expect("actual producer definition retained in guarded artifact");
        assert_eq!(
            retained
                .params
                .iter()
                .map(|param| param.local_id)
                .collect::<Vec<_>>(),
            method
                .params
                .iter()
                .map(|param| param.local_id)
                .collect::<Vec<_>>()
        );
        eprintln!("HKT_SOURCE_REGISTRATION producerDefinition={:?} producerDefinitionSpan={definition:?} actualArtifactBodyDefinition={:?} parameterIdsPreserved=true", method.id, retained.id);
        for parameter in &method.params {
            registration_locals += 1;
            assert!(registration_locals <= 4);
            let span = frontend
                .analysis
                .hir
                .source_map
                .local_span(method.id, parameter.local_id);
            eprintln!("HKT_SOURCE_REGISTRATION producerDefinition={:?} actualParameter={:?} producerLocalSpan={span:?} artifactParameterHasSpanField=false; no fallback or invented transport authority", method.id, parameter.local_id);
            assert!(span.is_some(), "producer source registration loss must remain a measured boundary, not assumed artifact loss");
        }
    }
    assert_eq!((registration_roots, registration_locals), (2, 4));
    let selected = frontend
        .analysis
        .hir
        .program
        .impls
        .values()
        .flat_map(|implementation| implementation.methods.values())
        .filter(|method| {
            ["chars", "char_indices", "new"].contains(&method.name.as_str())
                && frontend
                    .analysis
                    .hir
                    .source_map
                    .definition_span(method.id)
                    .is_some_and(|span| span.file_path == root.join("stdlib/string.rk"))
        })
        .map(|method| method.id)
        .collect::<HashSet<_>>();
    assert!(selected.len() <= 8 && !selected.is_empty());
    let fingerprint = crate::product_source_fingerprint(
        &config,
        &frontend.source_db,
        &frontend.fingerprint_source_files,
    );
    let identity = crate::products::ProductCrateIdentity::local("stdlib".into())
        .with_source_fingerprint(fingerprint);
    let context = crate::mono::CompilationIdentityContext::new(
        frontend.analysis.hir.root_crate_id,
        identity,
        Default::default(),
    );
    let mut mono =
        crate::mono::monomorphize_with_crates(frontend.analysis.hir, &frontend.crate_ctx, context)
            .unwrap();
    let bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut mono);
    let program =
        crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies);
    let summaries =
        BorrowChecker::return_summaries_for_purpose(&program, OriginPurpose::ValueLoans);
    let checked = BorrowChecker::checked_native_return_origins(&program);
    let mut observed = 0;
    let mut certified = 0;
    for (id, body) in program.functions() {
        let Some(key) = program.backend_contract.function_bodies.get(id) else {
            continue;
        };
        let Some(declaration) = program.backend_contract.callable(key) else {
            continue;
        };
        if !declaration
            .source_def_id
            .is_some_and(|definition| selected.contains(&definition))
        {
            continue;
        }
        observed += 1;
        if checked.contains_key(id) {
            certified += 1;
        }
        assert!(observed <= 8 && body.local_decls.len() <= 64 && body.basic_blocks.len() <= 16);
        eprintln!("NATIVE_PRODUCER definition={:?} id={id:?} available={} summary={:?} checked={:?} header={declaration:?}", declaration.source_def_id,
            BorrowChecker::body_available_for_purpose(&program, id, body, OriginPurpose::ValueLoans), summaries.get(id), checked.get(id));
        for (block, data) in body.basic_blocks.iter().enumerate() {
            assert!(data.statements.len() <= 64);
            for statement in &data.statements {
                eprintln!("NATIVE_PRODUCER id={id:?} block={block} statement={statement:?}");
            }
            eprintln!(
                "NATIVE_PRODUCER id={id:?} block={block} terminator={:?}",
                data.terminator
            );
        }
    }
    assert_eq!((observed, certified), (6, 4));
}

#[test]
fn native_return_contract_wrapper_resolution_requires_actual_allocation_and_body() {
    let source = r#"
struct Cursor
    < text: &Str
impl Cursor
    new: &Str -> Cursor
    new = text -> Cursor
        text: text
copied: & &Str -> Cursor
copied = metadata -> Cursor::new *metadata
main = !->
    cursor = copied &"é🦀"
"#;
    let (program, _) = mutation_source_program("native_wrapper_exact", source, false);
    BorrowChecker::run(&program).unwrap();
    let resolved = call_targets::resolved_bodies(&program);
    assert_eq!(resolved.len(), 1);
    let id = resolved.keys().next().unwrap().clone();
    let body = program.functions.get(&id).unwrap();
    let closure = body
        .basic_blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            StatementKind::Assign(_, Rvalue::Closure(closure)) => Some(closure),
            _ => None,
        })
        .unwrap()
        .clone();
    let target = MirFunctionId::Closure(Box::new(closure.id.clone()));
    let summary = BorrowChecker::checked_native_return_origins(&program);
    assert_eq!(
        summary.get(&id).unwrap().sources,
        vec![NativeReturnSource::ArgumentContents(0)]
    );
    for mutation in 0..5 {
        let mut altered = program.clone();
        match mutation {
            0 => {
                altered.functions.remove(&target);
            }
            1 => {
                altered
                    .functions
                    .get_mut(&target)
                    .unwrap()
                    .closure_captures
                    .push(crate::mir::MirClosureCapture {
                        name: "unwitnessed".into(),
                        span: None,
                        local: Local(1),
                        kind: crate::mir::MirClosureCaptureKind::ByRef,
                    });
            }
            2 => {
                altered.backend_contract.function_bodies.remove(&target);
            }
            3 => {
                for block in &mut altered.functions.get_mut(&id).unwrap().basic_blocks {
                    for statement in &mut block.statements {
                        if let StatementKind::Assign(_, Rvalue::Closure(closure)) =
                            &mut statement.kind
                        {
                            closure.id.owner = target.clone();
                        }
                    }
                }
            }
            4 => {
                let body = altered.functions.get_mut(&id).unwrap();
                let block = body
                    .basic_blocks
                    .iter_mut()
                    .find(|block| {
                        matches!(
                            block.terminator,
                            Some(Terminator::Call {
                                func: Operand::Copy(_) | Operand::Move(_),
                                ..
                            })
                        )
                    })
                    .unwrap();
                block.statements.push(StatementData::new(
                    StatementKind::Assign(
                        Place {
                            local: Local(1),
                            projection: vec![Projection::Deref],
                        },
                        Rvalue::Use(Operand::Constant(Constant::Unit)),
                    ),
                    None,
                ));
            }
            _ => unreachable!(),
        }
        assert!(
            !call_targets::resolved_bodies(&altered).contains_key(&id),
            "allocation/body mutation {mutation}"
        );
        assert!(
            call_targets::witnessed_calls(&altered)
                .keys()
                .all(|(owner, _)| owner != &id),
            "allocation/body mutation {mutation} cannot become an invocation target witness"
        );
        assert!(
            !BorrowChecker::checked_native_return_origins(&altered).contains_key(&id),
            "authority mutation {mutation}"
        );
    }
}

#[test]
fn capture_free_call_witnesses_preserve_type_metadata_but_not_opaque_effects_or_repeated_sites() {
    for (name, argument, expected) in [
        ("metadata_target", "(~SizeOf 42)", 1),
        ("opaque_target", "observe!", 0),
    ] {
        let source = format!("extern observe: () -> I64\nstruct Owner\n    < value: I64\nimpl Owner\n    new = value -> Owner\n        value: value\nmain = !->\n    owner = Owner::new {argument}\n");
        let (program, _) = mutation_source_program(name, &source, false);
        let calls = call_targets::witnessed_calls(&program);
        assert_eq!(
            calls.len(),
            expected,
            "metadata instructions cannot modify callable storage; foreign calls stay opaque"
        );
        for ((id, destination), witness) in calls {
            let body = program.functions.get(&id).unwrap();
            let term = body
                .basic_blocks
                .iter()
                .find_map(|block| match &block.terminator {
                    Some(Terminator::Call {
                        func,
                        destination: actual,
                        ..
                    }) if actual == &destination => Some(func),
                    _ => None,
                })
                .unwrap();
            assert!(
                matches!(term, Operand::Copy(_) | Operand::Move(_)),
                "the runtime call must remain indirect"
            );
            assert!(witness.target(term).is_some());
            let mut repeated = program.clone();
            let extra = body
                .basic_blocks
                .iter()
                .find(|block| {
                    matches!(&block.terminator,
                Some(Terminator::Call { destination: actual, .. }) if actual == &destination)
                })
                .unwrap()
                .clone();
            repeated
                .functions
                .get_mut(&id)
                .unwrap()
                .basic_blocks
                .push(extra);
            assert!(crate::mir::agreement::check_mir_runtime_agreement(&repeated).is_clean());
            assert!(
                !call_targets::witnessed_calls(&repeated).contains_key(&(id, destination)),
                "a repeated destination cannot identify one measured call occurrence"
            );
        }
    }
}

#[test]
fn native_return_contract_wrapper_resolution_keeps_opaque_capturing_and_alternative_calls_unknown()
{
    let source = r#"
struct Cursor
    < text: &Str
impl Cursor
    new: &Str -> Cursor
    new = text -> Cursor
        text: text
opaque: & &Str -> (&Str -> Cursor) -> Cursor
opaque = metadata, callback -> callback *metadata
capturing: & &Str -> Cursor
capturing = metadata ->
    callback: &Str -> Cursor = text -> Cursor
        text: *metadata
    callback *metadata
alternative: & &Str -> Bool -> Cursor
alternative = metadata, flag ->
    callback = if flag then Cursor::new else Cursor::new
    callback *metadata
main = !->
    text = "é🦀"
    a = opaque &text, Cursor::new
    b = capturing &text
    c = alternative &text, true
"#;
    let (program, entry) = mutation_source_program("native_wrapper_unknown", source, false);
    let resolved = call_targets::resolved_bodies(&program);
    let contracts = BorrowChecker::checked_native_return_origins(&program);
    for name in ["opaque", "capturing", "alternative"] {
        let start = source.find(&format!("{name} =")).unwrap();
        let end = start + source[start..].find('\n').unwrap();
        let functions = program
            .functions()
            .filter(|(_, body)| {
                matches!(
                    program.type_context.type_for(body.ret_type),
                    Type::Struct { .. }
                ) && body.local_decls.iter().any(|local| {
                    local.span.as_ref().is_some_and(|span| {
                        span.file_path == entry && span.start >= start && span.start < end
                    })
                })
            })
            .collect::<Vec<_>>();
        assert!(
            !functions.is_empty() && functions.len() <= 2,
            "{name}: every actual invocation body, including curry clones"
        );
        for (id, _) in functions {
            assert!(
                !resolved.contains_key(id),
                "{name} {id:?} has no allocation-complete must-target"
            );
            assert!(
                !contracts.contains_key(id),
                "{name} {id:?} has no checked portable result authority"
            );
        }
    }
}
