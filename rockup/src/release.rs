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
    shell::{ensure_shell_setup, ensure_shims, link_proxies},
    toolchain::validate_toolchain_layout,
};

const RELEASES: &str = "https://github.com/Rock-lang-org/Rock/releases";
const RELEASES_API: &str = "https://api.github.com/repos/Rock-lang-org/Rock/releases";
/// Hosts with published release assets.
const RELEASE_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

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
    let target = release_target()?;
    collect_available_releases(&target, |page| {
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
    target: &str,
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
            let archive = format!("rock-{}-{}.tar.gz", release.tag_name, target);
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
    release_target().map(|_| ())
}

/// This host's release target, if releases are published for it.
fn release_target() -> Result<String, String> {
    let host = crate::layout::host_target_triple();
    if RELEASE_TARGETS.contains(&host.as_str()) {
        Ok(host)
    } else {
        Err(format!(
            "GitHub release installation supports {}, not {}",
            RELEASE_TARGETS.join(", "),
            host
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
        if cfg!(windows) { "NUL" } else { "/dev/null" },
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

fn sha256_hex(path: &Path) -> Result<String, String> {
    use sha2::Digest;
    let mut file =
        fs::File::open(path).map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
    let mut hasher = sha2::Sha256::new();
    io::copy(&mut file, &mut hasher)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect())
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
    let actual = sha256_hex(path)?;
    if !actual.eq_ignore_ascii_case(&line[..64]) {
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
        #[cfg_attr(not(unix), allow(unused_mut))]
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

fn open_archive(archive: &Path) -> Result<tar::Archive<flate2::read::GzDecoder<fs::File>>, String> {
    let file = fs::File::open(archive)
        .map_err(|e| format!("Failed to open {}: {}", archive.display(), e))?;
    Ok(tar::Archive::new(flate2::read::GzDecoder::new(file)))
}

/// The relative path of a safe archive entry. Only regular files and
/// directories with plain ASCII names are accepted: no links, devices, FIFOs,
/// absolute or escaping paths, control or non-ASCII characters.
fn safe_entry_path(kind: tar::EntryType, name: &[u8]) -> Result<PathBuf, String> {
    let shown = String::from_utf8_lossy(name);
    let text = std::str::from_utf8(name)
        .ok()
        .filter(|text| {
            !text.is_empty()
                && !text.starts_with('/')
                && text
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"/._-+".contains(&c))
                && !text.split('/').any(|part| part == "..")
        })
        .ok_or_else(|| format!("Unsafe archive entry '{}'", shown))?;
    if !matches!(kind, tar::EntryType::Regular | tar::EntryType::Directory) {
        return Err(format!("Unsafe archive entry '{}'", shown));
    }
    Ok(text
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect())
}

fn extract_verified_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    let invalid = |e: io::Error| format!("Invalid archive {}: {}", archive.display(), e);
    // Validate every entry before writing anything.
    let mut count = 0;
    for entry in open_archive(archive)?.entries().map_err(invalid)? {
        let entry = entry.map_err(invalid)?;
        safe_entry_path(entry.header().entry_type(), &entry.path_bytes())?;
        count += 1;
    }
    if count == 0 {
        return Err("Invalid or empty archive".into());
    }
    fs::create_dir(destination).map_err(|e| e.to_string())?;
    for entry in open_archive(archive)?.entries().map_err(invalid)? {
        let mut entry = entry.map_err(invalid)?;
        let kind = entry.header().entry_type();
        let path = destination.join(safe_entry_path(kind, &entry.path_bytes())?);
        if kind == tar::EntryType::Directory {
            fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // A duplicate entry must not overwrite an earlier file.
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("Failed to create {}: {}", path.display(), e))?;
        io::copy(&mut entry, &mut file).map_err(invalid)?;
        if entry.header().mode().is_ok_and(|mode| mode & 0o111 != 0) {
            drop(file);
            make_executable(&path)?;
        }
    }
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
    let asset = format!("rock-{}-{}.tar.gz", tag, release_target()?);
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
    let executable = dunce::canonicalize(std::env::current_exe().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let parent = executable
        .parent()
        .ok_or("Executable has no parent directory")?;
    let stage = Stage::new(parent, "self-update")?;
    let tag = resolve_tag("stable")?;
    let target = release_target()?;
    let asset = format!("rockup-{}{}", target, std::env::consts::EXE_SUFFIX);
    let downloaded = download_verified(&tag, &asset, &stage.path)?;
    make_executable(&downloaded)?;
    replace_running_executable(&downloaded, &executable)?;
    // Proxies are hard links or copies of the manager: point them at the new
    // file. This only links files; it never runs the downloaded binary.
    if let Ok(home) = RockupHome::resolve() {
        let installed = home.bin_dir().join(crate::constants::exe_name("rockup"));
        if dunce::canonicalize(&installed).is_ok_and(|installed| installed == executable) {
            link_proxies(&home, &executable)?;
        }
    }
    println!("Updated rockup to {} at {}", tag, executable.display());
    Ok(())
}

/// Puts `new` at `executable`, which is running. Unix renames over it; the
/// running process keeps the old file. Windows cannot replace a running
/// executable but can rename it, so the old one moves aside first.
fn replace_running_executable(new: &Path, executable: &Path) -> Result<(), String> {
    let replace =
        |error: std::io::Error| format!("Failed to replace {}: {}", executable.display(), error);
    if cfg!(windows) {
        // Not in the stage: the running image cannot be deleted until it
        // exits, so the next update removes it instead.
        let previous = executable.with_file_name(".rockup-previous.exe");
        let _ = fs::remove_file(&previous);
        fs::rename(executable, &previous).map_err(replace)?;
        if let Err(error) = fs::rename(new, executable) {
            let _ = fs::rename(&previous, executable);
            return Err(replace(error));
        }
        Ok(())
    } else {
        // Rename over the running executable, never truncate it.
        fs::rename(new, executable).map_err(replace)
    }
}

#[cfg(test)]
mod tests;
