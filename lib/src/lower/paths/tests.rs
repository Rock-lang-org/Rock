use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

use crate::ast::{
    Block, Expression, Ident, IdentOrType, IdentPattern, IdentifierPath, LambdaArrowKind,
    LambdaDecl, Operand, Pattern, PatternKind, PrimaryExpr, Statement, UnaryExpr,
};
use crate::crate_system::CrateContext;
use crate::ids::{CrateId, DefId, FieldId, HirLocalId, LocalDefId, VariantId};
use crate::infer::ResolvedHirProgram;
use crate::lower::body_context::{BodyLoweringContext, BodyOwner};
use crate::types::{
    CallableKind, CaptureKind, GenericParamDecl, GenericParamId, Predicate, TraitBound, Type,
};
use crate::{collect, infer, lower, macro_expansion, parser, Config};

static LOWER_PATH_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn compile_source_to_resolved_hir_for_test(source: &str) -> ResolvedHirProgram {
    let id = LOWER_PATH_TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "rock_lower_path_hir_ref_test_{}_{}",
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

fn find_first_struct_literal(
    block: &crate::hir::AcceptedHirBlock,
) -> Option<(
    &String,
    &Option<crate::ids::DefId>,
    &Vec<crate::hir::HirStructLiteralFieldFor<crate::hir::AcceptedHir>>,
)> {
    block.stmts.iter().find_map(|stmt| match stmt {
        crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
            find_first_struct_literal_expr(expr)
        }
        crate::hir::HirStmtFor::Let { value, .. } => find_first_struct_literal_expr(value),
        _ => None,
    })
}

fn find_first_struct_literal_expr(
    expr: &crate::hir::AcceptedHirExpr,
) -> Option<(
    &String,
    &Option<crate::ids::DefId>,
    &Vec<crate::hir::HirStructLiteralFieldFor<crate::hir::AcceptedHir>>,
)> {
    match &expr.kind {
        crate::hir::HirExprKindFor::StructLiteral(name, id, fields) => Some((name, id, fields)),
        crate::hir::HirExprKindFor::Call(callee, args, _) => find_first_struct_literal_expr(callee)
            .or_else(|| args.iter().find_map(find_first_struct_literal_expr)),
        crate::hir::HirExprKindFor::Block(block)
        | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_struct_literal(block),
        _ => None,
    }
}

fn find_static_method_target(expr: &crate::hir::AcceptedHirExpr) -> Option<&HirStaticMethodTarget> {
    match &expr.kind {
        crate::hir::HirExprKindFor::Call(
            _,
            _,
            Some(crate::hir::HirCallTarget::StaticMethod(target)),
        ) => Some(target),
        crate::hir::HirExprKindFor::Call(callee, args, _) => find_static_method_target(callee)
            .or_else(|| args.iter().find_map(find_static_method_target)),
        crate::hir::HirExprKindFor::Lambda { body, .. }
        | crate::hir::HirExprKindFor::Block(body)
        | crate::hir::HirExprKindFor::UnsafeBlock(body) => {
            body.stmts.iter().find_map(|stmt| match stmt {
                crate::hir::HirStmtFor::Let { value, .. }
                | crate::hir::HirStmtFor::Expr(value)
                | crate::hir::HirStmtFor::Return(Some(value))
                | crate::hir::HirStmtFor::Break(Some(value)) => find_static_method_target(value),
                _ => None,
            })
        }
        _ => None,
    }
}

fn find_first_enum_variant(
    block: &crate::hir::AcceptedHirBlock,
) -> Option<(
    &String,
    &String,
    &Vec<crate::hir::AcceptedHirExpr>,
    &Option<crate::hir::HirVariantLocation>,
)> {
    block.stmts.iter().find_map(|stmt| match stmt {
        crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
            find_first_enum_variant_expr(expr)
        }
        crate::hir::HirStmtFor::Let { value, .. } => find_first_enum_variant_expr(value),
        _ => None,
    })
}

fn find_first_enum_variant_expr(
    expr: &crate::hir::AcceptedHirExpr,
) -> Option<(
    &String,
    &String,
    &Vec<crate::hir::AcceptedHirExpr>,
    &Option<crate::hir::HirVariantLocation>,
)> {
    match &expr.kind {
        crate::hir::HirExprKindFor::EnumVariant(enum_name, variant_name, args, location) => {
            Some((enum_name, variant_name, args, location))
        }
        crate::hir::HirExprKindFor::Call(callee, args, _) => find_first_enum_variant_expr(callee)
            .or_else(|| args.iter().find_map(find_first_enum_variant_expr)),
        crate::hir::HirExprKindFor::Block(block)
        | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_enum_variant(block),
        _ => None,
    }
}

fn find_first_resolved_var(block: &crate::hir::AcceptedHirBlock) -> Option<&HirVarRef> {
    block.stmts.iter().find_map(|stmt| match stmt {
        crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
            find_first_resolved_var_expr(expr)
        }
        _ => None,
    })
}

fn find_first_resolved_var_expr(expr: &crate::hir::AcceptedHirExpr) -> Option<&HirVarRef> {
    match &expr.kind {
        crate::hir::HirExprKindFor::ResolvedVar(reference) => Some(reference),
        crate::hir::HirExprKindFor::Call(callee, args, _) => find_first_resolved_var_expr(callee)
            .or_else(|| args.iter().find_map(find_first_resolved_var_expr)),
        crate::hir::HirExprKindFor::Block(block)
        | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_resolved_var(block),
        _ => None,
    }
}

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

#[test]
fn static_method_value_lambda_carries_exact_target() {
    let resolved = compile_source_to_resolved_hir_for_test(
            "struct Math\n\nimpl Math\n    double = value -> value\n\nmain = ->\n    operation = Math::double\n    operation 6\n",
        );
    let program = resolved.program.program();
    let imp = program
        .impls
        .values()
        .find(|imp| imp.methods.contains_key("double"))
        .expect("static impl");
    let method_id = imp.methods["double"].id;
    let (_, main) = program.test_function_by_name("main").expect("main");
    let value = main
        .body
        .stmts
        .iter()
        .find_map(|stmt| match stmt {
            HirStmtFor::Let { value, .. } => Some(value),
            _ => None,
        })
        .expect("static method value binding");
    let HirExprKindFor::Lambda { body, .. } = &value.kind else {
        panic!("expected static method value lambda, got {:?}", value.kind);
    };
    let [HirStmtFor::Expr(HirExprFor {
        kind: HirExprKindFor::Call(_, _, Some(HirCallTarget::StaticMethod(target))),
        ..
    })] = body.stmts.as_slice()
    else {
        panic!("expected exact static call in generated lambda")
    };

    assert_eq!(target.method.impl_id(), Some(imp.id));
    assert_eq!(target.method.method_id(), Some(method_id));
}

#[test]
fn generic_box_static_constructor_value_lowers() {
    let resolved = compile_source_to_resolved_hir_for_test(
        r#"
struct Box T
    value: T

impl Box T
    new = value ->
        Box
            value: value

main = ->
    Box::new 21
"#,
    );

    assert!(resolved.program.test_function_by_name("main").is_some());
}

fn resolved_generic_static_method(
    _lowerer: &mut Lowerer,
    impl_id: DefId,
    method_id: DefId,
    struct_id: DefId,
    owner_param: GenericParamId,
    method_params: Vec<GenericParamId>,
    params: Vec<Type>,
    ret: Type,
) -> crate::lower::resolution::LowerResolvedStaticMethod {
    let mut method = test_function(method_id, "new");
    method.generic_params = method_params
        .iter()
        .enumerate()
        .map(|(index, param)| GenericParamDecl::type_param(*param, format!("Method{index}")))
        .collect();
    method.params = params
        .iter()
        .enumerate()
        .map(|(index, ty)| HirParam {
            name: format!("arg{index}"),
            local_id: HirLocalId(index as u32),
            ty: ty.clone(),
            mutable: false,
            is_ref: false,
        })
        .collect();
    method.ret_type = ret.clone();
    crate::lower::resolution::LowerResolvedStaticMethod {
        value: crate::lower::resolution::LowerResolvedValue {
            name: "Box::new".to_string(),
            ty: Type::function(params, ret),
            target: Some(HirVarTarget::Function(method_id)),
            is_alias: false,
            scope_index: None,
            should_instantiate: true,
        },
        target: HirMethodCallTarget::impl_method(impl_id, method_id, None),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: struct_id,
            args: vec![Type::Generic(owner_param)],
        }),
        owner_generic_params: vec![owner_param],
        method_generic_params: method_params,
    }
}

#[test]
fn trait_backed_structural_static_owner_substitutes_trait_args_for_artifact_validation() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(160);
    let trait_id = def_id(159);
    let trait_member_id = def_id(1600);
    let impl_id = def_id(161);
    let method_id = def_id(162);
    let owner_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let mut resolved = resolved_generic_static_method(
        &mut lowerer,
        impl_id,
        method_id,
        struct_id,
        owner_param,
        vec![method_param],
        vec![Type::Struct {
            id: struct_id,
            args: vec![Type::Generic(method_param)],
        }],
        Type::Unit,
    );
    resolved.target = HirMethodCallTarget::impl_method(
        impl_id,
        method_id,
        Some(HirSelectedTraitMember {
            trait_id,
            member_id: trait_member_id,
            trait_args: vec![Type::Generic(owner_param)],
        }),
    );

    let (_, target) = lowerer
        .with_test_body_context(|lowerer| {
            lowerer.instantiate_static_method_value(resolved, crate::lexer::Span::test())
        })
        .expect("structural owner inference");

    assert_eq!(target.method.owner_substitution[0].param, owner_param);
    assert_eq!(target.method.method_substitution[0].param, method_param);
    assert_eq!(
        target.method.owner_substitution[0].ty,
        target.method.method_substitution[0].ty
    );
    assert!(matches!(
        target.method.owner_substitution[0].ty,
        Type::TypeVar(_)
    ));
    let HirSelectedMethodTarget::ImplMethod {
        selected_trait: Some(selected_trait),
        ..
    } = target.method.target
    else {
        panic!("expected trait-backed static method target")
    };
    assert_eq!(
        selected_trait.trait_args,
        vec![target.method.owner_substitution[0].ty.clone()]
    );
}

#[test]
fn structural_static_owner_fallback_preserves_nested_method_generic_shape() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(163);
    let impl_id = def_id(164);
    let method_id = def_id(165);
    let owner_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let resolved = resolved_generic_static_method(
        &mut lowerer,
        impl_id,
        method_id,
        struct_id,
        owner_param,
        vec![method_param],
        vec![Type::Struct {
            id: struct_id,
            args: vec![Type::Tuple(vec![Type::Generic(method_param)])],
        }],
        Type::Unit,
    );

    let (_, target) = lowerer
        .with_test_body_context(|lowerer| {
            lowerer.instantiate_static_method_value(resolved, crate::lexer::Span::test())
        })
        .expect("nested structural owner inference");

    assert_eq!(target.method.owner_substitution[0].param, owner_param);
    assert_eq!(target.method.method_substitution[0].param, method_param);
    assert_eq!(
        target.method.owner_substitution[0].ty,
        Type::Tuple(vec![target.method.method_substitution[0].ty.clone()])
    );
}

#[test]
fn structural_static_owner_fallback_visits_projection_nominal_types() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(169);
    let impl_id = def_id(170);
    let method_id = def_id(171);
    let trait_id = def_id(172);
    let owner_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let resolved = resolved_generic_static_method(
        &mut lowerer,
        impl_id,
        method_id,
        struct_id,
        owner_param,
        vec![method_param],
        vec![Type::Projection {
            ty: Box::new(Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(method_param)],
            }),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: crate::ids::AssocTypeId(0),
            },
            trait_args: Vec::new(),
        }],
        Type::Unit,
    );

    let (_, target) = lowerer
        .with_test_body_context(|lowerer| {
            lowerer.instantiate_static_method_value(resolved, crate::lexer::Span::test())
        })
        .expect("projection structural owner inference");

    assert_eq!(
        target.method.owner_substitution[0].ty,
        target.method.method_substitution[0].ty
    );
    assert!(matches!(
        target.method.owner_substitution[0].ty,
        Type::TypeVar(_)
    ));
}

#[test]
fn conflicting_structural_static_owner_occurrences_are_an_error() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(166);
    let impl_id = def_id(167);
    let method_id = def_id(168);
    let owner_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let left_method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let right_method_param = GenericParamId {
        owner: method_id,
        index: 1,
    };
    lowerer
        .items
        .insert_structure(empty_struct(struct_id, "Box"));
    register_item_path(&mut lowerer, "Box", struct_id);
    let mut method = test_function(method_id, "new");
    method.generic_params = vec![
        GenericParamDecl::type_param(left_method_param, "Left"),
        GenericParamDecl::type_param(right_method_param, "Right"),
    ];
    method.params = vec![
        HirParam {
            name: "left".to_string(),
            local_id: HirLocalId(0),
            ty: Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(left_method_param)],
            },
            mutable: false,
            is_ref: false,
        },
        HirParam {
            name: "right".to_string(),
            local_id: HirLocalId(1),
            ty: Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(right_method_param)],
            },
            mutable: false,
            is_ref: false,
        },
    ];
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: vec![GenericParamDecl::type_param(owner_param, "Owner")],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(owner_param)],
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("new".to_string(), method)]),
        })
        .unwrap();

    let expr = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_identifier_path(&identifier_path(&["Box", "new"]))
    });

    assert_eq!(expr.ty, Type::Error);
    assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "<error>"));
    let diagnostic = lowerer
        .diagnostics
        .errors()
        .iter()
        .find(|error| error.message.contains("conflicting owner inference"))
        .expect("conflicting owner inference diagnostic");
    assert!(diagnostic.message.contains("_ vs _"));
    assert!(!diagnostic.message.contains("GenericParamId"));
}

fn register_item_path(lowerer: &mut Lowerer, path: &str, id: DefId) {
    lowerer.resolver.item_paths.insert(path.to_string(), id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(id, path.to_string());
}

fn static_method_value_parts(expr: &HirExpr) -> (&HirVarRef, &HirStaticMethodTarget) {
    let HirExprKind::Lambda { body, .. } = &expr.kind else {
        panic!("expected static method value lambda, got {:?}", expr.kind);
    };
    let [HirStmt::Expr(HirExpr {
        kind: HirExprKind::Call(callee, _, Some(HirCallTarget::StaticMethod(static_target))),
        ..
    })] = body.stmts.as_slice()
    else {
        panic!("expected exact static call in generated lambda")
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("expected resolved static callee")
    };
    (reference, static_target)
}

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: crate::lexer::Span::test(),
    }
}

fn ident_pattern(name: &str) -> Pattern {
    Pattern {
        binding: Some(ident(name)),
        kind: PatternKind::Ident(IdentPattern {
            name: ident(name),
            mut_: false,
        }),
    }
}

fn var_expr(name: &str) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Ident(IdentifierPath {
            path: vec![IdentOrType::Ident(ident(name))],
        }),
        secondaries: None,
        type_annotation: None,
    }))
}

fn int_expr(value: &str) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Literal(crate::ast::Literal {
            kind: crate::ast::LiteralKind::Number(value.parse().unwrap()),
            span: crate::lexer::Span::test(),
        }),
        secondaries: None,
        type_annotation: None,
    }))
}

fn identifier_path(names: &[&str]) -> IdentifierPath {
    IdentifierPath {
        path: names
            .iter()
            .map(|name| IdentOrType::Ident(ident(name)))
            .collect(),
    }
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
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn register_static_impl(
    lowerer: &mut Lowerer,
    impl_id: DefId,
    owner_id: DefId,
    owner_name: &str,
    method_name: &str,
    method: HirFunction,
) {
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named(owner_name.to_string()),
            type_name: owner_name.to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner_id,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([(method_name.to_string(), method)]),
        })
        .unwrap();
}

#[test]
fn local_reads_resolve_to_shadowing_local_ids() {
    let mut lowerer = Lowerer::new_for_test();
    let (inner, expr) = lowerer.with_test_body_context(|lowerer| {
        let outer = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("x".to_string(), Type::I64, false, outer);
        lowerer.scope.push();
        let inner = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("x".to_string(), Type::Bool, false, inner);
        (
            inner,
            lowerer.lower_identifier_path(&identifier_path(&["x"])),
        )
    });

    match expr.kind {
        HirExprKind::ResolvedVar(HirVarRef {
            target: HirVarTarget::Local(id),
            name,
        }) => {
            assert_eq!(name, "x");
            assert_eq!(id, inner);
        }
        other => panic!("expected resolved local var, got {other:?}"),
    }
}

#[test]
fn parameter_reads_resolve_to_parameter_local_ids() {
    let resolved = compile_source_to_resolved_hir_for_test(
        r#"
identity = x -> x
"#,
    );
    let (_, function) = resolved.program.test_function_by_name("identity").unwrap();
    let param_id = function.params[0].local_id;

    let var = find_first_resolved_var(&function.body).unwrap();
    assert_eq!(var.name, "x");
    assert_eq!(var.target, HirVarTarget::Local(param_id));
}

fn point_struct() -> HirStruct {
    HirStruct {
        id: def_id(10),
        name: "Point".to_string(),
        generic_params: Vec::new(),
        fields: vec![
            HirField {
                id: FieldId(0),
                name: "x".to_string(),
                ty: Type::I64,
                public: true,
            },
            HirField {
                id: FieldId(1),
                name: "y".to_string(),
                ty: Type::I64,
                public: true,
            },
        ],
    }
}

fn option_enum() -> HirEnum {
    HirEnum {
        id: def_id(20),
        name: "Maybe".to_string(),
        generic_params: Vec::new(),
        variants: vec![HirVariant {
            id: VariantId(0),
            name: "Some".to_string(),
            fields: HirVariantFields::Positional(vec![Type::I64]),
        }],
    }
}

fn record_enum() -> HirEnum {
    HirEnum {
        id: def_id(21),
        name: "Record".to_string(),
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
                HirField {
                    id: FieldId(2),
                    name: "third".to_string(),
                    ty: Type::I64,
                    public: true,
                },
                HirField {
                    id: FieldId(3),
                    name: "fourth".to_string(),
                    ty: Type::I64,
                    public: true,
                },
            ]),
        }],
    }
}

fn single_unit_variant_enum(id: DefId, name: &str, variant_name: &str) -> HirEnum {
    HirEnum {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        variants: vec![HirVariant {
            id: VariantId(0),
            name: variant_name.to_string(),
            fields: HirVariantFields::Unit,
        }],
    }
}

fn empty_struct(id: DefId, name: &str) -> HirStruct {
    HirStruct {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    }
}

fn private_field_struct(id: DefId, name: &str) -> HirStruct {
    HirStruct {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        fields: vec![HirField {
            id: FieldId(0),
            name: "ptr".to_string(),
            ty: Type::I64,
            public: false,
        }],
    }
}

#[test]
fn lower_struct_literal_records_struct_id() {
    let source = r#"
struct Box
    < value: I64

main = ->
    box = Box
        value: 7
    box.value
"#;

    let resolved = compile_source_to_resolved_hir_for_test(source);
    let (_, main) = resolved.program.test_function_by_name("main").unwrap();
    let struct_id = resolved.program.names.structs_by_name["Box"];

    let literal = find_first_struct_literal(&main.body).unwrap();
    assert_eq!(literal.1, &Some(struct_id));
}

#[test]
fn lower_enum_variant_records_enum_and_variant_ids() {
    let source = r#"
enum Maybe
    Some I64
    None

main = ->
    Maybe::Some 7
"#;

    let resolved = compile_source_to_resolved_hir_for_test(source);
    let (_, main) = resolved.program.test_function_by_name("main").unwrap();
    let (enum_id, maybe_enum) = resolved.program.test_enum_by_name("Maybe").unwrap();
    let expected_variant_id = maybe_enum
        .variants
        .iter()
        .find(|variant| variant.name == "Some")
        .expect("Maybe::Some exists")
        .id;

    let variant = find_first_enum_variant(&main.body).unwrap();
    let location = variant.3.as_ref().expect("enum variant must be resolved");
    assert_eq!(location.owner, enum_id);
    assert_eq!(location.variant_id, expected_variant_id);
    assert_eq!(location.name, "Some");
}

#[test]
fn lower_direct_function_reference_records_function_id() {
    let source = r#"
answer = -> 7

main = ->
    answer!
"#;

    let resolved = compile_source_to_resolved_hir_for_test(source);
    let (answer_id, _) = resolved.program.test_function_by_name("answer").unwrap();
    let (_, main) = resolved.program.test_function_by_name("main").unwrap();

    let var = find_first_resolved_var(&main.body).unwrap();
    assert_eq!(var.name, "answer");
    assert_eq!(var.target, HirVarTarget::Function(answer_id));
}

#[test]
fn lower_direct_extern_reference_records_extern_id() {
    let source = r#"
extern puts: I32 -> I32

main = ->
    puts 0
"#;

    let resolved = compile_source_to_resolved_hir_for_test(source);
    let extern_id = resolved.program.names.externs_by_name["puts"];
    let (_, main) = resolved.program.test_function_by_name("main").unwrap();

    let var = find_first_resolved_var(&main.body).unwrap();
    assert_eq!(var.name, "puts");
    assert_eq!(var.target, HirVarTarget::Extern(extern_id));
}

#[test]
fn lower_qualified_function_reference_records_function_id() {
    let mut lowerer = Lowerer::new_for_test();
    let answer_id = def_id(30);
    lowerer
        .items
        .insert_function(test_function(answer_id, "answer"));
    register_item_path(&mut lowerer, "pkg::answer", answer_id);

    let expr = lowerer.lower_identifier_path(&identifier_path(&["pkg", "answer"]));

    match expr.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "pkg::answer");
            assert_eq!(reference.target, HirVarTarget::Function(answer_id));
        }
        other => panic!("expected resolved function reference, got {other:?}"),
    }
}

#[test]
fn lower_static_impl_reference_records_method_id() {
    let mut lowerer = Lowerer::new_for_test();
    let box_id = def_id(40);
    lowerer.items.insert_structure(HirStruct {
        id: box_id,
        name: "Box".to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    });
    register_item_path(&mut lowerer, "Box", box_id);
    let method_id = def_id(41);
    register_static_impl(
        &mut lowerer,
        def_id(42),
        box_id,
        "Box",
        "make",
        test_function(method_id, "make"),
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["Box", "make"]));

    let (reference, target) = static_method_value_parts(&expr);
    assert_eq!(reference.name, "Box::make");
    assert_eq!(reference.target, HirVarTarget::Function(method_id));
    assert_eq!(target.method.impl_id(), Some(def_id(42)));
    assert_eq!(target.method.method_id(), Some(method_id));
}

#[test]
fn constructor_trait_selection_concrete_call_records_exact_impl_authority() {
    let hir = compile_source_to_resolved_hir_for_test(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Maybe T
    None
    Some T

impl Applicative for Maybe
    pure = value -> Maybe::Some value

main = ->
    value = Maybe::pure 42
    0
"#,
    );
    let main = hir
        .program
        .functions
        .values()
        .find(|function| function.name == "main")
        .expect("main function");
    let value = main
        .body
        .stmts
        .iter()
        .find_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Let { value, .. } => Some(value),
            _ => None,
        })
        .expect("lowered constructor call");
    let crate::hir::HirExprKindFor::Call(callee, _, _) = &value.kind else {
        panic!("expected call of static method value, got {:?}", value.kind);
    };
    let crate::hir::HirExprKindFor::Lambda { body, .. } = &callee.kind else {
        panic!("expected static method lambda, got {:?}", callee.kind);
    };
    let target = body
        .stmts
        .iter()
        .find_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Expr(crate::hir::HirExprFor {
                kind:
                    crate::hir::HirExprKindFor::Call(
                        _,
                        _,
                        Some(crate::hir::HirCallTarget::StaticMethod(target)),
                    ),
                ..
            }) => Some(target),
            _ => None,
        })
        .expect("static method authority");

    assert!(target.method.impl_id().is_some());
    assert!(target.method.trait_id().is_some());
    assert!(target.method.method_id().is_some());
    assert!(matches!(target.owner_ty, Type::Constructor { .. }));
}

#[test]
fn constructor_trait_selection_generic_calls_record_deferred_bound_authority() {
    let hir = compile_source_to_resolved_hir_for_test(
        r#"
trait Applicative for F _
    pure: A -> F A

make: A -> F A where F _: Applicative
make = value -> F::pure value

make_qualified: A -> F A where F _: Applicative
make_qualified = value -> F::Applicative::pure value
"#,
    );

    for function_name in ["make", "make_qualified"] {
        let function = hir
            .program
            .functions
            .values()
            .find(|function| function.name == function_name)
            .unwrap_or_else(|| panic!("{function_name} function"));
        let target = function
            .body
            .stmts
            .iter()
            .find_map(|stmt| match stmt {
                crate::hir::HirStmtFor::Expr(expr)
                | crate::hir::HirStmtFor::Return(Some(expr))
                | crate::hir::HirStmtFor::Let { value: expr, .. } => {
                    find_static_method_target(expr)
                }
                _ => None,
            })
            .expect("deferred constructor authority");

        assert!(target.method.impl_id().is_none());
        assert!(matches!(
            target.method.target,
            HirSelectedMethodTarget::TraitMethod {
                dispatch: HirTraitDispatchKind::TraitBound,
                ..
            }
        ));
        assert!(matches!(target.owner_ty, Type::Generic(_)));
        assert!(target.method.trait_id().is_some());
        assert!(target.method.method_id().is_some());
        assert!(!target.method.owner_substitution.is_empty());
    }
}

#[test]
fn constructor_trait_selection_result_section_preserves_fixed_error() {
    let hir = compile_source_to_resolved_hir_for_test(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Outcome T, E
    Ok T
    Err E

struct IoError

impl Applicative for Outcome _, E
    pure = value -> Outcome::Ok value

main = ->
    value = (Outcome _, IoError)::pure 42
    0
"#,
    );
    let io_error_id = hir
        .program
        .structs
        .values()
        .find(|structure| structure.name == "IoError")
        .expect("IoError structure")
        .id;
    let main = hir
        .program
        .functions
        .values()
        .find(|function| function.name == "main")
        .expect("main function");
    let target = main
        .body
        .stmts
        .iter()
        .find_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Let { value, .. } => find_static_method_target(value),
            _ => None,
        })
        .expect("selected Outcome section authority");

    assert!(target.method.impl_id().is_some());
    assert!(target.method.owner_substitution.iter().any(|binding| {
        binding.ty
            == Type::Struct {
                id: io_error_id,
                args: Vec::new(),
            }
    }));
}

#[test]
fn constructor_trait_selection_preserves_inherent_precedence_and_qualification() {
    let hir = compile_source_to_resolved_hir_for_test(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Maybe T
    None
    Some T

impl Maybe T
    pure = value -> Maybe::None

impl Applicative for Maybe
    pure = value -> Maybe::Some value

main = ->
    inherent: Maybe I64 = Maybe::pure 1
    qualified: Maybe I64 = Maybe::Applicative::pure 2
    0
"#,
    );
    let main = hir
        .program
        .functions
        .values()
        .find(|function| function.name == "main")
        .expect("main function");
    let targets = main
        .body
        .stmts
        .iter()
        .filter_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Let { value, .. } => find_static_method_target(value),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(targets.len(), 2);
    assert!(matches!(
        targets[0].method.target,
        HirSelectedMethodTarget::ImplMethod {
            selected_trait: None,
            ..
        }
    ));
    assert!(matches!(
        targets[1].method.target,
        HirSelectedMethodTarget::ImplMethod {
            selected_trait: Some(_),
            ..
        }
    ));
}

#[test]
fn generic_static_method_value_preserves_owner_generic_param_id() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(43);
    let impl_id = def_id(44);
    let method_id = def_id(45);
    let owner_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    lowerer.items.insert_structure(HirStruct {
        id: struct_id,
        name: "Widget".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: struct_id,
                index: 0,
            },
            "StructDisplay",
        )],
        fields: Vec::new(),
    });
    register_item_path(&mut lowerer, "Widget", struct_id);

    let mut method = test_function(method_id, "create");
    method.generic_params = vec![GenericParamDecl::type_param(method_param, "MethodDisplay")];
    method.params = vec![
        HirParam {
            name: "owner".to_string(),
            local_id: HirLocalId(0),
            ty: Type::Generic(owner_param),
            mutable: false,
            is_ref: false,
        },
        HirParam {
            name: "value".to_string(),
            local_id: HirLocalId(1),
            ty: Type::Generic(method_param),
            mutable: false,
            is_ref: false,
        },
    ];
    method.ret_type = Type::Tuple(vec![
        Type::Generic(owner_param),
        Type::Generic(method_param),
    ]);
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Widget".to_string()),
            type_name: "Widget".to_string(),
            type_generics: vec![GenericParamDecl::type_param(owner_param, "ImplDisplay")],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(owner_param)],
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("create".to_string(), method)]),
        })
        .unwrap();

    let expr = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_identifier_path(&identifier_path(&["Widget", "create"]))
    });

    let (reference, target) = static_method_value_parts(&expr);
    assert_eq!(reference.target, HirVarTarget::Function(method_id));
    assert_eq!(target.method.impl_id(), Some(impl_id));
    assert_eq!(target.method.method_id(), Some(method_id));
    assert_eq!(target.method.owner_substitution.len(), 1);
    assert_eq!(target.method.owner_substitution[0].param, owner_param);
    assert_eq!(target.method.method_substitution.len(), 1);
    assert_eq!(target.method.method_substitution[0].param, method_param);
}

#[test]
fn enum_static_method_value_preserves_exact_impl_and_method_authority() {
    let mut lowerer = Lowerer::new_for_test();
    let enum_id = def_id(50);
    let impl_id = def_id(51);
    let method_id = def_id(52);
    lowerer.items.insert_enumeration(HirEnum {
        id: enum_id,
        name: "Choice".to_string(),
        generic_params: Vec::new(),
        variants: Vec::new(),
    });
    register_item_path(&mut lowerer, "Choice", enum_id);
    let method = test_function(method_id, "default");
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Choice".to_string()),
            type_name: "Choice".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Enum {
                id: enum_id,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("default".to_string(), method)]),
        })
        .unwrap();

    let expr = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_identifier_path(&identifier_path(&["Choice", "default"]))
    });

    let (reference, target) = static_method_value_parts(&expr);
    assert_eq!(reference.target, HirVarTarget::Function(method_id));
    assert_eq!(target.method.impl_id(), Some(impl_id));
    assert_eq!(target.method.method_id(), Some(method_id));
}

#[test]
fn canonical_static_owner_without_method_is_an_error_not_a_raw_var() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(53);
    lowerer
        .items
        .insert_structure(empty_struct(struct_id, "Widget"));
    register_item_path(&mut lowerer, "Widget", struct_id);

    let expr = lowerer.lower_identifier_path(&identifier_path(&["Widget", "missing"]));

    assert_eq!(expr.ty, Type::Error);
    assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "<error>"));
    assert!(lowerer
        .diagnostics
        .errors()
        .iter()
        .any(|error| { error.message.contains("Widget::missing") && error.span().is_some() }));
}

#[test]
fn ambiguous_static_bound_method_is_an_error_without_internal_candidate_ids() {
    let mut lowerer = Lowerer::new_for_test();
    let generic_owner = def_id(53);
    let owner_param = GenericParamId {
        owner: generic_owner,
        index: 0,
    };
    let left_trait = def_id(54);
    let left_member = def_id(55);
    let right_trait = def_id(56);
    let right_member = def_id(57);
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: left_trait,
        name: "Left".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("build".to_string(), test_function(left_member, "build"))]),
        signatures: HashMap::new(),
    });
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: right_trait,
        name: "Right".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("build".to_string(), test_function(right_member, "build"))]),
        signatures: HashMap::new(),
    });
    let bounds = HashMap::from([(
        owner_param,
        vec![
            TraitBound {
                trait_id: right_trait,
                type_args: Vec::new(),
            },
            TraitBound {
                trait_id: left_trait,
                type_args: Vec::new(),
            },
        ],
    )]);

    let expr = lowerer.with_body_context(
        BodyLoweringContext::new(
            "test".to_string(),
            BodyOwner::Function(def_id(58)),
            Some(generic_owner),
            vec!["T".to_string()],
            bounds.into(),
            false,
        ),
        |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "build"])),
    );

    assert_eq!(expr.ty, Type::Error);
    assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "<error>"));
    let diagnostic = lowerer
        .diagnostics
        .errors()
        .iter()
        .find(|error| error.message.contains("ambiguous static bound method"))
        .expect("ambiguity diagnostic");
    assert!(diagnostic.message.contains("matching trait bounds"));
    assert!(!diagnostic.message.contains("DefId"));
    assert!(diagnostic.span().is_some());
}

#[test]
fn static_bound_method_lookup_expands_supertraits() {
    let mut lowerer = Lowerer::new_for_test();
    let generic_owner = def_id(59);
    let owner_param = GenericParamId {
        owner: generic_owner,
        index: 0,
    };
    let parent_trait = def_id(60);
    let parent_member = def_id(61);
    let child_trait = def_id(62);
    let child_target = GenericParamId {
        owner: child_trait,
        index: 0,
    };
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: parent_trait,
        name: "Parent".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("build".to_string(), test_function(parent_member, "build"))]),
        signatures: HashMap::new(),
    });
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: vec![Predicate::Trait {
            subject: Type::Generic(child_target),
            trait_id: parent_trait,
            args: Vec::new(),
        }],
        id: child_trait,
        name: "Child".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    });
    let bounds = HashMap::from([(
        owner_param,
        vec![TraitBound {
            trait_id: child_trait,
            type_args: Vec::new(),
        }],
    )]);

    let expr = lowerer.with_body_context(
        BodyLoweringContext::new(
            "test".to_string(),
            BodyOwner::Function(def_id(63)),
            Some(generic_owner),
            vec!["T".to_string()],
            bounds.into(),
            false,
        ),
        |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "build"])),
    );

    let (_, target) = static_method_value_parts(&expr);
    assert_eq!(target.method.trait_id(), Some(parent_trait));
    assert_eq!(target.method.method_id(), Some(parent_member));
}

#[test]
fn distinct_static_bound_trait_args_remain_ambiguous() {
    let mut lowerer = Lowerer::new_for_test();
    let generic_owner = def_id(64);
    let owner_param = GenericParamId {
        owner: generic_owner,
        index: 0,
    };
    let trait_id = def_id(65);
    let member_id = def_id(66);
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Factory".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "Arg",
        )],
        associated_types: Vec::new(),
        methods: HashMap::from([("build".to_string(), test_function(member_id, "build"))]),
        signatures: HashMap::new(),
    });
    let bounds = HashMap::from([(
        owner_param,
        vec![
            TraitBound {
                trait_id,
                type_args: vec![Type::I64],
            },
            TraitBound {
                trait_id,
                type_args: vec![Type::Bool],
            },
        ],
    )]);

    let expr = lowerer.with_body_context(
        BodyLoweringContext::new(
            "test".to_string(),
            BodyOwner::Function(def_id(67)),
            Some(generic_owner),
            vec!["T".to_string()],
            bounds.into(),
            false,
        ),
        |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "build"])),
    );

    assert_eq!(expr.ty, Type::Error);
    assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "<error>"));
    let diagnostic = lowerer
        .diagnostics
        .errors()
        .iter()
        .find(|error| error.message.contains("ambiguous static bound method"))
        .expect("ambiguity diagnostic");
    assert!(diagnostic.message.contains("Bool"));
    assert!(diagnostic.message.contains("I64"));
    assert!(diagnostic.message.find("Bool") < diagnostic.message.find("I64"));
}

#[test]
fn generic_static_bound_method_instantiates_method_generics_without_replacing_owner() {
    let mut lowerer = Lowerer::new_for_test();
    let generic_owner = def_id(68);
    let owner_param = GenericParamId {
        owner: generic_owner,
        index: 0,
    };
    let trait_id = def_id(69);
    let method_id = def_id(70);
    let trait_param = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let mut method = test_function(method_id, "make");
    method.generic_params = vec![GenericParamDecl::type_param(method_param, "MethodDisplay")];
    method.params = vec![HirParam {
        name: "value".to_string(),
        local_id: HirLocalId(0),
        ty: Type::Generic(method_param),
        mutable: false,
        is_ref: false,
    }];
    method.ret_type = Type::Tuple(vec![
        Type::Generic(trait_param),
        Type::Generic(method_param),
    ]);
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Factory".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "TraitDisplay",
        )],
        associated_types: Vec::new(),
        methods: HashMap::from([("make".to_string(), method)]),
        signatures: HashMap::new(),
    });
    let bounds = HashMap::from([(
        owner_param,
        vec![TraitBound {
            trait_id,
            type_args: vec![Type::Generic(owner_param)],
        }],
    )]);

    let expr = lowerer.with_body_context(
        BodyLoweringContext::new(
            "test".to_string(),
            BodyOwner::Function(def_id(71)),
            Some(generic_owner),
            vec!["T".to_string()],
            bounds.into(),
            false,
        ),
        |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "make"])),
    );

    let Type::Function { params, ret, .. } = &expr.ty else {
        panic!("expected generic static method value function type")
    };
    assert!(matches!(params.as_slice(), [Type::TypeVar(_)]));
    assert!(matches!(
        ret.as_ref(),
        Type::Tuple(elements)
            if matches!(elements.as_slice(), [Type::Generic(param), Type::TypeVar(_)] if *param == owner_param)
    ));
    let (_, target) = static_method_value_parts(&expr);
    assert_eq!(target.method.method_id(), Some(method_id));
    assert_eq!(target.method.method_substitution.len(), 1);
    assert_eq!(target.method.method_substitution[0].param, method_param);
    assert!(matches!(
        target.method.method_substitution[0].ty,
        Type::TypeVar(_)
    ));
    assert_eq!(target.method.trait_args(), &[Type::Generic(owner_param)]);
}

#[test]
fn identical_generic_static_bounds_select_one_fresh_method_target() {
    let mut lowerer = Lowerer::new_for_test();
    let generic_owner = def_id(72);
    let owner_param = GenericParamId {
        owner: generic_owner,
        index: 0,
    };
    let trait_id = def_id(73);
    let method_id = def_id(74);
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let mut method = test_function(method_id, "make");
    method.generic_params = vec![GenericParamDecl::type_param(method_param, "MethodDisplay")];
    method.params = vec![HirParam {
        name: "value".to_string(),
        local_id: HirLocalId(0),
        ty: Type::Generic(method_param),
        mutable: false,
        is_ref: false,
    }];
    method.ret_type = Type::Generic(method_param);
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Factory".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "TraitDisplay",
        )],
        associated_types: Vec::new(),
        methods: HashMap::from([("make".to_string(), method)]),
        signatures: HashMap::new(),
    });
    let bound = TraitBound {
        trait_id,
        type_args: vec![Type::Generic(owner_param)],
    };
    let bounds = HashMap::from([(owner_param, vec![bound.clone(), bound])]);

    let expr = lowerer.with_body_context(
        BodyLoweringContext::new(
            "test".to_string(),
            BodyOwner::Function(def_id(75)),
            Some(generic_owner),
            vec!["T".to_string()],
            bounds.into(),
            false,
        ),
        |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "make"])),
    );

    assert!(matches!(expr.ty, Type::Function { .. }));
    assert!(lowerer.diagnostics.errors().is_empty());
    let (_, target) = static_method_value_parts(&expr);
    assert_eq!(target.method.method_id(), Some(method_id));
    assert_eq!(target.method.trait_args(), &[Type::Generic(owner_param)]);
    assert_eq!(target.method.method_substitution.len(), 1);
    assert_eq!(target.method.method_substitution[0].param, method_param);
    assert!(matches!(
        target.method.method_substitution[0].ty,
        Type::TypeVar(_)
    ));
}

#[test]
fn trait_backed_generic_static_method_preserves_member_and_trait_args() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(59);
    let trait_id = def_id(60);
    let trait_member_id = def_id(61);
    let impl_id = def_id(62);
    let method_id = def_id(63);
    let owner_param = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_param = GenericParamId {
        owner: method_id,
        index: 0,
    };
    lowerer.items.insert_structure(HirStruct {
        id: struct_id,
        name: "Tagged".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: struct_id,
                index: 0,
            },
            "NominalDisplay",
        )],
        fields: Vec::new(),
    });
    register_item_path(&mut lowerer, "Tagged", struct_id);
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Factory".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "TraitArgumentDisplay",
        )],
        associated_types: Vec::new(),
        methods: HashMap::from([(
            "create".to_string(),
            test_function(trait_member_id, "create"),
        )]),
        signatures: HashMap::new(),
    });
    let mut method = test_function(method_id, "create");
    method.generic_params = vec![GenericParamDecl::type_param(method_param, "MethodDisplay")];
    method.params = vec![HirParam {
        name: "value".to_string(),
        local_id: HirLocalId(0),
        ty: Type::Struct {
            id: struct_id,
            args: vec![Type::Generic(method_param)],
        },
        mutable: false,
        is_ref: false,
    }];
    method.ret_type = Type::Struct {
        id: struct_id,
        args: vec![Type::Generic(method_param)],
    };
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Tagged".to_string()),
            type_name: "Tagged".to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                owner_param,
                "ImplOwnerDisplay",
            )],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(owner_param)],
            }),
            trait_name: Some("Factory".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(owner_param, "TraitDisplay")],
            trait_arg_types: vec![Type::Generic(owner_param)],
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("create".to_string(), method)]),
        })
        .unwrap();

    let expr = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_identifier_path(&identifier_path(&["Tagged", "create"]))
    });

    let (_, target) = static_method_value_parts(&expr);
    assert_eq!(target.method.impl_id(), Some(impl_id));
    assert_eq!(target.method.method_id(), Some(method_id));
    assert_eq!(target.method.owner_substitution.len(), 1);
    assert_eq!(target.method.owner_substitution[0].param, owner_param);
    assert_eq!(target.method.method_substitution.len(), 1);
    assert_eq!(target.method.method_substitution[0].param, method_param);
    assert!(matches!(
        target.method.owner_substitution[0].ty,
        Type::TypeVar(_)
    ));
    assert_eq!(
        target.method.owner_substitution[0].ty,
        target.method.method_substitution[0].ty
    );
    let HirSelectedMethodTarget::ImplMethod {
        selected_trait: Some(selected_trait),
        ..
    } = &target.method.target
    else {
        panic!("expected trait-backed impl method authority")
    };
    assert_eq!(selected_trait.trait_id, trait_id);
    assert_eq!(selected_trait.member_id, trait_member_id);
    assert_eq!(selected_trait.trait_args.len(), 1);
    assert_eq!(
        selected_trait.trait_args[0],
        target.method.owner_substitution[0].ty
    );
}

#[test]
fn trait_backed_static_method_with_unknown_trait_authority_is_an_error() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(46);
    let impl_id = def_id(47);
    let method_id = def_id(48);
    let missing_trait_id = def_id(49);
    lowerer
        .items
        .insert_structure(empty_struct(struct_id, "Widget"));
    register_item_path(&mut lowerer, "Widget", struct_id);
    let method = test_function(method_id, "create");
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Widget".to_string()),
            type_name: "Widget".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: Vec::new(),
            }),
            trait_name: Some("MissingTrait".to_string()),
            trait_id: Some(missing_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("create".to_string(), method)]),
        })
        .unwrap();

    let expr = lowerer.with_test_body_context(|lowerer| {
        lowerer.lower_identifier_path(&identifier_path(&["Widget", "create"]))
    });

    assert_eq!(expr.ty, Type::Error);
    assert!(!matches!(
        expr.kind,
        HirExprKind::ResolvedVar(HirVarRef {
            target: HirVarTarget::Function(id),
            ..
        }) if id == method_id
    ));
    assert!(lowerer
        .diagnostics
        .errors()
        .iter()
        .any(|error| { error.message.contains("unknown trait") && error.span().is_some() }));
}

#[test]
fn module_local_enum_alias_shadows_root_enum_for_qualified_variant() {
    let mut lowerer = Lowerer::new_for_test();
    let root_id = def_id(42);
    let module_id = def_id(43);
    lowerer
        .items
        .insert_enumeration(single_unit_variant_enum(root_id, "Thing", "Variant"));
    lowerer.items.insert_enumeration(single_unit_variant_enum(
        module_id,
        "demo::helper::Thing",
        "Variant",
    ));
    lowerer
        .resolver
        .item_paths
        .insert("Thing".to_string(), root_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_id, "Thing".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "Thing".to_string(),
        "demo::helper::Thing".to_string(),
        module_id,
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["Thing", "Variant"]));

    match expr.kind {
        HirExprKind::EnumVariant(enum_name, variant_name, _, Some(location)) => {
            assert_eq!(enum_name, "demo::helper::Thing");
            assert_eq!(variant_name, "Variant");
            assert_eq!(location.owner, module_id);
        }
        other => panic!("expected module alias enum variant, got {other:?}"),
    }
}

#[test]
fn module_local_struct_alias_shadows_root_struct_for_static_method() {
    let mut lowerer = Lowerer::new_for_test();
    let root_struct_id = def_id(44);
    let module_struct_id = def_id(45);
    let root_method_id = def_id(46);
    let module_method_id = def_id(47);
    lowerer
        .items
        .insert_structure(empty_struct(root_struct_id, "Thing"));
    lowerer
        .items
        .insert_structure(empty_struct(module_struct_id, "demo::helper::Thing"));
    register_static_impl(
        &mut lowerer,
        def_id(146),
        root_struct_id,
        "Thing",
        "make",
        test_function(root_method_id, "make"),
    );
    register_static_impl(
        &mut lowerer,
        def_id(147),
        module_struct_id,
        "demo::helper::Thing",
        "make",
        test_function(module_method_id, "make"),
    );
    lowerer
        .resolver
        .item_paths
        .insert("Thing".to_string(), root_struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_struct_id, "Thing".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "Thing".to_string(),
        "demo::helper::Thing".to_string(),
        module_struct_id,
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["Thing", "make"]));

    let (reference, target) = static_method_value_parts(&expr);
    assert_eq!(reference.name, "demo::helper::Thing::make");
    assert_eq!(reference.target, HirVarTarget::Function(module_method_id));
    assert_eq!(target.method.impl_id(), Some(def_id(147)));
    assert_eq!(target.method.method_id(), Some(module_method_id));
}

#[test]
fn import_alias_struct_resolves_static_impl_method_by_canonical_owner() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(48);
    let method_id = def_id(49);
    lowerer
        .items
        .insert_structure(empty_struct(struct_id, "std::string::String"));
    lowerer.resolver.insert_import_alias_with_name(
        "String".to_string(),
        "std::string::String".to_string(),
        struct_id,
    );
    lowerer
        .items
        .insert_impl(HirImpl {
            id: def_id(50),
            owner: HirImplOwner::Named("std::string::String".to_string()),
            type_name: "String".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("from".to_string(), test_function(method_id, "from"))]),
        })
        .unwrap();
    let expr = lowerer.lower_identifier_path(&identifier_path(&["String", "from"]));

    let (reference, target) = static_method_value_parts(&expr);
    assert_eq!(reference.name, "std::string::String::from");
    assert_eq!(reference.target, HirVarTarget::Function(method_id));
    assert_eq!(target.method.impl_id(), Some(def_id(50)));
    assert_eq!(target.method.method_id(), Some(method_id));
}

#[test]
fn import_alias_struct_without_canonical_name_does_not_resolve_static_impl_method_by_id() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(51);
    lowerer
        .items
        .insert_structure(empty_struct(struct_id, "String"));
    lowerer
        .resolver
        .import_aliases
        .insert("String".to_string(), struct_id);
    let expr = lowerer.lower_identifier_path(&identifier_path(&["String", "from"]));

    match expr.kind {
        HirExprKind::Var(name) => assert_eq!(name, "String::from"),
        other => panic!("expected unresolved static method path, got {other:?}"),
    }
}

#[test]
fn import_alias_struct_uses_canonical_static_impl_owner() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(53);
    let canonical_method_id = def_id(55);
    lowerer
        .items
        .insert_structure(empty_struct(struct_id, "String"));
    lowerer.resolver.insert_import_alias_with_name(
        "String".to_string(),
        "std::string::String".to_string(),
        struct_id,
    );
    register_static_impl(
        &mut lowerer,
        def_id(155),
        struct_id,
        "std::string::String",
        "from",
        test_function(canonical_method_id, "from"),
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["String", "from"]));

    let (reference, target) = static_method_value_parts(&expr);
    assert_eq!(reference.name, "std::string::String::from");
    assert_eq!(
        reference.target,
        HirVarTarget::Function(canonical_method_id)
    );
    assert_eq!(target.method.impl_id(), Some(def_id(155)));
    assert_eq!(target.method.method_id(), Some(canonical_method_id));
}

#[test]
fn lower_import_alias_function_reference_records_function_id() {
    let mut lowerer = Lowerer::new_for_test();
    let answer_id = def_id(31);
    lowerer
        .items
        .insert_function(test_function(answer_id, "answer"));
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );
    lowerer.resolver.insert_import_alias_with_name(
        "answer".to_string(),
        "pkg::answer".to_string(),
        answer_id,
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "pkg::answer");
            assert_eq!(reference.target, HirVarTarget::Function(answer_id));
        }
        other => panic!("expected resolved function alias, got {other:?}"),
    }
}

#[test]
fn module_local_alias_resolves_through_resolver_without_lowerer_string_map() {
    let mut lowerer = Lowerer::new_for_test();
    let function_id = def_id(32);
    lowerer
        .items
        .insert_function(test_function(function_id, "answer"));
    lowerer.resolver.insert_module_alias_with_name(
        "answer".to_string(),
        "demo::math::answer".to_string(),
        function_id,
    );
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::ResolvedVar(HirVarRef {
            name,
            target: HirVarTarget::Function(id),
        }) => {
            assert_eq!(name, "demo::math::answer");
            assert_eq!(id, function_id);
        }
        other => panic!("expected resolver-backed module alias, got {other:?}"),
    }
}

#[test]
fn lower_identifier_path_requires_resolver_id_for_top_level_function() {
    let mut lowerer = Lowerer::new_for_test();
    let function_id = def_id(38);
    lowerer
        .items
        .insert_function(test_function(function_id, "answer"));

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::Var(name) => assert_eq!(name, "answer"),
        other => panic!("expected unresolved var without resolver id, got {other:?}"),
    }
}

#[test]
fn module_local_alias_shadows_root_item_with_same_short_name() {
    let mut lowerer = Lowerer::new_for_test();
    let root_id = def_id(36);
    let module_id = def_id(37);
    lowerer
        .items
        .insert_function(test_function(root_id, "answer"));
    lowerer
        .items
        .insert_function(test_function(module_id, "answer"));
    lowerer
        .resolver
        .item_paths
        .insert("answer".to_string(), root_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_id, "answer".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "answer".to_string(),
        "demo::helper::answer".to_string(),
        module_id,
    );
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::ResolvedVar(HirVarRef {
            name,
            target: HirVarTarget::Function(id),
        }) => {
            assert_eq!(name, "demo::helper::answer");
            assert_eq!(id, module_id);
        }
        other => panic!("expected module alias to shadow root item, got {other:?}"),
    }
}

#[test]
fn lower_nested_scope_module_local_alias_function_reference_records_function_id() {
    let mut lowerer = Lowerer::new_for_test();
    let answer_id = def_id(34);
    lowerer
        .items
        .insert_function(test_function(answer_id, "answer"));
    lowerer.scope.push();
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );
    lowerer.resolver.insert_module_alias_with_name(
        "answer".to_string(),
        "pkg::answer".to_string(),
        answer_id,
    );
    lowerer.scope.push();

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "pkg::answer");
            assert_eq!(reference.target, HirVarTarget::Function(answer_id));
        }
        other => panic!("expected resolved nested module-local function alias, got {other:?}"),
    }
}

#[test]
fn lower_local_shadowing_module_local_alias_stays_var() {
    let mut lowerer = Lowerer::new_for_test();
    let answer_id = def_id(35);
    lowerer
        .items
        .insert_function(test_function(answer_id, "answer"));
    lowerer.scope.push();
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );
    lowerer.resolver.insert_module_alias_with_name(
        "answer".to_string(),
        "pkg::answer".to_string(),
        answer_id,
    );
    lowerer.scope.push();
    lowerer.scope.define("answer".to_string(), Type::I64, false);

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::Var(name) => assert_eq!(name, "answer"),
        other => panic!("expected local shadow to stay a var, got {other:?}"),
    }
}

#[test]
fn lower_import_alias_extern_reference_records_extern_id() {
    let mut lowerer = Lowerer::new_for_test();
    let extern_id = def_id(33);
    lowerer.items.insert_extern(HirExtern {
        id: extern_id,
        name: "pkg::puts".to_string(),
        params: vec![Type::I32],
        ret: Type::I32,
        variadic: false,
        is_unsafe: false,
    });
    lowerer.scope.define_alias(
        "puts".to_string(),
        Type::function(vec![Type::I32], Type::I32),
        false,
    );
    lowerer.resolver.insert_import_alias_with_name(
        "puts".to_string(),
        "pkg::puts".to_string(),
        extern_id,
    );

    let expr = lowerer.lower_identifier_path(&identifier_path(&["puts"]));

    match expr.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "pkg::puts");
            assert_eq!(reference.target, HirVarTarget::Extern(extern_id));
        }
        other => panic!("expected resolved extern alias, got {other:?}"),
    }
}

#[test]
fn test_lower_lambda_carries_capture_metadata() {
    let mut lowerer = Lowerer::new_for_test();
    let lambda = LambdaDecl {
        parameters: vec![ident_pattern("x")],
        body: Block {
            statements: vec![Statement::Expression(var_expr("base"))],
        },
        arrow_kind: LambdaArrowKind::Normal,
        span: crate::lexer::Span::test(),
    };

    let hir = lowerer.with_test_body_context(|lowerer| {
        let base_id = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("base".to_string(), Type::I64, false, base_id);
        lowerer.lower_lambda(&lambda)
    });
    let Type::Function {
        callable_kind,
        captures: type_captures,
        ..
    } = &hir.ty
    else {
        panic!("expected lambda function type");
    };
    assert_eq!(*callable_kind, CallableKind::Fn);
    assert_eq!(type_captures[0].kind, CaptureKind::SharedBorrow);
    assert_eq!(type_captures[0].ty, Type::I64);
    match hir.kind {
        HirExprKind::Lambda { captures, .. } => {
            assert_eq!(captures.len(), 1);
            assert_eq!(captures[0].name, "base");
        }
        _ => panic!("expected lambda"),
    }
}

#[test]
fn lower_lambda_reference_pattern_wraps_parameter_type() {
    let mut lowerer = Lowerer::new_for_test();
    let lambda = LambdaDecl {
        parameters: vec![Pattern {
            binding: None,
            kind: PatternKind::Reference {
                pattern: Box::new(ident_pattern("value")),
                mutable: false,
            },
        }],
        body: Block {
            statements: vec![Statement::Expression(var_expr("value"))],
        },
        arrow_kind: LambdaArrowKind::Normal,
        span: crate::lexer::Span::test(),
    };

    let hir = lowerer.with_test_body_context(|lowerer| lowerer.lower_lambda(&lambda));
    let HirExprKind::Lambda { params, .. } = &hir.kind else {
        panic!("expected lambda");
    };
    let Type::Function {
        params: function_params,
        ..
    } = &hir.ty
    else {
        panic!("expected function type");
    };

    assert_eq!(params[0].ty, function_params[0]);
    assert!(matches!(
        &params[0].ty,
        Type::Reference {
            mutable: false,
            inner
        } if matches!(inner.as_ref(), Type::TypeVar(_))
    ));
}

#[test]
fn outer_lambda_does_not_capture_local_used_by_nested_lambda() {
    let mut lowerer = Lowerer::new_for_test();
    let captures = lowerer.with_test_body_context(|lowerer| {
        let next_id = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("next".to_string(), Type::I64, false, next_id);
        let nested_body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "next".to_string(),
                    target: HirVarTarget::Local(next_id),
                }),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            })],
            ty: Type::I64,
        };
        let body = HirBlock {
            stmts: vec![
                HirStmt::Let {
                    name: "next".to_string(),
                    local_id: next_id,
                    ty: Type::I64,
                    value: HirExpr {
                        kind: HirExprKind::IntLiteral(1),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    },
                    mutable: false,
                },
                HirStmt::Expr(HirExpr {
                    kind: HirExprKind::Lambda {
                        params: Vec::new(),
                        body: nested_body,
                        captures: Vec::new(),
                    },
                    ty: Type::function(Vec::new(), Type::I64),
                    span: crate::lexer::Span::test(),
                }),
            ],
            ty: Type::function(Vec::new(), Type::I64),
        };

        lowerer.collect_lambda_captures(&body, &[])
    });

    assert!(captures.is_empty());
}

#[test]
fn lower_lambda_captures_resolved_local_reference() {
    let mut lowerer = Lowerer::new_for_test();
    let lambda = LambdaDecl {
        parameters: vec![ident_pattern("x")],
        body: Block {
            statements: vec![Statement::Expression(var_expr("base"))],
        },
        arrow_kind: LambdaArrowKind::Normal,
        span: crate::lexer::Span::test(),
    };

    let hir = lowerer.with_test_body_context(|lowerer| {
        let base_id = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("base".to_string(), Type::I64, false, base_id);
        lowerer.lower_lambda(&lambda)
    });
    match hir.kind {
        HirExprKind::Lambda { captures, .. } => {
            assert_eq!(captures.len(), 1);
            assert_eq!(captures[0].name, "base");
        }
        _ => panic!("expected lambda"),
    }
}

#[test]
fn lambda_capture_records_captured_local_id() {
    let mut lowerer = Lowerer::new_for_test();
    let lambda = LambdaDecl {
        parameters: vec![ident_pattern("x")],
        body: Block {
            statements: vec![Statement::Expression(var_expr("base"))],
        },
        arrow_kind: LambdaArrowKind::Normal,
        span: crate::lexer::Span::test(),
    };

    let (captured_id, hir) = lowerer.with_test_body_context(|lowerer| {
        let captured_id = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("base".to_string(), Type::I64, false, captured_id);
        (captured_id, lowerer.lower_lambda(&lambda))
    });
    match hir.kind {
        HirExprKind::Lambda { captures, .. } => {
            assert_eq!(captures.len(), 1);
            assert_eq!(captures[0].name, "base");
            assert_eq!(captures[0].local_id, captured_id);
        }
        other => panic!("expected lambda, got {other:?}"),
    }
}

#[test]
fn lambda_captures_root_scope_lexical_function_value() {
    let mut lowerer = Lowerer::new_for_test();
    let lambda = LambdaDecl {
        parameters: vec![],
        body: Block {
            statements: vec![Statement::Expression(var_expr("callback"))],
        },
        arrow_kind: LambdaArrowKind::Normal,
        span: crate::lexer::Span::test(),
    };
    let callback_id = HirLocalId(91);
    lowerer.scope.define_local(
        "callback".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
        callback_id,
    );

    let hir = lowerer.lower_lambda(&lambda);

    let HirExprKind::Lambda { captures, .. } = hir.kind else {
        panic!("expected lambda");
    };
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].name, "callback");
    assert_eq!(captures[0].local_id, callback_id);
}

#[test]
fn lambda_does_not_capture_explicit_top_level_function() {
    let mut lowerer = Lowerer::new_for_test();
    let lambda = LambdaDecl {
        parameters: vec![],
        body: Block {
            statements: vec![Statement::Expression(var_expr("callback"))],
        },
        arrow_kind: LambdaArrowKind::Normal,
        span: crate::lexer::Span::test(),
    };
    lowerer.scope.define_top_level(
        "callback".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );

    let hir = lowerer.lower_lambda(&lambda);

    let HirExprKind::Lambda { captures, .. } = hir.kind else {
        panic!("expected lambda");
    };
    assert!(captures.is_empty());
}

#[test]
fn lowers_known_field_access_with_resolved_field_location() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_structure(point_struct());
    register_item_path(&mut lowerer, "Point", def_id(10));

    let base = HirExpr {
        kind: HirExprKind::Var("point".to_string()),
        ty: Type::Struct {
            id: def_id(10),
            args: Vec::new(),
        },
        span: crate::lexer::Span::test(),
    };

    let field = lowerer.apply_secondary(
        base,
        &ast::SecondaryExpr::Dot(ast::IdentOrNumber::Ident(ident("y"))),
        crate::lower::expression::ExprUse::Value,
        false,
    );

    match field.kind {
        HirExprKind::FieldAccess(_, name, Some(location)) => {
            assert_eq!(name, "y");
            assert_eq!(location.owner, def_id(10));
            assert_eq!(location.field_id, FieldId(1));
            assert_eq!(location.name, "y");
        }
        other => panic!("expected resolved field access, got {other:?}"),
    }
}

#[test]
fn lowers_known_struct_literal_fields_with_resolved_field_locations() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_structure(point_struct());
    register_item_path(&mut lowerer, "Point", def_id(10));

    let literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![IdentOrType::Ident(ident("Point"))],
        },
        fields: HashMap::from([(ident("x"), int_expr("1")), (ident("y"), int_expr("2"))]),
    });

    match literal.kind {
        HirExprKind::StructLiteral(_, _, fields) => {
            let y = fields.iter().find(|field| field.name == "y").unwrap();
            let location = y.field.as_ref().expect("known field should be resolved");
            assert_eq!(location.owner, def_id(10));
            assert_eq!(location.field_id, FieldId(1));
            assert_eq!(location.name, "y");
        }
        other => panic!("expected struct literal, got {other:?}"),
    }
}

#[test]
fn struct_literal_resolved_fields_all_have_locations() {
    let point_id = def_id(10);
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_structure(point_struct());
    register_item_path(&mut lowerer, "Point", point_id);

    let literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![IdentOrType::Ident(ident("Point"))],
        },
        fields: HashMap::from([(ident("x"), int_expr("1")), (ident("y"), int_expr("2"))]),
    });

    match literal.kind {
        HirExprKind::StructLiteral(struct_name, Some(id), fields) => {
            assert_eq!(struct_name, "Point");
            assert_eq!(id, point_id);
            let x = fields.iter().find(|field| field.name == "x").unwrap();
            let location = x
                .field
                .as_ref()
                .expect("resolved field should have location");
            assert_eq!(location.owner, point_id);
            assert_eq!(location.field_id, FieldId(0));
            assert_eq!(location.name, "x");
            assert!(
                fields.iter().all(|field| field.field.is_some()),
                "all resolved fields should have locations"
            );
        }
        other => panic!("expected resolved Point struct literal, got {other:?}"),
    }
}

#[test]
fn module_local_struct_alias_shadows_root_struct_for_instance_construction() {
    let mut lowerer = Lowerer::new_for_test();
    let root_id = def_id(48);
    let module_id = def_id(49);
    lowerer
        .items
        .insert_structure(empty_struct(root_id, "Thing"));
    lowerer
        .items
        .insert_structure(empty_struct(module_id, "demo::helper::Thing"));
    lowerer
        .resolver
        .item_paths
        .insert("Thing".to_string(), root_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_id, "Thing".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "Thing".to_string(),
        "demo::helper::Thing".to_string(),
        module_id,
    );

    let literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![IdentOrType::Ident(ident("Thing"))],
        },
        fields: HashMap::new(),
    });

    match literal.kind {
        HirExprKind::StructLiteral(name, Some(id), _) => {
            assert_eq!(name, "demo::helper::Thing");
            assert_eq!(id, module_id);
        }
        other => panic!("expected module alias struct literal, got {other:?}"),
    }
}

#[test]
fn struct_literal_private_field_check_matches_current_impl_by_struct_id() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(56);
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "String"));
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "std::string_type::String"));
    lowerer
        .resolver
        .item_paths
        .insert("String".to_string(), struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "std::string_type::String".to_string());
    lowerer.current_struct_impl = Some("String".to_string());

    let _literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![IdentOrType::Ident(ident("String"))],
        },
        fields: HashMap::from([(ident("ptr"), int_expr("1"))]),
    });

    assert!(
        lowerer
            .errors()
            .iter()
            .all(|error| !error.message.contains("private fields")),
        "same-struct construction should not report private fields: {:?}",
        lowerer.errors()
    );
}

#[test]
fn struct_literal_private_field_check_matches_current_impl_by_pre_resolved_id() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(58);
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "std::string_type::String"));
    lowerer
        .resolver
        .item_paths
        .insert("String".to_string(), struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "std::string_type::String".to_string());
    lowerer.current_struct_impl = Some("std::string_type::String".to_string());

    let _literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![IdentOrType::Ident(ident("String"))],
        },
        fields: HashMap::from([(ident("ptr"), int_expr("1"))]),
    });

    assert!(
        lowerer
            .errors()
            .iter()
            .all(|error| !error.message.contains("private fields")),
        "same-struct construction should not report private fields: {:?}",
        lowerer.errors()
    );
}

#[test]
fn struct_literal_private_field_check_requires_resolved_current_impl_id() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(57);
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "String"));
    lowerer
        .items
        .insert_structure(private_field_struct(struct_id, "std::string_type::String"));
    register_item_path(&mut lowerer, "std::string_type::String", struct_id);
    lowerer.current_struct_impl = Some("String".to_string());

    let _literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![
                IdentOrType::Ident(ident("std")),
                IdentOrType::Ident(ident("string_type")),
                IdentOrType::Ident(ident("String")),
            ],
        },
        fields: HashMap::from([(ident("ptr"), int_expr("1"))]),
    });

    assert!(
        lowerer
            .errors()
            .iter()
            .any(|error| error.message.contains("private fields")),
        "unresolved current impl name should not grant private construction: {:?}",
        lowerer.errors()
    );
}

#[test]
fn lowers_known_enum_variant_constructor_with_resolved_variant_location() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_enumeration(option_enum());
    register_item_path(&mut lowerer, "Maybe", def_id(20));

    let variant = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![
                IdentOrType::Ident(ident("Maybe")),
                IdentOrType::Ident(ident("Some")),
            ],
        },
        fields: HashMap::from([(ident("value"), int_expr("1"))]),
    });

    match variant.kind {
        HirExprKind::EnumVariant(enum_name, variant_name, _, Some(location)) => {
            assert_eq!(enum_name, "Maybe");
            assert_eq!(variant_name, "Some");
            assert_eq!(location.owner, def_id(20));
            assert_eq!(location.variant_id, VariantId(0));
            assert_eq!(location.name, "Some");
        }
        other => panic!("expected resolved enum variant, got {other:?}"),
    }
}

#[test]
fn lowers_named_enum_variant_fields_in_variant_definition_order() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_enumeration(record_enum());
    register_item_path(&mut lowerer, "Record", def_id(21));

    let variant = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![
                IdentOrType::Ident(ident("Record")),
                IdentOrType::Ident(ident("Pair")),
            ],
        },
        fields: HashMap::from([
            (ident("fourth"), int_expr("4")),
            (ident("third"), int_expr("3")),
            (ident("second"), int_expr("2")),
            (ident("first"), int_expr("1")),
        ]),
    });

    match variant.kind {
        HirExprKind::EnumVariant(_, _, args, Some(location)) => {
            assert_eq!(location.owner, def_id(21));
            assert_eq!(location.variant_id, VariantId(0));
            assert!(matches!(args[0].kind, HirExprKind::IntLiteral(1)));
            assert!(matches!(args[1].kind, HirExprKind::IntLiteral(2)));
            assert!(matches!(args[2].kind, HirExprKind::IntLiteral(3)));
            assert!(matches!(args[3].kind, HirExprKind::IntLiteral(4)));
        }
        other => panic!("expected resolved enum variant, got {other:?}"),
    }
}

#[test]
fn leaves_unknown_struct_literal_fields_unresolved() {
    let mut lowerer = Lowerer::new_for_test();

    let literal = lowerer.lower_instance(&ast::Instance {
        name: ast::TypePath {
            path: vec![IdentOrType::Ident(ident("Unknown"))],
        },
        fields: HashMap::from([(ident("value"), int_expr("1"))]),
    });

    match literal.kind {
        HirExprKind::StructLiteral(_, _, fields) => {
            assert_eq!(fields[0].name, "value");
            assert!(fields[0].field.is_none());
        }
        other => panic!("expected fallback struct literal, got {other:?}"),
    }
}
