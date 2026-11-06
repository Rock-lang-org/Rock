use super::ArithmeticSummary;
use std::collections::{BTreeMap, HashMap};

use super::artifact_metadata::{
    ProductBodies, ProductCrateIdentity, ProductDependencyIdentity, ProductIdentityTable,
    ProductLanguageItems, ProductLinkData, ProductSourceFingerprint,
};
use super::ProductDefId;
use crate::hir::{
    HirEnum, HirExtern, HirFunctionFor, HirGenericBounds, HirImplFor, HirPhase, HirStruct,
    HirTraitFor, HirTypeAlias, HirVariantFields,
};
use crate::ids::DefId;
use crate::types::{GenericParamDecl, Type};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilerProducts {
    pub crate_identity: ProductCrateIdentity,
    pub identity_table: ProductIdentityTable,
    pub interface: ProductInterface,
    pub bodies: ProductBodies,
    pub link: ProductLinkData,
    pub dependencies: Vec<ProductDependencyIdentity>,
    pub source_fingerprint: ProductSourceFingerprint,
    /// Every operator visible to this crate, including inherited ones.
    pub infix_precedence: BTreeMap<String, ProductInfixOperator>,
    pub proc_macros: Vec<crate::macro_expansion::proc_macro::ProcMacroArtifact>,
}

/// An `infix` declaration as seen by a crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductInfixOperator {
    pub precedence: u8,
    /// The crate whose source declares the operator. Crates re-list the
    /// operators they inherit, so this tells one declaration seen through two
    /// dependencies apart from two different declarations.
    pub declaring_crate: String,
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
