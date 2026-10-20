//! Integration tests for the Rock compiler
//!
//! These tests compile example .rk files and verify their output.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "support/std_cache_key.rs"]
mod std_cache_key;
use std_cache_key::{std_cache_compiler_stamp, std_cache_key};

#[path = "support/callable.rs"]
mod callable;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn macro_a0_10_compiles_utf8_and_terminal_comments_through_lossless_source() {
    assert_eq!(compile_and_run("main = !->\r\n  \"🦀\".println!\r\n// EOF comment"), "🦀\n");
}

#[test]
fn macro_a0_10_source_failures_reach_compiler_diagnostics_with_exact_spans() {
    use rock_lib::diagnostic::DiagnosticLocation;

    for (source, expected) in [
        ("main = -> 42\0ignored", 12..13),
        ("main = -> \"🦀", 10..15),
        ("main = ->\n   42\n", 10..13),
        ("main = -> 42 /* broken", 13..22),
    ] {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = test_temp_dir(id);
        fs::create_dir_all(&dir).unwrap();
        let _cleanup = TestDirCleanup(dir.clone());
        let entry = dir.join("bad.rk");
        fs::write(&entry, source).unwrap();
        let errors = rock_lib::compile(&rock_lib::Config {
            entry_file: entry.clone(), output_dir: dir, no_link: true,
            no_std: true, no_prelude: true, ..Default::default()
        }).unwrap_err();
        assert!(errors.0.iter().any(|diagnostic| matches!(&diagnostic.location,
            DiagnosticLocation::Source(span) if span.file_path == entry && (span.start..span.end) == expected
        )), "{source:?}: {errors:?}");
    }
}

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
        }).unwrap_err();
        assert!(errors.0.iter().any(|error| {
            error.message.contains("invalid constant integer arithmetic")
                && matches!(&error.location, rock_lib::diagnostic::DiagnosticLocation::Source(span) if span.file_path == entry)
        }), "{expression}: {errors:?}");
        if expression.contains("println") {
            let source = fs::read_to_string(&entry).unwrap();
            let diagnostic = errors.0.iter().find(|error| error.message.contains("invalid constant integer arithmetic")).unwrap();
            let rock_lib::diagnostic::DiagnosticLocation::Source(span) = &diagnostic.location else { panic!("missing source span"); };
            let highlighted = &source[span.start..span.end];
            assert!(highlighted.contains('+') && !highlighted.contains("println"), "wrong operation highlighted: {highlighted}");
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
        rock_lib::compile_with_products(&config).unwrap().products.unwrap()
            .write_artifact_to_path(&artifact).unwrap();
        fs::remove_file(dependency).unwrap();
        for (expression, invalid, opt_level) in [
            ("(127 as I8) + (1 as I8)", checked),
            ("(127 as I8) - (1 as I8)", true),
            ("(1 as I8) - (2 as I8)", false),
        ].into_iter().flat_map(|(expression, invalid)| [0, 3].map(move |opt| (expression, invalid, opt))) {
            let entry = dir.join("consumer.rk");
            fs::write(&entry, format!(
                "> operators::Add\n> operators::Sub\nmain = !->\n    {expression}\n"
            )).unwrap();
            let result = rock_lib::compile(&rock_lib::Config {
                entry_file: entry,
                current_crate_name: None,
                overflow_checks: Some(!checked),
                opt_level,
                extern_artifacts: vec![("operators".to_string(), artifact.clone())],
                ..config.clone()
            });
            assert_eq!(result.is_err(), invalid, "checks={checked}, {expression}: {result:?}");
            if let Err(errors) = result {
                assert!(errors.0.iter().any(|error| error.message.contains("invalid constant integer arithmetic")), "{errors:?}");
            }
        }
    }
}

#[test]
fn test_constant_integer_arithmetic_keeps_explicit_nontrapping_families() {
    assert_eq!(compile_and_run(r#"
main = !->
    ((127 as I8).wrapping_add (1 as I8) == (-128 as I8)).println!
    ((127 as I8).saturating_add (1 as I8) == (127 as I8)).println!
    match ((127 as I8).checked_add (1 as I8))
        Option::None => true.println!
        Option::Some value => false.println!
    ((127 as I8).overflowing_add (1 as I8)).1.println!
    ((-128 as I8).wrapping_neg! == (-128 as I8)).println!
    ((~IntWrappingMul (-1 as U64), (-1 as U64)) == (1 as U64)).println!
"#), "true\n".repeat(6));
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
        entry_file: entry.clone(), output_dir: dir.clone(),
        current_crate_name: Some("arithmetic".to_string()),
        no_std: true, no_prelude: true, no_link: true,
        ..Default::default()
    };
    let artifact = dir.join("arithmetic.rkca");
    rock_lib::compile_with_products(&config).unwrap().products.unwrap()
        .write_artifact_to_path(&artifact).unwrap();
    fs::remove_file(entry).unwrap();
    for imported in [false, true] {
        for (expression, invalid) in [
            ("(1 as I8).wrapped (0 as I8)", true),
            ("(1 as I8).discarded (0 as I8)", true),
            ("(1 as I8).ignored (0 as I8)", true),
            ("(6 as I8).wrapped (2 as I8)", false),
        ] {
            let entry = dir.join("consumer.rk");
            let prefix = if imported { "> arithmetic::Divide" } else { library };
            fs::write(&entry, format!("{prefix}\nmain = !->\n    {expression}\n")).unwrap();
            let result = rock_lib::compile(&rock_lib::Config {
                entry_file: entry, current_crate_name: None,
                extern_artifacts: if imported { vec![("arithmetic".to_string(), artifact.clone())] } else { Vec::new() },
                ..config.clone()
            });
            assert_eq!(result.is_err(), invalid, "imported={imported}, {expression}: {result:?}");
            if let Err(errors) = result {
                assert!(errors.0.iter().any(|error| error.message.contains("invalid constant integer arithmetic")), "{errors:?}");
            }
        }
    }
    let bridge = dir.join("bridge.rk");
    fs::write(&bridge, r#"
> arithmetic::Divide
< trait Relay
    @relay: I8 -> I8
impl Relay for I8
    @relay = other -> self.wrapped other
"#).unwrap();
    let bridge_artifact = dir.join("bridge.rkca");
    rock_lib::compile_with_products(&rock_lib::Config {
        entry_file: bridge.clone(), current_crate_name: Some("bridge".to_string()),
        extern_artifacts: vec![("arithmetic".to_string(), artifact.clone())],
        ..config.clone()
    }).unwrap().products.unwrap().write_artifact_to_path(&bridge_artifact).unwrap();
    fs::remove_file(bridge).unwrap();
    let entry = dir.join("consumer.rk");
    fs::write(&entry, "> bridge::Relay\nmain = !->\n    (1 as I8).relay (0 as I8)\n").unwrap();
    let errors = rock_lib::compile(&rock_lib::Config {
        entry_file: entry, current_crate_name: None,
        extern_artifacts: vec![("arithmetic".to_string(), artifact), ("bridge".to_string(), bridge_artifact)],
        ..config
    }).unwrap_err();
    assert!(errors.0.iter().any(|error| error.message.contains("invalid constant integer arithmetic")), "{errors:?}");
    compile_should_fail("main = !->\n    (1 as I8).wrapping_div (0 as I8)\n", "invalid constant integer arithmetic");
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
            assert_eq!(result.status.success(), !checked, "optimization={opt_level}, checks={override_checks:?}");

            let dependency = dir.join("dependency.rk");
            fs::write(&dependency, r#"
compute: I8 -> I8
< compute = value ->
    if ~OverflowChecks!
        ~IntCheckedAdd value, (1 as I8)
    else
        ~IntWrappingAdd value, (1 as I8)
"#).unwrap();
            let artifact = dir.join("dependency.rkca");
            let output = rock_lib::compile_with_products(&rock_lib::Config {
                entry_file: dependency.clone(),
                current_crate_name: Some("dependency".to_string()),
                no_link: true,
                emit_object: Some(dir.join("dependency.o")),
                ..config.clone()
            }).unwrap();
            output.products.unwrap().write_artifact_to_path(&artifact).unwrap();
            fs::remove_file(dependency).unwrap();
            let consumer = dir.join("consumer.rk");
            fs::write(&consumer, r#"
> dependency::compute
main = ->
    result = compute (127 as I8)
    if ~I8Eq result, (~I8Not (127 as I8)) then 0 else 1
"#).unwrap();
            rock_lib::compile(&rock_lib::Config {
                entry_file: consumer,
                overflow_checks: Some(!checked),
                opt_level: if opt_level == 0 { 3 } else { 0 },
                extern_artifacts: vec![("dependency".to_string(), artifact)],
                ..config
            }).unwrap();
            let result = Command::new(dir.join("consumer")).output().unwrap();
            assert_eq!(result.status.success(), !checked, "dependency policy changed across artifact boundary");
        }
    }
}

#[test]
fn test_std_release_artifact_keeps_wrapping_operators_in_checked_consumers() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let std_entry = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("std/src/lib.rk");
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
    }).unwrap().products.unwrap().write_artifact_to_path(&artifact).unwrap();
    let entry = dir.join("consumer.rk");
    fs::write(&entry, r#"
main = !->
    high = 127 as I8
    low = high + (1 as I8)
    (low == (~I8Not high)).println!
    ((low - (1 as I8)) == high).println!
    ((high * (2 as I8)) == (-2 as I8)).println!
    ((-low) == low).println!
    (((1 as I8) << (8 as I8)) == (1 as I8)).println!
    (((2 as I8) >> (9 as I8)) == (1 as I8)).println!
"#).unwrap();
    for opt_level in [0, 3] {
        rock_lib::compile(&rock_lib::Config {
            entry_file: entry.clone(),
            output_dir: dir.clone(),
            extern_artifacts: vec![("std".to_string(), artifact.clone())],
            opt_level,
            overflow_checks: Some(true),
            ..Default::default()
        }).unwrap();
        let output = Command::new(dir.join("consumer")).output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), "true\n".repeat(6));
    }
}

#[test]
fn test_integer_intrinsics_reject_bad_operands_and_invalid_constants() {
    for (expression, expected) in [
        ("~IntOverflowingAdd true, false", "integer operands"),
        ("~IntOverflowingAdd (1 as I8), (2 as I16)", "integer operands"),
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
        }).unwrap_err();
        assert!(errors.0.iter().any(|error| error.message.contains(expected)), "{expression}: {errors:?}");
    }
}

#[test]
fn test_integer_runtime_failures_never_reach_llvm_poison() {
    use std::os::unix::process::ExitStatusExt;

    for opt_level in [0, 3] {
        for (operation, operand) in [
            ("~IntWrappingDiv (1 as I8), value", "0 as I8"),
            ("~IntOverflowingRem (1 as I8), value", "0 as I8"),
            ("~IntCheckedDiv (~I8Not (127 as I8)), value", "~I8Not (0 as I8)"),
            ("~IntCheckedRem (~I8Not (127 as I8)), value", "~I8Not (0 as I8)"),
            ("~IntCheckedNeg value", "~I8Not (127 as I8)"),
            ("~IntCheckedShl (1 as I8), value", "8 as I8"),
            ("~IntCheckedShr (1 as I8), value", "~I8Not (0 as I8)"),
        ] {
            let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = test_temp_dir(id);
            fs::create_dir_all(&dir).unwrap();
            let entry = dir.join("trap.rk");
            let result = if operation.starts_with("~IntOverflowing") { "(I8, Bool)" } else { "I8" };
            fs::write(&entry, format!(
                "bad: I8 -> {result}\nbad = value -> {operation}\nmain = !->\n    bad ({operand})\n"
            )).unwrap();
            rock_lib::compile(&rock_lib::Config {
                entry_file: entry,
                output_dir: dir.clone(),
                no_std: true,
                no_prelude: true,
                opt_level,
                overflow_checks: Some(false),
                ..Default::default()
            }).unwrap();
            let output = Command::new(dir.join("trap")).output().unwrap();
            assert!(output.status.signal().is_some(), "{operation} must trap even with checks disabled");
        }
    }
}

#[test]
fn test_integer_arithmetic_families_all_widths() {
    for (ty, bits, signed) in [
        ("I8", 8, true), ("I16", 16, true), ("I32", 32, true), ("I64", 64, true),
        ("U8", 8, false), ("U16", 16, false), ("U32", 32, false), ("U64", 64, false),
    ] {
        let mut source = String::from("main = !->\n");
        let mut count = 0;
        source.push_str(&format!(
            "    z{ty} = 0 as {ty}\n    one{ty} = 1 as {ty}\n    two{ty} = 2 as {ty}\n    lo{ty} = z{ty}.min_value!\n    hi{ty} = z{ty}.max_value!\n"
        ));
        let wrapped_max = if signed { format!("lo{ty}") } else { format!("z{ty}") };
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
            (format!("one{ty}.checked_shl (1 as U32)"), format!("two{ty}")),
            (format!("two{ty}.checked_shr (1 as U32)"), format!("one{ty}")),
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
            for expression in [format!("lo{ty}.checked_neg!"), format!("lo{ty}.checked_div minus{ty}"), format!("lo{ty}.checked_rem minus{ty}")] {
                source.push_str(&format!("    match ({expression})\n        Option::None => true.println!\n        Option::Some value => false.println!\n"));
                count += 1;
            }
        }
        for opt_level in [0, 3] {
            assert_eq!(compile_and_run_at_opt(&source, opt_level), "true\n".repeat(count), "{ty}, O{opt_level}");
        }
    }
}

#[test]
fn test_integer_minimum_formatting_never_negates_the_minimum() {
    let source = r#"
main = !->
    smallest = (0 as I64).min_value!
    smallest.println!
    (String::from smallest).println!
    ((0 as I64).max_value!).println!
    (-1).println!
    0.println!
"#;
    for opt_level in [0, 3] {
        assert_eq!(compile_and_run_at_opt(source, opt_level),
            "-9223372036854775808\n-9223372036854775808\n9223372036854775807\n-1\n0\n");
    }
}

#[test]
fn test_raw_wide_pointers_and_utf8_constructors() {
    let output = compile_and_run(r#"
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
"#);
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
        fs::write(&dependency, r#"
unsafe borrow: *I64 -> I64 -> &[I64]
< unsafe borrow = data, len ->
    raw: *[I64] = ~PtrFromParts data, len
    unsafe &*raw
unsafe borrow_mut: *I64 -> I64 -> &mut [I64]
< unsafe borrow_mut = data, len ->
    raw: *[I64] = ~PtrFromParts data, len
    unsafe &mut *raw
"#).unwrap();
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
        rock_lib::compile_with_products(&config).unwrap().products.unwrap()
            .write_artifact_to_path(&artifact).unwrap();
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
        fs::write(&entry, r#"
> raw_slices::borrow
main = ->
    values = [1, 2]
    view = unsafe borrow (~ArrPtr (&values)), 2
    if ~I64Eq (~ArrayLen *view), 2 then 0 else 1
"#).unwrap();
        rock_lib::compile(&config).unwrap();
        assert!(Command::new(dir.join("consumer")).status().unwrap().success());
        for (source, expected) in [
            (r#"
> raw_slices::borrow
escape: () -> &[I64]
escape = ->
    values = [1, 2]
    unsafe borrow (~ArrPtr (&values)), 2
main = !->
    escape!
"#, "local"),
            (r#"
> raw_slices::borrow_mut
main = ->
    mut values = [1, 2]
    data = ~ArrPtr (&mut values)
    first = unsafe borrow_mut data, 2
    second = unsafe borrow_mut data, 2
    ~I64Add (~ArrayLen *first), (~ArrayLen *second)
"#, "borrow"),
        ] {
            fs::write(&entry, source).unwrap();
            let diagnostics = rock_lib::compile(&config).unwrap_err();
            assert!(diagnostics.0.iter().any(|error| error.message.contains(expected)), "{diagnostics:?}");
        }
    }
}

#[test]
fn test_raw_wide_pointer_borrow_cannot_escape_local_storage() {
    compile_should_fail(r#"
escape: () -> &[I64]
escape = ->
    values = [1, 2]
    raw: *[I64] = ~PtrFromParts values.as_ptr!, 2
    unsafe &*raw

main = !->
    escaped = escape!
    escaped[0].println!
"#, "local");
}

#[test]
fn test_raw_wide_pointer_borrows_preserve_exclusivity() {
    compile_should_fail(r#"
main = !->
    mut values = [1, 2]
    raw: *[I64] = ~PtrFromParts values.as_mut_ptr!, 2
    first = unsafe &mut *raw
    second = unsafe &mut *raw
    first[0].println!
    second[0].println!
"#, "borrow");
    compile_should_fail(r#"
main = !->
    mut values = [1, 2]
    data = values.as_mut_ptr!
    first = unsafe slice_from_raw_parts_mut data, 2
    second = unsafe slice_from_raw_parts_mut data, 2
    first[0].println!
    second[0].println!
"#, "borrow");
    compile_should_fail(r#"
main = !->
    mut values = [1, 2]
    raw: *[I64] = ~PtrFromParts values.as_ptr!, 2
    view = unsafe &*raw
    values[0] = 9
    view[0].println!
"#, "borrow");
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
    let output = compile_and_run(r#"
struct Empty
main = !->
    empty = Empty
    data = &empty as *Empty
    view = unsafe slice_from_raw_parts data, 9223372036854775807
    view.len!.println!
"#);
    assert_eq!(output, "9223372036854775807\n");
}

#[test]
fn test_raw_pointer_projected_borrows_retain_storage_loans() {
    for pointer in ["holder.ptr", "pair.0"] {
        let source = format!(r#"
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
"#);
        compile_should_fail(&source, "borrow conflict");
        assert_eq!(compile_and_run(&source.replace("    values[0] = 9\n", "")
            .replace("    view[0]\n", "    view[0].println!\n    0\n")), "1\n");
    }
}

#[test]
fn test_return_loan_mixed_inputs_preserve_shared_access() {
    for (result, value, shared, exclusive) in [
        ("(&I64, &mut I64)", "(a, b)", "pair.0", "pair.1"),
        ("((&I64, I64), &mut I64)", "((a, 0), b)", "(pair.0).0", "pair.1"),
        ("Views", "Views\n        shared: a\n        exclusive: b", "pair.shared", "pair.exclusive"),
    ] {
        let source = format!(r#"
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
"#);
        assert_eq!(compile_and_run(&source), "2\n7\n");
        for mutation in ["a = 9", "b = 9"] {
            compile_should_fail(&source.replace("    pair = split &a, &mut b\n",
                &format!("    pair = split &a, &mut b\n    {mutation}\n")), "borrow conflict");
        }
    }
}

#[test]
fn test_return_loan_raw_forwarding_preserves_selected_authority() {
    for selected in ["first", "second"] {
        let unselected = if selected == "first" { "b" } else { "a" };
        let source = format!(r#"
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
"#);
        assert_eq!(compile_and_run(&source), "7\n7\n");
        compile_should_fail(&source.replace("    *view = 7", &format!("    {} = 9",
            if selected == "first" { "a" } else { "b" })), "borrow conflict");
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
    fs::write(&dependency, r#"
split: &I64 -> &mut I64 -> (&I64, &mut I64)
< split = a, b -> (a, b)

identity: *I64 -> *I64
< identity = ptr -> ptr
"#).unwrap();
    let config = rock_lib::Config {
        entry_file: dependency.clone(), output_dir: dir.clone(),
        current_crate_name: Some("loans".to_string()),
        no_std: true, no_prelude: true, no_link: true,
        ..Default::default()
    };
    let artifact = dir.join("loans.rkca");
    rock_lib::compile_with_products(&config).unwrap().products.unwrap()
        .write_artifact_to_path(&artifact).unwrap();
    fs::remove_file(dependency).unwrap();
    for (source, exit_code) in [(r#"
> loans::split
main = ->
    mut a = 1
    mut b = 2
    shared = &a
    pair = split &a, &mut b
    *pair.1 = 7
    result = ~I64Add *shared, *pair.0
    ~I64Add result, *pair.1
"#, 9), (r#"
> loans::identity
main = ->
    mut value = 1
    ptr = &mut value as *I64
    forwarded = identity ptr
    view = unsafe &mut *forwarded
    *view = 7
    result = *view
    unsafe ~I64Add result, *ptr
"#, 14)] {
        for opt_level in [0, 3] {
            for invalid in [false, true] {
                let entry = dir.join("consumer.rk");
                let source = if invalid {
                    source.replace("    *pair.1 = 7", "    a = 9\n    *pair.1 = 7")
                        .replace("    view = unsafe &mut *forwarded",
                            "    sibling = unsafe &*ptr\n    view = unsafe &mut *forwarded\n    *sibling")
                } else { source.to_string() };
                fs::write(&entry, source).unwrap();
                let result = rock_lib::compile(&rock_lib::Config {
                    entry_file: entry, current_crate_name: None, opt_level,
                    extern_artifacts: vec![("loans".to_string(), artifact.clone())],
                    no_link: invalid,
                    ..config.clone()
                });
                if invalid {
                    let errors = result.unwrap_err();
                    assert!(errors.0.iter().any(|error| error.message.contains("borrow conflict")), "{errors:?}");
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
    compile_should_fail(r#"
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
"#, "borrow conflict");
}

#[test]
fn test_raw_pointer_aggregate_arguments_retain_return_loans() {
    for reference in ["&I64", "&mut I64"] {
        for (argument, projection, value) in [
            ("Holder", "holder.ptr", "Holder\n        ptr: &mut value as *I64"),
            ("(*I64, I64)", "holder.0", "(&mut value as *I64, 0)"),
            ("((*I64, I64), Bool)", "(holder.0).0", "((&mut value as *I64, 0), true)"),
        ].into_iter().chain((reference == "&I64").then_some(
            ("[*I64; 1]", "holder[0]", "[&mut value as *I64]"),
        )) {
            let source = format!(r#"
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
"#, reference_expr = if reference == "&I64" { "&" } else { "&mut " });
            compile_should_fail(&source, "borrow conflict");
            let valid = source.replace("    value = 9\n", "");
            assert_eq!(compile_and_run(&valid), "1\n");
            if reference == "&mut I64" {
                assert_eq!(compile_and_run(&valid.replace("    (*view).println!", "    *view = 7\n    (*view).println!")), "7\n");
            }
        }
    }
}

#[test]
fn test_raw_pointer_aggregate_results_retain_return_loans() {
    for reference in ["&I64", "&mut I64"] {
        for (result, value, projection) in [
            (format!("({reference}, I64)"), "(view, 0)", "pair.0"),
            (format!("(({reference}, I64), Bool)"), "((view, 0), true)", "(pair.0).0"),
            ("View".to_string(), "View\n        value: view", "pair.value"),
        ].into_iter().chain((reference == "&I64").then_some(
            (format!("[{reference}; 1]"), "[view]", "pair[0]"),
        )) {
            let source = format!(r#"
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
"#, reference_expr = if reference == "&I64" { "&" } else { "&mut " });
            compile_should_fail(&source, "borrow conflict");
            let valid = source.replace("    value = 9\n", "");
            assert_eq!(compile_and_run(&valid), "1\n");
            if reference == "&mut I64" {
                let write = valid.replace(&format!("    (*{projection}).println!"),
                    &format!("    *{projection} = 7\n    (*{projection}).println!"));
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
        let source = format!(r#"
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
"#, reference_expr = if reference == "&I64" { "&" } else { "&mut " });
        compile_should_fail(&source, "borrow conflict");
        assert_eq!(compile_and_run(&source.replace("    value = 9\n", "")), "1\n");
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
    fs::write(&dependency, r#"
< struct Holder
    < ptr: *I64

unsafe borrow: Holder -> (&I64, I64)
< unsafe borrow = holder -> (unsafe &*holder.ptr, 0)

unsafe borrow_mut: Holder -> (&mut I64, I64)
< unsafe borrow_mut = holder -> (unsafe &mut *holder.ptr, 0)
"#).unwrap();
    let config = rock_lib::Config {
        entry_file: dependency.clone(), output_dir: dir.clone(),
        current_crate_name: Some("loans".to_string()),
        no_std: true, no_prelude: true, no_link: true,
        ..Default::default()
    };
    let artifact = dir.join("loans.rkca");
    rock_lib::compile_with_products(&config).unwrap().products.unwrap()
        .write_artifact_to_path(&artifact).unwrap();
    fs::remove_file(dependency).unwrap();
    for method in ["borrow", "borrow_mut"] {
        for invalid in [false, true] {
            let entry = dir.join("consumer.rk");
            let mutation = if invalid { "value = 9" } else if method == "borrow_mut" { "*pair.0 = 7" } else { "*pair.0" };
            fs::write(&entry, format!(r#"
> loans::Holder
> loans::{method}

main = ->
    mut value = 1
    holder = Holder
        ptr: &mut value as *I64
    pair = unsafe {method} holder
    {mutation}
    *pair.0
"#)).unwrap();
            let result = rock_lib::compile(&rock_lib::Config {
                entry_file: entry, current_crate_name: None,
                extern_artifacts: vec![("loans".to_string(), artifact.clone())],
                ..config.clone()
            });
            assert_eq!(result.is_err(), invalid, "{method}, {mutation}: {result:?}");
            if let Err(errors) = result {
                assert!(errors.0.iter().any(|error| error.message.contains("borrow conflict")), "{errors:?}");
            }
        }
    }
}

#[test]
fn test_raw_pointer_return_reborrow_uses_selected_argument() {
    for selected in ["first", "second"] {
        let source = format!(r#"
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
"#, unselected = if selected == "first" { "b" } else { "a" });
        assert_eq!(compile_and_run(&source), "9\n9\n");
        let conflicting = source.replace("    *view = 9", &format!("    {} = 9", if selected == "first" { "a" } else { "b" }));
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
    fs::write(&dependency, r#"
unsafe choose: &I64 -> *I64 -> Bool -> &I64
< unsafe choose = first, second, flag ->
    if flag then first else unsafe &*second

unsafe second: *I64 -> *I64 -> &mut I64
< unsafe second = first, second -> unsafe &mut *second
"#).unwrap();
    let config = rock_lib::Config {
        entry_file: dependency.clone(), output_dir: dir.clone(),
        current_crate_name: Some("loans".to_string()),
        no_std: true, no_prelude: true, no_link: true,
        ..Default::default()
    };
    let artifact = dir.join("loans.rkca");
    rock_lib::compile_with_products(&config).unwrap().products.unwrap()
        .write_artifact_to_path(&artifact).unwrap();
    fs::remove_file(dependency).unwrap();
    for (call, mutation, invalid) in [
        ("choose &a, holder.ptr, false", "b = 9", true),
        ("choose &a, holder.ptr, false", "*view", false),
        ("second (&mut a as *I64), (holder.ptr)", "*view = 9", false),
        ("second (&mut a as *I64), (holder.ptr)", "b = 9", true),
    ] {
        let entry = dir.join("consumer.rk");
        fs::write(&entry, format!(r#"
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
"#)).unwrap();
        let result = rock_lib::compile(&rock_lib::Config {
            entry_file: entry, current_crate_name: None,
            extern_artifacts: vec![("loans".to_string(), artifact.clone())],
            ..config.clone()
        });
        assert_eq!(result.is_err(), invalid, "{call}, {mutation}: {result:?}");
        if let Err(errors) = result {
            assert!(errors.0.iter().any(|error| error.message.contains("borrow conflict")), "{errors:?}");
        }
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
    source.push_str(r#"
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
"#);
    assert_eq!(compile_and_run(&source), "true\n".repeat(97));
}

#[test]
fn test_std_bitwise_traits_are_independent_and_support_bool() {
    let output = compile_and_run(r#"
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
"#);
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
    let output = compile_and_run(r#"
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
"#);
    assert_eq!(output, "true\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\n");
}

#[test]
fn test_std_partial_and_total_comparison_contracts() {
    let output = compile_and_run(r#"
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
"#);
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
    let output = compile_and_run(r#"
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
"#);
    assert_eq!(output, "true\n".repeat(9));
}

#[test]
fn test_std_total_equality_requires_partial_equality() {
    compile_should_fail(r#"
struct MissingEquality

impl Eq for MissingEquality

main = !-> return
"#, "supertrait obligation");
}

#[test]
fn test_removed_make_arr_intrinsic_is_rejected() {
    compile_should_fail(r#"
main = !->
    values = [1, 2]
    unsafe ~MakeArr values.as_ptr!, 2
"#, "MakeArr");
}

#[test]
fn test_mutable_raw_slice_borrow_requires_unsafe() {
    compile_should_fail(r#"
main = !->
    mut values = [1, 2]
    raw: *[I64] = ~PtrFromParts values.as_mut_ptr!, 2
    view = &mut *raw
    view[0].println!
"#, "requires an unsafe block");
}

#[test]
fn test_slice_methods_lengths_and_empty_views() {
    let output = compile_and_run(r#"
inspect: &[I64] -> ()
inspect = values !->
    values.len!.println!
    values.is_empty!.println!

main = !->
    mut values = [10, 20, 30]
    values.len!.println!
    values.is_empty!.println!
    inspect &values
    inspect &values[1..]
    inspect &values[3..3]
    view = &mut values[..]
    view.len!.println!
    view.is_empty!.println!
    ptr = view.as_mut_ptr!
    unsafe *ptr = 42
    view[0].println!
    view.len!.println!
    empty: [U8; 0] = []
    empty.len!.println!
    empty.is_empty!.println!
"#);
    assert_eq!(output, "3\nfalse\n3\nfalse\n2\nfalse\n0\ntrue\n3\nfalse\n42\n3\n0\ntrue\n");
}

#[test]
fn test_slice_methods_pointer_identity_and_mutation() {
    let output = compile_and_run(r#"
main = !->
    mut values = [10, 20, 30]
    original = values.as_ptr! as I64
    borrowed = (&values).as_ptr! as I64
    (original == borrowed).println!
    tail = &values[1..]
    offset = unsafe ~PtrOffset values.as_ptr!, 1
    ((tail.as_ptr! as I64) == (offset as I64)).println!
    ptr = values.as_mut_ptr!
    ((ptr as I64) == original).println!
    unsafe *ptr = 42
    values[0].println!
    values[1].println!
"#);
    assert_eq!(output, "true\ntrue\ntrue\n42\n20\n");
}

#[test]
fn test_slice_methods_do_not_move_non_copy_elements() {
    let output = compile_and_run(r#"
main = !->
    values = [(String::from "first"), (String::from "second")]
    values.len!.println!
    ptr = values.as_ptr!
    (unsafe &*ptr).println!
    values[1].println!
    values.len!.println!
"#);
    assert_eq!(output, "2\nfirst\nsecond\n2\n");
}

#[test]
fn test_slice_methods_borrowed_str_bytes_and_pointer() {
    let output = compile_and_run(r#"
main = !->
    text = "aé🦀"
    text.len!.println!
    text.is_empty!.println!
    "".is_empty!.println!
    bytes = text.as_bytes!
    bytes.len!.println!
    bytes[0].println!
    bytes[1].println!
    ((text.as_ptr! as I64) == (bytes.as_ptr! as I64)).println!
    owned = String::from text
    owned.as_str!.as_bytes!.len!.println!
    owned.println!
"#);
    assert_eq!(output, "7\nfalse\ntrue\n7\n97\n195\ntrue\n7\naé🦀\n");
}

#[test]
fn test_slice_methods_shared_slice_cannot_get_mut_pointer() {
    compile_should_fail(r#"
main = !->
    values = [1, 2]
    shared = &values[..]
    ptr = shared.as_mut_ptr!
    unsafe *ptr = 42
"#, "as_mut_ptr");
}

#[test]
fn test_slice_methods_immutable_array_cannot_get_mut_pointer() {
    compile_should_fail(r#"
main = !->
    values = [1, 2]
    ptr = values.as_mut_ptr!
    unsafe *ptr = 42
"#, "mutable receiver");
}

#[test]
fn test_slice_methods_byte_view_cannot_escape_local_string() {
    compile_should_fail(r#"
make_bytes: () -> &[U8]
make_bytes = ->
    text = String::from "local"
    text.as_str!.as_bytes!

main = !->
    make_bytes!.len!.println!
"#, "does not live long enough");
}

#[test]
fn test_std_sizeof_borrows_values_and_queries_pointees() {
    let source = r#"
> std::mem::sizeof
> std::mem::sizeof_pointee
> std::mem::Sized
> std::mem::Box
> std::mem::Drop
> std::show::Show

inspect: &T -> I64 where T: Sized
inspect = value -> sizeof value

slice_view: &[I64] -> &[I64]
slice_view = values -> values

struct Empty

struct Pair
    < small: U8
    < large: I64

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = !-> self.value.println!

main = !->
    (sizeof &(0 as U8)).println!
    (sizeof &(0 as U16)).println!
    (sizeof &(0 as U32)).println!
    (sizeof &(0 as I64)).println!
    (sizeof &true).println!
    (sizeof &'🦀').println!
    (sizeof &Empty).println!
    values = [1, 2, 3]
    (inspect &values).println!
    view = slice_view &values
    (sizeof &view).println!
    pair = Pair
        small: 1 as U8
        large: 2
    (sizeof &pair).println!
    null_pair = 0 as *Pair
    (sizeof_pointee null_pair).println!
    null_empty = 0 as *Empty
    (sizeof_pointee null_empty).println!
    null_integer = 0 as *I64
    (sizeof &null_integer).println!
    (sizeof_pointee null_integer).println!
    value = Tracked
        value: 73
    (inspect &value).println!
    value.value.println!
    boxed = Box::new value
    (sizeof boxed.as_ref!).println!
    "end".println!
"#;
    for opt_level in [0, 3] {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = test_temp_dir(id);
        fs::create_dir_all(&dir).unwrap();
        let _cleanup = TestDirCleanup(dir.clone());
        let entry = dir.join("sizeof.rk");
        fs::write(&entry, source).unwrap();
        let mut config = test_config(entry, dir.clone());
        config.opt_level = opt_level;
        config.no_prelude = true;
        rock_lib::compile(&config).unwrap();
        let output = run_test_command(&mut Command::new(dir.join("sizeof")));
        assert_eq!(output.status.code(), Some(0), "optimization level {opt_level}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "1\n2\n4\n8\n1\n4\n0\n24\n16\n16\n16\n0\n8\n8\n8\n73\n8\nend\n73\n",
            "optimization level {opt_level}",
        );
    }
}

#[test]
fn test_std_sizeof_rejects_unsized_pointees() {
    for source in [
        r#"
main = !->
    values = [1, 2, 3]
    slice = &values[..]
    sizeof slice
"#,
        r#"
main = !->
    sizeof "hello"
"#,
        r#"
main = !->
    values = [1, 2, 3]
    slice = &values[..]
    sizeof_pointee (slice as *[I64])
"#,
    ] {
        compile_should_fail(source, "Sized");
    }
}

#[test]
fn test_std_domain_facades_without_prelude() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("facades.rk");
    std::fs::write(
        &entry,
        r#"
> std::collections::Vec
> std::control::Option
> std::control::Result
> std::control::ControlFlow
> std::mem::Box
> std::mem::sizeof
> std::string::String
> std::functional::Functor
> std::functional::fmap
> std::cmp::Eq
> std::num::Add
> std::ops::Index
> std::ops::Deref
> std::concurrency::Mutex
> std::concurrency::thread::spawn
> std::io::Write
> std::io::fs::File
> std::io::net::Ipv4Addr
> std::ffi::libc::sqrt

first: &Vec I64 -> I64
first = values -> values[0]

main = ->
    mut values: Vec I64 = Vec::new!
    values.push 42
    boxed: Box I64 = Box::new (first &values)
    selected: Option I64 = fmap (value -> value), (Option::Some *boxed)
    match selected
        Option::Some value => value
        Option::None => 0
"#,
    )
    .unwrap();
    let mut config = test_config(entry, dir.clone());
    config.no_prelude = true;
    rock_lib::compile(&config).expect("domain facades should work without prelude imports");
    let output = Command::new(dir.join("facades")).output().unwrap();
    assert_eq!(output.status.code(), Some(42));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_std_domain_facades_with_prelude() {
    let output = compile_and_run(
        r#"
> std::collections::Vec
> std::control::Option
> std::mem::Box
> std::string::String

main = !->
    mut values: Vec I64 = Vec::new!
    values.push 42
    first: Option I64 = Option::Some values[0]
    first.println!
    boxed = Box::new 42
    (*boxed).println!
    text = String::from "hello"
    text.println!
"#,
    );
    assert_eq!(output, "Some(42)\n42\nhello\n");
}

#[test]
fn test_std_old_flat_module_paths_are_not_aliases() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("old_paths.rk");
    for (path, value) in [
        ("std::vec::Vec", "values: Vec I64 = Vec::new!"),
        ("std::option::Option", "value: Option I64 = Option::Some 42"),
        ("std::sync::Mutex", "value: Mutex I64 = Mutex::new 42"),
    ] {
        std::fs::write(&entry, format!("> {path}\nmain = !->\n    {value}\n")).unwrap();
        let mut config = test_config(entry.clone(), dir.clone());
        config.no_prelude = true;
        let diagnostics = rock_lib::compile(&config).expect_err("old module paths must not resolve");
        assert!(format!("{diagnostics:?}").contains(path));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

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

#[test]
fn test_utf8_strings_and_range_indexing() {
    let output = compile_and_run(
        r#"
main = !->
    text = "aé中🦀z"
    text.len!.println!
    (&text[1..3]).println!
    (&text[3..=5]).println!
    (&text[6..10]).println!
    (&text[..1]).println!
    (&text[10..]).println!
    (&text[..]).println!
    (&text[11..11]).len!.println!
    owned = String::from text
    (&owned[1..6]).println!
    owned.char_at 3 .println!
    str_char_at text, 5 .println!
    str_char_at text, -1 .println!
    String::from 'é' .println!
    String::from '中' .println!
    String::from '🦀' .println!
    ('🦀' as I64).println!
"#,
    );
    assert_eq!(
        output,
        "11\né\n中\n🦀\na\nz\naé中🦀z\n0\né中\nSome(🦀)\nNone\nNone\né\n中\n🦀\n129408\n"
    );
}

#[test]
fn test_utf8_validation() {
    for bytes in [
        vec![0x80],
        vec![0xc0, 0x80],
        vec![0xc2],
        vec![0xe0, 0x80, 0x80],
        vec![0xed, 0xa0, 0x80],
        vec![0xf0, 0x80, 0x80, 0x80],
        vec![0xf4, 0x90, 0x80, 0x80],
        vec![0xf5, 0x80, 0x80, 0x80],
        vec![0xc2, 0x41],
        vec![0xff],
    ] {
        let values = bytes
            .iter()
            .map(|b| format!("{b} as U8"))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!(
            "main = !->\n    bytes = [{values}]\n    String::from_utf8 (&bytes) .println!\n"
        );
        assert_eq!(compile_and_run(&source), "None\n", "{bytes:?}");
    }
    assert_eq!(
        compile_and_run(
            r#"
main = !->
    bytes = [240 as U8, 159 as U8, 166 as U8, 128 as U8]
    String::from_utf8 (&bytes) .println!
    empty: [U8; 0] = []
    String::from_utf8 (&empty) .println!
"#
        ),
        "Some(🦀)\nSome()\n"
    );
}

#[test]
fn test_utf8_invalid_ranges_are_checked() {
    for range in [
        "2..3",
        "1..2",
        "2..2",
        "-1..1",
        "3..1",
        "0..5",
        "..=4",
        "..=9223372036854775807",
    ] {
        for receiver in ["\"aéz\"", "String::from \"aéz\""] {
            let source =
                format!("main = !->\n    text = {receiver}\n    (&text[{range}]).println!\n");
            let (output, success) = compile_and_run_with_status(&source);
            assert!(!success, "{receiver}[{range}] unexpectedly succeeded");
            assert!(
                output.contains("out of bounds") || output.contains("UTF-8 boundary"),
                "{output}"
            );
        }
    }
}

#[test]
fn test_utf8_char_width_escapes_and_nul() {
    let output = compile_and_run(
        r#"
echo: Char -> Char
echo = value -> value
main = !->
    chars = ['é', '中', '🦀']
    chars[2].println!
    echo '中' .println!
    ('\n' as I64).println!
    ('\'' as I64).println!
    ('\\' as I64).println!
    text = String::from "é\0🦀"
    text.len!.println!
    text.println!
    str_char_at "é", 1 .println!
    String::from (1114111 as Char) .len!.println!
    match '\n'
        '\n' => 1.println!
        _ => 0.println!
"#,
    );
    assert_eq!(output, "🦀\n中\n10\n39\n92\n7\né\0🦀\nSome(́)\n4\n1\n");
}

#[test]
fn test_utf8_invalid_char_casts_trap_before_narrowing() {
    for value in [-1_i64, 0xd800, 0xdfff, 0x110000, 0x100000041] {
        let source = format!("main = !->\n    value = {value}\n    (value as Char).println!\n");
        assert!(
            !compile_and_run_with_status(&source).1,
            "{value} must not form a Char"
        );
    }
}

#[test]
fn test_utf8_invalid_byte_conversion_is_checked() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = !->
    bytes = [255 as U8]
    String::from (&bytes) .println!
"#,
    );
    assert!(!success);
    assert_eq!(output, "invalid UTF-8\n");
}

#[test]
fn test_utf8_scalar_boundaries_roundtrip() {
    let scalars = [
        0_u32, 0x7f, 0x80, 0x7ff, 0x800, 0xd7ff, 0xe000, 0xffff, 0x10000, 0x10ffff,
    ];
    let mut source = String::from("main = !->\n");
    let mut expected = String::new();
    for (i, scalar) in scalars.into_iter().enumerate() {
        source.push_str(&format!(
            "    text{i} = String::from ({scalar} as Char)\n    is_utf8 text{i}.as_bytes! .println!\n    text{i}.len!.println!\n    decoded{i} = text{i}.char_at 0 .unwrap_or 'x'\n    (decoded{i} as I64).println!\n"
        ));
        expected.push_str(&format!(
            "true\n{}\n{scalar}\n",
            char::from_u32(scalar).unwrap().len_utf8()
        ));
    }
    assert_eq!(compile_and_run(&source), expected);
}

#[test]
fn test_utf8_char_literals_require_one_scalar() {
    for literal in ["''", "'ab'", "'é'", "'\\q'"] {
        compile_should_fail(
            &format!("main = !->\n    {literal}.println!\n"),
            "character literal must contain exactly one Unicode scalar",
        );
    }
}

#[test]
fn test_utf8_strings_do_not_allow_byte_mutation_or_integer_indexing() {
    compile_should_fail(
        r#"
main = !->
    text = String::from "é"
    text[0].println!
"#,
        "No implementation found for operator '[]'",
    );
    compile_should_fail(
        r#"
main = !->
    mut text = String::from "é"
    text[0..2] = "hi"
"#,
        "No mutable indexing implementation found",
    );
}

#[test]
fn test_utf8_string_slice_cannot_outlive_owner() {
    compile_should_fail(
        r#"
bad: () -> &Str
bad = ->
    text = String::from "é🦀"
    &text[2..]

main = !-> bad!.println!
"#,
        "Cannot return reference",
    );
}

#[test]
fn test_multiline_chain_targets_completed_call() {
    let output = compile_and_run(
        r#"
make = value -> Option::Some value
add = left, right -> Option::Some (left + right)

main = !->
    make 42
        .unwrap_or 0
        .println!
    add 20, 22
        .map (+ 1)
        .unwrap_or 0
        .println!
    make (make 44
        .unwrap_or 0)
        .unwrap_or 0
        .println!
"#,
    );
    assert_eq!(output, "42\n43\n44\n");
}

#[test]
fn test_call_result_propagation_operator_continuations() {
    let output = compile_and_run(
        r#"
infix 1 %%
infix 12 **
%% = left, right -> left + right
** = left, right -> left + right

get: I64 -> Result I64, I64
get = value -> if value < 0 then Result::Err 99 else Result::Ok value

compute: () -> Result I64, I64
compute = ->
    first = get 20? + 1
    second = get 20? %% 1
    third = get 20? ** 1
    fourth = get 20? |> (+ 1)
    same = get 21? == 21
    same.println!
    converted = get 21? as I32
    converted.println!
    Result::Ok (first + second + third + fourth)

never = value ->
    999.println!
    value

fail: () -> Result I64, I64
fail = -> Result::Ok (get -1? |> never)

main = !->
    compute!.println!
    fail!.println!
"#,
    );
    assert_eq!(output, "true\n21\nOk(84)\nErr(99)\n");
}

#[test]
fn test_call_result_propagation_suffixes_and_grouped_arguments() {
    let output = compile_and_run(
        r#"
get: I64 -> Result I64, I64
get = value -> Result::Ok value
pair: I64 -> Result (Vec I64), I64
pair = value ->
    mut values = Vec::new!
    values.push value
    values.push value + 1
    Result::Ok values
nested: I64 -> Result (Result I64, I64), I64
nested = value -> Result::Ok (Result::Ok value)
adder: I64 -> Result (I64 -> I64), I64
adder = ignored -> Result::Ok (extra -> extra + 1)

compute: () -> Result I64, I64
compute = ->
    get 42?.println!
    pair 41?[1].println!
    nested 42??.println!
    applied = adder 0? 41
    applied.println!
    explicit = get (get 42?)?
    explicit.println!
    borrowed = [get 42?]
    borrowed[0].println!
    Result::Ok 0

main = !-> compute!.println!
"#,
    );
    assert_eq!(output, "42\n42\n42\n42\n42\n42\nOk(0)\n");
}

#[test]
fn test_nested_bind_transitive_capture() {
    let output = compile_and_run(
        r#"
compute: () -> Result I64, I64
compute = ->
    match true
        _ =>
            bind (Result::Ok 16), (count ->
                bind (Result::Ok 0), (_ ->
                    bind (Result::Ok 6), (appended ->
                        total = count + appended
                        pure total)))

main = !-> compute! .unwrap_or -1 .println!
"#,
    );
    assert_eq!(output, "22\n");
}

#[test]
fn test_match_local_shadow_restores_outer_binding() {
    let output = compile_and_run(
        r#"
main = !->
    outside = 5
    inner = match true
        _ =>
            outside: I64 = 9
            outside
    inner.println!
    outside.println!
"#,
    );
    assert_eq!(output, "9\n5\n");
}

#[test]
fn test_do_notation_result_option_and_scope() {
    let output = compile_and_run(
        r#"
success: I64 -> Result I64, I64
success = value -> Result::Ok value

compute: () -> Result I64, I64
compute = -> do
    count <- success 16
    appended <- success 6
    total = count + appended
    pure total

main = !->
    compute! .unwrap_or -1 .println!
    outside = 5
    nested = do
        outside: I64 = 9
        x <- Option::Some outside
        y <- do
            z <- Option::Some 2
            pure (z + 1)
        Option::Some 0
        _ <- Option::Some 1
        pure (x + y)
    nested.unwrap_or -1 .println!
    outside.println!
    singleton: Option I64 = do
        local = 42
        pure local
    singleton.unwrap_or -1 .println!
"#,
    );
    assert_eq!(output, "22\n12\n5\n42\n");
}

#[test]
fn test_do_notation_short_circuits_and_preserves_order() {
    let output = compile_and_run(
        r#"
step: I64 -> Result I64, I64
step = value ->
    value.println!
    if value == 2 then Result::Err 99 else Result::Ok value

run: () -> Result I64, I64
run = -> do
    first <- step 1
    step 2
    later <- step 3
    pure (first + later)

main = !->
    run! .unwrap_or -1 .println!
    absent: Option I64 = do
        x <- Option::None
        printed = 100.println!
        pure x
    absent.unwrap_or -2 .println!
"#,
    );
    assert_eq!(output, "1\n2\n-1\n-2\n");
}

#[test]
fn test_do_notation_rebinds_bind_and_supports_generic_helpers() {
    let output = compile_and_run(
        r#"
twice: F I64 -> F I64 where F _: Monad
twice = action -> do
    value <- action
    pure (value * 2)

main = !->
    twice (Option::Some 21) .unwrap_or 0 .println!
    bind: I64 -> (I64 -> I64) -> I64 = value, callback -> callback (value + 10)
    result = do
        x <- 2
        y <- 3
        x + y
    result.println!
"#,
    );
    assert_eq!(output, "42\n25\n");
}

#[test]
fn test_do_notation_without_std() {
    let status = compile_and_run_without_std(
        r#"
bind: I64 -> (I64 -> I64) -> I64
bind = value, callback -> callback value

main = -> do
    value <- 7
    value
"#,
    );
    assert_eq!(status, 7);
}

#[test]
fn test_do_notation_repeated_continuations() {
    let output = compile_and_run(
        r#"
repeat: I64 -> M -> I64 where M: FnMut I64, I64
repeat = value, mut callback ->
    first = callback value
    second = callback (value + 1)
    first + second

main = !->
    bind: I64 -> (I64 -> I64) -> I64 = repeat
    result = do
        x <- 1
        printed = x.println!
        x
    result.println!
"#,
    );
    assert_eq!(output, "1\n2\n3\n");
}

#[test]
fn test_do_notation_owned_payload_drops_on_success_and_failure() {
    let output = compile_and_run(
        r#"
struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = !-> self.id.println!

run: I64 -> Option I64 -> Option I64
run = id, next -> do
    owned = Tracked
        id: id
    count <- next
    pure (owned.id + count)

main = !->
    run 10, (Option::Some 1) .unwrap_or -1 .println!
    run 20, Option::None .unwrap_or -1 .println!
    length = do
        text <- Option::Some (String::from "hello")
        pure text.len!
    length.unwrap_or 0 .println!
"#,
    );
    assert_eq!(output, "10\n11\n20\n-1\n5\n");
}

#[test]
fn test_do_notation_preserves_move_errors() {
    compile_should_fail(
        r#"
main = !->
    text = Option::Some (String::from "hello")
    length = do
        value <- text
        pure value.len!
    text.println!
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_do_notation_exported_generic_artifact() {
    let dir = test_temp_dir(TEST_COUNTER.fetch_add(1, Ordering::SeqCst));
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let library = dir.join("helpers.rk");
    fs::write(
        &library,
        r#"
increment: F I64 -> F I64 where F _: Monad
< increment = action -> do
    value <- action
    pure (value + 1)
"#,
    )
    .unwrap();
    let object = dir.join("helpers.o");
    let artifact = dir.join("helpers.rkca");
    let mut config = test_config(library, dir.clone());
    config.current_crate_name = Some("helpers".into());
    config.no_link = true;
    config.emit_object = Some(object);
    let mut products = rock_lib::compile_with_products(&config)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some(PathBuf::from("helpers.o"));
    products.write_artifact_to_path(&artifact).unwrap();

    let entry = dir.join("main.rk");
    fs::write(
        &entry,
        "> helpers::increment\nmain = -> increment (Option::Some 41) .unwrap_or 0\n",
    )
    .unwrap();
    let mut config = test_config(entry, dir.clone());
    config.extern_artifacts.push(("helpers".into(), artifact));
    rock_lib::compile(&config).unwrap();
    let output = run_test_command(&mut Command::new(dir.join("main")));
    assert_eq!(output.status.code(), Some(42));
}

#[test]
fn test_do_notation_does_not_reserve_bind_operator_globally() {
    let status = compile_and_run_without_std(
        r#"
infix 8 <-
<-: I64 -> I64 -> I64
<- = left, right -> ~I64Add left, right
main = -> 2 <- 3
"#,
    );
    assert_eq!(status, 5);
}

#[test]
fn test_do_notation_rejects_mixed_carriers() {
    compile_should_fail(
        r#"
main = !->
    result = do
        x <- Option::Some 1
        Result::Ok x
    result.println!
"#,
        "ambiguous constraint at quiescence: call to generic function 'bind'",
    );
}

#[test]
fn test_do_notation_macro_expansion() {
    let output = compile_and_run(
        r#"
macro make_main
    =>
        main = !->
            result = do
                x <- Option::Some 40
                pure (x + 2)
            result.unwrap_or 0 .println!

%make_main
"#,
    );
    assert_eq!(output, "42\n");
}

const STD_CACHE_LOCK_STALE_AFTER: Duration = Duration::from_secs(10 * 60);
const TEST_PROCESS_TIMEOUT: Duration = Duration::from_secs(15);

fn test_temp_dir(id: u64) -> PathBuf {
    let tid = std::thread::current().id();
    std::env::temp_dir().join(format!("rock_test_{}_{:?}_{}", std::process::id(), tid, id))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn std_path() -> PathBuf {
    workspace_root().join("std")
}

struct CacheLock {
    path: PathBuf,
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn acquire_cache_lock(path: &Path) -> Option<CacheLock> {
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                if let Err(err) = file.write_all(cache_lock_metadata().as_bytes()) {
                    let _ = std::fs::remove_file(path);
                    panic!(
                        "failed to write std cache lock {}: {}",
                        path.display(),
                        err
                    );
                }
                return Some(CacheLock {
                    path: path.to_path_buf(),
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if std_cache_entry_is_ready_for_lock(path) {
                    return None;
                }
                if recover_stale_cache_lock(path, STD_CACHE_LOCK_STALE_AFTER) {
                    continue;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(err) => panic!(
                "failed to acquire std cache lock {}: {}",
                path.display(),
                err
            ),
        }
    }
}

fn cache_lock_metadata() -> String {
    format!(
        "pid={}\ncreated_unix_nanos={}\n",
        std::process::id(),
        current_unix_nanos()
    )
}

fn current_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn std_cache_entry_is_ready_for_lock(lock_path: &Path) -> bool {
    let artifact_dir = lock_path.with_extension("");
    std_cache_entry_is_ready(
        &artifact_dir.join("std.rkca"),
        &artifact_dir.join("std.o"),
        &artifact_dir.join("ready"),
    )
}

fn std_cache_lock_is_stale(path: &Path, stale_after: Duration) -> bool {
    let age = std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| lock_age_from_metadata(&contents))
        .or_else(|| lock_age_from_file_metadata(path));

    age.is_some_and(|age| age >= stale_after)
}

fn recover_stale_cache_lock(path: &Path, stale_after: Duration) -> bool {
    if std_cache_entry_is_ready_for_lock(path) || !std_cache_lock_is_stale(path, stale_after)
    {
        return false;
    }

    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(err) => panic!(
            "failed to remove stale std cache lock {}: {}",
            path.display(),
            err
        ),
    }
}

fn lock_age_from_metadata(contents: &str) -> Option<Duration> {
    let created = contents
        .lines()
        .find_map(|line| line.strip_prefix("created_unix_nanos="))?
        .parse::<u128>()
        .ok()?;
    let now = current_unix_nanos();
    if created > now {
        return None;
    }

    Some(Duration::from_nanos(
        (now - created).min(u128::from(u64::MAX)) as u64,
    ))
}

fn lock_age_from_file_metadata(path: &Path) -> Option<Duration> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .elapsed()
        .ok()
}

fn std_cache_entry_is_ready(
    artifact_path: &Path,
    object_path: &Path,
    ready_path: &Path,
) -> bool {
    ready_path.is_file()
        && object_path.is_file()
        && artifact_path.is_file()
        && rock_lib::products::CompilerProducts::read_artifact_from_path(artifact_path).is_ok()
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), dst_path)?;
        }
    }

    Ok(())
}

fn publish_std_cache_dir(
    build_dir: &Path,
    artifact_dir: &Path,
    ready_path: &Path,
) -> Result<(), String> {
    remove_file_if_exists(ready_path).map_err(|err| {
        format!(
            "failed to remove stale std cache ready marker {}: {}",
            ready_path.display(),
            err
        )
    })?;
    remove_dir_if_exists(artifact_dir).map_err(|err| {
        format!(
            "failed to remove stale std cache directory {}: {}",
            artifact_dir.display(),
            err
        )
    })?;
    if artifact_dir.exists() {
        return Err(format!(
            "std cache destination still exists before publish: {}",
            artifact_dir.display()
        ));
    }

    match std::fs::rename(build_dir, artifact_dir) {
        Ok(()) => {}
        Err(rename_err) => {
            let publish_dir = artifact_dir.with_extension(format!(
                "publishing-{}-{}",
                std::process::id(),
                current_unix_nanos()
            ));
            remove_dir_if_exists(&publish_dir).map_err(|cleanup_err| {
                format!(
                    "failed to clean std cache publish directory {} after rename failed: {}; cleanup failed: {}",
                    publish_dir.display(),
                    rename_err,
                    cleanup_err
                )
            })?;
            copy_dir_all(build_dir, &publish_dir).map_err(|copy_err| {
                let cleanup = cleanup_error_suffix(remove_dir_if_exists(&publish_dir));
                format!(
                    "failed to copy std cache from {} to {} after rename failed: {}; copy failed: {}{}",
                    build_dir.display(),
                    publish_dir.display(),
                    rename_err,
                    copy_err,
                    cleanup
                )
            })?;
            validate_std_cache_payload(&publish_dir).map_err(|validate_err| {
                let cleanup = cleanup_error_suffix(remove_dir_if_exists(&publish_dir));
                format!(
                    "failed to validate std cache publish directory {}: {}{}",
                    publish_dir.display(),
                    validate_err,
                    cleanup
                )
            })?;
            std::fs::rename(&publish_dir, artifact_dir).map_err(|publish_err| {
                let cleanup = cleanup_error_suffix(remove_dir_if_exists(&publish_dir));
                format!(
                    "failed to publish copied std cache from {} to {} after rename failed: {}; publish failed: {}{}",
                    publish_dir.display(),
                    artifact_dir.display(),
                    rename_err,
                    publish_err,
                    cleanup
                )
            })?;
            remove_dir_if_exists(build_dir).map_err(|cleanup_err| {
                format!(
                    "failed to remove copied std cache build directory {}: {}",
                    build_dir.display(),
                    cleanup_err
                )
            })?;
        }
    }

    validate_std_cache_payload(artifact_dir)?;
    std::fs::write(ready_path, b"ready").map_err(|err| {
        format!(
            "failed to mark std cache directory ready {}: {}",
            ready_path.display(),
            err
        )
    })
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn remove_dir_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn cleanup_error_suffix(result: std::io::Result<()>) -> String {
    match result {
        Ok(()) => String::new(),
        Err(err) => format!("; cleanup failed: {err}"),
    }
}

fn validate_std_cache_payload(dir: &Path) -> Result<(), String> {
    let artifact_path = dir.join("std.rkca");
    let object_path = dir.join("std.o");
    if !object_path.is_file() {
        return Err(format!(
            "std cache object file is missing: {}",
            object_path.display()
        ));
    }
    rock_lib::products::CompilerProducts::read_artifact_from_path(&artifact_path).map(|_| ())
}

fn write_minimal_std_artifact_for_cache_test(path: &Path) {
    rock_lib::products::CompilerProducts {
        crate_identity: rock_lib::products::ProductCrateIdentity::local("std".to_string()),
        identity_table: rock_lib::products::ProductIdentityTable::default(),
        interface: rock_lib::products::ProductInterface::default(),
        bodies: rock_lib::products::ProductBodies::default(),
        link: rock_lib::products::ProductLinkData {
            object_path: Some(PathBuf::from("std.o")),
            ..rock_lib::products::ProductLinkData::default()
        },
        dependencies: Vec::new(),
        source_fingerprint: rock_lib::products::ProductSourceFingerprint::default(),
        infix_precedence: std::collections::BTreeMap::new(),
        proc_macros: Vec::new(),
    }
    .write_artifact_to_path(path)
    .unwrap();
}

#[test]
fn std_cache_key_changes_when_std_source_changes() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let std_dir = dir.join("std");
    std::fs::create_dir_all(std_dir.join("src/values")).unwrap();
    std::fs::write(std_dir.join("src/lib.rk"), "mod values\n").unwrap();
    let values = std_dir.join("src/values/mod.rk");
    std::fs::write(&values, "answer = -> 0\n").unwrap();

    let first = std_cache_key(&std_dir, "compiler-stamp");
    let second = std_cache_key(&std_dir, "compiler-stamp");
    assert_eq!(first, second);

    std::fs::write(&values, "answer = -> 1\n").unwrap();
    let changed = std_cache_key(&std_dir, "compiler-stamp");

    assert_ne!(first, changed);
    assert_ne!(changed, std_cache_key(&std_dir, "rebuilt-compiler"));

    std::fs::write(std_dir.join("rock.toml"), "[crate]\nname = \"std\"\n").unwrap();
    let with_manifest = std_cache_key(&std_dir, "compiler-stamp");
    assert_ne!(changed, with_manifest);
    std::fs::write(
        std_dir.join("rock.toml"),
        "[crate]\nname = \"renamed\"\n",
    )
    .unwrap();
    assert_ne!(
        with_manifest,
        std_cache_key(&std_dir, "compiler-stamp")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn std_cache_rejects_missing_ready_marker() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    write_minimal_std_artifact_for_cache_test(&dir.join("std.rkca"));
    std::fs::write(dir.join("std.o"), b"object").unwrap();

    assert!(!std_cache_entry_is_ready(
        &dir.join("std.rkca"),
        &dir.join("std.o"),
        &dir.join("ready")
    ));
}

#[test]
fn std_cache_stale_lock_recovery_does_not_spin_forever() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let artifact_dir = dir.join("cache");
    let lock_path = artifact_dir.with_extension("lock");
    std::fs::write(&lock_path, "pid=0\ncreated_unix_nanos=0\n").unwrap();

    assert!(recover_stale_cache_lock(&lock_path, Duration::from_secs(1)));
    assert!(!lock_path.exists());
}

#[test]
fn std_cache_publish_replaces_stale_ready_entry_atomically() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let artifact_dir = dir.join("cache");
    let build_dir = dir.join("build");
    let ready_path = artifact_dir.join("ready");
    std::fs::create_dir_all(&artifact_dir).unwrap();
    std::fs::create_dir_all(&build_dir).unwrap();
    std::fs::write(artifact_dir.join("ready"), b"ready").unwrap();
    std::fs::write(artifact_dir.join("std.rkca"), b"invalid stale artifact").unwrap();
    std::fs::write(artifact_dir.join("std.o"), b"stale object").unwrap();
    write_minimal_std_artifact_for_cache_test(&build_dir.join("std.rkca"));
    std::fs::write(build_dir.join("std.o"), b"fresh object").unwrap();

    publish_std_cache_dir(&build_dir, &artifact_dir, &ready_path)
        .expect("cache publish should replace stale ready entry");

    assert!(std_cache_entry_is_ready(
        &artifact_dir.join("std.rkca"),
        &artifact_dir.join("std.o"),
        &ready_path,
    ));
    assert_eq!(
        std::fs::read(artifact_dir.join("std.o")).unwrap(),
        b"fresh object"
    );
}

fn std_artifact_path() -> PathBuf {
    static STD_ARTIFACT: OnceLock<PathBuf> = OnceLock::new();

    STD_ARTIFACT
        .get_or_init(|| {
            let std_dir = std_path();
            let cache_key = std_cache_key(&std_dir, &std_cache_compiler_stamp());
            let artifact_dir = std::env::temp_dir().join(format!(
                "rock_integration_std_product_artifact_{}",
                cache_key
            ));
            let artifact_path = artifact_dir.join("std.rkca");
            let object_path = artifact_dir.join("std.o");
            let ready_path = artifact_dir.join("ready");
            let lock_path = artifact_dir.with_extension("lock");
            if std_cache_entry_is_ready(&artifact_path, &object_path, &ready_path) {
                return artifact_path;
            }

            let Some(_cache_lock) = acquire_cache_lock(&lock_path) else {
                return artifact_path;
            };
            if std_cache_entry_is_ready(&artifact_path, &object_path, &ready_path) {
                return artifact_path;
            }

            let build_dir = artifact_dir.with_extension(format!("building-{}", std::process::id()));
            remove_dir_if_exists(&build_dir)
                .expect("Failed to clean std product artifact build directory");
            std::fs::create_dir_all(&build_dir).unwrap();
            let build_artifact_path = build_dir.join("std.rkca");
            let build_object_path = build_dir.join("std.o");

            let output = rock_lib::compile_with_products(&rock_lib::Config {
                entry_file: std_dir.join("src/lib.rk"),
                output_dir: build_dir.clone(),
                debug_print: vec![],
                meta_files: vec![],
                extern_artifacts: vec![],
                source_providers: Vec::new(),
                current_crate_name: Some("std".to_string()),
                opt_level: 0,
                overflow_checks: None,
                emit_llvm: false,
                no_link: true,
                emit_object: Some(build_object_path),
                no_prelude: false,
                no_std: true,
                sysroot: None,
            })
            .unwrap_or_else(|diagnostics| {
                diagnostics.report();
                panic!("Failed to compile std product artifact for integration tests");
            });
            let mut products = output
                .products
                .expect("std compile should produce product data");
            products.link.object_path = Some(PathBuf::from("std.o"));
            products
                .write_artifact_to_path(&build_artifact_path)
                .expect("Failed to write std product artifact for integration tests");

            rock_lib::products::CompilerProducts::read_artifact_from_path(&build_artifact_path)
                .expect("Failed to validate std product artifact for integration tests");

            publish_std_cache_dir(&build_dir, &artifact_dir, &ready_path)
                .expect("Failed to publish std product artifact cache");
            artifact_path
        })
        .clone()
}

fn test_config(entry_file: PathBuf, output_dir: PathBuf) -> rock_lib::Config {
    rock_lib::Config {
        entry_file,
        output_dir,
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![("std".to_string(), std_artifact_path())],
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: false,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    }
}

struct TestDirCleanup(PathBuf);

impl Drop for TestDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn child_exited_without_reaping(child: &Child) -> Result<bool, std::io::Error> {
    let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { info.assume_init().si_pid() } != 0)
}

fn terminate_test_command_group(child: &Child) -> Result<(), std::io::Error> {
    // A negative PID targets the child's process group; ESRCH only means it already exited.
    let result = unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    Ok(())
}

fn terminate_and_collect_test_command(
    child: &mut Child,
    stdout_reader: std::thread::JoinHandle<Vec<u8>>,
    stderr_reader: std::thread::JoinHandle<Vec<u8>>,
) -> Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>), (std::io::Error, std::process::ExitStatus)>
{
    if let Err(error) = terminate_test_command_group(child) {
        let _ = child.kill();
        let status = child.wait().expect("failed child must be reaped");
        drop(stdout_reader);
        drop(stderr_reader);
        return Err((error, status));
    }
    let status = child.wait().expect("failed child must be reaped");
    let stdout = stdout_reader.join().expect("stdout reader must finish");
    let stderr = stderr_reader.join().expect("stderr reader must finish");
    Ok((status, stdout, stderr))
}

fn run_test_command(command: &mut Command) -> Output {
    let executable = command.get_program().to_string_lossy().into_owned();
    command.process_group(0);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to spawn {executable}: {error}"));
    let stdout = child.stdout.take().expect("child stdout must be piped");
    let stderr = child.stderr.take().expect("child stderr must be piped");
    let stdout_reader = std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    loop {
        match child_exited_without_reaping(&child) {
            Ok(true) => break,
            Ok(false) if started.elapsed() < TEST_PROCESS_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(false) => {
                let (status, stdout, stderr) = terminate_and_collect_test_command(
                    &mut child,
                    stdout_reader,
                    stderr_reader,
                )
                .unwrap_or_else(|(error, status)| {
                    panic!(
                        "failed to terminate process group {} for {executable}: {error} (direct child status {status})",
                        child.id()
                    )
                });
                panic!(
                    "{executable} exceeded {:?} (status {status}); stdout:\n{}\nstderr:\n{}",
                    TEST_PROCESS_TIMEOUT,
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr),
                );
            }
            Err(error) => {
                let (status, stdout, stderr) = terminate_and_collect_test_command(
                    &mut child,
                    stdout_reader,
                    stderr_reader,
                )
                .unwrap_or_else(|(cleanup_error, status)| {
                    panic!(
                        "failed while waiting for {executable}: {error}; failed to terminate process group {}: {cleanup_error} (direct child status {status})",
                        child.id()
                    )
                });
                panic!(
                    "failed while waiting for {executable}: {error} (status {status}); stdout:\n{}\nstderr:\n{}",
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr),
                );
            }
        }
    }
    if let Err(error) = terminate_test_command_group(&child) {
        let status = child.wait().expect("exited child must be reaped");
        drop(stdout_reader);
        drop(stderr_reader);
        panic!(
            "failed to terminate process group {} for {executable}: {error} (direct child status {status})",
            child.id(),
        );
    }
    let status = child.wait().expect("exited child must be reaped");
    let stdout = stdout_reader.join().expect("stdout reader must finish");
    let stderr = stderr_reader.join().expect("stderr reader must finish");

    Output {
        status,
        stdout,
        stderr,
    }
}

fn compile_and_run_without_std(source: &str) -> i32 {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let source_path = dir.join(format!("no_std_{id}.rk"));
    fs::write(&source_path, source).unwrap();

    let mut config = test_config(source_path, dir.clone());
    config.extern_artifacts.clear();
    config.no_std = true;
    config.no_prelude = true;
    rock_lib::compile(&config).expect("no-std compilation failed");

    let mut command = Command::new(dir.join(format!("no_std_{id}")));
    run_test_command(&mut command).status.code().unwrap_or(1)
}

fn compile_and_run(source: &str) -> String {
    compile_and_run_at_opt(source, 0)
}

fn compile_and_run_at_opt(source: &str, opt_level: u8) -> String {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    let source_path = dir.join(format!("test_{}.rk", id));
    std::fs::write(&source_path, source).unwrap();

    let mut config = test_config(source_path.clone(), dir.clone());
    config.opt_level = opt_level;

    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join(format!("test_{}", id));
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    assert!(
        output.status.success(),
        "compiled program failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    // Cleanup
    let _ = std::fs::remove_dir_all(&dir);

    stdout
}

fn compile_and_run_with_args(source: &str, args: &[&str]) -> String {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    let source_path = dir.join(format!("test_{}.rk", id));
    std::fs::write(&source_path, source).unwrap();

    let config = test_config(source_path, dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join(format!("test_{}", id));
    let output = Command::new(&binary)
        .args(args)
        .output()
        .expect("Failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    let _ = std::fs::remove_dir_all(&dir);
    stdout
}

fn compile_and_run_with_status(source: &str) -> (String, bool) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    let source_path = dir.join(format!("test_{}.rk", id));
    std::fs::write(&source_path, source).unwrap();

    let config = test_config(source_path.clone(), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join(format!("test_{}", id));
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let success = output.status.success();

    let _ = std::fs::remove_dir_all(&dir);

    (stdout, success)
}

fn compile_and_run_in_working_dir(source: &str, dir: &Path) -> Output {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    fs::create_dir_all(dir).unwrap();

    let source_path = dir.join(format!("test_{id}.rk"));
    fs::write(&source_path, source).unwrap();

    let config = test_config(source_path, dir.to_path_buf());
    rock_lib::compile(&config).expect("Compilation failed");

    let mut command = Command::new(dir.join(format!("test_{id}")));
    command.current_dir(dir);
    run_test_command(&mut command)
}

fn compile_example(name: &str) -> String {
    let examples_dir = workspace_root().join("examples");
    let source_path = examples_dir.join(format!("{}.rk", name));
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|_| panic!("Failed to read example: {}", name));
    compile_and_run(&source)
}

#[test]
fn test_hello_world() {
    let output = compile_example("hello");
    assert_eq!(output.trim(), "Hello, World!");
}

#[test]
fn test_std_program_args_are_owned_and_available_outside_main() {
    let output = compile_and_run_with_args(
        r#"
> std::env::args

read_args = -> args!

main = ->
    first = read_args!
    second = read_args!
    first.len!.println!
    first[1].println!
    second[2].println!
    0
"#,
        &["alpha", "two words"],
    );

    assert_eq!(output, "3\nalpha\ntwo words\n");
}

#[test]
fn test_declarative_macro_expands_user_program() {
    let (_stdout, success) = compile_and_run_with_status(
        r#"macro make_main
    =>
        main = -> 0
%make_main"#,
    );

    assert!(success);
}

#[test]
fn macro_a_06_grouped_expression_forwarding_runs_normally() {
    let output = compile_and_run("macro id\n    $value:expr =>\n        $value\nmacro times\n    $value:expr =>\n        %id ($value * 3)\nmain = !->\n    result = %times 1 + 2\n    result.println!\n");
    assert_eq!(output, "9\n");
}

#[test]
fn macro_a_09_generated_inline_import_and_expression_calls_run_after_item_discovery() {
    let output = compile_and_run("> tools::make\n%make\nmacro setup\n    =>\n        mod tools\n            < make\n            < number\n            macro make\n                =>\n                    main = !->\n                        (%tools::number!).println!\n            macro number\n                =>\n                    42\n%setup\n");
    assert_eq!(output, "42\n");
}

#[test]
fn macro_a_09_generated_file_modules_and_transitive_generated_children_run() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let mut config = test_config(dir.join("main.rk"), dir.clone());
    config.current_crate_name = Some("app".into());
    config.extern_artifacts.clear();
    config.no_prelude = true;
    config.no_std = true;
    config.source_providers = vec![
        rock_lib::SourceProvider::Virtual { path: dir.join("main.rk"), text: "> tools::make\n%make\nmacro setup\n    =>\n        mod tools\n%setup\n".into() },
        rock_lib::SourceProvider::Virtual { path: dir.join("tools.rk"), text: "< make\nmacro make\n    =>\n        main = -> tools::work!\nmacro setup\n    =>\n        mod inner\n%setup\n< work = -> inner::answer!\n".into() },
        rock_lib::SourceProvider::Virtual { path: dir.join("inner.rk"), text: "< answer = -> 42\n".into() },
    ];
    rock_lib::compile(&config).unwrap();
    let output = run_test_command(&mut Command::new(dir.join("main")));
    assert_eq!(output.status.code(), Some(42));
}

#[test]
fn macro_a_06_generated_block_keeps_return_in_the_callers_function() {
    let output = compile_and_run("macro early\n    =>\n        if true\n            return 7\n        else\n            0\nwork = ->\n    %early!\n    99\nmain = !->\n    work! .println!\n");
    assert_eq!(output, "7\n");
}

#[test]
fn macro_a_06_generated_block_keeps_break_in_the_callers_loop() {
    let output = compile_and_run("macro stop\n    =>\n        if true\n            break\n        else\n            ()\nmain = !->\n    loop\n        %stop!\n        99.println!\n    7.println!\n");
    assert_eq!(output, "7\n");
}

#[test]
fn test_declarative_macro_can_produce_call_argument_holes() {
    let output = compile_and_run(
        r#"macro make_main
    =>
        combine = a, b, c -> a * 100 + b * 10 + c
        main = ->
            section = combine 1, _, 3
            section 2 .println!
            0
%make_main"#,
    );

    assert_eq!(output.trim(), "123");
}

#[test]
fn test_arithmetic() {
    let output = compile_example("arithmetic");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "30");
    assert_eq!(lines[1], "205");
}

#[test]
fn test_operator_resolution_does_not_depend_on_std_trait_names() {
    let output = compile_and_run(
        r#"< struct Boxed
    < value: I64

< trait Combine Rhs
    @+: Rhs -> I64

impl Combine Boxed for Boxed
    @+ = other -> self.value + other.value

main = ->
    lhs = Boxed
        value: 40
    rhs = Boxed
        value: 2
    result = lhs + rhs
    result.println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_primitive_operator_semantics_come_from_user_defined_operator() {
    let output = compile_and_run(
        r#"+ = left, right -> ~I64Sub left, right

main = ->
    40 + 2 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "38");
}

#[test]
fn test_unsafe_operator_function_requires_unsafe() {
    compile_should_fail(
        r#"unsafe + = left, right -> ~I64Add left, right

main = ->
    value = 40 + 2
    value.println!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_ampersand_function_reference_call_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_ref: &I64 -> I64
read_ref = x -> *x

main = ->
    value = 7
    result = read_ref & value
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_operator_method_requires_unsafe() {
    compile_should_fail(
        r#"< struct UnsafeAdder
    < value: I64

< trait Combine Rhs
    type Output
    @+: Rhs -> Self::Output

impl Combine I64 for UnsafeAdder
    type Output = I64

    unsafe @+ = other -> self.value + other

main = ->
    lhs = UnsafeAdder
        value: 40
    value = lhs + 2
    value.println!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_method_value_requires_unsafe() {
    compile_should_fail(
        r#"struct Secret
    < value: I64

impl Secret
    unsafe @read = -> @value

main = ->
    secret = Secret
        value: 7
    read = secret.read
    value = read!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_method_value_created_in_unsafe_still_requires_unsafe_call() {
    compile_should_fail(
        r#"struct Secret
    < value: I64

impl Secret
    unsafe @read = -> @value

main = ->
    secret = Secret
        value: 7
    read = unsafe secret.read
    value = read!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_function_alias_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64 -> I64
read_secret = value -> value

main = ->
    alias = read_secret
    value = alias 7
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_function_alias_ampersand_call_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_ref: &I64 -> I64
read_ref = value -> *value

main = ->
    value = 7
    alias = read_ref
    result = alias & value
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_callable_cannot_flow_to_safe_callable_slot() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

apply: (() -> I64) -> I64
apply = reader -> reader!

main = ->
    value = apply read_secret
    0
"#,
        "unsafe",
    );
}

#[test]
fn test_unsafe_function_returned_callable_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64 -> I64
read_secret = value -> value

make_reader = -> unsafe read_secret

main = ->
    reader = make_reader!
    value = reader 7
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_method_returned_callable_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

struct Factory

impl Factory
    @make_reader = -> unsafe read_secret

main = ->
    factory = Factory
    reader = factory.make_reader!
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_if_returned_callable_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_reader = flag ->
    if flag
        unsafe read_secret
    else
        safe_reader

main = ->
    reader = make_reader true
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_if_returned_callable_safe_then_unsafe_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_reader = flag ->
    if flag
        safe_reader
    else
        unsafe read_secret

main = ->
    reader = make_reader true
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_match_returned_callable_safe_then_unsafe_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_reader = flag ->
    match flag
        true => safe_reader
        false => unsafe read_secret

main = ->
    reader = make_reader true
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_if_tuple_callable_safe_then_unsafe_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_pair = flag ->
    if flag
        (safe_reader, 0)
    else
        (unsafe read_secret, 1)

main = ->
    pair = make_pair true
    reader = pair.0
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_match_tuple_callable_safe_then_unsafe_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_pair = flag ->
    match flag
        true => (safe_reader, 0)
        false => (unsafe read_secret, 1)

main = ->
    pair = make_pair true
    reader = pair.0
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_callable_tuple_projection_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_reader = -> unsafe read_secret

main = ->
    pair = (unsafe make_reader!, safe_reader)
    reader = pair.0
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_callable_struct_field_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

struct Holder
    < reader: () -> I64

make_reader = -> unsafe read_secret

main = ->
    holder = Holder
        reader: unsafe make_reader!
    value = holder.reader!
    0
"#,
        "unsafe",
    );
}

#[test]
fn test_unsafe_callable_reassignment_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_reader = -> unsafe read_secret

main = ->
    mut reader = safe_reader
    reader = unsafe make_reader!
    value = reader!
    0
"#,
        "unsafe",
    );
}

#[test]
fn test_unsafe_method_to_method_returned_callable_requires_unsafe() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("test.rk");
    std::fs::write(
        &source_path,
        r#"unsafe read_secret: I64
read_secret = -> 7

struct Factory

impl Factory
    @make_reader = -> unsafe read_secret
    @forward_reader = -> self.make_reader!

main = ->
    factory = Factory
    reader = factory.forward_reader!
    value = reader!
    0
"#,
    )
    .unwrap();

    let mut config = test_config(source_path, dir.clone());
    config.no_std = true;
    config.no_prelude = true;
    config.extern_artifacts.clear();
    let result = rock_lib::compile(&config);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.is_err(),
        "expected method-to-method unsafe callable laundering to be rejected"
    );
}

#[test]
fn test_unsafe_nested_method_laundering_requires_unsafe() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("test.rk");
    std::fs::write(
        &source_path,
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

struct Factory

impl Factory
    @use_reader = ->
        reader = if true
            self.forward_reader!
        else
            safe_reader
        if true
            reader!
        else
            0

    @forward_reader = -> self.make_reader!
    @make_reader = -> unsafe read_secret

main = ->
    factory = Factory
    value = factory.use_reader!
    0
"#,
    )
    .unwrap();

    let mut config = test_config(source_path, dir.clone());
    config.no_std = true;
    config.no_prelude = true;
    config.extern_artifacts.clear();
    let result = rock_lib::compile(&config);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.is_err(),
        "expected nested unsafe callable laundering to be rejected"
    );
}

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
fn test_primitive_operator_accepts_inferred_rhs_type() {
    let output = compile_and_run(
        r#"main = ->
    values = [2]
    result = 40 + values[0]
    result.println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_functions() {
    let output = compile_example("functions");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "25");
    assert_eq!(lines[2], "25");
}

#[test]
fn test_if_else() {
    let output = compile_example("if_else");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "10");
    assert_eq!(lines[2], "42");
}

#[test]
fn test_structs() {
    let output = compile_example("structs");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "20");
    assert_eq!(lines[2], "30");
}

#[test]
fn test_while_loop() {
    let output = compile_example("while_loop");
    let lines: Vec<&str> = output.trim().lines().collect();
    for i in 0..10 {
        assert_eq!(lines[i], i.to_string());
    }
    assert_eq!(lines[10], "done");
}

#[test]
fn test_for_loop() {
    let output = compile_example("for_loop");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "20");
    assert_eq!(lines[2], "30");
    assert_eq!(lines[3], "40");
    assert_eq!(lines[4], "50");
    assert_eq!(lines[5], "done");
}

#[test]
fn test_recursion() {
    let output = compile_example("recursion");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "3628800"); // 10!
    assert_eq!(lines[1], "55"); // fib(10)
}

#[test]
fn test_match_expr() {
    let output = compile_example("match_expr");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "0");
    assert_eq!(lines[1], "10");
    assert_eq!(lines[2], "20");
    assert_eq!(lines[3], "99");
}

#[test]
fn test_enums() {
    let output = compile_example("enums");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "enum test");
    assert_eq!(lines[1], "42");
}

#[test]
fn test_enum_match() {
    let output = compile_example("enum_match");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "3");
}

#[test]
fn test_enum_string_payload_pattern_reports_diagnostic() {
    compile_should_fail(
        r#"
enum Token
    Text &Str
    Other

main = ->
    token = Token::Text "x"
    match token
        Token::Text "x" => 1
        _ => 0
"#,
        "unsupported enum payload literal pattern",
    );
}

#[test]
fn test_impl_method_enum_payload_pattern_reports_diagnostic() {
    compile_should_fail(
        r#"
enum Token
    Text &Str
    Other

struct Wrapper
    < token: Token

impl Wrapper
    @check = ->
        match @token
            Token::Text "x" => 1
            _ => 0

main = ->
    wrapper = Wrapper
        token: Token::Text "x"
    wrapper.check!
"#,
        "unsupported enum payload literal pattern",
    );
}

#[test]
fn test_trait_default_enum_payload_pattern_reports_diagnostic() {
    compile_should_fail(
        r#"
enum Token
    Text &Str
    Other

struct Wrapper

trait ChecksToken
    @check: Token -> I64
    @check = token ->
        match token
            Token::Text "x" => 1
            _ => 0

impl ChecksToken for Wrapper

main = ->
    wrapper = Wrapper
    wrapper.check (Token::Text "x")
"#,
        "unsupported enum payload literal pattern",
    );
}

#[test]
fn test_guarded_noncopy_enum_payload_binding_reports_diagnostic() {
    compile_should_fail(
        r#"
struct Boxed
    < value: I64

enum MaybeBoxed
    Some Boxed
    None

main = ->
    item = MaybeBoxed::Some (Boxed
        value: 1)
    match item
        MaybeBoxed::Some value if 1 == 1 => value.value
        _ => 0
"#,
        "unsupported guarded non-copy enum payload binding",
    );
}

#[test]
fn test_nested_control_flow() {
    let output = compile_example("nested");
    let lines: Vec<&str> = output.trim().lines().collect();
    // fizzbuzz for 1-20
    let expected = [
        "3", "3", "1", "3", "2", "1", "3", "3", "1", "2", "3", "1", "3", "3", "0", "3", "3", "1",
        "3", "2",
    ];
    for (i, exp) in expected.iter().enumerate() {
        assert_eq!(lines[i], *exp, "Mismatch at position {}", i);
    }
}

#[test]
fn test_string_literal_indexing_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    s = "abc"
    s[1] .println!
    0
"#,
        "No implementation found for operator '[]'",
    );
}

#[test]
fn test_duplicate_generic_parameter_names_are_rejected() {
    compile_should_fail(
        r#"
struct Box T, T
    value: T

main = ->
    0
"#,
        "duplicate generic parameter 'T'",
    );
}

#[test]
fn test_borrowed_u8_slice_struct_field_indexes_as_byte() {
    let output = compile_and_run(
        r#"
struct Holder
    < data: &[U8]

second: &[U8] -> U8
second = s -> s[1]

main = ->
    bytes = [97 as U8, 98 as U8, 99 as U8]
    holder = Holder
        data: &bytes
    slice = holder.data
    (second slice) as I64 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "98");
}

#[test]
fn test_bare_slice_struct_field_is_rejected() {
    compile_should_fail(
        r#"
struct Holder
    < data: [U8]

main = -> 0
"#,
        "bare slice type [T] must be written behind a reference",
    );
}

#[test]
fn test_array_literal_keeps_fixed_array_type_until_coercion() {
    let output = compile_and_run(
        r#"
show_slice: &[I64] -> String
show_slice = s -> s.show!

main = ->
    arr = [1, 2, 3]
    show_slice &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_show_impl_for_slice_resolves_through_builtin_receiver_type() {
    let output = compile_and_run(
        r#"
main = ->
    s = "abc"
    s.println!
    0
"#,
    );

    assert_eq!(output.trim(), "abc");
}

#[test]
fn test_array_is_available_as_user_defined_type_name() {
    let output = compile_and_run(
        r#"
struct Array
    < value: I64

main = ->
    a = Array
        value: 41
    a.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "41");
}

#[test]
fn test_bare_slice_impl_target_is_rejected() {
    compile_should_fail(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for [T]
    @slice_len = -> ~ArrayLen self

main = -> 0
"#,
        "bare slice type [T] must be written behind a reference",
    );
}

#[test]
fn test_bare_str_parameter_is_rejected() {
    compile_should_fail(
        r#"
len: Str -> I64
len = s -> ~ArrayLen s

main = -> 0
"#,
        "bare string slice type Str must be written behind a reference",
    );
}

#[test]
fn test_borrowed_slice_impl_target_dispatches() {
    let output = compile_and_run(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for &[T]
    @slice_len = -> ~ArrayLen (*self)

main = ->
    arr = [1, 2, 3]
    (&arr).slice_len!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_str_trait_impl_uses_borrowed_str_not_u8_slice() {
    let output = compile_and_run(
        r#"
trait Kind
    @kind = -> 0

impl Kind for &Str
    @kind = -> 7

impl Kind for &[U8]
    @kind = -> 3

main = ->
    s = "abc"
    s.kind!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_u8_slice_trait_impl_does_not_apply_to_string_literal() {
    compile_should_fail(
        r#"
trait BytesOnly
    @bytes_only = -> 0

impl BytesOnly for &[U8]
    @bytes_only = -> 1

main = ->
    s = "abc"
    s.bytes_only!.println!
    0
"#,
        "bytes_only",
    );
}

#[test]
fn test_user_defined_array_does_not_receive_slice_methods() {
    compile_should_fail(
        r#"
struct Array
    < value: I64

main = ->
    a = Array
        value: 1
    a.println!
    0
"#,
        "Unknown field 'println' on struct 'Array'",
    );
}

#[test]
fn test_fixed_array_value_does_not_coerce_to_slice_parameter() {
    compile_should_fail(
        r#"
sum: &[I64] -> I64
sum = s ->
    (s[0] + s[1]) + s[2]

main = ->
    arr = [1, 2, 3]
    sum arr .println!
    0
"#,
        "Type mismatch",
    );
}

#[test]
fn test_fixed_array_value_does_not_coerce_to_slice_return() {
    compile_should_fail(
        r#"
make: () -> &[I64]
make = -> [1, 2, 3]

main = ->
    0
"#,
        "Type mismatch",
    );
}

#[test]
fn test_overapplied_non_unary_call_is_rejected() {
    compile_should_fail(
        r#"
add = x, y -> x + y

main = ->
    add 1, 2, 3 .println!
    0
"#,
        "Type mismatch",
    );
}

#[test]
fn test_fixed_array_borrow_coerces_to_slice_parameter() {
    let output = compile_and_run(
        r#"
sum: &[I64] -> I64
sum = s ->
    (s[0] + s[1]) + s[2]

main = ->
    arr = [1, 2, 3]
    sum &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "6");
}

#[test]
fn test_array_literal_cast_seed_infers_remaining_integer_elements() {
    let output = compile_and_run(
        r#"
first_byte: &[U8] -> I64
first_byte = s -> s[0] as I64

main = ->
    buf = [7 as U8, 8, 9]
    first_byte &buf .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_array_literal_binding_annotation_infers_integer_elements() {
    let output = compile_and_run(
        r#"
main = ->
    buf: [U8; 5] = [1, 2, 3, 4, 5]
    buf[4] as I64 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_repeat_array_initializer_is_evaluated_once() {
    let output = compile_and_run(
        r#"
next_byte: &mut I64 -> U8
next_byte = calls ->
    *calls = *calls + 1
    7 as U8

main = ->
    mut calls = 0
    buf: [U8; 256] = [next_byte (&mut calls); 256]
    buf[255] as I64 .println!
    calls.println!
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["7", "1"]);
}

#[test]
fn test_repeat_array_binding_annotation_infers_integer_element() {
    let output = compile_and_run(
        r#"
main = ->
    buf: [U8; 256] = [0; 256]
    buf[255] as I64 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "0");
}

#[test]
fn test_repeat_array_rejects_non_copy_initializer() {
    compile_should_fail(
        r#"
main = ->
    values = [String::from "owned"; 2]
    values
    0
"#,
        "array repeat initializer must be Copy",
    );
}

#[test]
fn test_array_literal_mut_slice_use_infers_integer_elements() {
    let output = compile_and_run(
        r#"
write_first: &mut [U8] -> I64
write_first = s ->
    ptr = (~ArrPtr (*s)) as *U8
    unsafe *ptr = 42 as U8
    unsafe *ptr as I64

main = ->
    mut buf = [0, 0, 0, 0, 0]
    slice = &mut buf
    write_first slice .println!
    buf[0] as I64 .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["42", "42"]);
}

#[test]
fn test_array_literal_mut_slice_use_infers_integer_expressions() {
    let output = compile_and_run(
        r#"
first_byte: &mut [U8] -> I64
first_byte = s ->
    ptr = (~ArrPtr (*s)) as *U8
    unsafe *ptr as I64

main = ->
    value = 40
    mut buf = [value + 2, 0, 0]
    slice = &mut buf
    first_byte slice .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_array_integer_literal_does_not_infer_as_bool() {
    compile_should_fail(
        r#"
main = ->
    buf: [Bool; 1] = [1]
    0
"#,
        "integer literal resolved to non-integer type `Bool`",
    );
}

#[test]
fn test_fixed_array_borrow_coercion_preserves_original_storage_alias() {
    let output = compile_and_run(
        r#"
slice_addr: &[I64] -> I64
slice_addr = s -> (~ArrPtr (*s)) as I64

first_elem_addr: &[I64] -> I64
first_elem_addr = s -> (&((*s)[0]) as *I64) as I64

main = ->
    arr = [1, 2, 3]
    base = slice_addr &arr
    first = first_elem_addr &arr
    if base == first
        1.println!
    else
        0.println!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn test_arr_ptr_rejects_fixed_array_values() {
    compile_should_fail(
        r#"
main = ->
    arr = [1, 2, 3]
    ptr = ~ArrPtr arr
    unsafe *ptr .println!
    0
"#,
        "ArrPtr expected slice",
    );
}

#[test]
fn test_fixed_array_show_uses_slice_impl_via_coercion() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [1, 2, 3]
    arr.println!
    0
"#,
    );

    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_u8_buffer_show_prints_byte_values() {
    let output = compile_and_run(
        r#"
main = ->
    buf = [104 as U8, 101 as U8, 108 as U8]
    buf.println!
    0
"#,
    );

    assert_eq!(output.trim(), "[104, 101, 108]");
}

#[test]
fn test_explicit_fixed_array_trait_impl_dispatches_at_call_site() {
    let output = compile_and_run(
        r#"
trait FixedArrayCode
    @code = -> 0

impl FixedArrayCode for [I64; 3]
    @code = -> 33

main = ->
    arr: [I64; 3] = [1, 2, 3]
    arr.code!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "33");
}

#[test]
fn test_fixed_array_impl_body_can_use_self_as_array() {
    let output = compile_and_run(
        r#"
trait FixedArraySecond
    @second = -> 0

impl FixedArraySecond for [I64; 3]
    @second = -> self[1]

main = ->
    arr: [I64; 3] = [7, 8, 9]
    arr.second!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_generic_fixed_array_impl_method_uses_receiver_type_substitution() {
    let output = compile_and_run(
        r#"
trait EchoArray
    @echo = x -> x

impl EchoArray for [T; 3]
    @echo = x -> x

main = ->
    arr: [I64; 3] = [7, 8, 9]
    arr.echo 7 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_fixed_array_trait_bound_solver_finds_concrete_impl() {
    let output = compile_and_run(
        r#"
trait FixedArrayCode
    @code = -> 0

impl FixedArrayCode for [I64; 3]
    @code = -> 33

use_code: T -> I64 where T: FixedArrayCode
use_code = value -> value.code!

main = ->
    arr: [I64; 3] = [1, 2, 3]
    use_code arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "33");
}

#[test]
fn test_explicit_fixed_array_index_assignment_updates_storage() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr: [I64; 3] = [1, 2, 3]
    arr[1] = 9
    arr[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn nested_index_mut_updates_matrix_element() {
    let output = compile_and_run(
        r#"
> std::collections::Vec

main = ->
    mut first = Vec::new!
    first.push 1
    first.push 2
    mut second = Vec::new!
    second.push 3
    second.push 4
    mut matrix = Vec::new!
    matrix.push first
    matrix.push second
    matrix[1][0] = 9
    matrix[1][0].println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn index_mut_then_field_updates_record() {
    let output = compile_and_run(
        r#"
struct Record
    < field: I64

main = ->
    mut records: [Record; 2] = [Record field: 1, Record field: 2]
    records[1].field = 9
    records[1].field.println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn indexed_assignment_rhs_uses_index() {
    let output = compile_and_run(
        r#"
struct Cell
    < read_value: I64
    < write_value: I64

impl Index I64 for Cell
    type Output = I64
    @index = _ -> &@read_value

impl IndexMut I64 for Cell
    type Output = I64
    ^@index_mut = _ -> &mut @write_value

main = ->
    mut target = Cell
        read_value: 1
        write_value: 0
    source = Cell
        read_value: 7
        write_value: 2
    target[0] = source[0]
    target.write_value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn mutable_index_reference_uses_index_mut_authority() {
    let output = compile_and_run(
        r#"
struct Cell
    < read_value: I64
    < write_value: I64

impl Index I64 for Cell
    type Output = I64
    @index = _ -> &@read_value

impl IndexMut I64 for Cell
    type Output = I64
    ^@index_mut = _ -> &mut @write_value

main = ->
    mut target = Cell
        read_value: 1
        write_value: 2
    mutable = &mut target[0]
    *mutable = 9
    target.read_value.println!
    target.write_value.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "9"]);
}

#[test]
fn shared_index_reference_uses_index_authority() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr: [I64; 2] = [1, 2]
    shared = &arr[0]
    *shared .println!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn mutable_index_reference_preserves_distinct_index_aliases() {
    let output = compile_and_run(
        r#"
struct Cell
    < read_value: I64
    < write_value: I64

impl Index I64 for Cell
    type Output = I64
    @index = _ -> &@read_value

impl IndexMut I64 for Cell
    type Output = I64
    ^@index_mut = _ -> &mut @write_value

main = ->
    mut target = Cell
        read_value: 1
        write_value: 2
    mutable = &mut target[0]
    *mutable = 9
    shared = &target[0]
    *shared .println!
    target.write_value.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "9"]);
}

#[test]
fn shared_index_borrow_blocks_index_mut_assignment() {
    compile_should_fail(
        r#"
main = ->
    mut arr: [I64; 2] = [1, 2]
    shared: &I64 = &arr[0]
    arr[0] = 9
    value = *shared
    value.println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn shared_range_borrow_blocks_overlapping_mutable_range() {
    compile_should_fail(
        r#"
main = ->
    mut values = [10, 20, 30, 40]
    shared: &[I64] = &values[..2]
    mut overlapping: &mut [I64] = &mut values[1..]
    overlapping[0] = 99
    shared[0].println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn mutable_index_reference_preserves_alias_rules() {
    compile_should_fail(
        r#"
main = ->
    mut arr: [I64; 2] = [1, 2]
    mutable = &mut arr[0]
    shared = &arr[0]
    *mutable .println!
    *shared .println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn aggregate_mut_reference_blocks_shared_access() {
    compile_should_fail(
        r#"
struct Holder
    < reference: &mut I64

main = ->
    mut x = 1
    holder = Holder
        reference: &mut x
    shared = &x
    *holder.reference .println!
    *shared .println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn aggregate_mut_reference_blocks_mutable_access() {
    compile_should_fail(
        r#"
struct Holder
    < reference: &mut I64

main = ->
    mut x = 1
    holder = Holder
        reference: &mut x
    other = &mut x
    *holder.reference .println!
    *other .println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn value_index_reads_release_before_later_mutation() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr: [I64; 2] = [2, 1]
    if arr[0] > arr[1] then
        arr[0] = arr[1]
    arr[0] .println!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn test_array_read_without_index_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
main = ->
    arr: [I64; 3] = [1, 2, 3]
    arr[0]
    0
"#,
        "Cannot use indexing because the Index language-item protocol is unavailable",
    );
}

#[test]
fn test_array_write_without_index_mut_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
main = ->
    arr: [I64; 3] = [1, 2, 3]
    arr[0] = 4
    0
"#,
        "Cannot use mutable indexing because the IndexMut language-item protocol is unavailable",
    );
}

#[test]
fn test_slice_read_without_index_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
read: &[I64] -> I64
read = slice -> slice[0]

main = -> 0
"#,
        "Cannot use indexing because the Index language-item protocol is unavailable",
    );
}

#[test]
fn test_slice_write_without_index_mut_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
write: &mut [I64] -> ()
write = slice -> slice[0] = 4

main = -> 0
"#,
        "Cannot use mutable indexing because the IndexMut language-item protocol is unavailable",
    );
}

#[test]
fn test_pointer_read_without_index_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
read: *I64 -> I64
read = ptr -> unsafe ptr[0]

main = -> 0
"#,
        "Cannot use indexing because the Index language-item protocol is unavailable",
    );
}

#[test]
fn test_pointer_write_without_index_mut_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
write: *I64 -> ()
write = ptr -> unsafe ptr[0] = 4

main = -> 0
"#,
        "Cannot use mutable indexing because the IndexMut language-item protocol is unavailable",
    );
}

#[test]
fn test_foreign_index_impl_for_structural_array_is_rejected() {
    compile_should_fail(
        r#"
impl Index I64 for [Bool; 3]
    type Output = Bool
    @index = i ->
        ptr: *Bool = self as *Bool
        unsafe &(ptr[2 - i])

main = ->
    arr: [Bool; 3] = [false, false, true]
    arr[0].println!
    p = &arr[0]
    *p .println!
    0
"#,
        "orphan impl",
    );
}

#[test]
fn test_generic_use_of_foreign_index_impl_for_structural_array_is_rejected() {
    compile_should_fail(
        r#"
impl Index I64 for [Bool; 3]
    type Output = Bool
    @index = i ->
        ptr: *Bool = self as *Bool
        unsafe &(ptr[2 - i])

is_selected_at_two = value ->
    if value[2]
        1
    else
        0

main = ->
    arr: [Bool; 3] = [true, false, false]
    is_selected_at_two arr .println!
    0
"#,
        "orphan impl",
    );
}

#[test]
fn test_repeated_receiver_generic_impl_does_not_match_incompatible_receiver_args() {
    compile_should_fail(
        r#"
trait SameCode
    @code = -> 0

struct Pair A, B
    < left: A
    < right: B

impl SameCode for Pair T, T
    @code = -> 99

main = ->
    pair = Pair I64, Bool
        left: 1
        right: true
    pair.code!.println!
    0
"#,
        "Unknown field 'code' on struct 'Pair'",
    );
}

#[test]
fn test_explicit_generic_slice_impl_uses_self_body_and_dispatches() {
    let output = compile_and_run(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for &[T]
    @slice_len = ->
        ~ArrayLen (*self)

main = ->
    arr = [97 as U8, 98 as U8, 99 as U8]
    (&arr).slice_len!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_explicit_generic_slice_impl_method_as_value_falls_back_from_u8_slice() {
    let output = compile_and_run(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for &[T]
    @slice_len = ->
        ~ArrayLen (*self)

main = ->
    arr = [97 as U8, 98 as U8, 99 as U8]
    f = (&arr).slice_len
    f!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_mut_ref_assign() {
    let output = compile_example("mir_tests/mut_ref_assign");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
}

#[test]
fn test_std_math() {
    let output = compile_and_run(
        r#"
main = ->
    std::ffi::libc::sqrt 144.0 .println!
    std::ffi::libc::sin 0.0 .println!
    std::ffi::libc::cos 0.0 .println!
    std::ffi::libc::pow 2.0, 10.0 .println!
    std::ffi::libc::ceil 3.2 .println!
    std::ffi::libc::floor 3.8 .println!
    "hello".len!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12"); // sqrt(144)
    assert_eq!(lines[1], "0"); // sin(0)
    assert_eq!(lines[2], "1"); // cos(0)
    assert_eq!(lines[3], "1024"); // pow(2,10)
    assert_eq!(lines[4], "4"); // ceil(3.2)
    assert_eq!(lines[5], "3"); // floor(3.8)
    assert_eq!(lines[6], "5"); // strlen("hello")
}

#[test]
fn test_std_prelude_extern_is_available_without_explicit_import() {
    let output = compile_and_run(
        r#"
main = ->
    sqrt 144.0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_socket_posix_ffi_surface_compiles_and_returns_errno() {
    let output = compile_and_run(
        r#"
> std::ffi::libc::socket
> std::ffi::libc::close
> std::ffi::libc::__errno_location

last_errno = ->
    ptr = __errno_location!
    unsafe *ptr
main = ->
    fd = socket (-1), 1, 0
    if fd < (0 as I32)
        last_errno! .println!
    else
        close fd .println!
    0
"#,
    );

    let errno = output
        .trim()
        .parse::<i64>()
        .expect("errno should print as integer");
    assert!(
        errno > 0,
        "invalid socket domain should set errno, got {errno}"
    );
}

#[test]
fn test_socket_addr_v4_constructors() {
    let output = compile_and_run(
        r#"
> std::io::net::Ipv4Addr
> std::io::net::SocketAddrV4

main = ->
    local = Ipv4Addr::localhost!
    any = Ipv4Addr::any!
    custom = Ipv4Addr::new 1, 2, 3, 4
    addr = SocketAddrV4::localhost 8080
    any_addr = SocketAddrV4::any 7000
    local.a as I64 .println!
    local.d as I64 .println!
    any.a as I64 .println!
    custom.c as I64 .println!
    new_addr = SocketAddrV4::new custom, 9000
    addr.port.println!
    new_addr.ip.a as I64 .println!
    new_addr.port.println!
    any_addr.ip.a as I64 .println!
    any_addr.port.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["127", "1", "0", "3", "8080", "1", "9000", "0", "7000"]
    );
}

#[test]
fn test_tcp_listener_bind_port_zero_reports_local_addr() {
    let output = compile_and_run(
        r#"
> std::io::net::TcpListener
> std::io::net::SocketAddrV4
> std::control::Result

main = ->
    match (TcpListener::bind (SocketAddrV4::localhost 0))
        Result::Ok listener =>
            match listener.local_addr!
                Result::Ok addr =>
                    addr.port > 0 .println!
                    addr.ip.a as I64 .println!
                    addr.ip.b as I64 .println!
                    addr.ip.c as I64 .println!
                    addr.ip.d as I64 .println!
                    0
                Result::Err _ => 2
        Result::Err _ => 1
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["true", "127", "0", "0", "1"]);
}

#[test]
fn test_tcp_listener_fd_field_is_private() {
    compile_should_fail(
        r#"
> std::control::Result
> std::io::net::TcpListener
> std::io::net::SocketAddrV4

main = ->
    match (TcpListener::bind (SocketAddrV4::localhost 0))
        Result::Ok listener =>
            fd = listener.fd
            fd as I64
        Result::Err _ => 1
"#,
        "Field 'fd' of struct 'TcpListener' is private",
    );
}

#[test]
fn test_tcp_stream_fd_field_is_private() {
    compile_should_fail(
        r#"
> std::control::Result
> std::io::net::TcpStream
> std::io::net::SocketAddrV4

main = ->
    match (TcpStream::connect (SocketAddrV4::localhost 1))
        Result::Ok stream =>
            fd = stream.fd
            fd as I64
        Result::Err _ => 1
"#,
        "Field 'fd' of struct 'TcpStream' is private",
    );
}

#[test]
fn test_tcp_stream_connect_failure_returns_error() {
    let output = compile_and_run(
        r#"
> std::control::Result
> std::io::net::TcpStream
> std::io::net::SocketAddrV4
> std::io::IoError

main = ->
    match (TcpStream::connect (SocketAddrV4::localhost 70000))
        Result::Ok _ =>
            0.println!
            0
        Result::Err IoError::InvalidAddress =>
            2.println!
            0
        Result::Err _ =>
            1.println!
            0
"#,
    );

    assert_eq!(output.trim(), "2");
}

#[test]
fn test_tcp_stream_shared_safe_io_and_write_shutdown() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());
    let output = compile_and_run_in_working_dir(
        r#"
> std::concurrency::Arc
> std::io::IoError
> std::io::Write
> std::control::Result
> std::io::net::TcpListener
> std::io::net::TcpStream
> std::io::net::SocketAddrV4

roundtrip: () -> Result I64, IoError
roundtrip = ->
    listener = TcpListener::bind (SocketAddrV4::localhost 0)?
    addr = listener.local_addr!?
    client = Arc::new (TcpStream::connect addr?)
    server = Arc::new (listener.accept!?)
    bytes = [112 as U8, 105, 110, 103, 120]
    written = client.write_all (&bytes[..4])?
    mut received = [0 as U8, 0, 0, 0]
    read = server.recv (&mut received)?
    stopped = client.shutdown_write!
    written.println!
    read.println!
    received[0] as I64 .println!
    received[3] as I64 .println!
    Result::Ok 0

main = ->
    match roundtrip!
        Result::Ok code => code
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(
        output.status.success(),
        "binary failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .lines()
            .collect::<Vec<_>>(),
        ["4", "4", "112", "103"],
    );
}

#[test]
fn test_stdout_write_all_uses_native_slice() {
    let output = compile_and_run(
        r#"
> std::io::stdout

main = ->
    mut output = stdout!
    bytes = [104 as U8, 101, 108, 108, 111]
    written = output.write_all (&bytes[..3])
    0
"#,
    );

    assert_eq!(output, "hel");
}

#[test]
fn test_std_arc_forwards_shared_write_bound() {
    let output = compile_and_run(
        r#"
> std::concurrency::Arc
> std::io::IoError
> std::io::Write
> std::control::Result

struct Sink

impl Write for Sink
    @write = _ -> Result::Ok 1
    @write_str = value -> std::io::write_str self, value
    @write_all = bytes -> std::io::write_all self, bytes

write_one: &W -> Result I64, IoError where W: Write
write_one = target ->
    bytes: [U8; 1] = [0; 1]
    target.write_all (&bytes)

main = ->
    sink = Arc::new Sink
    match write_one (&sink)
        Result::Ok count => count.println!
        Result::Err _ => -1 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn test_write_helpers_handle_partial_string_and_zero_writes() {
    let output = compile_and_run(
        r#"
> std::io::IoError
> std::io::Write
> std::control::Result

struct PartialSink
struct ZeroSink

impl Write for PartialSink
    @write = bytes ->
        len = ~ArrayLen bytes
        Result::Ok (if len < 2
            len
        else
            2)
    @write_str = value -> std::io::write_str self, value
    @write_all = bytes -> std::io::write_all self, bytes

impl Write for ZeroSink
    @write = _ -> Result::Ok 0
    @write_str = value -> std::io::write_str self, value
    @write_all = bytes -> std::io::write_all self, bytes

main = ->
    sink = PartialSink
    bytes = [1 as U8, 2, 3, 4, 5]
    match sink.write_all (&bytes)
        Result::Ok count => count.println!
        Result::Err _ => -1 .println!
    match sink.write_str "hello"
        Result::Ok count => count.println!
        Result::Err _ => -1 .println!
    zero = ZeroSink
    match zero.write_all (&bytes)
        Result::Err IoError::WriteZero => 1 .println!
        _ => 0 .println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["5", "5", "1"]);
}

#[test]
fn test_tcp_listener_stream_localhost_roundtrip() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());
    let output = compile_and_run_in_working_dir(
        r#"
> std::io::IoError
> std::io::Read
> std::io::Write
> std::control::Result
> std::io::net::TcpListener
> std::io::net::TcpStream
> std::io::net::SocketAddrV4

roundtrip: () -> Result I64, IoError
roundtrip = ->
    listener = TcpListener::bind (SocketAddrV4::localhost 0)?
    addr = listener.local_addr!?
    mut client = TcpStream::connect addr?
    mut server = listener.accept!?
    written = client.write_str "ping"?
    mut buf = [0 as U8, 0, 0, 0]
    slice = &mut buf
    read = server.read slice?
    written.println!
    read.println!
    buf[0] as I64 .println!
    buf[3] as I64 .println!
    Result::Ok 0

main = ->
    match roundtrip!
        Result::Ok code => code
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(
        output.status.success(),
        "binary failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines, vec!["4", "4", "112", "103"]);
}

#[test]
fn test_tcp_listener_accept_method_section_in_result_chain() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());
    let output = compile_and_run_in_working_dir(
        r#"
> std::io::IoError
> std::io::Read
> std::io::Write
> std::control::Result
> std::io::net::TcpListener
> std::io::net::TcpStream
> std::io::net::SocketAddrV4

accept_with_method_section: TcpListener -> Result TcpStream, IoError
accept_with_method_section = listener ->
    (listener.local_addr! >>= (ignored -> Result::Ok listener)) >>= (.accept!)

roundtrip: () -> Result I64, IoError
roundtrip = ->
    listener = TcpListener::bind (SocketAddrV4::localhost 0)?
    addr = listener.local_addr!?
    mut client = TcpStream::connect addr?
    mut server = accept_with_method_section listener?
    written = server.write_str "hello"?
    mut buf = [0, 0, 0, 0, 0]
    slice = &mut buf
    read = client.read slice?
    written.println!
    read.println!
    buf[0] as I64 .println!
    buf[4] as I64 .println!
    Result::Ok 0

main = ->
    match roundtrip!
        Result::Ok code => code
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(
        output.status.success(),
        "binary failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines, vec!["5", "5", "104", "111"]);
}

#[test]
fn test_file_create_write_str_uses_generic_write_trait() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());

    let output = compile_and_run_in_working_dir(
        r#"
> std::io::fs::File
> std::io::IoError
> std::io::Write
> std::control::Result

write_text: &mut W -> &Str -> Result I64, IoError where W: Write
write_text = writer, text ->
    written = writer.write_str text?
    Result::Ok written

write_file: &Str -> Result I64, IoError
write_file = path ->
    mut file = File::create path?
    written = write_text (&mut file), "hello"?
    Result::Ok written

main = ->
    match write_file "out.txt"
        Result::Ok written =>
            written.println!
            0
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(
        output.status.success(),
        "binary failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "5");
    assert_eq!(fs::read_to_string(dir.join("out.txt")).unwrap(), "hello");
}

#[test]
fn test_file_open_read_uses_generic_read_trait() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("input.txt"), b"world").unwrap();

    let output = compile_and_run_in_working_dir(
        r#"
> std::io::fs::File
> std::io::IoError
> std::io::Read
> std::control::Result

read_into: &mut R -> &mut [U8] -> Result I64, IoError where R: Read
read_into = reader, buf ->
    read = reader.read buf?
    Result::Ok read

read_file: &Str -> Result I64, IoError
read_file = path ->
    mut file = File::open path?
    mut buf = [0 as U8, 0, 0, 0, 0]
    slice = &mut buf
    read = read_into (&mut file), slice?
    read.println!
    buf[0] as I64 .println!
    buf[4] as I64 .println!
    Result::Ok 0

main = ->
    match read_file "input.txt"
        Result::Ok code => code
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(
        output.status.success(),
        "binary failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim().lines().collect::<Vec<_>>(),
        vec!["5", "119", "100"]
    );
}

#[test]
fn test_file_append_writes_at_end() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("log.txt"), b"first").unwrap();

    let output = compile_and_run_in_working_dir(
        r#"
> std::io::fs::File
> std::io::IoError
> std::io::Write
> std::control::Result

append_text: &Str -> Result I64, IoError
append_text = path ->
    mut file = File::append path?
    written = file.write_str "-second"?
    Result::Ok written

main = ->
    match append_text "log.txt"
        Result::Ok written =>
            written.println!
            0
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "7");
    assert_eq!(
        fs::read_to_string(dir.join("log.txt")).unwrap(),
        "first-second"
    );
}

#[test]
fn test_io_pipe_operator_consumes_reader_and_copies_until_eof() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let contents = "functional byte pipe".repeat(1024);
    fs::write(dir.join("input.txt"), contents.as_bytes()).unwrap();

    let output = compile_and_run_in_working_dir(
        r#"
> std::io::fs::File
> std::io::IoError
> std::control::Result

copy_file: &Str -> &Str -> Result I64, IoError
copy_file = source, target ->
    output = File::create target?
    (File::open source?) |>> &output

main = ->
    match copy_file "input.txt", "output.txt"
        Result::Ok copied =>
            copied.println!
            0
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(
        output.status.success(),
        "pipe binary failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        contents.len().to_string()
    );
    assert_eq!(
        fs::read_to_string(dir.join("output.txt")).unwrap(),
        contents
    );
}

#[test]
fn test_file_create_rejects_embedded_nul_path() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("nul_path.txt"), b"safe").unwrap();

    let output = compile_and_run_in_working_dir(
        r#"
> std::io::fs::File
> std::io::IoError
> std::io::Write
> std::control::Result

overwrite: &Str -> Result I64, IoError
overwrite = path ->
    mut file = File::create path?
    written = file.write_str "mutated"?
    Result::Ok written

main = ->
    match overwrite "nul_path.txt\0suffix"
        Result::Err IoError::InvalidPath => 0
        Result::Ok _ => 1
        Result::Err _ => 2
"#,
        &dir,
    );

    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(dir.join("nul_path.txt")).unwrap(),
        "safe"
    );
}

#[test]
fn test_file_open_missing_path_returns_os_error() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());

    let output = compile_and_run_in_working_dir(
        r#"
> std::io::fs::File
> std::io::IoError
> std::control::Result

main = ->
    match File::open "missing.txt"
        Result::Err IoError::Os _ => 0
        Result::Ok _ => 1
        Result::Err _ => 2
"#,
        &dir,
    );

    assert!(output.status.success());
}

#[test]
fn test_file_fd_field_is_private() {
    compile_should_fail(
        r#"
> std::io::fs::File
> std::io::IoError
> std::control::Result

read_fd: &Str -> Result I64, IoError
read_fd = path ->
    file = File::open path?
    Result::Ok (file.fd as I64)

main = -> 0
"#,
        "Field 'fd' of struct 'File' is private",
    );
}

#[test]
fn test_file_raw_fd_constructor_not_public() {
    compile_should_fail(
        r#"
> std::io::fs::File

main = ->
    file = File
        fd: 0
    0
"#,
        "Cannot construct struct 'std::io::fs::File' because it has private fields",
    );
}

#[test]
fn test_function_name_can_match_c_extern_symbol() {
    let output = compile_and_run(
        r#"
> std::io::net::TcpListener

main = ->
    listen! .println!
    0

listen = -> 42
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_tcp_listener_raw_fd_constructor_not_public() {
    compile_should_fail(
        r#"
> std::io::net::TcpListener

main = ->
    listener = TcpListener
        fd: 0
    0
"#,
        "Cannot construct struct 'std::io::net::TcpListener' because it has private fields",
    );
}

#[test]
fn test_hash_trait_scalar_and_str() {
    let output = compile_and_run(
        r#"
main = ->
    42.hash! .println!
    (-42).hash! .println!
    true.hash! .println!
    false.hash! .println!
    'A'.hash! .println!
    "abc".hash! .println!
    "abc".hash! .println!
    "abd".hash! .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "-42");
    assert_eq!(lines[2], "1");
    assert_eq!(lines[3], "0");
    assert_eq!(lines[4], "65");
    assert_eq!(lines[5], lines[6]);
    assert_ne!(lines[5], lines[7]);
}

#[test]
fn test_method_call_mut_receiver_requires_mutable_binding() {
    compile_should_fail(
        r#"
main = ->
    k = Lol::new!
    k.mdr!
    0

struct Lol
    mdr: I64

impl Lol
    new = ->
        Lol
            mdr: 8
    ^@mdr = -> 2
"#,
        "mutable receiver",
    );
}

#[test]
fn immutable_binding_cannot_call_mut_receiver_method() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@inc: ()
    ^@inc = ->
        self.value = self.value + 1
        return

main = ->
    c = Counter
        value: 0
    c.inc!
    0
"#,
        "mutable receiver",
    );
}

#[test]
fn shared_reference_cannot_call_mut_receiver_method() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@inc: ()
    ^@inc = ->
        self.value = self.value + 1
        return

use_shared: &Counter -> ()
use_shared = c ->
    c.inc!
    return

main = -> 0
"#,
        "mutable receiver",
    );
}

#[test]
fn immutable_field_cannot_call_mut_receiver_method() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@inc: ()
    ^@inc = ->
        self.value = self.value + 1
        return

struct Wrapper
    < counter: Counter

main = ->
    w = Wrapper
        counter: Counter
            value: 0
    w.counter.inc!
    0
"#,
        "mutable receiver",
    );
}

#[test]
fn deferred_method_section_reports_mutable_receiver_requirement() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@advance: Result I64, I64
    ^@advance = ->
        self.value = self.value + 1
        Result::Ok self.value

make: () -> Result Counter, I64
make = ->
    Result::Ok (Counter
        value: 0)

main = ->
    result = make! >>= (.advance!)
    0
"#,
        "Cannot call mutable receiver method 'advance' without a mutable receiver",
    );
}

#[test]
fn deferred_method_section_autorefs_shared_receiver() {
    let output = compile_and_run(
        r#"
> std::control::Result

struct Counter
    < value: I64

impl Counter
    @read: Result I64, I64
    @read = -> Result::Ok self.value

make: () -> Result Counter, I64
make = -> Result::Ok (Counter
    value: 41)

main = ->
    result: Result I64, I64 = make! >>= (.read!)
    result.println!
    0
"#,
    );
    assert_eq!(output.trim(), "Ok(41)");
}

#[test]
fn deferred_method_section_uses_autoderef_receiver() {
    let output = compile_and_run(
        r#"
struct Inner
    < value: I64

impl Inner
    @read = -> self.value

struct Box T
    < value: T

impl Deref for Box T
    type Target = T
    @* = -> &@value

make: () -> Box Inner
make = -> Box
    value: Inner
        value: 42

main = ->
    result = make! |> (.read!)
    result.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn deferred_method_value_execution_preserves_selected_method() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    @read = -> self.value

identity = value -> value

main = ->
    counter = identity (Counter
        value: 43)
    reader = counter.read
    value: I64 = reader!
    value.println!
    0
"#,
    );
    assert_eq!(output.trim(), "43");
}

#[test]
fn deferred_bounded_generic_method_rejects_invalid_argument() {
    compile_should_fail(
        r#"
trait Marker

struct Box

impl Box
    @requires_marker: T -> () where T: Marker
    @requires_marker = _ -> return

make: () -> Result Box, I64
make = -> Result::Ok Box

main = ->
    result = make! >>= (.requires_marker 1)
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn deferred_unknown_method_section_remains_unknown_field() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

make: () -> Result Counter, I64
make = ->
    Result::Ok (Counter
        value: 0)

main = ->
    result = make! >>= (.missing!)
    0
"#,
        "Unknown field 'missing' on struct 'Counter'",
    );
}

#[test]
fn mutable_binding_can_call_mut_receiver_method() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@inc: ()
    ^@inc = ->
        self.value = self.value + 1
        return

    @get: I64
    @get = -> self.value

main = ->
    mut c = Counter
        value: 1
    c.inc!
    c.get! .println!
    0
"#,
    );
    assert_eq!(output.trim(), "2");
}

#[test]
fn trait_bound_mut_receiver_call_autorefs_mut_reference() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

trait Touch
    ^@touch: I64

impl Touch for Counter
    ^@touch = ->
        self.value = self.value + 1
        self.value

poke: &mut T -> I64 where T: Touch
poke = value -> value.touch!

main = ->
    mut counter = Counter
        value: 41
    poke (&mut counter) .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn trait_bound_shared_receiver_on_ref_does_not_double_borrow() {
    let output = compile_and_run(
        r#"
trait Read
    @read: I64

impl Read for I64
    @read = -> *self

read_ref: &T -> I64 where T: Read
read_ref = value -> value.read!

main = ->
    value = 42
    read_ref &value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_invalid_receiver_sequence_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    k = Lol::new!
    k.mdr!.println!
    k.ta!.println!
    k.mdr!.println!
    0

struct Lol
    mdr: I64

impl Lol
    new = ->
        Lol
            mdr: 8
    ^@mdr = -> 2
    ~@ta = -> 1
"#,
        "mutable receiver",
    );
}

#[test]
fn test_inline_program() {
    let output = compile_and_run(
        r#"
max = a, b ->
    if a > b then a else b

min = a, b ->
    if a < b then a else b

main = ->
    max 10, 20 .println!
    min 10, 20 .println!
    max 5, 3 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "20");
    assert_eq!(lines[1], "10");
    assert_eq!(lines[2], "5");
}

#[test]
fn test_inline_nested_functions() {
    let output = compile_and_run(
        r#"
double = x -> x * 2
triple = x -> x * 3

main = ->
    double 5 .println!
    triple 5 .println!
    double (triple 3) .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "15");
    assert_eq!(lines[2], "18");
}

#[test]
fn unit_arrow_evaluates_body_and_discards_named_function_and_lambda_results() {
    let output = compile_and_run(
        r#"
discard = value !->
    7.println!
    value

emit_value = ->
    9.println!
    42

main = !->
    discard 99
    f = value !->
        8.println!
        value
    f 9
    run_effect = !-> emit_value!
    run_effect!
    emit_value!
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        ["7", "8", "9", "9"]
    );
}

#[test]
fn unit_arrow_main_exits_zero_without_std() {
    for source in [
        "main = !-> 42\n",
        "main = !-> (42, 7)\n",
        "main = !-> return\n",
    ] {
        assert_eq!(compile_and_run_without_std(source), 0);
    }
    assert_eq!(compile_and_run_without_std("main = -> 7\n"), 7);
}

#[test]
fn test_main_exit_status_at_all_optimization_levels() {
    let cases = [
        ("std_effect", "main = !-> true.println!\n", true, true, 0, "true\n"),
        ("raw_effect", "extern putchar: I32 -> I32\nmain = !-> putchar (65 as I32)\n", false, true, 0, "A"),
        ("discard_integer", "main = !-> 42\n", false, true, 0, ""),
        ("explicit_unit", "main = -> ()\n", false, true, 0, ""),
        ("early_cleanup", r#"
struct Cleanup
    < value: I64

impl Drop for Cleanup
    ~@drop = !-> self.value.println!

main = !->
    cleanup = Cleanup
        value: 17
    true.println!
    return
"#, true, true, 0, "true\n17\n"),
        ("integer_i64", "main = -> 7\n", false, false, 7, ""),
        ("integer_i32", "main = -> 13 as I32\n", false, false, 13, ""),
    ];
    for opt_level in 0..=3 {
        for (name, source, with_std, unit_return, exit_code, stdout) in cases {
            let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = test_temp_dir(id);
            fs::create_dir_all(&dir).unwrap();
            let _cleanup = TestDirCleanup(dir.clone());
            let entry = dir.join(format!("{name}.rk"));
            fs::write(&entry, source).unwrap();
            let mut config = test_config(entry, dir.clone());
            config.opt_level = opt_level;
            config.emit_llvm = true;
            if !with_std {
                config.extern_artifacts.clear();
                config.no_std = true;
                config.no_prelude = true;
            }
            rock_lib::compile(&config).unwrap_or_else(|error| panic!("{name} at -O {opt_level}: {error:?}"));
            let output = run_test_command(&mut Command::new(dir.join(name)));
            assert_eq!(output.status.code(), Some(exit_code), "{name} at -O {opt_level}");
            assert_eq!(String::from_utf8_lossy(&output.stdout), stdout, "{name} at -O {opt_level}");
            if unit_return {
                let ir = fs::read_to_string(dir.join(format!("{name}.ll"))).unwrap();
                assert!(ir.contains("define internal void @__rock_main()"), "{name} must retain its unit ABI");
                assert!(ir.contains("ret i32 0"), "{name} must return zero from the C entry wrapper");
            }
        }
    }
}

#[test]
fn test_impl_methods() {
    let output = compile_example("impl_methods");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7"); // sum: 3+4
    assert_eq!(lines[1], "3"); // get_x: 3
    assert_eq!(lines[2], "13"); // add 10: 3+10
}

#[test]
fn test_lambdas() {
    let output = compile_example("lambdas");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10"); // double 5
    assert_eq!(lines[1], "14"); // apply double 7
    assert_eq!(lines[2], "105"); // apply (x -> x+100) 5
}

#[test]
fn test_call_argument_holes_desugar_to_lambdas() {
    let output = compile_and_run(
        r#"
struct Boxed
    < value: I64

combine = a, b, c -> a * 100 + b * 10 + c
apply = f, value -> f value
box = value -> Boxed value: value

main = ->
    fixed = 3
    middle = combine 1, _, fixed
    outer = combine _, 5, _
    nested = apply (combine 7, _, 9), 8
    projected = box _.value
    middle 2 .println!
    outer 4, 6 .println!
    nested.println!
    projected 42 .println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        ["123", "456", "789", "42"]
    );
}

#[test]
fn test_modulo_operator() {
    let output = compile_and_run(
        r#"
main = ->
    10 % 3 .println!
    15 % 5 .println!
    7 % 2 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "0");
    assert_eq!(lines[2], "1");
}

#[test]
fn test_string_operations() {
    let output = compile_and_run(
        r#"
main = ->
    "hello".len!.println!
    "".len!.println!
    "world!".len!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "0");
    assert_eq!(lines[2], "6");
}

#[test]
fn test_owned_string_borrowed_view_matches_string_literals() {
    let output = compile_and_run(
        r#"
classify: String -> I64
classify = value ->
    match value.as_str!
        "listen" => 1
        "connect" => 2
        _ => 3

main = ->
    classify (String::from "listen") .println!
    classify (String::from "connect") .println!
    classify (String::from "other") .println!
    0
"#,
    );

    assert_eq!(output, "1\n2\n3\n");
}

#[test]
fn test_owned_string_literal_match_requests_borrowed_str_view() {
    compile_should_fail(
        r#"
main = ->
    value = String::from "listen"
    match value
        "listen" => 1
        _ => 0
"#,
        "call .as_str! before matching an owned String",
    );
}

#[test]
fn test_complex_program() {
    let output = compile_example("complex");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5"); // sqrt(3^2 + 4^2) = 5
    assert_eq!(lines[1], "55"); // sum 1..10
    assert_eq!(lines[2], "true"); // is_even(4)
    assert_eq!(lines[3], "false"); // is_even(7)
}

#[test]
fn test_extern_functions() {
    let output = compile_example("extern_test");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12"); // sqrt(144)
    assert_eq!(lines[1], "1.41421"); // sqrt(2)
}

#[test]
fn test_modules() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    // Write the module file
    std::fs::write(
        dir.join("utils.rk"),
        "\
add = a, b -> a + b

double = x -> x * 2

square = x -> x * x

< add
< double
< square
",
    )
    .unwrap();

    // Write the main file that uses the module
    std::fs::write(
        dir.join("test.rk"),
        "\
mod utils
> utils::add
> utils::double
> utils::square

main = ->
    add 3, 4 .println!
    double 10 .println!
    square 5 .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());

    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        output.status.success(),
        "binary failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout,
        stderr
    );
    assert!(
        !stdout.trim().is_empty(),
        "binary produced empty stdout with status {:?}\nstderr:\n{}",
        output.status.code(),
        stderr
    );

    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "7"); // add 3 4
    assert_eq!(lines[1], "20"); // double 10
    assert_eq!(lines[2], "25"); // square 5
}

#[test]
fn test_module_type_alias_import_export_and_qualification() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("types.rk"), "type Number = I64\n\n< Number\n").unwrap();
    std::fs::write(
        dir.join("test.rk"),
        "mod types\n> types::Number\n\nidentity: Number -> Number\nidentity = value -> value\n\nmain = ->\n    identity 42 .println!\n    0\n",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");
    let output = Command::new(dir.join("test"))
        .output()
        .expect("Failed to run compiled binary");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
}

#[test]
fn test_module_directory_mod_file_resolution() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(dir.join("utils")).unwrap();

    std::fs::write(
        dir.join("utils").join("mod.rk"),
        "\
answer = -> 42

< answer
",
    )
    .unwrap();

    std::fs::write(
        dir.join("test.rk"),
        "\
mod utils
> utils::answer

main = ->
    answer!.println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(stdout.trim(), "42");
}

#[test]
fn test_missing_module_reports_all_searched_paths() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(dir.join("test.rk"), "mod missing\nmain = -> 0\n").unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    let result = rock_lib::compile(&config);
    let diagnostics = result.expect_err("missing module should fail");
    let text = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(text.contains("missing.rk"), "diagnostics were {text}");
    assert!(text.contains("missing/mod.rk"), "diagnostics were {text}");
}

#[test]
fn test_circular_module_load_reports_diagnostic() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(dir.join("test.rk"), "mod a\nmain = -> 0\n").unwrap();
    std::fs::write(dir.join("a.rk"), "mod b\n").unwrap();
    std::fs::write(dir.join("b.rk"), "mod a\n").unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    let result = rock_lib::compile(&config);
    let diagnostics = result.expect_err("cycle should fail");
    let text = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        text.contains("Circular module load"),
        "diagnostics were {text}"
    );
    assert!(text.contains("a.rk"), "diagnostics were {text}");
    assert!(text.contains("b.rk"), "diagnostics were {text}");
}

#[test]
fn test_module_glob_import_and_export() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("math_impl.rk"),
        "\
add = a, b -> a + b

double = x -> x * 2

< add
< double
",
    )
    .unwrap();

    std::fs::write(
        dir.join("api.rk"),
        "\
mod math_impl

< math_impl::*
",
    )
    .unwrap();

    std::fs::write(
        dir.join("test.rk"),
        "\
mod api
> api::*

main = ->
    add 2, 5 .println!
    double 9 .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());

    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "18");
}

#[test]
fn test_id_owned_hir_preserves_same_name_module_calls() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("left.rk"),
        "\
value = -> 11

< value
",
    )
    .unwrap();
    std::fs::write(
        dir.join("right.rk"),
        "\
value = -> 31

< value
",
    )
    .unwrap();
    std::fs::write(
        dir.join("test.rk"),
        "\
mod left
mod right

main = ->
    left::value! + right::value! .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(stdout.trim(), "42");
}

#[test]
fn test_module_with_structs() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    // Write the math module with a struct and functions
    std::fs::write(
        dir.join("geometry.rk"),
        "\
struct Point
    < x: I64
    < y: I64

make_point = a, b ->
    Point
        x: a
        y: b

distance_sq = p ->
    p.x * p.x + p.y * p.y

< Point
< make_point
< distance_sq
",
    )
    .unwrap();

    // Write the main file
    std::fs::write(
        dir.join("test.rk"),
        "\
mod geometry
> geometry::Point
> geometry::make_point
> geometry::distance_sq

main = ->
    p = make_point 3, 4
    p.x.println!
    p.y.println!
    distance_sq p .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());

    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "3"); // p.x
    assert_eq!(lines[1], "4"); // p.y
    assert_eq!(lines[2], "25"); // 3^2 + 4^2 = 25
}

#[test]
fn test_array_indexing() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [10, 20, 30, 40, 50]
    arr[0] .println!
    arr[2] .println!
    arr[4] .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "30");
    assert_eq!(lines[2], "50");
}

#[test]
fn test_array_for_loop() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [1, 2, 3, 4, 5]
    sum = 0
    for x in arr
        sum = sum + x
    sum.println!
    0
"#,
    );
    assert_eq!(output.trim(), "15");
}

#[test]
fn test_string_concat() {
    let output = compile_and_run(
        r#"
main = ->
    a = "Hello, "
    b = "World!"
    c = string_concat a, b
    c.println!
    0
"#,
    );
    assert_eq!(output.trim(), "Hello, World!");
}

#[test]
fn test_string_add_operator_concats_owned_string_and_borrowed_str() {
    let output = compile_and_run(
        r#"
main = ->
    (String::from "Hello, ") + "World!" .println!
    "Hello, " + "World!" .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["Hello, World!"; 2]);
}

#[test]
fn test_struct_field_assignment() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    < y: I64

main = ->
    p = Point
        x: 10
        y: 20
    p.x.println!
    p.x = 42
    p.x.println!
    p.y.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10"); // original x
    assert_eq!(lines[1], "42"); // modified x
    assert_eq!(lines[2], "20"); // y unchanged
}

#[test]
fn test_array_index_assignment() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr = [10, 20, 30]
    arr[1] .println!
    arr[1] = 99
    arr[1] .println!
    arr[0] .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "20"); // original arr[1]
    assert_eq!(lines[1], "99"); // modified arr[1]
    assert_eq!(lines[2], "10"); // arr[0] unchanged
}

#[test]
fn test_slice_indexing() {
    let output = compile_and_run(
        r#"
read_second: &[I64] -> I64
read_second = slice -> slice[1]

main = ->
    arr = [7, 8, 9]
    read_second &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_native_ranges_borrow_slice_views() {
    let output = compile_and_run(
        r#"
inspect: &[I64] -> ()
inspect = slice !->
    (~ArrayLen slice).println!
    slice[0].println!

inspect_ranges: &[I64] -> ()
inspect_ranges = slice !->
    inspect (&slice[..2])
    inspect (&slice[1..])
    inspect (&slice[1..3])
    inspect (&slice[..=1])
    inspect (&slice[1..=2])
    inspect (&slice[..])
    selected = 1..3
    inspect (&slice[selected])

main = ->
    values = [10, 20, 30, 40]
    inspect_ranges (&values)
    inspect (&values[..2])
    0
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        ["2", "10", "3", "20", "2", "20", "2", "10", "2", "20", "4", "10", "2", "20", "2", "10"]
    );
}

#[test]
fn test_inclusive_native_range_for_loop_includes_end() {
    let output = compile_and_run(
        r#"
main = ->
    mut total = 0
    for value in 1..=3
        total = total + value
    total.println!
    0
"#,
    );

    assert_eq!(output.trim(), "6");
}

#[test]
fn test_slice_index_assignment() {
    let output = compile_and_run(
        r#"
write_second: &mut [I64] -> I64
write_second = input ->
    mut slice = input
    slice[1] = 42
    0

main = ->
    mut values = [7, 8, 9]
    write_second (&mut values)
    values[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_native_mutable_range_updates_original_slice() {
    let output = compile_and_run(
        r#"
update_middle: &mut [I64] -> ()
update_middle = slice !->
    mut middle = &mut slice[1..3]
    middle[0] = 77

main = ->
    mut values = [10, 20, 30, 40]
    update_middle (&mut values)
    values[1].println!
    values[2].println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), ["77", "30"]);
}

#[test]
fn test_native_ranges_index_vec_views() {
    let output = compile_and_run(
        r#"
inspect: &[I64] -> ()
inspect = slice !->
    (~ArrayLen slice).println!
    slice[0].println!

update_prefix: &mut (Vec I64) -> ()
update_prefix = values !->
    mut prefix: &mut [I64] = &mut (*values)[..2]
    prefix[0] = 55

main = ->
    mut values: Vec I64 = Vec::new!
    values.push 10
    values.push 20
    values.push 30
    inspect (&values[1..])
    update_prefix (&mut values)
    values[0].println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), ["2", "20", "55"]);
}

#[test]
fn test_native_range_out_of_bounds_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
main = ->
    values = [10, 20, 30]
    slice = &values[1..4]
    (~ArrayLen *slice).println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("range out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_fixed_array_index_negative_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
main = ->
    arr = [7, 8, 9]
    arr[(0 - 1)].println!
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
fn test_fixed_array_index_upper_bound_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
main = ->
    arr = [7, 8, 9]
    arr[3].println!
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
fn test_slice_index_negative_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
read_negative: &[I64] -> I64
read_negative = slice -> slice[(0 - 1)]

main = ->
    arr = [7, 8, 9]
    read_negative &arr .println!
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
fn test_slice_index_upper_bound_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
read_upper: &[I64] -> I64
read_upper = slice -> slice[3]

main = ->
    arr = [7, 8, 9]
    read_upper &arr .println!
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
fn test_type_conversion() {
    let output = compile_and_run(
        r#"
main = ->
    x = 42
    f = to_float x
    f.println!
    y = 3.7
    i = to_int y
    i.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42"); // int to float (printed with %g)
    assert_eq!(lines[1], "3"); // float to int (truncates)
}

#[test]
fn test_array_len() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [10, 20, 30, 40, 50]
    ~ArrayLen arr .println!
    empty = [1]
    ~ArrayLen empty .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "1");
}

#[test]
fn test_array_len_accepts_borrowed_slice() {
    let output = compile_and_run(
        r#"
sum_array: &[I64] -> I64
sum_array = arr ->
    total = 0
    i = 0
    len = ~ArrayLen arr
    while i < len
        total = total + arr[i]
        i = i + 1
    total

main = ->
    arr = [1, 2, 3, 4]
    sum_array &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "10");
}

#[test]
fn test_boolean_printing() {
    let output = compile_and_run(
        r#"
main = ->
    true.println!
    false.println!
    3 > 2 .println!
    3 < 2 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "false");
    assert_eq!(lines[2], "true");
    assert_eq!(lines[3], "false");
}

#[test]
fn test_comprehensive_program() {
    // Tests: structs, methods, enums, match, arrays, loops, functions, recursion, strings
    let output = compile_and_run(
        r#"
struct Vec2
    < x: I64
    < y: I64

impl Vec2
    @dot = other ->
        @x * other.x + @y * other.y

    @magnitude_sq = ->
        @x * @x + @y * @y

abs = n ->
    if n < 0 then 0 - n else n

gcd = a, b ->
    if b == 0 then a
    else gcd b, (a % b)

sum_array: &[I64] -> I64
sum_array = arr ->
    total = 0
    i = 0
    while i < (~ArrayLen arr)
        total = total + arr[i]
        i = i + 1
    total

main = ->
    v1 = Vec2
        x: 3
        y: 4
    v2 = Vec2
        x: 1
        y: 2

    v1.dot v2 .println!
    v1.magnitude_sq!.println!

    gcd 48, 18 .println!

    arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    sum_array &arr .println!

    abs 42 .println!
    abs (0 - 7) .println!

    s1 = string_concat "Rock", " "
    greeting = s1.concat (String::from "lang!")
    greeting.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "11"); // dot product: 3*1 + 4*2 = 11
    assert_eq!(lines[1], "25"); // magnitude_sq: 9 + 16 = 25
    assert_eq!(lines[2], "6"); // gcd(48, 18) = 6
    assert_eq!(lines[3], "55"); // sum 1..10 = 55
    assert_eq!(lines[4], "42"); // abs(42) = 42
    assert_eq!(lines[5], "7"); // abs(-7) = 7
    assert_eq!(lines[6], "Rock lang!"); // string concat
}

#[test]
fn test_closure_capture() {
    let output = compile_and_run(
        r#"
main = ->
    base = 100
    f = x -> x + base
    f 5 .println!
    f 20 .println!
    offset = 10
    g = y -> y * 2 + offset
    g 3 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "105"); // 5 + 100
    assert_eq!(lines[1], "120"); // 20 + 100
    assert_eq!(lines[2], "16"); // 3*2 + 10
}

#[test]
fn test_closure_capture_still_works_without_codegen_rebinding() {
    let output = compile_and_run(
        r#"
main = ->
    x = 1
    f = -> x.println!
    f!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn test_std_thread_spawn_join_returns_value() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn

main = ->
    match spawn (-> 42)
        Result::Ok handle =>
            match handle.join!
                Result::Ok value => value.println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_std_thread_handle_drop_detaches_without_waiting() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::sleep_ms
> std::concurrency::thread::spawn

launch = ->
    match spawn (->
        sleep_ms 500
        "worker finished".println!
        return)
        Result::Ok _ => 0
        Result::Err _ => 1

main = ->
    code = launch!
    "handle dropped".println!
    code
"#,
    );

    assert_eq!(output.trim(), "handle dropped");
}

#[test]
fn test_std_detached_thread_drops_unobserved_result() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::sleep_ms
> std::concurrency::thread::spawn

struct DetachedResult

impl Drop for DetachedResult
    ~@drop = ->
        "detached result dropped".println!
        return

launch = ->
    match spawn (-> DetachedResult)
        Result::Ok _ => 0
        Result::Err _ => 1

main = ->
    code = launch!
    sleep_ms 100
    code
"#,
    );

    assert_eq!(output.trim(), "detached result dropped");
}

#[test]
fn test_unit_name_is_not_a_primitive_type_spelling() {
    compile_should_fail(
        r#"
identity: Unit -> Unit
identity = value -> value

main = -> 0
"#,
        "unknown type 'Unit'",
    );
}

#[test]
fn test_std_thread_spawn_accepts_owned_send_capture() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn

struct Owned
    < value: I64

impl Owned
    ~@take = -> self.value

main = ->
    owned = Owned
        value: 73
    match spawn (-> owned.take!)
        Result::Ok handle =>
            match handle.join!
                Result::Ok value => value.println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!
    0
"#,
    );

    assert_eq!(output.trim(), "73");
}

#[test]
fn test_std_thread_spawn_rejects_borrowed_capture() {
    compile_should_fail(
        r#"
> std::concurrency::thread::spawn

main = ->
    value = 41
    handle = spawn (-> value + 1)
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_std_thread_spawn_rejects_borrowed_capture_after_branch_merge() {
    compile_should_fail(
        r#"
> std::concurrency::thread::spawn

main = ->
    value = 41
    task = if true then (-> value + 1) else (-> 0)
    handle = spawn task
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_std_thread_spawn_rejects_borrowed_method_value() {
    compile_should_fail(
        r#"
> std::concurrency::thread::spawn

struct Value
    < number: I64

impl Value
    @read = -> self.number

main = ->
    value = Value
        number: 41
    task = value.read
    handle = spawn task
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_std_thread_spawn_rejects_downstream_send_impl() {
    compile_should_fail(
        r#"
> std::concurrency::thread::spawn

struct FakeSend
    < pointer: *I64

impl Send for FakeSend

impl FakeSend
    ~@consume = -> 1

main = ->
    value = FakeSend
        pointer: 0 as *I64
    handle = spawn (-> value.consume!)
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_std_thread_spawn_accepts_struct_callable_impl() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn

struct Task

impl FnOnce (), I64 for Task
    type Output = I64
    ~@call_once = _ -> 64

main = ->
    match spawn Task
        Result::Ok handle =>
            match handle.join!
                Result::Ok value => value.println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!
    0
"#,
    );

    assert_eq!(output.trim(), "64");
}

#[test]
fn test_std_arc_mutex_synchronizes_threaded_updates() {
    let output = compile_and_run(
        r#"
> std::concurrency::Arc
> std::concurrency::Mutex
> std::concurrency::thread::spawn

struct Counter
    < value: I64

impl Counter
    ^@increment = -> self.value = self.value + 1

struct Worker
    < counter: Arc (Mutex Counter)
    < iterations: I64

impl Worker
    ~@run: I64
    ~@run = ->
        mut i: I64 = 0
        while i < self.iterations
            mut guard = self.counter.lock!
            guard.get_mut!.increment!
            i = i + 1
        i

main = ->
    counter = Arc::new (Mutex::new (Counter
        value: 0))
    first = Worker
        counter: counter.clone!
        iterations: 1000
    second = Worker
        counter: counter.clone!
        iterations: 1000
    first_handle = spawn (-> first.run!)
    second_handle = spawn (-> second.run!)
    match first_handle
        Result::Ok handle => result = handle.join!
        Result::Err _ => "first spawn failed".println!
    match second_handle
        Result::Ok handle => result = handle.join!
        Result::Err _ => "second spawn failed".println!
    guard = counter.lock!
    guard.get!.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "2000");
}

#[test]
fn test_std_mutex_guard_drop_unlocks_mutex() {
    let output = compile_and_run(
        r#"
> std::concurrency::Mutex

set_value: &Mutex I64 -> ()
set_value = mutex ->
    mut guard = mutex.lock!
    *guard = 41
    return

main = ->
    mutex = Mutex::new 0
    set_value (&mutex)
    guard = mutex.lock!
    guard.println!
    0
"#,
    );

    assert_eq!(output.trim(), "41");
}

#[test]
fn test_std_mutex_guard_is_not_send() {
    compile_should_fail(
        r#"
> std::concurrency::Mutex
> std::concurrency::thread::spawn

main = ->
    mutex = Mutex::new 7
    guard = mutex.lock!
    handle = spawn (-> guard.get!)
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_std_thread_spawn_rejects_reference_hidden_in_moved_struct() {
    compile_should_fail(
        r#"
> std::concurrency::thread::spawn

struct Borrowed
    < value: &I64

impl Borrowed
    ~@read = -> *self.value

main = ->
    value = 7
    borrowed = Borrowed
        value: &value
    handle = spawn (-> borrowed.read!)
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_std_mutex_try_lock_observes_guard_lifetime() {
    let output = compile_and_run(
        r#"
> std::concurrency::Mutex

lock_once: &Mutex I64 -> ()
lock_once = mutex ->
    guard = mutex.lock!
    match mutex.try_lock!
        Option::Some _ => "unexpected lock".println!
        Option::None => "locked".println!
    return

main = ->
    mutex = Mutex::new 9
    lock_once (&mutex)
    match mutex.try_lock!
        Option::Some guard => guard.get!.println!
        Option::None => "still locked".println!
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), ["locked", "9"]);
}

#[test]
fn test_std_arc_drops_owned_value_once() {
    let (output, success) = compile_and_run_with_status(
        r#"
> std::concurrency::Arc

main = ->
    value = Arc::new (String::from "shared")
    other = value.clone!
    value.println!
    other.println!
    0
"#,
    );

    assert!(success);
    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        ["shared", "shared"]
    );
}

#[test]
fn test_atomic_intrinsics_require_unsafe() {
    compile_should_fail(
        r#"
main = ->
    ptr = 0 as *U64
    previous = ~AtomicU64Exchange ptr, (1 as U64)
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_atomic_intrinsics_require_exact_u64_operands() {
    compile_should_fail(
        r#"
main = ->
    ptr = 0 as *U64
    previous = unsafe ~AtomicU64FetchAdd ptr, (1 as U8)
    0
"#,
        "expected argument type U64, found U8",
    );
}

#[test]
fn test_atomic_intrinsics_require_exact_argument_count() {
    compile_should_fail(
        r#"
main = ->
    ptr = 0 as *U64
    previous = unsafe ~AtomicU64FetchSub ptr
    0
"#,
        "expects exactly 2 arguments",
    );
}

#[test]
fn test_bubble_sort() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr = [5, 3, 1, 4, 2]
    i = 0
    while i < 4
        j = i + 1
        while j < 5
            if arr[i] > arr[j] then
                temp = arr[i]
                arr[i] = arr[j]
                arr[j] = temp
            j = j + 1
        i = i + 1
    k = 0
    while k < 5
        arr[k] .println!
        k = k + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2", "3", "4", "5"]);
}

#[test]
fn test_fibonacci_loop() {
    let output = compile_and_run(
        r#"
main = ->
    a = 0
    b = 1
    i = 0
    while i < 10
        a.println!
        temp = a + b
        a = b
        b = temp
        i = i + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["0", "1", "1", "2", "3", "5", "8", "13", "21", "34"]
    );
}

#[test]
fn test_nested_function_calls() {
    let output = compile_and_run(
        r#"
square = x -> x * x
double = x -> x * 2
add = x, y -> x + y

main = ->
    add (square 3), (double 5) .println!
    square (double 3) .println!
    double (square 4) .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "19"); // 9 + 10
    assert_eq!(lines[1], "36"); // (3*2)^2 = 36
    assert_eq!(lines[2], "32"); // (4^2)*2 = 32
}

#[test]
fn test_multi_string_concat() {
    let output = compile_and_run(
        r#"
main = ->
    a = "Hello"
    b = ", "
    c = "World"
    d = "!"
    s1 = string_concat a, b
    s2 = s1.concat (String::from c)
    result = s2.concat (String::from d)
    result.println!
    result.len!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "Hello, World!");
    assert_eq!(lines[1], "13");
}

#[test]
fn test_method_signature_omits_shared_self_receiver() {
    let (output, success) = compile_and_run_with_status(
        r#"
struct Counter
    < value: I64

impl Counter
    @value: I64
    @value = -> self.value

main = ->
    c = Counter
        value: 7
    c.value! .println!
    0
"#,
    );

    assert!(success);
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_method_signature_omits_mut_and_move_self_receiver() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@set: I64 -> ()
    ^@set = value ->
        self.value = value
        return

    ~@take: I64
    ~@take = -> self.value

main = ->
    mut c = Counter
        value: 1
    c.set 9
    c.take! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_trait_method_signature_omits_self_receiver() {
    let output = compile_and_run(
        r#"
trait Value
    @value: I64

struct Boxed
    < value: I64

impl Value for Boxed
    @value = -> @value

main = ->
    b = Boxed
        value: 11
    b.value! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "11");
}

#[test]
fn test_trait_method_signature_implicit_self_preserves_associated_output() {
    let output = compile_and_run(
        r#"
trait Projector
    type Output
    @project: Self::Output -> Self::Output

struct Id

impl Projector for Id
    type Output = I64
    @project = value -> value

main = ->
    id = Id
    id.project 13 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "13");
}

#[test]
fn test_bang_rejects_method_with_required_argument() {
    compile_should_fail(
        r#"
struct Counter

impl Counter
    @set: I64 -> I64
    @set = value -> value

main = ->
    counter = Counter
    counter.set! 9
    0
"#,
        "expected 1 args, got 0",
    );
}

#[test]
fn test_unsafe_trait_signature_bound_dispatch_requires_unsafe() {
    compile_should_fail(
        r#"
trait Risky
    unsafe @read: I64

struct Secret
    < value: I64

impl Risky for Secret
    @read = -> @value

read_value: T -> I64 where T: Risky
read_value = value -> value.read!

main = ->
    secret = Secret
        value: 7
    value = read_value secret
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_trait_signature_current_trait_dispatch_requires_unsafe() {
    compile_should_fail(
        r#"
trait Risky
    unsafe @read: I64
    @read_safely: I64
    @read_safely = -> self.read!

struct Secret
    < value: I64

impl Risky for Secret
    @read = -> @value

main = ->
    secret = Secret
        value: 7
    value = secret.read_safely!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_method_signature_allows_explicit_self_operand_after_implicit_receiver() {
    let output = compile_and_run(
        r#"
trait Same
    @same: Self -> Bool

struct Token
    < value: I64

impl Same for Token
    @same = other -> self.value == other.value

main = ->
    a = Token
        value: 4
    b = Token
        value: 4
    a.same b .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

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
fn test_enum_pattern_matching() {
    let output = compile_and_run(
        r#"
enum Option
    Some I64
    None

unwrap_or = x, default ->
    match x
        Option::Some val => val
        Option::None => default

main = ->
    a = Option::Some 42
    b = Option::None
    r1 = unwrap_or a, 0
    r2 = unwrap_or b, 99
    r1.println!
    r2.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "99");
}

#[test]
fn test_enum_multiple_variants() {
    let output = compile_and_run(
        r#"
enum Color
    Red
    Green
    Blue

to_num = c ->
    match c
        Color::Red => 1
        Color::Green => 2
        Color::Blue => 3

main = ->
    r = Color::Red
    g = Color::Green
    b = Color::Blue
    to_num r .println!
    to_num g .println!
    to_num b .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "3");
}

#[test]
fn test_unqualified_same_named_enum_variant_pattern_uses_scrutinee_owner() {
    let output = compile_and_run(
        r#"
enum AlphaChoice
    Hit Bool
    Miss

enum DecoyChoiceA
    Hit Bool
    Miss

enum DecoyChoiceB
    Hit Bool
    Miss

enum DecoyChoiceC
    Hit Bool
    Miss

enum DecoyChoiceD
    Hit Bool
    Miss

enum DecoyChoiceE
    Hit Bool
    Miss

enum DecoyChoiceF
    Hit Bool
    Miss

enum DecoyChoiceG
    Hit Bool
    Miss

enum DecoyChoiceH
    Hit Bool
    Miss

enum DecoyChoiceI
    Hit Bool
    Miss

enum DecoyChoiceJ
    Hit Bool
    Miss

enum BetaChoice
    Hit I64
    Miss

score: BetaChoice -> I64
score = choice ->
    match choice
        Hit value => value + 1
        Miss => 0

main = ->
    score (BetaChoice::Hit 41) .println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_unqualified_enum_variant_pattern_rejects_known_non_enum_scrutinee() {
    compile_should_fail(
        r#"
enum Choice
    Hit

main = ->
    match 1
        Hit => 1
        _ => 0
"#,
        "pattern type mismatch",
    );
}

#[test]
fn test_qualified_enum_variant_pattern_rejects_different_enum_with_same_variant_name() {
    compile_should_fail(
        r#"
enum A
    Hit I64
    Miss

enum B
    Hit I64
    Miss

score: A -> I64
score = value ->
    match value
        B::Hit n => n
        A::Hit n => n + 1
        A::Miss => 0

main = ->
    score (A::Hit 41)
"#,
        "pattern type mismatch",
    );
}

#[test]
fn test_unqualified_enum_variant_pattern_rejects_variant_missing_from_known_enum() {
    compile_should_fail(
        r#"
enum A
    Hit

enum B
    OnlyOnB

score: A -> I64
score = value ->
    match value
        OnlyOnB => 1
        Hit => 0

main = ->
    score A::Hit
"#,
        "pattern type mismatch",
    );
}

#[test]
fn test_comprehensive_program_v2() {
    let output = compile_and_run(
        r#"
struct Matrix
    < a: I64
    < b: I64
    < c: I64
    < d: I64

impl Matrix
    @det = -> @a * @d - @b * @c
    @mul = other ->
        Matrix
            a: @a * other.a + @b * other.c
            b: @a * other.b + @b * other.d
            c: @c * other.a + @d * other.c
            d: @c * other.b + @d * other.d

enum Result
    Ok I64
    Err I64

safe_div = a, b ->
    if b == 0
        Result::Err 0
    else
        Result::Ok (a / b)

unwrap = r ->
    match r
        Result::Ok val => val
        Result::Err _ => 0 - 1

gcd = a, b ->
    while b > 0
        t = b
        b = a % b
        a = t
    a

is_prime = n ->
    if n < 2
        0
    else
        i = 2
        result = 1
        while i * i <= n
            if n % i == 0
                result = 0
            i = i + 1
        result

main = ->
    m1 = Matrix
        a: 1
        b: 2
        c: 3
        d: 4
    m2 = Matrix
        a: 5
        b: 6
        c: 7
        d: 8
    m1.det!.println!
    m3 = m1.mul m2
    m3.a.println!
    m3.d.println!
    r1 = safe_div 10, 3
    r2 = safe_div 10, 0
    unwrap r1 .println!
    unwrap r2 .println!
    gcd 48, 18 .println!
    is_prime 7 .println!
    is_prime 15 .println!
    is_prime 97 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "-2"); // det(1,2,3,4) = 1*4-2*3
    assert_eq!(lines[1], "19"); // m3.a = 1*5+2*7
    assert_eq!(lines[2], "50"); // m3.d = 3*6+4*8
    assert_eq!(lines[3], "3"); // 10/3
    assert_eq!(lines[4], "-1"); // safe_div 10,0 -> Err -> -1
    assert_eq!(lines[5], "6"); // gcd(48,18)
    assert_eq!(lines[6], "1"); // is_prime(7)
    assert_eq!(lines[7], "0"); // is_prime(15)
    assert_eq!(lines[8], "1"); // is_prime(97)
}

#[test]
fn test_std_try_protocol_methods() {
    let output = compile_and_run(
        r#"
main: () -> I64
main = ->
    input: Result I64, I64 = Result::Ok 41
    res = input.branch!
    value = match res
        ControlFlow::Continue value => value + 1
        ControlFlow::Break residual => 0
    value.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_std_into_uses_from_blanket_impl() {
    let output = compile_and_run(
        r#"
struct Small
    < value: I64

struct Big
    < value: I64

impl From Small for Big
    from = small ->
        Big
            value: small.value + 1

main = ->
    small = Small
        value: 41
    big: Big = small.into!
    big.value.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_static_trait_overloads_infer_arguments_without_std() {
    let status = compile_and_run_without_std(
        r#"
trait Convert T
    convert: T -> Self

struct Value
    < value: I64

impl Convert I64 for Value
    convert = input -> Value
        value: input

impl Convert Bool for Value
    convert = input -> Value
        value: if input
            42
        else
            0

main = ->
    convert_integer: I64 -> Value = Value::convert
    integer = convert_integer 7
    boolean = Value::convert true
    ~I64Add integer.value, boolean.value
"#,
    );
    assert_eq!(status, 49);
}

#[test]
fn test_std_string_from_conversions() {
    let output = compile_and_run(
        r#"
convert: T -> U where U: From T
convert = value -> U::from value

main = !->
    text = String::from "hello"
    text.println!
    text.len!.println!
    String::from 42 .println!
    String::from (-7) .println!
    String::from 0 .println!
    String::from 3.5 .println!
    String::from 'R' .println!
    String::from "" .len!.println!
    bytes = [82 as U8, 111 as U8, 99 as U8, 107 as U8]
    String::from (&bytes) .println!
    generic_text: String = convert "generic"
    generic_number: String = convert 123
    generic_text.println!
    generic_number.println!
    converted: String = 7.into!
    converted.println!
    owned = String::from text
    owned.println!
"#,
    );
    assert_eq!(
        output,
        "hello\n5\n42\n-7\n0\n3.5\nR\n0\nRock\ngeneric\n123\n7\nhello\n"
    );
}

#[test]
fn test_std_string_from_rejects_unsupported_source() {
    compile_should_fail(
        r#"
main = !->
    String::from true .println!
"#,
        "does not implement trait",
    );
}

#[test]
fn test_std_fixed_u8_arrays_convert_to_string_for_any_length() {
    let output = compile_and_run(
        r#"
main = ->
    short = [82 as U8, 111 as U8, 99 as U8, 107 as U8]
    short_string: String = String::from (&short)
    short_string.println!

    long = [72 as U8, 101 as U8, 108 as U8, 108 as U8, 111 as U8, 44 as U8, 32 as U8, 82 as U8, 111 as U8, 99 as U8, 107 as U8]
    long_string: String = String::from (&long)
    long_string.println!

    from_bytes = [70 as U8, 114 as U8, 111 as U8, 109 as U8]
    from_string: String = String::from (&from_bytes)
    from_string.println!

    empty: [U8; 0] = []
    empty_string: String = String::from (&empty)
    empty_string.len!.println!
    0
"#,
    );

    assert_eq!(output, "Rock\nHello, Rock\nFrom\n0\n");
}

#[test]
fn constructor_trait_selection_option_pure_compiles() {
    compile_should_pass(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Maybe T
    None
    Some T

impl Applicative for Maybe
    pure = value -> Maybe::Some value

main = ->
    value = Maybe::pure 42
    0
"#,
    );
}

#[test]
fn constructor_trait_selection_generic_call_specializes_before_mir() {
    compile_should_pass(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Maybe T
    None
    Some T

impl Applicative for Maybe
    pure = value -> Maybe::Some value

repure: F I64 -> F I64 where F _: Applicative
repure = ignored -> F::pure 42

main = ->
    value = repure (Maybe::Some 1)
    0
"#,
    );
}

#[test]
fn constructor_trait_selection_result_section_specializes_before_llvm() {
    compile_should_pass(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Outcome T, E
    Ok T
    Err E

struct IoError

impl Applicative for Outcome _, E
    pure = value -> Outcome::Ok value

repure: Outcome I64, E -> Outcome I64, E
repure = ignored -> (Outcome _, E)::pure 42

main = ->
    input: Outcome I64, IoError = Outcome::Ok 1
    value = repure input
    0
"#,
    );
}

#[test]
fn constructor_trait_selection_rejects_bare_binary_constructor_for_unary_trait() {
    compile_should_fail(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Outcome T, E
    Ok T
    Err E

impl Applicative for Outcome _, E
    pure = value -> Outcome::Ok value

main = ->
    value = Outcome::pure 42
    0
"#,
        "has kind Type -> Type -> Type",
    );
}

#[test]
fn test_try_custom_carrier_uses_try_and_from_residual() {
    let output = compile_and_run(
        r#"
enum MyFlow T
    Value T
    Stop I64

enum MyResidual
    Stop I64

impl Try for MyFlow T
    type Output = T
    type Residual = MyResidual

    ~@branch = ->
        match self
            MyFlow::Value value => ControlFlow::Continue value
            MyFlow::Stop code => ControlFlow::Break (MyResidual::Stop code)

impl FromResidual MyResidual for MyFlow T
    from_residual = residual ->
        match residual
            MyResidual::Stop code => MyFlow::Stop code

next: Bool -> MyFlow I64
next = ok ->
    if ok
        MyFlow::Value 41
    else
        MyFlow::Stop 7

compute: Bool -> MyFlow I64
compute = ok ->
    value = next ok?
    MyFlow::Value (value + 1)

main = ->
    success = compute true
    failure = compute false
    match success
        MyFlow::Value value => value.println!
        MyFlow::Stop code => code.println!
    match failure
        MyFlow::Value value => value.println!
        MyFlow::Stop code => code.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["42", "7"]);
}

#[test]
fn test_try_cross_residual_conversion_uses_distinct_return_carrier() {
    let output = compile_and_run(
        r#"
enum Inner T
    Value T
    Stop I64

enum Outer T
    Value T
    Stop I64

enum InnerResidual
    Stop I64

impl Try for Inner T
    type Output = T
    type Residual = InnerResidual

    ~@branch = ->
        match self
            Inner::Value value => ControlFlow::Continue value
            Inner::Stop code => ControlFlow::Break (InnerResidual::Stop code)

impl FromResidual InnerResidual for Outer T
    from_residual = residual ->
        match residual
            InnerResidual::Stop code => Outer::Stop code

source = ok ->
    if ok
        Inner::Value 41
    else
        Inner::Stop 7

compute = ok ->
    value = source ok?
    Outer::Value (value + 1)

main = ->
    match compute true
        Outer::Value value => value.println!
        Outer::Stop code => code.println!
    match compute false
        Outer::Value value => value.println!
        Outer::Stop code => code.println!
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["42", "7"]);
}

#[test]
fn test_try_uses_complete_renamed_language_item_protocol() {
    let exit_code = compile_and_run_without_std(
        r#"
lang control_flow
< enum Flow B, C
    lang break
    Stop B
    lang continue
    Next C

lang try
< trait Carrier
    lang output
    type Value
    lang residual
    type Remainder
    lang branch
    ~@split: Flow Self::Remainder, Self::Value

lang from_residual
< trait Recover R
    lang method
    recover: R -> Self

enum Halt
    Halt I64

enum Maybe T
    Present T
    Absent

impl Carrier for Maybe T
    type Value = T
    type Remainder = Halt

    ~@split = ->
        match self
            Maybe::Present value => Flow::Next value
            Maybe::Absent => Flow::Stop (Halt::Halt 7)

impl Recover Halt for Maybe T
    recover = residual ->
        match residual
            Halt::Halt _ => Maybe::Absent

next: Bool -> Maybe I64
next = ok ->
    if ok
        Maybe::Present 42
    else
        Maybe::Absent

compute: Bool -> Maybe I64
compute = ok ->
    value = next ok?
    Maybe::Present value

main = ->
    success = compute true
    failure = compute false
    match success
        Maybe::Present value =>
            match failure
                Maybe::Present _ => 1
                Maybe::Absent => value
        Maybe::Absent => 2
"#,
    );

    assert_eq!(exit_code, 42);
}

#[test]
fn test_try_does_not_use_unmarked_conventional_protocol_names() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let source_path = dir.join(format!("no_std_{id}.rk"));
    fs::write(
        &source_path,
        r#"
enum ControlFlow B, C
    Break B
    Continue C

trait Try
    type Output
    type Residual
    ~@branch: ControlFlow Self::Residual, Self::Output

trait FromResidual R
    from_residual: R -> Self

enum Maybe T
    Some T
    None

main = ->
    carrier = Maybe::Some 41
    value = carrier?
    value
"#,
    )
    .unwrap();

    let mut config = test_config(source_path, dir);
    config.extern_artifacts.clear();
    config.no_std = true;
    config.no_prelude = true;
    let diagnostics = rock_lib::compile(&config).expect_err("unmarked protocol must not lower '?'");
    assert!(diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message == "Cannot use '?' because the Try language-item protocol is unavailable"
    }));
}

#[test]
fn test_try_result_error_conversion_uses_from() {
    let output = compile_and_run(
        r#"
enum SmallError
    Bad I64

enum BigError
    Wrapped I64

impl From SmallError for BigError
    from = err ->
        match err
            SmallError::Bad code => BigError::Wrapped (code + 1)

fail: Result I64, SmallError
fail = -> Result::Err (SmallError::Bad 6)

compute: Result I64, BigError
compute = ->
    value = fail!?
    Result::Ok value

main = ->
    result = compute!
    match result
        Result::Ok value => value.println!
        Result::Err err =>
            match err
                BigError::Wrapped code => code.println!
    0
"#,
    );
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_try_result_success_path_unwraps() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Result I64, I64
maybe_value = ok ->
    if ok
        Result::Ok 41
    else
        Result::Err 7

compute: Bool -> Result I64, I64
compute = ok ->
    value = maybe_value ok?
    Result::Ok (value + 1)

main = ->
    result = compute true
    match result
        Result::Ok value => value.println!
        Result::Err err => (err + 0).println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_result_error_path_returns_early() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Result I64, I64
maybe_value = ok ->
    if ok
        Result::Ok 41
    else
        Result::Err 7

compute: Bool -> Result I64, I64
compute = ok ->
    value = maybe_value ok?
    Result::Ok (value + 1)

main = ->
    result = compute false
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_try_in_instance_method_uses_declared_method_return_type() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Result I64, I64
maybe_value = ok ->
    if ok
        Result::Ok 41
    else
        Result::Err 7

struct Worker

impl Worker
    @compute: Bool -> Result I64, I64
    @compute = ok ->
        value = maybe_value ok?
        Result::Ok (value + 1)

main = ->
    worker = Worker
    success = worker.compute true
    failure = worker.compute false
    match success
        Result::Ok value => value.println!
        Result::Err err => err.println!
    match failure
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["42", "7"]);
}

#[test]
fn test_try_in_signature_backed_trait_default_uses_declared_return_type() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Result I64, I64
maybe_value = ok ->
    if ok
        Result::Ok 41
    else
        Result::Err 7

struct Worker

trait Computes
    @compute: Bool -> Result I64, I64
    @compute = ok ->
        value = maybe_value ok?
        Result::Ok (value + 1)

impl Computes for Worker

main = ->
    worker = Worker
    success = worker.compute true
    failure = worker.compute false
    match success
        Result::Ok value => value.println!
        Result::Err err => err.println!
    match failure
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["42", "7"]);
}

#[test]
fn test_try_option_some_path_unwraps() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Option I64
maybe_value = ok ->
    if ok
        Option::Some 41
    else
        Option::None

compute: Bool -> Option I64
compute = ok ->
    value = maybe_value ok?
    Option::Some (value + 1)

main = ->
    result = compute true
    match result
        Option::Some value => value.println!
        Option::None => 0.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_option_none_path_returns_early() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Option I64
maybe_value = ok ->
    if ok
        Option::Some 41
    else
        Option::None

compute: Bool -> Option I64
compute = ok ->
    value = maybe_value ok?
    Option::Some (value + 1)

main = ->
    result = compute false
    match result
        Option::Some value => value.println!
        Option::None => 7.println!
    0
"#,
    );
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_try_non_carrier_reports_diagnostic() {
    compile_should_fail(
        r#"
main = ->
    value = 1?
    value
"#,
        "Cannot use '?' on non-carrier type",
    );
}

#[test]
fn test_signatureless_method_receiver_is_constrained_by_call_site() {
    let output = compile_and_run(
        r#"
run_mode = mode -> mode.as_str!

main = ->
    mode = String::from "listen"
    run_mode &mode .println!
    0
"#,
    );

    assert_eq!(output.trim(), "listen");
}

#[test]
fn test_signatureless_higher_order_helper_is_instantiated_at_two_targets() {
    let output = compile_and_run(
        r#"
apply = value, mapper -> mapper value

increment = value -> value + 1
identity_bool = value ->
    if value
        true
    else
        false

main = ->
    first = apply 41, increment
    second = apply true, identity_bool
    first.println!
    second.println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        vec!["42", "true"]
    );
}

#[test]
fn test_signatureless_curried_partial_application_propagates_nested_return() {
    compile_should_pass(
        r#"
enum Wrapped
    Value I64
    Empty

impl Wrapped
    ~@unwrap_or_zero = ->
        match self
            Wrapped::Value value => value
            Wrapped::Empty => 0

wrap = state, value ~>
    Wrapped::Value (state + value)

apply = value, mapper -> mapper value

main = ->
    wrapped = apply 42, (wrap 0)
    wrapped.unwrap_or_zero!.println!
    0
"#,
    );
}

#[test]
fn test_signatureless_callback_infers_mut_reference_parameter() {
    let output = compile_and_run(
        r#"
dispatch = mut target, callback ->
    callback target
    0

main = ->
    mut value = 0
    dispatch &mut value, (slot -> *slot = 41)
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "41");
}

#[test]
fn test_signatureless_named_callback_receives_call_site_receiver_type() {
    let output = compile_and_run(
        r#"
struct First
struct Second

impl First
    @write_all: &[U8] -> I64
    @write_all = bytes -> ~ArrayLen bytes

impl Second
    @write_all: &[U8] -> I64
    @write_all = _ -> 99

dispatch = mut target, callback ->
    bytes = [1 as U8, 2 as U8]
    callback &mut target, &bytes[..], 0

forward = output, bytes, _ -> output.write_all bytes

main = !->
    dispatch First, forward .println!
"#,
    );
    assert_eq!(output.trim(), "2");
}

#[test]
fn test_signatureless_io_callback_receiver_is_inferred_through_result_loop() {
    let output = compile_and_run(
        r#"
> std::io::Write
> std::io::stdout
> std::io::IoError

struct Source

impl Source
    @recv: &mut [U8] -> Result I64, I64
    @recv = _ -> Result::Ok 0

receive_with = stream, mut target, write ->
    mut buffer = [0; 4]
    mut total = 0
    while true
        match stream.recv &mut buffer
            Result::Ok count =>
                if count <= 0
                    return total
                else
                    match write &mut target, &buffer[..count], count
                        Result::Ok written => total = total + written
                        Result::Err _ => return total
            Result::Err _ => return total
    total

discard = _, _, count ->
    if true
        Result::Ok count
    else
        Result::Err IoError::Os 1

impl Source
    @consume = -> receive_with Source, 0, discard
    @consume_stdout = -> receive_with Source, stdout!, write_stdout

write_stdout = output, bytes, _ -> output.write_all bytes

main = !->
    source = Source
    source.consume!.println!
    source.consume_stdout!.println!
"#,
    );
    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["0", "0"]);
}

#[test]
fn test_signatureless_forward_try_and_hkt_map_are_inferred() {
    let output = compile_and_run(
        r#"
finish_connection = ->
    value = source!?
    wrapped = if true
        Result::Ok value
    else
        Result::Err 0
    wrapped.map (item -> item + 1)

source = ->
    if true
        Result::Ok 41
    else
        Result::Err 0

main = ->
    result = finish_connection!
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_signatureless_chained_hkt_operators_defer_unknown_constructor_heads() {
    let output = compile_and_run(
        r#"
source = ->
    if true
        Result::Ok 41
    else
        Result::Err 0
convert_error = value -> value == 0

finish = ->
    total = 1
    source!
        <!> convert_error
        <&> _ -> total + 41

main = ->
    finish!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "Ok(42)");
}

#[test]
fn test_signatureless_deferred_method_argument_uses_call_parameter_type() {
    let output = compile_and_run(
        r#"
struct Left
struct Right

impl Left
    @make = -> 41

impl Right
    @make = -> true

consume: I64 -> Result I64, I64
consume = value -> Result::Ok value

wrapper = value ->
    result = consume value.make!
    result

main = ->
    wrapper Left .println!
    0
"#,
    );
    assert_eq!(output.trim(), "Ok(41)");
}

#[test]
fn test_signatureless_deferred_method_constrains_argument_from_concrete_parameter() {
    let output = compile_and_run(
        r#"
struct Sink

impl Sink
    @write: I64 -> I64
    @write = value -> value

    @write_bytes: &mut [U8] -> I64
    @write_bytes = _ -> 42

struct Wrapper
    < sink: Sink

impl Wrapper
    @forward = value -> self.sink.write value

    @forward_bytes = ->
        mut bytes = [0; 4]
        self.sink.write_bytes &mut bytes

forward_unknown = sink ->
    mut bytes = [0; 4]
    sink.write_bytes &mut bytes

main = ->
    wrapper = Wrapper
        sink: Sink
    wrapper.forward 41 .println!
    wrapper.forward_bytes!.println!
    forward_unknown Sink .println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        vec!["41", "42", "42"]
    );
}

#[test]
fn test_signatureless_ambiguous_method_still_reports_diagnostic() {
    compile_should_fail(
        r#"
trait First
    @ping: () -> I64

trait Second
    @ping: () -> I64

struct Service

impl First for Service
    @ping = -> 1

impl Second for Service
    @ping = -> 2

run = value -> value.ping!

main = ->
    run Service
    0
"#,
        "Ambiguous selection",
    );
}

#[test]
fn test_signatureless_mutual_recursion_remains_monomorphic() {
    let output = compile_and_run(
        r#"
even = value ->
    if value == 0
        true
    else
        odd (value - 1)

odd = value ->
    if value == 0
        false
    else
        even (value - 1)

main = ->
    even 10 .println!
    odd 10 .println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        vec!["true", "false"]
    );
}

#[test]
fn test_signatureless_direct_recursion_registers_specialization_before_body() {
    let output = compile_and_run(
        r#"
count_down = value ->
    if value == 0
        42
    else
        count_down (value - 1)

main = ->
    count_down 3 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_in_inferred_return_function_is_inferred_successfully() {
    let output = compile_and_run(
        r#"
fallible: () -> Result I64, I64
fallible = -> Result::Ok 41

compute = ->
    value = fallible!?
    if true
        Result::Ok (value + 1)
    else
        Result::Err 0

main = ->
    result = compute!
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_inferred_later_defined_helper_is_materialized_after_body_unification() {
    let output = compile_and_run(
        r#"
compute = ->
    value = later true?
    if true
        Result::Ok (value + 1)
    else
        Result::Err 0

later = ok ->
    if ok
        Result::Ok 41
    else
        Result::Err 7

main = ->
    result = compute!
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_inferred_impl_method_materializes_before_method_generalization() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Result I64, I64
maybe_value = ok ->
    if ok
        Result::Ok 41
    else
        Result::Err 7

struct Worker

impl Worker
    @compute = ok ->
        value = maybe_value ok?
        if true
            Result::Ok (value + 1)
        else
            Result::Err 0

main = ->
    worker = Worker
    result = worker.compute true
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_inferred_function_generalizes_unrelated_parameter() {
    let output = compile_and_run(
        r#"
fallible: () -> Result I64, I64
fallible = -> Result::Ok 41

compute = ignored ->
    value = fallible!?
    if true
        Result::Ok (value + 1)
    else
        Result::Err 0

main = ->
    first = compute 1
    second = compute true
    match first
        Result::Ok value => value.println!
        Result::Err err => err.println!
    match second
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );
    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["42", "42"]);
}

#[test]
fn test_try_deferred_non_carrier_reports_diagnostic() {
    compile_should_fail(
        r#"
compute = ->
    value = later!?
    if true
        Result::Ok value
    else
        Result::Err 0

later = -> 1

main = ->
    compute!
    0
"#,
        "Cannot use '?' on non-carrier type",
    );
}

#[test]
fn test_try_deferred_from_residual_reports_diagnostic() {
    compile_should_fail(
        r#"
maybe: () -> Option I64
maybe = -> Option::None

compute = ->
    value = maybe!?
    if true
        Result::Ok value
    else
        Result::Err 0

main = ->
    compute!
    0
"#,
        "FromResidual",
    );
}

#[test]
fn test_try_deferred_unsafe_branch_requires_unsafe_block() {
    compile_should_fail(
        r#"
enum Carrier T
    Value T
    Stop I64

enum Residual
    Stop I64

impl Try for Carrier T
    type Output = T
    type Residual = Residual

    unsafe ~@branch = ->
        match self
            Carrier::Value value => ControlFlow::Continue value
            Carrier::Stop code => ControlFlow::Break (Residual::Stop code)

impl FromResidual Residual for Carrier T
    from_residual = residual ->
        match residual
            Residual::Stop code => Carrier::Stop code

compute = ->
    value = source!?
    if true
        Carrier::Value value
    else
        Carrier::Stop 0

source: () -> Carrier I64
source = -> Carrier::Value 41

main = ->
    compute!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_try_deferred_unsafe_branch_is_allowed_inside_unsafe_block() {
    let output = compile_and_run(
        r#"
enum Carrier T
    Value T
    Stop I64

enum Residual
    Stop I64

impl Try for Carrier T
    type Output = T
    type Residual = Residual

    unsafe ~@branch = ->
        match self
            Carrier::Value value => ControlFlow::Continue value
            Carrier::Stop code => ControlFlow::Break (Residual::Stop code)

impl FromResidual Residual for Carrier T
    from_residual = residual ->
        match residual
            Residual::Stop code => Carrier::Stop code

compute = ->
    value = unsafe
        source!?
    if true
        Carrier::Value value
    else
        Carrier::Stop 0

source: () -> Carrier I64
source = -> Carrier::Value 41

main = ->
    compute!
    0
"#,
    );
    assert_eq!(output.trim(), "");
}

#[test]
fn test_try_in_lambda_uses_lambda_return_type_not_outer_function() {
    let output = compile_and_run(
        r#"
maybe = -> Option::Some 41

outer: () -> Result I64, I64
outer = ->
    convert = ->
        value = maybe!?
        if true
            Option::Some value
        else
            Option::None
    ignored = convert!
    Result::Ok 42

main = ->
    result = outer!
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_generic_carrier_is_rejected_without_ambiguous_selection() {
    compile_should_fail(
        r#"
compute = value ->
    extracted = value?
    extracted

main = -> 0
"#,
        "carrier type is unresolved or generic",
    );
}

#[test]
fn test_try_option_to_result_without_from_residual_reports_diagnostic() {
    compile_should_fail(
        r#"
maybe: Option I64
maybe = -> Option::None

main: Result I64, I64
main = ->
    value = maybe!?
    Result::Ok value
"#,
        "FromResidual",
    );
}

#[test]
fn test_higher_order_functions() {
    let output = compile_and_run(
        r#"
double = x -> x * 2
triple = x -> x * 3

apply = f, x -> f x

main = ->
    apply double, 5 .println!
    apply triple, 5 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "15");
}

#[test]
fn test_higher_order_function_with_capturing_lambda() {
    let output = compile_and_run(
        r#"
apply = f, x -> f x

main = ->
    base = 10
    apply (y -> y + base), 5 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "15");
}

#[test]
fn test_curried_function_partial_application() {
    let output = compile_and_run(
        r#"
add = a, b ~> a + b

main = ->
    inc = add 1
    inc 2 .println!
    add 1, 2 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "3");
    assert_eq!(lines[1], "3");
}

#[test]
fn test_curried_function_with_explicit_signature() {
    let output = compile_and_run(
        r#"
add: I64 -> I64 -> I64
add = a, b ~> a + b

main = ->
    inc = add 2
    inc 5 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
}

#[test]
fn test_curried_function_try_uses_terminal_return_type() {
    let output = compile_and_run(
        r#"
add_result: I64 -> Result I64, I64 -> Result I64, I64
add_result = base, input ~>
    value = input?
    Result::Ok (base + value)

main = ->
    add_ten = add_result 10
    value = (add_ten (Result::Ok 2)).fold
        _ -> 0
        result -> result
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_zero_arg_method_is_value_until_bang_call() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    @get = -> self.value

main = ->
    counter = Counter
        value: 21
    get = counter.get
    get!.println!
    counter.get!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "21");
    assert_eq!(lines[1], "21");
}

#[test]
fn test_curried_method_partial_application() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    @add = x, y ~> self.value + x + y

main = ->
    counter = Counter
        value: 10
    add_one = counter.add 1
    add_one 2 .println!
    counter.add 1, 2 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "13");
    assert_eq!(lines[1], "13");
}

#[test]
fn test_mut_and_move_self_receivers() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

struct Holder
    < value: I64

impl Counter
    ^@read = -> 41

impl Holder
    ~@consume = -> 7

main = ->
    mut counter = Counter
        value: 41
    holder = Holder
        value: 7
    counter.read!.println!
    holder.consume!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "41");
    assert_eq!(lines[1], "7");
}

#[test]
fn test_recursive_functions() {
    let output = compile_and_run(
        r#"
factorial = n ->
    if n <= 1
        1
    else
        n * (factorial (n - 1))

fib = n ->
    if n <= 1
        n
    else
        (fib (n - 1)) + (fib (n - 2))

main = ->
    factorial 5 .println!
    factorial 10 .println!
    fib 10 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "120");
    assert_eq!(lines[1], "3628800");
    assert_eq!(lines[2], "55");
}

#[test]
fn test_logical_operators() {
    let output = compile_and_run(
        r#"
main = ->
    a = 5
    b = 10
    if a > 3 && b < 20
        1.println!
    else
        0.println!
    if a > 10 || b > 5
        1.println!
    else
        0.println!
    if a > 10 && b > 100
        1.println!
    else
        0.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1"); // 5>3 && 10<20 = true
    assert_eq!(lines[1], "1"); // 5>10 || 10>5 = true
    assert_eq!(lines[2], "0"); // 5>10 && 10>100 = false
}

#[test]
fn test_logical_operators_short_circuit_rhs_bounds_checks() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = ->
    arr = [1]
    if false && arr[1] == 0
        99.println!
    else
        1.println!
    if true || arr[1] == 0
        2.println!
    else
        99.println!
    0
"#,
    );

    assert!(
        success,
        "short-circuited RHS should not evaluate bounds checks"
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_tuples() {
    let output = compile_and_run(
        r#"
swap = a, b -> (b, a)

main = ->
    t = (1, 2, 3)
    t.0.println!
    t.1.println!
    t.2.println!
    s = swap 10, 20
    s.0.println!
    s.1.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "3");
    assert_eq!(lines[3], "20");
    assert_eq!(lines[4], "10");
}

#[test]
fn test_collatz() {
    let output = compile_and_run(
        r#"
collatz_steps = n ->
    steps = 0
    while n > 1
        if n % 2 == 0
            n = n / 2
        else
            n = n * 3 + 1
        steps = steps + 1
    steps

main = ->
    collatz_steps 1 .println!
    collatz_steps 6 .println!
    collatz_steps 27 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "0");
    assert_eq!(lines[1], "8");
    assert_eq!(lines[2], "111");
}

#[test]
fn test_vec_get_returns_optional_reference() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v.push 20

    match (v.get 0)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    match (v.get 5)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["10", "-1"]);
}

#[test]
fn test_vec_get_borrow_blocks_later_mutation() {
    compile_should_fail(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    saved = v.get 0
    v.push 2
    match saved
        Option::Some val => *val .println!
        Option::None => 0.println!
    0
"#,
        "borrow",
    );
}

#[test]
fn vec_deref_does_not_return_reference_to_temporary_slice() {
    compile_should_fail(
        r#"
> std::collections::Vec

bad: Vec I64 -> &I64
bad = v -> &v.len!

main = ->
    mut v = Vec::new!
    v.push 1
    rr = bad v
    0
"#,
        "temporary",
    );
}

#[test]
fn test_vec_push() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    v.push 2
    v.push 3
    v.len! .println!

    match (v.get 0)
        Option::Some val => *val .println!
        Option::None => 0.println!
    match (v.get 1)
        Option::Some val => *val .println!
        Option::None => 0.println!
    match (v.get 2)
        Option::Some val => *val .println!
        Option::None => 0.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "3");
    assert_eq!(lines[1], "1");
    assert_eq!(lines[2], "2");
    assert_eq!(lines[3], "3");
}

#[test]
fn test_std_vec_show_formats_owned_elements() {
    let output = compile_and_run(
        r#"
main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from "alpha")
    values.push (String::from "two words")
    values.println!
    0
"#,
    );

    assert_eq!(output.trim(), "[alpha, two words]");
}

#[test]
fn test_std_vec_rejects_zero_sized_elements_on_push() {
    let (_output, success) = compile_and_run_with_status(
        r#"
struct Empty

main = ->
    mut v = Vec::new!
    v.push Empty
    0
"#,
    );

    assert!(!success, "Vec::push should reject zero-sized elements");
}

#[test]
fn test_std_vec_new_as_slice_uses_non_null_buffer() {
    let output = compile_and_run(
        r#"
main = ->
    v: Vec I64 = Vec::new!
    slice = v.as_slice!
    ((~ArrPtr (*slice)) as I64) != 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_std_vec_push_uses_non_null_buffer() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    slice = v.as_slice!
    ((~ArrPtr (*slice)) as I64) != 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_std_vec_index_works() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    v[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_std_vec_index_out_of_bounds_traps() {
    let (_output, success) = compile_and_run_with_status(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v[1].println!
    0
"#,
    );

    assert!(!success, "Vec index out of bounds should trap");
}

#[test]
fn test_std_vec_shared_binding_index_works() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    shared = v
    shared[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_hash_map_insert_get_len_and_contains() {
    let output = compile_and_run(
        r#"
main = ->
    mut map = HashMap::new!
    map.len! .println!
    map.insert 10, 100
    map.insert 20, 200
    map.len! .println!
    ten = 10
    twenty = 20
    thirty = 30
    map.contains_key &ten .println!
    map.contains_key &thirty .println!

    match (map.get &ten)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    match (map.get &twenty)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    match (map.get &thirty)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["0", "2", "true", "false", "100", "200", "-1"],);
}

#[test]
fn test_std_hash_map_rejects_zero_sized_values_on_insert() {
    let (_output, success) = compile_and_run_with_status(
        r#"
struct Empty

main = ->
    mut map = HashMap::new!
    map.insert 1, Empty
    0
"#,
    );

    assert!(!success, "HashMap::insert should reject zero-sized values");
}

#[test]
fn test_std_hash_map_rejects_zero_sized_keys_on_insert() {
    let (_output, success) = compile_and_run_with_status(
        r#"
struct Empty

impl Eq for Empty

impl PartialEq Empty for Empty
    @== = other -> true
    @!= = other -> false

impl Hash for Empty
    @hash = -> 0

main = ->
    mut map = HashMap::new!
    map.insert Empty, 1
    0
"#,
    );

    assert!(!success, "HashMap::insert should reject zero-sized keys");
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
fn test_default_type_arguments_annotations_and_overrides() {
    let output = compile_and_run(r#"
struct Pair A, {B = A}
    < first: A
    < second: B

type Bytes {T = U8} = Pair T

enum Choice {T = I64}
    Empty
    Value T

sum: Pair I64 -> I64
sum = pair -> pair.first + pair.second

byte: Bytes -> U8
byte = pair -> pair.second

read: Choice -> I64
read = choice -> match choice
    Choice::Empty => 0
    Choice::Value value => value

main = !->
    pair = Pair
        first: 20
        second: 22
    (sum pair).println!
    bytes = Pair
        first: 3 as U8
        second: 7 as U8
    (byte bytes).println!
    mixed: Pair I64, Bool = Pair
        first: 1
        second: true
    mixed.second.println!
    (read (Choice::Value 9)).println!
"#);
    assert_eq!(output.trim(), "42\n7\ntrue\n9");
}

#[test]
fn test_default_type_arguments_inference_and_fallback() {
    let output = compile_and_run(r#"
struct Value {T = Bool}
    < value: T

impl Value T
    new = value -> Value
        value: value

struct Phantom {T = I64}

enum Choice {T = I64}
    Empty
    Value T

use_phantom: Phantom I64 -> I64
use_phantom = value -> 42

identity = value -> value

main = !->
    value = Value
        value: 7
    value.value.println!
    constructed = Value::new 11
    constructed.value.println!
    phantom = Phantom
    (use_phantom phantom).println!
    empty = Choice::Empty
    match empty
        Choice::Empty => 8.println!
        Choice::Value value => value.println!
    through_call = identity Phantom
    (use_phantom through_call).println!
    unconstrained = identity Choice::Empty
    match unconstrained
        Choice::Empty => 12.println!
        Choice::Value value => 13.println!
"#);
    assert_eq!(output.trim(), "7\n11\n42\n8\n42\n12");
}

#[test]
fn test_default_type_arguments_trait_self_and_explicit_rhs() {
    let output = compile_and_run(r#"
trait Compare {Rhs = Self}
    @same: &Rhs -> Bool

impl Compare for I64
    @same = rhs -> *self == *rhs

impl Compare I64 for Bool
    @same = rhs -> *self

same: T -> T -> Bool where T: Compare
same = left, right -> left.same &right

main = !->
    (same 4, 4).println!
    (true.same &4).println!
"#);
    assert_eq!(output.trim(), "true\ntrue");
}

#[test]
fn test_default_type_arguments_recursive_nominal_fields() {
    compile_should_pass(r#"
struct Node {T = I64}
    value: T
    next: *Node

enum Chain {T = I64}
    End
    Link T, *Chain

main = !-> return
"#);
}

#[test]
fn test_default_type_arguments_mutually_recursive_nominal_fields() {
    let first = "struct First {T = I64}\n    value: T\n    other: *Second\n";
    for second in [
        "struct Second {T = Bool}\n    value: T\n    other: *First\n",
        "enum Second {T = Bool}\n    End\n    Link T, *First\n",
    ] {
        for declarations in [format!("{first}\n{second}"), format!("{second}\n{first}")] {
            compile_should_pass(&format!("{declarations}\nmain = !-> return\n"));
        }
    }
}

#[test]
fn test_default_type_arguments_mutual_fields_in_deleted_source_artifacts() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("models.rk"), r#"
< struct First {T = I64}
    < value: T
    < other: *Second

< enum Second {T = Bool}
    End
    Link T, *First
"#).unwrap();
    let entry = dir.join("lib.rk");
    fs::write(&entry, "< mod models\n").unwrap();
    let config = rock_lib::Config {
        entry_file: entry.clone(), output_dir: dir.clone(),
        current_crate_name: Some("recursive".to_string()),
        no_std: true, no_prelude: true, no_link: true,
        ..Default::default()
    };
    let artifact = dir.join("recursive.rkca");
    rock_lib::compile_with_products(&config).unwrap().products.unwrap()
        .write_artifact_to_path(&artifact).unwrap();
    fs::remove_file(dir.join("models.rk")).unwrap();
    fs::remove_file(&entry).unwrap();
    fs::write(&entry, r#"
> recursive::models::First
> recursive::models::Second

read: First -> I64
read = value -> value.value

main = ->
    second: Second = Second::End
    first = First
        value: 7
        other: &second as *Second
    read first
"#).unwrap();
    rock_lib::compile(&rock_lib::Config {
        entry_file: entry, current_crate_name: None,
        extern_artifacts: vec![("recursive".to_string(), artifact)],
        ..config
    }).expect("mutual defaults must survive publication and remapping");
}

#[test]
fn test_default_type_arguments_mutual_fields_across_file_modules() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("first.rk"), r#"
> second::Second
type Seed = I64
< struct First {T = Seed}
    < value: T
    < tag: Seed
    < other: *Second
"#).unwrap();
    fs::write(dir.join("second.rk"), r#"
> first::First
type Seed = Bool
< enum Second {T = Seed}
    End
    Link T, Seed, *First
"#).unwrap();
    let entry = dir.join("main.rk");
    for modules in ["mod first\nmod second", "mod second\nmod first"] {
        fs::write(&entry, format!(r#"
{modules}
> first::First
> second::Second
read: First -> I64
read = value -> value.tag

main = ->
    other: Second = Second::End
    value: First = First
        value: 7
        tag: 8
        other: &other as *Second
    read value
"#)).unwrap();
        rock_lib::compile(&rock_lib::Config {
            entry_file: entry.clone(), output_dir: dir.clone(),
            no_std: true, no_prelude: true, no_link: true,
            ..Default::default()
        }).expect("mutual field defaults and aliases must keep their module scope");
    }
}

#[test]
fn test_default_type_arguments_static_factory_fallback_and_inference() {
    assert_eq!(compile_and_run(r#"
struct Phantom {T = I64}

impl Phantom T
    new: () -> Phantom T
    new = -> Phantom

struct Value {T = Bool}
    < value: T

impl Value T
    new: T -> Value T
    new = value -> Value
        value: value

main = !->
    phantom = Phantom::new!
    explicit: Phantom Bool = Phantom::new!
    value = Value::new 42
    value.value.println!
"#), "42\n");
}

#[test]
fn test_default_type_arguments_forward_declarations_in_file_modules() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("models.rk"), r#"
read: Outer -> I64
< read = value -> value.value.value

< struct Outer {T = Inner}
    < value: T

< struct Inner {T = I64}
    < value: T
"#).unwrap();
    fs::write(dir.join("test.rk"), "mod models\nmain = !-> return\n").unwrap();
    rock_lib::compile(&test_config(dir.join("test.rk"), dir.clone())).expect("Compilation failed");
}

#[test]
fn test_default_type_arguments_reject_invalid_defaults() {
    for (source, expected) in [
        ("struct Bad {T = Missing}\nmain = !->\n    ()\n", "unknown type 'Missing' in generic default"),
        ("struct Bad {T = T}\nmain = !->\n    ()\n", "default cannot reference current or later parameter 'T'"),
        ("struct Bad {T = U}, {U = I64}\nmain = !->\n    ()\n", "default cannot reference current or later parameter 'U'"),
        ("struct Bad {T = I64}, U\nmain = !->\n    ()\n", "required parameters must precede defaulted parameters"),
        ("struct Bad {F _ = I64}\nmain = !->\n    ()\n", "has kind Type, expected Type -> Type"),
        ("type Bad {T = Bad} = T\nmain = !->\n    ()\n", "invalid or cyclic type in generic default"),
        ("type First {T = Second} = T\ntype Second {T = First} = T\nmain = !->\n    ()\n", "invalid or cyclic type in generic default"),
        ("struct First {T = Second}\nstruct Second {T = First}\nmain = !-> return\n", "default"),
        ("struct Bad {T = Self}\nmain = !->\n    ()\n", "unknown type 'Self' in generic default"),
    ] {
        compile_should_fail(source, expected);
    }
}

#[test]
fn test_default_type_arguments_declaration_order_and_constructor_kinds() {
    let output = compile_and_run(r#"
read: Outer -> I64
read = value -> value.value.value

struct Outer {T = Inner}
    < value: T

struct Inner {T = I64}
    < value: T

struct Chain A, {B = A}, {C = B}
    < value: C

struct Hold T, {F _ = Option}
    < value: F T

struct Apply (F _), T
    < value: F T

chain: Chain I64 -> I64
chain = value -> value.value

option: Hold I64 -> I64
option = value -> match value.value
    Option::Some item => item
    Option::None => 0

explicit_constructor: Apply Inner, I64 -> I64
explicit_constructor = value -> value.value.value

main = !->
    inner = Inner
        value: 42
    outer = Outer
        value: inner
    (read outer).println!
    chained = Chain
        value: 9
    (chain chained).println!
    held = Hold
        value: Option::Some 7
    (option held).println!
    applied = Apply
        value: Inner
            value: 42
    (explicit_constructor applied).println!
"#);
    assert_eq!(output.trim(), "42\n9\n7\n42");
}

#[test]
fn test_default_type_arguments_use_declaration_module_scope() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("models.rk"), r#"
struct Seed
    < value: I64

< struct Envelope {T = Seed}
    < value: T

< make = -> Envelope
    value: Seed
        value: 42
"#).unwrap();
    fs::write(dir.join("test.rk"), r#"
struct Holder {T = Envelope}
    < value: T

struct Seed
    value: Bool

> models::Envelope
> models::make

mod models

read: Holder -> I64
read = value -> value.value.value.value

main = !->
    holder = Holder
        value: make!
    (read holder).println!
"#).unwrap();
    rock_lib::compile(&test_config(dir.join("test.rk"), dir.clone())).expect("Compilation failed");
    let output = run_test_command(&mut Command::new(dir.join("test")));
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
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
fn test_hash_map_insert_overwrites_without_growing_len() {
    let output = compile_and_run(
        r#"
main = ->
    mut map = HashMap::new!
    map.insert 1, 10
    map.insert 1, 99
    map.len! .println!
    one = 1

    match (map.get &one)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "99"]);
}

#[test]
fn test_hash_map_handles_collisions_and_growth() {
    let output = compile_and_run(
        r#"
main = ->
    mut map = HashMap::new!
    map.insert 1, 10
    map.insert 9, 90
    map.insert 17, 170
    map.insert 25, 250
    map.insert 33, 330
    map.insert 41, 410
    map.insert 49, 490

    map.len! .println!
    one = 1
    nine = 9
    forty_nine = 49

    match (map.get &one)
        Option::Some val => *val .println!
        Option::None => -1 .println!
    match (map.get &nine)
        Option::Some val => *val .println!
        Option::None => -1 .println!
    match (map.get &forty_nine)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["7", "10", "90", "490"]);
}

#[test]
fn test_hash_map_distinguishes_unequal_keys_with_same_hash() {
    let output = compile_and_run(
        r#"
struct BadKey
    < id: I64

impl Eq for BadKey

impl PartialEq BadKey for BadKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for BadKey
    @hash = -> 1

main = ->
    mut map = HashMap::new!
    map.insert (BadKey
        id: 1), 10
    map.insert (BadKey
        id: 2), 20
    map.len! .println!

    one = BadKey
        id: 1
    match (map.get &one)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    two = BadKey
        id: 2
    match (map.get &two)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["2", "10", "20"]);
}

#[test]
fn test_hash_map_private_storage_fields_are_rejected() {
    compile_should_fail(
        r#"
main = ->
    map = HashMap::new!
    x = map.raw_len
    0
"#,
        "Field 'raw_len' of struct 'HashMap' is private",
    );
}

#[test]
fn test_hash_map_str_keys() {
    let output = compile_and_run(
        r#"
main = ->
    mut map = HashMap::new!
    map.insert "red", 1
    map.insert "blue", 2
    map.insert "red", 3
    map.len! .println!
    red = "red"
    green = "green"

    match (map.get &red)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    match (map.get &green)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["2", "3", "-1"]);
}

#[test]
fn test_std_hash_map_drops_keys_and_values() {
    let output = compile_and_run(
        r#"
struct TrackedKey
    < id: I64
    < drop_id: I64

impl Eq for TrackedKey

impl PartialEq TrackedKey for TrackedKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for TrackedKey
    @hash = -> (*self).id

impl Drop for TrackedKey
    ~@drop = ->
        self.drop_id.println!
        return

struct TrackedValue
    < id: I64

impl Drop for TrackedValue
    ~@drop = ->
        self.id.println!
        return

main = ->
    mut map = HashMap::new!
    map.insert (TrackedKey
        id: 1
        drop_id: 101), (TrackedValue
        id: 10)
    map.insert (TrackedKey
        id: 2
        drop_id: 102), (TrackedValue
        id: 20)
    0
"#,
    );

    let mut lines: Vec<i64> = output
        .trim()
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect();
    lines.sort();
    assert_eq!(lines, vec![10, 20, 101, 102]);
}

#[test]
fn test_std_hash_map_overwrite_drops_old_value_and_unused_key_once() {
    let output = compile_and_run(
        r#"
struct TrackedKey
    < id: I64
    < drop_id: I64

impl Eq for TrackedKey

impl PartialEq TrackedKey for TrackedKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for TrackedKey
    @hash = -> (*self).id

impl Drop for TrackedKey
    ~@drop = ->
        self.drop_id.println!
        return

struct TrackedValue
    < id: I64

impl Drop for TrackedValue
    ~@drop = ->
        self.id.println!
        return

main = ->
    mut map = HashMap::new!
    map.insert (TrackedKey
        id: 1
        drop_id: 101), (TrackedValue
        id: 10)
    map.insert (TrackedKey
        id: 1
        drop_id: 102), (TrackedValue
        id: 20)
    0
"#,
    );

    let lines: Vec<i64> = output
        .trim()
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect();
    assert_eq!(lines, vec![10, 102, 20, 101]);
}

#[test]
fn test_std_hash_map_get_and_contains_borrow_probe_key_without_consuming() {
    let output = compile_and_run(
        r#"
struct TrackedKey
    < id: I64
    < drop_id: I64

impl Eq for TrackedKey

impl PartialEq TrackedKey for TrackedKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for TrackedKey
    @hash = -> (*self).id

impl Drop for TrackedKey
    ~@drop = ->
        self.drop_id.println!
        return

struct TrackedValue
    < id: I64

impl Drop for TrackedValue
    ~@drop = ->
        self.id.println!
        return

main = ->
    mut map = HashMap::new!
    map.insert (TrackedKey
        id: 1
        drop_id: 101), (TrackedValue
        id: 10)

    get_probe = TrackedKey
        id: 1
        drop_id: 201
    match (map.get &get_probe)
        Option::Some val => (*val).id.println!
        Option::None => -1 .println!
    get_probe.id.println!

    contains_probe = TrackedKey
        id: 1
        drop_id: 202
    map.contains_key &contains_probe .println!
    contains_probe.id.println!

    missing_probe = TrackedKey
        id: 2
        drop_id: 203
    match (map.get &missing_probe)
        Option::Some val => (*val).id.println!
        Option::None => -1 .println!
    missing_probe.id.println!

    second_get_probe = TrackedKey
        id: 1
        drop_id: 204
    match (map.get &second_get_probe)
        Option::Some val => (*val).id.println!
        Option::None => -1 .println!
    second_get_probe.id.println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["10", "1", "true", "1", "-1", "2", "10", "1", "204", "203", "202", "201", "10", "101"]
    );
}

#[test]
fn test_std_hash_map_growth_drops_moved_keys_and_values_once() {
    let output = compile_and_run(
        r#"
struct TrackedKey
    < id: I64
    < drop_id: I64

impl Eq for TrackedKey

impl PartialEq TrackedKey for TrackedKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for TrackedKey
    @hash = -> (*self).id

impl Drop for TrackedKey
    ~@drop = ->
        self.drop_id.println!
        return

struct TrackedValue
    < id: I64

impl Drop for TrackedValue
    ~@drop = ->
        self.id.println!
        return

main = ->
    mut map = HashMap::new!
    map.insert (TrackedKey
        id: 1
        drop_id: 101), (TrackedValue
        id: 10)
    map.insert (TrackedKey
        id: 2
        drop_id: 102), (TrackedValue
        id: 20)
    map.insert (TrackedKey
        id: 3
        drop_id: 103), (TrackedValue
        id: 30)
    map.insert (TrackedKey
        id: 4
        drop_id: 104), (TrackedValue
        id: 40)
    map.insert (TrackedKey
        id: 5
        drop_id: 105), (TrackedValue
        id: 50)
    map.insert (TrackedKey
        id: 6
        drop_id: 106), (TrackedValue
        id: 60)
    map.insert (TrackedKey
        id: 7
        drop_id: 107), (TrackedValue
        id: 70)
    0
"#,
    );

    let mut lines: Vec<i64> = output
        .trim()
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        vec![10, 20, 30, 40, 50, 60, 70, 101, 102, 103, 104, 105, 106, 107],
    );
}

#[test]
fn test_vec_as_slice_index_works_without_stored_slice_field() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v.push 20
    slice = v.as_slice!
    slice[1] .println!
    0
"#,
    );

    assert_eq!(output.trim(), "20");
}

#[test]
fn test_vec_as_slice_borrows_existing_buffer() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 11
    v.push 22
    slice = v.as_slice!
    slice[1] .println!
    0
"#,
    );

    assert_eq!(output.trim(), "22");
}

#[test]
fn test_vec_as_slice_borrow_blocks_later_mutation() {
    compile_should_fail(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    slice = v.as_slice!
    v.push 2
    slice[0] .println!
    0
"#,
        "borrow",
    );
}

#[test]
fn test_string_len_method() {
    let output = compile_and_run(
        r#"
main = ->
    s = String::from "hello"
    s.len! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_move_non_copy_value_out_of_shared_reference_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    s = String::from "hello"
    r = &s
    moved = *r
    0
"#,
        "Cannot move non-copy value",
    );
}

#[test]
fn test_borrowed_enum_match_payload_binding_is_shared_reference() {
    let output = compile_and_run(
        r#"
show_option: &Option String -> I64
show_option = opt ->
    match *opt
        Option::Some value => value.len!
        Option::None => 0

main = ->
    value = Option::Some (String::from "hello")
    show_option &value .println!
    show_option &value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5\n5");
}

#[test]
fn test_borrowed_enum_match_payload_field_projection_uses_referent() {
    let output = compile_and_run(
        r#"
struct Boxed
    < text: String

show_boxed: &Option Boxed -> I64
show_boxed = opt ->
    match *opt
        Option::Some value => value.text.len!
        Option::None => 0

main = ->
    value = Option::Some (Boxed
        text: String::from "hello")
    show_boxed &value .println!
    show_boxed &value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5\n5");
}

#[test]
fn test_borrowed_enum_match_reference_payload_binding_uses_payload_type() {
    let output = compile_and_run(
        r#"
read_option_ref: &Option &I64 -> I64
read_option_ref = opt ->
    match *opt
        Option::Some value => *value
        Option::None => 0

main = ->
    number = 42
    value = Option::Some (&number)
    read_option_ref &value .println!
    read_option_ref &value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42\n42");
}

#[test]
fn test_copy_non_cleanup_struct_out_of_shared_reference_is_rejected() {
    compile_should_fail(
        r#"
struct Pair
    < x: I64

main = ->
    p = Pair
        x: 1
    r = &p
    copied = *r
    0
"#,
        "Cannot move non-copy value",
    );
}

#[test]
fn test_vec_raw_len_field_access_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    x = v.raw_len
    0
"#,
        "Field 'raw_len' of struct 'Vec' is private",
    );
}

#[test]
fn test_string_raw_len_field_access_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
main = ->
    s = String::from "hello"
    x = s.raw_len
    0
"#,
        "Field 'raw_len' of struct 'String' is private",
    );
}

#[test]
fn test_generic_function_argument_in_monomorphized_method_call() {
    let output = compile_and_run(
        r#"
identity = x -> x

struct Box T
    value: T

impl Box T
    new = value ->
        Box
            value: value

    @apply = f -> f self.value

main = ->
    b = Box::new 21
    b.apply identity .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "21");
}

#[test]
fn test_generic_instance_call_edges_for_function_and_method() {
    let output = compile_and_run(
        r#"
identity = x -> x

struct Holder T
    value: T

impl Holder T
    new = value ->
        Holder
            value: value

    @apply = f -> f self.value

main = ->
    value = identity 42
    h = Holder::new value
    h.apply identity .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_generic_method_call_preserves_receiver_specialization() {
    let output = compile_and_run(
        r#"
struct Box T
    value: T

impl Box T
    new = value ->
        Box
            value: value

    @get = -> @value

main = ->
    ib = Box::new 41
    bb = Box::new true
    if bb.get!
        ib.get!.println!
    else
        0.println!
    0
"#,
    );

    assert_eq!(output.trim(), "41");
}

fn compile_to_llvm_ir(source: &str) -> String {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join(format!("test_{}.rk", id));
    std::fs::write(&source_path, source).unwrap();
    let mut config = test_config(source_path.clone(), dir.clone());
    config.no_link = true;
    config.emit_llvm = true;
    config.emit_object = Some(dir.join(format!("test_{}.o", id)));

    rock_lib::compile(&config).expect("Compilation failed");

    let module_name = source_path.file_stem().unwrap().to_string_lossy();
    let llvm = std::fs::read_to_string(dir.join(format!("{}.ll", module_name)))
        .expect("Expected LLVM IR output");
    let _ = std::fs::remove_dir_all(&dir);
    llvm
}

fn symbol_fragment_bytes(fragment: &str) -> String {
    let mut mangled = String::new();
    for byte in fragment.as_bytes() {
        mangled.push_str(&format!("{byte:02x}"));
    }
    mangled
}

fn defined_symbol_mentions_fragment(line: &str, fragment: &str) -> bool {
    line.contains(fragment) || line.contains(&symbol_fragment_bytes(fragment))
}

#[test]
fn test_instance_dce_does_not_emit_unused_current_function() {
    let llvm = compile_to_llvm_ir(
        r#"
used = -> 1
unused = -> 2

main = ->
    used! .println!
    0
"#,
    );

    let defined_symbols: Vec<&str> = llvm
        .lines()
        .filter(|line| line.starts_with("define "))
        .collect();
    let used_symbol = "__rock_fn_d0_";
    let unused_symbol = "__rock_fn_d1_";
    assert!(
        defined_symbols
            .iter()
            .any(|line| defined_symbol_mentions_fragment(line, used_symbol)),
        "expected used function definition in LLVM IR: {defined_symbols:?}"
    );
    assert!(
        !defined_symbols
            .iter()
            .any(|line| defined_symbol_mentions_fragment(line, unused_symbol)),
        "unused function should not be defined in LLVM IR: {defined_symbols:?}"
    );
}

#[test]
fn test_instance_dce_does_not_emit_unused_generic_specialization() {
    let llvm = compile_to_llvm_ir(
        r#"
identity = x -> x

use_i64 = -> identity 7
use_i32 = -> identity (8 as I32)

main = ->
    use_i64! .println!
    0
"#,
    );

    let defined_symbols: Vec<&str> = llvm
        .lines()
        .filter(|line| line.starts_with("define "))
        .collect();
    let identity_specializations: Vec<&str> = defined_symbols
        .iter()
        .copied()
        .filter(|line| defined_symbol_mentions_fragment(line, "__rock_fn_d0_"))
        .collect();
    assert_eq!(
        identity_specializations.len(),
        1,
        "only the reachable identity specialization should be defined: {defined_symbols:?}"
    );
    let unused_caller_symbol = "__rock_fn_d2_";
    assert!(
        !defined_symbols
            .iter()
            .any(|line| defined_symbol_mentions_fragment(line, unused_caller_symbol)),
        "unused caller should not be defined in LLVM IR: {defined_symbols:?}"
    );
}

#[test]
fn test_generic_function_argument_app_ir_does_not_define_std_object_symbols() {
    let llvm = compile_to_llvm_ir(
        r#"
identity = x -> x

struct Box T
    value: T

impl Box T
    new = value ->
        Box
            value: value

    @apply = f -> f self.value

main = ->
    b = Box::new 21
    b.apply identity .println!
    0
"#,
    );
    let offending_defs: Vec<&str> = llvm
        .lines()
        .filter(|line| line.starts_with("define ") && line.contains("@std__String_none_new"))
        .collect();
    assert!(
        offending_defs.is_empty(),
        "app IR unexpectedly defines std object-backed symbols: {:?}",
        offending_defs
    );
}

#[test]
fn test_array_print() {
    let output = compile_and_run(
        r#"
main = ->
    // Test 1: Simple integer array
    arr = [1, 2, 3, 4, 5]
    arr.println!

    // Test 2: Array with different values
    numbers = [10, 20, 30]
    numbers.println!

    // Test 3: Empty array
    empty: [I64; 0] = []
    empty.println!

    // Test 4: Single element
    single = [42]
    single.println!

    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "[1, 2, 3, 4, 5]");
    assert_eq!(lines[1], "[10, 20, 30]");
    assert_eq!(lines[2], "[]");
    assert_eq!(lines[3], "[42]");
}

#[test]
fn test_int_to_string() {
    let output = compile_and_run(
        r#"
main = ->
    s = int_to_string 42
    s.println!
    num_str = int_to_string 100
    result = (String::from "Value: ").concat (num_str.clone!)
    result.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "Value: 100");
}

#[test]
fn test_std_conversions_return_owned_strings() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = ->
    i = int_to_string 12345
    i.println!
    i.len!.println!

    neg = int_to_string (-7)
    neg.println!
    neg.len!.println!

    f = float_to_string 3.5
    f.println!
    f.len! > 0 .println!

    string_to_int "42" .println!
    string_to_float "2.5" .println!
    0
"#,
    );
    assert!(success, "program failed with stdout: {output:?}");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12345");
    assert_eq!(lines[1], "5");
    assert_eq!(lines[2], "-7");
    assert_eq!(lines[3], "2");
    assert_eq!(lines[4], "3.5");
    assert_eq!(lines[5], "true");
    assert_eq!(lines[6], "42");
    assert_eq!(lines[7], "2.5");
}

#[test]
fn test_std_string_helpers_return_owned_values() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = ->
    joined: String = string_concat "Rock", "Lang"
    joined.println!
    joined.len!.println!

    bytes = [82 as U8, 111 as U8, 99 as U8, 107 as U8]
    slice_in = &bytes
    sub: Vec U8 = byte_substr slice_in, 1, 2
    sub.len!.println!
    slice = sub.as_slice!
    slice[0] as I64 .println!
    slice[1] as I64 .println!
    0
"#,
    );
    assert!(success, "program failed with stdout: {output:?}");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["RockLang", "8", "2", "111", "99"]);
}

#[test]
fn test_std_string_add_operators() {
    let src = r#"
main = ->
    suffix = String::from "!"
    prefix = "Hello, " + "Rock"
    a = prefix + " language"
    b = "Greeting: " + a
    c = b + suffix
    c.println!
    0
"#;

    let output = compile_and_run(src);
    assert_eq!(output, "Greeting: Hello, Rock language!\n");
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

#[test]
fn test_std_repeated_primitive_show_println_drops_temporaries() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = ->
    i = 0
    while i < 5
        (100 + i).println!
        (i as F64).println!
        true.println!
        'x'.println!
        i = i + 1
    0
"#,
    );
    assert!(success, "program failed with stdout: {output:?}");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 20);
    assert_eq!(lines[0], "100");
    assert_eq!(lines[1], "0");
    assert_eq!(lines[2], "true");
    assert_eq!(lines[3], "x");
    assert_eq!(lines[16], "104");
    assert_eq!(lines[18], "true");
    assert_eq!(lines[19], "x");
}

#[test]
fn test_struct_method_with_struct_param() {
    let output = compile_and_run(
        r#"
struct Vec2
    < x: I64
    < y: I64

impl Vec2
    @add = other ->
        Vec2
            x: @x + other.x
            y: @y + other.y
    @dot = other -> @x * other.x + @y * other.y
    @magnitude_sq = -> @x * @x + @y * @y

main = ->
    v1 = Vec2
        x: 3
        y: 4
    v2 = Vec2
        x: 1
        y: 2
    v2_for_dot = Vec2
        x: 1
        y: 2
    v3 = v1.add v2
    v3.x.println!
    v3.y.println!
    v1.dot v2_for_dot .println!
    v1.magnitude_sq!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "4");
    assert_eq!(lines[1], "6");
    assert_eq!(lines[2], "11");
    assert_eq!(lines[3], "25");
}

#[test]
fn test_string_operations_advanced() {
    let output = compile_and_run(
        r#"
repeat_str = s, n ->
    result = String::from ""
    i = 0
    while i < n
        result = result.concat (String::from s)
        i = i + 1
    result

main = ->
    stars = repeat_str "*", 5
    stars.println!
    stars.len!.println!
    num_str = int_to_string 42
    msg = (String::from "Count: ").concat (num_str.clone!)
    msg.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "*****");
    assert_eq!(lines[1], "5");
    assert_eq!(lines[2], "Count: 42");
}

#[test]
fn test_showcase_example() {
    let output = compile_example("showcase");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 14);
    assert_eq!(lines[0], "4"); // v3.x = 3+1
    assert_eq!(lines[1], "6"); // v3.y = 4+2
    assert_eq!(lines[2], "11"); // v1.dot(v2) = 3*1+4*2
    assert_eq!(lines[3], "25"); // magnitude_sq = 9+16
    assert_eq!(lines[4], "75"); // circle area = 5*5*3
    assert_eq!(lines[5], "16"); // square area = 4*4
    assert_eq!(lines[6], "3628800"); // factorial(10)
    assert_eq!(lines[7], "1"); // is_prime(97)
    assert_eq!(lines[8], "150"); // sum_array
    assert_eq!(lines[9], "5"); // array_len
    assert_eq!(lines[10], "Result: 11"); // string concat
    assert_eq!(lines[11], "42"); // abs(-42)
    assert_eq!(lines[12], "5"); // min(5,10)
    assert_eq!(lines[13], "10"); // max(5,10)
}

#[test]
fn test_builtin_functions() {
    let output = compile_and_run(
        r#"
main = ->
    abs (0 - 5) .println!
    abs 3 .println!
    min 10, 20 .println!
    max 10, 20 .println!
    min (0 - 5), 3 .println!
    max (0 - 5), 3 .println!
    s = float_to_string 3.14
    s.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "3");
    assert_eq!(lines[2], "10");
    assert_eq!(lines[3], "20");
    assert_eq!(lines[4], "-5");
    assert_eq!(lines[5], "3");
    assert_eq!(lines[6], "3.14");
}

#[test]
fn test_string_builtins() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    s = "Hello, World!"
    s.len!.println!
    string_find s, "World" .println!
    string_find s, "xyz" .println!
    string_contains s, "Hello" .println!
    string_contains s, "xyz" .println!
    buf = malloc 8
    unsafe
        *buf = 97
        *(~PtrOffset buf, 1) = 98
        *(~PtrOffset buf, 2) = 0
        *(~PtrOffset buf, 3) = 99
        *(~PtrOffset buf, 4) = 100
        *(~PtrOffset buf, 5) = 101
        *(~PtrOffset buf, 6) = 102
        *(~PtrOffset buf, 7) = 0
    bounded = unsafe str_from_utf8_unchecked (slice_from_raw_parts buf, 7)
    string_find bounded, "def" .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "13");
    assert_eq!(lines[1], "7");
    assert_eq!(lines[2], "-1");
    assert_eq!(lines[3], "1");
    assert_eq!(lines[4], "0");
    assert_eq!(lines[5], "4");
}

#[test]
fn test_closures() {
    let output = compile_and_run(
        r#"
main = ->
    x = 10
    add_x = a -> a + x
    add_x 5 .println!
    add_x 20 .println!
    mul = a, b -> a * b
    mul 3, 7 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "15"); // 5 + 10
    assert_eq!(lines[1], "30"); // 20 + 10
    assert_eq!(lines[2], "21"); // 3 * 7
}

#[test]
fn test_nested_structs() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    < y: I64

struct Rect
    < origin: Point
    < width: I64
    < height: I64

impl Rect
    @area = -> @width * @height

main = ->
    r = Rect
        origin: Point
            x: 10
            y: 20
        width: 100
        height: 50
    r.area! .println!
    r.origin.x.println!
    r.origin.y.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5000");
    assert_eq!(lines[1], "10");
    assert_eq!(lines[2], "20");
}

#[test]
fn test_method_chaining() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    @inc = ->
        Counter
            value: @value + 1
    @dec = ->
        Counter
            value: @value - 1
    @get = -> @value

main = ->
    c = Counter
        value: 0
    c = c.inc!
    c = c.inc!
    c = c.inc!
    c = c.dec!
    c.get! .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "2");
}

#[test]
fn test_enum_unwrap() {
    let output = compile_and_run(
        r#"
enum Option
    Some I64
    None

unwrap_or = opt, default ->
    match opt
        Option::Some val => val
        Option::None => default

main = ->
    a = Option::Some 42
    b = Option::None
    unwrap_or a, 0 .println!
    unwrap_or b, 99 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "99");
}

#[test]
fn test_string_equality() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    a = "hello"
    b = "hello"
    c = "world"
    if a == b
        1.println!
    else
        0.println!
    if a == c
        1.println!
    else
        0.println!
    left_buf = malloc 5
    right_buf = malloc 5
    unsafe
        *left_buf = 97
        *(~PtrOffset left_buf, 1) = 98
        *(~PtrOffset left_buf, 2) = 0
        *(~PtrOffset left_buf, 3) = 99
        *(~PtrOffset left_buf, 4) = 0
        *right_buf = 97
        *(~PtrOffset right_buf, 1) = 98
        *(~PtrOffset right_buf, 2) = 0
        *(~PtrOffset right_buf, 3) = 100
        *(~PtrOffset right_buf, 4) = 0
    left = unsafe str_from_utf8_unchecked (slice_from_raw_parts left_buf, 4)
    right = unsafe str_from_utf8_unchecked (slice_from_raw_parts right_buf, 4)
    if left == right
        1.println!
    else
        0.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1"); // "hello" == "hello"
    assert_eq!(lines[1], "0"); // "hello" != "world"
    assert_eq!(lines[2], "0"); // "ab\0c" != "ab\0d"
}

#[test]
fn test_float_arithmetic() {
    let output = compile_and_run(
        r#"
main = ->
    x = 3.14
    y = 2.0
    x + y .println!
    x * y .println!
    x - y .println!
    x / y .println!
    sqrt x .println!
    to_int x .println!
    z = to_float 42
    z.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5.14");
    assert_eq!(lines[1], "6.28");
    assert_eq!(lines[2], "1.14");
    assert_eq!(lines[3], "1.57");
    assert!(lines[4].starts_with("1.772")); // sqrt(3.14)
    assert_eq!(lines[5], "3"); // to_int(3.14) truncates
    assert_eq!(lines[6], "42"); // to_float(42) prints as 42 with %g
}

#[test]
fn test_map_array() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [1, 2, 3, 4, 5]
    i = 0
    while i < (~ArrayLen arr)
        arr[i] * 2 .println!
        i = i + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "2");
    assert_eq!(lines[1], "4");
    assert_eq!(lines[2], "6");
    assert_eq!(lines[3], "8");
    assert_eq!(lines[4], "10");
}

#[test]
fn test_for_loop_sum() {
    let output = compile_and_run(
        r#"
main = ->
    total = 0
    for x in [10, 20, 30, 40, 50]
        total = total + x
    total.println!
    0
"#,
    );
    assert_eq!(output.trim(), "150");
}

#[test]
fn test_string_parsing() {
    let output = compile_and_run(
        r#"
main = ->
    n = string_to_int "12345"
    n.println!
    n + 1 .println!
    f = string_to_float "3.14"
    f.println!
    string_to_int "-42" .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12345");
    assert_eq!(lines[1], "12346");
    assert_eq!(lines[2], "3.14");
    assert_eq!(lines[3], "-42");
}

#[test]
fn test_tuple_destructuring() {
    let output = compile_and_run(
        r#"
swap = a, b -> (b, a)

main = ->
    (x, y) = swap 3, 7
    x.println!
    y.println!
    (a, b, c) = (10, 20, 30)
    a.println!
    b.println!
    c.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "3");
    assert_eq!(lines[2], "10");
    assert_eq!(lines[3], "20");
    assert_eq!(lines[4], "30");
}

#[test]
fn test_matrix_multiply() {
    let output = compile_and_run(
        r#"
struct Matrix
    < a: I64
    < b: I64
    < c: I64
    < d: I64

impl Matrix
    @det = -> @a * @d - @b * @c
    @trace = -> @a + @d
    @mul = other ->
        Matrix
            a: @a * other.a + @b * other.c
            b: @a * other.b + @b * other.d
            c: @c * other.a + @d * other.c
            d: @c * other.b + @d * other.d

main = ->
    m1 = Matrix
        a: 1
        b: 2
        c: 3
        d: 4
    m2 = Matrix
        a: 5
        b: 6
        c: 7
        d: 8
    m1.det! .println!
    m1.trace! .println!
    m3 = m1.mul m2
    m3.a.println!
    m3.b.println!
    m3.c.println!
    m3.d.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "-2"); // det(1,2;3,4) = 1*4-2*3 = -2
    assert_eq!(lines[1], "5"); // trace = 1+4
    assert_eq!(lines[2], "19"); // m3.a = 1*5+2*7
    assert_eq!(lines[3], "22"); // m3.b = 1*6+2*8
    assert_eq!(lines[4], "43"); // m3.c = 3*5+4*7
    assert_eq!(lines[5], "50"); // m3.d = 3*6+4*8
}

#[test]
fn test_bubble_sort_algorithm() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr = [5, 3, 8, 1, 9, 2, 7, 4, 6, 10]
    len = ~ArrayLen arr
    i = 0
    while i < len
        j = 0
        while j < len - 1 - i
            if arr[j] > arr[j + 1]
                temp = arr[j]
                arr[j] = arr[j + 1]
                arr[j + 1] = temp
            j = j + 1
        i = i + 1
    k = 0
    while k < len
        arr[k].println!
        k = k + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    for i in 0..10 {
        assert_eq!(lines[i], (i + 1).to_string());
    }
}

#[test]
fn test_fibonacci() {
    let output = compile_and_run(
        r#"
fib = n ->
    if n <= 1
        n
    else
        (fib (n - 1)) + (fib (n - 2))

main = ->
    i = 0
    while i < 10
        fib i .println!
        i = i + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    let expected = ["0", "1", "1", "2", "3", "5", "8", "13", "21", "34"];
    for (i, exp) in expected.iter().enumerate() {
        assert_eq!(lines[i], *exp);
    }
}

#[test]
fn test_fizzbuzz() {
    let output = compile_and_run(
        r#"
fizzbuzz = n ->
    if n % 15 == 0
        "FizzBuzz".println!
    else if n % 3 == 0
        "Fizz".println!
    else if n % 5 == 0
        "Buzz".println!
    else
        n.println!

main = ->
    i = 1
    while i <= 15
        fizzbuzz i
        i = i + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[2], "Fizz");
    assert_eq!(lines[4], "Buzz");
    assert_eq!(lines[14], "FizzBuzz");
}

#[test]
fn test_string_escape_sequences() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    "Hello\tWorld".println!
    "Line1\nLine2".println!
    "Quote: \"hi\"".println!
    "Backslash: \\".println!
    buf = malloc 6
    unsafe
        *buf = 97
        *(~PtrOffset buf, 1) = 98
        *(~PtrOffset buf, 2) = 0
        *(~PtrOffset buf, 3) = 99
        *(~PtrOffset buf, 4) = 100
        *(~PtrOffset buf, 5) = 0
    bounded = unsafe str_from_utf8_unchecked (slice_from_raw_parts buf, 5)
    bounded.println!
    0
"#,
    );
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines[0], "Hello\tWorld");
    assert_eq!(lines[1], "Line1");
    assert_eq!(lines[2], "Line2");
    assert_eq!(lines[3], "Quote: \"hi\"");
    assert_eq!(lines[4], "Backslash: \\");
    assert_eq!(lines[5].as_bytes(), b"ab\0cd");
}

#[test]
fn test_enum_match_strings() {
    let output = compile_and_run(
        r#"
enum Color
    Red
    Green
    Blue

color_name = c ->
    match c
        Color::Red => "red"
        Color::Green => "green"
        Color::Blue => "blue"

main = ->
    color_name (Color::Red) .println!
    color_name (Color::Green) .println!
    color_name (Color::Blue) .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "red");
    assert_eq!(lines[1], "green");
    assert_eq!(lines[2], "blue");
}

#[test]
fn test_sieve_of_eratosthenes() {
    let output = compile_and_run(
        r#"
main = ->
    limit = 30
    mut sieve = Vec::new!
    i = 0
    while i < limit
        sieve.push 1
        i = i + 1
    sieve.set 0, 0
    sieve.set 1, 0
    i = 2
    while i * i < limit
        is_prime = match (sieve.get i)
            Option::Some val => *val
            Option::None => 0
        if is_prime == 1
            j = i * i
            while j < limit
                sieve.set j, 0
                j = j + i
        i = i + 1
    count = 0
    i = 2
    while i < limit
        is_prime = match (sieve.get i)
            Option::Some val => *val
            Option::None => 0
        if is_prime == 1
            count = count + 1
        i = i + 1
    count.println!
    0
"#,
    );
    assert_eq!(output.trim(), "10"); // primes below 30: 2,3,5,7,11,13,17,19,23,29
}

#[test]
fn test_negative_numbers() {
    let output = compile_and_run(
        r#"
main = ->
    x = -42
    x.println!
    y = 10
    z = -y
    z.println!
    abs (0 - 7) .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "-42");
    assert_eq!(lines[1], "-10");
    assert_eq!(lines[2], "7");
}

#[test]
fn test_doc_casts_negative_branches_and_unit_values() {
    let output = compile_and_run(
        r#"
average = total, count -> total as F64 / count as F64

classify = value ->
    if value <= 0
        -1
    else
        if value == 1
            -2
        else
            value

main = !->
    ()
    average 9, 2 .println!
    classify 0 .println!
    classify 1 .println!
    classify 3 .println!
"#,
    );
    assert_eq!(output.trim(), "4.5\n-1\n-2\n3");
}

#[test]
fn test_gcd_lcm() {
    let output = compile_and_run(
        r#"
gcd = a, b ->
    if b == 0
        a
    else
        gcd b, (a % b)

lcm = a, b -> a / (gcd a, b) * b

main = ->
    gcd 48, 18 .println!
    lcm 12, 18 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "6");
    assert_eq!(lines[1], "36");
}

#[test]
fn test_break_continue() {
    let output = compile_and_run(
        r#"
main = ->
    i = 0
    while i < 100
        if i == 42
            break
        i = i + 1
    i.println!

    count = 0
    j = 0
    while j < 20
        j = j + 1
        if j % 3 == 0
            continue
        count = count + 1
    count.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "14");
}

#[test]
fn test_string_building() {
    let output = compile_and_run(
        r#"
main = ->
    result = String::from ""
    i = 1
    while i <= 5
        if i > 1
            result = result.concat (String::from ", ")
        num_str = int_to_string i
        result = result.concat (num_str.clone!)
        i = i + 1
    result.println!
    0
"#,
    );
    assert_eq!(output.trim(), "1, 2, 3, 4, 5");
}

#[test]
fn test_trait_with_defaults() {
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
        name: 1
    c = Cat
        name: 2
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

// Helper for testing that compilation fails with expected error message
fn compile_should_fail(source: &str, expected_error: &str) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    let source_path = dir.join("test.rk");
    std::fs::write(&source_path, source).unwrap();

    let config = test_config(source_path.clone(), dir.clone());

    let result = rock_lib::compile(&config);
    assert!(
        result.is_err(),
        "Expected compilation to fail but it succeeded"
    );

    // Check that the diagnostics contain the expected error
    if let Err(diagnostics) = result {
        let messages: Vec<String> = diagnostics.0.iter().map(|d| d.message.clone()).collect();
        let has_expected = messages.iter().any(|m| m.contains(expected_error));
        assert!(
            has_expected,
            "Expected error containing '{}' but got: {:?}",
            expected_error, messages
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
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

fn compile_should_fail_with_exact_diagnostic(source: &str, expected_error: &str) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("test.rk");
    fs::write(&source_path, source).unwrap();

    let mut config = test_config(source_path, dir.clone());
    config.extern_artifacts.clear();
    config.no_std = true;
    config.no_prelude = true;
    config.no_link = true;
    let diagnostics = rock_lib::compile(&config).expect_err("expected compilation to fail");
    let messages: Vec<_> = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();

    assert!(
        messages.iter().any(|message| *message == expected_error),
        "expected exact diagnostic {expected_error:?}, got {messages:?}"
    );
    let _ = fs::remove_dir_all(dir);
}

fn compile_should_pass(source: &str) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    let source_path = dir.join("test.rk");
    std::fs::write(&source_path, source).unwrap();

    let mut config = test_config(source_path, dir.clone());
    config.no_link = true;
    rock_lib::compile(&config).expect("Compilation failed");

    let _ = std::fs::remove_dir_all(&dir);
}

fn compile_should_pass_without_std(source: &str) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("test.rk");
    fs::write(&source_path, source).unwrap();

    let mut config = test_config(source_path, dir.clone());
    config.extern_artifacts.clear();
    config.no_std = true;
    config.no_prelude = true;
    config.no_link = true;
    rock_lib::compile(&config).expect("expected compilation to pass");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_integer_literal_defaults_to_i64() {
    compile_should_pass_without_std(
        r#"main = ->
    value = 42
    0
"#,
    );
}

#[test]
fn test_float_literal_defaults_to_f64() {
    compile_should_pass_without_std(
        r#"main = ->
    value = 4.2
    0
"#,
    );
}

#[test]
fn test_unconstrained_local_lambda_is_rejected() {
    compile_should_fail(
        r#"main = ->
    identity = value -> value
    0
"#,
        "ambiguous type",
    );
}

#[test]
fn test_generic_function_remains_polymorphic_under_strict_finalization() {
    compile_should_pass_without_std(
        r#"identity = value -> value

main = ->
    integer: I64 = identity 42
    boolean: Bool = identity true
    0
"#,
    );
}

#[test]
fn test_public_struct_field_access_succeeds_outside_impl() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    < y: I64

main = ->
    p = Point
        x: 10
        y: 20
    p.x.println!
    p.y.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["10", "20"]);
}

#[test]
fn test_private_struct_field_access_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y

main = ->
    p = Point::make 10, 20
    p.y.println!
    0
"#,
        "Field 'y' of struct 'Point' is private",
    );
}

#[test]
fn test_private_struct_field_access_is_rejected_through_inferred_receiver() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y

get_y = p -> p.y

main = ->
    p = Point::make 10, 20
    get_y p .println!
    0
"#,
        "Field 'y' of struct 'Point' is private",
    );
}

#[test]
fn test_private_struct_field_access_succeeds_inside_own_impl() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y
    @sum = -> @x + @y

main = ->
    p = Point::make 10, 20
    p.sum! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "30");
}

#[test]
fn test_private_struct_construction_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    y: I64

main = ->
    p = Point
        x: 10
        y: 20
    p.x.println!
    0
"#,
        "Cannot construct struct 'Point' because it has private fields",
    );
}

#[test]
fn test_private_struct_construction_succeeds_inside_own_impl() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y

main = ->
    p = Point::make 10, 20
    p.x.println!
    0
"#,
    );

    assert_eq!(output.trim(), "10");
}

#[test]
fn test_private_struct_pattern_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y

inspect = p ->
    match p
        Point y: _ => 0

main = ->
    p = Point::make 10, 20
    inspect p .println!
    0
"#,
        "Field 'y' of struct 'Point' is private",
    );
}

#[test]
fn test_private_struct_pattern_succeeds_inside_own_impl() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y
    @sum = ->
        match *self
            Point x: x, y: y => x + y

main = ->
    p = Point::make 10, 20
    p.sum! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "30");
}

#[test]
fn test_struct_pattern_rejects_refutable_field_subpatterns() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    < y: I64

main = ->
    p = Point
        x: 10
        y: 20
    match p
        Point x: 10, y: y => y
"#,
        "Struct field patterns currently only support bindings and wildcards",
    );
}

#[test]
fn test_struct_pattern_rejects_mismatched_struct_type() {
    compile_should_fail(
        r#"
struct Point
    < x: I64

struct Other
    < x: I64

main = ->
    p = Point
        x: 10
    match p
        Other x: x => x
        Point x: x => x + 1
"#,
        "pattern type mismatch",
    );
}

#[test]
fn test_generic_struct_pattern_infers_scrutinee_type() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

unwrap = box ->
    match box
        Box value: value => value + 1

main = ->
    unwrap (Box
        value: 10) .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "11");
}

#[test]
fn test_generic_struct_pattern_binds_non_i64_payload() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

unwrap = box ->
    match box
        Box value: value => value

main = ->
    unwrap (Box
        value: "hello") .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "hello");
}

#[test]
fn test_trait_default_method_substitutes_trait_generic_struct_pattern_types() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

struct Counter
    < seed: I64

trait UnwrapBox T
    @unwrap: Box T -> T
    @unwrap = boxed ->
        match boxed
            Box value: value => value

impl UnwrapBox I64 for Counter

main = ->
    receiver = Counter
        seed: 0
    boxed = Box
        value: 7
    unwrapped = receiver.unwrap boxed
    unwrapped.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
}

#[test]
fn test_all_public_struct_construction_succeeds_outside_impl() {
    compile_should_pass(
        r#"
struct Point
    < x: I64
    < y: I64

main = ->
    p = Point
        x: 10
        y: 20
    p.x.println!
    0
"#,
    );
}

#[test]
fn test_type_error_int_plus_string() {
    compile_should_fail(
        r#"
main = ->
    x = 42 + "hello"
    0
"#,
        "No implementation found for operator '+' on type I64",
    );
}

#[test]
fn test_type_error_float_plus_string() {
    compile_should_fail(
        r#"
main = ->
    x = 3.14 + "world"
    0
"#,
        "No implementation found for operator '+' on type F64",
    );
}

#[test]
fn test_type_error_annotation_mismatch() {
    compile_should_fail(
        r#"
main = ->
    x: I64 = "hello"
    0
"#,
        "Type annotation mismatch",
    );
}

#[test]
fn test_type_error_bool_plus_int() {
    compile_should_fail(
        r#"
main = ->
    x = true + 42
    0
"#,
        "No implementation found for operator '+' on type Bool",
    );
}

#[test]
fn test_explicit_fixed_array_type_syntax_lowers_successfully() {
    let output = compile_and_run(
        r#"
get_first : [I64; 4] -> I64
get_first = arr ->
    arr[0]

main = ->
    arr: [I64; 4] = [10, 20, 30, 40]
    get_first arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "10");
}

#[test]
fn test_mut_fixed_array_borrow_coerces_to_mut_slice_parameter() {
    let output = compile_and_run(
        r#"
write_first: &mut [I64] -> I64
write_first = s ->
    ptr = (~ArrPtr (*s)) as *I64
    unsafe *ptr = 9
    unsafe *ptr

main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    write_first r .println!
    arr[0].println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "9");
    assert_eq!(lines[1], "9");
}

#[test]
fn test_mut_fixed_array_borrow_coerces_to_shared_slice_parameter() {
    let output = compile_and_run(
        r#"
sum: &[I64] -> I64
sum = s ->
    (s[0] + s[1]) + s[2]

main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    sum r .println!
    0
"#,
    );

    assert_eq!(output.trim(), "6");
}

#[test]
fn test_mut_fixed_array_borrow_dispatches_shared_slice_method() {
    let output = compile_and_run(
        r#"
show_mut: &mut [I64] -> String
show_mut = s -> s.show!

main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    show_mut r .println!
    0
"#,
    );

    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_mut_array_borrow_directly_dispatches_shared_slice_method() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    r.println!
    0
"#,
    );

    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_mut_array_borrow_dispatches_custom_shared_slice_method() {
    let output = compile_and_run(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for &[T]
    @slice_len = ->
        ~ArrayLen (*self)

main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    r.slice_len!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_string_concat_builtin() {
    let output = compile_and_run(
        r#"
main = ->
    a = "Hello, "
    b = "World!"
    c = string_concat a, b
    c.println!
    d = (string_concat "foo", "bar").concat (String::from "baz")
    d.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "Hello, World!");
    assert_eq!(lines[1], "foobarbaz");
}

#[test]
fn test_vec_swap_remove_moves_owned_value() {
    let output = compile_and_run(
        r#"
main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from "first")
    values.push (String::from "second")
    values.push (String::from "third")
    match values.swap_remove 1
        Option::Some value => value.println!
        Option::None => "missing".println!
    values.len!.println!
    values[1].println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        ["second", "2", "third"]
    );
}

#[test]
fn test_vec_set_get() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v.push 20
    v.push 30
    v.set 1, 99
    match (v.get 0)
        Option::Some val => *val .println!
        Option::None => 0.println!
    match (v.get 1)
        Option::Some val => *val .println!
        Option::None => 0.println!
    match (v.get 2)
        Option::Some val => *val .println!
        Option::None => 0.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "99");
    assert_eq!(lines[2], "30");
}

#[test]
fn test_std_vec_drops_initialized_elements() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut v = Vec::new!
    v.push (Tracked
        value: 1)
    v.push (Tracked
        value: 2)
    v.push (Tracked
        value: 3)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["3", "2", "1"]);
}

#[test]
fn test_std_vec_set_drops_replaced_element_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut v = Vec::new!
    v.push (Tracked
        value: 1)
    v.push (Tracked
        value: 2)
    v.set 0, (Tracked
        value: 9)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2", "9"]);
}

#[test]
fn test_std_vec_set_out_of_bounds_drops_ignored_value_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut v = Vec::new!
    v.push (Tracked
        value: 1)
    v.set (-1), (Tracked
        value: 8)
    v.set 3, (Tracked
        value: 9)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["8", "9", "1"]);
}

#[test]
fn test_std_vec_growth_drops_initialized_elements_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut v = Vec::new!
    v.push (Tracked
        value: 1)
    v.push (Tracked
        value: 2)
    v.push (Tracked
        value: 3)
    v.push (Tracked
        value: 4)
    v.push (Tracked
        value: 5)
    v.push (Tracked
        value: 6)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["6", "5", "4", "3", "2", "1"]);
}

#[test]
fn test_std_option_methods() {
    let output = compile_and_run(
        r#"
inc = x -> x + 1

to_even = x ->
    if x % 2 == 0
        Option::Some x
    else
        Option::None

main = ->
    some_map = Option::Some 41
    none_map = Option::None
    some_and_then = Option::Some 41
    none_fold = Option::None
    nested = Option::Some (Option::Some 9)
    some_fold = Option::Some 41
    some_ok_or: Result I64, I64 = Option::Some 12 .ok_or 5
    none_ok_or: Result I64, I64 = Option::None .ok_or 7
    some_folded: I64 = some_fold.fold
        0
        x -> x + 1
    none_folded: I64 = none_fold.fold
        7
        x -> x + 1

    (some_map.map inc).unwrap_or 0 .println!
    (none_map.map inc).unwrap_or 0 .println!
    (some_and_then.and_then to_even).unwrap_or 0 .println!
    ((Option::Some 8).and_then to_even).unwrap_or 0 .println!
    (nested.flatten!).unwrap_or 0 .println!
    some_folded.println!
    none_folded.println!
    some_ok_or.fold (x -> x), (x -> x) .println!
    none_ok_or.fold (x -> x), (x -> x) .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["42", "0", "0", "8", "9", "42", "7", "12", "7"]);
}

#[test]
fn test_std_option_inspect_borrows_payload_and_returns_option() {
    let output = compile_and_run(
        r#"
print_ref: &I64 -> I32
print_ref = x -> *x .println!

main = ->
    value = (Option::Some 4).inspect print_ref
    value.unwrap_or 0 .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["4", "4"]);
}

#[test]
fn test_std_option_inspect_owned_string_borrows_and_returns_owned_option() {
    let output = compile_and_run(
        r#"
print_len: &String -> I32
print_len = value -> value.len! .println!

main = ->
    value = Option::Some (String::from "hello")
    returned = value.inspect print_len
    returned.println!
    0
"#,
    );

    assert_eq!(output.trim(), "5\nSome(hello)");
}

#[test]
fn test_std_option_inspect_owned_string_consumes_receiver() {
    compile_should_fail(
        r#"
print_len: &String -> I32
print_len = value -> value.len! .println!

main = ->
    value = Option::Some (String::from "hello")
    returned = value.inspect print_len
    returned.println!
    value.println!
    0
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_std_result_methods() {
    let output = compile_and_run(
        r#"
inc = x -> x + 1
tag = err -> err + 5
id_i64: I64 -> I64
id_i64 = x -> x
inc1 = x -> x + 1

parse = x ->
    if x > 0
        Result::Ok x
    else
        Result::Err 3

main = ->
    ok_map: Result I64, I64 = Result::Ok 41
    err_map: Result I64, I64 = Result::Err 4
    err_map_err: Result I64, I64 = Result::Err 4
    ok_and_then: Result I64, I64 = Result::Ok 41
    err_or: Result I64, I64 = Result::Err 4
    nested_ok: Result (Result I64, I64), I64 = Result::Ok (Result::Ok 9)
    nested_err: Result (Result I64, I64), I64 = Result::Ok (Result::Err 6)
    ok_fold: Result I64, I64 = Result::Ok 41
    err_fold: Result I64, I64 = Result::Err 4
    (ok_map.map inc).unwrap_or 0 .println!
    (err_map.map inc).unwrap_or 0 .println!
    (err_map_err.map_err tag).fold id_i64, id_i64 .println!
    (ok_and_then.and_then parse).unwrap_or 0 .println!
    ((Result::Ok (-1)).and_then parse).unwrap_or 0 .println!
    (err_or.or (Result::Ok 9)).unwrap_or 0 .println!
    (nested_ok.flatten!).unwrap_or 0 .println!
    (nested_err.flatten!).fold id_i64, id_i64 .println!
    (ok_fold.fold id_i64, inc1) .println!
    (err_fold.fold id_i64, id_i64) .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["42", "0", "9", "41", "0", "9", "9", "6", "42", "4"]
    );
}

#[test]
fn test_hkt_functor_option_result_vec() {
    let products =
        rock_lib::products::CompilerProducts::read_artifact_from_path(&std_artifact_path())
            .expect("read std artifact");
    let functional_trait_ids = products
        .interface
        .traits
        .iter()
        .filter_map(|(id, trait_interface)| {
            [
                "Bifunctor",
                "Functor",
                "Applicative",
                "Monad",
                "Foldable",
                "Traversable",
            ]
            .iter()
            .any(|name| trait_interface.name.ends_with(name))
            .then_some(*id)
        })
        .collect::<Vec<_>>();
    assert_eq!(functional_trait_ids.len(), 6);
    let language_items = &products.interface.language_items;
    let language_trait_ids = [
        language_items.sized.as_ref().map(|items| items.trait_id),
        language_items.drop.as_ref().map(|items| items.trait_id),
        language_items.index.as_ref().map(|items| items.trait_id),
        language_items
            .index_mut
            .as_ref()
            .map(|items| items.trait_id),
        language_items.fn_once.as_ref().map(|items| items.trait_id),
        language_items.fn_mut.as_ref().map(|items| items.trait_id),
        language_items.fn_trait.as_ref().map(|items| items.trait_id),
        language_items.send.as_ref().map(|items| items.trait_id),
        language_items.sync.as_ref().map(|items| items.trait_id),
        language_items
            .try_protocol
            .as_ref()
            .map(|items| items.try_trait_id),
        language_items
            .try_protocol
            .as_ref()
            .map(|items| items.from_residual_trait_id),
    ];
    assert!(functional_trait_ids
        .iter()
        .all(|id| !language_trait_ids.contains(&Some(*id))));

    let output = compile_and_run(
        r#"
> std::io::IoError

type IoResult = Result _, IoError

map_any: M -> F A -> F B where F _: Functor, M: FnMut A, B
map_any = mapper, value -> F::Functor::fmap mapper, value

map_result: M -> Result A, E -> Result B, E where M: FnMut A, B
map_result = mapper, value -> (Result _, E)::Functor::fmap mapper, value

inc: I64 -> I64
inc = value -> value + 1

double: I64 -> I64
double = value -> value * 2

make_values: () -> Vec I64
make_values = ->
    mut values = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    values

map_values: Vec I64 -> Vec I64
map_values = values -> map_any double, values

main = ->
    mapped_option: Option I64 = map_any inc, (Option::Some 4)
    mapped_result: Result I64, I64 = map_result inc, (Result::Ok 5)
    mapped_vec: Vec I64 = (make_values!) <&> double
    qualified: Option I64 = Option::Functor::fmap inc, (Option::Some 6)
    fixed_error: Result I64, IoError = (Result _, IoError)::Applicative::pure 8
    alias_error: Result I64, IoError = IoResult::Applicative::pure 9
    nested: Option (Vec I64) = Option::Some (make_values!)
    mapped_nested: Option (Vec I64) = map_any map_values, nested

    mapped_option.println!
    mapped_result.println!
    mapped_vec.println!
    qualified.println!
    fixed_error.println!
    alias_error.println!
    mapped_nested.println!
    0
"#,
    );

    assert_eq!(
        output,
        "Some(5)\nOk(6)\n[2, 4, 6]\nSome(7)\nOk(8)\nOk(9)\nSome([2, 4, 6])\n"
    );
}

#[test]
fn test_hkt_applicative_and_monad_laws() {
    let output = compile_and_run(
        r#"
repure_any: F I64 -> F I64 where F _: Applicative
repure_any = ignored -> F::Applicative::pure 2

ap_any: F (A -> B) -> F A -> F B where F _: Applicative
ap_any = wrapped_function, wrapped_value ->
    F::Applicative::ap wrapped_function, wrapped_value

bind_any: F A -> M -> F B where F _: Monad, M: FnMut A, (F B)
bind_any = value, callback -> F::Monad::bind value, callback

id_i64: I64 -> I64
id_i64 = value -> value

inc: I64 -> I64
inc = value -> value + 1

double: I64 -> I64
double = value -> value * 2

inc_after_double: I64 -> I64
inc_after_double = value -> inc (double value)

apply_five: (I64 -> I64) -> I64
apply_five = function -> function 5

option_add_two: I64 -> Option I64
option_add_two = value -> Option::Some (value + 2)

option_double: I64 -> Option I64
option_double = value -> Option::Some (value * 2)

option_pure: I64 -> Option I64
option_pure = value -> Option::Applicative::pure value

result_add_two: I64 -> Result I64, I64
result_add_two = value -> Result::Ok (value + 2)

result_double: I64 -> Result I64, I64
result_double = value -> Result::Ok (value * 2)

result_pure: I64 -> Result I64, I64
result_pure = value -> (Result _, I64)::Applicative::pure value

main = ->
    generic_pure: Option I64 = repure_any (Option::Some 0)
    generic_function: Option (I64 -> I64) = Option::Some inc
    generic_ap: Option I64 = ap_any generic_function, generic_pure
    generic_bind: Option I64 = bind_any generic_ap, option_add_two

    option_value: Option I64 = Option::Some 3
    option_identity_function: Option (I64 -> I64) = Option::Applicative::pure id_i64
    option_identity_l: Option I64 = Option::Applicative::ap option_identity_function, option_value
    option_identity_r: Option I64 = Option::Some 3

    option_composition_l: Option I64 = Option::Applicative::ap (Option::Some inc_after_double), (Option::Some 3)
    option_composition_inner: Option I64 = Option::Applicative::ap (Option::Some double), (Option::Some 3)
    option_composition_r: Option I64 = Option::Applicative::ap (Option::Some inc), option_composition_inner

    option_inc: Option (I64 -> I64) = Option::Applicative::pure inc
    option_four: Option I64 = Option::Applicative::pure 4
    option_homomorphism_l: Option I64 = Option::Applicative::ap option_inc, option_four
    option_homomorphism_r: Option I64 = Option::Applicative::pure (inc 4)
    option_five: Option I64 = Option::Applicative::pure 5
    option_interchange_l: Option I64 = Option::Applicative::ap (Option::Some inc), option_five
    option_apply_five: Option ((I64 -> I64) -> I64) = Option::Applicative::pure apply_five
    option_interchange_r: Option I64 = Option::Applicative::ap option_apply_five, (Option::Some inc)

    option_six: Option I64 = Option::Applicative::pure 6
    option_left_identity_l: Option I64 = Option::Monad::bind option_six, option_add_two
    option_left_identity_r: Option I64 = option_add_two 6
    option_right_identity_l: Option I64 = Option::Monad::bind (Option::Some 7), option_pure
    option_right_identity_r: Option I64 = Option::Some 7
    option_associativity_inner: Option I64 = Option::Monad::bind (Option::Some 8), option_add_two
    option_associativity_l: Option I64 = Option::Monad::bind option_associativity_inner, option_double
    option_associativity_r: Option I64 = Option::Monad::bind (Option::Some 8), (value -> Option::Monad::bind (option_add_two value), option_double)

    result_value: Result I64, I64 = Result::Ok 3
    result_identity_function: Result (I64 -> I64), I64 = (Result _, I64)::Applicative::pure id_i64
    result_identity_l: Result I64, I64 = (Result _, I64)::Applicative::ap result_identity_function, result_value
    result_identity_r: Result I64, I64 = Result::Ok 3

    result_composition_l: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok inc_after_double), (Result::Ok 3)
    result_composition_inner: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok double), (Result::Ok 3)
    result_composition_r: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok inc), result_composition_inner

    result_homomorphism_l: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok inc), (Result::Ok 4)
    result_homomorphism_r: Result I64, I64 = Result::Ok (inc 4)
    result_interchange_l: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok inc), (Result::Ok 5)
    result_interchange_r: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok apply_five), (Result::Ok inc)

    result_left_identity_l: Result I64, I64 = (Result _, I64)::Monad::bind (Result::Ok 6), result_add_two
    result_left_identity_r: Result I64, I64 = result_add_two 6
    result_right_identity_l: Result I64, I64 = (Result _, I64)::Monad::bind (Result::Ok 7), result_pure
    result_right_identity_r: Result I64, I64 = Result::Ok 7
    result_associativity_l: Result I64, I64 = (Result _, I64)::Monad::bind ((Result _, I64)::Monad::bind (Result::Ok 8), result_add_two), result_double
    result_associativity_r: Result I64, I64 = (Result _, I64)::Monad::bind (Result::Ok 8), (value -> (Result _, I64)::Monad::bind (result_add_two value), result_double)

    first_error: Result (I64 -> I64), I64 = Result::Err 1
    second_error: Result I64, I64 = Result::Err 2
    result_error_precedence: Result I64, I64 = (Result _, I64)::Applicative::ap first_error, second_error

    generic_bind.println!
    option_identity_l.println!
    option_identity_r.println!
    option_composition_l.println!
    option_composition_r.println!
    option_homomorphism_l.println!
    option_homomorphism_r.println!
    option_interchange_l.println!
    option_interchange_r.println!
    option_left_identity_l.println!
    option_left_identity_r.println!
    option_right_identity_l.println!
    option_right_identity_r.println!
    option_associativity_l.println!
    option_associativity_r.println!
    result_identity_l.println!
    result_identity_r.println!
    result_composition_l.println!
    result_composition_r.println!
    result_homomorphism_l.println!
    result_homomorphism_r.println!
    result_interchange_l.println!
    result_interchange_r.println!
    result_left_identity_l.println!
    result_left_identity_r.println!
    result_right_identity_l.println!
    result_right_identity_r.println!
    result_associativity_l.println!
    result_associativity_r.println!
    result_error_precedence.println!
    0
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "Some(5)", "Some(3)", "Some(3)", "Some(7)", "Some(7)", "Some(5)", "Some(5)", "Some(6)",
            "Some(6)", "Some(8)", "Some(8)", "Some(7)", "Some(7)", "Some(20)", "Some(20)", "Ok(3)",
            "Ok(3)", "Ok(7)", "Ok(7)", "Ok(5)", "Ok(5)", "Ok(6)", "Ok(6)", "Ok(8)", "Ok(8)",
            "Ok(7)", "Ok(7)", "Ok(20)", "Ok(20)", "Err(1)",
        ]
    );
}

#[test]
fn test_hkt_standalone_and_receiver_operations() {
    let output = compile_and_run(
        r#"
increment: I64 -> I64
increment = value -> value + 1

some_increment: I64 -> Option I64
some_increment = value -> pure (value + 1)

digits: (I64, I64) -> I64
digits = pair -> pair.0 * 10 + pair.1

values: () -> Vec I64
values = ->
    mut result = Vec::new!
    result.push 1
    result.push 2
    result

effects: () -> Vec (Option I64)
effects = -> fmap some_increment, values!

map_generic: M -> F A -> F B where F _: Functor, M: FnMut A, B
map_generic = mapper, value -> value.fmap mapper

result_value: () -> Result I64, I64
result_value = -> pure 4

main = !->
    (fmap increment, Option::Some 4).println!
    (Option::Some 4).fmap increment .println!
    (map_generic increment, result_value!).println!
    (map_generic increment, values!).println!
    (ap (Option::Some increment), (Option::Some 4)).println!
    (Option::Some increment).ap (Option::Some 4) .println!
    (bind (Option::Some 4), some_increment).println!
    (Option::Some 4).bind some_increment .println!
    (foldl digits, 0, values!).println!
    values!.foldl digits, 0 .println!
    (traverse some_increment, values!).println!
    values!.traverse some_increment .println!
    (traverse_m some_increment, values!).println!
    values!.traverse_m some_increment .println!
    (sequence effects!).println!
    effects!.sequence!.println!
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "Some(5)",
            "Some(5)",
            "Ok(5)",
            "[2, 3]",
            "Some(5)",
            "Some(5)",
            "Some(5)",
            "Some(5)",
            "12",
            "12",
            "Some([2, 3])",
            "Some([2, 3])",
            "Some([2, 3])",
            "Some([2, 3])",
            "Some([2, 3])",
            "Some([2, 3])",
        ]
    );
}

#[test]
fn test_hkt_receiver_custom_carrier_and_owned_mutable_callbacks() {
    let output = compile_and_run(
        r#"
enum Parcel T
    Item T

impl Functor for Parcel
    fmap = mut mapper, value ->
        match value
            Parcel::Item item => Parcel::Item (mapper item)

string_length: String -> I64
string_length = value -> value.len!

main = !->
    parcel = Parcel::Item (String::from "hello")
    match parcel.fmap string_length
        Parcel::Item length => length.println!
    match fmap string_length, Parcel::Item (String::from "world")
        Parcel::Item length => length.println!

    mut total = 0
    mut values = Vec::new!
    values.push 1
    values.push 2
    mapped = values.fmap (value ->
        total = total + value
        total)
    mapped.println!
    total.println!

    mut words = Vec::new!
    words.push (String::from "one")
    words.push (String::from "four")
    words.fmap string_length .println!
"#,
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["5", "5", "[1, 3]", "3", "[3, 4]"]
    );
}

#[test]
fn test_hkt_receiver_failure_paths_and_constructor_sections() {
    let output = compile_and_run(
        r#"
increment: I64 -> I64
increment = value -> value + 1

success: I64 -> Result I64, String
success = value -> pure value

some: I64 -> Option I64
some = value -> pure value

stop: I64 -> Result I64, String
stop = value ->
    value.println!
    if value == 2
        Result::Err (String::from "stop")
    else
        pure (value + 1)

values: () -> Vec I64
values = ->
    mut items = Vec::new!
    items.push 1
    items.push 2
    items.push 3
    items

bind_generic: F A -> M -> F B where F _: Monad, M: FnMut A, (F B)
bind_generic = value, callback -> value.bind callback

main = !->
    failed: Result I64, String = Result::Err (String::from "error")
    failed.fmap increment .println!
    absent = Option::None
    absent.fmap increment .println!
    absent_bind = Option::None
    absent_bind.bind some .println!
    (bind_generic (success 4), success).println!
    wrapped: Result (I64 -> I64), String = pure increment
    wrapped.ap (success 4) .println!
    (success 4).traverse success .unwrap_or (success 0) .unwrap_or 0 .println!
    (success 4).traverse_m success .unwrap_or (success 0) .unwrap_or 0 .println!
    values!.traverse_m stop .println!
    mut effects = Vec::new!
    effects.push (success 1)
    effects.push (Result::Err (String::from "error"))
    effects.sequence!.println!
"#,
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "Err(error)",
            "None",
            "None",
            "Ok(4)",
            "Ok(5)",
            "4",
            "4",
            "1",
            "2",
            "Err(stop)",
            "Err(error)",
        ]
    );
}

#[test]
fn test_hkt_pure_requires_constructor_context() {
    compile_should_fail(
        r#"
main = !->
    value = pure 4
"#,
        "ambiguous",
    );
}

#[test]
fn test_hkt_receiver_fmap_consumes_owned_source() {
    compile_should_fail(
        r#"
main = !->
    mut values = Vec::new!
    values.push 1
    mapped = values.fmap (value -> value + 1)
    values.len!.println!
    mapped.println!
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_hkt_foldable_and_traversable() {
    let output = compile_and_run(
        r#"
fold_digits: (I64, I64) -> I64
fold_digits = pair -> pair.0 * 10 + pair.1

option_pure_inc: I64 -> Option I64
option_pure_inc = value -> Option::Some (value + 1)

option_pure_double: I64 -> Option I64
option_pure_double = value -> Option::Some (value * 2)

option_pure_double_then_inc: I64 -> Option I64
option_pure_double_then_inc = value -> Option::Some (value * 2 + 1)

result_pure_inc: I64 -> Result I64, I64
result_pure_inc = value -> Result::Ok (value + 1)

option_to_result: Option A -> Result A, I64
option_to_result = value ->
    match value
        Option::Some item => Result::Ok item
        Option::None => Result::Err 99

make_values: () -> Vec I64
make_values = ->
    mut values = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    values

traverse_option_inc: Option I64 -> Option (Option I64)
traverse_option_inc = value -> Option::Traversable::traverse option_pure_inc, value

traverse_result_inc: Result I64, I64 -> Option (Result I64, I64)
traverse_result_inc = value -> (Result _, I64)::Traversable::traverse option_pure_inc, value

traverse_vec_inc: Vec I64 -> Option (Vec I64)
traverse_vec_inc = value -> Vec::Traversable::traverse option_pure_inc, value

option_stop: I64 -> Option I64
option_stop = value ->
    value.println!
    if value == 2
        Option::None
    else
        Option::Some (value + 1)

result_stop: I64 -> Result I64, I64
result_stop = value ->
    value.println!
    if value == 2
        Result::Err 9
    else
        Result::Ok (value + 1)

encode_option_option: Option (Option I64) -> I64
encode_option_option = outer ->
    match outer
        Option::Some inner =>
            match inner
                Option::Some value => value
                Option::None => 0 - 2
        Option::None => 0 - 1

encode_option_result: Option (Result I64, I64) -> I64
encode_option_result = outer ->
    match outer
        Option::Some inner =>
            match inner
                Result::Ok value => value
                Result::Err error => 0 - 100 - error
        Option::None => 0 - 1

encode_option_option_option: Option (Option (Option I64)) -> I64
encode_option_option_option = outer ->
    match outer
        Option::Some middle => encode_option_option middle
        Option::None => 0 - 1

encode_option_option_result: Option (Option (Result I64, I64)) -> I64
encode_option_option_result = outer ->
    match outer
        Option::Some middle => encode_option_result middle
        Option::None => 0 - 1

encode_vec: Vec I64 -> I64
encode_vec = values -> Vec::Foldable::foldl fold_digits, 0, values

encode_option_vec: Option (Vec I64) -> I64
encode_option_vec = outer ->
    match outer
        Option::Some values => encode_vec values
        Option::None => 0 - 1

encode_option_option_vec: Option (Option (Vec I64)) -> I64
encode_option_option_vec = outer ->
    match outer
        Option::Some middle => encode_option_vec middle
        Option::None => 0 - 1

encode_result_option: Result (Option I64), I64 -> I64
encode_result_option = outer ->
    match outer
        Result::Ok inner =>
            match inner
                Option::Some value => value
                Option::None => 0 - 2
        Result::Err error => 0 - 100 - error

encode_result_result: Result (Result I64, I64), I64 -> I64
encode_result_result = outer ->
    match outer
        Result::Ok inner =>
            match inner
                Result::Ok value => value
                Result::Err error => 0 - 100 - error
        Result::Err error => 0 - 200 - error

encode_result_vec: Result (Vec I64), I64 -> I64
encode_result_vec = outer ->
    match outer
        Result::Ok values => encode_vec values
        Result::Err error => 0 - error

main = ->
    fold_option: I64 = Option::Foldable::foldl fold_digits, 0, (Option::Some 4)
    fold_ok_input: Result I64, I64 = Result::Ok 5
    fold_ok: I64 = (Result _, I64)::Foldable::foldl fold_digits, 0, fold_ok_input
    fold_error_input: Result I64, I64 = Result::Err 7
    fold_error: I64 = (Result _, I64)::Foldable::foldl fold_digits, 0, fold_error_input
    fold_vec: I64 = Vec::Foldable::foldl fold_digits, 0, (make_values!)
    mut calls = 0
    captured_step = pair ->
        calls = calls + 1
        pair.0 + pair.1
    captured_fold: I64 = Vec::Foldable::foldl captured_step, 0, (make_values!)
    fold_option.println!
    fold_ok.println!
    fold_error.println!
    fold_vec.println!
    captured_fold.println!
    calls.println!
    option_identity_l: Option (Option I64) = Option::Traversable::traverse option_pure_inc, (Option::Some 1)
    option_identity_r: Option (Option I64) = Option::Applicative::pure (Option::Some 2)
    encode_option_option option_identity_l .println!
    encode_option_option option_identity_r .println!
    result_identity_input: Result I64, I64 = Result::Ok 1
    result_identity_l: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse option_pure_inc, result_identity_input
    result_identity_r: Option (Result I64, I64) = Option::Applicative::pure (Result::Ok 2)
    encode_option_result result_identity_l .println!
    encode_option_result result_identity_r .println!
    vec_identity_l: Option (Vec I64) = Vec::Traversable::traverse (value -> Option::Some value), (make_values!)
    vec_identity_r: Option (Vec I64) = Option::Applicative::pure (make_values!)
    encode_option_vec vec_identity_l .println!
    encode_option_vec vec_identity_r .println!

    option_composition_inner: Option (Option I64) = Option::Traversable::traverse option_pure_double_then_inc, (Option::Some 1)
    option_composition_l: Option (Option (Option I64)) = Option::Some option_composition_inner
    option_first: Option (Option I64) = Option::Traversable::traverse option_pure_double, (Option::Some 1)
    option_composition_r: Option (Option (Option I64)) = Option::Functor::fmap traverse_option_inc, option_first

    result_composition_input_l: Result I64, I64 = Result::Ok 1
    result_composition_inner: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse option_pure_double_then_inc, result_composition_input_l
    result_composition_l: Option (Option (Result I64, I64)) = Option::Some result_composition_inner
    result_composition_input_r: Result I64, I64 = Result::Ok 1
    result_first: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse option_pure_double, result_composition_input_r
    result_composition_r: Option (Option (Result I64, I64)) = Option::Functor::fmap traverse_result_inc, result_first

    vec_composition_inner: Option (Vec I64) = Vec::Traversable::traverse option_pure_double_then_inc, (make_values!)
    vec_composition_l: Option (Option (Vec I64)) = Option::Some vec_composition_inner
    vec_first: Option (Vec I64) = Vec::Traversable::traverse option_pure_double, (make_values!)
    vec_composition_r: Option (Option (Vec I64)) = Option::Functor::fmap traverse_vec_inc, vec_first
    encode_option_option_option option_composition_l .println!
    encode_option_option_option option_composition_r .println!
    encode_option_option_result result_composition_l .println!
    encode_option_option_result result_composition_r .println!
    encode_option_option_vec vec_composition_l .println!
    encode_option_option_vec vec_composition_r .println!

    option_natural_input_l: Option I64 = Option::Some 1
    option_natural_effect: Option (Option I64) = Option::Traversable::traverse option_pure_inc, option_natural_input_l
    option_naturality_l: Result (Option I64), I64 = option_to_result option_natural_effect
    option_natural_input_r: Option I64 = Option::Some 1
    option_naturality_r: Result (Option I64), I64 = Option::Traversable::traverse result_pure_inc, option_natural_input_r

    result_natural_input_l: Result I64, I64 = Result::Ok 1
    result_natural_effect: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse option_pure_inc, result_natural_input_l
    result_naturality_l: Result (Result I64, I64), I64 = option_to_result result_natural_effect
    result_natural_input_r: Result I64, I64 = Result::Ok 1
    result_naturality_r: Result (Result I64, I64), I64 = (Result _, I64)::Traversable::traverse result_pure_inc, result_natural_input_r

    vec_natural_effect: Option (Vec I64) = Vec::Traversable::traverse option_pure_inc, (make_values!)
    vec_naturality_l: Result (Vec I64), I64 = option_to_result vec_natural_effect
    vec_naturality_r: Result (Vec I64), I64 = Vec::Traversable::traverse result_pure_inc, (make_values!)
    encode_result_option option_naturality_l .println!
    encode_result_option option_naturality_r .println!
    encode_result_result result_naturality_l .println!
    encode_result_result result_naturality_r .println!
    encode_result_vec vec_naturality_l .println!
    encode_result_vec vec_naturality_r .println!

    "app-option".println!
    vec_app_option: Option (Vec I64) = Vec::Traversable::traverse option_stop, (make_values!)
    encode_option_vec vec_app_option .println!
    "monad-option".println!
    vec_monad_option: Option (Vec I64) = Vec::Traversable::traverse_m option_stop, (make_values!)
    encode_option_vec vec_monad_option .println!
    "app-result".println!
    vec_app_result: Result (Vec I64), I64 = Vec::Traversable::traverse result_stop, (make_values!)
    encode_result_vec vec_app_result .println!
    "monad-result".println!
    vec_monad_result: Result (Vec I64), I64 = Vec::Traversable::traverse_m result_stop, (make_values!)
    encode_result_vec vec_monad_result .println!

    "container-short".println!
    none_input: Option I64 = Option::None
    none_traverse: Option (Option I64) = Option::Traversable::traverse_m option_stop, none_input
    failed_input: Result I64, I64 = Result::Err 7
    failed_traverse: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse_m option_stop, failed_input
    encode_option_option none_traverse .println!
    encode_option_result failed_traverse .println!
    0
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "4",
            "5",
            "0",
            "123",
            "6",
            "3",
            "2",
            "2",
            "2",
            "2",
            "123",
            "123",
            "3",
            "3",
            "3",
            "3",
            "357",
            "357",
            "2",
            "2",
            "2",
            "2",
            "234",
            "234",
            "app-option",
            "1",
            "2",
            "3",
            "-1",
            "monad-option",
            "1",
            "2",
            "-1",
            "app-result",
            "1",
            "2",
            "3",
            "-9",
            "monad-result",
            "1",
            "2",
            "-9",
            "container-short",
            "-2",
            "-107",
        ]
    );
}

#[test]
fn test_std_traversable_sequence() {
    let output = compile_and_run(
        r#"
main = ->
    mut effects: Vec (Option I64) = Vec::new!
    effects.push (Option::Some 4)
    effects.push (Option::Some 5)
    sequenced: Option (Vec I64) = sequence effects
    match sequenced
        Option::Some values =>
            values.len!.println!
            match (values.get 0)
                Option::Some value => *value .println!
                Option::None => 0.println!
        Option::None => 0.println!

    mut failed_effects: Vec (Option I64) = Vec::new!
    failed_effects.push (Option::Some 1)
    failed_effects.push Option::None
    failed_effects.push (Option::Some 3)
    failed: Option (Vec I64) = sequence failed_effects
    match failed
        Option::Some _ => 1.println!
        Option::None => -1 .println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["2", "4", "-1"]);
}

#[test]
fn test_std_vec_eager_map_move_only_preserves_order() {
    let output = compile_and_run(
        r#"
string_len: String -> I64
string_len = value -> value.len!

print_value: I64 -> ()
print_value = value ->
    value.println!
    return

main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from "a")
    values.push (String::from "bbb")
    mapped: Vec I64 = values.map string_len
    mapped.for_each_owned print_value
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["1", "3"]);
}

#[test]
fn test_std_for_each_ranges_and_mutable_capture() {
    let output = compile_and_run(
        r#"
main = !->
    mut digits: I64 = 0
    for_each 1..=3, number !->
        digits = digits * 10 + number
    digits.println!
    for_each 4..6, (!.println!)
    for_each 5..5, (!.println!)
    for_each 5..3, (!.println!)
    for_each 5..=3, (!.println!)
    for_each 7..=7, (!.println!)
    for_each 9223372036854775806..=9223372036854775807, (!.println!)
    mut visits: I64 = 0
    for_each 0..100000, _ !->
        visits = visits + 1
    visits.println!
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "123",
            "4",
            "5",
            "7",
            "9223372036854775806",
            "9223372036854775807",
            "100000"
        ]
    );
}

#[test]
fn test_std_for_each_owned_and_borrowed_collections() {
    let output = compile_and_run(
        r#"
main = !->
    mut words: Vec String = Vec::new!
    words.push String::from "first"
    words.push String::from "second"
    for_each &words, (!.println!)
    words.len!.println!
    for_each words, (!.println!)

    values = [3, 4, 5]
    view: &[I64] = &values[..]
    for_each view, value !-> (*value).println!
    values[0].println!
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["first", "second", "2", "first", "second", "3", "4", "5", "3"]
    );
}

#[test]
fn test_std_for_each_book_fizzbuzz() {
    let source = include_str!("../../docs/examples/fizzbuzz.rk");
    let chapter = include_str!("../../docs/src/getting-started/first-project.md");
    let listing = chapter
        .split_once("```rock\n")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0;
    assert_eq!(listing.trim(), source.trim());
    let expected = (1..=30)
        .map(|number| match (number % 3, number % 5) {
            (0, 0) => "FizzBuzz".to_string(),
            (0, _) => "Fizz".to_string(),
            (_, 0) => "Buzz".to_string(),
            _ => number.to_string(),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        compile_and_run(source).lines().collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn test_std_for_each_hkt_foldable_carriers() {
    let output = compile_and_run(
        r#"
struct One T
    < value: T

impl Foldable for One
    try_fold = mut step, initial, value ->
        step.call_mut (initial, value.value)

main = !->
    for_each (Option::Some 7), (!.println!)
    absent: Option I64 = Option::None
    for_each absent, (!.println!)
    success: Result I64, I64 = Result::Ok 9
    failure: Result I64, I64 = Result::Err 99
    for_each success, (!.println!)
    for_each failure, (!.println!)
    one = One
        value: String::from "custom foldable"
    for_each one, (!.println!)
"#,
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["7", "9", "custom foldable"]
    );
}

#[test]
fn test_unit_receiver_sections_discard_results_and_capture_arguments() {
    let output = compile_and_run(
        r#"
struct Number
    < value: I64

impl Number
    @plus: I64 -> I64
    @plus = amount -> self.value + amount

main = !->
    for_each 1..=3, (!.println!)
    offset = 5
    value = Number
        value: 7
    for_each (Option::Some value), (!.plus offset .println!)
    suffix = String::from "!"
    for_each (Option::Some (String::from "hello")), (!.concat suffix.clone! .println!)
    read: Number -> I64 = (.plus offset)
    ignore: Number -> () = (!.plus offset)
    other = Number
        value: 10
    read other .println!
    ignored = Number
        value: 20
    ignore ignored
"#,
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        ["1", "2", "3", "12", "hello!", "15"]
    );
}

#[test]
fn test_std_for_each_rejects_open_ended_ranges() {
    for range in ["(..)", "(1..)", "(..3)", "(..=3)"] {
        let (output, success) = compile_and_run_with_status(&format!(
            "main = !->\n    for_each {range}, number !-> number.println!\n"
        ));
        assert!(!success, "range {range} should fail");
        assert_eq!(output.trim(), "for_each requires a bounded range");
    }
}

#[test]
fn test_std_for_each_drops_owned_items_and_callback_once() {
    let output = compile_and_run(
        r#"
struct Item
    < id: I64

impl Drop for Item
    ~@drop = !-> self.id.println!

struct Counter
    < total: I64

impl FnOnce Item, () for Counter
    type Output = ()
    ~@call_once = item !-> item.id.println!

impl FnMut Item, () for Counter
    type Output = ()
    ^@call_mut = item !->
        self.total = self.total + item.id

impl Drop for Counter
    ~@drop = !-> self.total.println!

main = !->
    mut items: Vec Item = Vec::new!
    first = Item
        id: 2
    second = Item
        id: 3
    items.push first
    items.push second
    counter = Counter
        total: 0
    for_each items, counter
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["2", "3", "5"]);
}

#[test]
fn test_std_for_each_consumes_owned_source() {
    compile_should_fail(
        r#"
main = !->
    mut values: Vec I64 = Vec::new!
    values.push 1
    for_each values, value !-> value.println!
    values.len!.println!
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_std_for_each_rejects_non_unit_action() {
    compile_should_fail(
        r#"
main = !->
    for_each 1..=3, value -> value
"#,
        "does not implement trait",
    );
}

#[test]
fn test_std_vec_map_ref_preserves_source() {
    let output = compile_and_run(
        r#"
string_len: &String -> I64
string_len = value -> value.len!

print_value: I64 -> ()
print_value = value ->
    value.println!
    return

main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from "aa")
    values.push (String::from "bbbb")
    mapped: Vec I64 = values.map_ref string_len
    mapped.for_each_owned print_value
    values.len!.println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["2", "4", "2"]);
}

#[test]
fn test_std_vec_map_ref_contextually_types_closure_parameter() {
    let output = compile_and_run(
        r#"
print_value: I64 -> ()
print_value = value ->
    value.println!
    return

main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from "aa")
    values.push (String::from "bbbb")
    mapped: Vec I64 = values.map_ref (value -> value.len!)
    mapped.for_each_owned print_value
    values.len!.println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["2", "4", "2"]);
}

#[test]
fn test_std_vec_try_for_each_short_circuits() {
    let output = compile_and_run(
        r#"
visit: &I64 -> Result (), I64
visit = value ->
    item = *value
    item.println!
    if item == 2
        Result::Err 9
    else
        unit: () = make_unit!
        Result::Ok unit

make_unit: () -> ()
make_unit = -> return

main = ->
    mut values: Vec I64 = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    result: Result (), I64 = values.try_for_each visit
    match result
        Result::Ok unit => 0.println!
        Result::Err error => error.println!
    values.len!.println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["1", "2", "9", "3"]);
}

#[test]
fn test_std_vec_eager_operations_preserve_order_and_are_stack_safe() {
    let output = compile_and_run(
        r#"
double: I64 -> I64
double = value -> value * 2

print_ref: &I64 -> ()
print_ref = value ->
    *value .println!
    return

print_owned: I64 -> ()
print_owned = value ->
    value.println!
    return

is_odd: &I64 -> Bool
is_odd = value -> *value % 2 == 1

keep_even: I64 -> Option I64
keep_even = value ->
    if value % 2 == 0
        Option::Some (value * 10)
    else
        Option::None

try_double: I64 -> Result I64, I64
try_double = value ->
    value.println!
    if value == 3
        Result::Err 99
    else
        Result::Ok (value * 2)

main = ->
    mut empty: Vec I64 = Vec::new!
    mapped_empty: Vec I64 = empty.map double
    mapped_empty.len!.println!

    mut single: Vec I64 = Vec::new!
    single.push 7
    mapped_single: Vec I64 = single.map double
    mapped_single.for_each print_ref

    mut borrowed: Vec I64 = Vec::new!
    borrowed.push 1
    borrowed.push 2
    borrowed.push 3
    borrowed.for_each print_ref
    borrowed.len!.println!

    mut stateful: Vec I64 = Vec::new!
    stateful.push 1
    stateful.push 2
    stateful.push 3
    total = 0
    stateful.for_each_owned (value -> total = total + value)
    total.println!

    mut owned: Vec I64 = Vec::new!
    owned.push 4
    owned.push 5
    owned.for_each_owned print_owned

    mut retained: Vec I64 = Vec::new!
    retained.push 1
    retained.push 2
    retained.push 3
    retained.push 4
    retained.retain is_odd
    retained.for_each print_ref

    mut filtered: Vec I64 = Vec::new!
    filtered.push 1
    filtered.push 2
    filtered.push 3
    filtered.push 4
    kept: Vec I64 = filtered.filter is_odd
    kept.for_each print_ref

    mut filter_mapped: Vec I64 = Vec::new!
    filter_mapped.push 1
    filter_mapped.push 2
    filter_mapped.push 3
    filter_mapped.push 4
    transformed: Vec I64 = filter_mapped.filter_map keep_even
    transformed.for_each print_ref

    mut fallible: Vec I64 = Vec::new!
    fallible.push 1
    fallible.push 2
    fallible.push 3
    fallible.push 4
    match (fallible.try_map try_double)
        Result::Ok values => values.len!.println!
        Result::Err error => error.println!

    mut large: Vec I64 = Vec::new!
    i = 0
    while i < 10000
        large.push i
        i = i + 1
    doubled: Vec I64 = large.map double
    doubled.len!.println!
    0
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "0", "14", "1", "2", "3", "3", "6", "4", "5", "1", "3", "1", "3", "20", "40", "1", "2",
            "3", "99", "10000",
        ]
    );
}

#[test]
fn test_std_vec_eager_operations_drop_inputs_and_outputs_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

transform: Tracked -> Tracked
transform = value ->
    Tracked
        value: value.value + 10

main = ->
    mut values: Vec Tracked = Vec::new!
    values.push (Tracked
        value: 1)
    values.push (Tracked
        value: 2)
    values.push (Tracked
        value: 3)
    mapped: Vec Tracked = values.map transform
    mapped.len!.println!
    0
"#,
    );

    let mut lines = output.lines().collect::<Vec<_>>();
    lines.sort_unstable();
    assert_eq!(lines, vec!["1", "11", "12", "13", "2", "3", "3"]);
}

#[test]
fn test_std_vec_filtering_and_try_map_drop_each_value_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

is_odd: &Tracked -> Bool
is_odd = value -> value.value % 2 == 1

filter_transform: Tracked -> Option Tracked
filter_transform = value ->
    if value.value % 2 == 1
        Option::Some (Tracked
            value: value.value + 100)
    else
        Option::None

try_transform: Tracked -> Result Tracked, I64
try_transform = value ->
    0 - value.value .println!
    if value.value == 502
        Result::Err 9
    else
        Result::Ok (Tracked
            value: value.value + 100)

main = ->
    mut retained: Vec Tracked = Vec::new!
    retained.push (Tracked
        value: 101)
    retained.push (Tracked
        value: 102)
    retained.push (Tracked
        value: 103)
    retained.retain is_odd

    mut filtered: Vec Tracked = Vec::new!
    filtered.push (Tracked
        value: 201)
    filtered.push (Tracked
        value: 202)
    filtered.push (Tracked
        value: 203)
    kept: Vec Tracked = filtered.filter is_odd

    mut filter_mapped: Vec Tracked = Vec::new!
    filter_mapped.push (Tracked
        value: 301)
    filter_mapped.push (Tracked
        value: 302)
    filter_mapped.push (Tracked
        value: 303)
    transformed: Vec Tracked = filter_mapped.filter_map filter_transform

    mut fallible: Vec Tracked = Vec::new!
    fallible.push (Tracked
        value: 501)
    fallible.push (Tracked
        value: 502)
    fallible.push (Tracked
        value: 503)
    fallible.push (Tracked
        value: 504)
    match (fallible.try_map try_transform)
        Result::Ok values => values.len!.println!
        Result::Err error => error.println!
    0
"#,
    );

    let mut values = output
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect::<Vec<_>>();
    values.sort_unstable();
    assert_eq!(
        values,
        vec![
            -502, -501, 9, 101, 102, 103, 201, 202, 203, 301, 302, 303, 401, 403, 501, 502, 503,
            504, 601,
        ]
    );
}

#[test]
fn test_std_option_result_fp_operators() {
    let output = compile_and_run(
        r#"
inc = x -> x + 1

keep_even_opt = x ->
    if x % 2 == 0
        Option::Some x
    else
        Option::None

keep_even_res = x ->
    if x % 2 == 0
        Result::Ok x
    else
        Result::Err 1

main = ->
    some = Option::Some 4
    none: Option I64 = Option::None
    fallback_none: Option I64 = Option::None
    some_for_result: Option I64 = Option::Some 12
    none_for_result: Option I64 = Option::None
    wrapped_inc: Option (I64 -> I64) = Option::Some inc
    ok: Result I64, I64 = Result::Ok 4
    err: Result I64, I64 = Result::Err 7
    mapped_ok: Result I64, I64 = Result::Ok 4
    mapped_ok_reverse: Result I64, I64 = Result::Ok 4
    fallback_err: Result I64, I64 = Result::Err 7
    wrapped_res_inc: Result (I64 -> I64), I64 = Result::Ok inc

    (some >>= keep_even_opt).unwrap_or 0 .println!
    (none >>= keep_even_opt).unwrap_or 0 .println!
    (inc <$> Option::Some 4).unwrap_or 0 .println!
    ((Option::Some 4) <&> inc).unwrap_or 0 .println!
    (wrapped_inc <*> Option::Some 4).unwrap_or 0 .println!
    (fallback_none <|> Option::Some 9).unwrap_or 0 .println!
    (some_for_result !> 5).fold (x -> x), (x -> x) .println!
    (none_for_result !> 7).fold (x -> x), (x -> x) .println!

    (ok >>= keep_even_res).unwrap_or 0 .println!
    (err >>= keep_even_res).unwrap_or 0 .println!
    (inc <$> mapped_ok).unwrap_or 0 .println!
    (mapped_ok_reverse <&> inc).unwrap_or 0 .println!
    (wrapped_res_inc <*> Result::Ok 4).unwrap_or 0 .println!
    (fallback_err <|> Result::Ok 9).unwrap_or 0 .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["4", "0", "5", "5", "5", "9", "12", "7", "4", "0", "5", "5", "5", "9"]
    );
}

#[test]
fn test_std_result_bifunctor_and_error_operator() {
    let output = compile_and_run(
        r#"
> std::functional::first
> std::functional::second

inc = value -> value + 1
double = value -> value * 2
add_ten = value -> value + 10
triple = value -> value * 3
double_after_inc = value -> double (inc value)
triple_after_add_ten = value -> triple (add_ten value)
identity = value -> value

main = ->
    mapped_ok: Result I64, I64 = bimap inc, add_ten, (Result::Ok 4)
    mapped_err: Result I64, I64 = bimap inc, add_ten, (Result::Err 7)
    first_ok: Result I64, I64 = first inc, (Result::Ok 4)
    second_err: Result I64, I64 = second add_ten, (Result::Err 7)
    operator_err: Result I64, I64 = (Result::Err 7) <!> add_ten

    identity_ok: Result I64, I64 = bimap identity, identity, (Result::Ok 4)
    identity_err: Result I64, I64 = bimap identity, identity, (Result::Err 7)

    composed_ok_l: Result I64, I64 = bimap double_after_inc, triple_after_add_ten, (Result::Ok 4)
    composed_ok_r: Result I64, I64 = bimap double, triple, (bimap inc, add_ten, (Result::Ok 4))
    composed_err_l: Result I64, I64 = bimap double_after_inc, triple_after_add_ten, (Result::Err 7)
    composed_err_r: Result I64, I64 = bimap double, triple, (bimap inc, add_ten, (Result::Err 7))

    mapped_ok.println!
    mapped_err.println!
    first_ok.println!
    second_err.println!
    operator_err.println!
    identity_ok.println!
    identity_err.println!
    composed_ok_l.println!
    composed_ok_r.println!
    composed_err_l.println!
    composed_err_r.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec![
            "Ok(5)", "Err(17)", "Ok(5)", "Err(17)", "Err(17)", "Ok(4)", "Err(7)", "Ok(10)",
            "Ok(10)", "Err(51)", "Err(51)",
        ]
    );
}

#[test]
fn test_std_show_default_println() {
    let src = r#"
struct Named
    < value: I64

impl Show for Named
    @show = -> "Named(" + (int_to_string self.value) + ")"

main = ->
    n = Named
        value: 7
    result: Result I64, I64 = Result::Ok 4
    n.println!
    Option::Some 3 .println!
    result.println!
    0
"#;

    let output = compile_and_run(src);
    assert_eq!(output, "Named(7)\nSome(3)\nOk(4)\n");
}

#[test]
fn test_std_option_result_show_prints_payloads() {
    let output = compile_and_run(
        r#"
> std::control::Option
> std::control::Result
> std::io::IoError

main = ->
    none: Option I64 = Option::None
    ok_41: Result I64, I64 = Result::Ok 41
    err_4: Result I64, I64 = Result::Err 4
    ok_5: Result I64, I64 = Result::Ok 5
    (Option::Some 3).println!
    none.println!
    ok_41.println!
    err_4.println!
    ok_5.println!
    err_io_a: Result I64, IoError = Result::Err (IoError::Os (0 as I32))
    err_io_b: Result I64, IoError = Result::Err (IoError::Os (0 as I32))
    err_io_a.println!
    err_io_b.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec![
            "Some(3)",
            "None",
            "Ok(41)",
            "Err(4)",
            "Ok(5)",
            "Err(IoError)",
            "Err(IoError)"
        ]
    );
}

#[test]
fn test_std_option_show_owned_string_is_non_consuming() {
    let output = compile_and_run(
        r#"
main = ->
    value = Option::Some (String::from "hello")
    value.println!
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "Some(hello)\nSome(hello)");
}

#[test]
fn test_std_result_show_owned_string_is_non_consuming() {
    let output = compile_and_run(
        r#"
main = ->
    ok: Result String, String = Result::Ok (String::from "hello")
    err: Result String, String = Result::Err (String::from "oops")
    ok.println!
    ok.println!
    err.println!
    err.println!
    0
"#,
    );

    assert_eq!(output.trim(), "Ok(hello)\nOk(hello)\nErr(oops)\nErr(oops)");
}

#[test]
fn test_std_option_map_owned_string_consumes_receiver() {
    compile_should_fail(
        r#"
main = ->
    value = Option::Some (String::from "hello")
    mapped = value.map (s -> s.len!)
    mapped.unwrap_or 0 .println!
    again = value.show!
    again.println!
    0
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_std_result_map_owned_string_consumes_receiver() {
    compile_should_fail(
        r#"
main = ->
    value: Result String, String = Result::Ok (String::from "hello")
    mapped = value.map (s -> s.len!)
    mapped.unwrap_or 0 .println!
    again = value.show!
    again.println!
    0
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_std_result_map_err_owned_string_consumes_receiver() {
    compile_should_fail(
        r#"
main = ->
    value: Result String, String = Result::Err (String::from "oops")
    mapped = value.map_err (s -> s.len!)
    (mapped.fold (n -> n), (s -> s.len!)) .println!
    again = value.show!
    again.println!
    0
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_std_option_bind_owned_string_forwards_move_receiver() {
    let output = compile_and_run(
        r#"
bind: String -> Option I64
bind = s -> Option::Some (s.len!)

main = ->
    value = Option::Some (String::from "hello")
    mapped = value >>= bind
    mapped.unwrap_or 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_std_result_bind_owned_string_forwards_move_receiver() {
    let output = compile_and_run(
        r#"
bind: String -> Result I64, String
bind = s -> Result::Ok (s.len!)

main = ->
    value: Result String, String = Result::Ok (String::from "hello")
    mapped = value >>= bind
    mapped.unwrap_or 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_result_method_sections_preserve_unresolved_payload_until_materialization() {
    let output = compile_and_run(
        r#"
> std::control::Result

struct Source
    < value: I64

struct Sink
    < value: I64

impl Source
    @next: Result Sink, I64
    @next = -> Result::Ok (Sink
        value: self.value)

impl Sink
    @finish: Result I64, I64
    @finish = -> Result::Ok self.value

main = ->
    source = Source
        value: 42
    initial: Result Source, I64 = Result::Ok source
    match (initial >>= (.next!) >>= (.finish!))
        Result::Ok result =>
            result.println!
            0
        Result::Err _ => 1
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_std_pipe_operator_function() {
    let output = compile_and_run(
        r#"
inc = x -> x + 1

main = ->
    41 |> inc .println!
    (((Option::Some 41) <&> inc).unwrap_or 0) .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["42", "42"]);
}

#[test]
fn test_module_local_pipe_operator_shadows_prelude() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("ops.rk"),
        "\
|> = x, f -> 43

< |>
",
    )
    .unwrap();

    std::fs::write(
        dir.join("test.rk"),
        "\
mod ops
> ops::|>

inc = x -> x + 1

main = ->
    41 |> inc .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(stdout.trim(), "43");
}

#[test]
fn test_root_pipe_operator_shadows_prelude_with_same_param_names() {
    let output = compile_and_run(
        r#"
|> = x, f -> 43
inc = x -> x + 1

main = ->
    41 |> inc .println!
    0
"#,
    );

    assert_eq!(output.trim(), "43");
}

#[test]
fn test_application_binds_before_infix() {
    let output = compile_and_run(
        r#"
double = x -> x * 2

main = ->
    complete_argument = double 2 + 2
    complete_argument.println!
    application_first = (double 2) + 2
    application_first.println!
    double 2 + 2 .println!
    Option::Some 2 <&> (+ 2) .unwrap_or 0 .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["8", "6", "8", "4"]);
}

#[test]
fn test_vec_get_option() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v.push 20
    match (v.get 0)
        Option::Some val => *val .println!
        Option::None => "None".println!
    match (v.get 5)
        Option::Some val => *val .println!
        Option::None => "None".println!
    match (v.get 1)
        Option::Some val => *val .println!
        Option::None => 99.println!
    match (v.get 99)
        Option::Some val => *val .println!
        Option::None => 99.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "None");
    assert_eq!(lines[2], "20");
    assert_eq!(lines[3], "99");
}

#[test]
fn test_range_for_loop() {
    let output = compile_and_run(
        r#"
main = ->
    for i in 0..5
        i.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "0");
    assert_eq!(lines[4], "4");
}

#[test]
fn test_range_sum() {
    let output = compile_and_run(
        r#"
main = ->
    sum = 0
    for i in 1..101
        sum = sum + i
    sum.println!
    0
"#,
    );
    assert_eq!(output.trim(), "5050");
}

#[test]
fn test_range_with_variable() {
    let output = compile_and_run(
        r#"
main = ->
    n = 5
    total = 0
    for i in 0..n
        total = total + i
    total.println!
    0
"#,
    );
    assert_eq!(output.trim(), "10");
}

#[test]
fn test_range_nested() {
    let output = compile_and_run(
        r#"
main = ->
    count = 0
    for i in 0..3
        for j in 0..4
            count = count + 1
    count.println!
    0
"#,
    );
    assert_eq!(output.trim(), "12");
}

#[test]
fn test_comments() {
    let output = compile_and_run(
        r#"
// top level comment
// another top level comment
main = ->
    // comment in body
    x = 42
    // comment between statements
    y = x + 8
    // comment before result
    y.println!
    // trailing comment
    0
"#,
    );
    assert_eq!(output.trim(), "50");
}

#[test]
fn test_str_indexing() {
    compile_should_fail(
        r#"
main = ->
    s = "hello"
    s[0].println!
    0
"#,
        "No implementation found for operator '[]'",
    );
}

#[test]
fn test_as_cast_int_to_float() {
    // `x as F64` — integer to float
    let output = compile_and_run(
        r#"
main = ->
    x = 42
    f = x as F64
    f.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_as_cast_float_to_int() {
    // `f as I64` — float to int (truncates)
    let output = compile_and_run(
        r#"
main = ->
    f = 3.7
    i = f as I64
    i.println!
    0
"#,
    );
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_as_cast_int_narrowing() {
    // `val as I32` — i64 narrowed to i32
    let output = compile_and_run(
        r#"
main = ->
    x = 1000
    y = x as I32
    y.println!
    0
"#,
    );
    assert_eq!(output.trim(), "1000");
}

#[test]
fn test_as_cast_u8_to_char() {
    // byte as Char — same as old ~U8ToChar
    let output = compile_and_run(
        r#"
main = ->
    bytes = [104 as U8]
    c = bytes[0] as Char
    c.println!
    0
"#,
    );
    assert_eq!(output.trim(), "h");
}

#[test]
fn test_char_show_literal() {
    let output = compile_and_run(
        r#"
main = ->
    'A'.println!
    0
"#,
    );
    assert_eq!(output.trim(), "A");
}

#[test]
fn test_as_cast_in_binop() {
    // `a + b as I64` — cast binds to RHS only (same-type cast verifies precedence)
    let output = compile_and_run(
        r#"
main = ->
    a = 10
    b = 5
    result = a + b as I64
    result.println!
    0
"#,
    );
    assert_eq!(output.trim(), "15");
}

#[test]
fn test_as_cast_ptr_to_int() {
    // Pointer-to-int cast: pointer value cast to I64 should be non-zero
    let output = compile_and_run(
        r#"
main = ->
    s = "hello"
    p = s.as_ptr!
    i = p as I64
    if i > 0
        "ok".println!
    0
"#,
    );
    assert_eq!(output.trim(), "ok");
}

#[test]
fn test_type_conversion_via_as() {
    // `to_float` and `to_int` now use `as` under the hood
    let output = compile_and_run(
        r#"
main = ->
    x = 7
    f = to_float x
    f.println!
    y = 2.9
    i = to_int y
    i.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "2");
}

#[test]
fn test_str_literal_index_out_of_bounds_is_rejected_as_str_indexing() {
    compile_should_fail(
        r#"
main = ->
    c = "hello"[10]
    c.println!
    0
"#,
        "No implementation found for operator '[]'",
    );
}

#[test]
fn test_str_variable_indexing_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    s = "hello"
    i = 3
    s[i].println!
    0
"#,
        "No implementation found for operator '[]'",
    );
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
fn test_drop_in_place_drops_raw_pointer_value() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8
extern free: *U8 -> ()

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    ptr: *Tracked = (malloc 8) as *Tracked
    unsafe *ptr = Tracked
        value: 9
    unsafe drop_in_place ptr
    free (ptr as *U8)
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_std_box_drops_inner_value() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    boxed = Box::new (Tracked
        value: 7)
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_std_box_rejects_zero_sized_value() {
    let (_output, success) = compile_and_run_with_status(
        r#"
> std::mem::Box

struct Empty

main = ->
    boxed = Box::new Empty
    0
"#,
    );

    assert!(!success, "Box::new should reject zero-sized values");
}

#[test]
fn test_std_box_new_uses_non_null_pointer() {
    let output = compile_and_run(
        r#"
> std::mem::Box

main = ->
    boxed = Box::new 42
    (boxed.as_ptr! as I64) != 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_inherent_drop_method_is_not_automatic_cleanup() {
    let output = compile_and_run(
        r#"
struct NotDrop
    < value: I64

impl NotDrop
    @drop = ->
        self.value.println!
        return

main = ->
    value = NotDrop
        value: 99
    value.drop!
    0
"#,
    );

    assert_eq!(output.trim(), "99");
}

#[test]
fn test_std_box_drops_aggregate_field_value() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < tracked: Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    boxed = Box::new (Wrapper
        tracked: Tracked
            value: 8)
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_drop_order_runs_owner_before_field_cleanup() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Child
    < value: I64

struct Owner
    < child: Child

impl Drop for Child
    ~@drop = ->
        self.value.println!
        return

impl Drop for Owner
    ~@drop = ->
        self.child.value + 100 .println!
        return

main = ->
    owner = Owner
        child: Child
            value: 4
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["104", "4"]);
}

#[test]
fn test_branch_move_drops_value_once_after_merge() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

branch = cond, value ->
    a = Tracked
        value: value
    if cond
        b = a
    0

main = ->
    branch true, 1
    branch false, 2
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["1", "2"]);
}

#[test]
fn test_field_assignment_drops_old_field_before_overwrite() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < tracked: Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut wrapper = Wrapper
        tracked: Tracked
            value: 1
    wrapper.tracked = Tracked
        value: 2
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["1", "2"]);
}

#[test]
fn test_tuple_array_and_enum_payload_cleanup_drop_elements() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Tracked
    < value: I64

enum MaybeTracked
    Some Tracked
    None

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    tuple = (Tracked
        value: 3, Tracked
        value: 4)
    array = [Tracked
        value: 5, Tracked
        value: 6]
    some = MaybeTracked::Some (Tracked
        value: 7)
    none = MaybeTracked::None
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["3", "4", "5", "6", "7"]);
}

#[test]
fn test_expression_statement_temporary_is_dropped() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

make = value -> Tracked
    value: value

main = ->
    make 8
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_borrowed_temporary_drops_when_call_expression_finishes() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Holder
    < value: I64

impl Drop for Holder
    ~@drop = ->
        2.println!
        return

make_holder = ->
    Holder
        value: 1

use_holder: &Holder -> ()
use_holder = holder ->
    holder.value.println!
    return

main = ->
    use_holder (&(make_holder!))
    3.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2", "3"]);
}

#[test]
fn test_return_expression_temporary_drops_before_returning_to_caller() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Holder
    < value: I64

impl Drop for Holder
    ~@drop = ->
        2.println!
        return

make_holder = ->
    Holder
        value: 1

read_holder: &Holder -> I64
read_holder = holder -> holder.value

make_value = ->
    return read_holder (&(make_holder!))

main = ->
    make_value! .println!
    3.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["2", "1", "3"]);
}

#[test]
fn test_owned_string_temporary_lives_through_ffi_call() {
    let output = compile_and_run(
        r#"
> std::ffi::libc::puts
> std::string::String

main = ->
    puts ((String::from "hello").as_ptr!)
    0
"#,
    );

    assert_eq!(output.trim(), "hello");
}

#[test]
fn test_std_has_no_keep_alive_lifetime_workarounds() {
    let std_files = ["src/convert.rk", "src/control/option.rk", "src/control/result.rk", "src/show.rk"];
    let offenders = std_files
        .iter()
        .filter_map(|file| {
            let path = std_path().join(file);
            let source = fs::read_to_string(&path).unwrap();
            source.contains("keep_alive").then_some(file.to_string())
        })
        .collect::<Vec<_>>();

    assert!(
        offenders.is_empty(),
        "std should rely on full-expression temporary lifetimes, found keep_alive in {offenders:?}"
    );
}

#[test]
fn test_unrelated_trait_named_drop_is_not_automatic_cleanup() {
    let output = compile_and_run(
        r#"
trait Drop
    ~@drop: ()

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    value = Tracked
        value: 9
    0
"#,
    );

    assert_eq!(output.trim(), "");
}

#[test]
fn test_unrelated_trait_named_drop_allows_field_moves() {
    let output = compile_and_run(
        r#"
trait Drop
    ~@drop: ()

struct Child
    < value: I64

struct Tracked
    < child: Child

impl Drop for Tracked
    ~@drop = ->
        self.child.value.println!
        return

take_child = tracked -> tracked.child

main = ->
    tracked = Tracked
        child: Child
            value: 9
    child = take_child tracked
    child.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_std_box_move_drops_inner_value_once() {
    let (output, success) = compile_and_run_with_status(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    a = Box::new (Tracked
        value: 3)
    b = a
    0
"#,
    );

    assert!(success, "program failed with output {output:?}");
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_returned_std_box_binding_drops_inner_value_once() {
    let (output, success) = compile_and_run_with_status(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

make_box: () -> Box Tracked
make_box = -> Box::new (Tracked
    value: 5)

main = ->
    box = make_box!
    0
"#,
    );

    assert!(success, "program should not double-free or abort");
    assert_eq!(output.trim(), "5");
}

#[test]
fn test_method_by_value_owned_arg_drops_once() {
    let (output, success) = compile_and_run_with_status(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = ->
        self.id.println!
        return

struct Sink

impl Sink
    @take: Box Tracked -> ()
    @take = value ->
        return

main = ->
    sink = Sink
    sink.take (Box::new (Tracked
        id: 11))
    0
"#,
    );

    assert!(success, "program should not double-free or abort");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["11"]);
}

#[test]
fn test_partial_move_from_direct_drop_type_is_rejected() {
    compile_should_fail(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = ->
        self.id.println!
        return

struct Owner
    < value: Box Tracked

impl Drop for Owner
    ~@drop = ->
        99.println!
        return

take_value = owner -> owner.value

main = ->
    owner = Owner
        value: Box::new (Tracked
            id: 1)
    boxed = take_value owner
    0
"#,
        "because it implements Drop",
    );
}

#[test]
fn test_nested_partial_move_from_direct_drop_type_is_rejected() {
    compile_should_fail(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = ->
        self.id.println!
        return

struct Inner
    < value: Box Tracked

struct Owner
    < inner: Inner

impl Drop for Owner
    ~@drop = ->
        99.println!
        return

take_value = owner -> owner.inner.value

main = ->
    owner = Owner
        inner: Inner
            value: Box::new (Tracked
                id: 1)
    boxed = take_value owner
    0
"#,
        "because it implements Drop",
    );
}

#[test]
fn test_tuple_partial_move_from_direct_drop_type_is_rejected() {
    compile_should_fail(
        r#"
> std::mem::drop::Drop

struct Child
    < id: I64

struct Owner
    < pair: (Child, I64)

impl Drop for Owner
    ~@drop = ->
        99.println!
        return

make_child = ->
    Child
        id: 1

take_value = owner -> owner.pair.0

main = ->
    owner = Owner
        pair: (make_child!, 2)
    child = take_value owner
    0
"#,
        "because it implements Drop",
    );
}

#[test]
fn test_array_element_move_of_cleanup_value_is_rejected() {
    compile_should_fail(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = ->
        self.id.println!
        return

take_first: [Box Tracked; 1] -> Tracked
take_first = values -> *(values[0])

main = ->
    values: [Box Tracked; 1] = [Box::new (Tracked
        id: 1)]
    boxed = take_first values
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_std_box_reassignment_drops_old_and_new_values_once() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    box = Box::new (Tracked
        value: 1)
    box = Box::new (Tracked
        value: 2)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_by_value_owned_parameter_is_dropped_when_unused() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

consume_unused: Box Tracked -> ()
consume_unused = x ->
    return

main = ->
    consume_unused (Box::new (Tracked
        value: 10))
    0
"#,
    );

    assert_eq!(output.trim(), "10");
}

#[test]
fn test_by_value_owned_parameter_with_local_drop_is_dropped_when_unused() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

consume_unused: Tracked -> ()
consume_unused = x ->
    return

main = ->
    consume_unused (Tracked
        value: 11)
    0
"#,
    );

    assert_eq!(output.trim(), "11");
}

#[test]
fn test_assigning_moved_out_field_does_not_drop_stale_value() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < field: Box Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut wrapper = Wrapper
        field: Box::new (Tracked
            value: 1)
    moved = wrapper.field
    wrapper.field = Box::new (Tracked
        value: 2)
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_enum_partial_payload_move_drops_unmoved_payload_field() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

enum Pair
    Both (Box Tracked) (Box Tracked)

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    pair = Pair::Both (Box::new (Tracked
        value: 1)), (Box::new (Tracked
        value: 2))
    match pair
        Pair::Both first, _ => 0
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_parent_cleanup_skips_moved_field_but_drops_unmoved_sibling() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < first: Box Tracked
    < second: Box Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    wrapper = Wrapper
        first: Box::new (Tracked
            value: 1)
        second: Box::new (Tracked
            value: 2)
    moved = wrapper.first
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_branch_skipped_field_move_drops_both_fields_after_merge() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < first: Box Tracked
    < second: Box Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    wrapper = Wrapper
        first: Box::new (Tracked
            value: 1)
        second: Box::new (Tracked
            value: 2)
    if false
        moved = wrapper.first
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_branch_taken_field_move_drops_moved_field_once_and_sibling_after_merge() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < first: Box Tracked
    < second: Box Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    wrapper = Wrapper
        first: Box::new (Tracked
            value: 1)
        second: Box::new (Tracked
            value: 2)
    if true
        moved = wrapper.first
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
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

#[test]
fn test_unsupported_concrete_indexing_fails_during_lowering() {
    compile_should_fail(
        r#"
main = ->
    value = 42
    x = value[0]
    0
"#,
        "No implementation found for operator '[]' on type I64",
    );
}

#[test]
fn test_deref_receiver_without_index_support_fails_during_lowering() {
    compile_should_fail(
        r#"
trait Deref
    type Target
    @*: &Self::Target

struct Box T
    < value: T

impl Deref for Box T
    type Target = T
    @* = -> &@value

main = ->
    boxed = Box
        value: 7
    boxed[0]
    0
"#,
        "No implementation found for operator '[]' on type Box I64",
    );
}

// ── Borrow checker / MIR tests ──────────────────────────────────────────────

fn compile_example_should_fail(name: &str, expected_error: &str) {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let examples_dir = PathBuf::from(manifest_dir)
        .parent()
        .unwrap()
        .join("examples");
    let source_path = examples_dir.join(format!("{}.rk", name));
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|_| panic!("Failed to read example: {}", name));
    compile_should_fail(&source, expected_error);
}

fn compile_example_should_pass(name: &str) {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let examples_dir = PathBuf::from(manifest_dir)
        .parent()
        .unwrap()
        .join("examples");
    let source_path = examples_dir.join(format!("{}.rk", name));
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|_| panic!("Failed to read example: {}", name));
    compile_and_run(&source);
}

#[test]
fn test_borrow_use_after_move() {
    compile_example_should_fail("mir_tests/use_after_move", "borrow of moved value");
}

#[test]
fn test_borrow_double_move() {
    compile_example_should_fail("mir_tests/double_move", "use of moved value");
}

#[test]
fn test_borrow_uaf() {
    compile_example_should_fail("mir_tests/uaf", "borrow of moved value");
}

#[test]
fn test_borrow_mut_ref_requires_mutable_binding() {
    compile_should_fail(
        r#"
main = ->
    x = 1
    r = &mut x
    0
"#,
        "Cannot take a mutable reference to immutable binding",
    );
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

#[test]
fn test_borrow_mut_ref_not_copy() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    y = r
    z = r
    0
"#,
        "use of moved value",
    );
}

#[test]
fn test_borrow_mut_ref_moves_across_function_call() {
    compile_should_fail(
        r#"
helper = r -> 0

main = ->
    mut x = 1
    r = &mut x
    helper r
    y = r
    0
"#,
        "use of moved value",
    );
}

#[test]
fn test_borrow_mut_ref_blocks_later_use() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    x.println!
    r.println!
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_mut_ref_blocks_assignment() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    x = 2
    r.println!
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_reborrow_allows_write_through_reborrow() {
    compile_should_pass(
        r#"
main = ->
    mut x = 1
    r = &mut x
    s = &mut *r
    *s = 2
    0
"#,
    );
}

#[test]
fn test_borrow_reborrow_blocks_owner_assignment_before_reborrow_use() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    s = &mut *r
    x = 2
    *s = 3
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_mut_ref_blocks_drop_at_scope_end() {
    compile_should_pass(
        r#"
main = ->
    mut x = String::from "hello"
    r = &mut x
    0
"#,
    );
}

#[test]
fn test_borrow_shared_ref_blocks_assignment() {
    compile_should_pass(
        r#"
main = ->
    mut x = 1
    r = &x
    x = 2
    0
"#,
    );
}

#[test]
fn test_borrow_shared_ref_blocks_move() {
    compile_should_pass(
        r#"
main = ->
    mut x: Vec I64 = Vec::new!
    r = &x
    y = x
    0
"#,
    );
}

#[test]
fn test_borrow_reference_copy_releases_with_owner() {
    compile_should_pass(
        r#"
main = ->
    mut x = 1
    r = &x
    s = r
    x = 2
    0
"#,
    );
}

#[test]
fn test_borrow_reference_copy_keeps_original_alias_live() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &x
    s = r
    x = 2
    t = r
    0
"#,
        "borrow",
    );
}

#[test]
fn test_borrow_shared_ref_blocks_assignment_rust_parity() {
    compile_example_should_fail("mir_tests/shared_borrow_then_assign", "borrow");
}

#[test]
fn test_borrow_shared_ref_blocks_move_rust_parity() {
    compile_example_should_fail("mir_tests/shared_borrow_then_move", "borrow");
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
    let mut source = String::from(r#"
equal: T -> T -> Bool where T: Eq
equal = a, b -> a == b

unequal: T -> T -> Bool where T: Eq
unequal = a, b -> a != b

main = !->
"#);
    for ty in ["I8", "I16", "I32", "I64", "U8", "U16", "U32", "U64"] {
        for (method, rhs) in [("equal", 7), ("equal", 8), ("unequal", 7), ("unequal", 8)] {
            source.push_str(&format!("    ({method} (7 as {ty}), ({rhs} as {ty})).println!\n"));
        }
    }
    source.push_str(r#"
    (equal '🦀', '🦀').println!
    (equal 'é', '中').println!
    (unequal 'é', 'é').println!
    (unequal 'a', 'b').println!
    (equal (255 as U8), (127 as U8)).println!
    (equal (65535 as U16), (32767 as U16)).println!
    (equal (4294967295 as U32), (2147483647 as U32)).println!
    high = !(0 as U64)
    (equal high, (9223372036854775807 as U64)).println!
"#);
    assert_eq!(compile_and_run(&source), "true\nfalse\nfalse\ntrue\n".repeat(9) + "false\nfalse\nfalse\nfalse\n");
}

#[test]
fn test_std_numeric_operators_not_preserves_integer_width() {
    let mut source = String::from(r#"
main = !->
"#);
    let mut expected = String::new();
    for (ty, ones) in [
        ("I8", -1i64), ("I16", -1), ("I32", -1), ("I64", -1),
        ("U8", 255), ("U16", 65535), ("U32", 4294967295), ("U64", -1),
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
    let output = compile_and_run(r#"
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
"#);
    assert_eq!(output, "7\n0\n7\n-7\n-7\n3\n3\n4\n4\ntrue\ntrue\n0\n");
}

#[test]
fn test_std_numeric_operators_hash_map_explicit_grow_with_collisions() {
    let output = compile_and_run(r#"
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
"#);
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
fn test_vec_as_slice_index_dispatches_to_slice() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    slice = v.as_slice!
    slice[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_vec_indexing() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    v[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_vec_index_assignment() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    v[1] = 42
    v[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_vec_index_out_of_bounds_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    slice = v.as_slice!
    slice[1].println!
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
fn test_borrowed_vec_can_index_explicit_slice() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    shared = &v
    slice = (*shared).as_slice!
    slice[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_slice_index_of_slice_returns_inner_slice_ref() {
    let output = compile_and_run(
        r#"
main = ->
    rows = ["ab", "cd"]
    rows[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "cd");
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

#[test]
fn test_borrow_closure_shared_capture_blocks_mutation() {
    compile_example_should_fail("mir_tests/closure_shared_capture_blocks_mutation", "borrow");
}

#[test]
fn test_borrow_closure_copy_keeps_capture_loan_live() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    f = -> x.println!
    g = f
    x = 2
    g!
    0
"#,
        "borrow",
    );
}

#[test]
fn test_borrow_closure_move_capture_moves_value() {
    compile_example_should_fail("mir_tests/closure_move_capture_moves_value", "moved");
}

#[test]
fn test_borrow_raw_pointer_from_borrow_preserves_reference_rules() {
    compile_example_should_fail("mir_tests/raw_pointer_from_borrow", "borrow");
}

#[test]
fn test_borrow_raw_pointer_copy_keeps_reference_rules() {
    compile_should_fail(
        r#"
main = ->
    mut x = 42
    r = &mut x
    ptr = r as *I64
    ptr2 = ptr
    y = x
    unsafe *ptr2 = 7
    0
"#,
        "borrow",
    );
}

#[test]
fn test_borrow_deref_mut_borrow_conflicts() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    shared = &x
    mutable = &mut *shared
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_mut_ref_branch_merge_blocks_later_use() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    if x == 1
        r = &mut x
    else
        r = &mut x
    x = 2
    r.println!
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_move_valid() {
    compile_example_should_pass("mir_tests/move_valid");
}

#[test]
fn test_borrow_loop() {
    compile_example_should_pass("mir_tests/loop_test");
}

#[test]
fn test_borrow_shared_borrow() {
    compile_example_should_pass("mir_tests/shared_borrow");
}

#[test]
fn test_borrow_conditional_move() {
    compile_example_should_pass("mir_tests/conditional_move");
}

#[test]
fn test_borrow_mut_and_shared() {
    compile_example_should_pass("mir_tests/mut_and_shared");
}

#[test]
fn test_reference_lifetime_cleanup() {
    compile_example_should_pass("mir_tests/reference_lifetime_cleanup");
}

#[test]
fn test_borrow_disjoint_fields() {
    compile_example_should_pass("mir_tests/field_borrowing");
}

fn proc_macro_host_with_response(
    response: rock_lib::macro_expansion::proc_macro::ProcMacroResponse,
) -> (
    rock_lib::macro_expansion::proc_macro::ProcMacroArtifact,
    PathBuf,
) {
    let response = rock_lib::macro_expansion::proc_macro::encode_response(&response).unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "rock-integration-proc-macro-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let host = temp_dir.join("proc_macro_host");
    fs::write(host.with_extension("response"), response).unwrap();
    fs::write(&host, "#!/bin/sh\ncat >/dev/null\ncat \"$0.response\"\n").unwrap();
    let mut permissions = fs::metadata(&host).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&host, permissions).unwrap();

    let artifact = rock_lib::macro_expansion::proc_macro::ProcMacroArtifact {
        artifact_format_version: rock_lib::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "integration_macros".to_string(),
        protocol_version: rock_lib::macro_expansion::proc_macro::PROC_MACRO_PROTOCOL_VERSION,
        host_triple: "x86_64-unknown-linux-gnu".to_string(),
        executable: host,
        capabilities: vec![rock_lib::macro_expansion::proc_macro::ProcMacroCapability::Stdio],
        exports: vec![rock_lib::macro_expansion::proc_macro::ProcMacroExport {
            name: "make_main".to_string(),
            identity: "integration_macros::make_main".to_string(),
            kind: rock_lib::macro_expansion::proc_macro::ProcMacroKind::FunctionLike,
            input_shape: rock_lib::macro_expansion::proc_macro::ProcMacroInputShape::TokenStream,
        }],
    };

    (artifact, temp_dir)
}

fn encode_generated_main_tokens() -> Vec<u8> {
    let stream = rock_lib::macro_expansion::TokenStream::from_bytes(
        "<proc-macro-output>".into(), b"main = -> 0\n".to_vec(),
        rock_lib::macro_expansion::ScanLimits { max_source_bytes: 65536, max_runs: 4096 },
    ).unwrap();
    rock_lib::macro_expansion::proc_macro::encode_stream(&stream).unwrap()
}

#[test]
fn test_function_like_proc_macro_expands_through_public_macro_api() {
    let config = rock_lib::Config {
        entry_file: PathBuf::new(),
        output_dir: PathBuf::new(),
        debug_print: Vec::new(),
        meta_files: Vec::new(),
        extern_artifacts: Vec::new(),
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: false,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    };
    let response = rock_lib::macro_expansion::proc_macro::ProcMacroResponse::Expand {
        output: encode_generated_main_tokens(),
    };
    let (artifact, temp_dir) = proc_macro_host_with_response(response);
    let expanded = (|| {
        let input_program = rock_lib::parser::parse_string("%integration_macros::make_main", &config).unwrap();
        let context = rock_lib::macro_expansion::MacroExpansionContext::new(&config)
            .with_proc_macro_artifact(artifact);

        rock_lib::macro_expansion::expand_macros_with_context(input_program, &context)
    })();

    let _ = fs::remove_dir_all(temp_dir);
    let expanded = expanded.unwrap();

    assert!(expanded.module.top_level_from_ident("main").is_some());
}

#[test]
fn test_type_alias_survives_collection_and_lowering() {
    let output = compile_and_run(
        r#"
type Number = Identity I64
type Identity T = T

identity: Number -> Number
identity = value -> value

main = ->
    identity 42 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_type_alias_cycle_is_rejected() {
    compile_should_fail(
        r#"
type Left = Right
type Right = Left

main = -> 0
"#,
        "type alias cycle",
    );
}
