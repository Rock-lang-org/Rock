use super::*;

pub(super) fn generic_params(owner: DefId, names: &[&str]) -> Vec<GenericParamDecl> {
    GenericParamDecl::type_params(owner, names.iter().copied())
}

pub(super) fn products_from_hir(hir: &ResolvedHirProgram) -> Result<CompilerProducts, String> {
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

pub(super) fn rebuild_program(
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

pub(super) fn mutate_program(
    program: &mut crate::hir::AcceptedHirProgram,
    mutate: impl FnOnce(&mut HirProgramFor<AcceptedHir>),
) {
    let mut mutated = program.program().clone();
    mutate(&mut mutated);
    *program = crate::hir::AcceptedHirProgram::revalidate_for_test(mutated)
        .expect("mutated product fixture must satisfy accepted HIR invariants");
}

pub(super) fn int_body() -> HirBlock {
    HirBlock {
        stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
            kind: crate::hir::HirExprKindFor::IntLiteral(1),
            ty: Type::I64,
            span: Span::test(),
        })],
        ty: Type::I64,
    }
}

pub(super) fn empty_body(ty: Type) -> HirBlock {
    HirBlock {
        stmts: Vec::new(),
        ty,
    }
}

pub(super) fn test_function(
    id: DefId,
    name: &str,
    generic_params: Vec<GenericParamDecl>,
) -> HirFunction {
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

pub(super) fn accept_function(function: HirFunction) -> AcceptedHirFunction {
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

pub(super) fn local_def_ids_after(next_raw: u32) -> IdGen<LocalDefId> {
    let mut ids = IdGen::new();
    for _ in 0..next_raw {
        ids.fresh();
    }
    ids
}

pub(super) fn resolved_hir_for_products() -> ResolvedHirProgram {
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
