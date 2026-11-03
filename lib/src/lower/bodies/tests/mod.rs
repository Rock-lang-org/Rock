use super::*;
use crate::ast::{
    Block, Expression, FunctionDecl, Ident, IdentOrNumber, IdentOrType, IdentifierPath,
    LambdaArrowKind, LambdaDecl, Literal, LiteralKind, Operand, ParseTypeInner, PrimaryExpr,
    SecondaryExpr, SelfReceiverMode, Statement, TypePath, UnaryExpr,
};
use crate::hir::{
    HirBlock, HirEnum, HirExprKind, HirField, HirFunction, HirImpl, HirImplOwner,
    HirImplReceiverPattern, HirParam, HirStruct,
};
use crate::ids::{FieldId, LocalDefId, VariantId};
use crate::lexer::Span;
use std::collections::HashMap;

mod generics;
mod impl_bodies;

fn def_id(local: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(local))
}

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: Span::test(),
    }
}

fn empty_function_decl(name: &str) -> FunctionDecl {
    FunctionDecl {
        name: ident(name),
        lambda: LambdaDecl {
            parameters: Vec::new(),
            body: Block {
                statements: Vec::new(),
            },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        },
        self_receiver: None,
        is_unsafe: false,
        exported: false,
    }
}
