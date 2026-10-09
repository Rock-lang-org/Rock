//! One Forward member entry serves future U layouts, including input loans.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::mir::MirErasedType;
use rock_lib::Config;

const PROVIDER: &str = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
< trait Forward
    @forward: U -> U
< trait Dispatch
    @apply: M -> U -> U where M: Forward
< struct Service
    < marker: I64
impl Dispatch for Service
    @apply = maker, value -> maker.forward value
as_dispatch: &Service -> &Dispatch
< as_dispatch = service ->
    object: &Dispatch = service
    object
"#;

const MAKER: &str = r#"
< struct Maker
    < reads: *I64
note: &Maker -> ()
note = maker !->
    unsafe *maker.reads = ~I64Add (*maker.reads), 1
impl Forward for Maker
    @forward = value ->
        note self
        value
"#;

const CLIENT: &str = r#"
struct Later
    < value: I64
    < drops: *I64
impl Dispose for Later
    ~@dispose = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
extract: Later -> I64
extract = result -> result.value
main = ->
    mut drops = 0
    mut reads = 0
    service = Service
        marker: 0
    object = as_dispatch &service
    reads_pointer = (&mut reads) as *I64
    drops_pointer = (&mut drops) as *I64
    first_maker = Maker
        reads: reads_pointer
    second_maker = Maker
        reads: reads_pointer
    first = Later
        value: 42
        drops: drops_pointer
    second = Later
        value: 42
        drops: drops_pointer
    first_result = object.apply first_maker, first
    second_result = object.apply second_maker, second
    first_value = extract first_result
    second_value = extract second_result
    if ~I64Ne first_value, 42 then 1
    else if ~I64Ne second_value, 42 then 2
    else if ~I64Ne reads, 2 then 3
    else if ~I64Ne drops, 2 then 4
    else 42
"#;

const BORROW_CLIENT: &str = r#"
main = ->
    mut reads = 0
    service = Service
        marker: 0
    object = as_dispatch &service
    maker = Maker
        reads: (&mut reads) as *I64
    mut value = 42
    result = object.apply maker, &value
    actual = *result
    if ~I64Ne actual, 42 then 1
    else if ~I64Ne reads, 1 then 2
    else 42
"#;

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn scratch() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures").join(format!("rock-polymorphic-dictionaries-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&root).unwrap(); root
}
fn config(root: &Path, entry: &Path, opt_level: u8) -> Config {
    Config { entry_file: entry.into(), output_dir: root.into(), current_crate_name: Some("poly_dispatch".into()),
        no_std: true, no_prelude: true, emit_llvm: true, opt_level, ..Default::default() }
}
fn maker(wide: bool) -> String {
    if !wide { return MAKER.into(); }
    MAKER.replace("< struct Maker\n    < reads", "< struct Maker\n    < leading: (I64, I64)\n    < reads")
        .replace("    < reads: *I64", "    < reads: *I64\n    < trailing: I64")
}
fn client(wide: bool) -> String {
    if !wide { return CLIENT.into(); }
    CLIENT.replace("struct Later\n    < value", "struct Later\n    < leading: (I64, I64)\n    < value")
        .replace("    < drops: *I64", "    < drops: *I64\n    < trailing: I64")
        .replace("first_maker = Maker\n        reads", "first_maker = Maker\n        leading: (1, 2)\n        trailing: 3\n        reads")
        .replace("second_maker = Maker\n        reads", "second_maker = Maker\n        leading: (4, 5)\n        trailing: 6\n        reads")
        .replace("first = Later\n        value", "first = Later\n        leading: (7, 8)\n        trailing: 9\n        value")
        .replace("second = Later\n        value", "second = Later\n        leading: (10, 11)\n        trailing: 12\n        value")
}
fn run(config: &Config) {
    rock_lib::compile(config).unwrap();
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert_eq!(output.status.code(), Some(42), "actual generic forwarding, initialized whole result, exact calls/drops: {output:?}");
}
fn provider_ir(ir: &str) {
    assert!(!ir.contains("Maker") && !ir.contains("Later"), "no future constructor or payload in producer");
    let bodies = ir.split("define ").skip(1).map(|body| body.split_once("\n}").unwrap().0).collect::<Vec<_>>();
    let body = bodies.iter().find(|body| body.starts_with("internal void @rock.erased.") && body.contains("%dictionary.entry = load ptr")).unwrap();
    assert_eq!(body.lines().next().unwrap().split_once('(').unwrap().1.split_once(')').unwrap().0.split(',').count(), 8,
        "out/Self,M,U descriptors/single Forward dictionary/receiver/maker/value");
    let call = body.lines().find(|line| line.trim_start().starts_with("call void %dictionary.entry(")).unwrap();
    assert_eq!(call.split_once('(').unwrap().1.split_once(')').unwrap().0.split(',').count(), 5,
        "one genuinely polymorphic member slot, actual U evidence is per call");
    assert!(body.lines().filter(|line| line.contains("call ")).all(|line| {
        let callee = line.split_once('(').unwrap().0.split_whitespace().last().unwrap();
        callee.starts_with('%') || callee.starts_with("@llvm.")
    }), "no native helper forwarding U");
}

#[test]
fn polymorphic_dictionary_forwards_noncopy_later_values_in_two_layouts_at_o0_o3() {
    let root = scratch(); let entry = root.join("main.rk");
    for wide in [false, true] {
        fs::write(&entry, format!("{PROVIDER}{}{}", maker(wide), client(wide))).unwrap();
        for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
    }
}

#[test]
fn source_free_polymorphic_dictionary_reuses_unchanged_dispatch_and_maker_objects() {
    let root = scratch(); let provider = root.join("provider.rk"); fs::write(&provider, PROVIDER).unwrap();
    let object = root.join("provider.o");
    let producer = Config { no_link: true, emit_object: Some(object.clone()), ..config(&root, &provider, 0) };
    let mut products = rock_lib::compile_with_products(&producer).unwrap().products.unwrap();
    assert!(products.link.runtime_support.is_empty(), "plain nested member calls do not request factories");
    let slots = products.interface.object_abi.schemas.iter().flat_map(|schema| &schema.erased_slots).collect::<Vec<_>>();
    assert_eq!(slots.len(), 1); assert_eq!(slots[0].signature.dictionaries.len(), 1);
    let member = &slots[0].signature.dictionaries[0].members[0];
    assert_eq!(member.binder.parameters.len(), 1); assert!(member.binder.dictionaries.is_empty());
    assert_eq!(member.params[1], MirErasedType::MemberParameter(0)); assert_eq!(member.ret, MirErasedType::MemberParameter(0));
    assert!(!member.borrowing.returns.is_empty()); assert!(member.borrowing.retentions.is_empty());
    provider_ir(&fs::read_to_string(root.join("provider.ll")).unwrap());
    products.link.object_path = Some("provider.o".into()); let artifact = root.join("provider.rkca"); products.write_artifact_to_path(&artifact).unwrap();
    let object_before = fs::read(&object).unwrap(); let artifact_before = fs::read(&artifact).unwrap(); fs::remove_file(&provider).unwrap();
    for wide in [false, true] {
        let maker_source = root.join("maker.rk"); fs::write(&maker_source, format!("> poly_dispatch::Forward\n{}", maker(wide))).unwrap();
        let maker_object = root.join("maker.o");
        let maker_config = Config { current_crate_name: Some("later_maker".into()), no_link: true, emit_object: Some(maker_object.clone()),
            extern_artifacts: vec![("poly_dispatch".into(), artifact.clone())], ..config(&root, &maker_source, 0) };
        let mut maker_products = rock_lib::compile_with_products(&maker_config).unwrap().products.unwrap();
        let maker_ir = fs::read_to_string(root.join("maker.ll")).unwrap();
        assert!(!maker_ir.contains("Later"));
        let entries = maker_ir.split("define ").skip(1).filter(|body| body.starts_with("void @rock.erased.member.")).collect::<Vec<_>>();
        assert_eq!(entries.len(), 1, "Maker producer emits exactly one generic U body");
        let member_body = entries[0].split_once("\n}").unwrap().0;
        assert!(member_body.lines().any(|line| line.contains("call void @llvm.memmove.") && line.contains("ptr align 1 %0,"))
            && member_body.lines().any(|line| line.contains("getelementptr") && line.contains("ptr %1,")),
            "the genuine body uses its actual U descriptor and initializes caller-provided whole-value output: {member_body}");
        assert!(maker_products.link.records.values().any(|record| record.backend_symbol.starts_with("rock.erased.member.")), "canonical generic method link record");
        maker_products.link.object_path = Some("maker.o".into()); let maker_artifact = root.join("maker.rkca"); maker_products.write_artifact_to_path(&maker_artifact).unwrap();
        let maker_object_before = fs::read(&maker_object).unwrap(); let maker_artifact_before = fs::read(&maker_artifact).unwrap(); fs::remove_file(&maker_source).unwrap();
        let entry = root.join("main.rk");
        fs::write(&entry, format!("> poly_dispatch::Dispose\n> poly_dispatch::Service\n> poly_dispatch::as_dispatch\n> later_maker::Maker\n{}", client(wide))).unwrap();
        for optimization in [0, 3] {
            let consumer = Config { current_crate_name: Some("later_payload".into()),
                extern_artifacts: vec![("poly_dispatch".into(), artifact.clone()), ("later_maker".into(), maker_artifact.clone())], ..config(&root, &entry, optimization) };
            run(&consumer);
            assert_eq!(fs::read(&object).unwrap(), object_before); assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
            assert_eq!(fs::read(&maker_object).unwrap(), maker_object_before); assert_eq!(fs::read(&maker_artifact).unwrap(), maker_artifact_before);
            let ir = fs::read_to_string(root.join("main.ll")).unwrap();
            assert!(ir.contains("declare void @rock.erased.member."));
            assert!(!ir.contains("define void @rock.erased.member."), "consumer cannot clone or U-specialize the maker body");
        }
        let borrowed = if wide { BORROW_CLIENT.replace("maker = Maker\n        reads", "maker = Maker\n        leading: (1, 2)\n        trailing: 3\n        reads") }
            else { BORROW_CLIENT.into() };
        fs::write(&entry, format!("> poly_dispatch::Service\n> poly_dispatch::as_dispatch\n> later_maker::Maker\n{borrowed}")).unwrap();
        for optimization in [0, 3] {
            let consumer = Config { current_crate_name: Some("later_reference".into()),
                extern_artifacts: vec![("poly_dispatch".into(), artifact.clone()), ("later_maker".into(), maker_artifact.clone())], ..config(&root, &entry, optimization) };
            run(&consumer);
            assert_eq!(fs::read(&object).unwrap(), object_before); assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
            assert_eq!(fs::read(&maker_object).unwrap(), maker_object_before); assert_eq!(fs::read(&maker_artifact).unwrap(), maker_artifact_before);
        }
        let mut missing = maker_products.clone();
        missing.link.records.retain(|_, record| !record.backend_symbol.starts_with("rock.erased.member."));
        let missing_artifact = root.join("missing-member.rkca"); missing.write_artifact_to_path(&missing_artifact).unwrap();
        let consumer = Config { current_crate_name: Some("missing_member_client".into()),
            extern_artifacts: vec![("poly_dispatch".into(), artifact.clone()), ("later_maker".into(), missing_artifact)], ..config(&root, &entry, 0) };
        let diagnostics = rock_lib::compile(&consumer).expect_err("an object-provided member may not fall back to an accepted-body clone");
        assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("polymorphic dictionary method") && diagnostic.message.contains("link record")), "exact missing-erased-member authority: {diagnostics:?}");
        let mut native_only = maker_products.clone();
        for record in native_only.link.records.values_mut() { record.polymorphic_member = None; }
        let native_artifact = root.join("native-only-member.rkca"); native_only.write_artifact_to_path(&native_artifact).unwrap();
        let consumer = Config { current_crate_name: Some("wrong_member_abi_client".into()),
            extern_artifacts: vec![("poly_dispatch".into(), artifact.clone()), ("later_maker".into(), native_artifact)], ..config(&root, &entry, 0) };
        let diagnostics = rock_lib::compile(&consumer).expect_err("an untyped native link symbol cannot authorize a polymorphic member ABI");
        assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("polymorphic dictionary method") && diagnostic.message.contains("link record ABI")), "exact missing-erased-ABI authority: {diagnostics:?}");
        for control in 0..4 {
            let mut corrupt = maker_products.clone();
            let link = corrupt.link.records.values_mut().find_map(|record| record.polymorphic_member.as_mut()).unwrap();
            match control {
                0 => link.abi = 2,
                1 => link.entry.member.ret = MirErasedType::MemberParameter(1),
                2 => link.entry.member.borrowing.returns.clear(),
                _ => link.entry.method_id.local.0 = u32::MAX,
            }
            assert!(corrupt.write_artifact_to_path(&root.join(format!("bad-member-link-{control}.rkca"))).is_err());
        }
        assert_eq!(fs::read(&object).unwrap(), object_before); assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
        assert_eq!(fs::read(&maker_object).unwrap(), maker_object_before); assert_eq!(fs::read(&maker_artifact).unwrap(), maker_artifact_before);
    }
}

#[test]
fn polymorphic_dictionary_reference_result_preserves_the_input_loan_at_o0_o3() {
    let root = scratch(); let entry = root.join("main.rk"); fs::write(&entry, format!("{PROVIDER}{MAKER}{BORROW_CLIENT}")).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
}

#[test]
fn polymorphic_dictionary_reference_result_cannot_hide_input_mutation_or_escape() {
    let root = scratch(); let entry = root.join("main.rk");
    fs::write(&entry, format!("{PROVIDER}{MAKER}{}", BORROW_CLIENT.replace("    actual = *result", "    value = 9\n    actual = *result"))).unwrap();
    let diagnostics = rock_lib::compile(&config(&root, &entry, 0)).expect_err("the returned actual input loan must forbid mutation");
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.code == Some(rock_lib::diagnostic::DiagnosticCode::Borrow)), "actual loan diagnostic, not a generic/member frontend blocker: {diagnostics:?}");
    let escape = r#"
escape: &Dispatch -> Maker -> &I64
escape = object, maker ->
    value = 42
    object.apply maker, &value
main = ->
    mut reads = 0
    service = Service
        marker: 0
    object = as_dispatch &service
    maker = Maker
        reads: (&mut reads) as *I64
    result = escape object, maker
    *result
"#;
    fs::write(&entry, format!("{PROVIDER}{MAKER}{escape}")).unwrap();
    let diagnostics = rock_lib::compile(&config(&root, &entry, 0)).expect_err("returned input loan may not outlive its local source");
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.code == Some(rock_lib::diagnostic::DiagnosticCode::Borrow)), "actual loan-escape diagnostic, not a generic/member frontend blocker: {diagnostics:?}");
}

#[test]
fn polymorphic_dictionary_artifact_rejects_member_kind_scope_capture_and_borrow_forgery() {
    let root = scratch(); let provider = root.join("provider.rk"); fs::write(&provider, PROVIDER).unwrap();
    let producer = Config { no_link: true, emit_object: Some(root.join("provider.o")), ..config(&root, &provider, 0) };
    let products = rock_lib::compile_with_products(&producer).unwrap().products.unwrap();
    for control in 0..5 {
        let mut corrupt = products.clone();
        let slot = corrupt.interface.object_abi.schemas.iter_mut().flat_map(|schema| &mut schema.erased_slots).next().unwrap();
        let member = &mut slot.signature.dictionaries[0].members[0];
        match control {
            0 => member.binder.parameters[0].kind = rock_lib::type_services::kind::Kind::arrow(rock_lib::type_services::kind::Kind::Type, rock_lib::type_services::kind::Kind::Type),
            1 => member.ret = MirErasedType::MemberParameter(1),
            2 => member.binder.parameters[0].source = slot.signature.parameters[1].source,
            3 => member.borrowing.returns.clear(),
            _ => member.borrowing.returns[0].sources[0].argument = 0,
        }
        assert!(corrupt.write_artifact_to_path(&root.join(format!("bad-{control}.rkca"))).is_err(), "artifact control {control}");
    }
}
