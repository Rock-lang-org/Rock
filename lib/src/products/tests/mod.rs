use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use crate::collect::resolver::ResolverTables;
use crate::hir::{
    AcceptedHir, AcceptedHirFunction, AcceptedHirProgram, HirAssociatedTypeDecl, HirBlock,
    HirCallTarget, HirClosureCapture, HirClosureCaptureKind, HirEnum, HirExpr, HirExtern,
    HirFieldLocation, HirFunction, HirFunctionSig, HirImpl, HirImplFor, HirLanguageItems,
    HirNameTables, HirParam, HirPattern, HirProgram, HirProgramFor, HirStruct,
    HirStructLiteralField, HirStructPatternField, HirTrait, HirTraitFor, HirVarRef, HirVarTarget,
    HirVariantLocation,
};
use crate::ids::IdGen;
use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
use crate::infer::ResolvedHirProgram;
use crate::language_items::{
    DropLanguageItems, IndexLanguageItems, IndexMutLanguageItems, LanguageItems,
    RangeLanguageItems, SizedLanguageItems, TryLanguageItems,
};
use crate::lexer::Span;
use crate::products::type_table;
use crate::products::{
    encode_product_artifact, read_product_artifact_header, serialized_artifact_from_bytes_for_test,
    CompilerProducts, ProductArtifactHeader, ProductArtifactPreamble, ProductBodies,
    ProductCrateId, ProductCrateIdentity, ProductDefId, ProductDependencyCapabilities,
    ProductDependencyIdentity, ProductDependencyLinkCapability, ProductIdentityTable,
    ProductLanguageItems, ProductLinkData, ProductLinkRecord, ProductLocalDefId,
    ProductSourceFingerprint, MAX_PRODUCT_ARTIFACT_BYTES, MAX_PRODUCT_ARTIFACT_HEADER_BYTES,
    MAX_PRODUCT_ARTIFACT_STRING_BYTES, MAX_PRODUCT_ARTIFACT_TOTAL_STRING_BYTES,
    PRODUCT_ARTIFACT_FORMAT_VERSION, PRODUCT_ARTIFACT_MAGIC,
};
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, Type};

mod emission;
mod format;
mod ids;
mod invalid_ids;
mod language_items;
mod link_records;
mod prelude_exports;
mod remap;
mod support;
mod trait_defaults;
mod type_tables;

use support::*;
