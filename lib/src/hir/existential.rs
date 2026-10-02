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
        loops: Vec::new(),
        returns: Vec::new(),
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
    active: HashMap<WitnessId, &'a HirOpenBinding>,
    seen_openings: HashSet<WitnessId>,
    loops: Vec<Vec<WitnessId>>,
    returns: Vec<WitnessId>,
    errors: &'a mut Vec<String>,
    phase: std::marker::PhantomData<P>,
}

impl<'a, P: HirPhase> Scope<'a, P> {
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
        for statement in &block.stmts {
            match statement {
                HirStmt::Let { ty, value, .. } => {
                    self.ty(ty);
                    self.expr(value);
                }
                HirStmt::Expr(value) => self.expr(value),
                HirStmt::Return(Some(value)) => {
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
                    let allowed = self.loops.last().cloned().unwrap_or_default();
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
                self.active.insert(binding.witness, binding);
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
                        if let Some(binding) = self.active.get(witness).copied() {
                            let receiver_ok = (*mode == Some(ReceiverMode::Move)
                                && receiver.ty == Type::Witness(*witness))
                                || matches!(&receiver.ty, Type::Reference { mutable, inner } if inner.as_ref() == &Type::Witness(*witness) && (*mode == Some(ReceiverMode::Shared) || (*mode == Some(ReceiverMode::Mut) && *mutable)));
                            let opened = binding.object.try_map_types(|ty| {
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
                            let signature_ok = object_methods::opened_signature(
                                &self.program.traits,
                                target,
                                &binding.object,
                                *witness,
                            )
                            .is_ok_and(|(params, ret)| {
                                ret == expression.ty
                                    && params.len() == args.len()
                                    && params.iter().zip(args).all(|(param, arg)| param == &arg.ty)
                            });
                            if !receiver_ok || !bound_ok || !signature_ok {
                                self.errors.push("opened call does not match its exact scoped witness signature/access".into());
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
                    if self.active.get(witness).is_none_or(|binding| {
                        call.object != Type::Object(Box::new(binding.object.clone()))
                    }) {
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
                            .and_then(|binding| {
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
                                    &binding.object,
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
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expr(condition);
                self.block(then_branch);
                if let Some(body) = else_branch {
                    self.block(body);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for arm in arms {
                    self.pattern(&arm.pattern);
                    if let Some(guard) = &arm.guard {
                        self.expr(guard);
                    }
                    self.block(&arm.body);
                }
            }
            HirExprKind::While { condition, body } => {
                self.expr(condition);
                self.loop_body(body);
            }
            HirExprKind::For { iter, body, .. } => {
                self.expr(iter);
                self.loop_body(body);
            }
            HirExprKind::Loop(body) => self.loop_body(body),
            HirExprKind::Block(body) | HirExprKind::UnsafeBlock(body) => self.block(body),
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
                }
                let loops = std::mem::take(&mut self.loops);
                let returns =
                    std::mem::replace(&mut self.returns, self.active.keys().copied().collect());
                self.block(body);
                self.loops = loops;
                self.returns = returns;
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
            HirExprKind::ArrayLiteral(values)
            | HirExprKind::TupleLiteral(values)
            | HirExprKind::EnumVariant(_, _, values, _)
            | HirExprKind::Intrinsic { args: values, .. } => {
                for value in values {
                    self.expr(value);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    self.expr(&field.value);
                }
            }
            HirExprKind::ArrayRepeat(value, _)
            | HirExprKind::FieldAccess(value, _, _)
            | HirExprKind::TupleIndex(value, _)
            | HirExprKind::UnaryOp(_, value)
            | HirExprKind::Ref(_, value)
            | HirExprKind::Deref(value) => self.expr(value),
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                self.expr(left);
                self.expr(right);
            }
            _ => {}
        }
    }

    fn loop_body(&mut self, body: &'a HirBlockFor<P>) {
        self.loops.push(self.active.keys().copied().collect());
        self.block(body);
        self.loops.pop();
    }
}

fn validate_owner(program: &HirProgram, binding: &HirOpenBinding) -> Result<(), String> {
    let Some(owner) = &binding.owner else {
        return Ok(());
    };
    let Type::Struct { id, args } = &binding.source_ty else {
        return Err("opened owner requires nominal source".into());
    };
    let protocol = program
        .language_items
        .object_owner
        .as_ref()
        .ok_or("opened owner has no protocol provider")?;
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
    let constructor = Type::Constructor {
        id: *id,
        flavor: crate::types::NominalTypeKind::Struct,
    };
    let state = service
        .infer_trait_arguments(&constructor, protocol.trait_id)
        .ok_or("opened owner lacks unique constructor evidence")?;
    if state.as_slice() != std::slice::from_ref(&owner.state_ty)
        || crate::type_services::visit::type_any(&owner.state_ty, |ty| {
            matches!(
                ty,
                Type::Witness(_) | Type::Generic(_) | Type::TypeVar(_) | Type::ObjectSelf { .. }
            )
        })
        || crate::type_services::layout::TypeLayout::sizedness(&owner.state_ty, &|_| {
            crate::type_services::layout::Sizedness::Unknown
        }) != crate::type_services::layout::Sizedness::Sized
    {
        return Err("opened owner has forged payload-dependent State".into());
    }
    let [payload] = args.as_slice() else {
        return Err("opened owner must be unary".into());
    };
    let expected_into = Type::function_with_safety(
        vec![binding.source_ty.clone()],
        Type::Tuple(vec![
            Type::Pointer(Box::new(payload.clone())),
            owner.state_ty.clone(),
        ]),
        crate::types::FunctionSafety::Unsafe,
    );
    let expected_from = Type::function_with_safety(
        vec![
            Type::Pointer(Box::new(Type::Witness(binding.witness))),
            owner.state_ty.clone(),
        ],
        binding.value.ty.clone(),
        crate::types::FunctionSafety::Unsafe,
    );
    for (actual, member, signature) in [
        (&owner.into_parts, protocol.into_parts_id, expected_into),
        (&owner.from_parts, protocol.from_parts_id, expected_from),
    ] {
        let (_, expected) = crate::infer::owner_coercion::select_operation(
            &service,
            &constructor,
            protocol.trait_id,
            &state,
            member,
            &signature,
        )?;
        if actual != &expected {
            return Err(
                "opened owner split/rebuild authority disagrees with canonical protocol".into(),
            );
        }
    }
    Ok(())
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
exercise: &Rebuild -> ()
exercise = object !->
    open object as Hidden, value
        rebuilt = value.rebuild!
        value.accept &rebuilt
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
}
