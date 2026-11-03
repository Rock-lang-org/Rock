use crate::type_services::normalize::{TypeNormalizationEnv, TypeNormalizer};

use super::kinds::constructor_kind;

pub(crate) fn register_type_aliases<'a>(
    env: &mut TypeNormalizationEnv,
    aliases: impl IntoIterator<Item = &'a crate::hir::HirTypeAlias>,
) {
    let aliases = aliases.into_iter().collect::<Vec<_>>();
    for alias in &aliases {
        env.register_alias(
            alias.id,
            constructor_kind(&alias.generic_params),
            alias.ty.clone(),
        );
        for param in &alias.generic_params {
            env.register_generic_kind(param.id, param.kind.clone());
        }
    }

    // Alias bodies may themselves be constructors. Iterate so alias chains can
    // acquire their body-derived kinds without declaration-order dependence.
    for _ in 0..aliases.len() {
        let derived = aliases
            .iter()
            .filter_map(|alias| {
                TypeNormalizer::new(env)
                    .kind_of(&alias.ty)
                    .ok()
                    .map(|kind| (alias.id, kind, alias.ty.clone()))
            })
            .collect::<Vec<_>>();
        for (id, kind, body) in derived {
            env.register_alias(id, kind, body);
        }
    }
}
