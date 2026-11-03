use super::*;
use crate::lexer::Span;

fn format_node<T: FormatNode>(node: &T) -> String {
    let mut context = FormatContext::new();
    let mut output = String::new();
    node.fmt_with(&mut context, &mut output)
        .expect("formatting into a String should not fail");
    output
}

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: Span::test(),
    }
}

#[test]
fn function_sig_formats_self_receiver_prefixes() {
    let cases = [
        (Some(SelfReceiverMode::Shared), "@a : ()\n"),
        (Some(SelfReceiverMode::Mut), "^@a : ()\n"),
        (Some(SelfReceiverMode::Move), "~@a : ()\n"),
        (None, "a : ()\n"),
    ];

    for (self_receiver, expected) in cases {
        let sig = FunctionSig {
            name: ident("a"),
            sig: ParseType::Unit(crate::lexer::Span::test()),
            where_clauses: vec![],
            self_receiver,
            is_unsafe: false,
            exported: false,
        };

        assert_eq!(format_node(&sig), expected);
    }
}

#[test]
fn function_decl_formats_self_receiver_prefixes() {
    let cases = [
        (Some(SelfReceiverMode::Shared), "@a = -> \n"),
        (Some(SelfReceiverMode::Mut), "^@a = -> \n"),
        (Some(SelfReceiverMode::Move), "~@a = -> \n"),
        (None, "a = -> \n"),
    ];

    for (self_receiver, expected) in cases {
        let decl = FunctionDecl {
            name: ident("a"),
            lambda: LambdaDecl {
                parameters: vec![],
                body: Block { statements: vec![] },
                arrow_kind: LambdaArrowKind::Normal,
                span: crate::lexer::Span::test(),
            },
            self_receiver,
            is_unsafe: false,
            exported: false,
        };

        assert_eq!(format_node(&decl), expected);
    }
}
