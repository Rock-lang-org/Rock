use std::collections::{BTreeMap, HashMap};

use super::hir_schema::{
    SerializedGenericBounds, SerializedHirAssociatedTypeDef, SerializedHirBlock,
    SerializedHirField, SerializedHirFunctionSig, SerializedHirImpl, SerializedHirParam,
    SerializedHirVariantFields, SerializedPredicate,
};
use super::type_codec::{ProductTypeDecoder, ProductTypeEncoder};
use super::type_rows::{ProductGenericParamDecl, ProductTypeId, ProductTypeTable};
use crate::products::{
    ProductCrateIdentity, ProductDefId, ProductDependencyIdentity, ProductFreshnessMetadata,
    ProductIdentityTable, ProductLanguageItems, ProductLinkData, ProductSourceFingerprint,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedProductArtifact {
    pub format_version: u32,
    pub type_table: ProductTypeTable,
    pub products: SerializedCompilerProducts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedCompilerProducts {
    pub crate_identity: ProductCrateIdentity,
    pub identity_table: ProductIdentityTable,
    pub freshness: ProductFreshnessMetadata,
    pub interface: SerializedProductInterface,
    pub bodies: SerializedProductBodies,
    pub link: ProductLinkData,
    pub dependencies: Vec<ProductDependencyIdentity>,
    pub source_fingerprint: ProductSourceFingerprint,
    pub infix_precedence: BTreeMap<String, crate::products::ProductInfixOperator>,
    pub proc_macros: Vec<crate::macro_expansion::proc_macro::ProcMacroArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(in crate::products) struct SerializedProductInterface {
    pub functions: BTreeMap<ProductDefId, SerializedProductFunctionInterface>,
    pub structs: BTreeMap<ProductDefId, SerializedProductStructInterface>,
    pub enums: BTreeMap<ProductDefId, SerializedProductEnumInterface>,
    pub type_aliases: BTreeMap<ProductDefId, SerializedProductTypeAliasInterface>,
    pub traits: BTreeMap<ProductDefId, SerializedProductTraitInterface>,
    pub impls: BTreeMap<ProductDefId, SerializedProductImplInterface>,
    pub externs: BTreeMap<ProductDefId, SerializedProductExternInterface>,
    pub effective_trait_methods: BTreeMap<(ProductDefId, ProductDefId), ProductDefId>,
    #[serde(default)]
    pub language_items: ProductLanguageItems,
    #[serde(default)]
    pub macros: Vec<crate::products::ProductMacroExport>,
    #[serde(default)]
    pub prelude_macros: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) enum SerializedHirImplReceiverPattern {
    Exact(ProductTypeId),
    SliceFamily { element: ProductTypeId },
    Constructor(ProductTypeId),
}

impl SerializedHirImplReceiverPattern {
    pub(super) fn encode(
        pattern: &crate::hir::HirImplReceiverPattern,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(match pattern {
            crate::hir::HirImplReceiverPattern::Exact(ty) => Self::Exact(encoder.encode_type(ty)?),
            crate::hir::HirImplReceiverPattern::SliceFamily { element } => Self::SliceFamily {
                element: encoder.encode_type(element)?,
            },
            crate::hir::HirImplReceiverPattern::Constructor(ty) => {
                Self::Constructor(encoder.encode_type(ty)?)
            }
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirImplReceiverPattern, String> {
        Ok(match self {
            Self::Exact(ty) => crate::hir::HirImplReceiverPattern::Exact(decoder.decode_type(ty)?),
            Self::SliceFamily { element } => crate::hir::HirImplReceiverPattern::SliceFamily {
                element: decoder.decode_type(element)?,
            },
            Self::Constructor(ty) => {
                crate::hir::HirImplReceiverPattern::Constructor(decoder.decode_type(ty)?)
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(in crate::products) struct SerializedProductBodies {
    pub functions: BTreeMap<ProductDefId, SerializedHirFunction>,
    pub generic_impls: BTreeMap<ProductDefId, SerializedHirImpl>,
    pub trait_default_methods: BTreeMap<ProductDefId, SerializedHirFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedHirFunction {
    pub id: crate::ids::DefId,
    pub name: String,
    pub generic_params: Vec<ProductGenericParamDecl>,
    pub generic_bounds: SerializedGenericBounds,
    pub params: Vec<SerializedHirParam>,
    pub ret_type: ProductTypeId,
    pub body: SerializedHirBlock,
    pub is_curried: bool,
    pub is_method: bool,
    pub self_receiver: Option<crate::types::ReceiverMode>,
    pub is_unsafe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedProductFunctionInterface {
    pub id: crate::ids::DefId,
    pub name: String,
    pub generic_params: Vec<ProductGenericParamDecl>,
    pub generic_bounds: SerializedGenericBounds,
    pub params: Vec<ProductTypeId>,
    pub ret_type: ProductTypeId,
    pub is_curried: bool,
    pub is_method: bool,
    pub self_receiver: Option<crate::types::ReceiverMode>,
    pub is_unsafe: bool,
    pub constant_arithmetic: Option<crate::products::ArithmeticSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedProductStructInterface {
    pub id: crate::ids::DefId,
    pub name: String,
    pub generic_params: Vec<ProductGenericParamDecl>,
    pub fields: Vec<SerializedHirField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedProductTypeAliasInterface {
    pub id: crate::ids::DefId,
    pub name: String,
    pub generic_params: Vec<ProductGenericParamDecl>,
    pub ty: ProductTypeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedProductEnumInterface {
    pub id: crate::ids::DefId,
    pub name: String,
    pub generic_params: Vec<ProductGenericParamDecl>,
    pub variants: Vec<SerializedProductEnumVariantInterface>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedProductEnumVariantInterface {
    pub id: crate::ids::VariantId,
    pub name: String,
    pub fields: SerializedHirVariantFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedProductTraitInterface {
    pub id: crate::ids::DefId,
    pub name: String,
    pub generic_params: Vec<ProductGenericParamDecl>,
    pub target: Option<ProductGenericParamDecl>,
    pub predicates: Vec<SerializedPredicate>,
    pub associated_types: Vec<crate::hir::HirAssociatedTypeDecl>,
    pub methods: BTreeMap<String, SerializedProductFunctionInterface>,
    pub signatures: HashMap<String, SerializedHirFunctionSig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedProductImplInterface {
    pub id: crate::ids::DefId,
    pub owner: crate::hir::HirImplOwner,
    pub type_name: String,
    pub type_generics: Vec<ProductGenericParamDecl>,
    pub receiver_pattern: SerializedHirImplReceiverPattern,
    pub trait_name: Option<String>,
    pub trait_id: Option<crate::ids::DefId>,
    pub trait_generics: Vec<ProductGenericParamDecl>,
    pub trait_arg_types: Vec<ProductTypeId>,
    pub associated_types: Vec<SerializedHirAssociatedTypeDef>,
    pub bounds: SerializedGenericBounds,
    pub methods: BTreeMap<String, SerializedProductFunctionInterface>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedProductExternInterface {
    pub id: crate::ids::DefId,
    pub name: String,
    pub params: Vec<ProductTypeId>,
    pub ret: ProductTypeId,
    pub variadic: bool,
    #[serde(default)]
    pub is_unsafe: bool,
}
