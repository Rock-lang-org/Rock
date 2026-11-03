#![allow(dead_code)]

mod collection_codec;
mod expr_conversion;
mod generic_conversion;
mod hir_body_conversion;
mod hir_item_conversion;
mod hir_schema;
mod interface_conversion;
mod product_schema;
mod products_conversion;
mod string_limits;
#[cfg(test)]
mod test_support;
mod type_codec;
mod type_graph_validation;
mod type_rows;

use crate::products::CompilerProducts;
use product_schema::SerializedCompilerProducts;
use string_limits::validate_portable_artifact_limits;
use type_codec::{ProductTypeDecoder, ProductTypeEncoder};

pub(super) use product_schema::SerializedProductArtifact;
pub(super) use string_limits::validate_portable_artifact_limits_with_initial_string_bytes;

#[cfg(test)]
use generic_conversion::{decode_generic_param_decls, encode_generic_param_decls};
#[cfg(test)]
use hir_schema::{SerializedHirExpr, SerializedHirExprKind};
#[cfg(test)]
pub(super) use test_support::{
    artifact_type_table_contains_type_for_test, artifact_type_table_len_for_test,
};
#[cfg(test)]
use test_support::{decode_type_for_test, encode_types_for_test};
#[cfg(test)]
use type_codec::product_def_id;
#[cfg(test)]
use type_graph_validation::validate_type_table_graph;
#[cfg(test)]
use type_rows::{ProductTypeId, ProductTypeRow, ProductTypeTable};

pub(super) fn products_to_artifact(
    products: &CompilerProducts,
    format_version: u32,
) -> Result<SerializedProductArtifact, String> {
    let mut encoder = ProductTypeEncoder::new();
    let products = SerializedCompilerProducts::encode(products, &mut encoder)?;
    let type_table = encoder.finish();

    Ok(SerializedProductArtifact {
        format_version,
        type_table,
        products,
    })
}

pub(super) fn artifact_to_products(
    artifact: SerializedProductArtifact,
) -> Result<CompilerProducts, String> {
    validate_portable_artifact_limits(&artifact)?;
    let mut decoder = ProductTypeDecoder::new(&artifact.type_table);
    artifact.products.decode(&mut decoder)
}

#[cfg(test)]
mod tests;
