use crate::{
    diagnostic::Diagnostics,
    lexer::{Span, Token, TokenType},
    parser::{expression, ParseCtx, ParseError, Parser},
};

use super::{
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
pub struct MacroArgMatcher<'args, 'ctx, 'cfg> {
    threads: Vec<MacroThread<'args>>,
    must_be_completed: bool,
    args: &'args [Token],
    macro_span: Span,
    invoc_span: Span,
    context: &'ctx MacroExpansionContext<'cfg>,
}

impl<'args, 'ctx, 'cfg> MacroArgMatcher<'args, 'ctx, 'cfg> {
    pub fn new(
        args: &'args [Token],
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
            must_be_completed: false,
            macro_span,
            invoc_span,
            context,
        }
    }

    pub fn run(&mut self) -> Result<Correspondance, Diagnostics> {
        self.must_be_completed = true;

        let (correspondances, _) = self.match_threads()?;

        Ok(correspondances)
    }

    fn match_threads(&mut self) -> Result<(Correspondance, &'args [Token]), Diagnostics> {
        let mut threads = self.threads.clone();
        let mut most_advanced_arg_idx = 0;

        while !has_one_solution(threads.clone(), self.must_be_completed) && !threads.is_empty() {
            let mut new_threads = vec![];

            for thread in &mut threads {
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
                                        .insert_direct(name.clone(), vec![arg.clone()]);

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
                                if let Ok((remaining_tokens, _)) = expression
                                    .process(ParseCtx::from(thread.args, self.context.config))
                                {
                                    thread.correspondances.insert_direct(
                                        name.clone(),
                                        thread.args[..thread.args.len() - remaining_tokens.len()]
                                            .to_vec(),
                                    );

                                    thread.args =
                                        &thread.args[thread.args.len() - remaining_tokens.len()..];

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
                                        .insert_direct(name.clone(), vec![arg.clone()]);

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
                                    repetition.clone(),
                                    self.macro_span.clone(),
                                    self.invoc_span.clone(),
                                    self.context,
                                );

                                let Ok((correspondances, new_args)) = matcher.match_threads()
                                else {
                                    break;
                                };
                                if new_args.len() == repeated_args.len() {
                                    break;
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
                } else {
                    // FOUND IT
                    if (self.must_be_completed
                        && thread.tokens.is_empty()
                        && thread.args.is_empty())
                        || (!self.must_be_completed && (thread.tokens.is_empty()))
                    {
                        return Ok((thread.correspondances.clone(), thread.args));
                    }
                }
            }
            self.threads = new_threads.clone();
            threads = new_threads.clone();
        }

        if let Some(thread) = get_correspondances_thread(threads.clone(), self.must_be_completed) {
            Ok((thread.correspondances.clone(), thread.args))
        } else {
            if most_advanced_arg_idx >= self.args.len() {
                most_advanced_arg_idx = self.args.len().saturating_sub(1);
            }

            let Some(last_found_arg) = self.args.get(most_advanced_arg_idx) else {
                return Err(ParseError::MacroNoCorrespondance {
                    macro_name: self.macro_span.clone(),
                    invoc_name: self.invoc_span.clone(),
                    invoc_arg: None,
                }
                .into());
            };

            Err(ParseError::MacroNoCorrespondance {
                macro_name: self.macro_span.clone(),
                invoc_name: self.invoc_span.clone(),
                invoc_arg: Some(last_found_arg.span.clone()),
            }
            .into())
        }
    }
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

fn has_one_solution(threads: Vec<MacroThread>, must_be_completed: bool) -> bool {
    threads.iter().any(|thread| {
        (must_be_completed
            && (remaining_is_all_repetition_or_empty(&thread.tokens) && thread.args.is_empty()))
            || (!must_be_completed
                && (remaining_is_all_repetition_or_empty(&thread.tokens) || thread.args.is_empty()))
    })
}

fn get_correspondances_thread(
    threads: Vec<MacroThread<'_>>,
    must_be_completed: bool,
) -> Option<MacroThread<'_>> {
    if must_be_completed {
        return threads
            .iter()
            .find(|thread| {
                remaining_is_all_repetition_or_empty(&thread.tokens) && thread.args.is_empty()
            })
            .cloned();
    }

    if let Some(found) = threads.iter().find(|thread| {
        remaining_is_all_repetition_or_empty(&thread.tokens) && thread.args.is_empty()
    }) {
        Some(found.clone())
    } else if let Some(found) = threads.iter().find(|thread| thread.args.is_empty()) {
        return Some(found.clone());
    } else if let Some(found) = threads
        .iter()
        .find(|thread| remaining_is_all_repetition_or_empty(&thread.tokens))
    {
        return Some(found.clone());
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::lexer::{Token, TokenType};
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
        }
    }

    #[test]
    fn macro_arg_matcher_accepts_compiled_matcher_fragments() {
        let config = test_config();
        let context = MacroExpansionContext::new(&config);
        let args = vec![Token::from(TokenType::Ident("main".to_string()))];
        let matcher = vec![MatcherFragment::Capture {
            name: "name".to_string(),
            kind: CaptureKind::Ident,
        }];

        let correspondance =
            MacroArgMatcher::new(&args, matcher, Span::test(), Span::test(), &context)
                .run()
                .unwrap();

        assert!(correspondance.get("name", 0).is_some());
    }
}
