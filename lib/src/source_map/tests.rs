use super::*;

use std::path::PathBuf;

use crate::collect::item_index::{index_root_module_items, IndexingIds};
use crate::ids::{CrateId, LocalDefId};

fn span(start: usize, end: usize) -> Span {
    Span {
        file_path: PathBuf::from("/virtual/main.rk"),
        start,
        end,
    }
}

#[test]
fn semantic_source_map_preserves_indexed_definition_name_span() {
    let path = PathBuf::from("/virtual/main.rk");
    let module = crate::parser::parse_source(
        path.clone(),
        "struct Widget\n\nmain = -> 0\n",
        &crate::Config::default(),
    )
    .expect("source should parse");
    let mut ids = IndexingIds::new_root();
    let index = index_root_module_items(&mut ids, &module);
    let widget = index
        .items()
        .iter()
        .find(|item| item.name == "Widget")
        .expect("Widget definition");

    let sources = SemanticSourceMap::from_item_index(&index);
    let span = sources
        .definition_span(widget.def_id)
        .expect("definition span");
    assert_eq!(span.file_path, path);
    assert_eq!(
        &"struct Widget\n\nmain = -> 0\n"[span.start..span.end],
        "Widget"
    );
}

#[test]
fn semantic_source_map_records_collected_declarations_from_real_source() {
    let path = PathBuf::from("/virtual/decls.rk");
    let text = "struct Widget T\n    < value: T\n\nmain = -> 0\n";
    let module = crate::parser::parse_source(path.clone(), text, &crate::Config::default())
        .expect("source should parse");
    let program = crate::ast::Program { module };
    let declarations = crate::collect::collect(
        &program,
        &crate::crate_system::CrateContext::new(),
        false,
        None,
    )
    .expect("declarations should collect");
    let widget_id = declarations.item_index.defs_named("Widget")[0];
    let widget = declarations
        .items
        .structure(widget_id)
        .expect("Widget struct");
    let generic = widget.generic_params[0].id;
    let field = widget.fields[0].id;
    let sources = declarations.source_map;

    let declaration = sources
        .symbol(&SourceSymbol::Definition(widget_id))
        .expect("Widget definition");
    assert_eq!(
        &text[declaration.name_span.start..declaration.name_span.end],
        "Widget"
    );
    assert!(declaration.declaration_span.as_ref().unwrap().end > declaration.name_span.end);
    assert_eq!(
        &text[sources
            .symbol(&SourceSymbol::Generic(generic))
            .unwrap()
            .name_span
            .start
            ..sources
                .symbol(&SourceSymbol::Generic(generic))
                .unwrap()
                .name_span
                .end],
        "T"
    );
    let field_span = &sources
        .symbol(&SourceSymbol::Field {
            owner: widget_id,
            field,
        })
        .expect("field declaration")
        .name_span;
    assert_eq!(&text[field_span.start..field_span.end], "value");
    assert!(sources.references().iter().any(|reference| {
        reference.target == SourceSymbol::Generic(generic)
            && &text[reference.span.start..reference.span.end] == "T"
    }));
}

#[test]
fn semantic_source_map_records_enum_and_alias_default_parameters() {
    let path = PathBuf::from("/virtual/defaults.rk");
    let text = "enum Choice A, {B = A}\n    First A\n    Second B\ntype Selected {T = I64} = Choice T\nmain = -> 0\n";
    let module = crate::parser::parse_source(path, text, &crate::Config::default()).unwrap();
    let declarations = crate::collect::collect(
        &crate::ast::Program { module },
        &crate::crate_system::CrateContext::new(),
        false,
        None,
    )
    .unwrap();
    let choice = declarations.item_index.defs_named("Choice")[0];
    let selected = declarations.item_index.defs_named("Selected")[0];
    let enumeration = declarations.items.enumeration(choice).unwrap();
    let alias = declarations.items.type_alias(selected).unwrap();
    for param in enumeration
        .generic_params
        .iter()
        .chain(&alias.generic_params)
    {
        let symbol = declarations
            .source_map
            .symbol(&SourceSymbol::Generic(param.id))
            .unwrap();
        assert_eq!(
            &text[symbol.name_span.start..symbol.name_span.end],
            param.name
        );
    }
    let default_offset = text.find("= A").unwrap() + 2;
    assert!(declarations.source_map.references.iter().any(|reference| {
        reference.span.start == default_offset
            && reference.target == SourceSymbol::Generic(enumeration.generic_params[0].id)
    }));
}

#[test]
fn semantic_source_map_records_trait_and_impl_type_references_from_real_source() {
    let path = PathBuf::from("/virtual/trait_refs.rk");
    let text = "struct Box T\n    value: T\n\ntrait Parent T\n    type Output\n    parent: T -> &Self::Output\n\ntrait Child T\n    type Output\n    child: T -> T\n\nimpl Child I64 for Box I64 where I64: Parent I64\n    type Output = I64\n    child: I64 -> I64\n\nmain = -> 0\n";
    let module = crate::parser::parse_source(path, text, &crate::Config::default())
        .expect("trait and impl source should parse");
    let declarations = crate::collect::collect(
        &crate::ast::Program { module },
        &crate::crate_system::CrateContext::new(),
        false,
        None,
    )
    .expect("trait and impl declarations should collect");

    let references = declarations.source_map.references();
    let has_text = |name: &str| {
        references.iter().any(|reference| {
            &text[reference.span.start..reference.span.end] == name
                && matches!(reference.target, SourceSymbol::Definition(_))
        })
    };
    assert!(has_text("Box"));
    assert!(has_text("Parent"));
    assert!(
        has_text("Child"),
        "recorded references: {:?}",
        references
            .iter()
            .map(|reference| &text[reference.span.start..reference.span.end])
            .collect::<Vec<_>>()
    );
    assert!(references.iter().any(|reference| {
        &text[reference.span.start..reference.span.end] == "T"
            && matches!(reference.target, SourceSymbol::Generic(_))
    }));
    assert!(references.iter().any(|reference| {
        &text[reference.span.start..reference.span.end] == "Output"
            && matches!(reference.target, SourceSymbol::AssociatedType { .. })
    }));
}

#[test]
fn semantic_source_map_tracks_child_and_generic_symbols() {
    let owner = DefId::new(CrateId(0), LocalDefId(1));
    let child = DefId::new(CrateId(0), LocalDefId(2));
    let generic = GenericParamId { owner, index: 0 };
    let mut sources = SemanticSourceMap::default();
    sources.insert_definition(owner, span(1, 5));
    sources.insert_definition(child, span(23, 28));
    sources.insert_generic(generic, span(6, 7), owner);
    sources.insert_field(owner, FieldId(0), span(8, 9));
    sources.insert_variant(owner, VariantId(0), span(10, 15));
    sources.insert_associated_type(owner, AssocTypeId(0), span(16, 22));

    assert_eq!(
        &sources
            .symbol(&SourceSymbol::Generic(generic))
            .unwrap()
            .name_span,
        &span(6, 7)
    );
    assert!(sources.symbol(&SourceSymbol::Definition(child)).is_some());
    assert!(sources
        .symbol(&SourceSymbol::Field {
            owner,
            field: FieldId(0)
        })
        .is_some());
    assert!(sources
        .symbol(&SourceSymbol::Variant {
            owner,
            variant: VariantId(0)
        })
        .is_some());
    assert!(sources
        .symbol(&SourceSymbol::AssociatedType {
            owner,
            associated: AssocTypeId(0)
        })
        .is_some());
}

#[test]
fn semantic_source_map_keeps_cross_module_and_shadowed_local_targets_distinct() {
    let owner = DefId::new(CrateId(0), LocalDefId(1));
    let imported = DefId::new(CrateId(2), LocalDefId(9));
    let first = HirLocalId(3);
    let second = HirLocalId(4);
    let mut sources = SemanticSourceMap::default();
    sources.insert_local(owner, first, span(1, 2));
    sources.insert_local(owner, second, span(3, 4));
    sources.record_reference(
        span(10, 15),
        SourceSymbol::Definition(imported),
        Some(owner),
    );
    sources.record_reference(
        span(20, 21),
        SourceSymbol::Local {
            owner,
            local: first,
        },
        Some(owner),
    );
    sources.record_reference(
        span(30, 31),
        SourceSymbol::Local {
            owner,
            local: second,
        },
        Some(owner),
    );

    assert_eq!(
        sources
            .reference_spans(&SourceSymbol::Definition(imported))
            .count(),
        1
    );
    assert_eq!(
        sources
            .reference_spans(&SourceSymbol::Local {
                owner,
                local: first
            })
            .count(),
        1
    );
    assert_eq!(
        sources
            .reference_spans(&SourceSymbol::Local {
                owner,
                local: second
            })
            .count(),
        1
    );
}

#[test]
fn semantic_source_map_remaps_generated_method_owners_without_serialization() {
    let from = DefId::new(CrateId(0), LocalDefId(10));
    let to = DefId::new(CrateId(0), LocalDefId(20));
    let local = HirLocalId(3);
    let generic = GenericParamId {
        owner: from,
        index: 0,
    };
    let mut sources = SemanticSourceMap::default();
    sources.insert_definition(from, span(1, 4));
    sources.insert_local(from, local, span(5, 6));
    sources.insert_generic(generic, span(7, 8), from);
    sources.record_reference(
        span(10, 11),
        SourceSymbol::Local { owner: from, local },
        Some(from),
    );

    sources.remap_owner(from, to);

    assert_eq!(sources.definition_span(to), Some(&span(1, 4)));
    assert_eq!(sources.local_span(to, local), Some(&span(5, 6)));
    assert!(sources
        .symbol(&SourceSymbol::Generic(GenericParamId {
            owner: to,
            index: 0
        }))
        .is_some());
    assert_eq!(
        sources
            .reference_spans(&SourceSymbol::Local { owner: to, local })
            .count(),
        1
    );
}

#[test]
fn semantic_source_map_records_active_scope_for_shadowed_references() {
    let owner = DefId::new(CrateId(0), LocalDefId(10));
    let local = HirLocalId(1);
    let mut sources = SemanticSourceMap::default();
    sources.insert_local_in_scope(owner, local, span(0, 1), Some(3));
    sources.record_reference_in_scope(
        span(1, 2),
        SourceSymbol::Local { owner, local },
        Some(owner),
        Some(3),
    );
    sources.record_reference_in_scope(
        span(4, 5),
        SourceSymbol::Local { owner, local },
        Some(owner),
        Some(4),
    );

    assert_eq!(sources.references()[0].scope_id, Some(3));
    assert_eq!(sources.references()[1].scope_id, Some(4));
    assert_eq!(
        sources
            .symbol(&SourceSymbol::Local { owner, local })
            .unwrap()
            .scope_id,
        Some(3)
    );
    assert_ne!(
        (
            sources.references()[0].target.clone(),
            sources.references()[0].scope_id
        ),
        (
            sources.references()[1].target.clone(),
            sources.references()[1].scope_id
        )
    );
}
