#![allow(dead_code)]

pub(super) mod object_abi;

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::products::{
    CompilerProducts, ProductAssociatedTypeInterface, ProductBodies, ProductCrateId,
    ProductCrateIdentity, ProductDefId, ProductDependencyIdentity, ProductEnumInterface,
    ProductEnumVariantInterface, ProductExternInterface, ProductFreshnessMetadata,
    ProductFunctionInterface, ProductIdentityTable, ProductImplInterface, ProductInterface,
    ProductLanguageItems, ProductLinkData, ProductLocalDefId, ProductSourceFingerprint,
    ProductStructInterface, ProductTraitInterface, ProductTypeAliasInterface,
};
use crate::type_services::kind::Kind;
use crate::types::{
    AssociatedTypeKey, CallableKind, CaptureKind, FunctionCapture, FunctionSafety,
    GenericParamDecl, GenericParamId, Type,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedProductArtifact {
    pub format_version: u32,
    pub type_table: ProductTypeTable,
    pub products: SerializedCompilerProducts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedCompilerProducts {
    pub crate_identity: ProductCrateIdentity,
    pub identity_table: ProductIdentityTable,
    pub freshness: ProductFreshnessMetadata,
    pub interface: SerializedProductInterface,
    pub bodies: SerializedProductBodies,
    pub link: ProductLinkData,
    pub dependencies: Vec<ProductDependencyIdentity>,
    pub source_fingerprint: ProductSourceFingerprint,
    pub infix_precedence: BTreeMap<String, u8>,
    pub proc_macros: Vec<crate::macro_expansion::proc_macro::ProcMacroArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct SerializedProductInterface {
    pub object_abi: object_abi::SerializedObjectAbi,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum SerializedHirImplReceiverPattern {
    Exact(ProductTypeId),
    SliceFamily { element: ProductTypeId },
    Constructor(ProductTypeId),
}

impl SerializedHirImplReceiverPattern {
    fn encode(
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

    fn decode(
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
pub(super) struct SerializedProductBodies {
    pub functions: BTreeMap<ProductDefId, SerializedHirFunction>,
    pub generic_impls: BTreeMap<ProductDefId, SerializedHirImpl>,
    pub trait_default_methods: BTreeMap<ProductDefId, SerializedHirFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirFunction {
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
pub(super) struct SerializedProductFunctionInterface {
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedProductStructInterface {
    pub id: crate::ids::DefId,
    pub name: String,
    pub generic_params: Vec<ProductGenericParamDecl>,
    pub fields: Vec<SerializedHirField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedProductTypeAliasInterface {
    pub id: crate::ids::DefId,
    pub name: String,
    pub generic_params: Vec<ProductGenericParamDecl>,
    pub ty: ProductTypeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedProductEnumInterface {
    pub id: crate::ids::DefId,
    pub name: String,
    pub generic_params: Vec<ProductGenericParamDecl>,
    pub variants: Vec<SerializedProductEnumVariantInterface>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedProductEnumVariantInterface {
    pub id: crate::ids::VariantId,
    pub name: String,
    pub fields: SerializedHirVariantFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedProductTraitInterface {
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
pub(super) struct SerializedProductImplInterface {
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
pub(super) struct SerializedProductExternInterface {
    pub id: crate::ids::DefId,
    pub name: String,
    pub params: Vec<ProductTypeId>,
    pub ret: ProductTypeId,
    pub variadic: bool,
    #[serde(default)]
    pub is_unsafe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirParam {
    pub name: String,
    pub local_id: crate::ids::HirLocalId,
    pub ty: ProductTypeId,
    pub mutable: bool,
    pub is_ref: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirField {
    pub id: crate::ids::FieldId,
    pub name: String,
    pub ty: ProductTypeId,
    pub public: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirStruct {
    pub id: crate::ids::DefId,
    pub name: String,
    pub generic_params: Vec<ProductGenericParamDecl>,
    pub fields: Vec<SerializedHirField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirEnum {
    pub id: crate::ids::DefId,
    pub name: String,
    pub generic_params: Vec<ProductGenericParamDecl>,
    pub variants: Vec<SerializedHirVariant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirVariant {
    pub id: crate::ids::VariantId,
    pub name: String,
    pub fields: SerializedHirVariantFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum SerializedHirVariantFields {
    Named(Vec<SerializedHirField>),
    Positional(Vec<ProductTypeId>),
    Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirFunctionSig {
    pub id: crate::ids::DefId,
    pub name: String,
    pub generic_params: Vec<ProductGenericParamDecl>,
    pub params: Vec<ProductTypeId>,
    pub ret: ProductTypeId,
    pub generic_bounds: SerializedGenericBounds,
    pub self_receiver: Option<crate::types::ReceiverMode>,
    pub is_unsafe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirImpl {
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
    pub methods: HashMap<String, SerializedHirFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirAssociatedTypeDef {
    pub id: crate::ids::AssocTypeId,
    pub name: String,
    pub kind: crate::type_services::kind::Kind,
    pub ty: ProductTypeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedTraitBound {
    pub trait_id: ProductDefId,
    pub type_args: Vec<ProductTypeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedGenericBounds {
    pub relaxed_sized: Vec<ProductGenericParamId>,
    pub bounds: Vec<(ProductGenericParamId, Vec<SerializedTraitBound>)>,
    pub predicates: Vec<SerializedPredicate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedPredicate {
    pub subject: ProductTypeId,
    pub trait_id: ProductDefId,
    pub args: Vec<ProductTypeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirExtern {
    pub id: crate::ids::DefId,
    pub name: String,
    pub params: Vec<ProductTypeId>,
    pub ret: ProductTypeId,
    pub variadic: bool,
    #[serde(default)]
    pub is_unsafe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirClosureCapture {
    pub name: String,
    pub local_id: crate::ids::HirLocalId,
    pub kind: crate::hir::HirClosureCaptureKind,
    pub mutable: bool,
    pub ty: ProductTypeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirBlock {
    pub stmts: Vec<SerializedHirStmt>,
    pub ty: ProductTypeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum SerializedHirStmt {
    Let {
        name: String,
        local_id: crate::ids::HirLocalId,
        ty: ProductTypeId,
        value: SerializedHirExpr,
        mutable: bool,
    },
    Expr(SerializedHirExpr),
    Return(Option<SerializedHirExpr>),
    Break(Option<SerializedHirExpr>),
    Continue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirExpr {
    pub kind: SerializedHirExprKind,
    pub ty: ProductTypeId,
    pub span: crate::lexer::Span,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum SerializedHirExprKind {
    Open {
        source: Box<SerializedHirExpr>,
        binding: SerializedOpenBinding,
        body: SerializedHirBlock,
    },
    IntLiteral(i64),
    FloatLiteral(f64),
    BoolLiteral(bool),
    StringLiteral(String),
    CharLiteral(char),
    ArrayLiteral(Vec<SerializedHirExpr>),
    ArrayRepeat(Box<SerializedHirExpr>, usize),
    TupleLiteral(Vec<SerializedHirExpr>),
    Unit,
    Var(String),
    ResolvedVar(SerializedHirVarRef),
    FieldAccess(
        Box<SerializedHirExpr>,
        String,
        Option<crate::hir::HirFieldLocation>,
    ),
    TupleIndex(Box<SerializedHirExpr>, u32),
    BinOp(
        crate::hir::BinOp,
        Box<SerializedHirExpr>,
        Box<SerializedHirExpr>,
    ),
    UnaryOp(crate::hir::UnaryOp, Box<SerializedHirExpr>),
    Call(
        Box<SerializedHirExpr>,
        Vec<SerializedHirExpr>,
        Option<SerializedHirCallTarget>,
    ),
    MethodCall(
        Box<SerializedHirExpr>,
        String,
        Vec<SerializedHirExpr>,
        Option<crate::types::ReceiverMode>,
        Option<SerializedHirMethodCallTarget>,
    ),
    Try {
        expr: Box<SerializedHirExpr>,
        branch_method: Option<SerializedHirMethodCallTarget>,
        branch_self_receiver: Option<crate::types::ReceiverMode>,
        from_residual_target: Option<SerializedHirCallTarget>,
        output_ty: ProductTypeId,
        residual_ty: ProductTypeId,
        return_ty: ProductTypeId,
        control_flow_enum: crate::ids::DefId,
        break_variant: crate::hir::HirVariantLocation,
        continue_variant: crate::hir::HirVariantLocation,
    },
    StructLiteral(
        String,
        Option<crate::ids::DefId>,
        Vec<SerializedHirStructLiteralField>,
    ),
    EnumVariant(
        String,
        String,
        Vec<SerializedHirExpr>,
        Option<crate::hir::HirVariantLocation>,
    ),
    If {
        condition: Box<SerializedHirExpr>,
        then_branch: SerializedHirBlock,
        else_branch: Option<SerializedHirBlock>,
    },
    Match {
        scrutinee: Box<SerializedHirExpr>,
        arms: Vec<SerializedHirMatchArm>,
    },
    While {
        condition: Box<SerializedHirExpr>,
        body: SerializedHirBlock,
    },
    For {
        var: String,
        local_id: crate::ids::HirLocalId,
        iter: Box<SerializedHirExpr>,
        body: SerializedHirBlock,
    },
    Loop(SerializedHirBlock),
    Block(SerializedHirBlock),
    Lambda {
        params: Vec<SerializedHirParam>,
        body: SerializedHirBlock,
        captures: Vec<SerializedHirClosureCapture>,
    },
    Ref(bool, Box<SerializedHirExpr>),
    Deref(Box<SerializedHirExpr>),
    Cast(Box<SerializedHirExpr>, ProductTypeId),
    ObjectCoercion(Box<SerializedHirExpr>, SerializedObjectCoercion),
    OwnedObjectCall {
        owner: Box<SerializedHirExpr>,
        args: Vec<SerializedHirExpr>,
        call: SerializedOwnedObjectCall,
    },
    Assign(Box<SerializedHirExpr>, Box<SerializedHirExpr>),
    Intrinsic {
        name: String,
        args: Vec<SerializedHirExpr>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum SerializedHirCallTarget {
    Function(crate::ids::DefId),
    Extern(crate::ids::DefId),
    Local(crate::ids::HirLocalId),
    Intrinsic(String),
    StaticMethod {
        owner_ty: ProductTypeId,
        method: SerializedHirMethodCallTarget,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirVarRef {
    name: String,
    target: SerializedHirVarTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum SerializedHirVarTarget {
    Function(crate::ids::DefId),
    Extern(crate::ids::DefId),
    Local(crate::ids::HirLocalId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirMethodCallTarget {
    pub target: SerializedHirSelectedMethodTarget,
    pub owner_substitution: Vec<SerializedHirTypeBinding>,
    pub method_substitution: Vec<SerializedHirTypeBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedOwnedObjectCall {
    method: SerializedHirMethodCallTarget,
    object: ProductTypeId,
    state: ProductTypeId,
    into_owner: ProductTypeId,
    into_parts: SerializedHirMethodCallTarget,
    release_owner: ProductTypeId,
    release: SerializedHirMethodCallTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedOpenBinding {
    witness: ProductWitnessId,
    value: SerializedHirParam,
    object: ProductTypeId,
    source_ty: ProductTypeId,
    owner: Option<SerializedOpenOwner>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedOpenOwner {
    state_ty: ProductTypeId,
    into_owner: ProductTypeId,
    into_parts: SerializedHirMethodCallTarget,
    from_owner: ProductTypeId,
    from_parts: SerializedHirMethodCallTarget,
}

impl SerializedOpenBinding {
    fn encode(
        binding: &crate::hir::HirOpenBinding,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            witness: ProductWitnessId::encode(binding.witness),
            value: SerializedHirParam::encode(&binding.value, encoder)?,
            object: encoder.encode_type(&Type::Object(Box::new(binding.object.clone())))?,
            source_ty: encoder.encode_type(&binding.source_ty)?,
            owner: binding
                .owner
                .as_ref()
                .map(|owner| {
                    Ok::<_, String>(SerializedOpenOwner {
                        state_ty: encoder.encode_type(&owner.state_ty)?,
                        into_owner: encoder.encode_type(&owner.into_parts.owner_ty)?,
                        into_parts: SerializedHirMethodCallTarget::encode(
                            &owner.into_parts.method,
                            encoder,
                        )?,
                        from_owner: encoder.encode_type(&owner.from_parts.owner_ty)?,
                        from_parts: SerializedHirMethodCallTarget::encode(
                            &owner.from_parts.method,
                            encoder,
                        )?,
                    })
                })
                .transpose()?,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirOpenBinding, String> {
        let Type::Object(object) = decoder.decode_type(self.object)? else {
            return Err("opening schema must reference an object type row".into());
        };
        Ok(crate::hir::HirOpenBinding {
            witness: self.witness.decode(),
            value: self.value.decode(decoder)?,
            object: *object,
            source_ty: decoder.decode_type(self.source_ty)?,
            owner: self
                .owner
                .map(|owner| {
                    Ok::<_, String>(crate::hir::HirOpenOwner {
                        state_ty: decoder.decode_type(owner.state_ty)?,
                        into_parts: crate::hir::HirStaticMethodTarget {
                            owner_ty: decoder.decode_type(owner.into_owner)?,
                            method: owner.into_parts.decode(decoder)?,
                        },
                        from_parts: crate::hir::HirStaticMethodTarget {
                            owner_ty: decoder.decode_type(owner.from_owner)?,
                            method: owner.from_parts.decode(decoder)?,
                        },
                    })
                })
                .transpose()?,
        })
    }
}

impl SerializedOwnedObjectCall {
    fn encode(
        call: &crate::hir::HirOwnedObjectCall,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            method: SerializedHirMethodCallTarget::encode(&call.method, encoder)?,
            object: encoder.encode_type(&call.object)?,
            state: encoder.encode_type(&call.state)?,
            into_owner: encoder.encode_type(&call.into_parts.owner_ty)?,
            into_parts: SerializedHirMethodCallTarget::encode(&call.into_parts.method, encoder)?,
            release_owner: encoder.encode_type(&call.release.owner_ty)?,
            release: SerializedHirMethodCallTarget::encode(&call.release.method, encoder)?,
        })
    }
    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirOwnedObjectCall, String> {
        Ok(crate::hir::HirOwnedObjectCall {
            method: self.method.decode(decoder)?,
            object: decoder.decode_type(self.object)?,
            state: decoder.decode_type(self.state)?,
            into_parts: crate::hir::HirStaticMethodTarget {
                owner_ty: decoder.decode_type(self.into_owner)?,
                method: self.into_parts.decode(decoder)?,
            },
            release: crate::hir::HirStaticMethodTarget {
                owner_ty: decoder.decode_type(self.release_owner)?,
                method: self.release.decode(decoder)?,
            },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum SerializedHirSelectedMethodTarget {
    ImplMethod {
        impl_id: crate::ids::DefId,
        method_id: crate::ids::DefId,
        selected_trait: Option<SerializedHirSelectedTraitMember>,
    },
    TraitMethod {
        trait_id: crate::ids::DefId,
        member_id: crate::ids::DefId,
        trait_args: Vec<ProductTypeId>,
        dispatch: crate::hir::HirTraitDispatchKind,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirSelectedTraitMember {
    pub trait_id: crate::ids::DefId,
    pub member_id: crate::ids::DefId,
    pub trait_args: Vec<ProductTypeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirTypeBinding {
    pub param: crate::types::GenericParamId,
    pub ty: ProductTypeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedObjectCoercion {
    target: ProductTypeId,
    evidence: SerializedObjectEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum SerializedObjectEvidence {
    Concrete {
        source: ProductTypeId,
        views: Vec<SerializedObjectView>,
    },
    Upcast {
        source: ProductTypeId,
        target: ProductTypeId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializedObjectView {
    trait_id: crate::ids::DefId,
    type_args: Vec<ProductTypeId>,
    origin: SerializedObjectOrigin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum SerializedObjectOrigin {
    Impl {
        impl_id: crate::ids::DefId,
        substitution: Vec<SerializedHirTypeBinding>,
    },
    Bound,
    Callable {
        kind: crate::types::CallableKind,
    },
    Auto {
        role: crate::language_items::LanguageItemRole,
    },
}

impl SerializedObjectCoercion {
    fn encode(
        value: &crate::hir::HirObjectCoercion,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        use crate::hir::{HirObjectEvidence as E, HirObjectWitnessOrigin as O};
        let evidence = match value
            .evidence
            .as_ref()
            .ok_or("unresolved object coercion in artifact")?
        {
            E::Upcast { source, target } => SerializedObjectEvidence::Upcast {
                source: encoder.encode_type(source)?,
                target: encoder.encode_type(target)?,
            },
            E::Concrete { source, views } => SerializedObjectEvidence::Concrete {
                source: encoder.encode_type(source)?,
                views: views
                    .iter()
                    .map(|view| {
                        Ok(SerializedObjectView {
                            trait_id: view.trait_ref.trait_id,
                            type_args: view
                                .trait_ref
                                .type_args
                                .iter()
                                .map(|ty| encoder.encode_type(ty))
                                .collect::<Result<_, _>>()?,
                            origin: match &view.origin {
                                O::Impl {
                                    impl_id,
                                    substitution,
                                } => SerializedObjectOrigin::Impl {
                                    impl_id: *impl_id,
                                    substitution: substitution
                                        .iter()
                                        .map(|binding| {
                                            Ok(SerializedHirTypeBinding {
                                                param: binding.param,
                                                ty: encoder.encode_type(&binding.ty)?,
                                            })
                                        })
                                        .collect::<Result<_, String>>()?,
                                },
                                O::Bound => SerializedObjectOrigin::Bound,
                                O::Callable { kind } => {
                                    SerializedObjectOrigin::Callable { kind: *kind }
                                }
                                O::Auto { role } => SerializedObjectOrigin::Auto { role: *role },
                            },
                        })
                    })
                    .collect::<Result<_, String>>()?,
            },
        };
        Ok(Self {
            target: encoder.encode_type(&value.target)?,
            evidence,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirObjectCoercion, String> {
        use crate::hir::{HirObjectEvidence as E, HirObjectWitnessOrigin as O};
        let evidence = match self.evidence {
            SerializedObjectEvidence::Upcast { source, target } => E::Upcast {
                source: decoder.decode_type(source)?,
                target: decoder.decode_type(target)?,
            },
            SerializedObjectEvidence::Concrete { source, views } => E::Concrete {
                source: decoder.decode_type(source)?,
                views: views
                    .into_iter()
                    .map(|view| {
                        Ok(crate::hir::HirObjectView {
                            trait_ref: crate::types::TraitBound {
                                trait_id: view.trait_id,
                                type_args: view
                                    .type_args
                                    .into_iter()
                                    .map(|ty| decoder.decode_type(ty))
                                    .collect::<Result<_, _>>()?,
                            },
                            origin: match view.origin {
                                SerializedObjectOrigin::Impl {
                                    impl_id,
                                    substitution,
                                } => O::Impl {
                                    impl_id,
                                    substitution: substitution
                                        .into_iter()
                                        .map(|binding| {
                                            Ok(crate::hir::HirTypeBinding {
                                                param: binding.param,
                                                ty: decoder.decode_type(binding.ty)?,
                                            })
                                        })
                                        .collect::<Result<_, String>>()?,
                                },
                                SerializedObjectOrigin::Bound => O::Bound,
                                SerializedObjectOrigin::Callable { kind } => O::Callable { kind },
                                SerializedObjectOrigin::Auto { role } => O::Auto { role },
                            },
                        })
                    })
                    .collect::<Result<_, String>>()?,
            },
        };
        Ok(crate::hir::HirObjectCoercion {
            target: decoder.decode_type(self.target)?,
            evidence: Some(evidence),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirStructLiteralField {
    pub name: String,
    pub value: SerializedHirExpr,
    pub field: Option<crate::hir::HirFieldLocation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirMatchArm {
    pub pattern: SerializedHirPattern,
    pub guard: Option<SerializedHirExpr>,
    pub body: SerializedHirBlock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SerializedHirStructPatternField {
    pub name: String,
    pub field: Option<crate::hir::HirFieldLocation>,
    pub pattern: SerializedHirPattern,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum SerializedHirPattern {
    Wildcard,
    Binding {
        name: String,
        local_id: crate::ids::HirLocalId,
        mutable: bool,
    },
    Literal(crate::hir::HirLiteralPattern),
    Tuple(Vec<SerializedHirPattern>),
    Struct(
        String,
        Option<crate::ids::DefId>,
        Vec<ProductTypeId>,
        Vec<SerializedHirStructPatternField>,
    ),
    Enum(
        String,
        String,
        Option<crate::hir::HirVariantLocation>,
        Vec<SerializedHirPattern>,
    ),
    Or(Vec<SerializedHirPattern>),
}

pub(super) fn products_to_artifact(
    products: &CompilerProducts,
    format_version: u32,
) -> Result<SerializedProductArtifact, String> {
    let mut encoder = ProductTypeEncoder::new();
    let products = SerializedCompilerProducts::encode(products, &mut encoder)?;
    let type_table = encoder.finish();

    Ok(SerializedProductArtifact {
        format_version,
        type_table,
        products,
    })
}

pub(super) fn artifact_to_products(
    artifact: SerializedProductArtifact,
) -> Result<CompilerProducts, String> {
    validate_portable_artifact_limits(&artifact)?;
    let mut decoder = ProductTypeDecoder::new(&artifact.type_table);
    artifact.products.decode(&mut decoder)
}

pub(super) fn validate_portable_artifact_limits(
    artifact: &SerializedProductArtifact,
) -> Result<(), String> {
    validate_portable_artifact_limits_with_initial_string_bytes(artifact, 0)
}

pub(super) fn validate_portable_artifact_limits_with_initial_string_bytes(
    artifact: &SerializedProductArtifact,
    initial_string_bytes: usize,
) -> Result<(), String> {
    if artifact.format_version != crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION {
        return Err(format!(
            "Unsupported product artifact payload format {} (expected {})",
            artifact.format_version,
            crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION
        ));
    }
    validate_serialized_product_strings(artifact, initial_string_bytes)?;
    if artifact.type_table.rows.len() > crate::products::MAX_PRODUCT_ARTIFACT_TYPE_ROWS {
        return Err(format!(
            "Product artifact type-table rows limit exceeded: declared {}, maximum {}",
            artifact.type_table.rows.len(),
            crate::products::MAX_PRODUCT_ARTIFACT_TYPE_ROWS
        ));
    }
    let mut components = Vec::new();
    let mut abi_nodes = artifact
        .products
        .interface
        .object_abi
        .trait_members
        .values()
        .map(Vec::len)
        .sum::<usize>();
    for schema in &artifact.products.interface.object_abi.schemas {
        if !matches!(
            artifact.type_table.rows.get(schema.object.0 as usize),
            Some(ProductTypeRow::Object(_))
        ) {
            return Err("object ABI schema root is not an object signature".into());
        }
        abi_nodes = abi_nodes
            .saturating_add(1)
            .saturating_add(schema.views.len())
            .saturating_add(schema.slots.len());
        for slot in &schema.slots {
            if slot.params.len() > crate::products::MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS
                || slot.trait_args.len() > crate::products::MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS
            {
                return Err("object ABI slot parameter limit exceeded".into());
            }
            components.extend(slot.trait_args.iter().copied());
        }
        for slot in &schema.erased_slots {
            abi_nodes = abi_nodes
                .saturating_add(1)
                .saturating_add(slot.signature.parameters.len())
                .saturating_add(slot.signature.layouts.len());
            components.extend(slot.trait_args.iter().copied());
        }
    }
    if abi_nodes > crate::products::MAX_PRODUCT_ARTIFACT_DECLARATIONS {
        return Err("object ABI declaration limit exceeded".into());
    }
    validate_type_table_graph_with_object_components(&artifact.type_table, &components)?;

    let interface = &artifact.products.interface;
    let declaration_count = interface
        .functions
        .len()
        .saturating_add(interface.structs.len())
        .saturating_add(interface.enums.len())
        .saturating_add(interface.type_aliases.len())
        .saturating_add(interface.traits.len())
        .saturating_add(interface.impls.len())
        .saturating_add(interface.externs.len())
        .saturating_add(artifact.products.bodies.functions.len())
        .saturating_add(artifact.products.bodies.generic_impls.len())
        .saturating_add(artifact.products.bodies.trait_default_methods.len());
    if declaration_count > crate::products::MAX_PRODUCT_ARTIFACT_DECLARATIONS {
        return Err(format!(
            "Product artifact declarations limit exceeded: declared {declaration_count}, maximum {}",
            crate::products::MAX_PRODUCT_ARTIFACT_DECLARATIONS
        ));
    }

    let check_params = |owner: &str, count: usize| -> Result<(), String> {
        if count > crate::products::MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS {
            return Err(format!(
                "Product artifact generic params limit exceeded for {owner}: declared {count}, maximum {}",
                crate::products::MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS
            ));
        }
        Ok(())
    };
    for function in interface.functions.values() {
        check_params(&function.name, function.generic_params.len())?;
    }
    for structure in interface.structs.values() {
        check_params(&structure.name, structure.generic_params.len())?;
    }
    for enumeration in interface.enums.values() {
        check_params(&enumeration.name, enumeration.generic_params.len())?;
    }
    for alias in interface.type_aliases.values() {
        check_params(&alias.name, alias.generic_params.len())?;
    }
    for trait_def in interface.traits.values() {
        check_params(&trait_def.name, trait_def.generic_params.len())?;
        for method in trait_def.methods.values() {
            check_params(&method.name, method.generic_params.len())?;
        }
        for signature in trait_def.signatures.values() {
            check_params(&signature.name, signature.generic_params.len())?;
        }
    }
    for imp in interface.impls.values() {
        check_params(&imp.type_name, imp.type_generics.len())?;
        check_params(&imp.type_name, imp.trait_generics.len())?;
        for method in imp.methods.values() {
            check_params(&method.name, method.generic_params.len())?;
        }
    }
    for function in artifact
        .products
        .bodies
        .functions
        .values()
        .chain(artifact.products.bodies.trait_default_methods.values())
    {
        check_params(&function.name, function.generic_params.len())?;
    }
    for imp in artifact.products.bodies.generic_impls.values() {
        check_params(&imp.type_name, imp.type_generics.len())?;
        check_params(&imp.type_name, imp.trait_generics.len())?;
        for method in imp.methods.values() {
            check_params(&method.name, method.generic_params.len())?;
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ProductStringLimitError(String);

impl std::fmt::Display for ProductStringLimitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ProductStringLimitError {}

impl serde::ser::Error for ProductStringLimitError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self(message.to_string())
    }
}

struct ProductStringValidator {
    total: usize,
}

fn validate_serialized_product_strings(
    artifact: &SerializedProductArtifact,
    initial_string_bytes: usize,
) -> Result<(), String> {
    use serde::Serialize;

    let mut validator = ProductStringValidator {
        total: initial_string_bytes,
    };
    artifact
        .serialize(&mut validator)
        .map_err(|error| error.to_string())
}

impl ProductStringValidator {
    fn visit_str(&mut self, value: &str) -> Result<(), ProductStringLimitError> {
        super::check_product_artifact_string(value, &mut self.total)
            .map_err(ProductStringLimitError)
    }
}

impl<'a> serde::Serializer for &'a mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_bool(self, _: bool) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_i8(self, _: i8) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_i16(self, _: i16) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_i32(self, _: i32) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_i64(self, _: i64) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_i128(self, _: i128) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_u8(self, _: u8) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_u16(self, _: u16) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_u32(self, _: u32) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_u64(self, _: u64) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_u128(self, _: u128) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_f32(self, _: f32) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_f64(self, _: f64) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_char(self, _: char) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        self.visit_str(value)
    }

    fn serialize_bytes(self, _: &[u8]) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_some<T: ?Sized + serde::Serialize>(
        self,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_newtype_struct<T: ?Sized + serde::Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + serde::Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(self)
    }

    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(self)
    }

    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(self)
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(self)
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(self)
    }

    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(self)
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(self)
    }

    fn collect_str<T: ?Sized + std::fmt::Display>(
        self,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.visit_str(&value.to_string())
    }
}

impl serde::ser::SerializeSeq for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_element<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl serde::ser::SerializeTuple for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_element<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl serde::ser::SerializeTupleStruct for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_field<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl serde::ser::SerializeTupleVariant for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_field<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl serde::ser::SerializeMap for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_key<T: ?Sized + serde::Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        key.serialize(&mut **self)
    }

    fn serialize_value<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl serde::ser::SerializeStruct for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_field<T: ?Sized + serde::Serialize>(
        &mut self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl serde::ser::SerializeStructVariant for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_field<T: ?Sized + serde::Serialize>(
        &mut self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

fn validate_type_table_graph(table: &ProductTypeTable) -> Result<(), String> {
    validate_type_table_graph_with_object_components(table, &[])
}

fn validate_type_table_graph_with_object_components(
    table: &ProductTypeTable,
    components: &[ProductTypeId],
) -> Result<(), String> {
    fn children(row: &ProductTypeRow) -> Vec<ProductTypeId> {
        match row {
            ProductTypeRow::Object(object) => object.children().collect(),
            ProductTypeRow::Slice(inner)
            | ProductTypeRow::Array(inner, _)
            | ProductTypeRow::Pointer(inner)
            | ProductTypeRow::Reference { inner, .. } => vec![*inner],
            ProductTypeRow::Tuple(elements) => elements.clone(),
            ProductTypeRow::Function {
                params,
                ret,
                captures,
                ..
            } => params
                .iter()
                .copied()
                .chain(std::iter::once(*ret))
                .chain(captures.iter().map(|capture| capture.ty))
                .collect(),
            ProductTypeRow::Struct { args, .. } | ProductTypeRow::Enum { args, .. } => args.clone(),
            ProductTypeRow::Projection { ty, trait_args, .. } => std::iter::once(*ty)
                .chain(trait_args.iter().copied())
                .collect(),
            ProductTypeRow::Apply { constructor, args } => std::iter::once(*constructor)
                .chain(args.iter().copied())
                .collect(),
            ProductTypeRow::Lambda { body, .. } => vec![*body],
            ProductTypeRow::I8
            | ProductTypeRow::ObjectSelf { .. }
            | ProductTypeRow::Witness(_)
            | ProductTypeRow::I16
            | ProductTypeRow::I32
            | ProductTypeRow::I64
            | ProductTypeRow::U8
            | ProductTypeRow::U16
            | ProductTypeRow::U32
            | ProductTypeRow::U64
            | ProductTypeRow::F32
            | ProductTypeRow::F64
            | ProductTypeRow::Bool
            | ProductTypeRow::Str
            | ProductTypeRow::Char
            | ProductTypeRow::Unit
            | ProductTypeRow::Never
            | ProductTypeRow::Generic(_)
            | ProductTypeRow::Constructor { .. }
            | ProductTypeRow::BoundVar { .. } => Vec::new(),
        }
    }

    fn visit(
        table: &ProductTypeTable,
        id: ProductTypeId,
        binders: &mut Vec<usize>,
        object_depth: usize,
        stack: &mut Vec<ProductTypeId>,
        nodes: &mut usize,
    ) -> Result<(), String> {
        if stack.len() >= crate::products::MAX_PRODUCT_ARTIFACT_TYPE_DEPTH {
            return Err(format!(
                "Product artifact type depth limit exceeded: maximum {}",
                crate::products::MAX_PRODUCT_ARTIFACT_TYPE_DEPTH
            ));
        }
        if stack.contains(&id) {
            return Err(format!("recursive product type id {}", id.0));
        }
        let row = table
            .rows
            .get(id.0 as usize)
            .ok_or_else(|| format!("unknown product type id {}", id.0))?;
        if let ProductTypeRow::ObjectSelf { depth } = row {
            if *depth as usize >= object_depth {
                return Err(format!("out-of-scope product object Self depth {depth}"));
            }
        }
        *nodes = nodes.saturating_add(1);
        if *nodes > crate::products::MAX_PRODUCT_ARTIFACT_NORMALIZATION_NODES {
            return Err(format!(
                "Product artifact normalization output nodes limit exceeded: maximum {}",
                crate::products::MAX_PRODUCT_ARTIFACT_NORMALIZATION_NODES
            ));
        }
        if let ProductTypeRow::BoundVar { depth, index, .. } = row {
            let depth = *depth as usize;
            let Some(scope) = binders.iter().rev().nth(depth) else {
                return Err(format!("out-of-scope product bound variable depth {depth}"));
            };
            if *index as usize >= *scope {
                return Err(format!(
                    "out-of-scope product bound variable index {index} for binder width {scope}"
                ));
            }
            return Ok(());
        }
        stack.push(id);
        if let ProductTypeRow::Lambda { params, body } = row {
            if params.len() > crate::products::MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS {
                return Err(format!(
                    "Product artifact lambda params limit exceeded: declared {}, maximum {}",
                    params.len(),
                    crate::products::MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS
                ));
            }
            binders.push(params.len());
            visit(table, *body, binders, object_depth, stack, nodes)?;
            binders.pop();
        } else {
            for child in children(row) {
                let child_object_depth =
                    object_depth + usize::from(matches!(row, ProductTypeRow::Object(_)));
                visit(table, child, binders, child_object_depth, stack, nodes)?;
            }
        }
        stack.pop();
        Ok(())
    }

    let mut referenced = vec![false; table.rows.len()];
    for row in &table.rows {
        if let ProductTypeRow::Object(object) = row {
            if object
                .guarantees
                .len()
                .saturating_add(object.bindings.len())
                > crate::products::MAX_PRODUCT_ARTIFACT_DECLARATIONS
            {
                return Err("Product artifact object evidence limit exceeded".to_string());
            }
            for bound in std::iter::once(&object.principal)
                .chain(&object.guarantees)
                .chain(object.bindings.iter().map(|binding| &binding.trait_ref))
            {
                if bound.args.len() > crate::products::MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS {
                    return Err("Product artifact object trait argument limit exceeded".to_string());
                }
            }
        }
        for child in children(row) {
            let Some(slot) = referenced.get_mut(child.0 as usize) else {
                return Err(format!("unknown product type id {}", child.0));
            };
            *slot = true;
        }
    }
    let mut nodes = 0usize;
    // Only schema trait-argument positions open this implicit object binder.
    // Ordinary declaration roots are still validated independently by decoding.
    for component in components {
        let referenced = referenced
            .get_mut(component.0 as usize)
            .ok_or_else(|| format!("unknown product type id {}", component.0))?;
        *referenced = true;
        visit(
            table,
            *component,
            &mut Vec::new(),
            1,
            &mut Vec::new(),
            &mut nodes,
        )?;
    }
    for (index, is_referenced) in referenced.iter().enumerate() {
        if !is_referenced {
            visit(
                table,
                ProductTypeId(index as u32),
                &mut Vec::new(),
                0,
                &mut Vec::new(),
                &mut nodes,
            )?;
        }
    }
    let mut state = vec![0; table.rows.len()];
    for index in 0..table.rows.len() {
        if state[index] != 0 {
            continue;
        }
        let mut pending = vec![(ProductTypeId(index as u32), false)];
        while let Some((id, exiting)) = pending.pop() {
            let index = id.0 as usize;
            if exiting {
                state[index] = 2;
                continue;
            }
            match state[index] {
                1 => return Err(format!("recursive product type id {}", id.0)),
                2 => continue,
                _ => {}
            }
            state[index] = 1;
            pending.push((id, true));
            for child in children(&table.rows[index]).into_iter().rev() {
                if state[child.0 as usize] == 1 {
                    return Err(format!("recursive product type id {}", child.0));
                }
                if state[child.0 as usize] == 0 {
                    pending.push((child, false));
                }
            }
        }
    }
    Ok(())
}

impl SerializedCompilerProducts {
    fn encode(
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

    fn decode(self, decoder: &mut ProductTypeDecoder<'_>) -> Result<CompilerProducts, String> {
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
            object_abi: object_abi::SerializedObjectAbi::encode(&interface.object_abi, encoder)?,
        })
    }

    fn decode(self, decoder: &mut ProductTypeDecoder<'_>) -> Result<ProductInterface, String> {
        Ok(ProductInterface {
            object_abi: self.object_abi.decode(decoder)?,
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

fn encode_map<T, U>(
    map: &BTreeMap<ProductDefId, T>,
    encoder: &mut ProductTypeEncoder,
    encode: fn(&T, &mut ProductTypeEncoder) -> Result<U, String>,
) -> Result<BTreeMap<ProductDefId, U>, String> {
    map.iter()
        .map(|(id, value)| Ok((*id, encode(value, encoder)?)))
        .collect()
}

fn decode_map<T, U>(
    map: BTreeMap<ProductDefId, T>,
    decoder: &mut ProductTypeDecoder<'_>,
    decode: fn(T, &mut ProductTypeDecoder<'_>) -> Result<U, String>,
) -> Result<BTreeMap<ProductDefId, U>, String> {
    map.into_iter()
        .map(|(id, value)| Ok((id, decode(value, decoder)?)))
        .collect()
}

impl SerializedHirFunction {
    fn encode(
        value: &crate::hir::AcceptedHirFunction,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params),
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

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::AcceptedHirFunction, String> {
        Ok(crate::hir::HirFunctionFor::<crate::hir::AcceptedHir> {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params),
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

impl SerializedProductFunctionInterface {
    fn encode(
        value: &ProductFunctionInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params),
            generic_bounds: encode_generic_bounds(&value.generic_bounds, encoder)?,
            params: encoder.encode_types(&value.params)?,
            ret_type: encoder.encode_type(&value.ret_type)?,
            is_curried: value.is_curried,
            is_method: value.is_method,
            self_receiver: value.self_receiver,
            is_unsafe: value.is_unsafe,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductFunctionInterface, String> {
        Ok(ProductFunctionInterface {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params),
            generic_bounds: decode_generic_bounds(self.generic_bounds, decoder)?,
            params: decoder.decode_types(&self.params)?,
            ret_type: decoder.decode_type(self.ret_type)?,
            is_curried: self.is_curried,
            is_method: self.is_method,
            self_receiver: self.self_receiver,
            is_unsafe: self.is_unsafe,
        })
    }
}

impl SerializedProductStructInterface {
    fn encode(
        value: &ProductStructInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params),
            fields: value
                .fields
                .iter()
                .map(|field| SerializedHirField::encode_interface(field, encoder))
                .collect::<Result<_, _>>()?,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductStructInterface, String> {
        Ok(ProductStructInterface {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params),
            fields: self
                .fields
                .into_iter()
                .map(|field| field.decode_interface(decoder))
                .collect::<Result<_, _>>()?,
        })
    }
}

impl SerializedProductTypeAliasInterface {
    fn encode(
        value: &ProductTypeAliasInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params),
            ty: encoder.encode_type(&value.ty)?,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductTypeAliasInterface, String> {
        Ok(ProductTypeAliasInterface {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params),
            ty: decoder.decode_type(self.ty)?,
        })
    }
}

impl SerializedProductEnumInterface {
    fn encode(
        value: &ProductEnumInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params),
            variants: value
                .variants
                .iter()
                .map(|variant| SerializedProductEnumVariantInterface::encode(variant, encoder))
                .collect::<Result<_, _>>()?,
        })
    }

    fn decode(self, decoder: &mut ProductTypeDecoder<'_>) -> Result<ProductEnumInterface, String> {
        Ok(ProductEnumInterface {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params),
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
    fn encode(
        value: &ProductTraitInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params),
            target: value
                .target
                .as_ref()
                .map(|target| encode_generic_param_decls(std::slice::from_ref(target)).remove(0)),
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

    fn decode(self, decoder: &mut ProductTypeDecoder<'_>) -> Result<ProductTraitInterface, String> {
        Ok(ProductTraitInterface {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params),
            target: self
                .target
                .map(|target| decode_generic_param_decls(vec![target]).remove(0)),
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
    fn encode(
        value: &ProductImplInterface,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            owner: value.owner.clone(),
            type_name: value.type_name.clone(),
            type_generics: encode_generic_param_decls(&value.type_generics),
            receiver_pattern: SerializedHirImplReceiverPattern::encode(
                &value.receiver_pattern,
                encoder,
            )?,
            trait_name: value.trait_name.clone(),
            trait_id: value.trait_id,
            trait_generics: encode_generic_param_decls(&value.trait_generics),
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

    fn decode(self, decoder: &mut ProductTypeDecoder<'_>) -> Result<ProductImplInterface, String> {
        Ok(ProductImplInterface {
            id: self.id,
            owner: self.owner,
            type_name: self.type_name,
            type_generics: decode_generic_param_decls(self.type_generics),
            receiver_pattern: self.receiver_pattern.decode(decoder)?,
            trait_name: self.trait_name,
            trait_id: self.trait_id,
            trait_generics: decode_generic_param_decls(self.trait_generics),
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
    fn encode(
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

    fn decode(
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

impl SerializedHirParam {
    fn encode(
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

    fn decode(self, decoder: &mut ProductTypeDecoder<'_>) -> Result<crate::hir::HirParam, String> {
        Ok(crate::hir::HirParam {
            name: self.name,
            local_id: self.local_id,
            ty: decoder.decode_type(self.ty)?,
            mutable: self.mutable,
            is_ref: self.is_ref,
        })
    }
}

impl SerializedHirStruct {
    fn encode(
        value: &crate::hir::HirStruct,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params),
            fields: value
                .fields
                .iter()
                .map(|field| SerializedHirField::encode(field, encoder))
                .collect::<Result<_, _>>()?,
        })
    }

    fn decode(self, decoder: &mut ProductTypeDecoder<'_>) -> Result<crate::hir::HirStruct, String> {
        Ok(crate::hir::HirStruct {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params),
            fields: self
                .fields
                .into_iter()
                .map(|field| field.decode(decoder))
                .collect::<Result<_, _>>()?,
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

    fn encode_interface(
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

    fn decode_interface(
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

impl SerializedHirEnum {
    fn encode(
        value: &crate::hir::HirEnum,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params),
            variants: value
                .variants
                .iter()
                .map(|variant| SerializedHirVariant::encode(variant, encoder))
                .collect::<Result<_, _>>()?,
        })
    }

    fn decode(self, decoder: &mut ProductTypeDecoder<'_>) -> Result<crate::hir::HirEnum, String> {
        Ok(crate::hir::HirEnum {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params),
            variants: self
                .variants
                .into_iter()
                .map(|variant| variant.decode(decoder))
                .collect::<Result<_, _>>()?,
        })
    }
}

impl SerializedHirVariant {
    fn encode(
        value: &crate::hir::HirVariant,
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
    ) -> Result<crate::hir::HirVariant, String> {
        Ok(crate::hir::HirVariant {
            id: self.id,
            name: self.name,
            fields: self.fields.decode(decoder)?,
        })
    }
}

impl SerializedHirVariantFields {
    fn encode(
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

    fn decode(
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
    fn encode(
        value: &crate::hir::HirFunctionSig,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            generic_params: encode_generic_param_decls(&value.generic_params),
            params: encoder.encode_types(&value.params)?,
            ret: encoder.encode_type(&value.ret)?,
            generic_bounds: encode_generic_bounds(&value.generic_bounds, encoder)?,
            self_receiver: value.self_receiver,
            is_unsafe: value.is_unsafe,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirFunctionSig, String> {
        Ok(crate::hir::HirFunctionSig {
            id: self.id,
            name: self.name,
            generic_params: decode_generic_param_decls(self.generic_params),
            params: decoder.decode_types(&self.params)?,
            ret: decoder.decode_type(self.ret)?,
            generic_bounds: decode_generic_bounds(self.generic_bounds, decoder)?,
            self_receiver: self.self_receiver,
            is_unsafe: self.is_unsafe,
        })
    }
}

impl SerializedHirImpl {
    fn encode(
        value: &crate::hir::AcceptedHirImpl,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: value.id,
            owner: value.owner.clone(),
            type_name: value.type_name.clone(),
            type_generics: encode_generic_param_decls(&value.type_generics),
            receiver_pattern: SerializedHirImplReceiverPattern::encode(
                &value.receiver_pattern,
                encoder,
            )?,
            trait_name: value.trait_name.clone(),
            trait_id: value.trait_id,
            trait_generics: encode_generic_param_decls(&value.trait_generics),
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

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::AcceptedHirImpl, String> {
        Ok(crate::hir::HirImplFor::<crate::hir::AcceptedHir> {
            id: self.id,
            owner: self.owner,
            type_name: self.type_name,
            type_generics: decode_generic_param_decls(self.type_generics),
            receiver_pattern: self.receiver_pattern.decode(decoder)?,
            trait_name: self.trait_name,
            trait_id: self.trait_id,
            trait_generics: decode_generic_param_decls(self.trait_generics),
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

    fn encode_interface(
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

    fn decode_interface(
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
    fn encode(
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

    fn decode(
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

impl SerializedHirExtern {
    fn encode(
        value: &crate::hir::HirExtern,
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

    fn decode(self, decoder: &mut ProductTypeDecoder<'_>) -> Result<crate::hir::HirExtern, String> {
        Ok(crate::hir::HirExtern {
            id: self.id,
            name: self.name,
            params: decoder.decode_types(&self.params)?,
            ret: decoder.decode_type(self.ret)?,
            variadic: self.variadic,
            is_unsafe: self.is_unsafe,
        })
    }
}

impl SerializedHirBlock {
    fn encode(
        value: &crate::hir::AcceptedHirBlock,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            stmts: value
                .stmts
                .iter()
                .map(|stmt| SerializedHirStmt::encode(stmt, encoder))
                .collect::<Result<_, _>>()?,
            ty: encoder.encode_type(&value.ty)?,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::AcceptedHirBlock, String> {
        Ok(crate::hir::HirBlockFor::<crate::hir::AcceptedHir> {
            stmts: self
                .stmts
                .into_iter()
                .map(|stmt| stmt.decode(decoder))
                .collect::<Result<_, _>>()?,
            ty: decoder.decode_type(self.ty)?,
        })
    }
}

impl SerializedHirStmt {
    fn encode(
        value: &crate::hir::HirStmtFor<crate::hir::AcceptedHir>,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(match value {
            crate::hir::HirStmtFor::Let {
                name,
                local_id,
                ty,
                value,
                mutable,
            } => Self::Let {
                name: name.clone(),
                local_id: *local_id,
                ty: encoder.encode_type(ty)?,
                value: SerializedHirExpr::encode(value, encoder)?,
                mutable: *mutable,
            },
            crate::hir::HirStmtFor::Expr(expr) => {
                Self::Expr(SerializedHirExpr::encode(expr, encoder)?)
            }
            crate::hir::HirStmtFor::Return(expr) => Self::Return(
                expr.as_ref()
                    .map(|expr| SerializedHirExpr::encode(expr, encoder))
                    .transpose()?,
            ),
            crate::hir::HirStmtFor::Break(expr) => Self::Break(
                expr.as_ref()
                    .map(|expr| SerializedHirExpr::encode(expr, encoder))
                    .transpose()?,
            ),
            crate::hir::HirStmtFor::Continue => Self::Continue,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirStmtFor<crate::hir::AcceptedHir>, String> {
        Ok(match self {
            Self::Let {
                name,
                local_id,
                ty,
                value,
                mutable,
            } => crate::hir::HirStmtFor::Let {
                name,
                local_id,
                ty: decoder.decode_type(ty)?,
                value: value.decode(decoder)?,
                mutable,
            },
            Self::Expr(expr) => crate::hir::HirStmtFor::Expr(expr.decode(decoder)?),
            Self::Return(expr) => {
                crate::hir::HirStmtFor::Return(expr.map(|expr| expr.decode(decoder)).transpose()?)
            }
            Self::Break(expr) => {
                crate::hir::HirStmtFor::Break(expr.map(|expr| expr.decode(decoder)).transpose()?)
            }
            Self::Continue => crate::hir::HirStmtFor::Continue,
        })
    }
}

impl SerializedHirExpr {
    fn encode(
        value: &crate::hir::AcceptedHirExpr,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            kind: SerializedHirExprKind::encode(&value.kind, encoder)?,
            ty: encoder.encode_type(&value.ty)?,
            span: value.span.clone(),
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::AcceptedHirExpr, String> {
        Ok(crate::hir::HirExprFor::<crate::hir::AcceptedHir> {
            kind: self.kind.decode(decoder)?,
            ty: decoder.decode_type(self.ty)?,
            span: self.span,
        })
    }
}

impl SerializedHirExprKind {
    fn encode(
        value: &crate::hir::HirExprKindFor<crate::hir::AcceptedHir>,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(match value {
            crate::hir::HirExprKindFor::OwnedObjectCall { owner, args, call } => {
                Self::OwnedObjectCall {
                    owner: Box::new(SerializedHirExpr::encode(owner, encoder)?),
                    args: encode_exprs(args, encoder)?,
                    call: SerializedOwnedObjectCall::encode(call, encoder)?,
                }
            }
            crate::hir::HirExprKindFor::Open {
                source,
                binding,
                body,
            } => Self::Open {
                source: Box::new(SerializedHirExpr::encode(source, encoder)?),
                binding: SerializedOpenBinding::encode(binding, encoder)?,
                body: SerializedHirBlock::encode(body, encoder)?,
            },
            crate::hir::HirExprKindFor::IntLiteral(value) => Self::IntLiteral(*value),
            crate::hir::HirExprKindFor::FloatLiteral(value) => Self::FloatLiteral(*value),
            crate::hir::HirExprKindFor::BoolLiteral(value) => Self::BoolLiteral(*value),
            crate::hir::HirExprKindFor::StringLiteral(value) => Self::StringLiteral(value.clone()),
            crate::hir::HirExprKindFor::CharLiteral(value) => Self::CharLiteral(*value),
            crate::hir::HirExprKindFor::ArrayLiteral(elems) => {
                Self::ArrayLiteral(encode_exprs(elems, encoder)?)
            }
            crate::hir::HirExprKindFor::ArrayRepeat(value, len) => {
                Self::ArrayRepeat(Box::new(SerializedHirExpr::encode(value, encoder)?), *len)
            }
            crate::hir::HirExprKindFor::TupleLiteral(elems) => {
                Self::TupleLiteral(encode_exprs(elems, encoder)?)
            }
            crate::hir::HirExprKindFor::Unit => Self::Unit,
            crate::hir::HirExprKindFor::Var(name) => Self::Var(name.clone()),
            crate::hir::HirExprKindFor::ResolvedVar(reference) => {
                Self::ResolvedVar(SerializedHirVarRef::from_hir(reference))
            }
            crate::hir::HirExprKindFor::FieldAccess(inner, name, location) => Self::FieldAccess(
                Box::new(SerializedHirExpr::encode(inner, encoder)?),
                name.clone(),
                location.clone(),
            ),
            crate::hir::HirExprKindFor::TupleIndex(inner, index) => {
                Self::TupleIndex(Box::new(SerializedHirExpr::encode(inner, encoder)?), *index)
            }
            crate::hir::HirExprKindFor::BinOp(op, lhs, rhs) => Self::BinOp(
                *op,
                Box::new(SerializedHirExpr::encode(lhs, encoder)?),
                Box::new(SerializedHirExpr::encode(rhs, encoder)?),
            ),
            crate::hir::HirExprKindFor::UnaryOp(op, inner) => {
                Self::UnaryOp(*op, Box::new(SerializedHirExpr::encode(inner, encoder)?))
            }
            crate::hir::HirExprKindFor::Call(func, args, target) => Self::Call(
                Box::new(SerializedHirExpr::encode(func, encoder)?),
                encode_exprs(args, encoder)?,
                target
                    .as_ref()
                    .map(|target| SerializedHirCallTarget::encode(target, encoder))
                    .transpose()?,
            ),
            crate::hir::HirExprKindFor::MethodCall(func, name, args, receiver, target) => {
                Self::MethodCall(
                    Box::new(SerializedHirExpr::encode(func, encoder)?),
                    name.clone(),
                    encode_exprs(args, encoder)?,
                    *receiver,
                    Some(SerializedHirMethodCallTarget::encode(target, encoder)?),
                )
            }
            crate::hir::HirExprKindFor::Try {
                expr,
                branch_method,
                branch_target: _,
                branch_self_receiver,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                control_flow_enum,
                break_variant,
                continue_variant,
            } => Self::Try {
                expr: Box::new(SerializedHirExpr::encode(expr, encoder)?),
                branch_method: Some(SerializedHirMethodCallTarget::encode(
                    branch_method,
                    encoder,
                )?),
                branch_self_receiver: *branch_self_receiver,
                from_residual_target: Some(SerializedHirCallTarget::encode(
                    from_residual_target,
                    encoder,
                )?),
                output_ty: encoder.encode_type(output_ty)?,
                residual_ty: encoder.encode_type(residual_ty)?,
                return_ty: encoder.encode_type(return_ty)?,
                control_flow_enum: *control_flow_enum,
                break_variant: break_variant.clone(),
                continue_variant: continue_variant.clone(),
            },
            crate::hir::HirExprKindFor::StructLiteral(name, struct_id, fields) => {
                Self::StructLiteral(
                    name.clone(),
                    *struct_id,
                    fields
                        .iter()
                        .map(|field| SerializedHirStructLiteralField::encode(field, encoder))
                        .collect::<Result<_, _>>()?,
                )
            }
            crate::hir::HirExprKindFor::EnumVariant(enum_name, variant_name, args, location) => {
                Self::EnumVariant(
                    enum_name.clone(),
                    variant_name.clone(),
                    encode_exprs(args, encoder)?,
                    location.clone(),
                )
            }
            crate::hir::HirExprKindFor::If {
                condition,
                then_branch,
                else_branch,
            } => Self::If {
                condition: Box::new(SerializedHirExpr::encode(condition, encoder)?),
                then_branch: SerializedHirBlock::encode(then_branch, encoder)?,
                else_branch: else_branch
                    .as_ref()
                    .map(|block| SerializedHirBlock::encode(block, encoder))
                    .transpose()?,
            },
            crate::hir::HirExprKindFor::Match { scrutinee, arms } => Self::Match {
                scrutinee: Box::new(SerializedHirExpr::encode(scrutinee, encoder)?),
                arms: arms
                    .iter()
                    .map(|arm| SerializedHirMatchArm::encode(arm, encoder))
                    .collect::<Result<_, _>>()?,
            },
            crate::hir::HirExprKindFor::While { condition, body } => Self::While {
                condition: Box::new(SerializedHirExpr::encode(condition, encoder)?),
                body: SerializedHirBlock::encode(body, encoder)?,
            },
            crate::hir::HirExprKindFor::For {
                var,
                local_id,
                iter,
                body,
            } => Self::For {
                var: var.clone(),
                local_id: *local_id,
                iter: Box::new(SerializedHirExpr::encode(iter, encoder)?),
                body: SerializedHirBlock::encode(body, encoder)?,
            },
            crate::hir::HirExprKindFor::Loop(body) => {
                Self::Loop(SerializedHirBlock::encode(body, encoder)?)
            }
            crate::hir::HirExprKindFor::Block(body) => {
                Self::Block(SerializedHirBlock::encode(body, encoder)?)
            }
            crate::hir::HirExprKindFor::UnsafeBlock(body) => {
                Self::Block(SerializedHirBlock::encode(body, encoder)?)
            }
            crate::hir::HirExprKindFor::Lambda {
                params,
                body,
                captures,
            } => Self::Lambda {
                params: params
                    .iter()
                    .map(|param| SerializedHirParam::encode(param, encoder))
                    .collect::<Result<_, _>>()?,
                body: SerializedHirBlock::encode(body, encoder)?,
                captures: captures
                    .iter()
                    .map(|capture| SerializedHirClosureCapture::encode(capture, encoder))
                    .collect::<Result<_, _>>()?,
            },
            crate::hir::HirExprKindFor::Ref(mutable, inner) => Self::Ref(
                *mutable,
                Box::new(SerializedHirExpr::encode(inner, encoder)?),
            ),
            crate::hir::HirExprKindFor::Deref(inner) => {
                Self::Deref(Box::new(SerializedHirExpr::encode(inner, encoder)?))
            }
            crate::hir::HirExprKindFor::Cast(inner, ty) => Self::Cast(
                Box::new(SerializedHirExpr::encode(inner, encoder)?),
                encoder.encode_type(ty)?,
            ),
            crate::hir::HirExprKindFor::ObjectCoercion(inner, coercion) => Self::ObjectCoercion(
                Box::new(SerializedHirExpr::encode(inner, encoder)?),
                SerializedObjectCoercion::encode(coercion, encoder)?,
            ),
            crate::hir::HirExprKindFor::Assign(lhs, rhs) => Self::Assign(
                Box::new(SerializedHirExpr::encode(lhs, encoder)?),
                Box::new(SerializedHirExpr::encode(rhs, encoder)?),
            ),
            crate::hir::HirExprKindFor::Intrinsic { name, args } => Self::Intrinsic {
                name: name.clone(),
                args: encode_exprs(args, encoder)?,
            },
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirExprKindFor<crate::hir::AcceptedHir>, String> {
        Ok(match self {
            Self::OwnedObjectCall { owner, args, call } => {
                crate::hir::HirExprKindFor::OwnedObjectCall {
                    owner: Box::new(owner.decode(decoder)?),
                    args: decode_exprs(args, decoder)?,
                    call: call.decode(decoder)?,
                }
            }
            Self::Open {
                source,
                binding,
                body,
            } => crate::hir::HirExprKindFor::Open {
                source: Box::new(source.decode(decoder)?),
                binding: binding.decode(decoder)?,
                body: body.decode(decoder)?,
            },
            Self::IntLiteral(value) => crate::hir::HirExprKindFor::IntLiteral(value),
            Self::FloatLiteral(value) => crate::hir::HirExprKindFor::FloatLiteral(value),
            Self::BoolLiteral(value) => crate::hir::HirExprKindFor::BoolLiteral(value),
            Self::StringLiteral(value) => crate::hir::HirExprKindFor::StringLiteral(value),
            Self::CharLiteral(value) => crate::hir::HirExprKindFor::CharLiteral(value),
            Self::ArrayLiteral(elems) => {
                crate::hir::HirExprKindFor::ArrayLiteral(decode_exprs(elems, decoder)?)
            }
            Self::ArrayRepeat(value, len) => {
                crate::hir::HirExprKindFor::ArrayRepeat(Box::new(value.decode(decoder)?), len)
            }
            Self::TupleLiteral(elems) => {
                crate::hir::HirExprKindFor::TupleLiteral(decode_exprs(elems, decoder)?)
            }
            Self::Unit => crate::hir::HirExprKindFor::Unit,
            Self::Var(name) => crate::hir::HirExprKindFor::Var(name),
            Self::ResolvedVar(reference) => {
                crate::hir::HirExprKindFor::ResolvedVar(reference.into_hir())
            }
            Self::FieldAccess(inner, name, location) => crate::hir::HirExprKindFor::FieldAccess(
                Box::new(inner.decode(decoder)?),
                name,
                location,
            ),
            Self::TupleIndex(inner, index) => {
                crate::hir::HirExprKindFor::TupleIndex(Box::new(inner.decode(decoder)?), index)
            }
            Self::BinOp(op, lhs, rhs) => crate::hir::HirExprKindFor::BinOp(
                op,
                Box::new(lhs.decode(decoder)?),
                Box::new(rhs.decode(decoder)?),
            ),
            Self::UnaryOp(op, inner) => {
                crate::hir::HirExprKindFor::UnaryOp(op, Box::new(inner.decode(decoder)?))
            }
            Self::Call(func, args, target) => crate::hir::HirExprKindFor::Call(
                Box::new(func.decode(decoder)?),
                decode_exprs(args, decoder)?,
                target.map(|target| target.decode(decoder)).transpose()?,
            ),
            Self::MethodCall(func, name, args, receiver, target) => {
                crate::hir::HirExprKindFor::MethodCall(
                    Box::new(func.decode(decoder)?),
                    name,
                    decode_exprs(args, decoder)?,
                    receiver,
                    target
                        .map(|target| target.decode(decoder))
                        .transpose()?
                        .ok_or_else(|| {
                            "serialized accepted method call lacks authority".to_string()
                        })?,
                )
            }
            Self::Try {
                expr,
                branch_method,
                branch_self_receiver,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                control_flow_enum,
                break_variant,
                continue_variant,
            } => crate::hir::HirExprKindFor::Try {
                expr: Box::new(expr.decode(decoder)?),
                branch_method: branch_method
                    .map(|target| target.decode(decoder))
                    .transpose()?
                    .ok_or_else(|| "serialized accepted Try lacks branch authority".to_string())?,
                branch_target: None,
                branch_self_receiver,
                from_residual_target: from_residual_target
                    .map(|target| target.decode(decoder))
                    .transpose()?
                    .ok_or_else(|| {
                        "serialized accepted Try lacks residual authority".to_string()
                    })?,
                output_ty: decoder.decode_type(output_ty)?,
                residual_ty: decoder.decode_type(residual_ty)?,
                return_ty: decoder.decode_type(return_ty)?,
                control_flow_enum,
                break_variant,
                continue_variant,
            },
            Self::StructLiteral(name, struct_id, fields) => {
                crate::hir::HirExprKindFor::StructLiteral(
                    name,
                    struct_id,
                    fields
                        .into_iter()
                        .map(|field| field.decode(decoder))
                        .collect::<Result<_, _>>()?,
                )
            }
            Self::EnumVariant(enum_name, variant_name, args, location) => {
                crate::hir::HirExprKindFor::EnumVariant(
                    enum_name,
                    variant_name,
                    decode_exprs(args, decoder)?,
                    location,
                )
            }
            Self::If {
                condition,
                then_branch,
                else_branch,
            } => crate::hir::HirExprKindFor::If {
                condition: Box::new(condition.decode(decoder)?),
                then_branch: then_branch.decode(decoder)?,
                else_branch: else_branch.map(|block| block.decode(decoder)).transpose()?,
            },
            Self::Match { scrutinee, arms } => crate::hir::HirExprKindFor::Match {
                scrutinee: Box::new(scrutinee.decode(decoder)?),
                arms: arms
                    .into_iter()
                    .map(|arm| arm.decode(decoder))
                    .collect::<Result<_, _>>()?,
            },
            Self::While { condition, body } => crate::hir::HirExprKindFor::While {
                condition: Box::new(condition.decode(decoder)?),
                body: body.decode(decoder)?,
            },
            Self::For {
                var,
                local_id,
                iter,
                body,
            } => crate::hir::HirExprKindFor::For {
                var,
                local_id,
                iter: Box::new(iter.decode(decoder)?),
                body: body.decode(decoder)?,
            },
            Self::Loop(body) => crate::hir::HirExprKindFor::Loop(body.decode(decoder)?),
            Self::Block(body) => crate::hir::HirExprKindFor::Block(body.decode(decoder)?),
            Self::Lambda {
                params,
                body,
                captures,
            } => crate::hir::HirExprKindFor::Lambda {
                params: params
                    .into_iter()
                    .map(|param| param.decode(decoder))
                    .collect::<Result<_, _>>()?,
                body: body.decode(decoder)?,
                captures: captures
                    .into_iter()
                    .map(|capture| capture.decode(decoder))
                    .collect::<Result<_, _>>()?,
            },
            Self::Ref(mutable, inner) => {
                crate::hir::HirExprKindFor::Ref(mutable, Box::new(inner.decode(decoder)?))
            }
            Self::Deref(inner) => {
                crate::hir::HirExprKindFor::Deref(Box::new(inner.decode(decoder)?))
            }
            Self::Cast(inner, ty) => crate::hir::HirExprKindFor::Cast(
                Box::new(inner.decode(decoder)?),
                decoder.decode_type(ty)?,
            ),
            Self::ObjectCoercion(inner, coercion) => crate::hir::HirExprKindFor::ObjectCoercion(
                Box::new(inner.decode(decoder)?),
                coercion.decode(decoder)?,
            ),
            Self::Assign(lhs, rhs) => crate::hir::HirExprKindFor::Assign(
                Box::new(lhs.decode(decoder)?),
                Box::new(rhs.decode(decoder)?),
            ),
            Self::Intrinsic { name, args } => crate::hir::HirExprKindFor::Intrinsic {
                name,
                args: decode_exprs(args, decoder)?,
            },
        })
    }
}

impl SerializedHirClosureCapture {
    fn encode(
        value: &crate::hir::HirClosureCapture,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            name: value.name.clone(),
            local_id: value.local_id,
            kind: value.kind,
            mutable: value.mutable,
            ty: encoder.encode_type(&value.ty)?,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirClosureCapture, String> {
        Ok(crate::hir::HirClosureCapture {
            name: self.name,
            local_id: self.local_id,
            kind: self.kind,
            mutable: self.mutable,
            ty: decoder.decode_type(self.ty)?,
        })
    }
}

impl SerializedHirMethodCallTarget {
    fn encode(
        value: &crate::hir::HirMethodCallTarget,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        let target = match &value.target {
            crate::hir::HirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait,
            } => SerializedHirSelectedMethodTarget::ImplMethod {
                impl_id: *impl_id,
                method_id: *method_id,
                selected_trait: selected_trait
                    .as_ref()
                    .map(|selected| {
                        Ok::<_, String>(SerializedHirSelectedTraitMember {
                            trait_id: selected.trait_id,
                            member_id: selected.member_id,
                            trait_args: encoder.encode_types(&selected.trait_args)?,
                        })
                    })
                    .transpose()?,
            },
            crate::hir::HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                trait_args,
                dispatch,
            } => SerializedHirSelectedMethodTarget::TraitMethod {
                trait_id: *trait_id,
                member_id: *member_id,
                trait_args: encoder.encode_types(trait_args)?,
                dispatch: *dispatch,
            },
        };
        let mut encode_bindings = |bindings: &[crate::hir::HirTypeBinding]| {
            bindings
                .iter()
                .map(|binding| {
                    Ok(SerializedHirTypeBinding {
                        param: binding.param,
                        ty: encoder.encode_type(&binding.ty)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        };
        Ok(Self {
            target,
            owner_substitution: encode_bindings(&value.owner_substitution)?,
            method_substitution: encode_bindings(&value.method_substitution)?,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirMethodCallTarget, String> {
        let target = match self.target {
            SerializedHirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait,
            } => crate::hir::HirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait: selected_trait
                    .map(|selected| {
                        Ok::<_, String>(crate::hir::HirSelectedTraitMember {
                            trait_id: selected.trait_id,
                            member_id: selected.member_id,
                            trait_args: decoder.decode_types(&selected.trait_args)?,
                        })
                    })
                    .transpose()?,
            },
            SerializedHirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                trait_args,
                dispatch,
            } => crate::hir::HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                trait_args: decoder.decode_types(&trait_args)?,
                dispatch,
            },
        };
        let mut decode_bindings = |bindings: Vec<SerializedHirTypeBinding>| {
            bindings
                .into_iter()
                .map(|binding| {
                    Ok(crate::hir::HirTypeBinding {
                        param: binding.param,
                        ty: decoder.decode_type(binding.ty)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        };
        Ok(crate::hir::HirMethodCallTarget {
            target,
            owner_substitution: decode_bindings(self.owner_substitution)?,
            method_substitution: decode_bindings(self.method_substitution)?,
        })
    }
}

impl SerializedHirVarRef {
    fn from_hir(reference: &crate::hir::HirVarRef) -> Self {
        let target = match reference.target {
            crate::hir::HirVarTarget::Function(id) => SerializedHirVarTarget::Function(id),
            crate::hir::HirVarTarget::Extern(id) => SerializedHirVarTarget::Extern(id),
            crate::hir::HirVarTarget::Local(id) => SerializedHirVarTarget::Local(id),
            crate::hir::HirVarTarget::Instance(id) => {
                panic!("pre-mono products cannot serialize local instance variable {id:?}")
            }
        };
        Self {
            name: reference.name.clone(),
            target,
        }
    }

    fn into_hir(self) -> crate::hir::HirVarRef {
        let target = match self.target {
            SerializedHirVarTarget::Function(id) => crate::hir::HirVarTarget::Function(id),
            SerializedHirVarTarget::Extern(id) => crate::hir::HirVarTarget::Extern(id),
            SerializedHirVarTarget::Local(id) => crate::hir::HirVarTarget::Local(id),
        };
        crate::hir::HirVarRef {
            name: self.name,
            target,
        }
    }
}

impl SerializedHirCallTarget {
    fn encode(
        value: &crate::hir::HirCallTarget,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(match value {
            crate::hir::HirCallTarget::Function(id) => Self::Function(*id),
            crate::hir::HirCallTarget::Extern(id) => Self::Extern(*id),
            crate::hir::HirCallTarget::Instance(id) => {
                panic!("pre-mono products cannot serialize local instance target {id:?}")
            }
            crate::hir::HirCallTarget::Local(id) => Self::Local(*id),
            crate::hir::HirCallTarget::Intrinsic(name) => Self::Intrinsic(name.clone()),
            crate::hir::HirCallTarget::StaticMethod(target) => Self::StaticMethod {
                owner_ty: encoder.encode_type(&target.owner_ty)?,
                method: SerializedHirMethodCallTarget::encode(&target.method, encoder)?,
            },
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirCallTarget, String> {
        Ok(match self {
            Self::Function(id) => crate::hir::HirCallTarget::Function(id),
            Self::Extern(id) => crate::hir::HirCallTarget::Extern(id),
            Self::Local(id) => crate::hir::HirCallTarget::Local(id),
            Self::Intrinsic(name) => crate::hir::HirCallTarget::Intrinsic(name),
            Self::StaticMethod { owner_ty, method } => {
                crate::hir::HirCallTarget::StaticMethod(crate::hir::HirStaticMethodTarget {
                    owner_ty: decoder.decode_type(owner_ty)?,
                    method: method.decode(decoder)?,
                })
            }
        })
    }
}

impl SerializedHirStructLiteralField {
    fn encode(
        value: &crate::hir::HirStructLiteralFieldFor<crate::hir::AcceptedHir>,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            name: value.name.clone(),
            value: SerializedHirExpr::encode(&value.value, encoder)?,
            field: value.field.clone(),
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirStructLiteralFieldFor<crate::hir::AcceptedHir>, String> {
        Ok(crate::hir::HirStructLiteralFieldFor {
            name: self.name,
            value: self.value.decode(decoder)?,
            field: self.field,
        })
    }
}

impl SerializedHirMatchArm {
    fn encode(
        value: &crate::hir::HirMatchArmFor<crate::hir::AcceptedHir>,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            pattern: SerializedHirPattern::encode(&value.pattern, encoder)?,
            guard: value
                .guard
                .as_ref()
                .map(|guard| SerializedHirExpr::encode(guard, encoder))
                .transpose()?,
            body: SerializedHirBlock::encode(&value.body, encoder)?,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirMatchArmFor<crate::hir::AcceptedHir>, String> {
        Ok(crate::hir::HirMatchArmFor {
            pattern: self.pattern.decode(decoder)?,
            guard: self.guard.map(|guard| guard.decode(decoder)).transpose()?,
            body: self.body.decode(decoder)?,
        })
    }
}

impl SerializedHirStructPatternField {
    fn encode(
        value: &crate::hir::HirStructPatternField,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            name: value.name.clone(),
            field: value.field.clone(),
            pattern: SerializedHirPattern::encode(&value.pattern, encoder)?,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirStructPatternField, String> {
        Ok(crate::hir::HirStructPatternField {
            name: self.name,
            field: self.field,
            pattern: self.pattern.decode(decoder)?,
        })
    }
}

impl SerializedHirPattern {
    fn encode(
        value: &crate::hir::HirPattern,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(match value {
            crate::hir::HirPattern::Wildcard => Self::Wildcard,
            crate::hir::HirPattern::Binding {
                name,
                local_id,
                mutable,
            } => Self::Binding {
                name: name.clone(),
                local_id: *local_id,
                mutable: *mutable,
            },
            crate::hir::HirPattern::Literal(value) => Self::Literal(value.clone()),
            crate::hir::HirPattern::Tuple(patterns) => {
                Self::Tuple(encode_patterns(patterns, encoder)?)
            }
            crate::hir::HirPattern::Struct(name, struct_id, type_args, fields) => Self::Struct(
                name.clone(),
                *struct_id,
                encoder.encode_types(type_args)?,
                fields
                    .iter()
                    .map(|field| SerializedHirStructPatternField::encode(field, encoder))
                    .collect::<Result<_, _>>()?,
            ),
            crate::hir::HirPattern::Enum(enum_name, variant_name, location, patterns) => {
                Self::Enum(
                    enum_name.clone(),
                    variant_name.clone(),
                    location.clone(),
                    encode_patterns(patterns, encoder)?,
                )
            }
            crate::hir::HirPattern::Or(patterns) => Self::Or(encode_patterns(patterns, encoder)?),
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirPattern, String> {
        Ok(match self {
            Self::Wildcard => crate::hir::HirPattern::Wildcard,
            Self::Binding {
                name,
                local_id,
                mutable,
            } => crate::hir::HirPattern::Binding {
                name,
                local_id,
                mutable,
            },
            Self::Literal(value) => crate::hir::HirPattern::Literal(value),
            Self::Tuple(patterns) => {
                crate::hir::HirPattern::Tuple(decode_patterns(patterns, decoder)?)
            }
            Self::Struct(name, struct_id, type_args, fields) => crate::hir::HirPattern::Struct(
                name,
                struct_id,
                decoder.decode_types(&type_args)?,
                fields
                    .into_iter()
                    .map(|field| field.decode(decoder))
                    .collect::<Result<_, _>>()?,
            ),
            Self::Enum(enum_name, variant_name, location, patterns) => {
                crate::hir::HirPattern::Enum(
                    enum_name,
                    variant_name,
                    location,
                    decode_patterns(patterns, decoder)?,
                )
            }
            Self::Or(patterns) => crate::hir::HirPattern::Or(decode_patterns(patterns, decoder)?),
        })
    }
}

fn encode_exprs(
    values: &[crate::hir::AcceptedHirExpr],
    encoder: &mut ProductTypeEncoder,
) -> Result<Vec<SerializedHirExpr>, String> {
    values
        .iter()
        .map(|value| SerializedHirExpr::encode(value, encoder))
        .collect()
}

fn decode_exprs(
    values: Vec<SerializedHirExpr>,
    decoder: &mut ProductTypeDecoder<'_>,
) -> Result<Vec<crate::hir::AcceptedHirExpr>, String> {
    values
        .into_iter()
        .map(|value| value.decode(decoder))
        .collect()
}

fn encode_patterns(
    values: &[crate::hir::HirPattern],
    encoder: &mut ProductTypeEncoder,
) -> Result<Vec<SerializedHirPattern>, String> {
    values
        .iter()
        .map(|value| SerializedHirPattern::encode(value, encoder))
        .collect()
}

fn decode_patterns(
    values: Vec<SerializedHirPattern>,
    decoder: &mut ProductTypeDecoder<'_>,
) -> Result<Vec<crate::hir::HirPattern>, String> {
    values
        .into_iter()
        .map(|value| value.decode(decoder))
        .collect()
}

fn encode_hash_map<T, U>(
    map: &HashMap<String, T>,
    encoder: &mut ProductTypeEncoder,
    encode: fn(&T, &mut ProductTypeEncoder) -> Result<U, String>,
) -> Result<HashMap<String, U>, String> {
    map.iter()
        .map(|(name, value)| Ok((name.clone(), encode(value, encoder)?)))
        .collect()
}

fn decode_hash_map<T, U>(
    map: HashMap<String, T>,
    decoder: &mut ProductTypeDecoder<'_>,
    decode: fn(T, &mut ProductTypeDecoder<'_>) -> Result<U, String>,
) -> Result<HashMap<String, U>, String> {
    map.into_iter()
        .map(|(name, value)| Ok((name, decode(value, decoder)?)))
        .collect()
}

fn encode_generic_bounds(
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
        relaxed_sized: bounds
            .relaxed_sized
            .iter()
            .copied()
            .map(encode_generic_param_id)
            .collect(),
        bounds: encoded,
        predicates: bounds
            .predicates
            .iter()
            .map(|predicate| SerializedPredicate::encode(predicate, encoder))
            .collect::<Result<_, _>>()?,
    })
}

fn decode_generic_bounds(
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
    decoded.relaxed_sized = bounds
        .relaxed_sized
        .into_iter()
        .map(decode_generic_param_id)
        .collect();
    Ok(decoded)
}

fn encode_generic_param_decls(decls: &[GenericParamDecl]) -> Vec<ProductGenericParamDecl> {
    decls
        .iter()
        .map(|decl| ProductGenericParamDecl {
            maybe_unsized: decl.maybe_unsized,
            id: encode_generic_param_id(decl.id),
            name: decl.name.clone(),
            kind: decl.kind.clone(),
        })
        .collect()
}

fn decode_generic_param_decls(decls: Vec<ProductGenericParamDecl>) -> Vec<GenericParamDecl> {
    decls
        .into_iter()
        .map(|decl| GenericParamDecl {
            maybe_unsized: decl.maybe_unsized,
            id: decode_generic_param_id(decl.id),
            name: decl.name,
            kind: decl.kind,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) struct ProductTypeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) struct ProductGenericParamId {
    pub owner: ProductDefId,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct ProductGenericParamDecl {
    pub maybe_unsized: bool,
    pub id: ProductGenericParamId,
    pub name: String,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct ProductAssociatedTypeKey {
    pub owner: ProductDefId,
    pub assoc_type_id: AssocTypeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) enum ProductFunctionSafety {
    Safe,
    Unsafe,
}

impl From<FunctionSafety> for ProductFunctionSafety {
    fn from(value: FunctionSafety) -> Self {
        match value {
            FunctionSafety::Safe => ProductFunctionSafety::Safe,
            FunctionSafety::Unsafe => ProductFunctionSafety::Unsafe,
        }
    }
}

impl From<ProductFunctionSafety> for FunctionSafety {
    fn from(value: ProductFunctionSafety) -> Self {
        match value {
            ProductFunctionSafety::Safe => FunctionSafety::Safe,
            ProductFunctionSafety::Unsafe => FunctionSafety::Unsafe,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) enum ProductCallableKind {
    Fn,
    FnMut,
    FnOnce,
}

impl From<CallableKind> for ProductCallableKind {
    fn from(value: CallableKind) -> Self {
        match value {
            CallableKind::Fn => Self::Fn,
            CallableKind::FnMut => Self::FnMut,
            CallableKind::FnOnce => Self::FnOnce,
        }
    }
}

impl From<ProductCallableKind> for CallableKind {
    fn from(value: ProductCallableKind) -> Self {
        match value {
            ProductCallableKind::Fn => Self::Fn,
            ProductCallableKind::FnMut => Self::FnMut,
            ProductCallableKind::FnOnce => Self::FnOnce,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) enum ProductCaptureKind {
    SharedBorrow,
    MutableBorrow,
    Move,
}

impl From<CaptureKind> for ProductCaptureKind {
    fn from(value: CaptureKind) -> Self {
        match value {
            CaptureKind::SharedBorrow => Self::SharedBorrow,
            CaptureKind::MutableBorrow => Self::MutableBorrow,
            CaptureKind::Move => Self::Move,
        }
    }
}

impl From<ProductCaptureKind> for CaptureKind {
    fn from(value: ProductCaptureKind) -> Self {
        match value {
            ProductCaptureKind::SharedBorrow => Self::SharedBorrow,
            ProductCaptureKind::MutableBorrow => Self::MutableBorrow,
            ProductCaptureKind::Move => Self::Move,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct ProductFunctionCapture {
    pub kind: ProductCaptureKind,
    pub ty: ProductTypeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) enum ProductNominalTypeKind {
    Struct,
    Enum,
    Alias,
}

impl From<crate::types::NominalTypeKind> for ProductNominalTypeKind {
    fn from(value: crate::types::NominalTypeKind) -> Self {
        match value {
            crate::types::NominalTypeKind::Struct => Self::Struct,
            crate::types::NominalTypeKind::Enum => Self::Enum,
            crate::types::NominalTypeKind::Alias => Self::Alias,
        }
    }
}

impl From<ProductNominalTypeKind> for crate::types::NominalTypeKind {
    fn from(value: ProductNominalTypeKind) -> Self {
        match value {
            ProductNominalTypeKind::Struct => Self::Struct,
            ProductNominalTypeKind::Enum => Self::Enum,
            ProductNominalTypeKind::Alias => Self::Alias,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct ProductObjectTrait {
    id: ProductDefId,
    args: Vec<ProductTypeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct ProductObjectBinding {
    trait_ref: ProductObjectTrait,
    member: AssocTypeId,
    ty: ProductTypeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct ProductObjectType {
    principal: ProductObjectTrait,
    guarantees: Vec<ProductObjectTrait>,
    bindings: Vec<ProductObjectBinding>,
}

impl ProductObjectType {
    fn children(&self) -> impl Iterator<Item = ProductTypeId> + '_ {
        self.principal
            .args
            .iter()
            .copied()
            .chain(
                self.guarantees
                    .iter()
                    .flat_map(|bound| bound.args.iter().copied()),
            )
            .chain(self.bindings.iter().flat_map(|binding| {
                binding
                    .trait_ref
                    .args
                    .iter()
                    .copied()
                    .chain(std::iter::once(binding.ty))
            }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) enum ProductTypeRow {
    Witness(ProductWitnessId),
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    Str,
    Char,
    Unit,
    Never,
    Slice(ProductTypeId),
    Array(ProductTypeId, usize),
    Tuple(Vec<ProductTypeId>),
    Function {
        params: Vec<ProductTypeId>,
        ret: ProductTypeId,
        safety: ProductFunctionSafety,
        callable_kind: ProductCallableKind,
        captures: Vec<ProductFunctionCapture>,
    },
    Struct {
        id: ProductDefId,
        args: Vec<ProductTypeId>,
    },
    Enum {
        id: ProductDefId,
        args: Vec<ProductTypeId>,
    },
    Reference {
        mutable: bool,
        inner: ProductTypeId,
    },
    Pointer(ProductTypeId),
    Object(ProductObjectType),
    ObjectSelf {
        depth: u32,
    },
    Generic(ProductGenericParamId),
    Projection {
        ty: ProductTypeId,
        trait_id: ProductDefId,
        assoc_type: ProductAssociatedTypeKey,
        trait_args: Vec<ProductTypeId>,
    },
    Constructor {
        id: ProductDefId,
        flavor: ProductNominalTypeKind,
    },
    Apply {
        constructor: ProductTypeId,
        args: Vec<ProductTypeId>,
    },
    Lambda {
        params: Vec<Kind>,
        body: ProductTypeId,
    },
    BoundVar {
        depth: u32,
        index: u32,
        kind: Kind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct ProductWitnessId {
    owner: ProductDefId,
    local: u32,
}

impl ProductWitnessId {
    fn encode(witness: crate::types::WitnessId) -> Self {
        Self {
            owner: product_def_id(witness.owner),
            local: witness.local.0,
        }
    }

    fn decode(self) -> crate::types::WitnessId {
        crate::types::WitnessId {
            owner: def_id(self.owner),
            local: crate::ids::HirLocalId(self.local),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct ProductTypeTable {
    pub rows: Vec<ProductTypeRow>,
}

#[derive(Debug, Default)]
pub(super) struct ProductTypeEncoder {
    table: ProductTypeTable,
    ids: HashMap<ProductTypeRow, ProductTypeId>,
}

impl ProductTypeEncoder {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn finish(self) -> ProductTypeTable {
        self.table
    }

    pub(super) fn encode_type(&mut self, ty: &Type) -> Result<ProductTypeId, String> {
        let row = self.encode_row(ty)?;
        if let Some(id) = self.ids.get(&row).copied() {
            return Ok(id);
        }

        let id = ProductTypeId(self.table.rows.len() as u32);
        self.table.rows.push(row.clone());
        self.ids.insert(row, id);
        Ok(id)
    }

    pub(super) fn encode_types(&mut self, types: &[Type]) -> Result<Vec<ProductTypeId>, String> {
        types.iter().map(|ty| self.encode_type(ty)).collect()
    }

    fn encode_object_bound(
        &mut self,
        bound: &crate::types::TraitBound,
    ) -> Result<ProductObjectTrait, String> {
        Ok(ProductObjectTrait {
            id: product_def_id(bound.trait_id),
            args: self.encode_types(&bound.type_args)?,
        })
    }

    fn encode_row(&mut self, ty: &Type) -> Result<ProductTypeRow, String> {
        Ok(match ty {
            Type::Witness(witness) => ProductTypeRow::Witness(ProductWitnessId::encode(*witness)),
            Type::ObjectSelf { depth } => ProductTypeRow::ObjectSelf { depth: *depth },
            Type::Object(object) => {
                if object.conflicting_binding().is_some()
                    || object.guarantees.contains(&object.principal)
                {
                    return Err("noncanonical trait object signature".to_string());
                }
                ProductTypeRow::Object(ProductObjectType {
                    principal: self.encode_object_bound(&object.principal)?,
                    guarantees: object
                        .guarantees
                        .iter()
                        .map(|bound| self.encode_object_bound(bound))
                        .collect::<Result<_, _>>()?,
                    bindings: object
                        .bindings
                        .iter()
                        .map(|binding| {
                            Ok(ProductObjectBinding {
                                trait_ref: self.encode_object_bound(&binding.key.trait_ref)?,
                                member: binding.key.member,
                                ty: self.encode_type(&binding.ty)?,
                            })
                        })
                        .collect::<Result<_, String>>()?,
                })
            }
            Type::I8 => ProductTypeRow::I8,
            Type::I16 => ProductTypeRow::I16,
            Type::I32 => ProductTypeRow::I32,
            Type::I64 => ProductTypeRow::I64,
            Type::U8 => ProductTypeRow::U8,
            Type::U16 => ProductTypeRow::U16,
            Type::U32 => ProductTypeRow::U32,
            Type::U64 => ProductTypeRow::U64,
            Type::F32 => ProductTypeRow::F32,
            Type::F64 => ProductTypeRow::F64,
            Type::Bool => ProductTypeRow::Bool,
            Type::Str => ProductTypeRow::Str,
            Type::Char => ProductTypeRow::Char,
            Type::Unit => ProductTypeRow::Unit,
            Type::Never => ProductTypeRow::Never,
            Type::Slice(inner) => ProductTypeRow::Slice(self.encode_type(inner)?),
            Type::Array(inner, len) => ProductTypeRow::Array(self.encode_type(inner)?, *len),
            Type::Tuple(elems) => ProductTypeRow::Tuple(self.encode_types(elems)?),
            Type::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
            } => ProductTypeRow::Function {
                params: self.encode_types(params)?,
                ret: self.encode_type(ret)?,
                safety: (*safety).into(),
                callable_kind: (*callable_kind).into(),
                captures: captures
                    .iter()
                    .map(|capture| {
                        Ok(ProductFunctionCapture {
                            kind: capture.kind.into(),
                            ty: self.encode_type(&capture.ty)?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
            },
            Type::Struct { id, args } => ProductTypeRow::Struct {
                id: product_def_id(*id),
                args: self.encode_types(args)?,
            },
            Type::Enum { id, args } => ProductTypeRow::Enum {
                id: product_def_id(*id),
                args: self.encode_types(args)?,
            },
            Type::Reference { mutable, inner } => ProductTypeRow::Reference {
                mutable: *mutable,
                inner: self.encode_type(inner)?,
            },
            Type::Pointer(inner) => ProductTypeRow::Pointer(self.encode_type(inner)?),
            Type::TypeVar(_) => {
                return Err(
                    "product artifacts cannot serialize unresolved Type::TypeVar values"
                        .to_string(),
                );
            }
            Type::Generic(param) => ProductTypeRow::Generic(ProductGenericParamId {
                owner: product_def_id(param.owner),
                index: param.index,
            }),
            Type::Projection {
                ty,
                trait_id,
                assoc_type,
                trait_args,
            } => ProductTypeRow::Projection {
                ty: self.encode_type(ty)?,
                trait_id: product_def_id(*trait_id),
                assoc_type: ProductAssociatedTypeKey {
                    owner: product_def_id(assoc_type.owner),
                    assoc_type_id: assoc_type.assoc_type_id,
                },
                trait_args: self.encode_types(trait_args)?,
            },
            Type::Constructor { id, flavor } => ProductTypeRow::Constructor {
                id: product_def_id(*id),
                flavor: (*flavor).into(),
            },
            Type::Apply { constructor, args } => ProductTypeRow::Apply {
                constructor: self.encode_type(constructor)?,
                args: self.encode_types(args)?,
            },
            Type::Lambda { params, body } => ProductTypeRow::Lambda {
                params: params.clone(),
                body: self.encode_type(body)?,
            },
            Type::BoundVar { depth, index, kind } => ProductTypeRow::BoundVar {
                depth: *depth,
                index: *index,
                kind: kind.clone(),
            },
            Type::Error => {
                return Err(
                    "product artifacts cannot serialize unresolved Type::Error values".to_string(),
                );
            }
        })
    }
}

pub(super) struct ProductTypeDecoder<'a> {
    table: &'a ProductTypeTable,
    stack: Vec<ProductTypeId>,
    decoded_nodes: usize,
}

impl<'a> ProductTypeDecoder<'a> {
    pub(super) fn new(table: &'a ProductTypeTable) -> Self {
        Self {
            table,
            stack: Vec::new(),
            decoded_nodes: 0,
        }
    }

    pub(super) fn decode_type(&mut self, id: ProductTypeId) -> Result<Type, String> {
        self.decode_type_in_object_scope(id, 0)
    }

    pub(super) fn decode_object_component(&mut self, id: ProductTypeId) -> Result<Type, String> {
        self.decode_type_in_object_scope(id, 1)
    }

    fn decode_type_in_object_scope(
        &mut self,
        id: ProductTypeId,
        object_depth: u32,
    ) -> Result<Type, String> {
        self.decoded_nodes = self.decoded_nodes.saturating_add(1);
        if self.decoded_nodes > crate::products::MAX_PRODUCT_ARTIFACT_NORMALIZATION_NODES {
            return Err("Product artifact decoded type node limit exceeded".into());
        }
        let root = self.stack.is_empty();
        let row = self
            .table
            .rows
            .get(id.0 as usize)
            .ok_or_else(|| format!("unknown product type id {}", id.0))?;
        if self.stack.contains(&id) {
            return Err(format!("recursive product type id {}", id.0));
        }

        self.stack.push(id);
        let decoded = self.decode_row(row);
        self.stack.pop();
        let decoded = decoded?;
        if root {
            object_abi::validate_component_scope(&decoded, object_depth)?;
        }
        Ok(decoded)
    }

    pub(super) fn decode_types(&mut self, ids: &[ProductTypeId]) -> Result<Vec<Type>, String> {
        ids.iter().map(|id| self.decode_type(*id)).collect()
    }

    fn decode_object_bound(
        &mut self,
        bound: &ProductObjectTrait,
    ) -> Result<crate::types::TraitBound, String> {
        Ok(crate::types::TraitBound {
            trait_id: def_id(bound.id),
            type_args: self.decode_types(&bound.args)?,
        })
    }

    fn decode_row(&mut self, row: &ProductTypeRow) -> Result<Type, String> {
        Ok(match row {
            ProductTypeRow::Witness(witness) => Type::Witness(witness.decode()),
            ProductTypeRow::ObjectSelf { depth } => Type::ObjectSelf { depth: *depth },
            ProductTypeRow::Object(encoded) => {
                let mut object =
                    crate::types::ObjectType::new(self.decode_object_bound(&encoded.principal)?);
                for bound in &encoded.guarantees {
                    let bound = self.decode_object_bound(bound)?;
                    if bound == object.principal || !object.guarantees.insert(bound) {
                        return Err("duplicate trait object guarantee".to_string());
                    }
                }
                for binding in &encoded.bindings {
                    let binding = crate::types::ObjectBinding {
                        key: crate::types::ObjectAssociatedType {
                            trait_ref: self.decode_object_bound(&binding.trait_ref)?,
                            member: binding.member,
                        },
                        ty: self.decode_type(binding.ty)?,
                    };
                    if !object.bindings.insert(binding) {
                        return Err("duplicate trait object associated binding".to_string());
                    }
                }
                if object.conflicting_binding().is_some() {
                    return Err("conflicting trait object associated binding".to_string());
                }
                Type::Object(Box::new(object))
            }
            ProductTypeRow::I8 => Type::I8,
            ProductTypeRow::I16 => Type::I16,
            ProductTypeRow::I32 => Type::I32,
            ProductTypeRow::I64 => Type::I64,
            ProductTypeRow::U8 => Type::U8,
            ProductTypeRow::U16 => Type::U16,
            ProductTypeRow::U32 => Type::U32,
            ProductTypeRow::U64 => Type::U64,
            ProductTypeRow::F32 => Type::F32,
            ProductTypeRow::F64 => Type::F64,
            ProductTypeRow::Bool => Type::Bool,
            ProductTypeRow::Str => Type::Str,
            ProductTypeRow::Char => Type::Char,
            ProductTypeRow::Unit => Type::Unit,
            ProductTypeRow::Never => Type::Never,
            ProductTypeRow::Slice(inner) => Type::Slice(Box::new(self.decode_type(*inner)?)),
            ProductTypeRow::Array(inner, len) => {
                Type::Array(Box::new(self.decode_type(*inner)?), *len)
            }
            ProductTypeRow::Tuple(elems) => Type::Tuple(self.decode_types(elems)?),
            ProductTypeRow::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
            } => Type::Function {
                params: self.decode_types(params)?,
                ret: Box::new(self.decode_type(*ret)?),
                safety: (*safety).into(),
                callable_kind: (*callable_kind).into(),
                captures: captures
                    .iter()
                    .map(|capture| {
                        Ok(FunctionCapture {
                            kind: capture.kind.into(),
                            ty: self.decode_type(capture.ty)?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
            },
            ProductTypeRow::Struct { id, args } => Type::Struct {
                id: def_id(*id),
                args: self.decode_types(args)?,
            },
            ProductTypeRow::Enum { id, args } => Type::Enum {
                id: def_id(*id),
                args: self.decode_types(args)?,
            },
            ProductTypeRow::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(self.decode_type(*inner)?),
            },
            ProductTypeRow::Pointer(inner) => Type::Pointer(Box::new(self.decode_type(*inner)?)),
            ProductTypeRow::Generic(param) => Type::Generic(GenericParamId {
                owner: def_id(param.owner),
                index: param.index,
            }),
            ProductTypeRow::Projection {
                ty,
                trait_id,
                assoc_type,
                trait_args,
            } => Type::Projection {
                ty: Box::new(self.decode_type(*ty)?),
                trait_id: def_id(*trait_id),
                assoc_type: AssociatedTypeKey {
                    owner: def_id(assoc_type.owner),
                    assoc_type_id: assoc_type.assoc_type_id,
                },
                trait_args: self.decode_types(trait_args)?,
            },
            ProductTypeRow::Constructor { id, flavor } => Type::Constructor {
                id: def_id(*id),
                flavor: (*flavor).into(),
            },
            ProductTypeRow::Apply { constructor, args } => Type::Apply {
                constructor: Box::new(self.decode_type(*constructor)?),
                args: self.decode_types(args)?,
            },
            ProductTypeRow::Lambda { params, body } => Type::Lambda {
                params: params.clone(),
                body: Box::new(self.decode_type(*body)?),
            },
            ProductTypeRow::BoundVar { depth, index, kind } => Type::BoundVar {
                depth: *depth,
                index: *index,
                kind: kind.clone(),
            },
        })
    }
}

fn product_def_id(id: DefId) -> ProductDefId {
    ProductDefId {
        crate_id: ProductCrateId(id.crate_id.0),
        local_id: ProductLocalDefId(id.local.0),
    }
}

fn def_id(id: ProductDefId) -> DefId {
    DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0))
}

#[cfg(test)]
pub(super) fn encode_types_for_test(
    types: &[Type],
) -> Result<(ProductTypeTable, Vec<ProductTypeId>), String> {
    let mut encoder = ProductTypeEncoder::new();
    let ids = encoder.encode_types(types)?;
    Ok((encoder.finish(), ids))
}

#[cfg(test)]
pub(super) fn decode_type_for_test(
    table: &ProductTypeTable,
    id: ProductTypeId,
) -> Result<Type, String> {
    ProductTypeDecoder::new(table).decode_type(id)
}

#[cfg(test)]
pub(super) fn artifact_type_table_len_for_test(bytes: &[u8]) -> Result<usize, String> {
    let artifact = super::serialized_artifact_from_bytes_for_test(bytes)?;
    Ok(artifact.type_table.rows.len())
}

#[cfg(test)]
pub(super) fn artifact_type_table_contains_type_for_test(
    bytes: &[u8],
    expected: &Type,
) -> Result<bool, String> {
    let artifact = super::serialized_artifact_from_bytes_for_test(bytes)?;
    let mut decoder = ProductTypeDecoder::new(&artifact.type_table);

    for index in 0..artifact.type_table.rows.len() {
        if decoder.decode_type(ProductTypeId(index as u32))? == *expected {
            return Ok(true);
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, TypeVarId, VariantId};
    use crate::type_services::kind::Kind;
    use crate::types::{
        AssociatedTypeKey, CallableKind, CaptureKind, FunctionCapture, GenericParamDecl,
        GenericParamId, Type,
    };

    use super::{
        decode_type_for_test, encode_types_for_test, ProductTypeId, ProductTypeRow,
        ProductTypeTable, SerializedHirExpr, SerializedHirExprKind,
    };

    fn def(local: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(local))
    }

    #[test]
    fn portable_open_binding_round_trip_keeps_descriptor_and_local_identity() {
        let witness = crate::types::WitnessId {
            owner: def(2),
            local: crate::ids::HirLocalId(7),
        };
        let object = crate::types::ObjectType::new(crate::types::TraitBound {
            trait_id: def(4),
            type_args: vec![],
        });
        let binding = crate::hir::HirOpenBinding {
            witness,
            value: crate::hir::HirParam {
                name: "value".into(),
                local_id: witness.local,
                ty: Type::Reference {
                    mutable: true,
                    inner: Box::new(Type::Witness(witness)),
                },
                mutable: false,
                is_ref: true,
            },
            source_ty: Type::Reference {
                mutable: true,
                inner: Box::new(Type::Object(Box::new(object.clone()))),
            },
            object: object.clone(),
            owner: None,
        };
        let mut encoder = super::ProductTypeEncoder::new();
        let encoded = super::SerializedOpenBinding::encode(&binding, &mut encoder).unwrap();
        let table = encoder.finish();
        assert!(table
            .rows
            .iter()
            .any(|row| matches!(row, ProductTypeRow::Witness(id) if id.decode() == witness)));
        let mut decoder = super::ProductTypeDecoder::new(&table);
        let decoded = encoded.decode(&mut decoder).unwrap();
        assert_eq!(decoded.witness, witness);
        assert_eq!(decoded.value.local_id, witness.local);
        assert_eq!(decoded.value.ty, binding.value.ty);
        assert_eq!(decoded.object, object);
        assert_eq!(decoded.source_ty, binding.source_ty);
    }

    #[test]
    fn portable_object_self_is_scoped_only_by_object_signatures() {
        let signature = super::ProductObjectType {
            principal: super::ProductObjectTrait {
                id: super::product_def_id(def(1)),
                args: vec![ProductTypeId(0)],
            },
            guarantees: vec![],
            bindings: vec![],
        };
        let mut table = ProductTypeTable {
            rows: vec![
                ProductTypeRow::ObjectSelf { depth: 0 },
                ProductTypeRow::Object(signature),
            ],
        };
        super::validate_type_table_graph(&table).unwrap();
        table.rows[0] = ProductTypeRow::ObjectSelf { depth: 1 };
        assert!(super::validate_type_table_graph(&table)
            .unwrap_err()
            .contains("out-of-scope product object Self"));
        table.rows[0] = ProductTypeRow::ObjectSelf { depth: 0 };
        table.rows[1] = ProductTypeRow::Lambda {
            params: vec![Kind::Type],
            body: ProductTypeId(0),
        };
        assert!(super::validate_type_table_graph(&table)
            .unwrap_err()
            .contains("out-of-scope product object Self"));
    }

    #[test]
    fn portable_object_type_roundtrips_instantiated_associated_bindings() {
        let mut object = crate::types::ObjectType::new(crate::types::TraitBound {
            trait_id: def(1),
            type_args: vec![Type::I64],
        });
        object.guarantees.insert(crate::types::TraitBound {
            trait_id: def(2),
            type_args: vec![],
        });
        object.guarantees.insert(crate::types::TraitBound {
            trait_id: def(1),
            type_args: vec![Type::Bool],
        });
        for (arg, ty) in [(Type::I64, Type::Bool), (Type::Bool, Type::I64)] {
            object.bindings.insert(crate::types::ObjectBinding {
                key: crate::types::ObjectAssociatedType {
                    trait_ref: crate::types::TraitBound {
                        trait_id: def(1),
                        type_args: vec![arg],
                    },
                    member: AssocTypeId(3),
                },
                ty,
            });
        }
        let ty = Type::Object(Box::new(object));
        let mut encoder = super::ProductTypeEncoder::new();
        let id = encoder.encode_type(&ty).unwrap();
        let table = encoder.finish();
        super::validate_type_table_graph(&table).unwrap();
        assert_eq!(decode_type_for_test(&table, id).unwrap(), ty);
    }

    #[test]
    fn portable_object_type_rejects_duplicate_guarantees_and_invalid_children() {
        let bound = super::ProductObjectTrait {
            id: super::product_def_id(def(1)),
            args: vec![],
        };
        let mut table = ProductTypeTable {
            rows: vec![ProductTypeRow::Object(super::ProductObjectType {
                principal: bound.clone(),
                guarantees: vec![bound],
                bindings: vec![],
            })],
        };
        assert!(decode_type_for_test(&table, ProductTypeId(0))
            .unwrap_err()
            .contains("duplicate trait object"));
        let ProductTypeRow::Object(object) = &mut table.rows[0] else {
            unreachable!()
        };
        object.guarantees.clear();
        object.principal.args.push(ProductTypeId(123));
        assert!(super::validate_type_table_graph(&table)
            .unwrap_err()
            .contains("unknown product type id"));
    }

    fn unit_expr() -> SerializedHirExpr {
        SerializedHirExpr {
            kind: SerializedHirExprKind::Unit,
            ty: ProductTypeId(0),
            span: crate::lexer::Span::test(),
        }
    }

    #[test]
    fn product_generic_param_decl_roundtrips_kind_name_and_id() {
        let declarations = vec![GenericParamDecl::new(
            GenericParamId {
                owner: def(7),
                index: 2,
            },
            "F",
            Kind::arrow(Kind::Type, Kind::Type),
        )];

        let encoded = super::encode_generic_param_decls(&declarations);
        assert_eq!(encoded[0].id.owner, super::product_def_id(def(7)));
        assert_eq!(encoded[0].name, "F");
        assert_eq!(encoded[0].kind, Kind::arrow(Kind::Type, Kind::Type));
        assert_eq!(super::decode_generic_param_decls(encoded), declarations);
    }

    #[test]
    fn serialized_method_try_and_array_repeat_roundtrip() {
        let variants = vec![
            SerializedHirExprKind::MethodCall(
                Box::new(unit_expr()),
                "borrow".to_string(),
                Vec::new(),
                Some(crate::types::ReceiverMode::Shared),
                None,
            ),
            SerializedHirExprKind::Try {
                expr: Box::new(unit_expr()),
                branch_method: None,
                branch_self_receiver: Some(crate::types::ReceiverMode::Move),
                from_residual_target: None,
                output_ty: ProductTypeId(0),
                residual_ty: ProductTypeId(0),
                return_ty: ProductTypeId(0),
                control_flow_enum: def(1),
                break_variant: crate::hir::HirVariantLocation {
                    owner: def(1),
                    variant_id: VariantId(0),
                    name: "Break".to_string(),
                },
                continue_variant: crate::hir::HirVariantLocation {
                    owner: def(1),
                    variant_id: VariantId(1),
                    name: "Continue".to_string(),
                },
            },
            SerializedHirExprKind::ArrayRepeat(Box::new(unit_expr()), 256),
        ];

        let bytes = bincode::serialize(&variants).unwrap();
        let decoded: Vec<SerializedHirExprKind> = bincode::deserialize(&bytes).unwrap();

        assert!(matches!(
            &decoded[0],
            SerializedHirExprKind::MethodCall(_, _, _, Some(crate::types::ReceiverMode::Shared), _)
        ));
        assert!(matches!(
            &decoded[1],
            SerializedHirExprKind::Try {
                branch_self_receiver: Some(crate::types::ReceiverMode::Move),
                ..
            }
        ));
        assert!(matches!(
            &decoded[2],
            SerializedHirExprKind::ArrayRepeat(_, 256)
        ));
    }

    #[test]
    fn product_type_table_deduplicates_equal_types() {
        let ty = Type::Struct {
            id: def(2),
            args: vec![Type::I64],
        };
        let (table, ids) = encode_types_for_test(&[ty.clone(), ty]).unwrap();

        assert_eq!(ids, vec![ProductTypeId(1), ProductTypeId(1)]);
        assert_eq!(
            table.rows.len(),
            2,
            "struct row plus i64 row should be stored once each"
        );
    }

    #[test]
    fn product_type_table_roundtrips_nested_projection_type() {
        let ty = Type::Projection {
            ty: Box::new(Type::Struct {
                id: def(2),
                args: vec![Type::Generic(GenericParamId {
                    owner: def(1),
                    index: 0,
                })],
            }),
            trait_id: def(3),
            assoc_type: AssociatedTypeKey {
                owner: def(3),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            }],
        };
        let (table, ids) = encode_types_for_test(&[ty.clone()]).unwrap();
        let decoded = decode_type_for_test(&table, ids[0]).unwrap();

        assert_eq!(decoded, ty);
    }

    #[test]
    fn product_type_table_roundtrips_function_safety() {
        let ty = Type::unsafe_function(vec![Type::I64], Type::Bool);
        let (table, ids) = encode_types_for_test(&[ty.clone()]).unwrap();
        let decoded = decode_type_for_test(&table, ids[0]).unwrap();

        assert_eq!(decoded, ty);
    }

    #[test]
    fn product_type_table_roundtrips_callable_kind_and_captures() {
        let ty = Type::function_with_metadata(
            vec![Type::I64],
            Type::Bool,
            crate::types::FunctionSafety::Safe,
            CallableKind::FnOnce,
            vec![FunctionCapture::new(CaptureKind::Move, Type::Str)],
        );
        let (table, ids) = encode_types_for_test(std::slice::from_ref(&ty)).unwrap();
        let decoded = decode_type_for_test(&table, ids[0]).unwrap();

        assert_eq!(decoded, ty);
    }

    #[test]
    fn product_type_table_rejects_type_vars() {
        let err = encode_types_for_test(&[Type::TypeVar(TypeVarId(0))]).unwrap_err();

        assert!(err.contains("TypeVar"), "unexpected error: {err}");
    }

    #[test]
    fn product_type_table_rejects_nested_type_vars() {
        let ty = Type::Tuple(vec![Type::Reference {
            mutable: false,
            inner: Box::new(Type::TypeVar(TypeVarId(0))),
        }]);

        let err = encode_types_for_test(&[ty]).unwrap_err();

        assert!(err.contains("TypeVar"), "unexpected error: {err}");
    }

    #[test]
    fn product_type_table_rejects_error_types() {
        let err = encode_types_for_test(&[Type::Error]).unwrap_err();

        assert!(err.contains("Type::Error"), "unexpected error: {err}");
    }

    #[test]
    fn product_type_table_rejects_nested_error_types() {
        let ty = Type::Struct {
            id: def(2),
            args: vec![Type::Tuple(vec![Type::Reference {
                mutable: false,
                inner: Box::new(Type::Error),
            }])],
        };

        let err = encode_types_for_test(&[ty]).unwrap_err();

        assert!(err.contains("Type::Error"), "unexpected error: {err}");
    }

    #[test]
    fn product_type_table_rejects_invalid_type_id() {
        let (table, _) = encode_types_for_test(&[Type::I64]).unwrap();
        let err = decode_type_for_test(&table, ProductTypeId(99)).unwrap_err();

        assert!(
            err.contains("unknown product type id 99"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_type_table_rejects_invalid_nested_type_ids() {
        let table = ProductTypeTable {
            rows: vec![ProductTypeRow::Tuple(vec![ProductTypeId(4)])],
        };
        let err = decode_type_for_test(&table, ProductTypeId(0)).unwrap_err();

        assert!(
            err.contains("unknown product type id 4"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn portable_type_table_validates_lambda_binder_scope() {
        let valid = ProductTypeTable {
            rows: vec![
                ProductTypeRow::BoundVar {
                    depth: 0,
                    index: 0,
                    kind: Kind::Type,
                },
                ProductTypeRow::Lambda {
                    params: vec![Kind::Type],
                    body: ProductTypeId(0),
                },
            ],
        };
        super::validate_type_table_graph(&valid).unwrap();

        let invalid = ProductTypeTable {
            rows: vec![ProductTypeRow::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            }],
        };
        let error = super::validate_type_table_graph(&invalid).unwrap_err();
        assert!(error.contains("out-of-scope"), "{error}");
    }

    #[test]
    fn portable_type_table_rejects_excessive_type_depth() {
        let mut rows = vec![ProductTypeRow::I64];
        for index in 0..crate::products::MAX_PRODUCT_ARTIFACT_TYPE_DEPTH {
            rows.push(ProductTypeRow::Slice(ProductTypeId(index as u32)));
        }
        let error = super::validate_type_table_graph(&ProductTypeTable { rows }).unwrap_err();

        assert!(error.contains("type depth limit exceeded"), "{error}");
    }

    #[test]
    fn product_type_table_rejects_recursive_type_id() {
        let table = ProductTypeTable {
            rows: vec![ProductTypeRow::Slice(ProductTypeId(0))],
        };
        let err = decode_type_for_test(&table, ProductTypeId(0)).unwrap_err();

        assert!(
            err.contains("recursive product type id 0"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn product_type_table_decodes_product_def_ids_as_def_ids() {
        let ty = Type::Enum {
            id: DefId::new(CrateId(7), LocalDefId(9)),
            args: Vec::new(),
        };
        let (table, ids) = encode_types_for_test(&[ty.clone()]).unwrap();
        let row = &table.rows[ids[0].0 as usize];

        assert!(format!("{row:?}").contains("ProductCrateId(7)"));
        assert_eq!(decode_type_for_test(&table, ids[0]).unwrap(), ty);
    }

    #[test]
    fn serialized_product_artifact_roundtrips_empty_products() {
        let products = crate::products::CompilerProducts {
            crate_identity: crate::products::ProductCrateIdentity::local("empty".to_string()),
            identity_table: crate::products::ProductIdentityTable::default(),
            interface: crate::products::ProductInterface::default(),
            bodies: crate::products::ProductBodies::default(),
            link: crate::products::ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: crate::products::ProductSourceFingerprint::default(),
            infix_precedence: std::collections::BTreeMap::new(),
            proc_macros: Vec::new(),
        };
        let bytes = products.to_artifact_bytes().unwrap();
        let roundtrip = crate::products::CompilerProducts::from_artifact_bytes(&bytes).unwrap();

        assert_eq!(roundtrip.crate_identity.name, "empty");
    }

    #[test]
    fn serialized_product_artifact_roundtrips_interface_without_metadata_functions() {
        let product_id = crate::products::ProductDefId {
            crate_id: crate::products::ProductCrateId(0),
            local_id: crate::products::ProductLocalDefId(1),
        };
        let mut interface = crate::products::ProductInterface::default();
        interface.functions.insert(
            product_id,
            crate::products::ProductFunctionInterface {
                id: def(1),
                name: "identity".to_string(),
                generic_params: Vec::new(),
                generic_bounds: std::collections::HashMap::new().into(),
                params: vec![Type::I64],
                ret_type: Type::I64,
                is_curried: false,
                is_method: false,
                self_receiver: None,
                is_unsafe: false,
            },
        );
        let products = crate::products::CompilerProducts {
            crate_identity: crate::products::ProductCrateIdentity::local("interface".to_string()),
            identity_table: crate::products::ProductIdentityTable::default(),
            interface,
            bodies: crate::products::ProductBodies::default(),
            link: crate::products::ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: crate::products::ProductSourceFingerprint::default(),
            infix_precedence: std::collections::BTreeMap::new(),
            proc_macros: Vec::new(),
        };

        let bytes = products.to_artifact_bytes().unwrap();
        let decoded = crate::products::CompilerProducts::from_artifact_bytes(&bytes).unwrap();

        assert_eq!(decoded.interface.functions[&product_id].name, "identity");
        assert!(decoded.bodies.functions.is_empty());
    }

    #[test]
    fn typed_predicate_artifact_roundtrips_constructor_arguments() {
        let product_id = crate::products::ProductDefId {
            crate_id: crate::products::ProductCrateId(0),
            local_id: crate::products::ProductLocalDefId(10),
        };
        let trait_id = def(10);
        let parent_id = def(11);
        let constructor_id = def(12);
        let target = GenericParamDecl::new(
            crate::types::GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "F",
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            ),
        );
        let predicate = crate::types::Predicate::Trait {
            subject: Type::Generic(target.id),
            trait_id: parent_id,
            args: vec![Type::Constructor {
                id: constructor_id,
                flavor: crate::types::NominalTypeKind::Struct,
            }],
        };
        let mut interface = crate::products::ProductInterface::default();
        interface.traits.insert(
            product_id,
            crate::products::ProductTraitInterface {
                id: trait_id,
                name: "Applicative".to_string(),
                generic_params: Vec::new(),
                target: Some(target.clone()),
                predicates: vec![predicate.clone()],
                associated_types: Vec::new(),
                methods: std::collections::BTreeMap::new(),
                signatures: std::collections::HashMap::new(),
            },
        );
        let products = crate::products::CompilerProducts {
            crate_identity: crate::products::ProductCrateIdentity::local("predicates".to_string()),
            identity_table: crate::products::ProductIdentityTable::default(),
            interface,
            bodies: crate::products::ProductBodies::default(),
            link: crate::products::ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: crate::products::ProductSourceFingerprint::default(),
            infix_precedence: std::collections::BTreeMap::new(),
            proc_macros: Vec::new(),
        };

        let bytes = products.to_artifact_bytes().unwrap();
        let decoded = crate::products::CompilerProducts::from_artifact_bytes(&bytes).unwrap();
        let decoded_trait = &decoded.interface.traits[&product_id];

        assert_eq!(decoded_trait.target, Some(target));
        assert_eq!(decoded_trait.predicates, vec![predicate]);
    }

    #[test]
    fn serialized_product_artifact_uses_type_table_shell_for_empty_products() {
        let products = crate::products::CompilerProducts {
            crate_identity: crate::products::ProductCrateIdentity::local("empty".to_string()),
            identity_table: crate::products::ProductIdentityTable::default(),
            interface: crate::products::ProductInterface::default(),
            bodies: crate::products::ProductBodies::default(),
            link: crate::products::ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: crate::products::ProductSourceFingerprint::default(),
            infix_precedence: std::collections::BTreeMap::new(),
            proc_macros: Vec::new(),
        };

        let bytes = products.to_artifact_bytes().unwrap();
        let artifact = crate::products::serialized_artifact_from_bytes_for_test(&bytes).unwrap();

        assert_eq!(
            artifact.format_version,
            crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION
        );
        assert_eq!(artifact.type_table.rows.len(), 0);
        assert_eq!(artifact.products.crate_identity.name, "empty");
    }
}
