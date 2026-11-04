use std::collections::HashMap;

use crate::hir::{
    HirBlockFor, HirEnum, HirExprFor, HirExprKindFor as HirExprKind, HirExtern, HirFunctionFor,
    HirImplFor, HirPattern, HirPhase, HirProgramFor, HirStmtFor, HirStruct, HirTraitFor,
    HirVariantFields,
};
use crate::ids::{AssocTypeId, DefId, FieldId, TypeId, VariantId};
use crate::type_context::TypeContext;
use crate::types::{GenericParamId, TraitBound, Type};

mod body_types;
mod definition_types;
mod location;

use body_types::*;
use definition_types::*;
pub use location::HirTypeLocation;

#[derive(Debug, Clone, Default)]
pub struct HirTypeIds {
    ids: HashMap<HirTypeLocation, TypeId>,
}

impl HirTypeIds {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, location: HirTypeLocation, id: TypeId) {
        if let Some(existing) = self.ids.insert(location.clone(), id) {
            debug_assert_eq!(
                existing, id,
                "duplicate HIR type location with different TypeId: {:?}",
                location
            );
        }
    }

    pub fn get(&self, location: &HirTypeLocation) -> Option<TypeId> {
        self.ids.get(location).copied()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&HirTypeLocation, TypeId)> {
        self.ids.iter().map(|(location, id)| (location, *id))
    }
}

pub fn collect_hir_type_ids<P: HirPhase>(
    program: &HirProgramFor<P>,
    context: &mut TypeContext,
) -> HirTypeIds {
    let mut ids = HirTypeIds::new();

    let mut functions: Vec<_> = program.functions.values().collect();
    functions.sort_by_key(|function| function.id);
    for function in functions {
        collect_function_signature(function, context, &mut ids);
        collect_block(function.id, &function.body, Vec::new(), context, &mut ids);
    }

    let mut externs: Vec<_> = program.externs.values().collect();
    externs.sort_by_key(|ext| ext.id);
    for ext in externs {
        collect_extern_signature(ext, context, &mut ids);
    }

    let mut aliases: Vec<_> = program.type_aliases.values().collect();
    aliases.sort_by_key(|alias| alias.id);
    for alias in aliases {
        record_type(
            context,
            &mut ids,
            HirTypeLocation::TypeAliasBody { alias: alias.id },
            &alias.ty,
        );
    }

    let mut structs: Vec<_> = program.structs.values().collect();
    structs.sort_by_key(|strukt| strukt.id);
    for strukt in structs {
        collect_struct_fields(strukt, context, &mut ids);
    }

    let mut enums: Vec<_> = program.enums.values().collect();
    enums.sort_by_key(|enm| enm.id);
    for enm in enums {
        collect_enum_fields(enm, context, &mut ids);
    }

    let mut traits: Vec<_> = program.traits.values().collect();
    traits.sort_by_key(|trait_def| trait_def.id);
    for trait_def in traits {
        collect_trait_signatures(trait_def, context, &mut ids);
        let mut methods: Vec<_> = trait_def.methods.values().collect();
        methods.sort_by_key(|method| method.id);
        for method in methods {
            collect_function_signature(method, context, &mut ids);
            collect_block(method.id, &method.body, Vec::new(), context, &mut ids);
        }
    }

    let mut impls: Vec<_> = program.impls.values().collect();
    impls.sort_by_key(|imp| imp.id);
    for imp in impls {
        collect_impl_types(imp, context, &mut ids);
        let mut methods: Vec<_> = imp.methods.values().collect();
        methods.sort_by_key(|method| method.id);
        for method in methods {
            collect_function_signature(method, context, &mut ids);
            collect_block(method.id, &method.body, Vec::new(), context, &mut ids);
        }
    }

    ids
}

fn record_type(
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
    location: HirTypeLocation,
    ty: &Type,
) {
    let id = context.intern_canonical_type(ty);
    ids.insert(location, id);
}

#[cfg(test)]
mod tests;
