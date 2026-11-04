use super::type_table;
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};

use super::artifact_codec::{
    decode_product_artifact_payload, product_artifact_header_string_bytes,
    read_product_artifact_header,
};
use super::artifact_metadata::{
    ProductCrateIdentity, ProductDependencyIdentity, ProductFreshnessMetadata,
};
use super::interface_types::CompilerProducts;
use super::ProductCrateId;
use bincode::Options;
use serde::{Deserialize, Serialize};

pub const PRODUCT_ARTIFACT_FORMAT_VERSION: u32 = 51;
pub const PRODUCT_ARTIFACT_MAGIC: [u8; 8] = *b"ROCKRKCA";
pub const MAX_PRODUCT_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_PRODUCT_ARTIFACT_HEADER_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_PRODUCT_ARTIFACT_STRING_BYTES: usize = 1024 * 1024;
pub const MAX_PRODUCT_ARTIFACT_TOTAL_STRING_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PRODUCT_ARTIFACT_TYPE_ROWS: usize = 1_000_000;
pub const MAX_PRODUCT_ARTIFACT_DECLARATIONS: usize = 1_000_000;
pub const MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS: usize = 1_024;
pub const MAX_PRODUCT_ARTIFACT_TYPE_DEPTH: usize = 256;
pub const MAX_PRODUCT_ARTIFACT_NORMALIZATION_NODES: usize = 4_000_000;
pub(super) const PRODUCT_ARTIFACT_PREAMBLE_BYTES: u64 = 8 + 4 + 8 + 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductArtifactPreamble {
    pub magic: [u8; 8],
    pub format_version: u32,
    pub header_len: u64,
    pub payload_len: u64,
}

impl ProductArtifactPreamble {
    pub(super) fn encode(self) -> [u8; PRODUCT_ARTIFACT_PREAMBLE_BYTES as usize] {
        let mut bytes = [0; PRODUCT_ARTIFACT_PREAMBLE_BYTES as usize];
        bytes[..8].copy_from_slice(&self.magic);
        bytes[8..12].copy_from_slice(&self.format_version.to_le_bytes());
        bytes[12..20].copy_from_slice(&self.header_len.to_le_bytes());
        bytes[20..28].copy_from_slice(&self.payload_len.to_le_bytes());
        bytes
    }

    pub(super) fn decode(
        bytes: [u8; PRODUCT_ARTIFACT_PREAMBLE_BYTES as usize],
    ) -> Result<Self, String> {
        let preamble = Self {
            magic: bytes[..8].try_into().expect("fixed magic width"),
            format_version: u32::from_le_bytes(bytes[8..12].try_into().expect("fixed u32 width")),
            header_len: u64::from_le_bytes(bytes[12..20].try_into().expect("fixed u64 width")),
            payload_len: u64::from_le_bytes(bytes[20..28].try_into().expect("fixed u64 width")),
        };
        preamble.validate()?;
        Ok(preamble)
    }

    pub(super) fn validate(self) -> Result<(), String> {
        if self.magic != PRODUCT_ARTIFACT_MAGIC {
            return Err("Invalid product artifact magic".to_string());
        }
        if self.format_version != PRODUCT_ARTIFACT_FORMAT_VERSION {
            return Err(format!(
                "Unsupported product artifact format {} (expected {})",
                self.format_version, PRODUCT_ARTIFACT_FORMAT_VERSION
            ));
        }
        check_artifact_limit(
            "header bytes",
            self.header_len,
            MAX_PRODUCT_ARTIFACT_HEADER_BYTES,
        )?;
        let total = PRODUCT_ARTIFACT_PREAMBLE_BYTES
            .checked_add(self.header_len)
            .and_then(|size| size.checked_add(self.payload_len))
            .ok_or_else(|| "Product artifact declared size overflows u64".to_string())?;
        check_artifact_limit("artifact bytes", total, MAX_PRODUCT_ARTIFACT_BYTES)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductArtifactHeader {
    pub crate_identity: ProductCrateIdentity,
    pub freshness: ProductFreshnessMetadata,
    pub identity_dependencies: BTreeMap<ProductCrateId, ProductCrateIdentity>,
    pub dependencies: Vec<ProductDependencyIdentity>,
    #[serde(skip)]
    pub(super) header_len: u64,
    #[serde(skip)]
    pub(super) payload_len: u64,
}

impl ProductArtifactHeader {
    pub(super) fn from_products(products: &CompilerProducts) -> Self {
        Self {
            crate_identity: products.crate_identity.clone(),
            freshness: products.freshness_metadata(),
            identity_dependencies: products.identity_table.dependencies.clone(),
            dependencies: products.dependencies.clone(),
            header_len: 0,
            payload_len: 0,
        }
    }

    pub fn read_from_path_bounded(path: &std::path::Path) -> Result<Self, String> {
        let file_len = std::fs::metadata(path)
            .map_err(|error| {
                format!(
                    "Failed to inspect product artifact {}: {error}",
                    path.display()
                )
            })?
            .len();
        check_artifact_limit("artifact bytes", file_len, MAX_PRODUCT_ARTIFACT_BYTES)?;
        let mut file = std::fs::File::open(path).map_err(|error| {
            format!(
                "Failed to open product artifact {}: {error}",
                path.display()
            )
        })?;
        let (preamble, header) = read_product_artifact_header(&mut file, file_len)?;
        Ok(Self {
            header_len: preamble.header_len,
            payload_len: preamble.payload_len,
            ..header
        })
    }

    fn matches_products(&self, products: &CompilerProducts) -> bool {
        self.crate_identity == products.crate_identity
            && self.freshness == products.freshness_metadata()
            && self.identity_dependencies == products.identity_table.dependencies
            && self.dependencies == products.dependencies
    }
}

pub struct PortableProductArtifact {
    pub(super) header: ProductArtifactHeader,
    pub(super) artifact: type_table::SerializedProductArtifact,
}

impl PortableProductArtifact {
    pub fn read_payload_from_path_bounded(
        path: &std::path::Path,
        expected_header: &ProductArtifactHeader,
    ) -> Result<Self, String> {
        let file_len = std::fs::metadata(path)
            .map_err(|error| {
                format!(
                    "Failed to inspect product artifact {}: {error}",
                    path.display()
                )
            })?
            .len();
        check_artifact_limit("artifact bytes", file_len, MAX_PRODUCT_ARTIFACT_BYTES)?;
        let mut file = std::fs::File::open(path).map_err(|error| {
            format!(
                "Failed to open product artifact {}: {error}",
                path.display()
            )
        })?;
        let (preamble, header) = read_product_artifact_header(&mut file, file_len)?;
        if header.crate_identity != expected_header.crate_identity
            || header.freshness != expected_header.freshness
            || header.identity_dependencies != expected_header.identity_dependencies
            || header.dependencies != expected_header.dependencies
            || preamble.header_len != expected_header.header_len
            || preamble.payload_len != expected_header.payload_len
        {
            return Err("Product artifact header changed between staged reads".to_string());
        }
        file.seek(SeekFrom::Start(
            PRODUCT_ARTIFACT_PREAMBLE_BYTES + preamble.header_len,
        ))
        .map_err(|error| format!("Failed to seek product artifact payload: {error}"))?;
        let payload_len = usize::try_from(preamble.payload_len)
            .map_err(|_| "Product artifact payload length does not fit usize".to_string())?;
        let mut payload = vec![0; payload_len];
        file.read_exact(&mut payload)
            .map_err(|error| format!("Failed to read product artifact payload: {error}"))?;
        let artifact = decode_product_artifact_payload(&payload)?;
        let header_string_bytes = product_artifact_header_string_bytes(&header)?;
        type_table::validate_portable_artifact_limits_with_initial_string_bytes(
            &artifact,
            header_string_bytes,
        )?;
        Ok(Self {
            header: expected_header.clone(),
            artifact,
        })
    }

    pub fn into_products(self) -> Result<CompilerProducts, String> {
        let products = type_table::artifact_to_products(self.artifact)?;
        if !self.header.matches_products(&products) {
            return Err("Product artifact header does not match semantic payload".to_string());
        }
        Ok(products)
    }
}

pub(super) fn check_artifact_limit(
    category: &str,
    declared: u64,
    maximum: u64,
) -> Result<(), String> {
    if declared > maximum {
        return Err(format!(
            "Product artifact {category} limit exceeded: declared {declared}, maximum {maximum}"
        ));
    }
    Ok(())
}

pub(super) fn artifact_bincode() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
}

#[cfg(test)]
mod tests;
