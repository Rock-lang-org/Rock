use super::*;

#[test]
fn test_print_target_triple_does_not_require_entry_file() {
    let config = Config::try_parse_from(["rockc", "--print", "target-triple"]).unwrap();

    assert_eq!(
        print_request_output(&config).unwrap(),
        Some(rock_lib::sysroot::host_target_triple())
    );
}

#[test]
fn test_print_target_libdir_uses_sysroot_override() {
    let config = Config::try_parse_from([
        "rockc",
        "--sysroot",
        "/tmp/rock-toolchain",
        "--print",
        "target-libdir",
    ])
    .unwrap();
    let expected = rock_lib::sysroot::SysrootLayout::new(
        PathBuf::from("/tmp/rock-toolchain"),
        rock_lib::sysroot::host_target_triple(),
    )
    .target_libdir
    .display()
    .to_string();

    assert_eq!(print_request_output(&config).unwrap(), Some(expected));
}

#[test]
fn test_build_profiles() {
    for (flags, expected) in [(vec![], 0), (vec!["--debug"], 0), (vec!["--release"], 3)] {
        let mut args = vec!["rockc", "--entry-file", "main.rk"];
        args.extend(flags);
        let config = Config::try_parse_from(args)
            .unwrap()
            .into_compiler_config()
            .unwrap();
        assert_eq!(config.opt_level, expected);
    }
    for flags in [vec!["--debug", "--release"], vec!["--release", "--debug"]] {
        let mut args = vec!["rockc", "--entry-file", "main.rk"];
        args.extend(flags);
        assert!(Config::try_parse_from(args).is_err());
    }
}

#[test]
fn test_overflow_checks_are_independent_of_build_profile() {
    for (profile, expected_opt) in [("--debug", 0), ("--release", 3)] {
        for (flag, expected) in [("true", true), ("false", false)] {
            let config = Config::try_parse_from([
                "rockc",
                "--entry-file",
                "main.rk",
                profile,
                "--overflow-checks",
                flag,
            ])
            .unwrap()
            .into_compiler_config()
            .unwrap();
            assert_eq!(config.opt_level, expected_opt);
            assert_eq!(config.overflow_checks, Some(expected));
        }
    }
    assert!(Config::try_parse_from(["rockc", "--overflow-checks", "invalid"]).is_err());
}

#[test]
fn test_numeric_optimization_flags_are_unknown() {
    for flags in [
        vec!["-O"],
        vec!["-O", "0"],
        vec!["-O", "3"],
        vec!["-O0"],
        vec!["-O1"],
        vec!["-O2"],
        vec!["-O3"],
        vec!["--opt-level"],
        vec!["--opt-level", "3"],
        vec!["--opt-level=0"],
        vec!["--opt-level=3"],
    ] {
        let mut args = vec!["rockc", "--entry-file", "main.rk"];
        args.extend(flags);
        let error = Config::try_parse_from(args).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }
    let help = <Config as clap::CommandFactory>::command()
        .render_long_help()
        .to_string();
    assert!(!help.contains("-O"));
    assert!(!help.contains("--opt-level"));
}

#[test]
fn test_build_profile_flags_compile_unit_main_with_zero_exit_status() {
    let dir = artifact_deps_test_dir("profile_unit_main");
    let entry = dir.join("main.rk");
    std::fs::write(
        &entry,
        "extern putchar: I32 -> I32\nmain = !-> putchar (65 as I32)\n",
    )
    .unwrap();
    for flag in [None, Some("--debug"), Some("--release")] {
        let output_dir = dir.join(flag.unwrap_or("default").trim_start_matches('-'));
        let mut args = vec![
            "rockc".to_string(),
            "--entry-file".to_string(),
            entry.display().to_string(),
            "--output-dir".to_string(),
            output_dir.display().to_string(),
            "--no-std".to_string(),
            "--no-prelude".to_string(),
        ];
        if let Some(flag) = flag {
            args.push(flag.to_string());
        }
        let config = Config::try_parse_from(args)
            .unwrap()
            .into_compiler_config()
            .unwrap();
        rock_lib::compile(&config).unwrap();
        let output = std::process::Command::new(output_dir.join("main"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "profile {flag:?}");
        assert_eq!(output.stdout, b"A", "profile {flag:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn test_entry_file_is_required_for_compile_mode() {
    let config = Config::try_parse_from(["rockc"]).unwrap();

    assert!(config.into_compiler_config().is_err());
}

#[test]
fn test_extern_artifact_parses() {
    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        "main.rk",
        "--extern-artifact",
        "dep=/tmp/dep.rkca",
    ])
    .unwrap();

    assert_eq!(
        config.extern_artifact,
        vec!["dep=/tmp/dep.rkca".to_string()]
    );
}

#[test]
fn test_malformed_extern_artifact_is_rejected_without_panic() {
    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        "main.rk",
        "--extern-artifact",
        "dep",
    ])
    .unwrap();

    assert!(config.into_compiler_config().is_err());
}

#[test]
fn test_extern_product_artifact_is_rejected() {
    let error = Config::try_parse_from([
        "rockc",
        "--entry-file",
        "main.rk",
        "--extern-product-artifact",
        "dep=build/dep.rkca",
    ])
    .unwrap_err();

    assert!(error.to_string().contains("unexpected argument"));
}

#[test]
fn test_positional_artifacts_are_rejected() {
    let error = Config::try_parse_from(["rockc", "--entry-file", "main.rk", "dep=/tmp/dep.rkca"])
        .unwrap_err();

    assert!(error.to_string().contains("unexpected argument"));
}

#[test]
fn test_emit_artifact_and_object_parse() {
    let config = Config::try_parse_from([
        "rockc",
        "--entry-file",
        "main.rk",
        "--emit-artifact",
        "build/main.rkca",
        "--emit-object",
        "build/main.o",
        "--crate-name",
        "demo",
    ])
    .unwrap();

    assert_eq!(config.emit_artifact, Some(PathBuf::from("build/main.rkca")));
    assert_eq!(config.emit_object, Some(PathBuf::from("build/main.o")));
    assert_eq!(config.crate_name, Some("demo".to_string()));
}

#[test]
fn test_removed_dependency_flags_are_rejected() {
    assert!(Config::try_parse_from([
        "rockc",
        "--entry-file",
        "main.rk",
        "--extern-crate",
        "dep=/tmp/dep.rk",
    ])
    .is_err());

    assert!(Config::try_parse_from([
        "rockc",
        "--entry-file",
        "main.rk",
        "--crate-path",
        "/tmp/dep",
    ])
    .is_err());
}
