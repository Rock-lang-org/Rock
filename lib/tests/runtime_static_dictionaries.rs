//! Unknown concrete Self is initialized by one receiver-free static dictionary entry.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::diagnostic::DiagnosticCode;
use rock_lib::mir::MirErasedType;
use rock_lib::products::{ProductRuntimeSupport, RuntimeRootSymbol};
use rock_lib::Config;

const PROVIDER: &str = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
< trait Factory
    make: I64 -> Self
< trait Dispatch
    @build: I64 -> T where T: Factory
< struct Service
    < marker: I64
impl Dispatch for Service
    @build: I64 -> T where T: Factory
    @build = seed -> T::make seed
as_dispatch: &Service -> &Dispatch
< as_dispatch = service ->
    object: &Dispatch = service
    object
"#;

const FACTORY: &str = r#"
< struct Audit
    < makes: I64
    < drops: I64
< struct LaterProduct
    < value: I64
    < audit: *Audit
impl Factory for LaterProduct
    make = seed ->
        audit = seed as *Audit
        unsafe (*audit).makes = (~I64Add ((*audit).makes), 1)
        LaterProduct
            value: 42
            audit: audit
impl Dispose for LaterProduct
    ~@dispose = !->
        unsafe (*self.audit).drops = (~I64Add ((*self.audit).drops), 1)
extract: LaterProduct -> I64
< extract = product -> product.value
"#;

const CLIENT: &str = r#"
main = ->
    mut audit = Audit
        makes: 0
        drops: 0
    service = Service
        marker: 0
    object = as_dispatch &service
    seed = ((&mut audit) as *Audit) as I64
    first_value = extract (object.build seed)
    second_value = extract (object.build seed)
    if ~I64Ne first_value, 42 then 1
    else if ~I64Ne second_value, 42 then 2
    else if ~I64Ne audit.makes, 2 then 3
    else if ~I64Ne audit.drops, 2 then 4
    else 42
"#;

fn selected_provider() -> String {
    PROVIDER.replace("< trait Factory", "< trait StaticMake\n    create: I64 -> Self\n< trait Factory")
        .replace("where T: Factory", "where T: StaticMake")
        .replace("< trait Dispatch", "impl Factory for T where T: StaticMake\n    make = seed -> T::create seed\nstatic_helper: I64 -> U where U: Factory\nstatic_helper = seed -> U::make seed\n< trait Dispatch")
        .replace("@build = seed -> T::make seed", "@build = seed -> static_helper seed")
}
fn factory(wide: bool, selected: bool) -> String {
    let source = if selected { FACTORY.replace("impl Factory", "impl StaticMake").replace("    make =", "    create =") }
        else { FACTORY.into() };
    if !wide { return source; }
    source.replace("< struct LaterProduct\n    < value", "< struct LaterProduct\n    < leading: (I64, I64)\n    < value")
        .replace("    < audit: *Audit", "    < audit: *Audit\n    < trailing: I64")
        .replace("        LaterProduct\n            value", "        LaterProduct\n            leading: (1, 2)\n            trailing: 3\n            value")
}

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn scratch() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures").join(format!("rock-static-dictionaries-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&root).unwrap(); root
}
fn config(root: &Path, entry: &Path, opt_level: u8) -> Config {
    let mut config = Config { entry_file: entry.into(), output_dir: root.into(), current_crate_name: Some("static_dispatch".into()),
        no_std: true, no_prelude: true, emit_llvm: true, opt_level, ..Default::default() };
    // One fixture-sized production trace per test process, opt-in only. Use an
    // exact test filter so the observed source/case is unambiguous.
    if std::env::var_os("ROCK_STATIC_TRACE").as_deref() == Some(std::ffi::OsStr::new("1")) {
        static TRACE: std::sync::Once = std::sync::Once::new();
        TRACE.call_once(|| {
            config.debug_print = vec![rock_lib::DebugPrint::Hir, rock_lib::DebugPrint::Mir, rock_lib::DebugPrint::Llvm];
            eprintln!("static production trace: entry={} output={} optimization={opt_level}", entry.display(), root.display());
        });
    }
    config
}
fn run(config: &Config) {
    rock_lib::compile(config).unwrap();
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        eprintln!("static execution: entry={} optimization={} executable={} code={:?} signal={:?} core_dumped={}",
            config.entry_file.display(), config.opt_level, config.output_dir.join("main").display(),
            output.status.code(), output.status.signal(), output.status.core_dumped());
    }
    assert_eq!(output.status.code(), Some(42), "actual static construction, two calls, two initialized non-Copy products and two drops: entry={} optimization={} executable={} {output:?}",
        config.entry_file.display(), config.opt_level, config.output_dir.join("main").display());
}
fn arity(line: &str) -> usize { line.split_once('(').unwrap().1.split_once(')').unwrap().0.split(',').count() }
fn provider_ir(ir: &str, selected: bool) {
    assert!(!ir.contains("LaterProduct"), "producer knows no later concrete Self");
    let bodies = ir.split("define ").skip(1).map(|body| body.split_once("\n}").unwrap().0).collect::<Vec<_>>();
    if !selected {
        let producer = bodies.iter().find(|body| body.starts_with("internal void @rock.erased.") && body.contains("%dictionary.entry = load ptr")).unwrap();
        assert_eq!(arity(producer.lines().next().unwrap()), 6, "out, Self/T descriptors, Factory dictionary, Dispatch receiver, seed");
        let call = producer.lines().find(|line| line.trim_start().starts_with("call void %dictionary.entry(")).unwrap();
        assert_eq!(arity(call), 3, "out, dictionary environment, seed: no artificial T receiver");
    } else {
        let adapter = bodies.iter().find(|body| body.starts_with("internal void @rock.dictionary.selected.")).unwrap();
        assert_eq!(arity(adapter.lines().next().unwrap()), 3);
        let entry = bodies.iter().find(|body| body.starts_with("internal void @rock.erased.selected.")).unwrap();
        assert_eq!(arity(entry.lines().next().unwrap()), 4, "out, captured T descriptor, captured StaticMake dictionary, seed");
        let call = entry.lines().find(|line| line.trim_start().starts_with("call void %dictionary.entry(")).unwrap();
        assert_eq!(arity(call), 3);
    }
    assert!(bodies.iter().filter(|body| body.starts_with("internal void @rock.erased.")).all(|body|
        body.lines().filter(|line| line.contains("call ")).all(|line| {
            let callee = line.split_once('(').unwrap().0.split_whitespace().last().unwrap();
            callee.starts_with('%') || callee.starts_with("@llvm.") || (selected && (callee.starts_with("@rock.erased.") || callee == "@rock_dictionary_selected"))
        })), "no native producer construction helper or later-Self bridge");
}

#[test]
fn static_dictionary_constructs_counted_noncopy_later_products_at_o0_o3() {
    let root = scratch(); let entry = root.join("main.rk");
    for wide in [false, true] {
        fs::write(&entry, format!("{PROVIDER}{}{CLIENT}", factory(wide, false))).unwrap();
        for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
    }
}

fn assert_no_local_runtime_factories(ir: &str, imported: &BTreeSet<ProductRuntimeSupport>) {
    let factories = [
        RuntimeRootSymbol::SelectedDictionaryFactoryV1,
        RuntimeRootSymbol::TupleDescriptorFactoryV1,
        RuntimeRootSymbol::NominalDescriptorFactoryV1,
    ].map(|root| format!("@{}", root.symbol()));
    let imported_symbols: BTreeSet<_> = imported.iter().flat_map(|support| match support {
        ProductRuntimeSupport::SelectedDictionariesV1 { roots, .. }
        | ProductRuntimeSupport::TupleDescriptorsV1 { roots, .. }
        | ProductRuntimeSupport::NominalDescriptorsV1 { roots, .. } => roots,
    }).map(|root| format!("@{}", root.symbol())).collect();
    for declaration in ir.lines().filter(|line| line.starts_with("declare ")) {
        let callee = declaration.split_once('(').unwrap().0.split_whitespace().last().unwrap();
        if factories.iter().any(|factory| factory == callee) {
            assert!(imported_symbols.contains(callee),
                "a runtime factory declaration requires the exact imported typed root: {declaration}");
        }
    }
    for definition in ir.split("define ").skip(1) {
        let (header, body) = definition.split_once('\n').unwrap();
        let body = body.split_once("\n}").unwrap().0;
        for line in body.lines().filter(|line| line.contains("call ")) {
            let call = line.split_once("call ").unwrap().1;
            let callee = call.split_once('(').unwrap().0.split_whitespace().last().unwrap();
            assert!(!factories.iter().any(|factory| factory == callee),
                "concrete later producer must add no local runtime-factory call: {header}\n{line}");
        }
    }
}

fn source_free(selected: bool) {
    let root = scratch(); let provider = root.join("provider.rk");
    let source = if selected { selected_provider() } else { PROVIDER.into() };
    fs::write(&provider, source).unwrap(); let object = root.join("provider.o");
    let producer = Config { no_link: true, emit_object: Some(object.clone()), ..config(&root, &provider, 0) };
    let mut products = rock_lib::compile_with_products(&producer).unwrap().products.unwrap();
    eprintln!("static provider typed runtime support: entry={} selected={selected} support={:?}", provider.display(), products.link.runtime_support);
    let slot = products.interface.object_abi.schemas.iter().flat_map(|schema| &schema.erased_slots).next().unwrap();
    assert_eq!(slot.signature.parameters.len(), 2); assert_eq!(slot.signature.params.len(), 2);
    assert_eq!(slot.signature.dictionaries.len(), 1); assert_eq!(slot.signature.ret, MirErasedType::Parameter(1));
    assert!(slot.signature.require_borrow_free.is_empty());
    let member = &slot.signature.dictionaries[0].members[0];
    assert!(member.receiver.is_none() && member.binder.parameters.is_empty() && member.binder.dictionaries.is_empty());
    assert_eq!(member.params, vec![MirErasedType::Concrete(rock_lib::types::Type::I64)]);
    assert_eq!(member.ret, slot.signature.dictionaries[0].subject);
    assert_eq!(member.borrowing.returns.len(), 1); assert_eq!(member.borrowing.returns[0].sources[0].argument, 0);
    assert_eq!(member.borrowing.returns[0].sources[0].kind, rock_lib::hir::borrowing::BorrowSourceKind::Value);
    assert!(member.borrowing.retentions.is_empty());
    if selected {
        assert!(products.link.runtime_support.iter().any(|support| matches!(support,
            ProductRuntimeSupport::SelectedDictionariesV1 { roots, .. } if roots == &std::collections::BTreeSet::from([RuntimeRootSymbol::SelectedDictionaryFactoryV1]))));
    } else { assert!(products.link.runtime_support.is_empty(), "declarations/native static leaves alone request no registry component"); }
    let dependency_runtime_support = products.link.runtime_support.clone();
    provider_ir(&fs::read_to_string(root.join("provider.ll")).unwrap(), selected);
    products.link.object_path = Some("provider.o".into()); let artifact = root.join("provider.rkca"); products.write_artifact_to_path(&artifact).unwrap();
    let object_before = fs::read(&object).unwrap(); let artifact_before = fs::read(&artifact).unwrap(); fs::remove_file(&provider).unwrap();
    for wide in [false, true] {
        let factory_source = root.join("factory.rk");
        let bound = if selected { "StaticMake" } else { "Factory" };
        fs::write(&factory_source, format!("> static_dispatch::Dispose\n> static_dispatch::{bound}\n{}", factory(wide, selected))).unwrap();
        let factory_object = root.join("factory.o");
        let factory_config = Config { current_crate_name: Some("later_factory".into()), no_link: true, emit_object: Some(factory_object.clone()),
            extern_artifacts: vec![("static_dispatch".into(), artifact.clone())], ..config(&root, &factory_source, 0) };
        let mut factory_products = rock_lib::compile_with_products(&factory_config).unwrap().products.unwrap();
        eprintln!("static factory typed runtime support before strict root assertion: entry={} selected={selected} dependency_support={:?} factory_support={:?}",
            factory_source.display(), dependency_runtime_support, factory_products.link.runtime_support);
        for implementation in factory_products.interface.impls.values() {
            eprintln!("static factory declared implementation: id={:?} generics={:?} receiver={:?} trait={:?} trait_args={:?}",
                implementation.id, implementation.type_generics, implementation.receiver_pattern, implementation.trait_id, implementation.trait_arg_types);
            for method in implementation.methods.values() {
                eprintln!("static factory declared method: id={:?} generics={:?} receiver={:?} params={:?} ret={:?} bounds={:?}",
                    method.id, method.generic_params, method.self_receiver, method.params, method.ret_type, method.generic_bounds);
            }
        }
        assert_eq!(factory_products.link.runtime_support, dependency_runtime_support,
            "a concrete static native leaf adds no runtime component, target or root beyond its object-backed dependency");
        let method = factory_products.interface.impls.values().flat_map(|implementation| implementation.methods.values())
            .find(|method| method.self_receiver.is_none() && method.params == vec![rock_lib::types::Type::I64]).unwrap();
        let method_id = rock_lib::products::ProductDefId::from(method.id);
        let record = &factory_products.link.records[&method_id];
        assert!(record.polymorphic_member.is_none(), "native concrete leaf is not a fabricated polymorphic entry");
        let symbol = record.backend_symbol.clone();
        let ir = fs::read_to_string(root.join("factory.ll")).unwrap();
        assert_no_local_runtime_factories(&ir, &dependency_runtime_support);
        let leaves = ir.split("define ").skip(1).filter(|body| body.lines().next().unwrap().contains(&format!("@{symbol}("))).collect::<Vec<_>>();
        assert_eq!(leaves.len(), 1, "ordinary concrete Self body emitted once in the later producer");
        assert_eq!(arity(leaves[0].lines().next().unwrap()), 1, "native method has only seed");
        assert!(!ir.contains("define void @rock.erased.member."));
        factory_products.link.object_path = Some("factory.o".into()); let factory_artifact = root.join("factory.rkca"); factory_products.write_artifact_to_path(&factory_artifact).unwrap();
        let factory_object_before = fs::read(&factory_object).unwrap(); let factory_artifact_before = fs::read(&factory_artifact).unwrap(); fs::remove_file(&factory_source).unwrap();
        let entry = root.join("main.rk");
        fs::write(&entry, format!("> static_dispatch::Service\n> static_dispatch::as_dispatch\n> later_factory::Audit\n> later_factory::extract\n{CLIENT}")).unwrap();
        for optimization in [0, 3] {
            let consumer = Config { current_crate_name: Some("static_later_client".into()),
                extern_artifacts: vec![("static_dispatch".into(), artifact.clone()), ("later_factory".into(), factory_artifact.clone())], ..config(&root, &entry, optimization) };
            run(&consumer);
            assert_eq!(fs::read(&object).unwrap(), object_before); assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
            assert_eq!(fs::read(&factory_object).unwrap(), factory_object_before); assert_eq!(fs::read(&factory_artifact).unwrap(), factory_artifact_before);
            let ir = fs::read_to_string(root.join("main.ll")).unwrap();
            assert!(ir.lines().any(|line| line.starts_with("declare ") && line.contains(&format!("@{symbol}("))));
            assert!(!ir.lines().any(|line| line.starts_with("define ") && line.contains(&format!("@{symbol}("))), "bodyless typed native leaf cannot be cloned");
            assert!(!ir.contains("dictionary.factory.status"), "source-free consumer cannot rebuild the generic Dispatch/selected Factory body");
        }
        let mut missing = factory_products.clone(); missing.link.records.remove(&method_id);
        let bad = root.join("missing-static-leaf.rkca"); missing.write_artifact_to_path(&bad).unwrap();
        let consumer = Config { current_crate_name: Some("missing_static_client".into()),
            extern_artifacts: vec![("static_dispatch".into(), artifact.clone()), ("later_factory".into(), bad)], ..config(&root, &entry, 0) };
        assert!(rock_lib::compile(&consumer).is_err(), "no fallback to clone a concrete object-provided static method lacking its link");
        assert_eq!(fs::read(&object).unwrap(), object_before); assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
        assert_eq!(fs::read(&factory_object).unwrap(), factory_object_before); assert_eq!(fs::read(&factory_artifact).unwrap(), factory_artifact_before);
    }
}

#[test]
fn source_free_static_dictionary_preserves_complete_dispatch_and_factory_bytes() { source_free(false); }

#[test]
fn selected_static_dictionary_executes_and_preserves_both_source_free_producers() {
    let root = scratch(); let entry = root.join("main.rk");
    for wide in [false, true] {
        fs::write(&entry, format!("{}{}{CLIENT}", selected_provider(), factory(wide, true))).unwrap();
        for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
    }
    source_free(true);
}

#[test]
fn static_native_leaf_cannot_return_a_local_reference_or_an_omitted_capture() {
    let root = scratch(); let entry = root.join("main.rk");
    let borrowed = r#"
struct LaterReference
    < value: &I64
impl Factory for LaterReference
    make = seed ->
        local = seed
        LaterReference
            value: &local
read: LaterReference -> I64
read = product -> *product.value
main = ->
    service = Service
        marker: 0
    object = as_dispatch &service
    read (object.build 42)
"#;
    fs::write(&entry, format!("{PROVIDER}{borrowed}")).unwrap();
    let diagnostics = rock_lib::compile(&config(&root, &entry, 0)).expect_err("a static leaf cannot return a reference into its seed/local frame");
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)), "actual Borrow-stage rejection: {diagnostics:?}");
    let captured = borrowed.replace("    < value: &I64", "    < value: () -> I64")
        .replace("        local = seed", "        local = seed\n        borrowed = &local")
        .replace("            value: &local", "            value: -> *borrowed")
        .replace("read = product -> *product.value", "read = product -> product.value!");
    fs::write(&entry, format!("{PROVIDER}{captured}")).unwrap();
    let mut capture_config = config(&root, &entry, 0);
    // Trace only this second safety negative, never the successful runtime
    // fixtures. MIR prints before Borrow checking; do not execute its output.
    if std::env::var_os("ROCK_STATIC_CAPTURE_TRACE").as_deref() == Some(std::ffi::OsStr::new("1")) {
        capture_config.debug_print = vec![rock_lib::DebugPrint::Hir, rock_lib::DebugPrint::Mir];
        eprintln!("static omitted-capture trace: entry={} output={} compile-only; executable must not be run",
            entry.display(), root.display());
    }
    let diagnostics = rock_lib::compile(&capture_config).expect_err("a static leaf cannot omit a borrowed closure capture in its output authority");
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)), "actual capture-lifetime rejection, not a static frontend blocker: {diagnostics:?}");
}

#[test]
fn static_dictionary_artifact_rejects_receiver_kind_scope_arity_result_and_borrow_forgery() {
    let root = scratch(); let provider = root.join("provider.rk"); fs::write(&provider, PROVIDER).unwrap();
    let producer = Config { no_link: true, emit_object: Some(root.join("provider.o")), ..config(&root, &provider, 0) };
    let products = rock_lib::compile_with_products(&producer).unwrap().products.unwrap();
    for control in 0..7 {
        let mut corrupt = products.clone();
        let slot = corrupt.interface.object_abi.schemas.iter_mut().flat_map(|schema| &mut schema.erased_slots).next().unwrap();
        let subject = slot.signature.dictionaries[0].subject.clone();
        let member = &mut slot.signature.dictionaries[0].members[0];
        match control {
            0 => member.receiver = Some(rock_lib::types::ReceiverMode::Shared),
            1 => member.params.push(subject),
            2 => member.ret = MirErasedType::Parameter(0),
            3 => member.ret = MirErasedType::MemberParameter(0),
            4 => member.borrowing.returns.clear(),
            5 => member.borrowing.returns[0].sources[0].kind = rock_lib::hir::borrowing::BorrowSourceKind::Contents,
            _ => slot.signature.parameters[1].kind = rock_lib::type_services::kind::Kind::arrow(rock_lib::type_services::kind::Kind::Type, rock_lib::type_services::kind::Kind::Type),
        }
        assert!(corrupt.write_artifact_to_path(&root.join(format!("bad-static-{control}.rkca"))).is_err(), "static artifact control {control}");
    }
}
