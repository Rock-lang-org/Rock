use std::collections::HashMap;

use crate::ast::{MacroDecl, MacroFragment};
use crate::lexer::{Token, TokenType};
use crate::macro_expansion::{ExpansionId, GeneratedSourceId, TokenStream};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarativeMacro {
    pub name: String,
    pub arms: Vec<DeclarativeMacroArm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarativeMacroArm {
    pub matcher: MacroMatcher,
    pub template: MacroTemplate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroMatcher {
    pub fragments: Vec<MatcherFragment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroTemplate {
    pub fragments: Vec<TemplateFragment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind {
    Ident,
    Expr,
    Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatcherFragment {
    Capture { name: String, kind: CaptureKind },
    Token(Token),
    Repetition(Vec<MatcherFragment>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateFragment {
    Capture {
        name: String,
    },
    Token(Token),
    /// `line_indent` is the indentation of the template line when a single-line
    /// repetition fills that whole line: each copy is then emitted as its own
    /// line. Any other repetition repeats its copies inline.
    Repetition {
        fragments: Vec<TemplateFragment>,
        line_indent: Option<Token>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct CaptureSet {
    direct: HashMap<String, Vec<TokenStream>>,
    repeated: HashMap<String, Vec<TokenStream>>,
}

impl CaptureSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_direct(&mut self, name: String, stream: TokenStream) {
        let streams = self.direct.entry(name).or_default();
        if !streams.contains(&stream) {
            streams.push(stream);
        }
    }

    pub fn get_direct(&self, name: &str) -> Option<&[TokenStream]> {
        self.direct.get(name).map(Vec::as_slice)
    }

    pub fn insert_repeated(&mut self, name: String, streams: Vec<TokenStream>) {
        self.repeated.insert(name, streams);
    }

    pub fn get_repeated(&self, name: &str) -> Option<&[TokenStream]> {
        self.repeated.get(name).map(Vec::as_slice)
    }
}

impl DeclarativeMacro {
    pub fn from_ast(decl: &MacroDecl) -> Self {
        Self {
            name: decl.name.name.clone(),
            arms: decl
                .entries
                .iter()
                .map(|entry| DeclarativeMacroArm {
                    matcher: MacroMatcher {
                        fragments: compile_matcher_fragments(&entry.defs),
                    },
                    template: MacroTemplate {
                        fragments: compile_template_fragments(&entry.body),
                    },
                })
                .collect(),
        }
    }
}

impl MacroTemplate {
    #[cfg(test)]
    pub fn expand(&self, captures: &CaptureSet) -> Result<TokenStream, String> {
        expand_template_fragments(&self.fragments, captures, None)
    }

    pub fn expand_with_origins(
        &self,
        captures: &CaptureSet,
        expansion: ExpansionId,
        generated_source: GeneratedSourceId,
    ) -> Result<TokenStream, String> {
        expand_template_fragments_with_origins(
            &self.fragments,
            captures,
            None,
            expansion,
            generated_source,
        )
    }
}

#[cfg(test)]
fn expand_template_fragments(
    fragments: &[TemplateFragment],
    captures: &CaptureSet,
    repeated_index: Option<usize>,
) -> Result<TokenStream, String> {
    let mut trees = Vec::new();
    for fragment in fragments {
        match fragment {
            TemplateFragment::Capture { name } => {
                if let Some(index) = repeated_index {
                    if let Some(streams) = captures.get_repeated(name) {
                        let Some(stream) = streams.get(index) else {
                            return Err(format!("missing repeated macro capture '{}'", name));
                        };
                        trees.extend(stream.trees.clone());
                        continue;
                    }
                }

                let Some(streams) = captures.get_direct(name) else {
                    return Err(format!("missing macro capture '{}'", name));
                };
                for stream in streams {
                    trees.extend(stream.trees.clone());
                }
            }
            TemplateFragment::Token(token) => {
                trees.extend(TokenStream::from_tokens(vec![token.clone()]).trees);
            }
            TemplateFragment::Repetition {
                fragments: inner,
                line_indent,
            } => {
                let capture_names = capture_names_in_template(inner);
                let mut len = None;

                for name in &capture_names {
                    if let Some(streams) = captures.get_repeated(name) {
                        match len {
                            Some(len) if streams.len() != len => {
                                return Err(format!(
                                    "mismatched repeated macro capture lengths for '{}'",
                                    name
                                ));
                            }
                            None => len = Some(streams.len()),
                            _ => {}
                        }
                    };
                }

                let Some(len) = len else { continue };

                for index in 0..len {
                    let expanded = expand_template_fragments(inner, captures, Some(index))?;
                    trees.extend(expanded.trees);
                    if let (Some(indent), true) = (line_indent, index + 1 < len) {
                        trees.extend(TokenStream::from_tokens(line_break_tokens(indent)).trees);
                    }
                }
            }
        }
    }
    Ok(TokenStream { trees })
}

fn expand_template_fragments_with_origins(
    fragments: &[TemplateFragment],
    captures: &CaptureSet,
    repeated_index: Option<usize>,
    expansion: ExpansionId,
    generated_source: GeneratedSourceId,
) -> Result<TokenStream, String> {
    let mut trees = Vec::new();
    for fragment in fragments {
        match fragment {
            TemplateFragment::Capture { name } => {
                if let Some(index) = repeated_index {
                    if let Some(streams) = captures.get_repeated(name) {
                        let Some(stream) = streams.get(index) else {
                            return Err(format!("missing repeated macro capture '{}'", name));
                        };
                        trees.extend(stream.trees.clone());
                        continue;
                    }
                }

                let Some(streams) = captures.get_direct(name) else {
                    return Err(format!("missing macro capture '{}'", name));
                };
                for stream in streams {
                    trees.extend(stream.trees.clone());
                }
            }
            TemplateFragment::Token(token) => {
                trees.extend(
                    TokenStream::generated_from_tokens(
                        vec![token.clone()],
                        expansion,
                        generated_source,
                    )
                    .trees,
                );
            }
            TemplateFragment::Repetition {
                fragments: inner,
                line_indent,
            } => {
                let capture_names = capture_names_in_template(inner);
                let mut len = None;

                for name in &capture_names {
                    if let Some(streams) = captures.get_repeated(name) {
                        match len {
                            Some(len) if streams.len() != len => {
                                return Err(format!(
                                    "mismatched repeated macro capture lengths for '{}'",
                                    name
                                ));
                            }
                            None => len = Some(streams.len()),
                            _ => {}
                        }
                    };
                }

                let Some(len) = len else { continue };

                for index in 0..len {
                    let expanded = expand_template_fragments_with_origins(
                        inner,
                        captures,
                        Some(index),
                        expansion,
                        generated_source,
                    )?;
                    trees.extend(expanded.trees);
                    if let (Some(indent), true) = (line_indent, index + 1 < len) {
                        trees.extend(
                            TokenStream::generated_from_tokens(
                                line_break_tokens(indent),
                                expansion,
                                generated_source,
                            )
                            .trees,
                        );
                    }
                }
            }
        }
    }
    Ok(TokenStream { trees })
}

fn capture_names_in_template(fragments: &[TemplateFragment]) -> Vec<String> {
    let mut names = Vec::new();
    for fragment in fragments {
        match fragment {
            TemplateFragment::Capture { name } => {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            TemplateFragment::Token(_) => {}
            TemplateFragment::Repetition {
                fragments: inner, ..
            } => {
                for name in capture_names_in_template(inner) {
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
        }
    }
    names
}

fn compile_matcher_fragments(fragments: &[MacroFragment]) -> Vec<MatcherFragment> {
    fragments
        .iter()
        .map(|fragment| match fragment {
            MacroFragment::Ident(name) => MatcherFragment::Capture {
                name: name.name.clone(),
                kind: CaptureKind::Ident,
            },
            MacroFragment::Expr(name) => MatcherFragment::Capture {
                name: name.name.clone(),
                kind: CaptureKind::Expr,
            },
            MacroFragment::Type(name) => MatcherFragment::Capture {
                name: name.name.clone(),
                kind: CaptureKind::Type,
            },
            MacroFragment::Token(token) => MatcherFragment::Token(token.clone()),
            MacroFragment::Repetition(inner) => {
                MatcherFragment::Repetition(compile_matcher_fragments(inner))
            }
        })
        .collect()
}

fn compile_template_fragments(fragments: &[MacroFragment]) -> Vec<TemplateFragment> {
    fragments
        .iter()
        .enumerate()
        .map(|(index, fragment)| match fragment {
            MacroFragment::Ident(name) | MacroFragment::Expr(name) | MacroFragment::Type(name) => {
                TemplateFragment::Capture {
                    name: name.name.clone(),
                }
            }
            MacroFragment::Token(token) => TemplateFragment::Token(token.clone()),
            MacroFragment::Repetition(inner) => TemplateFragment::Repetition {
                fragments: compile_template_fragments(inner),
                line_indent: whole_line_repetition_indent(fragments, index),
            },
        })
        .collect()
}

/// Returns the line's indent token when the repetition at `index` is written
/// on a single line and is the only thing on that line.
fn whole_line_repetition_indent(fragments: &[MacroFragment], index: usize) -> Option<Token> {
    let MacroFragment::Repetition(inner) = &fragments[index] else {
        return None;
    };
    if contains_line_break(inner) {
        return None;
    }

    let indent = match index.checked_sub(1).map(|previous| &fragments[previous]) {
        Some(MacroFragment::Token(
            token @ Token {
                token_type: TokenType::Indent(_),
                ..
            },
        )) => token.clone(),
        _ => return None,
    };
    let ends_line = match fragments.get(index + 1) {
        None => true,
        Some(MacroFragment::Token(token)) => {
            matches!(token.token_type, TokenType::Eol | TokenType::Eof)
        }
        Some(_) => false,
    };

    ends_line.then_some(indent)
}

fn contains_line_break(fragments: &[MacroFragment]) -> bool {
    fragments.iter().any(|fragment| match fragment {
        MacroFragment::Token(token) => matches!(token.token_type, TokenType::Eol),
        MacroFragment::Repetition(inner) => contains_line_break(inner),
        _ => false,
    })
}

fn line_break_tokens(indent: &Token) -> Vec<Token> {
    vec![
        Token {
            token_type: TokenType::Eol,
            span: indent.span.clone(),
        },
        indent.clone(),
    ]
}

#[cfg(test)]
mod tests;
