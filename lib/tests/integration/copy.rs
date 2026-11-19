use crate::support::*;

#[test]
fn copy_impl_rejects_types_with_drop() {
    compile_should_fail(
        r#"
> std::mem::Copy

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = -> return

impl Copy for Tracked

main = !-> ()
"#,
        "cannot implement Copy because it implements Drop",
    );
}

#[test]
fn copy_impl_rejects_owned_string_fields() {
    compile_should_fail(
        r#"
> std::mem::Copy

struct Label
    text: String

impl Copy for Label

main = !-> ()
"#,
        "cannot implement Copy because a field of type `String` is not Copy",
    );
}

#[test]
fn generic_copy_impl_is_proved_by_its_bound() {
    let output = compile_and_run(
        r#"
> std::mem::Copy

struct Cell T
    < value: T

impl Copy for Cell T where T: Copy

main = !->
    cell = Cell
        value: 7
    copied = cell
    (cell.value + copied.value).println!
"#,
    );
    assert_eq!(output.trim(), "14");
}

#[test]
fn generic_copy_impl_without_bound_is_rejected() {
    compile_should_fail(
        r#"
> std::mem::Copy

struct Cell T
    < value: T

impl Copy for Cell T

main = !-> ()
"#,
        "cannot implement Copy because a field of type",
    );
}

#[test]
fn explicit_copy_impls_and_generic_bounds_allow_reuse() {
    let output = compile_and_run(
        r#"
> std::mem::Copy

struct Point
    < x: I64

impl Copy for Point

enum Marker
    At Point

impl Copy for Marker

duplicate: T -> (T, T) where T: Copy
duplicate = value -> (value, value)

from_ref: &Marker -> Marker
from_ref = value -> *value

read = marker -> match marker
    Marker::At point => point.x

main = !->
    point = Point
        x: 42
    original = Marker::At point
    (left, right) = duplicate original
    (read original).println!
    (read (from_ref &original)).println!
    (read left).println!
    (read right).println!
"#,
    );
    assert_eq!(output.trim(), "42\n42\n42\n42");
}
