mod artifact_codec;
mod artifact_format;
mod artifact_io;
mod artifact_metadata;
mod body_type_remap;
mod from_resolved_hir;
mod id_validation;
mod identity_recording;
mod interface_types;
mod language_items_build;
mod location_id_remap;
mod owned_type_id_remap;
mod prelude_export_ids;
mod type_id_remap;

use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{CrateId, DefId, Idx, LocalDefId};
use serde::{Deserialize, Serialize};

use artifact_codec::check_product_artifact_string;
#[cfg(test)]
pub(crate) use artifact_format::artifact_type_table_len_for_test;
pub use artifact_format::{
    PortableProductArtifact, ProductArtifactHeader, ProductArtifactPreamble,
    MAX_PRODUCT_ARTIFACT_BYTES, MAX_PRODUCT_ARTIFACT_DECLARATIONS,
    MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS, MAX_PRODUCT_ARTIFACT_HEADER_BYTES,
    MAX_PRODUCT_ARTIFACT_NORMALIZATION_NODES, MAX_PRODUCT_ARTIFACT_STRING_BYTES,
    MAX_PRODUCT_ARTIFACT_TOTAL_STRING_BYTES, MAX_PRODUCT_ARTIFACT_TYPE_DEPTH,
    MAX_PRODUCT_ARTIFACT_TYPE_ROWS, PRODUCT_ARTIFACT_FORMAT_VERSION, PRODUCT_ARTIFACT_MAGIC,
};
pub use artifact_metadata::{
    ProductBodies, ProductCrateIdentity, ProductDependencyCapabilities, ProductDependencyFreshness,
    ProductDependencyIdentity, ProductDependencyLinkCapability, ProductFreshnessMetadata,
    ProductIdentityTable, ProductLanguageItems, ProductLinkData, ProductLinkRecord,
    ProductSourceFingerprint,
};
pub use interface_types::{
    CompilerProducts, ProductAssociatedTypeInterface, ProductEnumInterface,
    ProductEnumVariantInterface, ProductExternInterface, ProductFieldInterface,
    ProductFunctionInterface, ProductImplInterface, ProductInterface, ProductMacroExport,
    ProductStructInterface, ProductTraitInterface, ProductTypeAliasInterface,
};

#[cfg(test)]
use artifact_codec::{
    encode_product_artifact, read_product_artifact_header, serialized_artifact_from_bytes_for_test,
};
#[cfg(test)]
use identity_recording::{
    product_def_id_to_def_id, record_export_name, PreferredTraitDefaultMethods,
};
#[cfg(test)]
use language_items_build::product_language_items_from_program;
#[cfg(test)]
use location_id_remap::remap_expr_location_product_ids;
#[cfg(test)]
use owned_type_id_remap::remap_pattern_owned_type_ids;

pub(crate) mod fingerprint;
pub(crate) mod identity;
pub(crate) mod link;
mod type_table;

pub use crate::infer::constants::{ArithmeticSummary, ArithmeticType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProductCrateId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProductLocalDefId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProductDefId {
    pub crate_id: ProductCrateId,
    pub local_id: ProductLocalDefId,
}

pub(crate) type ProductIdRemap = BTreeMap<ProductDefId, BTreeSet<ProductDefId>>;

impl From<CrateId> for ProductCrateId {
    fn from(value: CrateId) -> Self {
        Self(value.raw())
    }
}

impl From<LocalDefId> for ProductLocalDefId {
    fn from(value: LocalDefId) -> Self {
        Self(value.raw())
    }
}

impl From<DefId> for ProductDefId {
    fn from(value: DefId) -> Self {
        Self {
            crate_id: ProductCrateId::from(value.crate_id),
            local_id: ProductLocalDefId::from(value.local),
        }
    }
}

#[cfg(test)]
mod tests;
