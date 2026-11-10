use std::collections::HashMap;

use crate::hir::{HirEnum, HirImpl, HirStruct, HirVariantFields};
use crate::ids::DefId;
use crate::types::{CaptureKind, GenericParamId, Type};

use super::impl_lookup::explicit_impl_exists_for;
use super::BuiltinTraitIds;

pub(super) fn auto_trait_satisfied(
    ty: &Type,
    trait_id: DefId,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
    visiting: &mut std::collections::HashSet<Type>,
) -> bool {
    let is_send = builtin_traits.send == Some(trait_id);
    let is_sync = builtin_traits.sync == Some(trait_id);
    if !is_send && !is_sync {
        return false;
    }
    if !visiting.insert(ty.clone()) {
        return true;
    }

    let nested = |nested: &Type, visiting: &mut std::collections::HashSet<Type>| {
        explicit_impl_exists_for(nested, trait_id, &[], impls, structs, enums, builtin_traits)
            || auto_trait_satisfied(
                nested,
                trait_id,
                impls,
                structs,
                enums,
                builtin_traits,
                visiting,
            )
    };

    let satisfied = match ty {
        Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::F32
        | Type::F64
        | Type::Bool
        | Type::Char
        | Type::Unit
        | Type::Never
        | Type::Str => true,
        Type::Array(inner, _) | Type::Slice(inner) => nested(inner, visiting),
        Type::Tuple(elements) => elements.iter().all(|element| nested(element, visiting)),
        Type::Reference { .. } if is_send => false,
        Type::Reference { inner, .. } => {
            let required = builtin_traits.sync;
            required.is_some_and(|required| {
                explicit_impl_exists_for(
                    inner,
                    required,
                    &[],
                    impls,
                    structs,
                    enums,
                    builtin_traits,
                ) || auto_trait_satisfied(
                    inner,
                    required,
                    impls,
                    structs,
                    enums,
                    builtin_traits,
                    visiting,
                )
            })
        }
        Type::Pointer(_) => false,
        Type::Function { captures, .. } if is_send => captures
            .iter()
            .all(|capture| capture.kind == CaptureKind::Move && nested(&capture.ty, visiting)),
        Type::Function { captures, .. } => captures.iter().all(|capture| {
            let required = match capture.kind {
                CaptureKind::SharedBorrow => builtin_traits.sync,
                CaptureKind::MutableBorrow | CaptureKind::Move => builtin_traits.sync,
            };
            required.is_some_and(|required| {
                explicit_impl_exists_for(
                    &capture.ty,
                    required,
                    &[],
                    impls,
                    structs,
                    enums,
                    builtin_traits,
                ) || auto_trait_satisfied(
                    &capture.ty,
                    required,
                    impls,
                    structs,
                    enums,
                    builtin_traits,
                    visiting,
                )
            })
        }),
        Type::Struct { id, args } => structs.get(id).is_some_and(|structure| {
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
            structure
                .fields
                .iter()
                .all(|field| nested(&field.ty.substitute_generics(&subst), visiting))
        }),
        Type::Enum { id, args } => enums.get(id).is_some_and(|enumeration| {
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
            enumeration
                .variants
                .iter()
                .all(|variant| match &variant.fields {
                    HirVariantFields::Unit => true,
                    HirVariantFields::Positional(fields) => fields
                        .iter()
                        .all(|field| nested(&field.substitute_generics(&subst), visiting)),
                    HirVariantFields::Named(fields) => fields
                        .iter()
                        .all(|field| nested(&field.ty.substitute_generics(&subst), visiting)),
                })
        }),
        Type::Generic(_)
        | Type::Projection { .. }
        | Type::TypeVar(_)
        | Type::Constructor { .. }
        | Type::Apply { .. }
        | Type::Lambda { .. }
        | Type::BoundVar { .. }
        | Type::ConstValue(_)
        | Type::Error => false,
    };
    visiting.remove(ty);
    satisfied
}

pub(super) fn type_is_sized(ty: &Type) -> bool {
    match ty {
        Type::Slice(_) | Type::Str => false,
        Type::Array(inner, _) => type_is_sized(inner),
        Type::Tuple(elems) => elems.iter().all(type_is_sized),
        Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::F32
        | Type::F64
        | Type::Bool
        | Type::Char
        | Type::Unit
        | Type::Never
        | Type::Struct { .. }
        | Type::Enum { .. }
        | Type::Reference { .. }
        | Type::Pointer(_)
        | Type::Function { .. } => true,
        Type::Generic(_)
        | Type::Projection { .. }
        | Type::TypeVar(_)
        | Type::Constructor { .. }
        | Type::Apply { .. }
        | Type::Lambda { .. }
        | Type::BoundVar { .. }
        | Type::ConstValue(_)
        | Type::Error => false,
    }
}
