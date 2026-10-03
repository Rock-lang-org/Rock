//! One signature authority shared by body production and artifact admission.
use crate::hir::{HirFunctionSig, HirPhase, HirTraitFor};
use crate::mir::MirObjectResult;
use crate::type_services::normalize::{TypeNormalizationEnv, TypeNormalizer};
use crate::type_services::visit::{fold_type, fold_type_children, TypeFolder};
use crate::types::{GenericParamId, ObjectType, ReceiverMode, TraitBound, Type};
use std::collections::HashMap;

pub(crate) struct ObjectMethodAbi {
    pub receiver: ReceiverMode,
    pub params: Vec<Type>,
    pub ret: Type,
    pub result: MirObjectResult,
}

/// Logical view closure, independent of ABI order or importer ID numbering.
pub(crate) fn admitted_object_views<P: HirPhase>(
    ty: &Type,
    traits: &HashMap<crate::ids::DefId, HirTraitFor<P>>,
    env: &TypeNormalizationEnv,
) -> Result<std::collections::BTreeSet<Type>, String> {
    let Type::Object(object) = ty else {
        return Err("view closure requires an object signature".into());
    };
    let bounds = std::iter::once(object.principal.clone())
        .chain(object.guarantees.iter().cloned())
        .collect::<Vec<_>>();
    let mut subsets = vec![Vec::new()];
    for bound in &bounds {
        if subsets
            .len()
            .checked_mul(2)
            .and_then(|count| count.checked_mul(bounds.len().max(1)))
            .is_none_or(|nodes| {
                nodes > crate::type_services::normalize::DEFAULT_MAX_NORMALIZATION_NODES
            })
        {
            return Err("object view lattice exceeds the compiler metadata resource budget".into());
        }
        let extended = subsets
            .iter()
            .map(|set| {
                let mut set = set.clone();
                set.push(bound.clone());
                set
            })
            .collect::<Vec<_>>();
        subsets.extend(extended);
    }
    let mut views = std::collections::BTreeSet::new();
    for principal in &bounds {
        for guarantees in &subsets {
            let mut view = ObjectType::new(principal.clone());
            view.guarantees.extend(guarantees.iter().cloned());
            view.bindings = object.bindings.clone();
            loop {
                match crate::traits::objects::admit_object(&view, traits, env) {
                    Ok(view) => {
                        let view = Type::Object(Box::new(view));
                        if view != *ty {
                            views.insert(view);
                        }
                        break;
                    }
                    Err(crate::traits::objects::ObjectAdmissionError::UnrelatedBinding(key)) => {
                        view.bindings.retain(|binding| binding.key != key);
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
        }
    }
    Ok(views)
}

struct BindAssociated<'a>(&'a ObjectType);
impl TypeFolder for BindAssociated<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        if matches!(ty, Type::Object(_)) {
            return ty;
        }
        let ty = fold_type_children(ty, self);
        if let Type::Projection {
            ty: base,
            trait_id,
            trait_args,
            assoc_type,
        } = &ty
        {
            if matches!(base.as_ref(), Type::ObjectSelf { depth: 0 }) {
                if let Some(binding) = self.0.bindings.iter().find(|b| {
                    b.key.member == assoc_type.assoc_type_id
                        && b.key.trait_ref.trait_id == *trait_id
                        && b.key.trait_ref.type_args == *trait_args
                }) {
                    return binding.ty.clone();
                }
            }
        }
        ty
    }
}

pub(crate) fn object_method_abi<P: HirPhase>(
    ty: &Type,
    bound: &TraitBound,
    declaration: &HirTraitFor<P>,
    signature: &HirFunctionSig,
    env: &TypeNormalizationEnv,
) -> Result<ObjectMethodAbi, String> {
    let Type::Object(object) = ty else {
        return Err("object method ABI requires a signature binder".into());
    };
    let receiver = signature
        .self_receiver
        .ok_or("static member is not an object slot")?;
    if declaration.generic_params.len() != bound.type_args.len() {
        return Err("object trait argument arity mismatch".into());
    }
    let self_param = declaration
        .target
        .as_ref()
        .map(|p| p.id)
        .unwrap_or(GenericParamId {
            owner: declaration.id,
            index: declaration.generic_params.len() as u32,
        });
    let mut substitution: HashMap<_, _> = declaration
        .generic_params
        .iter()
        .zip(&bound.type_args)
        .map(|(p, ty)| (p.id, ty.clone()))
        .collect();
    substitution.insert(self_param, Type::ObjectSelf { depth: 0 });
    if signature
        .generic_params
        .iter()
        .any(|p| !substitution.contains_key(&p.id))
    {
        return Err("generic virtual methods require the erased descriptor/dictionary ABI".into());
    }
    let bind = |ty: &Type| {
        let ty = crate::type_services::substitution::instantiate(ty, &substitution)
            .map_err(|e| e.to_string())?;
        TypeNormalizer::new(env)
            .normalize(&fold_type(ty, &mut BindAssociated(object)))
            .map_err(|e| e.to_string())
    };
    let mut params = signature
        .params
        .iter()
        .map(bind)
        .collect::<Result<Vec<_>, _>>()?;
    if params.is_empty() {
        return Err("object method has no receiver parameter".into());
    }
    params[0] = if receiver == ReceiverMode::Move {
        Type::Pointer(Box::new(ty.clone()))
    } else {
        Type::Reference {
            inner: Box::new(ty.clone()),
            mutable: receiver == ReceiverMode::Mut,
        }
    };
    let declared_ret = bind(&signature.ret)?;
    let (ret, result) = match declared_ret {
        Type::Reference { inner, mutable }
            if matches!(inner.as_ref(), Type::ObjectSelf { depth: 0 }) =>
        {
            if mutable && receiver != ReceiverMode::Mut {
                return Err("mutable Self return requires a mutable receiver".into());
            }
            (
                Type::Reference {
                    inner: Box::new(ty.clone()),
                    mutable,
                },
                MirObjectResult::BorrowedSelf { mutable },
            )
        }
        ret => (ret, MirObjectResult::Direct),
    };
    if params
        .iter()
        .skip(1)
        .chain((result == MirObjectResult::Direct).then_some(&ret))
        .any(|ty| crate::type_services::layout::TypeLayout::validate_runtime_type(ty).is_err())
    {
        return Err(
            "virtual method requires unsupported opened-Self or erased runtime evidence".into(),
        );
    }
    Ok(ObjectMethodAbi {
        receiver,
        params,
        ret,
        result,
    })
}

pub(crate) fn method_needs_erasure<P: HirPhase>(
    declaration: &HirTraitFor<P>,
    signature: &HirFunctionSig,
) -> bool {
    let self_id = declaration
        .target
        .as_ref()
        .map(|p| p.id)
        .unwrap_or(GenericParamId {
            owner: declaration.id,
            index: declaration.generic_params.len() as u32,
        });
    let hidden_self = |ty: &Type| {
        crate::type_services::visit::type_any(
            ty,
            |ty| matches!(ty, Type::Generic(id) if *id == self_id),
        )
    };
    let borrowed_self_result = matches!(&signature.ret, Type::Reference { inner, .. }
        if matches!(inner.as_ref(), Type::Generic(id) if *id == self_id));
    signature.generic_params.iter().any(|p| {
        p.id != self_id
            && !declaration
                .generic_params
                .iter()
                .any(|owner| owner.id == p.id)
    })
        // The concrete object ABI replaces only its receiver and a direct
        // borrowed-Self result. Other Self positions need scoped descriptors.
        || signature.params.iter().skip(1).any(hidden_self)
        || (!borrowed_self_result && hidden_self(&signature.ret))
}

pub(crate) fn erased_object_method_abi<P: HirPhase>(
    object_type: &Type,
    bound: &TraitBound,
    declaration: &HirTraitFor<P>,
    method: &HirFunctionSig,
    traits: &HashMap<crate::ids::DefId, HirTraitFor<P>>,
    orders: &std::collections::BTreeMap<crate::ids::DefId, Vec<crate::ids::DefId>>,
    env: &TypeNormalizationEnv,
    tc: &mut crate::type_context::TypeContext,
) -> Result<(crate::mir::MirErasedSignature, MirObjectResult), String> {
    use crate::mir::{MirErasedParameter, MirErasedType};
    let Type::Object(object) = object_type else {
        return Err("erased method requires an object signature".into());
    };
    let receiver = method
        .self_receiver
        .ok_or("erased virtual member has no receiver")?;
    let self_id = declaration
        .target
        .as_ref()
        .map(|p| p.id)
        .unwrap_or(GenericParamId {
            owner: declaration.id,
            index: declaration.generic_params.len() as u32,
        });
    let mut static_bindings: HashMap<_, _> = declaration
        .generic_params
        .iter()
        .zip(&bound.type_args)
        .map(|(p, ty)| (p.id, ty.clone()))
        .collect();
    static_bindings.insert(self_id, Type::ObjectSelf { depth: 0 });
    let mut parameters = vec![MirErasedParameter {
        source: self_id,
        kind: crate::type_services::kind::Kind::Type,
    }];
    parameters.extend(
        method
            .generic_params
            .iter()
            .filter(|p| !static_bindings.contains_key(&p.id))
            .map(|p| MirErasedParameter {
                source: p.id,
                kind: p.kind.clone(),
            }),
    );
    let indices = parameters
        .iter()
        .enumerate()
        .map(|(index, p)| (p.source, index as u32))
        .collect();
    let bind = |ty: &Type| {
        crate::type_services::substitution::instantiate(ty, &static_bindings)
            .map(|ty| fold_type(ty, &mut BindAssociated(object)))
            .map_err(|e| e.to_string())
    };
    let mut params = method
        .params
        .iter()
        .map(bind)
        .map(|ty| {
            ty.and_then(|ty| {
                MirErasedType::from_type(&ty, &indices, Some(0), tc, env)
                    .map_err(|e| format!("{e:?}"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if params.is_empty() {
        return Err("erased receiver parameter missing".into());
    }
    params[0] = if receiver == ReceiverMode::Move {
        MirErasedType::Parameter(0)
    } else {
        MirErasedType::Reference {
            inner: Box::new(MirErasedType::Parameter(0)),
            mutable: receiver == ReceiverMode::Mut,
        }
    };
    let ret = MirErasedType::from_type(&bind(&method.ret)?, &indices, Some(0), tc, env)
        .map_err(|e| format!("{e:?}"))?;
    let result = match &ret {
        MirErasedType::Reference { inner, mutable } if **inner == MirErasedType::Parameter(0) => {
            if *mutable && receiver != ReceiverMode::Mut {
                return Err("mutable Self result requires mutable access".into());
            }
            MirObjectResult::BorrowedSelf { mutable: *mutable }
        }
        _ => MirObjectResult::Direct,
    };
    let dictionaries = erased_bound_schemas(
        &parameters,
        &method.generic_bounds,
        &static_bindings,
        traits,
        orders,
        env,
        tc,
        true,
    )?;
    let mut layouts = parameters
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == crate::type_services::kind::Kind::Type)
        .map(|(i, _)| MirErasedType::Parameter(i as u32))
        .collect::<Vec<_>>();
    for ty in params.iter().chain(std::iter::once(&ret)) {
        ty.collect_layouts(&mut layouts, true);
    }
    let mut require_borrow_free = std::collections::BTreeSet::new();
    for dictionary in &dictionaries {
        for member in &dictionary.members {
            for ty in member.params.iter().chain(std::iter::once(&member.ret)) {
                ty.collect_layouts(&mut layouts, true);
            }
            // Dictionary calls have no symbolic origin transport yet; do not
            // let their inputs or returned values conceal escaping loans.
            require_borrow_free.extend(member.params.iter().skip(1).cloned());
            require_borrow_free.insert(member.ret.clone());
        }
    }
    Ok((
        crate::mir::MirErasedSignature {
            parameters,
            params,
            ret,
            dictionaries,
            layouts,
            require_borrow_free,
        },
        result,
    ))
}

pub(crate) fn erased_bound_schemas<P: HirPhase>(
    parameters: &[crate::mir::MirErasedParameter],
    bounds: &crate::hir::HirGenericBounds,
    static_bindings: &HashMap<GenericParamId, Type>,
    traits: &HashMap<crate::ids::DefId, HirTraitFor<P>>,
    orders: &std::collections::BTreeMap<crate::ids::DefId, Vec<crate::ids::DefId>>,
    env: &TypeNormalizationEnv,
    tc: &mut crate::type_context::TypeContext,
    witness: bool,
) -> Result<Vec<crate::mir::MirErasedDictionarySchema>, String> {
    use crate::mir::{MirErasedDictionaryMember, MirErasedDictionarySchema, MirErasedType};
    let indices = parameters
        .iter()
        .enumerate()
        .map(|(i, p)| (p.source, i as u32))
        .collect();
    let mut dictionaries = Vec::new();
    for (index, parameter) in parameters.iter().enumerate() {
        if witness && index == 0 {
            continue;
        }
        let Some(roots) = bounds.get(&parameter.source) else {
            continue;
        };
        let subject = Type::Generic(parameter.source);
        for bound in crate::traits::evidence::implied_trait_bounds(traits, &subject, roots) {
            let declaration = traits
                .get(&bound.trait_id)
                .ok_or("erased bound trait declaration missing")?;
            let args = bound
                .type_args
                .iter()
                .map(|ty| {
                    crate::type_services::substitution::instantiate(ty, static_bindings)
                        .map_err(|e| e.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut substitutions = static_bindings.clone();
            substitutions.extend(
                declaration
                    .generic_params
                    .iter()
                    .zip(&args)
                    .map(|(p, ty)| (p.id, ty.clone())),
            );
            let self_id = declaration
                .target
                .as_ref()
                .map(|p| p.id)
                .unwrap_or(GenericParamId {
                    owner: declaration.id,
                    index: declaration.generic_params.len() as u32,
                });
            substitutions.insert(self_id, subject.clone());
            let mut members = Vec::new();
            for member in orders
                .get(&bound.trait_id)
                .ok_or("erased bound lacks producer member layout")?
            {
                let signature = declaration
                    .signatures
                    .values()
                    .find(|sig| sig.id == *member)
                    .ok_or("erased dictionary member identity missing")?;
                let Some(receiver) = signature.self_receiver else {
                    continue;
                };
                if signature
                    .generic_params
                    .iter()
                    .any(|p| !substitutions.contains_key(&p.id))
                {
                    return Err(
                        "generic dictionary members require nested erased entry evidence".into(),
                    );
                }
                let mut convert = |ty: &Type| {
                    let ty = crate::type_services::substitution::instantiate(ty, &substitutions)
                        .map_err(|e| e.to_string())?;
                    MirErasedType::from_type(&ty, &indices, witness.then_some(0), tc, env)
                        .map_err(|e| format!("{e:?}"))
                };
                let mut params = signature
                    .params
                    .iter()
                    .map(&mut convert)
                    .collect::<Result<Vec<_>, _>>()?;
                if params.is_empty() {
                    return Err("dictionary receiver missing".into());
                }
                params[0] = MirErasedType::Parameter(index as u32);
                members.push(MirErasedDictionaryMember {
                    member_id: *member,
                    receiver,
                    params,
                    ret: convert(&signature.ret)?,
                });
            }
            let trait_args = args
                .iter()
                .map(|ty| {
                    MirErasedType::from_type(ty, &indices, witness.then_some(0), tc, env)
                        .map_err(|e| format!("{e:?}"))
                })
                .collect::<Result<_, _>>()?;
            dictionaries.push(MirErasedDictionarySchema {
                subject: MirErasedType::Parameter(index as u32),
                trait_id: bound.trait_id,
                trait_args,
                members,
            });
        }
    }
    Ok(dictionaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{AcceptedHir, HirTraitFor};
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::mir::MirErasedType;
    use crate::mono::Monomorphizer;
    use crate::types::GenericParamDecl;

    #[test]
    fn nongeneric_self_input_uses_erased_schema_and_preserves_native_abi_guard() {
        let trait_id = DefId::new(CrateId(0), LocalDefId(10));
        let member_id = DefId::new(CrateId(0), LocalDefId(11));
        let self_id = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let self_ref = Type::Reference {
            inner: Box::new(Type::Generic(self_id)),
            mutable: false,
        };
        let signature = HirFunctionSig {
            id: member_id,
            name: "sum".into(),
            generic_params: vec![],
            params: vec![self_ref.clone(), self_ref.clone()],
            ret: Type::I64,
            generic_bounds: Default::default(),
            self_receiver: Some(ReceiverMode::Shared),
            is_unsafe: false,
        };
        let declaration: HirTraitFor<AcceptedHir> = HirTraitFor {
            id: trait_id,
            name: "Sum".into(),
            generic_params: vec![],
            target: Some(GenericParamDecl::type_param(self_id, "Self")),
            predicates: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([("sum".into(), signature.clone())]),
        };
        let bound = TraitBound {
            trait_id,
            type_args: vec![],
        };
        let object = Type::Object(Box::new(ObjectType::new(bound.clone())));
        let mut mono = Monomorphizer::new();
        mono.identity_context = crate::mono::CompilationIdentityContext::new(
            CrateId(0),
            crate::products::ProductCrateIdentity::local("consumer".into()),
            std::collections::BTreeMap::from([(
                CrateId(1),
                crate::products::ProductCrateIdentity::local("producer".into()),
            )]),
        );
        assert!(method_needs_erasure(&declaration, &signature));
        assert!(
            object_method_abi(
                &object,
                &bound,
                &declaration,
                &signature,
                &mono.normalization_env,
            )
            .is_err(),
            "the ordinary concrete ABI must still reject hidden Self inputs"
        );
        mono.object_traits.insert(trait_id, declaration.clone());
        mono.producer_trait_members
            .insert(trait_id, vec![member_id]);
        let object_id = mono.ensure_object_schema(&object).unwrap();
        let schema = &mono.object_schemas[&object_id];
        assert!(schema.slots.is_empty());
        assert_eq!(schema.erased_slots.len(), 1);
        let slot = &schema.erased_slots[0];
        assert_eq!(slot.member_id, member_id);
        assert_eq!(slot.signature.parameters.len(), 1);
        let hidden_ref = MirErasedType::Reference {
            inner: Box::new(MirErasedType::Parameter(0)),
            mutable: false,
        };
        assert_eq!(slot.signature.params, vec![hidden_ref.clone(), hidden_ref]);
        assert_eq!(
            slot.signature.ret,
            MirErasedType::Concrete(mono.type_context.id_for_type(&Type::I64).unwrap())
        );
        let slot = slot.clone();

        let mut reader = signature;
        reader.params.truncate(1);
        assert!(!method_needs_erasure(&declaration, &reader));
        assert!(object_method_abi(
            &object,
            &bound,
            &declaration,
            &reader,
            &mono.normalization_env,
        )
        .is_ok());
        reader.ret = self_ref;
        assert!(!method_needs_erasure(&declaration, &reader));
        assert_eq!(
            object_method_abi(
                &object,
                &bound,
                &declaration,
                &reader,
                &mono.normalization_env,
            )
            .unwrap()
            .result,
            MirObjectResult::BorrowedSelf { mutable: false }
        );
        reader.ret = Type::Generic(self_id);
        assert!(method_needs_erasure(&declaration, &reader));
        reader.ret = Type::Tuple(vec![Type::Generic(self_id), Type::I64]);
        assert!(method_needs_erasure(&declaration, &reader));

        // Imported interface/body-provider rows share one canonical impl and
        // one native link. Selection must not count those rows as ambiguity.
        use crate::mono::hir_types::{HirBlock, HirFunction, HirImpl, HirParam};
        let impl_id = DefId::new(CrateId(1), LocalDefId(20));
        let method_id = DefId::new(CrateId(1), LocalDefId(21));
        let native_ref = Type::Reference {
            inner: Box::new(Type::I64),
            mutable: false,
        };
        let method = HirFunction {
            id: method_id,
            name: "sum".into(),
            generic_params: vec![],
            generic_bounds: Default::default(),
            params: (0..2)
                .map(|index| HirParam {
                    name: format!("input{index}"),
                    local_id: crate::ids::HirLocalId(index),
                    ty: native_ref.clone(),
                    mutable: false,
                    is_ref: false,
                })
                .collect(),
            ret_type: Type::I64,
            body: HirBlock {
                stmts: vec![],
                ty: Type::I64,
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Shared),
            is_unsafe: false,
        };
        let implementation = HirImpl {
            id: impl_id,
            owner: crate::hir::HirImplOwner::Named("Scalar".into()),
            type_name: "Scalar".into(),
            type_generics: vec![],
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
            trait_name: Some("Sum".into()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: Default::default(),
            methods: HashMap::from([("sum".into(), method.clone())]),
        };
        let origin = mono.method_instance_origin(&implementation, &method);
        let instance = mono.instances.intern(
            crate::mono::InstanceKey::new(origin.clone(), vec![]),
            |id| crate::mono::InstanceRecord {
                id,
                origin,
                substitution: vec![],
                symbols: crate::mono::InstanceSymbols::new("producer::sum", "canonical_native_sum"),
                declared: Some(method),
                provided_by_object: true,
                is_specialization: false,
            },
        );
        mono.effective_trait_methods
            .insert((impl_id, member_id), method_id);
        mono.trait_impls
            .insert(trait_id, vec![implementation.clone(), implementation]);
        let key = mono
            .materialize_erased_object_method(
                &Type::I64,
                &slot,
                &[crate::hir::HirObjectView {
                    trait_ref: bound,
                    origin: crate::hir::HirObjectWitnessOrigin::Impl {
                        impl_id,
                        substitution: vec![],
                    },
                }],
                &crate::lexer::Span::test(),
            )
            .unwrap();
        assert_eq!(key.definition, method_id);
        assert!(mono.instances.pre_mir_body(instance).is_none());
        assert!(matches!(
            &mono.erased.functions[&key].body.as_ref().unwrap().blocks[0].statements[0].value,
            crate::mir::MirErasedRvalue::NativeCall { target: crate::mir::MirCallableKey::Instance(selected), .. }
                if *selected == instance
        ));
    }
}
