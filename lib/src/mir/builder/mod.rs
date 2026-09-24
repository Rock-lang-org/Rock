mod blocks;
mod expr;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};

use crate::hir::{
    AcceptedHir, HirCallTarget, HirClosureCapture, HirLiteralPattern, HirPattern, HirVarTarget,
    HirVariantFields,
};
use crate::ids::{AssocTypeId, DefId, InstanceId, TypeId, VariantId};
use crate::lexer::Span;
use crate::source_map::SemanticSourceMap;
use crate::type_context::TypeContext;
use crate::type_services::facts::TypeFacts;
use crate::type_services::projection::{
    ProjectionAssociatedType, ProjectionImpl, ProjectionNormalizer, ProjectionProvider,
};
use crate::types::Type;

use super::{
    BasicBlock, BasicBlockId, Constant, Local, LocalDecl, LocalSource, MirBinOp, MirCallable,
    MirClosureCapture, MirClosureCaptureKind, MirClosureId, MirDropObligation, MirFieldIdentity,
    MirFunction, MirFunctionId, MirInstanceBodies, MirInstanceBody, MirIntrinsicId,
    MirOwnershipMetadata, MirProgram, Mutability, Operand, Place, Projection, ReferenceOrigin,
    Rvalue, StatementData, Terminator,
};

type HirProgram = crate::hir::HirProgramFor<AcceptedHir>;
type HirFunction = crate::hir::HirFunctionFor<AcceptedHir>;
type HirImpl = crate::hir::HirImplFor<AcceptedHir>;
type HirBlock = crate::hir::HirBlockFor<AcceptedHir>;
type HirStmt = crate::hir::HirStmtFor<AcceptedHir>;
type HirExpr = crate::hir::HirExprFor<AcceptedHir>;
type HirExprKind = crate::hir::HirExprKindFor<AcceptedHir>;

pub struct MirBuilder<'a> {
    program: &'a HirProgram,
    type_context: &'a RefCell<TypeContext>,
    blocks: Vec<BasicBlock>,
    locals: Vec<LocalDecl>,
    current_block: Option<BasicBlockId>,
    var_map: HashMap<String, Local>,
    loop_stack: Vec<LoopTargets>,
    scope_locals: Vec<Vec<Local>>,
    moved_places: HashSet<Place>,
    statically_moved_cleanup_places: HashSet<Place>,
    drop_flags: HashMap<Local, Local>,
    projection_drop_flags: HashMap<Place, Local>,
    closure_captures: Vec<MirClosureCapture>,
    lambda_counter: usize,
    current_function_id: Option<MirFunctionId>,
    current_function_name: Option<String>,
    nested_functions: Vec<MirFunction>,
    pending_closure_body_captures: Vec<HirClosureCapture>,
    callable_instances_by_def_id: HashMap<DefId, InstanceId>,
    method_def_ids: HashSet<DefId>,
    method_instance_receiver_modes: HashMap<InstanceId, Option<crate::types::ReceiverMode>>,
    direct_drop_types: HashSet<Type>,
    ownership: MirOwnershipMetadata,
    source_map: SemanticSourceMap,
    current_source_owner: Option<DefId>,
    cleanup_mode: bool,
}

#[derive(Debug, Clone, Copy)]
struct LoopTargets {
    break_target: BasicBlockId,
    continue_target: BasicBlockId,
    cleanup_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchScrutineeMode {
    ByValue,
    Borrowed,
}

struct MirProjectionResolutionProvider<'a> {
    program: &'a HirProgram,
}

impl MirProjectionResolutionProvider<'_> {
    fn matching_impls(
        &self,
        base_ty: &Type,
        trait_id: DefId,
        trait_args: &[Type],
        assoc_type_id: Option<AssocTypeId>,
    ) -> Vec<(&HirImpl, HashMap<crate::types::GenericParamId, Type>)> {
        let mut matches = self
            .program
            .impls_in_order()
            .filter_map(|(_, imp)| {
                if imp.trait_id != Some(trait_id)
                    || assoc_type_id
                        .is_some_and(|id| !imp.associated_types.iter().any(|assoc| assoc.id == id))
                    || imp.trait_arg_types.len() != trait_args.len()
                {
                    return None;
                }

                let mut subst = crate::selection::receiver_pattern_substitution(
                    &imp.receiver_pattern,
                    base_ty,
                )?;
                if imp
                    .trait_arg_types
                    .iter()
                    .zip(trait_args)
                    .all(|(expected, actual)| {
                        crate::selection::type_pattern_matches(expected, actual, &mut subst)
                    })
                {
                    Some((imp, subst))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|(imp, _)| imp.id);
        matches.dedup_by_key(|(imp, _)| imp.id);
        matches
    }

    fn projection_impl(imp: &HirImpl) -> ProjectionImpl {
        ProjectionImpl {
            impl_id: imp.id,
            receiver_pattern: imp.receiver_pattern.clone(),
            trait_arg_types: imp.trait_arg_types.clone(),
            associated_types: imp
                .associated_types
                .iter()
                .map(|assoc| ProjectionAssociatedType {
                    id: assoc.id,
                    name: assoc.name.clone(),
                    ty: assoc.ty.clone(),
                })
                .collect(),
        }
    }
}

impl ProjectionProvider for MirProjectionResolutionProvider<'_> {
    fn resolve_projection_output(
        &self,
        base_ty: &Type,
        trait_id: DefId,
        assoc_type_id: AssocTypeId,
        trait_args: &[Type],
    ) -> Option<Type> {
        let mut matches = self
            .matching_impls(base_ty, trait_id, trait_args, Some(assoc_type_id))
            .into_iter()
            .filter_map(|(imp, subst)| {
                imp.associated_types
                    .iter()
                    .find(|assoc| assoc.id == assoc_type_id)
                    .map(|assoc| (imp.id, assoc.ty.substitute_generics(&subst)))
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|(impl_id, _)| *impl_id);
        matches.dedup_by_key(|(impl_id, _)| *impl_id);
        match matches.as_slice() {
            [(_, output)] => Some(output.clone()),
            _ => None,
        }
    }

    fn find_projection_impl(
        &self,
        base_ty: &Type,
        trait_id: DefId,
        trait_args: &[Type],
    ) -> Option<ProjectionImpl> {
        match self
            .matching_impls(base_ty, trait_id, trait_args, None)
            .as_slice()
        {
            [(imp, _)] => Some(Self::projection_impl(imp)),
            _ => None,
        }
    }
}

impl<'a> MirBuilder<'a> {
    fn generic_substitution_for_fields(
        fields: &[Type],
        source_ty: &Type,
    ) -> HashMap<crate::types::GenericParamId, Type> {
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

    pub fn new(program: &'a HirProgram, type_context: &'a RefCell<TypeContext>) -> Self {
        Self {
            program,
            type_context,
            blocks: Vec::new(),
            locals: Vec::new(),
            current_block: None,
            var_map: HashMap::new(),
            loop_stack: Vec::new(),
            scope_locals: Vec::new(),
            moved_places: HashSet::new(),
            statically_moved_cleanup_places: HashSet::new(),
            drop_flags: HashMap::new(),
            projection_drop_flags: HashMap::new(),
            closure_captures: Vec::new(),
            lambda_counter: 0,
            current_function_id: None,
            current_function_name: None,
            nested_functions: Vec::new(),
            pending_closure_body_captures: Vec::new(),
            callable_instances_by_def_id: HashMap::new(),
            method_def_ids: program.indexes.methods_by_id.keys().copied().collect(),
            method_instance_receiver_modes: HashMap::new(),
            direct_drop_types: HashSet::new(),
            ownership: MirOwnershipMetadata::default(),
            source_map: SemanticSourceMap::default(),
            current_source_owner: None,
            cleanup_mode: false,
        }
    }

    fn with_instance_metadata(
        program: &'a HirProgram,
        type_context: &'a RefCell<TypeContext>,
        method_instance_receiver_modes: HashMap<InstanceId, Option<crate::types::ReceiverMode>>,
        direct_drop_types: HashSet<Type>,
    ) -> Self {
        Self {
            method_instance_receiver_modes,
            direct_drop_types,
            ..Self::new(program, type_context)
        }
    }

    fn with_source_map(mut self, source_map: &SemanticSourceMap) -> Self {
        self.source_map = source_map.clone();
        self
    }

    pub fn build(program: &HirProgram) -> MirProgram {
        let mut functions = std::collections::BTreeMap::new();
        let mut initial_type_context = TypeContext::new();
        let _ = crate::hir::collect_hir_type_ids(program, &mut initial_type_context);
        let type_context = RefCell::new(initial_type_context);

        for (id, name, func) in program.functions_by_id() {
            let mut builder = MirBuilder::new(program, &type_context);
            let mir_id = MirFunctionId::Function(id);
            let mir_func =
                builder.build_function(mir_id.clone(), name, func, false, Some(id), None);
            functions.insert(mir_id, mir_func);
            for nested in builder.take_nested_functions() {
                functions.insert(nested.id.clone(), nested);
            }
        }

        let type_context = type_context.into_inner();
        let mut backend_contract = crate::mir::MirBackendContract::default();
        Self::populate_backend_contract_function_bodies(
            &mut backend_contract,
            &functions,
            &type_context,
        );
        Self::populate_backend_contract_runtime_requirements(&mut backend_contract, &functions);

        MirProgram {
            functions,
            type_context,
            backend_contract,
        }
    }

    pub fn build_monomorphized(program: &crate::mono::MonomorphizedProgram) -> MirProgram {
        let mut program = program.clone();
        let bodies = Self::take_mir_instance_bodies(&mut program);
        Self::build_monomorphized_with_instance_bodies(&program, &bodies)
    }

    pub fn take_mir_instance_bodies(
        program: &mut crate::mono::MonomorphizedProgram,
    ) -> MirInstanceBodies {
        let type_context = RefCell::new(program.type_context.clone());
        let callable_instances_by_def_id = Self::callable_instances_by_def_id_for_program(program);
        let method_def_ids = program
            .instances
            .values()
            .filter_map(|record| match record.origin {
                crate::mono::InstanceOrigin::ImplMethod { method, .. }
                | crate::mono::InstanceOrigin::TraitDefault { method, .. } => Some(method),
                crate::mono::InstanceOrigin::Function(_) => None,
            })
            .collect::<HashSet<_>>();
        let method_instance_receiver_modes = Self::method_instance_receiver_modes_for_program(
            &program.instances,
            &program.pre_mir_instance_bodies,
        );
        let direct_drop_types = Self::direct_drop_types_for_program(program);
        let mut bodies = MirInstanceBodies::new();
        for instance_id in Self::drop_glue_instance_roots(program) {
            bodies.insert_runtime_instance_root(instance_id);
        }

        for record in program.instances.values() {
            if record.provided_by_object {
                continue;
            }
            let Some(body) = program.pre_mir_instance_bodies.take(record.id) else {
                continue;
            };

            let mut builder = MirBuilder::with_instance_metadata(
                &program.program,
                &type_context,
                method_instance_receiver_modes.clone(),
                direct_drop_types.clone(),
            )
            .with_source_map(&program.source_map);
            builder.callable_instances_by_def_id = callable_instances_by_def_id.clone();
            builder
                .method_def_ids
                .extend(method_def_ids.iter().copied());
            let mir_id = MirFunctionId::Instance(record.id);
            let skip_self_cleanup = Self::instance_is_drop_method(program, record);
            let mir_func = builder.build_function(
                mir_id,
                &record.symbols.backend_symbol,
                &body,
                skip_self_cleanup,
                Some(Self::instance_origin_callable_def_id(&record.origin)),
                None,
            );
            bodies.insert(
                record.id,
                MirInstanceBody {
                    function: mir_func,
                    is_method: body.is_method,
                },
            );
            for nested in builder.take_nested_functions() {
                bodies.push_nested(nested);
            }
        }

        program.type_context = type_context.into_inner();
        bodies
    }

    fn drop_glue_instance_roots(
        program: &crate::mono::MonomorphizedProgram,
    ) -> std::collections::BTreeSet<InstanceId> {
        program
            .generated_drop_instances
            .values()
            .map(|generated| generated.instance_id)
            .collect()
    }

    fn instance_is_drop_method(
        program: &crate::mono::MonomorphizedProgram,
        record: &crate::mono::InstanceRecord,
    ) -> bool {
        program
            .generated_drop_instances
            .values()
            .any(|generated| generated.instance_id == record.id)
    }

    fn direct_drop_types_for_program(program: &crate::mono::MonomorphizedProgram) -> HashSet<Type> {
        program
            .generated_drop_instances
            .values()
            .map(|generated| program.type_context.type_for(generated.receiver_ty))
            .collect()
    }

    pub fn build_monomorphized_with_instance_bodies(
        program: &crate::mono::MonomorphizedProgram,
        bodies: &MirInstanceBodies,
    ) -> MirProgram {
        let mut functions = std::collections::BTreeMap::new();
        let type_context = RefCell::new(program.type_context.clone());

        for record in program.instances.values() {
            if record.provided_by_object {
                continue;
            }
            let Some(body) = bodies.get(record.id) else {
                continue;
            };
            functions.insert(body.function.id.clone(), body.function.clone());
        }
        for nested in bodies.nested_functions() {
            functions.insert(nested.id.clone(), nested.clone());
        }

        let mut backend_contract = Self::backend_contract_for_program(
            program,
            &functions,
            &mut type_context.borrow_mut(),
            bodies,
        );
        Self::populate_backend_contract_function_bodies(
            &mut backend_contract,
            &functions,
            &type_context.borrow(),
        );
        Self::populate_backend_contract_runtime_requirements(&mut backend_contract, &functions);

        MirProgram {
            functions,
            type_context: type_context.into_inner(),
            backend_contract,
        }
    }

    fn populate_backend_contract_function_bodies(
        contract: &mut super::MirBackendContract,
        functions: &std::collections::BTreeMap<MirFunctionId, MirFunction>,
        type_context: &TypeContext,
    ) {
        for (function_id, function) in functions {
            if contract.function_bodies.contains_key(function_id) {
                continue;
            }

            let key = match function_id {
                MirFunctionId::Function(id) => super::MirCallableKey::Function(*id),
                MirFunctionId::Extern(id) => super::MirCallableKey::Extern(*id),
                MirFunctionId::Instance(id) => super::MirCallableKey::Instance(*id),
                MirFunctionId::Closure(_) => super::MirCallableKey::Closure(function_id.clone()),
            };
            if contract.callables.contains_key(&key) {
                continue;
            }

            let params = function
                .local_decls
                .iter()
                .skip(1)
                .take(function.arg_count)
                .map(|local| local.ty)
                .collect::<Vec<_>>();
            let mut signature = super::MirCallableSignature::from_type_ids(
                &params,
                function.ret_type,
                super::MirPassMode::Direct,
            );
            if function.name == "main" {
                if let Some(i32_ty) = type_context.id_for_type(&Type::I32) {
                    signature.ret.abi_ty = i32_ty;
                }
            }

            contract.callables.insert(
                key.clone(),
                super::MirCallableDecl {
                    key: key.clone(),
                    source_def_id: match function_id {
                        MirFunctionId::Function(id) | MirFunctionId::Extern(id) => Some(*id),
                        MirFunctionId::Instance(_) | MirFunctionId::Closure(_) => None,
                    },
                    kind: super::MirCallableKind::LocalBody {
                        function_id: function_id.clone(),
                    },
                    llvm_symbol: Self::mir_contract_local_body_symbol(function_id, function),
                    linkage: super::MirLinkage::Internal,
                    signature,
                },
            );
            contract.function_bodies.insert(function_id.clone(), key);
        }

        Self::populate_backend_contract_intrinsic_callables(contract, functions, type_context);
    }

    fn populate_backend_contract_runtime_requirements(
        contract: &mut super::MirBackendContract,
        functions: &std::collections::BTreeMap<MirFunctionId, MirFunction>,
    ) {
        contract.runtime_requirements =
            super::backend_contract::runtime_requirements_for_functions(functions.values());
    }

    fn mir_contract_local_body_symbol(
        function_id: &MirFunctionId,
        function: &MirFunction,
    ) -> String {
        match function_id {
            MirFunctionId::Closure(closure_id) => {
                Self::mir_contract_closure_symbol(&function.name, closure_id)
            }
            _ => function.name.clone(),
        }
    }

    fn mir_contract_closure_symbol(parent_symbol: &str, closure_id: &MirClosureId) -> String {
        format!(
            "__mir_closure_{}_{}_{}",
            Self::sanitize_mir_contract_symbol(parent_symbol),
            Self::mir_function_id_symbol_key(&closure_id.owner),
            closure_id.local_index
        )
    }

    fn mir_function_id_symbol_key(id: &MirFunctionId) -> String {
        match id {
            MirFunctionId::Function(id) => format!("fn_{}_{}", id.crate_id.0, id.local.0),
            MirFunctionId::Extern(id) => format!("extern_{}_{}", id.crate_id.0, id.local.0),
            MirFunctionId::Instance(id) => format!("instance_{}", id.0),
            MirFunctionId::Closure(id) => format!(
                "closure_{}_{}",
                Self::mir_function_id_symbol_key(&id.owner),
                id.local_index
            ),
        }
    }

    fn sanitize_mir_contract_symbol(symbol: &str) -> String {
        symbol
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
            .collect()
    }

    fn populate_backend_contract_intrinsic_callables(
        contract: &mut super::MirBackendContract,
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
                    super::MirCallableKey::Intrinsic(intrinsic),
                ))) = func
                else {
                    continue;
                };
                let key = super::MirCallableKey::Intrinsic(intrinsic.clone());
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
                    super::MirCallableDecl {
                        key: key.clone(),
                        source_def_id: None,
                        kind: super::MirCallableKind::Intrinsic(intrinsic.clone()),
                        llvm_symbol: intrinsic.as_str().to_string(),
                        linkage: super::MirLinkage::Internal,
                        signature,
                    },
                );
            }
        }
    }

    fn intrinsic_signature_from_call(
        function: &MirFunction,
        contract: &super::MirBackendContract,
        type_context: &TypeContext,
        args: &[Operand],
        destination: &Place,
    ) -> Option<super::MirCallableSignature> {
        let params = args
            .iter()
            .map(|arg| Self::operand_type_id_for_contract(function, contract, type_context, arg))
            .collect::<Option<Vec<_>>>()?;
        let ret = Self::place_type_id_for_contract(function, contract, type_context, destination)
            .or_else(|| type_context.id_for_type(&Type::Unit))?;

        Some(super::MirCallableSignature::from_type_ids(
            &params,
            ret,
            super::MirPassMode::Direct,
        ))
    }

    fn operand_type_id_for_contract(
        function: &MirFunction,
        contract: &super::MirBackendContract,
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
            Operand::Constant(Constant::Char(_)) => type_context.id_for_type(&Type::U8),
            Operand::Constant(Constant::String(_)) => type_context.id_for_type(&Type::Str),
            Operand::Constant(Constant::Unit) => type_context.id_for_type(&Type::Unit),
            Operand::Constant(Constant::TypeId(id)) => Some(*id),
            Operand::Constant(Constant::Callable(_)) => None,
        }
    }

    fn place_type_id_for_contract(
        function: &MirFunction,
        contract: &super::MirBackendContract,
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
        contract: &super::MirBackendContract,
        type_context: &TypeContext,
        ty: &Type,
        index: usize,
        downcast_variant: Option<VariantId>,
    ) -> Option<Type> {
        let ty = contract.normalize_type(type_context, ty);
        let projected = match &ty {
            Type::Tuple(fields) => fields.get(index).cloned(),
            Type::Struct { id, args } => {
                let super::MirNominalLayout::Struct { fields, .. } =
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
                let super::MirNominalLayout::Enum { variants, .. } =
                    contract.nominal_layouts.get(id)?
                else {
                    return None;
                };
                let variant = variants.get(variant_id.0 as usize)?;
                let variant_fields = match &variant.fields {
                    super::MirVariantLayoutFields::Unit => return None,
                    super::MirVariantLayoutFields::Positional(fields) => fields
                        .iter()
                        .map(|field_ty| (None, *field_ty))
                        .collect::<Vec<_>>(),
                    super::MirVariantLayoutFields::Named(fields) => fields
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

    fn backend_contract_for_program(
        program: &crate::mono::MonomorphizedProgram,
        functions: &std::collections::BTreeMap<MirFunctionId, MirFunction>,
        type_context: &mut TypeContext,
        bodies: &MirInstanceBodies,
    ) -> super::MirBackendContract {
        let mut contract = crate::mir::MirBackendContract::default();
        Self::populate_backend_contract_externs(&mut contract, &program.program, type_context);
        Self::populate_backend_contract_nominal_layouts(
            &mut contract,
            &program.program,
            type_context,
        );

        contract.projection_traits.extend(
            program
                .program
                .traits_by_id()
                .map(|(trait_id, _, _)| trait_id),
        );
        contract.projection_traits.extend(
            program
                .program
                .impls_in_order()
                .filter_map(|(_, imp)| imp.trait_id),
        );

        let exportable_drop_glue_symbols = program
            .instances
            .values()
            .filter(|record| {
                Self::instance_is_drop_method(program, record)
                    && record.substitution.is_empty()
                    && bodies.contains_key(record.id)
                    && !record.provided_by_object
            })
            .map(|record| record.symbols.backend_symbol.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        for record in program.instances.values() {
            let (params, ret, is_method): (Vec<TypeId>, TypeId, bool) =
                if let Some(body) = bodies.get(record.id) {
                    (
                        body.function
                            .local_decls
                            .iter()
                            .skip(1)
                            .take(body.function.arg_count)
                            .map(|local| local.ty)
                            .collect(),
                        body.function.ret_type,
                        body.is_method,
                    )
                } else if let Some(function) = record.declared.as_ref() {
                    (
                        function
                            .params
                            .iter()
                            .map(|param| type_context.intern_type(&param.ty))
                            .collect(),
                        type_context.intern_type(&function.ret_type),
                        function.is_method,
                    )
                } else {
                    panic!(
                        "instance {:?} has neither a MIR body nor a declared signature",
                        record.id,
                    );
                };

            let key = super::MirCallableKey::Instance(record.id);
            let function_id = MirFunctionId::Instance(record.id);
            let kind = if record.provided_by_object {
                super::MirCallableKind::ObjectProvided
            } else {
                assert!(
                    contract
                        .function_bodies
                        .insert(function_id.clone(), key.clone())
                        .is_none(),
                    "duplicate instance body authority for {:?}",
                    record.id,
                );
                super::MirCallableKind::LocalBody { function_id }
            };
            let mut signature = super::MirCallableSignature::from_instance_type_ids(
                &params,
                ret,
                is_method,
                type_context,
            );
            if record.symbols.backend_symbol == "main" {
                signature.ret.abi_ty = type_context.intern_type(&Type::I32);
            }
            let linkage = if record.is_specialization
                && !record.provided_by_object
                && !exportable_drop_glue_symbols.contains(record.symbols.backend_symbol.as_str())
            {
                super::MirLinkage::Internal
            } else {
                super::MirLinkage::External
            };
            assert!(
                contract
                    .callables
                    .insert(
                        key.clone(),
                        super::MirCallableDecl {
                            key,
                            source_def_id: Some(Self::instance_origin_callable_def_id(
                                &record.origin
                            )),
                            kind,
                            llvm_symbol: record.symbols.backend_symbol.clone(),
                            linkage,
                            signature,
                        },
                    )
                    .is_none(),
                "duplicate instance callable authority for {:?}",
                record.id,
            );
            contract.artifact_exports.push(super::MirArtifactExport {
                origin_def_id: Some(Self::instance_origin_callable_def_id(&record.origin)),
                source_name: record.symbols.source_name.clone(),
                backend_symbol: record.symbols.backend_symbol.clone(),
                substitution_empty: record.substitution.is_empty(),
                has_body: bodies.contains_key(record.id),
                provided_by_object: record.provided_by_object,
                is_specialization: record.is_specialization,
                is_drop_glue: Self::instance_is_drop_method(program, record),
            });
        }

        Self::populate_backend_contract_projection_outputs(
            &mut contract,
            &program.program,
            functions,
            type_context,
        );
        Self::populate_backend_contract_drop_glue(&mut contract, &program.generated_drop_instances);

        contract
    }

    fn mir_generic_param_ids(
        params: &[crate::types::GenericParamDecl],
    ) -> Vec<crate::types::GenericParamId> {
        params.iter().map(|param| param.id).collect()
    }

    fn populate_backend_contract_externs(
        contract: &mut super::MirBackendContract,
        program: &HirProgram,
        type_context: &mut TypeContext,
    ) {
        for (_, ext) in program.externs_in_order() {
            let key = super::MirCallableKey::Extern(ext.id);
            let params = ext
                .params
                .iter()
                .map(|ty| type_context.intern_type(ty))
                .collect::<Vec<_>>();
            let ret = type_context.intern_type(&ext.ret);
            assert!(
                contract
                    .callables
                    .insert(
                        key.clone(),
                        super::MirCallableDecl {
                            key,
                            source_def_id: Some(ext.id),
                            kind: super::MirCallableKind::Extern {
                                link_name: ext.name.clone(),
                                variadic: ext.variadic,
                            },
                            llvm_symbol: ext.name.clone(),
                            linkage: super::MirLinkage::External,
                            signature: super::MirCallableSignature::from_type_ids(
                                &params,
                                ret,
                                super::MirPassMode::Direct,
                            ),
                        },
                    )
                    .is_none(),
                "duplicate extern callable authority for {:?}",
                ext.id,
            );
        }
    }

    fn populate_backend_contract_nominal_layouts(
        contract: &mut super::MirBackendContract,
        program: &HirProgram,
        type_context: &mut TypeContext,
    ) {
        for (id, _, structure) in program.structs_by_id() {
            assert!(
                contract
                    .nominal_layouts
                    .insert(
                        id,
                        super::MirNominalLayout::Struct {
                            id,
                            fields: structure
                                .fields
                                .iter()
                                .map(|field| {
                                    (field.name.clone(), type_context.intern_type(&field.ty))
                                })
                                .collect(),
                            generic_params: Self::mir_generic_param_ids(&structure.generic_params),
                        },
                    )
                    .is_none(),
                "duplicate struct layout authority for {id:?}",
            );
        }
        for (id, _, enum_def) in program.enums_by_id() {
            assert!(
                contract
                    .nominal_layouts
                    .insert(
                        id,
                        super::MirNominalLayout::Enum {
                            id,
                            variants: Self::mir_enum_variants(type_context, &enum_def.variants),
                            generic_params: Self::mir_generic_param_ids(&enum_def.generic_params),
                        },
                    )
                    .is_none(),
                "duplicate enum layout authority for {id:?}",
            );
        }
    }

    fn populate_backend_contract_drop_glue(
        contract: &mut super::MirBackendContract,
        generated: &std::collections::BTreeMap<TypeId, crate::mono::GeneratedMethodInstance>,
    ) {
        for entry in generated.values() {
            assert!(
                contract
                    .drop_glue
                    .insert(
                        entry.receiver_ty,
                        super::MirCallableKey::Instance(entry.instance_id),
                    )
                    .is_none(),
                "duplicate generated drop glue authority for {:?}",
                entry.receiver_ty,
            );
        }
    }

    fn populate_backend_contract_projection_outputs(
        contract: &mut super::MirBackendContract,
        program: &HirProgram,
        functions: &std::collections::BTreeMap<MirFunctionId, MirFunction>,
        type_context: &mut TypeContext,
    ) {
        let mut used_type_ids = BTreeSet::new();

        for function in functions.values() {
            used_type_ids.insert(function.ret_type);
            for local in &function.local_decls {
                used_type_ids.insert(local.ty);
            }
            for block in &function.basic_blocks {
                for statement in &block.statements {
                    if let super::StatementKind::Assign(_, super::Rvalue::Cast(_, ty)) =
                        &statement.kind
                    {
                        used_type_ids.insert(*ty);
                    }
                }
            }
        }

        for callable in contract.callables.values() {
            for param in &callable.signature.params {
                used_type_ids.insert(param.semantic_ty);
            }
            used_type_ids.insert(callable.signature.ret.semantic_ty);
            used_type_ids.insert(callable.signature.ret.abi_ty);
        }

        for layout in contract.nominal_layouts.values() {
            match layout {
                super::MirNominalLayout::Struct { fields, .. } => {
                    for (_, ty) in fields {
                        used_type_ids.insert(*ty);
                    }
                }
                super::MirNominalLayout::Enum { variants, .. } => {
                    for variant in variants {
                        match &variant.fields {
                            super::MirVariantLayoutFields::Unit => {}
                            super::MirVariantLayoutFields::Positional(fields) => {
                                for ty in fields {
                                    used_type_ids.insert(*ty);
                                }
                            }
                            super::MirVariantLayoutFields::Named(fields) => {
                                for (_, ty) in fields {
                                    used_type_ids.insert(*ty);
                                }
                            }
                        }
                    }
                }
            }
        }

        let provider = MirProjectionResolutionProvider { program };
        for ty in used_type_ids {
            Self::collect_projection_output_for_type_id(ty, type_context, &provider, contract);
        }
    }

    fn collect_projection_output_for_type_id(
        ty: TypeId,
        type_context: &mut TypeContext,
        provider: &MirProjectionResolutionProvider<'_>,
        contract: &mut super::MirBackendContract,
    ) {
        let ty = type_context.type_for(ty);
        Self::collect_projection_output_for_type(&ty, type_context, provider, contract);
    }

    fn collect_projection_output_for_type(
        ty: &Type,
        type_context: &mut TypeContext,
        provider: &MirProjectionResolutionProvider<'_>,
        contract: &mut super::MirBackendContract,
    ) {
        match ty {
            Type::Projection {
                ty: base,
                trait_id,
                assoc_type,
                trait_args,
            } => {
                Self::collect_projection_output_for_type(base, type_context, provider, contract);
                for arg in trait_args {
                    Self::collect_projection_output_for_type(arg, type_context, provider, contract);
                }

                if assoc_type.owner == *trait_id {
                    let resolved_base = ProjectionNormalizer::normalize(provider, base);
                    let resolved_trait_args = trait_args
                        .iter()
                        .map(|arg| ProjectionNormalizer::normalize(provider, arg))
                        .collect::<Vec<_>>();
                    let output = provider.resolve_projection_output(
                        &resolved_base,
                        *trait_id,
                        assoc_type.assoc_type_id,
                        &resolved_trait_args,
                    );

                    if let Some(output) = output {
                        let output = ProjectionNormalizer::normalize(provider, &output);
                        let base = type_context.intern_type(&resolved_base);
                        let trait_args = resolved_trait_args
                            .iter()
                            .map(|arg| type_context.intern_type(arg))
                            .collect();
                        let output = type_context.intern_type(&output);
                        let key = super::MirProjectionKey {
                            base,
                            trait_id: *trait_id,
                            assoc_type_id: assoc_type.assoc_type_id,
                            trait_args,
                        };
                        if let Some(existing) = contract.projection_outputs.get(&key) {
                            assert_eq!(
                                *existing, output,
                                "conflicting projection output for canonical key {key:?}"
                            );
                        } else {
                            contract.projection_outputs.insert(key, output);
                        }
                    }
                }
            }
            Type::Reference { inner, .. } | Type::Pointer(inner) | Type::Slice(inner) => {
                Self::collect_projection_output_for_type(inner, type_context, provider, contract);
            }
            Type::Array(inner, _) => {
                Self::collect_projection_output_for_type(inner, type_context, provider, contract);
            }
            Type::Tuple(elems) => {
                for elem in elems {
                    Self::collect_projection_output_for_type(
                        elem,
                        type_context,
                        provider,
                        contract,
                    );
                }
            }
            Type::Function {
                params,
                ret,
                captures,
                ..
            } => {
                for arg in params {
                    Self::collect_projection_output_for_type(arg, type_context, provider, contract);
                }
                Self::collect_projection_output_for_type(ret, type_context, provider, contract);
                for capture in captures {
                    Self::collect_projection_output_for_type(
                        &capture.ty,
                        type_context,
                        provider,
                        contract,
                    );
                }
            }
            Type::Struct { args, .. } | Type::Enum { args, .. } => {
                for arg in args {
                    Self::collect_projection_output_for_type(arg, type_context, provider, contract);
                }
            }
            _ => {}
        }
    }

    fn instance_origin_callable_def_id(origin: &crate::mono::InstanceOrigin) -> DefId {
        match origin {
            crate::mono::InstanceOrigin::Function(id) => *id,
            crate::mono::InstanceOrigin::ImplMethod { method, .. }
            | crate::mono::InstanceOrigin::TraitDefault { method, .. } => *method,
        }
    }

    fn mir_enum_variants(
        type_context: &mut TypeContext,
        variants: &[crate::hir::HirVariant],
    ) -> Vec<super::MirEnumVariantLayout> {
        variants
            .iter()
            .map(|variant| super::MirEnumVariantLayout {
                name: variant.name.clone(),
                fields: match &variant.fields {
                    HirVariantFields::Unit => super::MirVariantLayoutFields::Unit,
                    HirVariantFields::Positional(fields) => {
                        super::MirVariantLayoutFields::Positional(
                            fields
                                .iter()
                                .map(|ty| type_context.intern_type(ty))
                                .collect(),
                        )
                    }
                    HirVariantFields::Named(fields) => super::MirVariantLayoutFields::Named(
                        fields
                            .iter()
                            .map(|field| (field.name.clone(), type_context.intern_type(&field.ty)))
                            .collect(),
                    ),
                },
            })
            .collect()
    }

    fn callable_instances_by_def_id_for_program(
        program: &crate::mono::MonomorphizedProgram,
    ) -> HashMap<DefId, InstanceId> {
        let mut instances = HashMap::new();

        for record in program.instances.values() {
            if !record.substitution.is_empty() {
                continue;
            }
            let crate::mono::InstanceOrigin::Function(def_id) = record.origin else {
                continue;
            };
            instances.insert(def_id, record.id);
        }

        instances
    }

    fn method_instance_receiver_modes_for_program(
        instances: &std::collections::BTreeMap<InstanceId, crate::mono::InstanceRecord>,
        pre_mir_bodies: &crate::mono::PreMirInstanceBodies,
    ) -> HashMap<InstanceId, Option<crate::types::ReceiverMode>> {
        instances
            .values()
            .filter_map(|record| match record.origin {
                crate::mono::InstanceOrigin::ImplMethod { .. }
                | crate::mono::InstanceOrigin::TraitDefault { .. } => pre_mir_bodies
                    .get(record.id)
                    .or(record.declared.as_ref())
                    .map(|function| (record.id, function.self_receiver)),
                crate::mono::InstanceOrigin::Function(_) => None,
            })
            .collect()
    }

    fn new_block(&mut self) -> BasicBlockId {
        let id = BasicBlockId(self.blocks.len());
        self.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: None,
        });
        id
    }

    fn type_id_for(&self, ty: &Type) -> TypeId {
        self.type_context.borrow_mut().intern_type(ty)
    }

    fn new_local(&mut self, ty: TypeId, mutability: Mutability, name: Option<String>) -> Local {
        self.new_local_with_source(ty, mutability, name, LocalSource::Temporary, None)
    }

    fn new_local_with_source(
        &mut self,
        ty: TypeId,
        mutability: Mutability,
        name: Option<String>,
        source: LocalSource,
        span: Option<Span>,
    ) -> Local {
        let id = Local(self.locals.len());
        self.locals.push(LocalDecl {
            ty,
            mutability,
            name,
            span,
            source,
        });
        if source.is_temporary() {
            self.record_temporary_local(id);
        }
        id
    }

    fn new_local_from_expr(&mut self, ty: Type, expr: &HirExpr) -> Local {
        let id = Local(self.locals.len());
        let ty = self.type_id_for(&ty);
        self.locals.push(LocalDecl {
            ty,
            mutability: Mutability::Not,
            name: None,
            span: Some(expr.span.clone()),
            source: LocalSource::Temporary,
        });
        self.record_temporary_local(id);
        id
    }

    fn needs_move(ty: &Type) -> bool {
        !TypeFacts::is_copy(ty)
    }

    fn get_local_type(&self, local: Local) -> Option<Type> {
        self.locals
            .get(local.0)
            .map(|decl| self.type_context.borrow().type_for(decl.ty))
    }

    fn place_for_borrowed_binding_value(&self, local: Local, hir_ty: &Type) -> Option<Place> {
        match self.get_local_type(local)? {
            Type::Reference { mutable: _, inner } if inner.as_ref() == hir_ty => Some(Place {
                local,
                projection: vec![Projection::Deref],
            }),
            _ => None,
        }
    }

    fn place_for_local_expr(&self, local: Local, hir_ty: &Type) -> Place {
        self.place_for_borrowed_binding_value(local, hir_ty)
            .unwrap_or(Place {
                local,
                projection: vec![],
            })
    }

    fn new_closure_id_and_name(&mut self) -> (MirClosureId, String) {
        let owner = self
            .current_function_id
            .clone()
            .expect("closure lowering requires current MIR function id");
        let index = self.lambda_counter as u32;
        let name = format!("lambda_{}", self.lambda_counter);
        self.lambda_counter += 1;
        (
            MirClosureId {
                owner,
                local_index: index,
            },
            name,
        )
    }

    fn take_nested_functions(&mut self) -> Vec<MirFunction> {
        std::mem::take(&mut self.nested_functions)
    }

    fn lower_closure_captures(&mut self, captures: &[HirClosureCapture]) -> Vec<MirClosureCapture> {
        captures
            .iter()
            .filter_map(|capture| {
                let mut local = self.var_map.get(&capture.name).copied()?;
                let span = self.locals.get(local.0).and_then(|decl| decl.span.clone());
                let mut kind = MirClosureCaptureKind::from(capture.kind);
                if let Some(place) = self.place_for_borrowed_binding_value(local, &capture.ty) {
                    // A transitive capture borrows the original value, not the
                    // enclosing closure's reference slot. Materialize that
                    // reborrow because MIR captures store locals, not places.
                    let (ty, value) = match kind {
                        MirClosureCaptureKind::ByValue => (
                            capture.ty.clone(),
                            Rvalue::Use(self.operand_for_place(&capture.ty, place, false)),
                        ),
                        MirClosureCaptureKind::ByRef | MirClosureCaptureKind::ByMutRef => {
                            let mutable = kind == MirClosureCaptureKind::ByMutRef;
                            (
                                Type::Reference {
                                    mutable,
                                    inner: Box::new(capture.ty.clone()),
                                },
                                Rvalue::Ref(
                                    if mutable {
                                        Mutability::Mut
                                    } else {
                                        Mutability::Not
                                    },
                                    place,
                                ),
                            )
                        }
                    };
                    local = self.new_local_with_source(
                        self.type_id_for(&ty),
                        Mutability::Not,
                        None,
                        LocalSource::Temporary,
                        span.clone(),
                    );
                    self.register_scoped_temp(local);
                    self.emit_storage_live(local, span.clone());
                    self.emit_assign(
                        Place {
                            local,
                            projection: vec![],
                        },
                        value,
                        span.clone(),
                    );
                    kind = MirClosureCaptureKind::ByValue;
                }
                Some(MirClosureCapture {
                    name: capture.name.clone(),
                    local,
                    kind,
                    span,
                })
            })
            .collect()
    }

    fn source_local_span(
        &self,
        owner: Option<DefId>,
        local: crate::ids::HirLocalId,
    ) -> Option<Span> {
        owner.and_then(|owner| self.source_map.local_span(owner, local).cloned())
    }

    fn source_operation_span(
        &self,
        _owner: Option<DefId>,
        operation: Option<Span>,
    ) -> Option<Span> {
        operation
    }

    fn source_origin(&self, span: Option<Span>) -> super::MirOrigin {
        span.map(super::MirOrigin::Source).unwrap_or_else(|| {
            super::MirOrigin::synthetic(super::MirSyntheticOrigin::ControlFlow, None)
        })
    }

    fn cleanup_origin(&self, place: &Place) -> super::MirOrigin {
        super::MirOrigin::synthetic(
            super::MirSyntheticOrigin::Cleanup,
            self.locals
                .get(place.local.0)
                .and_then(|local| local.span.clone()),
        )
    }

    fn drop_origin(&self, place: &Place) -> super::MirOrigin {
        let span = self
            .locals
            .get(place.local.0)
            .and_then(|local| local.span.clone());
        if self.cleanup_mode {
            super::MirOrigin::synthetic(super::MirSyntheticOrigin::Cleanup, span)
        } else {
            self.source_origin(span)
        }
    }

    fn push_closure_body_function(
        &mut self,
        closure_id: MirClosureId,
        lambda_name: &str,
        params: Vec<crate::hir::HirParam>,
        body: HirBlock,
        captures: Vec<HirClosureCapture>,
        operation_span: Span,
    ) {
        let hir_id = match &closure_id.owner {
            MirFunctionId::Function(id) | MirFunctionId::Extern(id) => *id,
            MirFunctionId::Instance(_) | MirFunctionId::Closure(_) => {
                crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0))
            }
        };
        let function = HirFunction {
            id: hir_id,
            name: lambda_name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: crate::hir::HirGenericBounds::new(),
            params,
            ret_type: body.ty.clone(),
            body,
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };

        let mut builder = MirBuilder::with_instance_metadata(
            self.program,
            self.type_context,
            self.method_instance_receiver_modes.clone(),
            self.direct_drop_types.clone(),
        )
        .with_source_map(&self.source_map);
        builder.callable_instances_by_def_id = self.callable_instances_by_def_id.clone();
        builder.method_def_ids = self.method_def_ids.clone();
        builder.pending_closure_body_captures = captures;
        let function_id = MirFunctionId::Closure(Box::new(closure_id));
        let mir_func = builder.build_function(
            function_id,
            lambda_name,
            &function,
            false,
            self.current_source_owner,
            Some(operation_span),
        );
        self.nested_functions.push(mir_func);
        self.nested_functions
            .extend(builder.take_nested_functions());
    }

    fn callable_for_var_target(&self, target: &HirVarTarget) -> Option<MirCallable> {
        match target {
            HirVarTarget::Function(id) => Some(MirCallable::Resolved(
                self.callable_key_for_def_id(*id, super::MirCallableKey::Function(*id)),
            )),
            HirVarTarget::Extern(id) => Some(MirCallable::Resolved(
                self.callable_key_for_def_id(*id, super::MirCallableKey::Extern(*id)),
            )),
            HirVarTarget::Instance(id) => {
                Some(MirCallable::Resolved(super::MirCallableKey::Instance(*id)))
            }
            HirVarTarget::Local(_) => None,
        }
    }

    fn callable_for_expr(&self, expr: &HirExpr, ret_ty: &Type) -> Option<MirCallable> {
        match &expr.kind {
            HirExprKind::ResolvedVar(reference) => {
                self.callable_for_var_target_with_return(&reference.target, ret_ty)
            }
            _ => None,
        }
    }

    fn callable_for_call_target(
        &self,
        target: &HirCallTarget,
        _ret_ty: &Type,
    ) -> Option<MirCallable> {
        match target {
            HirCallTarget::Function(id) => Some(MirCallable::Resolved(
                self.callable_key_for_def_id(*id, super::MirCallableKey::Function(*id)),
            )),
            HirCallTarget::Extern(id) => Some(MirCallable::Resolved(
                self.callable_key_for_def_id(*id, super::MirCallableKey::Extern(*id)),
            )),
            HirCallTarget::Instance(id) => {
                Some(MirCallable::Resolved(super::MirCallableKey::Instance(*id)))
            }
            HirCallTarget::Local(_) => None,
            HirCallTarget::Intrinsic(name) => MirIntrinsicId::from_name(name)
                .map(|id| MirCallable::Resolved(super::MirCallableKey::Intrinsic(id))),
            HirCallTarget::StaticMethod(_) => {
                panic!("unmaterialized static method target reached MIR lowering")
            }
        }
    }

    fn callable_key_for_def_id(
        &self,
        id: DefId,
        fallback: super::MirCallableKey,
    ) -> super::MirCallableKey {
        if self.method_def_ids.contains(&id) {
            panic!("unmaterialized method DefId reached MIR lowering: {id:?}");
        }
        self.callable_instances_by_def_id
            .get(&id)
            .copied()
            .map(super::MirCallableKey::Instance)
            .unwrap_or(fallback)
    }

    fn callable_for_var_target_with_return(
        &self,
        target: &HirVarTarget,
        _ret_ty: &Type,
    ) -> Option<MirCallable> {
        match target {
            HirVarTarget::Function(id) => Some(MirCallable::Resolved(
                self.callable_key_for_def_id(*id, super::MirCallableKey::Function(*id)),
            )),
            _ => self.callable_for_var_target(target),
        }
    }

    fn field_access_resolves_to_struct_field(
        &self,
        base_ty: &Type,
        field_name: &str,
        location: Option<&crate::hir::HirFieldLocation>,
    ) -> bool {
        let Type::Struct { id, .. } = base_ty else {
            return false;
        };
        let Some((_, fields)) = self.program.struct_by_id(*id) else {
            return false;
        };

        location
            .filter(|location| location.owner == fields.id)
            .is_some_and(|location| {
                fields.fields.iter().any(|field| {
                    field.id == location.field_id
                        && field.name == location.name
                        && field.name == *field_name
                })
            })
    }

    fn unknown_field_message(&self, base_ty: &Type, field_name: &str) -> String {
        let type_name = match base_ty {
            Type::Struct { id, .. } => self
                .program
                .indexes
                .structs_by_id
                .get(id)
                .cloned()
                .unwrap_or_else(|| base_ty.to_string()),
            Type::Enum { id, .. } => self
                .program
                .indexes
                .enums_by_id
                .get(id)
                .cloned()
                .unwrap_or_else(|| base_ty.to_string()),
            _ => base_ty.to_string(),
        };

        format!("Unknown field '{}' on struct '{}'", field_name, type_name)
    }

    fn method_receiver_mode_for_callable(
        &self,
        callable: &MirCallable,
    ) -> Option<crate::types::ReceiverMode> {
        match callable {
            MirCallable::Resolved(super::MirCallableKey::Instance(id)) => self
                .method_instance_receiver_modes
                .get(id)
                .copied()
                .flatten(),
            _ => None,
        }
    }

    fn field_projection(
        &self,
        index: usize,
        owner: Option<crate::ids::DefId>,
        field_id: Option<crate::ids::FieldId>,
    ) -> Projection {
        Projection::Field {
            index,
            identity: owner
                .zip(field_id)
                .map(|(owner, field_id)| MirFieldIdentity::new(owner, field_id)),
        }
    }

    fn emit_assign(&mut self, dest: Place, rvalue: Rvalue, span: Option<Span>) {
        self.record_rvalue_moves(&rvalue);
        self.record_reference_assignment(&dest, &rvalue);
        self.clear_moved_place(&dest);
        if let Some(current) = self.current_block {
            self.blocks[current.0]
                .statements
                .push(StatementData::assign(dest.clone(), rvalue, span.clone()));
        }
        self.mark_place_initialized(&dest, span);
    }

    fn record_reference_assignment(&mut self, dest: &Place, rvalue: &Rvalue) {
        if !dest.projection.is_empty() || !self.local_type_is_reference(dest.local) {
            return;
        }

        let origin = match rvalue {
            Rvalue::Ref(_, place) => self.reference_origin_for_place(place),
            Rvalue::Use(Operand::Copy(place) | Operand::Move(place))
                if place.projection.is_empty() =>
            {
                self.reference_origin_for_local(place.local)
            }
            _ => None,
        };

        if let Some(origin) = origin {
            self.record_reference_origin(dest.local, origin);
        }
    }

    fn record_reference_origin(&mut self, local: Local, origin: ReferenceOrigin) {
        self.ownership
            .reference_origins
            .retain(|(origin_local, _)| *origin_local != local);
        self.ownership.reference_origins.push((local, origin));
    }

    fn record_temporary_local(&mut self, local: Local) {
        self.ownership.temporary_locals.push(local);
    }

    fn record_drop_obligation(
        &mut self,
        place: Place,
        ty: TypeId,
        kind: super::DropObligationKind,
    ) {
        self.ownership
            .drop_obligations
            .push(MirDropObligation { place, ty, kind });
    }

    fn local_type_is_reference(&self, local: Local) -> bool {
        self.get_local_type(local)
            .is_some_and(|ty| matches!(ty, Type::Reference { .. }))
    }

    fn local_type_is_pointer(&self, local: Local) -> bool {
        self.get_local_type(local)
            .is_some_and(|ty| matches!(ty, Type::Pointer(_)))
    }

    fn reference_origin_for_place(&self, place: &Place) -> Option<ReferenceOrigin> {
        if matches!(place.projection.first(), Some(Projection::Deref)) {
            if self.local_type_is_pointer(place.local) {
                return Some(ReferenceOrigin::UnknownExternal);
            }
            if let Some(origin) = self.reference_origin_for_local(place.local) {
                return Some(origin);
            }
        }

        if !place.projection.is_empty() {
            if let Some(ReferenceOrigin::Param(local)) =
                self.reference_origin_for_local(place.local)
            {
                return Some(ReferenceOrigin::Param(local));
            }
        }

        self.locals
            .get(place.local.0)
            .map(|decl| match decl.source {
                LocalSource::Argument => ReferenceOrigin::Local(place.local),
                LocalSource::UserBinding
                | LocalSource::ReturnPlace
                | LocalSource::ClosureCapture => ReferenceOrigin::Local(place.local),
                LocalSource::Temporary => ReferenceOrigin::Temporary(place.local),
            })
    }

    fn reference_origin_for_local(&self, local: Local) -> Option<ReferenceOrigin> {
        self.ownership
            .reference_origins
            .iter()
            .rev()
            .find_map(|(origin_local, origin)| (*origin_local == local).then_some(*origin))
            .or_else(|| {
                self.locals.get(local.0).and_then(|decl| {
                    if !self.local_type_is_reference(local) {
                        return None;
                    }
                    Some(match decl.source {
                        LocalSource::Argument => ReferenceOrigin::Param(local),
                        LocalSource::UserBinding
                        | LocalSource::ReturnPlace
                        | LocalSource::ClosureCapture => ReferenceOrigin::Local(local),
                        LocalSource::Temporary => ReferenceOrigin::Temporary(local),
                    })
                })
            })
    }

    fn record_operand_move(&mut self, operand: &Operand) {
        if let Operand::Move(place) = operand {
            if place.projection.is_empty() {
                self.set_drop_flag(place.local, false, None);
                self.set_projection_drop_flags_for_local(place.local, false, None);
                self.moved_places.retain(|moved| moved.local != place.local);
            } else {
                self.set_drop_flags_for_place_tree(place, false, None);
                self.moved_places.insert(place.clone());
            }
        }
    }

    fn clear_moved_place(&mut self, place: &Place) {
        if place.projection.is_empty() {
            self.moved_places.retain(|moved| moved.local != place.local);
            self.statically_moved_cleanup_places
                .retain(|moved| moved.local != place.local);
        } else {
            self.moved_places
                .retain(|moved| !Self::place_projection_is_prefix(place, moved));
            self.statically_moved_cleanup_places
                .retain(|moved| !Self::place_projection_is_prefix(place, moved));
        }
    }

    fn mark_place_moved_for_cleanup(&mut self, place: Place) {
        if place.projection.is_empty() {
            self.set_drop_flag(place.local, false, None);
            self.set_projection_drop_flags_for_local(place.local, false, None);
            self.moved_places.retain(|moved| moved.local != place.local);
        } else {
            self.set_drop_flags_for_place_tree(&place, false, None);
            self.moved_places
                .retain(|moved| !Self::place_projection_is_prefix(&place, moved));
        }
        self.moved_places.insert(place.clone());
        self.statically_moved_cleanup_places.insert(place);
    }

    fn place_may_be_moved(&self, place: &Place) -> bool {
        self.moved_places
            .iter()
            .any(|moved| Self::place_projection_is_prefix(moved, place))
    }

    fn place_was_statically_moved_for_cleanup(&self, place: &Place) -> bool {
        self.statically_moved_cleanup_places
            .iter()
            .any(|moved| Self::place_projection_is_prefix(moved, place))
    }

    fn place_has_moved_descendant(&self, place: &Place) -> bool {
        self.moved_places
            .iter()
            .any(|moved| Self::place_projection_is_strict_prefix(place, moved))
    }

    fn place_projection_is_prefix(prefix: &Place, place: &Place) -> bool {
        prefix.local == place.local
            && Self::projection_is_prefix(&prefix.projection, &place.projection)
    }

    fn place_projection_is_strict_prefix(prefix: &Place, place: &Place) -> bool {
        prefix.local == place.local
            && prefix.projection.len() < place.projection.len()
            && Self::projection_is_prefix(&prefix.projection, &place.projection)
    }

    fn projection_is_prefix(prefix: &[Projection], projection: &[Projection]) -> bool {
        prefix.len() <= projection.len()
            && prefix
                .iter()
                .zip(projection.iter())
                .all(|(lhs, rhs)| Self::projection_component_matches(lhs, rhs))
    }

    fn projection_component_matches(lhs: &Projection, rhs: &Projection) -> bool {
        match (lhs, rhs) {
            (Projection::Deref, Projection::Deref) => true,
            (Projection::Downcast(lhs), Projection::Downcast(rhs)) => lhs == rhs,
            (Projection::Index(lhs), Projection::Index(rhs)) => lhs == rhs,
            (Projection::Field { index: lhs, .. }, Projection::Field { index: rhs, .. }) => {
                lhs == rhs
            }
            _ => false,
        }
    }

    fn move_source_place_for_expr(&self, expr: &HirExpr) -> Option<Place> {
        match &expr.kind {
            HirExprKind::Var(name) => self.var_map.get(name).copied().map(|local| Place {
                local,
                projection: vec![],
            }),
            HirExprKind::ResolvedVar(reference) => {
                self.var_map
                    .get(&reference.name)
                    .copied()
                    .map(|local| Place {
                        local,
                        projection: vec![],
                    })
            }
            HirExprKind::FieldAccess(base, _, location) => {
                let mut place = self.move_source_place_for_expr(base)?;
                let location = location.as_ref()?;
                let Type::Struct { id, .. } = &base.ty else {
                    return None;
                };
                let (_, structure) = self.program.struct_by_id(*id)?;
                let index = structure
                    .fields
                    .iter()
                    .position(|field| field.id == location.field_id)?;
                place.projection.push(self.field_projection(
                    index,
                    Some(location.owner),
                    Some(location.field_id),
                ));
                Some(place)
            }
            HirExprKind::TupleIndex(base, index) => {
                let mut place = self.move_source_place_for_expr(base)?;
                place
                    .projection
                    .push(self.field_projection(*index as usize, None, None));
                Some(place)
            }
            _ => None,
        }
    }

    fn record_rvalue_moves(&mut self, rvalue: &Rvalue) {
        match rvalue {
            Rvalue::Use(operand) | Rvalue::Cast(operand, _) | Rvalue::UnaryOp(_, operand) => {
                self.record_operand_move(operand);
            }
            Rvalue::BinaryOp(_, lhs, rhs) => {
                self.record_operand_move(lhs);
                self.record_operand_move(rhs);
            }
            Rvalue::Aggregate(_, operands) => {
                for operand in operands {
                    self.record_operand_move(operand);
                }
            }
            Rvalue::Closure(closure) => {
                for capture in &closure.captures {
                    if capture.kind == MirClosureCaptureKind::ByValue
                        && capture.place().projection.is_empty()
                    {
                        self.set_drop_flag(capture.local, false, capture.span.clone());
                    }
                }
            }
            Rvalue::Ref(_, _) | Rvalue::Discriminant(_) => {}
        }
    }

    fn record_terminator_moves(&mut self, terminator: &Terminator) {
        if let Terminator::Call { func, args, .. } = terminator {
            self.record_operand_move(func);
            for arg in args {
                self.record_operand_move(arg);
            }
        }
    }

    fn set_terminator(&mut self, block: BasicBlockId, terminator: Terminator) {
        self.record_terminator_moves(&terminator);
        self.blocks[block.0].terminator = Some(terminator);
    }

    fn ensure_drop_flag(&mut self, local: Local) -> Option<Local> {
        if let Some(flag) = self.drop_flags.get(&local).copied() {
            return Some(flag);
        }

        let ty = self.get_local_type(local)?;
        if !self.type_needs_cleanup(&ty) {
            return None;
        }

        let flag = Local(self.locals.len());
        self.locals.push(LocalDecl {
            ty: self.type_id_for(&Type::Bool),
            mutability: Mutability::Mut,
            name: Some(format!("drop_flag_{}", local.0)),
            span: self.locals.get(local.0).and_then(|decl| decl.span.clone()),
            source: LocalSource::Temporary,
        });
        self.record_temporary_local(flag);
        self.drop_flags.insert(local, flag);
        self.ensure_projection_drop_flags(local, &ty);
        Some(flag)
    }

    fn ensure_projection_drop_flags(&mut self, local: Local, ty: &Type) {
        let mut entries = Vec::new();
        self.collect_projection_drop_flag_entries(ty, Vec::new(), &mut entries);
        for (projection, _) in entries {
            let place = Place { local, projection };
            if self.projection_drop_flags.contains_key(&place) {
                continue;
            }
            let flag = Local(self.locals.len());
            self.locals.push(LocalDecl {
                ty: self.type_id_for(&Type::Bool),
                mutability: Mutability::Mut,
                name: Some(format!("drop_flag_{}_{}", local.0, flag.0)),
                span: self.locals.get(local.0).and_then(|decl| decl.span.clone()),
                source: LocalSource::Temporary,
            });
            self.record_temporary_local(flag);
            self.projection_drop_flags.insert(place, flag);
        }
    }

    fn collect_projection_drop_flag_entries(
        &self,
        ty: &Type,
        prefix: Vec<Projection>,
        entries: &mut Vec<(Vec<Projection>, Type)>,
    ) {
        match ty {
            Type::Struct { id, args } => {
                for (index, (_field_id, field_ty)) in
                    Self::struct_field_types_with_substitution(self.program, *id, args)
                        .into_iter()
                        .enumerate()
                {
                    if !self.type_needs_cleanup(&field_ty) {
                        continue;
                    }
                    let mut projection = prefix.clone();
                    projection.push(Projection::Field {
                        index,
                        identity: None,
                    });
                    entries.push((projection.clone(), field_ty.clone()));
                    self.collect_projection_drop_flag_entries(&field_ty, projection, entries);
                }
            }
            Type::Tuple(elems) => {
                for (index, field_ty) in elems.iter().enumerate() {
                    if !self.type_needs_cleanup(field_ty) {
                        continue;
                    }
                    let mut projection = prefix.clone();
                    projection.push(Projection::Field {
                        index,
                        identity: None,
                    });
                    entries.push((projection.clone(), field_ty.clone()));
                    self.collect_projection_drop_flag_entries(field_ty, projection, entries);
                }
            }
            Type::Enum { id, args: _ } => {
                let Some((_, enumeration)) = self.program.enum_by_id(*id) else {
                    return;
                };
                let generic_subst = Self::generic_substitution_for_fields(
                    &enumeration
                        .variants
                        .iter()
                        .flat_map(|variant| {
                            Self::enum_variant_field_types_for(self.program, *id, variant.id)
                        })
                        .collect::<Vec<_>>(),
                    ty,
                );
                for variant in &enumeration.variants {
                    for (index, field_ty) in
                        Self::enum_variant_field_types_for(self.program, *id, variant.id)
                            .into_iter()
                            .map(|field_ty| field_ty.substitute_generics(&generic_subst))
                            .enumerate()
                    {
                        if !self.type_needs_cleanup(&field_ty) {
                            continue;
                        }
                        let mut projection = prefix.clone();
                        projection.push(Projection::Downcast(variant.id));
                        projection.push(Projection::Field {
                            index,
                            identity: None,
                        });
                        entries.push((projection.clone(), field_ty.clone()));
                        self.collect_projection_drop_flag_entries(&field_ty, projection, entries);
                    }
                }
            }
            Type::Array(_, _) => {}
            _ => {}
        }
    }

    fn drop_flag_for(&self, local: Local) -> Option<Local> {
        self.drop_flags.get(&local).copied()
    }

    fn drop_flag_for_place(&self, place: &Place) -> Option<Local> {
        if place.projection.is_empty() {
            self.drop_flag_for(place.local)
        } else {
            self.projection_drop_flags
                .iter()
                .find_map(|(candidate, flag)| {
                    (candidate.local == place.local
                        && candidate.projection.len() == place.projection.len()
                        && Self::projection_is_prefix(&candidate.projection, &place.projection))
                    .then_some(*flag)
                })
        }
    }

    fn projection_drop_flags_for_local(&self, local: Local) -> Vec<Local> {
        let mut flags = self
            .projection_drop_flags
            .iter()
            .filter_map(|(place, flag)| (place.local == local).then_some(*flag))
            .collect::<Vec<_>>();
        flags.sort_by_key(|flag| flag.0);
        flags
    }

    fn set_drop_flag_for_place(&mut self, place: &Place, initialized: bool, span: Option<Span>) {
        let Some(flag) = self.drop_flag_for_place(place) else {
            return;
        };
        self.set_drop_flag_local(flag, initialized, span);
    }

    fn set_drop_flag(&mut self, local: Local, initialized: bool, span: Option<Span>) {
        let Some(flag) = self.drop_flag_for(local) else {
            return;
        };
        self.set_drop_flag_local(flag, initialized, span);
    }

    fn set_projection_drop_flags_for_local(
        &mut self,
        local: Local,
        initialized: bool,
        span: Option<Span>,
    ) {
        for flag in self.projection_drop_flags_for_local(local) {
            self.set_drop_flag_local(flag, initialized, span.clone());
        }
    }

    fn set_drop_flags_for_place_tree(
        &mut self,
        place: &Place,
        initialized: bool,
        span: Option<Span>,
    ) {
        if place.projection.is_empty() {
            self.set_drop_flag(place.local, initialized, span.clone());
            self.set_projection_drop_flags_for_local(place.local, initialized, span);
            return;
        }

        let mut flags = self
            .projection_drop_flags
            .iter()
            .filter_map(|(candidate, flag)| {
                Self::place_projection_is_prefix(place, candidate).then_some(*flag)
            })
            .collect::<Vec<_>>();
        flags.sort_by_key(|flag| flag.0);
        for flag in flags {
            self.set_drop_flag_local(flag, initialized, span.clone());
        }
    }

    fn set_drop_flag_local(&mut self, flag: Local, initialized: bool, span: Option<Span>) {
        if let Some(current) = self.current_block {
            self.blocks[current.0]
                .statements
                .push(StatementData::assign(
                    Place {
                        local: flag,
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Constant(Constant::Bool(initialized))),
                    span,
                ));
        }
    }

    fn mark_place_initialized(&mut self, place: &Place, span: Option<Span>) {
        let Some(ty) = self.get_local_type(place.local) else {
            return;
        };
        if self.type_needs_cleanup(&ty) {
            self.set_drop_flags_for_place_tree(place, true, span);
        }
    }

    fn emit_assert(&mut self, assertion: crate::mir::MirAssert, span: Option<Span>) {
        if let Some(current) = self.current_block {
            self.blocks[current.0]
                .statements
                .push(StatementData::assert(assertion, span));
        }
    }

    fn index_base_needs_bounds_check(base_ty: &Type) -> bool {
        !matches!(
            base_ty,
            Type::Pointer(inner) if !matches!(inner.as_ref(), Type::Slice(_) | Type::Str)
        )
    }

    fn emit_bounds_check_for_index_place(
        &mut self,
        indexed_place: &Place,
        base_ty: &Type,
        span: Option<Span>,
    ) {
        if !Self::index_base_needs_bounds_check(base_ty) {
            return;
        }

        let Some(Projection::Index(index_local)) = indexed_place.projection.last().cloned() else {
            return;
        };
        let mut base_place = indexed_place.clone();
        base_place.projection.pop();
        self.emit_assert(
            crate::mir::MirAssert {
                kind: crate::mir::MirAssertKind::BoundsCheck,
                operands: vec![
                    Operand::Copy(base_place),
                    Operand::Copy(Place {
                        local: index_local,
                        projection: vec![],
                    }),
                ],
            },
            span,
        );
    }

    fn emit_storage_live(&mut self, local: Local, span: Option<Span>) {
        let flag = self.ensure_drop_flag(local);
        let projection_flags = self.projection_drop_flags_for_local(local);
        if let Some(current) = self.current_block {
            self.blocks[current.0]
                .statements
                .push(StatementData::storage_live(local, span.clone()));
            if let Some(flag) = flag {
                self.blocks[current.0]
                    .statements
                    .push(StatementData::storage_live(flag, span.clone()));
                self.blocks[current.0]
                    .statements
                    .push(StatementData::assign(
                        Place {
                            local: flag,
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Constant(Constant::Bool(false))),
                        span.clone(),
                    ));
            }
            for projection_flag in projection_flags {
                self.blocks[current.0]
                    .statements
                    .push(StatementData::storage_live(projection_flag, span.clone()));
                self.blocks[current.0]
                    .statements
                    .push(StatementData::assign(
                        Place {
                            local: projection_flag,
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Constant(Constant::Bool(false))),
                        span.clone(),
                    ));
            }
        }
    }

    fn register_scoped_temp(&mut self, local: Local) {
        if let Some(scope_locals) = self.scope_locals.last_mut() {
            scope_locals.push(local);
        }
    }

    fn build_function(
        &mut self,
        id: MirFunctionId,
        display_name: &str,
        func: &HirFunction,
        skip_self_param_cleanup: bool,
        source_owner: Option<DefId>,
        operation_span: Option<Span>,
    ) -> MirFunction {
        self.current_function_id = Some(id.clone());
        self.current_function_name = Some(display_name.to_string());
        self.current_source_owner = source_owner;
        self.drop_flags.clear();
        self.projection_drop_flags.clear();
        self.moved_places.clear();
        self.statically_moved_cleanup_places.clear();
        self.ownership = MirOwnershipMetadata::default();
        let entry_block = self.new_block();
        self.current_block = Some(entry_block);

        let return_span = self.source_operation_span(source_owner, operation_span.clone());
        self.new_local_with_source(
            self.type_id_for(&func.ret_type),
            Mutability::Mut,
            Some("return_place".to_string()),
            LocalSource::ReturnPlace,
            return_span,
        );

        let mut function_scope_locals = Vec::new();
        let mut cleanup_param_locals = Vec::new();
        for (index, param) in func.params.iter().enumerate() {
            let mutability = if param.mutable {
                Mutability::Mut
            } else {
                Mutability::Not
            };
            let param_span = self.source_local_span(source_owner, param.local_id);
            let local = self.new_local_with_source(
                self.type_id_for(&param.ty),
                mutability,
                Some(param.name.clone()),
                LocalSource::Argument,
                param_span,
            );
            self.var_map.insert(param.name.clone(), local);
            let borrowed_self_param = index == 0
                && matches!(
                    func.self_receiver,
                    Some(crate::types::ReceiverMode::Shared | crate::types::ReceiverMode::Mut)
                );
            if borrowed_self_param || matches!(param.ty, Type::Reference { .. }) {
                self.record_reference_origin(local, ReferenceOrigin::Param(local));
            }
            if !(skip_self_param_cleanup && index == 0) && !borrowed_self_param {
                function_scope_locals.push(local);
                cleanup_param_locals.push(local);
            }
        }

        for local in cleanup_param_locals {
            self.ensure_drop_flag(local);
            self.mark_place_initialized(
                &Place {
                    local,
                    projection: vec![],
                },
                None,
            );
        }

        for capture in std::mem::take(&mut self.pending_closure_body_captures) {
            let mutability = if capture.mutable
                || capture.kind == crate::hir::HirClosureCaptureKind::MutableBorrow
            {
                Mutability::Mut
            } else {
                Mutability::Not
            };
            let capture_ty = match capture.kind {
                crate::hir::HirClosureCaptureKind::Move => capture.ty.clone(),
                crate::hir::HirClosureCaptureKind::SharedBorrow => Type::Reference {
                    mutable: false,
                    inner: Box::new(capture.ty.clone()),
                },
                crate::hir::HirClosureCaptureKind::MutableBorrow => Type::Reference {
                    mutable: true,
                    inner: Box::new(capture.ty.clone()),
                },
            };
            let capture_span = self
                .source_local_span(source_owner, capture.local_id)
                .or_else(|| operation_span.clone());
            let local = self.new_local_with_source(
                self.type_id_for(&capture_ty),
                mutability,
                Some(capture.name.clone()),
                LocalSource::ClosureCapture,
                capture_span.clone(),
            );
            self.var_map.insert(capture.name.clone(), local);
            self.closure_captures.push(MirClosureCapture {
                name: capture.name,
                local,
                kind: MirClosureCaptureKind::from(capture.kind),
                span: capture_span,
            });
        }

        let ret_place = Place {
            local: Local(0),
            projection: vec![],
        };

        self.scope_locals.push(function_scope_locals);
        self.lower_block(&func.body, ret_place);
        let function_scope_locals = self.scope_locals.pop().unwrap_or_default();
        self.finish_scope_locals(function_scope_locals);

        if let Some(current) = self.current_block {
            let implicit_return_origin = self.source_origin(
                self.source_operation_span(self.current_source_owner, operation_span.clone()),
            );
            let block = &mut self.blocks[current.0];
            if block.terminator.is_none() {
                block.terminator = Some(super::Terminator::return_with_origin(
                    implicit_return_origin,
                ));
            }
        }

        MirFunction {
            id,
            name: display_name.to_string(),
            basic_blocks: std::mem::take(&mut self.blocks),
            local_decls: std::mem::take(&mut self.locals),
            closure_captures: std::mem::take(&mut self.closure_captures),
            arg_count: func.params.len(),
            ret_type: self.type_id_for(&func.ret_type),
            ownership: std::mem::take(&mut self.ownership),
        }
    }

    #[cfg(test)]
    fn needs_drop(ty: &Type) -> bool {
        match ty {
            Type::Array(_, _) => true,
            Type::Struct { .. } => true,
            Type::Enum { .. } => true,
            Type::Tuple(elems) => elems.iter().any(Self::needs_drop),
            Type::Function { .. } => true,
            _ => false,
        }
    }

    fn struct_field_types_with_substitution(
        program: &HirProgram,
        def_id: crate::ids::DefId,
        generic_args: &[Type],
    ) -> Vec<(crate::ids::FieldId, Type)> {
        let Some((_, structure)) = program.struct_by_id(def_id) else {
            return Vec::new();
        };
        let generic_subst = Self::generic_substitution_for_fields(
            &structure
                .fields
                .iter()
                .map(|f| f.ty.clone())
                .collect::<Vec<_>>(),
            &Type::Struct {
                id: def_id,
                args: generic_args.to_vec(),
            },
        );
        structure
            .fields
            .iter()
            .map(|field| {
                let field_ty = field.ty.substitute_generics(&generic_subst);
                (field.id, field_ty)
            })
            .collect()
    }

    fn lower_place(&mut self, expr: &HirExpr) -> Option<Place> {
        match &expr.kind {
            HirExprKind::Var(name) => {
                if let Some(local) = self.var_map.get(name) {
                    Some(self.place_for_local_expr(*local, &expr.ty))
                } else {
                    None
                }
            }
            HirExprKind::ResolvedVar(reference)
                if matches!(reference.target, HirVarTarget::Local(_)) =>
            {
                self.var_map
                    .get(&reference.name)
                    .map(|local| self.place_for_local_expr(*local, &expr.ty))
            }
            HirExprKind::Deref(base) => {
                let mut place = if let Some(place) = self.lower_place(base) {
                    place
                } else {
                    let base_temp = self.new_local_from_expr(base.ty.clone(), base);
                    self.register_scoped_temp(base_temp);
                    self.emit_storage_live(base_temp, Some(base.span.clone()));
                    let base_place = Place {
                        local: base_temp,
                        projection: vec![],
                    };
                    self.lower_expr(base, base_place.clone());
                    base_place
                };
                place.projection.push(Projection::Deref);
                Some(place)
            }
            HirExprKind::FieldAccess(base, field_name, location) => {
                let mut place = self.lower_place(base)?;
                let struct_id = location
                    .as_ref()
                    .map(|location| location.owner)
                    .or_else(|| match &base.ty {
                        Type::Struct { id, .. } => Some(*id),
                        _ => None,
                    })?;
                let (_, fields) = self.program.struct_by_id(struct_id)?;
                let location = location.as_ref().filter(|location| {
                    location.owner == fields.id
                        && fields.fields.iter().any(|field| {
                            field.id == location.field_id
                                && field.name == location.name
                                && field.name == *field_name
                        })
                })?;
                let field_idx = fields
                    .fields
                    .iter()
                    .position(|field| field.id == location.field_id)?;
                place.projection.push(self.field_projection(
                    field_idx,
                    Some(location.owner),
                    Some(location.field_id),
                ));
                Some(place)
            }
            HirExprKind::TupleIndex(base, idx) => {
                let mut place = self.lower_place(base)?;
                match &base.ty {
                    Type::Tuple(elems) if (*idx as usize) < elems.len() => {
                        place
                            .projection
                            .push(self.field_projection(*idx as usize, None, None));
                        Some(place)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn operand_for_place(&self, ty: &Type, place: Place, borrow_context: bool) -> Operand {
        if borrow_context {
            if matches!(ty, Type::Reference { mutable: true, .. }) {
                Operand::Move(place)
            } else {
                Operand::Copy(place)
            }
        } else if matches!(
            self.get_local_type(place.local),
            Some(Type::Reference { mutable: false, .. })
        ) && matches!(place.projection.first(), Some(Projection::Deref))
            && !self.type_needs_cleanup(ty)
        {
            Operand::Copy(place)
        } else if Self::needs_move(ty) {
            Operand::Move(place)
        } else {
            Operand::Copy(place)
        }
    }

    fn enum_variant_field_types(
        &self,
        enum_id: crate::ids::DefId,
        variant_id: VariantId,
    ) -> Vec<Type> {
        Self::enum_variant_field_types_for(self.program, enum_id, variant_id)
    }

    fn enum_variant_field_types_for(
        program: &HirProgram,
        enum_id: crate::ids::DefId,
        variant_id: VariantId,
    ) -> Vec<Type> {
        let Some((_, enumeration)) = program.enum_by_id(enum_id) else {
            return Vec::new();
        };
        let Some(variant) = enumeration
            .variants
            .iter()
            .find(|variant| variant.id == variant_id)
        else {
            return Vec::new();
        };
        match &variant.fields {
            HirVariantFields::Named(fields) => {
                fields.iter().map(|field| field.ty.clone()).collect()
            }
            HirVariantFields::Positional(fields) => fields.clone(),
            HirVariantFields::Unit => Vec::new(),
        }
    }

    fn bind_match_pattern_places(
        &mut self,
        pattern: &HirPattern,
        source_place: Place,
        source_ty: &Type,
        guarded: bool,
        enum_payload: bool,
        scrutinee_mode: MatchScrutineeMode,
        moved_enum_scrutinee: &mut Option<Local>,
        restored_bindings: &mut Vec<(String, Option<Local>)>,
    ) {
        match pattern {
            HirPattern::Binding { name, mutable, .. } => {
                let borrowed_payload =
                    enum_payload && scrutinee_mode == MatchScrutineeMode::Borrowed;
                if enum_payload && !borrowed_payload && !TypeFacts::is_copy(source_ty) {
                    if guarded {
                        return;
                    }
                    *moved_enum_scrutinee = Some(source_place.local);
                }
                let mutability = if *mutable {
                    Mutability::Mut
                } else {
                    Mutability::Not
                };
                let binding_ty = if borrowed_payload {
                    Type::Reference {
                        mutable: false,
                        inner: Box::new(source_ty.clone()),
                    }
                } else {
                    source_ty.clone()
                };
                let local = self.new_local_with_source(
                    self.type_id_for(&binding_ty),
                    mutability,
                    Some(name.clone()),
                    LocalSource::UserBinding,
                    None,
                );
                self.emit_storage_live(local, None);
                if let Some(scope_locals) = self.scope_locals.last_mut() {
                    scope_locals.push(local);
                }
                restored_bindings.push((name.clone(), self.var_map.insert(name.clone(), local)));
                let dest = Place {
                    local,
                    projection: vec![],
                };
                let rvalue = if borrowed_payload {
                    Rvalue::Ref(Mutability::Not, source_place)
                } else {
                    Rvalue::Use(self.operand_for_place(source_ty, source_place, guarded))
                };
                self.emit_assign(dest, rvalue, None);
            }
            HirPattern::Enum(_, _, Some(location), subpatterns) => {
                let field_types =
                    self.enum_variant_field_types(location.owner, location.variant_id);
                let generic_subst = match source_ty {
                    Type::Enum { args, .. } => {
                        let mut generic_params = std::collections::HashSet::new();
                        for field_ty in &field_types {
                            field_ty.collect_generic_params(&mut generic_params);
                        }
                        generic_params
                            .into_iter()
                            .filter_map(|param| {
                                args.get(param.index as usize)
                                    .cloned()
                                    .map(|arg| (param, arg))
                            })
                            .collect::<HashMap<_, _>>()
                    }
                    _ => HashMap::new(),
                };
                for (index, subpattern) in subpatterns.iter().enumerate() {
                    let mut field_place = source_place.clone();
                    field_place
                        .projection
                        .push(Projection::Downcast(location.variant_id));
                    field_place.projection.push(Projection::Field {
                        index,
                        identity: None,
                    });
                    let field_ty = field_types
                        .get(index)
                        .map(|ty| ty.substitute_generics(&generic_subst))
                        .unwrap_or(Type::Unit);
                    self.bind_match_pattern_places(
                        subpattern,
                        field_place,
                        &field_ty,
                        guarded,
                        true,
                        scrutinee_mode,
                        moved_enum_scrutinee,
                        restored_bindings,
                    );
                }
            }
            HirPattern::Tuple(subpatterns) => {
                let field_types = match source_ty {
                    Type::Tuple(fields) => fields.as_slice(),
                    _ => &[],
                };
                for (index, subpattern) in subpatterns.iter().enumerate() {
                    let mut field_place = source_place.clone();
                    field_place.projection.push(Projection::Field {
                        index,
                        identity: None,
                    });
                    let field_ty = field_types.get(index).unwrap_or(&Type::Unit);
                    self.bind_match_pattern_places(
                        subpattern,
                        field_place,
                        field_ty,
                        guarded,
                        enum_payload,
                        scrutinee_mode,
                        moved_enum_scrutinee,
                        restored_bindings,
                    );
                }
            }
            HirPattern::Struct(_, Some(struct_id), _, fields) => {
                let Some((_, structure)) = self.program.struct_by_id(*struct_id) else {
                    return;
                };
                let generic_subst = match source_ty {
                    Type::Struct { args, .. } => {
                        let mut generic_params = std::collections::HashSet::new();
                        for field in &structure.fields {
                            field.ty.collect_generic_params(&mut generic_params);
                        }
                        generic_params
                            .into_iter()
                            .filter_map(|param| {
                                args.get(param.index as usize)
                                    .cloned()
                                    .map(|arg| (param, arg))
                            })
                            .collect::<HashMap<_, _>>()
                    }
                    _ => HashMap::new(),
                };
                for field in fields {
                    let Some((index, hir_field)) =
                        structure.fields.iter().enumerate().find(|(_, hir_field)| {
                            field.field.as_ref().is_some_and(|location| {
                                location.owner == *struct_id
                                    && location.field_id == hir_field.id
                                    && location.name == hir_field.name
                            }) || hir_field.name == field.name
                        })
                    else {
                        continue;
                    };
                    let identity = field.field.as_ref().filter(|location| {
                        location.owner == *struct_id
                            && location.field_id == hir_field.id
                            && location.name == hir_field.name
                    });
                    let mut field_place = source_place.clone();
                    field_place.projection.push(self.field_projection(
                        index,
                        identity.map(|location| location.owner),
                        identity.map(|location| location.field_id),
                    ));
                    self.bind_match_pattern_places(
                        &field.pattern,
                        field_place,
                        &hir_field.ty.substitute_generics(&generic_subst),
                        guarded,
                        enum_payload,
                        scrutinee_mode,
                        moved_enum_scrutinee,
                        restored_bindings,
                    );
                }
            }
            HirPattern::Wildcard
            | HirPattern::Literal(_)
            | HirPattern::Or(_)
            | HirPattern::Enum(_, _, None, _)
            | HirPattern::Struct(_, None, _, _) => {}
        }
    }

    fn match_arm_variant_id(&self, pattern: &HirPattern) -> Option<VariantId> {
        match pattern {
            HirPattern::Enum(_, _, Some(location), _) => Some(location.variant_id),
            _ => None,
        }
    }

    fn enum_payload_patterns_irrefutable(pattern: &HirPattern) -> bool {
        let HirPattern::Enum(_, _, Some(_), subpatterns) = pattern else {
            return false;
        };

        subpatterns
            .iter()
            .all(Self::pattern_is_irrefutable_for_struct_match)
    }

    fn enum_payload_pattern_check_supported(pattern: &HirPattern) -> bool {
        match pattern {
            HirPattern::Literal(
                HirLiteralPattern::Int(_) | HirLiteralPattern::Bool(_) | HirLiteralPattern::Char(_),
            ) => true,
            HirPattern::Literal(HirLiteralPattern::Float(_) | HirLiteralPattern::String(_)) => {
                false
            }
            HirPattern::Enum(_, _, Some(_), _) => Self::enum_payload_patterns_irrefutable(pattern),
            _ => Self::pattern_is_irrefutable_for_struct_match(pattern),
        }
    }

    fn assert_enum_payload_patterns_supported(&self, pattern: &HirPattern) {
        let HirPattern::Enum(_, _, Some(_), subpatterns) = pattern else {
            return;
        };

        if subpatterns
            .iter()
            .any(|pattern| !Self::enum_payload_pattern_check_supported(pattern))
        {
            panic!("unsupported refutable enum payload pattern: {pattern:?}");
        }
    }

    fn match_arm_is_catch_all(&self, pattern: &HirPattern) -> bool {
        matches!(pattern, HirPattern::Wildcard | HirPattern::Binding { .. })
    }

    fn pattern_is_irrefutable_for_struct_match(pattern: &HirPattern) -> bool {
        match pattern {
            HirPattern::Wildcard | HirPattern::Binding { .. } => true,
            HirPattern::Tuple(fields) => fields
                .iter()
                .all(Self::pattern_is_irrefutable_for_struct_match),
            HirPattern::Struct(_, _, _, fields) => fields
                .iter()
                .all(|field| Self::pattern_is_irrefutable_for_struct_match(&field.pattern)),
            HirPattern::Literal(_) | HirPattern::Enum(_, _, _, _) | HirPattern::Or(_) => false,
        }
    }

    fn lower_match_pattern_check(
        &mut self,
        pattern: &HirPattern,
        source_place: Place,
        source_ty: &Type,
        success: BasicBlockId,
        failure: BasicBlockId,
        origin_span: Option<Span>,
    ) {
        match pattern {
            HirPattern::Wildcard | HirPattern::Binding { .. } => {
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(super::Terminator::goto(
                        success,
                        self.source_origin(origin_span.clone()),
                    ));
                }
            }
            HirPattern::Enum(_, _, Some(location), _) => {
                let discr_temp =
                    self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
                self.emit_storage_live(discr_temp, None);
                let discr_place = Place {
                    local: discr_temp,
                    projection: vec![],
                };
                self.emit_assign(
                    discr_place.clone(),
                    Rvalue::Discriminant(source_place),
                    None,
                );
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(super::Terminator::switch_int(
                        Operand::Copy(discr_place),
                        vec![(location.variant_id.0 as i64, success)],
                        failure,
                        self.source_origin(origin_span.clone()),
                    ));
                }
            }
            HirPattern::Literal(literal) => {
                let bool_temp =
                    self.new_local(self.type_id_for(&Type::Bool), Mutability::Not, None);
                self.emit_storage_live(bool_temp, None);
                let bool_place = Place {
                    local: bool_temp,
                    projection: vec![],
                };
                let literal_operand = match literal {
                    HirLiteralPattern::Int(value) => {
                        Operand::Constant(super::Constant::Int(*value))
                    }
                    HirLiteralPattern::Float(value) => {
                        Operand::Constant(super::Constant::Float(*value))
                    }
                    HirLiteralPattern::Bool(value) => {
                        Operand::Constant(super::Constant::Bool(*value))
                    }
                    HirLiteralPattern::String(value) => {
                        Operand::Constant(super::Constant::String(value.clone()))
                    }
                    HirLiteralPattern::Char(value) => {
                        Operand::Constant(super::Constant::Char(*value))
                    }
                };
                self.emit_assign(
                    bool_place.clone(),
                    Rvalue::BinaryOp(
                        MirBinOp::Eq,
                        self.operand_for_place(source_ty, source_place, true),
                        literal_operand,
                    ),
                    None,
                );
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(super::Terminator::switch_int(
                        Operand::Copy(bool_place),
                        vec![(1, success)],
                        failure,
                        self.source_origin(origin_span.clone()),
                    ));
                }
            }
            HirPattern::Tuple(_) | HirPattern::Struct(_, _, _, _)
                if Self::pattern_is_irrefutable_for_struct_match(pattern) =>
            {
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(super::Terminator::goto(
                        success,
                        super::MirOrigin::synthetic(super::MirSyntheticOrigin::ControlFlow, None),
                    ));
                }
            }
            HirPattern::Tuple(_)
            | HirPattern::Struct(_, _, _, _)
            | HirPattern::Or(_)
            | HirPattern::Enum(_, _, None, _) => {
                panic!("unsupported refutable MIR match pattern: {pattern:?}");
            }
        }
    }

    fn lower_enum_payload_pattern_checks(
        &mut self,
        pattern: &HirPattern,
        source_place: Place,
        source_ty: &Type,
        success: BasicBlockId,
        failure: BasicBlockId,
        origin_span: Option<Span>,
    ) {
        let HirPattern::Enum(_, _, Some(location), subpatterns) = pattern else {
            if let Some(current) = self.current_block {
                self.blocks[current.0].terminator = Some(super::Terminator::goto(
                    success,
                    self.source_origin(origin_span.clone()),
                ));
            }
            return;
        };

        self.assert_enum_payload_patterns_supported(pattern);

        if subpatterns.is_empty() {
            if let Some(current) = self.current_block {
                self.blocks[current.0].terminator = Some(super::Terminator::goto(
                    success,
                    self.source_origin(origin_span.clone()),
                ));
            }
            return;
        }

        let field_types = self.enum_variant_field_types(location.owner, location.variant_id);
        let generic_subst = match source_ty {
            Type::Enum { args, .. } => {
                let mut generic_params = std::collections::HashSet::new();
                for field_ty in &field_types {
                    field_ty.collect_generic_params(&mut generic_params);
                }
                generic_params
                    .into_iter()
                    .filter_map(|param| {
                        args.get(param.index as usize)
                            .cloned()
                            .map(|arg| (param, arg))
                    })
                    .collect::<HashMap<_, _>>()
            }
            _ => HashMap::new(),
        };

        for (index, subpattern) in subpatterns.iter().enumerate() {
            let next_success = if index + 1 == subpatterns.len() {
                success
            } else {
                self.new_block()
            };
            let mut field_place = source_place.clone();
            field_place
                .projection
                .push(Projection::Downcast(location.variant_id));
            field_place.projection.push(Projection::Field {
                index,
                identity: None,
            });
            let field_ty = field_types
                .get(index)
                .map(|ty| ty.substitute_generics(&generic_subst))
                .unwrap_or(Type::Unit);
            self.lower_match_pattern_check(
                subpattern,
                field_place,
                &field_ty,
                next_success,
                failure,
                origin_span.clone(),
            );
            if index + 1 < subpatterns.len() {
                self.current_block = Some(next_success);
            }
        }
    }

    fn restore_match_bindings(&mut self, restored_bindings: Vec<(String, Option<Local>)>) {
        for (name, previous) in restored_bindings.into_iter().rev() {
            if let Some(previous) = previous {
                self.var_map.insert(name, previous);
            } else {
                self.var_map.remove(&name);
            }
        }
    }

    fn lower_expr(&mut self, expr: &HirExpr, dest: Place) {
        self.lower_expr_with_context(expr, dest, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{
        BinOp, HirAssociatedTypeDef, HirExtern, HirField, HirImplOwner, HirImplReceiverPattern,
        HirLiteralPattern, HirNameTables, HirParam, HirPattern, HirStruct, HirVarRef, HirVarTarget,
        HirVariantFields, HirVariantLocation, UnaryOp,
    };
    use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, InstanceId, LocalDefId, VariantId};
    use crate::lexer::Span;
    use crate::mir::{AggregateKind, Constant, MirCallableKey, Operand, StatementKind, Terminator};
    use crate::types::{GenericParamDecl, GenericParamId, Type};

    type HirMatchArm = crate::hir::HirMatchArmFor<AcceptedHir>;
    type HirStructLiteralField = crate::hir::HirStructLiteralFieldFor<AcceptedHir>;
    type HirTrait = crate::hir::HirTraitFor<AcceptedHir>;

    fn span() -> Span {
        Span::test()
    }

    fn expr(kind: HirExprKind, ty: Type) -> HirExpr {
        HirExpr {
            kind,
            ty,
            span: span(),
        }
    }

    fn test_type_context(program: &HirProgram) -> RefCell<crate::type_context::TypeContext> {
        let mut type_context = crate::type_context::TypeContext::new();
        let _ = crate::hir::collect_hir_type_ids(program, &mut type_context);
        for ty in [
            Type::Unit,
            Type::I64,
            Type::I32,
            Type::U64,
            Type::F64,
            Type::Bool,
        ] {
            type_context.intern_type(&ty);
        }
        RefCell::new(type_context)
    }

    fn empty_program() -> HirProgram {
        let id = DefId::new(CrateId(0), LocalDefId(0));
        HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(
                id,
                HirStruct {
                    id,
                    name: "TestStruct".to_string(),
                    generic_params: vec![],
                    fields: vec![],
                },
            )]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                structs_by_name: std::collections::HashMap::from([("TestStruct".to_string(), id)]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(id, "TestStruct".to_string())]),
        )
    }

    #[test]
    fn binary_op_lowers_to_mir_owned_operator() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let add = expr(
            HirExprKind::BinOp(
                BinOp::Add,
                Box::new(expr(HirExprKind::IntLiteral(1), Type::I64)),
                Box::new(expr(HirExprKind::IntLiteral(2), Type::I64)),
            ),
            Type::I64,
        );

        builder.lower_expr(
            &add,
            Place {
                local: Local(0),
                projection: vec![],
            },
        );

        assert!(builder.blocks[0]
            .statements
            .iter()
            .any(|statement| matches!(
                &statement.kind,
                StatementKind::Assign(
                    Place {
                        local: Local(0),
                        projection,
                    },
                    Rvalue::BinaryOp(crate::mir::MirBinOp::Add, _, _),
                ) if projection.is_empty()
            )));
    }

    #[test]
    fn unary_op_lowers_to_mir_owned_operator() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::Bool),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let not = expr(
            HirExprKind::UnaryOp(
                UnaryOp::Not,
                Box::new(expr(HirExprKind::BoolLiteral(true), Type::Bool)),
            ),
            Type::Bool,
        );

        builder.lower_expr(
            &not,
            Place {
                local: Local(0),
                projection: vec![],
            },
        );

        assert!(builder.blocks[0]
            .statements
            .iter()
            .any(|statement| matches!(
                &statement.kind,
                StatementKind::Assign(
                    Place {
                        local: Local(0),
                        projection,
                    },
                    Rvalue::UnaryOp(crate::mir::MirUnaryOp::Not, _),
                ) if projection.is_empty()
            )));
    }

    fn empty_function(id: DefId, name: &str) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: std::collections::HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn program_with_struct(fields: Vec<&str>) -> HirProgram {
        let struct_fields = fields
            .into_iter()
            .enumerate()
            .map(|(index, name)| HirField {
                id: FieldId(index as u32),
                name: name.to_string(),
                ty: Type::I64,
                public: true,
            })
            .collect();
        let id = DefId::new(CrateId(0), LocalDefId(0));
        let structs = std::collections::HashMap::from([(
            id,
            HirStruct {
                id,
                name: "Foo".to_string(),
                generic_params: vec![],
                fields: struct_fields,
            },
        )]);
        HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            structs,
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                structs_by_name: std::collections::HashMap::from([("Foo".to_string(), id)]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(id, "Foo".to_string())]),
        )
    }

    #[test]
    fn build_monomorphized_backend_contract_type_ids_use_final_mir_context() {
        let struct_id = DefId::new(CrateId(0), LocalDefId(0));
        let function_id = DefId::new(CrateId(0), LocalDefId(10));
        let instance_id = InstanceId(0);
        let mut program = program_with_struct(vec!["value"]);
        let function = HirFunction {
            id: function_id,
            name: "main".to_string(),
            generic_params: Vec::new(),
            generic_bounds: std::collections::HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::I32,
            body: HirBlock {
                stmts: vec![HirStmt::Return(Some(expr(
                    HirExprKind::IntLiteral(1),
                    Type::I32,
                )))],
                ty: Type::I32,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        program.functions.insert(function_id, function.clone());
        program
            .names
            .functions_by_name
            .insert("main".to_string(), function_id);
        program.rebuild_indexes();
        let mut type_context = crate::type_context::TypeContext::new();
        let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
        let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
        pre_mir_instance_bodies.insert(instance_id, function);
        let mono = crate::mono::MonomorphizedProgram {
            program,
            instances: std::collections::BTreeMap::from([(
                instance_id,
                crate::mono::InstanceRecord {
                    id: instance_id,
                    origin: crate::mono::InstanceOrigin::Function(function_id),
                    substitution: Vec::new(),
                    symbols: crate::mono::InstanceSymbols::new(
                        "main".to_string(),
                        "main".to_string(),
                    ),
                    declared: None,
                    provided_by_object: false,
                    is_specialization: false,
                },
            )]),
            pre_mir_instance_bodies,
            generated_drop_instances: Default::default(),
            type_context,
            source_map: crate::source_map::SemanticSourceMap::default(),
        };

        let mir = MirBuilder::build_monomorphized(&mono);

        let layout = mir
            .backend_contract
            .nominal_layouts
            .get(&struct_id)
            .expect("struct layout should be present");
        let crate::mir::MirNominalLayout::Struct { fields, .. } = layout else {
            panic!("expected struct layout, got {layout:?}");
        };
        assert_eq!(mir.type_context.type_for(fields[0].1), Type::I64);
    }

    #[test]
    #[should_panic(expected = "unmaterialized method DefId reached MIR lowering")]
    fn build_monomorphized_rejects_direct_static_method_function_target() {
        let caller_id = DefId::new(CrateId(0), LocalDefId(10));
        let impl_id = DefId::new(CrateId(0), LocalDefId(20));
        let method_id = DefId::new(CrateId(0), LocalDefId(21));
        let caller_instance = InstanceId(0);
        let method_instance = InstanceId(1);
        let function_instance = InstanceId(2);
        let callee = expr(
            HirExprKind::ResolvedVar(HirVarRef {
                name: "Global::dealloc".to_string(),
                target: HirVarTarget::Function(method_id),
            }),
            Type::function(Vec::new(), Type::I64),
        );
        let call = expr(
            HirExprKind::Call(
                Box::new(callee),
                Vec::new(),
                Some(crate::hir::HirCallTarget::Function(method_id)),
            ),
            Type::I64,
        );
        let caller = HirFunction {
            id: caller_id,
            name: "caller".to_string(),
            generic_params: Vec::new(),
            generic_bounds: std::collections::HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::I64,
            body: HirBlock {
                stmts: vec![HirStmt::Return(Some(call))],
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let method = HirFunction {
            id: method_id,
            name: "dealloc".to_string(),
            generic_params: Vec::new(),
            generic_bounds: std::collections::HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::I64,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let mut program = empty_program();
        program.functions.insert(caller_id, caller.clone());
        program
            .names
            .functions_by_name
            .insert("caller".to_string(), caller_id);
        program.rebuild_indexes();
        let mut type_context = crate::type_context::TypeContext::new();
        let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
        let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
        pre_mir_instance_bodies.insert(caller_instance, caller);
        let mono = crate::mono::MonomorphizedProgram {
            program,
            instances: std::collections::BTreeMap::from([
                (
                    caller_instance,
                    crate::mono::InstanceRecord {
                        id: caller_instance,
                        origin: crate::mono::InstanceOrigin::Function(caller_id),
                        substitution: Vec::new(),
                        symbols: crate::mono::InstanceSymbols::new("caller", "caller"),
                        declared: None,
                        provided_by_object: false,
                        is_specialization: false,
                    },
                ),
                (
                    method_instance,
                    crate::mono::InstanceRecord {
                        id: method_instance,
                        origin: crate::mono::InstanceOrigin::ImplMethod {
                            owner: crate::mono::InstanceImplOwner::Named(impl_id),
                            method: method_id,
                        },
                        substitution: Vec::new(),
                        symbols: crate::mono::InstanceSymbols::new(
                            "__rock_Global_dealloc",
                            "__rock_Global_dealloc",
                        ),
                        declared: Some(method.clone()),
                        provided_by_object: true,
                        is_specialization: false,
                    },
                ),
                (
                    function_instance,
                    crate::mono::InstanceRecord {
                        id: function_instance,
                        origin: crate::mono::InstanceOrigin::Function(method_id),
                        substitution: Vec::new(),
                        symbols: crate::mono::InstanceSymbols::new(
                            "__rock_Global_dealloc",
                            "__rock_Global_dealloc",
                        ),
                        declared: Some(method.clone()),
                        provided_by_object: false,
                        is_specialization: false,
                    },
                ),
            ]),
            pre_mir_instance_bodies,
            generated_drop_instances: Default::default(),
            type_context,
            source_map: crate::source_map::SemanticSourceMap::default(),
        };

        let mir = MirBuilder::build_monomorphized(&mono);
        let caller = mir
            .functions
            .get(&MirFunctionId::Instance(caller_instance))
            .expect("caller MIR function");

        assert!(
            caller.basic_blocks.iter().any(|block| matches!(
                &block.terminator,
                Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(crate::mir::MirCallable::Resolved(
                        crate::mir::MirCallableKey::Instance(found),
                    ))),
                    ..
                }) if *found == function_instance
            )),
            "caller MIR was: {caller:#?}"
        );
    }

    #[test]
    fn backend_contract_records_one_canonical_layout_without_visiting_display_aliases() {
        let struct_id = DefId::new(CrateId(0), LocalDefId(0));
        let enum_id = DefId::new(CrateId(0), LocalDefId(1));
        let mut program = program_with_struct(vec!["value"]);
        program
            .names
            .structs_by_name
            .insert("AliasFoo".to_string(), struct_id);
        program.enums.insert(
            enum_id,
            crate::hir::HirEnum {
                id: enum_id,
                name: "Result".to_string(),
                generic_params: Vec::new(),
                variants: Vec::new(),
            },
        );
        program
            .names
            .enums_by_name
            .insert("AliasResult".to_string(), enum_id);
        let canonical_names = std::collections::HashMap::from([
            (struct_id, "Foo".to_string()),
            (enum_id, "Result".to_string()),
        ]);
        program.rebuild_indexes_with_canonical_names(&canonical_names);

        let mut type_context = crate::type_context::TypeContext::new();
        let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
        let mono = crate::mono::MonomorphizedProgram {
            program,
            instances: std::collections::BTreeMap::new(),
            pre_mir_instance_bodies: crate::mono::PreMirInstanceBodies::new(),
            generated_drop_instances: Default::default(),
            type_context,
            source_map: crate::source_map::SemanticSourceMap::default(),
        };
        let functions = std::collections::BTreeMap::new();
        let mut contract_type_context = crate::type_context::TypeContext::new();
        let bodies = MirInstanceBodies::new();

        let contract = MirBuilder::backend_contract_for_program(
            &mono,
            &functions,
            &mut contract_type_context,
            &bodies,
        );

        assert_eq!(contract.nominal_layouts.len(), 2);
        assert!(matches!(
            contract.nominal_layouts.get(&struct_id),
            Some(crate::mir::MirNominalLayout::Struct { id, .. }) if *id == struct_id
        ));
        assert!(matches!(
            contract.nominal_layouts.get(&enum_id),
            Some(crate::mir::MirNominalLayout::Enum { id, .. }) if *id == enum_id
        ));
    }

    #[test]
    #[should_panic(expected = "has neither a MIR body nor a declared signature")]
    fn backend_contract_rejects_instance_without_body_or_declared_signature() {
        let instance_id = InstanceId(90);
        let function_id = DefId::new(CrateId(0), LocalDefId(90));
        let mono = crate::mono::MonomorphizedProgram {
            program: empty_program(),
            instances: std::collections::BTreeMap::from([(
                instance_id,
                crate::mono::InstanceRecord {
                    id: instance_id,
                    origin: crate::mono::InstanceOrigin::Function(function_id),
                    substitution: Vec::new(),
                    symbols: crate::mono::InstanceSymbols::new("missing", "missing"),
                    declared: None,
                    provided_by_object: true,
                    is_specialization: false,
                },
            )]),
            pre_mir_instance_bodies: crate::mono::PreMirInstanceBodies::new(),
            generated_drop_instances: Default::default(),
            type_context: TypeContext::new(),
            source_map: crate::source_map::SemanticSourceMap::default(),
        };
        let mut type_context = TypeContext::new();

        MirBuilder::backend_contract_for_program(
            &mono,
            &std::collections::BTreeMap::new(),
            &mut type_context,
            &MirInstanceBodies::new(),
        );
    }

    #[test]
    fn build_monomorphized_lowers_trait_impl_projection_contract() {
        let impl_id = DefId::new(CrateId(0), LocalDefId(20));
        let trait_id = DefId::new(CrateId(0), LocalDefId(21));
        let method_id = DefId::new(CrateId(0), LocalDefId(22));
        let function_id = DefId::new(CrateId(0), LocalDefId(23));
        let instance_id = InstanceId(0);
        let assoc_type_id = AssocTypeId(0);
        let base_ty = Type::Array(Box::new(Type::I64), 3);
        let projection_ty = Type::Projection {
            ty: Box::new(base_ty.clone()),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: vec![Type::I64],
        };
        let method = empty_function(method_id, "index");
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::BuiltinSlice,
            type_name: "[T; 3]".to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                "T",
            )],
            receiver_pattern: HirImplReceiverPattern::SliceFamily { element: Type::I64 },
            trait_name: Some("Index".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::I64],
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::Bool,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: std::collections::HashMap::from([("index".to_string(), method)]),
        };
        let function = HirFunction {
            ret_type: projection_ty.clone(),
            body: HirBlock {
                stmts: Vec::new(),
                ty: projection_ty,
            },
            ..empty_function(function_id, "project")
        };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(function_id, function.clone())]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(impl_id, imp)]),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([(
                    "project".to_string(),
                    function_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(function_id, "project".to_string())]),
        );
        let mut type_context = crate::type_context::TypeContext::new();
        let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
        let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
        pre_mir_instance_bodies.insert(instance_id, function);
        let mono = crate::mono::MonomorphizedProgram {
            program,
            instances: std::collections::BTreeMap::from([(
                instance_id,
                crate::mono::InstanceRecord {
                    id: instance_id,
                    origin: crate::mono::InstanceOrigin::Function(function_id),
                    substitution: Vec::new(),
                    symbols: crate::mono::InstanceSymbols::new(
                        "project".to_string(),
                        "project".to_string(),
                    ),
                    declared: None,
                    provided_by_object: false,
                    is_specialization: false,
                },
            )]),
            pre_mir_instance_bodies,
            generated_drop_instances: Default::default(),
            type_context,
            source_map: crate::source_map::SemanticSourceMap::default(),
        };

        let mir = MirBuilder::build_monomorphized(&mono);
        assert!(mir.backend_contract.projection_traits.contains(&trait_id));
        assert!(mir
            .backend_contract
            .projection_outputs
            .iter()
            .any(|(key, output)| {
                mir.type_context.type_for(key.base) == base_ty
                    && key.trait_id == trait_id
                    && key.assoc_type_id == assoc_type_id
                    && key
                        .trait_args
                        .iter()
                        .map(|arg| mir.type_context.type_for(*arg))
                        .collect::<Vec<_>>()
                        == vec![Type::I64]
                    && mir.type_context.type_for(*output) == Type::Bool
            }));
    }

    #[test]
    fn projection_provider_refuses_multiple_matching_impl_ids() {
        let trait_id = DefId::new(CrateId(0), LocalDefId(24));
        let generic_impl_id = DefId::new(CrateId(0), LocalDefId(25));
        let concrete_impl_id = DefId::new(CrateId(0), LocalDefId(26));
        let owner_ty = Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(27)),
            args: Vec::new(),
        };
        let imp =
            |impl_id: DefId, receiver_pattern: HirImplReceiverPattern, output: Type| HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Owner".to_string()),
                type_name: "Owner".to_string(),
                type_generics: Vec::new(),
                receiver_pattern,
                trait_name: Some("Project".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: vec![HirAssociatedTypeDef {
                    id: AssocTypeId(0),
                    name: "Output".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: output,
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: std::collections::HashMap::new(),
            };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::from([
                (
                    generic_impl_id,
                    imp(
                        generic_impl_id,
                        HirImplReceiverPattern::Exact(Type::Generic(
                            crate::types::GenericParamId {
                                owner: generic_impl_id,
                                index: 0,
                            },
                        )),
                        Type::I64,
                    ),
                ),
                (
                    concrete_impl_id,
                    imp(
                        concrete_impl_id,
                        HirImplReceiverPattern::Exact(owner_ty.clone()),
                        Type::Bool,
                    ),
                ),
            ]),
            std::collections::HashMap::new(),
            HirNameTables::default(),
            &std::collections::HashMap::new(),
        );
        let provider = MirProjectionResolutionProvider { program: &program };

        let output = ProjectionProvider::resolve_projection_output(
            &provider,
            &owner_ty,
            trait_id,
            AssocTypeId(0),
            &[],
        );

        assert_eq!(output, None);
    }

    #[test]
    fn build_monomorphized_lowers_projection_output_for_concrete_projection_type() {
        let impl_id = DefId::new(CrateId(0), LocalDefId(30));
        let trait_id = DefId::new(CrateId(0), LocalDefId(31));
        let function_id = DefId::new(CrateId(0), LocalDefId(32));
        let assoc_type_id = AssocTypeId(0);
        let base_ty = Type::Array(Box::new(Type::I64), 3);
        let projection_ty = Type::Projection {
            ty: Box::new(base_ty.clone()),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: vec![Type::I64],
        };
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::BuiltinSlice,
            type_name: "[T; 3]".to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                "T",
            )],
            receiver_pattern: HirImplReceiverPattern::SliceFamily { element: Type::I64 },
            trait_name: Some("Index".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::I64],
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::Bool,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: std::collections::HashMap::new(),
        };
        let function = HirFunction {
            ret_type: projection_ty.clone(),
            body: HirBlock {
                stmts: Vec::new(),
                ty: projection_ty.clone(),
            },
            ..empty_function(function_id, "project")
        };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(function_id, function.clone())]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(impl_id, imp)]),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([(
                    "project".to_string(),
                    function_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(function_id, "project".to_string())]),
        );
        let mut type_context = crate::type_context::TypeContext::new();
        let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
        let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
        pre_mir_instance_bodies.insert(InstanceId(0), function);
        let mono = crate::mono::MonomorphizedProgram {
            program,
            instances: std::collections::BTreeMap::from([(
                InstanceId(0),
                crate::mono::InstanceRecord {
                    id: InstanceId(0),
                    origin: crate::mono::InstanceOrigin::Function(function_id),
                    substitution: Vec::new(),
                    symbols: crate::mono::InstanceSymbols::new(
                        "project".to_string(),
                        "project".to_string(),
                    ),
                    declared: None,
                    provided_by_object: false,
                    is_specialization: false,
                },
            )]),
            pre_mir_instance_bodies,
            generated_drop_instances: Default::default(),
            type_context,
            source_map: crate::source_map::SemanticSourceMap::default(),
        };

        let mir = MirBuilder::build_monomorphized(&mono);

        assert!(mir
            .backend_contract
            .projection_outputs
            .iter()
            .any(
                |(key, output)| mir.type_context.type_for(key.base) == base_ty
                    && key.trait_id == trait_id
                    && key.assoc_type_id == assoc_type_id
                    && key
                        .trait_args
                        .iter()
                        .map(|arg| mir.type_context.type_for(*arg))
                        .collect::<Vec<_>>()
                        == vec![Type::I64]
                    && mir.type_context.type_for(*output) == Type::Bool
            ));
    }

    #[test]
    fn projection_output_contract_matches_named_impl_owner() {
        let foo_id = DefId::new(CrateId(0), LocalDefId(33));
        let bar_id = DefId::new(CrateId(0), LocalDefId(34));
        let foo_impl_id = DefId::new(CrateId(0), LocalDefId(35));
        let bar_impl_id = DefId::new(CrateId(0), LocalDefId(36));
        let trait_id = DefId::new(CrateId(0), LocalDefId(37));
        let function_id = DefId::new(CrateId(0), LocalDefId(38));
        let assoc_type_id = AssocTypeId(0);
        let bar_ty = Type::Struct {
            id: bar_id,
            args: Vec::new(),
        };
        let projection_ty = Type::Projection {
            ty: Box::new(bar_ty.clone()),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: Vec::new(),
        };
        let foo_impl = HirImpl {
            id: foo_impl_id,
            owner: HirImplOwner::Named("Foo".to_string()),
            type_name: "Foo".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: foo_id,
                args: Vec::new(),
            }),
            trait_name: Some("Project".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::Bool,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: std::collections::HashMap::new(),
        };
        let bar_impl = HirImpl {
            id: bar_impl_id,
            owner: HirImplOwner::Named("Bar".to_string()),
            type_name: "Bar".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: bar_id,
                args: Vec::new(),
            }),
            trait_name: Some("Project".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::U8,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: std::collections::HashMap::new(),
        };
        let function = HirFunction {
            ret_type: projection_ty.clone(),
            body: HirBlock {
                stmts: Vec::new(),
                ty: projection_ty,
            },
            ..empty_function(function_id, "project_bar")
        };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(function_id, function.clone())]),
            std::collections::HashMap::from([
                (
                    foo_id,
                    HirStruct {
                        id: foo_id,
                        name: "Foo".to_string(),
                        generic_params: Vec::new(),
                        fields: Vec::new(),
                    },
                ),
                (
                    bar_id,
                    HirStruct {
                        id: bar_id,
                        name: "Bar".to_string(),
                        generic_params: Vec::new(),
                        fields: Vec::new(),
                    },
                ),
            ]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(foo_impl_id, foo_impl), (bar_impl_id, bar_impl)]),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([(
                    "project_bar".to_string(),
                    function_id,
                )]),
                structs_by_name: std::collections::HashMap::from([
                    ("Foo".to_string(), foo_id),
                    ("Bar".to_string(), bar_id),
                ]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([
                (function_id, "project_bar".to_string()),
                (foo_id, "Foo".to_string()),
                (bar_id, "Bar".to_string()),
            ]),
        );
        let mut type_context = crate::type_context::TypeContext::new();
        let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
        let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
        pre_mir_instance_bodies.insert(InstanceId(0), function);
        let mono = crate::mono::MonomorphizedProgram {
            program,
            instances: std::collections::BTreeMap::from([(
                InstanceId(0),
                crate::mono::InstanceRecord {
                    id: InstanceId(0),
                    origin: crate::mono::InstanceOrigin::Function(function_id),
                    substitution: Vec::new(),
                    symbols: crate::mono::InstanceSymbols::new(
                        "project_bar".to_string(),
                        "project_bar".to_string(),
                    ),
                    declared: None,
                    provided_by_object: false,
                    is_specialization: false,
                },
            )]),
            pre_mir_instance_bodies,
            generated_drop_instances: Default::default(),
            type_context,
            source_map: crate::source_map::SemanticSourceMap::default(),
        };

        let mir = MirBuilder::build_monomorphized(&mono);

        assert!(mir
            .backend_contract
            .projection_outputs
            .iter()
            .any(
                |(key, output)| mir.type_context.type_for(key.base) == bar_ty
                    && key.trait_id == trait_id
                    && key.assoc_type_id == assoc_type_id
                    && key.trait_args.is_empty()
                    && mir.type_context.type_for(*output) == Type::U8
            ));
        assert!(!mir
            .backend_contract
            .projection_outputs
            .iter()
            .any(
                |(key, output)| mir.type_context.type_for(key.base) == bar_ty
                    && key.trait_id == trait_id
                    && mir.type_context.type_for(*output) == Type::Bool
            ));
    }

    #[test]
    fn projection_outputs_use_normalized_nested_projection_keys() {
        let inner_impl_id = DefId::new(CrateId(0), LocalDefId(33));
        let inner_trait_id = DefId::new(CrateId(0), LocalDefId(34));
        let outer_impl_id = DefId::new(CrateId(0), LocalDefId(35));
        let outer_trait_id = DefId::new(CrateId(0), LocalDefId(36));
        let assoc_type_id = AssocTypeId(0);
        let function_id = DefId::new(CrateId(0), LocalDefId(38));
        let base_ty = Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(37)),
            args: Vec::new(),
        };
        let inner_projection = Type::Projection {
            ty: Box::new(base_ty.clone()),
            trait_id: inner_trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: inner_trait_id,
                assoc_type_id,
            },
            trait_args: Vec::new(),
        };
        let outer_projection = Type::Projection {
            ty: Box::new(inner_projection.clone()),
            trait_id: outer_trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: outer_trait_id,
                assoc_type_id,
            },
            trait_args: Vec::new(),
        };
        let inner_impl = HirImpl {
            id: inner_impl_id,
            owner: HirImplOwner::Named("InnerOwner".to_string()),
            type_name: "InnerOwner".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(base_ty.clone()),
            trait_name: Some("Inner".to_string()),
            trait_id: Some(inner_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::Bool,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: std::collections::HashMap::new(),
        };
        let outer_impl = HirImpl {
            id: outer_impl_id,
            owner: HirImplOwner::Named("Bool".to_string()),
            type_name: "Bool".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Bool),
            trait_name: Some("Outer".to_string()),
            trait_id: Some(outer_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::U8,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: std::collections::HashMap::new(),
        };
        let base_id = match &base_ty {
            Type::Struct { id, .. } => *id,
            _ => unreachable!(),
        };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(
                base_id,
                HirStruct {
                    id: base_id,
                    name: "InnerOwner".to_string(),
                    generic_params: Vec::new(),
                    fields: Vec::new(),
                },
            )]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::from([
                (inner_impl_id, inner_impl),
                (outer_impl_id, outer_impl),
            ]),
            std::collections::HashMap::new(),
            HirNameTables {
                structs_by_name: std::collections::HashMap::from([(
                    "InnerOwner".to_string(),
                    base_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(base_id, "InnerOwner".to_string())]),
        );
        let mut type_context = TypeContext::new();
        let ret_type = type_context.intern_type(&outer_projection);
        let mono = crate::mono::MonomorphizedProgram {
            program,
            instances: std::collections::BTreeMap::new(),
            pre_mir_instance_bodies: crate::mono::PreMirInstanceBodies::new(),
            generated_drop_instances: Default::default(),
            type_context: TypeContext::new(),
            source_map: crate::source_map::SemanticSourceMap::default(),
        };
        let functions = std::collections::BTreeMap::from([(
            MirFunctionId::Function(function_id),
            MirFunction {
                id: MirFunctionId::Function(function_id),
                name: "project_nested".to_string(),
                basic_blocks: Vec::new(),
                local_decls: Vec::new(),
                closure_captures: Vec::new(),
                arg_count: 0,
                ret_type,
                ownership: Default::default(),
            },
        )]);
        let contract = MirBuilder::backend_contract_for_program(
            &mono,
            &functions,
            &mut type_context,
            &MirInstanceBodies::new(),
        );

        assert!(contract
            .projection_outputs
            .iter()
            .any(|(key, output)| type_context.type_for(key.base) == base_ty
                && key.trait_id == inner_trait_id
                && key.assoc_type_id == assoc_type_id
                && key.trait_args.is_empty()
                && type_context.type_for(*output) == Type::Bool));
        assert!(contract
            .projection_outputs
            .iter()
            .any(
                |(key, output)| type_context.type_for(key.base) == Type::Bool
                    && key.trait_id == outer_trait_id
                    && key.assoc_type_id == assoc_type_id
                    && key.trait_args.is_empty()
                    && type_context.type_for(*output) == Type::U8
            ));
        assert!(!contract
            .projection_outputs
            .iter()
            .any(
                |(key, _)| type_context.type_for(key.base) == inner_projection
                    && key.trait_id == outer_trait_id
            ));
    }

    #[test]
    fn projection_outputs_use_selected_index_impl() {
        let impl_id = DefId::new(CrateId(0), LocalDefId(39));
        let trait_id = DefId::new(CrateId(0), LocalDefId(40));
        let assoc_type_id = AssocTypeId(0);
        let base_ty = Type::Array(Box::new(Type::U8), 4);
        let projection = Type::Projection {
            ty: Box::new(base_ty.clone()),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: vec![Type::I64],
        };
        let index_impl = HirImpl {
            id: impl_id,
            owner: HirImplOwner::BuiltinSlice,
            type_name: "[U8; 4]".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::SliceFamily { element: Type::U8 },
            trait_name: Some("Index".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::I64],
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::U8,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: std::collections::HashMap::new(),
        };
        let mut type_context = TypeContext::new();
        let ret_type = type_context.intern_type(&projection);
        let function_id = DefId::new(CrateId(0), LocalDefId(41));
        let mono = crate::mono::MonomorphizedProgram {
            program: HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
                std::collections::HashMap::from([(impl_id, index_impl)]),
                std::collections::HashMap::new(),
                HirNameTables::default(),
                &std::collections::HashMap::new(),
            ),
            instances: std::collections::BTreeMap::new(),
            pre_mir_instance_bodies: crate::mono::PreMirInstanceBodies::new(),
            generated_drop_instances: Default::default(),
            type_context: TypeContext::new(),
            source_map: crate::source_map::SemanticSourceMap::default(),
        };
        let functions = std::collections::BTreeMap::from([(
            MirFunctionId::Function(function_id),
            MirFunction {
                id: MirFunctionId::Function(function_id),
                name: "project_index".to_string(),
                basic_blocks: Vec::new(),
                local_decls: Vec::new(),
                closure_captures: Vec::new(),
                arg_count: 0,
                ret_type,
                ownership: Default::default(),
            },
        )]);
        let contract = MirBuilder::backend_contract_for_program(
            &mono,
            &functions,
            &mut type_context,
            &MirInstanceBodies::new(),
        );

        let exact_key = crate::mir::MirProjectionKey {
            base: type_context
                .id_for_type(&base_ty)
                .expect("interned base type"),
            trait_id,
            assoc_type_id,
            trait_args: vec![type_context
                .id_for_type(&Type::I64)
                .expect("interned index type")],
        };
        assert_eq!(
            contract
                .projection_outputs
                .get(&exact_key)
                .map(|output| type_context.type_for(*output)),
            Some(Type::U8)
        );
    }

    #[test]
    fn projection_outputs_include_callable_abi_return_type() {
        let impl_id = DefId::new(CrateId(0), LocalDefId(43));
        let trait_id = DefId::new(CrateId(0), LocalDefId(41));
        let assoc_type_id = AssocTypeId(0);
        let base_ty = Type::Array(Box::new(Type::U8), 4);
        let abi_projection = Type::Projection {
            ty: Box::new(base_ty.clone()),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: vec![Type::I64],
        };
        let index_impl = HirImpl {
            id: impl_id,
            owner: HirImplOwner::BuiltinSlice,
            type_name: "[U8; 4]".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::SliceFamily { element: Type::U8 },
            trait_name: Some("Index".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::I64],
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::U8,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: std::collections::HashMap::new(),
        };
        let mut type_context = TypeContext::new();
        let semantic_ret = type_context.intern_type(&Type::Unit);
        let abi_ret = type_context.intern_type(&abi_projection);
        let mut signature = crate::mir::MirCallableSignature::from_type_ids(
            &[],
            semantic_ret,
            crate::mir::MirPassMode::Direct,
        );
        signature.ret.abi_ty = abi_ret;
        let mut contract = crate::mir::MirBackendContract::default();
        let callable_id = DefId::new(CrateId(0), LocalDefId(42));
        let key = crate::mir::MirCallableKey::Extern(callable_id);
        contract.callables.insert(
            key.clone(),
            crate::mir::MirCallableDecl {
                key,
                source_def_id: Some(callable_id),
                kind: crate::mir::MirCallableKind::Extern {
                    link_name: "abi_projection".to_string(),
                    variadic: false,
                },
                llvm_symbol: "abi_projection".to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature,
            },
        );
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(impl_id, index_impl)]),
            std::collections::HashMap::new(),
            HirNameTables::default(),
            &std::collections::HashMap::new(),
        );

        MirBuilder::populate_backend_contract_projection_outputs(
            &mut contract,
            &program,
            &std::collections::BTreeMap::new(),
            &mut type_context,
        );

        assert!(contract
            .projection_outputs
            .iter()
            .any(|(key, output)| type_context.type_for(key.base) == base_ty
                && key.trait_id == trait_id
                && key.assoc_type_id == assoc_type_id
                && key
                    .trait_args
                    .iter()
                    .map(|arg| type_context.type_for(*arg))
                    .collect::<Vec<_>>()
                    == vec![Type::I64]
                && type_context.type_for(*output) == Type::U8));
    }

    #[test]
    fn projection_output_type_ids_are_deterministic() {
        let build_contract = || {
            let first_trait_id = DefId::new(CrateId(0), LocalDefId(43));
            let second_trait_id = DefId::new(CrateId(0), LocalDefId(44));
            let first_base = Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(45)),
                args: Vec::new(),
            };
            let second_base = Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(46)),
                args: Vec::new(),
            };
            let projection = |base: Type, trait_id| Type::Projection {
                ty: Box::new(base),
                trait_id,
                assoc_type: crate::types::AssociatedTypeKey {
                    owner: trait_id,
                    assoc_type_id: AssocTypeId(0),
                },
                trait_args: Vec::new(),
            };
            let first_projection = projection(first_base.clone(), first_trait_id);
            let second_projection = projection(second_base.clone(), second_trait_id);
            let impl_for = |impl_id: DefId, trait_id, base, output| HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Owner".to_string()),
                type_name: "Owner".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(base),
                trait_name: Some("Project".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: vec![HirAssociatedTypeDef {
                    id: AssocTypeId(0),
                    name: "Output".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: output,
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: std::collections::HashMap::new(),
            };
            let first_impl_id = DefId::new(CrateId(0), LocalDefId(47));
            let second_impl_id = DefId::new(CrateId(0), LocalDefId(48));
            let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
                std::collections::HashMap::from([
                    (
                        first_impl_id,
                        impl_for(
                            first_impl_id,
                            first_trait_id,
                            first_base,
                            Type::Tuple(vec![Type::Bool]),
                        ),
                    ),
                    (
                        second_impl_id,
                        impl_for(
                            second_impl_id,
                            second_trait_id,
                            second_base,
                            Type::Tuple(vec![Type::U8]),
                        ),
                    ),
                ]),
                std::collections::HashMap::new(),
                HirNameTables::default(),
                &std::collections::HashMap::new(),
            );
            let mut type_context = TypeContext::new();
            let semantic_ret = type_context.intern_type(&Type::Unit);
            let first_abi_ret = type_context.intern_type(&first_projection);
            let second_abi_ret = type_context.intern_type(&second_projection);
            let mut contract = crate::mir::MirBackendContract::default();

            for (callable_id, abi_ty) in [
                (DefId::new(CrateId(0), LocalDefId(49)), first_abi_ret),
                (DefId::new(CrateId(0), LocalDefId(50)), second_abi_ret),
            ] {
                let mut signature = crate::mir::MirCallableSignature::from_type_ids(
                    &[],
                    semantic_ret,
                    crate::mir::MirPassMode::Direct,
                );
                signature.ret.abi_ty = abi_ty;
                let key = crate::mir::MirCallableKey::Extern(callable_id);
                contract.callables.insert(
                    key.clone(),
                    crate::mir::MirCallableDecl {
                        key,
                        source_def_id: Some(callable_id),
                        kind: crate::mir::MirCallableKind::Extern {
                            link_name: format!("projection_{}", callable_id.local.0),
                            variadic: false,
                        },
                        llvm_symbol: format!("projection_{}", callable_id.local.0),
                        linkage: crate::mir::MirLinkage::External,
                        signature,
                    },
                );
            }

            MirBuilder::populate_backend_contract_projection_outputs(
                &mut contract,
                &program,
                &std::collections::BTreeMap::new(),
                &mut type_context,
            );
            let output_types = contract
                .projection_outputs
                .values()
                .map(|output| type_context.type_for(*output))
                .collect::<Vec<_>>();
            (contract.projection_outputs, output_types)
        };

        let expected = build_contract();
        for _ in 0..32 {
            assert_eq!(build_contract(), expected);
        }
    }

    fn struct_ty(local: u32) -> Type {
        Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(local)),
            args: vec![],
        }
    }

    fn option_program(variant_fields: Vec<(VariantId, &str, HirVariantFields)>) -> HirProgram {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let variants = variant_fields
            .into_iter()
            .map(|(id, name, fields)| crate::hir::HirVariant {
                id,
                name: name.to_string(),
                fields,
            })
            .collect();
        HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(
                enum_id,
                crate::hir::HirEnum {
                    id: enum_id,
                    name: "Option".to_string(),
                    generic_params: vec![],
                    variants,
                },
            )]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                enums_by_name: std::collections::HashMap::from([("Option".to_string(), enum_id)]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(enum_id, "Option".to_string())]),
        )
    }

    fn option_ty() -> Type {
        Type::Enum {
            id: DefId::new(CrateId(0), LocalDefId(40)),
            args: vec![],
        }
    }

    fn match_arm(pattern: HirPattern, guard: Option<HirExpr>, body: HirExpr) -> HirMatchArm {
        HirMatchArm {
            pattern,
            guard,
            body: HirBlock {
                stmts: vec![HirStmt::Expr(body.clone())],
                ty: body.ty,
            },
        }
    }

    fn builder_with_var<'a>(
        program: &'a HirProgram,
        type_context: &'a RefCell<crate::type_context::TypeContext>,
        name: &'a str,
        ty: Type,
    ) -> MirBuilder<'a> {
        let mut builder = MirBuilder::new(program, type_context);
        let ty_id = builder.type_id_for(&ty);
        builder.locals.push(LocalDecl {
            ty: ty_id,
            mutability: Mutability::Not,
            name: Some(name.to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.var_map.insert(name.to_string(), Local(0));
        builder
    }

    fn assert_builder_agreement_clean(builder: MirBuilder<'_>, ret_type: Type) {
        assert_builder_agreement_clean_with_contract(builder, ret_type, Default::default());
    }

    fn assert_builder_agreement_clean_with_contract(
        builder: MirBuilder<'_>,
        ret_type: Type,
        backend_contract: crate::mir::MirBackendContract,
    ) {
        let report = builder_agreement_report_with_contract(builder, ret_type, backend_contract);
        assert!(
            report.is_clean(),
            "unexpected MIR agreement report: {report:?}"
        );
    }

    fn builder_agreement_report_with_contract(
        builder: MirBuilder<'_>,
        ret_type: Type,
        backend_contract: crate::mir::MirBackendContract,
    ) -> crate::mir::agreement::MirAgreementReport {
        let id = DefId::new(CrateId(0), LocalDefId(999));
        let function_id = MirFunctionId::Function(id);
        let ret_type = builder
            .locals
            .first()
            .map(|local| local.ty)
            .unwrap_or_else(|| builder.type_id_for(&ret_type));
        let program = builder.program;
        let mut type_context = builder.type_context.borrow().clone();
        let functions = std::collections::BTreeMap::from([(
            function_id.clone(),
            MirFunction {
                id: function_id.clone(),
                name: "test".to_string(),
                basic_blocks: builder.blocks,
                local_decls: builder.locals,
                closure_captures: builder.closure_captures,
                arg_count: 0,
                ret_type,
                ownership: builder.ownership,
            },
        )]);
        let mut backend_contract = backend_contract;
        MirBuilder::populate_backend_contract_nominal_layouts(
            &mut backend_contract,
            program,
            &mut type_context,
        );
        MirBuilder::populate_backend_contract_function_bodies(
            &mut backend_contract,
            &functions,
            &type_context,
        );
        MirBuilder::populate_backend_contract_runtime_requirements(
            &mut backend_contract,
            &functions,
        );
        let mir = MirProgram {
            functions,
            type_context,
            backend_contract,
        };
        crate::mir::agreement::check_mir_runtime_agreement(&mir)
    }

    fn instance_callable_contract(
        id: InstanceId,
        callable_def_id: DefId,
        symbol: &str,
        is_method: bool,
        params: Vec<TypeId>,
        ret: TypeId,
        type_context: &TypeContext,
    ) -> crate::mir::MirBackendContract {
        let key = crate::mir::MirCallableKey::Instance(id);
        let mut contract = crate::mir::MirBackendContract::default();
        contract.callables.insert(
            key.clone(),
            crate::mir::MirCallableDecl {
                key,
                source_def_id: Some(callable_def_id),
                kind: crate::mir::MirCallableKind::ObjectProvided,
                llvm_symbol: symbol.to_string(),
                linkage: crate::mir::MirLinkage::External,
                signature: crate::mir::MirCallableSignature::from_instance_type_ids(
                    &params,
                    ret,
                    is_method,
                    type_context,
                ),
            },
        );
        contract
    }

    fn assert_mir_agreement_clean(mir: &MirProgram) {
        let report = crate::mir::agreement::check_mir_runtime_agreement(mir);
        assert!(
            report.is_clean(),
            "unexpected MIR agreement report: {report:?}"
        );
    }

    #[test]
    fn backend_contract_records_runtime_requirements_from_final_mir() {
        let bounds_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1000)));
        let captured_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1001)));
        let empty_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1002)));
        let closure_owner = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1003)));
        let closure_id = MirClosureId {
            owner: closure_owner,
            local_index: 0,
        };
        let mut type_context = TypeContext::new();
        let i64_ty = type_context.intern_type(&Type::I64);
        let function = |id: MirFunctionId, statements: Vec<StatementData>| MirFunction {
            id,
            name: "runtime_requirement_fixture".to_string(),
            basic_blocks: vec![BasicBlock {
                statements,
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: i64_ty,
                    mutability: Mutability::Not,
                    name: Some("base".to_string()),
                    span: None,
                    source: LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_ty,
                    mutability: Mutability::Not,
                    name: Some("index".to_string()),
                    span: None,
                    source: LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: TypeId(0),
            ownership: Default::default(),
        };
        let functions = std::collections::BTreeMap::from([
            (
                bounds_id.clone(),
                function(
                    bounds_id,
                    vec![StatementData::assert(
                        crate::mir::MirAssert {
                            kind: crate::mir::MirAssertKind::BoundsCheck,
                            operands: vec![
                                Operand::Copy(Place {
                                    local: Local(0),
                                    projection: vec![],
                                }),
                                Operand::Copy(Place {
                                    local: Local(1),
                                    projection: vec![],
                                }),
                            ],
                        },
                        None,
                    )],
                ),
            ),
            (
                captured_id.clone(),
                function(
                    captured_id,
                    vec![StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: vec![],
                        },
                        Rvalue::Closure(crate::mir::MirClosure {
                            id: closure_id.clone(),
                            display_name: "captured".to_string(),
                            captures: vec![crate::mir::MirClosureCapture {
                                name: "capture".to_string(),
                                local: Local(1),
                                kind: crate::mir::MirClosureCaptureKind::ByValue,
                                span: None,
                            }],
                        }),
                        None,
                    )],
                ),
            ),
            (
                empty_id.clone(),
                function(
                    empty_id,
                    vec![StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: vec![],
                        },
                        Rvalue::Closure(crate::mir::MirClosure {
                            id: closure_id,
                            display_name: "empty".to_string(),
                            captures: vec![],
                        }),
                        None,
                    )],
                ),
            ),
        ]);
        let mut contract = crate::mir::MirBackendContract::default();

        MirBuilder::populate_backend_contract_runtime_requirements(&mut contract, &functions);

        assert_eq!(
            contract.runtime_requirements,
            std::collections::BTreeSet::from([
                crate::mir::MirRuntimeHelper::BoundsCheck,
                crate::mir::MirRuntimeHelper::HeapAlloc,
            ])
        );
    }

    #[test]
    fn fallback_contract_closure_symbols_include_owner_identity() {
        let mut type_context = TypeContext::new();
        let unit_id = type_context.intern_type(&Type::Unit);
        let left_parent = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(10)));
        let right_parent = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(11)));
        let left_id = MirFunctionId::Closure(Box::new(MirClosureId {
            owner: left_parent,
            local_index: 0,
        }));
        let right_id = MirFunctionId::Closure(Box::new(MirClosureId {
            owner: right_parent,
            local_index: 0,
        }));
        let closure_function = |id: MirFunctionId| MirFunction {
            id,
            name: "lambda_0".to_string(),
            basic_blocks: vec![],
            local_decls: vec![LocalDecl {
                ty: unit_id,
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: unit_id,
            ownership: Default::default(),
        };
        let functions = std::collections::BTreeMap::from([
            (left_id.clone(), closure_function(left_id.clone())),
            (right_id.clone(), closure_function(right_id.clone())),
        ]);
        let mut contract = crate::mir::MirBackendContract::default();

        MirBuilder::populate_backend_contract_function_bodies(
            &mut contract,
            &functions,
            &type_context,
        );

        let left_symbol = &contract
            .callable(&crate::mir::MirCallableKey::Closure(left_id))
            .expect("left closure callable")
            .llvm_symbol;
        let right_symbol = &contract
            .callable(&crate::mir::MirCallableKey::Closure(right_id))
            .expect("right closure callable")
            .llvm_symbol;
        assert_ne!(left_symbol, right_symbol);
        assert_ne!(left_symbol, "lambda_0");
        assert_ne!(right_symbol, "lambda_0");
    }

    #[test]
    fn drop_glue_collection_consumes_generated_mono_instance() {
        let mut type_context = TypeContext::new();
        let struct_id = DefId::new(CrateId(0), LocalDefId(50));
        let drop_trait_id = DefId::new(CrateId(0), LocalDefId(51));
        let drop_member_id = DefId::new(CrateId(0), LocalDefId(52));
        let instance_id = InstanceId(55);
        let ty = Type::Struct {
            id: struct_id,
            args: vec![],
        };
        let ty_id = type_context.intern_type(&ty);
        let generated = std::collections::BTreeMap::from([(
            ty_id,
            crate::mono::GeneratedMethodInstance {
                receiver_ty: ty_id,
                trait_id: drop_trait_id,
                member_id: drop_member_id,
                instance_id,
                origin_span: None,
            },
        )]);

        let mut contract = crate::mir::MirBackendContract::default();
        MirBuilder::populate_backend_contract_drop_glue(&mut contract, &generated);

        assert_eq!(
            contract.drop_glue.get(&ty_id),
            Some(&crate::mir::MirCallableKey::Instance(instance_id)),
        );
    }

    #[test]
    fn intrinsic_contract_signature_uses_projected_place_types() {
        let struct_id = DefId::new(CrateId(0), LocalDefId(41));
        let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(42)));
        let mut type_context = TypeContext::new();
        let unit_id = type_context.intern_type(&Type::Unit);
        let ptr_id = type_context.intern_type(&Type::Pointer(Box::new(Type::U8)));
        let len_id = type_context.intern_type(&Type::I64);
        let slice_id = type_context.intern_type(&Type::Slice(Box::new(Type::U8)));
        let struct_id_ty = type_context.intern_type(&Type::Struct {
            id: struct_id,
            args: vec![],
        });
        let function = MirFunction {
            id: function_id.clone(),
            name: "uses_borrow_slice".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![],
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        crate::mir::MirCallableKey::Intrinsic(MirIntrinsicId::BorrowSlice),
                    ))),
                    args: vec![
                        Operand::Copy(Place {
                            local: Local(1),
                            projection: vec![Projection::Field {
                                index: 0,
                                identity: None,
                            }],
                        }),
                        Operand::Copy(Place {
                            local: Local(2),
                            projection: vec![],
                        }),
                    ],
                    destination: Place {
                        local: Local(3),
                        projection: vec![],
                    },
                    target: BasicBlockId(0),
                    span: None,
                }),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: unit_id,
                    mutability: Mutability::Not,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: struct_id_ty,
                    mutability: Mutability::Not,
                    name: Some("buffer".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: len_id,
                    mutability: Mutability::Not,
                    name: Some("len".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: slice_id,
                    mutability: Mutability::Not,
                    name: Some("slice".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: unit_id,
            ownership: Default::default(),
        };
        let functions = std::collections::BTreeMap::from([(function_id, function)]);
        let mut contract = crate::mir::MirBackendContract::default();
        contract.nominal_layouts.insert(
            struct_id,
            crate::mir::MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("ptr".to_string(), ptr_id), ("len".to_string(), len_id)],
                generic_params: vec![],
            },
        );

        MirBuilder::populate_backend_contract_function_bodies(
            &mut contract,
            &functions,
            &type_context,
        );

        let declaration = contract
            .callable(&crate::mir::MirCallableKey::Intrinsic(
                MirIntrinsicId::BorrowSlice,
            ))
            .expect("BorrowSlice intrinsic callable should be registered");
        assert_eq!(declaration.signature.params[0].semantic_ty, ptr_id);
        assert_eq!(declaration.signature.params[1].semantic_ty, len_id);
        assert_eq!(declaration.signature.ret.semantic_ty, slice_id);
    }

    #[test]
    fn intrinsic_contract_signature_uses_projected_aggregate_field_types() {
        let trait_id = DefId::new(CrateId(0), LocalDefId(43));
        let assoc_type_id = AssocTypeId(0);
        let struct_id = DefId::new(CrateId(0), LocalDefId(44));
        let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(45)));
        let mut type_context = TypeContext::new();
        let unit_id = type_context.intern_type(&Type::Unit);
        let base_id = type_context.intern_type(&Type::I64);
        let ptr_id = type_context.intern_type(&Type::Pointer(Box::new(Type::U8)));
        let len_id = type_context.intern_type(&Type::I64);
        let slice_id = type_context.intern_type(&Type::Slice(Box::new(Type::U8)));
        let struct_ty = Type::Struct {
            id: struct_id,
            args: vec![],
        };
        let struct_type_id = type_context.intern_type(&struct_ty);
        let projection_id = type_context.intern_type(&Type::Projection {
            ty: Box::new(Type::I64),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: Vec::new(),
        });
        let function = MirFunction {
            id: function_id.clone(),
            name: "uses_projected_borrow_slice".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![],
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        crate::mir::MirCallableKey::Intrinsic(MirIntrinsicId::BorrowSlice),
                    ))),
                    args: vec![
                        Operand::Copy(Place {
                            local: Local(1),
                            projection: vec![Projection::Field {
                                index: 0,
                                identity: None,
                            }],
                        }),
                        Operand::Copy(Place {
                            local: Local(2),
                            projection: vec![],
                        }),
                    ],
                    destination: Place {
                        local: Local(3),
                        projection: vec![],
                    },
                    target: BasicBlockId(0),
                    span: None,
                }),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: unit_id,
                    mutability: Mutability::Not,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: projection_id,
                    mutability: Mutability::Not,
                    name: Some("projected_buffer".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: len_id,
                    mutability: Mutability::Not,
                    name: Some("len".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: slice_id,
                    mutability: Mutability::Not,
                    name: Some("slice".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::Temporary,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: unit_id,
            ownership: Default::default(),
        };
        let functions = std::collections::BTreeMap::from([(function_id, function)]);
        let mut contract = crate::mir::MirBackendContract::default();
        contract.projection_outputs.insert(
            crate::mir::MirProjectionKey {
                base: base_id,
                trait_id,
                assoc_type_id,
                trait_args: Vec::new(),
            },
            struct_type_id,
        );
        contract.nominal_layouts.insert(
            struct_id,
            crate::mir::MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("ptr".to_string(), ptr_id), ("len".to_string(), len_id)],
                generic_params: Vec::new(),
            },
        );

        MirBuilder::populate_backend_contract_function_bodies(
            &mut contract,
            &functions,
            &type_context,
        );

        let declaration = contract
            .callable(&crate::mir::MirCallableKey::Intrinsic(
                MirIntrinsicId::BorrowSlice,
            ))
            .expect("BorrowSlice intrinsic callable should be registered for projected field");
        assert_eq!(declaration.signature.params[0].semantic_ty, ptr_id);
        assert_eq!(declaration.signature.params[1].semantic_ty, len_id);
        assert_eq!(declaration.signature.ret.semantic_ty, slice_id);
    }

    #[test]
    fn build_monomorphized_uses_explicit_instance_body_table() {
        use crate::mono::{InstanceOrigin, InstanceRecord, MonomorphizedProgram};

        let function_id = DefId::new(CrateId(0), LocalDefId(9));
        let instance_id = InstanceId(9);
        let backend_symbol = "main__explicit_body";
        let function = empty_function(function_id, "main");
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(function_id, function.clone())]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([(
                    "main".to_string(),
                    function_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(function_id, "main".to_string())]),
        );
        let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
        pre_mir_instance_bodies.insert(instance_id, function);
        let mut monomorphized = MonomorphizedProgram {
            program,
            instances: std::collections::BTreeMap::from([(
                instance_id,
                InstanceRecord {
                    id: instance_id,
                    origin: InstanceOrigin::Function(function_id),
                    substitution: Vec::new(),
                    symbols: crate::mono::InstanceSymbols::new(
                        "main".to_string(),
                        backend_symbol.to_string(),
                    ),
                    declared: None,
                    provided_by_object: false,
                    is_specialization: false,
                },
            )]),
            pre_mir_instance_bodies,
            generated_drop_instances: Default::default(),
            type_context: crate::type_context::TypeContext::new(),
            source_map: crate::source_map::SemanticSourceMap::default(),
        };
        let mut bodies = MirBuilder::take_mir_instance_bodies(&mut monomorphized);
        let unit_ty = monomorphized.type_context.intern_type(&Type::Unit);
        let closure_id = crate::mir::MirFunctionId::Closure(Box::new(crate::mir::MirClosureId {
            owner: crate::mir::MirFunctionId::Instance(instance_id),
            local_index: 0,
        }));
        bodies.push_nested(MirFunction {
            id: closure_id.clone(),
            name: "lambda_0".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![LocalDecl {
                ty: unit_ty,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: unit_ty,
            ownership: Default::default(),
        });

        let mir = MirBuilder::build_monomorphized_with_instance_bodies(&monomorphized, &bodies);

        let id = crate::mir::MirFunctionId::Instance(instance_id);
        let function = mir.function(id.clone()).expect("instance MIR body");
        assert_eq!(function.id, id);
        assert!(mir
            .backend_contract
            .artifact_exports
            .iter()
            .any(|export| export.backend_symbol == backend_symbol && export.has_body));
        let key = crate::mir::MirCallableKey::Instance(instance_id);
        assert!(mir.backend_contract.callable(&key).is_some());
        assert_eq!(mir.backend_contract.function_bodies.get(&id), Some(&key));
        let closure_key = crate::mir::MirCallableKey::Closure(closure_id.clone());
        assert!(mir.backend_contract.callable(&closure_key).is_some());
        assert_eq!(
            mir.backend_contract.function_bodies.get(&closure_id),
            Some(&closure_key),
        );
    }

    #[test]
    fn take_mir_instance_bodies_converts_and_drains_hir_instance_bodies() {
        use crate::mono::{InstanceOrigin, InstanceRecord, MonomorphizedProgram};

        let function_id = DefId::new(CrateId(0), LocalDefId(12));
        let instance_id = InstanceId(12);
        let function = empty_function(function_id, "answer");
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(function_id, function.clone())]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([(
                    "answer".to_string(),
                    function_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(function_id, "answer".to_string())]),
        );
        let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
        pre_mir_instance_bodies.insert(instance_id, function.clone());
        let mut monomorphized = MonomorphizedProgram {
            program,
            instances: std::collections::BTreeMap::from([(
                instance_id,
                InstanceRecord {
                    id: instance_id,
                    origin: InstanceOrigin::Function(function_id),
                    substitution: Vec::new(),
                    symbols: crate::mono::InstanceSymbols::new(
                        "answer".to_string(),
                        "answer__i64".to_string(),
                    ),
                    declared: Some(function.clone()),
                    provided_by_object: false,
                    is_specialization: false,
                },
            )]),
            pre_mir_instance_bodies,
            generated_drop_instances: Default::default(),
            type_context: crate::type_context::TypeContext::new(),
            source_map: crate::source_map::SemanticSourceMap::default(),
        };

        let bodies = MirBuilder::take_mir_instance_bodies(&mut monomorphized);

        assert!(monomorphized
            .pre_mir_instance_bodies
            .get(instance_id)
            .is_none());
        let body = bodies.get(instance_id).expect("MIR instance body");
        assert_eq!(
            body.function.id,
            crate::mir::MirFunctionId::Instance(instance_id)
        );
    }

    #[test]
    fn build_monomorphized_program_keys_functions_by_instance_id() {
        use crate::mono::{InstanceOrigin, InstanceRecord, MonomorphizedProgram};

        let function_id = DefId::new(CrateId(0), LocalDefId(10));
        let instance_id = InstanceId(7);
        let backend_symbol = "main__i64";
        let function = HirFunction {
            id: function_id,
            name: "main".to_string(),
            generic_params: Vec::new(),
            generic_bounds: std::collections::HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::I64,
            body: HirBlock {
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKind::IntLiteral(42),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                })],
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(function_id, function.clone())]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([(
                    "main".to_string(),
                    function_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(function_id, "main".to_string())]),
        );
        let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
        pre_mir_instance_bodies.insert(instance_id, function);
        let mut instances = std::collections::BTreeMap::new();
        instances.insert(
            instance_id,
            InstanceRecord {
                id: instance_id,
                origin: InstanceOrigin::Function(function_id),
                substitution: Vec::new(),
                symbols: crate::mono::InstanceSymbols::new(
                    "main".to_string(),
                    backend_symbol.to_string(),
                ),
                declared: None,
                provided_by_object: false,
                is_specialization: false,
            },
        );
        let monomorphized = MonomorphizedProgram {
            program,
            instances,
            pre_mir_instance_bodies,
            generated_drop_instances: Default::default(),
            type_context: crate::type_context::TypeContext::new(),
            source_map: crate::source_map::SemanticSourceMap::default(),
        };

        let mir = MirBuilder::build_monomorphized(&monomorphized);

        let id = crate::mir::MirFunctionId::Instance(instance_id);
        let function = mir.function(id.clone()).expect("instance MIR body");
        assert_eq!(function.id, id);
        assert_eq!(function.name, backend_symbol);
    }

    #[test]
    fn mir_builder_records_type_ids_for_return_locals_and_casts() {
        use crate::hir::{HirVarRef, HirVarTarget};
        use crate::mono::{InstanceOrigin, InstanceRecord, MonomorphizedProgram};

        let function_id = DefId::new(CrateId(0), LocalDefId(80));
        let instance_id = InstanceId(80);
        let local_id = crate::ids::HirLocalId(0);
        let cast_value = expr(
            HirExprKind::Cast(
                Box::new(expr(HirExprKind::IntLiteral(1), Type::I64)),
                Type::U64,
            ),
            Type::U64,
        );
        let body = HirBlock {
            stmts: vec![
                HirStmt::Let {
                    name: "x".to_string(),
                    local_id,
                    ty: Type::U64,
                    value: cast_value,
                    mutable: false,
                },
                HirStmt::Expr(expr(
                    HirExprKind::ResolvedVar(HirVarRef {
                        name: "x".to_string(),
                        target: HirVarTarget::Local(local_id),
                    }),
                    Type::U64,
                )),
            ],
            ty: Type::U64,
        };
        let function = HirFunction {
            id: function_id,
            name: "main".to_string(),
            generic_params: Vec::new(),
            generic_bounds: std::collections::HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::U64,
            body,
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(function_id, function.clone())]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([(
                    "main".to_string(),
                    function_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(function_id, "main".to_string())]),
        );
        let mut type_context = crate::type_context::TypeContext::new();
        let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
        let u64_id = type_context.id_for_type(&Type::U64).unwrap();
        let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
        pre_mir_instance_bodies.insert(instance_id, function);
        let mut instances = std::collections::BTreeMap::new();
        instances.insert(
            instance_id,
            InstanceRecord {
                id: instance_id,
                origin: InstanceOrigin::Function(function_id),
                substitution: Vec::new(),
                symbols: crate::mono::InstanceSymbols::new("main".to_string(), "main".to_string()),
                declared: None,
                provided_by_object: false,
                is_specialization: false,
            },
        );
        let monomorphized = MonomorphizedProgram {
            program,
            instances,
            pre_mir_instance_bodies,
            generated_drop_instances: Default::default(),
            type_context,
            source_map: crate::source_map::SemanticSourceMap::default(),
        };

        let mir = MirBuilder::build_monomorphized(&monomorphized);
        let main = mir
            .function(crate::mir::MirFunctionId::Instance(instance_id))
            .unwrap();
        let view = crate::type_context::TypeView::new(&monomorphized.type_context);

        assert_eq!(main.ret_type, u64_id);
        assert!(main.local_decls.iter().any(|local| local.ty == u64_id));
        assert!(
            main.basic_blocks
                .iter()
                .flat_map(|block| &block.statements)
                .any(|stmt| {
                    matches!(
                        &stmt.kind,
                        crate::mir::StatementKind::Assign(_, crate::mir::Rvalue::Cast(_, target))
                            if *target == u64_id && matches!(view.ty(*target), crate::type_context::Ty::U64)
                    )
                })
        );
    }

    #[test]
    fn needs_drop_includes_enums_and_closures() {
        let enum_ty = Type::Enum {
            id: DefId::new(CrateId(0), LocalDefId(60)),
            args: vec![],
        };
        assert!(MirBuilder::needs_drop(&enum_ty));
        assert!(MirBuilder::needs_drop(&Type::function(vec![], Type::Unit)));
    }

    #[test]
    fn closure_rvalue_uses_stable_closure_identity() {
        let function_id = DefId::new(CrateId(0), LocalDefId(50));
        let function = HirFunction {
            body: HirBlock {
                stmts: vec![HirStmt::Expr(expr(
                    HirExprKind::Lambda {
                        params: vec![],
                        body: HirBlock {
                            stmts: vec![],
                            ty: Type::Unit,
                        },
                        captures: vec![],
                    },
                    Type::Unit,
                ))],
                ty: Type::Unit,
            },
            ..empty_function(function_id, "make_closure")
        };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(function_id, function)]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([(
                    "make_closure".to_string(),
                    function_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(function_id, "make_closure".to_string())]),
        );
        let mir = MirBuilder::build(&program);
        assert_mir_agreement_clean(&mir);
        let function = mir
            .function(crate::mir::MirFunctionId::Function(function_id))
            .expect("function MIR should exist");

        assert!(function
            .basic_blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|stmt| matches!(
                &stmt.kind,
                StatementKind::Assign(_, Rvalue::Closure(closure))
                    if closure.id.owner == crate::mir::MirFunctionId::Function(function_id)
                        && closure.display_name.starts_with("lambda_")
            )));
    }

    #[test]
    fn build_function_emits_closure_body_function() {
        let function_id = DefId::new(CrateId(0), LocalDefId(20));
        let owner = crate::mir::MirFunctionId::Function(function_id);
        let lambda = HirExpr {
            kind: HirExprKind::Lambda {
                params: vec![crate::hir::HirParam {
                    name: "x".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::I64,
                    mutable: false,
                    is_ref: false,
                }],
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("x".to_string()),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::I64,
                },
                captures: Vec::new(),
            },
            ty: Type::function(vec![Type::I64], Type::I64),
            span: crate::lexer::Span::test(),
        };
        let function = HirFunction {
            id: function_id,
            name: "make_lambda".to_string(),
            generic_params: Vec::new(),
            generic_bounds: std::collections::HashMap::new().into(),
            params: Vec::new(),
            ret_type: lambda.ty.clone(),
            body: HirBlock {
                stmts: vec![HirStmt::Expr(lambda)],
                ty: Type::function(vec![Type::I64], Type::I64),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(function_id, function)]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([(
                    "make_lambda".to_string(),
                    function_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(function_id, "make_lambda".to_string())]),
        );

        let mir = MirBuilder::build(&program);
        let closure_id = crate::mir::MirClosureId {
            owner,
            local_index: 0,
        };

        assert!(mir
            .function(crate::mir::MirFunctionId::Closure(Box::new(
                closure_id.clone()
            )))
            .is_some());
    }

    #[test]
    fn build_function_binds_closure_body_captures() {
        let function_id = DefId::new(CrateId(0), LocalDefId(21));
        let owner = crate::mir::MirFunctionId::Function(function_id);
        let capture = crate::hir::HirClosureCapture {
            name: "captured".to_string(),
            local_id: crate::ids::HirLocalId(0),
            kind: crate::hir::HirClosureCaptureKind::SharedBorrow,
            mutable: false,
            ty: Type::I64,
        };
        let nested_lambda = HirExpr {
            kind: HirExprKind::Lambda {
                params: Vec::new(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("captured".to_string()),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::I64,
                },
                captures: vec![capture.clone()],
            },
            ty: Type::function(vec![], Type::I64),
            span: crate::lexer::Span::test(),
        };
        let lambda = HirExpr {
            kind: HirExprKind::Lambda {
                params: Vec::new(),
                body: HirBlock {
                    stmts: vec![
                        HirStmt::Let {
                            name: "nested".to_string(),
                            local_id: crate::ids::HirLocalId(1),
                            ty: Type::function(vec![], Type::I64),
                            value: nested_lambda,
                            mutable: false,
                        },
                        HirStmt::Expr(HirExpr {
                            kind: HirExprKind::Var("captured".to_string()),
                            ty: Type::I64,
                            span: crate::lexer::Span::test(),
                        }),
                    ],
                    ty: Type::I64,
                },
                captures: vec![capture],
            },
            ty: Type::function(vec![], Type::I64),
            span: crate::lexer::Span::test(),
        };
        let function = HirFunction {
            id: function_id,
            name: "make_capturing_lambda".to_string(),
            generic_params: Vec::new(),
            generic_bounds: std::collections::HashMap::new().into(),
            params: vec![crate::hir::HirParam {
                name: "captured".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::I64,
                mutable: false,
                is_ref: false,
            }],
            ret_type: lambda.ty.clone(),
            body: HirBlock {
                stmts: vec![HirStmt::Expr(lambda)],
                ty: Type::function(vec![], Type::I64),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(function_id, function)]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([(
                    "make_capturing_lambda".to_string(),
                    function_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(function_id, "make_capturing_lambda".to_string())]),
        );

        let mir = MirBuilder::build(&program);
        let type_context = mir.type_context.clone();
        let closure_id = crate::mir::MirClosureId {
            owner,
            local_index: 0,
        };
        let closure_function = mir
            .function(crate::mir::MirFunctionId::Closure(Box::new(
                closure_id.clone(),
            )))
            .expect("closure MIR body should exist");
        let capture = closure_function
            .closure_captures
            .iter()
            .find(|capture| capture.name == "captured")
            .expect("nested closure body should record captured local");

        assert_eq!(closure_function.arg_count, 0);
        assert_eq!(closure_function.closure_captures.len(), 1);
        assert_eq!(
            type_context.type_for(closure_function.local_decls[capture.local.0].ty),
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            }
        );
        assert_eq!(
            closure_function.local_decls[capture.local.0]
                .name
                .as_deref(),
            Some("captured")
        );
        assert!(closure_function
            .basic_blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|stmt| matches!(
                &stmt.kind,
                StatementKind::Assign(
                    Place {
                        local: Local(0),
                        projection
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local,
                        projection: source_projection,
                    }))
                ) if projection.is_empty()
                    && *local == capture.local
                    && source_projection == &[Projection::Deref]
            )));
    }

    #[test]
    fn builder_output_has_no_callable_unit_placeholders_for_simple_function_refs() {
        let callee_id = DefId::new(CrateId(0), LocalDefId(71));
        let caller_id = DefId::new(CrateId(0), LocalDefId(72));
        let callable_ty = Type::function(vec![], Type::Unit);
        let caller = HirFunction {
            body: HirBlock {
                stmts: vec![HirStmt::Expr(expr(
                    HirExprKind::ResolvedVar(HirVarRef {
                        name: "callee".to_string(),
                        target: HirVarTarget::Function(callee_id),
                    }),
                    callable_ty.clone(),
                ))],
                ty: callable_ty.clone(),
            },
            ret_type: callable_ty,
            ..empty_function(caller_id, "caller")
        };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([
                (callee_id, empty_function(callee_id, "callee")),
                (caller_id, caller),
            ]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([
                    ("callee".to_string(), callee_id),
                    ("caller".to_string(), caller_id),
                ]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([
                (callee_id, "callee".to_string()),
                (caller_id, "caller".to_string()),
            ]),
        );

        let mir = MirBuilder::build(&program);

        assert_mir_agreement_clean(&mir);
    }

    #[test]
    fn value_intrinsic_lowers_to_intrinsic_call_terminator() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.direct_drop_types.insert(struct_ty(0));
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let intrinsic_expr = expr(
            HirExprKind::Intrinsic {
                name: "I64Add".to_string(),
                args: vec![
                    expr(HirExprKind::IntLiteral(1), Type::I64),
                    expr(HirExprKind::IntLiteral(2), Type::I64),
                ],
            },
            Type::I64,
        );

        builder.lower_expr_with_context(
            &intrinsic_expr,
            Place {
                local: Local(0),
                projection: vec![],
            },
            false,
        );

        assert!(matches!(
            &builder.blocks[0].terminator,
            Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    crate::mir::MirCallableKey::Intrinsic(intrinsic)
                ))),
                args,
                destination,
                span: Some(call_span),
                ..
            }) if *intrinsic == MirIntrinsicId::I64Add
                && args.len() == 2
                && destination.local == Local(0)
                && call_span == &intrinsic_expr.span
        ));

        assert_builder_agreement_clean(builder, Type::I64);
    }

    #[test]
    #[should_panic(expected = "unknown intrinsic reached MIR lowering")]
    fn unknown_intrinsic_source_name_panics_before_mir_callable_identity() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::Unit),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let intrinsic_expr = expr(
            HirExprKind::Intrinsic {
                name: "NotARealIntrinsic".to_string(),
                args: vec![],
            },
            Type::Unit,
        );

        builder.lower_expr_with_context(
            &intrinsic_expr,
            Place {
                local: Local(0),
                projection: vec![],
            },
            false,
        );
    }

    #[test]
    fn drop_in_place_lowers_to_mir_drop_terminator() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let pointee_ty = struct_ty(0);
        let ptr_ty = Type::Pointer(Box::new(pointee_ty.clone()));
        let mut builder = builder_with_var(&program, &type_context, "ptr", ptr_ty.clone());
        builder.direct_drop_types.insert(pointee_ty.clone());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::Unit),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let intrinsic_expr = expr(
            HirExprKind::Intrinsic {
                name: "DropInPlace".to_string(),
                args: vec![expr(HirExprKind::Var("ptr".to_string()), ptr_ty)],
            },
            Type::Unit,
        );

        builder.lower_expr_with_context(
            &intrinsic_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
            false,
        );

        assert!(matches!(
            &builder.blocks[0].terminator,
            Some(Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. })
                if place.local == Local(0) && place.projection == vec![Projection::Deref]
        ));

        assert_builder_agreement_clean(builder, Type::Unit);
    }

    #[test]
    fn numeric_cast_lowers_to_runtime_cast_rvalue() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "n", Type::I64);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::F64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let cast_expr = expr(
            HirExprKind::Cast(
                Box::new(expr(HirExprKind::Var("n".to_string()), Type::I64)),
                Type::F64,
            ),
            Type::F64,
        );

        builder.lower_expr_with_context(
            &cast_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
            false,
        );

        let f64_id = builder.type_id_for(&Type::F64);
        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(_, Rvalue::Cast(Operand::Copy(_), ty))
                if *ty == f64_id
        )));

        assert_builder_agreement_clean(builder, Type::Unit);
    }

    #[test]
    fn enum_match_lowers_to_discriminant_switch() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let none_id = VariantId(0);
        let some_id = VariantId(1);
        let program = option_program(vec![
            (none_id, "None", HirVariantFields::Unit),
            (some_id, "Some", HirVariantFields::Unit),
        ]);
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "opt", option_ty());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
                arms: vec![
                    match_arm(
                        HirPattern::Enum(
                            "Option".to_string(),
                            "None".to_string(),
                            Some(HirVariantLocation {
                                owner: enum_id,
                                variant_id: none_id,
                                name: "None".to_string(),
                            }),
                            vec![],
                        ),
                        None,
                        expr(HirExprKind::IntLiteral(0), Type::I64),
                    ),
                    match_arm(
                        HirPattern::Enum(
                            "Option".to_string(),
                            "Some".to_string(),
                            Some(HirVariantLocation {
                                owner: enum_id,
                                variant_id: some_id,
                                name: "Some".to_string(),
                            }),
                            vec![],
                        ),
                        None,
                        expr(HirExprKind::IntLiteral(1), Type::I64),
                    ),
                ],
            },
            Type::I64,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );

        let switch_targets: Vec<i64> = builder
            .blocks
            .iter()
            .filter_map(|block| match &block.terminator {
                Some(
                    Terminator::SwitchInt { targets, .. }
                    | Terminator::SwitchIntWithOrigin { targets, .. },
                ) => Some(targets),
                _ => None,
            })
            .flatten()
            .map(|(value, _)| *value)
            .collect();
        assert!(switch_targets.contains(&(none_id.0 as i64)));
        assert!(switch_targets.contains(&(some_id.0 as i64)));
        assert!(builder
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|stmt| {
                matches!(
                    &stmt.kind,
                    StatementKind::Assign(_, Rvalue::Discriminant(_))
                )
            }));

        assert_builder_agreement_clean(builder, Type::Unit);
    }

    #[test]
    fn guarded_enum_arm_does_not_make_final_enum_arm_exhaustive() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let none_id = VariantId(0);
        let some_id = VariantId(1);
        let program = option_program(vec![
            (none_id, "None", HirVariantFields::Unit),
            (some_id, "Some", HirVariantFields::Unit),
        ]);
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "opt", option_ty());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
                arms: vec![
                    match_arm(
                        HirPattern::Enum(
                            "Option".to_string(),
                            "None".to_string(),
                            Some(HirVariantLocation {
                                owner: enum_id,
                                variant_id: none_id,
                                name: "None".to_string(),
                            }),
                            vec![],
                        ),
                        Some(expr(HirExprKind::BoolLiteral(false), Type::Bool)),
                        expr(HirExprKind::IntLiteral(0), Type::I64),
                    ),
                    match_arm(
                        HirPattern::Enum(
                            "Option".to_string(),
                            "Some".to_string(),
                            Some(HirVariantLocation {
                                owner: enum_id,
                                variant_id: some_id,
                                name: "Some".to_string(),
                            }),
                            vec![],
                        ),
                        None,
                        expr(HirExprKind::IntLiteral(1), Type::I64),
                    ),
                ],
            },
            Type::I64,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );

        let (matched_block, otherwise) = builder
            .blocks
            .iter()
            .find_map(|block| match &block.terminator {
                Some(
                    Terminator::SwitchInt {
                        targets, otherwise, ..
                    }
                    | Terminator::SwitchIntWithOrigin {
                        targets, otherwise, ..
                    },
                ) => targets
                    .iter()
                    .find(|(value, _)| *value == some_id.0 as i64)
                    .map(|(_, matched)| (*matched, *otherwise)),
                _ => None,
            })
            .expect("final enum arm discriminant switch");
        assert_ne!(otherwise, matched_block);

        assert_builder_agreement_clean(builder, Type::Unit);
    }

    #[test]
    fn enum_match_payload_binding_uses_downcast_field_projection() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let some_id = VariantId(1);
        let program = option_program(vec![(
            some_id,
            "Some",
            HirVariantFields::Positional(vec![Type::I64]),
        )]);
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "opt", option_ty());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let value_expr = expr(HirExprKind::Var("value".to_string()), Type::I64);
        let guard = expr(
            HirExprKind::BinOp(
                crate::hir::BinOp::Gt,
                Box::new(value_expr.clone()),
                Box::new(expr(HirExprKind::IntLiteral(0), Type::I64)),
            ),
            Type::Bool,
        );
        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
                arms: vec![match_arm(
                    HirPattern::Enum(
                        "Option".to_string(),
                        "Some".to_string(),
                        Some(HirVariantLocation {
                            owner: enum_id,
                            variant_id: some_id,
                            name: "Some".to_string(),
                        }),
                        vec![HirPattern::Binding {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            mutable: false,
                        }],
                    ),
                    Some(guard),
                    value_expr,
                )],
            },
            Type::I64,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );

        let value_local = builder
            .locals
            .iter()
            .position(|decl| decl.name.as_deref() == Some("value"))
            .map(Local)
            .expect("value binding local");
        assert!(builder
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|stmt| {
                matches!(
                    &stmt.kind,
                    StatementKind::Assign(
                        Place { local, .. },
                        Rvalue::Use(Operand::Copy(Place { projection, .. }))
                    ) if *local == value_local
                        && projection.contains(&Projection::Downcast(some_id))
                        && projection.contains(&Projection::Field { index: 0, identity: None })
                )
            }));
        assert!(builder.blocks.iter().any(|block| matches!(
            &block.terminator,
            Some(
                Terminator::SwitchInt {
                    discr: Operand::Copy(_),
                    ..
                } | Terminator::SwitchIntWithOrigin {
                    discr: Operand::Copy(_),
                    ..
                },
            )
        )));

        let bool_id = builder.type_id_for(&Type::Bool);
        let guard_false_cleanup = builder
            .blocks
            .iter()
            .find_map(|block| match &block.terminator {
                Some(
                    Terminator::SwitchInt {
                        discr: Operand::Copy(place),
                        targets,
                        otherwise,
                    }
                    | Terminator::SwitchIntWithOrigin {
                        discr: Operand::Copy(place),
                        targets,
                        otherwise,
                        ..
                    },
                ) if builder.locals[place.local.0].ty == bool_id
                    && targets.contains(&(1, *otherwise)) == false =>
                {
                    Some(*otherwise)
                }
                _ => None,
            })
            .expect("guard false cleanup block");
        assert!(builder.blocks[guard_false_cleanup.0].statements.iter().any(
            |stmt| matches!(&stmt.kind, StatementKind::StorageDead(local) if *local == value_local)
        ));

        assert_builder_agreement_clean(builder, Type::Unit);
    }

    #[test]
    fn literal_match_uses_value_comparison_not_discriminant() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "n", Type::I64);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("n".to_string()), Type::I64)),
                arms: vec![
                    match_arm(
                        HirPattern::Literal(HirLiteralPattern::Int(0)),
                        None,
                        expr(HirExprKind::IntLiteral(1), Type::I64),
                    ),
                    match_arm(
                        HirPattern::Wildcard,
                        None,
                        expr(HirExprKind::IntLiteral(2), Type::I64),
                    ),
                ],
            },
            Type::I64,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );

        assert!(!builder
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|stmt| {
                matches!(
                    &stmt.kind,
                    StatementKind::Assign(_, Rvalue::Discriminant(_))
                )
            }));
        assert!(builder
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|stmt| {
                matches!(
                    &stmt.kind,
                    StatementKind::Assign(
                        _,
                        Rvalue::BinaryOp(
                            crate::mir::MirBinOp::Eq,
                            _,
                            Operand::Constant(Constant::Int(0))
                        )
                    )
                )
            }));
    }

    #[test]
    fn enum_match_literal_payload_pattern_lowers_payload_check_before_body() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let none_id = VariantId(0);
        let some_id = VariantId(1);
        let program = option_program(vec![
            (none_id, "None", HirVariantFields::Unit),
            (
                some_id,
                "Some",
                HirVariantFields::Positional(vec![Type::I64]),
            ),
        ]);
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "opt", option_ty());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
                arms: vec![
                    match_arm(
                        HirPattern::Enum(
                            "Option".to_string(),
                            "Some".to_string(),
                            Some(HirVariantLocation {
                                owner: enum_id,
                                variant_id: some_id,
                                name: "Some".to_string(),
                            }),
                            vec![HirPattern::Literal(HirLiteralPattern::Int(0))],
                        ),
                        None,
                        expr(HirExprKind::IntLiteral(1), Type::I64),
                    ),
                    match_arm(
                        HirPattern::Wildcard,
                        None,
                        expr(HirExprKind::IntLiteral(2), Type::I64),
                    ),
                ],
            },
            Type::I64,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );

        let payload_check = builder
            .blocks
            .iter()
            .enumerate()
            .find_map(|(block_index, block)| {
                block.statements.iter().find_map(|stmt| match &stmt.kind {
                    StatementKind::Assign(
                        bool_place,
                        Rvalue::BinaryOp(
                            crate::mir::MirBinOp::Eq,
                            Operand::Copy(Place { projection, .. }),
                            Operand::Constant(Constant::Int(0)),
                        ),
                    ) if projection.contains(&Projection::Downcast(some_id))
                        && projection.contains(&Projection::Field {
                            index: 0,
                            identity: None,
                        }) =>
                    {
                        Some((BasicBlockId(block_index), bool_place.clone()))
                    }
                    _ => None,
                })
            })
            .expect("payload literal comparison");
        let (payload_check_block, payload_bool_place) = payload_check;
        let discriminant_success = builder
            .blocks
            .iter()
            .find_map(|block| match &block.terminator {
                Some(
                    Terminator::SwitchInt { targets, .. }
                    | Terminator::SwitchIntWithOrigin { targets, .. },
                ) => targets
                    .iter()
                    .find_map(|(value, target)| (*value == some_id.0 as i64).then_some(*target)),
                _ => None,
            })
            .expect("discriminant Some target");
        assert_eq!(discriminant_success, payload_check_block);

        let body_entry = match &builder.blocks[payload_check_block.0].terminator {
            Some(
                Terminator::SwitchInt {
                    discr: Operand::Copy(discr),
                    targets,
                    otherwise,
                }
                | Terminator::SwitchIntWithOrigin {
                    discr: Operand::Copy(discr),
                    targets,
                    otherwise,
                    ..
                },
            ) if *discr == payload_bool_place => (
                targets
                    .iter()
                    .find_map(|(value, target)| (*value == 1).then_some(*target))
                    .expect("payload comparison success target"),
                *otherwise,
            ),
            terminator => panic!("expected payload bool SwitchInt, got {terminator:?}"),
        };
        let (payload_success_entry, payload_failure_entry) = body_entry;
        let body_block = match &builder.blocks[payload_success_entry.0].terminator {
            Some(Terminator::Goto(target) | Terminator::GotoWithOrigin { target, .. }) => *target,
            terminator => panic!("expected payload success to enter body, got {terminator:?}"),
        };
        let fallback_matched_block = match &builder.blocks[payload_failure_entry.0].terminator {
            Some(Terminator::Goto(target) | Terminator::GotoWithOrigin { target, .. }) => *target,
            terminator => panic!("expected payload failure to reach fallback, got {terminator:?}"),
        };
        let fallback_body_block = match &builder.blocks[fallback_matched_block.0].terminator {
            Some(Terminator::Goto(target) | Terminator::GotoWithOrigin { target, .. }) => *target,
            terminator => {
                panic!("expected fallback matched block to enter body, got {terminator:?}")
            }
        };
        assert!(builder.blocks[body_block.0].statements.iter().any(|stmt| {
            matches!(
                &stmt.kind,
                StatementKind::Assign(
                    Place {
                        local: Local(1),
                        projection,
                    },
                    Rvalue::Use(Operand::Constant(Constant::Int(1)))
                ) if projection.is_empty()
            )
        }));
        assert!(builder.blocks[fallback_body_block.0]
            .statements
            .iter()
            .any(|stmt| {
                matches!(
                    &stmt.kind,
                    StatementKind::Assign(
                        Place {
                            local: Local(1),
                            projection,
                        },
                        Rvalue::Use(Operand::Constant(Constant::Int(2)))
                    ) if projection.is_empty()
                )
            }));

        assert_builder_agreement_clean(builder, Type::Unit);
    }

    #[test]
    #[should_panic(expected = "unsupported refutable enum payload pattern")]
    fn enum_match_string_payload_pattern_panics_loudly() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let some_id = VariantId(1);
        let program = option_program(vec![(
            some_id,
            "Some",
            HirVariantFields::Positional(vec![Type::Str]),
        )]);
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "opt", option_ty());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
                arms: vec![
                    match_arm(
                        HirPattern::Enum(
                            "Option".to_string(),
                            "Some".to_string(),
                            Some(HirVariantLocation {
                                owner: enum_id,
                                variant_id: some_id,
                                name: "Some".to_string(),
                            }),
                            vec![HirPattern::Literal(HirLiteralPattern::String(
                                "x".to_string(),
                            ))],
                        ),
                        None,
                        expr(HirExprKind::IntLiteral(1), Type::I64),
                    ),
                    match_arm(
                        HirPattern::Wildcard,
                        None,
                        expr(HirExprKind::IntLiteral(2), Type::I64),
                    ),
                ],
            },
            Type::I64,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );
    }

    #[test]
    #[should_panic(expected = "unsupported refutable enum payload pattern")]
    fn enum_match_float_payload_pattern_panics_loudly() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let some_id = VariantId(1);
        let program = option_program(vec![(
            some_id,
            "Some",
            HirVariantFields::Positional(vec![Type::F32]),
        )]);
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "opt", option_ty());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
                arms: vec![
                    match_arm(
                        HirPattern::Enum(
                            "Option".to_string(),
                            "Some".to_string(),
                            Some(HirVariantLocation {
                                owner: enum_id,
                                variant_id: some_id,
                                name: "Some".to_string(),
                            }),
                            vec![HirPattern::Literal(HirLiteralPattern::Float(1.5))],
                        ),
                        None,
                        expr(HirExprKind::IntLiteral(1), Type::I64),
                    ),
                    match_arm(
                        HirPattern::Wildcard,
                        None,
                        expr(HirExprKind::IntLiteral(2), Type::I64),
                    ),
                ],
            },
            Type::I64,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );
    }

    #[test]
    fn guarded_enum_payload_binding_gracefully_skips_non_copy_payload() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let some_id = VariantId(1);
        let payload_ty = struct_ty(0);
        let program = option_program(vec![(
            some_id,
            "Some",
            HirVariantFields::Positional(vec![payload_ty.clone()]),
        )]);

        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "opt", option_ty());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&payload_ty.clone()),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let value_expr = expr(HirExprKind::Var("value".to_string()), payload_ty.clone());
        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
                arms: vec![match_arm(
                    HirPattern::Enum(
                        "Option".to_string(),
                        "Some".to_string(),
                        Some(HirVariantLocation {
                            owner: enum_id,
                            variant_id: some_id,
                            name: "Some".to_string(),
                        }),
                        vec![HirPattern::Binding {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            mutable: false,
                        }],
                    ),
                    Some(expr(HirExprKind::BoolLiteral(false), Type::Bool)),
                    value_expr,
                )],
            },
            payload_ty,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );
    }

    #[test]
    fn unguarded_enum_payload_binding_moves_non_copy_payload_without_dropping_scrutinee() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let some_id = VariantId(1);
        let payload_ty = struct_ty(0);
        let program = option_program(vec![(
            some_id,
            "Some",
            HirVariantFields::Positional(vec![payload_ty.clone()]),
        )]);

        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "opt", option_ty());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&payload_ty.clone()),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let value_expr = expr(HirExprKind::Var("value".to_string()), payload_ty.clone());
        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
                arms: vec![match_arm(
                    HirPattern::Enum(
                        "Option".to_string(),
                        "Some".to_string(),
                        Some(HirVariantLocation {
                            owner: enum_id,
                            variant_id: some_id,
                            name: "Some".to_string(),
                        }),
                        vec![HirPattern::Binding {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            mutable: false,
                        }],
                    ),
                    None,
                    value_expr,
                )],
            },
            payload_ty,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );

        let value_local = builder
            .locals
            .iter()
            .position(|decl| decl.name.as_deref() == Some("value"))
            .map(Local)
            .expect("value binding local");
        let moved_scrutinee = builder
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|stmt| match &stmt.kind {
                StatementKind::Assign(
                    Place { local, .. },
                    Rvalue::Use(Operand::Move(Place { local: source, .. })),
                ) if *local == value_local => Some(*source),
                _ => None,
            })
            .expect("non-copy payload binding should move from scrutinee temp");
        assert!(!builder.blocks.iter().any(|block| matches!(
            &block.terminator,
            Some(Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. })
                if place.local == moved_scrutinee
        )));
    }

    #[test]
    fn moving_later_enum_arm_does_not_suppress_nonmoving_arm_scrutinee_cleanup() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let none_id = VariantId(0);
        let some_id = VariantId(1);
        let payload_ty = struct_ty(0);
        let program = option_program(vec![
            (none_id, "None", HirVariantFields::Unit),
            (
                some_id,
                "Some",
                HirVariantFields::Positional(vec![payload_ty.clone()]),
            ),
        ]);
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "opt", option_ty());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let value_expr = expr(HirExprKind::Var("value".to_string()), payload_ty.clone());
        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
                arms: vec![
                    match_arm(
                        HirPattern::Enum(
                            "Option".to_string(),
                            "None".to_string(),
                            Some(HirVariantLocation {
                                owner: enum_id,
                                variant_id: none_id,
                                name: "None".to_string(),
                            }),
                            vec![],
                        ),
                        None,
                        expr(HirExprKind::IntLiteral(0), Type::I64),
                    ),
                    match_arm(
                        HirPattern::Enum(
                            "Option".to_string(),
                            "Some".to_string(),
                            Some(HirVariantLocation {
                                owner: enum_id,
                                variant_id: some_id,
                                name: "Some".to_string(),
                            }),
                            vec![HirPattern::Binding {
                                name: "value".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                mutable: false,
                            }],
                        ),
                        None,
                        value_expr,
                    ),
                ],
            },
            Type::I64,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );

        let scrutinee_local = builder
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|stmt| match &stmt.kind {
                StatementKind::Assign(_, Rvalue::Discriminant(Place { local, .. })) => Some(*local),
                _ => None,
            })
            .expect("match discriminant should read scrutinee temp");
        assert!(builder.blocks.iter().any(|block| {
            block
                .statements
                .iter()
                .any(|stmt| matches!(&stmt.kind, StatementKind::StorageDead(local) if *local == scrutinee_local))
        }));
    }

    #[test]
    fn returning_enum_match_arm_cleans_match_temps_before_return() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let none_id = VariantId(0);
        let program = option_program(vec![(none_id, "None", HirVariantFields::Unit)]);
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&option_ty()),
            mutability: Mutability::Not,
            name: Some("opt".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.var_map.insert("opt".to_string(), Local(1));

        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
                arms: vec![HirMatchArm {
                    pattern: HirPattern::Enum(
                        "Option".to_string(),
                        "None".to_string(),
                        Some(HirVariantLocation {
                            owner: enum_id,
                            variant_id: none_id,
                            name: "None".to_string(),
                        }),
                        vec![],
                    ),
                    guard: None,
                    body: HirBlock {
                        stmts: vec![HirStmt::Return(Some(expr(
                            HirExprKind::IntLiteral(0),
                            Type::I64,
                        )))],
                        ty: Type::Never,
                    },
                }],
            },
            Type::I64,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(2),
                projection: vec![],
            },
        );

        let (discr_local, scrutinee_local) = builder
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|stmt| match &stmt.kind {
                StatementKind::Assign(
                    Place { local: discr, .. },
                    Rvalue::Discriminant(Place {
                        local: scrutinee, ..
                    }),
                ) => Some((*discr, *scrutinee)),
                _ => None,
            })
            .expect("match should assign discriminant from scrutinee temp");

        let return_block = builder
            .blocks
            .iter()
            .find(|block| {
                matches!(
                    block.terminator,
                    Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
                )
            })
            .expect("return terminator block");
        assert!(return_block.statements.iter().any(
            |stmt| matches!(&stmt.kind, StatementKind::StorageDead(local) if *local == discr_local)
        ));
        assert!(return_block
            .statements
            .iter()
            .any(|stmt| matches!(&stmt.kind, StatementKind::StorageDead(local) if *local == scrutinee_local)));

        assert_builder_agreement_clean(builder, Type::Unit);
    }

    #[test]
    fn returning_enum_match_guard_cleans_match_temps_before_return() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let none_id = VariantId(0);
        let program = option_program(vec![(none_id, "None", HirVariantFields::Unit)]);
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&option_ty()),
            mutability: Mutability::Not,
            name: Some("opt".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.var_map.insert("opt".to_string(), Local(1));

        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
                arms: vec![HirMatchArm {
                    pattern: HirPattern::Enum(
                        "Option".to_string(),
                        "None".to_string(),
                        Some(HirVariantLocation {
                            owner: enum_id,
                            variant_id: none_id,
                            name: "None".to_string(),
                        }),
                        vec![],
                    ),
                    guard: Some(expr(
                        HirExprKind::Block(HirBlock {
                            stmts: vec![HirStmt::Return(Some(expr(
                                HirExprKind::IntLiteral(0),
                                Type::I64,
                            )))],
                            ty: Type::Bool,
                        }),
                        Type::Bool,
                    )),
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(expr(HirExprKind::IntLiteral(1), Type::I64))],
                        ty: Type::I64,
                    },
                }],
            },
            Type::I64,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(2),
                projection: vec![],
            },
        );

        let (discr_local, scrutinee_local) = builder
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|stmt| match &stmt.kind {
                StatementKind::Assign(
                    Place { local: discr, .. },
                    Rvalue::Discriminant(Place {
                        local: scrutinee, ..
                    }),
                ) => Some((*discr, *scrutinee)),
                _ => None,
            })
            .expect("match should assign discriminant from scrutinee temp");

        let return_block = builder
            .blocks
            .iter()
            .find(|block| {
                matches!(
                    block.terminator,
                    Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
                )
            })
            .expect("return terminator block");
        assert!(return_block.statements.iter().any(
            |stmt| matches!(&stmt.kind, StatementKind::StorageDead(local) if *local == discr_local)
        ));
        assert!(return_block
            .statements
            .iter()
            .any(|stmt| matches!(&stmt.kind, StatementKind::StorageDead(local) if *local == scrutinee_local)));

        assert_builder_agreement_clean(builder, Type::Unit);
    }

    #[test]
    fn generic_enum_payload_binding_moves_without_dropping_scrutinee() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let some_id = VariantId(1);
        let payload_ty = Type::Generic(crate::types::GenericParamId {
            owner: enum_id,
            index: 0,
        });
        let option_ty = Type::Enum {
            id: enum_id,
            args: vec![payload_ty.clone()],
        };
        let program = option_program(vec![(
            some_id,
            "Some",
            HirVariantFields::Positional(vec![payload_ty.clone()]),
        )]);

        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "opt", option_ty.clone());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&payload_ty.clone()),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let value_expr = expr(HirExprKind::Var("value".to_string()), payload_ty.clone());
        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty)),
                arms: vec![match_arm(
                    HirPattern::Enum(
                        "Option".to_string(),
                        "Some".to_string(),
                        Some(HirVariantLocation {
                            owner: enum_id,
                            variant_id: some_id,
                            name: "Some".to_string(),
                        }),
                        vec![HirPattern::Binding {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            mutable: false,
                        }],
                    ),
                    None,
                    value_expr,
                )],
            },
            payload_ty,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );

        let value_local = builder
            .locals
            .iter()
            .position(|decl| decl.name.as_deref() == Some("value"))
            .map(Local)
            .expect("value binding local");
        let moved_scrutinee = builder
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|stmt| match &stmt.kind {
                StatementKind::Assign(
                    Place { local, .. },
                    Rvalue::Use(Operand::Move(Place { local: source, .. })),
                ) if *local == value_local => Some(*source),
                _ => None,
            })
            .expect("generic payload binding should move from scrutinee temp");
        assert!(!builder.blocks.iter().any(|block| matches!(
            &block.terminator,
            Some(Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. })
                if place.local == moved_scrutinee
        )));

        let report =
            builder_agreement_report_with_contract(builder, Type::Unit, Default::default());
        assert!(report.invalid_type_ids > 0);
    }

    #[test]
    fn function_generic_enum_payload_binding_moves_without_dropping_scrutinee() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(40));
        let function_id = DefId::new(CrateId(0), LocalDefId(41));
        let some_id = VariantId(1);
        let payload_ty = Type::Generic(crate::types::GenericParamId {
            owner: function_id,
            index: 0,
        });
        let option_ty = Type::Enum {
            id: enum_id,
            args: vec![payload_ty.clone()],
        };
        let program = option_program(vec![(
            some_id,
            "Some",
            HirVariantFields::Positional(vec![payload_ty.clone()]),
        )]);

        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "opt", option_ty.clone());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&payload_ty.clone()),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let value_expr = expr(HirExprKind::Var("value".to_string()), payload_ty.clone());
        let match_expr = expr(
            HirExprKind::Match {
                scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty)),
                arms: vec![match_arm(
                    HirPattern::Enum(
                        "Option".to_string(),
                        "Some".to_string(),
                        Some(HirVariantLocation {
                            owner: enum_id,
                            variant_id: some_id,
                            name: "Some".to_string(),
                        }),
                        vec![HirPattern::Binding {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            mutable: false,
                        }],
                    ),
                    None,
                    value_expr,
                )],
            },
            payload_ty,
        );

        builder.lower_expr(
            &match_expr,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );

        let value_local = builder
            .locals
            .iter()
            .position(|decl| decl.name.as_deref() == Some("value"))
            .map(Local)
            .expect("value binding local");
        let moved_scrutinee = builder
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|stmt| match &stmt.kind {
                StatementKind::Assign(
                    Place { local, .. },
                    Rvalue::Use(Operand::Move(Place { local: source, .. })),
                ) if *local == value_local => Some(*source),
                _ => None,
            })
            .expect("function generic payload binding should move from scrutinee temp");
        assert!(!builder.blocks.iter().any(|block| matches!(
            &block.terminator,
            Some(Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. })
                if place.local == moved_scrutinee
        )));

        let report =
            builder_agreement_report_with_contract(builder, Type::Unit, Default::default());
        assert!(report.invalid_type_ids > 0);
    }

    #[test]
    fn cleanup_drops_before_storage_dead_for_drop_needed_local() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.direct_drop_types.insert(struct_ty(0));
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&struct_ty(0)),
            mutability: Mutability::Not,
            name: Some("value".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        builder.finish_scope_locals(vec![Local(0)]);

        let drop_target = match &builder.blocks[0].terminator {
            Some(
                Terminator::Drop { place, target, .. }
                | Terminator::DropWithOrigin { place, target, .. },
            ) if place.local == Local(0) => *target,
            other => panic!("expected drop terminator before storage dead, got {other:?}"),
        };
        assert!(!builder.blocks[0]
            .statements
            .iter()
            .any(|stmt| matches!(&stmt.kind, StatementKind::StorageDead(Local(0)))));
        assert!(builder.blocks[drop_target.0]
            .statements
            .iter()
            .any(|stmt| matches!(&stmt.kind, StatementKind::StorageDead(Local(0)))));
    }

    #[test]
    fn recursive_field_drop_emits_root_drop_before_field_drops() {
        let inner_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(1));
        let outer_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0));
        let drop_trait_id =
            crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(2));
        let drop_trait_method_id =
            crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(3));
        let outer_drop_impl_id =
            crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(4));
        let outer_drop_method_id =
            crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(5));
        let inner_drop_impl_id =
            crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(6));
        let inner_drop_method_id =
            crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(7));
        let outer_ty = Type::Struct {
            id: outer_id,
            args: vec![],
        };
        let inner_ty = Type::Struct {
            id: inner_id,
            args: vec![],
        };
        let drop_function = |id, name: &str, ty: Type| {
            let mut function = empty_function(id, name);
            function.params.push(HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty,
                mutable: false,
                is_ref: false,
            });
            function.is_method = true;
            function.self_receiver = Some(crate::types::ReceiverMode::Move);
            function
        };

        let mut structs = std::collections::HashMap::new();
        structs.insert(
            inner_id,
            HirStruct {
                id: inner_id,
                name: "Inner".to_string(),
                generic_params: vec![],
                fields: vec![HirField {
                    id: crate::ids::FieldId(0),
                    name: "x".to_string(),
                    ty: Type::I64,
                    public: true,
                }],
            },
        );
        structs.insert(
            outer_id,
            HirStruct {
                id: outer_id,
                name: "Outer".to_string(),
                generic_params: vec![],
                fields: vec![HirField {
                    id: crate::ids::FieldId(0),
                    name: "inner".to_string(),
                    ty: inner_ty.clone(),
                    public: true,
                }],
            },
        );
        let mut program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            structs,
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(
                drop_trait_id,
                HirTrait {
                    target: None,
                    predicates: Vec::new(),
                    id: drop_trait_id,
                    name: "Drop".to_string(),
                    generic_params: vec![],
                    associated_types: vec![],
                    methods: std::collections::HashMap::from([(
                        "drop".to_string(),
                        empty_function(drop_trait_method_id, "drop"),
                    )]),
                    signatures: std::collections::HashMap::new(),
                },
            )]),
            std::collections::HashMap::from([
                (
                    outer_drop_impl_id,
                    HirImpl {
                        id: outer_drop_impl_id,
                        owner: HirImplOwner::Named("Outer".to_string()),
                        type_name: "Outer".to_string(),
                        type_generics: vec![],
                        receiver_pattern: vec![].into(),
                        trait_name: Some("Drop".to_string()),
                        trait_id: Some(drop_trait_id),
                        trait_generics: vec![],
                        trait_arg_types: vec![],
                        associated_types: vec![],
                        bounds: std::collections::HashMap::new().into(),
                        methods: std::collections::HashMap::from([(
                            "drop".to_string(),
                            drop_function(outer_drop_method_id, "drop", outer_ty.clone()),
                        )]),
                    },
                ),
                (
                    inner_drop_impl_id,
                    HirImpl {
                        id: inner_drop_impl_id,
                        owner: HirImplOwner::Named("Inner".to_string()),
                        type_name: "Inner".to_string(),
                        type_generics: vec![],
                        receiver_pattern: vec![].into(),
                        trait_name: Some("Drop".to_string()),
                        trait_id: Some(drop_trait_id),
                        trait_generics: vec![],
                        trait_arg_types: vec![],
                        associated_types: vec![],
                        bounds: std::collections::HashMap::new().into(),
                        methods: std::collections::HashMap::from([(
                            "drop".to_string(),
                            drop_function(inner_drop_method_id, "drop", inner_ty.clone()),
                        )]),
                    },
                ),
            ]),
            std::collections::HashMap::new(),
            HirNameTables {
                structs_by_name: std::collections::HashMap::from([
                    ("Inner".to_string(), inner_id),
                    ("Outer".to_string(), outer_id),
                ]),
                traits_by_name: std::collections::HashMap::from([(
                    "stdlib::drop::Drop".to_string(),
                    drop_trait_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([
                (inner_id, "Inner".to_string()),
                (outer_id, "Outer".to_string()),
                (drop_trait_id, "Drop".to_string()),
            ]),
        );
        program.language_items.drop = Some(crate::language_items::DropLanguageItems {
            trait_id: drop_trait_id,
            method_id: drop_trait_method_id,
        });

        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::with_instance_metadata(
            &program,
            &type_context,
            HashMap::new(),
            HashSet::from([outer_ty.clone(), inner_ty.clone()]),
        );
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&outer_ty),
            mutability: Mutability::Not,
            name: Some("value".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        builder.finish_scope_locals(vec![Local(0)]);

        let drops: Vec<_> = builder
            .blocks
            .iter()
            .filter_map(|block| match &block.terminator {
                Some(Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. }) => {
                    Some(place.clone())
                }
                _ => None,
            })
            .collect();

        assert!(
            drops.len() >= 2,
            "expected at least 2 Drop terminators (field + root), got {}",
            drops.len()
        );

        let inner_place = Place {
            local: Local(0),
            projection: vec![crate::mir::Projection::Field {
                index: 0,
                identity: None,
            }],
        };
        let root_place = Place {
            local: Local(0),
            projection: vec![],
        };

        let inner_drop_index = drops.iter().position(|p| *p == inner_place);
        let root_drop_index = drops.iter().position(|p| *p == root_place);
        assert!(
            inner_drop_index.is_some(),
            "expected Drop for inner field (projection [Field(0)]), found drops: {drops:?}"
        );
        assert!(
            root_drop_index.is_some(),
            "expected Drop for root, found drops: {drops:?}"
        );

        let inner_idx = inner_drop_index.unwrap();
        let root_idx = root_drop_index.unwrap();
        assert!(
            root_idx < inner_idx,
            "expected root Drop ({root_idx}) before field Drop ({inner_idx})"
        );

        assert_builder_agreement_clean(builder, Type::Unit);
    }

    #[test]
    fn resolved_function_var_lowers_to_callable_constant() {
        let id = DefId::new(CrateId(0), LocalDefId(11));
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(id, empty_function(id, "make"))]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([("make".to_string(), id)]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(id, "make".to_string())]),
        );
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::Unit),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        let resolved = expr(
            HirExprKind::ResolvedVar(HirVarRef {
                name: "make".to_string(),
                target: HirVarTarget::Function(id),
            }),
            Type::Unit,
        );

        builder.lower_expr(
            &resolved,
            Place {
                local: Local(0),
                projection: vec![],
            },
        );

        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(
                _,
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Callable(
                    crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Function(target))
                )))
            ) if *target == id
        )));
    }

    #[test]
    fn resolved_instance_var_lowers_to_callable_constant_without_name_lookup() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::Unit),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let instance = InstanceId(3);
        let resolved = expr(
            HirExprKind::ResolvedVar(HirVarRef {
                name: "generic".to_string(),
                target: HirVarTarget::Instance(instance),
            }),
            Type::Unit,
        );

        builder.lower_expr(
            &resolved,
            Place {
                local: Local(0),
                projection: vec![],
            },
        );

        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(
                _,
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Callable(
                    crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Instance(target))
                )))
            ) if *target == instance
        )));
    }

    #[test]
    fn named_callable_does_not_use_stale_function_name_table_owner() {
        let stale_id = DefId::new(CrateId(0), LocalDefId(42));
        let mut program = empty_program();
        program
            .names
            .functions_by_name
            .insert("ghost".to_string(), stale_id);
        let type_context = test_type_context(&program);
        let builder = MirBuilder::new(&program, &type_context);

        assert_eq!(
            builder.callable_for_expr(
                &expr(HirExprKind::Var("ghost".to_string()), Type::Unit),
                &Type::Unit,
            ),
            None
        );
    }

    #[test]
    fn named_callable_does_not_resolve_function_from_display_name_table() {
        let id = DefId::new(CrateId(0), LocalDefId(43));
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(id, empty_function(id, "make"))]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                functions_by_name: std::collections::HashMap::from([("make".to_string(), id)]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(id, "make".to_string())]),
        );
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::function(Vec::new(), Type::Unit)),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        builder.lower_expr(
            &expr(
                HirExprKind::Var("make".to_string()),
                Type::function(Vec::new(), Type::Unit),
            ),
            Place {
                local: Local(0),
                projection: vec![],
            },
        );

        assert!(builder.blocks[0].statements.iter().all(|stmt| !matches!(
            &stmt.kind,
            StatementKind::Assign(
                _,
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Callable(
                    crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Function(target))
                )))
            ) if *target == id
        )));
    }

    #[test]
    fn named_callable_does_not_resolve_extern_from_display_name_table() {
        let id = DefId::new(CrateId(0), LocalDefId(44));
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(
                id,
                HirExtern {
                    id,
                    name: "puts".to_string(),
                    params: vec![Type::Str],
                    ret: Type::I64,
                    variadic: false,
                    is_unsafe: false,
                },
            )]),
            HirNameTables {
                externs_by_name: std::collections::HashMap::from([("puts".to_string(), id)]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::new(),
        );
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::function(vec![Type::Str], Type::I64)),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });

        builder.lower_expr(
            &expr(
                HirExprKind::Var("puts".to_string()),
                Type::function(vec![Type::Str], Type::I64),
            ),
            Place {
                local: Local(0),
                projection: vec![],
            },
        );

        assert!(builder.blocks[0].statements.iter().all(|stmt| !matches!(
            &stmt.kind,
            StatementKind::Assign(
                _,
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Callable(
                    crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Extern(target))
                )))
            ) if *target == id
        )));
    }

    #[test]
    fn unresolved_impl_function_var_does_not_select_instance_by_name() {
        let impl_id = DefId::new(CrateId(0), LocalDefId(50));
        let method_id = DefId::new(CrateId(0), LocalDefId(51));
        let instance = InstanceId(5);
        let method = HirFunction {
            id: method_id,
            name: "from_str".to_string(),
            generic_params: Vec::new(),
            generic_bounds: std::collections::HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::I64,
            body: HirBlock {
                stmts: vec![HirStmt::Expr(expr(HirExprKind::IntLiteral(0), Type::I64))],
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(
                impl_id,
                HirImpl {
                    id: impl_id,
                    owner: HirImplOwner::Named("String".to_string()),
                    type_name: "String".to_string(),
                    type_generics: Vec::new(),
                    receiver_pattern: Vec::new().into(),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: std::collections::HashMap::from([("from_str".to_string(), method)]),
                },
            )]),
            std::collections::HashMap::new(),
            HirNameTables::default(),
            &std::collections::HashMap::new(),
        );
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::with_instance_metadata(
            &program,
            &type_context,
            std::collections::HashMap::new(),
            std::collections::HashSet::new(),
        );
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::function(Vec::new(), Type::I64)),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let unresolved = expr(
            HirExprKind::Var("String_from_str".to_string()),
            Type::function(Vec::new(), Type::I64),
        );

        builder.lower_expr(
            &unresolved,
            Place {
                local: Local(0),
                projection: vec![],
            },
        );

        assert!(builder.blocks[0].statements.iter().all(|stmt| !matches!(
            &stmt.kind,
            StatementKind::Assign(
                _,
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Callable(
                    crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Instance(target))
                )))
            ) if *target == instance
        )));
    }

    #[test]
    #[should_panic(expected = "unmaterialized method call reached MIR lowering")]
    fn method_call_terminator_uses_selected_method_callable() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "value", Type::I64);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let impl_id = DefId::new(CrateId(0), LocalDefId(21));
        let trait_id = DefId::new(CrateId(0), LocalDefId(22));
        let method_id = DefId::new(CrateId(0), LocalDefId(23));
        let call = expr(
            HirExprKind::MethodCall(
                Box::new(expr(HirExprKind::Var("value".to_string()), Type::I64)),
                "show".to_string(),
                vec![],
                Some(crate::types::ReceiverMode::Move),
                crate::hir::HirMethodCallTarget::impl_method(
                    impl_id,
                    method_id,
                    Some(crate::hir::HirSelectedTraitMember {
                        trait_id,
                        member_id: method_id,
                        trait_args: vec![Type::I64],
                    }),
                ),
            ),
            Type::I64,
        );

        builder.lower_expr(
            &call,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );
    }

    #[test]
    #[should_panic(expected = "unmaterialized method call reached MIR lowering")]
    fn mir_rejects_method_call_with_selected_authority() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "value", Type::I64);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let impl_id = DefId::new(CrateId(0), LocalDefId(31));
        let method_id = DefId::new(CrateId(0), LocalDefId(32));
        let call = expr(
            HirExprKind::MethodCall(
                Box::new(expr(HirExprKind::Var("value".to_string()), Type::I64)),
                "show".to_string(),
                vec![],
                Some(crate::types::ReceiverMode::Move),
                crate::hir::HirMethodCallTarget::impl_method(impl_id, method_id, None),
            ),
            Type::I64,
        );

        builder.lower_expr(
            &call,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );
    }

    #[test]
    fn monomorphized_method_call_reuses_instance_callable_target() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let instance = crate::ids::InstanceId(9);
        let callee = expr(
            HirExprKind::ResolvedVar(HirVarRef {
                name: "Box::unwrap".to_string(),
                target: HirVarTarget::Instance(instance),
            }),
            Type::function(vec![], Type::I64),
        );
        let call = expr(HirExprKind::Call(Box::new(callee), vec![], None), Type::I64);

        builder.lower_expr(
            &call,
            Place {
                local: Local(0),
                projection: vec![],
            },
        );

        assert!(builder.blocks.iter().any(|block| matches!(
            &block.terminator,
            Some(crate::mir::Terminator::Call {
                func: Operand::Constant(crate::mir::Constant::Callable(
                    crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Instance(target))
                )),
                ..
            }) if *target == instance
        )));

        let i64_ty = builder.type_id_for(&Type::I64);
        let backend_contract = {
            let type_context = type_context.borrow();
            instance_callable_contract(
                instance,
                DefId::new(CrateId(0), LocalDefId(9)),
                "Box::unwrap",
                true,
                vec![],
                i64_ty,
                &type_context,
            )
        };
        assert_builder_agreement_clean_with_contract(builder, Type::Unit, backend_contract);
    }

    #[test]
    fn function_call_argument_temp_moves_noncopy_local() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        let value_ty = struct_ty(77);
        let function_ty = Type::function(vec![value_ty.clone()], Type::Unit);
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::Unit),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&function_ty),
            mutability: Mutability::Not,
            name: Some("f".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&value_ty),
            mutability: Mutability::Not,
            name: Some("value".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.var_map.insert("f".to_string(), Local(1));
        builder.var_map.insert("value".to_string(), Local(2));
        builder.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        let call = expr(
            HirExprKind::Call(
                Box::new(expr(HirExprKind::Var("f".to_string()), function_ty)),
                vec![expr(HirExprKind::Var("value".to_string()), value_ty)],
                None,
            ),
            Type::Unit,
        );

        builder.lower_expr(
            &call,
            Place {
                local: Local(0),
                projection: vec![],
            },
        );

        let Some(crate::mir::Terminator::Call { args, .. }) = &builder.blocks[0].terminator else {
            panic!("expected a call terminator");
        };
        let Operand::Move(argument_place) = &args[0] else {
            panic!("non-copy argument must be moved into the call");
        };
        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(
                destination,
                Rvalue::Use(Operand::Move(Place {
                    local: Local(2),
                    projection: source_projection,
                }))
            ) if destination == argument_place && source_projection.is_empty()
        )));
    }

    #[test]
    fn method_receiver_mode_metadata_is_instance_keyed() {
        let struct_id = DefId::new(CrateId(0), LocalDefId(10));
        let impl_id = DefId::new(CrateId(0), LocalDefId(20));
        let method_id = DefId::new(CrateId(0), LocalDefId(21));
        let i64_instance = InstanceId(1);
        let bool_instance = InstanceId(2);
        let generic = Type::Generic(crate::types::GenericParamId {
            owner: impl_id,
            index: 0,
        });
        let method = HirFunction {
            id: method_id,
            name: "get".to_string(),
            generic_params: Vec::new(),
            generic_bounds: std::collections::HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: generic.clone(),
                mutable: false,
                is_ref: false,
            }],
            ret_type: generic.clone(),
            body: HirBlock {
                stmts: vec![HirStmt::Expr(expr(
                    HirExprKind::Var("self".to_string()),
                    generic.clone(),
                ))],
                ty: generic.clone(),
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(crate::types::ReceiverMode::Shared),
            is_unsafe: false,
        };
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(
                struct_id,
                HirStruct {
                    id: struct_id,
                    name: "Box".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(
                        GenericParamId {
                            owner: struct_id,
                            index: 0,
                        },
                        "T",
                    )],
                    fields: Vec::new(),
                },
            )]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(
                impl_id,
                HirImpl {
                    id: impl_id,
                    owner: HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: vec![GenericParamDecl::type_param(
                        GenericParamId {
                            owner: impl_id,
                            index: 0,
                        },
                        "T",
                    )],
                    receiver_pattern: vec![generic].into(),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: std::collections::HashMap::from([("get".to_string(), method)]),
                },
            )]),
            std::collections::HashMap::new(),
            HirNameTables {
                structs_by_name: std::collections::HashMap::from([("Box".to_string(), struct_id)]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(struct_id, "Box".to_string())]),
        );
        let type_context = test_type_context(&program);
        let builder = MirBuilder::with_instance_metadata(
            &program,
            &type_context,
            std::collections::HashMap::from([
                (i64_instance, Some(crate::types::ReceiverMode::Shared)),
                (bool_instance, Some(crate::types::ReceiverMode::Mut)),
            ]),
            std::collections::HashSet::new(),
        );
        assert_eq!(
            builder.method_instance_receiver_modes.get(&i64_instance),
            Some(&Some(crate::types::ReceiverMode::Shared))
        );
    }

    #[test]
    fn resolved_var_ignores_same_named_local_and_uses_canonical_target() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "make", Type::I64);
        builder.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let resolved = expr(
            HirExprKind::ResolvedVar(HirVarRef {
                name: "make".to_string(),
                target: HirVarTarget::Function(DefId::new(CrateId(0), LocalDefId(99))),
            }),
            Type::I64,
        );

        builder.lower_expr(
            &resolved,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );

        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(
                _,
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Callable(
                    crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Function(target))
                )))
            ) if *target == DefId::new(CrateId(0), LocalDefId(99))
        )));
    }

    #[test]
    fn resolved_local_moves_noncopy_in_borrow_context() {
        let program = program_with_struct(vec!["field"]);
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "self", struct_ty(0));
        builder.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&struct_ty(0)),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let resolved = expr(
            HirExprKind::ResolvedVar(HirVarRef {
                name: "self".to_string(),
                target: HirVarTarget::Local(crate::ids::HirLocalId(0)),
            }),
            struct_ty(0),
        );

        builder.lower_expr_with_context(
            &resolved,
            Place {
                local: Local(1),
                projection: vec![],
            },
            true,
        );

        assert!(matches!(
            &builder.blocks[0].statements[0].kind,
            StatementKind::Assign(
                dest,
                Rvalue::Use(Operand::Move(src))
            ) if dest.local == Local(1) && src.local == Local(0)
        ));
    }

    #[test]
    fn lower_place_resolves_local_var_target() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "v", Type::I64);
        let place_expr = expr(
            HirExprKind::ResolvedVar(HirVarRef {
                name: "v".to_string(),
                target: HirVarTarget::Local(crate::ids::HirLocalId(0)),
            }),
            Type::I64,
        );

        let place = builder
            .lower_place(&place_expr)
            .expect("expected resolved local place");

        assert_eq!(place.local, Local(0));
        assert!(place.projection.is_empty());
    }

    #[test]
    fn test_lower_place_field_projection() {
        let program = program_with_struct(vec!["a", "b"]);
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "x", struct_ty(0));
        let owner = program.test_struct_by_name("Foo").unwrap().0;
        let place_expr = expr(
            HirExprKind::FieldAccess(
                Box::new(expr(HirExprKind::Var("x".to_string()), struct_ty(0))),
                "b".to_string(),
                Some(crate::hir::HirFieldLocation {
                    owner,
                    field_id: crate::ids::FieldId(1),
                    name: "b".to_string(),
                }),
            ),
            Type::I64,
        );

        let place = builder
            .lower_place(&place_expr)
            .expect("expected field place");
        assert_eq!(place.local, Local(0));
        assert_eq!(
            place.projection,
            vec![Projection::Field {
                index: 1,
                identity: Some(crate::mir::MirFieldIdentity {
                    owner,
                    field_id: crate::ids::FieldId(1),
                }),
            }]
        );
    }

    #[test]
    fn lower_place_field_projection_requires_field_sidecar() {
        let program = program_with_struct(vec!["a", "b"]);
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "x", struct_ty(0));
        let place_expr = expr(
            HirExprKind::FieldAccess(
                Box::new(expr(HirExprKind::Var("x".to_string()), struct_ty(0))),
                "b".to_string(),
                None,
            ),
            Type::I64,
        );

        assert!(builder.lower_place(&place_expr).is_none());
    }

    #[test]
    fn test_lower_place_field_projection_uses_owned_struct_when_index_is_stale() {
        let mut program = program_with_struct(vec!["a", "b"]);
        program
            .indexes
            .structs_by_id
            .remove(&DefId::new(CrateId(0), LocalDefId(0)));
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "x", struct_ty(0));
        let owner = program.test_struct_by_name("Foo").unwrap().0;
        let place_expr = expr(
            HirExprKind::FieldAccess(
                Box::new(expr(HirExprKind::Var("x".to_string()), struct_ty(0))),
                "b".to_string(),
                Some(crate::hir::HirFieldLocation {
                    owner,
                    field_id: crate::ids::FieldId(1),
                    name: "b".to_string(),
                }),
            ),
            Type::I64,
        );

        let place = builder
            .lower_place(&place_expr)
            .expect("expected field place from owned struct");
        assert_eq!(place.local, Local(0));
        assert_eq!(
            place.projection,
            vec![Projection::Field {
                index: 1,
                identity: Some(crate::mir::MirFieldIdentity {
                    owner,
                    field_id: crate::ids::FieldId(1),
                }),
            }]
        );
    }

    #[test]
    fn test_lower_place_field_projection_ignores_mismatched_field_sidecar() {
        let program = program_with_struct(vec!["a", "b"]);
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "x", struct_ty(0));
        let owner = program.test_struct_by_name("Foo").unwrap().0;
        let place_expr = expr(
            HirExprKind::FieldAccess(
                Box::new(expr(HirExprKind::Var("x".to_string()), struct_ty(0))),
                "a".to_string(),
                Some(crate::hir::HirFieldLocation {
                    owner,
                    field_id: crate::ids::FieldId(1),
                    name: "b".to_string(),
                }),
            ),
            Type::I64,
        );

        assert!(builder.lower_place(&place_expr).is_none());
    }

    #[test]
    fn test_lower_struct_literal_orders_operands_by_field_id() {
        let program = program_with_struct(vec!["a", "b"]);
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&struct_ty(0)),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let owner = program.test_struct_by_name("Foo").unwrap().0;
        let literal = expr(
            HirExprKind::StructLiteral(
                "Foo".to_string(),
                None,
                vec![
                    HirStructLiteralField {
                        name: "b".to_string(),
                        value: expr(HirExprKind::IntLiteral(2), Type::I64),
                        field: Some(crate::hir::HirFieldLocation {
                            owner,
                            field_id: FieldId(1),
                            name: "b".to_string(),
                        }),
                    },
                    HirStructLiteralField {
                        name: "a".to_string(),
                        value: expr(HirExprKind::IntLiteral(1), Type::I64),
                        field: Some(crate::hir::HirFieldLocation {
                            owner,
                            field_id: FieldId(0),
                            name: "a".to_string(),
                        }),
                    },
                ],
            ),
            struct_ty(0),
        );

        builder.lower_expr(
            &literal,
            Place {
                local: Local(0),
                projection: vec![],
            },
        );

        let Some(StatementData {
            kind:
                StatementKind::Assign(
                    _,
                    Rvalue::Aggregate(
                        AggregateKind::Struct {
                            id: struct_id,
                            display_name,
                        },
                        operands,
                    ),
                ),
            ..
        }) = builder.blocks[0].statements.last()
        else {
            panic!("expected struct aggregate assignment");
        };

        assert_eq!(*struct_id, owner);
        assert_eq!(display_name, "Foo");
        assert!(matches!(
            operands[0],
            Operand::Copy(Place {
                local: Local(2),
                ..
            })
        ));
        assert!(matches!(
            operands[1],
            Operand::Copy(Place {
                local: Local(1),
                ..
            })
        ));
    }

    #[test]
    fn enum_variant_lowers_to_canonical_aggregate_with_payloads() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(30));
        let variant_id = crate::ids::VariantId(2);
        let mut enums = std::collections::HashMap::new();
        enums.insert(
            enum_id,
            crate::hir::HirEnum {
                id: enum_id,
                name: "Option".to_string(),
                generic_params: vec![],
                variants: vec![crate::hir::HirVariant {
                    id: variant_id,
                    name: "Some".to_string(),
                    fields: crate::hir::HirVariantFields::Positional(vec![Type::I64]),
                }],
            },
        );
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            enums,
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                enums_by_name: std::collections::HashMap::from([("Option".to_string(), enum_id)]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(enum_id, "Option".to_string())]),
        );
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::Enum {
                id: enum_id,
                args: vec![],
            }),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let expr = expr(
            HirExprKind::EnumVariant(
                "Option".to_string(),
                "Some".to_string(),
                vec![expr(HirExprKind::IntLiteral(9), Type::I64)],
                Some(crate::hir::HirVariantLocation {
                    owner: enum_id,
                    variant_id,
                    name: "Some".to_string(),
                }),
            ),
            Type::Enum {
                id: enum_id,
                args: vec![],
            },
        );

        builder.lower_expr(
            &expr,
            Place {
                local: Local(0),
                projection: vec![],
            },
        );

        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(
                _,
                Rvalue::Aggregate(
                    AggregateKind::EnumVariant {
                        enum_id: found_enum,
                        variant_id: found_variant,
                        enum_name,
                        variant_name,
                    },
                    operands,
                ),
            ) if *found_enum == enum_id
                && *found_variant == variant_id
                && enum_name == "Option"
                && variant_name == "Some"
                && operands.len() == 1
        )));

        assert_builder_agreement_clean(builder, Type::Unit);
    }

    #[test]
    fn struct_field_access_projection_carries_field_identity() {
        let struct_id = DefId::new(CrateId(0), LocalDefId(31));
        let field_id = crate::ids::FieldId(4);
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            std::collections::HashMap::from([(
                struct_id,
                crate::hir::HirStruct {
                    id: struct_id,
                    name: "Point".to_string(),
                    generic_params: vec![],
                    fields: vec![crate::hir::HirField {
                        id: field_id,
                        name: "x".to_string(),
                        ty: Type::I64,
                        public: true,
                    }],
                },
            )]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                structs_by_name: std::collections::HashMap::from([(
                    "Point".to_string(),
                    struct_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(struct_id, "Point".to_string())]),
        );
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(
            &program,
            &type_context,
            "p",
            Type::Struct {
                id: struct_id,
                args: vec![],
            },
        );
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let field = expr(
            HirExprKind::FieldAccess(
                Box::new(expr(
                    HirExprKind::Var("p".to_string()),
                    Type::Struct {
                        id: struct_id,
                        args: vec![],
                    },
                )),
                "x".to_string(),
                Some(crate::hir::HirFieldLocation {
                    owner: struct_id,
                    field_id,
                    name: "x".to_string(),
                }),
            ),
            Type::I64,
        );

        builder.lower_expr(
            &field,
            Place {
                local: Local(1),
                projection: vec![],
            },
        );

        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(
                _,
                Rvalue::Use(Operand::Copy(Place { projection, .. }))
            ) if projection.iter().any(|projection| matches!(
                projection,
                Projection::Field {
                    index: 0,
                    identity: Some(identity),
                } if identity.owner == struct_id && identity.field_id == field_id
            ))
        )));
    }

    #[test]
    fn non_place_struct_field_access_uses_canonical_id_and_field_identity() {
        let mut program = program_with_struct(vec!["a", "b"]);
        let struct_id = program.test_struct_by_name("Foo").unwrap().0;
        program.indexes.structs_by_id.remove(&struct_id);
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.scope_locals.push(Vec::new());
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let field = expr(
            HirExprKind::FieldAccess(
                Box::new(expr(
                    HirExprKind::StructLiteral(
                        "Foo".to_string(),
                        Some(struct_id),
                        vec![
                            HirStructLiteralField {
                                name: "a".to_string(),
                                value: expr(HirExprKind::IntLiteral(1), Type::I64),
                                field: Some(crate::hir::HirFieldLocation {
                                    owner: struct_id,
                                    field_id: FieldId(0),
                                    name: "a".to_string(),
                                }),
                            },
                            HirStructLiteralField {
                                name: "b".to_string(),
                                value: expr(HirExprKind::IntLiteral(2), Type::I64),
                                field: Some(crate::hir::HirFieldLocation {
                                    owner: struct_id,
                                    field_id: FieldId(1),
                                    name: "b".to_string(),
                                }),
                            },
                        ],
                    ),
                    struct_ty(0),
                )),
                "b".to_string(),
                Some(crate::hir::HirFieldLocation {
                    owner: struct_id,
                    field_id: FieldId(1),
                    name: "b".to_string(),
                }),
            ),
            Type::I64,
        );

        builder.lower_expr(
            &field,
            Place {
                local: Local(0),
                projection: vec![],
            },
        );

        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(
                _,
                Rvalue::Use(Operand::Copy(Place { projection, .. }))
            ) if projection.iter().any(|projection| matches!(
                projection,
                Projection::Field {
                    index: 1,
                    identity: Some(identity),
                } if identity.owner == struct_id && identity.field_id == FieldId(1)
            ))
        )));
    }

    #[test]
    #[should_panic(expected = "struct literal MIR lowering requires HIR field location")]
    fn struct_literal_missing_field_sidecar_panics() {
        let program = program_with_struct(vec!["a", "b"]);
        let struct_id = program.test_struct_by_name("Foo").unwrap().0;
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&struct_ty(0)),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let literal = expr(
            HirExprKind::StructLiteral(
                "Foo".to_string(),
                Some(struct_id),
                vec![HirStructLiteralField {
                    name: "a".to_string(),
                    value: expr(HirExprKind::IntLiteral(1), Type::I64),
                    field: None,
                }],
            ),
            struct_ty(0),
        );

        builder.lower_expr(
            &literal,
            Place {
                local: Local(0),
                projection: vec![],
            },
        );
    }

    #[test]
    fn test_lower_place_tuple_index_projection() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(
            &program,
            &type_context,
            "t",
            Type::Tuple(vec![Type::I64, Type::Bool]),
        );
        let place_expr = expr(
            HirExprKind::TupleIndex(
                Box::new(expr(
                    HirExprKind::Var("t".to_string()),
                    Type::Tuple(vec![Type::I64, Type::Bool]),
                )),
                1,
            ),
            Type::Bool,
        );

        let place = builder
            .lower_place(&place_expr)
            .expect("expected tuple place");
        assert_eq!(place.local, Local(0));
        assert_eq!(
            place.projection,
            vec![Projection::Field {
                index: 1,
                identity: None,
            }]
        );
    }

    #[test]
    fn test_lower_place_computed_pointer_deref_projection() {
        let pointee_ty = Type::U8;
        let pointer_ty = Type::Pointer(Box::new(pointee_ty.clone()));
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = builder_with_var(&program, &type_context, "ptr", pointer_ty.clone());
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        let pointer_offset = expr(
            HirExprKind::BinOp(
                crate::hir::BinOp::Add,
                Box::new(expr(
                    HirExprKind::Var("ptr".to_string()),
                    pointer_ty.clone(),
                )),
                Box::new(expr(HirExprKind::IntLiteral(1), Type::I64)),
            ),
            pointer_ty.clone(),
        );
        let place_expr = expr(HirExprKind::Deref(Box::new(pointer_offset)), pointee_ty);

        let place = builder
            .lower_place(&place_expr)
            .expect("expected computed pointer deref place");

        assert_eq!(place.projection, vec![Projection::Deref]);
        assert_eq!(
            builder.locals[place.local.0].ty,
            builder.type_id_for(&pointer_ty)
        );
    }

    #[test]
    fn mir_index_mut_lowers_through_instance_reference() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let array_ty = Type::Array(Box::new(Type::I64), 1);
        let index_mut_ref_ty = Type::Reference {
            mutable: true,
            inner: Box::new(Type::I64),
        };
        let instance_id = InstanceId(7);
        let mut builder = builder_with_var(&program, &type_context, "arr", array_ty.clone());
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::Unit),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::Temporary,
        });
        builder.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));

        let callee = expr(
            HirExprKind::ResolvedVar(HirVarRef {
                name: "index_mut".to_string(),
                target: HirVarTarget::Instance(instance_id),
            }),
            Type::function(vec![array_ty.clone(), Type::I64], index_mut_ref_ty.clone()),
        );
        let call = expr(
            HirExprKind::Call(
                Box::new(callee),
                vec![
                    expr(HirExprKind::Var("arr".to_string()), array_ty.clone()),
                    expr(HirExprKind::IntLiteral(0), Type::I64),
                ],
                Some(crate::hir::HirCallTarget::Instance(instance_id)),
            ),
            index_mut_ref_ty,
        );
        let assignment = expr(
            HirExprKind::Assign(
                Box::new(expr(HirExprKind::Deref(Box::new(call)), Type::I64)),
                Box::new(expr(HirExprKind::IntLiteral(42), Type::I64)),
            ),
            Type::Unit,
        );

        builder.lower_expr(
            &assignment,
            Place {
                local: Local(1),
                projection: Vec::new(),
            },
        );

        let (returned_ref_local, returned_ref_is_stored, called_instance) = builder
            .blocks
            .iter()
            .filter_map(|block| block.terminator.as_ref())
            .find_map(|terminator| match terminator {
                Terminator::Call {
                    func:
                        Operand::Constant(Constant::Callable(MirCallable::Resolved(
                            MirCallableKey::Instance(instance),
                        ))),
                    destination,
                    ..
                } => Some((
                    destination.local,
                    destination.projection.is_empty(),
                    *instance,
                )),
                _ => None,
            })
            .expect("IndexMut should lower to an instance call");
        assert_eq!(called_instance, instance_id);
        assert!(returned_ref_is_stored);
        assert_eq!(
            builder.locals[returned_ref_local.0].ty,
            builder.type_id_for(&Type::Reference {
                mutable: true,
                inner: Box::new(Type::I64),
            })
        );

        assert!(builder
            .blocks
            .iter()
            .flat_map(|block| block.statements.iter())
            .any(|statement| matches!(
                &statement.kind,
                StatementKind::Assign(
                    Place { local, projection },
                    Rvalue::Use(_),
                ) if *local == returned_ref_local
                    && projection.as_slice() == [Projection::Deref]
            )));
        assert!(!builder
            .blocks
            .iter()
            .flat_map(|block| block.statements.iter())
            .any(|statement| matches!(
                &statement.kind,
                StatementKind::Assign(
                    Place { projection, .. },
                    Rvalue::Use(_),
                ) if projection.iter().any(|projection| matches!(projection, Projection::Index(_)))
            )));
        assert!(!builder.locals.iter().any(|local| matches!(
            builder.type_context.borrow().type_for(local.ty),
            Type::Projection { .. }
        )));
    }

    #[test]
    fn build_uses_canonical_function_indexes_once_per_def_id() {
        let id = DefId::new(CrateId(0), LocalDefId(0));
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::from([(id, empty_function(id, "main"))]),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            crate::hir::HirNameTables {
                functions_by_name: std::collections::HashMap::from([
                    ("alias_main".to_string(), id),
                    ("main".to_string(), id),
                ]),
                ..crate::hir::HirNameTables::default()
            },
            &std::collections::HashMap::from([(id, "main".to_string())]),
        );

        let mir = MirBuilder::build(&program);
        let mir_id = crate::mir::MirFunctionId::Function(id);

        assert_eq!(mir.functions.len(), 1);
        assert!(mir.functions.contains_key(&mir_id));
        assert_eq!(mir.functions[&mir_id].name, "main");
    }

    #[test]
    fn test_lower_expr_cast_to_rvalue_cast() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::Pointer(Box::new(Type::I64))),
            mutability: Mutability::Not,
            name: Some("tmp".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.var_map.insert("tmp".to_string(), Local(0));
        let dest = Place {
            local: Local(0),
            projection: vec![],
        };
        let cast_expr = expr(
            HirExprKind::Cast(
                Box::new(expr(
                    HirExprKind::Var("tmp".to_string()),
                    Type::Pointer(Box::new(Type::I64)),
                )),
                Type::Pointer(Box::new(Type::U8)),
            ),
            Type::Pointer(Box::new(Type::U8)),
        );

        let target_id = builder.type_id_for(&Type::Pointer(Box::new(Type::U8)));
        builder.lower_expr_with_context(&cast_expr, dest, false);
        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(_, Rvalue::Cast(Operand::Copy(_), ty))
                if *ty == target_id
        )));
    }

    #[test]
    fn test_lower_expr_non_pointer_cast_remains_copy() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("tmp".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.var_map.insert("tmp".to_string(), Local(0));
        let dest = Place {
            local: Local(0),
            projection: vec![],
        };
        let cast_expr = expr(
            HirExprKind::Cast(
                Box::new(expr(HirExprKind::Var("tmp".to_string()), Type::I64)),
                Type::I64,
            ),
            Type::I64,
        );

        builder.lower_expr_with_context(&cast_expr, dest, false);
        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(_, Rvalue::Use(Operand::Copy(_)))
        )));
    }

    #[test]
    fn test_lower_expr_field_access_moves_non_copy_value() {
        let mut structs = std::collections::HashMap::new();
        let wrapper_id = DefId::new(CrateId(0), LocalDefId(0));
        structs.insert(
            wrapper_id,
            HirStruct {
                id: wrapper_id,
                name: "Wrapper".to_string(),
                generic_params: vec![],
                fields: vec![HirField {
                    id: FieldId(0),
                    name: "inner".to_string(),
                    ty: struct_ty(1),
                    public: true,
                }],
            },
        );
        let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            std::collections::HashMap::new(),
            structs,
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            HirNameTables {
                structs_by_name: std::collections::HashMap::from([(
                    "Wrapper".to_string(),
                    wrapper_id,
                )]),
                ..HirNameTables::default()
            },
            &std::collections::HashMap::from([(wrapper_id, "Wrapper".to_string())]),
        );
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&struct_ty(0)),
            mutability: Mutability::Not,
            name: Some("w".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.var_map.insert("w".to_string(), Local(0));
        let dest = Place {
            local: Local(0),
            projection: vec![],
        };
        let field_expr = expr(
            HirExprKind::FieldAccess(
                Box::new(expr(HirExprKind::Var("w".to_string()), struct_ty(0))),
                "inner".to_string(),
                Some(crate::hir::HirFieldLocation {
                    owner: DefId::new(CrateId(0), LocalDefId(0)),
                    field_id: FieldId(0),
                    name: "inner".to_string(),
                }),
            ),
            struct_ty(1),
        );

        builder.lower_expr_with_context(&field_expr, dest, false);
        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(_, Rvalue::Use(Operand::Move(_)))
        )));
    }

    #[test]
    fn test_lower_expr_field_access_from_non_place_base_avoids_unit_fallback() {
        let program = program_with_struct(vec!["a", "b"]);
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.scope_locals.push(Vec::new());
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("dest_value".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let dest = Place {
            local: Local(0),
            projection: vec![],
        };
        let owner = program.test_struct_by_name("Foo").unwrap().0;
        let field_expr = expr(
            HirExprKind::FieldAccess(
                Box::new(expr(
                    HirExprKind::StructLiteral(
                        "Foo".to_string(),
                        None,
                        vec![
                            HirStructLiteralField {
                                name: "a".to_string(),
                                value: expr(HirExprKind::IntLiteral(1), Type::I64),
                                field: Some(crate::hir::HirFieldLocation {
                                    owner,
                                    field_id: FieldId(0),
                                    name: "a".to_string(),
                                }),
                            },
                            HirStructLiteralField {
                                name: "b".to_string(),
                                value: expr(HirExprKind::IntLiteral(2), Type::I64),
                                field: Some(crate::hir::HirFieldLocation {
                                    owner,
                                    field_id: FieldId(1),
                                    name: "b".to_string(),
                                }),
                            },
                        ],
                    ),
                    struct_ty(0),
                )),
                "b".to_string(),
                Some(crate::hir::HirFieldLocation {
                    owner,
                    field_id: FieldId(1),
                    name: "b".to_string(),
                }),
            ),
            Type::I64,
        );

        builder.lower_expr_with_context(&field_expr, dest, false);
        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(_, Rvalue::Use(Operand::Copy(place)))
                if place.projection.contains(&Projection::Field {
                    index: 1,
                    identity: Some(crate::mir::MirFieldIdentity {
                        owner,
                        field_id: FieldId(1),
                    }),
                })
        )));
        assert!(!builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(
                _,
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Unit))
            )
        )));
        assert_eq!(
            builder.scope_locals.last().map(|locals| locals.len()),
            Some(1)
        );
    }

    #[test]
    fn test_lower_expr_tuple_index_from_non_place_base_avoids_unit_fallback() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.scope_locals.push(Vec::new());
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::Bool),
            mutability: Mutability::Not,
            name: Some("dest_value".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let dest = Place {
            local: Local(0),
            projection: vec![],
        };
        let tuple_expr = expr(
            HirExprKind::TupleIndex(
                Box::new(expr(
                    HirExprKind::TupleLiteral(vec![
                        expr(HirExprKind::IntLiteral(1), Type::I64),
                        expr(HirExprKind::BoolLiteral(true), Type::Bool),
                    ]),
                    Type::Tuple(vec![Type::I64, Type::Bool]),
                )),
                1,
            ),
            Type::Bool,
        );

        builder.lower_expr_with_context(&tuple_expr, dest, false);
        assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(_, Rvalue::Use(Operand::Copy(place)))
                if place.projection.contains(&Projection::Field {
                    index: 1,
                    identity: None,
                })
        )));
        assert!(!builder.blocks[0].statements.iter().any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(
                _,
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Unit))
            )
        )));
        assert_eq!(
            builder.scope_locals.last().map(|locals| locals.len()),
            Some(1)
        );
    }

    #[test]
    fn test_lower_expr_ref_non_place_base_registers_temp_for_cleanup() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.scope_locals.push(Vec::new());
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            }),
            mutability: Mutability::Not,
            name: Some("dest".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        let dest = Place {
            local: Local(0),
            projection: vec![],
        };
        let expr = expr(
            HirExprKind::Ref(false, Box::new(expr(HirExprKind::IntLiteral(1), Type::I64))),
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );

        builder.lower_expr(&expr, dest);
        assert_eq!(builder.scope_locals.last().map(|s| s.len()), Some(1));
    }

    #[test]
    fn reference_returning_call_destination_has_no_blanket_external_origin() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            }),
            mutability: Mutability::Mut,
            name: Some("destination".to_string()),
            span: None,
            source: LocalSource::Temporary,
        });

        builder.set_terminator(
            BasicBlockId(0),
            Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    MirCallableKey::Function(DefId::new(CrateId(0), LocalDefId(901))),
                ))),
                args: Vec::new(),
                destination: Place {
                    local: Local(0),
                    projection: Vec::new(),
                },
                target: BasicBlockId(1),
                span: Some(crate::lexer::Span::test()),
            },
        );

        assert!(!builder
            .ownership
            .reference_origins
            .contains(&(Local(0), ReferenceOrigin::UnknownExternal)));
    }

    #[test]
    fn test_build_function_exposes_closure_captures() {
        let program = empty_program();
        let func = HirFunction {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            name: "main".to_string(),
            generic_params: vec![],
            generic_bounds: std::collections::HashMap::new().into(),
            params: vec![],
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: vec![],
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };

        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        let mir_func = builder.build_function(
            MirFunctionId::Function(func.id),
            &func.name,
            &func,
            false,
            Some(func.id),
            None,
        );
        assert!(mir_func.closure_captures.is_empty());
        assert_eq!(
            mir_func.local_decls[0].origin(),
            crate::mir::MirOrigin::synthetic(crate::mir::MirSyntheticOrigin::Unknown, None)
        );
    }

    #[test]
    fn build_function_uses_session_source_map_for_parameter_and_return_origins() {
        let program = empty_program();
        let owner = DefId::new(CrateId(0), LocalDefId(300));
        let parameter_local = crate::ids::HirLocalId(7);
        let parameter_span = Span::new("/virtual/main.rk".into(), 10, 13);
        let operation_span = Span::new("/virtual/main.rk".into(), 0, 20);
        let function = HirFunction {
            id: owner,
            name: "main".to_string(),
            generic_params: vec![],
            generic_bounds: std::collections::HashMap::new().into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: parameter_local,
                ty: Type::I64,
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: vec![],
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let mut source_map = crate::source_map::SemanticSourceMap::default();
        source_map.insert_local(owner, parameter_local, parameter_span.clone());
        source_map.insert_definition(owner, operation_span.clone());

        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context).with_source_map(&source_map);
        let mir_func = builder.build_function(
            MirFunctionId::Function(owner),
            &function.name,
            &function,
            false,
            Some(owner),
            Some(operation_span.clone()),
        );

        assert_eq!(mir_func.local_decls[0].span, Some(operation_span.clone()));
        assert_eq!(mir_func.local_decls[1].span, Some(parameter_span.clone()));
        assert_eq!(
            mir_func.local_decls[0].origin(),
            crate::mir::MirOrigin::Source(operation_span)
        );
        assert_eq!(
            mir_func.local_decls[1].origin(),
            crate::mir::MirOrigin::Source(parameter_span)
        );
    }

    #[test]
    fn build_function_terminates_blocks_after_if_without_else() {
        let program = empty_program();
        let func = HirFunction {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            name: "if_without_else".to_string(),
            generic_params: vec![],
            generic_bounds: std::collections::HashMap::new().into(),
            params: vec![],
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: vec![HirStmt::Expr(expr(
                    HirExprKind::If {
                        condition: Box::new(expr(HirExprKind::BoolLiteral(true), Type::Bool)),
                        then_branch: HirBlock {
                            stmts: vec![HirStmt::Expr(expr(
                                HirExprKind::BoolLiteral(false),
                                Type::Bool,
                            ))],
                            ty: Type::Unit,
                        },
                        else_branch: None,
                    },
                    Type::Unit,
                ))],
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };

        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        let mir_func = builder.build_function(
            MirFunctionId::Function(func.id),
            &func.name,
            &func,
            false,
            Some(func.id),
            None,
        );

        assert!(mir_func
            .basic_blocks
            .iter()
            .enumerate()
            .all(|(_, block)| block.terminator.is_some()));
    }

    #[test]
    fn test_build_function_with_lambda_still_builds() {
        let program = empty_program();
        let func = HirFunction {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            name: "main".to_string(),
            generic_params: vec![],
            generic_bounds: std::collections::HashMap::new().into(),
            params: vec![],
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: vec![HirStmt::Expr(expr(
                    HirExprKind::Lambda {
                        params: vec![],
                        body: HirBlock {
                            stmts: vec![HirStmt::Expr(expr(HirExprKind::IntLiteral(1), Type::I64))],
                            ty: Type::I64,
                        },
                        captures: vec![],
                    },
                    Type::function(vec![], Type::I64),
                ))],
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };

        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        let mir_func = builder.build_function(
            MirFunctionId::Function(func.id),
            &func.name,
            &func,
            false,
            Some(func.id),
            None,
        );
        assert!(mir_func.closure_captures.is_empty());
    }

    #[test]
    fn test_lower_expr_lambda_emits_closure_rvalue() {
        let program = empty_program();
        let type_context = test_type_context(&program);
        let mut builder = MirBuilder::new(&program, &type_context);
        builder.blocks.push(BasicBlock {
            statements: vec![],
            terminator: None,
        });
        builder.current_block = Some(BasicBlockId(0));
        builder.current_function_id = Some(MirFunctionId::Function(DefId::new(
            CrateId(0),
            LocalDefId(0),
        )));
        builder.locals.push(LocalDecl {
            ty: builder.type_id_for(&Type::I64),
            mutability: Mutability::Not,
            name: Some("outer".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        });
        builder.var_map.insert("outer".to_string(), Local(0));

        let lambda = HirExpr {
            kind: HirExprKind::Lambda {
                params: vec![],
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(expr(
                        HirExprKind::Var("outer".to_string()),
                        Type::I64,
                    ))],
                    ty: Type::I64,
                },
                captures: vec![crate::hir::HirClosureCapture {
                    name: "outer".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    kind: crate::hir::HirClosureCaptureKind::SharedBorrow,
                    mutable: false,
                    ty: Type::I64,
                }],
            },
            ty: Type::function(vec![], Type::I64),
            span: span(),
        };

        let dest = Place {
            local: Local(0),
            projection: vec![],
        };
        builder.lower_expr_with_context(&lambda, dest, false);
        assert!(matches!(
            &builder.blocks[0].statements.last().unwrap().kind,
            StatementKind::Assign(_, Rvalue::Closure(_))
        ));
        let nested = builder.take_nested_functions();
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].local_decls[0].span, Some(lambda.span.clone()));
        assert_eq!(nested[0].local_decls[1].span, Some(lambda.span.clone()));
        assert_eq!(nested[0].closure_captures[0].span, Some(lambda.span));
    }
}
