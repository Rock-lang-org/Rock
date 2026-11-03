use super::*;

#[test]
fn assert_ir_order_ignores_target_dependent_aggregate_gep_index_widths() {
    let ir = "%field = getelementptr inbounds nuw { i64, i64 }, ptr %value, i64 0, i64 1\n\
                  %loaded = load i64, ptr %field";

    assert_ir_order_ignoring_aggregate_gep_index_widths(
        ir,
        &[
            "%field = getelementptr inbounds { i64, i64 }, ptr %value, i32 0, i32 1",
            "%loaded = load i64, ptr %field",
        ],
    );
}

#[test]
fn mir_codegen_reports_bad_aggregate_operand_counts() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    let too_few_tuple = compile_test_function_result(
        &mut codegen,
        &type_context,
        "bad_tuple_aggregate",
        Type::Tuple(vec![Type::I64, Type::I64]),
        vec![],
        vec![local_decl(
            &type_context,
            Type::Tuple(vec![Type::I64, Type::I64]),
            "return_place",
        )],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::Aggregate(
                AggregateKind::Tuple,
                vec![Operand::Constant(Constant::Int(1))],
            ),
            None,
        )],
    );
    assert!(too_few_tuple.is_err());
    assert!(too_few_tuple.unwrap_err().message.contains("operand count"));

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let too_many_array = catch_unwind(AssertUnwindSafe(|| {
        compile_test_function_result(
            &mut codegen,
            &type_context,
            "bad_array_aggregate",
            Type::Array(Box::new(Type::I64), 1),
            vec![],
            vec![local_decl(
                &type_context,
                Type::Array(Box::new(Type::I64), 1),
                "return_place",
            )],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Aggregate(
                    AggregateKind::Array,
                    vec![
                        Operand::Constant(Constant::Int(1)),
                        Operand::Constant(Constant::Int(2)),
                    ],
                ),
                None,
            )],
        )
    }));
    assert!(too_many_array.is_ok());
    let too_many_array = too_many_array.unwrap();
    assert!(too_many_array.is_err());
    assert!(too_many_array
        .unwrap_err()
        .message
        .contains("operand count"));

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let struct_id = DefId::new(CrateId(0), LocalDefId(30));
    let struct_ty = Type::Struct {
        id: struct_id,
        args: vec![],
    };
    let mut backend_contract = MirBackendContract::default();
    insert_struct_layout_contract(
        &mut backend_contract,
        struct_id,
        vec![("value".to_string(), type_id(&type_context, Type::I64))],
    );
    let too_few_struct = compile_test_function_result_with_contract(
        &mut codegen,
        &type_context,
        "bad_struct_aggregate",
        struct_ty.clone(),
        vec![],
        vec![local_decl(&type_context, struct_ty, "return_place")],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::Aggregate(
                AggregateKind::Struct {
                    id: struct_id,
                    display_name: "OneField".to_string(),
                },
                vec![],
            ),
            None,
        )],
        backend_contract,
    );
    assert!(too_few_struct.is_err());
    assert!(too_few_struct
        .unwrap_err()
        .message
        .contains("operand count"));

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let enum_id = DefId::new(CrateId(0), LocalDefId(31));
    let enum_ty = Type::Enum {
        id: enum_id,
        args: vec![],
    };
    let mut backend_contract = MirBackendContract::default();
    insert_enum_layout_contract(
        &mut backend_contract,
        enum_id,
        vec![
            MirEnumVariantLayout {
                name: "None".to_string(),
                fields: MirVariantLayoutFields::Unit,
            },
            MirEnumVariantLayout {
                name: "Some".to_string(),
                fields: MirVariantLayoutFields::Positional(vec![type_id(&type_context, Type::I64)]),
            },
        ],
    );
    let too_many_enum_payload = compile_test_function_result_with_contract(
        &mut codegen,
        &type_context,
        "bad_enum_aggregate",
        enum_ty.clone(),
        vec![],
        vec![local_decl(&type_context, enum_ty, "return_place")],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::Aggregate(
                AggregateKind::EnumVariant {
                    enum_id,
                    variant_id: VariantId(1),
                    enum_name: "MaybeInt".to_string(),
                    variant_name: "Some".to_string(),
                },
                vec![
                    Operand::Constant(Constant::Int(1)),
                    Operand::Constant(Constant::Int(2)),
                ],
            ),
            None,
        )],
        backend_contract,
    );
    assert!(too_many_enum_payload.is_err());
    assert!(too_many_enum_payload
        .unwrap_err()
        .message
        .contains("operand count"));
}

#[test]
fn mir_codegen_lowers_single_named_enum_payload_as_tuple() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let enum_id = DefId::new(CrateId(0), LocalDefId(32));
    let enum_ty = Type::Enum {
        id: enum_id,
        args: vec![],
    };

    let mut backend_contract = MirBackendContract::default();
    insert_enum_layout_contract(
        &mut backend_contract,
        enum_id,
        vec![
            MirEnumVariantLayout {
                name: "None".to_string(),
                fields: MirVariantLayoutFields::Unit,
            },
            MirEnumVariantLayout {
                name: "Some".to_string(),
                fields: MirVariantLayoutFields::Named(vec![(
                    "value".to_string(),
                    type_id(&type_context, Type::I64),
                )]),
            },
        ],
    );

    compile_test_function_with_contract(
        &mut codegen,
        &type_context,
        "single_named_enum_payload",
        enum_ty.clone(),
        vec![],
        vec![local_decl(&type_context, enum_ty, "return_place")],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::Aggregate(
                AggregateKind::EnumVariant {
                    enum_id,
                    variant_id: VariantId(1),
                    enum_name: "NamedMaybe".to_string(),
                    variant_name: "Some".to_string(),
                },
                vec![Operand::Constant(Constant::Int(77))],
            ),
            None,
        )],
        backend_contract,
    );

    let ir = codegen.get_ir();
    assert_ir_order(
            &ir,
            &[
                "store { i32, i64, { i64 } } { i32 1, i64 undef, { i64 } { i64 77 } }, ptr %return_place",
                "ret { i32, i64, { i64 } }",
            ],
        );
}

#[test]
fn mir_codegen_lowers_aggregates_and_projections() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let struct_id = DefId::new(CrateId(0), LocalDefId(20));
    let enum_id = DefId::new(CrateId(0), LocalDefId(21));
    let single_enum_id = DefId::new(CrateId(0), LocalDefId(22));
    let option_variant = VariantId(1);
    let tuple_ty = Type::Tuple(vec![Type::I64, Type::I64]);
    let array_ty = Type::Array(Box::new(Type::I64), 2);
    let struct_ty = Type::Struct {
        id: struct_id,
        args: vec![],
    };
    let enum_ty = Type::Enum {
        id: enum_id,
        args: vec![],
    };
    let single_enum_ty = Type::Enum {
        id: single_enum_id,
        args: vec![],
    };

    let mut backend_contract = MirBackendContract::default();
    insert_struct_layout_contract(
        &mut backend_contract,
        struct_id,
        vec![
            ("left".to_string(), type_id(&type_context, Type::I64)),
            ("right".to_string(), type_id(&type_context, Type::I64)),
        ],
    );
    insert_enum_layout_contract(
        &mut backend_contract,
        enum_id,
        vec![
            MirEnumVariantLayout {
                name: "None".to_string(),
                fields: MirVariantLayoutFields::Unit,
            },
            MirEnumVariantLayout {
                name: "Some".to_string(),
                fields: MirVariantLayoutFields::Positional(vec![
                    type_id(&type_context, Type::I64),
                    type_id(&type_context, Type::I64),
                ]),
            },
        ],
    );
    insert_enum_layout_contract(
        &mut backend_contract,
        single_enum_id,
        vec![
            MirEnumVariantLayout {
                name: "None".to_string(),
                fields: MirVariantLayoutFields::Unit,
            },
            MirEnumVariantLayout {
                name: "Some".to_string(),
                fields: MirVariantLayoutFields::Positional(vec![type_id(&type_context, Type::I64)]),
            },
        ],
    );

    compile_test_function_with_contract(
        &mut codegen,
        &type_context,
        "aggregates_and_projections",
        Type::I64,
        vec![],
        vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, tuple_ty.clone(), "tuple_value"),
            local_decl(&type_context, struct_ty.clone(), "struct_value"),
            local_decl(&type_context, enum_ty.clone(), "enum_value"),
            local_decl(&type_context, Type::I64, "projected"),
            local_decl(&type_context, Type::I32, "discriminant"),
            local_decl(&type_context, array_ty.clone(), "array_value"),
            local_decl(&type_context, Type::I64, "index"),
            local_decl(&type_context, single_enum_ty.clone(), "single_enum_value"),
            local_decl(&type_context, Type::I64, "single_projected"),
        ],
        vec![
            StatementData::assign(
                root_place(1),
                Rvalue::Aggregate(
                    AggregateKind::Tuple,
                    vec![
                        Operand::Constant(Constant::Int(11)),
                        Operand::Constant(Constant::Int(22)),
                    ],
                ),
                None,
            ),
            StatementData::assign(
                root_place(2),
                Rvalue::Aggregate(
                    AggregateKind::Struct {
                        id: struct_id,
                        display_name: "Pair".to_string(),
                    },
                    vec![
                        Operand::Constant(Constant::Int(33)),
                        Operand::Copy(field_place(1, 1)),
                    ],
                ),
                None,
            ),
            StatementData::assign(
                root_place(3),
                Rvalue::Aggregate(
                    AggregateKind::EnumVariant {
                        enum_id,
                        variant_id: option_variant,
                        enum_name: "MaybePair".to_string(),
                        variant_name: "Some".to_string(),
                    },
                    vec![
                        Operand::Constant(Constant::Int(44)),
                        Operand::Copy(field_place(2, 1)),
                    ],
                ),
                None,
            ),
            StatementData::assign(root_place(5), Rvalue::Discriminant(root_place(3)), None),
            StatementData::assign(
                root_place(6),
                Rvalue::Aggregate(
                    AggregateKind::Array,
                    vec![
                        Operand::Copy(downcast_field_place(3, option_variant, 1)),
                        Operand::Constant(Constant::Int(55)),
                    ],
                ),
                None,
            ),
            StatementData::assign(
                root_place(7),
                Rvalue::Use(Operand::Constant(Constant::Int(0))),
                None,
            ),
            StatementData::assign(
                root_place(4),
                Rvalue::Use(Operand::Copy(index_place(6, 7))),
                None,
            ),
            StatementData::assign(
                root_place(8),
                Rvalue::Aggregate(
                    AggregateKind::EnumVariant {
                        enum_id: single_enum_id,
                        variant_id: option_variant,
                        enum_name: "MaybeInt".to_string(),
                        variant_name: "Some".to_string(),
                    },
                    vec![Operand::Constant(Constant::Int(66))],
                ),
                None,
            ),
            StatementData::assign(
                root_place(9),
                Rvalue::Use(Operand::Copy(downcast_field_place(8, option_variant, 0))),
                None,
            ),
            StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Copy(root_place(4))),
                None,
            ),
        ],
        backend_contract,
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("insertvalue"));
    assert!(ir.contains("extractvalue"));
    assert!(ir.contains("getelementptr"));
    assert!(ir.contains("i32 1"));
    assert!(ir.contains("insertvalue { i32, i64, { i64, i64 } }"));
    assert!(ir.contains("extractvalue { i32, i64, { i64, i64 } }"));
    assert_ir_order_ignoring_aggregate_gep_index_widths(
        &ir,
        &[
            "store { i64, i64 } { i64 11, i64 22 }, ptr %tuple_value",
            "%mir_field = getelementptr inbounds { i64, i64 }, ptr %tuple_value, i32 0, i32 1",
            "%mir_place = load i64, ptr %mir_field",
        ],
    );
    assert_ir_order_ignoring_aggregate_gep_index_widths(
        &ir,
        &[
            "%struct_field = insertvalue { i64, i64 } { i64 33, i64 undef }, i64 %mir_place, 1",
            "store { i64, i64 } %struct_field, ptr %struct_value",
            "%mir_field1 = getelementptr inbounds { i64, i64 }, ptr %struct_value, i32 0, i32 1",
            "%mir_place2 = load i64, ptr %mir_field1",
        ],
    );
    assert_ir_order_ignoring_aggregate_gep_index_widths(
            &ir,
            &[
                "%payload_field = insertvalue { i64, i64 } { i64 44, i64 undef }, i64 %mir_place2, 1",
                "%payload = insertvalue { i32, i64, { i64, i64 } } { i32 1, i64 undef, { i64, i64 } undef }, { i64, i64 } %payload_field, 2",
                "store { i32, i64, { i64, i64 } } %payload, ptr %enum_value",
            ],
        );
    assert_ir_order_ignoring_aggregate_gep_index_widths(
        &ir,
        &[
            "%mir_place3 = load { i32, i64, { i64, i64 } }, ptr %enum_value",
            "%discriminant4 = extractvalue { i32, i64, { i64, i64 } } %mir_place3, 0",
            "store i32 %discriminant4, ptr %discriminant",
        ],
    );
    assert_ir_order_ignoring_aggregate_gep_index_widths(
            &ir,
            &[
                "%mir_variant_payload = getelementptr inbounds { i32, i64, { i64, i64 } }, ptr %enum_value, i32 0, i32 2",
                "%mir_field5 = getelementptr inbounds { i64, i64 }, ptr %mir_variant_payload, i32 0, i32 1",
                "%mir_place6 = load i64, ptr %mir_field5",
            ],
        );
    assert_ir_order_ignoring_aggregate_gep_index_widths(
        &ir,
        &[
            "%array_elem = insertvalue [2 x i64] undef, i64 %mir_place6, 0",
            "%array_elem7 = insertvalue [2 x i64] %array_elem, i64 55, 1",
            "store [2 x i64] %array_elem7, ptr %array_value",
            "%mir_index_ptr = getelementptr [2 x i64], ptr %array_value, i64 0, i64 %mir_index",
            "%mir_place8 = load i64, ptr %mir_index_ptr",
        ],
    );
    assert_ir_order_ignoring_aggregate_gep_index_widths(
            &ir,
            &[
                "store { i32, i64, i64 } { i32 1, i64 undef, i64 66 }, ptr %single_enum_value",
                "%mir_variant_payload9 = getelementptr inbounds { i32, i64, i64 }, ptr %single_enum_value, i32 0, i32 2",
                "%mir_place10 = load i64, ptr %mir_variant_payload9",
                "store i64 %mir_place10, ptr %single_projected",
            ],
        );
}

#[test]
fn mir_codegen_lowers_repeated_array_as_fill_loop() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let array_ty = Type::Array(Box::new(Type::I64), 256);

    compile_test_function_with_contract(
        &mut codegen,
        &type_context,
        "repeat_array",
        Type::I64,
        vec![],
        vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, Type::I64, "element"),
            local_decl(&type_context, array_ty, "array"),
        ],
        vec![
            StatementData::assign(
                root_place(1),
                Rvalue::Use(Operand::Constant(Constant::Int(7))),
                None,
            ),
            StatementData::assign(
                root_place(2),
                Rvalue::Aggregate(
                    AggregateKind::Array,
                    vec![Operand::Copy(root_place(1)); 256],
                ),
                None,
            ),
            StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Constant(Constant::Int(0))),
                None,
            ),
        ],
        MirBackendContract::default(),
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("array_repeat_loop"));
    assert!(ir.contains("array_repeat_element"));
    assert!(!ir.contains("insertvalue [256 x i64]"));
}

#[test]
fn mir_codegen_lowers_enum_discriminant_rvalue() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let enum_id = DefId::new(CrateId(0), LocalDefId(10));
    let enum_ty = Type::Enum {
        id: enum_id,
        args: vec![],
    };
    let mut backend_contract = MirBackendContract::default();
    insert_enum_layout_contract(
        &mut backend_contract,
        enum_id,
        vec![
            MirEnumVariantLayout {
                name: "None".to_string(),
                fields: MirVariantLayoutFields::Unit,
            },
            MirEnumVariantLayout {
                name: "Some".to_string(),
                fields: MirVariantLayoutFields::Positional(vec![type_id(&type_context, Type::I64)]),
            },
        ],
    );

    compile_test_function_with_contract(
        &mut codegen,
        &type_context,
        "enum_discriminant",
        Type::I64,
        vec![enum_ty.clone()],
        vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, enum_ty, "value"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::Discriminant(root_place(1)),
            None,
        )],
        backend_contract,
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("extractvalue"));
    assert!(ir.contains("discriminant"));
}
