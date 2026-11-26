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
fn normalizes_triple_aliases() {
    let canonical = |triple: &str| Target::parse(triple).unwrap().triple;
    assert_eq!(canonical("arm64-apple-darwin"), "aarch64-apple-darwin");
    assert_eq!(
        canonical("aarch64-apple-darwin23.4.0"),
        "aarch64-apple-darwin"
    );
    assert_eq!(canonical("x86_64-apple-macosx14.0"), "x86_64-apple-darwin");
    assert_eq!(
        canonical("aarch64-redhat-linux-gnu"),
        "aarch64-unknown-linux-gnu"
    );
    assert_eq!(
        canonical("x86_64-uwp-windows-msvc"),
        "x86_64-pc-windows-msvc"
    );
    assert_eq!(
        Target::parse("arm64-apple-darwin").unwrap(),
        Target::parse("aarch64-apple-darwin").unwrap()
    );
}

#[test]
fn rejects_unsupported_triples() {
    assert!(Target::parse("x86_64-pc-windows-gnu").is_err());
    assert!(Target::parse("wasm32-unknown-unknown").is_err());
    assert!(Target::parse("aarch64-pc-windows-msvc").is_err());
    assert!(Target::parse("aarch64-apple-darwin-invalid").is_err());
    assert!(Target::parse("aarch64-apple-darwininvalid").is_err());
    assert!(Target::parse("aarch64-apple-darwin23.").is_err());
    assert!(Target::parse("x86_64-apple-macos").is_ok());
}

#[test]
fn validates_cfg_vocabulary() {
    assert!(validate_cfg_pair("target_os", "windows").is_ok());
    assert!(validate_cfg_pair("target_os", "linx").is_err());
    assert!(validate_cfg_pair("target_oss", "linux").is_err());
}
