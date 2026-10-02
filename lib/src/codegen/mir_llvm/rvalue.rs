use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{
    AggregateKind, Constant, MirClosure, MirFunction, MirFunctionId, Operand, Rvalue,
};
use crate::types::Type;

use super::MirFunctionContext;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn compile_mir_rvalue_with_expected(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        rvalue: &Rvalue,
        expected_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        self.compile_mir_rvalue_inner(function, context, rvalue, Some(expected_ty))
    }

    fn compile_mir_rvalue_inner(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        rvalue: &Rvalue,
        expected_ty: Option<&Type>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match rvalue {
            Rvalue::Object(operand, conversion) => {
                self.compile_object_conversion(function, context, operand, conversion)
            }
            Rvalue::Use(operand) => self.compile_mir_operand(function, context, operand),
            Rvalue::BinaryOp(op, lhs, rhs) => {
                let lhs_value = self.compile_mir_operand(function, context, lhs)?;
                let rhs_value = self.compile_mir_operand(function, context, rhs)?;
                let op_ty = self.mir_binary_op_ty(context, lhs, rhs)?;
                self.compile_binop(*op, lhs_value, rhs_value, &op_ty)?
                    .ok_or_else(|| CodegenError::from("MIR binary op produced no value"))
            }
            Rvalue::UnaryOp(op, operand) => {
                let value = self.compile_mir_operand(function, context, operand)?;
                let ty = self.mir_operand_ty(context, operand)?;
                self.compile_unaryop(*op, value, &ty)?
                    .ok_or_else(|| CodegenError::from("MIR unary op produced no value"))
            }
            Rvalue::Cast(operand, target_ty) => {
                let value = self.compile_mir_operand(function, context, operand)?;
                let from_ty = self.mir_operand_ty(context, operand)?;
                let target_ty = self.structural_type_for(*target_ty);
                self.compile_mir_cast_value(value, &from_ty, &target_ty)
            }
            Rvalue::Ref(_, place) => {
                let expected_ty = expected_ty.ok_or_else(|| {
                    CodegenError::from("MIR reference codegen requires destination type")
                })?;
                self.compile_mir_ref_rvalue(context, place, expected_ty)
            }
            Rvalue::Discriminant(place) => {
                let (_, ty) = self.compile_mir_place_local(context, place)?;
                if !matches!(ty, Type::Enum { .. }) {
                    return Err(CodegenError::layout(format!(
                        "MIR discriminant codegen expected enum place, got {}",
                        self.display_type_for_diagnostic(&ty)
                    )));
                }

                let value = self
                    .compile_mir_place_value(context, place)?
                    .into_struct_value();
                self.builder
                    .build_extract_value(value, 0, "discriminant")
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to extract MIR discriminant: {}", e))
                    })
            }
            Rvalue::Aggregate(kind, operands) => {
                let expected_ty = expected_ty.ok_or_else(|| {
                    CodegenError::from("MIR aggregate codegen requires destination type")
                })?;
                self.compile_mir_aggregate(function, context, kind, operands, expected_ty)
            }
            Rvalue::Closure(closure) => {
                let expected_ty = expected_ty.ok_or_else(|| {
                    CodegenError::from("MIR closure codegen requires destination type")
                })?;
                self.compile_mir_closure_value(context, closure, expected_ty)
            }
        }
    }

    fn compile_mir_ref_rvalue(
        &mut self,
        context: &MirFunctionContext<'ctx>,
        place: &crate::mir::Place,
        expected_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if let Type::Reference { inner, mutable } = expected_ty {
            if matches!(inner.as_ref(), Type::Object(_)) {
                let mut handle = place.clone();
                if !matches!(handle.projection.pop(), Some(crate::mir::Projection::Deref)) {
                    return Err(CodegenError::backend_contract(
                        "object reborrow must retain existing metadata",
                    ));
                }
                let (_, source) = self.compile_mir_place_local(context, &handle)?;
                if !match &source {
                    Type::Reference {
                        inner: source,
                        mutable: source_mut,
                    } => source == inner && (!*mutable || *source_mut),
                    Type::Pointer(source) => source == inner,
                    _ => false,
                } {
                    return Err(CodegenError::backend_contract(
                        "object reborrow changes signature or gains mutable permission",
                    ));
                }
                return self.compile_mir_place_value(context, &handle);
            }
        }
        let (address, source_ty) = self.compile_mir_place_local(context, place)?;

        if self.is_fat_pointer_type(expected_ty) {
            return match &source_ty {
                Type::Array(element_ty, len) => {
                    let zero = self.context.i64_type().const_int(0, false);
                    let array_ty = self.llvm_type(&Type::Array(element_ty.clone(), *len));
                    let data_ptr = unsafe {
                        self.builder
                            .build_gep(array_ty, address, &[zero, zero], "array_ref_slice_ptr")
                            .map_err(|e| {
                                CodegenError::from(format!(
                                    "Failed to build MIR array ref slice ptr: {}",
                                    e
                                ))
                            })?
                    };
                    self.make_fat_slice_value(
                        data_ptr,
                        self.context.i64_type().const_int(*len as u64, false),
                    )
                }
                Type::Slice(_) | Type::Str | Type::Reference { .. } | Type::Pointer(_) => {
                    let value = self
                        .builder
                        .build_load(self.llvm_type(&source_ty), address, "mir_ref_slice_load")
                        .map_err(|e| {
                            CodegenError::from(format!(
                                "Failed to load MIR slice-compatible ref source: {}",
                                e
                            ))
                        })?;
                    let (data_ptr, len) = self.slice_parts_from_value(value, &source_ty)?;
                    self.make_fat_slice_value(data_ptr, len)
                }
                _ => Err(CodegenError::layout(format!(
                    "MIR fat reference expected slice-compatible source, got {}",
                    self.display_type_for_diagnostic(&source_ty)
                ))),
            };
        }

        Ok(address.into())
    }

    fn compile_mir_closure_value(
        &mut self,
        context: &MirFunctionContext<'ctx>,
        closure: &MirClosure,
        expected_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let closure_function_id = MirFunctionId::Closure(Box::new(closure.id.clone()));
        self.validate_mir_closure_value(context, closure, &closure_function_id, expected_ty)?;
        let symbol = self
            .mir_function_symbols
            .get(&closure_function_id)
            .cloned()
            .ok_or_else(|| {
                CodegenError::backend_contract(
                    "MIR closure function was not declared before codegen",
                )
            })?;
        let closure_fn = self.functions.get(&symbol).copied().ok_or_else(|| {
            CodegenError::layout(format!(
                "LLVM function '{}' not found for MIR closure",
                symbol
            ))
        })?;

        let mut capture_values = Vec::with_capacity(closure.captures.len());
        for capture in &closure.captures {
            let place = capture.place();
            let (capture_ptr, capture_ty) = self.compile_mir_place_local(context, &place)?;
            match capture.kind {
                crate::mir::MirClosureCaptureKind::ByValue => {
                    let value = self
                        .builder
                        .build_load(
                            self.llvm_type(&capture_ty),
                            capture_ptr,
                            &format!("mir_closure_capture_load_{}", capture_values.len()),
                        )
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to load MIR closure capture: {}", e))
                        })?;
                    capture_values.push((value, capture_ty));
                }
                crate::mir::MirClosureCaptureKind::ByRef
                | crate::mir::MirClosureCaptureKind::ByMutRef => {
                    capture_values.push((
                        capture_ptr.into(),
                        Type::Reference {
                            mutable: capture.kind == crate::mir::MirClosureCaptureKind::ByMutRef,
                            inner: Box::new(capture_ty),
                        },
                    ));
                }
            }
        }

        let env_ptr = if capture_values.is_empty() {
            self.context.ptr_type(AddressSpace::default()).const_null()
        } else {
            let env_ty = self.context.struct_type(
                &capture_values
                    .iter()
                    .map(|(_, ty)| self.llvm_type(ty))
                    .collect::<Vec<_>>(),
                false,
            );
            let env_size = env_ty
                .size_of()
                .ok_or_else(|| CodegenError::from("Cannot compute MIR closure environment size"))?;
            let malloc_fn = self
                .functions
                .get("malloc")
                .copied()
                .ok_or_else(|| CodegenError::from("malloc not declared"))?;
            let raw_env_ptr = self
                .builder
                .build_call(malloc_fn, &[env_size.into()], "mir_closure_env_alloc")
                .map_err(|e| {
                    CodegenError::from(format!("Failed to call malloc for MIR closure env: {}", e))
                })?
                .try_as_basic_value()
                .left()
                .ok_or_else(|| CodegenError::from("malloc returned void"))?
                .into_pointer_value();
            let env_ptr = self
                .builder
                .build_pointer_cast(
                    raw_env_ptr,
                    self.context.ptr_type(AddressSpace::default()),
                    "mir_closure_env_ptr",
                )
                .map_err(|e| {
                    CodegenError::from(format!("Failed to cast MIR closure env ptr: {}", e))
                })?;

            for (index, (value, ty)) in capture_values.iter().enumerate() {
                let field_ptr = self
                    .builder
                    .build_struct_gep(
                        env_ty,
                        env_ptr,
                        index as u32,
                        &format!("mir_closure_capture_store_{}", index),
                    )
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to access MIR closure env: {}", e))
                    })?;
                let coerced = self.coerce_value(*value, self.llvm_type(ty))?;
                self.builder.build_store(field_ptr, coerced).map_err(|e| {
                    CodegenError::from(format!("Failed to store MIR closure env: {}", e))
                })?;
            }

            raw_env_ptr
        };

        let code_slot = self
            .builder
            .build_alloca(
                self.context.ptr_type(AddressSpace::default()),
                "mir_closure_code_slot",
            )
            .map_err(|e| {
                CodegenError::from(format!("Failed to allocate MIR closure code slot: {}", e))
            })?;
        self.builder
            .build_store(code_slot, closure_fn.as_global_value().as_pointer_value())
            .map_err(|e| {
                CodegenError::from(format!("Failed to store MIR closure code ptr: {}", e))
            })?;
        let code_ptr = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                code_slot,
                "callable_code",
            )
            .map_err(|e| CodegenError::from(format!("Failed to load MIR closure code ptr: {}", e)))?
            .into_pointer_value();

        self.build_callable_value(code_ptr, env_ptr)
    }

    fn validate_mir_closure_value(
        &self,
        context: &MirFunctionContext<'ctx>,
        closure: &MirClosure,
        closure_function_id: &MirFunctionId,
        expected_ty: &Type,
    ) -> Result<(), CodegenError> {
        let Type::Function {
            params: expected_params,
            ret: expected_ret,
            ..
        } = expected_ty
        else {
            return Err(CodegenError::layout(format!(
                "MIR closure destination must be a function type, got {}",
                self.display_type_for_diagnostic(expected_ty)
            )));
        };
        let metadata = self
            .mir_closure_metadata
            .get(closure_function_id)
            .ok_or_else(|| {
                CodegenError::backend_contract(
                    "MIR closure metadata was not recorded before codegen",
                )
            })?;

        let metadata_params = metadata
            .params
            .iter()
            .map(|ty| self.structural_type_for(*ty))
            .collect::<Vec<_>>();
        let metadata_ret = self.structural_type_for(metadata.ret);
        if metadata_params != *expected_params || metadata_ret != **expected_ret {
            let expected_ret_display = self.display_type_for_diagnostic(expected_ret);
            let metadata_ret_display = self.display_type_for_diagnostic(&metadata_ret);
            return Err(CodegenError::layout(format!(
                "MIR closure signature mismatch for '{}': expected ({}) -> {}, target ({}) -> {}",
                closure.display_name,
                expected_params
                    .iter()
                    .map(|ty| self.display_type_for_diagnostic(ty))
                    .collect::<Vec<_>>()
                    .join(", "),
                expected_ret_display,
                metadata_params
                    .iter()
                    .map(|ty| self.display_type_for_diagnostic(ty))
                    .collect::<Vec<_>>()
                    .join(", "),
                metadata_ret_display
            )));
        }

        if closure.captures.len() != metadata.captures.len() {
            return Err(CodegenError::layout(format!(
                "MIR closure capture layout mismatch for '{}': value has {} captures, target has {}",
                closure.display_name,
                closure.captures.len(),
                metadata.captures.len()
            )));
        }

        for (index, (capture, target_ty)) in closure
            .captures
            .iter()
            .zip(metadata.captures.iter())
            .enumerate()
        {
            let place = capture.place();
            let (_, value_ty) = self.compile_mir_place_local(context, &place)?;
            let value_ty = match capture.kind {
                crate::mir::MirClosureCaptureKind::ByValue => value_ty,
                crate::mir::MirClosureCaptureKind::ByRef
                | crate::mir::MirClosureCaptureKind::ByMutRef => Type::Reference {
                    mutable: capture.kind == crate::mir::MirClosureCaptureKind::ByMutRef,
                    inner: Box::new(value_ty),
                },
            };
            let target_ty = self.structural_type_for(*target_ty);
            if value_ty != target_ty {
                return Err(CodegenError::layout(format!(
                    "MIR closure capture layout mismatch for '{}': capture {} has type {}, target expects {}",
                    closure.display_name,
                    index,
                    self.display_type_for_diagnostic(&value_ty),
                    self.display_type_for_diagnostic(&target_ty)
                )));
            }
        }

        Ok(())
    }

    fn compile_mir_aggregate(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        kind: &AggregateKind,
        operands: &[Operand],
        expected_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match kind {
            AggregateKind::Tuple => {
                let Type::Tuple(fields) = expected_ty else {
                    return Err(CodegenError::layout(format!(
                        "MIR tuple aggregate expected tuple destination, got {}",
                        self.display_type_for_diagnostic(expected_ty)
                    )));
                };
                self.validate_mir_aggregate_operand_count(
                    "tuple aggregate",
                    fields.len(),
                    operands.len(),
                )?;
                let field_types = fields
                    .iter()
                    .map(|ty| self.llvm_type(ty))
                    .collect::<Vec<_>>();
                let struct_ty = self.context.struct_type(&field_types, false);
                let mut value = struct_ty.get_undef();

                for (index, operand) in operands.iter().enumerate() {
                    let operand_value = self.compile_mir_operand(function, context, operand)?;
                    let coerced = self.coerce_value(operand_value, field_types[index])?;
                    value = self
                        .builder
                        .build_insert_value(value, coerced, index as u32, "tuple_field")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to insert MIR tuple field: {}", e))
                        })?
                        .into_struct_value();
                }

                Ok(value.into())
            }
            AggregateKind::Array => {
                let Type::Array(element_ty, len) = expected_ty else {
                    return Err(CodegenError::layout(format!(
                        "MIR array aggregate expected array destination, got {}",
                        self.display_type_for_diagnostic(expected_ty)
                    )));
                };
                self.validate_mir_aggregate_operand_count("array aggregate", *len, operands.len())?;
                let element_llvm_ty = self.llvm_type(element_ty);
                let array_ty = element_llvm_ty.array_type(*len as u32);
                let mut value = array_ty.get_undef();

                for (index, operand) in operands.iter().enumerate() {
                    let operand_value = self.compile_mir_operand(function, context, operand)?;
                    let coerced = self.coerce_value(operand_value, element_llvm_ty)?;
                    value = self
                        .builder
                        .build_insert_value(value, coerced, index as u32, "array_elem")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to insert MIR array element: {}", e))
                        })?
                        .into_array_value();
                }

                Ok(value.into())
            }
            AggregateKind::Struct { id, display_name } => {
                let Type::Struct { id: ty_id, args } = expected_ty else {
                    return Err(CodegenError::layout(format!(
                        "MIR struct aggregate expected struct destination in {}, got {} for {}",
                        function.name,
                        self.display_type_for_diagnostic(expected_ty),
                        display_name
                    )));
                };
                let layout_name = self
                    .struct_names_by_id
                    .get(id)
                    .map(String::as_str)
                    .unwrap_or(display_name.as_str());
                if id != ty_id {
                    return Err(CodegenError::layout(
                        "MIR struct aggregate identity does not match its destination",
                    ));
                }
                let fields = self
                    .struct_layouts_by_id
                    .get(id)
                    .cloned()
                    .ok_or_else(|| CodegenError::layout("Unknown MIR struct layout"))?;
                self.validate_mir_aggregate_operand_count(
                    &format!("struct aggregate {}", layout_name),
                    fields.len(),
                    operands.len(),
                )?;
                let subst = self.struct_substitution_by_id(*id, args);
                let field_types: Vec<BasicTypeEnum<'ctx>> = fields
                    .iter()
                    .map(|(_, ty)| {
                        let ty = self.structural_type_for(*ty);
                        self.llvm_type(&ty.substitute_generics(&subst))
                    })
                    .collect();
                let struct_ty = self.context.struct_type(&field_types, false);
                let mut value = struct_ty.get_undef();

                for (index, operand) in operands.iter().enumerate() {
                    let operand_value = self.compile_mir_operand(function, context, operand)?;
                    let coerced = self.coerce_value(operand_value, field_types[index])?;
                    value = self
                        .builder
                        .build_insert_value(value, coerced, index as u32, "struct_field")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to insert MIR struct field: {}", e))
                        })?
                        .into_struct_value();
                }

                Ok(value.into())
            }
            AggregateKind::EnumVariant {
                enum_id,
                variant_id,
                enum_name,
                variant_name: _,
            } => {
                let Type::Enum { id: ty_id, args } = expected_ty else {
                    return Err(CodegenError::layout(format!(
                        "MIR enum aggregate expected enum destination, got {}",
                        self.display_type_for_diagnostic(expected_ty)
                    )));
                };
                let layout_name = self
                    .enum_names_by_id
                    .get(enum_id)
                    .map(String::as_str)
                    .unwrap_or(enum_name.as_str());
                if enum_id != ty_id {
                    return Err(CodegenError::layout(
                        "MIR enum aggregate identity does not match its destination",
                    ));
                }
                let variant_index = variant_id.0 as usize;
                let expected_payload_count =
                    self.mir_enum_payload_operand_count(*enum_id, variant_index, enum_name)?;
                self.validate_mir_aggregate_operand_count(
                    &format!("enum payload {}::{}", layout_name, variant_index),
                    expected_payload_count,
                    operands.len(),
                )?;
                let layout_types = self
                    .enum_layout_types_by_id(*enum_id, args)
                    .ok_or_else(|| CodegenError::layout("Unknown MIR enum layout"))?;
                let enum_ty = self.context.struct_type(&layout_types, false);
                let mut value = enum_ty.get_undef();
                value = self
                    .builder
                    .build_insert_value(
                        value,
                        self.context
                            .i32_type()
                            .const_int(variant_index as u64, false),
                        0,
                        "tag",
                    )
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to insert MIR enum tag: {}", e))
                    })?
                    .into_struct_value();

                let payload_ty = self
                    .enum_variant_payload_type_by_id(*enum_id, args, variant_index)
                    .ok_or_else(|| {
                        CodegenError::layout(format!(
                            "Unknown MIR enum variant index {}",
                            variant_index
                        ))
                    })?;
                let payload_llvm_ty = self.llvm_type(&payload_ty);
                // One positional field is the payload itself, even when its
                // type is a tuple; named fields retain their aggregate wrapper.
                let single_positional =
                    self.enum_variant_payload_is_unwrapped(*enum_id, variant_index)?;
                let payload = if single_positional {
                    let operand = self.compile_mir_operand(function, context, &operands[0])?;
                    self.coerce_value(operand, payload_llvm_ty)?
                } else {
                    self.compile_mir_aggregate_payload(
                        function,
                        context,
                        operands,
                        &payload_ty,
                        payload_llvm_ty,
                    )?
                };
                value = self
                    .builder
                    .build_insert_value(value, payload, variant_index as u32 + 1, "payload")
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to insert MIR enum payload: {}", e))
                    })?
                    .into_struct_value();

                Ok(value.into())
            }
        }
    }

    fn validate_mir_aggregate_operand_count(
        &self,
        context: &str,
        expected: usize,
        actual: usize,
    ) -> Result<(), CodegenError> {
        if expected == actual {
            return Ok(());
        }

        Err(CodegenError::layout(format!(
            "MIR {} operand count mismatch: expected {}, got {}",
            context, expected, actual
        )))
    }

    fn mir_enum_payload_operand_count(
        &self,
        enum_id: crate::ids::DefId,
        variant_index: usize,
        display_name: &str,
    ) -> Result<usize, CodegenError> {
        let variants = self.enum_layouts_by_id.get(&enum_id).ok_or_else(|| {
            CodegenError::layout(format!("Unknown MIR enum layout: {}", display_name))
        })?;
        let variant = variants.get(variant_index).ok_or_else(|| {
            CodegenError::from(format!(
                "MIR enum variant index {} is out of bounds for {}",
                variant_index, display_name
            ))
        })?;

        Ok(match &variant.fields {
            crate::codegen::CodegenEnumVariantFields::Unit => 0,
            crate::codegen::CodegenEnumVariantFields::Positional(fields) => fields.len(),
            crate::codegen::CodegenEnumVariantFields::Named(fields) => fields.len(),
        })
    }

    fn compile_mir_aggregate_payload(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        operands: &[Operand],
        payload_ty: &Type,
        payload_llvm_ty: BasicTypeEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if operands.is_empty() {
            return Ok(payload_llvm_ty.const_zero());
        }

        if let Type::Tuple(fields) = payload_ty {
            let field_types = fields
                .iter()
                .map(|ty| self.llvm_type(ty))
                .collect::<Vec<_>>();
            let struct_ty = self.context.struct_type(&field_types, false);
            let mut payload = struct_ty.get_undef();

            for (index, operand) in operands.iter().enumerate() {
                let operand_value = self.compile_mir_operand(function, context, operand)?;
                let coerced = self.coerce_value(operand_value, field_types[index])?;
                payload = self
                    .builder
                    .build_insert_value(payload, coerced, index as u32, "payload_field")
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to insert MIR payload field: {}", e))
                    })?
                    .into_struct_value();
            }

            return Ok(payload.into());
        }

        if operands.len() == 1 {
            let value = self.compile_mir_operand(function, context, &operands[0])?;
            return self.coerce_value(value, payload_llvm_ty);
        }

        Err(CodegenError::layout(format!(
            "MIR enum payload got {} operands for non-tuple payload {}",
            operands.len(),
            self.display_type_for_diagnostic(payload_ty)
        )))
    }

    pub(crate) fn mir_operand_ty(
        &self,
        context: &MirFunctionContext<'ctx>,
        operand: &Operand,
    ) -> Result<Type, CodegenError> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let (_, ty) = self.compile_mir_place_local(context, place)?;
                Ok(ty)
            }
            Operand::Constant(Constant::Int(_)) => Ok(Type::I64),
            Operand::Constant(Constant::Float(_)) => Ok(Type::F64),
            Operand::Constant(Constant::Bool(_)) => Ok(Type::Bool),
            Operand::Constant(Constant::Char(_)) => Ok(Type::Char),
            Operand::Constant(Constant::Unit) => Ok(Type::Unit),
            Operand::Constant(Constant::TypeId(id)) => Ok(self.type_context().type_for(*id)),
            Operand::Constant(constant) => Err(CodegenError::backend_contract(format!(
                "MIR constant type is not available for codegen: {:?}",
                constant
            ))),
        }
    }

    pub(crate) fn mir_operand_ty_id(
        &self,
        context: &MirFunctionContext<'ctx>,
        operand: &Operand,
    ) -> Result<crate::ids::TypeId, CodegenError> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let (_, ty) = self.compile_mir_place_local_id(context, place)?;
                Ok(ty)
            }
            Operand::Constant(Constant::Int(_)) => self
                .type_context()
                .id_for_type(&Type::I64)
                .ok_or_else(|| CodegenError::from("MIR TypeContext is missing I64")),
            Operand::Constant(Constant::Float(_)) => self
                .type_context()
                .id_for_type(&Type::F64)
                .ok_or_else(|| CodegenError::from("MIR TypeContext is missing F64")),
            Operand::Constant(Constant::Bool(_)) => self
                .type_context()
                .id_for_type(&Type::Bool)
                .ok_or_else(|| CodegenError::from("MIR TypeContext is missing Bool")),
            Operand::Constant(Constant::Char(_)) => self
                .type_context()
                .id_for_type(&Type::Char)
                .ok_or_else(|| CodegenError::from("MIR TypeContext is missing Char")),
            Operand::Constant(Constant::Unit) => self
                .type_context()
                .id_for_type(&Type::Unit)
                .ok_or_else(|| CodegenError::from("MIR TypeContext is missing ()")),
            Operand::Constant(Constant::TypeId(id)) => Ok(*id),
            Operand::Constant(constant) => Err(CodegenError::backend_contract(format!(
                "MIR constant type is not available for codegen: {:?}",
                constant
            ))),
        }
    }

    fn mir_binary_op_ty(
        &self,
        context: &MirFunctionContext<'ctx>,
        lhs: &Operand,
        rhs: &Operand,
    ) -> Result<Type, CodegenError> {
        let lhs_ty = self.mir_operand_ty(context, lhs)?;
        if matches!(lhs, Operand::Constant(Constant::Int(_))) {
            let rhs_ty = self.mir_operand_ty(context, rhs)?;
            if rhs_ty.is_integer() {
                return Ok(rhs_ty);
            }
        }
        Ok(lhs_ty)
    }

    fn validate_unicode_scalar(
        &self,
        value: inkwell::values::IntValue<'ctx>,
        from_ty: &Type,
    ) -> Result<(), CodegenError> {
        use inkwell::IntPredicate;

        let error = |e: inkwell::builder::BuilderError| CodegenError::from(e.to_string());
        // Check before narrowing so large integers cannot wrap into valid scalars.
        let wide = self.context.i64_type();
        let value = if value.get_type().get_bit_width() < 64 {
            if from_ty.is_signed_integer() {
                self.builder
                    .build_int_s_extend(value, wide, "char_value")
                    .map_err(error)?
            } else {
                self.builder
                    .build_int_z_extend(value, wide, "char_value")
                    .map_err(error)?
            }
        } else {
            value
        };
        let out_of_range = self
            .builder
            .build_int_compare(
                IntPredicate::UGT,
                value,
                wide.const_int(0x10ffff, false),
                "char_out_of_range",
            )
            .map_err(error)?;
        let above = self
            .builder
            .build_int_compare(
                IntPredicate::UGE,
                value,
                wide.const_int(0xd800, false),
                "char_above_surrogate_start",
            )
            .map_err(error)?;
        let below = self
            .builder
            .build_int_compare(
                IntPredicate::ULE,
                value,
                wide.const_int(0xdfff, false),
                "char_below_surrogate_end",
            )
            .map_err(error)?;
        let surrogate = self
            .builder
            .build_and(above, below, "char_surrogate")
            .map_err(error)?;
        let invalid = self
            .builder
            .build_or(out_of_range, surrogate, "char_invalid")
            .map_err(error)?;
        let function = self
            .builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .ok_or_else(|| CodegenError::from("No current function for Unicode scalar check"))?;
        let fail = self.context.append_basic_block(function, "char_invalid");
        let valid = self.context.append_basic_block(function, "char_valid");
        self.builder
            .build_conditional_branch(invalid, fail, valid)
            .map_err(error)?;
        self.builder.position_at_end(fail);
        let trap = inkwell::intrinsics::Intrinsic::find("llvm.trap")
            .and_then(|intrinsic| intrinsic.get_declaration(&self.module, &[]))
            .ok_or_else(|| CodegenError::from("LLVM trap intrinsic is unavailable"))?;
        self.builder.build_call(trap, &[], "").map_err(error)?;
        self.builder.build_unreachable().map_err(error)?;
        self.builder.position_at_end(valid);
        Ok(())
    }

    fn compile_mir_cast_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        from_ty: &Type,
        target_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let to_llvm = self.llvm_type(target_ty);
        let from_is_fat_pointer = self.is_fat_pointer_type(from_ty);
        let target_is_fat_pointer = self.is_fat_pointer_type(target_ty);

        if from_ty.is_integer() && target_ty.is_float() {
            if from_ty.is_signed_integer() {
                return Ok(self
                    .builder
                    .build_signed_int_to_float(
                        value.into_int_value(),
                        to_llvm.into_float_type(),
                        "sitofp",
                    )
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            return Ok(self
                .builder
                .build_unsigned_int_to_float(
                    value.into_int_value(),
                    to_llvm.into_float_type(),
                    "uitofp",
                )
                .map_err(|e| CodegenError::from(format!("{}", e)))?
                .into());
        }

        if from_ty.is_float() && target_ty.is_integer() {
            if target_ty.is_signed_integer() {
                return Ok(self
                    .builder
                    .build_float_to_signed_int(
                        value.into_float_value(),
                        to_llvm.into_int_type(),
                        "fptosi",
                    )
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            return Ok(self
                .builder
                .build_float_to_unsigned_int(
                    value.into_float_value(),
                    to_llvm.into_int_type(),
                    "fptoui",
                )
                .map_err(|e| CodegenError::from(format!("{}", e)))?
                .into());
        }

        if matches!(from_ty, Type::Char) && target_ty.is_integer() {
            let from_width = value.into_int_value().get_type().get_bit_width();
            let to_width = to_llvm.into_int_type().get_bit_width();
            if from_width < to_width {
                return Ok(self
                    .builder
                    .build_int_z_extend(value.into_int_value(), to_llvm.into_int_type(), "zext")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            if from_width > to_width {
                return Ok(self
                    .builder
                    .build_int_truncate(value.into_int_value(), to_llvm.into_int_type(), "trunc")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            return Ok(value);
        }

        if from_ty.is_integer() && matches!(target_ty, Type::Char) {
            self.validate_unicode_scalar(value.into_int_value(), from_ty)?;
            let from_width = value.into_int_value().get_type().get_bit_width();
            let to_width = to_llvm.into_int_type().get_bit_width();
            if from_width < to_width {
                return Ok(self
                    .builder
                    .build_int_z_extend(value.into_int_value(), to_llvm.into_int_type(), "zext")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            if from_width > to_width {
                return Ok(self
                    .builder
                    .build_int_truncate(value.into_int_value(), to_llvm.into_int_type(), "trunc")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            return Ok(value);
        }

        if from_ty.is_integer() && target_ty.is_integer() {
            let from_width = value.into_int_value().get_type().get_bit_width();
            let to_width = to_llvm.into_int_type().get_bit_width();
            if from_width < to_width {
                if from_ty.is_signed_integer() {
                    return Ok(self
                        .builder
                        .build_int_s_extend(value.into_int_value(), to_llvm.into_int_type(), "sext")
                        .map_err(|e| CodegenError::from(format!("{}", e)))?
                        .into());
                }
                return Ok(self
                    .builder
                    .build_int_z_extend(value.into_int_value(), to_llvm.into_int_type(), "zext")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            if from_width > to_width {
                return Ok(self
                    .builder
                    .build_int_truncate(value.into_int_value(), to_llvm.into_int_type(), "trunc")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            return Ok(value);
        }

        if from_ty.is_float() && target_ty.is_float() {
            let from_value = value.into_float_value();
            if matches!(from_ty, Type::F32) && matches!(target_ty, Type::F64) {
                return Ok(self
                    .builder
                    .build_float_ext(from_value, self.context.f64_type(), "fpext")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            if matches!(from_ty, Type::F64) && matches!(target_ty, Type::F32) {
                return Ok(self
                    .builder
                    .build_float_trunc(from_value, self.context.f32_type(), "fptrunc")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            return Ok(value);
        }

        if from_is_fat_pointer && target_is_fat_pointer {
            if !matches!(value, BasicValueEnum::StructValue(_)) {
                return Err(CodegenError::from(format!(
                    "MIR fat pointer cast expected slice-layout value from {} to {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            }
            return Ok(value);
        }

        if matches!(from_ty, Type::Pointer(_)) && matches!(target_ty, Type::Pointer(_)) {
            if from_is_fat_pointer || target_is_fat_pointer {
                return Err(CodegenError::from(format!(
                    "Cannot cast between thin and fat MIR pointers from {} to {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            }
            let BasicValueEnum::PointerValue(pointer) = value else {
                return Err(CodegenError::from(format!(
                    "MIR pointer-to-pointer cast expected pointer value from {} to {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            };
            return Ok(self
                .builder
                .build_pointer_cast(
                    pointer,
                    self.context.ptr_type(AddressSpace::default()),
                    "ptrcast",
                )
                .map_err(|e| CodegenError::from(format!("{}", e)))?
                .into());
        }

        if matches!(from_ty, Type::Pointer(_)) && target_ty.is_integer() {
            if from_is_fat_pointer {
                return Err(CodegenError::from(format!(
                    "Cannot cast fat MIR pointer {} to integer {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            }
            let BasicValueEnum::PointerValue(pointer) = value else {
                return Err(CodegenError::from(format!(
                    "MIR pointer-to-integer cast expected pointer value from {} to {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            };
            return Ok(self
                .builder
                .build_ptr_to_int(pointer, to_llvm.into_int_type(), "ptrtoint")
                .map_err(|e| CodegenError::from(format!("{}", e)))?
                .into());
        }

        if from_ty.is_integer() && matches!(target_ty, Type::Pointer(_)) {
            if target_is_fat_pointer {
                return Err(CodegenError::from(format!(
                    "Cannot cast integer to fat MIR pointer {} from {}",
                    self.display_type_for_diagnostic(target_ty),
                    self.display_type_for_diagnostic(from_ty)
                )));
            }
            let BasicValueEnum::IntValue(integer) = value else {
                return Err(CodegenError::from(format!(
                    "MIR integer-to-pointer cast expected integer value from {} to {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            };
            return Ok(self
                .builder
                .build_int_to_ptr(
                    integer,
                    self.context.ptr_type(AddressSpace::default()),
                    "inttoptr",
                )
                .map_err(|e| CodegenError::from(format!("{}", e)))?
                .into());
        }

        if matches!(from_ty, Type::Reference { .. }) && target_is_fat_pointer {
            let (data_ptr, len) = self.slice_parts_from_value(value, from_ty)?;
            return self.make_fat_slice_value(data_ptr, len);
        }

        if matches!(from_ty, Type::Reference { .. }) && matches!(target_ty, Type::Pointer(_)) {
            if from_is_fat_pointer || target_is_fat_pointer {
                return Err(CodegenError::from(format!(
                    "Cannot cast fat MIR pointer {} to thin pointer {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            }
            if !matches!(value, BasicValueEnum::PointerValue(_)) {
                return Err(CodegenError::from(format!(
                    "MIR reference-to-pointer cast expected pointer value from {} to {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            }
            return Ok(value);
        }

        if from_ty == target_ty {
            return Ok(value);
        }

        Err(CodegenError::from(format!(
            "Unsupported MIR cast from {} to {}",
            self.display_type_for_diagnostic(from_ty),
            self.display_type_for_diagnostic(target_ty)
        )))
    }
}
