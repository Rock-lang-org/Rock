use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirMethodLocation {
    TraitDefault {
        trait_id: DefId,
        method_id: DefId,
        method_name: String,
    },
    ImplMethod {
        impl_id: DefId,
        method_id: DefId,
        method_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HirFieldLocation {
    pub owner: DefId,
    pub field_id: FieldId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HirVariantLocation {
    pub owner: DefId,
    pub variant_id: VariantId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HirAssociatedTypeLocation {
    pub owner: DefId,
    pub assoc_type_id: AssocTypeId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HirDefinitionIndexes {
    pub functions_by_id: HashMap<DefId, String>,
    pub structs_by_id: HashMap<DefId, String>,
    pub enums_by_id: HashMap<DefId, String>,
    pub traits_by_id: HashMap<DefId, String>,
    pub impls_by_id: HashMap<DefId, usize>,
    pub impl_owner_ids_by_id: HashMap<DefId, DefId>,
    pub externs_by_id: HashMap<DefId, usize>,
    pub type_aliases_by_id: HashMap<DefId, String>,
    pub methods_by_id: HashMap<DefId, HirMethodLocation>,
    /// For a trait implementation, the concrete method selected for each trait
    /// member. This is the post-conformance authority used by later compiler
    /// phases; they must not reconstruct the relationship from method names.
    pub effective_trait_methods: HashMap<(DefId, DefId), DefId>,
    pub fields_by_id: HashMap<(DefId, FieldId), HirFieldLocation>,
    pub variants_by_id: HashMap<(DefId, VariantId), HirVariantLocation>,
    pub associated_type_decls_by_id: HashMap<(DefId, AssocTypeId), HirAssociatedTypeLocation>,
    pub associated_type_defs_by_id: HashMap<(DefId, AssocTypeId), HirAssociatedTypeLocation>,
}

#[derive(Clone, Debug)]
struct TraitDefaultMethodCandidate {
    trait_id: DefId,
    trait_name: String,
    method_name: String,
}

pub(super) fn display_aliases_for_id<'a>(
    names_by_name: &'a HashMap<String, DefId>,
    owner_id: DefId,
    primary_name: Option<&'a str>,
) -> Vec<&'a str> {
    let mut names = primary_name.into_iter().collect::<Vec<_>>();
    let mut aliases = names_by_name
        .iter()
        .filter_map(|(name, id)| (*id == owner_id).then_some(name.as_str()))
        .collect::<Vec<_>>();
    aliases.sort();

    for alias in aliases {
        if !names.contains(&alias) {
            names.push(alias);
        }
    }

    names
}

pub(super) fn sorted_definition_ids<T>(definitions: &HashMap<DefId, T>) -> Vec<DefId> {
    let mut ids = definitions.keys().copied().collect::<Vec<_>>();
    ids.sort();
    ids
}

impl HirDefinitionIndexes {
    pub(super) fn from_parts<P: HirPhase>(
        _functions: &HashMap<DefId, HirFunctionFor<P>>,
        structs: &HashMap<DefId, HirStruct>,
        enums: &HashMap<DefId, HirEnum>,
        traits: &HashMap<DefId, HirTraitFor<P>>,
        impls: &HashMap<DefId, HirImplFor<P>>,
        externs: &HashMap<DefId, HirExtern>,
        type_aliases: &HashMap<DefId, HirTypeAlias>,
        names: &HirNameTables,
        order: &HirProgramOrder,
        canonical_names_by_id: &HashMap<DefId, String>,
    ) -> Self {
        let mut indexes = Self::default();

        index_owned_named_definitions(
            &mut indexes.functions_by_id,
            _functions,
            &names.functions_by_name,
            |function| function.name.as_str(),
            canonical_names_by_id,
            "function",
        );
        index_owned_named_definitions(
            &mut indexes.structs_by_id,
            structs,
            &names.structs_by_name,
            |structure| structure.name.as_str(),
            canonical_names_by_id,
            "struct",
        );
        index_owned_named_definitions(
            &mut indexes.enums_by_id,
            enums,
            &names.enums_by_name,
            |enumeration| enumeration.name.as_str(),
            canonical_names_by_id,
            "enum",
        );
        index_owned_named_definitions(
            &mut indexes.traits_by_id,
            traits,
            &names.traits_by_name,
            |trait_def| trait_def.name.as_str(),
            canonical_names_by_id,
            "trait",
        );
        index_owned_named_definitions(
            &mut indexes.type_aliases_by_id,
            type_aliases,
            &names.type_aliases_by_name,
            |alias| alias.name.as_str(),
            canonical_names_by_id,
            "type alias",
        );

        for structure in structs.values() {
            for field in &structure.fields {
                insert_unique_child(
                    &mut indexes.fields_by_id,
                    (structure.id, field.id),
                    HirFieldLocation {
                        owner: structure.id,
                        field_id: field.id,
                        name: field.name.clone(),
                    },
                    "field",
                );
            }
        }

        for enumeration in enums.values() {
            for variant in &enumeration.variants {
                insert_unique_child(
                    &mut indexes.variants_by_id,
                    (enumeration.id, variant.id),
                    HirVariantLocation {
                        owner: enumeration.id,
                        variant_id: variant.id,
                        name: variant.name.clone(),
                    },
                    "variant",
                );

                if let HirVariantFields::Named(fields) = &variant.fields {
                    for field in fields {
                        insert_unique_child(
                            &mut indexes.fields_by_id,
                            (enumeration.id, field.id),
                            HirFieldLocation {
                                owner: enumeration.id,
                                field_id: field.id,
                                name: field.name.clone(),
                            },
                            "field",
                        );
                    }
                }
            }
        }

        for trait_def in traits.values() {
            for associated_type in &trait_def.associated_types {
                insert_unique_child(
                    &mut indexes.associated_type_decls_by_id,
                    (trait_def.id, associated_type.id),
                    HirAssociatedTypeLocation {
                        owner: trait_def.id,
                        assoc_type_id: associated_type.id,
                        name: associated_type.name.clone(),
                    },
                    "associated type declaration",
                );
            }
        }

        for (index, id) in ordered_owned_ids(impls, &order.impls).iter().enumerate() {
            let imp = &impls[id];
            insert_unique(&mut indexes.impls_by_id, *id, index, "impl");
            for associated_type in &imp.associated_types {
                insert_unique_child(
                    &mut indexes.associated_type_defs_by_id,
                    (imp.id, associated_type.id),
                    HirAssociatedTypeLocation {
                        owner: imp.id,
                        assoc_type_id: associated_type.id,
                        name: associated_type.name.clone(),
                    },
                    "associated type definition",
                );
            }
            for (method_name, method) in &imp.methods {
                insert_unique(
                    &mut indexes.methods_by_id,
                    method.id,
                    HirMethodLocation::ImplMethod {
                        impl_id: imp.id,
                        method_id: method.id,
                        method_name: method_name.clone(),
                    },
                    "method",
                );
            }
        }

        for (index, id) in ordered_owned_ids(externs, &order.externs)
            .iter()
            .enumerate()
        {
            let ext = &externs[id];
            insert_unique(&mut indexes.externs_by_id, ext.id, index, "extern");
        }

        index_trait_default_methods(
            &mut indexes.methods_by_id,
            traits,
            &names.traits_by_name,
            canonical_names_by_id,
        );

        indexes
    }
}

pub(super) fn ordered_owned_ids<T>(
    definitions: &HashMap<DefId, T>,
    preferred_order: &[DefId],
) -> Vec<DefId> {
    let mut seen = HashSet::new();
    let mut ids = preferred_order
        .iter()
        .copied()
        .filter(|id| definitions.contains_key(id) && seen.insert(*id))
        .collect::<Vec<_>>();

    let mut missing = definitions
        .keys()
        .copied()
        .filter(|id| seen.insert(*id))
        .collect::<Vec<_>>();
    missing.sort();
    ids.extend(missing);
    ids
}

fn index_trait_default_methods<P: HirPhase>(
    output: &mut HashMap<DefId, HirMethodLocation>,
    traits: &HashMap<DefId, HirTraitFor<P>>,
    traits_by_name: &HashMap<String, DefId>,
    canonical_names_by_id: &HashMap<DefId, String>,
) {
    let mut grouped: HashMap<DefId, Vec<TraitDefaultMethodCandidate>> = HashMap::new();
    let mut trait_ids = traits.keys().copied().collect::<Vec<_>>();
    trait_ids.sort();

    for trait_id in trait_ids {
        let trait_def = &traits[&trait_id];
        let mut trait_names = traits_by_name
            .iter()
            .filter_map(|(name, name_id)| (name_id == &trait_id).then(|| name.clone()))
            .collect::<Vec<_>>();
        if !trait_names.iter().any(|name| name == &trait_def.name) {
            trait_names.push(trait_def.name.clone());
        }
        trait_names.sort();
        trait_names.dedup();

        let mut method_names = trait_def.methods.keys().cloned().collect::<Vec<_>>();
        method_names.sort();

        for trait_name in trait_names {
            for method_name in &method_names {
                let method = &trait_def.methods[method_name];
                grouped
                    .entry(method.id)
                    .or_default()
                    .push(TraitDefaultMethodCandidate {
                        trait_id: trait_def.id,
                        trait_name: trait_name.clone(),
                        method_name: method_name.clone(),
                    });
            }
        }
    }

    let mut method_ids = grouped.keys().copied().collect::<Vec<_>>();
    method_ids.sort();

    for method_id in method_ids {
        let candidates = grouped.remove(&method_id).unwrap_or_default();
        let selected =
            select_trait_default_method_candidate(method_id, candidates, canonical_names_by_id);
        insert_unique(
            output,
            method_id,
            HirMethodLocation::TraitDefault {
                trait_id: selected.trait_id,
                method_id,
                method_name: selected.method_name,
            },
            "method",
        );
    }
}

fn select_trait_default_method_candidate(
    method_id: DefId,
    candidates: Vec<TraitDefaultMethodCandidate>,
    canonical_names_by_id: &HashMap<DefId, String>,
) -> TraitDefaultMethodCandidate {
    if candidates.len() == 1 {
        return candidates.into_iter().next().unwrap();
    }

    let canonical_matches = candidates
        .iter()
        .filter(|candidate| {
            canonical_names_by_id
                .get(&candidate.trait_id)
                .is_some_and(|canonical_name| canonical_name == &candidate.trait_name)
        })
        .cloned()
        .collect::<Vec<_>>();
    if canonical_matches.len() == 1 {
        return canonical_matches.into_iter().next().unwrap();
    }

    if let Some(candidate) = single_qualified_trait_default_candidate(&candidates) {
        return candidate.clone();
    }

    panic!(
        "duplicate HIR trait default method DefId {:?}: {:?}",
        method_id, candidates
    );
}

fn single_qualified_trait_default_candidate(
    candidates: &[TraitDefaultMethodCandidate],
) -> Option<&TraitDefaultMethodCandidate> {
    let first = candidates.first()?;
    if candidates.iter().any(|candidate| {
        candidate.trait_id != first.trait_id || candidate.method_name != first.method_name
    }) {
        return None;
    }

    let mut qualified_candidates = candidates
        .iter()
        .filter(|candidate| candidate.trait_name.contains("::"));
    let qualified_candidate = qualified_candidates.next()?;
    if qualified_candidates.next().is_none() {
        Some(qualified_candidate)
    } else {
        None
    }
}

fn index_owned_named_definitions<T, NameOf>(
    output: &mut HashMap<DefId, String>,
    definitions: &HashMap<DefId, T>,
    names_by_name: &HashMap<String, DefId>,
    name_of: NameOf,
    canonical_names_by_id: &HashMap<DefId, String>,
    kind: &str,
) where
    NameOf: Fn(&T) -> &str,
{
    for (id, definition) in definitions {
        let default_name = name_of(definition);
        let mut stored_names = names_by_name
            .iter()
            .filter_map(|(name, name_id)| (name_id == id).then(|| name.clone()))
            .collect::<Vec<_>>();
        stored_names.sort();
        stored_names.dedup();

        let mut names = stored_names.clone();
        if !names.iter().any(|name| name == default_name) {
            names.push(default_name.to_string());
        }
        names.sort();
        names.dedup();

        let selected = if let Some(canonical_name) = canonical_names_by_id.get(id) {
            if names.iter().any(|name| name == canonical_name) {
                canonical_name.clone()
            } else if let Some(short_canonical_name) = canonical_name.rsplit("::").next() {
                if names.iter().any(|name| name == short_canonical_name) {
                    short_canonical_name.to_string()
                } else if names.len() == 1 {
                    names[0].clone()
                } else if let Some(canonical_path_name) = single_canonical_path_name(&names) {
                    canonical_path_name.clone()
                } else if let Some(mangled_name) = single_mangled_name(&names) {
                    mangled_name.clone()
                } else {
                    panic!("duplicate HIR {} DefId {:?}: {:?}", kind, id, names);
                }
            } else if names.len() == 1 {
                names[0].clone()
            } else if let Some(canonical_path_name) = single_canonical_path_name(&names) {
                canonical_path_name.clone()
            } else {
                panic!("duplicate HIR {} DefId {:?}: {:?}", kind, id, names);
            }
        } else if names.len() == 1 {
            names[0].clone()
        } else if stored_names.len() == 1 {
            stored_names[0].clone()
        } else {
            panic!("duplicate HIR {} DefId {:?}: {:?}", kind, id, names);
        };

        insert_unique(output, *id, selected, kind);
    }
}

fn single_canonical_path_name(names: &[String]) -> Option<&String> {
    let mut canonical_path_names = names.iter().filter(|name| name.contains("::"));
    let canonical_path_name = canonical_path_names.next()?;
    if canonical_path_names.next().is_none() {
        Some(canonical_path_name)
    } else {
        None
    }
}

fn single_mangled_name(names: &[String]) -> Option<&String> {
    let mut mangled_names = names.iter().filter(|name| name.contains('_'));
    let mangled_name = mangled_names.next()?;
    if mangled_names.next().is_none() {
        Some(mangled_name)
    } else {
        None
    }
}

fn insert_unique<T>(map: &mut HashMap<DefId, T>, id: DefId, value: T, kind: &str) {
    assert!(
        map.insert(id, value).is_none(),
        "duplicate HIR {} DefId: {:?}",
        kind,
        id
    );
}

fn insert_unique_child<K, T>(map: &mut HashMap<K, T>, key: K, value: T, kind: &str)
where
    K: Copy + Eq + Hash + std::fmt::Debug,
    T: PartialEq,
{
    if let Some(existing) = map.get(&key) {
        assert!(
            existing == &value,
            "duplicate HIR {} identity: {:?}",
            kind,
            key
        );
        return;
    }

    map.insert(key, value);
}
