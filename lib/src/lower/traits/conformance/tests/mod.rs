pub(crate) use super::service::{
    IndexProtocolIds, TraitConformanceContext, TraitConformanceService,
};
use crate::collect::resolver::ResolverTables;
use crate::hir::{
    HirAssociatedTypeDecl, HirAssociatedTypeDef, HirBlock, HirClosureCapture,
    HirClosureCaptureKind, HirEnum, HirExpr, HirExprKind, HirFunction, HirFunctionSig, HirImpl,
    HirImplOwner, HirImplReceiverPattern, HirMatchArm, HirMethodCallTarget, HirParam, HirPattern,
    HirStmt, HirStruct, HirStructPatternField, HirTrait,
};
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::types::ReceiverMode;
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, TraitBound, Type};
use std::collections::HashMap;
use std::path::PathBuf;

mod auto_impl_sized;
mod default_methods;
mod default_substitution;
mod projections;
mod rejections;
mod service;
mod substitution;

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn empty_trait(id: DefId, name: &str) -> HirTrait {
    HirTrait {
        target: None,
        predicates: Vec::new(),
        id,
        name: name.to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::new(),
    }
}

fn point_struct(id: DefId) -> HirStruct {
    HirStruct {
        id,
        name: "Point".to_string(),
        generic_params: vec![],
        fields: vec![],
    }
}

fn register_point(lowerer: &mut Lowerer, point_id: DefId) {
    lowerer.items.insert_structure(point_struct(point_id));
    lowerer
        .resolver
        .item_paths
        .insert("Point".to_string(), point_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(point_id, "Point".to_string());
    lowerer.current_def_ids.insert(point_id);
}
