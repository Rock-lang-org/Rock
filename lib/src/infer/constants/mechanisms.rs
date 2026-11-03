use crate::mir::integer::{IntegerIntrinsic, IntegerMode, IntegerOp};
use crate::types::Type;

use super::{Constant, Evaluation};

pub(super) fn width(ty: &Type) -> Option<(u32, bool)> {
    Some(match ty {
        Type::I8 => (8, true),
        Type::I16 => (16, true),
        Type::I32 => (32, true),
        Type::I64 => (64, true),
        Type::U8 => (8, false),
        Type::U16 => (16, false),
        Type::U32 => (32, false),
        Type::U64 => (64, false),
        _ => return None,
    })
}

pub(super) fn truncate(value: i128, ty: &Type) -> i128 {
    let Some((bits, signed)) = width(ty) else {
        return value;
    };
    let modulus = 1i128 << bits;
    let value = value.rem_euclid(modulus);
    if signed && value >= modulus / 2 {
        value - modulus
    } else {
        value
    }
}

fn primitive_suffix<'a>(name: &'a str, ty: &Type) -> Option<&'a str> {
    name.strip_prefix(match ty {
        Type::I8 => "I8",
        Type::I16 => "I16",
        Type::I32 => "I32",
        Type::I64 => "I64",
        Type::U8 => "U8",
        Type::U16 => "U16",
        Type::U32 => "U32",
        Type::U64 => "U64",
        Type::Bool => "Bool",
        _ => return None,
    })
}

pub(super) fn mechanism_arity(name: &str, ty: &Type) -> Option<usize> {
    if let Some(intrinsic) = IntegerIntrinsic::from_name(name) {
        return width(ty).map(|_| intrinsic.arity());
    }
    if !crate::lower::intrinsics::is_intrinsic_name(name) {
        return None;
    }
    Some(match primitive_suffix(name, ty)? {
        "Not" | "Neg" => 1,
        "Add" | "Sub" | "Mul" | "Div" | "Mod" | "Shl" | "Shr" | "And" | "Or" | "Xor" | "Eq"
        | "Ne" | "Lt" | "Le" | "Gt" | "Ge" => 2,
        _ => return None,
    })
}

pub(super) fn mechanism(name: &str, ty: &Type, values: &[Constant]) -> Evaluation {
    if mechanism_arity(name, ty) != Some(values.len()) {
        return Ok(None);
    }
    if let Some(intrinsic) = IntegerIntrinsic::from_name(name) {
        let Constant::Integer(a) = values[0] else {
            return Ok(None);
        };
        let b = if intrinsic.arity() == 1 {
            0
        } else {
            let Constant::Integer(b) = values[1] else {
                return Ok(None);
            };
            b
        };
        return integer(intrinsic, ty, a, b);
    }
    let Some(op) = primitive_suffix(name, ty) else {
        return Ok(None);
    };
    if let Constant::Bool(a) = values[0] {
        if *ty != Type::Bool {
            return Ok(None);
        }
        if op == "Not" {
            return Ok(Some(Constant::Bool(!a)));
        }
        let Some(Constant::Bool(b)) = values.get(1) else {
            return Ok(None);
        };
        return Ok(Some(Constant::Bool(match op {
            "And" => a & b,
            "Or" => a | b,
            "Xor" => a ^ b,
            "Eq" => a == *b,
            "Ne" => a != *b,
            _ => return Ok(None),
        })));
    }
    let Constant::Integer(a) = values[0] else {
        return Ok(None);
    };
    let b = match values.get(1) {
        Some(Constant::Integer(b)) => *b,
        None => 0,
        _ => return Ok(None),
    };
    let arithmetic = match op {
        "Add" => Some(IntegerOp::Add),
        "Sub" => Some(IntegerOp::Sub),
        "Mul" => Some(IntegerOp::Mul),
        "Div" => Some(IntegerOp::Div),
        "Mod" => Some(IntegerOp::Rem),
        "Neg" => Some(IntegerOp::Neg),
        "Shl" => Some(IntegerOp::Shl),
        "Shr" => Some(IntegerOp::Shr),
        _ => None,
    };
    if let Some(op) = arithmetic {
        return integer(
            IntegerIntrinsic {
                op,
                mode: if matches!(op, IntegerOp::Div | IntegerOp::Rem) {
                    IntegerMode::Checked
                } else {
                    IntegerMode::Wrapping
                },
            },
            ty,
            a,
            b,
        );
    }
    Ok(Some(match op {
        "Not" => Constant::Integer(truncate(!a, ty)),
        "And" => Constant::Integer(truncate(a & b, ty)),
        "Or" => Constant::Integer(truncate(a | b, ty)),
        "Xor" => Constant::Integer(truncate(a ^ b, ty)),
        "Eq" => Constant::Bool(a == b),
        "Ne" => Constant::Bool(a != b),
        "Lt" => Constant::Bool(a < b),
        "Le" => Constant::Bool(a <= b),
        "Gt" => Constant::Bool(a > b),
        "Ge" => Constant::Bool(a >= b),
        _ => return Ok(None),
    }))
}

pub(super) fn integer(intrinsic: IntegerIntrinsic, ty: &Type, a: i128, b: i128) -> Evaluation {
    let Some((bits, signed)) = width(ty) else {
        return Ok(None);
    };
    let min = if signed { -(1i128 << (bits - 1)) } else { 0 };
    let max = if signed {
        (1i128 << (bits - 1)) - 1
    } else {
        (1i128 << bits) - 1
    };
    if matches!(intrinsic.op, IntegerOp::Div | IntegerOp::Rem) && b == 0 {
        return Err("division by zero");
    }
    let (value, overflow) = match intrinsic.op {
        IntegerOp::Div | IntegerOp::Rem => {
            let overflow = signed && a == min && b == -1;
            let value = if intrinsic.op == IntegerOp::Div {
                a / b
            } else {
                a % b
            };
            (value, overflow)
        }
        IntegerOp::Shl | IntegerOp::Shr => {
            let count = b.rem_euclid(bits as i128) as u32;
            let value = if intrinsic.op == IntegerOp::Shl {
                a << count
            } else {
                a >> count
            };
            (value, b < 0 || b >= bits as i128)
        }
        op => {
            let exact = match op {
                IntegerOp::Add => a.checked_add(b),
                IntegerOp::Sub => a.checked_sub(b),
                IntegerOp::Mul => a.checked_mul(b),
                IntegerOp::Neg => a.checked_neg(),
                _ => unreachable!(),
            };
            // U64 multiplication can exceed i128; retain low bits without host overflow.
            let value = exact.unwrap_or_else(|| a.wrapping_mul(b));
            (value, exact.is_none_or(|value| value < min || value > max))
        }
    };
    if overflow && intrinsic.mode == IntegerMode::Checked {
        return Err("integer overflow");
    }
    let value = Constant::Integer(truncate(value, ty));
    Ok(Some(if intrinsic.mode == IntegerMode::Overflowing {
        Constant::Tuple(vec![value, Constant::Bool(overflow)])
    } else {
        value
    }))
}
