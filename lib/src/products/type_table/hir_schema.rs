use std::collections::HashMap;

use super::product_schema::{SerializedHirFunction, SerializedHirImplReceiverPattern};
use super::type_rows::{ProductGenericParamDecl, ProductGenericParamId, ProductTypeId};
use crate::products::ProductDefId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedHirParam {
    pub name: String,
    pub local_id: crate::ids::HirLocalId,
    pub ty: ProductTypeId,
    pub mutable: bool,
    pub is_ref: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedHirField {
    pub id: crate::ids::FieldId,
    pub name: String,
    pub ty: ProductTypeId,
    pub public: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) enum SerializedHirVariantFields {
    Named(Vec<SerializedHirField>),
    Positional(Vec<ProductTypeId>),
    Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedHirFunctionSig {
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
pub(in crate::products) struct SerializedHirImpl {
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
pub(in crate::products) struct SerializedHirAssociatedTypeDef {
    pub id: crate::ids::AssocTypeId,
    pub name: String,
    pub kind: crate::type_services::kind::Kind,
    pub ty: ProductTypeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedTraitBound {
    pub trait_id: ProductDefId,
    pub type_args: Vec<ProductTypeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedGenericBounds {
    pub bounds: Vec<(ProductGenericParamId, Vec<SerializedTraitBound>)>,
    pub predicates: Vec<SerializedPredicate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedPredicate {
    pub subject: ProductTypeId,
    pub trait_id: ProductDefId,
    pub args: Vec<ProductTypeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedHirClosureCapture {
    pub name: String,
    pub local_id: crate::ids::HirLocalId,
    pub kind: crate::hir::HirClosureCaptureKind,
    pub mutable: bool,
    pub ty: ProductTypeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedHirBlock {
    pub stmts: Vec<SerializedHirStmt>,
    pub ty: ProductTypeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) enum SerializedHirStmt {
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
pub(in crate::products) struct SerializedHirExpr {
    pub kind: SerializedHirExprKind,
    pub ty: ProductTypeId,
    pub span: crate::lexer::Span,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) enum SerializedHirExprKind {
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
    Assign(Box<SerializedHirExpr>, Box<SerializedHirExpr>),
    Intrinsic {
        name: String,
        args: Vec<SerializedHirExpr>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) enum SerializedHirCallTarget {
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
pub(in crate::products) struct SerializedHirVarRef {
    pub(super) name: String,
    pub(super) target: SerializedHirVarTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) enum SerializedHirVarTarget {
    Function(crate::ids::DefId),
    Extern(crate::ids::DefId),
    Local(crate::ids::HirLocalId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedHirMethodCallTarget {
    pub target: SerializedHirSelectedMethodTarget,
    pub owner_substitution: Vec<SerializedHirTypeBinding>,
    pub method_substitution: Vec<SerializedHirTypeBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) enum SerializedHirSelectedMethodTarget {
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
pub(in crate::products) struct SerializedHirSelectedTraitMember {
    pub trait_id: crate::ids::DefId,
    pub member_id: crate::ids::DefId,
    pub trait_args: Vec<ProductTypeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedHirTypeBinding {
    pub param: crate::types::GenericParamId,
    pub ty: ProductTypeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedHirStructLiteralField {
    pub name: String,
    pub value: SerializedHirExpr,
    pub field: Option<crate::hir::HirFieldLocation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedHirMatchArm {
    pub pattern: SerializedHirPattern,
    pub guard: Option<SerializedHirExpr>,
    pub body: SerializedHirBlock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) struct SerializedHirStructPatternField {
    pub name: String,
    pub field: Option<crate::hir::HirFieldLocation>,
    pub pattern: SerializedHirPattern,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::products) enum SerializedHirPattern {
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
