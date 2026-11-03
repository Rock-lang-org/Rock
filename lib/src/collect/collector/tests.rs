use super::*;

use crate::ast::{
    FunctionDecl, FunctionSig, Ident, Impl, LambdaArrowKind, LambdaDecl, Module, ParseType,
    ParseTypeInner, StructDecl, TopLevel, TraitDecl,
};
use crate::collect::item_index::{ItemIndex, ItemKind, ItemRecord, ItemSourceId};
use crate::ids::{CrateId, LocalDefId, ModuleId};
use crate::lexer::Span;

fn test_span(text: &str) -> Span {
    Span {
        file_path: "/test.rk".into(),
        start: 0,
        end: text.len(),
    }
}

fn type_inner(name: &str) -> ParseTypeInner {
    ParseTypeInner {
        name: name.to_string(),
        generics: vec![],
        span: test_span(name),
    }
}

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: test_span(name),
    }
}

fn function_decl(name: &str) -> FunctionDecl {
    FunctionDecl {
        name: ident(name),
        lambda: LambdaDecl {
            parameters: vec![],
            body: crate::ast::Block { statements: vec![] },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        },
        self_receiver: None,
        is_unsafe: false,
        exported: false,
    }
}

fn function_sig(name: &str) -> FunctionSig {
    FunctionSig {
        name: ident(name),
        sig: ParseType::Unit(crate::lexer::Span::test()),
        where_clauses: vec![],
        self_receiver: None,
        is_unsafe: false,
        exported: false,
    }
}

#[test]
fn local_collector_collects_root_struct_declarations() {
    let module = Module {
        name: None,
        top_levels: vec![TopLevel::StructDecl(StructDecl {
            name: type_inner("Point"),
            generic_params: vec![],
            fields: vec![],
            exported: false,
        })],
        is_inline: false,
        filepath: None,
    };

    let mut id_environment = CollectedIdEnvironment::default();
    let source = ItemSourceId {
        module_id: ModuleId(0),
        top_level_index: 0,
    };
    id_environment.current_source_items.insert(
        source,
        ItemRecord {
            name_span: test_span("Point"),
            def_id: DefId::new(CrateId(0), LocalDefId(0)),
            module_id: ModuleId(0),
            source,
            name: "Point".to_string(),
            kind: ItemKind::Struct,
        },
    );

    let context = CollectContext::bootstrap_for_collection(false, Some("test"), None);
    let mut collector = LocalCollector::new_with_id_environment(context, id_environment);
    collector.collect_local_declarations(&module, ModuleId(0));
    let collected = collector.finish(&ItemIndex::new(), ModuleId(0), Some("test"));

    assert!(collected.structs.contains_key("Point"));
}

#[test]
fn local_collector_errors_when_member_id_environment_is_missing() {
    let module = Module {
        name: None,
        top_levels: vec![
            TopLevel::StructDecl(StructDecl {
                name: type_inner("Box"),
                generic_params: vec![],
                fields: vec![],
                exported: false,
            }),
            TopLevel::TraitDecl(TraitDecl {
                where_clauses: Vec::new(),
                name: type_inner("Show"),
                generic_params: vec![],
                for_: None,
                associated_types: vec![],
                methods: HashMap::from([(ident("show"), function_decl("show"))]),
                signatures: HashMap::from([(ident("convert"), function_sig("convert"))]),
                exported: false,
                language_items: Default::default(),
            }),
            TopLevel::Impl(Impl {
                name: ParseType::Type(type_inner("Show")),
                for_: Some(ParseType::Type(type_inner("Box"))),
                associated_types: vec![],
                methods: HashMap::from([(ident("show"), function_decl("show"))]),
                signatures: HashMap::new(),
                where_clauses: vec![],
            }),
        ],
        is_inline: false,
        filepath: None,
    };
    let mut id_environment = CollectedIdEnvironment::default();
    for (top_level_index, name, kind) in [
        (0, "Box", ItemKind::Struct),
        (1, "Show", ItemKind::Trait),
        (2, "Show", ItemKind::Impl),
    ] {
        let source = ItemSourceId {
            module_id: ModuleId(0),
            top_level_index,
        };
        id_environment.current_source_items.insert(
            source,
            ItemRecord {
                name_span: test_span(name),
                def_id: DefId::new(CrateId(0), LocalDefId(top_level_index)),
                module_id: ModuleId(0),
                source,
                name: name.to_string(),
                kind,
            },
        );
    }

    let context = CollectContext::bootstrap_for_collection(false, Some("test"), None);
    let mut collector = LocalCollector::new_with_id_environment(context, id_environment);
    collector.collect_local_declarations(&module, ModuleId(0));
    let collected = collector.finish(&ItemIndex::new(), ModuleId(0), Some("test"));

    assert!(collected.errors.iter().any(|error| error
        .message
        .contains("missing canonical current-crate trait method identity for test::Show.show")));
    assert!(collected.errors.iter().any(|error| error.message.contains(
        "missing canonical current-crate trait signature identity for test::Show.convert"
    )));
    assert!(collected.errors.iter().any(|error| error
        .message
        .contains("missing canonical current-crate impl method identity for impl.show")));
}
