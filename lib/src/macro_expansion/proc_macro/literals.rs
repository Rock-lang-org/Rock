//! Prepare views from parser-recognized ranges; never interpret opaque DSL quotes.

use crate::ast::MacroInvoc;
use crate::lexer::literal::{decode_char, decoded_units};

use super::{LiteralMapping, ProcMacroArguments, ProcMacroLiteral, ProcMacroViews};

impl ProcMacroViews {
    pub(crate) fn for_invocation(invocation: &MacroInvoc) -> Result<Self, String> {
        let mut views = Self {
            arguments: (invocation.block.is_none() && (!invocation.arguments.is_empty() || invocation.args.len_bytes() == 0))
                .then(|| ProcMacroArguments { ranges: invocation.arguments.clone() }),
            ..Default::default()
        };
        // Even a valid-looking quoted substring in a block remains DSL text.
        if invocation.block.is_some() { return Ok(views); }
        if invocation.literal_ranges.len() > super::MAX_METADATA_ENTRIES {
            return Err("too many recognized literal views".into());
        }
        for range in &invocation.literal_ranges {
            let literal = invocation.args.slice(range.clone(), super::super::token_tree::MACRO_LIMITS)
                .map_err(|error| format!("invalid recognized literal range: {error:?}"))?;
            let spelling: String = literal.runs().iter().map(|run| run.text()).collect();
            let quote = spelling.as_bytes().first().copied().ok_or("empty recognized literal")?;
            if !matches!(quote, b'\'' | b'"') || spelling.len() < 2 || spelling.as_bytes().last() != Some(&quote) {
                return Err("invalid recognized literal spelling".into());
            }
            let body = &spelling[1..spelling.len() - 1];
            if quote == b'\'' && decode_char(body).is_none() {
                return Err("character literal must contain exactly one Unicode scalar or supported escape".into());
            }
            let mut decoded = String::with_capacity(body.len());
            let mut mapping = Vec::new();
            for (character, source) in decoded_units(body) {
                if mapping.len() == super::MAX_METADATA_ENTRIES {
                    return Err("literal source mapping limit exceeded".into());
                }
                let start = decoded.len();
                decoded.push(character);
                mapping.push(LiteralMapping {
                    decoded: start..decoded.len(),
                    source: range.start + 1 + source.start..range.start + 1 + source.end,
                });
            }
            views.literals.push(ProcMacroLiteral { source: range.clone(), decoded, mapping });
        }
        Ok(views)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Expression, Operand, Statement, TopLevel, UnaryExpr};
    use crate::parser::parse_string;
    use crate::Config;

    fn invocation(source: &str) -> MacroInvoc {
        let program = parse_string(source, &Config::default()).unwrap();
        match &program.module.top_levels[0] {
            TopLevel::MacroInvoc(invocation) => invocation.clone(),
            TopLevel::FunctionDecl(function) => {
                let Statement::Expression(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary))) = &function.lambda.body.statements[0] else { panic!("expected expression") };
                let Operand::MacroInvoc(invocation) = &primary.operand else { panic!("expected macro") };
                *invocation.clone()
            }
            _ => panic!("expected macro invocation"),
        }
    }

    #[test]
    fn macro_b_18_prepares_only_recognized_literals_with_precise_subranges() {
        let invocation = invocation("main = -> %inspect \"é\\n雪\\q\", ('🦀'), \"\", (f \"\\\"\")\n");
        let views = ProcMacroViews::for_invocation(&invocation).unwrap();
        assert_eq!(views.literals.iter().map(|view| view.decoded.as_str()).collect::<Vec<_>>(), ["é\n雪\\q", "🦀", "", "\""]);
        let view = &views.literals[0];
        assert_eq!(view.source, 1..12);
        assert_eq!(view.mapping, vec![
            LiteralMapping { decoded: 0..2, source: 2..4 },
            LiteralMapping { decoded: 2..3, source: 4..6 },
            LiteralMapping { decoded: 3..6, source: 6..9 },
            LiteralMapping { decoded: 6..7, source: 9..10 },
            LiteralMapping { decoded: 7..8, source: 10..11 },
        ]);
        assert!(views.literals[2].mapping.is_empty());
        let text: String = invocation.args.runs().iter().map(|run| run.text()).collect();
        for view in &views.literals {
            for mapping in &view.mapping {
                assert_eq!(&view.decoded[mapping.decoded.clone()], crate::lexer::literal::decode_string(&text[mapping.source.clone()]));
            }
        }
        // A subrange after an escape and a multibyte scalar maps to exact input bytes.
        assert_eq!(&text[view.mapping[2].source.clone()], "雪");
    }

    #[test]
    fn macro_b_18_opaque_blocks_never_request_literal_interpretation() {
        for source in [
            "%inspect\n    \"valid-looking\" '🦀'\n    \"unterminated <>{# arbitrary\n",
            "main = -> %inspect\n    \"valid-looking\" '🦀'\n    \"unterminated <>{# arbitrary\n",
        ] {
            let invocation = invocation(source);
            assert!(invocation.literal_ranges.is_empty());
            let views = ProcMacroViews::for_invocation(&invocation).unwrap();
            assert!(views.literals.is_empty());
            assert!(views.arguments.is_none());
        }
        let inline = invocation("%inspect \"é\\n\"\n");
        assert_eq!(ProcMacroViews::for_invocation(&inline).unwrap().literals[0].decoded, "é\n");
    }

    #[test]
    fn macro_b_18_view_preparation_checks_metadata_budgets_and_character_values() {
        let invalid = invocation("main = -> %inspect 'ab'\n");
        assert!(ProcMacroViews::for_invocation(&invalid).unwrap_err().contains("exactly one Unicode scalar"));
        let large = invocation(&format!("main = -> %inspect \"{}\"\n", "x".repeat(super::super::MAX_METADATA_ENTRIES + 1)));
        assert!(ProcMacroViews::for_invocation(&large).unwrap_err().contains("mapping limit"));
        let mut many = invocation("main = -> %inspect \"x\"\n");
        many.literal_ranges = vec![many.literal_ranges[0].clone(); super::super::MAX_METADATA_ENTRIES + 1];
        assert!(ProcMacroViews::for_invocation(&many).unwrap_err().contains("too many recognized literal views"));
        many.literal_ranges.truncate(1);
        many.literal_ranges[0] = 0..usize::MAX;
        assert!(ProcMacroViews::for_invocation(&many).is_err());
    }
}
