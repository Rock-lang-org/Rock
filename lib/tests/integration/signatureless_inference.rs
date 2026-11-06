use crate::support::*;

#[test]
fn test_signatureless_curried_function_calls_captured_callables() {
    let output = compile_and_run(
        r#"
apply_both = f, g ~>
    g (f 1)

compose = f, g, x ~>
    g f x

main = !->
    (apply_both (+ 1), (* 10)).println!
    staged = apply_both (+ 2)
    (staged (* 10)).println!
    above_two = compose (+ 1), (n -> n > 2)
    (above_two 2).println!
    (above_two 1).println!
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        ["20", "30", "true", "false"]
    );
}

#[test]
fn test_signatureless_method_receiver_is_constrained_by_call_site() {
    let output = compile_and_run(
        r#"
run_mode = mode -> mode.as_str!

main = ->
    mode = String::from "listen"
    run_mode &mode .println!
    0
"#,
    );

    assert_eq!(output.trim(), "listen");
}

#[test]
fn test_signatureless_higher_order_helper_is_instantiated_at_two_targets() {
    let output = compile_and_run(
        r#"
apply = value, mapper -> mapper value

increment = value -> value + 1
identity_bool = value ->
    if value
        true
    else
        false

main = ->
    first = apply 41, increment
    second = apply true, identity_bool
    first.println!
    second.println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        vec!["42", "true"]
    );
}

#[test]
fn test_signatureless_curried_partial_application_propagates_nested_return() {
    compile_should_pass(
        r#"
enum Wrapped
    Value I64
    Empty

impl Wrapped
    ~@unwrap_or_zero = ->
        match self
            Wrapped::Value value => value
            Wrapped::Empty => 0

wrap = state, value ~>
    Wrapped::Value (state + value)

apply = value, mapper -> mapper value

main = ->
    wrapped = apply 42, (wrap 0)
    wrapped.unwrap_or_zero!.println!
    0
"#,
    );
}

#[test]
fn test_signatureless_callback_infers_mut_reference_parameter() {
    let output = compile_and_run(
        r#"
dispatch = mut target, callback ->
    callback target
    0

main = ->
    mut value = 0
    dispatch &mut value, (slot -> *slot = 41)
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "41");
}

#[test]
fn test_signatureless_named_callback_receives_call_site_receiver_type() {
    let output = compile_and_run(
        r#"
struct First
struct Second

impl First
    @write_all: &[U8] -> I64
    @write_all = bytes -> ~ArrayLen bytes

impl Second
    @write_all: &[U8] -> I64
    @write_all = _ -> 99

dispatch = mut target, callback ->
    bytes = [1 as U8, 2 as U8]
    callback &mut target, &bytes[..], 0

forward = output, bytes, _ -> output.write_all bytes

main = !->
    dispatch First, forward .println!
"#,
    );
    assert_eq!(output.trim(), "2");
}

#[test]
fn test_signatureless_io_callback_receiver_is_inferred_through_result_loop() {
    let output = compile_and_run(
        r#"
> std::io::Write
> std::io::stdout
> std::io::IoError

struct Source

impl Source
    @recv: &mut [U8] -> Result I64, I64
    @recv = _ -> Result::Ok 0

receive_with = stream, mut target, write ->
    mut buffer = [0; 4]
    mut total = 0
    while true
        match stream.recv &mut buffer
            Result::Ok count =>
                if count <= 0
                    return total
                else
                    match write &mut target, &buffer[..count], count
                        Result::Ok written => total = total + written
                        Result::Err _ => return total
            Result::Err _ => return total
    total

discard = _, _, count ->
    if true
        Result::Ok count
    else
        Result::Err IoError::Os 1

impl Source
    @consume = -> receive_with Source, 0, discard
    @consume_stdout = -> receive_with Source, stdout!, write_stdout

write_stdout = output, bytes, _ -> output.write_all bytes

main = !->
    source = Source
    source.consume!.println!
    source.consume_stdout!.println!
"#,
    );
    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["0", "0"]);
}

#[test]
fn test_signatureless_forward_try_and_hkt_map_are_inferred() {
    let output = compile_and_run(
        r#"
finish_connection = ->
    value = source!?
    wrapped = if true
        Result::Ok value
    else
        Result::Err 0
    wrapped.map (item -> item + 1)

source = ->
    if true
        Result::Ok 41
    else
        Result::Err 0

main = ->
    result = finish_connection!
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_signatureless_chained_hkt_operators_defer_unknown_constructor_heads() {
    let output = compile_and_run(
        r#"
source = ->
    if true
        Result::Ok 41
    else
        Result::Err 0
convert_error = value -> value == 0

finish = ->
    total = 1
    source!
        <!> convert_error
        <&> _ -> total + 41

main = ->
    finish!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "Ok(42)");
}

#[test]
fn test_signatureless_deferred_method_argument_uses_call_parameter_type() {
    let output = compile_and_run(
        r#"
struct Left
struct Right

impl Left
    @make = -> 41

impl Right
    @make = -> true

consume: I64 -> Result I64, I64
consume = value -> Result::Ok value

wrapper = value ->
    result = consume value.make!
    result

main = ->
    wrapper Left .println!
    0
"#,
    );
    assert_eq!(output.trim(), "Ok(41)");
}

#[test]
fn test_signatureless_deferred_method_constrains_argument_from_concrete_parameter() {
    let output = compile_and_run(
        r#"
struct Sink

impl Sink
    @write: I64 -> I64
    @write = value -> value

    @write_bytes: &mut [U8] -> I64
    @write_bytes = _ -> 42

struct Wrapper
    < sink: Sink

impl Wrapper
    @forward = value -> self.sink.write value

    @forward_bytes = ->
        mut bytes = [0; 4]
        self.sink.write_bytes &mut bytes

forward_unknown = sink ->
    mut bytes = [0; 4]
    sink.write_bytes &mut bytes

main = ->
    wrapper = Wrapper
        sink: Sink
    wrapper.forward 41 .println!
    wrapper.forward_bytes!.println!
    forward_unknown Sink .println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        vec!["41", "42", "42"]
    );
}

#[test]
fn test_signatureless_ambiguous_method_still_reports_diagnostic() {
    compile_should_fail(
        r#"
trait First
    @ping: () -> I64

trait Second
    @ping: () -> I64

struct Service

impl First for Service
    @ping = -> 1

impl Second for Service
    @ping = -> 2

run = value -> value.ping!

main = ->
    run Service
    0
"#,
        "Ambiguous selection",
    );
}

#[test]
fn test_signatureless_mutual_recursion_remains_monomorphic() {
    let output = compile_and_run(
        r#"
even = value ->
    if value == 0
        true
    else
        odd (value - 1)

odd = value ->
    if value == 0
        false
    else
        even (value - 1)

main = ->
    even 10 .println!
    odd 10 .println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        vec!["true", "false"]
    );
}

#[test]
fn test_signatureless_direct_recursion_registers_specialization_before_body() {
    let output = compile_and_run(
        r#"
count_down = value ->
    if value == 0
        42
    else
        count_down (value - 1)

main = ->
    count_down 3 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}
