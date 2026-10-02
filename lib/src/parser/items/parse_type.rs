use crate::{
    ast::{GenericParamDecl, ParseType, ParseTypeInner, TypeApplication, TypeHole, TypeLambda},
    lexer::{Span, TokenType},
    parser::engine::*,
};

use super::{get_span, mut_prefix, primitives};

pub fn parse_type(stream: Input) -> IResult<ParseType> {
    let (stream, mut ty) = if matches!(peek_token(&stream), Some(TokenType::TypeLambda)) {
        parse_type_lambda(stream)?
    } else {
        parse_type_application(stream)?
    };

    if matches!(peek_token(&stream), Some(TokenType::Arrow)) {
        let (stream, _) = TokenType::Arrow.process(stream)?;
        let rhs_is_grouped_return = if matches!(peek_token(&stream), Some(TokenType::OpenParen)) {
            parse_parenthesized_type(stream)
                .map(|(after_group, _)| !matches!(peek_token(&after_group), Some(TokenType::Arrow)))
                .unwrap_or(false)
        } else {
            false
        };
        let (stream, rhs) = parse_type_after_arrow(stream)?;
        let mut types = vec![ty];
        match rhs {
            rhs if rhs_is_grouped_return => types.push(rhs),
            ParseType::Function(mut rhs) => types.append(&mut rhs),
            rhs => types.push(rhs),
        }
        ty = ParseType::Function(types);
        Ok((stream, ty))
    } else {
        Ok((stream, ty))
    }
}

fn parse_type_after_arrow(stream: Input) -> IResult<ParseType> {
    let token = stream.seek()?;
    if matches!(
        &token.token_type,
        TokenType::Eof | TokenType::Eol | TokenType::CloseParen | TokenType::Coma
    ) {
        return Err(ParseError::UnexpectedToken(
            "type expression after '->'".to_string(),
            token,
        ));
    }
    parse_type(stream)
}

fn parse_type_application(stream: Input) -> IResult<ParseType> {
    let (mut stream, constructor) = parse_type_atom(stream)?;
    let mut args = Vec::new();
    let constructor_span = type_span(&constructor);
    let mut end_span = constructor_span.clone();

    loop {
        if !starts_type_atom(&stream) {
            break;
        }

        let (next_stream, arg) = parse_type_atom(stream)?;
        end_span = type_span(&arg);
        args.push(arg);
        stream = next_stream;

        // Commas separate sibling arguments only after the constructor has
        // received an argument. This leaves `(A, B)` available to tuple parsing.
        if matches!(peek_token(&stream), Some(TokenType::Coma)) {
            let (after_comma, _) = TokenType::Coma.process(stream)?;
            if starts_type_atom(&after_comma) && !starts_where_subject(&after_comma) {
                stream = after_comma;
            } else {
                break;
            }
        }
    }

    let ty = if args.is_empty() {
        constructor
    } else {
        ParseType::Application(TypeApplication {
            constructor: Box::new(constructor),
            args,
            span: join_spans(&constructor_span, &end_span),
        })
    };
    if matches!(peek_token(&stream), Some(TokenType::OpenBrace)) {
        parse_object_qualifiers(stream, ty)
    } else {
        Ok((stream, ty))
    }
}

fn skip_object_layout(mut stream: Input) -> IResult<()> {
    while matches!(
        peek_token(&stream),
        Some(TokenType::Eol | TokenType::Indent(_))
    ) {
        stream = stream.consume()?.0;
    }
    Ok((stream, ()))
}

fn parse_object_qualifiers(stream: Input, base: ParseType) -> IResult<ParseType> {
    use crate::ast::{ObjectQualifier, ObjectTypeSyntax};
    let (mut stream, _) = TokenType::OpenBrace.process(stream)?;
    let mut qualifiers = Vec::new();
    stream = skip_object_layout(stream)?.0;
    if matches!(peek_token(&stream), Some(TokenType::CloseBrace)) {
        return Err(ParseError::UnexpectedToken(
            "object qualifier".to_string(),
            stream.seek()?,
        ));
    }
    loop {
        stream = skip_object_layout(stream)?.0;
        if matches!(peek_token(&stream), Some(TokenType::CloseBrace)) {
            break;
        }
        let (after_head, head) = parse_type_atom(stream)?;
        stream = after_head;
        let (owner, member) = if matches!(peek_token(&stream), Some(TokenType::DoubleColon)) {
            stream = TokenType::DoubleColon.process(stream)?.0;
            let (next, member) = type_token_with_span(stream)?;
            stream = next;
            (Some(Box::new(head.clone())), Some(member))
        } else {
            match &head {
                ParseType::Type(inner) if inner.generics.is_empty() => (
                    None,
                    Some(crate::ast::Ident {
                        name: inner.name.clone(),
                        span: inner.span.clone(),
                    }),
                ),
                ParseType::Associated { base, member } => (
                    Some(Box::new(ParseType::Type(base.clone()))),
                    Some(member.clone()),
                ),
                _ => (None, None),
            }
        };
        if matches!(peek_token(&stream), Some(TokenType::Equal)) {
            let Some(member) = member else {
                return Err(ParseError::UnexpectedToken(
                    "associated member name".to_string(),
                    stream.seek()?,
                ));
            };
            stream = TokenType::Equal.process(stream)?.0;
            let (next, ty) = parse_type_atom(stream)?;
            stream = next;
            qualifiers.push(ObjectQualifier::Binding { owner, member, ty });
        } else {
            if owner.is_some() {
                return Err(ParseError::UnexpectedToken(
                    "'=' after associated member".to_string(),
                    stream.seek()?,
                ));
            }
            qualifiers.push(ObjectQualifier::Trait(head));
        }
        stream = skip_object_layout(stream)?.0;
        if matches!(peek_token(&stream), Some(TokenType::CloseBrace)) {
            break;
        }
        stream = TokenType::Coma.process(stream)?.0;
    }
    let (stream, close) = stream.consume()?;
    Ok((
        stream,
        ParseType::Object(ObjectTypeSyntax {
            span: join_spans(&base.span(), &close.span),
            base: Box::new(base),
            qualifiers,
        }),
    ))
}

fn parse_type_atom(stream: Input) -> IResult<ParseType> {
    let token = stream.seek()?;
    match &token.token_type {
        TokenType::Type(_) => parse_named_type(stream),
        TokenType::Underscore => {
            let (stream, token) = stream.consume()?;
            Ok((stream, ParseType::Hole(TypeHole { span: token.span })))
        }
        TokenType::TypeLambda => parse_type_lambda(stream),
        TokenType::OpenParen => parse_parenthesized_type(stream),
        TokenType::OpenBracket => parse_array_type(stream),
        TokenType::Ampersand => parse_reference_type(stream),
        TokenType::StuckOperator(op) if op == "*" => parse_pointer_type(stream),
        _ => Err(ParseError::UnexpectedToken("type".to_string(), token)),
    }
}

fn parse_named_type(stream: Input) -> IResult<ParseType> {
    let (stream, token) = stream.consume()?;
    let TokenType::Type(name) = &token.token_type else {
        return Err(ParseError::UnexpectedToken("type".to_string(), token));
    };
    let inner = ParseTypeInner {
        name: name.clone(),
        generics: Vec::new(),
        span: token.span,
    };

    if matches!(peek_token(&stream), Some(TokenType::DoubleColon)) {
        let (stream, _) = TokenType::DoubleColon.process(stream)?;
        let (stream, member) = type_token_with_span(stream)?;
        return Ok((
            stream,
            ParseType::Associated {
                base: inner,
                member,
            },
        ));
    }

    Ok((stream, ParseType::Type(inner)))
}

pub fn parse_type_name(stream: Input) -> IResult<ParseTypeInner> {
    let (stream, name) = type_token_with_span(stream)?;
    Ok((
        stream,
        ParseTypeInner {
            name: name.name,
            generics: Vec::new(),
            span: name.span,
        },
    ))
}

pub fn parse_type_path_head(stream: Input) -> IResult<ParseType> {
    let (mut stream, name) = parse_type_name(stream)?;
    let constructor = ParseType::Type(name);
    let constructor_span = type_span(&constructor);
    let mut end_span = constructor_span.clone();
    let mut args = Vec::new();

    while matches!(peek_token(&stream), Some(TokenType::Type(_)))
        && !matches!(peek_nth(&stream, 1), Some(TokenType::DoubleColon))
    {
        let (next_stream, inner) = parse_type_name(stream)?;
        let arg = ParseType::Type(inner);
        end_span = type_span(&arg);
        args.push(arg);
        stream = next_stream;
        if matches!(peek_token(&stream), Some(TokenType::Coma)) {
            let (next_stream, _) = TokenType::Coma.process(stream)?;
            stream = next_stream;
        }
    }

    if args.is_empty() {
        Ok((stream, constructor))
    } else {
        Ok((
            stream,
            ParseType::Application(TypeApplication {
                constructor: Box::new(constructor),
                args,
                span: join_spans(&constructor_span, &end_span),
            }),
        ))
    }
}

fn type_token_with_span(stream: Input) -> IResult<crate::ast::Ident> {
    let (stream, token) = stream.consume()?;
    let span = token.span.clone();
    match &token.token_type {
        TokenType::Type(name) => Ok((
            stream,
            crate::ast::Ident {
                name: name.clone(),
                span,
            },
        )),
        _ => Err(ParseError::UnexpectedToken("type".to_string(), token)),
    }
}

fn parse_parenthesized_type(stream: Input) -> IResult<ParseType> {
    let (stream, (open_span, _)) = (get_span, TokenType::OpenParen).process(stream)?;
    if matches!(peek_token(&stream), Some(TokenType::CloseParen)) {
        let (stream, (close_span, _)) = (get_span, TokenType::CloseParen).process(stream)?;
        return Ok((
            stream,
            ParseType::Unit(Span::new(
                open_span.file_path,
                open_span.start,
                close_span.end,
            )),
        ));
    }

    let (mut stream, first) = parse_type(stream)?;
    if !matches!(peek_token(&stream), Some(TokenType::Coma)) {
        let (stream, _) = TokenType::CloseParen.process(stream)?;
        return Ok((stream, first));
    }

    let mut types = vec![first];
    while matches!(peek_token(&stream), Some(TokenType::Coma)) {
        let (next_stream, _) = TokenType::Coma.process(stream)?;
        stream = next_stream;
        if matches!(peek_token(&stream), Some(TokenType::CloseParen)) {
            break;
        }
        let (next_stream, ty) = parse_type(stream)?;
        types.push(ty);
        stream = next_stream;
    }
    let (stream, _) = TokenType::CloseParen.process(stream)?;
    Ok((stream, ParseType::Tuple(types)))
}

fn parse_reference_type(stream: Input) -> IResult<ParseType> {
    let (stream, _) = TokenType::Ampersand.process(stream)?;
    let (stream, is_mut) = mut_prefix.opt().process(stream)?;
    let (stream, pointee) = parse_type_application(stream)?;
    Ok((
        stream,
        ParseType::Reference {
            is_mut: is_mut.is_some(),
            pointee: Box::new(pointee),
        },
    ))
}

fn parse_pointer_type(stream: Input) -> IResult<ParseType> {
    let (stream, _) = TokenType::StuckOperator("*".to_string()).process(stream)?;
    let (stream, pointee) = parse_type_application(stream)?;
    Ok((stream, ParseType::Pointer(Box::new(pointee))))
}

pub fn parse_array_type(stream: Input) -> IResult<ParseType> {
    let (stream, _) = TokenType::OpenBracket.process(stream)?;
    let (stream, inner) = parse_type(stream)?;
    let (stream, result) = if is_semicolon(&stream) {
        let (stream, _) = stream.consume()?;
        let (stream, len) = primitives::int(stream)?;
        (
            stream,
            ParseType::Array {
                inner: Box::new(inner),
                len: len as usize,
            },
        )
    } else {
        (stream, ParseType::Slice(Box::new(inner)))
    };
    let (stream, _) = TokenType::CloseBracket.process(stream)?;
    Ok((stream, result))
}

fn is_semicolon(stream: &Input) -> bool {
    matches!(
        stream.seek().map(|token| token.token_type),
        Ok(TokenType::Operator(ref op)) if op == ";"
    ) || matches!(
        stream.seek().map(|token| token.token_type),
        Ok(TokenType::StuckOperator(ref op)) if op == ";"
    )
}

fn parse_type_lambda(stream: Input) -> IResult<ParseType> {
    let (mut stream, start) = stream.consume()?;
    let mut params = Vec::new();

    loop {
        let (next_stream, param) = parse_lambda_param(stream)?;
        params.push(param);
        stream = next_stream;
        if !matches!(peek_token(&stream), Some(TokenType::Coma)) {
            break;
        }
        let (next_stream, _) = TokenType::Coma.process(stream)?;
        stream = next_stream;
    }

    let (stream, _) = TokenType::Arrow.process(stream)?;
    let body_start = stream.seek()?;
    if matches!(
        body_start.token_type,
        TokenType::Eof | TokenType::Eol | TokenType::CloseParen | TokenType::Coma
    ) {
        return Err(ParseError::UnexpectedToken(
            "type lambda body".to_string(),
            body_start,
        ));
    }
    let (stream, body) = parse_type(stream)?;
    let span = join_spans(&start.span, &type_span(&body));
    Ok((
        stream,
        ParseType::Lambda(TypeLambda {
            params,
            body: Box::new(body),
            span,
        }),
    ))
}

fn parse_lambda_param(stream: Input) -> IResult<GenericParamDecl> {
    if matches!(stream.seek()?.token_type, TokenType::OpenParen) {
        parse_parenthesized_constructor_param(stream)
    } else {
        parse_plain_generic_param(stream)
    }
}

pub fn parse_generic_param_list(mut stream: Input) -> IResult<Vec<GenericParamDecl>> {
    let mut params = Vec::new();
    if !starts_generic_param(&stream) {
        return Ok((stream, params));
    }

    loop {
        let (next_stream, param) = if matches!(stream.seek()?.token_type, TokenType::OpenParen) {
            parse_parenthesized_constructor_param(stream)?
        } else {
            parse_plain_generic_param(stream)?
        };
        params.push(param);
        stream = next_stream;
        if !matches!(stream.seek()?.token_type, TokenType::Coma) {
            break;
        }
        let (next_stream, _) = TokenType::Coma.process(stream)?;
        stream = next_stream;
        if !starts_generic_param(&stream) {
            break;
        }
    }
    Ok((stream, params))
}

pub fn parse_constructor_param(stream: Input) -> IResult<GenericParamDecl> {
    parse_constructor_param_inner(stream, false)
}

pub(super) fn parse_parenthesized_constructor_param(stream: Input) -> IResult<GenericParamDecl> {
    let (stream, _) = TokenType::OpenParen.process(stream)?;
    let (stream, param) = parse_constructor_param_inner(stream, true)?;
    let (stream, _) = TokenType::CloseParen.process(stream)?;
    Ok((stream, param))
}

fn parse_constructor_param_inner(
    stream: Input,
    require_parenthesized: bool,
) -> IResult<GenericParamDecl> {
    let (mut stream, name_token) = stream.consume()?;
    let TokenType::Type(name) = &name_token.token_type else {
        return Err(ParseError::UnexpectedToken(
            "type binder".to_string(),
            name_token,
        ));
    };
    let name = crate::ast::Ident {
        name: name.clone(),
        span: name_token.span.clone(),
    };
    if matches!(stream.seek()?.token_type, TokenType::Colon) {
        if !require_parenthesized {
            return Err(ParseError::UnexpectedToken(
                "parenthesized explicit kind binder".to_string(),
                stream.seek()?,
            ));
        }
        let (next_stream, _) = TokenType::Colon.process(stream)?;
        if matches!(peek_token(&next_stream), Some(TokenType::Interogation)) {
            let (next_stream, _) = TokenType::Interogation.process(next_stream)?;
            let (stream, bound) = parse_type(next_stream)?;
            let span = join_spans(&name.span, &bound.span());
            return Ok((
                stream,
                GenericParamDecl {
                    name,
                    kind: None,
                    unsized_bound: Some(bound),
                    span,
                },
            ));
        }
        let kind_start = next_stream.seek()?;
        let (stream, kind) = parse_type(next_stream)?;
        if !is_explicit_kind_syntax(&kind) {
            return Err(ParseError::UnexpectedToken(
                "kind expression containing only 'Type' and '->'".to_string(),
                kind_start,
            ));
        }
        let span = join_spans(&name.span, &type_span(&kind));
        return Ok((
            stream,
            GenericParamDecl {
                name,
                unsized_bound: None,
                kind: Some(TypeApplication {
                    constructor: Box::new(kind),
                    args: Vec::new(),
                    span: span.clone(),
                }),
                span,
            },
        ));
    }
    let mut args = Vec::new();
    if !matches!(stream.seek()?.token_type, TokenType::Underscore) {
        return Err(ParseError::UnexpectedToken(
            "'_' after constructor binder".to_string(),
            stream.seek()?,
        ));
    }

    loop {
        let (next_stream, hole) = parse_type_atom(stream)?;
        if !matches!(&hole, ParseType::Hole(_)) {
            return Err(ParseError::UnexpectedToken(
                "type hole in constructor binder".to_string(),
                stream.seek()?,
            ));
        }
        args.push(hole);
        stream = next_stream;
        if !matches!(stream.seek()?.token_type, TokenType::Coma) {
            break;
        }
        let (next_stream, _) = TokenType::Coma.process(stream)?;
        stream = next_stream;
        if !matches!(stream.seek()?.token_type, TokenType::Underscore) {
            return Err(ParseError::UnexpectedToken(
                "type hole after constructor binder comma".to_string(),
                stream.seek()?,
            ));
        }
    }

    if require_parenthesized && args.is_empty() {
        return Err(ParseError::Fail);
    }
    let application = TypeApplication {
        constructor: Box::new(ParseType::Type(ParseTypeInner {
            name: name.name.clone(),
            generics: Vec::new(),
            span: name.span.clone(),
        })),
        span: join_spans(
            &name.span,
            &type_span(args.last().expect("constructor binder arg")),
        ),
        args,
    };
    let span = application.span.clone();
    Ok((
        stream,
        GenericParamDecl {
            name,
            kind: Some(application),
            unsized_bound: None,
            span,
        },
    ))
}

fn is_explicit_kind_syntax(kind: &ParseType) -> bool {
    match kind {
        ParseType::Type(inner) => inner.name == "Type" && inner.generics.is_empty(),
        ParseType::Function(types) => types.len() >= 2 && types.iter().all(is_explicit_kind_syntax),
        _ => false,
    }
}

pub fn parse_plain_generic_param(stream: Input) -> IResult<GenericParamDecl> {
    let (stream, token) = stream.consume()?;
    let span = token.span.clone();
    let TokenType::Type(name) = &token.token_type else {
        return Err(ParseError::UnexpectedToken(
            "type binder".to_string(),
            token,
        ));
    };
    let name = crate::ast::Ident {
        name: name.clone(),
        span: span.clone(),
    };
    Ok((
        stream,
        GenericParamDecl {
            name,
            kind: None,
            unsized_bound: None,
            span,
        },
    ))
}

fn starts_generic_param(stream: &Input) -> bool {
    matches!(
        stream.seek().map(|token| token.token_type),
        Ok(TokenType::Type(_)) | Ok(TokenType::OpenParen)
    )
}

fn starts_type_atom(stream: &Input) -> bool {
    match stream.seek().map(|token| token.token_type) {
        Ok(TokenType::Type(_))
        | Ok(TokenType::Underscore)
        | Ok(TokenType::TypeLambda)
        | Ok(TokenType::OpenParen)
        | Ok(TokenType::OpenBracket)
        | Ok(TokenType::Ampersand) => true,
        Ok(TokenType::StuckOperator(op)) => op == "*",
        _ => false,
    }
}

fn starts_where_subject(stream: &Input) -> bool {
    if !matches!(peek_token(stream), Some(TokenType::Type(_))) {
        return false;
    }
    if matches!(peek_nth(stream, 1), Some(TokenType::Colon)) {
        return true;
    }
    matches!(
        (peek_nth(stream, 1), peek_nth(stream, 2)),
        (Some(TokenType::Underscore), Some(TokenType::Colon))
    )
}

/// Compatibility parser for declaration names that still use `ParseTypeInner`.
/// New type expressions use `ParseType::Application` instead.
pub fn parse_type_inner(stream: Input) -> IResult<ParseTypeInner> {
    let (mut stream, name) = parse_type_name(stream)?;
    let mut generics = Vec::new();
    let mut end_span = name.span.clone();

    while starts_type_atom(&stream) {
        let (next_stream, generic) = if matches!(peek_token(&stream), Some(TokenType::OpenParen)) {
            parse_parenthesized_type(stream)?
        } else {
            parse_type(stream)?
        };
        end_span = type_span(&generic);
        generics.push(generic);
        stream = next_stream;
        if matches!(peek_token(&stream), Some(TokenType::Coma)) {
            let (next_stream, _) = TokenType::Coma.process(stream)?;
            stream = next_stream;
        }
    }

    Ok((
        stream,
        ParseTypeInner {
            name: name.name,
            generics,
            span: join_spans(&name.span, &end_span),
        },
    ))
}

fn type_span(ty: &ParseType) -> Span {
    ty.span()
}

fn join_spans(first: &Span, last: &Span) -> Span {
    Span {
        file_path: first.file_path.clone(),
        start: first.start,
        end: last.end,
    }
}

fn peek_token(stream: &Input) -> Option<TokenType> {
    stream.seek().ok().map(|token| token.token_type)
}

fn peek_nth(stream: &Input, index: usize) -> Option<TokenType> {
    stream
        .tokens
        .get(index)
        .map(|token| token.token_type.clone())
}
