use std::collections::{HashMap, HashSet};

use super::*;

use crate::ast::{
    self, AssociatedTypeDecl, AssociatedTypeDef, EnumDecl, EnumVariant, FunctionDecl, FunctionSig,
    Ident, Impl, LambdaArrowKind, LambdaDecl, NamedFieldsOrTypesList, ParseType, ParseTypeInner,
    Pattern, PatternKind, SelfReceiverMode, StructDecl, StructDeclField, TraitDecl, WhereClause,
};
use crate::collect::context::CollectContext;
use crate::hir::{HirFunctionSig, HirVariantFields};
use crate::ids::{CrateId, LocalDefId};
use crate::language_items::SizedLanguageItems;
use crate::lexer::Span;
use crate::types::{AssociatedTypeKey, GenericParamId, Type};

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: Span::test(),
    }
}

fn type_inner(name: &str) -> ParseTypeInner {
    ParseTypeInner {
        name: name.to_string(),
        generics: vec![],
        span: Span::test(),
    }
}

fn named_type(name: &str) -> ParseType {
    ParseType::Type(type_inner(name))
}

fn generic_type(name: &str) -> ParseType {
    ParseType::Type(type_inner(name))
}

fn generic_param(name: &str) -> ast::GenericParamDecl {
    ast::GenericParamDecl {
        default: None,
        name: ident(name),
        kind: None,
        span: Span::test(),
    }
}

fn def_id(local: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(local))
}

fn impl_method_ids(methods: &[(&str, DefId)]) -> HashMap<String, DefId> {
    methods
        .iter()
        .map(|(name, id)| ((*name).to_string(), *id))
        .collect()
}

fn ident_pattern(name: &str) -> Pattern {
    Pattern {
        binding: None,
        kind: PatternKind::Ident(ast::IdentPattern {
            name: ident(name),
            mut_: false,
        }),
    }
}

fn function_decl(
    name: &str,
    params: &[&str],
    self_receiver: Option<SelfReceiverMode>,
    arrow_kind: LambdaArrowKind,
) -> FunctionDecl {
    FunctionDecl {
        name: ident(name),
        lambda: LambdaDecl {
            parameters: params.iter().map(|name| ident_pattern(name)).collect(),
            body: ast::Block { statements: vec![] },
            arrow_kind,
            span: crate::lexer::Span::test(),
        },
        self_receiver,
        is_unsafe: false,
        exported: false,
    }
}

fn function_sig(
    name: &str,
    sig: ParseType,
    self_receiver: Option<SelfReceiverMode>,
    where_clauses: Vec<WhereClause>,
) -> FunctionSig {
    FunctionSig {
        name: ident(name),
        sig,
        where_clauses,
        self_receiver,
        is_unsafe: false,
        exported: false,
    }
}

mod functions;
mod impls;
mod nominals;
mod traits;
