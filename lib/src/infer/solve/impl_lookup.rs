use std::collections::HashMap;

use crate::hir::{HirEnum, HirImpl, HirImplReceiverPattern, HirStruct};
use crate::ids::DefId;
use crate::types::{CallableKind, Type};

use super::auto_traits::{auto_trait_satisfied, type_is_sized};
use super::BuiltinTraitIds;

/// Check whether `ty` has an impl of the canonical trait ID in the given impl list.
///
/// Matches on the `type_name` field of `HirImpl`. Generic impls (with
/// `type_generics` non-empty, e.g. `impl<T> Show for Vec<T>`) are treated as
/// matching any type of the right shape.
pub(super) fn impl_exists_for(
    ty: &Type,
    trait_id: DefId,
    trait_args: &[Type],
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
    assumed: &dyn Fn(&Type, DefId, &[Type]) -> bool,
) -> bool {
    if assumed(ty, trait_id, trait_args) {
        return true;
    }
    if builtin_traits.sized == Some(trait_id) {
        return type_is_sized(ty);
    }

    if let Some(satisfied) = callable_trait_satisfied(ty, trait_id, trait_args, builtin_traits) {
        return satisfied;
    }

    if explicit_impl_exists_for(
        ty,
        trait_id,
        trait_args,
        impls,
        structs,
        enums,
        builtin_traits,
        assumed,
    ) {
        return true;
    }

    auto_trait_satisfied(
        ty,
        trait_id,
        impls,
        structs,
        enums,
        builtin_traits,
        &mut std::collections::HashSet::new(),
        assumed,
    )
}

pub(super) fn explicit_impl_exists_for(
    ty: &Type,
    trait_id: DefId,
    trait_args: &[Type],
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
    assumed: &dyn Fn(&Type, DefId, &[Type]) -> bool,
) -> bool {
    let marker_provider_crate =
        if builtin_traits.send == Some(trait_id) || builtin_traits.sync == Some(trait_id)
            || builtin_traits.owned == Some(trait_id) {
            Some(trait_id.crate_id)
        } else {
            None
        };
    let mut impl_ids = impls.keys().copied().collect::<Vec<_>>();
    impl_ids.sort();
    impl_ids.into_iter().any(|id| {
        if marker_provider_crate.is_some_and(|provider| id.crate_id != provider) {
            return false;
        }
        let Some(imp) = impls.get(&id) else {
            return false;
        };
        if imp.trait_id != Some(trait_id) || !impl_receiver_owner_matches(ty, imp, structs, enums) {
            return false;
        }

        let Some(mut subst) = callable_impl_substitution(imp, ty, trait_id, builtin_traits, impls)
        else {
            return false;
        };

        impl_trait_args_match(&imp.trait_arg_types, trait_args, &mut subst)
            && impl_bounds_satisfied(imp, &subst, impls, structs, enums, builtin_traits, assumed)
    })
}

pub(super) fn impl_bounds_satisfied(
    imp: &HirImpl,
    subst: &HashMap<crate::types::GenericParamId, Type>,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
    assumed: &dyn Fn(&Type, DefId, &[Type]) -> bool,
) -> bool {
    imp.bounds.iter().all(|(generic_param, bounds)| {
        let Some(ty) = subst.get(generic_param) else {
            return false;
        };

        bounds.iter().all(|bound| {
            let type_args: Vec<Type> = bound
                .type_args
                .iter()
                .map(|arg| arg.substitute_generics(subst))
                .collect();
            impl_exists_for(
                ty,
                bound.trait_id,
                &type_args,
                impls,
                structs,
                enums,
                builtin_traits,
                assumed,
            )
        })
    })
}

pub(super) fn callable_trait_satisfied(
    ty: &Type,
    trait_id: DefId,
    trait_args: &[Type],
    builtin_traits: BuiltinTraitIds,
) -> Option<bool> {
    let required_kind = if builtin_traits.fn_trait == Some(trait_id) {
        CallableKind::Fn
    } else if builtin_traits.fn_mut == Some(trait_id) {
        CallableKind::FnMut
    } else if builtin_traits.fn_once == Some(trait_id) {
        CallableKind::FnOnce
    } else {
        return None;
    };
    let Type::Function {
        params,
        ret,
        callable_kind,
        safety,
        ..
    } = ty
    else {
        return None;
    };
    if trait_args.len() != 2
        || *callable_kind > required_kind
        || *safety == crate::types::FunctionSafety::Unsafe
    {
        return Some(false);
    }
    let args = match params.as_slice() {
        [] => Type::Unit,
        [param] => param.clone(),
        params => Type::Tuple(params.to_vec()),
    };
    let args_match = trait_args[0] == args
        || matches!((&trait_args[0], &args), (Type::Tuple(elements), Type::Unit) if elements.is_empty());
    Some(args_match && trait_args[1] == **ret)
}

pub(super) fn callable_impl_substitution(
    imp: &HirImpl,
    ty: &Type,
    trait_id: DefId,
    builtin: BuiltinTraitIds,
    impls: &HashMap<DefId, HirImpl>,
) -> Option<HashMap<crate::types::GenericParamId, Type>> {
    if [builtin.fn_trait, builtin.fn_mut, builtin.fn_once].contains(&Some(trait_id)) {
        if let crate::hir::HirImplReceiverPattern::Exact(pattern) = &imp.receiver_pattern {
            let mut subst = HashMap::new();
            return crate::selection::type_pattern_matches(pattern, ty, &mut subst)
                .then_some(subst);
        }
    }
    crate::selection::impl_receiver_pattern_substitution(imp, ty, impls.values())
}

fn impl_trait_args_match(
    expected: &[Type],
    actual: &[Type],
    subst: &mut HashMap<crate::types::GenericParamId, Type>,
) -> bool {
    if expected.len() != actual.len() {
        return false;
    }

    expected
        .iter()
        .zip(actual.iter())
        .all(|(expected, actual)| {
            matches!((expected, actual), (Type::Tuple(items), Type::Unit) if items.is_empty())
                || matches!((expected, actual), (Type::Unit, Type::Tuple(items)) if items.is_empty())
                || crate::selection::type_pattern_matches(expected, actual, subst)
        })
}

pub(super) fn impl_receiver_owner_matches(
    ty: &Type,
    imp: &HirImpl,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
) -> bool {
    if let (
        Type::Reference { .. },
        HirImplReceiverPattern::Exact(pattern @ Type::Reference { .. }),
    ) = (ty, &imp.receiver_pattern)
    {
        // Generic borrowed receivers have different display spellings once
        // instantiated; compare their structure and nominal IDs instead.
        return crate::selection::type_pattern_matches(pattern, ty, &mut HashMap::new());
    }
    if matches!(
        &imp.receiver_pattern,
        HirImplReceiverPattern::Exact(Type::Generic(_))
    ) || matches!(
        &imp.receiver_pattern,
        HirImplReceiverPattern::Exact(Type::Apply { constructor, .. })
            if matches!(constructor.as_ref(), Type::Generic(_))
    ) {
        return true;
    }
    if let Some(id) = constructor_owner_id(ty) {
        return (structs.contains_key(&id) || enums.contains_key(&id))
            && receiver_owner_id(imp) == Some(id);
    }
    match ty {
        Type::Struct { id, .. } => structs.contains_key(id) && receiver_owner_id(imp) == Some(*id),
        Type::Enum { id, .. } => enums.contains_key(id) && receiver_owner_id(imp) == Some(*id),
        _ => {
            let ty_name = type_base_name(ty);
            imp.type_name == ty_name
                || (!imp.type_generics.is_empty() && type_shape_matches(&imp.type_name, &ty_name))
        }
    }
}

fn constructor_owner_id(ty: &Type) -> Option<DefId> {
    match ty {
        Type::Constructor { id, .. } | Type::Struct { id, .. } | Type::Enum { id, .. } => Some(*id),
        Type::Apply { constructor, .. } => constructor_owner_id(constructor),
        Type::Lambda { body, .. } => constructor_owner_id(body),
        _ => None,
    }
}

fn receiver_owner_id(imp: &HirImpl) -> Option<DefId> {
    match &imp.receiver_pattern {
        crate::hir::HirImplReceiverPattern::Exact(Type::Struct { id, .. })
        | crate::hir::HirImplReceiverPattern::Exact(Type::Enum { id, .. }) => Some(*id),
        crate::hir::HirImplReceiverPattern::Constructor(ty) => constructor_owner_id(ty),
        crate::hir::HirImplReceiverPattern::Exact(_)
        | crate::hir::HirImplReceiverPattern::SliceFamily { .. } => None,
    }
}

fn fixed_array_len_from_type_name(type_name: &str) -> Option<usize> {
    let inner = type_name.strip_prefix('[')?.strip_suffix(']')?;
    let (_, len) = inner.split_once(';')?;
    len.trim().parse().ok()
}

fn type_shape_matches(candidate: &str, lookup: &str) -> bool {
    if candidate == lookup {
        return true;
    }

    match (
        fixed_array_len_from_type_name(candidate),
        fixed_array_len_from_type_name(lookup),
    ) {
        (Some(candidate_len), Some(lookup_len)) => candidate_len == lookup_len,
        _ => candidate == "Array" && lookup == "Array",
    }
}

/// Return the base name of a type for impl lookup (e.g. "I64", "Vec", "Point").
fn type_base_name(ty: &Type) -> String {
    match ty {
        Type::I8 => "I8".into(),
        Type::I16 => "I16".into(),
        Type::I32 => "I32".into(),
        Type::I64 => "I64".into(),
        Type::U8 => "U8".into(),
        Type::U16 => "U16".into(),
        Type::U32 => "U32".into(),
        Type::U64 => "U64".into(),
        Type::F32 => "F32".into(),
        Type::F64 => "F64".into(),
        Type::Bool => "Bool".into(),
        Type::Str => "Str".into(),
        Type::Char => "Char".into(),
        Type::Slice(_) => "Array".into(),
        Type::Array(_, _) => ty.to_string(),
        Type::Struct { .. } | Type::Enum { .. } => ty.to_string(),
        _ => ty.to_string(),
    }
}
