use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::{ProductCrateId, ProductDefId};
use crate::hir::{AcceptedHir, AcceptedHirFunction, HirImplFor};
use crate::language_items::LanguageItems;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProductCrateIdentity {
    pub name: String,
    pub version: String,
    pub target_triple: Option<String>,
    pub format_version: u32,
    pub source_fingerprint: ProductSourceFingerprint,
}

impl ProductCrateIdentity {
    pub fn local(name: String) -> Self {
        Self {
            name,
            version: "0.1.0".to_string(),
            target_triple: None,
            format_version: 1,
            source_fingerprint: ProductSourceFingerprint::default(),
        }
    }

    pub fn with_source_fingerprint(mut self, source_fingerprint: ProductSourceFingerprint) -> Self {
        self.source_fingerprint = source_fingerprint;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProductIdentityTable {
    pub local_crate: Option<ProductCrateId>,
    pub dependencies: BTreeMap<ProductCrateId, ProductCrateIdentity>,
    pub display_names: BTreeMap<ProductDefId, String>,
    pub export_names: BTreeMap<String, ProductDefId>,
    /// Ordinary public variant paths, distinct from exports of their enum.
    #[serde(default)]
    pub variant_export_names: BTreeMap<String, (ProductDefId, String)>,
    #[serde(default)]
    pub import_alias_names: BTreeMap<String, ProductDefId>,
    #[serde(default)]
    pub module_alias_names: BTreeMap<String, ProductDefId>,
    #[serde(default)]
    pub prelude_export_names: BTreeMap<String, ProductDefId>,
    /// Prelude names that are enum variants: the enum and the variant name.
    #[serde(default)]
    pub prelude_variant_export_names: BTreeMap<String, (ProductDefId, String)>,
    #[serde(default)]
    pub ambiguous_export_names: BTreeSet<String>,
}

pub type ProductLanguageItems = LanguageItems<ProductDefId>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProductBodies {
    pub functions: BTreeMap<ProductDefId, AcceptedHirFunction>,
    pub generic_impls: BTreeMap<ProductDefId, HirImplFor<AcceptedHir>>,
    pub trait_default_methods: BTreeMap<ProductDefId, AcceptedHirFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProductLinkData {
    pub object_path: Option<PathBuf>,
    pub records: BTreeMap<ProductDefId, ProductLinkRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductLinkRecord {
    pub backend_symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductDependencyIdentity {
    pub name: String,
    pub artifact_path: PathBuf,
    pub capabilities: ProductDependencyCapabilities,
}

impl ProductDependencyIdentity {
    pub fn artifact_object(name: String, artifact_path: PathBuf) -> Self {
        Self {
            name,
            artifact_path,
            capabilities: ProductDependencyCapabilities::artifact_object(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductFreshnessMetadata {
    pub compiler_version: String,
    pub artifact_format_version: u32,
    pub target_triple: Option<String>,
    pub config_hash: Option<String>,
    pub features: BTreeSet<String>,
    pub source_fingerprint: ProductSourceFingerprint,
    pub dependencies: Vec<ProductDependencyFreshness>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductDependencyFreshness {
    pub name: String,
    pub artifact_path: PathBuf,
    pub crate_identity: Option<ProductCrateIdentity>,
    pub capabilities: ProductDependencyCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProductDependencyCapabilities {
    pub metadata: bool,
    pub bodies: bool,
    pub link: ProductDependencyLinkCapability,
}

impl ProductDependencyCapabilities {
    pub fn artifact_object() -> Self {
        Self {
            metadata: true,
            bodies: true,
            link: ProductDependencyLinkCapability::Object,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ProductDependencyLinkCapability {
    Object,
    MetadataOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub struct ProductSourceFingerprint {
    pub manifest_hash: Option<String>,
    pub source_hash: Option<String>,
    pub loaded_files: Vec<PathBuf>,
}
