use super::collection_codec::{decode_hash_map, encode_hash_map};
use super::generic_conversion::{
    decode_generic_bounds, decode_generic_param_decls, encode_generic_bounds,
    encode_generic_param_decls,
};
use super::hir_schema::{
    SerializedHirAssociatedTypeDef, SerializedHirField, SerializedHirFunctionSig,
    SerializedHirVariantFields, SerializedPredicate,
};
use super::product_schema::{
    SerializedHirImplReceiverPattern, SerializedProductEnumInterface,
    SerializedProductEnumVariantInterface, SerializedProductExternInterface,
    SerializedProductFunctionInterface, SerializedProductImplInterface,
    SerializedProductStructInterface, SerializedProductTraitInterface,
    SerializedProductTypeAliasInterface,
};
use super::type_codec::{ProductTypeDecoder, ProductTypeEncoder};
use crate::products::{
    ProductEnumInterface, ProductEnumVariantInterface, ProductExternInterface,
    ProductFunctionInterface, ProductImplInterface, ProductStructInterface, ProductTraitInterface,
    ProductTypeAliasInterface,
};

impl SerializedProductFunctionInterface {
    pub(super) fn encode(
        value: &ProductFunctionInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        if let Some(summary) = &value.constant_arithmetic {
            summary.validate(value.params.len())?;
        }
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params, encoder)?,
            generic_bounds: encode_generic_bounds(&value.generic_bounds, encoder)?,
            params: encoder.encode_types(&value.params)?,
            ret_type: encoder.encode_type(&value.ret_type)?,
            is_curried: value.is_curried,
            is_method: value.is_method,
            self_receiver: value.self_receiver,
            is_unsafe: value.is_unsafe,
            constant_arithmetic: value.constant_arithmetic.clone(),
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductFunctionInterface, String> {
        if let Some(summary) = &self.constant_arithmetic {
            summary.validate(self.params.len())?;
        }
        Ok(ProductFunctionInterface {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params, decoder)?,
            generic_bounds: decode_generic_bounds(self.generic_bounds, decoder)?,
            params: decoder.decode_types(&self.params)?,
            ret_type: decoder.decode_type(self.ret_type)?,
            is_curried: self.is_curried,
            is_method: self.is_method,
            self_receiver: self.self_receiver,
            is_unsafe: self.is_unsafe,
            constant_arithmetic: self.constant_arithmetic,
        })
    }
}

impl SerializedProductStructInterface {
    pub(super) fn encode(
        value: &ProductStructInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params, encoder)?,
            fields: value
                .fields
                .iter()
                .map(|field| SerializedHirField::encode_interface(field, encoder))
                .collect::<Result<_, _>>()?,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductStructInterface, String> {
        Ok(ProductStructInterface {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params, decoder)?,
            fields: self
                .fields
                .into_iter()
                .map(|field| field.decode_interface(decoder))
                .collect::<Result<_, _>>()?,
        })
    }
}

impl SerializedProductTypeAliasInterface {
    pub(super) fn encode(
        value: &ProductTypeAliasInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params, encoder)?,
            ty: encoder.encode_type(&value.ty)?,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductTypeAliasInterface, String> {
        Ok(ProductTypeAliasInterface {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params, decoder)?,
            ty: decoder.decode_type(self.ty)?,
        })
    }
}

impl SerializedProductEnumInterface {
    pub(super) fn encode(
        value: &ProductEnumInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params, encoder)?,
            variants: value
                .variants
                .iter()
                .map(|variant| SerializedProductEnumVariantInterface::encode(variant, encoder))
                .collect::<Result<_, _>>()?,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductEnumInterface, String> {
        Ok(ProductEnumInterface {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params, decoder)?,
            variants: self
                .variants
                .into_iter()
                .map(|variant| variant.decode(decoder))
                .collect::<Result<_, _>>()?,
        })
    }
}

impl SerializedProductEnumVariantInterface {
    fn encode(
        value: &ProductEnumVariantInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            fields: SerializedHirVariantFields::encode(&value.fields, encoder)?,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductEnumVariantInterface, String> {
        Ok(ProductEnumVariantInterface {
            id: self.id,
            name: self.name,
            fields: self.fields.decode(decoder)?,
        })
    }
}

impl SerializedProductTraitInterface {
    pub(super) fn encode(
        value: &ProductTraitInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params, encoder)?,
            target: value
                .target
                .as_ref()
                .map(|target| {
                    encode_generic_param_decls(std::slice::from_ref(target), encoder)
                        .map(|mut params| params.remove(0))
                })
                .transpose()?,
            predicates: value
                .predicates
                .iter()
                .map(|predicate| SerializedPredicate::encode(predicate, encoder))
                .collect::<Result<_, _>>()?,
            associated_types: value.associated_types.clone(),
            methods: value
                .methods
                .iter()
                .map(|(name, method)| {
                    Ok((
                        name.clone(),
                        SerializedProductFunctionInterface::encode(method, encoder)?,
                    ))
                })
                .collect::<Result<_, String>>()?,
            signatures: encode_hash_map(
                &value.signatures,
                encoder,
                SerializedHirFunctionSig::encode,
            )?,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductTraitInterface, String> {
        Ok(ProductTraitInterface {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params, decoder)?,
            target: self
                .target
                .map(|target| {
                    decode_generic_param_decls(vec![target], decoder)
                        .map(|mut params| params.remove(0))
                })
                .transpose()?,
            predicates: self
                .predicates
                .into_iter()
                .map(|predicate| predicate.decode(decoder))
                .collect::<Result<_, _>>()?,
            associated_types: self.associated_types,
            methods: self
                .methods
                .into_iter()
                .map(|(name, method)| Ok((name, method.decode(decoder)?)))
                .collect::<Result<_, String>>()?,
            signatures: decode_hash_map(
                self.signatures,
                decoder,
                SerializedHirFunctionSig::decode,
            )?,
        })
    }
}

impl SerializedProductImplInterface {
    pub(super) fn encode(
        value: &ProductImplInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            owner: value.owner.clone(),
            type_name: value.type_name.clone(),
            type_generics: encode_generic_param_decls(&value.type_generics, encoder)?,
            receiver_pattern: SerializedHirImplReceiverPattern::encode(
                &value.receiver_pattern,
                encoder,
            )?,
            trait_name: value.trait_name.clone(),
            trait_id: value.trait_id,
            trait_generics: encode_generic_param_decls(&value.trait_generics, encoder)?,
            trait_arg_types: encoder.encode_types(&value.trait_arg_types)?,
            associated_types: value
                .associated_types
                .iter()
                .map(|associated_type| {
                    SerializedHirAssociatedTypeDef::encode_interface(associated_type, encoder)
                })
                .collect::<Result<_, _>>()?,
            bounds: encode_generic_bounds(&value.bounds, encoder)?,
            methods: value
                .methods
                .iter()
                .map(|(name, method)| {
                    Ok((
                        name.clone(),
                        SerializedProductFunctionInterface::encode(method, encoder)?,
                    ))
                })
                .collect::<Result<_, String>>()?,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductImplInterface, String> {
        Ok(ProductImplInterface {
            id: self.id,
            owner: self.owner,
            type_name: self.type_name,
            type_generics: decode_generic_param_decls(self.type_generics, decoder)?,
            receiver_pattern: self.receiver_pattern.decode(decoder)?,
            trait_name: self.trait_name,
            trait_id: self.trait_id,
            trait_generics: decode_generic_param_decls(self.trait_generics, decoder)?,
            trait_arg_types: decoder.decode_types(&self.trait_arg_types)?,
            associated_types: self
                .associated_types
                .into_iter()
                .map(|associated_type| associated_type.decode_interface(decoder))
                .collect::<Result<_, _>>()?,
            bounds: decode_generic_bounds(self.bounds, decoder)?,
            methods: self
                .methods
                .into_iter()
                .map(|(name, method)| Ok((name, method.decode(decoder)?)))
                .collect::<Result<_, String>>()?,
        })
    }
}

impl SerializedProductExternInterface {
    pub(super) fn encode(
        value: &ProductExternInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            params: encoder.encode_types(&value.params)?,
            ret: encoder.encode_type(&value.ret)?,
            variadic: value.variadic,
            is_unsafe: value.is_unsafe,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductExternInterface, String> {
        Ok(ProductExternInterface {
            id: self.id,
            name: self.name,
            params: decoder.decode_types(&self.params)?,
            ret: decoder.decode_type(self.ret)?,
            variadic: self.variadic,
            is_unsafe: self.is_unsafe,
        })
    }
}
