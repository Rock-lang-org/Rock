use crate::support::*;

#[test]
fn test_trait_basic() {
    let output = compile_and_run(
        r#"
struct Circle
    < radius: I64

struct Rectangle
    < width: I64
    < height: I64

trait Shape
    @area = -> 0

impl Shape for Circle
    @area = -> @radius * @radius * 3

impl Shape for Rectangle
    @area = -> @width * @height

main = ->
    c = Circle
        radius: 5
    r = Rectangle
        width: 4
        height: 6
    c.area!.println!
    r.area!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "75");
    assert_eq!(lines[1], "24");
}

#[test]
fn test_trait_default_methods() {
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
        name: 10
    c = Cat
        name: 20
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
fn test_trait_impl_requires_associated_type_definition() {
    compile_should_fail(
        r#"
trait Deref
    type Target
    @deref: () -> &Self::Target

struct Box T
    value: T

impl Deref for Box T
    @deref = -> @value

main = -> 0
"#,
        "required associated type 'Target'",
    );
}

#[test]
fn test_trait_impl_rejects_unknown_associated_type_definition() {
    compile_should_fail(
        r#"
trait Deref
    type Target
    @deref: () -> &Self::Target

struct Box T
    value: T

impl Deref for Box T
    type Output = T
    type Target = T
    @deref = -> @value

main = -> 0
"#,
        "unknown associated type 'Output'",
    );
}

#[test]
fn test_trait_impl_rejects_unknown_trait_name() {
    compile_should_fail(
        r#"
struct Box
    value: I64

impl MissingTrait for Box
    @value = -> @value

main = -> 0
"#,
        "unknown trait 'MissingTrait'",
    );
}

#[test]
fn test_binary_operator_resolves_by_symbol_not_trait_name() {
    let output = compile_and_run(
        r#"
trait FakePlus
    @+: Self -> Self

struct Wrapper
    < value: I64

impl FakePlus for Wrapper
    @+ = other -> Wrapper
        value: @value + other.value

main = ->
    left = Wrapper
        value: 1
    right = Wrapper
        value: 2
    sum = left + right
    sum.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_binary_operator_allows_local_trait_name_matching_prelude_trait() {
    let output = compile_and_run(
        r#"
trait Num
    @+: Self -> Self

struct Wrapper
    < value: I64

impl Num for Wrapper
    @+ = other -> Wrapper
        value: 99

main = ->
    left = Wrapper
        value: 1
    right = Wrapper
        value: 2
    sum = left + right
    sum.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "99");
}

#[test]
fn test_same_name_trait_methods_do_not_dispatch_by_method_name_only() {
    let output = compile_and_run(
        r#"
trait First
    @value: I64

trait Second
    @value: I64

struct Box
    < value: I64

impl Second for Box
    @value = -> 2

impl First for Box
    @value = -> 1

force_second: T -> I64 where T: Second
force_second = value -> value.value!

main = ->
    box = Box
        value: 0
    force_second box .println!
    0
"#,
    );

    assert_eq!(output.trim(), "2");
}

#[test]
fn test_generic_trait_bound_dispatch_uses_trait_arguments() {
    let output = compile_and_run(
        r#"
trait Pick T
    @pick: T

struct Both

impl Pick I64 for Both
    @pick = -> 1

impl Pick Bool for Both
    @pick = -> true

force_bool: T -> Bool where T: Pick Bool
force_bool = value -> value.pick!

main = ->
    both = Both
    force_bool both .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_concrete_trait_bound_solver_checks_trait_arguments() {
    compile_should_fail(
        r#"trait Pick T

impl Pick Bool for I64

requires_i64_pick: T -> () where T: Pick I64
requires_i64_pick = _ -> return

main = ->
    requires_i64_pick 1
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_concrete_trait_bound_solver_checks_receiver_and_trait_arg_substitution() {
    compile_should_fail(
        r#"trait Pick T

struct Box T
    < value: T

impl Pick T for Box T

requires_i64_pick: T -> () where T: Pick I64
requires_i64_pick = _ -> return

main = ->
    boxed = Box
        value: true
    requires_i64_pick boxed
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_generic_trait_bound_dispatch_matches_generic_impl_trait_arguments() {
    let output = compile_and_run(
        r#"
trait Pick T
    @pick: T

struct Box T
    < value: T

impl Pick T for Box T
    @pick = -> @value

force_i64: T -> I64 where T: Pick I64
force_i64 = value -> value.pick!

main = ->
    box = Box
        value: 9
    force_i64 box .println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_trait_default_method_substitutes_trait_generic_output() {
    let output = compile_and_run(
        r#"
trait Unwrap T
    @unwrap_or: T -> T
    @unwrap_or = fallback -> fallback

struct Empty

impl Unwrap I64 for Empty

main = ->
    empty = Empty
    empty.unwrap_or 9 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_trait_default_method_explicit_body_type_uses_trait_generic_argument() {
    let output = compile_and_run(
        r#"
trait Identity T
    @identity: T -> T
    @identity = value ->
        typed: T = value
        typed

struct Empty

impl Identity I64 for Empty

main = ->
    empty = Empty
    empty.identity 11 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "11");
}

#[test]
fn test_trait_default_method_substitutes_generic_trait_self_output_projection() {
    let output = compile_and_run(
        r#"
trait Project T
    type Output
    @project: T -> Self::Output
    @project = value -> value

struct Factory

impl Project I64 for Factory
    type Output = I64

main = ->
    factory = Factory
    factory.project 12 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_trait_default_method_body_must_match_concrete_signature_return() {
    compile_should_fail(
        r#"
trait Flag
    @flag: Bool
    @flag = -> 1

struct Marker

impl Flag for Marker

main = -> 0
"#,
        "return type mismatch",
    );
}

#[test]
fn test_trait_default_method_substitutes_trait_generic_in_nested_struct_return() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

trait MakeBox T
    @make: T -> Box T
    @make = value ->
        Box
            value: value

struct Factory

impl MakeBox I64 for Factory

main = ->
    factory = Factory
    boxed = factory.make 7
    boxed.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_trait_default_method_substitutes_trait_generic_in_nested_enum_return() {
    let output = compile_and_run(
        r#"
enum Maybe T
    Some T

trait MakeMaybe T
    @make: T -> Maybe T
    @make = value -> Maybe::Some value

struct Factory

impl MakeMaybe I64 for Factory

main = ->
    factory = Factory
    result = factory.make 7
    match result
        Maybe::Some value => value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_trait_default_method_lambda_captures_self_output_projection() {
    let output = compile_and_run(
        r#"
trait MakeClosure
    type Output
    @make: Self::Output -> (() -> Self::Output)
    @make = value -> -> value

struct Factory

impl MakeClosure for Factory
    type Output = I64

main = ->
    factory = Factory
    closure = factory.make 13
    closure!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "13");
}

#[test]
fn test_same_name_trait_default_methods_select_bound_trait_body() {
    let output = compile_and_run(
        r#"
trait First
    @value: I64
    @value = -> 1

trait Second
    @value: I64
    @value = -> 2

struct Box

impl First for Box

impl Second for Box

force_second: T -> I64 where T: Second
force_second = value -> value.value!

main = ->
    box = Box
    force_second box .println!
    0
"#,
    );

    assert_eq!(output.trim(), "2");
}

#[test]
fn test_same_name_generic_trait_default_methods_select_bound_trait_body() {
    let output = compile_and_run(
        r#"
trait First T
    @value: I64
    @value = -> 1

trait Second T
    @value: I64
    @value = -> 2

struct Box

impl First Bool for Box

impl Second I64 for Box

force_second: T -> I64 where T: Second I64
force_second = value -> value.value!

main = ->
    box = Box
    force_second box .println!
    0
"#,
    );

    assert_eq!(output.trim(), "2");
}

#[test]
fn test_same_name_trait_default_projection_is_ambiguous_without_bound_authority() {
    compile_should_fail(
        r#"
trait First
    type Output
    @value: Self::Output -> Self::Output
    @value = input -> input

trait Second
    type Output
    @value: Self::Output -> Self::Output
    @value = input -> input

struct Box

impl Second for Box
    type Output = I64

impl First for Box
    type Output = Bool

main = ->
    box = Box
    box.value 10 .println!
    0
"#,
        "Ambiguous selection for 'value'",
    );
}

#[test]
fn test_trait_default_method_substitutes_self_output_projection() {
    let output = compile_and_run(
        r#"
trait DefaultValue
    type Output
    @value: Self::Output -> Self::Output
    @value = fallback -> fallback

struct Box

impl DefaultValue for Box
    type Output = I64

main = ->
    box = Box
    box.value 12 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_trait_impl_body_must_match_associated_type_signature() {
    compile_should_fail(
        r#"
trait Deref
    type Target
    @deref: () -> &Self::Target

struct Box T
    < value: T

impl Deref for Box T
    type Target = T
    @deref = -> @value

main = -> 0
"#,
        "return type mismatch",
    );
}

#[test]
fn test_unary_neg_dispatches_through_std_trait_with_associated_output() {
    let output = compile_and_run(
        r#"
struct Wrapper
    < value: I64

impl Neg for Wrapper
    type Output = I64
    @- = -> @value

main = ->
    value = Wrapper
        value: 7
    -value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_std_numeric_operators_eq_for_all_integer_widths_and_char() {
    let mut source = String::from(
        r#"
equal: T -> T -> Bool where T: Eq
equal = a, b -> a == b

unequal: T -> T -> Bool where T: Eq
unequal = a, b -> a != b

main = !->
"#,
    );
    for ty in ["I8", "I16", "I32", "I64", "U8", "U16", "U32", "U64"] {
        for (method, rhs) in [("equal", 7), ("equal", 8), ("unequal", 7), ("unequal", 8)] {
            source.push_str(&format!(
                "    ({method} (7 as {ty}), ({rhs} as {ty})).println!\n"
            ));
        }
    }
    source.push_str(
        r#"
    (equal '🦀', '🦀').println!
    (equal 'é', '中').println!
    (unequal 'é', 'é').println!
    (unequal 'a', 'b').println!
    (equal (255 as U8), (127 as U8)).println!
    (equal (65535 as U16), (32767 as U16)).println!
    (equal (4294967295 as U32), (2147483647 as U32)).println!
    high = !(0 as U64)
    (equal high, (9223372036854775807 as U64)).println!
"#,
    );
    assert_eq!(
        compile_and_run(&source),
        "true\nfalse\nfalse\ntrue\n".repeat(9) + "false\nfalse\nfalse\nfalse\n"
    );
}

#[test]
fn test_std_numeric_operators_not_preserves_integer_width() {
    let mut source = String::from(
        r#"
main = !->
"#,
    );
    let mut expected = String::new();
    for (ty, ones) in [
        ("I8", -1i64),
        ("I16", -1),
        ("I32", -1),
        ("I64", -1),
        ("U8", 255),
        ("U16", 65535),
        ("U32", 4294967295),
        ("U64", -1),
    ] {
        source.push_str(&format!(
            "    zero_{ty} = 0 as {ty}
    ones_{ty}: {ty} = !zero_{ty}
    (ones_{ty} as I64).println!
    ((!ones_{ty}) == zero_{ty}).println!
    ((!(!(85 as {ty}))) == (85 as {ty})).println!
"
        ));
        expected.push_str(&format!("{ones}\ntrue\ntrue\n"));
    }
    source.push_str("    (!true).println!\n    (!false).println!\n");
    expected.push_str("false\ntrue\n");
    assert_eq!(compile_and_run(&source), expected);
}

#[test]
fn test_std_numeric_operators_integer_utilities() {
    let output = compile_and_run(
        r#"
main = !->
    (abs (-7)).println!
    (abs 0).println!
    (abs 7).println!
    (min (-7), 3).println!
    (min 3, (-7)).println!
    (max (-7), 3).println!
    (max 3, (-7)).println!
    (min 4, 4).println!
    (max 4, 4).println!
    smallest = (-9223372036854775807) - 1
    ((abs smallest) == smallest).println!
    ((min smallest, 0) == smallest).println!
    (max smallest, 0).println!
"#,
    );
    assert_eq!(output, "7\n0\n7\n-7\n-7\n3\n3\n4\n4\ntrue\ntrue\n0\n");
}

#[test]
fn test_std_numeric_operators_hash_map_explicit_grow_with_collisions() {
    let output = compile_and_run(
        r#"
show_entry: &HashMap I64, I64 -> I64 -> ()
show_entry = map, key !->
    match map.get (&key)
        Option::Some value => (*value).println!
        Option::None => "missing".println!

main = !->
    mut map = HashMap::new!
    map.insert 7, 70
    map.insert 15, 150
    map.insert 23, 230
    map.insert (-1), 10
    map.grow (sizeof &0), (sizeof &0)
    map.len!.println!
    show_entry &map, 7
    show_entry &map, 15
    show_entry &map, 23
    show_entry &map, (-1)
    map.insert 15, 151
    show_entry &map, 15
"#,
    );
    assert_eq!(output, "4\n70\n150\n230\n10\n151\n");
}

#[test]
fn test_unary_not_dispatches_through_std_trait_with_associated_output() {
    let output = compile_and_run(
        r#"
struct Wrapper
    < value: Bool

impl Not for Wrapper
    type Output = Bool
    @! = -> @value

main = ->
    value = Wrapper
        value: true
    !value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_unary_not_dispatches_through_operator_symbol_with_associated_output() {
    let output = compile_and_run(
        r#"
struct Wrapper
    < value: Bool

impl Not for Wrapper
    type Output = I64
    @! = -> 11

force_not: Wrapper -> I64
force_not = value -> !value

main = ->
    value = Wrapper
        value: true
    force_not value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "11");
}

#[test]
fn test_unary_neg_allows_local_trait_name_matching_prelude_trait() {
    let output = compile_and_run(
        r#"
trait Neg
    type Output
    @-: Self::Output

struct Wrapper
    < value: I64

impl Neg for Wrapper
    type Output = I64
    @- = -> @value

main = ->
    value = Wrapper
        value: 7
    -value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_unary_not_allows_local_trait_name_matching_prelude_trait() {
    let output = compile_and_run(
        r#"
trait Not
    type Output
    @!: Self::Output

struct Wrapper
    < value: Bool

impl Not for Wrapper
    type Output = Bool
    @! = -> @value

main = ->
    value = Wrapper
        value: true
    !value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_static_method_value_preserves_selected_authority() {
    let output = compile_and_run(
        r#"
struct Math

impl Math
    double = value -> value + value

main = ->
    operation = Math::double
    operation 6 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_generic_move_receiver_direct_call_path_uses_move_abi() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

impl Box T
    ~@take = -> @value

main = ->
    box = Box
        value: 9
    box.take!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_selfless_trait_default_stays_associated_function_when_injected() {
    let output = compile_and_run(
        r#"
trait Factory
    make = -> 5

struct Thing

impl Factory for Thing

main = ->
    Thing::make! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}
