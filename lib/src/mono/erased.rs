use super::{
    hir_types::{HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt},
    Monomorphizer,
};
use crate::hir::{
    HirMethodCallTarget, HirSelectedMethodTarget, HirTraitDispatchKind, HirVarTarget,
};
use crate::ids::{DefId, HirLocalId, TypeId};
use crate::lexer::Span;
use crate::mir::*;
use crate::types::{GenericParamId, ReceiverMode, Type};
use std::collections::{BTreeMap, HashMap};

impl Monomorphizer {
    pub(super) fn requires_scoped_entry(function: &HirFunction) -> bool {
        function.generic_params.is_empty()
            && function
                .params
                .iter()
                .map(|parameter| &parameter.ty)
                .chain(std::iter::once(&function.ret_type))
                .all(crate::type_services::facts::TypeFacts::is_codegen_concrete)
            && scoped_block_span(&function.body).is_some()
    }

    pub(super) fn lower_scoped_function(
        &mut self,
        function: &mut HirFunction,
    ) -> Result<bool, (Span, String)> {
        let Some(span) = scoped_block_span(&function.body) else {
            return Ok(false);
        };
        if !Self::requires_scoped_entry(function) {
            return Err((
                span,
                "scoped concrete wrapper has a nonconcrete public signature".into(),
            ));
        }
        let key = MirErasedKey {
            definition: function.id,
            static_args: Vec::new(),
        };
        let params = function
            .params
            .iter()
            .map(|parameter| MirErasedType::Concrete(self.intern_type(&parameter.ty)))
            .collect();
        let signature = MirErasedSignature {
            parameters: Vec::new(),
            params,
            ret: MirErasedType::Concrete(self.intern_type(&function.ret_type)),
            dictionaries: Vec::new(),
            layouts: Vec::new(),
            require_borrow_free: Default::default(),
        };
        let symbol = format!(
            "rock.scoped.{}",
            self.backend_symbol_for_origin(&super::InstanceOrigin::Function(function.id), &[])
        );
        self.erased.functions.insert(
            key.clone(),
            MirErasedFunction {
                key: key.clone(),
                symbol,
                linkage: MirLinkage::Internal,
                signature: signature.clone(),
                body: None,
            },
        );
        let body = ErasedBodyBuilder::new(self, signature, BTreeMap::new(), function)
            .and_then(|builder| builder.block(&function.body))
            .map_err(|error| (span.clone(), error))?;
        self.erased
            .functions
            .get_mut(&key)
            .ok_or_else(|| (span.clone(), "scoped entry declaration disappeared".into()))?
            .body = Some(body);
        self.erased.scoped_entries.insert(function.id, key);
        // The concrete wrapper transports arguments only; witnesses stay exclusively in the erased body.
        let arguments = function
            .params
            .iter()
            .map(|parameter| HirExpr {
                kind: HirExprKind::ResolvedVar(crate::hir::HirVarRef {
                    name: parameter.name.clone(),
                    target: HirVarTarget::Local(parameter.local_id),
                }),
                ty: parameter.ty.clone(),
                span: span.clone(),
            })
            .collect();
        let callee = HirExpr {
            kind: HirExprKind::ResolvedVar(crate::hir::HirVarRef {
                name: function.name.clone(),
                target: HirVarTarget::Function(function.id),
            }),
            ty: Type::function(
                function
                    .params
                    .iter()
                    .map(|parameter| parameter.ty.clone())
                    .collect(),
                function.ret_type.clone(),
            ),
            span: span.clone(),
        };
        function.body = HirBlock {
            ty: function.ret_type.clone(),
            stmts: vec![HirStmt::Return(Some(HirExpr {
                kind: HirExprKind::Call(
                    Box::new(callee),
                    arguments,
                    Some(crate::hir::HirCallTarget::Function(function.id)),
                ),
                ty: function.ret_type.clone(),
                span,
            }))],
        };
        Ok(true)
    }
    pub(super) fn erased_member_orders(&self) -> BTreeMap<DefId, Vec<DefId>> {
        let mut orders = self.producer_trait_members.clone();
        for declaration in self
            .object_traits
            .values()
            .filter(|declaration| declaration.id.crate_id == self.identity_context.current_crate_id)
        {
            orders.insert(
                declaration.id,
                crate::products::object_abi::declaration_member_order(&declaration.signatures),
            );
        }
        orders
    }

    pub(super) fn prepare_erased_virtual_call(
        &mut self,
        object: TypeId,
        slot: u32,
        target: &HirMethodCallTarget,
        span: &Span,
    ) -> Result<Option<Type>, String> {
        let signature = self.object_schemas[&object].erased_slots[slot as usize]
            .signature
            .clone();
        if target.method_substitution.len() != signature.parameters.len() - 1 {
            return Err("generic virtual canonical method substitution arity disagrees".into());
        }
        let mut bindings = vec![None];
        let mut type_args = Vec::new();
        for parameter in signature.parameters.iter().skip(1) {
            let matches = target
                .method_substitution
                .iter()
                .filter(|binding| binding.param == parameter.source)
                .collect::<Vec<_>>();
            let [binding] = matches.as_slice() else {
                return Err("generic virtual canonical method binding absent/duplicated".into());
            };
            if target
                .owner_substitution
                .iter()
                .any(|binding| binding.param == parameter.source)
            {
                return Err("generic virtual local binder conflicts with owner evidence".into());
            }
            if self.contains_generic(&binding.ty) {
                return Err("generic virtual call from an erased body requires explicit nested dispatch evidence".into());
            }
            bindings.push(Some(binding.ty.clone()));
            type_args.push(self.intern_type(&binding.ty));
        }
        if signature.ret.contains_parameter(0)
            && !matches!(
                self.object_schemas[&object].erased_slots[slot as usize].result,
                MirObjectResult::BorrowedSelf { .. }
            )
        {
            return Err("by-value/nested Self results require an opened or owner-provided result destination".into());
        }
        let key = MirErasedInvocationKey {
            object,
            slot,
            type_args: type_args.clone(),
        };
        let mut dictionaries = Vec::new();
        for required in &signature.dictionaries {
            dictionaries.push(self.materialize_erased_dictionary(required, &bindings, span)?);
        }
        // Only exact selected static producers may refine a nominal result
        // binder. Dictionary schemas and implementation selection stay nominal;
        // measured storage travels separately in this invocation's descriptors.
        let mut measured = BTreeMap::new();
        for (required, dictionary) in signature.dictionaries.iter().zip(&dictionaries) {
            let selected = self.erased.dictionaries[dictionary].clone();
            for (member, actual) in required.members.iter().zip(&selected.members) {
                let MirErasedType::Parameter(index) = member.ret else {
                    continue;
                };
                if member.receiver.is_some()
                    || !member.binder.parameters.is_empty()
                    || member.ret != required.subject
                    || index == 0
                    || signature
                        .params
                        .iter()
                        .any(|ty| ty.contains_parameter(index))
                {
                    continue;
                }
                let Some(MirCallableKey::Instance(instance)) = actual.target.native() else {
                    continue;
                };
                let Some(body) = self.instances.pre_mir_body(*instance).or_else(|| {
                    self.instances
                        .record(*instance)
                        .and_then(|record| record.declared.as_ref())
                }) else {
                    continue;
                };
                let storage = body.ret_type.clone();
                if !matches!(
                    storage,
                    Type::StructRepresentation { .. } | Type::EnumRepresentation { .. }
                ) {
                    continue;
                }
                let semantic = bindings
                    .get(index as usize)
                    .and_then(Option::as_ref)
                    .ok_or("static result binder is unbound")?;
                if crate::mir::backend_contract::semantic_nominal_type(storage.clone()) != *semantic
                {
                    return Err(
                        "static producer storage disagrees with the nominal result binder".into(),
                    );
                }
                if measured
                    .insert(index, storage.clone())
                    .is_some_and(|previous| previous != storage)
                {
                    return Err(
                        "selected static producers require incompatible result storage".into(),
                    );
                }
            }
        }
        for (index, storage) in measured {
            let storage_id = self.intern_type(&storage);
            *type_args
                .get_mut(index as usize - 1)
                .ok_or("static result binder has no invocation argument")? = storage_id;
            *bindings
                .get_mut(index as usize)
                .ok_or("static result binder has no storage binding")? = Some(storage);
        }
        let result = signature
            .ret
            .instantiate_optional(&bindings, &self.type_context)
            .ok()
            .filter(|ty| {
                matches!(
                    ty,
                    Type::StructRepresentation { .. } | Type::EnumRepresentation { .. }
                )
            });
        for layout in &signature.layouts {
            if *layout == MirErasedType::Parameter(0) {
                continue;
            }
            let ty = layout
                .instantiate_optional(&bindings, &self.type_context)
                .map_err(|e| format!("erased layout evidence: {e:?}; layout {layout:?}, canonical parameters {:?}, finalized method substitution {:?}", signature.parameters, target.method_substitution))?;
            if crate::type_services::layout::TypeLayout::sizedness(&ty, &|_| {
                crate::type_services::layout::Sizedness::Unknown
            }) != crate::type_services::layout::Sizedness::Sized
            {
                return Err("erased entry requires a sized value descriptor; unsized application evidence is not implemented".into());
            }
            self.ensure_payload_schemas(&ty, &mut Default::default())?;
            self.intern_type(&ty);
            self.monomorphize_drop_for_type(&ty, Some(span.clone()));
        }
        if self.erased_invocations.contains_key(&key) {
            return Ok(result);
        }
        self.erased_invocations.insert(
            key,
            MirErasedCall {
                target: MirErasedCallTarget::Virtual(MirVirtualTarget { object, slot }),
                type_args,
                dictionaries,
            },
        );
        Ok(result)
    }

    fn materialize_erased_dictionary(
        &mut self,
        required: &MirErasedDictionarySchema,
        bindings: &[Option<Type>],
        span: &Span,
    ) -> Result<MirDictionaryId, String> {
        let source = required
            .subject
            .instantiate_optional(bindings, &self.type_context)
            .map_err(|e| format!("dictionary subject: {e:?}"))?;
        let arguments = required
            .trait_args
            .iter()
            .map(|ty| {
                ty.instantiate_optional(bindings, &self.type_context)
                    .map_err(|e| format!("dictionary argument: {e:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let subject = self.intern_type(&source);
        let trait_args = arguments
            .iter()
            .map(|ty| self.intern_type(ty))
            .collect::<Vec<_>>();
        if let Some((id, _)) = self.erased.dictionaries.iter().find(|(_, dictionary)| {
            dictionary.subject == subject
                && dictionary.trait_id == required.trait_id
                && dictionary.trait_args == trait_args
        }) {
            return Ok(*id);
        }
        let mut members = Vec::new();
        for member in &required.members {
            if !member.binder.parameters.is_empty() {
                let parent_arguments = bindings
                    .iter()
                    .map(|binding| {
                        binding
                            .as_ref()
                            .map(|ty| self.intern_type(ty))
                            .map(MirErasedType::Concrete)
                    })
                    .collect::<Vec<_>>();
                // Only parents actually referenced by this member must be bound
                // (the hidden virtual Self evidence has no concrete substitution).
                let parent_arguments = parent_arguments
                    .into_iter()
                    .enumerate()
                    .map(|(index, ty)| ty.unwrap_or(MirErasedType::Parameter(index as u32)))
                    .collect::<Vec<_>>();
                let mut schema = MirErasedDictionarySchema {
                    subject: required.subject.clone(),
                    trait_id: required.trait_id,
                    trait_args: required.trait_args.clone(),
                    members: vec![member.clone()],
                }
                .substitute(&parent_arguments, &self.type_context)
                .map_err(|error| format!("polymorphic parent binding: {error:?}"))?;
                for ty in &mut schema.members[0].params {
                    *ty = self.simplify_erased_type(ty)?;
                }
                schema.members[0].ret = self.simplify_erased_type(&schema.members[0].ret)?;
                schema.members[0].params[0] = MirErasedType::Concrete(subject);
                let implementation_map = self
                    .trait_impls
                    .values()
                    .flatten()
                    .map(|imp| (imp.id, imp.clone()))
                    .collect::<HashMap<_, _>>();
                let (implementation, substitution) =
                    crate::selection::SelectionService::select_declared_dictionary_impl(
                        &self.object_traits,
                        &implementation_map,
                        &crate::hir::HirGenericBounds::new(),
                        &source,
                        required.trait_id,
                        &arguments,
                    )
                    .map_err(|error| format!("polymorphic dictionary selection: {error:?}"))?;
                let implementation = implementation.clone();
                let key = self.materialize_polymorphic_member(
                    &implementation,
                    &schema.members[0],
                    &[],
                    &[],
                    &substitution,
                )?;
                members.push(MirConcreteDictionaryMember {
                    member_id: member.member_id,
                    receiver: member.receiver,
                    target: MirDictionaryMemberTarget::Polymorphic(key),
                });
                continue;
            }
            let target = if let Some((kind, _, _)) =
                self.language_items
                    .callable_protocols()
                    .find(|(_, trait_id, method_id)| {
                        *trait_id == required.trait_id && *method_id == member.member_id
                    }) {
                if let Type::Function {
                    callable_kind,
                    params,
                    ret,
                    safety,
                    ..
                } = &source
                {
                    if *callable_kind > kind {
                        return Err("callable dictionary exceeds its receiver capability".into());
                    }
                    if !safety.permits_call_without_unsafe() {
                        return Err("unsafe callable requires explicit effect evidence, not a safe callable dictionary".into());
                    }
                    let packed = match params.as_slice() {
                        [] => Type::Unit,
                        [param] => param.clone(),
                        _ => Type::Tuple(params.clone()),
                    };
                    let receiver = match kind {
                        crate::types::CallableKind::Fn => crate::types::ReceiverMode::Shared,
                        crate::types::CallableKind::FnMut => crate::types::ReceiverMode::Mut,
                        crate::types::CallableKind::FnOnce => crate::types::ReceiverMode::Move,
                    };
                    let member_params = member
                        .params
                        .iter()
                        .map(|ty| {
                            ty.instantiate_optional(bindings, &self.type_context)
                                .map_err(|e| format!("callable dictionary parameter: {e:?}"))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let member_ret = member
                        .ret
                        .instantiate_optional(bindings, &self.type_context)
                        .map_err(|e| format!("callable dictionary result: {e:?}"))?;
                    if member.receiver != Some(receiver)
                        || arguments != vec![packed.clone(), *ret.clone()]
                        || member_params != vec![source.clone(), packed]
                        || member_ret != **ret
                    {
                        return Err(
                            "native callable does not match the canonical dictionary signature"
                                .into(),
                        );
                    }
                    MirCallableKey::ObjectAdapter(MirObjectAdapterKey::NativeCallable {
                        concrete: subject,
                        kind,
                    })
                } else {
                    self.materialize_dictionary_method(
                        &source,
                        required.trait_id,
                        &arguments,
                        member,
                        bindings,
                        span,
                    )?
                }
            } else {
                self.materialize_dictionary_method(
                    &source,
                    required.trait_id,
                    &arguments,
                    member,
                    bindings,
                    span,
                )?
            };
            members.push(MirConcreteDictionaryMember {
                member_id: member.member_id,
                receiver: member.receiver,
                target: MirDictionaryMemberTarget::Native(target),
            });
        }
        if members.is_empty() {
            let auto_sized = self
                .language_items
                .sized
                .as_ref()
                .is_some_and(|role| role.trait_id == required.trait_id);
            let mut proofs = self
                .trait_impls
                .get(&required.trait_id)
                .into_iter()
                .flatten()
                .filter(|implementation| {
                    let Some(mut substitution) =
                        crate::selection::impl_receiver_pattern_substitution(
                            implementation,
                            &source,
                            self.trait_impls.values().flatten(),
                        )
                    else {
                        return false;
                    };
                    implementation.trait_arg_types.len() == arguments.len()
                        && implementation.trait_arg_types.iter().zip(&arguments).all(
                            |(pattern, argument)| {
                                crate::selection::type_pattern_matches(
                                    pattern,
                                    argument,
                                    &mut substitution,
                                )
                            },
                        )
                })
                .map(|implementation| implementation.id)
                .collect::<Vec<_>>();
            proofs.sort();
            proofs.dedup();
            if !auto_sized && proofs.len() != 1 {
                return Err("marker dictionary lacks a unique compile-time witness".into());
            }
        }
        self.monomorphize_drop_for_type(&source, Some(span.clone()));
        let id = MirDictionaryId(self.erased.dictionaries.len() as u32);
        let impls = self
            .trait_impls
            .values()
            .flatten()
            .map(|imp| (imp.id, imp.clone()))
            .collect::<HashMap<_, _>>();
        let assumed = crate::hir::HirGenericBounds::new();
        if let Ok((implementation, _)) =
            crate::selection::SelectionService::select_declared_dictionary_impl(
                &self.object_traits,
                &impls,
                &assumed,
                &source,
                required.trait_id,
                &arguments,
            )
        {
            let methods = required
                .members
                .iter()
                .map(|m| {
                    self.effective_trait_methods
                        .get(&(implementation.id, m.member_id))
                        .copied()
                })
                .collect::<Option<Vec<_>>>();
            if let Some(methods) = methods {
                let mut schema = required.clone();
                let mut convert = |ty: &MirErasedType| -> Result<MirErasedType, String> {
                    if let MirErasedType::MemberParameter(_) = ty {
                        return Ok(ty.clone());
                    }
                    let concrete = ty
                        .instantiate_optional(bindings, &self.type_context)
                        .map_err(|e| format!("{e:?}"))?;
                    Ok(MirErasedType::Concrete(self.intern_type(&concrete)))
                };
                schema.subject = convert(&schema.subject)?;
                schema.trait_args = schema
                    .trait_args
                    .iter()
                    .map(&mut convert)
                    .collect::<Result<_, _>>()?;
                for member in &mut schema.members {
                    member.params = member
                        .params
                        .iter()
                        .map(&mut convert)
                        .collect::<Result<_, _>>()?;
                    member.ret = convert(&member.ret)?;
                }
                for member in &mut schema.members {
                    member.borrowing = dictionary_member_borrow_contract(
                        &member.binder,
                        &member.params,
                        &member.ret,
                        member.receiver,
                        &self.type_context,
                    )
                    .map_err(|e| format!("{e:?}"))?;
                }
                self.erased.dictionary_witnesses.insert(
                    id,
                    MirDictionaryWitness {
                        implementation: implementation.id,
                        schema,
                        methods,
                    },
                );
            }
        }
        self.erased.dictionaries.insert(
            id,
            MirConcreteDictionary {
                subject,
                trait_id: required.trait_id,
                trait_args,
                members,
            },
        );
        Ok(id)
    }

    fn materialize_polymorphic_member(
        &mut self,
        implementation: &super::hir_types::HirImpl,
        member: &MirErasedDictionaryMember,
        captures: &[MirErasedParameter],
        bounds: &[MirErasedDictionarySchema],
        substitution: &HashMap<GenericParamId, Type>,
    ) -> Result<MirErasedKey, String> {
        let trait_id = implementation
            .trait_id
            .ok_or("polymorphic entry is not a trait impl")?;
        let method_id = *self
            .effective_trait_methods
            .get(&(implementation.id, member.member_id))
            .ok_or("polymorphic effective member authority absent")?;
        let method = implementation
            .methods
            .values()
            .find(|method| method.id == method_id)
            .cloned()
            .ok_or("polymorphic accepted method absent")?;
        if method.is_unsafe
            || method.self_receiver != member.receiver
            || !polymorphic_impl_member_bounds_are_captured(implementation, &method)
        {
            return Err(
                "polymorphic impl member bounds/receiver require additional authority".into(),
            );
        }
        let locals = method
            .generic_params
            .iter()
            .filter(|p| {
                !implementation
                    .type_generics
                    .iter()
                    .any(|parent| parent.id == p.id)
            })
            .collect::<Vec<_>>();
        if locals.len() != member.binder.parameters.len()
            || locals
                .iter()
                .zip(&member.binder.parameters)
                .any(|(actual, canonical)| actual.kind != canonical.kind)
        {
            return Err("polymorphic impl/member binder arity or kinds disagree".into());
        }
        let static_args = if !captures.is_empty() {
            vec![]
        } else {
            implementation
                .type_generics
                .iter()
                .map(|p| {
                    substitution
                        .get(&p.id)
                        .cloned()
                        .ok_or("polymorphic owner substitution incomplete")
                })
                .collect::<Result<Vec<_>, _>>()?
                .iter()
                .map(|ty| self.intern_type(ty))
                .collect::<Vec<_>>()
        };
        let key = MirErasedKey {
            definition: method_id,
            static_args,
        };
        let mut signature = member
            .entry_signature(captures)
            .map_err(|error| format!("{error:?}"))?;
        signature.dictionaries = bounds.to_vec();
        let mut variables = captures
            .iter()
            .enumerate()
            .map(|(index, p)| (p.source, index as u32))
            .collect::<BTreeMap<_, _>>();
        if signature.params.len() != method.params.len() {
            return Err("polymorphic impl/member value arity disagrees".into());
        }
        for (expected, actual) in signature.params.iter().zip(&method.params) {
            let actual = crate::type_services::substitution::instantiate(&actual.ty, substitution)
                .map_err(|error| error.to_string())?;
            bind_erased_parameters(expected, &actual, &mut variables, &self.type_context)?;
        }
        let ret = crate::type_services::substitution::instantiate(&method.ret_type, substitution)
            .map_err(|error| error.to_string())?;
        bind_erased_parameters(&signature.ret, &ret, &mut variables, &self.type_context)?;
        for (index, (actual, canonical)) in locals.iter().zip(&member.binder.parameters).enumerate()
        {
            let ordinal = (captures.len() + index) as u32;
            if variables.get(&actual.id) != Some(&ordinal) {
                return Err(
                    "polymorphic method binder order disagrees with canonical signature".into(),
                );
            }
            if variables
                .insert(canonical.source, ordinal)
                .is_some_and(|old| old != ordinal)
            {
                return Err(
                    "polymorphic canonical local conflicts with an accepted capture or binder"
                        .into(),
                );
            }
        }
        let authority = MirErasedMemberEntry {
            implementation: implementation.id,
            trait_id,
            method_id,
            captures: captures.to_vec(),
            member: member.clone(),
        };
        if let Some(existing) = self.erased.member_entries.get(&key) {
            if existing != &authority
                || self
                    .erased
                    .functions
                    .get(&key)
                    .is_none_or(|entry| entry.signature != signature)
            {
                return Err("polymorphic entry key conflicts with selected authority".into());
            }
            return Ok(key);
        }
        let imported = self.imported_member_symbols.get(&method_id).cloned();
        if let Some((symbol, link)) = &imported {
            let imported_authority = link
                .entry
                .map_types(&mut |ty| Ok::<_, String>(self.intern_type(ty)))?;
            if imported_authority != authority {
                return Err(
                    "polymorphic dictionary link record disagrees with accepted member ABI".into(),
                );
            }
            self.erased
                .imported_member_links
                .insert(key.clone(), (symbol.clone(), imported_authority));
        }
        let symbol = imported
            .as_ref()
            .map(|(symbol, _)| symbol.clone())
            .unwrap_or_else(|| {
                format!(
                    "rock.erased.member.{}",
                    self.backend_symbol_for_origin(
                        &super::InstanceOrigin::ImplMethod {
                            owner: self.impl_owner_identity(implementation),
                            method: method_id
                        },
                        &key.static_args
                    )
                )
            });
        self.erased.member_entries.insert(key.clone(), authority);
        self.erased.functions.insert(
            key.clone(),
            MirErasedFunction {
                key: key.clone(),
                symbol,
                linkage: if imported.is_some()
                    || (captures.is_empty() && implementation.type_generics.is_empty())
                {
                    MirLinkage::External
                } else {
                    MirLinkage::Internal
                },
                signature: signature.clone(),
                body: None,
            },
        );
        if imported.is_some() {
            return Ok(key);
        }
        if method.body.stmts.is_empty() {
            return Err(
                "polymorphic member lacks an accepted body or exact producer link record".into(),
            );
        }
        let static_types = substitution
            .iter()
            .map(|(id, ty)| (*id, self.intern_type(ty)))
            .collect();
        let body = self.substitute_block(&method.body, &static_types);
        let body = ErasedBodyBuilder::new(self, signature, variables, &method)?
            .for_entry(key.clone())
            .block(&body)?;
        self.erased
            .functions
            .get_mut(&key)
            .ok_or("polymorphic entry declaration disappeared")?
            .body = Some(body);
        Ok(key)
    }

    pub(super) fn prepare_polymorphic_members(
        &mut self,
        program: &super::hir_types::HirProgram,
    ) -> Result<(), String> {
        let implementations = program
            .impls_in_order()
            .map(|(_, implementation)| implementation.clone())
            .collect::<Vec<_>>();
        for implementation in implementations {
            if implementation.id.crate_id != self.identity_context.current_crate_id
                || !implementation.type_generics.is_empty()
                || !implementation.bounds.is_empty()
                || !implementation.bounds.predicates.is_empty()
                || !implementation.associated_types.is_empty()
            {
                continue;
            }
            let Some(trait_id) = implementation.trait_id else {
                continue;
            };
            let crate::hir::HirImplReceiverPattern::Exact(source) =
                &implementation.receiver_pattern
            else {
                continue;
            };
            if !crate::type_services::facts::TypeFacts::is_codegen_concrete(source) {
                continue;
            }
            let declaration = self
                .object_traits
                .get(&trait_id)
                .ok_or("polymorphic trait authority absent")?;
            if !declaration.associated_types.is_empty() || !declaration.predicates.is_empty() {
                continue;
            }
            let subject = GenericParamId {
                owner: implementation.id,
                index: 0,
            };
            let parameters = vec![MirErasedParameter {
                source: subject,
                kind: crate::type_services::kind::Kind::Type,
            }];
            let bounds = crate::hir::HirGenericBounds::from_iter([(
                subject,
                vec![crate::types::TraitBound {
                    trait_id,
                    type_args: implementation.trait_arg_types.clone(),
                }],
            )]);
            // Unsupported exported generic shapes are not promoted to an ABI.
            // A requested dictionary reports the specific schema diagnostic.
            let Ok(mut schemas) = super::object_schema::erased_bound_schemas(
                &parameters,
                &bounds,
                &HashMap::new(),
                &self.object_traits,
                &self.erased_member_orders(),
                &self.normalization_env,
                &mut self.type_context,
                false,
            ) else {
                continue;
            };
            let source_id = self.intern_type(source);
            let Some(schema) = schemas.pop() else {
                continue;
            };
            let schema = schema
                .substitute(&[MirErasedType::Concrete(source_id)], &self.type_context)
                .map_err(|error| {
                    format!(
                        "polymorphic producer schema {:?}: {error:?}",
                        implementation.id
                    )
                })?;
            for member in &schema.members {
                if member.binder.parameters.is_empty() {
                    continue;
                }
                self.materialize_polymorphic_member(
                    &implementation,
                    member,
                    &[],
                    &[],
                    &HashMap::new(),
                )?;
            }
        }
        Ok(())
    }

    fn materialize_dictionary_method(
        &mut self,
        source: &Type,
        trait_id: DefId,
        trait_args: &[Type],
        member: &MirErasedDictionaryMember,
        bindings: &[Option<Type>],
        span: &Span,
    ) -> Result<MirCallableKey, String> {
        if member.receiver.is_none() {
            let implementations = self
                .trait_impls
                .values()
                .flatten()
                .map(|implementation| (implementation.id, implementation.clone()))
                .collect::<HashMap<_, _>>();
            let (implementation, substitution) =
                crate::selection::SelectionService::select_declared_dictionary_impl(
                    &self.object_traits,
                    &implementations,
                    &crate::hir::HirGenericBounds::new(),
                    source,
                    trait_id,
                    trait_args,
                )
                .map_err(|error| format!("static dictionary selection: {error:?}"))?;
            let method_id = *self
                .effective_trait_methods
                .get(&(implementation.id, member.member_id))
                .ok_or("static dictionary effective member missing")?;
            let method = implementation
                .methods
                .values()
                .find(|method| method.id == method_id)
                .ok_or("static dictionary accepted method missing")?;
            if method.self_receiver.is_some()
                || method.is_unsafe
                || !member.binder.parameters.is_empty()
                || method.generic_params.iter().any(|p| {
                    !implementation
                        .type_generics
                        .iter()
                        .any(|owner| owner.id == p.id)
                })
            {
                return Err(
                    "static dictionary leaf requires the exact safe nongeneric static header"
                        .into(),
                );
            }
            let target = HirMethodCallTarget {
                target: HirSelectedMethodTarget::ImplMethod {
                    impl_id: implementation.id,
                    method_id,
                    selected_trait: Some(crate::hir::HirSelectedTraitMember {
                        trait_id,
                        member_id: member.member_id,
                        trait_args: trait_args.to_vec(),
                    }),
                },
                owner_substitution: implementation
                    .type_generics
                    .iter()
                    .map(|p| {
                        substitution
                            .get(&p.id)
                            .cloned()
                            .map(|ty| crate::hir::HirTypeBinding { param: p.id, ty })
                            .ok_or("static dictionary owner binding missing")
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                method_substitution: vec![],
            };
            let args = member
                .params
                .iter()
                .map(|ty| {
                    ty.instantiate_optional(bindings, &self.type_context)
                        .map(|ty| HirExpr {
                            kind: HirExprKind::Unit,
                            ty,
                            span: span.clone(),
                        })
                        .map_err(|e| format!("static dictionary argument: {e:?}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let (_, target, ret) = self
                .monomorphize_static_method_call(source, &target, &args, span.clone())
                .map_err(|error| format!("static dictionary leaf: {:?}", error.kind))?;
            if crate::mir::backend_contract::semantic_nominal_type(ret)
                != member
                    .ret
                    .instantiate_optional(bindings, &self.type_context)
                    .map_err(|e| format!("static result: {e:?}"))?
            {
                return Err("static dictionary native result disagrees with Self authority".into());
            }
            let crate::hir::HirCallTarget::Instance(instance) = target else {
                return Err("static dictionary native instance absent".into());
            };
            return Ok(MirCallableKey::Instance(instance));
        }
        let receiver = HirExpr {
            kind: HirExprKind::Unit,
            ty: source.clone(),
            span: span.clone(),
        };
        let args = member
            .params
            .iter()
            .skip(1)
            .map(|ty| {
                ty.instantiate_optional(bindings, &self.type_context)
                    .map(|ty| HirExpr {
                        kind: HirExprKind::Unit,
                        ty,
                        span: span.clone(),
                    })
                    .map_err(|e| format!("dictionary parameter: {e:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let target = HirMethodCallTarget {
            target: HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id: member.member_id,
                trait_args: trait_args.to_vec(),
                dispatch: HirTraitDispatchKind::TraitBound,
            },
            owner_substitution: vec![],
            method_substitution: vec![],
        };
        let ret = member
            .ret
            .instantiate_optional(bindings, &self.type_context)
            .map_err(|e| format!("dictionary result: {e:?}"))?;
        let mut call = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(receiver.clone()),
                String::new(),
                args.clone(),
                member.receiver,
                target,
            ),
            ty: ret,
            span: span.clone(),
        };
        let mut all_args = vec![receiver];
        all_args.extend(args);
        self.monomorphize_trait_method_call("", &all_args, &mut call)
            .map_err(|error| format!("dictionary selection failed: {:?}", error.kind))?;
        match call.kind {
            HirExprKind::Call(_, _, Some(crate::hir::HirCallTarget::Instance(id))) => {
                Ok(MirCallableKey::Instance(id))
            }
            _ => Err("dictionary did not materialize a constant callable key".into()),
        }
    }

    pub(super) fn materialize_erased_object_method(
        &mut self,
        source: &Type,
        slot: &MirErasedObjectSlot,
        evidence: &[crate::hir::HirObjectView],
        span: &Span,
    ) -> Result<MirErasedKey, String> {
        let trait_args = slot
            .trait_args
            .iter()
            .map(|id| self.open_object_component(&self.type_for(*id), source))
            .collect::<Result<Vec<_>, _>>()?;
        let proof = evidence
            .iter()
            .find(|proof| {
                proof.trait_ref.trait_id == slot.trait_id
                    && proof
                        .trait_ref
                        .type_args
                        .iter()
                        .map(|ty| self.open_object_component(ty, source))
                        .collect::<Result<Vec<_>, _>>()
                        .is_ok_and(|args| args == trait_args)
            })
            .ok_or("generic object entry lacks its trait witness")?;
        let implementations = self
            .trait_impls
            .get(&slot.trait_id)
            .cloned()
            .unwrap_or_default();
        let mut selected = Vec::new();
        for implementation in implementations {
            if let crate::hir::HirObjectWitnessOrigin::Impl { impl_id, .. } = proof.origin {
                if implementation.id != impl_id {
                    continue;
                }
            }
            let Some(mut substitution) = crate::selection::impl_receiver_pattern_substitution(
                &implementation,
                source,
                self.trait_impls.values().flatten(),
            ) else {
                continue;
            };
            if implementation.trait_arg_types.len() != trait_args.len()
                || !implementation.trait_arg_types.iter().zip(&trait_args).all(
                    |(expected, actual)| {
                        crate::selection::type_pattern_matches(expected, actual, &mut substitution)
                    },
                )
            {
                continue;
            }
            selected.push((implementation, substitution));
        }
        selected.sort_by_key(|(implementation, _)| implementation.id);
        selected.dedup_by(|duplicate, canonical| {
            if duplicate.0.id != canonical.0.id
                || duplicate.1 != canonical.1
                || duplicate.0.receiver_pattern != canonical.0.receiver_pattern
                || duplicate.0.trait_arg_types != canonical.0.trait_arg_types
                || self.impl_owner_identity(&duplicate.0) != self.impl_owner_identity(&canonical.0)
            {
                return false;
            }
            // A validated interface row and its body provider are the same
            // implementation, not competing candidates. Keep the actual body
            // when a runtime-generic member needs it; native links need headers.
            if let Some(method_id) = self
                .effective_trait_methods
                .get(&(canonical.0.id, slot.member_id))
            {
                let has_body = |implementation: &super::hir_types::HirImpl| {
                    implementation
                        .methods
                        .values()
                        .find(|method| method.id == *method_id)
                        .map(|method| !method.body.stmts.is_empty())
                };
                if has_body(&duplicate.0) > has_body(&canonical.0) {
                    std::mem::swap(&mut duplicate.0, &mut canonical.0);
                }
            }
            true
        });
        if selected.len() != 1 {
            return Err("erased object method requires one canonical selected impl".into());
        }
        let (implementation, mut substitution) = selected.pop().ok_or("erased impl missing")?;
        let declaration = &self.object_traits[&slot.trait_id];
        let self_id = declaration
            .target
            .as_ref()
            .map(|p| p.id)
            .unwrap_or(GenericParamId {
                owner: declaration.id,
                index: declaration.generic_params.len() as u32,
            });
        substitution.insert(self_id, source.clone());
        substitution.extend(
            declaration
                .generic_params
                .iter()
                .zip(&trait_args)
                .map(|(p, ty)| (p.id, ty.clone())),
        );
        let method_id = *self
            .effective_trait_methods
            .get(&(implementation.id, slot.member_id))
            .ok_or("erased effective method/default missing")?;
        let method = implementation
            .methods
            .values()
            .find(|method| method.id == method_id)
            .cloned()
            .ok_or("erased method body provider missing")?;
        let source_id = self.intern_type(source);
        let mut static_args = vec![source_id];
        for parameter in &implementation.type_generics {
            static_args.push(
                self.intern_type(
                    substitution
                        .get(&parameter.id)
                        .ok_or("erased owner substitution incomplete")?,
                ),
            );
        }
        let key = MirErasedKey {
            definition: method.id,
            static_args,
        };
        if self.erased.functions.contains_key(&key) {
            return Ok(key);
        }
        let mut signature = slot.signature.clone();
        let mut map = (0..signature.parameters.len())
            .map(|i| MirErasedType::Parameter(i as u32))
            .collect::<Vec<_>>();
        map[0] = MirErasedType::Concrete(source_id);
        for ty in &mut signature.params {
            *ty = self.simplify_erased_type(&ty.substitute(&map).map_err(|e| format!("{e:?}"))?)?;
        }
        signature.ret = self.simplify_erased_type(
            &signature
                .ret
                .substitute(&map)
                .map_err(|e| format!("{e:?}"))?,
        )?;
        for layout in &mut signature.layouts {
            if *layout != MirErasedType::Parameter(0) {
                *layout = self.simplify_erased_type(
                    &layout.substitute(&map).map_err(|e| format!("{e:?}"))?,
                )?;
            }
        }
        for dictionary in &mut signature.dictionaries {
            *dictionary = dictionary
                .substitute(&map, &self.type_context)
                .map_err(|e| format!("{e:?}"))?;
        }
        if signature.params.len() != method.params.len() {
            return Err("impl parameter count disagrees with virtual schema".into());
        }
        // Establish a bijection before adding the declaration's canonical aliases.
        let mut variables = BTreeMap::new();
        for (expected, actual) in signature
            .params
            .iter()
            .skip(1)
            .zip(method.params.iter().skip(1))
        {
            let actual = crate::type_services::substitution::instantiate(&actual.ty, &substitution)
                .map_err(|e| e.to_string())?;
            bind_erased_parameters(expected, &actual, &mut variables, &self.type_context)?;
        }
        let actual_return =
            crate::type_services::substitution::instantiate(&method.ret_type, &substitution)
                .map_err(|e| e.to_string())?;
        bind_erased_parameters(
            &signature.ret,
            &actual_return,
            &mut variables,
            &self.type_context,
        )?;
        let runtime_parameters = method
            .generic_params
            .iter()
            .filter(|parameter| !substitution.contains_key(&parameter.id))
            .collect::<Vec<_>>();
        if runtime_parameters.len() != signature.parameters.len() - 1
            || runtime_parameters
                .iter()
                .zip(signature.parameters.iter().skip(1))
                .enumerate()
                .any(|(index, (actual, canonical))| {
                    actual.kind != canonical.kind
                        || variables.get(&actual.id) != Some(&((index + 1) as u32))
                })
        {
            return Err("accepted impl runtime binder order/kind disagrees with its declared virtual member".into());
        }
        for (index, parameter) in signature.parameters.iter().enumerate().skip(1) {
            if variables
                .insert(parameter.source, index as u32)
                .is_some_and(|old| old != index as u32)
            {
                return Err("impl runtime parameter conflicts with its selected member".into());
            }
        }
        let symbol = format!(
            "rock.erased.{}",
            self.backend_symbol_for_origin(
                &super::InstanceOrigin::ImplMethod {
                    owner: self.impl_owner_identity(&implementation),
                    method: method.id
                },
                &key.static_args
            )
        );
        self.erased.functions.insert(
            key.clone(),
            MirErasedFunction {
                key: key.clone(),
                symbol,
                linkage: MirLinkage::Internal,
                signature: signature.clone(),
                body: None,
            },
        );
        if signature.parameters.len() == 1
            && signature.dictionaries.is_empty()
            && method.generic_params.is_empty()
            && crate::type_services::facts::TypeFacts::is_codegen_concrete(source)
            && signature
                .params
                .iter()
                .chain(std::iter::once(&signature.ret))
                .all(|ty| {
                    ty.instantiate(&[], &self.type_context).is_ok_and(|ty| {
                        crate::type_services::facts::TypeFacts::is_codegen_concrete(&ty)
                    })
                })
            && key.static_args.iter().all(|id| {
                crate::type_services::facts::TypeFacts::is_concrete_type_argument(
                    &self.type_for(*id),
                )
            })
        {
            // No method-owned runtime parameters remain. Compile the exact
            // selected implementation through ordinary MIR, then bridge its ABI.
            let native_args = signature
                .params
                .iter()
                .enumerate()
                .map(|(index, ty)| {
                    let ty = ty
                        .instantiate(&[], &self.type_context)
                        .map_err(|error| format!("native erased parameter: {error:?}"))?;
                    Ok(HirExpr {
                        kind: HirExprKind::Unit,
                        ty: if index == 0 { source.clone() } else { ty },
                        span: span.clone(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let receiver = native_args
                .first()
                .cloned()
                .ok_or("native erased receiver missing")?;
            let target = HirMethodCallTarget {
                target: HirSelectedMethodTarget::ImplMethod {
                    impl_id: implementation.id,
                    method_id: method.id,
                    selected_trait: Some(crate::hir::HirSelectedTraitMember {
                        trait_id: slot.trait_id,
                        member_id: slot.member_id,
                        trait_args: trait_args.clone(),
                    }),
                },
                owner_substitution: implementation
                    .type_generics
                    .iter()
                    .map(|parameter| {
                        substitution
                            .get(&parameter.id)
                            .cloned()
                            .map(|ty| crate::hir::HirTypeBinding {
                                param: parameter.id,
                                ty,
                            })
                            .ok_or("native erased owner substitution missing".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                method_substitution: Vec::new(),
            };
            let ret = signature
                .ret
                .instantiate(&[], &self.type_context)
                .map_err(|error| format!("native erased result: {error:?}"))?;
            let mut call = HirExpr {
                kind: HirExprKind::MethodCall(
                    Box::new(receiver),
                    String::new(),
                    native_args.iter().skip(1).cloned().collect(),
                    Some(slot.receiver),
                    target,
                ),
                ty: ret,
                span: span.clone(),
            };
            self.monomorphize_trait_method_call("", &native_args, &mut call)
                .map_err(|error| format!("native erased specialization: {:?}", error.kind))?;
            let HirExprKind::Call(_, _, Some(crate::hir::HirCallTarget::Instance(instance))) =
                call.kind
            else {
                return Err("native erased bridge lacks its exact selected instance".into());
            };
            let mut locals = signature.params.clone();
            let destination = MirErasedLocal(locals.len() as u32);
            locals.push(signature.ret.clone());
            let operand = |index: u32, ty: &MirErasedType| {
                let place = MirErasedPlace::local(MirErasedLocal(index));
                if matches!(ty, MirErasedType::Concrete(id) if self.type_for(*id).is_copy()) {
                    MirErasedOperand::Copy(place)
                } else {
                    MirErasedOperand::Move(place)
                }
            };
            let args = signature
                .params
                .iter()
                .enumerate()
                .map(|(index, ty)| operand(index as u32, ty))
                .collect();
            let returned = operand(destination.0, &signature.ret);
            self.erased
                .functions
                .get_mut(&key)
                .ok_or("native erased declaration disappeared")?
                .body = Some(MirErasedBody {
                locals,
                mutable_locals: Default::default(),
                scopes: Vec::new(),
                blocks: vec![MirErasedBasicBlock {
                    statements: vec![MirErasedAssignment {
                        destination,
                        value: MirErasedRvalue::NativeCall {
                            target: MirCallableKey::Instance(instance),
                            args,
                        },
                    }],
                    terminator: MirErasedTerminator::Return(returned),
                }],
            });
            return Ok(key);
        }
        let static_types = substitution
            .iter()
            .map(|(id, ty)| (*id, self.intern_type(ty)))
            .collect();
        let body = self.substitute_block(&method.body, &static_types);
        let code = ErasedBodyBuilder::new(self, signature, variables, &method)?
            .for_entry(key.clone())
            .block(&body)?;
        self.erased
            .functions
            .get_mut(&key)
            .ok_or("erased declaration disappeared")?
            .body = Some(code);
        let _ = span;
        Ok(key)
    }

    fn simplify_erased_type(&mut self, ty: &MirErasedType) -> Result<MirErasedType, String> {
        if let Ok(ty) = ty.instantiate(&[], &self.type_context) {
            return Ok(MirErasedType::Concrete(self.intern_type(&ty)));
        }
        Ok(ty.clone())
    }

    fn materialize_dictionary_template(
        &mut self,
        implementation: &super::hir_types::HirImpl,
        _span: &Span,
    ) -> Result<MirDictionaryFactoryTemplate, String> {
        if let Some(template) = self.erased.dictionary_templates.get(&implementation.id) {
            return Ok(template.clone());
        }
        let crate::hir::HirImplReceiverPattern::Exact(Type::Generic(subject)) =
            &implementation.receiver_pattern
        else {
            return Err(
                "selected dictionary factories currently require a first-order generic subject"
                    .into(),
            );
        };
        let trait_id = implementation
            .trait_id
            .ok_or("dictionary factory is not a trait impl")?;
        let declaration = self
            .object_traits
            .get(&trait_id)
            .ok_or("dictionary trait unavailable")?;
        if !declaration.associated_types.is_empty()
            || !implementation.associated_types.is_empty()
            || !declaration.predicates.is_empty()
        {
            return Err(
                "associated dictionary factory members require additional authority".into(),
            );
        }
        let parameters = implementation
            .type_generics
            .iter()
            .map(|p| MirErasedParameter {
                source: p.id,
                kind: p.kind.clone(),
            })
            .collect::<Vec<_>>();
        if parameters.is_empty()
            || parameters
                .iter()
                .any(|p| p.kind != crate::type_services::kind::Kind::Type)
        {
            return Err("dictionary factory captures require Type-kind parameters".into());
        }
        let bounds = super::object_schema::erased_bound_schemas(
            &parameters,
            &implementation.bounds,
            &HashMap::new(),
            &self.object_traits,
            &self.erased_member_orders(),
            &self.normalization_env,
            &mut self.type_context,
            false,
        )?;
        for bound in &bounds {
            let declaration = self
                .object_traits
                .get(&bound.trait_id)
                .ok_or("dictionary child trait unavailable")?;
            if !declaration.associated_types.is_empty() || !declaration.predicates.is_empty() {
                return Err(
                    "associated/related child dictionaries require additional evidence".into(),
                );
            }
        }
        for predicate in &implementation.bounds.predicates {
            let crate::types::Predicate::Trait {
                subject: Type::Generic(param),
                trait_id,
                args,
            } = predicate
            else {
                return Err(
                    "compound selected impl bounds require explicit dictionary capture authority"
                        .into(),
                );
            };
            if !implementation.bounds.get(param).is_some_and(|roots| {
                roots.contains(&crate::types::TraitBound {
                    trait_id: *trait_id,
                    type_args: args.clone(),
                })
            }) {
                return Err(
                    "selected impl predicate is absent from canonical bound captures".into(),
                );
            }
        }
        let result_bounds = crate::hir::HirGenericBounds::from_iter([(
            *subject,
            vec![crate::types::TraitBound {
                trait_id,
                type_args: implementation.trait_arg_types.clone(),
            }],
        )]);
        let result = super::object_schema::erased_bound_schemas(
            &parameters,
            &result_bounds,
            &HashMap::new(),
            &self.object_traits,
            &self.erased_member_orders(),
            &self.normalization_env,
            &mut self.type_context,
            false,
        )?
        .into_iter()
        .find(|d| d.trait_id == trait_id)
        .ok_or("selected dictionary result schema missing")?;
        let mut template = MirDictionaryFactoryTemplate {
            implementation: implementation.id,
            parameters,
            result,
            bounds,
            members: vec![],
        };
        for member in &template.result.members {
            let method_id = *self
                .effective_trait_methods
                .get(&(implementation.id, member.member_id))
                .ok_or("selected impl member authority missing")?;
            if !implementation.methods.values().any(|m| m.id == method_id) {
                return Err("selected impl member body unavailable".into());
            }
            template.members.push(MirDictionaryFactoryMember {
                member_id: member.member_id,
                method_id,
                entry: MirErasedKey {
                    definition: method_id,
                    static_args: vec![],
                },
            });
        }
        // Publish the selected declaration before compiling its method bodies;
        // recursive *calls* reuse entries, whereas cyclic evidence still rejects.
        self.erased
            .dictionary_templates
            .insert(implementation.id, template.clone());
        for (binding, member) in template.members.iter().zip(&template.result.members) {
            if !member.binder.parameters.is_empty() {
                let substitution = implementation
                    .type_generics
                    .iter()
                    .map(|p| (p.id, Type::Generic(p.id)))
                    .collect();
                let entry = self.materialize_polymorphic_member(
                    implementation,
                    member,
                    &template.parameters,
                    &template.bounds,
                    &substitution,
                )?;
                if entry != binding.entry {
                    return Err("selected polymorphic factory entry key disagrees".into());
                }
                continue;
            }
            if self.erased.functions.contains_key(&binding.entry) {
                continue;
            }
            let method = implementation
                .methods
                .values()
                .find(|m| m.id == binding.method_id)
                .ok_or("selected method missing")?
                .clone();
            let mut variables = template
                .parameters
                .iter()
                .enumerate()
                .map(|(i, p)| (p.source, i as u32))
                .collect::<BTreeMap<_, _>>();
            let mut params = member.params.clone();
            if let Some(mode) = member.receiver.filter(|mode| *mode != ReceiverMode::Move) {
                params[0] = MirErasedType::Reference {
                    inner: Box::new(params[0].clone()),
                    mutable: mode == ReceiverMode::Mut,
                };
            }
            if params.len() != method.params.len() || method.self_receiver != member.receiver {
                return Err("selected member parameter arity/receiver differs".into());
            }
            for (expected, actual) in params.iter().zip(&method.params) {
                bind_erased_parameters(expected, &actual.ty, &mut variables, &self.type_context)?;
            }
            bind_erased_parameters(
                &member.ret,
                &method.ret_type,
                &mut variables,
                &self.type_context,
            )?;
            if method
                .generic_params
                .iter()
                .any(|p| !variables.contains_key(&p.id))
            {
                return Err("member-generic dictionary entries require nested evidence".into());
            }
            let mut layouts = (0..template.parameters.len())
                .map(|i| MirErasedType::Parameter(i as u32))
                .collect::<Vec<_>>();
            for ty in params.iter().chain(std::iter::once(&member.ret)) {
                ty.collect_layouts(&mut layouts, false);
            }
            let signature = MirErasedSignature {
                require_borrow_free: crate::mir::erased_callback_storage_requirements(
                    &params,
                    &template.bounds,
                ),
                parameters: template.parameters.clone(),
                params,
                ret: member.ret.clone(),
                dictionaries: template.bounds.clone(),
                layouts,
            };
            let symbol = format!(
                "rock.erased.selected.{}",
                self.backend_symbol_for_origin(
                    &super::InstanceOrigin::ImplMethod {
                        owner: self.impl_owner_identity(implementation),
                        method: method.id,
                    },
                    &[]
                )
            );
            self.erased.functions.insert(
                binding.entry.clone(),
                MirErasedFunction {
                    key: binding.entry.clone(),
                    symbol,
                    linkage: MirLinkage::Internal,
                    signature: signature.clone(),
                    body: None,
                },
            );
            let body = ErasedBodyBuilder::new(self, signature, variables, &method)?
                .for_entry(binding.entry.clone())
                .block(&method.body)?;
            self.erased
                .functions
                .get_mut(&binding.entry)
                .ok_or("selected entry disappeared")?
                .body = Some(body);
        }
        Ok(template)
    }

    fn materialize_erased_function(&mut self, id: DefId) -> Result<MirErasedKey, String> {
        let key = MirErasedKey {
            definition: id,
            static_args: vec![],
        };
        if self.erased.functions.contains_key(&key) {
            return Ok(key);
        }
        let (_, function) = self
            .generic_function_by_id(id)
            .ok_or("erased generic function body provider missing")?;
        let parameters = function
            .generic_params
            .iter()
            .map(|p| MirErasedParameter {
                source: p.id,
                kind: p.kind.clone(),
            })
            .collect::<Vec<_>>();
        let variables = parameters
            .iter()
            .enumerate()
            .map(|(i, p)| (p.source, i as u32))
            .collect();
        let params = function
            .params
            .iter()
            .map(|p| {
                MirErasedType::from_type(
                    &p.ty,
                    &variables,
                    None,
                    &mut self.type_context,
                    &self.normalization_env,
                )
                .map_err(|e| format!("{e:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let ret = MirErasedType::from_type(
            &function.ret_type,
            &variables,
            None,
            &mut self.type_context,
            &self.normalization_env,
        )
        .map_err(|e| format!("{e:?}"))?;
        let dictionaries = super::object_schema::erased_bound_schemas(
            &parameters,
            &function.generic_bounds,
            &HashMap::new(),
            &self.object_traits,
            &self.erased_member_orders(),
            &self.normalization_env,
            &mut self.type_context,
            false,
        )?;
        let mut layouts = parameters
            .iter()
            .enumerate()
            .filter(|(_, p)| p.kind == crate::type_services::kind::Kind::Type)
            .map(|(i, _)| MirErasedType::Parameter(i as u32))
            .collect::<Vec<_>>();
        for ty in params.iter().chain(std::iter::once(&ret)) {
            ty.collect_layouts(&mut layouts, false);
        }
        for dictionary in &dictionaries {
            for member in &dictionary.members {
                member.collect_parent_layouts(&mut layouts, false);
            }
        }
        let require_borrow_free =
            crate::mir::erased_callback_storage_requirements(&params, &dictionaries);
        let signature = MirErasedSignature {
            parameters,
            params,
            ret,
            dictionaries,
            layouts,
            require_borrow_free,
        };
        let symbol = format!(
            "rock.erased.{}",
            self.backend_symbol_for_origin(&super::InstanceOrigin::Function(id), &[])
        );
        self.erased.functions.insert(
            key.clone(),
            MirErasedFunction {
                key: key.clone(),
                symbol,
                linkage: MirLinkage::Internal,
                signature: signature.clone(),
                body: None,
            },
        );
        let body =
            ErasedBodyBuilder::new(self, signature, variables, &function)?.block(&function.body)?;
        self.erased
            .functions
            .get_mut(&key)
            .ok_or("erased declaration disappeared")?
            .body = Some(body);
        Ok(key)
    }
}

fn scoped_block_span(block: &HirBlock) -> Option<Span> {
    block.stmts.iter().find_map(|statement| match statement {
        HirStmt::Let { value, .. }
        | HirStmt::Expr(value)
        | HirStmt::Return(Some(value))
        | HirStmt::Break(Some(value)) => scoped_expr_span(value),
        _ => None,
    })
}

fn scoped_expr_span(expression: &HirExpr) -> Option<Span> {
    match &expression.kind {
        HirExprKind::Open { .. } | HirExprKind::PackageOpen { .. } => Some(expression.span.clone()),
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => scoped_expr_span(condition)
            .or_else(|| scoped_block_span(then_branch))
            .or_else(|| else_branch.as_ref().and_then(scoped_block_span)),
        HirExprKind::Block(body)
        | HirExprKind::UnsafeBlock(body)
        | HirExprKind::Loop(body)
        | HirExprKind::Lambda { body, .. } => scoped_block_span(body),
        HirExprKind::While { condition, body } => {
            scoped_expr_span(condition).or_else(|| scoped_block_span(body))
        }
        HirExprKind::For { iter, body, .. } => {
            scoped_expr_span(iter).or_else(|| scoped_block_span(body))
        }
        HirExprKind::Match { scrutinee, arms } => scoped_expr_span(scrutinee)
            .or_else(|| arms.iter().find_map(|arm| scoped_block_span(&arm.body))),
        HirExprKind::MethodCall(receiver, _, args, _, _) | HirExprKind::Call(receiver, args, _) => {
            scoped_expr_span(receiver).or_else(|| args.iter().find_map(scoped_expr_span))
        }
        HirExprKind::OwnedObjectCall { owner, args, .. } => {
            scoped_expr_span(owner).or_else(|| args.iter().find_map(scoped_expr_span))
        }
        HirExprKind::PackagePack { payload: inner, .. }
        | HirExprKind::ObjectCoercion(inner, _)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Deref(inner)
        | HirExprKind::TupleIndex(inner, _)
        | HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::UnaryOp(_, inner)
        | HirExprKind::Cast(inner, _)
        | HirExprKind::ArrayRepeat(inner, _)
        | HirExprKind::Try { expr: inner, .. } => scoped_expr_span(inner),
        HirExprKind::BinOp(_, left, right) | HirExprKind::Assign(left, right) => {
            scoped_expr_span(left).or_else(|| scoped_expr_span(right))
        }
        HirExprKind::TupleLiteral(values)
        | HirExprKind::ArrayLiteral(values)
        | HirExprKind::EnumVariant(_, _, values, _)
        | HirExprKind::Intrinsic { args: values, .. } => values.iter().find_map(scoped_expr_span),
        HirExprKind::StructLiteral(_, _, fields) => fields
            .iter()
            .find_map(|field| scoped_expr_span(&field.value)),
        _ => None,
    }
}

struct ErasedBodyBuilder<'a> {
    entry: MirErasedKey,
    dictionary_factories: Vec<MirDictionaryFactoryRequirement>,
    dictionary_active: Vec<MirErasedDictionarySchema>,
    mono: &'a mut Monomorphizer,
    signature: MirErasedSignature,
    variables: BTreeMap<GenericParamId, u32>,
    bindings: HashMap<HirLocalId, MirErasedLocal>,
    locals: Vec<MirErasedType>,
    cleanup: crate::mir::cleanup::CleanupScopes<MirErasedLocal>,
    mutable_locals: std::collections::BTreeSet<MirErasedLocal>,
    blocks: Vec<MirErasedBasicBlock>,
    current: MirErasedBlock,
    terminated: bool,
    scopes: Vec<MirErasedScope>,
    witnesses: BTreeMap<crate::types::WitnessId, u32>,
}

impl<'a> ErasedBodyBuilder<'a> {
    fn new(
        mono: &'a mut Monomorphizer,
        signature: MirErasedSignature,
        variables: BTreeMap<GenericParamId, u32>,
        function: &HirFunction,
    ) -> Result<Self, String> {
        if function.params.len() != signature.params.len() {
            return Err("erased body parameter count mismatch".into());
        }
        let bindings = function
            .params
            .iter()
            .enumerate()
            .map(|(index, param)| (param.local_id, MirErasedLocal(index as u32)))
            .collect();
        let locals = signature.params.clone();
        let mut cleanup = crate::mir::cleanup::CleanupScopes::default();
        cleanup.enter(
            (0..locals.len())
                .map(|index| MirErasedLocal(index as u32))
                .collect(),
        );
        let mutable_locals = function
            .params
            .iter()
            .enumerate()
            .filter(|(_, param)| param.mutable)
            .map(|(index, _)| MirErasedLocal(index as u32))
            .collect();
        Ok(Self {
            entry: MirErasedKey {
                definition: function.id,
                static_args: vec![],
            },
            dictionary_factories: vec![],
            dictionary_active: vec![],
            mono,
            signature,
            variables,
            bindings,
            locals,
            cleanup,
            mutable_locals,
            blocks: vec![MirErasedBasicBlock {
                statements: Vec::new(),
                terminator: MirErasedTerminator::Unreachable,
            }],
            current: MirErasedBlock(0),
            terminated: false,
            scopes: Vec::new(),
            witnesses: BTreeMap::new(),
        })
    }
    fn ty(&mut self, ty: &Type) -> Result<MirErasedType, String> {
        MirErasedType::from_type_scoped(
            ty,
            &self.variables,
            None,
            &self.witnesses,
            &mut self.mono.type_context,
            &self.mono.normalization_env,
        )
        .map_err(|e| format!("erased type: {e:?}; actual typed input: {ty:?}"))
    }
    fn for_entry(mut self, entry: MirErasedKey) -> Self {
        self.entry = entry;
        self
    }

    fn dictionary(
        &mut self,
        required: &MirErasedDictionarySchema,
        span: &Span,
    ) -> Result<u32, String> {
        if let Some(index) = self
            .signature
            .dictionaries
            .iter()
            .position(|d| d == required)
        {
            return Ok(index as u32);
        }
        if let Some(index) = self
            .dictionary_factories
            .iter()
            .position(|r| &r.result == required)
        {
            return Ok((self.signature.dictionaries.len() + index) as u32);
        }
        if self.dictionary_active.len() >= 128 || self.dictionary_active.contains(required) {
            return Err("recursive selected dictionary construction".into());
        }
        self.dictionary_active.push(required.clone());
        let bindings = self
            .signature
            .parameters
            .iter()
            .map(|p| Some(Type::Generic(p.source)))
            .collect::<Vec<_>>();
        let source = required
            .subject
            .instantiate_optional(&bindings, &self.mono.type_context)
            .map_err(|e| format!("{e:?}"))?;
        let arguments = required
            .trait_args
            .iter()
            .map(|ty| {
                ty.instantiate_optional(&bindings, &self.mono.type_context)
                    .map_err(|e| format!("{e:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut assumed = crate::hir::HirGenericBounds::new();
        for schema in &self.signature.dictionaries {
            let MirErasedType::Parameter(index) = schema.subject else {
                continue;
            };
            let parameter = self
                .signature
                .parameters
                .get(index as usize)
                .ok_or("dictionary bound ordinal missing")?;
            let type_args = schema
                .trait_args
                .iter()
                .map(|ty| {
                    ty.instantiate_optional(&bindings, &self.mono.type_context)
                        .map_err(|e| format!("{e:?}"))
                })
                .collect::<Result<_, _>>()?;
            assumed
                .entry(parameter.source)
                .or_default()
                .push(crate::types::TraitBound {
                    trait_id: schema.trait_id,
                    type_args,
                });
        }
        let impls = self
            .mono
            .trait_impls
            .values()
            .flatten()
            .map(|imp| (imp.id, imp.clone()))
            .collect::<HashMap<_, _>>();
        let (implementation, substitution) =
            crate::selection::SelectionService::select_declared_dictionary_impl(
                &self.mono.object_traits,
                &impls,
                &assumed,
                &source,
                required.trait_id,
                &arguments,
            )
            .map_err(|e| format!("selected dictionary proof: {e:?}"))?;
        let implementation = implementation.clone();
        let template = self
            .mono
            .materialize_dictionary_template(&implementation, span)?;
        let indices = self
            .signature
            .parameters
            .iter()
            .enumerate()
            .map(|(i, p)| (p.source, i as u32))
            .collect();
        let type_arguments = template
            .parameters
            .iter()
            .map(|p| {
                let ty = substitution
                    .get(&p.source)
                    .ok_or("selected dictionary capture missing")?;
                MirErasedType::from_type(
                    ty,
                    &indices,
                    None,
                    &mut self.mono.type_context,
                    &self.mono.normalization_env,
                )
                .map_err(|e| format!("{e:?}"))
            })
            .collect::<Result<Vec<_>, String>>()?;
        if template
            .result
            .substitute(&type_arguments, &self.mono.type_context)
            .map_err(|e| format!("{e:?}"))?
            != *required
        {
            return Err("selected dictionary schema disagrees with the callee declaration".into());
        }
        let mut children = Vec::new();
        for bound in &template.bounds {
            let bound = bound
                .substitute(&type_arguments, &self.mono.type_context)
                .map_err(|e| format!("{e:?}"))?;
            children.push(self.dictionary(&bound, span)?);
        }
        self.dictionary_active.pop();
        let index = (self.signature.dictionaries.len() + self.dictionary_factories.len()) as u32;
        self.dictionary_factories
            .push(MirDictionaryFactoryRequirement {
                implementation: implementation.id,
                arguments: type_arguments,
                children,
                result: required.clone(),
            });
        Ok(index)
    }

    fn allocate_local(&mut self, ty: MirErasedType) -> MirErasedLocal {
        let destination = MirErasedLocal(self.locals.len() as u32);
        self.locals.push(ty);
        self.mutable_locals.insert(destination);
        destination
    }
    fn local(&mut self, ty: MirErasedType) -> MirErasedLocal {
        let destination = self.allocate_local(ty);
        self.cleanup.register(destination);
        destination
    }
    fn assign(&mut self, destination: MirErasedLocal, value: MirErasedRvalue) {
        self.blocks[self.current.0 as usize]
            .statements
            .push(MirErasedAssignment { destination, value });
    }
    fn temp(&mut self, ty: MirErasedType, value: MirErasedRvalue) -> MirErasedPlace {
        let destination = self.local(ty);
        self.assign(destination, value);
        MirErasedPlace::local(destination)
    }
    fn finish_scope_locals(&mut self, locals: Vec<MirErasedLocal>) {
        if self.terminated || locals.is_empty() {
            return;
        }
        let target = self.new_block();
        self.terminate(MirErasedTerminator::Cleanup { locals, target });
        self.enter(target);
    }
    fn return_value(&mut self, value: MirErasedOperand) {
        // Transport the result before destroying any storage in its source scope.
        let ty = self.signature.ret.clone();
        let returned = self.allocate_local(ty.clone());
        self.assign(returned, MirErasedRvalue::Use(value));
        self.finish_scope_locals(self.cleanup.active_from(0));
        self.terminate(MirErasedTerminator::Return(
            self.operand(MirErasedPlace::local(returned), &ty),
        ));
    }
    fn new_block(&mut self) -> MirErasedBlock {
        let block = MirErasedBlock(self.blocks.len() as u32);
        self.blocks.push(MirErasedBasicBlock {
            statements: Vec::new(),
            terminator: MirErasedTerminator::Unreachable,
        });
        block
    }
    fn terminate(&mut self, terminator: MirErasedTerminator) {
        self.blocks[self.current.0 as usize].terminator = terminator;
        self.terminated = true;
    }
    fn enter(&mut self, block: MirErasedBlock) {
        self.current = block;
        self.terminated = false;
    }
    fn operand(&self, place: MirErasedPlace, ty: &MirErasedType) -> MirErasedOperand {
        let copy = crate::mir::copy_proven(ty, &self.mono.type_context);
        if copy {
            MirErasedOperand::Copy(place)
        } else {
            MirErasedOperand::Move(place)
        }
    }
    fn expression(&mut self, expr: &HirExpr) -> Result<Option<MirErasedPlace>, String> {
        let ty = self.ty(&expr.ty)?;
        Ok(Some(match &expr.kind {
            HirExprKind::PackageOpen {
                source,
                binding,
                body,
            } => {
                let Some(source_place) = self.expression(source)? else {
                    return Ok(None);
                };
                let Type::Exists(package_type) = &binding.source_ty else {
                    return Err("borrowed package opening is not supported".into());
                };
                self.mono
                    .ensure_payload_schemas(&binding.source_ty, &mut Default::default())?;
                let object =
                    crate::type_services::substitution::existential_object_schema(package_type)
                        .map_err(|error| error.to_string())?;
                let object = self
                    .mono
                    .ensure_object_schema(&Type::Object(Box::new(object)))?;
                let package = self.mono.intern_type(&binding.source_ty);
                let scope = self.scopes.len() as u32;
                self.scopes.push(MirErasedScope {
                    witness: binding.witness,
                    object,
                    source: MirErasedScopeSource::Package { package },
                });
                if self.witnesses.insert(binding.witness, scope).is_some() {
                    return Err("package witness binder was reused".into());
                }
                let payload_ty = self.ty(&binding.value.ty)?;
                let value = self.temp(
                    payload_ty,
                    MirErasedRvalue::PackageOpen {
                        scope,
                        source: MirErasedOperand::Move(source_place),
                    },
                );
                let previous = self.bindings.insert(binding.value.local_id, value.local);
                let result = self.lower_block(body)?;
                self.witnesses.remove(&binding.witness);
                if let Some(previous) = previous {
                    self.bindings.insert(binding.value.local_id, previous);
                } else {
                    self.bindings.remove(&binding.value.local_id);
                }
                let Some(result) = result else {
                    return Ok(None);
                };
                self.temp(ty, MirErasedRvalue::Use(result))
            }
            HirExprKind::PackagePack { payload, proof } => {
                self.mono
                    .ensure_payload_schemas(&expr.ty, &mut Default::default())?;
                let Type::Exists(package_type) = &expr.ty else {
                    return Err("package target is not existential".into());
                };
                let object =
                    crate::type_services::substitution::existential_object_schema(package_type)
                        .map_err(|error| error.to_string())?;
                let object = self
                    .mono
                    .ensure_object_schema(&Type::Object(Box::new(object)))?;
                let package = self.mono.intern_type(&expr.ty);
                let Some(place) = self.expression(payload)? else {
                    return Ok(None);
                };
                let payload_ty = self.ty(&payload.ty)?;
                let (witness, payload_descriptor) = if let Type::Witness(witness) = &proof.witness {
                    let scope = *self
                        .witnesses
                        .get(witness)
                        .ok_or("pack witness has no dominating opening")?;
                    // Only the exact whole package payload may forward its original
                    // owner descriptor; reconstructed reference tuples use a global
                    // structurally drop-free descriptor instead.
                    let from_package = if let MirErasedScopeSource::Package { package: source } =
                        self.scopes[scope as usize].source
                    {
                        crate::mir::package_payload_type(
                            source,
                            &proof.witness,
                            &self.mono.type_context,
                        )
                        .is_ok_and(|original| original == payload.ty)
                            && matches!(payload.kind, HirExprKind::ResolvedVar(_))
                    } else {
                        false
                    };
                    (
                        MirPackageWitnessEvidence::Scoped {
                            scope,
                            view: object,
                        },
                        if from_package {
                            MirPackagePayloadDescriptor::FromPackage(scope)
                        } else {
                            MirPackagePayloadDescriptor::UniformDropFree
                        },
                    )
                } else {
                    let table = self.mono.materialize_object_table(
                        object,
                        &proof.witness,
                        &proof.views,
                        &expr.span,
                    )?;
                    self.mono
                        .monomorphize_drop_for_type(&payload.ty, Some(payload.span.clone()));
                    (
                        MirPackageWitnessEvidence::Concrete {
                            witness: self.mono.intern_type(&proof.witness),
                            table,
                        },
                        MirPackagePayloadDescriptor::Concrete(self.mono.intern_type(&payload.ty)),
                    )
                };
                let operand = self.operand(place, &payload_ty);
                self.temp(
                    ty,
                    MirErasedRvalue::PackagePack {
                        payload: operand,
                        plan: MirPackagePackPlan {
                            package,
                            witness,
                            payload_descriptor,
                        },
                    },
                )
            }
            HirExprKind::Open {
                source,
                binding,
                body,
            } => {
                if binding.owner.is_some() {
                    return Err("owning Open requires canonical split/rebuild execution in the scoped environment".into());
                }
                let Some(source_place) = self.expression(source)? else {
                    return Ok(None);
                };
                let source_ty = self.ty(&source.ty)?;
                let source_operand = self.operand(source_place, &source_ty);
                let object = self
                    .mono
                    .ensure_object_schema(&Type::Object(Box::new(binding.object.clone())))?;
                let Type::Reference { mutable, .. } = &binding.source_ty else {
                    return Err("borrowed scoped Open requires an object reference".into());
                };
                let scope = self.scopes.len() as u32;
                self.scopes.push(MirErasedScope {
                    witness: binding.witness,
                    object,
                    source: MirErasedScopeSource::Object { mutable: *mutable },
                });
                if self.witnesses.insert(binding.witness, scope).is_some() {
                    return Err("scoped witness binder was reused".into());
                }
                let value_ty = self.ty(&binding.value.ty)?;
                let value = self.temp(
                    value_ty,
                    MirErasedRvalue::Open {
                        scope,
                        source: source_operand,
                    },
                );
                let previous = self.bindings.insert(binding.value.local_id, value.local);
                let result = self.lower_block(body)?;
                self.witnesses.remove(&binding.witness);
                if let Some(previous) = previous {
                    self.bindings.insert(binding.value.local_id, previous);
                } else {
                    self.bindings.remove(&binding.value.local_id);
                }
                let Some(result) = result else {
                    return Ok(None);
                };
                self.temp(ty, MirErasedRvalue::Use(result))
            }
            HirExprKind::ObjectCoercion(value, coercion) => {
                let Some(crate::hir::HirObjectEvidence::Concrete {
                    source: Type::Witness(witness),
                    ..
                }) = &coercion.evidence
                else {
                    return Err(
                        "scoped object construction must retain accepted witness evidence".into(),
                    );
                };
                let scope = *self
                    .witnesses
                    .get(witness)
                    .ok_or("reclose has no dominating Open")?;
                let Type::Reference { inner, .. } = &coercion.target else {
                    return Err("scoped reclose currently requires a borrowed value".into());
                };
                let object = self.mono.ensure_object_schema(inner)?;
                let Some(source) = self.expression(value)? else {
                    return Ok(None);
                };
                let source_ty = self.ty(&value.ty)?;
                self.temp(
                    ty,
                    MirErasedRvalue::Reclose {
                        scope,
                        value: self.operand(source, &source_ty),
                        object,
                    },
                )
            }
            HirExprKind::Block(block) | HirExprKind::UnsafeBlock(block) => {
                let Some(operand) = self.lower_block(block)? else {
                    return Ok(None);
                };
                self.temp(ty, MirErasedRvalue::Use(operand))
            }
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let Some(condition_place) = self.expression(condition)? else {
                    return Ok(None);
                };
                let condition_ty = self.ty(&condition.ty)?;
                let condition = self.operand(condition_place, &condition_ty);
                let then_block = self.new_block();
                let else_block = self.new_block();
                let join = self.new_block();
                let mut destination = None;
                self.terminate(MirErasedTerminator::Branch {
                    condition,
                    then_block,
                    else_block,
                });
                self.enter(then_block);
                let result = self.lower_block(then_branch)?;
                if let Some(result) = result {
                    let local = self.local(ty.clone());
                    destination = Some(local);
                    self.blocks[self.current.0 as usize]
                        .statements
                        .push(MirErasedAssignment {
                            destination: local,
                            value: MirErasedRvalue::Use(result),
                        });
                    self.terminate(MirErasedTerminator::Goto(join));
                }
                self.enter(else_block);
                let result = if let Some(block) = else_branch {
                    self.lower_block(block)?
                } else {
                    let unit = MirErasedType::Concrete(self.mono.intern_type(&Type::Unit));
                    let place = self.temp(
                        unit.clone(),
                        MirErasedRvalue::Literal(MirErasedLiteral::Unit),
                    );
                    Some(self.operand(place, &unit))
                };
                if let Some(result) = result {
                    let local = *destination.get_or_insert_with(|| self.local(ty.clone()));
                    self.blocks[self.current.0 as usize]
                        .statements
                        .push(MirErasedAssignment {
                            destination: local,
                            value: MirErasedRvalue::Use(result),
                        });
                    self.terminate(MirErasedTerminator::Goto(join));
                }
                self.enter(join);
                let Some(destination) = destination else {
                    self.terminated = true;
                    return Ok(None);
                };
                MirErasedPlace::local(destination)
            }
            HirExprKind::ResolvedVar(reference) => {
                let HirVarTarget::Local(id) = reference.target else {
                    return Err("erased first-class named callable materialization requires dictionary evidence".into());
                };
                let local = *self
                    .bindings
                    .get(&id)
                    .ok_or("erased local binding missing")?;
                let mut place = MirErasedPlace::local(local);
                if !erased_types_agree(&self.locals[local.0 as usize], &ty, &self.mono.type_context)
                {
                    match &self.locals[local.0 as usize] {
                        MirErasedType::Concrete(id)
                            if matches!(self.mono.type_for(*id), Type::Reference { inner, .. }
                                if ty.instantiate(&[], &self.mono.type_context).is_ok_and(|actual| *inner == actual)) =>
                        {
                            place.projection.push(MirErasedProjection::Deref)
                        }
                        MirErasedType::Reference { inner, .. }
                            if erased_types_agree(inner, &ty, &self.mono.type_context) => {
                            place.projection.push(MirErasedProjection::Deref)
                        }
                        _ => return Err(format!("erased local type authority mismatch at {:?}: local {id:?}, storage {:?}, accepted view {ty:?}", expr.span, self.locals[local.0 as usize])),
                    }
                }
                place
            }
            HirExprKind::IntLiteral(value) => {
                self.temp(ty, MirErasedRvalue::Literal(MirErasedLiteral::Int(*value)))
            }
            HirExprKind::BoolLiteral(value) => {
                self.temp(ty, MirErasedRvalue::Literal(MirErasedLiteral::Bool(*value)))
            }
            HirExprKind::Unit => self.temp(ty, MirErasedRvalue::Literal(MirErasedLiteral::Unit)),
            HirExprKind::TupleLiteral(values) => {
                let mut operands = Vec::new();
                for value in values {
                    let Some(place) = self.expression(value)? else {
                        return Ok(None);
                    };
                    let ty = self.ty(&value.ty)?;
                    operands.push(self.operand(place, &ty));
                }
                self.temp(ty, MirErasedRvalue::Aggregate(operands))
            }
            HirExprKind::StructLiteral(_, definition, fields) => {
                let owner = match &ty {
                    MirErasedType::Concrete(id) => match self.mono.type_for(*id) {
                        Type::Struct { id, .. } => id,
                        _ => return Err("erased native aggregate is not a struct".into()),
                    },
                    MirErasedType::Nominal {
                        id,
                        flavor: crate::types::NominalTypeKind::Struct,
                        ..
                    } => *id,
                    _ => {
                        return Err(
                            "struct reconstruction requires a selected nominal template".into()
                        )
                    }
                };
                if *definition != Some(owner) {
                    return Err("struct literal lacks exact definition authority".into());
                }
                let field_ids = self
                    .mono
                    .nominal_field_ids
                    .get(&owner)
                    .cloned()
                    .ok_or("erased struct field layout unavailable")?;
                let mut operands = vec![None; field_ids.len()];
                for field in fields {
                    let location = field
                        .field
                        .as_ref()
                        .ok_or("erased struct initializer has no field identity")?;
                    if location.owner != owner {
                        return Err("erased struct field owner mismatch".into());
                    }
                    let Some(place) = self.expression(&field.value)? else {
                        return Ok(None);
                    };
                    let field_ty = self.ty(&field.value.ty)?;
                    let operand = self.operand(place, &field_ty);
                    let index = field_ids
                        .iter()
                        .position(|id| *id == location.field_id)
                        .ok_or("erased field identity absent from owner")?;
                    let entry = operands
                        .get_mut(index)
                        .ok_or("erased field index out of bounds")?;
                    if entry.replace(operand).is_some() {
                        return Err("duplicate erased struct field".into());
                    }
                }
                let fields = operands
                    .into_iter()
                    .map(|field| field.ok_or("missing erased struct field".to_string()))
                    .collect::<Result<Vec<_>, _>>()?;
                let value = if let MirErasedType::Concrete(id) = &ty {
                    self.mono
                        .monomorphize_drop_for_type(&expr.ty, Some(expr.span.clone()));
                    MirErasedRvalue::NativeAggregate { ty: *id, fields }
                } else {
                    MirErasedRvalue::NominalAggregate {
                        ty: ty.clone(),
                        fields: field_ids.into_iter().zip(fields).collect(),
                    }
                };
                self.temp(ty, value)
            }
            HirExprKind::FieldAccess(base, _, Some(location)) => {
                let Some(mut place) = self.expression(base)? else {
                    return Ok(None);
                };
                let mut base_ty = self.ty(&base.ty)?;
                loop {
                    let inner = match &base_ty {
                        MirErasedType::Reference { inner, .. } => Some(*inner.clone()),
                        MirErasedType::Concrete(id) => match self.mono.type_for(*id) {
                            Type::Reference { inner, .. } => {
                                Some(MirErasedType::Concrete(self.mono.intern_type(&inner)))
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    let Some(inner) = inner else {
                        break;
                    };
                    place.projection.push(MirErasedProjection::Deref);
                    base_ty = inner;
                }
                let owner = match base_ty {
                    MirErasedType::Nominal {
                        id,
                        flavor: crate::types::NominalTypeKind::Struct,
                        ..
                    } => id,
                    MirErasedType::Concrete(id) => match self.mono.type_for(id) {
                        Type::Struct { id, .. } => id,
                        _ => return Err("field base is not a struct".into()),
                    },
                    _ => return Err("field access lacks a selected nominal owner".into()),
                };
                if owner != location.owner
                    || !self
                        .mono
                        .nominal_field_ids
                        .get(&owner)
                        .is_some_and(|ids| ids.contains(&location.field_id))
                {
                    return Err(
                        "erased field access disagrees with canonical owner/field authority".into(),
                    );
                }
                place.projection.push(MirErasedProjection::NominalField {
                    owner,
                    field: location.field_id,
                });
                place
            }
            HirExprKind::TupleIndex(base, index) => {
                let Some(mut place) = self.expression(base)? else {
                    return Ok(None);
                };
                place.projection.push(MirErasedProjection::Field(*index));
                place
            }
            HirExprKind::Deref(base) => {
                let Some(mut place) = self.expression(base)? else {
                    return Ok(None);
                };
                place.projection.push(MirErasedProjection::Deref);
                place
            }
            HirExprKind::Ref(mutable, inner) => {
                let Some(place) = self.expression(inner)? else {
                    return Ok(None);
                };
                self.temp(
                    ty,
                    MirErasedRvalue::Borrow {
                        place,
                        mutable: *mutable,
                    },
                )
            }
            HirExprKind::MethodCall(receiver, _, args, _, target) => {
                let (trait_id, member_id) = match &target.target {
                    HirSelectedMethodTarget::TraitMethod {
                        trait_id,
                        member_id,
                        ..
                    } => (*trait_id, *member_id),
                    HirSelectedMethodTarget::ImplMethod {
                        impl_id,
                        method_id,
                        selected_trait: Some(selected),
                    } => {
                        let implementation = self.mono.trait_impls.get(&selected.trait_id)
                            .and_then(|implementations| implementations.iter().find(|implementation| implementation.id == *impl_id))
                            .ok_or_else(|| format!("erased selected impl {impl_id:?} is absent for trait {:?} at {:?}", selected.trait_id, expr.span))?;
                        if implementation.trait_id != Some(selected.trait_id)
                            || self
                                .mono
                                .effective_trait_methods
                                .get(&(*impl_id, selected.member_id))
                                != Some(method_id)
                        {
                            return Err(format!("erased selected impl/member authority disagrees: {impl_id:?}/{method_id:?}/{:?} at {:?}", selected.member_id, expr.span));
                        }
                        // Method receivers can already carry the selected autoref.
                        // Bind the impl subject, not that call-only reference, just
                        // as the dictionary consumer below addresses the payload.
                        let mut subject = &receiver.ty;
                        if !matches!(
                            &implementation.receiver_pattern,
                            crate::hir::HirImplReceiverPattern::Exact(Type::Reference { .. })
                        ) {
                            while let Type::Reference { inner, .. } = subject {
                                subject = inner;
                            }
                        }
                        let mut substitution = crate::selection::impl_receiver_pattern_substitution(
                            implementation, subject, self.mono.trait_impls.values().flatten())
                            .ok_or_else(|| format!("erased selected impl {impl_id:?} receiver substitution is invalid at {:?}", expr.span))?;
                        let mut bound_parameters = std::collections::BTreeSet::new();
                        for binding in &target.owner_substitution {
                            if !implementation
                                .type_generics
                                .iter()
                                .any(|parameter| parameter.id == binding.param)
                                || !bound_parameters.insert(binding.param)
                                || !crate::selection::type_pattern_matches(
                                    &Type::Generic(binding.param),
                                    &binding.ty,
                                    &mut substitution,
                                )
                            {
                                return Err(format!("erased selected impl {impl_id:?} owner substitution disagrees at {:?}", expr.span));
                            }
                        }
                        let trait_args = implementation
                            .trait_arg_types
                            .iter()
                            .map(|ty| {
                                crate::type_services::substitution::instantiate(ty, &substitution)
                                    .map_err(|e| e.to_string())
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        if trait_args != selected.trait_args
                            || implementation
                                .type_generics
                                .iter()
                                .any(|parameter| !substitution.contains_key(&parameter.id))
                        {
                            return Err(format!("erased selected impl {impl_id:?} trait instantiation is incomplete/stale at {:?}", expr.span));
                        }
                        (selected.trait_id, selected.member_id)
                    }
                    _ => {
                        return Err(format!(
                            "erased method lacks selected trait/member authority at {:?}",
                            expr.span
                        ))
                    }
                };
                if let HirSelectedMethodTarget::TraitMethod {
                    dispatch: HirTraitDispatchKind::Opened(witness),
                    ..
                } = &target.target
                {
                    let scope = *self
                        .witnesses
                        .get(witness)
                        .ok_or("opened call has no dominating witness")?;
                    let object = self.scopes[scope as usize].object;
                    let schema = self.mono.object_schemas[&object].clone();
                    let matches = |candidate_trait, candidate_member, candidate_args: &[TypeId]| {
                        candidate_trait == trait_id
                            && candidate_member == member_id
                            && candidate_args.len() == target.trait_args().len()
                            && candidate_args.iter().zip(target.trait_args()).all(
                                |(candidate, actual)| {
                                    self.mono
                                        .open_object_component(
                                            &self.mono.type_for(*candidate),
                                            &Type::Witness(*witness),
                                        )
                                        .is_ok_and(|expected| expected == *actual)
                                },
                            )
                    };
                    let slot = schema
                        .slots
                        .iter()
                        .position(|slot| matches(slot.trait_id, slot.member_id, &slot.trait_args));
                    let erased_slot = schema
                        .erased_slots
                        .iter()
                        .position(|slot| matches(slot.trait_id, slot.member_id, &slot.trait_args));
                    let receiver_mode = slot
                        .map(|slot| schema.slots[slot].receiver)
                        .or_else(|| erased_slot.map(|slot| schema.erased_slots[slot].receiver))
                        .ok_or("opened member is absent from its canonical producer slots")?;
                    let receiver_place = if let HirExprKind::Ref(mutable, place) = &receiver.kind {
                        if receiver_mode == ReceiverMode::Move
                            || *mutable != (receiver_mode == ReceiverMode::Mut)
                            || receiver.ty
                                != (Type::Reference {
                                    inner: Box::new(place.ty.clone()),
                                    mutable: *mutable,
                                })
                        {
                            return Err(
                                "opened receiver reborrow lacks exact selected access/type".into(),
                            );
                        }
                        // OpenedCall borrows this place for the call and keeps a
                        // loan only if the result carries one; do not materialize
                        // a call-only reference local with an enclosing lifetime.
                        let Some(place) = self.expression(place)? else {
                            return Ok(None);
                        };
                        place
                    } else {
                        let Some(mut place) = self.expression(receiver)? else {
                            return Ok(None);
                        };
                        if matches!(self.ty(&receiver.ty)?, MirErasedType::Reference { .. }) {
                            place.projection.push(MirErasedProjection::Deref);
                        }
                        place
                    };
                    let mut operands = Vec::new();
                    for argument in args {
                        let Some(place) = self.expression(argument)? else {
                            return Ok(None);
                        };
                        let ty = self.ty(&argument.ty)?;
                        operands.push(self.operand(place, &ty));
                    }
                    if let Some(slot) = slot {
                        return Ok(Some(self.temp(
                            ty,
                            MirErasedRvalue::OpenedCall {
                                scope,
                                slot: slot as u32,
                                receiver: receiver_place,
                                args: operands,
                            },
                        )));
                    }
                    let slot = erased_slot
                        .ok_or("opened member is absent from its canonical producer slots")?;
                    let signature = &schema.erased_slots[slot].signature;
                    let mut type_args = vec![MirErasedType::Scoped(scope)];
                    for parameter in signature.parameters.iter().skip(1) {
                        let binding = target
                            .method_substitution
                            .iter()
                            .chain(&target.owner_substitution)
                            .find(|binding| binding.param == parameter.source)
                            .ok_or("opened erased method substitution missing")?;
                        type_args.push(self.ty(&binding.ty)?);
                    }
                    let dictionaries = signature
                        .dictionaries
                        .iter()
                        .map(|required| {
                            let required = required
                                .substitute(&type_args, &self.mono.type_context)
                                .map_err(|error| format!("{error:?}"))?;
                            self.signature
                                .dictionaries
                                .iter()
                                .position(|available| *available == required)
                                .map(|index| index as u32)
                                .ok_or(
                                    "opened erased call lacks its exact dictionary environment"
                                        .to_string(),
                                )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(Some(self.temp(
                        ty,
                        MirErasedRvalue::OpenedErasedCall {
                            scope,
                            slot: slot as u32,
                            type_args,
                            dictionaries,
                            receiver: receiver_place,
                            args: operands,
                        },
                    )));
                }
                let mut receiver_ty = self.ty(&receiver.ty)?;
                let mut dereferences = 0;
                while let MirErasedType::Reference { inner, .. } = receiver_ty {
                    receiver_ty = *inner;
                    dereferences += 1;
                }
                let trait_args = target
                    .trait_args()
                    .iter()
                    .map(|ty| self.ty(ty))
                    .collect::<Result<Vec<_>, _>>()?;
                let schema = self
                    .signature
                    .dictionaries
                    .iter()
                    .chain(self.dictionary_factories.iter().map(|r| &r.result))
                    .position(|d| {
                        d.trait_id == trait_id
                            && d.subject == receiver_ty
                            && d.trait_args == trait_args
                    })
                    .map(|index| {
                        if index < self.signature.dictionaries.len() {
                            self.signature.dictionaries[index].clone()
                        } else {
                            self.dictionary_factories[index - self.signature.dictionaries.len()]
                                .result
                                .clone()
                        }
                    });
                let required = if let Some(schema) = schema {
                    schema
                } else {
                    let MirErasedType::Parameter(index) = receiver_ty else {
                        return Err(
                            "selected dictionary method requires a simple Type-kind subject".into(),
                        );
                    };
                    let parameter = self
                        .signature
                        .parameters
                        .get(index as usize)
                        .ok_or("dictionary receiver ordinal missing")?;
                    let bindings = self
                        .signature
                        .parameters
                        .iter()
                        .map(|p| Some(Type::Generic(p.source)))
                        .collect::<Vec<_>>();
                    let arguments = trait_args
                        .iter()
                        .map(|ty| {
                            ty.instantiate_optional(&bindings, &self.mono.type_context)
                                .map_err(|e| format!("{e:?}"))
                        })
                        .collect::<Result<_, _>>()?;
                    let bounds = crate::hir::HirGenericBounds::from_iter([(
                        parameter.source,
                        vec![crate::types::TraitBound {
                            trait_id,
                            type_args: arguments,
                        }],
                    )]);
                    super::object_schema::erased_bound_schemas(
                        &self.signature.parameters,
                        &bounds,
                        &HashMap::new(),
                        &self.mono.object_traits,
                        &self.mono.erased_member_orders(),
                        &self.mono.normalization_env,
                        &mut self.mono.type_context,
                        false,
                    )?
                    .into_iter()
                    .find(|d| {
                        d.trait_id == trait_id
                            && d.subject == receiver_ty
                            && d.trait_args == trait_args
                    })
                    .ok_or("canonical dictionary schema missing")?
                };
                let dictionary = self.dictionary(&required, &expr.span)?;
                let member = required
                    .members
                    .iter()
                    .position(|m| m.member_id == member_id)
                    .ok_or("erased dictionary member missing")?;
                let member_schema = &required.members[member];
                let authority = match &target.target {
                    HirSelectedMethodTarget::ImplMethod {
                        impl_id, method_id, ..
                    } => {
                        let implementation = self
                            .mono
                            .trait_impls
                            .get(&trait_id)
                            .and_then(|impls| impls.iter().find(|imp| imp.id == *impl_id))
                            .ok_or("member-generic selected impl authority missing")?;
                        let method = implementation
                            .methods
                            .values()
                            .find(|method| method.id == *method_id)
                            .ok_or("member-generic selected method authority missing")?;
                        method
                            .generic_params
                            .iter()
                            .filter(|p| {
                                !implementation
                                    .type_generics
                                    .iter()
                                    .any(|parent| parent.id == p.id)
                            })
                            .map(|p| (p.id, p.kind.clone()))
                            .collect::<Vec<_>>()
                    }
                    HirSelectedMethodTarget::TraitMethod {
                        trait_id,
                        member_id,
                        ..
                    } => {
                        let declaration = self
                            .mono
                            .object_traits
                            .get(trait_id)
                            .ok_or("member-generic declared trait authority missing")?;
                        let callee = declaration
                            .signatures
                            .values()
                            .find(|signature| signature.id == *member_id)
                            .ok_or("member-generic declared callee authority missing")?;
                        let locals = callee
                            .generic_params
                            .iter()
                            .filter(|p| {
                                !declaration
                                    .generic_params
                                    .iter()
                                    .any(|parent| parent.id == p.id)
                                    && declaration
                                        .target
                                        .as_ref()
                                        .is_none_or(|parent| parent.id != p.id)
                            })
                            .map(|p| (p.id, p.kind.clone()))
                            .collect::<Vec<_>>();
                        if locals
                            .iter()
                            .map(|(source, kind)| MirErasedParameter {
                                source: *source,
                                kind: kind.clone(),
                            })
                            .ne(member_schema.binder.parameters.iter().cloned())
                        {
                            return Err("member-generic dictionary binder disagrees with its exact declared callee".into());
                        }
                        locals
                    }
                };
                if authority.len() != member_schema.binder.parameters.len()
                    || target.method_substitution.len() != authority.len()
                    || authority
                        .iter()
                        .zip(&member_schema.binder.parameters)
                        .any(|((_, kind), p)| kind != &p.kind)
                {
                    return Err(
                        "member-generic canonical method substitution arity/kind disagrees".into(),
                    );
                }
                let mut member_arguments = Vec::new();
                for (id, _) in authority {
                    let bindings = target
                        .method_substitution
                        .iter()
                        .filter(|binding| binding.param == id)
                        .collect::<Vec<_>>();
                    let [binding] = bindings.as_slice() else {
                        return Err("member-generic canonical binding absent/duplicated".into());
                    };
                    member_arguments.push(self.ty(&binding.ty)?);
                }
                let Some(mut receiver) = self.expression(receiver)? else {
                    return Ok(None);
                };
                receiver
                    .projection
                    .extend(std::iter::repeat(MirErasedProjection::Deref).take(dereferences));
                let mut operands = Vec::new();
                for arg in args {
                    let Some(place) = self.expression(arg)? else {
                        return Ok(None);
                    };
                    let ty = self.ty(&arg.ty)?;
                    operands.push(self.operand(place, &ty));
                }
                self.temp(
                    ty,
                    MirErasedRvalue::DictionaryCall {
                        dictionary: dictionary as u32,
                        member: member as u32,
                        type_args: member_arguments,
                        dictionaries: vec![],
                        receiver: Some(receiver),
                        args: operands,
                    },
                )
            }
            HirExprKind::Call(callee, args, target) => {
                if target.is_none() {
                    if let Some(inner) =
                        super::object_schema::applied_static_forwarder(callee, args, &expr.ty)?
                    {
                        let HirExprKind::Call(source, _, authority) = &inner.kind else {
                            return Err("checked static forwarder lost its call authority".into());
                        };
                        let applied = HirExpr {
                            ty: expr.ty.clone(),
                            span: expr.span.clone(),
                            kind: HirExprKind::Call(
                                source.clone(),
                                args.clone(),
                                authority.clone(),
                            ),
                        };
                        return self.expression(&applied);
                    }
                }
                if let Some(crate::hir::HirCallTarget::StaticMethod(selected)) = target {
                    let trait_id = selected
                        .method
                        .trait_id()
                        .ok_or("static dictionary call lacks trait authority")?;
                    let member_id = match &selected.method.target {
                        HirSelectedMethodTarget::TraitMethod { member_id, .. } => *member_id,
                        HirSelectedMethodTarget::ImplMethod {
                            impl_id,
                            method_id,
                            selected_trait: Some(member),
                        } => {
                            if self
                                .mono
                                .effective_trait_methods
                                .get(&(*impl_id, member.member_id))
                                != Some(method_id)
                            {
                                return Err(
                                    "static dictionary selected impl/member disagrees".into()
                                );
                            }
                            member.member_id
                        }
                        _ => {
                            return Err(
                                "static erased call requires declared trait member authority"
                                    .into(),
                            )
                        }
                    };
                    let declaration = self
                        .mono
                        .object_traits
                        .get(&trait_id)
                        .ok_or("static trait declaration missing")?;
                    let callee = declaration
                        .signatures
                        .values()
                        .find(|signature| signature.id == member_id)
                        .ok_or("static member declaration missing")?;
                    let declared_self =
                        super::object_schema::static_member_self(declaration, callee)?;
                    let declared_arguments = selected.method.trait_args();
                    if callee.self_receiver.is_some()
                        || callee.is_unsafe
                        || !selected.method.method_substitution.is_empty()
                        || !callee.generic_bounds.is_empty()
                        || !callee.generic_bounds.predicates.is_empty()
                        || !declaration.associated_types.is_empty()
                        || !declaration.predicates.is_empty()
                        || declaration.generic_params.len() != declared_arguments.len()
                        || callee.generic_params.iter().any(|parameter| {
                            parameter.id != declared_self
                                && !declaration
                                    .generic_params
                                    .iter()
                                    .any(|parent| parent.id == parameter.id)
                        })
                    {
                        return Err("static dictionary call has receiver/local/effect evidence outside this scope".into());
                    }
                    let mut declared_bindings = declaration
                        .generic_params
                        .iter()
                        .zip(declared_arguments)
                        .map(|(parameter, argument)| (parameter.id, argument.clone()))
                        .collect::<HashMap<_, _>>();
                    declared_bindings.insert(declared_self, selected.owner_ty.clone());
                    let ret = crate::type_services::substitution::instantiate(
                        &callee.ret,
                        &declared_bindings,
                    )
                    .map_err(|e| e.to_string())?;
                    let params = callee
                        .params
                        .iter()
                        .map(|ty| {
                            crate::type_services::substitution::instantiate(ty, &declared_bindings)
                        })
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| e.to_string())?;
                    if ret != selected.owner_ty
                        || ret != expr.ty
                        || params.len() != args.len()
                        || !params.iter().zip(args).all(|(declared, argument)| {
                            *declared == argument.ty
                                && crate::mir::static_dictionary_scalar_type(declared)
                        })
                    {
                        return Err("static call parameters/result disagree with the exact declared Self instantiation".into());
                    }
                    if let HirSelectedMethodTarget::TraitMethod { .. } = &selected.method.target {
                        let mut seen = std::collections::HashSet::new();
                        if selected.method.owner_substitution.iter().any(|binding| {
                            !seen.insert(binding.param)
                                || declared_bindings.get(&binding.param) != Some(&binding.ty)
                        }) {
                            return Err("static call owner substitution disagrees with canonical declared binders".into());
                        }
                    }
                    let subject = self.ty(&selected.owner_ty)?;
                    let trait_args = selected
                        .method
                        .trait_args()
                        .iter()
                        .map(|ty| self.ty(ty))
                        .collect::<Result<Vec<_>, _>>()?;
                    let available = self
                        .signature
                        .dictionaries
                        .iter()
                        .chain(self.dictionary_factories.iter().map(|r| &r.result))
                        .find(|schema| {
                            schema.subject == subject
                                && schema.trait_id == trait_id
                                && schema.trait_args == trait_args
                        })
                        .cloned();
                    let required = if let Some(schema) = available {
                        schema
                    } else {
                        let MirErasedType::Parameter(index) = subject else {
                            return Err(
                                "static dictionary subject requires exact Type parameter evidence"
                                    .into(),
                            );
                        };
                        let parameter = self
                            .signature
                            .parameters
                            .get(index as usize)
                            .ok_or("static subject ordinal missing")?;
                        let bindings = self
                            .signature
                            .parameters
                            .iter()
                            .map(|p| Some(Type::Generic(p.source)))
                            .collect::<Vec<_>>();
                        let arguments = trait_args
                            .iter()
                            .map(|ty| {
                                ty.instantiate_optional(&bindings, &self.mono.type_context)
                                    .map_err(|e| format!("{e:?}"))
                            })
                            .collect::<Result<_, _>>()?;
                        let bounds = crate::hir::HirGenericBounds::from_iter([(
                            parameter.source,
                            vec![crate::types::TraitBound {
                                trait_id,
                                type_args: arguments,
                            }],
                        )]);
                        super::object_schema::erased_bound_schemas(
                            &self.signature.parameters,
                            &bounds,
                            &HashMap::new(),
                            &self.mono.object_traits,
                            &self.mono.erased_member_orders(),
                            &self.mono.normalization_env,
                            &mut self.mono.type_context,
                            false,
                        )?
                        .into_iter()
                        .find(|schema| {
                            schema.subject == subject
                                && schema.trait_id == trait_id
                                && schema.trait_args == trait_args
                        })
                        .ok_or("static dictionary canonical schema missing")?
                    };
                    let dictionary = self.dictionary(&required, &expr.span)?;
                    let member = required
                        .members
                        .iter()
                        .position(|member| member.member_id == member_id)
                        .ok_or("static dictionary member absent")?;
                    if required.members[member].receiver.is_some() {
                        return Err("static dictionary receiver forged".into());
                    }
                    let mut operands = Vec::new();
                    for argument in args {
                        let Some(place) = self.expression(argument)? else {
                            return Ok(None);
                        };
                        let argument_type = self.ty(&argument.ty)?;
                        operands.push(self.operand(place, &argument_type));
                    }
                    return Ok(Some(self.temp(
                        ty,
                        MirErasedRvalue::DictionaryCall {
                            dictionary,
                            member: member as u32,
                            type_args: vec![],
                            dictionaries: vec![],
                            receiver: None,
                            args: operands,
                        },
                    )));
                }
                if let Some(crate::hir::HirCallTarget::Function(id)) = target {
                    let native = self
                        .mono
                        .zero_substitution_callable_instance(*id)
                        .or_else(|| {
                            let function = self.mono.concrete_function_by_id(*id)?;
                            if !function.generic_params.is_empty() {
                                return None;
                            }
                            let name = self.mono.function_names_by_id.get(id)?.clone();
                            // Reserve the exact source instance; the normal production
                            // traversal replaces its body with processed HIR before MIR.
                            Some(self.mono.register_function_instance(&name, &function))
                        });
                    if let Some(instance) = native {
                        let mut operands = Vec::new();
                        for argument in args {
                            let Some(place) = self.expression(argument)? else {
                                return Ok(None);
                            };
                            let argument_ty = self.ty(&argument.ty)?;
                            if !matches!(argument_ty, MirErasedType::Concrete(_)) {
                                return Err("native scoped call requires concrete arguments".into());
                            }
                            operands.push(self.operand(place, &argument_ty));
                        }
                        return Ok(Some(self.temp(
                            ty,
                            MirErasedRvalue::NativeApply {
                                target: MirCallableKey::Instance(instance),
                                args: operands,
                            },
                        )));
                    }
                    let key = self.mono.materialize_erased_function(*id)?;
                    let signature = self.mono.erased.functions[&key].signature.clone();
                    let mut actual = Vec::new();
                    let mut operands = Vec::new();
                    for arg in args {
                        let Some(place) = self.expression(arg)? else {
                            return Ok(None);
                        };
                        let ty = self.ty(&arg.ty)?;
                        operands.push(self.operand(place, &ty));
                        actual.push(ty);
                    }
                    let mut inferred = BTreeMap::new();
                    for (pattern, actual) in signature.params.iter().zip(&actual) {
                        infer_erased_arguments(pattern, actual, &mut inferred)?;
                    }
                    infer_erased_arguments(&signature.ret, &ty, &mut inferred)?;
                    let type_args = (0..signature.parameters.len())
                        .map(|index| {
                            inferred.get(&(index as u32)).cloned().ok_or_else(|| {
                                "erased generic call argument inference incomplete".to_string()
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let mut dictionaries = Vec::new();
                    for required in &signature.dictionaries {
                        let required = required
                            .substitute(&type_args, &self.mono.type_context)
                            .map_err(|e| format!("{e:?}"))?;
                        dictionaries.push(self.dictionary(&required, &expr.span)?);
                    }
                    self.temp(
                        ty,
                        MirErasedRvalue::Call {
                            target: key,
                            type_args,
                            dictionaries,
                            args: operands,
                        },
                    )
                } else {
                    if !matches!(target, None | Some(crate::hir::HirCallTarget::Local(_))) {
                        return Err("erased call target is not a dictionary-backed callback".into());
                    }
                    let callee_ty = self.ty(&callee.ty)?;
                    let argument_types =
                        args.iter()
                            .map(|arg| self.ty(&arg.ty))
                            .collect::<Result<Vec<_>, _>>()?;
                    let packed_ty = match argument_types.as_slice() {
                        [] => MirErasedType::Concrete(self.mono.intern_type(&Type::Unit)),
                        [argument] => argument.clone(),
                        _ => self
                            .mono
                            .simplify_erased_type(&MirErasedType::Tuple(argument_types))?,
                    };
                    let protocol = self
                        .mono
                        .language_items
                        .callable_protocols()
                        .find_map(|(_, trait_id, member)| {
                            self.signature
                                .dictionaries
                                .iter()
                                .enumerate()
                                .find(|(_, dictionary)| {
                                    dictionary.subject == callee_ty
                                        && dictionary.trait_id == trait_id
                                        && dictionary.trait_args
                                            == vec![packed_ty.clone(), ty.clone()]
                                })
                                .map(|(dictionary, definition)| {
                                    (
                                        dictionary,
                                        definition
                                            .members
                                            .iter()
                                            .position(|m| m.member_id == member),
                                    )
                                })
                        })
                        .ok_or("erased callback has no canonical callable dictionary")?;
                    let member = protocol.1.ok_or("callable dictionary member missing")?;
                    let Some(receiver) = self.expression(callee)? else {
                        return Ok(None);
                    };
                    let packed = if args.len() == 1 {
                        let Some(place) = self.expression(&args[0])? else {
                            return Ok(None);
                        };
                        place
                    } else if args.is_empty() {
                        let unit = MirErasedType::Concrete(self.mono.intern_type(&Type::Unit));
                        self.temp(unit, MirErasedRvalue::Literal(MirErasedLiteral::Unit))
                    } else {
                        let mut values = Vec::new();
                        for arg in args {
                            let Some(place) = self.expression(arg)? else {
                                return Ok(None);
                            };
                            let ty = self.ty(&arg.ty)?;
                            values.push(self.operand(place, &ty));
                        }
                        self.temp(packed_ty.clone(), MirErasedRvalue::Aggregate(values))
                    };
                    let definition = &self.signature.dictionaries[protocol.0].members[member];
                    if definition.params != vec![callee_ty, packed_ty.clone()]
                        || definition.ret != ty
                    {
                        return Err("callback arguments/result disagree with the canonical dictionary member".into());
                    }
                    let operand = self.operand(packed, &packed_ty);
                    self.temp(
                        ty,
                        MirErasedRvalue::DictionaryCall {
                            dictionary: protocol.0 as u32,
                            member: member as u32,
                            type_args: vec![],
                            dictionaries: vec![],
                            receiver: Some(receiver),
                            args: vec![operand],
                        },
                    )
                }
            }
            _ => {
                return Err(
                    "operation requires an unsupported erased control-flow/storage/effect lowering"
                        .into(),
                )
            }
        }))
    }
    fn lower_block(&mut self, block: &HirBlock) -> Result<Option<MirErasedOperand>, String> {
        let outer_bindings = self.bindings.clone();
        self.cleanup.enter(Vec::new());
        let mut result = None;
        for (index, statement) in block.stmts.iter().enumerate() {
            if self.terminated {
                break;
            }
            match statement {
                HirStmt::Let {
                    local_id,
                    value,
                    mutable,
                    ..
                } => {
                    self.cleanup.enter(Vec::new());
                    let binding = if let Some(place) = self.expression(value)? {
                        let ty = self.ty(&value.ty)?;
                        let binding = self.allocate_local(ty.clone());
                        self.assign(binding, MirErasedRvalue::Use(self.operand(place, &ty)));
                        if !*mutable {
                            self.mutable_locals.remove(&binding);
                        }
                        Some(binding)
                    } else {
                        None
                    };
                    let temporaries = self.cleanup.leave();
                    self.finish_scope_locals(temporaries);
                    if let Some(binding) = binding {
                        self.cleanup.register(binding);
                        self.bindings.insert(*local_id, binding);
                    }
                }
                HirStmt::Expr(value) => {
                    self.cleanup.enter(Vec::new());
                    if let Some(place) = self.expression(value)? {
                        let ty = self.ty(&value.ty)?;
                        let operand = self.operand(place, &ty);
                        if index + 1 == block.stmts.len() {
                            // A caller-owned result outlives this scope; never
                            // allocate a destination for an expression that exits.
                            let destination = self.allocate_local(ty.clone());
                            self.assign(destination, MirErasedRvalue::Use(operand));
                            result = Some((destination, ty));
                        } else {
                            self.temp(ty, MirErasedRvalue::Use(operand));
                        }
                    }
                    let temporaries = self.cleanup.leave();
                    self.finish_scope_locals(temporaries);
                }
                HirStmt::Return(Some(value)) => {
                    self.cleanup.enter(Vec::new());
                    if let Some(place) = self.expression(value)? {
                        let ty = self.ty(&value.ty)?;
                        self.return_value(self.operand(place, &ty));
                    }
                    self.cleanup.leave();
                }
                HirStmt::Return(None) => {
                    let unit = MirErasedType::Concrete(self.mono.intern_type(&Type::Unit));
                    let place = self.temp(
                        unit.clone(),
                        MirErasedRvalue::Literal(MirErasedLiteral::Unit),
                    );
                    self.return_value(self.operand(place, &unit));
                }
                _ => {
                    return Err(
                        "erased body requires an explicit supported result/control-flow operation"
                            .into(),
                    )
                }
            }
        }
        self.bindings = outer_bindings;
        if !self.terminated && result.is_none() {
            let unit = MirErasedType::Concrete(self.mono.intern_type(&Type::Unit));
            let destination = self.allocate_local(unit.clone());
            self.assign(
                destination,
                MirErasedRvalue::Literal(MirErasedLiteral::Unit),
            );
            result = Some((destination, unit));
        }
        let locals = self.cleanup.leave();
        self.finish_scope_locals(locals);
        Ok(result.map(|(destination, ty)| {
            self.cleanup.register(destination);
            self.operand(MirErasedPlace::local(destination), &ty)
        }))
    }
    fn block(mut self, block: &HirBlock) -> Result<MirErasedBody, String> {
        let result = self.lower_block(block)?;
        if let Some(result) = result {
            self.return_value(result);
        }
        if !self.dictionary_factories.is_empty() {
            self.mono
                .erased
                .dictionary_factories
                .insert(self.entry, self.dictionary_factories);
        }
        Ok(MirErasedBody {
            locals: self.locals,
            mutable_locals: self.mutable_locals,
            scopes: self.scopes,
            blocks: self.blocks,
        })
    }
}

fn infer_erased_arguments(
    pattern: &MirErasedType,
    actual: &MirErasedType,
    result: &mut BTreeMap<u32, MirErasedType>,
) -> Result<(), String> {
    match (pattern, actual) {
        (MirErasedType::Parameter(index), actual) => {
            if result
                .insert(*index, actual.clone())
                .is_some_and(|old| old != *actual)
            {
                return Err("erased generic argument conflict".into());
            }
        }
        (
            MirErasedType::Reference {
                inner: expected,
                mutable: a,
            },
            MirErasedType::Reference {
                inner: actual,
                mutable: b,
            },
        ) if a == b => infer_erased_arguments(expected, actual, result)?,
        (MirErasedType::Tuple(expected), MirErasedType::Tuple(actual))
            if expected.len() == actual.len() =>
        {
            for (expected, actual) in expected.iter().zip(actual) {
                infer_erased_arguments(expected, actual, result)?;
            }
        }
        (expected, actual) if expected == actual => {}
        _ => return Err("erased generic argument structure mismatch".into()),
    }
    Ok(())
}

pub(crate) fn bind_erased_parameters(
    expected: &MirErasedType,
    actual: &Type,
    bindings: &mut BTreeMap<GenericParamId, u32>,
    tc: &crate::type_context::TypeContext,
) -> Result<(), String> {
    match (expected, actual) {
        (MirErasedType::Parameter(index), Type::Generic(id)) => {
            if bindings
                .iter()
                .any(|(other, bound)| other != id && bound == index)
            {
                return Err("distinct impl parameters cannot share one erased parameter".into());
            }
            if bindings
                .insert(*id, *index)
                .is_some_and(|old| old != *index)
            {
                return Err("impl runtime parameter conflicts with its selected member".into());
            }
        }
        (MirErasedType::Concrete(id), actual) if tc.type_for(*id) == *actual => {}
        (
            MirErasedType::Reference { inner, mutable },
            Type::Reference {
                inner: actual,
                mutable: actual_mut,
            },
        ) if mutable == actual_mut => bind_erased_parameters(inner, actual, bindings, tc)?,
        (MirErasedType::Pointer(inner), Type::Pointer(actual)) => {
            bind_erased_parameters(inner, actual, bindings, tc)?
        }
        (MirErasedType::Tuple(fields), Type::Tuple(actual)) if fields.len() == actual.len() => {
            for (expected, actual) in fields.iter().zip(actual) {
                bind_erased_parameters(expected, actual, bindings, tc)?;
            }
        }
        _ => {
            return Err(format!(
                "impl signature disagrees with erased member: {expected:?} versus {actual:?}"
            ))
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Use the same accepted-HIR -> mono -> authoritative MIR path as the
    // existing scoped-wrapper regression, without LLVM or source execution.
    fn package_source_mir(source: &str) -> MirProgram {
        let (mono, bodies) = package_source_bodies(source);
        crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies)
    }

    fn package_source_bodies(
        source: &str,
    ) -> (crate::mono::MonomorphizedProgram, MirInstanceBodies) {
        let path = std::path::PathBuf::from("/virtual/package_backend/main.rk");
        let analysis = crate::analyze(&crate::Config {
            entry_file: path.clone(),
            source_providers: vec![crate::SourceProvider::Virtual {
                path,
                text: source.into(),
            }],
            no_std: true,
            no_prelude: true,
            ..Default::default()
        })
        .unwrap();
        let mut mono = crate::mono::monomorphize_with_crates(
            analysis.hir,
            &crate::crate_system::CrateContext::new(),
            crate::mono::CompilationIdentityContext::local_default(),
        )
        .unwrap();
        let bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut mono);
        (mono, bodies)
    }

    #[test]
    fn package_native_signature_interning_preserves_closed_binders() {
        let mir = package_source_mir(
            r#"
trait Accept
    @accept: &Self -> I64
struct Number
    < value: I64
impl Accept for Number
    @accept = other -> ~I64Add self.value, other.value
type Pair = exists T where T: Accept in (&T, &T)
make: &Number -> &Number -> Pair
make = left, right -> pack (Pair) ((left, right))
relay: Pair -> Pair
relay = package -> package
consume: Pair -> I64
consume = package ->
    open package as Hidden, pair
        pair.0.accept pair.1
main = ->
    number = Number
        value: 21
    package = make (&number), (&number)
    consume (relay package)
"#,
        );
        let report = crate::mir::agreement::check_mir_runtime_agreement(&mir);
        assert!(report.is_clean(), "{report:?}");
        let package = *mir.backend_contract.packages.keys().next().unwrap();
        assert!(mir.functions.values().any(|function| {
            function.ret_type == package
                && function.arg_count == 1
                && function.local_decls[1].ty == package
        }));
        let Type::Exists(package) = mir.type_context.type_for(package) else {
            panic!("closed package must retain its own binder")
        };
        assert!(crate::type_services::visit::type_any(
            &package.payload,
            |ty| {
                matches!(
                    ty,
                    Type::BoundVar {
                        depth: 0,
                        index: 0,
                        ..
                    }
                )
            }
        ));
    }

    #[test]
    fn package_owned_native_signature_and_whole_payload_relay_keep_actual_drop_authority() {
        let mir = package_source_mir(
            r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
trait Accept
    @accept: &Self -> I64
struct Number
impl Accept for Number
    @accept = other -> 0
struct Indirect T
    < pointer: *T
impl Dispose for Indirect T
    ~@dispose = !-> ()
type Owned = exists T where T: Accept in Indirect T
relay: Owned -> Owned
relay = package -> package
repack: Owned -> Owned
repack = package ->
    open package as Hidden, payload
        pack (Owned) (payload)
main = !->
    number = Number
    payload = Indirect
        pointer: (&number) as *Number
    package = pack (Owned) (payload)
    repack (relay package)
"#,
        );
        let report = crate::mir::agreement::check_mir_runtime_agreement(&mir);
        assert!(report.is_clean(), "{report:?}");
        let package = *mir.backend_contract.packages.keys().next().unwrap();
        assert!(mir.functions.values().any(|function| {
            function.ret_type == package
                && function.arg_count == 1
                && function.local_decls[1].ty == package
        }));
        assert!(!package_payload_drop_free(
            &mir.backend_contract,
            &mir.type_context,
            package
        ));
        assert!(mir
            .backend_contract
            .erased
            .descriptors
            .values()
            .any(|descriptor| {
                matches!(
                    mir.type_context.type_for(descriptor.ty),
                    Type::Struct { .. }
                ) && descriptor.drop.user_drop.is_some()
            }));
    }

    fn owned_package_cleanup_mir(last_statements: &str) -> MirProgram {
        owned_package_cleanup_mir_with_arrow(last_statements, "!->")
    }

    fn owned_package_cleanup_mir_with_arrow(last_statements: &str, arrow: &str) -> MirProgram {
        let source = format!(
            r#"
extern putchar: I32 -> I32
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
trait Accept
    @accept: &Self -> I64
struct Number
impl Accept for Number
    @accept = other -> 0
struct Indirect T
    < pointer: *T
    < marker: I32
impl Dispose for Indirect T
    ~@dispose = !-> putchar self.marker
type Owned = exists T where T: Accept in Indirect T
repack: Owned -> Owned
repack = package ->
    open package as Hidden, payload
        pack (Owned) (payload)
relay: Owned -> Owned
relay = package -> package
consume: Owned -> ()
consume = package !-> ()
main = {arrow}
    number = Number
    payload = Indirect
        pointer: (&number) as *Number
        marker: 65 as I32
    package = pack (Owned) (payload)
{last_statements}
"#
        );
        let (mut mono, mut bodies) = package_source_bodies(&source);
        let dce = crate::dce::prune_unreachable_instances(&mut mono, &mut bodies);
        assert!(dce.missing_instance_edges.is_empty(), "{dce:?}");
        let mir = crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(
            &mono, &bodies,
        );
        let agreement = crate::mir::agreement::check_mir_runtime_agreement(&mir);
        assert!(agreement.is_clean(), "{agreement:?}");
        crate::mir::borrowck::BorrowChecker::run(&mir).unwrap();

        // The actual packed owner descriptor, not the witness descriptor,
        // must still lead to a retained, executable user destructor after DCE.
        let plan = mir
            .functions
            .values()
            .flat_map(|function| &function.basic_blocks)
            .flat_map(|block| &block.statements)
            .find_map(|statement| match &statement.kind {
                StatementKind::Assign(_, Rvalue::Package(_, plan)) => Some(plan),
                _ => None,
            })
            .unwrap();
        let MirPackagePayloadDescriptor::Concrete(payload) = plan.payload_descriptor else {
            panic!("source-backed owner packing must name its actual descriptor")
        };
        let owner_drop = mir.backend_contract.erased.descriptors[&payload]
            .drop
            .user_drop
            .as_ref()
            .unwrap();
        assert_eq!(
            mir.backend_contract.drop_glue.get(&payload),
            Some(owner_drop)
        );
        let declaration = &mir.backend_contract.callables[owner_drop];
        let MirCallableKind::LocalBody { function_id } = &declaration.kind else {
            panic!("source-backed destructor must retain its native body")
        };
        let destructor = &mir.functions[function_id];
        assert!(
            destructor.basic_blocks.iter().any(|block| matches!(
                &block.terminator,
                Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Extern(_)
                    ))),
                    ..
                }),
            )),
            "the marker-writing user destructor must survive DCE"
        );
        assert_eq!(
            mir.backend_contract.erased.descriptors[&plan.package]
                .drop
                .children,
            MirPayloadDropChildren::Package
        );
        assert!(mir.backend_contract.erased.functions.values().any(|function| {
            function.body.as_ref().is_some_and(|body| body.blocks.iter().any(|block| {
                block.statements.iter().any(|statement| matches!(
                    &statement.value,
                    MirErasedRvalue::PackagePack { plan, .. }
                        if matches!(plan.payload_descriptor, MirPackagePayloadDescriptor::FromPackage(_)),
                ))
            }))
        }));
        mir
    }

    // Inspect only the generated, acyclic cleanup control flow and literal
    // drop flags; calls take their normal continuation without executing them.
    // This proves which native places remain cleanup owners, not runtime output.
    fn native_cleanup_path(function: &MirFunction) -> Vec<Place> {
        native_cleanup_trace(function).drops
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum NativeReturnLiteral {
        Unit,
        Int(i64),
    }

    struct NativeCleanupTrace {
        drops: Vec<Place>,
        return_value: Option<NativeReturnLiteral>,
        return_values_before_drop: Vec<Option<NativeReturnLiteral>>,
    }

    fn native_cleanup_trace(function: &MirFunction) -> NativeCleanupTrace {
        let mut flags = vec![None; function.local_decls.len()];
        let mut block = BasicBlockId(0);
        let mut drops = Vec::new();
        let mut return_value = None;
        let mut return_values_before_drop = Vec::new();
        for _ in 0..function.basic_blocks.len() {
            let current = &function.basic_blocks[block.0];
            for statement in &current.statements {
                if let StatementKind::Assign(place, value) = &statement.kind {
                    if place.local == Local(0) && place.projection.is_empty() {
                        return_value = match value {
                            Rvalue::Use(Operand::Constant(Constant::Unit)) => {
                                Some(NativeReturnLiteral::Unit)
                            }
                            Rvalue::Use(Operand::Constant(Constant::Int(value))) => {
                                Some(NativeReturnLiteral::Int(*value))
                            }
                            _ => None,
                        };
                    }
                }
                match &statement.kind {
                    StatementKind::Assign(
                        place,
                        Rvalue::Use(Operand::Constant(Constant::Bool(value))),
                    ) => {
                        assert!(place.projection.is_empty());
                        flags[place.local.0] = Some(*value);
                    }
                    StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                        flags[local.0] = None;
                        if *local == Local(0) {
                            return_value = None;
                        }
                    }
                    _ => {}
                }
            }
            block = match current.terminator.as_ref().unwrap() {
                Terminator::Goto(target) | Terminator::GotoWithOrigin { target, .. } => *target,
                Terminator::Call {
                    destination,
                    target,
                    ..
                } => {
                    if destination.local == Local(0) && destination.projection.is_empty() {
                        return_value = None;
                    }
                    *target
                }
                Terminator::Drop { place, target }
                | Terminator::DropWithOrigin { place, target, .. } => {
                    assert!(function
                        .ownership
                        .drop_obligations
                        .iter()
                        .any(|obligation| obligation.place == *place));
                    drops.push(place.clone());
                    return_values_before_drop.push(return_value);
                    *target
                }
                Terminator::SwitchInt {
                    discr,
                    targets,
                    otherwise,
                }
                | Terminator::SwitchIntWithOrigin {
                    discr,
                    targets,
                    otherwise,
                    ..
                } => {
                    let Operand::Copy(place) = discr else {
                        panic!("cleanup flag operand")
                    };
                    assert!(place.projection.is_empty());
                    let flag = i64::from(flags[place.local.0].expect("initialized cleanup flag"));
                    targets
                        .iter()
                        .find_map(|(value, target)| (*value == flag).then_some(*target))
                        .unwrap_or(*otherwise)
                }
                Terminator::Return | Terminator::ReturnWithOrigin { .. } => {
                    return NativeCleanupTrace {
                        drops,
                        return_value,
                        return_values_before_drop,
                    }
                }
                other => panic!("unexpected cleanup fixture terminator: {other:?}"),
            };
        }
        panic!("cleanup fixture must terminate without a loop")
    }

    #[test]
    fn package_discarded_scoped_return_has_one_live_descriptor_cleanup_after_dce() {
        let mir = owned_package_cleanup_mir("    repack package");
        let main = mir
            .functions
            .values()
            .find(|function| function.name == "main")
            .unwrap();
        let returned = main
            .basic_blocks
            .iter()
            .find_map(|block| match &block.terminator {
                Some(Terminator::Call {
                    func: Operand::Constant(Constant::ErasedCall(_)),
                    destination,
                    ..
                }) => Some(destination.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            main.local_decls[returned.local.0].source,
            LocalSource::Temporary
        );
        let package = main.local_decls[returned.local.0].ty;
        assert!(mir.backend_contract.packages.contains_key(&package));
        assert!(main.ownership.drop_obligations.iter().any(|obligation| {
            obligation.place == returned
                && obligation.ty == package
                && obligation.kind == DropObligationKind::Structural
        }));
        assert_eq!(native_cleanup_path(main), vec![returned],
            "the returned package, not its moved original payload/package, owns exactly one cleanup");
    }

    #[test]
    fn package_moved_scoped_return_transfers_cleanup_to_native_consumer_after_dce() {
        let mir =
            owned_package_cleanup_mir("    result = repack package\n    consume (relay result)");
        let main = mir
            .functions
            .values()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(
            native_cleanup_path(main).is_empty(),
            "moved sources must not be destroyed by main"
        );
        let package = *mir.backend_contract.packages.keys().next().unwrap();
        let relay = mir
            .functions
            .values()
            .find(|function| {
                function.arg_count == 1
                    && function.local_decls[1].ty == package
                    && function.ret_type == package
            })
            .unwrap();
        assert!(
            native_cleanup_path(relay).is_empty(),
            "return transfers the parameter's ownership"
        );
        let consumer = mir
            .functions
            .values()
            .find(|function| {
                function.arg_count == 1
                    && function.local_decls[1].ty == package
                    && mir.type_context.type_for(function.ret_type) == Type::Unit
            })
            .unwrap();
        assert_eq!(
            native_cleanup_path(consumer),
            vec![Place {
                local: Local(1),
                projection: vec![]
            }],
            "only the final consumer destroys the package through its retained descriptor"
        );
    }

    #[test]
    fn package_effects_main_initializes_its_unit_return_before_guarded_owner_cleanup() {
        let mir = owned_package_cleanup_mir("    result = repack package");
        let main = mir
            .functions
            .values()
            .find(|function| function.name == "main")
            .unwrap();
        assert_eq!(mir.type_context.type_for(main.ret_type), Type::Unit);
        assert_eq!(main.local_decls[0].source, LocalSource::ReturnPlace);
        let trace = native_cleanup_trace(main);
        assert_eq!(
            trace.return_value,
            Some(NativeReturnLiteral::Unit),
            "the lowered !-> unit tail must initialize the actual return place, not a temporary"
        );
        assert_eq!(
            trace.return_values_before_drop,
            vec![Some(NativeReturnLiteral::Unit)],
            "the semantic unit result must be written before the live owner cleanup branch"
        );
        assert_eq!(trace.drops.len(), 1);
        assert!(mir
            .backend_contract
            .packages
            .contains_key(&main.local_decls[trace.drops[0].local.0].ty));

        // Also cover the original discarded-result shape, whose statement
        // temporary is cleaned before the appended unit tail is evaluated.
        let discarded = owned_package_cleanup_mir("    repack package");
        let main = discarded
            .functions
            .values()
            .find(|function| function.name == "main")
            .unwrap();
        let trace = native_cleanup_trace(main);
        assert_eq!(trace.return_value, Some(NativeReturnLiteral::Unit));
        assert_eq!(trace.drops.len(), 1);

        // The same destination rule must initialize ordinary unit functions,
        // not just change the process entrypoint's status.
        let consumed = owned_package_cleanup_mir("    result = repack package\n    consume result");
        let package = *consumed.backend_contract.packages.keys().next().unwrap();
        let consumer = consumed
            .functions
            .values()
            .find(|function| {
                function.arg_count == 1
                    && function.local_decls[1].ty == package
                    && consumed.type_context.type_for(function.ret_type) == Type::Unit
            })
            .unwrap();
        let trace = native_cleanup_trace(consumer);
        assert_eq!(trace.return_value, Some(NativeReturnLiteral::Unit));
        assert_eq!(
            trace.return_values_before_drop,
            vec![Some(NativeReturnLiteral::Unit)]
        );
        assert_eq!(trace.drops.len(), 1);
    }

    #[test]
    fn package_integer_main_keeps_its_explicit_tail_through_guarded_owner_cleanup() {
        let mir = owned_package_cleanup_mir_with_arrow("    result = repack package\n    37", "->");
        let main = mir
            .functions
            .values()
            .find(|function| function.name == "main")
            .unwrap();
        assert_eq!(mir.type_context.type_for(main.ret_type), Type::I64);
        let trace = native_cleanup_trace(main);
        assert_eq!(trace.return_value, Some(NativeReturnLiteral::Int(37)),
            "cleanup must not replace an intentional integer exit status with unit or a destructor result");
        assert_eq!(
            trace.return_values_before_drop,
            vec![Some(NativeReturnLiteral::Int(37))]
        );
        assert_eq!(trace.drops.len(), 1);
        assert!(mir
            .backend_contract
            .packages
            .contains_key(&main.local_decls[trace.drops[0].local.0].ty));
    }

    fn mutable_package_calls() -> MirProgram {
        package_source_mir(
            r#"
trait Change
    ^@change: I64 -> ()
    @read: I64
    @borrow: &Self
struct Number
    < value: I64
impl Change for Number
    ^@change = value !-> self.value = value
    @read = -> self.value
    @borrow = -> self
type Exclusive = exists T where T: Change in &mut T
main = ->
    mut number = Number
        value: 0
    package = pack (Exclusive) (&mut number)
    open package as Hidden, value
        value.change 41
        value.change 42
        value.read!
"#,
        )
    }

    #[test]
    fn package_call_only_mutable_reborrow_ends_before_the_next_call() {
        let mir = mutable_package_calls();
        let report = crate::mir::agreement::check_mir_runtime_agreement(&mir);
        assert!(report.is_clean(), "{report:?}");
        let function = mir
            .backend_contract
            .erased
            .functions
            .values()
            .find(|function| {
                function.body.as_ref().is_some_and(|body| {
                    body.scopes
                        .iter()
                        .any(|scope| matches!(scope.source, MirErasedScopeSource::Package { .. }))
                })
            })
            .unwrap();
        let statements = function
            .body
            .as_ref()
            .unwrap()
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .collect::<Vec<_>>();
        let calls = statements
            .iter()
            .filter_map(|statement| match &statement.value {
                MirErasedRvalue::OpenedCall { receiver, .. } => Some(receiver),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 3);
        assert!(calls.windows(2).all(|calls| calls[0] == calls[1]));
        assert!(
            !statements.iter().any(|statement| match &statement.value {
                MirErasedRvalue::Borrow { place, .. } => matches!(
                    erased_place_type(function, place, &mir.backend_contract, &mir.type_context),
                    Ok(MirErasedType::Scoped(_)),
                ),
                _ => false,
            }),
            "call-only reborrows must not become enclosing-lived reference locals"
        );
    }

    #[test]
    fn package_borrow_carrying_call_result_still_blocks_mutation_after_copy() {
        let mut mir = mutable_package_calls();
        assert_eq!(validate_erased_contract(&mir), vec![]);
        let key = mir
            .backend_contract
            .erased
            .functions
            .values()
            .find(|function| {
                function
                    .body
                    .as_ref()
                    .is_some_and(|body| !body.scopes.is_empty())
            })
            .unwrap()
            .key
            .clone();
        let object = mir.backend_contract.erased.functions[&key]
            .body
            .as_ref()
            .unwrap()
            .scopes[0]
            .object;
        let slot = mir.backend_contract.object_schemas[&object]
            .slots
            .iter()
            .position(|slot| slot.result == MirObjectResult::BorrowedSelf { mutable: false })
            .unwrap() as u32;
        let function = mir.backend_contract.erased.functions.get_mut(&key).unwrap();
        let body = function.body.as_mut().unwrap();
        let (block, first_call) = body
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| {
                block
                    .statements
                    .iter()
                    .position(|statement| {
                        matches!(statement.value, MirErasedRvalue::OpenedCall { .. })
                    })
                    .map(|statement| (index, statement))
            })
            .unwrap();
        let MirErasedRvalue::OpenedCall {
            scope, receiver, ..
        } = body.blocks[block].statements[first_call].value.clone()
        else {
            panic!("mutable source fixture")
        };
        let result = MirErasedLocal(body.locals.len() as u32);
        let copy = MirErasedLocal(result.0 + 1);
        let reference = MirErasedType::Reference {
            inner: Box::new(MirErasedType::Scoped(scope)),
            mutable: false,
        };
        body.locals.extend([reference.clone(), reference]);
        body.blocks[block].statements.splice(
            first_call..first_call,
            [
                MirErasedAssignment {
                    destination: result,
                    value: MirErasedRvalue::OpenedCall {
                        scope,
                        slot,
                        receiver,
                        args: vec![],
                    },
                },
                MirErasedAssignment {
                    destination: copy,
                    value: MirErasedRvalue::Use(MirErasedOperand::Copy(MirErasedPlace::local(
                        result,
                    ))),
                },
            ],
        );
        let function = function.clone();
        assert_eq!(
            validate_erased_function(&function, &mir.backend_contract, &mir.type_context),
            Err(MirErasedError::BorrowConflict)
        );
        assert_eq!(
            validate_erased_contract(&mir),
            vec![(Some(key), MirErasedError::BorrowConflict)]
        );
    }

    #[test]
    fn package_shared_call_accepts_a_shared_loan_but_rejects_a_live_unique_loan() {
        for mutable in [false, true] {
            let mut mir = mutable_package_calls();
            assert_eq!(validate_erased_contract(&mir), vec![]);
            let key = mir
                .backend_contract
                .erased
                .functions
                .values()
                .find(|function| {
                    function
                        .body
                        .as_ref()
                        .is_some_and(|body| !body.scopes.is_empty())
                })
                .unwrap()
                .key
                .clone();
            let function = mir.backend_contract.erased.functions.get_mut(&key).unwrap();
            let body = function.body.as_mut().unwrap();
            let (block, read) = body
                .blocks
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, block)| {
                    block
                        .statements
                        .iter()
                        .rposition(|statement| {
                            matches!(statement.value, MirErasedRvalue::OpenedCall { .. })
                        })
                        .map(|statement| (index, statement))
                })
                .unwrap();
            let MirErasedRvalue::OpenedCall {
                scope, receiver, ..
            } = body.blocks[block].statements[read].value.clone()
            else {
                panic!("shared source fixture")
            };
            let reference = MirErasedLocal(body.locals.len() as u32);
            body.locals.push(MirErasedType::Reference {
                inner: Box::new(MirErasedType::Scoped(scope)),
                mutable,
            });
            body.blocks[block].statements.insert(
                read,
                MirErasedAssignment {
                    destination: reference,
                    value: MirErasedRvalue::Borrow {
                        place: receiver,
                        mutable,
                    },
                },
            );
            // Keep the injected loan live across the call, but end its storage
            // with that statement before the enclosing package is destroyed.
            let MirErasedTerminator::Cleanup { locals, .. } = &mut body.blocks[block].terminator
            else {
                panic!("read statement must have explicit cleanup")
            };
            locals.push(reference);
            let function = function.clone();
            if mutable {
                assert_eq!(
                    validate_erased_function(&function, &mir.backend_contract, &mir.type_context),
                    Err(MirErasedError::BorrowConflict)
                );
                assert_eq!(
                    validate_erased_contract(&mir),
                    vec![(Some(key), MirErasedError::BorrowConflict)]
                );
            } else {
                let report = crate::mir::agreement::check_mir_runtime_agreement(&mir);
                assert!(report.is_clean(), "{report:?}");
            }
        }
    }

    #[test]
    fn scoped_wrapper_instance_call_reaches_erased_entry_without_native_recursion() {
        let path = std::path::PathBuf::from("/virtual/scoped_wrapper/main.rk");
        let source = r#"
trait Read
    @read: I64
read_open: &Read -> I64
read_open = object ->
    open object as T, value
        value.read!
ordinary: &Read -> I64
ordinary = object -> object.read!
relay: &Read -> I64
relay = object -> ordinary object
main = -> 0
"#;
        let analysis = crate::analyze(&crate::Config {
            entry_file: path.clone(),
            source_providers: vec![crate::SourceProvider::Virtual {
                path,
                text: source.into(),
            }],
            no_std: true,
            no_prelude: true,
            ..Default::default()
        })
        .unwrap();
        let definitions = analysis
            .hir
            .program
            .functions
            .values()
            .map(|function| (function.name.clone(), function.id))
            .collect::<HashMap<_, _>>();
        let mut mono = crate::mono::monomorphize_with_crates(
            analysis.hir,
            &crate::crate_system::CrateContext::new(),
            crate::mono::CompilationIdentityContext::local_default(),
        )
        .unwrap();
        let instance = |name: &str| {
            mono.instances
                .values()
                .find(|record| {
                    record.origin == crate::mono::InstanceOrigin::Function(definitions[name])
                        && record.substitution.is_empty()
                })
                .unwrap()
                .id
        };
        let wrapper = instance("read_open");
        let ordinary = instance("ordinary");
        let relay = instance("relay");
        let crate::mono::InstanceOrigin::Function(definition) = mono.instances[&wrapper].origin
        else {
            panic!("expected the source function's canonical instance");
        };
        let key = mono.erased.scoped_entries[&definition].clone();
        let wrapper_body = mono.pre_mir_instance_bodies.get(wrapper).unwrap();
        assert!(
            matches!(
                &wrapper_body.body.stmts[0],
                HirStmt::Return(Some(HirExpr {
                    kind: HirExprKind::Call(_, _, Some(crate::hir::HirCallTarget::Instance(id))),
                    ..
                })) if *id == wrapper
            ),
            "{:?}",
            wrapper_body.body.stmts
        );
        let bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut mono);
        let mir = crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(
            &mono, &bodies,
        );
        let calls = |instance| {
            mir.functions[&MirFunctionId::Instance(instance)]
                .basic_blocks
                .iter()
                .filter_map(|block| match &block.terminator {
                    Some(Terminator::Call { func, .. }) => Some(func.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(matches!(
            calls(wrapper).as_slice(),
            [Operand::Constant(Constant::ErasedCall(call))]
                if call.target == MirErasedCallTarget::Direct(key)
                    && call.type_args.is_empty() && call.dictionaries.is_empty()
        ));
        assert!(matches!(
            calls(relay).as_slice(),
            [Operand::Constant(Constant::Callable(MirCallable::Resolved(
                MirCallableKey::Instance(id)
            )))] if *id == ordinary
        ));
        let report = crate::mir::agreement::check_mir_runtime_agreement(&mir);
        assert!(report.is_clean(), "{report:?}");
    }

    #[test]
    fn erased_nested_return_stops_enclosing_value_and_binding_lowering() {
        let span = Span::test();
        let returned = HirExpr {
            kind: HirExprKind::IntLiteral(42),
            ty: Type::I64,
            span: span.clone(),
        };
        let early = HirExpr {
            kind: HirExprKind::Block(HirBlock {
                ty: Type::Never,
                stmts: vec![HirStmt::Return(Some(returned))],
            }),
            ty: Type::Never,
            span: span.clone(),
        };
        let unreachable_value = HirStmt::Expr(HirExpr {
            kind: HirExprKind::IntLiteral(99),
            ty: Type::I64,
            span: span.clone(),
        });
        for statement in [
            HirStmt::Let {
                name: "never_bound".into(),
                local_id: HirLocalId(0),
                ty: Type::Never,
                value: early.clone(),
                mutable: false,
            },
            HirStmt::Expr(HirExpr {
                kind: HirExprKind::TupleLiteral(vec![early.clone()]),
                ty: Type::Tuple(vec![Type::Never]),
                span: span.clone(),
            }),
            HirStmt::Return(Some(early.clone())),
            HirStmt::Expr(HirExpr {
                kind: HirExprKind::If {
                    condition: Box::new(early.clone()),
                    then_branch: HirBlock {
                        ty: Type::Unit,
                        stmts: vec![],
                    },
                    else_branch: None,
                },
                ty: Type::Never,
                span: span.clone(),
            }),
        ] {
            let mut mono = Monomorphizer::new();
            let scalar = mono.intern_type(&Type::I64);
            let function = HirFunction {
                id: DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(100)),
                name: "nested_return".into(),
                generic_params: vec![],
                generic_bounds: Default::default(),
                params: vec![],
                ret_type: Type::I64,
                body: HirBlock {
                    stmts: vec![statement, unreachable_value.clone()],
                    ty: Type::I64,
                },
                is_curried: false,
                is_method: false,
                self_receiver: None,
                is_unsafe: false,
            };
            let signature = MirErasedSignature {
                parameters: vec![],
                params: vec![],
                ret: MirErasedType::Concrete(scalar),
                layouts: vec![],
                dictionaries: vec![],
                require_borrow_free: Default::default(),
            };
            let body =
                ErasedBodyBuilder::new(&mut mono, signature.clone(), BTreeMap::new(), &function)
                    .unwrap()
                    .block(&function.body)
                    .unwrap();
            assert_eq!(body.locals, vec![MirErasedType::Concrete(scalar); 2]);
            assert_eq!(body.blocks.len(), 2);
            assert_eq!(
                body.blocks[0].statements.len(),
                2,
                "only the literal and return transport precede cleanup"
            );
            assert_eq!(
                body.blocks[1].terminator,
                MirErasedTerminator::Return(MirErasedOperand::Copy(MirErasedPlace::local(
                    MirErasedLocal(1)
                )))
            );
            let mut contract = MirBackendContract::default();
            let drop = payload_drop_plan(&contract, &mut mono.type_context, scalar).unwrap();
            contract.erased.descriptors.insert(
                scalar,
                MirTypeDescriptor {
                    ty: scalar,
                    drop,
                    fields: vec![],
                },
            );
            assert!(validate_erased_function(
                &MirErasedFunction {
                    key: MirErasedKey {
                        definition: function.id,
                        static_args: vec![]
                    },
                    symbol: "nested_return".into(),
                    linkage: MirLinkage::Internal,
                    signature,
                    body: Some(body),
                },
                &contract,
                &mono.type_context
            )
            .is_ok());
        }
    }

    #[test]
    fn erased_if_with_two_returning_arms_has_no_fabricated_join_result() {
        let mut mono = Monomorphizer::new();
        let scalar = mono.intern_type(&Type::I64);
        let boolean = mono.intern_type(&Type::Bool);
        let returning = |value| HirBlock {
            ty: Type::Never,
            stmts: vec![HirStmt::Return(Some(HirExpr {
                kind: HirExprKind::IntLiteral(value),
                ty: Type::I64,
                span: Span::test(),
            }))],
        };
        let function = HirFunction {
            id: DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(101)),
            name: "returning_arms".into(),
            generic_params: vec![],
            generic_bounds: Default::default(),
            params: vec![],
            ret_type: Type::I64,
            body: HirBlock {
                ty: Type::I64,
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKind::If {
                        condition: Box::new(HirExpr {
                            kind: HirExprKind::BoolLiteral(true),
                            ty: Type::Bool,
                            span: Span::test(),
                        }),
                        then_branch: returning(42),
                        else_branch: Some(returning(43)),
                    },
                    ty: Type::Never,
                    span: Span::test(),
                })],
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let signature = MirErasedSignature {
            parameters: vec![],
            params: vec![],
            ret: MirErasedType::Concrete(scalar),
            layouts: vec![],
            dictionaries: vec![],
            require_borrow_free: Default::default(),
        };
        let body = ErasedBodyBuilder::new(&mut mono, signature.clone(), BTreeMap::new(), &function)
            .unwrap()
            .block(&function.body)
            .unwrap();
        assert_eq!(
            body.locals,
            vec![
                MirErasedType::Concrete(boolean),
                MirErasedType::Concrete(scalar),
                MirErasedType::Concrete(scalar),
                MirErasedType::Concrete(scalar),
                MirErasedType::Concrete(scalar)
            ]
        );
        assert!(body.blocks[3].statements.is_empty());
        assert_eq!(body.blocks[3].terminator, MirErasedTerminator::Unreachable);
        let mut contract = MirBackendContract::default();
        for ty in [scalar, boolean] {
            let drop = payload_drop_plan(&contract, &mut mono.type_context, ty).unwrap();
            contract.erased.descriptors.insert(
                ty,
                MirTypeDescriptor {
                    ty,
                    drop,
                    fields: vec![],
                },
            );
        }
        assert!(validate_erased_function(
            &MirErasedFunction {
                key: MirErasedKey {
                    definition: function.id,
                    static_args: vec![]
                },
                symbol: "returning_arms".into(),
                linkage: MirLinkage::Internal,
                signature,
                body: Some(body),
            },
            &contract,
            &mono.type_context
        )
        .is_ok());
    }

    #[test]
    fn erased_forwarder_binds_result_only_parameters_consistently() {
        let mut bindings = BTreeMap::new();
        infer_erased_arguments(
            &MirErasedType::Parameter(0),
            &MirErasedType::Parameter(4),
            &mut bindings,
        )
        .unwrap();
        infer_erased_arguments(
            &MirErasedType::Parameter(1),
            &MirErasedType::Parameter(5),
            &mut bindings,
        )
        .unwrap();
        infer_erased_arguments(
            &MirErasedType::Parameter(2),
            &MirErasedType::Parameter(6),
            &mut bindings,
        )
        .unwrap();
        assert_eq!(bindings.get(&2), Some(&MirErasedType::Parameter(6)));
        assert!(infer_erased_arguments(
            &MirErasedType::Parameter(2),
            &MirErasedType::Parameter(7),
            &mut bindings
        )
        .is_err());
    }

    #[test]
    fn erased_signature_binding_preserves_independent_parameters() {
        let tc = crate::type_context::TypeContext::new();
        let owner = DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(1));
        let first = GenericParamId { owner, index: 0 };
        let second = GenericParamId { owner, index: 1 };
        let mut bindings = BTreeMap::new();
        bind_erased_parameters(
            &MirErasedType::Parameter(1),
            &Type::Generic(first),
            &mut bindings,
            &tc,
        )
        .unwrap();
        bind_erased_parameters(
            &MirErasedType::Parameter(1),
            &Type::Generic(first),
            &mut bindings,
            &tc,
        )
        .unwrap();
        assert!(bind_erased_parameters(
            &MirErasedType::Parameter(1),
            &Type::Generic(second),
            &mut bindings,
            &tc
        )
        .is_err());
        assert!(bind_erased_parameters(
            &MirErasedType::Parameter(2),
            &Type::Generic(first),
            &mut bindings,
            &tc
        )
        .is_err());
    }

    #[test]
    fn nested_scoped_tuple_copy_requires_every_leaf_to_have_copy_authority() {
        let context = crate::type_context::TypeContext::new();
        let reference = MirErasedType::Reference {
            inner: Box::new(MirErasedType::Scoped(0)),
            mutable: false,
        };
        let pair = MirErasedType::Tuple(vec![reference.clone(), reference]);
        let nested = MirErasedType::Tuple(vec![pair.clone(), pair]);
        assert!(crate::mir::copy_proven(&nested, &context));
        for non_copy in [
            MirErasedType::Scoped(0),
            MirErasedType::Parameter(0),
            MirErasedType::Reference {
                inner: Box::new(MirErasedType::Scoped(0)),
                mutable: true,
            },
        ] {
            assert!(!crate::mir::copy_proven(
                &MirErasedType::Tuple(vec![nested.clone(), non_copy]),
                &context,
            ));
        }
    }
}
