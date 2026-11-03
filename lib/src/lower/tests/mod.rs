use super::*;
use crate::ast::{Ident, IdentOrType, IdentifierPath, Program};
use crate::collect::item_index::{IndexingIds, ItemIndex};
use crate::collect::{DeclarationItems, DeclarationTypeVars, Declarations};
use crate::crate_artifact::ArtifactExport;
use crate::crate_system::CrateContext;
use crate::ids::{DefId, LocalDefId, ModuleId};

mod declarations;
mod imports;
mod modules;

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: crate::lexer::Span::test(),
    }
}

fn test_function(id: DefId, name: &str, stmts: Vec<HirStmt>) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::I64,
        body: HirBlock {
            stmts,
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn declaration_items(
    functions: HashMap<DefId, HirFunction>,
    externs: HashMap<DefId, HirExtern>,
) -> DeclarationItems {
    DeclarationItems::from_id_maps(
        functions,
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        externs,
    )
    .unwrap()
}
