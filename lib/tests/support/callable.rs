use super::{compile_and_run, compile_and_run_without_stdlib, compile_should_fail};

#[test]
fn callable_generic_mut_repeated_and_native_reborrow() {
    let output = compile_and_run(
        r#"
twice: F -> I64 -> I64 where F: FnMut I64, I64
twice = mut function, value ->
    function value
    function value

main = !->
    mut count = 0
    mut callback = value ->
        count = count + 1
        value + count
    callback 10 .println!
    callback 10 .println!
    twice &mut callback, 10 .println!
    callback 10 .println!
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["11", "12", "14", "15"]);
}

#[test]
fn callable_struct_dispatch_and_hierarchy() {
    let output = compile_and_run(
        r#"
struct Add
    < amount: I64

impl FnOnce I64, I64 for Add
    type Output = I64
    ~@call_once = value -> value + self.amount
impl FnMut I64, I64 for Add
    type Output = I64
    ^@call_mut = value -> value + self.amount
impl Fn I64, I64 for Add
    type Output = I64
    @call = value -> value + self.amount

once: F -> I64 where F: FnOnce I64, I64
once = function -> function 10
shared: F -> I64 where F: Fn I64, I64
shared = function -> once function
borrowed: F -> I64 where F: Fn I64, I64
borrowed = function -> once &function

main = !->
    add = Add
        amount: 3
    add 1 .println!
    add 2 .println!
    borrowed add .println!
    shared (Add
        amount: 7) .println!
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["4", "5", "13", "17"]);
}

#[test]
fn callable_zero_multi_and_tuple_arguments() {
    let output = compile_and_run(
        r#"
zero: F -> I64 where F: FnOnce (), I64
zero = function -> function!
two: F -> I64 where F: FnOnce (I64, I64), I64
two = function -> function 4, 7
tuple: F -> I64 where F: FnOnce (I64, I64), I64
tuple = function -> function (4, 7)

main = !->
    zero (-> 9) .println!
    two (a, b -> a + b) .println!
    tuple (pair -> pair.0 * pair.1) .println!
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["9", "11", "28"]);
}

#[test]
fn callable_mut_requires_mutable_binding() {
    compile_should_fail(
        r#"
main = !->
    mut count = 0
    callback = !-> count = count + 1
    callback!
"#,
        "immutable binding",
    );
}

#[test]
fn callable_once_cannot_be_called_twice() {
    compile_should_fail(
        r#"
main = !->
    value = String::from_str "owned"
    consume = -> value
    consume!
    consume!
"#,
        "moved",
    );
}

#[test]
fn callable_non_callable_reports_source_error() {
    compile_should_fail(
        r#"
main = !->
    value = 42
    value 1
"#,
        "not callable",
    );
}

#[test]
fn callable_inferred_parameter_accepts_struct() {
    let output = compile_and_run(
        r#"
struct Task
impl FnOnce (), I64 for Task
    type Output = I64
    ~@call_once = _ -> 37
apply = function -> function!
main = !-> apply Task .println!
"#,
    );
    assert_eq!(output.trim(), "37");
}

#[test]
fn callable_mutable_reference_to_struct_is_reborrowed() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64
impl FnOnce (), I64 for Counter
    type Output = I64
    ~@call_once = _ -> self.value
impl FnMut (), I64 for Counter
    type Output = I64
    ^@call_mut = _ ->
        self.value = self.value + 1
        self.value
twice: F -> I64 where F: FnMut (), I64
twice = mut function ->
    function!
    function!
main = !->
    mut counter = Counter
        value: 0
    twice &mut counter .println!
    counter!.println!
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["2", "3"]);
}

#[test]
fn callable_shared_reference_cannot_invoke_mutable_closure() {
    compile_should_fail(
        r#"
apply: F -> () where F: FnMut (), ()
apply = mut function !-> function!
main = !->
    mut count = 0
    mut callback = !-> count = count + 1
    apply &callback
"#,
        "does not implement",
    );
}

#[test]
fn callable_once_moves_and_drops_captures_once() {
    let output = compile_and_run(
        r#"
struct Owned
    < value: I64
impl Drop for Owned
    ~@drop = !-> self.value.println!
apply: F -> Owned where F: FnOnce (), Owned
apply = function -> function!
main = !->
    value = Owned
        value: 42
    consume = -> value
    result = apply consume
    result.value.println!
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["42", "42"]);
}

#[test]
fn callable_uses_marked_member_identity_without_stdlib() {
    let status = compile_and_run_without_stdlib(
        r#"
lang fn_once
< trait Run Args, Ret
    lang output
    type Answer
    lang method
    ~@invoke: Args -> Ret
struct Task
impl Run I64, I64 for Task
    type Answer = I64
    ~@invoke = value -> ~I64Add value, 1
apply: F -> I64 where F: Run I64, I64
apply = function -> function 6
main = -> ~I64Add (apply Task), (apply (value -> value))
"#,
    );
    assert_eq!(status, 13);
}

#[test]
fn callable_explicit_methods_support_native_arities_and_references() {
    let output = compile_and_run(
        r#"
main = !->
    add = a, b -> a + b
    add.call (2, 3) .println!
    nothing = -> 7
    nothing.call () .println!
    mut borrowed = &add
    borrowed.call_mut (4, 5) .println!
    borrowed.call_once (6, 7) .println!
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["5", "7", "9", "13"]);
}

#[test]
fn callable_mut_does_not_fall_back_to_consuming_immutable_struct() {
    compile_should_fail(
        r#"
struct Task
impl FnOnce (), () for Task
    type Output = ()
    ~@call_once = _ !-> ()
impl FnMut (), () for Task
    type Output = ()
    ^@call_mut = _ !-> ()
main = !->
    task = Task
    task!
"#,
        "mutable receiver",
    );
}

#[test]
fn callable_fn_rejects_mutating_closure() {
    compile_should_fail(
        r#"
apply: F -> () where F: Fn (), ()
apply = function !-> function!
main = !->
    mut count = 0
    callback = !-> count = count + 1
    apply callback
"#,
        "does not implement",
    );
}

#[test]
fn callable_fnmut_requires_fnonce_supertrait() {
    compile_should_fail(
        r#"
struct Task
impl FnMut (), () for Task
    type Output = ()
    ^@call_mut = _ !-> ()
main = !-> ()
"#,
        "supertrait",
    );
}

#[test]
fn callable_callee_and_argument_pack_are_evaluated_once_in_order() {
    let output = compile_and_run(
        r#"
struct Task
impl FnOnce (I64, I64), I64 for Task
    type Output = I64
    ~@call_once = pair -> pair.0 + pair.1
make = ->
    1.println!
    Task
argument = value ->
    value.println!
    value
main = !->
    (make!) (argument 2), (argument 3) .println!
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["1", "2", "3", "5"]);
}

#[test]
fn callable_stdlib_artifact_callback_uses_ordinary_calls() {
    let output = compile_and_run(
        r#"
main = !->
    mut values = Vec::new!
    values.push 10
    values.push 20
    mut count = 0
    mapped = values.map value ->
        count = count + 1
        value + count
    mapped[0].println!
    mapped[1].println!
    count.println!
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["11", "22", "2"]);
}

#[test]
fn callable_mutable_field_requires_mutable_owner() {
    compile_should_fail(
        r#"
struct Holder F
    < callback: F
main = !->
    mut count = 0
    holder = Holder
        callback: !-> count = count + 1
    holder.callback!
"#,
        "immutable binding",
    );
}

#[test]
fn callable_inferred_field_selects_callable_protocol() {
    let output = compile_and_run(
        r#"
struct Task
impl FnOnce (), I64 for Task
    type Output = I64
    ~@call_once = _ -> 19
struct Holder F
    < callback: F
run = holder -> holder.callback!
main = !->
    holder = Holder
        callback: Task
    run holder .println!
"#,
    );
    assert_eq!(output.trim(), "19");
}
