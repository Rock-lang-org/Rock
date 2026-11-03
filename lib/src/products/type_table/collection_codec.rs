use std::collections::{BTreeMap, HashMap};

use super::hir_schema::{SerializedHirExpr, SerializedHirPattern};
use super::type_codec::{ProductTypeDecoder, ProductTypeEncoder};
use crate::products::ProductDefId;

pub(super) fn encode_map<T, U>(
    map: &BTreeMap<ProductDefId, T>,
    encoder: &mut ProductTypeEncoder,
    encode: fn(&T, &mut ProductTypeEncoder) -> Result<U, String>,
) -> Result<BTreeMap<ProductDefId, U>, String> {
    map.iter()
        .map(|(id, value)| Ok((*id, encode(value, encoder)?)))
        .collect()
}

pub(super) fn decode_map<T, U>(
    map: BTreeMap<ProductDefId, T>,
    decoder: &mut ProductTypeDecoder<'_>,
    decode: fn(T, &mut ProductTypeDecoder<'_>) -> Result<U, String>,
) -> Result<BTreeMap<ProductDefId, U>, String> {
    map.into_iter()
        .map(|(id, value)| Ok((id, decode(value, decoder)?)))
        .collect()
}

pub(super) fn encode_exprs(
    values: &[crate::hir::AcceptedHirExpr],
    encoder: &mut ProductTypeEncoder,
) -> Result<Vec<SerializedHirExpr>, String> {
    values
        .iter()
        .map(|value| SerializedHirExpr::encode(value, encoder))
        .collect()
}

pub(super) fn decode_exprs(
    values: Vec<SerializedHirExpr>,
    decoder: &mut ProductTypeDecoder<'_>,
) -> Result<Vec<crate::hir::AcceptedHirExpr>, String> {
    values
        .into_iter()
        .map(|value| value.decode(decoder))
        .collect()
}

pub(super) fn encode_patterns(
    values: &[crate::hir::HirPattern],
    encoder: &mut ProductTypeEncoder,
) -> Result<Vec<SerializedHirPattern>, String> {
    values
        .iter()
        .map(|value| SerializedHirPattern::encode(value, encoder))
        .collect()
}

pub(super) fn decode_patterns(
    values: Vec<SerializedHirPattern>,
    decoder: &mut ProductTypeDecoder<'_>,
) -> Result<Vec<crate::hir::HirPattern>, String> {
    values
        .into_iter()
        .map(|value| value.decode(decoder))
        .collect()
}

pub(super) fn encode_hash_map<T, U>(
    map: &HashMap<String, T>,
    encoder: &mut ProductTypeEncoder,
    encode: fn(&T, &mut ProductTypeEncoder) -> Result<U, String>,
) -> Result<HashMap<String, U>, String> {
    map.iter()
        .map(|(name, value)| Ok((name.clone(), encode(value, encoder)?)))
        .collect()
}

pub(super) fn decode_hash_map<T, U>(
    map: HashMap<String, T>,
    decoder: &mut ProductTypeDecoder<'_>,
    decode: fn(T, &mut ProductTypeDecoder<'_>) -> Result<U, String>,
) -> Result<HashMap<String, U>, String> {
    map.into_iter()
        .map(|(name, value)| Ok((name, decode(value, decoder)?)))
        .collect()
}
