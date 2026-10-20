use super::*;

#[test]
fn macro_b_14_proc_only_product_has_an_explicit_role_and_no_runtime_surface() {
    let artifact = ProcMacroArtifact {
        artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "provider".into(), protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        compiler_version: env!("CARGO_PKG_VERSION").into(),
        host_triple: crate::sysroot::host_target_triple(), executable: "worker".into(),
        executable_identity: ProcMacroFileIdentity { bytes: 1, sha256: [1; 32] }, required_files: Vec::new(),
        capabilities: vec![ProcMacroCapability::Stdio], definition_dependencies: Vec::new(),
        exports: vec![ProcMacroExport { name: "identity".into(), identity: "provider::identity".into(),
            kind: ProcMacroKind::FunctionLike, input_shape: ProcMacroInputShape::TokenStream }],
    };
    let mut products = crate::products::CompilerProducts::for_proc_macro(
        crate::products::ProductCrateIdentity::local("provider".into()), artifact,
        crate::products::ProductSourceFingerprint::default()).unwrap();
    assert_eq!(products.role(), crate::products::ProductRole::ProcMacro);
    assert!(products.link.object_path.is_none());
    let bytes = products.to_artifact_bytes().unwrap();
    assert_eq!(crate::products::CompilerProducts::from_artifact_bytes(&bytes).unwrap().role(), crate::products::ProductRole::ProcMacro);
    let mut wrong_role = bytes.clone();
    wrong_role[28..32].copy_from_slice(&0u32.to_le_bytes());
    assert!(crate::products::CompilerProducts::from_artifact_bytes(&wrong_role).unwrap_err().contains("header"));
    let mut wrong_role = bytes.clone(); wrong_role[28..32].copy_from_slice(&9u32.to_le_bytes());
    assert!(crate::products::CompilerProducts::from_artifact_bytes(&wrong_role).is_err());
    products.identity_table.export_names.insert("leaked".into(), crate::products::ProductDefId {
        crate_id: crate::products::ProductCrateId(0), local_id: crate::products::ProductLocalDefId(0),
    });
    assert!(products.to_artifact_bytes().unwrap_err().contains("runtime"));
}

#[cfg(unix)]
#[test]
fn macro_b_14_host_identity_files_and_compatibility_reject_before_spawn() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("rock-b14-identities-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let executable = root.join("worker");
    let marker = root.join("spawned");
    std::fs::write(&executable, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut artifact = ProcMacroArtifact::for_executable("provider".into(), executable.clone(), vec![ProcMacroExport {
        name: "identity".into(), identity: "provider::identity".into(), kind: ProcMacroKind::FunctionLike,
        input_shape: ProcMacroInputShape::TokenStream,
    }]).unwrap();
    let resource = root.join("data"); std::fs::write(&resource, "owned host resource").unwrap();
    artifact.required_files.push(ProcMacroHostFile { path: resource.clone(), identity: ProcMacroFileIdentity::read(&resource).unwrap() });
    artifact.validate_host_files().unwrap();
    let mut relative = artifact.clone(); relative.executable = "worker".into(); relative.required_files[0].path = "data".into();
    assert_eq!(relative.resolve_for_host(&root.join("provider.rkca")).unwrap(), artifact);
    let request = ProcMacroRequest {
        protocol_version: PROC_MACRO_PROTOCOL_VERSION, compiler_version: env!("CARGO_PKG_VERSION").into(),
        macro_name: "identity".into(), macro_identity: "provider::identity".into(), source_module: None, expansion_id: Some(1),
        invocation_id: crate::ast::InvocationId::fresh().unwrap(), parent: None,
        output_category: crate::ast::MacroOutputCategory::Items, kind: ProcMacroKind::FunctionLike,
        header_indent: 0, block_base: None, input: encode_stream(&crate::macro_expansion::token_tree::source("")).unwrap(),
        definition_dependencies: Vec::new(), views: Default::default(),
    };
    for mutation in 0..7 {
        let mut bad = artifact.clone();
        match mutation {
            0 => bad.host_triple = "different-host".into(),
            1 => bad.compiler_version = "different-compiler".into(),
            2 => bad.protocol_version += 1,
            3 => bad.artifact_format_version -= 1,
            4 => bad.executable_identity.sha256[0] ^= 1,
            5 => bad.required_files[0].identity.sha256[0] ^= 1,
            _ => bad.exports[0].kind = ProcMacroKind::DeriveLike,
        }
        let error = run_proc_macro_process(&bad, &request, Duration::from_secs(1), false, None).unwrap_err();
        assert!(!error.contains("failed to start"), "validation must precede spawn: {error}");
        assert!(!marker.exists());
    }
    let mut unknown = request.clone(); unknown.macro_identity = "provider::missing".into();
    assert!(run_proc_macro_process(&artifact, &unknown, Duration::from_secs(1), false, None).unwrap_err().contains("unknown proc export"));
    std::fs::write(&resource, "changed resource").unwrap();
    assert!(artifact.validate_host_files().unwrap_err().contains("content identity"));
    std::fs::remove_file(&resource).unwrap();
    assert!(artifact.validate_host_files().is_err());
    std::fs::remove_dir_all(root).unwrap();
}
