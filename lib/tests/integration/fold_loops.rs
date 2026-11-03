use crate::support::*;

#[test]
fn test_fold_loop_utf8_traversal() {
    let output = compile_and_run(
        r#"
main = !->
    text = String::from "aé🦀"
    for_each text.chars!, (!.println!)
    for ch in text.chars!
        ch.println!
    for (offset, ch) in text.char_indices!
        offset.println!
        ch.println!
    text.println!
"#,
    );
    assert_eq!(output, "a\né\n🦀\na\né\n🦀\n0\na\n1\né\n3\n🦀\naé🦀\n");
}

#[test]
fn test_fold_loop_control_flow() {
    let output = compile_and_run(
        r#"
pick: () -> I64
pick = ->
    for (offset, ch) in "aé🦀z".char_indices!
        if offset == 0
            continue
        if offset == 3
            return ch as I64
    -1

main = !->
    total = 0
    for (offset, ch) in "aé🦀z".char_indices!
        if offset == 0
            continue
        total = total + offset
        if offset == 3
            break
    total.println!
    pick!.println!
    for ch in "é".chars!
        for n in 0..3
            if n == 1
                continue
            n.println!
        ch.println!
    for outer in "ab".chars!
        for inner in "xy".chars!
            inner.println!
            break
        outer.println!
"#,
    );
    assert_eq!(output, "4\n129408\n0\n2\né\nx\na\nx\nb\n");
}

#[test]
fn test_fold_loop_try_returns_from_enclosing_function() {
    let output = compile_and_run(
        r#"
check: I64 -> Result I64, I64
check = value -> if value == 1 then Result::Err 99 else Result::Ok value
run: () -> Result I64, I64
run = ->
    for (offset, ch) in "abc".char_indices!
        value = check offset?
        value.println!
    Result::Ok 42
main = !-> run!.println!
"#,
    );
    assert_eq!(output, "0\nErr(99)\n");
}

#[test]
fn test_fold_loop_user_collections_and_generic_source() {
    let output = compile_and_run(
        r#"
struct One T
    < value: T
impl Foldable for One
    try_fold = mut step, initial, source -> step (initial, source.value)

visit: C -> () where C: FoldableValue I64
visit = source !->
    for value in source
        value.println!

main = !->
    value = One
        value: 42
    visit value
    for value in (Option::Some 7)
        value.println!
    range = 2..4
    visit range
    mut values = Vec::new!
    values.push 8
    values.push 9
    for value in &values
        value.println!
    for value in values
        value.println!
"#,
    );
    assert_eq!(output, "42\n7\n2\n3\n8\n9\n8\n9\n");
}

#[test]
fn test_fold_loop_infers_source_and_element_types() {
    let output = compile_and_run(
        r#"
count_all = source ->
    count = 0
    for value in source
        count = count + 1
    count

main = !->
    count_all "é🦀".chars! .println!
    count_all (Option::Some 42) .println!
"#,
    );
    assert_eq!(output, "2\n1\n");
}

#[test]
fn test_fold_loop_uses_marked_protocol_without_std() {
    let status = compile_and_run_without_std(
        r#"
lang control_flow
< enum Signal Stop, State
    lang break
    Halt Stop
    lang continue
    More State

lang try
< trait Propagate
    lang output
    type Value
    lang residual
    type Error
    lang branch
    ~@split: Signal Self::Error, Self::Value

lang from_residual
< trait Lift E
    lang method
    lift: E -> Self

lang fn_once
< trait Call Args, Ret
    lang output
    type Output
    lang method
    ~@invoke: Args -> Ret

lang fn_mut
< trait CallMut Args, Ret where Self: Call Args, Ret
    lang output
    type Output
    lang method
    ^@invoke_mut: Args -> Ret

lang fold
< trait Each A
    lang method
    ~@scan: S -> B -> Signal R, B where S: CallMut (B, A), (Signal R, B)

struct Once
    < value: I64

impl Each I64 for Once
    ~@scan = mut step, initial -> step (initial, self.value)

main = ->
    source = Once
        value: 42
    for value in source
        return value
    0
"#,
    );
    assert_eq!(status, 42);
}

#[test]
fn test_fold_loop_custom_source_stops_requesting_elements() {
    let output = compile_and_run(
        r#"
struct Counter
    < end: I64

impl FoldableValue I64 for Counter
    ~@try_fold = mut step, initial ->
        state = initial
        i = 0
        while i < self.end
            (100 + i).println!
            match step (state, i)
                ControlFlow::Continue next => state = next
                ControlFlow::Break result =>
                    stopped: ControlFlow R, B = ControlFlow::Break result
                    return stopped
            i = i + 1
        ControlFlow::Continue state

make = ->
    999.println!
    Counter
        end: 5

main = !->
    for value in make!
        value.println!
        if value == 1
            break
"#,
    );
    assert_eq!(output, "999\n100\n0\n101\n1\n");
}

#[test]
fn test_fold_loop_owned_elements_drop_on_break_and_return() {
    let output = compile_and_run(
        r#"
struct Item
    < value: I64
impl Drop for Item
    ~@drop = !-> self.value.println!

first: Vec String -> String
first = values ->
    for value in values
        return value
    String::from "empty"

main = !->
    mut values = Vec::new!
    values.push (Item
        value: 1)
    values.push (Item
        value: 2)
    values.push (Item
        value: 3)
    for item in values
        break
    mut words = Vec::new!
    words.push (String::from "first")
    words.push (String::from "second")
    first words .println!
"#,
    );
    assert_eq!(output, "1\n2\n3\nfirst\n");
}

#[test]
fn test_fold_loop_utf8_cursor_resume_and_empty() {
    let output = compile_and_run(
        r#"
main = !->
    mut chars = "é🦀".chars!
    chars.next!.println!
    for ch in chars
        ch.println!
    mut indexed = "é🦀".char_indices!
    match indexed.next!
        Option::Some pair => pair.0.println!
        Option::None => -1.println!
    for (offset, ch) in indexed
        offset.println!
        ch.println!
    for ch in "".chars!
        999.println!
    mut empty = "".chars!
    empty.next!.println!
    empty.next!.println!
"#,
    );
    assert_eq!(output, "Some(é)\n🦀\n0\n2\n🦀\nNone\nNone\n");
}

#[test]
fn test_fold_loop_cursor_foldl_and_direct_try_fold() {
    let output = compile_and_run(
        r#"
sum: (I64, Char) -> I64
sum = pair -> pair.0 + (pair.1 as I64)
stop: (I64, (I64, Char)) -> ControlFlow I64, I64
stop = pair ->
    (total, entry) = pair
    if entry.0 == 3
        ControlFlow::Break total
    else
        ControlFlow::Continue (total + 1)

main = !->
    "aé🦀".chars!.foldl sum, 0 .println!
    match "aé🦀z".char_indices!.try_fold stop, 0
        ControlFlow::Break total => total.println!
        ControlFlow::Continue total => total.println!
"#,
    );
    assert_eq!(output, "129738\n2\n");
}

#[test]
fn test_fold_loop_native_loop_and_nested_lambda_control() {
    let output = compile_and_run(
        r#"
main = !->
    count = 0
    loop
        count = count + 1
        if count < 3
            continue
        break
    count.println!
    for ch in "ab".chars!
        local = ->
            return 7
        local!.println!
        ch.println!
        break
"#,
    );
    assert_eq!(output, "3\n7\na\n");
}

#[test]
fn test_fold_loop_utf8_cursor_cannot_outlive_string() {
    compile_should_fail(
        r#"
bad: () -> Chars
bad = ->
    text = String::from "é"
    text.chars!
main = !-> for_each bad!, (!.println!)
"#,
        "Cannot return reference",
    );
}

#[test]
fn test_fold_loop_rejects_refutable_pattern() {
    compile_should_fail(
        r#"
main = !->
    for 1 in (Option::Some 1)
        42.println!
"#,
        "for loop pattern must be irrefutable",
    );
}
