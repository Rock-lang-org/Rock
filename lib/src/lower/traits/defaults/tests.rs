use std::collections::HashMap;

use super::*;

use crate::ast::{
    Block, Expression, FunctionDecl, FunctionSig, Ident, IdentOrNumber, IdentOrType,
    IdentifierPath, LambdaArrowKind, LambdaDecl, Literal, LiteralKind, Module, ModuleDecl, Operand,
    ParseType, ParseTypeInner, PrimaryExpr, SecondaryExpr, SelfReceiverMode, Statement, TopLevel,
    TraitDecl, UnaryExpr,
};
use crate::collect::item_index::{index_root_module_items, IndexingIds};
use crate::hir::{HirBlock, HirExprKind, HirFunction, HirFunctionSig, HirParam, HirTrait};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::lexer::Span;
use crate::types::{
    CallableKind, CaptureKind, FunctionCapture, FunctionSafety, GenericParamDecl, GenericParamId,
    Type,
};

fn def_id(local: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(local))
}

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: Span::test(),
    }
}

fn self_method_value_expr(method: &str) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: crate::ast::Operand::Ident(IdentifierPath {
            path: vec![IdentOrType::Ident(ident("self"))],
        }),
        secondaries: Some(vec![SecondaryExpr::Dot(IdentOrNumber::Ident(ident(
            method,
        )))]),
        type_annotation: None,
    }))
}

fn named_type(name: &str) -> ParseType {
    ParseType::Type(ParseTypeInner {
        name: name.to_string(),
        generics: Vec::new(),
        span: Span::test(),
    })
}

fn literal_default_decl(name: &str, kind: LiteralKind) -> FunctionDecl {
    FunctionDecl {
        name: ident(name),
        lambda: LambdaDecl {
            parameters: Vec::new(),
            body: Block {
                statements: vec![Statement::Expression(Expression::UnaryExpr(
                    UnaryExpr::PrimaryExpr(PrimaryExpr {
                        operand: Operand::Literal(Literal {
                            kind,
                            span: Span::test(),
                        }),
                        secondaries: None,
                        type_annotation: None,
                    }),
                ))],
            },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        },
        self_receiver: None,
        is_unsafe: false,
        exported: false,
    }
}

fn trait_decl(name: &str, method: FunctionDecl) -> TraitDecl {
    TraitDecl {
        where_clauses: Vec::new(),
        name: ParseTypeInner {
            name: name.to_string(),
            generics: Vec::new(),
            span: Span::test(),
        },
        generic_params: Vec::new(),
        for_: None,
        associated_types: Vec::new(),
        methods: HashMap::from([(ident(&method.name.name), method)]),
        signatures: HashMap::new(),
        exported: false,
        language_items: Default::default(),
    }
}

fn trait_method(id: DefId, name: &str, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: true,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn trait_with_method(id: DefId, name: &str, method: HirFunction) -> HirTrait {
    HirTrait {
        target: None,
        predicates: Vec::new(),
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([(method.name.clone(), method)]),
        signatures: HashMap::new(),
    }
}

#[test]
fn trait_default_signature_stub_uses_canonical_params_without_duplicate_self() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(70);
    let signature_id = def_id(71);
    let default_id = def_id(72);
    let self_param = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let self_ty = Type::Generic(self_param);
    let method_value_ty = Type::function_with_metadata(
        vec![Type::I64],
        Type::Bool,
        FunctionSafety::Safe,
        CallableKind::FnMut,
        vec![FunctionCapture::new(
            CaptureKind::MutableBorrow,
            self_ty.clone(),
        )],
    );
    lowerer
        .resolver
        .item_paths
        .insert("Reader".to_string(), trait_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "Reader".to_string());
    let default_method = HirFunction {
        id: default_id,
        name: "default".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: self_ty.clone(),
            mutable: true,
            is_ref: false,
        }],
        ret_type: method_value_ty.clone(),
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(crate::types::ReceiverMode::Mut),
        is_unsafe: false,
    };
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Reader".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("default".to_string(), default_method)]),
        signatures: HashMap::from([(
            "set".to_string(),
            HirFunctionSig {
                id: signature_id,
                name: "set".to_string(),
                generic_params: vec![GenericParamDecl::type_param(self_param, "Self")],
                params: vec![self_ty.clone(), Type::I64],
                ret: Type::Bool,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(crate::types::ReceiverMode::Mut),
                is_unsafe: false,
            },
        )]),
    });
    let trait_decl = TraitDecl {
        where_clauses: Vec::new(),
        name: ParseTypeInner {
            name: "Reader".to_string(),
            generics: Vec::new(),
            span: Span::test(),
        },
        generic_params: Vec::new(),
        for_: None,
        associated_types: Vec::new(),
        methods: HashMap::from([(
            ident("default"),
            FunctionDecl {
                name: ident("default"),
                lambda: LambdaDecl {
                    parameters: Vec::new(),
                    body: Block {
                        statements: vec![Statement::Expression(self_method_value_expr("set"))],
                    },
                    arrow_kind: LambdaArrowKind::Normal,
                    span: crate::lexer::Span::test(),
                },
                self_receiver: Some(SelfReceiverMode::Mut),
                is_unsafe: false,
                exported: false,
            },
        )]),
        signatures: HashMap::from([(
            ident("set"),
            FunctionSig {
                name: ident("set"),
                sig: named_type("Bool"),
                where_clauses: Vec::new(),
                self_receiver: Some(SelfReceiverMode::Mut),
                is_unsafe: false,
                exported: false,
            },
        )]),
        exported: false,
        language_items: Default::default(),
    };

    lowerer.lower_trait_bodies(&trait_decl, trait_id);

    let lowered = &lowerer.items.trait_def(trait_id).unwrap().methods["default"];
    let Some(crate::hir::HirStmt::Expr(expr)) = lowered.body.stmts.first() else {
        panic!("expected lowered method value expression");
    };
    assert_eq!(expr.ty, method_value_ty);
    match &expr.kind {
        HirExprKind::Lambda { params, .. } => {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].ty, Type::I64);
        }
        other => panic!("expected method value lambda, got {other:?}"),
    }
}

#[test]
fn trait_default_bodies_report_missing_canonical_trait_name() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(73);
    let trait_decl = trait_decl(
        "Hidden",
        literal_default_decl("value", LiteralKind::Number(1)),
    );
    lowerer.items.insert_trait_def(trait_with_method(
        trait_id,
        "Hidden",
        trait_method(def_id(74), "value", Type::I64),
    ));

    lowerer.lower_trait_bodies(&trait_decl, trait_id);

    let error = lowerer
        .errors()
        .iter()
        .find(|error| {
            error
                .message
                .contains("missing canonical trait name while lowering default bodies")
        })
        .expect("missing canonical trait metadata should be diagnosed");
    assert_eq!(error.span(), None);
    assert_eq!(
        lowerer.items.trait_def(trait_id).unwrap().methods["value"]
            .body
            .ty,
        Type::Unit
    );
}

#[test]
fn trait_default_bodies_restore_caller_trait_context() {
    let mut lowerer = Lowerer::new_for_test();
    let outer_trait_id = def_id(75);
    let trait_id = def_id(76);
    let trait_decl = trait_decl(
        "Inner",
        literal_default_decl("value", LiteralKind::Number(1)),
    );
    lowerer.current_trait = Some("Outer".to_string());
    lowerer.current_trait_id = Some(outer_trait_id);
    lowerer.current_trait_generics = vec!["T".to_string()];
    lowerer.generic_context = Some(GenericLoweringContext::new(
        outer_trait_id,
        vec!["T".to_string()],
    ));
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "Inner".to_string());
    lowerer.items.insert_trait_def(trait_with_method(
        trait_id,
        "Inner",
        trait_method(def_id(77), "value", Type::I64),
    ));

    lowerer.lower_trait_bodies(&trait_decl, trait_id);

    assert_eq!(lowerer.current_trait.as_deref(), Some("Outer"));
    assert_eq!(lowerer.current_trait_id, Some(outer_trait_id));
    assert_eq!(lowerer.current_trait_generics, vec!["T".to_string()]);
    assert_eq!(lowerer.current_generic_owner(), Some(outer_trait_id));
    assert_eq!(lowerer.current_generic_params(), ["T".to_string()]);
}

#[test]
fn trait_default_body_diagnostics_follow_method_def_id_order() {
    for _ in 0..16 {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(78);
        lowerer
            .resolver
            .item_names_by_id
            .insert(trait_id, "Ordered".to_string());
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Ordered".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([
                (
                    "zeta".to_string(),
                    trait_method(def_id(79), "zeta", Type::Bool),
                ),
                (
                    "beta".to_string(),
                    trait_method(def_id(80), "beta", Type::Bool),
                ),
                (
                    "alpha".to_string(),
                    trait_method(def_id(81), "alpha", Type::Bool),
                ),
            ]),
            signatures: HashMap::new(),
        });
        let trait_decl = TraitDecl {
            where_clauses: Vec::new(),
            name: ParseTypeInner {
                name: "Ordered".to_string(),
                generics: Vec::new(),
                span: Span::test(),
            },
            generic_params: Vec::new(),
            for_: None,
            associated_types: Vec::new(),
            methods: HashMap::from([
                (
                    ident("alpha"),
                    literal_default_decl("alpha", LiteralKind::Number(1)),
                ),
                (
                    ident("beta"),
                    literal_default_decl("beta", LiteralKind::Number(1)),
                ),
                (
                    ident("zeta"),
                    literal_default_decl("zeta", LiteralKind::Number(1)),
                ),
            ]),
            signatures: HashMap::new(),
            exported: false,
            language_items: Default::default(),
        };

        lowerer.lower_trait_bodies(&trait_decl, trait_id);

        let method_names: Vec<_> = lowerer
            .errors()
            .iter()
            .filter_map(|error| {
                error
                    .message
                    .strip_prefix("In trait default method 'Ordered.")
                    .and_then(|message| message.split_once("': return type mismatch"))
                    .map(|(method_name, _)| method_name)
            })
            .collect();
        assert_eq!(method_names, ["zeta", "beta", "alpha"]);
    }
}

#[test]
fn trait_default_bodies_use_indexed_inline_trait_owner_ids() {
    let module = Module {
        name: None,
        top_levels: vec![
            TopLevel::Module(ModuleDecl(Module {
                name: Some(ident("left")),
                top_levels: vec![TopLevel::TraitDecl(trait_decl(
                    "Shared",
                    literal_default_decl("value", LiteralKind::Number(1)),
                ))],
                is_inline: true,
                filepath: None,
            })),
            TopLevel::Module(ModuleDecl(Module {
                name: Some(ident("right")),
                top_levels: vec![TopLevel::TraitDecl(trait_decl(
                    "Shared",
                    literal_default_decl("value", LiteralKind::Bool(true)),
                ))],
                is_inline: true,
                filepath: None,
            })),
        ],
        is_inline: true,
        filepath: None,
    };
    let mut indexing_ids = IndexingIds::new_root();
    let root_module_id = indexing_ids.root_module_id();
    let mut lowerer = Lowerer::new_for_test();
    lowerer.item_index = index_root_module_items(&mut indexing_ids, &module);
    let left_module_id = lowerer
        .item_index
        .child_module_id(root_module_id, "left")
        .unwrap();
    let right_module_id = lowerer
        .item_index
        .child_module_id(root_module_id, "right")
        .unwrap();
    let left_trait_id = lowerer
        .item_index
        .item_at_source(left_module_id, 0)
        .unwrap()
        .def_id;
    let right_trait_id = lowerer
        .item_index
        .item_at_source(right_module_id, 0)
        .unwrap()
        .def_id;
    lowerer.items.insert_trait_def(trait_with_method(
        left_trait_id,
        "left::Shared",
        trait_method(def_id(80), "value", Type::I64),
    ));
    lowerer.items.insert_trait_def(trait_with_method(
        right_trait_id,
        "right::Shared",
        trait_method(def_id(81), "value", Type::Bool),
    ));
    // The old name lookup selects this trait for both inline declarations.
    lowerer
        .resolver
        .item_paths
        .insert("Shared".to_string(), left_trait_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(left_trait_id, "left::Shared".to_string());
    lowerer
        .resolver
        .item_names_by_id
        .insert(right_trait_id, "right::Shared".to_string());

    lowerer.lower_trait_default_bodies(&module, root_module_id);

    assert_eq!(
        lowerer.items.trait_def(left_trait_id).unwrap().methods["value"]
            .body
            .ty,
        Type::I64
    );
    assert_eq!(
        lowerer.items.trait_def(right_trait_id).unwrap().methods["value"]
            .body
            .ty,
        Type::Bool
    );
}

#[test]
fn trait_default_bodies_report_missing_indexed_trait_owner() {
    let module = Module {
        name: None,
        top_levels: vec![TopLevel::TraitDecl(trait_decl(
            "Missing",
            literal_default_decl("value", LiteralKind::Number(1)),
        ))],
        is_inline: true,
        filepath: None,
    };
    let mut indexing_ids = IndexingIds::new_root();
    let root_module_id = indexing_ids.root_module_id();
    let mut lowerer = Lowerer::new_for_test();
    lowerer.item_index = index_root_module_items(&mut indexing_ids, &module);

    lowerer.lower_trait_default_bodies(&module, root_module_id);

    assert!(lowerer.errors().iter().any(|error| error
        .message
        .contains("missing indexed trait declaration while lowering default bodies")));
}

#[test]
fn trait_default_bodies_report_mismatched_indexed_owner_and_missing_member() {
    let trait_module = Module {
        name: None,
        top_levels: vec![TopLevel::TraitDecl(trait_decl(
            "Missing",
            literal_default_decl("value", LiteralKind::Number(1)),
        ))],
        is_inline: true,
        filepath: None,
    };
    let indexed_module = Module {
        name: None,
        top_levels: vec![TopLevel::FunctionDecl(literal_default_decl(
            "not_a_trait",
            LiteralKind::Number(1),
        ))],
        is_inline: true,
        filepath: None,
    };
    let mut indexing_ids = IndexingIds::new_root();
    let root_module_id = indexing_ids.root_module_id();
    let mut lowerer = Lowerer::new_for_test();
    lowerer.item_index = index_root_module_items(&mut indexing_ids, &indexed_module);

    lowerer.lower_trait_default_bodies(&trait_module, root_module_id);

    assert!(lowerer.errors().iter().any(|error| error
        .message
        .contains("indexed declaration kind does not match trait syntax")));

    let mut indexing_ids = IndexingIds::new_root();
    let root_module_id = indexing_ids.root_module_id();
    lowerer.item_index = index_root_module_items(&mut indexing_ids, &trait_module);
    let trait_id = lowerer
        .item_index
        .item_at_source(root_module_id, 0)
        .unwrap()
        .def_id;
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Missing".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    });
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "Missing".to_string());

    lowerer.lower_trait_default_bodies(&trait_module, root_module_id);

    assert!(lowerer.errors().iter().any(|error| error
        .message
        .contains("missing indexed trait method 'value' while lowering default body")));
}
