//! Intrinsic function type inference helpers

use crate::hir::HirExpr;
use crate::types::Type;

/// Check if a name refers to an intrinsic function
pub fn is_intrinsic_name(name: &str) -> bool {
    matches!(
        name,
        // Arithmetic: all numeric types (no Bool)
        |"I8Add"| "I16Add"  | "I32Add"  | "I64Add"
        | "U8Add"  | "U16Add"  | "U32Add"  | "U64Add"
        | "F32Add" | "F64Add"
        | "I8Sub"  | "I16Sub"  | "I32Sub"  | "I64Sub"
        | "U8Sub"  | "U16Sub"  | "U32Sub"  | "U64Sub"
        | "F32Sub" | "F64Sub"
        | "I8Mul"  | "I16Mul"  | "I32Mul"  | "I64Mul"
        | "U8Mul"  | "U16Mul"  | "U32Mul"  | "U64Mul"
        | "F32Mul" | "F64Mul"
        | "I8Div"  | "I16Div"  | "I32Div"  | "I64Div"
        | "U8Div"  | "U16Div"  | "U32Div"  | "U64Div"
        | "F32Div" | "F64Div"
        | "I8Mod"  | "I16Mod"  | "I32Mod"  | "I64Mod"
        | "U8Mod"  | "U16Mod"  | "U32Mod"  | "U64Mod"
        | "F32Mod" | "F64Mod"
        // Negation: signed integers and floats only (no unsigned, no Bool)
        | "I8Neg"  | "I16Neg"  | "I32Neg"  | "I64Neg"
        | "F32Neg" | "F64Neg"
        // Comparisons: all numeric types + Bool
        | "I8Lt"  | "I16Lt"  | "I32Lt"  | "I64Lt"
        | "U8Lt"  | "U16Lt"  | "U32Lt"  | "U64Lt"
        | "F32Lt" | "F64Lt"
        | "I8Le"  | "I16Le"  | "I32Le"  | "I64Le"
        | "U8Le"  | "U16Le"  | "U32Le"  | "U64Le"
        | "F32Le" | "F64Le"
        | "I8Gt"  | "I16Gt"  | "I32Gt"  | "I64Gt"
        | "U8Gt"  | "U16Gt"  | "U32Gt"  | "U64Gt"
        | "F32Gt" | "F64Gt"
        | "I8Ge"  | "I16Ge"  | "I32Ge"  | "I64Ge"
        | "U8Ge"  | "U16Ge"  | "U32Ge"  | "U64Ge"
        | "F32Ge" | "F64Ge"
        | "I8Eq"  | "I16Eq"  | "I32Eq"  | "I64Eq"
        | "U8Eq"  | "U16Eq"  | "U32Eq"  | "U64Eq"
        | "F32Eq" | "F64Eq"  | "BoolEq"
        | "I8Ne"  | "I16Ne"  | "I32Ne"  | "I64Ne"
        | "U8Ne"  | "U16Ne"  | "U32Ne"  | "U64Ne"
        | "F32Ne" | "F64Ne"  | "BoolNe"
        // Bitwise: integer types only (no floats, no Bool arithmetic)
        | "I8And"  | "I16And"  | "I32And"  | "I64And"
        | "U8And"  | "U16And"  | "U32And"  | "U64And"
        | "I8Or"   | "I16Or"   | "I32Or"   | "I64Or"
        | "U8Or"   | "U16Or"   | "U32Or"   | "U64Or"
        | "I8Xor"  | "I16Xor"  | "I32Xor"  | "I64Xor"
        | "U8Xor"  | "U16Xor"  | "U32Xor"  | "U64Xor"
        | "I8Not"  | "I16Not"  | "I32Not"  | "I64Not"
        | "U8Not"  | "U16Not"  | "U32Not"  | "U64Not"
        | "I8Shl"  | "I16Shl"  | "I32Shl"  | "I64Shl"
        | "U8Shl"  | "U16Shl"  | "U32Shl"  | "U64Shl"
        | "I8Shr"  | "I16Shr"  | "I32Shr"  | "I64Shr"
        | "U8Shr"  | "U16Shr"  | "U32Shr"  | "U64Shr"
        // Bool logical ops
        | "BoolAnd" | "BoolOr" | "BoolXor" | "BoolNot"
        // Array / memory
        | "ArrayLen"
        | "PtrOffset" | "MakeArr" | "BorrowSlice" | "BorrowSliceMut" | "BorrowStr" | "ArrPtr" | "SizeOf" | "AlignOf" | "SizeOfValue" | "AlignOfValue"
        | "DropInPlace" | "Forget"
        | "AtomicU64Exchange" | "AtomicU64FetchAdd" | "AtomicU64FetchSub" | "AtomicU64Store"
    )
}

/// Infer the return type of an intrinsic based on its name and arguments
pub fn infer_intrinsic_return_type(name: &str, args: &[HirExpr]) -> Type {
    // Comparison operators return Bool
    if name.ends_with("Lt")
        || name.ends_with("Le")
        || name.ends_with("Gt")
        || name.ends_with("Ge")
        || name.ends_with("Eq")
        || name.ends_with("Ne")
    {
        return Type::Bool;
    }

    // Logical operators return Bool
    if name.ends_with("And") || name.ends_with("Or") || name.ends_with("Xor") {
        // But for integers, these are bitwise ops
        if name.starts_with("Bool") {
            return Type::Bool;
        }
        // For integer types, return the same type as first arg
        if !args.is_empty() {
            return args[0].ty.clone();
        }
    }

    match name {
        "ArrayLen" => return Type::I64,
        "DropInPlace" => return Type::Unit,
        "Forget" => return Type::Unit,
        "AtomicU64Store" => return Type::Unit,
        "AtomicU64Exchange" | "AtomicU64FetchAdd" | "AtomicU64FetchSub" => return Type::U64,
        // Pointer/raw memory intrinsics
        // PtrOffset: returns same pointer type as first arg (element-stride GEP)
        "PtrOffset" => {
            if !args.is_empty() {
                return args[0].ty.clone();
            }
            return Type::Pointer(Box::new(Type::U8));
        }
        // MakeArr: returns a slice matching the pointed-to element type.
        "MakeArr" => {
            if let Some(HirExpr {
                ty: Type::Pointer(inner),
                ..
            }) = args.first()
            {
                return Type::Slice(inner.clone());
            }
            return Type::Slice(Box::new(Type::U8));
        }
        // BorrowSlice: returns a borrowed slice matching the pointed-to element type.
        "BorrowSlice" | "BorrowSliceMut" => {
            let elem_ty = if let Some(HirExpr {
                ty: Type::Pointer(inner),
                ..
            }) = args.first()
            {
                inner.clone()
            } else {
                Box::new(Type::U8)
            };

            return Type::Reference {
                mutable: name == "BorrowSliceMut",
                inner: Box::new(Type::Slice(elem_ty)),
            };
        }
        "BorrowStr" => {
            return Type::Reference {
                mutable: false,
                inner: Box::new(Type::Str),
            };
        }
        // ArrPtr: returns *T where T is the element type of the slice argument.
        // The actual element type is resolved in control_flow.rs during unification;
        // this fallback is only hit if called outside that context.
        "ArrPtr" => {
            if !args.is_empty() {
                match &args[0].ty {
                    Type::Slice(elem_ty) => return Type::Pointer(elem_ty.clone()),
                    Type::Reference { inner, .. } => match inner.as_ref() {
                        Type::Slice(elem_ty) => return Type::Pointer(elem_ty.clone()),
                        Type::Str => return Type::Pointer(Box::new(Type::U8)),
                        _ => {}
                    },
                    _ => {}
                }
            }
            return Type::Pointer(Box::new(Type::U8));
        }
        "SizeOf" | "AlignOf" | "SizeOfValue" | "AlignOfValue" => return Type::I64,
        _ => {}
    }

    // Arithmetic operators return the same type as their first argument
    if !args.is_empty() {
        return args[0].ty.clone();
    }

    // Fallback: try to extract type from name
    if name.starts_with("I64") {
        return Type::I64;
    }
    if name.starts_with("I32") {
        return Type::I32;
    }
    if name.starts_with("I16") {
        return Type::I16;
    }
    if name.starts_with("I8") {
        return Type::I8;
    }
    if name.starts_with("U64") {
        return Type::U64;
    }
    if name.starts_with("U32") {
        return Type::U32;
    }
    if name.starts_with("U16") {
        return Type::U16;
    }
    if name.starts_with("U8") {
        return Type::U8;
    }
    if name.starts_with("F64") {
        return Type::F64;
    }
    if name.starts_with("F32") {
        return Type::F32;
    }

    Type::Error
}

/// Infer the expected argument types for an intrinsic call
pub fn infer_intrinsic_arg_types(name: &str) -> Vec<Type> {
    match name {
        // Unary: negation (signed integers + floats only)
        "I8Neg" => return vec![Type::I8],
        "I16Neg" => return vec![Type::I16],
        "I32Neg" => return vec![Type::I32],
        "I64Neg" => return vec![Type::I64],
        "F32Neg" => return vec![Type::F32],
        "F64Neg" => return vec![Type::F64],
        // Unary: bitwise NOT (integer types only)
        "I8Not" => return vec![Type::I8],
        "I16Not" => return vec![Type::I16],
        "I32Not" => return vec![Type::I32],
        "I64Not" => return vec![Type::I64],
        "U8Not" => return vec![Type::U8],
        "U16Not" => return vec![Type::U16],
        "U32Not" => return vec![Type::U32],
        "U64Not" => return vec![Type::U64],
        // Unary: boolean NOT
        "BoolNot" => return vec![Type::Bool],
        // Array / memory (generic - handled specially at call site)
        "ArrayLen" => return vec![],
        "ArrPtr" => return vec![],
        "DropInPlace" => return vec![],
        "Forget" => return vec![],
        "AtomicU64Exchange" | "AtomicU64FetchAdd" | "AtomicU64FetchSub" | "AtomicU64Store" => {
            return vec![Type::Pointer(Box::new(Type::U64)), Type::U64]
        }
        "BorrowStr" => return vec![Type::Pointer(Box::new(Type::U8)), Type::I64],
        "SizeOf" | "AlignOf" | "SizeOfValue" | "AlignOfValue" => return vec![],
        _ => {}
    }

    // Binary ops: extract type prefix from name (e.g., I64Add -> [I64, I64])
    let type_prefixes = [
        ("I64", Type::I64),
        ("I32", Type::I32),
        ("I16", Type::I16),
        ("I8", Type::I8),
        ("U64", Type::U64),
        ("U32", Type::U32),
        ("U16", Type::U16),
        ("U8", Type::U8),
        ("F64", Type::F64),
        ("F32", Type::F32),
        ("Bool", Type::Bool),
    ];

    for (prefix, ty) in &type_prefixes {
        if name.starts_with(prefix) {
            let op = &name[prefix.len()..];
            match op {
                "Add" | "Sub" | "Mul" | "Div" | "Mod" | "Lt" | "Le" | "Gt" | "Ge" | "Eq" | "Ne"
                | "And" | "Or" | "Xor" | "Shl" | "Shr" => return vec![ty.clone(), ty.clone()],
                _ => {}
            }
        }
    }

    vec![]
}
