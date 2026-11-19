//! Phase 4 constraint solver: discharge accumulated constraints after lowering.
//!
//! This runs between generalization and finalization. It:
//!   - Verifies trait bounds: if a type variable resolved to a concrete type, checks
//!     that the type has an impl for the required trait.
//!   - Checks integer/float literal constraints: if a TypeVar from a literal still
//!     has no concrete type, emits an ambiguity warning.
//!
//! Trait bound violations on concrete types are hard errors.

mod auto_traits;
mod constructor_obligations;
mod impl_lookup;
mod structural;
mod trait_bounds;
mod type_predicates;
mod worklist;

use std::collections::{HashMap, HashSet};

use crate::hir::{HirEnum, HirImpl, HirStruct};
use crate::ids::DefId;
use crate::ids::TypeVarId;
use crate::infer::constraints::{ConstraintOwner, ConstraintStore};
use crate::infer::{InferenceEngine, UnifyError};
use crate::language_items::LanguageItems;
use crate::types::{TraitBound, Type};
use worklist::solve_constraints_in_place_mode;

#[derive(Debug, Clone)]
pub enum SolveErrorKind {
    Unify {
        context: String,
        error: UnifyError,
    },
    TraitNotImplemented {
        ty: Type,
        trait_name: String,
        trait_args: Vec<Type>,
        context: String,
    },
    TryNonCarrier {
        ty: Type,
    },
    IntegerLiteral {
        found: Type,
    },
    /// An obligation still undecided after the final solve. Often a consequence
    /// of a definite failure elsewhere that left its types unsolved.
    Ambiguous {
        context: String,
    },
}

#[derive(Debug, Clone)]
pub struct SolveError {
    pub kind: SolveErrorKind,
    pub span: crate::lexer::Span,
}

impl SolveError {
    pub fn contains(&self, pattern: &str) -> bool {
        self.message().contains(pattern)
    }

    pub fn message(&self) -> String {
        self.render_with(|ty| ty.to_string())
    }

    pub fn render(&self, engine: &InferenceEngine) -> String {
        self.render_with(|ty| engine.display_type(ty))
    }

    fn render_with(&self, display: impl Fn(&Type) -> String) -> String {
        match &self.kind {
            SolveErrorKind::Unify { context, error } => {
                format!("{context}: {}", error.render_with(&display))
            }
            SolveErrorKind::TraitNotImplemented {
                ty,
                trait_name,
                trait_args,
                context,
            } => {
                let description = if let Type::Function { params, ret, captures, .. } = ty {
                    let params = params.iter().map(&display).collect::<Vec<_>>().join(", ");
                    let captures = captures.iter().map(|capture| {
                        let kind = match capture.kind {
                            crate::types::CaptureKind::SharedBorrow => "shared borrow",
                            crate::types::CaptureKind::MutableBorrow => "mutable borrow",
                            crate::types::CaptureKind::Move => "move",
                        };
                        format!("{kind} of `{}`", display(&capture.ty))
                    }).collect::<Vec<_>>().join(", ");
                    format!("closure `({params}) -> {}` with captures [{captures}]", display(ret))
                } else {
                    format!("type `{}`", display(ty))
                };
                // Users read `FnMut`, not its module path; arguments that are
                // themselves applied types are grouped as in source syntax.
                let trait_name = trait_name.rsplit("::").next().unwrap_or(trait_name);
                let trait_display = if trait_args.is_empty() {
                    trait_name.to_string()
                } else {
                    let args = trait_args
                        .iter()
                        .map(|arg| {
                            let shown = display(arg);
                            if shown.contains(' ') && !shown.starts_with('(') {
                                format!("({shown})")
                            } else {
                                shown
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{trait_name} {args}")
                };
                format!(
                    "{} does not implement trait `{}` (required by {})",
                    description, trait_display, context
                )
            },
            SolveErrorKind::TryNonCarrier { ty } => {
                format!("Cannot use '?' on non-carrier type {}", display(ty))
            }
            SolveErrorKind::IntegerLiteral { found } => format!(
                "integer literal resolved to non-integer type `{}`",
                display(found)
            ),
            SolveErrorKind::Ambiguous { context } => format!(
                "ambiguous constraint at quiescence: {context}; add type information to select one canonical solution"
            ),
        }
    }

    pub fn is_ambiguity(&self) -> bool {
        matches!(self.kind, SolveErrorKind::Ambiguous { .. })
    }
}

pub(crate) fn program_type_is_copy<P: crate::hir::HirPhase>(
    program: &crate::hir::HirProgramFor<P>,
    ty: &Type,
) -> bool {
    // Only nominal types can gain Copy through an explicit impl; every other
    // shape is decided structurally without touching the impl table.
    fn mentions_nominal(ty: &Type) -> bool {
        match ty {
            Type::Struct { .. } | Type::Enum { .. } => true,
            Type::Tuple(elements) => elements.iter().any(mentions_nominal),
            Type::Array(element, _) => mentions_nominal(element),
            _ => false,
        }
    }

    let Some(copy) = program.language_items.copy.filter(|_| mentions_nominal(ty)) else {
        return crate::type_services::facts::TypeFacts::is_copy(ty);
    };
    impl_lookup::impl_exists_for(
        ty,
        copy,
        &[],
        &program.impls,
        &program.structs,
        &program.enums,
        BuiltinTraitIds::from_language_items(&program.language_items),
        &|_, _, _| false,
    )
}

/// Names a field type for Copy diagnostics using source names for nominal
/// types and the impl's own generic parameters.
fn copy_diagnostic_type_name<P: crate::hir::HirPhase>(
    program: &crate::hir::HirProgramFor<P>,
    imp: &crate::hir::HirImplFor<P>,
    ty: &Type,
) -> String {
    let mut context = crate::type_services::display::TypeDisplayContext::default();
    for (id, name) in program
        .structs
        .iter()
        .map(|(id, def)| (id, &def.name))
        .chain(program.enums.iter().map(|(id, def)| (id, &def.name)))
    {
        context.insert_definition_name(*id, name.rsplit("::").next().unwrap_or(name).to_string());
    }
    for generic in &imp.type_generics {
        context.insert_generic_name(generic.id, generic.name.clone());
    }
    crate::type_services::display::display_type_with_context(ty, &context).to_string()
}

/// Checks every explicit `impl Copy`: the receiver must be a struct or enum
/// without a `Drop` impl whose fields are all provably Copy. Errors carry the
/// offending impl so callers can attach its source span.
pub(crate) fn validate_copy_impls<P: crate::hir::HirPhase>(
    program: &crate::hir::HirProgramFor<P>,
) -> Vec<(DefId, String)> {
    use crate::hir::{HirImplReceiverPattern, HirVariantFields};
    use crate::types::GenericParamId;

    let Some(copy) = program.language_items.copy else {
        return Vec::new();
    };
    let builtin = BuiltinTraitIds::from_language_items(&program.language_items);
    let drop_trait = program
        .language_items
        .drop
        .as_ref()
        .map(|drop| drop.trait_id);
    let mut errors = Vec::new();
    let mut implementations: Vec<_> = program
        .impls
        .values()
        .filter(|imp| imp.trait_id == Some(copy))
        .collect();
    implementations.sort_by_key(|imp| imp.id);
    for imp in implementations {
        let not_nominal = || {
            (
                imp.id,
                format!(
                    "Copy may only be implemented for a struct or enum, not `{}`",
                    imp.type_name
                ),
            )
        };
        let Some(HirImplReceiverPattern::Exact(receiver)) =
            P::impl_receiver_pattern(&imp.receiver_pattern)
        else {
            errors.push(not_nominal());
            continue;
        };
        let (id, args, fields): (_, _, Vec<_>) = match receiver {
            Type::Struct { id, args } => {
                let Some(def) = program.structs.get(id) else {
                    errors.push((
                        imp.id,
                        format!("unknown struct `{}` in Copy impl", imp.type_name),
                    ));
                    continue;
                };
                (id, args, def.fields.iter().map(|field| &field.ty).collect())
            }
            Type::Enum { id, args } => {
                let Some(def) = program.enums.get(id) else {
                    errors.push((
                        imp.id,
                        format!("unknown enum `{}` in Copy impl", imp.type_name),
                    ));
                    continue;
                };
                let fields = def
                    .variants
                    .iter()
                    .flat_map(|variant| match &variant.fields {
                        HirVariantFields::Unit => Vec::new(),
                        HirVariantFields::Positional(fields) => fields.iter().collect(),
                        HirVariantFields::Named(fields) => {
                            fields.iter().map(|field| &field.ty).collect()
                        }
                    })
                    .collect();
                (id, args, fields)
            }
            _ => {
                errors.push(not_nominal());
                continue;
            }
        };
        if drop_trait.is_some_and(|drop| {
            program.impls.values().any(|other| {
                other.trait_id == Some(drop)
                    && impl_lookup::impl_receiver_owner_matches(
                        receiver,
                        other,
                        &program.structs,
                        &program.enums,
                    )
            })
        }) {
            errors.push((
                imp.id,
                format!(
                    "{} cannot implement Copy because it implements Drop",
                    imp.type_name
                ),
            ));
        }
        let subst = args
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, ty)| {
                (
                    GenericParamId {
                        owner: *id,
                        index: index as u32,
                    },
                    ty,
                )
            })
            .collect();
        for field in fields {
            let field = field.substitute_generics(&subst);
            let proved = impl_lookup::impl_exists_for(
                &field,
                copy,
                &[],
                &program.impls,
                &program.structs,
                &program.enums,
                builtin,
                &|ty, trait_id, args| match ty {
                    Type::Generic(param) => imp.bounds.get(param).is_some_and(|bounds| {
                        bounds
                            .iter()
                            .any(|bound| bound.trait_id == trait_id && bound.type_args == args)
                    }),
                    _ => false,
                },
            );
            if !proved {
                errors.push((
                    imp.id,
                    format!(
                        "{} cannot implement Copy because a field of type `{}` is not Copy",
                        imp.type_name,
                        copy_diagnostic_type_name(program, imp, &field),
                    ),
                ));
                break;
            }
        }
    }
    errors
}

#[derive(Debug, Clone, Copy, Default)]
struct BuiltinTraitIds {
    sized: Option<DefId>,
    copy: Option<DefId>,
    fn_once: Option<DefId>,
    fn_mut: Option<DefId>,
    fn_trait: Option<DefId>,
    send: Option<DefId>,
    owned: Option<DefId>,
    sync: Option<DefId>,
}

impl BuiltinTraitIds {
    fn from_language_items(items: &LanguageItems<DefId>) -> Self {
        Self {
            sized: items.sized.as_ref().map(|items| items.trait_id),
            copy: items.copy,
            fn_once: items.fn_once.as_ref().map(|items| items.trait_id),
            fn_mut: items.fn_mut.as_ref().map(|items| items.trait_id),
            fn_trait: items.fn_trait.as_ref().map(|items| items.trait_id),
            send: items.send.as_ref().map(|items| items.trait_id),
            owned: items.owned.as_ref().map(|items| items.trait_id),
            sync: items.sync.as_ref().map(|items| items.trait_id),
        }
    }
}

/// Result of constraint solving.
pub struct SolveResult {
    /// Warnings (non-fatal: ambiguous literals that were defaulted).
    pub warnings: Vec<String>,
    /// Hard errors: concrete type violates a required trait bound.
    pub errors: Vec<SolveError>,
    /// TypeVar ids that remain free but have at least one trait bound attached.
    pub generic_bounds: HashMap<TypeVarId, Vec<TraitBound>>,
    /// Representatives changed while this worklist invocation was running.
    pub changed_type_vars: Vec<TypeVarId>,
    /// Final-pass trait obligations on a concrete subject whose trait arguments are
    /// still free (`String: From ?x`). The caller either turns them into where
    /// clauses of the owning function during generalization or reports them.
    pub deferred_predicates: Vec<crate::infer::constraints::ObligationId>,
}

/// Solve all constraints in `store`.
///
/// `impls` is the list of all known trait implementations.
/// Trait violations on concrete types are always hard errors.
pub fn solve_constraints(
    engine: &mut InferenceEngine,
    store: &ConstraintStore,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
    language_items: &LanguageItems<DefId>,
) -> SolveResult {
    let mut store = store.clone();
    solve_constraints_in_place(
        engine,
        &mut store,
        impls,
        structs,
        enums,
        traits,
        language_items,
    )
}

pub fn solve_constraints_in_place(
    engine: &mut InferenceEngine,
    store: &mut ConstraintStore,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
    language_items: &LanguageItems<DefId>,
) -> SolveResult {
    solve_constraints_in_place_mode(
        engine,
        store,
        impls,
        structs,
        enums,
        traits,
        language_items,
        true,
        None,
        &crate::hir::HirGenericBounds::new(),
        &HashSet::new(),
    )
}

pub(crate) fn solve_constraints_in_place_for_owners(
    engine: &mut InferenceEngine,
    store: &mut ConstraintStore,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
    language_items: &LanguageItems<DefId>,
    owners: &HashSet<ConstraintOwner>,
    finalize_pending: bool,
    assumptions: &crate::hir::HirGenericBounds,
    generalizable: &HashSet<TypeVarId>,
) -> SolveResult {
    solve_constraints_in_place_mode(
        engine,
        store,
        impls,
        structs,
        enums,
        traits,
        language_items,
        finalize_pending,
        Some(owners),
        assumptions,
        generalizable,
    )
}

#[cfg(test)]
mod tests;
