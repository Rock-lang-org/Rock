//! Bounded constant arithmetic checking through selected method bodies.
//!
//! Local variables and ordinary function calls remain runtime values. A method
//! application is constant only when its operands and the selected pure body are
//! known; operator spellings and dependency profiles supply no arithmetic meaning.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::diagnostic::DiagnosticCode;
use crate::hir::*;
use crate::ids::{DefId, HirLocalId};
use crate::lower::ResolveError;
use crate::mir::integer::{IntegerIntrinsic, IntegerMode, IntegerOp};
use crate::types::Type;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Constant {
    Integer(i128),
    Bool(bool),
    Ref(Box<Constant>),
    Tuple(Vec<Constant>),
}

type Environment = HashMap<HirLocalId, Constant>;
type Evaluation = Result<Option<Constant>, &'static str>;

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
    I8, I16, I32, I64, U8, U16, U32, U64, Bool,
}

impl ArithmeticType {
    fn from_type(ty: &Type) -> Option<Self> {
        Some(match ty {
            Type::I8 => Self::I8, Type::I16 => Self::I16, Type::I32 => Self::I32, Type::I64 => Self::I64,
            Type::U8 => Self::U8, Type::U16 => Self::U16, Type::U32 => Self::U32, Type::U64 => Self::U64,
            Type::Bool => Self::Bool,
            _ => return None,
        })
    }

    fn ty(self) -> Type {
        match self {
            Self::I8 => Type::I8, Self::I16 => Type::I16, Self::I32 => Type::I32, Self::I64 => Type::I64,
            Self::U8 => Type::U8, Self::U16 => Type::U16, Self::U32 => Type::U32, Self::U64 => Type::U64,
            Self::Bool => Type::Bool,
        }
    }
}

impl ArithmeticSummary {
    fn charge(&self, fuel: &mut usize) -> Option<()> {
        if *fuel == 0 { return None; }
        *fuel -= 1;
        match self {
            Self::Ref(inner) | Self::Deref(inner) | Self::Cast(inner, _)
            | Self::TupleIndex(inner, _) => inner.charge(fuel)?,
            Self::Intrinsic(_, _, args) | Self::Sequence(args) => {
                for arg in args { arg.charge(fuel)?; }
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

    pub(crate) fn from_function<P: HirPhase>(function: &HirFunctionFor<P>) -> Option<Self> {
        if !function.is_method || function.is_unsafe || !function.generic_params.is_empty()
            || function.params.iter().any(|param| !scalar(&param.ty))
        { return None; }
        let bindings = function.params.iter().enumerate()
            .map(|(index, param)| (param.local_id, Self::Argument(index))).collect();
        let summary = summarize_block(&function.body, bindings, &mut 512, 0)?;
        summary.validate(function.params.len()).ok()?;
        Some(summary)
    }

    pub(crate) fn validate(&self, arity: usize) -> Result<(), String> {
        fn walk(value: &ArithmeticSummary, arity: usize, fuel: &mut usize, depth: usize) -> bool {
            if *fuel == 0 || depth >= 32 { return false; }
            *fuel -= 1;
            match value {
                ArithmeticSummary::Argument(index) => *index < arity,
                ArithmeticSummary::Integer(value) => (i64::MIN as i128..=u64::MAX as i128).contains(value),
                ArithmeticSummary::Bool(_) => true,
                ArithmeticSummary::Ref(inner) | ArithmeticSummary::Deref(inner)
                | ArithmeticSummary::TupleIndex(inner, 0..=1) => walk(inner, arity, fuel, depth + 1),
                ArithmeticSummary::Cast(inner, ty) => width(&ty.ty()).is_some() && walk(inner, arity, fuel, depth + 1),
                ArithmeticSummary::Intrinsic(name, ty, args) => mechanism_arity(name, &ty.ty()) == Some(args.len())
                    && args.iter().all(|arg| walk(arg, arity, fuel, depth + 1)),
                ArithmeticSummary::If(condition, yes, no) => [condition, yes, no].into_iter()
                    .all(|expr| walk(expr, arity, fuel, depth + 1)),
                ArithmeticSummary::Sequence(values) => !values.is_empty()
                    && values.iter().all(|value| walk(value, arity, fuel, depth + 1)),
                _ => false,
            }
        }
        if walk(self, arity, &mut 512, 0) { Ok(()) }
        else { Err("invalid or excessive constant arithmetic summary".to_string()) }
    }

    fn evaluate(&self, args: &[Constant]) -> Evaluation {
        Ok(match self {
            Self::Argument(index) => args.get(*index).cloned(),
            Self::Integer(value) => Some(Constant::Integer(*value)),
            Self::Bool(value) => Some(Constant::Bool(*value)),
            Self::Ref(inner) => inner.evaluate(args)?.map(|value| Constant::Ref(Box::new(value))),
            Self::Deref(inner) => match inner.evaluate(args)? {
                Some(Constant::Ref(value)) => Some(*value),
                _ => None,
            },
            Self::Cast(inner, ty) => match inner.evaluate(args)? {
                Some(Constant::Integer(value)) => Some(Constant::Integer(truncate(value, &ty.ty()))),
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
                    let Some(value) = operand.evaluate(args)? else { return Ok(None); };
                    values.push(value);
                }
                mechanism(name, &ty.ty(), &values)?
            }
            Self::Sequence(values) => {
                let mut result = None;
                for value in values {
                    result = value.evaluate(args)?;
                    if result.is_none() { return Ok(None); }
                }
                result
            }
        })
    }
}

fn scalar(ty: &Type) -> bool {
    width(ty).is_some() || *ty == Type::Bool
        || matches!(ty, Type::Reference { mutable: false, inner } if width(inner).is_some() || **inner == Type::Bool)
}

fn summarize_block<P: HirPhase>(block: &HirBlockFor<P>, mut env: HashMap<HirLocalId, ArithmeticSummary>, fuel: &mut usize, depth: usize) -> Option<ArithmeticSummary> {
    let mut result = None;
    let mut sequence = Vec::new();
    for stmt in &block.stmts {
        match stmt {
            HirStmtFor::Let { local_id, value, mutable: false, .. } => {
                let value = summarize(value, &env, fuel, depth)?;
                sequence.push(value.clone());
                env.insert(*local_id, value);
                result = None;
            }
            HirStmtFor::Expr(expr) => {
                let value = summarize(expr, &env, fuel, depth)?;
                sequence.push(value.clone());
                result = Some(value);
            }
            _ => return None,
        }
    }
    result.map(|value| if sequence.len() == 1 { value } else { ArithmeticSummary::Sequence(sequence) })
}

fn summarize<P: HirPhase>(expr: &HirExprFor<P>, env: &HashMap<HirLocalId, ArithmeticSummary>, fuel: &mut usize, depth: usize) -> Option<ArithmeticSummary> {
    if *fuel == 0 || depth >= 32 { return None; }
    *fuel -= 1;
    let inner = |expr, fuel: &mut usize| summarize(expr, env, fuel, depth + 1).map(Box::new);
    Some(match &expr.kind {
        HirExprKindFor::IntLiteral(value) if width(&expr.ty).is_some() => ArithmeticSummary::Integer(truncate(*value as i128, &expr.ty)),
        HirExprKindFor::BoolLiteral(value) => ArithmeticSummary::Bool(*value),
        HirExprKindFor::ResolvedVar(reference) => match reference.target {
            HirVarTarget::Local(id) => {
                let summary = env.get(&id)?;
                summary.charge(fuel)?;
                summary.clone()
            }
            _ => return None,
        },
        HirExprKindFor::Ref(false, value) => ArithmeticSummary::Ref(inner(value, fuel)?),
        HirExprKindFor::Deref(value) if matches!(value.ty, Type::Reference { mutable: false, .. }) => ArithmeticSummary::Deref(inner(value, fuel)?),
        HirExprKindFor::Cast(value, ty) if width(ty).is_some() => ArithmeticSummary::Cast(inner(value, fuel)?, ArithmeticType::from_type(ty)?),
        HirExprKindFor::TupleIndex(value, index) => ArithmeticSummary::TupleIndex(inner(value, fuel)?, *index),
        HirExprKindFor::Block(block) => summarize_block(block, env.clone(), fuel, depth + 1)?,
        HirExprKindFor::If { condition, then_branch, else_branch: Some(no) } => ArithmeticSummary::If(
            inner(condition, fuel)?,
            Box::new(summarize_block(then_branch, env.clone(), fuel, depth + 1)?),
            Box::new(summarize_block(no, env.clone(), fuel, depth + 1)?),
        ),
        HirExprKindFor::Intrinsic { name, args } => ArithmeticSummary::Intrinsic(
            name.clone(), ArithmeticType::from_type(&args.first()?.ty)?,
            args.iter().map(|arg| summarize(arg, env, fuel, depth + 1)).collect::<Option<_>>()?,
        ),
        _ => return None,
    })
}

struct Check<'a> {
    program: &'a HirProgram,
    summaries: &'a HashMap<DefId, ArithmeticSummary>,
}

impl std::ops::Deref for Check<'_> {
    type Target = HirProgram;
    fn deref(&self) -> &Self::Target { self.program }
}

pub(super) fn validate(program: &HirProgram, current: &BTreeSet<DefId>, summaries: &HashMap<DefId, ArithmeticSummary>) -> Vec<ResolveError> {
    let program = &Check { program, summaries };
    let mut errors = Vec::new();
    let mut functions = program.functions.values()
        .chain(program.impls.values().flat_map(|imp| imp.methods.values()))
        .chain(program.traits.values().flat_map(|trt| trt.methods.values()))
        .filter(|function| current.contains(&function.id))
        .collect::<Vec<_>>();
    functions.sort_by_key(|function| function.id);
    for function in functions {
        scan_block(program, &function.body, &mut errors);
    }
    errors
}

fn scan_block(program: &Check<'_>, block: &HirBlock, errors: &mut Vec<ResolveError>) {
    for stmt in &block.stmts {
        match stmt {
            HirStmt::Let { value, .. } | HirStmt::Expr(value)
            | HirStmt::Return(Some(value)) | HirStmt::Break(Some(value)) => scan(program, value, errors),
            _ => {}
        }
    }
}

fn scan(program: &Check<'_>, expr: &HirExpr, errors: &mut Vec<ResolveError>) {
    let previous_errors = errors.len();
    match &expr.kind {
        HirExprKind::BinOp(_, lhs, rhs) | HirExprKind::Assign(lhs, rhs) => {
            scan(program, lhs, errors);
            scan(program, rhs, errors);
        }
        HirExprKind::Call(function, args, _) => {
            scan(program, function, errors);
            for arg in args { scan(program, arg, errors); }
        }
        HirExprKind::MethodCall(receiver, _, args, _, _) => {
            scan(program, receiver, errors);
            for arg in args { scan(program, arg, errors); }
        }
        HirExprKind::UnaryOp(_, inner) | HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::TupleIndex(inner, _) | HirExprKind::ArrayRepeat(inner, _)
        | HirExprKind::Ref(_, inner) | HirExprKind::Deref(inner)
        | HirExprKind::Cast(inner, _) | HirExprKind::Try { expr: inner, .. } => scan(program, inner, errors),
        HirExprKind::ArrayLiteral(args) | HirExprKind::TupleLiteral(args)
        | HirExprKind::EnumVariant(_, _, args, _) | HirExprKind::Intrinsic { args, .. } => {
            for arg in args { scan(program, arg, errors); }
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields { scan(program, &field.value, errors); }
        }
        HirExprKind::If { condition, then_branch, else_branch } => {
            scan(program, condition, errors);
            scan_block(program, then_branch, errors);
            if let Some(block) = else_branch { scan_block(program, block, errors); }
        }
        HirExprKind::Match { scrutinee, arms } => {
            scan(program, scrutinee, errors);
            for arm in arms {
                if let Some(guard) = &arm.guard { scan(program, guard, errors); }
                scan_block(program, &arm.body, errors);
            }
        }
        HirExprKind::While { condition, body } => {
            scan(program, condition, errors);
            scan_block(program, body, errors);
        }
        HirExprKind::For { iter, body, .. } => {
            scan(program, iter, errors);
            scan_block(program, body, errors);
        }
        HirExprKind::Loop(body) | HirExprKind::Block(body)
        | HirExprKind::UnsafeBlock(body) | HirExprKind::Lambda { body, .. } => scan_block(program, body, errors),
        _ => {}
    }
    // Diagnose the innermost failing application, not a surrounding print/call
    // or the dependency's implementation; valid operands can still fail here.
    if errors.len() == previous_errors
        && matches!(expr.kind, HirExprKind::MethodCall(..) | HirExprKind::Intrinsic { .. })
    {
        if let Err(reason) = evaluate(program, expr, &Environment::new(), &mut 512, 0) {
            errors.push(ResolveError::with_span_code(
                format!("invalid constant integer arithmetic: {reason}"),
                arithmetic_span(expr),
                DiagnosticCode::Type,
            ));
        }
    }
}

fn arithmetic_span(expr: &HirExpr) -> crate::lexer::Span {
    let mut span = expr.span.clone();
    let mut include = |child: &HirExpr| {
        let child = arithmetic_span(child);
        if child.file_path == span.file_path {
            span.start = span.start.min(child.start);
            span.end = span.end.max(child.end);
        }
    };
    match &expr.kind {
        HirExprKind::MethodCall(receiver, _, args, _, _) => {
            include(receiver);
            for arg in args { include(arg); }
        }
        HirExprKind::Intrinsic { args, .. } => {
            for arg in args { include(arg); }
        }
        HirExprKind::Cast(inner, _) | HirExprKind::Ref(_, inner)
        | HirExprKind::Deref(inner) | HirExprKind::TupleIndex(inner, _) => include(inner),
        _ => {}
    }
    span
}

fn evaluate(program: &Check<'_>, expr: &HirExpr, env: &Environment, fuel: &mut usize, depth: usize) -> Evaluation {
    if *fuel == 0 || depth >= 32 { return Ok(None); }
    *fuel -= 1;
    let eval = |expr: &HirExpr, fuel: &mut usize| evaluate(program, expr, env, fuel, depth + 1);
    Ok(match &expr.kind {
        HirExprKind::IntLiteral(value) => width(&expr.ty).map(|_| Constant::Integer(truncate(*value as i128, &expr.ty))),
        HirExprKind::BoolLiteral(value) => Some(Constant::Bool(*value)),
        HirExprKind::ResolvedVar(reference) => match reference.target {
            HirVarTarget::Local(id) => env.get(&id).cloned(),
            _ => None,
        },
        HirExprKind::Cast(inner, ty) => match eval(inner, fuel)? {
            Some(Constant::Integer(value)) if width(ty).is_some() => Some(Constant::Integer(truncate(value, ty))),
            _ => None,
        },
        HirExprKind::Ref(false, inner) => eval(inner, fuel)?.map(|value| Constant::Ref(Box::new(value))),
        HirExprKind::Deref(inner) => match eval(inner, fuel)? {
            Some(Constant::Ref(value)) => Some(*value),
            _ => None,
        },
        HirExprKind::TupleIndex(inner, index) => match eval(inner, fuel)? {
            Some(Constant::Tuple(values)) => values.get(*index as usize).cloned(),
            _ => None,
        },
        HirExprKind::If { condition, then_branch, else_branch } => {
            match eval(condition, fuel)? {
                Some(Constant::Bool(true)) => evaluate_block(program, then_branch, env.clone(), fuel, depth + 1)?,
                Some(Constant::Bool(false)) => match else_branch {
                    Some(block) => evaluate_block(program, block, env.clone(), fuel, depth + 1)?,
                    None => None,
                },
                _ => None,
            }
        }
        HirExprKind::Block(block) => evaluate_block(program, block, env.clone(), fuel, depth + 1)?,
        HirExprKind::MethodCall(receiver, _, args, _, Some(target)) => {
            let HirSelectedMethodTarget::ImplMethod { impl_id, method_id, .. } = target.target else { return Ok(None); };
            let Some(method) = program.impls.get(&impl_id)
                .and_then(|imp| imp.methods.values().find(|method| method.id == method_id)) else { return Ok(None); };
            if method.is_unsafe || method.params.len() != args.len() + 1 { return Ok(None); }
            let Some(mut receiver) = eval(receiver, fuel)? else { return Ok(None); };
            if matches!(method.params[0].ty, Type::Reference { mutable: false, .. }) && !matches!(receiver, Constant::Ref(_)) {
                receiver = Constant::Ref(Box::new(receiver));
            }
            let mut bindings = Environment::new();
            let mut values = vec![receiver.clone()];
            bindings.insert(method.params[0].local_id, receiver);
            for (arg, param) in args.iter().zip(&method.params[1..]) {
                let Some(value) = eval(arg, fuel)? else { return Ok(None); };
                values.push(value.clone());
                bindings.insert(param.local_id, value);
            }
            if method.body.stmts.is_empty() {
                match program.summaries.get(&method_id) {
                    Some(summary) => summary.evaluate(&values)?,
                    None => None,
                }
            } else {
                evaluate_block(program, &method.body, bindings, fuel, depth + 1)?
            }
        }
        HirExprKind::Intrinsic { name, args } => {
            let Some(first) = args.first() else { return Ok(None); };
            if mechanism_arity(name, &first.ty) != Some(args.len()) { return Ok(None); }
            let mut values = Vec::new();
            for arg in args {
                let Some(value) = eval(arg, fuel)? else { return Ok(None); };
                values.push(value);
            }
            mechanism(name, &first.ty, &values)?
        }
        // Do not execute arbitrary functions, mutable storage, loops or unsafe code.
        _ => None,
    })
}

fn evaluate_block(program: &Check<'_>, block: &HirBlock, mut env: Environment, fuel: &mut usize, depth: usize) -> Evaluation {
    let mut result = None;
    for stmt in &block.stmts {
        match stmt {
            HirStmt::Let { local_id, value, mutable: false, .. } => {
                let Some(value) = evaluate(program, value, &env, fuel, depth)? else { return Ok(None); };
                env.insert(*local_id, value);
                result = None;
            }
            HirStmt::Expr(expr) => {
                result = evaluate(program, expr, &env, fuel, depth)?;
                if result.is_none() { return Ok(None); }
            }
            _ => return Ok(None),
        }
    }
    Ok(result)
}

fn width(ty: &Type) -> Option<(u32, bool)> {
    Some(match ty {
        Type::I8 => (8, true), Type::I16 => (16, true), Type::I32 => (32, true), Type::I64 => (64, true),
        Type::U8 => (8, false), Type::U16 => (16, false), Type::U32 => (32, false), Type::U64 => (64, false),
        _ => return None,
    })
}

fn truncate(value: i128, ty: &Type) -> i128 {
    let Some((bits, signed)) = width(ty) else { return value; };
    let modulus = 1i128 << bits;
    let value = value.rem_euclid(modulus);
    if signed && value >= modulus / 2 { value - modulus } else { value }
}

fn primitive_suffix<'a>(name: &'a str, ty: &Type) -> Option<&'a str> {
    name.strip_prefix(match ty {
        Type::I8 => "I8", Type::I16 => "I16", Type::I32 => "I32", Type::I64 => "I64",
        Type::U8 => "U8", Type::U16 => "U16", Type::U32 => "U32", Type::U64 => "U64",
        Type::Bool => "Bool",
        _ => return None,
    })
}

fn mechanism_arity(name: &str, ty: &Type) -> Option<usize> {
    if let Some(intrinsic) = IntegerIntrinsic::from_name(name) {
        return width(ty).map(|_| intrinsic.arity());
    }
    if !crate::lower::intrinsics::is_intrinsic_name(name) { return None; }
    Some(match primitive_suffix(name, ty)? {
        "Not" | "Neg" => 1,
        "Add" | "Sub" | "Mul" | "Div" | "Mod" | "Shl" | "Shr"
        | "And" | "Or" | "Xor" | "Eq" | "Ne" | "Lt" | "Le" | "Gt" | "Ge" => 2,
        _ => return None,
    })
}

fn mechanism(name: &str, ty: &Type, values: &[Constant]) -> Evaluation {
    if mechanism_arity(name, ty) != Some(values.len()) { return Ok(None); }
    if let Some(intrinsic) = IntegerIntrinsic::from_name(name) {
        let Constant::Integer(a) = values[0] else { return Ok(None); };
        let b = if intrinsic.arity() == 1 { 0 } else {
            let Constant::Integer(b) = values[1] else { return Ok(None); };
            b
        };
        return integer(intrinsic, ty, a, b);
    }
    let Some(op) = primitive_suffix(name, ty) else { return Ok(None); };
    if let Constant::Bool(a) = values[0] {
        if *ty != Type::Bool { return Ok(None); }
        if op == "Not" { return Ok(Some(Constant::Bool(!a))); }
        let Some(Constant::Bool(b)) = values.get(1) else { return Ok(None); };
        return Ok(Some(Constant::Bool(match op {
            "And" => a & b, "Or" => a | b, "Xor" => a ^ b,
            "Eq" => a == *b, "Ne" => a != *b,
            _ => return Ok(None),
        })));
    }
    let Constant::Integer(a) = values[0] else { return Ok(None); };
    let b = match values.get(1) { Some(Constant::Integer(b)) => *b, None => 0, _ => return Ok(None) };
    let arithmetic = match op {
        "Add" => Some(IntegerOp::Add), "Sub" => Some(IntegerOp::Sub), "Mul" => Some(IntegerOp::Mul),
        "Div" => Some(IntegerOp::Div), "Mod" => Some(IntegerOp::Rem), "Neg" => Some(IntegerOp::Neg),
        "Shl" => Some(IntegerOp::Shl), "Shr" => Some(IntegerOp::Shr), _ => None,
    };
    if let Some(op) = arithmetic {
        return integer(IntegerIntrinsic { op, mode: if matches!(op, IntegerOp::Div | IntegerOp::Rem) {
            IntegerMode::Checked
        } else { IntegerMode::Wrapping } }, ty, a, b);
    }
    Ok(Some(match op {
        "Not" => Constant::Integer(truncate(!a, ty)),
        "And" => Constant::Integer(truncate(a & b, ty)),
        "Or" => Constant::Integer(truncate(a | b, ty)),
        "Xor" => Constant::Integer(truncate(a ^ b, ty)),
        "Eq" => Constant::Bool(a == b), "Ne" => Constant::Bool(a != b),
        "Lt" => Constant::Bool(a < b), "Le" => Constant::Bool(a <= b),
        "Gt" => Constant::Bool(a > b), "Ge" => Constant::Bool(a >= b),
        _ => return Ok(None),
    }))
}

fn integer(intrinsic: IntegerIntrinsic, ty: &Type, a: i128, b: i128) -> Evaluation {
    let Some((bits, signed)) = width(ty) else { return Ok(None); };
    let min = if signed { -(1i128 << (bits - 1)) } else { 0 };
    let max = if signed { (1i128 << (bits - 1)) - 1 } else { (1i128 << bits) - 1 };
    if matches!(intrinsic.op, IntegerOp::Div | IntegerOp::Rem) && b == 0 {
        return Err("division by zero");
    }
    let (value, overflow) = match intrinsic.op {
        IntegerOp::Div | IntegerOp::Rem => {
            let overflow = signed && a == min && b == -1;
            let value = if intrinsic.op == IntegerOp::Div { a / b } else { a % b };
            (value, overflow)
        }
        IntegerOp::Shl | IntegerOp::Shr => {
            let count = b.rem_euclid(bits as i128) as u32;
            let value = if intrinsic.op == IntegerOp::Shl { a << count } else { a >> count };
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
    if overflow && intrinsic.mode == IntegerMode::Checked { return Err("integer overflow"); }
    let value = Constant::Integer(truncate(value, ty));
    Ok(Some(if intrinsic.mode == IntegerMode::Overflowing {
        Constant::Tuple(vec![value, Constant::Bool(overflow)])
    } else { value }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_constants_preserve_width_and_report_only_trapping_modes() {
        for ty in [Type::I8, Type::I16, Type::I32, Type::I64,
            Type::U8, Type::U16, Type::U32, Type::U64]
        {
            let (bits, signed) = width(&ty).unwrap();
            let min = if signed { -(1i128 << (bits - 1)) } else { 0 };
            let max = if signed { (1i128 << (bits - 1)) - 1 } else { (1i128 << bits) - 1 };
            for (op, a, b, wrapped) in [
                (IntegerOp::Add, max, 1, min),
                (IntegerOp::Sub, min, 1, max),
                (IntegerOp::Shl, 1, bits as i128, 1),
                (IntegerOp::Shr, 1, -1, 0),
            ] {
                assert!(integer(IntegerIntrinsic { op, mode: IntegerMode::Checked }, &ty, a, b).is_err());
                assert_eq!(integer(IntegerIntrinsic { op, mode: IntegerMode::Wrapping }, &ty, a, b),
                    Ok(Some(Constant::Integer(wrapped))));
                assert_eq!(integer(IntegerIntrinsic { op, mode: IntegerMode::Overflowing }, &ty, a, b),
                    Ok(Some(Constant::Tuple(vec![Constant::Integer(wrapped), Constant::Bool(true)]))));
            }
            for op in [IntegerOp::Div, IntegerOp::Rem] {
                for mode in [IntegerMode::Checked, IntegerMode::Wrapping, IntegerMode::Overflowing] {
                    assert!(integer(IntegerIntrinsic { op, mode }, &ty, 1, 0).is_err());
                }
                if signed {
                    assert!(integer(IntegerIntrinsic { op, mode: IntegerMode::Checked }, &ty, min, -1).is_err());
                    let expected = if op == IntegerOp::Div { min } else { 0 };
                    assert_eq!(integer(IntegerIntrinsic { op, mode: IntegerMode::Wrapping }, &ty, min, -1),
                        Ok(Some(Constant::Integer(expected))));
                }
            }
        }
    }

    #[test]
    fn u64_constant_multiplication_never_overflows_the_host_evaluator() {
        let max = u64::MAX as i128;
        assert_eq!(integer(IntegerIntrinsic { op: IntegerOp::Mul, mode: IntegerMode::Overflowing }, &Type::U64, max, max),
            Ok(Some(Constant::Tuple(vec![Constant::Integer(1), Constant::Bool(true)]))));
        assert!(integer(IntegerIntrinsic { op: IntegerOp::Mul, mode: IntegerMode::Checked }, &Type::U64, max, max).is_err());
    }

    #[test]
    fn arithmetic_summaries_bound_resources_and_select_only_the_taken_policy() {
        let failure = ArithmeticSummary::Intrinsic("IntCheckedAdd".to_string(), ArithmeticType::I8,
            vec![ArithmeticSummary::Integer(127), ArithmeticSummary::Integer(1)]);
        let selected = ArithmeticSummary::If(Box::new(ArithmeticSummary::Bool(false)),
            Box::new(failure), Box::new(ArithmeticSummary::Integer(0)));
        selected.validate(0).unwrap();
        assert_eq!(selected.evaluate(&[]), Ok(Some(Constant::Integer(0))));
        let mut deep = ArithmeticSummary::Integer(0);
        for _ in 0..33 { deep = ArithmeticSummary::Ref(Box::new(deep)); }
        assert!(deep.validate(0).is_err());
        assert_eq!(mechanism("I8Neg", &Type::I8, &[Constant::Bool(true)]), Ok(None));
    }
}
