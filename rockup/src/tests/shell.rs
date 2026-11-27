use std::fs;

use crate::{
    constants::ENV_FILE_NAME,
    home::default_rockup_home,
    shell::{ensure_env_file_with, ensure_shell_config_with, ensure_shims, shell_config_path},
};

use super::support::{fake_home, temp_test_dir};

#[test]
fn test_ensure_shims_persists_manager_and_preserves_existing_binary() {
    let temp_dir = temp_test_dir("persistent_manager");
    let home = fake_home(temp_dir.join("custom home"));
    ensure_shims(&home).unwrap();
    let installed = home.bin_dir().join("rockup");
    assert_eq!(
        fs::read(&installed).unwrap(),
        fs::read(std::env::current_exe().unwrap()).unwrap()
    );
    for binary in ["rock", "rockc", "rock-lsp"] {
        let shim = fs::read_to_string(home.bin_dir().join(binary)).unwrap();
        assert!(shim.contains(installed.canonicalize().unwrap().to_str().unwrap()));
    }
    fs::write(&installed, b"existing manager").unwrap();
    ensure_shims(&home).unwrap();
    assert_eq!(fs::read(&installed).unwrap(), b"existing manager");
    fs::remove_dir_all(temp_dir).unwrap();
}

#[cfg(unix)]
#[test]
fn test_ensure_shims_rejects_manager_symlink() {
    let temp_dir = temp_test_dir("manager_symlink");
    let home = fake_home(temp_dir.join("home"));
    fs::create_dir_all(home.bin_dir()).unwrap();
    std::os::unix::fs::symlink("missing", home.bin_dir().join("rockup")).unwrap();
    assert!(ensure_shims(&home).is_err());
    assert!(!home.bin_dir().join("rock").exists());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn test_ensure_env_file_writes_default_home_setup() {
    let temp_dir = temp_test_dir("env_file_default_home");
    let user_home = temp_dir.join("user-home");
    let home = fake_home(default_rockup_home(&user_home));

    let env_path = ensure_env_file_with(&home, &user_home).unwrap();
    let contents = fs::read_to_string(&env_path).unwrap();

    assert_eq!(env_path, home.root.join(ENV_FILE_NAME));
    assert!(contents.contains("export ROCKUP_HOME=\"${ROCKUP_HOME:-$HOME/.rockup}\""));
    assert!(contents.contains("export PATH=\"$ROCKUP_HOME/bin:$PATH\""));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_ensure_shell_config_writes_bashrc_with_default_home_setup() {
    let temp_dir = temp_test_dir("shell_config_default_home");
    let user_home = temp_dir.join("user-home");
    let home = fake_home(default_rockup_home(&user_home));

    let config_path = ensure_shell_config_with(&home, &user_home, Some("/bin/bash")).unwrap();
    let contents = fs::read_to_string(&config_path).unwrap();

    assert_eq!(
        config_path,
        shell_config_path(&user_home, Some("/bin/bash"))
    );
    assert!(contents.contains("export ROCKUP_HOME=\"${ROCKUP_HOME:-$HOME/.rockup}\""));
    assert!(contents.contains("if [ -f \"$ROCKUP_HOME/env\" ]; then"));
    assert!(contents.contains(". \"$ROCKUP_HOME/env\""));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_ensure_shell_config_is_idempotent() {
    let temp_dir = temp_test_dir("shell_config_idempotent");
    let user_home = temp_dir.join("user-home");
    let home = fake_home(default_rockup_home(&user_home));

    let config_path = ensure_shell_config_with(&home, &user_home, Some("/bin/bash")).unwrap();
    let first = fs::read_to_string(&config_path).unwrap();
    ensure_shell_config_with(&home, &user_home, Some("/bin/bash")).unwrap();
    let second = fs::read_to_string(&config_path).unwrap();

    assert_eq!(first, second);

    let _ = fs::remove_dir_all(temp_dir);
}

// Expects the home path verbatim; on Windows the shell escaping doubles its backslashes.
#[cfg(unix)]
#[test]
fn test_ensure_shell_config_uses_custom_rockup_home() {
    let temp_dir = temp_test_dir("shell_config_custom_home");
    let user_home = temp_dir.join("user-home");
    let home = fake_home(temp_dir.join("custom-rockup-home"));

    let config_path = ensure_shell_config_with(&home, &user_home, Some("/bin/zsh")).unwrap();
    let contents = fs::read_to_string(&config_path).unwrap();

    assert_eq!(config_path, shell_config_path(&user_home, Some("/bin/zsh")));
    assert!(contents.contains(&format!(
        "export ROCKUP_HOME=\"${{ROCKUP_HOME:-{}}}\"",
        home.root.display()
    )));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_ensure_shell_config_also_updates_existing_zshrc() {
    let temp_dir = temp_test_dir("shell_config_existing_zshrc");
    let user_home = temp_dir.join("user-home");
    let home = fake_home(default_rockup_home(&user_home));
    let zshrc_path = user_home.join(".zshrc");

    fs::create_dir_all(&user_home).unwrap();
    fs::write(&zshrc_path, "# existing zsh config\n").unwrap();

    let config_path = ensure_shell_config_with(&home, &user_home, Some("/bin/bash")).unwrap();
    let zshrc_contents = fs::read_to_string(&zshrc_path).unwrap();

    assert_eq!(
        config_path,
        shell_config_path(&user_home, Some("/bin/bash"))
    );
    assert!(zshrc_contents.contains("# >>> rockup initialize >>>"));
    assert!(zshrc_contents.contains(". \"$ROCKUP_HOME/env\""));

    let _ = fs::remove_dir_all(temp_dir);
}
