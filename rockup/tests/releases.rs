// Runs on every host with published release assets.
#![cfg(any(
    all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"),
    target_os = "macos",
    all(target_arch = "x86_64", target_os = "windows", target_env = "msvc")
))]

use std::{
    collections::HashMap,
    env::consts::EXE_SUFFIX,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};

const TARGET: &str = if cfg!(target_os = "windows") {
    "x86_64-pc-windows-msvc"
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    "aarch64-apple-darwin"
} else if cfg!(target_os = "macos") {
    "x86_64-apple-darwin"
} else {
    "x86_64-unknown-linux-gnu"
};
static NEXT: AtomicU64 = AtomicU64::new(0);

/// `name` with the host's executable suffix.
fn exe(name: &str) -> String {
    format!("{name}{EXE_SUFFIX}")
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = canonical_temp_dir().join(format!(
            "rockup-release-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("tools")).unwrap();
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::create_dir_all(root.join("user")).unwrap();
        copy_executable(&fake_curl(), &root.join("tools").join(exe("curl")));
        fs::write(root.join("releases.json"), "[]").unwrap();
        Self { root }
    }

    fn command(&self) -> Command {
        self.command_for(Path::new(env!("CARGO_BIN_EXE_rockup")))
    }

    fn command_for(&self, executable: &Path) -> Command {
        let mut command = Command::new(executable);
        let mut paths = vec![self.root.join("tools")];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        command
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("HOME", self.root.join("user"))
            .env("ROCKUP_HOME", self.root.join("home"))
            .env_remove("ROCKUP_TOOLCHAIN")
            .env("SHELL", "/bin/sh")
            .env("FIXTURE", &self.root)
            .current_dir(&self.root);
        // Never change the test machine's user PATH in the registry.
        if cfg!(windows) {
            command.env("ROCKUP_NO_MODIFY_PATH", "1");
        }
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn source(&self, version: &str) -> PathBuf {
        let source = self.root.join("source");
        fs::create_dir_all(source.join("bin")).unwrap();
        for binary in ["rock", "rockc", "rock-lsp"] {
            fake_tool(&source.join("bin").join(exe(binary)), version);
        }
        let lib = source.join("lib/rocklib").join(TARGET);
        fs::create_dir_all(&lib).unwrap();
        // Filenames match the current ToolchainLayout contract.
        for file in [
            rock_shared::sysroot::STD_ARTIFACT_FILE_NAME,
            rock_shared::sysroot::STD_OBJECT_FILE_NAME,
            rock_shared::sysroot::TOOLCHAIN_MANIFEST_FILE_NAME,
            rock_shared::sysroot::COMPONENTS_MANIFEST_FILE_NAME,
        ] {
            fs::write(lib.join(file), "fixture").unwrap();
        }
        source
    }

    fn asset(&self) -> PathBuf {
        self.root
            .join("assets")
            .join(format!("rock-v1.2.3-{TARGET}.tar.gz"))
    }

    /// Packs `source` as the release archive, plus any unsafe `extra` entries.
    fn pack(&self, source: &Path, extra: &[Extra]) {
        let file = fs::File::create(self.asset()).unwrap();
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::fast(),
        ));
        builder.follow_symlinks(false);
        builder.append_dir_all(".", source).unwrap();
        for entry in extra {
            entry.append(&mut builder);
        }
        builder.into_inner().unwrap().finish().unwrap();
        self.sign(&self.asset());
    }

    fn sign(&self, asset: &Path) {
        use sha2::Digest;
        let hash: String = sha2::Sha256::digest(fs::read(asset).unwrap())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        fs::write(
            PathBuf::from(format!("{}.sha256", asset.display())),
            format!("{hash}  {}\n", asset.file_name().unwrap().to_string_lossy()),
        )
        .unwrap();
    }

    fn assert_old(&self) {
        let result = self.run(&["run", "stable", "rock"]);
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "old");
        success(result);
        assert_eq!(
            fs::read_to_string(self.root.join("home/default-toolchain"))
                .unwrap()
                .trim(),
            "stable"
        );
        assert!(!self.root.join("home/.rockup-release-install").exists());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Archive entries rockup must refuse, written with raw headers because the
/// tar crate's checked setters reject them.
enum Extra {
    Regular(&'static str),
    Symlink(&'static str, &'static str),
    HardLink(&'static str, &'static str),
    /// A safe header name whose PAX `path` record overrides it.
    PaxPath(&'static str),
}

impl Extra {
    fn append<W: std::io::Write>(&self, builder: &mut tar::Builder<W>) {
        let (name, kind, link) = match *self {
            Extra::Regular(name) => (name, tar::EntryType::Regular, None),
            Extra::Symlink(name, target) => (name, tar::EntryType::Symlink, Some(target)),
            Extra::HardLink(name, target) => (name, tar::EntryType::Link, Some(target)),
            Extra::PaxPath(path) => {
                builder
                    .append_pax_extensions([("path", path.as_bytes())])
                    .unwrap();
                ("pax-entry", tar::EntryType::Regular, None)
            }
        };
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(kind);
        header.set_mode(0o644);
        header.set_size(0);
        header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        if let Some(link) = link {
            header.as_old_mut().linkname[..link.len()].copy_from_slice(link.as_bytes());
        }
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();
    }
}

/// Compiles a helper program once per test process.
fn compiled(name: &str, source: &str) -> PathBuf {
    static BUILT: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    let mut built = BUILT
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(path) = built.get(name) {
        return path.clone();
    }
    let dir = canonical_temp_dir().join(format!("rockup-release-tools-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{name}.rs"));
    let binary = dir.join(exe(name));
    fs::write(&file, source).unwrap();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    success(
        Command::new(rustc)
            .args(["--edition", "2021", "-o"])
            .arg(&binary)
            .arg(&file)
            .output()
            .unwrap(),
    );
    built.insert(name.to_string(), binary.clone());
    binary
}

fn fake_curl() -> PathBuf {
    compiled("fake_curl", include_str!("fixtures/fake_curl.rs"))
}

/// Installs at `path` a program that prints `text` and exits successfully,
/// standing in for a toolchain binary or a downloaded manager.
fn fake_tool(path: &Path, text: &str) {
    assert!(text.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'));
    let source = format!("fn main() {{ println!(\"{text}\"); }}\n");
    copy_executable(&compiled(&format!("fake_tool_{text}"), &source), path);
}

fn copy_executable(from: &Path, to: &Path) {
    let _ = fs::remove_file(to);
    if cfg!(unix) {
        // Keep writable executable descriptors out of the multithreaded test
        // process: concurrent child spawns can inherit them until exec and
        // cause ETXTBSY. Waiting for cp also closes its descriptor first.
        success(
            Command::new("cp")
                .arg("--")
                .arg(from)
                .arg(to)
                .output()
                .unwrap(),
        );
    } else {
        fs::copy(from, to).unwrap();
    }
}

fn copy_manager(path: &Path) {
    copy_executable(Path::new(env!("CARGO_BIN_EXE_rockup")), path);
}

/// Proxies are hard links to the manager on Unix; on Windows they may be
/// copies, so the bytes must match.
fn assert_same_file(path: &Path, expected: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (left, right) = (fs::metadata(path).unwrap(), fs::metadata(expected).unwrap());
        assert_eq!(
            (left.dev(), left.ino()),
            (right.dev(), right.ino()),
            "{} must be a hard link to {}",
            path.display(),
            expected.display()
        );
    }
    #[cfg(not(unix))]
    assert_eq!(fs::read(path).unwrap(), fs::read(expected).unwrap());
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn self_install_persists_only_manager_then_install_fetches_toolchain() {
    let fixture = Fixture::new();
    let downloaded = fixture.root.join("downloaded-rockup");
    copy_manager(&downloaded);
    success(
        fixture
            .command_for(&downloaded)
            .args(["self", "install"])
            .output()
            .unwrap(),
    );
    fs::remove_file(downloaded).unwrap();
    let manager = fixture.root.join("home/bin").join(exe("rockup"));
    assert!(!fixture.root.join("requests").exists());
    let list = fixture.command_for(&manager).arg("list").output().unwrap();
    assert!(String::from_utf8_lossy(&list.stdout)
        .starts_with("Installed toolchains (* active):\n  (none)\n"));
    success(list);
    // Proxies are hard links to the persisted manager.
    for binary in ["rock", "rockc", "rock-lsp"] {
        assert_same_file(&fixture.root.join("home/bin").join(exe(binary)), &manager);
    }
    assert!(fixture.root.join("home/env").is_file());
    // Windows updates the user PATH instead of shell startup files.
    assert_eq!(fixture.root.join("user/.profile").is_file(), cfg!(unix));
    assert!(!fixture.root.join("home/default-toolchain").exists());
    let requests = fs::read_to_string(fixture.root.join("requests")).unwrap();
    assert_eq!(requests.lines().count(), 1);
    assert!(requests.contains("https://api.github.com/"));
    let source = fixture.source("old");
    fixture.pack(&source, &[]);
    success(
        fixture
            .command_for(&manager)
            .arg("install")
            .output()
            .unwrap(),
    );
    fixture.assert_old();
}

#[test]
fn install_update_default_and_local_path() {
    let fixture = Fixture::new();
    let source = fixture.source("old");
    fixture.pack(&source, &[]);
    success(fixture.run(&["install"]));
    fixture.assert_old();
    let listed = fixture.run(&["list"]);
    assert!(String::from_utf8_lossy(&listed.stdout)
        .starts_with("Installed toolchains (* active):\n* stable\n"));
    success(listed);
    assert!(!fixture.run(&["install"]).status.success());

    fixture.source("new");
    fixture.pack(&source, &[]);
    success(fixture.run(&["update"]));
    let output = fixture.run(&["run", "stable", "rock"]);
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "new");
    success(output);
    success(fixture.run(&["default", "1.2.3"]));
    assert!(fixture
        .root
        .join("home/toolchains/v1.2.3/bin")
        .join(exe("rock"))
        .is_file());
    assert_eq!(
        fs::read_to_string(fixture.root.join("home/default-toolchain"))
            .unwrap()
            .trim(),
        "v1.2.3"
    );
    success(fixture.run(&["update", "v1.2.3"]));
    success(fixture.run(&["install", "local", "--path", source.to_str().unwrap()]));
    success(fixture.run(&["default", "local"]));
    success(fixture.run(&["remove", "v1.2.3"]));
    assert!(!fixture.root.join("home/toolchains/v1.2.3").exists());
    success(fixture.run(&["install", "1.2.3"]));

    // Long names use a tar extension record.
    let long_name = "a".repeat(140);
    fs::write(source.join(&long_name), "long name").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(source.join("bin/rock"), fs::Permissions::from_mode(0o6755)).unwrap();
    }
    fixture.pack(&source, &[]);
    success(fixture.run(&["update"]));
    assert!(fixture
        .root
        .join("home/toolchains/stable")
        .join(long_name)
        .is_file());
    // Set-id bits never survive extraction.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(fixture.root.join("home/toolchains/stable/bin/rock"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o755
        );
    }
}

#[test]
fn list_shows_available_versions_and_survives_network_failure() {
    let fixture = Fixture::new();
    let source = fixture.source("old");
    fixture.pack(&source, &[]);
    success(fixture.run(&["install", "v1.2.3"]));
    let releases: Vec<_> = [("v1.2.3", false), ("v1.3.0-rc.1", true)]
        .into_iter()
        .map(|(tag, prerelease)| {
            let archive = format!("rock-{tag}-{TARGET}.tar.gz");
            serde_json::json!({
                "tag_name": tag,
                "draft": false,
                "prerelease": prerelease,
                "assets": [{"name": archive}, {"name": format!("{archive}.sha256")}],
            })
        })
        .collect();
    fs::write(
        fixture.root.join("releases.json"),
        serde_json::to_vec(&releases).unwrap(),
    )
    .unwrap();
    let listed = fixture.run(&["list"]);
    let stdout = String::from_utf8_lossy(&listed.stdout);
    assert!(stdout.contains("* v1.2.3\n"));
    assert!(
        stdout.contains("Available versions:\n  v1.2.3 (installed)\n  v1.3.0-rc.1 (prerelease)\n")
    );
    assert!(listed.stderr.is_empty());
    success(listed);

    let offline = fixture
        .command()
        .env("FAIL_DOWNLOAD", "yes")
        .arg("list")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&offline.stdout);
    assert!(stdout.contains("* v1.2.3\n"));
    assert!(stdout.contains("Available versions:\n  (unavailable)"));
    assert!(String::from_utf8_lossy(&offline.stderr).contains("could not list available versions"));
    success(offline);
}

#[test]
fn failed_updates_preserve_installed_toolchain() {
    let fixture = Fixture::new();
    let source = fixture.source("old");
    fixture.pack(&source, &[]);
    success(fixture.run(&["default", "stable"]));

    let result = fixture
        .command()
        .env("FAIL_DOWNLOAD", "yes")
        .arg("update")
        .output()
        .unwrap();
    assert!(!result.status.success());
    fixture.assert_old();

    fs::write(fixture.asset(), "corrupted download").unwrap();
    let output = fixture.run(&["update"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("SHA-256 mismatch"));
    fixture.assert_old();

    fixture.sign(&fixture.asset());
    assert!(!fixture.run(&["update"]).status.success());
    fixture.assert_old();

    let rockc = source.join("bin").join(exe("rockc"));
    fs::remove_file(&rockc).unwrap();
    fixture.pack(&source, &[]);
    assert!(!fixture.run(&["update"]).status.success());
    fixture.assert_old();

    fs::create_dir(&rockc).unwrap();
    fixture.pack(&source, &[]);
    assert!(!fixture.run(&["update"]).status.success());
    fixture.assert_old();

    fs::remove_dir(&rockc).unwrap();
    // A shell startup file that cannot be updated fails the update.
    #[cfg(unix)]
    {
        fixture.source("new");
        fixture.pack(&source, &[]);
        fs::remove_file(fixture.root.join("user/.profile")).unwrap();
        fs::create_dir(fixture.root.join("user/.profile")).unwrap();
        assert!(!fixture.run(&["update"]).status.success());
        fixture.assert_old();
    }
}

#[test]
fn unsafe_archives_and_sidecars_are_rejected() {
    let fixture = Fixture::new();
    let source = fixture.source("old");
    fixture.pack(&source, &[]);
    success(fixture.run(&["install", "stable"]));

    for extra in [
        Extra::Regular("../escape"),
        Extra::Regular("/tmp/rockup-escape/rock"),
        Extra::Symlink("link", "../../outside"),
        Extra::HardLink("hardlink", "bin/rock"),
        Extra::PaxPath("../escape"),
    ] {
        fixture.pack(&source, &[extra]);
        let output = fixture.run(&["update"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Unsafe archive entry"));
        fixture.assert_old();
    }
    assert!(!fixture.root.join("escape").exists());

    let checksum = PathBuf::from(format!("{}.sha256", fixture.asset().display()));
    for contents in ["bad\n".into(), format!("{}  ../wrong\n", "0".repeat(64))] {
        fs::write(&checksum, contents).unwrap();
        let output = fixture.run(&["update"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid SHA-256 sidecar"));
        fixture.assert_old();
    }
}

#[test]
fn unsafe_names_are_rejected_without_network_or_outside_writes() {
    let fixture = Fixture::new();
    for args in [
        vec!["install", "../escape"],
        vec!["install", "../escape", "--path", "/nonexistent"],
        vec!["remove", "../escape"],
        vec!["default", "../escape"],
        vec!["update", "../escape"],
        vec!["run", "../escape", "rock"],
        vec![
            "target",
            "add",
            TARGET,
            "--toolchain",
            "../escape",
            "--path",
            "/nonexistent",
        ],
    ] {
        let output = fixture.run(&args);
        assert!(!output.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid toolchain name"));
    }
    let output = fixture
        .command()
        .env("ROCKUP_TOOLCHAIN", "../escape")
        .args(["proxy", "rock"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    fs::write(
        fixture.root.join("rock-toolchain.toml"),
        "[toolchain]\nchannel = '../escape'\n",
    )
    .unwrap();
    assert!(!fixture.run(&["proxy", "rock"]).status.success());
    assert!(!fixture.root.join("requests").exists());

    // Creating symlinks on Windows needs Developer Mode or elevation.
    #[cfg(unix)]
    {
        fs::create_dir_all(fixture.root.join("home/toolchains")).unwrap();
        let outside = fixture.source("outside");
        std::os::unix::fs::symlink(&outside, fixture.root.join("home/toolchains/linked")).unwrap();
        for args in [
            vec!["default", "linked"],
            vec!["remove", "linked"],
            vec!["run", "linked", "rock"],
        ] {
            let output = fixture.run(&args);
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("must not be a symlink"));
        }
        assert!(outside.join("bin/rock").is_file());
    }
}

#[test]
fn self_update_is_atomic_and_keeps_shim_destination() {
    let fixture = Fixture::new();
    let executable = fixture.root.join(exe("rockup"));
    copy_manager(&executable);
    let source = fixture.source("old");
    success(
        fixture
            .command_for(&executable)
            .args(["install", "local", "--path", source.to_str().unwrap()])
            .output()
            .unwrap(),
    );
    fs::remove_file(&executable).unwrap();
    let executable = fixture.root.join("home/bin").join(exe("rockup"));
    let rock = fixture.root.join("home/bin").join(exe("rock"));
    assert_same_file(&rock, &executable);
    success(fixture.command_for(&rock).output().unwrap());
    let original = fs::read(&executable).unwrap();
    let asset = fixture
        .root
        .join("assets")
        .join(exe(&format!("rockup-{TARGET}")));
    fake_tool(&asset, "updated-rockup");
    fixture.sign(&asset);
    fs::write(&asset, "bad checksum").unwrap();
    assert!(!fixture
        .command_for(&executable)
        .args(["self", "update"])
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(fs::read(&executable).unwrap(), original);
    fake_tool(&asset, "updated-rockup");
    fixture.sign(&asset);
    success(
        fixture
            .command_for(&executable)
            .args(["self", "update"])
            .output()
            .unwrap(),
    );
    let output = Command::new(&executable).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "updated-rockup"
    );
    success(output);
    // Proxies are relinked to the new manager, never left on the old file.
    for binary in ["rock", "rockc", "rock-lsp"] {
        assert_same_file(
            &fixture.root.join("home/bin").join(exe(binary)),
            &executable,
        );
    }
    assert!(!fixture.root.join("home/bin/.rockup-self-update").exists());
}

// The tools report resolved paths: Darwin's temp dir is a symlink into
// /private, and Windows' may use 8.3 short names.
fn canonical_temp_dir() -> std::path::PathBuf {
    dunce::canonicalize(std::env::temp_dir()).unwrap()
}
