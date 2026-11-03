use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

use crate::crate_system::CrateContext;
use crate::ids::{CrateId, DefId, FieldId, HirLocalId, LocalDefId, VariantId};
use crate::infer::ResolvedHirProgram;
use crate::{collect, infer, lower, macro_expansion, parser, Config};

static LOWER_PATTERN_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn ident(name: &str) -> ast::Ident {
    ast::Ident {
        name: name.to_string(),
        span: crate::lexer::Span::test(),
    }
}

fn ident_pattern(name: &str, mutable: bool) -> ast::Pattern {
    ast::Pattern {
        binding: Some(ident(name)),
        kind: ast::PatternKind::Ident(ast::IdentPattern {
            name: ident(name),
            mut_: mutable,
        }),
    }
}

fn compile_source_to_resolved_hir_for_test(source: &str) -> ResolvedHirProgram {
    let id = LOWER_PATTERN_TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "rock_lower_pattern_hir_ref_test_{}_{}",
        std::process::id(),
        id
    ));
    let entry_file = dir.join("main.rk");

    let config = Config {
        entry_file: entry_file.clone(),
        output_dir: dir,
        debug_print: Vec::new(),
        meta_files: Vec::new(),
        extern_artifacts: Vec::new(),
        source_providers: Vec::new(),
        current_crate_name: Some("test".to_string()),
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: true,
        no_std: true,
        sysroot: None,
    };

    let ast = parser::parse_source(entry_file, source, &config)
        .map(|module| crate::ast::Program { module })
        .expect("test source parses");
    let macro_context = macro_expansion::MacroExpansionContext::new(&config);
    let ast = macro_expansion::expand_macros_with_context(ast, &macro_context)
        .expect("test macros expand");
    let crate_ctx = CrateContext::new();
    let decls =
        collect::collect(&ast, &crate_ctx, false, Some("test")).expect("test declarations collect");
    let partial = lower::program::lower_from_declarations(&ast, decls, &crate_ctx, Some("test"))
        .expect("test source lowers");
    infer::finalize(partial).expect("test HIR finalizes")
}

fn find_first_struct_pattern(
    block: &crate::hir::AcceptedHirBlock,
) -> Option<(
    &String,
    &Option<DefId>,
    &Vec<Type>,
    &Vec<HirStructPatternField>,
)> {
    block.stmts.iter().find_map(|stmt| match stmt {
        crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
            find_first_struct_pattern_expr(expr)
        }
        _ => None,
    })
}

fn find_first_struct_pattern_expr(
    expr: &crate::hir::AcceptedHirExpr,
) -> Option<(
    &String,
    &Option<DefId>,
    &Vec<Type>,
    &Vec<HirStructPatternField>,
)> {
    match &expr.kind {
        crate::hir::HirExprKindFor::Match { arms, .. } => {
            arms.iter().find_map(|arm| match &arm.pattern {
                HirPattern::Struct(name, id, type_args, fields) => {
                    Some((name, id, type_args, fields))
                }
                _ => None,
            })
        }
        crate::hir::HirExprKindFor::Block(block)
        | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_struct_pattern(block),
        _ => None,
    }
}

fn find_first_enum_pattern(
    block: &crate::hir::AcceptedHirBlock,
) -> Option<(
    &String,
    &String,
    &Option<HirVariantLocation>,
    &Vec<HirPattern>,
)> {
    block.stmts.iter().find_map(|stmt| match stmt {
        crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
            find_first_enum_pattern_expr(expr)
        }
        _ => None,
    })
}

fn find_first_enum_pattern_expr(
    expr: &crate::hir::AcceptedHirExpr,
) -> Option<(
    &String,
    &String,
    &Option<HirVariantLocation>,
    &Vec<HirPattern>,
)> {
    match &expr.kind {
        crate::hir::HirExprKindFor::Match { arms, .. } => {
            arms.iter().find_map(|arm| match &arm.pattern {
                HirPattern::Enum(enum_name, variant_name, location, items) => {
                    Some((enum_name, variant_name, location, items))
                }
                _ => None,
            })
        }
        crate::hir::HirExprKindFor::Block(block)
        | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_enum_pattern(block),
        _ => None,
    }
}

fn unit_enum(id: DefId, name: &str, variant_id: u32, variant_name: &str) -> HirEnum {
    HirEnum {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        variants: vec![HirVariant {
            id: VariantId(variant_id),
            name: variant_name.to_string(),
            fields: HirVariantFields::Unit,
        }],
    }
}

fn named_field_enum() -> HirEnum {
    HirEnum {
        id: def_id(40),
        name: "Record".to_string(),
        generic_params: Vec::new(),
        variants: vec![HirVariant {
            id: VariantId(0),
            name: "Pair".to_string(),
            fields: HirVariantFields::Named(vec![HirField {
                id: FieldId(0),
                name: "first".to_string(),
                ty: Type::I64,
                public: true,
            }]),
        }],
    }
}

fn two_field_named_enum() -> HirEnum {
    HirEnum {
        id: def_id(41),
        name: "Record2".to_string(),
        generic_params: Vec::new(),
        variants: vec![HirVariant {
            id: VariantId(0),
            name: "Pair".to_string(),
            fields: HirVariantFields::Named(vec![
                HirField {
                    id: FieldId(0),
                    name: "first".to_string(),
                    ty: Type::I64,
                    public: true,
                },
                HirField {
                    id: FieldId(1),
                    name: "second".to_string(),
                    ty: Type::I64,
                    public: true,
                },
            ]),
        }],
    }
}

fn qualified_named_enum_field_pattern() -> ast::Pattern {
    ast::Pattern {
        binding: None,
        kind: ast::PatternKind::Instance(ast::InstancePattern {
            name: ast::TypePath {
                path: vec![
                    ast::IdentOrType::Ident(ident("Record")),
                    ast::IdentOrType::Ident(ident("Pair")),
                ],
            },
            args: ast::FieldsPatternOrArgumentsPattern::Fields(vec![ast::FieldPattern {
                name: ident("first"),
                pattern: ident_pattern("value", false),
            }]),
        }),
    }
}

fn qualified_second_named_enum_field_pattern() -> ast::Pattern {
    ast::Pattern {
        binding: None,
        kind: ast::PatternKind::Instance(ast::InstancePattern {
            name: ast::TypePath {
                path: vec![
                    ast::IdentOrType::Ident(ident("Record2")),
                    ast::IdentOrType::Ident(ident("Pair")),
                ],
            },
            args: ast::FieldsPatternOrArgumentsPattern::Fields(vec![ast::FieldPattern {
                name: ident("second"),
                pattern: ident_pattern("second", false),
            }]),
        }),
    }
}

fn qualified_unknown_named_enum_field_pattern() -> ast::Pattern {
    ast::Pattern {
        binding: None,
        kind: ast::PatternKind::Instance(ast::InstancePattern {
            name: ast::TypePath {
                path: vec![
                    ast::IdentOrType::Ident(ident("Record2")),
                    ast::IdentOrType::Ident(ident("Pair")),
                ],
            },
            args: ast::FieldsPatternOrArgumentsPattern::Fields(vec![ast::FieldPattern {
                name: ident("typo"),
                pattern: ident_pattern("value", false),
            }]),
        }),
    }
}

fn unqualified_unit_pattern(name: &str) -> ast::Pattern {
    ast::Pattern {
        binding: None,
        kind: ast::PatternKind::Instance(ast::InstancePattern {
            name: ast::TypePath {
                path: vec![ast::IdentOrType::Ident(ast::Ident {
                    name: name.to_string(),
                    span: crate::lexer::Span::test(),
                })],
            },
            args: ast::FieldsPatternOrArgumentsPattern::Arguments(Vec::new()),
        }),
    }
}

#[test]
fn pattern_binding_allocates_local_id_and_defines_scope() {
    let mut lowerer = Lowerer::new_for_test();
    let mut pattern = ident_pattern("value", false);
    let binding_span = crate::lexer::Span {
        file_path: "main.rk".into(),
        start: 8,
        end: 13,
    };
    if let ast::PatternKind::Ident(binding) = &mut pattern.kind {
        binding.name.span = binding_span.clone();
    }

    let (hir, scoped_local_id, owner) = lowerer.with_test_body_context(|lowerer| {
        let hir = lowerer.lower_pattern(&pattern, &Type::I64);
        let scoped_local_id = lowerer.scope.lookup("value").unwrap().local_id;
        (hir, scoped_local_id, lowerer.current_body_def_id().unwrap())
    });

    match hir {
        HirPattern::Binding {
            name,
            local_id,
            mutable,
        } => {
            assert_eq!(name, "value");
            assert!(!mutable);
            assert_ne!(local_id, HirLocalId(u32::MAX));
            assert_eq!(scoped_local_id, Some(local_id));
            let span = lowerer
                .source_map
                .local_span(owner, local_id)
                .expect("local source span");
            assert_eq!(span.file_path, binding_span.file_path);
            assert_eq!((span.start, span.end), (8, 13));
        }
        other => panic!("expected binding pattern, got {other:?}"),
    }
}

#[test]
fn lower_struct_pattern_records_struct_and_field_ids() {
    let source = r#"
struct Box
    < value: I64

read: Box -> I64
read = box ->
    match box
        Box value: value => value
"#;

    let resolved = compile_source_to_resolved_hir_for_test(source);
    let (_, read) = resolved.program.test_function_by_name("read").unwrap();
    let (struct_id, struct_def) = resolved.program.test_struct_by_name("Box").unwrap();
    let expected_field_id = struct_def
        .fields
        .iter()
        .find(|field| field.name == "value")
        .unwrap()
        .id;

    let pattern = find_first_struct_pattern(&read.body).unwrap();
    assert_eq!(*pattern.1, Some(struct_id));
    let location = pattern.3[0]
        .field
        .as_ref()
        .expect("struct pattern field must be resolved");
    assert_eq!(location.owner, struct_id);
    assert_eq!(location.field_id, expected_field_id);
    assert_eq!(location.name, "value");
    assert_eq!(pattern.3[0].name, "value");
}

#[test]
fn unresolved_unqualified_enum_pattern_keeps_ambiguous_location_unresolved() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_enumeration(unit_enum(def_id(30), "First", 0, "Hit"));
    lowerer
        .items
        .insert_enumeration(unit_enum(def_id(31), "Second", 1, "Hit"));
    let expected_ty = lowerer.engine.fresh_type_var();
    let pattern = unqualified_unit_pattern("Hit");

    let lowered =
        lowerer.with_test_body_context(|lowerer| lowerer.lower_pattern(&pattern, &expected_ty));

    match lowered {
        HirPattern::Enum(_, variant_name, location, _) => {
            assert_eq!(variant_name, "Hit");
            assert!(location.is_none());
        }
        other => panic!("expected enum pattern, got {other:?}"),
    }
}

#[test]
fn lower_enum_pattern_records_enum_and_variant_ids() {
    let source = r#"
enum Maybe
    Some I64
    None

read: Maybe -> I64
read = value ->
    match value
        Some x => x
        None => 0
"#;

    let resolved = compile_source_to_resolved_hir_for_test(source);
    let (_, read) = resolved.program.test_function_by_name("read").unwrap();
    let (enum_id, enum_def) = resolved.program.test_enum_by_name("Maybe").unwrap();
    let some_variant = enum_def
        .variants
        .iter()
        .find(|variant| variant.name == "Some")
        .unwrap();

    let pattern = find_first_enum_pattern(&read.body).unwrap();
    let location = pattern.2.as_ref().expect("enum pattern must be resolved");
    assert_eq!(location.owner, enum_id);
    assert_eq!(location.variant_id, some_variant.id);
    assert_eq!(location.name, "Some");
}

#[test]
fn qualified_named_enum_field_pattern_records_variant_location() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_enumeration(named_field_enum());
    lowerer
        .resolver
        .item_paths
        .insert("Record".to_string(), def_id(40));
    lowerer
        .resolver
        .item_names_by_id
        .insert(def_id(40), "Record".to_string());
    let expected_ty = Type::Enum {
        id: def_id(40),
        args: Vec::new(),
    };

    let lowered = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_pattern(&qualified_named_enum_field_pattern(), &expected_ty)
    });

    match lowered {
        HirPattern::Enum(enum_name, variant_name, Some(location), patterns) => {
            assert_eq!(enum_name, "Record");
            assert_eq!(variant_name, "Pair");
            assert_eq!(location.owner, def_id(40));
            assert_eq!(location.variant_id, VariantId(0));
            assert_eq!(location.name, "Pair");
            assert_eq!(patterns.len(), 1);
        }
        other => panic!("expected resolved enum field pattern, got {other:?}"),
    }
}

#[test]
fn qualified_named_enum_field_pattern_preserves_omitted_field_positions() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_enumeration(two_field_named_enum());
    lowerer
        .resolver
        .item_paths
        .insert("Record2".to_string(), def_id(41));
    lowerer
        .resolver
        .item_names_by_id
        .insert(def_id(41), "Record2".to_string());
    let expected_ty = Type::Enum {
        id: def_id(41),
        args: Vec::new(),
    };

    let lowered = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_pattern(&qualified_second_named_enum_field_pattern(), &expected_ty)
    });

    match lowered {
        HirPattern::Enum(_, _, Some(location), patterns) => {
            assert_eq!(location.owner, def_id(41));
            assert_eq!(patterns.len(), 2);
            assert!(matches!(patterns[0], HirPattern::Wildcard));
            match &patterns[1] {
                HirPattern::Binding { name, .. } => assert_eq!(name, "second"),
                other => panic!("expected second field binding, got {other:?}"),
            }
        }
        other => panic!("expected resolved enum field pattern, got {other:?}"),
    }
}

#[test]
fn qualified_named_enum_field_pattern_rejects_unknown_field() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_enumeration(two_field_named_enum());
    lowerer
        .resolver
        .item_paths
        .insert("Record2".to_string(), def_id(41));
    lowerer
        .resolver
        .item_names_by_id
        .insert(def_id(41), "Record2".to_string());
    let expected_ty = Type::Enum {
        id: def_id(41),
        args: Vec::new(),
    };

    let lowered =
        lowerer.lower_pattern(&qualified_unknown_named_enum_field_pattern(), &expected_ty);

    assert!(matches!(lowered, HirPattern::Wildcard));
    assert!(
        lowerer
            .diagnostics
            .errors()
            .iter()
            .any(|error| error.message.contains("has no field 'typo'")),
        "expected unknown field diagnostic, got {:?}",
        lowerer.diagnostics.errors()
    );
}
