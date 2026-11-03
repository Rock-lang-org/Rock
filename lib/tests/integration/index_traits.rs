use crate::support::*;
use std::fs;
use std::sync::atomic::Ordering;

#[test]
fn test_index_dispatches_through_trait_with_associated_output() {
    let output = compile_and_run(
        r#"
struct Boxed
    < value: I64

impl Index I64 for Boxed
    type Output = I64
    @index = i -> &@value

main = ->
    boxed = Boxed
        value: 7
    boxed[0].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn index_syntax_uses_marked_trait_member_and_output_ids() {
    let exit_code = compile_and_run_without_std(
        r#"lang index
< trait Lookup Key
    lang output
    type Value
    lang method
    @lookup: Key -> &Self::Value

struct Boxed
    < value: I64

impl Lookup I64 for Boxed
    type Value = I64
    @lookup = _ -> &@value

main = ->
    boxed = Boxed value: 42
    boxed[0]
"#,
    );

    assert_eq!(exit_code, 42);
}

#[test]
fn index_syntax_selects_separate_read_and_write_authority() {
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

struct Cell
    < read_value: I64
    < write_value: I64

impl ReadAt I64 for Cell
    type ReadValue = I64
    @read_at = _ -> &@read_value

impl WriteAt I64 for Cell
    type WriteValue = I64
    ^@write_at = _ -> &mut @write_value

probe = ->
    mut cell = Cell
        read_value: 2
        write_value: 4
    first = cell[0]
    cell[0] = 9
    match first
        2 => cell.write_value
        _ => 0

main = ->
    probe!
"#,
    );

    assert_eq!(exit_code, 9);
}

#[test]
fn cast_containing_index_assignment_is_rejected() {
    compile_should_fail_with_exact_diagnostic(
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

struct Cell
    < read_value: I64
    < write_value: I64

impl ReadAt I64 for Cell
    type ReadValue = I64
    @read_at = _ -> &@read_value

impl WriteAt I64 for Cell
    type WriteValue = I64
    ^@write_at = _ ->
        self.write_value = 9
        &mut @write_value

probe = ->
    mut cell = Cell
        read_value: 2
        write_value: 4
    (cell[0] as I64) = 9
    0

main = ->
    probe!
"#,
        "Cast expressions cannot be used as assignment places",
    );
}

#[test]
fn read_only_index_rejects_indexed_assignment() {
    compile_should_fail_with_exact_diagnostic(
        r#"lang index
< trait Index Key
    lang output
    type Output
    lang method
    @index: Key -> &Self::Output

struct ReadOnly
    < value: I64

impl Index I64 for ReadOnly
    type Output = I64
    @index = _ -> &@value

main = ->
    mut value = ReadOnly value: 1
    value[0] = 9
    0
"#,
        "Cannot use mutable indexing because the IndexMut language-item protocol is unavailable",
    );
}

#[test]
fn index_mut_impl_requires_matching_index_impl() {
    compile_should_fail_with_exact_diagnostic(
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

struct Cell
    < value: I64

impl WriteAt I64 for Cell
    type WriteValue = I64
    ^@write_at = _ -> &mut @value

main = ->
    0
"#,
        "IndexMut implementation for Cell with key I64 requires a matching Index implementation",
    );
}

#[test]
fn index_mut_impl_requires_matching_output() {
    compile_should_fail_with_exact_diagnostic(
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

struct Cell
    < read_value: I64
    < write_value: Bool

impl ReadAt I64 for Cell
    type ReadValue = I64
    @read_at = _ -> &@read_value

impl WriteAt I64 for Cell
    type WriteValue = Bool
    ^@write_at = _ -> &mut @write_value

main = ->
    0
"#,
        "IndexMut implementation output Bool does not match Index output I64",
    );
}

#[test]
fn index_mut_impl_pairs_reordered_bounds() {
    compile_should_pass_without_std(
        r#"trait First
trait Second

lang index
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

struct Cell T
    < read_value: T
    < write_value: T

impl ReadAt I64 for Cell T where T: First, T: Second
    type ReadValue = T
    @read_at = _ -> &@read_value

impl WriteAt I64 for Cell T where T: Second, T: First
    type WriteValue = T
    ^@write_at = _ -> &mut @write_value

main = ->
    0
"#,
    );
}

#[test]
fn index_mut_impl_accepts_shared_mutable_slice_pair() {
    compile_should_pass_without_std(
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

impl ReadAt I64 for &[I64]
    type ReadValue = I64
    @read_at = key ->
        slice = *self
        ptr = ~ArrPtr slice
        unsafe &*(~PtrOffset ptr, key)

impl WriteAt I64 for &mut [I64]
    type WriteValue = I64
    ^@write_at = key ->
        slice = &*self
        ptr = ~ArrPtr slice
        unsafe &mut *(~PtrOffset ptr, key)

main = ->
    0
"#,
    );
}

#[test]
fn index_mut_impl_rejects_reverse_reference_orientation() {
    compile_should_fail_with_exact_diagnostic(
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

struct Cell
    < read_value: I64
    < write_value: I64

impl ReadAt I64 for &mut Cell
    type ReadValue = I64
    @read_at = _ -> &@read_value

impl WriteAt I64 for &Cell
    type WriteValue = I64
    ^@write_at = _ -> &mut @write_value

main = ->
    0
"#,
        "IndexMut implementation for &Cell with key I64 requires a matching Index implementation",
    );
}

#[test]
fn index_mut_impl_rejects_same_shared_reference_orientation() {
    compile_should_fail_with_exact_diagnostic(
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

struct Cell
    < read_value: I64
    < write_value: I64

impl ReadAt I64 for &Cell
    type ReadValue = I64
    @read_at = _ -> &@read_value

impl WriteAt I64 for &Cell
    type WriteValue = I64
    ^@write_at = _ -> &mut @write_value

main = ->
    0
"#,
        "IndexMut implementation for &Cell with key I64 requires a matching Index implementation",
    );
}

#[test]
fn index_syntax_prefers_marked_array_impl_before_builtin_key_coercion() {
    let exit_code = compile_and_run_without_std(
        r#"lang index
< trait Lookup Key
    lang output
    type Value
    lang method
    @lookup: Key -> &Self::Value

impl Lookup U8 for [I64; 3]
    type Value = I64
    @lookup = _ ->
        ptr: *I64 = self as *I64
        unsafe &*ptr

main = ->
    values: [I64; 3] = [7, 8, 9]
    key: U8 = 1
    values[key]
"#,
    );

    assert_eq!(exit_code, 7);
}

#[test]
fn marked_slice_index_mut_updates_fixed_array() {
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

impl ReadAt U8 for &[I64]
    type ReadValue = I64
    @read_at = key ->
        slice = *(*self)
        ptr: *I64 = (~ArrPtr slice) as *I64
        unsafe &*(~PtrOffset ptr, (key as I64))

impl WriteAt U8 for &mut [I64]
    type WriteValue = I64
    ~@write_at = key ->
        slice = *self
        ptr: *I64 = (~ArrPtr slice) as *I64
        unsafe &mut *(~PtrOffset ptr, (key as I64))

main = ->
    mut values: [I64; 3] = [1, 2, 3]
    values[(1 as U8)] = 9
    values[(1 as U8)]
"#,
    );

    assert_eq!(exit_code, 9);
}

#[test]
fn index_syntax_rejects_unmarked_same_spelled_trait_without_provider() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let source_path = dir.join(format!("no_std_{id}.rk"));
    fs::write(
        &source_path,
        r#"trait Index Key
    type Output
    @index: Key -> &Self::Output

struct Boxed
    < value: I64

impl Index I64 for Boxed
    type Output = I64
    @index = _ -> &@value

main = ->
    boxed = Boxed value: 42
    boxed[0]
"#,
    )
    .unwrap();
    let mut config = test_config(source_path, dir);
    config.extern_artifacts.clear();
    config.no_std = true;
    config.no_prelude = true;

    let diagnostics =
        rock_lib::compile(&config).expect_err("unmarked Index must not receive [] dispatch");
    assert!(diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message
            == "Cannot use indexing because the Index language-item protocol is unavailable"
    }));
}

#[test]
fn test_std_index_trait_is_available_from_prelude() {
    let output = compile_and_run(
        r#"
struct Boxed
    < value: I64

impl Index I64 for Boxed
    type Output = I64
    @index = i -> &@value

main = ->
    boxed = Boxed
        value: 20
    boxed[0].println!
    0
"#,
    );

    assert_eq!(output.trim(), "20");
}

#[test]
fn test_std_deref_trait_is_available_from_prelude() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

impl Deref for Box T
    type Target = T
    @* = -> &@value

main = ->
    boxed = Box
        value: 12
    value: I64 = *boxed
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_index_rejects_wrong_concrete_argument_type_during_lowering() {
    compile_should_fail(
        r#"
trait Index Idx
    type Output
    @index: Idx -> &Self::Output

struct Boxed
    < value: I64

impl Index I64 for Boxed
    type Output = I64
    @index = i -> &@value

main = ->
    boxed = Boxed
        value: 7
    boxed[true]
    0
"#,
        "No implementation found for operator '[]'",
    );
}

#[test]
fn test_index_operator_rejects_local_shadow_index_trait() {
    compile_should_fail(
        r#"
trait Index Idx
    type Output
    @index: Idx -> &Self::Output

struct Boxed
    < value: I64

impl Index I64 for Boxed
    type Output = I64
    @index = i -> &@value

main = ->
    boxed = Boxed
        value: 7
    boxed[0]
"#,
        "No implementation found for operator '[]' on type Boxed",
    );
}

#[test]
fn test_local_index_trait_does_not_shadow_builtin_array_indexing() {
    let output = compile_and_run(
        r#"
trait Index Idx
    type Output
    @index: Idx -> &Self::Output

impl Index I64 for [I64; 3]
    type Output = Bool
    @index = i -> &(i == 0)

main = ->
    arr: [I64; 3] = [7, 8, 9]
    arr[0].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_index_dispatches_to_matching_impl_when_receiver_has_multiple_index_impls() {
    let output = compile_and_run(
        r#"
struct Boxed
    < number: I64
    < flag: Bool

impl Index I64 for Boxed
    type Output = I64
    @index = i -> &@number

impl Index Bool for Boxed
    type Output = Bool
    @index = b -> &@flag

main = ->
    boxed = Boxed
        number: 7
        flag: true
    boxed[0].println!
    boxed[true].println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "true");
}

#[test]
fn test_index_dispatches_to_matching_impl_when_receiver_instantiations_differ() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

impl Index I64 for Box I64
    type Output = I64
    @index = i -> &@value

impl Index I64 for Box Bool
    type Output = Bool
    @index = i -> &@value

main = ->
    i = Box
        value: 7
    b = Box
        value: true
    i[0].println!
    b[0].println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "true");
}

#[test]
fn test_index_operator_rejects_impl_with_unsatisfied_concrete_bound() {
    compile_should_fail(
        r#"
trait Marker

struct Box T
    < value: T

impl Index I64 for Box T where T: Marker
    type Output = T
    @index = i -> &@value

main = ->
    boxed = Box
        value: 7
    boxed[0]
    0
"#,
        "No implementation found for operator '[]' on type Box I64",
    );
}

#[test]
fn test_unsafe_index_operator_method_requires_unsafe() {
    compile_should_fail(
        r#"struct Bag
    < value: I64

impl Index I64 for Bag
    type Output = I64

    unsafe @index = _ -> &@value

main = ->
    bag = Bag
        value: 7
    value = bag[0]
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_deref_target_type_drives_unary_deref() {
    let output = compile_and_run(
        r#"
trait PointerLike
    type Target
    @*: &Self::Target

struct Box T
    < value: T

impl PointerLike for Box T
    type Target = T
    @* = -> &@value

main = ->
    boxed = Box
        value: 9
    value: I64 = *boxed
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}
