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

pub(crate) mod fingerprint;
pub(crate) mod identity;
pub(crate) mod link;
mod type_table;

pub use crate::infer::constants::{ArithmeticSummary, ArithmeticType};

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

pub const PRODUCT_ARTIFACT_FORMAT_VERSION: u32 = 51;
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
    /// Declarative macros exported with `< macro`.
    #[serde(default)]
    pub macros: Vec<ProductMacroExport>,
    /// Prelude aliases of exported macros, such as `format` -> `fmt::format`.
    #[serde(default)]
    pub prelude_macros: BTreeMap<String, String>,
}

/// A declarative macro exported with `< macro name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductMacroExport {
    /// Path of the macro inside its crate, without the crate name (`fmt::format`).
    pub path: String,
    pub decl: crate::ast::MacroDecl,
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
    pub constant_arithmetic: Option<ArithmeticSummary>,
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
            constant_arithmetic: None,
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
            let mut signature_backed_method_ids = BTreeMap::new();
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
                if trt
                    .methods
                    .get(&signature_name)
                    .is_some_and(|method| method.id == original_signature_id)
                {
                    signature_backed_method_ids.insert(signature_name.clone(), signature_id);
                }
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
                // A default body and its signature share one canonical member ID.
                // Reserving it twice makes call-target remapping ambiguous.
                let method_id = signature_backed_method_ids
                    .get(&method_name)
                    .copied()
                    .unwrap_or_else(|| {
                        reserve_product_id(
                            requested_method_id,
                            &mut used_ids,
                            fallback_crate,
                            &mut next_fallback_local,
                            false,
                        )
                    });
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
        let summaries = hir
            .constant_arithmetic
            .iter()
            .flat_map(|(id, summary)| {
                let product_id = ProductDefId::from(*id);
                id_remap
                    .get(&product_id)
                    .into_iter()
                    .flatten()
                    .map(move |id| (*id, summary))
            })
            .collect::<BTreeMap<_, _>>();
        for function in interface
            .functions
            .values_mut()
            .chain(
                interface
                    .impls
                    .values_mut()
                    .flat_map(|imp| imp.methods.values_mut()),
            )
            .chain(
                interface
                    .traits
                    .values_mut()
                    .flat_map(|trt| trt.methods.values_mut()),
            )
        {
            function.constant_arithmetic = summaries
                .get(&ProductDefId::from(function.id))
                .map(|summary| (*summary).clone());
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
        Type::Struct { id, .. } | Type::Enum { id, .. } | Type::Constructor { id, .. } => {
            assert_valid_product_input_def_id(*id)
        }
        Type::Generic(param) => assert_valid_product_input_def_id(param.owner),
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
        if let Some(default) = &decl.default {
            assert_valid_product_type_ids(default);
        }
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
            ..
        } => {
            assert_valid_product_input_def_id(*trait_id);
            assert_valid_product_input_def_id(*member_id);
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
        if let Some(default) = &mut generic_decl.default {
            default.remap_def_ids(&mut |id| {
                unambiguous_product_id(ProductDefId::from(id), id_remap)
                    .map(product_def_id_to_def_id)
                    .unwrap_or(id)
            });
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
            ..
        } => {
            remap_product_def_id_owner(trait_id, id_remap);
            remap_product_def_id_owner(member_id, id_remap);
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
            if let Type::Generic(param) = &mut ty {
                if param.owner == self.old_id {
                    param.owner = self.new_id;
                }
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
        if let Some(default) = &mut generic_decl.default {
            remap_type_owned_generic_owner(default, old_id, new_id);
        }
    }
}

fn remap_generic_bounds_owned_owner(
    bounds: &mut crate::hir::HirGenericBounds,
    old_id: DefId,
    new_id: DefId,
) {
    let mut original = std::mem::take(bounds);
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
mod tests;
