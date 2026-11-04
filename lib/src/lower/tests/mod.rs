use super::*;
use crate::ast::{Ident, IdentOrType, IdentifierPath, Program};
use crate::collect::item_index::{IndexingIds, ItemIndex};
use crate::collect::resolver::ResolverTables;
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

impl Lowerer {
    pub(crate) fn resolve_item_def_id(&self, name: &str) -> Option<DefId> {
        crate::lower::resolution::LowerResolutionContext::new(self).resolve_item_id(name)
    }

    pub(crate) fn canonical_name_for_alias_or_item(&self, name: &str) -> Option<String> {
        crate::lower::resolution::LowerResolutionContext::new(self).canonical_name_for_item(name)
    }
}

impl Lowerer {
    pub(crate) fn new_for_test() -> Self {
        let lowerer = Self::new();
        lowerer
    }

    pub(crate) fn with_test_body_context<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.with_body_context(
            BodyLoweringContext::new(
                BodyOwner::Function(DefId::new(CrateId(0), LocalDefId(u32::MAX - 1))),
                self.current_generic_owner(),
                self.current_generic_params().to_vec(),
                self.current_impl_bounds().clone(),
                false,
            ),
            f,
        )
    }
}
