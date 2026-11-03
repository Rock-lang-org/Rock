use std::collections::HashMap;
use std::path::PathBuf;

use crate::collect::resolver::ResolverTables;
use crate::hir::{
    HirAssociatedTypeDecl, HirAssociatedTypeDef, HirBlock, HirClosureCapture,
    HirClosureCaptureKind, HirEnum, HirExpr, HirExprKind, HirFunction, HirFunctionSig, HirImpl,
    HirImplOwner, HirImplReceiverPattern, HirMatchArm, HirMethodCallTarget, HirParam, HirPattern,
    HirStmt, HirStruct, HirStructPatternField, HirTrait,
};
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::types::ReceiverMode;
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, TraitBound, Type};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn empty_trait(id: DefId, name: &str) -> HirTrait {
    HirTrait {
        target: None,
        predicates: Vec::new(),
        id,
        name: name.to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::new(),
    }
}

fn point_struct(id: DefId) -> HirStruct {
    HirStruct {
        id,
        name: "Point".to_string(),
        generic_params: vec![],
        fields: vec![],
    }
}

fn register_point(lowerer: &mut Lowerer, point_id: DefId) {
    lowerer.items.insert_structure(point_struct(point_id));
    lowerer
        .resolver
        .item_paths
        .insert("Point".to_string(), point_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(point_id, "Point".to_string());
    lowerer.current_def_ids.insert(point_id);
}

#[test]
fn trait_conformance_display_context_renders_generic_projection_names() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(80);
    let generic = GenericParamDecl::type_param(
        GenericParamId {
            owner: trait_id,
            index: 0,
        },
        "T",
    );
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "Render".to_string());
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Render".to_string(),
        generic_params: vec![generic],
        associated_types: vec![HirAssociatedTypeDecl {
            id: AssocTypeId(0),
            name: "Item".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
    });

    let service = super::TraitConformanceService::new(super::TraitConformanceContext {
        items: &mut lowerer.items,
        engine: &mut lowerer.engine,
        resolver: &lowerer.resolver,
        dependency_resolvers: &lowerer.dependency_resolvers,
        root_crate_id: lowerer.root_crate_id,
        local_def_ids: &mut lowerer.local_def_ids,
        current_def_ids: &mut lowerer.current_def_ids,
        source_map: &lowerer.source_map,
        index_protocol_ids: None,
    });
    let ty = Type::Projection {
        ty: Box::new(Type::Generic(GenericParamId {
            owner: trait_id,
            index: 0,
        })),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![Type::Generic(GenericParamId {
            owner: trait_id,
            index: 0,
        })],
    };

    assert_eq!(service.display_type(&ty), "<T as Render T>::Item");
}

#[test]
fn index_mut_pair_diagnostic_uses_index_mut_method_span() {
    let mut lowerer = Lowerer::new_for_test();
    let index_trait_id = def_id(40);
    let index_mut_trait_id = def_id(41);
    let index_mut_method_id = def_id(42);
    let impl_id = def_id(43);
    let point_id = def_id(44);
    let span = Span {
        file_path: PathBuf::from("cell.rk"),
        start: 12,
        end: 24,
    };

    lowerer.language_items.index = Some(crate::language_items::IndexLanguageItems {
        trait_id: index_trait_id,
        output_id: AssocTypeId(1),
        method_id: def_id(45),
    });
    lowerer.language_items.index_mut = Some(crate::language_items::IndexMutLanguageItems {
        trait_id: index_mut_trait_id,
        output_id: AssocTypeId(2),
        method_id: index_mut_method_id,
    });
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Point".to_string()),
            type_name: "Point".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: point_id,
                args: Vec::new(),
            }),
            trait_name: Some("IndexMut".to_string()),
            trait_id: Some(index_mut_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::I64],
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([(
                "write_at".to_string(),
                HirFunction {
                    id: index_mut_method_id,
                    name: "write_at".to_string(),
                    generic_params: Vec::new(),
                    generic_bounds: HashMap::new().into(),
                    params: Vec::new(),
                    ret_type: Type::Reference {
                        mutable: true,
                        inner: Box::new(Type::I64),
                    },
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::Unit,
                            ty: Type::Unit,
                            span: span.clone(),
                        })],
                        ty: Type::Unit,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();
    lowerer.source_map.insert_symbol(
        crate::source_map::SourceSymbol::Definition(impl_id),
        span.clone(),
        Some(span.clone()),
        None,
    );

    let output = {
        let mut service = super::TraitConformanceService::new(super::TraitConformanceContext {
            items: &mut lowerer.items,
            engine: &mut lowerer.engine,
            resolver: &lowerer.resolver,
            dependency_resolvers: &lowerer.dependency_resolvers,
            root_crate_id: lowerer.root_crate_id,
            local_def_ids: &mut lowerer.local_def_ids,
            current_def_ids: &mut lowerer.current_def_ids,
            source_map: &lowerer.source_map,
            index_protocol_ids: Some(super::IndexProtocolIds {
                index_trait: index_trait_id,
                index_output: AssocTypeId(1),
                index_mut_trait: index_mut_trait_id,
                index_mut_output: AssocTypeId(2),
            }),
        });
        service.validate_index_mut_pairs();
        service.finish()
    };

    let diagnostic = output
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.message
                    == "IndexMut implementation for Point with key I64 requires a matching Index implementation"
            })
            .expect("missing IndexMut pairing diagnostic");
    assert!(!diagnostic.message.contains("DefId"));
    assert!(!diagnostic.message.contains("TypeVarId"));
    assert_eq!(diagnostic.code, crate::diagnostic::DiagnosticCode::Resolve);
    let diagnostic_span = diagnostic.span().expect("diagnostic should have a span");
    assert_eq!(diagnostic_span.start, span.start);
    assert_eq!(diagnostic_span.end, span.end);
}

#[test]
fn conformance_does_not_resolve_dependency_export_alias_without_import() {
    let mut lowerer = Lowerer::new_for_test();
    let point_id = def_id(30);
    let trait_id = DefId::new(CrateId(7), LocalDefId(31));
    register_point(&mut lowerer, point_id);
    lowerer
        .items
        .insert_trait_def(empty_trait(trait_id, "dep::Show"));

    let mut dep_resolver = ResolverTables::default();
    dep_resolver.item_paths.insert("Show".to_string(), trait_id);
    dep_resolver
        .item_paths
        .insert("dep::Show".to_string(), trait_id);
    dep_resolver
        .item_names_by_id
        .insert(trait_id, "dep::Show".to_string());
    dep_resolver.insert_export_alias_with_name(
        "Show".to_string(),
        "dep::Show".to_string(),
        trait_id,
    );
    lowerer
        .dependency_resolvers
        .insert("dep".to_string(), dep_resolver);
    lowerer
        .items
        .insert_impl(HirImpl {
            id: def_id(32),
            owner: HirImplOwner::Named("Point".to_string()),
            type_name: "Point".to_string(),
            type_generics: vec![],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: point_id,
                args: Vec::new(),
            }),
            trait_name: Some("Show".to_string()),
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert_eq!(lowerer.items.impl_def(def_id(32)).unwrap().trait_id, None);
}

#[test]
fn conformance_requires_resolver_id_for_trait_name() {
    let mut lowerer = Lowerer::new_for_test();
    let point_id = def_id(33);
    let trait_id = def_id(34);
    register_point(&mut lowerer, point_id);
    lowerer
        .items
        .insert_trait_def(empty_trait(trait_id, "Show"));
    lowerer
        .items
        .insert_impl(HirImpl {
            id: def_id(35),
            owner: HirImplOwner::Named("Point".to_string()),
            type_name: "Point".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Show".to_string()),
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert_eq!(lowerer.items.impl_def(def_id(35)).unwrap().trait_id, None);
}

#[test]
fn auto_impl_sized_ignores_local_trait_named_sized() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(10);
    let point_id = def_id(11);
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "Sized"));
    register_point(&mut lowerer, point_id);

    lowerer.auto_impl_sized();

    assert!(lowerer
        .items
        .impl_defs()
        .all(|(_, imp)| imp.trait_id != Some(sized_id)));
}

#[test]
fn auto_impl_sized_uses_marked_trait_id() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(20);
    let point_id = def_id(21);
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "Sized"));
    lowerer.language_items.sized =
        Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
    register_point(&mut lowerer, point_id);

    lowerer.auto_impl_sized();

    assert!(lowerer
        .items
        .impl_defs()
        .any(|(_, imp)| imp.type_name == "Point" && imp.trait_id == Some(sized_id)));
}

#[test]
fn auto_impl_sized_uses_marked_renamed_trait_id() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(22);
    let point_id = def_id(23);
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "StaticLayout"));
    lowerer.language_items.sized =
        Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
    register_point(&mut lowerer, point_id);

    lowerer.auto_impl_sized();

    let generated = lowerer
        .items
        .impl_defs()
        .find_map(|(_, imp)| (imp.type_name == "Point").then_some(imp))
        .expect("marked trait should receive automatic implementations");
    assert_eq!(generated.trait_id, Some(sized_id));
    assert_eq!(generated.trait_name.as_deref(), Some("StaticLayout"));
}

#[test]
fn auto_impl_sized_does_not_run_without_marked_bundle() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(24);
    let point_id = def_id(25);
    lowerer
        .items
        .insert_trait_def(empty_trait(trait_id, "StaticLayout"));
    register_point(&mut lowerer, point_id);

    lowerer.auto_impl_sized();

    assert!(lowerer.items.impl_defs().next().is_none());
}

#[test]
fn auto_impl_sized_preserves_generic_struct_shape() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(5);
    let struct_id = def_id(6);
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "Sized"));
    lowerer.language_items.sized =
        Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
    lowerer.items.insert_structure(HirStruct {
        id: struct_id,
        name: "Box".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: struct_id,
                index: 0,
            },
            "T",
        )],
        fields: vec![],
    });
    lowerer
        .resolver
        .item_paths
        .insert("Box".to_string(), struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "Box".to_string());
    lowerer.current_def_ids.insert(struct_id);

    lowerer.auto_impl_sized();

    let impl_def = lowerer
        .items
        .impl_defs()
        .find_map(|(_, imp)| (imp.type_name == "Box").then_some(imp))
        .expect("generic Box should receive Sized");
    assert_eq!(impl_def.type_generics[0].name, "T");
    assert_eq!(impl_def.type_generics[0].id.owner, impl_def.id);
    assert_eq!(
        impl_def.receiver_pattern,
        HirImplReceiverPattern::Exact(Type::Struct {
            id: struct_id,
            args: vec![Type::Generic(GenericParamId {
                owner: impl_def.id,
                index: 0,
            })],
        })
    );
}

#[test]
fn auto_impl_sized_preserves_generic_enum_shape() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(7);
    let enum_id = def_id(8);
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "Sized"));
    lowerer.language_items.sized =
        Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
    lowerer.items.insert_enumeration(HirEnum {
        id: enum_id,
        name: "Option".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: enum_id,
                index: 0,
            },
            "T",
        )],
        variants: vec![],
    });
    lowerer
        .resolver
        .item_paths
        .insert("Option".to_string(), enum_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(enum_id, "Option".to_string());
    lowerer.current_def_ids.insert(enum_id);

    lowerer.auto_impl_sized();

    let impl_def = lowerer
        .items
        .impl_defs()
        .find_map(|(_, imp)| (imp.type_name == "Option").then_some(imp))
        .expect("generic Option should receive Sized");
    assert_eq!(impl_def.type_generics[0].name, "T");
    assert_eq!(impl_def.type_generics[0].id.owner, impl_def.id);
    assert_eq!(
        impl_def.receiver_pattern,
        HirImplReceiverPattern::Exact(Type::Enum {
            id: enum_id,
            args: vec![Type::Generic(GenericParamId {
                owner: impl_def.id,
                index: 0,
            })],
        })
    );
}

#[test]
fn auto_impl_sized_generates_impls_in_owner_def_id_order() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(5);
    let high_id = def_id(30);
    let low_id = def_id(10);
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "Sized"));
    lowerer.language_items.sized =
        Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
    lowerer.items.insert_structure(HirStruct {
        id: high_id,
        name: "High".to_string(),
        generic_params: vec![],
        fields: vec![],
    });
    lowerer.items.insert_enumeration(HirEnum {
        id: low_id,
        name: "Low".to_string(),
        generic_params: vec![],
        variants: vec![],
    });
    for (id, name) in [(high_id, "High"), (low_id, "Low")] {
        lowerer.resolver.item_paths.insert(name.to_string(), id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(id, name.to_string());
        lowerer.current_def_ids.insert(id);
    }

    lowerer.auto_impl_sized();

    assert_eq!(
        lowerer
            .items
            .impl_defs_in_order()
            .map(|(_, imp)| imp.type_name.as_str())
            .collect::<Vec<_>>(),
        vec!["Low", "High"]
    );
}

#[test]
fn auto_impl_sized_skips_foreign_structs_without_current_provenance() {
    let mut lowerer = Lowerer::new_for_test();
    let sized_id = def_id(20);
    let foreign_point_id = DefId::new(CrateId(7), LocalDefId(21));
    lowerer
        .items
        .insert_trait_def(empty_trait(sized_id, "Sized"));
    lowerer.language_items.sized =
        Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
    lowerer
        .items
        .insert_structure(point_struct(foreign_point_id));
    lowerer
        .resolver
        .item_paths
        .insert("dep::Point".to_string(), foreign_point_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(foreign_point_id, "dep::Point".to_string());

    lowerer.auto_impl_sized();

    assert!(lowerer
        .items
        .impl_defs()
        .all(|(_, imp)| !(imp.type_name == "dep::Point" && imp.trait_id == Some(sized_id))));
}

#[test]
fn conformance_preserves_empty_override_without_default_injection() {
    let mut lowerer = Lowerer::new_for_test();

    let trait_id = def_id(6);
    let impl_id = def_id(7);
    let default_method_id = def_id(8);
    let override_method_id = def_id(9);

    let default_method = HirFunction {
        id: default_method_id,
        name: "print".to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(GenericParamId {
                owner: trait_id,
                index: 0,
            }),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I32,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::IntLiteral(42),
                ty: Type::I32,
                span: crate::lexer::Span::test(),
            })],
            ty: Type::I32,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Shared),
        is_unsafe: false,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Printable".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::from([("print".to_string(), default_method.clone())]),
        signatures: HashMap::from([(
            "print".to_string(),
            HirFunctionSig {
                id: def_id(41),
                name: "print".to_string(),
                generic_params: vec![],
                params: vec![Type::Generic(GenericParamId {
                    owner: trait_id,
                    index: 0,
                })],
                ret: Type::I32,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Printable".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "print".to_string(),
                HirFunction {
                    id: override_method_id,
                    name: "print".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I64,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::I32,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Unit,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["print"];
    assert_eq!(method.id, override_method_id);
    assert_eq!(method.params[0].ty, Type::I64);
    assert_eq!(method.ret_type, Type::I32);
    assert!(method.body.stmts.is_empty());
}

#[test]
fn conformance_resolves_generic_impl_self_type_from_canonical_owner() {
    let mut lowerer = Lowerer::new_for_test();

    let trait_id = def_id(60);
    let impl_id = def_id(61);
    let default_method_id = def_id(62);
    let local_box_id = def_id(63);
    let local_enum_box_id = def_id(64);
    let dependency_box_id = DefId::new(CrateId(2), LocalDefId(10));
    let trait_self = Type::Generic(GenericParamId {
        owner: trait_id,
        index: 0,
    });

    lowerer.items.insert_structure(HirStruct {
        id: local_box_id,
        name: "Box".to_string(),
        generic_params: vec![],
        fields: vec![],
    });
    lowerer.items.insert_enumeration(HirEnum {
        id: local_enum_box_id,
        name: "Box".to_string(),
        generic_params: vec![],
        variants: vec![],
    });
    lowerer.items.insert_structure(HirStruct {
        id: dependency_box_id,
        name: "Box".to_string(),
        generic_params: vec![],
        fields: vec![],
    });
    lowerer
        .resolver
        .item_paths
        .insert("Box".to_string(), local_box_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(local_box_id, "Box".to_string());
    let mut dep_resolver = ResolverTables::default();
    dep_resolver
        .item_paths
        .insert("dep_b::Box".to_string(), dependency_box_id);
    dep_resolver
        .item_names_by_id
        .insert(dependency_box_id, "dep_b::Box".to_string());
    lowerer
        .dependency_resolvers
        .insert("dep_b".to_string(), dep_resolver);

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "CloneSelf".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::from([(
            "clone_self".to_string(),
            HirFunction {
                id: default_method_id,
                name: "clone_self".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: trait_self.clone(),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: trait_self.clone(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("self".to_string()),
                        ty: trait_self.clone(),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: trait_self.clone(),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("dep_b::Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: vec![],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: dependency_box_id,
                args: Vec::new(),
            }),
            trait_name: Some("CloneSelf".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["clone_self"];
    let expected_self = Type::Struct {
        id: dependency_box_id,
        args: vec![],
    };
    assert_eq!(method.params[0].ty, expected_self);
    assert_eq!(method.ret_type, expected_self);
    assert!(
        lowerer.errors().is_empty(),
        "unexpected errors: {:?}",
        lowerer.errors()
    );
}

#[test]
fn conformance_resolves_signatureless_default_method_return_type_from_body() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(50);
    let impl_id = def_id(51);
    let default_method_id = def_id(52);
    let ret_ty = lowerer.engine.fresh_type_var();

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Animal".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::from([(
            "legs".to_string(),
            HirFunction {
                id: default_method_id,
                name: "legs".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(GenericParamId {
                        owner: trait_id,
                        index: 0,
                    }),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: ret_ty,
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::IntLiteral(4),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::I64,
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Dog".to_string()),
            type_name: "Dog".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Animal".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert_eq!(
        lowerer.items.impl_def(impl_id).unwrap().methods["legs"].ret_type,
        Type::I64
    );
}

#[test]
fn trait_fixture_is_keyed_by_its_exact_id() {
    let expected_id = def_id(10);
    let wrong_id = def_id(11);
    let traits = HashMap::from([(
        wrong_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: wrong_id,
            name: "Debug".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    )]);

    assert!(traits.get(&expected_id).is_none());
    assert_eq!(traits.get(&wrong_id).unwrap().id, wrong_id);
}

#[test]
fn test_conformance_preserves_generic_borrowed_slice_self_param() {
    let mut lowerer = Lowerer::new_for_test();
    let self_ty = lowerer.engine.fresh_type_var();

    let trait_id = def_id(1);
    let impl_id = def_id(2);

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Show".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "show".to_string(),
            HirFunctionSig {
                id: def_id(42),
                name: "show".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(GenericParamId {
                    owner: trait_id,
                    index: 0,
                })],
                ret: Type::I32,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("&[T]".to_string()),
            type_name: "&[T]".to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                "T",
            )],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Reference {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })))),
            }),
            trait_name: Some("Show".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "show".to_string(),
                HirFunction {
                    id: def_id(3),
                    name: "show".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: self_ty,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::I32,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::I32,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert_eq!(
        lowerer.items.impl_def(impl_id).unwrap().methods["show"].params[0].ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Slice(Box::new(Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0
            })))),
        }
    );
}

#[test]
fn conformance_substitutes_zero_based_declared_trait_generics() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(10);
    let impl_id = def_id(11);
    let method_id = def_id(12);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let value_ty = lowerer.engine.fresh_type_var();
    let ret_ty = lowerer.engine.fresh_type_var();

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Sink".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "take".to_string(),
            HirFunctionSig {
                id: def_id(43),
                name: "take".to_string(),
                generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
                params: vec![Type::Generic(trait_self), Type::Generic(trait_generic)],
                ret: Type::Generic(trait_generic),
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::I32),
            trait_name: Some("Sink".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "Bool",
            )],
            trait_arg_types: vec![Type::Bool],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "take".to_string(),
                HirFunction {
                    id: method_id,
                    name: "take".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![
                        HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I32,
                            mutable: false,
                            is_ref: false,
                        },
                        HirParam {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: value_ty,
                            mutable: false,
                            is_ref: false,
                        },
                    ],
                    ret_type: ret_ty,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Bool,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["take"];
    assert_eq!(method.params[1].ty, Type::Bool);
    assert_eq!(method.ret_type, Type::Bool);
}

#[test]
fn conformance_rejects_impl_method_missing_explicit_signature_arg() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(13);
    let impl_id = def_id(14);
    let method_id = def_id(15);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Sink".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "take".to_string(),
            HirFunctionSig {
                id: def_id(16),
                name: "take".to_string(),
                generic_params: Vec::new(),
                params: vec![
                    Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::Generic(trait_self)),
                    },
                    Type::I64,
                ],
                ret: Type::Bool,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::I32),
            trait_name: Some("Sink".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "take".to_string(),
                HirFunction {
                    id: method_id,
                    name: "take".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I32),
                        },
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::Bool,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Bool,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert!(lowerer.errors().iter().any(|error| {
        error
            .message
            .contains("method 'take' parameter count mismatch")
            && error.message.contains("expected 2")
            && error.message.contains("found 1")
    }));
}

#[test]
fn conformance_rejects_impl_method_receiver_mode_mismatch() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(420);
    let impl_id = def_id(421);
    let method_id = def_id(422);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "DropLike".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "drop".to_string(),
            HirFunctionSig {
                id: def_id(423),
                name: "drop".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Generic(trait_self)),
                }],
                ret: Type::Unit,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("DropLike".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "drop".to_string(),
                HirFunction {
                    id: method_id,
                    name: "drop".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I32,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::Unit,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Unit,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert!(lowerer.errors().iter().any(|error| {
        error
            .message
            .contains("method 'drop' parameter 0 type mismatch")
            && error.message.contains("trait 'DropLike'")
    }));
}

#[test]
fn conformance_rejects_impl_method_return_type_mismatch() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(424);
    let impl_id = def_id(425);
    let method_id = def_id(426);

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Flag".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "flag".to_string(),
            HirFunctionSig {
                id: def_id(427),
                name: "flag".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::I32],
                ret: Type::Bool,
                generic_bounds: HashMap::new().into(),
                self_receiver: None,
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Flag".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "flag".to_string(),
                HirFunction {
                    id: method_id,
                    name: "flag".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I32,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::I64,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::I64,
                    },
                    is_curried: false,
                    is_method: false,
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert!(lowerer.errors().iter().any(|error| {
        error.message.contains("method 'flag' return type mismatch")
            && error.message.contains("trait 'Flag'")
    }));
}

#[test]
fn conformance_does_not_recheck_dependency_impl_signatures() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(428);
    let impl_id = DefId::new(CrateId(7), LocalDefId(429));
    let method_id = DefId::new(CrateId(7), LocalDefId(430));

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Flag".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "flag".to_string(),
            HirFunctionSig {
                id: def_id(431),
                name: "flag".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::I32],
                ret: Type::Bool,
                generic_bounds: HashMap::new().into(),
                self_receiver: None,
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Flag".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "flag".to_string(),
                HirFunction {
                    id: method_id,
                    name: "flag".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I32,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::I64,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::I64,
                    },
                    is_curried: false,
                    is_method: false,
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert!(lowerer.errors().is_empty());
}

#[test]
fn conformance_substitutes_trait_arg_types_not_display_names() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(20);
    let impl_id = def_id(21);
    let method_id = def_id(22);
    let box_id = def_id(23);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let boxed_i64 = Type::Struct {
        id: box_id,
        args: vec![Type::I64],
    };
    let value_ty = lowerer.engine.fresh_type_var();
    let ret_ty = lowerer.engine.fresh_type_var();

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Sink".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "take".to_string(),
            HirFunctionSig {
                id: def_id(44),
                name: "take".to_string(),
                generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
                params: vec![Type::Generic(trait_self), Type::Generic(trait_generic)],
                ret: Type::Generic(trait_generic),
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Sink".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "Box",
            )],
            trait_arg_types: vec![boxed_i64.clone()],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "take".to_string(),
                HirFunction {
                    id: method_id,
                    name: "take".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![
                        HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I32,
                            mutable: false,
                            is_ref: false,
                        },
                        HirParam {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: value_ty,
                            mutable: false,
                            is_ref: false,
                        },
                    ],
                    ret_type: ret_ty,
                    body: HirBlock {
                        stmts: vec![],
                        ty: boxed_i64.clone(),
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["take"];
    assert_eq!(method.params[1].ty, boxed_i64);
    assert_eq!(method.ret_type, boxed_i64);
}

#[test]
fn conformance_uses_resolved_trait_id_for_projection_substitution() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(30);
    let impl_id = def_id(31);
    let method_id = def_id(32);
    let assoc_id = AssocTypeId(0);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let ret_ty = lowerer.engine.fresh_type_var();

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Deref".to_string(),
        generic_params: vec![],
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_id,
            name: "Target".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "deref".to_string(),
            HirFunctionSig {
                id: def_id(45),
                name: "deref".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(trait_self)],
                ret: Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Projection {
                        ty: Box::new(Type::Generic(trait_self)),
                        trait_id,
                        assoc_type: AssociatedTypeKey {
                            owner: trait_id,
                            assoc_type_id: assoc_id,
                        },
                        trait_args: Vec::new(),
                    }),
                },
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });
    lowerer
        .resolver
        .item_paths
        .insert("Deref".to_string(), trait_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "Deref".to_string());

    let method = HirFunction {
        id: method_id,
        name: "deref".to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        }],
        ret_type: ret_ty,
        body: HirBlock {
            stmts: vec![],
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Deref".to_string()),
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_id,
                name: "Target".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("deref".to_string(), method)]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let imp = lowerer.items.impl_def(impl_id).unwrap();
    assert_eq!(imp.trait_id, Some(trait_id));
    assert_eq!(
        imp.methods["deref"].ret_type,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        }
    );
}

#[test]
fn conformance_does_not_substitute_projection_owned_by_different_trait() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(50);
    let other_trait_id = def_id(51);
    let impl_id = def_id(52);
    let method_id = def_id(53);
    let assoc_id = AssocTypeId(0);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let ret_ty = lowerer.engine.fresh_type_var();

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Deref".to_string(),
        generic_params: vec![],
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_id,
            name: "Target".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "deref".to_string(),
            HirFunctionSig {
                id: def_id(54),
                name: "deref".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(trait_self)],
                ret: Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Projection {
                        ty: Box::new(Type::Generic(trait_self)),
                        trait_id,
                        assoc_type: AssociatedTypeKey {
                            owner: other_trait_id,
                            assoc_type_id: assoc_id,
                        },
                        trait_args: Vec::new(),
                    }),
                },
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Deref".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_id,
                name: "Target".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "deref".to_string(),
                HirFunction {
                    id: method_id,
                    name: "deref".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I64,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: ret_ty,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I64),
                        },
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert!(matches!(
        &lowerer.items.impl_def(impl_id).unwrap().methods["deref"].ret_type,
        Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::Projection { .. })
    ));
}

#[test]
fn conformance_does_not_substitute_projection_with_different_trait_args() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(55);
    let impl_id = def_id(56);
    let method_id = def_id(57);
    let assoc_id = AssocTypeId(0);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let ret_ty = lowerer.engine.fresh_type_var();

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Project".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "T",
        )],
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "project".to_string(),
            HirFunctionSig {
                id: def_id(58),
                name: "project".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(trait_self)],
                ret: Type::Projection {
                    ty: Box::new(Type::Generic(trait_self)),
                    trait_id,
                    assoc_type: AssociatedTypeKey {
                        owner: trait_id,
                        assoc_type_id: assoc_id,
                    },
                    trait_args: vec![Type::Bool],
                },
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Project".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "project".to_string(),
                HirFunction {
                    id: method_id,
                    name: "project".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I64,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: ret_ty,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Projection {
                            ty: Box::new(Type::I64),
                            trait_id,
                            assoc_type: AssociatedTypeKey {
                                owner: trait_id,
                                assoc_type_id: assoc_id,
                            },
                            trait_args: vec![Type::Bool],
                        },
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert!(matches!(
        &lowerer.items.impl_def(impl_id).unwrap().methods["project"].ret_type,
        Type::Projection { trait_args, .. } if trait_args.as_slice() == [Type::Bool]
    ));
}

#[test]
fn conformance_does_not_substitute_projection_on_non_self_generic_base() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(70);
    let impl_id = def_id(71);
    let method_id = def_id(72);
    let assoc_id = AssocTypeId(0);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let non_self_projection = Type::Projection {
        ty: Box::new(Type::Generic(trait_generic)),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: assoc_id,
        },
        trait_args: vec![Type::Generic(trait_generic)],
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Project".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "T",
        )],
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::from([(
            "value".to_string(),
            HirFunction {
                id: method_id,
                name: "value".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![
                    HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    },
                    HirParam {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: non_self_projection.clone(),
                        mutable: false,
                        is_ref: false,
                    },
                ],
                ret_type: non_self_projection.clone(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("value".to_string()),
                        ty: non_self_projection.clone(),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: non_self_projection.clone(),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Project".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::Bool,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["value"];
    assert!(matches!(method.params[1].ty, Type::Projection { .. }));
    assert!(matches!(method.ret_type, Type::Projection { .. }));
}

#[test]
fn conformance_substitutes_default_method_nested_struct_trait_generic_types() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(73);
    let impl_id = def_id(74);
    let method_id = def_id(75);
    let box_id = def_id(76);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let boxed_trait_generic = Type::Struct {
        id: box_id,
        args: vec![Type::Generic(trait_generic)],
    };
    let boxed_i64 = Type::Struct {
        id: box_id,
        args: vec![Type::I64],
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "MakeBox".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "make".to_string(),
            HirFunction {
                id: method_id,
                name: "make".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(trait_self),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: boxed_trait_generic.clone(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("boxed".to_string()),
                        ty: boxed_trait_generic.clone(),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: boxed_trait_generic.clone(),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("MakeBox".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["make"];
    assert_eq!(method.ret_type, boxed_i64);
    assert_eq!(method.body.ty, boxed_i64);
}

#[test]
fn conformance_substitutes_default_method_nested_enum_trait_generic_types() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(77);
    let impl_id = def_id(78);
    let method_id = def_id(79);
    let maybe_id = def_id(80);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let maybe_trait_generic = Type::Enum {
        id: maybe_id,
        args: vec![Type::Generic(trait_generic)],
    };
    let maybe_i64 = Type::Enum {
        id: maybe_id,
        args: vec![Type::I64],
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "MakeMaybe".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "make".to_string(),
            HirFunction {
                id: method_id,
                name: "make".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(trait_self),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: maybe_trait_generic.clone(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("maybe".to_string()),
                        ty: maybe_trait_generic.clone(),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: maybe_trait_generic.clone(),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("MakeMaybe".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["make"];
    assert_eq!(method.ret_type, maybe_i64);
    assert_eq!(method.body.ty, maybe_i64);
}

#[test]
fn conformance_substitutes_default_method_call_target_trait_args() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(81);
    let impl_id = def_id(82);
    let method_id = def_id(83);
    let target_method_id = def_id(84);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Caller".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "call".to_string(),
            HirFunction {
                id: method_id,
                name: "call".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(trait_self),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: Type::I64,
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::MethodCall(
                            Box::new(HirExpr {
                                kind: HirExprKind::Var("self".to_string()),
                                ty: Type::Generic(trait_self),
                                span: crate::lexer::Span::test(),
                            }),
                            "target".to_string(),
                            Vec::new(),
                            Some(ReceiverMode::Shared),
                            Some(HirMethodCallTarget::impl_method(
                                impl_id,
                                target_method_id,
                                Some(crate::hir::HirSelectedTraitMember {
                                    trait_id,
                                    member_id: target_method_id,
                                    trait_args: vec![Type::Generic(trait_generic)],
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
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Caller".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let HirStmt::Expr(expr) = &lowerer.items.impl_def(impl_id).unwrap().methods["call"]
        .body
        .stmts[0]
    else {
        panic!("default body should contain method call");
    };
    let HirExprKind::MethodCall(_, _, _, _, Some(target)) = &expr.kind else {
        panic!("default body should contain method target");
    };
    assert_eq!(target.trait_args(), &[Type::I64]);
}

#[test]
fn conformance_does_not_retarget_default_method_bound_non_self_receiver() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(113);
    let impl_id = def_id(114);
    let score_sig_id = def_id(115);
    let default_method_id = def_id(116);
    let impl_score_method_id = def_id(117);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let method_generic = GenericParamId {
        owner: default_method_id,
        index: 0,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Metric".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::from([(
            "score_other".to_string(),
            HirFunction {
                id: default_method_id,
                name: "score_other".to_string(),
                generic_params: vec![GenericParamDecl::type_param(method_generic, "T")],
                generic_bounds: HashMap::from([(
                    method_generic,
                    vec![TraitBound {
                        trait_id,
                        type_args: vec![],
                    }],
                )])
                .into(),
                params: vec![
                    HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    },
                    HirParam {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(1),
                        ty: Type::Generic(method_generic),
                        mutable: false,
                        is_ref: false,
                    },
                ],
                ret_type: Type::I64,
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::MethodCall(
                            Box::new(HirExpr {
                                kind: HirExprKind::Var("value".to_string()),
                                ty: Type::Generic(method_generic),
                                span: crate::lexer::Span::test(),
                            }),
                            "score".to_string(),
                            Vec::new(),
                            Some(ReceiverMode::Shared),
                            Some(HirMethodCallTarget::trait_method(
                                trait_id,
                                score_sig_id,
                                Vec::new(),
                                crate::hir::HirTraitDispatchKind::CurrentTrait,
                            )),
                        ),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::I64,
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::from([(
            "score".to_string(),
            HirFunctionSig {
                id: score_sig_id,
                name: "score".to_string(),
                generic_params: vec![],
                params: vec![Type::Generic(trait_self)],
                ret: Type::I64,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Metric".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "score".to_string(),
                HirFunction {
                    id: impl_score_method_id,
                    name: "score".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I32,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::I64,
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::IntLiteral(1),
                            ty: Type::I64,
                            span: crate::lexer::Span::test(),
                        })],
                        ty: Type::I64,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let HirStmt::Expr(expr) = &lowerer.items.impl_def(impl_id).unwrap().methods["score_other"]
        .body
        .stmts[0]
    else {
        panic!("default body should contain method call");
    };
    let HirExprKind::MethodCall(_, _, _, _, Some(target)) = &expr.kind else {
        panic!("default body should contain method target");
    };
    assert_eq!(target.impl_id(), None);
    assert_eq!(target.method_id(), Some(score_sig_id));
}

#[test]
fn conformance_substitutes_default_method_match_guard_and_pattern_types() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(85);
    let impl_id = def_id(86);
    let method_id = def_id(87);
    let box_id = def_id(88);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let boxed_trait_generic = Type::Struct {
        id: box_id,
        args: vec![Type::Generic(trait_generic)],
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Unwrap".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "unwrap".to_string(),
            HirFunction {
                id: method_id,
                name: "unwrap".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![
                    HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    },
                    HirParam {
                        name: "boxed".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: boxed_trait_generic.clone(),
                        mutable: false,
                        is_ref: false,
                    },
                ],
                ret_type: Type::Generic(trait_generic),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Match {
                            scrutinee: Box::new(HirExpr {
                                kind: HirExprKind::Var("boxed".to_string()),
                                ty: boxed_trait_generic.clone(),
                                span: crate::lexer::Span::test(),
                            }),
                            arms: vec![HirMatchArm {
                                pattern: HirPattern::Struct(
                                    "Box".to_string(),
                                    None,
                                    vec![Type::Generic(trait_generic)],
                                    vec![HirStructPatternField {
                                        name: "value".to_string(),
                                        field: None,
                                        pattern: HirPattern::Binding {
                                            name: "value".to_string(),
                                            local_id: crate::ids::HirLocalId(0),
                                            mutable: false,
                                        },
                                    }],
                                ),
                                guard: Some(HirExpr {
                                    kind: HirExprKind::Var("value".to_string()),
                                    ty: Type::Generic(trait_generic),
                                    span: crate::lexer::Span::test(),
                                }),
                                body: HirBlock {
                                    stmts: vec![HirStmt::Expr(HirExpr {
                                        kind: HirExprKind::Var("value".to_string()),
                                        ty: Type::Generic(trait_generic),
                                        span: crate::lexer::Span::test(),
                                    })],
                                    ty: Type::Generic(trait_generic),
                                },
                            }],
                        },
                        ty: Type::Generic(trait_generic),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::Generic(trait_generic),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Unwrap".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let HirStmt::Expr(expr) = &lowerer.items.impl_def(impl_id).unwrap().methods["unwrap"]
        .body
        .stmts[0]
    else {
        panic!("default body should contain match expression");
    };
    let HirExprKind::Match { arms, .. } = &expr.kind else {
        panic!("default body should contain match expression");
    };
    let HirPattern::Struct(_, _, type_args, _) = &arms[0].pattern else {
        panic!("match arm should contain struct pattern");
    };
    assert_eq!(type_args, &vec![Type::I64]);
    assert_eq!(arms[0].guard.as_ref().unwrap().ty, Type::I64);
}

#[test]
fn conformance_substitutes_default_method_generic_bound_type_args() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(89);
    let impl_id = def_id(90);
    let method_id = def_id(91);
    let bound_trait_id = def_id(92);
    let method_generic = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Factory".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "make".to_string(),
            HirFunction {
                id: method_id,
                name: "make".to_string(),
                generic_params: vec![GenericParamDecl::type_param(method_generic, "U")],
                generic_bounds: HashMap::from([(
                    method_generic,
                    vec![TraitBound {
                        trait_id: bound_trait_id,
                        type_args: vec![Type::Generic(trait_generic)],
                    }],
                )])
                .into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(trait_self),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: Type::I64,
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::IntLiteral(1),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::I64,
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Factory".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["make"];
    let remapped_method_generic = method.generic_params[0].id;
    assert_eq!(remapped_method_generic.owner, method.id);
    assert_eq!(
        method.generic_bounds[&remapped_method_generic][0].type_args,
        vec![Type::I64]
    );
}

#[test]
fn conformance_default_body_generic_ids_match_trait_header_order() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(95);
    let impl_id = def_id(96);
    let method_id = def_id(97);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Identity".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "identity".to_string(),
            HirFunction {
                id: method_id,
                name: "identity".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![
                    HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    },
                    HirParam {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_generic),
                        mutable: false,
                        is_ref: false,
                    },
                ],
                ret_type: Type::Generic(trait_generic),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("value".to_string()),
                        ty: Type::Generic(trait_generic),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::Generic(trait_generic),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::I32),
            trait_name: Some("Identity".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["identity"];
    assert_eq!(method.params[0].ty, Type::I32);
    assert_eq!(method.params[1].ty, Type::I64);
    assert_eq!(method.ret_type, Type::I64);
    assert_eq!(method.body.ty, Type::I64);
    let HirStmt::Expr(expr) = &method.body.stmts[0] else {
        panic!("default body should contain expression");
    };
    assert_eq!(expr.ty, Type::I64);
}

#[test]
fn conformance_substitutes_default_method_lambda_capture_types() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(60);
    let impl_id = def_id(61);
    let default_method_id = def_id(62);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let self_ty = Type::Generic(trait_self);
    let capture_ty = Type::Generic(trait_generic);
    let lambda_ty = Type::function(Vec::new(), capture_ty.clone());

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Factory".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "make".to_string(),
            HirFunction {
                id: default_method_id,
                name: "make".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: self_ty.clone(),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: lambda_ty.clone(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Lambda {
                            params: Vec::new(),
                            body: HirBlock {
                                stmts: Vec::new(),
                                ty: capture_ty.clone(),
                            },
                            captures: vec![HirClosureCapture {
                                name: "captured".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                kind: HirClosureCaptureKind::Move,
                                mutable: false,
                                ty: capture_ty.clone(),
                            }],
                        },
                        ty: lambda_ty.clone(),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: lambda_ty.clone(),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Factory".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let HirStmt::Expr(expr) = &lowerer.items.impl_def(impl_id).unwrap().methods["make"]
        .body
        .stmts[0]
    else {
        panic!("default method body should contain lambda expression");
    };
    let HirExprKind::Lambda { captures, .. } = &expr.kind else {
        panic!("default method body should contain lambda expression");
    };
    assert_eq!(captures[0].ty, Type::I64);
}

#[test]
fn conformance_substitutes_default_method_lambda_projection_carriers() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(100);
    let impl_id = def_id(101);
    let default_method_id = def_id(102);
    let box_id = def_id(103);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let self_ty = Type::Generic(trait_self);
    let projection_ty = Type::Projection {
        ty: Box::new(self_ty.clone()),
        trait_id,
        assoc_type: AssociatedTypeKey {
            assoc_type_id: AssocTypeId(0),
            owner: trait_id,
        },
        trait_args: vec![Type::Generic(trait_generic)],
    };
    let impl_output_ty = Type::Struct {
        id: box_id,
        args: vec![Type::I64],
    };
    let lambda_ty = Type::function(vec![projection_ty.clone()], projection_ty.clone());

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Factory".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![HirAssociatedTypeDecl {
            id: AssocTypeId(0),
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::from([(
            "make".to_string(),
            HirFunction {
                id: default_method_id,
                name: "make".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: self_ty.clone(),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: lambda_ty.clone(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Lambda {
                            params: vec![HirParam {
                                name: "value".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                ty: projection_ty.clone(),
                                mutable: false,
                                is_ref: false,
                            }],
                            body: HirBlock {
                                stmts: Vec::new(),
                                ty: projection_ty.clone(),
                            },
                            captures: vec![HirClosureCapture {
                                name: "captured".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                kind: HirClosureCaptureKind::Move,
                                mutable: false,
                                ty: projection_ty.clone(),
                            }],
                        },
                        ty: lambda_ty.clone(),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: lambda_ty.clone(),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Factory".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![HirAssociatedTypeDef {
                id: AssocTypeId(0),
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: impl_output_ty.clone(),
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["make"];
    let expected_lambda_ty = Type::function(vec![impl_output_ty.clone()], impl_output_ty.clone());
    assert_eq!(method.ret_type, expected_lambda_ty);

    let HirStmt::Expr(expr) = &method.body.stmts[0] else {
        panic!("default method body should contain lambda expression");
    };
    assert_eq!(expr.ty, expected_lambda_ty);

    let HirExprKind::Lambda {
        params,
        body,
        captures,
    } = &expr.kind
    else {
        panic!("default method body should contain lambda expression");
    };
    assert_eq!(params[0].ty, impl_output_ty);
    assert_eq!(body.ty, impl_output_ty);
    assert_eq!(captures[0].ty, impl_output_ty);
}

#[test]
fn generated_default_method_is_attached_to_exact_impl_owner_and_remaps_generics() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(110);
    let impl_id = def_id(111);
    let default_method_id = def_id(112);
    let bound_trait_id = def_id(114);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let default_method_generic = GenericParamId {
        owner: default_method_id,
        index: 0,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Factory".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::from([(
            "id".to_string(),
            HirFunction {
                id: default_method_id,
                name: "id".to_string(),
                generic_params: vec![GenericParamDecl::type_param(default_method_generic, "U")],
                generic_bounds: HashMap::from([(
                    default_method_generic,
                    vec![TraitBound {
                        trait_id: bound_trait_id,
                        type_args: vec![Type::Generic(default_method_generic)],
                    }],
                )])
                .into(),
                params: vec![
                    HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    },
                    HirParam {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(default_method_generic),
                        mutable: false,
                        is_ref: false,
                    },
                ],
                ret_type: Type::Generic(default_method_generic),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("value".to_string()),
                        ty: Type::Generic(default_method_generic),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::Generic(default_method_generic),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Factory".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["id"];
    assert_ne!(method.id, default_method_id);
    assert!(lowerer.current_def_ids.contains(&method.id));
    let remapped_method_generic = GenericParamId {
        owner: method.id,
        index: 0,
    };
    assert_eq!(method.generic_params[0].id, remapped_method_generic);
    assert!(method.generic_bounds.contains_key(&remapped_method_generic));
    assert!(!method.generic_bounds.contains_key(&default_method_generic));
    assert_eq!(method.params[1].ty, Type::Generic(remapped_method_generic));
    assert_eq!(method.ret_type, Type::Generic(remapped_method_generic));
    assert_eq!(method.body.ty, Type::Generic(remapped_method_generic));
    assert_eq!(
        method.generic_bounds[&remapped_method_generic][0].type_args,
        vec![Type::Generic(remapped_method_generic)]
    );
    let HirStmt::Expr(expr) = &method.body.stmts[0] else {
        panic!("default body should contain expression");
    };
    assert_eq!(expr.ty, Type::Generic(remapped_method_generic));
}

#[test]
fn trait_conformance_service_uses_explicit_deps_for_selection_and_required_items() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(130);
    let impl_id = def_id(131);
    let assoc_id = AssocTypeId(0);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "module::Complete".to_string(),
        generic_params: vec![],
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "finish".to_string(),
            HirFunctionSig {
                id: def_id(132),
                name: "finish".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(trait_self)],
                ret: Type::I64,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });
    lowerer.resolver.insert_import_alias_with_name(
        "Complete".to_string(),
        "module::Complete".to_string(),
        trait_id,
    );
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Complete".to_string()),
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    let output = {
        let mut service = super::TraitConformanceService::new(super::TraitConformanceContext {
            items: &mut lowerer.items,
            engine: &mut lowerer.engine,
            resolver: &lowerer.resolver,
            dependency_resolvers: &lowerer.dependency_resolvers,
            root_crate_id: lowerer.root_crate_id,
            local_def_ids: &mut lowerer.local_def_ids,
            current_def_ids: &mut lowerer.current_def_ids,
            source_map: &lowerer.source_map,
            index_protocol_ids: None,
        });
        service.check_trait_conformance();
        service.finish()
    };

    assert_eq!(
        lowerer.items.impl_def(impl_id).unwrap().trait_id,
        Some(trait_id)
    );
    assert!(output
        .diagnostics
        .iter()
        .any(|error| error.message.contains("required associated type 'Output'")));
    assert!(output
        .diagnostics
        .iter()
        .any(|error| error.message.contains("required method 'finish'")));
}

#[test]
fn lowerer_conformance_delegate_reports_required_items() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(140);
    let impl_id = def_id(141);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Complete".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "finish".to_string(),
            HirFunctionSig {
                id: def_id(142),
                name: "finish".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(trait_self)],
                ret: Type::I64,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Complete".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert!(lowerer
        .errors()
        .iter()
        .any(|error| error.message.contains("required method 'finish'")));
}

#[test]
fn conformance_rejects_missing_supertrait_implementation() {
    let mut lowerer = Lowerer::new_for_test();
    let parent_id = def_id(700);
    let child_id = def_id(701);
    let point_id = def_id(702);
    let impl_id = def_id(703);
    register_point(&mut lowerer, point_id);
    lowerer
        .items
        .insert_trait_def(empty_trait(parent_id, "Parent"));
    lowerer.items.insert_trait_def(HirTrait {
        id: child_id,
        name: "Child".to_string(),
        generic_params: Vec::new(),
        target: None,
        predicates: vec![crate::types::Predicate::Trait {
            subject: Type::Generic(GenericParamId {
                owner: child_id,
                index: 0,
            }),
            trait_id: parent_id,
            args: Vec::new(),
        }],
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    });
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Point".to_string()),
            type_name: "Point".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: point_id,
                args: Vec::new(),
            }),
            trait_name: Some("Child".to_string()),
            trait_id: Some(child_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: crate::hir::HirGenericBounds::new(),
            methods: HashMap::new(),
        })
        .unwrap();
    lowerer.current_def_ids.insert(impl_id);

    lowerer.check_trait_conformance();

    assert!(
        lowerer.errors().iter().any(|error| {
            error
                .message
                .contains("does not satisfy supertrait obligation")
                && error.message.contains("Parent")
        }),
        "unexpected diagnostics: {:?}",
        lowerer.errors()
    );
}
