use crate::hir::{
    HirMethodCallTarget, HirObjectCoercion, HirObjectEvidence, HirObjectView,
    HirObjectWitnessOrigin, HirSelectedMethodTarget, HirSelectedTraitMember, HirTraitDispatchKind,
};
use crate::ids::TypeId;
use crate::lexer::Span;
use crate::mir::{
    MirCallableKey, MirCallableSignature, MirObjectResult, MirObjectSchema, MirObjectSlot,
    MirPassMode, MirVtable, MirVtableId,
};
use crate::types::{GenericParamId, ReceiverMode, TraitBound, Type};

use super::{
    hir_types::{HirCallTarget, HirExpr, HirExprKind},
    Monomorphizer,
};

impl Monomorphizer {
    pub(super) fn materialize_owned_object_call(
        &mut self,
        owner: &HirExpr,
        call: &crate::hir::HirOwnedObjectCall,
        span: &Span,
    ) -> Result<(), String> {
        if matches!(
            call.method.target,
            HirSelectedMethodTarget::TraitMethod {
                dispatch: HirTraitDispatchKind::Opened(_),
                ..
            }
        ) {
            return Err("opened consuming calls require the enclosing scoped descriptor environment, not concrete owner materialization".into());
        }
        let object = self.ensure_object_schema(&call.object)?;
        let trait_args = call
            .method
            .trait_args()
            .iter()
            .map(|ty| self.intern_type(ty))
            .collect::<Vec<_>>();
        let concrete_slot = self.object_schemas[&object].slots.iter().position(|slot| {
            Some(slot.trait_id) == call.method.trait_id()
                && Some(slot.member_id) == call.method.method_id()
                && slot.trait_args == trait_args
        });
        let (slot, dictionaries) = if let Some(slot) = concrete_slot {
            let schema_slot = &self.object_schemas[&object].slots[slot];
            if schema_slot.receiver != ReceiverMode::Move
                || schema_slot.result != MirObjectResult::Direct
            {
                return Err("owned virtual call requires a consuming direct-result entry; hidden Self packing is unsupported".into());
            }
            (
                crate::mir::MirOwnedObjectSlot::Concrete(slot as u32),
                Vec::new(),
            )
        } else {
            let slot = self.object_schemas[&object]
                .erased_slots
                .iter()
                .position(|slot| {
                    Some(slot.trait_id) == call.method.trait_id()
                        && Some(slot.member_id) == call.method.method_id()
                        && slot.trait_args == trait_args
                })
                .ok_or("owned erased member is absent from its exact object schema")?;
            let schema_slot = &self.object_schemas[&object].erased_slots[slot];
            if schema_slot.receiver != ReceiverMode::Move
                || schema_slot.result != MirObjectResult::Direct
                || schema_slot.signature.params.first()
                    != Some(&crate::mir::MirErasedType::Parameter(0))
            {
                return Err(
                    "owned erased call requires a by-value payload receiver and direct result"
                        .into(),
                );
            }
            self.prepare_erased_virtual_call(object, slot as u32, &call.method, span)?;
            let parameters = self.object_schemas[&object].erased_slots[slot]
                .signature
                .parameters
                .clone();
            let type_args = parameters
                .iter()
                .skip(1)
                .map(|parameter| {
                    let binding = call
                        .method
                        .method_substitution
                        .iter()
                        .chain(&call.method.owner_substitution)
                        .find(|binding| binding.param == parameter.source)
                        .ok_or("owned erased argument lacks accepted substitution authority")?;
                    Ok(self.intern_type(&binding.ty))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let evidence = &self.erased_invocations[&crate::mir::MirErasedInvocationKey {
                object,
                slot: slot as u32,
                type_args: type_args.clone(),
            }];
            (
                crate::mir::MirOwnedObjectSlot::Erased {
                    slot: slot as u32,
                    type_args,
                },
                evidence.dictionaries.clone(),
            )
        };
        let (_, split, _) = self
            .monomorphize_static_method_call(
                &call.into_parts.owner_ty,
                &call.into_parts.method,
                std::slice::from_ref(owner),
                span.clone(),
            )
            .map_err(|e| format!("owner split specialization failed: {:?}", e.kind))?;
        let pointer = HirExpr {
            kind: HirExprKind::Unit,
            ty: Type::Pointer(Box::new(Type::U8)),
            span: span.clone(),
        };
        let state = HirExpr {
            kind: HirExprKind::Unit,
            ty: call.state.clone(),
            span: span.clone(),
        };
        let (_, release, _) = self
            .monomorphize_static_method_call(
                &call.release.owner_ty,
                &call.release.method,
                &[pointer, state],
                span.clone(),
            )
            .map_err(|e| format!("owner release specialization failed: {:?}", e.kind))?;
        let (
            super::hir_types::HirCallTarget::Instance(split),
            super::hir_types::HirCallTarget::Instance(release),
        ) = (split, release)
        else {
            return Err("owner protocol calls must materialize exact instances".into());
        };
        let key = crate::mir::MirOwnedObjectKey {
            owner: self.intern_type(&owner.ty),
            object,
            slot,
        };
        let plan = crate::mir::MirOwnedObjectPlan {
            state: self.intern_type(&call.state),
            into_parts: MirCallableKey::Instance(split),
            release: MirCallableKey::Instance(release),
            dictionaries,
        };
        if self
            .owned_object_calls
            .insert(key, plan.clone())
            .is_some_and(|old| old != plan)
        {
            return Err("conflicting owner protocol plans".into());
        }
        Ok(())
    }

    pub(super) fn import_object_abis(
        &mut self,
        crates: &crate::crate_system::CrateContext,
    ) -> Result<(), String> {
        for dependency in crates.extern_crates() {
            let abi = &dependency.metadata().interface().object_abi;
            abi.validate_shape()?;
            for (id, order) in &abi.trait_members {
                if self
                    .producer_trait_members
                    .insert(*id, order.clone())
                    .is_some_and(|old| old != *order)
                {
                    return Err("conflicting admitted producer trait layouts".into());
                }
            }
            for portable in &abi.schemas {
                let schema = portable.intern(&mut self.type_context, &self.normalization_env)?;
                if self
                    .object_schemas
                    .insert(schema.object, schema.clone())
                    .is_some_and(|old| old != schema)
                {
                    return Err("conflicting admitted producer object schemas".into());
                }
            }
        }
        Ok(())
    }
    pub(super) fn open_object_component(&self, ty: &Type, witness: &Type) -> Result<Type, String> {
        let instantiated = crate::type_services::substitution::instantiate_object_self(ty, witness)
            .map_err(|error| error.to_string())?;
        crate::type_services::normalize::TypeNormalizer::new(&self.normalization_env)
            .normalize(&instantiated)
            .map_err(|error| error.to_string())
    }
    pub(super) fn ensure_payload_schemas(
        &mut self,
        ty: &Type,
        seen: &mut std::collections::HashSet<Type>,
    ) -> Result<(), String> {
        if !seen.insert(ty.clone()) {
            return Ok(());
        }
        if matches!(ty, Type::Object(_)) {
            if !self.contains_generic(ty) {
                self.ensure_object_schema(ty)?;
            }
            return Ok(());
        }
        let mut nested = Vec::new();
        crate::type_services::visit::visit_type_children(ty, &mut |child: &Type| {
            nested.push(child.clone());
        });
        for child in nested {
            self.ensure_payload_schemas(&child, seen)?;
        }
        if let Type::Struct { id, args } | Type::Enum { id, args } = ty {
            let fields = self
                .nominal_field_types
                .get(id)
                .cloned()
                .unwrap_or_default();
            let substitution = args
                .iter()
                .enumerate()
                .map(|(index, ty)| {
                    (
                        GenericParamId {
                            owner: *id,
                            index: index as u32,
                        },
                        ty.clone(),
                    )
                })
                .collect();
            for field in fields {
                let field = crate::type_services::substitution::instantiate(&field, &substitution)
                    .map_err(|error| error.to_string())?;
                self.ensure_payload_schemas(&field, seen)?;
            }
        }
        Ok(())
    }
    pub(super) fn ensure_object_schema(&mut self, ty: &Type) -> Result<TypeId, String> {
        let Type::Object(object) = ty else {
            return Err("virtual dispatch requires an object signature".into());
        };
        let id = self.intern_type(ty);
        if self.object_schemas.contains_key(&id) {
            return Ok(id);
        }
        let mut bounds = std::iter::once(object.principal.clone())
            .chain(object.guarantees.iter().cloned())
            .map(|bound| {
                let interned = TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|ty| self.intern_type(ty))
                        .collect(),
                };
                (self.write_object_bound(&interned), bound)
            })
            .collect::<Vec<_>>();
        // Canonical product identity encoding, independent of importer crate/TypeId allocation.
        bounds.sort_by(|a, b| a.0.cmp(&b.0));
        let mut slots = Vec::new();
        let mut erased_slots = Vec::new();
        for (_, bound) in &bounds {
            let declaration = self
                .object_traits
                .get(&bound.trait_id)
                .cloned()
                .ok_or_else(|| "object trait interface missing".to_string())?;
            let order = match self.producer_trait_members.get(&bound.trait_id) {
                Some(order) => order.clone(),
                None if bound.trait_id.crate_id == self.identity_context.current_crate_id => {
                    crate::products::object_abi::declaration_member_order(&declaration.signatures)
                }
                None => {
                    return Err(
                        "imported object trait is missing its producer member layout".into(),
                    )
                }
            };
            for member in order {
                let sig = declaration
                    .signatures
                    .values()
                    .find(|sig| sig.id == member)
                    .ok_or("producer member layout references an unknown member")?;
                if sig.self_receiver.is_none() {
                    continue;
                }
                if super::object_schema::method_needs_erasure(&declaration, sig) {
                    let orders = self.erased_member_orders();
                    let (signature, result) = super::object_schema::erased_object_method_abi(
                        ty,
                        bound,
                        &declaration,
                        sig,
                        &self.object_traits,
                        &orders,
                        &self.normalization_env,
                        &mut self.type_context,
                    )?;
                    erased_slots.push(crate::mir::MirErasedObjectSlot {
                        trait_id: bound.trait_id,
                        member_id: sig.id,
                        trait_args: bound
                            .type_args
                            .iter()
                            .map(|ty| self.intern_type(ty))
                            .collect(),
                        receiver: sig.self_receiver.ok_or("erased receiver missing")?,
                        signature,
                        result,
                    });
                    continue;
                }
                let super::object_schema::ObjectMethodAbi {
                    receiver,
                    params,
                    ret,
                    result,
                } = super::object_schema::object_method_abi(
                    ty,
                    bound,
                    &declaration,
                    sig,
                    &self.normalization_env,
                )?;
                let param_ids = params
                    .iter()
                    .map(|ty| self.intern_type(ty))
                    .collect::<Vec<_>>();
                let ret = self.intern_type(&ret);
                let mut signature =
                    MirCallableSignature::from_type_ids(&param_ids, ret, MirPassMode::Direct);
                signature.params[0].pass_mode = if receiver == ReceiverMode::Move {
                    MirPassMode::Pointer
                } else {
                    MirPassMode::FatDirect
                };
                slots.push(MirObjectSlot {
                    trait_id: bound.trait_id,
                    member_id: sig.id,
                    trait_args: bound
                        .type_args
                        .iter()
                        .map(|ty| self.intern_type(ty))
                        .collect(),
                    receiver,
                    signature,
                    result,
                });
            }
        }
        // Insert before descending into the closure: supertrait views can share views.
        self.object_schemas.insert(
            id,
            MirObjectSchema {
                object: id,
                slots,
                erased_slots,
                views: Vec::new(),
            },
        );
        // Enumerate the finite lattice of admitted guarantee views of this
        // signature, not the set of callers in this compilation. A producer can
        // therefore satisfy a subset chosen by a separately compiled consumer.
        let mut candidates = std::collections::BTreeMap::new();
        for view in super::object_schema::admitted_object_views(
            ty,
            &self.object_traits,
            &self.normalization_env,
        )? {
            let view_id = self.intern_type(&view);
            let mut identity = String::new();
            self.write_type_id(&mut identity, view_id);
            candidates.insert(identity, view);
        }
        let mut views = Vec::new();
        for view in candidates.into_values() {
            views.push(self.ensure_object_schema(&view)?);
        }
        self.object_schemas.get_mut(&id).unwrap().views = views;
        Ok(id)
    }

    pub(super) fn materialize_object_coercion(
        &mut self,
        coercion: &HirObjectCoercion,
        span: &Span,
    ) -> Result<(), String> {
        let (Type::Reference { inner: object, .. } | Type::Pointer(object)) = &coercion.target
        else {
            return Err(
                "object coercion currently requires shared or mutable borrowed access".into(),
            );
        };
        let object_id = self.ensure_object_schema(object)?;
        match coercion
            .evidence
            .as_ref()
            .ok_or_else(|| "accepted object coercion is missing evidence".to_string())?
        {
            HirObjectEvidence::Concrete { source, views } => {
                if matches!(source, Type::Witness(_)) {
                    return Err("reclosing a witness requires the enclosing scoped descriptor environment, not a concrete impl table".into());
                }
                self.materialize_object_table(object_id, source, views, span)?;
            }
            HirObjectEvidence::Upcast { source, target } => {
                let source = self.ensure_object_schema(source)?;
                let target = self.ensure_object_schema(target)?;
                if source != target && !self.object_schemas[&source].views.contains(&target) {
                    return Err(
                        "upcast target is not an admitted guarantee subset of the source signature"
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }

    fn materialize_object_table(
        &mut self,
        object: TypeId,
        source: &Type,
        evidence: &[HirObjectView],
        span: &Span,
    ) -> Result<MirVtableId, String> {
        let concrete = self.intern_type(source);
        self.ensure_payload_schemas(source, &mut Default::default())?;
        if let Some((id, _)) = self
            .vtables
            .iter()
            .find(|(_, t)| t.object == object && t.concrete == concrete)
        {
            return Ok(*id);
        }
        if !crate::mir::object::object_runtime_type_is_valid_for_schemas(
            &self.object_schemas,
            &self.type_context,
            concrete,
        ) || matches!(source, Type::Slice(_) | Type::Str)
        {
            return Err("concrete object payload must have a validated sized layout".into());
        }
        let schema = self.object_schemas[&object].clone();
        for proof in evidence {
            if let HirObjectWitnessOrigin::Auto { role } = proof.origin {
                let provider = match role {
                    crate::language_items::LanguageItemRole::Sized => {
                        self.language_items.sized.as_ref().map(|p| p.trait_id)
                    }
                    crate::language_items::LanguageItemRole::Send => {
                        self.language_items.send.as_ref().map(|p| p.trait_id)
                    }
                    crate::language_items::LanguageItemRole::Sync => {
                        self.language_items.sync.as_ref().map(|p| p.trait_id)
                    }
                    _ => None,
                };
                if provider != Some(proof.trait_ref.trait_id) {
                    return Err(
                        "auto object evidence does not name its canonical protocol provider".into(),
                    );
                }
            }
        }
        let id = MirVtableId(self.vtables.len() as u32);
        self.vtables.insert(
            id,
            MirVtable {
                object,
                concrete,
                methods: Vec::new(),
                erased_methods: Vec::new(),
                views: Vec::new(),
                drop: None,
            },
        );
        let mut methods = Vec::new();
        for slot in schema.slots {
            let trait_args = slot
                .trait_args
                .iter()
                .map(|id| self.open_object_component(&self.type_for(*id), source))
                .collect::<Result<Vec<_>, _>>()?;
            let proof = evidence
                .iter()
                .find(|view| {
                    view.trait_ref.trait_id == slot.trait_id
                        && view
                            .trait_ref
                            .type_args
                            .iter()
                            .map(|ty| self.open_object_component(ty, source))
                            .collect::<Result<Vec<_>, _>>()
                            .is_ok_and(|args| args == trait_args)
                })
                .ok_or_else(|| "object method lacks its instantiated trait evidence".to_string())?;
            if let HirObjectWitnessOrigin::Callable { .. } = proof.origin {
                let (kind, _, _) = self
                    .language_items
                    .callable_protocols()
                    .find(|(_, trait_id, member_id)| {
                        *trait_id == slot.trait_id && *member_id == slot.member_id
                    })
                    .ok_or("callable witness does not name a canonical invocation member")?;
                let Type::Function {
                    callable_kind,
                    safety,
                    captures,
                    ..
                } = source
                else {
                    return Err(
                        "native callable object requires a concrete function/closure payload"
                            .into(),
                    );
                };
                if *callable_kind > kind
                    || crate::types::CallableKind::from_captures(captures) > *callable_kind
                {
                    return Err("native callable witness requires stronger access than its payload supports".into());
                }
                if *safety == crate::types::FunctionSafety::Unsafe
                    && !self
                        .object_traits
                        .get(&slot.trait_id)
                        .is_some_and(|definition| {
                            definition.signatures.values().any(|signature| {
                                signature.id == slot.member_id && signature.is_unsafe
                            })
                        })
                {
                    return Err(
                        "unsafe native callable cannot provide a safe object invocation member"
                            .into(),
                    );
                }
                methods.push(MirCallableKey::ObjectAdapter(
                    crate::mir::MirObjectAdapterKey::NativeCallable { concrete, kind },
                ));
                continue;
            }
            let target = match &proof.origin {
                HirObjectWitnessOrigin::Impl {
                    impl_id,
                    substitution,
                } => {
                    let method_id = *self
                        .effective_trait_methods
                        .get(&(*impl_id, slot.member_id))
                        .ok_or_else(|| "object impl/default entry missing".to_string())?;
                    let mut target = HirMethodCallTarget::impl_method(
                        *impl_id,
                        method_id,
                        Some(HirSelectedTraitMember {
                            trait_id: slot.trait_id,
                            member_id: slot.member_id,
                            trait_args: trait_args.clone(),
                        }),
                    );
                    target.owner_substitution = substitution
                        .iter()
                        .map(|binding| {
                            self.open_object_component(&binding.ty, source).map(|ty| {
                                crate::hir::HirTypeBinding {
                                    param: binding.param,
                                    ty,
                                }
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    target
                }
                HirObjectWitnessOrigin::Bound => HirMethodCallTarget {
                    target: HirSelectedMethodTarget::TraitMethod {
                        trait_id: slot.trait_id,
                        member_id: slot.member_id,
                        trait_args,
                        dispatch: HirTraitDispatchKind::TraitBound,
                    },
                    owner_substitution: Vec::new(),
                    method_substitution: Vec::new(),
                },
                HirObjectWitnessOrigin::Callable { .. } | HirObjectWitnessOrigin::Auto { .. } => {
                    return Err("marker-only evidence cannot supply a virtual method entry".into())
                }
            };
            let receiver = HirExpr {
                kind: HirExprKind::Unit,
                ty: source.clone(),
                span: span.clone(),
            };
            let args = slot
                .signature
                .params
                .iter()
                .skip(1)
                .map(|p| HirExpr {
                    kind: HirExprKind::Unit,
                    ty: self.type_for(p.semantic_ty),
                    span: span.clone(),
                })
                .collect::<Vec<_>>();
            let mut call = HirExpr {
                kind: HirExprKind::MethodCall(
                    Box::new(receiver.clone()),
                    String::new(),
                    args.clone(),
                    Some(slot.receiver),
                    target,
                ),
                ty: match slot.result {
                    MirObjectResult::Direct => self.type_for(slot.signature.ret.semantic_ty),
                    MirObjectResult::BorrowedSelf { mutable } => Type::Reference {
                        inner: Box::new(source.clone()),
                        mutable,
                    },
                },
                span: span.clone(),
            };
            let mut all_args = vec![receiver];
            all_args.extend(args);
            self.monomorphize_trait_method_call("", &all_args, &mut call)
                .map_err(|e| format!("object method materialization failed: {:?}", e.kind))?;
            let HirExprKind::Call(_, _, Some(HirCallTarget::Instance(instance))) = call.kind else {
                return Err("object entry did not materialize a resolved instance".into());
            };
            methods.push(if slot.receiver == ReceiverMode::Move {
                MirCallableKey::ObjectAdapter(crate::mir::MirObjectAdapterKey::ConsumingMethod {
                    concrete,
                    target: instance,
                })
            } else {
                MirCallableKey::Instance(instance)
            });
        }
        let mut views = Vec::new();
        let mut erased_methods = Vec::new();
        for slot in &schema.erased_slots {
            erased_methods
                .push(self.materialize_erased_object_method(source, slot, evidence, span)?);
        }
        for view in schema.views {
            views.push(self.materialize_object_table(view, source, evidence, span)?);
        }
        self.monomorphize_drop_for_type(source, Some(span.clone()));
        let drop = Some(MirCallableKey::ObjectAdapter(
            crate::mir::MirObjectAdapterKey::PayloadDrop(concrete),
        ));
        self.vtables.insert(
            id,
            MirVtable {
                object,
                concrete,
                methods,
                erased_methods,
                views,
                drop,
            },
        );
        Ok(id)
    }
}
