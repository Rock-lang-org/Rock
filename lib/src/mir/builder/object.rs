use super::MirBuilder;
use crate::mir::*;
use crate::type_context::TypeContext;
use crate::types::{CallableKind, Type};

impl MirBuilder<'_> {
    pub(super) fn populate_object_adapters(
        contract: &mut MirBackendContract,
        tc: &mut TypeContext,
    ) {
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
                let concrete = tc.type_for(table.concrete);
                let (adapter, signature) = match adapter_key {
                    MirObjectAdapterKey::PayloadDrop(ty) => {
                        let plan = payload_drop_plan(contract, tc, *ty)
                            .expect("accepted sized payload has a complete drop layout");
                        let receiver = tc.intern_type(&Type::Reference {
                            inner: Box::new(concrete),
                            mutable: true,
                        });
                        let unit = tc.intern_type(&Type::Unit);
                        (
                            MirObjectAdapter::PayloadDrop(plan),
                            MirCallableSignature::from_type_ids(
                                &[receiver],
                                unit,
                                MirPassMode::Pointer,
                            ),
                        )
                    }
                    MirObjectAdapterKey::NativeCallable { concrete: ty, kind } => {
                        let release_environment = *kind == CallableKind::FnOnce
                            && callable_environment_is_unique(&concrete)
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
                            semantic_ty: tc.intern_type(&receiver),
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
                            semantic_ty: tc.intern_type(&Type::Pointer(Box::new(concrete))),
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
