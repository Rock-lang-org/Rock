#![cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]

use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

const TARGET: &str = "x86_64-unknown-linux-gnu";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "rockup-release-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("tools")).unwrap();
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::create_dir_all(root.join("user")).unwrap();
        // A process-local transport fixture: production has no insecure URL or
        // fixture override. Also assert the required curl security flags.
        script(
            &root.join("tools/curl"),
            r#"
case " $* " in *" --disable --fail --location "*) ;; *) exit 90;; esac
case " $* " in *" --retry 3 "*) ;; *) exit 91;; esac
case " $* " in *" --proto =https --proto-redir =https "*) ;; *) exit 92;; esac
printf '%s\n' "$*" >> "$FIXTURE/requests"
[ "${FAIL_DOWNLOAD:-}" != yes ] || exit 22
output=
latest=no
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output) shift; output=$1 ;;
        --write-out) shift; latest=yes ;;
        https://*) url=$1 ;;
    esac
    shift
done
if [ "$url" = 'https://api.github.com/repos/Rock-lang-org/Rock/releases?per_page=100&page=1' ]; then
    cat "$FIXTURE/releases.json"
elif [ "$latest" = yes ]; then
    [ "$url" = https://github.com/Rock-lang-org/Rock/releases/latest ] || exit 93
    printf 'https://github.com/Rock-lang-org/Rock/releases/tag/v1.2.3'
else
    case "$url" in
        https://github.com/Rock-lang-org/Rock/releases/download/v1.2.3/*) ;;
        *) exit 93 ;;
    esac
    cp "$FIXTURE/assets/${url##*/}" "$output"
fi
"#,
        );
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
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn source(&self, version: &str) -> PathBuf {
        let source = self.root.join("source");
        fs::create_dir_all(source.join("bin")).unwrap();
        for binary in ["rock", "rockc", "rock-lsp"] {
            script(
                &source.join("bin").join(binary),
                &format!("printf '{version}\\n'"),
            );
        }
        let lib = source.join("lib/rocklib").join(TARGET);
        fs::create_dir_all(&lib).unwrap();
        // Filenames match the current ToolchainLayout contract.
        for file in [
            rock_shared::sysroot::STDLIB_ARTIFACT_FILE_NAME,
            rock_shared::sysroot::STDLIB_OBJECT_FILE_NAME,
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

    fn pack(&self, source: &Path, extra: &[&str]) {
        let result = Command::new("tar")
            .env_remove("TAR_OPTIONS")
            .arg("-czf")
            .arg(self.asset())
            .arg("-C")
            .arg(source)
            .args(extra)
            .arg(".")
            .output()
            .unwrap();
        success(result);
        self.sign(&self.asset());
    }

    fn sign(&self, asset: &Path) {
        let result = Command::new("sha256sum")
            .current_dir(asset.parent().unwrap())
            .arg(asset.file_name().unwrap())
            .output()
            .unwrap();
        assert!(result.status.success());
        fs::write(
            PathBuf::from(format!("{}.sha256", asset.display())),
            result.stdout,
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

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
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
    fs::copy(env!("CARGO_BIN_EXE_rockup"), &downloaded).unwrap();
    success(
        fixture
            .command_for(&downloaded)
            .args(["self", "install"])
            .output()
            .unwrap(),
    );
    fs::remove_file(downloaded).unwrap();
    let manager = fixture.root.join("home/bin/rockup");
    assert!(!fixture.root.join("requests").exists());
    let list = fixture.command_for(&manager).arg("list").output().unwrap();
    assert!(String::from_utf8_lossy(&list.stdout)
        .starts_with("Installed toolchains (* active):\n  (none)\n"));
    success(list);
    for binary in ["rock", "rockc", "rock-lsp"] {
        let shim = fs::read_to_string(fixture.root.join("home/bin").join(binary)).unwrap();
        assert!(shim.contains(manager.to_str().unwrap()));
    }
    assert!(fixture.root.join("home/env").is_file());
    assert!(fixture.root.join("user/.profile").is_file());
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
        .join("home/toolchains/v1.2.3/bin/rock")
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

    let long_name = "a".repeat(140);
    fs::write(source.join(&long_name), "long PAX name").unwrap();
    fs::set_permissions(source.join("bin/rock"), fs::Permissions::from_mode(0o6755)).unwrap();
    fixture.pack(&source, &["--format=pax"]);
    success(fixture.run(&["update"]));
    assert!(fixture
        .root
        .join("home/toolchains/stable")
        .join(long_name)
        .is_file());
    assert_eq!(
        fs::metadata(fixture.root.join("home/toolchains/stable/bin/rock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o755
    );
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

    fs::remove_file(source.join("bin/rockc")).unwrap();
    fixture.pack(&source, &[]);
    assert!(!fixture.run(&["update"]).status.success());
    fixture.assert_old();

    fs::create_dir(source.join("bin/rockc")).unwrap();
    fixture.pack(&source, &[]);
    assert!(!fixture.run(&["update"]).status.success());
    fixture.assert_old();

    fs::remove_dir(source.join("bin/rockc")).unwrap();
    fixture.source("new");
    fixture.pack(&source, &[]);
    fs::remove_file(fixture.root.join("user/.profile")).unwrap();
    fs::create_dir(fixture.root.join("user/.profile")).unwrap();
    assert!(!fixture.run(&["update"]).status.success());
    fixture.assert_old();
}

#[test]
fn unsafe_archives_and_sidecars_are_rejected() {
    let fixture = Fixture::new();
    let source = fixture.source("old");
    fixture.pack(&source, &[]);
    success(fixture.run(&["install", "stable"]));

    for transform in ["s|^./|../|", "s|^./|/tmp/rockup-escape/|"] {
        fixture.pack(&source, &["--transform", transform]);
        let output = fixture.run(&["update"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Unsafe archive entry"));
        fixture.assert_old();
    }

    symlink("../../outside", source.join("link")).unwrap();
    fixture.pack(&source, &[]);
    assert!(!fixture.run(&["update"]).status.success());
    fixture.assert_old();
    fs::remove_file(source.join("link")).unwrap();
    fs::hard_link(source.join("bin/rock"), source.join("hardlink")).unwrap();
    fixture.pack(&source, &[]);
    assert!(!fixture.run(&["update"]).status.success());
    fixture.assert_old();

    fs::remove_file(source.join("hardlink")).unwrap();
    fixture.pack(&source, &["--format=pax", "--pax-option=path=../escape"]);
    assert!(!fixture.run(&["update"]).status.success());
    fixture.assert_old();

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

    fs::create_dir_all(fixture.root.join("home/toolchains")).unwrap();
    let outside = fixture.source("outside");
    symlink(&outside, fixture.root.join("home/toolchains/linked")).unwrap();
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

#[test]
fn self_update_is_atomic_and_keeps_shim_destination() {
    let fixture = Fixture::new();
    let executable = fixture.root.join("rockup");
    fs::copy(env!("CARGO_BIN_EXE_rockup"), &executable).unwrap();
    let source = fixture.source("old");
    success(
        fixture
            .command_for(&executable)
            .args(["install", "local", "--path", source.to_str().unwrap()])
            .output()
            .unwrap(),
    );
    fs::remove_file(&executable).unwrap();
    let executable = fixture.root.join("home/bin/rockup");
    let shim = fs::read_to_string(fixture.root.join("home/bin/rock")).unwrap();
    assert!(shim.contains(executable.to_str().unwrap()));
    success(
        fixture
            .command_for(&fixture.root.join("home/bin/rock"))
            .output()
            .unwrap(),
    );
    let original = fs::read(&executable).unwrap();
    let asset = fixture.root.join("assets").join(format!("rockup-{TARGET}"));
    script(&asset, "printf 'updated-rockup\\n'");
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
    script(&asset, "printf 'updated-rockup\\n'");
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
    assert_eq!(
        fs::read_to_string(fixture.root.join("home/bin/rock")).unwrap(),
        shim
    );
    assert!(!fixture.root.join("home/bin/.rockup-self-update").exists());
}
