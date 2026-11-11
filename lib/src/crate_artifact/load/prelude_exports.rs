use std::collections::BTreeMap;

use super::product_names::{
    product_def_id_to_prelude_export_def_id, product_display_name, qualify_product_name,
};
use super::ProductIdentityRemap;
use crate::crate_artifact::{ArtifactCrateInterface, ArtifactExport};
use crate::products::CompilerProducts;

pub(super) fn normalize_product_prelude_exports(
    products: &CompilerProducts,
    remap: &ProductIdentityRemap,
    interface: &ArtifactCrateInterface,
    crate_name: &str,
    prelude_prefix: &str,
) -> Result<BTreeMap<String, ArtifactExport>, String> {
    let mut normalized_ids = BTreeMap::new();

    for (alias, id) in &products.identity_table.prelude_export_names {
        let display_name = product_display_name(products, *id).ok_or_else(|| {
            format!(
                "Product artifact prelude export '{}' has no display name or canonical DefId",
                alias
            )
        })?;
        let def_id =
            product_def_id_to_prelude_export_def_id(products, remap, *id)?.ok_or_else(|| {
                format!(
                    "Product artifact prelude export '{}' has no canonical prelude item ID",
                    alias
                )
            })?;
        let source = qualify_product_name(crate_name, &display_name);
        normalized_ids.insert(alias.clone(), ArtifactExport { source, id: def_id });
    }

    // A variant export keeps its enum's identity; the source path continues
    // to the variant so consumers can tell it from an export of the enum.
    for (alias, (id, variant)) in &products.identity_table.prelude_variant_export_names {
        let display_name = product_display_name(products, *id).ok_or_else(|| {
            format!("Product artifact prelude variant export '{alias}' has no enum display name")
        })?;
        let def_id =
            product_def_id_to_prelude_export_def_id(products, remap, *id)?.ok_or_else(|| {
                format!("Product artifact prelude variant export '{alias}' has no canonical enum ID")
            })?;
        let source = format!("{}::{variant}", qualify_product_name(crate_name, &display_name));
        normalized_ids.insert(alias.clone(), ArtifactExport { source, id: def_id });
    }

    for name in product_interface_prelude_names(interface, prelude_prefix) {
        if let Some(alias) = name.strip_prefix(&format!("{}::", prelude_prefix)) {
            if !normalized_ids.contains_key(alias) {
                let export = artifact_export_for_source(interface, &name).ok_or_else(|| {
                    format!(
                        "Product artifact prelude export '{}' references '{}' without a canonical DefId",
                        alias, name
                    )
                })?;
                normalized_ids.insert(alias.to_string(), export);
            }
        }
    }

    Ok(normalized_ids)
}

fn artifact_export_for_source(
    interface: &ArtifactCrateInterface,
    source: &str,
) -> Option<ArtifactExport> {
    interface.export_for_source(source)
}

fn product_interface_prelude_names(
    interface: &ArtifactCrateInterface,
    prelude_prefix: &str,
) -> Vec<String> {
    interface.prelude_source_names(prelude_prefix)
}
