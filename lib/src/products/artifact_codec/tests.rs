use super::*;

pub(crate) fn serialized_artifact_from_bytes_for_test(
    bytes: &[u8],
) -> Result<type_table::SerializedProductArtifact, String> {
    Ok(portable_product_artifact_from_bytes(bytes)?.artifact)
}
