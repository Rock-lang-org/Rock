use crate::support::*;

#[test]
fn test_method_call_mut_receiver_requires_mutable_binding() {
    compile_should_fail(
        r#"
main = ->
    k = Lol::new!
    k.mdr!
    0

struct Lol
    mdr: I64

impl Lol
    new = ->
        Lol
            mdr: 8
    ^@mdr = -> 2
"#,
        "mutable receiver",
    );
}

#[test]
fn immutable_binding_cannot_call_mut_receiver_method() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@inc: ()
    ^@inc = ->
        self.value = self.value + 1
        return

main = ->
    c = Counter
        value: 0
    c.inc!
    0
"#,
        "mutable receiver",
    );
}

#[test]
fn shared_reference_cannot_call_mut_receiver_method() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@inc: ()
    ^@inc = ->
        self.value = self.value + 1
        return

use_shared: &Counter -> ()
use_shared = c ->
    c.inc!
    return

main = -> 0
"#,
        "mutable receiver",
    );
}

#[test]
fn immutable_field_cannot_call_mut_receiver_method() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@inc: ()
    ^@inc = ->
        self.value = self.value + 1
        return

struct Wrapper
    < counter: Counter

main = ->
    w = Wrapper
        counter: Counter
            value: 0
    w.counter.inc!
    0
"#,
        "mutable receiver",
    );
}

#[test]
fn deferred_method_section_reports_mutable_receiver_requirement() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@advance: Result I64, I64
    ^@advance = ->
        self.value = self.value + 1
        Result::Ok self.value

make: () -> Result Counter, I64
make = ->
    Result::Ok (Counter
        value: 0)

main = ->
    result = make! >>= (.advance!)
    0
"#,
        "Cannot call mutable receiver method 'advance' without a mutable receiver",
    );
}

#[test]
fn deferred_method_section_autorefs_shared_receiver() {
    let output = compile_and_run(
        r#"
> std::control::Result

struct Counter
    < value: I64

impl Counter
    @read: Result I64, I64
    @read = -> Result::Ok self.value

make: () -> Result Counter, I64
make = -> Result::Ok (Counter
    value: 41)

main = ->
    result: Result I64, I64 = make! >>= (.read!)
    result.println!
    0
"#,
    );
    assert_eq!(output.trim(), "Ok(41)");
}

#[test]
fn deferred_method_section_uses_autoderef_receiver() {
    let output = compile_and_run(
        r#"
struct Inner
    < value: I64

impl Inner
    @read = -> self.value

struct Box T
    < value: T

impl Deref for Box T
    type Target = T
    @* = -> &@value

make: () -> Box Inner
make = -> Box
    value: Inner
        value: 42

main = ->
    result = make! |> (.read!)
    result.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn deferred_method_value_execution_preserves_selected_method() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    @read = -> self.value

identity = value -> value

main = ->
    counter = identity (Counter
        value: 43)
    reader = counter.read
    value: I64 = reader!
    value.println!
    0
"#,
    );
    assert_eq!(output.trim(), "43");
}

#[test]
fn deferred_bounded_generic_method_rejects_invalid_argument() {
    compile_should_fail(
        r#"
trait Marker

struct Box

impl Box
    @requires_marker: T -> () where T: Marker
    @requires_marker = _ -> return

make: () -> Result Box, I64
make = -> Result::Ok Box

main = ->
    result = make! >>= (.requires_marker 1)
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn deferred_unknown_method_section_remains_unknown_field() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

make: () -> Result Counter, I64
make = ->
    Result::Ok (Counter
        value: 0)

main = ->
    result = make! >>= (.missing!)
    0
"#,
        "Unknown field 'missing' on struct 'Counter'",
    );
}

#[test]
fn mutable_binding_can_call_mut_receiver_method() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@inc: ()
    ^@inc = ->
        self.value = self.value + 1
        return

    @get: I64
    @get = -> self.value

main = ->
    mut c = Counter
        value: 1
    c.inc!
    c.get! .println!
    0
"#,
    );
    assert_eq!(output.trim(), "2");
}

#[test]
fn trait_bound_mut_receiver_call_autorefs_mut_reference() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

trait Touch
    ^@touch: I64

impl Touch for Counter
    ^@touch = ->
        self.value = self.value + 1
        self.value

poke: &mut T -> I64 where T: Touch
poke = value -> value.touch!

main = ->
    mut counter = Counter
        value: 41
    poke (&mut counter) .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn trait_bound_shared_receiver_on_ref_does_not_double_borrow() {
    let output = compile_and_run(
        r#"
trait Read
    @read: I64

impl Read for I64
    @read = -> *self

read_ref: &T -> I64 where T: Read
read_ref = value -> value.read!

main = ->
    value = 42
    read_ref &value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_invalid_receiver_sequence_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    k = Lol::new!
    k.mdr!.println!
    k.ta!.println!
    k.mdr!.println!
    0

struct Lol
    mdr: I64

impl Lol
    new = ->
        Lol
            mdr: 8
    ^@mdr = -> 2
    ~@ta = -> 1
"#,
        "mutable receiver",
    );
}

#[test]
fn test_method_signature_omits_shared_self_receiver() {
    let (output, success) = compile_and_run_with_status(
        r#"
struct Counter
    < value: I64

impl Counter
    @value: I64
    @value = -> self.value

main = ->
    c = Counter
        value: 7
    c.value! .println!
    0
"#,
    );

    assert!(success);
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_method_signature_omits_mut_and_move_self_receiver() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@set: I64 -> ()
    ^@set = value ->
        self.value = value
        return

    ~@take: I64
    ~@take = -> self.value

main = ->
    mut c = Counter
        value: 1
    c.set 9
    c.take! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_trait_method_signature_omits_self_receiver() {
    let output = compile_and_run(
        r#"
trait Value
    @value: I64

struct Boxed
    < value: I64

impl Value for Boxed
    @value = -> @value

main = ->
    b = Boxed
        value: 11
    b.value! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "11");
}

#[test]
fn test_trait_method_signature_implicit_self_preserves_associated_output() {
    let output = compile_and_run(
        r#"
trait Projector
    type Output
    @project: Self::Output -> Self::Output

struct Id

impl Projector for Id
    type Output = I64
    @project = value -> value

main = ->
    id = Id
    id.project 13 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "13");
}

#[test]
fn test_bang_rejects_method_with_required_argument() {
    compile_should_fail(
        r#"
struct Counter

impl Counter
    @set: I64 -> I64
    @set = value -> value

main = ->
    counter = Counter
    counter.set! 9
    0
"#,
        "expected 1 args, got 0",
    );
}

#[test]
fn test_unsafe_trait_signature_bound_dispatch_requires_unsafe() {
    compile_should_fail(
        r#"
trait Risky
    unsafe @read: I64

struct Secret
    < value: I64

impl Risky for Secret
    @read = -> @value

read_value: T -> I64 where T: Risky
read_value = value -> value.read!

main = ->
    secret = Secret
        value: 7
    value = read_value secret
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_trait_signature_current_trait_dispatch_requires_unsafe() {
    compile_should_fail(
        r#"
trait Risky
    unsafe @read: I64
    @read_safely: I64
    @read_safely = -> self.read!

struct Secret
    < value: I64

impl Risky for Secret
    @read = -> @value

main = ->
    secret = Secret
        value: 7
    value = secret.read_safely!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_method_signature_allows_explicit_self_operand_after_implicit_receiver() {
    let output = compile_and_run(
        r#"
trait Same
    @same: Self -> Bool

struct Token
    < value: I64

impl Same for Token
    @same = other -> self.value == other.value

main = ->
    a = Token
        value: 4
    b = Token
        value: 4
    a.same b .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}
