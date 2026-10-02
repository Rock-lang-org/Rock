//! Completely typed, contract-owned adapters. LLVM performs no impl selection.
use super::{MirBackendContract, MirCallableKey, MirNominalLayout, MirVariantLayoutFields};
use crate::ids::{DefId, InstanceId, TypeId};
use crate::type_context::TypeContext;
use crate::types::{CallableKind, Type};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MirObjectAdapterKey {
    PayloadDrop(TypeId),
    NativeCallable {
        concrete: TypeId,
        kind: CallableKind,
    },
    ConsumingMethod {
        concrete: TypeId,
        target: InstanceId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirObjectAdapter {
    PayloadDrop(MirPayloadDropPlan),
    NativeCallable {
        concrete: TypeId,
        kind: CallableKind,
        trait_id: DefId,
        member_id: DefId,
        release_environment: bool,
    },
    ConsumingMethod {
        concrete: TypeId,
        target: MirCallableKey,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MirPayloadDropPlan {
    pub ty: TypeId,
    pub user_drop: Option<MirCallableKey>,
    /// Children are in layout order; destruction runs in reverse field order.
    pub children: MirPayloadDropChildren,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirPayloadDropChildren {
    None,
    Fields(Vec<MirPayloadDropPlan>),
    Array {
        len: usize,
        element: Box<MirPayloadDropPlan>,
    },
    /// One plan per variant, using the existing enum's exact payload layout.
    Variants(Vec<MirPayloadDropPlan>),
    ClosureEnvironment {
        fields: Vec<MirPayloadDropPlan>,
        release: bool,
    },
}

impl MirPayloadDropPlan {
    pub fn has_drop(&self) -> bool {
        self.user_drop.is_some()
            || match &self.children {
                MirPayloadDropChildren::None => false,
                MirPayloadDropChildren::Fields(fields)
                | MirPayloadDropChildren::Variants(fields) => fields.iter().any(Self::has_drop),
                MirPayloadDropChildren::Array { len, element } => *len != 0 && element.has_drop(),
                MirPayloadDropChildren::ClosureEnvironment { fields, release } => {
                    *release || fields.iter().any(Self::has_drop)
                }
            }
    }
    pub fn requires_heap_free(&self) -> bool {
        match &self.children {
            MirPayloadDropChildren::None => false,
            MirPayloadDropChildren::Fields(fields) | MirPayloadDropChildren::Variants(fields) => {
                fields.iter().any(Self::requires_heap_free)
            }
            MirPayloadDropChildren::Array { len, element } => {
                *len != 0 && element.requires_heap_free()
            }
            MirPayloadDropChildren::ClosureEnvironment { fields, release } => {
                *release || fields.iter().any(Self::requires_heap_free)
            }
        }
    }
}

pub fn callable_environment_is_unique(ty: &Type) -> bool {
    matches!(ty, Type::Function { captures, .. } if !captures.is_empty()) && !ty.is_copy()
}

pub fn object_adapter_is_valid(
    contract: &MirBackendContract,
    tc: &TypeContext,
    declaration: &super::MirCallableDecl,
) -> bool {
    use super::{MirCallableKind, MirObjectAdapterKey, MirPassMode};
    let MirCallableKind::ObjectAdapter(adapter) = &declaration.kind else {
        return !matches!(declaration.key, MirCallableKey::ObjectAdapter(_));
    };
    let MirCallableKey::ObjectAdapter(key) = &declaration.key else {
        return false;
    };
    let signature = &declaration.signature;
    if signature
        .params
        .iter()
        .map(|p| p.semantic_ty)
        .chain([signature.ret.semantic_ty, signature.ret.abi_ty])
        .any(|id| !super::object_runtime_type_is_valid(contract, tc, id))
    {
        return false;
    }
    let ty = |id| tc.type_id_tree_is_valid(id).then(|| tc.type_for(id));
    let receiver = |concrete, mutable, consuming| {
        signature.params.first().is_some_and(|p| p.pass_mode == MirPassMode::Pointer && if consuming {
        matches!(ty(p.semantic_ty), Some(Type::Pointer(inner)) if tc.id_for_type(&inner) == Some(concrete))
    } else { matches!(ty(p.semantic_ty), Some(Type::Reference { inner, mutable: actual }) if actual == mutable && tc.id_for_type(&inner) == Some(concrete)) })
    };
    match (key, adapter) {
        (MirObjectAdapterKey::PayloadDrop(concrete), MirObjectAdapter::PayloadDrop(plan)) => {
            *concrete == plan.ty
                && receiver(*concrete, true, false)
                && signature.params.len() == 1
                && matches!(ty(signature.ret.semantic_ty), Some(Type::Unit))
                && signature.ret.abi_ty == signature.ret.semantic_ty
                && payload_drop_plan(contract, &mut tc.clone(), *concrete)
                    .is_ok_and(|expected| expected == *plan)
        }
        (
            MirObjectAdapterKey::NativeCallable { concrete, kind },
            MirObjectAdapter::NativeCallable {
                concrete: source,
                kind: selected,
                release_environment,
                ..
            },
        ) => {
            if concrete != source
                || kind != selected
                || !receiver(
                    *concrete,
                    *kind == CallableKind::FnMut,
                    *kind == CallableKind::FnOnce,
                )
                || signature.params.len() != 2
            {
                return false;
            }
            let Some(Type::Function {
                params,
                ret,
                callable_kind,
                captures,
                ..
            }) = ty(*concrete)
            else {
                return false;
            };
            let packed = match params.as_slice() {
                [] => Type::Unit,
                [param] => param.clone(),
                _ => Type::Tuple(params),
            };
            *release_environment
                == (*kind == CallableKind::FnOnce
                    && !matches!(ret.as_ref(), Type::Never)
                    && ty(*concrete).is_some_and(|ty| callable_environment_is_unique(&ty)))
                && CallableKind::from_captures(&captures) <= callable_kind
                && callable_kind <= *kind
                && ty(signature.params[1].semantic_ty) == Some(packed)
                && signature.params[1].pass_mode == MirPassMode::Direct
                && ty(signature.ret.semantic_ty) == Some(*ret)
                && signature.ret.abi_ty == signature.ret.semantic_ty
        }
        (
            MirObjectAdapterKey::ConsumingMethod { concrete, target },
            MirObjectAdapter::ConsumingMethod {
                concrete: source,
                target: selected,
            },
        ) => {
            let key = MirCallableKey::Instance(*target);
            concrete == source
                && *selected == key
                && receiver(*concrete, false, true)
                && contract.callable(&key).is_some_and(|callee| {
                    callee
                        .signature
                        .params
                        .first()
                        .is_some_and(|p| p.semantic_ty == *concrete)
                        && callee.signature.params.get(1..) == signature.params.get(1..)
                        && callee.signature.ret == signature.ret
                })
        }
        _ => false,
    }
}

pub fn payload_drop_plan(
    contract: &MirBackendContract,
    tc: &mut TypeContext,
    ty: TypeId,
) -> Result<MirPayloadDropPlan, String> {
    fn build(
        contract: &MirBackendContract,
        tc: &mut TypeContext,
        ty: TypeId,
        active: &mut std::collections::HashSet<TypeId>,
    ) -> Result<MirPayloadDropPlan, String> {
        if !super::object_runtime_type_is_valid(contract, tc, ty) || !active.insert(ty) {
            return Err("invalid or recursively unsized payload layout".into());
        }
        let structural = contract.normalize_type(tc, &tc.type_for(ty));
        let children = match &structural {
            Type::Tuple(fields) => {
                let mut plans = Vec::new();
                for field in fields {
                    let field = tc.intern_type(field);
                    plans.push(build(contract, tc, field, active)?);
                }
                MirPayloadDropChildren::Fields(plans)
            }
            Type::Array(element, len) => {
                let element = tc.intern_type(element);
                MirPayloadDropChildren::Array {
                    len: *len,
                    element: Box::new(build(contract, tc, element, active)?),
                }
            }
            Type::Function { captures, .. } if !captures.is_empty() => {
                let mut fields = Vec::new();
                for capture in captures {
                    let field = match capture.kind {
                        crate::types::CaptureKind::Move => capture.ty.clone(),
                        crate::types::CaptureKind::SharedBorrow
                        | crate::types::CaptureKind::MutableBorrow => Type::Reference {
                            inner: Box::new(capture.ty.clone()),
                            mutable: capture.kind == crate::types::CaptureKind::MutableBorrow,
                        },
                    };
                    let field = tc.intern_type(&field);
                    fields.push(build(contract, tc, field, active)?);
                }
                MirPayloadDropChildren::ClosureEnvironment {
                    fields,
                    release: callable_environment_is_unique(&structural),
                }
            }
            Type::Struct { id, args } | Type::Enum { id, args } => {
                let layout = contract
                    .nominal_layouts
                    .get(id)
                    .ok_or("payload nominal layout missing")?;
                let (params, fields, variants) = match layout {
                    MirNominalLayout::Struct {
                        fields,
                        generic_params,
                        ..
                    } => (
                        generic_params,
                        fields.iter().map(|(_, ty)| vec![*ty]).collect::<Vec<_>>(),
                        None,
                    ),
                    MirNominalLayout::Enum {
                        variants,
                        generic_params,
                        ..
                    } => (generic_params, Vec::new(), Some(variants)),
                };
                if params.len() != args.len() {
                    return Err("payload nominal type argument arity mismatch".into());
                }
                let substitution = params.iter().copied().zip(args.iter().cloned()).collect();
                let field_type = |id| {
                    if tc.type_id_tree_is_valid(id) {
                        Ok(tc.type_for(id).substitute_generics(&substitution))
                    } else {
                        Err("payload field type missing")
                    }
                };
                let types = if let Some(variants) = variants {
                    variants
                        .iter()
                        .map(|variant| match &variant.fields {
                            MirVariantLayoutFields::Unit => Ok(Type::Unit),
                            MirVariantLayoutFields::Positional(fields) if fields.len() == 1 => {
                                field_type(fields[0])
                            }
                            MirVariantLayoutFields::Positional(fields) => fields
                                .iter()
                                .map(|id| field_type(*id))
                                .collect::<Result<Vec<_>, _>>()
                                .map(Type::Tuple),
                            MirVariantLayoutFields::Named(fields) => fields
                                .iter()
                                .map(|(_, id)| field_type(*id))
                                .collect::<Result<Vec<_>, _>>()
                                .map(Type::Tuple),
                        })
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    fields
                        .iter()
                        .map(|field| field_type(field[0]))
                        .collect::<Result<Vec<_>, _>>()?
                };
                let mut plans = Vec::new();
                for field in types {
                    let field = tc.intern_type(&field);
                    plans.push(build(contract, tc, field, active)?);
                }
                if variants.is_some() {
                    MirPayloadDropChildren::Variants(plans)
                } else {
                    MirPayloadDropChildren::Fields(plans)
                }
            }
            Type::Object(_) | Type::ObjectSelf { .. } | Type::Str | Type::Slice(_) => {
                return Err("unsized payload drop requires an owner protocol".into())
            }
            _ => MirPayloadDropChildren::None,
        };
        active.remove(&ty);
        Ok(MirPayloadDropPlan {
            ty,
            user_drop: contract.drop_glue.get(&ty).cloned(),
            children,
        })
    }
    build(contract, tc, ty, &mut Default::default())
}
