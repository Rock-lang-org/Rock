//! Typed integer mechanisms; ordinary operator policy remains in std bodies.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum IntegerOp {
    Add, Sub, Mul, Div, Rem, Neg, Shl, Shr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum IntegerMode {
    Checked, Wrapping, Overflowing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct IntegerIntrinsic {
    pub op: IntegerOp,
    pub mode: IntegerMode,
}

macro_rules! integer_names {
    ($($op:ident => ($checked:literal, $wrapping:literal, $overflowing:literal)),* $(,)?) => {
        impl IntegerIntrinsic {
            pub fn from_name(name: &str) -> Option<Self> {
                Some(match name {
                    $($checked => Self { op: IntegerOp::$op, mode: IntegerMode::Checked },
                    $wrapping => Self { op: IntegerOp::$op, mode: IntegerMode::Wrapping },
                    $overflowing => Self { op: IntegerOp::$op, mode: IntegerMode::Overflowing },)*
                    _ => return None,
                })
            }

            pub fn as_str(self) -> &'static str {
                match (self.op, self.mode) {
                    $((IntegerOp::$op, IntegerMode::Checked) => $checked,
                    (IntegerOp::$op, IntegerMode::Wrapping) => $wrapping,
                    (IntegerOp::$op, IntegerMode::Overflowing) => $overflowing,)*
                }
            }
        }
    };
}

integer_names! {
    Add => ("IntCheckedAdd", "IntWrappingAdd", "IntOverflowingAdd"),
    Sub => ("IntCheckedSub", "IntWrappingSub", "IntOverflowingSub"),
    Mul => ("IntCheckedMul", "IntWrappingMul", "IntOverflowingMul"),
    Div => ("IntCheckedDiv", "IntWrappingDiv", "IntOverflowingDiv"),
    Rem => ("IntCheckedRem", "IntWrappingRem", "IntOverflowingRem"),
    Neg => ("IntCheckedNeg", "IntWrappingNeg", "IntOverflowingNeg"),
    Shl => ("IntCheckedShl", "IntWrappingShl", "IntOverflowingShl"),
    Shr => ("IntCheckedShr", "IntWrappingShr", "IntOverflowingShr"),
}

impl IntegerIntrinsic {
    pub fn arity(self) -> usize {
        if self.op == IntegerOp::Neg { 1 } else { 2 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_identities_round_trip_without_source_type_names() {
        for op in [IntegerOp::Add, IntegerOp::Sub, IntegerOp::Mul, IntegerOp::Div,
            IntegerOp::Rem, IntegerOp::Neg, IntegerOp::Shl, IntegerOp::Shr]
        {
            for mode in [IntegerMode::Checked, IntegerMode::Wrapping, IntegerMode::Overflowing] {
                let intrinsic = IntegerIntrinsic { op, mode };
                assert_eq!(IntegerIntrinsic::from_name(intrinsic.as_str()), Some(intrinsic));
                let bytes = bincode::serialize(&intrinsic).unwrap();
                assert_eq!(bincode::deserialize::<IntegerIntrinsic>(&bytes).unwrap(), intrinsic);
            }
        }
        assert!(IntegerIntrinsic::from_name("IntUncheckedAdd").is_none());
    }
}
