use std::collections::HashMap;

use crate::hir::{
    AcceptedHirProgram, HirAssociatedTypeDecl, HirAssociatedTypeDef, HirAssociatedTypeLocation,
    HirBlock, HirCallTarget, HirEnum, HirExpr, HirExprKind, HirExtern, HirField, HirFieldLocation,
    HirFunction, HirImpl, HirImplOwner, HirImplReceiverPattern, HirMethodCallTarget,
    HirMethodLocation, HirNameTables, HirParam, HirPattern, HirProgram, HirSelectedMethodTarget,
    HirSelectedTraitMember, HirStmt, HirStruct, HirStructPatternField, HirTrait,
    HirTraitDispatchKind, HirTypeBinding, HirVarRef, HirVarTarget, HirVariant, HirVariantFields,
    HirVariantLocation,
};
use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
use crate::types::{GenericParamDecl, GenericParamId, Type};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn field_id(index: u32) -> FieldId {
    FieldId(index)
}

fn variant_id(index: u32) -> VariantId {
    VariantId(index)
}

fn assoc_type_id(index: u32) -> AssocTypeId {
    AssocTypeId(index)
}

fn empty_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn test_function(id: DefId, name: &str) -> HirFunction {
    empty_function(id, name)
}

fn empty_struct(id: DefId, name: &str) -> HirStruct {
    HirStruct {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        fields: vec![HirField {
            id: field_id(0),
            name: "value".to_string(),
            ty: Type::I32,
            public: true,
        }],
    }
}

fn empty_enum(id: DefId, name: &str) -> HirEnum {
    HirEnum {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        variants: Vec::new(),
    }
}

fn trait_with_default_method(id: DefId, name: &str, method_id: DefId) -> HirTrait {
    HirTrait {
        target: None,
        predicates: Vec::new(),
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([(
            "default_method".to_string(),
            empty_function(method_id, "default_method"),
        )]),
        signatures: HashMap::new(),
    }
}

fn impl_with_method(id: DefId, method_id: DefId) -> HirImpl {
    HirImpl {
        id,
        owner: HirImplOwner::Named("Thing".to_string()),
        type_name: "Thing".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: vec![HirAssociatedTypeDef {
            id: assoc_type_id(0),
            name: "Item".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::I32,
        }],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "impl_method".to_string(),
            empty_function(method_id, "impl_method"),
        )]),
    }
}

fn program(
    functions: HashMap<DefId, HirFunction>,
    structs: HashMap<DefId, HirStruct>,
    enums: HashMap<DefId, HirEnum>,
    traits: HashMap<DefId, HirTrait>,
    impls: HashMap<DefId, HirImpl>,
    externs: HashMap<DefId, HirExtern>,
    names: HirNameTables,
    canonical_names: HashMap<DefId, String>,
) -> HirProgram {
    HirProgram::from_id_parts_with_names_and_canonical_names(
        functions,
        structs,
        enums,
        traits,
        impls,
        externs,
        names,
        crate::hir::HirLanguageItems::default(),
        &canonical_names,
    )
}

mod acceptance;
mod aliases;
mod indexes;
mod rebuild;
mod targets;
