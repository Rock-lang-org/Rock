use std::collections::{HashMap, HashSet};

use crate::ids::{AssocTypeId, DefId, TypeId};
use crate::mir::{MirBackendContract, MirNominalLayout, MirProjectionKey};
use crate::type_context::{Ty, TypeView};
use crate::types::{GenericParamId, Type};

pub(super) fn contract_struct_fields(
    backend_contract: &MirBackendContract,
    id: DefId,
) -> Option<&[(String, TypeId)]> {
    match backend_contract.nominal_layouts.get(&id) {
        Some(MirNominalLayout::Struct { fields, .. }) => Some(fields.as_slice()),
        _ => None,
    }
}

pub(super) fn contract_enum_variants(
    backend_contract: &MirBackendContract,
    id: DefId,
) -> Option<&[crate::mir::MirEnumVariantLayout]> {
    match backend_contract.nominal_layouts.get(&id) {
        Some(MirNominalLayout::Enum { variants, .. }) => Some(variants.as_slice()),
        _ => None,
    }
}

pub(super) fn type_id_is_pointer(view: TypeView<'_>, ty: crate::ids::TypeId) -> bool {
    matches!(view.ty(ty), Ty::Pointer(_))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OriginTracking {
    All,
    Pointers,
    References,
    MutableReferences,
}

pub(super) fn type_id_contains_mutable_reference(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: TypeId,
) -> bool {
    type_id_tracks_origin_inner(
        view,
        backend_contract,
        ty,
        OriginTracking::MutableReferences,
        &mut HashSet::new(),
    )
}

pub(super) fn type_id_contains_pointer(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: TypeId,
) -> bool {
    type_id_tracks_origin_inner(
        view,
        backend_contract,
        ty,
        OriginTracking::Pointers,
        &mut HashSet::new(),
    )
}

pub(super) fn type_id_contains_reference(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: crate::ids::TypeId,
) -> bool {
    let mut seen = HashSet::new();
    type_id_tracks_origin_inner(
        view,
        backend_contract,
        ty,
        OriginTracking::References,
        &mut seen,
    )
}

pub(super) fn type_id_tracks_origin(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: crate::ids::TypeId,
) -> bool {
    let mut seen = HashSet::new();
    type_id_tracks_origin_inner(view, backend_contract, ty, OriginTracking::All, &mut seen)
}

fn type_id_tracks_origin_inner(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: crate::ids::TypeId,
    tracking: OriginTracking,
    seen: &mut HashSet<crate::ids::TypeId>,
) -> bool {
    if !seen.insert(ty) {
        return false;
    }

    match view.ty(ty) {
        Ty::Reference { mutable, .. } => {
            tracking != OriginTracking::Pointers
                && (*mutable || tracking != OriginTracking::MutableReferences)
        }
        Ty::Pointer(_) if matches!(tracking, OriginTracking::All | OriginTracking::Pointers) => {
            true
        }
        Ty::Tuple(elems) => elems
            .iter()
            .any(|elem| type_id_tracks_origin_inner(view, backend_contract, *elem, tracking, seen)),
        // Bare slice/string MIR values still carry borrowed storage metadata.
        Ty::Slice(_) | Ty::Str => {
            matches!(tracking, OriginTracking::All | OriginTracking::References)
        }
        Ty::Array { inner, .. } => {
            type_id_tracks_origin_inner(view, backend_contract, *inner, tracking, seen)
        }
        Ty::Function {
            params,
            ret,
            captures,
            ..
        } => {
            params.iter().any(|param| {
                type_id_tracks_origin_inner(view, backend_contract, *param, tracking, seen)
            }) || type_id_tracks_origin_inner(view, backend_contract, *ret, tracking, seen)
                || captures.iter().any(|capture| {
                    type_id_tracks_origin_inner(view, backend_contract, capture.ty, tracking, seen)
                })
        }
        Ty::Struct { id, args } => {
            args.iter().any(|arg| {
                type_id_tracks_origin_inner(view, backend_contract, *arg, tracking, seen)
            }) || contract_struct_fields(backend_contract, *id).is_some_and(|layout| {
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
                layout.iter().any(|(_, field_ty)| {
                    type_id_tracks_origin_for_structural_type(
                        view,
                        backend_contract,
                        &view.type_for(*field_ty).substitute_generics(&subst),
                        tracking,
                        seen,
                    )
                })
            })
        }
        Ty::Enum { id, args } => {
            args.iter().any(|arg| {
                type_id_tracks_origin_inner(view, backend_contract, *arg, tracking, seen)
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
                        args: args.iter().map(|arg| view.type_for(*arg)).collect(),
                    },
                );
                variants.iter().any(|variant| match &variant.fields {
                    crate::mir::MirVariantLayoutFields::Unit => false,
                    crate::mir::MirVariantLayoutFields::Positional(fields) => {
                        fields.iter().any(|field_ty| {
                            type_id_tracks_origin_for_structural_type(
                                view,
                                backend_contract,
                                &view.type_for(*field_ty).substitute_generics(&subst),
                                tracking,
                                seen,
                            )
                        })
                    }
                    crate::mir::MirVariantLayoutFields::Named(fields) => {
                        fields.iter().any(|(_, field_ty)| {
                            type_id_tracks_origin_for_structural_type(
                                view,
                                backend_contract,
                                &view.type_for(*field_ty).substitute_generics(&subst),
                                tracking,
                                seen,
                            )
                        })
                    }
                })
            })
        }
        Ty::Projection {
            ty,
            trait_id,
            assoc_type,
            trait_args,
        } => {
            let output_tracks = projection_output_type_id(
                backend_contract,
                *ty,
                *trait_id,
                assoc_type.assoc_type_id,
                trait_args,
            )
            .is_some_and(|output| {
                let mut output_seen = seen.clone();
                type_id_tracks_origin_inner(
                    view,
                    backend_contract,
                    output,
                    tracking,
                    &mut output_seen,
                )
            });

            output_tracks
                || type_id_tracks_origin_inner(view, backend_contract, *ty, tracking, seen)
                || trait_args.iter().any(|arg| {
                    type_id_tracks_origin_inner(view, backend_contract, *arg, tracking, seen)
                })
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
        | Ty::Never
        | Ty::Pointer(_)
        | Ty::ConstValue(_)
        | Ty::Error => false,
        Ty::Generic(_)
        | Ty::TypeVar(_)
        | Ty::Constructor { .. }
        | Ty::Apply { .. }
        | Ty::Lambda { .. }
        | Ty::BoundVar { .. } => true,
    }
}

fn type_id_tracks_origin_for_structural_type(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: &Type,
    tracking: OriginTracking,
    seen: &mut HashSet<crate::ids::TypeId>,
) -> bool {
    if let Some(id) = view.id_for_type(ty) {
        return type_id_tracks_origin_inner(view, backend_contract, id, tracking, seen);
    }

    match ty {
        Type::Reference { mutable, .. } => {
            tracking != OriginTracking::Pointers
                && (*mutable || tracking != OriginTracking::MutableReferences)
        }
        Type::Pointer(_) if matches!(tracking, OriginTracking::All | OriginTracking::Pointers) => {
            true
        }
        Type::Tuple(elems) => elems.iter().any(|elem| {
            type_id_tracks_origin_for_structural_type(view, backend_contract, elem, tracking, seen)
        }),
        Type::Slice(_) | Type::Str => {
            matches!(tracking, OriginTracking::All | OriginTracking::References)
        }
        Type::Array(inner, _) => {
            type_id_tracks_origin_for_structural_type(view, backend_contract, inner, tracking, seen)
        }
        Type::Function {
            params,
            ret,
            captures,
            ..
        } => {
            params.iter().any(|param| {
                type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    param,
                    tracking,
                    seen,
                )
            }) || type_id_tracks_origin_for_structural_type(
                view,
                backend_contract,
                ret,
                tracking,
                seen,
            ) || captures.iter().any(|capture| {
                type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    &capture.ty,
                    tracking,
                    seen,
                )
            })
        }
        Type::Struct { id, args } => {
            args.iter().any(|arg| {
                type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    arg,
                    tracking,
                    seen,
                )
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
                    type_id_tracks_origin_for_structural_type(
                        view,
                        backend_contract,
                        &field_ty.substitute_generics(&subst),
                        tracking,
                        seen,
                    )
                })
            })
        }
        Type::Enum { id, args } => {
            args.iter().any(|arg| {
                type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    arg,
                    tracking,
                    seen,
                )
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
                    type_id_tracks_origin_for_structural_type(
                        view,
                        backend_contract,
                        &field_ty.substitute_generics(&subst),
                        tracking,
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
            let output_tracks = projection_output_for_structural_type(
                view,
                backend_contract,
                ty,
                *trait_id,
                assoc_type.assoc_type_id,
                trait_args,
            )
            .is_some_and(|output| {
                let mut output_seen = seen.clone();
                type_id_tracks_origin_inner(
                    view,
                    backend_contract,
                    output,
                    tracking,
                    &mut output_seen,
                )
            });

            output_tracks
                || type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    ty,
                    tracking,
                    seen,
                )
                || trait_args.iter().any(|arg| {
                    type_id_tracks_origin_for_structural_type(
                        view,
                        backend_contract,
                        arg,
                        tracking,
                        seen,
                    )
                })
        }
        Type::Apply { constructor, args } => {
            type_id_tracks_origin_for_structural_type(
                view,
                backend_contract,
                constructor,
                tracking,
                seen,
            ) || args.iter().any(|arg| {
                type_id_tracks_origin_for_structural_type(
                    view,
                    backend_contract,
                    arg,
                    tracking,
                    seen,
                )
            })
        }
        Type::Lambda { body, .. } => {
            type_id_tracks_origin_for_structural_type(view, backend_contract, body, tracking, seen)
        }
        Type::Generic(_) | Type::TypeVar(_) | Type::Constructor { .. } | Type::BoundVar { .. } => {
            true
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
        | Type::Char
        | Type::Unit
        | Type::Never
        | Type::Pointer(_)
        | Type::ConstValue(_)
        | Type::Error => false,
    }
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

pub(super) fn generic_substitution_for_fields(
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

pub(super) fn type_id_is_mut_reference(view: TypeView<'_>, ty: crate::ids::TypeId) -> bool {
    matches!(view.ty(ty), Ty::Reference { mutable: true, .. })
}
