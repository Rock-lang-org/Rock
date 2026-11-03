use super::*;

#[test]
fn bounded_runner_kills_descendants_holding_pipes_open() {
    let started = Instant::now();
    let result = std::panic::catch_unwind(|| {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & wait"]);
        run_test_command_with_timeout(&mut command, Duration::from_millis(100));
    });

    assert!(result.is_err(), "timed-out command must panic");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "descendant holding stdout open must be terminated with its parent"
    );
}

#[test]
fn bounded_runner_kills_descendants_after_parent_exits() {
    let started = Instant::now();
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 2 & exit 7"]);
    let output = run_test_command_with_timeout(&mut command, Duration::from_secs(5));

    assert_eq!(output.status.code(), Some(7));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "runner must not wait for a descendant that inherited its pipes"
    );
}

#[test]
fn std_product_cache_reuses_and_repairs_artifacts() {
    let temp_dir = temp_test_dir("std_cache_repair");
    let _cleanup = TestDirCleanup(temp_dir.clone());
    let std_dir = temp_dir.join("std");
    write_answer_std(&std_dir, 7);
    let cache_dir = temp_dir.join("cache");

    let artifact = std::thread::scope(|scope| {
        let first = scope.spawn(|| cached_std_product_artifact(&std_dir, &cache_dir));
        let second = scope.spawn(|| cached_std_product_artifact(&std_dir, &cache_dir));
        let artifact = first.join().unwrap();
        assert_eq!(artifact, second.join().unwrap());
        artifact
    });
    let modified = fs::metadata(&artifact).unwrap().modified().unwrap();
    cached_std_product_artifact(&std_dir, &cache_dir);
    assert_eq!(
        modified,
        fs::metadata(&artifact).unwrap().modified().unwrap()
    );

    fs::remove_file(cache_dir.join("std.o")).unwrap();
    cached_std_product_artifact(&std_dir, &cache_dir);
    assert!(cache_dir.join("std.o").is_file());

    fs::write(&artifact, b"interrupted artifact write").unwrap();
    cached_std_product_artifact(&std_dir, &cache_dir);
    CompilerProducts::read_artifact_from_path(&artifact).unwrap();
    assert_eq!(
        write_and_run_artifact_app(
            "> std::math::answer\n\nmain = -> answer!\n",
            vec![("std".to_string(), artifact)],
        ),
        7
    );
}
