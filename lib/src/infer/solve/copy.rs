//! Copy semantics for programs with an explicit `copy` language item.
//!
//! Tuples and arrays are Copy structurally, structs and enums only through a
//! validated `impl Copy`. MIR is built only for monomorphized instances, so
//! the oracle never needs in-scope `T: Copy` assumptions; a stray
//! `Type::Generic` conservatively answers "not Copy" (a move).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::hir::{HirPhase, HirProgramFor};
use crate::ids::DefId;
use crate::type_services::facts::TypeFacts;
use crate::types::Type;

use super::{impl_lookup, BuiltinTraitIds};

/// Answers "is this concrete type Copy?" for one program, memoized.
///
/// Only types mentioning a struct or enum that has an `impl Copy` need the
/// impl table; everything else is decided structurally without a lookup.
pub(crate) struct CopyOracle<'p, P: HirPhase> {
    program: &'p HirProgramFor<P>,
    copy: Option<DefId>,
    /// Owners of `impl Copy` receivers; `None` when some impl has no
    /// nominal owner, which disables the fast path.
    owners: Option<HashSet<DefId>>,
    cache: RefCell<HashMap<Type, bool>>,
}

impl<'p, P: HirPhase> CopyOracle<'p, P> {
    pub(crate) fn new(program: &'p HirProgramFor<P>) -> Self {
        let copy = program.language_items.copy;
        let owners = copy.map_or(Some(HashSet::new()), |copy| {
            program
                .impls
                .values()
                .filter(|imp| imp.trait_id == Some(copy))
                .map(impl_lookup::receiver_owner_id)
                .collect()
        });
        Self {
            program,
            copy,
            owners,
            cache: RefCell::default(),
        }
    }

    pub(crate) fn is_copy(&self, ty: &Type) -> bool {
        let Some(copy) = self.copy.filter(|_| self.may_gain_copy(ty)) else {
            return TypeFacts::is_copy(ty);
        };
        if let Some(&answer) = self.cache.borrow().get(ty) {
            return answer;
        }
        let answer = impl_lookup::impl_exists_for(
            ty,
            copy,
            &[],
            &self.program.impls,
            &self.program.structs,
            &self.program.enums,
            BuiltinTraitIds::from_language_items(&self.program.language_items),
            &|_, _, _| false,
        );
        self.cache.borrow_mut().insert(ty.clone(), answer);
        answer
    }

    /// Whether `ty` structurally contains a nominal type with a Copy impl.
    fn may_gain_copy(&self, ty: &Type) -> bool {
        match ty {
            Type::Struct { id, .. } | Type::Enum { id, .. } => self
                .owners
                .as_ref()
                .map_or(true, |owners| owners.contains(id)),
            Type::Tuple(elements) => elements.iter().any(|element| self.may_gain_copy(element)),
            Type::Array(element, _) => self.may_gain_copy(element),
            _ => false,
        }
    }
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
