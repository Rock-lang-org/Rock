use std::collections::HashMap;

use crate::{
    ast::{AssociatedTypeDef, ParseType},
    lexer::TokenType,
    parser::{engine::*, parse_type, parse_type_inner, FunctionDecl, FunctionSig, Impl},
};

use super::{
    empty_lines, function_decl, function_sig, indent, parse_constructor_param,
    parse_parenthesized_constructor_param, parse_plain_generic_param, parse_where_clause,
};

enum FnDeclOrSig {
    AssociatedType(AssociatedTypeDef),
    Decl(FunctionDecl),
    Sig(FunctionSig),
}

fn associated_type_def(stream: Input) -> IResult<AssociatedTypeDef> {
    (
        TokenType::Keyword("type".to_string()),
        parse_parenthesized_constructor_param
            .or(parse_constructor_param)
            .or(parse_plain_generic_param),
        TokenType::Equal,
        parse_type,
        TokenType::Eol,
    )
        .map(|(_, param, _, ty, _)| AssociatedTypeDef {
            name: param.name,
            kind: param.kind,
            ty,
        })
        .process(stream)
}

pub fn r#impl(stream: Input) -> IResult<Impl> {
    let start = stream;
    let (rest, implementation) = (
        TokenType::Keyword("impl".to_string()),
        parse_type_inner.map(ParseType::Type).or(parse_type),
        preceded(TokenType::Keyword("for".to_string()), parse_type).opt(),
        // Parse optional where clauses: `where T: Show, U: Display`
        preceded(
            TokenType::Keyword("where".to_string()),
            separated1(parse_where_clause, TokenType::Coma),
        )
        .opt(),
        TokenType::Eol.followed_by(empty_lines),
        indented(many(
            preceded(
                indent,
                associated_type_def
                    .map(FnDeclOrSig::AssociatedType)
                    .or(function_decl
                        .map(FnDeclOrSig::Decl)
                        .or(function_sig.map(FnDeclOrSig::Sig))),
            )
            .followed_by(empty_lines),
        )),
    )
        .map(|(_, name, for_, where_clauses, _, items)| {
            let mut associated_types = Vec::new();
            let mut methods = HashMap::new();
            let mut signatures = HashMap::new();

            for item in items {
                match item {
                    FnDeclOrSig::AssociatedType(assoc) => associated_types.push(assoc),
                    FnDeclOrSig::Decl(decl) => {
                        methods.insert(decl.name.clone(), decl);
                    }
                    FnDeclOrSig::Sig(sig) => {
                        signatures.insert(sig.name.clone(), sig);
                    }
                }
            }

            Impl {
                name,
                for_,
                associated_types,
                methods,
                signatures,
                where_clauses: where_clauses.unwrap_or_default(),
            }
        })
        .process(stream)?;
    if implementation.for_.is_some() && !matches!(implementation.name, ParseType::Type(_)) {
        return Err(ParseError::UnexpectedToken("trait name before 'for'".to_string(), start.seek()?));
    }
    Ok((rest, implementation))
}
