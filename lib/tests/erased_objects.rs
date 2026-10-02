use rock_lib::{Config, SourceProvider};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn raw_object_drop_invokes_payload_destructor_without_freeing_storage() {
    assert_eq!(
        execute(
            r#"
lang drop
< trait Finish
    lang method
    ~@drop: ()
trait Read
    @read: I64
struct Payload
    < counter: *I64
impl Read for Payload
    @read = -> 42
impl Finish for Payload
    ~@drop = !->
        unsafe
            current = *self.counter
            *self.counter = ~I64Add current, 1
main = ->
    mut counter = 0
    payload = Payload
        counter: (&mut counter) as *I64
    raw: *Payload = (&payload) as *Payload
    object: *Read = raw
    unsafe ~DropInPlace object
    ~Forget payload
    counter
"#
        ),
        1
    );
}

#[test]
fn raw_object_metadata_reborrow_and_payload_drop_execute() {
    assert_eq!(
        execute(
            r#"
trait Read
    @read: I64
trait Derived where Self: Read
    @extra: I64
struct Payload
    < value: I64
impl Read for Payload
    @read = -> self.value
impl Derived for Payload
    @extra = -> 0
main = ->
    payload = Payload
        value: 42
    raw: *Payload = (&payload) as *Payload
    derived: *Derived = raw
    object: *Read = derived
    size = ~SizeOfValue object
    align = ~AlignOfValue object
    if ~I64Ne size, (~SizeOf payload)
        return 1
    if ~I64Ne align, (~AlignOf payload)
        return 2
    borrowed = unsafe &*object
    if ~I64Ne (~SizeOfValue borrowed), size
        return 3
    if ~I64Ne (~AlignOfValue borrowed), align
        return 4
    result = borrowed.read!
    unsafe ~DropInPlace object
    result
"#
        ),
        42
    );
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn execute(source: &str) -> i32 {
    let root = std::env::temp_dir().join(format!(
        "rock_erased_objects_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let _cleanup = Scratch(root.clone());
    let entry = root.join("main.rk");
    let config = Config {
        entry_file: entry.clone(),
        output_dir: root.clone(),
        current_crate_name: Some("erased_objects".into()),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual {
            path: entry,
            text: source.into(),
        }],
        ..Default::default()
    };
    rock_lib::compile(&config).unwrap();
    let output = std::process::Command::new(root.join("main"))
        .output()
        .unwrap();
    output
        .status
        .code()
        .unwrap_or_else(|| panic!("{}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn generic_object_identity_moves_a_noncopy_aggregate() {
    assert_eq!(
        execute(
            r#"
trait Identity
    @identity: T -> T
struct Service
    < marker: I64
impl Identity for Service
    @identity = value -> value
struct Payload
    < first: I64
    < second: I64
    < third: I64
main = ->
    service = Service
        marker: 0
    object: &Identity = &service
    input = Payload
        first: 40
        second: 1
        third: 1
    output = object.identity input
    ~I64Add (~I64Add (output.first), (output.second)), (output.third)
"#
        ),
        42
    );
}

#[test]
fn generic_object_method_calls_an_aot_generic_forwarder() {
    assert_eq!(
        execute(
            r#"
trait Identity
    @identity: T -> T
relay: T -> T
relay = value -> value
struct Service
    < marker: I64
impl Identity for Service
    @identity = value -> relay value
struct Payload
    < value: I64
main = ->
    service = Service
        marker: 0
    object: &Identity = &service
    input = Payload
        value: 42
    output = object.identity input
    output.value
"#
        ),
        42
    );
}

#[test]
fn bounded_generic_object_method_uses_selected_dictionary() {
    assert_eq!(
        execute(
            r#"
trait Read
    @read: I64
trait Reader
    @apply: T -> I64 where T: Read
struct Service
    < marker: I64
impl Reader for Service
    @apply = value -> value.read!
struct Payload
    < value: I64
impl Read for Payload
    @read = -> self.value
main = ->
    service = Service
        marker: 0
    object: &Reader = &service
    input = Payload
        value: 42
    object.apply input
"#
        ),
        42
    );
}

// Deliberately non-stdlib names: invocation authority comes from lang IDs.
const CALLBACK_PROTOCOLS: &str = r#"
lang fn_once
< trait Consume Args, Ret
    lang output
    type Output
    lang method
    ~@consume: Args -> Ret
lang fn_mut
< trait Change Args, Ret where Self: Consume Args, Ret
    lang output
    type Output
    lang method
    ^@change: Args -> Ret
lang fn
< trait Observe Args, Ret where Self: Change Args, Ret
    lang output
    type Output
    lang method
    @observe: Args -> Ret
"#;

const CALLBACK_SERVICE: &str = r#"
< trait Callbacks
    @shared: F -> T -> R where F: Observe T, R
    @mutable: F -> I64 -> I64 where F: Change I64, I64
    @once: F -> T -> R where F: Consume T, R
    @pair: F -> T -> U -> R where F: Observe (T, U), R
relay_callback: F -> T -> R where F: Observe T, R
relay_callback = callback, value -> callback value
< struct Service
    < marker: I64
impl Callbacks for Service
    @shared = callback, value -> relay_callback callback, value
    @mutable = mut callback, value ->
        callback value
        callback value
        callback value
    @once = callback, value -> callback value
    @pair = callback, first, second -> callback first, second
as_callbacks: &Service -> &Callbacks
< as_callbacks = service ->
    object: &Callbacks = service
    object
"#;

fn callback_program(body: &str) -> String {
    format!("{CALLBACK_PROTOCOLS}\n{CALLBACK_SERVICE}\n{body}")
}

#[test]
fn erased_shared_callback_and_generic_forwarder_execute() {
    assert_eq!(
        execute(&callback_program(
            r#"
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    offset = 2
    callback = value -> ~I64Add value, offset
    object.shared callback, 40
"#
        )),
        42
    );
}

#[test]
fn erased_mutable_callback_preserves_environment_between_invocations() {
    assert_eq!(
        execute(&callback_program(
            r#"
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    mut total = 39
    callback = value ->
        total = ~I64Add total, value
        total
    object.mutable callback, 1
"#
        )),
        42
    );
}

#[test]
fn erased_callback_packs_multiple_arguments_with_descriptor_evidence() {
    assert_eq!(
        execute(&callback_program(
            r#"
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    callback = first, second -> ~I64Add first, second
    object.pair callback, 40, 2
"#
        )),
        42
    );
}

#[test]
fn erased_mutable_callback_uses_selected_non_native_impl() {
    assert_eq!(
        execute(&callback_program(
            r#"
struct Counter
    < total: I64
impl Consume I64, I64 for Counter
    type Output = I64
    ~@consume = value -> ~I64Add (self.total), value
impl Change I64, I64 for Counter
    type Output = I64
    ^@change = value ->
        self.total = ~I64Add (self.total), value
        self.total
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    counter = Counter
        total: 39
    object.mutable counter, 1
"#
        )),
        42
    );
}

const CONSUMING_CALLBACK_APP: &str = r#"
lang drop
< trait Finish
    lang method
    ~@finish: ()
struct Payload
    < counter: *I64
    < value: I64
impl Finish for Payload
    ~@finish = !->
        unsafe
            current = *self.counter
            *self.counter = ~I64Add current, 1
run: &Callbacks -> *I64 -> I64
run = object, counter ->
    payload = Payload
        counter: counter
        value: 41
    callback = -> payload
    output = object.once callback, ()
    output.value
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    mut drops = 0
    result = run object, ((&mut drops) as *I64)
    ~I64Add result, drops
"#;

#[test]
fn erased_consuming_callback_transfers_noncopy_result_and_drops_once() {
    assert_eq!(execute(&callback_program(CONSUMING_CALLBACK_APP)), 42);
}

#[test]
fn erased_callbacks_accept_later_closures_without_rebuilding_producer() {
    let root = std::env::temp_dir().join(format!(
        "rock_erased_callbacks_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let _cleanup = Scratch(root.clone());
    let producer_entry = root.join("callbacks.rk");
    std::fs::write(&producer_entry, callback_program("")).unwrap();
    let object = root.join("callbacks.o");
    let config = Config {
        entry_file: producer_entry.clone(),
        output_dir: root.clone(),
        current_crate_name: Some("callbacks".into()),
        no_std: true,
        no_prelude: true,
        no_link: true,
        emit_object: Some(object.clone()),
        ..Default::default()
    };
    let mut products = rock_lib::compile_with_products(&config)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some(PathBuf::from("callbacks.o"));
    let artifact = root.join("callbacks.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let object_before = std::fs::read(&object).unwrap();
    let artifact_before = std::fs::read(&artifact).unwrap();
    std::fs::remove_file(producer_entry).unwrap();
    let entry = root.join("main.rk");
    let sources = [
        CONSUMING_CALLBACK_APP.to_string(),
        r#"
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    mut total = 39
    callback = value ->
        total = ~I64Add total, value
        total
    object.mutable callback, 1
"#
        .into(),
        r#"
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    offset = 2
    callback = value -> ~I64Add value, offset
    object.shared callback, 40
"#
        .into(),
    ];
    for source in sources {
        std::fs::write(
            &entry,
            format!(
                "> callbacks::Service\n> callbacks::Callbacks\n> callbacks::as_callbacks\n{source}"
            ),
        )
        .unwrap();
        let caller = Config {
            entry_file: entry.clone(),
            output_dir: root.clone(),
            current_crate_name: Some("callback_caller".into()),
            no_std: true,
            no_prelude: true,
            extern_artifacts: vec![("callbacks".into(), artifact.clone())],
            ..Default::default()
        };
        rock_lib::compile(&caller).unwrap();
        let output = std::process::Command::new(root.join("main"))
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(42),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read(&object).unwrap(), object_before);
        assert_eq!(std::fs::read(&artifact).unwrap(), artifact_before);
    }
}
