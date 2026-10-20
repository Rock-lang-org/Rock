use crate::hir::{
    HirEnum, HirExtern, HirFunction, HirFunctionSig, HirImpl, HirStruct, HirTrait, HirTypeAlias,
};
use crate::ids::DefId;
use crate::lower::ResolveError;
use std::collections::HashMap;

/// Canonical declaration payloads produced by collection.
#[derive(Debug, Default)]
pub struct DeclarationItems {
    functions: HashMap<DefId, HirFunction>,
    function_sigs: HashMap<DefId, HirFunctionSig>,
    structs: HashMap<DefId, HirStruct>,
    enums: HashMap<DefId, HirEnum>,
    traits: HashMap<DefId, HirTrait>,
    impls: HashMap<DefId, HirImpl>,
    externs: HashMap<DefId, HirExtern>,
    type_aliases: HashMap<DefId, HirTypeAlias>,
}

impl DeclarationItems {
    pub fn from_id_maps(
        functions: HashMap<DefId, HirFunction>,
        function_sigs: HashMap<DefId, HirFunctionSig>,
        structs: HashMap<DefId, HirStruct>,
        enums: HashMap<DefId, HirEnum>,
        traits: HashMap<DefId, HirTrait>,
        impls: HashMap<DefId, HirImpl>,
        externs: HashMap<DefId, HirExtern>,
    ) -> Result<Self, Vec<ResolveError>> {
        Self::from_id_maps_with_aliases(
            functions,
            function_sigs,
            structs,
            enums,
            traits,
            impls,
            externs,
            HashMap::new(),
        )
    }

    pub fn from_id_maps_with_aliases(
        functions: HashMap<DefId, HirFunction>,
        function_sigs: HashMap<DefId, HirFunctionSig>,
        structs: HashMap<DefId, HirStruct>,
        enums: HashMap<DefId, HirEnum>,
        traits: HashMap<DefId, HirTrait>,
        impls: HashMap<DefId, HirImpl>,
        externs: HashMap<DefId, HirExtern>,
        type_aliases: HashMap<DefId, HirTypeAlias>,
    ) -> Result<Self, Vec<ResolveError>> {
        let items = Self {
            functions,
            function_sigs,
            structs,
            enums,
            traits,
            impls,
            externs,
            type_aliases,
        };
        items.validate()?;
        Ok(items)
    }

    #[allow(dead_code)]
    pub(crate) fn from_named_maps(
        functions: HashMap<String, HirFunction>,
        function_sigs: HashMap<String, HirFunctionSig>,
        structs: HashMap<String, HirStruct>,
        enums: HashMap<String, HirEnum>,
        traits: HashMap<String, HirTrait>,
        impls: Vec<HirImpl>,
        externs: Vec<HirExtern>,
        canonical_names_by_id: &HashMap<DefId, String>,
    ) -> Result<Self, Vec<ResolveError>> {
        Self::from_named_maps_with_aliases(
            functions,
            function_sigs,
            structs,
            enums,
            traits,
            impls,
            externs,
            HashMap::new(),
            canonical_names_by_id,
        )
    }

    pub(crate) fn from_named_maps_with_aliases(
        functions: impl IntoIterator<Item = (String, HirFunction)>,
        function_sigs: impl IntoIterator<Item = (String, HirFunctionSig)>,
        structs: impl IntoIterator<Item = (String, HirStruct)>,
        enums: impl IntoIterator<Item = (String, HirEnum)>,
        traits: impl IntoIterator<Item = (String, HirTrait)>,
        impls: Vec<HirImpl>,
        externs: Vec<HirExtern>,
        type_aliases: impl IntoIterator<Item = (String, HirTypeAlias)>,
        canonical_names_by_id: &HashMap<DefId, String>,
    ) -> Result<Self, Vec<ResolveError>> {
        let mut errors = Vec::new();
        let functions = collapse_named_declarations(
            functions,
            canonical_names_by_id,
            |function| function.id,
            declarations_are_same_function,
            "function declaration",
            &mut errors,
        );
        let function_sigs = collapse_named_declarations(
            function_sigs,
            canonical_names_by_id,
            |signature| signature.id,
            declarations_are_same_function_sig,
            "function signature declaration",
            &mut errors,
        );
        let structs = collapse_named_declarations(
            structs,
            canonical_names_by_id,
            |structure| structure.id,
            declarations_are_same_struct,
            "struct declaration",
            &mut errors,
        );
        let enums = collapse_named_declarations(
            enums,
            canonical_names_by_id,
            |enumeration| enumeration.id,
            declarations_are_same_enum,
            "enum declaration",
            &mut errors,
        );
        let traits = collapse_named_declarations(
            traits,
            canonical_names_by_id,
            |trait_def| trait_def.id,
            declarations_are_same_trait,
            "trait declaration",
            &mut errors,
        );
        let impls = id_map_from_values(impls, |imp| imp.id, "impl declaration", &mut errors);
        let externs = id_map_from_values(
            externs,
            |extern_| extern_.id,
            "extern declaration",
            &mut errors,
        );
        let type_aliases = collapse_named_declarations(
            type_aliases,
            canonical_names_by_id,
            |alias| alias.id,
            |left, right| left == right,
            "type alias declaration",
            &mut errors,
        );

        match Self::from_id_maps_with_aliases(
            functions,
            function_sigs,
            structs,
            enums,
            traits,
            impls,
            externs,
            type_aliases,
        ) {
            Ok(items) if errors.is_empty() => Ok(items),
            Ok(_) => Err(errors),
            Err(mut validation_errors) => {
                errors.append(&mut validation_errors);
                Err(errors)
            }
        }
    }

    pub(crate) fn validate(&self) -> Result<(), Vec<ResolveError>> {
        let mut errors = Vec::new();
        validate_id_map(
            &self.functions,
            |function| function.id,
            "function declaration",
            &mut errors,
        );
        validate_id_map(
            &self.type_aliases,
            |alias| alias.id,
            "type alias declaration",
            &mut errors,
        );
        validate_id_map(
            &self.function_sigs,
            |signature| signature.id,
            "function signature declaration",
            &mut errors,
        );
        validate_id_map(
            &self.structs,
            |structure| structure.id,
            "struct declaration",
            &mut errors,
        );
        validate_id_map(
            &self.enums,
            |enumeration| enumeration.id,
            "enum declaration",
            &mut errors,
        );
        validate_id_map(
            &self.traits,
            |trait_def| trait_def.id,
            "trait declaration",
            &mut errors,
        );
        validate_id_map(&self.impls, |imp| imp.id, "impl declaration", &mut errors);
        validate_id_map(
            &self.externs,
            |extern_| extern_.id,
            "extern declaration",
            &mut errors,
        );

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn functions(&self) -> &HashMap<DefId, HirFunction> {
        &self.functions
    }

    pub fn function_sigs(&self) -> &HashMap<DefId, HirFunctionSig> {
        &self.function_sigs
    }

    pub fn structs(&self) -> &HashMap<DefId, HirStruct> {
        &self.structs
    }

    pub fn enums(&self) -> &HashMap<DefId, HirEnum> {
        &self.enums
    }

    pub fn traits(&self) -> &HashMap<DefId, HirTrait> {
        &self.traits
    }

    pub fn impls(&self) -> &HashMap<DefId, HirImpl> {
        &self.impls
    }

    pub fn externs(&self) -> &HashMap<DefId, HirExtern> {
        &self.externs
    }

    pub fn type_aliases(&self) -> &HashMap<DefId, HirTypeAlias> {
        &self.type_aliases
    }

    pub fn function(&self, id: DefId) -> Option<&HirFunction> {
        self.functions.get(&id)
    }

    pub(crate) fn function_mut(&mut self, id: DefId) -> Option<&mut HirFunction> {
        self.functions.get_mut(&id)
    }

    pub(crate) fn function_defs(&self) -> impl Iterator<Item = (DefId, &HirFunction)> {
        self.functions.iter().map(|(&id, function)| (id, function))
    }

    pub(crate) fn insert_function(&mut self, function: HirFunction) -> Option<HirFunction> {
        self.functions.insert(function.id, function)
    }

    pub(crate) fn remove_function(&mut self, id: DefId) -> Option<HirFunction> {
        self.functions.remove(&id)
    }

    pub(crate) fn function_sig(&self, id: DefId) -> Option<&HirFunctionSig> {
        self.function_sigs.get(&id)
    }

    pub(crate) fn function_sig_mut(&mut self, id: DefId) -> Option<&mut HirFunctionSig> {
        self.function_sigs.get_mut(&id)
    }

    pub(crate) fn function_sig_defs(&self) -> impl Iterator<Item = (DefId, &HirFunctionSig)> {
        self.function_sigs
            .iter()
            .map(|(&id, signature)| (id, signature))
    }

    pub(crate) fn insert_function_sig(
        &mut self,
        signature: HirFunctionSig,
    ) -> Option<HirFunctionSig> {
        self.function_sigs.insert(signature.id, signature)
    }

    pub(crate) fn remove_function_sig(&mut self, id: DefId) -> Option<HirFunctionSig> {
        self.function_sigs.remove(&id)
    }

    pub(crate) fn structure(&self, id: DefId) -> Option<&HirStruct> {
        self.structs.get(&id)
    }

    pub(crate) fn structure_mut(&mut self, id: DefId) -> Option<&mut HirStruct> {
        self.structs.get_mut(&id)
    }

    pub(crate) fn structure_defs(&self) -> impl Iterator<Item = (DefId, &HirStruct)> {
        self.structs.iter().map(|(&id, structure)| (id, structure))
    }

    pub(crate) fn insert_structure(&mut self, structure: HirStruct) -> Option<HirStruct> {
        self.structs.insert(structure.id, structure)
    }

    pub(crate) fn remove_structure(&mut self, id: DefId) -> Option<HirStruct> {
        self.structs.remove(&id)
    }

    pub(crate) fn enumeration(&self, id: DefId) -> Option<&HirEnum> {
        self.enums.get(&id)
    }

    pub(crate) fn enumeration_mut(&mut self, id: DefId) -> Option<&mut HirEnum> {
        self.enums.get_mut(&id)
    }

    pub(crate) fn enumeration_defs(&self) -> impl Iterator<Item = (DefId, &HirEnum)> {
        self.enums
            .iter()
            .map(|(&id, enumeration)| (id, enumeration))
    }

    pub(crate) fn insert_enumeration(&mut self, enumeration: HirEnum) -> Option<HirEnum> {
        self.enums.insert(enumeration.id, enumeration)
    }

    pub(crate) fn remove_enumeration(&mut self, id: DefId) -> Option<HirEnum> {
        self.enums.remove(&id)
    }

    pub(crate) fn trait_def(&self, id: DefId) -> Option<&HirTrait> {
        self.traits.get(&id)
    }

    pub(crate) fn trait_def_mut(&mut self, id: DefId) -> Option<&mut HirTrait> {
        self.traits.get_mut(&id)
    }

    pub(crate) fn trait_defs(&self) -> impl Iterator<Item = (DefId, &HirTrait)> {
        self.traits.iter().map(|(&id, trait_def)| (id, trait_def))
    }

    pub(crate) fn insert_trait_def(&mut self, trait_def: HirTrait) -> Option<HirTrait> {
        self.traits.insert(trait_def.id, trait_def)
    }

    pub(crate) fn remove_trait_def(&mut self, id: DefId) -> Option<HirTrait> {
        self.traits.remove(&id)
    }

    pub(crate) fn impl_def(&self, id: DefId) -> Option<&HirImpl> {
        self.impls.get(&id)
    }

    pub(crate) fn impl_def_mut(&mut self, id: DefId) -> Option<&mut HirImpl> {
        self.impls.get_mut(&id)
    }

    pub(crate) fn impl_defs(&self) -> impl Iterator<Item = (DefId, &HirImpl)> {
        self.impls.iter().map(|(&id, impl_def)| (id, impl_def))
    }

    pub(crate) fn insert_impl(&mut self, impl_def: HirImpl) -> Option<HirImpl> {
        self.impls.insert(impl_def.id, impl_def)
    }

    pub(crate) fn remove_impl(&mut self, id: DefId) -> Option<HirImpl> {
        self.impls.remove(&id)
    }

    pub(crate) fn extern_def(&self, id: DefId) -> Option<&HirExtern> {
        self.externs.get(&id)
    }

    pub(crate) fn extern_mut(&mut self, id: DefId) -> Option<&mut HirExtern> {
        self.externs.get_mut(&id)
    }

    pub(crate) fn extern_defs(&self) -> impl Iterator<Item = (DefId, &HirExtern)> {
        self.externs
            .iter()
            .map(|(&id, extern_def)| (id, extern_def))
    }

    pub(crate) fn insert_extern(&mut self, extern_def: HirExtern) -> Option<HirExtern> {
        self.externs.insert(extern_def.id, extern_def)
    }

    pub(crate) fn remove_extern(&mut self, id: DefId) -> Option<HirExtern> {
        self.externs.remove(&id)
    }

    pub(crate) fn type_alias(&self, id: DefId) -> Option<&HirTypeAlias> {
        self.type_aliases.get(&id)
    }

    pub(crate) fn type_alias_defs(&self) -> impl Iterator<Item = (DefId, &HirTypeAlias)> {
        self.type_aliases.iter().map(|(&id, alias)| (id, alias))
    }

    pub(crate) fn insert_type_alias(&mut self, alias: HirTypeAlias) -> Option<HirTypeAlias> {
        self.type_aliases.insert(alias.id, alias)
    }

    pub(crate) fn into_id_maps(
        self,
    ) -> (
        HashMap<DefId, HirFunction>,
        HashMap<DefId, HirFunctionSig>,
        HashMap<DefId, HirStruct>,
        HashMap<DefId, HirEnum>,
        HashMap<DefId, HirTrait>,
        HashMap<DefId, HirImpl>,
        HashMap<DefId, HirExtern>,
        HashMap<DefId, HirTypeAlias>,
    ) {
        (
            self.functions,
            self.function_sigs,
            self.structs,
            self.enums,
            self.traits,
            self.impls,
            self.externs,
            self.type_aliases,
        )
    }
}

fn collapse_named_declarations<T, IdOf, Same>(
    entries: impl IntoIterator<Item = (String, T)>,
    canonical_names_by_id: &HashMap<DefId, String>,
    id_of: IdOf,
    same_definition: Same,
    kind: &str,
    errors: &mut Vec<ResolveError>,
) -> HashMap<DefId, T>
where
    IdOf: Fn(&T) -> DefId,
    Same: Fn(&T, &T) -> bool,
{
    let mut grouped: HashMap<DefId, Vec<(String, T)>> = HashMap::new();
    for (name, item) in entries {
        grouped.entry(id_of(&item)).or_default().push((name, item));
    }

    let mut grouped: Vec<_> = grouped.into_iter().collect();
    grouped.sort_by_key(|(id, _)| *id);

    let mut output = HashMap::new();
    for (id, mut candidates) in grouped {
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        let selected_index = canonical_names_by_id
            .get(&id)
            .and_then(|canonical_name| {
                candidates
                    .iter()
                    .position(|(name, _)| name == canonical_name)
            })
            .unwrap_or(0);
        let selected = &candidates[selected_index].1;
        if candidates
            .iter()
            .any(|(_, candidate)| !same_definition(selected, candidate))
        {
            errors.push(ResolveError::non_source(format!(
                "conflicting {kind} payloads for canonical DefId {id:?}"
            )));
            continue;
        }
        output.insert(id, candidates.swap_remove(selected_index).1);
    }
    output
}

fn id_map_from_values<T, IdOf>(
    mut values: Vec<T>,
    id_of: IdOf,
    kind: &str,
    errors: &mut Vec<ResolveError>,
) -> HashMap<DefId, T>
where
    IdOf: Fn(&T) -> DefId,
{
    values.sort_by_key(|value| id_of(value));

    let mut output = HashMap::new();
    for value in values {
        let id = id_of(&value);
        if output.insert(id, value).is_some() {
            errors.push(ResolveError::non_source(format!(
                "duplicate {kind} payload for canonical DefId {id:?}"
            )));
        }
    }
    output
}

fn validate_id_map<T, IdOf>(
    entries: &HashMap<DefId, T>,
    id_of: IdOf,
    kind: &str,
    errors: &mut Vec<ResolveError>,
) where
    IdOf: Fn(&T) -> DefId,
{
    let mut entries: Vec<_> = entries.iter().collect();
    entries.sort_by_key(|(key, _)| **key);

    for (key, item) in entries {
        let payload_id = id_of(item);
        if *key != payload_id {
            errors.push(ResolveError::non_source(format!(
                "{kind} key {key:?} does not match payload DefId {payload_id:?}"
            )));
        }
    }
}

fn declarations_are_same_function(left: &HirFunction, right: &HirFunction) -> bool {
    left.id == right.id
        && left.generic_params == right.generic_params
        && same_generic_bounds(&left.generic_bounds, &right.generic_bounds)
        && left.params.len() == right.params.len()
        && left.params.iter().zip(&right.params).all(|(left, right)| {
            left.name == right.name
                && left.local_id == right.local_id
                && left.ty == right.ty
                && left.mutable == right.mutable
                && left.is_ref == right.is_ref
        })
        && left.ret_type == right.ret_type
        && left.is_curried == right.is_curried
        && left.is_method == right.is_method
        && left.self_receiver == right.self_receiver
        && left.is_unsafe == right.is_unsafe
}

fn declarations_are_same_function_sig(left: &HirFunctionSig, right: &HirFunctionSig) -> bool {
    left.id == right.id
        && left.generic_params == right.generic_params
        && left.params == right.params
        && left.ret == right.ret
        && same_generic_bounds(&left.generic_bounds, &right.generic_bounds)
        && left.self_receiver == right.self_receiver
        && left.is_unsafe == right.is_unsafe
}

fn declarations_are_same_struct(left: &HirStruct, right: &HirStruct) -> bool {
    left.id == right.id
        && left.generic_params == right.generic_params
        && same_fields(&left.fields, &right.fields)
}

fn declarations_are_same_enum(left: &HirEnum, right: &HirEnum) -> bool {
    left.id == right.id
        && left.generic_params == right.generic_params
        && left.variants.len() == right.variants.len()
        && left
            .variants
            .iter()
            .zip(&right.variants)
            .all(|(left, right)| {
                left.id == right.id
                    && left.name == right.name
                    && same_variant_fields(&left.fields, &right.fields)
            })
}

fn declarations_are_same_trait(left: &HirTrait, right: &HirTrait) -> bool {
    left.id == right.id
        && left.generic_params == right.generic_params
        && left.associated_types.len() == right.associated_types.len()
        && left
            .associated_types
            .iter()
            .zip(&right.associated_types)
            .all(|(left, right)| left.id == right.id && left.name == right.name)
        && same_map(
            &left.methods,
            &right.methods,
            declarations_are_same_function,
        )
        && same_map(
            &left.signatures,
            &right.signatures,
            declarations_are_same_function_sig,
        )
}

fn same_generic_bounds(
    left: &crate::hir::HirGenericBounds,
    right: &crate::hir::HirGenericBounds,
) -> bool {
    same_map(left, right, |left, right| left == right)
}

fn same_fields(left: &[crate::hir::HirField], right: &[crate::hir::HirField]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.id == right.id
                && left.name == right.name
                && left.ty == right.ty
                && left.public == right.public
        })
}

fn same_variant_fields(
    left: &crate::hir::HirVariantFields,
    right: &crate::hir::HirVariantFields,
) -> bool {
    match (left, right) {
        (crate::hir::HirVariantFields::Named(left), crate::hir::HirVariantFields::Named(right)) => {
            same_fields(left, right)
        }
        (
            crate::hir::HirVariantFields::Positional(left),
            crate::hir::HirVariantFields::Positional(right),
        ) => left == right,
        (crate::hir::HirVariantFields::Unit, crate::hir::HirVariantFields::Unit) => true,
        _ => false,
    }
}

fn same_map<K, V>(
    left: &HashMap<K, V>,
    right: &HashMap<K, V>,
    same_value: impl Fn(&V, &V) -> bool,
) -> bool
where
    K: Eq + std::hash::Hash,
{
    left.len() == right.len()
        && left.iter().all(|(key, left_value)| {
            right
                .get(key)
                .is_some_and(|right_value| same_value(left_value, right_value))
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::hir::HirBlock;
    use crate::ids::{CrateId, LocalDefId};
    use crate::types::{GenericParamDecl, GenericParamId, TraitBound, Type};

    fn def_id(local: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(local))
    }

    fn function(
        id: DefId,
        name: &str,
        generic_bounds: HashMap<GenericParamId, Vec<TraitBound>>,
    ) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: vec![
                GenericParamDecl::type_param(
                    GenericParamId {
                        owner: id,
                        index: 0,
                    },
                    "T",
                ),
                GenericParamDecl::type_param(
                    GenericParamId {
                        owner: id,
                        index: 1,
                    },
                    "U",
                ),
            ],
            generic_bounds: generic_bounds.into(),
            params: vec![],
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: vec![],
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    #[test]
    fn declaration_items_accept_equivalent_alias_clones_with_different_bound_insertion_order() {
        let id = def_id(1);
        let first_bound = TraitBound {
            trait_id: def_id(2),
            type_args: vec![],
        };
        let second_bound = TraitBound {
            trait_id: def_id(3),
            type_args: vec![],
        };
        let first_bounds = HashMap::from([
            (
                GenericParamId {
                    owner: id,
                    index: 0,
                },
                vec![first_bound.clone()],
            ),
            (
                GenericParamId {
                    owner: id,
                    index: 1,
                },
                vec![second_bound.clone()],
            ),
        ]);
        let second_bounds = HashMap::from([
            (
                GenericParamId {
                    owner: id,
                    index: 1,
                },
                vec![second_bound],
            ),
            (
                GenericParamId {
                    owner: id,
                    index: 0,
                },
                vec![first_bound],
            ),
        ]);

        let items = DeclarationItems::from_named_maps(
            HashMap::from([
                ("alias".to_string(), function(id, "alias", first_bounds)),
                (
                    "canonical".to_string(),
                    function(id, "canonical", second_bounds),
                ),
            ]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            vec![],
            vec![],
            &HashMap::from([(id, "canonical".to_string())]),
        )
        .expect("equivalent aliases should collapse");

        assert_eq!(items.function(id).unwrap().name, "canonical");
    }

    #[test]
    fn declaration_items_reject_conflicting_alias_clones() {
        let id = def_id(1);
        let mut conflicting = function(id, "canonical", HashMap::new());
        conflicting.ret_type = Type::I64;

        let errors = DeclarationItems::from_named_maps(
            HashMap::from([
                ("alias".to_string(), function(id, "alias", HashMap::new())),
                ("canonical".to_string(), conflicting),
            ]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            vec![],
            vec![],
            &HashMap::from([(id, "canonical".to_string())]),
        )
        .unwrap_err();

        assert!(errors[0]
            .message
            .contains("conflicting function declaration payloads"));
    }

    #[test]
    fn declaration_item_diagnostics_are_deterministic_across_input_orders() {
        let function_ids = [def_id(7), def_id(4)];
        let extern_ids = [def_id(6), def_id(2)];
        let expected_named_errors = vec![
            format!(
                "conflicting function declaration payloads for canonical DefId {:?}",
                function_ids[1]
            ),
            format!(
                "conflicting function declaration payloads for canonical DefId {:?}",
                function_ids[0]
            ),
            format!(
                "duplicate extern declaration payload for canonical DefId {:?}",
                extern_ids[1]
            ),
            format!(
                "duplicate extern declaration payload for canonical DefId {:?}",
                extern_ids[0]
            ),
        ];

        for reverse in [false, true] {
            let mut function_entries = vec![
                ("seven-alias", function_ids[0], false),
                ("seven-canonical", function_ids[0], true),
                ("four-alias", function_ids[1], false),
                ("four-canonical", function_ids[1], true),
            ];
            let mut extern_entries = vec![
                HirExtern {
                    id: extern_ids[0],
                    name: "six-first".to_string(),
                    params: vec![],
                    ret: Type::Unit,
                    variadic: false,
                    is_unsafe: false,
                },
                HirExtern {
                    id: extern_ids[1],
                    name: "two-first".to_string(),
                    params: vec![],
                    ret: Type::Unit,
                    variadic: false,
                    is_unsafe: false,
                },
                HirExtern {
                    id: extern_ids[0],
                    name: "six-second".to_string(),
                    params: vec![],
                    ret: Type::Unit,
                    variadic: false,
                    is_unsafe: false,
                },
                HirExtern {
                    id: extern_ids[1],
                    name: "two-second".to_string(),
                    params: vec![],
                    ret: Type::Unit,
                    variadic: false,
                    is_unsafe: false,
                },
            ];
            if reverse {
                function_entries.reverse();
                extern_entries.reverse();
            }

            let functions = function_entries
                .into_iter()
                .map(|(name, id, conflicting)| {
                    let mut function = function(id, name, HashMap::new());
                    if conflicting {
                        function.ret_type = Type::I64;
                    }
                    (name.to_string(), function)
                })
                .collect();
            let errors = DeclarationItems::from_named_maps(
                functions,
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                vec![],
                extern_entries,
                &HashMap::from([
                    (function_ids[0], "seven-canonical".to_string()),
                    (function_ids[1], "four-canonical".to_string()),
                ]),
            )
            .unwrap_err();
            assert_eq!(
                errors
                    .into_iter()
                    .map(|error| error.message)
                    .collect::<Vec<_>>(),
                expected_named_errors
            );
        }

        let expected_validation_errors = vec![
            format!(
                "function declaration key {:?} does not match payload DefId {:?}",
                def_id(3),
                def_id(12)
            ),
            format!(
                "function declaration key {:?} does not match payload DefId {:?}",
                def_id(9),
                def_id(11)
            ),
            format!(
                "struct declaration key {:?} does not match payload DefId {:?}",
                def_id(2),
                def_id(14)
            ),
            format!(
                "struct declaration key {:?} does not match payload DefId {:?}",
                def_id(8),
                def_id(13)
            ),
        ];

        for reverse in [false, true] {
            let mut functions = vec![(def_id(9), def_id(11)), (def_id(3), def_id(12))];
            let mut structs = vec![(def_id(8), def_id(13)), (def_id(2), def_id(14))];
            if reverse {
                functions.reverse();
                structs.reverse();
            }
            let functions = functions
                .into_iter()
                .map(|(key, id)| (key, function(id, "function", HashMap::new())))
                .collect();
            let structs = structs
                .into_iter()
                .map(|(key, id)| {
                    (
                        key,
                        HirStruct {
                            id,
                            name: "structure".to_string(),
                            generic_params: vec![],
                            fields: vec![],
                        },
                    )
                })
                .collect();
            let errors = DeclarationItems::from_id_maps(
                functions,
                HashMap::new(),
                structs,
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
            )
            .unwrap_err();
            assert_eq!(
                errors
                    .into_iter()
                    .map(|error| error.message)
                    .collect::<Vec<_>>(),
                expected_validation_errors
            );
        }
    }
}
