//! Owning openings transport allocation and non-Copy State through selected protocols.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::Config;

const PROVIDER: &str = r#"
lang sized
< trait KnownSize
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
lang object_owner
< trait Transport State for F _
    lang owner_into_parts
    unsafe split: F T -> (*T, State) where T: ?KnownSize
    lang owner_from_parts
    unsafe rebuild: *T -> State -> F T where T: ?KnownSize
lang object_owner_unique
< trait Unique State for F _
    lang owner_release
    unsafe release: *U8 -> State -> ()
extern malloc: I64 -> *U8
extern free: *U8 -> ()
record: *I64 -> I64 -> ()
< record = pointer, tag !->
    unsafe *pointer = ~I64Add (~I64Mul (*pointer), 10), tag
< struct State
    < base: *U8
    < events: *I64
impl Dispose for State
    ~@dispose = !-> record self.events, 3
release_memory: State -> ()
release_memory = state !->
    record state.events, 2
    free state.base
finish: *U8 -> *I64 -> ()
finish = base, events !->
    release_memory (State
        base: base
        events: events)
unsafe destroy_at: *T -> () where T: ?KnownSize
unsafe destroy_at = pointer !-> unsafe ~DropInPlace pointer
< struct Own (T: ?KnownSize)
    < raw: *T
    < base: *U8
    < events: *I64
impl Own T
    new: T -> *I64 -> Own T
    new = value, events ->
        base = malloc (~SizeOf value)
        pointer: *T = base as *T
        unsafe *pointer = value
        Own
            raw: pointer
            base: base
            events: events
impl Transport State for Own
    unsafe split = owner ->
        parts = (owner.raw, (State
            base: owner.base
            events: owner.events))
        ~Forget owner
        parts
    unsafe rebuild = pointer, state ->
        owner = Own
            raw: pointer
            base: state.base
            events: state.events
        ~Forget state
        owner
impl Unique State for Own
    unsafe release = _, state !-> release_memory state
impl Dispose for Own T where T: ?KnownSize
    ~@dispose = !->
        unsafe destroy_at self.raw
        finish self.base, self.events
< trait Take
    ~@take: I64
    ~@with: T -> T
take_open: Own Take -> Bool -> I64
< take_open = owner, early ->
    open owner as Hidden, value
        if early then return 42
        value.take!
with_open: Own Take -> I64 -> I64
< with_open = owner, argument ->
    open owner as Hidden, value
        value.with argument
"#;

const CLIENT: &str = r#"
struct Item
    < events: *I64
    < tag: I64
impl Take for Item
    ~@take = -> self.tag
    ~@with = argument -> argument
impl Dispose for Item
    ~@dispose = !-> record self.events, 1
main = ->
    mut events = 0
    pointer = (&mut events) as *I64
    owner: Own Take = Own::new (Item
        events: pointer
        tag: 42), pointer
    answer = take_open owner, false
    if ~I64Eq events, 123 then answer else 1
"#;

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn scratch() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures")
        .join(format!("owning-open-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&root).unwrap();
    root
}
fn config(root: &Path, entry: &Path, opt_level: u8) -> Config {
    Config {
        entry_file: entry.into(), output_dir: root.into(), no_std: true, no_prelude: true,
        current_crate_name: Some("owning_open".into()), emit_llvm: true, opt_level,
        ..Default::default()
    }
}
fn run(config: &Config) {
    rock_lib::compile(config).unwrap_or_else(|errors| panic!("{:?}", errors.0.iter()
        .map(|error| (&error.code, &error.message, &error.primary)).collect::<Vec<_>>()));
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert_eq!(output.status.code(), Some(42), "owning transport events: {output:?}");
}

#[test]
fn owning_open_consumption_and_early_cleanup_preserve_noncopy_state_at_o0_o3() {
    let root = scratch();
    let entry = root.join("main.rk");
    for early in [false, true] {
        fs::write(&entry, format!("{PROVIDER}{}", CLIENT.replace("take_open owner, false", &format!("take_open owner, {early}")))).unwrap();
        for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
    }
}

#[test]
fn owning_open_erased_consumption_initializes_result_before_release_at_o0_o3() {
    let root = scratch();
    let entry = root.join("main.rk");
    fs::write(&entry, format!("{PROVIDER}{}", CLIENT.replace("take_open owner, false", "with_open owner, 42"))).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
}

#[test]
fn owning_open_rejects_a_live_owner_loan_and_a_second_consumption() {
    let root = scratch();
    let entry = root.join("main.rk");
    for body in ["view = &value\n        value.take!\n        keep view\n        42", "value.take!\n        value.take!"] {
        let provider = PROVIDER.replace("        value.take!", &format!("        {body}"));
        fs::write(&entry, format!("keep: T -> ()\nkeep = value !-> ()\n{provider}{CLIENT}")).unwrap();
        let errors = rock_lib::compile(&config(&root, &entry, 0)).unwrap_err();
        assert!(errors.0.iter().any(|error| error.code == Some(rock_lib::diagnostic::DiagnosticCode::Borrow)
            && error.primary.as_ref().is_some_and(|label| {
                fs::read_to_string(&entry).unwrap().get(label.span.start..label.span.end) == Some("value")
            })), "{:?}", errors.0.iter().map(|error| (&error.code, &error.message, &error.primary)).collect::<Vec<_>>());
        assert!(errors.0.iter().all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)));
    }
}

#[test]
fn source_free_owning_open_preserves_provider_bytes_for_two_payload_layouts() {
    let root = scratch();
    let provider = root.join("provider.rk");
    fs::write(&provider, PROVIDER).unwrap();
    let object = root.join("provider.o");
    let mut products = rock_lib::compile_with_products(&Config {
        no_link: true, emit_object: Some(object.clone()), ..config(&root, &provider, 0)
    }).unwrap().products.unwrap();
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let object_bytes = fs::read(&object).unwrap();
    let artifact_bytes = fs::read(&artifact).unwrap();
    fs::remove_file(provider).unwrap();
    let entry = root.join("main.rk");
    for layout in 0..4 {
        let client = match layout {
            0 => CLIENT.into(),
            1 => CLIENT.replace("< tag: I64", "< tag: I64\n    < padding: (I64, I64, I64)")
                .replace("tag: 42", "tag: 42\n        padding: (2, 3, 4)"),
            2 => zero_sized_client(),
            _ => format!(r#"
extern malloc: I64 -> *U8
make_offset: T -> *I64 -> Own T
make_offset = value, events ->
    base = malloc (~I64Add (~SizeOf value), 16)
    pointer: *T = (~I64Add (base as I64), 16) as *T
    unsafe *pointer = value
    Own
        raw: pointer
        base: base
        events: events
{}"#, CLIENT.replace("Own::new", "make_offset")),
        };
        for call in ["take_open owner, false", "take_open owner, true", "with_open owner, 42"] {
            fs::write(&entry, format!("\n> owning_open::Take\n> owning_open::Dispose\n> owning_open::Own\n> owning_open::record\n> owning_open::take_open\n> owning_open::with_open\n{}",
                client.replace("take_open owner, false", call))).unwrap();
            for optimization in [0, 3] {
                run(&Config {
                    extern_artifacts: vec![("owning_open".into(), artifact.clone())],
                    current_crate_name: Some("owning_open_client".into()),
                    ..config(&root, &entry, optimization)
                });
                assert_eq!(fs::read(&object).unwrap(), object_bytes);
                assert_eq!(fs::read(&artifact).unwrap(), artifact_bytes);
            }
        }
    }
}

fn borrowed_state_provider() -> String {
    PROVIDER.replace("< events: *I64", "< events: *I64\n    < marker: &I64")
        .replace("new: T -> *I64 -> Own T", "new: T -> *I64 -> &I64 -> Own T")
        .replace("new = value, events ->", "new = value, events, marker ->")
        .replace("            events: events", "            events: events\n            marker: marker")
        .replace("            events: owner.events", "            events: owner.events\n            marker: owner.marker")
        .replace("            events: state.events", "            events: state.events\n            marker: state.marker")
        .replace("finish: *U8 -> *I64 -> ()", "finish: *U8 -> *I64 -> &I64 -> ()")
        .replace("finish = base, events !->", "finish = base, events, marker !->")
        .replace("        events: events)", "        events: events\n        marker: marker)")
        .replace("finish self.base, self.events", "finish self.base, self.events, self.marker")
}

#[test]
fn owning_state_loan_survives_consumption_then_ends_at_the_scalar_return() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = borrowed_state_provider().replace("        value.take!", "        view = value.marker\n        value.take!\n        *view");
    let client = CLIENT.replace("    pointer =", "    mut marker = 42\n    pointer =")
        .replace("tag: 42), pointer", "tag: 42), pointer, &marker")
        .replace("    if ~I64Eq events", "    marker = 7\n    if ~I64Eq events");
    fs::write(&entry, format!("{provider}{client}")).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
}

#[test]
fn returned_state_reference_keeps_its_caller_loan_after_owner_release() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = format!("{}\nmarker_open: Own Take -> &I64\nmarker_open = owner ->\n    open owner as Hidden, value\n        view = value.marker\n        value.take!\n        view\n", borrowed_state_provider());
    let client = CLIENT.replace("    pointer =", "    mut marker = 42\n    pointer =")
        .replace("tag: 42), pointer", "tag: 42), pointer, &marker")
        .replace("answer = take_open owner, false", "view = marker_open owner\n    answer = *view");
    fs::write(&entry, format!("{provider}{client}")).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
    let negative = client.replace("    answer = *view", "    marker = 7\n    answer = *view");
    fs::write(&entry, format!("{provider}{negative}")).unwrap();
    let errors = rock_lib::compile(&config(&root, &entry, 0)).unwrap_err();
    assert!(errors.0.iter().any(|error| error.code == Some(rock_lib::diagnostic::DiagnosticCode::Borrow) && error.primary.is_some()),
        "{:?}", errors.0.iter().map(|error| (&error.code, &error.message)).collect::<Vec<_>>());
    assert!(errors.0.iter().all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)));
}

#[test]
fn zero_sized_payload_is_consumed_without_fabricating_a_payload_drop() {
    let root = scratch();
    let entry = root.join("main.rk");
    fs::write(&entry, format!("{PROVIDER}{}", zero_sized_client())).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
}

fn zero_sized_client() -> String {
    CLIENT.replace("struct Item\n    < events: *I64\n    < tag: I64", "struct Item")
        .replace("~@take = -> self.tag", "~@take = -> 42")
        .replace("impl Dispose for Item\n    ~@dispose = !-> record self.events, 1\n", "")
        .replace("(Item\n        events: pointer\n        tag: 42), pointer", "(Item), pointer")
        .replace("events, 123", "events, 23")
}

#[test]
fn owning_open_preserves_each_payload_position_and_fixed_phantom_arguments() {
    let root = scratch();
    let entry = root.join("main.rk");
    for position in 0..3 {
        let arguments = |payload: &str, allocator: &str| {
            let mut args = vec![allocator, "I64"];
            args.insert(position, payload);
            args.join(", ")
        };
        let mut parameters = vec!["A", "B"];
        parameters.insert(position, "(T: ?KnownSize)");
        let mut generics = vec!["A", "B"];
        generics.insert(position, "T");
        let provider = PROVIDER.replace("Own (T: ?KnownSize)", &format!("Own {}", parameters.join(", ")))
            .replace("Own T\n", &format!("Own {}\n", generics.join(", ")))
            .replace("Own T where ", &format!("Own {} where ", generics.join(", ")))
            .replace("new: T -> *I64 ->", "new: T -> A -> B -> *I64 ->")
            .replace("new = value, events ->", "new = value, _, _, events ->")
            .replace("for Own\n", &format!("for Own {}\n", arguments("_", "A")))
            .replace("Own Take", &format!("Own {}", arguments("Take", "Allocator")));
        let client = CLIENT.replace("Own Take", &format!("Own {}", arguments("Take", "Allocator")))
            .replace("tag: 42), pointer", "tag: 42), (Allocator), 0, pointer");
        fs::write(&entry, format!("struct Allocator\n{provider}{client}")).unwrap();
        for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
    }
}

#[test]
fn owning_open_keeps_aligned_payload_address_distinct_from_the_released_base() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace("base = malloc (~SizeOf value)", "base = malloc (~I64Add (~SizeOf value), 16)")
        .replace("pointer: *T = base as *T", "pointer: *T = (~I64Add (base as I64), 16) as *T")
        .replace("unsafe release = _, state !-> release_memory state", "unsafe release = pointer, state !-> release_offset pointer, state");
    let helpers = r#"
extern abort: () -> ()
release_offset: *U8 -> State -> ()
release_offset = pointer, state !->
    if ~I64Ne (pointer as I64), (~I64Add (state.base as I64), 16) then abort!
    if ~I64Ne (~I64Mod (pointer as I64), 8), 0 then abort!
    release_memory state
"#;
    for early in [false, true] {
        fs::write(&entry, format!("{provider}{helpers}{}", CLIENT.replace("take_open owner, false", &format!("take_open owner, {early}")))).unwrap();
        for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
    }
}

#[test]
fn owning_erased_result_is_destroyed_once_after_payload_release_and_state_cleanup() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace("with_open: Own Take -> I64 -> I64", "with_open: Own Take -> T -> T");
    let helpers = r#"
extern abort: () -> ()
struct Result
    < events: *I64
impl Dispose for Result
    ~@dispose = !-> record self.events, 4
discard: T -> ()
discard = value !-> ()
"#;
    let client = CLIENT.replace("answer = take_open owner, false", "result = with_open owner, (Result\n        events: pointer)\n    if ~I64Ne events, 123 then abort!\n    discard result")
        .replace("events, 123 then answer", "events, 1234 then 42");
    fs::write(&entry, format!("{provider}{helpers}{client}")).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
}

#[test]
fn two_concrete_scoped_specializations_keep_separate_signatures_and_cleanup() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace("with_open: Own Take -> I64 -> I64", "with_open: Own Take -> T -> T");
    let helper = r#"
struct Result
    < events: *I64
impl Dispose for Result
    ~@dispose = !-> record self.events, 4
discard: T -> ()
discard = value !-> ()
"#;
    let client = CLIENT.replace("answer = take_open owner, false", "answer = with_open owner, 42\n    second: Own Take = Own::new (Item\n        events: pointer\n        tag: 42), pointer\n    result = with_open second, (Result\n        events: pointer)\n    discard result")
        .replace("events, 123 then answer", "events, 1231234 then answer");
    fs::write(&entry, format!("{provider}{helper}{client}")).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
}

#[test]
fn consuming_a_borrowed_payload_keeps_only_its_external_reference_alive() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace("~@take: I64", "~@take: &I64")
        .replace("take_open: Own Take -> Bool -> I64", "take_open: Own Take -> Bool -> &I64")
        .replace("if early then return 42", "if early then return value.take!");
    let client = CLIENT.replace("< tag: I64", "< tag: &I64")
        .replace("    pointer =", "    mut marker = 42\n    pointer =")
        .replace("tag: 42", "tag: &marker")
        .replace("answer = take_open owner, false", "view = take_open owner, false\n    answer = *view");
    fs::write(&entry, format!("{provider}{client}")).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
    fs::write(&entry, format!("{provider}{}", client.replace("    answer = *view", "    marker = 7\n    answer = *view"))).unwrap();
    let errors = rock_lib::compile(&config(&root, &entry, 0)).unwrap_err();
    assert!(errors.0.iter().any(|error| error.code == Some(rock_lib::diagnostic::DiagnosticCode::Borrow) && error.primary.is_some()),
        "{:?}", errors.0.iter().map(|error| (&error.code, &error.message)).collect::<Vec<_>>());
    assert!(errors.0.iter().all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)));
}

#[test]
fn source_free_state_reference_keeps_the_exact_caller_loan_after_consumption() {
    let root = scratch();
    let provider = root.join("provider.rk");
    fs::write(&provider, format!("{}\nmarker_open: Own Take -> &I64\n< marker_open = owner ->\n    open owner as Hidden, value\n        view = value.marker\n        value.take!\n        view\n", borrowed_state_provider())).unwrap();
    let object = root.join("provider.o");
    let mut products = rock_lib::compile_with_products(&Config {
        no_link: true, emit_object: Some(object), ..config(&root, &provider, 0)
    }).unwrap().products.unwrap();
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    fs::remove_file(provider).unwrap();
    let entry = root.join("main.rk");
    let client = CLIENT.replace("    pointer =", "    mut marker = 42\n    pointer =")
        .replace("tag: 42), pointer", "tag: 42), pointer, &marker")
        .replace("answer = take_open owner, false", "view = marker_open owner\n    answer = *view");
    let imports = "> owning_open::Take\n> owning_open::Dispose\n> owning_open::Own\n> owning_open::record\n> owning_open::marker_open\n";
    let cfg = |opt_level| Config {
        extern_artifacts: vec![("owning_open".into(), artifact.clone())],
        current_crate_name: Some("owning_state_client".into()), ..config(&root, &entry, opt_level)
    };
    fs::write(&entry, format!("{imports}{client}")).unwrap();
    for optimization in [0, 3] { run(&cfg(optimization)); }
    fs::write(&entry, format!("{imports}{}", client.replace("    answer = *view", "    marker = 7\n    answer = *view"))).unwrap();
    let errors = rock_lib::compile(&cfg(0)).unwrap_err();
    assert!(errors.0.iter().any(|error| error.code == Some(rock_lib::diagnostic::DiagnosticCode::Borrow) && error.primary.is_some()),
        "{:?}", errors.0.iter().map(|error| (&error.code, &error.message)).collect::<Vec<_>>());
    assert!(errors.0.iter().all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)));
}

#[test]
fn unchanged_stdlib_box_protocol_executes_owning_open_and_early_cleanup() {
    let root = scratch();
    let entry = root.join("main.rk");
    let source = r#"
> stdlib::box_type::Box
> stdlib::drop::Drop
trait Take
    ~@take: I64
struct Item
    < drops: *I64
impl Drop for Item
    ~@drop = !-> unsafe *self.drops = ~I64Add (*self.drops), 1
impl Take for Item
    ~@take = -> 42
take_open: Box Take -> Bool -> I64
take_open = owner, early ->
    open owner as Hidden, value
        if early then return 42
        value.take!
main = ->
    mut drops = 0
    owner: Box Take = Box::new (Item
        drops: (&mut drops) as *I64)
    answer = take_open owner, false
    if ~I64Eq drops, 1 then answer else 1
"#;
    let artifact = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.agent-work/artifacts/stdlib60/contracts-2/stdlib.rkca");
    for early in [false, true] {
        fs::write(&entry, source.replace("take_open owner, false", &format!("take_open owner, {early}"))).unwrap();
        for optimization in [0, 3] {
            run(&Config { extern_artifacts: vec![("stdlib".into(), artifact.clone())], ..config(&root, &entry, optimization) });
        }
    }
}
