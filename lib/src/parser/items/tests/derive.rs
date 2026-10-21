use crate::ast::{DeriveTarget, IdentOrType, TopLevel};
use crate::macro_expansion::proc_macro::ProcMacroKind;
use crate::parser::{parse_string, ParseError};
use crate::Config;

#[test]
fn macro_c_02_lists_comments_and_consecutive_directives_attach_without_polluting_original() {
    let target = "// target comment\r\n< struct Packet T, {U = T}\r\n    < value: U\r\n";
    let source = format!("%derive pretty::Show, Eq // first list\r\n\r\n// between directives\r\n%derive Clone,\r\n{target}next = -> 0\r\n");
    let program = parse_string(&source, &Config::default()).unwrap();
    let TopLevel::DerivedDecl(node) = &program.module.top_levels[0] else { panic!("attached struct") };
    assert_eq!(node.derives.iter().map(|attachment| crate::collect::path_names(&attachment.path.path).join("::")).collect::<Vec<_>>(), ["pretty::Show", "Eq", "Clone"]);
    assert_ne!(node.derives[0].id, node.derives[1].id);
    let original = node.target.original().unwrap();
    let stream = original.stream().unwrap();
    assert_eq!(stream.runs().iter().map(|run| run.text()).collect::<String>(), target);
    assert!(matches!(&node.target, DeriveTarget::Struct(target) if target.exported && target.fields[0].public));
    assert_eq!(program.module.top_levels.len(), 2);
    let cloned = node.clone();
    assert_eq!(cloned.id, node.id);
    assert_eq!(cloned.derives, node.derives);
}

#[test]
fn macro_c_02_nested_enum_attachment_is_not_an_opaque_macro_block() {
    let source = "mod nested\n    %derive Show\n    enum Choice\n        Empty\n        Value I64\n";
    let program = parse_string(source, &Config::default()).unwrap();
    let TopLevel::Module(module) = &program.module.top_levels[0] else { panic!("module") };
    let TopLevel::DerivedDecl(node) = &module.0.top_levels[0] else { panic!("attached enum") };
    assert!(matches!(&node.target, DeriveTarget::Enum(target) if target.variants.len() == 2));
    assert_eq!(node.target.original().unwrap().header_indent(), 4);
    assert_eq!(node.derives.len(), 1);
}

#[test]
fn macro_c_02_invalid_lists_targets_and_attribute_spellings_diagnose() {
    for source in [
        "%derive\nstruct S\n", "%derive!\nstruct S\n", "%derive , Show\nstruct S\n",
        "%derive Show Eq\nstruct S\n", "%derive p::\nstruct S\n", "%derive Show,, Eq\nstruct S\n",
        "%derive Show, Show\nstruct S\n", "%derive Show\n%derive Show\nstruct S\n",
        "%derive Show\n", "%derive Show\nfunction = -> 0\nstruct S\n",
        "%derive Show\n> other::Show\nstruct S\n", "%derive Show\ntype S = I64\n",
        "%derive Show\ntrait S\n", "%derive Show\nmod other\nstruct S\n",
        "%derive Show\n%m!\nstruct S\n", "%derive Show\n    struct S\n",
        "mod nested\n    %derive Show\nstruct S\n", "mod nested\n    %derive Show\n    %derive Eq\nstruct S\n",
        "%derive\n    unmatched \"\n", "main = -> %derive Show\n    unmatched \"\n",
    ] {
        let error = parse_string(source, &Config::default()).unwrap_err();
        assert!(format!("{error:?}").contains("derive") || matches!(error.clone().without_source_context(), ParseError::UnexpectedIndent(_, _)), "{source:?}: {error:?}");
    }
    for source in ["#[derive(Show)]\nstruct S\n", "@derive Show\nstruct S\n", "@other\nstruct S\n"] {
        assert!(parse_string(source, &Config::default()).is_err(), "{source}");
    }
}

#[test]
fn macro_c_02_derive_declarations_have_explicit_kinds_and_preserve_ordinary_proc_syntax() {
    let program = parse_string("macro hidden = context, input -> input\n< macro derive Show = context, input -> input\nmacro derive serialize = context, input -> input\n< serialize\n", &Config::default()).unwrap();
    for (index, name, kind, exported) in [(0, "hidden", ProcMacroKind::FunctionLike, false), (1, "Show", ProcMacroKind::DeriveLike, true), (2, "serialize", ProcMacroKind::DeriveLike, false)] {
        let TopLevel::ProcMacroDecl(node) = &program.module.top_levels[index] else { panic!("proc declaration") };
        assert_eq!(node.function.name.name, name);
        assert_eq!(node.kind, kind);
        assert_eq!(node.function.exported, exported);
        assert_eq!(node.function.lambda.parameters.len(), 2);
    }
    for source in ["macro derive = context, input -> input\n", "macro derive Show\n    =>\n        1\n"] {
        assert!(parse_string(source, &Config::default()).is_err());
    }
    let program = parse_string("main = -> %provider::Show!\n", &Config::default()).unwrap();
    assert!(program.module.top_levels[0].get_ident().is_some());
}

#[test]
fn macro_c_02_generated_attachments_preserve_context_parent_and_serialized_identity() {
    use crate::lexer::lossless::SyntaxContext;
    use crate::macro_expansion::ExpansionId;
    let stream = crate::macro_expansion::token_tree::source("%derive tools::Show\n< struct S\n")
        .map_contexts(|_| Ok(SyntaxContext::allocated(9))).unwrap();
    let module = crate::parser::parse_lossless_source(&stream, &Config::default(), Some(ExpansionId(7))).unwrap();
    let TopLevel::DerivedDecl(node) = &module.top_levels[0] else { panic!("derived") };
    assert_eq!(node.derives[0].parent, Some(ExpansionId(7)));
    assert!(node.derives[0].path.path.iter().all(|part| matches!(part, IdentOrType::Ident(name) if name.context == SyntaxContext::allocated(9))));
    assert_eq!(node.target.name().context, SyntaxContext::allocated(9));
    let bytes = bincode::serialize(node).unwrap();
    let restored: crate::ast::DerivedDecl = bincode::deserialize(&bytes).unwrap();
    assert_eq!(restored.id, node.id);
    assert_eq!(restored.derives, node.derives);
    assert_eq!(restored.target.original().unwrap().range(), node.target.original().unwrap().range());
    let program = parse_string("%payload\n    #[derive(Show)] arbitrary \"\nnext = -> 0\n", &Config::default()).unwrap();
    assert!(matches!(&program.module.top_levels[0], TopLevel::MacroInvoc(input) if input.block.is_some()));
}

#[test]
fn macro_c_02_reserved_expression_derive_fails_before_reading_its_indented_payload() {
    let error = parse_string("main = -> %derive\n    unterminated \"\n", &Config::default()).unwrap_err();
    assert!(matches!(error, ParseError::HardError(message, _) if message.contains("derive")));
}

#[test]
fn macro_c_02_requested_lexical_failures_are_not_rewritten_as_list_errors() {
    for source in ["%derive \"unterminated\n", "%derive p::\"unterminated\n",
        "macro derive \"unterminated\n", "macro derive Show \"unterminated\n",
        "< macro derive \"unterminated\n", "< macro derive Show \"unterminated\n"] {
        let error = parse_string(source, &Config::default()).unwrap_err().without_source_context();
        assert!(matches!(error, ParseError::Lexer(_)), "{source:?}: {error:?}");
    }
    let error = parse_string("%derive p::\nstruct S\n", &Config::default()).unwrap_err();
    assert!(matches!(error, ParseError::HardError(message, _) if message.contains("path segment after '::'")));
}
