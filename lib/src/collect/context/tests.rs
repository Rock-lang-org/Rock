use super::*;

use crate::ast::{Ident, Module, ParseType, ParseTypeInner};
use crate::hir::HirAssociatedTypeDecl;
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::lexer::Span;
use crate::types::{GenericParamDecl, GenericParamId, Type};

fn named_type(name: &str) -> ParseType {
    ParseType::Type(ParseTypeInner {
        name: name.to_string(),
        generics: vec![],
        span: Span::test(),
    })
}

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

#[test]
fn load_external_crate_module_prefers_seeded_module_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_external_dir_module_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(temp_dir.join("util")).unwrap();
    let dep_root = temp_dir.join("lib.rk");
    let util_mod = temp_dir.join("util/mod.rk");
    std::fs::write(&dep_root, "mod util\n").unwrap();
    std::fs::write(&util_mod, "answer = -> 42\n").unwrap();

    let module = Module {
        name: Some(Ident {
            name: "util".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![],
        is_inline: false,
        filepath: Some(util_mod.clone()),
    };
    let mut context = CollectContext::new();
    context
        .loaded_module_paths
        .push(("dep".to_string(), dep_root.clone()));
    context
        .loaded_module_paths
        .push(("dep::util".to_string(), util_mod.clone()));
    context.module_file_cache.insert(util_mod.clone(), module);

    let loaded = context
        .load_external_crate_module("util", "dep")
        .expect("external module loading should use graph-seeded util/mod.rk path");

    assert_eq!(loaded.filepath.as_ref(), Some(&util_mod));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn load_module_requires_seeded_module_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_requires_seeded_module_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let root = temp_dir.join("main.rk");
    let util = temp_dir.join("util.rk");

    let module = Module {
        name: Some(Ident {
            name: "util".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![],
        is_inline: false,
        filepath: Some(util.clone()),
    };
    let mut context = CollectContext::new();
    context.current_module_path = root;
    context.module_file_cache.insert(util, module);

    let error = context
        .load_module("util")
        .expect_err("module cache should not bypass graph-seeded module paths");

    assert!(error.contains("was not loaded by the source database"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn load_module_with_prefix_does_not_fallback_to_unqualified_module() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_no_unqualified_fallback_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let top_level_io = temp_dir.join("io.rk");

    let module = Module {
        name: Some(Ident {
            name: "io".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![],
        is_inline: false,
        filepath: Some(top_level_io.clone()),
    };
    let mut context = CollectContext::new();
    context
        .loaded_module_paths
        .push(("io".to_string(), top_level_io.clone()));
    context.module_file_cache.insert(top_level_io, module);

    let error = context
        .load_module_with_prefix("io", Some("math"))
        .expect_err("prefixed lookup must not fall back to top-level io");

    assert!(error.contains("math::io"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn load_module_with_prefix_prefers_current_crate_prefixed_graph_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_prefers_current_crate_graph_path_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let alias_io = temp_dir.join("alias_io.rk");
    let graph_io = temp_dir.join("graph_io.rk");

    let alias_module = Module {
        name: Some(Ident {
            name: "io".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![],
        is_inline: false,
        filepath: Some(alias_io.clone()),
    };
    let graph_module = Module {
        name: Some(Ident {
            name: "io".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![],
        is_inline: false,
        filepath: Some(graph_io.clone()),
    };
    let mut context = CollectContext::new();
    context.current_crate_name = Some("test".to_string());
    context
        .loaded_module_paths
        .push(("math::io".to_string(), alias_io.clone()));
    context
        .loaded_module_paths
        .push(("test::math::io".to_string(), graph_io.clone()));
    context.module_file_cache.insert(alias_io, alias_module);
    context
        .module_file_cache
        .insert(graph_io.clone(), graph_module);

    let module = context
        .load_module_with_prefix("io", Some("math"))
        .expect("current-crate-prefixed graph path should resolve");

    assert_eq!(module.filepath.as_ref(), Some(&graph_io));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn load_external_crate_module_requires_seeded_child_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_requires_seeded_external_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let dep_root = temp_dir.join("lib.rk");
    let util = temp_dir.join("util.rk");

    let module = Module {
        name: Some(Ident {
            name: "util".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![],
        is_inline: false,
        filepath: Some(util.clone()),
    };
    let mut context = CollectContext::new();
    context
        .loaded_module_paths
        .push(("dep".to_string(), dep_root));
    context.module_file_cache.insert(util, module);

    let error = context
        .load_external_crate_module("util", "dep")
        .expect_err("external module cache should not bypass graph-seeded module paths");

    assert!(error.contains("was not loaded by the source database"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_context_lower_parse_type_reports_bare_slice_error() {
    let mut context = CollectContext::new();

    let ty = context.lower_parse_type(&ParseType::Slice(Box::new(named_type("I64"))));

    assert_eq!(ty, Type::Error);
    assert!(context
        .errors
        .iter()
        .any(|error| error.message.contains("bare slice type [T]")));
}

#[test]
fn collect_context_lower_parse_type_creates_implicit_generic() {
    let owner = def_id(70);
    let mut context = CollectContext::new();
    context.current_generic_owner = Some(owner);

    let ty = context.lower_parse_type(&named_type("T"));

    assert_eq!(ty, Type::Generic(GenericParamId { owner, index: 0 }));
    assert_eq!(context.current_generic_params, vec!["T".to_string()]);
    assert!(context.errors.is_empty());
}

#[test]
fn collect_context_unknown_generic_error_keeps_parse_span() {
    let owner = def_id(71);
    let span = Span::new("/virtual/types.rk".into(), 9, 10);
    let mut context = CollectContext::new();
    context.current_generic_owner = Some(owner);
    context.current_generic_params = vec!["U".to_string()];
    context.current_generic_kinds = vec![crate::type_services::kind::Kind::Type];

    let ty = context.lower_parse_type(&ParseType::Type(ParseTypeInner {
        name: "missing".to_string(),
        generics: Vec::new(),
        span: span.clone(),
    }));

    assert_eq!(ty, Type::Error);
    assert_eq!(context.errors[0].span(), Some(span));
}

#[test]
fn collect_context_reports_duplicate_generic_names_before_lookup() {
    let owner = def_id(72);
    let mut context = CollectContext::new();

    context.push_generic_context(owner, vec!["T".to_string(), "T".to_string()]);

    assert!(context
        .errors
        .iter()
        .any(|error| { error.message.contains("duplicate generic parameter 'T'") }));
}

#[test]
fn collect_context_nominal_type_does_not_resolve_by_suffix() {
    let struct_id = def_id(75);
    let mut context = CollectContext::new();
    context.structs.insert(
        "dep::Widget".to_string(),
        HirStruct {
            id: struct_id,
            name: "dep::Widget".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        },
    );

    let ty = context.lower_parse_type(&named_type("Widget"));

    assert_eq!(ty, Type::Error);
    assert!(!context.errors.is_empty());
}

#[test]
fn collect_context_lowers_associated_type_through_canonical_trait_import_id() {
    let trait_id = def_id(80);
    let assoc_type_id = AssocTypeId(3);
    let mut context = CollectContext::new();
    context.traits.insert(
        "dep::Iterable".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "dep::Iterable".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "Self",
            )],
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_type_id,
                name: "Item".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    context
        .names
        .record_canonical_alias("Iterable".to_string(), trait_id, true);
    context
        .names
        .set_canonical_name(trait_id, "dep::Iterable".to_string());
    context.current_trait = Some("Iterable".to_string());
    context.current_generic_owner = Some(trait_id);
    context.current_generic_params = vec!["Self".to_string()];

    let ty = context.lower_parse_type(&ParseType::Associated {
        base: ParseTypeInner {
            name: "Self".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        member: Ident {
            name: "Item".to_string(),
            span: Span::test(),
        },
    });

    assert!(matches!(ty, Type::Projection { trait_id: id, .. } if id == trait_id));
    assert!(context.errors.is_empty());
}
