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
use crate::types::{GenericParamId, Type};
use std::collections::{BTreeMap, HashMap};

impl Monomorphizer {
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
    ) -> Result<(), String> {
        let signature = self.object_schemas[&object].erased_slots[slot as usize]
            .signature
            .clone();
        let mut bindings = vec![None];
        let mut type_args = Vec::new();
        for parameter in signature.parameters.iter().skip(1) {
            let binding = target
                .method_substitution
                .iter()
                .chain(&target.owner_substitution)
                .find(|binding| binding.param == parameter.source)
                .ok_or("generic virtual argument was not finalized by inference")?;
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
        for layout in &signature.layouts {
            if *layout == MirErasedType::Parameter(0) {
                continue;
            }
            let ty = layout
                .instantiate_optional(&bindings, &self.type_context)
                .map_err(|e| format!("erased layout evidence: {e:?}"))?;
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
        let key = MirErasedInvocationKey {
            object,
            slot,
            type_args: type_args.clone(),
        };
        if self.erased_invocations.contains_key(&key) {
            return Ok(());
        }
        let mut dictionaries = Vec::new();
        for required in &signature.dictionaries {
            dictionaries.push(self.materialize_erased_dictionary(required, &bindings, span)?);
        }
        self.erased_invocations.insert(
            key,
            MirErasedCall {
                target: MirErasedCallTarget::Virtual(MirVirtualTarget { object, slot }),
                type_args,
                dictionaries,
            },
        );
        Ok(())
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
                    if member.receiver != receiver
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
                target,
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

    fn materialize_dictionary_method(
        &mut self,
        source: &Type,
        trait_id: DefId,
        trait_args: &[Type],
        member: &MirErasedDictionaryMember,
        bindings: &[Option<Type>],
        span: &Span,
    ) -> Result<MirCallableKey, String> {
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
                Some(member.receiver),
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
            *dictionary = dictionary.substitute(&map).map_err(|e| format!("{e:?}"))?;
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
        let static_types = substitution
            .iter()
            .map(|(id, ty)| (*id, self.intern_type(ty)))
            .collect();
        let body = self.substitute_block(&method.body, &static_types);
        let code = ErasedBodyBuilder::new(self, signature, variables, &method)?.block(&body)?;
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
                for ty in member
                    .params
                    .iter()
                    .skip(1)
                    .chain(std::iter::once(&member.ret))
                {
                    ty.collect_layouts(&mut layouts, false);
                }
            }
        }
        let require_borrow_free = dictionaries
            .iter()
            .flat_map(|d| &d.members)
            .flat_map(|m| {
                m.params
                    .iter()
                    .skip(1)
                    .chain(std::iter::once(&m.ret))
                    .cloned()
            })
            .collect();
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

struct ErasedBodyBuilder<'a> {
    mono: &'a mut Monomorphizer,
    signature: MirErasedSignature,
    variables: BTreeMap<GenericParamId, u32>,
    bindings: HashMap<HirLocalId, MirErasedLocal>,
    locals: Vec<MirErasedType>,
    mutable_locals: std::collections::BTreeSet<MirErasedLocal>,
    statements: Vec<MirErasedAssignment>,
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
        let mutable_locals = function
            .params
            .iter()
            .enumerate()
            .filter(|(_, param)| param.mutable)
            .map(|(index, _)| MirErasedLocal(index as u32))
            .collect();
        Ok(Self {
            mono,
            signature,
            variables,
            bindings,
            locals,
            mutable_locals,
            statements: Vec::new(),
        })
    }
    fn ty(&mut self, ty: &Type) -> Result<MirErasedType, String> {
        MirErasedType::from_type(
            ty,
            &self.variables,
            None,
            &mut self.mono.type_context,
            &self.mono.normalization_env,
        )
        .map_err(|e| format!("erased type: {e:?}"))
    }
    fn temp(&mut self, ty: MirErasedType, value: MirErasedRvalue) -> MirErasedPlace {
        let destination = MirErasedLocal(self.locals.len() as u32);
        self.locals.push(ty);
        self.mutable_locals.insert(destination);
        self.statements
            .push(MirErasedAssignment { destination, value });
        MirErasedPlace::local(destination)
    }
    fn operand(&self, place: MirErasedPlace, ty: &MirErasedType) -> MirErasedOperand {
        let copy = matches!(ty, MirErasedType::Concrete(id) if self.mono.type_for(*id).is_copy())
            || matches!(
                ty,
                MirErasedType::Reference { mutable: false, .. } | MirErasedType::Pointer(_)
            );
        if copy {
            MirErasedOperand::Copy(place)
        } else {
            MirErasedOperand::Move(place)
        }
    }
    fn expression(&mut self, expr: &HirExpr) -> Result<MirErasedPlace, String> {
        let ty = self.ty(&expr.ty)?;
        Ok(match &expr.kind {
            HirExprKind::ResolvedVar(reference) => {
                let HirVarTarget::Local(id) = reference.target else {
                    return Err("erased first-class named callable materialization requires dictionary evidence".into());
                };
                let local = *self
                    .bindings
                    .get(&id)
                    .ok_or("erased local binding missing")?;
                let mut place = MirErasedPlace::local(local);
                if self.locals[local.0 as usize] != ty {
                    match &self.locals[local.0 as usize] {
                        MirErasedType::Concrete(id)
                            if matches!(self.mono.type_for(*id), Type::Reference { .. }) =>
                        {
                            place.projection.push(MirErasedProjection::Deref)
                        }
                        MirErasedType::Reference { inner, .. } if **inner == ty => {
                            place.projection.push(MirErasedProjection::Deref)
                        }
                        _ => return Err("erased local type authority mismatch".into()),
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
                    let place = self.expression(value)?;
                    let ty = self.ty(&value.ty)?;
                    operands.push(self.operand(place, &ty));
                }
                self.temp(ty, MirErasedRvalue::Aggregate(operands))
            }
            HirExprKind::TupleIndex(base, index) => {
                let mut place = self.expression(base)?;
                place.projection.push(MirErasedProjection::Field(*index));
                place
            }
            HirExprKind::Deref(base) => {
                let mut place = self.expression(base)?;
                place.projection.push(MirErasedProjection::Deref);
                place
            }
            HirExprKind::Ref(mutable, inner) => {
                let place = self.expression(inner)?;
                self.temp(
                    ty,
                    MirErasedRvalue::Borrow {
                        place,
                        mutable: *mutable,
                    },
                )
            }
            HirExprKind::MethodCall(receiver, _, args, _, target) => {
                let trait_id = target
                    .trait_id()
                    .ok_or("erased method lacks trait authority")?;
                let member_id = target
                    .method_id()
                    .ok_or("erased method lacks member authority")?;
                let receiver_ty = self.ty(&receiver.ty)?;
                let trait_args = match &target.target {
                    HirSelectedMethodTarget::TraitMethod { trait_args, .. } => trait_args
                        .iter()
                        .map(|ty| self.ty(ty))
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => return Err("erased method lacks a selected trait instantiation".into()),
                };
                let dictionary = self
                    .signature
                    .dictionaries
                    .iter()
                    .position(|d| {
                        d.trait_id == trait_id
                            && d.subject == receiver_ty
                            && d.trait_args == trait_args
                    })
                    .ok_or("erased call lacks its declared dictionary")?;
                let member = self.signature.dictionaries[dictionary]
                    .members
                    .iter()
                    .position(|m| m.member_id == member_id)
                    .ok_or("erased dictionary member missing")?;
                let receiver = self.expression(receiver)?;
                let mut operands = Vec::new();
                for arg in args {
                    let place = self.expression(arg)?;
                    let ty = self.ty(&arg.ty)?;
                    operands.push(self.operand(place, &ty));
                }
                self.temp(
                    ty,
                    MirErasedRvalue::DictionaryCall {
                        dictionary: dictionary as u32,
                        member: member as u32,
                        receiver,
                        args: operands,
                    },
                )
            }
            HirExprKind::Call(callee, args, target) => {
                if let Some(crate::hir::HirCallTarget::Function(id)) = target {
                    let key = self.mono.materialize_erased_function(*id)?;
                    let signature = self.mono.erased.functions[&key].signature.clone();
                    let mut actual = Vec::new();
                    let mut operands = Vec::new();
                    for arg in args {
                        let place = self.expression(arg)?;
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
                            .substitute(&type_args)
                            .map_err(|e| format!("{e:?}"))?;
                        dictionaries.push(
                            self.signature
                                .dictionaries
                                .iter()
                                .position(|d| *d == required)
                                .ok_or("erased callee dictionary unavailable")?
                                as u32,
                        );
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
                    let receiver = self.expression(callee)?;
                    let packed = if args.len() == 1 {
                        self.expression(&args[0])?
                    } else if args.is_empty() {
                        let unit = MirErasedType::Concrete(self.mono.intern_type(&Type::Unit));
                        self.temp(unit, MirErasedRvalue::Literal(MirErasedLiteral::Unit))
                    } else {
                        let mut values = Vec::new();
                        for arg in args {
                            let place = self.expression(arg)?;
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
                            receiver,
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
        })
    }
    fn block(mut self, block: &HirBlock) -> Result<MirErasedBody, String> {
        let mut result = None;
        for statement in &block.stmts {
            match statement {
                HirStmt::Let {
                    local_id,
                    value,
                    mutable,
                    ..
                } => {
                    let place = self.expression(value)?;
                    let ty = self.ty(&value.ty)?;
                    let binding =
                        self.temp(ty.clone(), MirErasedRvalue::Use(self.operand(place, &ty)));
                    self.bindings.insert(*local_id, binding.local);
                    if !*mutable {
                        self.mutable_locals.remove(&binding.local);
                    }
                    result = None;
                }
                HirStmt::Expr(value) => {
                    let place = self.expression(value)?;
                    let ty = self.ty(&value.ty)?;
                    result = Some(self.operand(place, &ty));
                }
                HirStmt::Return(Some(value)) => {
                    let place = self.expression(value)?;
                    let ty = self.ty(&value.ty)?;
                    result = Some(self.operand(place, &ty));
                    break;
                }
                _ => {
                    return Err(
                        "erased body requires an explicit supported result/control-flow operation"
                            .into(),
                    )
                }
            }
        }
        Ok(MirErasedBody {
            locals: self.locals,
            mutable_locals: self.mutable_locals,
            statements: self.statements,
            result: result.ok_or("erased body result missing")?,
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

fn bind_erased_parameters(
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
}
