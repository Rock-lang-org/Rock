use super::MirBuilder;
use crate::mir::*;
use crate::type_context::TypeContext;
use crate::types::{CallableKind, Type};

impl MirBuilder<'_> {
    fn populate_payload_drop_adapter(
        contract: &mut MirBackendContract,
        tc: &mut TypeContext,
        ty: crate::ids::TypeId,
    ) {
        let key = MirCallableKey::ObjectAdapter(MirObjectAdapterKey::PayloadDrop(ty));
        if contract.callables.contains_key(&key) {
            return;
        }
        let plan = payload_drop_plan(contract, tc, ty)
            .expect("accepted sized payload has a complete drop layout");
        let concrete = tc.type_for(ty);
        let receiver = intern_backend_type(
            tc,
            &Type::Reference {
                inner: Box::new(concrete),
                mutable: true,
            },
        );
        let unit = tc.intern_type(&Type::Unit);
        let llvm_symbol = format!("rock.object.adapter.{}", contract.callables.len());
        contract.callables.insert(
            key.clone(),
            MirCallableDecl {
                key,
                source_def_id: None,
                kind: MirCallableKind::ObjectAdapter(MirObjectAdapter::PayloadDrop(plan)),
                llvm_symbol,
                linkage: MirLinkage::Internal,
                signature: MirCallableSignature::from_type_ids(
                    &[receiver],
                    unit,
                    MirPassMode::Pointer,
                ),
            },
        );
    }

    pub(super) fn populate_object_adapters(
        contract: &mut MirBackendContract,
        tc: &mut TypeContext,
        functions: &std::collections::BTreeMap<MirFunctionId, MirFunction>,
    ) {
        // The same opaque-environment cleanup is needed without an object view.
        // This is structural cleanup, never a user destructor in drop_glue.
        let callable_payloads = functions
            .values()
            .flat_map(|function| {
                function.basic_blocks.iter().filter_map(|block| {
                    let (place, consuming_call) = match &block.terminator {
                        Some(
                            Terminator::Drop { place, .. }
                            | Terminator::DropWithOrigin { place, .. },
                        ) => (place, false),
                        Some(Terminator::Call { func: Operand::Move(place) | Operand::Copy(place), .. }) => (place, true),
                        _ => return None,
                    };
                    let id = contract.place_type_id(tc, function, place)?;
                    if consuming_call {
                        let ty = tc.type_for(id);
                        if !matches!(&ty, Type::Function { callable_kind: CallableKind::FnOnce, ret, .. } if !matches!(ret.as_ref(), Type::Never))
                        || !callable_environment_is_owned(&ty) { return None; }
                    }
                    Some(id)
                })
            })
            .filter(|id| {
                let ty = tc.type_for(*id);
                crate::type_services::facts::TypeFacts::is_codegen_concrete(&ty)
                    && matches!(ty, Type::Function { captures, .. } if !captures.is_empty())
            })
            .collect::<std::collections::BTreeSet<_>>();
        for ty in callable_payloads {
            Self::populate_payload_drop_adapter(contract, tc, ty);
        }
        let tables = contract.vtables.values().cloned().collect::<Vec<_>>();
        for table in tables {
            let schema = contract.object_schemas[&table.object].clone();
            for (index, key) in table
                .methods
                .iter()
                .enumerate()
                .map(|(i, key)| (Some(i), key))
                .chain(table.drop.iter().map(|key| (None, key)))
            {
                let MirCallableKey::ObjectAdapter(adapter_key) = key else {
                    continue;
                };
                if contract.callables.contains_key(key) {
                    continue;
                }
                if let MirObjectAdapterKey::PayloadDrop(ty) = adapter_key {
                    Self::populate_payload_drop_adapter(contract, tc, *ty);
                    continue;
                }
                let concrete = tc.type_for(table.concrete);
                let (adapter, signature) = match adapter_key {
                    MirObjectAdapterKey::PayloadDrop(_) => {
                        unreachable!("payload adapter populated above")
                    }
                    MirObjectAdapterKey::NativeCallable { concrete: ty, kind } => {
                        let release_environment = *kind == CallableKind::FnOnce
                            && callable_environment_is_owned(&concrete)
                            && matches!(&concrete, Type::Function { ret, .. } if !matches!(ret.as_ref(), Type::Never));
                        let slot =
                            &schema.slots[index.expect("callable adapter is a method entry")];
                        let mut signature = slot.signature.clone();
                        let receiver = if *kind == CallableKind::FnOnce {
                            Type::Pointer(Box::new(concrete))
                        } else {
                            Type::Reference {
                                inner: Box::new(concrete),
                                mutable: *kind == CallableKind::FnMut,
                            }
                        };
                        signature.params[0] = MirParamAbi {
                            semantic_ty: intern_backend_type(tc, &receiver),
                            pass_mode: MirPassMode::Pointer,
                        };
                        (
                            MirObjectAdapter::NativeCallable {
                                concrete: *ty,
                                kind: *kind,
                                trait_id: slot.trait_id,
                                member_id: slot.member_id,
                                release_environment,
                            },
                            signature,
                        )
                    }
                    MirObjectAdapterKey::ConsumingMethod {
                        concrete: ty,
                        target,
                    } => {
                        let target = MirCallableKey::Instance(*target);
                        let mut signature = contract
                            .callable(&target)
                            .expect("consuming entry instance is materialized")
                            .signature
                            .clone();
                        signature.params[0] = MirParamAbi {
                            semantic_ty: intern_backend_type(
                                tc,
                                &Type::Pointer(Box::new(concrete)),
                            ),
                            pass_mode: MirPassMode::Pointer,
                        };
                        (
                            MirObjectAdapter::ConsumingMethod {
                                concrete: *ty,
                                target,
                            },
                            signature,
                        )
                    }
                };
                let llvm_symbol = format!("rock.object.adapter.{}", contract.callables.len());
                contract.callables.insert(
                    key.clone(),
                    MirCallableDecl {
                        key: key.clone(),
                        source_def_id: None,
                        kind: MirCallableKind::ObjectAdapter(adapter),
                        llvm_symbol,
                        linkage: MirLinkage::Internal,
                        signature,
                    },
                );
            }
        }
    }
}
