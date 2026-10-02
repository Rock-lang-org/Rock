use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use bincode::Options;
use serde::{Deserialize, Serialize};

use crate::hir::{
    function_requires_downstream_specialization, hir_function_is_codegen_concrete,
    impl_requires_downstream_specialization, AcceptedHir, AcceptedHirFunction, HirCallTarget,
    HirEnum, HirExtern, HirFunctionFor, HirGenericBounds, HirImplFor, HirMethodCallTarget,
    HirPattern, HirPhase, HirProgramFor, HirSelectedMethodTarget, HirStruct, HirTraitFor,
    HirTypeAlias, HirVarTarget, HirVariantFields,
};
use crate::ids::{CrateId, DefId, Idx, LocalDefId};
use crate::infer::ResolvedHirProgram;
use crate::language_items::{
    DropLanguageItems, FnLanguageItems, FnMutLanguageItems, FnOnceLanguageItems, FoldLanguageItems,
    IndexLanguageItems, IndexMutLanguageItems, LanguageItems, RangeLanguageItems,
    SendLanguageItems, SizedLanguageItems, SyncLanguageItems, TryLanguageItems,
};
use crate::types::{GenericParamDecl, Type};

pub mod object_abi;
#[cfg(test)]
mod object_abi_tests;
mod type_table;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProductCrateId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProductLocalDefId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProductDefId {
    pub crate_id: ProductCrateId,
    pub local_id: ProductLocalDefId,
}

pub(crate) type ProductIdRemap = BTreeMap<ProductDefId, BTreeSet<ProductDefId>>;

impl From<CrateId> for ProductCrateId {
    fn from(value: CrateId) -> Self {
        Self(value.raw())
    }
}

impl From<LocalDefId> for ProductLocalDefId {
    fn from(value: LocalDefId) -> Self {
        Self(value.raw())
    }
}

impl From<DefId> for ProductDefId {
    fn from(value: DefId) -> Self {
        Self {
            crate_id: ProductCrateId::from(value.crate_id),
            local_id: ProductLocalDefId::from(value.local),
        }
    }
}

pub const PRODUCT_ARTIFACT_FORMAT_VERSION: u32 = 53;
pub const PRODUCT_ARTIFACT_MAGIC: [u8; 8] = *b"ROCKRKCA";
pub const MAX_PRODUCT_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_PRODUCT_ARTIFACT_HEADER_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_PRODUCT_ARTIFACT_STRING_BYTES: usize = 1024 * 1024;
pub const MAX_PRODUCT_ARTIFACT_TOTAL_STRING_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PRODUCT_ARTIFACT_TYPE_ROWS: usize = 1_000_000;
pub const MAX_PRODUCT_ARTIFACT_DECLARATIONS: usize = 1_000_000;
pub const MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS: usize = 1_024;
pub const MAX_PRODUCT_ARTIFACT_TYPE_DEPTH: usize = 256;
pub const MAX_PRODUCT_ARTIFACT_NORMALIZATION_NODES: usize = 4_000_000;
const PRODUCT_ARTIFACT_PREAMBLE_BYTES: u64 = 8 + 4 + 8 + 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductArtifactPreamble {
    pub magic: [u8; 8],
    pub format_version: u32,
    pub header_len: u64,
    pub payload_len: u64,
}

impl ProductArtifactPreamble {
    fn encode(self) -> [u8; PRODUCT_ARTIFACT_PREAMBLE_BYTES as usize] {
        let mut bytes = [0; PRODUCT_ARTIFACT_PREAMBLE_BYTES as usize];
        bytes[..8].copy_from_slice(&self.magic);
        bytes[8..12].copy_from_slice(&self.format_version.to_le_bytes());
        bytes[12..20].copy_from_slice(&self.header_len.to_le_bytes());
        bytes[20..28].copy_from_slice(&self.payload_len.to_le_bytes());
        bytes
    }

    fn decode(bytes: [u8; PRODUCT_ARTIFACT_PREAMBLE_BYTES as usize]) -> Result<Self, String> {
        let preamble = Self {
            magic: bytes[..8].try_into().expect("fixed magic width"),
            format_version: u32::from_le_bytes(bytes[8..12].try_into().expect("fixed u32 width")),
            header_len: u64::from_le_bytes(bytes[12..20].try_into().expect("fixed u64 width")),
            payload_len: u64::from_le_bytes(bytes[20..28].try_into().expect("fixed u64 width")),
        };
        preamble.validate()?;
        Ok(preamble)
    }

    fn validate(self) -> Result<(), String> {
        if self.magic != PRODUCT_ARTIFACT_MAGIC {
            return Err("Invalid product artifact magic".to_string());
        }
        if self.format_version != PRODUCT_ARTIFACT_FORMAT_VERSION {
            return Err(format!(
                "Unsupported product artifact format {} (expected {})",
                self.format_version, PRODUCT_ARTIFACT_FORMAT_VERSION
            ));
        }
        check_artifact_limit(
            "header bytes",
            self.header_len,
            MAX_PRODUCT_ARTIFACT_HEADER_BYTES,
        )?;
        let total = PRODUCT_ARTIFACT_PREAMBLE_BYTES
            .checked_add(self.header_len)
            .and_then(|size| size.checked_add(self.payload_len))
            .ok_or_else(|| "Product artifact declared size overflows u64".to_string())?;
        check_artifact_limit("artifact bytes", total, MAX_PRODUCT_ARTIFACT_BYTES)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductArtifactHeader {
    pub crate_identity: ProductCrateIdentity,
    pub freshness: ProductFreshnessMetadata,
    pub identity_dependencies: BTreeMap<ProductCrateId, ProductCrateIdentity>,
    pub dependencies: Vec<ProductDependencyIdentity>,
    #[serde(skip)]
    header_len: u64,
    #[serde(skip)]
    payload_len: u64,
}

impl ProductArtifactHeader {
    fn from_products(products: &CompilerProducts) -> Self {
        Self {
            crate_identity: products.crate_identity.clone(),
            freshness: products.freshness_metadata(),
            identity_dependencies: products.identity_table.dependencies.clone(),
            dependencies: products.dependencies.clone(),
            header_len: 0,
            payload_len: 0,
        }
    }

    pub fn read_from_path_bounded(path: &std::path::Path) -> Result<Self, String> {
        let file_len = std::fs::metadata(path)
            .map_err(|error| {
                format!(
                    "Failed to inspect product artifact {}: {error}",
                    path.display()
                )
            })?
            .len();
        check_artifact_limit("artifact bytes", file_len, MAX_PRODUCT_ARTIFACT_BYTES)?;
        let mut file = std::fs::File::open(path).map_err(|error| {
            format!(
                "Failed to open product artifact {}: {error}",
                path.display()
            )
        })?;
        let (preamble, header) = read_product_artifact_header(&mut file, file_len)?;
        Ok(Self {
            header_len: preamble.header_len,
            payload_len: preamble.payload_len,
            ..header
        })
    }

    fn matches_products(&self, products: &CompilerProducts) -> bool {
        self.crate_identity == products.crate_identity
            && self.freshness == products.freshness_metadata()
            && self.identity_dependencies == products.identity_table.dependencies
            && self.dependencies == products.dependencies
    }
}

pub struct PortableProductArtifact {
    header: ProductArtifactHeader,
    artifact: type_table::SerializedProductArtifact,
}

impl PortableProductArtifact {
    pub fn read_payload_from_path_bounded(
        path: &std::path::Path,
        expected_header: &ProductArtifactHeader,
    ) -> Result<Self, String> {
        let file_len = std::fs::metadata(path)
            .map_err(|error| {
                format!(
                    "Failed to inspect product artifact {}: {error}",
                    path.display()
                )
            })?
            .len();
        check_artifact_limit("artifact bytes", file_len, MAX_PRODUCT_ARTIFACT_BYTES)?;
        let mut file = std::fs::File::open(path).map_err(|error| {
            format!(
                "Failed to open product artifact {}: {error}",
                path.display()
            )
        })?;
        let (preamble, header) = read_product_artifact_header(&mut file, file_len)?;
        if header.crate_identity != expected_header.crate_identity
            || header.freshness != expected_header.freshness
            || header.identity_dependencies != expected_header.identity_dependencies
            || header.dependencies != expected_header.dependencies
            || preamble.header_len != expected_header.header_len
            || preamble.payload_len != expected_header.payload_len
        {
            return Err("Product artifact header changed between staged reads".to_string());
        }
        file.seek(SeekFrom::Start(
            PRODUCT_ARTIFACT_PREAMBLE_BYTES + preamble.header_len,
        ))
        .map_err(|error| format!("Failed to seek product artifact payload: {error}"))?;
        let payload_len = usize::try_from(preamble.payload_len)
            .map_err(|_| "Product artifact payload length does not fit usize".to_string())?;
        let mut payload = vec![0; payload_len];
        file.read_exact(&mut payload)
            .map_err(|error| format!("Failed to read product artifact payload: {error}"))?;
        let artifact = decode_product_artifact_payload(&payload)?;
        let header_string_bytes = product_artifact_header_string_bytes(&header)?;
        type_table::validate_portable_artifact_limits_with_initial_string_bytes(
            &artifact,
            header_string_bytes,
        )?;
        Ok(Self {
            header: expected_header.clone(),
            artifact,
        })
    }

    pub fn into_products(self) -> Result<CompilerProducts, String> {
        let products = type_table::artifact_to_products(self.artifact)?;
        if !self.header.matches_products(&products) {
            return Err("Product artifact header does not match semantic payload".to_string());
        }
        Ok(products)
    }
}

fn check_artifact_limit(category: &str, declared: u64, maximum: u64) -> Result<(), String> {
    if declared > maximum {
        return Err(format!(
            "Product artifact {category} limit exceeded: declared {declared}, maximum {maximum}"
        ));
    }
    Ok(())
}

fn artifact_bincode() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
}

#[cfg(test)]
pub(crate) fn artifact_type_table_len_for_test(bytes: &[u8]) -> Result<usize, String> {
    type_table::artifact_type_table_len_for_test(bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilerProducts {
    pub crate_identity: ProductCrateIdentity,
    pub identity_table: ProductIdentityTable,
    pub interface: ProductInterface,
    pub bodies: ProductBodies,
    pub link: ProductLinkData,
    pub dependencies: Vec<ProductDependencyIdentity>,
    pub source_fingerprint: ProductSourceFingerprint,
    pub infix_precedence: BTreeMap<String, u8>,
    pub proc_macros: Vec<crate::macro_expansion::proc_macro::ProcMacroArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProductInterface {
    pub object_abi: object_abi::ProductObjectAbi,
    pub functions: BTreeMap<ProductDefId, ProductFunctionInterface>,
    pub structs: BTreeMap<ProductDefId, ProductStructInterface>,
    pub enums: BTreeMap<ProductDefId, ProductEnumInterface>,
    pub type_aliases: BTreeMap<ProductDefId, ProductTypeAliasInterface>,
    pub traits: BTreeMap<ProductDefId, ProductTraitInterface>,
    pub impls: BTreeMap<ProductDefId, ProductImplInterface>,
    pub externs: BTreeMap<ProductDefId, ProductExternInterface>,
    pub effective_trait_methods: BTreeMap<(ProductDefId, ProductDefId), ProductDefId>,
    #[serde(default)]
    pub language_items: ProductLanguageItems,
}

impl ProductInterface {
    /// Declare a producer trait and its explicit member layout together.
    pub fn insert_trait(
        &mut self,
        id: ProductDefId,
        declaration: ProductTraitInterface,
    ) -> Option<ProductTraitInterface> {
        self.object_abi.trait_members.insert(
            declaration.id,
            object_abi::declaration_member_order(&declaration.signatures),
        );
        self.traits.insert(id, declaration)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductFunctionInterface {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub generic_bounds: crate::hir::HirGenericBounds,
    pub params: Vec<Type>,
    pub ret_type: Type,
    pub is_curried: bool,
    pub is_method: bool,
    pub self_receiver: Option<crate::types::ReceiverMode>,
    pub is_unsafe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductTypeAliasInterface {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<GenericParamDecl>,
    pub ty: Type,
}

impl From<&HirTypeAlias> for ProductTypeAliasInterface {
    fn from(alias: &HirTypeAlias) -> Self {
        Self {
            id: alias.id,
            name: alias.name.clone(),
            generic_params: alias.generic_params.clone(),
            ty: alias.ty.clone(),
        }
    }
}

impl<P: HirPhase> From<&HirFunctionFor<P>> for ProductFunctionInterface {
    fn from(function: &HirFunctionFor<P>) -> Self {
        Self {
            id: function.id,
            name: function.name.clone(),
            generic_params: function.generic_params.clone(),
            generic_bounds: function.generic_bounds.clone(),
            params: function
                .params
                .iter()
                .map(|param| param.ty.clone())
                .collect(),
            ret_type: function.ret_type.clone(),
            is_curried: function.is_curried,
            is_method: function.is_method,
            self_receiver: function.self_receiver,
            is_unsafe: function.is_unsafe,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductFieldInterface {
    pub id: crate::ids::FieldId,
    pub name: String,
    pub ty: Type,
    pub public: bool,
}

impl From<&crate::hir::HirField> for ProductFieldInterface {
    fn from(field: &crate::hir::HirField) -> Self {
        Self {
            id: field.id,
            name: field.name.clone(),
            ty: field.ty.clone(),
            public: field.public,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductStructInterface {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub fields: Vec<ProductFieldInterface>,
}

impl From<&HirStruct> for ProductStructInterface {
    fn from(strukt: &HirStruct) -> Self {
        Self {
            id: strukt.id,
            name: strukt.name.clone(),
            generic_params: strukt.generic_params.clone(),
            fields: strukt
                .fields
                .iter()
                .map(ProductFieldInterface::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductEnumVariantInterface {
    pub id: crate::ids::VariantId,
    pub name: String,
    pub fields: HirVariantFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductEnumInterface {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub variants: Vec<ProductEnumVariantInterface>,
}

impl From<&HirEnum> for ProductEnumInterface {
    fn from(enm: &HirEnum) -> Self {
        Self {
            id: enm.id,
            name: enm.name.clone(),
            generic_params: enm.generic_params.clone(),
            variants: enm
                .variants
                .iter()
                .map(|variant| ProductEnumVariantInterface {
                    id: variant.id,
                    name: variant.name.clone(),
                    fields: variant.fields.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductTraitInterface {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub target: Option<crate::types::GenericParamDecl>,
    pub predicates: Vec<crate::types::Predicate>,
    pub associated_types: Vec<crate::hir::HirAssociatedTypeDecl>,
    pub methods: BTreeMap<String, ProductFunctionInterface>,
    pub signatures: HashMap<String, crate::hir::HirFunctionSig>,
}

impl<P: HirPhase> From<&HirTraitFor<P>> for ProductTraitInterface {
    fn from(trt: &HirTraitFor<P>) -> Self {
        Self {
            id: trt.id,
            name: trt.name.clone(),
            generic_params: trt.generic_params.clone(),
            target: trt.target.clone(),
            predicates: trt.predicates.clone(),
            associated_types: trt.associated_types.clone(),
            methods: trt
                .methods
                .iter()
                .map(|(name, method)| (name.clone(), ProductFunctionInterface::from(method)))
                .collect(),
            signatures: trt.signatures.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductAssociatedTypeInterface {
    pub id: crate::ids::AssocTypeId,
    pub name: String,
    pub kind: crate::type_services::kind::Kind,
    pub ty: Type,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductImplInterface {
    pub id: DefId,
    pub owner: crate::hir::HirImplOwner,
    pub type_name: String,
    pub type_generics: Vec<crate::types::GenericParamDecl>,
    pub receiver_pattern: crate::hir::HirImplReceiverPattern,
    pub trait_name: Option<String>,
    pub trait_id: Option<DefId>,
    pub trait_generics: Vec<crate::types::GenericParamDecl>,
    pub trait_arg_types: Vec<Type>,
    pub associated_types: Vec<ProductAssociatedTypeInterface>,
    pub bounds: HirGenericBounds,
    pub methods: BTreeMap<String, ProductFunctionInterface>,
}

impl From<&HirImplFor<crate::hir::AcceptedHir>> for ProductImplInterface {
    fn from(imp: &HirImplFor<crate::hir::AcceptedHir>) -> Self {
        Self {
            id: imp.id,
            owner: imp.owner.clone(),
            type_name: imp.type_name.clone(),
            type_generics: imp.type_generics.clone(),
            receiver_pattern: imp.receiver_pattern.clone(),
            trait_name: imp.trait_name.clone(),
            trait_id: imp.trait_id,
            trait_generics: imp.trait_generics.clone(),
            trait_arg_types: imp.trait_arg_types.clone(),
            associated_types: imp
                .associated_types
                .iter()
                .map(|assoc| ProductAssociatedTypeInterface {
                    id: assoc.id,
                    name: assoc.name.clone(),
                    kind: assoc.kind.clone(),
                    ty: assoc.ty.clone(),
                })
                .collect(),
            bounds: imp.bounds.clone(),
            methods: imp
                .methods
                .iter()
                .map(|(name, method)| (name.clone(), ProductFunctionInterface::from(method)))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductExternInterface {
    pub id: DefId,
    pub name: String,
    pub params: Vec<Type>,
    pub ret: Type,
    pub variadic: bool,
    #[serde(default)]
    pub is_unsafe: bool,
}

impl From<&HirExtern> for ProductExternInterface {
    fn from(ext: &HirExtern) -> Self {
        Self {
            id: ext.id,
            name: ext.name.clone(),
            params: ext.params.clone(),
            ret: ext.ret.clone(),
            variadic: ext.variadic,
            is_unsafe: ext.is_unsafe,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProductCrateIdentity {
    pub name: String,
    pub version: String,
    pub target_triple: Option<String>,
    pub format_version: u32,
    pub source_fingerprint: ProductSourceFingerprint,
}

impl ProductCrateIdentity {
    pub fn local(name: String) -> Self {
        Self {
            name,
            version: "0.1.0".to_string(),
            target_triple: None,
            format_version: 1,
            source_fingerprint: ProductSourceFingerprint::default(),
        }
    }

    pub fn with_source_fingerprint(mut self, source_fingerprint: ProductSourceFingerprint) -> Self {
        self.source_fingerprint = source_fingerprint;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProductIdentityTable {
    pub local_crate: Option<ProductCrateId>,
    pub dependencies: BTreeMap<ProductCrateId, ProductCrateIdentity>,
    pub display_names: BTreeMap<ProductDefId, String>,
    pub export_names: BTreeMap<String, ProductDefId>,
    #[serde(default)]
    pub import_alias_names: BTreeMap<String, ProductDefId>,
    #[serde(default)]
    pub module_alias_names: BTreeMap<String, ProductDefId>,
    #[serde(default)]
    pub prelude_export_names: BTreeMap<String, ProductDefId>,
    #[serde(default)]
    pub ambiguous_export_names: BTreeSet<String>,
}

pub type ProductLanguageItems = LanguageItems<ProductDefId>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProductBodies {
    pub functions: BTreeMap<ProductDefId, AcceptedHirFunction>,
    pub generic_impls: BTreeMap<ProductDefId, HirImplFor<AcceptedHir>>,
    pub trait_default_methods: BTreeMap<ProductDefId, AcceptedHirFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProductLinkData {
    pub object_path: Option<PathBuf>,
    pub records: BTreeMap<ProductDefId, ProductLinkRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductLinkRecord {
    pub backend_symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductDependencyIdentity {
    pub name: String,
    pub artifact_path: PathBuf,
    pub capabilities: ProductDependencyCapabilities,
}

impl ProductDependencyIdentity {
    pub fn artifact_object(name: String, artifact_path: PathBuf) -> Self {
        Self {
            name,
            artifact_path,
            capabilities: ProductDependencyCapabilities::artifact_object(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductFreshnessMetadata {
    pub compiler_version: String,
    pub artifact_format_version: u32,
    pub target_triple: Option<String>,
    pub config_hash: Option<String>,
    pub features: BTreeSet<String>,
    pub source_fingerprint: ProductSourceFingerprint,
    pub dependencies: Vec<ProductDependencyFreshness>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductDependencyFreshness {
    pub name: String,
    pub artifact_path: PathBuf,
    pub crate_identity: Option<ProductCrateIdentity>,
    pub capabilities: ProductDependencyCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProductDependencyCapabilities {
    pub metadata: bool,
    pub bodies: bool,
    pub link: ProductDependencyLinkCapability,
}

impl ProductDependencyCapabilities {
    pub fn artifact_object() -> Self {
        Self {
            metadata: true,
            bodies: true,
            link: ProductDependencyLinkCapability::Object,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ProductDependencyLinkCapability {
    Object,
    MetadataOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub struct ProductSourceFingerprint {
    pub manifest_hash: Option<String>,
    pub source_hash: Option<String>,
    pub loaded_files: Vec<PathBuf>,
}

fn read_product_artifact_header(
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

fn product_artifact_header_string_bytes(header: &ProductArtifactHeader) -> Result<usize, String> {
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

fn check_product_artifact_string(value: &str, total: &mut usize) -> Result<(), String> {
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

fn decode_product_artifact_payload(
    payload: &[u8],
) -> Result<type_table::SerializedProductArtifact, String> {
    artifact_bincode()
        .with_limit(MAX_PRODUCT_ARTIFACT_BYTES)
        .deserialize(payload)
        .map_err(|error| format!("Failed to deserialize product artifact payload: {error}"))
}

fn encode_product_artifact(
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

fn portable_product_artifact_from_bytes(bytes: &[u8]) -> Result<PortableProductArtifact, String> {
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
fn serialized_artifact_from_bytes_for_test(
    bytes: &[u8],
) -> Result<type_table::SerializedProductArtifact, String> {
    Ok(portable_product_artifact_from_bytes(bytes)?.artifact)
}

impl CompilerProducts {
    /// Attach interface ABI only after MIR has finalized all runtime type IDs.
    pub(crate) fn attach_object_schemas(
        &mut self,
        mir: &crate::mir::MirProgram,
        remap: &ProductIdRemap,
    ) -> Result<(), String> {
        let mut schemas = Vec::new();
        for schema in mir.backend_contract.object_schemas.values() {
            let mut portable =
                object_abi::ProductObjectSchema::from_mir(schema, &mir.type_context)?;
            portable.try_remap_def_ids(&mut |id| {
                strict_object_product_id(id, &self.identity_table, remap)
            })?;
            schemas.push(portable);
        }
        self.interface.object_abi.schemas = schemas;
        self.interface.object_abi.target =
            (!self.interface.object_abi.schemas.is_empty()).then(object_abi::host_object_target);
        self.interface.object_abi.validate_shape()
    }

    pub fn to_artifact_bytes(&self) -> Result<Vec<u8>, String> {
        let artifact = type_table::products_to_artifact(self, PRODUCT_ARTIFACT_FORMAT_VERSION)?;
        let header = ProductArtifactHeader::from_products(self);
        encode_product_artifact(&header, &artifact)
    }

    pub fn from_artifact_bytes(bytes: &[u8]) -> Result<Self, String> {
        portable_product_artifact_from_bytes(bytes)?.into_products()
    }

    pub fn freshness_metadata(&self) -> ProductFreshnessMetadata {
        ProductFreshnessMetadata {
            compiler_version: env!("CARGO_PKG_VERSION").to_string(),
            artifact_format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
            target_triple: self.crate_identity.target_triple.clone(),
            config_hash: None,
            features: BTreeSet::new(),
            source_fingerprint: self.source_fingerprint.clone(),
            dependencies: self
                .dependencies
                .iter()
                .map(|dependency| ProductDependencyFreshness {
                    name: dependency.name.clone(),
                    artifact_path: dependency.artifact_path.clone(),
                    crate_identity: self
                        .identity_table
                        .dependencies
                        .values()
                        .find(|identity| identity.name == dependency.name)
                        .cloned(),
                    capabilities: dependency.capabilities.clone(),
                })
                .collect(),
        }
    }

    pub(crate) fn validate_freshness_metadata(
        &self,
        metadata: &ProductFreshnessMetadata,
    ) -> Result<(), String> {
        let expected = self.freshness_metadata();
        if metadata != &expected {
            return Err(format!(
                "Product artifact freshness metadata mismatch: expected {:?}, found {:?}",
                expected, metadata
            ));
        }

        Ok(())
    }

    pub fn write_artifact_to_path(&self, path: &std::path::Path) -> Result<(), String> {
        let bytes = self.to_artifact_bytes()?;
        let mut file = std::fs::File::create(path).map_err(|err| {
            format!(
                "Failed to write product artifact {}: {}",
                path.display(),
                err
            )
        })?;
        file.write_all(&bytes).map_err(|err| {
            format!(
                "Failed to write product artifact {}: {}",
                path.display(),
                err
            )
        })
    }

    pub fn read_artifact_from_path(path: &std::path::Path) -> Result<Self, String> {
        let header = ProductArtifactHeader::read_from_path_bounded(path)?;
        PortableProductArtifact::read_payload_from_path_bounded(path, &header)
            .and_then(PortableProductArtifact::into_products)
            .map_err(|err| {
                format!(
                    "Failed to deserialize product artifact {}: {}",
                    path.display(),
                    err
                )
            })
    }

    pub fn record_prelude_export_ids(
        &mut self,
        exports: impl IntoIterator<Item = (String, crate::crate_artifact::ArtifactExport)>,
    ) {
        for (alias, export) in exports {
            if let Some(id) = self.product_def_id_for_export(&export) {
                self.identity_table.prelude_export_names.insert(alias, id);
            }
        }
    }

    fn product_def_id_for_export(
        &self,
        export: &crate::crate_artifact::ArtifactExport,
    ) -> Option<ProductDefId> {
        let explicit_id = ProductDefId::from(export.id);
        if let Some(display_name) = self.identity_table.display_names.get(&explicit_id) {
            if self.product_id_is_exportable_prelude_item(explicit_id)
                && self.prelude_export_source_is_compatible(&export.source, display_name)
            {
                return Some(explicit_id);
            }
        }

        self.product_def_id_for_source(&export.source)
    }

    fn prelude_export_source_is_compatible(&self, source: &str, display_name: &str) -> bool {
        source == display_name
            || source == format!("{}::{}", self.crate_identity.name, display_name)
    }

    fn product_def_id_for_source(&self, source: &str) -> Option<ProductDefId> {
        self.identity_table
            .display_names
            .iter()
            .find_map(|(id, name)| {
                let crate_qualified = format!("{}::{}", self.crate_identity.name, name);
                (self.product_id_is_exportable_prelude_item(*id)
                    && (name == source || crate_qualified == source))
                    .then_some(*id)
            })
    }

    fn product_id_is_exportable_prelude_item(&self, id: ProductDefId) -> bool {
        self.interface.functions.contains_key(&id)
            || self.interface.structs.contains_key(&id)
            || self.interface.enums.contains_key(&id)
            || self.interface.traits.contains_key(&id)
            || self.interface.externs.contains_key(&id)
    }

    pub fn from_resolved_hir(
        crate_identity: ProductCrateIdentity,
        hir: &ResolvedHirProgram,
        dependencies: Vec<ProductDependencyIdentity>,
        dependency_crate_identities: BTreeMap<ProductCrateId, ProductCrateIdentity>,
        source_fingerprint: ProductSourceFingerprint,
        link: ProductLinkData,
    ) -> Result<Self, String> {
        Self::from_resolved_hir_with_remap(
            crate_identity,
            hir,
            dependencies,
            dependency_crate_identities,
            source_fingerprint,
            link,
        )
        .map(|(products, _)| products)
    }

    pub(crate) fn from_resolved_hir_with_remap(
        crate_identity: ProductCrateIdentity,
        hir: &ResolvedHirProgram,
        dependencies: Vec<ProductDependencyIdentity>,
        dependency_crate_identities: BTreeMap<ProductCrateId, ProductCrateIdentity>,
        source_fingerprint: ProductSourceFingerprint,
        link: ProductLinkData,
    ) -> Result<(Self, ProductIdRemap), String> {
        let crate_identity = crate_identity.with_source_fingerprint(source_fingerprint.clone());
        let local_crate = ProductCrateId::from(hir.root_crate_id);
        assert_valid_current_crate_hir_ids(hir);
        let mut identity_table = ProductIdentityTable {
            local_crate: Some(local_crate),
            dependencies: dependency_crate_identities,
            ..ProductIdentityTable::default()
        };
        let mut interface = ProductInterface::default();
        let mut bodies = ProductBodies::default();
        let preferred_default_methods = PreferredTraitDefaultMethods::new(&hir.program);
        let fallback_crate = ProductCrateId::from(hir.root_crate_id);
        let crate_name = crate_identity.name.clone();
        let mut next_fallback_local = hir.local_def_ids.next_raw();
        let mut used_ids = BTreeSet::new();
        let mut id_remap = ProductIdRemap::new();
        let mut trait_member_product_ids = BTreeMap::new();
        let mut functions = hir.program.functions_by_id().collect::<Vec<_>>();
        functions.sort_by_key(|(id, name, _)| (*id, (*name).to_string()));
        for (_, name, function) in functions {
            if !is_current_crate_def(hir, function.id) {
                continue;
            }
            let requested_id = ProductDefId::from(function.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_name(&mut identity_table, id, name);
            let mut function = function.clone();
            let original_id = function.id;
            function.id = product_def_id_to_def_id(id);
            remap_function_owned_type_ids(&mut function, original_id);
            interface
                .functions
                .insert(id, ProductFunctionInterface::from(&function));
            if function_requires_downstream_specialization(&function)
                || !hir_function_is_codegen_concrete(&function)
            {
                bodies.functions.insert(id, function);
            }
        }

        let mut structs = hir.program.structs_by_id().collect::<Vec<_>>();
        structs.sort_by_key(|(id, name, _)| (*id, (*name).to_string()));
        for (_, name, strukt) in structs {
            if !is_current_crate_def(hir, strukt.id) {
                continue;
            }
            let requested_id = ProductDefId::from(strukt.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_name(&mut identity_table, id, name);
            let mut strukt = strukt.clone();
            let original_id = strukt.id;
            strukt.id = product_def_id_to_def_id(id);
            remap_struct_owned_type_ids(&mut strukt, original_id);
            interface
                .structs
                .insert(id, ProductStructInterface::from(&strukt));
        }

        let mut enums = hir.program.enums_by_id().collect::<Vec<_>>();
        enums.sort_by_key(|(id, name, _)| (*id, (*name).to_string()));
        for (_, name, enm) in enums {
            if !is_current_crate_def(hir, enm.id) {
                continue;
            }
            let requested_id = ProductDefId::from(enm.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_name(&mut identity_table, id, name);
            let mut enm = enm.clone();
            let original_id = enm.id;
            enm.id = product_def_id_to_def_id(id);
            remap_enum_owned_type_ids(&mut enm, original_id);
            interface.enums.insert(id, ProductEnumInterface::from(&enm));
        }

        let mut type_aliases = hir.program.type_aliases.iter().collect::<Vec<_>>();
        type_aliases.sort_by_key(|(id, alias)| (**id, alias.name.clone()));
        for (_, alias) in type_aliases {
            if !is_current_crate_def(hir, alias.id) {
                continue;
            }
            let requested_id = ProductDefId::from(alias.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_name(&mut identity_table, id, &alias.name);
            let mut alias = alias.clone();
            let original_id = alias.id;
            alias.id = product_def_id_to_def_id(id);
            remap_type_alias_owned_type_ids(&mut alias, original_id);
            interface
                .type_aliases
                .insert(id, ProductTypeAliasInterface::from(&alias));
        }

        let mut traits = hir.program.traits_by_id().collect::<Vec<_>>();
        traits.sort_by_key(|(id, name, _)| (*id, (*name).to_string()));
        for (_, name, trt) in traits {
            if !is_current_crate_def(hir, trt.id) {
                continue;
            }
            let requested_id = ProductDefId::from(trt.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_name(&mut identity_table, id, name);
            let mut trt = trt.clone();
            let mut signature_product_ids = BTreeMap::new();
            let mut signature_names = trt.signatures.keys().cloned().collect::<Vec<_>>();
            signature_names.sort();
            for signature_name in signature_names {
                let sig = trt
                    .signatures
                    .get_mut(&signature_name)
                    .expect("signature name collected from trait signature map");
                let original_signature_id = sig.id;
                let requested_signature_id = ProductDefId::from(original_signature_id);
                let signature_id = reserve_product_id(
                    requested_signature_id,
                    &mut used_ids,
                    fallback_crate,
                    &mut next_fallback_local,
                    false,
                );
                record_id_remap(&mut id_remap, requested_signature_id, signature_id);
                trait_member_product_ids.insert(original_signature_id, signature_id);
                signature_product_ids.insert(original_signature_id, signature_id);
                record_display_name(
                    &mut identity_table,
                    signature_id,
                    &format!("{}::{}", name, signature_name),
                );
                sig.id = product_def_id_to_def_id(signature_id);
            }
            let mut method_names = trt.methods.keys().cloned().collect::<Vec<_>>();
            method_names.sort();
            for method_name in method_names {
                let method = &trt.methods[&method_name];
                let original_method_id = method.id;
                let requested_method_id = ProductDefId::from(original_method_id);
                // A default body and its signature describe one canonical member.
                let method_id = if let Some(id) = signature_product_ids.get(&original_method_id) {
                    *id
                } else {
                    reserve_product_id(
                        requested_method_id,
                        &mut used_ids,
                        fallback_crate,
                        &mut next_fallback_local,
                        false,
                    )
                };
                record_id_remap(&mut id_remap, requested_method_id, method_id);
                trait_member_product_ids.insert(original_method_id, method_id);
                record_display_name(
                    &mut identity_table,
                    method_id,
                    &format!("{}::{}", name, method_name),
                );
                let mut method =
                    preferred_default_methods.get(original_method_id, &trt.name, &method_name);
                method.id = product_def_id_to_def_id(method_id);
                remap_function_owned_type_ids(&mut method, original_method_id);
                bodies
                    .trait_default_methods
                    .insert(method_id, method.clone());
                trt.methods.insert(method_name, method);
            }
            let original_id = trt.id;
            trt.id = product_def_id_to_def_id(id);
            remap_trait_owned_type_ids(&mut trt, original_id);
            trt.name = qualify_metadata_name(&crate_name, name);
            interface
                .traits
                .insert(id, ProductTraitInterface::from(&trt));
        }

        let mut impls = hir.program.impls_by_id().collect::<Vec<_>>();
        let mut seen_impl_ids = impls.iter().map(|(id, _, _)| *id).collect::<BTreeSet<_>>();
        for (index, (id, imp)) in hir.program.impls_in_order().enumerate() {
            if seen_impl_ids.insert(imp.id) {
                impls.push((id, index, imp));
            }
        }
        impls.sort_by_key(|(id, index, _)| (*id, *index));
        impls.dedup_by_key(|(id, index, _)| (*id, *index));
        for (_, _, imp) in impls {
            if !is_current_crate_def(hir, imp.id) {
                continue;
            }
            let requested_id = ProductDefId::from(imp.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_display_name(&mut identity_table, id, &impl_display_name(imp));
            let mut imp = imp.clone();
            let impl_name = impl_display_name(&imp);
            let original_id = imp.id;
            let mut impl_method_product_ids = BTreeMap::new();
            imp.id = product_def_id_to_def_id(id);
            let mut method_names = imp.methods.keys().cloned().collect::<Vec<_>>();
            method_names.sort();
            for method_name in method_names {
                let method = imp
                    .methods
                    .get_mut(&method_name)
                    .expect("method name collected from impl method map");
                let original_method_id = method.id;
                let requested_method_id = ProductDefId::from(original_method_id);
                let method_id = reserve_product_id(
                    requested_method_id,
                    &mut used_ids,
                    fallback_crate,
                    &mut next_fallback_local,
                    false,
                );
                record_id_remap(&mut id_remap, requested_method_id, method_id);
                impl_method_product_ids.insert(original_method_id, method_id);
                record_display_name(
                    &mut identity_table,
                    method_id,
                    &format!("{}::{}", impl_name, method_name),
                );
                method.id = product_def_id_to_def_id(method_id);
                remap_function_owned_type_ids(method, original_method_id);
            }
            for (&(impl_id, trait_member_id), &impl_method_id) in
                &hir.program.indexes.effective_trait_methods
            {
                if impl_id != original_id {
                    continue;
                }
                let trait_member_id = trait_member_product_ids
                    .get(&trait_member_id)
                    .copied()
                    .unwrap_or_else(|| ProductDefId::from(trait_member_id));
                let impl_method_id =
                    *impl_method_product_ids
                        .get(&impl_method_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "effective impl method {:?} has no product identity on impl {:?}",
                                impl_method_id, original_id
                            )
                        });
                interface
                    .effective_trait_methods
                    .insert((id, trait_member_id), impl_method_id);
            }
            remap_impl_owned_type_ids(&mut imp, original_id);
            if impl_requires_downstream_specialization(&imp) {
                bodies.generic_impls.insert(id, imp.clone());
            }
            interface.impls.insert(id, ProductImplInterface::from(&imp));
        }

        let mut externs = hir.program.externs_by_id().collect::<Vec<_>>();
        let mut seen_extern_ids = externs
            .iter()
            .map(|(id, _, _)| *id)
            .collect::<BTreeSet<_>>();
        for (index, (id, ext)) in hir.program.externs_in_order().enumerate() {
            if seen_extern_ids.insert(ext.id) {
                externs.push((id, index, ext));
            }
        }
        externs.sort_by_key(|(id, _, ext)| (*id, ext.name.clone()));
        for (_, _, ext) in externs {
            if !is_current_crate_def(hir, ext.id) {
                continue;
            }
            let requested_id = ProductDefId::from(ext.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_name(&mut identity_table, id, &ext.name);
            let mut ext = ext.clone();
            let original_id = ext.id;
            ext.id = product_def_id_to_def_id(id);
            remap_extern_owned_type_ids(&mut ext, original_id);
            ext.name = qualify_metadata_name(&crate_name, &ext.name);
            interface
                .externs
                .insert(id, ProductExternInterface::from(&ext));
        }

        remap_product_type_def_ids(&mut interface, &mut bodies, &id_remap);
        // Declaration rows already own their reserved product identities; a
        // global source-ID remap may be ambiguous after an identity collision.
        for declaration in interface.traits.values() {
            let members = object_abi::declaration_member_order(&declaration.signatures);
            interface
                .object_abi
                .trait_members
                .insert(declaration.id, members);
        }
        interface.language_items =
            product_language_items_from_program(&hir.program, &hir.current_def_ids, &id_remap)?;
        record_product_aliases(
            &mut identity_table.import_alias_names,
            &hir.resolver.import_aliases,
            &id_remap,
        );
        record_product_aliases(
            &mut identity_table.export_names,
            &hir.resolver.export_aliases,
            &id_remap,
        );
        record_product_aliases(
            &mut identity_table.module_alias_names,
            &hir.resolver.module_aliases,
            &id_remap,
        );

        let link = remap_link_data(link, &id_remap);

        Ok((
            Self {
                crate_identity,
                identity_table,
                interface,
                bodies,
                link,
                dependencies,
                source_fingerprint,
                infix_precedence: BTreeMap::new(),
                proc_macros: Vec::new(),
            },
            id_remap,
        ))
    }
}

fn is_current_crate_def(hir: &ResolvedHirProgram, id: DefId) -> bool {
    hir.current_def_ids.contains(&id)
}

fn product_language_items_from_program<P: HirPhase>(
    program: &HirProgramFor<P>,
    current_def_ids: &BTreeSet<DefId>,
    id_remap: &ProductIdRemap,
) -> Result<ProductLanguageItems, String> {
    let language_items = &program.language_items;
    let object_owner = language_items
        .object_owner
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "object_owner",
                &[items.trait_id, items.into_parts_id, items.from_parts_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| crate::language_items::ObjectOwnerLanguageItems {
                    trait_id: ids[0],
                    into_parts_id: ids[1],
                    from_parts_id: ids[2],
                })
            })
        })
        .transpose()?
        .flatten();
    let owner_operation =
        |name, items: &crate::language_items::OwnerOperationLanguageItems<DefId>| {
            map_product_language_item_bundle(
                name,
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| crate::language_items::OwnerOperationLanguageItems {
                    trait_id: ids[0],
                    method_id: ids[1],
                })
            })
        };
    let object_owner_allocate = language_items
        .object_owner_allocate
        .as_ref()
        .map(|items| owner_operation("object_owner_allocate", items))
        .transpose()?
        .flatten();
    let object_owner_unique = language_items
        .object_owner_unique
        .as_ref()
        .map(|items| owner_operation("object_owner_unique", items))
        .transpose()?
        .flatten();
    let sized = language_items
        .sized
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle("sized", &[items.trait_id], current_def_ids, id_remap)
                .map(|ids| ids.map(|ids| SizedLanguageItems { trait_id: ids[0] }))
        })
        .transpose()?
        .flatten();
    let drop = language_items
        .drop
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "drop",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| DropLanguageItems {
                    trait_id: ids[0],
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    let fold = language_items
        .fold
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "fold",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| FoldLanguageItems {
                    trait_id: ids[0],
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    let index = language_items
        .index
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "index",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| IndexLanguageItems {
                    trait_id: ids[0],
                    output_id: items.output_id,
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    let index_mut = language_items
        .index_mut
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "index_mut",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| IndexMutLanguageItems {
                    trait_id: ids[0],
                    output_id: items.output_id,
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    if language_items.index_mut.is_some() && index.is_some() != index_mut.is_some() {
        return Err(
            "language item index/index_mut provider pair mixes current-crate and dependency definitions"
                .to_string(),
        );
    }
    let fn_once = language_items
        .fn_once
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "fn_once",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| FnOnceLanguageItems {
                    trait_id: ids[0],
                    output_id: items.output_id,
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    let fn_mut = language_items
        .fn_mut
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "fn_mut",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| FnMutLanguageItems {
                    trait_id: ids[0],
                    output_id: items.output_id,
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    let fn_trait = language_items
        .fn_trait
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "fn",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| FnLanguageItems {
                    trait_id: ids[0],
                    output_id: items.output_id,
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    let send = language_items
        .send
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle("send", &[items.trait_id], current_def_ids, id_remap)
                .map(|ids| ids.map(|ids| SendLanguageItems { trait_id: ids[0] }))
        })
        .transpose()?
        .flatten();
    let sync = language_items
        .sync
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle("sync", &[items.trait_id], current_def_ids, id_remap)
                .map(|ids| ids.map(|ids| SyncLanguageItems { trait_id: ids[0] }))
        })
        .transpose()?
        .flatten();
    let try_protocol = language_items
        .try_protocol
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "try",
                &[
                    items.try_trait_id,
                    items.branch_method_id,
                    items.from_residual_trait_id,
                    items.from_residual_method_id,
                    items.control_flow_enum_id,
                ],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| TryLanguageItems {
                    try_trait_id: ids[0],
                    output_id: items.output_id,
                    residual_id: items.residual_id,
                    branch_method_id: ids[1],
                    from_residual_trait_id: ids[2],
                    from_residual_method_id: ids[3],
                    control_flow_enum_id: ids[4],
                    break_variant_id: items.break_variant_id,
                    continue_variant_id: items.continue_variant_id,
                })
            })
        })
        .transpose()?
        .flatten();
    let range = language_items
        .range
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle("range", &[items.enum_id], current_def_ids, id_remap)
                .map(|ids| {
                    ids.map(|ids| RangeLanguageItems {
                        enum_id: ids[0],
                        full_variant_id: items.full_variant_id,
                        from_variant_id: items.from_variant_id,
                        to_variant_id: items.to_variant_id,
                        to_inclusive_variant_id: items.to_inclusive_variant_id,
                        exclusive_variant_id: items.exclusive_variant_id,
                        inclusive_variant_id: items.inclusive_variant_id,
                    })
                })
        })
        .transpose()?
        .flatten();

    Ok(ProductLanguageItems {
        object_owner,
        object_owner_allocate,
        object_owner_unique,
        fold,
        sized,
        drop,
        index,
        index_mut,
        fn_once,
        fn_mut,
        fn_trait,
        send,
        sync,
        try_protocol,
        range,
    })
}

fn map_product_language_item_bundle(
    bundle: &str,
    ids: &[DefId],
    current_def_ids: &BTreeSet<DefId>,
    id_remap: &ProductIdRemap,
) -> Result<Option<Vec<ProductDefId>>, String> {
    let current = ids
        .iter()
        .map(|id| current_def_ids.contains(id))
        .collect::<Vec<_>>();
    if current.iter().all(|current| *current) {
        return ids
            .iter()
            .map(|id| {
                unambiguous_product_id(ProductDefId::from(*id), id_remap).ok_or_else(|| {
                    format!(
                        "language item {bundle} bundle has no unambiguous product ID for {id:?}"
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some);
    }
    if current.iter().all(|current| !*current) {
        return Ok(None);
    }

    Err(format!(
        "language item {bundle} bundle mixes current-crate and dependency definitions"
    ))
}

fn assert_valid_current_crate_hir_ids(hir: &ResolvedHirProgram) {
    for id in &hir.current_def_ids {
        assert_valid_product_input_def_id(*id);
    }

    for function in hir.program.functions.values() {
        if is_current_crate_def(hir, function.id) {
            assert_valid_product_function_ids(function);
        }
    }
    for strukt in hir.program.structs.values() {
        if is_current_crate_def(hir, strukt.id) {
            assert_valid_product_struct_ids(strukt);
        }
    }
    for enm in hir.program.enums.values() {
        if is_current_crate_def(hir, enm.id) {
            assert_valid_product_enum_ids(enm);
        }
    }
    for alias in hir.program.type_aliases.values() {
        if is_current_crate_def(hir, alias.id) {
            assert_valid_product_input_def_id(alias.id);
            assert_valid_product_generic_param_decls(&alias.generic_params);
            assert_valid_product_type_ids(&alias.ty);
        }
    }
    for trait_def in hir.program.traits.values() {
        if is_current_crate_def(hir, trait_def.id) {
            assert_valid_product_trait_ids(trait_def);
        }
    }
    for imp in hir.program.impls.values() {
        if is_current_crate_def(hir, imp.id) {
            assert_valid_product_impl_ids(imp);
        }
    }
    for ext in hir.program.externs.values() {
        if is_current_crate_def(hir, ext.id) {
            assert_valid_product_extern_ids(ext);
        }
    }
}

fn assert_valid_product_input_def_id(id: DefId) {
    if id.crate_id == CrateId(u32::MAX) || id.local == LocalDefId(u32::MAX) {
        panic!(
            "compiler product emission received invalid current-crate DefId {:?}",
            id
        );
    }
}

fn assert_valid_product_type_ids(ty: &crate::types::Type) {
    use crate::types::Type;

    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| match nested {
        Type::Object(object) => {
            for bound in std::iter::once(&object.principal)
                .chain(&object.guarantees)
                .chain(object.bindings.iter().map(|binding| &binding.key.trait_ref))
            {
                assert_valid_product_input_def_id(bound.trait_id);
            }
        }
        Type::Struct { id, .. } | Type::Enum { id, .. } | Type::Constructor { id, .. } => {
            assert_valid_product_input_def_id(*id)
        }
        Type::Generic(param) => assert_valid_product_input_def_id(param.owner),
        Type::Witness(witness) => assert_valid_product_input_def_id(witness.owner),
        Type::Projection {
            trait_id,
            assoc_type,
            ..
        } => {
            assert_valid_product_input_def_id(*trait_id);
            assert_valid_product_input_def_id(assoc_type.owner);
        }
        _ => {}
    });
}

fn assert_valid_product_generic_param_decls(decls: &[GenericParamDecl]) {
    for decl in decls {
        assert_valid_product_input_def_id(decl.id.owner);
    }
}

fn assert_valid_product_generic_bounds(bounds: &crate::hir::HirGenericBounds) {
    for (generic_id, trait_bounds) in bounds {
        assert_valid_product_input_def_id(generic_id.owner);
        for bound in trait_bounds {
            assert_valid_product_input_def_id(bound.trait_id);
            for ty in &bound.type_args {
                assert_valid_product_type_ids(ty);
            }
        }
    }
    assert_valid_product_predicates(&bounds.predicates);
}

fn assert_valid_product_predicates(predicates: &[crate::types::Predicate]) {
    for predicate in predicates {
        match predicate {
            crate::types::Predicate::Trait {
                subject,
                trait_id,
                args,
            } => {
                assert_valid_product_input_def_id(*trait_id);
                assert_valid_product_type_ids(subject);
                for arg in args {
                    assert_valid_product_type_ids(arg);
                }
            }
        }
    }
}

fn assert_valid_product_function_ids<P: HirPhase>(function: &HirFunctionFor<P>) {
    assert_valid_product_input_def_id(function.id);
    assert_valid_product_generic_param_decls(&function.generic_params);
    assert_valid_product_generic_bounds(&function.generic_bounds);
    for param in &function.params {
        assert_valid_product_type_ids(&param.ty);
    }
    assert_valid_product_type_ids(&function.ret_type);
    assert_valid_product_block_ids(&function.body);
}

fn assert_valid_product_function_sig_ids(sig: &crate::hir::HirFunctionSig) {
    assert_valid_product_input_def_id(sig.id);
    assert_valid_product_generic_param_decls(&sig.generic_params);
    assert_valid_product_generic_bounds(&sig.generic_bounds);
    for param in &sig.params {
        assert_valid_product_type_ids(param);
    }
    assert_valid_product_type_ids(&sig.ret);
}

fn assert_valid_product_block_ids<P: HirPhase>(block: &crate::hir::HirBlockFor<P>) {
    assert_valid_product_type_ids(&block.ty);
    for stmt in &block.stmts {
        assert_valid_product_stmt_ids(stmt);
    }
}

fn assert_valid_product_stmt_ids<P: HirPhase>(stmt: &crate::hir::HirStmtFor<P>) {
    match stmt {
        crate::hir::HirStmtFor::Let { ty, value, .. } => {
            assert_valid_product_type_ids(ty);
            assert_valid_product_expr_ids(value);
        }
        crate::hir::HirStmtFor::Expr(expr) => assert_valid_product_expr_ids(expr),
        crate::hir::HirStmtFor::Return(expr) | crate::hir::HirStmtFor::Break(expr) => {
            if let Some(expr) = expr {
                assert_valid_product_expr_ids(expr);
            }
        }
        crate::hir::HirStmtFor::Continue => {}
    }
}

fn assert_valid_product_expr_ids<P: HirPhase>(expr: &crate::hir::HirExprFor<P>) {
    assert_valid_product_type_ids(&expr.ty);
    match &expr.kind {
        crate::hir::HirExprKindFor::Open { source, binding, body } => {
            assert_valid_product_expr_ids(source);
            binding.visit_types(&mut assert_valid_product_type_ids);
            if let Some(owner) = &binding.owner {
                assert_valid_product_method_target(&owner.into_parts.method);
                assert_valid_product_method_target(&owner.from_parts.method);
            }
            assert_valid_product_block_ids(body);
        }
        crate::hir::HirExprKindFor::OwnedObjectCall { owner, args, call } => {
            assert_valid_product_expr_ids(owner);
            for arg in args {
                assert_valid_product_expr_ids(arg);
            }
            call.visit_types(&mut assert_valid_product_type_ids);
            for method in [&call.method, &call.into_parts.method, &call.release.method] {
                assert_valid_product_method_target(method);
            }
        }
        crate::hir::HirExprKindFor::ObjectCoercion(value, coercion) => {
            assert_valid_product_expr_ids(value);
            coercion.visit_types(&mut |ty| assert_valid_product_type_ids(ty));
            if let Some(crate::hir::HirObjectEvidence::Concrete { views, .. }) = &coercion.evidence
            {
                for view in views {
                    assert_valid_product_input_def_id(view.trait_ref.trait_id);
                    if let crate::hir::HirObjectWitnessOrigin::Impl {
                        impl_id,
                        substitution,
                    } = &view.origin
                    {
                        assert_valid_product_input_def_id(*impl_id);
                        for binding in substitution {
                            assert_valid_product_input_def_id(binding.param.owner);
                        }
                    }
                }
            }
        }
        crate::hir::HirExprKindFor::ArrayLiteral(elems)
        | crate::hir::HirExprKindFor::TupleLiteral(elems) => {
            for elem in elems {
                assert_valid_product_expr_ids(elem);
            }
        }
        crate::hir::HirExprKindFor::ArrayRepeat(value, _) => {
            assert_valid_product_expr_ids(value);
        }
        crate::hir::HirExprKindFor::FieldAccess(inner, _, location) => {
            assert_valid_product_expr_ids(inner);
            if let Some(location) = location {
                assert_valid_product_input_def_id(location.owner);
            }
        }
        crate::hir::HirExprKindFor::TupleIndex(inner, _)
        | crate::hir::HirExprKindFor::UnaryOp(_, inner)
        | crate::hir::HirExprKindFor::Ref(_, inner)
        | crate::hir::HirExprKindFor::Deref(inner) => assert_valid_product_expr_ids(inner),
        crate::hir::HirExprKindFor::BinOp(_, lhs, rhs)
        | crate::hir::HirExprKindFor::Assign(lhs, rhs) => {
            assert_valid_product_expr_ids(lhs);
            assert_valid_product_expr_ids(rhs);
        }
        crate::hir::HirExprKindFor::Call(func, args, target) => {
            if let Some(target) = target {
                assert_valid_product_call_target(target);
            }
            assert_valid_product_expr_ids(func);
            for arg in args {
                assert_valid_product_expr_ids(arg);
            }
        }
        crate::hir::HirExprKindFor::MethodCall(func, _, args, _, target) => {
            if let Some(target) = P::method_authority(target) {
                assert_valid_product_method_target(target);
            }
            assert_valid_product_expr_ids(func);
            for arg in args {
                assert_valid_product_expr_ids(arg);
            }
        }
        crate::hir::HirExprKindFor::Try {
            expr,
            branch_method,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            control_flow_enum,
            break_variant,
            continue_variant,
            ..
        } => {
            assert_valid_product_expr_ids(expr);
            if let Some(target) = P::method_authority(branch_method) {
                assert_valid_product_method_target(target);
            }
            if let Some(target) = P::residual_authority(from_residual_target) {
                assert_valid_product_call_target(target);
            }
            assert_valid_product_type_ids(output_ty);
            assert_valid_product_type_ids(residual_ty);
            assert_valid_product_type_ids(return_ty);
            assert_valid_product_input_def_id(*control_flow_enum);
            assert_valid_product_input_def_id(break_variant.owner);
            assert_valid_product_input_def_id(continue_variant.owner);
        }
        crate::hir::HirExprKindFor::StructLiteral(_, struct_id, fields) => {
            if let Some(struct_id) = struct_id {
                assert_valid_product_input_def_id(*struct_id);
            }
            for field in fields {
                if let Some(location) = &field.field {
                    assert_valid_product_input_def_id(location.owner);
                }
                assert_valid_product_expr_ids(&field.value);
            }
        }
        crate::hir::HirExprKindFor::EnumVariant(_, _, args, location) => {
            if let Some(location) = location {
                assert_valid_product_input_def_id(location.owner);
            }
            for arg in args {
                assert_valid_product_expr_ids(arg);
            }
        }
        crate::hir::HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            assert_valid_product_expr_ids(condition);
            assert_valid_product_block_ids(then_branch);
            if let Some(else_branch) = else_branch {
                assert_valid_product_block_ids(else_branch);
            }
        }
        crate::hir::HirExprKindFor::Match { scrutinee, arms } => {
            assert_valid_product_expr_ids(scrutinee);
            for arm in arms {
                assert_valid_product_pattern_ids(&arm.pattern);
                if let Some(guard) = &arm.guard {
                    assert_valid_product_expr_ids(guard);
                }
                assert_valid_product_block_ids(&arm.body);
            }
        }
        crate::hir::HirExprKindFor::While { condition, body } => {
            assert_valid_product_expr_ids(condition);
            assert_valid_product_block_ids(body);
        }
        crate::hir::HirExprKindFor::For { iter, body, .. } => {
            assert_valid_product_expr_ids(iter);
            assert_valid_product_block_ids(body);
        }
        crate::hir::HirExprKindFor::Loop(body)
        | crate::hir::HirExprKindFor::Block(body)
        | crate::hir::HirExprKindFor::UnsafeBlock(body) => {
            assert_valid_product_block_ids(body);
        }
        crate::hir::HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => {
            for param in params {
                assert_valid_product_type_ids(&param.ty);
            }
            for capture in captures {
                assert_valid_product_type_ids(&capture.ty);
            }
            assert_valid_product_block_ids(body);
        }
        crate::hir::HirExprKindFor::Cast(inner, ty) => {
            assert_valid_product_expr_ids(inner);
            assert_valid_product_type_ids(ty);
        }
        crate::hir::HirExprKindFor::ResolvedVar(reference) => match reference.target {
            HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
                assert_valid_product_input_def_id(id);
            }
            HirVarTarget::Instance(_) | HirVarTarget::Local(_) => {}
        },
        crate::hir::HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                assert_valid_product_expr_ids(arg);
            }
        }
        crate::hir::HirExprKindFor::IntLiteral(_)
        | crate::hir::HirExprKindFor::FloatLiteral(_)
        | crate::hir::HirExprKindFor::BoolLiteral(_)
        | crate::hir::HirExprKindFor::StringLiteral(_)
        | crate::hir::HirExprKindFor::CharLiteral(_)
        | crate::hir::HirExprKindFor::Unit
        | crate::hir::HirExprKindFor::Var(_) => {}
    }
}

fn assert_valid_product_call_target(target: &HirCallTarget) {
    match target {
        HirCallTarget::Function(id) | HirCallTarget::Extern(id) => {
            assert_valid_product_input_def_id(*id);
        }
        HirCallTarget::StaticMethod(target) => {
            assert_valid_product_type_ids(&target.owner_ty);
            assert_valid_product_method_target(&target.method);
        }
        HirCallTarget::Instance(_) | HirCallTarget::Local(_) | HirCallTarget::Intrinsic(_) => {}
    }
}

fn assert_valid_product_method_target(target: &HirMethodCallTarget) {
    match &target.target {
        HirSelectedMethodTarget::ImplMethod {
            impl_id,
            method_id,
            selected_trait,
        } => {
            assert_valid_product_input_def_id(*impl_id);
            assert_valid_product_input_def_id(*method_id);
            if let Some(selected_trait) = selected_trait {
                assert_valid_product_input_def_id(selected_trait.trait_id);
                assert_valid_product_input_def_id(selected_trait.member_id);
            }
        }
        HirSelectedMethodTarget::TraitMethod {
            trait_id,
            member_id,
            dispatch,
            ..
        } => {
            assert_valid_product_input_def_id(*trait_id);
            assert_valid_product_input_def_id(*member_id);
            if let crate::hir::HirTraitDispatchKind::Opened(witness) = dispatch {
                assert_valid_product_input_def_id(witness.owner);
            }
        }
    }
    for arg in target.trait_args() {
        assert_valid_product_type_ids(arg);
    }
    for binding in target
        .owner_substitution
        .iter()
        .chain(target.method_substitution.iter())
    {
        assert_valid_product_input_def_id(binding.param.owner);
        assert_valid_product_type_ids(&binding.ty);
    }
}

fn assert_valid_product_pattern_ids(pattern: &HirPattern) {
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                assert_valid_product_pattern_ids(pattern);
            }
        }
        HirPattern::Struct(_, struct_id, args, fields) => {
            if let Some(struct_id) = struct_id {
                assert_valid_product_input_def_id(*struct_id);
            }
            for arg in args {
                assert_valid_product_type_ids(arg);
            }
            for field in fields {
                if let Some(location) = &field.field {
                    assert_valid_product_input_def_id(location.owner);
                }
                assert_valid_product_pattern_ids(&field.pattern);
            }
        }
        HirPattern::Enum(_, _, location, patterns) => {
            if let Some(location) = location {
                assert_valid_product_input_def_id(location.owner);
            }
            for pattern in patterns {
                assert_valid_product_pattern_ids(pattern);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

fn assert_valid_product_struct_ids(strukt: &HirStruct) {
    assert_valid_product_input_def_id(strukt.id);
    assert_valid_product_generic_param_decls(&strukt.generic_params);
    for field in &strukt.fields {
        assert_valid_product_type_ids(&field.ty);
    }
}

fn assert_valid_product_enum_ids(enm: &HirEnum) {
    assert_valid_product_input_def_id(enm.id);
    assert_valid_product_generic_param_decls(&enm.generic_params);
    for variant in &enm.variants {
        match &variant.fields {
            HirVariantFields::Named(fields) => {
                for field in fields {
                    assert_valid_product_type_ids(&field.ty);
                }
            }
            HirVariantFields::Positional(types) => {
                for ty in types {
                    assert_valid_product_type_ids(ty);
                }
            }
            HirVariantFields::Unit => {}
        }
    }
}

fn assert_valid_product_trait_ids<P: HirPhase>(trait_def: &HirTraitFor<P>) {
    assert_valid_product_input_def_id(trait_def.id);
    assert_valid_product_generic_param_decls(&trait_def.generic_params);
    if let Some(target) = &trait_def.target {
        assert_valid_product_generic_param_decls(std::slice::from_ref(target));
    }
    assert_valid_product_predicates(&trait_def.predicates);
    for method in trait_def.methods.values() {
        assert_valid_product_function_ids(method);
    }
    for sig in trait_def.signatures.values() {
        assert_valid_product_function_sig_ids(sig);
    }
}

fn assert_valid_product_impl_ids<P: HirPhase>(imp: &HirImplFor<P>) {
    assert_valid_product_input_def_id(imp.id);
    assert_valid_product_generic_param_decls(&imp.type_generics);
    assert_valid_product_generic_param_decls(&imp.trait_generics);
    if let Some(trait_id) = imp.trait_id {
        assert_valid_product_input_def_id(trait_id);
    }
    P::visit_impl_receiver_types(&imp.receiver_pattern, &mut assert_valid_product_type_ids);
    for ty in &imp.trait_arg_types {
        assert_valid_product_type_ids(ty);
    }
    for associated in &imp.associated_types {
        assert_valid_product_type_ids(&associated.ty);
    }
    for (param, bounds) in &imp.bounds {
        assert_valid_product_input_def_id(param.owner);
        for bound in bounds {
            assert_valid_product_input_def_id(bound.trait_id);
            for ty in &bound.type_args {
                assert_valid_product_type_ids(ty);
            }
        }
    }
    for method in imp.methods.values() {
        assert_valid_product_function_ids(method);
    }
}

fn assert_valid_product_extern_ids(ext: &HirExtern) {
    assert_valid_product_input_def_id(ext.id);
    for param in &ext.params {
        assert_valid_product_type_ids(param);
    }
    assert_valid_product_type_ids(&ext.ret);
}

fn remap_product_type_def_ids(
    interface: &mut ProductInterface,
    bodies: &mut ProductBodies,
    id_remap: &ProductIdRemap,
) {
    let mut remap_type = |ty: &mut crate::types::Type| {
        ty.remap_def_ids(&mut |id| {
            unambiguous_product_id(ProductDefId::from(id), id_remap)
                .map(product_def_id_to_def_id)
                .unwrap_or(id)
        });
    };

    for function in interface.functions.values_mut() {
        remap_generic_param_decls_product_ids(&mut function.generic_params, id_remap);
        remap_generic_bounds_product_ids(&mut function.generic_bounds, id_remap, &mut remap_type);
        remap_function_interface_signature_types(function, &mut remap_type);
    }
    for function in bodies.functions.values_mut() {
        remap_generic_param_decls_product_ids(&mut function.generic_params, id_remap);
        remap_generic_bounds_product_ids(&mut function.generic_bounds, id_remap, &mut remap_type);
        remap_function_signature_types(function, &mut remap_type);
        remap_function_body_types(function, &mut remap_type);
        remap_function_body_location_product_ids(function, id_remap);
    }
    for function in bodies.trait_default_methods.values_mut() {
        remap_generic_param_decls_product_ids(&mut function.generic_params, id_remap);
        remap_generic_bounds_product_ids(&mut function.generic_bounds, id_remap, &mut remap_type);
        remap_function_signature_types(function, &mut remap_type);
        remap_function_body_types(function, &mut remap_type);
        remap_function_body_location_product_ids(function, id_remap);
    }

    for strukt in interface.structs.values_mut() {
        remap_generic_param_decls_product_ids(&mut strukt.generic_params, id_remap);
        for field in &mut strukt.fields {
            remap_type(&mut field.ty);
        }
    }

    for enm in interface.enums.values_mut() {
        remap_generic_param_decls_product_ids(&mut enm.generic_params, id_remap);
        for variant in &mut enm.variants {
            match &mut variant.fields {
                HirVariantFields::Named(fields) => {
                    for field in fields {
                        remap_type(&mut field.ty);
                    }
                }
                HirVariantFields::Positional(types) => {
                    for ty in types {
                        remap_type(ty);
                    }
                }
                HirVariantFields::Unit => {}
            }
        }
    }

    for alias in interface.type_aliases.values_mut() {
        remap_generic_param_decls_product_ids(&mut alias.generic_params, id_remap);
        remap_type(&mut alias.ty);
    }

    for trait_ in interface.traits.values_mut() {
        remap_generic_param_decls_product_ids(&mut trait_.generic_params, id_remap);
        if let Some(target) = &mut trait_.target {
            remap_generic_param_decls_product_ids(std::slice::from_mut(target), id_remap);
        }
        for predicate in &mut trait_.predicates {
            predicate.remap_def_ids(&mut |id| {
                unambiguous_product_id(ProductDefId::from(id), id_remap)
                    .map(product_def_id_to_def_id)
                    .unwrap_or(id)
            });
        }
        for method in trait_.methods.values_mut() {
            remap_generic_param_decls_product_ids(&mut method.generic_params, id_remap);
            remap_generic_bounds_product_ids(&mut method.generic_bounds, id_remap, &mut remap_type);
            remap_function_interface_signature_types(method, &mut remap_type);
        }
        for sig in trait_.signatures.values_mut() {
            remap_generic_param_decls_product_ids(&mut sig.generic_params, id_remap);
            remap_generic_bounds_product_ids(&mut sig.generic_bounds, id_remap, &mut remap_type);
            for param in &mut sig.params {
                remap_type(param);
            }
            remap_type(&mut sig.ret);
        }
    }

    for imp in interface.impls.values_mut() {
        remap_generic_param_decls_product_ids(&mut imp.type_generics, id_remap);
        remap_generic_param_decls_product_ids(&mut imp.trait_generics, id_remap);
        remap_impl_interface_generic_param_decls_product_ids(imp, id_remap, &mut remap_type);
        remap_impl_interface_trait_ids_product_ids(imp, id_remap);
        remap_impl_interface_types(imp, &mut remap_type);
    }
    for imp in bodies.generic_impls.values_mut() {
        remap_generic_param_decls_product_ids(&mut imp.type_generics, id_remap);
        remap_generic_param_decls_product_ids(&mut imp.trait_generics, id_remap);
        remap_impl_generic_param_decls_product_ids(imp, id_remap, &mut remap_type);
        remap_impl_trait_ids_product_ids(imp, id_remap);
        remap_impl_types(imp, &mut remap_type);
        remap_impl_body_location_product_ids(imp, id_remap);
    }

    for ext in interface.externs.values_mut() {
        for param in &mut ext.params {
            remap_type(param);
        }
        remap_type(&mut ext.ret);
    }
}

fn remap_generic_param_decls_product_ids(
    generic_param_decls: &mut [GenericParamDecl],
    id_remap: &ProductIdRemap,
) {
    for generic_decl in generic_param_decls {
        if let Some(product_id) =
            unambiguous_product_id(ProductDefId::from(generic_decl.id.owner), id_remap)
        {
            generic_decl.id.owner = product_def_id_to_def_id(product_id);
        }
    }
}

fn remap_generic_bounds_product_ids<F>(
    bounds: &mut crate::hir::HirGenericBounds,
    id_remap: &ProductIdRemap,
    remap_type: &mut F,
) where
    F: FnMut(&mut crate::types::Type),
{
    let mut original = std::mem::take(bounds);
    bounds.relaxed_sized = std::mem::take(&mut original.relaxed_sized)
        .into_iter()
        .map(|mut parameter| {
            remap_product_def_id_owner(&mut parameter.owner, id_remap);
            parameter
        })
        .collect();
    let mut predicates = std::mem::take(&mut original.predicates);
    for predicate in &mut predicates {
        match predicate {
            crate::types::Predicate::Trait {
                subject,
                trait_id,
                args,
            } => {
                remap_product_def_id_owner(trait_id, id_remap);
                remap_type(subject);
                for arg in args {
                    remap_type(arg);
                }
            }
        }
    }
    for (mut generic_param, mut trait_bounds) in original {
        if let Some(product_id) =
            unambiguous_product_id(ProductDefId::from(generic_param.owner), id_remap)
        {
            generic_param.owner = product_def_id_to_def_id(product_id);
        }
        for bound in &mut trait_bounds {
            remap_product_def_id_owner(&mut bound.trait_id, id_remap);
            for ty in &mut bound.type_args {
                remap_type(ty);
            }
        }
        bounds.insert(generic_param, trait_bounds);
    }
    bounds.predicates = predicates;
}

fn remap_impl_generic_param_decls_product_ids<P: HirPhase, F>(
    imp: &mut HirImplFor<P>,
    id_remap: &ProductIdRemap,
    remap_type: &mut F,
) where
    F: FnMut(&mut crate::types::Type),
{
    for method in imp.methods.values_mut() {
        remap_generic_param_decls_product_ids(&mut method.generic_params, id_remap);
        remap_generic_bounds_product_ids(&mut method.generic_bounds, id_remap, remap_type);
    }
}

fn remap_impl_interface_generic_param_decls_product_ids<F>(
    imp: &mut ProductImplInterface,
    id_remap: &ProductIdRemap,
    remap_type: &mut F,
) where
    F: FnMut(&mut crate::types::Type),
{
    for method in imp.methods.values_mut() {
        remap_generic_param_decls_product_ids(&mut method.generic_params, id_remap);
        remap_generic_bounds_product_ids(&mut method.generic_bounds, id_remap, remap_type);
    }
}

fn remap_impl_trait_ids_product_ids<P: HirPhase>(
    imp: &mut HirImplFor<P>,
    id_remap: &ProductIdRemap,
) {
    if let Some(trait_id) = &mut imp.trait_id {
        remap_product_def_id_owner(trait_id, id_remap);
    }
    let original = std::mem::take(&mut imp.bounds);
    for (mut generic_param, mut trait_bounds) in original {
        if let Some(product_id) =
            unambiguous_product_id(ProductDefId::from(generic_param.owner), id_remap)
        {
            generic_param.owner = product_def_id_to_def_id(product_id);
        }
        for bound in &mut trait_bounds {
            remap_product_def_id_owner(&mut bound.trait_id, id_remap);
        }
        imp.bounds.insert(generic_param, trait_bounds);
    }
}

fn remap_impl_interface_trait_ids_product_ids(
    imp: &mut ProductImplInterface,
    id_remap: &ProductIdRemap,
) {
    if let Some(trait_id) = &mut imp.trait_id {
        remap_product_def_id_owner(trait_id, id_remap);
    }
    let original = std::mem::take(&mut imp.bounds);
    for (mut generic_param, mut trait_bounds) in original {
        if let Some(product_id) =
            unambiguous_product_id(ProductDefId::from(generic_param.owner), id_remap)
        {
            generic_param.owner = product_def_id_to_def_id(product_id);
        }
        for bound in &mut trait_bounds {
            remap_product_def_id_owner(&mut bound.trait_id, id_remap);
        }
        imp.bounds.insert(generic_param, trait_bounds);
    }
}

fn remap_function_interface_signature_types<F>(
    function: &mut ProductFunctionInterface,
    remap_type: &mut F,
) where
    F: FnMut(&mut crate::types::Type),
{
    for param in &mut function.params {
        remap_type(param);
    }
    remap_type(&mut function.ret_type);
}

fn remap_function_signature_types<P: HirPhase, F>(
    function: &mut HirFunctionFor<P>,
    remap_type: &mut F,
) where
    F: FnMut(&mut crate::types::Type),
{
    for param in &mut function.params {
        remap_type(&mut param.ty);
    }
    remap_type(&mut function.ret_type);
    remap_type(&mut function.body.ty);
}

fn remap_function_body_types<P: HirPhase, F>(function: &mut HirFunctionFor<P>, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    remap_block_types(&mut function.body, remap_type);
}

fn remap_block_types<P: HirPhase, F>(block: &mut crate::hir::HirBlockFor<P>, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    for stmt in &mut block.stmts {
        remap_stmt_types(stmt, remap_type);
    }
    remap_type(&mut block.ty);
}

fn remap_stmt_types<P: HirPhase, F>(stmt: &mut crate::hir::HirStmtFor<P>, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    match stmt {
        crate::hir::HirStmtFor::Let { ty, value, .. } => {
            remap_type(ty);
            remap_expr_types(value, remap_type);
        }
        crate::hir::HirStmtFor::Expr(expr) => remap_expr_types(expr, remap_type),
        crate::hir::HirStmtFor::Return(expr) | crate::hir::HirStmtFor::Break(expr) => {
            if let Some(expr) = expr {
                remap_expr_types(expr, remap_type);
            }
        }
        crate::hir::HirStmtFor::Continue => {}
    }
}

fn remap_expr_types<P: HirPhase, F>(expr: &mut crate::hir::HirExprFor<P>, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    remap_type(&mut expr.ty);
    match &mut expr.kind {
        crate::hir::HirExprKindFor::Open { source, binding, body } => {
            remap_expr_types(source, remap_type);
            binding.visit_types_mut(remap_type);
            remap_block_types(body, remap_type);
        }
        crate::hir::HirExprKindFor::OwnedObjectCall { owner, args, call } => {
            remap_expr_types(owner, remap_type);
            for arg in args {
                remap_expr_types(arg, remap_type);
            }
            call.visit_types_mut(remap_type);
        }
        crate::hir::HirExprKindFor::ObjectCoercion(value, coercion) => {
            remap_expr_types(value, remap_type);
            coercion.visit_types_mut(remap_type);
        }
        crate::hir::HirExprKindFor::ArrayLiteral(elems)
        | crate::hir::HirExprKindFor::TupleLiteral(elems) => {
            for elem in elems {
                remap_expr_types(elem, remap_type);
            }
        }
        crate::hir::HirExprKindFor::ArrayRepeat(value, _) => remap_expr_types(value, remap_type),
        crate::hir::HirExprKindFor::FieldAccess(inner, _, _)
        | crate::hir::HirExprKindFor::TupleIndex(inner, _)
        | crate::hir::HirExprKindFor::UnaryOp(_, inner)
        | crate::hir::HirExprKindFor::Ref(_, inner)
        | crate::hir::HirExprKindFor::Deref(inner) => remap_expr_types(inner, remap_type),
        crate::hir::HirExprKindFor::BinOp(_, lhs, rhs)
        | crate::hir::HirExprKindFor::Assign(lhs, rhs) => {
            remap_expr_types(lhs, remap_type);
            remap_expr_types(rhs, remap_type);
        }
        crate::hir::HirExprKindFor::Call(func, args, target) => {
            if let Some(target) = target {
                remap_call_target_types(target, remap_type);
            }
            remap_expr_types(func, remap_type);
            for arg in args {
                remap_expr_types(arg, remap_type);
            }
        }
        crate::hir::HirExprKindFor::MethodCall(func, _, args, _, target) => {
            if let Some(target) = P::method_authority_mut(target) {
                remap_method_target_types(target, remap_type);
            }
            remap_expr_types(func, remap_type);
            for arg in args {
                remap_expr_types(arg, remap_type);
            }
        }
        crate::hir::HirExprKindFor::Try {
            expr,
            branch_method,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            ..
        } => {
            if let Some(target) = P::method_authority_mut(branch_method) {
                remap_method_target_types(target, remap_type);
            }
            if let Some(target) = P::residual_authority_mut(from_residual_target) {
                remap_call_target_types(target, remap_type);
            }
            remap_expr_types(expr, remap_type);
            remap_type(output_ty);
            remap_type(residual_ty);
            remap_type(return_ty);
        }
        crate::hir::HirExprKindFor::StructLiteral(_, _, fields) => {
            for field in fields {
                remap_expr_types(&mut field.value, remap_type);
            }
        }
        crate::hir::HirExprKindFor::EnumVariant(_, _, args, _)
        | crate::hir::HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                remap_expr_types(arg, remap_type);
            }
        }
        crate::hir::HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            remap_expr_types(condition, remap_type);
            remap_block_types(then_branch, remap_type);
            if let Some(else_branch) = else_branch {
                remap_block_types(else_branch, remap_type);
            }
        }
        crate::hir::HirExprKindFor::Match { scrutinee, arms } => {
            remap_expr_types(scrutinee, remap_type);
            for arm in arms {
                remap_pattern_types(&mut arm.pattern, remap_type);
                if let Some(guard) = &mut arm.guard {
                    remap_expr_types(guard, remap_type);
                }
                remap_block_types(&mut arm.body, remap_type);
            }
        }
        crate::hir::HirExprKindFor::While { condition, body } => {
            remap_expr_types(condition, remap_type);
            remap_block_types(body, remap_type);
        }
        crate::hir::HirExprKindFor::For { iter, body, .. } => {
            remap_expr_types(iter, remap_type);
            remap_block_types(body, remap_type);
        }
        crate::hir::HirExprKindFor::Loop(body)
        | crate::hir::HirExprKindFor::Block(body)
        | crate::hir::HirExprKindFor::UnsafeBlock(body) => {
            remap_block_types(body, remap_type);
        }
        crate::hir::HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => {
            for param in params {
                remap_type(&mut param.ty);
            }
            for capture in captures {
                remap_type(&mut capture.ty);
            }
            remap_block_types(body, remap_type);
        }
        crate::hir::HirExprKindFor::Cast(inner, ty) => {
            remap_expr_types(inner, remap_type);
            remap_type(ty);
        }
        crate::hir::HirExprKindFor::IntLiteral(_)
        | crate::hir::HirExprKindFor::FloatLiteral(_)
        | crate::hir::HirExprKindFor::BoolLiteral(_)
        | crate::hir::HirExprKindFor::StringLiteral(_)
        | crate::hir::HirExprKindFor::CharLiteral(_)
        | crate::hir::HirExprKindFor::Unit
        | crate::hir::HirExprKindFor::Var(_)
        | crate::hir::HirExprKindFor::ResolvedVar(_) => {}
    }
}

fn remap_pattern_types<F>(pattern: &mut HirPattern, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                remap_pattern_types(pattern, remap_type);
            }
        }
        HirPattern::Struct(_, _, args, fields) => {
            for arg in args {
                remap_type(arg);
            }
            for field in fields {
                remap_pattern_types(&mut field.pattern, remap_type);
            }
        }
        HirPattern::Enum(_, _, _, patterns) => {
            for pattern in patterns {
                remap_pattern_types(pattern, remap_type);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

fn remap_impl_body_location_product_ids<P: HirPhase>(
    imp: &mut HirImplFor<P>,
    id_remap: &ProductIdRemap,
) {
    for method in imp.methods.values_mut() {
        remap_function_body_location_product_ids(method, id_remap);
    }
}

fn remap_function_body_location_product_ids<P: HirPhase>(
    function: &mut HirFunctionFor<P>,
    id_remap: &ProductIdRemap,
) {
    remap_block_location_product_ids(&mut function.body, id_remap);
}

fn remap_block_location_product_ids<P: HirPhase>(
    block: &mut crate::hir::HirBlockFor<P>,
    id_remap: &ProductIdRemap,
) {
    for stmt in &mut block.stmts {
        remap_stmt_location_product_ids(stmt, id_remap);
    }
}

fn remap_stmt_location_product_ids<P: HirPhase>(
    stmt: &mut crate::hir::HirStmtFor<P>,
    id_remap: &ProductIdRemap,
) {
    match stmt {
        crate::hir::HirStmtFor::Let { value, .. } | crate::hir::HirStmtFor::Expr(value) => {
            remap_expr_location_product_ids(value, id_remap);
        }
        crate::hir::HirStmtFor::Return(value) | crate::hir::HirStmtFor::Break(value) => {
            if let Some(value) = value {
                remap_expr_location_product_ids(value, id_remap);
            }
        }
        crate::hir::HirStmtFor::Continue => {}
    }
}

fn remap_expr_location_product_ids<P: HirPhase>(
    expr: &mut crate::hir::HirExprFor<P>,
    id_remap: &ProductIdRemap,
) {
    match &mut expr.kind {
        crate::hir::HirExprKindFor::Open { source, binding, body } => {
            remap_expr_location_product_ids(source, id_remap);
            if let Some(owner) = &mut binding.owner {
                remap_method_target_product_ids(&mut owner.into_parts.method, id_remap);
                remap_method_target_product_ids(&mut owner.from_parts.method, id_remap);
            }
            remap_block_location_product_ids(body, id_remap);
        }
        crate::hir::HirExprKindFor::OwnedObjectCall { owner, args, call } => {
            remap_expr_location_product_ids(owner, id_remap);
            for arg in args {
                remap_expr_location_product_ids(arg, id_remap);
            }
            call.for_each_def_id_mut(|id| remap_product_def_id_owner(id, id_remap));
        }
        crate::hir::HirExprKindFor::ObjectCoercion(value, coercion) => {
            remap_expr_location_product_ids(value, id_remap);
            if let Some(crate::hir::HirObjectEvidence::Concrete { views, .. }) =
                &mut coercion.evidence
            {
                for view in views {
                    remap_product_def_id_owner(&mut view.trait_ref.trait_id, id_remap);
                    if let crate::hir::HirObjectWitnessOrigin::Impl {
                        impl_id,
                        substitution,
                    } = &mut view.origin
                    {
                        remap_product_def_id_owner(impl_id, id_remap);
                        for binding in substitution {
                            remap_product_def_id_owner(&mut binding.param.owner, id_remap);
                        }
                    }
                }
            }
        }
        crate::hir::HirExprKindFor::FieldAccess(inner, _, location) => {
            remap_expr_location_product_ids(inner, id_remap);
            if let Some(location) = location {
                remap_product_def_id_owner(&mut location.owner, id_remap);
            }
        }
        crate::hir::HirExprKindFor::StructLiteral(_, struct_id, fields) => {
            if let Some(struct_id) = struct_id {
                remap_product_def_id_owner(struct_id, id_remap);
            }
            for field in fields {
                if let Some(location) = &mut field.field {
                    remap_product_def_id_owner(&mut location.owner, id_remap);
                }
                remap_expr_location_product_ids(&mut field.value, id_remap);
            }
        }
        crate::hir::HirExprKindFor::EnumVariant(_, _, args, location) => {
            if let Some(location) = location {
                remap_product_def_id_owner(&mut location.owner, id_remap);
            }
            for arg in args {
                remap_expr_location_product_ids(arg, id_remap);
            }
        }
        crate::hir::HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                remap_expr_location_product_ids(arg, id_remap);
            }
        }
        crate::hir::HirExprKindFor::ArrayLiteral(elems)
        | crate::hir::HirExprKindFor::TupleLiteral(elems) => {
            for elem in elems {
                remap_expr_location_product_ids(elem, id_remap);
            }
        }
        crate::hir::HirExprKindFor::ArrayRepeat(value, _) => {
            remap_expr_location_product_ids(value, id_remap);
        }
        crate::hir::HirExprKindFor::TupleIndex(inner, _)
        | crate::hir::HirExprKindFor::UnaryOp(_, inner)
        | crate::hir::HirExprKindFor::Ref(_, inner)
        | crate::hir::HirExprKindFor::Deref(inner)
        | crate::hir::HirExprKindFor::Cast(inner, _) => {
            remap_expr_location_product_ids(inner, id_remap)
        }
        crate::hir::HirExprKindFor::BinOp(_, lhs, rhs)
        | crate::hir::HirExprKindFor::Assign(lhs, rhs) => {
            remap_expr_location_product_ids(lhs, id_remap);
            remap_expr_location_product_ids(rhs, id_remap);
        }
        crate::hir::HirExprKindFor::Call(func, args, target) => {
            if let Some(target) = target {
                remap_call_target_product_ids(target, id_remap);
            }
            remap_expr_location_product_ids(func, id_remap);
            for arg in args {
                remap_expr_location_product_ids(arg, id_remap);
            }
        }
        crate::hir::HirExprKindFor::MethodCall(func, _, args, _, target) => {
            if let Some(target) = P::method_authority_mut(target) {
                remap_method_target_product_ids(target, id_remap);
            }
            remap_expr_location_product_ids(func, id_remap);
            for arg in args {
                remap_expr_location_product_ids(arg, id_remap);
            }
        }
        crate::hir::HirExprKindFor::Try {
            expr,
            branch_method,
            from_residual_target,
            control_flow_enum,
            break_variant,
            continue_variant,
            ..
        } => {
            if let Some(target) = P::method_authority_mut(branch_method) {
                remap_method_target_product_ids(target, id_remap);
            }
            if let Some(target) = P::residual_authority_mut(from_residual_target) {
                remap_call_target_product_ids(target, id_remap);
            }
            remap_product_def_id_owner(control_flow_enum, id_remap);
            remap_product_def_id_owner(&mut break_variant.owner, id_remap);
            remap_product_def_id_owner(&mut continue_variant.owner, id_remap);
            remap_expr_location_product_ids(expr, id_remap);
        }
        crate::hir::HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            remap_expr_location_product_ids(condition, id_remap);
            remap_block_location_product_ids(then_branch, id_remap);
            if let Some(else_branch) = else_branch {
                remap_block_location_product_ids(else_branch, id_remap);
            }
        }
        crate::hir::HirExprKindFor::Match { scrutinee, arms } => {
            remap_expr_location_product_ids(scrutinee, id_remap);
            for arm in arms {
                remap_pattern_location_product_ids(&mut arm.pattern, id_remap);
                if let Some(guard) = &mut arm.guard {
                    remap_expr_location_product_ids(guard, id_remap);
                }
                remap_block_location_product_ids(&mut arm.body, id_remap);
            }
        }
        crate::hir::HirExprKindFor::While { condition, body } => {
            remap_expr_location_product_ids(condition, id_remap);
            remap_block_location_product_ids(body, id_remap);
        }
        crate::hir::HirExprKindFor::For { iter, body, .. } => {
            remap_expr_location_product_ids(iter, id_remap);
            remap_block_location_product_ids(body, id_remap);
        }
        crate::hir::HirExprKindFor::Loop(body)
        | crate::hir::HirExprKindFor::Block(body)
        | crate::hir::HirExprKindFor::UnsafeBlock(body) => {
            remap_block_location_product_ids(body, id_remap);
        }
        crate::hir::HirExprKindFor::Lambda { body, .. } => {
            remap_block_location_product_ids(body, id_remap);
        }
        crate::hir::HirExprKindFor::ResolvedVar(reference) => {
            remap_var_target_product_ids(&mut reference.target, id_remap);
        }
        crate::hir::HirExprKindFor::IntLiteral(_)
        | crate::hir::HirExprKindFor::FloatLiteral(_)
        | crate::hir::HirExprKindFor::BoolLiteral(_)
        | crate::hir::HirExprKindFor::StringLiteral(_)
        | crate::hir::HirExprKindFor::CharLiteral(_)
        | crate::hir::HirExprKindFor::Unit
        | crate::hir::HirExprKindFor::Var(_) => {}
    }
}

fn remap_call_target_product_ids(target: &mut HirCallTarget, id_remap: &ProductIdRemap) {
    match target {
        HirCallTarget::Function(id) | HirCallTarget::Extern(id) => {
            remap_product_def_id_owner(id, id_remap);
        }
        HirCallTarget::StaticMethod(target) => {
            remap_method_target_product_ids(&mut target.method, id_remap);
        }
        HirCallTarget::Instance(_) | HirCallTarget::Local(_) | HirCallTarget::Intrinsic(_) => {}
    }
}

fn remap_method_target_product_ids(target: &mut HirMethodCallTarget, id_remap: &ProductIdRemap) {
    match &mut target.target {
        HirSelectedMethodTarget::ImplMethod {
            impl_id,
            method_id,
            selected_trait,
        } => {
            remap_product_def_id_owner(impl_id, id_remap);
            remap_product_def_id_owner(method_id, id_remap);
            if let Some(selected_trait) = selected_trait {
                remap_product_def_id_owner(&mut selected_trait.trait_id, id_remap);
                remap_product_def_id_owner(&mut selected_trait.member_id, id_remap);
            }
        }
        HirSelectedMethodTarget::TraitMethod {
            trait_id,
            member_id,
            dispatch,
            ..
        } => {
            remap_product_def_id_owner(trait_id, id_remap);
            remap_product_def_id_owner(member_id, id_remap);
            if let crate::hir::HirTraitDispatchKind::Opened(witness) = dispatch {
                remap_product_def_id_owner(&mut witness.owner, id_remap);
            }
        }
    }
    for binding in target
        .owner_substitution
        .iter_mut()
        .chain(target.method_substitution.iter_mut())
    {
        remap_product_def_id_owner(&mut binding.param.owner, id_remap);
    }
}

fn remap_call_target_types<F>(target: &mut HirCallTarget, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    if let HirCallTarget::StaticMethod(target) = target {
        remap_type(&mut target.owner_ty);
        remap_method_target_types(&mut target.method, remap_type);
    }
}

fn remap_method_target_types<F>(target: &mut HirMethodCallTarget, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    target.for_each_type_mut(remap_type);
}

fn remap_var_target_product_ids(target: &mut HirVarTarget, id_remap: &ProductIdRemap) {
    match target {
        HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
            remap_product_def_id_owner(id, id_remap);
        }
        HirVarTarget::Instance(_) | HirVarTarget::Local(_) => {}
    }
}

fn remap_pattern_location_product_ids(pattern: &mut HirPattern, id_remap: &ProductIdRemap) {
    match pattern {
        HirPattern::Struct(_, struct_id, _, fields) => {
            if let Some(struct_id) = struct_id {
                remap_product_def_id_owner(struct_id, id_remap);
            }
            for field in fields {
                if let Some(location) = &mut field.field {
                    remap_product_def_id_owner(&mut location.owner, id_remap);
                }
                remap_pattern_location_product_ids(&mut field.pattern, id_remap);
            }
        }
        HirPattern::Enum(_, _, location, patterns) => {
            if let Some(location) = location {
                remap_product_def_id_owner(&mut location.owner, id_remap);
            }
            for pattern in patterns {
                remap_pattern_location_product_ids(pattern, id_remap);
            }
        }
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                remap_pattern_location_product_ids(pattern, id_remap);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

fn remap_product_def_id_owner(owner: &mut DefId, id_remap: &ProductIdRemap) {
    if let Some(product_id) = unambiguous_product_id(ProductDefId::from(*owner), id_remap) {
        *owner = product_def_id_to_def_id(product_id);
    }
}

fn remap_impl_types<P: HirPhase, F>(imp: &mut HirImplFor<P>, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    P::visit_impl_receiver_types_mut(&mut imp.receiver_pattern, remap_type);
    for ty in &mut imp.trait_arg_types {
        remap_type(ty);
    }
    for assoc in &mut imp.associated_types {
        remap_type(&mut assoc.ty);
    }
    for bounds in imp.bounds.values_mut() {
        for bound in bounds {
            for ty in &mut bound.type_args {
                remap_type(ty);
            }
        }
    }
    for method in imp.methods.values_mut() {
        remap_function_signature_types(method, remap_type);
        remap_function_body_types(method, remap_type);
    }
}

fn remap_impl_interface_types<F>(imp: &mut ProductImplInterface, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    match &mut imp.receiver_pattern {
        crate::hir::HirImplReceiverPattern::Exact(ty)
        | crate::hir::HirImplReceiverPattern::Constructor(ty) => remap_type(ty),
        crate::hir::HirImplReceiverPattern::SliceFamily { element } => remap_type(element),
    }
    for ty in &mut imp.trait_arg_types {
        remap_type(ty);
    }
    for assoc in &mut imp.associated_types {
        remap_type(&mut assoc.ty);
    }
    for bounds in imp.bounds.values_mut() {
        for bound in bounds {
            for ty in &mut bound.type_args {
                remap_type(ty);
            }
        }
    }
    for method in imp.methods.values_mut() {
        remap_function_interface_signature_types(method, remap_type);
    }
}

struct PreferredTraitDefaultMethods {
    by_id: HashMap<DefId, AcceptedHirFunction>,
}

impl PreferredTraitDefaultMethods {
    fn new(program: &HirProgramFor<AcceptedHir>) -> Self {
        let mut preferred = Self {
            by_id: HashMap::new(),
        };

        for (_, _, trait_def) in program.traits_by_id() {
            for method in trait_def.methods.values() {
                insert_preferred_default(&mut preferred.by_id, method.id, method.clone());
            }
        }

        preferred
    }

    fn get(&self, id: DefId, _trait_name: &str, _method_name: &str) -> AcceptedHirFunction {
        self.by_id
            .get(&id)
            .cloned()
            .expect("trait default method should have a preferred body")
    }
}

fn insert_preferred_default<K>(
    preferred: &mut HashMap<K, AcceptedHirFunction>,
    key: K,
    method: AcceptedHirFunction,
) where
    K: Eq + std::hash::Hash,
{
    preferred
        .entry(key)
        .and_modify(|current| {
            if current.body.stmts.is_empty() && !method.body.stmts.is_empty() {
                *current = method.clone();
            }
        })
        .or_insert(method);
}

fn record_name(identity_table: &mut ProductIdentityTable, id: ProductDefId, name: &str) {
    record_display_name(identity_table, id, name);
    record_export_name(identity_table, id, name);
}

fn record_display_name(identity_table: &mut ProductIdentityTable, id: ProductDefId, name: &str) {
    identity_table.display_names.insert(id, name.to_string());
}

fn record_export_name(identity_table: &mut ProductIdentityTable, id: ProductDefId, name: &str) {
    if identity_table.ambiguous_export_names.contains(name) {
        return;
    }

    match identity_table.export_names.get(name).copied() {
        Some(existing) if existing != id => {
            identity_table.export_names.remove(name);
            identity_table
                .ambiguous_export_names
                .insert(name.to_string());
        }
        Some(_) => {}
        None => {
            identity_table.export_names.insert(name.to_string(), id);
        }
    }
}

fn record_product_aliases(
    out: &mut BTreeMap<String, ProductDefId>,
    aliases: &HashMap<String, DefId>,
    id_remap: &ProductIdRemap,
) {
    for (alias, id) in aliases {
        if let Some(product_id) = product_alias_id(ProductDefId::from(*id), id_remap) {
            out.insert(alias.clone(), product_id);
        }
    }
}

fn product_alias_id(id: ProductDefId, id_remap: &ProductIdRemap) -> Option<ProductDefId> {
    match id_remap.get(&id) {
        Some(ids) if ids.len() == 1 => ids.iter().next().copied(),
        _ => None,
    }
}

fn record_id_remap(id_remap: &mut ProductIdRemap, requested: ProductDefId, actual: ProductDefId) {
    id_remap.entry(requested).or_default().insert(actual);
}

fn remap_link_data(link: ProductLinkData, id_remap: &ProductIdRemap) -> ProductLinkData {
    let records = link
        .records
        .into_iter()
        // Ambiguous original IDs have no safe product owner, so drop them.
        .filter_map(|(id, record)| unambiguous_product_id(id, id_remap).map(|id| (id, record)))
        .collect();

    ProductLinkData {
        object_path: link.object_path,
        records,
    }
}

fn unambiguous_product_id(id: ProductDefId, id_remap: &ProductIdRemap) -> Option<ProductDefId> {
    let ids = id_remap.get(&id)?;
    if ids.len() == 1 {
        ids.iter().next().copied()
    } else {
        None
    }
}

fn strict_object_product_id(
    id: DefId,
    identities: &ProductIdentityTable,
    remap: &ProductIdRemap,
) -> Result<DefId, String> {
    let product = ProductDefId::from(id);
    if let Some(mapped) = unambiguous_product_id(product, remap) {
        return Ok(product_def_id_to_def_id(mapped));
    }
    if !remap.contains_key(&product) && identities.dependencies.contains_key(&product.crate_id) {
        return Ok(id);
    }
    Err(format!(
        "object ABI definition {id:?} has no unambiguous product identity"
    ))
}

fn reserve_product_id(
    requested: ProductDefId,
    used_ids: &mut BTreeSet<ProductDefId>,
    fallback_crate: ProductCrateId,
    next_fallback_local: &mut u32,
    force_fallback: bool,
) -> ProductDefId {
    if force_fallback || requested.crate_id == ProductCrateId(u32::MAX) {
        used_ids.insert(requested);
        return fresh_product_id(fallback_crate, next_fallback_local, used_ids);
    }

    if !force_fallback && used_ids.insert(requested) {
        requested
    } else {
        fresh_product_id(fallback_crate, next_fallback_local, used_ids)
    }
}

fn fresh_product_id(
    fallback_crate: ProductCrateId,
    next_fallback_local: &mut u32,
    used_ids: &mut BTreeSet<ProductDefId>,
) -> ProductDefId {
    loop {
        let id = ProductDefId {
            crate_id: fallback_crate,
            local_id: ProductLocalDefId(*next_fallback_local),
        };
        *next_fallback_local = next_fallback_local
            .checked_add(1)
            .expect("product fallback ID generator exhausted u32 ID space");

        if used_ids.insert(id) {
            return id;
        }
    }
}

fn impl_display_name<P: HirPhase>(imp: &HirImplFor<P>) -> String {
    match &imp.trait_name {
        Some(trait_name) => format!("{} as {}", imp.type_name, trait_name),
        None => imp.type_name.clone(),
    }
}

fn qualify_metadata_name(crate_name: &str, name: &str) -> String {
    if name.contains("::") {
        name.to_string()
    } else {
        format!("{}::{}", crate_name, name)
    }
}

fn product_def_id_to_def_id(id: ProductDefId) -> DefId {
    DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0))
}

fn remap_def_id_if_matches(id: &mut DefId, old_id: DefId, new_id: DefId) {
    if *id == old_id {
        *id = new_id;
    }
}

fn remap_type_owned_generic_owner(ty: &mut crate::types::Type, old_id: DefId, new_id: DefId) {
    use crate::types::Type;

    struct OwnedGenericOwnerFolder {
        old_id: DefId,
        new_id: DefId,
    }

    impl crate::type_services::visit::TypeFolder for OwnedGenericOwnerFolder {
        fn fold_type(&mut self, mut ty: Type) -> Type {
            match &mut ty {
                Type::Generic(param) => {
                    remap_def_id_if_matches(&mut param.owner, self.old_id, self.new_id);
                }
                Type::Witness(witness) => {
                    remap_def_id_if_matches(&mut witness.owner, self.old_id, self.new_id);
                }
                _ => {}
            }
            crate::type_services::visit::fold_type_children(ty, self)
        }
    }

    crate::type_services::visit::fold_type_in_place(
        ty,
        &mut OwnedGenericOwnerFolder { old_id, new_id },
    );
}

fn remap_generic_param_decls_owned_owner(
    generic_param_decls: &mut [GenericParamDecl],
    old_id: DefId,
    new_id: DefId,
) {
    for generic_decl in generic_param_decls {
        if generic_decl.id.owner == old_id {
            generic_decl.id.owner = new_id;
        }
    }
}

fn remap_generic_bounds_owned_owner(
    bounds: &mut crate::hir::HirGenericBounds,
    old_id: DefId,
    new_id: DefId,
) {
    let mut original = std::mem::take(bounds);
    bounds.relaxed_sized = std::mem::take(&mut original.relaxed_sized)
        .into_iter()
        .map(|mut parameter| {
            if parameter.owner == old_id {
                parameter.owner = new_id;
            }
            parameter
        })
        .collect();
    let mut predicates = std::mem::take(&mut original.predicates);
    for predicate in &mut predicates {
        match predicate {
            crate::types::Predicate::Trait { subject, args, .. } => {
                remap_type_owned_generic_owner(subject, old_id, new_id);
                for arg in args {
                    remap_type_owned_generic_owner(arg, old_id, new_id);
                }
            }
        }
    }
    for (mut generic_param, mut trait_bounds) in original {
        if generic_param.owner == old_id {
            generic_param.owner = new_id;
        }
        for bound in &mut trait_bounds {
            for ty in &mut bound.type_args {
                remap_type_owned_generic_owner(ty, old_id, new_id);
            }
        }
        bounds.insert(generic_param, trait_bounds);
    }
    bounds.predicates = predicates;
}

fn remap_function_owned_type_ids<P: HirPhase>(function: &mut HirFunctionFor<P>, old_id: DefId) {
    let new_id = function.id;
    remap_generic_param_decls_owned_owner(&mut function.generic_params, old_id, new_id);
    remap_generic_bounds_owned_owner(&mut function.generic_bounds, old_id, new_id);
    let mut remap_type = |ty: &mut crate::types::Type| {
        remap_type_owned_generic_owner(ty, old_id, new_id);
    };
    remap_function_signature_types(function, &mut remap_type);
    remap_block_owned_type_ids(&mut function.body, old_id, new_id);
}

fn remap_struct_owned_type_ids(strukt: &mut HirStruct, old_id: DefId) {
    let new_id = strukt.id;
    remap_generic_param_decls_owned_owner(&mut strukt.generic_params, old_id, new_id);
    for field in &mut strukt.fields {
        remap_type_owned_generic_owner(&mut field.ty, old_id, new_id);
    }
}

fn remap_enum_owned_type_ids(enm: &mut HirEnum, old_id: DefId) {
    let new_id = enm.id;
    remap_generic_param_decls_owned_owner(&mut enm.generic_params, old_id, new_id);
    for variant in &mut enm.variants {
        match &mut variant.fields {
            HirVariantFields::Named(fields) => {
                for field in fields {
                    remap_type_owned_generic_owner(&mut field.ty, old_id, new_id);
                }
            }
            HirVariantFields::Positional(types) => {
                for ty in types {
                    remap_type_owned_generic_owner(ty, old_id, new_id);
                }
            }
            HirVariantFields::Unit => {}
        }
    }
}

fn remap_type_alias_owned_type_ids(alias: &mut HirTypeAlias, old_id: DefId) {
    let new_id = alias.id;
    remap_generic_param_decls_owned_owner(&mut alias.generic_params, old_id, new_id);
    remap_type_owned_generic_owner(&mut alias.ty, old_id, new_id);
}

fn remap_trait_owned_type_ids<P: HirPhase>(trait_: &mut HirTraitFor<P>, old_id: DefId) {
    let new_id = trait_.id;
    remap_generic_param_decls_owned_owner(&mut trait_.generic_params, old_id, new_id);
    if let Some(target) = &mut trait_.target {
        if target.id.owner == old_id {
            target.id.owner = new_id;
        }
    }
    let mut remap_type = |ty: &mut crate::types::Type| {
        remap_type_owned_generic_owner(ty, old_id, new_id);
    };
    for predicate in &mut trait_.predicates {
        match predicate {
            crate::types::Predicate::Trait {
                subject,
                trait_id,
                args,
            } => {
                if *trait_id == old_id {
                    *trait_id = new_id;
                }
                remap_type(subject);
                for arg in args {
                    remap_type(arg);
                }
            }
        }
    }
    for method in trait_.methods.values_mut() {
        remap_generic_param_decls_owned_owner(&mut method.generic_params, old_id, new_id);
        remap_generic_bounds_owned_owner(&mut method.generic_bounds, old_id, new_id);
        remap_function_signature_types(method, &mut remap_type);
    }
    for sig in trait_.signatures.values_mut() {
        remap_generic_param_decls_owned_owner(&mut sig.generic_params, old_id, new_id);
        remap_generic_bounds_owned_owner(&mut sig.generic_bounds, old_id, new_id);
        for param in &mut sig.params {
            remap_type(param);
        }
        remap_type(&mut sig.ret);
    }
}

fn remap_impl_owned_type_ids<P: HirPhase>(imp: &mut HirImplFor<P>, old_id: DefId) {
    let new_id = imp.id;
    remap_generic_param_decls_owned_owner(&mut imp.type_generics, old_id, new_id);
    remap_generic_param_decls_owned_owner(&mut imp.trait_generics, old_id, new_id);
    for method in imp.methods.values_mut() {
        remap_generic_param_decls_owned_owner(&mut method.generic_params, old_id, new_id);
        remap_generic_bounds_owned_owner(&mut method.generic_bounds, old_id, new_id);
    }
    let mut remap_type = |ty: &mut crate::types::Type| {
        remap_type_owned_generic_owner(ty, old_id, new_id);
    };
    remap_impl_types(imp, &mut remap_type);
}

fn remap_extern_owned_type_ids(ext: &mut HirExtern, old_id: DefId) {
    let new_id = ext.id;
    for param in &mut ext.params {
        remap_type_owned_generic_owner(param, old_id, new_id);
    }
    remap_type_owned_generic_owner(&mut ext.ret, old_id, new_id);
}

fn remap_block_owned_type_ids<P: HirPhase>(
    block: &mut crate::hir::HirBlockFor<P>,
    old_id: DefId,
    new_id: DefId,
) {
    for stmt in &mut block.stmts {
        remap_stmt_owned_type_ids(stmt, old_id, new_id);
    }
    remap_type_owned_generic_owner(&mut block.ty, old_id, new_id);
}

fn remap_stmt_owned_type_ids<P: HirPhase>(
    stmt: &mut crate::hir::HirStmtFor<P>,
    old_id: DefId,
    new_id: DefId,
) {
    match stmt {
        crate::hir::HirStmtFor::Let { ty, value, .. } => {
            remap_type_owned_generic_owner(ty, old_id, new_id);
            remap_expr_owned_type_ids(value, old_id, new_id);
        }
        crate::hir::HirStmtFor::Expr(expr) => remap_expr_owned_type_ids(expr, old_id, new_id),
        crate::hir::HirStmtFor::Return(expr) | crate::hir::HirStmtFor::Break(expr) => {
            if let Some(expr) = expr {
                remap_expr_owned_type_ids(expr, old_id, new_id);
            }
        }
        crate::hir::HirStmtFor::Continue => {}
    }
}

fn remap_expr_owned_type_ids<P: HirPhase>(
    expr: &mut crate::hir::HirExprFor<P>,
    old_id: DefId,
    new_id: DefId,
) {
    remap_type_owned_generic_owner(&mut expr.ty, old_id, new_id);
    match &mut expr.kind {
        crate::hir::HirExprKindFor::Open { source, binding, body } => {
            remap_expr_owned_type_ids(source, old_id, new_id);
            binding.visit_types_mut(&mut |ty| remap_type_owned_generic_owner(ty, old_id, new_id));
            if let Some(owner) = &mut binding.owner {
                remap_method_target_owned_type_ids(&mut owner.into_parts.method, old_id, new_id);
                remap_method_target_owned_type_ids(&mut owner.from_parts.method, old_id, new_id);
            }
            remap_block_owned_type_ids(body, old_id, new_id);
        }
        crate::hir::HirExprKindFor::OwnedObjectCall { owner, args, call } => {
            remap_expr_owned_type_ids(owner, old_id, new_id);
            for arg in args {
                remap_expr_owned_type_ids(arg, old_id, new_id);
            }
            call.visit_types_mut(&mut |ty| remap_type_owned_generic_owner(ty, old_id, new_id));
            for method in [
                &mut call.method,
                &mut call.into_parts.method,
                &mut call.release.method,
            ] {
                for binding in method
                    .owner_substitution
                    .iter_mut()
                    .chain(&mut method.method_substitution)
                {
                    remap_def_id_if_matches(&mut binding.param.owner, old_id, new_id);
                }
            }
        }
        crate::hir::HirExprKindFor::ObjectCoercion(value, coercion) => {
            remap_expr_owned_type_ids(value, old_id, new_id);
            coercion.visit_types_mut(&mut |ty| remap_type_owned_generic_owner(ty, old_id, new_id));
            if let Some(crate::hir::HirObjectEvidence::Concrete { views, .. }) =
                &mut coercion.evidence
            {
                for view in views {
                    if let crate::hir::HirObjectWitnessOrigin::Impl { substitution, .. } =
                        &mut view.origin
                    {
                        for binding in substitution {
                            if binding.param.owner == old_id {
                                binding.param.owner = new_id;
                            }
                        }
                    }
                }
            }
        }
        crate::hir::HirExprKindFor::ArrayLiteral(elems)
        | crate::hir::HirExprKindFor::TupleLiteral(elems) => {
            for elem in elems {
                remap_expr_owned_type_ids(elem, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::ArrayRepeat(value, _) => {
            remap_expr_owned_type_ids(value, old_id, new_id);
        }
        crate::hir::HirExprKindFor::FieldAccess(inner, _, _)
        | crate::hir::HirExprKindFor::TupleIndex(inner, _)
        | crate::hir::HirExprKindFor::UnaryOp(_, inner)
        | crate::hir::HirExprKindFor::Ref(_, inner)
        | crate::hir::HirExprKindFor::Deref(inner) => {
            remap_expr_owned_type_ids(inner, old_id, new_id)
        }
        crate::hir::HirExprKindFor::BinOp(_, lhs, rhs)
        | crate::hir::HirExprKindFor::Assign(lhs, rhs) => {
            remap_expr_owned_type_ids(lhs, old_id, new_id);
            remap_expr_owned_type_ids(rhs, old_id, new_id);
        }
        crate::hir::HirExprKindFor::Call(func, args, target) => {
            if let Some(target) = target {
                remap_call_target_owned_type_ids(target, old_id, new_id);
            }
            remap_expr_owned_type_ids(func, old_id, new_id);
            for arg in args {
                remap_expr_owned_type_ids(arg, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::MethodCall(func, _, args, _, target) => {
            if let Some(target) = P::method_authority_mut(target) {
                remap_method_target_owned_type_ids(target, old_id, new_id);
            }
            remap_expr_owned_type_ids(func, old_id, new_id);
            for arg in args {
                remap_expr_owned_type_ids(arg, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::Try {
            expr,
            branch_method,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            control_flow_enum,
            break_variant,
            continue_variant,
            ..
        } => {
            if let Some(target) = P::method_authority_mut(branch_method) {
                remap_method_target_owned_type_ids(target, old_id, new_id);
            }
            if let Some(target) = P::residual_authority_mut(from_residual_target) {
                remap_call_target_owned_type_ids(target, old_id, new_id);
            }
            remap_type_owned_generic_owner(output_ty, old_id, new_id);
            remap_type_owned_generic_owner(residual_ty, old_id, new_id);
            remap_type_owned_generic_owner(return_ty, old_id, new_id);
            remap_def_id_if_matches(control_flow_enum, old_id, new_id);
            remap_def_id_if_matches(&mut break_variant.owner, old_id, new_id);
            remap_def_id_if_matches(&mut continue_variant.owner, old_id, new_id);
            remap_expr_owned_type_ids(expr, old_id, new_id);
        }
        crate::hir::HirExprKindFor::StructLiteral(_, struct_id, fields) => {
            if let Some(struct_id) = struct_id {
                remap_def_id_if_matches(struct_id, old_id, new_id);
            }
            for field in fields {
                remap_expr_owned_type_ids(&mut field.value, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::EnumVariant(_, _, args, _)
        | crate::hir::HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                remap_expr_owned_type_ids(arg, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            remap_expr_owned_type_ids(condition, old_id, new_id);
            remap_block_owned_type_ids(then_branch, old_id, new_id);
            if let Some(else_branch) = else_branch {
                remap_block_owned_type_ids(else_branch, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::Match { scrutinee, arms } => {
            remap_expr_owned_type_ids(scrutinee, old_id, new_id);
            for arm in arms {
                remap_pattern_owned_type_ids(&mut arm.pattern, old_id, new_id);
                if let Some(guard) = &mut arm.guard {
                    remap_expr_owned_type_ids(guard, old_id, new_id);
                }
                remap_block_owned_type_ids(&mut arm.body, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::While { condition, body } => {
            remap_expr_owned_type_ids(condition, old_id, new_id);
            remap_block_owned_type_ids(body, old_id, new_id);
        }
        crate::hir::HirExprKindFor::For { iter, body, .. } => {
            remap_expr_owned_type_ids(iter, old_id, new_id);
            remap_block_owned_type_ids(body, old_id, new_id);
        }
        crate::hir::HirExprKindFor::Loop(body)
        | crate::hir::HirExprKindFor::Block(body)
        | crate::hir::HirExprKindFor::UnsafeBlock(body) => {
            remap_block_owned_type_ids(body, old_id, new_id);
        }
        crate::hir::HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => {
            for param in params {
                remap_type_owned_generic_owner(&mut param.ty, old_id, new_id);
            }
            for capture in captures {
                remap_type_owned_generic_owner(&mut capture.ty, old_id, new_id);
            }
            remap_block_owned_type_ids(body, old_id, new_id);
        }
        crate::hir::HirExprKindFor::Cast(inner, ty) => {
            remap_expr_owned_type_ids(inner, old_id, new_id);
            remap_type_owned_generic_owner(ty, old_id, new_id);
        }
        crate::hir::HirExprKindFor::IntLiteral(_)
        | crate::hir::HirExprKindFor::FloatLiteral(_)
        | crate::hir::HirExprKindFor::BoolLiteral(_)
        | crate::hir::HirExprKindFor::StringLiteral(_)
        | crate::hir::HirExprKindFor::CharLiteral(_)
        | crate::hir::HirExprKindFor::Unit
        | crate::hir::HirExprKindFor::Var(_) => {}
        crate::hir::HirExprKindFor::ResolvedVar(reference) => {
            remap_var_target_owned_type_ids(&mut reference.target, old_id, new_id);
        }
    }
}

fn remap_var_target_owned_type_ids(target: &mut HirVarTarget, old_id: DefId, new_id: DefId) {
    match target {
        HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
            remap_def_id_if_matches(id, old_id, new_id);
        }
        HirVarTarget::Instance(_) | HirVarTarget::Local(_) => {}
    }
}

fn remap_call_target_owned_type_ids(target: &mut HirCallTarget, old_id: DefId, new_id: DefId) {
    if let HirCallTarget::StaticMethod(target) = target {
        remap_type_owned_generic_owner(&mut target.owner_ty, old_id, new_id);
        remap_method_target_owned_type_ids(&mut target.method, old_id, new_id);
    }
}

fn remap_method_target_owned_type_ids(
    target: &mut HirMethodCallTarget,
    old_id: DefId,
    new_id: DefId,
) {
    target.for_each_type_mut(|ty| remap_type_owned_generic_owner(ty, old_id, new_id));
    if let HirSelectedMethodTarget::TraitMethod { dispatch: crate::hir::HirTraitDispatchKind::Opened(witness), .. } = &mut target.target {
        remap_def_id_if_matches(&mut witness.owner, old_id, new_id);
    }
    for binding in target
        .owner_substitution
        .iter_mut()
        .chain(target.method_substitution.iter_mut())
    {
        remap_def_id_if_matches(&mut binding.param.owner, old_id, new_id);
    }
}

fn remap_pattern_owned_type_ids(pattern: &mut HirPattern, old_id: DefId, new_id: DefId) {
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                remap_pattern_owned_type_ids(pattern, old_id, new_id);
            }
        }
        HirPattern::Struct(_, struct_id, args, fields) => {
            if let Some(struct_id) = struct_id {
                remap_def_id_if_matches(struct_id, old_id, new_id);
            }
            for arg in args {
                remap_type_owned_generic_owner(arg, old_id, new_id);
            }
            for field in fields {
                if let Some(location) = &mut field.field {
                    remap_def_id_if_matches(&mut location.owner, old_id, new_id);
                }
                remap_pattern_owned_type_ids(&mut field.pattern, old_id, new_id);
            }
        }
        HirPattern::Enum(_, _, location, patterns) => {
            if let Some(location) = location {
                remap_def_id_if_matches(&mut location.owner, old_id, new_id);
            }
            for pattern in patterns {
                remap_pattern_owned_type_ids(pattern, old_id, new_id);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::path::PathBuf;

    use crate::collect::resolver::ResolverTables;
    use crate::hir::{
        AcceptedHir, AcceptedHirFunction, AcceptedHirProgram, HirAssociatedTypeDecl, HirBlock,
        HirCallTarget, HirClosureCapture, HirClosureCaptureKind, HirEnum, HirExpr, HirExtern,
        HirFieldLocation, HirFunction, HirFunctionSig, HirImpl, HirImplFor, HirLanguageItems,
        HirNameTables, HirParam, HirPattern, HirProgram, HirProgramFor, HirStruct,
        HirStructLiteralField, HirStructPatternField, HirTrait, HirTraitFor, HirVarRef,
        HirVarTarget, HirVariantLocation,
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
        encode_product_artifact, read_product_artifact_header,
        serialized_artifact_from_bytes_for_test, CompilerProducts, ProductArtifactHeader,
        ProductArtifactPreamble, ProductBodies, ProductCrateId, ProductCrateIdentity, ProductDefId,
        ProductDependencyCapabilities, ProductDependencyIdentity, ProductDependencyLinkCapability,
        ProductIdentityTable, ProductLanguageItems, ProductLinkData, ProductLinkRecord,
        ProductLocalDefId, ProductSourceFingerprint, MAX_PRODUCT_ARTIFACT_BYTES,
        MAX_PRODUCT_ARTIFACT_HEADER_BYTES, MAX_PRODUCT_ARTIFACT_STRING_BYTES,
        MAX_PRODUCT_ARTIFACT_TOTAL_STRING_BYTES, PRODUCT_ARTIFACT_FORMAT_VERSION,
        PRODUCT_ARTIFACT_MAGIC,
    };
    use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, Type};

    #[test]
    fn product_def_id_preserves_def_id_raw_parts() {
        let def_id = DefId::new(CrateId(7), LocalDefId(42));

        let product_id = ProductDefId::from(def_id);

        assert_eq!(product_id.crate_id, ProductCrateId(7));
        assert_eq!(product_id.local_id, ProductLocalDefId(42));
    }

    #[test]
    fn product_def_id_is_usable_as_stable_btree_key() {
        let first = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(2),
        };
        let second = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(1),
        };

        let mut map = BTreeMap::new();
        map.insert(first, "second item".to_string());
        map.insert(second, "first item".to_string());

        let keys = map.keys().copied().collect::<Vec<_>>();
        assert_eq!(keys, vec![second, first]);
    }

    fn product_def_id(local_id: u32) -> ProductDefId {
        ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(local_id),
        }
    }

    fn generic_params(owner: DefId, names: &[&str]) -> Vec<GenericParamDecl> {
        GenericParamDecl::type_params(owner, names.iter().copied())
    }

    fn resolved_hir_with_language_items(
        language_items: LanguageItems<DefId>,
        current_def_ids: BTreeSet<DefId>,
    ) -> ResolvedHirProgram {
        let mut traits = HashMap::new();
        if let Some(items) = &language_items.sized {
            traits.insert(
                items.trait_id,
                HirTrait {
                    target: None,
                    predicates: Vec::new(),
                    id: items.trait_id,
                    name: "SizedProvider".to_string(),
                    generic_params: Vec::new(),
                    associated_types: Vec::new(),
                    methods: HashMap::new(),
                    signatures: HashMap::new(),
                },
            );
        }
        if let Some(items) = &language_items.index {
            let key = Type::Generic(GenericParamId {
                owner: items.trait_id,
                index: 0,
            });
            let self_ty = Type::Generic(GenericParamId {
                owner: items.trait_id,
                index: 1,
            });
            traits.insert(
                items.trait_id,
                HirTrait {
                    target: None,
                    predicates: Vec::new(),
                    id: items.trait_id,
                    name: "IndexProvider".to_string(),
                    generic_params: generic_params(items.trait_id, &["Key"]),
                    associated_types: vec![HirAssociatedTypeDecl {
                        id: items.output_id,
                        name: "Output".to_string(),
                        kind: crate::type_services::kind::Kind::Type,
                    }],
                    methods: HashMap::new(),
                    signatures: HashMap::from([(
                        "index".to_string(),
                        HirFunctionSig {
                            id: items.method_id,
                            name: "index".to_string(),
                            generic_params: Vec::new(),
                            params: vec![
                                Type::Reference {
                                    mutable: false,
                                    inner: Box::new(self_ty.clone()),
                                },
                                key.clone(),
                            ],
                            ret: Type::Reference {
                                mutable: false,
                                inner: Box::new(Type::Projection {
                                    ty: Box::new(self_ty),
                                    trait_id: items.trait_id,
                                    assoc_type: AssociatedTypeKey {
                                        owner: items.trait_id,
                                        assoc_type_id: items.output_id,
                                    },
                                    trait_args: vec![key],
                                }),
                            },
                            generic_bounds: HashMap::new().into(),
                            self_receiver: Some(crate::types::ReceiverMode::Shared),
                            is_unsafe: false,
                        },
                    )]),
                },
            );
        }
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            traits,
            HashMap::new(),
            HashMap::new(),
            HirNameTables::default(),
            language_items,
            &HashMap::new(),
        );
        let accepted = AcceptedHirProgram::try_from(program)
            .expect("product fixture must construct accepted HIR");
        ResolvedHirProgram::from_accepted(
            accepted,
            ResolverTables::default(),
            current_def_ids,
            CrateId(0),
            IdGen::new(),
        )
    }

    fn products_from_hir(hir: &ResolvedHirProgram) -> Result<CompilerProducts, String> {
        CompilerProducts::from_resolved_hir_with_remap(
            ProductCrateIdentity::local("test".to_string()),
            hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .map(|(products, _)| products)
    }

    #[test]
    fn product_artifact_roundtrip_preserves_complete_language_item_registry() {
        let language_items = ProductLanguageItems {
            object_owner: None,
            object_owner_allocate: None,
            object_owner_unique: None,
            fold: None,
            sized: Some(SizedLanguageItems {
                trait_id: product_def_id(17),
            }),
            drop: Some(DropLanguageItems {
                trait_id: product_def_id(23),
                method_id: product_def_id(29),
            }),
            index: Some(IndexLanguageItems {
                trait_id: product_def_id(31),
                output_id: AssocTypeId(37),
                method_id: product_def_id(41),
            }),
            index_mut: Some(IndexMutLanguageItems {
                trait_id: product_def_id(11),
                output_id: AssocTypeId(0),
                method_id: product_def_id(12),
            }),
            fn_once: None,
            fn_mut: None,
            fn_trait: None,
            send: None,
            sync: None,
            try_protocol: Some(TryLanguageItems {
                try_trait_id: product_def_id(43),
                output_id: AssocTypeId(47),
                residual_id: AssocTypeId(53),
                branch_method_id: product_def_id(59),
                from_residual_trait_id: product_def_id(61),
                from_residual_method_id: product_def_id(67),
                control_flow_enum_id: product_def_id(71),
                break_variant_id: VariantId(73),
                continue_variant_id: VariantId(79),
            }),
            range: Some(RangeLanguageItems {
                enum_id: product_def_id(83),
                full_variant_id: VariantId(0),
                from_variant_id: VariantId(1),
                to_variant_id: VariantId(2),
                to_inclusive_variant_id: VariantId(3),
                exclusive_variant_id: VariantId(4),
                inclusive_variant_id: VariantId(5),
            }),
        };
        let mut products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("language-items".to_string()),
            identity_table: ProductIdentityTable::default(),
            interface: super::ProductInterface {
                language_items: language_items.clone(),
                ..super::ProductInterface::default()
            },
            bodies: ProductBodies::default(),
            link: ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        };
        for (local_id, receiver) in [
            None,
            Some(crate::types::ReceiverMode::Shared),
            Some(crate::types::ReceiverMode::Mut),
            Some(crate::types::ReceiverMode::Move),
        ]
        .into_iter()
        .enumerate()
        {
            let local_id = local_id as u32 + 100;
            products.interface.functions.insert(
                product_def_id(local_id),
                super::ProductFunctionInterface {
                    id: DefId::new(CrateId(0), LocalDefId(local_id)),
                    name: format!("receiver_{local_id}"),
                    generic_params: Vec::new(),
                    generic_bounds: HashMap::new().into(),
                    params: Vec::new(),
                    ret_type: Type::Unit,
                    is_curried: false,
                    is_method: receiver.is_some(),
                    self_receiver: receiver,
                    is_unsafe: false,
                },
            );
        }
        let body_id = DefId::new(CrateId(0), LocalDefId(104));
        let mut body = test_function(body_id, "body_receiver", Vec::new());
        body.is_method = true;
        body.self_receiver = Some(crate::types::ReceiverMode::Move);
        products
            .bodies
            .functions
            .insert(product_def_id(104), accept_function(body));
        products.interface.insert_trait(
            product_def_id(105),
            super::ProductTraitInterface {
                target: None,
                predicates: Vec::new(),
                id: DefId::new(CrateId(0), LocalDefId(105)),
                name: "ReceiverTrait".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: BTreeMap::new(),
                signatures: HashMap::from([(
                    "receive".to_string(),
                    HirFunctionSig {
                        id: DefId::new(CrateId(0), LocalDefId(106)),
                        name: "receive".to_string(),
                        generic_params: Vec::new(),
                        params: Vec::new(),
                        ret: Type::Unit,
                        generic_bounds: HashMap::new().into(),
                        self_receiver: Some(crate::types::ReceiverMode::Mut),
                        is_unsafe: false,
                    },
                )]),
            },
        );
        products.identity_table.local_crate = Some(ProductCrateId(0));

        let bytes = products.to_artifact_bytes().expect("artifact serializes");
        let decoded = CompilerProducts::from_artifact_bytes(&bytes).expect("artifact decodes");

        assert_eq!(decoded.interface.language_items, language_items);
        assert_eq!(
            decoded
                .interface
                .functions
                .values()
                .map(|function| function.self_receiver)
                .collect::<Vec<_>>(),
            vec![
                None,
                Some(crate::types::ReceiverMode::Shared),
                Some(crate::types::ReceiverMode::Mut),
                Some(crate::types::ReceiverMode::Move),
            ]
        );
        assert_eq!(
            decoded.bodies.functions[&product_def_id(104)].self_receiver,
            Some(crate::types::ReceiverMode::Move)
        );
        assert_eq!(
            decoded.interface.traits[&product_def_id(105)].signatures["receive"].self_receiver,
            Some(crate::types::ReceiverMode::Mut)
        );
    }

    #[test]
    fn product_mapping_omits_dependency_sized_bundle() {
        let dependency_sized_trait = DefId::new(CrateId(9), LocalDefId(17));
        let hir = resolved_hir_with_language_items(
            LanguageItems {
                sized: Some(SizedLanguageItems {
                    trait_id: dependency_sized_trait,
                }),
                ..LanguageItems::default()
            },
            BTreeSet::new(),
        );

        let products = products_from_hir(&hir).expect("dependency-only bundle is omitted");

        assert_eq!(
            products.interface.language_items,
            ProductLanguageItems::default()
        );
    }

    #[test]
    fn product_mapping_maps_current_index_bundle_preserving_output_id() {
        let index_trait = DefId::new(CrateId(0), LocalDefId(17));
        let index_method = DefId::new(CrateId(0), LocalDefId(19));
        let hir = resolved_hir_with_language_items(
            LanguageItems {
                index: Some(IndexLanguageItems {
                    trait_id: index_trait,
                    output_id: AssocTypeId(23),
                    method_id: index_method,
                }),
                ..LanguageItems::default()
            },
            BTreeSet::from([index_trait, index_method]),
        );

        let products = products_from_hir(&hir).expect("current bundle maps");

        assert_eq!(
            products.interface.language_items.index,
            Some(IndexLanguageItems {
                trait_id: ProductDefId::from(index_trait),
                output_id: AssocTypeId(23),
                method_id: ProductDefId::from(index_method),
            })
        );
    }

    #[test]
    fn product_mapping_rejects_mixed_drop_bundle() {
        let current_drop_trait = DefId::new(CrateId(0), LocalDefId(17));
        let dependency_drop_method = DefId::new(CrateId(9), LocalDefId(19));
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables::default(),
            LanguageItems {
                drop: Some(DropLanguageItems {
                    trait_id: current_drop_trait,
                    method_id: dependency_drop_method,
                }),
                ..LanguageItems::default()
            },
            &HashMap::new(),
        );

        let error = super::product_language_items_from_program(
            &program,
            &BTreeSet::from([current_drop_trait]),
            &super::ProductIdRemap::new(),
        )
        .expect_err("mixed bundle is an invariant error");

        assert!(error.contains("drop"));
    }

    #[test]
    fn product_mapping_rejects_current_index_mut_without_current_index() {
        let dependency_index_trait = DefId::new(CrateId(9), LocalDefId(17));
        let dependency_index_method = DefId::new(CrateId(9), LocalDefId(19));
        let index_mut_trait = DefId::new(CrateId(0), LocalDefId(23));
        let index_mut_method = DefId::new(CrateId(0), LocalDefId(29));
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables::default(),
            LanguageItems {
                index: Some(IndexLanguageItems {
                    trait_id: dependency_index_trait,
                    output_id: AssocTypeId(0),
                    method_id: dependency_index_method,
                }),
                index_mut: Some(IndexMutLanguageItems {
                    trait_id: index_mut_trait,
                    output_id: AssocTypeId(0),
                    method_id: index_mut_method,
                }),
                ..LanguageItems::default()
            },
            &HashMap::new(),
        );

        let mut remap = super::ProductIdRemap::new();
        for id in [index_mut_trait, index_mut_method] {
            remap.insert(
                ProductDefId::from(id),
                BTreeSet::from([ProductDefId::from(id)]),
            );
        }
        let error = super::product_language_items_from_program(
            &program,
            &BTreeSet::from([index_mut_trait, index_mut_method]),
            &remap,
        )
        .expect_err("current IndexMut without current Index is an invariant error");

        assert_eq!(
            error,
            "language item index/index_mut provider pair mixes current-crate and dependency definitions"
        );
    }

    #[test]
    fn product_mapping_rejects_current_index_without_current_index_mut() {
        let index_trait = DefId::new(CrateId(0), LocalDefId(17));
        let index_method = DefId::new(CrateId(0), LocalDefId(19));
        let dependency_index_mut_trait = DefId::new(CrateId(9), LocalDefId(23));
        let dependency_index_mut_method = DefId::new(CrateId(9), LocalDefId(29));
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables::default(),
            LanguageItems {
                index: Some(IndexLanguageItems {
                    trait_id: index_trait,
                    output_id: AssocTypeId(0),
                    method_id: index_method,
                }),
                index_mut: Some(IndexMutLanguageItems {
                    trait_id: dependency_index_mut_trait,
                    output_id: AssocTypeId(0),
                    method_id: dependency_index_mut_method,
                }),
                ..LanguageItems::default()
            },
            &HashMap::new(),
        );

        let mut remap = super::ProductIdRemap::new();
        for id in [index_trait, index_method] {
            remap.insert(
                ProductDefId::from(id),
                BTreeSet::from([ProductDefId::from(id)]),
            );
        }
        let error = super::product_language_items_from_program(
            &program,
            &BTreeSet::from([index_trait, index_method]),
            &remap,
        )
        .expect_err("current Index without current IndexMut is an invariant error");

        assert_eq!(
            error,
            "language item index/index_mut provider pair mixes current-crate and dependency definitions"
        );
    }

    #[test]
    fn product_mapping_maps_current_index_mut_bundle_preserving_output_id() {
        let index_trait = DefId::new(CrateId(0), LocalDefId(17));
        let index_method = DefId::new(CrateId(0), LocalDefId(19));
        let index_mut_trait = DefId::new(CrateId(0), LocalDefId(23));
        let index_mut_method = DefId::new(CrateId(0), LocalDefId(29));
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables::default(),
            LanguageItems {
                index: Some(IndexLanguageItems {
                    trait_id: index_trait,
                    output_id: AssocTypeId(31),
                    method_id: index_method,
                }),
                index_mut: Some(IndexMutLanguageItems {
                    trait_id: index_mut_trait,
                    output_id: AssocTypeId(37),
                    method_id: index_mut_method,
                }),
                ..LanguageItems::default()
            },
            &HashMap::new(),
        );
        let mut remap = super::ProductIdRemap::new();
        for id in [index_trait, index_method, index_mut_trait, index_mut_method] {
            remap.insert(
                ProductDefId::from(id),
                BTreeSet::from([ProductDefId::from(id)]),
            );
        }

        let items = super::product_language_items_from_program(
            &program,
            &BTreeSet::from([index_trait, index_method, index_mut_trait, index_mut_method]),
            &remap,
        )
        .expect("current index pair maps");

        assert_eq!(
            items.index_mut,
            Some(IndexMutLanguageItems {
                trait_id: ProductDefId::from(index_mut_trait),
                output_id: AssocTypeId(37),
                method_id: ProductDefId::from(index_mut_method),
            })
        );
    }

    #[test]
    fn remap_expr_location_product_ids_remaps_new_reference_sidecars() {
        let old_struct = DefId::new(CrateId(0), LocalDefId(1));
        let old_function = DefId::new(CrateId(0), LocalDefId(2));
        let old_extern = DefId::new(CrateId(0), LocalDefId(3));
        let new_struct = DefId::new(CrateId(7), LocalDefId(11));
        let new_function = DefId::new(CrateId(7), LocalDefId(12));
        let new_extern = DefId::new(CrateId(7), LocalDefId(13));
        let mut remap = BTreeMap::new();
        remap.insert(
            ProductDefId::from(old_struct),
            BTreeSet::from([ProductDefId::from(new_struct)]),
        );
        remap.insert(
            ProductDefId::from(old_function),
            BTreeSet::from([ProductDefId::from(new_function)]),
        );
        remap.insert(
            ProductDefId::from(old_extern),
            BTreeSet::from([ProductDefId::from(new_extern)]),
        );

        let mut struct_expr = HirExpr {
            kind: crate::hir::HirExprKindFor::StructLiteral(
                "Box".to_string(),
                Some(old_struct),
                vec![HirStructLiteralField {
                    name: "value".to_string(),
                    value: HirExpr {
                        kind: crate::hir::HirExprKindFor::ResolvedVar(HirVarRef {
                            name: "make".to_string(),
                            target: HirVarTarget::Function(old_function),
                        }),
                        ty: Type::I64,
                        span: Span::test(),
                    },
                    field: Some(HirFieldLocation {
                        owner: old_struct,
                        field_id: FieldId(0),
                        name: "value".to_string(),
                    }),
                }],
            ),
            ty: Type::I64,
            span: Span::test(),
        };
        let mut extern_expr = HirExpr {
            kind: crate::hir::HirExprKindFor::ResolvedVar(HirVarRef {
                name: "puts".to_string(),
                target: HirVarTarget::Extern(old_extern),
            }),
            ty: Type::I64,
            span: Span::test(),
        };

        super::remap_expr_location_product_ids(&mut struct_expr, &remap);
        super::remap_expr_location_product_ids(&mut extern_expr, &remap);

        let crate::hir::HirExprKindFor::StructLiteral(_, Some(struct_id), fields) =
            &struct_expr.kind
        else {
            panic!("expected struct literal");
        };
        assert_eq!(*struct_id, new_struct);
        assert_eq!(fields[0].field.as_ref().unwrap().owner, new_struct);
        let crate::hir::HirExprKindFor::ResolvedVar(reference) = &fields[0].value.kind else {
            panic!("expected resolved var field value");
        };
        assert_eq!(reference.target, HirVarTarget::Function(new_function));
        let crate::hir::HirExprKindFor::ResolvedVar(reference) = &extern_expr.kind else {
            panic!("expected resolved extern var");
        };
        assert_eq!(reference.target, HirVarTarget::Extern(new_extern));
    }

    #[test]
    fn remap_expr_location_product_ids_remaps_call_target_sidecars() {
        let old_function = DefId::new(CrateId(0), LocalDefId(2));
        let old_extern = DefId::new(CrateId(0), LocalDefId(3));
        let new_function = DefId::new(CrateId(7), LocalDefId(12));
        let new_extern = DefId::new(CrateId(7), LocalDefId(13));
        let mut remap = BTreeMap::new();
        remap.insert(
            ProductDefId::from(old_function),
            BTreeSet::from([ProductDefId::from(new_function)]),
        );
        remap.insert(
            ProductDefId::from(old_extern),
            BTreeSet::from([ProductDefId::from(new_extern)]),
        );

        let mut function_call = HirExpr {
            kind: crate::hir::HirExprKindFor::Call(
                Box::new(HirExpr {
                    kind: crate::hir::HirExprKindFor::Var("make".to_string()),
                    ty: Type::function(Vec::new(), Type::I64),
                    span: Span::test(),
                }),
                Vec::new(),
                Some(HirCallTarget::Function(old_function)),
            ),
            ty: Type::I64,
            span: Span::test(),
        };
        let mut extern_call = HirExpr {
            kind: crate::hir::HirExprKindFor::Call(
                Box::new(HirExpr {
                    kind: crate::hir::HirExprKindFor::Var("puts".to_string()),
                    ty: Type::function(Vec::new(), Type::I64),
                    span: Span::test(),
                }),
                Vec::new(),
                Some(HirCallTarget::Extern(old_extern)),
            ),
            ty: Type::I64,
            span: Span::test(),
        };

        super::remap_expr_location_product_ids(&mut function_call, &remap);
        super::remap_expr_location_product_ids(&mut extern_call, &remap);

        let crate::hir::HirExprKindFor::Call(_, _, Some(function_target)) = &function_call.kind
        else {
            panic!("expected function call target");
        };
        assert_eq!(function_target, &HirCallTarget::Function(new_function));
        let crate::hir::HirExprKindFor::Call(_, _, Some(extern_target)) = &extern_call.kind else {
            panic!("expected extern call target");
        };
        assert_eq!(extern_target, &HirCallTarget::Extern(new_extern));
    }

    #[test]
    fn owned_object_walkers_preserve_operands_types_and_protocol_identities() {
        use crate::hir::{
            HirExprKindFor, HirMethodCallTarget, HirOwnedObjectCall, HirStaticMethodTarget,
            HirTraitDispatchKind, HirTypeBinding,
        };

        let old = DefId::new(CrateId(0), LocalDefId(2));
        let new = DefId::new(CrateId(0), LocalDefId(12));
        let parameter = crate::types::GenericParamId {
            owner: old,
            index: 0,
        };
        let ty = Type::Generic(parameter);
        let mut method = HirMethodCallTarget::trait_method(
            old,
            old,
            vec![ty.clone()],
            HirTraitDispatchKind::Object,
        );
        method.owner_substitution.push(HirTypeBinding {
            param: parameter,
            ty: ty.clone(),
        });
        method.method_substitution.push(HirTypeBinding {
            param: parameter,
            ty: ty.clone(),
        });
        let operation = HirStaticMethodTarget {
            owner_ty: ty.clone(),
            method: method.clone(),
        };
        let operand = HirExpr {
            kind: HirExprKindFor::Call(
                Box::new(HirExpr {
                    kind: HirExprKindFor::Var("operand".into()),
                    ty: ty.clone(),
                    span: Span::test(),
                }),
                Vec::new(),
                Some(HirCallTarget::Function(old)),
            ),
            ty: ty.clone(),
            span: Span::test(),
        };
        // This is a traversal fixture, not a forged executable ownership proof.
        let mut expression = HirExpr {
            kind: HirExprKindFor::OwnedObjectCall {
                owner: Box::new(operand.clone()),
                args: vec![operand],
                call: HirOwnedObjectCall {
                    method,
                    object: ty.clone(),
                    state: ty,
                    into_parts: operation.clone(),
                    release: operation,
                },
            },
            ty: Type::Unit,
            span: Span::test(),
        };
        super::assert_valid_product_expr_ids(&expression);
        super::remap_expr_owned_type_ids(&mut expression, old, new);
        let remap = BTreeMap::from([(
            ProductDefId::from(old),
            BTreeSet::from([ProductDefId::from(new)]),
        )]);
        super::remap_expr_location_product_ids(&mut expression, &remap);
        let HirExprKindFor::OwnedObjectCall { owner, args, call } = &mut expression.kind else {
            panic!("owned call");
        };
        for operand in [owner.as_ref(), &args[0]] {
            assert_eq!(
                operand.ty,
                Type::Generic(crate::types::GenericParamId {
                    owner: new,
                    index: 0
                })
            );
            assert!(
                matches!(operand.kind, HirExprKindFor::Call(_, _, Some(HirCallTarget::Function(id))) if id == new)
            );
        }
        call.visit_types(&mut |ty| {
            assert_eq!(
                *ty,
                Type::Generic(crate::types::GenericParamId {
                    owner: new,
                    index: 0
                })
            );
        });
        call.for_each_def_id_mut(|id| assert_eq!(*id, new));
        let mut visited = 0;
        super::remap_expr_types(&mut expression, &mut |ty| {
            if matches!(ty, Type::Generic(_)) {
                *ty = Type::I64;
                visited += 1;
            }
        });
        assert_eq!(visited, 17); // Four operand types and thirteen proof types.
        let HirExprKindFor::OwnedObjectCall { owner, args, call } = &expression.kind else {
            panic!("owned call");
        };
        assert_eq!(owner.ty, Type::I64);
        assert_eq!(args[0].ty, Type::I64);
        call.visit_types(&mut |ty| assert_eq!(*ty, Type::I64));
    }

    #[test]
    fn opened_object_walkers_preserve_scoped_binder_and_dispatch_identity() {
        use crate::hir::{
            HirExprKindFor, HirMethodCallTarget, HirOpenBinding, HirParam, HirSelectedMethodTarget,
            HirTraitDispatchKind, HirVarRef, HirVarTarget,
        };
        use crate::types::{ObjectType, TraitBound, WitnessId};

        let old = DefId::new(CrateId(0), LocalDefId(2));
        let reserved = DefId::new(CrateId(0), LocalDefId(12));
        let imported = DefId::new(CrateId(7), LocalDefId(12));
        let trait_id = DefId::new(CrateId(0), LocalDefId(40));
        let witness = WitnessId {
            owner: old,
            local: crate::ids::HirLocalId(3),
        };
        let object = ObjectType::new(TraitBound {
            trait_id,
            type_args: vec![],
        });
        let source_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Object(Box::new(object.clone()))),
        };
        let value_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Witness(witness)),
        };
        let value = HirParam {
            name: "value".into(),
            local_id: crate::ids::HirLocalId(4),
            ty: value_ty.clone(),
            mutable: false,
            is_ref: true,
        };
        let call = HirExpr {
            kind: HirExprKindFor::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKindFor::ResolvedVar(HirVarRef {
                        name: value.name.clone(),
                        target: HirVarTarget::Local(value.local_id),
                    }),
                    ty: value_ty,
                    span: Span::test(),
                }),
                "read".into(),
                vec![],
                Some(crate::types::ReceiverMode::Shared),
                Some(HirMethodCallTarget::trait_method(
                    trait_id,
                    DefId::new(CrateId(0), LocalDefId(41)),
                    vec![],
                    HirTraitDispatchKind::Opened(witness),
                )),
            ),
            ty: Type::I64,
            span: Span::test(),
        };
        // A traversal fixture; full binder/evidence admission belongs to HIR.
        let mut expression = HirExpr {
            kind: HirExprKindFor::Open {
                source: Box::new(HirExpr {
                    kind: HirExprKindFor::ResolvedVar(HirVarRef {
                        name: "object".into(),
                        target: HirVarTarget::Local(crate::ids::HirLocalId(0)),
                    }),
                    ty: source_ty.clone(),
                    span: Span::test(),
                }),
                binding: HirOpenBinding {
                    witness,
                    value,
                    object,
                    source_ty,
                    owner: None,
                },
                body: HirBlock {
                    stmts: vec![crate::hir::HirStmtFor::Expr(call)],
                    ty: Type::I64,
                },
            },
            ty: Type::I64,
            span: Span::test(),
        };
        super::assert_valid_product_expr_ids(&expression);
        super::remap_expr_owned_type_ids(&mut expression, old, reserved);
        super::remap_expr_types(&mut expression, &mut |ty| {
            ty.remap_def_ids(&mut |id| if id == reserved { imported } else { id });
        });
        super::remap_expr_location_product_ids(
            &mut expression,
            &BTreeMap::from([(
                ProductDefId::from(reserved),
                BTreeSet::from([ProductDefId::from(imported)]),
            )]),
        );
        let HirExprKindFor::Open { binding, body, .. } = &expression.kind else {
            panic!("opening");
        };
        let expected = WitnessId {
            owner: imported,
            ..witness
        };
        assert_eq!(binding.witness, expected);
        assert_eq!(binding.value.local_id, crate::ids::HirLocalId(4));
        assert_eq!(
            binding.value.ty,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Witness(expected)),
            }
        );
        let crate::hir::HirStmtFor::Expr(call) = &body.stmts[0] else {
            panic!("opened call");
        };
        let HirExprKindFor::MethodCall(receiver, _, _, _, Some(target)) = &call.kind else {
            panic!("opened member authority");
        };
        assert_eq!(receiver.ty, binding.value.ty);
        assert!(matches!(
            target.target,
            HirSelectedMethodTarget::TraitMethod {
                dispatch: HirTraitDispatchKind::Opened(actual),
                ..
            } if actual == expected
        ));
        super::assert_valid_product_expr_ids(&expression);
    }

    #[test]
    fn remap_pattern_owned_type_ids_remaps_new_pattern_sidecars() {
        let old_id = DefId::new(CrateId(0), LocalDefId(1));
        let new_id = DefId::new(CrateId(0), LocalDefId(2));
        let mut pattern = HirPattern::Enum(
            "Option".to_string(),
            "Some".to_string(),
            Some(HirVariantLocation {
                owner: old_id,
                variant_id: VariantId(0),
                name: "Some".to_string(),
            }),
            vec![HirPattern::Struct(
                "Box".to_string(),
                Some(old_id),
                vec![Type::Struct {
                    id: old_id,
                    args: Vec::new(),
                }],
                vec![HirStructPatternField {
                    name: "value".to_string(),
                    field: Some(HirFieldLocation {
                        owner: old_id,
                        field_id: FieldId(0),
                        name: "value".to_string(),
                    }),
                    pattern: HirPattern::Wildcard,
                }],
            )],
        );

        super::remap_pattern_owned_type_ids(&mut pattern, old_id, new_id);

        let HirPattern::Enum(_, _, Some(location), payloads) = pattern else {
            panic!("expected enum pattern");
        };
        assert_eq!(location.owner, new_id);
        let HirPattern::Struct(_, Some(struct_id), args, fields) = &payloads[0] else {
            panic!("expected struct payload pattern");
        };
        assert_eq!(*struct_id, new_id);
        assert_eq!(
            args[0],
            Type::Struct {
                id: old_id,
                args: Vec::new(),
            }
        );
        assert_eq!(fields[0].field.as_ref().unwrap().owner, new_id);
    }

    #[test]
    fn remap_expr_location_product_ids_remaps_match_pattern_sidecars() {
        let old_id = DefId::new(CrateId(0), LocalDefId(1));
        let new_id = DefId::new(CrateId(7), LocalDefId(2));
        let mut remap = BTreeMap::new();
        remap.insert(
            ProductDefId::from(old_id),
            BTreeSet::from([ProductDefId::from(new_id)]),
        );
        let mut expr = HirExpr {
            kind: crate::hir::HirExprKindFor::Match {
                scrutinee: Box::new(HirExpr {
                    kind: crate::hir::HirExprKindFor::Var("value".to_string()),
                    ty: Type::I64,
                    span: Span::test(),
                }),
                arms: vec![crate::hir::HirMatchArm {
                    pattern: HirPattern::Enum(
                        "Option".to_string(),
                        "Some".to_string(),
                        Some(HirVariantLocation {
                            owner: old_id,
                            variant_id: VariantId(0),
                            name: "Some".to_string(),
                        }),
                        vec![HirPattern::Struct(
                            "Box".to_string(),
                            Some(old_id),
                            Vec::new(),
                            vec![HirStructPatternField {
                                name: "value".to_string(),
                                field: Some(HirFieldLocation {
                                    owner: old_id,
                                    field_id: FieldId(0),
                                    name: "value".to_string(),
                                }),
                                pattern: HirPattern::Wildcard,
                            }],
                        )],
                    ),
                    guard: None,
                    body: HirBlock {
                        stmts: Vec::new(),
                        ty: Type::Unit,
                    },
                }],
            },
            ty: Type::Unit,
            span: Span::test(),
        };

        super::remap_expr_location_product_ids(&mut expr, &remap);

        let crate::hir::HirExprKindFor::Match { arms, .. } = &expr.kind else {
            panic!("expected match expression");
        };
        let HirPattern::Enum(_, _, Some(location), payloads) = &arms[0].pattern else {
            panic!("expected enum pattern");
        };
        assert_eq!(location.owner, new_id);
        let HirPattern::Struct(_, Some(struct_id), _, fields) = &payloads[0] else {
            panic!("expected struct pattern");
        };
        assert_eq!(*struct_id, new_id);
        assert_eq!(fields[0].field.as_ref().unwrap().owner, new_id);
    }

    fn rebuild_program(
        program: &mut crate::hir::AcceptedHirProgram,
        mutate: impl FnOnce(
            &mut HashMap<DefId, AcceptedHirFunction>,
            &mut HashMap<DefId, HirStruct>,
            &mut HashMap<DefId, HirEnum>,
            &mut HashMap<DefId, HirTraitFor<AcceptedHir>>,
            &mut HashMap<DefId, HirImplFor<AcceptedHir>>,
            &mut HashMap<DefId, HirExtern>,
            &mut crate::hir::HirNameTables,
            &mut HashMap<(DefId, DefId), DefId>,
        ),
    ) {
        let source = program.program();
        let mut canonical_names_by_id = source.indexes.functions_by_id.clone();
        canonical_names_by_id.extend(source.indexes.structs_by_id.clone());
        canonical_names_by_id.extend(source.indexes.enums_by_id.clone());
        canonical_names_by_id.extend(source.indexes.traits_by_id.clone());
        for function in source.functions.values() {
            canonical_names_by_id
                .entry(function.id)
                .or_insert_with(|| function.name.clone());
        }
        let mut effective_trait_methods = source.indexes.effective_trait_methods.clone();
        let mut functions = source.functions.clone();
        let mut structs = source.structs.clone();
        let mut enums = source.enums.clone();
        let mut traits = source.traits.clone();
        let mut impls = source.impls.clone();
        let mut externs = source.externs.clone();
        let mut names = source.names.clone();

        mutate(
            &mut functions,
            &mut structs,
            &mut enums,
            &mut traits,
            &mut impls,
            &mut externs,
            &mut names,
            &mut effective_trait_methods,
        );
        let mut rebuilt =
            HirProgramFor::<AcceptedHir>::from_accepted_id_parts_with_names_and_canonical_names(
                functions,
                structs,
                enums,
                traits,
                impls,
                externs,
                names,
                &canonical_names_by_id,
            );
        rebuilt
            .indexes
            .effective_trait_methods
            .extend(effective_trait_methods);
        rebuilt
            .indexes
            .effective_trait_methods
            .retain(|(impl_id, _), _| rebuilt.impls.contains_key(impl_id));
        *program = crate::hir::AcceptedHirProgram::revalidate_for_test(rebuilt)
            .expect("rebuilt product fixture must satisfy accepted HIR invariants");
    }

    fn mutate_program(
        program: &mut crate::hir::AcceptedHirProgram,
        mutate: impl FnOnce(&mut HirProgramFor<AcceptedHir>),
    ) {
        let mut mutated = program.program().clone();
        mutate(&mut mutated);
        *program = crate::hir::AcceptedHirProgram::revalidate_for_test(mutated)
            .expect("mutated product fixture must satisfy accepted HIR invariants");
    }

    fn int_body() -> HirBlock {
        HirBlock {
            stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
                kind: crate::hir::HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            })],
            ty: Type::I64,
        }
    }

    fn empty_body(ty: Type) -> HirBlock {
        HirBlock {
            stmts: Vec::new(),
            ty,
        }
    }

    fn test_function(id: DefId, name: &str, generic_params: Vec<GenericParamDecl>) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params,
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::I64,
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I64,
            body: int_body(),
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn accept_function(function: HirFunction) -> AcceptedHirFunction {
        let id = function.id;
        let name = function.name.clone();
        let accepted = crate::hir::AcceptedHirProgram::try_from(
            HirProgram::from_id_parts_with_names_and_canonical_names(
                HashMap::from([(id, function)]),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HirNameTables {
                    functions_by_name: HashMap::from([(name.clone(), id)]),
                    ..HirNameTables::default()
                },
                HirLanguageItems::default(),
                &HashMap::from([(id, name)]),
            ),
        )
        .expect("fixture function is accepted");
        accepted.function_by_id(id).unwrap().1.clone()
    }

    fn local_def_ids_after(next_raw: u32) -> IdGen<LocalDefId> {
        let mut ids = IdGen::new();
        for _ in 0..next_raw {
            ids.fresh();
        }
        ids
    }

    fn resolved_hir_for_products() -> ResolvedHirProgram {
        let plain_id = DefId::new(CrateId(0), LocalDefId(0));
        let generic_id = DefId::new(CrateId(0), LocalDefId(1));
        let struct_id = DefId::new(CrateId(0), LocalDefId(2));
        let enum_id = DefId::new(CrateId(0), LocalDefId(3));
        let trait_id = DefId::new(CrateId(0), LocalDefId(4));
        let impl_id = DefId::new(CrateId(0), LocalDefId(5));
        let extern_id = DefId::new(CrateId(0), LocalDefId(6));

        let mut functions = HashMap::new();
        functions.insert(plain_id, test_function(plain_id, "plain", Vec::new()));
        functions.insert(
            generic_id,
            test_function(generic_id, "identity", generic_params(generic_id, &["T"])),
        );

        let mut structs = HashMap::new();
        structs.insert(
            struct_id,
            HirStruct {
                id: struct_id,
                name: "Box".to_string(),
                generic_params: generic_params(struct_id, &["T"]),
                fields: Vec::new(),
            },
        );

        let mut enums = HashMap::new();
        enums.insert(
            enum_id,
            HirEnum {
                id: enum_id,
                name: "Maybe".to_string(),
                generic_params: generic_params(enum_id, &["T"]),
                variants: Vec::new(),
            },
        );

        let default_method_id = DefId::new(CrateId(0), LocalDefId(7));
        let default_method = test_function(default_method_id, "show", Vec::new());
        let effective_method_id = DefId::new(CrateId(0), LocalDefId(8));
        let mut effective_method = default_method.clone();
        effective_method.id = effective_method_id;
        let mut trait_methods = HashMap::new();
        trait_methods.insert("show".to_string(), default_method.clone());
        let mut traits = HashMap::new();
        traits.insert(
            trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Show".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: trait_methods,
                signatures: HashMap::new(),
            },
        );

        let impls = HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: generic_params(impl_id, &["T"]),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: vec![Type::Generic(crate::types::GenericParamId {
                        owner: impl_id,
                        index: 0,
                    })],
                }),
                trait_name: Some("Show".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("show".to_string(), effective_method)]),
            },
        )]);

        let externs = HashMap::from([(
            extern_id,
            HirExtern {
                id: extern_id,
                name: "puts".to_string(),
                params: vec![Type::Pointer(Box::new(Type::U8))],
                ret: Type::I32,
                variadic: false,
                is_unsafe: false,
            },
        )]);

        let mut program = HirProgram::from_id_parts_with_names_and_canonical_names(
            functions,
            structs,
            enums,
            traits,
            impls,
            externs,
            HirNameTables {
                functions_by_name: HashMap::from([
                    ("plain".to_string(), plain_id),
                    ("identity".to_string(), generic_id),
                ]),
                structs_by_name: HashMap::from([("Box".to_string(), struct_id)]),
                enums_by_name: HashMap::from([("Maybe".to_string(), enum_id)]),
                traits_by_name: HashMap::from([("Show".to_string(), trait_id)]),
                externs_by_name: HashMap::from([("puts".to_string(), extern_id)]),
                type_aliases_by_name: HashMap::new(),
            },
            HirLanguageItems::default(),
            &HashMap::from([
                (plain_id, "plain".to_string()),
                (generic_id, "identity".to_string()),
                (struct_id, "Box".to_string()),
                (enum_id, "Maybe".to_string()),
                (trait_id, "Show".to_string()),
            ]),
        );
        program
            .indexes
            .effective_trait_methods
            .insert((impl_id, default_method_id), effective_method_id);

        ResolvedHirProgram::new(
            program,
            ResolverTables::default(),
            (0..9)
                .map(|local| DefId::new(CrateId(0), LocalDefId(local)))
                .collect(),
            CrateId(0),
            local_def_ids_after(9),
        )
    }

    fn resolved_hir_with_non_generic_impl_generic_method() -> ResolvedHirProgram {
        let struct_id = DefId::new(CrateId(0), LocalDefId(41));
        let impl_id = DefId::new(CrateId(0), LocalDefId(42));
        let method_id = DefId::new(CrateId(0), LocalDefId(43));
        let method_generic = crate::types::GenericParamId {
            owner: method_id,
            index: 0,
        };

        let method = HirFunction {
            id: method_id,
            name: "id".to_string(),
            generic_params: vec![GenericParamDecl::type_param(method_generic, "T")],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(method_generic),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Generic(method_generic),
            body: HirBlock {
                stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
                    kind: crate::hir::HirExprKindFor::Var("value".to_string()),
                    ty: Type::Generic(method_generic),
                    span: Span::test(),
                })],
                ty: Type::Generic(method_generic),
            },
            is_curried: false,
            is_method: true,
            self_receiver: None,
            is_unsafe: false,
        };

        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::from([(
                struct_id,
                HirStruct {
                    id: struct_id,
                    name: "Foo".to_string(),
                    generic_params: Vec::new(),
                    fields: Vec::new(),
                },
            )]),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                impl_id,
                HirImpl {
                    id: impl_id,
                    owner: crate::hir::HirImplOwner::Named("Foo".to_string()),
                    type_name: "Foo".to_string(),
                    type_generics: Vec::new(),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: struct_id,
                        args: Vec::new(),
                    }),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::from([("id".to_string(), method)]),
                },
            )]),
            HashMap::new(),
            HirNameTables {
                structs_by_name: HashMap::from([("Foo".to_string(), struct_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([(struct_id, "Foo".to_string())]),
        );
        ResolvedHirProgram::new(
            program,
            ResolverTables::default(),
            BTreeSet::from([struct_id, impl_id, method_id]),
            CrateId(0),
            local_def_ids_after(44),
        )
    }

    #[test]
    fn compiler_products_keep_static_impl_methods_out_of_top_level_function_payloads() {
        let struct_id = DefId::new(CrateId(0), LocalDefId(100));
        let impl_id = DefId::new(CrateId(0), LocalDefId(101));
        let method_id = DefId::new(CrateId(0), LocalDefId(102));
        let mut method = test_function(method_id, "new", Vec::new());
        method.is_method = true;
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::from([(
                struct_id,
                HirStruct {
                    id: struct_id,
                    name: "Box".to_string(),
                    generic_params: Vec::new(),
                    fields: Vec::new(),
                },
            )]),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                impl_id,
                HirImpl {
                    id: impl_id,
                    owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: Vec::new(),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: struct_id,
                        args: Vec::new(),
                    }),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: HashMap::new().into(),
                    methods: HashMap::from([("new".to_string(), method)]),
                },
            )]),
            HashMap::new(),
            HirNameTables {
                structs_by_name: HashMap::from([("Box".to_string(), struct_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([(struct_id, "Box".to_string())]),
        );
        let hir = ResolvedHirProgram::new(
            program,
            ResolverTables::default(),
            BTreeSet::from([struct_id, impl_id, method_id]),
            CrateId(0),
            local_def_ids_after(103),
        );
        let method_product_id = ProductDefId::from(method_id);
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData {
                object_path: None,
                records: BTreeMap::from([(
                    method_product_id,
                    ProductLinkRecord {
                        backend_symbol: "box_new".to_string(),
                    },
                )]),
            },
        )
        .expect("test HIR has valid product language items");

        assert!(!products
            .interface
            .functions
            .contains_key(&method_product_id));
        assert!(!products.bodies.functions.contains_key(&method_product_id));
        assert_eq!(
            products.interface.impls[&ProductDefId::from(impl_id)].methods["new"].id,
            method_id
        );
        assert_eq!(
            products.link.records[&method_product_id].backend_symbol,
            "box_new"
        );
    }

    #[test]
    fn product_bodies_only_include_downstream_specialized_functions() {
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &resolved_hir_for_products(),
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let plain_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(0),
        };
        let generic_id = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(1),
        };

        let plain_decl = products
            .interface
            .functions
            .get(&plain_id)
            .expect("plain function interface row");
        assert_eq!(plain_decl.id, DefId::new(CrateId(0), LocalDefId(0)));
        assert_eq!(plain_decl.name, "plain");
        assert_eq!(plain_decl.params, vec![Type::I64]);
        assert_eq!(plain_decl.ret_type, Type::I64);
        assert!(!products.bodies.functions.contains_key(&plain_id));

        assert!(products.interface.functions.contains_key(&generic_id));
        assert!(products.bodies.functions.contains_key(&generic_id));
    }

    fn resolved_hir_with_traits_in_order(names: &[&str]) -> ResolvedHirProgram {
        let mut traits = HashMap::new();

        for (index, name) in names.iter().enumerate() {
            let method_id = match *name {
                "Show" => DefId::new(CrateId(0), LocalDefId(100)),
                "Debug" => DefId::new(CrateId(0), LocalDefId(101)),
                _ => DefId::new(CrateId(0), LocalDefId(200 + index as u32)),
            };
            let mut methods = HashMap::new();
            methods.insert(
                "show".to_string(),
                test_function(method_id, "show", Vec::new()),
            );
            traits.insert(
                DefId::new(CrateId(0), LocalDefId(index as u32)),
                HirTrait {
                    target: None,
                    predicates: Vec::new(),
                    id: DefId::new(CrateId(0), LocalDefId(index as u32)),
                    name: (*name).to_string(),
                    generic_params: Vec::new(),
                    associated_types: Vec::new(),
                    methods,
                    signatures: HashMap::new(),
                },
            );
        }

        ResolvedHirProgram::new(
            HirProgram::from_id_parts_with_names_and_canonical_names(
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                traits,
                HashMap::new(),
                HashMap::new(),
                HirNameTables {
                    traits_by_name: names
                        .iter()
                        .enumerate()
                        .map(|(index, name)| {
                            (
                                (*name).to_string(),
                                DefId::new(CrateId(0), LocalDefId(index as u32)),
                            )
                        })
                        .collect(),
                    ..HirNameTables::default()
                },
                HirLanguageItems::default(),
                &names
                    .iter()
                    .enumerate()
                    .map(|(index, name)| {
                        (
                            DefId::new(CrateId(0), LocalDefId(index as u32)),
                            (*name).to_string(),
                        )
                    })
                    .collect(),
            ),
            ResolverTables::default(),
            names
                .iter()
                .enumerate()
                .map(|(index, _)| DefId::new(CrateId(0), LocalDefId(index as u32)))
                .chain(names.iter().filter_map(|name| match *name {
                    "Show" => Some(DefId::new(CrateId(0), LocalDefId(100))),
                    "Debug" => Some(DefId::new(CrateId(0), LocalDefId(101))),
                    _ => None,
                }))
                .collect(),
            CrateId(0),
            IdGen::new(),
        )
    }

    fn refresh_resolved_type_ids(resolved: &mut ResolvedHirProgram) {
        resolved.type_context = crate::type_context::TypeContext::new();
        resolved.type_ids =
            crate::hir::collect_hir_type_ids(&resolved.program, &mut resolved.type_context);
    }

    #[test]
    fn compiler_products_key_interface_by_product_def_id() {
        let hir = resolved_hir_for_products();

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));
        let extern_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(6)));

        assert!(products.interface.functions.contains_key(&plain_id));
        assert!(products.interface.externs.contains_key(&extern_id));
        assert_eq!(
            products.identity_table.export_names.get("plain"),
            Some(&plain_id)
        );
        assert_eq!(
            products.identity_table.display_names.get(&plain_id),
            Some(&"plain".to_string())
        );
    }

    #[test]
    fn compiler_products_persist_resolver_aliases_by_product_id() {
        let alias_id = DefId::new(CrateId(0), LocalDefId(4));
        let mut hir = resolved_hir_for_products();
        hir.resolver.insert_import_alias_with_name(
            "short".to_string(),
            "demo::plain".to_string(),
            alias_id,
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        assert_eq!(
            products.identity_table.import_alias_names.get("short"),
            Some(&ProductDefId::from(alias_id))
        );
    }

    #[test]
    fn compiler_products_do_not_export_dependency_aliases_as_current_roots() {
        let dependency_id = DefId::new(CrateId(1), LocalDefId(42));
        let mut hir = resolved_hir_for_products();
        hir.resolver.insert_export_alias_with_name(
            "stdlib::alloc::Global".to_string(),
            "stdlib::alloc::Global".to_string(),
            dependency_id,
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        assert!(!products
            .identity_table
            .export_names
            .contains_key("stdlib::alloc::Global"));
    }

    #[test]
    fn compiler_products_record_prelude_export_ids() {
        let hir = resolved_hir_for_products();
        let mut products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));

        products.record_prelude_export_ids([(
            "plain".to_string(),
            crate::crate_artifact::ArtifactExport {
                source: "plain".to_string(),
                id: DefId::new(CrateId(0), LocalDefId(0)),
            },
        )]);

        assert_eq!(
            products.identity_table.prelude_export_names.get("plain"),
            Some(&plain_id)
        );
    }

    #[test]
    fn compiler_products_record_prelude_export_ids_without_string_payload() {
        let hir = resolved_hir_for_products();
        let mut products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let plain_def_id = DefId::new(CrateId(0), LocalDefId(0));
        let plain_product_id = ProductDefId::from(plain_def_id);

        products.record_prelude_export_ids([(
            "plain".to_string(),
            crate::crate_artifact::ArtifactExport {
                source: "plain".to_string(),
                id: plain_def_id,
            },
        )]);

        assert_eq!(
            products.identity_table.prelude_export_names.get("plain"),
            Some(&plain_product_id)
        );
    }

    #[test]
    fn compiler_products_roundtrip_preserves_prelude_export_ids() {
        let hir = resolved_hir_for_products();
        let mut products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));
        products.record_prelude_export_ids([(
            "plain".to_string(),
            crate::crate_artifact::ArtifactExport {
                source: "plain".to_string(),
                id: DefId::new(CrateId(0), LocalDefId(0)),
            },
        )]);

        let bytes = products.to_artifact_bytes().unwrap();
        let decoded = CompilerProducts::from_artifact_bytes(&bytes).unwrap();

        assert_eq!(
            decoded.identity_table.prelude_export_names.get("plain"),
            Some(&plain_id)
        );
    }

    #[test]
    fn compiler_products_record_prelude_export_ids_prefer_explicit_export_id() {
        let hir = resolved_hir_for_products();
        let mut products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let plain_def_id = DefId::new(CrateId(0), LocalDefId(0));
        let plain_id = ProductDefId::from(plain_def_id);

        products.record_prelude_export_ids([(
            "prelude_alias".to_string(),
            crate::crate_artifact::ArtifactExport {
                source: "plain".to_string(),
                id: plain_def_id,
            },
        )]);

        assert_eq!(
            products
                .identity_table
                .prelude_export_names
                .get("prelude_alias"),
            Some(&plain_id)
        );
    }

    #[test]
    fn compiler_products_record_prelude_export_ids_fall_back_on_mismatched_source_id() {
        let hir = resolved_hir_for_products();
        let mut products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let plain_def_id = DefId::new(CrateId(0), LocalDefId(0));
        let identity_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(1)));

        products.record_prelude_export_ids([(
            "prelude_alias".to_string(),
            crate::crate_artifact::ArtifactExport {
                source: "identity".to_string(),
                id: plain_def_id,
            },
        )]);

        assert_eq!(
            products
                .identity_table
                .prelude_export_names
                .get("prelude_alias"),
            Some(&identity_id)
        );
    }

    #[test]
    fn compiler_products_record_prelude_export_ids_skip_non_item_explicit_ids() {
        let hir = resolved_hir_for_products();
        let mut products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let impl_def_id = DefId::new(CrateId(0), LocalDefId(5));
        let impl_id = ProductDefId::from(impl_def_id);

        assert!(products.interface.impls.contains_key(&impl_id));
        assert_eq!(
            products.identity_table.display_names.get(&impl_id),
            Some(&"Box as Show".to_string())
        );

        products.record_prelude_export_ids([(
            "impl_alias".to_string(),
            crate::crate_artifact::ArtifactExport {
                source: "Box as Show".to_string(),
                id: impl_def_id,
            },
        )]);

        assert!(products.identity_table.prelude_export_names.is_empty());
        assert!(!products
            .identity_table
            .prelude_export_names
            .contains_key("impl_alias"));
    }

    #[test]
    fn product_emission_reads_id_owned_hir_definitions() {
        let function_id = DefId::new(CrateId(0), LocalDefId(0));
        let struct_id = DefId::new(CrateId(0), LocalDefId(1));
        let function = test_function(function_id, "answer", Vec::new());
        let structure = HirStruct {
            id: struct_id,
            name: "Box".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        };

        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::from([(function_id, function)]),
            HashMap::from([(struct_id, structure)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("answer".to_string(), function_id)]),
                structs_by_name: HashMap::from([("Box".to_string(), struct_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([
                (function_id, "answer".to_string()),
                (struct_id, "Box".to_string()),
            ]),
        );
        let hir = ResolvedHirProgram::new(
            program,
            ResolverTables::default(),
            BTreeSet::from([function_id, struct_id]),
            CrateId(0),
            IdGen::new(),
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("test".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        assert!(products
            .interface
            .functions
            .contains_key(&ProductDefId::from(function_id)));
        assert!(products
            .interface
            .structs
            .contains_key(&ProductDefId::from(struct_id)));
    }

    #[test]
    fn compiler_products_emit_named_interface_when_derived_indexes_are_stale() {
        let mut hir = resolved_hir_for_products();
        let plain_id = DefId::new(CrateId(0), LocalDefId(0));
        let struct_id = DefId::new(CrateId(0), LocalDefId(2));
        let enum_id = DefId::new(CrateId(0), LocalDefId(3));
        let trait_id = DefId::new(CrateId(0), LocalDefId(4));
        mutate_program(&mut hir.program, |program| {
            program.indexes.functions_by_id.remove(&plain_id);
            program.indexes.structs_by_id.remove(&struct_id);
            program.indexes.enums_by_id.remove(&enum_id);
            program.indexes.traits_by_id.remove(&trait_id);
        });

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let plain_id = ProductDefId::from(plain_id);
        let struct_id = ProductDefId::from(struct_id);
        let enum_id = ProductDefId::from(enum_id);
        let trait_id = ProductDefId::from(trait_id);
        assert_eq!(products.interface.functions[&plain_id].name, "plain");
        assert_eq!(products.interface.structs[&struct_id].name, "Box");
        assert_eq!(products.interface.enums[&enum_id].name, "Maybe");
        assert_eq!(products.interface.traits[&trait_id].name, "demo::Show");
        assert_eq!(products.identity_table.export_names["plain"], plain_id);
        assert_eq!(products.identity_table.export_names["Box"], struct_id);
        assert_eq!(products.identity_table.export_names["Maybe"], enum_id);
        assert_eq!(products.identity_table.export_names["Show"], trait_id);
    }

    #[test]
    #[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
    fn compiler_products_reject_function_generic_param_decls_with_invalid_current_crate_ids() {
        let original_id = DefId::new(CrateId(u32::MAX), LocalDefId(17));
        let original_generic = crate::types::GenericParamId {
            owner: original_id,
            index: 0,
        };
        let mut function =
            test_function(original_id, "identity", generic_params(original_id, &["T"]));
        function.generic_params[0].id = original_generic;
        function.params[0].ty = Type::Generic(original_generic);
        function.ret_type = Type::Generic(original_generic);
        function.body = empty_body(Type::Generic(original_generic));

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &ResolvedHirProgram::new(
                HirProgram::from_id_parts_with_names_and_canonical_names(
                    HashMap::from([(original_id, function)]),
                    HashMap::new(),
                    HashMap::new(),
                    HashMap::new(),
                    HashMap::new(),
                    HashMap::new(),
                    HirNameTables {
                        functions_by_name: HashMap::from([("identity".to_string(), original_id)]),
                        ..HirNameTables::default()
                    },
                    HirLanguageItems::default(),
                    &HashMap::from([(original_id, "identity".to_string())]),
                ),
                ResolverTables::default(),
                BTreeSet::from([original_id]),
                CrateId(0),
                IdGen::new(),
            ),
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let (product_id, function) = products
            .interface
            .functions
            .iter()
            .next()
            .expect("product should contain remapped generic function metadata");
        let expected_owner = DefId::new(
            CrateId(product_id.crate_id.0),
            LocalDefId(product_id.local_id.0),
        );

        assert_ne!(*product_id, ProductDefId::from(original_id));
        assert_eq!(function.generic_params[0].id.owner, expected_owner);
        assert_eq!(
            function.params[0],
            Type::Generic(crate::types::GenericParamId {
                owner: expected_owner,
                index: 0,
            })
        );
    }

    #[test]
    #[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
    fn compiler_products_reject_function_body_generic_owners_with_invalid_current_crate_ids() {
        let function_id = DefId::new(CrateId(0), LocalDefId(17));
        let invalid_owner = DefId::new(CrateId(u32::MAX), LocalDefId(18));
        let invalid_generic = crate::types::GenericParamId {
            owner: invalid_owner,
            index: 0,
        };
        let mut function =
            test_function(function_id, "identity", generic_params(function_id, &["T"]));
        function.generic_params[0].id = invalid_generic;
        function.params[0].ty = Type::Generic(invalid_generic);
        function.ret_type = Type::Generic(invalid_generic);
        function.body = empty_body(Type::Generic(invalid_generic));

        let _ = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &ResolvedHirProgram::new(
                HirProgram::from_id_parts_with_names_and_canonical_names(
                    HashMap::from([(function_id, function)]),
                    HashMap::new(),
                    HashMap::new(),
                    HashMap::new(),
                    HashMap::new(),
                    HashMap::new(),
                    HirNameTables {
                        functions_by_name: HashMap::from([("identity".to_string(), function_id)]),
                        ..HirNameTables::default()
                    },
                    HirLanguageItems::default(),
                    &HashMap::from([(function_id, "identity".to_string())]),
                ),
                ResolverTables::default(),
                BTreeSet::from([function_id]),
                CrateId(0),
                IdGen::new(),
            ),
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
    }

    #[test]
    #[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
    fn compiler_products_reject_trait_default_body_generic_owners_with_invalid_current_crate_ids() {
        let trait_id = DefId::new(CrateId(u32::MAX), LocalDefId(17));
        let method_id = DefId::new(CrateId(u32::MAX), LocalDefId(18));
        let self_generic = crate::types::GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let method = HirFunction {
            id: method_id,
            name: "identity".to_string(),
            generic_params: vec![],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(self_generic),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Generic(self_generic),
            body: HirBlock {
                stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
                    kind: crate::hir::HirExprKindFor::Var("self".to_string()),
                    ty: Type::Generic(self_generic),
                    span: Span::test(),
                })],
                ty: Type::Generic(self_generic),
            },
            is_curried: false,
            is_method: true,
            self_receiver: None,
            is_unsafe: false,
        };
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                trait_id,
                HirTrait {
                    target: None,
                    predicates: Vec::new(),
                    id: trait_id,
                    name: "Identity".to_string(),
                    generic_params: vec![],
                    associated_types: vec![],
                    methods: HashMap::from([("identity".to_string(), method)]),
                    signatures: HashMap::new(),
                },
            )]),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                traits_by_name: HashMap::from([("Identity".to_string(), trait_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([(trait_id, "Identity".to_string())]),
        );
        let hir = ResolvedHirProgram::new(
            program,
            ResolverTables::default(),
            BTreeSet::from([trait_id, method_id]),
            CrateId(0),
            IdGen::new(),
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let trait_product_id = products
            .interface
            .traits
            .keys()
            .next()
            .copied()
            .expect("product should contain trait interface");
        let expected_owner = DefId::new(
            CrateId(trait_product_id.crate_id.0),
            LocalDefId(trait_product_id.local_id.0),
        );
        let method = products
            .bodies
            .trait_default_methods
            .values()
            .next()
            .expect("product should contain trait default body");
        let crate::hir::HirStmtFor::Expr(expr) = &method.body.stmts[0] else {
            panic!("expected expression statement in default method body");
        };

        assert_eq!(
            expr.ty,
            Type::Generic(crate::types::GenericParamId {
                owner: expected_owner,
                index: 0,
            })
        );
    }

    #[test]
    #[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
    fn compiler_products_reject_generic_impl_method_body_generic_owners_with_invalid_current_crate_ids(
    ) {
        let impl_id = DefId::new(CrateId(u32::MAX), LocalDefId(19));
        let method_id = DefId::new(CrateId(u32::MAX), LocalDefId(20));
        let impl_generic = crate::types::GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let method_generic = crate::types::GenericParamId {
            owner: method_id,
            index: 0,
        };
        let method = HirFunction {
            id: method_id,
            name: "get".to_string(),
            generic_params: vec![GenericParamDecl::type_param(method_generic, "U")],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(impl_generic),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Generic(method_generic),
            body: HirBlock {
                stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
                    kind: crate::hir::HirExprKindFor::Var("self".to_string()),
                    ty: Type::Generic(impl_generic),
                    span: Span::test(),
                })],
                ty: Type::Generic(impl_generic),
            },
            is_curried: false,
            is_method: true,
            self_receiver: None,
            is_unsafe: false,
        };
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                impl_id,
                HirImpl {
                    id: impl_id,
                    owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: generic_params(impl_id, &["T"]),
                    receiver_pattern: vec![Type::Generic(impl_generic)].into(),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: vec![],
                    trait_arg_types: vec![],
                    associated_types: vec![],
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::from([("get".to_string(), method)]),
                },
            )]),
            HashMap::new(),
            HirNameTables::default(),
            HirLanguageItems::default(),
            &HashMap::new(),
        );
        let hir = ResolvedHirProgram::new(
            program,
            ResolverTables::default(),
            BTreeSet::from([impl_id, method_id]),
            CrateId(0),
            IdGen::new(),
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let impl_product_id = products
            .interface
            .impls
            .keys()
            .next()
            .copied()
            .expect("product should contain impl interface");
        let expected_owner = DefId::new(
            CrateId(impl_product_id.crate_id.0),
            LocalDefId(impl_product_id.local_id.0),
        );
        let imp = products
            .bodies
            .generic_impls
            .values()
            .next()
            .expect("product should contain generic impl body");
        let method = &imp.methods["get"];
        assert_ne!(method.id, method_id);
        assert_ne!(method.id.crate_id, CrateId(u32::MAX));
        assert_eq!(
            method.generic_params[0].id.owner, method.id,
            "method-owned generic params should be rehomed to the product method ID"
        );
        assert_eq!(
            method.ret_type,
            Type::Generic(crate::types::GenericParamId {
                owner: method.id,
                index: 0,
            })
        );
        let crate::hir::HirStmtFor::Expr(expr) = &method.body.stmts[0] else {
            panic!("expected expression statement in impl method body");
        };

        assert_eq!(
            expr.ty,
            Type::Generic(crate::types::GenericParamId {
                owner: expected_owner,
                index: 0,
            })
        );
    }

    #[test]
    #[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
    fn compiler_products_reject_body_field_and_variant_location_owners_with_invalid_current_crate_ids(
    ) {
        let struct_id = DefId::new(CrateId(u32::MAX), LocalDefId(21));
        let enum_id = DefId::new(CrateId(u32::MAX), LocalDefId(22));
        let impl_id = DefId::new(CrateId(u32::MAX), LocalDefId(23));
        let method_id = DefId::new(CrateId(u32::MAX), LocalDefId(24));
        let method = HirFunction {
            id: method_id,
            name: "locations".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                crate::types::GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                "T",
            )],
            generic_bounds: HashMap::new().into(),
            params: vec![],
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: vec![
                    crate::hir::HirStmtFor::Expr(HirExpr {
                        kind: crate::hir::HirExprKindFor::FieldAccess(
                            Box::new(HirExpr {
                                kind: crate::hir::HirExprKindFor::Var("boxed".to_string()),
                                ty: Type::Struct {
                                    id: struct_id,
                                    args: vec![],
                                },
                                span: Span::test(),
                            }),
                            "value".to_string(),
                            Some(crate::hir::HirFieldLocation {
                                owner: struct_id,
                                field_id: FieldId(0),
                                name: "value".to_string(),
                            }),
                        ),
                        ty: Type::I64,
                        span: Span::test(),
                    }),
                    crate::hir::HirStmtFor::Expr(HirExpr {
                        kind: crate::hir::HirExprKindFor::EnumVariant(
                            "Maybe".to_string(),
                            "Some".to_string(),
                            vec![],
                            Some(crate::hir::HirVariantLocation {
                                owner: enum_id,
                                variant_id: VariantId(0),
                                name: "Some".to_string(),
                            }),
                        ),
                        ty: Type::Enum {
                            id: enum_id,
                            args: vec![],
                        },
                        span: Span::test(),
                    }),
                ],
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::from([(
                struct_id,
                HirStruct {
                    id: struct_id,
                    name: "Box".to_string(),
                    generic_params: vec![],
                    fields: Vec::new(),
                },
            )]),
            HashMap::from([(
                enum_id,
                HirEnum {
                    id: enum_id,
                    name: "Maybe".to_string(),
                    generic_params: vec![],
                    variants: Vec::new(),
                },
            )]),
            HashMap::new(),
            HashMap::from([(
                impl_id,
                HirImpl {
                    id: impl_id,
                    owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: generic_params(impl_id, &["T"]),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: struct_id,
                        args: vec![Type::Generic(crate::types::GenericParamId {
                            owner: impl_id,
                            index: 0,
                        })],
                    }),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: vec![],
                    trait_arg_types: vec![],
                    associated_types: vec![],
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::from([("locations".to_string(), method)]),
                },
            )]),
            HashMap::new(),
            HirNameTables {
                structs_by_name: HashMap::from([("Box".to_string(), struct_id)]),
                enums_by_name: HashMap::from([("Maybe".to_string(), enum_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([
                (struct_id, "Box".to_string()),
                (enum_id, "Maybe".to_string()),
            ]),
        );
        let hir = ResolvedHirProgram::new(
            program,
            ResolverTables::default(),
            BTreeSet::from([struct_id, enum_id, impl_id, method_id]),
            CrateId(0),
            IdGen::new(),
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let expected_struct_owner = products
            .interface
            .structs
            .keys()
            .next()
            .copied()
            .map(|id| DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0)))
            .expect("product should contain struct interface");
        let expected_enum_owner = products
            .interface
            .enums
            .keys()
            .next()
            .copied()
            .map(|id| DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0)))
            .expect("product should contain enum interface");
        let imp = products
            .bodies
            .generic_impls
            .values()
            .next()
            .expect("product should contain generic impl body");
        let method = &imp.methods["locations"];
        let crate::hir::HirStmtFor::Expr(field_expr) = &method.body.stmts[0] else {
            panic!("expected field access expression statement");
        };
        let crate::hir::HirExprKindFor::FieldAccess(_, _, Some(field_location)) = &field_expr.kind
        else {
            panic!("expected field access location");
        };
        let crate::hir::HirStmtFor::Expr(variant_expr) = &method.body.stmts[1] else {
            panic!("expected enum variant expression statement");
        };
        let crate::hir::HirExprKindFor::EnumVariant(_, _, _, Some(variant_location)) =
            &variant_expr.kind
        else {
            panic!("expected enum variant location");
        };

        assert_eq!(field_location.owner, expected_struct_owner);
        assert_eq!(variant_location.owner, expected_enum_owner);
    }

    #[test]
    #[should_panic(expected = "compiler product emission received invalid current-crate DefId")]
    fn compiler_products_reject_call_targets_with_invalid_current_crate_ids() {
        let function_id = DefId::new(CrateId(0), LocalDefId(0));
        let invalid_target = DefId::new(CrateId(u32::MAX), LocalDefId(99));
        let mut function = test_function(function_id, "caller", Vec::new());
        function.body = HirBlock {
            stmts: vec![crate::hir::HirStmtFor::Expr(HirExpr {
                kind: crate::hir::HirExprKindFor::Call(
                    Box::new(HirExpr {
                        kind: crate::hir::HirExprKindFor::Var("target".to_string()),
                        ty: Type::function(Vec::new(), Type::I64),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    Some(HirCallTarget::Function(invalid_target)),
                ),
                ty: Type::I64,
                span: Span::test(),
            })],
            ty: Type::I64,
        };
        let hir = ResolvedHirProgram::new(
            HirProgram::from_id_parts_with_names_and_canonical_names(
                HashMap::from([(function_id, function)]),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HirNameTables {
                    functions_by_name: HashMap::from([("caller".to_string(), function_id)]),
                    ..HirNameTables::default()
                },
                HirLanguageItems::default(),
                &HashMap::from([(function_id, "caller".to_string())]),
            ),
            ResolverTables::default(),
            BTreeSet::from([function_id]),
            CrateId(0),
            local_def_ids_after(1),
        );

        CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
    }

    #[test]
    fn compiler_products_emit_alias_duplicate_function_once_by_def_id() {
        let mut hir = resolved_hir_for_products();
        let generic_id = DefId::new(CrateId(0), LocalDefId(1));
        rebuild_program(&mut hir.program, |_, _, _, _, _, _, names, _| {
            names
                .functions_by_name
                .insert("alias_identity".to_string(), generic_id);
        });
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let generic_product_id = ProductDefId::from(generic_id);
        assert_eq!(
            products
                .interface
                .functions
                .values()
                .filter(|function| function.id == generic_id)
                .count(),
            1
        );
        assert_eq!(
            products.identity_table.export_names.get("identity"),
            Some(&generic_product_id)
        );
        assert!(!products
            .identity_table
            .export_names
            .contains_key("alias_identity"));
    }

    #[test]
    fn compiler_products_select_generic_bodies_by_product_def_id() {
        let hir = resolved_hir_for_products();

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));
        let generic_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(1)));
        let impl_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(5)));
        let default_method_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(7)));

        assert!(!products.bodies.functions.contains_key(&plain_id));
        assert!(products.bodies.functions.contains_key(&generic_id));
        assert!(products.bodies.generic_impls.contains_key(&impl_id));
        assert!(products
            .bodies
            .trait_default_methods
            .contains_key(&default_method_id));
    }

    #[test]
    fn compiler_products_preserve_non_generic_impl_with_generic_method_body() {
        let hir = resolved_hir_with_non_generic_impl_generic_method();
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let impl_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(42)));
        assert!(products.bodies.generic_impls.contains_key(&impl_id));
    }

    #[test]
    fn compiler_products_emit_only_current_crate_hir() {
        let mut hir = resolved_hir_for_products();
        let foreign_function_id = DefId::new(CrateId(0), LocalDefId(50));
        let foreign_impl_id = DefId::new(CrateId(0), LocalDefId(51));
        hir.current_def_ids = (0..8)
            .map(|local| DefId::new(CrateId(0), LocalDefId(local)))
            .collect();
        rebuild_program(&mut hir.program, |functions, _, _, _, impls, _, _, _| {
            functions.insert(
                foreign_function_id,
                accept_function(test_function(
                    foreign_function_id,
                    "stdlib::identity",
                    generic_params(foreign_function_id, &["T"]),
                )),
            );
            impls.insert(
                foreign_impl_id,
                HirImplFor::<AcceptedHir> {
                    id: foreign_impl_id,
                    owner: crate::hir::HirImplOwner::Named("stdlib::Box".to_string()),
                    type_name: "stdlib::Box".to_string(),
                    type_generics: generic_params(foreign_impl_id, &["T"]),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: DefId::new(CrateId(0), LocalDefId(2)),
                        args: vec![Type::Generic(crate::types::GenericParamId {
                            owner: foreign_impl_id,
                            index: 0,
                        })],
                    }),
                    trait_name: Some("stdlib::Show".to_string()),
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::new(),
                },
            );
        });

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            vec![ProductDependencyIdentity::artifact_object(
                "stdlib".to_string(),
                PathBuf::from("build/stdlib.rkca"),
            )],
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let foreign_function_id = ProductDefId::from(foreign_function_id);
        let foreign_impl_id = ProductDefId::from(foreign_impl_id);

        assert!(!products
            .interface
            .functions
            .contains_key(&foreign_function_id));
        assert!(!products.bodies.functions.contains_key(&foreign_function_id));
        assert!(!products.interface.impls.contains_key(&foreign_impl_id));
        assert!(!products.bodies.generic_impls.contains_key(&foreign_impl_id));
        assert_eq!(products.interface.impls.len(), 1);
        assert_eq!(products.bodies.generic_impls.len(), 1);
    }

    #[test]
    fn compiler_products_do_not_treat_empty_current_ids_as_root_crate_ownership() {
        let mut hir = resolved_hir_for_products();
        let foreign_function_id = DefId::new(CrateId(0), LocalDefId(50));
        hir.current_def_ids = BTreeSet::new();
        rebuild_program(
            &mut hir.program,
            |functions, structs, enums, traits, impls, externs, _, effective_methods| {
                functions.clear();
                structs.clear();
                enums.clear();
                traits.clear();
                impls.clear();
                externs.clear();
                effective_methods.clear();
                functions.insert(
                    foreign_function_id,
                    accept_function(test_function(
                        foreign_function_id,
                        "stdlib::identity",
                        generic_params(foreign_function_id, &["T"]),
                    )),
                );
            },
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            vec![ProductDependencyIdentity::artifact_object(
                "stdlib".to_string(),
                PathBuf::from("build/stdlib.rkca"),
            )],
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        assert!(products.interface.functions.is_empty());
        assert!(products.bodies.functions.is_empty());
    }

    #[test]
    fn compiler_products_keep_empty_trait_default_bodies_by_product_def_id() {
        let mut hir = resolved_hir_for_products();
        let empty_method_id = DefId::new(CrateId(0), LocalDefId(10));
        hir.current_def_ids.insert(empty_method_id);
        let empty_method = HirFunction {
            body: empty_body(Type::Unit),
            ret_type: Type::Unit,
            ..test_function(empty_method_id, "noop", Vec::new())
        };
        let effective_method_id = DefId::new(CrateId(0), LocalDefId(11));
        hir.current_def_ids.insert(effective_method_id);
        rebuild_program(
            &mut hir.program,
            |_, _, _, traits, impls, _, _, effective_methods| {
                traits
                    .get_mut(&DefId::new(CrateId(0), LocalDefId(4)))
                    .unwrap()
                    .methods
                    .insert("noop".to_string(), accept_function(empty_method.clone()));
                let mut effective_method = empty_method;
                effective_method.id = effective_method_id;
                impls
                    .get_mut(&DefId::new(CrateId(0), LocalDefId(5)))
                    .unwrap()
                    .methods
                    .insert("noop".to_string(), accept_function(effective_method));
                effective_methods.insert(
                    (DefId::new(CrateId(0), LocalDefId(5)), empty_method_id),
                    effective_method_id,
                );
            },
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let empty_method_id = ProductDefId::from(empty_method_id);

        assert!(products
            .bodies
            .trait_default_methods
            .contains_key(&empty_method_id));
    }

    #[test]
    fn compiler_products_record_dependency_identities_by_product_crate_id() {
        let hir = resolved_hir_for_products();
        let stdlib = ProductDependencyIdentity::artifact_object(
            "stdlib".to_string(),
            PathBuf::from("build/stdlib.rkca"),
        );
        let math = ProductDependencyIdentity::artifact_object(
            "math".to_string(),
            PathBuf::from("build/math.rkca"),
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            vec![stdlib, math],
            BTreeMap::from([
                (
                    ProductCrateId(7),
                    ProductCrateIdentity::local("stdlib".to_string()),
                ),
                (
                    ProductCrateId(42),
                    ProductCrateIdentity::local("math".to_string()),
                ),
            ]),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        assert_eq!(products.dependencies.len(), 2);
        assert_eq!(products.identity_table.local_crate, Some(ProductCrateId(0)));
        assert_eq!(
            products
                .identity_table
                .dependencies
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![ProductCrateId(7), ProductCrateId(42)]
        );
        assert_eq!(
            products.identity_table.dependencies[&ProductCrateId(7)].name,
            "stdlib"
        );
        assert_eq!(
            products.identity_table.dependencies[&ProductCrateId(42)].name,
            "math"
        );
    }

    #[test]
    fn compiler_products_freshness_metadata_includes_schema_source_and_dependency_identity() {
        let hir = resolved_hir_for_products();
        let source_fingerprint = ProductSourceFingerprint {
            manifest_hash: Some("manifest".to_string()),
            source_hash: Some("source".to_string()),
            loaded_files: vec![PathBuf::from("src/main.rk")],
        };
        let mut crate_identity = ProductCrateIdentity::local("demo".to_string());
        crate_identity.target_triple = Some("test-target".to_string());

        let products = CompilerProducts::from_resolved_hir(
            crate_identity,
            &hir,
            vec![ProductDependencyIdentity::artifact_object(
                "stdlib".to_string(),
                PathBuf::from("build/stdlib.rkca"),
            )],
            BTreeMap::from([(
                ProductCrateId(7),
                ProductCrateIdentity::local("stdlib".to_string()),
            )]),
            source_fingerprint.clone(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let freshness = products.freshness_metadata();

        assert_eq!(
            freshness.artifact_format_version,
            PRODUCT_ARTIFACT_FORMAT_VERSION
        );
        assert_eq!(freshness.target_triple.as_deref(), Some("test-target"));
        assert_eq!(freshness.source_fingerprint, source_fingerprint);
        assert_eq!(freshness.dependencies.len(), 1);
        assert_eq!(freshness.dependencies[0].name, "stdlib");
        assert_eq!(
            freshness.dependencies[0]
                .crate_identity
                .as_ref()
                .map(|identity| identity.name.as_str()),
            Some("stdlib")
        );
        assert!(!freshness.compiler_version.is_empty());
    }

    #[test]
    fn compiler_products_reject_freshness_metadata_mismatch_on_load() {
        let hir = resolved_hir_for_products();
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let mut artifact =
            serialized_artifact_from_bytes_for_test(&products.to_artifact_bytes().unwrap())
                .unwrap();
        artifact.products.freshness.source_fingerprint.source_hash = Some("tampered".to_string());
        let bytes =
            encode_product_artifact(&ProductArtifactHeader::from_products(&products), &artifact)
                .unwrap();

        let err = CompilerProducts::from_artifact_bytes(&bytes).unwrap_err();

        assert!(
            err.contains("freshness metadata mismatch"),
            "expected freshness mismatch rejection, got {err}"
        );
    }

    #[test]
    fn compiler_products_roundtrip_preserves_dependency_capabilities() {
        let hir = resolved_hir_for_products();
        let capabilities = ProductDependencyCapabilities {
            metadata: true,
            bodies: true,
            link: ProductDependencyLinkCapability::Object,
        };
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            vec![ProductDependencyIdentity {
                name: "dep".to_string(),
                artifact_path: PathBuf::from("build/dep.rkca"),
                capabilities: capabilities.clone(),
            }],
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let bytes = products.to_artifact_bytes().unwrap();
        let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();

        assert_eq!(roundtrip.dependencies[0].capabilities, capabilities);
    }

    #[test]
    fn compiler_products_roundtrip_preserves_product_def_ids() {
        let hir = resolved_hir_for_products();
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            vec![ProductDependencyIdentity::artifact_object(
                "dep".to_string(),
                std::path::PathBuf::from("build/dep.rkca"),
            )],
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let bytes = bincode::serialize(&products).unwrap();
        let decoded: CompilerProducts = bincode::deserialize(&bytes).unwrap();

        let generic_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(1)));
        assert!(decoded.interface.functions.contains_key(&generic_id));
        assert!(decoded.bodies.functions.contains_key(&generic_id));
        assert_eq!(
            decoded.identity_table.export_names.get("identity"),
            Some(&generic_id)
        );
        assert_eq!(decoded.dependencies[0].name, "dep");
        assert_eq!(
            decoded.dependencies[0].artifact_path,
            std::path::PathBuf::from("build/dep.rkca")
        );
    }

    #[test]
    fn compiler_products_write_and_read_product_artifact_bytes() {
        let hir = resolved_hir_for_products();
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            vec![ProductDependencyIdentity::artifact_object(
                "dep".to_string(),
                PathBuf::from("build/dep.rkca"),
            )],
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let bytes = products.to_artifact_bytes().unwrap();
        let decoded = CompilerProducts::from_artifact_bytes(&bytes).unwrap();
        let identity_id = decoded.identity_table.export_names["identity"];

        assert!(decoded.interface.functions.contains_key(&identity_id));
        assert!(decoded.bodies.functions.contains_key(&identity_id));
        assert_eq!(decoded.dependencies[0].name, "dep");
        assert_eq!(
            decoded.dependencies[0].artifact_path,
            PathBuf::from("build/dep.rkca")
        );
    }

    #[test]
    fn compiler_products_v25_artifact_contains_type_table() {
        let hir = resolved_hir_for_products();
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let bytes = products.to_artifact_bytes().unwrap();

        assert!(super::type_table::artifact_type_table_len_for_test(&bytes).unwrap() > 0);
    }

    #[test]
    fn product_artifact_schema_does_not_serialize_compiler_products_directly() {
        let hir = resolved_hir_for_products();
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let bytes = products.to_artifact_bytes().unwrap();

        let direct_products_decode = bincode::deserialize::<CompilerProducts>(&bytes);

        assert!(
            direct_products_decode.is_err(),
            "v25 artifacts must not be raw CompilerProducts payloads"
        );
        assert!(super::type_table::artifact_type_table_len_for_test(&bytes).unwrap() > 0);
    }

    #[test]
    fn serialized_product_artifact_roundtrips_function_signature_types() {
        let resolved = resolved_hir_for_products();
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &resolved,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let roundtrip =
            CompilerProducts::from_artifact_bytes(&products.to_artifact_bytes().unwrap()).unwrap();
        let function = roundtrip
            .interface
            .functions
            .values()
            .find(|function| function.name == "identity")
            .unwrap();

        assert_eq!(function.params[0], Type::I64);
        assert_eq!(function.ret_type, Type::I64);
    }

    #[test]
    fn serialized_product_artifact_roundtrips_type_alias_identity_and_body() {
        let mut resolved = resolved_hir_for_products();
        let alias_id = DefId::new(resolved.root_crate_id, LocalDefId(200));
        let alias = crate::hir::HirTypeAlias {
            id: alias_id,
            name: "Identity".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                crate::types::GenericParamId {
                    owner: alias_id,
                    index: 0,
                },
                "T",
            )],
            ty: Type::Lambda {
                params: vec![crate::type_services::kind::Kind::Type],
                body: Box::new(Type::BoundVar {
                    depth: 0,
                    index: 0,
                    kind: crate::type_services::kind::Kind::Type,
                }),
            },
        };
        mutate_program(&mut resolved.program, |program| {
            program.type_aliases.insert(alias_id, alias.clone());
            program.rebuild_indexes();
        });
        resolved.current_def_ids.insert(alias_id);

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &resolved,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .unwrap();
        let roundtrip =
            CompilerProducts::from_artifact_bytes(&products.to_artifact_bytes().unwrap()).unwrap();
        let product_id = ProductDefId::from(alias_id);
        let roundtrip_alias = roundtrip.interface.type_aliases.get(&product_id).unwrap();

        assert_eq!(roundtrip_alias.id, alias_id);
        assert_eq!(roundtrip_alias.name, "Identity");
        assert_eq!(roundtrip_alias.generic_params, alias.generic_params);
        assert_eq!(roundtrip_alias.ty, alias.ty);
    }

    #[test]
    fn serialized_product_artifact_roundtrips_struct_enum_trait_impl_and_extern_types() {
        let mut resolved = resolved_hir_for_products();
        let field_ty = Type::Pointer(Box::new(Type::U32));
        let variant_ty = Type::Array(Box::new(Type::Bool), 3);
        mutate_program(&mut resolved.program, |program| {
            program
                .structs
                .values_mut()
                .find(|strukt| strukt.name == "Box")
                .unwrap()
                .fields
                .push(crate::hir::HirField {
                    id: FieldId(99),
                    name: "payload".to_string(),
                    ty: field_ty.clone(),
                    public: true,
                });
            program
                .enums
                .values_mut()
                .find(|enm| enm.name == "Maybe")
                .unwrap()
                .variants
                .push(crate::hir::HirVariant {
                    id: VariantId(99),
                    name: "Some".to_string(),
                    fields: crate::hir::HirVariantFields::Positional(vec![variant_ty.clone()]),
                });
        });
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &resolved,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let bytes = products.to_artifact_bytes().unwrap();
        let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();

        let strukt = roundtrip
            .interface
            .structs
            .values()
            .find(|strukt| strukt.name == "Box")
            .unwrap();
        assert_eq!(strukt.fields[0].ty, field_ty);
        let enm = roundtrip
            .interface
            .enums
            .values()
            .find(|enm| enm.name == "Maybe")
            .unwrap();
        match &enm.variants[0].fields {
            crate::hir::HirVariantFields::Positional(types) => {
                assert_eq!(types.as_slice(), &[variant_ty.clone()]);
            }
            other => panic!("expected positional variant fields, got {other:?}"),
        }
        assert!(
            super::type_table::artifact_type_table_contains_type_for_test(&bytes, &field_ty)
                .unwrap()
        );
        assert!(
            super::type_table::artifact_type_table_contains_type_for_test(&bytes, &variant_ty)
                .unwrap()
        );
        assert!(roundtrip
            .interface
            .traits
            .values()
            .any(|trait_def| trait_def.name == "demo::Show"));
        assert!(roundtrip.interface.impls.values().any(|imp| !matches!(
            imp.receiver_pattern,
            crate::hir::HirImplReceiverPattern::Exact(Type::Unit)
        )));
        assert!(roundtrip
            .interface
            .externs
            .values()
            .any(|ext| ext.ret == crate::types::Type::I32));
    }

    #[test]
    fn serialized_product_artifact_roundtrips_constructor_impl_target() {
        let mut resolved = resolved_hir_for_products();
        let mut expected = None;
        mutate_program(&mut resolved.program, |program| {
            let box_id = program
                .structs
                .values()
                .find(|structure| structure.name == "Box")
                .unwrap()
                .id;
            let impl_generic = program.impls.values().next().unwrap().type_generics[0].id;
            let target = Type::Lambda {
                params: vec![crate::type_services::kind::Kind::Type],
                body: Box::new(Type::Struct {
                    id: box_id,
                    args: vec![Type::Tuple(vec![
                        Type::Generic(impl_generic),
                        Type::BoundVar {
                            depth: 0,
                            index: 0,
                            kind: crate::type_services::kind::Kind::Type,
                        },
                    ])],
                }),
            };
            program.impls.values_mut().next().unwrap().receiver_pattern =
                crate::hir::HirImplReceiverPattern::Constructor(target.clone());
            expected = Some(target);
        });
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &resolved,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let roundtrip =
            CompilerProducts::from_artifact_bytes(&products.to_artifact_bytes().unwrap()).unwrap();

        assert!(roundtrip.interface.impls.values().any(|imp| {
            imp.receiver_pattern
                == crate::hir::HirImplReceiverPattern::Constructor(expected.clone().unwrap())
        }));
    }

    #[test]
    fn compiler_products_type_table_roundtrips_nested_body_type_locations() {
        let mut resolved = resolved_hir_for_products();
        let block_ty = Type::Array(Box::new(Type::U8), 4);
        let let_ty = Type::Tuple(vec![Type::U16, Type::Bool]);
        let nested_expr_ty = Type::Pointer(Box::new(Type::Bool));
        let cast_ty = Type::Array(Box::new(Type::U32), 2);
        let lambda_param_ty = Type::Pointer(Box::new(Type::F32));
        let lambda_capture_ty = Type::Reference {
            mutable: true,
            inner: Box::new(Type::U64),
        };
        let lambda_body_ty = Type::Slice(Box::new(Type::Char));
        let lambda_expr_ty = Type::function(vec![lambda_param_ty.clone()], lambda_body_ty.clone());
        mutate_program(&mut resolved.program, |program| {
            let identity = program
                .functions
                .values_mut()
                .find(|function| function.name == "identity")
                .unwrap();
            identity.body = crate::hir::HirBlockFor::<AcceptedHir> {
                stmts: vec![
                    crate::hir::HirStmtFor::Let {
                        name: "converted".to_string(),
                        local_id: crate::ids::HirLocalId(11),
                        ty: let_ty.clone(),
                        value: crate::hir::HirExprFor::<AcceptedHir> {
                            kind: crate::hir::HirExprKindFor::Cast(
                                Box::new(crate::hir::HirExprFor::<AcceptedHir> {
                                    kind: crate::hir::HirExprKindFor::IntLiteral(1),
                                    ty: nested_expr_ty.clone(),
                                    span: Span::test(),
                                }),
                                cast_ty.clone(),
                            ),
                            ty: cast_ty.clone(),
                            span: Span::test(),
                        },
                        mutable: false,
                    },
                    crate::hir::HirStmtFor::Expr(crate::hir::HirExprFor::<AcceptedHir> {
                        kind: crate::hir::HirExprKindFor::Lambda {
                            params: vec![HirParam {
                                name: "arg".to_string(),
                                local_id: crate::ids::HirLocalId(12),
                                ty: lambda_param_ty.clone(),
                                mutable: false,
                                is_ref: false,
                            }],
                            body: crate::hir::HirBlockFor::<AcceptedHir> {
                                stmts: vec![crate::hir::HirStmtFor::Expr(
                                    crate::hir::HirExprFor::<AcceptedHir> {
                                        kind: crate::hir::HirExprKindFor::Unit,
                                        ty: lambda_body_ty.clone(),
                                        span: Span::test(),
                                    },
                                )],
                                ty: lambda_body_ty.clone(),
                            },
                            captures: vec![HirClosureCapture {
                                name: "captured".to_string(),
                                local_id: crate::ids::HirLocalId(13),
                                kind: HirClosureCaptureKind::MutableBorrow,
                                mutable: true,
                                ty: lambda_capture_ty.clone(),
                            }],
                        },
                        ty: lambda_expr_ty.clone(),
                        span: Span::test(),
                    }),
                ],
                ty: block_ty.clone(),
            };
        });
        refresh_resolved_type_ids(&mut resolved);
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &resolved,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let bytes = products.to_artifact_bytes().unwrap();
        let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();
        let function = roundtrip
            .bodies
            .functions
            .values()
            .find(|function| function.name == "identity")
            .unwrap();

        assert_eq!(function.body.ty, block_ty);
        let crate::hir::HirStmtFor::Let { ty, value, .. } = &function.body.stmts[0] else {
            panic!("expected let statement");
        };
        assert_eq!(ty, &let_ty);
        assert_eq!(value.ty, cast_ty);
        let crate::hir::HirExprKindFor::Cast(inner, target_ty) = &value.kind else {
            panic!("expected cast expression");
        };
        assert_eq!(inner.ty, nested_expr_ty);
        assert_eq!(target_ty, &cast_ty);
        let crate::hir::HirStmtFor::Expr(lambda) = &function.body.stmts[1] else {
            panic!("expected lambda expression statement");
        };
        assert_eq!(lambda.ty, lambda_expr_ty);
        let crate::hir::HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } = &lambda.kind
        else {
            panic!("expected lambda expression");
        };
        assert_eq!(params[0].ty, lambda_param_ty);
        assert_eq!(captures[0].ty, lambda_capture_ty);
        assert_eq!(body.ty, lambda_body_ty);

        for expected in [
            &let_ty,
            &cast_ty,
            &lambda_param_ty,
            &lambda_capture_ty,
            &lambda_body_ty,
        ] {
            assert!(
                super::type_table::artifact_type_table_contains_type_for_test(&bytes, expected)
                    .unwrap(),
                "artifact type table should contain body-only type {expected:?}"
            );
        }
    }

    #[test]
    fn compiler_products_type_table_roundtrips_struct_pattern_type_args() {
        let mut resolved = resolved_hir_for_products();
        let pattern_arg_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::U16),
        };
        mutate_program(&mut resolved.program, |program| {
            let identity = program
                .functions
                .values_mut()
                .find(|function| function.name == "identity")
                .unwrap();
            identity.body = crate::hir::HirBlockFor::<AcceptedHir> {
                stmts: vec![crate::hir::HirStmtFor::Expr(crate::hir::HirExprFor::<
                    AcceptedHir,
                > {
                    kind: crate::hir::HirExprKindFor::Match {
                        scrutinee: Box::new(crate::hir::HirExprFor::<AcceptedHir> {
                            kind: crate::hir::HirExprKindFor::Var("value".to_string()),
                            ty: Type::I64,
                            span: Span::test(),
                        }),
                        arms: vec![crate::hir::HirMatchArmFor::<AcceptedHir> {
                            pattern: HirPattern::Struct(
                                "Box".to_string(),
                                None,
                                vec![pattern_arg_ty.clone()],
                                vec![HirStructPatternField {
                                    name: "value".to_string(),
                                    field: None,
                                    pattern: HirPattern::Wildcard,
                                }],
                            ),
                            guard: None,
                            body: crate::hir::HirBlockFor::<AcceptedHir> {
                                stmts: Vec::new(),
                                ty: Type::Unit,
                            },
                        }],
                    },
                    ty: Type::Unit,
                    span: Span::test(),
                })],
                ty: Type::Unit,
            };
        });
        refresh_resolved_type_ids(&mut resolved);
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &resolved,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let bytes = products.to_artifact_bytes().unwrap();
        let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();
        let function = roundtrip
            .bodies
            .functions
            .values()
            .find(|function| function.name == "identity")
            .unwrap();
        let crate::hir::HirStmtFor::Expr(expr) = &function.body.stmts[0] else {
            panic!("expected match expression statement");
        };
        let crate::hir::HirExprKindFor::Match { arms, .. } = &expr.kind else {
            panic!("expected match expression");
        };
        let HirPattern::Struct(_, _, type_args, _) = &arms[0].pattern else {
            panic!("expected struct pattern");
        };

        assert_eq!(type_args, &vec![pattern_arg_ty.clone()]);
        assert!(
            super::type_table::artifact_type_table_contains_type_for_test(&bytes, &pattern_arg_ty)
                .unwrap()
        );
    }

    #[test]
    fn compiler_products_preserve_proc_macro_exports() {
        use std::collections::BTreeMap;

        let mut products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local("macros".to_string()),
            identity_table: ProductIdentityTable::default(),
            interface: crate::products::ProductInterface::default(),
            bodies: ProductBodies::default(),
            link: ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: BTreeMap::new(),
            proc_macros: Vec::new(),
        };
        products
            .proc_macros
            .push(crate::macro_expansion::proc_macro::ProcMacroArtifact {
                artifact_format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
                crate_identity: "macros".to_string(),
                protocol_version: crate::macro_expansion::proc_macro::PROC_MACRO_PROTOCOL_VERSION,
                host_triple: "x86_64-unknown-linux-gnu".to_string(),
                executable: "macro-host".into(),
                capabilities: vec![crate::macro_expansion::proc_macro::ProcMacroCapability::Stdio],
                exports: vec![crate::macro_expansion::proc_macro::ProcMacroExport {
                    name: "make_main".to_string(),
                    identity: "macros::make_main".to_string(),
                    kind: crate::macro_expansion::proc_macro::ProcMacroKind::FunctionLike,
                    input_shape:
                        crate::macro_expansion::proc_macro::ProcMacroInputShape::TokenStream,
                }],
            });

        let bytes = products.to_artifact_bytes().unwrap();
        let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();

        assert_eq!(roundtrip.proc_macros[0].exports[0].name, "make_main");
    }

    #[test]
    fn compiler_products_serialize_structural_types_as_explicit_compatibility_boundary() {
        let resolved = resolved_hir_for_products();
        let identity_id = DefId::new(CrateId(0), LocalDefId(1));
        let ret_id = resolved
            .type_id_at(&crate::hir::HirTypeLocation::FunctionReturn {
                function: identity_id,
            })
            .unwrap();

        assert_eq!(resolved.type_at(ret_id), Type::I64);

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &resolved,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let roundtrip =
            CompilerProducts::from_artifact_bytes(&products.to_artifact_bytes().unwrap()).unwrap();
        let function = roundtrip
            .interface
            .functions
            .values()
            .find(|function| function.name == "identity")
            .unwrap();

        assert_eq!(function.ret_type, Type::I64);
        assert_eq!(function.params[0], Type::I64);
    }

    #[test]
    fn product_artifact_rejects_unsupported_format_version() {
        let bytes = ProductArtifactPreamble {
            magic: PRODUCT_ARTIFACT_MAGIC,
            format_version: PRODUCT_ARTIFACT_FORMAT_VERSION + 1,
            header_len: 0,
            payload_len: 0,
        }
        .encode();

        let err = CompilerProducts::from_artifact_bytes(&bytes).unwrap_err();

        assert!(err.contains("Unsupported product artifact format"));
    }

    #[test]
    fn product_artifact_preamble_rejects_invalid_magic_and_declared_sizes() {
        let invalid_magic = ProductArtifactPreamble {
            magic: *b"NOTRKCA!",
            format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
            header_len: 0,
            payload_len: 0,
        }
        .encode();
        assert!(CompilerProducts::from_artifact_bytes(&invalid_magic)
            .unwrap_err()
            .contains("magic"));

        let oversized_header = ProductArtifactPreamble {
            magic: PRODUCT_ARTIFACT_MAGIC,
            format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
            header_len: MAX_PRODUCT_ARTIFACT_HEADER_BYTES + 1,
            payload_len: 0,
        }
        .encode();
        let error = CompilerProducts::from_artifact_bytes(&oversized_header).unwrap_err();
        assert!(error.contains("header bytes limit exceeded"), "{error}");

        let oversized_payload = ProductArtifactPreamble {
            magic: PRODUCT_ARTIFACT_MAGIC,
            format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
            header_len: 0,
            payload_len: MAX_PRODUCT_ARTIFACT_BYTES,
        }
        .encode();
        let error = CompilerProducts::from_artifact_bytes(&oversized_payload).unwrap_err();
        assert!(error.contains("artifact bytes limit exceeded"), "{error}");

        let mismatched_size = ProductArtifactPreamble {
            magic: PRODUCT_ARTIFACT_MAGIC,
            format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
            header_len: 1,
            payload_len: 0,
        }
        .encode();
        let error = CompilerProducts::from_artifact_bytes(&mismatched_size).unwrap_err();
        assert!(error.contains("size mismatch"), "{error}");
    }

    #[test]
    fn product_artifact_header_decodes_without_reading_semantic_payload() {
        let hir = resolved_hir_for_products();
        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("header-only".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .unwrap();
        let mut bytes = products.to_artifact_bytes().unwrap();
        *bytes.last_mut().expect("payload byte") ^= 0xff;
        let mut cursor = std::io::Cursor::new(&bytes);

        let (_, header) = read_product_artifact_header(&mut cursor, bytes.len() as u64).unwrap();

        assert_eq!(header.crate_identity.name, "header-only");
        assert!(CompilerProducts::from_artifact_bytes(&bytes).is_err());
    }

    #[test]
    fn product_artifact_header_rejects_oversized_strings() {
        let mut products = CompilerProducts {
            crate_identity: ProductCrateIdentity::local(
                "x".repeat(MAX_PRODUCT_ARTIFACT_STRING_BYTES + 1),
            ),
            identity_table: ProductIdentityTable::default(),
            interface: Default::default(),
            bodies: Default::default(),
            link: Default::default(),
            dependencies: Vec::new(),
            source_fingerprint: Default::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        };

        let error = products.to_artifact_bytes().unwrap_err();

        assert!(error.contains("string bytes limit exceeded"), "{error}");

        products.crate_identity = ProductCrateIdentity::local("payload-strings".to_string());
        products
            .infix_precedence
            .insert("x".repeat(MAX_PRODUCT_ARTIFACT_STRING_BYTES + 1), 1);
        let error = products.to_artifact_bytes().unwrap_err();
        assert!(error.contains("string bytes limit exceeded"), "{error}");

        products.infix_precedence.clear();
        let artifact =
            type_table::products_to_artifact(&products, PRODUCT_ARTIFACT_FORMAT_VERSION).unwrap();
        let error = type_table::validate_portable_artifact_limits_with_initial_string_bytes(
            &artifact,
            MAX_PRODUCT_ARTIFACT_TOTAL_STRING_BYTES,
        )
        .unwrap_err();
        assert!(
            error.contains("total string bytes limit exceeded"),
            "{error}"
        );
    }

    #[test]
    fn compiler_products_reject_format_23_product_artifacts() {
        let bytes = ProductArtifactPreamble {
            magic: PRODUCT_ARTIFACT_MAGIC,
            format_version: 23,
            header_len: 0,
            payload_len: 0,
        }
        .encode();

        let err = CompilerProducts::from_artifact_bytes(&bytes).unwrap_err();

        assert!(
            err.contains("Unsupported product artifact format 23")
                && err.contains(&format!("expected {}", PRODUCT_ARTIFACT_FORMAT_VERSION)),
            "expected format-23 rejection, got {err}"
        );
    }

    #[test]
    fn compiler_products_rejects_previous_format_before_full_deserialize() {
        let previous_version = PRODUCT_ARTIFACT_FORMAT_VERSION - 1;
        let bytes = ProductArtifactPreamble {
            magic: PRODUCT_ARTIFACT_MAGIC,
            format_version: previous_version,
            header_len: 0,
            payload_len: 0,
        }
        .encode();

        let err = CompilerProducts::from_artifact_bytes(&bytes).unwrap_err();

        assert!(
            err.contains(&format!(
                "Unsupported product artifact format {previous_version}"
            )) && err.contains(&format!("expected {PRODUCT_ARTIFACT_FORMAT_VERSION}")),
            "expected format-{previous_version} rejection before full deserialize, got {err}"
        );
    }

    #[test]
    fn compiler_products_rejects_unsupported_format_before_full_deserialize() {
        let unsupported_version = PRODUCT_ARTIFACT_FORMAT_VERSION + 1;
        let bytes = ProductArtifactPreamble {
            magic: PRODUCT_ARTIFACT_MAGIC,
            format_version: unsupported_version,
            header_len: 0,
            payload_len: 0,
        }
        .encode();

        let err = CompilerProducts::from_artifact_bytes(&bytes).unwrap_err();

        assert!(
            err.contains(&format!(
                "Unsupported product artifact format {unsupported_version}"
            )) && err.contains(&format!("expected {}", PRODUCT_ARTIFACT_FORMAT_VERSION)),
            "expected format-{unsupported_version} rejection before full deserialize, got {err}"
        );
    }

    #[test]
    fn product_artifact_format_version_matches_shared_contract() {
        assert_eq!(PRODUCT_ARTIFACT_FORMAT_VERSION, 52);
        assert_eq!(
            PRODUCT_ARTIFACT_FORMAT_VERSION,
            rock_shared::sysroot::PRODUCT_ARTIFACT_FORMAT_VERSION
        );
    }

    #[test]
    fn compiler_products_preserve_backend_symbols_in_link_records() {
        let hir = resolved_hir_for_products();
        let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));
        let mut records = BTreeMap::new();
        records.insert(
            plain_id,
            ProductLinkRecord {
                backend_symbol: "rock_main".to_string(),
            },
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData {
                object_path: None,
                records,
            },
        )
        .expect("test HIR has valid product language items");

        assert_eq!(
            products
                .link
                .records
                .get(&plain_id)
                .map(|record| &record.backend_symbol),
            Some(&"rock_main".to_string())
        );
    }

    #[test]
    fn compiler_products_preserve_generic_impls_with_real_local_id_zero() {
        let mut hir = resolved_hir_for_products();
        let first_id = DefId::new(CrateId(0), LocalDefId(0));
        let second_id = DefId::new(CrateId(0), LocalDefId(99));
        hir.current_def_ids.insert(second_id);
        rebuild_program(
            &mut hir.program,
            |_, _, _, _, impls, _, _, effective_methods| {
                *impls = [
                    HirImplFor::<AcceptedHir> {
                        id: first_id,
                        owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                        type_name: "Box".to_string(),
                        type_generics: generic_params(first_id, &["T"]),
                        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                            id: DefId::new(CrateId(0), LocalDefId(2)),
                            args: vec![Type::Generic(crate::types::GenericParamId {
                                owner: first_id,
                                index: 0,
                            })],
                        }),
                        trait_name: Some("Show".to_string()),
                        trait_id: None,
                        trait_generics: Vec::new(),
                        trait_arg_types: Vec::new(),
                        associated_types: Vec::new(),
                        bounds: std::collections::HashMap::new().into(),
                        methods: HashMap::new(),
                    },
                    HirImplFor::<AcceptedHir> {
                        id: second_id,
                        owner: crate::hir::HirImplOwner::Named("Maybe".to_string()),
                        type_name: "Maybe".to_string(),
                        type_generics: generic_params(second_id, &["T"]),
                        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Enum {
                            id: DefId::new(CrateId(0), LocalDefId(3)),
                            args: vec![Type::Generic(crate::types::GenericParamId {
                                owner: second_id,
                                index: 0,
                            })],
                        }),
                        trait_name: Some("Show".to_string()),
                        trait_id: None,
                        trait_generics: Vec::new(),
                        trait_arg_types: Vec::new(),
                        associated_types: Vec::new(),
                        bounds: std::collections::HashMap::new().into(),
                        methods: HashMap::new(),
                    },
                ]
                .into_iter()
                .map(|impl_def| (impl_def.id, impl_def))
                .collect();
                effective_methods.clear();
            },
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        assert_eq!(products.interface.impls.len(), 2);
        assert_eq!(products.bodies.generic_impls.len(), 2);
    }

    #[test]
    fn compiler_products_preserve_real_impl_id_zero_without_collision() {
        let impl_id = DefId::new(CrateId(0), LocalDefId(0));
        let owner_id = DefId::new(CrateId(0), LocalDefId(1));
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::from([(
                owner_id,
                HirStruct {
                    id: owner_id,
                    name: "Box".to_string(),
                    generic_params: Vec::new(),
                    fields: Vec::new(),
                },
            )]),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                impl_id,
                HirImpl {
                    id: impl_id,
                    owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: Vec::new(),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: owner_id,
                        args: Vec::new(),
                    }),
                    trait_name: Some("Show".to_string()),
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::new(),
                },
            )]),
            HashMap::new(),
            HirNameTables {
                structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([(owner_id, "Box".to_string())]),
        );
        let hir = ResolvedHirProgram::new(
            program,
            ResolverTables::default(),
            BTreeSet::from([impl_id]),
            CrateId(0),
            local_def_ids_after(1),
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let product_id = ProductDefId::from(impl_id);

        assert!(products.interface.impls.contains_key(&product_id));
        assert_eq!(products.interface.impls[&product_id].id, impl_id);
    }

    #[test]
    fn compiler_products_preserve_real_extern_id_zero_without_collision() {
        let extern_id = DefId::new(CrateId(0), LocalDefId(0));
        let hir = ResolvedHirProgram::new(
            HirProgram::from_id_parts_with_names_and_canonical_names(
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::from([(
                    extern_id,
                    HirExtern {
                        id: extern_id,
                        name: "puts".to_string(),
                        params: vec![Type::Pointer(Box::new(Type::U8))],
                        ret: Type::I32,
                        variadic: false,
                        is_unsafe: false,
                    },
                )]),
                HirNameTables {
                    externs_by_name: HashMap::from([("puts".to_string(), extern_id)]),
                    ..HirNameTables::default()
                },
                HirLanguageItems::default(),
                &HashMap::new(),
            ),
            ResolverTables::default(),
            BTreeSet::from([extern_id]),
            CrateId(0),
            local_def_ids_after(1),
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let product_id = ProductDefId::from(extern_id);

        assert!(products.interface.externs.contains_key(&product_id));
        assert_eq!(products.interface.externs[&product_id].id, extern_id);
    }

    #[test]
    fn compiler_products_preserve_trait_defaults_with_distinct_method_ids() {
        let mut hir = resolved_hir_for_products();
        let trait_id = DefId::new(CrateId(0), LocalDefId(8));
        let method_id = DefId::new(CrateId(0), LocalDefId(9));
        hir.current_def_ids.insert(trait_id);
        hir.current_def_ids.insert(method_id);
        let mut methods = HashMap::new();
        methods.insert(
            "show".to_string(),
            accept_function(test_function(method_id, "show", Vec::new())),
        );
        rebuild_program(&mut hir.program, |_, _, _, traits, _, _, _, _| {
            traits.insert(
                trait_id,
                HirTraitFor::<AcceptedHir> {
                    target: None,
                    predicates: Vec::new(),
                    id: trait_id,
                    name: "Debug".to_string(),
                    generic_params: Vec::new(),
                    associated_types: Vec::new(),
                    methods,
                    signatures: HashMap::new(),
                },
            );
        });

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        assert_eq!(products.bodies.trait_default_methods.len(), 2);
        assert_eq!(
            products
                .interface
                .object_abi
                .trait_members
                .keys()
                .copied()
                .collect::<BTreeSet<_>>(),
            products
                .interface
                .traits
                .values()
                .map(|declaration| declaration.id)
                .collect(),
        );
    }

    #[test]
    fn compiler_products_record_trait_default_method_ids_in_metadata() {
        let mut hir = resolved_hir_for_products();
        let trait_id = DefId::new(CrateId(0), LocalDefId(8));
        let method_id = DefId::new(CrateId(0), LocalDefId(0));
        hir.current_def_ids.insert(trait_id);
        let mut methods = HashMap::new();
        methods.insert(
            "show".to_string(),
            accept_function(test_function(method_id, "show", Vec::new())),
        );
        rebuild_program(&mut hir.program, |_, _, _, traits, _, _, _, _| {
            traits.insert(
                trait_id,
                HirTraitFor::<AcceptedHir> {
                    target: None,
                    predicates: Vec::new(),
                    id: trait_id,
                    name: "Debug".to_string(),
                    generic_params: Vec::new(),
                    associated_types: Vec::new(),
                    methods,
                    signatures: HashMap::new(),
                },
            );
        });
        hir.local_def_ids = local_def_ids_after(9);
        let next_raw = hir.local_def_ids.next_raw();

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let debug_trait_id = ProductDefId::from(trait_id);
        let interface_method_id = products.interface.traits[&debug_trait_id].methods["show"].id;
        let debug_method_id = ProductDefId::from(interface_method_id);

        assert!(products
            .interface
            .object_abi
            .trait_members
            .contains_key(&products.interface.traits[&debug_trait_id].id,));

        assert_ne!(debug_method_id, ProductDefId::from(method_id));
        assert!(debug_method_id.local_id.0 >= next_raw);

        assert_eq!(
            products.bodies.trait_default_methods[&debug_method_id].id,
            interface_method_id
        );
        assert_eq!(
            products.interface.traits[&debug_trait_id].methods["show"].id,
            interface_method_id
        );
    }

    #[test]
    fn compiler_products_prefer_exact_trait_default_method_identity() {
        let shared_id = DefId::new(CrateId(0), LocalDefId(7));
        let exact_method = test_function(shared_id, "right", Vec::new());
        let accepted = crate::hir::AcceptedHirProgram::try_from(
            HirProgram::from_id_parts_with_names_and_canonical_names(
                HashMap::from([(shared_id, exact_method)]),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HirNameTables {
                    functions_by_name: HashMap::from([("right".to_string(), shared_id)]),
                    ..HirNameTables::default()
                },
                HirLanguageItems::default(),
                &HashMap::from([(shared_id, "right".to_string())]),
            ),
        )
        .expect("fixture HIR is accepted");
        let exact_method = accepted.function_by_id(shared_id).unwrap().1.clone();
        let preferred = super::PreferredTraitDefaultMethods {
            by_id: HashMap::from([(shared_id, exact_method)]),
        };

        let selected = preferred.get(shared_id, "Debug", "show");

        assert_eq!(selected.name, "right");
    }

    #[test]
    #[should_panic(expected = "trait default method should have a preferred body")]
    fn compiler_products_do_not_fallback_trait_defaults_by_display_name() {
        let missing_id = DefId::new(CrateId(0), LocalDefId(7));
        let preferred = super::PreferredTraitDefaultMethods {
            by_id: HashMap::new(),
        };

        let _ = preferred.get(missing_id, "Debug", "show");
    }

    #[test]
    fn compiler_products_record_impl_display_names_without_export_names() {
        let hir = resolved_hir_for_products();

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        assert!(products
            .identity_table
            .display_names
            .values()
            .any(|name| name == "Box as Show"));
        assert!(!products
            .identity_table
            .export_names
            .contains_key("Box as Show"));
    }

    #[test]
    fn compiler_products_do_not_export_trait_default_methods() {
        let hir = resolved_hir_for_products();

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        assert!(products
            .identity_table
            .display_names
            .values()
            .any(|name| name == "Show::show"));
        assert!(!products
            .identity_table
            .export_names
            .contains_key("Show::show"));
    }

    #[test]
    fn compiler_products_do_not_export_colliding_impl_method_display_names() {
        let method_a_id = DefId::new(CrateId(0), LocalDefId(10));
        let method_b_id = DefId::new(CrateId(0), LocalDefId(11));
        let impl_a_id = DefId::new(CrateId(0), LocalDefId(12));
        let impl_b_id = DefId::new(CrateId(0), LocalDefId(13));
        let owner_id = DefId::new(CrateId(0), LocalDefId(14));
        let impls = HashMap::from([
            (
                impl_a_id,
                HirImpl {
                    id: impl_a_id,
                    owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: Vec::new(),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: owner_id,
                        args: Vec::new(),
                    }),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::from([(
                        "show".to_string(),
                        test_function(method_a_id, "show", Vec::new()),
                    )]),
                },
            ),
            (
                impl_b_id,
                HirImpl {
                    id: impl_b_id,
                    owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: Vec::new(),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: owner_id,
                        args: Vec::new(),
                    }),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::from([(
                        "show".to_string(),
                        test_function(method_b_id, "show", Vec::new()),
                    )]),
                },
            ),
        ]);
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::from([(
                owner_id,
                HirStruct {
                    id: owner_id,
                    name: "Box".to_string(),
                    generic_params: Vec::new(),
                    fields: Vec::new(),
                },
            )]),
            HashMap::new(),
            HashMap::new(),
            impls,
            HashMap::new(),
            HirNameTables {
                structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([(owner_id, "Box".to_string())]),
        );
        let hir = ResolvedHirProgram::new(
            program,
            ResolverTables::default(),
            BTreeSet::from([impl_a_id, impl_b_id, method_a_id, method_b_id]),
            CrateId(0),
            local_def_ids_after(22),
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        let colliding_methods = products
            .identity_table
            .display_names
            .values()
            .filter(|name| *name == "Box::show")
            .count();
        assert_eq!(colliding_methods, 2);
        assert!(!products
            .identity_table
            .export_names
            .contains_key("Box::show"));
    }

    #[test]
    fn compiler_products_drop_ambiguous_export_name_collisions() {
        let mut identity_table = ProductIdentityTable::default();
        let first = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(1),
        };
        let second = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(2),
        };

        super::record_export_name(&mut identity_table, first, "same");
        super::record_export_name(&mut identity_table, second, "same");

        assert!(!identity_table.export_names.contains_key("same"));
    }

    #[test]
    fn compiler_products_keep_export_name_ambiguous_after_third_collision() {
        let mut identity_table = ProductIdentityTable::default();
        let first = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(1),
        };
        let second = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(2),
        };
        let third = ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(3),
        };

        super::record_export_name(&mut identity_table, first, "same");
        super::record_export_name(&mut identity_table, second, "same");
        super::record_export_name(&mut identity_table, third, "same");

        assert!(!identity_table.export_names.contains_key("same"));
    }

    #[test]
    fn compiler_products_preserve_link_records_for_real_local_id_zero() {
        let impl_def_id = DefId::new(CrateId(0), LocalDefId(0));
        let mut records = BTreeMap::new();
        records.insert(
            ProductDefId::from(impl_def_id),
            ProductLinkRecord {
                backend_symbol: "impl_box_show".to_string(),
            },
        );
        let owner_id = DefId::new(CrateId(0), LocalDefId(1));
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::from([(
                owner_id,
                HirStruct {
                    id: owner_id,
                    name: "Box".to_string(),
                    generic_params: generic_params(owner_id, &["T"]),
                    fields: Vec::new(),
                },
            )]),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                impl_def_id,
                HirImpl {
                    id: impl_def_id,
                    owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: generic_params(impl_def_id, &["T"]),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: owner_id,
                        args: vec![Type::Generic(crate::types::GenericParamId {
                            owner: impl_def_id,
                            index: 0,
                        })],
                    }),
                    trait_name: Some("Show".to_string()),
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::new(),
                },
            )]),
            HashMap::new(),
            HirNameTables {
                structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([(owner_id, "Box".to_string())]),
        );
        let hir = ResolvedHirProgram::new(
            program,
            ResolverTables::default(),
            BTreeSet::from([impl_def_id]),
            CrateId(0),
            IdGen::new(),
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData {
                object_path: None,
                records,
            },
        )
        .expect("test HIR has valid product language items");
        let impl_id = products.interface.impls.keys().copied().next().unwrap();

        assert_eq!(impl_id, ProductDefId::from(impl_def_id));
        assert_eq!(
            products.interface.impls[&impl_id].id,
            super::product_def_id_to_def_id(impl_id)
        );
        assert_eq!(
            products
                .link
                .records
                .get(&impl_id)
                .map(|record| &record.backend_symbol),
            Some(&"impl_box_show".to_string())
        );
    }

    #[test]
    fn compiler_products_drop_link_records_for_ambiguous_link_ids() {
        let shared_id = DefId::new(CrateId(0), LocalDefId(0));
        let shared_product_id = ProductDefId::from(shared_id);
        let symbol = "ambiguous_impl_symbol".to_string();
        let mut records = BTreeMap::new();
        records.insert(
            shared_product_id,
            ProductLinkRecord {
                backend_symbol: symbol.clone(),
            },
        );
        let mut functions = HashMap::new();
        functions.insert(shared_id, test_function(shared_id, "plain", Vec::new()));
        let owner_id = DefId::new(CrateId(0), LocalDefId(1));
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            functions,
            HashMap::from([(
                owner_id,
                HirStruct {
                    id: owner_id,
                    name: "Box".to_string(),
                    generic_params: generic_params(owner_id, &["T"]),
                    fields: Vec::new(),
                },
            )]),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                shared_id,
                HirImpl {
                    id: shared_id,
                    owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: generic_params(shared_id, &["T"]),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: owner_id,
                        args: vec![Type::Generic(crate::types::GenericParamId {
                            owner: shared_id,
                            index: 0,
                        })],
                    }),
                    trait_name: Some("Show".to_string()),
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::new(),
                },
            )]),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("plain".to_string(), shared_id)]),
                structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([
                (shared_id, "plain".to_string()),
                (owner_id, "Box".to_string()),
            ]),
        );
        let hir = ResolvedHirProgram::new(
            program,
            ResolverTables::default(),
            BTreeSet::from([shared_id]),
            CrateId(0),
            IdGen::new(),
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData {
                object_path: None,
                records,
            },
        )
        .expect("test HIR has valid product language items");

        assert!(!products
            .link
            .records
            .values()
            .any(|record| record.backend_symbol == symbol));
    }

    #[test]
    fn compiler_products_drop_aliases_for_ambiguous_product_ids() {
        let shared_id = DefId::new(CrateId(0), LocalDefId(0));
        let mut functions = HashMap::new();
        functions.insert(shared_id, test_function(shared_id, "plain", Vec::new()));
        let mut resolver = ResolverTables::default();
        resolver.insert_import_alias_with_name(
            "short".to_string(),
            "demo::plain".to_string(),
            shared_id,
        );
        resolver.insert_module_alias_with_name(
            "mod_short".to_string(),
            "demo::plain".to_string(),
            shared_id,
        );
        let owner_id = DefId::new(CrateId(0), LocalDefId(1));
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            functions,
            HashMap::from([(
                owner_id,
                HirStruct {
                    id: owner_id,
                    name: "Box".to_string(),
                    generic_params: generic_params(owner_id, &["T"]),
                    fields: Vec::new(),
                },
            )]),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                shared_id,
                HirImpl {
                    id: shared_id,
                    owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: generic_params(shared_id, &["T"]),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: owner_id,
                        args: vec![Type::Generic(crate::types::GenericParamId {
                            owner: shared_id,
                            index: 0,
                        })],
                    }),
                    trait_name: Some("Show".to_string()),
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::new(),
                },
            )]),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("plain".to_string(), shared_id)]),
                structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::from([
                (shared_id, "plain".to_string()),
                (owner_id, "Box".to_string()),
            ]),
        );
        let hir = ResolvedHirProgram::new(
            program,
            resolver,
            BTreeSet::from([shared_id]),
            CrateId(0),
            IdGen::new(),
        );

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &hir,
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        assert!(!products
            .identity_table
            .import_alias_names
            .contains_key("short"));
        assert!(!products
            .identity_table
            .module_alias_names
            .contains_key("mod_short"));
    }

    #[test]
    fn compiler_products_keep_trait_default_ids_stable_across_trait_order() {
        let first = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &resolved_hir_with_traits_in_order(&["Show", "Debug"]),
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");
        let second = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &resolved_hir_with_traits_in_order(&["Debug", "Show"]),
            Vec::new(),
            BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("test HIR has valid language items");

        for display_name in ["Debug::show", "Show::show"] {
            let first_id = first
                .identity_table
                .display_names
                .iter()
                .find_map(|(id, name)| (name == display_name).then_some(*id));
            let second_id = second
                .identity_table
                .display_names
                .iter()
                .find_map(|(id, name)| (name == display_name).then_some(*id));

            assert_eq!(first_id, second_id, "{display_name}");
        }
    }
}
