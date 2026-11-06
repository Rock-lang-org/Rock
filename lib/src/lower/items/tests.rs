use std::collections::HashMap;

use super::LowerItems;
use crate::collect::item_index::{IndexingIds, ItemIndex};
use crate::collect::resolver::ResolverTables;
use crate::collect::{DeclarationItems, DeclarationTypeVars, Declarations};
use crate::hir::{
    HirBlock, HirEnum, HirExtern, HirFunction, HirImpl, HirImplOwner, HirImplReceiverPattern,
    HirStruct, HirTrait,
};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::lower::Lowerer;
use crate::types::Type;

fn test_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::I64,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
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

fn test_extern(id: DefId, name: &str) -> HirExtern {
    HirExtern {
        id,
        name: name.to_string(),
        params: Vec::new(),
        ret: Type::Unit,
        variadic: false,
        is_unsafe: false,
    }
}

fn test_impl(id: DefId, owner: &str, type_name: &str) -> HirImpl {
    HirImpl {
        id,
        owner: HirImplOwner::Named(owner.to_string()),
        type_name: type_name.to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::I64),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::new(),
    }
}

#[test]
fn lowerer_from_declarations_populates_item_store() {
    let function_id = DefId::new(CrateId(0), LocalDefId(1));
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(function_id, "answer".to_string());
    let decls = Declarations {
        indexing_ids: IndexingIds::new_root(),
        item_index: ItemIndex::default(),
        resolver,
        current_def_ids: [function_id].into_iter().collect(),
        items: DeclarationItems::from_id_maps(
            HashMap::from([(function_id, test_function(function_id, "answer"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
        .unwrap(),
        function_type_vars: HashMap::new(),
        infix_precedence: HashMap::new(),
        infix_owners: HashMap::new(),
        loaded_module_paths: Vec::new(),
        type_vars: DeclarationTypeVars::new(),
        inject_prelude: false,
        loaded_prelude_export_ids: HashMap::new(),
        module_file_cache: HashMap::new(),
        source_modules: crate::source_loader::SourceModuleSet::default(),
        dependency_root_export_ids: HashMap::new(),
        source_map: Default::default(),
        language_items: Default::default(),
    };

    let lowerer = Lowerer::from_declarations(decls).unwrap();

    assert!(lowerer.items.function(function_id).is_some());
}

#[test]
fn lowerer_items_are_keyed_by_declaration_ids_not_map_names() {
    let function_id = DefId::new(CrateId(0), LocalDefId(1));
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(function_id, "answer".to_string());
    let decls = Declarations {
        indexing_ids: IndexingIds::new_root(),
        item_index: ItemIndex::default(),
        resolver,
        current_def_ids: [function_id].into_iter().collect(),
        items: DeclarationItems::from_id_maps(
            HashMap::from([(function_id, test_function(function_id, "answer"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
        .unwrap(),
        function_type_vars: HashMap::new(),
        infix_precedence: HashMap::new(),
        infix_owners: HashMap::new(),
        loaded_module_paths: Vec::new(),
        type_vars: DeclarationTypeVars::new(),
        inject_prelude: false,
        loaded_prelude_export_ids: HashMap::new(),
        module_file_cache: HashMap::new(),
        source_modules: crate::source_loader::SourceModuleSet::default(),
        dependency_root_export_ids: HashMap::new(),
        source_map: Default::default(),
        language_items: Default::default(),
    };

    let lowerer = Lowerer::from_declarations(decls).unwrap();

    assert_eq!(lowerer.items.function(function_id).unwrap().name, "answer");
}

#[test]
fn function_mutation_uses_resolved_id_when_alias_and_display_name_differ() {
    let function_id = DefId::new(CrateId(0), LocalDefId(1));
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(function_id, "source::canonical_answer".to_string());
    resolver
        .import_aliases
        .insert("answer".to_string(), function_id);
    let decls = Declarations {
        indexing_ids: IndexingIds::new_root(),
        item_index: ItemIndex::default(),
        resolver,
        current_def_ids: [function_id].into_iter().collect(),
        items: DeclarationItems::from_id_maps(
            HashMap::from([(function_id, test_function(function_id, "displayed answer"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
        .unwrap(),
        function_type_vars: HashMap::new(),
        infix_precedence: HashMap::new(),
        infix_owners: HashMap::new(),
        loaded_module_paths: Vec::new(),
        type_vars: DeclarationTypeVars::new(),
        inject_prelude: false,
        loaded_prelude_export_ids: HashMap::new(),
        module_file_cache: HashMap::new(),
        source_modules: crate::source_loader::SourceModuleSet::default(),
        dependency_root_export_ids: HashMap::new(),
        source_map: Default::default(),
        language_items: Default::default(),
    };

    let mut lowerer = Lowerer::from_declarations(decls).unwrap();
    let resolved_id = lowerer
        .resolve_module_alias_or_item_def_id("answer")
        .unwrap();
    lowerer.items.function_mut(resolved_id).unwrap().name = "lowered answer".to_string();

    assert_eq!(lowerer.items.functions().count(), 1);
    assert_eq!(
        lowerer.items.function(function_id).unwrap().name,
        "lowered answer"
    );
}

#[test]
fn function_iteration_and_mutation_are_id_keyed() {
    let function_id = DefId::new(CrateId(0), LocalDefId(2));
    let mut items = LowerItems::new();
    items.insert_function(test_function(function_id, "answer"));

    let entries: Vec<_> = items
        .functions()
        .map(|(id, function)| (id, function.name.clone()))
        .collect();
    assert_eq!(entries, vec![(function_id, "answer".to_string())]);

    items.function_mut(function_id).unwrap().name = "changed".to_string();
    assert_eq!(items.function(function_id).unwrap().name, "changed");
}

#[test]
fn declaration_store_moves_through_lower_items_with_exact_id_mutation() {
    let function_id = DefId::new(CrateId(0), LocalDefId(8));
    let declarations = DeclarationItems::from_id_maps(
        HashMap::from([(function_id, test_function(function_id, "collected"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
    )
    .unwrap();

    let mut items = LowerItems::from_declarations(declarations).unwrap();
    items.function_mut(function_id).unwrap().name = "lowered".to_string();
    let declarations = items.into_validated_declarations().unwrap();

    assert_eq!(declarations.function(function_id).unwrap().id, function_id);
    assert_eq!(declarations.function(function_id).unwrap().name, "lowered");
}

#[test]
fn impl_storage_is_id_keyed_ordered_and_rejects_duplicates() {
    let first_id = DefId::new(CrateId(3), LocalDefId(9));
    let second_id = DefId::new(CrateId(1), LocalDefId(2));
    let mut items = LowerItems::new();

    items
        .insert_impl(test_impl(first_id, "display::Alpha", "renamed Alpha"))
        .unwrap();
    items
        .insert_impl(test_impl(second_id, "display::Beta", "renamed Beta"))
        .unwrap();

    items.impl_def_mut(first_id).unwrap().type_name = "changed Alpha".to_string();

    assert_eq!(
        items.impl_def(first_id).unwrap().owner,
        HirImplOwner::Named("display::Alpha".to_string())
    );
    assert_eq!(items.impl_def(second_id).unwrap().type_name, "renamed Beta");
    assert_eq!(
        items
            .impl_defs_in_order()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        vec![second_id, first_id]
    );
    assert!(items
        .insert_impl(test_impl(first_id, "other::Alpha", "other Alpha"))
        .is_err());
    assert_eq!(items.impl_def(first_id).unwrap().type_name, "changed Alpha");
}

#[test]
fn impl_declarations_remain_id_keyed_after_validation() {
    let first_id = DefId::new(CrateId(4), LocalDefId(8));
    let second_id = DefId::new(CrateId(4), LocalDefId(3));
    let mut items = LowerItems::new();
    items
        .insert_impl(test_impl(first_id, "display::First", "First"))
        .unwrap();
    items
        .insert_impl(test_impl(second_id, "display::Second", "Second"))
        .unwrap();

    let declarations = items.into_validated_declarations().unwrap();
    let impls = declarations.impls();

    assert_eq!(impls.len(), 2);
    assert_eq!(impls.get(&first_id).unwrap().id, first_id);
    assert_eq!(impls.get(&second_id).unwrap().id, second_id);
}

#[test]
fn extern_lookup_and_mutation_use_resolved_id_not_alias_or_display_name() {
    let extern_id = DefId::new(CrateId(7), LocalDefId(2));
    let mut items = LowerItems::new();
    items.insert_extern(test_extern(extern_id, "display puts"));

    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(extern_id, "dep::puts".to_string());
    resolver
        .import_aliases
        .insert("puts".to_string(), extern_id);
    let resolved_id = resolver.resolve_item_or_alias("puts").unwrap();

    items.extern_mut(resolved_id).unwrap().name = "lowered puts".to_string();

    assert_eq!(items.externs().count(), 1);
    assert_eq!(items.extern_def(extern_id).unwrap().name, "lowered puts");
}

#[test]
fn struct_lookup_and_mutation_use_resolved_id_not_alias_or_display_name() {
    let struct_id = DefId::new(CrateId(0), LocalDefId(3));
    let mut items = LowerItems::new();
    items.insert_structure(test_struct(struct_id, "display Widget"));

    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(struct_id, "dep::Widget".to_string());
    resolver
        .import_aliases
        .insert("Widget".to_string(), struct_id);
    let resolved_id = resolver.resolve_item_or_alias("Widget").unwrap();

    items.structure_mut(resolved_id).unwrap().name = "lowered Widget".to_string();

    assert_eq!(items.structures().count(), 1);
    assert_eq!(items.structure(struct_id).unwrap().name, "lowered Widget");
}

#[test]
fn enum_lookup_and_mutation_use_resolved_id_not_alias_or_display_name() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(4));
    let mut items = LowerItems::new();
    items.insert_enumeration(test_enum(enum_id, "display Choice"));

    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(enum_id, "dep::Choice".to_string());
    resolver
        .import_aliases
        .insert("Choice".to_string(), enum_id);
    let resolved_id = resolver.resolve_item_or_alias("Choice").unwrap();

    items.enumeration_mut(resolved_id).unwrap().name = "lowered Choice".to_string();

    assert_eq!(items.enumerations().count(), 1);
    assert_eq!(items.enumeration(enum_id).unwrap().name, "lowered Choice");
}

#[test]
fn trait_lookup_and_mutation_use_resolved_id_not_alias_or_display_name() {
    let trait_id = DefId::new(CrateId(0), LocalDefId(5));
    let mut items = LowerItems::new();
    items.insert_trait_def(test_trait(trait_id, "display Reader"));

    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(trait_id, "dep::Reader".to_string());
    resolver
        .import_aliases
        .insert("Reader".to_string(), trait_id);
    let resolved_id = resolver.resolve_item_or_alias("Reader").unwrap();

    items.trait_def_mut(resolved_id).unwrap().name = "lowered Reader".to_string();

    assert_eq!(items.trait_defs().count(), 1);
    assert_eq!(items.trait_def(trait_id).unwrap().name, "lowered Reader");
}

#[test]
fn lowerer_from_declarations_resumes_collection_type_var_allocator() {
    let mut type_vars = DeclarationTypeVars::new();
    assert_eq!(
        type_vars.fresh_type_var(),
        Type::TypeVar(crate::ids::TypeVarId(0))
    );
    assert_eq!(
        type_vars.fresh_type_var(),
        Type::TypeVar(crate::ids::TypeVarId(1))
    );
    let decls = Declarations {
        indexing_ids: IndexingIds::new_root(),
        item_index: ItemIndex::default(),
        resolver: ResolverTables::default(),
        current_def_ids: std::collections::BTreeSet::new(),
        items: DeclarationItems::from_id_maps(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
        .unwrap(),
        function_type_vars: HashMap::new(),
        infix_precedence: HashMap::new(),
        infix_owners: HashMap::new(),
        loaded_module_paths: Vec::new(),
        type_vars,
        inject_prelude: false,
        loaded_prelude_export_ids: HashMap::new(),
        module_file_cache: HashMap::new(),
        source_modules: crate::source_loader::SourceModuleSet::default(),
        dependency_root_export_ids: HashMap::new(),
        source_map: Default::default(),
        language_items: Default::default(),
    };

    let mut lowerer = Lowerer::from_declarations(decls).unwrap();

    assert_eq!(
        lowerer.engine.fresh_type_var(),
        Type::TypeVar(crate::ids::TypeVarId(2))
    );
}

impl LowerItems {
    pub(crate) fn insert_trait_def(&mut self, trait_def: HirTrait) -> Option<HirTrait> {
        self.declarations.insert_trait_def(trait_def)
    }

    pub(crate) fn insert_structure(&mut self, structure: HirStruct) -> Option<HirStruct> {
        self.declarations.insert_structure(structure)
    }

    pub(crate) fn insert_enumeration(&mut self, enumeration: HirEnum) -> Option<HirEnum> {
        self.declarations.insert_enumeration(enumeration)
    }

    pub(crate) fn insert_extern(&mut self, extern_def: HirExtern) -> Option<HirExtern> {
        self.declarations.insert_extern(extern_def)
    }

    pub(crate) fn function_mut(&mut self, id: DefId) -> Option<&mut HirFunction> {
        self.declarations.function_mut(id)
    }

    pub(crate) fn structure_mut(&mut self, id: DefId) -> Option<&mut HirStruct> {
        self.declarations.structure_mut(id)
    }

    pub(crate) fn extern_mut(&mut self, id: DefId) -> Option<&mut HirExtern> {
        self.declarations.extern_mut(id)
    }

    pub(crate) fn enumeration_mut(&mut self, id: DefId) -> Option<&mut HirEnum> {
        self.declarations.enumeration_mut(id)
    }
}
