//! Source-item reachability after inference, before monomorphization.
//!
//! References are followed with a worklist, including generic dispatch and
//! implicit destruction. Dependencies are never removed by this pass.

mod edges;
mod indexes;
mod roots;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use crate::hir::{HirMethodLocation, HirVariantFields};
use crate::ids::{DefId, VariantId};
use crate::infer::ResolvedHirProgram;
use crate::lint::{is_intentionally_unused, Lint, LintSink, SourceModules};
use crate::{Config, CrateKind};

use edges::Edges;

/// Analysis does not mutate HIR: callers such as the language server retain
/// the complete typed program. Compilation applies the result separately.
pub(crate) struct Reachability {
    live: HashSet<DefId>,
    constructed: HashSet<(DefId, VariantId)>,
    library: bool,
}

impl Reachability {
    pub(crate) fn compute(
        hir: &ResolvedHirProgram,
        modules: &SourceModules<'_>,
        config: &Config,
        additional_roots: impl IntoIterator<Item = DefId>,
    ) -> Self {
        let program = &hir.program;
        let mut pending = roots::roots(hir, modules, config);
        if config.crate_kind == CrateKind::Lib {
            pending.extend(additional_roots);
        }
        let mut live = HashSet::new();
        let mut constructed = HashSet::new();
        let mut trait_impls: HashMap<DefId, Vec<DefId>> = HashMap::new();
        let mut nominal_impls: HashMap<DefId, Vec<DefId>> = HashMap::new();
        let mut source_edges: HashMap<DefId, Vec<DefId>> = HashMap::new();
        for reference in hir.source_map.references() {
            if let (Some(owner), crate::source_map::SourceSymbol::Definition(target)) =
                (reference.lexical_owner, &reference.target)
            {
                if program.type_aliases.contains_key(target)
                    || program.structs.contains_key(target) || program.enums.contains_key(target)
                    || program.traits.contains_key(target) {
                    // A separate signature has a source item id of its own;
                    // the resolver's canonical path identifies its body item.
                    let owner = hir.resolver.canonical_name(owner)
                        .and_then(|name| hir.resolver.item_paths.get(name))
                        .copied().unwrap_or(owner);
                    source_edges.entry(owner).or_default().push(*target);
                }
            }
        }
        for imp in program.impls.values() {
            if let Some(trait_id) = imp.trait_id {
                trait_impls.entry(trait_id).or_default().push(imp.id);
            }
        }
        // Library impls are downstream API even without a direct source call.
        // Drop impls can also be selected implicitly from interned types.
        let drop_trait = program.language_items.drop.as_ref().map(|items| items.trait_id);
        for imp in program.impls.values() {
            if imp.trait_id == drop_trait && drop_trait.is_some() {
                pending.push(imp.id);
            } else if config.crate_kind == CrateKind::Lib {
                let receiver = match &imp.receiver_pattern {
                    crate::hir::HirImplReceiverPattern::Exact(ty)
                    | crate::hir::HirImplReceiverPattern::Constructor(ty) => nominal_receiver(ty),
                    crate::hir::HirImplReceiverPattern::SliceFamily { .. } => None,
                };
                match receiver {
                    Some(owner) if owner.crate_id == imp.id.crate_id => {
                        nominal_impls.entry(owner).or_default().push(imp.id);
                    }
                    // Primitive/slice, blanket and foreign-receiver implementations
                    // have no local nominal visibility gate and can be selected
                    // downstream.
                    _ => pending.push(imp.id),
                }
            }
        }
        while let Some(id) = pending.pop() {
            if !live.insert(id) {
                continue;
            }
            let mut edges = Edges::default();
            edges.ids.extend(nominal_impls.get(&id).into_iter().flatten().copied());
            edges.ids.extend(source_edges.get(&id).into_iter().flatten().copied());
            if let Some(function) = program.functions.get(&id) {
                edges.function(function);
            } else if let Some(structure) = program.structs.get(&id) {
                edges.generics(&structure.generic_params);
                for field in &structure.fields {
                    edges.ty(&field.ty);
                }
            } else if let Some(enumeration) = program.enums.get(&id) {
                edges.generics(&enumeration.generic_params);
                for variant in &enumeration.variants {
                    match &variant.fields {
                        HirVariantFields::Named(fields) => {
                            fields.iter().for_each(|field| edges.ty(&field.ty));
                        }
                        HirVariantFields::Positional(types) => {
                            types.iter().for_each(|ty| edges.ty(ty));
                        }
                        HirVariantFields::Unit => {}
                    }
                }
            } else if let Some(alias) = program.type_aliases.get(&id) {
                edges.generics(&alias.generic_params);
                edges.ty(&alias.ty);
            } else if let Some(ext) = program.externs.get(&id) {
                ext.params.iter().for_each(|ty| edges.ty(ty));
                edges.ty(&ext.ret);
            } else if let Some(trait_def) = program.traits.get(&id) {
                edges.generics(&trait_def.generic_params);
                edges.predicates(&trait_def.predicates);
                for signature in trait_def.signatures.values() {
                    edges.signature(signature);
                }
                edges.ids.extend(trait_def.methods.values().map(|method| method.id));
                // Dependency generic bodies can be loaded only during mono.
                // A bound on a live trait must retain its candidate impls even
                // if no local HIR body exposes the eventual abstract call.
                edges.ids.extend(trait_impls.get(&id).into_iter().flatten().copied());
            } else if let Some(imp) = program.impls.get(&id) {
                edges.generics(&imp.type_generics);
                edges.generics(&imp.trait_generics);
                edges.receiver(&imp.receiver_pattern);
                edges.ids.extend(imp.trait_id);
                imp.trait_arg_types.iter().for_each(|ty| edges.ty(ty));
                imp.associated_types.iter().for_each(|associated| edges.ty(&associated.ty));
                edges.bounds(&imp.bounds);
                // Effective trait-member records require the whole impl to
                // remain coherent for products and downstream specialization.
                if imp.trait_id.is_some() || config.crate_kind == CrateKind::Lib {
                    edges.ids.extend(imp.methods.values().map(|method| method.id));
                }
            } else if let Some(location) = program.indexes.methods_by_id.get(&id) {
                match location {
                    HirMethodLocation::ImplMethod { impl_id, method_name, .. } => {
                        if let Some(imp) = program.impls.get(impl_id) {
                            edges.ids.push(*impl_id);
                            if let Some(method) = imp.methods.get(method_name) {
                                edges.function(method);
                            }
                        }
                    }
                    HirMethodLocation::TraitDefault { trait_id, method_name, .. } => {
                        edges.ids.push(*trait_id);
                        if let Some(method) = program.traits.get(trait_id).and_then(|tr| tr.methods.get(method_name)) {
                            edges.function(method);
                        }
                    }
                }
            }
            constructed.extend(edges.constructed);
            pending.extend(edges.ids.into_iter().filter(|id| !live.contains(id)));
        }
        Self { live, constructed, library: config.crate_kind == CrateKind::Lib }
    }

    pub(crate) fn warn(&self, hir: &ResolvedHirProgram, sink: &mut LintSink<'_>) {
        if !sink.is_enabled(Lint::DeadCode) {
            return;
        }
        let program = &hir.program;
        let items = program.functions.values().map(|item| (item.id, item.name.as_str(), "function"))
            .chain(program.structs.values().map(|item| (item.id, item.name.as_str(), "struct")))
            .chain(program.enums.values().map(|item| (item.id, item.name.as_str(), "enum")))
            .chain(program.traits.values().map(|item| (item.id, item.name.as_str(), "trait")))
            .chain(program.type_aliases.values().map(|item| (item.id, item.name.as_str(), "type alias")))
            .chain(program.externs.values().map(|item| (item.id, item.name.as_str(), "extern function")))
            .chain(program.impls.values().filter(|imp| imp.trait_id.is_none())
                .flat_map(|imp| imp.methods.values())
                .map(|method| (method.id, method.name.as_str(), "method")));
        for (id, name, kind) in items {
            if self.live.contains(&id) || !hir.current_def_ids.contains(&id)
                || is_intentionally_unused(name.rsplit("::").next().unwrap_or(name)) {
                continue;
            }
            if let Some(span) = hir.source_map.definition_span(id) {
                sink.emit(Lint::DeadCode, crate::diagnostic::Diagnostic::new(
                    format!("{kind} `{name}` is never used"), span.clone(),
                ));
            }
        }
        if !self.library {
            let lang = &program.language_items;
            for enumeration in program.enums.values() {
                if !hir.current_def_ids.contains(&enumeration.id) || !self.live.contains(&enumeration.id)
                    || lang.range.as_ref().is_some_and(|items| items.enum_id == enumeration.id)
                    || lang.try_protocol.as_ref().is_some_and(|items| items.control_flow_enum_id == enumeration.id) {
                    continue;
                }
                for variant in &enumeration.variants {
                    if self.constructed.contains(&(enumeration.id, variant.id))
                        || is_intentionally_unused(&variant.name) {
                        continue;
                    }
                    if let Some(info) = hir.source_map.symbol(&crate::source_map::SourceSymbol::Variant {
                        owner: enumeration.id, variant: variant.id,
                    }) {
                        sink.emit(Lint::DeadCode, crate::diagnostic::Diagnostic::new(
                            format!("variant `{}::{}` is never constructed", enumeration.name, variant.name),
                            info.name_span.clone(),
                        ));
                    }
                }
            }
        }
    }

    pub(crate) fn prune(&self, hir: &mut ResolvedHirProgram) {
        let keep = |id: &DefId| !hir.current_def_ids.contains(id) || self.live.contains(id);
        let program = &hir.program;
        let has_dead_items = program.functions.keys()
            .chain(program.structs.keys()).chain(program.enums.keys())
            .chain(program.traits.keys()).chain(program.impls.keys())
            .chain(program.externs.keys()).chain(program.type_aliases.keys())
            .any(|id| !keep(id))
            || program.impls.values().filter(|imp| imp.trait_id.is_none())
                .flat_map(|imp| imp.methods.values()).any(|method| !keep(&method.id));
        if !has_dead_items {
            return;
        }
        let program = hir.program.program_mut();
        program.functions.retain(|id, _| keep(id));
        program.structs.retain(|id, _| keep(id));
        program.enums.retain(|id, _| keep(id));
        program.traits.retain(|id, _| keep(id));
        program.impls.retain(|id, _| keep(id));
        for imp in program.impls.values_mut() {
            if imp.trait_id.is_none() {
                imp.methods.retain(|_, method| keep(&method.id));
            }
        }
        program.externs.retain(|id, _| keep(id));
        program.type_aliases.retain(|id, _| keep(id));
        program.order.impls.retain(&keep);
        program.order.externs.retain(&keep);
        program.names.functions_by_name.retain(|_, id| keep(id));
        program.names.structs_by_name.retain(|_, id| keep(id));
        program.names.enums_by_name.retain(|_, id| keep(id));
        program.names.traits_by_name.retain(|_, id| keep(id));
        program.names.externs_by_name.retain(|_, id| keep(id));
        program.names.type_aliases_by_name.retain(|_, id| keep(id));
        indexes::retain(&mut program.indexes, &keep);
        hir.constant_arithmetic.retain(|id, _| keep(id));
        hir.type_ids.retain_owners(&keep);
        // Keep current_def_ids as the provenance set: product construction uses
        // it to distinguish missing local definitions from external definitions.
    }
}

fn nominal_receiver(ty: &crate::types::Type) -> Option<DefId> {
    use crate::types::Type;
    match ty {
        Type::Struct { id, .. } | Type::Enum { id, .. } | Type::Constructor { id, .. } => Some(*id),
        Type::Apply { constructor, .. } => nominal_receiver(constructor),
        Type::Reference { inner, .. } | Type::Pointer(inner) => nominal_receiver(inner),
        _ => None,
    }
}
