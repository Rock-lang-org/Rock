use super::*;

#[test]
fn parses_supported_triples() {
    let linux = Target::parse("x86_64-unknown-linux-gnu").unwrap();
    assert_eq!(linux.cfg_value("target_os"), Some("linux"));
    assert_eq!(linux.cfg_value("target_family"), Some("unix"));
    assert_eq!(linux.cfg_value("target_env"), Some("gnu"));

    let mac = Target::parse("aarch64-apple-darwin").unwrap();
    assert_eq!(mac.cfg_value("target_os"), Some("macos"));
    assert_eq!(mac.cfg_value("target_arch"), Some("aarch64"));
    assert_eq!(mac.cfg_value("target_env"), Some(""));

    let windows = Target::parse("x86_64-pc-windows-msvc").unwrap();
    assert_eq!(windows.cfg_value("target_family"), Some("windows"));
    assert_eq!(windows.exe_suffix(), ".exe");
}

#[test]
fn rejects_unsupported_triples() {
    assert!(Target::parse("x86_64-pc-windows-gnu").is_err());
    assert!(Target::parse("wasm32-unknown-unknown").is_err());
}

#[test]
fn validates_cfg_vocabulary() {
    assert!(validate_cfg_pair("target_os", "windows").is_ok());
    assert!(validate_cfg_pair("target_os", "linx").is_err());
    assert!(validate_cfg_pair("target_oss", "linux").is_err());
}
