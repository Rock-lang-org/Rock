//! Type inference finalization — converts PartialHir (with type variables)
//! into a fully-typed HirProgram.
mod authority;
pub(crate) mod constants;
pub mod constraints;
mod engine;
mod finalize;
mod generalize;
mod helpers;
pub mod solve;
mod type_vars;

pub use constraints::{ConstraintOwner, ConstraintStore, ObligationId, ObligationState};
pub use engine::{InferenceEngine, InferenceError, UnifyError};
pub use generalize::{generalize_single_function, generalize_single_function_with_exclusions};
mod entry;
mod partial_to_program;
mod quiescence;
mod validation;

pub use entry::finalize;
pub(crate) use entry::finalize_with_crates;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use crate::collect::resolver::ResolverTables;
use crate::hir::{
    collect_hir_type_ids, AcceptedHirProgram, HirEnum, HirExtern, HirFunction, HirImpl,
    HirLanguageItems, HirStruct, HirTrait, HirTypeAlias, HirTypeIds, HirTypeLocation,
};
use crate::ids::{DefId, IdGen, LocalDefId, TypeId, TypeVarId};
use crate::lower::ResolveError;
use crate::type_context::TypeContext;
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::Type;

#[derive(Debug)]
pub struct ResolvedHirProgram {
    pub program: AcceptedHirProgram,
    pub resolver: ResolverTables,
    pub current_def_ids: BTreeSet<DefId>,
    pub root_crate_id: crate::ids::CrateId,
    pub local_def_ids: IdGen<LocalDefId>,
    pub type_context: TypeContext,
    pub type_ids: HirTypeIds,
    pub source_map: crate::source_map::SemanticSourceMap,
    pub(crate) normalization_env: TypeNormalizationEnv,
    pub(crate) constant_arithmetic: HashMap<DefId, constants::ArithmeticSummary>,
}

impl ResolvedHirProgram {
    #[cfg(test)]
    pub fn new(
        program: HirProgram,
        resolver: ResolverTables,
        current_def_ids: BTreeSet<DefId>,
        root_crate_id: crate::ids::CrateId,
        local_def_ids: IdGen<LocalDefId>,
    ) -> Self {
        let program = AcceptedHirProgram::try_from(program)
            .expect("ResolvedHirProgram requires validated method authority");
        Self::from_accepted(
            program,
            resolver,
            current_def_ids,
            root_crate_id,
            local_def_ids,
        )
    }

    pub fn from_accepted(
        program: AcceptedHirProgram,
        resolver: ResolverTables,
        current_def_ids: BTreeSet<DefId>,
        root_crate_id: crate::ids::CrateId,
        local_def_ids: IdGen<LocalDefId>,
    ) -> Self {
        let summaries = constants::method_summaries(program.program(), HashMap::new());
        Self::from_accepted_with_normalization_env(
            program,
            resolver,
            current_def_ids,
            root_crate_id,
            local_def_ids,
            TypeNormalizationEnv::new(),
            crate::source_map::SemanticSourceMap::default(),
            summaries,
        )
    }

    fn from_accepted_with_normalization_env(
        program: AcceptedHirProgram,
        resolver: ResolverTables,
        current_def_ids: BTreeSet<DefId>,
        root_crate_id: crate::ids::CrateId,
        local_def_ids: IdGen<LocalDefId>,
        normalization_env: TypeNormalizationEnv,
        source_map: crate::source_map::SemanticSourceMap,
        constant_arithmetic: HashMap<DefId, constants::ArithmeticSummary>,
    ) -> Self {
        let mut type_context = TypeContext::with_normalization_env(normalization_env.clone());
        let type_ids = collect_hir_type_ids(program.program(), &mut type_context);
        Self {
            program,
            resolver,
            current_def_ids,
            root_crate_id,
            local_def_ids,
            type_context,
            type_ids,
            source_map,
            normalization_env,
            constant_arithmetic,
        }
    }

    pub fn type_id_at(&self, location: &HirTypeLocation) -> Option<TypeId> {
        self.type_ids.get(location)
    }

    pub fn require_type_id_at(&self, location: HirTypeLocation) -> Result<TypeId, ResolveError> {
        self.type_id_at(&location).ok_or_else(|| {
            ResolveError::non_source(format!("missing finalized HIR TypeId at {:?}", location))
        })
    }

    pub fn type_at(&self, id: TypeId) -> Type {
        self.type_context.type_for(id)
    }

    pub fn type_display_context(&self) -> crate::type_services::display::TypeDisplayContext {
        let mut context =
            crate::type_services::display::TypeDisplayContext::from_resolver(&self.resolver);
        context.extend_hir_program(self.program.program());
        context
    }

    pub fn display_type(&self, ty: &Type) -> String {
        crate::type_services::display::display_type_with_context(ty, &self.type_display_context())
            .to_string()
    }
}

/// HIR with type variables still present — output of the lower stage.
pub struct PartialHir {
    pub functions: HashMap<DefId, HirFunction>,
    pub structs: HashMap<DefId, HirStruct>,
    pub enums: HashMap<DefId, HirEnum>,
    pub traits: HashMap<DefId, HirTrait>,
    pub impls: HashMap<DefId, HirImpl>,
    pub externs: HashMap<DefId, HirExtern>,
    pub type_aliases: HashMap<DefId, HirTypeAlias>,
    pub engine: InferenceEngine,
    pub function_type_vars: HashMap<DefId, HashSet<TypeVarId>>,
    pub import_aliases: HashMap<String, String>,
    pub loaded_module_paths: Vec<(String, PathBuf)>,
    pub constraint_store: ConstraintStore,
    pub resolver: ResolverTables,
    pub current_def_ids: BTreeSet<DefId>,
    pub root_crate_id: crate::ids::CrateId,
    pub local_def_ids: IdGen<LocalDefId>,
    pub language_items: HirLanguageItems,
    pub imported_effective_trait_methods: HashMap<(DefId, DefId), DefId>,
    /// Function-to-SCC membership computed before body lowering.
    pub inference_sccs: HashMap<DefId, DefId>,
    /// Dependency-first SCC order, including isolated components.
    pub inference_scc_order: Vec<DefId>,
    pub source_map: crate::source_map::SemanticSourceMap,
}

#[cfg(test)]
use crate::hir::HirProgram;
#[cfg(test)]
use crate::ids::CrateId;
#[cfg(test)]
mod tests;
