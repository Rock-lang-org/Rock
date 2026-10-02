use std::fmt;
use std::hash::{Hash, Hasher};

use crate::ids::{DefId, FieldId, InstanceId, VariantId};

use super::backend_contract::MirCallableKey;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MirFunctionId {
    Function(DefId),
    Extern(DefId),
    Instance(InstanceId),
    Closure(Box<MirClosureId>),
}

impl MirFunctionId {
    pub fn display_fallback(self, _fallback: &str) -> String {
        match self {
            MirFunctionId::Function(id) => format!("def#{}:{}", id.crate_id.0, id.local.0),
            MirFunctionId::Extern(id) => format!("extern#{}:{}", id.crate_id.0, id.local.0),
            MirFunctionId::Instance(id) => format!("instance#{}", id.0),
            MirFunctionId::Closure(id) => format!("closure#{:?}:{}", id.owner, id.local_index),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MirClosureId {
    pub owner: MirFunctionId,
    pub local_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MirRuntimeHelper {
    BoundsCheck,
    HeapAlloc,
    HeapFree,
    DropGlue,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MirIntrinsicId {
    I8Add,
    I16Add,
    I32Add,
    I64Add,
    U8Add,
    U16Add,
    U32Add,
    U64Add,
    F32Add,
    F64Add,
    I8Sub,
    I16Sub,
    I32Sub,
    I64Sub,
    U8Sub,
    U16Sub,
    U32Sub,
    U64Sub,
    F32Sub,
    F64Sub,
    I8Mul,
    I16Mul,
    I32Mul,
    I64Mul,
    U8Mul,
    U16Mul,
    U32Mul,
    U64Mul,
    F32Mul,
    F64Mul,
    I8Div,
    I16Div,
    I32Div,
    I64Div,
    U8Div,
    U16Div,
    U32Div,
    U64Div,
    F32Div,
    F64Div,
    I8Mod,
    I16Mod,
    I32Mod,
    I64Mod,
    U8Mod,
    U16Mod,
    U32Mod,
    U64Mod,
    F32Mod,
    F64Mod,
    I8Neg,
    I16Neg,
    I32Neg,
    I64Neg,
    F32Neg,
    F64Neg,
    I8Lt,
    I16Lt,
    I32Lt,
    I64Lt,
    U8Lt,
    U16Lt,
    U32Lt,
    U64Lt,
    F32Lt,
    F64Lt,
    I8Le,
    I16Le,
    I32Le,
    I64Le,
    U8Le,
    U16Le,
    U32Le,
    U64Le,
    F32Le,
    F64Le,
    I8Gt,
    I16Gt,
    I32Gt,
    I64Gt,
    U8Gt,
    U16Gt,
    U32Gt,
    U64Gt,
    F32Gt,
    F64Gt,
    I8Ge,
    I16Ge,
    I32Ge,
    I64Ge,
    U8Ge,
    U16Ge,
    U32Ge,
    U64Ge,
    F32Ge,
    F64Ge,
    I8Eq,
    I16Eq,
    I32Eq,
    I64Eq,
    U8Eq,
    U16Eq,
    U32Eq,
    U64Eq,
    F32Eq,
    F64Eq,
    BoolEq,
    I8Ne,
    I16Ne,
    I32Ne,
    I64Ne,
    U8Ne,
    U16Ne,
    U32Ne,
    U64Ne,
    F32Ne,
    F64Ne,
    BoolNe,
    I8And,
    I16And,
    I32And,
    I64And,
    U8And,
    U16And,
    U32And,
    U64And,
    I8Or,
    I16Or,
    I32Or,
    I64Or,
    U8Or,
    U16Or,
    U32Or,
    U64Or,
    I8Xor,
    I16Xor,
    I32Xor,
    I64Xor,
    U8Xor,
    U16Xor,
    U32Xor,
    U64Xor,
    I8Not,
    I16Not,
    I32Not,
    I64Not,
    U8Not,
    U16Not,
    U32Not,
    U64Not,
    I8Shl,
    I16Shl,
    I32Shl,
    I64Shl,
    U8Shl,
    U16Shl,
    U32Shl,
    U64Shl,
    I8Shr,
    I16Shr,
    I32Shr,
    I64Shr,
    U8Shr,
    U16Shr,
    U32Shr,
    U64Shr,
    BoolAnd,
    BoolOr,
    BoolXor,
    BoolNot,
    ArrayRefToSlice,
    ArrayLen,
    PtrOffset,
    MakeArr,
    BorrowSlice,
    BorrowSliceMut,
    BorrowStr,
    ArrPtr,
    SizeOf,
    AlignOf,
    SizeOfValue,
    AlignOfValue,
    DropInPlace,
    Forget,
    AtomicU64Exchange,
    AtomicU64FetchAdd,
    AtomicU64FetchSub,
    AtomicU64Store,
    #[cfg(test)]
    DisplayNameI64Add,
}

impl MirIntrinsicId {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "I8Add" => Self::I8Add,
            "I16Add" => Self::I16Add,
            "I32Add" => Self::I32Add,
            "I64Add" => Self::I64Add,
            "U8Add" => Self::U8Add,
            "U16Add" => Self::U16Add,
            "U32Add" => Self::U32Add,
            "U64Add" => Self::U64Add,
            "F32Add" => Self::F32Add,
            "F64Add" => Self::F64Add,
            "I8Sub" => Self::I8Sub,
            "I16Sub" => Self::I16Sub,
            "I32Sub" => Self::I32Sub,
            "I64Sub" => Self::I64Sub,
            "U8Sub" => Self::U8Sub,
            "U16Sub" => Self::U16Sub,
            "U32Sub" => Self::U32Sub,
            "U64Sub" => Self::U64Sub,
            "F32Sub" => Self::F32Sub,
            "F64Sub" => Self::F64Sub,
            "I8Mul" => Self::I8Mul,
            "I16Mul" => Self::I16Mul,
            "I32Mul" => Self::I32Mul,
            "I64Mul" => Self::I64Mul,
            "U8Mul" => Self::U8Mul,
            "U16Mul" => Self::U16Mul,
            "U32Mul" => Self::U32Mul,
            "U64Mul" => Self::U64Mul,
            "F32Mul" => Self::F32Mul,
            "F64Mul" => Self::F64Mul,
            "I8Div" => Self::I8Div,
            "I16Div" => Self::I16Div,
            "I32Div" => Self::I32Div,
            "I64Div" => Self::I64Div,
            "U8Div" => Self::U8Div,
            "U16Div" => Self::U16Div,
            "U32Div" => Self::U32Div,
            "U64Div" => Self::U64Div,
            "F32Div" => Self::F32Div,
            "F64Div" => Self::F64Div,
            "I8Mod" => Self::I8Mod,
            "I16Mod" => Self::I16Mod,
            "I32Mod" => Self::I32Mod,
            "I64Mod" => Self::I64Mod,
            "U8Mod" => Self::U8Mod,
            "U16Mod" => Self::U16Mod,
            "U32Mod" => Self::U32Mod,
            "U64Mod" => Self::U64Mod,
            "F32Mod" => Self::F32Mod,
            "F64Mod" => Self::F64Mod,
            "I8Neg" => Self::I8Neg,
            "I16Neg" => Self::I16Neg,
            "I32Neg" => Self::I32Neg,
            "I64Neg" => Self::I64Neg,
            "F32Neg" => Self::F32Neg,
            "F64Neg" => Self::F64Neg,
            "I8Lt" => Self::I8Lt,
            "I16Lt" => Self::I16Lt,
            "I32Lt" => Self::I32Lt,
            "I64Lt" => Self::I64Lt,
            "U8Lt" => Self::U8Lt,
            "U16Lt" => Self::U16Lt,
            "U32Lt" => Self::U32Lt,
            "U64Lt" => Self::U64Lt,
            "F32Lt" => Self::F32Lt,
            "F64Lt" => Self::F64Lt,
            "I8Le" => Self::I8Le,
            "I16Le" => Self::I16Le,
            "I32Le" => Self::I32Le,
            "I64Le" => Self::I64Le,
            "U8Le" => Self::U8Le,
            "U16Le" => Self::U16Le,
            "U32Le" => Self::U32Le,
            "U64Le" => Self::U64Le,
            "F32Le" => Self::F32Le,
            "F64Le" => Self::F64Le,
            "I8Gt" => Self::I8Gt,
            "I16Gt" => Self::I16Gt,
            "I32Gt" => Self::I32Gt,
            "I64Gt" => Self::I64Gt,
            "U8Gt" => Self::U8Gt,
            "U16Gt" => Self::U16Gt,
            "U32Gt" => Self::U32Gt,
            "U64Gt" => Self::U64Gt,
            "F32Gt" => Self::F32Gt,
            "F64Gt" => Self::F64Gt,
            "I8Ge" => Self::I8Ge,
            "I16Ge" => Self::I16Ge,
            "I32Ge" => Self::I32Ge,
            "I64Ge" => Self::I64Ge,
            "U8Ge" => Self::U8Ge,
            "U16Ge" => Self::U16Ge,
            "U32Ge" => Self::U32Ge,
            "U64Ge" => Self::U64Ge,
            "F32Ge" => Self::F32Ge,
            "F64Ge" => Self::F64Ge,
            "I8Eq" => Self::I8Eq,
            "I16Eq" => Self::I16Eq,
            "I32Eq" => Self::I32Eq,
            "I64Eq" => Self::I64Eq,
            "U8Eq" => Self::U8Eq,
            "U16Eq" => Self::U16Eq,
            "U32Eq" => Self::U32Eq,
            "U64Eq" => Self::U64Eq,
            "F32Eq" => Self::F32Eq,
            "F64Eq" => Self::F64Eq,
            "BoolEq" => Self::BoolEq,
            "I8Ne" => Self::I8Ne,
            "I16Ne" => Self::I16Ne,
            "I32Ne" => Self::I32Ne,
            "I64Ne" => Self::I64Ne,
            "U8Ne" => Self::U8Ne,
            "U16Ne" => Self::U16Ne,
            "U32Ne" => Self::U32Ne,
            "U64Ne" => Self::U64Ne,
            "F32Ne" => Self::F32Ne,
            "F64Ne" => Self::F64Ne,
            "BoolNe" => Self::BoolNe,
            "I8And" => Self::I8And,
            "I16And" => Self::I16And,
            "I32And" => Self::I32And,
            "I64And" => Self::I64And,
            "U8And" => Self::U8And,
            "U16And" => Self::U16And,
            "U32And" => Self::U32And,
            "U64And" => Self::U64And,
            "I8Or" => Self::I8Or,
            "I16Or" => Self::I16Or,
            "I32Or" => Self::I32Or,
            "I64Or" => Self::I64Or,
            "U8Or" => Self::U8Or,
            "U16Or" => Self::U16Or,
            "U32Or" => Self::U32Or,
            "U64Or" => Self::U64Or,
            "I8Xor" => Self::I8Xor,
            "I16Xor" => Self::I16Xor,
            "I32Xor" => Self::I32Xor,
            "I64Xor" => Self::I64Xor,
            "U8Xor" => Self::U8Xor,
            "U16Xor" => Self::U16Xor,
            "U32Xor" => Self::U32Xor,
            "U64Xor" => Self::U64Xor,
            "I8Not" => Self::I8Not,
            "I16Not" => Self::I16Not,
            "I32Not" => Self::I32Not,
            "I64Not" => Self::I64Not,
            "U8Not" => Self::U8Not,
            "U16Not" => Self::U16Not,
            "U32Not" => Self::U32Not,
            "U64Not" => Self::U64Not,
            "I8Shl" => Self::I8Shl,
            "I16Shl" => Self::I16Shl,
            "I32Shl" => Self::I32Shl,
            "I64Shl" => Self::I64Shl,
            "U8Shl" => Self::U8Shl,
            "U16Shl" => Self::U16Shl,
            "U32Shl" => Self::U32Shl,
            "U64Shl" => Self::U64Shl,
            "I8Shr" => Self::I8Shr,
            "I16Shr" => Self::I16Shr,
            "I32Shr" => Self::I32Shr,
            "I64Shr" => Self::I64Shr,
            "U8Shr" => Self::U8Shr,
            "U16Shr" => Self::U16Shr,
            "U32Shr" => Self::U32Shr,
            "U64Shr" => Self::U64Shr,
            "BoolAnd" => Self::BoolAnd,
            "BoolOr" => Self::BoolOr,
            "BoolXor" => Self::BoolXor,
            "BoolNot" => Self::BoolNot,
            "ArrayRefToSlice" => Self::ArrayRefToSlice,
            "ArrayLen" => Self::ArrayLen,
            "PtrOffset" => Self::PtrOffset,
            "MakeArr" => Self::MakeArr,
            "BorrowSlice" => Self::BorrowSlice,
            "BorrowSliceMut" => Self::BorrowSliceMut,
            "BorrowStr" => Self::BorrowStr,
            "ArrPtr" => Self::ArrPtr,
            "SizeOf" => Self::SizeOf,
            "AlignOf" => Self::AlignOf,
            "SizeOfValue" => Self::SizeOfValue,
            "AlignOfValue" => Self::AlignOfValue,
            "DropInPlace" => Self::DropInPlace,
            "Forget" => Self::Forget,
            "AtomicU64Exchange" => Self::AtomicU64Exchange,
            "AtomicU64FetchAdd" => Self::AtomicU64FetchAdd,
            "AtomicU64FetchSub" => Self::AtomicU64FetchSub,
            "AtomicU64Store" => Self::AtomicU64Store,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::I8Add => "I8Add",
            Self::I16Add => "I16Add",
            Self::I32Add => "I32Add",
            Self::I64Add => "I64Add",
            Self::U8Add => "U8Add",
            Self::U16Add => "U16Add",
            Self::U32Add => "U32Add",
            Self::U64Add => "U64Add",
            Self::F32Add => "F32Add",
            Self::F64Add => "F64Add",
            Self::I8Sub => "I8Sub",
            Self::I16Sub => "I16Sub",
            Self::I32Sub => "I32Sub",
            Self::I64Sub => "I64Sub",
            Self::U8Sub => "U8Sub",
            Self::U16Sub => "U16Sub",
            Self::U32Sub => "U32Sub",
            Self::U64Sub => "U64Sub",
            Self::F32Sub => "F32Sub",
            Self::F64Sub => "F64Sub",
            Self::I8Mul => "I8Mul",
            Self::I16Mul => "I16Mul",
            Self::I32Mul => "I32Mul",
            Self::I64Mul => "I64Mul",
            Self::U8Mul => "U8Mul",
            Self::U16Mul => "U16Mul",
            Self::U32Mul => "U32Mul",
            Self::U64Mul => "U64Mul",
            Self::F32Mul => "F32Mul",
            Self::F64Mul => "F64Mul",
            Self::I8Div => "I8Div",
            Self::I16Div => "I16Div",
            Self::I32Div => "I32Div",
            Self::I64Div => "I64Div",
            Self::U8Div => "U8Div",
            Self::U16Div => "U16Div",
            Self::U32Div => "U32Div",
            Self::U64Div => "U64Div",
            Self::F32Div => "F32Div",
            Self::F64Div => "F64Div",
            Self::I8Mod => "I8Mod",
            Self::I16Mod => "I16Mod",
            Self::I32Mod => "I32Mod",
            Self::I64Mod => "I64Mod",
            Self::U8Mod => "U8Mod",
            Self::U16Mod => "U16Mod",
            Self::U32Mod => "U32Mod",
            Self::U64Mod => "U64Mod",
            Self::F32Mod => "F32Mod",
            Self::F64Mod => "F64Mod",
            Self::I8Neg => "I8Neg",
            Self::I16Neg => "I16Neg",
            Self::I32Neg => "I32Neg",
            Self::I64Neg => "I64Neg",
            Self::F32Neg => "F32Neg",
            Self::F64Neg => "F64Neg",
            Self::I8Lt => "I8Lt",
            Self::I16Lt => "I16Lt",
            Self::I32Lt => "I32Lt",
            Self::I64Lt => "I64Lt",
            Self::U8Lt => "U8Lt",
            Self::U16Lt => "U16Lt",
            Self::U32Lt => "U32Lt",
            Self::U64Lt => "U64Lt",
            Self::F32Lt => "F32Lt",
            Self::F64Lt => "F64Lt",
            Self::I8Le => "I8Le",
            Self::I16Le => "I16Le",
            Self::I32Le => "I32Le",
            Self::I64Le => "I64Le",
            Self::U8Le => "U8Le",
            Self::U16Le => "U16Le",
            Self::U32Le => "U32Le",
            Self::U64Le => "U64Le",
            Self::F32Le => "F32Le",
            Self::F64Le => "F64Le",
            Self::I8Gt => "I8Gt",
            Self::I16Gt => "I16Gt",
            Self::I32Gt => "I32Gt",
            Self::I64Gt => "I64Gt",
            Self::U8Gt => "U8Gt",
            Self::U16Gt => "U16Gt",
            Self::U32Gt => "U32Gt",
            Self::U64Gt => "U64Gt",
            Self::F32Gt => "F32Gt",
            Self::F64Gt => "F64Gt",
            Self::I8Ge => "I8Ge",
            Self::I16Ge => "I16Ge",
            Self::I32Ge => "I32Ge",
            Self::I64Ge => "I64Ge",
            Self::U8Ge => "U8Ge",
            Self::U16Ge => "U16Ge",
            Self::U32Ge => "U32Ge",
            Self::U64Ge => "U64Ge",
            Self::F32Ge => "F32Ge",
            Self::F64Ge => "F64Ge",
            Self::I8Eq => "I8Eq",
            Self::I16Eq => "I16Eq",
            Self::I32Eq => "I32Eq",
            Self::I64Eq => "I64Eq",
            Self::U8Eq => "U8Eq",
            Self::U16Eq => "U16Eq",
            Self::U32Eq => "U32Eq",
            Self::U64Eq => "U64Eq",
            Self::F32Eq => "F32Eq",
            Self::F64Eq => "F64Eq",
            Self::BoolEq => "BoolEq",
            Self::I8Ne => "I8Ne",
            Self::I16Ne => "I16Ne",
            Self::I32Ne => "I32Ne",
            Self::I64Ne => "I64Ne",
            Self::U8Ne => "U8Ne",
            Self::U16Ne => "U16Ne",
            Self::U32Ne => "U32Ne",
            Self::U64Ne => "U64Ne",
            Self::F32Ne => "F32Ne",
            Self::F64Ne => "F64Ne",
            Self::BoolNe => "BoolNe",
            Self::I8And => "I8And",
            Self::I16And => "I16And",
            Self::I32And => "I32And",
            Self::I64And => "I64And",
            Self::U8And => "U8And",
            Self::U16And => "U16And",
            Self::U32And => "U32And",
            Self::U64And => "U64And",
            Self::I8Or => "I8Or",
            Self::I16Or => "I16Or",
            Self::I32Or => "I32Or",
            Self::I64Or => "I64Or",
            Self::U8Or => "U8Or",
            Self::U16Or => "U16Or",
            Self::U32Or => "U32Or",
            Self::U64Or => "U64Or",
            Self::I8Xor => "I8Xor",
            Self::I16Xor => "I16Xor",
            Self::I32Xor => "I32Xor",
            Self::I64Xor => "I64Xor",
            Self::U8Xor => "U8Xor",
            Self::U16Xor => "U16Xor",
            Self::U32Xor => "U32Xor",
            Self::U64Xor => "U64Xor",
            Self::I8Not => "I8Not",
            Self::I16Not => "I16Not",
            Self::I32Not => "I32Not",
            Self::I64Not => "I64Not",
            Self::U8Not => "U8Not",
            Self::U16Not => "U16Not",
            Self::U32Not => "U32Not",
            Self::U64Not => "U64Not",
            Self::I8Shl => "I8Shl",
            Self::I16Shl => "I16Shl",
            Self::I32Shl => "I32Shl",
            Self::I64Shl => "I64Shl",
            Self::U8Shl => "U8Shl",
            Self::U16Shl => "U16Shl",
            Self::U32Shl => "U32Shl",
            Self::U64Shl => "U64Shl",
            Self::I8Shr => "I8Shr",
            Self::I16Shr => "I16Shr",
            Self::I32Shr => "I32Shr",
            Self::I64Shr => "I64Shr",
            Self::U8Shr => "U8Shr",
            Self::U16Shr => "U16Shr",
            Self::U32Shr => "U32Shr",
            Self::U64Shr => "U64Shr",
            Self::BoolAnd => "BoolAnd",
            Self::BoolOr => "BoolOr",
            Self::BoolXor => "BoolXor",
            Self::BoolNot => "BoolNot",
            Self::ArrayRefToSlice => "ArrayRefToSlice",
            Self::ArrayLen => "ArrayLen",
            Self::PtrOffset => "PtrOffset",
            Self::MakeArr => "MakeArr",
            Self::BorrowSlice => "BorrowSlice",
            Self::BorrowSliceMut => "BorrowSliceMut",
            Self::BorrowStr => "BorrowStr",
            Self::ArrPtr => "ArrPtr",
            Self::SizeOf => "SizeOf",
            Self::AlignOf => "AlignOf",
            Self::SizeOfValue => "SizeOfValue",
            Self::AlignOfValue => "AlignOfValue",
            Self::DropInPlace => "DropInPlace",
            Self::Forget => "Forget",
            Self::AtomicU64Exchange => "AtomicU64Exchange",
            Self::AtomicU64FetchAdd => "AtomicU64FetchAdd",
            Self::AtomicU64FetchSub => "AtomicU64FetchSub",
            Self::AtomicU64Store => "AtomicU64Store",
            #[cfg(test)]
            Self::DisplayNameI64Add => "I64Add",
        }
    }

    pub fn is_value_intrinsic(&self) -> bool {
        matches!(
            self,
            Self::I8Add
                | Self::I16Add
                | Self::I32Add
                | Self::I64Add
                | Self::U8Add
                | Self::U16Add
                | Self::U32Add
                | Self::U64Add
                | Self::F32Add
                | Self::F64Add
                | Self::I8Sub
                | Self::I16Sub
                | Self::I32Sub
                | Self::I64Sub
                | Self::U8Sub
                | Self::U16Sub
                | Self::U32Sub
                | Self::U64Sub
                | Self::F32Sub
                | Self::F64Sub
                | Self::I8Mul
                | Self::I16Mul
                | Self::I32Mul
                | Self::I64Mul
                | Self::U8Mul
                | Self::U16Mul
                | Self::U32Mul
                | Self::U64Mul
                | Self::F32Mul
                | Self::F64Mul
                | Self::I8Div
                | Self::I16Div
                | Self::I32Div
                | Self::I64Div
                | Self::U8Div
                | Self::U16Div
                | Self::U32Div
                | Self::U64Div
                | Self::F32Div
                | Self::F64Div
                | Self::I8Mod
                | Self::I16Mod
                | Self::I32Mod
                | Self::I64Mod
                | Self::U8Mod
                | Self::U16Mod
                | Self::U32Mod
                | Self::U64Mod
                | Self::F32Mod
                | Self::F64Mod
                | Self::I8Neg
                | Self::I16Neg
                | Self::I32Neg
                | Self::I64Neg
                | Self::F32Neg
                | Self::F64Neg
                | Self::I8Lt
                | Self::I16Lt
                | Self::I32Lt
                | Self::I64Lt
                | Self::U8Lt
                | Self::U16Lt
                | Self::U32Lt
                | Self::U64Lt
                | Self::F32Lt
                | Self::F64Lt
                | Self::I8Le
                | Self::I16Le
                | Self::I32Le
                | Self::I64Le
                | Self::U8Le
                | Self::U16Le
                | Self::U32Le
                | Self::U64Le
                | Self::F32Le
                | Self::F64Le
                | Self::I8Gt
                | Self::I16Gt
                | Self::I32Gt
                | Self::I64Gt
                | Self::U8Gt
                | Self::U16Gt
                | Self::U32Gt
                | Self::U64Gt
                | Self::F32Gt
                | Self::F64Gt
                | Self::I8Ge
                | Self::I16Ge
                | Self::I32Ge
                | Self::I64Ge
                | Self::U8Ge
                | Self::U16Ge
                | Self::U32Ge
                | Self::U64Ge
                | Self::F32Ge
                | Self::F64Ge
                | Self::I8Eq
                | Self::I16Eq
                | Self::I32Eq
                | Self::I64Eq
                | Self::U8Eq
                | Self::U16Eq
                | Self::U32Eq
                | Self::U64Eq
                | Self::F32Eq
                | Self::F64Eq
                | Self::BoolEq
                | Self::I8Ne
                | Self::I16Ne
                | Self::I32Ne
                | Self::I64Ne
                | Self::U8Ne
                | Self::U16Ne
                | Self::U32Ne
                | Self::U64Ne
                | Self::F32Ne
                | Self::F64Ne
                | Self::BoolNe
                | Self::I8And
                | Self::I16And
                | Self::I32And
                | Self::I64And
                | Self::U8And
                | Self::U16And
                | Self::U32And
                | Self::U64And
                | Self::I8Or
                | Self::I16Or
                | Self::I32Or
                | Self::I64Or
                | Self::U8Or
                | Self::U16Or
                | Self::U32Or
                | Self::U64Or
                | Self::I8Xor
                | Self::I16Xor
                | Self::I32Xor
                | Self::I64Xor
                | Self::U8Xor
                | Self::U16Xor
                | Self::U32Xor
                | Self::U64Xor
                | Self::I8Not
                | Self::I16Not
                | Self::I32Not
                | Self::I64Not
                | Self::U8Not
                | Self::U16Not
                | Self::U32Not
                | Self::U64Not
                | Self::I8Shl
                | Self::I16Shl
                | Self::I32Shl
                | Self::I64Shl
                | Self::U8Shl
                | Self::U16Shl
                | Self::U32Shl
                | Self::U64Shl
                | Self::I8Shr
                | Self::I16Shr
                | Self::I32Shr
                | Self::I64Shr
                | Self::U8Shr
                | Self::U16Shr
                | Self::U32Shr
                | Self::U64Shr
                | Self::BoolAnd
                | Self::BoolOr
                | Self::BoolXor
                | Self::BoolNot
                | Self::MakeArr
                | Self::BorrowSlice
                | Self::BorrowSliceMut
                | Self::BorrowStr
                | Self::AtomicU64Exchange
                | Self::AtomicU64FetchAdd
                | Self::AtomicU64FetchSub
        )
    }

    pub fn is_type_only(&self) -> bool {
        matches!(self, Self::ArrayLen | Self::SizeOf | Self::AlignOf)
    }
}

impl fmt::Display for MirIntrinsicId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirCallable {
    Resolved(MirCallableKey),
}

impl MirCallable {
    pub fn function_id(&self) -> Option<MirFunctionId> {
        match self {
            MirCallable::Resolved(key) => match key {
                MirCallableKey::Function(id) => Some(MirFunctionId::Function(*id)),
                MirCallableKey::Extern(id) => Some(MirFunctionId::Extern(*id)),
                MirCallableKey::Instance(id) => Some(MirFunctionId::Instance(*id)),
                MirCallableKey::Closure(id) => Some(id.clone()),
                MirCallableKey::Intrinsic(_)
                | MirCallableKey::RuntimeHelper(_)
                | MirCallableKey::ObjectAdapter(_) => None,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum MirAggregateIdentity {
    Struct {
        id: DefId,
        display_name: String,
    },
    EnumVariant {
        enum_id: DefId,
        variant_id: VariantId,
        enum_name: String,
        variant_name: String,
    },
}

impl PartialEq for MirAggregateIdentity {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                MirAggregateIdentity::Struct { id: left, .. },
                MirAggregateIdentity::Struct { id: right, .. },
            ) => left == right,
            (
                MirAggregateIdentity::EnumVariant {
                    enum_id: left_enum_id,
                    variant_id: left_variant_id,
                    ..
                },
                MirAggregateIdentity::EnumVariant {
                    enum_id: right_enum_id,
                    variant_id: right_variant_id,
                    ..
                },
            ) => left_enum_id == right_enum_id && left_variant_id == right_variant_id,
            _ => false,
        }
    }
}

impl Eq for MirAggregateIdentity {}

impl Hash for MirAggregateIdentity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            MirAggregateIdentity::Struct { id, .. } => {
                0_u8.hash(state);
                id.hash(state);
            }
            MirAggregateIdentity::EnumVariant {
                enum_id,
                variant_id,
                ..
            } => {
                1_u8.hash(state);
                enum_id.hash(state);
                variant_id.hash(state);
            }
        }
    }
}

impl MirAggregateIdentity {
    pub fn def_id(&self) -> DefId {
        match self {
            MirAggregateIdentity::Struct { id, .. } => *id,
            MirAggregateIdentity::EnumVariant { enum_id, .. } => *enum_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MirFieldIdentity {
    pub owner: DefId,
    pub field_id: FieldId,
}

impl MirFieldIdentity {
    pub fn new(owner: DefId, field_id: FieldId) -> Self {
        Self { owner, field_id }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirAssertKind {
    BoundsCheck,
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use crate::ids::{CrateId, DefId, FieldId, InstanceId, LocalDefId, VariantId};

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn mir_function_identity_is_canonical_not_display_name() {
        let id = DefId::new(CrateId(0), LocalDefId(1));
        let left = MirFunctionId::Function(id);
        let right = MirFunctionId::Function(id);

        assert_eq!(left, right);
        assert_eq!(left.display_fallback("alias"), "def#0:1");
    }

    #[test]
    fn callable_and_aggregate_identities_carry_existing_compiler_ids() {
        let function = DefId::new(CrateId(0), LocalDefId(2));
        let structure = DefId::new(CrateId(0), LocalDefId(3));
        let enumeration = DefId::new(CrateId(0), LocalDefId(4));
        let variant = VariantId(5);

        assert_eq!(
            MirCallable::Resolved(MirCallableKey::Function(function)).function_id(),
            Some(MirFunctionId::Function(function))
        );
        assert_eq!(
            MirCallable::Resolved(MirCallableKey::Instance(InstanceId(7))).function_id(),
            Some(MirFunctionId::Instance(InstanceId(7)))
        );
        assert_eq!(
            MirAggregateIdentity::Struct {
                id: structure,
                display_name: "Point".to_string(),
            }
            .def_id(),
            structure
        );
        assert_eq!(
            MirAggregateIdentity::EnumVariant {
                enum_id: enumeration,
                variant_id: variant,
                enum_name: "Option".to_string(),
                variant_name: "Some".to_string(),
            }
            .def_id(),
            enumeration
        );
        assert_eq!(
            MirFieldIdentity::new(structure, FieldId(1)).field_id,
            FieldId(1)
        );
    }

    #[test]
    fn callable_intrinsic_identity_is_typed() {
        fn intrinsic_callable(id: MirIntrinsicId) -> MirCallable {
            MirCallable::Resolved(MirCallableKey::Intrinsic(id))
        }

        assert_eq!(
            intrinsic_callable(MirIntrinsicId::BorrowSlice),
            MirCallable::Resolved(MirCallableKey::Intrinsic(MirIntrinsicId::BorrowSlice))
        );
    }

    #[test]
    fn runtime_helper_has_explicit_heap_allocation_identity() {
        assert_ne!(MirRuntimeHelper::HeapAlloc, MirRuntimeHelper::DropGlue);
    }

    #[test]
    fn intrinsic_from_name_rejects_unknown_source_name() {
        assert!(MirIntrinsicId::from_name("NotARealIntrinsic").is_none());
    }

    #[test]
    fn struct_identity_ignores_display_name() {
        let id = DefId::new(CrateId(0), LocalDefId(20));
        let left = MirAggregateIdentity::Struct {
            id,
            display_name: "Point".to_string(),
        };
        let right = MirAggregateIdentity::Struct {
            id,
            display_name: "Alias".to_string(),
        };

        assert_eq!(left, right);
        assert_eq!(hash_of(&left), hash_of(&right));
    }

    #[test]
    fn enum_variant_identity_ignores_display_names() {
        let enum_id = DefId::new(CrateId(0), LocalDefId(30));
        let variant_id = VariantId(31);
        let left = MirAggregateIdentity::EnumVariant {
            enum_id,
            variant_id,
            enum_name: "Option".to_string(),
            variant_name: "Some".to_string(),
        };
        let right = MirAggregateIdentity::EnumVariant {
            enum_id,
            variant_id,
            enum_name: "Maybe".to_string(),
            variant_name: "Present".to_string(),
        };

        assert_eq!(left, right);
        assert_eq!(hash_of(&left), hash_of(&right));
    }
}
