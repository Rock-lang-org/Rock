use super::hir_schema::{SerializedGenericBounds, SerializedPredicate, SerializedTraitBound};
use super::type_codec::{def_id, product_def_id, ProductTypeDecoder, ProductTypeEncoder};
use super::type_rows::{ProductGenericParamDecl, ProductGenericParamId};
use crate::types::{GenericParamDecl, GenericParamId};

impl SerializedTraitBound {
    fn encode(
        value: &crate::types::TraitBound,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            trait_id: product_def_id(value.trait_id),
            type_args: encoder.encode_types(&value.type_args)?,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::types::TraitBound, String> {
        Ok(crate::types::TraitBound {
            trait_id: def_id(self.trait_id),
            type_args: decoder.decode_types(&self.type_args)?,
        })
    }
}

impl SerializedPredicate {
    pub(super) fn encode(
        value: &crate::types::Predicate,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        match value {
            crate::types::Predicate::Trait {
                subject,
                trait_id,
                args,
            } => Ok(Self {
                subject: encoder.encode_type(subject)?,
                trait_id: product_def_id(*trait_id),
                args: encoder.encode_types(args)?,
            }),
        }
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::types::Predicate, String> {
        Ok(crate::types::Predicate::Trait {
            subject: decoder.decode_type(self.subject)?,
            trait_id: def_id(self.trait_id),
            args: decoder.decode_types(&self.args)?,
        })
    }
}

pub(super) fn encode_generic_bounds(
    bounds: &crate::hir::HirGenericBounds,
    encoder: &mut ProductTypeEncoder,
) -> Result<SerializedGenericBounds, String> {
    let mut encoded = bounds
        .iter()
        .map(|(param, bounds)| {
            Ok((
                encode_generic_param_id(*param),
                bounds
                    .iter()
                    .map(|bound| SerializedTraitBound::encode(bound, encoder))
                    .collect::<Result<_, _>>()?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    encoded.sort_by_key(|(param, _)| *param);
    Ok(SerializedGenericBounds {
        bounds: encoded,
        predicates: bounds
            .predicates
            .iter()
            .map(|predicate| SerializedPredicate::encode(predicate, encoder))
            .collect::<Result<_, _>>()?,
    })
}

pub(super) fn decode_generic_bounds(
    bounds: SerializedGenericBounds,
    decoder: &mut ProductTypeDecoder<'_>,
) -> Result<crate::hir::HirGenericBounds, String> {
    let mut decoded: crate::hir::HirGenericBounds = bounds
        .bounds
        .into_iter()
        .map(|(param, bounds)| {
            Ok((
                decode_generic_param_id(param),
                bounds
                    .into_iter()
                    .map(|bound| bound.decode(decoder))
                    .collect::<Result<_, _>>()?,
            ))
        })
        .collect::<Result<_, String>>()?;
    decoded.predicates = bounds
        .predicates
        .into_iter()
        .map(|predicate| predicate.decode(decoder))
        .collect::<Result<_, _>>()?;
    Ok(decoded)
}

pub(super) fn encode_generic_param_decls(
    decls: &[GenericParamDecl],
    encoder: &mut ProductTypeEncoder,
) -> Result<Vec<ProductGenericParamDecl>, String> {
    decls
        .iter()
        .map(|decl| {
            Ok(ProductGenericParamDecl {
                id: encode_generic_param_id(decl.id),
                name: decl.name.clone(),
                kind: decl.kind.clone(),
                default: decl
                    .default
                    .as_ref()
                    .map(|ty| encoder.encode_type(ty))
                    .transpose()?,
            })
        })
        .collect()
}

pub(super) fn decode_generic_param_decls(
    decls: Vec<ProductGenericParamDecl>,
    decoder: &mut ProductTypeDecoder<'_>,
) -> Result<Vec<GenericParamDecl>, String> {
    decls
        .into_iter()
        .map(|decl| {
            Ok(GenericParamDecl {
                id: decode_generic_param_id(decl.id),
                name: decl.name,
                kind: decl.kind,
                default: decl.default.map(|ty| decoder.decode_type(ty)).transpose()?,
            })
        })
        .collect()
}

fn encode_generic_param_id(id: GenericParamId) -> ProductGenericParamId {
    ProductGenericParamId {
        owner: product_def_id(id.owner),
        index: id.index,
    }
}

fn decode_generic_param_id(id: ProductGenericParamId) -> GenericParamId {
    GenericParamId {
        owner: def_id(id.owner),
        index: id.index,
    }
}
