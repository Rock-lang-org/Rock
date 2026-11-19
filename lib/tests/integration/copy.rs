use std::fs;
use std::process::Command;
use std::sync::atomic::Ordering;

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

#[test]
fn copy_impl_rejects_non_nominal_receivers() {
    // Outside the crate that declares Copy the orphan rule already rejects
    // such impls, so declare the marker locally to reach the Copy check.
    compile_should_fail_with_exact_diagnostic(
        r#"
lang copy
< trait Copy

struct Point
    < x: I64

impl Copy for (Point, Point)

main = -> 0
"#,
        "Copy may only be implemented for a struct or enum, not `(Point, Point)`",
    );
}

#[test]
fn copy_impl_rejects_enum_payloads_that_are_not_copy() {
    compile_should_fail(
        r#"
> std::mem::Copy

enum Message
    Quit
    Text String

impl Copy for Message

main = !-> ()
"#,
        "Message cannot implement Copy because a field of type `String` is not Copy",
    );
}

#[test]
fn copy_impl_rejects_types_with_drop_on_one_instantiation() {
    compile_should_fail(
        r#"
> std::mem::Copy

struct Cell T
    < value: T

impl Drop for Cell I64
    ~@drop = -> return

impl Copy for Cell T where T: Copy

main = !-> ()
"#,
        "cannot implement Copy because it implements Drop",
    );
}

#[test]
fn copy_impls_from_a_dependency_crate_allow_reuse() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());

    let lib_entry = dir.join("lib.rk");
    fs::write(
        &lib_entry,
        r#"
> std::mem::Copy

< struct Point
    < x: I64

impl Copy for Point
"#,
    )
    .unwrap();
    let artifact = dir.join("geometry.rkca");
    let mut lib_config = test_config(lib_entry, dir.clone());
    lib_config.current_crate_name = Some("geometry".to_string());
    lib_config.crate_kind = rock_lib::CrateKind::Lib;
    lib_config.no_link = true;
    rock_lib::compile_with_products(&lib_config)
        .expect("library with a Copy impl must compile")
        .products
        .expect("library compilation must publish products")
        .write_artifact_to_path(&artifact)
        .unwrap();

    let entry = dir.join("main.rk");
    fs::write(
        &entry,
        r#"
> geometry::Point

main = !->
    point = Point
        x: 21
    copied = point
    (point.x + copied.x).println!
"#,
    )
    .unwrap();
    let mut config = test_config(entry, dir.clone());
    config
        .extern_artifacts
        .push(("geometry".to_string(), artifact));
    rock_lib::compile(&config).expect("Copy impl must survive the artifact boundary");

    let output = Command::new(dir.join("main")).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
}
