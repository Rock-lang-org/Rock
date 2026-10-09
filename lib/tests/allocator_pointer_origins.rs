//! Production-input guards for native raw contents/storage authority and State extraction.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::diagnostic::DiagnosticCode;
use rock_lib::{Config, SourceProvider};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(Config);

impl Fixture {
    fn new(source: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures");
        fs::create_dir_all(&root).unwrap();
        let output = root.join(format!(
            "rock_pointer_origins_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&output).unwrap();
        let entry = output.join("main.rk");
        Self(Config {
            entry_file: entry.clone(),
            output_dir: output,
            current_crate_name: Some("pointer_origins".into()),
            no_std: true,
            no_prelude: true,
            source_providers: vec![SourceProvider::Virtual {
                path: entry,
                text: source.into(),
            }],
            ..Config::default()
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0.output_dir);
    }
}

fn execute(source: &str) {
    let mut fixture = Fixture::new(source);
    if std::env::var("ROCK_POINTER_ORIGINS_FRONTEND_ONLY").as_deref() == Ok("1") {
        fixture.0.debug_print = vec![rock_lib::DebugPrint::AstFull, rock_lib::DebugPrint::Hir];
        eprintln!("POINTER_ORIGINS_FRONTEND_ONLY_SOURCE {}\n{source}", fixture.0.entry_file.display());
        // This self-contained input has no dependencies: observe its real local
        // lowering before production analyze consumes the failed inference state.
        let module = rock_lib::parser::parse_source(fixture.0.entry_file.clone(), source, &fixture.0)
            .expect("parse unchanged pointer-origin source");
        let partial = rock_lib::lower::Lowerer::with_options(false)
            .lower_program(&rock_lib::ast::Program { module })
            .expect("lower unchanged pointer-origin source");
        for function in partial.functions.values().filter(|function| {
            matches!(function.name.as_str(), "main" | "consume")
        }).chain(partial.impls.values().filter_map(|implementation| implementation.methods.get("extract"))) {
            eprintln!("POINTER_ORIGINS_LOCAL_LOWERED function={function:#?} signature_slots={:?} resolved_params={:?} resolved_result={:?}",
                partial.function_type_vars.get(&function.id),
                function.params.iter().map(|param| partial.engine.resolve(&param.ty)).collect::<Vec<_>>(),
                partial.engine.resolve(&function.ret_type));
        }
        for implementation in partial.impls.values().filter(|implementation| implementation.methods.contains_key("extract")) {
            eprintln!("POINTER_ORIGINS_LOCAL_LOWERED impl={:?} receiver={:?} generic_params={:?} bounds={:?}",
                implementation.id, implementation.receiver_pattern,
                implementation.type_generics, implementation.bounds);
        }
        eprintln!("POINTER_ORIGINS_LOCAL_LOWERED constraints={:#?}", partial.constraint_store);
        rock_lib::analyze(&fixture.0).unwrap_or_else(|errors| {
            panic!("frontend-only analysis rejected actual pointer-origin source: {errors:?}")
        });
        // A diagnostic-only run must not stand in for O0/O3 execution/count gates.
        panic!("POINTER_ORIGINS_FRONTEND_ONLY_COMPLETE: runtime NOT RUN");
    }
    for level in [0, 3] {
        fixture.0.opt_level = level;
        rock_lib::compile(&fixture.0).unwrap_or_else(|errors| panic!("O{level}: {errors:?}"));
        let output = Command::new(fixture.0.output_dir.join("main")).output().unwrap();
        assert_eq!(output.status.code(), Some(42), "O{level}: {output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty(), "{output:?}");
    }
}

fn reject(source: &str, messages: &[&str]) {
    let mut fixture = Fixture::new(source);
    fixture.0.no_link = true;
    let errors = rock_lib::compile(&fixture.0).expect_err("must reject at production Borrow checking");
    assert!(errors.0.iter().all(|error| error.code == Some(DiagnosticCode::Borrow)), "{errors:?}");
    for message in messages {
        let error = errors.0.iter().find(|error| error.message.contains(message))
            .unwrap_or_else(|| panic!("missing {message:?}: {errors:?}"));
        let span = &error.primary.as_ref().expect("actual source primary").span;
        assert_eq!(span.file_path, fixture.0.entry_file);
        assert!(span.start < span.end && span.end <= source.len());
    }
}

const DROP: &str = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
"#;

const EXTRACTION: &str = r#"
struct Allocator
    < drops: *I64
impl Dispose for Allocator
    ~@dispose = !-> unsafe *self.drops = (~I64Add (*self.drops), 1)
struct State A
    < allocator: A
    < drops: *I64
impl Dispose for State A
    ~@dispose = !-> unsafe *self.drops = (~I64Add (*self.drops), 1)
struct Holder A
    < pointer: *I64
    < state: State A
impl Dispose for Holder A
    ~@dispose = !-> ()
consume: State A -> ()
consume = state !-> ()
"#;

// Exact original virtual input: 1203 bytes, SHA256
// b293cfe47ee2264a7a4e8a159c11cbce87e071abe4d070f665d5b43006db6afb.
const UNDERCONSTRAINED_STATE_SPLIT: &str =
    include_str!("fixtures/allocator_state_split_underconstrained.rk");

#[test]
fn raw_state_reinterpretation_without_split_contract_is_type_ambiguous() {
    // Preserve the previous generated source, including leading and inter-block
    // blank lines, as an independent check on the full-source archive.
    let original = format!(r#"{DROP}{EXTRACTION}
impl Holder A
    unsafe extract = owner ->
        pointer = owner.pointer
        state_pointer = (&owner.state) as *(State A)
        state = unsafe *state_pointer
        ~Forget owner
        (pointer, state)
main = ->
    mut allocator_drops = 0
    mut state_drops = 0
    value = 42
    allocator = Allocator
        drops: (&mut allocator_drops) as *I64
    owner = Holder
        pointer: (&value) as *I64
        state: State
            allocator: allocator
            drops: (&mut state_drops) as *I64
    parts = unsafe Holder::extract owner
    pointer = parts.0
    consume parts.1
    if (~I64Ne allocator_drops, 1) then return 1
    if (~I64Ne state_drops, 1) then return 2
    unsafe *pointer
"#);
    assert_eq!(original.as_bytes(), UNDERCONSTRAINED_STATE_SPLIT.as_bytes());
    assert_eq!(UNDERCONSTRAINED_STATE_SPLIT.len(), 1203);
    let fixture = Fixture::new(UNDERCONSTRAINED_STATE_SPLIT);
    let errors = rock_lib::analyze(&fixture.0)
        .expect_err("unsafe reinterpretation does not tie input Holder B to result State A");
    assert!(errors.0.iter().all(|error| error.code == Some(DiagnosticCode::Type)), "{errors:?}");
    assert!(errors.0.iter().any(|error| {
        let Some(primary) = &error.primary else { return false; };
        primary.span.file_path == fixture.0.entry_file
            && matches!(
                (primary.span.start, primary.span.end),
                (1073, 1080) | (598, 603)
            )
    }), "missing original consume/state ambiguity primary: {errors:?}");
    for error in &errors.0 {
        let span = &error.primary.as_ref().expect("actual source primary").span;
        assert_eq!(span.file_path, fixture.0.entry_file);
        assert!(span.start < span.end && span.end <= UNDERCONSTRAINED_STATE_SPLIT.len());
    }
}

#[test]
fn declared_split_raw_state_read_then_forget_drops_exact_state_and_allocator_once() {
    const HEADER: &str = "    unsafe extract: Holder A -> (*I64, State A)\n";
    const IMPL: &str = "impl Holder A\n";
    assert_eq!(UNDERCONSTRAINED_STATE_SPLIT.matches(IMPL).count(), 1);
    let source = UNDERCONSTRAINED_STATE_SPLIT.replacen(IMPL, &format!("{IMPL}{HEADER}"), 1);
    assert_eq!(source.matches(HEADER).count(), 1);
    assert_eq!(source.replacen(HEADER, "", 1).as_bytes(), UNDERCONSTRAINED_STATE_SPLIT.as_bytes());
    // Only the declared API relation is new; the read/Forget order, main and
    // exact drop counters remain the original source's O0/O3 obligations.
    execute(&source);
}

#[test]
fn ordinary_drop_field_move_then_forget_rejects_even_in_unsafe() {
    reject(&format!(r#"{DROP}{EXTRACTION}
main = !->
    mut drops = 0
    value = 42
    owner = Holder
        pointer: (&value) as *I64
        state: State
            allocator: Allocator
                drops: (&mut drops) as *I64
            drops: (&mut drops) as *I64
    unsafe
        parts = (owner.pointer, owner.state)
        ~Forget owner
        consume parts.1
"#), &["because it implements Drop", "partially moved"]);
}

#[test]
fn native_allocation_and_copied_audit_contents_do_not_lease_receiver_storage() {
    execute(&format!(r#"{DROP}
extern malloc: I64 -> *U8
extern free: *U8 -> ()
struct Audit
    < value: I64
    < drops: I64
struct Allocator
    < audit: *Audit
impl Allocator
    @copied = -> self.audit
    unsafe @allocate = -> malloc 8
impl Dispose for Allocator
    ~@dispose = !-> unsafe (*self.audit).drops = (~I64Add ((*self.audit).drops), 1)
consume: Allocator -> ()
consume = allocator !-> ()
main = ->
    mut audit = Audit
        value: 0
        drops: 0
    allocator = Allocator
        audit: (&mut audit) as *Audit
    copied = allocator.copied!
    pointer = (unsafe allocator.allocate!) as *I64
    consume allocator
    unsafe
        (*copied).value = 41
        *pointer = 42
    if (~I64Ne audit.value, 41) then return 1
    if (~I64Ne audit.drops, 1) then return 2
    result = unsafe *pointer
    free (pointer as *U8)
    result
"#));
    // Independent shared-receiver address control, reusing the O0/O3 runner.
    // The original copied-audit/allocation/drop program above is unchanged.
    execute(r#"
extern malloc: I64 -> *U8
extern free: *U8 -> ()
struct Payload
    < value: I64
impl Payload
    @inspect: *Payload -> I64
    @inspect = expected ->
        address = self as *Payload
        reference_storage = (&self) as *Payload
        if (~I64Ne (address as I64), (expected as I64)) then return 1
        if (~I64Eq (reference_storage as I64), (expected as I64)) then return 2
        self.value
main = ->
    pointer = (malloc 8) as *Payload
    if (~I64Eq (pointer as I64), 0) then return 3
    answer = unsafe
        *pointer = Payload
            value: 42
        (&(*pointer)).inspect pointer
    free (pointer as *U8)
    answer
"#);
}

const STORAGE: &str = r#"
struct Storage
    < value: I64
impl Storage
    @address = -> (&self.value) as *I64
consume: Storage -> ()
consume = owner !-> ()
"#;

#[test]
fn raw_address_of_receiver_field_keeps_parent_storage_loan() {
    reject(&format!(r#"{STORAGE}
main = !->
    owner = Storage
        value: 7
    raw = owner.address!
    consume owner
    unsafe *raw
"#), &["borrow conflict"]);
}

#[test]
fn raw_address_reconstructed_as_reference_blocks_parent_write() {
    reject(&format!(r#"{STORAGE}
main = !->
    mut owner = Storage
        value: 7
    raw = owner.address!
    view = unsafe &*raw
    owner.value = 9
    *view
"#), &["borrow conflict"]);
}

#[test]
fn mutable_receiver_address_retains_parent_move_and_read_guards() {
    let declarations = format!(r#"{STORAGE}
impl Storage
    ^@mutable_address = -> (&mut self.value) as *I64
"#);
    for access in ["consume owner", "owner.value"] {
        reject(&format!(r#"{declarations}
main = !->
    mut owner = Storage
        value: 7
    raw = owner.mutable_address!
    {access}
    unsafe *raw
"#), &["borrow conflict"]);
    }
}

#[test]
fn opaque_reference_input_raw_result_remains_conservative() {
    reject(&format!(r#"{STORAGE}
extern opaque: &Storage -> *I64
forward = owner -> opaque owner
main = !->
    owner = Storage
        value: 7
    raw = forward &owner
    consume owner
    unsafe *raw
"#), &["borrow conflict"]);
}

#[test]
fn raw_reinterpretation_does_not_erase_live_safe_alias() {
    reject(r#"
main = !->
    value = 7
    shared = &value
    raw = shared as *I64
    unsafe *raw = 9
    *shared
"#, &["borrow conflict"]);
}

#[test]
fn raw_state_read_keeps_contained_reference_alive_through_forget() {
    reject(&format!(r#"{DROP}
struct State
    < view: &I64
struct Holder
    < state: State
impl Dispose for Holder
    ~@dispose = !-> ()
main = !->
    mut value = 7
    owner = Holder
        state: State
            view: &value
    pointer = (&owner.state) as *State
    state = unsafe *pointer
    ~Forget owner
    value = 9
    *state.view
"#), &["borrow conflict"]);
}

#[test]
fn raw_state_read_does_not_end_live_contained_safe_alias() {
    reject(&format!(r#"{DROP}
struct State
    < view: &I64
struct Holder
    < state: State
impl Dispose for Holder
    ~@dispose = !-> ()
main = !->
    value = 7
    alias = (&value) as *I64
    owner = Holder
        state: State
            view: &value
    pointer = (&owner.state) as *State
    state = unsafe *pointer
    ~Forget owner
    unsafe *alias = 9
    *state.view
"#), &["borrow conflict"]);
}

#[test]
fn raw_state_pointer_used_after_forget_keeps_owner_address_lease() {
    reject(&format!(r#"{DROP}
struct State
    < value: I64
struct Holder
    < state: State
impl Dispose for Holder
    ~@dispose = !-> ()
main = !->
    owner = Holder
        state: State
            value: 7
    pointer = (&owner.state) as *State
    state = unsafe *pointer
    ~Forget owner
    unsafe (*pointer).value
    state.value
"#), &["borrow conflict"]);
}
