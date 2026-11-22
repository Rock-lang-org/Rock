use crate::support::*;

#[test]
fn test_function_signature_projection_requires_unambiguous_bound() {
    compile_should_fail(
        r#"
trait Left
    type Value

trait Right
    type Value

read: T -> T::Value where T: Left, T: Right
read = value -> value

main = !-> ()
"#,
        "associated type 'Value' of 'T' is ambiguous",
    );
}

#[test]
fn test_associated_type_in_function_signature() {
    let output = compile_and_run(
        r#"
trait Measure
    type Value
    @measure: Self::Value

struct Distance

impl Measure for Distance
    type Value = I64
    @measure = -> 42

read: T -> T::Value where T: Measure
read = value -> value.measure!

main = !->
    read Distance .println!
"#,
    );
    assert_eq!(output, "42\n");
}

const CONVERT: &str = r#"
trait Convert T
    type Out
    convert: T -> Self::Out

trait Forward T
    type Out
    ~@forward: ((Option T), Self::Out)

struct Meters
    < value: I64

impl Convert I64 for Meters
    type Out = Bool
    convert = value -> value > 10
"#;

#[test]
fn test_impl_associated_type_projects_through_generic_bound() {
    let output = compile_and_run(&format!(
        "{CONVERT}
impl Forward U for T where U: Convert T
    type Out = U::Out
    ~@forward = -> (Option::None, U::convert self)

main = !->
    result: ((Option Meters), Bool) = 42.forward!
    result.1.println!
"
    ));
    assert_eq!(output, "true\n");
}

#[test]
fn test_generic_associated_type_without_declaring_bound_is_rejected() {
    compile_should_fail(
        r#"
trait Convert T
    type Out

trait Forward T
    type Out

impl Forward U for T
    type Out = U::Out

main = !-> ()
"#,
        "no bound on 'U' declares an associated type 'Out'",
    );
}

#[test]
fn test_generic_associated_type_declared_by_two_bounds_is_ambiguous() {
    compile_should_fail(
        r#"
trait Left
    type Out

trait Right
    type Out

trait Forward
    type Out

impl Forward for T where T: Left, T: Right
    type Out = T::Out

main = !-> ()
"#,
        "associated type 'Out' of 'T' is ambiguous",
    );
}

#[test]
fn test_generic_associated_type_book_example() {
    let chapter = include_str!("../../../docs/src/language/generics.md");
    let listing = chapter
        .split_once("### Associated Types of a Generic Parameter")
        .unwrap()
        .1
        .split_once("```rock\n")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0;
    assert_eq!(compile_and_run(listing), "1500\n");
}
