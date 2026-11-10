use super::*;

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
                if let ArrayLen::Param(param) = expected_len {
                    // A const parameter binds to whatever length the actual
                    // array carries (a concrete value or another parameter).
                    subst
                        .entry(*param)
                        .or_insert_with(|| actual_len.to_const_type());
                    infer_generic_subst_from_types(inner_expected, inner_actual, subst);
                } else if expected_len == actual_len {
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
