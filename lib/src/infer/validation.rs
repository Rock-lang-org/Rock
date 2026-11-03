use std::collections::HashMap;

use crate::ids::{CrateId, DefId, Idx};
use crate::lower::ResolveError;

use super::PartialHir;

impl PartialHir {
    pub(super) fn validate_item_ids(&self) -> Result<(), Vec<ResolveError>> {
        let mut mismatches = Vec::new();
        validate_item_ids(
            &self.functions,
            |function| function.id,
            "function declaration",
            &mut mismatches,
        );
        validate_item_ids(
            &self.structs,
            |structure| structure.id,
            "struct declaration",
            &mut mismatches,
        );
        validate_item_ids(
            &self.enums,
            |enumeration| enumeration.id,
            "enum declaration",
            &mut mismatches,
        );
        validate_item_ids(
            &self.traits,
            |trait_def| trait_def.id,
            "trait declaration",
            &mut mismatches,
        );
        validate_item_ids(
            &self.impls,
            |imp| imp.id,
            "impl declaration",
            &mut mismatches,
        );
        validate_item_ids(
            &self.externs,
            |extern_| extern_.id,
            "extern declaration",
            &mut mismatches,
        );
        validate_item_ids(
            &self.type_aliases,
            |alias| alias.id,
            "type alias declaration",
            &mut mismatches,
        );

        if mismatches.is_empty() {
            Ok(())
        } else {
            mismatches.sort_by(|left, right| {
                left.key
                    .cmp(&right.key)
                    .then_with(|| left.category.cmp(right.category))
            });
            Err(mismatches
                .into_iter()
                .map(|mismatch| {
                    ResolveError::non_source(format!(
                        "{} ID mismatch in compiler state",
                        mismatch.category
                    ))
                })
                .collect())
        }
    }
}

pub(super) struct ItemIdMismatch {
    key: DefId,
    category: &'static str,
}

pub(super) fn validate_item_ids<T>(
    items: &HashMap<DefId, T>,
    id_of: impl Fn(&T) -> DefId,
    category: &'static str,
    mismatches: &mut Vec<ItemIdMismatch>,
) {
    for (key, item) in items {
        let payload_id = id_of(item);
        if *key != payload_id {
            mismatches.push(ItemIdMismatch {
                key: *key,
                category,
            });
        }
    }
}

pub(super) fn validate_method_ids(hir: &PartialHir) -> Result<(), Vec<ResolveError>> {
    let mut errors = Vec::new();
    let mut seen = HashMap::<DefId, String>::new();

    let mut record = |id: DefId, owner: String, errors: &mut Vec<ResolveError>| {
        if id.crate_id == CrateId(u32::MAX) {
            errors.push(ResolveError::non_source(format!(
                "provisional method identity is not allowed for {owner}"
            )));
            return;
        }

        if let Some(previous_owner) = seen.get(&id) {
            if previous_owner != &owner {
                errors.push(ResolveError::non_source(format!(
                    "duplicate method identity {:?} used by {} and {}",
                    id, previous_owner, owner
                )));
            }
        } else {
            seen.insert(id, owner);
        }
    };

    let mut trait_ids = hir.traits.keys().copied().collect::<Vec<_>>();
    trait_ids.sort();
    for id in trait_ids {
        let trait_def = &hir.traits[&id];
        let mut methods = trait_def
            .methods
            .iter()
            .map(|(name, method)| (method.id, name))
            .collect::<Vec<_>>();
        methods.sort_by_key(|(id, _)| *id);
        for (_, method_name) in methods {
            let method = &trait_def.methods[method_name];
            record(
                method.id,
                format!("trait {}.{}", trait_def.id.local.raw(), method_name),
                &mut errors,
            );
        }
        let mut signatures = trait_def
            .signatures
            .iter()
            .map(|(name, signature)| (signature.id, name))
            .collect::<Vec<_>>();
        signatures.sort_by_key(|(id, _)| *id);
        for (_, signature_name) in signatures {
            let signature = &trait_def.signatures[signature_name];
            record(
                signature.id,
                format!("trait {}.{}", trait_def.id.local.raw(), signature_name),
                &mut errors,
            );
        }
    }

    let mut impl_ids = hir.impls.keys().copied().collect::<Vec<_>>();
    impl_ids.sort();
    for id in impl_ids {
        let imp = &hir.impls[&id];
        let mut methods = imp
            .methods
            .iter()
            .map(|(name, method)| (method.id, name))
            .collect::<Vec<_>>();
        methods.sort_by_key(|(id, _)| *id);
        for (_, method_name) in methods {
            let method = &imp.methods[method_name];
            record(
                method.id,
                format!("impl {}.{}", imp.id.local.raw(), method_name),
                &mut errors,
            );
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
