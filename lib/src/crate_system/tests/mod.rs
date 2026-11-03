use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::ast::Module;
use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::{ArtifactCrateInterface, ArtifactCrossCrateHir, ArtifactExport};
use crate::hir::{
    AcceptedHirBlock as HirBlock, AcceptedHirFunction as HirFunction, AcceptedHirImpl as HirImpl,
    AcceptedHirTrait as HirTrait, HirImplOwner,
};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::types::{GenericParamDecl, GenericParamId, Type};
use crate::SourceProvider;

use super::*;

mod compilation_order;
mod context;
mod extern_artifact_loading;
mod extern_crate_ref;
mod extern_store;
mod source_loading;

fn hir_function_with_id(
    id: DefId,
    name: &str,
    _qualified_name: Option<&str>,
    generic_params: &[&str],
) -> HirFunction {
    let generic_params = generic_params
        .iter()
        .enumerate()
        .map(|(index, name)| {
            GenericParamDecl::type_param(
                GenericParamId {
                    owner: id,
                    index: index as u32,
                },
                *name,
            )
        })
        .collect();

    HirFunction {
        id,
        name: name.to_string(),
        generic_params,
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
