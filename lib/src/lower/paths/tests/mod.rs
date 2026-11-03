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
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

mod constructor_selection;
mod lambdas;
mod references;
mod static_bounds;
mod static_owners;
mod struct_literals;

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

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
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

fn empty_struct(id: DefId, name: &str) -> HirStruct {
    HirStruct {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    }
}
