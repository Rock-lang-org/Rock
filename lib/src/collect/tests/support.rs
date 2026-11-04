use super::*;

pub(super) fn type_inner(name: &str) -> ParseTypeInner {
    ParseTypeInner {
        name: name.to_string(),
        generics: vec![],
        span: Span::test(),
    }
}

pub(super) fn generic_type_inner(name: &str, generics: Vec<ParseType>) -> ParseTypeInner {
    ParseTypeInner {
        name: name.to_string(),
        generics,
        span: Span::test(),
    }
}

pub(super) fn generic_type(name: &str, generics: Vec<ParseType>) -> ParseType {
    ParseType::Type(generic_type_inner(name, generics))
}

pub(super) fn struct_field(name: &str, ty: ParseType) -> crate::ast::StructDeclField {
    crate::ast::StructDeclField {
        name: ident(name),
        ty,
        public: false,
        default: None,
    }
}

pub(super) fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: Span::test(),
    }
}

pub(super) fn program_with_struct(name: &str) -> Program {
    Program {
        module: Module {
            name: None,
            top_levels: vec![TopLevel::StructDecl(StructDecl {
                name: type_inner(name),
                generic_params: vec![],
                fields: vec![],
                exported: false,
            })],
            is_inline: false,
            filepath: None,
        },
    }
}

pub(super) fn inline_module(name: &str, top_levels: Vec<TopLevel>) -> TopLevel {
    TopLevel::Module(ModuleDecl(Module {
        name: Some(ident(name)),
        top_levels,
        is_inline: true,
        filepath: None,
    }))
}

pub(super) fn load_source_program(
    entry: PathBuf,
    source: &str,
    current_crate_name: Option<&str>,
) -> (Program, crate::source_loader::ModuleGraph) {
    std::fs::write(&entry, source).unwrap();
    let config = crate::Config {
        entry_file: entry.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: current_crate_name.map(ToString::to_string),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(entry, &config).unwrap();
    let program = Program {
        module: graph.root_module().clone(),
    };
    (program, graph)
}

pub(super) fn function_decl_with_body_expr(
    name: &str,
    expr: crate::ast::Expression,
) -> crate::ast::FunctionDecl {
    crate::ast::FunctionDecl {
        name: ident(name),
        lambda: crate::ast::LambdaDecl {
            parameters: vec![],
            body: crate::ast::Block {
                statements: vec![crate::ast::Statement::Expression(expr)],
            },
            arrow_kind: crate::ast::LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        },
        self_receiver: None,
        is_unsafe: false,
        exported: false,
    }
}

pub(super) fn function_decl(name: &str) -> FunctionDecl {
    FunctionDecl {
        name: ident(name),
        lambda: LambdaDecl {
            parameters: vec![],
            body: crate::ast::Block { statements: vec![] },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        },
        self_receiver: None,
        is_unsafe: false,
        exported: false,
    }
}

pub(super) fn ident_pattern(name: &str) -> Pattern {
    Pattern {
        binding: None,
        kind: PatternKind::Ident(crate::ast::IdentPattern {
            name: ident(name),
            mut_: false,
        }),
    }
}

pub(super) fn generic_param_decl(name: &str) -> crate::ast::GenericParamDecl {
    crate::ast::GenericParamDecl {
        default: None,
        name: ident(name),
        kind: None,
        span: crate::lexer::Span::test(),
    }
}

pub(super) fn function_sig(name: &str, sig: ParseType) -> FunctionSig {
    FunctionSig {
        name: ident(name),
        sig,
        where_clauses: vec![],
        self_receiver: None,
        is_unsafe: false,
        exported: false,
    }
}

pub(super) fn single_param_function_decl(name: &str, param: &str) -> FunctionDecl {
    FunctionDecl {
        name: ident(name),
        lambda: LambdaDecl {
            parameters: vec![ident_pattern(param)],
            body: crate::ast::Block { statements: vec![] },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        },
        self_receiver: None,
        is_unsafe: false,
        exported: false,
    }
}

pub(super) fn generic_declaration_program() -> Program {
    let generic_t = || ParseType::Type(type_inner("T"));
    let inner_t = || generic_type("Inner", vec![generic_t()]);
    let outer_t = || generic_type("Outer", vec![generic_t()]);
    let choice_inner_t = || generic_type("Choice", vec![inner_t()]);
    let choice_outer_t = || generic_type("Choice", vec![outer_t()]);

    Program {
        module: Module {
            name: None,
            top_levels: vec![
                TopLevel::StructDecl(StructDecl {
                    name: generic_type_inner("Inner", vec![]),
                    generic_params: vec![generic_param_decl("T")],
                    fields: vec![struct_field("value", generic_t())],
                    exported: false,
                }),
                TopLevel::EnumDecl(crate::ast::EnumDecl {
                    name: generic_type_inner("Choice", vec![]),
                    generic_params: vec![generic_param_decl("T")],
                    variants: vec![crate::ast::EnumVariant {
                        name: type_inner("Some"),
                        fields: crate::ast::NamedFieldsOrTypesList::TypesList(vec![inner_t()]),
                    }],
                    exported: false,
                    language_items: Default::default(),
                }),
                TopLevel::StructDecl(StructDecl {
                    name: generic_type_inner("Outer", vec![]),
                    generic_params: vec![generic_param_decl("T")],
                    fields: vec![struct_field("choice", choice_inner_t())],
                    exported: false,
                }),
                TopLevel::TraitDecl(crate::ast::TraitDecl {
                    where_clauses: Vec::new(),
                    name: generic_type_inner("Mapper", vec![]),
                    generic_params: vec![generic_param_decl("T")],
                    for_: None,
                    associated_types: vec![],
                    methods: HashMap::new(),
                    signatures: HashMap::from([(
                        ident("map"),
                        function_sig(
                            "map",
                            ParseType::Function(vec![outer_t(), choice_outer_t()]),
                        ),
                    )]),
                    exported: false,
                    language_items: Default::default(),
                }),
                TopLevel::FunctionSig(function_sig(
                    "identity",
                    ParseType::Function(vec![outer_t(), choice_outer_t()]),
                )),
                TopLevel::FunctionDecl(single_param_function_decl("identity", "value")),
                TopLevel::Impl(Impl {
                    name: ParseType::Type(generic_type_inner("Outer", vec![generic_t()])),
                    for_: None,
                    associated_types: vec![],
                    methods: HashMap::from([(ident("new"), function_decl("new"))]),
                    signatures: HashMap::new(),
                    where_clauses: vec![],
                }),
            ],
            is_inline: false,
            filepath: None,
        },
    }
}

pub(super) fn assert_generic_param_descriptor(
    params: &[GenericParamDecl],
    owner: DefId,
    index: u32,
    name: &str,
) {
    assert_eq!(
        params,
        &[GenericParamDecl::new(
            GenericParamId { owner, index },
            name,
            Kind::Type,
        )]
    );
}

pub(super) fn assert_generic_declaration_ids(decls: &Declarations) {
    let root_module_id = decls.indexing_ids.root_module_id();
    let item_id = |index| {
        decls
            .item_index
            .item_at_source(root_module_id, index)
            .unwrap()
            .def_id
    };
    let inner_id = item_id(0);
    let choice_id = item_id(1);
    let outer_id = item_id(2);
    let mapper_id = item_id(3);
    let function_id = item_id(5);
    let impl_id = item_id(6);

    let inner = decls.items.structs().get(&inner_id).unwrap();
    let choice = decls.items.enums().get(&choice_id).unwrap();
    let outer = decls.items.structs().get(&outer_id).unwrap();
    let mapper = decls.items.traits().get(&mapper_id).unwrap();
    let function = decls.items.functions().get(&function_id).unwrap();
    let impl_ = decls.items.impls().get(&impl_id).unwrap();

    assert_eq!(inner.id, inner_id);
    assert_eq!(choice.id, choice_id);
    assert_eq!(outer.id, outer_id);
    assert_eq!(mapper.id, mapper_id);
    assert_eq!(function.id, function_id);
    assert_eq!(impl_.id, impl_id);

    assert_generic_param_descriptor(&inner.generic_params, inner_id, 0, "T");
    assert_generic_param_descriptor(&choice.generic_params, choice_id, 0, "T");
    assert_generic_param_descriptor(&outer.generic_params, outer_id, 0, "T");
    assert_generic_param_descriptor(&mapper.generic_params, mapper_id, 0, "T");
    assert_generic_param_descriptor(&function.generic_params, function_id, 0, "T");
    assert_generic_param_descriptor(&impl_.type_generics, impl_id, 0, "T");
    assert!(mapper
        .signatures
        .get("map")
        .expect("generic trait method signature should be collected")
        .generic_params
        .is_empty());

    assert_eq!(inner.generic_params[0].name, choice.generic_params[0].name);
    assert_ne!(inner.generic_params[0], choice.generic_params[0]);
    assert_ne!(inner.generic_params[0].id, choice.generic_params[0].id);

    let crate::hir::HirVariantFields::Positional(choice_fields) = &choice.variants[0].fields else {
        panic!("generic choice variant should retain its positional payload");
    };
    assert_eq!(
        choice_fields,
        &[Type::Struct {
            id: inner_id,
            args: vec![Type::Generic(GenericParamId {
                owner: choice_id,
                index: 0,
            })],
        }]
    );
    assert_eq!(
        outer.fields[0].ty,
        Type::Enum {
            id: choice_id,
            args: vec![Type::Struct {
                id: inner_id,
                args: vec![Type::Generic(GenericParamId {
                    owner: outer_id,
                    index: 0,
                })],
            }],
        }
    );
    assert_eq!(
        impl_.receiver_pattern,
        crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
            id: outer_id,
            args: vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })],
        })
    );
    assert!(decls.current_def_ids.contains(&mapper.signatures["map"].id));
    assert!(decls.current_def_ids.contains(&impl_.methods["new"].id));
}

pub(super) fn hir_function(name: &str) -> HirFunction {
    HirFunction {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        name: name.to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: vec![],
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

pub(super) fn test_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: vec![],
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

pub(super) fn hir_trait(id: DefId, name: &str, method_names: &[&str]) -> HirTrait {
    HirTrait {
        target: None,
        predicates: Vec::new(),
        id,
        name: name.to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: method_names
            .iter()
            .map(|method_name| (method_name.to_string(), hir_function(method_name)))
            .collect(),
        signatures: HashMap::new(),
    }
}

pub(super) fn inherent_impl(type_name: &str, method_name: &str) -> Impl {
    Impl {
        name: ParseType::Type(type_inner(type_name)),
        for_: None,
        associated_types: vec![],
        methods: HashMap::from([(ident(method_name), function_decl(method_name))]),
        signatures: HashMap::new(),
        where_clauses: vec![],
    }
}

pub(super) fn trait_decl(name: &str) -> TopLevel {
    TopLevel::TraitDecl(crate::ast::TraitDecl {
        where_clauses: Vec::new(),
        name: type_inner(name),
        generic_params: vec![],
        for_: None,
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        exported: false,
        language_items: Default::default(),
    })
}

pub(super) fn trait_impl(type_name: &str, trait_name: &str, method_name: &str) -> Impl {
    Impl {
        name: ParseType::Type(type_inner(trait_name)),
        for_: Some(crate::ast::ParseType::Type(type_inner(type_name))),
        associated_types: vec![],
        methods: HashMap::from([(ident(method_name), function_decl(method_name))]),
        signatures: HashMap::new(),
        where_clauses: vec![],
    }
}

pub(super) fn is_invalid_current_crate_id(id: DefId) -> bool {
    id.crate_id == CrateId(u32::MAX)
}

pub(super) fn function_named<'a>(decls: &'a Declarations, name: &str) -> Option<&'a HirFunction> {
    decls.items.functions().values().find(|function| {
        decls
            .resolver
            .item_names_by_id
            .get(&function.id)
            .map(String::as_str)
            == Some(name)
            || function.name == name
    })
}

pub(super) fn function_sig_named<'a>(
    decls: &'a Declarations,
    name: &str,
) -> Option<&'a HirFunctionSig> {
    decls.items.function_sigs().values().find(|signature| {
        decls
            .resolver
            .item_names_by_id
            .get(&signature.id)
            .map(String::as_str)
            == Some(name)
            || signature.name == name
    })
}

pub(super) fn struct_named<'a>(decls: &'a Declarations, name: &str) -> Option<&'a HirStruct> {
    decls.items.structs().values().find(|structure| {
        decls
            .resolver
            .item_names_by_id
            .get(&structure.id)
            .map(String::as_str)
            == Some(name)
            || structure.name == name
    })
}

pub(super) fn trait_named<'a>(decls: &'a Declarations, name: &str) -> Option<&'a HirTrait> {
    decls.items.traits().values().find(|trait_def| {
        decls
            .resolver
            .item_names_by_id
            .get(&trait_def.id)
            .map(String::as_str)
            == Some(name)
            || trait_def.name == name
    })
}

pub(super) fn assert_no_invalid_current_crate_ids(decls: &Declarations) {
    for function in decls.items.functions().values() {
        let name = &function.name;
        assert!(
            !is_invalid_current_crate_id(function.id),
            "function {name} kept invalid current-crate id {:?}",
            function.id
        );
        for generic in &function.generic_params {
            let generic_id = generic.id;
            assert!(
                !is_invalid_current_crate_id(generic_id.owner),
                "function {name} generic {:?} kept invalid owner",
                generic_id
            );
        }
    }

    for sig in decls.items.function_sigs().values() {
        let name = &sig.name;
        assert!(
            !is_invalid_current_crate_id(sig.id),
            "signature {name} kept invalid current-crate id {:?}",
            sig.id
        );
        for generic in &sig.generic_params {
            let generic_id = generic.id;
            assert!(
                !is_invalid_current_crate_id(generic_id.owner),
                "signature {name} generic {:?} kept invalid owner",
                generic_id
            );
        }
    }

    for strukt in decls.items.structs().values() {
        let name = &strukt.name;
        assert!(
            !is_invalid_current_crate_id(strukt.id),
            "struct {name} kept invalid current-crate id {:?}",
            strukt.id
        );
    }

    for enum_ in decls.items.enums().values() {
        let name = &enum_.name;
        assert!(
            !is_invalid_current_crate_id(enum_.id),
            "enum {name} kept invalid current-crate id {:?}",
            enum_.id
        );
    }

    for trait_def in decls.items.traits().values() {
        let name = &trait_def.name;
        assert!(
            !is_invalid_current_crate_id(trait_def.id),
            "trait {name} kept invalid current-crate id {:?}",
            trait_def.id
        );
        for (method_name, method) in &trait_def.methods {
            assert!(
                !is_invalid_current_crate_id(method.id),
                "trait method {name}.{method_name} kept invalid id {:?}",
                method.id
            );
        }
        for (sig_name, sig) in &trait_def.signatures {
            assert!(
                !is_invalid_current_crate_id(sig.id),
                "trait signature {name}.{sig_name} kept invalid id {:?}",
                sig.id
            );
            for generic in &sig.generic_params {
                let generic_id = generic.id;
                assert!(
                    !is_invalid_current_crate_id(generic_id.owner),
                    "trait signature {name}.{sig_name} generic {:?} kept invalid owner",
                    generic_id
                );
            }
        }
    }

    for imp in decls.items.impls().values() {
        assert!(
            !is_invalid_current_crate_id(imp.id),
            "impl for {} kept invalid id {:?}",
            imp.type_name,
            imp.id
        );
        for (method_name, method) in &imp.methods {
            assert!(
                !is_invalid_current_crate_id(method.id),
                "impl method {}.{} kept invalid id {:?}",
                imp.type_name,
                method_name,
                method.id
            );
        }
    }

    for ext in decls.items.externs().values() {
        assert!(
            !is_invalid_current_crate_id(ext.id),
            "extern {} kept invalid id {:?}",
            ext.name,
            ext.id
        );
    }
}

pub(super) fn add_artifact_extern_crate(
    crate_ctx: &mut CrateContext,
    crate_name: &str,
    interface: ArtifactCrateInterface,
    resolver: crate::collect::resolver::ResolverTables,
    prelude_export_ids: BTreeMap<String, ArtifactExport>,
) {
    crate_ctx
        .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
            CrateId(1),
            crate_name.to_string(),
            crate::crate_system::ExternCrateMetadata::new(interface, resolver, prelude_export_ids),
            crate::crate_system::ExternCrateBodies::default(),
            crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .unwrap();
}

pub(super) fn parse_language_item_source(source: &str) -> Program {
    crate::parser::parse_string(source, &crate::Config::default())
        .expect("language item source should parse")
}

pub(super) fn collect_language_item_program(
    program: &Program,
) -> Result<Declarations, Vec<ResolveError>> {
    collect(&program, &CrateContext::new(), false, Some("test"))
}

pub(super) fn marker_span_bounds(
    program: &Program,
    top_level_index: usize,
    member_index: Option<usize>,
) -> (usize, usize) {
    let markers = match &program.module.top_levels[top_level_index] {
        TopLevel::TraitDecl(trait_decl) => &trait_decl.language_items,
        TopLevel::EnumDecl(enum_decl) => &enum_decl.language_items,
        _ => panic!("expected marked trait or enum"),
    };
    let span = match member_index {
        Some(member_index) => &markers.members[member_index].marker.span,
        None => &markers.root.as_ref().expect("expected root marker").span,
    };
    (span.start, span.end)
}

pub(super) fn error_details(errors: Vec<ResolveError>) -> Vec<(String, Option<(usize, usize)>)> {
    errors
        .into_iter()
        .map(|error| {
            let span = error.span().map(|span| (span.start, span.end));
            (error.message, span)
        })
        .collect()
}

pub(super) struct CurrentCrateCanonicalFixture {
    pub(super) program: ast::Program,
    pub(super) graph: crate::source_loader::ModuleGraph,
    temp_dir: PathBuf,
}

impl Drop for CurrentCrateCanonicalFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

pub(super) fn current_crate_canonical_fixture() -> CurrentCrateCanonicalFixture {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_canonical_paths_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let root_path = temp_dir.join("main.rk");
    let io_path = temp_dir.join("io.rk");
    std::fs::write(&root_path, "mod io\n").unwrap();
    std::fs::write(&io_path, "struct Writer\n").unwrap();
    let config = crate::Config {
        entry_file: root_path.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("test".to_string()),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(root_path.clone(), &config).unwrap();

    CurrentCrateCanonicalFixture {
        program: ast::Program {
            module: ast::Module {
                name: None,
                top_levels: vec![
                    ast::TopLevel::StructDecl(ast::StructDecl {
                        name: ast::ParseTypeInner {
                            name: "RootThing".to_string(),
                            generics: vec![],
                            span: crate::lexer::Span::test(),
                        },
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    ast::TopLevel::Module(ast::ModuleDecl(ast::Module {
                        name: Some(ast::Ident {
                            name: "math".to_string(),
                            span: crate::lexer::Span::test(),
                        }),
                        top_levels: vec![ast::TopLevel::StructDecl(ast::StructDecl {
                            name: ast::ParseTypeInner {
                                name: "Vector".to_string(),
                                generics: vec![],
                                span: crate::lexer::Span::test(),
                            },
                            generic_params: vec![],
                            fields: vec![],
                            exported: false,
                        })],
                        is_inline: true,
                        filepath: None,
                    })),
                    ast::TopLevel::Mod(
                        ast::Ident {
                            name: "io".to_string(),
                            span: crate::lexer::Span::test(),
                        },
                        false,
                    ),
                ],
                is_inline: false,
                filepath: Some(root_path),
            },
        },
        graph,
        temp_dir,
    }
}
