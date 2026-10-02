//! Public signatures for a virtual edge, without exposing its hidden witness.
use std::collections::HashMap;

use super::*;
use crate::selection::SelectedMethod;
use crate::types::ObjectType;

pub(crate) fn select_callable(
    service: &crate::selection::SelectionService<'_>,
    traits: &HashMap<DefId, HirTrait>,
    items: &HirLanguageItems,
    candidates: &[crate::selection::ReceiverCandidate],
    object: &ObjectType,
    member_name: Option<&str>,
) -> Result<Option<SelectedMethod>, String> {
    let roots = std::iter::once(object.principal.clone())
        .chain(object.guarantees.iter().cloned())
        .collect::<Vec<_>>();
    let views = crate::traits::evidence::implied_trait_bounds(
        traits,
        &Type::ObjectSelf { depth: 0 },
        &roots,
    );
    for (kind, trait_id, method_id) in items.callable_protocols() {
        if kind == crate::types::CallableKind::FnOnce {
            continue;
        }
        let Some(definition) = traits.get(&trait_id) else {
            continue;
        };
        let name = definition
            .signatures
            .iter()
            .find(|(_, method)| method.id == method_id)
            .map(|(name, _)| name)
            .or_else(|| {
                definition
                    .methods
                    .iter()
                    .find(|(_, method)| method.id == method_id)
                    .map(|(name, _)| name)
            });
        let Some(name) = name else {
            return Err("callable language item has no canonical member".into());
        };
        if member_name.is_some_and(|requested| requested != name) {
            continue;
        }
        let bounds = views
            .iter()
            .filter(|bound| bound.trait_id == trait_id)
            .cloned()
            .collect::<Vec<_>>();
        if bounds.is_empty() {
            continue;
        }
        let mut selected = service
            .select_bound_method(
                candidates,
                &bounds,
                name,
                Type::Object(Box::new(object.clone())),
            )
            .map_err(|error| error.message())?;
        if selected.target.method_id() != Some(method_id) {
            return Err("callable object selected a non-protocol member".into());
        }
        adapt(&mut selected, traits, object)?;
        return Ok(Some(selected));
    }
    Ok(None)
}

pub(crate) fn signature(
    traits: &HashMap<DefId, HirTrait>,
    target: &HirMethodCallTarget,
    object: &ObjectType,
) -> Result<(Vec<Type>, Type), String> {
    signature_for_access(traits, target, object, false, None)
}

pub(crate) fn consuming_signature(
    traits: &HashMap<DefId, HirTrait>,
    target: &HirMethodCallTarget,
    object: &ObjectType,
) -> Result<(Vec<Type>, Type), String> {
    signature_for_access(traits, target, object, true, None)
}

pub(crate) fn opened_signature(
    traits: &HashMap<DefId, HirTrait>,
    target: &HirMethodCallTarget,
    object: &ObjectType,
    witness: crate::types::WitnessId,
) -> Result<(Vec<Type>, Type), String> {
    let consuming = target
        .trait_id()
        .and_then(|id| traits.get(&id))
        .and_then(|declaration| {
            declaration
                .signatures
                .values()
                .find(|signature| Some(signature.id) == target.method_id())
                .map(|signature| signature.self_receiver)
                .or_else(|| {
                    declaration
                        .methods
                        .values()
                        .find(|method| Some(method.id) == target.method_id())
                        .map(|method| method.self_receiver)
                })
        })
        .flatten()
        == Some(ReceiverMode::Move);
    opened_signature_for_access(traits, target, object, witness, consuming)
}

pub(crate) fn opened_consuming_signature(
    traits: &HashMap<DefId, HirTrait>,
    target: &HirMethodCallTarget,
    object: &ObjectType,
    witness: crate::types::WitnessId,
) -> Result<(Vec<Type>, Type), String> {
    opened_signature_for_access(traits, target, object, witness, true)
}

fn opened_signature_for_access(
    traits: &HashMap<DefId, HirTrait>,
    target: &HirMethodCallTarget,
    object: &ObjectType,
    witness: crate::types::WitnessId,
    consuming: bool,
) -> Result<(Vec<Type>, Type), String> {
    let object = object
        .try_map_types(|ty| {
            crate::type_services::substitution::instantiate_object_self(ty, &Type::Witness(witness))
        })
        .map_err(|error| error.to_string())?;
    signature_for_access(traits, target, &object, consuming, Some(witness))
}

fn signature_for_access(
    traits: &HashMap<DefId, HirTrait>,
    target: &HirMethodCallTarget,
    object: &ObjectType,
    consuming: bool,
    witness: Option<crate::types::WitnessId>,
) -> Result<(Vec<Type>, Type), String> {
    let HirSelectedMethodTarget::TraitMethod {
        trait_id,
        member_id,
        trait_args,
        ..
    } = &target.target
    else {
        return Err("virtual edge requires a canonical trait member".into());
    };
    let definition = traits.get(trait_id).ok_or("unknown virtual trait")?;
    let self_id = definition
        .target
        .as_ref()
        .map(|target| target.id)
        .unwrap_or(GenericParamId {
            owner: definition.id,
            index: definition.generic_params.len() as u32,
        });
    let mut substitution: HashMap<_, _> = definition
        .generic_params
        .iter()
        .zip(trait_args)
        .map(|(parameter, ty)| (parameter.id, ty.clone()))
        .collect();
    substitution.extend(
        target
            .method_substitution
            .iter()
            .map(|binding| (binding.param, binding.ty.clone())),
    );
    substitution.insert(self_id, Type::ObjectSelf { depth: 0 });
    let (params, ret, receiver) = if let Some(signature) = definition
        .signatures
        .values()
        .find(|signature| signature.id == *member_id)
    {
        (
            signature.params.iter().skip(1).cloned().collect::<Vec<_>>(),
            signature.ret.clone(),
            signature.self_receiver,
        )
    } else {
        let method = definition
            .methods
            .values()
            .find(|method| method.id == *member_id)
            .ok_or("unknown virtual member")?;
        (
            method
                .params
                .iter()
                .skip(1)
                .map(|param| param.ty.clone())
                .collect(),
            method.ret_type.clone(),
            method.self_receiver,
        )
    };
    if consuming && receiver != Some(ReceiverMode::Move) {
        return Err("owned object dispatch requires a consuming member".into());
    }
    if !consuming && !matches!(receiver, Some(ReceiverMode::Shared | ReceiverMode::Mut)) {
        return Err("a borrowed object requires a borrowed method receiver".into());
    }
    let mut folder = ObjectMethodTypes { object, depth: 0 };
    let mut ret =
        crate::type_services::visit::fold_type(ret.substitute_generics(&substitution), &mut folder);
    if let Type::Reference { mutable, inner } = &ret {
        if witness.is_none()
            && !consuming
            && matches!(inner.as_ref(), Type::ObjectSelf { depth: 0 })
        {
            ret = Type::Reference {
                mutable: *mutable,
                inner: Box::new(Type::Object(Box::new(object.clone()))),
            };
        }
    }
    let mut params = params
        .into_iter()
        .map(|ty| {
            crate::type_services::visit::fold_type(
                ty.substitute_generics(&substitution),
                &mut folder,
            )
        })
        .collect::<Vec<_>>();
    if let Some(witness) = witness {
        ret = crate::type_services::substitution::instantiate_object_self(
            &ret,
            &Type::Witness(witness),
        )
        .map_err(|error| error.to_string())?;
        params = params
            .iter()
            .map(|ty| {
                crate::type_services::substitution::instantiate_object_self(
                    ty,
                    &Type::Witness(witness),
                )
                .map_err(|error| error.to_string())
            })
            .collect::<Result<_, _>>()?;
    }
    if std::iter::once(&ret)
        .chain(params.iter())
        .any(crate::type_services::substitution::has_free_object_self)
    {
        return Err(
            "this object method exposes hidden Self and requires an existential opening".into(),
        );
    }
    if params.iter().chain(std::iter::once(&ret)).any(|ty| {
        crate::type_services::layout::TypeLayout::sizedness(ty, &|_| {
            crate::type_services::layout::Sizedness::Unknown
        }) == crate::type_services::layout::Sizedness::Unsized
    }) {
        return Err(
            "virtual arguments and results require sized storage; use an explicit handle".into(),
        );
    }
    Ok((params, ret))
}

pub(crate) fn adapt(
    selected: &mut SelectedMethod,
    traits: &HashMap<DefId, HirTrait>,
    object: &ObjectType,
) -> Result<(), String> {
    let (params, ret) = signature(traits, &selected.target, object)?;
    apply_signature(selected, params, ret)
}

pub(crate) fn adapt_owned(
    selected: &mut SelectedMethod,
    traits: &HashMap<DefId, HirTrait>,
    object: &ObjectType,
) -> Result<(), String> {
    let (params, ret) = consuming_signature(traits, &selected.target, object)?;
    apply_signature(selected, params, ret)
}

pub(crate) fn adapt_opened(
    selected: &mut SelectedMethod,
    traits: &HashMap<DefId, HirTrait>,
    object: &ObjectType,
    witness: crate::types::WitnessId,
) -> Result<(), String> {
    let (params, ret) = opened_signature(traits, &selected.target, object, witness)?;
    apply_signature(selected, params, ret)?;
    if let HirSelectedMethodTarget::TraitMethod { dispatch, .. } = &mut selected.target.target {
        *dispatch = HirTraitDispatchKind::Opened(witness);
    }
    Ok(())
}

pub(crate) fn adapt_opened_owned(
    selected: &mut SelectedMethod,
    traits: &HashMap<DefId, HirTrait>,
    object: &ObjectType,
    witness: crate::types::WitnessId,
) -> Result<(), String> {
    let (params, ret) = opened_consuming_signature(traits, &selected.target, object, witness)?;
    apply_signature(selected, params, ret)?;
    if let HirSelectedMethodTarget::TraitMethod { dispatch, .. } = &mut selected.target.target {
        *dispatch = HirTraitDispatchKind::Opened(witness);
    }
    Ok(())
}

fn apply_signature(
    selected: &mut SelectedMethod,
    params: Vec<Type>,
    ret: Type,
) -> Result<(), String> {
    let HirSelectedMethodTarget::TraitMethod { dispatch, .. } = &mut selected.target.target else {
        return Err("virtual edge requires a trait member".into());
    };
    *dispatch = HirTraitDispatchKind::Object;
    selected.return_type = ret.clone();
    for (param, ty) in selected.substituted_params.iter_mut().zip(&params) {
        param.ty = ty.clone();
    }
    if let Some(function) = &mut selected.function {
        function.ret_type = ret;
        for (param, ty) in function.params.iter_mut().skip(1).zip(params) {
            param.ty = ty;
        }
    }
    Ok(())
}

struct ObjectMethodTypes<'a> {
    object: &'a ObjectType,
    depth: u32,
}

impl crate::type_services::visit::TypeFolder for ObjectMethodTypes<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        let binder = matches!(ty, Type::Object(_));
        self.depth += u32::from(binder);
        let ty = crate::type_services::visit::fold_type_children(ty, self);
        self.depth -= u32::from(binder);
        if let Type::Projection {
            ty: subject,
            trait_id,
            assoc_type,
            trait_args,
        } = &ty
        {
            if matches!(subject.as_ref(), Type::ObjectSelf { depth } if *depth == self.depth)
                && self.depth == 0
            {
                if let Some(binding) = self.object.bindings.iter().find(|binding| {
                    binding.key.member == assoc_type.assoc_type_id
                        && binding.key.trait_ref.trait_id == *trait_id
                        && binding.key.trait_ref.type_args == *trait_args
                }) {
                    return binding.ty.clone();
                }
            }
        }
        ty
    }
}
