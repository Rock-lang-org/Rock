use serde::{Deserialize, Serialize};

use crate::hir::*;
use crate::types::Type;

use super::mechanisms::{mechanism, mechanism_arity, truncate, width};
use super::summarize::{scalar, summarize_block, SummaryContext};
use super::{Constant, Evaluation};

/// Identity-free scalar expression retained for diagnostics, never for codegen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ArithmeticSummary {
    Argument(usize),
    Integer(i128),
    Bool(bool),
    Ref(Box<Self>),
    Deref(Box<Self>),
    Cast(Box<Self>, ArithmeticType),
    Intrinsic(String, ArithmeticType, Vec<Self>),
    If(Box<Self>, Box<Self>, Box<Self>),
    TupleIndex(Box<Self>, u32),
    Sequence(Vec<Self>),
}

/// Only scalar types are admitted; summaries cannot smuggle nominal identities.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ArithmeticType {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    Bool,
}

impl ArithmeticType {
    pub(super) fn from_type(ty: &Type) -> Option<Self> {
        Some(match ty {
            Type::I8 => Self::I8,
            Type::I16 => Self::I16,
            Type::I32 => Self::I32,
            Type::I64 => Self::I64,
            Type::U8 => Self::U8,
            Type::U16 => Self::U16,
            Type::U32 => Self::U32,
            Type::U64 => Self::U64,
            Type::Bool => Self::Bool,
            _ => return None,
        })
    }

    fn ty(self) -> Type {
        match self {
            Self::I8 => Type::I8,
            Self::I16 => Type::I16,
            Self::I32 => Type::I32,
            Self::I64 => Type::I64,
            Self::U8 => Type::U8,
            Self::U16 => Type::U16,
            Self::U32 => Type::U32,
            Self::U64 => Type::U64,
            Self::Bool => Type::Bool,
        }
    }
}

impl ArithmeticSummary {
    pub(super) fn charge(&self, fuel: &mut usize) -> Option<()> {
        if *fuel == 0 {
            return None;
        }
        *fuel -= 1;
        match self {
            Self::Ref(inner)
            | Self::Deref(inner)
            | Self::Cast(inner, _)
            | Self::TupleIndex(inner, _) => inner.charge(fuel)?,
            Self::Intrinsic(_, _, args) | Self::Sequence(args) => {
                for arg in args {
                    arg.charge(fuel)?;
                }
            }
            Self::If(condition, yes, no) => {
                condition.charge(fuel)?;
                yes.charge(fuel)?;
                no.charge(fuel)?;
            }
            _ => {}
        }
        Some(())
    }

    pub(super) fn from_function<P: HirPhase>(
        function: &HirFunctionFor<P>,
        context: &SummaryContext<'_, P>,
    ) -> Option<Self> {
        if !function.is_method
            || function.is_unsafe
            || !function.generic_params.is_empty()
            || function.params.iter().any(|param| !scalar(&param.ty))
        {
            return None;
        }
        let bindings = function
            .params
            .iter()
            .enumerate()
            .map(|(index, param)| (param.local_id, Self::Argument(index)))
            .collect();
        let summary = summarize_block(&function.body, bindings, context, &mut 512, 0)?;
        summary.validate(function.params.len()).ok()?;
        Some(summary)
    }

    pub(crate) fn validate(&self, arity: usize) -> Result<(), String> {
        fn walk(value: &ArithmeticSummary, arity: usize, fuel: &mut usize, depth: usize) -> bool {
            if *fuel == 0 || depth >= 32 {
                return false;
            }
            *fuel -= 1;
            match value {
                ArithmeticSummary::Argument(index) => *index < arity,
                ArithmeticSummary::Integer(value) => {
                    (i64::MIN as i128..=u64::MAX as i128).contains(value)
                }
                ArithmeticSummary::Bool(_) => true,
                ArithmeticSummary::Ref(inner)
                | ArithmeticSummary::Deref(inner)
                | ArithmeticSummary::TupleIndex(inner, 0..=1) => {
                    walk(inner, arity, fuel, depth + 1)
                }
                ArithmeticSummary::Cast(inner, ty) => {
                    width(&ty.ty()).is_some() && walk(inner, arity, fuel, depth + 1)
                }
                ArithmeticSummary::Intrinsic(name, ty, args) => {
                    mechanism_arity(name, &ty.ty()) == Some(args.len())
                        && args.iter().all(|arg| walk(arg, arity, fuel, depth + 1))
                }
                ArithmeticSummary::If(condition, yes, no) => [condition, yes, no]
                    .into_iter()
                    .all(|expr| walk(expr, arity, fuel, depth + 1)),
                ArithmeticSummary::Sequence(values) => {
                    !values.is_empty()
                        && values
                            .iter()
                            .all(|value| walk(value, arity, fuel, depth + 1))
                }
                _ => false,
            }
        }
        if walk(self, arity, &mut 512, 0) {
            Ok(())
        } else {
            Err("invalid or excessive constant arithmetic summary".to_string())
        }
    }

    pub(super) fn evaluate(&self, args: &[Constant]) -> Evaluation {
        Ok(match self {
            Self::Argument(index) => args.get(*index).cloned(),
            Self::Integer(value) => Some(Constant::Integer(*value)),
            Self::Bool(value) => Some(Constant::Bool(*value)),
            Self::Ref(inner) => inner
                .evaluate(args)?
                .map(|value| Constant::Ref(Box::new(value))),
            Self::Deref(inner) => match inner.evaluate(args)? {
                Some(Constant::Ref(value)) => Some(*value),
                _ => None,
            },
            Self::Cast(inner, ty) => match inner.evaluate(args)? {
                Some(Constant::Integer(value)) => {
                    Some(Constant::Integer(truncate(value, &ty.ty())))
                }
                _ => None,
            },
            Self::TupleIndex(inner, index) => match inner.evaluate(args)? {
                Some(Constant::Tuple(values)) => values.get(*index as usize).cloned(),
                _ => None,
            },
            Self::If(condition, yes, no) => match condition.evaluate(args)? {
                Some(Constant::Bool(true)) => yes.evaluate(args)?,
                Some(Constant::Bool(false)) => no.evaluate(args)?,
                _ => None,
            },
            Self::Intrinsic(name, ty, operands) => {
                let mut values = Vec::new();
                for operand in operands {
                    let Some(value) = operand.evaluate(args)? else {
                        return Ok(None);
                    };
                    values.push(value);
                }
                mechanism(name, &ty.ty(), &values)?
            }
            Self::Sequence(values) => {
                let mut result = None;
                for value in values {
                    result = value.evaluate(args)?;
                    if result.is_none() {
                        return Ok(None);
                    }
                }
                result
            }
        })
    }
}
