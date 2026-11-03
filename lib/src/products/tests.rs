use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use crate::collect::resolver::ResolverTables;
use crate::hir::{
    AcceptedHir, AcceptedHirFunction, AcceptedHirProgram, HirAssociatedTypeDecl, HirBlock,
    HirCallTarget, HirClosureCapture, HirClosureCaptureKind, HirEnum, HirExpr, HirExtern,
    HirFieldLocation, HirFunction, HirFunctionSig, HirImpl, HirImplFor, HirLanguageItems,
    HirNameTables, HirParam, HirPattern, HirProgram, HirProgramFor, HirStruct,
    HirStructLiteralField, HirStructPatternField, HirTrait, HirTraitFor, HirVarRef, HirVarTarget,
    HirVariantLocation,
};
use crate::ids::IdGen;
use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
use crate::infer::ResolvedHirProgram;
use crate::language_items::{
    DropLanguageItems, IndexLanguageItems, IndexMutLanguageItems, LanguageItems,
    RangeLanguageItems, SizedLanguageItems, TryLanguageItems,
};
use crate::lexer::Span;
use crate::products::type_table;
use crate::products::{
    encode_product_artifact, read_product_artifact_header, serialized_artifact_from_bytes_for_test,
    CompilerProducts, ProductArtifactHeader, ProductArtifactPreamble, ProductBodies,
    ProductCrateId, ProductCrateIdentity, ProductDefId, ProductDependencyCapabilities,
    ProductDependencyIdentity, ProductDependencyLinkCapability, ProductIdentityTable,
    ProductLanguageItems, ProductLinkData, ProductLinkRecord, ProductLocalDefId,
    ProductSourceFingerprint, MAX_PRODUCT_ARTIFACT_BYTES, MAX_PRODUCT_ARTIFACT_HEADER_BYTES,
    MAX_PRODUCT_ARTIFACT_STRING_BYTES, MAX_PRODUCT_ARTIFACT_TOTAL_STRING_BYTES,
    PRODUCT_ARTIFACT_FORMAT_VERSION, PRODUCT_ARTIFACT_MAGIC,
};
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, Type};

#[test]
fn product_def_id_preserves_def_id_raw_parts() {
    let def_id = DefId::new(CrateId(7), LocalDefId(42));

    let product_id = ProductDefId::from(def_id);

    assert_eq!(product_id.crate_id, ProductCrateId(7));
    assert_eq!(product_id.local_id, ProductLocalDefId(42));
}

#[test]
fn product_def_id_is_usable_as_stable_btree_key() {
    let first = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(2),
    };
    let second = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(1),
    };

    let mut map = BTreeMap::new();
    map.insert(first, "second item".to_string());
    map.insert(second, "first item".to_string());

    let keys = map.keys().copied().collect::<Vec<_>>();
    assert_eq!(keys, vec![second, first]);
}

fn product_def_id(local_id: u32) -> ProductDefId {
    ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(local_id),
    }
}

fn generic_params(owner: DefId, names: &[&str]) -> Vec<GenericParamDecl> {
    GenericParamDecl::type_params(owner, names.iter().copied())
}

fn resolved_hir_with_language_items(
    language_items: LanguageItems<DefId>,
    current_def_ids: BTreeSet<DefId>,
) -> ResolvedHirProgram {
    let mut traits = HashMap::new();
    if let Some(items) = &language_items.sized {
        traits.insert(
            items.trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: items.trait_id,
                name: "SizedProvider".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
    }
    if let Some(items) = &language_items.index {
        let key = Type::Generic(GenericParamId {
            owner: items.trait_id,
            index: 0,
        });
        let self_ty = Type::Generic(GenericParamId {
            owner: items.trait_id,
            index: 1,
        });
        traits.insert(
            items.trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: items.trait_id,
                name: "IndexProvider".to_string(),
                generic_params: generic_params(items.trait_id, &["Key"]),
                associated_types: vec![HirAssociatedTypeDecl {
                    id: items.output_id,
                    name: "Output".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                }],
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "index".to_string(),
                    HirFunctionSig {
                        id: items.method_id,
                        name: "index".to_string(),
                        generic_params: Vec::new(),
                        params: vec![
                            Type::Reference {
                                mutable: false,
                                inner: Box::new(self_ty.clone()),
                            },
                            key.clone(),
                        ],
                        ret: Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::Projection {
                                ty: Box::new(self_ty),
                                trait_id: items.trait_id,
                                assoc_type: AssociatedTypeKey {
                                    owner: items.trait_id,
                                    assoc_type_id: items.output_id,
                                },
                                trait_args: vec![key],
                            }),
                        },
                        generic_bounds: HashMap::new().into(),
                        self_receiver: Some(crate::types::ReceiverMode::Shared),
                        is_unsafe: false,
                    },
                )]),
            },
        );
    }
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        traits,
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        language_items,
        &HashMap::new(),
    );
    let accepted =
        AcceptedHirProgram::try_from(program).expect("product fixture must construct accepted HIR");
    ResolvedHirProgram::from_accepted(
        accepted,
        ResolverTables::default(),
        current_def_ids,
        CrateId(0),
        IdGen::new(),
    )
}

fn products_from_hir(hir: &ResolvedHirProgram) -> Result<CompilerProducts, String> {
    CompilerProducts::from_resolved_hir_with_remap(
        ProductCrateIdentity::local("test".to_string()),
        hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .map(|(products, _)| products)
}

#[test]
fn product_artifact_roundtrip_preserves_complete_language_item_registry() {
    let language_items = ProductLanguageItems {
        fold: None,
        sized: Some(SizedLanguageItems {
            trait_id: product_def_id(17),
        }),
        drop: Some(DropLanguageItems {
            trait_id: product_def_id(23),
            method_id: product_def_id(29),
        }),
        index: Some(IndexLanguageItems {
            trait_id: product_def_id(31),
            output_id: AssocTypeId(37),
            method_id: product_def_id(41),
        }),
        index_mut: Some(IndexMutLanguageItems {
            trait_id: product_def_id(11),
            output_id: AssocTypeId(0),
            method_id: product_def_id(12),
        }),
        fn_once: None,
        fn_mut: None,
        fn_trait: None,
        send: None,
        sync: None,
        try_protocol: Some(TryLanguageItems {
            try_trait_id: product_def_id(43),
            output_id: AssocTypeId(47),
            residual_id: AssocTypeId(53),
            branch_method_id: product_def_id(59),
            from_residual_trait_id: product_def_id(61),
            from_residual_method_id: product_def_id(67),
            control_flow_enum_id: product_def_id(71),
            break_variant_id: VariantId(73),
            continue_variant_id: VariantId(79),
        }),
        range: Some(RangeLanguageItems {
            enum_id: product_def_id(83),
            full_variant_id: VariantId(0),
            from_variant_id: VariantId(1),
            to_variant_id: VariantId(2),
            to_inclusive_variant_id: VariantId(3),
            exclusive_variant_id: VariantId(4),
            inclusive_variant_id: VariantId(5),
        }),
    };
    let mut products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("language-items".to_string()),
        identity_table: ProductIdentityTable::default(),
        interface: super::ProductInterface {
            language_items: language_items.clone(),
            ..super::ProductInterface::default()
        },
        bodies: ProductBodies::default(),
        link: ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: BTreeMap::new(),
        proc_macros: Vec::new(),
    };
    for (local_id, receiver) in [
        None,
        Some(crate::types::ReceiverMode::Shared),
        Some(crate::types::ReceiverMode::Mut),
        Some(crate::types::ReceiverMode::Move),
    ]
    .into_iter()
    .enumerate()
    {
        let local_id = local_id as u32 + 100;
        products.interface.functions.insert(
            product_def_id(local_id),
            super::ProductFunctionInterface {
                id: DefId::new(CrateId(0), LocalDefId(local_id)),
                name: format!("receiver_{local_id}"),
                generic_params: Vec::new(),
                generic_bounds: HashMap::new().into(),
                params: Vec::new(),
                ret_type: Type::Unit,
                is_curried: false,
                is_method: receiver.is_some(),
                self_receiver: receiver,
                is_unsafe: false,
                constant_arithmetic: None,
            },
        );
    }
    let body_id = DefId::new(CrateId(0), LocalDefId(104));
    let mut body = test_function(body_id, "body_receiver", Vec::new());
    body.is_method = true;
    body.self_receiver = Some(crate::types::ReceiverMode::Move);
    products
        .bodies
        .functions
        .insert(product_def_id(104), accept_function(body));
    products.interface.traits.insert(
        product_def_id(105),
        super::ProductTraitInterface {
            target: None,
            predicates: Vec::new(),
            id: DefId::new(CrateId(0), LocalDefId(105)),
            name: "ReceiverTrait".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: BTreeMap::new(),
            signatures: HashMap::from([(
                "receive".to_string(),
                HirFunctionSig {
                    id: DefId::new(CrateId(0), LocalDefId(106)),
                    name: "receive".to_string(),
                    generic_params: Vec::new(),
                    params: Vec::new(),
                    ret: Type::Unit,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(crate::types::ReceiverMode::Mut),
                    is_unsafe: false,
                },
            )]),
        },
    );
    products.identity_table.local_crate = Some(ProductCrateId(0));

    let bytes = products.to_artifact_bytes().expect("artifact serializes");
    let decoded = CompilerProducts::from_artifact_bytes(&bytes).expect("artifact decodes");

    assert_eq!(decoded.interface.language_items, language_items);
    assert_eq!(
        decoded
            .interface
            .functions
            .values()
            .map(|function| function.self_receiver)
            .collect::<Vec<_>>(),
        vec![
            None,
            Some(crate::types::ReceiverMode::Shared),
            Some(crate::types::ReceiverMode::Mut),
            Some(crate::types::ReceiverMode::Move),
        ]
    );
    assert_eq!(
        decoded.bodies.functions[&product_def_id(104)].self_receiver,
        Some(crate::types::ReceiverMode::Move)
    );
    assert_eq!(
        decoded.interface.traits[&product_def_id(105)].signatures["receive"].self_receiver,
        Some(crate::types::ReceiverMode::Mut)
    );
}

#[test]
fn product_mapping_omits_dependency_sized_bundle() {
    let dependency_sized_trait = DefId::new(CrateId(9), LocalDefId(17));
    let hir = resolved_hir_with_language_items(
        LanguageItems {
            sized: Some(SizedLanguageItems {
                trait_id: dependency_sized_trait,
            }),
            ..LanguageItems::default()
        },
        BTreeSet::new(),
    );

    let products = products_from_hir(&hir).expect("dependency-only bundle is omitted");

    assert_eq!(
        products.interface.language_items,
        ProductLanguageItems::default()
    );
}

#[test]
fn product_mapping_maps_current_index_bundle_preserving_output_id() {
    let index_trait = DefId::new(CrateId(0), LocalDefId(17));
    let index_method = DefId::new(CrateId(0), LocalDefId(19));
    let hir = resolved_hir_with_language_items(
        LanguageItems {
            index: Some(IndexLanguageItems {
                trait_id: index_trait,
                output_id: AssocTypeId(23),
                method_id: index_method,
            }),
            ..LanguageItems::default()
        },
        BTreeSet::from([index_trait, index_method]),
    );

    let products = products_from_hir(&hir).expect("current bundle maps");

    assert_eq!(
        products.interface.language_items.index,
        Some(IndexLanguageItems {
            trait_id: ProductDefId::from(index_trait),
            output_id: AssocTypeId(23),
            method_id: ProductDefId::from(index_method),
        })
    );
}

#[test]
fn product_mapping_rejects_mixed_drop_bundle() {
    let current_drop_trait = DefId::new(CrateId(0), LocalDefId(17));
    let dependency_drop_method = DefId::new(CrateId(9), LocalDefId(19));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        LanguageItems {
            drop: Some(DropLanguageItems {
                trait_id: current_drop_trait,
                method_id: dependency_drop_method,
            }),
            ..LanguageItems::default()
        },
        &HashMap::new(),
    );

    let error = super::product_language_items_from_program(
        &program,
        &BTreeSet::from([current_drop_trait]),
        &super::ProductIdRemap::new(),
    )
    .expect_err("mixed bundle is an invariant error");

    assert!(error.contains("drop"));
}

#[test]
fn product_mapping_rejects_current_index_mut_without_current_index() {
    let dependency_index_trait = DefId::new(CrateId(9), LocalDefId(17));
    let dependency_index_method = DefId::new(CrateId(9), LocalDefId(19));
    let index_mut_trait = DefId::new(CrateId(0), LocalDefId(23));
    let index_mut_method = DefId::new(CrateId(0), LocalDefId(29));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        LanguageItems {
            index: Some(IndexLanguageItems {
                trait_id: dependency_index_trait,
                output_id: AssocTypeId(0),
                method_id: dependency_index_method,
            }),
            index_mut: Some(IndexMutLanguageItems {
                trait_id: index_mut_trait,
                output_id: AssocTypeId(0),
                method_id: index_mut_method,
            }),
            ..LanguageItems::default()
        },
        &HashMap::new(),
    );

    let mut remap = super::ProductIdRemap::new();
    for id in [index_mut_trait, index_mut_method] {
        remap.insert(
            ProductDefId::from(id),
            BTreeSet::from([ProductDefId::from(id)]),
        );
    }
    let error = super::product_language_items_from_program(
        &program,
        &BTreeSet::from([index_mut_trait, index_mut_method]),
        &remap,
    )
    .expect_err("current IndexMut without current Index is an invariant error");

    assert_eq!(
            error,
            "language item index/index_mut provider pair mixes current-crate and dependency definitions"
        );
}

#[test]
fn product_mapping_rejects_current_index_without_current_index_mut() {
    let index_trait = DefId::new(CrateId(0), LocalDefId(17));
    let index_method = DefId::new(CrateId(0), LocalDefId(19));
    let dependency_index_mut_trait = DefId::new(CrateId(9), LocalDefId(23));
    let dependency_index_mut_method = DefId::new(CrateId(9), LocalDefId(29));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        LanguageItems {
            index: Some(IndexLanguageItems {
                trait_id: index_trait,
                output_id: AssocTypeId(0),
                method_id: index_method,
            }),
            index_mut: Some(IndexMutLanguageItems {
                trait_id: dependency_index_mut_trait,
                output_id: AssocTypeId(0),
                method_id: dependency_index_mut_method,
            }),
            ..LanguageItems::default()
        },
        &HashMap::new(),
    );

    let mut remap = super::ProductIdRemap::new();
    for id in [index_trait, index_method] {
        remap.insert(
            ProductDefId::from(id),
            BTreeSet::from([ProductDefId::from(id)]),
        );
    }
    let error = super::product_language_items_from_program(
        &program,
        &BTreeSet::from([index_trait, index_method]),
        &remap,
    )
    .expect_err("current Index without current IndexMut is an invariant error");

    assert_eq!(
            error,
            "language item index/index_mut provider pair mixes current-crate and dependency definitions"
        );
}

#[test]
fn product_mapping_maps_current_index_mut_bundle_preserving_output_id() {
    let index_trait = DefId::new(CrateId(0), LocalDefId(17));
    let index_method = DefId::new(CrateId(0), LocalDefId(19));
    let index_mut_trait = DefId::new(CrateId(0), LocalDefId(23));
    let index_mut_method = DefId::new(CrateId(0), LocalDefId(29));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        LanguageItems {
            index: Some(IndexLanguageItems {
                trait_id: index_trait,
                output_id: AssocTypeId(31),
                method_id: index_method,
            }),
            index_mut: Some(IndexMutLanguageItems {
                trait_id: index_mut_trait,
                output_id: AssocTypeId(37),
                method_id: index_mut_method,
            }),
            ..LanguageItems::default()
        },
        &HashMap::new(),
    );
    let mut remap = super::ProductIdRemap::new();
    for id in [index_trait, index_method, index_mut_trait, index_mut_method] {
        remap.insert(
            ProductDefId::from(id),
            BTreeSet::from([ProductDefId::from(id)]),
        );
    }

    let items = super::product_language_items_from_program(
        &program,
        &BTreeSet::from([index_trait, index_method, index_mut_trait, index_mut_method]),
        &remap,
    )
    .expect("current index pair maps");

    assert_eq!(
        items.index_mut,
        Some(IndexMutLanguageItems {
            trait_id: ProductDefId::from(index_mut_trait),
            output_id: AssocTypeId(37),
            method_id: ProductDefId::from(index_mut_method),
        })
    );
}

#[test]
fn remap_expr_location_product_ids_remaps_new_reference_sidecars() {
    let old_struct = DefId::new(CrateId(0), LocalDefId(1));
    let old_function = DefId::new(CrateId(0), LocalDefId(2));
    let old_extern = DefId::new(CrateId(0), LocalDefId(3));
    let new_struct = DefId::new(CrateId(7), LocalDefId(11));
    let new_function = DefId::new(CrateId(7), LocalDefId(12));
    let new_extern = DefId::new(CrateId(7), LocalDefId(13));
    let mut remap = BTreeMap::new();
    remap.insert(
        ProductDefId::from(old_struct),
        BTreeSet::from([ProductDefId::from(new_struct)]),
    );
    remap.insert(
        ProductDefId::from(old_function),
        BTreeSet::from([ProductDefId::from(new_function)]),
    );
    remap.insert(
        ProductDefId::from(old_extern),
        BTreeSet::from([ProductDefId::from(new_extern)]),
    );

    let mut struct_expr = HirExpr {
        kind: crate::hir::HirExprKindFor::StructLiteral(
            "Box".to_string(),
            Some(old_struct),
            vec![HirStructLiteralField {
                name: "value".to_string(),
                value: HirExpr {
                    kind: crate::hir::HirExprKindFor::ResolvedVar(HirVarRef {
                        name: "make".to_string(),
                        target: HirVarTarget::Function(old_function),
                    }),
                    ty: Type::I64,
                    span: Span::test(),
                },
                field: Some(HirFieldLocation {
                    owner: old_struct,
                    field_id: FieldId(0),
                    name: "value".to_string(),
                }),
            }],
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let mut extern_expr = HirExpr {
        kind: crate::hir::HirExprKindFor::ResolvedVar(HirVarRef {
            name: "puts".to_string(),
            target: HirVarTarget::Extern(old_extern),
        }),
        ty: Type::I64,
        span: Span::test(),
    };

    super::remap_expr_location_product_ids(&mut struct_expr, &remap);
    super::remap_expr_location_product_ids(&mut extern_expr, &remap);

    let crate::hir::HirExprKindFor::StructLiteral(_, Some(struct_id), fields) = &struct_expr.kind
    else {
        panic!("expected struct literal");
    };
    assert_eq!(*struct_id, new_struct);
    assert_eq!(fields[0].field.as_ref().unwrap().owner, new_struct);
    let crate::hir::HirExprKindFor::ResolvedVar(reference) = &fields[0].value.kind else {
        panic!("expected resolved var field value");
    };
    assert_eq!(reference.target, HirVarTarget::Function(new_function));
    let crate::hir::HirExprKindFor::ResolvedVar(reference) = &extern_expr.kind else {
        panic!("expected resolved extern var");
    };
    assert_eq!(reference.target, HirVarTarget::Extern(new_extern));
}

#[test]
fn remap_expr_location_product_ids_remaps_call_target_sidecars() {
    let old_function = DefId::new(CrateId(0), LocalDefId(2));
    let old_extern = DefId::new(CrateId(0), LocalDefId(3));
    let new_function = DefId::new(CrateId(7), LocalDefId(12));
    let new_extern = DefId::new(CrateId(7), LocalDefId(13));
    let mut remap = BTreeMap::new();
    remap.insert(
        ProductDefId::from(old_function),
        BTreeSet::from([ProductDefId::from(new_function)]),
    );
    remap.insert(
        ProductDefId::from(old_extern),
        BTreeSet::from([ProductDefId::from(new_extern)]),
    );

    let mut function_call = HirExpr {
        kind: crate::hir::HirExprKindFor::Call(
            Box::new(HirExpr {
                kind: crate::hir::HirExprKindFor::Var("make".to_string()),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            }),
            Vec::new(),
            Some(HirCallTarget::Function(old_function)),
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let mut extern_call = HirExpr {
        kind: crate::hir::HirExprKindFor::Call(
            Box::new(HirExpr {
                kind: crate::hir::HirExprKindFor::Var("puts".to_string()),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            }),
            Vec::new(),
            Some(HirCallTarget::Extern(old_extern)),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    super::remap_expr_location_product_ids(&mut function_call, &remap);
    super::remap_expr_location_product_ids(&mut extern_call, &remap);

    let crate::hir::HirExprKindFor::Call(_, _, Some(function_target)) = &function_call.kind else {
        panic!("expected function call target");
    };
    assert_eq!(function_target, &HirCallTarget::Function(new_function));
    let crate::hir::HirExprKindFor::Call(_, _, Some(extern_target)) = &extern_call.kind else {
        panic!("expected extern call target");
    };
    assert_eq!(extern_target, &HirCallTarget::Extern(new_extern));
}

#[test]
fn remap_pattern_owned_type_ids_remaps_new_pattern_sidecars() {
    let old_id = DefId::new(CrateId(0), LocalDefId(1));
    let new_id = DefId::new(CrateId(0), LocalDefId(2));
    let mut pattern = HirPattern::Enum(
        "Option".to_string(),
        "Some".to_string(),
        Some(HirVariantLocation {
            owner: old_id,
            variant_id: VariantId(0),
            name: "Some".to_string(),
        }),
        vec![HirPattern::Struct(
            "Box".to_string(),
            Some(old_id),
            vec![Type::Struct {
                id: old_id,
                args: Vec::new(),
            }],
            vec![HirStructPatternField {
                name: "value".to_string(),
                field: Some(HirFieldLocation {
                    owner: old_id,
                    field_id: FieldId(0),
                    name: "value".to_string(),
                }),
                pattern: HirPattern::Wildcard,
            }],
        )],
    );

    super::remap_pattern_owned_type_ids(&mut pattern, old_id, new_id);

    let HirPattern::Enum(_, _, Some(location), payloads) = pattern else {
        panic!("expected enum pattern");
    };
    assert_eq!(location.owner, new_id);
    let HirPattern::Struct(_, Some(struct_id), args, fields) = &payloads[0] else {
        panic!("expected struct payload pattern");
    };
    assert_eq!(*struct_id, new_id);
    assert_eq!(
        args[0],
        Type::Struct {
            id: old_id,
            args: Vec::new(),
        }
    );
    assert_eq!(fields[0].field.as_ref().unwrap().owner, new_id);
}

#[test]
fn remap_expr_location_product_ids_remaps_match_pattern_sidecars() {
    let old_id = DefId::new(CrateId(0), LocalDefId(1));
    let new_id = DefId::new(CrateId(7), LocalDefId(2));
    let mut remap = BTreeMap::new();
    remap.insert(
        ProductDefId::from(old_id),
        BTreeSet::from([ProductDefId::from(new_id)]),
    );
    let mut expr = HirExpr {
        kind: crate::hir::HirExprKindFor::Match {
            scrutinee: Box::new(HirExpr {
                kind: crate::hir::HirExprKindFor::Var("value".to_string()),
                ty: Type::I64,
                span: Span::test(),
            }),
            arms: vec![crate::hir::HirMatchArm {
                pattern: HirPattern::Enum(
                    "Option".to_string(),
                    "Some".to_string(),
                    Some(HirVariantLocation {
                        owner: old_id,
                        variant_id: VariantId(0),
                        name: "Some".to_string(),
                    }),
                    vec![HirPattern::Struct(
                        "Box".to_string(),
                        Some(old_id),
                        Vec::new(),
                        vec![HirStructPatternField {
                            name: "value".to_string(),
                            field: Some(HirFieldLocation {
                                owner: old_id,
                                field_id: FieldId(0),
                                name: "value".to_string(),
                            }),
                            pattern: HirPattern::Wildcard,
                        }],
                    )],
                ),
                guard: None,
                body: HirBlock {
                    stmts: Vec::new(),
                    ty: Type::Unit,
                },
            }],
        },
        ty: Type::Unit,
        span: Span::test(),
    };

    super::remap_expr_location_product_ids(&mut expr, &remap);

    let crate::hir::HirExprKindFor::Match { arms, .. } = &expr.kind else {
        panic!("expected match expression");
    };
    let HirPattern::Enum(_, _, Some(location), payloads) = &arms[0].pattern else {
        panic!("expected enum pattern");
    };
    assert_eq!(location.owner, new_id);
    let HirPattern::Struct(_, Some(struct_id), _, fields) = &payloads[0] else {
        panic!("expected struct pattern");
    };
    assert_eq!(*struct_id, new_id);
    assert_eq!(fields[0].field.as_ref().unwrap().owner, new_id);
}

fn rebuild_program(
    program: &mut crate::hir::AcceptedHirProgram,
    mutate: impl FnOnce(
        &mut HashMap<DefId, AcceptedHirFunction>,
        &mut HashMap<DefId, HirStruct>,
        &mut HashMap<DefId, HirEnum>,
        &mut HashMap<DefId, HirTraitFor<AcceptedHir>>,
        &mut HashMap<DefId, HirImplFor<AcceptedHir>>,
        &mut HashMap<DefId, HirExtern>,
        &mut crate::hir::HirNameTables,
        &mut HashMap<(DefId, DefId), DefId>,
    ),
) {
    let source = program.program();
    let mut canonical_names_by_id = source.indexes.functions_by_id.clone();
    canonical_names_by_id.extend(source.indexes.structs_by_id.clone());
    canonical_names_by_id.extend(source.indexes.enums_by_id.clone());
    canonical_names_by_id.extend(source.indexes.traits_by_id.clone());
    for function in source.functions.values() {
        canonical_names_by_id
            .entry(function.id)
            .or_insert_with(|| function.name.clone());
    }
    let mut effective_trait_methods = source.indexes.effective_trait_methods.clone();
    let mut functions = source.functions.clone();
    let mut structs = source.structs.clone();
    let mut enums = source.enums.clone();
    let mut traits = source.traits.clone();
    let mut impls = source.impls.clone();
    let mut externs = source.externs.clone();
    let mut names = source.names.clone();

    mutate(
        &mut functions,
        &mut structs,
        &mut enums,
        &mut traits,
        &mut impls,
        &mut externs,
        &mut names,
        &mut effective_trait_methods,
    );
    let mut rebuilt =
        HirProgramFor::<AcceptedHir>::from_accepted_id_parts_with_names_and_canonical_names(
            functions,
            structs,
            enums,
            traits,
            impls,
            externs,
            names,
            &canonical_names_by_id,
        );
    rebuilt
        .indexes
        .effective_trait_methods
        .extend(effective_trait_methods);
    rebuilt
        .indexes
        .effective_trait_methods
        .retain(|(impl_id, _), _| rebuilt.impls.contains_key(impl_id));
    *program = crate::hir::AcceptedHirProgram::revalidate_for_test(rebuilt)
        .expect("rebuilt product fixture must satisfy accepted HIR invariants");
}

fn mutate_program(
    program: &mut crate::hir::AcceptedHirProgram,
    mutate: impl FnOnce(&mut HirProgramFor<AcceptedHir>),
) {
    let mut mutated = program.program().clone();
    mutate(&mut mutated);
    *program = crate::hir::AcceptedHirProgram::revalidate_for_test(mutated)
        .expect("mutated product fixture must satisfy accepted HIR invariants");
}

fn int_body() -> HirBlock {
    HirBlock {
        stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
            kind: crate::hir::HirExprKindFor::IntLiteral(1),
            ty: Type::I64,
            span: Span::test(),
        })],
        ty: Type::I64,
    }
}

fn empty_body(ty: Type) -> HirBlock {
    HirBlock {
        stmts: Vec::new(),
        ty,
    }
}

fn test_function(id: DefId, name: &str, generic_params: Vec<GenericParamDecl>) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params,
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: int_body(),
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn accept_function(function: HirFunction) -> AcceptedHirFunction {
    let id = function.id;
    let name = function.name.clone();
    let accepted = crate::hir::AcceptedHirProgram::try_from(
        HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::from([(id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([(name.clone(), id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([(id, name)]),
        ),
    )
    .expect("fixture function is accepted");
    accepted.function_by_id(id).unwrap().1.clone()
}

fn local_def_ids_after(next_raw: u32) -> IdGen<LocalDefId> {
    let mut ids = IdGen::new();
    for _ in 0..next_raw {
        ids.fresh();
    }
    ids
}

fn resolved_hir_for_products() -> ResolvedHirProgram {
    let plain_id = DefId::new(CrateId(0), LocalDefId(0));
    let generic_id = DefId::new(CrateId(0), LocalDefId(1));
    let struct_id = DefId::new(CrateId(0), LocalDefId(2));
    let enum_id = DefId::new(CrateId(0), LocalDefId(3));
    let trait_id = DefId::new(CrateId(0), LocalDefId(4));
    let impl_id = DefId::new(CrateId(0), LocalDefId(5));
    let extern_id = DefId::new(CrateId(0), LocalDefId(6));

    let mut functions = HashMap::new();
    functions.insert(plain_id, test_function(plain_id, "plain", Vec::new()));
    functions.insert(
        generic_id,
        test_function(generic_id, "identity", generic_params(generic_id, &["T"])),
    );

    let mut structs = HashMap::new();
    structs.insert(
        struct_id,
        HirStruct {
            id: struct_id,
            name: "Box".to_string(),
            generic_params: generic_params(struct_id, &["T"]),
            fields: Vec::new(),
        },
    );

    let mut enums = HashMap::new();
    enums.insert(
        enum_id,
        HirEnum {
            id: enum_id,
            name: "Maybe".to_string(),
            generic_params: generic_params(enum_id, &["T"]),
            variants: Vec::new(),
        },
    );

    let default_method_id = DefId::new(CrateId(0), LocalDefId(7));
    let default_method = test_function(default_method_id, "show", Vec::new());
    let effective_method_id = DefId::new(CrateId(0), LocalDefId(8));
    let mut effective_method = default_method.clone();
    effective_method.id = effective_method_id;
    let mut trait_methods = HashMap::new();
    trait_methods.insert("show".to_string(), default_method.clone());
    let mut traits = HashMap::new();
    traits.insert(
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: trait_methods,
            signatures: HashMap::new(),
        },
    );

    let impls = HashMap::from([(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: crate::hir::HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: generic_params(impl_id, &["T"]),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(crate::types::GenericParamId {
                    owner: impl_id,
                    index: 0,
                })],
            }),
            trait_name: Some("Show".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("show".to_string(), effective_method)]),
        },
    )]);

    let externs = HashMap::from([(
        extern_id,
        HirExtern {
            id: extern_id,
            name: "puts".to_string(),
            params: vec![Type::Pointer(Box::new(Type::U8))],
            ret: Type::I32,
            variadic: false,
            is_unsafe: false,
        },
    )]);

    let mut program = HirProgram::from_id_parts_with_names_and_canonical_names(
        functions,
        structs,
        enums,
        traits,
        impls,
        externs,
        HirNameTables {
            functions_by_name: HashMap::from([
                ("plain".to_string(), plain_id),
                ("identity".to_string(), generic_id),
            ]),
            structs_by_name: HashMap::from([("Box".to_string(), struct_id)]),
            enums_by_name: HashMap::from([("Maybe".to_string(), enum_id)]),
            traits_by_name: HashMap::from([("Show".to_string(), trait_id)]),
            externs_by_name: HashMap::from([("puts".to_string(), extern_id)]),
            type_aliases_by_name: HashMap::new(),
        },
        HirLanguageItems::default(),
        &HashMap::from([
            (plain_id, "plain".to_string()),
            (generic_id, "identity".to_string()),
            (struct_id, "Box".to_string()),
            (enum_id, "Maybe".to_string()),
            (trait_id, "Show".to_string()),
        ]),
    );
    program
        .indexes
        .effective_trait_methods
        .insert((impl_id, default_method_id), effective_method_id);

    ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        (0..9)
            .map(|local| DefId::new(CrateId(0), LocalDefId(local)))
            .collect(),
        CrateId(0),
        local_def_ids_after(9),
    )
}

fn resolved_hir_with_non_generic_impl_generic_method() -> ResolvedHirProgram {
    let struct_id = DefId::new(CrateId(0), LocalDefId(41));
    let impl_id = DefId::new(CrateId(0), LocalDefId(42));
    let method_id = DefId::new(CrateId(0), LocalDefId(43));
    let method_generic = crate::types::GenericParamId {
        owner: method_id,
        index: 0,
    };

    let method = HirFunction {
        id: method_id,
        name: "id".to_string(),
        generic_params: vec![GenericParamDecl::type_param(method_generic, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(method_generic),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(method_generic),
        body: HirBlock {
            stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
                kind: crate::hir::HirExprKindFor::Var("value".to_string()),
                ty: Type::Generic(method_generic),
                span: Span::test(),
            })],
            ty: Type::Generic(method_generic),
        },
        is_curried: false,
        is_method: true,
        self_receiver: None,
        is_unsafe: false,
    };

    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::from([(
            struct_id,
            HirStruct {
                id: struct_id,
                name: "Foo".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: crate::hir::HirImplOwner::Named("Foo".to_string()),
                type_name: "Foo".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("id".to_string(), method)]),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([("Foo".to_string(), struct_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([(struct_id, "Foo".to_string())]),
    );
    ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([struct_id, impl_id, method_id]),
        CrateId(0),
        local_def_ids_after(44),
    )
}

#[test]
fn compiler_products_keep_static_impl_methods_out_of_top_level_function_payloads() {
    let struct_id = DefId::new(CrateId(0), LocalDefId(100));
    let impl_id = DefId::new(CrateId(0), LocalDefId(101));
    let method_id = DefId::new(CrateId(0), LocalDefId(102));
    let mut method = test_function(method_id, "new", Vec::new());
    method.is_method = true;
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::from([(
            struct_id,
            HirStruct {
                id: struct_id,
                name: "Box".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("new".to_string(), method)]),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([("Box".to_string(), struct_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([(struct_id, "Box".to_string())]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([struct_id, impl_id, method_id]),
        CrateId(0),
        local_def_ids_after(103),
    );
    let method_product_id = ProductDefId::from(method_id);
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData {
            object_path: None,
            records: BTreeMap::from([(
                method_product_id,
                ProductLinkRecord {
                    backend_symbol: "box_new".to_string(),
                },
            )]),
        },
    )
    .expect("test HIR has valid product language items");

    assert!(!products
        .interface
        .functions
        .contains_key(&method_product_id));
    assert!(!products.bodies.functions.contains_key(&method_product_id));
    assert_eq!(
        products.interface.impls[&ProductDefId::from(impl_id)].methods["new"].id,
        method_id
    );
    assert_eq!(
        products.link.records[&method_product_id].backend_symbol,
        "box_new"
    );
}

#[test]
fn product_bodies_only_include_downstream_specialized_functions() {
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved_hir_for_products(),
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let plain_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let generic_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(1),
    };

    let plain_decl = products
        .interface
        .functions
        .get(&plain_id)
        .expect("plain function interface row");
    assert_eq!(plain_decl.id, DefId::new(CrateId(0), LocalDefId(0)));
    assert_eq!(plain_decl.name, "plain");
    assert_eq!(plain_decl.params, vec![Type::I64]);
    assert_eq!(plain_decl.ret_type, Type::I64);
    assert!(!products.bodies.functions.contains_key(&plain_id));

    assert!(products.interface.functions.contains_key(&generic_id));
    assert!(products.bodies.functions.contains_key(&generic_id));
}

fn resolved_hir_with_traits_in_order(names: &[&str]) -> ResolvedHirProgram {
    let mut traits = HashMap::new();

    for (index, name) in names.iter().enumerate() {
        let method_id = match *name {
            "Show" => DefId::new(CrateId(0), LocalDefId(100)),
            "Debug" => DefId::new(CrateId(0), LocalDefId(101)),
            _ => DefId::new(CrateId(0), LocalDefId(200 + index as u32)),
        };
        let mut methods = HashMap::new();
        methods.insert(
            "show".to_string(),
            test_function(method_id, "show", Vec::new()),
        );
        traits.insert(
            DefId::new(CrateId(0), LocalDefId(index as u32)),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: DefId::new(CrateId(0), LocalDefId(index as u32)),
                name: (*name).to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods,
                signatures: HashMap::new(),
            },
        );
    }

    ResolvedHirProgram::new(
        HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            traits,
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                traits_by_name: names
                    .iter()
                    .enumerate()
                    .map(|(index, name)| {
                        (
                            (*name).to_string(),
                            DefId::new(CrateId(0), LocalDefId(index as u32)),
                        )
                    })
                    .collect(),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    (
                        DefId::new(CrateId(0), LocalDefId(index as u32)),
                        (*name).to_string(),
                    )
                })
                .collect(),
        ),
        ResolverTables::default(),
        names
            .iter()
            .enumerate()
            .map(|(index, _)| DefId::new(CrateId(0), LocalDefId(index as u32)))
            .chain(names.iter().filter_map(|name| match *name {
                "Show" => Some(DefId::new(CrateId(0), LocalDefId(100))),
                "Debug" => Some(DefId::new(CrateId(0), LocalDefId(101))),
                _ => None,
            }))
            .collect(),
        CrateId(0),
        IdGen::new(),
    )
}

fn refresh_resolved_type_ids(resolved: &mut ResolvedHirProgram) {
    resolved.type_context = crate::type_context::TypeContext::new();
    resolved.type_ids =
        crate::hir::collect_hir_type_ids(&resolved.program, &mut resolved.type_context);
}

#[test]
fn compiler_products_key_interface_by_product_def_id() {
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

    let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));
    let extern_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(6)));

    assert!(products.interface.functions.contains_key(&plain_id));
    assert!(products.interface.externs.contains_key(&extern_id));
    assert_eq!(
        products.identity_table.export_names.get("plain"),
        Some(&plain_id)
    );
    assert_eq!(
        products.identity_table.display_names.get(&plain_id),
        Some(&"plain".to_string())
    );
}

#[test]
fn compiler_products_persist_resolver_aliases_by_product_id() {
    let alias_id = DefId::new(CrateId(0), LocalDefId(4));
    let mut hir = resolved_hir_for_products();
    hir.resolver.insert_import_alias_with_name(
        "short".to_string(),
        "demo::plain".to_string(),
        alias_id,
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    assert_eq!(
        products.identity_table.import_alias_names.get("short"),
        Some(&ProductDefId::from(alias_id))
    );
}

#[test]
fn compiler_products_do_not_export_dependency_aliases_as_current_roots() {
    let dependency_id = DefId::new(CrateId(1), LocalDefId(42));
    let mut hir = resolved_hir_for_products();
    hir.resolver.insert_export_alias_with_name(
        "std::alloc::Global".to_string(),
        "std::alloc::Global".to_string(),
        dependency_id,
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    assert!(!products
        .identity_table
        .export_names
        .contains_key("std::alloc::Global"));
}

#[test]
fn compiler_products_record_prelude_export_ids() {
    let hir = resolved_hir_for_products();
    let mut products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));

    products.record_prelude_export_ids([(
        "plain".to_string(),
        crate::crate_artifact::ArtifactExport {
            source: "plain".to_string(),
            id: DefId::new(CrateId(0), LocalDefId(0)),
        },
    )]);

    assert_eq!(
        products.identity_table.prelude_export_names.get("plain"),
        Some(&plain_id)
    );
}

#[test]
fn compiler_products_record_prelude_export_ids_without_string_payload() {
    let hir = resolved_hir_for_products();
    let mut products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let plain_def_id = DefId::new(CrateId(0), LocalDefId(0));
    let plain_product_id = ProductDefId::from(plain_def_id);

    products.record_prelude_export_ids([(
        "plain".to_string(),
        crate::crate_artifact::ArtifactExport {
            source: "plain".to_string(),
            id: plain_def_id,
        },
    )]);

    assert_eq!(
        products.identity_table.prelude_export_names.get("plain"),
        Some(&plain_product_id)
    );
}

#[test]
fn compiler_products_roundtrip_preserves_prelude_export_ids() {
    let hir = resolved_hir_for_products();
    let mut products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));
    products.record_prelude_export_ids([(
        "plain".to_string(),
        crate::crate_artifact::ArtifactExport {
            source: "plain".to_string(),
            id: DefId::new(CrateId(0), LocalDefId(0)),
        },
    )]);

    let bytes = products.to_artifact_bytes().unwrap();
    let decoded = CompilerProducts::from_artifact_bytes(&bytes).unwrap();

    assert_eq!(
        decoded.identity_table.prelude_export_names.get("plain"),
        Some(&plain_id)
    );
}

#[test]
fn compiler_products_roundtrip_and_validate_constant_arithmetic_summaries() {
    use super::{ArithmeticSummary, ArithmeticType};
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
fn compiler_products_record_prelude_export_ids_prefer_explicit_export_id() {
    let hir = resolved_hir_for_products();
    let mut products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let plain_def_id = DefId::new(CrateId(0), LocalDefId(0));
    let plain_id = ProductDefId::from(plain_def_id);

    products.record_prelude_export_ids([(
        "prelude_alias".to_string(),
        crate::crate_artifact::ArtifactExport {
            source: "plain".to_string(),
            id: plain_def_id,
        },
    )]);

    assert_eq!(
        products
            .identity_table
            .prelude_export_names
            .get("prelude_alias"),
        Some(&plain_id)
    );
}

#[test]
fn compiler_products_record_prelude_export_ids_fall_back_on_mismatched_source_id() {
    let hir = resolved_hir_for_products();
    let mut products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let plain_def_id = DefId::new(CrateId(0), LocalDefId(0));
    let identity_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(1)));

    products.record_prelude_export_ids([(
        "prelude_alias".to_string(),
        crate::crate_artifact::ArtifactExport {
            source: "identity".to_string(),
            id: plain_def_id,
        },
    )]);

    assert_eq!(
        products
            .identity_table
            .prelude_export_names
            .get("prelude_alias"),
        Some(&identity_id)
    );
}

#[test]
fn compiler_products_record_prelude_export_ids_skip_non_item_explicit_ids() {
    let hir = resolved_hir_for_products();
    let mut products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let impl_def_id = DefId::new(CrateId(0), LocalDefId(5));
    let impl_id = ProductDefId::from(impl_def_id);

    assert!(products.interface.impls.contains_key(&impl_id));
    assert_eq!(
        products.identity_table.display_names.get(&impl_id),
        Some(&"Box as Show".to_string())
    );

    products.record_prelude_export_ids([(
        "impl_alias".to_string(),
        crate::crate_artifact::ArtifactExport {
            source: "Box as Show".to_string(),
            id: impl_def_id,
        },
    )]);

    assert!(products.identity_table.prelude_export_names.is_empty());
    assert!(!products
        .identity_table
        .prelude_export_names
        .contains_key("impl_alias"));
}

#[test]
fn product_emission_reads_id_owned_hir_definitions() {
    let function_id = DefId::new(CrateId(0), LocalDefId(0));
    let struct_id = DefId::new(CrateId(0), LocalDefId(1));
    let function = test_function(function_id, "answer", Vec::new());
    let structure = HirStruct {
        id: struct_id,
        name: "Box".to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    };

    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::from([(function_id, function)]),
        HashMap::from([(struct_id, structure)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("answer".to_string(), function_id)]),
            structs_by_name: HashMap::from([("Box".to_string(), struct_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([
            (function_id, "answer".to_string()),
            (struct_id, "Box".to_string()),
        ]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([function_id, struct_id]),
        CrateId(0),
        IdGen::new(),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("test".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    assert!(products
        .interface
        .functions
        .contains_key(&ProductDefId::from(function_id)));
    assert!(products
        .interface
        .structs
        .contains_key(&ProductDefId::from(struct_id)));
}

#[test]
fn compiler_products_emit_named_interface_when_derived_indexes_are_stale() {
    let mut hir = resolved_hir_for_products();
    let plain_id = DefId::new(CrateId(0), LocalDefId(0));
    let struct_id = DefId::new(CrateId(0), LocalDefId(2));
    let enum_id = DefId::new(CrateId(0), LocalDefId(3));
    let trait_id = DefId::new(CrateId(0), LocalDefId(4));
    mutate_program(&mut hir.program, |program| {
        program.indexes.functions_by_id.remove(&plain_id);
        program.indexes.structs_by_id.remove(&struct_id);
        program.indexes.enums_by_id.remove(&enum_id);
        program.indexes.traits_by_id.remove(&trait_id);
    });

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let plain_id = ProductDefId::from(plain_id);
    let struct_id = ProductDefId::from(struct_id);
    let enum_id = ProductDefId::from(enum_id);
    let trait_id = ProductDefId::from(trait_id);
    assert_eq!(products.interface.functions[&plain_id].name, "plain");
    assert_eq!(products.interface.structs[&struct_id].name, "Box");
    assert_eq!(products.interface.enums[&enum_id].name, "Maybe");
    assert_eq!(products.interface.traits[&trait_id].name, "demo::Show");
    assert_eq!(products.identity_table.export_names["plain"], plain_id);
    assert_eq!(products.identity_table.export_names["Box"], struct_id);
    assert_eq!(products.identity_table.export_names["Maybe"], enum_id);
    assert_eq!(products.identity_table.export_names["Show"], trait_id);
}

#[test]
#[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
fn compiler_products_reject_function_generic_param_decls_with_invalid_current_crate_ids() {
    let original_id = DefId::new(CrateId(u32::MAX), LocalDefId(17));
    let original_generic = crate::types::GenericParamId {
        owner: original_id,
        index: 0,
    };
    let mut function = test_function(original_id, "identity", generic_params(original_id, &["T"]));
    function.generic_params[0].id = original_generic;
    function.params[0].ty = Type::Generic(original_generic);
    function.ret_type = Type::Generic(original_generic);
    function.body = empty_body(Type::Generic(original_generic));

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &ResolvedHirProgram::new(
            HirProgram::from_id_parts_with_names_and_canonical_names(
                HashMap::from([(original_id, function)]),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HirNameTables {
                    functions_by_name: HashMap::from([("identity".to_string(), original_id)]),
                    ..HirNameTables::default()
                },
                HirLanguageItems::default(),
                &HashMap::from([(original_id, "identity".to_string())]),
            ),
            ResolverTables::default(),
            BTreeSet::from([original_id]),
            CrateId(0),
            IdGen::new(),
        ),
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let (product_id, function) = products
        .interface
        .functions
        .iter()
        .next()
        .expect("product should contain remapped generic function metadata");
    let expected_owner = DefId::new(
        CrateId(product_id.crate_id.0),
        LocalDefId(product_id.local_id.0),
    );

    assert_ne!(*product_id, ProductDefId::from(original_id));
    assert_eq!(function.generic_params[0].id.owner, expected_owner);
    assert_eq!(
        function.params[0],
        Type::Generic(crate::types::GenericParamId {
            owner: expected_owner,
            index: 0,
        })
    );
}

#[test]
#[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
fn compiler_products_reject_function_body_generic_owners_with_invalid_current_crate_ids() {
    let function_id = DefId::new(CrateId(0), LocalDefId(17));
    let invalid_owner = DefId::new(CrateId(u32::MAX), LocalDefId(18));
    let invalid_generic = crate::types::GenericParamId {
        owner: invalid_owner,
        index: 0,
    };
    let mut function = test_function(function_id, "identity", generic_params(function_id, &["T"]));
    function.generic_params[0].id = invalid_generic;
    function.params[0].ty = Type::Generic(invalid_generic);
    function.ret_type = Type::Generic(invalid_generic);
    function.body = empty_body(Type::Generic(invalid_generic));

    let _ = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &ResolvedHirProgram::new(
            HirProgram::from_id_parts_with_names_and_canonical_names(
                HashMap::from([(function_id, function)]),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HirNameTables {
                    functions_by_name: HashMap::from([("identity".to_string(), function_id)]),
                    ..HirNameTables::default()
                },
                HirLanguageItems::default(),
                &HashMap::from([(function_id, "identity".to_string())]),
            ),
            ResolverTables::default(),
            BTreeSet::from([function_id]),
            CrateId(0),
            IdGen::new(),
        ),
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
}

#[test]
#[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
fn compiler_products_reject_trait_default_body_generic_owners_with_invalid_current_crate_ids() {
    let trait_id = DefId::new(CrateId(u32::MAX), LocalDefId(17));
    let method_id = DefId::new(CrateId(u32::MAX), LocalDefId(18));
    let self_generic = crate::types::GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let method = HirFunction {
        id: method_id,
        name: "identity".to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(self_generic),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(self_generic),
        body: HirBlock {
            stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
                kind: crate::hir::HirExprKindFor::Var("self".to_string()),
                ty: Type::Generic(self_generic),
                span: Span::test(),
            })],
            ty: Type::Generic(self_generic),
        },
        is_curried: false,
        is_method: true,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Identity".to_string(),
                generic_params: vec![],
                associated_types: vec![],
                methods: HashMap::from([("identity".to_string(), method)]),
                signatures: HashMap::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([("Identity".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([(trait_id, "Identity".to_string())]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([trait_id, method_id]),
        CrateId(0),
        IdGen::new(),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let trait_product_id = products
        .interface
        .traits
        .keys()
        .next()
        .copied()
        .expect("product should contain trait interface");
    let expected_owner = DefId::new(
        CrateId(trait_product_id.crate_id.0),
        LocalDefId(trait_product_id.local_id.0),
    );
    let method = products
        .bodies
        .trait_default_methods
        .values()
        .next()
        .expect("product should contain trait default body");
    let crate::hir::HirStmtFor::Expr(expr) = &method.body.stmts[0] else {
        panic!("expected expression statement in default method body");
    };

    assert_eq!(
        expr.ty,
        Type::Generic(crate::types::GenericParamId {
            owner: expected_owner,
            index: 0,
        })
    );
}

#[test]
#[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
fn compiler_products_reject_generic_impl_method_body_generic_owners_with_invalid_current_crate_ids()
{
    let impl_id = DefId::new(CrateId(u32::MAX), LocalDefId(19));
    let method_id = DefId::new(CrateId(u32::MAX), LocalDefId(20));
    let impl_generic = crate::types::GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_generic = crate::types::GenericParamId {
        owner: method_id,
        index: 0,
    };
    let method = HirFunction {
        id: method_id,
        name: "get".to_string(),
        generic_params: vec![GenericParamDecl::type_param(method_generic, "U")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(impl_generic),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(method_generic),
        body: HirBlock {
            stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
                kind: crate::hir::HirExprKindFor::Var("self".to_string()),
                ty: Type::Generic(impl_generic),
                span: Span::test(),
            })],
            ty: Type::Generic(impl_generic),
        },
        is_curried: false,
        is_method: true,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: generic_params(impl_id, &["T"]),
                receiver_pattern: vec![Type::Generic(impl_generic)].into(),
                trait_name: None,
                trait_id: None,
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("get".to_string(), method)]),
            },
        )]),
        HashMap::new(),
        HirNameTables::default(),
        HirLanguageItems::default(),
        &HashMap::new(),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([impl_id, method_id]),
        CrateId(0),
        IdGen::new(),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let impl_product_id = products
        .interface
        .impls
        .keys()
        .next()
        .copied()
        .expect("product should contain impl interface");
    let expected_owner = DefId::new(
        CrateId(impl_product_id.crate_id.0),
        LocalDefId(impl_product_id.local_id.0),
    );
    let imp = products
        .bodies
        .generic_impls
        .values()
        .next()
        .expect("product should contain generic impl body");
    let method = &imp.methods["get"];
    assert_ne!(method.id, method_id);
    assert_ne!(method.id.crate_id, CrateId(u32::MAX));
    assert_eq!(
        method.generic_params[0].id.owner, method.id,
        "method-owned generic params should be rehomed to the product method ID"
    );
    assert_eq!(
        method.ret_type,
        Type::Generic(crate::types::GenericParamId {
            owner: method.id,
            index: 0,
        })
    );
    let crate::hir::HirStmtFor::Expr(expr) = &method.body.stmts[0] else {
        panic!("expected expression statement in impl method body");
    };

    assert_eq!(
        expr.ty,
        Type::Generic(crate::types::GenericParamId {
            owner: expected_owner,
            index: 0,
        })
    );
}

#[test]
#[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
fn compiler_products_reject_body_field_and_variant_location_owners_with_invalid_current_crate_ids()
{
    let struct_id = DefId::new(CrateId(u32::MAX), LocalDefId(21));
    let enum_id = DefId::new(CrateId(u32::MAX), LocalDefId(22));
    let impl_id = DefId::new(CrateId(u32::MAX), LocalDefId(23));
    let method_id = DefId::new(CrateId(u32::MAX), LocalDefId(24));
    let method = HirFunction {
        id: method_id,
        name: "locations".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        generic_bounds: HashMap::new().into(),
        params: vec![],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: vec![
                crate::hir::HirStmtFor::Expr(HirExpr {
                    kind: crate::hir::HirExprKindFor::FieldAccess(
                        Box::new(HirExpr {
                            kind: crate::hir::HirExprKindFor::Var("boxed".to_string()),
                            ty: Type::Struct {
                                id: struct_id,
                                args: vec![],
                            },
                            span: Span::test(),
                        }),
                        "value".to_string(),
                        Some(crate::hir::HirFieldLocation {
                            owner: struct_id,
                            field_id: FieldId(0),
                            name: "value".to_string(),
                        }),
                    ),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                crate::hir::HirStmtFor::Expr(HirExpr {
                    kind: crate::hir::HirExprKindFor::EnumVariant(
                        "Maybe".to_string(),
                        "Some".to_string(),
                        vec![],
                        Some(crate::hir::HirVariantLocation {
                            owner: enum_id,
                            variant_id: VariantId(0),
                            name: "Some".to_string(),
                        }),
                    ),
                    ty: Type::Enum {
                        id: enum_id,
                        args: vec![],
                    },
                    span: Span::test(),
                }),
            ],
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::from([(
            struct_id,
            HirStruct {
                id: struct_id,
                name: "Box".to_string(),
                generic_params: vec![],
                fields: Vec::new(),
            },
        )]),
        HashMap::from([(
            enum_id,
            HirEnum {
                id: enum_id,
                name: "Maybe".to_string(),
                generic_params: vec![],
                variants: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: generic_params(impl_id, &["T"]),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: vec![Type::Generic(crate::types::GenericParamId {
                        owner: impl_id,
                        index: 0,
                    })],
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("locations".to_string(), method)]),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([("Box".to_string(), struct_id)]),
            enums_by_name: HashMap::from([("Maybe".to_string(), enum_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([
            (struct_id, "Box".to_string()),
            (enum_id, "Maybe".to_string()),
        ]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([struct_id, enum_id, impl_id, method_id]),
        CrateId(0),
        IdGen::new(),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let expected_struct_owner = products
        .interface
        .structs
        .keys()
        .next()
        .copied()
        .map(|id| DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0)))
        .expect("product should contain struct interface");
    let expected_enum_owner = products
        .interface
        .enums
        .keys()
        .next()
        .copied()
        .map(|id| DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0)))
        .expect("product should contain enum interface");
    let imp = products
        .bodies
        .generic_impls
        .values()
        .next()
        .expect("product should contain generic impl body");
    let method = &imp.methods["locations"];
    let crate::hir::HirStmtFor::Expr(field_expr) = &method.body.stmts[0] else {
        panic!("expected field access expression statement");
    };
    let crate::hir::HirExprKindFor::FieldAccess(_, _, Some(field_location)) = &field_expr.kind
    else {
        panic!("expected field access location");
    };
    let crate::hir::HirStmtFor::Expr(variant_expr) = &method.body.stmts[1] else {
        panic!("expected enum variant expression statement");
    };
    let crate::hir::HirExprKindFor::EnumVariant(_, _, _, Some(variant_location)) =
        &variant_expr.kind
    else {
        panic!("expected enum variant location");
    };

    assert_eq!(field_location.owner, expected_struct_owner);
    assert_eq!(variant_location.owner, expected_enum_owner);
}

#[test]
#[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
fn compiler_products_reject_call_targets_with_invalid_current_crate_ids() {
    let function_id = DefId::new(CrateId(0), LocalDefId(0));
    let invalid_target = DefId::new(CrateId(u32::MAX), LocalDefId(99));
    let mut function = test_function(function_id, "caller", Vec::new());
    function.body = HirBlock {
        stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
            kind: crate::hir::HirExprKindFor::Call(
                Box::new(HirExpr {
                    kind: crate::hir::HirExprKindFor::Var("target".to_string()),
                    ty: Type::function(Vec::new(), Type::I64),
                    span: Span::test(),
                }),
                Vec::new(),
                Some(HirCallTarget::Function(invalid_target)),
            ),
            ty: Type::I64,
            span: Span::test(),
        })],
        ty: Type::I64,
    };
    let hir = ResolvedHirProgram::new(
        HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::from([(function_id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("caller".to_string(), function_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([(function_id, "caller".to_string())]),
        ),
        ResolverTables::default(),
        BTreeSet::from([function_id]),
        CrateId(0),
        local_def_ids_after(1),
    );

    CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
}

#[test]
fn compiler_products_emit_alias_duplicate_function_once_by_def_id() {
    let mut hir = resolved_hir_for_products();
    let generic_id = DefId::new(CrateId(0), LocalDefId(1));
    rebuild_program(&mut hir.program, |_, _, _, _, _, _, names, _| {
        names
            .functions_by_name
            .insert("alias_identity".to_string(), generic_id);
    });
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let generic_product_id = ProductDefId::from(generic_id);
    assert_eq!(
        products
            .interface
            .functions
            .values()
            .filter(|function| function.id == generic_id)
            .count(),
        1
    );
    assert_eq!(
        products.identity_table.export_names.get("identity"),
        Some(&generic_product_id)
    );
    assert!(!products
        .identity_table
        .export_names
        .contains_key("alias_identity"));
}

#[test]
fn compiler_products_select_generic_bodies_by_product_def_id() {
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

    let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));
    let generic_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(1)));
    let impl_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(5)));
    let default_method_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(7)));

    assert!(!products.bodies.functions.contains_key(&plain_id));
    assert!(products.bodies.functions.contains_key(&generic_id));
    assert!(products.bodies.generic_impls.contains_key(&impl_id));
    assert!(products
        .bodies
        .trait_default_methods
        .contains_key(&default_method_id));
}

#[test]
fn compiler_products_preserve_non_generic_impl_with_generic_method_body() {
    let hir = resolved_hir_with_non_generic_impl_generic_method();
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let impl_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(42)));
    assert!(products.bodies.generic_impls.contains_key(&impl_id));
}

#[test]
fn compiler_products_emit_only_current_crate_hir() {
    let mut hir = resolved_hir_for_products();
    let foreign_function_id = DefId::new(CrateId(0), LocalDefId(50));
    let foreign_impl_id = DefId::new(CrateId(0), LocalDefId(51));
    hir.current_def_ids = (0..8)
        .map(|local| DefId::new(CrateId(0), LocalDefId(local)))
        .collect();
    rebuild_program(&mut hir.program, |functions, _, _, _, impls, _, _, _| {
        functions.insert(
            foreign_function_id,
            accept_function(test_function(
                foreign_function_id,
                "std::identity",
                generic_params(foreign_function_id, &["T"]),
            )),
        );
        impls.insert(
            foreign_impl_id,
            HirImplFor::<AcceptedHir> {
                id: foreign_impl_id,
                owner: crate::hir::HirImplOwner::Named("std::Box".to_string()),
                type_name: "std::Box".to_string(),
                type_generics: generic_params(foreign_impl_id, &["T"]),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: DefId::new(CrateId(0), LocalDefId(2)),
                    args: vec![Type::Generic(crate::types::GenericParamId {
                        owner: foreign_impl_id,
                        index: 0,
                    })],
                }),
                trait_name: Some("std::Show".to_string()),
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            },
        );
    });

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        vec![ProductDependencyIdentity::artifact_object(
            "std".to_string(),
            PathBuf::from("build/std.rkca"),
        )],
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let foreign_function_id = ProductDefId::from(foreign_function_id);
    let foreign_impl_id = ProductDefId::from(foreign_impl_id);

    assert!(!products
        .interface
        .functions
        .contains_key(&foreign_function_id));
    assert!(!products.bodies.functions.contains_key(&foreign_function_id));
    assert!(!products.interface.impls.contains_key(&foreign_impl_id));
    assert!(!products.bodies.generic_impls.contains_key(&foreign_impl_id));
    assert_eq!(products.interface.impls.len(), 1);
    assert_eq!(products.bodies.generic_impls.len(), 1);
}

#[test]
fn compiler_products_do_not_treat_empty_current_ids_as_root_crate_ownership() {
    let mut hir = resolved_hir_for_products();
    let foreign_function_id = DefId::new(CrateId(0), LocalDefId(50));
    hir.current_def_ids = BTreeSet::new();
    rebuild_program(
        &mut hir.program,
        |functions, structs, enums, traits, impls, externs, _, effective_methods| {
            functions.clear();
            structs.clear();
            enums.clear();
            traits.clear();
            impls.clear();
            externs.clear();
            effective_methods.clear();
            functions.insert(
                foreign_function_id,
                accept_function(test_function(
                    foreign_function_id,
                    "std::identity",
                    generic_params(foreign_function_id, &["T"]),
                )),
            );
        },
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        vec![ProductDependencyIdentity::artifact_object(
            "std".to_string(),
            PathBuf::from("build/std.rkca"),
        )],
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    assert!(products.interface.functions.is_empty());
    assert!(products.bodies.functions.is_empty());
}

#[test]
fn compiler_products_keep_empty_trait_default_bodies_by_product_def_id() {
    let mut hir = resolved_hir_for_products();
    let empty_method_id = DefId::new(CrateId(0), LocalDefId(10));
    hir.current_def_ids.insert(empty_method_id);
    let empty_method = HirFunction {
        body: empty_body(Type::Unit),
        ret_type: Type::Unit,
        ..test_function(empty_method_id, "noop", Vec::new())
    };
    let effective_method_id = DefId::new(CrateId(0), LocalDefId(11));
    hir.current_def_ids.insert(effective_method_id);
    rebuild_program(
        &mut hir.program,
        |_, _, _, traits, impls, _, _, effective_methods| {
            traits
                .get_mut(&DefId::new(CrateId(0), LocalDefId(4)))
                .unwrap()
                .methods
                .insert("noop".to_string(), accept_function(empty_method.clone()));
            let mut effective_method = empty_method;
            effective_method.id = effective_method_id;
            impls
                .get_mut(&DefId::new(CrateId(0), LocalDefId(5)))
                .unwrap()
                .methods
                .insert("noop".to_string(), accept_function(effective_method));
            effective_methods.insert(
                (DefId::new(CrateId(0), LocalDefId(5)), empty_method_id),
                effective_method_id,
            );
        },
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let empty_method_id = ProductDefId::from(empty_method_id);

    assert!(products
        .bodies
        .trait_default_methods
        .contains_key(&empty_method_id));
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
fn compiler_products_roundtrip_preserves_product_def_ids() {
    let hir = resolved_hir_for_products();
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        vec![ProductDependencyIdentity::artifact_object(
            "dep".to_string(),
            std::path::PathBuf::from("build/dep.rkca"),
        )],
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let bytes = bincode::serialize(&products).unwrap();
    let decoded: CompilerProducts = bincode::deserialize(&bytes).unwrap();

    let generic_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(1)));
    assert!(decoded.interface.functions.contains_key(&generic_id));
    assert!(decoded.bodies.functions.contains_key(&generic_id));
    assert_eq!(
        decoded.identity_table.export_names.get("identity"),
        Some(&generic_id)
    );
    assert_eq!(decoded.dependencies[0].name, "dep");
    assert_eq!(
        decoded.dependencies[0].artifact_path,
        std::path::PathBuf::from("build/dep.rkca")
    );
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

    assert!(super::type_table::artifact_type_table_len_for_test(&bytes).unwrap() > 0);
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
    assert!(super::type_table::artifact_type_table_len_for_test(&bytes).unwrap() > 0);
}

#[test]
fn serialized_product_artifact_roundtrips_function_signature_types() {
    let resolved = resolved_hir_for_products();
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let roundtrip =
        CompilerProducts::from_artifact_bytes(&products.to_artifact_bytes().unwrap()).unwrap();
    let function = roundtrip
        .interface
        .functions
        .values()
        .find(|function| function.name == "identity")
        .unwrap();

    assert_eq!(function.params[0], Type::I64);
    assert_eq!(function.ret_type, Type::I64);
}

#[test]
fn serialized_product_artifact_roundtrips_type_alias_identity_and_body() {
    let mut resolved = resolved_hir_for_products();
    let alias_id = DefId::new(resolved.root_crate_id, LocalDefId(200));
    let alias = crate::hir::HirTypeAlias {
        id: alias_id,
        name: "Identity".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: alias_id,
                index: 0,
            },
            "T",
        )],
        ty: Type::Lambda {
            params: vec![crate::type_services::kind::Kind::Type],
            body: Box::new(Type::BoundVar {
                depth: 0,
                index: 0,
                kind: crate::type_services::kind::Kind::Type,
            }),
        },
    };
    mutate_program(&mut resolved.program, |program| {
        program.type_aliases.insert(alias_id, alias.clone());
        program.rebuild_indexes();
    });
    resolved.current_def_ids.insert(alias_id);

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .unwrap();
    let roundtrip =
        CompilerProducts::from_artifact_bytes(&products.to_artifact_bytes().unwrap()).unwrap();
    let product_id = ProductDefId::from(alias_id);
    let roundtrip_alias = roundtrip.interface.type_aliases.get(&product_id).unwrap();

    assert_eq!(roundtrip_alias.id, alias_id);
    assert_eq!(roundtrip_alias.name, "Identity");
    assert_eq!(roundtrip_alias.generic_params, alias.generic_params);
    assert_eq!(roundtrip_alias.ty, alias.ty);
}

#[test]
fn serialized_product_artifact_roundtrips_struct_enum_trait_impl_and_extern_types() {
    let mut resolved = resolved_hir_for_products();
    let field_ty = Type::Pointer(Box::new(Type::U32));
    let variant_ty = Type::Array(Box::new(Type::Bool), 3);
    mutate_program(&mut resolved.program, |program| {
        program
            .structs
            .values_mut()
            .find(|strukt| strukt.name == "Box")
            .unwrap()
            .fields
            .push(crate::hir::HirField {
                id: FieldId(99),
                name: "payload".to_string(),
                ty: field_ty.clone(),
                public: true,
            });
        program
            .enums
            .values_mut()
            .find(|enm| enm.name == "Maybe")
            .unwrap()
            .variants
            .push(crate::hir::HirVariant {
                id: VariantId(99),
                name: "Some".to_string(),
                fields: crate::hir::HirVariantFields::Positional(vec![variant_ty.clone()]),
            });
    });
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let bytes = products.to_artifact_bytes().unwrap();
    let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();

    let strukt = roundtrip
        .interface
        .structs
        .values()
        .find(|strukt| strukt.name == "Box")
        .unwrap();
    assert_eq!(strukt.fields[0].ty, field_ty);
    let enm = roundtrip
        .interface
        .enums
        .values()
        .find(|enm| enm.name == "Maybe")
        .unwrap();
    match &enm.variants[0].fields {
        crate::hir::HirVariantFields::Positional(types) => {
            assert_eq!(types.as_slice(), &[variant_ty.clone()]);
        }
        other => panic!("expected positional variant fields, got {other:?}"),
    }
    assert!(
        super::type_table::artifact_type_table_contains_type_for_test(&bytes, &field_ty).unwrap()
    );
    assert!(
        super::type_table::artifact_type_table_contains_type_for_test(&bytes, &variant_ty).unwrap()
    );
    assert!(roundtrip
        .interface
        .traits
        .values()
        .any(|trait_def| trait_def.name == "demo::Show"));
    assert!(roundtrip.interface.impls.values().any(|imp| !matches!(
        imp.receiver_pattern,
        crate::hir::HirImplReceiverPattern::Exact(Type::Unit)
    )));
    assert!(roundtrip
        .interface
        .externs
        .values()
        .any(|ext| ext.ret == crate::types::Type::I32));
}

#[test]
fn serialized_product_artifact_roundtrips_constructor_impl_target() {
    let mut resolved = resolved_hir_for_products();
    let mut expected = None;
    mutate_program(&mut resolved.program, |program| {
        let box_id = program
            .structs
            .values()
            .find(|structure| structure.name == "Box")
            .unwrap()
            .id;
        let impl_generic = program.impls.values().next().unwrap().type_generics[0].id;
        let target = Type::Lambda {
            params: vec![crate::type_services::kind::Kind::Type],
            body: Box::new(Type::Struct {
                id: box_id,
                args: vec![Type::Tuple(vec![
                    Type::Generic(impl_generic),
                    Type::BoundVar {
                        depth: 0,
                        index: 0,
                        kind: crate::type_services::kind::Kind::Type,
                    },
                ])],
            }),
        };
        program.impls.values_mut().next().unwrap().receiver_pattern =
            crate::hir::HirImplReceiverPattern::Constructor(target.clone());
        expected = Some(target);
    });
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let roundtrip =
        CompilerProducts::from_artifact_bytes(&products.to_artifact_bytes().unwrap()).unwrap();

    assert!(roundtrip.interface.impls.values().any(|imp| {
        imp.receiver_pattern
            == crate::hir::HirImplReceiverPattern::Constructor(expected.clone().unwrap())
    }));
}

#[test]
fn compiler_products_type_table_roundtrips_nested_body_type_locations() {
    let mut resolved = resolved_hir_for_products();
    let block_ty = Type::Array(Box::new(Type::U8), 4);
    let let_ty = Type::Tuple(vec![Type::U16, Type::Bool]);
    let nested_expr_ty = Type::Pointer(Box::new(Type::Bool));
    let cast_ty = Type::Array(Box::new(Type::U32), 2);
    let lambda_param_ty = Type::Pointer(Box::new(Type::F32));
    let lambda_capture_ty = Type::Reference {
        mutable: true,
        inner: Box::new(Type::U64),
    };
    let lambda_body_ty = Type::Slice(Box::new(Type::Char));
    let lambda_expr_ty = Type::function(vec![lambda_param_ty.clone()], lambda_body_ty.clone());
    mutate_program(&mut resolved.program, |program| {
        let identity = program
            .functions
            .values_mut()
            .find(|function| function.name == "identity")
            .unwrap();
        identity.body = crate::hir::HirBlockFor::<AcceptedHir> {
            stmts: vec![
                crate::hir::HirStmtFor::Let {
                    name: "converted".to_string(),
                    local_id: crate::ids::HirLocalId(11),
                    ty: let_ty.clone(),
                    value: crate::hir::HirExprFor::<AcceptedHir> {
                        kind: crate::hir::HirExprKindFor::Cast(
                            Box::new(crate::hir::HirExprFor::<AcceptedHir> {
                                kind: crate::hir::HirExprKindFor::IntLiteral(1),
                                ty: nested_expr_ty.clone(),
                                span: Span::test(),
                            }),
                            cast_ty.clone(),
                        ),
                        ty: cast_ty.clone(),
                        span: Span::test(),
                    },
                    mutable: false,
                },
                crate::hir::HirStmtFor::Expr(crate::hir::HirExprFor::<AcceptedHir> {
                    kind: crate::hir::HirExprKindFor::Lambda {
                        params: vec![HirParam {
                            name: "arg".to_string(),
                            local_id: crate::ids::HirLocalId(12),
                            ty: lambda_param_ty.clone(),
                            mutable: false,
                            is_ref: false,
                        }],
                        body: crate::hir::HirBlockFor::<AcceptedHir> {
                            stmts: vec![crate::hir::HirStmtFor::Expr(crate::hir::HirExprFor::<
                                AcceptedHir,
                            > {
                                kind: crate::hir::HirExprKindFor::Unit,
                                ty: lambda_body_ty.clone(),
                                span: Span::test(),
                            })],
                            ty: lambda_body_ty.clone(),
                        },
                        captures: vec![HirClosureCapture {
                            name: "captured".to_string(),
                            local_id: crate::ids::HirLocalId(13),
                            kind: HirClosureCaptureKind::MutableBorrow,
                            mutable: true,
                            ty: lambda_capture_ty.clone(),
                        }],
                    },
                    ty: lambda_expr_ty.clone(),
                    span: Span::test(),
                }),
            ],
            ty: block_ty.clone(),
        };
    });
    refresh_resolved_type_ids(&mut resolved);
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let bytes = products.to_artifact_bytes().unwrap();
    let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();
    let function = roundtrip
        .bodies
        .functions
        .values()
        .find(|function| function.name == "identity")
        .unwrap();

    assert_eq!(function.body.ty, block_ty);
    let crate::hir::HirStmtFor::Let { ty, value, .. } = &function.body.stmts[0] else {
        panic!("expected let statement");
    };
    assert_eq!(ty, &let_ty);
    assert_eq!(value.ty, cast_ty);
    let crate::hir::HirExprKindFor::Cast(inner, target_ty) = &value.kind else {
        panic!("expected cast expression");
    };
    assert_eq!(inner.ty, nested_expr_ty);
    assert_eq!(target_ty, &cast_ty);
    let crate::hir::HirStmtFor::Expr(lambda) = &function.body.stmts[1] else {
        panic!("expected lambda expression statement");
    };
    assert_eq!(lambda.ty, lambda_expr_ty);
    let crate::hir::HirExprKindFor::Lambda {
        params,
        body,
        captures,
    } = &lambda.kind
    else {
        panic!("expected lambda expression");
    };
    assert_eq!(params[0].ty, lambda_param_ty);
    assert_eq!(captures[0].ty, lambda_capture_ty);
    assert_eq!(body.ty, lambda_body_ty);

    for expected in [
        &let_ty,
        &cast_ty,
        &lambda_param_ty,
        &lambda_capture_ty,
        &lambda_body_ty,
    ] {
        assert!(
            super::type_table::artifact_type_table_contains_type_for_test(&bytes, expected)
                .unwrap(),
            "artifact type table should contain body-only type {expected:?}"
        );
    }
}

#[test]
fn compiler_products_type_table_roundtrips_struct_pattern_type_args() {
    let mut resolved = resolved_hir_for_products();
    let pattern_arg_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::U16),
    };
    mutate_program(&mut resolved.program, |program| {
        let identity = program
            .functions
            .values_mut()
            .find(|function| function.name == "identity")
            .unwrap();
        identity.body = crate::hir::HirBlockFor::<AcceptedHir> {
            stmts: vec![crate::hir::HirStmtFor::Expr(crate::hir::HirExprFor::<
                AcceptedHir,
            > {
                kind: crate::hir::HirExprKindFor::Match {
                    scrutinee: Box::new(crate::hir::HirExprFor::<AcceptedHir> {
                        kind: crate::hir::HirExprKindFor::Var("value".to_string()),
                        ty: Type::I64,
                        span: Span::test(),
                    }),
                    arms: vec![crate::hir::HirMatchArmFor::<AcceptedHir> {
                        pattern: HirPattern::Struct(
                            "Box".to_string(),
                            None,
                            vec![pattern_arg_ty.clone()],
                            vec![HirStructPatternField {
                                name: "value".to_string(),
                                field: None,
                                pattern: HirPattern::Wildcard,
                            }],
                        ),
                        guard: None,
                        body: crate::hir::HirBlockFor::<AcceptedHir> {
                            stmts: Vec::new(),
                            ty: Type::Unit,
                        },
                    }],
                },
                ty: Type::Unit,
                span: Span::test(),
            })],
            ty: Type::Unit,
        };
    });
    refresh_resolved_type_ids(&mut resolved);
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let bytes = products.to_artifact_bytes().unwrap();
    let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();
    let function = roundtrip
        .bodies
        .functions
        .values()
        .find(|function| function.name == "identity")
        .unwrap();
    let crate::hir::HirStmtFor::Expr(expr) = &function.body.stmts[0] else {
        panic!("expected match expression statement");
    };
    let crate::hir::HirExprKindFor::Match { arms, .. } = &expr.kind else {
        panic!("expected match expression");
    };
    let HirPattern::Struct(_, _, type_args, _) = &arms[0].pattern else {
        panic!("expected struct pattern");
    };

    assert_eq!(type_args, &vec![pattern_arg_ty.clone()]);
    assert!(
        super::type_table::artifact_type_table_contains_type_for_test(&bytes, &pattern_arg_ty)
            .unwrap()
    );
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
fn compiler_products_serialize_structural_types_as_explicit_compatibility_boundary() {
    let resolved = resolved_hir_for_products();
    let identity_id = DefId::new(CrateId(0), LocalDefId(1));
    let ret_id = resolved
        .type_id_at(&crate::hir::HirTypeLocation::FunctionReturn {
            function: identity_id,
        })
        .unwrap();

    assert_eq!(resolved.type_at(ret_id), Type::I64);

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let roundtrip =
        CompilerProducts::from_artifact_bytes(&products.to_artifact_bytes().unwrap()).unwrap();
    let function = roundtrip
        .interface
        .functions
        .values()
        .find(|function| function.name == "identity")
        .unwrap();

    assert_eq!(function.ret_type, Type::I64);
    assert_eq!(function.params[0], Type::I64);
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
        .insert("x".repeat(MAX_PRODUCT_ARTIFACT_STRING_BYTES + 1), 1);
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
    assert_eq!(PRODUCT_ARTIFACT_FORMAT_VERSION, 51);
    assert_eq!(
        PRODUCT_ARTIFACT_FORMAT_VERSION,
        rock_shared::sysroot::PRODUCT_ARTIFACT_FORMAT_VERSION
    );
}

#[test]
fn compiler_products_preserve_backend_symbols_in_link_records() {
    let hir = resolved_hir_for_products();
    let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));
    let mut records = BTreeMap::new();
    records.insert(
        plain_id,
        ProductLinkRecord {
            backend_symbol: "rock_main".to_string(),
        },
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData {
            object_path: None,
            records,
        },
    )
    .expect("test HIR has valid product language items");

    assert_eq!(
        products
            .link
            .records
            .get(&plain_id)
            .map(|record| &record.backend_symbol),
        Some(&"rock_main".to_string())
    );
}

#[test]
fn compiler_products_preserve_generic_impls_with_real_local_id_zero() {
    let mut hir = resolved_hir_for_products();
    let first_id = DefId::new(CrateId(0), LocalDefId(0));
    let second_id = DefId::new(CrateId(0), LocalDefId(99));
    hir.current_def_ids.insert(second_id);
    rebuild_program(
        &mut hir.program,
        |_, _, _, _, impls, _, _, effective_methods| {
            *impls = [
                HirImplFor::<AcceptedHir> {
                    id: first_id,
                    owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: generic_params(first_id, &["T"]),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: DefId::new(CrateId(0), LocalDefId(2)),
                        args: vec![Type::Generic(crate::types::GenericParamId {
                            owner: first_id,
                            index: 0,
                        })],
                    }),
                    trait_name: Some("Show".to_string()),
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::new(),
                },
                HirImplFor::<AcceptedHir> {
                    id: second_id,
                    owner: crate::hir::HirImplOwner::Named("Maybe".to_string()),
                    type_name: "Maybe".to_string(),
                    type_generics: generic_params(second_id, &["T"]),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Enum {
                        id: DefId::new(CrateId(0), LocalDefId(3)),
                        args: vec![Type::Generic(crate::types::GenericParamId {
                            owner: second_id,
                            index: 0,
                        })],
                    }),
                    trait_name: Some("Show".to_string()),
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::new(),
                },
            ]
            .into_iter()
            .map(|impl_def| (impl_def.id, impl_def))
            .collect();
            effective_methods.clear();
        },
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    assert_eq!(products.interface.impls.len(), 2);
    assert_eq!(products.bodies.generic_impls.len(), 2);
}

#[test]
fn compiler_products_preserve_real_impl_id_zero_without_collision() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(0));
    let owner_id = DefId::new(CrateId(0), LocalDefId(1));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::from([(
            owner_id,
            HirStruct {
                id: owner_id,
                name: "Box".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: owner_id,
                    args: Vec::new(),
                }),
                trait_name: Some("Show".to_string()),
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([(owner_id, "Box".to_string())]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([impl_id]),
        CrateId(0),
        local_def_ids_after(1),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let product_id = ProductDefId::from(impl_id);

    assert!(products.interface.impls.contains_key(&product_id));
    assert_eq!(products.interface.impls[&product_id].id, impl_id);
}

#[test]
fn compiler_products_preserve_real_extern_id_zero_without_collision() {
    let extern_id = DefId::new(CrateId(0), LocalDefId(0));
    let hir = ResolvedHirProgram::new(
        HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                extern_id,
                HirExtern {
                    id: extern_id,
                    name: "puts".to_string(),
                    params: vec![Type::Pointer(Box::new(Type::U8))],
                    ret: Type::I32,
                    variadic: false,
                    is_unsafe: false,
                },
            )]),
            HirNameTables {
                externs_by_name: HashMap::from([("puts".to_string(), extern_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::new(),
        ),
        ResolverTables::default(),
        BTreeSet::from([extern_id]),
        CrateId(0),
        local_def_ids_after(1),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let product_id = ProductDefId::from(extern_id);

    assert!(products.interface.externs.contains_key(&product_id));
    assert_eq!(products.interface.externs[&product_id].id, extern_id);
}

#[test]
fn compiler_products_preserve_trait_defaults_with_distinct_method_ids() {
    let mut hir = resolved_hir_for_products();
    let trait_id = DefId::new(CrateId(0), LocalDefId(8));
    let method_id = DefId::new(CrateId(0), LocalDefId(9));
    hir.current_def_ids.insert(trait_id);
    hir.current_def_ids.insert(method_id);
    let mut methods = HashMap::new();
    methods.insert(
        "show".to_string(),
        accept_function(test_function(method_id, "show", Vec::new())),
    );
    rebuild_program(&mut hir.program, |_, _, _, traits, _, _, _, _| {
        traits.insert(
            trait_id,
            HirTraitFor::<AcceptedHir> {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Debug".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods,
                signatures: HashMap::new(),
            },
        );
    });

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    assert_eq!(products.bodies.trait_default_methods.len(), 2);
}

#[test]
fn compiler_products_share_signature_and_default_member_identity() {
    let mut hir = resolved_hir_for_products();
    let trait_id = DefId::new(CrateId(0), LocalDefId(4));
    let member_id = DefId::new(CrateId(0), LocalDefId(7));
    let impl_id = DefId::new(CrateId(0), LocalDefId(5));
    let effective_id = DefId::new(CrateId(0), LocalDefId(8));
    rebuild_program(&mut hir.program, |_, _, _, traits, _, _, _, _| {
        let trait_def = traits.get_mut(&trait_id).unwrap();
        let method = &trait_def.methods["show"];
        trait_def.signatures.insert(
            "show".to_string(),
            HirFunctionSig {
                id: member_id,
                name: "show".to_string(),
                generic_params: method.generic_params.clone(),
                params: method.params.iter().map(|param| param.ty.clone()).collect(),
                ret: method.ret_type.clone(),
                generic_bounds: method.generic_bounds.clone(),
                self_receiver: method.self_receiver,
                is_unsafe: method.is_unsafe,
            },
        );
    });
    let (products, remap) = CompilerProducts::from_resolved_hir_with_remap(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let trait_def = &products.interface.traits[&ProductDefId::from(trait_id)];
    assert_eq!(
        trait_def.methods["show"].id,
        trait_def.signatures["show"].id
    );
    assert_eq!(trait_def.methods["show"].id, member_id);
    assert_eq!(remap[&ProductDefId::from(member_id)].len(), 1);
    assert_eq!(
        products.interface.effective_trait_methods
            [&(ProductDefId::from(impl_id), ProductDefId::from(member_id))],
        ProductDefId::from(effective_id),
    );
    assert!(products
        .bodies
        .trait_default_methods
        .contains_key(&ProductDefId::from(member_id)));
}

#[test]
fn compiler_products_record_trait_default_method_ids_in_metadata() {
    let mut hir = resolved_hir_for_products();
    let trait_id = DefId::new(CrateId(0), LocalDefId(8));
    let method_id = DefId::new(CrateId(0), LocalDefId(0));
    hir.current_def_ids.insert(trait_id);
    let mut methods = HashMap::new();
    methods.insert(
        "show".to_string(),
        accept_function(test_function(method_id, "show", Vec::new())),
    );
    rebuild_program(&mut hir.program, |_, _, _, traits, _, _, _, _| {
        traits.insert(
            trait_id,
            HirTraitFor::<AcceptedHir> {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Debug".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods,
                signatures: HashMap::new(),
            },
        );
    });
    hir.local_def_ids = local_def_ids_after(9);
    let next_raw = hir.local_def_ids.next_raw();

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let debug_trait_id = ProductDefId::from(trait_id);
    let interface_method_id = products.interface.traits[&debug_trait_id].methods["show"].id;
    let debug_method_id = ProductDefId::from(interface_method_id);

    assert_ne!(debug_method_id, ProductDefId::from(method_id));
    assert!(debug_method_id.local_id.0 >= next_raw);

    assert_eq!(
        products.bodies.trait_default_methods[&debug_method_id].id,
        interface_method_id
    );
    assert_eq!(
        products.interface.traits[&debug_trait_id].methods["show"].id,
        interface_method_id
    );
}

#[test]
fn compiler_products_prefer_exact_trait_default_method_identity() {
    let shared_id = DefId::new(CrateId(0), LocalDefId(7));
    let exact_method = test_function(shared_id, "right", Vec::new());
    let accepted = crate::hir::AcceptedHirProgram::try_from(
        HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::from([(shared_id, exact_method)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("right".to_string(), shared_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([(shared_id, "right".to_string())]),
        ),
    )
    .expect("fixture HIR is accepted");
    let exact_method = accepted.function_by_id(shared_id).unwrap().1.clone();
    let preferred = super::PreferredTraitDefaultMethods {
        by_id: HashMap::from([(shared_id, exact_method)]),
    };

    let selected = preferred.get(shared_id, "Debug", "show");

    assert_eq!(selected.name, "right");
}

#[test]
#[should_panic(expected = "trait default method should have a preferred body")]
fn compiler_products_do_not_fallback_trait_defaults_by_display_name() {
    let missing_id = DefId::new(CrateId(0), LocalDefId(7));
    let preferred = super::PreferredTraitDefaultMethods {
        by_id: HashMap::new(),
    };

    let _ = preferred.get(missing_id, "Debug", "show");
}

#[test]
fn compiler_products_record_impl_display_names_without_export_names() {
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

    assert!(products
        .identity_table
        .display_names
        .values()
        .any(|name| name == "Box as Show"));
    assert!(!products
        .identity_table
        .export_names
        .contains_key("Box as Show"));
}

#[test]
fn compiler_products_do_not_export_trait_default_methods() {
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

    assert!(products
        .identity_table
        .display_names
        .values()
        .any(|name| name == "Show::show"));
    assert!(!products
        .identity_table
        .export_names
        .contains_key("Show::show"));
}

#[test]
fn compiler_products_do_not_export_colliding_impl_method_display_names() {
    let method_a_id = DefId::new(CrateId(0), LocalDefId(10));
    let method_b_id = DefId::new(CrateId(0), LocalDefId(11));
    let impl_a_id = DefId::new(CrateId(0), LocalDefId(12));
    let impl_b_id = DefId::new(CrateId(0), LocalDefId(13));
    let owner_id = DefId::new(CrateId(0), LocalDefId(14));
    let impls = HashMap::from([
        (
            impl_a_id,
            HirImpl {
                id: impl_a_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: owner_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "show".to_string(),
                    test_function(method_a_id, "show", Vec::new()),
                )]),
            },
        ),
        (
            impl_b_id,
            HirImpl {
                id: impl_b_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: owner_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "show".to_string(),
                    test_function(method_b_id, "show", Vec::new()),
                )]),
            },
        ),
    ]);
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::from([(
            owner_id,
            HirStruct {
                id: owner_id,
                name: "Box".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        impls,
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([(owner_id, "Box".to_string())]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([impl_a_id, impl_b_id, method_a_id, method_b_id]),
        CrateId(0),
        local_def_ids_after(22),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let colliding_methods = products
        .identity_table
        .display_names
        .values()
        .filter(|name| *name == "Box::show")
        .count();
    assert_eq!(colliding_methods, 2);
    assert!(!products
        .identity_table
        .export_names
        .contains_key("Box::show"));
}

#[test]
fn compiler_products_drop_ambiguous_export_name_collisions() {
    let mut identity_table = ProductIdentityTable::default();
    let first = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(1),
    };
    let second = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(2),
    };

    super::record_export_name(&mut identity_table, first, "same");
    super::record_export_name(&mut identity_table, second, "same");

    assert!(!identity_table.export_names.contains_key("same"));
}

#[test]
fn compiler_products_keep_export_name_ambiguous_after_third_collision() {
    let mut identity_table = ProductIdentityTable::default();
    let first = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(1),
    };
    let second = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(2),
    };
    let third = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(3),
    };

    super::record_export_name(&mut identity_table, first, "same");
    super::record_export_name(&mut identity_table, second, "same");
    super::record_export_name(&mut identity_table, third, "same");

    assert!(!identity_table.export_names.contains_key("same"));
}

#[test]
fn compiler_products_preserve_link_records_for_real_local_id_zero() {
    let impl_def_id = DefId::new(CrateId(0), LocalDefId(0));
    let mut records = BTreeMap::new();
    records.insert(
        ProductDefId::from(impl_def_id),
        ProductLinkRecord {
            backend_symbol: "impl_box_show".to_string(),
        },
    );
    let owner_id = DefId::new(CrateId(0), LocalDefId(1));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::from([(
            owner_id,
            HirStruct {
                id: owner_id,
                name: "Box".to_string(),
                generic_params: generic_params(owner_id, &["T"]),
                fields: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            impl_def_id,
            HirImpl {
                id: impl_def_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: generic_params(impl_def_id, &["T"]),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: owner_id,
                    args: vec![Type::Generic(crate::types::GenericParamId {
                        owner: impl_def_id,
                        index: 0,
                    })],
                }),
                trait_name: Some("Show".to_string()),
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([(owner_id, "Box".to_string())]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([impl_def_id]),
        CrateId(0),
        IdGen::new(),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData {
            object_path: None,
            records,
        },
    )
    .expect("test HIR has valid product language items");
    let impl_id = products.interface.impls.keys().copied().next().unwrap();

    assert_eq!(impl_id, ProductDefId::from(impl_def_id));
    assert_eq!(
        products.interface.impls[&impl_id].id,
        super::product_def_id_to_def_id(impl_id)
    );
    assert_eq!(
        products
            .link
            .records
            .get(&impl_id)
            .map(|record| &record.backend_symbol),
        Some(&"impl_box_show".to_string())
    );
}

#[test]
fn compiler_products_drop_link_records_for_ambiguous_link_ids() {
    let shared_id = DefId::new(CrateId(0), LocalDefId(0));
    let shared_product_id = ProductDefId::from(shared_id);
    let symbol = "ambiguous_impl_symbol".to_string();
    let mut records = BTreeMap::new();
    records.insert(
        shared_product_id,
        ProductLinkRecord {
            backend_symbol: symbol.clone(),
        },
    );
    let mut functions = HashMap::new();
    functions.insert(shared_id, test_function(shared_id, "plain", Vec::new()));
    let owner_id = DefId::new(CrateId(0), LocalDefId(1));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        functions,
        HashMap::from([(
            owner_id,
            HirStruct {
                id: owner_id,
                name: "Box".to_string(),
                generic_params: generic_params(owner_id, &["T"]),
                fields: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            shared_id,
            HirImpl {
                id: shared_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: generic_params(shared_id, &["T"]),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: owner_id,
                    args: vec![Type::Generic(crate::types::GenericParamId {
                        owner: shared_id,
                        index: 0,
                    })],
                }),
                trait_name: Some("Show".to_string()),
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("plain".to_string(), shared_id)]),
            structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([
            (shared_id, "plain".to_string()),
            (owner_id, "Box".to_string()),
        ]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([shared_id]),
        CrateId(0),
        IdGen::new(),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData {
            object_path: None,
            records,
        },
    )
    .expect("test HIR has valid product language items");

    assert!(!products
        .link
        .records
        .values()
        .any(|record| record.backend_symbol == symbol));
}

#[test]
fn compiler_products_drop_aliases_for_ambiguous_product_ids() {
    let shared_id = DefId::new(CrateId(0), LocalDefId(0));
    let mut functions = HashMap::new();
    functions.insert(shared_id, test_function(shared_id, "plain", Vec::new()));
    let mut resolver = ResolverTables::default();
    resolver.insert_import_alias_with_name(
        "short".to_string(),
        "demo::plain".to_string(),
        shared_id,
    );
    resolver.insert_module_alias_with_name(
        "mod_short".to_string(),
        "demo::plain".to_string(),
        shared_id,
    );
    let owner_id = DefId::new(CrateId(0), LocalDefId(1));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        functions,
        HashMap::from([(
            owner_id,
            HirStruct {
                id: owner_id,
                name: "Box".to_string(),
                generic_params: generic_params(owner_id, &["T"]),
                fields: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            shared_id,
            HirImpl {
                id: shared_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: generic_params(shared_id, &["T"]),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: owner_id,
                    args: vec![Type::Generic(crate::types::GenericParamId {
                        owner: shared_id,
                        index: 0,
                    })],
                }),
                trait_name: Some("Show".to_string()),
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("plain".to_string(), shared_id)]),
            structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([
            (shared_id, "plain".to_string()),
            (owner_id, "Box".to_string()),
        ]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        resolver,
        BTreeSet::from([shared_id]),
        CrateId(0),
        IdGen::new(),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    assert!(!products
        .identity_table
        .import_alias_names
        .contains_key("short"));
    assert!(!products
        .identity_table
        .module_alias_names
        .contains_key("mod_short"));
}

#[test]
fn compiler_products_keep_trait_default_ids_stable_across_trait_order() {
    let first = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved_hir_with_traits_in_order(&["Show", "Debug"]),
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let second = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved_hir_with_traits_in_order(&["Debug", "Show"]),
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    for display_name in ["Debug::show", "Show::show"] {
        let first_id = first
            .identity_table
            .display_names
            .iter()
            .find_map(|(id, name)| (name == display_name).then_some(*id));
        let second_id = second
            .identity_table
            .display_names
            .iter()
            .find_map(|(id, name)| (name == display_name).then_some(*id));

        assert_eq!(first_id, second_id, "{display_name}");
    }
}
