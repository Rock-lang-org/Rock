use super::type_table;
use std::io::{Read, Seek};

use super::artifact_format::{
    artifact_bincode, check_artifact_limit, PortableProductArtifact, ProductArtifactHeader,
    ProductArtifactPreamble, MAX_PRODUCT_ARTIFACT_BYTES, MAX_PRODUCT_ARTIFACT_DECLARATIONS,
    MAX_PRODUCT_ARTIFACT_HEADER_BYTES, MAX_PRODUCT_ARTIFACT_STRING_BYTES,
    MAX_PRODUCT_ARTIFACT_TOTAL_STRING_BYTES, PRODUCT_ARTIFACT_FORMAT_VERSION,
    PRODUCT_ARTIFACT_MAGIC, PRODUCT_ARTIFACT_PREAMBLE_BYTES,
};
use super::artifact_metadata::ProductCrateIdentity;
use bincode::Options;

pub(super) fn read_product_artifact_header(
    reader: &mut (impl Read + Seek),
    file_len: u64,
) -> Result<(ProductArtifactPreamble, ProductArtifactHeader), String> {
    if file_len < PRODUCT_ARTIFACT_PREAMBLE_BYTES {
        return Err(format!(
            "Product artifact is shorter than the {}-byte preamble",
            PRODUCT_ARTIFACT_PREAMBLE_BYTES
        ));
    }
    let mut preamble_bytes = [0; PRODUCT_ARTIFACT_PREAMBLE_BYTES as usize];
    reader
        .read_exact(&mut preamble_bytes)
        .map_err(|error| format!("Failed to read product artifact preamble: {error}"))?;
    let preamble = ProductArtifactPreamble::decode(preamble_bytes)?;
    let declared_len = PRODUCT_ARTIFACT_PREAMBLE_BYTES
        .checked_add(preamble.header_len)
        .and_then(|size| size.checked_add(preamble.payload_len))
        .ok_or_else(|| "Product artifact declared size overflows u64".to_string())?;
    if declared_len != file_len {
        return Err(format!(
            "Product artifact size mismatch: declared {declared_len}, actual {file_len}"
        ));
    }
    let header_len = usize::try_from(preamble.header_len)
        .map_err(|_| "Product artifact header length does not fit usize".to_string())?;
    let mut header_bytes = vec![0; header_len];
    reader
        .read_exact(&mut header_bytes)
        .map_err(|error| format!("Failed to read product artifact header: {error}"))?;
    let mut header: ProductArtifactHeader = artifact_bincode()
        .with_limit(MAX_PRODUCT_ARTIFACT_HEADER_BYTES)
        .deserialize(&header_bytes)
        .map_err(|error| format!("Failed to deserialize product artifact header: {error}"))?;
    validate_product_artifact_header(&header)?;
    header.header_len = preamble.header_len;
    header.payload_len = preamble.payload_len;
    Ok((preamble, header))
}

fn validate_product_artifact_header(header: &ProductArtifactHeader) -> Result<(), String> {
    if header.dependencies.len() > MAX_PRODUCT_ARTIFACT_DECLARATIONS
        || header.identity_dependencies.len() > MAX_PRODUCT_ARTIFACT_DECLARATIONS
        || header.freshness.dependencies.len() > MAX_PRODUCT_ARTIFACT_DECLARATIONS
    {
        return Err(format!(
            "Product artifact declaration count limit exceeded: maximum {}",
            MAX_PRODUCT_ARTIFACT_DECLARATIONS
        ));
    }

    product_artifact_header_string_bytes(header).map(|_| ())
}

pub(super) fn product_artifact_header_string_bytes(
    header: &ProductArtifactHeader,
) -> Result<usize, String> {
    let mut total = 0usize;
    check_product_identity_strings(&header.crate_identity, &mut total)?;
    for identity in header.identity_dependencies.values() {
        check_product_identity_strings(identity, &mut total)?;
    }
    check_product_artifact_string(&header.freshness.compiler_version, &mut total)?;
    if let Some(target) = &header.freshness.target_triple {
        check_product_artifact_string(target, &mut total)?;
    }
    if let Some(hash) = &header.freshness.config_hash {
        check_product_artifact_string(hash, &mut total)?;
    }
    for feature in &header.freshness.features {
        check_product_artifact_string(feature, &mut total)?;
    }
    for dependency in &header.dependencies {
        check_product_artifact_string(&dependency.name, &mut total)?;
        check_product_artifact_string(&dependency.artifact_path.to_string_lossy(), &mut total)?;
    }
    for dependency in &header.freshness.dependencies {
        check_product_artifact_string(&dependency.name, &mut total)?;
        check_product_artifact_string(&dependency.artifact_path.to_string_lossy(), &mut total)?;
        if let Some(identity) = &dependency.crate_identity {
            check_product_identity_strings(identity, &mut total)?;
        }
    }
    Ok(total)
}

pub(super) fn check_product_artifact_string(value: &str, total: &mut usize) -> Result<(), String> {
    if value.len() > MAX_PRODUCT_ARTIFACT_STRING_BYTES {
        return Err(format!(
            "Product artifact string bytes limit exceeded: declared {}, maximum {}",
            value.len(),
            MAX_PRODUCT_ARTIFACT_STRING_BYTES
        ));
    }
    *total = total
        .checked_add(value.len())
        .ok_or_else(|| "Product artifact total string bytes overflow".to_string())?;
    if *total > MAX_PRODUCT_ARTIFACT_TOTAL_STRING_BYTES {
        return Err(format!(
            "Product artifact total string bytes limit exceeded: declared {total}, maximum {}",
            MAX_PRODUCT_ARTIFACT_TOTAL_STRING_BYTES
        ));
    }
    Ok(())
}

fn check_product_identity_strings(
    identity: &ProductCrateIdentity,
    total: &mut usize,
) -> Result<(), String> {
    check_product_artifact_string(&identity.name, total)?;
    check_product_artifact_string(&identity.version, total)?;
    if let Some(target) = &identity.target_triple {
        check_product_artifact_string(target, total)?;
    }
    if let Some(hash) = &identity.source_fingerprint.manifest_hash {
        check_product_artifact_string(hash, total)?;
    }
    if let Some(hash) = &identity.source_fingerprint.source_hash {
        check_product_artifact_string(hash, total)?;
    }
    for path in &identity.source_fingerprint.loaded_files {
        check_product_artifact_string(&path.to_string_lossy(), total)?;
    }
    Ok(())
}

pub(super) fn decode_product_artifact_payload(
    payload: &[u8],
) -> Result<type_table::SerializedProductArtifact, String> {
    artifact_bincode()
        .with_limit(MAX_PRODUCT_ARTIFACT_BYTES)
        .deserialize(payload)
        .map_err(|error| format!("Failed to deserialize product artifact payload: {error}"))
}

pub(super) fn encode_product_artifact(
    header: &ProductArtifactHeader,
    artifact: &type_table::SerializedProductArtifact,
) -> Result<Vec<u8>, String> {
    validate_product_artifact_header(header)?;
    let header_string_bytes = product_artifact_header_string_bytes(header)?;
    type_table::validate_portable_artifact_limits_with_initial_string_bytes(
        artifact,
        header_string_bytes,
    )?;
    let payload = artifact_bincode()
        .serialize(artifact)
        .map_err(|error| format!("Failed to serialize product artifact payload: {error}"))?;
    let header = artifact_bincode()
        .serialize(header)
        .map_err(|error| format!("Failed to serialize product artifact header: {error}"))?;
    check_artifact_limit(
        "header bytes",
        header.len() as u64,
        MAX_PRODUCT_ARTIFACT_HEADER_BYTES,
    )?;
    let preamble = ProductArtifactPreamble {
        magic: PRODUCT_ARTIFACT_MAGIC,
        format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
        header_len: header.len() as u64,
        payload_len: payload.len() as u64,
    };
    preamble.validate()?;
    let capacity = usize::try_from(
        PRODUCT_ARTIFACT_PREAMBLE_BYTES + preamble.header_len + preamble.payload_len,
    )
    .map_err(|_| "Product artifact size does not fit usize".to_string())?;
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(&preamble.encode());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

pub(super) fn portable_product_artifact_from_bytes(
    bytes: &[u8],
) -> Result<PortableProductArtifact, String> {
    check_artifact_limit(
        "artifact bytes",
        bytes.len() as u64,
        MAX_PRODUCT_ARTIFACT_BYTES,
    )?;
    let mut cursor = std::io::Cursor::new(bytes);
    let (preamble, header) = read_product_artifact_header(&mut cursor, bytes.len() as u64)?;
    let payload_start = usize::try_from(PRODUCT_ARTIFACT_PREAMBLE_BYTES + preamble.header_len)
        .map_err(|_| "Product artifact payload offset does not fit usize".to_string())?;
    let artifact = decode_product_artifact_payload(&bytes[payload_start..])?;
    let header_string_bytes = product_artifact_header_string_bytes(&header)?;
    type_table::validate_portable_artifact_limits_with_initial_string_bytes(
        &artifact,
        header_string_bytes,
    )?;
    Ok(PortableProductArtifact { header, artifact })
}

#[cfg(test)]
pub(crate) mod tests;
