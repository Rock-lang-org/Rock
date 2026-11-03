use crate::support::*;

#[test]
fn test_std_vec_eager_map_move_only_preserves_order() {
    let output = compile_and_run(
        r#"
string_len: String -> I64
string_len = value -> value.len!

print_value: I64 -> ()
print_value = value ->
    value.println!
    return

main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from "a")
    values.push (String::from "bbb")
    mapped: Vec I64 = values.map string_len
    mapped.for_each_owned print_value
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["1", "3"]);
}

#[test]
fn test_std_for_each_ranges_and_mutable_capture() {
    let output = compile_and_run(
        r#"
main = !->
    mut digits: I64 = 0
    for_each 1..=3, number !->
        digits = digits * 10 + number
    digits.println!
    for_each 4..6, (!.println!)
    for_each 5..5, (!.println!)
    for_each 5..3, (!.println!)
    for_each 5..=3, (!.println!)
    for_each 7..=7, (!.println!)
    for_each 9223372036854775806..=9223372036854775807, (!.println!)
    mut visits: I64 = 0
    for_each 0..100000, _ !->
        visits = visits + 1
    visits.println!
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "123",
            "4",
            "5",
            "7",
            "9223372036854775806",
            "9223372036854775807",
            "100000"
        ]
    );
}

#[test]
fn test_std_for_each_owned_and_borrowed_collections() {
    let output = compile_and_run(
        r#"
main = !->
    mut words: Vec String = Vec::new!
    words.push String::from "first"
    words.push String::from "second"
    for_each &words, (!.println!)
    words.len!.println!
    for_each words, (!.println!)

    values = [3, 4, 5]
    view: &[I64] = &values[..]
    for_each view, value !-> (*value).println!
    values[0].println!
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["first", "second", "2", "first", "second", "3", "4", "5", "3"]
    );
}

#[test]
fn test_std_for_each_book_fizzbuzz() {
    let source = include_str!("../../../docs/examples/fizzbuzz.rk");
    let chapter = include_str!("../../../docs/src/getting-started/first-project.md");
    let listing = chapter
        .split_once("```rock\n")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0;
    assert_eq!(listing.trim(), source.trim());
    let expected = (1..=30)
        .map(|number| match (number % 3, number % 5) {
            (0, 0) => "FizzBuzz".to_string(),
            (0, _) => "Fizz".to_string(),
            (_, 0) => "Buzz".to_string(),
            _ => number.to_string(),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        compile_and_run(source).lines().collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn test_std_for_each_hkt_foldable_carriers() {
    let output = compile_and_run(
        r#"
struct One T
    < value: T

impl Foldable for One
    try_fold = mut step, initial, value ->
        step.call_mut (initial, value.value)

main = !->
    for_each (Option::Some 7), (!.println!)
    absent: Option I64 = Option::None
    for_each absent, (!.println!)
    success: Result I64, I64 = Result::Ok 9
    failure: Result I64, I64 = Result::Err 99
    for_each success, (!.println!)
    for_each failure, (!.println!)
    one = One
        value: String::from "custom foldable"
    for_each one, (!.println!)
"#,
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["7", "9", "custom foldable"]
    );
}

#[test]
fn test_unit_receiver_sections_discard_results_and_capture_arguments() {
    let output = compile_and_run(
        r#"
struct Number
    < value: I64

impl Number
    @plus: I64 -> I64
    @plus = amount -> self.value + amount

main = !->
    for_each 1..=3, (!.println!)
    offset = 5
    value = Number
        value: 7
    for_each (Option::Some value), (!.plus offset .println!)
    suffix = String::from "!"
    for_each (Option::Some (String::from "hello")), (!.concat suffix.clone! .println!)
    read: Number -> I64 = (.plus offset)
    ignore: Number -> () = (!.plus offset)
    other = Number
        value: 10
    read other .println!
    ignored = Number
        value: 20
    ignore ignored
"#,
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        ["1", "2", "3", "12", "hello!", "15"]
    );
}

#[test]
fn test_std_for_each_rejects_open_ended_ranges() {
    for range in ["(..)", "(1..)", "(..3)", "(..=3)"] {
        let (output, success) = compile_and_run_with_status(&format!(
            "main = !->\n    for_each {range}, number !-> number.println!\n"
        ));
        assert!(!success, "range {range} should fail");
        assert_eq!(output.trim(), "for_each requires a bounded range");
    }
}

#[test]
fn test_std_for_each_drops_owned_items_and_callback_once() {
    let output = compile_and_run(
        r#"
struct Item
    < id: I64

impl Drop for Item
    ~@drop = !-> self.id.println!

struct Counter
    < total: I64

impl FnOnce Item, () for Counter
    type Output = ()
    ~@call_once = item !-> item.id.println!

impl FnMut Item, () for Counter
    type Output = ()
    ^@call_mut = item !->
        self.total = self.total + item.id

impl Drop for Counter
    ~@drop = !-> self.total.println!

main = !->
    mut items: Vec Item = Vec::new!
    first = Item
        id: 2
    second = Item
        id: 3
    items.push first
    items.push second
    counter = Counter
        total: 0
    for_each items, counter
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["2", "3", "5"]);
}

#[test]
fn test_std_for_each_consumes_owned_source() {
    compile_should_fail(
        r#"
main = !->
    mut values: Vec I64 = Vec::new!
    values.push 1
    for_each values, value !-> value.println!
    values.len!.println!
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_std_for_each_rejects_non_unit_action() {
    compile_should_fail(
        r#"
main = !->
    for_each 1..=3, value -> value
"#,
        "does not implement trait",
    );
}

#[test]
fn test_std_vec_map_ref_preserves_source() {
    let output = compile_and_run(
        r#"
string_len: &String -> I64
string_len = value -> value.len!

print_value: I64 -> ()
print_value = value ->
    value.println!
    return

main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from "aa")
    values.push (String::from "bbbb")
    mapped: Vec I64 = values.map_ref string_len
    mapped.for_each_owned print_value
    values.len!.println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["2", "4", "2"]);
}

#[test]
fn test_std_vec_map_ref_contextually_types_closure_parameter() {
    let output = compile_and_run(
        r#"
print_value: I64 -> ()
print_value = value ->
    value.println!
    return

main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from "aa")
    values.push (String::from "bbbb")
    mapped: Vec I64 = values.map_ref (value -> value.len!)
    mapped.for_each_owned print_value
    values.len!.println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["2", "4", "2"]);
}

#[test]
fn test_std_vec_try_for_each_short_circuits() {
    let output = compile_and_run(
        r#"
visit: &I64 -> Result (), I64
visit = value ->
    item = *value
    item.println!
    if item == 2
        Result::Err 9
    else
        unit: () = make_unit!
        Result::Ok unit

make_unit: () -> ()
make_unit = -> return

main = ->
    mut values: Vec I64 = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    result: Result (), I64 = values.try_for_each visit
    match result
        Result::Ok unit => 0.println!
        Result::Err error => error.println!
    values.len!.println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["1", "2", "9", "3"]);
}

#[test]
fn test_std_vec_eager_operations_preserve_order_and_are_stack_safe() {
    let output = compile_and_run(
        r#"
double: I64 -> I64
double = value -> value * 2

print_ref: &I64 -> ()
print_ref = value ->
    *value .println!
    return

print_owned: I64 -> ()
print_owned = value ->
    value.println!
    return

is_odd: &I64 -> Bool
is_odd = value -> *value % 2 == 1

keep_even: I64 -> Option I64
keep_even = value ->
    if value % 2 == 0
        Option::Some (value * 10)
    else
        Option::None

try_double: I64 -> Result I64, I64
try_double = value ->
    value.println!
    if value == 3
        Result::Err 99
    else
        Result::Ok (value * 2)

main = ->
    mut empty: Vec I64 = Vec::new!
    mapped_empty: Vec I64 = empty.map double
    mapped_empty.len!.println!

    mut single: Vec I64 = Vec::new!
    single.push 7
    mapped_single: Vec I64 = single.map double
    mapped_single.for_each print_ref

    mut borrowed: Vec I64 = Vec::new!
    borrowed.push 1
    borrowed.push 2
    borrowed.push 3
    borrowed.for_each print_ref
    borrowed.len!.println!

    mut stateful: Vec I64 = Vec::new!
    stateful.push 1
    stateful.push 2
    stateful.push 3
    total = 0
    stateful.for_each_owned (value -> total = total + value)
    total.println!

    mut owned: Vec I64 = Vec::new!
    owned.push 4
    owned.push 5
    owned.for_each_owned print_owned

    mut retained: Vec I64 = Vec::new!
    retained.push 1
    retained.push 2
    retained.push 3
    retained.push 4
    retained.retain is_odd
    retained.for_each print_ref

    mut filtered: Vec I64 = Vec::new!
    filtered.push 1
    filtered.push 2
    filtered.push 3
    filtered.push 4
    kept: Vec I64 = filtered.filter is_odd
    kept.for_each print_ref

    mut filter_mapped: Vec I64 = Vec::new!
    filter_mapped.push 1
    filter_mapped.push 2
    filter_mapped.push 3
    filter_mapped.push 4
    transformed: Vec I64 = filter_mapped.filter_map keep_even
    transformed.for_each print_ref

    mut fallible: Vec I64 = Vec::new!
    fallible.push 1
    fallible.push 2
    fallible.push 3
    fallible.push 4
    match (fallible.try_map try_double)
        Result::Ok values => values.len!.println!
        Result::Err error => error.println!

    mut large: Vec I64 = Vec::new!
    i = 0
    while i < 10000
        large.push i
        i = i + 1
    doubled: Vec I64 = large.map double
    doubled.len!.println!
    0
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "0", "14", "1", "2", "3", "3", "6", "4", "5", "1", "3", "1", "3", "20", "40", "1", "2",
            "3", "99", "10000",
        ]
    );
}

#[test]
fn test_std_vec_eager_operations_drop_inputs_and_outputs_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

transform: Tracked -> Tracked
transform = value ->
    Tracked
        value: value.value + 10

main = ->
    mut values: Vec Tracked = Vec::new!
    values.push (Tracked
        value: 1)
    values.push (Tracked
        value: 2)
    values.push (Tracked
        value: 3)
    mapped: Vec Tracked = values.map transform
    mapped.len!.println!
    0
"#,
    );

    let mut lines = output.lines().collect::<Vec<_>>();
    lines.sort_unstable();
    assert_eq!(lines, vec!["1", "11", "12", "13", "2", "3", "3"]);
}

#[test]
fn test_std_vec_filtering_and_try_map_drop_each_value_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

is_odd: &Tracked -> Bool
is_odd = value -> value.value % 2 == 1

filter_transform: Tracked -> Option Tracked
filter_transform = value ->
    if value.value % 2 == 1
        Option::Some (Tracked
            value: value.value + 100)
    else
        Option::None

try_transform: Tracked -> Result Tracked, I64
try_transform = value ->
    0 - value.value .println!
    if value.value == 502
        Result::Err 9
    else
        Result::Ok (Tracked
            value: value.value + 100)

main = ->
    mut retained: Vec Tracked = Vec::new!
    retained.push (Tracked
        value: 101)
    retained.push (Tracked
        value: 102)
    retained.push (Tracked
        value: 103)
    retained.retain is_odd

    mut filtered: Vec Tracked = Vec::new!
    filtered.push (Tracked
        value: 201)
    filtered.push (Tracked
        value: 202)
    filtered.push (Tracked
        value: 203)
    kept: Vec Tracked = filtered.filter is_odd

    mut filter_mapped: Vec Tracked = Vec::new!
    filter_mapped.push (Tracked
        value: 301)
    filter_mapped.push (Tracked
        value: 302)
    filter_mapped.push (Tracked
        value: 303)
    transformed: Vec Tracked = filter_mapped.filter_map filter_transform

    mut fallible: Vec Tracked = Vec::new!
    fallible.push (Tracked
        value: 501)
    fallible.push (Tracked
        value: 502)
    fallible.push (Tracked
        value: 503)
    fallible.push (Tracked
        value: 504)
    match (fallible.try_map try_transform)
        Result::Ok values => values.len!.println!
        Result::Err error => error.println!
    0
"#,
    );

    let mut values = output
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect::<Vec<_>>();
    values.sort_unstable();
    assert_eq!(
        values,
        vec![
            -502, -501, 9, 101, 102, 103, 201, 202, 203, 301, 302, 303, 401, 403, 501, 502, 503,
            504, 601,
        ]
    );
}
