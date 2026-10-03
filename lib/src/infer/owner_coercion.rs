//! Constructor-owned split/convert/rebuild, retaining ordinary call authority.
use crate::hir::*;
use crate::ids::{DefId, HirLocalId};
use crate::selection::{type_pattern_matches, SelectionService};
use crate::types::{FunctionSafety, TraitBound, Type};

pub(crate) fn is_owner_destination(ty: &Type) -> bool {
    // This only defers a candidate coercion; the constructor impl determines
    // which slot is the payload and must prove split/rebuild before admission.
    matches!(ty, Type::Struct { args, .. } if args.iter().any(|arg| {
        crate::type_services::visit::type_any(arg, |nested| matches!(nested, Type::Object(_)))
    }))
}

pub(crate) fn select_operation(
    service: &SelectionService<'_>,
    constructor: &Type,
    trait_id: DefId,
    state: &[Type],
    member: DefId,
    instantiated: &Type,
) -> Result<(HirFunction, HirStaticMethodTarget), String> {
    let mut selected = service
        .select_constructor_trait_member(constructor, trait_id, state, member)
        .map_err(|error| error.message())?;
    if !selected.function.is_unsafe || selected.function.self_receiver.is_some() {
        return Err("owner operation must be an unsafe static method".into());
    }
    let template = Type::function_with_safety(
        selected
            .function
            .params
            .iter()
            .map(|param| param.ty.clone())
            .collect(),
        selected.function.ret_type.clone(),
        FunctionSafety::Unsafe,
    );
    let mut substitution = selected.owner_substitution.clone();
    if !type_pattern_matches(&template, instantiated, &mut substitution) {
        return Err(
            "owner implementation signature does not match its canonical transport contract".into(),
        );
    }
    if selected.pending_impl_bounds.iter().any(|(subject, bound)| {
        let bound = crate::types::TraitBound {
            trait_id: bound.trait_id,
            type_args: bound
                .type_args
                .iter()
                .map(|ty| ty.substitute_generics(&substitution))
                .collect(),
        };
        !service.trait_bound_satisfied(&subject.substitute_generics(&substitution), &bound)
    }) {
        return Err("owner implementation has unproven obligations".into());
    }
    for parameter in &selected.function.generic_params {
        if selected
            .target
            .owner_substitution
            .iter()
            .any(|binding| binding.param == parameter.id)
        {
            continue;
        }
        if parameter.kind != crate::type_services::kind::Kind::Type
            || (!parameter.maybe_unsized
                && !selected
                    .function
                    .generic_bounds
                    .relaxed_sized
                    .contains(&parameter.id))
            || selected
                .function
                .generic_bounds
                .get(&parameter.id)
                .is_some_and(|bounds| !bounds.is_empty())
        {
            return Err("owner operation implementation must preserve the unrestricted ?Sized payload contract".into());
        }
        let ty = substitution
            .get(&parameter.id)
            .ok_or("unresolved owner method parameter")?
            .clone();
        selected.target.method_substitution.push(HirTypeBinding {
            param: parameter.id,
            ty,
        });
    }
    selected
        .target
        .method_substitution
        .sort_by_key(|binding| binding.param);
    Ok((
        selected.function,
        HirStaticMethodTarget {
            owner_ty: constructor.clone(),
            method: selected.target,
        },
    ))
}

pub(crate) fn lower(
    value: HirExpr,
    target: &Type,
    service: &SelectionService<'_>,
    language_items: &HirLanguageItems,
    local_id: HirLocalId,
) -> Result<HirExpr, String> {
    let Type::Struct { id, .. } = target else {
        return Err("owner destination must be a nominal owner".into());
    };
    let Type::Struct { id: source_id, .. } = &value.ty else {
        return Err("owner coercion requires an owned source".into());
    };
    if source_id != id {
        return Err("owner coercion must preserve the exact owner constructor".into());
    }
    let protocol = language_items
        .object_owner
        .as_ref()
        .ok_or("owner coercion requires an explicitly provided object_owner protocol")?;
    let (constructor, source, state) = service
        .infer_unary_constructor_application(&value.ty, protocol.trait_id)
        .map_err(|error| error.message())?
        .ok_or(
            "owner coercion requires one proven constructor implementation with independent State",
        )?;
    let (destination_constructor, destination, destination_state) = service
        .infer_unary_constructor_application(target, protocol.trait_id)
        .map_err(|error| error.message())?
        .ok_or("owner destination requires a proven constructor application")?;
    if constructor != destination_constructor || state != destination_state {
        return Err("owner coercion must preserve the exact owner constructor and State".into());
    }
    if !matches!(&destination, Type::Object(_)) {
        return Err("owner coercion must convert the constructor's payload slot".into());
    }
    let [state_ty] = state.as_slice() else {
        return Err("invalid object_owner State arity".into());
    };
    if crate::type_services::visit::type_any(state_ty, |ty| {
        matches!(
            ty,
            Type::TypeVar(_) | Type::Error | Type::ObjectSelf { .. } | Type::Witness(_)
        )
    }) || !language_items.sized.as_ref().map_or_else(
        || {
            crate::type_services::layout::TypeLayout::sizedness(state_ty, &|_| {
                crate::type_services::layout::Sizedness::Unknown
            }) == crate::type_services::layout::Sizedness::Sized
        },
        |sized| {
            service.trait_bound_satisfied(
                state_ty,
                &TraitBound {
                    trait_id: sized.trait_id,
                    type_args: Vec::new(),
                },
            )
        },
    ) {
        return Err(
            "owner State must be a uniquely determined sized type independent of its payload"
                .into(),
        );
    }
    let span = value.span.clone();
    let parts_ty = Type::Tuple(vec![
        Type::Pointer(Box::new(source.clone())),
        state_ty.clone(),
    ]);
    let pointer_ty = Type::Pointer(Box::new(destination));
    let operation =
        |member, params: Vec<Type>, ret: Type| -> Result<(HirExpr, HirCallTarget), String> {
            let instantiated = Type::function_with_safety(params, ret, FunctionSafety::Unsafe);
            let (function, authority) = select_operation(
                service,
                &constructor,
                protocol.trait_id,
                &state,
                member,
                &instantiated,
            )?;
            let callee = HirExpr {
                ty: instantiated,
                span: span.clone(),
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: function.name,
                    target: HirVarTarget::Function(function.id),
                }),
            };
            Ok((callee, HirCallTarget::StaticMethod(authority)))
        };
    let (into, into_target) = operation(
        protocol.into_parts_id,
        vec![value.ty.clone()],
        parts_ty.clone(),
    )?;
    let (from, from_target) = operation(
        protocol.from_parts_id,
        vec![pointer_ty.clone(), state_ty.clone()],
        target.clone(),
    )?;
    let parts = HirExpr {
        ty: parts_ty.clone(),
        span: span.clone(),
        kind: HirExprKind::ResolvedVar(HirVarRef {
            name: "<owner-parts>".into(),
            target: HirVarTarget::Local(local_id),
        }),
    };
    let pointer = HirExpr {
        ty: Type::Pointer(Box::new(source)),
        span: span.clone(),
        kind: HirExprKind::TupleIndex(Box::new(parts.clone()), 0),
    };
    let converted = HirExpr {
        ty: pointer_ty.clone(),
        span: span.clone(),
        kind: HirExprKind::ObjectCoercion(
            Box::new(pointer),
            HirObjectCoercion {
                target: pointer_ty,
                evidence: None,
            },
        ),
    };
    let state_value = HirExpr {
        ty: state_ty.clone(),
        span: span.clone(),
        kind: HirExprKind::TupleIndex(Box::new(parts), 1),
    };
    let body = HirBlock {
        ty: target.clone(),
        stmts: vec![
            HirStmt::Let {
                name: "<owner-parts>".into(),
                local_id,
                ty: parts_ty.clone(),
                mutable: false,
                value: HirExpr {
                    ty: parts_ty,
                    span: span.clone(),
                    kind: HirExprKind::Call(Box::new(into), vec![value], Some(into_target)),
                },
            },
            HirStmt::Expr(HirExpr {
                ty: target.clone(),
                span: span.clone(),
                kind: HirExprKind::Call(
                    Box::new(from),
                    vec![converted, state_value],
                    Some(from_target),
                ),
            }),
        ],
    };
    Ok(HirExpr {
        ty: target.clone(),
        span,
        kind: HirExprKind::UnsafeBlock(body),
    })
}
