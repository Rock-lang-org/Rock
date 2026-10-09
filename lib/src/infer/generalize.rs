//! Function generalization: converting type variables to generic parameters.

use std::collections::{HashMap, HashSet};

use crate::hir::*;
use crate::ids::{DefId, TypeVarId};
use crate::types::{GenericParamDecl, GenericParamId, Type};

use super::type_vars::{
    replace_type_vars_in_block_composite, replace_type_vars_with_generics_composite,
};
use super::{ConstraintStore, InferenceEngine, PartialHir};

fn collect_type_vars_in_order(ty: &Type, out: &mut Vec<TypeVarId>) {
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        if let Type::TypeVar(id) = nested {
            if !out.contains(id) {
                out.push(*id);
            }
        }
    });
}

fn callable_input_local(expression: &HirExpr) -> Option<crate::ids::HirLocalId> {
    match &expression.kind {
        HirExprKind::ResolvedVar(HirVarRef {
            target: HirVarTarget::Local(local),
            ..
        }) => Some(*local),
        _ => None,
    }
}

fn escaping_callable_requirement(
    engine: &InferenceEngine,
    parameter: &HirParam,
    argument_count: usize,
) -> Option<Type> {
    use crate::types::{CallableKind, CallableResultBorrowing};

    if parameter.is_ref || parameter.mutable {
        return None;
    }
    let mut required = engine.resolve(&parameter.ty);
    let Type::Function {
        params,
        callable_kind,
        result_borrowing,
        ..
    } = &mut required
    else {
        return None;
    };
    if *result_borrowing != CallableResultBorrowing::Unknown || params.len() != argument_count {
        return None;
    }
    // Consuming the input excludes its own storage, not dependencies carried by
    // the input or arguments. This is a demand, never a producer/effect proof.
    *result_borrowing = crate::type_services::callable::owned_result_requirement(params)?;
    *callable_kind = CallableKind::FnOnce;
    Some(required)
}

fn infer_direct_callable_result_demand(engine: &InferenceEngine, function: &mut HirFunction) {
    use crate::ids::HirLocalId;

    struct Demands<'a> {
        engine: &'a InferenceEngine,
        function: &'a HirFunction,
        requirements: HashMap<HirLocalId, Type>,
        escaped: HashSet<HirLocalId>,
        aliases: HashMap<HirLocalId, HirLocalId>,
    }
    impl Demands<'_> {
        fn input(&self, local: HirLocalId) -> Option<HirLocalId> {
            let input = *self.aliases.get(&local).unwrap_or(&local);
            self.function
                .params
                .iter()
                .find(|parameter| {
                    parameter.local_id == input
                        && matches!(self.engine.resolve(&parameter.ty), Type::Function { .. })
                })
                .map(|parameter| parameter.local_id)
        }

        fn collect_block(&mut self, block: &HirBlock, repeated: bool) -> Option<()> {
            for statement in &block.stmts {
                match statement {
                    HirStmt::Let {
                        local_id,
                        value,
                        mutable: false,
                        ..
                    } if callable_input_local(value)
                        .and_then(|local| self.input(local))
                        .is_some() =>
                    {
                        let input = self.input(callable_input_local(value)?)?;
                        // IDs identify declarations, not names. A second definition
                        // cannot silently replace the first alias's authority.
                        if self.aliases.insert(*local_id, input).is_some()
                            || self
                                .function
                                .params
                                .iter()
                                .any(|param| param.local_id == *local_id)
                        {
                            return None;
                        }
                    }
                    HirStmt::Let { value, .. }
                    | HirStmt::Expr(value)
                    | HirStmt::Return(Some(value))
                    | HirStmt::Break(Some(value)) => {
                        self.collect(value, repeated)?;
                    }
                    _ => {}
                }
            }
            Some(())
        }

        fn collect(&mut self, expression: &HirExpr, repeated: bool) -> Option<()> {
            match &expression.kind {
                HirExprKind::Call(callee, args, _)
                    if callable_input_local(callee)
                        .and_then(|local| self.input(local))
                        .is_some() =>
                {
                    if repeated {
                        return None;
                    }
                    let input = self.input(callable_input_local(callee)?)?;
                    let parameter = self
                        .function
                        .params
                        .iter()
                        .find(|parameter| parameter.local_id == input)?;
                    // This is only a consuming input requirement. Repeated or
                    // escaping uses need a stronger definition-owned protocol.
                    if self
                        .requirements
                        .insert(
                            input,
                            escaping_callable_requirement(self.engine, parameter, args.len())?,
                        )
                        .is_some()
                    {
                        return None;
                    }
                    for argument in args {
                        self.collect(argument, repeated)?;
                    }
                }
                HirExprKind::ResolvedVar(_) => {
                    if let Some(input) =
                        callable_input_local(expression).and_then(|local| self.input(local))
                    {
                        self.escaped.insert(input);
                    }
                }
                HirExprKind::Lambda { captures, .. } => {
                    // A deferred body can be invoked more than once. Captures
                    // veto consumption demands; do not traverse recycled IDs.
                    for capture in captures {
                        if let Some(input) = self.input(capture.local_id) {
                            self.escaped.insert(input);
                        }
                    }
                }
                _ => {
                    let repeated = repeated
                        || matches!(
                            expression.kind,
                            HirExprKind::While { .. }
                                | HirExprKind::For { .. }
                                | HirExprKind::Loop(_)
                                | HirExprKind::ArrayRepeat(_, _)
                        );
                    let mut valid = Some(());
                    crate::hir::visit::visit_expr_children(expression, &mut |child| {
                        if valid.is_some() {
                            valid = match child {
                                crate::hir::visit::HirChild::Expr(child) => {
                                    self.collect(child, repeated)
                                }
                                crate::hir::visit::HirChild::Block(block) => {
                                    self.collect_block(block, repeated)
                                }
                            };
                        }
                    });
                    valid?;
                }
            }
            Some(())
        }
    }
    fn rewrite(
        expression: &mut HirExpr,
        requirements: &HashMap<HirLocalId, Type>,
        aliases: &HashMap<HirLocalId, HirLocalId>,
    ) {
        if let HirExprKind::Call(callee, _, _) = &mut expression.kind {
            if let Some(required) = callable_input_local(callee)
                .and_then(|id| requirements.get(aliases.get(&id).unwrap_or(&id)))
            {
                callee.ty = required.clone();
            }
        }
        // A captured body owns different callable-input contracts.
        if matches!(expression.kind, HirExprKind::Lambda { .. }) {
            return;
        }
        crate::hir::visit::visit_expr_children_mut(expression, &mut |child| match child {
            crate::hir::visit::HirChildMut::Expr(child) => rewrite(child, requirements, aliases),
            crate::hir::visit::HirChildMut::Block(block) => {
                rewrite_block(block, requirements, aliases)
            }
        });
    }

    fn rewrite_block(
        block: &mut HirBlock,
        requirements: &HashMap<HirLocalId, Type>,
        aliases: &HashMap<HirLocalId, HirLocalId>,
    ) {
        for statement in &mut block.stmts {
            let expression = match statement {
                HirStmt::Let {
                    local_id,
                    ty,
                    value,
                    ..
                } => {
                    if let Some(required) = aliases
                        .get(local_id)
                        .and_then(|input| requirements.get(input))
                    {
                        *ty = required.clone();
                        value.ty = required.clone();
                    }
                    value
                }
                HirStmt::Expr(expression)
                | HirStmt::Return(Some(expression))
                | HirStmt::Break(Some(expression)) => expression,
                _ => continue,
            };
            rewrite(expression, requirements, aliases);
        }
    }

    let mut demands = Demands {
        engine,
        function,
        requirements: HashMap::new(),
        escaped: HashSet::new(),
        aliases: HashMap::new(),
    };
    if demands.collect_block(&function.body, false).is_none()
        || demands.requirements.is_empty()
        || demands
            .escaped
            .iter()
            .any(|input| demands.requirements.contains_key(input))
    {
        return;
    }
    // Commit only after the whole definition tree agrees; a failed branch
    // cannot leave a partial obligation on a declaration or inference variable.
    let requirements = demands.requirements;
    let aliases = demands.aliases;
    for parameter in &mut function.params {
        if let Some(required) = requirements.get(&parameter.local_id) {
            parameter.ty = required.clone();
        }
    }
    rewrite_block(&mut function.body, &requirements, &aliases);
}

/// Generalize all functions by converting unconstrained type variables to generic parameters.
pub(super) fn generalize_methods_only(hir: &mut PartialHir) {
    generalize_methods(hir);
}

pub(super) fn generalize_functions(
    hir: &mut PartialHir,
    function_ids: &[DefId],
    root_entrypoint_id: Option<DefId>,
) {
    for id in function_ids.iter().copied() {
        if root_entrypoint_id == Some(id) {
            continue;
        }
        if let Some(func) = hir.functions.get_mut(&id) {
            if !func.generic_params.is_empty() {
                infer_direct_callable_result_demand(&hir.engine, func);
                continue;
            }
        }

        if let Some(func) = hir.functions.remove(&id) {
            let generalized = generalize_single_function(
                &hir.engine,
                &hir.constraint_store,
                &hir.function_type_vars,
                func,
            );
            hir.functions.insert(id, generalized);
        }
    }
}

fn generalize_methods(hir: &mut PartialHir) {
    let mut trait_methods_to_generalize = Vec::new();
    let mut trait_ids = hir.traits.keys().copied().collect::<Vec<_>>();
    trait_ids.sort();
    for id in &trait_ids {
        let trait_def = &hir.traits[id];
        let mut methods = trait_def
            .methods
            .iter()
            .map(|(name, function)| (function.id, name))
            .collect::<Vec<_>>();
        methods.sort_by_key(|(method_id, _)| *method_id);
        for (_, method_name) in methods {
            let function = &trait_def.methods[method_name];
            if function.generic_params.is_empty() {
                trait_methods_to_generalize.push((*id, method_name.clone(), function.clone()));
            }
        }
    }
    for (trait_id, method_name, function) in trait_methods_to_generalize {
        let generalized = generalize_single_function(
            &hir.engine,
            &hir.constraint_store,
            &hir.function_type_vars,
            function,
        );
        if let Some(trait_def) = hir.traits.get_mut(&trait_id) {
            trait_def.methods.insert(method_name, generalized);
        }
    }

    // Also generalize impl methods
    let mut impl_methods_to_generalize: Vec<(DefId, String, HirFunction)> = Vec::new();
    let mut impl_ids: Vec<DefId> = hir.impls.keys().copied().collect();
    impl_ids.sort();
    for id in &impl_ids {
        let imp = &hir.impls[id];
        let mut methods = imp
            .methods
            .iter()
            .map(|(name, function)| (function.id, name))
            .collect::<Vec<_>>();
        methods.sort_by_key(|(method_id, _)| *method_id);
        for (_, method_name) in methods {
            let func = &imp.methods[method_name];
            impl_methods_to_generalize.push((*id, method_name.clone(), func.clone()));
        }
    }

    for (impl_id, method_name, func) in impl_methods_to_generalize {
        let generalized = generalize_single_function(
            &hir.engine,
            &hir.constraint_store,
            &hir.function_type_vars,
            func,
        );
        if let Some(imp) = hir.impls.get_mut(&impl_id) {
            imp.methods.insert(method_name, generalized);
        }
    }
}

/// Generalize a single function by converting type variables to generic parameters.
///
/// This is the canonical implementation — both the finalization pass and the
/// per-body lowering pass delegate here.
pub fn generalize_single_function(
    engine: &InferenceEngine,
    constraints: &ConstraintStore,
    function_type_vars: &HashMap<DefId, HashSet<TypeVarId>>,
    func: HirFunction,
) -> HirFunction {
    generalize_single_function_with_exclusions(
        engine,
        constraints,
        function_type_vars,
        &HashSet::new(),
        func,
    )
}

/// Generalize a function while preserving inference variables that belong to
/// an authority which must be materialized after body lowering.
pub fn generalize_single_function_with_exclusions(
    engine: &InferenceEngine,
    constraints: &ConstraintStore,
    function_type_vars: &HashMap<DefId, HashSet<TypeVarId>>,
    excluded: &HashSet<TypeVarId>,
    mut func: HirFunction,
) -> HirFunction {
    let func_header_vars = function_type_vars
        .get(&func.id)
        .cloned()
        .unwrap_or_default();

    if !func_header_vars.is_empty() || !func.generic_params.is_empty() {
        infer_direct_callable_result_demand(engine, &mut func);
    }
    if func_header_vars.is_empty() {
        return func;
    }

    // Inferred and explicit generic declarations acquire demands from their
    // own definitions; fully closed Unknown input declarations stay opaque.

    if func.body.stmts.is_empty() && matches!(func.body.ty, Type::Unit) {
        return func;
    }

    let mut resolved_signature_type_vars = Vec::new();
    for param in &func.params {
        collect_type_vars_in_order(
            &engine.resolve(&param.ty),
            &mut resolved_signature_type_vars,
        );
    }
    collect_type_vars_in_order(
        &engine.resolve(&func.ret_type),
        &mut resolved_signature_type_vars,
    );
    // Callable outputs can occur only in a parameter's bound (for example an
    // ignored callback result). They still belong to the polymorphic scheme.
    let mut index = 0;
    while index < resolved_signature_type_vars.len() {
        for bound in engine.get_bounds(resolved_signature_type_vars[index]) {
            for argument in bound.type_args {
                collect_type_vars_in_order(
                    &engine.resolve(&argument),
                    &mut resolved_signature_type_vars,
                );
            }
        }
        index += 1;
    }
    resolved_signature_type_vars.retain(|representative| {
        !is_excluded_representative(engine, excluded, *representative)
            && !constraints.has_literal_evidence_for_representative(*representative, |var| {
                match engine.resolve(&Type::TypeVar(var)) {
                    Type::TypeVar(representative) => Some(representative),
                    _ => None,
                }
            })
    });

    if resolved_signature_type_vars.is_empty() {
        return func;
    }

    let ordered_representatives = resolved_signature_type_vars;
    let mut composite_types = HashMap::new();
    let mut header_aliases = HashMap::new();
    for var_id in &func_header_vars {
        match engine.resolve(&Type::TypeVar(*var_id)) {
            Type::TypeVar(representative) => {
                header_aliases.insert(*var_id, representative);
            }
            resolved => {
                let mut nested = Vec::new();
                collect_type_vars_in_order(&resolved, &mut nested);
                if !nested.is_empty() {
                    composite_types.insert(*var_id, resolved);
                }
            }
        }
    }

    let mut rep_to_generic: HashMap<TypeVarId, GenericParamId> = HashMap::new();
    let generic_names = ["T", "U", "V", "W", "X", "Y", "Z"];
    // A body may already mention rigid impl parameters (for example `*T`)
    // while another signature slot remains inferred. Preserve those owners and
    // append only the remaining signature representatives after constraints.
    let first_index = func
        .generic_params
        .iter()
        .filter(|param| param.id.owner == func.id)
        .map(|param| param.id.index + 1)
        .max()
        .unwrap_or(0);
    let mut used_names = func
        .generic_params
        .iter()
        .map(|param| param.name.clone())
        .collect::<HashSet<_>>();
    let mut name_index = 0;

    let generic_param_names: Vec<String> = ordered_representatives
        .iter()
        .map(|_| loop {
            let name = generic_names
                .get(name_index)
                .map(|name| (*name).to_string())
                .unwrap_or_else(|| format!("T{name_index}"));
            name_index += 1;
            if used_names.insert(name.clone()) {
                break name;
            }
        })
        .collect();

    for (index, rep_id) in ordered_representatives.iter().enumerate() {
        rep_to_generic.insert(
            *rep_id,
            GenericParamId {
                owner: func.id,
                index: first_index + index as u32,
            },
        );
    }

    let inferred_params = generic_param_names
        .into_iter()
        .zip(&ordered_representatives)
        .enumerate()
        .map(|(index, (name, representative))| {
            GenericParamDecl::new(
                GenericParamId {
                    owner: func.id,
                    index: first_index + index as u32,
                },
                name,
                engine.kind_of_type_var(*representative),
            )
        })
        .collect::<Vec<_>>();
    func.generic_params.extend(inferred_params);

    let mut var_to_generic = rep_to_generic.clone();
    for (alias, representative) in header_aliases {
        if let Some(generic_id) = rep_to_generic.get(&representative) {
            var_to_generic.insert(alias, *generic_id);
        }
    }

    for (representative, generic_id) in &rep_to_generic {
        let mut inferred_bounds = engine.get_bounds(*representative);
        for header_var in &func_header_vars {
            if engine.resolve(&Type::TypeVar(*header_var)) == Type::TypeVar(*representative) {
                inferred_bounds.extend(engine.get_bounds(*header_var));
            }
        }
        let destination = func.generic_bounds.entry(*generic_id).or_default();
        for mut bound in inferred_bounds {
            for arg in &mut bound.type_args {
                *arg = replace_type_vars_with_generics_composite(
                    engine,
                    arg,
                    &var_to_generic,
                    &composite_types,
                );
            }
            if !destination.contains(&bound) {
                destination.push(bound);
            }
        }
    }

    for param in &mut func.params {
        param.ty = replace_type_vars_with_generics_composite(
            engine,
            &param.ty,
            &var_to_generic,
            &composite_types,
        );
    }
    func.ret_type = replace_type_vars_with_generics_composite(
        engine,
        &func.ret_type,
        &var_to_generic,
        &composite_types,
    );
    for bounds in func.generic_bounds.values_mut() {
        for bound in bounds {
            for ty in &mut bound.type_args {
                *ty = replace_type_vars_with_generics_composite(
                    engine,
                    ty,
                    &var_to_generic,
                    &composite_types,
                );
            }
        }
    }
    for predicate in &mut func.generic_bounds.predicates {
        match predicate {
            crate::types::Predicate::Trait { subject, args, .. } => {
                *subject = replace_type_vars_with_generics_composite(
                    engine,
                    subject,
                    &var_to_generic,
                    &composite_types,
                );
                for arg in args {
                    *arg = replace_type_vars_with_generics_composite(
                        engine,
                        arg,
                        &var_to_generic,
                        &composite_types,
                    );
                }
            }
        }
    }
    replace_type_vars_in_block_composite(engine, &mut func.body, &var_to_generic, &composite_types);

    func
}

fn is_excluded_representative(
    engine: &InferenceEngine,
    excluded: &HashSet<TypeVarId>,
    representative: TypeVarId,
) -> bool {
    let representative = engine.resolve(&Type::TypeVar(representative));
    excluded
        .iter()
        .any(|var| engine.resolve(&Type::TypeVar(*var)) == representative)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::hir::{HirBlock, HirExpr, HirExprKind, HirParam, HirStmt};
    use crate::ids::{CrateId, DefId, LocalDefId, TypeVarId};

    fn def_id(local: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(local))
    }

    fn invoking_input(
        engine: &mut InferenceEngine,
    ) -> (HirFunction, HashMap<DefId, HashSet<TypeVarId>>) {
        let Type::TypeVar(result) = engine.fresh_type_var() else {
            unreachable!()
        };
        let mut function = function_with_type_vars(def_id(90), &[result]);
        let callable = Type::function(vec![], Type::TypeVar(result));
        function.params[0].ty = callable.clone();
        let callee = HirExpr {
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: function.params[0].name.clone(),
                target: HirVarTarget::Local(function.params[0].local_id),
            }),
            ty: callable,
            span: crate::lexer::Span::test(),
        };
        function.body.stmts = vec![HirStmt::Expr(HirExpr {
            kind: HirExprKind::Call(Box::new(callee), vec![], None),
            ty: Type::TypeVar(result),
            span: crate::lexer::Span::test(),
        })];
        let headers = HashMap::from([(function.id, HashSet::from([result]))]);
        (function, headers)
    }

    #[test]
    fn definition_callable_demand_precedes_result_generalization_and_keeps_dependencies() {
        use crate::types::{CallableResultBorrowing as B, CallableResultSource as S};
        let mut engine = InferenceEngine::new();
        let (function, headers) = invoking_input(&mut engine);
        let generalized =
            generalize_single_function(&engine, &ConstraintStore::new(), &headers, function);
        let Type::Function {
            params,
            ret,
            result_borrowing,
            ..
        } = &generalized.params[0].ty
        else {
            unreachable!()
        };
        assert!(params.is_empty());
        assert_eq!(**ret, generalized.ret_type);
        assert!(matches!(generalized.ret_type, Type::Generic(_)));
        assert_eq!(
            *result_borrowing,
            B::Sources(Box::new(vec![
                S::EnvironmentValue,
                S::EnvironmentContents,
                S::Static,
                S::Unknown
            ]))
        );
        let HirStmt::Expr(expression) = &generalized.body.stmts[0] else {
            unreachable!()
        };
        let HirExprKind::Call(callee, _, _) = &expression.kind else {
            unreachable!()
        };
        assert_eq!(callee.ty, generalized.params[0].ty);
    }

    #[test]
    fn composed_callable_demands_are_definition_owned_and_commit_only_a_linear_tree() {
        use crate::types::{CallableKind, CallableResultBorrowing as B, CallableResultSource as S};
        let mut engine = InferenceEngine::new();
        let (mut function, headers) = invoking_input(&mut engine);
        let inner = function.params[0].clone();
        function.params[0].ty =
            Type::function(vec![function.ret_type.clone()], function.ret_type.clone());
        function.params.push(HirParam {
            name: "inner".into(),
            local_id: crate::ids::HirLocalId(1),
            ..inner
        });
        let HirStmt::Expr(expression) = &mut function.body.stmts[0] else {
            unreachable!()
        };
        let nested = expression.clone();
        let HirExprKind::Call(callee, args, _) = &mut expression.kind else {
            unreachable!()
        };
        callee.ty = function.params[0].ty.clone();
        let mut nested = nested;
        let HirExprKind::Call(inner, _, _) = &mut nested.kind else {
            unreachable!()
        };
        inner.kind = HirExprKind::ResolvedVar(HirVarRef {
            name: "inner".into(),
            target: HirVarTarget::Local(function.params[1].local_id),
        });
        args.push(nested);
        let original = function.clone();
        let generalized =
            generalize_single_function(&engine, &ConstraintStore::new(), &headers, function);
        for (index, parameter) in generalized.params.iter().enumerate() {
            let Type::Function {
                result_borrowing: B::Sources(sources),
                callable_kind,
                ..
            } = &parameter.ty
            else {
                panic!("composed input demand")
            };
            assert_eq!(*callable_kind, CallableKind::FnOnce);
            assert!(
                sources.contains(&S::EnvironmentValue)
                    && sources.contains(&S::EnvironmentContents)
                    && sources.contains(&S::Static)
                    && sources.contains(&S::Unknown)
            );
            assert!(!sources.contains(&S::EnvironmentStorage));
            assert_eq!(sources.contains(&S::ArgumentValue(0)), index == 0);
            assert!(matches!(
                engine.resolve(&original.params[index].ty),
                Type::Function {
                    result_borrowing: B::Unknown,
                    ..
                }
            ));
        }
        assert!(matches!(generalized.ret_type, Type::Generic(_)));
        let HirStmt::Expr(expression) = &generalized.body.stmts[0] else {
            unreachable!()
        };
        let HirExprKind::Call(outer, args, _) = &expression.kind else {
            unreachable!()
        };
        assert_eq!(outer.ty, generalized.params[0].ty);
        let HirExprKind::Call(inner, _, _) = &args[0].kind else {
            unreachable!()
        };
        assert_eq!(inner.ty, generalized.params[1].ty);

        for mode in 0..3 {
            let mut changed = original.clone();
            match mode {
                0 => changed.params[1].mutable = true,
                1 => changed.params[1].is_ref = true,
                2 => {
                    let HirStmt::Expr(expression) = &mut changed.body.stmts[0] else {
                        unreachable!()
                    };
                    let HirExprKind::Call(_, args, _) = &mut expression.kind else {
                        unreachable!()
                    };
                    let HirExprKind::Call(inner, _, _) = &mut args[0].kind else {
                        unreachable!()
                    };
                    inner.kind = HirExprKind::ResolvedVar(HirVarRef {
                        name: "outer".into(),
                        target: HirVarTarget::Local(changed.params[0].local_id),
                    });
                }
                _ => unreachable!(),
            }
            infer_direct_callable_result_demand(&engine, &mut changed);
            assert!(
                changed.params.iter().all(|parameter| matches!(
                    parameter.ty,
                    Type::Function {
                        result_borrowing: B::Unknown,
                        ..
                    }
                )),
                "a vetoed tree cannot commit even its otherwise eligible outer demand"
            );
        }
    }

    #[test]
    fn callable_demands_follow_definition_uses_not_the_result_syntax() {
        use crate::types::{CallableKind, CallableResultBorrowing as B};
        let mut engine = InferenceEngine::new();
        let (mut function, _) = invoking_input(&mut engine);
        let HirStmt::Expr(call) = function.body.stmts.pop().unwrap() else {
            unreachable!()
        };
        let result = crate::ids::HirLocalId(1);
        let variable = HirExpr {
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: "result".into(),
                target: HirVarTarget::Local(result),
            }),
            ty: call.ty.clone(),
            span: crate::lexer::Span::test(),
        };
        let tuple = Type::Tuple(vec![call.ty.clone(), call.ty.clone()]);
        let incidental = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "incidental".into(),
                        target: HirVarTarget::Function(def_id(91)),
                    }),
                    ty: Type::function(vec![], Type::Unit),
                    span: crate::lexer::Span::test(),
                }),
                vec![],
                None,
            ),
            ty: Type::Unit,
            span: crate::lexer::Span::test(),
        };
        function.body.stmts = vec![
            HirStmt::Expr(incidental),
            HirStmt::Let {
                name: "result".into(),
                local_id: result,
                ty: call.ty.clone(),
                value: call,
                mutable: false,
            },
            HirStmt::Expr(HirExpr {
                kind: HirExprKind::TupleLiteral(vec![variable.clone(), variable]),
                ty: tuple.clone(),
                span: crate::lexer::Span::test(),
            }),
        ];
        function.ret_type = tuple.clone();
        function.body.ty = tuple;
        infer_direct_callable_result_demand(&engine, &mut function);
        let Type::Function {
            callable_kind,
            result_borrowing,
            ..
        } = &function.params[0].ty
        else {
            unreachable!()
        };
        assert_eq!(*callable_kind, CallableKind::FnOnce);
        assert!(matches!(result_borrowing, B::Sources(_)));
        let HirStmt::Let { value, .. } = &function.body.stmts[1] else {
            unreachable!()
        };
        let HirExprKind::Call(callee, ..) = &value.kind else {
            unreachable!()
        };
        assert_eq!(callee.ty, function.params[0].ty);
        assert_eq!(
            function.ret_type, function.body.ty,
            "a demand cannot replace the result's type or certify incidental effects"
        );
    }

    #[test]
    fn definition_use_demands_veto_repeated_borrowed_and_captured_inputs_atomically() {
        use crate::types::CallableResultBorrowing as B;
        let mut engine = InferenceEngine::new();
        let (original, _) = invoking_input(&mut engine);
        let HirStmt::Expr(call) = &original.body.stmts[0] else {
            unreachable!()
        };
        let HirExprKind::Call(callee, ..) = &call.kind else {
            unreachable!()
        };
        for mode in 0..4 {
            let mut function = original.clone();
            let expression = match mode {
                0 => HirExpr {
                    kind: HirExprKind::Loop(function.body.clone()),
                    ty: Type::Never,
                    span: crate::lexer::Span::test(),
                },
                1 => HirExpr {
                    kind: HirExprKind::Ref(false, callee.clone()),
                    ty: Type::Reference {
                        mutable: false,
                        inner: Box::new(callee.ty.clone()),
                    },
                    span: crate::lexer::Span::test(),
                },
                2 => HirExpr {
                    kind: HirExprKind::Lambda {
                        params: vec![],
                        body: function.body.clone(),
                        captures: vec![HirClosureCapture {
                            name: "callback".into(),
                            local_id: function.params[0].local_id,
                            kind: HirClosureCaptureKind::Move,
                            mutable: false,
                            ty: function.params[0].ty.clone(),
                        }],
                    },
                    ty: Type::function(vec![], function.ret_type.clone()),
                    span: crate::lexer::Span::test(),
                },
                3 => call.clone(),
                _ => unreachable!(),
            };
            function.body.stmts.insert(0, HirStmt::Expr(expression));
            infer_direct_callable_result_demand(&engine, &mut function);
            assert!(
                matches!(
                    function.params[0].ty,
                    Type::Function {
                        result_borrowing: B::Unknown,
                        ..
                    }
                ),
                "a vetoed definition cannot leave partial input authority (mode={mode})"
            );
            assert_eq!(function.params[0].ty, original.params[0].ty);
        }
    }

    #[test]
    fn definition_callable_demand_does_not_claim_single_consumption_for_nested_uses() {
        use crate::types::CallableResultBorrowing as B;
        let mut engine = InferenceEngine::new();
        let (mut function, headers) = invoking_input(&mut engine);
        let callable = Type::function(vec![Type::I64], function.ret_type.clone());
        function.params[0].ty = callable.clone();
        let HirStmt::Expr(expression) = &mut function.body.stmts[0] else {
            unreachable!()
        };
        let HirExprKind::Call(callee, args, _) = &mut expression.kind else {
            unreachable!()
        };
        callee.ty = callable;
        args.push(HirExpr {
            kind: HirExprKind::Call(
                callee.clone(),
                vec![HirExpr {
                    kind: HirExprKind::IntLiteral(42),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                }],
                None,
            ),
            ty: function.ret_type.clone(),
            span: crate::lexer::Span::test(),
        });
        let generalized =
            generalize_single_function(&engine, &ConstraintStore::new(), &headers, function);
        assert!(
            matches!(
                generalized.params[0].ty,
                Type::Function {
                    result_borrowing: B::Unknown,
                    ..
                }
            ),
            "nested input uses need separate composition/capability evidence"
        );
    }

    #[test]
    fn definition_callable_demand_does_not_replace_declared_obligations_or_borrowed_inputs() {
        use crate::types::CallableResultBorrowing as B;
        let mut engine = InferenceEngine::new();
        let (mut function, headers) = invoking_input(&mut engine);
        let Type::Function {
            result_borrowing, ..
        } = &mut function.params[0].ty
        else {
            unreachable!()
        };
        *result_borrowing = B::Sources(Box::default());
        let generalized =
            generalize_single_function(&engine, &ConstraintStore::new(), &headers, function);
        assert!(
            matches!(&generalized.params[0].ty, Type::Function { result_borrowing: B::Sources(sources), .. } if sources.is_empty())
        );
        for (mutable, is_ref) in [(true, false), (false, true)] {
            let (mut function, headers) = invoking_input(&mut engine);
            function.params[0].mutable = mutable;
            function.params[0].is_ref = is_ref;
            let generalized =
                generalize_single_function(&engine, &ConstraintStore::new(), &headers, function);
            assert!(matches!(
                generalized.params[0].ty,
                Type::Function {
                    result_borrowing: B::Unknown,
                    ..
                }
            ));
        }
    }

    #[test]
    fn definition_callable_demand_does_not_certify_closed_or_argument_bearing_headers() {
        use crate::types::CallableResultBorrowing as B;
        let mut engine = InferenceEngine::new();
        let (mut function, _) = invoking_input(&mut engine);
        function.params[0].ty = Type::function(vec![], Type::I64);
        function.ret_type = Type::I64;
        let generalized =
            generalize_single_function(&engine, &ConstraintStore::new(), &HashMap::new(), function);
        assert!(matches!(
            generalized.params[0].ty,
            Type::Function {
                result_borrowing: B::Unknown,
                ..
            }
        ));
        let (mut function, headers) = invoking_input(&mut engine);
        let Type::Function { params, .. } = &mut function.params[0].ty else {
            unreachable!()
        };
        params.push(Type::I64);
        let generalized =
            generalize_single_function(&engine, &ConstraintStore::new(), &headers, function);
        assert!(matches!(
            generalized.params[0].ty,
            Type::Function {
                result_borrowing: B::Unknown,
                ..
            }
        ));
    }

    fn function_with_type_vars(id: DefId, var_order: &[TypeVarId]) -> HirFunction {
        HirFunction {
            id,
            name: "pair".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: var_order
                .iter()
                .enumerate()
                .map(|(index, var_id)| HirParam {
                    name: format!("param{index}"),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::TypeVar(*var_id),
                    mutable: false,
                    is_ref: false,
                })
                .collect(),
            ret_type: Type::TypeVar(var_order[0]),
            body: HirBlock {
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKind::Var("param0".to_string()),
                    ty: Type::TypeVar(var_order[0]),
                    span: crate::lexer::Span::test(),
                })],
                ty: Type::TypeVar(var_order[0]),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    #[test]
    fn generalize_assigns_generic_ids_in_signature_order() {
        let engine = InferenceEngine::new();
        let id = def_id(40);
        let var_order = [
            TypeVarId(6),
            TypeVarId(5),
            TypeVarId(4),
            TypeVarId(3),
            TypeVarId(2),
            TypeVarId(1),
            TypeVarId(0),
        ];
        let func = function_with_type_vars(id, &var_order);
        let function_type_vars = HashMap::from([(id, var_order.iter().copied().collect())]);

        let generalized = generalize_single_function(
            &engine,
            &ConstraintStore::default(),
            &function_type_vars,
            func,
        );

        assert_eq!(
            generalized
                .generic_params
                .iter()
                .map(|param| param.name.as_str())
                .collect::<Vec<_>>(),
            vec!["T", "U", "V", "W", "X", "Y", "Z"]
        );
        for (expected_index, param) in generalized.params.iter().enumerate() {
            assert_eq!(
                param.ty,
                Type::Generic(GenericParamId {
                    owner: id,
                    index: expected_index as u32,
                })
            );
        }
        assert_eq!(
            generalized.ret_type,
            Type::Generic(GenericParamId {
                owner: id,
                index: 0,
            })
        );
    }

    #[test]
    fn generalize_discovers_type_vars_nested_in_resolved_signature_types() {
        let id = def_id(41);
        let header_var = TypeVarId(0);
        let nested_var = TypeVarId(1);
        let body_alias = TypeVarId(2);
        let option_id = def_id(42);
        let option_ty = Type::Enum {
            id: option_id,
            args: vec![Type::TypeVar(nested_var)],
        };
        let mut engine = InferenceEngine::new();
        engine
            .unify(&Type::TypeVar(header_var), &option_ty)
            .unwrap();
        engine
            .unify(&Type::TypeVar(body_alias), &Type::TypeVar(nested_var))
            .unwrap();
        let mut function = function_with_type_vars(id, &[header_var]);
        function.params.clear();
        function.ret_type = Type::TypeVar(header_var);
        function.body.ty = Type::Enum {
            id: option_id,
            args: vec![Type::TypeVar(body_alias)],
        };
        let function_type_vars = HashMap::from([(id, HashSet::from([header_var]))]);

        let generalized = generalize_single_function(
            &engine,
            &ConstraintStore::default(),
            &function_type_vars,
            function,
        );
        let generic = Type::Generic(GenericParamId {
            owner: id,
            index: 0,
        });

        assert_eq!(
            generalized
                .generic_params
                .iter()
                .map(|param| param.name.as_str())
                .collect::<Vec<_>>(),
            vec!["T"]
        );
        assert_eq!(
            generalized.ret_type,
            Type::Enum {
                id: option_id,
                args: vec![generic.clone()],
            }
        );
        assert_eq!(
            generalized.body.ty,
            Type::Enum {
                id: option_id,
                args: vec![generic],
            }
        );
    }

    #[test]
    fn generalize_transports_inferred_trait_bounds_to_generic_parameters() {
        let id = def_id(43);
        let trait_id = def_id(44);
        let var = TypeVarId(0);
        let mut engine = InferenceEngine::new();
        engine.add_bound(
            var,
            crate::types::TraitBound {
                trait_id,
                type_args: vec![Type::I64],
            },
        );
        let function = function_with_type_vars(id, &[var]);
        let function_type_vars = HashMap::from([(id, HashSet::from([var]))]);

        let generalized = generalize_single_function(
            &engine,
            &ConstraintStore::default(),
            &function_type_vars,
            function,
        );
        let generic = GenericParamId {
            owner: id,
            index: 0,
        };

        assert_eq!(
            generalized.generic_bounds.get(&generic),
            Some(&vec![crate::types::TraitBound {
                trait_id,
                type_args: vec![Type::I64],
            }])
        );
    }

    #[test]
    fn generalize_preserves_constructor_variable_kinds() {
        let id = def_id(44);
        let mut engine = InferenceEngine::new();
        let variable = engine.fresh_type_var_of_kind(crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        ));
        let Type::TypeVar(var_id) = variable.clone() else {
            unreachable!();
        };
        let applied = Type::Apply {
            constructor: Box::new(variable),
            args: vec![Type::I64],
        };
        let mut function = function_with_type_vars(id, &[var_id]);
        function.params[0].ty = applied.clone();
        function.ret_type = applied;
        let function_type_vars = HashMap::from([(id, HashSet::from([var_id]))]);

        let generalized = generalize_single_function(
            &engine,
            &ConstraintStore::default(),
            &function_type_vars,
            function,
        );

        assert_eq!(
            generalized.generic_params[0].kind,
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            )
        );
        assert_eq!(
            generalized.params[0].ty,
            Type::Apply {
                constructor: Box::new(Type::Generic(GenericParamId {
                    owner: id,
                    index: 0,
                })),
                args: vec![Type::I64],
            }
        );
    }

    #[test]
    fn generalize_preserves_literal_backed_signature_vars_for_defaulting() {
        let id = def_id(43);
        let var = TypeVarId(0);
        let engine = InferenceEngine::new();
        let function = function_with_type_vars(id, &[var]);
        let function_type_vars = HashMap::from([(id, HashSet::from([var]))]);
        let mut constraints = ConstraintStore::default();
        constraints.add_int_literal(var, crate::lexer::Span::test());

        let generalized =
            generalize_single_function(&engine, &constraints, &function_type_vars, function);

        assert!(generalized.generic_params.is_empty());
        assert_eq!(generalized.ret_type, Type::TypeVar(var));
        assert_eq!(generalized.body.ty, Type::TypeVar(var));
    }

    #[test]
    fn mixed_signature_generalization_preserves_existing_ids_and_body_only_unknowns() {
        let id = def_id(80);
        let impl_parameter = GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(81),
                index: 1,
            },
            "T",
        );
        let method_parameter = GenericParamDecl::type_param(
            GenericParamId {
                owner: id,
                index: 3,
            },
            "U",
        );
        let mut engine = InferenceEngine::new();
        let Type::TypeVar(slot) = engine.fresh_type_var_at(crate::lexer::Span::test()) else {
            unreachable!()
        };
        let body_only = engine.fresh_type_var_at(crate::lexer::Span::test());
        let mut function = function_with_type_vars(id, &[slot]);
        function.generic_params = vec![impl_parameter.clone(), method_parameter.clone()];
        function.ret_type = Type::Tuple(vec![
            Type::Generic(impl_parameter.id),
            Type::Generic(method_parameter.id),
            Type::TypeVar(slot),
        ]);
        function.body.ty = function.ret_type.clone();
        function.body.stmts.push(HirStmt::Expr(HirExpr {
            kind: HirExprKind::Var("unconstrained_local".into()),
            ty: body_only.clone(),
            span: crate::lexer::Span::test(),
        }));
        let headers = HashMap::from([(id, HashSet::from([slot]))]);
        let generalized =
            generalize_single_function(&engine, &ConstraintStore::new(), &headers, function);
        assert_eq!(
            &generalized.generic_params[..2],
            &[impl_parameter, method_parameter]
        );
        assert_eq!(generalized.generic_params.len(), 3);
        assert_eq!(
            generalized.generic_params[2].id,
            GenericParamId {
                owner: id,
                index: 4
            }
        );
        assert_eq!(generalized.generic_params[2].name, "V");
        let HirStmt::Expr(local) = &generalized.body.stmts[1] else {
            unreachable!()
        };
        assert_eq!(local.ty, body_only);
        let mut errors = Vec::new();
        let _ = engine.finalize_strict(&local.ty, &mut errors);
        assert_eq!(
            errors.len(),
            1,
            "signature generalization cannot accept unknown locals"
        );
    }
}

#[cfg(test)]
#[path = "allocator_constructor_tests.rs"]
mod allocator_constructor_tests;
