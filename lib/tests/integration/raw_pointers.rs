use crate::support::*;
use std::fs;
use std::process::Command;
use std::sync::atomic::Ordering;

#[test]
fn test_raw_wide_pointers_and_utf8_constructors() {
    let output = compile_and_run(
        r#"
main = !->
    mut values = [10, 20, 30]
    data = values.as_mut_ptr!
    raw: *[I64] = ~PtrFromParts data, 3
    view = unsafe &mut *raw
    view[1] = 42
    view[1].println!
    bytes = [195 as U8, 169 as U8]
    match str_from_utf8 (&bytes)
        Option::Some text => text.println!
        Option::None => "bad".println!
    invalid = [255 as U8]
    match str_from_utf8 (&invalid)
        Option::Some text => "bad".println!
        Option::None => "invalid".println!
    empty = unsafe slice_from_raw_parts data, 0
    empty.len!.println!
    mut number = 7
    thin = &mut number as *I64
    wide: *[I64] = ~PtrFromParts thin, 1
    borrowed = unsafe &mut *wide
    borrowed[0] = 8
    borrowed[0].println!
"#,
    );
    assert_eq!(output, "42\né\ninvalid\n0\n8\n");
}

#[test]
fn test_raw_wide_pointer_rejects_invalid_types_and_ambiguity() {
    for source in [
        "main = !->\n    raw: *[I64] = ~PtrFromParts (0 as *U8), 1\n",
        "main = !->\n    raw: *Str = ~PtrFromParts (0 as *I64), 1\n",
        "main = !->\n    raw: *I64 = ~PtrFromParts (0 as *I64), 1\n",
        "main = !->\n    raw = ~PtrFromParts (0 as *I64), 1\n",
        "main = !->\n    raw: *[I64] = ~PtrFromParts (0 as *I64)\n",
    ] {
        compile_should_fail(source, "PtrFromParts");
    }
}

#[test]
fn test_source_free_raw_slice_constructor_preserves_origins() {
    for opt_level in [0, 3] {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = test_temp_dir(id);
        fs::create_dir_all(&dir).unwrap();
        let dependency = dir.join("raw_slices.rk");
        fs::write(
            &dependency,
            r#"
unsafe borrow: *I64 -> I64 -> &[I64]
< unsafe borrow = data, len ->
    raw: *[I64] = ~PtrFromParts data, len
    unsafe &*raw
unsafe borrow_mut: *I64 -> I64 -> &mut [I64]
< unsafe borrow_mut = data, len ->
    raw: *[I64] = ~PtrFromParts data, len
    unsafe &mut *raw
"#,
        )
        .unwrap();
        let config = rock_lib::Config {
            entry_file: dependency.clone(),
            current_crate_name: Some("raw_slices".to_string()),
            output_dir: dir.clone(),
            opt_level,
            no_std: true,
            no_prelude: true,
            no_link: true,
            emit_object: Some(dir.join("raw_slices.o")),
            ..Default::default()
        };
        let artifact = dir.join("raw_slices.rkca");
        rock_lib::compile_with_products(&config)
            .unwrap()
            .products
            .unwrap()
            .write_artifact_to_path(&artifact)
            .unwrap();
        fs::remove_file(dependency).unwrap();
        let entry = dir.join("consumer.rk");
        let config = rock_lib::Config {
            entry_file: entry.clone(),
            current_crate_name: None,
            extern_artifacts: vec![("raw_slices".to_string(), artifact)],
            no_link: false,
            emit_object: None,
            ..config
        };
        fs::write(
            &entry,
            r#"
> raw_slices::borrow
main = ->
    values = [1, 2]
    view = unsafe borrow (~ArrPtr (&values)), 2
    if ~I64Eq (~ArrayLen *view), 2 then 0 else 1
"#,
        )
        .unwrap();
        rock_lib::compile(&config).unwrap();
        assert!(Command::new(dir.join("consumer"))
            .status()
            .unwrap()
            .success());
        for (source, expected) in [
            (
                r#"
> raw_slices::borrow
escape: () -> &[I64]
escape = ->
    values = [1, 2]
    unsafe borrow (~ArrPtr (&values)), 2
main = !->
    escape!
"#,
                "local",
            ),
            (
                r#"
> raw_slices::borrow_mut
main = ->
    mut values = [1, 2]
    data = ~ArrPtr (&mut values)
    first = unsafe borrow_mut data, 2
    second = unsafe borrow_mut data, 2
    ~I64Add (~ArrayLen *first), (~ArrayLen *second)
"#,
                "borrow",
            ),
        ] {
            fs::write(&entry, source).unwrap();
            let diagnostics = rock_lib::compile(&config).unwrap_err();
            assert!(
                diagnostics
                    .0
                    .iter()
                    .any(|error| error.message.contains(expected)),
                "{diagnostics:?}"
            );
        }
    }
}

#[test]
fn test_raw_wide_pointer_borrow_cannot_escape_local_storage() {
    compile_should_fail(
        r#"
escape: () -> &[I64]
escape = ->
    values = [1, 2]
    raw: *[I64] = ~PtrFromParts values.as_ptr!, 2
    unsafe &*raw

main = !->
    escaped = escape!
    escaped[0].println!
"#,
        "local",
    );
}

#[test]
fn test_raw_wide_pointer_borrows_preserve_exclusivity() {
    compile_should_fail(
        r#"
main = !->
    mut values = [1, 2]
    raw: *[I64] = ~PtrFromParts values.as_mut_ptr!, 2
    first = unsafe &mut *raw
    second = unsafe &mut *raw
    first[0].println!
    second[0].println!
"#,
        "borrow",
    );
    compile_should_fail(
        r#"
main = !->
    mut values = [1, 2]
    data = values.as_mut_ptr!
    first = unsafe slice_from_raw_parts_mut data, 2
    second = unsafe slice_from_raw_parts_mut data, 2
    first[0].println!
    second[0].println!
"#,
        "borrow",
    );
    compile_should_fail(
        r#"
main = !->
    mut values = [1, 2]
    raw: *[I64] = ~PtrFromParts values.as_ptr!, 2
    view = unsafe &*raw
    values[0] = 9
    view[0].println!
"#,
        "borrow",
    );
}

#[test]
fn test_raw_slice_constructors_validate_scalar_preconditions_and_allow_zsts() {
    for source in [
        "main = !->\n    view = unsafe slice_from_raw_parts (0 as *I64), 0\n    view.len!.println!\n",
        "main = !->\n    values = [1]\n    view = unsafe slice_from_raw_parts values.as_ptr!, -1\n    view.len!.println!\n",
        "main = !->\n    values = [1]\n    view = unsafe slice_from_raw_parts values.as_ptr!, 9223372036854775807\n    view.len!.println!\n",
    ] {
        let (_, success) = compile_and_run_with_status(source);
        assert!(!success, "raw slice construction must reject invalid scalar metadata");
    }
    let output = compile_and_run(
        r#"
struct Empty
main = !->
    empty = Empty
    data = &empty as *Empty
    view = unsafe slice_from_raw_parts data, 9223372036854775807
    view.len!.println!
"#,
    );
    assert_eq!(output, "9223372036854775807\n");
}

#[test]
fn test_raw_pointer_projected_borrows_retain_storage_loans() {
    for pointer in ["holder.ptr", "pair.0"] {
        let source = format!(
            r#"
struct Holder
    < ptr: *[I64]

main = ->
    mut values = [1, 2]
    raw: *[I64] = ~PtrFromParts (~ArrPtr (&mut values)), 2
    holder = Holder
        ptr: raw
    pair = (raw, 0)
    view = unsafe &*{pointer}
    values[0] = 9
    view[0]
"#
        );
        compile_should_fail(&source, "borrow conflict");
        assert_eq!(
            compile_and_run(
                &source
                    .replace("    values[0] = 9\n", "")
                    .replace("    view[0]\n", "    view[0].println!\n    0\n")
            ),
            "1\n"
        );
    }
}

fn compile_should_fail_with_source_diagnostic(source: &str, expected_error: &str) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("test.rk");
    fs::write(&source_path, source).unwrap();
    let config = test_config(source_path.clone(), dir.clone());

    let diagnostics = rock_lib::compile(&config).expect_err("expected compilation to fail");
    for diagnostic in &diagnostics.0 {
        for forbidden in [
            "struct#",
            "enum#",
            "generic#",
            "trait#",
            "?T",
            "DefId",
            "TypeVarId",
            "GenericParamId",
            "FieldId",
            "VariantId",
            "Local(",
        ] {
            assert!(
                !diagnostic.message.contains(forbidden),
                "source diagnostic exposed compiler identity {forbidden:?}: {}",
                diagnostic.message
            );
        }
    }
    let diagnostic = diagnostics
        .0
        .iter()
        .find(|diagnostic| diagnostic.message.contains(expected_error))
        .unwrap_or_else(|| panic!("expected diagnostic containing {expected_error:?}"));
    let rock_lib::diagnostic::DiagnosticLocation::Source(span) = &diagnostic.location else {
        panic!("source diagnostic should have a source location");
    };
    assert_eq!(span.file_path, source_path);
    assert!(span.start <= span.end);
    let attached = diagnostic
        .source
        .as_ref()
        .expect("source diagnostic should carry its loaded source");
    assert_eq!(attached.text, source);
    assert_eq!(
        attached.origin,
        rock_lib::diagnostic::DiagnosticSourceOrigin::FileSystem
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_ptr_deref_and_store() {
    let output = compile_and_run(
        r#"
main = ->
    x: I64 = 42
    ptr: *I64 = &x as *I64
    val = unsafe *ptr
    unsafe *ptr = 100
    val2 = unsafe *ptr
    val2.println!
    0
"#,
    );
    assert_eq!(output.trim(), "100");
}

#[test]
fn test_ptr_arithmetic_add() {
    // ~PtrOffset advances by n elements (element-stride, not byte-stride)
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    buf: *I64 = (malloc 24) as *I64
    unsafe *(~PtrOffset buf, 0) = 10
    unsafe *(~PtrOffset buf, 1) = 20
    unsafe *(~PtrOffset buf, 2) = 30
    a = unsafe *(~PtrOffset buf, 0)
    b = unsafe *(~PtrOffset buf, 1)
    c = unsafe *(~PtrOffset buf, 2)
    a.println!
    b.println!
    c.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "20");
    assert_eq!(lines[2], "30");
}

#[test]
fn test_ptr_arithmetic_sub() {
    // ~PtrOffset with a negative offset moves backwards by n elements
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    buf: *I64 = (malloc 24) as *I64
    unsafe
        *(~PtrOffset buf, 0) = 100
        *(~PtrOffset buf, 1) = 200
        end = ~PtrOffset buf, 1
        val = *(~PtrOffset end, -1)
        val.println!
    0
"#,
    );
    assert_eq!(output.trim(), "100");
}

#[test]
fn test_ptr_index_read() {
    // ptr[idx] reads the element at offset idx
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    buf: *I64 = (malloc 24) as *I64
    unsafe *(~PtrOffset buf, 0) = 7
    unsafe *(~PtrOffset buf, 1) = 8
    unsafe *(~PtrOffset buf, 2) = 9
    a = unsafe buf[0]
    b = unsafe buf[1]
    c = unsafe buf[2]
    a.println!
    b.println!
    c.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "8");
    assert_eq!(lines[2], "9");
}

#[test]
fn test_ptr_index_write() {
    // ptr[idx] = val writes the element at offset idx
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    mut buf: *I64 = (malloc 24) as *I64
    unsafe buf[0] = 11
    unsafe buf[1] = 22
    unsafe buf[2] = 33
    a = unsafe buf[0]
    b = unsafe buf[1]
    c = unsafe buf[2]
    a.println!
    b.println!
    c.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "11");
    assert_eq!(lines[1], "22");
    assert_eq!(lines[2], "33");
}

#[test]
fn test_ptr_index_roundtrip() {
    // Mix of ptr[idx] reads and writes, including overwrite
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    mut buf: *I64 = (malloc 16) as *I64
    unsafe buf[0] = 1
    unsafe buf[1] = 2
    unsafe buf[0] = unsafe buf[0] + unsafe buf[1]
    unsafe buf[0].println!
    0
"#,
    );
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_raw_slice_pointer_index_read() {
    let output = compile_and_run(
        r#"
read_second: &[I64] -> I64
read_second = s ->
    ptr: *[I64] = s as *[I64]
    unsafe ptr[1]

main = ->
    arr = [7, 8, 9]
    read_second &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_raw_slice_pointer_index_write() {
    let output = compile_and_run(
        r#"
write_second: &[I64] -> I64
write_second = s ->
    mut ptr: *[I64] = s as *[I64]
    unsafe ptr[1] = 42
    s[1]

main = ->
    arr = [7, 8, 9]
    write_second &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_raw_slice_pointer_index_write_out_of_bounds_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
write_oob: &[I64] -> I64
write_oob = s ->
    mut ptr: *[I64] = s as *[I64]
    unsafe ptr[999] = 42
    0

main = ->
    arr = [7, 8, 9]
    write_oob &arr .println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("index out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_raw_slice_pointer_index_read_requires_unsafe() {
    compile_should_fail(
        r#"
read_first: &[I64] -> I64
read_first = s ->
    ptr: *[I64] = s as *[I64]
    ptr[0]

main = ->
    arr = [7, 8, 9]
    read_first &arr .println!
    0
"#,
        "unsafe",
    );
}

#[test]
fn test_raw_slice_pointer_index_write_requires_unsafe() {
    compile_should_fail(
        r#"
write_first: &[I64] -> I64
write_first = s ->
    ptr: *[I64] = s as *[I64]
    ptr[0] = 42
    0

main = ->
    arr = [7, 8, 9]
    write_first &arr .println!
    0
"#,
        "unsafe",
    );
}

#[test]
fn safe_custom_pointer_index_impl_allows_read_and_write_outside_unsafe() {
    let exit_code = compile_and_run_without_std(
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue

impl ReadAt I64 for *I64
    type ReadValue = I64
    @read_at = i -> unsafe &*(~PtrOffset *self, i)

impl WriteAt I64 for *I64
    type WriteValue = I64
    ^@write_at = i -> unsafe &mut *(~PtrOffset *self, i)

main = ->
    mut value = 1
    mut ptr: *I64 = &value as *I64
    ptr[0] = 9
    ptr[0]
"#,
    );

    assert_eq!(exit_code, 9);
}

#[test]
fn test_raw_slice_pointer_cast_to_int_is_rejected() {
    compile_should_fail(
        r#"
read_addr: &[I64] -> I64
read_addr = s ->
    ptr: *[I64] = s as *[I64]
    ptr as I64

main = ->
    arr = [7, 8, 9]
    read_addr &arr .println!
    0
"#,
        "Cannot cast fat raw slice pointer to integer",
    );
}

#[test]
fn test_int_to_raw_slice_pointer_cast_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    addr: I64 = 0
    ptr = addr as *[I64]
    unsafe ptr[0]
"#,
        "Cannot cast integer to fat raw slice pointer",
    );
}

#[test]
fn test_raw_slice_pointer_arithmetic_is_rejected() {
    compile_should_fail(
        r#"
advance: &[I64] -> I64
advance = s ->
    ptr: *[I64] = s as *[I64]
    next = unsafe ptr + 1
    unsafe next[0] as I64

main = ->
    arr = [7, 8, 9]
    advance &arr .println!
    0
"#,
        "No implementation found for operator '+' on type *[I64]",
    );
}

#[test]
fn test_generic_impl_pointer_operator_rejects_unsized_instantiation() {
    compile_should_fail_with_source_diagnostic(
        r#"
struct PtrBox T
    < ptr: *T

impl PtrBox T
    unsafe @+ = offset -> @ptr + offset

main = ->
    arr = [7, 8, 9]
    ptr: *[I64] = (&arr) as *[I64]
    boxed = PtrBox
        ptr: ptr
    unsafe boxed + 1
    0
"#,
        "No implementation found for operator '+' on type PtrBox [I64]",
    );
}

#[test]
fn test_ptr_add_requires_unsafe() {
    compile_should_fail(
        r#"extern malloc: I64 -> *U8

main = ->
    buf: *I64 = (malloc 16) as *I64
    p = buf + 1
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_ptr_sub_requires_unsafe() {
    compile_should_fail(
        r#"extern malloc: I64 -> *U8

main = ->
    buf: *I64 = (malloc 16) as *I64
    p = unsafe ~PtrOffset buf, 1
    q = p - 1
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_ptr_add_sub_work_inside_unsafe_block() {
    let output = compile_and_run(
        r#"extern malloc: I64 -> *U8
extern free: *U8 -> ()

main = ->
    buf: *I64 = (malloc 24) as *I64
    unsafe
        *buf = 10
        *(buf + 1) = 20
        *(buf + 2) = 30
        *(buf + 1) .println!
        *(buf + 2 - 1) .println!
    free (buf as *U8)
    0
"#,
    );

    assert_eq!(output.trim(), "20\n20");
}

#[test]
fn test_local_sized_shadow_keeps_pointer_operator_bound_canonical() {
    let output = compile_and_run(
        r#"extern malloc: I64 -> *U8
extern free: *U8 -> ()

trait Sized

struct PtrBox T
    < ptr: *T

impl PtrBox T
    unsafe @offset = delta -> @ptr + delta

main = ->
    buf: *I64 = (malloc 16) as *I64
    unsafe
        *buf = 10
        *(buf + 1) = 42
        boxed = PtrBox
            ptr: buf
        ptr = boxed.offset 1
        *ptr .println!
    free (buf as *U8)
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_local_sized_shadow_where_clause_is_not_builtin_sized() {
    compile_should_fail(
        r#"trait Sized

struct Only

impl Sized for Only

requires_local_sized: T -> () where T: Sized
requires_local_sized = _ -> return

main = ->
    requires_local_sized 1
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn marked_renamed_sized_trait_drives_implicit_bounds_by_id() {
    let exit_code = compile_and_run_without_std(
        r#"lang sized
< trait StaticLayout

struct Boxed T
    < value: T

requires_layout: T -> I64 where T: StaticLayout
requires_layout = _ -> 42

main = ->
    boxed = Boxed
        value: 1
    requires_layout boxed
"#,
    );

    assert_eq!(exit_code, 42);
}

#[test]
fn test_raw_pointer_write_read_round_trip() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8
extern free: *U8 -> ()

main = ->
    ptr: *I64 = (malloc 8) as *I64
    unsafe *ptr = 42
    value = unsafe *ptr
    free (ptr as *U8)
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_std_raw_buffer_capacity_and_pointer_access() {
    let (output, success) = compile_and_run_with_status(
        r#"
> std::mem::raw_buffer::RawBuffer

main = ->
    empty: RawBuffer I64 = RawBuffer::new!
    empty.cap!.println!

    buf: RawBuffer I64 = unsafe RawBuffer::with_capacity (std::mem::sizeof &0), 2
    buf.cap!.println!
    ptr = buf.ptr!
    unsafe *ptr = 41
    unsafe *(~PtrOffset ptr, 1) = 42
    unsafe *ptr .println!
    unsafe *(~PtrOffset ptr, 1) .println!
    0
"#,
    );

    assert!(success, "program failed with output {output:?}");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["0", "2", "41", "42"]);
}

#[test]
fn test_std_raw_buffer_with_capacity_requires_unsafe() {
    compile_should_fail(
        r#"
> std::mem::raw_buffer::RawBuffer

main = ->
    buf = RawBuffer::with_capacity (std::mem::sizeof &0), 2
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_std_raw_buffer_as_slice_rejects_len_above_capacity() {
    let (_output, success) = compile_and_run_with_status(
        r#"
> std::mem::raw_buffer::RawBuffer

main = ->
    buf: RawBuffer I64 = RawBuffer::new!
    slice = unsafe buf.as_slice 1
    0
"#,
    );

    assert!(
        !success,
        "RawBuffer::as_slice should reject len above capacity"
    );
}

#[test]
fn test_std_raw_buffer_as_slice_requires_unsafe() {
    compile_should_fail(
        r#"
> std::mem::raw_buffer::RawBuffer

main = ->
    buf: RawBuffer I64 = RawBuffer::new!
    slice = buf.as_slice 0
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_std_raw_buffer_zero_capacity_as_slice_uses_non_null_buffer() {
    let output = compile_and_run(
        r#"
> std::mem::raw_buffer::RawBuffer

main = ->
    buf: RawBuffer I64 = unsafe RawBuffer::with_capacity (std::mem::sizeof &0), 0
    slice = unsafe buf.as_slice 0
    ((~ArrPtr (*slice)) as I64) != 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_global_alloc_requires_unsafe() {
    compile_should_fail(
        r#"
> std::mem::alloc::Global
> std::mem::alloc::Layout

main = ->
    layout = Layout
        size: 8
        align: 1
    ptr = Global::alloc layout
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_drop_in_place_wrapper_requires_unsafe() {
    compile_should_fail(
        r#"
extern malloc: I64 -> *U8

main = ->
    ptr: *I64 = (malloc 8) as *I64
    drop_in_place ptr
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_drop_in_place_rejects_non_pointer_argument() {
    compile_should_fail(
        r#"
main = ->
    unsafe ~DropInPlace 1
    0
"#,
        "DropInPlace expected raw pointer",
    );
}

#[test]
fn test_drop_in_place_requires_one_argument() {
    compile_should_fail(
        r#"
main = ->
    unsafe ~DropInPlace!
    0
"#,
        "DropInPlace requires exactly one argument",
    );
}

#[test]
fn test_drop_in_place_rejects_extra_arguments() {
    compile_should_fail(
        r#"
extern malloc: I64 -> *U8

main = ->
    ptr: *I64 = (malloc 8) as *I64
    unsafe ~DropInPlace ptr, ptr
    0
"#,
        "DropInPlace requires exactly one argument",
    );
}

#[test]
fn test_ptr_index_requires_unsafe() {
    // ptr[idx] outside unsafe block should be a compile error
    compile_should_fail(
        r#"
extern malloc: I64 -> *U8

main = ->
    buf: *I64 = (malloc 8) as *I64
    v = buf[0]
    0
"#,
        "unsafe",
    );
}
