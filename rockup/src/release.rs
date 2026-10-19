use std::{
    fs,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use serde::Deserialize;

use crate::{
    fsutil::make_executable,
    home::{installed_toolchain_names, validate_name, RockupHome},
    layout::ToolchainLayout,
    shell::{ensure_shell_setup, ensure_shims},
    toolchain::validate_toolchain_layout,
};

const RELEASES: &str = "https://github.com/Rock-lang-org/Rock/releases";
const RELEASES_API: &str = "https://api.github.com/repos/Rock-lang-org/Rock/releases";
const TARGET: &str = "x86_64-unknown-linux-gnu";

pub(crate) struct AvailableRelease {
    pub(crate) tag: String,
    pub(crate) prerelease: bool,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
}

pub(crate) fn available_releases() -> Result<Vec<AvailableRelease>, String> {
    check_platform()?;
    collect_available_releases(|page| {
        checked_output(curl().args([
            "--header",
            "Accept: application/vnd.github+json",
            "--header",
            "X-GitHub-Api-Version: 2022-11-28",
            "--user-agent",
            concat!("rockup/", env!("CARGO_PKG_VERSION")),
            // Listing should fail promptly offline rather than use download timeouts.
            "--retry",
            "0",
            "--connect-timeout",
            "5",
            "--max-time",
            "15",
            &format!("{}?per_page=100&page={}", RELEASES_API, page),
        ]))
        .map(|output| output.stdout)
    })
}

fn collect_available_releases(
    mut fetch_page: impl FnMut(usize) -> Result<Vec<u8>, String>,
) -> Result<Vec<AvailableRelease>, String> {
    let mut available = Vec::new();
    for page in 1.. {
        let releases: Vec<GithubRelease> = serde_json::from_slice(&fetch_page(page)?)
            .map_err(|e| format!("Invalid GitHub releases response: {}", e))?;
        let last_page = releases.len() < 100;
        for release in releases {
            if release.draft
                || release.tag_name == "stable"
                || release_name(&release.tag_name).as_ref() != Ok(&release.tag_name)
            {
                continue;
            }
            let archive = format!("rock-{}-{}.tar.gz", release.tag_name, TARGET);
            let checksum = format!("{}.sha256", archive);
            if release.assets.iter().any(|asset| asset.name == archive)
                && release.assets.iter().any(|asset| asset.name == checksum)
                && !available
                    .iter()
                    .any(|entry: &AvailableRelease| entry.tag == release.tag_name)
            {
                available.push(AvailableRelease {
                    tag: release.tag_name,
                    prerelease: release.prerelease,
                });
            }
        }
        if last_page {
            break;
        }
    }
    Ok(available)
}

fn check_platform() -> Result<(), String> {
    if cfg!(all(
        target_arch = "x86_64",
        target_os = "linux",
        target_env = "gnu"
    )) {
        Ok(())
    } else {
        Err(format!(
            "GitHub release installation only supports {}",
            TARGET
        ))
    }
}

pub(crate) fn release_name(name: &str) -> Result<String, String> {
    validate_name(name)?;
    if name == "stable" {
        return Ok(name.into());
    }
    let version = name.strip_prefix('v').unwrap_or(name);
    let core = version.split(['-', '+']).next().unwrap_or("");
    if core
        .split('.')
        .any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()))
    {
        return Err(format!(
            "Expected stable or a version such as v1.2.3, got '{}'",
            name
        ));
    }
    let tag = format!("v{}", version);
    validate_name(&tag)?;
    Ok(tag)
}

fn checked_output(command: &mut Command) -> Result<Output, String> {
    let output = command
        .output()
        .map_err(|e| format!("Failed to run {:?}: {}", command.get_program(), e))?;
    if !output.status.success() {
        return Err(format!(
            "{:?} failed ({}): {}",
            command.get_program(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output)
}

fn curl() -> Command {
    let mut command = Command::new("curl");
    // Ignore user curl configuration, and forbid HTTP even across redirects.
    command.args([
        "--disable",
        "--fail",
        "--location",
        "--silent",
        "--show-error",
        "--retry",
        "3",
        "--connect-timeout",
        "30",
        "--max-time",
        "1800",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
    ]);
    command
}

fn latest_tag_from_url(url: &str) -> Result<String, String> {
    let prefix = format!("{}/tag/", RELEASES);
    let tag = url
        .trim()
        .strip_prefix(&prefix)
        .ok_or("GitHub latest did not redirect to a release tag")?;
    let normalized = release_name(tag)?;
    if normalized != tag || tag == "stable" {
        return Err(format!("Invalid GitHub release tag '{}'", tag));
    }
    Ok(normalized)
}

fn resolve_tag(name: &str) -> Result<String, String> {
    let name = release_name(name)?;
    if name != "stable" {
        return Ok(name);
    }
    let output = checked_output(curl().args([
        "--output",
        "/dev/null",
        "--write-out",
        "%{url_effective}",
        &format!("{}/latest", RELEASES),
    ]))?;
    latest_tag_from_url(std::str::from_utf8(&output.stdout).map_err(|e| e.to_string())?)
}

fn download_verified(tag: &str, asset: &str, directory: &Path) -> Result<PathBuf, String> {
    let path = directory.join(asset);
    let checksum = directory.join(format!("{}.sha256", asset));
    let url = format!("{}/download/{}/{}", RELEASES, tag, asset);
    download_asset(&url, &path)?;
    checked_output(
        curl()
            .arg("--output")
            .arg(&checksum)
            .arg(format!("{}.sha256", url)),
    )?;
    verify_checksum(&path, &checksum, asset)?;
    Ok(path)
}

fn download_asset(url: &str, path: &Path) -> Result<(), String> {
    let mut command = curl();
    command.arg("--output").arg(path).arg(url);
    if !io::stderr().is_terminal() {
        return checked_output(&mut command).map(|_| ());
    }

    // Reuse curl's transfer accounting, including its five-second current-speed
    // average and retry handling, instead of measuring the staging file.
    command
        .args(["--no-silent", "--no-progress-bar"])
        .env("LC_ALL", "C")
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to run curl: {}", e))?;
    let result = (|| -> io::Result<()> {
        let mut source = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("Missing curl progress stream"))?;
        let mut terminal = io::stderr().lock();
        let mut pending = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = source.read(&mut buffer)?;
            for &byte in &buffer[..count] {
                if matches!(byte, b'\r' | b'\n') {
                    show_download_progress(&pending, &mut terminal)?;
                    pending.clear();
                } else {
                    pending.push(byte);
                }
            }
            if count == 0 {
                show_download_progress(&pending, &mut terminal)?;
                break;
            }
        }
        writeln!(terminal)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for curl: {}", e))?;
    result.map_err(|e| format!("Failed to display download progress: {}", e))?;
    if !status.success() {
        return Err(format!("curl failed ({}) downloading {}", status, url));
    }
    Ok(())
}

fn show_download_progress(line: &[u8], terminal: &mut impl Write) -> io::Result<()> {
    let text = String::from_utf8_lossy(line);
    let fields: Vec<_> = text.split_whitespace().collect();
    if fields.is_empty() || text.contains("% Total") || text.contains("Dload  Upload") {
        return Ok(());
    }
    // curl's standard meter: total %, total, received %, received, sent %,
    // sent, average download, average upload, three times, current speed.
    // Some curl versions leave all three time columns blank before transfer,
    // producing nine zero fields rather than the usual twelve fields.
    if fields.len() == 9 && fields.iter().all(|field| *field == "0") {
        return Ok(());
    }
    if fields.len() == 12 {
        if let Ok(percent) = fields[2].parse::<usize>() {
            if percent <= 100 {
                let filled = percent / 5;
                let bar = format!("{}{}", "=".repeat(filled), " ".repeat(20 - filled));
                let progress = if fields[1] == "0" {
                    format!("[{}] {} received", bar, fields[3])
                } else {
                    format!("[{}] {:3}% {}/{}", bar, percent, fields[3], fields[1])
                };
                write!(
                    terminal,
                    "\r\x1b[2K{} | avg {}/s | current {}/s",
                    progress, fields[6], fields[11]
                )?;
                return terminal.flush();
            }
        }
    }
    // Preserve curl errors and retry notices alongside the live meter.
    writeln!(terminal, "\r\x1b[2K{}", text)?;
    terminal.flush()
}

fn verify_checksum(path: &Path, checksum: &Path, asset: &str) -> Result<(), String> {
    let contents =
        fs::read_to_string(checksum).map_err(|e| format!("Failed to read checksum: {}", e))?;
    let line = contents.trim_end_matches('\n');
    let bytes = line.as_bytes();
    if bytes.len() != 66 + asset.len()
        || !bytes[..64].iter().all(u8::is_ascii_hexdigit)
        || bytes[64] != b' '
        || !matches!(bytes[65], b' ' | b'*')
        || &line[66..] != asset
    {
        return Err("Invalid SHA-256 sidecar: expected one sha256sum line naming the asset".into());
    }
    let output = checked_output(Command::new("sha256sum").arg("--").arg(path))?;
    let actual = std::str::from_utf8(&output.stdout).map_err(|e| e.to_string())?;
    if !actual
        .get(..64)
        .is_some_and(|hash| hash.eq_ignore_ascii_case(&line[..64]))
    {
        return Err(format!("SHA-256 mismatch for {}", asset));
    }
    Ok(())
}

// The fixed, private directory also excludes concurrent release operations.
struct Stage {
    path: PathBuf,
    keep: bool,
}

impl Stage {
    fn new(parent: &Path, label: &str) -> Result<Self, String> {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let path = parent.join(format!(".rockup-{}", label));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).map_err(|e| format!("Cannot create staging lock {} (another operation or interrupted update may exist): {}", path.display(), e))?;
        Ok(Self { path, keep: false })
    }

    fn publish(
        &mut self,
        source: &Path,
        destination: &Path,
        finish: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let backup = self.path.join("previous");
        let existed = destination.try_exists().map_err(|e| e.to_string())?;
        if existed {
            fs::rename(destination, &backup)
                .map_err(|e| format!("Failed to stage previous toolchain: {}", e))?;
        }
        let result = fs::rename(source, destination)
            .map_err(|e| format!("Failed to publish toolchain: {}", e))
            .and_then(|()| finish());
        if let Err(error) = result {
            let rollback = (|| -> std::io::Result<()> {
                if destination.exists() {
                    fs::rename(destination, source)?;
                }
                if existed {
                    fs::rename(&backup, destination)?;
                }
                Ok(())
            })();
            if let Err(rollback) = rollback {
                self.keep = true;
                return Err(format!(
                    "{}; rollback failed: {}. Recovery files retained at {}",
                    error,
                    rollback,
                    self.path.display()
                ));
            }
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn tar() -> Command {
    let mut command = Command::new("tar");
    command.env_remove("TAR_OPTIONS").env("LC_ALL", "C");
    command
}

fn validate_archive_listing(names: &str, types: &str) -> Result<(), String> {
    if names.lines().count() == 0 || names.lines().count() != types.lines().count() {
        return Err("Invalid or empty archive listing".into());
    }
    for (name, metadata) in names.lines().zip(types.lines()) {
        // Reject links, devices, FIFOs and escaped/control/non-ASCII names. GNU tar
        // lists effective names, including GNU long-name and PAX path overrides.
        if !matches!(metadata.as_bytes().first(), Some(b'-' | b'd'))
            || name.is_empty()
            || name.starts_with('/')
            || name.split('/').any(|part| part == "..")
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/._-+".contains(&c))
        {
            return Err(format!("Unsafe archive entry '{}'", name));
        }
    }
    Ok(())
}

fn extract_verified_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    let names = checked_output(
        tar()
            .args([
                "--list",
                "--gzip",
                "--absolute-names",
                "--quoting-style=escape",
                "--file",
            ])
            .arg(archive),
    )?;
    let types = checked_output(
        tar()
            .args([
                "--list",
                "--verbose",
                "--gzip",
                "--absolute-names",
                "--quoting-style=escape",
                "--file",
            ])
            .arg(archive),
    )?;
    validate_archive_listing(
        std::str::from_utf8(&names.stdout).map_err(|e| e.to_string())?,
        std::str::from_utf8(&types.stdout).map_err(|e| e.to_string())?,
    )?;
    fs::create_dir(destination).map_err(|e| e.to_string())?;
    checked_output(
        tar()
            .args([
                "--extract",
                "--gzip",
                "--no-same-owner",
                "--no-same-permissions",
                "--delay-directory-restore",
                "--file",
            ])
            .arg(archive)
            .arg("--directory")
            .arg(destination),
    )?;
    sanitize_permissions(destination)?;
    Ok(())
}

fn sanitize_permissions(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir() && !metadata.is_file() {
        return Err(format!("Unsafe extracted file {}", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if metadata.is_dir() || metadata.permissions().mode() & 0o111 != 0 {
            0o755
        } else {
            0o644
        };
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            sanitize_permissions(&entry.map_err(|e| e.to_string())?.path())?;
        }
    }
    Ok(())
}

pub(crate) fn ensure_default_installed(home: &RockupHome, name: &str) -> Result<String, String> {
    if home.toolchain_dir(name)?.exists() {
        return Ok(name.into());
    }
    let name = release_name(name)?;
    if !home.toolchain_dir(&name)?.exists() {
        install_release(home, &name, false)?;
    }
    Ok(name)
}

pub(crate) fn install_release(
    home: &RockupHome,
    name: &str,
    update: bool,
) -> Result<PathBuf, String> {
    check_platform()?;
    let name = release_name(name)?;
    let destination = home.toolchain_dir(&name)?;
    let mut stage = Stage::new(&home.root, "release-install")?;
    if destination.exists() && !update {
        return Err(format!(
            "Toolchain '{}' already exists; use rockup update {}",
            name, name
        ));
    }
    let tag = resolve_tag(&name)?;
    let asset = format!("rock-{}-{}.tar.gz", tag, TARGET);
    let archive = download_verified(&tag, &asset, &stage.path)?;
    install_archive(home, &name, &archive, &tag, &mut stage)?;
    Ok(destination)
}

fn install_archive(
    home: &RockupHome,
    name: &str,
    archive: &Path,
    tag: &str,
    stage: &mut Stage,
) -> Result<(), String> {
    let destination = home.toolchain_dir(name)?;
    let set_default =
        !home.default_toolchain_file().exists() && installed_toolchain_names(home)?.is_empty();
    let extracted = stage.path.join("toolchain");
    extract_verified_archive(archive, &extracted)?;
    let layout = ToolchainLayout::new(extracted.clone());
    validate_toolchain_layout(&layout)?;
    for binary in [&layout.rock_bin, &layout.rockc_bin, &layout.rock_lsp_bin] {
        if !binary.is_file() {
            return Err(format!("Not a regular binary: {}", binary.display()));
        }
        make_executable(binary)?;
    }
    fs::write(extracted.join(".rockup-release"), format!("{}\n", tag))
        .map_err(|e| e.to_string())?;
    fs::create_dir_all(home.toolchains_dir()).map_err(|e| e.to_string())?;
    ensure_shims(home)?;
    ensure_shell_setup(home)?;
    let default = stage.path.join("default");
    if set_default {
        fs::write(&default, format!("{}\n", name)).map_err(|e| e.to_string())?;
    }
    stage.publish(&extracted, &destination, || {
        if set_default {
            fs::rename(&default, home.default_toolchain_file())
                .map_err(|e| format!("Failed to set default: {}", e))?;
        }
        Ok(())
    })
}

pub(crate) fn self_update() -> Result<(), String> {
    check_platform()?;
    let executable = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let parent = executable
        .parent()
        .ok_or("Executable has no parent directory")?;
    let stage = Stage::new(parent, "self-update")?;
    let tag = resolve_tag("stable")?;
    let asset = format!("rockup-{}", TARGET);
    let downloaded = download_verified(&tag, &asset, &stage.path)?;
    make_executable(&downloaded)?;
    // Rename over the running executable, never truncate it. Existing shims keep
    // pointing at this path, and ensure_shims must not run from the staged binary.
    fs::rename(&downloaded, &executable)
        .map_err(|e| format!("Failed to replace {}: {}", executable.display(), e))?;
    println!("Updated rockup to {} at {}", tag, executable.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github_release(tag: &str) -> serde_json::Value {
        let archive = format!("rock-{}-{}.tar.gz", tag, TARGET);
        serde_json::json!({
            "tag_name": tag,
            "draft": false,
            "prerelease": false,
            "assets": [{"name": archive}, {"name": format!("{}.sha256", archive)}],
        })
    }

    #[test]
    fn available_versions_require_installable_tags_and_both_assets() {
        let stable = github_release("v1.2.3");
        let mut prerelease = github_release("v1.3.0-rc.1");
        prerelease["prerelease"] = true.into();
        let mut draft = github_release("v1.4.0");
        draft["draft"] = true.into();
        let mut missing_checksum = github_release("v1.2.2");
        missing_checksum["assets"].as_array_mut().unwrap().pop();
        let mut wrong_platform = github_release("v1.2.1");
        wrong_platform["assets"] = serde_json::json!([
            {"name": "rock-v1.2.1-aarch64-apple-darwin.tar.gz"},
            {"name": "rock-v1.2.1-aarch64-apple-darwin.tar.gz.sha256"},
        ]);
        let body = serde_json::to_vec(&vec![
            stable.clone(),
            prerelease,
            draft,
            missing_checksum,
            wrong_platform,
            github_release("nightly"),
            github_release("stable"),
            github_release("1.2.3"),
            github_release("v1.2.3\n"),
            stable,
        ])
        .unwrap();
        let available = collect_available_releases(|page| {
            assert_eq!(page, 1);
            Ok(body.clone())
        })
        .unwrap();
        assert_eq!(available.len(), 2);
        assert_eq!(available[0].tag, "v1.2.3");
        assert!(!available[0].prerelease);
        assert_eq!(available[1].tag, "v1.3.0-rc.1");
        assert!(available[1].prerelease);
    }

    #[test]
    fn available_versions_paginate_before_filtering() {
        let mut draft = github_release("v2.0.0");
        draft["draft"] = true.into();
        let mut pages = Vec::new();
        let available = collect_available_releases(|page| {
            pages.push(page);
            Ok(serde_json::to_vec(&match page {
                1 => vec![draft.clone(); 100],
                2 => vec![github_release("v1.0.0")],
                _ => panic!("unexpected page {}", page),
            })
            .unwrap())
        })
        .unwrap();
        assert_eq!(pages, [1, 2]);
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].tag, "v1.0.0");
    }

    #[test]
    fn available_versions_handle_empty_invalid_and_failed_responses() {
        assert!(collect_available_releases(|_| Ok(b"[]".to_vec()))
            .unwrap()
            .is_empty());
        for body in ["not json", "{}", "[{\"tag_name\":\"v1.0.0\"}]"] {
            assert!(collect_available_releases(|_| Ok(body.as_bytes().to_vec())).is_err());
        }
        assert!(matches!(
            collect_available_releases(|_| Err("offline".into())),
            Err(error) if error == "offline"
        ));
    }

    #[test]
    fn download_meter_shows_percentage_and_both_speeds() {
        let mut output = Vec::new();
        show_download_progress(
            b" 50 2048k 50 1024k 0 0 256k 0 0:00:08 0:00:04 0:00:04 300k",
            &mut output,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\r\x1b[2K[==========          ]  50% 1024k/2048k | avg 256k/s | current 300k/s"
        );
    }

    #[test]
    fn download_meter_suppresses_zero_rows_with_blank_times() {
        let mut output = Vec::new();
        for _ in 0..4 {
            show_download_progress(
                b"  0      0   0      0   0      0      0      0                              0",
                &mut output,
            )
            .unwrap();
        }
        assert!(output.is_empty());
        show_download_progress(
            b"73 38.73M 73 28.52M 0 0 203.9k 0 0:03:14 0:02:23 0:00:51 215.0k",
            &mut output,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\r\x1b[2K[==============      ]  73% 28.52M/38.73M | avg 203.9k/s | current 215.0k/s"
        );
    }

    #[test]
    fn download_meter_handles_unknown_size_and_preserves_errors() {
        let mut output = Vec::new();
        for line in [
            "% Total    % Received % Xferd  Average Speed   Time    Time     Time  Current",
            "                                 Dload  Upload   Total   Spent    Left  Speed",
            "",
        ] {
            show_download_progress(line.as_bytes(), &mut output).unwrap();
        }
        assert!(output.is_empty());
        show_download_progress(
            b"0 0 0 1024 0 0 512 0 --:--:-- 0:00:02 --:--:-- 512",
            &mut output,
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("1024 received"));
        assert!(!text.contains('%'));
        let mut output = Vec::new();
        show_download_progress(b"curl: (22) HTTP error 404", &mut output).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\r\x1b[2Kcurl: (22) HTTP error 404\n"
        );
    }

    #[test]
    fn release_names_and_latest_redirect() {
        for name in ["1.2.3", "v1.2.3"] {
            assert_eq!(release_name(name).unwrap(), "v1.2.3");
        }
        assert_eq!(release_name("stable").unwrap(), "stable");
        assert_eq!(release_name("1").unwrap(), "v1");
        assert_eq!(release_name("v1.2").unwrap(), "v1.2");
        assert_eq!(release_name("1.2.3+build.4").unwrap(), "v1.2.3+build.4");
        for name in [
            "",
            "..",
            "../stable",
            "/tmp/toolchain",
            "a/b",
            "a\\b",
            "-stable",
            "v",
            "nightly",
            "v1.2.x",
        ] {
            assert!(release_name(name).is_err(), "{name}");
        }
        assert_eq!(
            latest_tag_from_url(&format!("{RELEASES}/tag/v1.2.3")).unwrap(),
            "v1.2.3"
        );
        for url in [
            "http://github.com/Rock-lang-org/Rock/releases/tag/v1.2.3",
            "https://evil.test/tag/v1.2.3",
            "https://github.com/Rock-lang-org/Rock/releases/latest",
            "https://github.com/Rock-lang-org/Rock/releases/tag/1.2.3",
        ] {
            assert!(latest_tag_from_url(url).is_err());
        }
    }

    #[test]
    fn archive_listing_rejects_traversal_links_and_escaped_names() {
        assert!(
            validate_archive_listing("./\n./bin/rock\n", "drwx metadata\n-rwx metadata\n").is_ok()
        );
        for name in [
            "/bin/rock",
            "../rock",
            "bin/../../rock",
            "bin/evil\\nname",
            "bin/a b",
            "bin/\u{e9}",
        ] {
            assert!(
                validate_archive_listing(name, "-rwx metadata").is_err(),
                "{name}"
            );
        }
        for kind in ['l', 'h', 'b', 'c', 'p', 's'] {
            assert!(validate_archive_listing("bin/rock", &format!("{kind} metadata")).is_err());
        }
        assert!(validate_archive_listing("", "").is_err());
    }

    #[test]
    fn publication_rolls_back_and_staging_excludes_concurrent_operations() {
        let root = std::env::temp_dir().join(format!("rockup-rollback-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut stage = Stage::new(&root, "test").unwrap();
        assert!(Stage::new(&root, "test").is_err());
        let old = root.join("stable");
        let new = stage.path.join("new");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("version"), "old").unwrap();
        fs::create_dir(&new).unwrap();
        fs::write(new.join("version"), "new").unwrap();
        assert!(stage
            .publish(&new, &old, || Err("injected failure".into()))
            .is_err());
        assert_eq!(fs::read_to_string(old.join("version")).unwrap(), "old");
        assert_eq!(fs::read_to_string(new.join("version")).unwrap(), "new");
        stage.publish(&new, &old, || Ok(())).unwrap();
        assert_eq!(fs::read_to_string(old.join("version")).unwrap(), "new");
        drop(stage);
        assert!(!root.join(".rockup-test").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
