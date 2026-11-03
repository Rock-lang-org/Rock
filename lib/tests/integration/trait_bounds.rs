use crate::support::*;
use std::sync::atomic::Ordering;

#[test]
fn test_generic_function_alias_preserves_trait_bounds() {
    compile_should_fail(
        r#"trait Marker

requires_marker: T -> () where T: Marker
requires_marker = _ -> return

main = ->
    alias = requires_marker
    alias 1
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_generic_operator_function_preserves_trait_bounds() {
    compile_should_fail(
        r#"infix 9 %%

trait Marker

%%: T -> T -> T where T: Marker
%% = left, _ -> left

main = ->
    value = 1 %% 2
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_qualified_generic_function_value_preserves_trait_bounds() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("utils.rk"),
        r#"trait Marker

requires_marker: T -> () where T: Marker
requires_marker = _ -> return

< Marker
< requires_marker
"#,
    )
    .unwrap();

    std::fs::write(
        dir.join("test.rk"),
        r#"mod utils

main = ->
    alias = utils::requires_marker
    alias 1
    0
"#,
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    let result = rock_lib::compile(&config);
    let _ = std::fs::remove_dir_all(&dir);
    let diagnostics = result.expect_err("expected compilation to fail");
    let messages: Vec<String> = diagnostics.0.iter().map(|d| d.message.clone()).collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("does not implement trait")),
        "expected trait-bound failure, got: {messages:?}"
    );
}

#[test]
fn test_generic_method_call_preserves_method_bounds() {
    compile_should_fail(
        r#"trait Marker

struct Box

impl Box
    @requires_marker: T -> () where T: Marker
    @requires_marker = _ -> return

main = ->
    box = Box
    box.requires_marker 1
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_generic_method_value_preserves_method_bounds() {
    compile_should_fail(
        r#"trait Marker

struct Box

impl Box
    @requires_marker: T -> () where T: Marker
    @requires_marker = _ -> return

main = ->
    box = Box
    requires_marker = box.requires_marker
    requires_marker 1
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_trait_bound_method_signature_preserves_method_bounds() {
    compile_should_fail(
        r#"trait Marker

trait Provider
    @requires_marker: T -> () where T: Marker

struct Service

impl Provider for Service
    @requires_marker = _ -> return

call_requires: P -> () where P: Provider
call_requires = provider -> provider.requires_marker 1

main = ->
    service = Service
    call_requires service
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_generic_trait_bound_solver_checks_impl_bounds() {
    compile_should_fail(
        r#"trait Marker

trait Provider

struct Box T
    < value: T

impl Provider for Box T where T: Marker

requires_provider: T -> () where T: Provider
requires_provider = _ -> return

main = ->
    boxed = Box
        value: 7
    requires_provider boxed
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_generic_eq_bound_dispatches_binary_operator() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

same_boxed: Box T -> Box T -> Bool where T: Eq
same_boxed = left, right -> left.value == right.value

main = ->
    a = Box
        value: 7
    a2 = Box
        value: 7
    b = Box
        value: 7
    c = Box
        value: 9
    same_boxed a, b .println!
    same_boxed a2, c .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["true", "false"]);
}

#[test]
fn test_generic_eq_bound_borrows_rhs_without_move() {
    let output = compile_and_run(
        r#"
struct Tracked
    < id: I64

impl Eq for Tracked

impl PartialEq Tracked for Tracked
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

same_then_use_right: T -> T -> T where T: Eq
same_then_use_right = left, right ->
    if left == right
        1
    else
        0
    right

main = ->
    left = Tracked
        id: 7
    right = Tracked
        id: 7
    result = same_then_use_right left, right
    result.id.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["7"]);
}

#[test]
fn test_ord_operator_borrows_rhs_without_move() {
    let output = compile_and_run(
        r#"
struct Tracked
    < id: I64

impl Eq for Tracked

impl PartialEq Tracked for Tracked
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Ord for Tracked
    @cmp = other -> self.id.cmp &((*other).id)

impl PartialOrd for Tracked
    @partial_cmp = other -> Option::Some (self.cmp other)
    @< = other -> self.id < (*other).id
    @<= = other -> self.id <= (*other).id
    @> = other -> self.id > (*other).id
    @>= = other -> self.id >= (*other).id

less_then_use_right: T -> T -> T where T: Ord
less_then_use_right = left, right ->
    if left < right
        1
    else
        0
    right

main = ->
    left = Tracked
        id: 3
    right = Tracked
        id: 9
    result = less_then_use_right left, right
    result.id.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["9"]);
}

#[test]
fn test_generic_eq_operator_uses_explicit_symbol_bound() {
    compile_should_pass(
        r#"
trait SameNameEquals
    @==: Self -> Bool

struct Box T
    < value: T

bad_eq: Box T -> Box T -> Bool where T: SameNameEquals
bad_eq = left, right -> left.value == right.value

main = ->
    0
"#,
    );
}

#[test]
fn test_generic_eq_operator_rejects_unrelated_rhs_type() {
    compile_should_fail(
        r#"
bad_eq: T -> U -> Bool where T: Eq
bad_eq = left, right -> left == right

main = ->
    bad_eq 1, true .println!
    0
"#,
        "Type mismatch",
    );
}

#[test]
fn test_parameterized_equality_selects_rhs_and_inherits_not_equal() {
    let output = compile_and_run(r#"
struct Tag
    < id: I64

impl PartialEq Tag for Tag
    @== = other -> self.id == other.id

impl PartialEq I64 for Tag
    @== = other -> self.id == *other

impl PartialEq Bool for Tag
    @== = other -> (self.id != 0) == *other

compare_both: T -> U -> V -> Bool where T: PartialEq U, T: PartialEq V
compare_both = left, first, second -> left == first && left != second

main = !->
    tag = Tag
        id: 7
    same = Tag
        id: 7
    (tag == same).println!
    (tag != same).println!
    (tag == 7).println!
    (tag != 8).println!
    (tag == true).println!
    (tag != false).println!
    tag.id.println!
    same.id.println!
    compare_both tag, 7, false .println!
    compare_both same, true, 8 .println!
"#);
    assert_eq!(output, "true\nfalse\ntrue\ntrue\ntrue\ntrue\n7\n7\ntrue\ntrue\n");
}

#[test]
fn test_parameterized_equality_borrows_owned_strings_in_both_directions() {
    let output = compile_and_run(r#"
same_then_use_right: T -> U -> U where T: PartialEq U
same_then_use_right = left, right ->
    (left == right).println!
    (left != right).println!
    right

main = !->
    owned = String::from "Rock"
    same = String::from "Rock"
    (owned == same).println!
    (owned != same).println!
    (owned == "Rock").println!
    (owned != "Rock!").println!
    ("Rock" == owned).println!
    ("rock" != owned).println!
    (String::new! == "").println!
    (String::from "é" == "é").println!
    same_then_use_right "Rock", owned .println!
    same.println!
"#);
    assert_eq!(output, "true\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nRock\nRock\n");
}

#[test]
fn test_parameterized_equality_generic_impl_preserves_rhs_substitution() {
    let output = compile_and_run(r#"
struct Wrapper T
    < value: T

impl PartialEq (Wrapper U) for Wrapper T where T: PartialEq U
    @== = other -> self.value == other.value

main = !->
    left = Wrapper
        value: String::from "Rock"
    right = Wrapper
        value: "Rock"
    (left == right).println!
    (left != right).println!
    left.value.println!
    right.value.println!
"#);
    assert_eq!(output, "true\nfalse\nRock\nRock\n");
}

#[test]
fn test_parameterized_equality_infers_signatureless_operands() {
    let output = compile_and_run(r#"
same = left, right -> left == right
different = left, right -> left != right

main = !->
    owned = String::from "Rock"
    same owned, "Rock" .println!
    different "Rock", (String::from "rock") .println!
"#);
    assert_eq!(output, "true\ntrue\n");
}

#[test]
fn test_parameterized_equality_does_not_imply_reverse_impl() {
    compile_should_fail(r#"
struct Tag
    < id: I64

impl PartialEq I64 for Tag
    @== = other -> self.id == *other

main = !->
    tag = Tag
        id: 7
    (7 == tag).println!
"#, "No implementation found");
}

#[test]
fn test_parameterized_equality_cross_type_impl_does_not_satisfy_eq() {
    compile_should_fail(r#"
struct Tag
    < id: I64

impl PartialEq I64 for Tag
    @== = other -> self.id == *other

impl Eq for Tag

main = !-> return
"#, "supertrait obligation");
}

#[test]
fn test_std_eq_default_not_equal() {
    let src = r#"
struct Tag
    < value: I64

impl PartialEq Tag for Tag
    @== = other -> self.value == other.value

main = ->
    1 != 2 .println!
    1 != 1 .println!
    "abc" != "abd" .println!
    "abc" != "abc" .println!
    true != false .println!
    true != true .println!
    first = Tag
        value: 1
    same = Tag
        value: 1
    different = Tag
        value: 2
    first != different .println!
    first != same .println!
    0
"#;

    let output = compile_and_run(src);
    assert_eq!(output, "true\nfalse\ntrue\nfalse\ntrue\nfalse\ntrue\nfalse\n");
}
