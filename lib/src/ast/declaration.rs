use std::ops::Range;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::lexer::lossless::{LosslessError, LosslessStream, ScanLimits};

/// An immutable declaration range in the parser's original lossless input.
/// All declarations from one parse share backing; cloning an AST never prints
/// syntax or duplicates its source. Materialize only when a consumer needs it.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct DeclarationInput {
    source: Arc<LosslessStream>,
    range: Range<usize>,
    header_indent: usize,
}

impl DeclarationInput {
    pub(crate) fn new(source: Arc<LosslessStream>, range: Range<usize>, header_indent: usize) -> Self {
        Self { source, range, header_indent }
    }

    /// Logical offsets in the original input, not guessed filesystem spans.
    pub fn range(&self) -> Range<usize> { self.range.clone() }

    pub fn header_indent(&self) -> usize { self.header_indent }

    pub fn stream(&self) -> Result<LosslessStream, LosslessError> {
        self.source.slice(self.range.clone(), ScanLimits {
            max_source_bytes: 16 * 1024 * 1024, max_runs: 1024 * 1024,
        })
    }
}

impl std::fmt::Debug for DeclarationInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // AST debug output should not repeat the entire source for every item.
        f.debug_struct("DeclarationInput").field("range", &self.range)
            .field("header_indent", &self.header_indent).finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ast::{Module, NamedFieldsOrTypesList, ParseType, TopLevel};
    use crate::lexer::lossless::{JoinBoundary, SyntaxContext};
    use crate::macro_expansion::{ExpansionId, GeneratedSourceId, TokenProvenance};
    use crate::{parser, Config};

    const LIMITS: ScanLimits = parser::SOURCE_LIMITS;

    fn source(text: &str) -> LosslessStream {
        LosslessStream::from_bytes("/virtual/declarations.rk".into(), text.as_bytes().to_vec(), LIMITS).unwrap()
    }

    fn parse(text: &str) -> Module {
        parser::parse_source("/virtual/declarations.rk".into(), text, &Config::default()).unwrap()
    }

    fn spelling(stream: &LosslessStream) -> String {
        stream.runs().iter().map(|run| run.text()).collect()
    }

    #[test]
    fn macro_c_01_struct_retains_visibility_kinds_defaults_and_exact_trivia() {
        let declaration = "< struct Packet (F: Type -> Type), T, {U = T}\r\n    // field comment 雪\r\n    < value: F U\r\n    count: I64 = 7 /* exact default */\r\n\r\n";
        let module = parse(&format!("{declaration}next = -> 0\r\n"));
        let TopLevel::StructDecl(node) = &module.top_levels[0] else { panic!("expected struct") };
        let original = node.original.as_ref().unwrap();
        assert!(node.exported && node.fields[0].public && node.fields[1].default.is_some());
        assert_eq!(node.generic_params.len(), 3);
        assert!(node.generic_params[0].kind.is_some() && node.generic_params[2].default.is_some());
        assert_eq!(original.range(), 0..declaration.len());
        assert_eq!(original.header_indent(), 0);
        assert_eq!(spelling(&original.stream().unwrap()), declaration);
        assert_eq!(module.top_levels.len(), 2, "the next declaration is not captured");
    }

    #[test]
    fn macro_c_01_enum_shapes_and_multiple_consumers_share_immutable_input() {
        let declaration = "< enum Choice T, {U = T}\n    Empty // nullary\n    Pair T, U\n    Named\n        < value: U\n\n";
        let mut module = parse(&format!("{declaration}struct Following\n"));
        let TopLevel::EnumDecl(node) = &module.top_levels[0] else { panic!("expected enum") };
        assert!(node.exported);
        assert_eq!(node.variants.iter().map(|variant| variant.name.name.as_str()).collect::<Vec<_>>(), ["Empty", "Pair", "Named"]);
        assert!(matches!(&node.variants[1].fields, NamedFieldsOrTypesList::TypesList(fields) if fields.len() == 2));
        assert!(matches!(&node.variants[2].fields, NamedFieldsOrTypesList::NamedFields(fields) if fields.len() == 1 && fields[0].public));
        let first = node.original.as_ref().unwrap().clone();
        let second = first.clone();
        let TopLevel::StructDecl(following) = &module.top_levels[1] else { panic!("expected following struct") };
        assert!(Arc::ptr_eq(&first.source, &second.source));
        assert!(Arc::ptr_eq(&first.source, &following.original.as_ref().unwrap().source));
        let TopLevel::EnumDecl(node) = &mut module.top_levels[0] else { unreachable!() };
        node.name.name = "Changed".into();
        node.variants.clear();
        drop(module);
        assert_eq!(spelling(&first.stream().unwrap()), declaration);
        assert_eq!(second.stream().unwrap(), first.stream().unwrap());
    }

    #[test]
    fn macro_c_01_nested_declarations_retain_their_physical_margin() {
        let text = "mod nested\n    < struct Empty // header\n\n    enum Flag\n        On\n        Off\n    next = -> 0\n";
        let module = parse(text);
        let TopLevel::Module(nested) = &module.top_levels[0] else { panic!("expected inline module") };
        let TopLevel::StructDecl(node) = &nested.0.top_levels[0] else { panic!("expected nested struct") };
        let original = node.original.as_ref().unwrap();
        assert_eq!(original.header_indent(), 4);
        assert_eq!(spelling(&original.stream().unwrap()), "    < struct Empty // header\n\n");
        let TopLevel::EnumDecl(node) = &nested.0.top_levels[1] else { panic!("expected nested enum") };
        let original = node.original.as_ref().unwrap();
        assert_eq!(original.header_indent(), 4);
        assert_eq!(spelling(&original.stream().unwrap()), "    enum Flag\n        On\n        Off\n");
    }

    #[test]
    fn macro_c_01_eof_and_bounded_source_keep_bytes_not_synthetic_layout() {
        for text in ["struct Empty", "struct Empty // é", "enum Empty\r\n", "enum Flag\n    On"] {
            let module = parse(text);
            let original = match &module.top_levels[0] {
                TopLevel::StructDecl(node) => node.original.as_ref().unwrap(),
                TopLevel::EnumDecl(node) => node.original.as_ref().unwrap(),
                _ => panic!("expected declaration"),
            };
            assert_eq!(original.range(), 0..text.len());
            assert_eq!(spelling(&original.stream().unwrap()), text);
        }
        let text = "< struct Empty\n";
        let backing = source(&format!("prefix\n{text}suffix"));
        let bounded = backing.slice(7..7 + text.len(), LIMITS).unwrap();
        let module = parser::parse_lossless_source(&bounded, &Config::default(), None).unwrap();
        let TopLevel::StructDecl(node) = &module.top_levels[0] else { panic!("expected struct") };
        let original = node.original.as_ref().unwrap();
        assert_eq!(original.range(), 0..text.len(), "ranges are logical, not source spans");
        assert_eq!(original.stream().unwrap().start_span().start, 7);
        assert_eq!(spelling(&original.stream().unwrap()), text);
    }

    #[test]
    fn macro_c_01_generated_declaration_keeps_mixed_contexts_and_origins() {
        let expansion = ExpansionId(7);
        let head = source("< struct Packet\n    < value: ").map_contexts(|_| Ok(SyntaxContext::allocated(6))).unwrap()
            .generated(expansion, GeneratedSourceId(1));
        let field = source("I64").map_contexts(|_| Ok(SyntaxContext::allocated(9))).unwrap()
            .captured(expansion, &crate::lexer::Span::new("/caller.rk".into(), 10, 20));
        let tail = source("\n").generated(expansion, GeneratedSourceId(2));
        let stream = head.join(&field, JoinBoundary::Separate, LIMITS).unwrap()
            .join(&tail, JoinBoundary::Separate, LIMITS).unwrap();
        let stream = stream.project_output(expansion, GeneratedSourceId(3), 99);
        let module = parser::parse_lossless_source(&stream, &Config::default(), Some(expansion)).unwrap();
        let TopLevel::StructDecl(node) = &module.top_levels[0] else { panic!("expected generated struct") };
        assert_eq!(node.name.context, SyntaxContext::allocated(6));
        assert!(matches!(&node.fields[0].ty, ParseType::Type(name) if name.context == SyntaxContext::allocated(9)));
        let retained = node.original.as_ref().unwrap().stream().unwrap();
        assert_eq!(retained.runs(), stream.runs());
        assert_eq!(retained.start_span(), stream.start_span());
        assert_eq!(retained.end_span(), stream.end_span());
        assert!(retained.runs().iter().any(|run| matches!(run.origin().provenance, TokenProvenance::Generated { .. })));
        assert!(retained.runs().iter().any(|run| matches!(run.origin().provenance, TokenProvenance::Captured { .. })));
    }

    #[test]
    fn macro_c_01_default_capture_and_serialization_preserve_original_stream() {
        let head = source("struct Setting\n    value: I64 = ");
        let expression = source("answer + 1").expression_fragment(4, LIMITS).unwrap();
        let stream = head.join(&expression, JoinBoundary::Separate, LIMITS).unwrap()
            .join(&source("\n"), JoinBoundary::Separate, LIMITS).unwrap();
        let module = parser::parse_lossless_source(&stream, &Config::default(), None).unwrap();
        let TopLevel::StructDecl(node) = &module.top_levels[0] else { panic!("expected struct") };
        let original = node.original.as_ref().unwrap();
        let retained = original.stream().unwrap();
        assert_eq!(retained, stream);
        assert_eq!(retained.expression_end_at(head.len_bytes()), Some(head.len_bytes() + expression.len_bytes()));
        let bytes = bincode::serialize(original).unwrap();
        let restored: DeclarationInput = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.range(), original.range());
        assert_eq!(bincode::serialize(&restored.stream().unwrap()).unwrap(), bincode::serialize(&retained).unwrap());
        let invalid = DeclarationInput::new(Arc::clone(&original.source), 0..usize::MAX, 0);
        assert!(matches!(invalid.stream(), Err(LosslessError::InvalidRange { .. })));
    }
}
