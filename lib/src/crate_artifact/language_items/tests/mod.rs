use super::index_mut_impls::validate_product_index_mut_impls;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::hir::{
    HirAssociatedTypeDecl, HirEnum, HirFunctionSig, HirImplOwner, HirImplReceiverPattern, HirTrait,
    HirVariant, HirVariantFields,
};
use crate::ids::{AssocTypeId, CrateId, VariantId};
use crate::language_items::{
    DropLanguageItems, IndexLanguageItems, IndexMutLanguageItems, LanguageItems,
    SizedLanguageItems, TryLanguageItems,
};
use crate::products::{
    CompilerProducts, ProductAssociatedTypeInterface, ProductBodies, ProductCrateId,
    ProductCrateIdentity, ProductFunctionInterface, ProductIdentityTable, ProductImplInterface,
    ProductInterface, ProductLinkData, ProductLinkRecord, ProductLocalDefId,
    ProductSourceFingerprint, ProductStructInterface,
};
use crate::types::ReceiverMode;
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, TraitBound, Type};

use super::{product_def_id, validate_product_language_items};

mod index_pairs;
mod registry;
mod support;
mod validation;

use support::*;
