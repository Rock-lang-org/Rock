use crate::ids::AssocTypeId;
use crate::products::ProductDefId;
use crate::type_services::kind::Kind;
use crate::types::{CallableKind, CaptureKind, FunctionSafety};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(in crate::products) struct ProductTypeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(in crate::products) struct ProductGenericParamId {
    pub owner: ProductDefId,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(in crate::products) struct ProductGenericParamDecl {
    pub id: ProductGenericParamId,
    pub name: String,
    pub kind: Kind,
    pub default: Option<ProductTypeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(in crate::products) struct ProductAssociatedTypeKey {
    pub owner: ProductDefId,
    pub assoc_type_id: AssocTypeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(in crate::products) enum ProductFunctionSafety {
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
pub(in crate::products) enum ProductCallableKind {
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
pub(in crate::products) enum ProductCaptureKind {
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
pub(in crate::products) struct ProductFunctionCapture {
    pub kind: ProductCaptureKind,
    pub effect: crate::types::CaptureEffect,
    pub ty: ProductTypeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(in crate::products) enum ProductNominalTypeKind {
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
pub(in crate::products) enum ProductTypeRow {
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
    ConstValue(u64),
    Slice(ProductTypeId),
    /// Element type and length; the length is a const-kinded type row
    /// (`ConstValue` or `Generic`) so id remapping treats it like any argument.
    Array(ProductTypeId, ProductTypeId),
    Tuple(Vec<ProductTypeId>),
    Function {
        params: Vec<ProductTypeId>,
        ret: ProductTypeId,
        safety: ProductFunctionSafety,
        callable_kind: ProductCallableKind,
        captures: Vec<ProductFunctionCapture>,
        captures_known: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(in crate::products) struct ProductTypeTable {
    pub rows: Vec<ProductTypeRow>,
}
