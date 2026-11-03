use crate::support::*;
use std::fs;
use std::process::Command;
use std::sync::atomic::Ordering;

#[test]
fn test_return_loan_mixed_inputs_preserve_shared_access() {
    for (result, value, shared, exclusive) in [
        ("(&I64, &mut I64)", "(a, b)", "pair.0", "pair.1"),
        (
            "((&I64, I64), &mut I64)",
            "((a, 0), b)",
            "(pair.0).0",
            "pair.1",
        ),
        (
            "Views",
            "Views\n        shared: a\n        exclusive: b",
            "pair.shared",
            "pair.exclusive",
        ),
    ] {
        let source = format!(
            r#"
struct Views
    < shared: &I64
    < exclusive: &mut I64

split: &I64 -> &mut I64 -> {result}
split = a, b ->
    {value}

main = !->
    mut a = 1
    mut b = 2
    shared = &a
    pair = split &a, &mut b
    *{exclusive} = 7
    (*shared + *{shared}).println!
    (*{exclusive}).println!
"#
        );
        assert_eq!(compile_and_run(&source), "2\n7\n");
        for mutation in ["a = 9", "b = 9"] {
            compile_should_fail(
                &source.replace(
                    "    pair = split &a, &mut b\n",
                    &format!("    pair = split &a, &mut b\n    {mutation}\n"),
                ),
                "borrow conflict",
            );
        }
    }
}

#[test]
fn test_return_loan_raw_forwarding_preserves_selected_authority() {
    for selected in ["first", "second"] {
        let unselected = if selected == "first" { "b" } else { "a" };
        let source = format!(
            r#"
identity: *I64 -> *I64
identity = ptr -> ptr

choose: *I64 -> *I64 -> *I64
choose = first, second -> identity {selected}

main = !->
    mut a = 1
    mut b = 2
    first = &mut a as *I64
    second = &mut b as *I64
    forwarded = choose first, second
    {unselected} = 5
    view = unsafe &mut *forwarded
    *view = 7
    (*view).println!
    unsafe (*{selected}).println!
"#
        );
        assert_eq!(compile_and_run(&source), "7\n7\n");
        compile_should_fail(
            &source.replace(
                "    *view = 7",
                &format!("    {} = 9", if selected == "first" { "a" } else { "b" }),
            ),
            "borrow conflict",
        );
        compile_should_fail(&source.replace("    view = unsafe &mut *forwarded",
            &format!("    sibling = unsafe &*{selected}\n    view = unsafe &mut *forwarded\n    (*sibling).println!")), "borrow conflict");
    }
}

#[test]
fn test_return_loan_repairs_survive_deleted_source_artifacts() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let dependency = dir.join("loans.rk");
    fs::write(
        &dependency,
        r#"
split: &I64 -> &mut I64 -> (&I64, &mut I64)
< split = a, b -> (a, b)

identity: *I64 -> *I64
< identity = ptr -> ptr
"#,
    )
    .unwrap();
    let config = rock_lib::Config {
        entry_file: dependency.clone(),
        output_dir: dir.clone(),
        current_crate_name: Some("loans".to_string()),
        no_std: true,
        no_prelude: true,
        no_link: true,
        ..Default::default()
    };
    let artifact = dir.join("loans.rkca");
    rock_lib::compile_with_products(&config)
        .unwrap()
        .products
        .unwrap()
        .write_artifact_to_path(&artifact)
        .unwrap();
    fs::remove_file(dependency).unwrap();
    for (source, exit_code) in [
        (
            r#"
> loans::split
main = ->
    mut a = 1
    mut b = 2
    shared = &a
    pair = split &a, &mut b
    *pair.1 = 7
    result = ~I64Add *shared, *pair.0
    ~I64Add result, *pair.1
"#,
            9,
        ),
        (
            r#"
> loans::identity
main = ->
    mut value = 1
    ptr = &mut value as *I64
    forwarded = identity ptr
    view = unsafe &mut *forwarded
    *view = 7
    result = *view
    unsafe ~I64Add result, *ptr
"#,
            14,
        ),
    ] {
        for opt_level in [0, 3] {
            for invalid in [false, true] {
                let entry = dir.join("consumer.rk");
                let source = if invalid {
                    source.replace("    *pair.1 = 7", "    a = 9\n    *pair.1 = 7")
                        .replace("    view = unsafe &mut *forwarded",
                            "    sibling = unsafe &*ptr\n    view = unsafe &mut *forwarded\n    *sibling")
                } else {
                    source.to_string()
                };
                fs::write(&entry, source).unwrap();
                let result = rock_lib::compile(&rock_lib::Config {
                    entry_file: entry,
                    current_crate_name: None,
                    opt_level,
                    extern_artifacts: vec![("loans".to_string(), artifact.clone())],
                    no_link: invalid,
                    ..config.clone()
                });
                if invalid {
                    let errors = result.unwrap_err();
                    assert!(
                        errors
                            .0
                            .iter()
                            .any(|error| error.message.contains("borrow conflict")),
                        "{errors:?}"
                    );
                } else {
                    result.unwrap();
                    let output = run_test_command(&mut Command::new(dir.join("consumer")));
                    assert_eq!(output.status.code(), Some(exit_code));
                }
            }
        }
    }
}

#[test]
fn test_raw_pointer_mixed_return_arguments_retain_all_loans() {
    compile_should_fail(
        r#"
unsafe choose: &I64 -> *I64 -> Bool -> &I64
unsafe choose = first, second, flag ->
    if flag then first else unsafe &*second

main = ->
    mut a = 1
    mut b = 2
    data = &mut b as *I64
    view = unsafe choose &a, data, false
    b = 9
    *view
"#,
        "borrow conflict",
    );
}

#[test]
fn test_raw_pointer_aggregate_arguments_retain_return_loans() {
    for reference in ["&I64", "&mut I64"] {
        for (argument, projection, value) in [
            (
                "Holder",
                "holder.ptr",
                "Holder\n        ptr: &mut value as *I64",
            ),
            ("(*I64, I64)", "holder.0", "(&mut value as *I64, 0)"),
            (
                "((*I64, I64), Bool)",
                "(holder.0).0",
                "((&mut value as *I64, 0), true)",
            ),
        ]
        .into_iter()
        .chain((reference == "&I64").then_some((
            "[*I64; 1]",
            "holder[0]",
            "[&mut value as *I64]",
        ))) {
            let source = format!(
                r#"
struct Holder
    < ptr: *I64

unsafe borrow: {argument} -> {reference}
unsafe borrow = holder -> unsafe {reference_expr}*{projection}

main = !->
    mut value = 1
    holder = {value}
    view = unsafe borrow holder
    value = 9
    (*view).println!
"#,
                reference_expr = if reference == "&I64" { "&" } else { "&mut " }
            );
            compile_should_fail(&source, "borrow conflict");
            let valid = source.replace("    value = 9\n", "");
            assert_eq!(compile_and_run(&valid), "1\n");
            if reference == "&mut I64" {
                assert_eq!(
                    compile_and_run(&valid.replace(
                        "    (*view).println!",
                        "    *view = 7\n    (*view).println!"
                    )),
                    "7\n"
                );
            }
        }
    }
}

#[test]
fn test_raw_pointer_aggregate_results_retain_return_loans() {
    for reference in ["&I64", "&mut I64"] {
        for (result, value, projection) in [
            (format!("({reference}, I64)"), "(view, 0)", "pair.0"),
            (
                format!("(({reference}, I64), Bool)"),
                "((view, 0), true)",
                "(pair.0).0",
            ),
            (
                "View".to_string(),
                "View\n        value: view",
                "pair.value",
            ),
        ]
        .into_iter()
        .chain((reference == "&I64").then_some((
            format!("[{reference}; 1]"),
            "[view]",
            "pair[0]",
        ))) {
            let source = format!(
                r#"
struct View
    < value: {reference}

unsafe borrow: *I64 -> {result}
unsafe borrow = ptr ->
    view = unsafe {reference_expr}*ptr
    {value}

main = !->
    mut value = 1
    ptr = &mut value as *I64
    pair = unsafe borrow ptr
    value = 9
    (*{projection}).println!
"#,
                reference_expr = if reference == "&I64" { "&" } else { "&mut " }
            );
            compile_should_fail(&source, "borrow conflict");
            let valid = source.replace("    value = 9\n", "");
            assert_eq!(compile_and_run(&valid), "1\n");
            if reference == "&mut I64" {
                let write = valid.replace(
                    &format!("    (*{projection}).println!"),
                    &format!("    *{projection} = 7\n    (*{projection}).println!"),
                );
                assert_eq!(compile_and_run(&write), "7\n");
                compile_should_fail(&valid.replace("    pair = unsafe borrow ptr",
                    "    sibling = unsafe &*ptr\n    pair = unsafe borrow ptr\n    (*sibling).println!"), "borrow conflict");
            }
        }
    }
}

#[test]
fn test_raw_pointer_enum_arguments_and_results_retain_return_loans() {
    for reference in ["&I64", "&mut I64"] {
        let source = format!(
            r#"
enum Input
    Value *I64

enum Output
    Value {reference}

unsafe borrow: Input -> Output
unsafe borrow = input -> match input
    Input::Value ptr => Output::Value (unsafe {reference_expr}*ptr)

main = !->
    mut value = 1
    input = Input::Value (&mut value as *I64)
    output = unsafe borrow input
    view = match output
        Output::Value view => view
    value = 9
    (*view).println!
"#,
            reference_expr = if reference == "&I64" { "&" } else { "&mut " }
        );
        compile_should_fail(&source, "borrow conflict");
        assert_eq!(
            compile_and_run(&source.replace("    value = 9\n", "")),
            "1\n"
        );
    }
}

#[test]
fn test_raw_pointer_aggregate_reborrow_uses_selected_input_authority() {
    let source = r#"
struct Holder
    < ptr: *I64

unsafe borrow: *I64 -> Holder -> &mut I64
unsafe borrow = unused, holder -> unsafe &mut *holder.ptr

main = !->
    mut a = 1
    mut b = 2
    unused = &mut a as *I64
    ptr = &mut b as *I64
    holder = Holder
        ptr: ptr
    view = unsafe borrow unused, holder
    a = 7
    *view = 9
    (*view).println!
"#;
    assert_eq!(compile_and_run(source), "9\n");
    compile_should_fail(&source.replace("    a = 7", "    b = 7"), "borrow conflict");
    compile_should_fail(&source.replace("    view = unsafe borrow unused, holder",
        "    sibling = unsafe &*ptr\n    view = unsafe borrow unused, holder\n    (*sibling).println!"), "borrow conflict");
}

#[test]
fn test_raw_pointer_aggregate_return_loans_survive_deleted_source_artifacts() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let dependency = dir.join("loans.rk");
    fs::write(
        &dependency,
        r#"
< struct Holder
    < ptr: *I64

unsafe borrow: Holder -> (&I64, I64)
< unsafe borrow = holder -> (unsafe &*holder.ptr, 0)

unsafe borrow_mut: Holder -> (&mut I64, I64)
< unsafe borrow_mut = holder -> (unsafe &mut *holder.ptr, 0)
"#,
    )
    .unwrap();
    let config = rock_lib::Config {
        entry_file: dependency.clone(),
        output_dir: dir.clone(),
        current_crate_name: Some("loans".to_string()),
        no_std: true,
        no_prelude: true,
        no_link: true,
        ..Default::default()
    };
    let artifact = dir.join("loans.rkca");
    rock_lib::compile_with_products(&config)
        .unwrap()
        .products
        .unwrap()
        .write_artifact_to_path(&artifact)
        .unwrap();
    fs::remove_file(dependency).unwrap();
    for method in ["borrow", "borrow_mut"] {
        for invalid in [false, true] {
            let entry = dir.join("consumer.rk");
            let mutation = if invalid {
                "value = 9"
            } else if method == "borrow_mut" {
                "*pair.0 = 7"
            } else {
                "*pair.0"
            };
            fs::write(
                &entry,
                format!(
                    r#"
> loans::Holder
> loans::{method}

main = ->
    mut value = 1
    holder = Holder
        ptr: &mut value as *I64
    pair = unsafe {method} holder
    {mutation}
    *pair.0
"#
                ),
            )
            .unwrap();
            let result = rock_lib::compile(&rock_lib::Config {
                entry_file: entry,
                current_crate_name: None,
                extern_artifacts: vec![("loans".to_string(), artifact.clone())],
                ..config.clone()
            });
            assert_eq!(result.is_err(), invalid, "{method}, {mutation}: {result:?}");
            if let Err(errors) = result {
                assert!(
                    errors
                        .0
                        .iter()
                        .any(|error| error.message.contains("borrow conflict")),
                    "{errors:?}"
                );
            }
        }
    }
}

#[test]
fn test_raw_pointer_return_reborrow_uses_selected_argument() {
    for selected in ["first", "second"] {
        let source = format!(
            r#"
unsafe choose: *I64 -> *I64 -> &mut I64
unsafe choose = first, second -> unsafe &mut *{selected}

main = !->
    mut a = 1
    mut b = 2
    first_ptr = &mut a as *I64
    second_ptr = &mut b as *I64
    view = unsafe choose first_ptr, second_ptr
    {unselected} = 7
    *view = 9
    (*view).println!
    unsafe (*{selected}_ptr).println!
"#,
            unselected = if selected == "first" { "b" } else { "a" }
        );
        assert_eq!(compile_and_run(&source), "9\n9\n");
        let conflicting = source.replace(
            "    *view = 9",
            &format!("    {} = 9", if selected == "first" { "a" } else { "b" }),
        );
        compile_should_fail(&conflicting, "borrow conflict");
    }
}

#[test]
fn test_raw_pointer_return_loans_survive_deleted_source_artifacts() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let dependency = dir.join("loans.rk");
    fs::write(
        &dependency,
        r#"
unsafe choose: &I64 -> *I64 -> Bool -> &I64
< unsafe choose = first, second, flag ->
    if flag then first else unsafe &*second

unsafe second: *I64 -> *I64 -> &mut I64
< unsafe second = first, second -> unsafe &mut *second
"#,
    )
    .unwrap();
    let config = rock_lib::Config {
        entry_file: dependency.clone(),
        output_dir: dir.clone(),
        current_crate_name: Some("loans".to_string()),
        no_std: true,
        no_prelude: true,
        no_link: true,
        ..Default::default()
    };
    let artifact = dir.join("loans.rkca");
    rock_lib::compile_with_products(&config)
        .unwrap()
        .products
        .unwrap()
        .write_artifact_to_path(&artifact)
        .unwrap();
    fs::remove_file(dependency).unwrap();
    for (call, mutation, invalid) in [
        ("choose &a, holder.ptr, false", "b = 9", true),
        ("choose &a, holder.ptr, false", "*view", false),
        ("second (&mut a as *I64), (holder.ptr)", "*view = 9", false),
        ("second (&mut a as *I64), (holder.ptr)", "b = 9", true),
    ] {
        let entry = dir.join("consumer.rk");
        fs::write(
            &entry,
            format!(
                r#"
> loans::choose
> loans::second
struct Holder
    < ptr: *I64
main = ->
    mut a = 1
    mut b = 2
    holder = Holder
        ptr: &mut b as *I64
    view = unsafe {call}
    {mutation}
    *view
"#
            ),
        )
        .unwrap();
        let result = rock_lib::compile(&rock_lib::Config {
            entry_file: entry,
            current_crate_name: None,
            extern_artifacts: vec![("loans".to_string(), artifact.clone())],
            ..config.clone()
        });
        assert_eq!(result.is_err(), invalid, "{call}, {mutation}: {result:?}");
        if let Err(errors) = result {
            assert!(
                errors
                    .0
                    .iter()
                    .any(|error| error.message.contains("borrow conflict")),
                "{errors:?}"
            );
        }
    }
}

#[test]
fn returning_reference_to_call_temporary_is_rejected() {
    compile_should_fail(
        r#"
make_value = -> 1

bad: () -> &I64
bad = -> &make_value!

main = ->
    bad!
    0
"#,
        "temporary",
    );
}

#[test]
fn binding_reference_to_call_temporary_is_rejected() {
    compile_should_fail(
        r#"
make_value = -> 1

main = ->
    r = &make_value!
    *r .println!
    0
"#,
        "temporary",
    );
}

#[test]
fn struct_field_reference_to_call_temporary_is_rejected() {
    compile_should_fail(
        r#"
struct Holder
    < ref: &I64

make_value = -> 1

main = ->
    holder = Holder
        ref: &make_value!
    *holder.ref .println!
    0
"#,
        "temporary",
    );
}

#[test]
fn tuple_field_reference_to_call_temporary_is_rejected() {
    compile_should_fail(
        r#"
make_value = -> 1

main = ->
    pair = (&make_value!, 0)
    *pair.0 .println!
    0
"#,
        "temporary",
    );
}

#[test]
fn returning_reference_to_local_is_rejected() {
    compile_should_fail(
        r#"
bad: () -> &I64
bad = ->
    value = 1
    &value

main = ->
    bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_reference_to_by_value_param_is_rejected() {
    compile_should_fail(
        r#"
bad: I64 -> &I64
bad = value -> &value

main = ->
    x = *(bad 1)
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_reference_via_known_call_to_local_is_rejected() {
    compile_should_fail(
        r#"
id_ref: &I64 -> &I64
id_ref = value -> value

bad: () -> &I64
bad = ->
    local = 1
    id_ref &local

main = ->
    x = *(bad!)
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_reference_from_aggregate_field_is_rejected() {
    compile_should_fail(
        r#"
struct Holder
    < ref: &I64

bad: () -> &I64
bad = ->
    local = 1
    holder = Holder
        ref: &local
    holder.ref

main = ->
    x = *(bad!)
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_option_some_reference_to_local_is_rejected() {
    compile_should_fail(
        r#"
bad: () -> Option &I64
bad = ->
    local = 1
    Option::Some (&local)

main = ->
    value = bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_struct_containing_reference_to_local_is_rejected() {
    compile_should_fail(
        r#"
struct Holder
    < ref: &I64

bad: () -> Holder
bad = ->
    local = 1
    Holder
        ref: &local

main = ->
    holder = bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_vec_get_from_local_vec_is_rejected() {
    compile_should_fail(
        r#"
> std::collections::Vec

bad: () -> Option &I64
bad = ->
    mut v = Vec::new!
    v.push 1
    v.get 0

main = ->
    value = bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_reference_to_local_vec_index_is_rejected() {
    compile_should_fail(
        r#"
> std::collections::Vec

bad: () -> &I64
bad = ->
    mut v = Vec::new!
    v.push 1
    &v[0]

main = ->
    value = bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_box_as_ref_from_local_box_is_rejected() {
    compile_should_fail(
        r#"
> std::mem::Box

bad: () -> &I64
bad = ->
    b = Box::new 1
    b.as_ref!

main = ->
    value = bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_raw_deref_from_local_box_pointer_is_rejected() {
    compile_should_fail(
        r#"
> std::mem::Box

bad: () -> &I64
bad = ->
    b = Box::new 1
    ptr = b.as_ptr!
    unsafe &*ptr

main = ->
    value = bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_vec_get_from_input_vec_reference_is_allowed() {
    let output = compile_and_run(
        r#"
> std::collections::Vec

get_first: &Vec I64 -> Option &I64
get_first = v -> (*v).get 0

main = ->
    mut v = Vec::new!
    v.push 7
    match (get_first &v)
        Option::Some value => *value .println!
        Option::None => 0.println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn returning_vec_index_from_input_vec_reference_is_allowed() {
    let output = compile_and_run(
        r#"
> std::collections::Vec

first: &Vec I64 -> &I64
first = v -> &((*v)[0])

main = ->
    mut v = Vec::new!
    v.push 7
    *(first &v) .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn index_reference_returning_from_reference_key_blocks_key_mutation() {
    compile_should_fail(
        r#"
struct Key
    < value: I64

struct Container
    < value: I64

impl Index &Key for Container
    type Output = I64
    @index = key -> &key.value

main = ->
    mut key = Key
        value: 1
    container = Container
        value: 7
    saved = &container[&key]
    key.value = 2
    *saved .println!
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn index_reference_returning_from_receiver_does_not_borrow_key() {
    let output = compile_and_run(
        r#"
struct Key
    < value: I64

struct Container
    < value: I64

impl Index &Key for Container
    type Output = I64
    @index = _ -> &@value

main = ->
    mut key = Key
        value: 1
    container = Container
        value: 7
    saved = &container[&key]
    key.value = 2
    *saved .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn returning_reference_from_branch_known_call_to_local_is_rejected() {
    compile_should_fail(
        r#"
id_ref: &I64 -> &I64
id_ref = value -> value

choose: Bool -> &I64 -> &I64
choose = flag, input ->
    if flag
        local = 1
        id_ref &local
    else
        input

main = ->
    value = 2
    x = *(choose true, &value)
    0
"#,
        "does not live long enough",
    );
}

#[test]
fn returning_reference_from_multi_param_summary_is_rejected() {
    compile_should_fail(
        r#"
choose_ref: Bool -> &I64 -> &I64 -> &I64
choose_ref = flag, left, right ->
    if flag
        left
    else
        right

bad: Bool -> &I64 -> &I64
bad = flag, input ->
    local = 1
    choose_ref flag, &local, input

main = ->
    value = 2
    x = *(bad false, &value)
    0
"#,
        "does not live long enough",
    );
}

#[test]
fn returning_reference_from_extern_call_to_local_is_rejected() {
    compile_should_fail(
        r#"
extern id_ref: &I64 -> &I64

bad: () -> &I64
bad = ->
    local = 1
    id_ref &local

main = ->
    x = *(bad!)
    0
"#,
        "does not live long enough",
    );
}

#[test]
fn returning_input_reference_is_allowed() {
    let output = compile_and_run(
        r#"
id_ref: &I64 -> &I64
id_ref = value -> value

main = ->
    value = 7
    *(id_ref &value) .println!
    0
"#,
    );
    assert_eq!(output.trim(), "7");
}
