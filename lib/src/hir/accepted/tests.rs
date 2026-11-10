use super::*;

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

fn unresolve_block(block: AcceptedHirBlock) -> HirBlock {
    HirBlockFor {
        stmts: block.stmts.into_iter().map(unresolve_stmt).collect(),
        ty: block.ty,
    }
}

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

fn unresolve_expr(expr: AcceptedHirExpr) -> HirExpr {
    let kind = match expr.kind {
        HirExprKindFor::IntLiteral(value) => HirExprKindFor::IntLiteral(value),
        HirExprKindFor::ConstParam(param) => HirExprKindFor::ConstParam(param),
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

impl AcceptedHirProgram {
    pub(crate) fn revalidate_for_test(
        program: HirProgramFor<AcceptedHir>,
    ) -> Result<Self, Vec<String>> {
        Self::try_from(unresolve_program(program))
    }

    pub(crate) fn program_mut_for_test(&mut self) -> &mut HirProgramFor<AcceptedHir> {
        &mut self.program
    }
}
