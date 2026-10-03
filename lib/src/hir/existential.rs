//! Independent scope validation at the accepted-HIR boundary.
use std::collections::{HashMap, HashSet};

use super::*;
use super::{HirExprKindFor as HirExprKind, HirStmtFor as HirStmt};
use crate::types::{Type, WitnessId};

pub(crate) fn validate_program(program: &HirProgram, errors: &mut Vec<String>) {
    for function in program
        .functions
        .values()
        .chain(
            program
                .impls
                .values()
                .flat_map(|implementation| implementation.methods.values()),
        )
        .chain(
            program
                .traits
                .values()
                .flat_map(|declaration| declaration.methods.values()),
        )
    {
        validate_function(program, function, errors);
    }
    // Opened witnesses are never exported as declaration parameters or fields.
    let mut context =
        crate::type_context::TypeContext::with_normalization_env(program.type_normalization_env());
    let ids = super::collect_hir_type_ids(program, &mut context);
    for (location, id) in ids.iter() {
        let declaration = matches!(
            location,
            HirTypeLocation::FunctionReturn { .. }
                | HirTypeLocation::FunctionParam { .. }
                | HirTypeLocation::FunctionBoundArg { .. }
                | HirTypeLocation::ExternReturn { .. }
                | HirTypeLocation::ExternParam { .. }
                | HirTypeLocation::StructField { .. }
                | HirTypeLocation::EnumVariantNamedField { .. }
                | HirTypeLocation::EnumVariantPositionalField { .. }
                | HirTypeLocation::TraitSignatureReturn { .. }
                | HirTypeLocation::TraitSignatureParam { .. }
                | HirTypeLocation::TraitSignatureBoundArg { .. }
                | HirTypeLocation::ImplReceiverArg { .. }
                | HirTypeLocation::ImplTraitArg { .. }
                | HirTypeLocation::ImplBoundArg { .. }
                | HirTypeLocation::PredicateSubject { .. }
                | HirTypeLocation::PredicateArg { .. }
                | HirTypeLocation::AssociatedTypeDef { .. }
                | HirTypeLocation::TypeAliasBody { .. }
        );
        if declaration && has_witness(&context.type_for(id)) {
            errors.push(format!(
                "opened witness escapes into declaration at {location:?}"
            ));
        }
    }
}

fn has_witness(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| matches!(nested, Type::Witness(_)))
}

pub(crate) fn validate_function<P: HirPhase>(
    program: &HirProgram,
    function: &HirFunctionFor<P>,
    errors: &mut Vec<String>,
) {
    let mut scope = Scope::<P> {
        program,
        owner: function.id,
        active: HashMap::new(),
        seen_openings: HashSet::new(),
        local_types: function
            .params
            .iter()
            .map(|param| (param.local_id, param.ty.clone()))
            .collect(),
        mutable_locals: function
            .params
            .iter()
            .filter(|parameter| parameter.mutable)
            .map(|parameter| parameter.local_id)
            .collect(),
        loops: Vec::new(),
        returns: Vec::new(),
        return_ty: &function.ret_type,
        uncaptured_witness_locals: Vec::new(),
        errors,
        phase: std::marker::PhantomData,
    };
    scope.ty(&function.ret_type);
    for parameter in &function.params {
        scope.ty(&parameter.ty);
    }
    for (_, bounds) in &function.generic_bounds {
        for bound in bounds {
            for ty in &bound.type_args {
                scope.ty(ty);
            }
        }
    }
    for predicate in &function.generic_bounds.predicates {
        let crate::types::Predicate::Trait { subject, args, .. } = predicate;
        scope.ty(subject);
        for ty in args {
            scope.ty(ty);
        }
    }
    scope.block(&function.body);
}

struct Scope<'a, P: HirPhase> {
    program: &'a HirProgram,
    owner: DefId,
    active: HashMap<WitnessId, crate::types::ObjectType>,
    seen_openings: HashSet<WitnessId>,
    local_types: HashMap<HirLocalId, Type>,
    mutable_locals: HashSet<HirLocalId>,
    loops: Vec<(Vec<WitnessId>, &'a Type)>,
    returns: Vec<WitnessId>,
    return_ty: &'a Type,
    uncaptured_witness_locals: Vec<HashSet<HirLocalId>>,
    errors: &'a mut Vec<String>,
    phase: std::marker::PhantomData<P>,
}

impl<'a, P: HirPhase> Scope<'a, P> {
    fn mutable_place(&self, expression: &HirExprFor<P>) -> bool {
        match &expression.kind {
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Local(local),
                ..
            }) => self.mutable_locals.contains(local),
            HirExprKind::Deref(reference) => matches!(
                reference.ty,
                Type::Reference { mutable: true, .. } | Type::Pointer(_)
            ),
            HirExprKind::FieldAccess(base, _, _) | HirExprKind::TupleIndex(base, _) => {
                self.mutable_place(base)
            }
            _ => false,
        }
    }

    fn result(&mut self, expected: &Type, actual: &Type, context: &str) {
        if (has_witness(expected) || has_witness(actual))
            && expected != actual
            && actual != &Type::Never
        {
            self.errors.push(format!(
                "opened witness result disagrees with its {context}; cannot erase or exchange its identity"
            ));
        }
    }

    fn pattern(&mut self, pattern: &HirPattern) {
        match pattern {
            HirPattern::Struct(_, _, args, fields) => {
                for ty in args {
                    self.ty(ty);
                }
                for field in fields {
                    self.pattern(&field.pattern);
                }
            }
            HirPattern::Tuple(patterns)
            | HirPattern::Or(patterns)
            | HirPattern::Enum(_, _, _, patterns) => {
                for pattern in patterns {
                    self.pattern(pattern);
                }
            }
            _ => {}
        }
    }

    fn ty(&mut self, ty: &Type) {
        if let Err(message) = crate::type_services::existential::validate_package_layouts(
            ty,
            &self.program.structs,
            &self.program.enums,
            &self.program.type_normalization_env(),
        ) {
            self.errors.push(message);
        }
        crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
            if let Type::Witness(witness) = nested {
                if !self.active.contains_key(witness) {
                    self.errors.push(format!(
                        "opened witness {witness:?} has no dominating descriptor/dictionary scope"
                    ));
                }
            }
        });
    }

    fn target(&mut self, target: &HirMethodCallTarget) {
        for ty in target.trait_args() {
            self.ty(ty);
        }
        for binding in target
            .owner_substitution
            .iter()
            .chain(&target.method_substitution)
        {
            self.ty(&binding.ty);
        }
        if let HirSelectedMethodTarget::TraitMethod {
            dispatch: HirTraitDispatchKind::Opened(witness),
            ..
        } = &target.target
        {
            if !self.active.contains_key(witness) {
                self.errors
                    .push("opened method authority has no dominating dictionary".into());
            }
        }
    }

    fn call_target(&mut self, target: &HirCallTarget) {
        match target {
            HirCallTarget::StaticMethod(target) => {
                self.ty(&target.owner_ty);
                self.target(&target.method);
            }
            _ => {}
        }
    }

    fn block(&mut self, block: &'a HirBlockFor<P>) {
        self.ty(&block.ty);
        if let Some(HirStmt::Expr(tail)) = block.stmts.last() {
            self.result(&block.ty, &tail.ty, "block tail");
        }
        for statement in &block.stmts {
            match statement {
                HirStmt::Let {
                    ty,
                    value,
                    local_id,
                    mutable,
                    ..
                } => {
                    self.ty(ty);
                    self.expr(value);
                    if (has_witness(ty) || has_witness(&value.ty))
                        && ty != &value.ty
                        && value.ty != Type::Never
                    {
                        self.errors.push(
                            "opened witness local disagrees with its initializer type".into(),
                        );
                    }
                    self.local_types.insert(*local_id, ty.clone());
                    if *mutable {
                        self.mutable_locals.insert(*local_id);
                    }
                }
                HirStmt::Expr(value) => self.expr(value),
                HirStmt::Return(Some(value)) => {
                    self.result(self.return_ty, &value.ty, "return target");
                    if crate::type_services::visit::type_any(
                        &value.ty,
                        |ty| matches!(ty, Type::Witness(witness) if !self.returns.contains(witness)),
                    ) {
                        self.errors.push(
                            "opened witness escapes through an early return; repackage it".into(),
                        );
                    }
                    self.expr(value);
                }
                HirStmt::Break(Some(value)) => {
                    let allowed = self
                        .loops
                        .last()
                        .map(|(witnesses, _)| witnesses.clone())
                        .unwrap_or_default();
                    if let Some(result) = self.loops.last().map(|(_, result)| *result) {
                        self.result(result, &value.ty, "break target");
                    }
                    if crate::type_services::visit::type_any(
                        &value.ty,
                        |ty| matches!(ty, Type::Witness(witness) if !allowed.contains(witness)),
                    ) {
                        self.errors
                            .push("opened witness escapes through a break".into());
                    }
                    self.expr(value);
                }
                _ => {}
            }
        }
    }

    fn expr(&mut self, expression: &'a HirExprFor<P>) {
        // The result of an Open is checked in the *outer* scope.
        self.ty(&expression.ty);
        match &expression.kind {
            HirExprKind::PackagePack { payload, proof } => {
                self.expr(payload);
                proof.visit_types(&mut |ty| self.ty(ty));
                let opened = if let Type::Witness(witness) = &proof.witness {
                    self.active.get(witness)
                } else {
                    None
                };
                let bounds = self
                    .program
                    .functions
                    .get(&self.owner)
                    .map(|f| &f.generic_bounds)
                    .cloned()
                    .unwrap_or_default();
                if let Err(message) = super::packages::validate_pack(
                    self.program,
                    payload,
                    &expression.ty,
                    proof,
                    &bounds,
                    opened,
                ) {
                    self.errors.push(message);
                }
            }
            HirExprKind::PackageOpen {
                source,
                binding,
                body,
            } => {
                self.expr(source);
                if binding.witness.owner != self.owner
                    || binding.witness.local != binding.value.local_id
                    || !self.seen_openings.insert(binding.witness)
                    || binding.source_ty != source.ty
                    || expression.ty != body.ty
                    || binding.value.mutable
                    || binding.value.is_ref != matches!(binding.value.ty, Type::Reference { .. })
                {
                    self.errors
                        .push("package opening has forged binder/source/result authority".into());
                    return;
                }
                let Type::Exists(package) = &source.ty else {
                    self.errors
                        .push("package opening requires a by-value existential source".into());
                    return;
                };
                let expected = crate::type_services::substitution::instantiate_existential(
                    package,
                    &Type::Witness(binding.witness),
                );
                if !expected
                    .as_ref()
                    .is_ok_and(|package| package.payload == binding.value.ty)
                {
                    self.errors.push(
                        "package opening payload disagrees with its exact witness substitution"
                            .into(),
                    );
                    return;
                }
                let Ok(schema) = binding.object_schema() else {
                    self.errors
                        .push("package opening has malformed bound binders".into());
                    return;
                };
                self.active.insert(binding.witness, schema);
                let previous = self
                    .local_types
                    .insert(binding.value.local_id, binding.value.ty.clone());
                if previous.is_some() {
                    self.errors
                        .push("package opening reuses an existing value-local identity".into());
                }
                binding.visit_types(&mut |ty| self.ty(ty));
                self.block(body);
                self.active.remove(&binding.witness);
                self.local_types.remove(&binding.value.local_id);
                if let Some(ty) = previous {
                    self.local_types.insert(binding.value.local_id, ty);
                }
            }
            HirExprKind::Open {
                source,
                binding,
                body,
            } => {
                self.expr(source);
                if expression.ty != body.ty {
                    self.errors
                        .push("opening result type disagrees with its scoped body".into());
                }
                if binding.witness.owner != self.owner
                    || binding.witness.local != binding.value.local_id
                    || !self.seen_openings.insert(binding.witness)
                    || binding.source_ty != source.ty
                {
                    self.errors
                        .push("opening has forged or duplicate witness/source authority".into());
                    return;
                }
                let expected = match &source.ty {
                    Type::Reference { mutable, inner }
                        if inner.as_ref() == &Type::Object(Box::new(binding.object.clone()))
                            && binding.owner.is_none() =>
                    {
                        Some(Type::Reference {
                            mutable: *mutable,
                            inner: Box::new(Type::Witness(binding.witness)),
                        })
                    }
                    Type::Struct { id, args }
                        if args.as_slice() == [Type::Object(Box::new(binding.object.clone()))]
                            && binding.owner.is_some() =>
                    {
                        Some(Type::Struct {
                            id: *id,
                            args: vec![Type::Witness(binding.witness)],
                        })
                    }
                    _ => None,
                };
                if expected.as_ref() != Some(&binding.value.ty) || binding.value.mutable {
                    self.errors.push(
                        "opening handle does not preserve its exact source access capability"
                            .into(),
                    );
                }
                self.active.insert(binding.witness, binding.object.clone());
                let old_local = self
                    .local_types
                    .insert(binding.value.local_id, binding.value.ty.clone());
                if old_local.is_some() {
                    self.errors
                        .push("opening reuses an existing value-local identity".into());
                }
                binding.visit_types(&mut |ty| self.ty(ty));
                if let Some(owner) = &binding.owner {
                    self.target(&owner.into_parts.method);
                    self.target(&owner.from_parts.method);
                    if let Err(message) = validate_owner(self.program, binding) {
                        self.errors.push(message);
                    }
                }
                self.block(body);
                self.active.remove(&binding.witness);
                self.local_types.remove(&binding.value.local_id);
                if let Some(ty) = old_local {
                    self.local_types.insert(binding.value.local_id, ty);
                }
            }
            HirExprKind::MethodCall(receiver, _, args, mode, target) => {
                self.expr(receiver);
                for arg in args {
                    self.expr(arg);
                }
                if let Some(target) = P::method_authority(target) {
                    self.target(target);
                    if let HirSelectedMethodTarget::TraitMethod {
                        trait_id,
                        trait_args,
                        dispatch: HirTraitDispatchKind::Opened(witness),
                        ..
                    } = &target.target
                    {
                        if let Some(object) = self.active.get(witness) {
                            let receiver_ok = (*mode == Some(ReceiverMode::Move)
                                && receiver.ty == Type::Witness(*witness))
                                || matches!(&receiver.ty, Type::Reference { mutable, inner } if inner.as_ref() == &Type::Witness(*witness) && (*mode == Some(ReceiverMode::Shared) || (*mode == Some(ReceiverMode::Mut) && *mutable)));
                            let opened = object.try_map_types(|ty| {
                                crate::type_services::substitution::instantiate_object_self(
                                    ty,
                                    &Type::Witness(*witness),
                                )
                            });
                            let Ok(opened) = opened else {
                                self.errors
                                    .push("opening schema has malformed Self binders".into());
                                return;
                            };
                            let roots = std::iter::once(opened.principal.clone())
                                .chain(opened.guarantees.iter().cloned())
                                .collect::<Vec<_>>();
                            let bounds = crate::traits::evidence::implied_trait_bounds(
                                &self.program.traits,
                                &Type::Witness(*witness),
                                &roots,
                            );
                            let bound_ok = bounds.iter().any(|bound| {
                                bound.trait_id == *trait_id && bound.type_args == *trait_args
                            });
                            let signature = object_methods::opened_signature(
                                &self.program.traits,
                                target,
                                object,
                                *witness,
                            );
                            let signature_ok = signature.as_ref().is_ok_and(|(params, ret)| {
                                ret == &expression.ty
                                    && params.len() == args.len()
                                    && params.iter().zip(args).all(|(param, arg)| param == &arg.ty)
                            });
                            if !receiver_ok || !bound_ok || !signature_ok {
                                self.errors.push(format!(
                                    "opened call does not match its exact scoped witness signature/access: receiver_ok={receiver_ok}, bound_ok={bound_ok}, signature_ok={signature_ok}; receiver={:?}, mode={mode:?}, target={target:?}, signature={signature:?}, arguments={:?}, result={:?}",
                                    receiver.ty,
                                    args.iter().map(|argument| &argument.ty).collect::<Vec<_>>(),
                                    expression.ty,
                                ));
                            }
                        }
                    }
                }
            }
            HirExprKind::Call(callee, args, target) => {
                self.expr(callee);
                for argument in args {
                    self.expr(argument);
                }
                if let Some(target) = target {
                    self.call_target(target);
                }
                if let Type::Function { params, ret, .. } = &callee.ty {
                    if params.len() == args.len() {
                        self.result(&expression.ty, ret, "call signature");
                        for (parameter, argument) in params.iter().zip(args) {
                            self.result(parameter, &argument.ty, "call argument");
                        }
                    }
                }
            }
            HirExprKind::OwnedObjectCall { owner, args, call } => {
                self.expr(owner);
                for argument in args {
                    self.expr(argument);
                }
                call.visit_types(&mut |ty| self.ty(ty));
                self.target(&call.method);
                if let HirSelectedMethodTarget::TraitMethod {
                    dispatch: HirTraitDispatchKind::Opened(witness),
                    ..
                } = &call.method.target
                {
                    if self
                        .active
                        .get(witness)
                        .is_none_or(|object| call.object != Type::Object(Box::new(object.clone())))
                    {
                        self.errors.push(
                            "consuming opened call has no exact dominating object schema".into(),
                        );
                    }
                }
                if let Err(message) =
                    super::owned_objects::validate(self.program, owner, args, &expression.ty, call)
                {
                    self.errors.push(message);
                }
            }
            HirExprKind::ObjectCoercion(value, coercion) => {
                self.expr(value);
                coercion.visit_types(&mut |ty| self.ty(ty));
                let proof = match &value.ty {
                    Type::Reference { inner, .. } | Type::Pointer(inner) => match inner.as_ref() {
                        Type::Witness(witness) => self
                            .active
                            .get(witness)
                            .ok_or_else(|| {
                                "opened coercion has no dominating dictionary".to_string()
                            })
                            .and_then(|object| {
                                let bounds = HirGenericBounds::new();
                                let context =
                                    crate::infer::object_coercion::ObjectEvidenceContext {
                                        traits: &self.program.traits,
                                        impls: &self.program.impls,
                                        structs: &self.program.structs,
                                        enums: &self.program.enums,
                                        language_items: &self.program.language_items,
                                        bounds: &bounds,
                                    };
                                let expected = context.prove_opened(
                                    &value.ty,
                                    &coercion.target,
                                    *witness,
                                    object,
                                )?;
                                super::validate_object_evidence(
                                    &expected,
                                    coercion
                                        .evidence
                                        .as_ref()
                                        .ok_or("object coercion has no evidence")?,
                                )
                            }),
                        _ => super::validate_object_coercion(self.program, value, coercion),
                    },
                    _ => super::validate_object_coercion(self.program, value, coercion),
                };
                if coercion.target != expression.ty {
                    self.errors
                        .push("object coercion target disagrees with expression type".into());
                }
                if let Err(message) = proof {
                    self.errors.push(message);
                }
            }
            HirExprKind::Cast(value, ty) => {
                self.expr(value);
                self.ty(ty);
            }
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Local(local),
                ..
            }) => {
                if self
                    .uncaptured_witness_locals
                    .iter()
                    .any(|locals| locals.contains(local))
                {
                    self.errors
                        .push("opened witness closure body uses an uncaptured local".into());
                }
                if let Some(declared) = self.local_types.get(local) {
                    if (has_witness(declared) || has_witness(&expression.ty))
                        && declared != &expression.ty
                    {
                        self.errors.push(
                            "opened witness local use forges its declared access/type".into(),
                        );
                    }
                }
            }
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expr(condition);
                self.result(&expression.ty, &then_branch.ty, "if branch");
                self.block(then_branch);
                if let Some(body) = else_branch {
                    self.result(&expression.ty, &body.ty, "if branch");
                    self.block(body);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for arm in arms {
                    self.result(&expression.ty, &arm.body.ty, "match arm");
                    self.pattern(&arm.pattern);
                    if let Some(guard) = &arm.guard {
                        self.expr(guard);
                    }
                    self.block(&arm.body);
                }
            }
            HirExprKind::While { condition, body } => {
                self.expr(condition);
                self.loop_body(body, &expression.ty);
            }
            HirExprKind::For { iter, body, .. } => {
                self.expr(iter);
                self.loop_body(body, &expression.ty);
            }
            HirExprKind::Loop(body) => self.loop_body(body, &expression.ty),
            HirExprKind::Block(body) | HirExprKind::UnsafeBlock(body) => {
                self.result(&expression.ty, &body.ty, "block expression");
                self.block(body);
            }
            HirExprKind::Lambda {
                params,
                captures,
                body,
            } => {
                for param in params {
                    self.ty(&param.ty);
                }
                for capture in captures {
                    self.ty(&capture.ty);
                    if self
                        .uncaptured_witness_locals
                        .iter()
                        .any(|locals| locals.contains(&capture.local_id))
                    {
                        self.errors.push("opened witness closure captures a local unavailable in its enclosing closure".into());
                    }
                    if let Some(declared) = self.local_types.get(&capture.local_id).cloned() {
                        self.result(&declared, &capture.ty, "closure capture binding");
                    }
                }
                let previous_return_ty = self.return_ty;
                if let Type::Function {
                    params: signature_params,
                    ret,
                    captures: signature_captures,
                    ..
                } = &expression.ty
                {
                    let actual_captures = captures
                        .iter()
                        .map(|capture| crate::types::FunctionCapture {
                            ty: capture.ty.clone(),
                            kind: match capture.kind {
                                HirClosureCaptureKind::SharedBorrow => {
                                    crate::types::CaptureKind::SharedBorrow
                                }
                                HirClosureCaptureKind::MutableBorrow => {
                                    crate::types::CaptureKind::MutableBorrow
                                }
                                HirClosureCaptureKind::Move => crate::types::CaptureKind::Move,
                            },
                        })
                        .collect::<Vec<_>>();
                    if (has_witness(&expression.ty)
                        || captures.iter().any(|capture| has_witness(&capture.ty)))
                        && &actual_captures != signature_captures
                    {
                        self.errors.push("opened witness closure capture metadata disagrees with its function type".into());
                    }
                    if signature_params.len() != params.len()
                        && (has_witness(&expression.ty)
                            || params.iter().any(|parameter| has_witness(&parameter.ty)))
                    {
                        self.errors.push(
                            "opened witness closure parameters disagree with its function type"
                                .into(),
                        );
                    }
                    for (expected, parameter) in signature_params.iter().zip(params) {
                        self.result(expected, &parameter.ty, "closure parameter");
                    }
                    self.result(ret, &body.ty, "closure result");
                    self.return_ty = ret;
                } else if params.iter().any(|parameter| has_witness(&parameter.ty))
                    || captures.iter().any(|capture| has_witness(&capture.ty))
                    || has_witness(&body.ty)
                {
                    self.errors
                        .push("opened witness closure has no matching function type".into());
                }
                let previous_locals = self.local_types.clone();
                let previous_mutable_locals = self.mutable_locals.clone();
                self.uncaptured_witness_locals.push(
                    self.local_types
                        .iter()
                        .filter_map(|(local, ty)| {
                            (has_witness(ty)
                                && !captures.iter().any(|capture| capture.local_id == *local)
                                && !params.iter().any(|parameter| parameter.local_id == *local))
                            .then_some(*local)
                        })
                        .collect(),
                );
                for parameter in params {
                    self.local_types
                        .insert(parameter.local_id, parameter.ty.clone());
                    if parameter.mutable {
                        self.mutable_locals.insert(parameter.local_id);
                    }
                }
                let loops = std::mem::take(&mut self.loops);
                let returns =
                    std::mem::replace(&mut self.returns, self.active.keys().copied().collect());
                self.block(body);
                self.loops = loops;
                self.returns = returns;
                self.return_ty = previous_return_ty;
                self.local_types = previous_locals;
                self.mutable_locals = previous_mutable_locals;
                self.uncaptured_witness_locals.pop();
            }
            HirExprKind::Try {
                expr,
                branch_method,
                branch_target,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                ..
            } => {
                self.expr(expr);
                self.ty(output_ty);
                self.ty(residual_ty);
                self.ty(return_ty);
                self.result(self.return_ty, return_ty, "residual return target");
                if crate::type_services::visit::type_any(
                    return_ty,
                    |ty| matches!(ty, Type::Witness(witness) if !self.returns.contains(witness)),
                ) {
                    self.errors.push(
                        "opened witness escapes through a residual return; repackage it".into(),
                    );
                }
                if let Some(target) = P::method_authority(branch_method) {
                    self.target(target);
                }
                if let Some(target) = branch_target {
                    self.call_target(target);
                }
                if let Some(target) = P::residual_authority(from_residual_target) {
                    self.call_target(target);
                }
            }
            HirExprKind::TupleLiteral(values) => {
                for value in values {
                    self.expr(value);
                }
                if has_witness(&expression.ty) || values.iter().any(|value| has_witness(&value.ty))
                {
                    let expected =
                        Type::Tuple(values.iter().map(|value| value.ty.clone()).collect());
                    if expression.ty != expected {
                        self.errors.push(
                            "opened payload tuple literal exchanges or erases witness identities"
                                .into(),
                        );
                    }
                }
            }
            HirExprKind::ArrayLiteral(values) => {
                for value in values {
                    self.expr(value);
                }
                if has_witness(&expression.ty) || values.iter().any(|value| has_witness(&value.ty))
                {
                    if !matches!(&expression.ty, Type::Array(element, len) if *len == values.len() && values.iter().all(|value| value.ty == **element || value.ty == Type::Never))
                    {
                        self.errors.push(
                            "opened payload array literal exchanges or erases witness identities"
                                .into(),
                        );
                    }
                }
            }
            HirExprKind::EnumVariant(_, _, values, _)
            | HirExprKind::Intrinsic { args: values, .. } => {
                for value in values {
                    self.expr(value);
                }
            }
            HirExprKind::StructLiteral(_, definition_id, fields) => {
                for field in fields {
                    self.expr(&field.value);
                }
                if has_witness(&expression.ty)
                    || fields.iter().any(|field| has_witness(&field.value.ty))
                {
                    let Type::Struct { id, args } = &expression.ty else {
                        self.errors.push(
                            "opened payload struct literal has no canonical nominal type".into(),
                        );
                        return;
                    };
                    let valid = self.program.structs.get(id).is_some_and(|definition| {
                        if *definition_id != Some(*id)
                            || definition.generic_params.len() != args.len()
                            || definition.fields.len() != fields.len()
                        {
                            return false;
                        }
                        let substitution = definition
                            .generic_params
                            .iter()
                            .zip(args)
                            .map(|(parameter, ty)| (parameter.id, ty.clone()))
                            .collect();
                        let mut seen = std::collections::HashSet::new();
                        fields.iter().all(|field| {
                            let Some(location) = &field.field else {
                                return false;
                            };
                            if location.owner != *id || !seen.insert(location.field_id) {
                                return false;
                            }
                            definition
                                .fields
                                .iter()
                                .find(|declared| {
                                    declared.id == location.field_id && declared.name == field.name
                                })
                                .is_some_and(|declared| {
                                    field.value.ty == declared.ty.substitute_generics(&substitution)
                                        || field.value.ty == Type::Never
                                })
                        })
                    });
                    if !valid {
                        self.errors.push("opened payload struct literal forges canonical field or witness authority".into());
                    }
                }
            }
            HirExprKind::Ref(mutable, value) => {
                self.expr(value);
                if has_witness(&expression.ty) || has_witness(&value.ty) {
                    let expected = Type::Reference {
                        mutable: *mutable,
                        inner: Box::new(value.ty.clone()),
                    };
                    if expression.ty != expected || (*mutable && !self.mutable_place(value)) {
                        self.errors.push(
                            "opened witness reference forges its declared access/type".into(),
                        );
                    }
                }
            }
            HirExprKind::Deref(value) => {
                self.expr(value);
                if has_witness(&expression.ty) || has_witness(&value.ty) {
                    match &value.ty {
                        Type::Reference { inner, .. } | Type::Pointer(inner)
                            if inner.as_ref() == &expression.ty => {}
                        _ => self.errors.push(
                            "opened witness dereference forges its declared access/type".into(),
                        ),
                    }
                }
            }
            HirExprKind::TupleIndex(value, index) => {
                self.expr(value);
                if has_witness(&value.ty) || has_witness(&expression.ty) {
                    let Type::Tuple(elements) = &value.ty else {
                        self.errors.push(
                            "opened payload tuple projection has no canonical tuple source".into(),
                        );
                        return;
                    };
                    if elements.get(*index as usize) != Some(&expression.ty) {
                        self.errors.push(
                            "opened payload projection exchanges or erases its witness identity"
                                .into(),
                        );
                    }
                }
            }
            HirExprKind::FieldAccess(value, name, location) => {
                self.expr(value);
                if has_witness(&value.ty) || has_witness(&expression.ty) {
                    let Type::Struct { id, args } = &value.ty else {
                        self.errors
                            .push("opened payload field has no canonical nominal source".into());
                        return;
                    };
                    let expected = self.program.structs.get(id).and_then(|definition| {
                        if definition.generic_params.len() != args.len() {
                            return None;
                        }
                        let field = definition.fields.iter().find(|field| &field.name == name)?;
                        if !location.as_ref().is_some_and(|location| {
                            location.owner == *id && location.field_id == field.id
                        }) {
                            return None;
                        }
                        let substitution = definition
                            .generic_params
                            .iter()
                            .zip(args)
                            .map(|(parameter, ty)| (parameter.id, ty.clone()))
                            .collect();
                        Some(field.ty.substitute_generics(&substitution))
                    });
                    if expected.as_ref() != Some(&expression.ty) {
                        self.errors.push(
                            "opened payload field projection forges type or field authority".into(),
                        );
                    }
                }
            }
            HirExprKind::ArrayRepeat(value, _) | HirExprKind::UnaryOp(_, value) => self.expr(value),
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                self.expr(left);
                self.expr(right);
                if matches!(&expression.kind, HirExprKind::Assign(..))
                    && (has_witness(&left.ty) || has_witness(&right.ty))
                    && left.ty != right.ty
                    && right.ty != Type::Never
                {
                    self.errors.push(
                        "assignment cannot erase or exchange opened witness identities".into(),
                    );
                }
            }
            _ => {}
        }
    }

    fn loop_body(&mut self, body: &'a HirBlockFor<P>, result: &'a Type) {
        self.loops
            .push((self.active.keys().copied().collect(), result));
        self.block(body);
        self.loops.pop();
    }
}

fn validate_owner(program: &HirProgram, binding: &HirOpenBinding) -> Result<(), String> {
    let Some(owner) = &binding.owner else {
        return Ok(());
    };
    let bounds = HirGenericBounds::new();
    let service = crate::selection::SelectionService::new(
        &program.traits,
        &program.impls,
        program
            .language_items
            .sized
            .as_ref()
            .map(|item| item.trait_id),
        None,
        &bounds,
    )
    .with_effective_trait_methods(&program.indexes.effective_trait_methods);
    let expected = prove_open_owner(
        &binding.source_ty,
        &binding.value.ty,
        binding.witness,
        &service,
        &program.language_items,
    )?;
    if owner.state_ty != expected.state_ty
        || owner.into_parts != expected.into_parts
        || owner.from_parts != expected.from_parts
    {
        return Err(
            "opened owner split/rebuild authority disagrees with canonical protocol".into(),
        );
    }
    Ok(())
}

/// Opening transports the existing allocation; no construction or release is performed.
pub(crate) fn prove_open_owner(
    source: &Type,
    value: &Type,
    witness: WitnessId,
    service: &crate::selection::SelectionService<'_>,
    items: &HirLanguageItems,
) -> Result<HirOpenOwner, String> {
    let Type::Struct { id, args } = source else {
        return Err("opened owner requires nominal source".into());
    };
    let [payload @ Type::Object(_)] = args.as_slice() else {
        return Err("opened owner must contain one object payload".into());
    };
    if value
        != &(Type::Struct {
            id: *id,
            args: vec![Type::Witness(witness)],
        })
    {
        return Err("opening must preserve the exact owner constructor and witness".into());
    }
    let protocol = items
        .object_owner
        .as_ref()
        .ok_or("opened owner has no protocol provider")?;
    let constructor = Type::Constructor {
        id: *id,
        flavor: crate::types::NominalTypeKind::Struct,
    };
    let state = service
        .infer_trait_arguments(&constructor, protocol.trait_id)
        .ok_or("opened owner lacks unique constructor evidence")?;
    let [state_ty] = state.as_slice() else {
        return Err("invalid object_owner State arity".into());
    };
    if crate::type_services::visit::type_any(state_ty, |ty| {
        matches!(
            ty,
            Type::Witness(_) | Type::Generic(_) | Type::TypeVar(_) | Type::ObjectSelf { .. }
        )
    }) || crate::type_services::layout::TypeLayout::sizedness(state_ty, &|_| {
        crate::type_services::layout::Sizedness::Unknown
    }) != crate::type_services::layout::Sizedness::Sized
    {
        return Err("opened owner has forged payload-dependent State".into());
    }
    let expected_into = Type::function_with_safety(
        vec![source.clone()],
        Type::Tuple(vec![
            Type::Pointer(Box::new(payload.clone())),
            state_ty.clone(),
        ]),
        crate::types::FunctionSafety::Unsafe,
    );
    let expected_from = Type::function_with_safety(
        vec![
            Type::Pointer(Box::new(Type::Witness(witness))),
            state_ty.clone(),
        ],
        value.clone(),
        crate::types::FunctionSafety::Unsafe,
    );
    let (_, into_parts) = crate::infer::owner_coercion::select_operation(
        service,
        &constructor,
        protocol.trait_id,
        &state,
        protocol.into_parts_id,
        &expected_into,
    )?;
    let (_, from_parts) = crate::infer::owner_coercion::select_operation(
        service,
        &constructor,
        protocol.trait_id,
        &state,
        protocol.from_parts_id,
        &expected_from,
    )?;
    Ok(HirOpenOwner {
        state_ty: state_ty.clone(),
        into_parts,
        from_parts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program() -> AcceptedHirProgram {
        let path = std::env::temp_dir().join("rock_scoped_hir/main.rk");
        crate::analyze(&crate::Config {
            entry_file: path.clone(),
            no_std: true,
            no_prelude: true,
            source_providers: vec![crate::SourceProvider::Virtual {
                path,
                text: r#"
trait Rebuild
    @rebuild: Self
    @accept: &Self -> ()
    @same: &Self
exercise: &Rebuild -> ()
exercise = object !->
    open object as Hidden, value
        rebuilt = value.rebuild!
        value.accept &rebuilt
relay: &Rebuild -> &Rebuild
relay = object ->
    open object as Hidden, value
        value.same!
closure: &Rebuild -> ()
closure = object !->
    open object as Hidden, value
        callback = !-> value.accept value
        callback!
main = !-> ()
"#
                .into(),
            }],
            ..Default::default()
        })
        .unwrap()
        .hir
        .program
    }

    #[test]
    fn accepted_open_rejects_a_naked_shared_receiver_or_mutable_reborrow_forgery() {
        for mutable_forgery in [false, true] {
            let mut program = program();
            let function = program
                .program_mut_for_test()
                .functions
                .values_mut()
                .find(|function| function.name == "exercise")
                .unwrap();
            let HirStmtFor::Expr(expression) = &mut function.body.stmts[0] else {
                panic!("expected opening")
            };
            let HirExprKindFor::Open { body, .. } = &mut expression.kind else {
                panic!("expected opening")
            };
            let HirStmtFor::Let { value, .. } = &mut body.stmts[0] else {
                panic!("expected witness producer")
            };
            let HirExprKindFor::MethodCall(receiver, _, _, _, _) = &mut value.kind else {
                panic!("expected method call")
            };
            let HirExprKindFor::Ref(access, place) = &mut receiver.kind else {
                panic!("expected explicit reborrow")
            };
            if mutable_forgery {
                *access = true;
                let Type::Reference { mutable, .. } = &mut receiver.ty else {
                    panic!("expected reference")
                };
                *mutable = true;
            } else {
                *receiver = place.clone();
            }
            let errors =
                AcceptedHirProgram::revalidate_for_test(program.into_program()).unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains(if mutable_forgery {
                        "reference forges its declared access"
                    } else {
                        "exact scoped witness signature/access"
                    })),
                "{errors:?}"
            );
        }
    }

    #[test]
    fn accepted_open_rejects_witness_result_erasure_by_control_flow() {
        for mutation in 0..5 {
            let mut program = program();
            let function = program
                .program_mut_for_test()
                .functions
                .values_mut()
                .find(|function| function.name == "exercise")
                .unwrap();
            let HirStmtFor::Expr(expression) = &mut function.body.stmts[0] else {
                panic!("expected opening");
            };
            let HirExprKindFor::Open { body, .. } = &mut expression.kind else {
                panic!("expected opening");
            };
            let HirStmtFor::Let { ty, value, .. } = &mut body.stmts[0] else {
                panic!("expected witness local");
            };
            let producer = value.clone();
            let branch = HirBlockFor {
                ty: producer.ty.clone(),
                stmts: vec![HirStmtFor::Expr(producer.clone())],
            };
            let context = match mutation {
                0 => {
                    value.kind = HirExprKindFor::Block(branch);
                    "block expression"
                }
                1 => {
                    value.kind = HirExprKindFor::If {
                        condition: Box::new(HirExprFor {
                            ty: Type::Bool,
                            span: producer.span.clone(),
                            kind: HirExprKindFor::BoolLiteral(true),
                        }),
                        then_branch: branch.clone(),
                        else_branch: Some(branch),
                    };
                    "if branch"
                }
                2 => {
                    value.kind = HirExprKindFor::Match {
                        scrutinee: Box::new(HirExprFor {
                            ty: Type::Bool,
                            span: producer.span.clone(),
                            kind: HirExprKindFor::BoolLiteral(true),
                        }),
                        arms: vec![HirMatchArmFor {
                            pattern: HirPattern::Wildcard,
                            guard: None,
                            body: branch,
                        }],
                    };
                    "match arm"
                }
                3 => {
                    value.kind = HirExprKindFor::Loop(HirBlockFor {
                        ty: Type::Never,
                        stmts: vec![HirStmtFor::Break(Some(producer.clone()))],
                    });
                    "break target"
                }
                _ => "block tail",
            };
            if mutation != 4 {
                value.ty = Type::I64;
                *ty = Type::I64;
            } else {
                body.stmts.push(HirStmtFor::Expr(producer));
            }
            let errors =
                AcceptedHirProgram::revalidate_for_test(program.into_program()).unwrap_err();
            assert!(
                errors.iter().any(|error| error.contains(&format!(
                    "opened witness result disagrees with its {context}"
                ))),
                "{errors:?}"
            );
        }
    }

    #[test]
    fn accepted_open_rejects_erased_or_missing_witness_closure_captures() {
        for mutation in 0..3 {
            let mut program = program();
            let function = program
                .program_mut_for_test()
                .functions
                .values_mut()
                .find(|function| function.name == "closure")
                .unwrap();
            let HirStmtFor::Expr(expression) = &mut function.body.stmts[0] else {
                panic!("expected opening");
            };
            let HirExprKindFor::Open { body, .. } = &mut expression.kind else {
                panic!("expected opening");
            };
            let HirStmtFor::Let { ty, value, .. } = &mut body.stmts[0] else {
                panic!("expected closure local");
            };
            let HirExprKindFor::Lambda { captures, .. } = &mut value.kind else {
                panic!("expected closure");
            };
            let Type::Function {
                captures: signature_captures,
                ..
            } = &mut value.ty
            else {
                panic!("expected function type");
            };
            match mutation {
                0 => signature_captures.clear(),
                1 => {
                    captures[0].ty = Type::I64;
                    signature_captures[0].ty = Type::I64;
                }
                _ => {
                    captures.clear();
                    signature_captures.clear();
                }
            }
            *ty = value.ty.clone();
            let errors =
                AcceptedHirProgram::revalidate_for_test(program.into_program()).unwrap_err();
            let expected = match mutation {
                0 => "closure capture metadata",
                1 => "closure capture binding",
                _ => "uncaptured local",
            };
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "{errors:?}"
            );
        }
    }

    #[test]
    fn accepted_open_rejects_forged_owner_and_unbound_body_result() {
        let mut program = program();
        let function = program
            .program_mut_for_test()
            .functions
            .values_mut()
            .find(|function| function.name == "exercise")
            .unwrap();
        let HirStmtFor::Expr(expression) = &mut function.body.stmts[0] else {
            panic!("expected opening");
        };
        let HirExprKindFor::Open { binding, .. } = &mut expression.kind else {
            panic!("expected opening");
        };
        binding.witness.owner = DefId::new(
            binding.witness.owner.crate_id,
            crate::ids::LocalDefId(u32::MAX),
        );
        let errors = AcceptedHirProgram::revalidate_for_test(program.into_program()).unwrap_err();
        assert!(errors.iter().any(|error| error.contains("witness")));

        let mut program = self::program();
        let function = program
            .program_mut_for_test()
            .functions
            .values_mut()
            .find(|function| function.name == "exercise")
            .unwrap();
        let HirStmtFor::Expr(expression) = &mut function.body.stmts[0] else {
            panic!("expected opening");
        };
        let HirExprKindFor::Open { binding, body, .. } = &mut expression.kind else {
            panic!("expected opening");
        };
        body.ty = Type::Witness(binding.witness);
        let errors = AcceptedHirProgram::revalidate_for_test(program.into_program()).unwrap_err();
        assert!(errors.iter().any(|error| error.contains("opening result")));
    }

    #[test]
    fn accepted_open_rejects_reused_binder_identity_in_a_sibling_scope() {
        let mut program = program();
        let function = program
            .program_mut_for_test()
            .functions
            .values_mut()
            .find(|function| function.name == "exercise")
            .unwrap();
        function.body.stmts.push(function.body.stmts[0].clone());
        let errors = AcceptedHirProgram::revalidate_for_test(program.into_program()).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.contains("duplicate witness")));
    }

    #[test]
    fn accepted_open_rejects_forged_local_access_capability() {
        let mut program = program();
        let function = program
            .program_mut_for_test()
            .functions
            .values_mut()
            .find(|function| function.name == "exercise")
            .unwrap();
        let HirStmtFor::Expr(expression) = &mut function.body.stmts[0] else {
            panic!("expected opening");
        };
        let HirExprKindFor::Open { body, .. } = &mut expression.kind else {
            panic!("expected opening");
        };
        let HirStmtFor::Expr(expression) = body.stmts.last_mut().unwrap() else {
            panic!("expected method call");
        };
        let HirExprKindFor::MethodCall(receiver, _, _, _, _) = &mut expression.kind else {
            panic!("expected opened dispatch");
        };
        let Type::Reference { mutable, .. } = &mut receiver.ty else {
            panic!("expected shared reference");
        };
        *mutable = true;
        let errors = AcceptedHirProgram::revalidate_for_test(program.into_program()).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.contains("forges its declared access")));
    }

    #[test]
    fn accepted_reclosing_rejects_forged_dictionary_origin_or_witness() {
        for mutate_origin in [false, true] {
            let mut program = program();
            let function = program
                .program_mut_for_test()
                .functions
                .values_mut()
                .find(|function| function.name == "relay")
                .unwrap();
            let HirStmtFor::Expr(expression) = &mut function.body.stmts[0] else {
                panic!("expected opening");
            };
            let HirExprKindFor::Open { body, .. } = &mut expression.kind else {
                panic!("expected opening");
            };
            let HirStmtFor::Expr(expression) = body.stmts.last_mut().unwrap() else {
                panic!("expected reclosing");
            };
            let HirExprKindFor::ObjectCoercion(_, coercion) = &mut expression.kind else {
                panic!("expected object evidence");
            };
            let Some(HirObjectEvidence::Concrete { source, views }) = &mut coercion.evidence else {
                panic!("expected dictionary evidence");
            };
            if mutate_origin {
                views[0].origin = HirObjectWitnessOrigin::Callable {
                    kind: crate::types::CallableKind::Fn,
                };
            } else {
                let Type::Witness(witness) = source else {
                    panic!("expected witness");
                };
                witness.local = crate::ids::HirLocalId(u32::MAX);
            }
            let errors =
                AcceptedHirProgram::revalidate_for_test(program.into_program()).unwrap_err();
            assert!(errors
                .iter()
                .any(|error| error.contains("coercion evidence") || error.contains("dominating")));
        }
    }
}
