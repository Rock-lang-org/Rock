use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::hir::{
    AcceptedHirFunction, AcceptedHirImpl, AcceptedHirTrait, HirAssociatedTypeDef, HirBlock,
    HirEnum, HirExtern, HirField, HirFunction, HirImpl, HirParam, HirStruct, HirTrait,
    HirTypeAlias, HirVariant,
};
use crate::ids::DefId;
use crate::products::{
    ProductAssociatedTypeInterface, ProductEnumInterface, ProductEnumVariantInterface,
    ProductExternInterface, ProductFunctionInterface, ProductImplInterface, ProductStructInterface,
    ProductTraitInterface, ProductTypeAliasInterface,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactExport {
    pub source: String,
    pub id: DefId,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtifactCrateInterface {
    pub object_abi: crate::products::object_abi::ProductObjectAbi,
    pub root_export_ids: BTreeMap<String, ArtifactExport>,
    pub canonical_names: BTreeMap<DefId, String>,
    pub functions: BTreeMap<DefId, ProductFunctionInterface>,
    pub structs: BTreeMap<DefId, ProductStructInterface>,
    pub enums: BTreeMap<DefId, ProductEnumInterface>,
    pub type_aliases: BTreeMap<DefId, ProductTypeAliasInterface>,
    pub traits: BTreeMap<DefId, ProductTraitInterface>,
    pub impls: BTreeMap<DefId, ProductImplInterface>,
    pub externs: BTreeMap<DefId, ProductExternInterface>,
    pub effective_trait_methods: BTreeMap<(DefId, DefId), DefId>,
    pub infix_precedence: BTreeMap<String, u8>,
}

impl ArtifactCrateInterface {
    #[cfg(test)]
    pub(crate) fn insert_function<P: crate::hir::HirPhase>(
        &mut self,
        canonical_name: String,
        function: crate::hir::HirFunctionFor<P>,
    ) {
        self.canonical_names.insert(function.id, canonical_name);
        self.functions
            .insert(function.id, ProductFunctionInterface::from(&function));
    }

    #[cfg(test)]
    pub(crate) fn insert_struct(&mut self, canonical_name: String, strukt: HirStruct) {
        self.canonical_names.insert(strukt.id, canonical_name);
        self.structs
            .insert(strukt.id, ProductStructInterface::from(&strukt));
    }

    #[cfg(test)]
    pub(crate) fn insert_trait<P: crate::hir::HirPhase>(
        &mut self,
        canonical_name: String,
        trt: crate::hir::HirTraitFor<P>,
    ) {
        self.object_abi.trait_members.insert(
            trt.id,
            crate::products::object_abi::declaration_member_order(&trt.signatures),
        );
        self.canonical_names.insert(trt.id, canonical_name);
        self.traits
            .insert(trt.id, ProductTraitInterface::from(&trt));
    }

    #[cfg(test)]
    pub(crate) fn insert_impl<P: crate::hir::HirPhase>(&mut self, imp: crate::hir::HirImplFor<P>) {
        let receiver_pattern = P::impl_receiver_pattern(&imp.receiver_pattern)
            .cloned()
            .unwrap_or_else(|| {
                let mut parts = Vec::new();
                P::visit_impl_receiver_types(&imp.receiver_pattern, &mut |ty| {
                    parts.push(ty.clone())
                });
                parts.into()
            });
        self.impls.insert(
            imp.id,
            ProductImplInterface {
                id: imp.id,
                owner: imp.owner,
                type_name: imp.type_name,
                type_generics: imp.type_generics,
                receiver_pattern,
                trait_name: imp.trait_name,
                trait_id: imp.trait_id,
                trait_generics: imp.trait_generics,
                trait_arg_types: imp.trait_arg_types,
                associated_types: imp
                    .associated_types
                    .into_iter()
                    .map(|assoc| crate::products::ProductAssociatedTypeInterface {
                        id: assoc.id,
                        name: assoc.name,
                        kind: assoc.kind,
                        ty: assoc.ty,
                    })
                    .collect(),
                bounds: imp.bounds,
                methods: imp
                    .methods
                    .iter()
                    .map(|(name, method)| (name.clone(), ProductFunctionInterface::from(method)))
                    .collect(),
            },
        );
    }

    pub(crate) fn canonical_name(&self, id: DefId) -> Option<&str> {
        self.canonical_names.get(&id).map(String::as_str)
    }

    #[cfg(test)]
    pub(crate) fn function_by_canonical_name(
        &self,
        name: &str,
    ) -> Option<&ProductFunctionInterface> {
        self.id_for_canonical_name(name)
            .and_then(|id| self.functions.get(&id))
    }

    #[cfg(test)]
    pub(crate) fn struct_by_canonical_name(&self, name: &str) -> Option<&ProductStructInterface> {
        self.id_for_canonical_name(name)
            .and_then(|id| self.structs.get(&id))
    }

    #[cfg(test)]
    pub(crate) fn enum_by_canonical_name(&self, name: &str) -> Option<&ProductEnumInterface> {
        self.id_for_canonical_name(name)
            .and_then(|id| self.enums.get(&id))
    }

    #[cfg(test)]
    pub(crate) fn trait_by_canonical_name(&self, name: &str) -> Option<&ProductTraitInterface> {
        self.id_for_canonical_name(name)
            .and_then(|id| self.traits.get(&id))
    }

    #[cfg(test)]
    fn id_for_canonical_name(&self, name: &str) -> Option<DefId> {
        self.canonical_names
            .iter()
            .find_map(|(id, canonical)| (canonical == name).then_some(*id))
    }

    pub(crate) fn function_items(&self) -> Vec<(String, HirFunction)> {
        self.functions
            .values()
            .map(|function| {
                let name = self.definition_name(function.id, &function.name);
                (name, hir_function_from_interface(function))
            })
            .collect()
    }

    pub(crate) fn struct_items(&self) -> Vec<(String, HirStruct)> {
        self.structs
            .values()
            .map(|strukt| {
                let name = self.definition_name(strukt.id, &strukt.name);
                (name, hir_struct_from_interface(strukt))
            })
            .collect()
    }

    pub(crate) fn enum_items(&self) -> Vec<(String, HirEnum)> {
        self.enums
            .values()
            .map(|enm| {
                let name = self.definition_name(enm.id, &enm.name);
                (name, hir_enum_from_interface(enm))
            })
            .collect()
    }

    pub(crate) fn type_alias_items(&self) -> Vec<(String, HirTypeAlias)> {
        self.type_aliases
            .values()
            .map(|alias| {
                let name = self.definition_name(alias.id, &alias.name);
                (name, hir_type_alias_from_interface(alias))
            })
            .collect()
    }

    pub(crate) fn trait_items(&self) -> Vec<(String, HirTrait)> {
        self.traits
            .values()
            .map(|trt| {
                let name = self.definition_name(trt.id, &trt.name);
                (name, hir_trait_from_interface(trt))
            })
            .collect()
    }

    pub(crate) fn impl_items(&self) -> Vec<HirImpl> {
        self.impls.values().map(hir_impl_from_interface).collect()
    }

    pub(crate) fn extern_items(&self) -> Vec<HirExtern> {
        self.externs
            .values()
            .map(hir_extern_from_interface)
            .collect()
    }

    pub(crate) fn export_for_source(&self, source: &str) -> Option<ArtifactExport> {
        self.functions
            .values()
            .find(|function| self.definition_name(function.id, &function.name) == source)
            .map(|function| function.id)
            .or_else(|| {
                self.structs
                    .values()
                    .find(|value| self.definition_name(value.id, &value.name) == source)
                    .map(|value| value.id)
            })
            .or_else(|| {
                self.enums
                    .values()
                    .find(|value| self.definition_name(value.id, &value.name) == source)
                    .map(|value| value.id)
            })
            .or_else(|| {
                self.type_aliases
                    .values()
                    .find(|value| self.definition_name(value.id, &value.name) == source)
                    .map(|value| value.id)
            })
            .or_else(|| {
                self.traits
                    .values()
                    .find(|value| self.definition_name(value.id, &value.name) == source)
                    .map(|value| value.id)
            })
            .or_else(|| {
                self.externs
                    .values()
                    .find(|ext| self.definition_name(ext.id, &ext.name) == source)
                    .map(|ext| ext.id)
            })
            .map(|id| ArtifactExport {
                source: source.to_string(),
                id,
            })
    }

    pub(crate) fn prelude_source_names(&self, prelude_prefix: &str) -> Vec<String> {
        let prefix = format!("{}::", prelude_prefix);
        self.functions
            .values()
            .map(|function| self.definition_name(function.id, &function.name))
            .chain(
                self.structs
                    .values()
                    .map(|value| self.definition_name(value.id, &value.name)),
            )
            .chain(
                self.enums
                    .values()
                    .map(|value| self.definition_name(value.id, &value.name)),
            )
            .chain(
                self.type_aliases
                    .values()
                    .map(|value| self.definition_name(value.id, &value.name)),
            )
            .chain(
                self.traits
                    .values()
                    .map(|value| self.definition_name(value.id, &value.name)),
            )
            .chain(
                self.externs
                    .values()
                    .map(|ext| self.definition_name(ext.id, &ext.name)),
            )
            .filter(|name| name.starts_with(&prefix))
            .collect()
    }

    fn definition_name(&self, id: DefId, fallback: &str) -> String {
        self.canonical_names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    }
}

pub(crate) fn hir_type_alias_from_interface(alias: &ProductTypeAliasInterface) -> HirTypeAlias {
    HirTypeAlias {
        id: alias.id,
        name: alias.name.clone(),
        generic_params: alias.generic_params.clone(),
        ty: alias.ty.clone(),
    }
}

pub(crate) fn hir_function_from_interface(function: &ProductFunctionInterface) -> HirFunction {
    HirFunction {
        id: function.id,
        name: function.name.clone(),
        generic_params: function.generic_params.clone(),
        generic_bounds: function.generic_bounds.clone(),
        params: function
            .params
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("arg{}", index),
                local_id: crate::ids::HirLocalId(index as u32),
                ty: ty.clone(),
                mutable: false,
                is_ref: false,
            })
            .collect(),
        ret_type: function.ret_type.clone(),
        body: HirBlock {
            stmts: Vec::new(),
            ty: function.ret_type.clone(),
        },
        is_curried: function.is_curried,
        is_method: function.is_method,
        self_receiver: function.self_receiver,
        is_unsafe: function.is_unsafe,
    }
}

pub(crate) fn hir_struct_from_interface(strukt: &ProductStructInterface) -> HirStruct {
    HirStruct {
        id: strukt.id,
        name: strukt.name.clone(),
        generic_params: strukt.generic_params.clone(),
        fields: strukt
            .fields
            .iter()
            .map(|field| HirField {
                id: field.id,
                name: field.name.clone(),
                ty: field.ty.clone(),
                public: field.public,
            })
            .collect(),
    }
}

pub(crate) fn hir_enum_from_interface(enm: &ProductEnumInterface) -> HirEnum {
    HirEnum {
        id: enm.id,
        name: enm.name.clone(),
        generic_params: enm.generic_params.clone(),
        variants: enm
            .variants
            .iter()
            .map(hir_variant_from_interface)
            .collect(),
    }
}

fn hir_variant_from_interface(variant: &ProductEnumVariantInterface) -> HirVariant {
    HirVariant {
        id: variant.id,
        name: variant.name.clone(),
        fields: variant.fields.clone(),
    }
}

pub(crate) fn hir_trait_from_interface(trt: &ProductTraitInterface) -> HirTrait {
    HirTrait {
        id: trt.id,
        name: trt.name.clone(),
        generic_params: trt.generic_params.clone(),
        target: trt.target.clone(),
        predicates: trt.predicates.clone(),
        associated_types: trt.associated_types.clone(),
        methods: trt
            .methods
            .iter()
            .map(|(name, method)| (name.clone(), hir_function_from_interface(method)))
            .collect(),
        signatures: trt.signatures.clone(),
    }
}

pub(crate) fn hir_impl_from_interface(imp: &ProductImplInterface) -> HirImpl {
    HirImpl {
        id: imp.id,
        owner: imp.owner.clone(),
        type_name: imp.type_name.clone(),
        type_generics: imp.type_generics.clone(),
        receiver_pattern: imp.receiver_pattern.clone(),
        trait_name: imp.trait_name.clone(),
        trait_id: imp.trait_id,
        trait_generics: imp.trait_generics.clone(),
        trait_arg_types: imp.trait_arg_types.clone(),
        associated_types: imp
            .associated_types
            .iter()
            .map(hir_assoc_type_from_interface)
            .collect(),
        bounds: imp.bounds.clone(),
        methods: imp
            .methods
            .iter()
            .map(|(name, method)| (name.clone(), hir_function_from_interface(method)))
            .collect(),
    }
}

fn hir_assoc_type_from_interface(assoc: &ProductAssociatedTypeInterface) -> HirAssociatedTypeDef {
    HirAssociatedTypeDef {
        id: assoc.id,
        name: assoc.name.clone(),
        kind: assoc.kind.clone(),
        ty: assoc.ty.clone(),
    }
}

pub(crate) fn hir_extern_from_interface(ext: &ProductExternInterface) -> HirExtern {
    HirExtern {
        id: ext.id,
        name: ext.name.clone(),
        params: ext.params.clone(),
        ret: ext.ret.clone(),
        variadic: ext.variadic,
        is_unsafe: ext.is_unsafe,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactCrossCrateHir {
    pub generic_functions: BTreeMap<DefId, AcceptedHirFunction>,
    pub traits_with_defaults: BTreeMap<DefId, AcceptedHirTrait>,
    pub generic_impls: Vec<AcceptedHirImpl>,
}
