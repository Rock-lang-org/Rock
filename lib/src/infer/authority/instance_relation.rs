use std::collections::{HashMap, HashSet};

use crate::hir::HirFunction;
use crate::ids::DefId;
use crate::infer::PartialHir;
use crate::lower::ResolveError;
use crate::types::Type;

use super::type_vars::collect_type_vars;

pub(super) fn propagate_instance_relation(
    engine: &mut crate::infer::InferenceEngine,
    source: &Type,
    instance: &Type,
    bindings: &mut HashMap<crate::ids::TypeVarId, Type>,
    shared: &HashSet<crate::ids::TypeVarId>,
) {
    let source = engine.resolve(source);
    let instance = engine.resolve(instance);
    match (&source, &instance) {
        // Occurs check: a binding whose replacement mentions its own variable
        // would make `substitute` recurse forever (it folds replacements again).
        (Type::TypeVar(id), _)
            if crate::type_services::visit::type_any(&instance, |ty| ty == &Type::TypeVar(*id)) =>
        {
            if instance != source {
                return;
            }
        }
        (Type::TypeVar(id), _) => {
            if shared.contains(id) {
                let _ = engine.unify(&source, &instance);
                bindings.insert(*id, instance);
            } else if let Some(bound) = bindings.get(id).cloned() {
                let _ = engine.unify(&bound, &instance);
            } else {
                bindings.insert(*id, instance);
            }
        }
        (source, Type::TypeVar(_)) if !matches!(source, Type::TypeVar(_)) => {
            let mut source_vars = HashSet::new();
            collect_type_vars(source, &mut source_vars);
            for id in source_vars {
                bindings.entry(id).or_insert_with(|| {
                    let kind = engine.kind_of_type_var(id);
                    engine.fresh_type_var_of_kind(kind)
                });
            }
            let specialized = source.substitute(bindings);
            let _ = engine.unify(&specialized, &instance);
        }
        (
            Type::Reference { inner: source, .. },
            Type::Reference {
                inner: instance, ..
            },
        )
        | (Type::Pointer(source), Type::Pointer(instance))
        | (Type::Slice(source), Type::Slice(instance)) => {
            propagate_instance_relation(engine, source, instance, bindings, shared);
        }
        (Type::Array(source, source_len), Type::Array(instance, instance_len))
            if source_len == instance_len =>
        {
            propagate_instance_relation(engine, source, instance, bindings, shared);
        }
        (Type::Tuple(source), Type::Tuple(instance)) => {
            for (source, instance) in source.iter().zip(instance) {
                propagate_instance_relation(engine, source, instance, bindings, shared);
            }
        }
        (
            Type::Function {
                params: source_params,
                ret: source_ret,
                ..
            },
            Type::Function {
                params: instance_params,
                ret: instance_ret,
                ..
            },
        ) => {
            for (source, instance) in source_params.iter().zip(instance_params) {
                propagate_instance_relation(engine, source, instance, bindings, shared);
            }
            propagate_instance_relation(engine, source_ret, instance_ret, bindings, shared);
        }
        (
            Type::Struct {
                id: source_id,
                args: source_args,
            },
            Type::Struct {
                id: instance_id,
                args: instance_args,
            },
        )
        | (
            Type::Enum {
                id: source_id,
                args: source_args,
            },
            Type::Enum {
                id: instance_id,
                args: instance_args,
            },
        ) if source_id == instance_id => {
            for (source, instance) in source_args.iter().zip(instance_args) {
                propagate_instance_relation(engine, source, instance, bindings, shared);
            }
        }
        (
            Type::Apply {
                constructor: source_constructor,
                args: source_args,
            },
            Type::Apply {
                constructor: instance_constructor,
                args: instance_args,
            },
        ) => {
            propagate_instance_relation(
                engine,
                source_constructor,
                instance_constructor,
                bindings,
                shared,
            );
            for (source, instance) in source_args.iter().zip(instance_args) {
                propagate_instance_relation(engine, source, instance, bindings, shared);
            }
        }
        _ => {}
    }
}

pub(super) fn propagate_generic_scheme_type(
    engine: &mut crate::infer::InferenceEngine,
    source: &Type,
    instance: &Type,
) {
    let source = engine.resolve(source);
    let instance = engine.resolve(instance);
    if matches!(source, Type::Generic(_)) {
        return;
    }
    match (&source, &instance) {
        (Type::TypeVar(_), _) => {}
        (Type::Apply { constructor, .. }, _)
            if matches!(constructor.as_ref(), Type::Generic(_)) => {}
        (
            Type::Reference {
                mutable: source_mutable,
                inner: source_inner,
            },
            Type::Reference {
                mutable: instance_mutable,
                inner: instance_inner,
            },
        ) if source_mutable == instance_mutable => {
            propagate_generic_scheme_type(engine, source_inner, instance_inner);
        }
        (Type::Pointer(source), Type::Pointer(instance))
        | (Type::Slice(source), Type::Slice(instance)) => {
            propagate_generic_scheme_type(engine, source, instance);
        }
        (Type::Array(source, source_len), Type::Array(instance, instance_len))
            if source_len == instance_len =>
        {
            propagate_generic_scheme_type(engine, source, instance);
        }
        (Type::Tuple(source), Type::Tuple(instance)) if source.len() == instance.len() => {
            for (source, instance) in source.iter().zip(instance) {
                propagate_generic_scheme_type(engine, source, instance);
            }
        }
        (
            Type::Function {
                params: source_params,
                ret: source_ret,
                ..
            },
            Type::Function {
                params: instance_params,
                ret: instance_ret,
                ..
            },
        ) => {
            for (source, instance) in source_params.iter().zip(instance_params) {
                propagate_generic_scheme_type(engine, source, instance);
            }
            propagate_generic_scheme_type(engine, source_ret, instance_ret);
        }
        (
            Type::Struct {
                id: source_id,
                args: source_args,
            },
            Type::Struct {
                id: instance_id,
                args: instance_args,
            },
        )
        | (
            Type::Enum {
                id: source_id,
                args: source_args,
            },
            Type::Enum {
                id: instance_id,
                args: instance_args,
            },
        ) if source_id == instance_id => {
            for (source, instance) in source_args.iter().zip(instance_args) {
                propagate_generic_scheme_type(engine, source, instance);
            }
        }
        (
            Type::Apply {
                constructor: source_constructor,
                args: source_args,
            },
            Type::Apply {
                constructor: instance_constructor,
                args: instance_args,
            },
        ) if source_args.len() == instance_args.len() => {
            propagate_generic_scheme_type(engine, source_constructor, instance_constructor);
            for (source, instance) in source_args.iter().zip(instance_args) {
                propagate_generic_scheme_type(engine, source, instance);
            }
        }
        _ => {
            let _ = engine.unify(&source, &instance);
        }
    }
}

pub(super) fn propagate_method_result(
    hir: &mut PartialHir,
    result_ty: Type,
    target: &crate::hir::HirMethodCallTarget,
    errors: &mut Vec<ResolveError>,
    span: &crate::lexer::Span,
) {
    let Some(method) = find_method(hir, target.method_id()) else {
        return;
    };
    let substitutions = target
        .owner_substitution
        .iter()
        .chain(&target.method_substitution)
        .map(|binding| (binding.param, binding.ty.clone()))
        .collect::<HashMap<_, _>>();
    let method_result = method.ret_type.substitute_generics(&substitutions);
    if !matches!(method_result, Type::Function { .. }) {
        return;
    }
    if let Err(error) = hir.engine.unify(&method_result, &result_ty) {
        errors.push(ResolveError::with_span(
            format!(
                "inferred method result for '{}' is incompatible with its solved scheme: {}",
                method.name,
                error.render(&hir.engine)
            ),
            span.clone(),
        ));
    }
}

pub(super) fn find_method(hir: &PartialHir, method_id: Option<DefId>) -> Option<HirFunction> {
    let method_id = method_id?;
    hir.impls
        .values()
        .flat_map(|imp| imp.methods.values())
        .find(|method| method.id == method_id)
        .cloned()
        .or_else(|| {
            hir.traits
                .values()
                .flat_map(|trait_def| trait_def.methods.values())
                .find(|method| method.id == method_id)
                .cloned()
        })
}
