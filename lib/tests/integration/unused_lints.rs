//! `unused_*` / `dead_code` lints: warnings on success, levels from config.

use std::sync::atomic::{AtomicU64, Ordering};

use rock_lib::diagnostic::{DiagnosticSeverity, Diagnostics};
use rock_lib::lint::LintLevel;

use crate::support::{test_config, test_temp_dir, TestDirCleanup};

static LINT_TEST_COUNTER: AtomicU64 = AtomicU64::new(1_000_000);

/// Compiles `source` (no link) with `levels` overrides and returns the
/// diagnostics: `Ok(warnings)` on success, `Err(errors)` on failure.
fn lint(source: &str, levels: &[(&str, LintLevel)]) -> Result<Diagnostics, Diagnostics> {
    lint_files(source, &[], levels)
}

/// Like [`lint`], with extra file modules (`name.rk`) next to the entry file.
fn lint_files(
    source: &str,
    modules: &[(&str, &str)],
    levels: &[(&str, LintLevel)],
) -> Result<Diagnostics, Diagnostics> {
    lint_files_named(source, modules, levels, None)
}

fn lint_files_named(
    source: &str,
    modules: &[(&str, &str)],
    levels: &[(&str, LintLevel)],
    crate_name: Option<&str>,
) -> Result<Diagnostics, Diagnostics> {
    let dir = test_temp_dir(LINT_TEST_COUNTER.fetch_add(1, Ordering::SeqCst));
    std::fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let source_path = dir.join("test.rk");
    std::fs::write(&source_path, source).unwrap();
    for (name, text) in modules {
        std::fs::write(dir.join(format!("{name}.rk")), text).unwrap();
    }
    let mut config = test_config(source_path, dir.clone());
    config.current_crate_name = crate_name.map(str::to_string);
    config.no_link = true;
    for (name, level) in levels {
        config.lint_levels.set(name, *level).unwrap();
    }
    rock_lib::compile_with_warnings(&config).map(|output| output.warnings)
}

fn messages(diagnostics: &Diagnostics) -> Vec<String> {
    diagnostics.0.iter().map(|d| d.message.clone()).collect()
}

#[test]
fn test_unused_variables_warn_for_locals_params_closures_and_patterns() {
    let warnings = lint(
        r#"
first = x, y -> x

main = !->
    t = 5
    used = 6
    f = (z -> 1)
    m = match used
        n => 2
    (first used, (f m)).println!
"#,
        &[],
    )
    .unwrap();
    assert_eq!(
        messages(&warnings),
        [
            "unused variable: `y`",
            "unused variable: `t`",
            "unused variable: `z`",
            "unused variable: `n`",
        ]
    );
    assert!(warnings
        .0
        .iter()
        .all(|warning| warning.severity == DiagnosticSeverity::Warning));
}

#[test]
fn test_underscore_prefix_and_reads_silence_unused_variables() {
    let warnings = lint(
        r#"
main = !->
    _skipped = 1
    read = 2
    captured = 3
    f = (-> captured)
    (read + (f!)).println!
"#,
        &[],
    )
    .unwrap();
    assert_eq!(messages(&warnings), Vec::<String>::new());
}

#[test]
fn test_assigned_but_never_read_local_is_unused() {
    let warnings = lint(
        r#"
main = !->
    w = 3
    w = 4
"#,
        &[],
    )
    .unwrap();
    assert_eq!(messages(&warnings), ["unused variable: `w`"]);
}

#[test]
fn test_unused_imports_warn_and_used_imports_do_not() {
    let warnings = lint(
        r#"
> std::concurrency::thread::spawn
> std::io::traits::IoError
> std::control::option::Option
> std::show::Show
> std::collections::vec::Vec

describe: T -> String where T: Show
describe = value -> value.show!

pick: Option I64 -> I64
pick = value ->
    match value
        Option::Some x => x
        Option::None => 0

main = !->
    error = IoError::InvalidInput
    (describe (pick (Option::Some 1))).println!
"#,
        &[("unused_variables", LintLevel::Allow)],
    )
    .unwrap();
    assert_eq!(
        messages(&warnings),
        [
            "unused import: `std::concurrency::thread::spawn`",
            "unused import: `std::collections::vec::Vec`",
        ]
    );
}

#[test]
fn test_unused_import_in_a_file_module_points_at_that_module() {
    let warnings = lint_files(
        "mod helpers\n\n> helpers::answer\n\nmain = !-> answer!.println!\n",
        &[(
            "helpers",
            "> std::concurrency::thread::spawn\n\n< answer = -> 42\n",
        )],
        &[],
    )
    .unwrap();
    assert_eq!(
        messages(&warnings),
        ["unused import: `std::concurrency::thread::spawn`"]
    );
    let primary = warnings.0[0].primary.as_ref().unwrap();
    assert!(primary.span.file_path.ends_with("helpers.rk"));
}

#[test]
fn test_root_import_used_only_by_a_child_module_is_used() {
    // Root imports are visible in child modules, so removing this one would
    // break `helpers`.
    let warnings = lint_files(
        "> std::io::traits::IoError\n\nmod helpers\n\n> helpers::fail\n\nmain = !->\n    error = fail!\n",
        &[("helpers", "< fail = -> IoError::InvalidInput\n")],
        &[("unused_variables", LintLevel::Allow)],
    )
    .unwrap();
    assert_eq!(messages(&warnings), Vec::<String>::new());
}

#[test]
fn test_local_parameter_closure_and_pattern_names_do_not_use_value_imports() {
    for body in [
        "choose = answer -> answer\nmain = !-> (choose 7).println!\n",
        "main = !->\n    answer = 7\n    answer.println!\n",
        "main = !->\n    callback = (answer -> answer)\n    (callback 7).println!\n",
        "main = !->\n    value = match 7\n        answer => answer\n    value.println!\n",
    ] {
        let warnings = lint_files(
            &format!("mod helpers\n> helpers::answer\n{body}"),
            &[("helpers", "< answer = -> 42\n")],
            &[("dead_code", LintLevel::Allow)],
        ).unwrap();
        assert_eq!(messages(&warnings), ["unused import: `helpers::answer`"], "{body}");
    }
}

#[test]
fn test_field_names_do_not_use_value_imports() {
    let warnings = lint_files(
        "mod helpers\n> helpers::answer\nstruct Record\n    < answer: I64\nmain = !->\n    record = Record\n        answer: 7\n    record.answer.println!\n",
        &[("helpers", "< answer = -> 42\n")],
        &[("dead_code", LintLevel::Allow)],
    ).unwrap();
    assert_eq!(messages(&warnings), ["unused import: `helpers::answer`"]);
}

#[test]
fn test_method_names_do_not_use_value_imports() {
    let warnings = lint_files(
        "mod helpers\n> helpers::answer\nstruct Record\n    < value: I64\nimpl Record\n    @answer = -> @value\nmain = !->\n    record = Record\n        value: 7\n    record.answer!.println!\n",
        &[("helpers", "< answer = -> 42\n")],
        &[("dead_code", LintLevel::Allow)],
    ).unwrap();
    assert_eq!(messages(&warnings), ["unused import: `helpers::answer`"]);
}

#[test]
fn test_qualified_reference_to_the_same_function_does_not_use_its_short_import() {
    let warnings = lint_files(
        "mod helpers\n> helpers::answer\nmain = !-> helpers::answer!.println!\n",
        &[("helpers", "< answer = -> 42\n")],
        &[],
    ).unwrap();
    assert_eq!(messages(&warnings), ["unused import: `helpers::answer`"]);
}

#[test]
fn test_value_imports_used_as_callbacks_and_inside_dead_bodies_count_as_used() {
    for body in [
        "main = !->\n    callback = answer\n    callback!.println!\n",
        "main = !->\n    callback = (-> answer!)\n    callback!.println!\n",
        "unused = -> answer!\nmain = !-> ()\n",
    ] {
        let warnings = lint_files(
            &format!("mod helpers\n> helpers::answer\n{body}"),
            &[("helpers", "< answer = -> 42\n")],
            &[("dead_code", LintLevel::Allow)],
        ).unwrap();
        assert!(warnings.0.is_empty(), "{:?}", messages(&warnings));
    }
}

#[test]
fn test_expanded_value_references_count_but_unexpanded_macro_tokens_do_not() {
    let warnings = lint_files(
        "mod helpers\n> helpers::answer\nmacro make_main\n    =>\n        main = !-> answer!.println!\n%make_main\n",
        &[("helpers", "< answer = -> 42\n")],
        &[],
    ).unwrap();
    assert!(warnings.0.is_empty(), "{:?}", messages(&warnings));
    let warnings = lint_files(
        "mod helpers\n> helpers::answer\nmacro make_main\n    =>\n        generated = -> answer!\nmain = !-> ()\n",
        &[("helpers", "< answer = -> 42\n")],
        &[("dead_code", LintLevel::Allow)],
    ).unwrap();
    assert_eq!(messages(&warnings), ["unused import: `helpers::answer`"]);
}

#[test]
fn test_a_nearer_import_of_the_same_function_does_not_use_the_outer_import() {
    let warnings = lint_files(
        "mod helpers\n> helpers::answer\nmod child\nmain = !-> child::run!.println!\n",
        &[
            ("helpers", "< answer = -> 42\n"),
            ("child", "> helpers::answer\n< run = -> answer!\n"),
        ],
        &[],
    ).unwrap();
    assert_eq!(messages(&warnings), ["unused import: `helpers::answer`"]);
    assert_eq!(warnings.0[0].primary.as_ref().unwrap().span.start, 14);
}

#[test]
fn test_a_value_import_reexport_counts_as_used() {
    let warnings = lint_files(
        "mod helpers\nmod api\nmain = !-> api::answer!.println!\n",
        &[
            ("helpers", "< answer = -> 42\n"),
            ("api", "> helpers::answer\n< answer\n"),
        ],
        &[],
    ).unwrap();
    assert!(warnings.0.is_empty(), "{:?}", messages(&warnings));
}

#[test]
fn test_named_crate_value_imports_use_the_correct_module_binding() {
    let warnings = lint_files_named(
        "mod helpers\n> helpers::answer\nmod child\nmain = !-> child::run!.println!\n",
        &[
            ("helpers", "< answer = -> 42\n"),
            ("child", "> lint_tests::helpers::answer\n< run = -> answer!\n"),
        ],
        &[],
        Some("lint_tests"),
    ).unwrap();
    assert_eq!(messages(&warnings), ["unused import: `helpers::answer`"]);
    assert_eq!(warnings.0[0].primary.as_ref().unwrap().span.start, 14);
}

#[test]
fn test_extern_imports_distinguish_callbacks_from_shadowing_locals() {
    let warnings = lint(
        "> std::ffi::libc::malloc\nmain = !->\n    malloc = 7\n    malloc.println!\n",
        &[],
    ).unwrap();
    assert_eq!(messages(&warnings), ["unused import: `std::ffi::libc::malloc`"]);
    let warnings = lint(
        "> std::ffi::libc::malloc\nmain = !->\n    _callback = malloc\n",
        &[],
    ).unwrap();
    assert!(warnings.0.is_empty(), "{:?}", messages(&warnings));
}

#[test]
fn test_shadowed_value_import_respects_allow_and_deny() {
    let source = "> std::ffi::libc::malloc\nmain = !->\n    malloc = 7\n    malloc.println!\n";
    assert!(lint(source, &[("unused_imports", LintLevel::Allow)]).unwrap().0.is_empty());
    let errors = lint(source, &[("unused_imports", LintLevel::Deny)]).unwrap_err();
    assert_eq!(messages(&errors), ["unused import: `std::ffi::libc::malloc`"]);
    assert_eq!(errors.0[0].severity, DiagnosticSeverity::Error);
}

#[test]
fn test_allowed_lint_is_silent_and_denied_lint_fails_the_build() {
    let source = "main = !->\n    t = 5\n";
    assert!(lint(source, &[("unused", LintLevel::Allow)]).unwrap().0.is_empty());
    let errors = lint(source, &[("unused_variables", LintLevel::Deny)]).unwrap_err();
    assert_eq!(messages(&errors), ["unused variable: `t`"]);
    assert_eq!(errors.0[0].severity, DiagnosticSeverity::Error);
}
