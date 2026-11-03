//! Static bound member path resolution.

use crate::hir::*;
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::types::{GenericParamId, Type};
use std::collections::{HashMap, HashSet};

pub(super) fn display_type_without_ids(ty: &Type) -> String {
    let context = crate::type_services::display::TypeDisplayContext::default();
    crate::type_services::display::display_type_with_context(ty, &context).to_string()
}

impl Lowerer {
    pub(super) fn current_generic_param_id_for_name(&self, name: &str) -> Option<GenericParamId> {
        let owner = self.current_generic_owner()?;
        let index = self
            .current_generic_params()
            .iter()
            .position(|param| param == name)?;
        Some(GenericParamId {
            owner,
            index: index as u32,
        })
    }

    pub(crate) fn resolve_static_bound_method_path(
        &mut self,
        owner_name: &str,
        method_name: &str,
        span: Span,
    ) -> Result<Option<(Type, HirStaticMethodTarget)>, String> {
        self.resolve_static_bound_member_path(owner_name, None, method_name, span)
    }

    pub(crate) fn resolve_static_qualified_bound_method_path(
        &mut self,
        owner_name: &str,
        trait_name: &str,
        method_name: &str,
        span: Span,
    ) -> Result<Option<(Type, HirStaticMethodTarget)>, String> {
        let trait_id = self
            .trait_by_name(trait_name)
            .map(|trait_def| trait_def.id)
            .ok_or_else(|| format!("unknown constructor trait `{trait_name}`"))?;
        self.resolve_static_bound_member_path(owner_name, Some(trait_id), method_name, span)
    }

    fn resolve_static_bound_member_path(
        &mut self,
        owner_name: &str,
        required_trait_id: Option<crate::ids::DefId>,
        method_name: &str,
        operation_span: Span,
    ) -> Result<Option<(Type, HirStaticMethodTarget)>, String> {
        let (owner_ty, bounds) =
            if let Some(owner_param) = self.current_generic_param_id_for_name(owner_name) {
                let owner_ty = Type::Generic(owner_param);
                let Some(bounds) = self.current_impl_bounds().get(&owner_param) else {
                    return Ok(None);
                };
                let bounds = self
                    .selection_service()
                    .trait_bounds_with_supertraits(&owner_ty, bounds);
                (owner_ty, bounds)
            } else {
                let Some(nominal) = crate::lower::resolution::LowerResolutionContext::new(self)
                    .resolve_nominal_type(owner_name)
                else {
                    return Ok(None);
                };
                let owner_ty = match nominal {
                    crate::type_lowering::ResolvedNominalType::Struct(def)
                        if def.generic_params.is_empty() =>
                    {
                        Type::Struct {
                            id: def.id,
                            args: Vec::new(),
                        }
                    }
                    crate::type_lowering::ResolvedNominalType::Enum(def)
                        if def.generic_params.is_empty() =>
                    {
                        Type::Enum {
                            id: def.id,
                            args: Vec::new(),
                        }
                    }
                    _ => return Ok(None),
                };
                let matching_traits = self
                    .items
                    .impl_defs_in_order()
                    .map(|(_, imp)| imp)
                    .filter(|imp| {
                        imp.receiver_pattern == HirImplReceiverPattern::Exact(owner_ty.clone())
                    })
                    .filter(|imp| {
                        imp.methods
                            .get(method_name)
                            .is_some_and(|method| !method.is_method)
                    })
                    .map(|imp| imp.trait_id)
                    .collect::<Vec<_>>();
                let Some(Some(trait_id)) = matching_traits.first().copied() else {
                    return Ok(None);
                };
                if required_trait_id.is_some_and(|required| required != trait_id)
                    || matching_traits.len() < 2
                    || matching_traits
                        .iter()
                        .any(|candidate| *candidate != Some(trait_id))
                {
                    return Ok(None);
                }
                // Resolve the shared trait member now, but let argument inference choose
                // its implementation rather than selecting an arbitrary overload.
                let trait_def = self
                    .trait_by_id(trait_id)
                    .cloned()
                    .ok_or_else(|| "static method has unknown trait authority".to_string())?;
                let bound = crate::types::TraitBound {
                    trait_id,
                    type_args: trait_def
                        .generic_params
                        .iter()
                        .map(|param| {
                            self.engine
                                .fresh_type_var_at_kind(operation_span.clone(), param.kind.clone())
                        })
                        .collect(),
                };
                self.constraint_store.add_trait(
                    owner_ty.clone(),
                    bound.clone(),
                    operation_span.clone(),
                    "static trait member",
                );
                (owner_ty, vec![bound])
            };

        let mut candidates = Vec::new();
        for bound in &bounds {
            if required_trait_id.is_some_and(|required| required != bound.trait_id) {
                continue;
            }
            let Some(trait_def) = self.trait_by_id(bound.trait_id).cloned() else {
                continue;
            };
            let owner_kind = self.engine.kind_of(&owner_ty).map_err(|error| {
                format!("cannot determine kind of constructor `{owner_name}`: {error}")
            })?;
            let target_kind = trait_def
                .target
                .as_ref()
                .map(|target| target.kind.clone())
                .unwrap_or(crate::type_services::kind::Kind::Type);
            if owner_kind != target_kind {
                if required_trait_id == Some(trait_def.id) {
                    return Err(format!(
                        "constructor target `{owner_name}` has kind {owner_kind}, but trait '{}' requires {target_kind}",
                        trait_def.name
                    ));
                }
                continue;
            }

            let mut subst = HashMap::new();
            for (index, arg) in bound.type_args.iter().enumerate() {
                subst.insert(
                    GenericParamId {
                        owner: trait_def.id,
                        index: index as u32,
                    },
                    arg.clone(),
                );
            }
            let target_param =
                trait_def
                    .target
                    .as_ref()
                    .map(|target| target.id)
                    .unwrap_or(GenericParamId {
                        owner: trait_def.id,
                        index: trait_def.generic_params.len() as u32,
                    });
            subst.insert(target_param, owner_ty.clone());

            let (method_id, method_generic_params, method_generic_bounds, params, ret, safety) =
                if let Some(method) = trait_def
                    .methods
                    .get(method_name)
                    .filter(|method| !method.is_method)
                {
                    (
                        method.id,
                        method.generic_params.clone(),
                        method.generic_bounds.clone(),
                        method.params.iter().map(|param| param.ty.clone()).collect(),
                        method.ret_type.clone(),
                        crate::types::FunctionSafety::from_is_unsafe(method.is_unsafe),
                    )
                } else if let Some(sig) = trait_def
                    .signatures
                    .get(method_name)
                    .filter(|sig| sig.self_receiver.is_none())
                {
                    (
                        sig.id,
                        sig.generic_params.clone(),
                        sig.generic_bounds.clone(),
                        sig.params.clone(),
                        sig.ret.clone(),
                        crate::types::FunctionSafety::from_is_unsafe(sig.is_unsafe),
                    )
                } else {
                    continue;
                };

            let mut inherited_generic_params = trait_def
                .generic_params
                .iter()
                .map(|param| param.id)
                .collect::<HashSet<_>>();
            if let Some(target) = trait_def.target.as_ref() {
                inherited_generic_params.insert(target.id);
            }
            let is_constructor_trait = trait_def.target.as_ref().is_some_and(|target| {
                !matches!(target.kind, crate::type_services::kind::Kind::Type)
            });
            let method_generic_params = method_generic_params
                .into_iter()
                .filter(|param| {
                    if is_constructor_trait {
                        !inherited_generic_params.contains(&param.id)
                    } else {
                        param.id.owner == method_id
                    }
                })
                .collect::<Vec<_>>();

            if let Some(target_decl) = trait_def.target.as_ref() {
                let declared_generics = trait_def
                    .methods
                    .get(method_name)
                    .map(|method| method.generic_params.as_slice())
                    .or_else(|| {
                        trait_def
                            .signatures
                            .get(method_name)
                            .map(|signature| signature.generic_params.as_slice())
                    })
                    .unwrap_or_default();
                for generic in declared_generics {
                    if generic.name == target_decl.name && generic.kind == target_decl.kind {
                        subst.insert(generic.id, owner_ty.clone());
                    }
                }
            }

            let params: Vec<Type> = params
                .into_iter()
                .map(|param| param.substitute_generics(&subst))
                .collect();
            let ret = ret.substitute_generics(&subst);
            let method_generic_bounds = method_generic_bounds
                .iter()
                .map(|(&param, bounds)| {
                    (
                        param,
                        bounds
                            .iter()
                            .map(|bound| crate::types::TraitBound {
                                trait_id: bound.trait_id,
                                type_args: bound
                                    .type_args
                                    .iter()
                                    .map(|arg| arg.substitute_generics(&subst))
                                    .collect(),
                            })
                            .collect(),
                    )
                })
                .collect::<HirGenericBounds>();
            let target = HirMethodCallTarget::trait_method(
                trait_def.id,
                method_id,
                bound
                    .type_args
                    .iter()
                    .map(|arg| arg.substitute_generics(&subst))
                    .collect(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            );
            let mut target = target;
            if !matches!(target_kind, crate::type_services::kind::Kind::Type) {
                target.owner_substitution = subst
                    .iter()
                    .map(|(&param, ty)| HirTypeBinding {
                        param,
                        ty: ty.clone(),
                    })
                    .collect();
                target.owner_substitution.sort_by_key(|binding| {
                    (
                        binding.param.owner.crate_id.0,
                        binding.param.owner.local.0,
                        binding.param.index,
                    )
                });
            }
            candidates.push((
                params,
                ret,
                safety,
                target,
                method_generic_params,
                method_generic_bounds,
            ));
        }
        candidates.sort_by_key(|(_, _, _, target, _, _)| {
            (
                target.trait_id(),
                target.method_id(),
                format!("{:?}", target.trait_args()),
            )
        });
        candidates.dedup_by(|left, right| left.3 == right.3);
        let mut candidate_descriptions = candidates
            .iter()
            .map(|(_, _, _, target, _, _)| {
                let trait_name = target
                    .trait_id()
                    .and_then(|trait_id| self.trait_by_id(trait_id))
                    .map(|trait_def| trait_def.name.as_str())
                    .unwrap_or("<unknown trait>");
                let args = target
                    .trait_args()
                    .iter()
                    .map(|arg| self.display_type(arg))
                    .collect::<Vec<_>>();
                if args.is_empty() {
                    trait_name.to_string()
                } else {
                    format!("{} {}", trait_name, args.join(", "))
                }
            })
            .collect::<Vec<_>>();
        candidate_descriptions.sort();
        candidate_descriptions.dedup();
        match candidates.len() {
            0 => Ok(None),
            1 => {
                let (
                    params,
                    ret,
                    safety,
                    mut target,
                    method_generic_params,
                    method_generic_bounds,
                ) =
                    candidates.pop().expect("single static bound candidate");
                let mut method_subst = HashMap::new();
                target.method_substitution = method_generic_params
                    .into_iter()
                    .map(|param| {
                        let ty = self
                            .engine
                            .fresh_type_var_at_kind(operation_span.clone(), param.kind);
                        method_subst.insert(param.id, ty.clone());
                        HirTypeBinding {
                            param: param.id,
                            ty,
                        }
                    })
                    .collect();
                for (generic_param, bounds) in &method_generic_bounds {
                    let Some(subject) = method_subst.get(generic_param).cloned() else {
                        continue;
                    };
                    for bound in bounds {
                        self.constraint_store.add_trait(
                            subject.clone(),
                            crate::types::TraitBound {
                                trait_id: bound.trait_id,
                                type_args: bound
                                    .type_args
                                    .iter()
                                    .map(|arg| arg.substitute_generics(&method_subst))
                                    .collect(),
                            },
                            operation_span.clone(),
                            "static constructor trait member",
                        );
                    }
                }
                let params = params
                    .into_iter()
                    .map(|param| param.substitute_generics(&method_subst))
                    .collect();
                let ret = ret.substitute_generics(&method_subst);
                Ok(Some((
                    Type::function_with_safety(params, ret, safety),
                    HirStaticMethodTarget {
                        owner_ty,
                        method: target,
                    },
                )))
            }
            _ => Err(format!(
                "ambiguous static bound method `{owner_name}::{method_name}`: matching trait bounds {}",
                candidate_descriptions.join(", ")
            )),
        }
    }
}
