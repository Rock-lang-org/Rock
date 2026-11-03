use std::collections::HashMap;

use crate::hir::{HirImpl, HirImplReceiverPattern, HirMethodCallTarget};
use crate::type_services::normalize::apply_type_lambda;
use crate::types::{FunctionSafety, GenericParamDecl, GenericParamId, Type};

pub fn receiver_pattern(ty: &Type) -> Vec<Type> {
    match ty {
        Type::Struct { args, .. } | Type::Enum { args, .. } => args.clone(),
        Type::Slice(inner) | Type::Array(inner, _) | Type::Pointer(inner) => {
            vec![inner.as_ref().clone()]
        }
        Type::Reference { inner, .. } => match inner.as_ref() {
            Type::Slice(elem) | Type::Array(elem, _) => vec![elem.as_ref().clone()],
            Type::Str => Vec::new(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

pub fn receiver_pattern_substitution(
    pattern: &HirImplReceiverPattern,
    receiver_ty: &Type,
) -> Option<HashMap<GenericParamId, Type>> {
    let mut substitution = HashMap::new();
    match pattern {
        HirImplReceiverPattern::Exact(expected) => {
            let try_match = |expected: &Type, actual: &Type| {
                let mut substitution = HashMap::new();
                type_pattern_matches(expected, actual, &mut substitution).then_some(substitution)
            };
            try_match(expected, receiver_ty)
                .or_else(|| match receiver_ty {
                    Type::Reference { inner, .. } => try_match(expected, inner),
                    _ => None,
                })
                .or_else(|| match expected {
                    Type::Reference { inner, .. } => try_match(inner, receiver_ty),
                    _ => None,
                })
                .or_else(|| match (expected, receiver_ty) {
                    (
                        Type::Reference {
                            mutable: false,
                            inner: expected_inner,
                        },
                        Type::Reference {
                            inner: actual_inner,
                            ..
                        },
                    ) => try_match(expected_inner, actual_inner),
                    _ => None,
                })
        }
        HirImplReceiverPattern::SliceFamily { element } => {
            let actual_element = match receiver_ty {
                Type::Slice(element) | Type::Array(element, _) => Some(element.as_ref()),
                Type::Reference { inner, .. } => match inner.as_ref() {
                    Type::Slice(element) | Type::Array(element, _) => Some(element.as_ref()),
                    _ => None,
                },
                _ => None,
            }?;
            type_pattern_matches(element, actual_element, &mut substitution).then_some(substitution)
        }
        HirImplReceiverPattern::Constructor(expected) => {
            type_pattern_matches(expected, receiver_ty, &mut substitution).then_some(substitution)
        }
    }
}

pub fn constructor_target_substitution(
    pattern: &HirImplReceiverPattern,
    target: &Type,
) -> Option<HashMap<GenericParamId, Type>> {
    let HirImplReceiverPattern::Constructor(expected) = pattern else {
        return None;
    };
    let mut substitution = HashMap::new();
    type_pattern_matches(expected, target, &mut substitution).then_some(substitution)
}

/// Match a higher-kinded blanket receiver using its constructor bounds.
/// For `F T where F _: Foldable`, the Foldable impl determines whether a
/// concrete carrier belongs to `Vec`, `Option`, or a section such as `Result _, E`.
pub fn impl_receiver_pattern_substitution<'a, P, I>(
    imp: &crate::hir::HirImplFor<P>,
    actual: &Type,
    impls: I,
) -> Option<HashMap<GenericParamId, Type>>
where
    P: crate::hir::HirPhase + 'a,
    I: IntoIterator<Item = &'a crate::hir::HirImplFor<P>>,
    I::IntoIter: Clone,
{
    let pattern = P::impl_receiver_pattern(&imp.receiver_pattern)?;
    if let Some(subst) = receiver_pattern_substitution(pattern, actual) {
        return Some(subst);
    }
    let HirImplReceiverPattern::Exact(expected) = pattern else {
        return None;
    };
    // Keep a repeatable borrowed view; only constructor-bound resolution needs
    // to walk the registry, not ordinary receiver mismatches.
    let impls = impls.into_iter();
    bounded_receiver_substitution(imp, expected, actual, &impls)
}

fn bounded_receiver_substitution<'a, P, I>(
    imp: &crate::hir::HirImplFor<P>,
    expected: &Type,
    actual: &Type,
    impls: &I,
) -> Option<HashMap<GenericParamId, Type>>
where
    P: crate::hir::HirPhase + 'a,
    I: Iterator<Item = &'a crate::hir::HirImplFor<P>> + Clone,
{
    let mut subst = HashMap::new();
    if type_pattern_matches(expected, actual, &mut subst) {
        return Some(subst);
    }
    // Constructor arguments can themselves be constructor applications, as in
    // T (G A). Resolve each nested head from its own bounds before validating
    // the whole receiver, including repeated generic parameters.
    if let (
        Type::Struct {
            id: left,
            args: expected_args,
        }
        | Type::Enum {
            id: left,
            args: expected_args,
        },
        Type::Struct {
            id: right,
            args: actual_args,
        }
        | Type::Enum {
            id: right,
            args: actual_args,
        },
    ) = (expected, actual)
    {
        if left != right || expected_args.len() != actual_args.len() {
            return None;
        }
        for (expected_arg, actual_arg) in expected_args.iter().zip(actual_args) {
            let nested = bounded_receiver_substitution(
                imp,
                &expected_arg.substitute_generics(&subst),
                actual_arg,
                impls,
            )?;
            subst.extend(nested);
        }
        return type_pattern_matches(expected, actual, &mut subst).then_some(subst);
    }
    let Type::Apply { constructor, args } = expected else {
        return None;
    };
    if let Some(applied) = apply_constructor_pattern(constructor, args) {
        return bounded_receiver_substitution(imp, &applied, actual, impls);
    }
    let Type::Generic(head) = constructor.as_ref() else {
        return None;
    };
    let bounds = imp.bounds.get(head)?;
    let mut candidates = Vec::new();
    for constructor_impl in impls.clone() {
        for bound in bounds {
            if constructor_impl.trait_id != Some(bound.trait_id)
                || constructor_impl.trait_arg_types.len() != bound.type_args.len()
            {
                continue;
            }
            let Some(target_pattern) = P::impl_receiver_pattern(&constructor_impl.receiver_pattern)
            else {
                continue;
            };
            let Some((target, mut target_subst)) =
                constructor_target_from_applied_type(target_pattern, actual)
            else {
                continue;
            };
            let Some(applied) = apply_constructor_pattern(&target, args) else {
                continue;
            };
            let Some(mut subst) = bounded_receiver_substitution(imp, &applied, actual, impls)
            else {
                continue;
            };
            if subst.get(head).is_some_and(|existing| *existing != target) {
                continue;
            }
            subst.insert(*head, target);
            if !constructor_impl
                .trait_arg_types
                .iter()
                .zip(&bound.type_args)
                .all(|(expected, required)| {
                    type_pattern_matches(
                        expected,
                        &required.substitute_generics(&subst),
                        &mut target_subst,
                    )
                })
            {
                continue;
            }
            if !candidates.contains(&subst) {
                candidates.push(subst);
            }
        }
    }
    if candidates.len() == 1 {
        candidates.pop()
    } else {
        None
    }
}

fn apply_constructor_pattern(constructor: &Type, args: &[Type]) -> Option<Type> {
    match constructor {
        Type::Constructor { id, flavor } => Some(match flavor {
            crate::types::NominalTypeKind::Struct => Type::Struct {
                id: *id,
                args: args.to_vec(),
            },
            crate::types::NominalTypeKind::Enum => Type::Enum {
                id: *id,
                args: args.to_vec(),
            },
            crate::types::NominalTypeKind::Alias => return None,
        }),
        Type::Lambda { .. } => apply_type_lambda(constructor, args),
        _ => None,
    }
}

pub fn constructor_target_from_applied_type(
    pattern: &HirImplReceiverPattern,
    actual: &Type,
) -> Option<(Type, HashMap<GenericParamId, Type>)> {
    let HirImplReceiverPattern::Constructor(target) = pattern else {
        return None;
    };

    if matches!(target, Type::Constructor { .. }) {
        let (actual_id, actual_flavor) = match actual {
            Type::Struct { id, .. } => (*id, crate::types::NominalTypeKind::Struct),
            Type::Enum { id, .. } => (*id, crate::types::NominalTypeKind::Enum),
            _ => return None,
        };
        return match target {
            Type::Constructor { id, flavor } if *id == actual_id && *flavor == actual_flavor => {
                Some((target.clone(), HashMap::new()))
            }
            _ => None,
        };
    }

    let Type::Lambda { params, body } = target else {
        return None;
    };
    let mut args = vec![None; params.len()];
    if !collect_section_arguments(body, actual, 0, &mut args) {
        return None;
    }
    let args = args.into_iter().collect::<Option<Vec<_>>>()?;
    let applied = apply_type_lambda(target, &args)?;
    let mut subst = HashMap::new();
    if !type_pattern_matches(&applied, actual, &mut subst) {
        return None;
    }
    Some((target.substitute_generics(&subst), subst))
}

fn collect_section_arguments(
    pattern: &Type,
    actual: &Type,
    lambda_depth: u32,
    args: &mut [Option<Type>],
) -> bool {
    match (pattern, actual) {
        (Type::BoundVar { depth, index, .. }, actual) if *depth == lambda_depth => {
            let Some(slot) = args.get_mut(*index as usize) else {
                return false;
            };
            match slot {
                Some(existing) => existing == actual,
                None => {
                    *slot = Some(actual.clone());
                    true
                }
            }
        }
        (Type::Generic(_), _) => true,
        (
            Type::Struct {
                id: pattern_id,
                args: pattern_args,
            },
            Type::Struct {
                id: actual_id,
                args: actual_args,
            },
        )
        | (
            Type::Enum {
                id: pattern_id,
                args: pattern_args,
            },
            Type::Enum {
                id: actual_id,
                args: actual_args,
            },
        ) => {
            pattern_id == actual_id
                && pattern_args.len() == actual_args.len()
                && pattern_args
                    .iter()
                    .zip(actual_args)
                    .all(|(pattern, actual)| {
                        collect_section_arguments(pattern, actual, lambda_depth, args)
                    })
        }
        (
            Type::Apply {
                constructor: pattern_constructor,
                args: pattern_args,
            },
            Type::Apply {
                constructor: actual_constructor,
                args: actual_args,
            },
        ) => {
            pattern_args.len() == actual_args.len()
                && collect_section_arguments(
                    pattern_constructor,
                    actual_constructor,
                    lambda_depth,
                    args,
                )
                && pattern_args
                    .iter()
                    .zip(actual_args)
                    .all(|(pattern, actual)| {
                        collect_section_arguments(pattern, actual, lambda_depth, args)
                    })
        }
        (Type::Tuple(pattern_items), Type::Tuple(actual_items)) => {
            pattern_items.len() == actual_items.len()
                && pattern_items
                    .iter()
                    .zip(actual_items)
                    .all(|(pattern, actual)| {
                        collect_section_arguments(pattern, actual, lambda_depth, args)
                    })
        }
        (
            Type::Function {
                params: pattern_params,
                ret: pattern_ret,
                ..
            },
            Type::Function {
                params: actual_params,
                ret: actual_ret,
                ..
            },
        ) => {
            pattern_params.len() == actual_params.len()
                && pattern_params
                    .iter()
                    .zip(actual_params)
                    .all(|(pattern, actual)| {
                        collect_section_arguments(pattern, actual, lambda_depth, args)
                    })
                && collect_section_arguments(pattern_ret, actual_ret, lambda_depth, args)
        }
        (Type::Slice(pattern), Type::Slice(actual))
        | (Type::Pointer(pattern), Type::Pointer(actual)) => {
            collect_section_arguments(pattern, actual, lambda_depth, args)
        }
        (Type::Array(pattern, pattern_len), Type::Array(actual, actual_len)) => {
            pattern_len == actual_len
                && collect_section_arguments(pattern, actual, lambda_depth, args)
        }
        (
            Type::Reference {
                mutable: pattern_mutable,
                inner: pattern,
            },
            Type::Reference {
                mutable: actual_mutable,
                inner: actual,
            },
        ) => {
            pattern_mutable == actual_mutable
                && collect_section_arguments(pattern, actual, lambda_depth, args)
        }
        (
            Type::Lambda {
                params: pattern_params,
                body: pattern_body,
            },
            Type::Lambda {
                params: actual_params,
                body: actual_body,
            },
        ) => {
            pattern_params == actual_params
                && collect_section_arguments(pattern_body, actual_body, lambda_depth + 1, args)
        }
        _ => pattern == actual,
    }
}

pub(crate) fn type_pattern_matches_after_subst(pattern: &Type, actual: &Type) -> bool {
    if let Type::Apply { constructor, args } = pattern {
        if let Some(applied) = apply_constructor_pattern(constructor, args) {
            return type_pattern_matches_after_subst(&applied, actual);
        }
    }
    match (pattern, actual) {
        (
            Type::Struct {
                id: expected_id,
                args: expected_args,
            },
            Type::Struct {
                id: actual_id,
                args: actual_args,
            },
        )
        | (
            Type::Enum {
                id: expected_id,
                args: expected_args,
            },
            Type::Enum {
                id: actual_id,
                args: actual_args,
            },
        ) => {
            expected_args.len() == actual_args.len()
                && expected_id == actual_id
                && expected_args
                    .iter()
                    .zip(actual_args.iter())
                    .all(|(expected, actual)| type_pattern_matches_after_subst(expected, actual))
        }
        (Type::Tuple(expected_elems), Type::Tuple(actual_elems)) => {
            expected_elems.len() == actual_elems.len()
                && expected_elems
                    .iter()
                    .zip(actual_elems.iter())
                    .all(|(expected, actual)| type_pattern_matches_after_subst(expected, actual))
        }
        (
            Type::Reference {
                mutable: left_mut,
                inner: left,
            },
            Type::Reference {
                mutable: right_mut,
                inner: right,
            },
        ) => {
            left_mut == right_mut
                && if *left_mut {
                    type_pattern_matches_invariant(left, right)
                } else {
                    type_pattern_matches_after_subst(left, right)
                }
        }
        (
            Type::Function {
                params: expected_params,
                ret: expected_ret,
                safety: expected_safety,
                callable_kind: expected_kind,
                captures: expected_captures,
                ..
            },
            Type::Function {
                params: actual_params,
                ret: actual_ret,
                safety: actual_safety,
                callable_kind: actual_kind,
                captures: actual_captures,
                ..
            },
        ) => {
            let safety_matches = !matches!(actual_safety, FunctionSafety::Unsafe)
                || !matches!(expected_safety, FunctionSafety::Safe);
            safety_matches
                && (expected_captures.is_empty() || actual_kind <= expected_kind)
                && (expected_captures.is_empty() || expected_captures == actual_captures)
                && expected_params.len() == actual_params.len()
                && expected_params
                    .iter()
                    .zip(actual_params.iter())
                    .all(|(expected, actual)| type_pattern_matches_after_subst(actual, expected))
                && type_pattern_matches_after_subst(expected_ret, actual_ret)
        }
        (Type::Slice(left), Type::Slice(right)) => type_pattern_matches_after_subst(left, right),
        (Type::Pointer(left), Type::Pointer(right)) => type_pattern_matches_invariant(left, right),
        (Type::Array(left, left_len), Type::Array(right, right_len)) => {
            left_len == right_len && type_pattern_matches_after_subst(left, right)
        }
        (
            Type::Constructor {
                id: left_id,
                flavor: left_flavor,
            },
            Type::Constructor {
                id: right_id,
                flavor: right_flavor,
            },
        ) => left_id == right_id && left_flavor == right_flavor,
        (
            Type::Apply {
                constructor: left_constructor,
                args: left_args,
            },
            Type::Apply {
                constructor: right_constructor,
                args: right_args,
            },
        ) => {
            left_args.len() == right_args.len()
                && type_pattern_matches_after_subst(left_constructor, right_constructor)
                && left_args
                    .iter()
                    .zip(right_args)
                    .all(|(left, right)| type_pattern_matches_after_subst(left, right))
        }
        (
            Type::Lambda {
                params: left_params,
                body: left_body,
            },
            Type::Lambda {
                params: right_params,
                body: right_body,
            },
        ) => left_params == right_params && type_pattern_matches_after_subst(left_body, right_body),
        (
            Type::Projection {
                ty: expected_ty,
                trait_id: expected_trait,
                assoc_type: expected_assoc,
                trait_args: expected_args,
            },
            Type::Projection {
                ty: actual_ty,
                trait_id: actual_trait,
                assoc_type: actual_assoc,
                trait_args: actual_args,
            },
        ) => {
            expected_trait == actual_trait
                && expected_assoc == actual_assoc
                && expected_args.len() == actual_args.len()
                && type_pattern_matches_after_subst(expected_ty, actual_ty)
                && expected_args
                    .iter()
                    .zip(actual_args.iter())
                    .all(|(expected, actual)| type_pattern_matches_after_subst(expected, actual))
        }
        _ => pattern == actual,
    }
}

fn type_pattern_matches_invariant(left: &Type, right: &Type) -> bool {
    type_pattern_matches_after_subst(left, right) && type_pattern_matches_after_subst(right, left)
}

pub fn type_pattern_matches(
    pattern: &Type,
    actual: &Type,
    subst: &mut HashMap<GenericParamId, Type>,
) -> bool {
    let mut next_subst = subst.clone();
    infer_generic_subst_from_types(pattern, actual, &mut next_subst);
    if type_pattern_matches_after_subst(&pattern.substitute_generics(&next_subst), actual) {
        *subst = next_subst;
        true
    } else {
        false
    }
}

pub fn infer_generic_subst_from_types(
    expected: &Type,
    actual: &Type,
    subst: &mut HashMap<GenericParamId, Type>,
) {
    match expected {
        Type::Generic(param) => {
            subst.entry(*param).or_insert_with(|| actual.clone());
        }
        Type::Slice(inner_expected) => {
            if let Type::Slice(inner_actual) = actual {
                infer_generic_subst_from_types(inner_expected, inner_actual, subst);
            }
        }
        Type::Array(inner_expected, expected_len) => {
            if let Type::Array(inner_actual, actual_len) = actual {
                if expected_len == actual_len {
                    infer_generic_subst_from_types(inner_expected, inner_actual, subst);
                }
            }
        }
        Type::Tuple(expected_elems) => {
            if let Type::Tuple(actual_elems) = actual {
                for (expected_elem, actual_elem) in expected_elems.iter().zip(actual_elems.iter()) {
                    infer_generic_subst_from_types(expected_elem, actual_elem, subst);
                }
            }
        }
        Type::Function {
            params: expected_args,
            ret: expected_ret,
            ..
        } => {
            if let Type::Function {
                params: actual_args,
                ret: actual_ret,
                ..
            } = actual
            {
                for (expected_arg, actual_arg) in expected_args.iter().zip(actual_args.iter()) {
                    infer_generic_subst_from_types(expected_arg, actual_arg, subst);
                }
                infer_generic_subst_from_types(expected_ret, actual_ret, subst);
            }
        }
        Type::Struct {
            id: expected_id,
            args: expected_generics,
        } => match actual {
            Type::Struct {
                id: actual_id,
                args: actual_generics,
            } if expected_generics.len() == actual_generics.len() && expected_id == actual_id => {
                for (expected_generic, actual_generic) in
                    expected_generics.iter().zip(actual_generics.iter())
                {
                    infer_generic_subst_from_types(expected_generic, actual_generic, subst);
                }
            }
            _ => {}
        },
        Type::Enum {
            id: expected_id,
            args: expected_generics,
        } => match actual {
            Type::Enum {
                id: actual_id,
                args: actual_generics,
            } if expected_generics.len() == actual_generics.len() && expected_id == actual_id => {
                for (expected_generic, actual_generic) in
                    expected_generics.iter().zip(actual_generics.iter())
                {
                    infer_generic_subst_from_types(expected_generic, actual_generic, subst);
                }
            }
            _ => {}
        },
        Type::Reference {
            mutable: expected_mutable,
            inner: expected_inner,
        } => {
            if let Type::Reference {
                mutable: actual_mutable,
                inner: actual_inner,
            } = actual
            {
                if expected_mutable == actual_mutable {
                    infer_generic_subst_from_types(expected_inner, actual_inner, subst);
                }
            }
        }
        Type::Pointer(expected_inner) => {
            if let Type::Pointer(actual_inner) = actual {
                infer_generic_subst_from_types(expected_inner, actual_inner, subst);
            }
        }
        Type::Projection {
            ty: expected_ty,
            trait_id: expected_trait,
            assoc_type: expected_assoc,
            trait_args: expected_args,
        } => {
            if let Type::Projection {
                ty: actual_ty,
                trait_id: actual_trait,
                assoc_type: actual_assoc,
                trait_args: actual_args,
            } = actual
            {
                if expected_trait == actual_trait
                    && expected_assoc == actual_assoc
                    && expected_args.len() == actual_args.len()
                {
                    infer_generic_subst_from_types(expected_ty, actual_ty, subst);
                    for (expected_arg, actual_arg) in expected_args.iter().zip(actual_args.iter()) {
                        infer_generic_subst_from_types(expected_arg, actual_arg, subst);
                    }
                }
            }
        }
        Type::Apply {
            constructor: expected_constructor,
            args: expected_args,
        } => {
            if let Type::Apply {
                constructor: actual_constructor,
                args: actual_args,
            } = actual
            {
                if expected_args.len() == actual_args.len() {
                    infer_generic_subst_from_types(expected_constructor, actual_constructor, subst);
                    for (expected_arg, actual_arg) in expected_args.iter().zip(actual_args) {
                        infer_generic_subst_from_types(expected_arg, actual_arg, subst);
                    }
                }
            }
        }
        Type::Lambda {
            params: expected_params,
            body: expected_body,
        } => {
            if let Type::Lambda {
                params: actual_params,
                body: actual_body,
            } = actual
            {
                if expected_params == actual_params {
                    infer_generic_subst_from_types(expected_body, actual_body, subst);
                }
            }
        }
        _ => {}
    }
}

pub fn seed_receiver_substitution_from_impl(
    imp: &HirImpl,
    recv_ty: &Type,
    subst: &mut HashMap<GenericParamId, Type>,
) {
    if let Some(receiver_substitution) =
        receiver_pattern_substitution(&imp.receiver_pattern, recv_ty)
    {
        subst.extend(receiver_substitution);
    }

    for (decl, concrete) in imp.type_generics.iter().zip(receiver_pattern(recv_ty)) {
        subst.entry(decl.id).or_insert(concrete);
    }
}

pub fn generic_substitution_for_owner(
    params: &[GenericParamDecl],
    args: &[Type],
) -> HashMap<GenericParamId, Type> {
    params
        .iter()
        .zip(args.iter())
        .map(|(decl, arg)| (decl.id, arg.clone()))
        .collect()
}

pub fn target_matches_impl<P: crate::hir::HirPhase>(
    imp: &crate::hir::HirImplFor<P>,
    target: Option<&HirMethodCallTarget>,
) -> bool {
    let Some(target) = target else {
        return true;
    };

    if let Some(impl_id) = target.impl_id() {
        if imp.id != impl_id {
            return false;
        }
    }

    if let Some(trait_id) = target.trait_id() {
        if imp.trait_id != Some(trait_id) {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests;
