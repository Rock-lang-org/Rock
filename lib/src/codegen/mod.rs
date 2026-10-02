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

#![cfg_attr(not(test), allow(dead_code))]

mod intrinsics;
pub(crate) mod mir_llvm;
mod output;
mod runtime;
mod types;

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::FunctionValue;
use inkwell::AddressSpace;

use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::ids::{DefId, TypeId};
use crate::lexer::Span;
use crate::mir::{
    MirBackendContract, MirCallable, MirCallableDecl, MirCallableKey, MirCallableKind,
    MirCallableSignature, MirFunction, MirFunctionId, MirLinkage, MirNominalLayout, MirParamAbi,
    MirPassMode, MirProjectionKey, MirRuntimeHelper, MirVariantLayoutFields,
};
use crate::types::{GenericParamId, Type};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodegenErrorKind {
    SourceOperation,
    BackendContract,
    Layout,
    Output,
    Link,
    Llvm,
    Toolchain,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodegenErrorLocation {
    Source(Span),
    File(PathBuf),
    Project(PathBuf),
    Artifact(PathBuf),
    Toolchain,
}

/// A structured code generation failure.
#[derive(Debug, Clone)]
pub(crate) struct CodegenError {
    message: String,
    kind: CodegenErrorKind,
    location: CodegenErrorLocation,
    notes: Vec<String>,
}

impl CodegenError {
    fn new(message: String) -> Self {
        Self {
            message,
            kind: CodegenErrorKind::Llvm,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    fn with_span(message: String, span: Span) -> Self {
        Self {
            message,
            kind: CodegenErrorKind::SourceOperation,
            location: CodegenErrorLocation::Source(span),
            notes: Vec::new(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::Internal,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    fn backend_contract(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::BackendContract,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    fn layout(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::Layout,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    fn project(message: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::Internal,
            location: CodegenErrorLocation::Project(path.into()),
            notes: Vec::new(),
        }
    }

    fn artifact(message: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::Internal,
            location: CodegenErrorLocation::Artifact(path.into()),
            notes: Vec::new(),
        }
    }

    fn output(message: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::Output,
            location: CodegenErrorLocation::File(path.into()),
            notes: Vec::new(),
        }
    }

    fn link(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::Link,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    fn toolchain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::Toolchain,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub(crate) fn internalize(mut self) -> Self {
        self.kind = CodegenErrorKind::Internal;
        self.location = CodegenErrorLocation::Toolchain;
        self
    }

    fn with_operation_span(mut self, span: Option<Span>) -> Self {
        if matches!(self.kind, CodegenErrorKind::SourceOperation) {
            if let Some(span) = span {
                self.location = CodegenErrorLocation::Source(span);
            }
        }
        self
    }

    pub(crate) fn into_diagnostic(self) -> Diagnostic {
        let CodegenError {
            message: raw_message,
            kind,
            location,
            notes,
        } = self;
        let message = Self::user_message(&raw_message);
        let mut diagnostic = match location {
            CodegenErrorLocation::Source(span) => {
                Diagnostic::new(message, span).with_code(DiagnosticCode::Codegen)
            }
            CodegenErrorLocation::File(path) => {
                Diagnostic::for_file(message, path).with_code(DiagnosticCode::Codegen)
            }
            CodegenErrorLocation::Project(path) => {
                Diagnostic::for_project(message, path).with_code(DiagnosticCode::Project)
            }
            CodegenErrorLocation::Artifact(path) => {
                Diagnostic::for_artifact(message, path).with_code(DiagnosticCode::Artifact)
            }
            CodegenErrorLocation::Toolchain => match kind {
                CodegenErrorKind::Internal
                | CodegenErrorKind::BackendContract
                | CodegenErrorKind::Layout => Diagnostic::for_internal(message),
                CodegenErrorKind::Output => {
                    Diagnostic::for_toolchain(message).with_code(DiagnosticCode::Codegen)
                }
                CodegenErrorKind::Link | CodegenErrorKind::Toolchain => {
                    Diagnostic::for_toolchain(message)
                }
                CodegenErrorKind::Llvm => {
                    Diagnostic::for_toolchain("LLVM code generation failed".to_string())
                        .with_code(DiagnosticCode::Codegen)
                        .with_note(message)
                }
                CodegenErrorKind::SourceOperation => Diagnostic::for_internal(message),
            },
        };
        for note in notes {
            diagnostic = diagnostic.with_note(Self::user_message(&note));
        }
        diagnostic
    }

    fn user_message(message: &str) -> String {
        const INTERNAL_IDENTITIES: [&str; 21] = [
            "DefId(",
            "DefId {",
            "InstanceId(",
            "TypeId(",
            "TypeVarId(",
            "TypeVarId {",
            "GenericParamId(",
            "GenericParamId {",
            "FieldId(",
            "FieldId {",
            "VariantId(",
            "VariantId {",
            "Local(",
            "Local {",
            "HirLocalId(",
            "HirLocalId {",
            "struct#",
            "enum#",
            "generic#",
            "trait#",
            "?T",
        ];
        if INTERNAL_IDENTITIES
            .iter()
            .any(|identity| message.contains(identity))
        {
            "Internal compiler error during MIR code generation".to_string()
        } else {
            message.to_string()
        }
    }
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "codegen error: {}", self.message)
    }
}

impl From<String> for CodegenError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for CodegenError {
    fn from(message: &str) -> Self {
        Self::new(message.to_string())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MirClosureCodegenMetadata {
    pub(crate) params: Vec<TypeId>,
    pub(crate) ret: TypeId,
    pub(crate) captures: Vec<TypeId>,
}

#[cfg(test)]
mod erased_tests;
#[cfg(test)]
mod object_adapter_tests;
#[cfg(test)]
mod object_tests;

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
    erased_functions: std::collections::BTreeMap<crate::mir::MirErasedKey, FunctionValue<'ctx>>,
    erased_descriptors: std::collections::BTreeMap<TypeId, inkwell::values::GlobalValue<'ctx>>,
    erased_dictionaries:
        std::collections::BTreeMap<crate::mir::MirDictionaryId, inkwell::values::GlobalValue<'ctx>>,
    erased_contract: crate::mir::MirErasedContract,
    object_schemas: std::collections::BTreeMap<TypeId, crate::mir::MirObjectSchema>,
    owned_object_calls:
        std::collections::BTreeMap<crate::mir::MirOwnedObjectKey, crate::mir::MirOwnedObjectPlan>,
    object_vtables:
        std::collections::BTreeMap<crate::mir::MirVtableId, inkwell::values::GlobalValue<'ctx>>,
    /// Known functions
    functions: HashMap<String, FunctionValue<'ctx>>,
    #[allow(dead_code)]
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
    fn register_mir_nominal_layouts(&mut self, contract: &MirBackendContract) {
        for layout in contract.nominal_layouts.values() {
            match layout {
                MirNominalLayout::Struct {
                    id,
                    fields,
                    generic_params,
                } => {
                    self.struct_layouts_by_id.insert(*id, fields.clone());
                    self.struct_generic_param_ids_by_id
                        .insert(*id, generic_params.clone());
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
                    self.enum_layouts_by_id.insert(*id, variants);
                    self.enum_generic_param_ids_by_id
                        .insert(*id, generic_params.clone());
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

    pub(crate) fn new(context: &'ctx Context, module_name: &str) -> Self {
        Self::new_inner(context, module_name, None)
    }

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
            erased_functions: Default::default(),
            erased_descriptors: Default::default(),
            erased_dictionaries: Default::default(),
            erased_contract: Default::default(),
            object_schemas: Default::default(),
            owned_object_calls: Default::default(),
            object_vtables: Default::default(),
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

    pub(crate) fn resolve_mir_callable_symbol(
        &self,
        callable: &MirCallable,
    ) -> Result<String, CodegenError> {
        let MirCallable::Resolved(key) = callable;

        self.callable_symbols_by_key
            .get(key)
            .cloned()
            .ok_or_else(|| {
                CodegenError::backend_contract(format!("Unknown MIR callable key {:?}", key))
            })
    }

    pub(crate) fn resolve_mir_callable_signature(
        &self,
        callable: &MirCallable,
    ) -> Option<MirCallableSignature> {
        let MirCallable::Resolved(key) = callable;
        self.callable_signatures_by_key.get(key).cloned()
    }

    pub(crate) fn resolve_mir_drop_glue_symbol(&self, ty: TypeId) -> Result<String, CodegenError> {
        if let Some(key) = self.drop_glue_callables_by_type.get(&ty) {
            return self
                .callable_symbols_by_key
                .get(key)
                .cloned()
                .ok_or_else(|| {
                    CodegenError::backend_contract(format!(
                        "Drop glue for type {:?} references unknown MIR callable key {:?}",
                        ty, key
                    ))
                });
        }

        Err(CodegenError::backend_contract(format!(
            "Missing drop glue for MIR type {:?}",
            ty
        )))
    }

    pub(crate) fn mir_drop_without_glue_is_noop(&self, ty: TypeId) -> bool {
        if self.drop_glue_callables_by_type.contains_key(&ty) {
            return false;
        }

        matches!(
            self.structural_type_for(ty),
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
                | Type::Slice(_)
                | Type::Reference { .. }
                | Type::Pointer(_)
        )
    }

    pub(crate) fn resolve_mir_callable_symbol_in_function(
        &self,
        callable: &MirCallable,
        function: &MirFunction,
    ) -> Result<String, CodegenError> {
        self.resolve_mir_callable_symbol(callable).map_err(|error| {
            CodegenError::from(format!(
                "{} in MIR function '{}'",
                error.message, function.name
            ))
        })
    }

    /// Declare the minimal C runtime support used directly by compiler-generated code.
    /// Ordinary external APIs are expected to come from user or dependency `extern`
    /// declarations, not compiler-owned registrations.
    fn declare_runtime(&mut self, runtime_requirements: &BTreeSet<MirRuntimeHelper>) {
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
        if runtime_requirements.contains(&MirRuntimeHelper::HeapFree)
            && !self.functions.contains_key("free")
        {
            let function = self.module.add_function(
                "free",
                self.context.void_type().fn_type(&[ptr_ty.into()], false),
                None,
            );
            self.functions.insert("free".into(), function);
        }
    }

    fn prepare_mir_program_declarations(
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
        self.emit_object_adapters(&mir.backend_contract)?;
        self.emit_erased_program(&mir.backend_contract)?;
        self.emit_object_vtables(&mir.backend_contract)?;

        Ok(())
    }

    fn declare_mir_functions(&mut self, mir: &crate::mir::MirProgram) -> Result<(), CodegenError> {
        self.declare_mir_closure_functions(mir)
    }

    fn validate_mir_backend_contract(
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

        let observed = crate::mir::backend_contract::runtime_requirements_for_backend(
            &mir.backend_contract,
            mir.functions.values(),
        );
        messages.extend(
            crate::mir::validate_object_contract(mir)
                .into_iter()
                .map(|error| format!("{error:?}")),
        );
        messages.extend(
            crate::mir::validate_erased_contract(mir)
                .into_iter()
                .map(|error| format!("invalid erased contract: {error:?}")),
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

    fn mir_place_is_projected_return_place(
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
                let fields = self.struct_layouts_by_id.get(id).ok_or_else(|| {
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
        self.callable_symbols_by_key
            .insert(declaration.key.clone(), declaration.llvm_symbol.clone());
        self.callable_signatures_by_key
            .insert(declaration.key.clone(), declaration.signature.clone());
        self.callable_signatures_by_symbol.insert(
            declaration.llvm_symbol.clone(),
            declaration.signature.clone(),
        );

        if let MirCallableKey::Closure(function_id) = &declaration.key {
            self.mir_function_symbols
                .insert(function_id.clone(), declaration.llvm_symbol.clone());
        }

        if let MirCallableKind::LocalBody { function_id } = &declaration.kind {
            self.mir_function_symbols
                .insert(function_id.clone(), declaration.llvm_symbol.clone());
        }
    }

    fn declare_mir_contract_callable(
        &mut self,
        declaration: &MirCallableDecl,
    ) -> Result<(), CodegenError> {
        match &declaration.kind {
            MirCallableKind::LocalBody { .. }
            | MirCallableKind::ObjectProvided
            | MirCallableKind::ObjectAdapter(_) => {
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
            Type::Unit | Type::Never => self.context.void_type().fn_type(&param_types, false),
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
        self.callable_signatures_by_symbol.insert(
            declaration.llvm_symbol.clone(),
            declaration.signature.clone(),
        );
        Ok(llvm_function)
    }

    fn register_mir_contract_actual_symbol(&mut self, declaration: &MirCallableDecl, symbol: &str) {
        self.callable_symbols_by_key
            .insert(declaration.key.clone(), symbol.to_string());
        if let Some(def_id) = declaration.source_def_id {
            self.mir_contract_symbol_overrides
                .insert(def_id, symbol.to_string());
        }

        if let MirCallableKey::Closure(function_id) = &declaration.key {
            self.mir_function_symbols
                .insert(function_id.clone(), symbol.to_string());
        }

        if let MirCallableKind::LocalBody { function_id } = &declaration.kind {
            self.mir_function_symbols
                .insert(function_id.clone(), symbol.to_string());
        }
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
            Type::Unit | Type::Never => self.context.void_type().fn_type(&param_types, variadic),
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
        self.callable_signatures_by_symbol.insert(
            declaration.llvm_symbol.clone(),
            declaration.signature.clone(),
        );
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
            Type::Unit | Type::Never => self.context.void_type().fn_type(&param_types, false),
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
        self.callable_signatures_by_symbol
            .insert(lookup_symbol.to_string(), signature.clone());
        self.callable_signatures_by_symbol
            .insert(llvm_symbol.to_string(), signature.clone());
        Ok(llvm_function)
    }

    fn local_mir_body_symbol_name(&self, declaration: &MirCallableDecl) -> String {
        if declaration.llvm_symbol == "main" {
            return self.unique_function_symbol_name("__rock_main");
        }

        if matches!(declaration.linkage, MirLinkage::Internal) {
            return self.unique_rock_function_symbol_name(&declaration.llvm_symbol);
        }

        let Some(namespace) = self.symbol_namespace.as_deref() else {
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

    pub(crate) fn compile_program_from_mir(
        &mut self,
        mir: &crate::mir::MirProgram,
    ) -> Result<(), CodegenError> {
        mir_llvm::compile_mir_program(self, mir)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use inkwell::context::Context;

    use crate::diagnostic::{DiagnosticCode, DiagnosticLocation};
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::mono::InstanceId;
    use crate::types::Type;

    use super::{CodeGen, CodegenError};

    fn type_id(
        type_context: &mut crate::type_context::TypeContext,
        ty: Type,
    ) -> crate::ids::TypeId {
        type_context.intern_type(&ty)
    }

    fn local_instance_contract(
        type_context: &crate::type_context::TypeContext,
        instance_id: InstanceId,
        def_id: DefId,
        symbol: &str,
        params: Vec<crate::ids::TypeId>,
        ret: crate::ids::TypeId,
        is_method: bool,
    ) -> crate::mir::MirBackendContract {
        let key = crate::mir::MirCallableKey::Instance(instance_id);
        let function_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mut contract = crate::mir::MirBackendContract::default();
        let mut signature = crate::mir::MirCallableSignature::from_instance_type_ids(
            &params,
            ret,
            is_method,
            type_context,
        );
        if symbol == "main" {
            if let Some(i32_ty) = type_context.id_for_type(&Type::I32) {
                signature.ret.abi_ty = i32_ty;
            }
        }
        contract.callables.insert(
            key.clone(),
            crate::mir::MirCallableDecl {
                key: key.clone(),
                source_def_id: Some(def_id),
                kind: crate::mir::MirCallableKind::LocalBody {
                    function_id: function_id.clone(),
                },
                llvm_symbol: symbol.to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature,
            },
        );
        contract.function_bodies.insert(function_id, key);
        contract
    }

    #[test]
    fn declare_runtime_only_declares_contract_requirements() {
        let context = Context::create();

        let mut bounds_codegen = CodeGen::new(&context, "bounds");
        bounds_codegen
            .declare_runtime(&BTreeSet::from([crate::mir::MirRuntimeHelper::BoundsCheck]));
        assert!(bounds_codegen.module.get_function("puts").is_some());
        assert!(bounds_codegen.module.get_function("exit").is_some());
        assert!(bounds_codegen.module.get_function("malloc").is_none());

        let mut heap_codegen = CodeGen::new(&context, "heap");
        heap_codegen.declare_runtime(&BTreeSet::from([crate::mir::MirRuntimeHelper::HeapAlloc]));
        assert!(heap_codegen.module.get_function("puts").is_none());
        assert!(heap_codegen.module.get_function("exit").is_none());
        assert!(heap_codegen.module.get_function("malloc").is_some());

        let mut empty_codegen = CodeGen::new(&context, "empty");
        empty_codegen.declare_runtime(&BTreeSet::new());
        assert!(empty_codegen.module.get_function("puts").is_none());
        assert!(empty_codegen.module.get_function("exit").is_none());
        assert!(empty_codegen.module.get_function("malloc").is_none());
    }

    #[test]
    fn compile_mir_program_rejects_missing_heap_allocation_requirement_before_lowering() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "missing_heap_requirement");
        let instance_id = InstanceId(301);
        let function_id = DefId::new(CrateId(0), LocalDefId(301));
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let closure_id = crate::mir::MirClosureId {
            owner: mir_id.clone(),
            local_index: 0,
        };
        let mut type_context = crate::type_context::TypeContext::new();
        let unit = type_id(&mut type_context, Type::Unit);
        let closure_ty = type_id(&mut type_context, Type::function(Vec::new(), Type::Unit));
        let function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "main".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(1),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Closure(crate::mir::MirClosure {
                        id: closure_id,
                        display_name: "captured".to_string(),
                        captures: vec![crate::mir::MirClosureCapture {
                            name: "capture".to_string(),
                            local: crate::mir::Local(0),
                            kind: crate::mir::MirClosureCaptureKind::ByValue,
                            span: None,
                        }],
                    }),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![
                crate::mir::LocalDecl {
                    ty: unit,
                    mutability: crate::mir::Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                crate::mir::LocalDecl {
                    ty: closure_ty,
                    mutability: crate::mir::Mutability::Mut,
                    name: Some("closure".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: unit,
            ownership: Default::default(),
        };
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, function)]),
            type_context: type_context.clone(),
            backend_contract: local_instance_contract(
                &type_context,
                instance_id,
                function_id,
                "main",
                Vec::new(),
                unit,
                false,
            ),
        };

        let error = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            error.message.contains("runtime requirement mismatch")
                && error.message.contains("HeapAlloc"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn compile_mir_program_rejects_stale_heap_allocation_requirement_before_lowering() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "stale_heap_requirement");
        let instance_id = InstanceId(302);
        let function_id = DefId::new(CrateId(0), LocalDefId(302));
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mut type_context = crate::type_context::TypeContext::new();
        let unit = type_id(&mut type_context, Type::Unit);
        let function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "main".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![crate::mir::LocalDecl {
                ty: unit,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: unit,
            ownership: Default::default(),
        };
        let mut backend_contract = local_instance_contract(
            &type_context,
            instance_id,
            function_id,
            "main",
            Vec::new(),
            unit,
            false,
        );
        backend_contract
            .runtime_requirements
            .insert(crate::mir::MirRuntimeHelper::HeapAlloc);
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, function)]),
            type_context,
            backend_contract,
        };

        let error = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            error.message.contains("runtime requirement mismatch")
                && error.message.contains("HeapAlloc"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn mangle_function_symbol_distinguishes_separator_and_underscore_paths() {
        let first = CodeGen::mangle_function_symbol("foo::bar__baz");
        let second = CodeGen::mangle_function_symbol("foo__bar::baz");

        assert_eq!(first, "s3_666f6f_s8_6261725f5f62617a");
        assert_eq!(second, "s8_666f6f5f5f626172_s3_62617a");
        assert_ne!(first, second);
    }

    #[test]
    fn mir_callable_resolved_key_missing_from_contract_fails_without_name_lookup() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let instance = InstanceId(210);
        let present_key = crate::mir::MirCallableKey::Instance(instance);
        let missing_key = crate::mir::MirCallableKey::Instance(InstanceId(211));
        codegen.functions.insert(
            "Box_show_exact".to_string(),
            codegen.module.add_function(
                "Box_show_exact",
                context.i64_type().fn_type(&[], false),
                None,
            ),
        );
        codegen.functions.insert(
            "show".to_string(),
            codegen
                .module
                .add_function("show", context.i64_type().fn_type(&[], false), None),
        );
        codegen
            .callable_symbols_by_key
            .insert(present_key.clone(), "Box_show_exact".to_string());

        let symbol = codegen
            .resolve_mir_callable_symbol(&crate::mir::MirCallable::Resolved(present_key))
            .unwrap();

        assert_eq!(symbol, "Box_show_exact");

        let err = codegen
            .resolve_mir_callable_symbol(&crate::mir::MirCallable::Resolved(missing_key))
            .unwrap_err();

        assert!(
            err.message.contains("Unknown MIR callable key"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compile_mir_program_uses_mir_body() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let function_id = DefId::new(CrateId(0), LocalDefId(1));
        let instance_id = InstanceId(0);
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        type_id(&mut type_context, Type::I32);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "main".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(99),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let backend_contract = local_instance_contract(
            &type_context,
            instance_id,
            function_id,
            "main",
            Vec::new(),
            i64_type,
            false,
        );
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        codegen.compile_program_from_mir(&mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("ret i32 99"), "IR was:\n{}", ir);
        assert!(!ir.contains("ret i32 1"), "IR was:\n{}", ir);
    }

    #[test]
    fn mir_llvm_module_exposes_focused_compile_api() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let function_id = DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(91));
        let instance_id = InstanceId(91);
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        type_id(&mut type_context, Type::I32);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "main".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(7),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let backend_contract = local_instance_contract(
            &type_context,
            instance_id,
            function_id,
            "main",
            Vec::new(),
            i64_type,
            false,
        );
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        super::mir_llvm::compile_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("ret i32 7"), "IR was:\n{}", ir);
    }

    #[test]
    fn compile_mir_program_declares_instance_from_mir_body_without_hir_body() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let function_id = DefId::new(CrateId(0), LocalDefId(7));
        let instance_id = InstanceId(7);
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        type_id(&mut type_context, Type::I32);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "main".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(42),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let backend_contract = local_instance_contract(
            &type_context,
            instance_id,
            function_id,
            "main",
            Vec::new(),
            i64_type,
            false,
        );
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        codegen.compile_program_from_mir(&mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("ret i32 42"), "IR was:\n{}", ir);
    }

    #[test]
    fn compile_mir_program_declares_instance_from_backend_contract() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let function_id = DefId::new(CrateId(0), LocalDefId(97));
        let instance_id = InstanceId(97);
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "contract_instance".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(123),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let key = crate::mir::MirCallableKey::Instance(instance_id);
        let mut backend_contract = crate::mir::MirBackendContract::default();
        backend_contract.callables.insert(
            key.clone(),
            crate::mir::MirCallableDecl {
                key: key.clone(),
                source_def_id: Some(function_id),
                kind: crate::mir::MirCallableKind::LocalBody {
                    function_id: mir_id.clone(),
                },
                llvm_symbol: "contract_instance_symbol".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[],
                    i64_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract.function_bodies.insert(mir_id.clone(), key);
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        codegen.compile_program_from_mir(&mir).unwrap();

        let ir = codegen.get_ir();
        assert!(
            ir.contains("define i64 @contract_instance_symbol()"),
            "IR was:\n{}",
            ir
        );
        assert!(ir.contains("ret i64 123"), "IR was:\n{}", ir);
    }

    #[test]
    fn compile_mir_program_rejects_missing_backend_contract() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let instance_id = InstanceId(98);
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "missing_contract".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(1),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract: Default::default(),
        };

        let err = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            err.message.contains("Invalid MIR backend contract"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compile_mir_program_rejects_resolved_callable_missing_from_backend_contract() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let function_id = DefId::new(CrateId(0), LocalDefId(198));
        let instance_id = InstanceId(198);
        let missing_key = crate::mir::MirCallableKey::Instance(InstanceId(199));
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "missing_resolved_callable".to_string(),
            basic_blocks: vec![
                crate::mir::BasicBlock {
                    statements: vec![],
                    terminator: Some(crate::mir::Terminator::Call {
                        func: crate::mir::Operand::Constant(crate::mir::Constant::Callable(
                            crate::mir::MirCallable::Resolved(missing_key.clone()),
                        )),
                        args: vec![],
                        destination: crate::mir::Place {
                            local: crate::mir::Local(0),
                            projection: vec![],
                        },
                        target: crate::mir::BasicBlockId(1),
                        span: None,
                    }),
                },
                crate::mir::BasicBlock {
                    statements: vec![],
                    terminator: Some(crate::mir::Terminator::Return),
                },
            ],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
        let mut backend_contract = crate::mir::MirBackendContract::default();
        backend_contract.callables.insert(
            caller_key.clone(),
            crate::mir::MirCallableDecl {
                key: caller_key.clone(),
                source_def_id: Some(function_id),
                kind: crate::mir::MirCallableKind::LocalBody {
                    function_id: mir_id.clone(),
                },
                llvm_symbol: "missing_resolved_callable".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[],
                    i64_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract
            .function_bodies
            .insert(mir_id.clone(), caller_key);
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        let err = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            err.message.contains("Invalid MIR backend contract")
                && err.message.contains("ResolvedCallableWithoutContract"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compile_mir_program_rejects_wrong_callable_key_even_when_source_alias_registered() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let caller_def = DefId::new(CrateId(0), LocalDefId(201));
        let callee_def = DefId::new(CrateId(0), LocalDefId(202));
        let callee_instance = InstanceId(202);
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let mir_id = crate::mir::MirFunctionId::Function(caller_def);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "source_alias_callable".to_string(),
            basic_blocks: vec![
                crate::mir::BasicBlock {
                    statements: vec![],
                    terminator: Some(crate::mir::Terminator::Call {
                        func: crate::mir::Operand::Constant(crate::mir::Constant::Callable(
                            crate::mir::MirCallable::Resolved(
                                crate::mir::MirCallableKey::Function(callee_def),
                            ),
                        )),
                        args: vec![],
                        destination: crate::mir::Place {
                            local: crate::mir::Local(0),
                            projection: vec![],
                        },
                        target: crate::mir::BasicBlockId(1),
                        span: None,
                    }),
                },
                crate::mir::BasicBlock {
                    statements: vec![],
                    terminator: Some(crate::mir::Terminator::Return),
                },
            ],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let caller_key = crate::mir::MirCallableKey::Function(caller_def);
        let callee_key = crate::mir::MirCallableKey::Instance(callee_instance);
        let mut backend_contract = crate::mir::MirBackendContract::default();
        backend_contract.callables.insert(
            caller_key.clone(),
            crate::mir::MirCallableDecl {
                key: caller_key.clone(),
                source_def_id: Some(caller_def),
                kind: crate::mir::MirCallableKind::LocalBody {
                    function_id: mir_id.clone(),
                },
                llvm_symbol: "source_alias_callable".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[],
                    i64_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract.callables.insert(
            callee_key.clone(),
            crate::mir::MirCallableDecl {
                key: callee_key,
                source_def_id: Some(callee_def),
                kind: crate::mir::MirCallableKind::ObjectProvided,
                llvm_symbol: "callee_alias_callable".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[],
                    i64_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract
            .function_bodies
            .insert(mir_id.clone(), caller_key);
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        let err = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            err.message.contains("Invalid MIR backend contract")
                && err.message.contains("ResolvedCallableWithoutContract"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compile_mir_program_rejects_direct_intrinsic_callable_without_contract() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let function_id = DefId::new(CrateId(0), LocalDefId(199));
        let instance_id = InstanceId(200);
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "direct_intrinsic_bypass".to_string(),
            basic_blocks: vec![
                crate::mir::BasicBlock {
                    statements: vec![],
                    terminator: Some(crate::mir::Terminator::Call {
                        func: crate::mir::Operand::Constant(crate::mir::Constant::Callable(
                            crate::mir::MirCallable::Resolved(
                                crate::mir::MirCallableKey::Intrinsic(
                                    crate::mir::MirIntrinsicId::I64Add,
                                ),
                            ),
                        )),
                        args: vec![
                            crate::mir::Operand::Constant(crate::mir::Constant::Int(1)),
                            crate::mir::Operand::Constant(crate::mir::Constant::Int(2)),
                        ],
                        destination: crate::mir::Place {
                            local: crate::mir::Local(0),
                            projection: vec![],
                        },
                        target: crate::mir::BasicBlockId(1),
                        span: None,
                    }),
                },
                crate::mir::BasicBlock {
                    statements: vec![],
                    terminator: Some(crate::mir::Terminator::Return),
                },
            ],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
        let mut backend_contract = crate::mir::MirBackendContract::default();
        backend_contract.callables.insert(
            caller_key.clone(),
            crate::mir::MirCallableDecl {
                key: caller_key.clone(),
                source_def_id: Some(function_id),
                kind: crate::mir::MirCallableKind::LocalBody {
                    function_id: mir_id.clone(),
                },
                llvm_symbol: "direct_intrinsic_bypass".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[],
                    i64_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract
            .function_bodies
            .insert(mir_id.clone(), caller_key);
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        let err = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            err.message.contains("Invalid MIR backend contract")
                || err.message.contains("Unknown MIR callable"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compile_mir_program_lowers_resolved_intrinsic_from_backend_contract() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let function_id = DefId::new(CrateId(0), LocalDefId(200));
        let instance_id = InstanceId(201);
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let intrinsic_key =
            crate::mir::MirCallableKey::Intrinsic(crate::mir::MirIntrinsicId::I64Add);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "resolved_intrinsic_contract".to_string(),
            basic_blocks: vec![
                crate::mir::BasicBlock {
                    statements: vec![],
                    terminator: Some(crate::mir::Terminator::Call {
                        func: crate::mir::Operand::Constant(crate::mir::Constant::Callable(
                            crate::mir::MirCallable::Resolved(intrinsic_key.clone()),
                        )),
                        args: vec![
                            crate::mir::Operand::Constant(crate::mir::Constant::Int(1)),
                            crate::mir::Operand::Constant(crate::mir::Constant::Int(2)),
                        ],
                        destination: crate::mir::Place {
                            local: crate::mir::Local(0),
                            projection: vec![],
                        },
                        target: crate::mir::BasicBlockId(1),
                        span: None,
                    }),
                },
                crate::mir::BasicBlock {
                    statements: vec![],
                    terminator: Some(crate::mir::Terminator::Return),
                },
            ],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
        let mut backend_contract = crate::mir::MirBackendContract::default();
        backend_contract.callables.insert(
            caller_key.clone(),
            crate::mir::MirCallableDecl {
                key: caller_key.clone(),
                source_def_id: Some(function_id),
                kind: crate::mir::MirCallableKind::LocalBody {
                    function_id: mir_id.clone(),
                },
                llvm_symbol: "resolved_intrinsic_contract".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[],
                    i64_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract.callables.insert(
            intrinsic_key.clone(),
            crate::mir::MirCallableDecl {
                key: intrinsic_key.clone(),
                source_def_id: None,
                kind: crate::mir::MirCallableKind::Intrinsic(crate::mir::MirIntrinsicId::I64Add),
                llvm_symbol: "I64Add".to_string(),
                linkage: crate::mir::MirLinkage::Internal,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[i64_type, i64_type],
                    i64_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract
            .function_bodies
            .insert(mir_id.clone(), caller_key);
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        codegen.compile_program_from_mir(&mir).unwrap();

        let ir = codegen.get_ir();
        assert!(codegen.callable_symbols_by_key.contains_key(&intrinsic_key));
        assert!(!ir.contains("call i64 @I64Add"), "IR was:\n{}", ir);
        assert!(ir.contains("ret i64"), "IR was:\n{}", ir);
    }

    #[test]
    fn compile_mir_program_declares_extern_from_backend_contract_without_hir_extern() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let extern_id = DefId::new(CrateId(0), LocalDefId(90));
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let extern_key = crate::mir::MirCallableKey::Extern(extern_id);
        let mut backend_contract = crate::mir::MirBackendContract::default();
        backend_contract.callables.insert(
            extern_key.clone(),
            crate::mir::MirCallableDecl {
                key: extern_key,
                source_def_id: Some(extern_id),
                kind: crate::mir::MirCallableKind::Extern {
                    link_name: "ffi::answer".to_string(),
                    variadic: false,
                },
                llvm_symbol: "ffi::answer".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[i64_type],
                    i64_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::new(),
            type_context,
            backend_contract,
        };

        codegen.compile_program_from_mir(&mir).unwrap();

        assert!(codegen.module.get_function("answer").is_some());
        assert_eq!(
            codegen
                .callable_symbols_by_key
                .get(&crate::mir::MirCallableKey::Extern(extern_id))
                .map(String::as_str),
            Some("ffi::answer")
        );
        assert!(codegen.functions.contains_key("ffi::answer"));
    }

    #[test]
    fn compile_mir_program_registers_struct_layout_from_backend_contract_without_hir_struct() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let struct_id = DefId::new(CrateId(0), LocalDefId(91));
        let instance_id = InstanceId(91);
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let point_type = type_id(
            &mut type_context,
            Type::Struct {
                id: struct_id,
                args: Vec::new(),
            },
        );
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "make_point".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: Vec::new(),
                    },
                    crate::mir::Rvalue::Aggregate(
                        crate::mir::AggregateKind::Struct {
                            id: struct_id,
                            display_name: "Point".to_string(),
                        },
                        vec![
                            crate::mir::Operand::Constant(crate::mir::Constant::Int(1)),
                            crate::mir::Operand::Constant(crate::mir::Constant::Int(2)),
                        ],
                    ),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![crate::mir::LocalDecl {
                ty: point_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: point_type,
            ownership: Default::default(),
        };
        let mut backend_contract = local_instance_contract(
            &type_context,
            instance_id,
            DefId::new(CrateId(0), LocalDefId(92)),
            "make_point",
            Vec::new(),
            point_type,
            false,
        );
        backend_contract.nominal_layouts.insert(
            struct_id,
            crate::mir::MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("x".to_string(), i64_type), ("y".to_string(), i64_type)],
                generic_params: Vec::new(),
            },
        );
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        codegen.compile_program_from_mir(&mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("{ i64 1, i64 2 }"), "IR was:\n{}", ir);
        assert_eq!(
            codegen.struct_layouts_by_id.get(&struct_id),
            Some(&vec![
                ("x".to_string(), i64_type),
                ("y".to_string(), i64_type),
            ])
        );
    }

    #[test]
    fn compile_mir_program_binds_shared_scalar_method_receiver_by_pointer() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let self_ref = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };
        let self_ref_type = type_id(&mut type_context, self_ref.clone());
        let function_id = DefId::new(CrateId(0), LocalDefId(80));
        let instance_id = InstanceId(0);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let self_place = crate::mir::Place {
            local: crate::mir::Local(1),
            projection: vec![],
        };
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "I64_identity".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Copy(crate::mir::Place {
                        local: self_place.local,
                        projection: vec![crate::mir::Projection::Deref],
                    })),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![
                crate::mir::LocalDecl {
                    ty: i64_type,
                    mutability: crate::mir::Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                crate::mir::LocalDecl {
                    ty: self_ref_type,
                    mutability: crate::mir::Mutability::Not,
                    name: Some("self".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let backend_contract = local_instance_contract(
            &type_context,
            instance_id,
            function_id,
            "I64_identity",
            vec![self_ref_type],
            i64_type,
            true,
        );
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        codegen.compile_program_from_mir(&mir).unwrap();

        let ir = codegen.get_ir();
        assert!(
            ir.contains("define i64 @I64_identity(ptr %0)"),
            "IR was:\n{}",
            ir
        );
        assert!(ir.contains("%self = alloca ptr"), "IR was:\n{}", ir);
        assert!(ir.contains("store ptr %0, ptr %self"), "IR was:\n{}", ir);
        assert!(ir.contains("load i64"), "IR was:\n{}", ir);
    }

    #[test]
    fn compile_mir_program_uses_contract_pass_mode_for_method_receiver_without_legacy_metadata() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let function_id = DefId::new(CrateId(0), LocalDefId(81));
        let instance_id = InstanceId(81);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "contract_only_shared_scalar_identity".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Copy(crate::mir::Place {
                        local: crate::mir::Local(1),
                        projection: vec![],
                    })),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![
                crate::mir::LocalDecl {
                    ty: i64_type,
                    mutability: crate::mir::Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                crate::mir::LocalDecl {
                    ty: i64_type,
                    mutability: crate::mir::Mutability::Not,
                    name: Some("self".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let key = crate::mir::MirCallableKey::Instance(instance_id);
        let mut backend_contract = crate::mir::MirBackendContract::default();
        backend_contract.callables.insert(
            key.clone(),
            crate::mir::MirCallableDecl {
                key: key.clone(),
                source_def_id: Some(function_id),
                kind: crate::mir::MirCallableKind::LocalBody {
                    function_id: mir_id.clone(),
                },
                llvm_symbol: "contract_only_shared_scalar_identity".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature {
                    params: vec![crate::mir::MirParamAbi {
                        semantic_ty: i64_type,
                        pass_mode: crate::mir::MirPassMode::Pointer,
                    }],
                    ret: crate::mir::MirReturnAbi {
                        semantic_ty: i64_type,
                        abi_ty: i64_type,
                    },
                },
            },
        );
        backend_contract.function_bodies.insert(mir_id.clone(), key);
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        codegen.compile_program_from_mir(&mir).unwrap();

        let ir = codegen.get_ir();
        assert!(
            ir.contains("define i64 @contract_only_shared_scalar_identity(ptr %0)"),
            "IR was:\n{}",
            ir,
        );
        assert!(ir.contains("load i64, ptr %0"), "IR was:\n{}", ir);
        assert!(ir.contains("store i64"), "IR was:\n{}", ir);
    }

    #[test]
    fn compile_mir_program_rejects_drop_glue_contract_without_receiver_param() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let struct_id = DefId::new(CrateId(0), LocalDefId(82));
        let box_type = Type::Struct {
            id: struct_id,
            args: Vec::new(),
        };
        let box_type_id = type_id(&mut type_context, box_type);
        let function_id = DefId::new(CrateId(0), LocalDefId(83));
        let instance_id = InstanceId(83);
        let drop_instance = InstanceId(84);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "drop_contract_missing_receiver".to_string(),
            basic_blocks: vec![
                crate::mir::BasicBlock {
                    statements: vec![],
                    terminator: Some(crate::mir::Terminator::Drop {
                        place: crate::mir::Place {
                            local: crate::mir::Local(1),
                            projection: vec![],
                        },
                        target: crate::mir::BasicBlockId(1),
                    }),
                },
                crate::mir::BasicBlock {
                    statements: vec![crate::mir::StatementData::assign(
                        crate::mir::Place {
                            local: crate::mir::Local(0),
                            projection: vec![],
                        },
                        crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                            crate::mir::Constant::Int(0),
                        )),
                        None,
                    )],
                    terminator: Some(crate::mir::Terminator::Return),
                },
            ],
            local_decls: vec![
                crate::mir::LocalDecl {
                    ty: i64_type,
                    mutability: crate::mir::Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                crate::mir::LocalDecl {
                    ty: box_type_id,
                    mutability: crate::mir::Mutability::Not,
                    name: Some("box_value".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
        let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
        let mut backend_contract = crate::mir::MirBackendContract::default();
        backend_contract.callables.insert(
            caller_key.clone(),
            crate::mir::MirCallableDecl {
                key: caller_key.clone(),
                source_def_id: Some(function_id),
                kind: crate::mir::MirCallableKind::LocalBody {
                    function_id: mir_id.clone(),
                },
                llvm_symbol: "drop_contract_missing_receiver".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[],
                    i64_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract.callables.insert(
            drop_key.clone(),
            crate::mir::MirCallableDecl {
                key: drop_key.clone(),
                source_def_id: Some(DefId::new(CrateId(0), LocalDefId(84))),
                kind: crate::mir::MirCallableKind::ObjectProvided,
                llvm_symbol: "Box_drop".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[],
                    type_id(&mut type_context, Type::Unit),
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract
            .function_bodies
            .insert(mir_id.clone(), caller_key);
        backend_contract.drop_glue.insert(box_type_id, drop_key);
        backend_contract.nominal_layouts.insert(
            struct_id,
            crate::mir::MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("value".to_string(), i64_type)],
                generic_params: Vec::new(),
            },
        );
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        let err = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            err.message.contains("Invalid MIR backend contract")
                && err.message.contains("receiver parameter"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compile_mir_program_rejects_unused_drop_glue_contract_with_bad_signature() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let struct_id = DefId::new(CrateId(0), LocalDefId(182));
        let box_type_id = type_id(
            &mut type_context,
            Type::Struct {
                id: struct_id,
                args: Vec::new(),
            },
        );
        let function_id = DefId::new(CrateId(0), LocalDefId(183));
        let instance_id = InstanceId(183);
        let drop_instance = InstanceId(184);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "unused_bad_drop_contract".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(0),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
        let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
        let mut backend_contract = local_instance_contract(
            &type_context,
            instance_id,
            function_id,
            "unused_bad_drop_contract",
            Vec::new(),
            i64_type,
            false,
        );
        backend_contract.callables.insert(
            drop_key.clone(),
            crate::mir::MirCallableDecl {
                key: drop_key.clone(),
                source_def_id: Some(DefId::new(CrateId(0), LocalDefId(184))),
                kind: crate::mir::MirCallableKind::ObjectProvided,
                llvm_symbol: "Box_drop".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[box_type_id, i64_type],
                    i64_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract
            .function_bodies
            .insert(mir_id.clone(), caller_key);
        backend_contract.drop_glue.insert(box_type_id, drop_key);
        backend_contract.nominal_layouts.insert(
            struct_id,
            crate::mir::MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("value".to_string(), i64_type)],
                generic_params: Vec::new(),
            },
        );
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        let err = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            err.message.contains("Invalid MIR backend contract")
                && err.message.contains("expected 1 receiver parameter")
                && err.message.contains("expected ()"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compile_mir_program_rejects_unused_drop_glue_contract_with_unknown_type_id() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let unit_type = type_id(&mut type_context, Type::Unit);
        let struct_id = DefId::new(CrateId(0), LocalDefId(185));
        let box_type_id = type_id(
            &mut type_context,
            Type::Struct {
                id: struct_id,
                args: Vec::new(),
            },
        );
        let unknown_type = crate::ids::TypeId(u32::MAX);
        let function_id = DefId::new(CrateId(0), LocalDefId(186));
        let instance_id = InstanceId(186);
        let drop_instance = InstanceId(187);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "unused_unknown_drop_contract".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(0),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
        let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
        let mut backend_contract = local_instance_contract(
            &type_context,
            instance_id,
            function_id,
            "unused_unknown_drop_contract",
            Vec::new(),
            i64_type,
            false,
        );
        backend_contract.callables.insert(
            drop_key.clone(),
            crate::mir::MirCallableDecl {
                key: drop_key.clone(),
                source_def_id: Some(DefId::new(CrateId(0), LocalDefId(187))),
                kind: crate::mir::MirCallableKind::ObjectProvided,
                llvm_symbol: "Box_drop".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[box_type_id],
                    unit_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract
            .function_bodies
            .insert(mir_id.clone(), caller_key);
        backend_contract.drop_glue.insert(unknown_type, drop_key);
        backend_contract.nominal_layouts.insert(
            struct_id,
            crate::mir::MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("value".to_string(), i64_type)],
                generic_params: Vec::new(),
            },
        );
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        let err = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            err.message.contains("Invalid MIR backend contract")
                && err.message.contains("unknown dropped type"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compile_mir_program_rejects_unused_drop_glue_contract_with_unknown_return_type_id() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let struct_id = DefId::new(CrateId(0), LocalDefId(188));
        let box_type_id = type_id(
            &mut type_context,
            Type::Struct {
                id: struct_id,
                args: Vec::new(),
            },
        );
        let unknown_type = crate::ids::TypeId(u32::MAX);
        let function_id = DefId::new(CrateId(0), LocalDefId(189));
        let instance_id = InstanceId(189);
        let drop_instance = InstanceId(190);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "unused_unknown_return_drop_contract".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(0),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
        let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
        let mut backend_contract = local_instance_contract(
            &type_context,
            instance_id,
            function_id,
            "unused_unknown_return_drop_contract",
            Vec::new(),
            i64_type,
            false,
        );
        backend_contract.callables.insert(
            drop_key.clone(),
            crate::mir::MirCallableDecl {
                key: drop_key.clone(),
                source_def_id: Some(DefId::new(CrateId(0), LocalDefId(190))),
                kind: crate::mir::MirCallableKind::ObjectProvided,
                llvm_symbol: "Box_drop".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[box_type_id],
                    unknown_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract
            .function_bodies
            .insert(mir_id.clone(), caller_key);
        backend_contract.drop_glue.insert(box_type_id, drop_key);
        backend_contract.nominal_layouts.insert(
            struct_id,
            crate::mir::MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("value".to_string(), i64_type)],
                generic_params: Vec::new(),
            },
        );
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        let err = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            err.message.contains("Invalid MIR backend contract")
                && err.message.contains("unknown return semantic type"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compile_mir_program_rejects_unused_callable_contract_with_unknown_type_id() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let unknown_type = crate::ids::TypeId(u32::MAX);
        let function_id = DefId::new(CrateId(0), LocalDefId(191));
        let instance_id = InstanceId(191);
        let extern_instance = InstanceId(192);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "unused_unknown_callable_contract".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(0),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
        let extern_key = crate::mir::MirCallableKey::Instance(extern_instance);
        let mut backend_contract = local_instance_contract(
            &type_context,
            instance_id,
            function_id,
            "unused_unknown_callable_contract",
            Vec::new(),
            i64_type,
            false,
        );
        backend_contract.callables.insert(
            extern_key.clone(),
            crate::mir::MirCallableDecl {
                key: extern_key,
                source_def_id: Some(DefId::new(CrateId(0), LocalDefId(192))),
                kind: crate::mir::MirCallableKind::ObjectProvided,
                llvm_symbol: "unknown_return_extern".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[i64_type],
                    unknown_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract
            .function_bodies
            .insert(mir_id.clone(), caller_key);
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        let err = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            err.message.contains("Invalid MIR backend contract")
                && err.message.contains("unknown return semantic type"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compile_mir_program_rejects_drop_glue_contract_with_wrong_receiver_pass_mode() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let unit_type = type_id(&mut type_context, Type::Unit);
        let struct_id = DefId::new(CrateId(0), LocalDefId(85));
        let box_type_id = type_id(
            &mut type_context,
            Type::Struct {
                id: struct_id,
                args: Vec::new(),
            },
        );
        let function_id = DefId::new(CrateId(0), LocalDefId(86));
        let instance_id = InstanceId(86);
        let drop_instance = InstanceId(87);
        let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
        let mir_function = crate::mir::MirFunction {
            id: mir_id.clone(),
            name: "drop_contract_wrong_receiver".to_string(),
            basic_blocks: vec![
                crate::mir::BasicBlock {
                    statements: vec![],
                    terminator: Some(crate::mir::Terminator::Drop {
                        place: crate::mir::Place {
                            local: crate::mir::Local(1),
                            projection: vec![],
                        },
                        target: crate::mir::BasicBlockId(1),
                    }),
                },
                crate::mir::BasicBlock {
                    statements: vec![crate::mir::StatementData::assign(
                        crate::mir::Place {
                            local: crate::mir::Local(0),
                            projection: vec![],
                        },
                        crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                            crate::mir::Constant::Int(0),
                        )),
                        None,
                    )],
                    terminator: Some(crate::mir::Terminator::Return),
                },
            ],
            local_decls: vec![
                crate::mir::LocalDecl {
                    ty: i64_type,
                    mutability: crate::mir::Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                crate::mir::LocalDecl {
                    ty: box_type_id,
                    mutability: crate::mir::Mutability::Not,
                    name: Some("box_value".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
        let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
        let mut backend_contract = crate::mir::MirBackendContract::default();
        backend_contract.callables.insert(
            caller_key.clone(),
            crate::mir::MirCallableDecl {
                key: caller_key.clone(),
                source_def_id: Some(function_id),
                kind: crate::mir::MirCallableKind::LocalBody {
                    function_id: mir_id.clone(),
                },
                llvm_symbol: "drop_contract_wrong_receiver".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[],
                    i64_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract.callables.insert(
            drop_key.clone(),
            crate::mir::MirCallableDecl {
                key: drop_key.clone(),
                source_def_id: Some(DefId::new(CrateId(0), LocalDefId(87))),
                kind: crate::mir::MirCallableKind::ObjectProvided,
                llvm_symbol: "Box_drop".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature {
                    params: vec![crate::mir::MirParamAbi {
                        semantic_ty: box_type_id,
                        pass_mode: crate::mir::MirPassMode::Pointer,
                    }],
                    ret: crate::mir::MirReturnAbi {
                        semantic_ty: unit_type,
                        abi_ty: unit_type,
                    },
                },
            },
        );
        backend_contract
            .function_bodies
            .insert(mir_id.clone(), caller_key);
        backend_contract.drop_glue.insert(box_type_id, drop_key);
        backend_contract.nominal_layouts.insert(
            struct_id,
            crate::mir::MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("value".to_string(), i64_type)],
                generic_params: Vec::new(),
            },
        );
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(mir_id, mir_function)]),
            type_context,
            backend_contract,
        };

        let err = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            err.message.contains("Invalid MIR backend contract")
                && err.message.contains("receiver pass mode"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compile_mir_program_declares_closure_callable_from_backend_contract() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_type = type_id(&mut type_context, Type::I64);
        let callable_ty = Type::function(vec![Type::I64], Type::I64);
        let callable_type = type_id(&mut type_context, callable_ty.clone());
        let parent_def = DefId::new(CrateId(0), LocalDefId(88));
        let parent_id = crate::mir::MirFunctionId::Function(parent_def);
        let closure_id = crate::mir::MirClosureId {
            owner: parent_id.clone(),
            local_index: 0,
        };
        let closure_function_id = crate::mir::MirFunctionId::Closure(Box::new(closure_id.clone()));
        let parent_capture = crate::mir::MirClosureCapture {
            name: "captured".to_string(),
            local: crate::mir::Local(1),
            kind: crate::mir::MirClosureCaptureKind::ByValue,
            span: None,
        };
        let closure_capture = crate::mir::MirClosureCapture {
            name: "captured".to_string(),
            local: crate::mir::Local(2),
            kind: crate::mir::MirClosureCaptureKind::ByValue,
            span: None,
        };
        let parent = crate::mir::MirFunction {
            id: parent_id.clone(),
            name: "make_contract_closure".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![
                    crate::mir::StatementData::assign(
                        crate::mir::Place {
                            local: crate::mir::Local(1),
                            projection: vec![],
                        },
                        crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                            crate::mir::Constant::Int(5),
                        )),
                        None,
                    ),
                    crate::mir::StatementData::assign(
                        crate::mir::Place {
                            local: crate::mir::Local(0),
                            projection: vec![],
                        },
                        crate::mir::Rvalue::Closure(crate::mir::MirClosure {
                            id: closure_id.clone(),
                            display_name: "make_contract_closure.closure0".to_string(),
                            captures: vec![parent_capture],
                        }),
                        None,
                    ),
                ],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![
                crate::mir::LocalDecl {
                    ty: callable_type,
                    mutability: crate::mir::Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                crate::mir::LocalDecl {
                    ty: i64_type,
                    mutability: crate::mir::Mutability::Not,
                    name: Some("captured".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: callable_type,
            ownership: Default::default(),
        };
        let closure = crate::mir::MirFunction {
            id: closure_function_id.clone(),
            name: "make_contract_closure.closure0".to_string(),
            basic_blocks: vec![crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Copy(crate::mir::Place {
                        local: crate::mir::Local(2),
                        projection: vec![],
                    })),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            }],
            local_decls: vec![
                crate::mir::LocalDecl {
                    ty: i64_type,
                    mutability: crate::mir::Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                crate::mir::LocalDecl {
                    ty: i64_type,
                    mutability: crate::mir::Mutability::Not,
                    name: Some("arg".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
                crate::mir::LocalDecl {
                    ty: i64_type,
                    mutability: crate::mir::Mutability::Not,
                    name: Some("captured".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![closure_capture],
            arg_count: 1,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let parent_key = crate::mir::MirCallableKey::Function(parent_def);
        let closure_key = crate::mir::MirCallableKey::Closure(closure_function_id.clone());
        let mut backend_contract = crate::mir::MirBackendContract::default();
        backend_contract.callables.insert(
            parent_key.clone(),
            crate::mir::MirCallableDecl {
                key: parent_key.clone(),
                source_def_id: Some(parent_def),
                kind: crate::mir::MirCallableKind::LocalBody {
                    function_id: parent_id.clone(),
                },
                llvm_symbol: "make_contract_closure".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[],
                    callable_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract.callables.insert(
            closure_key.clone(),
            crate::mir::MirCallableDecl {
                key: closure_key.clone(),
                source_def_id: None,
                kind: crate::mir::MirCallableKind::LocalBody {
                    function_id: closure_function_id.clone(),
                },
                llvm_symbol: "contract_closure_symbol".to_string(),
                linkage: crate::mir::MirLinkage::Internal,
                signature: crate::mir::MirCallableSignature::from_type_ids(
                    &[i64_type],
                    i64_type,
                    crate::mir::MirPassMode::Direct,
                ),
            },
        );
        backend_contract
            .function_bodies
            .insert(parent_id.clone(), parent_key);
        backend_contract
            .function_bodies
            .insert(closure_function_id.clone(), closure_key);
        backend_contract
            .runtime_requirements
            .insert(crate::mir::MirRuntimeHelper::HeapAlloc);
        let mir = crate::mir::MirProgram {
            functions: BTreeMap::from([(parent_id, parent), (closure_function_id, closure)]),
            type_context,
            backend_contract,
        };

        codegen.compile_program_from_mir(&mir).unwrap();

        let ir = codegen.get_ir();
        assert!(
            ir.contains("define internal i64 @contract_closure_symbol(ptr %0, i64 %1)"),
            "IR was:\n{}",
            ir,
        );
        assert!(!ir.contains("__mir_closure"), "IR was:\n{}", ir);
    }

    #[test]
    fn codegen_source_operation_keeps_exact_origin_and_code() {
        let span = crate::lexer::Span::new(std::path::PathBuf::from("/virtual/main.rk"), 4, 9);
        let diagnostic =
            CodegenError::with_span("operation failed".to_string(), span.clone()).into_diagnostic();

        assert_eq!(diagnostic.code, Some(DiagnosticCode::Codegen));
        assert_eq!(diagnostic.location, DiagnosticLocation::Source(span));
        assert!(diagnostic.primary.is_some());
    }

    #[test]
    fn codegen_source_operation_hides_debug_identity_payloads() {
        let span = crate::lexer::Span::new(std::path::PathBuf::from("/virtual/main.rk"), 4, 9);
        let diagnostic = CodegenError::with_span(
            "MIR callable DefId { local: 1 } uses GenericParamId { index: 0 }".to_string(),
            span.clone(),
        )
        .into_diagnostic();

        assert_eq!(diagnostic.location, DiagnosticLocation::Source(span));
        assert_eq!(
            diagnostic.message,
            "Internal compiler error during MIR code generation"
        );
        assert!(diagnostic.primary.is_some());
    }

    #[test]
    fn codegen_backend_contract_is_internal_and_non_source() {
        let diagnostic = CodegenError::backend_contract("invalid contract").into_diagnostic();

        assert_eq!(diagnostic.code, Some(DiagnosticCode::Internal));
        assert_eq!(diagnostic.location, DiagnosticLocation::Toolchain);
        assert!(diagnostic.primary.is_none());
    }

    #[test]
    fn codegen_output_failure_is_file_classified() {
        let path = std::path::PathBuf::from("/tmp/main.o");
        let diagnostic =
            CodegenError::output("cannot write object", path.clone()).into_diagnostic();

        assert_eq!(diagnostic.code, Some(DiagnosticCode::Codegen));
        assert_eq!(diagnostic.location, DiagnosticLocation::File(path));
        assert!(diagnostic.primary.is_none());
    }

    #[test]
    fn codegen_project_and_artifact_failures_keep_public_location_codes() {
        let project_path = std::path::PathBuf::from("/project/rock.toml");
        let artifact_path = std::path::PathBuf::from("/project/dep.rkca");

        let project =
            CodegenError::project("invalid project", project_path.clone()).into_diagnostic();
        let artifact =
            CodegenError::artifact("invalid artifact", artifact_path.clone()).into_diagnostic();

        assert_eq!(project.code, Some(DiagnosticCode::Project));
        assert_eq!(project.location, DiagnosticLocation::Project(project_path));
        assert_eq!(artifact.code, Some(DiagnosticCode::Artifact));
        assert_eq!(
            artifact.location,
            DiagnosticLocation::Artifact(artifact_path)
        );
    }

    #[test]
    fn llvm_details_are_kept_as_notes_without_a_fabricated_source_span() {
        let diagnostic =
            CodegenError::new("LLVM rejected the module".to_string()).into_diagnostic();

        assert_eq!(diagnostic.code, Some(DiagnosticCode::Codegen));
        assert_eq!(diagnostic.location, DiagnosticLocation::Toolchain);
        assert!(diagnostic.primary.is_none());
        assert_eq!(diagnostic.notes, vec!["LLVM rejected the module"]);
    }

    #[test]
    fn backend_errors_cannot_be_reclassified_as_source_operations() {
        let span = crate::lexer::Span::new(std::path::PathBuf::from("/virtual/main.rk"), 1, 2);
        let diagnostic = CodegenError::new("backend failed".to_string())
            .with_operation_span(Some(span))
            .into_diagnostic();

        assert_eq!(diagnostic.location, DiagnosticLocation::Toolchain);
        assert!(diagnostic.primary.is_none());
    }
}
