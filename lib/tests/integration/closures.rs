use crate::support::*;

#[test]
fn test_lambdas() {
    let output = compile_example("lambdas");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10"); // double 5
    assert_eq!(lines[1], "14"); // apply double 7
    assert_eq!(lines[2], "105"); // apply (x -> x+100) 5
}

#[test]
fn test_call_argument_holes_desugar_to_lambdas() {
    let output = compile_and_run(
        r#"
struct Boxed
    < value: I64

combine = a, b, c -> a * 100 + b * 10 + c
apply = f, value -> f value
box = value -> Boxed value: value

main = ->
    fixed = 3
    middle = combine 1, _, fixed
    outer = combine _, 5, _
    nested = apply (combine 7, _, 9), 8
    projected = box _.value
    middle 2 .println!
    outer 4, 6 .println!
    nested.println!
    projected 42 .println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        ["123", "456", "789", "42"]
    );
}

#[test]
fn test_closure_capture() {
    let output = compile_and_run(
        r#"
main = ->
    base = 100
    f = x -> x + base
    f 5 .println!
    f 20 .println!
    offset = 10
    g = y -> y * 2 + offset
    g 3 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "105"); // 5 + 100
    assert_eq!(lines[1], "120"); // 20 + 100
    assert_eq!(lines[2], "16"); // 3*2 + 10
}

#[test]
fn test_closure_capture_still_works_without_codegen_rebinding() {
    let output = compile_and_run(
        r#"
main = ->
    x = 1
    f = -> x.println!
    f!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn test_higher_order_functions() {
    let output = compile_and_run(
        r#"
double = x -> x * 2
triple = x -> x * 3

apply = f, x -> f x

main = ->
    apply double, 5 .println!
    apply triple, 5 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "15");
}

#[test]
fn test_higher_order_function_with_capturing_lambda() {
    let output = compile_and_run(
        r#"
apply = f, x -> f x

main = ->
    base = 10
    apply (y -> y + base), 5 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "15");
}

#[test]
fn test_curried_function_partial_application() {
    let output = compile_and_run(
        r#"
add = a, b ~> a + b

main = ->
    inc = add 1
    inc 2 .println!
    add 1, 2 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "3");
    assert_eq!(lines[1], "3");
}

#[test]
fn test_curried_function_with_explicit_signature() {
    let output = compile_and_run(
        r#"
add: I64 -> I64 -> I64
add = a, b ~> a + b

main = ->
    inc = add 2
    inc 5 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
}

#[test]
fn test_returned_closures_own_their_captures() {
    let output = compile_and_run(
        r#"
adder = a ->
    x -> x + a

adder_via_local = a ->
    add = x -> x + a
    add

length_adder = values ->
    x -> x + values.len!

main = !->
    (adder 5) 10 .println!
    (adder_via_local 6) 10 .println!
    mut values = Vec::new!
    values.push 1
    (length_adder values) 10 .println!
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), ["15", "16", "11"]);
}

#[test]
fn test_curried_function_returning_closure() {
    let output = compile_and_run(
        r#"
compose: (T -> U) -> (U -> V) -> (T -> V)
compose = f, g ~>
    x -> g f x

adder: I64 -> I64 -> (I64 -> I64)
adder = a, b ~>
    x -> x + a + b

main = !->
    (compose (+ 1), (* 10)) 2 .println!
    then_double = compose (+ 1)
    (then_double (* 2)) 3 .println!
    (adder 1, 2) 10 .println!
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), ["30", "8", "13"]);
}

#[test]
fn test_curried_function_as_operator() {
    let output = compile_and_run(
        r#"
infix 9 |+|
|+|: I64 -> I64 -> I64
|+| = a, b ~>
    a * 10 + b

infix 9 >->
>->: (T -> U) -> (U -> V) -> (T -> V)
>-> = f, g ~>
    x -> g f x

main = !->
    (1 |+| 2).println!
    ((+ 1) >-> (* 10)) 2 .println!
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), ["12", "30"]);
}

#[test]
fn test_too_many_arguments_names_the_called_type() {
    compile_should_fail(
        r#"
inc: I64 -> I64
inc = a -> a + 1

main = !->
    (inc 1, 2).println!
"#,
        "Too many arguments: a value of type I64 cannot be called",
    );
}

#[test]
fn test_curried_function_try_uses_terminal_return_type() {
    let output = compile_and_run(
        r#"
add_result: I64 -> Result I64, I64 -> Result I64, I64
add_result = base, input ~>
    value = input?
    Result::Ok (base + value)

main = ->
    add_ten = add_result 10
    value = (add_ten (Result::Ok 2)).fold
        _ -> 0
        result -> result
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_zero_arg_method_is_value_until_bang_call() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    @get = -> self.value

main = ->
    counter = Counter
        value: 21
    get = counter.get
    get!.println!
    counter.get!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "21");
    assert_eq!(lines[1], "21");
}

#[test]
fn test_curried_method_partial_application() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    @add = x, y ~> self.value + x + y

main = ->
    counter = Counter
        value: 10
    add_one = counter.add 1
    add_one 2 .println!
    counter.add 1, 2 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "13");
    assert_eq!(lines[1], "13");
}

#[test]
fn test_mut_and_move_self_receivers() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

struct Holder
    < value: I64

impl Counter
    ^@read = -> 41

impl Holder
    ~@consume = -> 7

main = ->
    mut counter = Counter
        value: 41
    holder = Holder
        value: 7
    counter.read!.println!
    holder.consume!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "41");
    assert_eq!(lines[1], "7");
}

#[test]
fn test_recursive_functions() {
    let output = compile_and_run(
        r#"
factorial = n ->
    if n <= 1
        1
    else
        n * (factorial (n - 1))

fib = n ->
    if n <= 1
        n
    else
        (fib (n - 1)) + (fib (n - 2))

main = ->
    factorial 5 .println!
    factorial 10 .println!
    fib 10 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "120");
    assert_eq!(lines[1], "3628800");
    assert_eq!(lines[2], "55");
}
