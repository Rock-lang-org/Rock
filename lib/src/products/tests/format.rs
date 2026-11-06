use super::*;

#[test]
fn compiler_products_roundtrip_and_validate_constant_arithmetic_summaries() {
    use super::super::{ArithmeticSummary, ArithmeticType};
    let mut products = products_from_hir(&resolved_hir_for_products()).unwrap();
    let id = *products.interface.functions.keys().next().unwrap();
    products
        .interface
        .functions
        .get_mut(&id)
        .unwrap()
        .constant_arithmetic = Some(ArithmeticSummary::Integer(42));
    let bytes = products.to_artifact_bytes().unwrap();
    let decoded = CompilerProducts::from_artifact_bytes(&bytes).unwrap();
    assert!(matches!(
        decoded.interface.functions[&id].constant_arithmetic,
        Some(ArithmeticSummary::Integer(42))
    ));

    for invalid in [
        ArithmeticSummary::Argument(usize::MAX),
        ArithmeticSummary::Integer(i128::MIN),
        ArithmeticSummary::Cast(
            Box::new(ArithmeticSummary::Integer(0)),
            ArithmeticType::Bool,
        ),
        ArithmeticSummary::Intrinsic("IntCheckedAdd".to_string(), ArithmeticType::I8, Vec::new()),
    ] {
        products
            .interface
            .functions
            .get_mut(&id)
            .unwrap()
            .constant_arithmetic = Some(invalid);
        assert!(products
            .to_artifact_bytes()
            .unwrap_err()
            .contains("constant arithmetic summary"));
    }
}

#[test]
fn compiler_products_record_dependency_identities_by_product_crate_id() {
    let hir = resolved_hir_for_products();
    let std = ProductDependencyIdentity::artifact_object(
        "std".to_string(),
        PathBuf::from("build/std.rkca"),
    );
    let math = ProductDependencyIdentity::artifact_object(
        "math".to_string(),
        PathBuf::from("build/math.rkca"),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        vec![std, math],
        BTreeMap::from([
            (
                ProductCrateId(7),
                ProductCrateIdentity::local("std".to_string()),
            ),
            (
                ProductCrateId(42),
                ProductCrateIdentity::local("math".to_string()),
            ),
        ]),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    assert_eq!(products.dependencies.len(), 2);
    assert_eq!(products.identity_table.local_crate, Some(ProductCrateId(0)));
    assert_eq!(
        products
            .identity_table
            .dependencies
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![ProductCrateId(7), ProductCrateId(42)]
    );
    assert_eq!(
        products.identity_table.dependencies[&ProductCrateId(7)].name,
        "std"
    );
    assert_eq!(
        products.identity_table.dependencies[&ProductCrateId(42)].name,
        "math"
    );
}

#[test]
fn compiler_products_freshness_metadata_includes_schema_source_and_dependency_identity() {
    let hir = resolved_hir_for_products();
    let source_fingerprint = ProductSourceFingerprint {
        manifest_hash: Some("manifest".to_string()),
        source_hash: Some("source".to_string()),
        loaded_files: vec![PathBuf::from("src/main.rk")],
    };
    let mut crate_identity = ProductCrateIdentity::local("demo".to_string());
    crate_identity.target_triple = Some("test-target".to_string());

    let products = CompilerProducts::from_resolved_hir(
        crate_identity,
        &hir,
        vec![ProductDependencyIdentity::artifact_object(
            "std".to_string(),
            PathBuf::from("build/std.rkca"),
        )],
        BTreeMap::from([(
            ProductCrateId(7),
            ProductCrateIdentity::local("std".to_string()),
        )]),
        source_fingerprint.clone(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let freshness = products.freshness_metadata();

    assert_eq!(
        freshness.artifact_format_version,
        PRODUCT_ARTIFACT_FORMAT_VERSION
    );
    assert_eq!(freshness.target_triple.as_deref(), Some("test-target"));
    assert_eq!(freshness.source_fingerprint, source_fingerprint);
    assert_eq!(freshness.dependencies.len(), 1);
    assert_eq!(freshness.dependencies[0].name, "std");
    assert_eq!(
        freshness.dependencies[0]
            .crate_identity
            .as_ref()
            .map(|identity| identity.name.as_str()),
        Some("std")
    );
    assert!(!freshness.compiler_version.is_empty());
}

#[test]
fn compiler_products_reject_freshness_metadata_mismatch_on_load() {
    let hir = resolved_hir_for_products();
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let mut artifact =
        serialized_artifact_from_bytes_for_test(&products.to_artifact_bytes().unwrap()).unwrap();
    artifact.products.freshness.source_fingerprint.source_hash = Some("tampered".to_string());
    let bytes =
        encode_product_artifact(&ProductArtifactHeader::from_products(&products), &artifact)
            .unwrap();

    let err = CompilerProducts::from_artifact_bytes(&bytes).unwrap_err();

    assert!(
        err.contains("freshness metadata mismatch"),
        "expected freshness mismatch rejection, got {err}"
    );
}

#[test]
fn compiler_products_roundtrip_preserves_dependency_capabilities() {
    let hir = resolved_hir_for_products();
    let capabilities = ProductDependencyCapabilities {
        metadata: true,
        bodies: true,
        link: ProductDependencyLinkCapability::Object,
    };
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        vec![ProductDependencyIdentity {
            name: "dep".to_string(),
            artifact_path: PathBuf::from("build/dep.rkca"),
            capabilities: capabilities.clone(),
        }],
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let bytes = products.to_artifact_bytes().unwrap();
    let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();

    assert_eq!(roundtrip.dependencies[0].capabilities, capabilities);
}

#[test]
fn compiler_products_write_and_read_product_artifact_bytes() {
    let hir = resolved_hir_for_products();
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        vec![ProductDependencyIdentity::artifact_object(
            "dep".to_string(),
            PathBuf::from("build/dep.rkca"),
        )],
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let bytes = products.to_artifact_bytes().unwrap();
    let decoded = CompilerProducts::from_artifact_bytes(&bytes).unwrap();
    let identity_id = decoded.identity_table.export_names["identity"];

    assert!(decoded.interface.functions.contains_key(&identity_id));
    assert!(decoded.bodies.functions.contains_key(&identity_id));
    assert_eq!(decoded.dependencies[0].name, "dep");
    assert_eq!(
        decoded.dependencies[0].artifact_path,
        PathBuf::from("build/dep.rkca")
    );
}

#[test]
fn compiler_products_v25_artifact_contains_type_table() {
    let hir = resolved_hir_for_products();
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let bytes = products.to_artifact_bytes().unwrap();

    assert!(super::super::type_table::tests::artifact_type_table_len_for_test(&bytes).unwrap() > 0);
}

#[test]
fn product_artifact_schema_does_not_serialize_compiler_products_directly() {
    let hir = resolved_hir_for_products();
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let bytes = products.to_artifact_bytes().unwrap();

    let direct_products_decode = bincode::deserialize::<CompilerProducts>(&bytes);

    assert!(
        direct_products_decode.is_err(),
        "v25 artifacts must not be raw CompilerProducts payloads"
    );
    assert!(super::super::type_table::tests::artifact_type_table_len_for_test(&bytes).unwrap() > 0);
}

#[test]
fn compiler_products_preserve_proc_macro_exports() {
    use std::collections::BTreeMap;

    let mut products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("macros".to_string()),
        identity_table: ProductIdentityTable::default(),
        interface: crate::products::ProductInterface::default(),
        bodies: ProductBodies::default(),
        link: ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: BTreeMap::new(),
        proc_macros: Vec::new(),
    };
    products
        .proc_macros
        .push(crate::macro_expansion::proc_macro::ProcMacroArtifact {
            artifact_format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
            crate_identity: "macros".to_string(),
            protocol_version: crate::macro_expansion::proc_macro::PROC_MACRO_PROTOCOL_VERSION,
            host_triple: "x86_64-unknown-linux-gnu".to_string(),
            executable: "macro-host".into(),
            capabilities: vec![crate::macro_expansion::proc_macro::ProcMacroCapability::Stdio],
            exports: vec![crate::macro_expansion::proc_macro::ProcMacroExport {
                name: "make_main".to_string(),
                identity: "macros::make_main".to_string(),
                kind: crate::macro_expansion::proc_macro::ProcMacroKind::FunctionLike,
                input_shape: crate::macro_expansion::proc_macro::ProcMacroInputShape::TokenStream,
            }],
        });

    let bytes = products.to_artifact_bytes().unwrap();
    let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();

    assert_eq!(roundtrip.proc_macros[0].exports[0].name, "make_main");
}

#[test]
fn product_artifact_rejects_unsupported_format_version() {
    let bytes = ProductArtifactPreamble {
        magic: PRODUCT_ARTIFACT_MAGIC,
        format_version: PRODUCT_ARTIFACT_FORMAT_VERSION + 1,
        header_len: 0,
        payload_len: 0,
    }
    .encode();

    let err = CompilerProducts::from_artifact_bytes(&bytes).unwrap_err();

    assert!(err.contains("Unsupported product artifact format"));
}

#[test]
fn product_artifact_preamble_rejects_invalid_magic_and_declared_sizes() {
    let invalid_magic = ProductArtifactPreamble {
        magic: *b"NOTRKCA!",
        format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
        header_len: 0,
        payload_len: 0,
    }
    .encode();
    assert!(CompilerProducts::from_artifact_bytes(&invalid_magic)
        .unwrap_err()
        .contains("magic"));

    let oversized_header = ProductArtifactPreamble {
        magic: PRODUCT_ARTIFACT_MAGIC,
        format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
        header_len: MAX_PRODUCT_ARTIFACT_HEADER_BYTES + 1,
        payload_len: 0,
    }
    .encode();
    let error = CompilerProducts::from_artifact_bytes(&oversized_header).unwrap_err();
    assert!(error.contains("header bytes limit exceeded"), "{error}");

    let oversized_payload = ProductArtifactPreamble {
        magic: PRODUCT_ARTIFACT_MAGIC,
        format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
        header_len: 0,
        payload_len: MAX_PRODUCT_ARTIFACT_BYTES,
    }
    .encode();
    let error = CompilerProducts::from_artifact_bytes(&oversized_payload).unwrap_err();
    assert!(error.contains("artifact bytes limit exceeded"), "{error}");

    let mismatched_size = ProductArtifactPreamble {
        magic: PRODUCT_ARTIFACT_MAGIC,
        format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
        header_len: 1,
        payload_len: 0,
    }
    .encode();
    let error = CompilerProducts::from_artifact_bytes(&mismatched_size).unwrap_err();
    assert!(error.contains("size mismatch"), "{error}");
}

#[test]
fn product_artifact_header_decodes_without_reading_semantic_payload() {
    let hir = resolved_hir_for_products();
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("header-only".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .unwrap();
    let mut bytes = products.to_artifact_bytes().unwrap();
    *bytes.last_mut().expect("payload byte") ^= 0xff;
    let mut cursor = std::io::Cursor::new(&bytes);

    let (_, header) = read_product_artifact_header(&mut cursor, bytes.len() as u64).unwrap();

    assert_eq!(header.crate_identity.name, "header-only");
    assert!(CompilerProducts::from_artifact_bytes(&bytes).is_err());
}

#[test]
fn product_artifact_header_rejects_oversized_strings() {
    let mut products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local(
            "x".repeat(MAX_PRODUCT_ARTIFACT_STRING_BYTES + 1),
        ),
        identity_table: ProductIdentityTable::default(),
        interface: Default::default(),
        bodies: Default::default(),
        link: Default::default(),
        dependencies: Vec::new(),
        source_fingerprint: Default::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };

    let error = products.to_artifact_bytes().unwrap_err();

    assert!(error.contains("string bytes limit exceeded"), "{error}");

    products.crate_identity = ProductCrateIdentity::local("payload-strings".to_string());
    products
        .infix_precedence
        .insert(
            "x".repeat(MAX_PRODUCT_ARTIFACT_STRING_BYTES + 1),
            crate::products::ProductInfixOperator {
                precedence: 1,
                declaring_crate: "payload-strings".to_string(),
            },
        );
    let error = products.to_artifact_bytes().unwrap_err();
    assert!(error.contains("string bytes limit exceeded"), "{error}");

    products.infix_precedence.clear();
    let artifact =
        type_table::products_to_artifact(&products, PRODUCT_ARTIFACT_FORMAT_VERSION).unwrap();
    let error = type_table::validate_portable_artifact_limits_with_initial_string_bytes(
        &artifact,
        MAX_PRODUCT_ARTIFACT_TOTAL_STRING_BYTES,
    )
    .unwrap_err();
    assert!(
        error.contains("total string bytes limit exceeded"),
        "{error}"
    );
}

#[test]
fn compiler_products_reject_format_23_product_artifacts() {
    let bytes = ProductArtifactPreamble {
        magic: PRODUCT_ARTIFACT_MAGIC,
        format_version: 23,
        header_len: 0,
        payload_len: 0,
    }
    .encode();

    let err = CompilerProducts::from_artifact_bytes(&bytes).unwrap_err();

    assert!(
        err.contains("Unsupported product artifact format 23")
            && err.contains(&format!("expected {}", PRODUCT_ARTIFACT_FORMAT_VERSION)),
        "expected format-23 rejection, got {err}"
    );
}

#[test]
fn compiler_products_rejects_previous_format_before_full_deserialize() {
    let previous_version = PRODUCT_ARTIFACT_FORMAT_VERSION - 1;
    let bytes = ProductArtifactPreamble {
        magic: PRODUCT_ARTIFACT_MAGIC,
        format_version: previous_version,
        header_len: 0,
        payload_len: 0,
    }
    .encode();

    let err = CompilerProducts::from_artifact_bytes(&bytes).unwrap_err();

    assert!(
        err.contains(&format!(
            "Unsupported product artifact format {previous_version}"
        )) && err.contains(&format!("expected {PRODUCT_ARTIFACT_FORMAT_VERSION}")),
        "expected format-{previous_version} rejection before full deserialize, got {err}"
    );
}

#[test]
fn compiler_products_rejects_unsupported_format_before_full_deserialize() {
    let unsupported_version = PRODUCT_ARTIFACT_FORMAT_VERSION + 1;
    let bytes = ProductArtifactPreamble {
        magic: PRODUCT_ARTIFACT_MAGIC,
        format_version: unsupported_version,
        header_len: 0,
        payload_len: 0,
    }
    .encode();

    let err = CompilerProducts::from_artifact_bytes(&bytes).unwrap_err();

    assert!(
        err.contains(&format!(
            "Unsupported product artifact format {unsupported_version}"
        )) && err.contains(&format!("expected {}", PRODUCT_ARTIFACT_FORMAT_VERSION)),
        "expected format-{unsupported_version} rejection before full deserialize, got {err}"
    );
}

#[test]
fn product_artifact_format_version_matches_shared_contract() {
    assert_eq!(PRODUCT_ARTIFACT_FORMAT_VERSION, 52);
    assert_eq!(
        PRODUCT_ARTIFACT_FORMAT_VERSION,
        rock_shared::sysroot::PRODUCT_ARTIFACT_FORMAT_VERSION
    );
}
