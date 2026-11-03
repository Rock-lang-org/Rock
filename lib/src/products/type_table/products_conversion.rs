use super::collection_codec::{decode_map, encode_map};
use super::hir_schema::SerializedHirImpl;
use super::product_schema::{
    SerializedCompilerProducts, SerializedHirFunction, SerializedProductBodies,
    SerializedProductEnumInterface, SerializedProductExternInterface,
    SerializedProductFunctionInterface, SerializedProductImplInterface, SerializedProductInterface,
    SerializedProductStructInterface, SerializedProductTraitInterface,
    SerializedProductTypeAliasInterface,
};
use super::type_codec::{ProductTypeDecoder, ProductTypeEncoder};
use crate::products::{CompilerProducts, ProductBodies, ProductInterface};

impl SerializedCompilerProducts {
    pub(super) fn encode(
        products: &CompilerProducts,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            crate_identity: products.crate_identity.clone(),
            identity_table: products.identity_table.clone(),
            freshness: products.freshness_metadata(),
            interface: SerializedProductInterface::encode(&products.interface, encoder)?,
            bodies: SerializedProductBodies::encode(&products.bodies, encoder)?,
            link: products.link.clone(),
            dependencies: products.dependencies.clone(),
            source_fingerprint: products.source_fingerprint.clone(),
            infix_precedence: products.infix_precedence.clone(),
            proc_macros: products.proc_macros.clone(),
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<CompilerProducts, String> {
        let freshness = self.freshness;
        let products = CompilerProducts {
            crate_identity: self.crate_identity,
            identity_table: self.identity_table,
            interface: self.interface.decode(decoder)?,
            bodies: self.bodies.decode(decoder)?,
            link: self.link,
            dependencies: self.dependencies,
            source_fingerprint: self.source_fingerprint,
            infix_precedence: self.infix_precedence,
            proc_macros: self.proc_macros,
        };
        products.validate_freshness_metadata(&freshness)?;
        Ok(products)
    }
}

impl SerializedProductInterface {
    fn encode(
        interface: &ProductInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            functions: encode_map(
                &interface.functions,
                encoder,
                SerializedProductFunctionInterface::encode,
            )?,
            structs: encode_map(
                &interface.structs,
                encoder,
                SerializedProductStructInterface::encode,
            )?,
            enums: encode_map(
                &interface.enums,
                encoder,
                SerializedProductEnumInterface::encode,
            )?,
            type_aliases: encode_map(
                &interface.type_aliases,
                encoder,
                SerializedProductTypeAliasInterface::encode,
            )?,
            traits: encode_map(
                &interface.traits,
                encoder,
                SerializedProductTraitInterface::encode,
            )?,
            impls: encode_map(
                &interface.impls,
                encoder,
                SerializedProductImplInterface::encode,
            )?,
            externs: encode_map(
                &interface.externs,
                encoder,
                SerializedProductExternInterface::encode,
            )?,
            effective_trait_methods: interface.effective_trait_methods.clone(),
            language_items: interface.language_items.clone(),
            macros: interface.macros.clone(),
            prelude_macros: interface.prelude_macros.clone(),
        })
    }

    fn decode(self, decoder: &mut ProductTypeDecoder<'_>) -> Result<ProductInterface, String> {
        Ok(ProductInterface {
            functions: decode_map(
                self.functions,
                decoder,
                SerializedProductFunctionInterface::decode,
            )?,
            structs: decode_map(
                self.structs,
                decoder,
                SerializedProductStructInterface::decode,
            )?,
            enums: decode_map(self.enums, decoder, SerializedProductEnumInterface::decode)?,
            type_aliases: decode_map(
                self.type_aliases,
                decoder,
                SerializedProductTypeAliasInterface::decode,
            )?,
            traits: decode_map(
                self.traits,
                decoder,
                SerializedProductTraitInterface::decode,
            )?,
            impls: decode_map(self.impls, decoder, SerializedProductImplInterface::decode)?,
            externs: decode_map(
                self.externs,
                decoder,
                SerializedProductExternInterface::decode,
            )?,
            effective_trait_methods: self.effective_trait_methods,
            language_items: self.language_items,
            macros: self.macros,
            prelude_macros: self.prelude_macros,
        })
    }
}

impl SerializedProductBodies {
    fn encode(bodies: &ProductBodies, encoder: &mut ProductTypeEncoder) -> Result<Self, String> {
        Ok(Self {
            functions: encode_map(&bodies.functions, encoder, SerializedHirFunction::encode)?,
            generic_impls: encode_map(&bodies.generic_impls, encoder, SerializedHirImpl::encode)?,
            trait_default_methods: encode_map(
                &bodies.trait_default_methods,
                encoder,
                SerializedHirFunction::encode,
            )?,
        })
    }

    fn decode(self, decoder: &mut ProductTypeDecoder<'_>) -> Result<ProductBodies, String> {
        Ok(ProductBodies {
            functions: decode_map(self.functions, decoder, SerializedHirFunction::decode)?,
            generic_impls: decode_map(self.generic_impls, decoder, SerializedHirImpl::decode)?,
            trait_default_methods: decode_map(
                self.trait_default_methods,
                decoder,
                SerializedHirFunction::decode,
            )?,
        })
    }
}
