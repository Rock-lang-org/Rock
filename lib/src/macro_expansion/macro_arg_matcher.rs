use crate::{
    diagnostic::Diagnostics,
    lexer::{Span, Token, TokenType},
    parser::ParseError,
};

use super::{
    TokenStream,
    token_tree::MACRO_LIMITS,
    context::MacroExpansionContext,
    correspondances::Correspondance,
    declarative::{CaptureKind, MatcherFragment},
};

#[derive(Clone, Debug)]
struct MacroThread<'a> {
    pub args: &'a [Token],
    pub tokens: Vec<MatcherFragment>,
    pub correspondances: Correspondance,
}

#[derive(Debug)]
pub(super) enum MatchError {
    NoMatch(Diagnostics),
    Invalid(Diagnostics),
}

impl MatchError {
    #[cfg(test)]
    pub(super) fn into_diagnostics(self) -> Diagnostics {
        match self { Self::NoMatch(diagnostics) | Self::Invalid(diagnostics) => diagnostics }
    }
}

impl From<Diagnostics> for MatchError {
    fn from(diagnostics: Diagnostics) -> Self { Self::Invalid(diagnostics) }
}

impl From<ParseError> for MatchError {
    fn from(error: ParseError) -> Self { Self::Invalid(error.into()) }
}

#[derive(Debug)]
pub struct MacroArgMatcher<'args, 'ctx, 'cfg> {
    threads: Vec<MacroThread<'args>>,
    must_be_completed: bool,
    args: &'args [Token],
    input: &'args TokenStream,
    header_indent: usize,
    ranges: &'args [std::ops::Range<usize>],
    macro_span: Span,
    invoc_span: Span,
    context: &'ctx MacroExpansionContext<'cfg>,
}

impl<'args, 'ctx, 'cfg> MacroArgMatcher<'args, 'ctx, 'cfg> {
    pub fn new(
        args: &'args [Token],
        input: &'args TokenStream,
        header_indent: usize,
        ranges: &'args [std::ops::Range<usize>],
        thread: Vec<MatcherFragment>,
        macro_span: Span,
        invoc_span: Span,
        context: &'ctx MacroExpansionContext<'cfg>,
    ) -> Self {
        Self {
            threads: vec![MacroThread {
                args,
                tokens: thread,
                correspondances: Correspondance::new(),
            }],
            args,
            input,
            header_indent,
            ranges,
            must_be_completed: false,
            macro_span,
            invoc_span,
            context,
        }
    }

    pub fn run(&mut self) -> Result<Correspondance, MatchError> {
        for thread in &self.threads {
            validate_matcher(&thread.tokens).map_err(|message| ParseError::HardError(message, self.macro_span.clone()))?;
        }
        self.must_be_completed = true;

        let (correspondances, _) = self.match_threads()?;

        Ok(correspondances)
    }

    fn capture(&self, args: &[Token], count: usize, expression: bool) -> Result<TokenStream, Diagnostics> {
        let offset = self.args.len() - args.len();
        let retained = self.ranges.get(offset..offset.saturating_add(count))
            .filter(|ranges| !ranges.is_empty())
            .ok_or_else(|| Diagnostics::from(ParseError::HardError("macro capture made no progress".into(), self.invoc_span.clone())))?;
        let range = retained[0].start..retained[retained.len() - 1].end;
        let mut view = crate::lexer::interpretation::RockView::new(self.input);
        view.advance_to(range.start).map_err(|error| ParseError::HardError(format!("invalid capture position: {error:?}"), self.invoc_span.clone()))?;
        let header_indent = view.header_indent(Some(self.header_indent));
        self.input.slice(range, MACRO_LIMITS).and_then(|input| {
            if expression { input.expression_fragment(header_indent, MACRO_LIMITS) } else { Ok(input) }
        }).map_err(|error| ParseError::HardError(
            format!("invalid macro capture: {error:?}"), self.invoc_span.clone(),
        ).into())
    }

    fn match_threads(&mut self) -> Result<(Correspondance, &'args [Token]), MatchError> {
        for thread in &self.threads {
            self.context.charge_matching(thread.tokens.len().saturating_add(thread.args.len()), &self.invoc_span)?;
        }
        let mut threads = self.threads.clone();
        let mut most_advanced_arg_idx = 0;
        let mut solution = None;

        while !threads.is_empty() {
            let mut new_threads = vec![];

            for thread in &mut threads {
                self.context.charge_matching(1usize.saturating_add(thread.tokens.len()).saturating_add(thread.args.len()), &self.invoc_span)?;
                if remaining_is_all_repetition_or_empty(&thread.tokens)
                    && (!self.must_be_completed || thread.args.is_empty())
                {
                    if !self.must_be_completed {
                        return Ok((thread.correspondances.clone(), thread.args));
                    }
                    if solution.is_some() {
                        return Err(ParseError::HardError("ambiguous declarative macro match".into(), self.invoc_span.clone()).into());
                    }
                    solution = Some((thread.correspondances.clone(), thread.args));
                    continue;
                }
                if let Some(arg) = thread.args.first() {
                    let tokens = &thread.tokens;

                    if tokens.is_empty() {
                        continue;
                    }

                    let fragment = &tokens[0];

                    match fragment {
                        MatcherFragment::Capture { name, kind } => match kind {
                            CaptureKind::Ident => {
                                if let TokenType::Ident(_) = arg.token_type {
                                    thread
                                        .correspondances
                                        .insert_direct(name.clone(), self.capture(thread.args, 1, false)?);

                                    new_threads.push(MacroThread {
                                        args: &thread.args[1..],
                                        tokens: tokens[1..].to_vec(),
                                        correspondances: thread.correspondances.clone(),
                                    });

                                    let last_found_arg =
                                        (self.args.len() - thread.args.len()).saturating_sub(1);

                                    if last_found_arg > most_advanced_arg_idx {
                                        most_advanced_arg_idx = last_found_arg;
                                    }
                                }
                            }
                            CaptureKind::Expr => {
                                let offset = self.args.len() - thread.args.len();
                                let start = self.ranges[offset].start;
                                let mut view = crate::lexer::interpretation::RockView::new(self.input);
                                view.advance_to(start).map_err(|error| ParseError::HardError(format!("invalid expression position: {error:?}"), self.invoc_span.clone()))?;
                                let margin = view.header_indent(Some(self.header_indent));
                                let remaining = self.input.slice(start..self.input.len_bytes(), MACRO_LIMITS)
                                    .map_err(|error| ParseError::HardError(format!("invalid expression input: {error:?}"), self.invoc_span.clone()))?;
                                if let Ok(bytes) = crate::parser::expression_prefix(&remaining, self.context.config, margin) {
                                    let end = start.checked_add(bytes).ok_or_else(|| ParseError::HardError("expression extent overflow".into(), self.invoc_span.clone()))?;
                                    let count = self.ranges[offset..].iter().take_while(|range| range.end <= end).count();
                                    if count == 0 { return Err(ParseError::HardError("macro expression capture made no progress".into(), self.invoc_span.clone()).into()); }
                                    thread.correspondances.insert_direct(
                                        name.clone(),
                                        self.capture(thread.args, count, true)?,
                                    );

                                    thread.args = &thread.args[count..];

                                    new_threads.push(MacroThread {
                                        args: thread.args,
                                        tokens: tokens[1..].to_vec(),
                                        correspondances: thread.correspondances.clone(),
                                    });

                                    let last_found_arg = self.args.len() - thread.args.len();

                                    if last_found_arg > most_advanced_arg_idx {
                                        most_advanced_arg_idx = last_found_arg;
                                    }
                                }
                            }
                            CaptureKind::Type => {
                                if let TokenType::Type(_) = arg.token_type {
                                    thread
                                        .correspondances
                                        .insert_direct(name.clone(), self.capture(thread.args, 1, false)?);

                                    new_threads.push(MacroThread {
                                        args: &thread.args[1..],
                                        tokens: tokens[1..].to_vec(),
                                        correspondances: thread.correspondances.clone(),
                                    });

                                    let last_found_arg =
                                        (self.args.len() - thread.args.len()).saturating_sub(1);

                                    if last_found_arg > most_advanced_arg_idx {
                                        most_advanced_arg_idx = last_found_arg;
                                    }
                                }
                            }
                        },
                        MatcherFragment::Token(t) => {
                            if t.token_type == arg.token_type {
                                new_threads.push(MacroThread {
                                    args: &thread.args[1..],
                                    tokens: tokens[1..].to_vec(),
                                    correspondances: thread.correspondances.clone(),
                                });

                                let last_found_arg =
                                    (self.args.len() - thread.args.len()).saturating_sub(1);

                                if last_found_arg > most_advanced_arg_idx {
                                    most_advanced_arg_idx = last_found_arg;
                                }
                            }
                        }
                        MatcherFragment::Repetition(repetition) => {
                            // Case no repetition
                            new_threads.push(MacroThread {
                                args: thread.args,
                                tokens: tokens[1..].to_vec(),
                                correspondances: thread.correspondances.clone(),
                            });

                            let mut repeated_args = thread.args;
                            let mut repeated_correspondances = thread.correspondances.clone();

                            loop {
                                let mut matcher = MacroArgMatcher::new(
                                    repeated_args,
                                    self.input,
                                    self.header_indent,
                                    &self.ranges[self.args.len() - repeated_args.len()..],
                                    repetition.clone(),
                                    self.macro_span.clone(),
                                    self.invoc_span.clone(),
                                    self.context,
                                );

                                let (correspondances, new_args) = match matcher.match_threads() {
                                    Ok(found) => found,
                                    Err(MatchError::NoMatch(_)) => break,
                                    Err(error) => return Err(error),
                                };
                                if new_args.len() == repeated_args.len() {
                                    return Err(ParseError::HardError(
                                        "declarative macro repetition made no input progress".into(), self.invoc_span.clone(),
                                    ).into());
                                }

                                repeated_correspondances.insert_nested(correspondances);
                                repeated_args = new_args;

                                // Case repetition found and it stops after this group.
                                new_threads.push(MacroThread {
                                    args: repeated_args,
                                    tokens: tokens[1..].to_vec(),
                                    correspondances: repeated_correspondances.clone(),
                                });
                            }
                        }
                    }
                }
            }
            self.threads = new_threads.clone();
            threads = new_threads.clone();
        }

        if let Some(solution) = solution {
            Ok(solution)
        } else {
            if most_advanced_arg_idx >= self.args.len() {
                most_advanced_arg_idx = self.args.len().saturating_sub(1);
            }

            let Some(last_found_arg) = self.args.get(most_advanced_arg_idx) else {
                return Err(MatchError::NoMatch(ParseError::MacroNoCorrespondance {
                    macro_name: self.macro_span.clone(),
                    invoc_name: self.invoc_span.clone(),
                    invoc_arg: None,
                }
                .into()));
            };

            Err(MatchError::NoMatch(ParseError::MacroNoCorrespondance {
                macro_name: self.macro_span.clone(),
                invoc_name: self.invoc_span.clone(),
                invoc_arg: Some(last_found_arg.span.clone()),
            }
            .into()))
        }
    }
}

/// The flat matcher ignores whitespace, physical line endings and Rock comments.
/// It does not synthesize parser layout or reinterpret arbitrary DSL units. Only
/// this temporary Rock view discards trivia; captures slice the retained input
/// from the first significant piece through the last, including internal trivia.
/// Spacing-sensitive Rock token kinds remain distinct during literal matching.
pub(super) fn rock_arguments(input: &TokenStream, config: &crate::Config, header_indent: usize) -> Result<(Vec<Token>, Vec<std::ops::Range<usize>>), Diagnostics> {
    use crate::lexer::interpretation::{PieceKind, RockView};
    let mut view = RockView::new(input);
    let mut tokens = Vec::new();
    let mut ranges = Vec::new();
    while let Some(piece) = view.next_piece().map_err(|error| ParseError::HardError(
        format!("invalid Rock macro arguments: {error:?}"), input.start_span(),
    ))? {
        if let PieceKind::Token(mut token_type) = piece.kind {
            let mut range = piece.range;
            let mut span = piece.span;
            if matches!(token_type, TokenType::MacroInvoc(_)) {
                let margin = view.header_indent(Some(header_indent));
                let remaining = input.slice(range.start..input.len_bytes(), MACRO_LIMITS)
                    .map_err(|error| ParseError::HardError(format!("invalid nested macro input: {error:?}"), span.clone()))?;
                if let Ok(bytes) = crate::parser::operand_prefix(&remaining, config, margin) {
                    let end = range.start.checked_add(bytes).filter(|end| *end >= view.offset() && *end <= input.len_bytes())
                        .ok_or_else(|| ParseError::HardError("nested macro input made no progress".into(), span.clone()))?;
                    let fragment = input.slice(range.start..end, MACRO_LIMITS)
                        .map_err(|error| ParseError::HardError(format!("invalid nested macro range: {error:?}"), span.clone()))?;
                    if fragment.runs().iter().any(|run| run.kind() == crate::lexer::lossless::RunKind::LineEnding) {
                        range.end = end;
                        span.end = fragment.end_span().end;
                        token_type = TokenType::ExpressionFragment(Box::new(crate::lexer::lossless::ExpressionFragment { input: fragment, header_indent: margin }));
                        view.advance_to(range.end).map_err(|error| ParseError::HardError(format!("invalid nested macro boundary: {error:?}"), span.clone()))?;
                    }
                }
                // An unsuccessful probe does not globally validate literal
                // matcher input as an expression or a balanced group.
            }
            tokens.push(Token { token_type, span, context: piece.context.unwrap_or_default() });
            ranges.push(range);
        }
    }
    Ok((tokens, ranges))
}

fn remaining_is_all_repetition_or_empty(tokens: &[MatcherFragment]) -> bool {
    if tokens.is_empty() {
        return true;
    }

    tokens.iter().all(|token| {
        if let MatcherFragment::Repetition(_) = token {
            true
        } else {
            false
        }
    })
}

pub(super) fn validate_matcher(fragments: &[MatcherFragment]) -> Result<(), String> {
    for fragment in fragments {
        if let MatcherFragment::Repetition(inner) = fragment {
            if inner.iter().any(|fragment| matches!(fragment, MatcherFragment::Repetition(_))) {
                return Err("nested repetition is not supported in declarative macro matchers".into());
            }
            if inner.is_empty() {
                return Err("declarative macro repetition must make input progress".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::macro_expansion::declarative::{CaptureKind, MatcherFragment};

    fn test_config() -> crate::Config {
        crate::Config {
            entry_file: PathBuf::from("/test.rk"),
            output_dir: PathBuf::new(),
            debug_print: Vec::new(),
            meta_files: Vec::new(),
            extern_artifacts: Vec::new(),
            source_providers: Vec::new(),
            current_crate_name: None,
            opt_level: 0,
            overflow_checks: None,
            emit_llvm: false,
            no_link: false,
            emit_object: None,
            no_prelude: false,
            no_std: false,
            sysroot: None,
            no_proc_execution: false,
        }
    }

    #[test]
    fn macro_arg_matcher_accepts_compiled_matcher_fragments() {
        let config = test_config();
        let context = MacroExpansionContext::new(&config);
        let input = super::super::token_tree::source("main");
        let (args, ranges) = rock_arguments(&input, &config, 0).unwrap();
        let matcher = vec![MatcherFragment::Capture {
            name: "name".to_string(),
            kind: CaptureKind::Ident,
        }];

        let correspondance =
            MacroArgMatcher::new(&args, &input, 0, &ranges, matcher, Span::test(), Span::test(), &context)
                .run()
                .unwrap();

        assert!(correspondance.get("name", 0).is_some());
    }
}
