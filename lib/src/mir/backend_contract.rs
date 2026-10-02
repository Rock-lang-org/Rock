use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::ids::{AssocTypeId, DefId, InstanceId, TypeId};
use crate::type_context::TypeContext;
use crate::type_services::projection::{ProjectionImpl, ProjectionNormalizer, ProjectionProvider};
use crate::types::{GenericParamId, Type};

use super::{
    Constant, MirAssertKind, MirCallable, MirFunction, MirFunctionId, MirIntrinsicId,
    MirRuntimeHelper, Operand, Place, Projection, Rvalue, StatementKind, Terminator,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MirCallableKey {
    ObjectAdapter(super::MirObjectAdapterKey),
    Function(DefId),
    Extern(DefId),
    Instance(InstanceId),
    Closure(MirFunctionId),
    Intrinsic(MirIntrinsicId),
    RuntimeHelper(MirRuntimeHelper),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirLinkage {
    External,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum MirPassMode {
    Direct,
    Pointer,
    FatDirect,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MirParamAbi {
    pub semantic_ty: TypeId,
    pub pass_mode: MirPassMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MirReturnAbi {
    pub semantic_ty: TypeId,
    pub abi_ty: TypeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MirCallableSignature {
    pub params: Vec<MirParamAbi>,
    pub ret: MirReturnAbi,
}

impl MirCallableSignature {
    pub fn from_type_ids(params: &[TypeId], ret: TypeId, pass_mode: MirPassMode) -> Self {
        Self {
            params: params
                .iter()
                .copied()
                .map(|semantic_ty| MirParamAbi {
                    semantic_ty,
                    pass_mode,
                })
                .collect(),
            ret: MirReturnAbi {
                semantic_ty: ret,
                abi_ty: ret,
            },
        }
    }

    pub fn from_instance_type_ids(
        params: &[TypeId],
        ret: TypeId,
        is_method: bool,
        type_context: &TypeContext,
    ) -> Self {
        let mut signature = Self::from_type_ids(params, ret, MirPassMode::Direct);
        if is_method {
            if let Some(receiver) = signature.params.first_mut() {
                receiver.pass_mode = receiver_pass_mode(receiver.semantic_ty, type_context);
            }
        }
        signature
    }
}

fn receiver_pass_mode(receiver_ty: TypeId, type_context: &TypeContext) -> MirPassMode {
    let Some(receiver_ty) = type_context
        .contains_type_id(receiver_ty)
        .then(|| type_context.type_for(receiver_ty))
    else {
        return MirPassMode::Direct;
    };

    if !matches!(receiver_ty, Type::Reference { .. }) {
        return MirPassMode::Direct;
    }

    if crate::type_services::layout::TypeLayout::is_slice_shape(&receiver_ty)
        || crate::type_services::layout::TypeLayout::is_fat_pointer_shape(&receiver_ty)
    {
        MirPassMode::FatDirect
    } else {
        MirPassMode::Pointer
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirCallableKind {
    ObjectAdapter(super::MirObjectAdapter),
    LocalBody { function_id: MirFunctionId },
    Extern { link_name: String, variadic: bool },
    ObjectProvided,
    Intrinsic(MirIntrinsicId),
    RuntimeHelper(MirRuntimeHelper),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MirCallableDecl {
    pub key: MirCallableKey,
    pub source_def_id: Option<DefId>,
    pub kind: MirCallableKind,
    pub llvm_symbol: String,
    pub linkage: MirLinkage,
    pub signature: MirCallableSignature,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirProjectionKey {
    pub base: TypeId,
    pub trait_id: DefId,
    pub assoc_type_id: AssocTypeId,
    pub trait_args: Vec<TypeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirNominalLayout {
    Struct {
        id: DefId,
        fields: Vec<(String, TypeId)>,
        generic_params: Vec<GenericParamId>,
    },
    Enum {
        id: DefId,
        variants: Vec<crate::mir::MirEnumVariantLayout>,
        generic_params: Vec<GenericParamId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MirArtifactExport {
    pub origin_def_id: Option<DefId>,
    pub source_name: String,
    pub backend_symbol: String,
    pub substitution_empty: bool,
    pub has_body: bool,
    pub provided_by_object: bool,
    pub is_specialization: bool,
    pub is_drop_glue: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirBackendContract {
    pub owned_object_calls: BTreeMap<super::MirOwnedObjectKey, super::MirOwnedObjectPlan>,
    pub erased: super::MirErasedContract,
    pub object_schemas: BTreeMap<TypeId, super::MirObjectSchema>,
    pub vtables: BTreeMap<super::MirVtableId, super::MirVtable>,
    pub callables: BTreeMap<MirCallableKey, MirCallableDecl>,
    pub function_bodies: BTreeMap<MirFunctionId, MirCallableKey>,
    pub nominal_layouts: BTreeMap<DefId, MirNominalLayout>,
    pub projection_outputs: BTreeMap<MirProjectionKey, TypeId>,
    pub projection_traits: BTreeSet<DefId>,
    pub drop_glue: BTreeMap<TypeId, MirCallableKey>,
    pub runtime_requirements: BTreeSet<MirRuntimeHelper>,
    pub artifact_exports: Vec<MirArtifactExport>,
}

impl MirBackendContract {
    pub fn callable(&self, key: &MirCallableKey) -> Option<&MirCallableDecl> {
        self.callables.get(key)
    }

    pub fn normalize_type(&self, type_context: &TypeContext, ty: &Type) -> Type {
        let ty = type_context
            .normalize_type(ty)
            .expect("MIR backend contract requires canonical kind-correct types");
        let provider = MirBackendContractProjectionProvider {
            contract: self,
            type_context,
        };
        let projected = ProjectionNormalizer::normalize(&provider, &ty);
        type_context
            .normalize_type(&projected)
            .expect("projection normalization must preserve kind correctness")
    }

    pub fn normalize_type_id(&self, type_context: &TypeContext, ty: TypeId) -> Option<TypeId> {
        if !type_context.type_id_tree_is_valid(ty) {
            return None;
        }
        let raw = type_context.type_for(ty);
        let normalized = self.normalize_type(type_context, &raw);
        if normalized == raw {
            Some(ty)
        } else {
            type_context.id_for_type(&normalized)
        }
    }

    pub fn place_type_id(
        &self,
        type_context: &TypeContext,
        function: &MirFunction,
        place: &Place,
    ) -> Option<TypeId> {
        let local = function.local_decls.get(place.local.0)?;
        if !type_context.type_id_tree_is_valid(local.ty) {
            return None;
        }
        let raw_local_ty = type_context.type_for(local.ty);
        let mut current_ty = self.normalize_type(type_context, &raw_local_ty);
        if place.projection.is_empty() {
            return if current_ty == raw_local_ty {
                Some(local.ty)
            } else {
                type_context.id_for_type(&current_ty)
            };
        }

        let mut downcast_variant = None;
        for projection in &place.projection {
            current_ty = self.normalize_type(type_context, &current_ty);
            current_ty = match projection {
                Projection::Deref => match &current_ty {
                    Type::Pointer(inner) | Type::Reference { inner, .. } => inner.as_ref().clone(),
                    _ => return None,
                },
                Projection::Field { index, .. } => self.project_field_type(
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
            current_ty = self.normalize_type(type_context, &current_ty);
        }

        type_context.id_for_type(&current_ty)
    }

    fn project_field_type(
        &self,
        type_context: &TypeContext,
        ty: &Type,
        index: usize,
        downcast_variant: Option<crate::ids::VariantId>,
    ) -> Option<Type> {
        let ty = self.normalize_type(type_context, ty);
        let projected = match &ty {
            Type::Tuple(fields) => fields.get(index).cloned(),
            Type::Struct { id, args } => {
                let MirNominalLayout::Struct {
                    fields,
                    generic_params,
                    ..
                } = self.nominal_layouts.get(id)?
                else {
                    return None;
                };
                let field_ty = fields.get(index).map(|(_, ty)| *ty)?;
                if !type_context.type_id_tree_is_valid(field_ty) {
                    return None;
                }
                let substitution = generic_params
                    .iter()
                    .copied()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                Some(
                    type_context
                        .type_for(field_ty)
                        .substitute_generics(&substitution),
                )
            }
            Type::Enum { id, args } => {
                let variant_id = downcast_variant?;
                let MirNominalLayout::Enum {
                    variants,
                    generic_params,
                    ..
                } = self.nominal_layouts.get(id)?
                else {
                    return None;
                };
                let variant = variants.get(variant_id.0 as usize)?;
                let field_ty = match &variant.fields {
                    crate::mir::MirVariantLayoutFields::Unit => return None,
                    crate::mir::MirVariantLayoutFields::Positional(fields) => *fields.get(index)?,
                    crate::mir::MirVariantLayoutFields::Named(fields) => fields.get(index)?.1,
                };
                if !type_context.type_id_tree_is_valid(field_ty) {
                    return None;
                }
                let substitution = generic_params
                    .iter()
                    .copied()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                Some(
                    type_context
                        .type_for(field_ty)
                        .substitute_generics(&substitution),
                )
            }
            _ => None,
        }?;
        Some(self.normalize_type(type_context, &projected))
    }
}

pub fn runtime_requirements_for_functions<'a>(
    functions: impl IntoIterator<Item = &'a MirFunction>,
) -> BTreeSet<MirRuntimeHelper> {
    let mut requirements = BTreeSet::new();
    for function in functions {
        for block in &function.basic_blocks {
            for statement in &block.statements {
                match &statement.kind {
                    StatementKind::Assert(assertion)
                        if assertion.kind == MirAssertKind::BoundsCheck
                            && assertion.operands.len() == 2 =>
                    {
                        requirements.insert(MirRuntimeHelper::BoundsCheck);
                    }
                    StatementKind::Assign(_, Rvalue::Closure(closure))
                        if !closure.captures.is_empty() =>
                    {
                        requirements.insert(MirRuntimeHelper::HeapAlloc);
                    }
                    StatementKind::Assign(_, _)
                    | StatementKind::Assert(_)
                    | StatementKind::StorageLive(_)
                    | StatementKind::StorageDead(_) => {}
                }
            }
        }
    }
    requirements
}

/// Runtime requirements of both ordinary MIR and executable contract adapters.
pub fn runtime_requirements_for_backend<'a>(
    contract: &MirBackendContract,
    functions: impl IntoIterator<Item = &'a MirFunction>,
) -> BTreeSet<MirRuntimeHelper> {
    let mut requirements = runtime_requirements_for_functions(functions);
    if contract
        .erased
        .descriptors
        .values()
        .any(|descriptor| descriptor.drop.requires_heap_free())
    {
        requirements.insert(MirRuntimeHelper::HeapFree);
    }
    for declaration in contract.callables.values() {
        let needs_free = match &declaration.kind {
            MirCallableKind::ObjectAdapter(super::MirObjectAdapter::PayloadDrop(plan)) => {
                plan.requires_heap_free()
            }
            MirCallableKind::ObjectAdapter(super::MirObjectAdapter::NativeCallable {
                release_environment,
                ..
            }) => *release_environment,
            _ => false,
        };
        if needs_free {
            requirements.insert(MirRuntimeHelper::HeapFree);
        }
    }
    requirements
}

struct MirBackendContractProjectionProvider<'a> {
    contract: &'a MirBackendContract,
    type_context: &'a TypeContext,
}

impl ProjectionProvider for MirBackendContractProjectionProvider<'_> {
    fn resolve_projection_output(
        &self,
        base_ty: &Type,
        trait_id: DefId,
        assoc_type_id: AssocTypeId,
        trait_args: &[Type],
    ) -> Option<Type> {
        let base = self.type_context.id_for_type(base_ty)?;
        let trait_args = trait_args
            .iter()
            .map(|arg| self.type_context.id_for_type(arg))
            .collect::<Option<Vec<_>>>()?;
        let output = self
            .contract
            .projection_outputs
            .get(&MirProjectionKey {
                base,
                trait_id,
                assoc_type_id,
                trait_args,
            })
            .copied()?;
        self.type_context
            .type_id_tree_is_valid(output)
            .then(|| self.type_context.type_for(output))
    }

    fn find_projection_impl(
        &self,
        _base_ty: &Type,
        _trait_id: DefId,
        _trait_args: &[Type],
    ) -> Option<ProjectionImpl> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirBackendContractError {
    CallableKeyMismatch {
        map_key: MirCallableKey,
        declaration_key: MirCallableKey,
    },
    BodyWithoutCallable {
        function: MirFunctionId,
        key: MirCallableKey,
    },
    CallableWithoutBody {
        key: MirCallableKey,
        function: MirFunctionId,
    },
    CallableBodyMissingFunction {
        key: MirCallableKey,
        function: MirFunctionId,
    },
    FunctionWithoutCallable {
        function: MirFunctionId,
    },
    ResolvedCallableWithoutContract {
        function: MirFunctionId,
        key: MirCallableKey,
    },
    LocalBodyParamCountMismatch {
        function: MirFunctionId,
        signature_params: usize,
        body_params: usize,
    },
    LocalBodyParamLocalMissing {
        function: MirFunctionId,
        param_index: usize,
        local_index: usize,
    },
    LocalBodyParamTypeMismatch {
        function: MirFunctionId,
        param_index: usize,
        signature_ty: TypeId,
        local_ty: TypeId,
    },
    LocalBodyReturnTypeMismatch {
        function: MirFunctionId,
        signature_ty: TypeId,
        body_ty: TypeId,
    },
    LocalBodyReturnAbiTypeMismatch {
        function: MirFunctionId,
        signature_ty: TypeId,
        body_ty: TypeId,
    },
    LocalBodyReturnLocalMissing {
        function: MirFunctionId,
    },
    LocalBodyReturnLocalTypeMismatch {
        function: MirFunctionId,
        local_ty: TypeId,
        body_ty: TypeId,
    },
    DropGlueCallableKindMismatch {
        dropped_ty: TypeId,
        key: MirCallableKey,
        kind: MirCallableKind,
    },
    DropGlueCallableMissing {
        dropped_ty: TypeId,
        key: MirCallableKey,
    },
}

pub fn validate_backend_contract(contract: &MirBackendContract) -> Vec<MirBackendContractError> {
    let mut errors = Vec::new();

    for (key, callable) in &contract.callables {
        if &callable.key != key {
            errors.push(MirBackendContractError::CallableKeyMismatch {
                map_key: key.clone(),
                declaration_key: callable.key.clone(),
            });
        }
    }

    for (function, key) in &contract.function_bodies {
        let Some(callable) = contract.callable(key) else {
            errors.push(MirBackendContractError::BodyWithoutCallable {
                function: function.clone(),
                key: key.clone(),
            });
            continue;
        };

        if !matches!(
            &callable.kind,
            MirCallableKind::LocalBody { function_id } if function_id == function
        ) {
            errors.push(MirBackendContractError::BodyWithoutCallable {
                function: function.clone(),
                key: key.clone(),
            });
        }
    }

    for (key, callable) in &contract.callables {
        let MirCallableKind::LocalBody { function_id } = &callable.kind else {
            continue;
        };
        if contract.function_bodies.get(function_id) != Some(key) {
            errors.push(MirBackendContractError::CallableWithoutBody {
                key: key.clone(),
                function: function_id.clone(),
            });
        }
    }

    for (dropped_ty, key) in &contract.drop_glue {
        let Some(callable) = contract.callable(key) else {
            errors.push(MirBackendContractError::DropGlueCallableMissing {
                dropped_ty: *dropped_ty,
                key: key.clone(),
            });
            continue;
        };
        if !matches!(
            callable.kind,
            MirCallableKind::LocalBody { .. }
                | MirCallableKind::Extern { .. }
                | MirCallableKind::ObjectProvided
        ) {
            errors.push(MirBackendContractError::DropGlueCallableKindMismatch {
                dropped_ty: *dropped_ty,
                key: key.clone(),
                kind: callable.kind.clone(),
            });
        }
    }

    errors
}

pub fn validate_backend_contract_against_functions<'a, I>(
    contract: &MirBackendContract,
    functions: I,
) -> Vec<MirBackendContractError>
where
    I: IntoIterator<Item = &'a MirFunctionId>,
{
    let mut errors = validate_backend_contract(contract);
    let functions = functions.into_iter().cloned().collect::<BTreeSet<_>>();

    for (key, callable) in &contract.callables {
        let MirCallableKind::LocalBody { function_id } = &callable.kind else {
            continue;
        };
        if !functions.contains(function_id) {
            errors.push(MirBackendContractError::CallableBodyMissingFunction {
                key: key.clone(),
                function: function_id.clone(),
            });
        }
    }

    for function in functions {
        if !contract.function_bodies.contains_key(&function) {
            errors.push(MirBackendContractError::FunctionWithoutCallable { function });
        }
    }

    errors
}

pub fn validate_backend_contract_against_mir<'a, I>(
    contract: &MirBackendContract,
    functions: I,
) -> Vec<MirBackendContractError>
where
    I: IntoIterator<Item = &'a MirFunction>,
{
    let functions = functions.into_iter().collect::<Vec<_>>();
    let mut errors = validate_backend_contract_against_functions(
        contract,
        functions.iter().map(|function| &function.id),
    );
    let mut seen_resolved = BTreeSet::new();

    for function in functions {
        validate_local_body_signature(contract, function, &mut errors);
        validate_function_callable_operands(contract, function, &mut seen_resolved, &mut errors);
    }

    errors
}

fn validate_local_body_signature(
    contract: &MirBackendContract,
    function: &MirFunction,
    errors: &mut Vec<MirBackendContractError>,
) {
    let Some(key) = contract.function_bodies.get(&function.id) else {
        return;
    };
    let Some(callable) = contract.callable(key) else {
        return;
    };
    if !matches!(
        &callable.kind,
        MirCallableKind::LocalBody { function_id } if function_id == &function.id
    ) {
        return;
    }

    if callable.signature.params.len() != function.arg_count {
        errors.push(MirBackendContractError::LocalBodyParamCountMismatch {
            function: function.id.clone(),
            signature_params: callable.signature.params.len(),
            body_params: function.arg_count,
        });
    }

    for param_index in 0..function.arg_count.min(callable.signature.params.len()) {
        let local_index = param_index + 1;
        let Some(local) = function.local_decls.get(local_index) else {
            errors.push(MirBackendContractError::LocalBodyParamLocalMissing {
                function: function.id.clone(),
                param_index,
                local_index,
            });
            continue;
        };
        let signature_ty = callable.signature.params[param_index].semantic_ty;
        if signature_ty != local.ty {
            errors.push(MirBackendContractError::LocalBodyParamTypeMismatch {
                function: function.id.clone(),
                param_index,
                signature_ty,
                local_ty: local.ty,
            });
        }
    }

    if callable.signature.ret.semantic_ty != function.ret_type {
        errors.push(MirBackendContractError::LocalBodyReturnTypeMismatch {
            function: function.id.clone(),
            signature_ty: callable.signature.ret.semantic_ty,
            body_ty: function.ret_type,
        });
    }
    if callable.signature.ret.abi_ty != function.ret_type
        && !local_body_allows_distinct_return_abi(function, callable)
    {
        errors.push(MirBackendContractError::LocalBodyReturnAbiTypeMismatch {
            function: function.id.clone(),
            signature_ty: callable.signature.ret.abi_ty,
            body_ty: function.ret_type,
        });
    }

    let Some(return_local) = function.local_decls.first() else {
        errors.push(MirBackendContractError::LocalBodyReturnLocalMissing {
            function: function.id.clone(),
        });
        return;
    };
    if return_local.ty != function.ret_type {
        errors.push(MirBackendContractError::LocalBodyReturnLocalTypeMismatch {
            function: function.id.clone(),
            local_ty: return_local.ty,
            body_ty: function.ret_type,
        });
    }
}

fn local_body_allows_distinct_return_abi(
    function: &MirFunction,
    callable: &MirCallableDecl,
) -> bool {
    function.name == "main" && callable.llvm_symbol == "main"
}

fn validate_function_callable_operands(
    contract: &MirBackendContract,
    function: &MirFunction,
    seen_resolved: &mut BTreeSet<(MirFunctionId, MirCallableKey)>,
    errors: &mut Vec<MirBackendContractError>,
) {
    for block in &function.basic_blocks {
        for statement in &block.statements {
            match &statement.kind {
                StatementKind::Assign(_, rvalue) => validate_rvalue_callable_operands(
                    contract,
                    function,
                    rvalue,
                    seen_resolved,
                    errors,
                ),
                StatementKind::Assert(assertion) => {
                    for operand in &assertion.operands {
                        validate_operand_callable_contract(
                            contract,
                            function,
                            operand,
                            seen_resolved,
                            errors,
                        );
                    }
                }
                StatementKind::StorageLive(_) | StatementKind::StorageDead(_) => {}
            }
        }

        let Some(terminator) = &block.terminator else {
            continue;
        };
        match terminator {
            Terminator::Call { func, args, .. } => {
                validate_operand_callable_contract(contract, function, func, seen_resolved, errors);
                for arg in args {
                    validate_operand_callable_contract(
                        contract,
                        function,
                        arg,
                        seen_resolved,
                        errors,
                    );
                }
            }
            Terminator::SwitchInt { discr, .. } | Terminator::SwitchIntWithOrigin { discr, .. } => {
                validate_operand_callable_contract(contract, function, discr, seen_resolved, errors)
            }
            Terminator::Return
            | Terminator::Unreachable { .. }
            | Terminator::ReturnWithOrigin { .. }
            | Terminator::Goto(_)
            | Terminator::GotoWithOrigin { .. }
            | Terminator::Drop { .. }
            | Terminator::DropWithOrigin { .. } => {}
        }
    }
}

fn validate_rvalue_callable_operands(
    contract: &MirBackendContract,
    function: &MirFunction,
    rvalue: &Rvalue,
    seen_resolved: &mut BTreeSet<(MirFunctionId, MirCallableKey)>,
    errors: &mut Vec<MirBackendContractError>,
) {
    match rvalue {
        Rvalue::Object(operand, _)
        | Rvalue::Use(operand)
        | Rvalue::Cast(operand, _)
        | Rvalue::UnaryOp(_, operand) => {
            validate_operand_callable_contract(contract, function, operand, seen_resolved, errors)
        }
        Rvalue::BinaryOp(_, left, right) => {
            validate_operand_callable_contract(contract, function, left, seen_resolved, errors);
            validate_operand_callable_contract(contract, function, right, seen_resolved, errors);
        }
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                validate_operand_callable_contract(
                    contract,
                    function,
                    operand,
                    seen_resolved,
                    errors,
                );
            }
        }
        Rvalue::Ref(_, _) | Rvalue::Closure(_) | Rvalue::Discriminant(_) => {}
    }
}

fn validate_operand_callable_contract(
    contract: &MirBackendContract,
    function: &MirFunction,
    operand: &Operand,
    seen_resolved: &mut BTreeSet<(MirFunctionId, MirCallableKey)>,
    errors: &mut Vec<MirBackendContractError>,
) {
    let Operand::Constant(Constant::Callable(callable)) = operand else {
        return;
    };

    let MirCallable::Resolved(key) = callable;
    if contract.callables.contains_key(key) {
        return;
    }
    let missing = (function.id.clone(), key.clone());
    if seen_resolved.insert(missing.clone()) {
        errors.push(MirBackendContractError::ResolvedCallableWithoutContract {
            function: missing.0,
            key: missing.1,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{CrateId, DefId, InstanceId, LocalDefId, TypeId};
    use crate::mir::{
        BasicBlock, BasicBlockId, Constant, Local, LocalDecl, MirAssert, MirAssertKind,
        MirCallable, MirFunction, MirFunctionId, Mutability, Operand, Place, StatementData,
        Terminator,
    };

    fn def_id(local: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(local))
    }

    #[test]
    fn contract_declares_callable_by_exact_key() {
        let key = MirCallableKey::Function(def_id(1));
        let declaration = MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(def_id(1)),
            kind: MirCallableKind::LocalBody {
                function_id: MirFunctionId::Function(def_id(1)),
            },
            llvm_symbol: "main".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(
                &[TypeId(1)],
                TypeId(2),
                MirPassMode::Direct,
            ),
        };
        let mut contract = MirBackendContract::default();
        contract.callables.insert(key.clone(), declaration);

        assert_eq!(contract.callable(&key).map(|decl| &decl.key), Some(&key));
        assert!(contract
            .callable(&MirCallableKey::Function(def_id(2)))
            .is_none());
    }

    #[test]
    fn runtime_requirements_ignore_malformed_bounds_checks() {
        let function = MirFunction {
            id: MirFunctionId::Function(def_id(100)),
            name: "malformed_bounds".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![Operand::Constant(Constant::Int(0))],
                    },
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: TypeId(0),
            ownership: Default::default(),
        };

        assert!(runtime_requirements_for_functions([&function]).is_empty());
    }

    #[test]
    fn runtime_requirements_scan_later_blocks() {
        let function = MirFunction {
            id: MirFunctionId::Function(def_id(101)),
            name: "later_bounds".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![],
                    terminator: Some(Terminator::Goto(BasicBlockId(1))),
                },
                BasicBlock {
                    statements: vec![StatementData::assert(
                        MirAssert {
                            kind: MirAssertKind::BoundsCheck,
                            operands: vec![
                                Operand::Constant(Constant::Int(0)),
                                Operand::Constant(Constant::Int(0)),
                            ],
                        },
                        None,
                    )],
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: TypeId(0),
            ownership: Default::default(),
        };

        assert_eq!(
            runtime_requirements_for_functions([&function]),
            BTreeSet::from([MirRuntimeHelper::BoundsCheck])
        );
    }

    #[test]
    fn validation_reports_callable_map_key_mismatch() {
        let map_key = MirCallableKey::Function(def_id(1));
        let declaration_key = MirCallableKey::Function(def_id(2));
        let mut contract = MirBackendContract::default();
        contract.callables.insert(
            map_key.clone(),
            MirCallableDecl {
                key: declaration_key.clone(),
                source_def_id: Some(def_id(2)),
                kind: MirCallableKind::Extern {
                    link_name: "ffi".to_string(),
                    variadic: false,
                },
                llvm_symbol: "ffi".to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
            },
        );

        assert_eq!(
            validate_backend_contract(&contract),
            vec![MirBackendContractError::CallableKeyMismatch {
                map_key,
                declaration_key,
            }],
        );
    }

    #[test]
    fn validation_reports_local_callable_without_body() {
        let key = MirCallableKey::Function(def_id(1));
        let function = MirFunctionId::Function(def_id(1));
        let declaration = MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(def_id(1)),
            kind: MirCallableKind::LocalBody {
                function_id: function.clone(),
            },
            llvm_symbol: "main".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
        };
        let mut contract = MirBackendContract::default();
        contract.callables.insert(key.clone(), declaration);
        contract
            .function_bodies
            .insert(function.clone(), key.clone());
        contract.function_bodies.remove(&function);

        assert_eq!(
            validate_backend_contract(&contract),
            vec![MirBackendContractError::CallableWithoutBody { key, function }],
        );
    }

    #[test]
    fn method_receiver_abi_is_part_of_callable_signature() {
        let mut type_context = TypeContext::new();
        let receiver = type_context.intern_type(&Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        });
        let ret = type_context.intern_type(&Type::Unit);
        let signature =
            MirCallableSignature::from_instance_type_ids(&[receiver], ret, true, &type_context);

        assert_eq!(signature.params[0].semantic_ty, receiver);
        assert_eq!(signature.params[0].pass_mode, MirPassMode::Pointer);
    }

    #[test]
    fn contract_intrinsic_identity_is_typed_not_string_backed() {
        fn callable_key_intrinsic_ctor(_: fn(MirIntrinsicId) -> MirCallableKey) {}
        fn callable_kind_intrinsic_ctor(_: fn(MirIntrinsicId) -> MirCallableKind) {}

        callable_key_intrinsic_ctor(MirCallableKey::Intrinsic);
        callable_kind_intrinsic_ctor(MirCallableKind::Intrinsic);
    }

    #[test]
    fn validation_reports_local_body_missing_from_program_functions() {
        let key = MirCallableKey::Function(def_id(1));
        let function = MirFunctionId::Function(def_id(1));
        let mut contract = MirBackendContract::default();
        contract.callables.insert(
            key.clone(),
            MirCallableDecl {
                key: key.clone(),
                source_def_id: Some(def_id(1)),
                kind: MirCallableKind::LocalBody {
                    function_id: function.clone(),
                },
                llvm_symbol: "main".to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
            },
        );
        contract
            .function_bodies
            .insert(function.clone(), key.clone());

        assert_eq!(
            validate_backend_contract_against_functions(&contract, std::iter::empty()),
            vec![MirBackendContractError::CallableBodyMissingFunction { key, function }],
        );
    }

    #[test]
    fn validation_reports_local_body_signature_mismatch() {
        let key = MirCallableKey::Function(def_id(1));
        let function_id = MirFunctionId::Function(def_id(1));
        let mut contract = MirBackendContract::default();
        contract.callables.insert(
            key.clone(),
            MirCallableDecl {
                key: key.clone(),
                source_def_id: Some(def_id(1)),
                kind: MirCallableKind::LocalBody {
                    function_id: function_id.clone(),
                },
                llvm_symbol: "body".to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(
                    &[TypeId(1)],
                    TypeId(3),
                    MirPassMode::Direct,
                ),
            },
        );
        contract
            .function_bodies
            .insert(function_id.clone(), key.clone());
        let function = MirFunction {
            id: function_id,
            name: "body".to_string(),
            basic_blocks: vec![],
            local_decls: vec![
                LocalDecl {
                    ty: TypeId(2),
                    mutability: Mutability::Not,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: TypeId(2),
                    mutability: Mutability::Not,
                    name: Some("arg".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Argument,
                },
            ],
            closure_captures: vec![],
            arg_count: 1,
            ret_type: TypeId(2),
            ownership: Default::default(),
        };

        let messages = validate_backend_contract_against_mir(&contract, [&function])
            .into_iter()
            .map(|error| format!("{:?}", error))
            .collect::<Vec<_>>();

        assert!(
            messages.iter().any(|message| {
                message.contains("LocalBodyParamTypeMismatch")
                    && message.contains("TypeId(1)")
                    && message.contains("TypeId(2)")
            }),
            "expected parameter type mismatch, got {messages:?}"
        );
        assert!(
            messages.iter().any(|message| {
                message.contains("LocalBodyReturnTypeMismatch")
                    && message.contains("TypeId(3)")
                    && message.contains("TypeId(2)")
            }),
            "expected return type mismatch, got {messages:?}"
        );
    }

    #[test]
    fn validation_reports_local_body_return_abi_mismatch() {
        let key = MirCallableKey::Function(def_id(1));
        let function_id = MirFunctionId::Function(def_id(1));
        let mut contract = MirBackendContract::default();
        contract.callables.insert(
            key.clone(),
            MirCallableDecl {
                key: key.clone(),
                source_def_id: Some(def_id(1)),
                kind: MirCallableKind::LocalBody {
                    function_id: function_id.clone(),
                },
                llvm_symbol: "body".to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature {
                    params: vec![],
                    ret: MirReturnAbi {
                        semantic_ty: TypeId(2),
                        abi_ty: TypeId(3),
                    },
                },
            },
        );
        contract
            .function_bodies
            .insert(function_id.clone(), key.clone());
        let function = MirFunction {
            id: function_id,
            name: "body".to_string(),
            basic_blocks: vec![],
            local_decls: vec![LocalDecl {
                ty: TypeId(2),
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: TypeId(2),
            ownership: Default::default(),
        };

        let messages = validate_backend_contract_against_mir(&contract, [&function])
            .into_iter()
            .map(|error| format!("{:?}", error))
            .collect::<Vec<_>>();

        assert!(
            messages.iter().any(|message| {
                message.contains("LocalBodyReturnAbiTypeMismatch")
                    && message.contains("TypeId(3)")
                    && message.contains("TypeId(2)")
            }),
            "expected return ABI mismatch, got {messages:?}"
        );
    }

    #[test]
    fn validation_reports_local_body_return_local_mismatch() {
        let key = MirCallableKey::Function(def_id(1));
        let function_id = MirFunctionId::Function(def_id(1));
        let mut contract = MirBackendContract::default();
        contract.callables.insert(
            key.clone(),
            MirCallableDecl {
                key: key.clone(),
                source_def_id: Some(def_id(1)),
                kind: MirCallableKind::LocalBody {
                    function_id: function_id.clone(),
                },
                llvm_symbol: "body".to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(&[], TypeId(2), MirPassMode::Direct),
            },
        );
        contract
            .function_bodies
            .insert(function_id.clone(), key.clone());
        let function = MirFunction {
            id: function_id,
            name: "body".to_string(),
            basic_blocks: vec![],
            local_decls: vec![LocalDecl {
                ty: TypeId(3),
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: TypeId(2),
            ownership: Default::default(),
        };

        let messages = validate_backend_contract_against_mir(&contract, [&function])
            .into_iter()
            .map(|error| format!("{:?}", error))
            .collect::<Vec<_>>();

        assert!(
            messages.iter().any(|message| {
                message.contains("LocalBodyReturnLocalTypeMismatch")
                    && message.contains("TypeId(3)")
                    && message.contains("TypeId(2)")
            }),
            "expected return local type mismatch, got {messages:?}"
        );
    }

    #[test]
    fn validation_reports_drop_glue_callable_that_codegen_will_not_declare() {
        let dropped_ty = TypeId(1);
        let key = MirCallableKey::Instance(InstanceId(9));
        let mut contract = MirBackendContract::default();
        contract.callables.insert(
            key.clone(),
            MirCallableDecl {
                key: key.clone(),
                source_def_id: None,
                kind: MirCallableKind::RuntimeHelper(MirRuntimeHelper::DropGlue),
                llvm_symbol: "Box_drop".to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(
                    &[dropped_ty],
                    TypeId(0),
                    MirPassMode::Direct,
                ),
            },
        );
        contract.drop_glue.insert(dropped_ty, key);

        let messages = validate_backend_contract(&contract)
            .into_iter()
            .map(|error| format!("{:?}", error))
            .collect::<Vec<_>>();

        assert!(
            messages
                .iter()
                .any(|message| message.contains("DropGlueCallableKindMismatch")),
            "expected drop-glue kind mismatch, got {messages:?}"
        );
    }

    #[test]
    fn validation_reports_drop_glue_missing_callable_key() {
        let dropped_ty = TypeId(1);
        let key = MirCallableKey::Instance(InstanceId(404));
        let mut contract = MirBackendContract::default();
        contract.drop_glue.insert(dropped_ty, key);

        let messages = validate_backend_contract(&contract)
            .into_iter()
            .map(|error| format!("{:?}", error))
            .collect::<Vec<_>>();

        assert!(
            messages
                .iter()
                .any(|message| message.contains("DropGlueCallableMissing")),
            "expected missing drop-glue callable, got {messages:?}"
        );
    }

    #[test]
    fn validation_reports_program_function_without_callable_mapping_when_contract_is_non_empty() {
        let function = MirFunctionId::Function(def_id(1));
        let mut contract = MirBackendContract::default();
        let ext_key = MirCallableKey::Extern(def_id(2));
        contract.callables.insert(
            ext_key.clone(),
            MirCallableDecl {
                key: ext_key,
                source_def_id: Some(def_id(2)),
                kind: MirCallableKind::Extern {
                    link_name: "ffi".to_string(),
                    variadic: false,
                },
                llvm_symbol: "ffi".to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
            },
        );

        assert_eq!(
            validate_backend_contract_against_functions(&contract, [&function]),
            vec![MirBackendContractError::FunctionWithoutCallable { function }],
        );
    }

    #[test]
    fn validation_reports_resolved_callable_key_used_by_body_without_contract_entry() {
        let caller = def_id(3);
        let function_id = MirFunctionId::Function(caller);
        let missing_key = MirCallableKey::Instance(InstanceId(404));
        let mut contract = MirBackendContract::default();
        let caller_key = MirCallableKey::Function(caller);
        contract.callables.insert(
            caller_key.clone(),
            MirCallableDecl {
                key: caller_key.clone(),
                source_def_id: Some(caller),
                kind: MirCallableKind::LocalBody {
                    function_id: function_id.clone(),
                },
                llvm_symbol: "main".to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
            },
        );
        contract
            .function_bodies
            .insert(function_id.clone(), caller_key);
        let function = MirFunction {
            id: function_id.clone(),
            name: "main".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![],
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        missing_key.clone(),
                    ))),
                    args: vec![],
                    destination: Place {
                        local: Local(0),
                        projection: vec![],
                    },
                    target: BasicBlockId(0),
                    span: None,
                }),
            }],
            local_decls: vec![LocalDecl {
                ty: TypeId(0),
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: TypeId(0),
            ownership: Default::default(),
        };

        assert_eq!(
            validate_backend_contract_against_mir(&contract, [&function]),
            vec![MirBackendContractError::ResolvedCallableWithoutContract {
                function: function_id,
                key: missing_key,
            }],
        );
    }

    #[test]
    fn validation_rejects_callable_key_not_in_contract_even_when_source_alias_matches() {
        let caller = def_id(4);
        let callee = def_id(5);
        let function_id = MirFunctionId::Function(caller);
        let caller_key = MirCallableKey::Function(caller);
        let callee_key = MirCallableKey::Instance(InstanceId(405));
        let mut contract = MirBackendContract::default();
        contract.callables.insert(
            caller_key.clone(),
            MirCallableDecl {
                key: caller_key.clone(),
                source_def_id: Some(caller),
                kind: MirCallableKind::LocalBody {
                    function_id: function_id.clone(),
                },
                llvm_symbol: "main".to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
            },
        );
        contract
            .function_bodies
            .insert(function_id.clone(), caller_key);
        contract.callables.insert(
            callee_key.clone(),
            MirCallableDecl {
                key: callee_key,
                source_def_id: Some(callee),
                kind: MirCallableKind::ObjectProvided,
                llvm_symbol: "callee_alias".to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
            },
        );
        let function = MirFunction {
            id: function_id,
            name: "main".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![],
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Function(callee),
                    ))),
                    args: vec![],
                    destination: Place {
                        local: Local(0),
                        projection: vec![],
                    },
                    target: BasicBlockId(0),
                    span: None,
                }),
            }],
            local_decls: vec![LocalDecl {
                ty: TypeId(0),
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: TypeId(0),
            ownership: Default::default(),
        };

        let errors = validate_backend_contract_against_mir(&contract, [&function]);
        let messages = errors
            .iter()
            .map(|error| format!("{:?}", error))
            .collect::<Vec<_>>();

        assert!(
            messages
                .iter()
                .any(|message| message.contains("ResolvedCallableWithoutContract")),
            "expected missing canonical callable error, got {messages:?}"
        );
    }

    #[test]
    fn backend_validation_reports_program_function_without_callable_mapping_when_contract_is_empty()
    {
        let function = MirFunctionId::Function(def_id(1));
        let contract = MirBackendContract::default();

        assert_eq!(
            validate_backend_contract_against_functions(&contract, [&function]),
            vec![MirBackendContractError::FunctionWithoutCallable { function }],
        );
    }
}
