//! Bounded source-to-MIR receipt for static UTF-8 cursor transport; never authority.

use std::collections::HashSet;
use std::path::PathBuf;

use super::allocator_trace_tests::{
    agent_work_path, integration_literal, integration_sha, mutation_source_program,
    GUARDED_STDLIB_SHA,
};
use super::*;

#[test]
fn utf8_cursor_static_receiver_source_observation() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/integration.rs");
    let fixture_bytes = std::fs::read(&fixture).unwrap();
    let source = integration_literal(&fixture, "fn test_fold_loop_utf8_cursor_resume_and_empty()");
    let artifact = agent_work_path("artifacts/stdlib58/contracts-1/stdlib.rkca");
    let artifact_bytes = std::fs::read(&artifact).unwrap();
    assert_eq!(artifact_bytes.len(), 927978);
    assert_eq!(integration_sha(&artifact), GUARDED_STDLIB_SHA);
    let (program, entry) = mutation_source_program("utf8_cursor_static_receiver", &source, true);
    assert_eq!(
        integration_sha(&entry),
        "78c14a2a854a08bd0a76f80c93af2ffe3284f3d1b52c2ee4753fecd8c5300d76"
    );
    BorrowChecker::run(&program).unwrap();
    let spans = HashSet::from([(28, 36), (127, 135), (392, 394)]);
    let tc = &program.type_context;
    let contract = &program.backend_contract;
    let summaries = BorrowChecker::reference_return_summaries(&program);
    let sources = return_sources::NativeReturnSources::for_program(&program);
    let mut roots = HashSet::new();
    let mut sites = 0;
    let mut proven_calls = 0;
    for (_, function) in program.functions() {
        let selected = function
            .local_decls
            .iter()
            .enumerate()
            .filter_map(|(index, decl)| {
                (decl.source == LocalSource::Temporary
                    && decl.span.as_ref().is_some_and(|span| {
                        span.file_path == entry && spans.contains(&(span.start, span.end))
                    }))
                .then_some(Local(index))
            })
            .collect::<HashSet<_>>();
        if selected.is_empty() {
            continue;
        }
        roots.insert(function.id.clone());
        let analysis = ReferenceEscapeAnalysis::new(
            function,
            tc,
            contract,
            &summaries,
            MissingSummaryMode::Conservative,
        )
        .with_return_sources(Some(&sources));
        let results = run_fixpoint(&analysis, function);
        for (block, data) in function.basic_blocks.iter().enumerate() {
            if !results.reachable[block] {
                continue;
            }
            let mut state = results.entry_sets[block].clone();
            for statement in &data.statements {
                analysis.apply_statement(&mut state, statement);
                if let StatementKind::Assign(destination, value) = &statement.kind {
                    if selected.contains(&destination.local) {
                        sites += 1;
                        eprintln!("UTF8_SOURCE receiver owner={:?} block={block} destination={destination:?} type={:?} value={value:?} actualOrigins={:?}",
                            function.id, tc.type_for(function.local_decls[destination.local.0].ty), state.origins.get(destination));
                    }
                }
            }
            let Some(Terminator::Call {
                func,
                args,
                destination,
                ..
            }) = &data.terminator
            else {
                continue;
            };
            if !args.iter().any(|arg| matches!(arg, Operand::Copy(place) | Operand::Move(place) if selected.contains(&place.local))) { continue; }
            sites += 1;
            assert_eq!(
                analysis.call_return_origins(&state, func, args, destination),
                HashSet::from([ReferenceOrigin::Static])
            );
            proven_calls += 1;
            assert!(
                sites <= 16 && roots.len() <= 8 && args.len() <= 8,
                "UTF8_SOURCE scope caps8roots/16sites/8args; no truncation"
            );
            let target = match func {
                Operand::Constant(Constant::Callable(callable)) => callable.function_id(),
                _ => None,
            };
            eprintln!("UTF8_SOURCE call owner={:?} block={block} actualFunc={func:?} args={args:?} destination={destination:?} returnedOrigins={:?} summary={:?}",
                function.id, analysis.call_return_origins(&state, func, args, destination), target.as_ref().and_then(|id| summaries.get(id)));
            if let Some(target) = target {
                roots.insert(target.clone());
                let Some((_, body)) = program.functions().find(|(id, _)| **id == target) else {
                    let declaration = match func {
                        Operand::Constant(Constant::Callable(MirCallable::Resolved(key))) => {
                            contract.callable(key)
                        }
                        _ => None,
                    };
                    eprintln!("UTF8_SOURCE bodylessTarget={target:?} canonicalDeclaration={declaration:?} sourceSummary=checked-imported; no caller body/effect precision fabricated");
                    continue;
                };
                assert!(
                    body.local_decls.len() <= 64
                        && body
                            .basic_blocks
                            .iter()
                            .map(|block| block.statements.len())
                            .sum::<usize>()
                            <= 64,
                    "UTF8_SOURCE selected method body caps64locals/64statements"
                );
                eprintln!(
                    "UTF8_SOURCE selectedBody={target:?} argCount={} ret={:?}",
                    body.arg_count,
                    tc.type_for(body.ret_type)
                );
                for (block, data) in body.basic_blocks.iter().enumerate() {
                    for statement in &data.statements {
                        eprintln!("UTF8_SOURCE selectedBody={target:?} block={block} statement={statement:?}");
                    }
                    eprintln!(
                        "UTF8_SOURCE selectedBody={target:?} block={block} terminator={:?}",
                        data.terminator
                    );
                }
            }
        }
    }
    assert!(sites > 0 && sites <= 16 && roots.len() <= 8);
    assert_eq!(proven_calls, 3);
    assert_eq!(std::fs::read(&entry).unwrap(), source.as_bytes());
    assert_eq!(std::fs::read(&fixture).unwrap(), fixture_bytes);
    assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
    eprintln!("UTF8_SOURCE totals roots={} sites={sites} checkedStaticCalls={proven_calls} exactDiagnostics=0 execution=NONE; checked native transport, no mutation/callback authority", roots.len());
}
