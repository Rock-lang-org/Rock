mod from_products;
mod index_mut_impls;
mod program_rows;
mod validation;

use crate::ids::{CrateId, DefId, LocalDefId};
use crate::products::ProductDefId;

pub(super) use from_products::language_items_from_products;

fn product_def_id(id: ProductDefId) -> DefId {
    DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0))
}

#[cfg(test)]
mod tests;
