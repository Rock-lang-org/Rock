//! Callable binding shape, separate from measured borrowing/effect authority.

use crate::types::{CallableResultBorrowing, CallableResultSource as S, FunctionSafety, Type};

/// Resolve capture-derived capability after resolving child types. An abstract
/// environment supplies no proof and must retain its declared requirement.
pub(crate) fn resolve_environment_kind(mut ty: Type) -> Type {
    if let Type::Function {
        callable_kind,
        captures,
        ..
    } = &mut ty
    {
        if !captures.is_empty() {
            *callable_kind = crate::types::CallableKind::from_captures(captures);
        }
    }
    ty
}

pub(crate) fn resolve_environment_kinds(ty: Type) -> Type {
    struct Resolve;
    impl crate::type_services::visit::TypeFolder for Resolve {
        fn fold_type(&mut self, ty: Type) -> Type {
            resolve_environment_kind(crate::type_services::visit::fold_type_children(ty, self))
        }
    }
    crate::type_services::visit::fold_type(ty, &mut Resolve)
}

/// Instantiate a value's inferred representation, not its input contract.
/// Default arrow metadata is inference's wildcard; actual environments must
/// survive it for Copy, moves and destruction. No producer proof is supplied.
pub(crate) fn value_representation(inferred: &Type, actual: &Type) -> Option<Type> {
    fn fields(inferred: &[Type], actual: &[Type]) -> Option<Vec<Type>> {
        (inferred.len() == actual.len()).then_some(())?;
        inferred
            .iter()
            .zip(actual)
            .map(|(a, b)| value_representation(a, b))
            .collect()
    }
    Some(match (inferred, actual) {
        (
            Type::Function {
                params: a,
                ret: ar,
                safety: sa,
                callable_kind: ka,
                captures: ca,
                result_borrowing: ba,
            },
            Type::Function {
                params: b,
                ret: br,
                safety: sb,
                callable_kind: kb,
                captures: cb,
                result_borrowing: bb,
            },
        ) => {
            if sa != sb
                || (!ka.is_default(ca) && (ka != kb || ca != cb))
                || (ba != bb && *ba != CallableResultBorrowing::Unknown)
            {
                return None;
            }
            Type::Function {
                params: fields(a, b)?,
                ret: Box::new(value_representation(ar, br)?),
                safety: *sb,
                callable_kind: *kb,
                captures: cb.clone(),
                result_borrowing: bb.clone(),
            }
        }
        (Type::Tuple(a), Type::Tuple(b)) => Type::Tuple(fields(a, b)?),
        (Type::Array(a, a_len), Type::Array(b, b_len)) if a_len == b_len => {
            Type::Array(Box::new(value_representation(a, b)?), *a_len)
        }
        (
            Type::Reference {
                mutable: a_mutable,
                inner: a,
            },
            Type::Reference {
                mutable: b_mutable,
                inner: b,
            },
        ) if a_mutable == b_mutable
            && crate::type_services::visit::type_any(b, |ty| {
                matches!(
                    ty,
                    Type::StructRepresentation { .. } | Type::EnumRepresentation { .. }
                )
            }) =>
        {
            // A known safe loan retains its pointee's measured nominal fields;
            // this changes neither access nor the loan/source contract.
            Type::Reference {
                mutable: *a_mutable,
                inner: Box::new(value_representation(a, b)?),
            }
        }
        (
            Type::Struct { id: a, args: aa },
            Type::StructRepresentation {
                id: b,
                args: ba,
                fields: actual_fields,
            },
        ) if a == b => Type::StructRepresentation {
            id: *a,
            args: fields(aa, ba)?,
            fields: actual_fields.clone(),
        },
        (
            Type::StructRepresentation {
                id: a,
                args: aa,
                fields: af,
            },
            Type::StructRepresentation {
                id: b,
                args: ba,
                fields: bf,
            },
        ) if a == b => Type::StructRepresentation {
            id: *a,
            args: fields(aa, ba)?,
            fields: fields(af, bf)?,
        },
        (Type::Struct { id: a, args: aa }, Type::Struct { id: b, args: ba }) if a == b => {
            Type::Struct {
                id: *a,
                args: fields(aa, ba)?,
            }
        }
        (Type::Enum { id: a, args: aa }, Type::Enum { id: b, args: ba }) if a == b => Type::Enum {
            id: *a,
            args: fields(aa, ba)?,
        },
        (
            Type::Enum { id: a, args: aa },
            Type::EnumRepresentation {
                id: b,
                args: ba,
                variants,
            },
        ) if a == b => Type::EnumRepresentation {
            id: *a,
            args: fields(aa, ba)?,
            variants: variants.clone(),
        },
        (
            Type::EnumRepresentation {
                id: a,
                args: aa,
                variants: av,
            },
            Type::EnumRepresentation {
                id: b,
                args: ba,
                variants: bv,
            },
        ) if a == b => {
            if av.len() != bv.len() {
                return None;
            }
            Type::EnumRepresentation {
                id: *a,
                args: fields(aa, ba)?,
                variants: av
                    .iter()
                    .zip(bv)
                    .map(|(a, b)| fields(a, b))
                    .collect::<Option<_>>()?,
            }
        }
        (
            Type::Apply {
                constructor: a,
                args: aa,
            },
            Type::Apply {
                constructor: b,
                args: ba,
            },
        ) if a == b => Type::Apply {
            constructor: a.clone(),
            args: fields(aa, ba)?,
        },
        _ if inferred == actual => actual.clone(),
        _ => return None,
    })
}

/// Refine only arrow slots present in the declaration. A generic binder's
/// selected representation stays authoritative even when it contains arrows.
pub(crate) fn declared_value_representation(
    declared: &Type,
    specialized: &Type,
    actual: &Type,
) -> Option<Type> {
    if specialized == actual {
        return Some(specialized.clone());
    }
    fn fields(declared: &[Type], specialized: &[Type], actual: &[Type]) -> Option<Vec<Type>> {
        if declared.len() != specialized.len() || specialized.len() != actual.len() {
            return None;
        }
        declared
            .iter()
            .zip(specialized)
            .zip(actual)
            .map(|((d, s), a)| declared_value_representation(d, s, a))
            .collect()
    }
    Some(match (declared, specialized, actual) {
        (
            Type::Function {
                params: dp,
                ret: dr,
                ..
            },
            Type::Function {
                params: sp,
                ret: sr,
                ..
            },
            Type::Function {
                params: ap,
                ret: ar,
                ..
            },
        ) => {
            let mut refined = value_representation(specialized, actual)?;
            let Type::Function { params, ret, .. } = &mut refined else {
                unreachable!()
            };
            *params = fields(dp, sp, ap)?;
            **ret = declared_value_representation(dr, sr, ar)?;
            refined
        }
        (Type::Tuple(d), Type::Tuple(s), Type::Tuple(a)) => Type::Tuple(fields(d, s, a)?),
        (
            Type::Struct { id: di, args: d },
            Type::Struct { id: si, args: s },
            Type::Struct { id: ai, args: a },
        ) if di == si && si == ai => Type::Struct {
            id: *si,
            args: fields(d, s, a)?,
        },
        (
            Type::Enum { id: di, args: d },
            Type::Enum { id: si, args: s },
            Type::Enum { id: ai, args: a },
        ) if di == si && si == ai => Type::Enum {
            id: *si,
            args: fields(d, s, a)?,
        },
        // Generic leaves, unresolved constructor applications and storage are
        // not declaration-owned arrow slots.
        _ => return None,
    })
}

/// Conditional demand for an owned callable input: dependencies carried by its
/// value and arguments may escape, but the input's own storage may not.
/// This constructs a requirement, never producer, capture or effect evidence.
pub(crate) fn owned_result_requirement(params: &[Type]) -> Option<CallableResultBorrowing> {
    let mut sources = vec![
        S::EnvironmentValue,
        S::EnvironmentContents,
        S::Static,
        S::Unknown,
    ];
    for (index, parameter) in params.iter().enumerate() {
        let index = u32::try_from(index).ok()?;
        sources.push(S::ArgumentValue(index));
        if matches!(parameter, Type::Reference { .. } | Type::Pointer(_)) {
            sources.push(S::ArgumentContents(index));
        }
    }
    sources.sort();
    Some(CallableResultBorrowing::Sources(Box::new(sources)))
}

/// Match a concrete input to a canonical callable requirement without treating
/// its result-source header or abstract capture list as producer evidence.
pub(crate) fn input_shape_matches(required: &Type, actual: &Type) -> bool {
    let (
        Type::Function {
            params: required_args,
            ret: required_ret,
            safety: required_safety,
            callable_kind: required_kind,
            captures: required_captures,
            ..
        },
        Type::Function {
            params: actual_args,
            ret: actual_ret,
            safety: actual_safety,
            callable_kind: actual_kind,
            captures: actual_captures,
            ..
        },
    ) = (required, actual)
    else {
        return false;
    };
    required_args == actual_args
        && required_ret == actual_ret
        && !matches!(
            (actual_safety, required_safety),
            (FunctionSafety::Unsafe, FunctionSafety::Safe)
        )
        && actual_kind <= required_kind
        && (required_captures.is_empty() || required_captures == actual_captures)
}

/// Fn's canonical Args pack is not a native parameter list: `(A, B)` may
/// describe one tuple parameter or two parameters packed by the native adapter.
/// Shape agreement supplies neither producer nor result-borrowing authority.
pub(crate) fn packed_input_shape_matches(required: &Type, actual: &Type) -> bool {
    let (
        Type::Function {
            params: required_params,
            ..
        },
        Type::Function {
            params: actual_params,
            ..
        },
    ) = (required, actual)
    else {
        return false;
    };
    let pack = |params: &[Type]| match params {
        [] => Type::Unit,
        [ty] => ty.clone(),
        _ => Type::Tuple(params.to_vec()),
    };
    if pack(required_params) != pack(actual_params) {
        return false;
    }
    let mut physical = required.clone();
    let Type::Function { params, .. } = &mut physical else {
        unreachable!()
    };
    *params = actual_params.clone();
    input_shape_matches(&physical, actual)
}

/// Instantiate a canonical owned Fn demand at the native Args-pack ABI.
/// Only the canonical dependency envelope can be regenerated at new arity;
/// arbitrary or narrow source declarations are never broadened here.
pub(crate) fn physical_owned_input_requirement(required: &Type, actual: &Type) -> Option<Type> {
    if input_shape_matches(required, actual) {
        return Some(required.clone());
    }
    if !packed_input_shape_matches(required, actual) {
        return None;
    }
    let (
        Type::Function {
            params: canonical,
            result_borrowing: borrowing,
            ..
        },
        Type::Function {
            params: physical, ..
        },
    ) = (required, actual)
    else {
        return None;
    };
    if owned_result_requirement(canonical).as_ref() != Some(borrowing) {
        return None;
    }
    let mut instantiated = required.clone();
    let Type::Function {
        params,
        result_borrowing,
        ..
    } = &mut instantiated
    else {
        unreachable!()
    };
    *params = physical.clone();
    *result_borrowing = owned_result_requirement(params)?;
    Some(instantiated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_slots_cannot_refine_callable_layouts_introduced_by_generic_binders() {
        use crate::ids::{CrateId, DefId, LocalDefId};
        use crate::types::{CallableKind, CaptureKind, FunctionCapture, GenericParamId};

        let generic = Type::Generic(GenericParamId {
            owner: DefId::new(CrateId(0), LocalDefId(90)),
            index: 0,
        });
        let arrow = Type::function(vec![], Type::Unit);
        let actual = Type::function_with_metadata(
            vec![],
            Type::Unit,
            FunctionSafety::Safe,
            CallableKind::FnOnce,
            vec![FunctionCapture::new(
                CaptureKind::Move,
                Type::Array(Box::new(Type::I64), 2),
            )],
        );
        assert_eq!(
            declared_value_representation(&arrow, &arrow, &actual),
            Some(actual.clone())
        );
        assert!(declared_value_representation(&generic, &arrow, &actual).is_none());
        assert!(declared_value_representation(
            &Type::Tuple(vec![generic.clone()]),
            &Type::Tuple(vec![arrow.clone()]),
            &Type::Tuple(vec![actual.clone()]),
        )
        .is_none());
        let declared = Type::function(vec![generic], Type::Unit);
        let specialized = Type::function(vec![arrow], Type::Unit);
        let different_input = Type::function(vec![actual.clone()], Type::Unit);
        assert!(declared_value_representation(&declared, &specialized, &different_input).is_none());
        let mut explicit = actual.clone();
        let Type::Function {
            callable_kind,
            captures,
            ..
        } = &mut explicit
        else {
            unreachable!()
        };
        *callable_kind = CallableKind::FnMut;
        *captures = Box::new([]);
        assert!(declared_value_representation(&explicit, &explicit, &actual).is_none());
    }

    #[test]
    fn value_representation_does_not_change_declared_protocols_or_certify_producers() {
        use crate::types::{CallableKind, CaptureKind, FunctionCapture};
        let inferred = Type::function(vec![], Type::I64);
        let actual = Type::function_with_metadata(
            vec![],
            Type::I64,
            FunctionSafety::Safe,
            CallableKind::FnOnce,
            vec![FunctionCapture::new(
                CaptureKind::Move,
                Type::Array(Box::new(Type::I64), 1),
            )],
        );
        assert_eq!(
            value_representation(
                &Type::Tuple(vec![inferred.clone()]),
                &Type::Tuple(vec![actual.clone()])
            ),
            Some(Type::Tuple(vec![actual.clone()]))
        );
        let mut bounded = inferred.clone();
        let Type::Function { callable_kind, .. } = &mut bounded else {
            unreachable!()
        };
        *callable_kind = CallableKind::FnMut;
        assert!(value_representation(&bounded, &actual).is_none());
        let mut required = inferred.clone();
        let Type::Function {
            result_borrowing, ..
        } = &mut required
        else {
            unreachable!()
        };
        *result_borrowing = owned_result_requirement(&[]).unwrap();
        assert!(value_representation(&required, &actual).is_none());
        assert!(!input_shape_matches(&bounded, &actual));
        assert!(
            !input_shape_matches(&inferred, &actual),
            "Fn still rejects an FnOnce producer"
        );
        let mut unsafe_actual = actual.clone();
        let Type::Function { safety, .. } = &mut unsafe_actual else {
            unreachable!()
        };
        *safety = FunctionSafety::Unsafe;
        assert!(value_representation(&inferred, &unsafe_actual).is_none());
        assert!(
            value_representation(
                &Type::Pointer(Box::new(inferred)),
                &Type::Pointer(Box::new(actual))
            )
            .is_none(),
            "mutable/raw storage is invariant"
        );
    }

    #[test]
    fn nominal_representations_keep_distinct_fields_without_changing_semantic_arguments() {
        use crate::ids::{CrateId, DefId, LocalDefId};
        use crate::types::{CallableKind, CaptureKind, FunctionCapture};
        let id = DefId::new(CrateId(0), LocalDefId(42));
        let declared = Type::Struct {
            id,
            args: vec![Type::Bool],
        };
        let captured = |environment| {
            Type::function_with_metadata(
                vec![],
                Type::I64,
                FunctionSafety::Safe,
                CallableKind::Fn,
                vec![FunctionCapture::new(CaptureKind::Move, environment)],
            )
        };
        let scalar = Type::StructRepresentation {
            id,
            args: vec![Type::Bool],
            fields: vec![captured(Type::I64)],
        };
        let tuple = Type::StructRepresentation {
            id,
            args: vec![Type::Bool],
            fields: vec![captured(Type::Tuple(vec![Type::I64, Type::I64]))],
        };
        assert_eq!(
            value_representation(&declared, &scalar),
            Some(scalar.clone())
        );
        assert_eq!(value_representation(&declared, &tuple), Some(tuple.clone()));
        assert!(value_representation(&scalar, &tuple).is_none());
        assert!(value_representation(&scalar, &declared).is_none());
        for mutable in [false, true] {
            let reference = |inner| Type::Reference {
                mutable,
                inner: Box::new(inner),
            };
            assert_eq!(
                value_representation(&reference(declared.clone()), &reference(scalar.clone())),
                Some(reference(scalar.clone()))
            );
            assert!(
                value_representation(&reference(scalar.clone()), &reference(tuple.clone()))
                    .is_none()
            );
        }
        assert!(value_representation(
            &Type::Reference {
                mutable: false,
                inner: Box::new(declared.clone())
            },
            &Type::Reference {
                mutable: true,
                inner: Box::new(scalar.clone())
            }
        )
        .is_none());
        assert!(value_representation(
            &Type::Pointer(Box::new(declared.clone())),
            &Type::Pointer(Box::new(scalar.clone()))
        )
        .is_none());
        assert!(value_representation(
            &Type::Struct {
                id,
                args: vec![Type::I64]
            },
            &scalar
        )
        .is_none());
        let mut context = crate::type_context::TypeContext::new();
        let declared_id = context.intern_type(&declared);
        let scalar_id = context.intern_type(&scalar);
        let tuple_id = context.intern_type(&tuple);
        assert_ne!(declared_id, scalar_id);
        assert_ne!(scalar_id, tuple_id);
        assert_eq!(context.type_for(scalar_id), scalar);
        assert_eq!(context.id_for_type(&tuple), Some(tuple_id));
        assert!(context.type_id_tree_is_valid(tuple_id));
    }

    #[test]
    fn resolved_environment_kinds_preserve_abstract_protocols_and_borrowing_authority() {
        use crate::types::{CallableKind, CaptureKind, FunctionCapture};
        for kind in [CallableKind::Fn, CallableKind::FnMut, CallableKind::FnOnce] {
            let mut callable = Type::function(vec![Type::I64], Type::I64);
            let Type::Function {
                callable_kind,
                result_borrowing,
                safety,
                ..
            } = &mut callable
            else {
                unreachable!()
            };
            *callable_kind = kind;
            *safety = FunctionSafety::Unsafe;
            *result_borrowing =
                CallableResultBorrowing::Sources(Box::new(vec![S::EnvironmentStorage]));
            assert_eq!(
                resolve_environment_kinds(callable.clone()),
                callable,
                "abstract protocols are requirements, not capture evidence"
            );
            for (capture_kind, capture_ty, expected) in [
                (CaptureKind::Move, Type::I64, CallableKind::Fn),
                (
                    CaptureKind::Move,
                    Type::Array(Box::new(Type::I64), 1),
                    CallableKind::FnOnce,
                ),
                (CaptureKind::SharedBorrow, Type::I64, CallableKind::Fn),
                (CaptureKind::MutableBorrow, Type::I64, CallableKind::FnMut),
            ] {
                let mut actual = callable.clone();
                let Type::Function { captures, .. } = &mut actual else {
                    unreachable!()
                };
                *captures = vec![FunctionCapture::new(capture_kind, capture_ty)].into();
                let resolved = resolve_environment_kinds(Type::Tuple(vec![actual.clone()]));
                let Type::Tuple(resolved) = resolved else {
                    unreachable!()
                };
                let Type::Function {
                    callable_kind,
                    safety,
                    result_borrowing,
                    ..
                } = &resolved[0]
                else {
                    unreachable!()
                };
                assert_eq!(*callable_kind, expected);
                assert_eq!(*safety, FunctionSafety::Unsafe);
                assert_eq!(
                    *result_borrowing,
                    CallableResultBorrowing::Sources(Box::new(vec![S::EnvironmentStorage])),
                    "environment resolution cannot manufacture a producer certificate"
                );
                assert_eq!(resolve_environment_kinds(resolved[0].clone()), resolved[0]);
            }
        }
    }

    #[test]
    fn owned_input_requirement_preserves_dependencies_without_storage_authority() {
        let params = vec![
            Type::I64,
            Type::Reference {
                inner: Box::new(Type::I64),
                mutable: true,
            },
            Type::Pointer(Box::new(Type::I64)),
        ];
        let requirement = owned_result_requirement(&params).unwrap();
        let CallableResultBorrowing::Sources(sources) = &requirement else {
            panic!("conditional sources")
        };
        for index in 0..3 {
            assert!(sources.contains(&S::ArgumentValue(index)));
        }
        assert!(!sources.contains(&S::ArgumentContents(0)));
        assert!(sources.contains(&S::ArgumentContents(1)));
        assert!(sources.contains(&S::ArgumentContents(2)));
        for source in [
            S::EnvironmentValue,
            S::EnvironmentContents,
            S::Static,
            S::Unknown,
        ] {
            assert!(sources.contains(&source));
        }
        assert!(!sources.contains(&S::EnvironmentStorage));
        assert!(!sources.contains(&S::EnvironmentBorrowedContents));
        assert!(
            !requirement.accepts(&CallableResultBorrowing::Sources(Box::new(vec![
                S::EnvironmentStorage
            ])))
        );
        assert!(owned_result_requirement(&[])
            .unwrap()
            .sources_are_valid(0, |_| false));
        assert!(requirement.sources_are_valid(params.len(), |index| index != 0));
    }

    #[test]
    fn canonical_pack_preserves_one_tuple_or_multiple_native_parameters() {
        let two = Type::function(vec![Type::I64, Type::Bool], Type::I64);
        let one = Type::function(vec![Type::Tuple(vec![Type::I64, Type::Bool])], Type::I64);
        assert!(packed_input_shape_matches(&two, &one));
        assert!(packed_input_shape_matches(&one, &two));
        assert!(!input_shape_matches(&two, &one));
        assert!(!packed_input_shape_matches(
            &two,
            &Type::function(vec![Type::Tuple(vec![Type::Bool, Type::I64])], Type::I64)
        ));
        assert!(!packed_input_shape_matches(
            &two,
            &Type::function(vec![Type::Tuple(vec![Type::I64, Type::Bool])], Type::Bool)
        ));
    }

    #[test]
    fn physical_pack_adaptation_cannot_broaden_narrow_or_unsafe_requirements() {
        let reference = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };
        let params = vec![reference.clone(), reference.clone()];
        let mut canonical = Type::function(params.clone(), reference.clone());
        let Type::Function {
            result_borrowing, ..
        } = &mut canonical
        else {
            unreachable!()
        };
        *result_borrowing = owned_result_requirement(&params).unwrap();
        let actual = Type::function(vec![Type::Tuple(params)], reference);
        let physical = physical_owned_input_requirement(&canonical, &actual).unwrap();
        let Type::Function {
            params,
            result_borrowing,
            ..
        } = &physical
        else {
            unreachable!()
        };
        assert_eq!(params.len(), 1);
        assert_eq!(result_borrowing, &owned_result_requirement(params).unwrap());
        let mut unsafe_actual = actual.clone();
        let Type::Function { safety, .. } = &mut unsafe_actual else {
            unreachable!()
        };
        *safety = FunctionSafety::Unsafe;
        assert!(physical_owned_input_requirement(&canonical, &unsafe_actual).is_none());
        let Type::Function {
            result_borrowing, ..
        } = &mut canonical
        else {
            unreachable!()
        };
        *result_borrowing = CallableResultBorrowing::Sources(Box::new(vec![S::ArgumentValue(0)]));
        assert!(physical_owned_input_requirement(&canonical, &actual).is_none());
        let mut once_actual = actual;
        let Type::Function { callable_kind, .. } = &mut once_actual else {
            unreachable!()
        };
        *callable_kind = crate::types::CallableKind::FnOnce;
        assert!(physical_owned_input_requirement(&canonical, &once_actual).is_none());
    }
}
