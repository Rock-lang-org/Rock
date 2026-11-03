use std::collections::HashMap;

use super::*;
use crate::hir::{
    HirAssociatedTypeDef, HirBlock, HirEnum, HirExpr, HirExprKind, HirExtern, HirField,
    HirFunction, HirFunctionSig, HirImpl, HirImplOwner, HirMethodCallTarget, HirNameTables,
    HirParam, HirPattern, HirProgram, HirStmt, HirStruct, HirTrait, HirVariant, HirVariantFields,
};
use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
use crate::type_context::TypeContext;
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, TraitBound, Type};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn generic(owner: DefId, index: u32) -> GenericParamId {
    GenericParamId { owner, index }
}

fn empty_body(ty: Type) -> HirBlock {
    HirBlock {
        stmts: Vec::new(),
        ty,
    }
}

fn function(id: DefId, name: &str, param_ty: Type, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: param_ty,
            mutable: false,
            is_ref: false,
        }],
        ret_type: ret_type.clone(),
        body: empty_body(ret_type),
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn program(
    functions: HashMap<DefId, HirFunction>,
    structs: HashMap<DefId, HirStruct>,
    enums: HashMap<DefId, HirEnum>,
    traits: HashMap<DefId, HirTrait>,
    impls: HashMap<DefId, HirImpl>,
    externs: HashMap<DefId, HirExtern>,
    names: HirNameTables,
    canonical_names: HashMap<DefId, String>,
) -> HirProgram {
    HirProgram::from_id_parts_with_names_and_canonical_names(
        functions,
        structs,
        enums,
        traits,
        impls,
        externs,
        names,
        crate::hir::HirLanguageItems::default(),
        &canonical_names,
    )
}

fn program_with_function_body(owner: DefId, body: HirBlock) -> HirProgram {
    let function = HirFunction {
        id: owner,
        name: "main".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: body.ty.clone(),
        body,
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    program(
        HashMap::from([(owner, function)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("main".to_string(), owner)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    )
}

fn single_type_arg_bound(trait_id: DefId, ty: Type) -> TraitBound {
    TraitBound {
        trait_id,
        type_args: vec![ty],
    }
}

#[test]
fn hir_type_ids_record_and_read_locations() {
    let function = def_id(1);
    let location = HirTypeLocation::FunctionReturn { function };
    let id = crate::ids::TypeId(7);
    let mut ids = HirTypeIds::new();

    assert!(ids.is_empty());
    ids.insert(location.clone(), id);

    assert_eq!(ids.get(&location), Some(id));
    assert_eq!(ids.len(), 1);
    assert!(!ids.is_empty());
    assert_eq!(ids.iter().collect::<Vec<_>>(), vec![(&location, id)]);
}

#[test]
fn hir_type_ids_collect_top_level_type_locations() {
    let function_id = def_id(1);
    let extern_id = def_id(2);
    let struct_id = def_id(3);
    let other_struct_id = def_id(4);
    let enum_id = def_id(5);
    let trait_id = def_id(6);
    let signature_id = def_id(7);
    let impl_id = def_id(8);
    let assoc_id = AssocTypeId(0);
    let generic_param = generic(impl_id, 0);
    let projection = Type::Projection {
        ty: Box::new(Type::Generic(generic_param)),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: assoc_id,
        },
        trait_args: vec![Type::I64],
    };

    let program = program(
        HashMap::from([(
            function_id,
            function(function_id, "id", Type::I64, Type::I64),
        )]),
        HashMap::from([
            (
                struct_id,
                HirStruct {
                    id: struct_id,
                    name: "Box".to_string(),
                    generic_params: Vec::new(),
                    fields: vec![HirField {
                        id: FieldId(0),
                        name: "value".to_string(),
                        ty: Type::Struct {
                            id: struct_id,
                            args: Vec::new(),
                        },
                        public: false,
                    }],
                },
            ),
            (
                other_struct_id,
                HirStruct {
                    id: other_struct_id,
                    name: "OtherBox".to_string(),
                    generic_params: Vec::new(),
                    fields: vec![HirField {
                        id: FieldId(0),
                        name: "value".to_string(),
                        ty: Type::Struct {
                            id: other_struct_id,
                            args: Vec::new(),
                        },
                        public: false,
                    }],
                },
            ),
        ]),
        HashMap::from([(
            enum_id,
            HirEnum {
                id: enum_id,
                name: "Maybe".to_string(),
                generic_params: Vec::new(),
                variants: vec![
                    HirVariant {
                        id: VariantId(0),
                        name: "Named".to_string(),
                        fields: HirVariantFields::Named(vec![HirField {
                            id: FieldId(0),
                            name: "payload".to_string(),
                            ty: Type::Bool,
                            public: false,
                        }]),
                    },
                    HirVariant {
                        id: VariantId(1),
                        name: "Tuple".to_string(),
                        fields: HirVariantFields::Positional(vec![Type::I64]),
                    },
                ],
            },
        )]),
        HashMap::from([(
            trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Iterable".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "next".to_string(),
                    HirFunctionSig {
                        id: signature_id,
                        name: "next".to_string(),
                        generic_params: Vec::new(),
                        params: vec![Type::I64],
                        ret: Type::Bool,
                        generic_bounds: HashMap::new().into(),
                        self_receiver: None,
                        is_unsafe: false,
                    },
                )]),
            },
        )]),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: GenericParamDecl::type_params(impl_id, ["T"]),
                receiver_pattern: vec![Type::Generic(generic_param)].into(),
                trait_name: Some("Iterable".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: vec![Type::I64],
                associated_types: vec![HirAssociatedTypeDef {
                    id: assoc_id,
                    name: "Item".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: projection.clone(),
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            },
        )]),
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
            functions_by_name: HashMap::from([("id".to_string(), function_id)]),
            structs_by_name: HashMap::from([
                ("Box".to_string(), struct_id),
                ("OtherBox".to_string(), other_struct_id),
            ]),
            enums_by_name: HashMap::from([("Maybe".to_string(), enum_id)]),
            traits_by_name: HashMap::from([("Iterable".to_string(), trait_id)]),
            externs_by_name: HashMap::from([("puts".to_string(), extern_id)]),
            type_aliases_by_name: HashMap::new(),
        },
        HashMap::new(),
    );

    let mut context = TypeContext::new();
    let ids = collect_hir_type_ids(&program, &mut context);

    let function_ret = ids
        .get(&HirTypeLocation::FunctionReturn {
            function: function_id,
        })
        .unwrap();
    let function_param = ids
        .get(&HirTypeLocation::FunctionParam {
            function: function_id,
            index: 0,
        })
        .unwrap();
    assert_eq!(function_ret, function_param);
    assert_eq!(context.type_for(function_ret), Type::I64);
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ExternParam {
                extern_id,
                index: 0,
            })
            .unwrap()
        ),
        Type::Pointer(Box::new(Type::U8))
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ExternReturn { extern_id })
                .unwrap()
        ),
        Type::I32
    );
    let first_nominal = ids
        .get(&HirTypeLocation::StructField {
            owner: struct_id,
            field: FieldId(0),
        })
        .unwrap();
    let second_nominal = ids
        .get(&HirTypeLocation::StructField {
            owner: other_struct_id,
            field: FieldId(0),
        })
        .unwrap();
    assert_ne!(first_nominal, second_nominal);
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::EnumVariantNamedField {
                owner: enum_id,
                variant: VariantId(0),
                field: FieldId(0),
            })
            .unwrap()
        ),
        Type::Bool
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::EnumVariantPositionalField {
                owner: enum_id,
                variant: VariantId(1),
                index: 0,
            })
            .unwrap()
        ),
        Type::I64
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::TraitSignatureReturn {
                trait_id,
                signature: signature_id,
            })
            .unwrap()
        ),
        Type::Bool
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::TraitSignatureParam {
                trait_id,
                signature: signature_id,
                index: 0,
            })
            .unwrap()
        ),
        Type::I64
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ImplReceiverArg { impl_id, index: 0 })
                .unwrap()
        ),
        Type::Generic(generic_param)
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ImplTraitArg { impl_id, index: 0 })
                .unwrap()
        ),
        Type::I64
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::AssociatedTypeDef {
                impl_id,
                assoc_type: assoc_id,
            })
            .unwrap()
        ),
        projection
    );
    assert!(!ids.is_empty());
}

#[test]
fn hir_type_ids_collect_downstream_required_payload_types() {
    let owner = def_id(200);
    let trait_id = def_id(201);
    let struct_id = def_id(202);
    let method_id = def_id(203);
    let mut context = TypeContext::new();
    let expr = HirExpr {
        kind: HirExprKind::Cast(
            Box::new(HirExpr {
                kind: HirExprKind::MethodCall(
                    Box::new(HirExpr {
                        kind: HirExprKind::Var("receiver".to_string()),
                        ty: Type::Struct {
                            id: struct_id,
                            args: vec![Type::I64],
                        },
                        span: crate::lexer::Span::test(),
                    }),
                    "value".to_string(),
                    Vec::new(),
                    None,
                    Some(HirMethodCallTarget::impl_method(
                        owner,
                        method_id,
                        Some(crate::hir::HirSelectedTraitMember {
                            trait_id,
                            member_id: method_id,
                            trait_args: vec![Type::Bool],
                        }),
                    )),
                ),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }),
            Type::U64,
        ),
        ty: Type::U64,
        span: crate::lexer::Span::test(),
    };
    let pattern = HirPattern::Struct(
        "Box".to_string(),
        Some(struct_id),
        vec![Type::Bool],
        Vec::new(),
    );
    let program = program_with_function_body(
        owner,
        HirBlock {
            stmts: vec![HirStmt::Expr(expr)],
            ty: Type::Unit,
        },
    );
    let ids = collect_hir_type_ids(&program, &mut context);

    assert!(ids
        .get(&HirTypeLocation::CastTarget {
            owner,
            path: vec![0, 0],
        })
        .is_some());
    assert!(ids
        .get(&HirTypeLocation::MethodCallTraitArg {
            owner,
            path: vec![0, 0, 0],
            index: 0,
        })
        .is_some());

    let mut pattern_context = TypeContext::new();
    let mut pattern_ids = HirTypeIds::new();
    collect_pattern_type_ids(
        owner,
        &pattern,
        vec![9],
        &mut pattern_context,
        &mut pattern_ids,
    );
    assert!(pattern_ids
        .get(&HirTypeLocation::StructPatternArg {
            owner,
            path: vec![9],
            index: 0,
        })
        .is_some());
}

#[test]
fn hir_type_ids_collect_body_type_locations() {
    let function_id = def_id(20);
    let capture_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };
    let lambda_ty = Type::function(vec![Type::I64], Type::I64);
    let function = HirFunction {
        id: function_id,
        name: "main".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: vec![
                crate::hir::HirStmt::Let {
                    name: "x".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::I64,
                    value: crate::hir::HirExpr {
                        kind: crate::hir::HirExprKind::IntLiteral(1),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    },
                    mutable: false,
                },
                crate::hir::HirStmt::Expr(crate::hir::HirExpr {
                    kind: crate::hir::HirExprKind::Lambda {
                        params: vec![HirParam {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I64,
                            mutable: false,
                            is_ref: false,
                        }],
                        body: HirBlock {
                            stmts: vec![crate::hir::HirStmt::Expr(crate::hir::HirExpr {
                                kind: crate::hir::HirExprKind::Var("value".to_string()),
                                ty: Type::I64,
                                span: crate::lexer::Span::test(),
                            })],
                            ty: Type::I64,
                        },
                        captures: vec![crate::hir::HirClosureCapture {
                            name: "x".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            kind: crate::hir::HirClosureCaptureKind::SharedBorrow,
                            mutable: false,
                            ty: capture_ty.clone(),
                        }],
                    },
                    ty: lambda_ty.clone(),
                    span: crate::lexer::Span::test(),
                }),
            ],
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = program(
        HashMap::from([(function_id, function)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("main".to_string(), function_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let mut context = TypeContext::new();
    let ids = collect_hir_type_ids(&program, &mut context);

    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::Block {
                owner: function_id,
                path: vec![],
            })
            .unwrap()
        ),
        Type::Unit
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::LetStmt {
                owner: function_id,
                path: vec![0],
                name: "x".to_string(),
            })
            .unwrap()
        ),
        Type::I64
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::Expr {
                owner: function_id,
                path: vec![0, 0],
            })
            .unwrap()
        ),
        Type::I64
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::Expr {
                owner: function_id,
                path: vec![1, 0],
            })
            .unwrap()
        ),
        lambda_ty
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ClosureCapture {
                owner: function_id,
                path: vec![1, 0],
                name: "x".to_string(),
            })
            .unwrap()
        ),
        capture_ty
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ClosureParam {
                owner: function_id,
                path: vec![1, 0],
                index: 0,
            })
            .unwrap()
        ),
        Type::I64
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::Block {
                owner: function_id,
                path: vec![1, 0, 1],
            })
            .unwrap()
        ),
        Type::I64
    );
}

#[test]
fn hir_type_ids_collect_in_stable_semantic_order() {
    let mut functions = HashMap::new();
    let mut function_names = HashMap::new();
    for index in (1..=16).rev() {
        let id = def_id(index);
        let ty = Type::Struct {
            id,
            args: Vec::new(),
        };
        functions.insert(id, function(id, &format!("f{index}"), ty.clone(), ty));
        function_names.insert(format!("f{index}"), id);
    }

    let program = program(
        functions,
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: function_names,
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let mut context = TypeContext::new();
    let ids = collect_hir_type_ids(&program, &mut context);

    for index in 1..=16 {
        let function = def_id(index);
        let id = ids
            .get(&HirTypeLocation::FunctionReturn { function })
            .unwrap();
        assert_eq!(id, crate::ids::TypeId(index - 1));
        assert_eq!(
            context.type_for(id),
            Type::Struct {
                id: function,
                args: Vec::new(),
            }
        );
    }
}

#[test]
fn hir_type_ids_collect_bound_type_args() {
    let function_id = def_id(1);
    let trait_id = def_id(2);
    let signature_id = def_id(3);
    let impl_id = def_id(4);
    let function_param = generic(function_id, 0);
    let signature_param = generic(signature_id, 0);
    let impl_param = generic(impl_id, 0);

    let mut bounded_function = function(function_id, "bounded", Type::I64, Type::I64);
    *bounded_function.generic_bounds = HashMap::from([(
        function_param,
        vec![single_type_arg_bound(
            trait_id,
            Type::Pointer(Box::new(Type::U8)),
        )],
    )])
    .into();

    let program = program(
        HashMap::from([(function_id, bounded_function)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Iterable".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "next".to_string(),
                    HirFunctionSig {
                        id: signature_id,
                        name: "next".to_string(),
                        generic_params: vec![GenericParamDecl::type_param(signature_param, "T")],
                        params: vec![Type::I64],
                        ret: Type::Bool,
                        generic_bounds: HashMap::from([(
                            signature_param,
                            vec![single_type_arg_bound(trait_id, Type::Bool)],
                        )])
                        .into(),
                        self_receiver: None,
                        is_unsafe: false,
                    },
                )]),
            },
        )]),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: GenericParamDecl::type_params(impl_id, ["T"]),
                receiver_pattern: Vec::new().into(),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::from([(
                    impl_param,
                    vec![single_type_arg_bound(trait_id, Type::Generic(impl_param))],
                )])
                .into(),
                methods: HashMap::new(),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("bounded".to_string(), function_id)]),
            traits_by_name: HashMap::from([("Iterable".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let mut context = TypeContext::new();
    let ids = collect_hir_type_ids(&program, &mut context);

    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::FunctionBoundArg {
                function: function_id,
                param: function_param,
                bound_index: 0,
                arg_index: 0,
            })
            .unwrap()
        ),
        Type::Pointer(Box::new(Type::U8))
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::TraitSignatureBoundArg {
                trait_id,
                signature: signature_id,
                param: signature_param,
                bound_index: 0,
                arg_index: 0,
            })
            .unwrap()
        ),
        Type::Bool
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ImplBoundArg {
                impl_id,
                param: impl_param,
                bound_index: 0,
                arg_index: 0,
            })
            .unwrap()
        ),
        Type::Generic(impl_param)
    );
}
