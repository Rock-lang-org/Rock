use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::ArtifactExport;
use crate::hir::{
    HirBlock, HirEnum, HirExtern, HirFunction, HirImpl, HirImplOwner, HirImplReceiverPattern,
    HirParam, HirStruct, HirTrait, HirVarTarget,
};
use crate::ids::{CrateId, DefId, HirLocalId, LocalDefId};
use crate::lower::prelude::PreludeImports;
use crate::lower::resolution::{LowerImportTarget, LowerResolutionContext};
use crate::lower::Lowerer;
use crate::types::Type;
use std::collections::HashMap;

mod nominals;
mod values;

fn def_id(crate_id: u32, local_id: u32) -> DefId {
    DefId::new(CrateId(crate_id), LocalDefId(local_id))
}

fn test_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::<HirParam>::new(),
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

impl<'a> LowerResolutionContext<'a> {
    pub(crate) fn canonical_type_name_for_method_lookup(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Struct { id, .. } | Type::Enum { id, .. } => self
                .canonical_name(*id)
                .map(str::to_string)
                .or_else(|| crate::lower::Lowerer::get_type_name_for_method_lookup(ty)),
            Type::Reference { inner, .. }
                if matches!(inner.as_ref(), Type::Slice(_) | Type::Str) =>
            {
                Some(ty.to_string())
            }
            Type::Reference { inner, .. } => self.canonical_type_name_for_method_lookup(inner),
            _ => crate::lower::Lowerer::get_type_name_for_method_lookup(ty),
        }
    }
}
