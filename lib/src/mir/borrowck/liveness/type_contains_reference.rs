use std::collections::{HashMap, HashSet};

use crate::mir::{MirBackendContract, MirProjectionKey};

use crate::ids::{AssocTypeId, DefId, TypeId};
use crate::type_context::{Ty, TypeView};
use crate::types::{GenericParamId, Type};

pub(super) fn type_id_contains_reference_with_contract(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: TypeId,
) -> bool {
    let mut seen = HashSet::new();
    type_id_contains_reference_inner(view, backend_contract, ty, &mut seen)
}

fn type_id_contains_reference_inner(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: TypeId,
    seen: &mut HashSet<TypeId>,
) -> bool {
    if !seen.insert(ty) {
        return false;
    }

    match view.ty(ty) {
        Ty::Reference { .. } => true,
        Ty::Tuple(elems) => elems
            .iter()
            .any(|elem| type_id_contains_reference_inner(view, backend_contract, *elem, seen)),
        Ty::Array { inner, .. } | Ty::Slice(inner) => {
            type_id_contains_reference_inner(view, backend_contract, *inner, seen)
        }
        Ty::Function {
            params,
            ret,
            captures,
            ..
        } => {
            params
                .iter()
                .any(|param| type_id_contains_reference_inner(view, backend_contract, *param, seen))
                || type_id_contains_reference_inner(view, backend_contract, *ret, seen)
                || captures.iter().any(|capture| {
                    type_id_contains_reference_inner(view, backend_contract, capture.ty, seen)
                })
        }
        Ty::Struct { id, args } => {
            args.iter()
                .any(|arg| type_id_contains_reference_inner(view, backend_contract, *arg, seen))
                || contract_struct_fields(backend_contract, *id).is_some_and(|layout| {
                    let fields = layout
                        .iter()
                        .map(|(_, field_ty)| view.type_for(*field_ty))
                        .collect::<Vec<_>>();
                    let subst = generic_substitution_for_fields(
                        &fields,
                        &Type::Struct {
                            id: *id,
                            args: args.iter().map(|arg| view.type_for(*arg)).collect(),
                        },
                    );
                    fields.into_iter().any(|field_ty| {
                        type_contains_reference_for_structural_type(
                            view,
                            backend_contract,
                            &field_ty.substitute_generics(&subst),
                            seen,
                        )
                    })
                })
        }
        Ty::Enum { id, args } => {
            args.iter()
                .any(|arg| type_id_contains_reference_inner(view, backend_contract, *arg, seen))
                || contract_enum_variants(backend_contract, *id).is_some_and(|variants| {
                    let fields = variants
                        .iter()
                        .flat_map(|variant| match &variant.fields {
                            crate::mir::MirVariantLayoutFields::Unit => Vec::new(),
                            crate::mir::MirVariantLayoutFields::Positional(fields) => fields
                                .iter()
                                .map(|field_ty| view.type_for(*field_ty))
                                .collect(),
                            crate::mir::MirVariantLayoutFields::Named(fields) => fields
                                .iter()
                                .map(|(_, field_ty)| view.type_for(*field_ty))
                                .collect(),
                        })
                        .collect::<Vec<_>>();
                    let subst = generic_substitution_for_fields(
                        &fields,
                        &Type::Enum {
                            id: *id,
                            args: args.iter().map(|arg| view.type_for(*arg)).collect(),
                        },
                    );
                    fields.into_iter().any(|field_ty| {
                        type_contains_reference_for_structural_type(
                            view,
                            backend_contract,
                            &field_ty.substitute_generics(&subst),
                            seen,
                        )
                    })
                })
        }
        Ty::Projection {
            ty,
            trait_id,
            assoc_type,
            trait_args,
        } => {
            let output_contains = projection_output_type_id(
                backend_contract,
                *ty,
                *trait_id,
                assoc_type.assoc_type_id,
                trait_args,
            )
            .is_some_and(|output| {
                let mut output_seen = seen.clone();
                type_id_contains_reference_inner(view, backend_contract, output, &mut output_seen)
            });

            output_contains
                || type_id_contains_reference_inner(view, backend_contract, *ty, seen)
                || trait_args
                    .iter()
                    .any(|arg| type_id_contains_reference_inner(view, backend_contract, *arg, seen))
        }
        Ty::I8
        | Ty::I16
        | Ty::I32
        | Ty::I64
        | Ty::U8
        | Ty::U16
        | Ty::U32
        | Ty::U64
        | Ty::F32
        | Ty::F64
        | Ty::Bool
        | Ty::Char
        | Ty::Unit
        | Ty::Str
        | Ty::Pointer(_)
        | Ty::TypeVar(_)
        | Ty::Generic(_)
        | Ty::Constructor { .. }
        | Ty::Apply { .. }
        | Ty::Lambda { .. }
        | Ty::BoundVar { .. }
        | Ty::Error
        | Ty::Never => false,
    }
}

fn contract_struct_fields(
    backend_contract: &MirBackendContract,
    id: DefId,
) -> Option<&[(String, TypeId)]> {
    match backend_contract.nominal_layouts.get(&id) {
        Some(crate::mir::MirNominalLayout::Struct { fields, .. }) => Some(fields.as_slice()),
        _ => None,
    }
}

fn contract_enum_variants(
    backend_contract: &MirBackendContract,
    id: DefId,
) -> Option<&[crate::mir::MirEnumVariantLayout]> {
    match backend_contract.nominal_layouts.get(&id) {
        Some(crate::mir::MirNominalLayout::Enum { variants, .. }) => Some(variants.as_slice()),
        _ => None,
    }
}

fn type_contains_reference_for_structural_type(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: &Type,
    seen: &mut HashSet<TypeId>,
) -> bool {
    if let Some(id) = view.id_for_type(ty) {
        return type_id_contains_reference_inner(view, backend_contract, id, seen);
    }

    match ty {
        Type::Reference { .. } => true,
        Type::Tuple(elems) => elems.iter().any(|elem| {
            type_contains_reference_for_structural_type(view, backend_contract, elem, seen)
        }),
        Type::Array(inner, _) | Type::Slice(inner) => {
            type_contains_reference_for_structural_type(view, backend_contract, inner, seen)
        }
        Type::Function {
            params,
            ret,
            captures,
            ..
        } => {
            params.iter().any(|param| {
                type_contains_reference_for_structural_type(view, backend_contract, param, seen)
            }) || type_contains_reference_for_structural_type(view, backend_contract, ret, seen)
                || captures.iter().any(|capture| {
                    type_contains_reference_for_structural_type(
                        view,
                        backend_contract,
                        &capture.ty,
                        seen,
                    )
                })
        }
        Type::Struct { id, args } => {
            args.iter().any(|arg| {
                type_contains_reference_for_structural_type(view, backend_contract, arg, seen)
            }) || contract_struct_fields(backend_contract, *id).is_some_and(|layout| {
                let fields = layout
                    .iter()
                    .map(|(_, field_ty)| view.type_for(*field_ty))
                    .collect::<Vec<_>>();
                let subst = generic_substitution_for_fields(
                    &fields,
                    &Type::Struct {
                        id: *id,
                        args: args.clone(),
                    },
                );
                fields.into_iter().any(|field_ty| {
                    type_contains_reference_for_structural_type(
                        view,
                        backend_contract,
                        &field_ty.substitute_generics(&subst),
                        seen,
                    )
                })
            })
        }
        Type::Enum { id, args } => {
            args.iter().any(|arg| {
                type_contains_reference_for_structural_type(view, backend_contract, arg, seen)
            }) || contract_enum_variants(backend_contract, *id).is_some_and(|variants| {
                let fields = variants
                    .iter()
                    .flat_map(|variant| match &variant.fields {
                        crate::mir::MirVariantLayoutFields::Unit => Vec::new(),
                        crate::mir::MirVariantLayoutFields::Positional(fields) => fields
                            .iter()
                            .map(|field_ty| view.type_for(*field_ty))
                            .collect(),
                        crate::mir::MirVariantLayoutFields::Named(fields) => fields
                            .iter()
                            .map(|(_, field_ty)| view.type_for(*field_ty))
                            .collect(),
                    })
                    .collect::<Vec<_>>();
                let subst = generic_substitution_for_fields(
                    &fields,
                    &Type::Enum {
                        id: *id,
                        args: args.clone(),
                    },
                );
                fields.into_iter().any(|field_ty| {
                    type_contains_reference_for_structural_type(
                        view,
                        backend_contract,
                        &field_ty.substitute_generics(&subst),
                        seen,
                    )
                })
            })
        }
        Type::Projection {
            ty,
            trait_id,
            assoc_type,
            trait_args,
        } => {
            let output_contains = projection_output_for_structural_type(
                view,
                backend_contract,
                ty,
                *trait_id,
                assoc_type.assoc_type_id,
                trait_args,
            )
            .is_some_and(|output| {
                let mut output_seen = seen.clone();
                type_id_contains_reference_inner(view, backend_contract, output, &mut output_seen)
            });

            output_contains
                || type_contains_reference_for_structural_type(view, backend_contract, ty, seen)
                || trait_args.iter().any(|arg| {
                    type_contains_reference_for_structural_type(view, backend_contract, arg, seen)
                })
        }
        Type::Apply { constructor, args } => {
            type_contains_reference_for_structural_type(view, backend_contract, constructor, seen)
                || args.iter().any(|arg| {
                    type_contains_reference_for_structural_type(view, backend_contract, arg, seen)
                })
        }
        Type::Lambda { body, .. } => {
            type_contains_reference_for_structural_type(view, backend_contract, body, seen)
        }
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
        | Type::Str
        | Type::Char
        | Type::Unit
        | Type::Never
        | Type::Pointer(_)
        | Type::TypeVar(_)
        | Type::Generic(_)
        | Type::Constructor { .. }
        | Type::BoundVar { .. }
        | Type::Error => false,
    }
}

fn projection_output_for_structural_type(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    base: &Type,
    trait_id: DefId,
    assoc_type_id: AssocTypeId,
    trait_args: &[Type],
) -> Option<TypeId> {
    let base = view.id_for_type(base)?;
    let trait_args = trait_args
        .iter()
        .map(|arg| view.id_for_type(arg))
        .collect::<Option<Vec<_>>>()?;
    projection_output_type_id(backend_contract, base, trait_id, assoc_type_id, &trait_args)
}

fn generic_substitution_for_fields(
    fields: &[Type],
    source_ty: &Type,
) -> HashMap<GenericParamId, Type> {
    let args = match source_ty {
        Type::Enum { args, .. } | Type::Struct { args, .. } => args,
        _ => return HashMap::new(),
    };
    let mut generic_params = HashSet::new();
    for field_ty in fields {
        field_ty.collect_generic_params(&mut generic_params);
    }
    generic_params
        .into_iter()
        .filter_map(|param| {
            args.get(param.index as usize)
                .cloned()
                .map(|arg| (param, arg))
        })
        .collect()
}

fn projection_output_type_id(
    backend_contract: &MirBackendContract,
    base: TypeId,
    trait_id: DefId,
    assoc_type_id: AssocTypeId,
    trait_args: &[TypeId],
) -> Option<TypeId> {
    backend_contract
        .projection_outputs
        .get(&MirProjectionKey {
            base,
            trait_id,
            assoc_type_id,
            trait_args: trait_args.to_vec(),
        })
        .copied()
}
