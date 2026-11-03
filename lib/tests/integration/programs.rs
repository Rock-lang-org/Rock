use crate::support::*;

#[test]
fn test_modulo_operator() {
    let output = compile_and_run(
        r#"
main = ->
    10 % 3 .println!
    15 % 5 .println!
    7 % 2 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "0");
    assert_eq!(lines[2], "1");
}

#[test]
fn test_string_operations() {
    let output = compile_and_run(
        r#"
main = ->
    "hello".len!.println!
    "".len!.println!
    "world!".len!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "0");
    assert_eq!(lines[2], "6");
}

#[test]
fn test_owned_string_borrowed_view_matches_string_literals() {
    let output = compile_and_run(
        r#"
classify: String -> I64
classify = value ->
    match value.as_str!
        "listen" => 1
        "connect" => 2
        _ => 3

main = ->
    classify (String::from "listen") .println!
    classify (String::from "connect") .println!
    classify (String::from "other") .println!
    0
"#,
    );

    assert_eq!(output, "1\n2\n3\n");
}

#[test]
fn test_owned_string_literal_match_requests_borrowed_str_view() {
    compile_should_fail(
        r#"
main = ->
    value = String::from "listen"
    match value
        "listen" => 1
        _ => 0
"#,
        "call .as_str! before matching an owned String",
    );
}

#[test]
fn test_complex_program() {
    let output = compile_example("complex");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5"); // sqrt(3^2 + 4^2) = 5
    assert_eq!(lines[1], "55"); // sum 1..10
    assert_eq!(lines[2], "true"); // is_even(4)
    assert_eq!(lines[3], "false"); // is_even(7)
}

#[test]
fn test_extern_functions() {
    let output = compile_example("extern_test");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12"); // sqrt(144)
    assert_eq!(lines[1], "1.41421"); // sqrt(2)
}

#[test]
fn test_comprehensive_program() {
    // Tests: structs, methods, enums, match, arrays, loops, functions, recursion, strings
    let output = compile_and_run(
        r#"
struct Vec2
    < x: I64
    < y: I64

impl Vec2
    @dot = other ->
        @x * other.x + @y * other.y

    @magnitude_sq = ->
        @x * @x + @y * @y

abs = n ->
    if n < 0 then 0 - n else n

gcd = a, b ->
    if b == 0 then a
    else gcd b, (a % b)

sum_array: &[I64] -> I64
sum_array = arr ->
    total = 0
    i = 0
    while i < (~ArrayLen arr)
        total = total + arr[i]
        i = i + 1
    total

main = ->
    v1 = Vec2
        x: 3
        y: 4
    v2 = Vec2
        x: 1
        y: 2

    v1.dot v2 .println!
    v1.magnitude_sq!.println!

    gcd 48, 18 .println!

    arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    sum_array &arr .println!

    abs 42 .println!
    abs (0 - 7) .println!

    s1 = string_concat "Rock", " "
    greeting = s1.concat (String::from "lang!")
    greeting.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "11"); // dot product: 3*1 + 4*2 = 11
    assert_eq!(lines[1], "25"); // magnitude_sq: 9 + 16 = 25
    assert_eq!(lines[2], "6"); // gcd(48, 18) = 6
    assert_eq!(lines[3], "55"); // sum 1..10 = 55
    assert_eq!(lines[4], "42"); // abs(42) = 42
    assert_eq!(lines[5], "7"); // abs(-7) = 7
    assert_eq!(lines[6], "Rock lang!"); // string concat
}

#[test]
fn test_bubble_sort() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr = [5, 3, 1, 4, 2]
    i = 0
    while i < 4
        j = i + 1
        while j < 5
            if arr[i] > arr[j] then
                temp = arr[i]
                arr[i] = arr[j]
                arr[j] = temp
            j = j + 1
        i = i + 1
    k = 0
    while k < 5
        arr[k] .println!
        k = k + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2", "3", "4", "5"]);
}

#[test]
fn test_fibonacci_loop() {
    let output = compile_and_run(
        r#"
main = ->
    a = 0
    b = 1
    i = 0
    while i < 10
        a.println!
        temp = a + b
        a = b
        b = temp
        i = i + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["0", "1", "1", "2", "3", "5", "8", "13", "21", "34"]
    );
}

#[test]
fn test_nested_function_calls() {
    let output = compile_and_run(
        r#"
square = x -> x * x
double = x -> x * 2
add = x, y -> x + y

main = ->
    add (square 3), (double 5) .println!
    square (double 3) .println!
    double (square 4) .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "19"); // 9 + 10
    assert_eq!(lines[1], "36"); // (3*2)^2 = 36
    assert_eq!(lines[2], "32"); // (4^2)*2 = 32
}

#[test]
fn test_multi_string_concat() {
    let output = compile_and_run(
        r#"
main = ->
    a = "Hello"
    b = ", "
    c = "World"
    d = "!"
    s1 = string_concat a, b
    s2 = s1.concat (String::from c)
    result = s2.concat (String::from d)
    result.println!
    result.len!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "Hello, World!");
    assert_eq!(lines[1], "13");
}

#[test]
fn test_comprehensive_program_v2() {
    let output = compile_and_run(
        r#"
struct Matrix
    < a: I64
    < b: I64
    < c: I64
    < d: I64

impl Matrix
    @det = -> @a * @d - @b * @c
    @mul = other ->
        Matrix
            a: @a * other.a + @b * other.c
            b: @a * other.b + @b * other.d
            c: @c * other.a + @d * other.c
            d: @c * other.b + @d * other.d

enum Result
    Ok I64
    Err I64

safe_div = a, b ->
    if b == 0
        Result::Err 0
    else
        Result::Ok (a / b)

unwrap = r ->
    match r
        Result::Ok val => val
        Result::Err _ => 0 - 1

gcd = a, b ->
    while b > 0
        t = b
        b = a % b
        a = t
    a

is_prime = n ->
    if n < 2
        0
    else
        i = 2
        result = 1
        while i * i <= n
            if n % i == 0
                result = 0
            i = i + 1
        result

main = ->
    m1 = Matrix
        a: 1
        b: 2
        c: 3
        d: 4
    m2 = Matrix
        a: 5
        b: 6
        c: 7
        d: 8
    m1.det!.println!
    m3 = m1.mul m2
    m3.a.println!
    m3.d.println!
    r1 = safe_div 10, 3
    r2 = safe_div 10, 0
    unwrap r1 .println!
    unwrap r2 .println!
    gcd 48, 18 .println!
    is_prime 7 .println!
    is_prime 15 .println!
    is_prime 97 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "-2"); // det(1,2,3,4) = 1*4-2*3
    assert_eq!(lines[1], "19"); // m3.a = 1*5+2*7
    assert_eq!(lines[2], "50"); // m3.d = 3*6+4*8
    assert_eq!(lines[3], "3"); // 10/3
    assert_eq!(lines[4], "-1"); // safe_div 10,0 -> Err -> -1
    assert_eq!(lines[5], "6"); // gcd(48,18)
    assert_eq!(lines[6], "1"); // is_prime(7)
    assert_eq!(lines[7], "0"); // is_prime(15)
    assert_eq!(lines[8], "1"); // is_prime(97)
}

#[test]
fn test_tuples() {
    let output = compile_and_run(
        r#"
swap = a, b -> (b, a)

main = ->
    t = (1, 2, 3)
    t.0.println!
    t.1.println!
    t.2.println!
    s = swap 10, 20
    s.0.println!
    s.1.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "3");
    assert_eq!(lines[3], "20");
    assert_eq!(lines[4], "10");
}

#[test]
fn test_collatz() {
    let output = compile_and_run(
        r#"
collatz_steps = n ->
    steps = 0
    while n > 1
        if n % 2 == 0
            n = n / 2
        else
            n = n * 3 + 1
        steps = steps + 1
    steps

main = ->
    collatz_steps 1 .println!
    collatz_steps 6 .println!
    collatz_steps 27 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "0");
    assert_eq!(lines[1], "8");
    assert_eq!(lines[2], "111");
}

#[test]
fn test_struct_method_with_struct_param() {
    let output = compile_and_run(
        r#"
struct Vec2
    < x: I64
    < y: I64

impl Vec2
    @add = other ->
        Vec2
            x: @x + other.x
            y: @y + other.y
    @dot = other -> @x * other.x + @y * other.y
    @magnitude_sq = -> @x * @x + @y * @y

main = ->
    v1 = Vec2
        x: 3
        y: 4
    v2 = Vec2
        x: 1
        y: 2
    v2_for_dot = Vec2
        x: 1
        y: 2
    v3 = v1.add v2
    v3.x.println!
    v3.y.println!
    v1.dot v2_for_dot .println!
    v1.magnitude_sq!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "4");
    assert_eq!(lines[1], "6");
    assert_eq!(lines[2], "11");
    assert_eq!(lines[3], "25");
}

#[test]
fn test_string_operations_advanced() {
    let output = compile_and_run(
        r#"
repeat_str = s, n ->
    result = String::from ""
    i = 0
    while i < n
        result = result.concat (String::from s)
        i = i + 1
    result

main = ->
    stars = repeat_str "*", 5
    stars.println!
    stars.len!.println!
    num_str = int_to_string 42
    msg = (String::from "Count: ").concat (num_str.clone!)
    msg.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "*****");
    assert_eq!(lines[1], "5");
    assert_eq!(lines[2], "Count: 42");
}

#[test]
fn test_showcase_example() {
    let output = compile_example("showcase");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 14);
    assert_eq!(lines[0], "4"); // v3.x = 3+1
    assert_eq!(lines[1], "6"); // v3.y = 4+2
    assert_eq!(lines[2], "11"); // v1.dot(v2) = 3*1+4*2
    assert_eq!(lines[3], "25"); // magnitude_sq = 9+16
    assert_eq!(lines[4], "75"); // circle area = 5*5*3
    assert_eq!(lines[5], "16"); // square area = 4*4
    assert_eq!(lines[6], "3628800"); // factorial(10)
    assert_eq!(lines[7], "1"); // is_prime(97)
    assert_eq!(lines[8], "150"); // sum_array
    assert_eq!(lines[9], "5"); // array_len
    assert_eq!(lines[10], "Result: 11"); // string concat
    assert_eq!(lines[11], "42"); // abs(-42)
    assert_eq!(lines[12], "5"); // min(5,10)
    assert_eq!(lines[13], "10"); // max(5,10)
}

#[test]
fn test_builtin_functions() {
    let output = compile_and_run(
        r#"
main = ->
    abs (0 - 5) .println!
    abs 3 .println!
    min 10, 20 .println!
    max 10, 20 .println!
    min (0 - 5), 3 .println!
    max (0 - 5), 3 .println!
    s = float_to_string 3.14
    s.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "3");
    assert_eq!(lines[2], "10");
    assert_eq!(lines[3], "20");
    assert_eq!(lines[4], "-5");
    assert_eq!(lines[5], "3");
    assert_eq!(lines[6], "3.14");
}

#[test]
fn test_string_builtins() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    s = "Hello, World!"
    s.len!.println!
    string_find s, "World" .println!
    string_find s, "xyz" .println!
    string_contains s, "Hello" .println!
    string_contains s, "xyz" .println!
    buf = malloc 8
    unsafe
        *buf = 97
        *(~PtrOffset buf, 1) = 98
        *(~PtrOffset buf, 2) = 0
        *(~PtrOffset buf, 3) = 99
        *(~PtrOffset buf, 4) = 100
        *(~PtrOffset buf, 5) = 101
        *(~PtrOffset buf, 6) = 102
        *(~PtrOffset buf, 7) = 0
    bounded = unsafe str_from_utf8_unchecked (slice_from_raw_parts buf, 7)
    string_find bounded, "def" .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "13");
    assert_eq!(lines[1], "7");
    assert_eq!(lines[2], "-1");
    assert_eq!(lines[3], "1");
    assert_eq!(lines[4], "0");
    assert_eq!(lines[5], "4");
}

#[test]
fn test_closures() {
    let output = compile_and_run(
        r#"
main = ->
    x = 10
    add_x = a -> a + x
    add_x 5 .println!
    add_x 20 .println!
    mul = a, b -> a * b
    mul 3, 7 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "15"); // 5 + 10
    assert_eq!(lines[1], "30"); // 20 + 10
    assert_eq!(lines[2], "21"); // 3 * 7
}

#[test]
fn test_nested_structs() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    < y: I64

struct Rect
    < origin: Point
    < width: I64
    < height: I64

impl Rect
    @area = -> @width * @height

main = ->
    r = Rect
        origin: Point
            x: 10
            y: 20
        width: 100
        height: 50
    r.area! .println!
    r.origin.x.println!
    r.origin.y.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5000");
    assert_eq!(lines[1], "10");
    assert_eq!(lines[2], "20");
}

#[test]
fn test_method_chaining() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    @inc = ->
        Counter
            value: @value + 1
    @dec = ->
        Counter
            value: @value - 1
    @get = -> @value

main = ->
    c = Counter
        value: 0
    c = c.inc!
    c = c.inc!
    c = c.inc!
    c = c.dec!
    c.get! .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "2");
}

#[test]
fn test_enum_unwrap() {
    let output = compile_and_run(
        r#"
enum Option
    Some I64
    None

unwrap_or = opt, default ->
    match opt
        Option::Some val => val
        Option::None => default

main = ->
    a = Option::Some 42
    b = Option::None
    unwrap_or a, 0 .println!
    unwrap_or b, 99 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "99");
}

#[test]
fn test_string_equality() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    a = "hello"
    b = "hello"
    c = "world"
    if a == b
        1.println!
    else
        0.println!
    if a == c
        1.println!
    else
        0.println!
    left_buf = malloc 5
    right_buf = malloc 5
    unsafe
        *left_buf = 97
        *(~PtrOffset left_buf, 1) = 98
        *(~PtrOffset left_buf, 2) = 0
        *(~PtrOffset left_buf, 3) = 99
        *(~PtrOffset left_buf, 4) = 0
        *right_buf = 97
        *(~PtrOffset right_buf, 1) = 98
        *(~PtrOffset right_buf, 2) = 0
        *(~PtrOffset right_buf, 3) = 100
        *(~PtrOffset right_buf, 4) = 0
    left = unsafe str_from_utf8_unchecked (slice_from_raw_parts left_buf, 4)
    right = unsafe str_from_utf8_unchecked (slice_from_raw_parts right_buf, 4)
    if left == right
        1.println!
    else
        0.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1"); // "hello" == "hello"
    assert_eq!(lines[1], "0"); // "hello" != "world"
    assert_eq!(lines[2], "0"); // "ab\0c" != "ab\0d"
}

#[test]
fn test_float_arithmetic() {
    let output = compile_and_run(
        r#"
main = ->
    x = 3.14
    y = 2.0
    x + y .println!
    x * y .println!
    x - y .println!
    x / y .println!
    sqrt x .println!
    to_int x .println!
    z = to_float 42
    z.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5.14");
    assert_eq!(lines[1], "6.28");
    assert_eq!(lines[2], "1.14");
    assert_eq!(lines[3], "1.57");
    assert!(lines[4].starts_with("1.772")); // sqrt(3.14)
    assert_eq!(lines[5], "3"); // to_int(3.14) truncates
    assert_eq!(lines[6], "42"); // to_float(42) prints as 42 with %g
}

#[test]
fn test_map_array() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [1, 2, 3, 4, 5]
    i = 0
    while i < (~ArrayLen arr)
        arr[i] * 2 .println!
        i = i + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "2");
    assert_eq!(lines[1], "4");
    assert_eq!(lines[2], "6");
    assert_eq!(lines[3], "8");
    assert_eq!(lines[4], "10");
}

#[test]
fn test_for_loop_sum() {
    let output = compile_and_run(
        r#"
main = ->
    total = 0
    for x in [10, 20, 30, 40, 50]
        total = total + x
    total.println!
    0
"#,
    );
    assert_eq!(output.trim(), "150");
}

#[test]
fn test_string_parsing() {
    let output = compile_and_run(
        r#"
main = ->
    n = string_to_int "12345"
    n.println!
    n + 1 .println!
    f = string_to_float "3.14"
    f.println!
    string_to_int "-42" .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12345");
    assert_eq!(lines[1], "12346");
    assert_eq!(lines[2], "3.14");
    assert_eq!(lines[3], "-42");
}

#[test]
fn test_tuple_destructuring() {
    let output = compile_and_run(
        r#"
swap = a, b -> (b, a)

main = ->
    (x, y) = swap 3, 7
    x.println!
    y.println!
    (a, b, c) = (10, 20, 30)
    a.println!
    b.println!
    c.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "3");
    assert_eq!(lines[2], "10");
    assert_eq!(lines[3], "20");
    assert_eq!(lines[4], "30");
}

#[test]
fn test_matrix_multiply() {
    let output = compile_and_run(
        r#"
struct Matrix
    < a: I64
    < b: I64
    < c: I64
    < d: I64

impl Matrix
    @det = -> @a * @d - @b * @c
    @trace = -> @a + @d
    @mul = other ->
        Matrix
            a: @a * other.a + @b * other.c
            b: @a * other.b + @b * other.d
            c: @c * other.a + @d * other.c
            d: @c * other.b + @d * other.d

main = ->
    m1 = Matrix
        a: 1
        b: 2
        c: 3
        d: 4
    m2 = Matrix
        a: 5
        b: 6
        c: 7
        d: 8
    m1.det! .println!
    m1.trace! .println!
    m3 = m1.mul m2
    m3.a.println!
    m3.b.println!
    m3.c.println!
    m3.d.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "-2"); // det(1,2;3,4) = 1*4-2*3 = -2
    assert_eq!(lines[1], "5"); // trace = 1+4
    assert_eq!(lines[2], "19"); // m3.a = 1*5+2*7
    assert_eq!(lines[3], "22"); // m3.b = 1*6+2*8
    assert_eq!(lines[4], "43"); // m3.c = 3*5+4*7
    assert_eq!(lines[5], "50"); // m3.d = 3*6+4*8
}

#[test]
fn test_bubble_sort_algorithm() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr = [5, 3, 8, 1, 9, 2, 7, 4, 6, 10]
    len = ~ArrayLen arr
    i = 0
    while i < len
        j = 0
        while j < len - 1 - i
            if arr[j] > arr[j + 1]
                temp = arr[j]
                arr[j] = arr[j + 1]
                arr[j + 1] = temp
            j = j + 1
        i = i + 1
    k = 0
    while k < len
        arr[k].println!
        k = k + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    for i in 0..10 {
        assert_eq!(lines[i], (i + 1).to_string());
    }
}

#[test]
fn test_fibonacci() {
    let output = compile_and_run(
        r#"
fib = n ->
    if n <= 1
        n
    else
        (fib (n - 1)) + (fib (n - 2))

main = ->
    i = 0
    while i < 10
        fib i .println!
        i = i + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    let expected = ["0", "1", "1", "2", "3", "5", "8", "13", "21", "34"];
    for (i, exp) in expected.iter().enumerate() {
        assert_eq!(lines[i], *exp);
    }
}

#[test]
fn test_fizzbuzz() {
    let output = compile_and_run(
        r#"
fizzbuzz = n ->
    if n % 15 == 0
        "FizzBuzz".println!
    else if n % 3 == 0
        "Fizz".println!
    else if n % 5 == 0
        "Buzz".println!
    else
        n.println!

main = ->
    i = 1
    while i <= 15
        fizzbuzz i
        i = i + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[2], "Fizz");
    assert_eq!(lines[4], "Buzz");
    assert_eq!(lines[14], "FizzBuzz");
}

#[test]
fn test_string_escape_sequences() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    "Hello\tWorld".println!
    "Line1\nLine2".println!
    "Quote: \"hi\"".println!
    "Backslash: \\".println!
    buf = malloc 6
    unsafe
        *buf = 97
        *(~PtrOffset buf, 1) = 98
        *(~PtrOffset buf, 2) = 0
        *(~PtrOffset buf, 3) = 99
        *(~PtrOffset buf, 4) = 100
        *(~PtrOffset buf, 5) = 0
    bounded = unsafe str_from_utf8_unchecked (slice_from_raw_parts buf, 5)
    bounded.println!
    0
"#,
    );
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines[0], "Hello\tWorld");
    assert_eq!(lines[1], "Line1");
    assert_eq!(lines[2], "Line2");
    assert_eq!(lines[3], "Quote: \"hi\"");
    assert_eq!(lines[4], "Backslash: \\");
    assert_eq!(lines[5].as_bytes(), b"ab\0cd");
}

#[test]
fn test_enum_match_strings() {
    let output = compile_and_run(
        r#"
enum Color
    Red
    Green
    Blue

color_name = c ->
    match c
        Color::Red => "red"
        Color::Green => "green"
        Color::Blue => "blue"

main = ->
    color_name (Color::Red) .println!
    color_name (Color::Green) .println!
    color_name (Color::Blue) .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "red");
    assert_eq!(lines[1], "green");
    assert_eq!(lines[2], "blue");
}

#[test]
fn test_sieve_of_eratosthenes() {
    let output = compile_and_run(
        r#"
main = ->
    limit = 30
    mut sieve = Vec::new!
    i = 0
    while i < limit
        sieve.push 1
        i = i + 1
    sieve.set 0, 0
    sieve.set 1, 0
    i = 2
    while i * i < limit
        is_prime = match (sieve.get i)
            Option::Some val => *val
            Option::None => 0
        if is_prime == 1
            j = i * i
            while j < limit
                sieve.set j, 0
                j = j + i
        i = i + 1
    count = 0
    i = 2
    while i < limit
        is_prime = match (sieve.get i)
            Option::Some val => *val
            Option::None => 0
        if is_prime == 1
            count = count + 1
        i = i + 1
    count.println!
    0
"#,
    );
    assert_eq!(output.trim(), "10"); // primes below 30: 2,3,5,7,11,13,17,19,23,29
}

#[test]
fn test_negative_numbers() {
    let output = compile_and_run(
        r#"
main = ->
    x = -42
    x.println!
    y = 10
    z = -y
    z.println!
    abs (0 - 7) .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "-42");
    assert_eq!(lines[1], "-10");
    assert_eq!(lines[2], "7");
}

#[test]
fn test_doc_casts_negative_branches_and_unit_values() {
    let output = compile_and_run(
        r#"
average = total, count -> total as F64 / count as F64

classify = value ->
    if value <= 0
        -1
    else
        if value == 1
            -2
        else
            value

main = !->
    ()
    average 9, 2 .println!
    classify 0 .println!
    classify 1 .println!
    classify 3 .println!
"#,
    );
    assert_eq!(output.trim(), "4.5\n-1\n-2\n3");
}

#[test]
fn test_gcd_lcm() {
    let output = compile_and_run(
        r#"
gcd = a, b ->
    if b == 0
        a
    else
        gcd b, (a % b)

lcm = a, b -> a / (gcd a, b) * b

main = ->
    gcd 48, 18 .println!
    lcm 12, 18 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "6");
    assert_eq!(lines[1], "36");
}

#[test]
fn test_break_continue() {
    let output = compile_and_run(
        r#"
main = ->
    i = 0
    while i < 100
        if i == 42
            break
        i = i + 1
    i.println!

    count = 0
    j = 0
    while j < 20
        j = j + 1
        if j % 3 == 0
            continue
        count = count + 1
    count.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "14");
}

#[test]
fn test_string_building() {
    let output = compile_and_run(
        r#"
main = ->
    result = String::from ""
    i = 1
    while i <= 5
        if i > 1
            result = result.concat (String::from ", ")
        num_str = int_to_string i
        result = result.concat (num_str.clone!)
        i = i + 1
    result.println!
    0
"#,
    );
    assert_eq!(output.trim(), "1, 2, 3, 4, 5");
}

#[test]
fn test_trait_with_defaults() {
    let output = compile_and_run(
        r#"
struct Dog
    < name: I64

struct Cat
    < name: I64

trait Animal
    @speak = -> 0
    @legs = -> 4

impl Animal for Dog
    @speak = -> 1

impl Animal for Cat
    @speak = -> 2

main = ->
    d = Dog
        name: 1
    c = Cat
        name: 2
    d.speak!.println!
    c.speak!.println!
    d.legs!.println!
    c.legs!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "4");
    assert_eq!(lines[3], "4");
}

#[test]
fn test_string_concat_builtin() {
    let output = compile_and_run(
        r#"
main = ->
    a = "Hello, "
    b = "World!"
    c = string_concat a, b
    c.println!
    d = (string_concat "foo", "bar").concat (String::from "baz")
    d.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "Hello, World!");
    assert_eq!(lines[1], "foobarbaz");
}
