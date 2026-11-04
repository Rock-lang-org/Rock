use super::collection_codec::{decode_hash_map, encode_hash_map};
use super::generic_conversion::{
    decode_generic_bounds, decode_generic_param_decls, encode_generic_bounds,
    encode_generic_param_decls,
};
use super::hir_schema::{
    SerializedHirAssociatedTypeDef, SerializedHirBlock, SerializedHirField,
    SerializedHirFunctionSig, SerializedHirImpl, SerializedHirParam, SerializedHirVariantFields,
};
use super::product_schema::{SerializedHirFunction, SerializedHirImplReceiverPattern};
use super::type_codec::{ProductTypeDecoder, ProductTypeEncoder};
use crate::products::ProductAssociatedTypeInterface;

impl SerializedHirFunction {
    pub(super) fn encode(
        value: &crate::hir::AcceptedHirFunction,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params, encoder)?,
            generic_bounds: encode_generic_bounds(&value.generic_bounds, encoder)?,
            params: value
                .params
                .iter()
                .map(|param| SerializedHirParam::encode(param, encoder))
                .collect::<Result<_, _>>()?,
            ret_type: encoder.encode_type(&value.ret_type)?,
            body: SerializedHirBlock::encode(&value.body, encoder)?,
            is_curried: value.is_curried,
            is_method: value.is_method,
            self_receiver: value.self_receiver,
            is_unsafe: value.is_unsafe,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::AcceptedHirFunction, String> {
        Ok(crate::hir::HirFunctionFor::<crate::hir::AcceptedHir> {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params, decoder)?,
            generic_bounds: decode_generic_bounds(self.generic_bounds, decoder)?,
            params: self
                .params
                .into_iter()
                .map(|param| param.decode(decoder))
                .collect::<Result<_, _>>()?,
            ret_type: decoder.decode_type(self.ret_type)?,
            body: self.body.decode(decoder)?,
            is_curried: self.is_curried,
            is_method: self.is_method,
            self_receiver: self.self_receiver,
            is_unsafe: self.is_unsafe,
        })
    }
}

impl SerializedHirParam {
    pub(super) fn encode(
        value: &crate::hir::HirParam,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            name: value.name.clone(),
            local_id: value.local_id,
            ty: encoder.encode_type(&value.ty)?,
            mutable: value.mutable,
            is_ref: value.is_ref,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirParam, String> {
        Ok(crate::hir::HirParam {
            name: self.name,
            local_id: self.local_id,
            ty: decoder.decode_type(self.ty)?,
            mutable: self.mutable,
            is_ref: self.is_ref,
        })
    }
}

impl SerializedHirField {
    fn encode(
        value: &crate::hir::HirField,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            ty: encoder.encode_type(&value.ty)?,
            public: value.public,
        })
    }

    pub(super) fn encode_interface(
        value: &crate::products::ProductFieldInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            ty: encoder.encode_type(&value.ty)?,
            public: value.public,
        })
    }

    fn decode(self, decoder: &mut ProductTypeDecoder<'_>) -> Result<crate::hir::HirField, String> {
        Ok(crate::hir::HirField {
            id: self.id,
            name: self.name,
            ty: decoder.decode_type(self.ty)?,
            public: self.public,
        })
    }

    pub(super) fn decode_interface(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::products::ProductFieldInterface, String> {
        Ok(crate::products::ProductFieldInterface {
            id: self.id,
            name: self.name,
            ty: decoder.decode_type(self.ty)?,
            public: self.public,
        })
    }
}

impl SerializedHirVariantFields {
    pub(super) fn encode(
        value: &crate::hir::HirVariantFields,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(match value {
            crate::hir::HirVariantFields::Named(fields) => Self::Named(
                fields
                    .iter()
                    .map(|field| SerializedHirField::encode(field, encoder))
                    .collect::<Result<_, _>>()?,
            ),
            crate::hir::HirVariantFields::Positional(types) => {
                Self::Positional(encoder.encode_types(types)?)
            }
            crate::hir::HirVariantFields::Unit => Self::Unit,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirVariantFields, String> {
        Ok(match self {
            Self::Named(fields) => crate::hir::HirVariantFields::Named(
                fields
                    .into_iter()
                    .map(|field| field.decode(decoder))
                    .collect::<Result<_, _>>()?,
            ),
            Self::Positional(types) => {
                crate::hir::HirVariantFields::Positional(decoder.decode_types(&types)?)
            }
            Self::Unit => crate::hir::HirVariantFields::Unit,
        })
    }
}

impl SerializedHirFunctionSig {
    pub(super) fn encode(
        value: &crate::hir::HirFunctionSig,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params, encoder)?,
            params: encoder.encode_types(&value.params)?,
            ret: encoder.encode_type(&value.ret)?,
            generic_bounds: encode_generic_bounds(&value.generic_bounds, encoder)?,
            self_receiver: value.self_receiver,
            is_unsafe: value.is_unsafe,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirFunctionSig, String> {
        Ok(crate::hir::HirFunctionSig {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params, decoder)?,
            params: decoder.decode_types(&self.params)?,
            ret: decoder.decode_type(self.ret)?,
            generic_bounds: decode_generic_bounds(self.generic_bounds, decoder)?,
            self_receiver: self.self_receiver,
            is_unsafe: self.is_unsafe,
        })
    }
}

impl SerializedHirImpl {
    pub(super) fn encode(
        value: &crate::hir::AcceptedHirImpl,
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
                    SerializedHirAssociatedTypeDef::encode(associated_type, encoder)
                })
                .collect::<Result<_, _>>()?,
            bounds: encode_generic_bounds(&value.bounds, encoder)?,
            methods: encode_hash_map(&value.methods, encoder, SerializedHirFunction::encode)?,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::AcceptedHirImpl, String> {
        Ok(crate::hir::HirImplFor::<crate::hir::AcceptedHir> {
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
                .map(|associated_type| associated_type.decode(decoder))
                .collect::<Result<_, _>>()?,
            bounds: decode_generic_bounds(self.bounds, decoder)?,
            methods: decode_hash_map(self.methods, decoder, SerializedHirFunction::decode)?,
        })
    }
}

impl SerializedHirAssociatedTypeDef {
    fn encode(
        value: &crate::hir::HirAssociatedTypeDef,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            kind: value.kind.clone(),
            ty: encoder.encode_type(&value.ty)?,
        })
    }

    pub(super) fn encode_interface(
        value: &ProductAssociatedTypeInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            kind: value.kind.clone(),
            ty: encoder.encode_type(&value.ty)?,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirAssociatedTypeDef, String> {
        Ok(crate::hir::HirAssociatedTypeDef {
            id: self.id,
            name: self.name,
            kind: self.kind,
            ty: decoder.decode_type(self.ty)?,
        })
    }

    pub(super) fn decode_interface(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductAssociatedTypeInterface, String> {
        Ok(ProductAssociatedTypeInterface {
            id: self.id,
            name: self.name,
            kind: self.kind,
            ty: decoder.decode_type(self.ty)?,
        })
    }
}
