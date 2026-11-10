use super::*;

impl HirProgram {
    pub fn validate_method_authorities(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for function in self.functions.values() {
            validate_method_authorities_in_block(self, &function.body, &mut errors);
        }
        for imp in self.impls.values() {
            if let Err(error) = validate_impl_receiver_pattern_bindings(imp, &imp.receiver_pattern)
            {
                errors.push(error);
            }
            if let Some(trait_id) = imp.trait_id {
                let Some(trait_def) = self.traits.get(&trait_id) else {
                    errors.push(format!(
                        "implementation {:?} references unknown trait {trait_id:?}",
                        imp.id
                    ));
                    continue;
                };
                let member_ids = trait_def.methods.values().map(|method| method.id).chain(
                    trait_def
                        .signatures
                        .iter()
                        .filter(|(name, _)| !trait_def.methods.contains_key(*name))
                        .map(|(_, signature)| signature.id),
                );
                for member_id in member_ids {
                    let Some(method_id) = self.effective_trait_method(imp.id, member_id) else {
                        errors.push(format!(
                            "implementation {:?} has no effective body for trait member {member_id:?}",
                            imp.id
                        ));
                        continue;
                    };
                    if !imp.methods.values().any(|method| method.id == method_id) {
                        errors.push(format!(
                            "implementation {:?} maps trait member {member_id:?} to method {method_id:?} outside the impl",
                            imp.id
                        ));
                        continue;
                    }
                    if let Err(error) = validate_effective_trait_method_receiver_shape(
                        trait_def, imp, member_id, method_id,
                    ) {
                        errors.push(error);
                    }
                }
            }
            for function in imp.methods.values() {
                validate_method_authorities_in_block(self, &function.body, &mut errors);
            }
        }
        for (&(impl_id, member_id), &method_id) in &self.indexes.effective_trait_methods {
            let Some(imp) = self.impls.get(&impl_id) else {
                errors.push(format!(
                    "effective trait method relation references unknown impl {impl_id:?}"
                ));
                continue;
            };
            let Some(trait_id) = imp.trait_id else {
                errors.push(format!(
                    "effective trait method relation references inherent impl {impl_id:?}"
                ));
                continue;
            };
            let member_exists = self.traits.get(&trait_id).is_some_and(|trait_def| {
                trait_def
                    .methods
                    .values()
                    .any(|method| method.id == member_id)
                    || trait_def
                        .signatures
                        .values()
                        .any(|signature| signature.id == member_id)
            });
            if !member_exists {
                errors.push(format!(
                    "effective trait method relation references member {member_id:?} outside trait {trait_id:?}"
                ));
            }
            if !imp.methods.values().any(|method| method.id == method_id) {
                errors.push(format!(
                    "effective trait method relation references method {method_id:?} outside impl {impl_id:?}"
                ));
            }
        }
        for trait_def in self.traits.values() {
            for function in trait_def.methods.values() {
                validate_method_authorities_in_block(self, &function.body, &mut errors);
            }
        }
        errors
    }
}

fn validate_method_target_authority(
    program: &HirProgram,
    target: &HirMethodCallTarget,
    errors: &mut Vec<String>,
) {
    let mut seen_bindings = HashSet::new();
    for binding in target
        .owner_substitution
        .iter()
        .chain(target.method_substitution.iter())
    {
        if !seen_bindings.insert(binding.param) {
            errors.push(format!(
                "method authority contains duplicate generic binding {:?}",
                binding.param
            ));
        }
        if type_contains_backend_forbidden_sentinel(&binding.ty) {
            errors.push(format!(
                "method authority binding {:?} is unresolved: {:?}",
                binding.param, binding.ty
            ));
        }
    }
    if target
        .trait_args()
        .iter()
        .any(type_contains_backend_forbidden_sentinel)
    {
        errors.push("method authority contains an unresolved trait argument".to_string());
    }

    let expected_bindings = match &target.target {
        HirSelectedMethodTarget::ImplMethod {
            impl_id,
            method_id,
            selected_trait,
        } => {
            let Some(imp) = program.impls.get(impl_id) else {
                errors.push(format!(
                    "method authority references unknown impl {impl_id:?}"
                ));
                return;
            };
            if !imp.methods.values().any(|method| method.id == *method_id) {
                errors.push(format!(
                    "method authority references method {method_id:?} outside impl {impl_id:?}"
                ));
            }
            let owner_params = (0..imp.type_generics.len())
                .map(|index| GenericParamId {
                    owner: *impl_id,
                    index: index as u32,
                })
                .collect::<Vec<_>>();
            let method_params: Vec<GenericParamId> = imp
                .methods
                .values()
                .find(|method| method.id == *method_id)
                .map(|method| {
                    method
                        .generic_params
                        .iter()
                        .map(|param| param.id)
                        .filter(|param| {
                            !owner_params.contains(param)
                                && selected_trait
                                    .as_ref()
                                    .is_none_or(|selected| param.owner != selected.trait_id)
                        })
                        .collect()
                })
                .unwrap_or_default();
            if let Some(selected_trait) = selected_trait {
                if imp.trait_id != Some(selected_trait.trait_id) {
                    errors.push(format!(
                        "method authority impl {impl_id:?} does not implement trait {:?}",
                        selected_trait.trait_id
                    ));
                }
                if program.effective_trait_method(*impl_id, selected_trait.member_id)
                    != Some(*method_id)
                {
                    errors.push(format!(
                        "method authority has no effective mapping from member {:?} to method {method_id:?}",
                        selected_trait.member_id
                    ));
                }
                if let Some(trait_def) = program.traits.get(&selected_trait.trait_id) {
                    if selected_trait.trait_args.len() != trait_def.generic_params.len() {
                        errors.push(format!(
                            "method authority trait argument arity mismatch for {:?}: expected {}, found {}",
                            selected_trait.trait_id,
                            trait_def.generic_params.len(),
                            selected_trait.trait_args.len()
                        ));
                    }
                    let owner_substitution = target
                        .owner_substitution
                        .iter()
                        .map(|binding| (binding.param, binding.ty.clone()))
                        .collect::<HashMap<_, _>>();
                    let normalization_env = program.type_normalization_env();
                    let expected_trait_args = imp
                        .trait_arg_types
                        .iter()
                        .map(|arg| {
                            crate::type_services::normalize::TypeNormalizer::new(&normalization_env)
                                .normalize(&arg.substitute_generics(&owner_substitution))
                        })
                        .collect::<Result<Vec<_>, _>>();
                    let expected_trait_args = match expected_trait_args {
                        Ok(args) => args,
                        Err(error) => {
                            errors.push(format!(
                                "cannot normalize method authority trait arguments for impl {impl_id:?}: {error}"
                            ));
                            return;
                        }
                    };
                    if expected_trait_args != selected_trait.trait_args {
                        errors.push(format!(
                            "method authority trait arguments do not match impl {impl_id:?}: expected {expected_trait_args:?}, found {:?}",
                            selected_trait.trait_args
                        ));
                    }
                }
            } else if imp.trait_id.is_some() {
                errors.push(format!(
                    "trait impl method authority for impl {impl_id:?} has no selected trait identity"
                ));
            }
            Some((owner_params, method_params))
        }
        HirSelectedMethodTarget::TraitMethod {
            trait_id,
            member_id,
            ..
        } => {
            let Some(trait_def) = program.traits.get(trait_id) else {
                errors.push(format!(
                    "method authority references unknown trait {trait_id:?}"
                ));
                return;
            };
            if target.trait_args().len() != trait_def.generic_params.len() {
                errors.push(format!(
                    "method authority trait argument arity mismatch for {trait_id:?}: expected {}, found {}",
                    trait_def.generic_params.len(),
                    target.trait_args().len()
                ));
            }
            if !trait_def
                .methods
                .values()
                .any(|method| method.id == *member_id)
                && !trait_def
                    .signatures
                    .values()
                    .any(|signature| signature.id == *member_id)
            {
                errors.push(format!(
                    "method authority references unknown member {member_id:?} on trait {trait_id:?}"
                ));
            }
            let is_constructor_trait = trait_def.target.as_ref().is_some_and(|target| {
                !matches!(target.kind, crate::type_services::kind::Kind::Type)
            });
            let mut owner_params = if is_constructor_trait {
                trait_def
                    .generic_params
                    .iter()
                    .map(|param| param.id)
                    .chain(trait_def.target.iter().map(|target| target.id))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            owner_params
                .sort_by_key(|param| (param.owner.crate_id.0, param.owner.local.0, param.index));
            let method_params: Vec<GenericParamId> = trait_def
                .methods
                .values()
                .find(|method| method.id == *member_id)
                .map(|method| {
                    method
                        .generic_params
                        .iter()
                        .map(|param| param.id)
                        .filter(|param| {
                            if is_constructor_trait {
                                !owner_params.contains(param)
                            } else {
                                param.owner == method.id
                            }
                        })
                        .collect()
                })
                .or_else(|| {
                    trait_def
                        .signatures
                        .values()
                        .find(|signature| signature.id == *member_id)
                        .map(|signature| {
                            signature
                                .generic_params
                                .iter()
                                .map(|param| param.id)
                                .filter(|param| {
                                    if is_constructor_trait {
                                        !owner_params.contains(param)
                                    } else {
                                        param.owner == signature.id
                                    }
                                })
                                .collect()
                        })
                })
                .unwrap_or_default();
            Some((owner_params, method_params))
        }
    };

    if let Some((expected_owner, expected_method)) = expected_bindings {
        let actual_owner = target
            .owner_substitution
            .iter()
            .map(|binding| binding.param)
            .collect::<Vec<_>>();
        let actual_method = target
            .method_substitution
            .iter()
            .map(|binding| binding.param)
            .collect::<Vec<_>>();
        if actual_owner != expected_owner {
            errors.push(format!(
                "method authority {:?} owner bindings are incomplete or non-canonical: expected {expected_owner:?}, found {actual_owner:?}",
                target.target
            ));
        }
        if actual_method != expected_method {
            errors.push(format!(
                "method authority {:?} method bindings are incomplete or non-canonical: expected {expected_method:?}, found {actual_method:?}",
                target.target
            ));
        }
    }
}

fn validate_method_authorities_in_block(
    program: &HirProgram,
    block: &HirBlock,
    errors: &mut Vec<String>,
) {
    for stmt in &block.stmts {
        match stmt {
            HirStmtFor::Let { value, .. }
            | HirStmtFor::Expr(value)
            | HirStmtFor::Return(Some(value))
            | HirStmtFor::Break(Some(value)) => {
                validate_method_authorities_in_expr(program, value, errors)
            }
            HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => {}
        }
    }
}

fn validate_method_authorities_in_expr(
    program: &HirProgram,
    expr: &HirExpr,
    errors: &mut Vec<String>,
) {
    match &expr.kind {
        HirExprKindFor::MethodCall(receiver, _, args, self_receiver, target) => {
            validate_method_authorities_in_expr(program, receiver, errors);
            for arg in args {
                validate_method_authorities_in_expr(program, arg, errors);
            }
            match target {
                Some(target) => {
                    validate_method_target_authority(program, target, errors);
                    let expected = method_target_self_receiver(program, target);
                    if expected == Some(None) {
                        errors.push("method-call authority references a static method".to_string());
                    }
                    if expected != Some(*self_receiver) {
                        errors.push(format!(
                            "method authority receiver mode mismatch: expected {expected:?}, found {self_receiver:?}"
                        ));
                    }
                }
                None => {
                    errors.push("accepted HIR method call has no selected authority".to_string())
                }
            }
        }
        HirExprKindFor::Call(callee, args, target) => {
            let callee_is_authorized_method = matches!(
                (&callee.kind, target),
                (
                    HirExprKindFor::ResolvedVar(HirVarRef {
                        target: HirVarTarget::Function(callee_id),
                        ..
                    }),
                    Some(HirCallTarget::StaticMethod(target))
                ) if target.method.method_id() == Some(*callee_id)
            );
            if !callee_is_authorized_method {
                validate_method_authorities_in_expr(program, callee, errors);
            }
            for arg in args {
                validate_method_authorities_in_expr(program, arg, errors);
            }
            if let Some(HirCallTarget::StaticMethod(target)) = target {
                validate_method_target_authority(program, &target.method, errors);
                if method_target_self_receiver(program, &target.method) != Some(None) {
                    errors.push("static method authority references a receiver method".to_string());
                }
                if type_contains_backend_forbidden_sentinel(&target.owner_ty) {
                    errors.push("static method authority has an unresolved owner type".to_string());
                }
            }
            if let Some(HirCallTarget::Function(id)) = target {
                if program.indexes.methods_by_id.contains_key(id) {
                    errors.push(format!(
                        "accepted HIR static method call {id:?} ({:?}) has no selected static-method authority",
                        callee.kind
                    ));
                }
            }
            if matches!(target, Some(HirCallTarget::Instance(_))) {
                errors.push(
                    "accepted HIR contains a pre-monomorphization instance call target".to_string(),
                );
            }
            if target.is_none() && matches!(&callee.kind, HirExprKindFor::FieldAccess(_, _, None)) {
                errors.push("accepted HIR contains an unresolved field method call".to_string());
            }
        }
        HirExprKindFor::Try {
            expr,
            branch_method,
            branch_target,
            branch_self_receiver,
            from_residual_target,
            ..
        } => {
            validate_method_authorities_in_expr(program, expr, errors);
            match branch_method {
                Some(target) => {
                    validate_method_target_authority(program, target, errors);
                    let expected = method_target_self_receiver(program, target);
                    if expected != Some(*branch_self_receiver) {
                        errors.push(format!(
                            "Try branch receiver mode mismatch: expected {expected:?}, found {branch_self_receiver:?}"
                        ));
                    }
                }
                None => errors.push("accepted Try expression has no branch authority".to_string()),
            }
            if branch_target.is_some() {
                errors.push(
                    "accepted Try expression contains a pre-monomorphization instance target"
                        .to_string(),
                );
            }
            match from_residual_target {
                Some(HirCallTarget::StaticMethod(target)) => {
                    validate_method_target_authority(program, &target.method, errors);
                    if type_contains_backend_forbidden_sentinel(&target.owner_ty) {
                        errors.push(
                            "Try from-residual authority has an unresolved owner type".to_string(),
                        );
                    }
                }
                Some(_) => errors.push(
                    "accepted Try expression has a non-static from-residual authority".to_string(),
                ),
                None => errors
                    .push("accepted Try expression has no from-residual authority".to_string()),
            }
        }
        HirExprKindFor::FieldAccess(base, field_name, location) => {
            validate_method_authorities_in_expr(program, base, errors);
            if location.is_none() {
                errors.push(format!(
                    "accepted HIR contains unresolved field access '{field_name}'"
                ));
            }
        }
        HirExprKindFor::ArrayLiteral(values) | HirExprKindFor::TupleLiteral(values) => {
            for value in values {
                validate_method_authorities_in_expr(program, value, errors);
            }
        }
        HirExprKindFor::ArrayRepeat(value, _) => {
            validate_method_authorities_in_expr(program, value, errors);
        }
        HirExprKindFor::StructLiteral(_, _, fields) => {
            for field in fields {
                validate_method_authorities_in_expr(program, &field.value, errors);
            }
        }
        HirExprKindFor::EnumVariant(_, _, args, _) | HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                validate_method_authorities_in_expr(program, arg, errors);
            }
        }
        HirExprKindFor::TupleIndex(inner, _)
        | HirExprKindFor::UnaryOp(_, inner)
        | HirExprKindFor::Ref(_, inner)
        | HirExprKindFor::Deref(inner)
        | HirExprKindFor::Cast(inner, _) => {
            validate_method_authorities_in_expr(program, inner, errors)
        }
        HirExprKindFor::BinOp(_, left, right) | HirExprKindFor::Assign(left, right) => {
            validate_method_authorities_in_expr(program, left, errors);
            validate_method_authorities_in_expr(program, right, errors);
        }
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            validate_method_authorities_in_expr(program, condition, errors);
            validate_method_authorities_in_block(program, then_branch, errors);
            if let Some(else_branch) = else_branch {
                validate_method_authorities_in_block(program, else_branch, errors);
            }
        }
        HirExprKindFor::Match { scrutinee, arms } => {
            validate_method_authorities_in_expr(program, scrutinee, errors);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    validate_method_authorities_in_expr(program, guard, errors);
                }
                validate_method_authorities_in_block(program, &arm.body, errors);
            }
        }
        HirExprKindFor::While { condition, body } => {
            validate_method_authorities_in_expr(program, condition, errors);
            validate_method_authorities_in_block(program, body, errors);
        }
        HirExprKindFor::For { iter, body, .. } => {
            validate_method_authorities_in_expr(program, iter, errors);
            validate_method_authorities_in_block(program, body, errors);
        }
        HirExprKindFor::Lambda { body, .. }
        | HirExprKindFor::Block(body)
        | HirExprKindFor::Loop(body)
        | HirExprKindFor::UnsafeBlock(body) => {
            validate_method_authorities_in_block(program, body, errors)
        }
        HirExprKindFor::IntLiteral(_) | HirExprKindFor::ConstParam(_)
        | HirExprKindFor::FloatLiteral(_)
        | HirExprKindFor::BoolLiteral(_)
        | HirExprKindFor::StringLiteral(_)
        | HirExprKindFor::CharLiteral(_)
        | HirExprKindFor::Unit
        | HirExprKindFor::Var(_) => {}
        HirExprKindFor::ResolvedVar(reference) => {
            if matches!(reference.target, HirVarTarget::Instance(_)) {
                errors.push(
                    "accepted HIR contains a pre-monomorphization instance variable target"
                        .to_string(),
                );
            }
            if matches!(reference.target, HirVarTarget::Function(id) if program.indexes.methods_by_id.contains_key(&id))
            {
                errors.push(format!(
                    "accepted HIR callable retains raw method DefId {:?}",
                    reference.target
                ));
            }
        }
    }
}

fn method_target_self_receiver(
    program: &HirProgram,
    target: &HirMethodCallTarget,
) -> Option<Option<ReceiverMode>> {
    match target.target {
        HirSelectedMethodTarget::ImplMethod {
            impl_id, method_id, ..
        } => program.impls.get(&impl_id).and_then(|imp| {
            imp.methods
                .values()
                .find(|method| method.id == method_id)
                .map(|method| method.self_receiver)
        }),
        HirSelectedMethodTarget::TraitMethod {
            trait_id,
            member_id,
            ..
        } => program.traits.get(&trait_id).and_then(|trait_def| {
            trait_def
                .methods
                .values()
                .find(|method| method.id == member_id)
                .map(|method| method.self_receiver)
                .or_else(|| {
                    trait_def
                        .signatures
                        .values()
                        .find(|signature| signature.id == member_id)
                        .map(|signature| signature.self_receiver)
                })
        }),
    }
}
