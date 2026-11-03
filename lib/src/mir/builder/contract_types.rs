use crate::ids::{TypeId, VariantId};
use crate::type_context::TypeContext;
use crate::types::Type;

use crate::mir::{
    Constant, MirCallable, MirFunction, MirFunctionId, Operand, Place, Projection, Terminator,
};

use super::*;

impl<'a> MirBuilder<'a> {
    pub(super) fn populate_backend_contract_intrinsic_callables(
        contract: &mut crate::mir::MirBackendContract,
        functions: &std::collections::BTreeMap<MirFunctionId, MirFunction>,
        type_context: &TypeContext,
    ) {
        for function in functions.values() {
            for block in &function.basic_blocks {
                let Some(Terminator::Call {
                    func,
                    args,
                    destination,
                    ..
                }) = &block.terminator
                else {
                    continue;
                };
                let Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    crate::mir::MirCallableKey::Intrinsic(intrinsic),
                ))) = func
                else {
                    continue;
                };
                let key = crate::mir::MirCallableKey::Intrinsic(intrinsic.clone());
                if contract.callables.contains_key(&key) {
                    continue;
                }
                let Some(signature) = Self::intrinsic_signature_from_call(
                    function,
                    contract,
                    type_context,
                    args,
                    destination,
                ) else {
                    continue;
                };
                contract.callables.insert(
                    key.clone(),
                    crate::mir::MirCallableDecl {
                        key: key.clone(),
                        source_def_id: None,
                        kind: crate::mir::MirCallableKind::Intrinsic(intrinsic.clone()),
                        llvm_symbol: intrinsic.as_str().to_string(),
                        linkage: crate::mir::MirLinkage::Internal,
                        signature,
                    },
                );
            }
        }
    }

    fn intrinsic_signature_from_call(
        function: &MirFunction,
        contract: &crate::mir::MirBackendContract,
        type_context: &TypeContext,
        args: &[Operand],
        destination: &Place,
    ) -> Option<crate::mir::MirCallableSignature> {
        let params = args
            .iter()
            .map(|arg| Self::operand_type_id_for_contract(function, contract, type_context, arg))
            .collect::<Option<Vec<_>>>()?;
        let ret = Self::place_type_id_for_contract(function, contract, type_context, destination)
            .or_else(|| type_context.id_for_type(&Type::Unit))?;

        Some(crate::mir::MirCallableSignature::from_type_ids(
            &params,
            ret,
            crate::mir::MirPassMode::Direct,
        ))
    }

    fn operand_type_id_for_contract(
        function: &MirFunction,
        contract: &crate::mir::MirBackendContract,
        type_context: &TypeContext,
        operand: &Operand,
    ) -> Option<TypeId> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                Self::place_type_id_for_contract(function, contract, type_context, place)
            }
            Operand::Constant(Constant::Int(_)) => type_context.id_for_type(&Type::I64),
            Operand::Constant(Constant::Float(_)) => type_context.id_for_type(&Type::F64),
            Operand::Constant(Constant::Bool(_)) => type_context.id_for_type(&Type::Bool),
            Operand::Constant(Constant::Char(_)) => type_context.id_for_type(&Type::Char),
            Operand::Constant(Constant::String(_)) => type_context.id_for_type(&Type::Str),
            Operand::Constant(Constant::Unit) => type_context.id_for_type(&Type::Unit),
            Operand::Constant(Constant::TypeId(id)) => Some(*id),
            Operand::Constant(Constant::Callable(_)) => None,
        }
    }

    fn place_type_id_for_contract(
        function: &MirFunction,
        contract: &crate::mir::MirBackendContract,
        type_context: &TypeContext,
        place: &Place,
    ) -> Option<TypeId> {
        let local = function.local_decls.get(place.local.0)?;
        let raw_local_ty = type_context.type_for(local.ty);
        let mut current_ty = contract.normalize_type(type_context, &raw_local_ty);
        if place.projection.is_empty() {
            return if current_ty == raw_local_ty {
                Some(local.ty)
            } else {
                type_context.id_for_type(&current_ty)
            };
        }

        let mut downcast_variant = None;
        for projection in &place.projection {
            current_ty = contract.normalize_type(type_context, &current_ty);
            current_ty = match projection {
                Projection::Deref => match &current_ty {
                    Type::Pointer(inner) | Type::Reference { inner, .. } => inner.as_ref().clone(),
                    _ => return None,
                },
                Projection::Field { index, .. } => Self::project_contract_field_type(
                    contract,
                    type_context,
                    &current_ty,
                    *index,
                    downcast_variant.take(),
                )?,
                Projection::Index(_) => match &current_ty {
                    Type::Array(element, _) | Type::Slice(element) | Type::Pointer(element) => {
                        element.as_ref().clone()
                    }
                    Type::Str => Type::U8,
                    _ => return None,
                },
                Projection::Downcast(variant_id) => {
                    downcast_variant = Some(*variant_id);
                    current_ty.clone()
                }
            };
            current_ty = contract.normalize_type(type_context, &current_ty);
        }

        type_context.id_for_type(&current_ty)
    }

    fn project_contract_field_type(
        contract: &crate::mir::MirBackendContract,
        type_context: &TypeContext,
        ty: &Type,
        index: usize,
        downcast_variant: Option<VariantId>,
    ) -> Option<Type> {
        let ty = contract.normalize_type(type_context, ty);
        let projected = match &ty {
            Type::Tuple(fields) => fields.get(index).cloned(),
            Type::Struct { id, args } => {
                let crate::mir::MirNominalLayout::Struct { fields, .. } =
                    contract.nominal_layouts.get(id)?
                else {
                    return None;
                };
                let field_ty = fields.get(index).map(|(_, ty)| *ty)?;
                let fields = fields
                    .iter()
                    .map(|(_, field_ty)| type_context.type_for(*field_ty))
                    .collect::<Vec<_>>();
                let subst = Self::generic_substitution_for_fields(
                    &fields,
                    &Type::Struct {
                        id: *id,
                        args: args.clone(),
                    },
                );
                Some(type_context.type_for(field_ty).substitute_generics(&subst))
            }
            Type::Enum { id, args } => {
                let variant_id = downcast_variant?;
                let crate::mir::MirNominalLayout::Enum { variants, .. } =
                    contract.nominal_layouts.get(id)?
                else {
                    return None;
                };
                let variant = variants.get(variant_id.0 as usize)?;
                let variant_fields = match &variant.fields {
                    crate::mir::MirVariantLayoutFields::Unit => return None,
                    crate::mir::MirVariantLayoutFields::Positional(fields) => fields
                        .iter()
                        .map(|field_ty| (None, *field_ty))
                        .collect::<Vec<_>>(),
                    crate::mir::MirVariantLayoutFields::Named(fields) => fields
                        .iter()
                        .map(|(name, field_ty)| (Some(name.as_str()), *field_ty))
                        .collect::<Vec<_>>(),
                };
                let field_ty = variant_fields.get(index).map(|(_, ty)| *ty)?;
                let fields = variant_fields
                    .iter()
                    .map(|(_, field_ty)| type_context.type_for(*field_ty))
                    .collect::<Vec<_>>();
                let subst = Self::generic_substitution_for_fields(
                    &fields,
                    &Type::Enum {
                        id: *id,
                        args: args.clone(),
                    },
                );
                Some(type_context.type_for(field_ty).substitute_generics(&subst))
            }
            _ => None,
        }?;
        Some(contract.normalize_type(type_context, &projected))
    }
}
