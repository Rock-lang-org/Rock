use crate::lexer::{Token, TokenType};
use crate::parser::{
    engine::*, Argument, Expression, Ident, IdentOrNumber, IdentOrType, Operand, Operator,
    PrimaryExpr, RangeExpr, SecondaryExpr, Tuple, UnaryExpr,
};

use super::{
    ampersand_token, block, empty_lines, function_shorthand, get_span, ident, ident_path, indent,
    instance, int, lambda_decl, mut_prefix, native_operator, operator, operator_token, parenthesis,
    r#loop, r#match,
};
use super::{indent_token, parse_if, parse_type};
use super::{literal, stuck_operator_token};

pub fn expression(stream: Input) -> IResult<Expression> {
    let (mut stream, mut expression) = range_expression(stream)?;

    if stream.inside_argument_list {
        return Ok((stream, expression));
    }

    while matches!(
        stream.tokens.first().map(|token| &token.token_type),
        Some(TokenType::SpacedDot)
    ) {
        let (next_stream, _) = TokenType::SpacedDot.process(stream)?;
        let (next_stream, member) = ident_or_number.process(next_stream)?;
        let (next_stream, trailing) = many(secondary).process(next_stream)?;

        let mut secondaries = vec![SecondaryExpr::Dot(member)];
        secondaries.extend(trailing);
        expression = append_secondaries(expression, secondaries);
        stream = next_stream;
    }

    Ok((stream, expression))
}

fn range_expression(stream: Input) -> IResult<Expression> {
    if let Ok((stream, (span, inclusive))) = range_operator(stream) {
        let (stream, end) = expression_without_spaced_dot.opt().process(stream)?;
        if inclusive && end.is_none() {
            return Err(ParseError::HardError(
                "inclusive range must have an end expression".to_string(),
                span,
            ));
        }
        return Ok((
            stream,
            Expression::Range(RangeExpr {
                start: None,
                end: end.map(Box::new),
                inclusive,
                span,
            }),
        ));
    }

    let (stream, start) = expression_without_spaced_dot(stream)?;
    let Ok((stream, (span, inclusive))) = range_operator(stream) else {
        return Ok((stream, start));
    };
    let (stream, end) = expression_without_spaced_dot.opt().process(stream)?;
    if inclusive && end.is_none() {
        return Err(ParseError::HardError(
            "inclusive range must have an end expression".to_string(),
            span,
        ));
    }

    Ok((
        stream,
        Expression::Range(RangeExpr {
            start: Some(Box::new(start)),
            end: end.map(Box::new),
            inclusive,
            span,
        }),
    ))
}

fn range_operator(stream: Input) -> IResult<(crate::lexer::Span, bool)> {
    (get_span, TokenType::DoubleDotEqual)
        .map(|(span, _)| (span, true))
        .or((get_span, TokenType::DoubleDot).map(|(span, _)| (span, false)))
        .process(stream)
}

fn expression_without_spaced_dot(stream: Input) -> IResult<Expression> {
    let (stream, unary) = unary_expr(stream)?;
    let (stream, base) = parse_cast_suffix(stream, Expression::UnaryExpr(unary))?;
    let (stream, binop) = (
        operator,
        // Allow expression continuation on the same line
        expression_without_spaced_dot
            // Or on the next line with indentation
            .or(preceded(
                TokenType::Eol,
                preceded(
                    empty_lines,
                    preceded(indent_token, expression_without_spaced_dot),
                ),
            )),
    )
        // Or operator at the beginning of the next line (indented by one level)
        .or(multiline_operator_continuation)
        .opt()
        .process(stream)
        .map_err(|e| e.with_context("expression"))?;

    let Some((op, rhs)) = binop else {
        return Ok((stream, base));
    };
    let lhs = match base {
        Expression::UnaryExpr(unary) => unary,
        expression => UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Expression(Box::new(expression)),
            secondaries: None,
            type_annotation: None,
        }),
    };
    Ok((stream, Expression::BinopExpr(lhs, op, Box::new(rhs))))
}

fn append_secondaries(expression: Expression, mut trailing: Vec<SecondaryExpr>) -> Expression {
    if let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(mut primary)) = expression {
        primary
            .secondaries
            .get_or_insert_with(Vec::new)
            .append(&mut trailing);
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary))
    } else {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Expression(Box::new(expression)),
            secondaries: Some(trailing),
            type_annotation: None,
        }))
    }
}

fn parse_cast_suffix(stream: Input, base: Expression) -> IResult<Expression> {
    if let Ok((stream_after_as, _)) = TokenType::Keyword("as".to_string()).process(stream) {
        if let Ok((stream2, ty)) = parse_type(stream_after_as) {
            return Ok((stream2, Expression::CastExpr(Box::new(base), ty)));
        }
    }
    Ok((stream, base))
}

/// Parse a multiline operator continuation: operator at the beginning of the next line,
/// indented by exactly one level relative to the current indent.
/// Unlike `indented()`, this does NOT increase the indent context, so repeated
/// continuations all happen at the same indent level:
/// ```ignore
/// 2 + 3 + 4
///     + 5 + 6
///     + 7
/// ```
/// This is also disallowed in single-line function bodies (no newline after `->`)
fn multiline_operator_continuation(stream: Input) -> IResult<(Operator, Expression)> {
    if stream.disallow_multiline_operators {
        return Err(ParseError::Fail);
    }

    let expected_indent = stream.indent_level + stream.indent_step;

    // Match Eol
    let (stream, _) = TokenType::Eol.process(stream)?;
    // Match empty lines
    let (stream, _) = empty_lines.process(stream)?;
    // Match indent token and check it's at the expected level
    let indent_span = stream
        .seek()
        .map(|token| token.span)
        .unwrap_or_else(|_| stream.eof_span());
    let (stream, level) = indent_token.process(stream)?;
    if (level as usize) != expected_indent {
        return Err(ParseError::UnexpectedIndent(level, indent_span));
    }
    // Parse operator and expression WITHOUT increasing indent context
    // This allows subsequent continuations at the same indent level
    // A stuck prefix operator starts the nested body, not an infix continuation.
    (
        operator_token.or(ampersand_token),
        expression_without_spaced_dot,
    )
        .process(stream)
}

pub fn unary_expr(stream: Input) -> IResult<UnaryExpr> {
    // Try: stuck_operator, `&mut`, `&`, or primary_expr
    (stuck_operator_token, unary_expr)
        .map(|(op, unary)| UnaryExpr::UnaryExpr(op, Box::new(unary)))
        .or(
            (ampersand_token, mut_prefix.opt(), unary_expr).map(|(mut op, mut_tok, unary)| {
                if mut_tok.is_some() {
                    op.value = "&mut".to_string();
                }
                UnaryExpr::UnaryExpr(op, Box::new(unary))
            }),
        )
        .or(primary_expr.map(UnaryExpr::PrimaryExpr))
        .process(stream)
        .map_err(|e| e.with_context("unary expression"))
}

pub fn primary_expr(stream: Input) -> IResult<PrimaryExpr> {
    let (stream, (operand, mut secondaries_vec, type_annotation)) = (
        operand,
        many(secondary),
        preceded(TokenType::Colon, parse_type).opt(),
    )
        .process(stream)
        .map_err(|e| e.with_context("primary expression"))?;

    move_trailing_argument_interogation_to_call(&mut secondaries_vec);

    // Reject `Type.method` syntax: Instance with no fields followed by a Dot secondary
    // means the user wrote `String.from_str` instead of `String::from_str`.
    if let Operand::Instance(ref inst) = operand {
        if inst.fields.is_empty() {
            if let Some(SecondaryExpr::Dot(IdentOrNumber::Ident(ref method_ident))) =
                secondaries_vec.first()
            {
                let type_name = inst
                    .name
                    .path
                    .last()
                    .map(|seg| match seg {
                        IdentOrType::Type(t) => t.type_name(),
                        IdentOrType::Ident(i) => i.name.clone(),
                    })
                    .unwrap_or_else(|| "Type".to_string());
                return Err(ParseError::HardError(
                    format!(
                        "Use '{}::{}' instead of '{}.{}' for associated function calls",
                        type_name, method_ident.name, type_name, method_ident.name
                    ),
                    method_ident.span.clone(),
                ));
            }
        }
    }

    Ok((
        stream,
        PrimaryExpr {
            operand,
            secondaries: if secondaries_vec.is_empty() {
                None
            } else {
                Some(secondaries_vec)
            },
            type_annotation,
        },
    ))
}

fn move_trailing_argument_interogation_to_call(secondaries: &mut Vec<SecondaryExpr>) {
    let mut index = 0;
    while index < secondaries.len() {
        let move_to_call = match &mut secondaries[index] {
            SecondaryExpr::Arguments(args) => args
                .last_mut()
                .is_some_and(|arg| remove_trailing_interogation(&mut arg.arg)),
            _ => false,
        };

        if move_to_call {
            secondaries.insert(index + 1, SecondaryExpr::Interogation);
            index += 1;
        }
        index += 1;
    }
}

fn remove_trailing_interogation(expr: &mut Expression) -> bool {
    let Expression::UnaryExpr(unary) = expr else {
        return false;
    };

    remove_trailing_interogation_from_unary(unary)
}

fn remove_trailing_interogation_from_unary(unary: &mut UnaryExpr) -> bool {
    let primary = match unary {
        UnaryExpr::PrimaryExpr(primary) => primary,
        UnaryExpr::UnaryExpr(_, inner) => {
            return remove_trailing_interogation_from_unary(inner);
        }
    };

    let Some(secondaries) = primary.secondaries.as_mut() else {
        return false;
    };
    if !matches!(secondaries.last(), Some(SecondaryExpr::Interogation)) {
        return false;
    }

    secondaries.pop();
    if secondaries.is_empty() {
        primary.secondaries = None;
    }
    true
}

pub fn operand(stream: Input) -> IResult<Operand> {
    parse_if
        .map(Box::new)
        .map(Operand::If)
        .or(r#loop.map(Box::new).map(Operand::Loop))
        .or(r#match.map(Box::new).map(Operand::Match))
        .or((get_span, TokenType::Keyword("unsafe".to_string()), block)
            .map(|(span, _, block)| Operand::Unsafe(block, span)))
        .or(self_ident)
        .or(instance.map(Operand::Instance))
        .or(tuple
            .followed_by(not(TokenType::DoubleColon))
            .map(Operand::Tuple))
        .or(function_shorthand.map(Operand::LambdaDecl))
        // Try lambda_decl before parenthesized expressions so nested parameter
        // patterns such as `(&value) -> ...` are not parsed as unary borrows.
        .or(lambda_decl.map(Operand::LambdaDecl))
        .or(parenthesis(reset_inside_argument_list(expression))
            .followed_by(not(TokenType::DoubleColon))
            .map(Box::new)
            .map(Operand::Expression))
        // TODO: disallow function calls after literal
        .or(literal.map(Operand::Literal))
        // Try lambda_decl before ident_path so that "param ->" is parsed as a lambda, not as an ident
        .or(native_operator.map(Operand::NativeOperator))
        .or(preceded(not(operator), ident_path.map(Operand::Ident)))
        .process(stream)
        .map_err(|e| e.with_context("operand"))
}

pub fn multiline_tuple(stream: Input) -> IResult<Vec<Expression>> {
    // For multiline tuples inside parentheses, we don't use indented() because
    // the parentheses themselves provide the grouping. We just need to handle
    // newlines and optional indentation.
    preceded(
        TokenType::Eol,
        preceded(
            empty_lines,
            separated_trailing(
                preceded(indent_token, separated1(expression, TokenType::Coma)),
                (
                    TokenType::Coma.opt(),
                    TokenType::Eol.followed_by(empty_lines),
                ),
            ),
        )
        .map(|elements| elements.into_iter().flatten().collect::<Vec<_>>()),
    )
    .followed_by(indent_token)
    .process(stream)
}

pub fn monoline_tuple(stream: Input) -> IResult<Vec<Expression>> {
    separated1(expression, TokenType::Coma).process(stream)
}

pub fn tuple(stream: Input) -> IResult<Tuple> {
    let (stream, (open_span, elements)) = (
        get_span,
        parenthesis(reset_inside_argument_list(
            multiline_tuple.or(monoline_tuple),
        )),
    )
        .process(stream)?;
    if elements.len() < 2 {
        Err(ParseError::UnexpectedToken(
            TokenType::OpenParen.discriminant().to_string(),
            Token {
                token_type: TokenType::OpenParen,
                span: open_span,
            },
        ))
    } else {
        Ok((stream, Tuple { elements }))
    }
}

pub fn self_ident(stream: Input) -> IResult<Operand> {
    preceded(TokenType::Arobase, ident.map(Operand::SelfIdent))
        .or((get_span, TokenType::Arobase).map(|(span, _)| {
            Operand::SelfIdent(Ident {
                name: "self".to_string(),
                span,
            })
        }))
        .process(stream)
}

pub fn secondary(stream: Input) -> IResult<SecondaryExpr> {
    // Check for argument list short circuit on multiline dots and double dots
    // For inline argument lists (e.g., .method1 a), multiline dots should close the argument list
    // UNLESS the dot is more indented than the current level (meaning it's part of the argument)
    // For multiline argument lists, only close if the dot is at the method chain level
    if stream.inside_argument_list {
        // Check for multiline dot
        if let Ok((_, (_, indent_level, _))) =
            (TokenType::Eol, indent_token, TokenType::Dot).process(stream)
        {
            // If we're in an inline argument list, multiline dots should close it
            // UNLESS the dot is more indented (meaning it's part of the argument expression)
            if stream.inside_inline_argument_list {
                // Only short-circuit if the dot is at or below the current indent level
                // Dots that are more indented are part of the argument expression
                if (indent_level as usize) <= stream.indent_level {
                    return arguments_list_short_circuit(stream)
                        .and_then(|_| Err(ParseError::ShortCircuit));
                }
            }

            // For multiline argument lists, calculate the method chain indent level
            // Arguments are at stream.indent_level, method chains would be at indent_level - indent_step
            let method_chain_level = if stream.indent_level >= stream.indent_step {
                stream.indent_level - stream.indent_step
            } else {
                0
            };

            // Only close if the dot is at or below the method chain level
            if (indent_level as usize) <= method_chain_level + stream.indent_step {
                return arguments_list_short_circuit(stream)
                    .and_then(|_| Err(ParseError::ShortCircuit));
            }
        }
    }

    let result = indice
        .map(SecondaryExpr::Indice)
        .or(dot.map(SecondaryExpr::Dot))
        .or(arguments.map(SecondaryExpr::Arguments))
        .or(TokenType::Interogation.map(|_| SecondaryExpr::Interogation))
        .process(stream)?;

    let (mut stream, secondary) = result;

    // Clear one-shot parser context flags after consuming a non-dot secondary.
    stream.after_closing_paren = false;
    if !matches!(secondary, SecondaryExpr::Dot(_)) {
        stream.after_multiline_dot = false;
    }

    Ok((stream, secondary))
}

pub fn arguments(stream: Input) -> IResult<Vec<Argument>> {
    TokenType::StuckOperator("!".to_string())
        .map(|_| vec![])
        .or(TokenType::Operator("!".to_string()).map(|_| vec![]))
        .or(preceded(
            not_inline_call_operator,
            inside_inline_argument_list(separated1(
                call_argument_expression.map(|arg| Argument { arg }),
                TokenType::Coma,
            )),
        ))
        .or(preceded(
            not(operator_token.or(ampersand_token)),
            preceded(
                not_multi_line_fn_call_short_circuit,
                preceded(
                    TokenType::Eol,
                    // Use context-aware argument parsing to avoid ambiguity
                    multiline_arguments_context_aware,
                ),
            ),
        ))
        .process(stream)
}

fn not_inline_call_operator(stream: Input) -> IResult<()> {
    let first_token = stream.tokens.first();
    let second_token = stream.tokens.get(1);
    let first = first_token.map(|token| &token.token_type);
    let second = second_token.map(|token| &token.token_type);
    if matches!(
        first,
        Some(TokenType::DoubleDot | TokenType::DoubleDotEqual)
    ) {
        return Err(ParseError::Fail);
    }
    let starts_shared_reference = matches!(first, Some(TokenType::Ampersand))
        && first_token
            .zip(second_token)
            .is_some_and(|(ampersand, operand)| ampersand.span.end == operand.span.start);
    let starts_mut_reference = matches!(
        (first, second),
        (
            Some(TokenType::Ampersand),
            Some(TokenType::Keyword(keyword))
        ) if keyword == "mut"
    ) || matches!(
        (first, second),
        (Some(TokenType::Ampersand), Some(TokenType::Caret))
    );

    if starts_shared_reference || starts_mut_reference {
        Ok((stream, ()))
    } else {
        not(operator_token.or(ampersand_token)).process(stream)
    }
}

fn call_argument_expression(stream: Input) -> IResult<Expression> {
    call_hole_expression.or(expression).process(stream)
}

fn call_hole_expression(stream: Input) -> IResult<Expression> {
    (
        get_span,
        TokenType::Underscore,
        not(TokenType::Arrow
            .or(TokenType::UnitArrow)
            .or(TokenType::CurriedArrow)),
    )
        .map(|(span, _, _)| {
            Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::CallHole(span),
                secondaries: None,
                type_annotation: None,
            }))
        })
        .process(stream)
}

// Wrapper parser that restores indent level after parsing an expression
// Used for multiline arguments to prevent nested multiline dots from affecting subsequent arguments
struct ExpressionWithIndentRestore {
    target_indent: usize,
}

impl Parser for ExpressionWithIndentRestore {
    type Output = Argument;

    fn process<'a>(&mut self, mut stream: Input<'a>) -> IResult<'a, Self::Output> {
        // Restore the indent level before checking indent
        // This ensures that nested multiline dots from the previous argument don't affect this one
        stream.indent_level = self.target_indent;

        let (stream, _) = indent.process(stream)?;
        let (mut stream, expr) = call_hole_expression.or(expression).process(stream)?;

        // Restore again after parsing the expression
        stream.indent_level = self.target_indent;
        Ok((stream, Argument { arg: expr }))
    }
}

pub fn multiline_arguments_context_aware(stream: Input) -> IResult<Vec<Argument>> {
    // Smart argument parsing that prevents ambiguous syntax
    // Arguments must be indented MORE than the current context to avoid ambiguity

    // Check the actual indentation level of the arguments
    let (arg_indent_level, arg_indent_span) = if let Ok(token) = stream.seek() {
        if let TokenType::Indent(level) = token.token_type {
            (level as usize, token.span)
        } else {
            (0, token.span)
        }
    } else {
        return Err(ParseError::UnexpectedEOF(stream.eof_span()));
    };

    // Arguments must be indented MORE than the current context
    if arg_indent_level <= stream.indent_level {
        // Not indented at all - definitely not arguments
        return Err(ParseError::UnexpectedIndent(
            arg_indent_level as u8,
            arg_indent_span.clone(),
        ));
    }

    // Prevent ambiguous cases where arguments could be confused with method chains
    // This only applies at the base level (indent 0) where multiline dots create ambiguity
    // Inside function bodies or other nested contexts, there's no ambiguity
    if stream.indent_level == 0 && stream.after_multiline_dot {
        if arg_indent_level == stream.indent_step {
            // At base level immediately after a multiline dot chain, one indent step is ambiguous:
            // it could start arguments or continue the surrounding call chain.
            return Err(ParseError::UnexpectedIndent(
                arg_indent_level as u8,
                arg_indent_span,
            ));
        }
    }

    // Parse arguments at their actual indent level (which we've already validated)
    // We need to temporarily set the stream's indent level to match the arguments
    let original_indent = stream.indent_level;
    let mut arg_stream = stream;
    arg_stream.indent_level = arg_indent_level;

    let result = inside_argument_list(separated1(
        ExpressionWithIndentRestore {
            target_indent: arg_indent_level,
        },
        TokenType::Eol,
    ))
    .process(arg_stream);

    // Restore the original indent level in the returned stream
    match result {
        Ok((mut stream, args)) => {
            stream.indent_level = original_indent;
            Ok((stream, args))
        }
        Err(e) => Err(e),
    }
}

pub fn inside_argument_list<P: Parser>(mut parser: P) -> impl FnMut(Input) -> IResult<P::Output> {
    move |mut stream| {
        let old_value = stream.inside_argument_list;
        stream.inside_argument_list = true;

        match parser.process(stream) {
            Ok((mut stream, t)) => {
                stream.inside_argument_list = old_value;

                Ok((stream, t))
            }
            Err(e) => {
                stream.inside_argument_list = old_value;

                Err(e)
            }
        }
    }
}

pub fn inside_inline_argument_list<P: Parser>(
    mut parser: P,
) -> impl FnMut(Input) -> IResult<P::Output> {
    move |mut stream| {
        let old_value = stream.inside_argument_list;
        let old_inline_value = stream.inside_inline_argument_list;
        stream.inside_argument_list = true;
        stream.inside_inline_argument_list = true;

        match parser.process(stream) {
            Ok((mut stream, t)) => {
                stream.inside_argument_list = old_value;
                stream.inside_inline_argument_list = old_inline_value;

                Ok((stream, t))
            }
            Err(e) => {
                // Don't modify the stream on error - it's not returned anyway
                // Just propagate the error
                Err(e)
            }
        }
    }
}

pub fn reset_inside_argument_list<P: Parser>(
    mut parser: P,
) -> impl FnMut(Input) -> IResult<P::Output> {
    move |mut stream| {
        let old_value = stream.inside_argument_list;
        let old_inline_value = stream.inside_inline_argument_list;
        stream.inside_argument_list = false;
        stream.inside_inline_argument_list = false;

        match parser.process(stream) {
            Ok((mut stream, t)) => {
                stream.inside_argument_list = old_value;
                stream.inside_inline_argument_list = old_inline_value;

                Ok((stream, t))
            }
            Err(e) => {
                stream.inside_argument_list = old_value;
                stream.inside_inline_argument_list = old_inline_value;

                Err(e)
            }
        }
    }
}

pub fn disallow_multiline_operators<P: Parser>(
    mut parser: P,
) -> impl FnMut(Input) -> IResult<P::Output> {
    move |mut stream| {
        let old_value = stream.disallow_multiline_operators;
        stream.disallow_multiline_operators = true;

        match parser.process(stream) {
            Ok((mut stream, t)) => {
                stream.disallow_multiline_operators = old_value;
                Ok((stream, t))
            }
            Err(e) => Err(e),
        }
    }
}

pub fn disallow_multiline_fn_call<P: Parser>(
    mut parser: P,
) -> impl FnMut(Input) -> IResult<P::Output> {
    move |mut stream| {
        let old_value = stream.disallowed_multiline_fn_call;
        stream.disallowed_multiline_fn_call = true;

        match parser.process(stream) {
            Ok((mut stream, t)) => {
                stream.disallowed_multiline_fn_call = old_value;

                Ok((stream, t))
            }
            Err(e) => {
                stream.disallowed_multiline_fn_call = old_value;

                Err(e)
            }
        }
    }
}

pub fn not_multi_line_fn_call_short_circuit(stream: Input) -> IResult<()> {
    if stream.disallowed_multiline_fn_call {
        Err(ParseError::ShortCircuit)
    } else {
        Ok((stream, ()))
    }
}

pub fn arguments_list_short_circuit(stream: Input) -> IResult<()> {
    // Don't modify the stream here - just return the error
    // The wrapper functions will handle restoring the flags
    if !stream.inside_argument_list {
        return Ok((stream, ()));
    }

    Err(ParseError::ShortCircuit)
}

pub fn dot(stream: Input) -> IResult<IdentOrNumber> {
    // Short-circuit inline dots after closing paren in argument context (e.g., foo(x).method)
    // This prevents .method from being parsed as part of the argument
    if stream.after_closing_paren && stream.inside_argument_list {
        if let Ok(_) = TokenType::Dot.process(stream) {
            return arguments_list_short_circuit(stream)
                .and_then(|_| Err(ParseError::ShortCircuit));
        }
    }

    // For multiline dots, update the indent level to match the dot's indent
    // This is needed so that lambda arguments after the dot have the correct indent context
    // But don't do this inside argument lists, as it would break multiline argument parsing
    let result = (TokenType::Eol, indent_token, TokenType::Dot).process(stream);

    if let Ok((stream, (_, dot_indent_level, _))) = result {
        // If the dot's indent level is less than the current indent level,
        // it belongs to an outer scope and should not be consumed here
        // This prevents lambda bodies from consuming dots that belong to the outer expression
        if (dot_indent_level as usize) < stream.indent_level {
            return Err(ParseError::Fail);
        }

        // Parse the identifier after the dot
        let (stream, ident) = ident_or_number.process(stream)?;

        // Update the indent level to match the dot's indent, but only if we're not inside an argument list
        // Inside argument lists, the indent level is managed by ExpressionWithIndentRestore
        let mut stream = stream;
        if !stream.inside_argument_list {
            stream.indent_level = dot_indent_level as usize;
        }
        stream.after_multiline_dot = true;

        return Ok((stream, ident));
    }

    // Try inline dots
    let (mut stream, ident) = preceded(TokenType::Dot, ident_or_number).process(stream)?;
    stream.after_multiline_dot = false;
    Ok((stream, ident))
}

pub fn ident_or_number(stream: Input) -> IResult<IdentOrNumber> {
    ident
        .map(IdentOrNumber::Ident)
        .or(int.map(IdentOrNumber::Number))
        .process(stream)
}

pub fn indice(stream: Input) -> IResult<Box<Expression>> {
    preceded(
        TokenType::OpenBracket,
        followed(expression.map(Box::new), TokenType::CloseBracket),
    )
    .process(stream)
}
