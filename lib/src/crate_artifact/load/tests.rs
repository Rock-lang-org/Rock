use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;

use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::{ArtifactCrateInterface, ArtifactCrossCrateHir};
use crate::crate_system::{
    CrateContext, ExternCrateBodies, ExternCrateLink, ExternCrateMetadata, ExternCrateRecord,
};
use crate::hir::{
    AcceptedHir, AcceptedHirBlock as HirBlock, AcceptedHirExpr as HirExpr,
    AcceptedHirFunction as HirFunction, AcceptedHirImpl as HirImpl, AcceptedHirTrait as HirTrait,
    HirAssociatedTypeDecl, HirCallTarget, HirClosureCapture, HirClosureCaptureKind, HirEnum,
    HirExprKindFor, HirExtern, HirField, HirFieldLocation, HirFunctionSig, HirImplOwner,
    HirMethodCallTarget, HirParam, HirPattern, HirStruct, HirStructPatternField, HirVarRef,
    HirVarTarget, HirVariant, HirVariantFields, HirVariantLocation,
};
use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
use crate::language_items::DropLanguageItems;
use crate::lexer::Span;
use crate::products::{
    CompilerProducts, ProductBodies, ProductCrateId, ProductCrateIdentity, ProductDefId,
    ProductEnumInterface, ProductExternInterface, ProductFieldInterface, ProductFunctionInterface,
    ProductIdentityTable, ProductImplInterface, ProductInterface, ProductLinkData,
    ProductLinkRecord, ProductLocalDefId, ProductSourceFingerprint, ProductStructInterface,
    ProductTraitInterface, PRODUCT_ARTIFACT_MAGIC,
};
use crate::type_services::kind::Kind;
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, Type};

type HirMatchArm = crate::hir::HirMatchArmFor<AcceptedHir>;
type HirStmt = crate::hir::HirStmtFor<AcceptedHir>;

fn product_def_id(crate_id: u32, local_id: u32) -> ProductDefId {
    ProductDefId {
        crate_id: ProductCrateId(crate_id),
        local_id: ProductLocalDefId(local_id),
    }
}

fn product_hir_def_id(id: ProductDefId) -> DefId {
    DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0))
}

fn generic_params(owner: DefId, names: &[&str]) -> Vec<GenericParamDecl> {
    GenericParamDecl::type_params(owner, names.iter().copied())
}

#[test]
fn remap_var_target_child_location_rejects_serialized_instance_target() {
    let product_crate = ProductCrateId(1);
    let remap = super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table: ProductIdentityTable::default(),
        interface: crate::products::ProductInterface::default(),
        bodies: ProductBodies::default(),
        link: ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: BTreeMap::new(),
        proc_macros: Vec::new(),
    };
    let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut reference = HirVarRef {
        name: "identity".to_string(),
        target: HirVarTarget::Instance(crate::ids::InstanceId(5)),
    };

    let err = super::remap_var_target_child_location(
        &mut reference,
        &remap,
        &validator,
        &BTreeSet::new(),
    )
    .unwrap_err();

    assert!(
        err.contains("local instance target"),
        "unexpected error: {err}"
    );
}

#[test]
fn remap_pattern_child_locations_remaps_new_pattern_sidecars() {
    let product_crate = ProductCrateId(1);
    let old_struct = product_def_id(1, 10);
    let old_enum = product_def_id(1, 11);
    let remap = super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let mut products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table: ProductIdentityTable::default(),
        interface: crate::products::ProductInterface::default(),
        bodies: ProductBodies::default(),
        link: ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: BTreeMap::new(),
        proc_macros: Vec::new(),
    };
    let struct_def = HirStruct {
        id: product_hir_def_id(old_struct),
        name: "dep::Box".to_string(),
        generic_params: Vec::new(),
        fields: vec![HirField {
            id: FieldId(0),
            name: "value".to_string(),
            ty: Type::I64,
            public: true,
        }],
    };
    products
        .interface
        .structs
        .insert(old_struct, ProductStructInterface::from(&struct_def));
    let enum_def = HirEnum {
        id: product_hir_def_id(old_enum),
        name: "dep::Option".to_string(),
        generic_params: Vec::new(),
        variants: vec![HirVariant {
            id: VariantId(0),
            name: "Some".to_string(),
            fields: HirVariantFields::Positional(vec![Type::I64]),
        }],
    };
    products
        .interface
        .enums
        .insert(old_enum, ProductEnumInterface::from(&enum_def));
    let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let mut pattern = HirPattern::Enum(
        "dep::Option".to_string(),
        "Some".to_string(),
        Some(HirVariantLocation {
            owner: product_hir_def_id(old_enum),
            variant_id: VariantId(0),
            name: "Some".to_string(),
        }),
        vec![HirPattern::Struct(
            "dep::Box".to_string(),
            Some(product_hir_def_id(old_struct)),
            vec![Type::Struct {
                id: product_hir_def_id(old_struct),
                args: Vec::new(),
            }],
            vec![HirStructPatternField {
                name: "value".to_string(),
                field: Some(HirFieldLocation {
                    owner: product_hir_def_id(old_struct),
                    field_id: FieldId(0),
                    name: "value".to_string(),
                }),
                pattern: HirPattern::Wildcard,
            }],
        )],
    );

    super::remap_pattern_child_locations(&mut pattern, &remap, &child_validator, &validator, 0)
        .unwrap();

    let expected_struct = remap.def_id(old_struct).unwrap();
    let expected_enum = remap.def_id(old_enum).unwrap();
    let HirPattern::Enum(_, _, Some(location), payloads) = pattern else {
        panic!("expected enum pattern");
    };
    assert_eq!(location.owner, expected_enum);
    let HirPattern::Struct(_, Some(struct_id), args, fields) = &payloads[0] else {
        panic!("expected struct payload pattern");
    };
    assert_eq!(*struct_id, expected_struct);
    assert_eq!(
        args[0],
        Type::Struct {
            id: expected_struct,
            args: Vec::new(),
        }
    );
    assert_eq!(fields[0].field.as_ref().unwrap().owner, expected_struct);
}

#[test]
fn remap_expr_child_locations_remaps_call_target_sidecars() {
    let product_crate = ProductCrateId(1);
    let function_product_id = product_def_id(1, 10);
    let extern_product_id = product_def_id(1, 11);
    let function_id = product_hir_def_id(function_product_id);
    let extern_id = product_hir_def_id(extern_product_id);
    let remap = super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let mut products = product_with_function(
        "dep",
        product_crate,
        function_product_id.local_id.0,
        PathBuf::from("dep.o"),
    );
    insert_interface_extern(
        &mut products,
        extern_product_id,
        HirExtern {
            id: extern_id,
            name: "foreign".to_string(),
            params: Vec::new(),
            ret: Type::I64,
            variadic: false,
            is_unsafe: false,
        },
    );
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut expr = HirExpr {
        kind: HirExprKindFor::TupleLiteral(vec![
            HirExpr {
                kind: HirExprKindFor::Call(
                    Box::new(HirExpr {
                        kind: HirExprKindFor::Var("dep::answer".to_string()),
                        ty: Type::function(Vec::new(), Type::I64),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    Some(HirCallTarget::Function(function_id)),
                ),
                ty: Type::I64,
                span: Span::test(),
            },
            HirExpr {
                kind: HirExprKindFor::Call(
                    Box::new(HirExpr {
                        kind: HirExprKindFor::Var("foreign".to_string()),
                        ty: Type::function(Vec::new(), Type::I64),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    Some(HirCallTarget::Extern(extern_id)),
                ),
                ty: Type::I64,
                span: Span::test(),
            },
        ]),
        ty: Type::Tuple(vec![Type::I64, Type::I64]),
        span: Span::test(),
    };

    super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
        .unwrap();

    let HirExprKindFor::TupleLiteral(items) = &expr.kind else {
        panic!("expected tuple literal");
    };
    let HirExprKindFor::Call(_, _, Some(function_target)) = &items[0].kind else {
        panic!("expected function call target");
    };
    assert_eq!(
        function_target,
        &HirCallTarget::Function(remap.def_id(function_product_id).unwrap())
    );
    let HirExprKindFor::Call(_, _, Some(extern_target)) = &items[1].kind else {
        panic!("expected extern call target");
    };
    assert_eq!(
        extern_target,
        &HirCallTarget::Extern(remap.def_id(extern_product_id).unwrap())
    );
}

#[test]
fn remap_function_child_locations_rejects_unknown_local_call_target() {
    let product_crate = ProductCrateId(1);
    let function_product_id = product_def_id(1, 10);
    let remap = super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let products = product_with_function(
        "dep",
        product_crate,
        function_product_id.local_id.0,
        PathBuf::from("dep.o"),
    );
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut function = products.bodies.functions[&function_product_id].clone();
    function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::Call(
                Box::new(HirExpr {
                    kind: HirExprKindFor::Var("local_fn".to_string()),
                    ty: Type::function(Vec::new(), Type::I64),
                    span: Span::test(),
                }),
                Vec::new(),
                Some(HirCallTarget::Local(crate::ids::HirLocalId(99))),
            ),
            ty: Type::I64,
            span: Span::test(),
        })],
        ty: Type::I64,
    };

    let err = super::remap_function_child_locations(
        &mut function,
        &remap,
        &child_validator,
        &type_validator,
    )
    .unwrap_err();

    assert!(
        err.contains("unknown local call target"),
        "unexpected error: {err}"
    );
}

#[test]
fn remap_function_child_locations_rejects_unknown_local_resolved_var_target() {
    let product_crate = ProductCrateId(1);
    let function_product_id = product_def_id(1, 10);
    let remap = super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let products = product_with_function(
        "dep",
        product_crate,
        function_product_id.local_id.0,
        PathBuf::from("dep.o"),
    );
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut function = products.bodies.functions[&function_product_id].clone();
    function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "local_fn".to_string(),
                target: HirVarTarget::Local(crate::ids::HirLocalId(99)),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };

    let err = super::remap_function_child_locations(
        &mut function,
        &remap,
        &child_validator,
        &type_validator,
    )
    .unwrap_err();

    assert!(
        err.contains("unknown local var target"),
        "unexpected error: {err}"
    );
}

#[test]
fn remap_function_child_locations_rejects_call_target_to_lambda_local() {
    let product_crate = ProductCrateId(1);
    let function_product_id = product_def_id(1, 10);
    let lambda_local = crate::ids::HirLocalId(42);
    let remap = super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let products = product_with_function(
        "dep",
        product_crate,
        function_product_id.local_id.0,
        PathBuf::from("dep.o"),
    );
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut function = products.bodies.functions[&function_product_id].clone();
    function.body = HirBlock {
        stmts: vec![
            HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::Lambda {
                    params: vec![HirParam {
                        name: "callback".to_string(),
                        local_id: lambda_local,
                        ty: Type::function(Vec::new(), Type::I64),
                        mutable: false,
                        is_ref: false,
                    }],
                    body: HirBlock {
                        stmts: Vec::new(),
                        ty: Type::Unit,
                    },
                    captures: Vec::new(),
                },
                ty: Type::function(Vec::new(), Type::Unit),
                span: Span::test(),
            }),
            HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::Call(
                    Box::new(HirExpr {
                        kind: HirExprKindFor::Var("callback".to_string()),
                        ty: Type::function(Vec::new(), Type::I64),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    Some(HirCallTarget::Local(lambda_local)),
                ),
                ty: Type::I64,
                span: Span::test(),
            }),
        ],
        ty: Type::I64,
    };

    let err = super::remap_function_child_locations(
        &mut function,
        &remap,
        &child_validator,
        &type_validator,
    )
    .unwrap_err();

    assert!(
        err.contains("unknown local call target"),
        "unexpected error: {err}"
    );
}

#[test]
fn remap_function_child_locations_rejects_call_target_to_current_let_local() {
    let product_crate = ProductCrateId(1);
    let function_product_id = product_def_id(1, 10);
    let let_local = crate::ids::HirLocalId(7);
    let remap = super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let products = product_with_function(
        "dep",
        product_crate,
        function_product_id.local_id.0,
        PathBuf::from("dep.o"),
    );
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut function = products.bodies.functions[&function_product_id].clone();
    function.body = HirBlock {
        stmts: vec![HirStmt::Let {
            name: "local_fn".to_string(),
            local_id: let_local,
            ty: Type::I64,
            value: HirExpr {
                kind: HirExprKindFor::Call(
                    Box::new(HirExpr {
                        kind: HirExprKindFor::Var("local_fn".to_string()),
                        ty: Type::function(Vec::new(), Type::I64),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    Some(HirCallTarget::Local(let_local)),
                ),
                ty: Type::I64,
                span: Span::test(),
            },
            mutable: false,
        }],
        ty: Type::I64,
    };

    let err = super::remap_function_child_locations(
        &mut function,
        &remap,
        &child_validator,
        &type_validator,
    )
    .unwrap_err();

    assert!(
        err.contains("unknown local call target"),
        "unexpected error: {err}"
    );
}

fn test_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::I64,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::IntLiteral(5),
                ty: Type::I64,
                span: Span::test(),
            })],
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

#[test]
fn intern_loaded_interface_types_rejects_one_display_alias_for_distinct_ids() {
    let first = DefId::new(CrateId(0), LocalDefId(1));
    let second = DefId::new(CrateId(0), LocalDefId(2));
    let mut second_function = test_function(second, "payload_second");
    second_function.ret_type = Type::Bool;
    second_function.body = HirBlock {
        stmts: Vec::new(),
        ty: Type::Unit,
    };

    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function(
        "same_alias".to_string(),
        test_function(first, "payload_first"),
    );
    interface.insert_function("same_alias".to_string(), second_function);
    let mut context = crate::type_context::TypeContext::new();

    let error =
        super::intern_loaded_interface_types(&interface, &ResolverTables::default(), &mut context)
            .expect_err("distinct artifact IDs must not share one display alias");

    assert!(error.contains("artifact function display alias 'same_alias'"));
}

#[test]
fn artifact_name_tables_keep_resolver_aliases_on_the_payload_def_id() {
    let id = DefId::new(CrateId(0), LocalDefId(3));
    let function = test_function(id, "payload_display");
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function("dep::canonical".to_string(), function.clone());
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(id, "dep::canonical".to_string());
    resolver.import_aliases.insert("short".to_string(), id);
    let unresolved =
        crate::crate_artifact::types::hir_function_from_interface(&interface.functions[&id]);

    let names = super::artifact_hir_name_tables(
        &interface,
        &resolver,
        &HashMap::from([(id, unresolved)]),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    )
    .expect("resolver alias should remain attached to the exact artifact DefId");

    assert_eq!(names.functions_by_name["short"], id);
    assert_eq!(names.functions_by_name["dep::canonical"], id);
}

#[test]
fn artifact_alias_conflicts_are_stable_across_reverse_insertion() {
    fn conflict(reverse: bool) -> String {
        let first = DefId::new(CrateId(0), LocalDefId(4));
        let second = DefId::new(CrateId(0), LocalDefId(5));
        let mut resolver = ResolverTables::default();
        let (first_source, second_source) = if reverse {
            (first, second)
        } else {
            (second, first)
        };
        resolver.item_paths.insert("same".to_string(), first_source);
        resolver
            .item_names_by_id
            .insert(second_source, "same".to_string());
        let functions = HashMap::from([
            (
                first,
                crate::crate_artifact::types::hir_function_from_interface(
                    &ProductFunctionInterface::from(&test_function(first, "first")),
                ),
            ),
            (
                second,
                crate::crate_artifact::types::hir_function_from_interface(
                    &ProductFunctionInterface::from(&test_function(second, "second")),
                ),
            ),
        ]);

        super::artifact_hir_name_tables(
            &ArtifactCrateInterface::default(),
            &resolver,
            &functions,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        )
        .expect_err("same artifact alias must have a stable conflict")
    }

    let expected = "artifact function display alias 'same' resolves to both DefId { crate_id: CrateId(0), local: LocalDefId(4) } and DefId { crate_id: CrateId(0), local: LocalDefId(5) }";
    assert_eq!(conflict(false), expected);
    assert_eq!(conflict(true), expected);
}

fn test_generic_impl(id: DefId, methods: HashMap<String, HirFunction>) -> HirImpl {
    let receiver_pattern = methods
        .values()
        .find(|method| method.is_method)
        .and_then(|method| method.params.first())
        .map(|param| match &param.ty {
            Type::Reference { inner, .. } => inner.as_ref().clone(),
            ty => ty.clone(),
        })
        .map(crate::hir::HirImplReceiverPattern::Exact)
        .unwrap_or_else(|| {
            crate::hir::HirImplReceiverPattern::Exact(Type::Generic(crate::types::GenericParamId {
                owner: id,
                index: 0,
            }))
        });
    HirImpl {
        id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: generic_params(id, &["T"]),
        receiver_pattern,
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods,
    }
}

fn insert_interface_function(
    products: &mut CompilerProducts,
    id: ProductDefId,
    function: &HirFunction,
) {
    products
        .interface
        .functions
        .insert(id, ProductFunctionInterface::from(function));
}

fn insert_function_body(products: &mut CompilerProducts, id: ProductDefId, function: HirFunction) {
    insert_interface_function(products, id, &function);
    products.bodies.functions.insert(id, function);
}

fn insert_interface_struct(products: &mut CompilerProducts, id: ProductDefId, strukt: HirStruct) {
    products
        .interface
        .structs
        .insert(id, ProductStructInterface::from(&strukt));
}

fn insert_interface_enum(products: &mut CompilerProducts, id: ProductDefId, enm: HirEnum) {
    products
        .interface
        .enums
        .insert(id, ProductEnumInterface::from(&enm));
}

fn insert_interface_trait(products: &mut CompilerProducts, id: ProductDefId, trait_def: HirTrait) {
    products
        .interface
        .traits
        .insert(id, ProductTraitInterface::from(&trait_def));
}

fn insert_interface_impl(products: &mut CompilerProducts, id: ProductDefId, imp: HirImpl) {
    products
        .interface
        .impls
        .insert(id, ProductImplInterface::from(&imp));
}

#[test]
fn product_artifact_rejects_receiver_pattern_missing_impl_generic() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let impl_id = product_def_id(0, 7);
    let imp = test_generic_impl(product_hir_def_id(impl_id), HashMap::new());
    insert_interface_impl(&mut products, impl_id, imp);
    products
        .interface
        .impls
        .get_mut(&impl_id)
        .unwrap()
        .receiver_pattern = crate::hir::HirImplReceiverPattern::Exact(Type::Unit);

    let error = super::validate_product_method_authority_maps(
        &products,
        &super::ProductDependencyDefinitions::default(),
    )
    .expect_err("receiver pattern must bind every impl generic");

    assert!(
        error.contains("receiver/trait argument generic bindings") && error.contains("0::7"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_allows_impl_generic_bound_by_trait_argument() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let impl_id = product_def_id(0, 8);
    let hir_impl_id = product_hir_def_id(impl_id);
    let mut imp = test_generic_impl(hir_impl_id, HashMap::new());
    imp.type_generics = generic_params(hir_impl_id, &["T", "U"]);
    imp.receiver_pattern =
        crate::hir::HirImplReceiverPattern::Exact(Type::Generic(crate::types::GenericParamId {
            owner: hir_impl_id,
            index: 0,
        }));
    imp.trait_arg_types = vec![Type::Generic(crate::types::GenericParamId {
        owner: hir_impl_id,
        index: 1,
    })];
    insert_interface_impl(&mut products, impl_id, imp);

    super::validate_product_method_authority_maps(
        &products,
        &super::ProductDependencyDefinitions::default(),
    )
    .expect("trait arguments may bind impl generics absent from the receiver");
}

fn insert_interface_extern(products: &mut CompilerProducts, id: ProductDefId, ext: HirExtern) {
    products
        .interface
        .externs
        .insert(id, ProductExternInterface::from(&ext));
}

fn product_with_function(
    crate_name: &str,
    product_crate_id: ProductCrateId,
    local_id: u32,
    object_path: PathBuf,
) -> CompilerProducts {
    let product_id = ProductDefId {
        crate_id: product_crate_id,
        local_id: ProductLocalDefId(local_id),
    };
    let function_id = DefId::new(CrateId(product_crate_id.0), LocalDefId(local_id));
    let function_name = format!("{}::answer", crate_name);
    let function = test_function(function_id, &function_name);

    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(product_crate_id);
    identity_table
        .display_names
        .insert(product_id, function_name.clone());
    identity_table
        .export_names
        .insert("answer".to_string(), product_id);

    let mut bodies = ProductBodies::default();
    bodies.functions.insert(product_id, function);

    let mut interface = ProductInterface::default();
    interface.functions.insert(
        product_id,
        ProductFunctionInterface::from(&bodies.functions[&product_id]),
    );

    let mut link_records = BTreeMap::new();
    link_records.insert(
        product_id,
        ProductLinkRecord {
            backend_symbol: format!("{}_answer", crate_name),
        },
    );

    CompilerProducts {
        crate_identity: ProductCrateIdentity::local(crate_name.to_string()),
        identity_table,
        interface,
        bodies,
        link: ProductLinkData {
            object_path: Some(object_path),
            records: link_records,
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    }
}

fn write_product_fixture(base: &std::path::Path, crate_name: &str) -> PathBuf {
    let artifact_path = base.join(format!("{}.rkca", crate_name));
    let object_path = base.join(format!("{}.o", crate_name));
    fs::write(&object_path, []).unwrap();

    let products = product_with_function(crate_name, ProductCrateId(0), 0, object_path);
    products.write_artifact_to_path(&artifact_path).unwrap();

    artifact_path
}

struct TempDirCleanup(PathBuf);

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_test_dir(label: &str) -> (PathBuf, TempDirCleanup) {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        label
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let cleanup = TempDirCleanup(base.clone());
    (base, cleanup)
}

#[test]
fn load_product_artifact_inserts_artifact_only_extern_record() {
    let (base, _cleanup) = temp_test_dir("artifact_only_extern_record");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let products = product_with_function("dep", ProductCrateId(0), 0, object_path.clone());
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx
        .extern_crate("dep")
        .expect("extern record should be inserted");
    let answer_id = dep
        .metadata()
        .interface()
        .function_by_canonical_name("dep::answer")
        .expect("function interface should exist")
        .id;
    assert!(dep.body_providers().generic_function(answer_id).is_some());
    assert_eq!(
        dep.link().object_path(),
        Some(&object_path.canonicalize().unwrap())
    );
}

#[test]
fn load_product_artifact_rejects_body_only_function_without_interface_declaration() {
    let (base, _cleanup) = temp_test_dir("body_only_function_without_interface");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.interface.functions.clear();
    products.identity_table.export_names.clear();
    products.link.records.clear();
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("body-only function rows should be rejected");

    assert!(
        err.contains("function body") && err.contains("interface declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_link_record_for_undeclared_callable_id() {
    let (base, _cleanup) = temp_test_dir("link_record_undeclared_callable");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let undeclared_id = product_def_id(0, 9);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.link.records.insert(
        undeclared_id,
        ProductLinkRecord {
            backend_symbol: "dep_hidden".to_string(),
        },
    );
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("link records for undeclared callable IDs should be rejected");

    assert!(
        err.contains("link record")
            && err.contains("backend symbol")
            && err.contains("interface callable declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_empty_link_record_backend_symbol() {
    let (base, _cleanup) = temp_test_dir("empty_link_record_backend_symbol");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let function_id = product_def_id(0, 0);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products
        .link
        .records
        .get_mut(&function_id)
        .unwrap()
        .backend_symbol = String::new();
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("empty backend symbols should be rejected");

    assert!(
        err.contains("0::0") && err.contains("empty backend symbol"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_object_function_missing_link_record() {
    let (base, _cleanup) = temp_test_dir("object_function_missing_link_record");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.bodies.functions.clear();
    products.link.records.clear();
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("object-backed concrete functions require link records");

    assert!(
        err.contains("missing link record")
            && err.contains("backend symbol")
            && err.contains("function"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_object_concrete_function_body_without_link_record() {
    let (base, _cleanup) = temp_test_dir("object_concrete_function_body_missing_link_record");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.link.records.clear();
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("object-backed concrete function bodies still require link records");

    assert!(
        err.contains("missing link record")
            && err.contains("backend symbol")
            && err.contains("function"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_object_impl_method_missing_link_record() {
    let (base, _cleanup) = temp_test_dir("object_impl_method_missing_link_record");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let artifact_path = base.join("dep.rkca");
    let impl_id = DefId::new(CrateId(0), LocalDefId(7));
    let method_id = DefId::new(CrateId(0), LocalDefId(8));
    let product_impl_id = ProductDefId::from(impl_id);
    let product_method_id = ProductDefId::from(method_id);
    let mut method = test_function(method_id, "make");
    method.is_method = true;
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("make".to_string(), method)]),
    };

    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(0));
    identity_table
        .display_names
        .insert(product_impl_id, "Box".to_string());
    identity_table
        .display_names
        .insert(product_method_id, "Box::make".to_string());
    let mut interface = ProductInterface::default();
    interface
        .impls
        .insert(product_impl_id, ProductImplInterface::from(&imp));
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies: ProductBodies::default(),
        link: ProductLinkData {
            object_path: Some(object_path),
            records: BTreeMap::new(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("object-backed concrete impl methods require link records");

    assert!(
        err.contains("missing link record")
            && err.contains("backend symbol")
            && err.contains("impl")
            && err.contains("method 'make'"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_object_concrete_impl_method_when_sibling_is_generic() {
    let (base, _cleanup) = temp_test_dir("object_impl_concrete_method_generic_sibling");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let artifact_path = base.join("dep.rkca");
    let impl_id = DefId::new(CrateId(0), LocalDefId(7));
    let method_id = DefId::new(CrateId(0), LocalDefId(8));
    let generic_method_id = DefId::new(CrateId(0), LocalDefId(9));
    let product_impl_id = ProductDefId::from(impl_id);
    let product_method_id = ProductDefId::from(method_id);
    let product_generic_method_id = ProductDefId::from(generic_method_id);

    let mut method = test_function(method_id, "make");
    method.is_method = true;

    let generic_param = crate::types::GenericParamId {
        owner: generic_method_id,
        index: 0,
    };
    let mut generic_method = test_function(generic_method_id, "id");
    generic_method.is_method = true;
    generic_method
        .generic_params
        .push(GenericParamDecl::type_param(generic_param, "T"));

    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([
            ("make".to_string(), method),
            ("id".to_string(), generic_method),
        ]),
    };

    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(0));
    identity_table
        .display_names
        .insert(product_impl_id, "Box".to_string());
    identity_table
        .display_names
        .insert(product_method_id, "Box::make".to_string());
    identity_table
        .display_names
        .insert(product_generic_method_id, "Box::id".to_string());
    let mut interface = ProductInterface::default();
    interface
        .impls
        .insert(product_impl_id, ProductImplInterface::from(&imp));
    let mut bodies = ProductBodies::default();
    bodies.generic_impls.insert(product_impl_id, imp);
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies,
        link: ProductLinkData {
            object_path: Some(object_path),
            records: BTreeMap::new(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("object-backed concrete impl methods require link records");

    assert!(
        err.contains("missing link record")
            && err.contains("backend symbol")
            && err.contains("impl")
            && err.contains("method 'make'"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_allows_generic_function_without_link_record() {
    let (base, _cleanup) = temp_test_dir("generic_function_without_link_record");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let artifact_path = base.join("dep.rkca");
    let product_id = product_def_id(0, 0);
    let function_id = product_hir_def_id(product_id);
    let mut function = test_function(function_id, "dep::id");
    function.generic_params.push(GenericParamDecl::type_param(
        crate::types::GenericParamId {
            owner: function_id,
            index: 0,
        },
        "T",
    ));

    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(0));
    identity_table
        .display_names
        .insert(product_id, "dep::id".to_string());
    let mut interface = ProductInterface::default();
    interface
        .functions
        .insert(product_id, ProductFunctionInterface::from(&function));
    let mut bodies = ProductBodies::default();
    bodies.functions.insert(product_id, function);
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies,
        link: ProductLinkData {
            object_path: Some(object_path),
            records: BTreeMap::new(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();
}

#[test]
fn load_product_artifact_rejects_body_only_function_link_record_without_interface_declaration() {
    let (base, _cleanup) = temp_test_dir("body_only_function_link_record");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let body_only_id = product_def_id(0, 9);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.bodies.functions.insert(
        body_only_id,
        test_function(product_hir_def_id(body_only_id), "hidden"),
    );
    products.link.records.insert(
        body_only_id,
        ProductLinkRecord {
            backend_symbol: "dep_hidden".to_string(),
        },
    );
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("body-only function link records should be rejected as backend symbols");

    assert!(
        err.contains("link record")
            && err.contains("backend symbol")
            && err.contains("interface callable declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_body_only_trait_default_without_interface_declaration() {
    let (base, _cleanup) = temp_test_dir("body_only_trait_default_without_interface");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let method_id = product_def_id(0, 7);
    let method_def = product_hir_def_id(method_id);
    let mut products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table: ProductIdentityTable {
            local_crate: Some(ProductCrateId(0)),
            display_names: BTreeMap::from([(method_id, "dep::Show::show".to_string())]),
            ..ProductIdentityTable::default()
        },
        interface: ProductInterface::default(),
        bodies: ProductBodies::default(),
        link: ProductLinkData {
            object_path: Some(object_path),
            records: BTreeMap::new(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: BTreeMap::new(),
        proc_macros: Vec::new(),
    };
    products
        .bodies
        .trait_default_methods
        .insert(method_id, test_function(method_def, "show"));
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("body-only trait default rows should be rejected");

    assert!(
        err.contains("trait default body") && err.contains("interface declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_body_only_generic_impl_without_interface_declaration() {
    let (base, _cleanup) = temp_test_dir("body_only_generic_impl_without_interface");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let impl_id = product_def_id(0, 7);
    products.bodies.generic_impls.insert(
        impl_id,
        HirImpl {
            id: product_hir_def_id(impl_id),
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: generic_params(product_hir_def_id(impl_id), &["T"]),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        },
    );
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("body-only generic impl rows should be rejected");

    assert!(
        err.contains("generic impl body") && err.contains("interface declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_generic_impl_body_extra_method_without_interface_declaration() {
    let (base, _cleanup) = temp_test_dir("generic_impl_body_extra_method");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let impl_id = product_def_id(0, 7);
    let method_id = product_def_id(0, 8);
    let extra_method_id = product_def_id(0, 9);

    let mut interface_method = test_function(product_hir_def_id(method_id), "value");
    interface_method.is_method = true;
    let interface_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([("value".to_string(), interface_method.clone())]),
    );
    insert_interface_impl(&mut products, impl_id, interface_impl);

    let mut extra_method = test_function(product_hir_def_id(extra_method_id), "hidden");
    extra_method.is_method = true;
    let body_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([
            ("value".to_string(), interface_method),
            ("hidden".to_string(), extra_method),
        ]),
    );
    products.bodies.generic_impls.insert(impl_id, body_impl);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("extra generic impl body methods should be rejected");

    assert!(
        err.contains("generic impl body")
            && err.contains("method 'hidden'")
            && err.contains("interface declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_generic_impl_body_method_id_mismatch() {
    let (base, _cleanup) = temp_test_dir("generic_impl_body_method_id_mismatch");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let impl_id = product_def_id(0, 7);
    let method_id = product_def_id(0, 8);
    let mismatched_method_id = product_def_id(0, 9);

    let mut interface_method = test_function(product_hir_def_id(method_id), "value");
    interface_method.is_method = true;
    let interface_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([("value".to_string(), interface_method)]),
    );
    insert_interface_impl(&mut products, impl_id, interface_impl);

    let mut body_method = test_function(product_hir_def_id(mismatched_method_id), "value");
    body_method.is_method = true;
    let body_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([("value".to_string(), body_method)]),
    );
    products.bodies.generic_impls.insert(impl_id, body_impl);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("generic impl body method ID mismatches should be rejected");

    assert!(
        err.contains("generic impl body")
            && err.contains("method 'value'")
            && err.contains("interface declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_trait_default_body_embedded_id_mismatch() {
    let (base, _cleanup) = temp_test_dir("trait_default_body_embedded_id_mismatch");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let trait_id = product_def_id(0, 7);
    let method_id = product_def_id(0, 8);
    let mismatched_method_id = product_def_id(0, 9);

    let mut interface_method = test_function(product_hir_def_id(method_id), "show");
    interface_method.is_method = true;
    let trait_def = HirTrait {
        target: None,
        predicates: Vec::new(),
        id: product_hir_def_id(trait_id),
        name: "Show".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("show".to_string(), interface_method)]),
        signatures: HashMap::new(),
    };
    insert_interface_trait(&mut products, trait_id, trait_def);
    products
        .identity_table
        .display_names
        .insert(trait_id, "dep::Show".to_string());

    let mut body = test_function(product_hir_def_id(mismatched_method_id), "show");
    body.is_method = true;
    products
        .bodies
        .trait_default_methods
        .insert(method_id, body);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("trait default body embedded ID mismatches should be rejected");

    assert!(
        err.contains("trait default body")
            && err.contains("embedded ID")
            && err.contains("interface declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_invalid_drop_trait_language_item_id() {
    let (base, _cleanup) = temp_test_dir("invalid_drop_trait_language_item");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.interface.language_items.drop = Some(DropLanguageItems {
        trait_id: product_def_id(0, 99),
        method_id: product_def_id(0, 100),
    });
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("invalid drop trait language items should be rejected");

    assert_eq!(
            err,
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(99) }: drop.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(99) } is no declaration"
        );
}

#[test]
fn load_product_artifact_rejects_nonlocal_sized_language_item_without_mutating_context() {
    let (base, _cleanup) = temp_test_dir("nonlocal_sized_language_item");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.interface.language_items.sized = Some(crate::language_items::SizedLanguageItems {
        trait_id: product_def_id(1, 7),
    });
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let mut type_context = crate::type_context::TypeContext::new();
    let error = ctx
        .load_product_artifact_from_path_with_type_context(artifact_path, &mut type_context)
        .expect_err("non-local language items must be rejected before loading");

    assert_eq!(
            error,
            "Product artifact language item sized.trait uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(7) }; expected product crate 0"
        );
    assert!(ctx.extern_crate("dep").is_none());
    assert_eq!(type_context.len(), 0);
}

#[test]
fn load_product_artifact_rejects_mismatched_drop_method_language_item_id() {
    let (base, _cleanup) = temp_test_dir("mismatched_drop_method_language_item");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let drop_trait_id = product_def_id(0, 7);
    let other_trait_id = product_def_id(0, 9);
    let other_method_id = product_def_id(0, 10);
    let other_method_def = product_hir_def_id(other_method_id);

    insert_interface_trait(
        &mut products,
        drop_trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: product_hir_def_id(drop_trait_id),
            name: "Drop".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    products
        .identity_table
        .display_names
        .insert(drop_trait_id, "dep::Drop".to_string());
    insert_interface_trait(
        &mut products,
        other_trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: product_hir_def_id(other_trait_id),
            name: "Other".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "drop".to_string(),
                HirFunctionSig {
                    id: other_method_def,
                    name: "drop".to_string(),
                    generic_params: Vec::new(),
                    params: Vec::new(),
                    ret: Type::Unit,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        },
    );
    products
        .identity_table
        .display_names
        .insert(other_trait_id, "dep::Other".to_string());
    products.interface.language_items.drop = Some(DropLanguageItems {
        trait_id: drop_trait_id,
        method_id: other_method_id,
    });
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("drop method language items must belong to the drop trait");

    assert_eq!(
            err,
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(7) }: drop.method DefId { crate_id: CrateId(0), local: LocalDefId(10) } is owned by trait DefId { crate_id: CrateId(0), local: LocalDefId(9) }, not DefId { crate_id: CrateId(0), local: LocalDefId(7) }"
        );
}

#[test]
fn load_product_artifact_rejects_drop_method_language_item_not_declared_by_trait() {
    let (base, _cleanup) = temp_test_dir("drop_method_not_declared_by_trait");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let drop_trait_id = product_def_id(0, 7);
    insert_interface_trait(
        &mut products,
        drop_trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: product_hir_def_id(drop_trait_id),
            name: "Drop".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    products
        .identity_table
        .display_names
        .insert(drop_trait_id, "dep::Drop".to_string());
    products.interface.language_items.drop = Some(DropLanguageItems {
        trait_id: drop_trait_id,
        method_id: product_def_id(0, 8),
    });
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("drop method language item must be declared by the drop trait");

    assert_eq!(
            err,
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(7) }: drop.method DefId { crate_id: CrateId(0), local: LocalDefId(8) } is not declared by any trait"
        );
}

#[test]
fn load_product_artifact_rejects_generic_impl_body_omitted_interface_method() {
    let (base, _cleanup) = temp_test_dir("generic_impl_body_omitted_interface_method");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let impl_id = product_def_id(0, 7);
    let method_id = product_def_id(0, 8);

    let mut interface_method = test_function(product_hir_def_id(method_id), "value");
    interface_method.is_method = true;
    let interface_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([("value".to_string(), interface_method)]),
    );
    insert_interface_impl(&mut products, impl_id, interface_impl);

    let body_impl = test_generic_impl(product_hir_def_id(impl_id), HashMap::new());
    products.bodies.generic_impls.insert(impl_id, body_impl);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("generic impl bodies must include every interface method body");

    assert!(
        err.contains("generic impl body")
            && err.contains("method 'value'")
            && err.contains("serialized body"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_generic_function_interface_without_body() {
    let (base, _cleanup) = temp_test_dir("generic_function_interface_without_body");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let function_id = product_def_id(0, 0);
    let function_def = product_hir_def_id(function_id);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let generic = crate::types::GenericParamId {
        owner: function_def,
        index: 0,
    };
    let function = products.interface.functions.get_mut(&function_id).unwrap();
    function
        .generic_params
        .push(GenericParamDecl::type_param(generic, "T"));
    products.bodies.functions.remove(&function_id);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("generic function interface declarations need serialized bodies");

    assert!(
        err.contains("generic function") && err.contains("serialized body"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_generic_impl_interface_without_body() {
    let (base, _cleanup) = temp_test_dir("generic_impl_interface_without_body");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let impl_id = product_def_id(0, 7);
    let method_id = product_def_id(0, 8);
    let mut method = test_function(product_hir_def_id(method_id), "value");
    method.is_method = true;
    let interface_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([("value".to_string(), method)]),
    );
    insert_interface_impl(&mut products, impl_id, interface_impl);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("generic impl interface declarations need serialized bodies");

    assert!(
        err.contains("generic impl") && err.contains("serialized body"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_interface_function_embedded_id_mismatch() {
    let (base, _cleanup) = temp_test_dir("interface_function_embedded_id_mismatch");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let function_id = product_def_id(0, 0);
    let mismatched_id = product_def_id(0, 9);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products
        .interface
        .functions
        .get_mut(&function_id)
        .unwrap()
        .id = product_hir_def_id(mismatched_id);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("interface function ID mismatches should be rejected");

    assert!(
        err.contains("interface function") && err.contains("embedded ID"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_interface_function_foreign_row_key() {
    let (base, _cleanup) = temp_test_dir("interface_function_foreign_row_key");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let foreign_function_id = product_def_id(1, 9);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("foreign".to_string()),
    );
    products
        .identity_table
        .display_names
        .insert(foreign_function_id, "foreign::answer".to_string());
    insert_interface_function(
        &mut products,
        foreign_function_id,
        &test_function(product_hir_def_id(foreign_function_id), "foreign::answer"),
    );
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("interface row keys must be owned by the artifact crate");

    assert!(
        err.contains("interface function") && err.contains("non-local"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_interface_trait_method_foreign_product_crate_id() {
    let (base, _cleanup) = temp_test_dir("interface_trait_method_foreign_product_crate");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let trait_id = product_def_id(0, 7);
    let foreign_method_id = product_def_id(1, 8);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("foreign".to_string()),
    );
    let mut method = test_function(product_hir_def_id(foreign_method_id), "show");
    method.is_method = true;
    insert_interface_trait(
        &mut products,
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: product_hir_def_id(trait_id),
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("show".to_string(), method.clone())]),
            signatures: HashMap::new(),
        },
    );
    products
        .identity_table
        .display_names
        .insert(trait_id, "dep::Show".to_string());
    products
        .bodies
        .trait_default_methods
        .insert(foreign_method_id, method);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("nested trait method IDs must be owned by the artifact crate");

    assert!(
        err.contains("trait") && err.contains("method 'show'") && err.contains("non-local"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_interface_shared_default_member_id_requires_matching_signature() {
    let trait_id = product_def_id(0, 1);
    let member_id = product_hir_def_id(product_def_id(0, 2));
    let method = test_function(member_id, "value");
    let signature = crate::hir::HirFunctionSig {
        id: member_id,
        name: "value".to_string(),
        generic_params: method.generic_params.clone(),
        params: method.params.iter().map(|param| param.ty.clone()).collect(),
        ret: method.ret_type.clone(),
        generic_bounds: method.generic_bounds.clone(),
        self_receiver: method.self_receiver,
        is_unsafe: method.is_unsafe,
    };
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_trait(
        &mut products,
        trait_id,
        HirTrait {
            id: product_hir_def_id(trait_id),
            name: "Value".to_string(),
            generic_params: Vec::new(),
            target: None,
            predicates: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("value".to_string(), method)]),
            signatures: HashMap::from([("value".to_string(), signature)]),
        },
    );
    super::validate_product_interface_rows(&products).unwrap();
    for change in 0..3 {
        let mut malformed = products.clone();
        let signature = malformed
            .interface
            .traits
            .get_mut(&trait_id)
            .unwrap()
            .signatures
            .get_mut("value")
            .unwrap();
        match change {
            0 => signature.ret = Type::Bool,
            1 => signature.params.push(Type::I64),
            _ => signature.is_unsafe = true,
        }
        assert!(super::validate_product_interface_rows(&malformed)
            .unwrap_err()
            .contains("disagrees"));
    }
    let trait_def = products.interface.traits.get_mut(&trait_id).unwrap();
    let mut unrelated = trait_def.signatures["value"].clone();
    unrelated.name = "other".to_string();
    trait_def.signatures.insert("other".to_string(), unrelated);
    assert!(super::validate_product_interface_rows(&products)
        .unwrap_err()
        .contains("duplicate callable ID"));
}

#[test]
fn load_product_artifact_rejects_nested_method_reusing_top_level_function_id() {
    let (base, _cleanup) = temp_test_dir("nested_method_reuses_function_id");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let function_id = product_def_id(0, 0);
    let impl_id = product_def_id(0, 7);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let mut method = test_function(product_hir_def_id(function_id), "value");
    method.is_method = true;
    insert_interface_impl(
        &mut products,
        impl_id,
        HirImpl {
            id: product_hir_def_id(impl_id),
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        },
    );
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("callable declaration IDs must be unique");

    assert!(
        err.contains("duplicate callable ID")
            && err.contains("function")
            && err.contains("method 'value'"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_interface_impl_method_name_mismatch() {
    let (base, _cleanup) = temp_test_dir("interface_impl_method_name_mismatch");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let impl_id = product_def_id(0, 7);
    let method_id = product_def_id(0, 8);
    let mut method = test_function(product_hir_def_id(method_id), "hidden");
    method.is_method = true;
    let interface_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([("value".to_string(), method)]),
    );
    insert_interface_impl(&mut products, impl_id, interface_impl);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("interface impl method names must match method row keys");

    assert!(
        err.contains("interface impl")
            && err.contains("method 'value'")
            && err.contains("name 'hidden'"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_reinterns_structural_types_for_consumer_context() {
    let (base, _cleanup) = temp_test_dir("reinterns_structural_types");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();
    let bytes = std::fs::read(&artifact_path).unwrap();
    assert!(crate::products::artifact_type_table_len_for_test(&bytes).unwrap() > 0);

    let mut type_context = crate::type_context::TypeContext::new();
    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_with_type_context(artifact_path, &mut type_context)
        .unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let function = dep
        .metadata()
        .interface()
        .function_by_canonical_name("dep::answer")
        .unwrap();
    let ret_id = type_context.id_for_type(&function.ret_type).unwrap();

    assert_eq!(type_context.type_for(ret_id), function.ret_type);
}

#[test]
fn load_product_artifact_as_reinterns_structural_types_for_consumer_context() {
    let (base, _cleanup) = temp_test_dir("as_reinterns_structural_types");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();
    let bytes = std::fs::read(&artifact_path).unwrap();
    assert!(crate::products::artifact_type_table_len_for_test(&bytes).unwrap() > 0);

    let mut type_context = crate::type_context::TypeContext::new();
    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_as_with_type_context(
        "dep",
        artifact_path,
        &mut type_context,
    )
    .unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let function = dep
        .metadata()
        .interface()
        .function_by_canonical_name("dep::answer")
        .unwrap();
    let ret_id = type_context.id_for_type(&function.ret_type).unwrap();

    assert_eq!(type_context.type_for(ret_id), function.ret_type);
}

#[test]
fn load_product_artifact_does_not_mutate_type_context_when_add_fails() {
    let (base, _cleanup) = temp_test_dir("failed_add_does_not_mutate_context");
    let first_object_path = base.join("dep.o");
    fs::write(&first_object_path, []).unwrap();
    let first_products = product_with_function("dep", ProductCrateId(0), 0, first_object_path);
    let first_artifact_path = base.join("dep.rkca");
    first_products
        .write_artifact_to_path(&first_artifact_path)
        .unwrap();

    let duplicate_object_path = base.join("dep_duplicate.o");
    fs::write(&duplicate_object_path, []).unwrap();
    let mut duplicate_products =
        product_with_function("dep", ProductCrateId(0), 0, duplicate_object_path);
    let product_id = product_def_id(0, 0);
    if let Some(function) = duplicate_products.interface.functions.get_mut(&product_id) {
        function.params.push(Type::Bool);
        function.ret_type = Type::Bool;
    }
    if let Some(function) = duplicate_products.bodies.functions.get_mut(&product_id) {
        function.params.push(HirParam {
            name: "flag".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Bool,
            mutable: false,
            is_ref: false,
        });
        function.ret_type = Type::Bool;
        function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::BoolLiteral(true),
                ty: Type::Bool,
                span: Span::test(),
            })],
            ty: Type::Bool,
        };
    }
    let duplicate_artifact_path = base.join("dep_duplicate.rkca");
    duplicate_products
        .write_artifact_to_path(&duplicate_artifact_path)
        .unwrap();

    let mut type_context = crate::type_context::TypeContext::new();
    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_with_type_context(first_artifact_path, &mut type_context)
        .unwrap();

    let err = ctx
        .load_product_artifact_from_path_with_type_context(
            duplicate_artifact_path,
            &mut type_context,
        )
        .unwrap_err();

    assert!(err.contains("duplicate external crate"));
    assert!(type_context.id_for_type(&Type::Bool).is_none());
}

#[test]
fn load_product_artifact_failure_does_not_mutate_crate_context_identity_state() {
    let (base, _cleanup) = temp_test_dir("failed_load_does_not_mutate_crate_context");
    let bad_object_path = base.join("bad.o");
    fs::write(&bad_object_path, []).unwrap();
    let mut bad_products = product_with_function("bad", ProductCrateId(0), 0, bad_object_path);
    bad_products.identity_table.dependencies.insert(
        ProductCrateId(0),
        ProductCrateIdentity::local("invalid_dep".to_string()),
    );
    let bad_artifact_path = base.join("bad.rkca");
    bad_products
        .write_artifact_to_path(&bad_artifact_path)
        .unwrap();

    let mut type_context = crate::type_context::TypeContext::new();
    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path_with_type_context(bad_artifact_path, &mut type_context)
        .unwrap_err();

    assert!(err.contains("maps dependency 'invalid_dep' to local product crate ID 0"));
    assert!(ctx.product_crate_ids.is_empty());

    let good_artifact_path = write_product_fixture(&base, "good");
    ctx.load_product_artifact_from_path_with_type_context(good_artifact_path, &mut type_context)
        .unwrap();

    assert_eq!(ctx.extern_crate("good").unwrap().crate_id(), CrateId(1));
}

#[test]
fn load_product_artifact_rejects_previous_format_before_context_mutation() {
    let (base, _cleanup) = temp_test_dir("rejects_previous_format_before_mutation");
    let artifact_path = base.join("old.rkca");
    let mut preamble = Vec::new();
    preamble.extend_from_slice(&PRODUCT_ARTIFACT_MAGIC);
    preamble.extend_from_slice(&43u32.to_le_bytes());
    preamble.extend_from_slice(&0u64.to_le_bytes());
    preamble.extend_from_slice(&0u64.to_le_bytes());
    fs::write(&artifact_path, preamble).unwrap();

    let mut ctx = CrateContext::new();
    let mut type_context = crate::type_context::TypeContext::new();
    let error = ctx
        .load_product_artifact_from_path_with_type_context(artifact_path, &mut type_context)
        .unwrap_err();

    assert!(error.contains("Unsupported product artifact format 43"));
    assert!(ctx.product_crate_ids.is_empty());
    assert!(type_context.id_for_type(&Type::I64).is_none());
}

#[test]
fn load_product_artifact_remap_failure_is_transactional_for_contexts() {
    let (base, _cleanup) = temp_test_dir("remap_failure_is_transactional");
    let good_object_path = base.join("good.o");
    fs::write(&good_object_path, []).unwrap();
    let good_artifact_path = base.join("good.rkca");
    product_with_function("good", ProductCrateId(0), 0, good_object_path)
        .write_artifact_to_path(&good_artifact_path)
        .unwrap();

    let bad_object_path = base.join("bad.o");
    fs::write(&bad_object_path, []).unwrap();
    let bad_artifact_path = base.join("bad.rkca");
    let bad_id = product_def_id(0, 0);
    let mut bad_products = product_with_function("bad", ProductCrateId(0), 0, bad_object_path);
    let mut bad_function = bad_products.bodies.functions[&bad_id].clone();
    bad_function.params.push(HirParam {
        name: "ghost".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(999)),
            args: Vec::new(),
        },
        mutable: false,
        is_ref: false,
    });
    insert_function_body(&mut bad_products, bad_id, bad_function);
    bad_products
        .write_artifact_to_path(&bad_artifact_path)
        .unwrap();

    let mut ctx = CrateContext::new();
    let mut type_context = crate::type_context::TypeContext::new();
    ctx.load_product_artifact_from_path_with_type_context(good_artifact_path, &mut type_context)
        .unwrap();
    let product_ids_before = ctx.product_crate_ids.clone();
    let i64_id_before = type_context.id_for_type(&Type::I64);

    let error = ctx
        .load_product_artifact_from_path_with_type_context(bad_artifact_path, &mut type_context)
        .unwrap_err();

    assert!(error.contains("unknown nominal type definition"));
    assert_eq!(ctx.product_crate_ids, product_ids_before);
    assert_eq!(type_context.id_for_type(&Type::I64), i64_id_before);
    assert!(ctx.extern_crate("good").is_some());
    assert!(ctx.extern_crate("bad").is_none());
}

#[test]
fn product_artifact_rejects_nested_error_in_unexported_interface_type() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let struct_id = product_def_id(0, 7);
    products.interface.structs.insert(
        struct_id,
        ProductStructInterface {
            id: product_hir_def_id(struct_id),
            name: "dep::Private".to_string(),
            generic_params: Vec::new(),
            fields: vec![ProductFieldInterface {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::Tuple(vec![Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Error),
                }]),
                public: false,
            }],
        },
    );

    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let error =
        super::validate_product_type_fields(&products, &remap, &CrateContext::new()).unwrap_err();

    assert!(error.contains("Type::Error"), "unexpected error: {error}");
}

#[test]
fn product_artifact_rejects_nested_type_var_in_unexported_body_type() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let function_id = product_def_id(0, 0);
    products
        .bodies
        .functions
        .get_mut(&function_id)
        .unwrap()
        .body
        .ty = Type::Tuple(vec![Type::Reference {
        mutable: false,
        inner: Box::new(Type::TypeVar(crate::ids::TypeVarId(0))),
    }]);

    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let error =
        super::validate_product_type_fields(&products, &remap, &CrateContext::new()).unwrap_err();

    assert!(
        error.contains("unresolved type"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_invalid_generic_defaults() {
    let id = product_def_id(0, 7);
    let owner = product_hir_def_id(id);
    for default in [
        Type::Error,
        Type::TypeVar(crate::ids::TypeVarId(0)),
        Type::Generic(GenericParamId { owner, index: 0 }),
        Type::Generic(GenericParamId {
            owner: product_hir_def_id(product_def_id(0, 0)),
            index: 0,
        }),
        Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::I64),
        },
        Type::BoundVar {
            depth: 0,
            index: 0,
            kind: Kind::Type,
        },
    ] {
        let mut products =
            product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
        let mut param = GenericParamDecl::type_param(GenericParamId { owner, index: 0 }, "T");
        param.default = Some(default);
        products.interface.structs.insert(
            id,
            ProductStructInterface {
                id: owner,
                name: "dep::Private".into(),
                generic_params: vec![param],
                fields: vec![],
            },
        );
        let remap = super::ProductIdentityRemap {
            local_crate: ProductCrateId(0),
            crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
        };
        assert!(
            super::validate_product_type_fields(&products, &remap, &CrateContext::new()).is_err()
        );
    }
}

#[test]
fn load_product_artifact_remaps_id_backed_import_aliases() {
    let (base, _cleanup) = temp_test_dir("id_backed_import_aliases");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 1, object_path);
    products
        .identity_table
        .import_alias_names
        .insert("short".to_string(), product_def_id(0, 1));

    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();
    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();
    let record = ctx.extern_crate("dep").unwrap();
    let resolver = record.metadata().resolver();

    assert_eq!(
        resolver.resolve_item_or_alias("short"),
        resolver.resolve_item_or_alias("dep::answer")
    );
}

#[test]
fn load_product_artifact_drops_aliases_without_canonical_bindings() {
    let (base, _cleanup) = temp_test_dir("stale_id_backed_aliases");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 1, object_path);
    products
        .identity_table
        .import_alias_names
        .insert("stale".to_string(), product_def_id(0, 99));
    products
        .identity_table
        .module_alias_names
        .insert("stale_mod".to_string(), product_def_id(0, 100));

    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();
    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();
    let record = ctx.extern_crate("dep").unwrap();
    let resolver = record.metadata().resolver();

    assert_eq!(resolver.resolve_item_or_alias("stale"), None);
    assert_eq!(resolver.resolve_item_or_alias("stale_mod"), None);
}

#[test]
fn load_product_artifacts_remap_same_local_crate_ids_to_distinct_consumer_crates() {
    let (base, _cleanup) = temp_test_dir("distinct_consumer_crates");

    let a_artifact = write_product_fixture(&base, "a");
    let b_artifact = write_product_fixture(&base, "b");

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(a_artifact).unwrap();
    ctx.load_product_artifact_from_path(b_artifact).unwrap();

    let a_id = ctx
        .extern_crate("a")
        .unwrap()
        .metadata()
        .resolver()
        .item_paths["a::answer"];
    let b_id = ctx
        .extern_crate("b")
        .unwrap()
        .metadata()
        .resolver()
        .item_paths["b::answer"];

    assert_ne!(a_id.crate_id, b_id.crate_id);
    assert_eq!(a_id.local, LocalDefId(0));
    assert_eq!(b_id.local, LocalDefId(0));
}

#[test]
fn product_crate_id_remap_reuses_shared_transitive_dependency_identity() {
    let shared = ProductCrateIdentity::local("shared".to_string());
    let mut first = product_with_function("first", ProductCrateId(0), 0, PathBuf::from("first.o"));
    first
        .identity_table
        .dependencies
        .insert(ProductCrateId(1), shared.clone());

    let mut second =
        product_with_function("second", ProductCrateId(0), 0, PathBuf::from("second.o"));
    second
        .identity_table
        .dependencies
        .insert(ProductCrateId(1), shared);

    let mut ctx = CrateContext::new();
    let first_remap = super::ProductIdentityRemap::from_products(&mut ctx, &first).unwrap();
    let second_remap = super::ProductIdentityRemap::from_products(&mut ctx, &second).unwrap();

    assert_ne!(
        first_remap.crate_ids[&ProductCrateId(0)],
        second_remap.crate_ids[&ProductCrateId(0)]
    );
    assert_eq!(
        first_remap.crate_ids[&ProductCrateId(1)],
        second_remap.crate_ids[&ProductCrateId(1)]
    );
}

#[test]
fn load_product_artifact_rejects_dependency_identity_mismatch() {
    let (base, _cleanup) = temp_test_dir("dependency_identity_mismatch");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    let mut mismatched_identity = ProductCrateIdentity::local("dep".to_string());
    mismatched_identity.format_version = mismatched_identity.format_version + 1;
    app_products
        .identity_table
        .dependencies
        .insert(ProductCrateId(1), mismatched_identity);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    let err = ctx
        .load_product_artifact_from_path(app_artifact)
        .unwrap_err();

    assert!(
        err.contains("dependency identity mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_dependency_fingerprint_mismatch() {
    let (base, _cleanup) = temp_test_dir("dependency_fingerprint_mismatch");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    dep_products.crate_identity.source_fingerprint.source_hash = Some("dep-v1".to_string());
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    let mut expected_identity = ProductCrateIdentity::local("dep".to_string());
    expected_identity.source_fingerprint.source_hash = Some("dep-v2".to_string());
    app_products
        .identity_table
        .dependencies
        .insert(ProductCrateId(1), expected_identity);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    let err = ctx
        .load_product_artifact_from_path(app_artifact)
        .unwrap_err();

    assert!(
        err.contains("dependency identity mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_remaps_generic_body_ids_with_interface_ids() {
    let (base, _cleanup) = temp_test_dir("generic_body_ids");
    let artifact_path = write_product_fixture(&base, "dep");

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let interface_id = dep
        .metadata()
        .interface()
        .function_by_canonical_name("dep::answer")
        .expect("function interface should exist")
        .id;
    let body_id = dep
        .body_providers()
        .generic_function(interface_id)
        .expect("generic function body should exist")
        .id;

    assert_eq!(interface_id, body_id);
    assert_ne!(body_id.crate_id, CrateId(0));
}

#[test]
fn product_artifact_generic_param_descriptor_round_trip_remaps_owners() {
    let (base, _cleanup) = temp_test_dir("generic_param_owner_remap");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let function_def = DefId::new(CrateId(0), LocalDefId(0));
    let generic = crate::types::GenericParamId {
        owner: function_def,
        index: 0,
    };
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let mut function = products.bodies.functions[&function_id].clone();
    function.generic_params = vec![GenericParamDecl::type_param(generic, "T")];
    function.params = vec![HirParam {
        name: "value".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Generic(generic),
        mutable: false,
        is_ref: false,
    }];
    function.ret_type = Type::Generic(generic);
    function.body.ty = Type::Generic(generic);
    insert_function_body(&mut products, function_id, function);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let interface_function = dep
        .metadata()
        .interface()
        .function_by_canonical_name("dep::answer")
        .expect("function interface should exist");
    let expected_generic = GenericParamDecl::new(
        GenericParamId {
            owner: interface_function.id,
            index: 0,
        },
        "T",
        Kind::Type,
    );
    assert_eq!(
        interface_function.generic_params,
        vec![expected_generic.clone()]
    );
    let body_function = dep
        .body_providers()
        .generic_function(interface_function.id)
        .expect("generic function body should exist");
    assert_eq!(body_function.generic_params, vec![expected_generic]);
    let Type::Generic(param) = body_function.params[0].ty else {
        panic!("expected remapped generic param in artifact body");
    };

    assert_eq!(param.owner, interface_function.id);
    assert_ne!(param.owner.crate_id, CrateId(0));
}

#[test]
fn product_artifact_remaps_function_generic_bounds() {
    let (base, _cleanup) = temp_test_dir("function_generic_bounds_remap");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let trait_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(1),
    };
    let function_def = DefId::new(CrateId(0), LocalDefId(0));
    let trait_def = DefId::new(CrateId(0), LocalDefId(1));
    let generic = crate::types::GenericParamId {
        owner: function_def,
        index: 0,
    };
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products
        .identity_table
        .display_names
        .insert(trait_id, "dep::Bound".to_string());
    insert_interface_trait(
        &mut products,
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_def,
            name: "Bound".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let mut function = products.bodies.functions[&function_id].clone();
    function.generic_params = vec![GenericParamDecl::type_param(generic, "T")];
    function.params = vec![HirParam {
        name: "value".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Generic(generic),
        mutable: false,
        is_ref: false,
    }];
    function.generic_bounds.insert(
        generic,
        vec![crate::types::TraitBound {
            trait_id: trait_def,
            type_args: vec![Type::Generic(generic)],
        }],
    );
    insert_function_body(&mut products, function_id, function);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let interface_function = dep
        .metadata()
        .interface()
        .function_by_canonical_name("dep::answer")
        .expect("function interface should exist");
    let body_function = dep
        .body_providers()
        .generic_function(interface_function.id)
        .expect("generic function body should exist");
    let remapped_generic = crate::types::GenericParamId {
        owner: interface_function.id,
        index: 0,
    };
    let bounds = body_function
        .generic_bounds
        .get(&remapped_generic)
        .expect("generic bound key should be remapped to interface function ID");

    assert_eq!(bounds[0].trait_id.local, LocalDefId(1));
    assert_ne!(bounds[0].trait_id.crate_id, CrateId(0));
    assert_eq!(bounds[0].trait_id.crate_id, interface_function.id.crate_id);
    assert_eq!(bounds[0].type_args, vec![Type::Generic(remapped_generic)]);
}

#[test]
fn product_artifact_remaps_signature_generic_bounds() {
    let (base, _cleanup) = temp_test_dir("signature_generic_bounds_remap");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let trait_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(1),
    };
    let trait_def = DefId::new(CrateId(0), LocalDefId(1));
    let generic = crate::types::GenericParamId {
        owner: trait_def,
        index: 0,
    };
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products
        .identity_table
        .display_names
        .insert(trait_id, "dep::Bound".to_string());

    let mut signature_bounds = HashMap::new();
    signature_bounds.insert(
        generic,
        vec![crate::types::TraitBound {
            trait_id: trait_def,
            type_args: vec![Type::Generic(generic)],
        }],
    );
    insert_interface_trait(
        &mut products,
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_def,
            name: "Bound".to_string(),
            generic_params: generic_params(trait_def, &["T"]),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "value".to_string(),
                HirFunctionSig {
                    id: DefId::new(CrateId(0), LocalDefId(31)),
                    name: "value".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(generic, "T")],
                    params: vec![Type::Generic(generic)],
                    ret: Type::Generic(generic),
                    generic_bounds: signature_bounds.into(),
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        },
    );
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let loaded_trait = dep
        .metadata()
        .interface()
        .trait_by_canonical_name("dep::Bound")
        .expect("trait interface should exist");
    let signature = &loaded_trait.signatures["value"];
    let remapped_generic = crate::types::GenericParamId {
        owner: loaded_trait.id,
        index: 0,
    };
    let bounds = signature
        .generic_bounds
        .get(&remapped_generic)
        .expect("signature generic bound key should be remapped to trait ID");

    assert_eq!(bounds[0].trait_id, loaded_trait.id);
    assert_eq!(bounds[0].type_args, vec![Type::Generic(remapped_generic)]);
}

#[test]
fn load_product_artifact_remaps_child_locations_in_generic_body() {
    let (base, _cleanup) = temp_test_dir("generic_body_child_locations");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let struct_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(5),
    };
    let enum_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(6),
    };
    let struct_def = DefId::new(CrateId(0), LocalDefId(5));
    let enum_def = DefId::new(CrateId(0), LocalDefId(6));

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut function = products.bodies.functions[&function_id].clone();
    function.body.stmts = vec![
        HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::FieldAccess(
                Box::new(HirExpr {
                    kind: HirExprKindFor::Var("widget".to_string()),
                    ty: Type::Struct {
                        id: struct_def,
                        args: Vec::new(),
                    },
                    span: Span::test(),
                }),
                "value".to_string(),
                Some(HirFieldLocation {
                    owner: struct_def,
                    field_id: FieldId(0),
                    name: "value".to_string(),
                }),
            ),
            ty: Type::I64,
            span: Span::test(),
        }),
        HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::EnumVariant(
                "dep::Choice".to_string(),
                "Some".to_string(),
                Vec::new(),
                Some(HirVariantLocation {
                    owner: enum_def,
                    variant_id: VariantId(0),
                    name: "Some".to_string(),
                }),
            ),
            ty: Type::Enum {
                id: enum_def,
                args: Vec::new(),
            },
            span: Span::test(),
        }),
    ];
    products.bodies.functions.insert(function_id, function);
    products
        .identity_table
        .display_names
        .insert(struct_product_id, "dep::Widget".to_string());
    products
        .identity_table
        .display_names
        .insert(enum_product_id, "dep::Choice".to_string());
    insert_interface_struct(
        &mut products,
        struct_product_id,
        HirStruct {
            id: struct_def,
            name: "dep::Widget".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );
    insert_interface_enum(
        &mut products,
        enum_product_id,
        HirEnum {
            id: enum_def,
            name: "dep::Choice".to_string(),
            generic_params: Vec::new(),
            variants: vec![HirVariant {
                id: VariantId(0),
                name: "Some".to_string(),
                fields: HirVariantFields::Unit,
            }],
        },
    );
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let interface = dep.metadata().interface();
    let function_id = interface
        .function_by_canonical_name("dep::answer")
        .expect("function interface should exist")
        .id;
    let body = dep
        .body_providers()
        .generic_function(function_id)
        .expect("generic function body should exist");
    let struct_owner = interface
        .struct_by_canonical_name("dep::Widget")
        .expect("struct interface should exist")
        .id;
    let enum_owner = interface
        .enum_by_canonical_name("dep::Choice")
        .expect("enum interface should exist")
        .id;
    assert_ne!(struct_owner.crate_id, CrateId(0));
    assert_ne!(enum_owner.crate_id, CrateId(0));

    match &body.body.stmts[0] {
        HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::FieldAccess(_, _, Some(location)),
            ..
        }) => assert_eq!(location.owner, struct_owner),
        other => panic!("expected remapped field location, got {other:?}"),
    }
    match &body.body.stmts[1] {
        HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::EnumVariant(_, _, _, Some(location)),
            ..
        }) => assert_eq!(location.owner, enum_owner),
        other => panic!("expected remapped variant location, got {other:?}"),
    }
}

#[test]
fn product_artifact_remaps_nominal_type_ids_in_hir_types() {
    let (base, _cleanup) = temp_test_dir("nominal_type_ids");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let struct_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(5),
    };
    let enum_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(6),
    };
    let struct_def = DefId::new(CrateId(0), LocalDefId(5));
    let enum_def = DefId::new(CrateId(0), LocalDefId(6));

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut function = products.bodies.functions[&function_id].clone();
    function.params.push(crate::hir::HirParam {
        name: "widget".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Struct {
            id: struct_def,
            args: vec![Type::Enum {
                id: enum_def,
                args: Vec::new(),
            }],
        },
        mutable: false,
        is_ref: false,
    });
    function.body.stmts = vec![
        HirStmt::Let {
            name: "local".to_string(),
            local_id: crate::ids::HirLocalId(1),
            ty: Type::Struct {
                id: struct_def,
                args: vec![Type::Enum {
                    id: enum_def,
                    args: Vec::new(),
                }],
            },
            value: HirExpr {
                kind: HirExprKindFor::Var("widget".to_string()),
                ty: Type::Struct {
                    id: struct_def,
                    args: vec![Type::Enum {
                        id: enum_def,
                        args: Vec::new(),
                    }],
                },
                span: Span::test(),
            },
            mutable: false,
        },
        HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::Match {
                scrutinee: Box::new(HirExpr {
                    kind: HirExprKindFor::Var("choice".to_string()),
                    ty: Type::Enum {
                        id: enum_def,
                        args: Vec::new(),
                    },
                    span: Span::test(),
                }),
                arms: vec![HirMatchArm {
                    pattern: HirPattern::Or(vec![
                        HirPattern::Struct(
                            "dep::Widget".to_string(),
                            None,
                            vec![Type::Enum {
                                id: enum_def,
                                args: Vec::new(),
                            }],
                            vec![HirStructPatternField {
                                name: "field".to_string(),
                                field: None,
                                pattern: HirPattern::Struct(
                                    "dep::Nested".to_string(),
                                    None,
                                    vec![Type::Struct {
                                        id: struct_def,
                                        args: Vec::new(),
                                    }],
                                    Vec::new(),
                                ),
                            }],
                        ),
                        HirPattern::Enum(
                            "dep::Choice".to_string(),
                            "Some".to_string(),
                            None,
                            vec![HirPattern::Tuple(vec![HirPattern::Struct(
                                "dep::Widget".to_string(),
                                None,
                                vec![Type::Struct {
                                    id: struct_def,
                                    args: Vec::new(),
                                }],
                                Vec::new(),
                            )])],
                        ),
                    ]),
                    guard: None,
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKindFor::IntLiteral(1),
                            ty: Type::I64,
                            span: Span::test(),
                        })],
                        ty: Type::I64,
                    },
                }],
            },
            ty: Type::I64,
            span: Span::test(),
        }),
        HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::Lambda {
                params: vec![HirParam {
                    name: "captured_widget".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Struct {
                        id: struct_def,
                        args: Vec::new(),
                    },
                    mutable: false,
                    is_ref: false,
                }],
                body: HirBlock {
                    stmts: Vec::new(),
                    ty: Type::Enum {
                        id: enum_def,
                        args: Vec::new(),
                    },
                },
                captures: vec![HirClosureCapture {
                    name: "widget".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    kind: HirClosureCaptureKind::SharedBorrow,
                    mutable: false,
                    ty: Type::Struct {
                        id: struct_def,
                        args: vec![Type::Enum {
                            id: enum_def,
                            args: Vec::new(),
                        }],
                    },
                }],
            },
            ty: Type::function(
                vec![Type::Struct {
                    id: struct_def,
                    args: Vec::new(),
                }],
                Type::Enum {
                    id: enum_def,
                    args: Vec::new(),
                },
            ),
            span: Span::test(),
        }),
    ];
    function.ret_type = Type::Enum {
        id: enum_def,
        args: Vec::new(),
    };
    insert_function_body(&mut products, function_id, function);
    products
        .identity_table
        .display_names
        .insert(struct_product_id, "dep::Widget".to_string());
    products
        .identity_table
        .display_names
        .insert(enum_product_id, "dep::Choice".to_string());
    insert_interface_struct(
        &mut products,
        struct_product_id,
        HirStruct {
            id: struct_def,
            name: "dep::Widget".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        },
    );
    insert_interface_enum(
        &mut products,
        enum_product_id,
        HirEnum {
            id: enum_def,
            name: "dep::Choice".to_string(),
            generic_params: Vec::new(),
            variants: Vec::new(),
        },
    );
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let interface = dep.metadata().interface();
    let function_id = interface
        .function_by_canonical_name("dep::answer")
        .expect("function interface should exist")
        .id;
    let body = dep
        .body_providers()
        .generic_function(function_id)
        .expect("generic function body should exist");
    let struct_owner = interface
        .struct_by_canonical_name("dep::Widget")
        .expect("struct interface should exist")
        .id;
    let enum_owner = interface
        .enum_by_canonical_name("dep::Choice")
        .expect("enum interface should exist")
        .id;

    assert_eq!(
        body.params[0].ty,
        Type::Struct {
            id: struct_owner,
            args: vec![Type::Enum {
                id: enum_owner,
                args: Vec::new(),
            }],
        }
    );
    assert_eq!(
        body.ret_type,
        Type::Enum {
            id: enum_owner,
            args: Vec::new(),
        }
    );
    match &body.body.stmts[0] {
        HirStmt::Let { ty, .. } => assert_eq!(
            ty,
            &Type::Struct {
                id: struct_owner,
                args: vec![Type::Enum {
                    id: enum_owner,
                    args: Vec::new(),
                }],
            }
        ),
        other => panic!("expected remapped let type, got {other:?}"),
    }
    let HirStmt::Expr(HirExpr {
        kind: HirExprKindFor::Match { arms, .. },
        ..
    }) = &body.body.stmts[1]
    else {
        panic!("expected match expression");
    };
    let HirPattern::Or(or_patterns) = &arms[0].pattern else {
        panic!("expected or pattern");
    };
    let HirPattern::Struct(_, _, struct_args, struct_fields) = &or_patterns[0] else {
        panic!("expected struct pattern");
    };
    assert_eq!(
        struct_args,
        &vec![Type::Enum {
            id: enum_owner,
            args: Vec::new(),
        }]
    );
    let HirPattern::Struct(_, _, nested_args, _) = &struct_fields[0].pattern else {
        panic!("expected nested struct pattern");
    };
    assert_eq!(
        nested_args,
        &vec![Type::Struct {
            id: struct_owner,
            args: Vec::new(),
        }]
    );
    let HirPattern::Enum(_, _, _, enum_payloads) = &or_patterns[1] else {
        panic!("expected enum pattern");
    };
    let HirPattern::Tuple(tuple_patterns) = &enum_payloads[0] else {
        panic!("expected tuple pattern");
    };
    let HirPattern::Struct(_, _, enum_nested_args, _) = &tuple_patterns[0] else {
        panic!("expected nested enum payload pattern");
    };
    assert_eq!(
        enum_nested_args,
        &vec![Type::Struct {
            id: struct_owner,
            args: Vec::new(),
        }]
    );
    let HirStmt::Expr(HirExpr {
        kind:
            HirExprKindFor::Lambda {
                params,
                body,
                captures,
            },
        ty,
        ..
    }) = &body.body.stmts[2]
    else {
        panic!("expected lambda expression");
    };
    assert_eq!(
        params[0].ty,
        Type::Struct {
            id: struct_owner,
            args: Vec::new(),
        }
    );
    assert_eq!(
        captures[0].ty,
        Type::Struct {
            id: struct_owner,
            args: vec![Type::Enum {
                id: enum_owner,
                args: Vec::new(),
            }],
        }
    );
    assert_eq!(
        body.ty,
        Type::Enum {
            id: enum_owner,
            args: Vec::new(),
        }
    );
    assert_eq!(
        *ty,
        Type::function(
            vec![Type::Struct {
                id: struct_owner,
                args: Vec::new(),
            }],
            Type::Enum {
                id: enum_owner,
                args: Vec::new(),
            },
        )
    );
}

#[test]
fn load_product_artifact_rejects_unknown_nominal_type_id() {
    let (base, _cleanup) = temp_test_dir("unknown_nominal_type_id");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut function = products.bodies.functions[&function_id].clone();
    function.params.push(HirParam {
        name: "ghost".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(999)),
            args: Vec::new(),
        },
        mutable: false,
        is_ref: false,
    });
    insert_function_body(&mut products, function_id, function);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("unknown nominal type ID should be rejected during artifact load");

    assert!(
        err.contains("unknown nominal type definition"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_invalid_projection_type_ids() {
    let (base, _cleanup) = temp_test_dir("invalid_projection_type_ids");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let trait_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(4),
    };
    let trait_id = DefId::new(CrateId(0), LocalDefId(4));
    insert_interface_trait(
        &mut products,
        trait_product_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "dep::Deref".to_string(),
            generic_params: Vec::new(),
            associated_types: vec![HirAssociatedTypeDecl {
                id: AssocTypeId(0),
                name: "Target".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut function = products.bodies.functions[&function_id].clone();
    function.params.push(HirParam {
        name: "bad".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Projection {
            ty: Box::new(Type::I64),
            trait_id,
            assoc_type: AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: AssocTypeId(99),
            },
            trait_args: Vec::new(),
        },
        mutable: false,
        is_ref: false,
    });
    insert_function_body(&mut products, function_id, function);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .unwrap_err();

    assert!(
        err.contains("unknown associated type ID"),
        "unexpected error: {err}"
    );
}

fn product_with_projection_trait(
    assoc_type_id: AssocTypeId,
) -> (
    CompilerProducts,
    super::ProductIdentityRemap,
    super::ProductNominalTypeValidator,
) {
    let trait_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(4),
    };
    let trait_def_id = DefId::new(CrateId(0), LocalDefId(4));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_trait(
        &mut products,
        trait_product_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_def_id,
            name: "dep::Deref".to_string(),
            generic_params: Vec::new(),
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_type_id,
                name: "Target".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    (products, remap, validator)
}

#[test]
fn product_artifact_remaps_projection_type_ids() {
    let (_products, remap, validator) = product_with_projection_trait(AssocTypeId(0));
    let mut ty = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id: DefId::new(CrateId(0), LocalDefId(4)),
        assoc_type: AssociatedTypeKey {
            owner: DefId::new(CrateId(0), LocalDefId(4)),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };

    super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap();

    assert_eq!(
        ty,
        Type::Projection {
            ty: Box::new(Type::I64),
            trait_id: DefId::new(CrateId(7), LocalDefId(4)),
            assoc_type: AssociatedTypeKey {
                owner: DefId::new(CrateId(7), LocalDefId(4)),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: Vec::new(),
        }
    );
}

#[test]
fn product_artifact_rejects_unknown_projection_trait_id() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    products.interface.traits.clear();
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut ty = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id: DefId::new(CrateId(0), LocalDefId(4)),
        assoc_type: AssociatedTypeKey {
            owner: DefId::new(CrateId(0), LocalDefId(4)),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };

    let err = super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap_err();

    assert!(
        err.contains("unknown trait definition"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_error_type_in_executable_body() {
    let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut expr = HirExpr {
        kind: HirExprKindFor::IntLiteral(1),
        ty: Type::Error,
        span: Span::test(),
    };

    let error =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        error.contains("unresolved type"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_error_type_in_impl_receiver_metadata() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let impl_id = product_def_id(0, 7);
    insert_interface_impl(
        &mut products,
        impl_id,
        HirImpl {
            id: product_hir_def_id(impl_id),
            owner: HirImplOwner::Named("Broken".to_string()),
            type_name: "Broken".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Error),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        },
    );

    let error = super::validate_product_method_authority_maps(
        &products,
        &super::ProductDependencyDefinitions::default(),
    )
    .unwrap_err();

    assert!(
        error.contains("unresolved type"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_unknown_projection_assoc_type_id() {
    let (_products, remap, validator) = product_with_projection_trait(AssocTypeId(0));
    let mut ty = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id: DefId::new(CrateId(0), LocalDefId(4)),
        assoc_type: AssociatedTypeKey {
            owner: DefId::new(CrateId(0), LocalDefId(4)),
            assoc_type_id: AssocTypeId(9),
        },
        trait_args: Vec::new(),
    };

    let err = super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap_err();

    assert!(
        err.contains("unknown associated type ID"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_projection_assoc_owner_mismatch_across_crates() {
    let (_products, mut remap, validator) = product_with_projection_trait(AssocTypeId(0));
    remap.crate_ids.insert(ProductCrateId(1), CrateId(8));
    let mut ty = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id: DefId::new(CrateId(0), LocalDefId(4)),
        assoc_type: AssociatedTypeKey {
            owner: DefId::new(CrateId(1), LocalDefId(4)),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };

    let err = super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap_err();

    assert!(
        err.contains("does not match associated type owner"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_unknown_dependency_projection_trait_id() {
    let (_products, mut remap, validator) = product_with_projection_trait(AssocTypeId(0));
    remap.crate_ids.insert(ProductCrateId(1), CrateId(8));
    let mut ty = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id: DefId::new(CrateId(1), LocalDefId(44)),
        assoc_type: AssociatedTypeKey {
            owner: DefId::new(CrateId(1), LocalDefId(44)),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };

    let err = super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap_err();

    assert!(
        err.contains("unknown dependency trait definition"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_unknown_method_call_target_impl_id() {
    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            None,
            HirMethodCallTarget::impl_method(
                DefId::new(CrateId(0), LocalDefId(99)),
                DefId::new(CrateId(0), LocalDefId(0)),
                None,
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        err.contains("unknown impl target"),
        "unexpected error: {err}"
    );
}

fn validate_artifact_expr(products: &CompilerProducts, expr: &mut HirExpr) -> Result<(), String> {
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(products);
    let type_validator = super::ProductNominalTypeValidator::from_products(products, &remap);
    super::remap_expr_child_locations(expr, &remap, &child_validator, &type_validator, 0)
}

fn trait_method_fixture(
    generic_params: Vec<GenericParamDecl>,
    self_receiver: Option<crate::types::ReceiverMode>,
) -> (CompilerProducts, DefId, DefId) {
    let trait_product_id = product_def_id(0, 30);
    let trait_id = product_hir_def_id(trait_product_id);
    let member_id = DefId::new(CrateId(0), LocalDefId(31));
    let mut member = test_function(member_id, "value");
    member.is_method = self_receiver.is_some();
    member.self_receiver = self_receiver;
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_trait(
        &mut products,
        trait_product_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "dep::Trait".to_string(),
            generic_params,
            associated_types: Vec::new(),
            methods: HashMap::from([("value".to_string(), member)]),
            signatures: HashMap::new(),
        },
    );
    (products, trait_id, member_id)
}

#[test]
fn product_artifact_load_rejects_receiver_trait_member_mapped_to_static_impl_method() {
    let (mut products, trait_id, member_id) =
        trait_method_fixture(Vec::new(), Some(crate::types::ReceiverMode::Move));
    let impl_product_id = product_def_id(0, 40);
    let impl_id = product_hir_def_id(impl_product_id);
    let method_id = DefId::new(CrateId(0), LocalDefId(41));
    insert_interface_impl(
        &mut products,
        impl_product_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
            trait_name: Some("Trait".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([(
                "different_source_name".to_string(),
                test_function(method_id, "different_source_name"),
            )]),
        },
    );
    products.interface.effective_trait_methods.insert(
        (impl_product_id, ProductDefId::from(member_id)),
        ProductDefId::from(method_id),
    );
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };

    let error =
        super::cross_crate_hir_from_products(&products, "dep", &remap, &CrateContext::new())
            .expect_err("malformed effective trait-member relation must fail artifact loading");

    assert!(
        error.contains("effective trait member")
            && error.contains(&format!("{member_id:?}"))
            && error.contains(&format!("{method_id:?}"))
            && error.contains("receiver/static mismatch"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_trait_method_target_with_missing_trait_argument() {
    let (products, trait_id, member_id) = trait_method_fixture(
        generic_params(DefId::new(CrateId(0), LocalDefId(30)), &["T"]),
        Some(crate::types::ReceiverMode::Move),
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            Some(crate::types::ReceiverMode::Move),
            HirMethodCallTarget::trait_method(
                trait_id,
                member_id,
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

    assert!(
        error.contains("trait argument arity"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_impl_target_with_inconsistent_trait_arguments() {
    let (mut products, trait_id, member_id) = trait_method_fixture(
        generic_params(DefId::new(CrateId(0), LocalDefId(30)), &["T"]),
        Some(crate::types::ReceiverMode::Move),
    );
    let impl_product_id = product_def_id(0, 40);
    let impl_id = product_hir_def_id(impl_product_id);
    let method_id = DefId::new(CrateId(0), LocalDefId(41));
    let mut method = test_function(method_id, "value");
    method.is_method = true;
    method.self_receiver = Some(crate::types::ReceiverMode::Move);
    insert_interface_impl(
        &mut products,
        impl_product_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
            trait_name: Some("Trait".to_string()),
            trait_id: Some(trait_id),
            trait_generics: generic_params(trait_id, &["T"]),
            trait_arg_types: vec![Type::I64],
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        },
    );
    products.interface.effective_trait_methods.insert(
        (impl_product_id, ProductDefId::from(member_id)),
        ProductDefId::from(method_id),
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            Some(crate::types::ReceiverMode::Move),
            HirMethodCallTarget::impl_method(
                impl_id,
                method_id,
                Some(crate::hir::HirSelectedTraitMember {
                    trait_id,
                    member_id,
                    trait_args: vec![Type::Bool],
                }),
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

    assert!(
        error.contains("trait arguments do not match impl"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_trait_impl_target_without_selected_trait_identity() {
    let (mut products, trait_id, member_id) =
        trait_method_fixture(Vec::new(), Some(crate::types::ReceiverMode::Move));
    let impl_product_id = product_def_id(0, 40);
    let impl_id = product_hir_def_id(impl_product_id);
    let method_id = DefId::new(CrateId(0), LocalDefId(41));
    let mut method = test_function(method_id, "value");
    method.is_method = true;
    method.self_receiver = Some(crate::types::ReceiverMode::Move);
    insert_interface_impl(
        &mut products,
        impl_product_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
            trait_name: Some("Trait".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        },
    );
    products.interface.effective_trait_methods.insert(
        (impl_product_id, ProductDefId::from(member_id)),
        ProductDefId::from(method_id),
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            Some(crate::types::ReceiverMode::Move),
            HirMethodCallTarget::impl_method(impl_id, method_id, None),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

    assert!(
        error.contains("has no selected trait identity"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_method_call_authority_for_static_method() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let impl_product_id = product_def_id(0, 40);
    let impl_id = product_hir_def_id(impl_product_id);
    let method_id = DefId::new(CrateId(0), LocalDefId(41));
    insert_interface_impl(
        &mut products,
        impl_product_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("make".to_string(), test_function(method_id, "make"))]),
        },
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "make".to_string(),
            Vec::new(),
            None,
            HirMethodCallTarget::impl_method(impl_id, method_id, None),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

    assert!(
        error.contains("references a static method"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_static_authority_for_receiver_method() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let impl_product_id = product_def_id(0, 40);
    let impl_id = product_hir_def_id(impl_product_id);
    let method_id = DefId::new(CrateId(0), LocalDefId(41));
    let mut method = test_function(method_id, "value");
    method.is_method = true;
    method.self_receiver = Some(crate::types::ReceiverMode::Move);
    insert_interface_impl(
        &mut products,
        impl_product_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        },
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::Call(
            Box::new(HirExpr {
                kind: HirExprKindFor::Var("value".to_string()),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            }),
            Vec::new(),
            Some(crate::hir::HirCallTarget::StaticMethod(
                crate::hir::HirStaticMethodTarget {
                    owner_ty: Type::I64,
                    method: HirMethodCallTarget::impl_method(impl_id, method_id, None),
                },
            )),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

    assert!(
        error.contains("references a receiver method"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_malformed_trait_method_authority_with_substitutions() {
    let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let mut target = HirMethodCallTarget::trait_method(
        DefId::new(CrateId(0), LocalDefId(999)),
        DefId::new(CrateId(0), LocalDefId(998)),
        Vec::new(),
        crate::hir::HirTraitDispatchKind::TraitBound,
    );
    target.owner_substitution.push(crate::hir::HirTypeBinding {
        param: crate::types::GenericParamId {
            owner: DefId::new(CrateId(0), LocalDefId(1)),
            index: 0,
        },
        ty: Type::I64,
    });
    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::ArrayLiteral(Vec::new()),
                ty: Type::Array(Box::new(Type::I64), 0),
                span: Span::test(),
            }),
            "index".to_string(),
            vec![HirExpr {
                kind: HirExprKindFor::IntLiteral(0),
                ty: Type::I64,
                span: Span::test(),
            }],
            None,
            target,
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

    assert!(
        error.contains("references unknown trait target"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_method_call_target_method_from_other_impl() {
    let first_impl_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(10),
    };
    let second_impl_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(20),
    };
    let first_method_id = DefId::new(CrateId(0), LocalDefId(11));
    let second_method_id = DefId::new(CrateId(0), LocalDefId(21));
    let mut first_method = test_function(first_method_id, "dep::Box_First_value");
    first_method.is_method = true;
    let mut second_method = test_function(second_method_id, "dep::Box_Second_value");
    second_method.is_method = true;

    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_impl(
        &mut products,
        first_impl_id,
        HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(10)),
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), first_method)]),
        },
    );
    insert_interface_impl(
        &mut products,
        second_impl_id,
        HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(20)),
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), second_method)]),
        },
    );

    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            None,
            HirMethodCallTarget::impl_method(
                DefId::new(CrateId(0), LocalDefId(10)),
                second_method_id,
                None,
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        err.contains("method target") && err.contains("impl target"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_trait_target_on_inherent_impl_method_call() {
    let trait_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(30),
    };
    let impl_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(10),
    };
    let method_id = DefId::new(CrateId(0), LocalDefId(11));
    let mut method = test_function(method_id, "dep::Box_value");
    method.is_method = true;

    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_trait(
        &mut products,
        trait_product_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: DefId::new(CrateId(0), LocalDefId(30)),
            name: "dep::Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    insert_interface_impl(
        &mut products,
        impl_product_id,
        HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(10)),
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        },
    );

    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            None,
            HirMethodCallTarget::impl_method(
                DefId::new(CrateId(0), LocalDefId(10)),
                method_id,
                Some(crate::hir::HirSelectedTraitMember {
                    trait_id: DefId::new(CrateId(0), LocalDefId(30)),
                    member_id: method_id,
                    trait_args: Vec::new(),
                }),
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        err.contains("impl target") && err.contains("trait target"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_method_call_target_method_from_other_trait() {
    let first_trait_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(30),
    };
    let second_trait_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(40),
    };
    let first_method_id = DefId::new(CrateId(0), LocalDefId(31));
    let second_method_id = DefId::new(CrateId(0), LocalDefId(41));
    let mut first_method = test_function(first_method_id, "dep::First_value");
    first_method.is_method = true;
    let mut second_method = test_function(second_method_id, "dep::Second_value");
    second_method.is_method = true;

    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_trait(
        &mut products,
        first_trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: DefId::new(CrateId(0), LocalDefId(30)),
            name: "dep::First".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("value".to_string(), first_method)]),
            signatures: HashMap::new(),
        },
    );
    insert_interface_trait(
        &mut products,
        second_trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: DefId::new(CrateId(0), LocalDefId(40)),
            name: "dep::Second".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("value".to_string(), second_method)]),
            signatures: HashMap::new(),
        },
    );

    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            None,
            HirMethodCallTarget::trait_method(
                DefId::new(CrateId(0), LocalDefId(30)),
                second_method_id,
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        err.contains("does not own selected member"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_trait_signature_target_using_trait_id_sentinel() {
    let trait_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(30),
    };
    let trait_id = DefId::new(CrateId(0), LocalDefId(30));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_trait(
        &mut products,
        trait_product_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "dep::Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "value".to_string(),
                HirFunctionSig {
                    id: DefId::new(CrateId(0), LocalDefId(31)),
                    name: "value".to_string(),
                    generic_params: Vec::new(),
                    params: Vec::new(),
                    ret: Type::I64,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        },
    );

    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            None,
            HirMethodCallTarget::trait_method(
                trait_id,
                trait_id,
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        err.contains("does not own selected member"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_unknown_resolved_var_function_target() {
    let mut expr = HirExpr {
        kind: HirExprKindFor::ResolvedVar(HirVarRef {
            name: "missing".to_string(),
            target: HirVarTarget::Function(DefId::new(CrateId(0), LocalDefId(99))),
        }),
        ty: Type::I64,
        span: Span::test(),
    };
    let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(err.contains("unknown function"), "unexpected error: {err}");
}

#[test]
fn product_artifact_rejects_resolved_var_function_target_to_extern() {
    let extern_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(7),
    };
    let extern_def = DefId::new(CrateId(0), LocalDefId(7));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_extern(
        &mut products,
        extern_id,
        HirExtern {
            id: extern_def,
            name: "dep::foreign".to_string(),
            params: Vec::new(),
            ret: Type::I64,
            variadic: false,
            is_unsafe: false,
        },
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::ResolvedVar(HirVarRef {
            name: "foreign".to_string(),
            target: HirVarTarget::Function(extern_def),
        }),
        ty: Type::I64,
        span: Span::test(),
    };
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        err.contains("function") && err.contains("extern"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_resolved_var_extern_target_to_function() {
    let function_def = DefId::new(CrateId(0), LocalDefId(0));
    let mut expr = HirExpr {
        kind: HirExprKindFor::ResolvedVar(HirVarRef {
            name: "answer".to_string(),
            target: HirVarTarget::Extern(function_def),
        }),
        ty: Type::I64,
        span: Span::test(),
    };
    let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        err.contains("extern") && err.contains("function"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_unknown_struct_literal_sidecar_without_fields() {
    let missing_struct = DefId::new(CrateId(0), LocalDefId(99));
    let mut expr = HirExpr {
        kind: HirExprKindFor::StructLiteral("Ghost".to_string(), Some(missing_struct), Vec::new()),
        ty: Type::I64,
        span: Span::test(),
    };
    let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        err.contains("unknown nominal type definition") && err.contains("as struct"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_unknown_struct_pattern_sidecar_without_fields() {
    let missing_struct = DefId::new(CrateId(0), LocalDefId(99));
    let mut pattern = HirPattern::Struct(
        "Ghost".to_string(),
        Some(missing_struct),
        Vec::new(),
        Vec::new(),
    );
    let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err = super::remap_pattern_child_locations(
        &mut pattern,
        &remap,
        &child_validator,
        &type_validator,
        0,
    )
    .unwrap_err();

    assert!(
        err.contains("unknown nominal type definition") && err.contains("as struct"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_struct_literal_field_owner_mismatch() {
    let first_struct_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(5),
    };
    let second_struct_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(6),
    };
    let first_struct_def = DefId::new(CrateId(0), LocalDefId(5));
    let second_struct_def = DefId::new(CrateId(0), LocalDefId(6));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_struct(
        &mut products,
        first_struct_id,
        HirStruct {
            id: first_struct_def,
            name: "dep::First".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );
    insert_interface_struct(
        &mut products,
        second_struct_id,
        HirStruct {
            id: second_struct_def,
            name: "dep::Second".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::StructLiteral(
            "First".to_string(),
            Some(first_struct_def),
            vec![crate::hir::HirStructLiteralFieldFor::<AcceptedHir> {
                name: "value".to_string(),
                value: HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                },
                field: Some(HirFieldLocation {
                    owner: second_struct_def,
                    field_id: FieldId(0),
                    name: "value".to_string(),
                }),
            }],
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        err.contains("struct literal field owner mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_struct_pattern_field_owner_mismatch() {
    let first_struct_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(5),
    };
    let second_struct_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(6),
    };
    let first_struct_def = DefId::new(CrateId(0), LocalDefId(5));
    let second_struct_def = DefId::new(CrateId(0), LocalDefId(6));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_struct(
        &mut products,
        first_struct_id,
        HirStruct {
            id: first_struct_def,
            name: "dep::First".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );
    insert_interface_struct(
        &mut products,
        second_struct_id,
        HirStruct {
            id: second_struct_def,
            name: "dep::Second".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );
    let mut pattern = HirPattern::Struct(
        "First".to_string(),
        Some(first_struct_def),
        Vec::new(),
        vec![HirStructPatternField {
            name: "value".to_string(),
            field: Some(HirFieldLocation {
                owner: second_struct_def,
                field_id: FieldId(0),
                name: "value".to_string(),
            }),
            pattern: HirPattern::Wildcard,
        }],
    );
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err = super::remap_pattern_child_locations(
        &mut pattern,
        &remap,
        &child_validator,
        &type_validator,
        0,
    )
    .unwrap_err();

    assert!(
        err.contains("struct pattern field owner mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_accepts_dependency_resolved_var_function_target() {
    let (base, _cleanup) = temp_test_dir("dependency_resolved_var_function_target");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let app_function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    app_products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("dep".to_string()),
    );
    let mut app_function = app_products.bodies.functions[&app_function_id].clone();
    app_function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "dep::answer".to_string(),
                target: HirVarTarget::Function(DefId::new(CrateId(1), LocalDefId(0))),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    ctx.load_product_artifact_from_path(app_artifact)
        .expect("dependency function ResolvedVar targets should validate");
}

#[test]
fn load_product_artifact_rejects_dependency_resolved_var_function_target_to_extern() {
    let (base, _cleanup) = temp_test_dir("dependency_resolved_var_wrong_kind");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_extern_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(7),
    };
    let dep_extern_def = DefId::new(CrateId(0), LocalDefId(7));
    let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    insert_interface_extern(
        &mut dep_products,
        dep_extern_id,
        HirExtern {
            id: dep_extern_def,
            name: "dep::foreign".to_string(),
            params: Vec::new(),
            ret: Type::I64,
            variadic: false,
            is_unsafe: false,
        },
    );
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let app_function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    app_products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("dep".to_string()),
    );
    let mut app_function = app_products.bodies.functions[&app_function_id].clone();
    app_function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "dep::foreign".to_string(),
                target: HirVarTarget::Function(DefId::new(CrateId(1), LocalDefId(7))),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    let err = ctx
        .load_product_artifact_from_path(app_artifact)
        .expect_err("dependency extern target used as function should be rejected");

    assert!(
        err.contains("dependency extern") && err.contains("as function"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_accepts_dependency_resolved_var_extern_target() {
    let (base, _cleanup) = temp_test_dir("dependency_resolved_var_extern_target");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_extern_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(7),
    };
    let dep_extern_def = DefId::new(CrateId(0), LocalDefId(7));
    let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    insert_interface_extern(
        &mut dep_products,
        dep_extern_id,
        HirExtern {
            id: dep_extern_def,
            name: "dep::foreign".to_string(),
            params: Vec::new(),
            ret: Type::I64,
            variadic: false,
            is_unsafe: false,
        },
    );
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let app_function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    app_products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("dep".to_string()),
    );
    let mut app_function = app_products.bodies.functions[&app_function_id].clone();
    app_function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "dep::foreign".to_string(),
                target: HirVarTarget::Extern(DefId::new(CrateId(1), LocalDefId(7))),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    ctx.load_product_artifact_from_path(app_artifact)
        .expect("dependency extern ResolvedVar targets should validate");
}

#[test]
fn load_product_artifact_rejects_dependency_resolved_var_extern_target_to_function() {
    let (base, _cleanup) = temp_test_dir("dependency_resolved_var_extern_wrong_kind");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let app_function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    app_products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("dep".to_string()),
    );
    let mut app_function = app_products.bodies.functions[&app_function_id].clone();
    app_function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "dep::answer".to_string(),
                target: HirVarTarget::Extern(DefId::new(CrateId(1), LocalDefId(0))),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    let err = ctx
        .load_product_artifact_from_path(app_artifact)
        .expect_err("dependency function target used as extern should be rejected");

    assert!(
        err.contains("dependency function") && err.contains("as extern"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_accepts_resolved_var_function_display_name_mismatch() {
    let (base, _cleanup) = temp_test_dir("resolved_var_function_name_mismatch");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let mut function = products.bodies.functions[&function_id].clone();
    function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "not_answer".to_string(),
                target: HirVarTarget::Function(DefId::new(CrateId(0), LocalDefId(0))),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    products.bodies.functions.insert(function_id, function);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();
}

#[test]
fn product_artifact_rejects_receiver_method_as_function_target() {
    let impl_id = product_def_id(0, 10);
    let method_id = product_def_id(0, 11);
    let method_def = product_hir_def_id(method_id);
    let mut method = test_function(method_def, "value");
    method.is_method = true;
    method.params.push(HirParam {
        name: "self".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::I64,
        mutable: false,
        is_ref: false,
    });
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_impl(
        &mut products,
        impl_id,
        HirImpl {
            id: product_hir_def_id(impl_id),
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        },
    );
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut reference = HirVarRef {
        name: "Box_value".to_string(),
        target: HirVarTarget::Function(method_def),
    };

    let err = super::remap_var_target_child_location(
        &mut reference,
        &remap,
        &validator,
        &BTreeSet::new(),
    )
    .unwrap_err();

    assert!(
        err.contains("receiver method") && err.contains("as function"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_accepts_static_method_function_display_name_mismatch() {
    let impl_id = product_def_id(0, 12);
    let method_id = product_def_id(0, 13);
    let method_def = product_hir_def_id(method_id);
    let mut method = test_function(method_def, "make");
    method.is_method = false;
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_impl(
        &mut products,
        impl_id,
        HirImpl {
            id: product_hir_def_id(impl_id),
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("make".to_string(), method)]),
        },
    );
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut reference = HirVarRef {
        name: "not_make".to_string(),
        target: HirVarTarget::Function(method_def),
    };

    super::remap_var_target_child_location(&mut reference, &remap, &validator, &BTreeSet::new())
        .unwrap();
}

#[test]
fn product_artifact_rejects_dependency_receiver_method_as_function_target() {
    let app_products = product_with_function("app", ProductCrateId(0), 0, PathBuf::from("app.o"));
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([
            (ProductCrateId(0), CrateId(7)),
            (ProductCrateId(1), CrateId(8)),
        ]),
    };
    let method_id = product_def_id(1, 11);
    let mut dependencies = super::ProductDependencyDefinitions::default();
    dependencies.methods.insert(method_id);
    let validator = super::ProductNominalTypeValidator::from_products_with_dependencies(
        &app_products,
        &remap,
        dependencies,
    );
    let mut reference = HirVarRef {
        name: "dep::Box_value".to_string(),
        target: HirVarTarget::Function(product_hir_def_id(method_id)),
    };

    let err = super::remap_var_target_child_location(
        &mut reference,
        &remap,
        &validator,
        &BTreeSet::new(),
    )
    .unwrap_err();

    assert!(
        err.contains("dependency receiver method") && err.contains("as function"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_allows_dependency_static_impl_method_canonical_name_as_function_target() {
    let app_products = product_with_function("app", ProductCrateId(0), 0, PathBuf::from("app.o"));
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([
            (ProductCrateId(0), CrateId(7)),
            (ProductCrateId(1), CrateId(8)),
        ]),
    };

    let dep_impl_id = DefId::new(CrateId(8), LocalDefId(10));
    let dep_method_id = DefId::new(CrateId(8), LocalDefId(11));
    let dep_method_product_id = product_def_id(1, 11);
    let dep_method = test_function(dep_method_id, "make");
    let dep_impl = HirImpl {
        id: dep_impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("make".to_string(), dep_method)]),
    };
    let mut interface = ArtifactCrateInterface::default();
    interface
        .canonical_names
        .insert(dep_method_id, "dep::Box::make".to_string());
    interface
        .impls
        .insert(dep_impl_id, ProductImplInterface::from(&dep_impl));
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(ExternCrateRecord::new(
        CrateId(8),
        "dep".to_string(),
        ExternCrateMetadata::new(
            interface,
            crate::collect::resolver::ResolverTables::default(),
            BTreeMap::new(),
        ),
        ExternCrateBodies::default(),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    ))
    .unwrap();

    let validator =
        super::ProductNominalTypeValidator::from_products_with_context(&app_products, &remap, &ctx);
    let mut reference = HirVarRef {
        name: "dep::Box::make".to_string(),
        target: HirVarTarget::Function(product_hir_def_id(dep_method_product_id)),
    };

    super::remap_var_target_child_location(&mut reference, &remap, &validator, &BTreeSet::new())
        .unwrap();

    assert_eq!(
        reference.target,
        HirVarTarget::Function(DefId::new(CrateId(8), LocalDefId(11)))
    );
}

#[test]
fn product_dependency_definitions_ignore_body_provider_generic_impls() {
    let app_products = product_with_function("app", ProductCrateId(0), 0, PathBuf::from("app.o"));
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([
            (ProductCrateId(0), CrateId(7)),
            (ProductCrateId(1), CrateId(8)),
        ]),
    };

    let dep_impl_id = DefId::new(CrateId(8), LocalDefId(10));
    let dep_method_id = DefId::new(CrateId(8), LocalDefId(11));
    let mut dep_method = test_function(dep_method_id, "value");
    dep_method.is_method = true;
    let dep_impl = HirImpl {
        id: dep_impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: generic_params(dep_impl_id, &["T"]),
        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Generic(
            crate::types::GenericParamId {
                owner: dep_impl_id,
                index: 0,
            },
        )),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("value".to_string(), dep_method)]),
    };
    let bodies = ExternCrateBodies::from_cross_crate_hir(ArtifactCrossCrateHir {
        generic_functions: BTreeMap::new(),
        traits_with_defaults: BTreeMap::new(),
        generic_impls: vec![dep_impl],
    });
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(ExternCrateRecord::new(
        CrateId(8),
        "dep".to_string(),
        ExternCrateMetadata::new(
            ArtifactCrateInterface::default(),
            crate::collect::resolver::ResolverTables::default(),
            BTreeMap::new(),
        ),
        bodies,
        ExternCrateLink::metadata_only(BTreeMap::new()),
    ))
    .unwrap();

    let validator = super::ProductChildLocationValidator::from_products_with_context(
        &app_products,
        &remap,
        &ctx,
    );
    let type_validator =
        super::ProductNominalTypeValidator::from_products_with_context(&app_products, &remap, &ctx);
    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            None,
            HirMethodCallTarget::impl_method(
                DefId::new(CrateId(1), LocalDefId(10)),
                DefId::new(CrateId(1), LocalDefId(11)),
                None,
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let err = super::remap_expr_child_locations(&mut expr, &remap, &validator, &type_validator, 0)
        .expect_err("dependency body providers must not authorize method targets");

    assert!(
        err.contains("unknown dependency impl target"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_accepts_resolved_var_extern_display_name_mismatch() {
    let (base, _cleanup) = temp_test_dir("resolved_var_extern_name_mismatch");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let extern_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(7),
    };
    let extern_def = DefId::new(CrateId(0), LocalDefId(7));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    insert_interface_extern(
        &mut products,
        extern_id,
        HirExtern {
            id: extern_def,
            name: "dep::foreign".to_string(),
            params: Vec::new(),
            ret: Type::I64,
            variadic: false,
            is_unsafe: false,
        },
    );
    let mut function = products.bodies.functions[&function_id].clone();
    function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "answer".to_string(),
                target: HirVarTarget::Extern(extern_def),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    products.bodies.functions.insert(function_id, function);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();
}

#[test]
fn load_product_artifact_accepts_dependency_resolved_var_function_display_name_mismatch() {
    let (base, _cleanup) = temp_test_dir("dependency_resolved_var_function_name_mismatch");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let app_function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    app_products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("dep".to_string()),
    );
    let mut app_function = app_products.bodies.functions[&app_function_id].clone();
    app_function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "app::answer".to_string(),
                target: HirVarTarget::Function(DefId::new(CrateId(1), LocalDefId(0))),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    ctx.load_product_artifact_from_path(app_artifact).unwrap();
}

#[test]
fn product_artifact_accepts_ambiguous_display_name_with_exact_function_target() {
    let left_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(10),
    };
    let right_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(11),
    };
    let left_def = DefId::new(CrateId(0), LocalDefId(10));
    let right_def = DefId::new(CrateId(0), LocalDefId(11));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    products
        .identity_table
        .display_names
        .insert(left_id, "left::value".to_string());
    products
        .identity_table
        .display_names
        .insert(right_id, "right::value".to_string());
    insert_interface_function(&mut products, left_id, &test_function(left_def, "value"));
    insert_interface_function(&mut products, right_id, &test_function(right_def, "value"));
    let mut expr = HirExpr {
        kind: HirExprKindFor::ResolvedVar(HirVarRef {
            name: "value".to_string(),
            target: HirVarTarget::Function(right_def),
        }),
        ty: Type::function(Vec::new(), Type::I64),
        span: Span::test(),
    };
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
        .unwrap();
}

#[test]
fn load_product_artifact_accepts_dependency_generic_impl_method_target() {
    let (base, _cleanup) = temp_test_dir("dependency_generic_impl_method_target");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_impl_id = DefId::new(CrateId(0), LocalDefId(10));
    let dep_method_id = DefId::new(CrateId(0), LocalDefId(11));
    let mut dep_method = test_function(dep_method_id, "value");
    dep_method.is_method = true;
    dep_method.self_receiver = Some(crate::types::ReceiverMode::Move);
    dep_method.params.push(crate::hir::HirParam {
        name: "self".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Generic(crate::types::GenericParamId {
            owner: dep_impl_id,
            index: 0,
        }),
        mutable: false,
        is_ref: false,
    });
    let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    let dep_impl_product_id = ProductDefId::from(dep_impl_id);
    let dep_impl = HirImpl {
        id: dep_impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: generic_params(dep_impl_id, &["T"]),
        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Generic(
            crate::types::GenericParamId {
                owner: dep_impl_id,
                index: 0,
            },
        )),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("value".to_string(), dep_method)]),
    };
    insert_interface_impl(&mut dep_products, dep_impl_product_id, dep_impl.clone());
    dep_products
        .bodies
        .generic_impls
        .insert(dep_impl_product_id, dep_impl);
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let app_function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    app_products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("dep".to_string()),
    );
    let mut app_function = app_products.bodies.functions[&app_function_id].clone();
    app_function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                "value".to_string(),
                Vec::new(),
                Some(crate::types::ReceiverMode::Move),
                HirMethodCallTarget::impl_method(
                    DefId::new(CrateId(1), LocalDefId(10)),
                    DefId::new(CrateId(1), LocalDefId(11)),
                    None,
                ),
            ),
            ty: Type::I64,
            span: Span::test(),
        })],
        ty: Type::I64,
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    ctx.load_product_artifact_from_path(app_artifact)
        .expect("dependency generic impl method targets should validate");
}

#[test]
fn load_product_artifact_rejects_dependency_field_sidecar_mismatch() {
    let (base, _cleanup) = temp_test_dir("dependency_field_sidecar_mismatch");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_struct_id = DefId::new(CrateId(0), LocalDefId(5));
    let dep_struct_product_id = ProductDefId::from(dep_struct_id);
    let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    dep_products
        .identity_table
        .display_names
        .insert(dep_struct_product_id, "dep::Widget".to_string());
    insert_interface_struct(
        &mut dep_products,
        dep_struct_product_id,
        HirStruct {
            id: dep_struct_id,
            name: "dep::Widget".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let app_function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let dep_struct_ref = DefId::new(CrateId(1), LocalDefId(5));
    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    app_products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("dep".to_string()),
    );
    let mut app_function = app_products.bodies.functions[&app_function_id].clone();
    app_function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::FieldAccess(
                Box::new(HirExpr {
                    kind: HirExprKindFor::Var("widget".to_string()),
                    ty: Type::Struct {
                        id: dep_struct_ref,
                        args: Vec::new(),
                    },
                    span: Span::test(),
                }),
                "value".to_string(),
                Some(HirFieldLocation {
                    owner: dep_struct_ref,
                    field_id: FieldId(0),
                    name: "other".to_string(),
                }),
            ),
            ty: Type::I64,
            span: Span::test(),
        })],
        ty: Type::I64,
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    let err = ctx
        .load_product_artifact_from_path(app_artifact)
        .expect_err("dependency field sidecar mismatch should be rejected");

    assert!(
        err.contains("field sidecar mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_dependency_variant_sidecar_mismatch() {
    let (base, _cleanup) = temp_test_dir("dependency_variant_sidecar_mismatch");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_enum_id = DefId::new(CrateId(0), LocalDefId(6));
    let dep_enum_product_id = ProductDefId::from(dep_enum_id);
    let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    dep_products
        .identity_table
        .display_names
        .insert(dep_enum_product_id, "dep::Choice".to_string());
    insert_interface_enum(
        &mut dep_products,
        dep_enum_product_id,
        HirEnum {
            id: dep_enum_id,
            name: "dep::Choice".to_string(),
            generic_params: Vec::new(),
            variants: vec![HirVariant {
                id: VariantId(0),
                name: "Some".to_string(),
                fields: HirVariantFields::Unit,
            }],
        },
    );
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let app_function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let dep_enum_ref = DefId::new(CrateId(1), LocalDefId(6));
    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    app_products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("dep".to_string()),
    );
    let mut app_function = app_products.bodies.functions[&app_function_id].clone();
    app_function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::EnumVariant(
                "Choice".to_string(),
                "Some".to_string(),
                Vec::new(),
                Some(HirVariantLocation {
                    owner: dep_enum_ref,
                    variant_id: VariantId(0),
                    name: "None".to_string(),
                }),
            ),
            ty: Type::Enum {
                id: dep_enum_ref,
                args: Vec::new(),
            },
            span: Span::test(),
        })],
        ty: Type::Enum {
            id: dep_enum_ref,
            args: Vec::new(),
        },
    };
    app_function.ret_type = Type::Enum {
        id: dep_enum_ref,
        args: Vec::new(),
    };
    insert_function_body(&mut app_products, app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    let err = ctx
        .load_product_artifact_from_path(app_artifact)
        .expect_err("dependency variant sidecar mismatch should be rejected");

    assert!(
        err.contains("variant sidecar mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_child_location_validator_rejects_mismatched_field_sidecar() {
    let struct_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(5),
    };
    let struct_def = DefId::new(CrateId(0), LocalDefId(5));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_struct(
        &mut products,
        struct_product_id,
        HirStruct {
            id: struct_def,
            name: "dep::Widget".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );

    let validator = super::ProductChildLocationValidator::from_products(&products);
    let err = validator
        .validate_field(struct_product_id, FieldId(0), "other", "value")
        .unwrap_err();

    assert!(
        err.contains("field sidecar mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_child_location_remap_rejects_excessive_depth() {
    let mut expr = HirExpr {
        kind: HirExprKindFor::IntLiteral(1),
        ty: Type::I64,
        span: Span::test(),
    };
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::new(),
    };
    let validator = super::ProductChildLocationValidator {
        fields_by_id: HashMap::new(),
        variants_by_id: HashMap::new(),
    };
    let type_validator = super::ProductNominalTypeValidator {
        normalization_env: Default::default(),
        local_crate: ProductCrateId(0),
        structs: BTreeSet::new(),
        enums: BTreeSet::new(),
        type_aliases: BTreeSet::new(),
        traits: BTreeMap::new(),
        trait_generic_counts: BTreeMap::new(),
        trait_methods: BTreeMap::new(),
        trait_signatures: BTreeMap::new(),
        impls: BTreeSet::new(),
        impl_traits: BTreeMap::new(),
        impl_trait_args: BTreeMap::new(),
        impl_methods: BTreeMap::new(),
        effective_trait_methods: BTreeMap::new(),
        methods: BTreeSet::new(),
        static_methods: BTreeSet::new(),
        method_receiver_modes: BTreeMap::new(),
        functions: BTreeSet::new(),
        externs: BTreeSet::new(),
        dependency_structs: BTreeSet::new(),
        dependency_enums: BTreeSet::new(),
        dependency_type_aliases: BTreeSet::new(),
        dependency_traits: BTreeMap::new(),
        dependency_trait_generic_counts: BTreeMap::new(),
        dependency_trait_methods: BTreeMap::new(),
        dependency_trait_signatures: BTreeMap::new(),
        dependency_impls: BTreeSet::new(),
        dependency_impl_traits: BTreeMap::new(),
        dependency_impl_trait_args: BTreeMap::new(),
        dependency_impl_methods: BTreeMap::new(),
        dependency_effective_trait_methods: BTreeMap::new(),
        dependency_methods: BTreeSet::new(),
        dependency_static_methods: BTreeSet::new(),
        dependency_method_receiver_modes: BTreeMap::new(),
        dependency_functions: BTreeSet::new(),
        dependency_externs: BTreeSet::new(),
    };

    let err = super::remap_expr_child_locations(
        &mut expr,
        &remap,
        &validator,
        &type_validator,
        super::MAX_ARTIFACT_HIR_REMAP_DEPTH + 1,
    )
    .unwrap_err();

    assert!(err.contains("maximum remap depth"));
}

#[test]
fn load_product_artifact_rejects_unmapped_product_crate_id() {
    let (base, _cleanup) = temp_test_dir("unmapped_product_crate");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let function_id = product_def_id(0, 0);
    products
        .interface
        .functions
        .get_mut(&function_id)
        .unwrap()
        .generic_params
        .push(GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: DefId::new(CrateId(9), LocalDefId(0)),
                index: 0,
            },
            "T",
        ));
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .unwrap_err();

    assert!(err.contains("unmapped product DefId 9::0"), "{err}");
}

#[test]
fn load_product_artifact_registers_object_backed_crate() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "object_crate"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let function_id = DefId::new(CrateId(1), LocalDefId(7));
    let product_id = ProductDefId::from(function_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    identity_table
        .display_names
        .insert(product_id, "dep::answer".to_string());
    identity_table
        .export_names
        .insert("answer".to_string(), product_id);
    let mut interface = ProductInterface::default();
    interface.functions.insert(
        product_id,
        ProductFunctionInterface::from(&test_function(function_id, "dep::answer")),
    );
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(object_path.clone()),
            records: BTreeMap::from([(
                product_id,
                ProductLinkRecord {
                    backend_symbol: "dep_answer".to_string(),
                },
            )]),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    assert!(dep.link().is_object_backed());
    assert_eq!(dep.link().object_path().cloned(), Some(object_path));
    assert_eq!(dep.link().backend_symbol(function_id), Some("dep_answer"));
    let interface = dep.metadata().interface();
    assert!(interface
        .function_by_canonical_name("dep::answer")
        .is_some());
    assert_eq!(
        interface
            .root_export_ids
            .get("answer")
            .map(|export| export.source.as_str()),
        Some("dep::answer")
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn load_product_artifact_records_root_export_ids() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "root_export_ids"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let interface = dep.metadata().interface();
    let exported = interface.root_export_ids.get("answer").unwrap();
    let function = interface.function_by_canonical_name("dep::answer").unwrap();

    assert_eq!(exported.source, "dep::answer");
    assert_eq!(exported.id, function.id);
    assert_eq!(dep.link().backend_symbol(function.id), Some("dep_answer"));

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_qualifies_method_like_root_export_sources() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "method_like_root_export"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");

    let trait_id = DefId::new(CrateId(1), LocalDefId(8));
    let method_id = DefId::new(CrateId(1), LocalDefId(9));
    let product_trait_id = ProductDefId::from(trait_id);
    let product_method_id = ProductDefId::from(method_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    identity_table
        .display_names
        .insert(product_trait_id, "Show".to_string());
    identity_table
        .display_names
        .insert(product_method_id, "Show::println".to_string());
    identity_table
        .export_names
        .insert("Show::println".to_string(), product_method_id);

    let method = test_function(method_id, "println");
    let mut bodies = ProductBodies::default();
    bodies
        .trait_default_methods
        .insert(product_method_id, method.clone());
    let mut interface = ProductInterface::default();
    interface.traits.insert(
        product_trait_id,
        ProductTraitInterface::from(&HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("println".to_string(), method)]),
            signatures: HashMap::new(),
        }),
    );

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies,
        link: ProductLinkData {
            object_path: None,
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let interface = dep.metadata().interface();
    let exported = interface.root_export_ids.get("Show::println").unwrap();

    assert_eq!(exported.source, "dep::Show::println");
    assert_eq!(exported.id.local, LocalDefId(9));
    assert_eq!(
        dep.metadata()
            .resolver()
            .item_paths
            .get("dep::Show::println"),
        Some(&exported.id)
    );
    assert_eq!(
        dep.metadata().resolver().item_names_by_id.get(&exported.id),
        Some(&"dep::Show::println".to_string())
    );

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_rejects_root_export_without_canonical_id() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "bad_root_export_id"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let missing_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(99),
    };
    products
        .identity_table
        .display_names
        .insert(missing_id, "dep::missing".to_string());
    products
        .identity_table
        .export_names
        .insert("missing".to_string(), missing_id);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .unwrap_err();

    assert!(
        err.contains("root export 'missing'") && err.contains("canonical DefId"),
        "unexpected error: {}",
        err
    );

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_rejects_root_export_without_display_name() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "bad_root_export_display_name"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let missing_name_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(99),
    };
    products
        .identity_table
        .export_names
        .insert("missing".to_string(), missing_name_id);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .unwrap_err();

    assert!(
        err.contains("root export 'missing'") && err.contains("display name"),
        "unexpected error: {}",
        err
    );

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_resolves_common_relative_object_path() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "relative_object"
    ));
    let _ = fs::remove_dir_all(&base);
    let build_dir = base.join("build");
    fs::create_dir_all(&build_dir).unwrap();
    let artifact_path = build_dir.join("dep.rkca");
    let object_path = build_dir.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table: ProductIdentityTable {
            local_crate: Some(ProductCrateId(0)),
            ..Default::default()
        },
        interface: ProductInterface::default(),
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(PathBuf::from("build/dep.o")),
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    assert_eq!(
        dep.link().object_path().cloned(),
        Some(object_path.canonicalize().unwrap())
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn load_product_artifact_prefers_artifact_relative_object_over_cwd() {
    let test_name = format!(
        "rock_product_loader_conflict_{}_{}",
        std::process::id(),
        "artifact_relative"
    );
    let base = std::env::temp_dir().join(&test_name);
    let _ = fs::remove_dir_all(&base);
    let artifact_dir = base.join("artifacts");
    fs::create_dir_all(&artifact_dir).unwrap();
    let artifact_path = artifact_dir.join("dep.rkca");

    let object_path = PathBuf::from("..")
        .join("target")
        .join(&test_name)
        .join("dep.o");
    let cwd_object_path = std::env::current_dir().unwrap().join(&object_path);
    let artifact_object_path = artifact_dir.join(&object_path);
    fs::create_dir_all(cwd_object_path.parent().unwrap()).unwrap();
    fs::create_dir_all(artifact_object_path.parent().unwrap()).unwrap();
    fs::write(&cwd_object_path, b"wrong object").unwrap();
    fs::write(&artifact_object_path, b"right object").unwrap();

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table: ProductIdentityTable {
            local_crate: Some(ProductCrateId(0)),
            ..Default::default()
        },
        interface: ProductInterface::default(),
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(object_path),
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    assert_eq!(
        dep.link().object_path().cloned(),
        Some(artifact_object_path.canonicalize().unwrap())
    );

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(
        std::env::current_dir()
            .unwrap()
            .join("..")
            .join("target")
            .join(test_name),
    );
}

#[test]
fn load_product_artifact_keeps_generic_body_without_display_metadata() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "generic_names"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let function_id = DefId::new(CrateId(1), LocalDefId(7));
    let product_id = ProductDefId::from(function_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    let mut identity = test_function(function_id, "identity");
    identity.generic_params = vec![GenericParamDecl::type_param(
        crate::types::GenericParamId {
            owner: function_id,
            index: 0,
        },
        "T",
    )];
    let mut interface = ProductInterface::default();
    interface
        .functions
        .insert(product_id, ProductFunctionInterface::from(&identity));
    let mut bodies = crate::products::ProductBodies::default();
    bodies.functions.insert(product_id, identity);
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies,
        link: ProductLinkData {
            object_path: Some(object_path.clone()),
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    assert!(dep.metadata().interface().canonical_names.is_empty());
    let loaded_id = *dep
        .metadata()
        .interface()
        .functions
        .keys()
        .next()
        .expect("function interface should survive without display metadata");
    assert!(dep.body_providers().generic_function(loaded_id).is_some());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn resolver_from_products_ignores_impl_display_names() {
    let impl_id = DefId::new(CrateId(1), LocalDefId(8));
    let product_id = ProductDefId::from(impl_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table
        .display_names
        .insert(product_id, "Box as Show".to_string());

    let mut interface = ProductInterface::default();
    interface.impls.insert(
        product_id,
        ProductImplInterface::from(&HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: Some("Show".to_string()),
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        }),
    );
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies: Default::default(),
        link: ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };

    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::new(),
    };
    let resolver =
        super::resolver_from_products(&products, "dep", &remap, &CrateContext::new()).unwrap();

    assert!(!resolver.item_paths.contains_key("Box as Show"));
}

#[test]
fn load_product_artifact_records_id_backed_prelude_exports() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "id_backed_prelude_exports"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("std.rkca");
    let object_path = base.join("std.o");
    fs::write(&object_path, []).unwrap();

    let struct_id = DefId::new(CrateId(1), LocalDefId(7));
    let product_id = ProductDefId::from(struct_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    identity_table
        .display_names
        .insert(product_id, "std::prelude::String".to_string());
    identity_table
        .prelude_export_names
        .insert("Text".to_string(), product_id);

    let mut interface = ProductInterface::default();
    interface.structs.insert(
        product_id,
        ProductStructInterface::from(&HirStruct {
            id: struct_id,
            name: "String".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        }),
    );

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("std".to_string()),
        identity_table,
        interface,
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(object_path.clone()),
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let std = ctx.extern_crate("std").unwrap();
    let export = std.metadata().prelude_export_ids().get("Text").unwrap();
    assert_eq!(export.source, "std::prelude::String");
    assert_eq!(export.id.crate_id, CrateId(1));

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_rejects_prelude_export_id_without_display_name() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "bad_id_backed_prelude_export"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("std.rkca");
    let object_path = base.join("std.o");
    fs::write(&object_path, []).unwrap();

    let product_id = ProductDefId {
        crate_id: ProductCrateId(1),
        local_id: ProductLocalDefId(7),
    };
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    identity_table
        .prelude_export_names
        .insert("String".to_string(), product_id);

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("std".to_string()),
        identity_table,
        interface: ProductInterface::default(),
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(object_path.clone()),
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .unwrap_err();

    assert!(
        err.contains("prelude export 'String'") && err.contains("display name"),
        "unexpected error: {}",
        err
    );

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_rejects_sidecar_id_backed_prelude_export() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "sidecar_id_backed_prelude_export"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("demo.rkca");
    let object_path = base.join("demo.o");
    fs::write(&object_path, []).unwrap();

    let trait_id = DefId::new(CrateId(1), LocalDefId(7));
    let trait_product_id = ProductDefId::from(trait_id);
    let method_id = DefId::new(CrateId(1), LocalDefId(8));
    let method_product_id = ProductDefId::from(method_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    identity_table
        .display_names
        .insert(trait_product_id, "Trait".to_string());
    identity_table
        .display_names
        .insert(method_product_id, "default_method".to_string());
    identity_table
        .prelude_export_names
        .insert("default_method".to_string(), method_product_id);

    let method = test_function(method_id, "default_method");
    let mut bodies = ProductBodies::default();
    bodies
        .trait_default_methods
        .insert(method_product_id, method.clone());
    let mut interface = ProductInterface::default();
    interface.traits.insert(
        trait_product_id,
        ProductTraitInterface::from(&HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Trait".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("default_method".to_string(), method)]),
            signatures: HashMap::new(),
        }),
    );

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("demo".to_string()),
        identity_table,
        interface,
        bodies,
        link: ProductLinkData {
            object_path: Some(object_path.clone()),
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .unwrap_err();

    assert!(
        err.contains("prelude export 'default_method'")
            && err.contains("canonical prelude item ID"),
        "unexpected error: {}",
        err
    );

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_derives_missing_prelude_exports_from_prelude_items() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "derived_prelude_exports"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("std.rkca");
    let object_path = base.join("std.o");
    fs::write(&object_path, []).unwrap();

    let struct_id = DefId::new(CrateId(1), LocalDefId(7));
    let product_id = ProductDefId::from(struct_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    identity_table
        .display_names
        .insert(product_id, "std::prelude::String".to_string());

    let mut interface = ProductInterface::default();
    interface.structs.insert(
        product_id,
        ProductStructInterface::from(&HirStruct {
            id: struct_id,
            name: "String".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        }),
    );

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("std".to_string()),
        identity_table,
        interface,
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(object_path.clone()),
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let std = ctx.extern_crate("std").unwrap();
    let interface = std.metadata().interface();
    let exported_struct = interface
        .struct_by_canonical_name("std::prelude::String")
        .unwrap();
    let prelude_export = std.metadata().prelude_export_ids().get("String").unwrap();
    assert_eq!(prelude_export.source, "std::prelude::String");
    assert_eq!(prelude_export.id, exported_struct.id);

    let _ = fs::remove_dir_all(base);
}
