use std::collections::BTreeSet;

use inkwell::module::Linkage;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::FunctionValue;
use inkwell::AddressSpace;

use crate::ids::TypeId;
use crate::mir::{
    MirBackendContract, MirCallableDecl, MirCallableKey, MirCallableKind, MirCallableSignature,
    MirLinkage, MirNominalLayout, MirParamAbi, MirPassMode, MirRuntimeHelper,
    MirVariantLayoutFields,
};
use crate::types::Type;

use super::{CodeGen, CodegenEnumVariantFields, CodegenEnumVariantLayout, CodegenError};

impl<'ctx> CodeGen<'ctx> {
    fn register_mir_nominal_layouts(&mut self, contract: &MirBackendContract) {
        for layout in contract.nominal_layouts.values() {
            match layout {
                MirNominalLayout::Struct {
                    id,
                    fields,
                    generic_params,
                } => {
                    self.nominal_layouts
                        .register_struct(*id, fields.clone(), generic_params.clone());
                }
                MirNominalLayout::Enum {
                    id,
                    variants,
                    generic_params,
                } => {
                    let variants = variants
                        .iter()
                        .map(|variant| CodegenEnumVariantLayout {
                            fields: Self::codegen_variant_fields_from_mir(&variant.fields),
                        })
                        .collect::<Vec<_>>();
                    self.nominal_layouts
                        .register_enum(*id, variants, generic_params.clone());
                }
            }
        }
    }

    fn codegen_variant_fields_from_mir(
        fields: &MirVariantLayoutFields,
    ) -> CodegenEnumVariantFields {
        match fields {
            MirVariantLayoutFields::Unit => CodegenEnumVariantFields::Unit,
            MirVariantLayoutFields::Positional(fields) => {
                CodegenEnumVariantFields::Positional(fields.clone())
            }
            MirVariantLayoutFields::Named(fields) => {
                CodegenEnumVariantFields::Named(fields.clone())
            }
        }
    }

    /// Declare the minimal C runtime support used directly by compiler-generated code.
    /// Ordinary external APIs are expected to come from user or dependency `extern`
    /// declarations, not compiler-owned registrations.
    pub(super) fn declare_runtime(&mut self, runtime_requirements: &BTreeSet<MirRuntimeHelper>) {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();

        if runtime_requirements.contains(&MirRuntimeHelper::BoundsCheck) {
            if self.functions.get("puts").is_none() {
                let fn_type = i32_ty.fn_type(&[ptr_ty.into()], false);
                let func = self.module.add_function("puts", fn_type, None);
                self.functions.insert("puts".to_string(), func);
            }

            if self.functions.get("exit").is_none() {
                let fn_type = self.context.void_type().fn_type(&[i32_ty.into()], false);
                let func = self.module.add_function("exit", fn_type, None);
                self.functions.insert("exit".to_string(), func);
            }
        }

        if runtime_requirements.contains(&MirRuntimeHelper::HeapAlloc)
            && self.functions.get("malloc").is_none()
        {
            let fn_type = ptr_ty.fn_type(&[i64_ty.into()], false);
            let func = self.module.add_function("malloc", fn_type, None);
            self.functions.insert("malloc".to_string(), func);
        }
    }

    pub(super) fn prepare_mir_program_declarations(
        &mut self,
        mir: &crate::mir::MirProgram,
    ) -> Result<(), CodegenError> {
        self.projection_outputs = mir
            .backend_contract
            .projection_outputs
            .clone()
            .into_iter()
            .collect();
        self.drop_glue_callables_by_type =
            mir.backend_contract.drop_glue.clone().into_iter().collect();

        self.register_mir_nominal_layouts(&mir.backend_contract);
        self.validate_mir_backend_contract(mir)?;
        self.declare_runtime(&mir.backend_contract.runtime_requirements);
        self.register_mir_backend_contract_callables(mir)
            .map_err(CodegenError::internalize)?;
        self.declare_mir_functions(mir)
            .map_err(CodegenError::internalize)?;
        self.materialize_program_entrypoint(mir)
            .map_err(CodegenError::internalize)?;

        Ok(())
    }

    fn declare_mir_functions(&mut self, mir: &crate::mir::MirProgram) -> Result<(), CodegenError> {
        self.declare_mir_closure_functions(mir)
    }

    fn register_mir_backend_contract_callables(
        &mut self,
        mir: &crate::mir::MirProgram,
    ) -> Result<(), CodegenError> {
        for declaration in mir.backend_contract.callables.values() {
            self.register_mir_contract_callable_metadata(declaration);
        }

        for declaration in mir.backend_contract.callables.values() {
            self.declare_mir_contract_callable(declaration)?;
        }

        Ok(())
    }

    fn register_mir_contract_callable_metadata(&mut self, declaration: &MirCallableDecl) {
        self.callables.register_declaration(declaration);
    }

    fn declare_mir_contract_callable(
        &mut self,
        declaration: &MirCallableDecl,
    ) -> Result<(), CodegenError> {
        match &declaration.kind {
            MirCallableKind::LocalBody { .. } | MirCallableKind::ObjectProvided => {
                let function = if matches!(declaration.key, MirCallableKey::Closure(_)) {
                    self.declare_mir_contract_closure_function(declaration)?
                } else {
                    self.declare_mir_contract_function(declaration)?
                };
                if matches!(declaration.linkage, MirLinkage::Internal) {
                    function.set_linkage(Linkage::Internal);
                }
            }
            MirCallableKind::Extern {
                link_name,
                variadic,
            } => {
                self.declare_mir_contract_extern(declaration, link_name, *variadic)?;
            }
            MirCallableKind::Intrinsic(_) | MirCallableKind::RuntimeHelper(_) => {}
        }

        Ok(())
    }

    fn declare_mir_contract_function(
        &mut self,
        declaration: &MirCallableDecl,
    ) -> Result<FunctionValue<'ctx>, CodegenError> {
        let llvm_symbol = if matches!(declaration.kind, MirCallableKind::LocalBody { .. }) {
            self.local_mir_body_symbol_name(declaration)
        } else {
            declaration.llvm_symbol.clone()
        };
        let function = self.declare_mir_function_with_signature(
            &declaration.llvm_symbol,
            &llvm_symbol,
            &declaration.signature,
        )?;

        if matches!(declaration.key, MirCallableKey::RuntimeHelper(MirRuntimeHelper::ArrayTailDrop(_))) {
            // Preserve sharing even when the caller is optimized for speed.
            function.add_attribute(AttributeLoc::Function, self.context.create_enum_attribute(
                Attribute::get_named_enum_kind_id("noinline"), 0,
            ));
        }

        if llvm_symbol != declaration.llvm_symbol {
            self.register_mir_contract_actual_symbol(declaration, &llvm_symbol);
        }

        if declaration.llvm_symbol == "main"
            && matches!(declaration.kind, MirCallableKind::LocalBody { .. })
        {
            function.set_linkage(Linkage::Internal);
        }

        Ok(function)
    }

    fn declare_mir_contract_closure_function(
        &mut self,
        declaration: &MirCallableDecl,
    ) -> Result<FunctionValue<'ctx>, CodegenError> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let mut param_types = vec![ptr_ty.into()];
        param_types.extend(
            declaration
                .signature
                .params
                .iter()
                .map(|param| self.llvm_param_type(param)),
        );
        let ret = declaration.signature.ret.abi_ty;
        let ret_ty = self.structural_type_for(ret);
        let fn_type = match ret_ty {
            Type::Unit => self.context.void_type().fn_type(&param_types, false),
            _ => self.llvm_type_id(ret).fn_type(&param_types, false),
        };
        let llvm_function =
            if let Some(existing) = self.module.get_function(&declaration.llvm_symbol) {
                existing
            } else {
                self.module
                    .add_function(&declaration.llvm_symbol, fn_type, None)
            };
        self.functions
            .insert(declaration.llvm_symbol.clone(), llvm_function);
        self.callables
            .bind_symbol_signature(&declaration.llvm_symbol, &declaration.signature);
        Ok(llvm_function)
    }

    fn register_mir_contract_actual_symbol(&mut self, declaration: &MirCallableDecl, symbol: &str) {
        self.callables.register_actual_symbol(declaration, symbol);
    }

    fn declare_mir_contract_extern(
        &mut self,
        declaration: &MirCallableDecl,
        link_name: &str,
        variadic: bool,
    ) -> Result<FunctionValue<'ctx>, CodegenError> {
        let param_types = declaration
            .signature
            .params
            .iter()
            .map(|param| self.llvm_param_type(param))
            .collect::<Vec<BasicMetadataTypeEnum<'ctx>>>();
        let ret = declaration.signature.ret.abi_ty;
        let ret_ty = self.structural_type_for(ret);
        let fn_type = match ret_ty {
            Type::Unit => self.context.void_type().fn_type(&param_types, variadic),
            _ => self.llvm_type_id(ret).fn_type(&param_types, variadic),
        };
        let base_name = link_name.split("::").last().unwrap_or(link_name);
        let llvm_function = if let Some(existing) = self.module.get_function(base_name) {
            existing
        } else {
            self.module.add_function(base_name, fn_type, None)
        };

        self.functions
            .insert(declaration.llvm_symbol.clone(), llvm_function);
        self.callables
            .bind_symbol_signature(&declaration.llvm_symbol, &declaration.signature);
        Ok(llvm_function)
    }

    pub(crate) fn llvm_param_type(&self, param: &MirParamAbi) -> BasicMetadataTypeEnum<'ctx> {
        match param.pass_mode {
            MirPassMode::Direct | MirPassMode::FatDirect => {
                self.llvm_type_id(param.semantic_ty).into()
            }
            MirPassMode::Pointer => self.context.ptr_type(AddressSpace::default()).into(),
        }
    }

    pub(crate) fn type_id_lowers_to_pointer(&self, ty: TypeId) -> bool {
        matches!(self.llvm_type_id(ty), BasicTypeEnum::PointerType(_))
    }

    fn declare_mir_function_with_signature(
        &mut self,
        lookup_symbol: &str,
        llvm_symbol: &str,
        signature: &MirCallableSignature,
    ) -> Result<FunctionValue<'ctx>, CodegenError> {
        let param_types = signature
            .params
            .iter()
            .map(|param| self.llvm_param_type(param))
            .collect::<Vec<BasicMetadataTypeEnum<'ctx>>>();
        let ret = signature.ret.abi_ty;
        let ret_type = self.structural_type_for(ret);
        let fn_type = match ret_type {
            Type::Unit => self.context.void_type().fn_type(&param_types, false),
            _ => self.llvm_type(&ret_type).fn_type(&param_types, false),
        };
        let llvm_function = if let Some(existing) = self.module.get_function(&llvm_symbol) {
            existing
        } else {
            self.module.add_function(&llvm_symbol, fn_type, None)
        };
        self.functions
            .insert(lookup_symbol.to_string(), llvm_function);
        self.functions
            .insert(llvm_symbol.to_string(), llvm_function);
        self.callables.bind_symbol_signature(lookup_symbol, signature);
        self.callables.bind_symbol_signature(llvm_symbol, signature);
        Ok(llvm_function)
    }

    fn local_mir_body_symbol_name(&self, declaration: &MirCallableDecl) -> String {
        if declaration.llvm_symbol == "main" {
            return self.unique_function_symbol_name("__rock_main");
        }

        if matches!(declaration.linkage, MirLinkage::Internal) {
            return self.unique_rock_function_symbol_name(&declaration.llvm_symbol);
        }

        let Some(namespace) = self.callables.namespace() else {
            return self.unique_rock_function_symbol_name(&declaration.llvm_symbol);
        };

        let desired =
            Self::exported_rock_function_symbol_name(&declaration.llvm_symbol, Some(namespace));
        self.unique_function_symbol_name(&desired)
    }

    pub(crate) fn exported_rock_function_symbol_name(
        name: &str,
        symbol_namespace: Option<&str>,
    ) -> String {
        let exported_name = if let Some(namespace) = symbol_namespace {
            let first_segment = name.split("::").next().unwrap_or(name);
            if name == "main" || first_segment == namespace {
                name.to_string()
            } else {
                format!("{}::{}", namespace, name)
            }
        } else {
            name.to_string()
        };

        if exported_name == "main" {
            exported_name
        } else {
            format!("__rock_{}", Self::mangle_function_symbol(&exported_name))
        }
    }

    fn unique_function_symbol_name(&self, desired: &str) -> String {
        if self.module.get_function(desired).is_none() {
            return desired.to_string();
        }

        let mut index = 0;
        loop {
            let candidate = format!("{}_{}", desired, index);
            if self.module.get_function(&candidate).is_none() {
                return candidate;
            }
            index += 1;
        }
    }
}
