use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

use crate::{
    constants::{
        ENV_FILE_NAME, ROCKC_BIN_NAME, ROCK_BIN_NAME, ROCK_LSP_BIN_NAME, SHELL_INIT_END_MARKER,
        SHELL_INIT_START_MARKER,
    },
    fsutil::make_executable,
    home::{default_rockup_home, home_dir, RockupHome},
};

pub(crate) fn ensure_shims(home: &RockupHome) -> Result<(), String> {
    fs::create_dir_all(home.bin_dir()).map_err(|e| {
        format!(
            "Failed to create rockup bin directory {}: {}",
            home.bin_dir().display(),
            e
        )
    })?;

    let rockup_path = home.bin_dir().join("rockup");
    match fs::symlink_metadata(&rockup_path) {
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!(
                "Refusing to use a non-regular rockup executable at {}",
                rockup_path.display()
            ));
        }
        Ok(_) => {} // Existing managers are replaced only by explicit self update.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let executable = env::current_exe()
                .map_err(|e| format!("Failed to resolve rockup executable: {}", e))?;
            let mut source = fs::File::open(&executable)
                .map_err(|e| format!("Failed to open {}: {}", executable.display(), e))?;
            let mut destination = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&rockup_path)
                .map_err(|e| format!("Failed to create {}: {}", rockup_path.display(), e))?;
            // Never leave shims pointing at a temporary bootstrap download.
            let result = std::io::copy(&mut source, &mut destination)
                .map_err(|e| format!("Failed to copy rockup: {}", e))
                .and_then(|_| make_executable(&rockup_path));
            if let Err(error) = result {
                let _ = fs::remove_file(&rockup_path);
                return Err(error);
            }
        }
        Err(error) => {
            return Err(format!(
                "Failed to inspect {}: {}",
                rockup_path.display(),
                error
            ));
        }
    }
    let rockup_path = dunce::canonicalize(&rockup_path)
        .map_err(|e| format!("Failed to canonicalize installed rockup: {}", e))?;
    write_shim(home, ROCK_BIN_NAME, &rockup_path)?;
    write_shim(home, ROCKC_BIN_NAME, &rockup_path)?;
    write_shim(home, ROCK_LSP_BIN_NAME, &rockup_path)?;
    Ok(())
}

pub(crate) fn ensure_shell_setup(home: &RockupHome) -> Result<(), String> {
    ensure_env_file(home)?;
    let _ = ensure_shell_config(home)?;
    Ok(())
}

fn ensure_env_file(home: &RockupHome) -> Result<PathBuf, String> {
    let user_home = home_dir()?;
    ensure_env_file_with(home, &user_home)
}

pub(crate) fn ensure_env_file_with(home: &RockupHome, user_home: &Path) -> Result<PathBuf, String> {
    let env_file = home.env_file();
    if let Some(parent) = env_file.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create rockup home directory {}: {}",
                parent.display(),
                e
            )
        })?;
    }

    let script = render_env_script(home, user_home);
    let existing = fs::read_to_string(&env_file).unwrap_or_default();
    if existing != script {
        fs::write(&env_file, script)
            .map_err(|e| format!("Failed to write env file {}: {}", env_file.display(), e))?;
    }

    Ok(env_file)
}

fn ensure_shell_config(home: &RockupHome) -> Result<PathBuf, String> {
    let user_home = home_dir()?;
    ensure_shell_config_with(home, &user_home, env::var("SHELL").ok().as_deref())
}

pub(crate) fn ensure_shell_config_with(
    home: &RockupHome,
    user_home: &Path,
    shell_env: Option<&str>,
) -> Result<PathBuf, String> {
    let shell_config_paths = shell_config_paths(user_home, shell_env);
    let block = shell_init_block(home, user_home);
    for shell_config_path in &shell_config_paths {
        let existing = fs::read_to_string(shell_config_path).unwrap_or_default();
        let updated = upsert_shell_init_block(&existing, &block);

        if updated != existing {
            if let Some(parent) = shell_config_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create shell config directory {}: {}",
                        parent.display(),
                        e
                    )
                })?;
            }

            fs::write(shell_config_path, updated).map_err(|e| {
                format!(
                    "Failed to update shell config {}: {}",
                    shell_config_path.display(),
                    e
                )
            })?;
        }
    }

    Ok(shell_config_paths[0].clone())
}

pub(crate) fn shell_config_path(user_home: &Path, shell_env: Option<&str>) -> PathBuf {
    match shell_env
        .and_then(|shell| Path::new(shell).file_name())
        .and_then(|name| name.to_str())
    {
        Some("bash") => user_home.join(".bashrc"),
        Some("zsh") => user_home.join(".zshrc"),
        _ => user_home.join(".profile"),
    }
}

fn shell_config_paths(user_home: &Path, shell_env: Option<&str>) -> Vec<PathBuf> {
    let primary = shell_config_path(user_home, shell_env);
    let mut paths = vec![primary.clone()];

    for candidate in [user_home.join(".zshrc"), user_home.join(".bashrc")] {
        if candidate != primary && candidate.exists() {
            paths.push(candidate);
        }
    }

    paths
}

fn shell_init_block(home: &RockupHome, user_home: &Path) -> String {
    let home_expr = rockup_home_expr(home, user_home);

    format!(
        concat!(
            "{start}\n",
            "export ROCKUP_HOME=\"${{ROCKUP_HOME:-{home_expr}}}\"\n",
            "if [ -f \"$ROCKUP_HOME/{env_file}\" ]; then\n",
            "    . \"$ROCKUP_HOME/{env_file}\"\n",
            "fi\n",
            "{end}\n"
        ),
        start = SHELL_INIT_START_MARKER,
        home_expr = home_expr,
        env_file = ENV_FILE_NAME,
        end = SHELL_INIT_END_MARKER,
    )
}

pub(crate) fn render_env_script(home: &RockupHome, user_home: &Path) -> String {
    let home_expr = rockup_home_expr(home, user_home);

    format!(
        concat!(
            "export ROCKUP_HOME=\"${{ROCKUP_HOME:-{home_expr}}}\"\n",
            "case \":$PATH:\" in\n",
            "    *\":$ROCKUP_HOME/bin:\"*) ;;\n",
            "    *) export PATH=\"$ROCKUP_HOME/bin:$PATH\" ;;\n",
            "esac\n"
        ),
        home_expr = home_expr,
    )
}

fn rockup_home_expr(home: &RockupHome, user_home: &Path) -> String {
    if home.root == default_rockup_home(user_home) {
        "$HOME/.rockup".to_string()
    } else {
        shell_double_quote_escape(&home.root.to_string_lossy())
    }
}

fn upsert_shell_init_block(existing: &str, block: &str) -> String {
    if let Some(start) = existing.find(SHELL_INIT_START_MARKER) {
        if let Some(relative_end) = existing[start..].find(SHELL_INIT_END_MARKER) {
            let end = start + relative_end + SHELL_INIT_END_MARKER.len();
            let prefix = existing[..start].trim_end_matches('\n');
            let suffix = existing[end..].trim_start_matches('\n');
            let mut updated = String::new();

            if !prefix.is_empty() {
                updated.push_str(prefix);
                updated.push_str("\n\n");
            }

            updated.push_str(block.trim_end_matches('\n'));

            if !suffix.is_empty() {
                updated.push_str("\n\n");
                updated.push_str(suffix);
            } else {
                updated.push('\n');
            }

            return updated;
        }
    }

    let trimmed = existing.trim_end_matches('\n');
    if trimmed.is_empty() {
        return block.to_string();
    }

    format!("{}\n\n{}", trimmed, block)
}

fn write_shim(home: &RockupHome, binary: &str, rockup_path: &Path) -> Result<(), String> {
    let shim_path = home.bin_dir().join(binary);
    let script = format!(
        "#!/bin/sh\nexec {} proxy {} \"$@\"\n",
        shell_quote(rockup_path.as_os_str()),
        shell_quote(OsString::from(binary).as_os_str()),
    );

    fs::write(&shim_path, script)
        .map_err(|e| format!("Failed to write shim {}: {}", shim_path.display(), e))?;
    make_executable(&shim_path)?;

    Ok(())
}

pub(crate) fn toolchain_prefixed_path(toolchain_bin_dir: &Path) -> Result<OsString, String> {
    let mut paths = vec![toolchain_bin_dir.to_path_buf()];
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    env::join_paths(paths).map_err(|e| format!("Failed to construct PATH: {}", e))
}

fn shell_quote(value: &OsStr) -> String {
    let text = value.to_string_lossy();
    format!("'{}'", text.replace('\'', "'\"'\"'"))
}

fn shell_double_quote_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
}

pub(crate) fn print_current_shell_activation_hint() -> Result<(), String> {
    let executable = dunce::canonicalize(RockupHome::resolve()?.bin_dir().join("rockup"))
        .map_err(|e| format!("Failed to canonicalize rockup executable path: {}", e))?;
    eprintln!(
        "For this shell, run: eval \"$({} env)\"",
        shell_quote(executable.as_os_str())
    );
    Ok(())
}
