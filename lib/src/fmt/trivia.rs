use crate::ast::{IdentOrType, IdentifierPath, Module, ParseType, Path, TopLevel, TypePath};
use crate::lexer::Span;

/// Formatter-side source trivia that is not stored in the semantic AST.
///
/// This line model preserves blank/comment-only lines and same-line comment
/// suffixes anchored to the corresponding formatted code line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatTrivia<'a> {
    leading: Vec<Vec<&'a str>>,
    item_trivia: Vec<LineTrivia<'a>>,
    module_trailing: Vec<&'a str>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LineTrivia<'a> {
    leading: Vec<Vec<&'a str>>,
    suffixes: Vec<Option<LineSuffix<'a>>>,
    trailing: Vec<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineSuffix<'a> {
    indent: &'a str,
    text: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriviaLineKind {
    Trivia,
    BlockStart,
    BlockEnd,
    Code,
    CodeAfterPartialBlock,
}

impl<'a> LineTrivia<'a> {
    fn record_code_line(&mut self, code_line: usize, suffix: Option<LineSuffix<'a>>) {
        if self.leading.len() <= code_line {
            self.leading.resize_with(code_line + 1, Vec::new);
            self.suffixes.resize(code_line + 1, None);
        }
        if self.suffixes[code_line].is_none() {
            self.suffixes[code_line] = suffix;
        }
    }

    fn record_leading(
        &mut self,
        code_line: usize,
        pending: &mut Vec<&'a str>,
        suffix: Option<LineSuffix<'a>>,
    ) {
        self.record_code_line(code_line, suffix);
        self.leading[code_line].append(pending);
    }

    fn apply_to_formatted(&self, formatted: &str) -> String {
        let mut output = String::new();
        let mut code_line = 0;
        let formatted_code_lines = formatted
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();

        for segment in formatted.split_inclusive('\n') {
            let (line, has_newline) = segment
                .strip_suffix('\n')
                .map(|line| (line, true))
                .unwrap_or((segment, false));
            if !line.trim().is_empty() {
                if let Some(leading) = self.leading.get(code_line) {
                    FormatTrivia::write_trivia_lines(&mut output, leading);
                }
                output.push_str(line);
                let own_suffix = self.suffixes.get(code_line).and_then(|suffix| *suffix);
                let collapsed_suffixes = if code_line + 1 == formatted_code_lines {
                    self.suffixes
                        .iter()
                        .skip(code_line + 1)
                        .filter_map(|suffix| *suffix)
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let suffix = own_suffix.or_else(|| collapsed_suffixes.first().copied());
                if let Some(suffix) = suffix {
                    if !line.ends_with(char::is_whitespace) {
                        output.push(' ');
                    }
                    output.push_str(suffix.text.trim_start());
                }
                if has_newline {
                    output.push('\n');
                }
                let extra_start = usize::from(own_suffix.is_none() && suffix.is_some());
                for suffix in collapsed_suffixes.iter().skip(extra_start) {
                    output.push_str(suffix.indent);
                    output.push_str(suffix.text.trim_start());
                    output.push('\n');
                }
                code_line += 1;
            } else {
                output.push_str(segment);
            }
        }

        for leading in self.leading.iter().skip(code_line) {
            FormatTrivia::write_trivia_lines(&mut output, leading);
        }
        FormatTrivia::write_trivia_lines(&mut output, &self.trailing);
        output
    }
}

impl<'a> FormatTrivia<'a> {
    pub fn from_source(source: &'a str) -> Self {
        Self::from_source_top_level_lines(source, Vec::new())
    }

    pub fn from_module_source(module: &Module, source: &'a str) -> Self {
        let line_starts = Self::line_starts(source);
        let mut start_lines = module
            .top_levels
            .iter()
            .map(|top_level| {
                Self::top_level_start_span(top_level)
                    .and_then(|span| Self::span_line(span, &line_starts))
            })
            .collect::<Vec<_>>();
        Self::fill_missing_start_lines(source, &mut start_lines);

        Self::from_source_top_level_lines(source, start_lines)
    }

    pub(crate) fn write_leading(&self, index: usize, output: &mut String) {
        if let Some(lines) = self.leading.get(index) {
            Self::write_trivia_lines(output, lines);
        }
    }

    pub(crate) fn apply_to_item(&self, index: usize, formatted: &str) -> String {
        self.item_trivia
            .get(index)
            .map(|trivia| trivia.apply_to_formatted(formatted))
            .unwrap_or_else(|| formatted.to_string())
    }

    pub(crate) fn write_module_trailing(&self, output: &mut String) {
        Self::write_trivia_lines(output, &self.module_trailing);
    }

    fn from_source_top_level_lines(source: &'a str, start_lines: Vec<Option<usize>>) -> Self {
        let mut leading = vec![Vec::new(); start_lines.len()];
        let mut item_trivia = vec![LineTrivia::default(); start_lines.len()];
        let mut module_trailing = Vec::new();
        let mut pending = Vec::new();
        let mut current_top_level = None;
        let mut top_level_code_lines = vec![0; start_lines.len()];
        let mut in_block_comment = false;
        let mut top_level_by_line = vec![None; source.lines().count()];
        for (index, line) in start_lines.iter().enumerate() {
            if let Some(line) = line {
                if let Some(slot) = top_level_by_line.get_mut(*line) {
                    *slot = Some(index);
                }
            }
        }

        for (line_index, line) in source.lines().enumerate() {
            match Self::classify_standalone_trivia_line(line, &mut in_block_comment) {
                TriviaLineKind::Trivia => {
                    pending.push(Self::stored_line(line));
                    continue;
                }
                TriviaLineKind::BlockStart => {
                    pending.push(Self::stored_line(line));
                    continue;
                }
                TriviaLineKind::BlockEnd => {
                    pending.push(Self::stored_line(line));
                    continue;
                }
                TriviaLineKind::Code => {
                    if let Some(prefix) = Self::leading_closed_block_comment_prefix(line) {
                        pending.push(prefix);
                    }
                }
                TriviaLineKind::CodeAfterPartialBlock => {
                    if let Some(prefix) = Self::closing_block_comment_prefix(line) {
                        pending.push(prefix);
                    }
                }
            }

            if let Some(top_level) = top_level_by_line.get(line_index).and_then(|line| *line) {
                leading[top_level].append(&mut pending);
                current_top_level = Some(top_level);
                item_trivia[top_level].record_code_line(
                    top_level_code_lines[top_level],
                    Self::trailing_comment_suffix(line),
                );
                top_level_code_lines[top_level] += 1;
            } else if let Some(top_level) = current_top_level {
                item_trivia[top_level].record_leading(
                    top_level_code_lines[top_level],
                    &mut pending,
                    Self::trailing_comment_suffix(line),
                );
                top_level_code_lines[top_level] += 1;
            } else if !leading.is_empty() {
                leading[0].append(&mut pending);
            } else {
                module_trailing.append(&mut pending);
            }
        }

        if let Some(top_level) = current_top_level {
            item_trivia[top_level].trailing.append(&mut pending);
        } else {
            module_trailing.append(&mut pending);
        }

        Self {
            leading,
            item_trivia,
            module_trailing,
        }
    }

    fn top_level_start_span(top_level: &TopLevel) -> Option<&Span> {
        match top_level {
            TopLevel::Module(module) => module.0.name.as_ref().map(|name| &name.span),
            TopLevel::Mod(ident, _) => Some(&ident.span),
            TopLevel::Import(path) | TopLevel::Export(path) => Self::path_span(path),
            TopLevel::GlobImport(_) | TopLevel::GlobExport(_) | TopLevel::InfixOperator(_, _) => {
                None
            }
            TopLevel::Extern(sig) | TopLevel::FunctionSig(sig) => Some(&sig.name.span),
            TopLevel::FunctionDecl(decl) => Some(&decl.name.span),
            TopLevel::StructDecl(decl) => Some(&decl.name.span),
            TopLevel::TraitDecl(decl) => decl
                .language_items
                .root
                .as_ref()
                .map(|marker| &marker.span)
                .or(Some(&decl.name.span)),
            TopLevel::EnumDecl(decl) => decl
                .language_items
                .root
                .as_ref()
                .map(|marker| &marker.span)
                .or(Some(&decl.name.span)),
            TopLevel::Impl(impl_) => Some(&impl_.name.span),
            TopLevel::NewType(inner, _) => Some(&inner.span),
            TopLevel::MacroDecl(decl) => Some(&decl.name.span),
            TopLevel::MacroInvoc(invoc) => Some(&invoc.name.span),
        }
    }

    fn path_span(path: &Path) -> Option<&Span> {
        match path {
            Path::Ident(path) => Self::identifier_path_span(path),
            Path::Type(path) => Self::type_path_span(path),
        }
    }

    fn identifier_path_span(path: &IdentifierPath) -> Option<&Span> {
        path.path.first().and_then(Self::ident_or_type_span)
    }

    fn type_path_span(path: &TypePath) -> Option<&Span> {
        path.path.first().and_then(Self::ident_or_type_span)
    }

    fn ident_or_type_span(segment: &IdentOrType) -> Option<&Span> {
        match segment {
            IdentOrType::Ident(ident) => Some(&ident.span),
            IdentOrType::Type(ty) => Self::parse_type_span(ty),
        }
    }

    fn parse_type_span(ty: &ParseType) -> Option<&Span> {
        match ty {
            ParseType::Object(object) => Some(&object.span),
            ParseType::Type(inner) => Some(&inner.span),
            ParseType::Application(application) => Some(&application.span),
            ParseType::Lambda(lambda) => Some(&lambda.span),
            ParseType::Hole(hole) => Some(&hole.span),
            ParseType::Associated { base, .. } => Some(&base.span),
            ParseType::Function(types) | ParseType::Tuple(types) => {
                types.first().and_then(Self::parse_type_span)
            }
            ParseType::Slice(inner)
            | ParseType::Array { inner, .. }
            | ParseType::Reference { pointee: inner, .. }
            | ParseType::Pointer(inner) => Self::parse_type_span(inner),
            ParseType::Unit(span) => Some(span),
        }
    }

    fn span_line(span: &Span, line_starts: &[usize]) -> Option<usize> {
        if span.end <= span.start {
            return None;
        }

        match line_starts.binary_search(&span.start) {
            Ok(line) => Some(line),
            Err(0) => None,
            Err(line) => Some(line - 1),
        }
    }

    fn line_starts(source: &str) -> Vec<usize> {
        let mut starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(index + 1);
            }
        }
        starts
    }

    fn fill_missing_start_lines(source: &str, start_lines: &mut [Option<usize>]) {
        let candidates = Self::fallback_top_level_code_lines(source);
        let mut candidate_index = 0;
        let mut previous_line = None;

        for index in 0..start_lines.len() {
            if let Some(line) = start_lines[index] {
                previous_line = Some(line);
                continue;
            }

            while let Some(candidate) = candidates.get(candidate_index).copied() {
                let already_used = start_lines.iter().flatten().any(|line| *line == candidate);
                let before_previous = previous_line
                    .map(|previous| candidate <= previous)
                    .unwrap_or(false);
                if !already_used && !before_previous {
                    start_lines[index] = Some(candidate);
                    previous_line = Some(candidate);
                    candidate_index += 1;
                    break;
                }
                candidate_index += 1;
            }
        }
    }

    fn fallback_top_level_code_lines(source: &str) -> Vec<usize> {
        let mut lines = Vec::new();
        let mut in_block_comment = false;

        for (index, line) in source.lines().enumerate() {
            match Self::classify_standalone_trivia_line(line, &mut in_block_comment) {
                TriviaLineKind::Trivia | TriviaLineKind::BlockStart | TriviaLineKind::BlockEnd => {
                    continue;
                }
                TriviaLineKind::Code | TriviaLineKind::CodeAfterPartialBlock => {}
            }

            if line.chars().next().is_some_and(|ch| !ch.is_whitespace()) {
                lines.push(index);
            }
        }

        lines
    }

    fn classify_standalone_trivia_line(line: &str, in_block_comment: &mut bool) -> TriviaLineKind {
        let trimmed = line.trim();
        if *in_block_comment {
            if let Some(close) = trimmed.find("*/") {
                *in_block_comment = false;
                return if Self::is_comment_only_remainder(&trimmed[close + 2..]) {
                    TriviaLineKind::BlockEnd
                } else {
                    TriviaLineKind::CodeAfterPartialBlock
                };
            }

            return TriviaLineKind::Trivia;
        }

        if trimmed.is_empty() || trimmed.starts_with("//") {
            return TriviaLineKind::Trivia;
        }

        if let Some(rest) = trimmed.strip_prefix("/*") {
            if let Some(close) = rest.find("*/") {
                return if Self::is_comment_only_remainder(&rest[close + 2..]) {
                    TriviaLineKind::Trivia
                } else {
                    TriviaLineKind::Code
                };
            }

            *in_block_comment = true;
            return TriviaLineKind::BlockStart;
        }

        TriviaLineKind::Code
    }

    fn is_comment_only_remainder(mut remainder: &str) -> bool {
        loop {
            let trimmed = remainder.trim();
            if trimmed.is_empty() || trimmed.starts_with("//") {
                return true;
            }

            let Some(rest) = trimmed.strip_prefix("/*") else {
                return false;
            };
            let Some(close) = rest.find("*/") else {
                return false;
            };

            remainder = &rest[close + 2..];
        }
    }

    fn trailing_comment_suffix(line: &'a str) -> Option<LineSuffix<'a>> {
        let mut index = 0;
        let mut saw_code = false;
        let indent_end = line
            .char_indices()
            .find_map(|(index, ch)| (!ch.is_whitespace()).then_some(index))
            .unwrap_or(line.len());
        let indent = &line[..indent_end];

        while index < line.len() {
            let rest = &line[index..];
            if rest.starts_with("//") {
                return saw_code.then_some(LineSuffix { indent, text: rest });
            }

            if rest.starts_with("/*") {
                if saw_code && Self::is_comment_only_remainder(rest) {
                    return Some(LineSuffix { indent, text: rest });
                }

                let Some(close) = rest[2..].find("*/") else {
                    return None;
                };
                index += 2 + close + 2;
                continue;
            }

            let ch = rest
                .chars()
                .next()
                .expect("index should be on a char boundary");
            if ch == '"' || ch == '\'' {
                index = Self::skip_quoted(line, index, ch);
                saw_code = true;
                continue;
            }

            if !ch.is_whitespace() {
                saw_code = true;
            }
            index += ch.len_utf8();
        }

        None
    }

    fn leading_closed_block_comment_prefix(line: &'a str) -> Option<&'a str> {
        let trimmed_start = line.trim_start();
        let indent_len = line.len() - trimmed_start.len();
        let rest = trimmed_start.strip_prefix("/*")?;
        let close = rest.find("*/")?;
        let end = indent_len + 2 + close + 2;

        Self::has_code_after_comment(&line[end..]).then(|| line[..end].trim_end())
    }

    fn closing_block_comment_prefix(line: &'a str) -> Option<&'a str> {
        let close = line.find("*/")? + 2;
        Self::has_code_after_comment(&line[close..]).then(|| line[..close].trim_end())
    }

    fn has_code_after_comment(remainder: &str) -> bool {
        let trimmed = remainder.trim();
        !trimmed.is_empty() && !Self::is_comment_only_remainder(remainder)
    }

    fn skip_quoted(line: &str, start: usize, quote: char) -> usize {
        let mut escaped = false;
        let mut chars = line[start + quote.len_utf8()..].char_indices();
        while let Some((offset, ch)) = chars.next() {
            let absolute = start + quote.len_utf8() + offset;
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                return absolute + ch.len_utf8();
            }
        }
        line.len()
    }

    fn stored_line(line: &'a str) -> &'a str {
        if line.trim().is_empty() {
            ""
        } else {
            line
        }
    }

    fn write_trivia_lines(output: &mut String, lines: &[&str]) {
        for line in lines {
            output.push_str(line);
            output.push('\n');
        }
    }
}
