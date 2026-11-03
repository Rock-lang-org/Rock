use std::collections::HashSet;

use crate::ids::{DefId, TypeId};
use crate::mir::{
    AggregateKind, Constant, MirAssertKind, MirBackendContractError, MirCallable, MirCallableDecl,
    MirCallableKey, MirFunction, MirNominalLayout, MirParamAbi, MirProgram, Operand, Place,
    Projection, Rvalue, StatementKind, Terminator,
};
use crate::type_context::{Ty, TypeView};
use crate::types::Type;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MirAgreementReport {
    pub non_unit_placeholder_units: usize,
    pub malformed_runtime_requirements: usize,
    pub missing_runtime_requirements: usize,
    pub stale_runtime_requirements: usize,
    pub invalid_type_ids: usize,
    pub invalid_constant_operands: usize,
    pub missing_callable_metadata: usize,
    pub invalid_layout_ids: usize,
    pub missing_projection_facts: usize,
    pub stale_projection_facts: usize,
    pub invalid_backend_contract: usize,
    pub backend_contract_errors: Vec<MirBackendContractError>,
}

impl MirAgreementReport {
    pub fn is_clean(&self) -> bool {
        self.non_unit_placeholder_units == 0
            && self.malformed_runtime_requirements == 0
            && self.missing_runtime_requirements == 0
            && self.stale_runtime_requirements == 0
            && self.invalid_type_ids == 0
            && self.invalid_constant_operands == 0
            && self.missing_callable_metadata == 0
            && self.invalid_layout_ids == 0
            && self.missing_projection_facts == 0
            && self.stale_projection_facts == 0
            && self.invalid_backend_contract == 0
    }
}

pub fn check_mir_runtime_agreement(program: &MirProgram) -> MirAgreementReport {
    let mut report = MirAgreementReport::default();
    let type_view = TypeView::new(&program.type_context);
    let callable_contract = CallableContract::new(program);
    let backend_contract = BackendContract::new(program);
    let mut seen_nominal_layout_ids = HashSet::new();

    validate_backend_contract_type_ids(program, &mut report);
    validate_backend_contract_projection_traits(program, &backend_contract, &mut report);
    validate_mir_backend_contract(program, &mut report);
    validate_runtime_requirements(program, &mut report);
    validate_backend_contract_nominal_layout_ids(
        program,
        &backend_contract,
        &mut seen_nominal_layout_ids,
        &mut report,
    );

    for (_, function) in program.functions() {
        validate_function_type_ids(program, function, &mut report);
        validate_function_nominal_layout_ids(
            program,
            &backend_contract,
            function,
            &mut seen_nominal_layout_ids,
            &mut report,
        );

        for block in &function.basic_blocks {
            for statement in &block.statements {
                match &statement.kind {
                    StatementKind::Assign(place, rvalue) => {
                        validate_place_layout_ids(&backend_contract, place, &mut report);
                        validate_rvalue_type_ids(
                            program,
                            &callable_contract,
                            function,
                            place,
                            rvalue,
                            &mut report,
                        );
                        validate_rvalue_layout_ids(&backend_contract, rvalue, &mut report);
                        let Some(local_decl) = function.local_decls.get(place.local.0) else {
                            continue;
                        };
                        if program.type_context.contains_type_id(local_decl.ty)
                            && place.projection.is_empty()
                            && !matches!(type_view.ty(local_decl.ty), Ty::Unit)
                            && matches!(rvalue, Rvalue::Use(Operand::Constant(Constant::Unit)))
                        {
                            report.non_unit_placeholder_units += 1;
                        }
                    }
                    StatementKind::Assert(assertion) => {
                        for operand in &assertion.operands {
                            validate_runtime_operand(program, operand, false, &mut report);
                        }
                        match assertion.kind {
                            MirAssertKind::BoundsCheck if assertion.operands.len() != 2 => {
                                report.malformed_runtime_requirements += 1;
                            }
                            MirAssertKind::BoundsCheck => {}
                        }
                    }
                    _ => {}
                }
            }

            match &block.terminator {
                Some(Terminator::Call { func, args, .. }) => {
                    validate_runtime_operand(program, func, true, &mut report);
                    if callable_contract.is_missing(func) {
                        report.missing_callable_metadata += 1;
                    }
                    validate_call_argument_metadata(
                        program,
                        &callable_contract,
                        function,
                        func,
                        args,
                        &mut report,
                    );
                    validate_call_constant_operands(
                        program,
                        &callable_contract,
                        function,
                        func,
                        args,
                        &mut report,
                    );
                    if is_unit_callable_operand(type_view, function, func) {
                        report.non_unit_placeholder_units += 1;
                    }
                }
                Some(Terminator::SwitchInt { discr, .. })
                | Some(Terminator::SwitchIntWithOrigin { discr, .. }) => {
                    validate_runtime_operand(program, discr, false, &mut report);
                }
                Some(Terminator::Drop { place, .. })
                | Some(Terminator::DropWithOrigin { place, .. }) => {
                    validate_place_layout_ids(&backend_contract, place, &mut report);
                }
                _ => {}
            }
        }
    }

    report
}

fn validate_runtime_requirements(program: &MirProgram, report: &mut MirAgreementReport) {
    let observed = crate::mir::backend_contract::runtime_requirements_for_functions(
        program.functions.values(),
    );

    report.missing_runtime_requirements = observed
        .difference(&program.backend_contract.runtime_requirements)
        .count();
    report.stale_runtime_requirements = program
        .backend_contract
        .runtime_requirements
        .difference(&observed)
        .count();
}

fn validate_mir_backend_contract(program: &MirProgram, report: &mut MirAgreementReport) {
    let errors = crate::mir::validate_backend_contract_against_mir(
        &program.backend_contract,
        program.functions.values(),
    );
    report.invalid_backend_contract += errors.len();
    report.backend_contract_errors.extend(errors);
}

fn validate_backend_contract_type_ids(program: &MirProgram, report: &mut MirAgreementReport) {
    for callable in program.backend_contract.callables.values() {
        for param in &callable.signature.params {
            validate_type_id(program, param.semantic_ty, report);
        }
        validate_type_id(program, callable.signature.ret.semantic_ty, report);
        validate_type_id(program, callable.signature.ret.abi_ty, report);
    }

    for layout in program.backend_contract.nominal_layouts.values() {
        match layout {
            MirNominalLayout::Struct {
                fields,
                generic_params,
                ..
            } => {
                for (_, field_ty) in fields {
                    validate_layout_template_type_id(program, *field_ty, generic_params, report);
                }
            }
            MirNominalLayout::Enum {
                variants,
                generic_params,
                ..
            } => {
                for variant in variants {
                    match &variant.fields {
                        crate::mir::MirVariantLayoutFields::Unit => {}
                        crate::mir::MirVariantLayoutFields::Positional(fields) => {
                            for field_ty in fields {
                                validate_layout_template_type_id(
                                    program,
                                    *field_ty,
                                    generic_params,
                                    report,
                                );
                            }
                        }
                        crate::mir::MirVariantLayoutFields::Named(fields) => {
                            for (_, field_ty) in fields {
                                validate_layout_template_type_id(
                                    program,
                                    *field_ty,
                                    generic_params,
                                    report,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    for (key, output) in &program.backend_contract.projection_outputs {
        validate_type_id(program, key.base, report);
        for trait_arg in &key.trait_args {
            validate_type_id(program, *trait_arg, report);
        }
        validate_type_id(program, *output, report);
    }

    for ty in program.backend_contract.drop_glue.keys() {
        validate_type_id(program, *ty, report);
    }
}

fn validate_layout_template_type_id(
    program: &MirProgram,
    id: TypeId,
    generic_params: &[crate::types::GenericParamId],
    report: &mut MirAgreementReport,
) {
    let Some(ty) = program.type_context.try_ty(id) else {
        report.invalid_type_ids += 1;
        return;
    };

    match ty {
        Ty::Slice(inner) | Ty::Reference { inner, .. } | Ty::Pointer(inner) => {
            validate_layout_template_type_id(program, *inner, generic_params, report);
        }
        Ty::Array { inner, .. } => {
            validate_layout_template_type_id(program, *inner, generic_params, report);
        }
        Ty::Tuple(elems) => {
            for elem in elems {
                validate_layout_template_type_id(program, *elem, generic_params, report);
            }
        }
        Ty::Function {
            params,
            ret,
            captures,
            ..
        } => {
            for param in params {
                validate_layout_template_type_id(program, *param, generic_params, report);
            }
            validate_layout_template_type_id(program, *ret, generic_params, report);
            for capture in captures {
                validate_layout_template_type_id(program, capture.ty, generic_params, report);
            }
        }
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => {
            for arg in args {
                if program.type_context.kind(*arg) == &crate::type_services::kind::Kind::Type {
                    validate_layout_template_type_id(program, *arg, generic_params, report);
                }
            }
        }
        Ty::Projection { ty, trait_args, .. } => {
            validate_layout_template_type_id(program, *ty, generic_params, report);
            for arg in trait_args {
                validate_layout_template_type_id(program, *arg, generic_params, report);
            }
        }
        Ty::Apply { constructor, args } => {
            validate_layout_template_type_id(program, *constructor, generic_params, report);
            for arg in args {
                validate_layout_template_type_id(program, *arg, generic_params, report);
            }
        }
        Ty::Generic(param) if generic_params.contains(param) => {}
        Ty::TypeVar(_)
        | Ty::Generic(_)
        | Ty::Constructor { .. }
        | Ty::Lambda { .. }
        | Ty::BoundVar { .. }
        | Ty::Error => report.invalid_type_ids += 1,
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
        | Ty::Str
        | Ty::Char
        | Ty::Unit
        | Ty::Never => {}
    }
}

fn validate_backend_contract_nominal_layout_ids(
    program: &MirProgram,
    backend_contract: &BackendContract,
    seen_nominals: &mut HashSet<DefId>,
    report: &mut MirAgreementReport,
) {
    for callable in program.backend_contract.callables.values() {
        for param in &callable.signature.params {
            validate_type_nominal_layout_id(
                program,
                backend_contract,
                param.semantic_ty,
                seen_nominals,
                report,
            );
        }
        validate_type_nominal_layout_id(
            program,
            backend_contract,
            callable.signature.ret.semantic_ty,
            seen_nominals,
            report,
        );
        validate_type_nominal_layout_id(
            program,
            backend_contract,
            callable.signature.ret.abi_ty,
            seen_nominals,
            report,
        );
    }

    for (key, layout) in &program.backend_contract.nominal_layouts {
        match layout {
            MirNominalLayout::Struct { id, fields, .. } => {
                if key != id {
                    report.invalid_layout_ids += 1;
                }
                for (_, field_ty) in fields {
                    validate_type_nominal_layout_id(
                        program,
                        backend_contract,
                        *field_ty,
                        seen_nominals,
                        report,
                    );
                }
            }
            MirNominalLayout::Enum { id, variants, .. } => {
                if key != id {
                    report.invalid_layout_ids += 1;
                }
                for variant in variants {
                    match &variant.fields {
                        crate::mir::MirVariantLayoutFields::Unit => {}
                        crate::mir::MirVariantLayoutFields::Positional(fields) => {
                            for field_ty in fields {
                                validate_type_nominal_layout_id(
                                    program,
                                    backend_contract,
                                    *field_ty,
                                    seen_nominals,
                                    report,
                                );
                            }
                        }
                        crate::mir::MirVariantLayoutFields::Named(fields) => {
                            for (_, field_ty) in fields {
                                validate_type_nominal_layout_id(
                                    program,
                                    backend_contract,
                                    *field_ty,
                                    seen_nominals,
                                    report,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    for (key, output) in &program.backend_contract.projection_outputs {
        validate_type_nominal_layout_id(program, backend_contract, key.base, seen_nominals, report);
        for trait_arg in &key.trait_args {
            validate_type_nominal_layout_id(
                program,
                backend_contract,
                *trait_arg,
                seen_nominals,
                report,
            );
        }
        validate_type_nominal_layout_id(program, backend_contract, *output, seen_nominals, report);
    }

    for ty in program.backend_contract.drop_glue.keys() {
        validate_type_nominal_layout_id(program, backend_contract, *ty, seen_nominals, report);
    }
}

fn validate_backend_contract_projection_traits(
    program: &MirProgram,
    backend_contract: &BackendContract,
    report: &mut MirAgreementReport,
) {
    for key in program.backend_contract.projection_outputs.keys() {
        if !backend_contract.projection_trait_known(key.trait_id) {
            report.stale_projection_facts += 1;
        }
    }
}

fn validate_function_type_ids(
    program: &MirProgram,
    function: &MirFunction,
    report: &mut MirAgreementReport,
) {
    validate_type_id(program, function.ret_type, report);
    for local in &function.local_decls {
        validate_type_id(program, local.ty, report);
    }
}

fn validate_function_nominal_layout_ids(
    program: &MirProgram,
    backend_contract: &BackendContract,
    function: &MirFunction,
    seen_nominals: &mut HashSet<DefId>,
    report: &mut MirAgreementReport,
) {
    validate_type_nominal_layout_id(
        program,
        backend_contract,
        function.ret_type,
        seen_nominals,
        report,
    );
    for local in &function.local_decls {
        validate_type_nominal_layout_id(program, backend_contract, local.ty, seen_nominals, report);
    }
}

fn validate_type_nominal_layout_id(
    program: &MirProgram,
    backend_contract: &BackendContract,
    id: TypeId,
    seen_nominals: &mut HashSet<DefId>,
    report: &mut MirAgreementReport,
) {
    if !program.type_context.type_id_tree_is_valid(id) {
        return;
    }
    let raw = program.type_context.type_for(id);
    let normalized = program
        .backend_contract
        .normalize_type(&program.type_context, &raw);
    validate_type_nominal_layout(
        &program.type_context,
        backend_contract,
        &normalized,
        seen_nominals,
        report,
    );
}

fn validate_type_nominal_layout(
    type_context: &crate::type_context::TypeContext,
    backend_contract: &BackendContract,
    ty: &Type,
    seen_nominals: &mut HashSet<DefId>,
    report: &mut MirAgreementReport,
) {
    match ty {
        Type::Struct { id, args } => {
            if seen_nominals.insert(*id) && !backend_contract.structs.contains(id) {
                report.invalid_layout_ids += 1;
            }
            for arg in args {
                if type_context.id_for_type(arg).map_or(true, |id| {
                    type_context.kind(id) == &crate::type_services::kind::Kind::Type
                }) {
                    validate_type_nominal_layout(
                        type_context,
                        backend_contract,
                        arg,
                        seen_nominals,
                        report,
                    );
                }
            }
        }
        Type::Enum { id, args } => {
            if seen_nominals.insert(*id) && !backend_contract.enums.contains(id) {
                report.invalid_layout_ids += 1;
            }
            for arg in args {
                if type_context.id_for_type(arg).map_or(true, |id| {
                    type_context.kind(id) == &crate::type_services::kind::Kind::Type
                }) {
                    validate_type_nominal_layout(
                        type_context,
                        backend_contract,
                        arg,
                        seen_nominals,
                        report,
                    );
                }
            }
        }
        Type::Reference { inner, .. } | Type::Pointer(inner) | Type::Slice(inner) => {
            validate_type_nominal_layout(
                type_context,
                backend_contract,
                inner,
                seen_nominals,
                report,
            );
        }
        Type::Array(inner, _) => {
            validate_type_nominal_layout(
                type_context,
                backend_contract,
                inner,
                seen_nominals,
                report,
            );
        }
        Type::Tuple(elems) => {
            for elem in elems {
                validate_type_nominal_layout(
                    type_context,
                    backend_contract,
                    elem,
                    seen_nominals,
                    report,
                );
            }
        }
        Type::Function {
            params,
            ret,
            captures,
            ..
        } => {
            for param in params {
                validate_type_nominal_layout(
                    type_context,
                    backend_contract,
                    param,
                    seen_nominals,
                    report,
                );
            }
            validate_type_nominal_layout(
                type_context,
                backend_contract,
                ret,
                seen_nominals,
                report,
            );
            for capture in captures {
                validate_type_nominal_layout(
                    type_context,
                    backend_contract,
                    &capture.ty,
                    seen_nominals,
                    report,
                );
            }
        }
        Type::Projection { ty, trait_args, .. } => {
            validate_type_nominal_layout(type_context, backend_contract, ty, seen_nominals, report);
            for arg in trait_args {
                validate_type_nominal_layout(
                    type_context,
                    backend_contract,
                    arg,
                    seen_nominals,
                    report,
                );
            }
        }
        Type::Apply { constructor, args } => {
            validate_type_nominal_layout(
                type_context,
                backend_contract,
                constructor,
                seen_nominals,
                report,
            );
            for arg in args {
                validate_type_nominal_layout(
                    type_context,
                    backend_contract,
                    arg,
                    seen_nominals,
                    report,
                );
            }
        }
        Type::Lambda { body, .. } => {
            validate_type_nominal_layout(
                type_context,
                backend_contract,
                body,
                seen_nominals,
                report,
            );
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
        | Type::Str
        | Type::Char
        | Type::Unit
        | Type::Never => {}
        Type::TypeVar(_)
        | Type::Generic(_)
        | Type::Constructor { .. }
        | Type::BoundVar { .. }
        | Type::Error => {}
    }
}

fn validate_type_id(program: &MirProgram, id: TypeId, report: &mut MirAgreementReport) {
    let Some(ty) = program.type_context.try_ty(id) else {
        report.invalid_type_ids += 1;
        return;
    };

    if program.type_context.kind(id) != &crate::type_services::kind::Kind::Type {
        report.invalid_type_ids += 1;
        return;
    }

    if matches!(ty, Ty::Projection { .. }) {
        let raw = program.type_context.type_for(id);
        let normalized = program
            .backend_contract
            .normalize_type(&program.type_context, &raw);
        if matches!(normalized, Type::Projection { .. }) {
            report.missing_projection_facts += 1;
        }
    }

    match ty {
        Ty::Slice(inner) | Ty::Reference { inner, .. } | Ty::Pointer(inner) => {
            validate_type_id(program, *inner, report);
        }
        Ty::Array { inner, .. } => validate_type_id(program, *inner, report),
        Ty::Tuple(elems) => {
            for elem in elems {
                validate_type_id(program, *elem, report);
            }
        }
        Ty::Function {
            params,
            ret,
            captures,
            ..
        } => {
            for param in params {
                validate_type_id(program, *param, report);
            }
            validate_type_id(program, *ret, report);
            for capture in captures {
                validate_type_id(program, capture.ty, report);
            }
        }
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => {
            for arg in args {
                if program.type_context.kind(*arg) == &crate::type_services::kind::Kind::Type {
                    validate_type_id(program, *arg, report);
                }
            }
        }
        Ty::Projection { ty, trait_args, .. } => {
            validate_type_id(program, *ty, report);
            for arg in trait_args {
                validate_type_id(program, *arg, report);
            }
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
        | Ty::Str
        | Ty::Char
        | Ty::Unit
        | Ty::Never => {}
        Ty::TypeVar(_)
        | Ty::Generic(_)
        | Ty::Constructor { .. }
        | Ty::Apply { .. }
        | Ty::Lambda { .. }
        | Ty::BoundVar { .. }
        | Ty::Error => report.invalid_type_ids += 1,
    }
}

fn validate_rvalue_type_ids(
    program: &MirProgram,
    callable_contract: &CallableContract<'_>,
    function: &MirFunction,
    destination: &Place,
    rvalue: &Rvalue,
    report: &mut MirAgreementReport,
) {
    match rvalue {
        Rvalue::Use(operand) => {
            validate_runtime_operand(program, operand, true, report);
            if callable_contract.is_missing(operand) {
                report.missing_callable_metadata += 1;
            }
            if matches!(operand, Operand::Constant(Constant::Callable(_))) {
                let destination_ty = program.backend_contract.place_type_id(
                    &program.type_context,
                    function,
                    destination,
                );
                if !destination_ty.is_some_and(|ty| is_function_type_id(program, ty)) {
                    report.invalid_constant_operands += 1;
                }
            }
        }
        Rvalue::Ref(_, _) | Rvalue::Closure(_) | Rvalue::Discriminant(_) => {}
        Rvalue::Cast(operand, target_ty) => {
            validate_runtime_operand(program, operand, false, report);
            validate_type_id(program, *target_ty, report);
        }
        Rvalue::BinaryOp(_, left, right) => {
            validate_runtime_operand(program, left, false, report);
            validate_runtime_operand(program, right, false, report);
        }
        Rvalue::UnaryOp(_, operand) => validate_runtime_operand(program, operand, false, report),
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                validate_runtime_operand(program, operand, false, report);
            }
        }
    }
}

fn validate_rvalue_layout_ids(
    backend_contract: &BackendContract,
    rvalue: &Rvalue,
    report: &mut MirAgreementReport,
) {
    match rvalue {
        Rvalue::Ref(_, place) | Rvalue::Discriminant(place) => {
            validate_place_layout_ids(backend_contract, place, report);
        }
        Rvalue::Aggregate(kind, _) => validate_aggregate_layout_id(backend_contract, kind, report),
        Rvalue::Use(_)
        | Rvalue::Cast(_, _)
        | Rvalue::Closure(_)
        | Rvalue::BinaryOp(_, _, _)
        | Rvalue::UnaryOp(_, _) => {}
    }
}

fn validate_place_layout_ids(
    backend_contract: &BackendContract,
    place: &Place,
    report: &mut MirAgreementReport,
) {
    for projection in &place.projection {
        match projection {
            Projection::Field {
                identity: Some(identity),
                ..
            } if !backend_contract.nominal_layout_known(identity.owner) => {
                report.invalid_layout_ids += 1;
            }
            Projection::Field { .. } | Projection::Deref | Projection::Index(_) => {}
            Projection::Downcast(_) => {}
        }
    }
}

fn validate_aggregate_layout_id(
    backend_contract: &BackendContract,
    kind: &AggregateKind,
    report: &mut MirAgreementReport,
) {
    match kind {
        AggregateKind::Struct { id, .. } if !backend_contract.structs.contains(id) => {
            report.invalid_layout_ids += 1;
        }
        AggregateKind::EnumVariant {
            enum_id,
            variant_id,
            ..
        } if !backend_contract.enum_variant_known(*enum_id, *variant_id) => {
            report.invalid_layout_ids += 1;
        }
        AggregateKind::Tuple | AggregateKind::Array | AggregateKind::Struct { .. } => {}
        AggregateKind::EnumVariant { .. } => {}
    }
}

fn validate_operand_type_ids(
    program: &MirProgram,
    operand: &Operand,
    report: &mut MirAgreementReport,
) {
    match operand {
        Operand::Constant(Constant::Callable(callable)) => {
            validate_callable_type_ids(program, callable, report);
        }
        Operand::Constant(Constant::TypeId(id)) => validate_type_id(program, *id, report),
        Operand::Copy(_) | Operand::Move(_) | Operand::Constant(_) => {}
    }
}

fn validate_runtime_operand(
    program: &MirProgram,
    operand: &Operand,
    allow_callable: bool,
    report: &mut MirAgreementReport,
) {
    validate_operand_type_ids(program, operand, report);
    if matches!(operand, Operand::Constant(Constant::TypeId(_)))
        || (!allow_callable && matches!(operand, Operand::Constant(Constant::Callable(_))))
    {
        report.invalid_constant_operands += 1;
    }
}

fn validate_call_constant_operands(
    program: &MirProgram,
    callable_contract: &CallableContract<'_>,
    function: &MirFunction,
    func: &Operand,
    args: &[Operand],
    report: &mut MirAgreementReport,
) {
    let metadata_intrinsic = match func {
        Operand::Constant(Constant::Callable(MirCallable::Resolved(
            MirCallableKey::Intrinsic(intrinsic),
        ))) if intrinsic.is_type_only() => Some(intrinsic),
        _ => None,
    };
    let contract_params = callable_contract.params_for_operand(func);
    let indirect_params = function_operand_params(program, function, func);

    let metadata_args_valid = match metadata_intrinsic {
        Some(crate::mir::MirIntrinsicId::SizeOf) => {
            args.len() == 1 && matches!(args[0], Operand::Constant(Constant::TypeId(_)))
        }
        Some(crate::mir::MirIntrinsicId::ArrayLen) => {
            array_len_args_are_valid(program, function, args)
        }
        Some(_) => unreachable!("is_type_only returned true for an unhandled MIR intrinsic"),
        None => true,
    };
    if !metadata_args_valid {
        report.invalid_constant_operands += 1;
    }

    for (index, arg) in args.iter().enumerate() {
        validate_operand_type_ids(program, arg, report);
        if metadata_intrinsic.is_none() && matches!(arg, Operand::Constant(Constant::TypeId(_))) {
            report.invalid_constant_operands += 1;
        }
        if matches!(arg, Operand::Constant(Constant::Callable(_))) {
            let expected_ty = contract_params
                .and_then(|params| params.get(index).map(|param| param.semantic_ty))
                .or_else(|| indirect_params.and_then(|params| params.get(index).copied()));
            if !expected_ty.is_some_and(|ty| is_function_type_id(program, ty)) {
                report.invalid_constant_operands += 1;
            }
        }
    }
}

fn array_len_args_are_valid(
    program: &MirProgram,
    function: &MirFunction,
    args: &[Operand],
) -> bool {
    let [arg] = args else {
        return false;
    };
    match arg {
        Operand::Constant(Constant::TypeId(id)) => {
            matches!(program.type_context.try_ty(*id), Some(Ty::Array { .. }))
        }
        Operand::Copy(place) | Operand::Move(place) => program
            .backend_contract
            .place_type_id(&program.type_context, function, place)
            .is_some_and(|id| array_len_runtime_type_is_supported(program, id)),
        Operand::Constant(_) => false,
    }
}

fn array_len_runtime_type_is_supported(program: &MirProgram, id: TypeId) -> bool {
    match program.type_context.try_ty(id) {
        Some(Ty::Array { .. } | Ty::Slice(_)) => true,
        Some(Ty::Reference { inner, .. }) => matches!(
            program.type_context.try_ty(*inner),
            Some(Ty::Array { .. } | Ty::Slice(_) | Ty::Str)
        ),
        _ => false,
    }
}

fn validate_callable_type_ids(
    _program: &MirProgram,
    _callable: &MirCallable,
    _report: &mut MirAgreementReport,
) {
}

fn validate_call_argument_metadata(
    program: &MirProgram,
    callable_contract: &CallableContract<'_>,
    function: &MirFunction,
    func: &Operand,
    args: &[Operand],
    report: &mut MirAgreementReport,
) {
    if let Some(params) = callable_contract.params_for_operand(func) {
        for (arg, param) in args.iter().zip(params) {
            if is_function_type_id(program, param.semantic_ty) && callable_contract.is_missing(arg)
            {
                report.missing_callable_metadata += 1;
            }
        }
        return;
    }

    let Some(params) = function_operand_params(program, function, func) else {
        return;
    };
    for (arg, param) in args.iter().zip(params) {
        if is_function_type_id(program, *param) && callable_contract.is_missing(arg) {
            report.missing_callable_metadata += 1;
        }
    }
}

fn function_operand_params<'a>(
    program: &'a MirProgram,
    function: &MirFunction,
    operand: &Operand,
) -> Option<&'a [TypeId]> {
    let local = match operand {
        Operand::Copy(place) | Operand::Move(place) if place.projection.is_empty() => place.local,
        _ => return None,
    };
    let local_ty = function.local_decls.get(local.0)?.ty;
    match program.type_context.try_ty(local_ty)? {
        Ty::Function { params, .. } => Some(params.as_slice()),
        _ => None,
    }
}

fn is_function_type_id(program: &MirProgram, id: TypeId) -> bool {
    matches!(program.type_context.try_ty(id), Some(Ty::Function { .. }))
}

struct CallableContract<'a> {
    callables: &'a std::collections::BTreeMap<MirCallableKey, MirCallableDecl>,
}

struct BackendContract {
    structs: HashSet<DefId>,
    enums: HashSet<DefId>,
    projection_traits: HashSet<DefId>,
}

impl BackendContract {
    fn new(program: &MirProgram) -> Self {
        let mut structs = HashSet::new();
        let mut enums = HashSet::new();
        for (id, layout) in &program.backend_contract.nominal_layouts {
            match layout {
                MirNominalLayout::Struct { .. } => {
                    structs.insert(*id);
                }
                MirNominalLayout::Enum { .. } => {
                    enums.insert(*id);
                }
            }
        }
        let projection_traits: HashSet<DefId> = program
            .backend_contract
            .projection_traits
            .iter()
            .copied()
            .collect();

        Self {
            structs,
            enums,
            projection_traits,
        }
    }

    fn nominal_layout_known(&self, id: DefId) -> bool {
        self.structs.contains(&id) || self.enums.contains(&id)
    }

    fn enum_variant_known(&self, enum_id: DefId, _variant_id: crate::ids::VariantId) -> bool {
        self.enums.contains(&enum_id)
    }

    fn projection_trait_known(&self, trait_id: DefId) -> bool {
        self.projection_traits.contains(&trait_id)
    }
}

impl<'a> CallableContract<'a> {
    fn new(program: &'a MirProgram) -> Self {
        Self {
            callables: &program.backend_contract.callables,
        }
    }

    fn params_for_operand(&self, operand: &Operand) -> Option<&[MirParamAbi]> {
        match operand {
            Operand::Constant(Constant::Callable(callable)) => self.params_for_callable(callable),
            _ => None,
        }
    }

    fn params_for_callable(&self, callable: &MirCallable) -> Option<&[MirParamAbi]> {
        let key = Self::callable_key(callable);
        self.callables
            .get(&key)
            .map(|declaration| declaration.signature.params.as_slice())
    }

    fn is_missing(&self, operand: &Operand) -> bool {
        match operand {
            Operand::Constant(Constant::Callable(callable)) => self.callable_is_missing(callable),
            _ => false,
        }
    }

    fn callable_is_missing(&self, callable: &MirCallable) -> bool {
        !self.callables.contains_key(Self::callable_key(callable))
    }

    fn callable_key(callable: &MirCallable) -> &MirCallableKey {
        let MirCallable::Resolved(key) = callable;
        key
    }
}

fn is_unit_callable_operand(
    type_view: TypeView<'_>,
    function: &crate::mir::MirFunction,
    operand: &Operand,
) -> bool {
    match operand {
        Operand::Constant(Constant::Unit) => true,
        Operand::Copy(place) | Operand::Move(place) => {
            place.projection.is_empty()
                && function
                    .local_decls
                    .get(place.local.0)
                    .is_some_and(|local| matches!(type_view.try_ty(local.ty), Some(Ty::Unit)))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
