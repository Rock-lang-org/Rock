use std::collections::HashMap;

use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::type_context::{Ty, TypeContext};
use crate::types::{AssociatedTypeKey, GenericParamId, Type};

fn def_id(local: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(local))
}

#[test]
fn type_id_substitution_rewrites_generic_projection_args() {
    let owner = def_id(90);
    let trait_id = def_id(91);
    let generic = GenericParamId { owner, index: 0 };
    let assoc = AssociatedTypeKey {
        owner: trait_id,
        assoc_type_id: AssocTypeId(0),
    };
    let mut context = TypeContext::new();
    let base = context.intern_ty(Ty::Generic(generic));
    let projection = context.intern_ty(Ty::Projection {
        ty: base,
        trait_id,
        assoc_type: assoc,
        trait_args: vec![base],
    });
    let replacement = context.intern_type(&Type::Struct {
        id: def_id(92),
        args: Vec::new(),
    });
    let mut subst = HashMap::new();
    subst.insert(generic, replacement);

    let rewritten = context.substitute_generics(projection, &subst);

    assert_eq!(
        context.type_for(rewritten),
        Type::Projection {
            ty: Box::new(Type::Struct {
                id: def_id(92),
                args: Vec::new(),
            }),
            trait_id,
            assoc_type: assoc,
            trait_args: vec![Type::Struct {
                id: def_id(92),
                args: Vec::new(),
            }],
        }
    );
}

fn const_param_expr(param: GenericParamId) -> crate::mono::hir_types::HirExpr {
    crate::mono::hir_types::HirExpr {
        kind: crate::mono::hir_types::HirExprKind::ConstParam(param),
        ty: Type::I64,
        span: crate::lexer::Span::new("/virtual/main.rk".into(), 0, 1),
    }
}

fn substitute_const_param(ty: Type) -> (crate::mono::hir_types::HirExprKind, super::Monomorphizer) {
    let param = GenericParamId {
        owner: def_id(1),
        index: 0,
    };
    let mut mono = super::Monomorphizer::new();
    let id = mono.intern_type(&ty);
    let substitution = HashMap::from([(param, id)]);
    let expr = mono.substitute_expr(&const_param_expr(param), &substitution);
    (expr.kind, mono)
}

#[test]
fn const_param_substitutes_to_integer_literal() {
    let (kind, mono) = substitute_const_param(Type::ConstValue(7));

    assert!(matches!(kind, crate::mono::hir_types::HirExprKind::IntLiteral(7)));
    assert!(mono.diagnostics.return_if_err().is_ok());
}

#[test]
fn const_param_value_above_i64_max_is_diagnosed_not_truncated() {
    let (kind, mono) = substitute_const_param(Type::ConstValue(i64::MAX as u64 + 1));

    assert!(mono
        .diagnostics
        .contains("does not fit in a signed 64-bit integer"));
    assert!(!matches!(
        kind,
        crate::mono::hir_types::HirExprKind::IntLiteral(i64::MIN)
    ));
}

#[test]
fn const_param_substituted_by_non_const_type_is_diagnosed() {
    let (kind, mono) = substitute_const_param(Type::I64);

    assert!(mono.diagnostics.contains("non-const type"));
    assert!(matches!(kind, crate::mono::hir_types::HirExprKind::ConstParam(_)));
}

#[test]
fn const_param_substituted_by_generic_stays_symbolic_without_diagnostic() {
    let other = GenericParamId {
        owner: def_id(2),
        index: 0,
    };
    let (kind, mono) = substitute_const_param(Type::Generic(other));

    assert!(matches!(kind, crate::mono::hir_types::HirExprKind::ConstParam(_)));
    assert!(mono.diagnostics.return_if_err().is_ok());
}
