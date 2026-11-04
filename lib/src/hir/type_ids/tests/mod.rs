use std::collections::HashMap;

pub(crate) use super::body_types::collect_pattern_type_ids;
use super::*;
use crate::hir::{
    HirAssociatedTypeDef, HirBlock, HirEnum, HirExpr, HirExprKind, HirExtern, HirField,
    HirFunction, HirFunctionSig, HirImpl, HirImplOwner, HirMethodCallTarget, HirNameTables,
    HirParam, HirPattern, HirProgram, HirStmt, HirStruct, HirTrait, HirVariant, HirVariantFields,
};
use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
use crate::type_context::TypeContext;
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, TraitBound, Type};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn generic(owner: DefId, index: u32) -> GenericParamId {
    GenericParamId { owner, index }
}

fn empty_body(ty: Type) -> HirBlock {
    HirBlock {
        stmts: Vec::new(),
        ty,
    }
}

fn function(id: DefId, name: &str, param_ty: Type, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: param_ty,
            mutable: false,
            is_ref: false,
        }],
        ret_type: ret_type.clone(),
        body: empty_body(ret_type),
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
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

mod bodies;
mod top_level;
