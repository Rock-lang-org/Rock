use std::collections::HashMap;

use crate::ids::{DefId, TypeId};
use crate::types::{GenericParamId, Type};

use super::CodegenEnumVariantLayout;

/// Struct and enum layouts registered once from the MIR backend contract, then queried
/// while lowering types, aggregates and projections.
#[derive(Default)]
pub(super) struct NominalLayouts {
    struct_fields: HashMap<DefId, Vec<(String, TypeId)>>,
    struct_generic_params: HashMap<DefId, Vec<GenericParamId>>,
    enum_variants: HashMap<DefId, Vec<CodegenEnumVariantLayout>>,
    /// Generic params used to monomorphize enum payload layouts.
    enum_generic_params: HashMap<DefId, Vec<GenericParamId>>,
}

impl NominalLayouts {
    pub(super) fn register_struct(
        &mut self,
        id: DefId,
        fields: Vec<(String, TypeId)>,
        generic_params: Vec<GenericParamId>,
    ) {
        self.struct_fields.insert(id, fields);
        self.struct_generic_params.insert(id, generic_params);
    }

    pub(super) fn register_enum(
        &mut self,
        id: DefId,
        variants: Vec<CodegenEnumVariantLayout>,
        generic_params: Vec<GenericParamId>,
    ) {
        self.enum_variants.insert(id, variants);
        self.enum_generic_params.insert(id, generic_params);
    }

    pub(super) fn struct_fields(&self, id: &DefId) -> Option<&Vec<(String, TypeId)>> {
        self.struct_fields.get(id)
    }

    pub(super) fn enum_variants(&self, id: &DefId) -> Option<&Vec<CodegenEnumVariantLayout>> {
        self.enum_variants.get(id)
    }

    pub(super) fn struct_substitution(
        &self,
        id: DefId,
        type_args: &[Type],
    ) -> HashMap<GenericParamId, Type> {
        Self::substitution(self.struct_generic_params.get(&id), type_args)
    }

    pub(super) fn enum_substitution(
        &self,
        id: DefId,
        type_args: &[Type],
    ) -> HashMap<GenericParamId, Type> {
        Self::substitution(self.enum_generic_params.get(&id), type_args)
    }

    fn substitution(
        generic_param_ids: Option<&Vec<GenericParamId>>,
        type_args: &[Type],
    ) -> HashMap<GenericParamId, Type> {
        generic_param_ids
            .map(|generic_param_ids| {
                generic_param_ids
                    .iter()
                    .zip(type_args.iter())
                    .map(|(generic_id, ty)| (*generic_id, ty.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}
