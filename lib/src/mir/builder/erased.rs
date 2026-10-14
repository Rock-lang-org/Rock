use super::MirBuilder;
use crate::mir::*;
use crate::type_context::TypeContext;
use crate::types::{CallableKind, Type};
use std::collections::BTreeSet;

impl MirBuilder<'_> {
    pub(super) fn populate_erased_contract(
        contract: &mut MirBackendContract,
        program: &crate::mono::MonomorphizedProgram,
        functions: &std::collections::BTreeMap<MirFunctionId, MirFunction>,
        tc: &mut TypeContext,
    ) {
        contract.owner_protocol = program.program.language_items.object_owner.as_ref().map(|protocol| {
            MirOwnerProtocolAuthority {
                transport: protocol.trait_id,
                split: protocol.into_parts_id,
                rebuild: protocol.from_parts_id,
                unique: program.program.language_items.object_owner_unique.as_ref()
                    .map(|unique| (unique.trait_id, unique.method_id)),
            }
        });
        for (id, implementation) in program.program.impls_in_order() {
            if let Some(trait_id) = implementation.trait_id {
                if contract.owner_protocol.as_ref().is_some_and(|protocol| {
                    protocol.transport == trait_id || protocol.unique.is_some_and(|(unique, _)| unique == trait_id)
                }) {
                    contract.owner_constructor_patterns.insert(id, implementation.receiver_pattern.clone());
                }
                let members = program
                    .program
                    .indexes
                    .effective_trait_methods
                    .iter()
                    .filter_map(|((owner, member), method)| {
                        (*owner == id).then_some((*member, *method))
                    })
                    .collect();
                let factory_compatible =
                    program
                        .program
                        .traits
                        .get(&trait_id)
                        .is_some_and(|declaration| {
                            declaration.associated_types.is_empty()
                                && declaration.predicates.is_empty()
                        })
                        && implementation.associated_types.is_empty();
                contract.dictionary_impl_members.insert(
                    id,
                    MirDictionaryImplAuthority {
                        trait_id,
                        members,
                        factory_compatible,
                        template: None,
                    },
                );
            }
        }
        let orders = program
            .program
            .traits
            .iter()
            .map(|(id, declaration)| {
                let mut members = declaration
                    .signatures
                    .values()
                    .map(|s| s.id)
                    .collect::<Vec<_>>();
                members.sort();
                (*id, members)
            })
            .collect();
        contract.erased.dictionary_templates.retain(|id, template| {
            let Some(implementation) = program.program.impls.get(id) else {
                return false;
            };
            let crate::hir::HirImplReceiverPattern::Exact(Type::Generic(subject)) =
                &implementation.receiver_pattern
            else {
                return false;
            };
            let Some(trait_id) = implementation.trait_id else {
                return false;
            };
            let Some(declaration) = program.program.traits.get(&trait_id) else {
                return false;
            };
            if !declaration.associated_types.is_empty()
                || !declaration.predicates.is_empty()
                || !implementation.associated_types.is_empty()
            {
                return false;
            }
            if template.bounds.iter().any(|bound| {
                program
                    .program
                    .traits
                    .get(&bound.trait_id)
                    .is_none_or(|declaration| {
                        !declaration.associated_types.is_empty()
                            || !declaration.predicates.is_empty()
                    })
            }) {
                return false;
            }
            if implementation.bounds.predicates.iter().any(|predicate| {
                let crate::types::Predicate::Trait {
                    subject: Type::Generic(parameter),
                    trait_id,
                    args,
                } = predicate
                else {
                    return true;
                };
                !implementation.bounds.get(parameter).is_some_and(|roots| {
                    roots.contains(&crate::types::TraitBound {
                        trait_id: *trait_id,
                        type_args: args.clone(),
                    })
                })
            }) {
                return false;
            }
            let parameters = implementation
                .type_generics
                .iter()
                .map(|p| MirErasedParameter {
                    source: p.id,
                    kind: p.kind.clone(),
                })
                .collect::<Vec<_>>();
            let result_bounds = crate::hir::HirGenericBounds::from_iter([(
                *subject,
                vec![crate::types::TraitBound {
                    trait_id,
                    type_args: implementation.trait_arg_types.clone(),
                }],
            )]);
            let derive = |bounds: &crate::hir::HirGenericBounds, tc: &mut TypeContext| {
                crate::mono::object_schema::erased_bound_schemas(
                    &parameters,
                    bounds,
                    &Default::default(),
                    &program.program.traits,
                    &orders,
                    &Default::default(),
                    tc,
                    false,
                )
            };
            template.implementation == *id
                && template.parameters == parameters
                && derive(&implementation.bounds, tc).is_ok_and(|bounds| bounds == template.bounds)
                && derive(&result_bounds, tc).is_ok_and(|results| {
                    results
                        .into_iter()
                        .find(|d| d.trait_id == trait_id)
                        .as_ref()
                        == Some(&template.result)
                })
                && template.members.len() == template.result.members.len()
                && template
                    .members
                    .iter()
                    .zip(&template.result.members)
                    .all(|(binding, member)| {
                        if program
                            .program
                            .indexes
                            .effective_trait_methods
                            .get(&(*id, binding.member_id))
                            != Some(&binding.method_id)
                            || binding.member_id != member.member_id
                            || binding.entry.definition != binding.method_id
                            || !binding.entry.static_args.is_empty()
                        {
                            return false;
                        }
                        let Some(method) = implementation
                            .methods
                            .values()
                            .find(|method| method.id == binding.method_id)
                        else {
                            return false;
                        };
                        if method.self_receiver != member.receiver
                            || method.params.len() != member.params.len()
                        {
                            return false;
                        }
                        let Ok(nested) = member.entry_signature(&parameters) else {
                            return false;
                        };
                        let params = nested.params;
                        let mut bindings = parameters
                            .iter()
                            .enumerate()
                            .map(|(index, parameter)| (parameter.source, index as u32))
                            .collect();
                        params.iter().zip(&method.params).all(|(expected, actual)| {
                            crate::mono::bind_erased_parameters(
                                expected,
                                &actual.ty,
                                &mut bindings,
                                tc,
                            )
                            .is_ok()
                        }) && crate::mono::bind_erased_parameters(
                            &nested.ret,
                            &method.ret_type,
                            &mut bindings,
                            tc,
                        )
                        .is_ok()
                            && method
                                .generic_params
                                .iter()
                                .all(|p| bindings.contains_key(&p.id))
                    })
        });
        for (id, template) in &contract.erased.dictionary_templates {
            if let Some(authority) = contract.dictionary_impl_members.get_mut(id) {
                authority.template = Some(template.clone());
            }
        }
        for (key, entry) in &contract.erased.member_entries {
            let Some(implementation) = program.program.impls.get(&entry.implementation) else {
                continue;
            };
            let Some(method) = implementation
                .methods
                .values()
                .find(|method| method.id == entry.method_id)
            else {
                continue;
            };
            let Some(declaration) = program.program.traits.get(&entry.trait_id) else {
                continue;
            };
            let Some(canonical) = declaration
                .signatures
                .values()
                .find(|signature| signature.id == entry.member.member_id)
            else {
                continue;
            };
            if program
                .program
                .indexes
                .effective_trait_methods
                .get(&(entry.implementation, entry.member.member_id))
                != Some(&entry.method_id)
                || implementation.trait_id != Some(entry.trait_id)
                || method.self_receiver != entry.member.receiver
                || canonical.self_receiver != entry.member.receiver
                || canonical.is_unsafe
                || method.is_unsafe
                || !polymorphic_impl_member_bounds_are_captured(implementation, method)
            {
                continue;
            }
            let static_self = if entry.member.receiver.is_none() {
                let Ok(id) = crate::mono::object_schema::static_member_self(declaration, canonical)
                else {
                    continue;
                };
                Some(id)
            } else {
                None
            };
            let locals = canonical
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
                        && static_self != Some(p.id)
                })
                .map(|p| MirErasedParameter {
                    source: p.id,
                    kind: p.kind.clone(),
                })
                .collect::<Vec<_>>();
            if locals != entry.member.binder.parameters {
                continue;
            }
            let Ok(signature) = entry.member.entry_signature(&entry.captures) else {
                continue;
            };
            let mut substitution = std::collections::HashMap::new();
            if entry.captures.is_empty() {
                if key.static_args.len() != implementation.type_generics.len() {
                    continue;
                }
                if key
                    .static_args
                    .iter()
                    .any(|id| !tc.type_id_tree_is_valid(*id))
                {
                    continue;
                }
                for (p, id) in implementation.type_generics.iter().zip(&key.static_args) {
                    substitution.insert(p.id, tc.type_for(*id));
                }
                let crate::hir::HirImplReceiverPattern::Exact(subject) =
                    &implementation.receiver_pattern
                else {
                    continue;
                };
                let Ok(subject) =
                    crate::type_services::substitution::instantiate(subject, &substitution)
                else {
                    continue;
                };
                if !crate::type_services::facts::TypeFacts::is_codegen_concrete(&subject) {
                    continue;
                }
                let subject_id = tc.intern_type(&subject);
                let synthetic = crate::types::GenericParamId {
                    owner: implementation.id,
                    index: 0,
                };
                let parent = vec![MirErasedParameter {
                    source: synthetic,
                    kind: crate::type_services::kind::Kind::Type,
                }];
                let bounds = crate::hir::HirGenericBounds::from_iter([(
                    synthetic,
                    vec![crate::types::TraitBound {
                        trait_id: entry.trait_id,
                        type_args: implementation.trait_arg_types.clone(),
                    }],
                )]);
                let Ok(schemas) = crate::mono::object_schema::erased_bound_schemas(
                    &parent,
                    &bounds,
                    &substitution,
                    &program.program.traits,
                    &orders,
                    &Default::default(),
                    tc,
                    false,
                ) else {
                    continue;
                };
                let Some(schema) = schemas
                    .into_iter()
                    .find(|schema| schema.trait_id == entry.trait_id)
                else {
                    continue;
                };
                let Ok(schema) = schema.substitute(&[MirErasedType::Concrete(subject_id)], tc)
                else {
                    continue;
                };
                if !schema.members.iter().any(|member| member == &entry.member) {
                    continue;
                }
            } else {
                let Some(template) = contract
                    .dictionary_impl_members
                    .get(&entry.implementation)
                    .and_then(|authority| authority.template.as_ref())
                else {
                    continue;
                };
                if entry.captures != template.parameters
                    || !template
                        .result
                        .members
                        .iter()
                        .any(|member| member == &entry.member)
                    || !key.static_args.is_empty()
                {
                    continue;
                }
            }
            let mut bindings = entry
                .captures
                .iter()
                .enumerate()
                .map(|(index, p)| (p.source, index as u32))
                .collect();
            let actual_params = method
                .params
                .iter()
                .map(|p| crate::type_services::substitution::instantiate(&p.ty, &substitution))
                .collect::<Result<Vec<_>, _>>();
            let Ok(actual_params) = actual_params else {
                continue;
            };
            let Ok(actual_ret) =
                crate::type_services::substitution::instantiate(&method.ret_type, &substitution)
            else {
                continue;
            };
            if signature.params.len() != actual_params.len()
                || !signature
                    .params
                    .iter()
                    .zip(&actual_params)
                    .all(|(expected, actual)| {
                        crate::mono::bind_erased_parameters(expected, actual, &mut bindings, tc)
                            .is_ok()
                    })
                || crate::mono::bind_erased_parameters(
                    &signature.ret,
                    &actual_ret,
                    &mut bindings,
                    tc,
                )
                .is_err()
            {
                continue;
            }
            let method_locals = method
                .generic_params
                .iter()
                .filter(|p| {
                    !implementation
                        .type_generics
                        .iter()
                        .any(|parent| parent.id == p.id)
                })
                .collect::<Vec<_>>();
            if method_locals.len() != locals.len()
                || method_locals.iter().zip(&locals).enumerate().any(
                    |(index, (actual, canonical))| {
                        actual.kind != canonical.kind
                            || bindings.get(&actual.id)
                                != Some(&((entry.captures.len() + index) as u32))
                    },
                )
            {
                continue;
            }
            if contract
                .erased
                .functions
                .get(key)
                .is_some_and(|function| function.body.is_none())
                && contract.erased.imported_member_links.get(key).is_none_or(
                    |(symbol, imported)| {
                        imported != entry || contract.erased.functions[key].symbol != *symbol
                    },
                )
            {
                continue;
            }
            contract
                .dictionary_member_authorities
                .insert(key.clone(), entry.clone());
        }
        for definition in program.program.structs.values() {
            contract.erased.constructor_kinds.insert(
                definition.id,
                crate::type_lowering::constructor_kind(&definition.generic_params),
            );
        }
        for definition in program.program.enums.values() {
            contract.erased.constructor_kinds.insert(
                definition.id,
                crate::type_lowering::constructor_kind(&definition.generic_params),
            );
        }
        let dictionaries = contract
            .erased
            .dictionaries
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for dictionary in &dictionaries {
            for member in &dictionary.members {
                let Some(target) = member.target.native() else {
                    continue;
                };
                let MirCallableKey::ObjectAdapter(MirObjectAdapterKey::NativeCallable {
                    concrete,
                    kind,
                }) = target
                else {
                    continue;
                };
                if contract.callables.contains_key(target) {
                    continue;
                }
                let source = tc.type_for(*concrete);
                let Type::Function { params, ret, .. } = &source else {
                    panic!("validated native dictionary source")
                };
                let packed = match params.as_slice() {
                    [] => Type::Unit,
                    [param] => param.clone(),
                    _ => Type::Tuple(params.clone()),
                };
                let receiver = if *kind == CallableKind::FnOnce {
                    Type::Pointer(Box::new(source.clone()))
                } else {
                    Type::Reference {
                        inner: Box::new(source.clone()),
                        mutable: *kind == CallableKind::FnMut,
                    }
                };
                let mut signature = MirCallableSignature::from_type_ids(
                    &[
                        intern_backend_type(tc, &receiver),
                        intern_backend_type(tc, &packed),
                    ],
                    intern_backend_type(tc, ret),
                    MirPassMode::Direct,
                );
                signature.params[0].pass_mode = MirPassMode::Pointer;
                let release_environment = *kind == CallableKind::FnOnce
                    && callable_environment_is_owned(&source)
                    && !matches!(ret.as_ref(), Type::Never);
                let symbol = format!("rock.erased.native.{}", contract.callables.len());
                contract.callables.insert(
                    target.clone(),
                    MirCallableDecl {
                        key: target.clone(),
                        source_def_id: None,
                        kind: MirCallableKind::ObjectAdapter(MirObjectAdapter::NativeCallable {
                            concrete: *concrete,
                            kind: *kind,
                            trait_id: dictionary.trait_id,
                            member_id: member.member_id,
                            release_environment,
                        }),
                        llvm_symbol: symbol,
                        linkage: MirLinkage::Internal,
                        signature,
                    },
                );
            }
        }
        let mut needed = BTreeSet::new();
        for requirement in contract.erased.dictionary_factories.values().flatten() {
            for argument in &requirement.arguments {
                let _ = argument.map_types(&mut |id| {
                    needed.insert(*id);
                    Ok::<_, std::convert::Infallible>(*id)
                });
            }
        }
        for schema in contract
            .erased
            .dictionary_templates
            .values()
            .flat_map(|template| std::iter::once(&template.result).chain(&template.bounds))
            .chain(
                contract
                    .erased
                    .dictionary_witnesses
                    .values()
                    .map(|witness| &witness.schema),
            )
        {
            for ty in
                std::iter::once(&schema.subject)
                    .chain(&schema.trait_args)
                    .chain(schema.members.iter().flat_map(|member| {
                        member.params.iter().chain(std::iter::once(&member.ret))
                    }))
            {
                let _ = ty.map_types(&mut |id| {
                    needed.insert(*id);
                    Ok::<_, std::convert::Infallible>(*id)
                });
            }
        }
        contract.erased.tuple_factories.clear();
        contract.erased.move_paths.clear();
        for (key, function) in &contract.erased.functions {
            if let Ok(paths) = crate::mir::erased_moves::collect_erased_move_paths(function, contract, tc) {
                if paths != crate::mir::erased_moves::MirErasedMovePaths::roots(function) {
                    contract.erased.move_paths.insert(key.clone(), paths);
                }
            }
        }
        contract.erased.nominal_factories.clear();
        let keys = contract
            .erased
            .functions
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            // Selected declaration templates are available only at the backend
            // contract boundary. Invalid/unavailable evidence stays absent and
            // is rejected by strict MIR rather than fabricated by codegen.
            if let Ok(requirements) =
                collect_body_factories(&contract.erased.functions[&key], contract, tc)
            {
                let mut tuples = Vec::new();
                let mut nominals = Vec::new();
                let mut handles = Vec::new();
                for requirement in requirements {
                    match requirement {
                        MirBodyFactoryRequirement::Tuple(r) => tuples.push(r),
                        MirBodyFactoryRequirement::Nominal(r) => nominals.push(r),
                        MirBodyFactoryRequirement::ThinHandle(r) => handles.push(r),
                    }
                }
                if !tuples.is_empty() {
                    contract.erased.tuple_factories.insert(key.clone(), tuples);
                }
                if !nominals.is_empty() {
                    contract.erased.nominal_factories.insert(key.clone(), nominals);
                }
                if !handles.is_empty() {
                    contract.erased.thin_handle_factories.insert(key, handles);
                }
            }
        }
        for (key, function) in &contract.erased.functions {
            if let Ok(requirements) = collect_scoped_factories(function, contract, tc) {
                if !requirements.is_empty() {
                    contract.erased.scoped_factories.insert(key.clone(), requirements);
                }
            }
        }
        for requirement in contract.erased.scoped_factories.values().flat_map(|scopes| scopes.values()).flatten() {
            for ty in std::iter::once(requirement.result()).chain(requirement.dependencies()) {
                let _ = ty.map_types(&mut |id| {
                    needed.insert(*id);
                    Ok::<_, std::convert::Infallible>(*id)
                });
            }
        }
        for requirement in contract.erased.nominal_factories.values().flatten() {
            for ty in requirement
                .arguments
                .iter()
                .chain(requirement.fields.iter().map(|(_, ty)| ty))
            {
                let _ = ty.map_types(&mut |id| {
                    needed.insert(*id);
                    Ok::<_, std::convert::Infallible>(*id)
                });
            }
        }
        for function in contract.erased.functions.values() {
            let mut collect = |ty: &MirErasedType| {
                let _ = ty.map_types(&mut |id| {
                    needed.insert(*id);
                    Ok::<_, std::convert::Infallible>(*id)
                });
                if let Ok(concrete) = ty.instantiate(&[], tc) {
                    needed.insert(intern_backend_type(tc, &concrete));
                }
            };
            for ty in function
                .signature
                .params
                .iter()
                .chain(std::iter::once(&function.signature.ret))
                .chain(&function.signature.layouts)
            {
                collect(ty);
            }
            for dictionary in &function.signature.dictionaries {
                for member in &dictionary.members {
                    for ty in member.params.iter().chain(std::iter::once(&member.ret)) {
                        collect(ty);
                    }
                }
            }
            if let Some(body) = &function.body {
                for ty in &body.locals {
                    collect(ty);
                }
                for statement in body.blocks.iter().flat_map(|block| &block.statements) {
                    if let Ok(types) = crate::mir::owner_open::transport_types(&statement.value, contract) {
                        for ty in &types { collect(ty); }
                    }
                    match &statement.value {
                        MirErasedRvalue::OwnerOpen { .. } | MirErasedRvalue::OwnerCall { .. } => {}
                        MirErasedRvalue::DropInPlace { pointee, .. } => collect(pointee),
                        MirErasedRvalue::PackagePack { plan, .. } => {
                            if let MirPackagePayloadDescriptor::Concrete(id) =
                                plan.payload_descriptor
                            {
                                collect(&MirErasedType::Concrete(id));
                            }
                        }
                        MirErasedRvalue::Call { type_args, .. }
                        | MirErasedRvalue::OpenedErasedCall { type_args, .. }
                        | MirErasedRvalue::DictionaryCall { type_args, .. } => {
                            for ty in type_args {
                                collect(ty);
                            }
                        }
                        MirErasedRvalue::Forget(_)
                        | MirErasedRvalue::Use(_)
                        | MirErasedRvalue::Literal(_)
                        | MirErasedRvalue::Aggregate(_)
                        | MirErasedRvalue::NativeAggregate { .. }
                        | MirErasedRvalue::NominalAggregate { .. }
                        | MirErasedRvalue::Borrow { .. }
                        | MirErasedRvalue::Open { .. }
                        | MirErasedRvalue::PackageOpen { .. }
                        | MirErasedRvalue::Reclose { .. }
                        | MirErasedRvalue::OpenedCall { .. }
                        | MirErasedRvalue::NativeCall { .. }
                        | MirErasedRvalue::NativeApply { .. } => {}
                    }
                }
            }
        }
        // Every producer table supports later scoped opening, independently of
        // whether its own clients select erased methods.
        for table in contract.vtables.values() {
            needed.insert(table.concrete);
        }
        for dictionary in &dictionaries {
            if tc.kind(dictionary.subject) == &crate::type_services::kind::Kind::Type {
                needed.insert(dictionary.subject);
            }
        }
        for function in functions.values() {
            for block in &function.basic_blocks {
                for statement in &block.statements {
                    if let StatementKind::Assign(_, Rvalue::Package(_, plan)) = &statement.kind {
                        if let MirPackagePayloadDescriptor::Concrete(id) = plan.payload_descriptor {
                            needed.insert(id);
                        }
                    }
                }
                let Some(Terminator::Call {
                    func: Operand::Constant(selector),
                    ..
                }) = &block.terminator
                else {
                    continue;
                };
                let owned_evidence;
                let call = match selector {
                    Constant::ErasedCall(call) => call,
                    Constant::OwnedObjectCall(key) => {
                        let Some(evidence) = key.erased_evidence(&contract.owned_object_calls[key])
                        else {
                            continue;
                        };
                        owned_evidence = evidence;
                        &owned_evidence
                    }
                    _ => continue,
                };
                let (signature, bindings) = match &call.target {
                    MirErasedCallTarget::Direct(key) => (
                        &contract.erased.functions[key].signature,
                        call.type_args
                            .iter()
                            .map(|id| Some(tc.type_for(*id)))
                            .collect::<Vec<_>>(),
                    ),
                    MirErasedCallTarget::Virtual(target) => (
                        &contract.object_schemas[&target.object].erased_slots[target.slot as usize]
                            .signature,
                        std::iter::once(None)
                            .chain(call.type_args.iter().map(|id| Some(tc.type_for(*id))))
                            .collect::<Vec<_>>(),
                    ),
                };
                for layout in &signature.layouts {
                    if matches!(call.target, MirErasedCallTarget::Virtual(_))
                        && *layout == MirErasedType::Parameter(0)
                    {
                        continue;
                    }
                    let ty = layout
                        .instantiate_optional(&bindings, tc)
                        .expect("mono finalized caller descriptor evidence");
                    needed.insert(intern_backend_type(tc, &ty));
                }
            }
        }
        let mut completed = BTreeSet::new();
        while let Some(id) = needed.pop_first() {
            if !completed.insert(id) {
                continue;
            }
            if tc.kind(id) != &crate::type_services::kind::Kind::Type {
                continue;
            }
            if let Type::Struct { id: owner, args } = tc.type_for(id) {
                // Closed owners keep their native destructor and opaque metadata;
                // an unsized Object argument is not a payload descriptor. Only
                // exact sealable nominal identities need a frozen argument array.
                if matches!(contract.nominal_drop_authority.get(&owner), Some(MirNominalDropAuthority::Selected(_)))
                    && descriptor_identity::semantic_identity(contract, tc, id).is_some()
                {
                    for argument in args { needed.insert(intern_backend_type(tc, &argument)); }
                }
            }
            let drop = payload_drop_plan(contract, tc, id)
                .expect("accepted erased values have complete concrete layout/drop evidence");
            let fields: Vec<_> = match &drop.children {
                MirPayloadDropChildren::Fields(fields) => {
                    fields.iter().map(|field| field.ty).collect()
                }
                _ => Vec::new(),
            };
            needed.extend(fields.iter().copied().filter(|id| !completed.contains(id)));
            contract.erased.descriptors.insert(
                id,
                MirTypeDescriptor {
                    ty: id,
                    drop,
                    fields,
                },
            );
        }
    }
}
