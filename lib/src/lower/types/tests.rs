use super::*;

use std::collections::HashMap;

use crate::ast::{ParseType, ParseTypeInner};
use crate::hir::{HirEnum, HirStruct, HirTrait};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::types::{GenericParamDecl, GenericParamId};

fn named_type(name: &str) -> ParseType {
    ParseType::Type(ParseTypeInner {
        name: name.to_string(),
        generics: vec![],
        span: crate::lexer::Span::test(),
    })
}

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn test_struct(id: DefId, name: &str) -> HirStruct {
    HirStruct {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    }
}

fn test_enum(id: DefId, name: &str) -> HirEnum {
    HirEnum {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        variants: Vec::new(),
    }
}

fn test_trait(id: DefId, name: &str) -> HirTrait {
    HirTrait {
        target: None,
        predicates: Vec::new(),
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    }
}

fn lowerer_with_nominals() -> Lowerer {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_structure(HirStruct {
        id: def_id(10),
        name: "Widget".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(10),
                index: 0,
            },
            "T",
        )],
        fields: Vec::new(),
    });
    lowerer
        .resolver
        .item_paths
        .insert("Widget".to_string(), def_id(10));
    lowerer
        .resolver
        .item_names_by_id
        .insert(def_id(10), "Widget".to_string());
    lowerer.items.insert_enumeration(HirEnum {
        id: def_id(20),
        name: "Choice".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(20),
                index: 0,
            },
            "T",
        )],
        variants: Vec::new(),
    });
    lowerer
        .resolver
        .item_paths
        .insert("Choice".to_string(), def_id(20));
    lowerer
        .resolver
        .item_names_by_id
        .insert(def_id(20), "Choice".to_string());
    lowerer
}

#[test]
fn lower_nominal_type_uses_decl_def_id() {
    let mut lowerer = lowerer_with_nominals();
    lowerer
        .resolver
        .item_names_by_id
        .insert(def_id(1), "other::Widget".to_string());
    lowerer
        .resolver
        .item_names_by_id
        .insert(def_id(2), "other::Choice".to_string());

    let struct_ty = lowerer.lower_parse_type(&ParseType::Type(ParseTypeInner {
        name: "Widget".to_string(),
        generics: vec![named_type("I64")],
        span: crate::lexer::Span::test(),
    }));
    let enum_ty = lowerer.lower_parse_type(&ParseType::Type(ParseTypeInner {
        name: "Choice".to_string(),
        generics: vec![named_type("Bool")],
        span: crate::lexer::Span::test(),
    }));

    assert_eq!(
        struct_ty,
        Type::Struct {
            id: def_id(10),
            args: vec![Type::I64],
        }
    );
    assert_eq!(
        enum_ty,
        Type::Enum {
            id: def_id(20),
            args: vec![Type::Bool],
        }
    );
    assert!(lowerer.source_map.references().iter().any(|reference| {
        reference.target == crate::source_map::SourceSymbol::Definition(def_id(10))
    }));
    assert!(lowerer.source_map.references().iter().any(|reference| {
        reference.target == crate::source_map::SourceSymbol::Definition(def_id(20))
    }));
}

#[test]
fn lower_nominal_type_prefers_resolver_alias_id_over_suffix_match() {
    let canonical_id = def_id(40);
    let suffix_collision_id = def_id(41);
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_structure(HirStruct {
        id: canonical_id,
        name: "dep::Widget".to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    });
    lowerer.items.insert_structure(HirStruct {
        id: suffix_collision_id,
        name: "Widget".to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    });
    lowerer.items.insert_structure(HirStruct {
        id: suffix_collision_id,
        name: "other::Widget".to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    });
    lowerer.resolver.insert_import_alias_with_name(
        "Widget".to_string(),
        "dep::Widget".to_string(),
        canonical_id,
    );

    let ty = lowerer.lower_parse_type(&named_type("Widget"));

    assert_eq!(
        ty,
        Type::Struct {
            id: canonical_id,
            args: Vec::new(),
        }
    );
}

#[test]
fn lower_struct_type_prefers_module_local_alias_over_root_type() {
    let root_id = def_id(42);
    let module_id = def_id(43);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(root_id, "Thing"));
    lowerer
        .items
        .insert_structure(test_struct(module_id, "demo::helper::Thing"));
    lowerer
        .resolver
        .item_paths
        .insert("Thing".to_string(), root_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_id, "Thing".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "Thing".to_string(),
        "demo::helper::Thing".to_string(),
        module_id,
    );

    let ty = lowerer.lower_parse_type(&named_type("Thing"));

    assert_eq!(
        ty,
        Type::Struct {
            id: module_id,
            args: Vec::new(),
        }
    );
}

#[test]
fn lower_enum_type_prefers_module_local_alias_over_root_type() {
    let root_id = def_id(44);
    let module_id = def_id(45);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_enumeration(test_enum(root_id, "Choice"));
    lowerer
        .items
        .insert_enumeration(test_enum(module_id, "demo::helper::Choice"));
    lowerer
        .resolver
        .item_paths
        .insert("Choice".to_string(), root_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_id, "Choice".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "Choice".to_string(),
        "demo::helper::Choice".to_string(),
        module_id,
    );

    let ty = lowerer.lower_parse_type(&named_type("Choice"));

    assert_eq!(
        ty,
        Type::Enum {
            id: module_id,
            args: Vec::new(),
        }
    );
}

#[test]
fn resolve_trait_type_prefers_module_local_alias_over_root_trait() {
    let root_id = def_id(46);
    let module_id = def_id(47);
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_trait_def(test_trait(root_id, "Show"));
    lowerer
        .items
        .insert_trait_def(test_trait(module_id, "demo::helper::Show"));
    lowerer
        .resolver
        .item_paths
        .insert("Show".to_string(), root_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_id, "Show".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "Show".to_string(),
        "demo::helper::Show".to_string(),
        module_id,
    );

    let trait_def = lowerer.resolve_trait_type("Show").unwrap();

    assert_eq!(trait_def.id, module_id);
}

#[test]
fn test_lower_parse_bare_slice_type_reports_error() {
    let mut lowerer = Lowerer::new_for_test();

    let ty = lowerer.lower_parse_type(&ParseType::Slice(Box::new(named_type("I64"))));

    assert_eq!(ty, Type::Error);
    assert!(lowerer.errors().iter().any(|err| err
        .message
        .contains("bare slice type [T] must be written behind a reference")));
}

#[test]
fn test_lower_parse_borrowed_slice_type_lowers_to_reference_to_slice_type() {
    let mut lowerer = Lowerer::new_for_test();

    let ty = lowerer.lower_parse_type(&ParseType::Reference {
        is_mut: false,
        pointee: Box::new(ParseType::Slice(Box::new(named_type("I64")))),
    });

    assert_eq!(
        ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Slice(Box::new(Type::I64))),
        }
    );
    assert!(lowerer.errors().is_empty());
}

#[test]
fn test_lower_parse_raw_slice_pointer_lowers_to_pointer_to_slice_type() {
    let mut lowerer = Lowerer::new_for_test();

    let ty = lowerer.lower_parse_type(&ParseType::Pointer(Box::new(ParseType::Slice(Box::new(
        named_type("I64"),
    )))));

    assert_eq!(
        ty,
        Type::Pointer(Box::new(Type::Slice(Box::new(Type::I64))))
    );
    assert!(lowerer.errors().is_empty());
}

#[test]
fn test_lower_parse_bare_str_reports_error() {
    let mut lowerer = Lowerer::new_for_test();

    let ty = lowerer.lower_parse_type(&named_type("Str"));

    assert_eq!(ty, Type::Error);
    assert!(lowerer.errors().iter().any(|err| err
        .message
        .contains("bare string slice type Str must be written behind a reference")));
}

#[test]
fn test_lower_parse_borrowed_str_lowers_to_reference_to_str_type() {
    let mut lowerer = Lowerer::new_for_test();

    let ty = lowerer.lower_parse_type(&ParseType::Reference {
        is_mut: false,
        pointee: Box::new(named_type("Str")),
    });

    assert_eq!(
        ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        }
    );
    assert!(lowerer.errors().is_empty());
}

#[test]
fn test_lower_parse_raw_str_pointer_lowers_to_pointer_to_str_type() {
    let mut lowerer = Lowerer::new_for_test();

    let ty = lowerer.lower_parse_type(&ParseType::Pointer(Box::new(named_type("Str"))));

    assert_eq!(ty, Type::Pointer(Box::new(Type::Str)));
    assert!(lowerer.errors().is_empty());
}

#[test]
fn test_lower_parse_mut_borrowed_str_lowers_to_mut_reference_to_str_type() {
    let mut lowerer = Lowerer::new_for_test();

    let ty = lowerer.lower_parse_type(&ParseType::Reference {
        is_mut: true,
        pointee: Box::new(named_type("Str")),
    });

    assert_eq!(
        ty,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::Str),
        }
    );
    assert!(lowerer.errors().is_empty());
}

#[test]
fn test_lower_parse_fixed_array_type_preserves_length_without_error() {
    let mut lowerer = Lowerer::new_for_test();

    let ty = lowerer.lower_parse_type(&ParseType::Array {
        inner: Box::new(named_type("I64")),
        len: ast::ParseArrayLen::Literal(4),
    });

    assert_eq!(ty, Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Value(4)));
    assert!(lowerer.errors().is_empty());
}

#[test]
fn lower_parse_type_creates_implicit_generic_in_current_context() {
    let owner = def_id(60);
    let mut lowerer = Lowerer::new_for_test();
    let _outer_frame = lowerer.item_scope.enter_generics(owner, Vec::new());

    let ty = lowerer.lower_parse_type(&named_type("T"));

    assert_eq!(
        ty,
        Type::Generic(crate::types::GenericParamId { owner, index: 0 })
    );
    assert_eq!(lowerer.current_generic_params(), &["T".to_string()]);
    assert!(lowerer.errors().is_empty());
}
