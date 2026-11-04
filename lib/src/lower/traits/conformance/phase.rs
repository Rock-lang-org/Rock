//! Trait conformance phase entry points.

use super::service::{
    collect_effective_trait_methods, IndexProtocolIds, TraitConformanceContext,
    TraitConformanceOutput, TraitConformancePhase, TraitConformanceService,
};
use crate::hir::*;
use crate::ids::DefId;
use crate::lower::Lowerer;
use crate::types::{GenericParamId, Type};
use std::collections::HashMap;

impl TraitConformancePhase {
    pub(crate) fn prepare_explicit_method_selection(lowerer: &mut Lowerer) {
        // Default bodies can call concrete trait implementations before defaults
        // are injected. Publish their exact member bindings, not name fallbacks.
        let mut output = TraitConformanceOutput::default();
        collect_effective_trait_methods(&lowerer.items, lowerer.root_crate_id, &mut output);
        lowerer
            .imported_effective_trait_methods
            .extend(output.effective_trait_methods);
        lowerer.diagnostics.extend(output.diagnostics);
    }

    fn annotate_callable_impl_patterns(lowerer: &mut Lowerer) {
        let callable_traits = [
            lowerer
                .language_items
                .fn_once
                .as_ref()
                .map(|items| (items.trait_id, crate::types::CallableKind::FnOnce)),
            lowerer
                .language_items
                .fn_mut
                .as_ref()
                .map(|items| (items.trait_id, crate::types::CallableKind::FnMut)),
            lowerer
                .language_items
                .fn_trait
                .as_ref()
                .map(|items| (items.trait_id, crate::types::CallableKind::Fn)),
        ]
        .into_iter()
        .flatten()
        .collect::<HashMap<_, _>>();
        if callable_traits.is_empty() {
            return;
        }

        let impl_ids = lowerer
            .items
            .impl_defs_in_order()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        for impl_id in impl_ids {
            let Some(imp) = lowerer.items.impl_def_mut(impl_id) else {
                continue;
            };
            let Some(callable_kind) = imp
                .trait_id
                .and_then(|trait_id| callable_traits.get(&trait_id).copied())
            else {
                continue;
            };
            if let HirImplReceiverPattern::Exact(Type::Function {
                callable_kind: pattern_kind,
                ..
            }) = &mut imp.receiver_pattern
            {
                *pattern_kind = callable_kind;
            }
        }
    }

    /// Auto-implement the marked Sized trait for all structs and enums.
    pub(super) fn auto_impl_sized(lowerer: &mut Lowerer) {
        let Some(sized_trait_id) = lowerer
            .language_items
            .sized
            .as_ref()
            .map(|items| items.trait_id)
        else {
            return;
        };
        let Some(sized_trait_name) = lowerer
            .items
            .trait_def(sized_trait_id)
            .map(|trait_def| trait_def.name.clone())
        else {
            return;
        };

        let mut owner_ids: Vec<DefId> = lowerer
            .items
            .structures()
            .map(|(id, _)| id)
            .chain(lowerer.items.enumerations().map(|(id, _)| id))
            .collect();
        owner_ids.sort();
        for owner_id in owner_ids {
            let Some(name) = lowerer
                .canonical_name_for_def_id(owner_id)
                .map(str::to_string)
            else {
                continue;
            };
            // Check if already has a Sized impl
            let already_has = lowerer
                .items
                .impl_defs_in_order()
                .any(|(_, imp)| imp.type_name == name && imp.trait_id == Some(sized_trait_id));
            if !already_has {
                if !lowerer.current_def_ids.contains(&owner_id) {
                    continue;
                }

                if let Some(owner_path) = lowerer.try_canonical_owner_path(&name) {
                    let id = DefId::new(lowerer.root_crate_id, lowerer.local_def_ids.fresh());
                    lowerer.current_def_ids.insert(id);
                    let (owner_generics, is_struct) =
                        if let Some(structure) = lowerer.items.structure(owner_id) {
                            (structure.generic_params.clone(), true)
                        } else {
                            (
                                lowerer
                                    .items
                                    .enumeration(owner_id)
                                    .expect("Sized owner ID must resolve to a struct or enum")
                                    .generic_params
                                    .clone(),
                                false,
                            )
                        };
                    let type_generics = owner_generics
                        .into_iter()
                        .enumerate()
                        .map(|(index, generic)| {
                            crate::types::GenericParamDecl::new(
                                GenericParamId {
                                    owner: id,
                                    index: index as u32,
                                },
                                generic.name,
                                generic.kind,
                            )
                        })
                        .collect::<Vec<_>>();
                    let type_args: Vec<_> = type_generics
                        .iter()
                        .enumerate()
                        .map(|(index, _)| {
                            Type::Generic(GenericParamId {
                                owner: id,
                                index: index as u32,
                            })
                        })
                        .collect();
                    let receiver_pattern = if is_struct {
                        HirImplReceiverPattern::Exact(Type::Struct {
                            id: owner_id,
                            args: type_args,
                        })
                    } else {
                        HirImplReceiverPattern::Exact(Type::Enum {
                            id: owner_id,
                            args: type_args,
                        })
                    };

                    lowerer
                        .items
                        .insert_impl(HirImpl {
                            id,
                            owner: HirImplOwner::Named(owner_path),
                            type_name: name.to_string(),
                            type_generics,
                            receiver_pattern,
                            trait_name: Some(sized_trait_name.clone()),
                            trait_id: Some(sized_trait_id),
                            trait_generics: vec![],
                            trait_arg_types: vec![],
                            associated_types: vec![],
                            bounds: crate::hir::HirGenericBounds::new(),
                            methods: HashMap::new(),
                        })
                        .expect("fresh automatic Sized impl ID must not collide");
                }
            }
        }
    }

    /// Check that trait impls provide all required methods, and inject defaults
    pub(crate) fn run(lowerer: &mut Lowerer) {
        Self::annotate_callable_impl_patterns(lowerer);
        Self::auto_impl_sized(lowerer);
        let index_protocol_ids = lowerer
            .language_items
            .index
            .as_ref()
            .zip(lowerer.language_items.index_mut.as_ref())
            .map(|(index, index_mut)| IndexProtocolIds {
                index_trait: index.trait_id,
                index_output: index.output_id,
                index_mut_trait: index_mut.trait_id,
                index_mut_output: index_mut.output_id,
            });
        let output = {
            let mut service = TraitConformanceService::new(TraitConformanceContext {
                items: &mut lowerer.items,
                engine: &mut lowerer.engine,
                resolver: &lowerer.resolver,
                dependency_resolvers: &lowerer.dependency_resolvers,
                root_crate_id: lowerer.root_crate_id,
                local_def_ids: &mut lowerer.local_def_ids,
                current_def_ids: &mut lowerer.current_def_ids,
                source_map: &lowerer.source_map,
                index_protocol_ids,
            });
            service.check_trait_conformance();
            service.finish()
        };
        lowerer
            .imported_effective_trait_methods
            .extend(output.effective_trait_methods);
        lowerer.diagnostics.extend(output.diagnostics);
    }
}

#[cfg(test)]
mod tests;
