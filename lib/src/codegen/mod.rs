//! LLVM IR Code Generation
//!
//! Converts MIR into LLVM IR using inkwell, then compiles to native code.
//!
//! This module is split into submodules by concern:
//! - `types`: Rock type to LLVM type mapping and coercion
//! - `intrinsics`: Typed intrinsic compilation (I64Add, F32Mul, etc.)
//! - `mir_llvm`: MIR-to-LLVM lowering boundary
//! - `runtime`: MIR/backend-shaped runtime helpers
//! - `output`: Object file writing, linking, and IR output

mod callable_resolution;
mod codegen_error;
mod contract_validation;
mod declarations;
mod integer;
mod intrinsics;
pub(crate) mod mir_llvm;
mod output;
mod runtime;
mod types;

use std::collections::HashMap;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::values::FunctionValue;

use crate::ids::{DefId, TypeId};
use crate::mir::{MirCallableKey, MirCallableSignature, MirFunctionId, MirProjectionKey};
use crate::types::{GenericParamId, Type};

pub(crate) use codegen_error::CodegenError;

#[derive(Debug, Clone)]
pub(crate) struct MirClosureCodegenMetadata {
    pub(crate) params: Vec<TypeId>,
    pub(crate) ret: TypeId,
    pub(crate) captures: Vec<TypeId>,
}

#[derive(Debug, Clone)]
pub(crate) enum CodegenEnumVariantFields {
    Unit,
    Positional(Vec<TypeId>),
    Named(Vec<(String, TypeId)>),
}

#[derive(Debug, Clone)]
pub(crate) struct CodegenEnumVariantLayout {
    pub(crate) fields: CodegenEnumVariantFields,
}

pub(crate) struct CodeGen<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    type_context: Option<crate::type_context::TypeContext>,
    /// Known functions
    functions: HashMap<String, FunctionValue<'ctx>>,
    mir_function_symbols: HashMap<MirFunctionId, String>,
    callable_symbols_by_key: HashMap<MirCallableKey, String>,
    callable_signatures_by_key: HashMap<MirCallableKey, MirCallableSignature>,
    callable_signatures_by_symbol: HashMap<String, MirCallableSignature>,
    mir_contract_symbol_overrides: HashMap<DefId, String>,
    symbol_namespace: Option<String>,
    mir_closure_metadata: HashMap<MirFunctionId, MirClosureCodegenMetadata>,
    projection_outputs: HashMap<MirProjectionKey, TypeId>,
    drop_glue_callables_by_type: HashMap<TypeId, MirCallableKey>,
    struct_layouts_by_id: HashMap<DefId, Vec<(String, TypeId)>>,
    struct_names_by_id: HashMap<DefId, String>,
    struct_generic_param_ids_by_id: HashMap<DefId, Vec<GenericParamId>>,
    /// Enum type info
    enum_layouts_by_id: HashMap<DefId, Vec<CodegenEnumVariantLayout>>,
    enum_names_by_id: HashMap<DefId, String>,
    /// Enum generic params for monomorphizing enum payload layouts
    enum_generic_param_ids_by_id: HashMap<DefId, Vec<GenericParamId>>,
    /// Current function being compiled
    current_function: Option<FunctionValue<'ctx>>,
    /// Wrapper thunks used when a named function is passed as a first-class value.
    function_value_wrappers:
        HashMap<(String, TypeId, Option<MirCallableSignature>), FunctionValue<'ctx>>,
}

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn new_with_symbol_namespace(
        context: &'ctx Context,
        module_name: &str,
        symbol_namespace: &str,
    ) -> Self {
        Self::new_inner(context, module_name, Some(symbol_namespace.to_string()))
    }

    fn new_inner(
        context: &'ctx Context,
        module_name: &str,
        symbol_namespace: Option<String>,
    ) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();

        Self {
            context,
            module,
            builder,
            type_context: None,
            functions: HashMap::new(),
            mir_function_symbols: HashMap::new(),
            callable_symbols_by_key: HashMap::new(),
            callable_signatures_by_key: HashMap::new(),
            callable_signatures_by_symbol: HashMap::new(),
            mir_contract_symbol_overrides: HashMap::new(),
            symbol_namespace,
            mir_closure_metadata: HashMap::new(),
            projection_outputs: HashMap::new(),
            drop_glue_callables_by_type: HashMap::new(),
            struct_layouts_by_id: HashMap::new(),
            struct_names_by_id: HashMap::new(),
            struct_generic_param_ids_by_id: HashMap::new(),
            enum_layouts_by_id: HashMap::new(),
            enum_names_by_id: HashMap::new(),
            enum_generic_param_ids_by_id: HashMap::new(),
            current_function: None,
            function_value_wrappers: HashMap::new(),
        }
    }

    pub(crate) fn mir_contract_symbol_overrides(&self) -> &HashMap<DefId, String> {
        &self.mir_contract_symbol_overrides
    }

    pub(crate) fn set_type_context(&mut self, type_context: crate::type_context::TypeContext) {
        self.type_context = Some(type_context);
    }

    pub(crate) fn type_context(&self) -> &crate::type_context::TypeContext {
        self.type_context
            .as_ref()
            .expect("codegen requires a TypeContext before TypeId lowering")
    }

    pub(crate) fn type_view(&self) -> crate::type_context::TypeView<'_> {
        crate::type_context::TypeView::new(self.type_context())
    }

    pub(crate) fn structural_type_for(&self, ty: TypeId) -> Type {
        self.type_context().type_for(ty)
    }

    pub(crate) fn display_type_for_diagnostic(&self, ty: &Type) -> String {
        let mut context = crate::type_services::display::TypeDisplayContext::default();
        for (id, name) in &self.struct_names_by_id {
            context.insert_definition_name(*id, name.clone());
        }
        for (id, name) in &self.enum_names_by_id {
            context.insert_definition_name(*id, name.clone());
        }
        crate::type_services::display::display_type_with_context(ty, &context).to_string()
    }

    pub(crate) fn intern_structural_type(&mut self, ty: &Type) -> TypeId {
        self.type_context
            .get_or_insert_with(crate::type_context::TypeContext::new)
            .intern_type(ty)
    }

    pub(crate) fn compile_program_from_mir(
        &mut self,
        mir: &crate::mir::MirProgram,
    ) -> Result<(), CodegenError> {
        mir_llvm::compile_mir_program(self, mir)
    }
}

#[cfg(test)]
impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn new(context: &'ctx Context, module_name: &str) -> Self {
        Self::new_inner(context, module_name, None)
    }
}

#[cfg(test)]
mod tests;
