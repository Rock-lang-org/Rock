use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MirIntrinsicId {
    Integer(crate::mir::integer::IntegerIntrinsic),
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
    PtrFromParts,
    ArrPtr,
    SizeOf,
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
                | Self::PtrFromParts
                | Self::AtomicU64Exchange
                | Self::AtomicU64FetchAdd
                | Self::AtomicU64FetchSub
        )
    }

    pub fn is_type_only(&self) -> bool {
        matches!(self, Self::ArrayLen | Self::SizeOf)
    }
}

impl fmt::Display for MirIntrinsicId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
