use super::type_codec::{ProductTypeDecoder, ProductTypeEncoder};
use super::type_rows::{ProductTypeId, ProductTypeTable};
use crate::types::Type;

#[cfg(test)]
pub(in crate::products) fn encode_types_for_test(
    types: &[Type],
) -> Result<(ProductTypeTable, Vec<ProductTypeId>), String> {
    let mut encoder = ProductTypeEncoder::new();
    let ids = encoder.encode_types(types)?;
    Ok((encoder.finish(), ids))
}

#[cfg(test)]
pub(in crate::products) fn decode_type_for_test(
    table: &ProductTypeTable,
    id: ProductTypeId,
) -> Result<Type, String> {
    ProductTypeDecoder::new(table).decode_type(id)
}

#[cfg(test)]
pub(in crate::products) fn artifact_type_table_len_for_test(bytes: &[u8]) -> Result<usize, String> {
    let artifact = crate::products::serialized_artifact_from_bytes_for_test(bytes)?;
    Ok(artifact.type_table.rows.len())
}

#[cfg(test)]
pub(in crate::products) fn artifact_type_table_contains_type_for_test(
    bytes: &[u8],
    expected: &Type,
) -> Result<bool, String> {
    let artifact = crate::products::serialized_artifact_from_bytes_for_test(bytes)?;
    let mut decoder = ProductTypeDecoder::new(&artifact.type_table);

    for index in 0..artifact.type_table.rows.len() {
        if decoder.decode_type(ProductTypeId(index as u32))? == *expected {
            return Ok(true);
        }
    }

    Ok(false)
}
