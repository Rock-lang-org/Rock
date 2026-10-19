use std::collections::HashMap;

use crate::ast::{MacroDecl, MacroFragment, MacroLexeme};
use crate::lexer::TokenType;
use crate::macro_expansion::{ExpansionId, GeneratedSourceId, TokenStream};
use super::token_tree::{append, MACRO_LIMITS};

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
    pub input: TokenStream,
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
    Token(MacroLexeme),
    Repetition(Vec<MatcherFragment>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateFragment {
    Capture { name: String },
    Input(TokenStream),
    Repetition(Vec<TemplateFragment>),
}

#[derive(Debug, Clone, Default)]
pub struct CaptureSet {
    direct: HashMap<String, Vec<TokenStream>>,
    repeated: HashMap<String, Vec<TokenStream>>,
}

impl CaptureSet {
    pub fn new() -> Self { Self::default() }

    pub fn insert_direct(&mut self, name: String, stream: TokenStream) {
        let streams = self.direct.entry(name).or_default();
        if !streams.contains(&stream) { streams.push(stream); }
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
            arms: decl.entries.iter().map(|entry| DeclarativeMacroArm {
                matcher: MacroMatcher { fragments: compile_matcher_fragments(&entry.defs) },
                template: MacroTemplate {
                    input: entry.input.clone(),
                    fragments: compile_template_fragments(&entry.body),
                },
            }).collect(),
        }
    }
}

impl MacroTemplate {
    #[cfg(test)]
    pub fn expand(&self, captures: &CaptureSet) -> Result<TokenStream, String> {
        self.expand_fragments(&self.fragments, captures, None, None)
    }

    pub fn expand_with_origins(&self, captures: &CaptureSet, expansion: ExpansionId, generated_source: GeneratedSourceId)
        -> Result<TokenStream, String>
    {
        self.expand_fragments(&self.fragments, captures, None, Some((expansion, generated_source)))
    }

    fn expand_fragments(&self, fragments: &[TemplateFragment], captures: &CaptureSet, repeated_index: Option<usize>,
        origin: Option<(ExpansionId, GeneratedSourceId)>) -> Result<TokenStream, String>
    {
        let mut output = self.input.slice(0..0, MACRO_LIMITS).map_err(|error| format!("invalid template anchor: {error:?}"))?;
        for fragment in fragments {
            match fragment {
                TemplateFragment::Capture { name } => {
                    if let Some(index) = repeated_index {
                        if let Some(streams) = captures.get_repeated(name) {
                            let stream = streams.get(index).ok_or_else(|| format!("missing repeated macro capture '{name}'"))?;
                            append(&mut output, stream)?;
                            continue;
                        }
                    }
                    let streams = captures.get_direct(name).ok_or_else(|| format!("missing macro capture '{name}'"))?;
                    for stream in streams { append(&mut output, stream)?; }
                }
                TemplateFragment::Input(input) => {
                    let input = match origin {
                        Some((expansion, generated_source)) => input.generated(expansion, generated_source),
                        None => input.clone(),
                    };
                    append(&mut output, &input)?;
                }
                TemplateFragment::Repetition(inner) => {
                    let mut len = None;
                    for name in capture_names_in_template(inner) {
                        if let Some(streams) = captures.get_repeated(&name) {
                            match len {
                                Some(len) if streams.len() != len => return Err(format!("mismatched repeated macro capture lengths for '{name}'")),
                                None => len = Some(streams.len()),
                                _ => {}
                            }
                        }
                    }
                    if let Some(len) = len {
                        for index in 0..len {
                            let expanded = self.expand_fragments(inner, captures, Some(index), origin)?;
                            append(&mut output, &expanded)?;
                        }
                    }
                }
            }
        }
        Ok(output)
    }
}

fn capture_names_in_template(fragments: &[TemplateFragment]) -> Vec<String> {
    let mut names = Vec::new();
    for fragment in fragments {
        match fragment {
            TemplateFragment::Capture { name } => if !names.contains(name) { names.push(name.clone()); },
            TemplateFragment::Input(_) => {}
            TemplateFragment::Repetition(inner) => for name in capture_names_in_template(inner) {
                if !names.contains(&name) { names.push(name); }
            },
        }
    }
    names
}

fn compile_matcher_fragments(fragments: &[MacroFragment]) -> Vec<MatcherFragment> {
    fragments.iter().filter_map(|fragment| Some(match fragment {
        MacroFragment::Ident(name) => MatcherFragment::Capture { name: name.name.clone(), kind: CaptureKind::Ident },
        MacroFragment::Expr(name) => MatcherFragment::Capture { name: name.name.clone(), kind: CaptureKind::Expr },
        MacroFragment::Type(name) => MatcherFragment::Capture { name: name.name.clone(), kind: CaptureKind::Type },
        MacroFragment::Token(token) => MatcherFragment::Token(token.clone()),
        MacroFragment::Trivia(_) => return None,
        MacroFragment::Repetition(inner) => MatcherFragment::Repetition(compile_matcher_fragments(&trim_repetition_close(inner))),
    })).collect()
}

fn compile_template_fragments(fragments: &[MacroFragment]) -> Vec<TemplateFragment> {
    fragments.iter().map(|fragment| match fragment {
        MacroFragment::Ident(name) | MacroFragment::Expr(name) | MacroFragment::Type(name) => TemplateFragment::Capture { name: name.name.clone() },
        MacroFragment::Token(token) => TemplateFragment::Input(token.input.clone()),
        MacroFragment::Trivia(input) => TemplateFragment::Input(input.clone()),
        MacroFragment::Repetition(inner) => TemplateFragment::Repetition(compile_template_fragments(&trim_repetition_close(inner))),
    }).collect()
}

fn trim_repetition_close(fragments: &[MacroFragment]) -> Vec<MacroFragment> {
    let mut fragments = fragments.to_vec();
    let close = fragments.iter().rposition(|fragment| match fragment {
        MacroFragment::Token(token) => !matches!(token.token_type, TokenType::Eol | TokenType::Indent(_)),
        MacroFragment::Trivia(_) => false,
        _ => true,
    });
    if let Some(index) = close {
        if matches!(&fragments[index], MacroFragment::Token(token) if token.token_type == TokenType::CloseParen) {
            fragments.remove(index);
        }
    }
    fragments
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::token_tree::source;
    use crate::lexer::interpretation::layout::LayoutView;
    use crate::macro_expansion::TokenOrigin;

    fn template(fragments: Vec<TemplateFragment>) -> MacroTemplate {
        MacroTemplate { input: source(""), fragments }
    }

    fn capture(name: &str) -> TemplateFragment { TemplateFragment::Capture { name: name.into() } }
    fn input(text: &str) -> TemplateFragment { TemplateFragment::Input(source(text)) }
    fn tokens(input: &TokenStream) -> Vec<TokenType> {
        let mut view = LayoutView::new(input, false);
        let mut tokens = Vec::new();
        while let Some(token) = view.next_token().unwrap() {
            if !matches!(token.token.token_type, TokenType::Indent(_) | TokenType::Eol | TokenType::Eof) {
                tokens.push(token.token.token_type);
            }
        }
        tokens
    }

    #[test]
    fn declarative_macro_compiles_capture_kinds_and_template_tokens() {
        let program = crate::parser::parse_string("macro make\n    $name:ident =>\n        $name = 0", &crate::Config::default()).unwrap();
        let crate::ast::TopLevel::MacroDecl(decl) = &program.module.top_levels[0] else { panic!("expected declaration") };
        let compiled = DeclarativeMacro::from_ast(decl);
        assert_eq!(compiled.name, "make");
        assert_eq!(compiled.arms.len(), 1);
        assert!(matches!(compiled.arms[0].matcher.fragments[0], MatcherFragment::Capture { kind: CaptureKind::Ident, .. }));
        assert!(compiled.arms[0].template.fragments.iter().any(|fragment| matches!(fragment, TemplateFragment::Capture { .. })));
    }

    #[test]
    fn declarative_template_expands_direct_capture_tokens() {
        let mut captures = CaptureSet::new();
        captures.insert_direct("name".into(), source("main"));
        let expanded = template(vec![capture("name"), input("=")]).expand(&captures).unwrap();
        assert_eq!(tokens(&expanded), vec![TokenType::Ident("main".into()), TokenType::Equal]);
    }

    #[test]
    fn declarative_template_marks_captured_and_generated_origins() {
        let expansion = ExpansionId(9);
        let generated_source = GeneratedSourceId(4);
        let invocation = crate::lexer::Span::new("call.rk".into(), 1, 7);
        let captured = source("prefix  main");
        let captured = captured.slice(8..12, MACRO_LIMITS).unwrap();
        let definition = source("                    =");
        let definition = definition.slice(20..21, MACRO_LIMITS).unwrap();
        let mut captures = CaptureSet::new();
        captures.insert_direct("name".into(), captured.captured(expansion, &invocation));
        let expanded = template(vec![capture("name"), TemplateFragment::Input(definition)])
            .expand_with_origins(&captures, expansion, generated_source).unwrap();
        assert!(matches!(expanded.runs()[0].origin(), TokenOrigin::Captured { expansion: id, capture_span, .. } if id == expansion && capture_span.start == 8 && capture_span.end == 12));
        assert!(matches!(expanded.runs()[1].origin(), TokenOrigin::Generated { expansion: id, generated_source: source, definition_span } if id == expansion && source == generated_source && definition_span.start == 20 && definition_span.end == 21));
    }

    #[test]
    fn declarative_template_expands_repeated_capture_groups() {
        let mut captures = CaptureSet::new();
        captures.insert_repeated("name".into(), vec![source("one"), source("two")]);
        let expanded = template(vec![TemplateFragment::Repetition(vec![capture("name"), input("=")])]).expand(&captures).unwrap();
        assert_eq!(tokens(&expanded), vec![TokenType::Ident("one".into()), TokenType::Equal, TokenType::Ident("two".into()), TokenType::Equal]);
    }

    #[test]
    fn declarative_template_repeats_direct_capture_with_repeated_capture_groups() {
        let mut captures = CaptureSet::new();
        captures.insert_direct("ret".into(), source("value"));
        captures.insert_repeated("name".into(), vec![source("one"), source("two")]);
        let expanded = template(vec![TemplateFragment::Repetition(vec![capture("name"), input("= ->"), capture("ret")])]).expand(&captures).unwrap();
        assert_eq!(tokens(&expanded), vec![TokenType::Ident("one".into()), TokenType::Equal, TokenType::Arrow, TokenType::Ident("value".into()), TokenType::Ident("two".into()), TokenType::Equal, TokenType::Arrow, TokenType::Ident("value".into())]);
    }
}
