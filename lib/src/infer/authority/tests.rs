use super::*;
use crate::collect::resolver::ResolverTables;
use crate::hir::HirLanguageItems;
use crate::ids::{CrateId, IdGen, LocalDefId};

fn function(id: DefId, var: crate::ids::TypeVarId) -> HirFunction {
    HirFunction {
        id,
        name: format!("f{}", id.local.0),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![crate::hir::HirParam {
            name: "value".to_string(),
            local_id: HirLocalId(0),
            ty: Type::TypeVar(var),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::TypeVar(var),
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::TypeVar(var),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn hir(functions: HashMap<DefId, HirFunction>) -> PartialHir {
    PartialHir {
        functions,
        structs: HashMap::new(),
        enums: HashMap::new(),
        traits: HashMap::new(),
        impls: HashMap::new(),
        externs: HashMap::new(),
        type_aliases: HashMap::new(),
        engine: crate::infer::InferenceEngine::new(),
        function_type_vars: HashMap::new(),
        import_aliases: HashMap::new(),
        loaded_module_paths: Vec::new(),
        constraint_store: ConstraintStore::new(),
        resolver: ResolverTables::default(),
        current_def_ids: std::collections::BTreeSet::new(),
        root_crate_id: CrateId(0),
        local_def_ids: IdGen::new(),
        language_items: HirLanguageItems::default(),
        imported_effective_trait_methods: HashMap::new(),
        inference_sccs: HashMap::new(),
        inference_scc_order: Vec::new(),
        source_map: Default::default(),
    }
}

#[test]
fn per_call_relation_preserves_repeated_source_variables() {
    let mut engine = crate::infer::InferenceEngine::new();
    let source = engine.fresh_type_var();
    let Type::TypeVar(source_id) = source else {
        unreachable!();
    };
    let callback_instance = engine.fresh_type_var();
    let mut bindings = HashMap::new();

    propagate_instance_relation(
        &mut engine,
        &Type::TypeVar(source_id),
        &Type::I64,
        &mut bindings,
        &HashSet::new(),
    );
    propagate_instance_relation(
        &mut engine,
        &Type::Reference {
            mutable: true,
            inner: Box::new(Type::TypeVar(source_id)),
        },
        &Type::Reference {
            mutable: true,
            inner: Box::new(callback_instance.clone()),
        },
        &mut bindings,
        &HashSet::new(),
    );

    assert_eq!(engine.resolve(&callback_instance), Type::I64);
    assert_eq!(
        engine.resolve(&Type::TypeVar(source_id)),
        Type::TypeVar(source_id)
    );
}

#[test]
fn constrained_relation_propagates_instance_authority_to_source() {
    let mut engine = crate::infer::InferenceEngine::new();
    let source = engine.fresh_type_var();
    let Type::TypeVar(source_id) = source else {
        unreachable!();
    };

    propagate_instance_relation(
        &mut engine,
        &Type::TypeVar(source_id),
        &Type::I64,
        &mut HashMap::new(),
        &HashSet::from([source_id]),
    );

    assert_eq!(engine.resolve(&Type::TypeVar(source_id)), Type::I64);
}

#[test]
fn authority_obligations_have_stable_owner_order_and_context() {
    let first = DefId::new(CrateId(0), LocalDefId(1));
    let second = DefId::new(CrateId(0), LocalDefId(2));
    let hir = hir(HashMap::from([
        (second, function(second, crate::ids::TypeVarId(1))),
        (first, function(first, crate::ids::TypeVarId(0))),
    ]));
    let owners = HashSet::from([ConstraintOwner::Body(second), ConstraintOwner::Body(first)]);

    let obligations = authority_obligations(&hir, &owners);

    assert_eq!(obligations.len(), 2);
    assert_eq!(obligations[0].id.raw(), 0);
    assert_eq!(obligations[0].owner, ConstraintOwner::Body(first));
    assert_eq!(obligations[0].kind, AuthorityObligationKind::Propagation);
    assert!(obligations[0].context.contains("call and type propagation"));
    assert_eq!(obligations[1].owner, ConstraintOwner::Body(second));
}

#[test]
fn authority_obligation_wakes_only_for_dependent_representative() {
    let id = DefId::new(CrateId(0), LocalDefId(1));
    let hir = hir(HashMap::from([(
        id,
        function(id, crate::ids::TypeVarId(0)),
    )]));
    let owners = HashSet::from([ConstraintOwner::Body(id)]);
    let obligations = authority_obligations(&hir, &owners);

    assert!(obligations[0].depends_on_any(&HashSet::from([crate::ids::TypeVarId(0)])));
    assert!(!obligations[0].depends_on_any(&HashSet::from([crate::ids::TypeVarId(1)])));
}

#[test]
fn authority_source_site_identity_targets_only_the_selected_expression() {
    let function_id = DefId::new(CrateId(0), LocalDefId(1));
    let struct_id = DefId::new(CrateId(0), LocalDefId(2));
    let receiver = || HirExpr {
        kind: HirExprKind::Var("record".to_string()),
        ty: Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
        span: crate::lexer::Span::test(),
    };
    let field = |name: &str, start| HirExpr {
        kind: HirExprKind::FieldAccess(Box::new(receiver()), name.to_string(), None),
        ty: Type::I64,
        span: crate::lexer::Span {
            file_path: "test.rk".into(),
            start,
            end: start + name.len(),
        },
    };
    let mut caller = function(function_id, crate::ids::TypeVarId(0));
    caller.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::Var("consume".to_string()),
                    ty: Type::Error,
                    span: crate::lexer::Span::test(),
                }),
                vec![field("first", 10), field("second", 20)],
                None,
            ),
            ty: Type::Unit,
            span: crate::lexer::Span::test(),
        })],
        ty: Type::Unit,
    };
    let mut hir = hir(HashMap::from([(function_id, caller)]));
    hir.structs.insert(
        struct_id,
        crate::hir::HirStruct {
            id: struct_id,
            name: "Record".to_string(),
            generic_params: Vec::new(),
            fields: vec![
                crate::hir::HirField {
                    id: crate::ids::FieldId(0),
                    name: "first".to_string(),
                    ty: Type::I64,
                    public: true,
                },
                crate::hir::HirField {
                    id: crate::ids::FieldId(1),
                    name: "second".to_string(),
                    ty: Type::I64,
                    public: true,
                },
            ],
        },
    );
    let owners = HashSet::from([ConstraintOwner::Body(function_id)]);
    let mut obligations = authority_obligations(&hir, &owners);
    let second = obligations
        .iter_mut()
        .find(|obligation| obligation.context == "field access 'second'")
        .expect("second field obligation");

    run_authority_obligation(&mut hir, second, false, false).unwrap();

    let HirStmt::Expr(HirExpr {
        kind: HirExprKind::Call(_, args, _),
        ..
    }) = &hir.functions[&function_id].body.stmts[0]
    else {
        panic!("expected call expression");
    };
    let locations = args
        .iter()
        .map(|arg| match &arg.kind {
            HirExprKind::FieldAccess(_, _, location) => location.as_ref(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(locations[0].is_none());
    assert_eq!(
        locations[1].map(|location| location.field_id),
        Some(crate::ids::FieldId(1))
    );
}
