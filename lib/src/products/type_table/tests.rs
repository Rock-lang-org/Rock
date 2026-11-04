use super::generic_conversion::{decode_generic_param_decls, encode_generic_param_decls};
use super::hir_schema::{SerializedHirExpr, SerializedHirExprKind};
use super::type_codec::product_def_id;
use super::type_codec::{ProductTypeDecoder, ProductTypeEncoder};
use super::type_graph_validation::validate_type_table_graph;
use super::type_rows::{ProductTypeId, ProductTypeRow, ProductTypeTable};
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, TypeVarId, VariantId};
use crate::type_services::kind::Kind;
use crate::types::{
    AssociatedTypeKey, CallableKind, CaptureKind, FunctionCapture, GenericParamDecl,
    GenericParamId, Type,
};

fn def(local: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(local))
}

fn unit_expr() -> SerializedHirExpr {
    SerializedHirExpr {
        kind: SerializedHirExprKind::Unit,
        ty: ProductTypeId(0),
        span: crate::lexer::Span::test(),
    }
}

#[test]
fn product_generic_param_decl_roundtrips_kind_name_and_id() {
    let declarations = vec![GenericParamDecl::new(
        GenericParamId {
            owner: def(7),
            index: 2,
        },
        "F",
        Kind::arrow(Kind::Type, Kind::Type),
    )];

    let mut encoder = super::ProductTypeEncoder::new();
    let encoded = encode_generic_param_decls(&declarations, &mut encoder).unwrap();
    assert_eq!(encoded[0].id.owner, product_def_id(def(7)));
    assert_eq!(encoded[0].name, "F");
    assert_eq!(encoded[0].kind, Kind::arrow(Kind::Type, Kind::Type));
    let table = encoder.finish();
    let mut decoder = super::ProductTypeDecoder::new(&table);
    assert_eq!(
        decode_generic_param_decls(encoded, &mut decoder).unwrap(),
        declarations
    );
}

#[test]
fn serialized_method_try_and_array_repeat_roundtrip() {
    let variants = vec![
        SerializedHirExprKind::MethodCall(
            Box::new(unit_expr()),
            "borrow".to_string(),
            Vec::new(),
            Some(crate::types::ReceiverMode::Shared),
            None,
        ),
        SerializedHirExprKind::Try {
            expr: Box::new(unit_expr()),
            branch_method: None,
            branch_self_receiver: Some(crate::types::ReceiverMode::Move),
            from_residual_target: None,
            output_ty: ProductTypeId(0),
            residual_ty: ProductTypeId(0),
            return_ty: ProductTypeId(0),
            control_flow_enum: def(1),
            break_variant: crate::hir::HirVariantLocation {
                owner: def(1),
                variant_id: VariantId(0),
                name: "Break".to_string(),
            },
            continue_variant: crate::hir::HirVariantLocation {
                owner: def(1),
                variant_id: VariantId(1),
                name: "Continue".to_string(),
            },
        },
        SerializedHirExprKind::ArrayRepeat(Box::new(unit_expr()), 256),
    ];

    let bytes = bincode::serialize(&variants).unwrap();
    let decoded: Vec<SerializedHirExprKind> = bincode::deserialize(&bytes).unwrap();

    assert!(matches!(
        &decoded[0],
        SerializedHirExprKind::MethodCall(_, _, _, Some(crate::types::ReceiverMode::Shared), _)
    ));
    assert!(matches!(
        &decoded[1],
        SerializedHirExprKind::Try {
            branch_self_receiver: Some(crate::types::ReceiverMode::Move),
            ..
        }
    ));
    assert!(matches!(
        &decoded[2],
        SerializedHirExprKind::ArrayRepeat(_, 256)
    ));
}

#[test]
fn product_type_table_deduplicates_equal_types() {
    let ty = Type::Struct {
        id: def(2),
        args: vec![Type::I64],
    };
    let (table, ids) = encode_types_for_test(&[ty.clone(), ty]).unwrap();

    assert_eq!(ids, vec![ProductTypeId(1), ProductTypeId(1)]);
    assert_eq!(
        table.rows.len(),
        2,
        "struct row plus i64 row should be stored once each"
    );
}

#[test]
fn product_type_table_roundtrips_nested_projection_type() {
    let ty = Type::Projection {
        ty: Box::new(Type::Struct {
            id: def(2),
            args: vec![Type::Generic(GenericParamId {
                owner: def(1),
                index: 0,
            })],
        }),
        trait_id: def(3),
        assoc_type: AssociatedTypeKey {
            owner: def(3),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        }],
    };
    let (table, ids) = encode_types_for_test(&[ty.clone()]).unwrap();
    let decoded = decode_type_for_test(&table, ids[0]).unwrap();

    assert_eq!(decoded, ty);
}

#[test]
fn product_type_table_roundtrips_function_safety() {
    let ty = Type::unsafe_function(vec![Type::I64], Type::Bool);
    let (table, ids) = encode_types_for_test(&[ty.clone()]).unwrap();
    let decoded = decode_type_for_test(&table, ids[0]).unwrap();

    assert_eq!(decoded, ty);
}

#[test]
fn product_type_table_roundtrips_callable_kind_and_captures() {
    let ty = Type::function_with_metadata(
        vec![Type::I64],
        Type::Bool,
        crate::types::FunctionSafety::Safe,
        CallableKind::FnOnce,
        vec![FunctionCapture::new(CaptureKind::Move, Type::Str)],
    );
    let (table, ids) = encode_types_for_test(std::slice::from_ref(&ty)).unwrap();
    let decoded = decode_type_for_test(&table, ids[0]).unwrap();

    assert_eq!(decoded, ty);
}

#[test]
fn product_type_table_rejects_type_vars() {
    let err = encode_types_for_test(&[Type::TypeVar(TypeVarId(0))]).unwrap_err();

    assert!(err.contains("TypeVar"), "unexpected error: {err}");
}

#[test]
fn product_type_table_rejects_nested_type_vars() {
    let ty = Type::Tuple(vec![Type::Reference {
        mutable: false,
        inner: Box::new(Type::TypeVar(TypeVarId(0))),
    }]);

    let err = encode_types_for_test(&[ty]).unwrap_err();

    assert!(err.contains("TypeVar"), "unexpected error: {err}");
}

#[test]
fn product_type_table_rejects_error_types() {
    let err = encode_types_for_test(&[Type::Error]).unwrap_err();

    assert!(err.contains("Type::Error"), "unexpected error: {err}");
}

#[test]
fn product_type_table_rejects_nested_error_types() {
    let ty = Type::Struct {
        id: def(2),
        args: vec![Type::Tuple(vec![Type::Reference {
            mutable: false,
            inner: Box::new(Type::Error),
        }])],
    };

    let err = encode_types_for_test(&[ty]).unwrap_err();

    assert!(err.contains("Type::Error"), "unexpected error: {err}");
}

#[test]
fn product_type_table_rejects_invalid_type_id() {
    let (table, _) = encode_types_for_test(&[Type::I64]).unwrap();
    let err = decode_type_for_test(&table, ProductTypeId(99)).unwrap_err();

    assert!(
        err.contains("unknown product type id 99"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_type_table_rejects_invalid_nested_type_ids() {
    let table = ProductTypeTable {
        rows: vec![ProductTypeRow::Tuple(vec![ProductTypeId(4)])],
    };
    let err = decode_type_for_test(&table, ProductTypeId(0)).unwrap_err();

    assert!(
        err.contains("unknown product type id 4"),
        "unexpected error: {err}"
    );
}

#[test]
fn portable_type_table_validates_lambda_binder_scope() {
    let valid = ProductTypeTable {
        rows: vec![
            ProductTypeRow::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            },
            ProductTypeRow::Lambda {
                params: vec![Kind::Type],
                body: ProductTypeId(0),
            },
        ],
    };
    validate_type_table_graph(&valid).unwrap();

    let invalid = ProductTypeTable {
        rows: vec![ProductTypeRow::BoundVar {
            depth: 0,
            index: 0,
            kind: Kind::Type,
        }],
    };
    let error = validate_type_table_graph(&invalid).unwrap_err();
    assert!(error.contains("out-of-scope"), "{error}");
}

#[test]
fn portable_type_table_rejects_excessive_type_depth() {
    let mut rows = vec![ProductTypeRow::I64];
    for index in 0..crate::products::MAX_PRODUCT_ARTIFACT_TYPE_DEPTH {
        rows.push(ProductTypeRow::Slice(ProductTypeId(index as u32)));
    }
    let error = validate_type_table_graph(&ProductTypeTable { rows }).unwrap_err();

    assert!(error.contains("type depth limit exceeded"), "{error}");
}

#[test]
fn product_type_table_rejects_recursive_type_id() {
    let table = ProductTypeTable {
        rows: vec![ProductTypeRow::Slice(ProductTypeId(0))],
    };
    let err = decode_type_for_test(&table, ProductTypeId(0)).unwrap_err();

    assert!(
        err.contains("recursive product type id 0"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_type_table_decodes_product_def_ids_as_def_ids() {
    let ty = Type::Enum {
        id: DefId::new(CrateId(7), LocalDefId(9)),
        args: Vec::new(),
    };
    let (table, ids) = encode_types_for_test(&[ty.clone()]).unwrap();
    let row = &table.rows[ids[0].0 as usize];

    assert!(format!("{row:?}").contains("ProductCrateId(7)"));
    assert_eq!(decode_type_for_test(&table, ids[0]).unwrap(), ty);
}

#[test]
fn serialized_product_artifact_roundtrips_empty_products() {
    let products = crate::products::CompilerProducts {
        crate_identity: crate::products::ProductCrateIdentity::local("empty".to_string()),
        identity_table: crate::products::ProductIdentityTable::default(),
        interface: crate::products::ProductInterface::default(),
        bodies: crate::products::ProductBodies::default(),
        link: crate::products::ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: crate::products::ProductSourceFingerprint::default(),
        infix_precedence: std::collections::BTreeMap::new(),
        proc_macros: Vec::new(),
    };
    let bytes = products.to_artifact_bytes().unwrap();
    let roundtrip = crate::products::CompilerProducts::from_artifact_bytes(&bytes).unwrap();

    assert_eq!(roundtrip.crate_identity.name, "empty");
}

#[test]
fn serialized_product_artifact_roundtrips_interface_without_metadata_functions() {
    let product_id = crate::products::ProductDefId {
        crate_id: crate::products::ProductCrateId(0),
        local_id: crate::products::ProductLocalDefId(1),
    };
    let mut interface = crate::products::ProductInterface::default();
    interface.functions.insert(
        product_id,
        crate::products::ProductFunctionInterface {
            id: def(1),
            name: "identity".to_string(),
            generic_params: Vec::new(),
            generic_bounds: std::collections::HashMap::new().into(),
            params: vec![Type::I64],
            ret_type: Type::I64,
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
            constant_arithmetic: None,
        },
    );
    let products = crate::products::CompilerProducts {
        crate_identity: crate::products::ProductCrateIdentity::local("interface".to_string()),
        identity_table: crate::products::ProductIdentityTable::default(),
        interface,
        bodies: crate::products::ProductBodies::default(),
        link: crate::products::ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: crate::products::ProductSourceFingerprint::default(),
        infix_precedence: std::collections::BTreeMap::new(),
        proc_macros: Vec::new(),
    };

    let bytes = products.to_artifact_bytes().unwrap();
    let decoded = crate::products::CompilerProducts::from_artifact_bytes(&bytes).unwrap();

    assert_eq!(decoded.interface.functions[&product_id].name, "identity");
    assert!(decoded.bodies.functions.is_empty());
}

#[test]
fn typed_predicate_artifact_roundtrips_constructor_arguments() {
    let product_id = crate::products::ProductDefId {
        crate_id: crate::products::ProductCrateId(0),
        local_id: crate::products::ProductLocalDefId(10),
    };
    let trait_id = def(10);
    let parent_id = def(11);
    let constructor_id = def(12);
    let target = GenericParamDecl::new(
        crate::types::GenericParamId {
            owner: trait_id,
            index: 0,
        },
        "F",
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        ),
    );
    let predicate = crate::types::Predicate::Trait {
        subject: Type::Generic(target.id),
        trait_id: parent_id,
        args: vec![Type::Constructor {
            id: constructor_id,
            flavor: crate::types::NominalTypeKind::Struct,
        }],
    };
    let mut interface = crate::products::ProductInterface::default();
    interface.traits.insert(
        product_id,
        crate::products::ProductTraitInterface {
            id: trait_id,
            name: "Applicative".to_string(),
            generic_params: Vec::new(),
            target: Some(target.clone()),
            predicates: vec![predicate.clone()],
            associated_types: Vec::new(),
            methods: std::collections::BTreeMap::new(),
            signatures: std::collections::HashMap::new(),
        },
    );
    let products = crate::products::CompilerProducts {
        crate_identity: crate::products::ProductCrateIdentity::local("predicates".to_string()),
        identity_table: crate::products::ProductIdentityTable::default(),
        interface,
        bodies: crate::products::ProductBodies::default(),
        link: crate::products::ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: crate::products::ProductSourceFingerprint::default(),
        infix_precedence: std::collections::BTreeMap::new(),
        proc_macros: Vec::new(),
    };

    let bytes = products.to_artifact_bytes().unwrap();
    let decoded = crate::products::CompilerProducts::from_artifact_bytes(&bytes).unwrap();
    let decoded_trait = &decoded.interface.traits[&product_id];

    assert_eq!(decoded_trait.target, Some(target));
    assert_eq!(decoded_trait.predicates, vec![predicate]);
}

#[test]
fn serialized_product_artifact_uses_type_table_shell_for_empty_products() {
    let products = crate::products::CompilerProducts {
        crate_identity: crate::products::ProductCrateIdentity::local("empty".to_string()),
        identity_table: crate::products::ProductIdentityTable::default(),
        interface: crate::products::ProductInterface::default(),
        bodies: crate::products::ProductBodies::default(),
        link: crate::products::ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: crate::products::ProductSourceFingerprint::default(),
        infix_precedence: std::collections::BTreeMap::new(),
        proc_macros: Vec::new(),
    };

    let bytes = products.to_artifact_bytes().unwrap();
    let artifact =
        crate::products::artifact_codec::tests::serialized_artifact_from_bytes_for_test(&bytes)
            .unwrap();

    assert_eq!(
        artifact.format_version,
        crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION
    );
    assert_eq!(artifact.type_table.rows.len(), 0);
    assert_eq!(artifact.products.crate_identity.name, "empty");
}

pub(in crate::products) fn encode_types_for_test(
    types: &[Type],
) -> Result<(ProductTypeTable, Vec<ProductTypeId>), String> {
    let mut encoder = ProductTypeEncoder::new();
    let ids = encoder.encode_types(types)?;
    Ok((encoder.finish(), ids))
}

pub(in crate::products) fn decode_type_for_test(
    table: &ProductTypeTable,
    id: ProductTypeId,
) -> Result<Type, String> {
    ProductTypeDecoder::new(table).decode_type(id)
}

pub(crate) fn artifact_type_table_len_for_test(bytes: &[u8]) -> Result<usize, String> {
    let artifact =
        crate::products::artifact_codec::tests::serialized_artifact_from_bytes_for_test(bytes)?;
    Ok(artifact.type_table.rows.len())
}

pub(crate) fn artifact_type_table_contains_type_for_test(
    bytes: &[u8],
    expected: &Type,
) -> Result<bool, String> {
    let artifact =
        crate::products::artifact_codec::tests::serialized_artifact_from_bytes_for_test(bytes)?;
    let mut decoder = ProductTypeDecoder::new(&artifact.type_table);

    for index in 0..artifact.type_table.rows.len() {
        if decoder.decode_type(ProductTypeId(index as u32))? == *expected {
            return Ok(true);
        }
    }

    Ok(false)
}
