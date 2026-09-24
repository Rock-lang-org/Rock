use crate::{
    ast::{
        visit, Argument, Block, DoStatement, Expression, Ident, IdentOrType, IdentPattern,
        IdentifierPath, LambdaArrowKind, LambdaDecl, Literal, LiteralKind, Loop, Match, MatchArm,
        Operand, Pattern, PatternKind, PrimaryExpr, SecondaryExpr, Statement, UnaryExpr,
    },
    lexer::TokenType,
    parser::engine::*,
};

use super::{
    empty_lines, expression, get_span, indent, pattern, reset_inside_argument_list, statement,
};

/// Expand sequencing while still in the syntax layer. The match supplies a
/// lexical block without introducing an extra closure or capturing its locals.
pub fn do_expression(stream: Input) -> IResult<Operand> {
    let (stream, (span, _)) = (get_span, TokenType::Keyword("do".into())).process(stream)?;
    let (stream, statements) = preceded(
        TokenType::Eol.followed_by(empty_lines),
        indented(separated1(
            preceded(indent, reset_inside_argument_list(do_statement)),
            TokenType::Eol.followed_by(empty_lines),
        )),
    )
    .process(stream)
    .map_err(|error| match error {
        ParseError::Fail => {
            ParseError::HardError("Expected an indented do block".into(), span.clone())
        }
        other => other.with_context("do block"),
    })?;

    let mut remaining = statements.clone();
    let Some(DoStatement::Statement(Statement::Expression(tail), _)) = remaining.pop() else {
        return Err(ParseError::HardError(
            "A do block must end with an expression".into(),
            span,
        ));
    };
    let mut body = vec![Statement::Expression(tail)];
    for entry in remaining.into_iter().rev() {
        let (parameter, action, bind_span) = match entry {
            DoStatement::Statement(Statement::Assignment(assignment), _) => {
                body.insert(0, Statement::Assignment(assignment));
                continue;
            }
            DoStatement::Bind(parameter, action, span) => (parameter, action, span),
            DoStatement::Statement(Statement::Expression(action), span) => {
                (wildcard(), action, span)
            }
            _ => unreachable!("control flow rejected by do_statement"),
        };
        let callback = expr(Operand::LambdaDecl(LambdaDecl {
            parameters: vec![parameter],
            body: Block { statements: body },
            arrow_kind: LambdaArrowKind::Normal,
            span: bind_span.clone(),
        }));
        body = vec![Statement::Expression(Expression::UnaryExpr(
            UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Ident(IdentifierPath {
                    path: vec![IdentOrType::Ident(Ident {
                        name: "bind".into(),
                        span: bind_span,
                    })],
                }),
                secondaries: Some(vec![SecondaryExpr::Arguments(vec![
                    Argument { arg: action },
                    Argument {
                        arg: expr(Operand::Expression(Box::new(callback))),
                    },
                ])]),
                type_annotation: None,
            }),
        ))];
    }
    Ok((
        stream,
        Operand::Match(Box::new(Match {
            expr: expr(Operand::Literal(Literal {
                kind: LiteralKind::Bool(true),
                span,
            })),
            arms: vec![MatchArm {
                pattern: wildcard(),
                condition: None,
                body: Block { statements: body },
            }],
            do_syntax: Some(statements),
        })),
    ))
}

fn do_statement(stream: Input) -> IResult<DoStatement> {
    let span = stream.seek()?.span.clone();
    let arrow = TokenType::Operator("<-".into()).or(TokenType::StuckOperator("<-".into()));
    let (rest, entry) = if let Ok((rest, (parameter, _))) = (pattern, arrow).process(stream) {
        if parameter.binding.is_some()
            || !matches!(
                &parameter.kind,
                PatternKind::Ident(IdentPattern { mut_: false, .. }) | PatternKind::Wildcard
            )
        {
            return Err(ParseError::HardError(
                "A do bind requires an identifier or '_'".into(),
                span,
            ));
        }
        let (rest, action) = expression(rest)?;
        (rest, DoStatement::Bind(parameter, action, span.clone()))
    } else {
        let (rest, value) = statement(stream)?;
        (rest, DoStatement::Statement(value, span.clone()))
    };
    let mut check = ControlFlowCheck {
        loops: 0,
        error: None,
    };
    match &entry {
        DoStatement::Bind(_, action, _) => action.visit(&mut check),
        DoStatement::Statement(value, _) => value.visit(&mut check),
    }
    if let Some(message) = check.error {
        return Err(ParseError::HardError(message.into(), span));
    }
    Ok((rest, entry))
}

fn wildcard() -> Pattern {
    Pattern {
        binding: None,
        kind: PatternKind::Wildcard,
    }
}

fn expr(operand: Operand) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand,
        secondaries: None,
        type_annotation: None,
    }))
}

struct ControlFlowCheck {
    loops: usize,
    error: Option<&'static str>,
}

impl<'ast> visit::Visitor<'ast> for ControlFlowCheck {
    fn visit_lambda_decl(&mut self, _: &'ast LambdaDecl) {}

    fn visit_loop(&mut self, node: &'ast Loop) {
        self.loops += 1;
        visit::walk_loop(self, node);
        self.loops -= 1;
    }

    fn visit_statement(&mut self, node: &'ast Statement) {
        match node {
            Statement::Return(_) => {
                self.error = Some("Use the final expression instead of return in a do block")
            }
            Statement::Break(_) | Statement::Continue(_) if self.loops == 0 => {
                self.error = Some("Control flow cannot leave a do block");
            }
            _ => visit::walk_statement(self, node),
        }
    }

    fn visit_secondary_expr(&mut self, node: &'ast SecondaryExpr) {
        if matches!(node, SecondaryExpr::Interogation) {
            self.error = Some("Use '<-' instead of '?' in a do block");
        } else {
            visit::walk_secondary_expr(self, node);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        fmt::{format, FormatInput},
        parser::parse_string,
        Config,
    };

    #[test]
    fn do_notation_format_roundtrip() {
        let source = "main = ->\n    result = do\n        x <- first!\n        _ <- second x\n        total = x + 1\n        pure total\n    result\n";
        let parsed = parse_string(source, &Config::default()).unwrap();
        let formatted = format(FormatInput::program(&parsed));
        assert!(formatted.contains("x <- first!"), "{formatted}");
        assert!(formatted.contains("_ <- second x"), "{formatted}");
        let reparsed = parse_string(&formatted, &Config::default()).unwrap();
        assert_eq!(formatted, format(FormatInput::program(&reparsed)));
    }

    #[test]
    fn do_notation_rejects_invalid_statements() {
        for source in [
            "main = -> do\n",
            "main = -> do\n    x <- action!\n",
            "main = -> do\n    x = 1\n",
            "main = -> do\n    (x, y) <- action!\n    pure x\n",
            "main = -> do\n    return action!\n",
            "main = -> do\n    x = action!?\n    pure x\n",
            "main = -> do\n    break\n    pure 1\n",
            "main = -> do\n    if true then return 1\n    pure 1\n",
        ] {
            assert!(
                parse_string(source, &Config::default()).is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn do_notation_allows_explicit_function_control_flow() {
        let source = "main = -> do\n    helper = ->\n        return 1\n    pure helper!\n";
        parse_string(source, &Config::default()).unwrap();
    }

    #[test]
    fn do_notation_nested_format_and_comments_roundtrip() {
        let source = "main = -> do\n    // Bind the outer value.\n    x <- first!\n    y <- do\n        z <- second x\n        pure z\n    pure (x + y)\n";
        let module =
            crate::parser::parse_source("example.rk".into(), source, &Config::default()).unwrap();
        let formatted = format(FormatInput::module_with_source(&module, source));
        assert!(
            formatted.contains("// Bind the outer value."),
            "{formatted}"
        );
        assert_eq!(formatted.matches("do").count(), 2, "{formatted}");
        let reparsed =
            crate::parser::parse_source("example.rk".into(), &formatted, &Config::default())
                .unwrap();
        assert_eq!(
            formatted,
            format(FormatInput::module_with_source(&reparsed, &formatted))
        );
    }

    #[test]
    fn do_notation_allows_local_loop_control_flow() {
        let source = "main = -> do\n    action = -> pure 1\n    ignored = loop\n        break\n    action!\n";
        parse_string(source, &Config::default()).unwrap();
    }
}
