//! Crate system for external crate loading and package management.

mod context;
mod extern_store;
mod manifest;
mod module_tree;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use crate::products::ProductCrateIdentity;

pub use rock_shared::manifest::{CrateConfig, CrateManifest, CrateTarget, Dependency, TargetRole};

#[allow(unused_imports)]
pub(crate) use extern_store::{
    CurrentCrateSource, DependencyLinkInputs, ExternCrateBodies, ExternCrateLink,
    ExternCrateLinkage, ExternCrateMetadata, ExternCrateRecord, ExternCrateRef, ExternCrateStore,
};

/// A hierarchical tree of modules within a crate
#[derive(Debug, Clone)]
pub struct ModuleTree {
    /// Map of qualified name to module node (e.g., "collections::hashmap" -> node)
    pub modules: BTreeMap<String, ModuleNode>,
}

/// A node in the module tree
#[derive(Debug, Clone)]
pub struct ModuleNode {
    pub name: String,
    /// Fully qualified name (e.g., "std::collections")
    pub qualified_name: String,
    /// Names of submodules
    pub submodules: Vec<String>,
}

/// Context for managing loaded crates
#[derive(Debug, Clone)]
pub struct CrateContext {
    source_crates: BTreeMap<String, CurrentCrateSource>,
    extern_crates: ExternCrateStore,
    pub(crate) product_crate_ids: BTreeMap<ProductCrateIdentity, crate::ids::CrateId>,
    next_product_crate_id: u32,
}
