use std::collections::HashMap;
use std::ops::Deref;

use super::{
    AcceptedHir, HirBlock, HirBlockFor, HirExpr, HirExprFor, HirExprKind, HirExprKindFor,
    HirFunction, HirFunctionFor, HirImpl, HirImplFor, HirMatchArm, HirMatchArmFor, HirProgram,
    HirProgramFor, HirStmt, HirStmtFor, HirStructLiteralField, HirStructLiteralFieldFor, HirTrait,
    HirTraitFor,
};

pub type AcceptedHirExpr = HirExprFor<AcceptedHir>;
pub type AcceptedHirBlock = HirBlockFor<AcceptedHir>;
pub type AcceptedHirFunction = HirFunctionFor<AcceptedHir>;
pub type AcceptedHirTrait = HirTraitFor<AcceptedHir>;
pub type AcceptedHirImpl = HirImplFor<AcceptedHir>;

#[derive(Debug, Clone)]
pub struct AcceptedHirProgram {
    program: HirProgramFor<AcceptedHir>,
}

impl TryFrom<HirProgram> for AcceptedHirProgram {
    type Error = Vec<String>;

    fn try_from(program: HirProgram) -> Result<Self, Self::Error> {
        let mut errors = program.validate_method_authorities();
        errors.extend(super::language_items::validate_language_items(&program));
        errors.extend(program.validate_accepted_types());
        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(Self {
            program: convert_program(program)?,
        })
    }
}

impl AcceptedHirProgram {
    #[cfg(test)]
    pub(crate) fn revalidate_for_test(
        program: HirProgramFor<AcceptedHir>,
    ) -> Result<Self, Vec<String>> {
        Self::try_from(unresolve_program(program))
    }

    pub fn program(&self) -> &HirProgramFor<AcceptedHir> {
        &self.program
    }

    #[cfg(test)]
    pub(crate) fn program_mut_for_test(&mut self) -> &mut HirProgramFor<AcceptedHir> {
        &mut self.program
    }

    pub(crate) fn into_program(self) -> HirProgramFor<AcceptedHir> {
        self.program
    }
}

impl Deref for AcceptedHirProgram {
    type Target = HirProgramFor<AcceptedHir>;

    fn deref(&self) -> &Self::Target {
        &self.program
    }
}

fn convert_program(program: HirProgram) -> Result<HirProgramFor<AcceptedHir>, Vec<String>> {
    let accepted = HirProgramFor {
        functions: convert_map(program.functions, convert_function)?,
        structs: program.structs,
        enums: program.enums,
        traits: convert_map(program.traits, convert_trait)?,
        impls: convert_map(program.impls, convert_impl)?,
        externs: program.externs,
        type_aliases: program.type_aliases,
        names: program.names,
        order: program.order,
        language_items: program.language_items,
        indexes: program.indexes,
    };
    Ok(accepted)
}

fn convert_map<T, U>(
    input: HashMap<super::DefId, T>,
    mut convert: impl FnMut(T) -> Result<U, Vec<String>>,
) -> Result<HashMap<super::DefId, U>, Vec<String>> {
    input
        .into_iter()
        .map(|(id, value)| convert(value).map(|value| (id, value)))
        .collect()
}

fn convert_function(function: HirFunction) -> Result<AcceptedHirFunction, Vec<String>> {
    Ok(HirFunctionFor {
        id: function.id,
        name: function.name,
        generic_params: function.generic_params,
        generic_bounds: function.generic_bounds,
        params: function.params,
        ret_type: function.ret_type,
        body: convert_block(function.body)?,
        is_curried: function.is_curried,
        is_method: function.is_method,
        self_receiver: function.self_receiver,
        is_unsafe: function.is_unsafe,
    })
}

fn convert_trait(trait_def: HirTrait) -> Result<AcceptedHirTrait, Vec<String>> {
    Ok(HirTraitFor {
        id: trait_def.id,
        name: trait_def.name,
        generic_params: trait_def.generic_params,
        target: trait_def.target,
        predicates: trait_def.predicates,
        associated_types: trait_def.associated_types,
        methods: trait_def
            .methods
            .into_iter()
            .map(|(name, function)| convert_function(function).map(|function| (name, function)))
            .collect::<Result<_, _>>()?,
        signatures: trait_def.signatures,
    })
}

fn convert_impl(imp: HirImpl) -> Result<AcceptedHirImpl, Vec<String>> {
    Ok(HirImplFor {
        id: imp.id,
        owner: imp.owner,
        type_name: imp.type_name,
        type_generics: imp.type_generics,
        receiver_pattern: imp.receiver_pattern,
        trait_name: imp.trait_name,
        trait_id: imp.trait_id,
        trait_generics: imp.trait_generics,
        trait_arg_types: imp.trait_arg_types,
        associated_types: imp.associated_types,
        bounds: imp.bounds,
        methods: imp
            .methods
            .into_iter()
            .map(|(name, function)| convert_function(function).map(|function| (name, function)))
            .collect::<Result<_, _>>()?,
    })
}

fn convert_block(block: HirBlock) -> Result<AcceptedHirBlock, Vec<String>> {
    Ok(HirBlockFor {
        stmts: block
            .stmts
            .into_iter()
            .map(convert_stmt)
            .collect::<Result<_, _>>()?,
        ty: block.ty,
    })
}

fn convert_stmt(stmt: HirStmt) -> Result<HirStmtFor<AcceptedHir>, Vec<String>> {
    Ok(match stmt {
        HirStmt::Let {
            name,
            local_id,
            ty,
            value,
            mutable,
        } => HirStmtFor::Let {
            name,
            local_id,
            ty,
            value: convert_expr(value)?,
            mutable,
        },
        HirStmt::Expr(expr) => HirStmtFor::Expr(convert_expr(expr)?),
        HirStmt::Return(expr) => HirStmtFor::Return(expr.map(convert_expr).transpose()?),
        HirStmt::Break(expr) => HirStmtFor::Break(expr.map(convert_expr).transpose()?),
        HirStmt::Continue => HirStmtFor::Continue,
    })
}

fn convert_expr(expr: HirExpr) -> Result<AcceptedHirExpr, Vec<String>> {
    let ty = expr.ty;
    let span = expr.span;
    let kind = match expr.kind {
        HirExprKind::Open { source, binding, body } => HirExprKindFor::Open {
            source: Box::new(convert_expr(*source)?), binding, body: convert_block(body)?,
        },
        HirExprKind::OwnedObjectCall { owner, args, call } => HirExprKindFor::OwnedObjectCall {
            owner: Box::new(convert_expr(*owner)?),
            args: convert_exprs(args)?,
            call,
        },
        HirExprKind::IntLiteral(value) => HirExprKindFor::IntLiteral(value),
        HirExprKind::FloatLiteral(value) => HirExprKindFor::FloatLiteral(value),
        HirExprKind::BoolLiteral(value) => HirExprKindFor::BoolLiteral(value),
        HirExprKind::StringLiteral(value) => HirExprKindFor::StringLiteral(value),
        HirExprKind::CharLiteral(value) => HirExprKindFor::CharLiteral(value),
        HirExprKind::ArrayLiteral(exprs) => HirExprKindFor::ArrayLiteral(convert_exprs(exprs)?),
        HirExprKind::ArrayRepeat(value, len) => {
            HirExprKindFor::ArrayRepeat(Box::new(convert_expr(*value)?), len)
        }
        HirExprKind::TupleLiteral(exprs) => HirExprKindFor::TupleLiteral(convert_exprs(exprs)?),
        HirExprKind::Unit => HirExprKindFor::Unit,
        HirExprKind::Var(name) => HirExprKindFor::Var(name),
        HirExprKind::ResolvedVar(reference) => HirExprKindFor::ResolvedVar(reference),
        HirExprKind::FieldAccess(base, name, location) => {
            HirExprKindFor::FieldAccess(Box::new(convert_expr(*base)?), name, location)
        }
        HirExprKind::TupleIndex(base, index) => {
            HirExprKindFor::TupleIndex(Box::new(convert_expr(*base)?), index)
        }
        HirExprKind::BinOp(op, left, right) => HirExprKindFor::BinOp(
            op,
            Box::new(convert_expr(*left)?),
            Box::new(convert_expr(*right)?),
        ),
        HirExprKind::UnaryOp(op, inner) => {
            HirExprKindFor::UnaryOp(op, Box::new(convert_expr(*inner)?))
        }
        HirExprKind::Call(callee, args, target) => HirExprKindFor::Call(
            Box::new(convert_expr(*callee)?),
            convert_exprs(args)?,
            target,
        ),
        HirExprKind::MethodCall(receiver, name, args, mode, target) => HirExprKindFor::MethodCall(
            Box::new(convert_expr(*receiver)?),
            name,
            convert_exprs(args)?,
            mode,
            target.ok_or_else(|| vec!["accepted method call lacks authority".to_string()])?,
        ),
        HirExprKind::Try {
            expr,
            branch_method,
            branch_target,
            branch_self_receiver,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            control_flow_enum,
            break_variant,
            continue_variant,
        } => HirExprKindFor::Try {
            expr: Box::new(convert_expr(*expr)?),
            branch_method: branch_method
                .ok_or_else(|| vec!["accepted Try lacks branch authority".to_string()])?,
            branch_target,
            branch_self_receiver,
            from_residual_target: from_residual_target
                .ok_or_else(|| vec!["accepted Try lacks residual authority".to_string()])?,
            output_ty,
            residual_ty,
            return_ty,
            control_flow_enum,
            break_variant,
            continue_variant,
        },
        HirExprKind::StructLiteral(name, id, fields) => HirExprKindFor::StructLiteral(
            name,
            id,
            fields
                .into_iter()
                .map(convert_struct_field)
                .collect::<Result<_, _>>()?,
        ),
        HirExprKind::EnumVariant(owner, variant, args, location) => {
            HirExprKindFor::EnumVariant(owner, variant, convert_exprs(args)?, location)
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => HirExprKindFor::If {
            condition: Box::new(convert_expr(*condition)?),
            then_branch: convert_block(then_branch)?,
            else_branch: else_branch.map(convert_block).transpose()?,
        },
        HirExprKind::Match { scrutinee, arms } => HirExprKindFor::Match {
            scrutinee: Box::new(convert_expr(*scrutinee)?),
            arms: arms
                .into_iter()
                .map(convert_match_arm)
                .collect::<Result<_, _>>()?,
        },
        HirExprKind::While { condition, body } => HirExprKindFor::While {
            condition: Box::new(convert_expr(*condition)?),
            body: convert_block(body)?,
        },
        HirExprKind::For {
            var,
            local_id,
            iter,
            body,
        } => HirExprKindFor::For {
            var,
            local_id,
            iter: Box::new(convert_expr(*iter)?),
            body: convert_block(body)?,
        },
        HirExprKind::Loop(body) => HirExprKindFor::Loop(convert_block(body)?),
        HirExprKind::Block(body) => HirExprKindFor::Block(convert_block(body)?),
        HirExprKind::UnsafeBlock(body) => HirExprKindFor::Block(convert_block(body)?),
        HirExprKind::Lambda {
            params,
            body,
            captures,
        } => HirExprKindFor::Lambda {
            params,
            body: convert_block(body)?,
            captures,
        },
        HirExprKind::Ref(mutable, inner) => {
            HirExprKindFor::Ref(mutable, Box::new(convert_expr(*inner)?))
        }
        HirExprKind::Deref(inner) => HirExprKindFor::Deref(Box::new(convert_expr(*inner)?)),
        HirExprKind::Cast(inner, target) => {
            HirExprKindFor::Cast(Box::new(convert_expr(*inner)?), target)
        }
        HirExprKind::ObjectCoercion(inner, coercion) => {
            if coercion.evidence.is_none() || coercion.target != ty {
                return Err(vec!["invalid or unresolved object coercion".to_string()]);
            }
            HirExprKindFor::ObjectCoercion(Box::new(convert_expr(*inner)?), coercion)
        }
        HirExprKind::Assign(left, right) => HirExprKindFor::Assign(
            Box::new(convert_expr(*left)?),
            Box::new(convert_expr(*right)?),
        ),
        HirExprKind::Intrinsic { name, args } => HirExprKindFor::Intrinsic {
            name,
            args: convert_exprs(args)?,
        },
    };
    Ok(HirExprFor { kind, ty, span })
}

fn convert_exprs(exprs: Vec<HirExpr>) -> Result<Vec<AcceptedHirExpr>, Vec<String>> {
    exprs.into_iter().map(convert_expr).collect()
}

fn convert_struct_field(
    field: HirStructLiteralField,
) -> Result<HirStructLiteralFieldFor<AcceptedHir>, Vec<String>> {
    Ok(HirStructLiteralFieldFor {
        name: field.name,
        value: convert_expr(field.value)?,
        field: field.field,
    })
}

fn convert_match_arm(arm: HirMatchArm) -> Result<HirMatchArmFor<AcceptedHir>, Vec<String>> {
    Ok(HirMatchArmFor {
        pattern: arm.pattern,
        guard: arm.guard.map(convert_expr).transpose()?,
        body: convert_block(arm.body)?,
    })
}

#[cfg(test)]
fn unresolve_program(program: HirProgramFor<AcceptedHir>) -> HirProgram {
    HirProgramFor {
        functions: program
            .functions
            .into_iter()
            .map(|(id, function)| (id, unresolve_function(function)))
            .collect(),
        structs: program.structs,
        enums: program.enums,
        traits: program
            .traits
            .into_iter()
            .map(|(id, trait_def)| (id, unresolve_trait(trait_def)))
            .collect(),
        impls: program
            .impls
            .into_iter()
            .map(|(id, imp)| (id, unresolve_impl(imp)))
            .collect(),
        externs: program.externs,
        type_aliases: program.type_aliases,
        names: program.names,
        order: program.order,
        language_items: program.language_items,
        indexes: program.indexes,
    }
}

#[cfg(test)]
fn unresolve_function(function: AcceptedHirFunction) -> HirFunction {
    HirFunctionFor {
        id: function.id,
        name: function.name,
        generic_params: function.generic_params,
        generic_bounds: function.generic_bounds,
        params: function.params,
        ret_type: function.ret_type,
        body: unresolve_block(function.body),
        is_curried: function.is_curried,
        is_method: function.is_method,
        self_receiver: function.self_receiver,
        is_unsafe: function.is_unsafe,
    }
}

#[cfg(test)]
fn unresolve_trait(trait_def: AcceptedHirTrait) -> HirTrait {
    HirTraitFor {
        id: trait_def.id,
        name: trait_def.name,
        generic_params: trait_def.generic_params,
        target: trait_def.target,
        predicates: trait_def.predicates,
        associated_types: trait_def.associated_types,
        methods: trait_def
            .methods
            .into_iter()
            .map(|(name, function)| (name, unresolve_function(function)))
            .collect(),
        signatures: trait_def.signatures,
    }
}

#[cfg(test)]
fn unresolve_impl(imp: AcceptedHirImpl) -> HirImpl {
    HirImplFor {
        id: imp.id,
        owner: imp.owner,
        type_name: imp.type_name,
        type_generics: imp.type_generics,
        receiver_pattern: imp.receiver_pattern,
        trait_name: imp.trait_name,
        trait_id: imp.trait_id,
        trait_generics: imp.trait_generics,
        trait_arg_types: imp.trait_arg_types,
        associated_types: imp.associated_types,
        bounds: imp.bounds,
        methods: imp
            .methods
            .into_iter()
            .map(|(name, function)| (name, unresolve_function(function)))
            .collect(),
    }
}

#[cfg(test)]
fn unresolve_block(block: AcceptedHirBlock) -> HirBlock {
    HirBlockFor {
        stmts: block.stmts.into_iter().map(unresolve_stmt).collect(),
        ty: block.ty,
    }
}

#[cfg(test)]
fn unresolve_stmt(stmt: HirStmtFor<AcceptedHir>) -> HirStmt {
    match stmt {
        HirStmtFor::Let {
            name,
            local_id,
            ty,
            value,
            mutable,
        } => HirStmtFor::Let {
            name,
            local_id,
            ty,
            value: unresolve_expr(value),
            mutable,
        },
        HirStmtFor::Expr(expr) => HirStmtFor::Expr(unresolve_expr(expr)),
        HirStmtFor::Return(expr) => HirStmtFor::Return(expr.map(unresolve_expr)),
        HirStmtFor::Break(expr) => HirStmtFor::Break(expr.map(unresolve_expr)),
        HirStmtFor::Continue => HirStmtFor::Continue,
    }
}

#[cfg(test)]
fn unresolve_expr(expr: AcceptedHirExpr) -> HirExpr {
    let kind = match expr.kind {
        HirExprKindFor::Open { source, binding, body } => HirExprKindFor::Open {
            source: Box::new(unresolve_expr(*source)), binding, body: unresolve_block(body),
        },
        HirExprKindFor::OwnedObjectCall { owner, args, call } => HirExprKindFor::OwnedObjectCall {
            owner: Box::new(unresolve_expr(*owner)),
            args: args.into_iter().map(unresolve_expr).collect(),
            call,
        },
        HirExprKindFor::IntLiteral(value) => HirExprKindFor::IntLiteral(value),
        HirExprKindFor::FloatLiteral(value) => HirExprKindFor::FloatLiteral(value),
        HirExprKindFor::BoolLiteral(value) => HirExprKindFor::BoolLiteral(value),
        HirExprKindFor::StringLiteral(value) => HirExprKindFor::StringLiteral(value),
        HirExprKindFor::CharLiteral(value) => HirExprKindFor::CharLiteral(value),
        HirExprKindFor::ArrayLiteral(exprs) => {
            HirExprKindFor::ArrayLiteral(exprs.into_iter().map(unresolve_expr).collect())
        }
        HirExprKindFor::ArrayRepeat(value, len) => {
            HirExprKindFor::ArrayRepeat(Box::new(unresolve_expr(*value)), len)
        }
        HirExprKindFor::TupleLiteral(exprs) => {
            HirExprKindFor::TupleLiteral(exprs.into_iter().map(unresolve_expr).collect())
        }
        HirExprKindFor::Unit => HirExprKindFor::Unit,
        HirExprKindFor::Var(name) => HirExprKindFor::Var(name),
        HirExprKindFor::ResolvedVar(reference) => HirExprKindFor::ResolvedVar(reference),
        HirExprKindFor::FieldAccess(base, name, location) => {
            HirExprKindFor::FieldAccess(Box::new(unresolve_expr(*base)), name, location)
        }
        HirExprKindFor::TupleIndex(base, index) => {
            HirExprKindFor::TupleIndex(Box::new(unresolve_expr(*base)), index)
        }
        HirExprKindFor::BinOp(op, left, right) => HirExprKindFor::BinOp(
            op,
            Box::new(unresolve_expr(*left)),
            Box::new(unresolve_expr(*right)),
        ),
        HirExprKindFor::UnaryOp(op, inner) => {
            HirExprKindFor::UnaryOp(op, Box::new(unresolve_expr(*inner)))
        }
        HirExprKindFor::Call(callee, args, target) => HirExprKindFor::Call(
            Box::new(unresolve_expr(*callee)),
            args.into_iter().map(unresolve_expr).collect(),
            target,
        ),
        HirExprKindFor::MethodCall(receiver, name, args, mode, target) => {
            HirExprKindFor::MethodCall(
                Box::new(unresolve_expr(*receiver)),
                name,
                args.into_iter().map(unresolve_expr).collect(),
                mode,
                Some(target),
            )
        }
        HirExprKindFor::Try {
            expr,
            branch_method,
            branch_target,
            branch_self_receiver,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            control_flow_enum,
            break_variant,
            continue_variant,
        } => HirExprKindFor::Try {
            expr: Box::new(unresolve_expr(*expr)),
            branch_method: Some(branch_method),
            branch_target,
            branch_self_receiver,
            from_residual_target: Some(from_residual_target),
            output_ty,
            residual_ty,
            return_ty,
            control_flow_enum,
            break_variant,
            continue_variant,
        },
        HirExprKindFor::StructLiteral(name, id, fields) => HirExprKindFor::StructLiteral(
            name,
            id,
            fields
                .into_iter()
                .map(|field| HirStructLiteralFieldFor {
                    name: field.name,
                    value: unresolve_expr(field.value),
                    field: field.field,
                })
                .collect(),
        ),
        HirExprKindFor::EnumVariant(owner, variant, args, location) => HirExprKindFor::EnumVariant(
            owner,
            variant,
            args.into_iter().map(unresolve_expr).collect(),
            location,
        ),
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => HirExprKindFor::If {
            condition: Box::new(unresolve_expr(*condition)),
            then_branch: unresolve_block(then_branch),
            else_branch: else_branch.map(unresolve_block),
        },
        HirExprKindFor::Match { scrutinee, arms } => HirExprKindFor::Match {
            scrutinee: Box::new(unresolve_expr(*scrutinee)),
            arms: arms
                .into_iter()
                .map(|arm| HirMatchArmFor {
                    pattern: arm.pattern,
                    guard: arm.guard.map(unresolve_expr),
                    body: unresolve_block(arm.body),
                })
                .collect(),
        },
        HirExprKindFor::While { condition, body } => HirExprKindFor::While {
            condition: Box::new(unresolve_expr(*condition)),
            body: unresolve_block(body),
        },
        HirExprKindFor::For {
            var,
            local_id,
            iter,
            body,
        } => HirExprKindFor::For {
            var,
            local_id,
            iter: Box::new(unresolve_expr(*iter)),
            body: unresolve_block(body),
        },
        HirExprKindFor::Loop(body) => HirExprKindFor::Loop(unresolve_block(body)),
        HirExprKindFor::Block(body) => HirExprKindFor::Block(unresolve_block(body)),
        HirExprKindFor::UnsafeBlock(body) => HirExprKindFor::Block(unresolve_block(body)),
        HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => HirExprKindFor::Lambda {
            params,
            body: unresolve_block(body),
            captures,
        },
        HirExprKindFor::Ref(mutable, inner) => {
            HirExprKindFor::Ref(mutable, Box::new(unresolve_expr(*inner)))
        }
        HirExprKindFor::Deref(inner) => HirExprKindFor::Deref(Box::new(unresolve_expr(*inner))),
        HirExprKindFor::Cast(inner, target) => {
            HirExprKindFor::Cast(Box::new(unresolve_expr(*inner)), target)
        }
        HirExprKindFor::ObjectCoercion(inner, coercion) => {
            HirExprKindFor::ObjectCoercion(Box::new(unresolve_expr(*inner)), coercion)
        }
        HirExprKindFor::Assign(left, right) => HirExprKindFor::Assign(
            Box::new(unresolve_expr(*left)),
            Box::new(unresolve_expr(*right)),
        ),
        HirExprKindFor::Intrinsic { name, args } => HirExprKindFor::Intrinsic {
            name,
            args: args.into_iter().map(unresolve_expr).collect(),
        },
    };
    HirExprFor {
        kind,
        ty: expr.ty,
        span: expr.span,
    }
}
