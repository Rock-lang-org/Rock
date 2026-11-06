use crate::ids::TypeId;
use crate::mir::{MirBackendContract, MirCallableKey, MirPassMode};
use crate::types::Type;

use super::{CodeGen, CodegenError};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn validate_mir_backend_contract(
        &self,
        mir: &crate::mir::MirProgram,
    ) -> Result<(), CodegenError> {
        let mut messages = crate::mir::validate_backend_contract_against_mir(
            &mir.backend_contract,
            mir.functions.values(),
        )
        .into_iter()
        .map(|error| format!("{:?}", error))
        .collect::<Vec<_>>();

        let observed = crate::mir::backend_contract::runtime_requirements_for_functions(
            mir.functions.values(),
        );
        let missing = observed
            .difference(&mir.backend_contract.runtime_requirements)
            .collect::<Vec<_>>();
        let stale = mir
            .backend_contract
            .runtime_requirements
            .difference(&observed)
            .collect::<Vec<_>>();
        if !missing.is_empty() || !stale.is_empty() {
            messages.push(format!(
                "runtime requirement mismatch: missing {missing:?}, stale {stale:?}"
            ));
        }

        self.validate_mir_backend_contract_type_ids(&mir.backend_contract, &mut messages);
        self.validate_mir_drop_glue_contract(mir, &mut messages)?;

        if messages.is_empty() {
            return Ok(());
        }

        Err(CodegenError::backend_contract(format!(
            "Invalid MIR backend contract: {}",
            messages.join(", ")
        )))
    }

    fn validate_mir_backend_contract_type_ids(
        &self,
        contract: &MirBackendContract,
        messages: &mut Vec<String>,
    ) {
        for (key, callable) in &contract.callables {
            for (param_index, param) in callable.signature.params.iter().enumerate() {
                self.validate_mir_backend_contract_type_id(
                    param.semantic_ty,
                    &format!("parameter {param_index} semantic type"),
                    key,
                    messages,
                );
            }

            self.validate_mir_backend_contract_type_id(
                callable.signature.ret.semantic_ty,
                "return semantic type",
                key,
                messages,
            );
            self.validate_mir_backend_contract_type_id(
                callable.signature.ret.abi_ty,
                "return ABI type",
                key,
                messages,
            );
        }
    }

    fn validate_mir_drop_glue_contract(
        &self,
        mir: &crate::mir::MirProgram,
        messages: &mut Vec<String>,
    ) -> Result<(), CodegenError> {
        for (place_ty, key) in &mir.backend_contract.drop_glue {
            self.validate_mir_backend_contract_type_id(*place_ty, "dropped type", key, messages);
            let Some(callable) = mir.backend_contract.callable(key) else {
                messages.push(format!(
                    "Drop glue for type {:?} references missing callable key {:?}",
                    place_ty, key
                ));
                continue;
            };
            if callable.signature.params.len() != 1 {
                messages.push(format!(
                    "Drop glue for type {:?} callable {:?} has {} parameters; expected 1 receiver parameter",
                    place_ty,
                    key,
                    callable.signature.params.len(),
                ));
            }
            let Some(receiver) = callable.signature.params.first() else {
                continue;
            };
            self.validate_mir_backend_contract_type_id(
                receiver.semantic_ty,
                "receiver parameter type",
                key,
                messages,
            );
            if receiver.semantic_ty != *place_ty {
                messages.push(format!(
                    "Drop glue for type {:?} callable {:?} has receiver parameter type {:?}",
                    place_ty, key, receiver.semantic_ty
                ));
            }
            if receiver.pass_mode != MirPassMode::Direct {
                messages.push(format!(
                    "Drop glue for type {:?} callable {:?} has receiver pass mode {:?}; expected {:?}",
                    place_ty,
                    key,
                    receiver.pass_mode,
                    MirPassMode::Direct,
                ));
            }
            let ret_semantic_valid = self.validate_mir_backend_contract_type_id(
                callable.signature.ret.semantic_ty,
                "return semantic type",
                key,
                messages,
            );
            let ret_abi_valid = self.validate_mir_backend_contract_type_id(
                callable.signature.ret.abi_ty,
                "return ABI type",
                key,
                messages,
            );
            if ret_semantic_valid
                && ret_abi_valid
                && (!matches!(
                    self.structural_type_for(callable.signature.ret.semantic_ty),
                    Type::Unit
                ) || !matches!(
                    self.structural_type_for(callable.signature.ret.abi_ty),
                    Type::Unit
                ))
            {
                messages.push(format!(
                    "Drop glue for type {:?} callable {:?} returns {:?}/{:?}; expected ()",
                    place_ty,
                    key,
                    callable.signature.ret.semantic_ty,
                    callable.signature.ret.abi_ty,
                ));
            }
        }

        for (_, function) in mir.functions() {
            for block in &function.basic_blocks {
                let Some(
                    crate::mir::Terminator::Drop { place, .. }
                    | crate::mir::Terminator::DropWithOrigin { place, .. },
                ) = &block.terminator
                else {
                    continue;
                };
                if function
                    .local_decls
                    .get(place.local.0)
                    .is_some_and(|local| {
                        place.projection.is_empty()
                            && local.source == crate::mir::LocalSource::ReturnPlace
                    })
                {
                    continue;
                }

                let place_ty = self.mir_place_type_id_for_contract_validation(function, place)?;
                let Some(key) = mir.backend_contract.drop_glue.get(&place_ty) else {
                    if !Self::mir_place_is_projected_return_place(function, place)
                        && self.mir_drop_without_glue_is_noop(place_ty)
                    {
                        continue;
                    }
                    messages.push(format!(
                        "Missing drop glue contract for MIR drop of type {:?} in MIR function '{}'",
                        place_ty, function.name
                    ));
                    continue;
                };
                if !mir.backend_contract.callables.contains_key(key) {
                    messages.push(format!(
                        "Drop glue for type {:?} references missing callable key {:?}",
                        place_ty, key
                    ));
                    continue;
                }
                let callable = mir
                    .backend_contract
                    .callable(key)
                    .expect("drop glue key existence checked above");
                let Some(receiver) = callable.signature.params.first() else {
                    messages.push(format!(
                        "Drop glue for type {:?} callable {:?} is missing receiver parameter",
                        place_ty, key
                    ));
                    continue;
                };
                if receiver.semantic_ty != place_ty {
                    messages.push(format!(
                        "Drop glue for type {:?} callable {:?} has receiver parameter type {:?}",
                        place_ty, key, receiver.semantic_ty
                    ));
                }
                if receiver.pass_mode != MirPassMode::Direct {
                    messages.push(format!(
                        "Drop glue for type {:?} callable {:?} has receiver pass mode {:?}; expected {:?}",
                        place_ty,
                        key,
                        receiver.pass_mode,
                        MirPassMode::Direct,
                    ));
                }
            }
        }

        Ok(())
    }

    fn validate_mir_backend_contract_type_id(
        &self,
        ty: TypeId,
        label: &str,
        key: &MirCallableKey,
        messages: &mut Vec<String>,
    ) -> bool {
        if self.type_context().contains_type_id(ty) {
            return true;
        }

        messages.push(format!(
            "MIR callable {:?} has unknown {} {:?}",
            key, label, ty,
        ));
        false
    }

    pub(super) fn mir_place_is_projected_return_place(
        function: &crate::mir::MirFunction,
        place: &crate::mir::Place,
    ) -> bool {
        !place.projection.is_empty()
            && function
                .local_decls
                .get(place.local.0)
                .is_some_and(|local| local.source == crate::mir::LocalSource::ReturnPlace)
    }

    fn mir_place_type_id_for_contract_validation(
        &self,
        function: &crate::mir::MirFunction,
        place: &crate::mir::Place,
    ) -> Result<TypeId, CodegenError> {
        let local = function.local_decls.get(place.local.0).ok_or_else(|| {
            CodegenError::from(format!(
                "MIR local {} is not available while validating backend contract",
                place.local.0
            ))
        })?;
        if place.projection.is_empty() {
            return Ok(local.ty);
        }

        let mut current_ty = self.normalize_projection_type(&self.structural_type_for(local.ty));
        let mut unwrapped_downcast_payload = false;
        for projection in &place.projection {
            current_ty = self.normalize_projection_type(&current_ty);
            current_ty = match projection {
                crate::mir::Projection::Deref => match &current_ty {
                    Type::Pointer(inner) | Type::Reference { inner, .. } => {
                        self.normalize_projection_type(inner)
                    }
                    _ => {
                        return Err(CodegenError::from(format!(
                            "MIR deref projection expected pointer or reference, got {}",
                            self.display_type_for_diagnostic(&current_ty)
                        )))
                    }
                },
                crate::mir::Projection::Field { index, .. } => {
                    if unwrapped_downcast_payload {
                        unwrapped_downcast_payload = false;
                        if *index == 0 {
                            continue;
                        }
                    }
                    let field_ty = self
                        .mir_projection_field_type(&current_ty, *index)
                        .map_err(|error| {
                            CodegenError::from(format!(
                                "{} in MIR function '{}' local {} projection {:?}",
                                error, function.name, place.local.0, place.projection
                            ))
                        })?;
                    self.normalize_projection_type(&field_ty)
                }
                crate::mir::Projection::Index(_) => {
                    unwrapped_downcast_payload = false;
                    match &current_ty {
                        Type::Array(element, _) | Type::Slice(element) | Type::Pointer(element) => {
                            self.normalize_projection_type(element)
                        }
                        _ => {
                            return Err(CodegenError::from(format!(
                                "MIR index projection expected array, slice, or pointer, got {}",
                                self.display_type_for_diagnostic(&current_ty)
                            )))
                        }
                    }
                }
                crate::mir::Projection::Downcast(variant_id) => {
                    let Type::Enum { id, args } = &current_ty else {
                        return Err(CodegenError::from(format!(
                            "MIR downcast projection expected enum, got {}",
                            self.display_type_for_diagnostic(&current_ty)
                        )));
                    };
                    let payload_ty = self
                        .enum_variant_payload_type_by_id(*id, args, variant_id.0 as usize)
                        .ok_or_else(|| {
                            CodegenError::from(format!(
                                "Unknown MIR enum DefId {:?} variant {}",
                                id, variant_id.0
                            ))
                        })?;
                    unwrapped_downcast_payload =
                        self.enum_variant_payload_is_unwrapped(*id, variant_id.0 as usize)?;
                    self.normalize_projection_type(&payload_ty)
                }
            };
        }

        let current_ty = self.normalize_projection_type(&current_ty);
        self.type_context().id_for_type(&current_ty).ok_or_else(|| {
            CodegenError::from(format!(
                "MIR place type {} was not interned",
                self.display_type_for_diagnostic(&current_ty)
            ))
        })
    }

    fn mir_projection_field_type(&self, ty: &Type, index: usize) -> Result<Type, CodegenError> {
        match ty {
            Type::Tuple(fields) => fields.get(index).cloned().ok_or_else(|| {
                CodegenError::from(format!("MIR tuple field index {} is out of bounds", index))
            }),
            Type::Struct { id, args } => {
                let fields = self.nominal_layouts.struct_fields(id).ok_or_else(|| {
                    CodegenError::from(format!("Unknown MIR struct DefId {:?}", id))
                })?;
                let (_, field_ty) = fields.get(index).ok_or_else(|| {
                    CodegenError::from(format!(
                        "MIR struct field index {} is out of bounds for {:?}",
                        index, id
                    ))
                })?;
                let subst = self.struct_substitution_by_id(*id, args);
                Ok(self
                    .structural_type_for(*field_ty)
                    .substitute_generics(&subst))
            }
            other => Err(CodegenError::from(format!(
                "MIR field projection expected aggregate, got {}",
                self.display_type_for_diagnostic(other)
            ))),
        }
    }
}
