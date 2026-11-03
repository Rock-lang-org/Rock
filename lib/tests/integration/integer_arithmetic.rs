use crate::support::*;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::Ordering;

#[test]
fn test_ordinary_constant_integer_arithmetic_is_diagnosed() {
    for expression in [
        "(127 as I8) + (1 as I8)",
        "(255 as U8) + (1 as U8)",
        "(32767 as I16) + (1 as I16)",
        "(65535 as U16) + (1 as U16)",
        "(2147483647 as I32) + (1 as I32)",
        "(4294967295 as U32) + (1 as U32)",
        "9223372036854775807 + 1",
        "(-1 as U64) + (1 as U64)",
        "(-128 as I8) - (1 as I8)",
        "(64 as I8) * (2 as I8)",
        "-(-128 as I8)",
        "(1 as I8) / (0 as I8)",
        "(-128 as I8) / (-1 as I8)",
        "(-128 as I8) % (-1 as I8)",
        "(1 as I8) << (8 as I8)",
        "(1 as I8) >> (-1 as I8)",
        "((120 as I8) + (7 as I8)) + (1 as I8)",
        "~IntCheckedAdd ((120 as I8) + (7 as I8)), (1 as I8)",
        "(~I8Not (127 as I8)) / (-1 as I8)",
        "((126 as I8) | (1 as I8)) + (1 as I8)",
        "(((127 as I8) + (1 as I8)) == (0 as I8)).println!",
    ] {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = test_temp_dir(id);
        fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("constant.rk");
        fs::write(&entry, format!("main = !->\n    {expression}\n")).unwrap();
        let errors = rock_lib::compile(&rock_lib::Config {
            no_link: true,
            ..test_config(entry.clone(), dir)
        })
        .unwrap_err();
        assert!(errors.0.iter().any(|error| {
            error.message.contains("invalid constant integer arithmetic")
                && matches!(&error.location, rock_lib::diagnostic::DiagnosticLocation::Source(span) if span.file_path == entry)
        }), "{expression}: {errors:?}");
        if expression.contains("println") {
            let source = fs::read_to_string(&entry).unwrap();
            let diagnostic = errors
                .0
                .iter()
                .find(|error| {
                    error
                        .message
                        .contains("invalid constant integer arithmetic")
                })
                .unwrap();
            let rock_lib::diagnostic::DiagnosticLocation::Source(span) = &diagnostic.location
            else {
                panic!("missing source span");
            };
            let highlighted = &source[span.start..span.end];
            assert!(
                highlighted.contains('+') && !highlighted.contains("println"),
                "wrong operation highlighted: {highlighted}"
            );
        }
    }
}

#[test]
fn test_constant_operator_analysis_uses_selected_body_and_producer_policy() {
    for checked in [false, true] {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = test_temp_dir(id);
        fs::create_dir_all(&dir).unwrap();
        let dependency = dir.join("operators.rk");
        fs::write(&dependency, r#"
infix 9 +
infix 9 -
< trait Add
    @+: Self -> Self
< trait Sub
    @-: Self -> Self
impl Add for I8
    @+ = other -> if ~OverflowChecks! then ~IntCheckedAdd *self, other else ~IntWrappingAdd *self, other
impl Sub for I8
    @- = other ->
        ~IntCheckedAdd *self, other
        0 as I8
"#).unwrap();
        let artifact = dir.join("operators.rkca");
        let config = rock_lib::Config {
            entry_file: dependency.clone(),
            output_dir: dir.clone(),
            current_crate_name: Some("operators".to_string()),
            overflow_checks: Some(checked),
            no_std: true,
            no_prelude: true,
            no_link: true,
            ..Default::default()
        };
        rock_lib::compile_with_products(&config)
            .unwrap()
            .products
            .unwrap()
            .write_artifact_to_path(&artifact)
            .unwrap();
        fs::remove_file(dependency).unwrap();
        for (expression, invalid, opt_level) in [
            ("(127 as I8) + (1 as I8)", checked),
            ("(127 as I8) - (1 as I8)", true),
            ("(1 as I8) - (2 as I8)", false),
        ]
        .into_iter()
        .flat_map(|(expression, invalid)| [0, 3].map(move |opt| (expression, invalid, opt)))
        {
            let entry = dir.join("consumer.rk");
            fs::write(
                &entry,
                format!("> operators::Add\n> operators::Sub\nmain = !->\n    {expression}\n"),
            )
            .unwrap();
            let result = rock_lib::compile(&rock_lib::Config {
                entry_file: entry,
                current_crate_name: None,
                overflow_checks: Some(!checked),
                opt_level,
                extern_artifacts: vec![("operators".to_string(), artifact.clone())],
                ..config.clone()
            });
            assert_eq!(
                result.is_err(),
                invalid,
                "checks={checked}, {expression}: {result:?}"
            );
            if let Err(errors) = result {
                assert!(
                    errors.0.iter().any(|error| error
                        .message
                        .contains("invalid constant integer arithmetic")),
                    "{errors:?}"
                );
            }
        }
    }
}

#[test]
fn test_constant_integer_arithmetic_keeps_explicit_nontrapping_families() {
    assert_eq!(
        compile_and_run(
            r#"
main = !->
    ((127 as I8).wrapping_add (1 as I8) == (-128 as I8)).println!
    ((127 as I8).saturating_add (1 as I8) == (127 as I8)).println!
    match ((127 as I8).checked_add (1 as I8))
        Option::None => true.println!
        Option::Some value => false.println!
    ((127 as I8).overflowing_add (1 as I8)).1.println!
    ((-128 as I8).wrapping_neg! == (-128 as I8)).println!
    ((~IntWrappingMul (-1 as U64), (-1 as U64)) == (1 as U64)).println!
"#
        ),
        "true\n".repeat(6)
    );
}

#[test]
fn test_constant_arithmetic_composes_selected_methods_across_artifacts() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let library = r#"
< trait Divide
    @divide: I8 -> I8
    @wrapped: I8 -> I8
    @discarded: I8 -> I8
    @ignore: I8 -> I8
    @ignored: I8 -> I8
impl Divide for I8
    @wrapped = other -> self.divide other
    @divide = other -> ~IntCheckedDiv *self, other
    @discarded = other ->
        self.wrapped other
        1 as I8
    @ignore = other -> 1 as I8
    @ignored = other -> self.ignore (self.wrapped other)
"#;
    let entry = dir.join("library.rk");
    fs::write(&entry, library).unwrap();
    let config = rock_lib::Config {
        entry_file: entry.clone(),
        output_dir: dir.clone(),
        current_crate_name: Some("arithmetic".to_string()),
        no_std: true,
        no_prelude: true,
        no_link: true,
        ..Default::default()
    };
    let artifact = dir.join("arithmetic.rkca");
    rock_lib::compile_with_products(&config)
        .unwrap()
        .products
        .unwrap()
        .write_artifact_to_path(&artifact)
        .unwrap();
    fs::remove_file(entry).unwrap();
    for imported in [false, true] {
        for (expression, invalid) in [
            ("(1 as I8).wrapped (0 as I8)", true),
            ("(1 as I8).discarded (0 as I8)", true),
            ("(1 as I8).ignored (0 as I8)", true),
            ("(6 as I8).wrapped (2 as I8)", false),
        ] {
            let entry = dir.join("consumer.rk");
            let prefix = if imported {
                "> arithmetic::Divide"
            } else {
                library
            };
            fs::write(&entry, format!("{prefix}\nmain = !->\n    {expression}\n")).unwrap();
            let result = rock_lib::compile(&rock_lib::Config {
                entry_file: entry,
                current_crate_name: None,
                extern_artifacts: if imported {
                    vec![("arithmetic".to_string(), artifact.clone())]
                } else {
                    Vec::new()
                },
                ..config.clone()
            });
            assert_eq!(
                result.is_err(),
                invalid,
                "imported={imported}, {expression}: {result:?}"
            );
            if let Err(errors) = result {
                assert!(
                    errors.0.iter().any(|error| error
                        .message
                        .contains("invalid constant integer arithmetic")),
                    "{errors:?}"
                );
            }
        }
    }
    let bridge = dir.join("bridge.rk");
    fs::write(
        &bridge,
        r#"
> arithmetic::Divide
< trait Relay
    @relay: I8 -> I8
impl Relay for I8
    @relay = other -> self.wrapped other
"#,
    )
    .unwrap();
    let bridge_artifact = dir.join("bridge.rkca");
    rock_lib::compile_with_products(&rock_lib::Config {
        entry_file: bridge.clone(),
        current_crate_name: Some("bridge".to_string()),
        extern_artifacts: vec![("arithmetic".to_string(), artifact.clone())],
        ..config.clone()
    })
    .unwrap()
    .products
    .unwrap()
    .write_artifact_to_path(&bridge_artifact)
    .unwrap();
    fs::remove_file(bridge).unwrap();
    let entry = dir.join("consumer.rk");
    fs::write(
        &entry,
        "> bridge::Relay\nmain = !->\n    (1 as I8).relay (0 as I8)\n",
    )
    .unwrap();
    let errors = rock_lib::compile(&rock_lib::Config {
        entry_file: entry,
        current_crate_name: None,
        extern_artifacts: vec![
            ("arithmetic".to_string(), artifact),
            ("bridge".to_string(), bridge_artifact),
        ],
        ..config
    })
    .unwrap_err();
    assert!(
        errors.0.iter().any(|error| error
            .message
            .contains("invalid constant integer arithmetic")),
        "{errors:?}"
    );
    compile_should_fail(
        "main = !->\n    (1 as I8).wrapping_div (0 as I8)\n",
        "invalid constant integer arithmetic",
    );
}

#[test]
fn test_overflow_policy_is_independent_of_optimization_and_survives_artifacts() {
    for opt_level in [0, 3] {
        for override_checks in [None, Some(false), Some(true)] {
            let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = test_temp_dir(id);
            fs::create_dir_all(&dir).unwrap();
            let entry = dir.join("policy.rk");
            fs::write(&entry, r#"
infix 9 +
trait Add
    @+: Self -> Self
impl Add for I8
    @+ = other -> if ~OverflowChecks! then ~IntCheckedAdd *self, other else ~IntWrappingAdd *self, other
main = ->
    x = 127 as I8
    wrapped = x + (1 as I8)
    if ~I8Eq wrapped, (~I8Not (127 as I8)) then 0 else 1
"#).unwrap();
            let config = rock_lib::Config {
                entry_file: entry,
                output_dir: dir.clone(),
                opt_level,
                overflow_checks: override_checks,
                no_std: true,
                no_prelude: true,
                ..Default::default()
            };
            rock_lib::compile(&config).unwrap();
            let result = Command::new(dir.join("policy")).output().unwrap();
            let checked = override_checks.unwrap_or(opt_level == 0);
            assert_eq!(
                result.status.success(),
                !checked,
                "optimization={opt_level}, checks={override_checks:?}"
            );

            let dependency = dir.join("dependency.rk");
            fs::write(
                &dependency,
                r#"
compute: I8 -> I8
< compute = value ->
    if ~OverflowChecks!
        ~IntCheckedAdd value, (1 as I8)
    else
        ~IntWrappingAdd value, (1 as I8)
"#,
            )
            .unwrap();
            let artifact = dir.join("dependency.rkca");
            let output = rock_lib::compile_with_products(&rock_lib::Config {
                entry_file: dependency.clone(),
                current_crate_name: Some("dependency".to_string()),
                no_link: true,
                emit_object: Some(dir.join("dependency.o")),
                ..config.clone()
            })
            .unwrap();
            output
                .products
                .unwrap()
                .write_artifact_to_path(&artifact)
                .unwrap();
            fs::remove_file(dependency).unwrap();
            let consumer = dir.join("consumer.rk");
            fs::write(
                &consumer,
                r#"
> dependency::compute
main = ->
    result = compute (127 as I8)
    if ~I8Eq result, (~I8Not (127 as I8)) then 0 else 1
"#,
            )
            .unwrap();
            rock_lib::compile(&rock_lib::Config {
                entry_file: consumer,
                overflow_checks: Some(!checked),
                opt_level: if opt_level == 0 { 3 } else { 0 },
                extern_artifacts: vec![("dependency".to_string(), artifact)],
                ..config
            })
            .unwrap();
            let result = Command::new(dir.join("consumer")).output().unwrap();
            assert_eq!(
                result.status.success(),
                !checked,
                "dependency policy changed across artifact boundary"
            );
        }
    }
}

#[test]
fn test_std_release_artifact_keeps_wrapping_operators_in_checked_consumers() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let std_entry = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("std/src/lib.rk");
    let artifact = dir.join("std.rkca");
    rock_lib::compile_with_products(&rock_lib::Config {
        entry_file: std_entry,
        output_dir: dir.clone(),
        current_crate_name: Some("std".to_string()),
        opt_level: 3,
        no_std: true,
        no_link: true,
        emit_object: Some(dir.join("std.o")),
        ..Default::default()
    })
    .unwrap()
    .products
    .unwrap()
    .write_artifact_to_path(&artifact)
    .unwrap();
    let entry = dir.join("consumer.rk");
    fs::write(
        &entry,
        r#"
main = !->
    high = 127 as I8
    low = high + (1 as I8)
    (low == (~I8Not high)).println!
    ((low - (1 as I8)) == high).println!
    ((high * (2 as I8)) == (-2 as I8)).println!
    ((-low) == low).println!
    (((1 as I8) << (8 as I8)) == (1 as I8)).println!
    (((2 as I8) >> (9 as I8)) == (1 as I8)).println!
"#,
    )
    .unwrap();
    for opt_level in [0, 3] {
        rock_lib::compile(&rock_lib::Config {
            entry_file: entry.clone(),
            output_dir: dir.clone(),
            extern_artifacts: vec![("std".to_string(), artifact.clone())],
            opt_level,
            overflow_checks: Some(true),
            ..Default::default()
        })
        .unwrap();
        let output = Command::new(dir.join("consumer")).output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), "true\n".repeat(6));
    }
}

#[test]
fn test_integer_intrinsics_reject_bad_operands_and_invalid_constants() {
    for (expression, expected) in [
        ("~IntOverflowingAdd true, false", "integer operands"),
        (
            "~IntOverflowingAdd (1 as I8), (2 as I16)",
            "integer operands",
        ),
        ("~IntOverflowingNeg (1 as F64)", "integer operands"),
        ("~IntOverflowingMul (1 as I64)", "integer operands"),
        ("~IntCheckedAdd (127 as I8), (1 as I8)", "invalid constant"),
        ("~IntCheckedShl (1 as U8), (8 as U32)", "invalid constant"),
        ("~IntWrappingDiv (1 as I64), (0 as I64)", "invalid constant"),
    ] {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = test_temp_dir(id);
        fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("invalid.rk");
        fs::write(&entry, format!("main = !->\n    {expression}\n")).unwrap();
        let errors = rock_lib::compile(&rock_lib::Config {
            entry_file: entry,
            output_dir: dir,
            no_std: true,
            no_prelude: true,
            ..Default::default()
        })
        .unwrap_err();
        assert!(
            errors
                .0
                .iter()
                .any(|error| error.message.contains(expected)),
            "{expression}: {errors:?}"
        );
    }
}

#[test]
fn test_integer_runtime_failures_never_reach_llvm_poison() {
    use std::os::unix::process::ExitStatusExt;

    for opt_level in [0, 3] {
        for (operation, operand) in [
            ("~IntWrappingDiv (1 as I8), value", "0 as I8"),
            ("~IntOverflowingRem (1 as I8), value", "0 as I8"),
            (
                "~IntCheckedDiv (~I8Not (127 as I8)), value",
                "~I8Not (0 as I8)",
            ),
            (
                "~IntCheckedRem (~I8Not (127 as I8)), value",
                "~I8Not (0 as I8)",
            ),
            ("~IntCheckedNeg value", "~I8Not (127 as I8)"),
            ("~IntCheckedShl (1 as I8), value", "8 as I8"),
            ("~IntCheckedShr (1 as I8), value", "~I8Not (0 as I8)"),
        ] {
            let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = test_temp_dir(id);
            fs::create_dir_all(&dir).unwrap();
            let entry = dir.join("trap.rk");
            let result = if operation.starts_with("~IntOverflowing") {
                "(I8, Bool)"
            } else {
                "I8"
            };
            fs::write(
                &entry,
                format!(
                "bad: I8 -> {result}\nbad = value -> {operation}\nmain = !->\n    bad ({operand})\n"
            ),
            )
            .unwrap();
            rock_lib::compile(&rock_lib::Config {
                entry_file: entry,
                output_dir: dir.clone(),
                no_std: true,
                no_prelude: true,
                opt_level,
                overflow_checks: Some(false),
                ..Default::default()
            })
            .unwrap();
            let output = Command::new(dir.join("trap")).output().unwrap();
            assert!(
                output.status.signal().is_some(),
                "{operation} must trap even with checks disabled"
            );
        }
    }
}

#[test]
fn test_integer_arithmetic_families_all_widths() {
    for (ty, bits, signed) in [
        ("I8", 8, true),
        ("I16", 16, true),
        ("I32", 32, true),
        ("I64", 64, true),
        ("U8", 8, false),
        ("U16", 16, false),
        ("U32", 32, false),
        ("U64", 64, false),
    ] {
        let mut source = String::from("main = !->\n");
        let mut count = 0;
        source.push_str(&format!(
            "    z{ty} = 0 as {ty}\n    one{ty} = 1 as {ty}\n    two{ty} = 2 as {ty}\n    lo{ty} = z{ty}.min_value!\n    hi{ty} = z{ty}.max_value!\n"
        ));
        let wrapped_max = if signed {
            format!("lo{ty}")
        } else {
            format!("z{ty}")
        };
        let expressions = [
            format!("(hi{ty}.wrapping_add one{ty}) == {wrapped_max}"),
            format!("(hi{ty}.overflowing_add one{ty}).1"),
            format!("(hi{ty}.saturating_add one{ty}) == hi{ty}"),
            format!("(lo{ty}.wrapping_sub one{ty}) == hi{ty}"),
            format!("(lo{ty}.overflowing_sub one{ty}).1"),
            format!("(lo{ty}.saturating_sub one{ty}) == lo{ty}"),
            format!("(hi{ty}.overflowing_mul two{ty}).1"),
            format!("(hi{ty}.saturating_mul two{ty}) == hi{ty}"),
            format!("(one{ty}.wrapping_shl ({bits} as U32)) == one{ty}"),
            format!("(one{ty}.overflowing_shl ({bits} as U32)).1"),
            format!("(two{ty}.wrapping_shr ({} as U32)) == one{ty}", bits + 1),
            format!("(two{ty}.overflowing_shr ({} as U32)).1", bits + 1),
            format!("(two{ty}.wrapping_div one{ty}) == two{ty}"),
            format!("(two{ty}.wrapping_rem one{ty}) == z{ty}"),
        ];
        for expression in expressions {
            source.push_str(&format!("    ({expression}).println!\n"));
            count += 1;
        }
        for expression in [
            format!("hi{ty}.checked_add one{ty}"),
            format!("lo{ty}.checked_sub one{ty}"),
            format!("hi{ty}.checked_mul two{ty}"),
            format!("one{ty}.checked_div z{ty}"),
            format!("one{ty}.checked_rem z{ty}"),
            format!("one{ty}.checked_shl ({bits} as U32)"),
            format!("one{ty}.checked_shr ({bits} as U32)"),
        ] {
            source.push_str(&format!("    match ({expression})\n        Option::None => true.println!\n        Option::Some value => false.println!\n"));
            count += 1;
        }
        for (expression, expected) in [
            (format!("one{ty}.checked_add one{ty}"), format!("two{ty}")),
            (format!("two{ty}.checked_sub one{ty}"), format!("one{ty}")),
            (format!("one{ty}.checked_mul two{ty}"), format!("two{ty}")),
            (format!("two{ty}.checked_div two{ty}"), format!("one{ty}")),
            (format!("one{ty}.checked_rem two{ty}"), format!("one{ty}")),
            (format!("z{ty}.checked_neg!"), format!("z{ty}")),
            (
                format!("one{ty}.checked_shl (1 as U32)"),
                format!("two{ty}"),
            ),
            (
                format!("two{ty}.checked_shr (1 as U32)"),
                format!("one{ty}"),
            ),
        ] {
            source.push_str(&format!("    match ({expression})\n        Option::None => false.println!\n        Option::Some value => (value == {expected}).println!\n"));
            count += 1;
        }
        if signed {
            source.push_str(&format!("    minus{ty} = z{ty}.wrapping_sub one{ty}\n"));
            for expression in [
                format!("lo{ty}.wrapping_neg! == lo{ty}"),
                format!("lo{ty}.overflowing_neg!.1"),
                format!("lo{ty}.saturating_neg! == hi{ty}"),
                format!("(lo{ty}.wrapping_div minus{ty}) == lo{ty}"),
                format!("(lo{ty}.overflowing_div minus{ty}).1"),
                format!("(lo{ty}.wrapping_rem minus{ty}) == z{ty}"),
                format!("(lo{ty}.overflowing_rem minus{ty}).1"),
                format!("(lo{ty}.saturating_div minus{ty}) == hi{ty}"),
                format!("(minus{ty}.wrapping_shr (1 as U32)) == minus{ty}"),
                format!("(lo{ty}.saturating_mul two{ty}) == lo{ty}"),
            ] {
                source.push_str(&format!("    ({expression}).println!\n"));
                count += 1;
            }
            for expression in [
                format!("lo{ty}.checked_neg!"),
                format!("lo{ty}.checked_div minus{ty}"),
                format!("lo{ty}.checked_rem minus{ty}"),
            ] {
                source.push_str(&format!("    match ({expression})\n        Option::None => true.println!\n        Option::Some value => false.println!\n"));
                count += 1;
            }
        }
        for opt_level in [0, 3] {
            assert_eq!(
                compile_and_run_at_opt(&source, opt_level),
                "true\n".repeat(count),
                "{ty}, O{opt_level}"
            );
        }
    }
}

#[test]
fn test_show_all_integer_widths_and_unit() {
    let source = r#"
main = !->
    i8_zero = 0 as I8
    i16_zero = 0 as I16
    u16_zero = 0 as U16
    u32_zero = 0 as U32
    %println (i8_zero.min_value!), ",", (i8_zero.max_value!), ",", (i16_zero.min_value!), ",", (i16_zero.max_value!)
    %println u16_zero, ",", (u16_zero.max_value!), ",", u32_zero, ",", (u32_zero.max_value!)
    unit_value = ()
    %println unit_value, ",", (Option::Some unit_value)
    (%format "unit=", unit_value).println!
"#;
    for opt_level in [0, 3] {
        assert_eq!(
            compile_and_run_at_opt(source, opt_level),
            "-128,127,-32768,32767\n0,65535,0,4294967295\n(),Some(())\nunit=()\n"
        );
    }
}

#[test]
fn test_u64_show_formats_full_range() {
    let source = r#"
main = !->
    zero = 0 as U64
    u64_max = zero.max_value!
    signed_max = ((0 as I64).max_value!) as U64
    above_signed_max = signed_max + (1 as U64)
    %println zero, "\n", (9 as U64), "\n", (10 as U64), "\n", signed_max, "\n", above_signed_max, "\n", u64_max
    u64_max.println!
    u64_max.show!.println!
    (%format "max=", u64_max).println!
"#;
    for opt_level in [0, 3] {
        assert_eq!(compile_and_run_at_opt(source, opt_level),
            "0\n9\n10\n9223372036854775807\n9223372036854775808\n18446744073709551615\n18446744073709551615\n18446744073709551615\nmax=18446744073709551615\n");
    }
}

#[test]
fn test_integer_minimum_formatting_never_negates_the_minimum() {
    let source = r#"
main = !->
    smallest = (0 as I64).min_value!
    smallest.println!
    smallest.show!.println!
    ((0 as I64).max_value!).println!
    (-1).println!
    0.println!
"#;
    for opt_level in [0, 3] {
        assert_eq!(
            compile_and_run_at_opt(source, opt_level),
            "-9223372036854775808\n-9223372036854775808\n9223372036854775807\n-1\n0\n"
        );
    }
}

#[test]
fn test_std_primitive_arithmetic_and_ordering_complete_integer_coverage() {
    let mut source = String::from("main = !->\n");
    for ty in ["I8", "I16", "I32", "I64", "U8", "U16", "U32", "U64"] {
        source.push_str(&format!(
            "    a_{ty} = 12 as {ty}\n    b_{ty} = 5 as {ty}\n"
        ));
        for (operator, expected) in [("+", 17), ("-", 7), ("*", 60), ("/", 2), ("%", 2)] {
            source.push_str(&format!(
                "    ((a_{ty} {operator} b_{ty}) == ({expected} as {ty})).println!\n"
            ));
        }
        for expression in [
            format!("b_{ty} < a_{ty}"),
            format!("b_{ty} <= a_{ty}"),
            format!("a_{ty} > b_{ty}"),
            format!("a_{ty} >= b_{ty}"),
            format!("a_{ty} >= a_{ty}"),
            format!("a_{ty} <= a_{ty}"),
        ] {
            source.push_str(&format!("    ({expression}).println!\n"));
        }
    }
    source.push_str(
        r#"
    ((-(7 as I8)) == ((0 as I8) - (7 as I8))).println!
    ((-(7 as I16)) == ((0 as I16) - (7 as I16))).println!
    ((255 as U8) > (127 as U8)).println!
    ((65535 as U16) > (32767 as U16)).println!
    ((4294967295 as U32) > (2147483647 as U32)).println!
    high = !(0 as U64)
    (high > (9223372036854775807 as U64)).println!
    (((255 as U8) / (2 as U8)) == (127 as U8)).println!
    (((-8 as I8) >> (1 as I8)) == (-4 as I8)).println!
    (((128 as U8) >> (1 as U8)) == (64 as U8)).println!
"#,
    );
    assert_eq!(compile_and_run(&source), "true\n".repeat(97));
}

#[test]
fn test_std_bitwise_traits_are_independent_and_support_bool() {
    let output = compile_and_run(
        r#"
struct Flag
    < value: Bool

impl BitAnd Bool for Flag
    type Output = Bool
    @& = other -> self.value & other

combine: T -> T -> () where T: BitXor T
combine = a, b !-> a ^ b

main = !->
    flag = Flag
        value: true
    (flag & false).println!
    (true & true).println!
    (false | true).println!
    combine true, false
    combine (5 as U16), (3 as U16)
    (true ^ false).println!
    (true ^ true).println!
    (((5 as U16) ^ (3 as U16)) == (6 as U16)).println!
"#,
    );
    assert_eq!(output, "false\ntrue\ntrue\ntrue\nfalse\ntrue\n");
}

#[test]
fn test_std_bitwise_bool_shifts_are_rejected() {
    for operator in ["<<", ">>"] {
        compile_should_fail(
            &format!("main = !->\n    (true {operator} false).println!\n"),
            "operator",
        );
    }
}

#[test]
fn test_float_intrinsic_inequality_matches_partial_equality_for_nan() {
    let output = compile_and_run(
        r#"
main = !->
    nan64 = (0 as F64) / (0 as F64)
    nan32 = (0 as F32) / (0 as F32)
    (nan64 != nan64).println!
    (nan32 != nan32).println!
    (~F64Ne nan64, nan64).println!
    (~F32Ne nan32, nan32).println!
    (~F64Ne nan64, (1 as F64)).println!
    (~F32Ne (1 as F32), nan32).println!
    (~F64Ne (0 as F64), (-0.0 as F64)).println!
    (~F32Ne (2 as F32), (2 as F32)).println!
"#,
    );
    assert_eq!(output, "true\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\n");
}

#[test]
fn test_std_partial_and_total_comparison_contracts() {
    let output = compile_and_run(
        r#"
describe: Option Ordering -> I64
describe = comparison ->
    match comparison
        Option::Some order =>
            match order
                Ordering::Less => -1
                Ordering::Equal => 0
                Ordering::Greater => 1
        Option::None => 2

partial: T -> T -> I64 where T: PartialOrd
partial = left, right -> describe (left.partial_cmp &right)

total: T -> T -> Bool where T: Ord
total = left, right -> (left.cmp &right) == Ordering::Less

main = !->
    (partial (3 as U8), (4 as U8)).println!
    (partial (4 as I16), (3 as I16)).println!
    (partial (4 as F64), (4 as F64)).println!
    nan64 = (0 as F64) / (0 as F64)
    nan32 = (0 as F32) / (0 as F32)
    nan64.is_nan!.println!
    nan32.is_nan!.println!
    (partial nan64, (1 as F64)).println!
    (partial (1 as F32), nan32).println!
    (nan64 < nan64).println!
    (nan64 <= nan64).println!
    (nan64 > nan64).println!
    (nan64 >= nan64).println!
    (nan32 == nan32).println!
    (nan32 != nan32).println!
    (partial (0 as F64), (-0.0 as F64)).println!
    infinity = (1 as F64) / (0 as F64)
    (partial infinity, infinity).println!
    (partial (1 as F64), infinity).println!
    (total false, true).println!
    (total 'é', '🦀').println!
    (total "abc", "abd").println!
    (total "ab", "abc").println!
    (total "é", "🦀").println!
    (partial (), ()).println!
    (() == ()).println!
    (() != ()).println!
    (() <= ()).println!
    (() < ()).println!
    (Ordering::Less < Ordering::Equal).println!
"#,
    );
    assert_eq!(output, "-1\n1\n0\ntrue\ntrue\n2\n2\nfalse\nfalse\nfalse\nfalse\nfalse\ntrue\n0\n0\n-1\ntrue\ntrue\ntrue\ntrue\ntrue\n0\ntrue\nfalse\ntrue\nfalse\ntrue\n");
}

#[test]
fn test_std_floats_do_not_satisfy_total_comparison_bounds() {
    for ty in ["F32", "F64"] {
        for bound in ["Eq", "Ord"] {
            compile_should_fail(
                &format!(
                    "require: T -> () where T: {bound}\nrequire = value !-> return\n\nmain = !->\n    require (1 as {ty})\n"
                ),
                bound,
            );
        }
    }
}

#[test]
fn test_std_wrapping_negation_and_pointer_subtraction_boundaries() {
    let output = compile_and_run(
        r#"
struct Empty

main = !->
    smallest = (-9223372036854775807) - 1
    (smallest.wrapping_neg! == smallest).println!
    min8 = (0 as I8).min_value!
    min16 = (0 as I16).min_value!
    (min8.wrapping_neg! == min8).println!
    (min16.wrapping_neg! == min16).println!
    ((((-2147483647 as I32) - (1 as I32)).wrapping_neg!) == ((-2147483647 as I32) - (1 as I32))).println!
    ((7 as I8).wrapping_neg! == (-7 as I8)).println!
    value = 42
    ptr = &value as *I64
    same = unsafe ptr - 0
    ((same as I64) == (ptr as I64)).println!
    forward = unsafe ptr - (-1)
    backward = unsafe forward - 1
    ((backward as I64) == (ptr as I64)).println!
    byte_ptr = ptr as *U8
    far = unsafe byte_ptr - smallest
    returned = unsafe far + smallest
    ((returned as I64) == (byte_ptr as I64)).println!
    empty = Empty
    empty_ptr = &empty as *Empty
    shifted = unsafe empty_ptr - smallest
    ((shifted as I64) == (empty_ptr as I64)).println!
"#,
    );
    assert_eq!(output, "true\n".repeat(9));
}

#[test]
fn test_std_total_equality_requires_partial_equality() {
    compile_should_fail(
        r#"
struct MissingEquality

impl Eq for MissingEquality

main = !-> return
"#,
        "supertrait obligation",
    );
}
