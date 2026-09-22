use std::fs;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::crate_system::CrateContext;
use crate::hir::{
    AcceptedHirExpr, AcceptedHirFunction, HirExprKindFor as HirExprKind, HirSelectedMethodTarget,
    HirStmtFor as HirStmt,
};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::products::{CompilerProducts, ProductDefId, ProductLanguageItems};
use crate::types::Type;
use crate::Config;

#[path = "../../tests/support/stdlib_cache_key.rs"]
mod stdlib_cache_key;
use stdlib_cache_key::{stdlib_cache_compiler_stamp, stdlib_cache_key};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
const TEST_PROCESS_TIMEOUT: Duration = Duration::from_secs(15);

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn temp_test_dir(name: &str) -> PathBuf {
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
    run_test_command_with_timeout(command, TEST_PROCESS_TIMEOUT)
}

fn run_test_command_with_timeout(command: &mut Command, timeout: Duration) -> Output {
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

fn write_crate(dir: &PathBuf, name: &str, dependencies: &str, lib: &str) {
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

fn write_answer_stdlib(dir: &PathBuf, answer: i32) {
    write_crate(dir, "stdlib", "", "< mod prelude\n< mod math\n");
    fs::write(dir.join("prelude.rk"), "< stdlib::math::answer\n").unwrap();
    fs::write(
        dir.join("math.rk"),
        format!("< answer = -> {}\n< answer\n", answer),
    )
    .unwrap();
}

fn delete_crate_source_files(crate_dir: &PathBuf) {
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

fn build_product_artifact(
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

fn build_product_artifact_with_extern_artifacts(
    crate_dir: &PathBuf,
    crate_name: &str,
    artifact_path: &PathBuf,
    no_std: bool,
    no_prelude: bool,
    extern_artifacts: Vec<(String, PathBuf)>,
) -> PathBuf {
    let manifest = CrateContext::load_manifest(&crate_dir.join("rock.toml")).unwrap();
    let entry_file = crate_dir.join(&manifest.lib.path);
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

fn build_stdlib_product_artifact(stdlib_dir: &PathBuf, artifact_path: &PathBuf) -> PathBuf {
    build_product_artifact(stdlib_dir, "stdlib", artifact_path, true, false)
}

fn collect_index_authorities_from_block(
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

fn collect_index_authorities(
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

fn collect_index_authorities_from_function(
    function: &AcceptedHirFunction,
) -> Vec<crate::hir::HirMethodCallTarget> {
    let mut targets = Vec::new();
    collect_index_authorities_from_block(&function.body, &mut targets);
    targets
}

fn shared_stdlib_product_artifact() -> PathBuf {
    static STDLIB_ARTIFACT: OnceLock<PathBuf> = OnceLock::new();

    STDLIB_ARTIFACT
        .get_or_init(|| {
            let stdlib_dir = workspace_root().join("stdlib");
            let key = stdlib_cache_key(&stdlib_dir, &stdlib_cache_compiler_stamp());
            let artifact_dir = std::env::temp_dir().join(format!("rock_artifact_stdlib_{key}"));
            cached_stdlib_product_artifact(&stdlib_dir, &artifact_dir)
        })
        .clone()
}

fn cached_stdlib_product_artifact(stdlib_dir: &PathBuf, artifact_dir: &PathBuf) -> PathBuf {
    // Keep the lock outside the payload directory; the OS releases it on exit,
    // including a crashed test process, without stale-lock polling.
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(artifact_dir.with_extension("lock"))
        .unwrap();
    lock.lock().unwrap();
    let artifact_path = artifact_dir.join("stdlib.rkca");
    let object_path = artifact_dir.join("stdlib.o");
    if object_path.is_file() && CompilerProducts::read_artifact_from_path(&artifact_path).is_ok() {
        return artifact_path;
    }

    fs::create_dir_all(artifact_dir).unwrap();
    match fs::remove_file(&artifact_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("failed to invalidate cached stdlib artifact: {error}"),
    }
    // Build in the private cache, never in the checked-out stdlib/build directory.
    let output = crate::compile_with_products(&Config {
        entry_file: stdlib_dir.join("lib.rk"),
        output_dir: artifact_dir.clone(),
        current_crate_name: Some("stdlib".to_string()),
        no_std: true,
        no_link: true,
        emit_object: Some(object_path),
        ..Config::default()
    })
    .unwrap();
    let mut products = output.products.unwrap();
    products.link.object_path = Some(PathBuf::from("stdlib.o"));
    let pending = artifact_dir.join("stdlib.rkca.pending");
    products.write_artifact_to_path(&pending).unwrap();
    CompilerProducts::read_artifact_from_path(&pending).unwrap();
    fs::rename(pending, &artifact_path).unwrap();
    artifact_path
}

#[test]
fn stdlib_product_cache_reuses_and_repairs_artifacts() {
    let temp_dir = temp_test_dir("stdlib_cache_repair");
    let _cleanup = TestDirCleanup(temp_dir.clone());
    let stdlib_dir = temp_dir.join("stdlib");
    write_answer_stdlib(&stdlib_dir, 7);
    let cache_dir = temp_dir.join("cache");

    let artifact = std::thread::scope(|scope| {
        let first = scope.spawn(|| cached_stdlib_product_artifact(&stdlib_dir, &cache_dir));
        let second = scope.spawn(|| cached_stdlib_product_artifact(&stdlib_dir, &cache_dir));
        let artifact = first.join().unwrap();
        assert_eq!(artifact, second.join().unwrap());
        artifact
    });
    let modified = fs::metadata(&artifact).unwrap().modified().unwrap();
    cached_stdlib_product_artifact(&stdlib_dir, &cache_dir);
    assert_eq!(
        modified,
        fs::metadata(&artifact).unwrap().modified().unwrap()
    );

    fs::remove_file(cache_dir.join("stdlib.o")).unwrap();
    cached_stdlib_product_artifact(&stdlib_dir, &cache_dir);
    assert!(cache_dir.join("stdlib.o").is_file());

    fs::write(&artifact, b"interrupted artifact write").unwrap();
    cached_stdlib_product_artifact(&stdlib_dir, &cache_dir);
    CompilerProducts::read_artifact_from_path(&artifact).unwrap();
    assert_eq!(
        write_and_run_artifact_app(
            "> stdlib::math::answer\n\nmain = -> answer!\n",
            vec![("stdlib".to_string(), artifact)],
        ),
        7
    );
}

fn write_sysroot_stdlib_product_bundle(stdlib_dir: &PathBuf, sysroot: &PathBuf) -> PathBuf {
    let layout =
        crate::sysroot::SysrootLayout::new(sysroot.clone(), crate::sysroot::host_target_triple());
    fs::create_dir_all(&layout.target_libdir).unwrap();

    let artifact_path = layout.stdlib_artifact.clone();
    let object_path = build_stdlib_product_artifact(stdlib_dir, &artifact_path);
    fs::copy(object_path, &layout.stdlib_object).unwrap();

    artifact_path
}

fn compile_with_options(
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

fn write_and_run_artifact_app_with_options(
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

fn write_and_run_artifact_app(source: &str, extern_artifacts: Vec<(String, PathBuf)>) -> i32 {
    write_and_run_artifact_app_with_options(source, extern_artifacts, None, false, false)
}

fn compile_and_run_artifact_app(
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

fn compile_artifact_app_without_stdlib_failure(
    source: &str,
    extern_artifacts: Vec<(String, PathBuf)>,
) -> crate::diagnostic::Diagnostics {
    let temp_dir = temp_test_dir("artifact_no_std_failure");
    let _cleanup = TestDirCleanup(temp_dir.clone());
    let entry_file = temp_dir.join("main.rk");
    fs::write(&entry_file, source).unwrap();

    crate::compile(&Config {
        entry_file,
        output_dir: temp_dir.join("out"),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts,
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: true,
        no_std: true,
        sysroot: None,
    })
    .expect_err("no-stdlib compilation should fail")
}

#[test]
fn test_compile_with_stdlib_artifact() {
    let hello_path = workspace_root().join("examples").join("hello.rk");
    let temp_dir = temp_test_dir("stdlib_artifact_compile");
    let artifact_path = shared_stdlib_product_artifact();
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    crate::compile(&Config {
        entry_file: hello_path,
        output_dir: temp_dir.join("out"),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![("stdlib".to_string(), artifact_path.clone())],
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    })
    .unwrap();

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn stdlib_index_selection_persists_exact_impl_and_trait_authority() {
    let temp_dir = temp_test_dir("stdlib_index_authority");
    let entry_file = temp_dir.join("main.rk");
    fs::write(
        &entry_file,
        r#"
to_slice: &[I64] -> &[I64]
to_slice = slice -> slice

struct Probe T
impl Probe T
    @probe = ->
        mut arr: [I64; 3] = [1, 2, 3]
        first = arr[0]
        slice = to_slice &arr
        second = slice[1]
        ptr: *I64 = (~ArrPtr slice) as *I64
        third = unsafe ptr[2]
        arr[0] = 9
        0

main = ->
    0
"#,
    )
    .unwrap();
    let artifact_path = shared_stdlib_product_artifact();
    let output = crate::compile_with_products(&Config {
        entry_file,
        output_dir: temp_dir.join("out"),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![("stdlib".to_string(), artifact_path)],
        source_providers: Vec::new(),
        current_crate_name: Some("app".to_string()),
        opt_level: 0,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    })
    .expect("stdlib-backed indexing should lower");
    let products = output.products.expect("products should be emitted");
    let stdlib_products =
        CompilerProducts::read_artifact_from_path(&shared_stdlib_product_artifact())
            .expect("stdlib artifact should load for authority assertions");
    let probe = products
        .bodies
        .generic_impls
        .values()
        .flat_map(|imp| imp.methods.values())
        .find(|method| method.name == "probe")
        .expect("generic probe body should be present");
    let targets = collect_index_authorities_from_function(probe);
    let index = stdlib_products
        .interface
        .language_items
        .index
        .as_ref()
        .expect("Index provider should be loaded");
    let index_mut = stdlib_products
        .interface
        .language_items
        .index_mut
        .as_ref()
        .expect("IndexMut provider should be loaded");
    let remap_external = |crate_id: crate::ids::CrateId, id: ProductDefId| {
        DefId::new(crate_id, LocalDefId(id.local_id.0))
    };
    let remap_interface_def =
        |crate_id: crate::ids::CrateId, id: DefId| DefId::new(crate_id, id.local);

    let mut index_count = 0;
    let mut index_mut_count = 0;
    for target in targets {
        let expected_trait_id = match &target.target {
            HirSelectedMethodTarget::ImplMethod {
                method_id,
                selected_trait: Some(selected_trait),
                ..
            } => {
                assert_eq!(target.method_id(), Some(*method_id));
                selected_trait.trait_id
            }
            _ => panic!("index method must persist concrete impl authority"),
        };
        assert_eq!(target.trait_id(), Some(expected_trait_id));
        let HirSelectedMethodTarget::ImplMethod {
            impl_id,
            method_id,
            selected_trait: Some(selected_trait),
        } = target.target
        else {
            panic!("index method must persist concrete impl authority");
        };
        let external_crate_id = selected_trait.trait_id.crate_id;
        let impl_def = stdlib_products
            .interface
            .impls
            .values()
            .find(|candidate| remap_interface_def(external_crate_id, candidate.id) == impl_id)
            .expect("selected impl must be present in the loaded product");
        assert_eq!(
            impl_def
                .trait_id
                .map(|id| remap_interface_def(external_crate_id, id)),
            Some(selected_trait.trait_id)
        );
        assert!(impl_def
            .methods
            .values()
            .any(|method| remap_interface_def(external_crate_id, method.id) == method_id));

        if selected_trait.trait_id == remap_external(external_crate_id, index.trait_id) {
            index_count += 1;
            assert_eq!(
                selected_trait.member_id,
                remap_external(external_crate_id, index.method_id)
            );
        } else if selected_trait.trait_id == remap_external(external_crate_id, index_mut.trait_id) {
            index_mut_count += 1;
            assert_eq!(
                selected_trait.member_id,
                remap_external(external_crate_id, index_mut.method_id)
            );
        } else {
            panic!("index authority selected an unrelated trait");
        }
    }

    assert_eq!((index_count, index_mut_count), (3, 1));
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn stdlib_product_artifact_provides_index_mut_language_items() {
    let artifact_path = shared_stdlib_product_artifact();
    let products = CompilerProducts::read_artifact_from_path(&artifact_path)
        .expect("shared stdlib product artifact should load");
    let language_items = &products.interface.language_items;

    assert!(
        language_items.index.is_some(),
        "stdlib artifact must provide Index"
    );
    assert!(
        language_items.index_mut.is_some(),
        "stdlib artifact must provide IndexMut"
    );
}

#[test]
fn stdlib_product_artifact_provides_complete_language_item_registry() {
    let artifact_path = shared_stdlib_product_artifact();
    let products = CompilerProducts::read_artifact_from_path(&artifact_path)
        .expect("shared stdlib product artifact should load");
    let product_sized = products
        .interface
        .language_items
        .sized
        .as_ref()
        .expect("stdlib product must provide Sized");
    let product_drop = products
        .interface
        .language_items
        .drop
        .as_ref()
        .expect("stdlib product must provide Drop");
    let product_index = products
        .interface
        .language_items
        .index
        .as_ref()
        .expect("stdlib product must provide Index");
    let product_try = products
        .interface
        .language_items
        .try_protocol
        .as_ref()
        .expect("stdlib product must provide Try");

    let mut crate_ctx = CrateContext::new();
    crate_ctx
        .load_product_artifact_from_path_as("stdlib", artifact_path)
        .expect("stdlib artifact should load into a fresh crate context");
    let runtime_stdlib = crate_ctx
        .extern_crate("stdlib")
        .expect("fresh context should contain stdlib");
    let runtime_crate_id = runtime_stdlib.crate_id();
    let runtime_language_items = runtime_stdlib.metadata().language_items();
    let runtime_sized = runtime_language_items
        .sized
        .as_ref()
        .expect("loaded stdlib must provide Sized");
    let runtime_drop = runtime_language_items
        .drop
        .as_ref()
        .expect("loaded stdlib must provide Drop");
    let runtime_index = runtime_language_items
        .index
        .as_ref()
        .expect("loaded stdlib must provide Index");
    let runtime_try = runtime_language_items
        .try_protocol
        .as_ref()
        .expect("loaded stdlib must provide Try");
    let remapped = |id: ProductDefId| DefId::new(runtime_crate_id, LocalDefId(id.local_id.0));

    assert_eq!(runtime_sized.trait_id, remapped(product_sized.trait_id));
    assert_eq!(runtime_drop.trait_id, remapped(product_drop.trait_id));
    assert_eq!(runtime_drop.method_id, remapped(product_drop.method_id));
    assert_eq!(runtime_index.trait_id, remapped(product_index.trait_id));
    assert_eq!(runtime_index.method_id, remapped(product_index.method_id));
    assert_eq!(runtime_try.try_trait_id, remapped(product_try.try_trait_id));
    assert_eq!(
        runtime_try.branch_method_id,
        remapped(product_try.branch_method_id)
    );
    assert_eq!(
        runtime_try.from_residual_trait_id,
        remapped(product_try.from_residual_trait_id)
    );
    assert_eq!(
        runtime_try.from_residual_method_id,
        remapped(product_try.from_residual_method_id)
    );
    assert_eq!(
        runtime_try.control_flow_enum_id,
        remapped(product_try.control_flow_enum_id)
    );

    assert_eq!(runtime_index.output_id, product_index.output_id);
    assert_eq!(runtime_try.output_id, product_try.output_id);
    assert_eq!(runtime_try.residual_id, product_try.residual_id);
    assert_eq!(runtime_try.break_variant_id, product_try.break_variant_id);
    assert_eq!(
        runtime_try.continue_variant_id,
        product_try.continue_variant_id
    );

    let interface = runtime_stdlib.metadata().interface();
    assert!(interface.traits.contains_key(&runtime_sized.trait_id));
    for (trait_id, assoc_type_ids, member_id) in [
        (runtime_drop.trait_id, &[][..], runtime_drop.method_id),
        (
            runtime_index.trait_id,
            std::slice::from_ref(&runtime_index.output_id),
            runtime_index.method_id,
        ),
        (
            runtime_try.try_trait_id,
            &[runtime_try.output_id, runtime_try.residual_id],
            runtime_try.branch_method_id,
        ),
        (
            runtime_try.from_residual_trait_id,
            &[][..],
            runtime_try.from_residual_method_id,
        ),
    ] {
        let trait_def = interface
            .traits
            .get(&trait_id)
            .expect("language-item trait must be declared by the loaded interface");
        assert!(assoc_type_ids.iter().all(|id| {
            trait_def
                .associated_types
                .iter()
                .any(|assoc| assoc.id == *id)
        }));
        assert!(
            trait_def
                .methods
                .values()
                .any(|method| method.id == member_id)
                || trait_def
                    .signatures
                    .values()
                    .any(|signature| signature.id == member_id)
        );
    }
    let control_flow = interface
        .enums
        .get(&runtime_try.control_flow_enum_id)
        .expect("ControlFlow must be declared by the loaded interface");
    assert!(control_flow
        .variants
        .iter()
        .any(|variant| variant.id == runtime_try.break_variant_id));
    assert!(control_flow
        .variants
        .iter()
        .any(|variant| variant.id == runtime_try.continue_variant_id));
}

#[test]
fn test_run_with_stdlib_product_artifact_links() {
    let temp_dir = temp_test_dir("stdlib_artifact_methods");
    let artifact_path = shared_stdlib_product_artifact();
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app(
        "main = -> 40 + 2\n",
        vec![("stdlib".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_stdlib_artifact_preserves_string_method_abi_and_prelude_exports() {
    let artifact_path = shared_stdlib_product_artifact();
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    assert_eq!(products.crate_identity.name, "stdlib");
    let sqrt_id = products.identity_table.prelude_export_names["sqrt"];
    let deref_id = products.identity_table.prelude_export_names["Deref"];
    assert_eq!(
        products
            .identity_table
            .display_names
            .get(&sqrt_id)
            .map(String::as_str),
        Some("stdlib::libc::sqrt")
    );
    assert_eq!(
        products
            .identity_table
            .display_names
            .get(&deref_id)
            .map(String::as_str),
        Some("stdlib::deref::Deref")
    );
    assert_eq!(products.infix_precedence.get("+"), Some(&9));
    assert_eq!(products.infix_precedence.get("*"), Some(&10));

    let string_impls = products
        .interface
        .impls
        .values()
        .filter(|imp| imp.type_name == "String" && imp.trait_name.is_none())
        .collect::<Vec<_>>();
    let from_str = string_impls
        .iter()
        .find_map(|imp| imp.methods.get("from_str"))
        .unwrap();
    let concat = string_impls
        .iter()
        .find_map(|imp| imp.methods.get("concat"))
        .unwrap();
    let expected_string_ty = from_str.ret_type.clone();

    assert!(matches!(expected_string_ty, Type::Struct { .. }));
    assert_eq!(concat.ret_type, expected_string_ty);
    assert_eq!(
        from_str.params[0],
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        }
    );
    assert_eq!(concat.params[1], from_str.ret_type);
    assert!(products
        .interface
        .traits
        .values()
        .any(|trait_| trait_.name == "stdlib::deref::Deref"));
}

#[test]
fn test_product_stdlib_artifact_records_static_impl_method_link_symbol_by_product_id() {
    let artifact_path = shared_stdlib_product_artifact();
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let string_impl = products
        .interface
        .impls
        .values()
        .filter(|imp| imp.type_name == "String" && imp.trait_name.is_none())
        .find(|imp| imp.methods.contains_key("from_str"))
        .unwrap();
    let from_str = &string_impl.methods["from_str"];
    let from_str_id = ProductDefId::from(from_str.id);
    assert!(!products.interface.functions.contains_key(&from_str_id));
    assert!(!products.bodies.functions.contains_key(&from_str_id));
    let record = products
        .link
        .records
        .get(&from_str_id)
        .unwrap_or_else(|| panic!("missing from_str link record for {from_str_id:?}"));
    assert!(record.backend_symbol.starts_with("__rock_"));
}

#[test]
fn test_product_artifacts_mangle_duplicate_exported_backend_symbols_by_crate() {
    let temp_dir = temp_test_dir("duplicate_exported_backend_symbols");
    let first_dir = temp_dir.join("first");
    let second_dir = temp_dir.join("second");
    let first_artifact = temp_dir.join("first.rkca");
    let second_artifact = temp_dir.join("second.rkca");

    write_crate(&first_dir, "first", "", "< helper = -> 11\n< helper\n");
    write_crate(&second_dir, "second", "", "< helper = -> 22\n< helper\n");

    build_product_artifact(&first_dir, "first", &first_artifact, true, true);
    build_product_artifact(&second_dir, "second", &second_artifact, true, true);

    let first_products = CompilerProducts::read_artifact_from_path(&first_artifact).unwrap();
    let second_products = CompilerProducts::read_artifact_from_path(&second_artifact).unwrap();
    let first_helper_id = first_products.identity_table.export_names["helper"];
    let second_helper_id = second_products.identity_table.export_names["helper"];
    let first_symbol = first_products.link.records[&first_helper_id]
        .backend_symbol
        .as_str();
    let second_symbol = second_products.link.records[&second_helper_id]
        .backend_symbol
        .as_str();
    assert!(first_symbol.starts_with("__rock_"));
    assert!(second_symbol.starts_with("__rock_"));
    assert_ne!(first_symbol, second_symbol);

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_as("first", first_artifact)
        .unwrap();
    ctx.load_product_artifact_from_path_as("second", second_artifact)
        .unwrap();
    let first = ctx.extern_crate("first").unwrap();
    let second = ctx.extern_crate("second").unwrap();
    let first_link_symbol = first
        .link()
        .backend_symbol(first.root_exports()["helper"].id)
        .unwrap()
        .to_string();
    let second_link_symbol = second
        .link()
        .backend_symbol(second.root_exports()["helper"].id)
        .unwrap()
        .to_string();

    assert_eq!(first_link_symbol, first_symbol);
    assert_eq!(second_link_symbol, second_symbol);
    assert_ne!(first_link_symbol, second_link_symbol);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifacts_distinguish_separator_underscore_exported_backend_symbols() {
    let temp_dir = temp_test_dir("separator_underscore_exported_backend_symbols");
    let first_dir = temp_dir.join("foo");
    let second_dir = temp_dir.join("foo__bar");
    let first_artifact = temp_dir.join("foo.rkca");
    let second_artifact = temp_dir.join("foo__bar.rkca");

    write_crate(&first_dir, "foo", "", "< bar__baz = -> 11\n< bar__baz\n");
    write_crate(&second_dir, "foo__bar", "", "< baz = -> 22\n< baz\n");

    build_product_artifact(&first_dir, "foo", &first_artifact, true, true);
    build_product_artifact(&second_dir, "foo__bar", &second_artifact, true, true);

    let first_products = CompilerProducts::read_artifact_from_path(&first_artifact).unwrap();
    let second_products = CompilerProducts::read_artifact_from_path(&second_artifact).unwrap();
    let first_id = first_products.identity_table.export_names["bar__baz"];
    let second_id = second_products.identity_table.export_names["baz"];
    let first_symbol = first_products.link.records[&first_id]
        .backend_symbol
        .as_str();
    let second_symbol = second_products.link.records[&second_id]
        .backend_symbol
        .as_str();

    assert_ne!(first_symbol, second_symbol);

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_as("foo", first_artifact)
        .unwrap();
    ctx.load_product_artifact_from_path_as("foo__bar", second_artifact)
        .unwrap();
    let first = ctx.extern_crate("foo").unwrap();
    let second = ctx.extern_crate("foo__bar").unwrap();
    let first_link_symbol = first
        .link()
        .backend_symbol(first.root_exports()["bar__baz"].id)
        .unwrap()
        .to_string();
    let second_link_symbol = second
        .link()
        .backend_symbol(second.root_exports()["baz"].id)
        .unwrap()
        .to_string();

    assert_eq!(first_link_symbol, first_symbol);
    assert_eq!(second_link_symbol, second_symbol);
    assert_ne!(first_link_symbol, second_link_symbol);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_stdlib_product_artifact_links_static_impl_method() {
    let artifact_path = shared_stdlib_product_artifact();

    let exit_code = write_and_run_artifact_app(
        r#"
main = ->
    s = String::from_str "hello"
    s.len!
"#,
        vec![("stdlib".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 5);
}

#[test]
fn test_stdlib_product_artifact_links_arithmetic_operator_impl_methods() {
    let artifact_path = shared_stdlib_product_artifact();
    let arithmetic_path = workspace_root().join("examples").join("arithmetic.rk");
    let temp_dir = temp_test_dir("stdlib_artifact_arithmetic");

    crate::compile(&Config {
        entry_file: arithmetic_path,
        output_dir: temp_dir.join("out"),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![("stdlib".to_string(), artifact_path)],
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    })
    .unwrap();

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_links_concrete_trait_impl_method() {
    let temp_dir = temp_test_dir("concrete_trait_impl_method_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        r#"
< trait Value
    @get: I64
< Value

< struct Box
    < value: I64
< Box

impl Value for Box
    @get = -> @value

< new_box = -> Box
    value: 37
"#,
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    delete_crate_source_files(&dep_dir);

    let exit_code = write_and_run_artifact_app_with_options(
        r#"
> dep::Value
> dep::Box
> dep::new_box

main = ->
    box = new_box!
    box.get!
"#,
        vec![("dep".to_string(), artifact_path)],
        None,
        true,
        false,
    );
    assert_eq!(exit_code, 37);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_stdlib_product_artifact_specializes_generic_function_used_by_generic_impl() {
    let artifact_path = shared_stdlib_product_artifact();

    let exit_code = write_and_run_artifact_app(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    v.len!
"#,
        vec![("stdlib".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 1);
}

#[test]
fn test_product_artifact_links_concrete_function_with_mono_in_name() {
    let temp_dir = temp_test_dir("mono_named_concrete_function_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        r#"< foo_mono_bar = -> 9
< foo_mono_bar
"#,
    );
    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        r#"
> dep::foo_mono_bar

main = -> foo_mono_bar!
"#,
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 9);
}

#[test]
fn test_product_artifact_preserves_concrete_function_bodies() {
    let temp_dir = temp_test_dir("source_backed_artifact_bodies");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "< answer = -> 5\n< answer\n");

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app(
        "> dep::answer\n\nmain = -> answer!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 5);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_preserves_associated_types() {
    let temp_dir = temp_test_dir("product_artifact_associated_types");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        r#"< trait Deref
    type Target
    @deref: () -> &Self::Target

struct Box T
    value: T

impl Deref for Box T
    type Target = T
    @deref = -> &@value
"#,
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let deref_trait = products
        .interface
        .traits
        .values()
        .find(|trait_| trait_.name == "dep::Deref")
        .unwrap();
    assert_eq!(deref_trait.associated_types.len(), 1);
    assert_eq!(deref_trait.associated_types[0].name, "Target");
    assert_eq!(
        deref_trait.signatures.get("deref").unwrap().ret,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Projection {
                ty: Box::new(Type::Generic(crate::types::GenericParamId {
                    owner: deref_trait.id,
                    index: 0
                })),
                trait_id: deref_trait.id,
                assoc_type: crate::types::AssociatedTypeKey {
                    owner: deref_trait.id,
                    assoc_type_id: deref_trait.associated_types[0].id,
                },
                trait_args: vec![],
            }),
        }
    );

    let deref_impl = products
        .interface
        .impls
        .values()
        .find(|imp| imp.trait_name.as_deref() == Some("Deref") && imp.type_name == "Box")
        .unwrap();
    assert_eq!(deref_impl.associated_types.len(), 1);
    assert_eq!(deref_impl.associated_types[0].name, "Target");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_interface_externs_use_resolved_def_ids() {
    let temp_dir = temp_test_dir("product_artifact_extern_def_ids");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "< marker = -> 0\nextern puts: *U8 -> I32\n",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let (product_id, extern_fn) = products
        .interface
        .externs
        .iter()
        .find(|(_, ext)| ext.name == "dep::puts")
        .unwrap_or_else(|| {
            panic!(
                "available externs: {:?}",
                products
                    .interface
                    .externs
                    .values()
                    .map(|ext| ext.name.clone())
                    .collect::<Vec<_>>()
            )
        });
    let placeholder = DefId::new(CrateId(0), LocalDefId(0));
    assert_ne!(extern_fn.id, placeholder);
    assert_eq!(
        extern_fn.id,
        DefId::new(
            CrateId(product_id.crate_id.0),
            LocalDefId(product_id.local_id.0),
        )
    );

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_imported_extern_call_links_by_def_id() {
    let temp_dir = temp_test_dir("product_artifact_imported_extern");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "extern tolower: I32 -> I32\n");
    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        "> dep::tolower\n\nmain = -> tolower 65\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 97);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_requires_explicit_stdlib_artifact_even_with_sysroot() {
    let temp_dir = temp_test_dir("sysroot_stdlib_compile");
    let sysroot = temp_dir.join("toolchain");
    let stdlib_dir = temp_dir.join("stdlib");
    let entry_file = temp_dir.join("main.rk");
    let output_dir = temp_dir.join("out");

    write_answer_stdlib(&stdlib_dir, 7);
    write_sysroot_stdlib_product_bundle(&stdlib_dir, &sysroot);
    fs::write(&entry_file, "main = -> answer!\n").unwrap();

    let config = Config {
        entry_file,
        output_dir: output_dir.clone(),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![],
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: Some(sysroot),
    };

    assert!(crate::compile(&config).is_err());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_explicit_stdlib_artifact_works_when_sysroot_is_set() {
    let temp_dir = temp_test_dir("sysroot_stdlib_override");
    let sysroot = temp_dir.join("toolchain");
    let sysroot_stdlib = temp_dir.join("sysroot_stdlib");
    let explicit_stdlib = temp_dir.join("explicit_stdlib");
    let explicit_artifact_path = temp_dir.join("explicit_stdlib.rkca");

    write_answer_stdlib(&sysroot_stdlib, 7);
    write_answer_stdlib(&explicit_stdlib, 5);

    write_sysroot_stdlib_product_bundle(&sysroot_stdlib, &sysroot);

    build_stdlib_product_artifact(&explicit_stdlib, &explicit_artifact_path);
    CompilerProducts::read_artifact_from_path(&explicit_artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app_with_options(
        "> stdlib::math::answer\n\nmain = -> answer!\n",
        vec![("stdlib".to_string(), explicit_artifact_path)],
        Some(sysroot),
        false,
        false,
    );
    assert_eq!(exit_code, 5);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_without_explicit_stdlib_artifact_fails_with_no_std() {
    let temp_dir = temp_test_dir("sysroot_no_std");
    let sysroot = temp_dir.join("toolchain");
    let stdlib_dir = temp_dir.join("stdlib");
    let entry_file = temp_dir.join("main.rk");

    write_answer_stdlib(&stdlib_dir, 7);
    write_sysroot_stdlib_product_bundle(&stdlib_dir, &sysroot);
    fs::write(&entry_file, "main = -> answer!\n").unwrap();

    let config = Config {
        entry_file,
        output_dir: temp_dir.join("out"),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![],
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: false,
        no_std: true,
        sysroot: Some(sysroot),
    };

    assert!(crate::compile(&config).is_err());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_with_source_free_product_artifact() {
    let temp_dir = temp_test_dir("interface_only_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "< mod math\n");
    fs::write(dep_dir.join("math.rk"), "< answer = -> 5\n< answer\n").unwrap();

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    delete_crate_source_files(&dep_dir);

    let exit_code = write_and_run_artifact_app(
        "> dep::math::answer\n\nmain = -> answer!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 5);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_root_glob_import_from_source_free_product_artifact() {
    let temp_dir = temp_test_dir("product_artifact_root_glob_import");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "answer = -> 5\n< answer\n");

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    delete_crate_source_files(&dep_dir);

    let exit_code = write_and_run_artifact_app(
        "> dep::*\n\nmain = -> answer!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 5);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_rejects_changed_dependency_fingerprint() {
    let temp_dir = temp_test_dir("changed_dependency_fingerprint");
    let dep_dir = temp_dir.join("dep");
    let app_dir = temp_dir.join("app");
    let dep_artifact = temp_dir.join("dep.rkca");
    let app_artifact = temp_dir.join("app.rkca");

    write_crate(&dep_dir, "dep", "", "answer = -> 5\n< answer\n");
    build_product_artifact(&dep_dir, "dep", &dep_artifact, true, true);

    write_crate(
        &app_dir,
        "app",
        "",
        "> dep::answer\n\nmain = -> answer!\n< main\n",
    );
    build_product_artifact_with_extern_artifacts(
        &app_dir,
        "app",
        &app_artifact,
        true,
        true,
        vec![("dep".to_string(), dep_artifact.clone())],
    );

    fs::write(dep_dir.join("lib.rk"), "answer = -> 7\n< answer\n").unwrap();
    build_product_artifact(&dep_dir, "dep", &dep_artifact, true, true);

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_as("dep", dep_artifact)
        .unwrap();
    let err = ctx
        .load_product_artifact_from_path_as("app", app_artifact)
        .unwrap_err();

    assert!(
        err.contains("dependency identity mismatch"),
        "unexpected error: {err}"
    );

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_generic_function_from_artifact_hir_bundle() {
    let temp_dir = temp_test_dir("generic_function_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "identity = x -> x\n< identity\n");

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app(
        "> dep::identity\n\nmain = -> identity 7\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 7);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_cross_crate_hkt_selection_and_generic_body_specialization() {
    let temp_dir = temp_test_dir("cross_crate_hkt_specialization");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        r#"
trait Functor for F _
    keep: F A -> F A
< Functor

trait Monad for F _
    pure: A -> F A
< Monad

enum Maybe T
    None
    Some T
< Maybe

impl Functor for Maybe
    keep = value -> value

impl Monad for Maybe
    pure = value -> Maybe::Some value

keep_generic: F I64 -> F I64 where F _: Functor
keep_generic = value -> F::keep value
< keep_generic

repure: F I64 -> F I64 where F _: Monad
repure = ignored -> F::pure 42
< repure
"#,
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    delete_crate_source_files(&dep_dir);

    let exit_code = write_and_run_artifact_app_with_options(
        r#"
> dep::Maybe
> dep::keep_generic
> dep::repure

main = ->
    kept = keep_generic (Maybe::Some 7)
    result = repure kept
    match result
        Maybe::Some value => value
        Maybe::None => 1
"#,
        vec![("dep".to_string(), artifact_path)],
        None,
        true,
        true,
    );
    assert_eq!(exit_code, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_generic_function_from_file_module_artifact_hir_bundle() {
    let temp_dir = temp_test_dir("generic_function_file_module_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "< mod generic\n");
    fs::write(
        dep_dir.join("generic.rk"),
        "< identity = x -> x\n< identity\n",
    )
    .unwrap();

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app(
        "> dep::generic::identity\n\nmain = -> identity 7\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 7);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_generic_impl_from_artifact_hir_bundle() {
    let temp_dir = temp_test_dir("generic_impl_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "identity = x -> x\n< identity\n\nstruct Box T\n    value: T\n< Box\n\nimpl Box T\n    new = value ->\n        Box\n            value: value\n\n    @apply = f -> f self.value\n",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app(
        "> dep::Box\n> dep::identity\n\nmain = ->\n    b = Box::new 21\n    b.apply identity\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 21);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_trait_backed_structural_static_method_from_artifact() {
    let temp_dir = temp_test_dir("trait_backed_structural_static_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "struct Box T
    value: T
< Box

impl Box T
    wrap = value ->
        Box
            value: value

    @get = -> self.value

trait Factory T
    create: Box T -> Box T
< Factory

impl Factory T for Box T
    create = value -> value
",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        "> dep::Box\n\nmain = ->\n    input = Box::wrap 42\n    output = Box::create input\n    output.get!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_generic_impl_from_file_module_artifact_hir_bundle() {
    let temp_dir = temp_test_dir("generic_impl_file_module_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "< mod boxes\n");
    fs::write(
        dep_dir.join("boxes.rk"),
        "< identity = x -> x\n< identity\n\n< struct Box T\n    value: T\n< Box\n\nimpl Box T\n    new = value ->\n        Box\n            value: value\n\n    @apply = f -> f self.value\n",
    )
    .unwrap();

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app(
        "> dep::boxes::Box\n> dep::boxes::identity\n\nmain = ->\n    b = Box::new 21\n    b.apply identity\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 21);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_preserves_trait_default_methods() {
    let temp_dir = temp_test_dir("product_trait_defaults");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "trait Animal\n    @legs = -> 4\n< Animal\n",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let animal = products
        .interface
        .traits
        .values()
        .find(|trait_| trait_.name == "dep::Animal")
        .unwrap();
    let legs = animal.methods.get("legs").unwrap();
    let legs_body = products
        .bodies
        .trait_default_methods
        .values()
        .find(|function| function.id == legs.id)
        .unwrap();
    assert_eq!(legs_body.name, "legs");
    assert!(!legs_body.body.stmts.is_empty());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_preserves_file_module_trait_default_methods() {
    let temp_dir = temp_test_dir("product_file_module_trait_defaults");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "< mod animals\n");
    fs::write(
        dep_dir.join("animals.rk"),
        "< trait Animal\n    @legs = -> 4\n< Animal\n",
    )
    .unwrap();

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let animal = products
        .interface
        .traits
        .values()
        .find(|trait_| trait_.name == "dep::animals::Animal")
        .unwrap();
    let legs = animal.methods.get("legs").unwrap();
    let legs_body = products
        .bodies
        .trait_default_methods
        .values()
        .find(|function| function.id == legs.id)
        .unwrap();
    assert_eq!(legs_body.name, "legs");
    assert!(!legs_body.body.stmts.is_empty());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_trait_default_method_from_product_artifact() {
    let temp_dir = temp_test_dir("product_trait_default_downstream");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "< trait Animal\n    @legs = -> 4\n< Animal\n",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        "> dep::Animal\n\nstruct Dog\n\nimpl Animal for Dog\n\nmain = ->\n    d = Dog\n    d.legs!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 4);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_trait_generic_default_for_downstream_impl_from_product_artifact() {
    let temp_dir = temp_test_dir("product_trait_generic_default_downstream_impl");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "trait Value T\n    @get: T\n    @same: T\n    @same = -> self.get!\n< Value\n",
    );
    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        "> dep::Value\n\nstruct Number\n    < value: I64\n\nimpl Value I64 for Number\n    @get = -> @value\n\nmain = ->\n    number = Number\n        value: 42\n    number.same!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_generic_trait_default_method_from_product_artifact() {
    let temp_dir = temp_test_dir("product_generic_trait_default_downstream");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "struct Box T
    value: T
< Box

impl Box T
    new = value ->
        Box
            value: value

trait Value T
    @get: T
    @same: T
    @same = -> self.get!
< Value

impl Value T for Box T
    @get = -> @value
",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        "> dep::Box
> dep::Value

main = ->
    b = Box::new 42
    b.same!
",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_preserves_generic_impl_inherited_trait_default_methods() {
    let temp_dir = temp_test_dir("product_generic_impl_inherited_trait_default");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "struct Box T
    value: T
< Box

impl Box T
    new = value ->
        Box
            value: value

trait Value T
    @get: T
    @same: T
    @same = -> self.get!
< Value

impl Value T for Box T
    @get = -> @value
",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();
    let generic_impl = products
        .bodies
        .generic_impls
        .values()
        .find(|imp| imp.methods.contains_key("same"))
        .expect("generic impl should preserve inherited default method");

    let same = generic_impl.methods.get("same").unwrap();
    assert!(!same.body.stmts.is_empty());
    assert!(generic_impl.methods.contains_key("get"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_remaps_trait_args_in_default_method_body() {
    let temp_dir = temp_test_dir("product_default_method_target_trait_args");
    let marker_dir = temp_dir.join("marker");
    let dep_dir = temp_dir.join("dep");
    let marker_artifact = temp_dir.join("marker.rkca");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&marker_dir, "marker", "", "struct Token\n< Token\n");
    build_product_artifact(&marker_dir, "marker", &marker_artifact, true, true);

    write_crate(
        &dep_dir,
        "dep",
        "",
        "> marker::Token

trait Value T
    @get: T
    @same: T
    @same = -> self.get!
< Value

keep_token: Token -> Token
keep_token = value -> value
< keep_token
",
    );

    build_product_artifact_with_extern_artifacts(
        &dep_dir,
        "dep",
        &artifact_path,
        true,
        true,
        vec![("marker".to_string(), marker_artifact.clone())],
    );

    let mut products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();
    let keep_token_id = products.identity_table.export_names["keep_token"];
    let token_ty = products.interface.functions[&keep_token_id]
        .ret_type
        .clone();
    assert!(matches!(token_ty, Type::Struct { .. }));

    let value_trait_id = products.identity_table.export_names["Value"];
    let same_method_id = products.interface.traits[&value_trait_id].methods["same"].id;
    let body = products
        .bodies
        .trait_default_methods
        .values_mut()
        .find(|method| method.id == same_method_id)
        .unwrap();
    let HirStmt::Expr(expr) = &mut body.body.stmts[0] else {
        panic!("default method body should contain method call expression");
    };
    let HirExprKind::MethodCall(_, _, _, _, target) = &mut expr.kind else {
        panic!("default method body should call required trait method");
    };
    *target
        .trait_args_mut()
        .expect("trait method target has trait arguments") = vec![token_ty];
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_as("marker", marker_artifact)
        .unwrap();
    ctx.load_product_artifact_from_path_as("dep", artifact_path)
        .unwrap();
    let marker_token_id = ctx
        .extern_crate("marker")
        .unwrap()
        .metadata()
        .resolver()
        .item_paths["marker::Token"];
    let dep = ctx.extern_crate("dep").unwrap();
    let value_trait_id = dep
        .metadata()
        .interface()
        .trait_by_canonical_name("dep::Value")
        .expect("trait interface should exist")
        .id;
    let value_trait = dep
        .body_providers()
        .trait_with_defaults(value_trait_id)
        .unwrap();
    let same = value_trait.methods.get("same").unwrap();
    let HirStmt::Expr(expr) = &same.body.stmts[0] else {
        panic!("default method body should contain method call expression");
    };
    let HirExprKind::MethodCall(_, method_name, _, _, target) = &expr.kind else {
        panic!("default method body should call required trait method");
    };

    assert_eq!(method_name, "get");
    assert_eq!(target.trait_id(), Some(value_trait.id));
    assert_eq!(
        target.trait_args(),
        &[Type::Struct {
            id: marker_token_id,
            args: Vec::new(),
        }]
    );

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_projection_trait_identity_from_product_artifact() {
    let temp_dir = temp_test_dir("product_projection_trait_identity");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "< trait HasValue\n    type Output\n    @get: Self::Output\n< HasValue\n",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        "> dep::HasValue\n\nstruct Local\n    < value: I64\n\nimpl HasValue for Local\n    type Output = I64\n    @get = -> @value\n\nmain = ->\n    local = Local\n        value: 33\n    local.get!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 33);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn renamed_language_item_provider_artifact_drives_all_protocols() {
    let temp_dir = temp_test_dir("renamed_language_item_provider");
    let provider_dir = temp_dir.join("protocols");
    let provider_artifact = temp_dir.join("protocols.rkca");

    write_crate(
        &provider_dir,
        "protocols",
        "",
        r#"lang sized
< trait Stature
< Stature

lang drop
< trait Farewell
    lang method
    ~@dismiss: ()
< Farewell

lang index
< trait Slot Key
    lang output
    type Yield
    lang method
    @fetch: Key -> &Self::Yield
< Slot

lang control_flow
< enum Fork B, C
    lang break
    Stop B
    lang continue
    Go C
< Fork

lang try
< trait Passage
    lang output
    type Resulting
    lang residual
    type Debris
    lang branch
    ~@divide: Fork Self::Debris, Self::Resulting
< Passage

lang from_residual
< trait Rebuild R
    lang method
    heal: R -> Self
< Rebuild

extern puts: *U8 -> I32

report_drop: &[U8] -> ()
report_drop = bytes ->
    puts (~ArrPtr *bytes)
    return
< report_drop

struct Ticket

impl Farewell for Ticket
    ~@dismiss = ->
        report_drop (&[100, 114, 111, 112, 0])
        return

make_ticket = -> Ticket
< Ticket
< make_ticket

struct Rack
    < value: I64

impl Slot I64 for Rack
    type Yield = I64
    @fetch = _ -> &@value

make_rack = ->
    Rack
        value: 1
< Rack
< make_rack

enum Outcome T
    Ready T
    Stalled
< Outcome

enum Residue
    Stalled I64

impl Passage for Outcome T
    type Resulting = T
    type Debris = Residue

    ~@divide = ->
        match self
            Outcome::Ready value => Fork::Go value
            Outcome::Stalled => Fork::Stop (Residue::Stalled 7)

impl Rebuild Residue for Outcome T
    heal = residue ->
        match residue
            Residue::Stalled _ => Outcome::Stalled

candidate: Bool -> Outcome I64
candidate = ok ->
    if ok
        Outcome::Ready 41
    else
        Outcome::Stalled
< candidate
"#,
    );
    build_product_artifact(&provider_dir, "protocols", &provider_artifact, true, true);

    // An explicit consuming Drop method call must link and consume the ticket exactly once.
    let (explicit_stdout, explicit_exit_code) = compile_and_run_artifact_app(
        r#"> protocols::*

extern exit: I32 -> ()

requires_stature: T -> I64 where T: Stature
requires_stature = _ -> 1

compute: Bool -> Outcome I64
compute = ok ->
    value = candidate ok?
    Outcome::Ready 42

main: () -> I64
main = ->
    ticket = make_ticket!
    rack = Rack
        value: 42
    indexed = rack[0]
    ticket.dismiss!
    layout = requires_stature rack
    success = compute true
    failure = compute false
    exit 42
    0
        "#,
        vec![("protocols".to_string(), provider_artifact.clone())],
    );

    assert_eq!(explicit_stdout, "drop\n");
    assert_eq!(explicit_exit_code, 42);

    // Normal scope return must invoke the marked Drop method without an explicit call.
    let (automatic_stdout, automatic_exit_code) = compile_and_run_artifact_app(
        r#"> protocols::*

main: () -> I64
main = ->
    ticket = make_ticket!
    42
"#,
        vec![("protocols".to_string(), provider_artifact)],
    );

    assert_eq!(automatic_stdout, "drop\n");
    assert_eq!(automatic_exit_code, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn renamed_artifact_index_and_index_mut_drive_downstream_syntax() {
    let temp_dir = temp_test_dir("renamed_artifact_index_pair");
    let provider_dir = temp_dir.join("provider");
    let provider_artifact = temp_dir.join("provider.rkca");

    write_crate(
        &provider_dir,
        "provider",
        "",
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue
< ReadAt

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue
< WriteAt
"#,
    );
    build_product_artifact(&provider_dir, "provider", &provider_artifact, true, true);

    let downstream_source = r#"> provider::*

struct Cell
    < read_value: I64
    < write_value: I64

impl ReadAt I64 for Cell
    type ReadValue = I64
    @read_at = _ -> &@read_value

impl WriteAt I64 for Cell
    type WriteValue = I64
    ^@write_at = _ -> &mut @write_value

probe: T -> I64
probe = _ ->
    mut target = Cell
        read_value: 1
        write_value: 0
    source = Cell
        read_value: 7
        write_value: 2
    target[0] = source[0]
    target.write_value

main: () -> I64
main = -> probe 0
"#;

    let (stdout, exit_code) = compile_and_run_artifact_app(
        downstream_source,
        vec![("provider".to_string(), provider_artifact.clone())],
    );
    assert_eq!(stdout, "");
    assert_eq!(exit_code, 7);

    let provider_products = CompilerProducts::read_artifact_from_path(&provider_artifact)
        .expect("provider artifact should load for ID assertions");
    let provider_read = provider_products
        .interface
        .language_items
        .index
        .as_ref()
        .expect("provider should mark ReadAt as Index");
    let provider_write = provider_products
        .interface
        .language_items
        .index_mut
        .as_ref()
        .expect("provider should mark WriteAt as IndexMut");

    let mut context = CrateContext::new();
    context
        .load_product_artifact_from_path_as("provider", provider_artifact.clone())
        .expect("provider artifact should load into a fresh context");
    let runtime_provider = context
        .extern_crate("provider")
        .expect("fresh context should contain provider");
    let runtime_items = runtime_provider.metadata().language_items();
    let runtime_read = runtime_items
        .index
        .as_ref()
        .expect("loaded provider should expose ReadAt");
    let runtime_write = runtime_items
        .index_mut
        .as_ref()
        .expect("loaded provider should expose WriteAt");
    let remapped =
        |id: ProductDefId| DefId::new(runtime_provider.crate_id(), LocalDefId(id.local_id.0));

    assert_eq!(runtime_read.trait_id, remapped(provider_read.trait_id));
    assert_eq!(runtime_read.method_id, remapped(provider_read.method_id));
    assert_eq!(runtime_write.trait_id, remapped(provider_write.trait_id));
    assert_eq!(runtime_write.method_id, remapped(provider_write.method_id));

    let temp_entry = temp_dir.join("downstream.rk");
    fs::write(&temp_entry, downstream_source).unwrap();
    let output = crate::compile_with_products(&Config {
        entry_file: temp_entry,
        output_dir: temp_dir.join("downstream-build"),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![("provider".to_string(), provider_artifact)],
        source_providers: Vec::new(),
        current_crate_name: Some("downstream".to_string()),
        opt_level: 0,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: true,
        no_std: true,
        sysroot: None,
    })
    .expect("downstream source should compile from the provider artifact only");
    let products = output
        .products
        .expect("downstream products should be emitted");
    let probe = products
        .bodies
        .functions
        .values()
        .find(|function| function.name == "probe")
        .expect("downstream generic probe body should be present");
    let targets = collect_index_authorities_from_function(probe);
    assert_eq!(targets.len(), 2);

    let mut read_count = 0;
    let mut write_count = 0;
    for target in targets {
        let HirSelectedMethodTarget::ImplMethod {
            selected_trait: Some(selected_trait),
            ..
        } = target.target
        else {
            panic!("indexed call must preserve provider trait authority");
        };
        if selected_trait.trait_id == runtime_read.trait_id {
            read_count += 1;
            assert_eq!(selected_trait.member_id, runtime_read.method_id);
        } else if selected_trait.trait_id == runtime_write.trait_id {
            write_count += 1;
            assert_eq!(selected_trait.member_id, runtime_write.method_id);
        } else {
            panic!("indexed call selected an unrelated provider trait");
        }
    }
    assert_eq!((read_count, write_count), (1, 1));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn root_glob_import_keeps_local_direct_calls_direct_in_artifact_app() {
    let temp_dir = temp_test_dir("root_glob_local_callable_app");
    let dependency_dir = temp_dir.join("dep");
    let dependency_artifact = temp_dir.join("dep.rkca");
    write_crate(&dependency_dir, "dep", "", "unused = -> 0\n< unused\n");
    build_product_artifact(&dependency_dir, "dep", &dependency_artifact, true, true);

    let (stdout, exit_code) = compile_and_run_artifact_app(
        r#"> dep::*

local: I64
local = -> 83

identity: T -> T
identity = value -> value

main: () -> I64
main = -> identity (local!)
"#,
        vec![("dep".to_string(), dependency_artifact)],
    );

    assert_eq!(stdout, "");
    assert_eq!(exit_code, 83);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn unmarked_try_and_index_protocol_spellings_have_no_authority_without_provider() {
    let try_diagnostics = compile_artifact_app_without_stdlib_failure(
        r#"enum ControlFlow B, C
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
    value = Maybe::Some 1
    value?
"#,
        Vec::new(),
    );
    assert!(try_diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message == "Cannot use '?' because the Try language-item protocol is unavailable"
    }));

    let index_diagnostics = compile_artifact_app_without_stdlib_failure(
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
        Vec::new(),
    );
    assert!(index_diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message
            == "Cannot use indexing because the Index language-item protocol is unavailable"
    }));
}

#[test]
fn duplicate_language_item_provider_artifacts_report_sorted_provider_names() {
    let temp_dir = temp_test_dir("duplicate_language_item_provider");
    let alpha_dir = temp_dir.join("alpha");
    let zeta_dir = temp_dir.join("zeta");
    let alpha_artifact = temp_dir.join("alpha.rkca");
    let zeta_artifact = temp_dir.join("zeta.rkca");
    write_crate(&alpha_dir, "alpha", "", "lang sized\n< trait AlphaShape\n");
    write_crate(&zeta_dir, "zeta", "", "lang sized\n< trait ZetaShape\n");
    build_product_artifact(&alpha_dir, "alpha", &alpha_artifact, true, true);
    build_product_artifact(&zeta_dir, "zeta", &zeta_artifact, true, true);

    let diagnostics = compile_artifact_app_without_stdlib_failure(
        "main = -> 0\n",
        vec![
            ("zeta".to_string(), zeta_artifact),
            ("alpha".to_string(), alpha_artifact),
        ],
    );
    let messages = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let conflict = messages
        .lines()
        .find(|message| message.contains("protocol 'sized'"))
        .unwrap_or_else(|| panic!("expected sized provider conflict, got: {messages}"));
    assert!(conflict.contains("alpha, zeta"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn intermediate_artifact_does_not_reexport_provider_language_items() {
    let temp_dir = temp_test_dir("intermediate_language_item_non_reexport");
    let provider_dir = temp_dir.join("provider");
    let bridge_dir = temp_dir.join("bridge");
    let isolated_dir = temp_dir.join("isolated_bridge");
    let provider_artifact = temp_dir.join("provider.rkca");
    let bridge_artifact = temp_dir.join("bridge.rkca");
    let isolated_artifact = temp_dir.join("isolated_bridge.rkca");

    write_crate(
        &provider_dir,
        "provider",
        "",
        r#"lang index
< trait Slot Key
    lang output
    type Yield
    lang method
    @fetch: Key -> &Self::Yield

lang control_flow
< enum Fork B, C
    lang break
    Stop B
    lang continue
    Go C

lang try
< trait Passage
    lang output
    type Resulting
    lang residual
    type Debris
    lang branch
    ~@divide: Fork Self::Debris, Self::Resulting

lang from_residual
< trait Rebuild R
    lang method
    heal: R -> Self

struct Rack
    < value: I64
< Rack

impl Slot I64 for Rack
    type Yield = I64
    @fetch = _ -> &@value

enum Packet T
    Ready T
    Stalled
< Packet

enum Residue
    Stalled I64

impl Passage for Packet T
    type Resulting = T
    type Debris = Residue
    ~@divide = ->
        match self
            Packet::Ready value => Fork::Go value
            Packet::Stalled => Fork::Stop (Residue::Stalled 7)

impl Rebuild Residue for Packet T
    heal = residue ->
        match residue
            Residue::Stalled _ => Packet::Stalled
"#,
    );
    build_product_artifact(&provider_dir, "provider", &provider_artifact, true, true);

    write_crate(
        &bridge_dir,
        "bridge",
        "",
        r#"> provider::*

consume: Packet I64 -> Packet I64
consume = packet ->
    rack = Rack
        value: 5
    indexed = rack[0]
    value = packet?
    Packet::Ready value
< consume
"#,
    );
    build_product_artifact_with_extern_artifacts(
        &bridge_dir,
        "bridge",
        &bridge_artifact,
        true,
        true,
        vec![("provider".to_string(), provider_artifact.clone())],
    );

    let bridge_products = CompilerProducts::read_artifact_from_path(&bridge_artifact).unwrap();
    assert_eq!(
        bridge_products.interface.language_items,
        ProductLanguageItems::default(),
        "bridge must serialize only language items it provides"
    );
    assert_eq!(bridge_products.dependencies.len(), 1);
    assert_eq!(bridge_products.dependencies[0].name, "provider");

    let missing_dependency = CrateContext::new()
        .load_product_artifact_from_path_as("bridge", bridge_artifact.clone())
        .expect_err("bridge must retain its provider dependency integrity");
    assert!(
        missing_dependency.contains("dependency"),
        "expected missing dependency diagnostic, got: {missing_dependency}"
    );

    write_crate(
        &isolated_dir,
        "isolated_bridge",
        "",
        "bridge_value = -> 0\n< bridge_value\n",
    );
    build_product_artifact(
        &isolated_dir,
        "isolated_bridge",
        &isolated_artifact,
        true,
        true,
    );
    let isolated_products = CompilerProducts::read_artifact_from_path(&isolated_artifact).unwrap();
    assert_eq!(
        isolated_products.interface.language_items,
        ProductLanguageItems::default(),
        "isolated non-provider artifact must not confer protocol authority"
    );

    let diagnostics = compile_artifact_app_without_stdlib_failure(
        r#"> isolated_bridge::*

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
    value = Maybe::Some 1
    value?
"#,
        vec![("isolated_bridge".to_string(), isolated_artifact.clone())],
    );
    assert!(diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message == "Cannot use '?' because the Try language-item protocol is unavailable"
    }));
    let index_diagnostics = compile_artifact_app_without_stdlib_failure(
        r#"> isolated_bridge::*

trait Index Key
    type Output
    @index: Key -> &Self::Output
struct Boxed
    < value: I64
impl Index I64 for Boxed
    type Output = I64
    @index = _ -> &@value
main = ->
    boxed = Boxed value: 1
    boxed[0]
"#,
        vec![("isolated_bridge".to_string(), isolated_artifact)],
    );
    assert!(index_diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message
            == "Cannot use indexing because the Index language-item protocol is unavailable"
    }));

    let (stdout, exit_code) = compile_and_run_artifact_app(
        r#"> provider::*
> bridge::consume

main: () -> I64
main = ->
    result = consume (Packet::Ready 7)
    0
"#,
        vec![
            ("provider".to_string(), provider_artifact),
            ("bridge".to_string(), bridge_artifact),
        ],
    );
    assert_eq!(stdout, "");
    assert_eq!(exit_code, 0);

    let _ = fs::remove_dir_all(temp_dir);
}
