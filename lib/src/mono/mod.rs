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

mod callable;
mod external;
mod methods;
mod process;
mod registry;
mod specialize;
mod substitute;

use crate::collect::resolver::ResolverTables;
use crate::crate_system::CrateContext;
use crate::diagnostic::{DiagnosticCode, Diagnostics};
use crate::hir::{self, AcceptedHirProgram};
use crate::ids::{DefId, HirLocalId, TypeId};
use crate::infer::ResolvedHirProgram;
use crate::products::ProductCrateIdentity;
use crate::source_map::{SemanticSourceMap, SourceSymbol};
use crate::type_context::{Ty, TypeContext};
use crate::type_services::normalize::{TypeNormalizationEnv, TypeNormalizer};
use crate::types::{GenericParamId, Type};
use hir_types::*;
use std::collections::{BTreeMap, HashMap, HashSet};

pub use crate::ids::InstanceId;
pub use registry::{
    GeneratedMethodInstance, InstanceImplOwner, InstanceKey, InstanceOrigin, InstanceRecord,
    InstanceRegistry, InstanceSymbols, MonomorphizedProgram, PreMirInstanceBodies,
    PreMirInstanceBody,
};

#[derive(Debug, Clone)]
pub(crate) struct CompilationIdentityContext {
    current_crate_id: crate::ids::CrateId,
    current_crate: ProductCrateIdentity,
    dependencies: BTreeMap<crate::ids::CrateId, ProductCrateIdentity>,
}

impl CompilationIdentityContext {
    pub(crate) fn new(
        current_crate_id: crate::ids::CrateId,
        current_crate: ProductCrateIdentity,
        dependencies: BTreeMap<crate::ids::CrateId, ProductCrateIdentity>,
    ) -> Self {
        Self {
            current_crate_id,
            current_crate,
            dependencies,
        }
    }

    fn local_default() -> Self {
        Self::new(
            crate::ids::CrateId(0),
            ProductCrateIdentity::local("local".to_string()),
            BTreeMap::new(),
        )
    }

    fn crate_identity(&self, crate_id: crate::ids::CrateId) -> &ProductCrateIdentity {
        if crate_id == self.current_crate_id {
            return &self.current_crate;
        }
        self.dependencies
            .get(&crate_id)
            .unwrap_or_else(|| panic!("missing product identity for session crate {crate_id:?}"))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MonoErrorKind {
    MissingImpl {
        impl_id: DefId,
    },
    MissingMethod {
        impl_id: DefId,
        method_id: DefId,
    },
    ReceiverMismatch {
        impl_id: DefId,
        receiver: Type,
        pattern: Option<HirImplReceiverPattern>,
    },
    InvalidBindings {
        target: HirSelectedMethodTarget,
    },
    TraitArgsMismatch {
        impl_id: DefId,
        expected: Vec<Type>,
        selected: Vec<Type>,
    },
    NoMatchingImpl {
        trait_id: DefId,
        member_id: DefId,
    },
    AmbiguousImpls {
        trait_id: DefId,
        member_id: DefId,
        impls: Vec<DefId>,
    },
    MissingEffectiveMethod {
        impl_id: DefId,
        member_id: DefId,
    },
    MissingInstance {
        origin: InstanceOrigin,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MonoError {
    pub kind: MonoErrorKind,
    pub span: crate::lexer::Span,
}

impl MonoError {
    #[cfg(test)]
    fn diagnostic(self, resolver: &ResolverTables) -> crate::diagnostic::Diagnostic {
        let context = crate::type_services::display::TypeDisplayContext::from_resolver(resolver);
        let display = |ty: &Type| {
            crate::type_services::display::display_type_with_context(ty, &context).to_string()
        };
        let message = match self.kind {
            MonoErrorKind::MissingImpl { .. } => {
                "selected implementation is unavailable during specialization".to_string()
            }
            MonoErrorKind::MissingMethod { .. } => {
                "selected method is unavailable during specialization".to_string()
            }
            MonoErrorKind::ReceiverMismatch {
                receiver, pattern, ..
            } => match pattern {
                Some(HirImplReceiverPattern::Exact(expected))
                | Some(HirImplReceiverPattern::Constructor(expected)) => format!(
                    "method receiver type mismatch: expected {}, found {}",
                    display(&expected),
                    display(&receiver)
                ),
                Some(HirImplReceiverPattern::SliceFamily { element }) => format!(
                    "method receiver type mismatch: expected a slice of {}, found {}",
                    display(&element),
                    display(&receiver)
                ),
                None => format!("method receiver type mismatch for {}", display(&receiver)),
            },
            MonoErrorKind::InvalidBindings { .. } => {
                "method has invalid generic arguments during specialization".to_string()
            }
            MonoErrorKind::TraitArgsMismatch {
                expected, selected, ..
            } => format!(
                "trait argument mismatch: expected {}, found {}",
                display_type_list(&expected, &display),
                display_type_list(&selected, &display)
            ),
            MonoErrorKind::NoMatchingImpl { .. } => {
                "no matching trait implementation was found during specialization".to_string()
            }
            MonoErrorKind::AmbiguousImpls { .. } => {
                "multiple trait implementations match during specialization".to_string()
            }
            MonoErrorKind::MissingEffectiveMethod { .. } => {
                "trait implementation does not provide the selected method".to_string()
            }
            MonoErrorKind::MissingInstance { .. } => {
                "required callable instance is unavailable during specialization".to_string()
            }
        };
        crate::diagnostic::Diagnostic::new(message, self.span).with_code(DiagnosticCode::Mono)
    }

    fn diagnostic_with_context(
        self,
        context: &crate::type_services::display::TypeDisplayContext,
    ) -> crate::diagnostic::Diagnostic {
        let display = |ty: &Type| {
            crate::type_services::display::display_type_with_context(ty, context).to_string()
        };
        let message = match self.kind {
            MonoErrorKind::MissingImpl { .. } => {
                "selected implementation is unavailable during specialization".to_string()
            }
            MonoErrorKind::MissingMethod { .. } => {
                "selected method is unavailable during specialization".to_string()
            }
            MonoErrorKind::ReceiverMismatch {
                receiver, pattern, ..
            } => match pattern {
                Some(HirImplReceiverPattern::Exact(expected))
                | Some(HirImplReceiverPattern::Constructor(expected)) => format!(
                    "method receiver type mismatch: expected {}, found {}",
                    display(&expected),
                    display(&receiver)
                ),
                Some(HirImplReceiverPattern::SliceFamily { element }) => format!(
                    "method receiver type mismatch: expected a slice of {}, found {}",
                    display(&element),
                    display(&receiver)
                ),
                None => format!("method receiver type mismatch for {}", display(&receiver)),
            },
            MonoErrorKind::InvalidBindings { .. } => {
                "method has invalid generic arguments during specialization".to_string()
            }
            MonoErrorKind::TraitArgsMismatch {
                expected, selected, ..
            } => format!(
                "trait argument mismatch: expected {}, found {}",
                display_type_list(&expected, &display),
                display_type_list(&selected, &display)
            ),
            MonoErrorKind::NoMatchingImpl { .. } => {
                "no matching trait implementation was found during specialization".to_string()
            }
            MonoErrorKind::AmbiguousImpls { .. } => {
                "multiple trait implementations match during specialization".to_string()
            }
            MonoErrorKind::MissingEffectiveMethod { .. } => {
                "trait implementation does not provide the selected method".to_string()
            }
            MonoErrorKind::MissingInstance { .. } => {
                "required callable instance is unavailable during specialization".to_string()
            }
        };
        crate::diagnostic::Diagnostic::new(message, self.span).with_code(DiagnosticCode::Mono)
    }
}

fn display_type_list(types: &[Type], display: &impl Fn(&Type) -> String) -> String {
    types.iter().map(display).collect::<Vec<_>>().join(", ")
}

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

#[cfg(test)]
pub(super) fn monomorphize_for_tests(
    program: AcceptedHirProgram,
) -> Result<MonomorphizedProgram, Diagnostics> {
    monomorphize(program, SemanticSourceMap::default())
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
    fn diagnostic_context(&self) -> crate::type_services::display::TypeDisplayContext {
        let mut context =
            crate::type_services::display::TypeDisplayContext::from_resolver(&self.resolver);
        for resolver in &self.dependency_resolvers {
            context.extend_resolver(resolver);
        }
        for function in self
            .concrete_functions
            .values()
            .chain(self.generic_functions.values())
        {
            context.insert_generic_names(&function.generic_params);
        }
        let mut extend_impl = |impl_def: &HirImpl| {
            context.insert_generic_names(&impl_def.type_generics);
            context.insert_generic_names(&impl_def.trait_generics);
            for associated in &impl_def.associated_types {
                for owner in [Some(impl_def.id), impl_def.trait_id] {
                    if let Some(owner) = owner {
                        context.insert_associated_name(
                            crate::types::AssociatedTypeKey {
                                owner,
                                assoc_type_id: associated.id,
                            },
                            associated.name.clone(),
                        );
                    }
                }
            }
            for method in impl_def.methods.values() {
                context.insert_generic_names(&method.generic_params);
            }
        };
        for impls in self.trait_impls.values() {
            for impl_def in impls {
                extend_impl(impl_def);
            }
        }
        for impl_def in self.generic_impls.values() {
            extend_impl(impl_def);
        }
        context
    }

    fn display_type_for_diagnostic(&self, ty: &Type) -> String {
        let context = self.diagnostic_context();
        crate::type_services::display::display_type_with_context(ty, &context).to_string()
    }

    fn source_span_for_type(&self, ty: &Type) -> Option<crate::lexer::Span> {
        let symbol = match ty {
            Type::Struct { id, .. } | Type::Enum { id, .. } | Type::Constructor { id, .. } => {
                SourceSymbol::Definition(*id)
            }
            Type::Generic(param) => SourceSymbol::Generic(*param),
            Type::Projection { assoc_type, .. } => SourceSymbol::AssociatedType {
                owner: assoc_type.owner,
                associated: assoc_type.assoc_type_id,
            },
            _ => return None,
        };
        self.source_map.symbol(&symbol).map(|info| {
            info.declaration_span
                .as_ref()
                .unwrap_or(&info.name_span)
                .clone()
        })
    }

    fn intern_type(&mut self, ty: &Type) -> crate::ids::TypeId {
        let normalized = self.normalize_type(ty);
        self.type_context
            .intern_normalized_type(&normalized, &self.normalization_env)
            .expect("mono type normalization must preserve accepted HIR invariants")
    }

    fn normalize_type(&self, ty: &Type) -> Type {
        TypeNormalizer::new(&self.normalization_env)
            .normalize(ty)
            .expect("mono type normalization must preserve accepted HIR invariants")
    }

    #[allow(dead_code)]
    fn type_for(&self, id: crate::ids::TypeId) -> Type {
        self.type_context.type_for(id)
    }

    #[cfg(test)]
    fn intern_types(&mut self, types: &[Type]) -> Vec<crate::ids::TypeId> {
        types.iter().map(|ty| self.intern_type(ty)).collect()
    }

    #[cfg(test)]
    fn register_test_dependency_identity(&mut self, crate_id: crate::ids::CrateId) {
        self.identity_context.dependencies.insert(
            crate_id,
            ProductCrateIdentity::local("test-dependency".to_string()),
        );
    }

    fn hash_canonical(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }

    fn push_text(output: &mut String, text: &str) {
        output.push_str(&text.len().to_string());
        output.push(':');
        output.push_str(text);
    }

    fn write_crate_identity(output: &mut String, identity: &ProductCrateIdentity) {
        Self::push_text(output, &identity.name);
        Self::push_text(output, &identity.version);
        Self::push_text(output, identity.target_triple.as_deref().unwrap_or(""));
        Self::push_text(output, &identity.format_version.to_string());
        Self::push_text(
            output,
            identity
                .source_fingerprint
                .manifest_hash
                .as_deref()
                .unwrap_or(""),
        );
        Self::push_text(
            output,
            identity
                .source_fingerprint
                .source_hash
                .as_deref()
                .unwrap_or(""),
        );
        Self::push_text(
            output,
            &identity.source_fingerprint.loaded_files.len().to_string(),
        );
        for path in &identity.source_fingerprint.loaded_files {
            Self::push_text(output, &path.to_string_lossy());
        }
    }

    fn write_def_id(&self, output: &mut String, def_id: DefId) {
        output.push('D');
        Self::write_crate_identity(
            output,
            self.identity_context.crate_identity(def_id.crate_id),
        );
        Self::push_text(output, &def_id.local.0.to_string());
    }

    fn write_kind(output: &mut String, kind: &crate::type_services::kind::Kind) {
        match kind {
            crate::type_services::kind::Kind::Type => output.push('T'),
            crate::type_services::kind::Kind::Arrow(input, output_kind) => {
                output.push('K');
                Self::write_kind(output, input);
                Self::write_kind(output, output_kind);
            }
        }
    }

    fn write_type_id(&self, output: &mut String, id: TypeId) {
        Self::write_kind(output, self.type_context.kind(id));
        match self.type_context.ty(id) {
            Ty::I8 => output.push_str("i8"),
            Ty::I16 => output.push_str("i16"),
            Ty::I32 => output.push_str("i32"),
            Ty::I64 => output.push_str("i64"),
            Ty::U8 => output.push_str("u8"),
            Ty::U16 => output.push_str("u16"),
            Ty::U32 => output.push_str("u32"),
            Ty::U64 => output.push_str("u64"),
            Ty::F32 => output.push_str("f32"),
            Ty::F64 => output.push_str("f64"),
            Ty::Bool => output.push('b'),
            Ty::Str => output.push_str("str"),
            Ty::Char => output.push('c'),
            Ty::Unit => output.push('u'),
            Ty::Never => output.push('n'),
            Ty::Slice(inner) => {
                output.push('s');
                self.write_type_id(output, *inner);
            }
            Ty::Array { inner, len } => {
                output.push('a');
                Self::push_text(output, &len.to_string());
                self.write_type_id(output, *inner);
            }
            Ty::Tuple(elements) => {
                output.push('t');
                Self::push_text(output, &elements.len().to_string());
                for element in elements {
                    self.write_type_id(output, *element);
                }
            }
            Ty::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
            } => {
                output.push('f');
                Self::push_text(output, &format!("{safety:?}"));
                Self::push_text(output, &format!("{callable_kind:?}"));
                Self::push_text(output, &params.len().to_string());
                for param in params {
                    self.write_type_id(output, *param);
                }
                self.write_type_id(output, *ret);
                Self::push_text(output, &captures.len().to_string());
                for capture in captures {
                    Self::push_text(output, &format!("{:?}", capture.kind));
                    self.write_type_id(output, capture.ty);
                }
            }
            Ty::Struct { id, args } => {
                output.push('S');
                self.write_def_id(output, *id);
                Self::push_text(output, &args.len().to_string());
                for arg in args {
                    self.write_type_id(output, *arg);
                }
            }
            Ty::Enum { id, args } => {
                output.push('E');
                self.write_def_id(output, *id);
                Self::push_text(output, &args.len().to_string());
                for arg in args {
                    self.write_type_id(output, *arg);
                }
            }
            Ty::Reference { mutable, inner } => {
                output.push(if *mutable { 'R' } else { 'r' });
                self.write_type_id(output, *inner);
            }
            Ty::Pointer(inner) => {
                output.push('p');
                self.write_type_id(output, *inner);
            }
            Ty::Generic(param) => {
                output.push('g');
                self.write_def_id(output, param.owner);
                Self::push_text(output, &param.index.to_string());
            }
            Ty::Projection {
                ty,
                trait_id,
                assoc_type,
                trait_args,
            } => {
                output.push('q');
                self.write_type_id(output, *ty);
                self.write_def_id(output, *trait_id);
                self.write_def_id(output, assoc_type.owner);
                Self::push_text(output, &assoc_type.assoc_type_id.0.to_string());
                Self::push_text(output, &trait_args.len().to_string());
                for arg in trait_args {
                    self.write_type_id(output, *arg);
                }
            }
            Ty::Constructor { id, flavor } => {
                output.push('C');
                Self::push_text(output, &format!("{flavor:?}"));
                self.write_def_id(output, *id);
            }
            Ty::Apply { constructor, args } => {
                output.push('A');
                self.write_type_id(output, *constructor);
                Self::push_text(output, &args.len().to_string());
                for arg in args {
                    self.write_type_id(output, *arg);
                }
            }
            Ty::Lambda { params, body } => {
                output.push('L');
                Self::push_text(output, &params.len().to_string());
                for param in params {
                    Self::write_kind(output, param);
                }
                self.write_type_id(output, *body);
            }
            Ty::BoundVar { depth, index, kind } => {
                output.push('B');
                Self::push_text(output, &depth.to_string());
                Self::push_text(output, &index.to_string());
                Self::write_kind(output, kind);
            }
            Ty::TypeVar(_) | Ty::Error => {
                panic!("unstable type state reached backend symbol fingerprint")
            }
        }
    }

    fn def_symbol_fragment(&self, def_id: DefId) -> String {
        let mut canonical = String::new();
        self.write_def_id(&mut canonical, def_id);
        format!(
            "d{}_h{:016x}",
            def_id.local.0,
            Self::hash_canonical(canonical.as_bytes())
        )
    }

    fn substitution_symbol_suffix(&self, substitution: &[TypeId]) -> String {
        if substitution.is_empty() {
            return "none".to_string();
        }

        let mut canonical = String::new();
        Self::push_text(&mut canonical, &substitution.len().to_string());
        for ty in substitution {
            self.write_type_id(&mut canonical, *ty);
        }
        format!("h{:016x}", Self::hash_canonical(canonical.as_bytes()))
    }

    fn backend_symbol_for_origin(
        &self,
        origin: &InstanceOrigin,
        substitution: &[TypeId],
    ) -> String {
        let suffix = self.substitution_symbol_suffix(substitution);
        match origin {
            InstanceOrigin::Function(def_id) => {
                format!("__rock_fn_{}_{}", self.def_symbol_fragment(*def_id), suffix)
            }
            InstanceOrigin::ImplMethod { owner, method } => {
                let owner = match owner {
                    InstanceImplOwner::Named(owner) => self.def_symbol_fragment(*owner),
                    InstanceImplOwner::BuiltinSlice => "builtin_slice".to_string(),
                };
                format!(
                    "__rock_impl_{}_method_{}_{}",
                    owner,
                    self.def_symbol_fragment(*method),
                    suffix
                )
            }
            InstanceOrigin::TraitDefault { trait_id, method } => {
                format!(
                    "__rock_trait_default_{}_method_{}_{}",
                    self.def_symbol_fragment(*trait_id),
                    self.def_symbol_fragment(*method),
                    suffix
                )
            }
        }
    }

    fn backend_symbol_for_function(
        &self,
        name: &str,
        origin: &InstanceOrigin,
        substitution: &[TypeId],
    ) -> String {
        if name == "main" && substitution.is_empty() {
            "main".to_string()
        } else {
            self.backend_symbol_for_origin(origin, substitution)
        }
    }

    fn function_instance_origin(&self, func: &HirFunction) -> InstanceOrigin {
        InstanceOrigin::Function(func.id)
    }

    fn register_concrete_function(&mut self, name: String, func: HirFunction) {
        self.function_names_by_id.entry(func.id).or_insert(name);
        self.concrete_functions.insert(func.id, func);
    }

    fn concrete_function_by_id(&self, id: DefId) -> Option<HirFunction> {
        self.concrete_functions.get(&id).cloned()
    }

    fn register_generic_function(&mut self, name: String, func: HirFunction) {
        self.function_names_by_id.entry(func.id).or_insert(name);
        self.generic_functions.insert(func.id, func);
    }

    fn register_external_generic_function(&mut self, name: String, func: HirFunction) {
        self.register_generic_function(name, func);
    }

    fn method_names_in_id_order(methods: &HashMap<String, HirFunction>) -> Vec<String> {
        let mut methods = methods
            .iter()
            .map(|(name, method)| (method.id, name.clone()))
            .collect::<Vec<_>>();
        methods.sort_by_key(|(id, _)| *id);
        methods.into_iter().map(|(_, name)| name).collect()
    }

    fn generic_function_by_id(&self, id: DefId) -> Option<(String, HirFunction)> {
        let name = self.function_names_by_id.get(&id)?.clone();
        let func = self.generic_functions.get(&id)?.clone();
        Some((name, func))
    }

    fn impl_owner_identity(&self, imp: &HirImpl) -> InstanceImplOwner {
        match &imp.owner {
            HirImplOwner::BuiltinSlice => InstanceImplOwner::BuiltinSlice,
            HirImplOwner::Named(_) => InstanceImplOwner::Named(imp.id),
        }
    }

    fn lookup_instance_function(&self, instance_id: InstanceId) -> Option<HirFunction> {
        self.instances.pre_mir_body(instance_id).cloned()
    }

    fn instance_callable_expr(
        &self,
        display_name: String,
        instance_id: InstanceId,
        ty: Type,
        span: crate::lexer::Span,
    ) -> HirExpr {
        HirExpr {
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: display_name,
                target: HirVarTarget::Instance(instance_id),
            }),
            ty,
            span,
        }
    }

    fn method_instance_origin(&self, imp: &HirImpl, method: &HirFunction) -> InstanceOrigin {
        InstanceOrigin::ImplMethod {
            owner: self.impl_owner_identity(imp),
            method: method.id,
        }
    }

    fn register_function_instance(&mut self, source_name: &str, func: &HirFunction) -> InstanceId {
        let origin = InstanceOrigin::Function(func.id);
        let key = InstanceKey::new(origin.clone(), Vec::new());
        let backend_symbol = self.backend_symbol_for_function(source_name, &origin, &[]);
        let body = func.clone();

        let instance_id = self.instances.intern(key, |id| InstanceRecord {
            id,
            origin,
            substitution: Vec::new(),
            symbols: InstanceSymbols::new(source_name, backend_symbol),
            declared: None,
            provided_by_object: false,
            is_specialization: false,
        });
        self.instances.insert_pre_mir_body(instance_id, body);
        instance_id
    }

    fn should_register_function_instance(func: &HirFunction) -> bool {
        func.generic_params.is_empty() && hir::hir_function_is_codegen_concrete(func)
    }

    fn register_impl_method_instance(
        &mut self,
        imp: &HirImpl,
        method_name: &str,
        method: &HirFunction,
    ) -> InstanceId {
        let origin = self.method_instance_origin(imp, method);
        let key = InstanceKey::new(origin.clone(), Vec::new());
        if let Some(instance_id) = self.instances.get(&key) {
            return instance_id;
        }
        let backend_symbol = self.backend_symbol_for_origin(&origin, &[]);
        let body = method.clone();

        let instance_id = self.instances.intern(key, |id| InstanceRecord {
            id,
            origin,
            substitution: Vec::new(),
            symbols: InstanceSymbols::new(
                format!("{}::{}", imp.type_name, method_name),
                backend_symbol,
            ),
            declared: None,
            provided_by_object: false,
            is_specialization: false,
        });
        self.instances.insert_pre_mir_body(instance_id, body);
        instance_id
    }

    fn should_register_method_instance(method: &HirFunction) -> bool {
        method.generic_params.is_empty() && hir::hir_function_is_codegen_concrete(method)
    }

    fn register_trait_default_instance(
        &mut self,
        trait_id: DefId,
        trait_name: &str,
        method_name: &str,
        method: &HirFunction,
    ) -> InstanceId {
        let origin = InstanceOrigin::TraitDefault {
            trait_id,
            method: method.id,
        };
        let key = InstanceKey::new(origin.clone(), Vec::new());
        if let Some(instance_id) = self.instances.get(&key) {
            return instance_id;
        }
        let backend_symbol = self.backend_symbol_for_origin(&origin, &[]);
        let body = method.clone();

        let instance_id = self.instances.intern(key, |id| InstanceRecord {
            id,
            origin,
            substitution: Vec::new(),
            symbols: InstanceSymbols::new(
                format!("{}::{}", trait_name, method_name),
                backend_symbol,
            ),
            declared: None,
            provided_by_object: false,
            is_specialization: false,
        });
        self.instances.insert_pre_mir_body(instance_id, body);
        instance_id
    }

    fn process_and_register_concrete_trait_defaults(&mut self, program: &mut HirProgram) {
        let mut traits = program
            .traits_by_id()
            .map(|(trait_id, trait_name, _)| (trait_id, trait_name.to_string()))
            .collect::<Vec<_>>();
        traits.sort_by_key(|(trait_id, _)| *trait_id);

        for (trait_id, trait_name) in &traits {
            let methods = program
                .traits
                .get(trait_id)
                .map(|trait_def| {
                    let mut methods = trait_def
                        .methods
                        .iter()
                        .filter_map(|(name, method)| {
                            hir::hir_function_is_codegen_concrete(method).then_some((
                                method.id,
                                name.clone(),
                                method.clone(),
                            ))
                        })
                        .collect::<Vec<_>>();
                    methods.sort_by_key(|(id, _, _)| *id);
                    methods
                        .into_iter()
                        .map(|(_, name, method)| (name, method))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for (method_name, method) in methods {
                self.register_trait_default_instance(*trait_id, trait_name, &method_name, &method);
            }
        }

        for (trait_id, trait_name) in traits {
            let method_names = program
                .traits
                .get(&trait_id)
                .map(|trait_def| Self::method_names_in_id_order(&trait_def.methods))
                .unwrap_or_default();

            for method_name in method_names {
                let Some(method) = program
                    .traits
                    .get_mut(&trait_id)
                    .and_then(|trait_def| trait_def.methods.remove(&method_name))
                else {
                    continue;
                };

                if !hir::hir_function_signature_is_codegen_concrete(&method) {
                    if let Some(trait_def) = program.traits.get_mut(&trait_id) {
                        trait_def.methods.insert(method_name, method);
                    }
                    continue;
                }

                let processed = self.process_function(method);
                let key = InstanceKey::new(
                    InstanceOrigin::TraitDefault {
                        trait_id,
                        method: processed.id,
                    },
                    Vec::new(),
                );
                if let Some(instance_id) = self.instances.get(&key) {
                    self.instances
                        .replace_pre_mir_body(instance_id, processed.clone());
                } else if Self::should_register_method_instance(&processed) {
                    self.register_trait_default_instance(
                        trait_id,
                        &trait_name,
                        &method_name,
                        &processed,
                    );
                }

                if let Some(trait_def) = program.traits.get_mut(&trait_id) {
                    trait_def.methods.insert(method_name, processed);
                }
            }
        }
    }

    fn materialize_registered_body_edges(&mut self) {
        let mut processed = std::collections::HashSet::new();
        loop {
            let bodies = self
                .instances
                .records()
                .filter(|record| !processed.contains(&record.id))
                .filter_map(|record| {
                    self.instances
                        .pre_mir_body(record.id)
                        .cloned()
                        .map(|body| (record.id, body))
                })
                .collect::<Vec<_>>();
            if bodies.is_empty() {
                break;
            }

            for (instance_id, body) in bodies {
                let processed_body = self.process_function(body);
                self.instances
                    .replace_pre_mir_body(instance_id, processed_body);
                processed.insert(instance_id);
            }
        }
    }

    fn validate_materialized_body_edges(&mut self) {
        let bodies = self
            .instances
            .records()
            .filter_map(|record| {
                self.instances
                    .pre_mir_body(record.id)
                    .cloned()
                    .map(|body| (record.symbols.source_name.clone(), body))
            })
            .collect::<Vec<_>>();
        let method_ids = self
            .generic_impls
            .values()
            .chain(self.trait_impls.values().flat_map(|impls| impls.iter()))
            .flat_map(|imp| imp.methods.values().map(|method| method.id))
            .collect::<HashSet<_>>();

        for (owner, body) in bodies {
            self.validate_materialized_block(&owner, &body.body, &method_ids);
        }
    }

    fn validate_materialized_block(
        &mut self,
        owner: &str,
        block: &HirBlock,
        method_ids: &HashSet<DefId>,
    ) {
        for stmt in &block.stmts {
            match stmt {
                HirStmt::Let { value, .. }
                | HirStmt::Expr(value)
                | HirStmt::Return(Some(value))
                | HirStmt::Break(Some(value)) => {
                    self.validate_materialized_expr(owner, value, method_ids)
                }
                HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
            }
        }
    }

    fn validate_materialized_expr(
        &mut self,
        owner: &str,
        expr: &HirExpr,
        method_ids: &HashSet<DefId>,
    ) {
        match &expr.kind {
            HirExprKind::MethodCall(receiver, name, args, _, _target) => {
                self.validate_materialized_expr(owner, receiver, method_ids);
                for arg in args {
                    self.validate_materialized_expr(owner, arg, method_ids);
                }
                self.diagnostics.push(
                    crate::diagnostic::Diagnostic::for_internal(
                    format!(
                        "post-monomorphization body '{owner}' retains unmaterialized method call '{name}'"
                    ),
                    )
                    .with_code(DiagnosticCode::Mono),
                );
            }
            HirExprKind::Call(callee, args, target) => {
                self.validate_materialized_expr(owner, callee, method_ids);
                for arg in args {
                    self.validate_materialized_expr(owner, arg, method_ids);
                }
                let unresolved_static = matches!(target, Some(HirCallTarget::StaticMethod(_)));
                let method_def_target = match target {
                    Some(HirCallTarget::Function(id)) => method_ids.contains(id),
                    _ => false,
                };
                let unresolved_field_call =
                    matches!(&callee.kind, HirExprKind::FieldAccess(_, _, None));
                if unresolved_static || method_def_target || unresolved_field_call {
                    self.diagnostics.push(
                        crate::diagnostic::Diagnostic::for_internal(format!(
                            "post-monomorphization body '{owner}' retains an unmaterialized method call edge"
                        ))
                        .with_code(DiagnosticCode::Mono),
                    );
                }
            }
            HirExprKind::Try {
                expr: carrier,
                branch_target,
                from_residual_target,
                ..
            } => {
                self.validate_materialized_expr(owner, carrier, method_ids);
                if !matches!(branch_target, Some(HirCallTarget::Instance(_)))
                    || !matches!(from_residual_target, HirCallTarget::Instance(_))
                {
                    self.diagnostics.push(
                        crate::diagnostic::Diagnostic::for_internal(format!(
                            "post-monomorphization body '{owner}' retains an unmaterialized Try call edge"
                        ))
                        .with_code(DiagnosticCode::Mono),
                    );
                }
            }
            HirExprKind::FieldAccess(base, _, _)
            | HirExprKind::TupleIndex(base, _)
            | HirExprKind::UnaryOp(_, base)
            | HirExprKind::Ref(_, base)
            | HirExprKind::Deref(base)
            | HirExprKind::Cast(base, _) => {
                self.validate_materialized_expr(owner, base, method_ids)
            }
            HirExprKind::ArrayLiteral(values) | HirExprKind::TupleLiteral(values) => {
                for value in values {
                    self.validate_materialized_expr(owner, value, method_ids);
                }
            }
            HirExprKind::ArrayRepeat(value, _) => {
                self.validate_materialized_expr(owner, value, method_ids);
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    self.validate_materialized_expr(owner, &field.value, method_ids);
                }
            }
            HirExprKind::EnumVariant(_, _, args, _) | HirExprKind::Intrinsic { args, .. } => {
                for arg in args {
                    self.validate_materialized_expr(owner, arg, method_ids);
                }
            }
            HirExprKind::BinOp(_, left, right) | HirExprKind::Assign(left, right) => {
                self.validate_materialized_expr(owner, left, method_ids);
                self.validate_materialized_expr(owner, right, method_ids);
            }
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.validate_materialized_expr(owner, condition, method_ids);
                self.validate_materialized_block(owner, then_branch, method_ids);
                if let Some(else_branch) = else_branch {
                    self.validate_materialized_block(owner, else_branch, method_ids);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.validate_materialized_expr(owner, scrutinee, method_ids);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.validate_materialized_expr(owner, guard, method_ids);
                    }
                    self.validate_materialized_block(owner, &arm.body, method_ids);
                }
            }
            HirExprKind::While { condition, body } => {
                self.validate_materialized_expr(owner, condition, method_ids);
                self.validate_materialized_block(owner, body, method_ids);
            }
            HirExprKind::For { iter, body, .. } => {
                self.validate_materialized_expr(owner, iter, method_ids);
                self.validate_materialized_block(owner, body, method_ids);
            }
            HirExprKind::Lambda { body, .. }
            | HirExprKind::Block(body)
            | HirExprKind::Loop(body)
            | HirExprKind::UnsafeBlock(body) => {
                self.validate_materialized_block(owner, body, method_ids)
            }
            HirExprKind::ResolvedVar(reference) => {
                if matches!(reference.target, HirVarTarget::Function(id) if method_ids.contains(&id))
                {
                    self.diagnostics.push(
                        crate::diagnostic::Diagnostic::for_internal(format!(
                            "post-monomorphization body '{owner}' retains an unmaterialized static method value"
                        ))
                        .with_code(DiagnosticCode::Mono),
                    );
                }
            }
            HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit
            | HirExprKind::Var(_) => {}
        }
    }

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

    fn register_nominal_field_types(&mut self, program: &HirProgram) {
        self.nominal_field_types.clear();
        self.nominal_field_spans.clear();
        for strukt in program.structs.values() {
            self.nominal_field_types.insert(
                strukt.id,
                strukt.fields.iter().map(|field| field.ty.clone()).collect(),
            );
            self.nominal_field_spans.insert(
                strukt.id,
                strukt
                    .fields
                    .iter()
                    .map(|field| {
                        self.source_map
                            .symbol(&SourceSymbol::Field {
                                owner: strukt.id,
                                field: field.id,
                            })
                            .map(|info| info.name_span.clone())
                    })
                    .collect(),
            );
        }
        for enm in program.enums.values() {
            let mut fields = Vec::new();
            let mut spans = Vec::new();
            for variant in &enm.variants {
                match &variant.fields {
                    HirVariantFields::Named(named) => {
                        fields.extend(named.iter().map(|field| field.ty.clone()));
                        spans.extend(named.iter().map(|field| {
                            self.source_map
                                .symbol(&SourceSymbol::Field {
                                    owner: enm.id,
                                    field: field.id,
                                })
                                .map(|info| info.name_span.clone())
                        }));
                    }
                    HirVariantFields::Positional(positional) => {
                        fields.extend(positional.iter().cloned());
                        spans.extend(std::iter::repeat(None).take(positional.len()));
                    }
                    HirVariantFields::Unit => {}
                }
            }
            self.nominal_field_types.insert(enm.id, fields);
            self.nominal_field_spans.insert(enm.id, spans);
        }
    }

    fn monomorphize_drop_fields_for_type(&mut self, ty: &Type) {
        let (Type::Struct { id, args } | Type::Enum { id, args }) = ty else {
            return;
        };
        let Some(field_types) = self.nominal_field_types.get(id).cloned() else {
            return;
        };
        let type_key = self.intern_type(ty);
        if !self
            .drop_field_monomorphization_in_progress
            .insert(type_key)
        {
            return;
        }

        let subst: HashMap<_, _> = args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                (
                    GenericParamId {
                        owner: *id,
                        index: index as u32,
                    },
                    arg.clone(),
                )
            })
            .collect();
        let field_spans = self
            .nominal_field_spans
            .get(id)
            .cloned()
            .unwrap_or_default();
        for (index, field_ty) in field_types.into_iter().enumerate() {
            self.monomorphize_drop_for_type(
                &field_ty.substitute_generics(&subst),
                field_spans.get(index).cloned().flatten(),
            );
        }

        self.drop_field_monomorphization_in_progress
            .remove(&type_key);
    }

    fn monomorphize_known_drop_types(&mut self) {
        let mut index = 0;
        while index < self.type_context.len() {
            let ty = self.type_for(TypeId(index as u32));
            index += 1;
            self.monomorphize_drop_for_type(&ty, None);
        }
    }

    /// Collect all trait impls from the program
    fn collect_impls(&mut self, impls: &[HirImpl]) {
        self.collect_impls_with_body_provider(impls, |_| false);
    }

    fn collect_impls_with_crate_capabilities(
        &mut self,
        impls: &[HirImpl],
        crate_ctx: &CrateContext,
    ) {
        self.collect_impls_with_body_provider(impls, |imp| crate_ctx.provides_impl_body(imp));
    }

    fn collect_impls_with_body_provider(
        &mut self,
        impls: &[HirImpl],
        _provided_by_dependency: impl Fn(&HirImpl) -> bool,
    ) {
        for imp in impls {
            if let Some(trait_id) = imp.trait_id {
                self.trait_impls
                    .entry(trait_id)
                    .or_insert_with(Vec::new)
                    .push(imp.clone());
            } else {
                self.generic_impls.insert(imp.id, imp.clone());
            }
        }
    }

    fn with_type_context(type_context: TypeContext) -> Self {
        Self {
            type_context,
            ..Self::new()
        }
    }

    /// Convert a type to a string suffix for monomorphized function names
    fn type_to_mono_suffix(ty: &Type) -> String {
        match ty {
            Type::I64 => "I64".to_string(),
            Type::I32 => "I32".to_string(),
            Type::I16 => "I16".to_string(),
            Type::I8 => "I8".to_string(),
            Type::U64 => "U64".to_string(),
            Type::U32 => "U32".to_string(),
            Type::U16 => "U16".to_string(),
            Type::U8 => "U8".to_string(),
            Type::F64 => "F64".to_string(),
            Type::F32 => "F32".to_string(),
            Type::Bool => "Bool".to_string(),
            Type::Char => "Char".to_string(),
            Type::Str => "Str".to_string(),
            Type::Struct { id, args } | Type::Enum { id, args } if args.is_empty() => {
                format!("{}_{}", id.crate_id.0, id.local.0)
            }
            Type::Struct { id, args: generics } | Type::Enum { id, args: generics } => {
                let args: String = generics
                    .iter()
                    .map(|g| format!("_{}", Self::type_to_mono_suffix(g)))
                    .collect();
                format!("{}_{}{}", id.crate_id.0, id.local.0, args)
            }
            Type::Pointer(inner) => format!("Ptr{}", Self::type_to_mono_suffix(inner)),
            Type::Slice(inner) => format!("Slice{}", Self::type_to_mono_suffix(inner)),
            Type::Array(inner, len) => {
                format!("Arr{}_{}", Self::type_to_mono_suffix(inner), len)
            }
            Type::Reference { inner, .. } => Self::type_to_mono_suffix(inner),
            _ => "T".to_string(),
        }
    }

    fn receiver_mono_suffix(ty: &Type) -> String {
        match ty {
            Type::Reference { inner, .. } => Self::receiver_mono_suffix(inner),
            _ => Self::type_to_mono_suffix(ty),
        }
    }

    fn impl_receiver_pattern_matches(&self, imp: &HirImpl, recv_ty: &Type) -> bool {
        if self
            .language_items
            .callable_protocols()
            .any(|(_, id, _)| imp.trait_id == Some(id))
        {
            if let HirImplReceiverPattern::Exact(pattern) = &imp.receiver_pattern {
                return crate::selection::type_pattern_matches(
                    pattern,
                    recv_ty,
                    &mut HashMap::new(),
                );
            }
        }
        crate::selection::impl_receiver_pattern_substitution(
            imp,
            recv_ty,
            self.trait_impls.values().flatten(),
        )
        .is_some()
    }

    fn impl_receiver_pattern_by_id(&self, impl_id: DefId) -> Option<&HirImplReceiverPattern> {
        self.generic_impls
            .get(&impl_id)
            .or_else(|| {
                self.trait_impls
                    .values()
                    .flat_map(|impls| impls.iter())
                    .find(|imp| imp.id == impl_id)
            })
            .map(|imp| &imp.receiver_pattern)
    }

    #[cfg(test)]
    fn set_impl_receiver_pattern_for_test(
        &mut self,
        impl_id: DefId,
        pattern: HirImplReceiverPattern,
    ) {
        if let Some(imp) = self.generic_impls.get_mut(&impl_id) {
            imp.receiver_pattern = pattern;
            return;
        }
        if let Some(imp) = self
            .trait_impls
            .values_mut()
            .flat_map(|impls| impls.iter_mut())
            .find(|imp| imp.id == impl_id)
        {
            imp.receiver_pattern = pattern;
        }
    }

    fn drop_receiver_matches(&self, imp: &HirImpl, recv_ty: &Type) -> bool {
        self.impl_receiver_pattern_matches(imp, recv_ty)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        monomorphize_for_tests, CompilationIdentityContext, InstanceImplOwner, InstanceKey,
        InstanceOrigin, MonoError, MonoErrorKind, Monomorphizer,
    };

    use crate::diagnostic::{DiagnosticCode, DiagnosticLocation};
    use crate::hir::{
        AcceptedHirProgram, HirAssociatedTypeDef, HirImplOwner, HirImplReceiverPattern, HirParam,
    };
    use crate::ids::{AssocTypeId, CrateId, DefId, HirLocalId, LocalDefId, TypeId};
    use crate::mono::hir_types::{HirBlock, HirFunction, HirImpl, HirProgram};
    use crate::products::{ProductCrateIdentity, ProductSourceFingerprint};
    use crate::type_services::kind::Kind;
    use crate::type_services::normalize::TypeNormalizationEnv;
    use crate::types::{GenericParamDecl, GenericParamId, NominalTypeKind, Type};

    use std::collections::HashMap;

    #[test]
    fn mono_receiver_diagnostic_uses_user_type_names_and_mono_code() {
        let span = crate::lexer::Span::new("/virtual/main.rk".into(), 2, 7);
        let receiver = Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(7)),
            args: vec![Type::I64],
        };
        let diagnostic = MonoError {
            kind: MonoErrorKind::ReceiverMismatch {
                impl_id: DefId::new(CrateId(0), LocalDefId(8)),
                receiver,
                pattern: None,
            },
            span: span.clone(),
        }
        .diagnostic(&crate::collect::resolver::ResolverTables::default());

        assert_eq!(diagnostic.code, Some(DiagnosticCode::Mono));
        assert_eq!(diagnostic.location, DiagnosticLocation::Source(span));
        assert!(!diagnostic.message.contains("struct#"));
        assert!(!diagnostic.message.contains("DefId"));
    }

    #[test]
    fn monomorphizer_display_context_renders_generic_projection_names() {
        let mut mono = Monomorphizer::new();
        let function_id = DefId::new(CrateId(0), LocalDefId(81));
        mono.generic_functions.insert(
            function_id,
            test_function(function_id, "identity", vec!["T".to_string()]),
        );

        let trait_id = DefId::new(CrateId(0), LocalDefId(82));
        mono.resolver
            .item_names_by_id
            .insert(trait_id, "Iterator".to_string());
        let impl_id = DefId::new(CrateId(0), LocalDefId(83));
        let impl_generic = GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        );
        let mut impl_def = concrete_impl_with_methods(impl_id, &[]);
        impl_def.type_generics = vec![impl_generic];
        impl_def.trait_id = Some(trait_id);
        impl_def.associated_types = vec![HirAssociatedTypeDef {
            id: AssocTypeId(0),
            name: "Item".to_string(),
            kind: Kind::Type,
            ty: Type::I64,
        }];
        mono.generic_impls.insert(impl_id, impl_def);

        let generic = Type::Generic(GenericParamId {
            owner: impl_id,
            index: 0,
        });
        assert_eq!(mono.display_type_for_diagnostic(&generic), "T");
        let projection = Type::Projection {
            ty: Box::new(generic),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: Vec::new(),
        };
        assert_eq!(
            mono.display_type_for_diagnostic(&projection),
            "<T as Iterator>::Item"
        );
    }

    fn test_function(id: DefId, name: &str, generic_params: Vec<String>) -> HirFunction {
        let generic_params = generic_params
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                GenericParamDecl::type_param(
                    GenericParamId {
                        owner: id,
                        index: index as u32,
                    },
                    name,
                )
            })
            .collect();

        HirFunction {
            id,
            name: name.to_string(),
            generic_params,
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

    fn test_method(id: DefId, name: &str) -> HirFunction {
        let mut method = test_function(id, name, Vec::new());
        method.is_method = true;
        method
    }

    fn concrete_function_with_param(id: DefId, name: &str, ty: Type) -> HirFunction {
        let mut function = test_function(id, name, Vec::new());
        function.params = vec![HirParam {
            name: "value".to_string(),
            local_id: HirLocalId(0),
            ty,
            mutable: false,
            is_ref: false,
        }];
        function
    }

    fn monomorphize_functions_in_order(order: &[DefId]) -> (Vec<InstanceOrigin>, Vec<Type>) {
        let first_id = DefId::new(CrateId(0), LocalDefId(10));
        let second_id = DefId::new(CrateId(0), LocalDefId(20));
        let mut functions = HashMap::new();
        for id in order {
            let function = match *id {
                id if id == first_id => concrete_function_with_param(id, "first", Type::I64),
                id if id == second_id => concrete_function_with_param(id, "second", Type::Bool),
                _ => panic!("unexpected fixture DefId"),
            };
            functions.insert(*id, function);
        }
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            functions,
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::from([
                (first_id, "first".to_string()),
                (second_id, "second".to_string()),
            ]),
        );
        let program = AcceptedHirProgram::revalidate_for_test(program)
            .expect("mono fixture must satisfy accepted HIR invariants");
        let monomorphized = monomorphize_for_tests(program).expect("monomorphization succeeds");

        let origins = monomorphized
            .instances
            .values()
            .map(|record| record.origin.clone())
            .collect();
        let types = (0..monomorphized.type_context.len())
            .map(|index| monomorphized.type_context.type_for(TypeId(index as u32)))
            .collect();
        (origins, types)
    }

    fn concrete_impl_with_methods(id: DefId, method_ids: &[DefId]) -> HirImpl {
        let methods = method_ids
            .iter()
            .map(|method_id| {
                (
                    format!("method_{}", method_id.local.0),
                    test_method(*method_id, "method"),
                )
            })
            .collect();
        HirImpl {
            id,
            owner: HirImplOwner::Named(format!("Impl_{}", id.local.0)),
            type_name: format!("Impl_{}", id.local.0),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods,
        }
    }

    fn monomorphize_impls_in_order(
        order: &[DefId],
        first_method_ids: &[DefId],
    ) -> Vec<InstanceOrigin> {
        let first_impl_id = DefId::new(CrateId(0), LocalDefId(30));
        let second_impl_id = DefId::new(CrateId(0), LocalDefId(40));
        let mut impls = HashMap::new();
        for id in order {
            let imp = match *id {
                id if id == first_impl_id => concrete_impl_with_methods(id, first_method_ids),
                id if id == second_impl_id => {
                    concrete_impl_with_methods(id, &[DefId::new(CrateId(0), LocalDefId(41))])
                }
                _ => panic!("unexpected fixture DefId"),
            };
            impls.insert(*id, imp);
        }
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            impls,
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        );
        let program = AcceptedHirProgram::revalidate_for_test(program)
            .expect("mono fixture must satisfy accepted HIR invariants");
        monomorphize_for_tests(program)
            .expect("monomorphization succeeds")
            .instances
            .values()
            .map(|record| record.origin.clone())
            .collect()
    }

    #[test]
    fn backend_symbol_for_origin_uses_ids_not_display_names() {
        let function_id = DefId::new(CrateId(0), LocalDefId(42));
        let origin = InstanceOrigin::Function(function_id);
        let mono = Monomorphizer::new();
        let symbol = mono.backend_symbol_for_origin(&origin, &[]);

        assert!(symbol.starts_with("__rock_fn_d42_h"));
        assert!(symbol.ends_with("_none"));
    }

    fn product_identity(name: &str, source_hash: &str) -> ProductCrateIdentity {
        ProductCrateIdentity::local(name.to_string()).with_source_fingerprint(
            ProductSourceFingerprint {
                manifest_hash: None,
                source_hash: Some(source_hash.to_string()),
                loaded_files: vec!["src/main.rk".into()],
            },
        )
    }

    #[test]
    fn backend_symbol_fingerprint_ignores_type_id_allocation_order() {
        let origin = InstanceOrigin::Function(DefId::new(CrateId(0), LocalDefId(1)));
        let ty = Type::Enum {
            id: DefId::new(CrateId(0), LocalDefId(2)),
            args: vec![Type::I64],
        };
        let mut first = Monomorphizer::new();
        first.intern_type(&Type::Bool);
        let first_ty = first.intern_type(&ty);
        let mut second = Monomorphizer::new();
        second.intern_type(&Type::U8);
        second.intern_type(&Type::Bool);
        let second_ty = second.intern_type(&ty);

        assert_ne!(first_ty, second_ty);
        assert_eq!(
            first.backend_symbol_for_origin(&origin, &[first_ty]),
            second.backend_symbol_for_origin(&origin, &[second_ty])
        );
    }

    #[test]
    fn backend_symbol_fingerprint_ignores_dependency_load_order() {
        let current = product_identity("app", "app-hash");
        let dependency = product_identity("dep", "dep-hash");
        let mut first = Monomorphizer::new();
        first.identity_context = CompilationIdentityContext::new(
            CrateId(0),
            current.clone(),
            std::collections::BTreeMap::from([(CrateId(1), dependency.clone())]),
        );
        let mut second = Monomorphizer::new();
        second.identity_context = CompilationIdentityContext::new(
            CrateId(0),
            current,
            std::collections::BTreeMap::from([(CrateId(9), dependency)]),
        );
        let first_ty = first.intern_type(&Type::Struct {
            id: DefId::new(CrateId(1), LocalDefId(7)),
            args: vec![],
        });
        let second_ty = second.intern_type(&Type::Struct {
            id: DefId::new(CrateId(9), LocalDefId(7)),
            args: vec![],
        });
        let origin = InstanceOrigin::Function(DefId::new(CrateId(0), LocalDefId(3)));

        assert_eq!(
            first.backend_symbol_for_origin(&origin, &[first_ty]),
            second.backend_symbol_for_origin(&origin, &[second_ty])
        );
    }

    #[test]
    fn backend_symbol_fingerprint_delimits_nested_nominal_arguments() {
        let outer = DefId::new(CrateId(0), LocalDefId(20));
        let inner = DefId::new(CrateId(0), LocalDefId(21));
        let origin = InstanceOrigin::Function(DefId::new(CrateId(0), LocalDefId(22)));
        let mut mono = Monomorphizer::new();
        let siblings = mono.intern_type(&Type::Struct {
            id: outer,
            args: vec![
                Type::Struct {
                    id: inner,
                    args: vec![],
                },
                Type::I64,
            ],
        });
        let nested = mono.intern_type(&Type::Struct {
            id: outer,
            args: vec![Type::Struct {
                id: inner,
                args: vec![Type::I64],
            }],
        });

        assert_ne!(
            mono.backend_symbol_for_origin(&origin, &[siblings]),
            mono.backend_symbol_for_origin(&origin, &[nested])
        );
    }

    #[test]
    fn constructor_instance_keys_distinguish_families_and_reuse_eta_equivalents() {
        let option = DefId::new(CrateId(0), LocalDefId(10));
        let vector = DefId::new(CrateId(0), LocalDefId(11));
        let unary = Kind::arrow(Kind::Type, Kind::Type);
        let mut env = TypeNormalizationEnv::new();
        env.register_constructor(option, NominalTypeKind::Enum, unary.clone());
        env.register_constructor(vector, NominalTypeKind::Struct, unary);
        let mut mono = Monomorphizer::new();
        mono.normalization_env = env;
        let option_ty = Type::Constructor {
            id: option,
            flavor: NominalTypeKind::Enum,
        };
        let option_id = mono.intern_type(&option_ty);
        let eta_option_id = mono.intern_type(&Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::Apply {
                constructor: Box::new(option_ty),
                args: vec![Type::BoundVar {
                    depth: 0,
                    index: 0,
                    kind: Kind::Type,
                }],
            }),
        });
        let vector_id = mono.intern_type(&Type::Constructor {
            id: vector,
            flavor: NominalTypeKind::Struct,
        });
        let origin = InstanceOrigin::Function(DefId::new(CrateId(0), LocalDefId(12)));

        assert_eq!(option_id, eta_option_id);
        assert_eq!(
            InstanceKey::new(origin.clone(), vec![option_id]),
            InstanceKey::new(origin.clone(), vec![eta_option_id])
        );
        assert_ne!(
            InstanceKey::new(origin, vec![option_id]),
            InstanceKey::new(
                InstanceOrigin::Function(DefId::new(CrateId(0), LocalDefId(12))),
                vec![vector_id]
            )
        );
    }

    #[test]
    fn monomorphize_processes_alias_duplicate_function_once_by_def_id() {
        let id = DefId::new(CrateId(0), LocalDefId(1));
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::from([(id, test_function(id, "canonical", Vec::new()))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables {
                functions_by_name: HashMap::from([
                    ("alias".to_string(), id),
                    ("canonical".to_string(), id),
                ]),
                ..crate::hir::HirNameTables::default()
            },
            &HashMap::from([(id, "canonical".to_string())]),
        );

        let program = AcceptedHirProgram::revalidate_for_test(program)
            .expect("mono fixture must satisfy accepted HIR invariants");
        let monomorphized = monomorphize_for_tests(program).expect("monomorphization succeeds");

        assert_eq!(monomorphized.program.functions.len(), 1);
        assert!(monomorphized.program.functions.contains_key(&id));
        assert!(monomorphized
            .program
            .names
            .functions_by_name
            .contains_key("canonical"));
        assert!(!monomorphized
            .program
            .names
            .functions_by_name
            .contains_key("alias"));
        assert_eq!(
            monomorphized.program.indexes.functions_by_id.get(&id),
            Some(&"canonical".to_string())
        );
    }

    #[test]
    fn monomorphize_allocates_top_level_instances_and_types_by_def_id() {
        let first_id = DefId::new(CrateId(0), LocalDefId(10));
        let second_id = DefId::new(CrateId(0), LocalDefId(20));
        let expected_origins = vec![
            InstanceOrigin::Function(first_id),
            InstanceOrigin::Function(second_id),
        ];
        let expected_types = vec![Type::I64, Type::Bool];

        for order in [[first_id, second_id], [second_id, first_id]] {
            for _ in 0..16 {
                let (origins, types) = monomorphize_functions_in_order(&order);
                assert_eq!(origins, expected_origins);
                assert_eq!(types, expected_types);
            }
        }
    }

    #[test]
    fn monomorphize_allocates_impl_method_instances_by_impl_and_method_def_id() {
        let first_impl_id = DefId::new(CrateId(0), LocalDefId(30));
        let second_impl_id = DefId::new(CrateId(0), LocalDefId(40));
        let first_method_id = DefId::new(CrateId(0), LocalDefId(31));
        let second_method_id = DefId::new(CrateId(0), LocalDefId(32));
        let third_method_id = DefId::new(CrateId(0), LocalDefId(41));
        let expected = vec![
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(first_impl_id),
                method: first_method_id,
            },
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(first_impl_id),
                method: second_method_id,
            },
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(second_impl_id),
                method: third_method_id,
            },
        ];

        for impl_order in [
            [first_impl_id, second_impl_id],
            [second_impl_id, first_impl_id],
        ] {
            for method_order in [
                [first_method_id, second_method_id],
                [second_method_id, first_method_id],
            ] {
                assert_eq!(
                    monomorphize_impls_in_order(&impl_order, &method_order),
                    expected
                );
            }
        }
    }

    fn generic_impl(owner: &str, type_name: &str, id: DefId) -> HirImpl {
        HirImpl {
            id,
            owner: HirImplOwner::Named(owner.to_string()),
            type_name: type_name.to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: id,
                    index: 0,
                },
                "T",
            )],
            receiver_pattern: vec![Type::Generic(crate::types::GenericParamId {
                owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0)),
                index: 0,
            })]
            .into(),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        }
    }

    #[test]
    fn collect_impls_preserves_same_short_name_generic_impls_by_owner() {
        let mut mono = Monomorphizer::new();
        let first_id = DefId::new(CrateId(0), LocalDefId(10));
        let second_id = DefId::new(CrateId(0), LocalDefId(11));

        mono.collect_impls(&[
            generic_impl("a::Foo", "Foo", first_id),
            generic_impl("b::Foo", "Foo", second_id),
        ]);

        assert_eq!(mono.generic_impls.len(), 2);
    }

    #[test]
    fn receiver_type_match_uses_impl_owner_identity_for_nominals() {
        let mono = Monomorphizer::new();
        let first_id = DefId::new(CrateId(0), LocalDefId(10));
        let second_id = DefId::new(CrateId(0), LocalDefId(11));
        let mut imp = generic_impl("b::Foo", "Foo", second_id);
        imp.receiver_pattern = HirImplReceiverPattern::Exact(Type::Struct {
            id: second_id,
            args: vec![Type::Generic(crate::types::GenericParamId {
                owner: second_id,
                index: 0,
            })],
        });
        let recv_ty = Type::Struct {
            id: first_id,
            args: vec![Type::I64],
        };

        assert!(!mono.impl_receiver_pattern_matches(&imp, &recv_ty));
    }

    #[test]
    fn receiver_type_match_accepts_artifact_owner_alias_for_canonical_receiver_path() {
        let mono = Monomorphizer::new();
        let option_id = DefId::new(CrateId(1), LocalDefId(10));
        let mut imp = generic_impl("stdlib::Option", "Option", option_id);
        imp.receiver_pattern = HirImplReceiverPattern::Exact(Type::Enum {
            id: option_id,
            args: vec![Type::Generic(crate::types::GenericParamId {
                owner: option_id,
                index: 0,
            })],
        });
        let recv_ty = Type::Enum {
            id: option_id,
            args: vec![Type::I64],
        };

        assert!(mono.impl_receiver_pattern_matches(&imp, &recv_ty));
    }

    #[test]
    fn receiver_type_match_rejects_ambiguous_artifact_owner_alias() {
        let mono = Monomorphizer::new();
        let first_id = DefId::new(CrateId(1), LocalDefId(10));
        let second_id = DefId::new(CrateId(1), LocalDefId(11));
        let mut imp = generic_impl("dep::Box", "Box", second_id);
        imp.receiver_pattern = HirImplReceiverPattern::Exact(Type::Struct {
            id: second_id,
            args: vec![Type::Generic(crate::types::GenericParamId {
                owner: second_id,
                index: 0,
            })],
        });
        let recv_ty = Type::Struct {
            id: first_id,
            args: vec![Type::I64],
        };

        assert!(!mono.impl_receiver_pattern_matches(&imp, &recv_ty));
    }

    #[test]
    fn builtin_slice_owner_identity_is_structural() {
        let mono = Monomorphizer::new();
        let method_id = DefId::new(CrateId(0), LocalDefId(1));
        let method = test_method(method_id, "println");
        let imp = HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            owner: HirImplOwner::BuiltinSlice,
            type_name: "&[T]".to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: DefId::new(CrateId(0), LocalDefId(0)),
                    index: 0,
                },
                "T",
            )],
            receiver_pattern: vec![Type::Generic(crate::types::GenericParamId {
                owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0)),
                index: 0,
            })]
            .into(),
            trait_name: Some("Show".to_string()),
            trait_id: Some(DefId::new(CrateId(0), LocalDefId(20))),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        };

        assert_eq!(
            mono.method_instance_origin(&imp, &method),
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::BuiltinSlice,
                method: method_id,
            }
        );
    }

    #[test]
    fn named_impl_method_origin_uses_impl_and_method_def_ids() {
        let mut mono = Monomorphizer::new();
        let foo_type_id = DefId::new(CrateId(0), LocalDefId(10));
        let display_impl_id = DefId::new(CrateId(0), LocalDefId(11));
        let debug_impl_id = DefId::new(CrateId(0), LocalDefId(12));
        let display_method_id = DefId::new(CrateId(0), LocalDefId(13));
        let debug_method_id = DefId::new(CrateId(0), LocalDefId(14));
        let display_method = test_method(display_method_id, "fmt");
        let debug_method = test_method(debug_method_id, "fmt");
        mono.resolver
            .item_paths
            .insert("Foo".to_string(), foo_type_id);

        let display_impl = HirImpl {
            id: display_impl_id,
            owner: HirImplOwner::Named("Foo".to_string()),
            type_name: "Foo".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Display".to_string()),
            trait_id: Some(DefId::new(CrateId(0), LocalDefId(30))),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        };
        let debug_impl = HirImpl {
            id: debug_impl_id,
            owner: HirImplOwner::Named("Foo".to_string()),
            type_name: "Foo".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Debug".to_string()),
            trait_id: Some(DefId::new(CrateId(0), LocalDefId(31))),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        };

        assert_eq!(
            mono.method_instance_origin(&display_impl, &display_method),
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(display_impl_id),
                method: display_method_id,
            }
        );
        assert_eq!(
            mono.method_instance_origin(&debug_impl, &debug_method),
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(debug_impl_id),
                method: debug_method_id,
            }
        );
    }
}
