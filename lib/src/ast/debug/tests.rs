use std::collections::HashMap;

use crate::lexer::Span;

use super::*;

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: Span::test(),
    }
}

#[test]
fn ast_walk_visits_associated_types_and_signatures_in_traits() {
    let signature_name = ident("split");
    let trait_decl = TraitDecl {
        where_clauses: Vec::new(),
        name: ParseTypeInner {
            name: "Carrier".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![],
        for_: None,
        associated_types: vec![AssociatedTypeDecl {
            name: ident("Value"),
            kind: None,
        }],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            signature_name.clone(),
            FunctionSig {
                name: signature_name,
                sig: ParseType::Unit(crate::lexer::Span::test()),
                where_clauses: vec![],
                self_receiver: None,
                is_unsafe: false,
                exported: false,
            },
        )]),
        exported: false,
        language_items: Default::default(),
    };
    let mut collector = IdentCollector::default();

    trait_decl.visit(&mut collector);

    assert!(collector.0.iter().any(|name| name == "Value"));
    assert!(collector.0.iter().any(|name| name == "split"));
}

/// Records every identifier the AST walk reaches.
#[derive(Default)]
struct IdentCollector(Vec<String>);

impl<'a> crate::ast::visit::Visitor<'a> for IdentCollector {
    fn visit_ident(&mut self, ident: &'a Ident) {
        self.0.push(ident.name.clone());
    }
}
