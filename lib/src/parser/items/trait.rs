use std::collections::HashMap;

use crate::{
    ast::{LanguageItemMarker, LanguageItemMemberKind, LanguageItemMemberMarker},
    language_items::LanguageItemRole,
    lexer::TokenType,
    parser::{engine::*, AssociatedTypeDecl, FunctionDecl, FunctionSig, TraitDecl},
};

use super::{
    empty_lines, empty_lines_permissive, function_decl, function_sig, indent, language_item_marker,
    parse_constructor_param, parse_parenthesized_constructor_param, parse_plain_generic_param,
    parse_type_name, parse_where_clause,
};

enum FnDeclOrSig {
    AssociatedType(AssociatedTypeDecl),
    Decl(FunctionDecl),
    Sig(FunctionSig),
}

impl FnDeclOrSig {
    fn kind_and_name(&self) -> (LanguageItemMemberKind, String) {
        match self {
            Self::AssociatedType(decl) => (
                LanguageItemMemberKind::AssociatedType,
                decl.name.name.clone(),
            ),
            Self::Decl(decl) => (LanguageItemMemberKind::Method, decl.name.name.clone()),
            Self::Sig(sig) => (LanguageItemMemberKind::Method, sig.name.name.clone()),
        }
    }
}

fn associated_type_decl(stream: Input) -> IResult<AssociatedTypeDecl> {
    (
        TokenType::Keyword("type".to_string()),
        parse_parenthesized_constructor_param
            .or(parse_constructor_param)
            .or(parse_plain_generic_param),
        TokenType::Eol,
    )
        .map(|(_, param, _)| AssociatedTypeDecl {
            name: param.name,
            kind: param.kind,
        })
        .process(stream)
}

fn trait_member(stream: Input) -> IResult<(FnDeclOrSig, Option<LanguageItemMarker>)> {
    let (stream, _) = indent(stream)?;
    let (stream, marker) = if matches!(stream.seek()?.token_type, TokenType::Ident(ref name) if name == "lang")
    {
        let (stream, marker) = language_item_marker(stream)?;
        (stream, Some(marker))
    } else {
        (stream, None)
    };
    if let Some(marker) = &marker {
        if marker.role == LanguageItemRole::IndexMut {
            return Err(ParseError::HardError(
                "language item role 'index_mut' may only mark a trait root".to_string(),
                marker.span.clone(),
            ));
        }
    }
    let stream = if marker.is_some() {
        let (stream, _) = empty_lines_permissive(stream)?;
        let (stream, _) = indent(stream).map_err(|_| {
            ParseError::HardError(
                "language item marker must be followed by a member at the same indentation"
                    .to_string(),
                marker.as_ref().unwrap().span.clone(),
            )
        })?;
        if matches!(stream.seek()?.token_type, TokenType::Ident(ref name) if name == "lang") {
            return Err(ParseError::HardError(
                "language item marker must apply to exactly one member".to_string(),
                stream.seek()?.span,
            ));
        }
        stream
    } else {
        stream
    };

    let mut parser = associated_type_decl
        .map(FnDeclOrSig::AssociatedType)
        .or(function_decl
            .map(FnDeclOrSig::Decl)
            .or(function_sig.map(FnDeclOrSig::Sig)));
    let (stream, item) = match parser.process(stream) {
        Ok(result) => result,
        Err(_) if marker.is_some() => {
            return Err(ParseError::HardError(
                "language item marker must be followed by a trait member".to_string(),
                marker
                    .as_ref()
                    .expect("guarded language item marker")
                    .span
                    .clone(),
            ));
        }
        Err(error) => return Err(error),
    };

    Ok((stream, (item, marker)))
}

pub fn r#trait(stream: Input) -> IResult<TraitDecl> {
    let (stream, (_, name, generic_params, for_, where_clauses, _, items)) = (
        TokenType::Keyword("trait".to_string()),
        parse_type_name,
        trait_generic_params,
        preceded(
            TokenType::Keyword("for".to_string()),
            parse_parenthesized_constructor_param.or(parse_constructor_param),
        )
        .opt(),
        preceded(
            TokenType::Keyword("where".to_string()),
            separated1(parse_where_clause, TokenType::Coma),
        )
        .opt(),
        TokenType::Eol.followed_by(empty_lines),
        indented(many(trait_member.followed_by(empty_lines_permissive))),
    )
        .process(stream)?;

    let mut associated_types = Vec::new();
    let mut methods = HashMap::new();
    let mut signatures = HashMap::new();
    let mut language_items = crate::ast::LanguageItemAnnotations::default();

    for (item, marker) in items {
        if let Some(marker) = marker {
            let (kind, member_name) = item.kind_and_name();
            language_items.members.push(LanguageItemMemberMarker {
                marker,
                kind,
                member_name,
            });
        }
        match item {
            FnDeclOrSig::AssociatedType(decl) => associated_types.push(decl),
            FnDeclOrSig::Decl(decl) => {
                methods.insert(decl.name.clone(), decl);
            }
            FnDeclOrSig::Sig(sig) => {
                signatures.insert(sig.name.clone(), sig);
            }
        }
    }

    Ok((
        stream,
        TraitDecl {
            name,
            generic_params,
            for_,
            where_clauses: where_clauses.unwrap_or_default(),
            associated_types,
            methods,
            signatures,
            exported: false,
            language_items,
        },
    ))
}

fn trait_generic_params(mut stream: Input) -> IResult<Vec<crate::ast::GenericParamDecl>> {
    let mut params = Vec::new();
    while !matches!(
        stream.seek()?.token_type,
        TokenType::Keyword(ref keyword) if keyword == "for" || keyword == "where"
    ) && !matches!(stream.seek()?.token_type, TokenType::Eol)
    {
        let parser = if matches!(stream.seek()?.token_type, TokenType::OpenParen) {
            parse_parenthesized_constructor_param
        } else if matches!(
            stream.tokens.get(1).map(|token| &token.token_type),
            Some(TokenType::Underscore)
        ) {
            parse_constructor_param
        } else {
            parse_plain_generic_param
        };
        let (next_stream, param) = parser(stream)?;
        params.push(param);
        stream = next_stream;
        if matches!(stream.seek()?.token_type, TokenType::Coma) {
            let (next_stream, _) = TokenType::Coma.process(stream)?;
            stream = next_stream;
        }
    }
    Ok((stream, params))
}
