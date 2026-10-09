//! Specialized generic allocator/State execution, not erased allocator/HKT completion.
//! All compilation goes through the production pipeline with an explicit stdlib artifact.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use rock_lib::diagnostic::DiagnosticCode;
use rock_lib::Config;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(".agent-work/fixtures");
        fs::create_dir_all(&root).expect("create project-local fixture root");
        let path = root.join(format!(
            "rock_allocator_state_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create a fresh fixture directory");
        Self(path)
    }

    fn config(&self, entry: PathBuf, dependencies: Vec<(String, PathBuf)>) -> Config {
        Config {
            entry_file: entry,
            output_dir: self.0.clone(),
            current_crate_name: Some("allocator_state_caller".into()),
            no_std: true,
            no_prelude: true,
            extern_artifacts: dependencies,
            ..Config::default()
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Library {
    // Keep the explicitly built dependency alive for every test in this target.
    _scratch: Scratch,
    artifact: PathBuf,
}

fn stdlib() -> PathBuf {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY
        .get_or_init(|| {
            let scratch = Scratch::new();
            let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
            let object = scratch.0.join("stdlib.o");
            let mut config = scratch.config(workspace.join("stdlib/lib.rk"), vec![]);
            config.current_crate_name = Some("stdlib".into());
            config.no_prelude = false;
            config.no_link = true;
            config.emit_object = Some(object);
            let mut products = rock_lib::compile_with_products(&config)
                .expect("compile the real stdlib for this test target")
                .products
                .unwrap();
            products.link.object_path = Some(PathBuf::from("stdlib.o"));
            let artifact = scratch.0.join("stdlib.rkca");
            products.write_artifact_to_path(&artifact).unwrap();
            Library {
                _scratch: scratch,
                artifact,
            }
        })
        .artifact
        .clone()
}

const IMPORTS: &str = r#"
> stdlib::sized::Sized
> stdlib::drop::Drop
> stdlib::alloc::Global
> stdlib::alloc::Layout
> stdlib::alloc::checked_alloc
> stdlib::object_owner::ObjectOwner
> stdlib::object_owner::ObjectOwnerUnique
> stdlib::object_owner::ObjectOwnerAllocate
> stdlib::libc::exit
> stdlib::libc::putchar
"#;

#[derive(Clone, Copy, Debug)]
enum PayloadPosition {
    First,
    Last,
}

impl PayloadPosition {
    fn args(self, allocator: &str, payload: &str) -> String {
        match self {
            Self::First => format!("{payload}, {allocator}"),
            Self::Last => format!("{allocator}, {payload}"),
        }
    }
}

fn provider(position: PayloadPosition) -> String {
    let declarations = r#"
< trait Read
    @read: I64
< trait Rich where Self: Read
    @marker: I64
    ~@take: I64
< struct Audit
    < base: *U8
    < bytes: I64
    < alignment: I64
    < token: I64
    < allocations: I64
    < releases: I64
    < allocator_drops: I64
    < resource_drops: I64
    < state_drops: I64
    < payload_drops: I64
    < owner_drops: I64
    < unique_releases: I64
    < splits: I64
    < rebuilds: I64
    < reads: I64
    < allocator_bytes: I64
    < state_bytes: I64
< trait Memory
    @audit: *Audit
    @key: I64
    unsafe @reserve: I64 -> I64 -> *U8
    unsafe @release: *U8 -> I64 -> I64 -> I64 -> ()
< fresh = -> Audit
    base: 0 as *U8
    bytes: 0
    alignment: 0
    token: 0
    allocations: 0
    releases: 0
    allocator_drops: 0
    resource_drops: 0
    state_drops: 0
    payload_drops: 0
    owner_drops: 0
    unique_releases: 0
    splits: 0
    rebuilds: 0
    reads: 0
    allocator_bytes: 0
    state_bytes: 0
< struct State A
    < allocator: A
    < audit: *Audit
    < base: *U8
    < bytes: I64
    < alignment: I64
    < token: I64
impl Drop for State A
    ~@drop = !->
        unsafe
            if (~I64Ne ((*self.audit).releases), 1) then exit (61 as I32)
            (*self.audit).state_drops = (~I64Add ((*self.audit).state_drops), 1)
verify: &(State A) -> () where A: Memory
< verify = state !->
    unsafe
        if (~I64Ne (state.base as I64), ((*state.audit).base as I64)) then exit (62 as I32)
        if (~I64Ne state.bytes, ((*state.audit).bytes)) then exit (63 as I32)
        if (~I64Ne state.alignment, ((*state.audit).alignment)) then exit (64 as I32)
        if (~I64Ne state.token, ((*state.audit).token)) then exit (65 as I32)
        if (~I64Ne state.allocator.key!, state.token) then exit (66 as I32)
        if (~I64Ne (~SizeOf state.allocator), ((*state.audit).allocator_bytes)) then exit (67 as I32)
        if (~I64Ne (~SizeOf (*state)), ((*state.audit).state_bytes)) then exit (68 as I32)
< struct Shell $PARAMS
    < pointer: *T
    < state: State A
impl Shell $ARGS where A: Memory
    new = allocator, value -> Shell::new_aligned allocator, value, (~AlignOf value)
    new_aligned = allocator, value, alignment ->
        if (~I64Lt alignment, (~AlignOf value)) then exit (69 as I32)
        audit = allocator.audit!
        token = allocator.key!
        bytes = ~SizeOf value
        base = unsafe allocator.reserve bytes, alignment
        pointer = base as *T
        unsafe *pointer = value
        state = State
            allocator: allocator
            audit: audit
            base: base
            bytes: bytes
            alignment: alignment
            token: token
        unsafe
            (*audit).allocator_bytes = ~SizeOf state.allocator
            (*audit).state_bytes = ~SizeOf state
        Shell
            pointer: pointer
            state: state
impl Shell $ARGS where T: ?Sized
    @borrow = -> unsafe &*self.pointer
impl ObjectOwner (State A) for $SECTION where A: Memory
    unsafe into_parts = owner ->
        verify &owner.state
        (*owner.state.audit).splits = (~I64Add ((*owner.state.audit).splits), 1)
        pointer = owner.pointer
        state_pointer = (&owner.state) as *(State A)
        state = unsafe *state_pointer
        ~Forget owner
        (pointer, state)
    unsafe from_parts = pointer, state ->
        verify &state
        (*state.audit).rebuilds = (~I64Add ((*state.audit).rebuilds), 1)
        Shell
            pointer: pointer
            state: state
impl ObjectOwnerUnique (State A) for $SECTION where A: Memory
    unsafe release = base, state !->
        verify &state
        if (~I64Ne (base as I64), (state.base as I64)) then exit (70 as I32)
        (*state.audit).unique_releases = (~I64Add ((*state.audit).unique_releases), 1)
        state.allocator.release state.base, state.bytes, state.alignment, state.token
impl Drop for Shell $ARGS where A: Memory, T: ?Sized
    ~@drop = !->
        verify &self.state
        unsafe
            (*self.state.audit).owner_drops = (~I64Add ((*self.state.audit).owner_drops), 1)
            ~DropInPlace self.pointer
            self.state.allocator.release self.state.base, self.state.bytes, self.state.alignment, self.state.token
erase: Shell $ARGS -> Shell $RICH where A: Memory, T: Rich
< erase = owner -> owner
upcast: Shell $RICH -> Shell $READ where A: Memory
< upcast = owner -> owner
finish: &Audit -> I64 -> I64 -> I64 -> ()
< finish = audit, payload_drops, owner_drops, rebuilds !->
    if (~I64Ne audit.allocations, 1) then exit (71 as I32)
    if (~I64Ne audit.releases, 1) then exit (72 as I32)
    if (~I64Ne audit.allocator_drops, 1) then exit (73 as I32)
    if (~I64Ne audit.resource_drops, 1) then exit (74 as I32)
    if (~I64Ne audit.state_drops, 1) then exit (75 as I32)
    if (~I64Ne audit.payload_drops, payload_drops) then exit (76 as I32)
    if (~I64Ne audit.owner_drops, owner_drops) then exit (77 as I32)
    if (~I64Ne audit.unique_releases, (~I64Sub 1, owner_drops)) then exit (78 as I32)
    if (~I64Ne audit.splits, 2) then exit (79 as I32)
    if (~I64Ne audit.rebuilds, rebuilds) then exit (80 as I32)
    if (~I64Eq (audit.base as I64), 0) then exit (81 as I32)
    if (~I64Ne (~I64And (audit.base as I64), (~I64Sub audit.alignment, 1)), 0) then exit (82 as I32)
"#;
    let params = match position {
        PayloadPosition::First => "(T: ?Sized), A",
        PayloadPosition::Last => "A, (T: ?Sized)",
    };
    format!("{IMPORTS}{declarations}")
        .replace("$PARAMS", params)
        .replace("$ARGS", &position.args("A", "T"))
        .replace("$SECTION", &format!("Shell {}", position.args("A", "_")))
        .replace("$RICH", &position.args("A", "Rich"))
        .replace("$READ", &position.args("A", "Read"))
}

// These types and implementations are deliberately absent from the owner producer.
const LATER: &str = r#"
struct Resource
    < base: *U8
    < audit: *Audit
impl Resource
    new = audit ->
        layout = Layout
            size: 8
            align: 8
        Resource
            base: checked_alloc layout
            audit: audit
impl Drop for Resource
    ~@drop = !->
        layout = Layout
            size: 8
            align: 8
        unsafe
            if (~I64Ne ((*self.audit).releases), 1) then exit (83 as I32)
            Global::dealloc self.base, layout
            (*self.audit).resource_drops = (~I64Add ((*self.audit).resource_drops), 1)
struct LaterSmall
    < resource: Resource
    < token: I64
impl LaterSmall
    new = audit -> LaterSmall
        resource: Resource::new audit
        token: 41
impl Memory for LaterSmall
    @audit = -> self.resource.audit
    @key = -> self.token
    unsafe @reserve = bytes, alignment -> reserve self.audit!, bytes, alignment, self.key!
    unsafe @release = base, bytes, alignment, token !-> release self.audit!, base, bytes, alignment, token, self.key!
impl Drop for LaterSmall
    ~@drop = !-> unsafe
        (*self.resource.audit).allocator_drops = (~I64Add ((*self.resource.audit).allocator_drops), 1)
struct LaterLarge
    < padding: (I64, I64, I64, I64)
    < resource: Resource
    < token: I64
impl LaterLarge
    new = audit -> LaterLarge
        padding: (3, 5, 7, 11)
        resource: Resource::new audit
        token: 77
impl Memory for LaterLarge
    @audit = -> self.resource.audit
    @key = -> (~I64Add self.token, self.padding.0)
    unsafe @reserve = bytes, alignment -> reserve self.audit!, bytes, alignment, self.key!
    unsafe @release = base, bytes, alignment, token !-> release self.audit!, base, bytes, alignment, token, self.key!
impl Drop for LaterLarge
    ~@drop = !-> unsafe
        (*self.resource.audit).allocator_drops = (~I64Add ((*self.resource.audit).allocator_drops), 1)
reserve = audit, bytes, alignment, token ->
    layout = Layout
        size: bytes
        align: alignment
    base = checked_alloc layout
    unsafe
        if (~I64Ne ((*audit).allocations), 0) then exit (84 as I32)
        (*audit).base = base
        (*audit).bytes = bytes
        (*audit).alignment = alignment
        (*audit).token = token
        (*audit).allocations = 1
    base
release = audit, base, bytes, alignment, token, key !->
    unsafe
        if (~I64Ne ((*audit).releases), 0) then exit (85 as I32)
        if (~I64Ne (base as I64), ((*audit).base as I64)) then exit (86 as I32)
        if (~I64Ne bytes, ((*audit).bytes)) then exit (87 as I32)
        if (~I64Ne alignment, ((*audit).alignment)) then exit (88 as I32)
        if (~I64Ne token, ((*audit).token)) then exit (89 as I32)
        if (~I64Ne key, token) then exit (90 as I32)
        layout = Layout
            size: bytes
            align: alignment
        Global::dealloc base, layout
        (*audit).releases = 1
struct Payload
    < value: I64
    < audit: *Audit
impl Read for Payload
    @read = ->
        address = self as *Payload
        unsafe
            if (~I64Ne (address as I64), ((*self.audit).base as I64)) then exit (91 as I32)
            (*self.audit).reads = (~I64Add ((*self.audit).reads), 1)
        self.value
impl Rich for Payload
    @marker = -> 7
    ~@take = -> self.value
impl Drop for Payload
    ~@drop = !-> unsafe
        (*self.audit).payload_drops = (~I64Add ((*self.audit).payload_drops), 1)
struct Zero
impl Read for Zero
    @read = -> 42
impl Rich for Zero
    @marker = -> 7
    ~@take = -> 42
impl Drop for Zero
    ~@drop = !-> putchar (90 as I32)
"#;

#[derive(Clone, Copy, Debug)]
enum Completion {
    Drop,
    Consume,
    BoundConsume,
}

fn application(completion: Completion) -> String {
    let run = match completion {
        Completion::Drop => "    view = owner.borrow!\n    view.read!",
        Completion::Consume => {
            "    view = owner.borrow!\n    value = view.read!\n    if (~I64Ne value, 42) then return 1\n    owner.take!"
        }
        Completion::BoundConsume => {
            "    view = owner.borrow!\n    value = view.read!\n    if (~I64Ne value, 42) then return 1\n    consume = owner.take\n    consume!"
        }
    };
    let mut source = format!(
        "{LATER}\nrun_small = owner ->\n{run}\nrun_large = owner ->\n{run}\nmain = ->\n"
    );
    for (allocator, suffix) in [("LaterSmall", "small"), ("LaterLarge", "large")] {
        for (case, payload, constructor, expected_drops) in [
            ("normal", "Payload", "new", 1),
            ("zero", "Zero", "new", 0),
            ("aligned", "Payload", "new_aligned", 1),
        ] {
            let name = format!("{suffix}_{case}");
            source.push_str(&format!(
                "    mut audit_{name} = fresh!\n    allocator_{name} = {allocator}::new ((&mut audit_{name}) as *Audit)\n"
            ));
            if payload == "Zero" {
                source.push_str(&format!("    payload_{name} = Zero\n"));
            } else {
                source.push_str(&format!(
                    "    payload_{name} = Payload\n        value: 42\n        audit: (&mut audit_{name}) as *Audit\n"
                ));
            }
            let alignment = if constructor == "new_aligned" { ", 4096" } else { "" };
            source.push_str(&format!(
                "    owner_{name} = Shell::{constructor} allocator_{name}, payload_{name}{alignment}\n    rich_{name} = erase owner_{name}\n    view_{name} = rich_{name}.borrow!\n    marker_{name} = view_{name}.marker!\n    if (~I64Ne marker_{name}, 7) then return 2\n"
            ));
            let (owner_drops, rebuilds) = match completion {
                Completion::Drop => {
                    source.push_str(&format!(
                        "    base_{name} = upcast rich_{name}\n    answer_{name} = run_{suffix} base_{name}\n"
                    ));
                    (1, 2)
                }
                Completion::Consume | Completion::BoundConsume => {
                    source.push_str(&format!("    answer_{name} = run_{suffix} rich_{name}\n"));
                    (0, 1)
                }
            };
            source.push_str(&format!(
                "    if (~I64Ne answer_{name}, 42) then return 3\n    finish &audit_{name}, {expected_drops}, {owner_drops}, {rebuilds}\n"
            ));
            if payload == "Zero" {
                source.push_str(&format!("    if (~I64Ne audit_{name}.bytes, 0) then return 4\n"));
            } else {
                source.push_str(&format!(
                    "    if (~I64Le audit_{name}.bytes, 0) then return 5\n    if (~I64Ne audit_{name}.reads, 1) then return 6\n"
                ));
            }
            if constructor == "new_aligned" {
                source.push_str(&format!("    if (~I64Ne audit_{name}.alignment, 4096) then return 7\n"));
            }
        }
    }
    source.push_str(
        "    if (~I64Le audit_large_normal.allocator_bytes, audit_small_normal.allocator_bytes) then return 8\n    if (~I64Le audit_large_normal.state_bytes, audit_small_normal.state_bytes) then return 9\n    42\n"
    );
    source
}

fn execute(config: &Config, expected_stdout: &[u8]) {
    // This path includes native MIR ownership, runtime agreement and LLVM validation.
    let mut observed = config.clone();
    if let Ok(flags) = std::env::var("ROCK_ALLOCATOR_EXECUTION_DEBUG") {
        for flag in flags.split(',') {
            let debug = match flag {
                "ast-full" => rock_lib::DebugPrint::AstFull,
                "hir" => rock_lib::DebugPrint::Hir,
                "tokens" => rock_lib::DebugPrint::Tokens,
                "mir" => rock_lib::DebugPrint::Mir,
                _ => panic!("unsupported allocator diagnostic flag: {flag}"),
            };
            if !observed.has_debug_print(debug.clone()) {
                observed.debug_print.push(debug);
            }
        }
    }
    if std::env::var("ROCK_ALLOCATOR_EXECUTION_MIR").as_deref() == Ok("1") {
        if !observed.has_debug_print(rock_lib::DebugPrint::Mir) {
            observed.debug_print.push(rock_lib::DebugPrint::Mir);
        }
    }
    if std::env::var("ROCK_ALLOCATOR_FRONTEND_ONLY").as_deref() == Ok("1") {
        eprintln!("ALLOCATOR_FRONTEND_ONLY_SOURCE {}\n{}", config.entry_file.display(),
            fs::read_to_string(&config.entry_file).expect("actual matrix source"));
        rock_lib::analyze(&observed).unwrap_or_else(|errors| {
            panic!("frontend-only analysis rejected actual matrix source: {errors:?}")
        });
        // Do not report an execution test as passing when only analysis ran.
        panic!("ALLOCATOR_FRONTEND_ONLY_COMPLETE: actual AST/HIR captured; runtime NOT RUN");
    }
    rock_lib::compile(&observed).unwrap_or_else(|errors| {
        panic!("O{}, {}: {errors:?}", config.opt_level, config.entry_file.display())
    });
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert_eq!(output.status.code(), Some(42), "{output:?}");
    assert_eq!(output.stdout, expected_stdout, "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

fn local_matrix(position: PayloadPosition) {
    let scratch = Scratch::new();
    let entry = scratch.0.join("main.rk");
    for completion in [Completion::Drop, Completion::Consume, Completion::BoundConsume] {
        fs::write(&entry, format!("{}{}", provider(position), application(completion))).unwrap();
        for opt_level in [0, 3] {
            let artifact = if std::env::var("ROCK_ALLOCATOR_FRONTEND_ONLY").as_deref() == Ok("1") {
                PathBuf::from(std::env::var_os("ROCK_ALLOCATOR_TRACE_STDLIB")
                    .expect("frontend-only trace requires an explicit real stdlib artifact"))
            } else {
                stdlib()
            };
            let mut config = scratch.config(entry.clone(), vec![("stdlib".into(), artifact)]);
            config.opt_level = opt_level;
            execute(&config, b"ZZ");
        }
    }
}

#[test]
fn payload_last_allocator_state_drop_and_unique_consumption_execute() {
    local_matrix(PayloadPosition::Last);
}

#[test]
fn payload_first_allocator_state_drop_and_unique_consumption_execute() {
    local_matrix(PayloadPosition::First);
}

#[test]
fn known_record_fields_keep_authority_despite_unrelated_allocator_methods() {
    let scratch = Scratch::new();
    let entry = scratch.0.join("main.rk");
    let source = r#"
struct Audit
    < value: I64
trait Memory
    @audit: *Audit
struct LaterSmall
    < pointer: *Audit
impl Memory for LaterSmall
    @audit = -> self.pointer
struct LaterLarge
    < padding: (I64, I64)
    < pointer: *Audit
impl Memory for LaterLarge
    @audit = -> self.pointer
struct State A
    < allocator: A
    < audit: *Audit
impl State A where A: Memory
    @read = -> unsafe
        pointer = self.allocator.audit!
        if (~I64Ne (pointer as I64), (self.audit as I64)) then exit (92 as I32)
        (*self.audit).value
struct Resource
    < audit: *Audit
impl Resource
    @read = -> unsafe (*self.audit).value
main = ->
    mut audit = Audit
        value: 42
    small = LaterSmall
        pointer: (&mut audit) as *Audit
    large = LaterLarge
        padding: (1, 2)
        pointer: (&mut audit) as *Audit
    first = State
        allocator: small
        audit: (&mut audit) as *Audit
    last = State
        allocator: large
        audit: (&mut audit) as *Audit
    resource = Resource
        audit: (&mut audit) as *Audit
    if (~I64Ne first.read!, 42) then return 1
    if (~I64Ne last.read!, 42) then return 2
    if (~I64Ne resource.read!, 42) then return 3
    view = &first
    unsafe
        if (~I64Ne ((*first.audit).value), 42) then return 4
        if (~I64Ne ((*view.audit).value), 42) then return 5
    42
"#;
    fs::write(&entry, format!("{IMPORTS}{source}")).unwrap();
    for opt_level in [0, 3] {
        let mut config = scratch.config(entry.clone(), vec![("stdlib".into(), stdlib())]);
        config.opt_level = opt_level;
        execute(&config, b"");
    }
}

fn source_free_matrix(position: PayloadPosition) {
    let scratch = Scratch::new();
    let entry = scratch.0.join("owners.rk");
    // LATER is only appended to the caller, never to this producer.
    fs::write(&entry, provider(position)).unwrap();
    let mut config = scratch.config(entry.clone(), vec![("stdlib".into(), stdlib())]);
    config.current_crate_name = Some("owners".into());
    config.no_link = true;
    config.emit_object = Some(scratch.0.join("owners.o"));
    let mut products = rock_lib::compile_with_products(&config)
        .expect("compile generic owner producer without future allocators or payloads")
        .products
        .unwrap();
    products.link.object_path = Some(PathBuf::from("owners.o"));
    let artifact = scratch.0.join("owners.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let stdlib_artifact = stdlib();
    let originals = [
        artifact.clone(),
        scratch.0.join("owners.o"),
        stdlib_artifact.clone(),
        stdlib_artifact.parent().unwrap().join("stdlib.o"),
    ]
    .into_iter()
    .map(|path| {
        let bytes = fs::read(&path).unwrap();
        (path, bytes)
    })
    .collect::<Vec<_>>();
    fs::remove_file(&entry).unwrap();
    assert!(!entry.exists());
    let imports = r#"
> owners::Audit
> owners::State
> owners::Shell
> owners::Memory
> owners::Read
> owners::Rich
> owners::fresh
> owners::erase
> owners::upcast
> owners::finish
"#;
    let caller = scratch.0.join("main.rk");
    for completion in [Completion::Drop, Completion::Consume, Completion::BoundConsume] {
        fs::write(&caller, format!("{IMPORTS}{imports}{}", application(completion))).unwrap();
        for opt_level in [0, 3] {
            let mut config = scratch.config(
                caller.clone(),
                vec![("stdlib".into(), stdlib()), ("owners".into(), artifact.clone())],
            );
            config.opt_level = opt_level;
            execute(&config, b"ZZ");
            for (path, original) in &originals {
                assert_eq!(&fs::read(path).unwrap(), original, "{}", path.display());
            }
            assert!(!entry.exists());
        }
    }
}

#[test]
fn source_free_payload_last_producer_accepts_later_allocator_and_payload_types() {
    source_free_matrix(PayloadPosition::Last);
}

#[test]
fn source_free_payload_first_producer_accepts_later_allocator_and_payload_types() {
    source_free_matrix(PayloadPosition::First);
}

#[test]
fn stdlib_allocation_capability_controls_honor_zero_and_overalignment() {
    let scratch = Scratch::new();
    let entry = scratch.0.join("main.rk");
    let mut source = format!("{IMPORTS}\n> stdlib::box_type::Box\n> stdlib::arc::Arc\nmain = ->\n    mut cancellations = 0\n");
    for owner in ["Box", "Arc"] {
        for (case, bytes, alignment) in [("zero", 0, 64), ("aligned", 24, 4096)] {
            let name = format!("{}_{}", owner.to_lowercase(), case);
            source.push_str(&format!(
                "    {name} = unsafe {owner}::allocate {bytes}, {alignment}\n    if (~I64Eq ({name}.0 as I64), 0) then return 1\n    if (~I64Ne (~I64And ({name}.0 as I64), {}), 0) then return 2\n    unsafe {owner}::cancel {name}.0, {name}.1\n    cancellations = (~I64Add cancellations, 1)\n",
                alignment - 1
            ));
        }
    }
    source.push_str("    if (~I64Ne cancellations, 4) then return 3\n    42\n");
    fs::write(&entry, source).unwrap();
    for opt_level in [0, 3] {
        let mut config = scratch.config(entry.clone(), vec![("stdlib".into(), stdlib())]);
        config.opt_level = opt_level;
        execute(&config, b"");
    }
}

fn reject(position: PayloadPosition, body: &str, expected: &[&str]) {
    let scratch = Scratch::new();
    let entry = scratch.0.join("main.rk");
    let source = format!(
        "{}{LATER}\nmain = !->\n    mut audit = fresh!\n    allocator = LaterSmall::new ((&mut audit) as *Audit)\n    payload = Payload\n        value: 42\n        audit: (&mut audit) as *Audit\n    concrete = Shell::new allocator, payload\n    owner = erase concrete\n    {body}\n",
        provider(position)
    );
    fs::write(&entry, &source).unwrap();
    let config = scratch.config(entry.clone(), vec![("stdlib".into(), stdlib())]);
    let errors = rock_lib::compile(&config).expect_err("invalid owner use must be rejected");
    assert!(errors.0.iter().all(|error| error.code != Some(DiagnosticCode::Internal)), "{errors:?}");
    let body_start = source.find(body).expect("negative body is in the fixture");
    let matching = errors.0.iter().find(|error| {
        matches!(error.code, Some(DiagnosticCode::Borrow | DiagnosticCode::Type | DiagnosticCode::Selection))
            && expected.iter().any(|message| error.message.contains(message))
            && error.primary.as_ref().is_some_and(|label| label.span.start >= body_start)
    })
        .unwrap_or_else(|| panic!("missing expected {expected:?} for {body:?}: {errors:?}"));
    let span = &matching.primary.as_ref().expect("source-backed rejection").span;
    assert_eq!(span.file_path, entry);
    assert!(span.start < span.end && span.end <= source.len(), "{span:?}");
}

#[test]
fn allocator_owners_reject_live_views_reuse_and_double_bound_consumption() {
    for position in [PayloadPosition::First, PayloadPosition::Last] {
        for body in [
            "view = owner.borrow!\n    owner.take!\n    view.read!",
            "owner.take!\n    owner.take!",
            "consume = owner.take\n    consume!\n    consume!",
        ] {
            reject(position, body, &["borrow", "move", "consum"]);
        }
    }
}

#[test]
fn allocator_owners_reject_changed_fixed_allocator_and_wrong_state() {
    for position in [PayloadPosition::First, PayloadPosition::Last] {
        let wrong_allocator = format!("changed: Shell {} = owner", position.args("LaterLarge", "Rich"));
        reject(position, &wrong_allocator, &["exact owner constructor and State"]);
        let wrong_state = format!(
            "other_allocator = LaterLarge::new ((&mut audit) as *Audit)\n    other_payload = Payload\n        value: 42\n        audit: (&mut audit) as *Audit\n    other = Shell::new other_allocator, other_payload\n    parts = unsafe (Shell {})::into_parts other\n    unsafe (Shell {})::from_parts parts.0, parts.1",
            position.args("LaterLarge", "_"),
            position.args("LaterSmall", "_")
        );
        reject(position, &wrong_state, &["Type mismatch"]);
    }
}
