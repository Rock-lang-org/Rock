//! Monomorphization pass
//!
//! Specializes generic functions and types by creating concrete versions
//! for each set of type arguments used in the program.

mod hir_types {
    pub type HirProgram = crate::hir::HirProgramFor<crate::hir::AcceptedHir>;
    pub type HirFunction = crate::hir::HirFunctionFor<crate::hir::AcceptedHir>;
    pub type HirImpl = crate::hir::HirImplFor<crate::hir::AcceptedHir>;
    pub type HirBlock = crate::hir::HirBlockFor<crate::hir::AcceptedHir>;
    pub type HirStmt = crate::hir::HirStmtFor<crate::hir::AcceptedHir>;
    pub type HirExpr = crate::hir::HirExprFor<crate::hir::AcceptedHir>;
    pub type HirExprKind = crate::hir::HirExprKindFor<crate::hir::AcceptedHir>;
    pub type HirMatchArm = crate::hir::HirMatchArmFor<crate::hir::AcceptedHir>;
    pub type HirStructLiteralField = crate::hir::HirStructLiteralFieldFor<crate::hir::AcceptedHir>;

    pub(crate) use crate::hir::{
        function_requires_downstream_specialization, hir_function_is_codegen_concrete,
        impl_requires_downstream_specialization, HirCallTarget, HirImplOwner,
        HirImplReceiverPattern, HirMethodCallTarget, HirParam, HirPattern, HirSelectedMethodTarget,
        HirStructPatternField, HirVarRef, HirVarTarget, HirVariantFields,
    };
}

mod array_lengths;
mod body_edges;
mod callable;
mod diagnostic_context;
mod drop_fields;
mod error;
mod external;
mod identity_context;
mod impl_lookup;
mod methods;
mod process;
mod registration;
mod registry;
mod specialize;
mod substitute;
mod symbol_identity;
mod type_interning;

use crate::collect::resolver::ResolverTables;
use crate::crate_system::CrateContext;
use crate::diagnostic::Diagnostics;
use crate::hir::AcceptedHirProgram;
use crate::ids::{DefId, HirLocalId, TypeId};
use crate::infer::ResolvedHirProgram;
use crate::source_map::SemanticSourceMap;
use crate::type_context::TypeContext;
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::Type;
use hir_types::*;
use std::collections::{HashMap, HashSet};

pub use crate::ids::InstanceId;
pub use error::{MonoError, MonoErrorKind};
pub(crate) use identity_context::CompilationIdentityContext;
pub use registry::{
    GeneratedMethodInstance, InstanceImplOwner, InstanceKey, InstanceOrigin, InstanceRecord,
    InstanceRegistry, InstanceSymbols, MonomorphizedProgram, PreMirInstanceBodies,
    PreMirInstanceBody,
};

/// Monomorphize a program with its session source metadata.
///
/// Source locations are intentionally an explicit input so production callers
/// cannot silently discard binding and declaration spans before MIR lowering.
pub fn monomorphize(
    program: AcceptedHirProgram,
    source_map: SemanticSourceMap,
) -> Result<MonomorphizedProgram, Diagnostics> {
    let mut mono = Monomorphizer::new().with_source_map(source_map);
    let program = mono.process(program.into_program());
    mono.check_instance_array_lengths();
    mono.diagnostics.return_if_err()?;
    let (instances, pre_mir_instance_bodies) = mono.instances.into_parts();
    Ok(MonomorphizedProgram {
        program,
        instances,
        pre_mir_instance_bodies,
        generated_drop_instances: mono.generated_drop_instances,
        type_context: mono.type_context,
        source_map: mono.source_map,
    })
}

/// Monomorphize a program with external crate support
///
/// This function accepts a CrateContext containing loaded external crates and
/// monomorphizes generic functions from those crates when they are used.
///
/// # Arguments
/// * `program` - The HIR program to monomorphize
/// * `crate_ctx` - The crate context with loaded external crates
///
/// # Returns
/// The monomorphized program wrapper with specialized generic functions
pub(crate) fn monomorphize_with_crates(
    program: ResolvedHirProgram,
    crate_ctx: &CrateContext,
    identity_context: CompilationIdentityContext,
) -> Result<MonomorphizedProgram, Diagnostics> {
    let ResolvedHirProgram {
        program,
        resolver,
        type_context,
        normalization_env,
        source_map,
        ..
    } = program;
    let mut mono = Monomorphizer::with_type_context(type_context);
    mono.source_map = source_map;
    mono.normalization_env = normalization_env;
    mono.identity_context = identity_context;
    mono.resolver = resolver;
    let program = mono.process_with_crates(program.into_program(), crate_ctx);
    mono.check_instance_array_lengths();
    mono.diagnostics.return_if_err()?;
    let (instances, pre_mir_instance_bodies) = mono.instances.into_parts();
    Ok(MonomorphizedProgram {
        program,
        instances,
        pre_mir_instance_bodies,
        generated_drop_instances: mono.generated_drop_instances,
        type_context: mono.type_context,
        source_map: mono.source_map,
    })
}

struct Monomorphizer {
    language_items: crate::hir::HirLanguageItems,
    /// Concrete function payloads, selected exclusively by canonical identity.
    concrete_functions: HashMap<DefId, HirFunction>,
    /// Generic function payloads, including external providers, keyed by canonical identity.
    generic_functions: HashMap<DefId, HirFunction>,
    /// Source/display names for function symbols after their DefId has been selected.
    function_names_by_id: HashMap<DefId, String>,
    /// Registry of concrete instances.
    instances: InstanceRegistry,
    /// Intern table for canonical type identities used at mono boundaries.
    type_context: TypeContext,
    /// Canonicalization environment carried from inference into specialization.
    normalization_env: TypeNormalizationEnv,
    /// Stable product identities used for backend symbols and fingerprints.
    identity_context: CompilationIdentityContext,
    /// Resolver tables used for canonical instance identity.
    resolver: ResolverTables,
    /// Resolver tables loaded from dependency crates.
    dependency_resolvers: Vec<ResolverTables>,
    /// Counter for generating unique names
    counter: u32,
    /// Current impl being processed (for method calls)
    /// Current type arguments for the impl being processed
    current_type_args: Vec<TypeId>,
    /// Trait implementations keyed by canonical trait identity.
    trait_impls: HashMap<DefId, Vec<HirImpl>>,
    /// Concrete implementation methods keyed by `(impl, trait member)`.
    ///
    /// This is copied from HIR after conformance has injected default methods.
    /// Mono must consume this identity relationship instead of rediscovering a
    /// method through its display name.
    effective_trait_methods: HashMap<(DefId, DefId), DefId>,
    /// Standalone generic impls keyed by canonical impl identity.
    generic_impls: HashMap<DefId, HirImpl>,
    /// Current local types keyed by canonical HIR local identity.
    var_types: HashMap<HirLocalId, TypeId>,
    /// Nominal field types keyed by canonical struct/enum identity for recursive drop glue.
    nominal_field_types: HashMap<DefId, Vec<Type>>,
    /// Source spans for nominal fields used while materializing recursive drop glue.
    nominal_field_spans: HashMap<DefId, Vec<Option<crate::lexer::Span>>>,
    /// Drop impl instance keys currently being specialized for automatic drop glue.
    drop_monomorphization_in_progress: HashSet<(DefId, Vec<TypeId>)>,
    /// Nominal types currently being expanded for field drop specialization.
    drop_field_monomorphization_in_progress: HashSet<TypeId>,
    /// Canonical Drop trait identity from the effective language item registry.
    drop_trait_id: Option<DefId>,
    /// Canonical Drop member identity from the effective language item registry.
    drop_method_id: Option<DefId>,
    /// Exact Drop selections generated by mono, keyed by the concrete receiver type.
    generated_drop_instances: std::collections::BTreeMap<TypeId, GeneratedMethodInstance>,
    /// Selection failures discovered while monomorphizing generated method edges.
    diagnostics: Diagnostics,
    /// Session-only source locations consumed by MIR and diagnostics.
    source_map: SemanticSourceMap,
    current_function_owner: Option<DefId>,
}

impl Monomorphizer {
    fn new() -> Self {
        Self {
            language_items: crate::hir::HirLanguageItems::default(),
            concrete_functions: HashMap::new(),
            generic_functions: HashMap::new(),
            function_names_by_id: HashMap::new(),
            var_types: HashMap::new(),
            nominal_field_types: HashMap::new(),
            nominal_field_spans: HashMap::new(),
            instances: InstanceRegistry::new(),
            type_context: TypeContext::new(),
            normalization_env: TypeNormalizationEnv::new(),
            identity_context: CompilationIdentityContext::local_default(),
            resolver: ResolverTables::default(),
            dependency_resolvers: Vec::new(),
            counter: 0,
            current_type_args: Vec::new(),
            trait_impls: HashMap::new(),
            effective_trait_methods: HashMap::new(),
            generic_impls: HashMap::new(),
            drop_monomorphization_in_progress: HashSet::new(),
            drop_field_monomorphization_in_progress: HashSet::new(),
            drop_trait_id: None,
            drop_method_id: None,
            generated_drop_instances: std::collections::BTreeMap::new(),
            diagnostics: Diagnostics::default(),
            source_map: SemanticSourceMap::default(),
            current_function_owner: None,
        }
    }

    fn with_source_map(mut self, source_map: SemanticSourceMap) -> Self {
        self.source_map = source_map;
        self
    }

    fn with_type_context(type_context: TypeContext) -> Self {
        Self {
            type_context,
            ..Self::new()
        }
    }
}

#[cfg(test)]
mod tests;
