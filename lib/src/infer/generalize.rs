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
        if let Some(func) = hir.functions.get(&id) {
            if !func.generic_params.is_empty() {
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
            if !func.generic_params.is_empty() {
                continue;
            }
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
    if !func.generic_params.is_empty() {
        return func;
    }

    let func_header_vars = function_type_vars
        .get(&func.id)
        .cloned()
        .unwrap_or_default();

    if func_header_vars.is_empty() {
        return func;
    }

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

    let generic_param_names: Vec<String> = ordered_representatives
        .iter()
        .enumerate()
        .map(|(index, _)| {
            generic_names
                .get(index)
                .map(|name| (*name).to_string())
                .unwrap_or_else(|| format!("T{index}"))
        })
        .collect();

    for (index, rep_id) in ordered_representatives.iter().enumerate() {
        rep_to_generic.insert(
            *rep_id,
            GenericParamId {
                owner: func.id,
                index: index as u32,
            },
        );
    }

    func.generic_params = generic_param_names
        .into_iter()
        .zip(&ordered_representatives)
        .enumerate()
        .map(|(index, (name, representative))| {
            GenericParamDecl::new(
                GenericParamId {
                    owner: func.id,
                    index: index as u32,
                },
                name,
                engine.kind_of_type_var(*representative),
            )
        })
        .collect();

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
}
