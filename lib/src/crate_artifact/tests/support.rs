use super::*;

pub(super) static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) const TEST_PROCESS_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

pub(super) fn temp_test_dir(name: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "rock_artifact_{}_{}_{}",
        name,
        std::process::id(),
        id
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) struct TestDirCleanup(pub(super) PathBuf);

impl Drop for TestDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn child_exited_without_reaping(child: &Child) -> Result<bool, std::io::Error> {
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

pub(super) fn terminate_test_command_group(child: &Child) -> Result<(), std::io::Error> {
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

pub(super) fn terminate_and_collect_test_command(
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

pub(super) fn run_test_command(command: &mut Command) -> Output {
    run_test_command_with_timeout(command, TEST_PROCESS_TIMEOUT)
}

pub(super) fn run_test_command_with_timeout(command: &mut Command, timeout: Duration) -> Output {
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
            Ok(false) if started.elapsed() < timeout => {
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
                    timeout,
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

pub(super) fn write_crate(dir: &PathBuf, name: &str, dependencies: &str, lib: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("rock.toml"),
        format!(
            "[crate]\nname = \"{}\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"lib.rk\"\n{}",
            name, dependencies
        ),
    )
    .unwrap();
    fs::write(dir.join("lib.rk"), lib).unwrap();
}

pub(super) fn write_answer_std(dir: &PathBuf, answer: i32) {
    write_crate(dir, "std", "", "< mod prelude\n< mod math\n");
    fs::write(dir.join("prelude.rk"), "< std::math::answer\n").unwrap();
    fs::write(
        dir.join("math.rk"),
        format!("< answer = -> {}\n< answer\n", answer),
    )
    .unwrap();
}

pub(super) fn delete_crate_source_files(crate_dir: &PathBuf) {
    fn remove_rk_files(path: &std::path::Path) {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    if entry_path.file_name().and_then(|name| name.to_str()) == Some("build") {
                        continue;
                    }
                    remove_rk_files(&entry_path);
                } else if entry_path.extension().and_then(|ext| ext.to_str()) == Some("rk") {
                    let _ = fs::remove_file(entry_path);
                }
            }
        }
    }

    remove_rk_files(crate_dir);
    let _ = fs::remove_file(crate_dir.join("rock.toml"));
}

pub(super) fn build_product_artifact(
    crate_dir: &PathBuf,
    crate_name: &str,
    artifact_path: &PathBuf,
    no_std: bool,
    no_prelude: bool,
) -> PathBuf {
    build_product_artifact_with_extern_artifacts(
        crate_dir,
        crate_name,
        artifact_path,
        no_std,
        no_prelude,
        Vec::new(),
    )
}

pub(super) fn build_product_artifact_with_extern_artifacts(
    crate_dir: &PathBuf,
    crate_name: &str,
    artifact_path: &PathBuf,
    no_std: bool,
    no_prelude: bool,
    extern_artifacts: Vec<(String, PathBuf)>,
) -> PathBuf {
    let manifest = CrateContext::load_manifest(&crate_dir.join("rock.toml")).unwrap();
    let entry_file = crate_dir.join(manifest.primary_target().path());
    let output_dir = crate_dir.join("build").join("products");
    let object_path = output_dir.join(format!("{}.o", crate_name));
    fs::create_dir_all(&output_dir).unwrap();

    let output = crate::compile_with_products(&Config {
        entry_file,
        output_dir,
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts,
        source_providers: Vec::new(),
        current_crate_name: Some(crate_name.to_string()),
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: true,
        emit_object: Some(object_path.clone()),
        no_prelude,
        no_std,
        sysroot: None,
    })
    .unwrap();

    output
        .products
        .unwrap()
        .write_artifact_to_path(artifact_path)
        .unwrap();

    object_path
}

pub(super) fn collect_index_authorities_from_block(
    block: &crate::hir::AcceptedHirBlock,
    targets: &mut Vec<crate::hir::HirMethodCallTarget>,
) {
    for stmt in &block.stmts {
        match stmt {
            HirStmt::Let { value, .. }
            | HirStmt::Expr(value)
            | HirStmt::Return(Some(value))
            | HirStmt::Break(Some(value)) => collect_index_authorities(value, targets),
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
        }
    }
}

pub(super) fn collect_index_authorities(
    expr: &AcceptedHirExpr,
    targets: &mut Vec<crate::hir::HirMethodCallTarget>,
) {
    match &expr.kind {
        HirExprKind::MethodCall(receiver, _, args, _, target) => {
            targets.push(target.clone());
            collect_index_authorities(receiver, targets);
            for arg in args {
                collect_index_authorities(arg, targets);
            }
        }
        HirExprKind::Deref(inner)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Cast(inner, _)
        | HirExprKind::UnaryOp(_, inner) => collect_index_authorities(inner, targets),
        HirExprKind::Assign(lhs, rhs) | HirExprKind::BinOp(_, lhs, rhs) => {
            collect_index_authorities(lhs, targets);
            collect_index_authorities(rhs, targets);
        }
        HirExprKind::Call(callee, args, _) => {
            collect_index_authorities(callee, targets);
            for arg in args {
                collect_index_authorities(arg, targets);
            }
        }
        HirExprKind::ArrayLiteral(elements) | HirExprKind::TupleLiteral(elements) => {
            for element in elements {
                collect_index_authorities(element, targets);
            }
        }
        HirExprKind::ArrayRepeat(value, _) => collect_index_authorities(value, targets),
        HirExprKind::Intrinsic { args, .. } => {
            for arg in args {
                collect_index_authorities(arg, targets);
            }
        }
        HirExprKind::Block(body) | HirExprKind::Loop(body) | HirExprKind::UnsafeBlock(body) => {
            collect_index_authorities_from_block(body, targets);
        }
        _ => {}
    }
}

pub(super) fn collect_index_authorities_from_function(
    function: &AcceptedHirFunction,
) -> Vec<crate::hir::HirMethodCallTarget> {
    let mut targets = Vec::new();
    collect_index_authorities_from_block(&function.body, &mut targets);
    targets
}

pub(super) fn shared_std_product_artifact() -> PathBuf {
    static STD_ARTIFACT: OnceLock<PathBuf> = OnceLock::new();

    STD_ARTIFACT
        .get_or_init(|| {
            let std_dir = workspace_root().join("std");
            let key = std_cache_key(&std_dir, &std_cache_compiler_stamp());
            let artifact_dir = std::env::temp_dir().join(format!("rock_artifact_std_{key}"));
            cached_std_product_artifact(&std_dir, &artifact_dir)
        })
        .clone()
}

pub(super) fn cached_std_product_artifact(std_dir: &PathBuf, artifact_dir: &PathBuf) -> PathBuf {
    // Keep the lock outside the payload directory; the OS releases it on exit,
    // including a crashed test process, without stale-lock polling.
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(artifact_dir.with_extension("lock"))
        .unwrap();
    lock.lock().unwrap();
    let artifact_path = artifact_dir.join("std.rkca");
    let object_path = artifact_dir.join("std.o");
    if object_path.is_file() && CompilerProducts::read_artifact_from_path(&artifact_path).is_ok() {
        return artifact_path;
    }

    fs::create_dir_all(artifact_dir).unwrap();
    match fs::remove_file(&artifact_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("failed to invalidate cached std artifact: {error}"),
    }
    // Build in the private cache, never in the checked-out std/build directory.
    let manifest = rock_shared::manifest::load_manifest(&std_dir.join("rock.toml")).unwrap();
    let output = crate::compile_with_products(&Config {
        entry_file: std_dir.join(manifest.primary_target().path()),
        output_dir: artifact_dir.clone(),
        current_crate_name: Some("std".to_string()),
        no_std: true,
        no_link: true,
        emit_object: Some(object_path),
        ..Config::default()
    })
    .unwrap();
    let mut products = output.products.unwrap();
    products.link.object_path = Some(PathBuf::from("std.o"));
    let pending = artifact_dir.join("std.rkca.pending");
    products.write_artifact_to_path(&pending).unwrap();
    CompilerProducts::read_artifact_from_path(&pending).unwrap();
    fs::rename(pending, &artifact_path).unwrap();
    artifact_path
}

pub(super) fn compile_with_options(
    entry_file: PathBuf,
    output_dir: PathBuf,
    extern_artifacts: Vec<(String, PathBuf)>,
    sysroot: Option<PathBuf>,
    no_std: bool,
    no_prelude: bool,
) -> PathBuf {
    let executable = if cfg!(target_os = "windows") {
        output_dir.join(format!(
            "{}.exe",
            entry_file
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("main")
        ))
    } else {
        output_dir.join(
            entry_file
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("main"),
        )
    };

    let config = Config {
        entry_file,
        output_dir: output_dir.clone(),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts,
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: false,
        emit_object: None,
        no_prelude,
        no_std,
        sysroot,
    };

    crate::compile(&config).unwrap();
    executable
}

pub(super) fn write_and_run_artifact_app_with_options(
    source: &str,
    extern_artifacts: Vec<(String, PathBuf)>,
    sysroot: Option<PathBuf>,
    no_std: bool,
    no_prelude: bool,
) -> i32 {
    let temp_dir = temp_test_dir("artifact_app");
    let _cleanup = TestDirCleanup(temp_dir.clone());
    let entry_file = temp_dir.join("main.rk");
    fs::write(&entry_file, source).unwrap();
    let executable = compile_with_options(
        entry_file,
        temp_dir.join("out"),
        extern_artifacts,
        sysroot,
        no_std,
        no_prelude,
    );
    let mut command = Command::new(&executable);
    let output = run_test_command(&mut command);
    output.status.code().unwrap_or(1)
}

pub(super) fn write_and_run_artifact_app(
    source: &str,
    extern_artifacts: Vec<(String, PathBuf)>,
) -> i32 {
    write_and_run_artifact_app_with_options(source, extern_artifacts, None, false, false)
}

pub(super) fn compile_and_run_artifact_app(
    source: &str,
    extern_artifacts: Vec<(String, PathBuf)>,
) -> (String, i32) {
    let temp_dir = temp_test_dir("artifact_no_std_app");
    let _cleanup = TestDirCleanup(temp_dir.clone());
    let entry_file = temp_dir.join("main.rk");
    fs::write(&entry_file, source).unwrap();
    let executable = compile_with_options(
        entry_file,
        temp_dir.join("out"),
        extern_artifacts,
        None,
        true,
        true,
    );
    let mut command = Command::new(executable);
    let output = run_test_command(&mut command);

    (
        String::from_utf8(output.stdout).expect("artifact app stdout must be UTF-8"),
        output.status.code().unwrap_or(1),
    )
}
