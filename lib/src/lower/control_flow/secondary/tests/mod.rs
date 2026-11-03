use super::Lowerer;
use crate::ast::{
    Expression, Ident, IdentOrNumber, IdentOrType, IdentifierPath, Literal, LiteralKind, Operand,
    PrimaryExpr, SecondaryExpr, UnaryExpr,
};
use crate::hir::{
    HirBlock, HirCallTarget, HirExpr, HirExprKind, HirExtern, HirField, HirFunction,
    HirFunctionSig, HirImpl, HirImplOwner, HirParam, HirStruct, HirTrait,
};
use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
use crate::language_items::{IndexLanguageItems, TryLanguageItems};
use crate::lexer::Span;
use crate::lower::expression::ExprUse;
use crate::types::{GenericParamDecl, GenericParamId, TraitBound, Type};
use std::collections::HashMap;

mod calls;
mod type_vars;

fn test_function(id: DefId, name: &str, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: ret_type.clone(),
        body: HirBlock {
            stmts: Vec::new(),
            ty: ret_type,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn receiver_expr(ty: Type) -> HirExpr {
    HirExpr {
        kind: HirExprKind::Var("receiver".to_string()),
        ty,
        span: Span::test(),
    }
}
