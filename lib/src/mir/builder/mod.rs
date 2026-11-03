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
            let signature = super::MirCallableSignature::from_type_ids(
                &params,
                function.ret_type,
                super::MirPassMode::Direct,
            );

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
            Operand::Constant(Constant::Char(_)) => type_context.id_for_type(&Type::Char),
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
                            .map(|param| type_context.intern_canonical_type(&param.ty))
                            .collect(),
                        type_context.intern_canonical_type(&function.ret_type),
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
            let signature = super::MirCallableSignature::from_instance_type_ids(
                &params,
                ret,
                is_method,
                type_context,
            );
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
                                    (
                                        field.name.clone(),
                                        type_context.intern_canonical_type(&field.ty),
                                    )
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
                                .map(|ty| type_context.intern_canonical_type(ty))
                                .collect(),
                        )
                    }
                    HirVariantFields::Named(fields) => super::MirVariantLayoutFields::Named(
                        fields
                            .iter()
                            .map(|field| {
                                (
                                    field.name.clone(),
                                    type_context.intern_canonical_type(&field.ty),
                                )
                            })
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
        // Accepted HIR is canonical, including constructor-valued nominal args.
        self.type_context.borrow_mut().intern_canonical_type(ty)
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
mod tests;
