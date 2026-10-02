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
                let MirCallableKey::ObjectAdapter(MirObjectAdapterKey::NativeCallable {
                    concrete,
                    kind,
                }) = &member.target
                else {
                    continue;
                };
                if contract.callables.contains_key(&member.target) {
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
                    &[tc.intern_type(&receiver), tc.intern_type(&packed)],
                    tc.intern_type(ret),
                    MirPassMode::Direct,
                );
                signature.params[0].pass_mode = MirPassMode::Pointer;
                let release_environment = *kind == CallableKind::FnOnce
                    && callable_environment_is_unique(&source)
                    && !matches!(ret.as_ref(), Type::Never);
                let symbol = format!("rock.erased.native.{}", contract.callables.len());
                contract.callables.insert(
                    member.target.clone(),
                    MirCallableDecl {
                        key: member.target.clone(),
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
        for function in contract.erased.functions.values() {
            let mut collect = |ty: &MirErasedType| {
                let _ = ty.map_types(&mut |id| {
                    needed.insert(*id);
                    Ok::<_, std::convert::Infallible>(*id)
                });
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
            }
        }
        for table in contract
            .vtables
            .values()
            .filter(|table| !table.erased_methods.is_empty())
        {
            needed.insert(table.concrete);
        }
        for dictionary in &dictionaries {
            if tc.kind(dictionary.subject) == &crate::type_services::kind::Kind::Type {
                needed.insert(dictionary.subject);
            }
        }
        for function in functions.values() {
            for block in &function.basic_blocks {
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
                    needed.insert(tc.intern_type(&ty));
                }
            }
        }
        for id in needed {
            if tc.kind(id) != &crate::type_services::kind::Kind::Type {
                continue;
            }
            let drop = payload_drop_plan(contract, tc, id)
                .expect("accepted erased values have complete concrete layout/drop evidence");
            let fields = match &drop.children {
                MirPayloadDropChildren::Fields(fields) => {
                    fields.iter().map(|field| field.ty).collect()
                }
                _ => Vec::new(),
            };
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
