use super::*;

#[test]
fn hir_method_locations_use_owner_ids_not_names_or_indexes() {
    let trait_id = def_id(20);
    let trait_method_id = def_id(21);
    let impl_id = def_id(22);
    let impl_method_id = def_id(23);

    let traits = HashMap::from([(
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Display".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("fmt".to_string(), test_function(trait_method_id, "fmt"))]),
            signatures: HashMap::new(),
        },
    )]);
    let impls = HashMap::from([(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Widget".to_string()),
            type_name: "Widget".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: Some("Display".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("fmt".to_string(), test_function(impl_method_id, "fmt"))]),
        },
    )]);

    let program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        traits,
        impls,
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([("Display".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    assert_eq!(
        program.indexes.methods_by_id.get(&trait_method_id),
        Some(&HirMethodLocation::TraitDefault {
            trait_id,
            method_id: trait_method_id,
            method_name: "fmt".to_string(),
        })
    );
    assert_eq!(
        program.indexes.methods_by_id.get(&impl_method_id),
        Some(&HirMethodLocation::ImplMethod {
            impl_id,
            method_id: impl_method_id,
            method_name: "fmt".to_string(),
        })
    );
    assert_eq!(
        program.effective_trait_method(impl_id, trait_method_id),
        None
    );
}

#[test]
fn hir_index_build_does_not_reconstruct_effective_trait_methods_by_name() {
    let trait_id = def_id(120);
    let trait_method_id = def_id(121);
    let impl_id = def_id(122);
    let impl_method_id = def_id(123);
    let traits = HashMap::from([(
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Display".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("fmt".to_string(), test_function(trait_method_id, "fmt"))]),
            signatures: HashMap::new(),
        },
    )]);
    let impls = HashMap::from([(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Widget".to_string()),
            type_name: "Widget".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: Some("Display".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("fmt".to_string(), test_function(impl_method_id, "fmt"))]),
        },
    )]);

    let mut program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        traits,
        impls,
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([("Display".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );
    program.rebuild_indexes();

    assert!(program.indexes.effective_trait_methods.is_empty());
}

#[test]
fn hir_program_does_not_reconstruct_impl_owner_id_from_display_alias() {
    let struct_id = def_id(80);
    let impl_id = def_id(81);
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::from([(struct_id, empty_struct(struct_id, "Box"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("AliasBox".to_string()),
                type_name: "AliasBox".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Unit),
                trait_name: None,
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
            structs_by_name: HashMap::from([
                ("module::Box".to_string(), struct_id),
                ("AliasBox".to_string(), struct_id),
            ]),
            ..HirNameTables::default()
        },
        crate::hir::HirLanguageItems::default(),
        &HashMap::from([(struct_id, "module::Box".to_string())]),
    );

    assert_eq!(program.impl_owner_id(impl_id), None);
    assert_eq!(
        program.impl_receiver_pattern(impl_id),
        Some(&HirImplReceiverPattern::Exact(Type::Unit))
    );
}

#[test]
fn hir_program_externs_keep_owned_ids_after_serde_roundtrip_and_rebuild() {
    let extern_id = def_id(13);
    let default_id = DefId::new(CrateId(0), LocalDefId(0));
    let program = program(
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
                params: vec![Type::Str],
                ret: Type::I32,
                variadic: false,
                is_unsafe: false,
            },
        )]),
        HirNameTables {
            externs_by_name: HashMap::from([("puts".to_string(), extern_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let bytes = bincode::serialize(&program).unwrap();
    let mut decoded: HirProgram = bincode::deserialize(&bytes).unwrap();
    decoded.rebuild_indexes();

    assert!(decoded.extern_by_id(extern_id).is_some());
    assert!(!decoded.indexes.externs_by_id.contains_key(&default_id));
}

#[test]
fn hir_program_indexes_same_named_fields_by_owner_and_field_id() {
    let left_owner = def_id(10);
    let right_owner = def_id(11);
    let program = program(
        HashMap::new(),
        HashMap::from([
            (
                left_owner,
                HirStruct {
                    id: left_owner,
                    name: "Left".to_string(),
                    generic_params: Vec::new(),
                    fields: vec![HirField {
                        id: field_id(0),
                        name: "value".to_string(),
                        ty: Type::I32,
                        public: true,
                    }],
                },
            ),
            (
                right_owner,
                HirStruct {
                    id: right_owner,
                    name: "Right".to_string(),
                    generic_params: Vec::new(),
                    fields: vec![HirField {
                        id: field_id(0),
                        name: "value".to_string(),
                        ty: Type::I64,
                        public: true,
                    }],
                },
            ),
        ]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([
                ("Left".to_string(), left_owner),
                ("Right".to_string(), right_owner),
            ]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    assert_eq!(
        program.indexes.fields_by_id[&(left_owner, field_id(0))],
        HirFieldLocation {
            owner: left_owner,
            field_id: field_id(0),
            name: "value".to_string(),
        }
    );
    assert_eq!(
        program.indexes.fields_by_id[&(right_owner, field_id(0))],
        HirFieldLocation {
            owner: right_owner,
            field_id: field_id(0),
            name: "value".to_string(),
        }
    );
    assert_eq!(program.indexes.fields_by_id.len(), 2);
}

#[test]
fn hir_program_indexes_same_named_variants_by_owner_and_variant_id() {
    let left_owner = def_id(20);
    let right_owner = def_id(21);
    let program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::from([
            (
                left_owner,
                HirEnum {
                    id: left_owner,
                    name: "LeftChoice".to_string(),
                    generic_params: Vec::new(),
                    variants: vec![HirVariant {
                        id: variant_id(0),
                        name: "Some".to_string(),
                        fields: HirVariantFields::Unit,
                    }],
                },
            ),
            (
                right_owner,
                HirEnum {
                    id: right_owner,
                    name: "RightChoice".to_string(),
                    generic_params: Vec::new(),
                    variants: vec![HirVariant {
                        id: variant_id(0),
                        name: "Some".to_string(),
                        fields: HirVariantFields::Unit,
                    }],
                },
            ),
        ]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            enums_by_name: HashMap::from([
                ("LeftChoice".to_string(), left_owner),
                ("RightChoice".to_string(), right_owner),
            ]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    assert_eq!(
        program.indexes.variants_by_id[&(left_owner, variant_id(0))],
        HirVariantLocation {
            owner: left_owner,
            variant_id: variant_id(0),
            name: "Some".to_string(),
        }
    );
    assert_eq!(
        program.indexes.variants_by_id[&(right_owner, variant_id(0))],
        HirVariantLocation {
            owner: right_owner,
            variant_id: variant_id(0),
            name: "Some".to_string(),
        }
    );
    assert_eq!(program.indexes.variants_by_id.len(), 2);
}

#[test]
fn hir_program_indexes_associated_type_decl_and_def_by_owner_and_assoc_type_id() {
    let trait_owner = def_id(30);
    let impl_owner = def_id(31);
    let program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            trait_owner,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_owner,
                name: "Iterable".to_string(),
                generic_params: Vec::new(),
                associated_types: vec![HirAssociatedTypeDecl {
                    id: assoc_type_id(0),
                    name: "Item".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                }],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        )]),
        HashMap::from([(
            impl_owner,
            HirImpl {
                id: impl_owner,
                owner: HirImplOwner::Named("List".to_string()),
                type_name: "List".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: Vec::new().into(),
                trait_name: Some("Iterable".to_string()),
                trait_id: Some(trait_owner),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: vec![HirAssociatedTypeDef {
                    id: assoc_type_id(0),
                    name: "Item".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: Type::I32,
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([("Iterable".to_string(), trait_owner)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    assert_eq!(
        program.indexes.associated_type_decls_by_id[&(trait_owner, assoc_type_id(0))],
        HirAssociatedTypeLocation {
            owner: trait_owner,
            assoc_type_id: assoc_type_id(0),
            name: "Item".to_string(),
        }
    );
    assert_eq!(
        program.indexes.associated_type_defs_by_id[&(impl_owner, assoc_type_id(0))],
        HirAssociatedTypeLocation {
            owner: impl_owner,
            assoc_type_id: assoc_type_id(0),
            name: "Item".to_string(),
        }
    );
    assert_eq!(program.indexes.associated_type_decls_by_id.len(), 1);
    assert_eq!(program.indexes.associated_type_defs_by_id.len(), 1);
}

#[test]
fn hir_extern_carries_canonical_def_id() {
    let extern_fn = HirExtern {
        id: DefId::new(CrateId(0), LocalDefId(9)),
        name: "puts".to_string(),
        params: vec![Type::Str],
        ret: Type::I32,
        variadic: false,
        is_unsafe: false,
    };

    assert_eq!(extern_fn.id, DefId::new(CrateId(0), LocalDefId(9)));
}

#[test]
fn hir_extern_serde_roundtrips_id() {
    let extern_fn = HirExtern {
        id: DefId::new(CrateId(1), LocalDefId(9)),
        name: "printf".to_string(),
        params: vec![Type::Pointer(Box::new(Type::I8))],
        ret: Type::I32,
        variadic: true,
        is_unsafe: false,
    };

    let bytes = bincode::serialize(&extern_fn).unwrap();
    let decoded: HirExtern = bincode::deserialize(&bytes).unwrap();

    assert_eq!(decoded.id, extern_fn.id);
    assert_eq!(decoded.name, extern_fn.name);
    assert_eq!(decoded.params, extern_fn.params);
    assert_eq!(decoded.ret, extern_fn.ret);
    assert_eq!(decoded.variadic, extern_fn.variadic);
}

#[test]
fn hir_program_serde_skips_derived_indexes() {
    let program = program(
        HashMap::from([(def_id(1), empty_function(def_id(1), "main"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            def_id(2),
            HirExtern {
                id: def_id(2),
                name: "puts".to_string(),
                params: vec![Type::Str],
                ret: Type::I32,
                variadic: false,
                is_unsafe: false,
            },
        )]),
        HirNameTables {
            functions_by_name: HashMap::from([("main".to_string(), def_id(1))]),
            externs_by_name: HashMap::from([("puts".to_string(), def_id(2))]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    assert!(!program.indexes.functions_by_id.is_empty());
    assert!(!program.indexes.externs_by_id.is_empty());

    let bytes = bincode::serialize(&program).unwrap();
    let decoded: HirProgram = bincode::deserialize(&bytes).unwrap();

    assert!(decoded.indexes.functions_by_id.is_empty());
    assert!(decoded.indexes.externs_by_id.is_empty());
    assert_eq!(decoded.externs[&def_id(2)].id, def_id(2));
}

#[test]
fn hir_program_indexes_definitions_by_def_id() {
    let program = program(
        HashMap::from([(def_id(1), empty_function(def_id(1), "main"))]),
        HashMap::from([(def_id(2), empty_struct(def_id(2), "Thing"))]),
        HashMap::from([(def_id(3), empty_enum(def_id(3), "Choice"))]),
        HashMap::from([(
            def_id(4),
            trait_with_default_method(def_id(4), "Show", def_id(8)),
        )]),
        HashMap::from([(def_id(5), impl_with_method(def_id(5), def_id(9)))]),
        HashMap::from([(
            def_id(6),
            HirExtern {
                id: def_id(6),
                name: "puts".to_string(),
                params: vec![Type::Str],
                ret: Type::I32,
                variadic: false,
                is_unsafe: false,
            },
        )]),
        HirNameTables {
            functions_by_name: HashMap::from([("main".to_string(), def_id(1))]),
            structs_by_name: HashMap::from([("Thing".to_string(), def_id(2))]),
            enums_by_name: HashMap::from([("Choice".to_string(), def_id(3))]),
            traits_by_name: HashMap::from([("Show".to_string(), def_id(4))]),
            externs_by_name: HashMap::from([("puts".to_string(), def_id(6))]),
            type_aliases_by_name: HashMap::new(),
        },
        HashMap::new(),
    );

    assert_eq!(program.indexes.functions_by_id[&def_id(1)], "main");
    assert_eq!(program.indexes.structs_by_id[&def_id(2)], "Thing");
    assert_eq!(program.indexes.enums_by_id[&def_id(3)], "Choice");
    assert_eq!(program.indexes.traits_by_id[&def_id(4)], "Show");
    assert_eq!(program.indexes.impls_by_id[&def_id(5)], 0);
    assert_eq!(program.indexes.externs_by_id[&def_id(6)], 0);
    assert_eq!(
        program.indexes.methods_by_id[&def_id(8)],
        HirMethodLocation::TraitDefault {
            trait_id: def_id(4),
            method_id: def_id(8),
            method_name: "default_method".to_string(),
        }
    );
    assert_eq!(
        program.indexes.methods_by_id[&def_id(9)],
        HirMethodLocation::ImplMethod {
            impl_id: def_id(5),
            method_id: def_id(9),
            method_name: "impl_method".to_string(),
        }
    );
}

#[test]
fn hir_indexes_distinguish_multiple_inherited_default_method_ids() {
    let first_method_id = def_id(8);
    let second_method_id = def_id(9);
    let program = program(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([
            (
                def_id(4),
                HirTrait {
                    target: None,
                    predicates: Vec::new(),
                    id: def_id(4),
                    name: "First".to_string(),
                    generic_params: Vec::new(),
                    associated_types: Vec::new(),
                    methods: HashMap::from([(
                        "value".to_string(),
                        empty_function(first_method_id, "value"),
                    )]),
                    signatures: HashMap::new(),
                },
            ),
            (
                def_id(5),
                HirTrait {
                    target: None,
                    predicates: Vec::new(),
                    id: def_id(5),
                    name: "Second".to_string(),
                    generic_params: Vec::new(),
                    associated_types: Vec::new(),
                    methods: HashMap::from([(
                        "value".to_string(),
                        empty_function(second_method_id, "value"),
                    )]),
                    signatures: HashMap::new(),
                },
            ),
        ]),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            traits_by_name: HashMap::from([
                ("First".to_string(), def_id(4)),
                ("Second".to_string(), def_id(5)),
            ]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    assert_eq!(
        program.indexes.methods_by_id[&first_method_id],
        HirMethodLocation::TraitDefault {
            trait_id: def_id(4),
            method_id: first_method_id,
            method_name: "value".to_string(),
        }
    );
    assert_eq!(
        program.indexes.methods_by_id[&second_method_id],
        HirMethodLocation::TraitDefault {
            trait_id: def_id(5),
            method_id: second_method_id,
            method_name: "value".to_string(),
        }
    );
}

#[test]
fn substitute_typevars_in_function_updates_method_target_trait_args() {
    let trait_id = def_id(100);
    let method_id = def_id(101);
    let impl_id = def_id(102);
    let generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let source_ty = Type::Generic(generic);
    let mut function = HirFunction {
        id: method_id,
        name: "call".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: source_ty.clone(),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::MethodCall(
                    Box::new(HirExpr {
                        kind: HirExprKind::Var("self".to_string()),
                        ty: source_ty.clone(),
                        span: crate::lexer::Span::test(),
                    }),
                    "target".to_string(),
                    Vec::new(),
                    None,
                    Some(HirMethodCallTarget::impl_method(
                        impl_id,
                        method_id,
                        Some(HirSelectedTraitMember {
                            trait_id,
                            member_id: method_id,
                            trait_args: vec![source_ty.clone()],
                        }),
                    )),
                ),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            })],
            ty: Type::I64,
        },
        is_curried: false,
        is_method: true,
        self_receiver: None,
        is_unsafe: false,
    };

    super::super::substitute_typevars_in_function(&mut function, &source_ty, &Type::I64);

    let HirStmt::Expr(expr) = &function.body.stmts[0] else {
        panic!("function body should contain method call");
    };
    let HirExprKind::MethodCall(_, _, _, _, Some(target)) = &expr.kind else {
        panic!("function body should contain method target");
    };
    assert_eq!(target.trait_args(), &[Type::I64]);
}
