use super::*;

#[test]
fn build_monomorphized_lowers_trait_impl_projection_contract() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(20));
    let trait_id = DefId::new(CrateId(0), LocalDefId(21));
    let method_id = DefId::new(CrateId(0), LocalDefId(22));
    let function_id = DefId::new(CrateId(0), LocalDefId(23));
    let instance_id = InstanceId(0);
    let assoc_type_id = AssocTypeId(0);
    let base_ty = Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Value(3));
    let projection_ty = Type::Projection {
        ty: Box::new(base_ty.clone()),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: vec![Type::I64],
    };
    let method = empty_function(method_id, "index");
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::BuiltinSlice,
        type_name: "[T; 3]".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: HirImplReceiverPattern::SliceFamily { element: Type::I64 },
        trait_name: Some("Index".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: vec![Type::I64],
        associated_types: vec![HirAssociatedTypeDef {
            id: assoc_type_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::Bool,
        }],
        bounds: std::collections::HashMap::new().into(),
        methods: std::collections::HashMap::from([("index".to_string(), method)]),
    };
    let function = HirFunction {
        ret_type: projection_ty.clone(),
        body: HirBlock {
            stmts: Vec::new(),
            ty: projection_ty,
        },
        ..empty_function(function_id, "project")
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(function_id, function.clone())]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(impl_id, imp)]),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([(
                "project".to_string(),
                function_id,
            )]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(function_id, "project".to_string())]),
    );
    let mut type_context = crate::type_context::TypeContext::new();
    let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
    let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
    pre_mir_instance_bodies.insert(instance_id, function);
    let mono = crate::mono::MonomorphizedProgram {
        program,
        instances: std::collections::BTreeMap::from([(
            instance_id,
            crate::mono::InstanceRecord {
                id: instance_id,
                origin: crate::mono::InstanceOrigin::Function(function_id),
                substitution: Vec::new(),
                symbols: crate::mono::InstanceSymbols::new(
                    "project".to_string(),
                    "project".to_string(),
                ),
                declared: None,
                provided_by_object: false,
                is_specialization: false,
            },
        )]),
        pre_mir_instance_bodies,
        generated_drop_instances: Default::default(),
        type_context,
        source_map: crate::source_map::SemanticSourceMap::default(),
    };

    let mir = MirBuilder::build_monomorphized(&mono).unwrap();
    assert!(mir.backend_contract.projection_traits.contains(&trait_id));
    assert!(mir
        .backend_contract
        .projection_outputs
        .iter()
        .any(|(key, output)| {
            mir.type_context.type_for(key.base) == base_ty
                && key.trait_id == trait_id
                && key.assoc_type_id == assoc_type_id
                && key
                    .trait_args
                    .iter()
                    .map(|arg| mir.type_context.type_for(*arg))
                    .collect::<Vec<_>>()
                    == vec![Type::I64]
                && mir.type_context.type_for(*output) == Type::Bool
        }));
}

#[test]
fn projection_provider_refuses_multiple_matching_impl_ids() {
    let trait_id = DefId::new(CrateId(0), LocalDefId(24));
    let generic_impl_id = DefId::new(CrateId(0), LocalDefId(25));
    let concrete_impl_id = DefId::new(CrateId(0), LocalDefId(26));
    let owner_ty = Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(27)),
        args: Vec::new(),
    };
    let imp = |impl_id: DefId, receiver_pattern: HirImplReceiverPattern, output: Type| HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Owner".to_string()),
        type_name: "Owner".to_string(),
        type_generics: Vec::new(),
        receiver_pattern,
        trait_name: Some("Project".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: vec![HirAssociatedTypeDef {
            id: AssocTypeId(0),
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: output,
        }],
        bounds: std::collections::HashMap::new().into(),
        methods: std::collections::HashMap::new(),
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::from([
            (
                generic_impl_id,
                imp(
                    generic_impl_id,
                    HirImplReceiverPattern::Exact(Type::Generic(crate::types::GenericParamId {
                        owner: generic_impl_id,
                        index: 0,
                    })),
                    Type::I64,
                ),
            ),
            (
                concrete_impl_id,
                imp(
                    concrete_impl_id,
                    HirImplReceiverPattern::Exact(owner_ty.clone()),
                    Type::Bool,
                ),
            ),
        ]),
        std::collections::HashMap::new(),
        HirNameTables::default(),
        &std::collections::HashMap::new(),
    );
    let provider = MirProjectionResolutionProvider { program: &program };

    let output = ProjectionProvider::resolve_projection_output(
        &provider,
        &owner_ty,
        trait_id,
        AssocTypeId(0),
        &[],
    );

    assert_eq!(output, None);
}

#[test]
fn build_monomorphized_lowers_projection_output_for_concrete_projection_type() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(30));
    let trait_id = DefId::new(CrateId(0), LocalDefId(31));
    let function_id = DefId::new(CrateId(0), LocalDefId(32));
    let assoc_type_id = AssocTypeId(0);
    let base_ty = Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Value(3));
    let projection_ty = Type::Projection {
        ty: Box::new(base_ty.clone()),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: vec![Type::I64],
    };
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::BuiltinSlice,
        type_name: "[T; 3]".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: HirImplReceiverPattern::SliceFamily { element: Type::I64 },
        trait_name: Some("Index".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: vec![Type::I64],
        associated_types: vec![HirAssociatedTypeDef {
            id: assoc_type_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::Bool,
        }],
        bounds: std::collections::HashMap::new().into(),
        methods: std::collections::HashMap::new(),
    };
    let function = HirFunction {
        ret_type: projection_ty.clone(),
        body: HirBlock {
            stmts: Vec::new(),
            ty: projection_ty.clone(),
        },
        ..empty_function(function_id, "project")
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(function_id, function.clone())]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(impl_id, imp)]),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([(
                "project".to_string(),
                function_id,
            )]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(function_id, "project".to_string())]),
    );
    let mut type_context = crate::type_context::TypeContext::new();
    let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
    let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
    pre_mir_instance_bodies.insert(InstanceId(0), function);
    let mono = crate::mono::MonomorphizedProgram {
        program,
        instances: std::collections::BTreeMap::from([(
            InstanceId(0),
            crate::mono::InstanceRecord {
                id: InstanceId(0),
                origin: crate::mono::InstanceOrigin::Function(function_id),
                substitution: Vec::new(),
                symbols: crate::mono::InstanceSymbols::new(
                    "project".to_string(),
                    "project".to_string(),
                ),
                declared: None,
                provided_by_object: false,
                is_specialization: false,
            },
        )]),
        pre_mir_instance_bodies,
        generated_drop_instances: Default::default(),
        type_context,
        source_map: crate::source_map::SemanticSourceMap::default(),
    };

    let mir = MirBuilder::build_monomorphized(&mono).unwrap();

    assert!(mir
        .backend_contract
        .projection_outputs
        .iter()
        .any(
            |(key, output)| mir.type_context.type_for(key.base) == base_ty
                && key.trait_id == trait_id
                && key.assoc_type_id == assoc_type_id
                && key
                    .trait_args
                    .iter()
                    .map(|arg| mir.type_context.type_for(*arg))
                    .collect::<Vec<_>>()
                    == vec![Type::I64]
                && mir.type_context.type_for(*output) == Type::Bool
        ));
}

#[test]
fn projection_output_contract_matches_named_impl_owner() {
    let foo_id = DefId::new(CrateId(0), LocalDefId(33));
    let bar_id = DefId::new(CrateId(0), LocalDefId(34));
    let foo_impl_id = DefId::new(CrateId(0), LocalDefId(35));
    let bar_impl_id = DefId::new(CrateId(0), LocalDefId(36));
    let trait_id = DefId::new(CrateId(0), LocalDefId(37));
    let function_id = DefId::new(CrateId(0), LocalDefId(38));
    let assoc_type_id = AssocTypeId(0);
    let bar_ty = Type::Struct {
        id: bar_id,
        args: Vec::new(),
    };
    let projection_ty = Type::Projection {
        ty: Box::new(bar_ty.clone()),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: Vec::new(),
    };
    let foo_impl = HirImpl {
        id: foo_impl_id,
        owner: HirImplOwner::Named("Foo".to_string()),
        type_name: "Foo".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: foo_id,
            args: Vec::new(),
        }),
        trait_name: Some("Project".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: vec![HirAssociatedTypeDef {
            id: assoc_type_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::Bool,
        }],
        bounds: std::collections::HashMap::new().into(),
        methods: std::collections::HashMap::new(),
    };
    let bar_impl = HirImpl {
        id: bar_impl_id,
        owner: HirImplOwner::Named("Bar".to_string()),
        type_name: "Bar".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: bar_id,
            args: Vec::new(),
        }),
        trait_name: Some("Project".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: vec![HirAssociatedTypeDef {
            id: assoc_type_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::U8,
        }],
        bounds: std::collections::HashMap::new().into(),
        methods: std::collections::HashMap::new(),
    };
    let function = HirFunction {
        ret_type: projection_ty.clone(),
        body: HirBlock {
            stmts: Vec::new(),
            ty: projection_ty,
        },
        ..empty_function(function_id, "project_bar")
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(function_id, function.clone())]),
        std::collections::HashMap::from([
            (
                foo_id,
                HirStruct {
                    id: foo_id,
                    name: "Foo".to_string(),
                    generic_params: Vec::new(),
                    fields: Vec::new(),
                },
            ),
            (
                bar_id,
                HirStruct {
                    id: bar_id,
                    name: "Bar".to_string(),
                    generic_params: Vec::new(),
                    fields: Vec::new(),
                },
            ),
        ]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(foo_impl_id, foo_impl), (bar_impl_id, bar_impl)]),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([(
                "project_bar".to_string(),
                function_id,
            )]),
            structs_by_name: std::collections::HashMap::from([
                ("Foo".to_string(), foo_id),
                ("Bar".to_string(), bar_id),
            ]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([
            (function_id, "project_bar".to_string()),
            (foo_id, "Foo".to_string()),
            (bar_id, "Bar".to_string()),
        ]),
    );
    let mut type_context = crate::type_context::TypeContext::new();
    let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
    let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
    pre_mir_instance_bodies.insert(InstanceId(0), function);
    let mono = crate::mono::MonomorphizedProgram {
        program,
        instances: std::collections::BTreeMap::from([(
            InstanceId(0),
            crate::mono::InstanceRecord {
                id: InstanceId(0),
                origin: crate::mono::InstanceOrigin::Function(function_id),
                substitution: Vec::new(),
                symbols: crate::mono::InstanceSymbols::new(
                    "project_bar".to_string(),
                    "project_bar".to_string(),
                ),
                declared: None,
                provided_by_object: false,
                is_specialization: false,
            },
        )]),
        pre_mir_instance_bodies,
        generated_drop_instances: Default::default(),
        type_context,
        source_map: crate::source_map::SemanticSourceMap::default(),
    };

    let mir = MirBuilder::build_monomorphized(&mono).unwrap();

    assert!(mir
        .backend_contract
        .projection_outputs
        .iter()
        .any(
            |(key, output)| mir.type_context.type_for(key.base) == bar_ty
                && key.trait_id == trait_id
                && key.assoc_type_id == assoc_type_id
                && key.trait_args.is_empty()
                && mir.type_context.type_for(*output) == Type::U8
        ));
    assert!(!mir
        .backend_contract
        .projection_outputs
        .iter()
        .any(
            |(key, output)| mir.type_context.type_for(key.base) == bar_ty
                && key.trait_id == trait_id
                && mir.type_context.type_for(*output) == Type::Bool
        ));
}

#[test]
fn projection_outputs_use_normalized_nested_projection_keys() {
    let inner_impl_id = DefId::new(CrateId(0), LocalDefId(33));
    let inner_trait_id = DefId::new(CrateId(0), LocalDefId(34));
    let outer_impl_id = DefId::new(CrateId(0), LocalDefId(35));
    let outer_trait_id = DefId::new(CrateId(0), LocalDefId(36));
    let assoc_type_id = AssocTypeId(0);
    let function_id = DefId::new(CrateId(0), LocalDefId(38));
    let base_ty = Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(37)),
        args: Vec::new(),
    };
    let inner_projection = Type::Projection {
        ty: Box::new(base_ty.clone()),
        trait_id: inner_trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: inner_trait_id,
            assoc_type_id,
        },
        trait_args: Vec::new(),
    };
    let outer_projection = Type::Projection {
        ty: Box::new(inner_projection.clone()),
        trait_id: outer_trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: outer_trait_id,
            assoc_type_id,
        },
        trait_args: Vec::new(),
    };
    let inner_impl = HirImpl {
        id: inner_impl_id,
        owner: HirImplOwner::Named("InnerOwner".to_string()),
        type_name: "InnerOwner".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(base_ty.clone()),
        trait_name: Some("Inner".to_string()),
        trait_id: Some(inner_trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: vec![HirAssociatedTypeDef {
            id: assoc_type_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::Bool,
        }],
        bounds: std::collections::HashMap::new().into(),
        methods: std::collections::HashMap::new(),
    };
    let outer_impl = HirImpl {
        id: outer_impl_id,
        owner: HirImplOwner::Named("Bool".to_string()),
        type_name: "Bool".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Bool),
        trait_name: Some("Outer".to_string()),
        trait_id: Some(outer_trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: vec![HirAssociatedTypeDef {
            id: assoc_type_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::U8,
        }],
        bounds: std::collections::HashMap::new().into(),
        methods: std::collections::HashMap::new(),
    };
    let base_id = match &base_ty {
        Type::Struct { id, .. } => *id,
        _ => unreachable!(),
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(
            base_id,
            HirStruct {
                id: base_id,
                name: "InnerOwner".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        )]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(inner_impl_id, inner_impl), (outer_impl_id, outer_impl)]),
        std::collections::HashMap::new(),
        HirNameTables {
            structs_by_name: std::collections::HashMap::from([("InnerOwner".to_string(), base_id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(base_id, "InnerOwner".to_string())]),
    );
    let mut type_context = TypeContext::new();
    let ret_type = type_context.intern_type(&outer_projection);
    let mono = crate::mono::MonomorphizedProgram {
        program,
        instances: std::collections::BTreeMap::new(),
        pre_mir_instance_bodies: crate::mono::PreMirInstanceBodies::new(),
        generated_drop_instances: Default::default(),
        type_context: TypeContext::new(),
        source_map: crate::source_map::SemanticSourceMap::default(),
    };
    let functions = std::collections::BTreeMap::from([(
        MirFunctionId::Function(function_id),
        MirFunction {
            id: MirFunctionId::Function(function_id),
            name: "project_nested".to_string(),
            basic_blocks: Vec::new(),
            local_decls: Vec::new(),
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        },
    )]);
    let contract = MirBuilder::backend_contract_for_program(
        &mono,
        &functions,
        &mut type_context,
        &MirInstanceBodies::new(),
    );

    assert!(contract
        .projection_outputs
        .iter()
        .any(|(key, output)| type_context.type_for(key.base) == base_ty
            && key.trait_id == inner_trait_id
            && key.assoc_type_id == assoc_type_id
            && key.trait_args.is_empty()
            && type_context.type_for(*output) == Type::Bool));
    assert!(contract
        .projection_outputs
        .iter()
        .any(
            |(key, output)| type_context.type_for(key.base) == Type::Bool
                && key.trait_id == outer_trait_id
                && key.assoc_type_id == assoc_type_id
                && key.trait_args.is_empty()
                && type_context.type_for(*output) == Type::U8
        ));
    assert!(!contract
        .projection_outputs
        .iter()
        .any(
            |(key, _)| type_context.type_for(key.base) == inner_projection
                && key.trait_id == outer_trait_id
        ));
}

#[test]
fn projection_outputs_use_selected_index_impl() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(39));
    let trait_id = DefId::new(CrateId(0), LocalDefId(40));
    let assoc_type_id = AssocTypeId(0);
    let base_ty = Type::Array(Box::new(Type::U8), crate::types::ArrayLen::Value(4));
    let projection = Type::Projection {
        ty: Box::new(base_ty.clone()),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: vec![Type::I64],
    };
    let index_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::BuiltinSlice,
        type_name: "[U8; 4]".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::SliceFamily { element: Type::U8 },
        trait_name: Some("Index".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: vec![Type::I64],
        associated_types: vec![HirAssociatedTypeDef {
            id: assoc_type_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::U8,
        }],
        bounds: std::collections::HashMap::new().into(),
        methods: std::collections::HashMap::new(),
    };
    let mut type_context = TypeContext::new();
    let ret_type = type_context.intern_type(&projection);
    let function_id = DefId::new(CrateId(0), LocalDefId(41));
    let mono = crate::mono::MonomorphizedProgram {
        program: HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(impl_id, index_impl)]),
            std::collections::HashMap::new(),
            HirNameTables::default(),
            &std::collections::HashMap::new(),
        ),
        instances: std::collections::BTreeMap::new(),
        pre_mir_instance_bodies: crate::mono::PreMirInstanceBodies::new(),
        generated_drop_instances: Default::default(),
        type_context: TypeContext::new(),
        source_map: crate::source_map::SemanticSourceMap::default(),
    };
    let functions = std::collections::BTreeMap::from([(
        MirFunctionId::Function(function_id),
        MirFunction {
            id: MirFunctionId::Function(function_id),
            name: "project_index".to_string(),
            basic_blocks: Vec::new(),
            local_decls: Vec::new(),
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        },
    )]);
    let contract = MirBuilder::backend_contract_for_program(
        &mono,
        &functions,
        &mut type_context,
        &MirInstanceBodies::new(),
    );

    let exact_key = crate::mir::MirProjectionKey {
        base: type_context
            .id_for_type(&base_ty)
            .expect("interned base type"),
        trait_id,
        assoc_type_id,
        trait_args: vec![type_context
            .id_for_type(&Type::I64)
            .expect("interned index type")],
    };
    assert_eq!(
        contract
            .projection_outputs
            .get(&exact_key)
            .map(|output| type_context.type_for(*output)),
        Some(Type::U8)
    );
}

#[test]
fn projection_outputs_include_callable_abi_return_type() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(43));
    let trait_id = DefId::new(CrateId(0), LocalDefId(41));
    let assoc_type_id = AssocTypeId(0);
    let base_ty = Type::Array(Box::new(Type::U8), crate::types::ArrayLen::Value(4));
    let abi_projection = Type::Projection {
        ty: Box::new(base_ty.clone()),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: vec![Type::I64],
    };
    let index_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::BuiltinSlice,
        type_name: "[U8; 4]".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::SliceFamily { element: Type::U8 },
        trait_name: Some("Index".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: vec![Type::I64],
        associated_types: vec![HirAssociatedTypeDef {
            id: assoc_type_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::U8,
        }],
        bounds: std::collections::HashMap::new().into(),
        methods: std::collections::HashMap::new(),
    };
    let mut type_context = TypeContext::new();
    let semantic_ret = type_context.intern_type(&Type::Unit);
    let abi_ret = type_context.intern_type(&abi_projection);
    let mut signature = crate::mir::MirCallableSignature::from_type_ids(
        &[],
        semantic_ret,
        crate::mir::MirPassMode::Direct,
    );
    signature.ret.abi_ty = abi_ret;
    let mut contract = crate::mir::MirBackendContract::default();
    let callable_id = DefId::new(CrateId(0), LocalDefId(42));
    let key = crate::mir::MirCallableKey::Extern(callable_id);
    contract.callables.insert(
        key.clone(),
        crate::mir::MirCallableDecl {
            key,
            source_def_id: Some(callable_id),
            kind: crate::mir::MirCallableKind::Extern {
                link_name: "abi_projection".to_string(),
                variadic: false,
            },
            llvm_symbol: "abi_projection".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature,
        },
    );
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(impl_id, index_impl)]),
        std::collections::HashMap::new(),
        HirNameTables::default(),
        &std::collections::HashMap::new(),
    );

    MirBuilder::populate_backend_contract_projection_outputs(
        &mut contract,
        &program,
        &std::collections::BTreeMap::new(),
        &mut type_context,
    );

    assert!(contract
        .projection_outputs
        .iter()
        .any(|(key, output)| type_context.type_for(key.base) == base_ty
            && key.trait_id == trait_id
            && key.assoc_type_id == assoc_type_id
            && key
                .trait_args
                .iter()
                .map(|arg| type_context.type_for(*arg))
                .collect::<Vec<_>>()
                == vec![Type::I64]
            && type_context.type_for(*output) == Type::U8));
}

#[test]
fn projection_output_type_ids_are_deterministic() {
    let build_contract = || {
        let first_trait_id = DefId::new(CrateId(0), LocalDefId(43));
        let second_trait_id = DefId::new(CrateId(0), LocalDefId(44));
        let first_base = Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(45)),
            args: Vec::new(),
        };
        let second_base = Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(46)),
            args: Vec::new(),
        };
        let projection = |base: Type, trait_id| Type::Projection {
            ty: Box::new(base),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: Vec::new(),
        };
        let first_projection = projection(first_base.clone(), first_trait_id);
        let second_projection = projection(second_base.clone(), second_trait_id);
        let impl_for = |impl_id: DefId, trait_id, base, output| HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Owner".to_string()),
            type_name: "Owner".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(base),
            trait_name: Some("Project".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: vec![HirAssociatedTypeDef {
                id: AssocTypeId(0),
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: output,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: std::collections::HashMap::new(),
        };
        let first_impl_id = DefId::new(CrateId(0), LocalDefId(47));
        let second_impl_id = DefId::new(CrateId(0), LocalDefId(48));
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::from([
                (
                    first_impl_id,
                    impl_for(
                        first_impl_id,
                        first_trait_id,
                        first_base,
                        Type::Tuple(vec![Type::Bool]),
                    ),
                ),
                (
                    second_impl_id,
                    impl_for(
                        second_impl_id,
                        second_trait_id,
                        second_base,
                        Type::Tuple(vec![Type::U8]),
                    ),
                ),
            ]),
            std::collections::HashMap::new(),
            HirNameTables::default(),
            &std::collections::HashMap::new(),
        );
        let mut type_context = TypeContext::new();
        let semantic_ret = type_context.intern_type(&Type::Unit);
        let first_abi_ret = type_context.intern_type(&first_projection);
        let second_abi_ret = type_context.intern_type(&second_projection);
        let mut contract = crate::mir::MirBackendContract::default();

        for (callable_id, abi_ty) in [
            (DefId::new(CrateId(0), LocalDefId(49)), first_abi_ret),
            (DefId::new(CrateId(0), LocalDefId(50)), second_abi_ret),
        ] {
            let mut signature = crate::mir::MirCallableSignature::from_type_ids(
                &[],
                semantic_ret,
                crate::mir::MirPassMode::Direct,
            );
            signature.ret.abi_ty = abi_ty;
            let key = crate::mir::MirCallableKey::Extern(callable_id);
            contract.callables.insert(
                key.clone(),
                crate::mir::MirCallableDecl {
                    key,
                    source_def_id: Some(callable_id),
                    kind: crate::mir::MirCallableKind::Extern {
                        link_name: format!("projection_{}", callable_id.local.0),
                        variadic: false,
                    },
                    llvm_symbol: format!("projection_{}", callable_id.local.0),
                    linkage: crate::mir::MirLinkage::External,
                    signature,
                },
            );
        }

        MirBuilder::populate_backend_contract_projection_outputs(
            &mut contract,
            &program,
            &std::collections::BTreeMap::new(),
            &mut type_context,
        );
        let output_types = contract
            .projection_outputs
            .values()
            .map(|output| type_context.type_for(*output))
            .collect::<Vec<_>>();
        (contract.projection_outputs, output_types)
    };

    let expected = build_contract();
    for _ in 0..32 {
        assert_eq!(build_contract(), expected);
    }
}
