use crate::support::*;
use std::fs;
use std::process::Command;
use std::sync::atomic::Ordering;

#[test]
fn test_default_type_arguments_annotations_and_overrides() {
    let output = compile_and_run(r#"
struct Pair A, {B = A}
    < first: A
    < second: B

type Bytes {T = U8} = Pair T

enum Choice {T = I64}
    Empty
    Value T

sum: Pair I64 -> I64
sum = pair -> pair.first + pair.second

byte: Bytes -> U8
byte = pair -> pair.second

read: Choice -> I64
read = choice -> match choice
    Choice::Empty => 0
    Choice::Value value => value

main = !->
    pair = Pair
        first: 20
        second: 22
    (sum pair).println!
    bytes = Pair
        first: 3 as U8
        second: 7 as U8
    (byte bytes).println!
    mixed: Pair I64, Bool = Pair
        first: 1
        second: true
    mixed.second.println!
    (read (Choice::Value 9)).println!
"#);
    assert_eq!(output.trim(), "42\n7\ntrue\n9");
}

#[test]
fn test_default_type_arguments_inference_and_fallback() {
    let output = compile_and_run(r#"
struct Value {T = Bool}
    < value: T

impl Value T
    new = value -> Value
        value: value

struct Phantom {T = I64}

enum Choice {T = I64}
    Empty
    Value T

use_phantom: Phantom I64 -> I64
use_phantom = value -> 42

identity = value -> value

main = !->
    value = Value
        value: 7
    value.value.println!
    constructed = Value::new 11
    constructed.value.println!
    phantom = Phantom
    (use_phantom phantom).println!
    empty = Choice::Empty
    match empty
        Choice::Empty => 8.println!
        Choice::Value value => value.println!
    through_call = identity Phantom
    (use_phantom through_call).println!
    unconstrained = identity Choice::Empty
    match unconstrained
        Choice::Empty => 12.println!
        Choice::Value value => 13.println!
"#);
    assert_eq!(output.trim(), "7\n11\n42\n8\n42\n12");
}

#[test]
fn test_default_type_arguments_trait_self_and_explicit_rhs() {
    let output = compile_and_run(r#"
trait Compare {Rhs = Self}
    @same: &Rhs -> Bool

impl Compare for I64
    @same = rhs -> *self == *rhs

impl Compare I64 for Bool
    @same = rhs -> *self

same: T -> T -> Bool where T: Compare
same = left, right -> left.same &right

main = !->
    (same 4, 4).println!
    (true.same &4).println!
"#);
    assert_eq!(output.trim(), "true\ntrue");
}

#[test]
fn test_default_type_arguments_recursive_nominal_fields() {
    compile_should_pass(r#"
struct Node {T = I64}
    value: T
    next: *Node

enum Chain {T = I64}
    End
    Link T, *Chain

main = !-> return
"#);
}

#[test]
fn test_default_type_arguments_mutually_recursive_nominal_fields() {
    let first = "struct First {T = I64}\n    value: T\n    other: *Second\n";
    for second in [
        "struct Second {T = Bool}\n    value: T\n    other: *First\n",
        "enum Second {T = Bool}\n    End\n    Link T, *First\n",
    ] {
        for declarations in [format!("{first}\n{second}"), format!("{second}\n{first}")] {
            compile_should_pass(&format!("{declarations}\nmain = !-> return\n"));
        }
    }
}

#[test]
fn test_default_type_arguments_mutual_fields_in_deleted_source_artifacts() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("models.rk"), r#"
< struct First {T = I64}
    < value: T
    < other: *Second

< enum Second {T = Bool}
    End
    Link T, *First
"#).unwrap();
    let entry = dir.join("lib.rk");
    fs::write(&entry, "< mod models\n").unwrap();
    let config = rock_lib::Config {
        entry_file: entry.clone(), output_dir: dir.clone(),
        current_crate_name: Some("recursive".to_string()),
        no_std: true, no_prelude: true, no_link: true,
        ..Default::default()
    };
    let artifact = dir.join("recursive.rkca");
    rock_lib::compile_with_products(&config).unwrap().products.unwrap()
        .write_artifact_to_path(&artifact).unwrap();
    fs::remove_file(dir.join("models.rk")).unwrap();
    fs::remove_file(&entry).unwrap();
    fs::write(&entry, r#"
> recursive::models::First
> recursive::models::Second

read: First -> I64
read = value -> value.value

main = ->
    second: Second = Second::End
    first = First
        value: 7
        other: &second as *Second
    read first
"#).unwrap();
    rock_lib::compile(&rock_lib::Config {
        entry_file: entry, current_crate_name: None,
        extern_artifacts: vec![("recursive".to_string(), artifact)],
        ..config
    }).expect("mutual defaults must survive publication and remapping");
}

#[test]
fn test_default_type_arguments_mutual_fields_across_file_modules() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("first.rk"), r#"
> second::Second
type Seed = I64
< struct First {T = Seed}
    < value: T
    < tag: Seed
    < other: *Second
"#).unwrap();
    fs::write(dir.join("second.rk"), r#"
> first::First
type Seed = Bool
< enum Second {T = Seed}
    End
    Link T, Seed, *First
"#).unwrap();
    let entry = dir.join("main.rk");
    for modules in ["mod first\nmod second", "mod second\nmod first"] {
        fs::write(&entry, format!(r#"
{modules}
> first::First
> second::Second
read: First -> I64
read = value -> value.tag

main = ->
    other: Second = Second::End
    value: First = First
        value: 7
        tag: 8
        other: &other as *Second
    read value
"#)).unwrap();
        rock_lib::compile(&rock_lib::Config {
            entry_file: entry.clone(), output_dir: dir.clone(),
            no_std: true, no_prelude: true, no_link: true,
            ..Default::default()
        }).expect("mutual field defaults and aliases must keep their module scope");
    }
}

#[test]
fn test_default_type_arguments_static_factory_fallback_and_inference() {
    assert_eq!(compile_and_run(r#"
struct Phantom {T = I64}

impl Phantom T
    new: () -> Phantom T
    new = -> Phantom

struct Value {T = Bool}
    < value: T

impl Value T
    new: T -> Value T
    new = value -> Value
        value: value

main = !->
    phantom = Phantom::new!
    explicit: Phantom Bool = Phantom::new!
    value = Value::new 42
    value.value.println!
"#), "42\n");
}

#[test]
fn test_default_type_arguments_forward_declarations_in_file_modules() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("models.rk"), r#"
read: Outer -> I64
< read = value -> value.value.value

< struct Outer {T = Inner}
    < value: T

< struct Inner {T = I64}
    < value: T
"#).unwrap();
    fs::write(dir.join("test.rk"), "mod models\nmain = !-> return\n").unwrap();
    rock_lib::compile(&test_config(dir.join("test.rk"), dir.clone())).expect("Compilation failed");
}

#[test]
fn test_default_type_arguments_reject_invalid_defaults() {
    for (source, expected) in [
        ("struct Bad {T = Missing}\nmain = !->\n    ()\n", "unknown type 'Missing' in generic default"),
        ("struct Bad {T = T}\nmain = !->\n    ()\n", "default cannot reference current or later parameter 'T'"),
        ("struct Bad {T = U}, {U = I64}\nmain = !->\n    ()\n", "default cannot reference current or later parameter 'U'"),
        ("struct Bad {T = I64}, U\nmain = !->\n    ()\n", "required parameters must precede defaulted parameters"),
        ("struct Bad {F _ = I64}\nmain = !->\n    ()\n", "has kind Type, expected Type -> Type"),
        ("type Bad {T = Bad} = T\nmain = !->\n    ()\n", "invalid or cyclic type in generic default"),
        ("type First {T = Second} = T\ntype Second {T = First} = T\nmain = !->\n    ()\n", "invalid or cyclic type in generic default"),
        ("struct First {T = Second}\nstruct Second {T = First}\nmain = !-> return\n", "default"),
        ("struct Bad {T = Self}\nmain = !->\n    ()\n", "unknown type 'Self' in generic default"),
    ] {
        compile_should_fail(source, expected);
    }
}

#[test]
fn test_default_type_arguments_declaration_order_and_constructor_kinds() {
    let output = compile_and_run(r#"
read: Outer -> I64
read = value -> value.value.value

struct Outer {T = Inner}
    < value: T

struct Inner {T = I64}
    < value: T

struct Chain A, {B = A}, {C = B}
    < value: C

struct Hold T, {F _ = Option}
    < value: F T

struct Apply (F _), T
    < value: F T

chain: Chain I64 -> I64
chain = value -> value.value

option: Hold I64 -> I64
option = value -> match value.value
    Option::Some item => item
    Option::None => 0

explicit_constructor: Apply Inner, I64 -> I64
explicit_constructor = value -> value.value.value

main = !->
    inner = Inner
        value: 42
    outer = Outer
        value: inner
    (read outer).println!
    chained = Chain
        value: 9
    (chain chained).println!
    held = Hold
        value: Option::Some 7
    (option held).println!
    applied = Apply
        value: Inner
            value: 42
    (explicit_constructor applied).println!
"#);
    assert_eq!(output.trim(), "42\n9\n7\n42");
}

#[test]
fn test_default_type_arguments_use_declaration_module_scope() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("models.rk"), r#"
struct Seed
    < value: I64

< struct Envelope {T = Seed}
    < value: T

< make = -> Envelope
    value: Seed
        value: 42
"#).unwrap();
    fs::write(dir.join("test.rk"), r#"
struct Holder {T = Envelope}
    < value: T

struct Seed
    value: Bool

> models::Envelope
> models::make

mod models

read: Holder -> I64
read = value -> value.value.value.value

main = !->
    holder = Holder
        value: make!
    (read holder).println!
"#).unwrap();
    rock_lib::compile(&test_config(dir.join("test.rk"), dir.clone())).expect("Compilation failed");
    let output = run_test_command(&mut Command::new(dir.join("test")));
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
}
