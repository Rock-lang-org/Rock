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
