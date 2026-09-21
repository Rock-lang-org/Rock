use crate::{
    ast::WhereClause,
    lexer::{Token, TokenType},
    parser::{
        engine::*, Block, FunctionDecl, FunctionSig, LambdaArrowKind, LambdaDecl, Pattern,
        SelfReceiverMode,
    },
};

use super::{
    block, consume_tokens_until, get_span, ident, operator_token, parenthesis, parse_type, pattern,
    reset_inside_argument_list, seek,
};

pub fn function_decl(stream: Input<'_>) -> IResult<'_, FunctionDecl> {
    (
        TokenType::Keyword("unsafe".to_string()).opt(),
        self_receiver_mode.opt(),
        ident,
        TokenType::Equal,
        lambda_decl,
        TokenType::Eol,
    )
        .map(
            |(is_unsafe, self_receiver, ident, _, lambda, _)| FunctionDecl {
                name: ident,
                lambda,
                self_receiver,
                is_unsafe: is_unsafe.is_some(),
                exported: false,
            },
        )
        .process(stream)
        .map_err(|e| e.with_context("function declaration"))
}

fn lambda_block(stream: Input) -> IResult<Block> {
    // Save the current indent level before parsing the lambda body
    let saved_indent = stream.indent_level;

    // Parse the block with reset argument list flags
    let result = reset_inside_argument_list(block).process(stream);

    // Restore the indent level after parsing
    match result {
        Ok((mut stream, block)) => {
            stream.indent_level = saved_indent;
            Ok((stream, block))
        }
        Err(e) => Err(e),
    }
}

pub fn lambda_decl(stream: Input) -> IResult<LambdaDecl> {
    function_shorthand
        .or((
            parameters,
            (get_span, TokenType::Arrow)
                .map(|(span, _)| (span, LambdaArrowKind::Normal))
                .or((get_span, TokenType::UnitArrow).map(|(span, _)| (span, LambdaArrowKind::Unit)))
                .or((get_span, TokenType::CurriedArrow)
                    .map(|(span, _)| (span, LambdaArrowKind::Curried))),
            lambda_block,
        )
            .map(|(parameters, (span, arrow_kind), body)| LambdaDecl {
                parameters,
                body,
                arrow_kind,
                span,
            }))
        .process(stream)
        .map_err(|e| e.with_context("lambda declaration"))
}

fn parameters(stream: Input) -> IResult<Vec<Pattern>> {
    if stream.inside_inline_argument_list {
        // At a call site, commas separate arguments rather than extending a
        // bare lambda's parameter list: `visit values, value !-> ...`.
        // Parenthesized lambdas reset this context and can have many parameters.
        return pattern
            .opt()
            .map(|parameter| parameter.into_iter().collect())
            .process(stream);
    }
    separated_trailing(pattern, TokenType::Coma).process(stream)
}

pub(super) fn parse_where_clause(stream: Input) -> IResult<WhereClause> {
    let (stream, subject) = parse_type(stream)?;
    if matches!(stream.seek()?.token_type, TokenType::Colon) {
        let (stream, _) = TokenType::Colon.process(stream)?;
        let (stream, trait_bound) = parse_type(stream)?;
        Ok((
            stream,
            WhereClause {
                subject,
                trait_bound: Some(trait_bound),
            },
        ))
    } else {
        Ok((
            stream,
            WhereClause {
                subject,
                trait_bound: None,
            },
        ))
    }
}

pub fn function_shorthand(stream: Input) -> IResult<LambdaDecl> {
    parenthesis(prefix_function_shorthand.or(suffix_function_shorthand)).process(stream)
}

pub fn prefix_function_shorthand(stream: Input) -> IResult<LambdaDecl> {
    let (stream, (span, _, inner_tokens)) = (
        get_span,
        seek(operator_token.map(|_| ()).or(TokenType::Dot.map(|_| ()))),
        consume_tokens_until(TokenType::CloseParen),
    )
        .process(stream)?;

    let new_inner_tokens = vec![
        Token {
            token_type: TokenType::Ident("x".to_string()),
            span: span.clone(),
        },
        Token {
            token_type: TokenType::Arrow,
            span: span.clone(),
        },
        Token {
            token_type: TokenType::Ident("x".to_string()),
            span: span.clone(),
        },
    ]
    .into_iter()
    .chain(inner_tokens.into_iter().map(|token| {
        if let TokenType::StuckOperator(op) = token.token_type {
            Token {
                token_type: TokenType::Operator(op),
                span: token.span.clone(),
            }
        } else {
            token
        }
    }))
    .collect::<Vec<_>>();

    let (other_remaining_tokens, lambda) =
        lambda_decl.process(ParseCtx::from(&new_inner_tokens, stream.config))?;

    if !other_remaining_tokens.is_empty() {
        return Err(ParseError::UnexpectedToken(
            TokenType::CloseParen.discriminant().to_string(),
            other_remaining_tokens.tokens[0].clone(),
        ));
    }

    Ok((stream, lambda))
}

pub fn suffix_function_shorthand(stream: Input) -> IResult<LambdaDecl> {
    let (stream, (span, inner_tokens)) =
        (get_span, consume_tokens_until(TokenType::CloseParen)).process(stream)?;

    if inner_tokens.is_empty() {
        return Err(ParseError::Fail);
    }

    let operator = inner_tokens.last().unwrap().clone();

    // Check if the last token is an operator suitable for function shorthand
    // Exclude "!" because it's used for zero-arg method calls (like show!), not as a binary operator
    match &operator.token_type {
        TokenType::Operator(op) if op != "!" => {}
        TokenType::StuckOperator(op) if op != "!" => {}
        _ => {
            return Err(ParseError::UnexpectedToken(
                TokenType::Operator("".to_string())
                    .discriminant()
                    .to_string(),
                operator.clone(),
            ));
        }
    }

    let inner_tokens = inner_tokens[..inner_tokens.len() - 1].to_vec();

    let new_inner_tokens = vec![
        Token {
            token_type: TokenType::Ident("x".to_string()),
            span: span.clone(),
        },
        Token {
            token_type: TokenType::Arrow,
            span: span.clone(),
        },
    ]
    .into_iter()
    .chain(inner_tokens.into_iter().map(|token| {
        if let TokenType::StuckOperator(op) = token.token_type {
            Token {
                token_type: TokenType::Operator(op),
                span: token.span.clone(),
            }
        } else {
            token
        }
    }))
    .chain(vec![
        operator,
        Token {
            token_type: TokenType::Ident("x".to_string()),
            span: span.clone(),
        },
    ])
    .collect::<Vec<_>>();

    let (other_remaining_tokens, lambda) =
        lambda_decl.process(ParseCtx::from(&new_inner_tokens, stream.config))?;

    if !other_remaining_tokens.is_empty() {
        return Err(ParseError::UnexpectedToken(
            TokenType::CloseParen.discriminant().to_string(),
            other_remaining_tokens.tokens[0].clone(),
        ));
    }

    Ok((stream, lambda))
}

pub fn function_sig(stream: Input) -> IResult<FunctionSig> {
    (
        TokenType::Keyword("unsafe".to_string()).opt(),
        self_receiver_mode.opt(),
        ident,
        TokenType::Colon,
        parse_type,
        preceded(
            TokenType::Keyword("where".to_string()),
            separated1(parse_where_clause, TokenType::Coma),
        )
        .opt(),
        TokenType::Eol,
    )
        .map(
            |(is_unsafe, self_receiver, name, _, sig, where_clauses, _)| FunctionSig {
                name,
                sig,
                where_clauses: where_clauses.unwrap_or_default(),
                self_receiver,
                is_unsafe: is_unsafe.is_some(),
                exported: false,
            },
        )
        .process(stream)
}

fn self_receiver_mode(stream: Input) -> IResult<SelfReceiverMode> {
    (
        TokenType::Caret
            .map(|_| SelfReceiverMode::Mut)
            .or(TokenType::Tilde.map(|_| SelfReceiverMode::Move))
            .opt(),
        TokenType::Arobase,
    )
        .map(|(mode, _)| mode.unwrap_or(SelfReceiverMode::Shared))
        .process(stream)
}
