//! Producer-owned object interface ABI. Concrete tables/adapters stay in the
//! producer object; these records contain no process-local instance identities.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::hir::{HirFunctionSig, HirPhase, HirTraitFor};
use crate::ids::DefId;
use crate::mir::{
    MirCallableSignature, MirObjectResult, MirObjectSchema, MirObjectSlot, MirParamAbi,
    MirPassMode, MirReturnAbi,
};
use crate::type_context::TypeContext;
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::{ReceiverMode, Type};

pub const OBJECT_ABI_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductObjectAbi {
    pub version: u32,
    pub target: Option<String>,
    /// All declaration members, including static members, in producer order.
    /// Generic bodies use these templates when producing new closed schemas.
    pub trait_members: BTreeMap<DefId, Vec<DefId>>,
    pub schemas: Vec<ProductObjectSchema>,
}

impl Default for ProductObjectAbi {
    fn default() -> Self {
        Self {
            version: OBJECT_ABI_VERSION,
            target: None,
            trait_members: BTreeMap::new(),
            schemas: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductObjectSchema {
    pub object: Type,
    pub slots: Vec<ProductObjectSlot>,
    pub erased_slots: Vec<ProductErasedObjectSlot>,
    pub views: Vec<Type>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductErasedObjectSlot {
    pub trait_id: DefId,
    pub member_id: DefId,
    pub trait_args: Vec<Type>,
    pub receiver: ReceiverMode,
    pub signature: crate::mir::MirErasedSignature<Type>,
    pub result: MirObjectResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductObjectSlot {
    pub trait_id: DefId,
    pub member_id: DefId,
    /// Components of the schema's implicit hidden-Self binder.
    pub trait_args: Vec<Type>,
    pub receiver: ReceiverMode,
    pub params: Vec<ProductObjectParam>,
    pub ret: Type,
    pub abi_ret: Type,
    pub result: MirObjectResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductObjectParam {
    pub ty: Type,
    pub pass_mode: MirPassMode,
}

fn remap_erased_signature_ids<T: Ord>(
    signature: &mut crate::mir::MirErasedSignature<T>,
    remap: &mut impl FnMut(DefId) -> Result<DefId, String>,
) -> Result<(), String> {
    for parameter in &mut signature.parameters {
        parameter.source.owner = remap(parameter.source.owner)?;
    }
    for ty in signature
        .params
        .iter_mut()
        .chain(std::iter::once(&mut signature.ret))
        .chain(&mut signature.layouts)
    {
        ty.try_remap_nominals(remap)?;
    }
    let mut requirements = BTreeSet::new();
    for mut requirement in std::mem::take(&mut signature.require_borrow_free) {
        requirement.try_remap_nominals(remap)?;
        requirements.insert(requirement);
    }
    signature.require_borrow_free = requirements;
    for dictionary in &mut signature.dictionaries {
        dictionary.trait_id = remap(dictionary.trait_id)?;
        dictionary.subject.try_remap_nominals(remap)?;
        for argument in &mut dictionary.trait_args {
            argument.try_remap_nominals(remap)?;
        }
        for member in &mut dictionary.members {
            member.member_id = remap(member.member_id)?;
            for ty in member
                .params
                .iter_mut()
                .chain(std::iter::once(&mut member.ret))
            {
                ty.try_remap_nominals(remap)?;
            }
        }
    }
    Ok(())
}

pub fn host_object_target() -> String {
    inkwell::targets::TargetMachine::get_default_triple()
        .as_str()
        .to_string_lossy()
        .into_owned()
}

pub(crate) fn declaration_member_order(signatures: &HashMap<String, HirFunctionSig>) -> Vec<DefId> {
    let mut members = signatures
        .values()
        .map(|signature| signature.id)
        .collect::<Vec<_>>();
    // This runs at the defining producer, before any importer ID remapping.
    members.sort_by_key(|member| member.local);
    members
}

impl ProductObjectSchema {
    pub fn from_mir(schema: &MirObjectSchema, tc: &TypeContext) -> Result<Self, String> {
        let ty = |id| {
            tc.type_id_tree_is_valid(id)
                .then(|| tc.type_for(id))
                .ok_or_else(|| "object schema references an invalid final MIR type".to_string())
        };
        Ok(Self {
            object: ty(schema.object)?,
            erased_slots: schema
                .erased_slots
                .iter()
                .map(|slot| {
                    Ok(ProductErasedObjectSlot {
                        trait_id: slot.trait_id,
                        member_id: slot.member_id,
                        trait_args: slot
                            .trait_args
                            .iter()
                            .map(|id| ty(*id))
                            .collect::<Result<_, _>>()?,
                        receiver: slot.receiver,
                        signature: slot.signature.map_types(&mut |id| ty(*id))?,
                        result: slot.result,
                    })
                })
                .collect::<Result<_, String>>()?,
            views: schema
                .views
                .iter()
                .map(|id| ty(*id))
                .collect::<Result<_, _>>()?,
            slots: schema
                .slots
                .iter()
                .map(|slot| {
                    Ok(ProductObjectSlot {
                        trait_id: slot.trait_id,
                        member_id: slot.member_id,
                        trait_args: slot
                            .trait_args
                            .iter()
                            .map(|id| ty(*id))
                            .collect::<Result<_, _>>()?,
                        receiver: slot.receiver,
                        params: slot
                            .signature
                            .params
                            .iter()
                            .map(|p| {
                                Ok(ProductObjectParam {
                                    ty: ty(p.semantic_ty)?,
                                    pass_mode: p.pass_mode,
                                })
                            })
                            .collect::<Result<_, String>>()?,
                        ret: ty(slot.signature.ret.semantic_ty)?,
                        abi_ret: ty(slot.signature.ret.abi_ty)?,
                        result: slot.result,
                    })
                })
                .collect::<Result<_, String>>()?,
        })
    }

    pub(crate) fn intern(
        &self,
        tc: &mut TypeContext,
        env: &TypeNormalizationEnv,
    ) -> Result<MirObjectSchema, String> {
        let mut ty = |ty: &Type| {
            tc.intern_normalized_type(ty, env)
                .map_err(|error| error.to_string())
        };
        Ok(MirObjectSchema {
            object: ty(&self.object)?,
            erased_slots: self
                .erased_slots
                .iter()
                .map(|slot| {
                    Ok(crate::mir::MirErasedObjectSlot {
                        trait_id: slot.trait_id,
                        member_id: slot.member_id,
                        trait_args: slot
                            .trait_args
                            .iter()
                            .map(&mut ty)
                            .collect::<Result<_, _>>()?,
                        receiver: slot.receiver,
                        signature: slot.signature.map_types(&mut ty)?,
                        result: slot.result,
                    })
                })
                .collect::<Result<_, String>>()?,
            views: self.views.iter().map(&mut ty).collect::<Result<_, _>>()?,
            slots: self
                .slots
                .iter()
                .map(|slot| {
                    Ok(MirObjectSlot {
                        trait_id: slot.trait_id,
                        member_id: slot.member_id,
                        trait_args: slot
                            .trait_args
                            .iter()
                            .map(&mut ty)
                            .collect::<Result<_, _>>()?,
                        receiver: slot.receiver,
                        signature: MirCallableSignature {
                            params: slot
                                .params
                                .iter()
                                .map(|p| {
                                    Ok(MirParamAbi {
                                        semantic_ty: ty(&p.ty)?,
                                        pass_mode: p.pass_mode,
                                    })
                                })
                                .collect::<Result<_, String>>()?,
                            ret: MirReturnAbi {
                                semantic_ty: ty(&slot.ret)?,
                                abi_ty: ty(&slot.abi_ret)?,
                            },
                        },
                        result: slot.result,
                    })
                })
                .collect::<Result<_, String>>()?,
        })
    }

    pub fn try_remap_def_ids(
        &mut self,
        remap: &mut impl FnMut(DefId) -> Result<DefId, String>,
    ) -> Result<(), String> {
        self.object.try_remap_def_ids(remap)?;
        for slot in &mut self.erased_slots {
            slot.trait_id = remap(slot.trait_id)?;
            slot.member_id = remap(slot.member_id)?;
            for arg in &mut slot.trait_args {
                arg.try_remap_def_ids(remap)?;
            }
            slot.signature = slot.signature.map_types(&mut |ty| {
                let mut ty = ty.clone();
                ty.try_remap_def_ids(remap)?;
                Ok::<_, String>(ty)
            })?;
            remap_erased_signature_ids(&mut slot.signature, remap)?;
        }
        for view in &mut self.views {
            view.try_remap_def_ids(remap)?;
        }
        for slot in &mut self.slots {
            slot.trait_id = remap(slot.trait_id)?;
            slot.member_id = remap(slot.member_id)?;
            for argument in &mut slot.trait_args {
                argument.try_remap_def_ids(remap)?;
            }
            for param in &mut slot.params {
                param.ty.try_remap_def_ids(remap)?;
            }
            slot.ret.try_remap_def_ids(remap)?;
            slot.abi_ret.try_remap_def_ids(remap)?;
        }
        Ok(())
    }
}

impl ProductObjectAbi {
    pub fn try_remap_def_ids(
        &mut self,
        remap: &mut impl FnMut(DefId) -> Result<DefId, String>,
    ) -> Result<(), String> {
        let mut members = BTreeMap::new();
        for (trait_id, order) in &self.trait_members {
            let id = remap(*trait_id)?;
            let order = order
                .iter()
                .map(|id| remap(*id))
                .collect::<Result<Vec<_>, _>>()?;
            if members.insert(id, order).is_some() {
                return Err("object ABI trait identities collide after remapping".into());
            }
        }
        self.trait_members = members;
        for schema in &mut self.schemas {
            schema.try_remap_def_ids(remap)?;
        }
        Ok(())
    }

    pub fn validate_shape(&self) -> Result<(), String> {
        if self.version != OBJECT_ABI_VERSION {
            return Err("unsupported producer object ABI version".into());
        }
        if !self.schemas.is_empty() && self.target.as_deref() != Some(host_object_target().as_str())
        {
            return Err("producer object ABI target mismatch or missing target".into());
        }
        let mut objects = BTreeSet::new();
        for schema in &self.schemas {
            if !matches!(schema.object, Type::Object(_)) || !objects.insert(&schema.object) {
                return Err("duplicate or non-object producer schema".into());
            }
            for ty in std::iter::once(&schema.object).chain(&schema.views).chain(
                schema
                    .slots
                    .iter()
                    .flat_map(|s| s.params.iter().map(|p| &p.ty).chain([&s.ret, &s.abi_ret])),
            ) {
                if crate::type_services::substitution::has_free_object_self(ty)
                    || crate::type_services::visit::type_any(ty, |ty| {
                        matches!(
                            ty,
                            Type::Generic(_) | Type::Witness(_) | Type::TypeVar(_) | Type::Error
                        )
                    })
                {
                    return Err("unbound type in producer object schema".into());
                }
            }
            for slot in &schema.slots {
                if slot.trait_args.iter().any(|ty| {
                    crate::type_services::substitution::has_free_object_self_at_depth(ty, 1)
                        || crate::type_services::visit::type_any(ty, |ty| {
                            matches!(
                                ty,
                                Type::Generic(_)
                                    | Type::Witness(_)
                                    | Type::TypeVar(_)
                                    | Type::Error
                            )
                        })
                }) {
                    return Err("unbound schema trait argument".into());
                }
            }
            for slot in &schema.erased_slots {
                for argument in &slot.trait_args {
                    if crate::type_services::substitution::has_free_object_self_at_depth(
                        argument, 1,
                    ) || crate::type_services::visit::type_any(argument, |ty| {
                        matches!(
                            ty,
                            Type::Generic(_) | Type::Witness(_) | Type::TypeVar(_) | Type::Error
                        )
                    }) {
                        return Err("unbound erased schema trait argument".into());
                    }
                }
                slot.signature.map_types(&mut |ty| {
                    if crate::type_services::substitution::has_free_object_self(ty)
                        || crate::type_services::visit::type_any(ty, |ty| {
                            matches!(
                                ty,
                                Type::Generic(_)
                                    | Type::Witness(_)
                                    | Type::TypeVar(_)
                                    | Type::Error
                            )
                        })
                    {
                        return Err("unbound concrete leaf in erased schema".to_string());
                    }
                    Ok(ty.clone())
                })?;
            }
        }
        for schema in &self.schemas {
            if schema.views.iter().any(|view| !objects.contains(view)) {
                return Err("producer object schema has an undeclared view".into());
            }
        }
        for order in self.trait_members.values() {
            if order.iter().collect::<BTreeSet<_>>().len() != order.len() {
                return Err("duplicate producer trait member ordinal".into());
            }
        }
        Ok(())
    }

    pub(crate) fn validate_declarations<P: HirPhase>(
        &self,
        traits: &HashMap<DefId, HirTraitFor<P>>,
        orders: &BTreeMap<DefId, Vec<DefId>>,
        env: &TypeNormalizationEnv,
    ) -> Result<(), String> {
        self.validate_shape()?;
        for (id, order) in &self.trait_members {
            let declaration = traits
                .get(id)
                .ok_or("producer object ABI references an unknown trait")?;
            if order.len() != declaration.signatures.len()
                || order.iter().copied().collect::<BTreeSet<_>>()
                    != declaration.signatures.values().map(|s| s.id).collect()
            {
                return Err("producer trait member layout disagrees with its declaration".into());
            }
        }
        for schema in &self.schemas {
            let Type::Object(object) = &schema.object else {
                unreachable!()
            };
            let admitted = crate::traits::objects::admit_object(object, traits, env)
                .map_err(|error| error.to_string())?;
            if admitted != **object {
                return Err("producer object schema is not an admitted canonical signature".into());
            }
            let views =
                crate::mono::object_schema::admitted_object_views(&schema.object, traits, env)?;
            if views.len() != schema.views.len() || views != schema.views.iter().cloned().collect()
            {
                return Err("producer object schema view closure/bindings mismatch".into());
            }
            let mut expected_members = BTreeSet::new();
            for bound in std::iter::once(&object.principal).chain(object.guarantees.iter()) {
                let declaration = traits
                    .get(&bound.trait_id)
                    .ok_or("object ABI trait declaration missing")?;
                let order = orders
                    .get(&bound.trait_id)
                    .ok_or("producer object ABI member order missing")?;
                let expected = order
                    .iter()
                    .filter(|id| {
                        declaration.signatures.values().any(|s| {
                            s.id == **id
                                && s.self_receiver.is_some()
                                && !crate::mono::object_schema::method_needs_erasure(declaration, s)
                        })
                    })
                    .copied()
                    .collect::<Vec<_>>();
                let actual = schema
                    .slots
                    .iter()
                    .filter(|s| s.trait_id == bound.trait_id && s.trait_args == bound.type_args)
                    .map(|s| s.member_id)
                    .collect::<Vec<_>>();
                if actual != expected {
                    return Err("object schema member order/completeness mismatch".into());
                }
                for member in expected {
                    expected_members.insert((bound.clone(), member));
                }
                let expected = order
                    .iter()
                    .filter(|id| {
                        declaration.signatures.values().any(|sig| {
                            sig.id == **id
                                && sig.self_receiver.is_some()
                                && crate::mono::object_schema::method_needs_erasure(
                                    declaration,
                                    sig,
                                )
                        })
                    })
                    .copied()
                    .collect::<Vec<_>>();
                let actual = schema
                    .erased_slots
                    .iter()
                    .filter(|slot| {
                        slot.trait_id == bound.trait_id && slot.trait_args == bound.type_args
                    })
                    .map(|slot| slot.member_id)
                    .collect::<Vec<_>>();
                if actual != expected {
                    return Err("erased object schema member order/completeness mismatch".into());
                }
                for member in expected {
                    expected_members.insert((bound.clone(), member));
                }
            }
            if expected_members.len() != schema.slots.len() + schema.erased_slots.len() {
                return Err("object schema contains duplicate or unrelated slots".into());
            }
            for slot in &schema.slots {
                let declaration = &traits[&slot.trait_id];
                let signature = declaration
                    .signatures
                    .values()
                    .find(|s| s.id == slot.member_id)
                    .ok_or("object schema member does not belong to its trait")?;
                let bound = crate::types::TraitBound {
                    trait_id: slot.trait_id,
                    type_args: slot.trait_args.clone(),
                };
                let expected = crate::mono::object_schema::object_method_abi(
                    &schema.object,
                    &bound,
                    declaration,
                    signature,
                    env,
                )?;
                let receiver_mode = if expected.receiver == ReceiverMode::Move {
                    MirPassMode::Pointer
                } else {
                    MirPassMode::FatDirect
                };
                if slot.receiver != expected.receiver
                    || slot.result != expected.result
                    || slot.ret != expected.ret
                    || slot.abi_ret != expected.ret
                    || slot.params.len() != expected.params.len()
                    || slot.params.iter().zip(&expected.params).enumerate().any(
                        |(i, (param, ty))| {
                            &param.ty != ty
                                || param.pass_mode
                                    != if i == 0 {
                                        receiver_mode
                                    } else {
                                        MirPassMode::Direct
                                    }
                        },
                    )
                {
                    return Err("producer object slot signature/result adaptation mismatch".into());
                }
            }
            for slot in &schema.erased_slots {
                let declaration = &traits[&slot.trait_id];
                let method = declaration
                    .signatures
                    .values()
                    .find(|sig| sig.id == slot.member_id)
                    .ok_or("erased object member missing")?;
                let bound = crate::types::TraitBound {
                    trait_id: slot.trait_id,
                    type_args: slot.trait_args.clone(),
                };
                let mut context = TypeContext::new();
                let (expected, result) = crate::mono::object_schema::erased_object_method_abi(
                    &schema.object,
                    &bound,
                    declaration,
                    method,
                    traits,
                    orders,
                    env,
                    &mut context,
                )?;
                let expected =
                    expected.map_types(&mut |id| Ok::<_, String>(context.type_for(*id)))?;
                if expected != slot.signature
                    || result != slot.result
                    || method.self_receiver != Some(slot.receiver)
                {
                    return Err("producer erased slot ABI/evidence mismatch".into());
                }
            }
        }
        Ok(())
    }
}
