use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use crate::hir::{
    HirBlock, HirCallTarget, HirClosureCapture, HirClosureCaptureKind, HirExpr, HirExprKind,
    HirFieldLocation, HirFunction, HirGenericBounds, HirStaticMethodTarget, HirStmt,
    HirTypeBinding, HirVarRef, HirVarTarget,
};
use crate::ids::{DefId, HirLocalId};
use crate::infer::constraints::Constraint;
use crate::infer::{ConstraintOwner, ConstraintStore, ObligationState, PartialHir};
use crate::language_items::TryLanguageItems;
use crate::lower::ResolveError;
use crate::selection::{
    type_pattern_matches, ReceiverAdjustment, ReceiverCandidate, SelectedMethod, SelectionService,
};
use crate::types::{
    CallableKind, CaptureKind, FunctionCapture, FunctionSafety, GenericParamId, TraitBound, Type,
};

mod callable;

fn native_call_type(ty: &Type) -> bool {
    match ty {
        Type::Function { .. } => true,
        Type::Reference { inner, .. } => native_call_type(inner),
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AuthorityPassResult {
    pub progress: bool,
    pub pending: bool,
    pub ambiguous: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct AuthorityObligationId(u32);

impl AuthorityObligationId {
    pub(super) const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum AuthorityObligationKind {
    Propagation,
    DeferredCall,
    CallableCall,
    MethodCall,
    Field,
    TryBranch,
    FromResidual,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AuthoritySiteId {
    kind: AuthorityObligationKind,
    file_path: std::path::PathBuf,
    start: usize,
    end: usize,
}

impl AuthoritySiteId {
    fn new(kind: AuthorityObligationKind, span: &crate::lexer::Span) -> Self {
        Self {
            kind,
            file_path: span.file_path.clone(),
            start: span.start,
            end: span.end,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct AuthorityObligation {
    pub id: AuthorityObligationId,
    pub owner: ConstraintOwner,
    pub kind: AuthorityObligationKind,
    pub state: ObligationState,
    pub last_generation: u64,
    pub attempts: u32,
    pub context: String,
    pub span: Option<crate::lexer::Span>,
    site: Option<AuthoritySiteId>,
    dependencies: HashSet<crate::ids::TypeVarId>,
}

impl AuthorityObligation {
    pub(super) fn depends_on_any(&self, changed: &HashSet<crate::ids::TypeVarId>) -> bool {
        self.dependencies.iter().any(|var| changed.contains(var))
    }

    pub(super) fn wake(&mut self) {
        if matches!(
            self.state,
            ObligationState::Pending | ObligationState::Ambiguous
        ) {
            self.state = ObligationState::Pending;
            self.last_generation = u64::MAX;
        }
    }
}

pub(super) fn authority_obligations(
    hir: &PartialHir,
    owners: &HashSet<ConstraintOwner>,
) -> Vec<AuthorityObligation> {
    let mut selected_owners = owners
        .iter()
        .copied()
        .filter(|owner| matches!(owner, ConstraintOwner::Body(_)))
        .collect::<Vec<_>>();
    selected_owners.sort();
    let mut obligations = Vec::with_capacity(selected_owners.len() * 2);
    for owner in selected_owners {
        let id = AuthorityObligationId(obligations.len() as u32);
        let mut propagation = AuthorityObligation {
            id,
            owner,
            kind: AuthorityObligationKind::Propagation,
            state: ObligationState::Pending,
            last_generation: u64::MAX,
            attempts: 0,
            context: function_for_owner(hir, owner)
                .map(|function| format!("call and type propagation for '{}'", function.name))
                .unwrap_or_else(|| "call and type propagation".to_string()),
            span: None,
            site: None,
            dependencies: HashSet::new(),
        };
        refresh_authority_dependencies(hir, &mut propagation);
        obligations.push(propagation);

        if let Some(function) = function_for_owner(hir, owner) {
            let mut sites = Vec::new();
            collect_authority_sites(&function.body, &hir.engine, &mut sites);
            for (kind, site, context, span, dependencies) in sites {
                let id = AuthorityObligationId(obligations.len() as u32);
                obligations.push(AuthorityObligation {
                    id,
                    owner,
                    kind,
                    state: ObligationState::Pending,
                    last_generation: u64::MAX,
                    attempts: 0,
                    context,
                    span: Some(span),
                    site: Some(site),
                    dependencies: dependencies
                        .into_iter()
                        .map(|var| {
                            hir.engine
                                .unresolved_type_var_representative_for_dependency(var)
                                .unwrap_or(var)
                        })
                        .collect(),
                });
            }
        }
    }
    obligations
}

pub(super) fn run_authority_obligation(
    hir: &mut PartialHir,
    obligation: &mut AuthorityObligation,
    strict: bool,
    try_strict: bool,
) -> Result<AuthorityPassResult, Vec<ResolveError>> {
    if obligation.state != ObligationState::Pending {
        return Ok(AuthorityPassResult {
            progress: false,
            pending: false,
            ambiguous: obligation.state == ObligationState::Ambiguous,
        });
    }
    let generation = hir.engine.substitution_generation();
    if obligation.last_generation == generation {
        return Ok(AuthorityPassResult {
            progress: false,
            pending: true,
            ambiguous: false,
        });
    }
    let owners = HashSet::from([obligation.owner]);
    let previous_owner = hir.constraint_store.replace_owner(obligation.owner);
    let result = match obligation.kind {
        AuthorityObligationKind::Propagation => {
            propagate_function_instances_for_owners_with_progress(hir, Some(&owners))
        }
        AuthorityObligationKind::DeferredCall
        | AuthorityObligationKind::CallableCall
        | AuthorityObligationKind::MethodCall
        | AuthorityObligationKind::Field
        | AuthorityObligationKind::TryBranch
        | AuthorityObligationKind::FromResidual => materialize_authority_site(
            hir,
            obligation.owner,
            obligation.site.clone().expect("selection obligation site"),
            strict,
            try_strict,
        ),
    };
    hir.constraint_store.replace_owner(previous_owner);
    let result = result?;
    obligation.last_generation = generation;
    obligation.attempts = obligation.attempts.saturating_add(1);
    if obligation.kind != AuthorityObligationKind::Propagation {
        if result.ambiguous {
            obligation.state = ObligationState::Ambiguous;
        } else if !result.pending {
            obligation.state = ObligationState::Solved;
        }
    }
    refresh_authority_dependencies(hir, obligation);
    Ok(result)
}

fn refresh_authority_dependencies(hir: &PartialHir, obligation: &mut AuthorityObligation) {
    obligation.dependencies.clear();
    let ConstraintOwner::Body(_) = obligation.owner else {
        return;
    };
    let function = function_for_owner(hir, obligation.owner);
    let Some(function) = function else {
        return;
    };
    if let Some(selected_site) = obligation.site.as_ref() {
        let mut sites = Vec::new();
        collect_authority_sites(&function.body, &hir.engine, &mut sites);
        if let Some((_, _, _, _, dependencies)) = sites
            .into_iter()
            .find(|(_, site, ..)| site == selected_site)
        {
            obligation.dependencies = dependencies;
        }
    } else {
        for ty in function
            .params
            .iter()
            .map(|param| &param.ty)
            .chain(std::iter::once(&function.ret_type))
        {
            collect_type_vars(&hir.engine.resolve(ty), &mut obligation.dependencies);
        }
        collect_block_type_vars(&function.body, &hir.engine, &mut obligation.dependencies);
    }
    let dependencies = obligation.dependencies.drain().collect::<Vec<_>>();
    for var in dependencies {
        obligation.dependencies.insert(
            hir.engine
                .unresolved_type_var_representative_for_dependency(var)
                .unwrap_or(var),
        );
    }
}

fn function_for_owner(hir: &PartialHir, owner: ConstraintOwner) -> Option<&HirFunction> {
    let ConstraintOwner::Body(id) = owner else {
        return None;
    };
    hir.functions
        .get(&id)
        .or_else(|| {
            hir.traits
                .values()
                .flat_map(|trait_def| trait_def.methods.values())
                .find(|method| method.id == id)
        })
        .or_else(|| {
            hir.impls
                .values()
                .flat_map(|imp| imp.methods.values())
                .find(|method| method.id == id)
        })
}

type AuthoritySite = (
    AuthorityObligationKind,
    AuthoritySiteId,
    String,
    crate::lexer::Span,
    HashSet<crate::ids::TypeVarId>,
);

fn collect_authority_sites(
    root: &HirBlock,
    engine: &crate::infer::InferenceEngine,
    output: &mut Vec<AuthoritySite>,
) {
    fn dependencies(
        types: impl IntoIterator<Item = Type>,
        engine: &crate::infer::InferenceEngine,
    ) -> HashSet<crate::ids::TypeVarId> {
        let mut output = HashSet::new();
        for ty in types {
            collect_type_vars(&engine.resolve(&ty), &mut output);
        }
        output
    }

    fn block(
        body: &HirBlock,
        engine: &crate::infer::InferenceEngine,
        output: &mut Vec<AuthoritySite>,
    ) {
        for statement in &body.stmts {
            match statement {
                HirStmt::Let { value, .. }
                | HirStmt::Expr(value)
                | HirStmt::Return(Some(value))
                | HirStmt::Break(Some(value)) => expr(value, engine, output, false),
                HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
            }
        }
    }

    fn expr(
        node: &HirExpr,
        engine: &crate::infer::InferenceEngine,
        output: &mut Vec<AuthoritySite>,
        suppress_field_slot: bool,
    ) {
        match &node.kind {
            HirExprKind::Call(callee, args, target) => {
                if !native_call_type(&callee.ty)
                    && !matches!(callee.kind, HirExprKind::FieldAccess(..))
                {
                    let kind = AuthorityObligationKind::CallableCall;
                    output.push((
                        kind,
                        AuthoritySiteId::new(kind, &node.span),
                        "function call".into(),
                        node.span.clone(),
                        dependencies(
                            std::iter::once(callee.ty.clone())
                                .chain(std::iter::once(node.ty.clone()))
                                .chain(args.iter().map(|arg| arg.ty.clone())),
                            engine,
                        ),
                    ));
                }
                if let HirExprKind::FieldAccess(receiver, name, location) = &callee.kind {
                    if target.is_none()
                        && (location.is_none()
                            || contains_recovery_type(&engine.resolve(&receiver.ty)))
                    {
                        let kind = AuthorityObligationKind::DeferredCall;
                        let types = std::iter::once(node.ty.clone())
                            .chain(std::iter::once(receiver.ty.clone()))
                            .chain(args.iter().map(|arg| arg.ty.clone()));
                        output.push((
                            kind,
                            AuthoritySiteId::new(kind, &node.span),
                            format!("method or operator call '{name}'"),
                            node.span.clone(),
                            dependencies(types, engine),
                        ));
                    }
                    expr(callee, engine, output, true);
                } else {
                    expr(callee, engine, output, false);
                }
                for arg in args {
                    expr(arg, engine, output, false);
                }
            }
            HirExprKind::MethodCall(receiver, name, args, _, target) => {
                if target.is_none() {
                    let kind = AuthorityObligationKind::MethodCall;
                    let types = std::iter::once(node.ty.clone())
                        .chain(std::iter::once(receiver.ty.clone()))
                        .chain(args.iter().map(|arg| arg.ty.clone()));
                    output.push((
                        kind,
                        AuthoritySiteId::new(kind, &node.span),
                        format!("method call '{name}'"),
                        node.span.clone(),
                        dependencies(types, engine),
                    ));
                }
                expr(receiver, engine, output, false);
                for arg in args {
                    expr(arg, engine, output, false);
                }
            }
            HirExprKind::FieldAccess(receiver, name, location) => {
                if !suppress_field_slot {
                    if location.is_none() {
                        let kind = AuthorityObligationKind::Field;
                        output.push((
                            kind,
                            AuthoritySiteId::new(kind, &node.span),
                            format!("field access '{name}'"),
                            node.span.clone(),
                            dependencies([node.ty.clone(), receiver.ty.clone()], engine),
                        ));
                    }
                }
                expr(receiver, engine, output, false);
            }
            HirExprKind::Try {
                expr: operand,
                branch_method,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                ..
            } => {
                if branch_method.is_none() {
                    let kind = AuthorityObligationKind::TryBranch;
                    output.push((
                        kind,
                        AuthoritySiteId::new(kind, &node.span),
                        "Try::branch".to_string(),
                        node.span.clone(),
                        dependencies(
                            [operand.ty.clone(), output_ty.clone(), residual_ty.clone()],
                            engine,
                        ),
                    ));
                }
                if from_residual_target.is_none() {
                    let kind = AuthorityObligationKind::FromResidual;
                    output.push((
                        kind,
                        AuthoritySiteId::new(kind, &node.span),
                        "FromResidual::from_residual".to_string(),
                        node.span.clone(),
                        dependencies([residual_ty.clone(), return_ty.clone()], engine),
                    ));
                }
                expr(operand, engine, output, false);
            }
            HirExprKind::Deref(inner)
            | HirExprKind::Ref(_, inner)
            | HirExprKind::Cast(inner, _)
            | HirExprKind::TupleIndex(inner, _)
            | HirExprKind::UnaryOp(_, inner)
            | HirExprKind::ArrayRepeat(inner, _) => expr(inner, engine, output, false),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                expr(condition, engine, output, false);
                block(then_branch, engine, output);
                if let Some(else_branch) = else_branch {
                    block(else_branch, engine, output);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                expr(scrutinee, engine, output, false);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        expr(guard, engine, output, false);
                    }
                    block(&arm.body, engine, output);
                }
            }
            HirExprKind::While { condition, body } => {
                expr(condition, engine, output, false);
                block(body, engine, output);
            }
            HirExprKind::For { iter, body, .. } => {
                expr(iter, engine, output, false);
                block(body, engine, output);
            }
            HirExprKind::Block(body)
            | HirExprKind::Loop(body)
            | HirExprKind::UnsafeBlock(body)
            | HirExprKind::Lambda { body, .. } => block(body, engine, output),
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                expr(left, engine, output, false);
                expr(right, engine, output, false);
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    expr(arg, engine, output, false);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    expr(&field.value, engine, output, false);
                }
            }
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit => {}
        }
    }

    block(root, engine, output);
}

fn materialize_authority_site(
    hir: &mut PartialHir,
    owner: ConstraintOwner,
    site: AuthoritySiteId,
    strict: bool,
    try_strict: bool,
) -> Result<AuthorityPassResult, Vec<ResolveError>> {
    let owners = HashSet::from([owner]);
    let before_generation = hir.engine.rigid_substitution_generation();
    let before_counts = authority_counts(hir, Some(&owners));
    let ambiguous =
        materialize_pending(hir, strict, try_strict, Some(&owners), Some(site.clone()))?;
    let mut result = authority_pass_result(hir, before_generation, before_counts, Some(&owners));
    result.pending = function_for_owner(hir, owner).is_some_and(|function| {
        let mut sites = Vec::new();
        collect_authority_sites(&function.body, &hir.engine, &mut sites);
        sites.iter().any(|(_, candidate, ..)| *candidate == site)
    });
    result.ambiguous = ambiguous;
    Ok(result)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct AuthorityCounts {
    methods: usize,
    pending_methods: usize,
    residuals: usize,
    pending_residuals: usize,
    fields: usize,
    pending_fields: usize,
}

pub(super) fn pending_authority_error(
    hir: &PartialHir,
    owners: Option<&HashSet<ConstraintOwner>>,
    obligations: &[AuthorityObligation],
) -> ResolveError {
    let message = if authority_counts(hir, owners).pending_residuals > 0 {
        "cannot resolve '?' because its carrier or enclosing residual remains unresolved"
            .to_string()
    } else if authority_counts(hir, owners).pending_methods > 0 {
        "cannot resolve method authority because the receiver remains unresolved".to_string()
    } else if let Some((member, receiver)) = first_pending_field(hir, owners) {
        format!(
            "cannot resolve member '{member}' because receiver type {} remains unresolved",
            hir.engine.display_type(&receiver)
        )
    } else {
        "cannot resolve field authority because the receiver remains unresolved".to_string()
    };
    let span = obligations
        .iter()
        .find(|obligation| {
            obligation.kind != AuthorityObligationKind::Propagation
                && obligation.state != ObligationState::Solved
        })
        .and_then(|obligation| obligation.span.clone())
        .expect("pending source authority obligation must carry its expression span");
    ResolveError::with_span_code(message, span, crate::diagnostic::DiagnosticCode::Selection)
}

pub(super) fn pending_authority_context(obligations: &[AuthorityObligation]) -> Option<&str> {
    obligations
        .iter()
        .find(|obligation| {
            obligation.kind != AuthorityObligationKind::Propagation
                && obligation.state != ObligationState::Solved
        })
        .map(|obligation| obligation.context.as_str())
}

fn first_pending_field(
    hir: &PartialHir,
    owners: Option<&HashSet<ConstraintOwner>>,
) -> Option<(String, Type)> {
    fn block(block: &HirBlock, engine: &crate::infer::InferenceEngine) -> Option<(String, Type)> {
        block.stmts.iter().find_map(|statement| match statement {
            HirStmt::Let { value, .. }
            | HirStmt::Expr(value)
            | HirStmt::Return(Some(value))
            | HirStmt::Break(Some(value)) => expr(value, engine),
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => None,
        })
    }

    fn expr(node: &HirExpr, engine: &crate::infer::InferenceEngine) -> Option<(String, Type)> {
        match &node.kind {
            HirExprKind::FieldAccess(receiver, name, None) => {
                Some((name.clone(), engine.resolve(&receiver.ty)))
            }
            HirExprKind::FieldAccess(receiver, _, Some(_)) => expr(receiver, engine),
            HirExprKind::Call(callee, args, _) => {
                expr(callee, engine).or_else(|| args.iter().find_map(|arg| expr(arg, engine)))
            }
            HirExprKind::MethodCall(receiver, _, args, _, _) => {
                expr(receiver, engine).or_else(|| args.iter().find_map(|arg| expr(arg, engine)))
            }
            HirExprKind::Try { expr: operand, .. }
            | HirExprKind::Deref(operand)
            | HirExprKind::Ref(_, operand)
            | HirExprKind::Cast(operand, _)
            | HirExprKind::TupleIndex(operand, _)
            | HirExprKind::UnaryOp(_, operand)
            | HirExprKind::ArrayRepeat(operand, _) => expr(operand, engine),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => expr(condition, engine)
                .or_else(|| block(then_branch, engine))
                .or_else(|| else_branch.as_ref().and_then(|body| block(body, engine))),
            HirExprKind::Match { scrutinee, arms } => expr(scrutinee, engine).or_else(|| {
                arms.iter().find_map(|arm| {
                    arm.guard
                        .as_ref()
                        .and_then(|guard| expr(guard, engine))
                        .or_else(|| block(&arm.body, engine))
                })
            }),
            HirExprKind::While { condition, body } => {
                expr(condition, engine).or_else(|| block(body, engine))
            }
            HirExprKind::For { iter, body, .. } => {
                expr(iter, engine).or_else(|| block(body, engine))
            }
            HirExprKind::Block(body)
            | HirExprKind::Loop(body)
            | HirExprKind::UnsafeBlock(body)
            | HirExprKind::Lambda { body, .. } => block(body, engine),
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                expr(left, engine).or_else(|| expr(right, engine))
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                args.iter().find_map(|arg| expr(arg, engine))
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                fields.iter().find_map(|field| expr(&field.value, engine))
            }
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit => None,
        }
    }

    hir.functions
        .iter()
        .filter(|(id, _)| owner_selected(owners, ConstraintOwner::Body(**id)))
        .map(|(_, function)| function)
        .chain(
            hir.traits
                .values()
                .flat_map(|trait_def| trait_def.methods.values())
                .filter(|function| owner_selected(owners, ConstraintOwner::Body(function.id))),
        )
        .chain(
            hir.impls
                .values()
                .flat_map(|imp| imp.methods.values())
                .filter(|function| owner_selected(owners, ConstraintOwner::Body(function.id))),
        )
        .find_map(|function| block(&function.body, &hir.engine))
}

pub(super) fn propagate_function_instances_for_owners_with_progress(
    hir: &mut PartialHir,
    owners: Option<&HashSet<ConstraintOwner>>,
) -> Result<AuthorityPassResult, Vec<ResolveError>> {
    let before_generation = hir.engine.rigid_substitution_generation();
    let before_counts = authority_counts(hir, owners);
    let mut constrained_vars = HashMap::new();
    for function in hir
        .functions
        .values()
        .chain(
            hir.traits
                .values()
                .flat_map(|trait_def| trait_def.methods.values()),
        )
        .chain(hir.impls.values().flat_map(|imp| imp.methods.values()))
    {
        let shared = constrained_signature_vars(hir, function);
        if !shared.is_empty() {
            constrained_vars.insert(function.id, shared);
        }
    }
    let mut errors = Vec::new();
    let mut function_ids = hir.functions.keys().copied().collect::<Vec<_>>();
    function_ids.sort();
    for id in &function_ids {
        if !owner_selected(owners, ConstraintOwner::Body(*id)) {
            continue;
        }
        if let Some(mut body) = hir.functions.get(id).map(|function| function.body.clone()) {
            propagate_block(hir, &mut body, &constrained_vars, &mut errors);
            if let Some(function) = hir.functions.get_mut(id) {
                let _ = hir.engine.unify(&function.ret_type, &body.ty);
                function.ret_type = hir.engine.resolve(&function.ret_type);
                function.body = body;
            }
        }
    }

    let mut impl_ids = hir.impls.keys().copied().collect::<Vec<_>>();
    impl_ids.sort();
    for impl_id in impl_ids {
        let mut method_names = hir.impls[&impl_id]
            .methods
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        method_names.sort();
        for method_name in method_names {
            let method_id = hir.impls[&impl_id].methods[&method_name].id;
            if !owner_selected(owners, ConstraintOwner::Body(method_id)) {
                continue;
            }
            if let Some(mut body) = hir
                .impls
                .get(&impl_id)
                .and_then(|imp| imp.methods.get(&method_name))
                .map(|method| method.body.clone())
            {
                propagate_block(hir, &mut body, &constrained_vars, &mut errors);
                if let Some(method) = hir
                    .impls
                    .get_mut(&impl_id)
                    .and_then(|imp| imp.methods.get_mut(&method_name))
                {
                    method.body = body;
                }
            }
        }
    }

    if errors.is_empty() {
        let mut result = authority_pass_result(hir, before_generation, before_counts, owners);
        // Instance propagation may create fresh call-site variables without
        // changing any authority; that is not observable progress for the
        // driver and must not keep waking the same call forever.
        result.progress = authority_counts(hir, owners) != before_counts;
        Ok(result)
    } else {
        Err(errors)
    }
}

fn owner_selected(owners: Option<&HashSet<ConstraintOwner>>, owner: ConstraintOwner) -> bool {
    owners.is_none_or(|owners| owners.contains(&owner))
}

fn authority_pass_result(
    hir: &PartialHir,
    before_generation: u64,
    before_counts: AuthorityCounts,
    owners: Option<&HashSet<ConstraintOwner>>,
) -> AuthorityPassResult {
    let after_counts = authority_counts(hir, owners);
    AuthorityPassResult {
        progress: before_generation != hir.engine.rigid_substitution_generation()
            || before_counts != after_counts,
        pending: after_counts.pending_methods > 0
            || after_counts.pending_residuals > 0
            || after_counts.pending_fields > 0,
        ambiguous: false,
    }
}

fn authority_counts(
    hir: &PartialHir,
    owners: Option<&HashSet<ConstraintOwner>>,
) -> AuthorityCounts {
    fn block(block: &HirBlock, counts: &mut AuthorityCounts) {
        for statement in &block.stmts {
            match statement {
                HirStmt::Let { value, .. }
                | HirStmt::Expr(value)
                | HirStmt::Return(Some(value))
                | HirStmt::Break(Some(value)) => expr(value, counts),
                HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
            }
        }
    }

    fn expr(node: &HirExpr, counts: &mut AuthorityCounts) {
        match &node.kind {
            HirExprKind::MethodCall(receiver, _, args, _, target) => {
                if target.is_some() {
                    counts.methods += 1;
                } else {
                    counts.pending_methods += 1;
                }
                expr(receiver, counts);
                for arg in args {
                    expr(arg, counts);
                }
            }
            HirExprKind::Try {
                expr: operand,
                branch_method,
                from_residual_target,
                ..
            } => {
                if branch_method.is_some() {
                    counts.methods += 1;
                } else {
                    counts.pending_residuals += 1;
                }
                if from_residual_target.is_some() {
                    counts.residuals += 1;
                } else {
                    counts.pending_residuals += 1;
                }
                expr(operand, counts);
            }
            HirExprKind::FieldAccess(receiver, _, location) => {
                if location.is_some() {
                    counts.fields += 1;
                } else {
                    counts.pending_fields += 1;
                }
                expr(receiver, counts);
            }
            HirExprKind::Call(callee, args, _) => {
                expr(callee, counts);
                for arg in args {
                    expr(arg, counts);
                }
            }
            HirExprKind::Deref(inner)
            | HirExprKind::Ref(_, inner)
            | HirExprKind::Cast(inner, _)
            | HirExprKind::TupleIndex(inner, _)
            | HirExprKind::UnaryOp(_, inner)
            | HirExprKind::ArrayRepeat(inner, _) => expr(inner, counts),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                expr(condition, counts);
                block(then_branch, counts);
                if let Some(else_branch) = else_branch {
                    block(else_branch, counts);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                expr(scrutinee, counts);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        expr(guard, counts);
                    }
                    block(&arm.body, counts);
                }
            }
            HirExprKind::While { condition, body } => {
                expr(condition, counts);
                block(body, counts);
            }
            HirExprKind::For { iter, body, .. } => {
                expr(iter, counts);
                block(body, counts);
            }
            HirExprKind::Block(body) | HirExprKind::Loop(body) | HirExprKind::UnsafeBlock(body) => {
                block(body, counts)
            }
            HirExprKind::Lambda { body, .. } => block(body, counts),
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                expr(left, counts);
                expr(right, counts);
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    expr(arg, counts);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    expr(&field.value, counts);
                }
            }
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit => {}
        }
    }

    let mut counts = AuthorityCounts::default();
    for (id, function) in &hir.functions {
        if owner_selected(owners, ConstraintOwner::Body(*id)) {
            block(&function.body, &mut counts);
        }
    }
    for trait_def in hir.traits.values() {
        for function in trait_def.methods.values() {
            if owner_selected(owners, ConstraintOwner::Body(function.id)) {
                block(&function.body, &mut counts);
            }
        }
    }
    for impl_def in hir.impls.values() {
        for function in impl_def.methods.values() {
            if owner_selected(owners, ConstraintOwner::Body(function.id)) {
                block(&function.body, &mut counts);
            }
        }
    }
    counts
}

fn collect_type_vars(ty: &Type, vars: &mut HashSet<crate::ids::TypeVarId>) {
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        if let Type::TypeVar(id) = nested {
            vars.insert(*id);
        }
    });
}

fn collect_block_type_vars(
    block: &HirBlock,
    engine: &crate::infer::InferenceEngine,
    output: &mut HashSet<crate::ids::TypeVarId>,
) {
    fn expr(
        node: &HirExpr,
        engine: &crate::infer::InferenceEngine,
        output: &mut HashSet<crate::ids::TypeVarId>,
    ) {
        collect_type_vars(&engine.resolve(&node.ty), output);
        match &node.kind {
            HirExprKind::Call(callee, args, _) | HirExprKind::MethodCall(callee, _, args, _, _) => {
                expr(callee, engine, output);
                for arg in args {
                    expr(arg, engine, output);
                }
            }
            HirExprKind::Try {
                expr: operand,
                output_ty,
                residual_ty,
                return_ty,
                ..
            } => {
                expr(operand, engine, output);
                for ty in [output_ty, residual_ty, return_ty] {
                    collect_type_vars(&engine.resolve(ty), output);
                }
            }
            HirExprKind::FieldAccess(inner, _, _)
            | HirExprKind::Deref(inner)
            | HirExprKind::Ref(_, inner)
            | HirExprKind::Cast(inner, _)
            | HirExprKind::TupleIndex(inner, _)
            | HirExprKind::UnaryOp(_, inner)
            | HirExprKind::ArrayRepeat(inner, _) => expr(inner, engine, output),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                expr(condition, engine, output);
                collect_block_type_vars(then_branch, engine, output);
                if let Some(else_branch) = else_branch {
                    collect_block_type_vars(else_branch, engine, output);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                expr(scrutinee, engine, output);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        expr(guard, engine, output);
                    }
                    collect_block_type_vars(&arm.body, engine, output);
                }
            }
            HirExprKind::While { condition, body } => {
                expr(condition, engine, output);
                collect_block_type_vars(body, engine, output);
            }
            HirExprKind::For { iter, body, .. } => {
                expr(iter, engine, output);
                collect_block_type_vars(body, engine, output);
            }
            HirExprKind::Block(body) | HirExprKind::Loop(body) | HirExprKind::UnsafeBlock(body) => {
                collect_block_type_vars(body, engine, output)
            }
            HirExprKind::Lambda { params, body, .. } => {
                for param in params {
                    collect_type_vars(&engine.resolve(&param.ty), output);
                }
                collect_block_type_vars(body, engine, output);
            }
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                expr(left, engine, output);
                expr(right, engine, output);
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    expr(arg, engine, output);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    expr(&field.value, engine, output);
                }
            }
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit => {}
        }
    }

    collect_type_vars(&engine.resolve(&block.ty), output);
    for statement in &block.stmts {
        match statement {
            HirStmt::Let { ty, value, .. } => {
                collect_type_vars(&engine.resolve(ty), output);
                expr(value, engine, output);
            }
            HirStmt::Expr(value) | HirStmt::Return(Some(value)) | HirStmt::Break(Some(value)) => {
                expr(value, engine, output)
            }
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
        }
    }
}

fn constrained_signature_vars(
    hir: &PartialHir,
    function: &HirFunction,
) -> HashSet<crate::ids::TypeVarId> {
    let mut signature_vars = HashSet::new();
    for ty in function
        .params
        .iter()
        .map(|param| &param.ty)
        .chain(std::iter::once(&function.ret_type))
    {
        collect_type_vars(&hir.engine.resolve(ty), &mut signature_vars);
    }
    let mut shared = HashSet::new();
    for (obligation, constraint) in hir.constraint_store.iter() {
        if hir.constraint_store.owner(obligation) != Some(ConstraintOwner::Body(function.id)) {
            continue;
        }
        let mut obligation_vars = HashSet::new();
        match constraint {
            Constraint::Trait { ty, bound, .. } => {
                if matches!(hir.engine.resolve(ty), Type::TypeVar(_))
                    && hir
                        .language_items
                        .callable_protocols()
                        .any(|(_, id, _)| id == bound.trait_id)
                {
                    // A callable requirement is a polymorphic predicate, not
                    // evidence fixing the declaration to one caller's type.
                    continue;
                }
                collect_type_vars(&hir.engine.resolve(ty), &mut obligation_vars);
                for arg in &bound.type_args {
                    collect_type_vars(&hir.engine.resolve(arg), &mut obligation_vars);
                }
            }
            Constraint::Equality { left, right, .. } => {
                collect_type_vars(&hir.engine.resolve(left), &mut obligation_vars);
                collect_type_vars(&hir.engine.resolve(right), &mut obligation_vars);
            }
            Constraint::Coercion {
                actual, expected, ..
            } => {
                collect_type_vars(&hir.engine.resolve(actual), &mut obligation_vars);
                collect_type_vars(&hir.engine.resolve(expected), &mut obligation_vars);
            }
            Constraint::Try {
                carrier,
                output,
                residual,
                return_ty,
                ..
            } => {
                for ty in [carrier, output, residual, return_ty] {
                    collect_type_vars(&hir.engine.resolve(ty), &mut obligation_vars);
                }
            }
            Constraint::IntLiteral { var, .. } | Constraint::FloatLiteral { var, .. } => {
                collect_type_vars(
                    &hir.engine.resolve(&Type::TypeVar(*var)),
                    &mut obligation_vars,
                );
            }
        }
        shared.extend(signature_vars.intersection(&obligation_vars).copied());
    }
    let mut authority_vars = HashSet::new();
    collect_unresolved_authority_vars(&function.body, &hir.engine, &mut authority_vars);
    shared.extend(signature_vars.intersection(&authority_vars).copied());
    shared
}

fn propagate_block(
    hir: &mut PartialHir,
    block: &mut HirBlock,
    constrained_vars: &HashMap<DefId, HashSet<crate::ids::TypeVarId>>,
    errors: &mut Vec<ResolveError>,
) {
    for stmt in &mut block.stmts {
        match stmt {
            HirStmt::Let { value, .. }
            | HirStmt::Expr(value)
            | HirStmt::Return(Some(value))
            | HirStmt::Break(Some(value)) => propagate_expr(hir, value, constrained_vars, errors),
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
        }
    }
    block.ty = hir.engine.resolve(&block.ty);
}

fn propagate_expr(
    hir: &mut PartialHir,
    expr: &mut HirExpr,
    constrained_vars: &HashMap<DefId, HashSet<crate::ids::TypeVarId>>,
    errors: &mut Vec<ResolveError>,
) {
    if let HirExprKind::ResolvedVar(HirVarRef {
        target: HirVarTarget::Function(function_id),
        ..
    }) = &expr.kind
    {
        propagate_function_value_instance(
            hir,
            expr.ty.clone(),
            *function_id,
            constrained_vars,
            errors,
            &expr.span,
        );
    }
    match &mut expr.kind {
        HirExprKind::Call(callee, args, _) => {
            // Establish the call-site scheme before descending into deferred
            // arguments so their method authority can use the parameter type.
            propagate_call_instance(hir, &expr.ty, callee, args, constrained_vars, errors);
            let resolved = hir.engine.resolve(&callee.ty);
            let mut native = &resolved;
            while let Type::Reference { inner, .. } = native {
                native = inner;
            }
            let indirect = !matches!(&callee.kind, HirExprKind::ResolvedVar(reference) if !matches!(reference.target, HirVarTarget::Local(_)));
            if let Type::Function { params, ret, .. } = native {
                if indirect && params.len() == args.len() {
                    for (arg, expected) in args.iter().zip(params) {
                        propagate_argument_type(
                            &mut hir.engine,
                            &arg.ty,
                            expected,
                            errors,
                            &arg.span,
                        );
                    }
                    let _ = hir.engine.unify(&expr.ty, ret);
                }
            }
            if !matches!(
                callee.kind,
                HirExprKind::ResolvedVar(HirVarRef {
                    target: HirVarTarget::Function(_),
                    ..
                })
            ) {
                propagate_expr(hir, callee, constrained_vars, errors);
            }
            for arg in args.iter_mut() {
                propagate_expr(hir, arg, constrained_vars, errors);
            }
            propagate_call_instance(hir, &expr.ty, callee, args, constrained_vars, errors);
        }
        HirExprKind::MethodCall(receiver, _, args, _, target) => {
            if let Some(target) = target.as_ref() {
                propagate_method_arguments(hir, receiver, args, target);
            }
            propagate_expr(hir, receiver, constrained_vars, errors);
            for arg in args {
                propagate_expr(hir, arg, constrained_vars, errors);
            }
            if let Some(target) = target {
                propagate_method_result(hir, expr.ty.clone(), target, errors, &expr.span);
            }
        }
        HirExprKind::Try { expr, .. } => propagate_expr(hir, expr, constrained_vars, errors),
        HirExprKind::FieldAccess(receiver, _, _)
        | HirExprKind::Deref(receiver)
        | HirExprKind::Ref(_, receiver)
        | HirExprKind::Cast(receiver, _) => propagate_expr(hir, receiver, constrained_vars, errors),
        HirExprKind::TupleIndex(receiver, index) => {
            propagate_expr(hir, receiver, constrained_vars, errors);
            if let Type::Tuple(elements) = hir.engine.resolve(&receiver.ty) {
                if let Some(element) = elements.get(*index as usize) {
                    let _ = hir.engine.unify(&expr.ty, element);
                }
            }
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            propagate_expr(hir, condition, constrained_vars, errors);
            propagate_block(hir, then_branch, constrained_vars, errors);
            if let Some(else_branch) = else_branch {
                propagate_block(hir, else_branch, constrained_vars, errors);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            propagate_expr(hir, scrutinee, constrained_vars, errors);
            for arm in arms {
                if let Some(guard) = &mut arm.guard {
                    propagate_expr(hir, guard, constrained_vars, errors);
                }
                propagate_block(hir, &mut arm.body, constrained_vars, errors);
            }
        }
        HirExprKind::While { condition, body } => {
            propagate_expr(hir, condition, constrained_vars, errors);
            propagate_block(hir, body, constrained_vars, errors);
        }
        HirExprKind::For { iter, body, .. } => {
            propagate_expr(hir, iter, constrained_vars, errors);
            propagate_block(hir, body, constrained_vars, errors);
        }
        HirExprKind::Block(body) | HirExprKind::Loop(body) | HirExprKind::UnsafeBlock(body) => {
            propagate_block(hir, body, constrained_vars, errors)
        }
        HirExprKind::Lambda { params, body, .. } => {
            if let Type::Function {
                params: expected_params,
                ret,
                ..
            } = hir.engine.resolve(&expr.ty)
            {
                for (param, expected) in params.iter_mut().zip(expected_params) {
                    let _ = hir.engine.unify(&param.ty, &expected);
                    param.ty = hir.engine.resolve(&param.ty);
                }
                let _ = hir.engine.unify(&body.ty, ret.as_ref());
            }
            propagate_block(hir, body, constrained_vars, errors);
        }
        HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
            propagate_expr(hir, left, constrained_vars, errors);
            propagate_expr(hir, right, constrained_vars, errors);
        }
        HirExprKind::UnaryOp(_, inner) | HirExprKind::ArrayRepeat(inner, _) => {
            propagate_expr(hir, inner, constrained_vars, errors)
        }
        HirExprKind::Intrinsic { args, .. }
        | HirExprKind::TupleLiteral(args)
        | HirExprKind::ArrayLiteral(args) => {
            for arg in args {
                propagate_expr(hir, arg, constrained_vars, errors);
            }
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                propagate_expr(hir, &mut field.value, constrained_vars, errors);
            }
        }
        HirExprKind::EnumVariant(_, _, args, _) => {
            for arg in args {
                propagate_expr(hir, arg, constrained_vars, errors);
            }
        }
        HirExprKind::Var(_)
        | HirExprKind::ResolvedVar(_)
        | HirExprKind::IntLiteral(_)
        | HirExprKind::FloatLiteral(_)
        | HirExprKind::BoolLiteral(_)
        | HirExprKind::StringLiteral(_)
        | HirExprKind::CharLiteral(_)
        | HirExprKind::Unit => {}
    }
    expr.ty = hir.engine.resolve(&expr.ty);
}

fn propagate_function_value_instance(
    hir: &mut PartialHir,
    value_ty: Type,
    function_id: DefId,
    _constrained_vars: &HashMap<DefId, HashSet<crate::ids::TypeVarId>>,
    errors: &mut Vec<ResolveError>,
    span: &crate::lexer::Span,
) {
    let Some(function) = hir
        .functions
        .get(&function_id)
        .cloned()
        .or_else(|| find_method(hir, Some(function_id)))
    else {
        return;
    };
    if !function.generic_params.is_empty() {
        return;
    }
    let source = Type::function_with_safety(
        function
            .params
            .iter()
            .map(|param| param.ty.clone())
            .collect(),
        function.ret_type.clone(),
        FunctionSafety::from_is_unsafe(function.is_unsafe),
    );
    unify_propagated_callable(
        &mut hir.engine,
        &value_ty,
        &source,
        errors,
        &format!("named callable '{}'", function.name),
        span,
    );
}

fn propagate_method_arguments(
    hir: &mut PartialHir,
    receiver: &HirExpr,
    args: &[HirExpr],
    target: &crate::hir::HirMethodCallTarget,
) {
    let Some(method) = find_method(hir, target.method_id()) else {
        return;
    };
    let substitutions = target
        .owner_substitution
        .iter()
        .chain(&target.method_substitution)
        .map(|binding| (binding.param, binding.ty.clone()))
        .collect::<HashMap<_, _>>();
    let (receiver_param, params) = if method.self_receiver.is_some() {
        (method.params.first(), &method.params[1..])
    } else {
        (None, &method.params[..])
    };
    if let Some(param) = receiver_param {
        let mut expected = param.ty.substitute_generics(&substitutions);
        if matches!(hir.engine.resolve(&receiver.ty), Type::TypeVar(_))
            && !method.ret_type.contains_reference()
        {
            if let Type::Reference { inner, .. } = expected {
                expected = *inner;
            }
        }
        let _ = hir.engine.unify(&receiver.ty, &expected);
    }
    for (arg, param) in args.iter().zip(params) {
        if let Type::TypeVar(id) = hir.engine.resolve(&param.ty) {
            if hir.engine.get_bounds(id).iter().any(|bound| {
                hir.language_items
                    .callable_protocols()
                    .any(|(_, trait_id, _)| trait_id == bound.trait_id)
            }) {
                // This callable parameter has an independently instantiated
                // bound at the call site; do not monomorphize its declaration.
                continue;
            }
        }
        let expected = param.ty.substitute_generics(&substitutions);
        let _ = hir.engine.unify(&arg.ty, &expected);
    }
}

fn propagate_call_instance(
    hir: &mut PartialHir,
    result_ty: &Type,
    callee: &HirExpr,
    args: &mut [HirExpr],
    constrained_vars: &HashMap<DefId, HashSet<crate::ids::TypeVarId>>,
    errors: &mut Vec<ResolveError>,
) {
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        return;
    };
    let crate::hir::HirVarTarget::Function(function_id) = reference.target else {
        return;
    };
    let Some(function) = hir
        .functions
        .get(&function_id)
        .cloned()
        .or_else(|| find_method(hir, Some(function_id)))
    else {
        return;
    };
    if !function.generic_params.is_empty() {
        let source_ret = hir.engine.resolve(&function.ret_type);
        if !matches!(source_ret, Type::Generic(_)) {
            let resolved_callee = hir.engine.resolve(&callee.ty);
            if let Type::Function { params, ret, .. } = resolved_callee {
                for (arg, expected) in args.iter().zip(params.iter()) {
                    propagate_argument_type(&mut hir.engine, &arg.ty, expected, errors, &arg.span);
                }
                propagate_generic_scheme_type(&mut hir.engine, &source_ret, ret.as_ref());
                let resolved_ret = hir.engine.resolve(ret.as_ref());
                let call_result = if args.len() < params.len() {
                    Type::function_with_safety(
                        params[args.len()..].to_vec(),
                        resolved_ret,
                        FunctionSafety::from_is_unsafe(function.is_unsafe),
                    )
                } else {
                    resolved_ret
                };
                let _ = hir.engine.unify(result_ty, &call_result);
            }
        }
        return;
    }
    if function.params.iter().any(|param| param.is_ref) {
        return;
    }
    let shared_vars = constrained_vars
        .get(&function_id)
        .cloned()
        .unwrap_or_default();
    let source_type = Type::function_with_safety(
        function
            .params
            .iter()
            .map(|param| hir.engine.resolve(&param.ty))
            .collect(),
        hir.engine.resolve(&function.ret_type),
        crate::types::FunctionSafety::from_is_unsafe(function.is_unsafe),
    );
    if !contains_recovery_type(&source_type) {
        let _ = hir.engine.unify(&callee.ty, &source_type);
    }
    let resolved_callee = hir.engine.resolve(&callee.ty);
    let resolved_result = hir.engine.resolve(result_ty);
    if let Type::Function { params, ret, .. } = resolved_callee {
        let mut source_partial_result = None;
        if let Type::Function {
            params: source_params,
            ret: source_ret,
            ..
        } = source_type
        {
            let mut instance_bindings = HashMap::new();
            for (instance, source) in params.iter().zip(source_params.iter()) {
                if contains_recovery_type(source) {
                    propagate_instance_relation(
                        &mut hir.engine,
                        source,
                        instance,
                        &mut instance_bindings,
                        &shared_vars,
                    );
                } else {
                    let _ = hir.engine.unify(instance, source);
                }
            }
            if contains_recovery_type(source_ret.as_ref()) {
                propagate_instance_relation(
                    &mut hir.engine,
                    source_ret.as_ref(),
                    ret.as_ref(),
                    &mut instance_bindings,
                    &shared_vars,
                );
            } else {
                let _ = hir.engine.unify(ret.as_ref(), source_ret.as_ref());
            }
            if args.len() < source_params.len() {
                source_partial_result = Some(Type::function_with_safety(
                    source_params[args.len()..].to_vec(),
                    *source_ret,
                    FunctionSafety::from_is_unsafe(function.is_unsafe),
                ));
            }
        }
        for (arg, expected) in args.iter().zip(params.iter()) {
            propagate_argument_type(&mut hir.engine, &arg.ty, expected, errors, &arg.span);
        }
        if let Some(source_result) = source_partial_result {
            let _ = hir.engine.unify(result_ty, &source_result);
        }
        let _ = hir.engine.unify(
            result_ty,
            &if args.len() < params.len() {
                Type::function_with_safety(
                    params[args.len()..].to_vec(),
                    *ret,
                    FunctionSafety::Safe,
                )
            } else {
                *ret
            },
        );
        return;
    }
    if !contains_recovery_type(&resolved_callee) && !contains_recovery_type(&resolved_result) {
        return;
    }
    let source_params = function
        .params
        .iter()
        .map(|param| hir.engine.resolve(&param.ty))
        .collect::<Vec<_>>();
    let source_ret = hir.engine.resolve(&function.ret_type);
    let instance_params = source_params;
    let instance_ret = source_ret;
    let instance_type = Type::function_with_safety(
        instance_params,
        instance_ret,
        crate::types::FunctionSafety::from_is_unsafe(function.is_unsafe),
    );
    let before = hir.engine.substitution_generation();
    let _ = hir.engine.unify(&callee.ty, &instance_type);
    let Type::Function { params, ret, .. } = instance_type else {
        return;
    };
    let instance_ret = *ret;
    for (arg, expected) in args.iter().zip(params.iter()) {
        propagate_argument_type(&mut hir.engine, &arg.ty, expected, errors, &arg.span);
    }
    let result_type = if args.len() < params.len() {
        Type::function_with_safety(
            params[args.len()..].to_vec(),
            instance_ret.clone(),
            crate::types::FunctionSafety::from_is_unsafe(function.is_unsafe),
        )
    } else {
        instance_ret
    };
    let _ = hir.engine.unify(result_ty, &result_type);
    if before == hir.engine.substitution_generation() {
        return;
    }
}

fn propagate_argument_type(
    engine: &mut crate::infer::InferenceEngine,
    actual: &Type,
    expected: &Type,
    errors: &mut Vec<ResolveError>,
    span: &crate::lexer::Span,
) {
    if let (
        Type::Function {
            ret: actual_ret, ..
        },
        Type::Function {
            ret: expected_ret, ..
        },
    ) = (engine.resolve(actual), engine.resolve(expected))
    {
        unify_propagated_callable(
            engine,
            actual_ret.as_ref(),
            expected_ret.as_ref(),
            errors,
            "callable return",
            span,
        );
    }
    unify_propagated_callable(engine, actual, expected, errors, "callable argument", span);
}

fn unify_propagated_callable(
    engine: &mut crate::infer::InferenceEngine,
    actual: &Type,
    expected: &Type,
    errors: &mut Vec<ResolveError>,
    context: &str,
    span: &crate::lexer::Span,
) -> bool {
    let mut probe = engine.clone_for_probe();
    match probe.unify(actual, expected) {
        Ok(()) => {
            engine.commit_probe(probe);
            true
        }
        Err(error)
            if !contains_recovery_type(&engine.resolve(actual))
                && !contains_recovery_type(&engine.resolve(expected)) =>
        {
            errors.push(ResolveError::with_span(
                format!("{context} type mismatch: {}", error.render(engine)),
                span.clone(),
            ));
            false
        }
        Err(_) => false,
    }
}

fn propagate_instance_relation(
    engine: &mut crate::infer::InferenceEngine,
    source: &Type,
    instance: &Type,
    bindings: &mut HashMap<crate::ids::TypeVarId, Type>,
    shared: &HashSet<crate::ids::TypeVarId>,
) {
    let source = engine.resolve(source);
    let instance = engine.resolve(instance);
    match (&source, &instance) {
        (Type::TypeVar(id), _) => {
            if shared.contains(id) {
                let _ = engine.unify(&source, &instance);
                bindings.insert(*id, instance);
            } else if let Some(bound) = bindings.get(id).cloned() {
                let _ = engine.unify(&bound, &instance);
            } else {
                bindings.insert(*id, instance);
            }
        }
        (source, Type::TypeVar(_)) if !matches!(source, Type::TypeVar(_)) => {
            let mut source_vars = HashSet::new();
            collect_type_vars(source, &mut source_vars);
            for id in source_vars {
                bindings.entry(id).or_insert_with(|| {
                    let kind = engine.kind_of_type_var(id);
                    engine.fresh_type_var_of_kind(kind)
                });
            }
            let specialized = source.substitute(bindings);
            let _ = engine.unify(&specialized, &instance);
        }
        (
            Type::Reference { inner: source, .. },
            Type::Reference {
                inner: instance, ..
            },
        )
        | (Type::Pointer(source), Type::Pointer(instance))
        | (Type::Slice(source), Type::Slice(instance)) => {
            propagate_instance_relation(engine, source, instance, bindings, shared);
        }
        (Type::Array(source, source_len), Type::Array(instance, instance_len))
            if source_len == instance_len =>
        {
            propagate_instance_relation(engine, source, instance, bindings, shared);
        }
        (Type::Tuple(source), Type::Tuple(instance)) => {
            for (source, instance) in source.iter().zip(instance) {
                propagate_instance_relation(engine, source, instance, bindings, shared);
            }
        }
        (
            Type::Function {
                params: source_params,
                ret: source_ret,
                ..
            },
            Type::Function {
                params: instance_params,
                ret: instance_ret,
                ..
            },
        ) => {
            for (source, instance) in source_params.iter().zip(instance_params) {
                propagate_instance_relation(engine, source, instance, bindings, shared);
            }
            propagate_instance_relation(engine, source_ret, instance_ret, bindings, shared);
        }
        (
            Type::Struct {
                id: source_id,
                args: source_args,
            },
            Type::Struct {
                id: instance_id,
                args: instance_args,
            },
        )
        | (
            Type::Enum {
                id: source_id,
                args: source_args,
            },
            Type::Enum {
                id: instance_id,
                args: instance_args,
            },
        ) if source_id == instance_id => {
            for (source, instance) in source_args.iter().zip(instance_args) {
                propagate_instance_relation(engine, source, instance, bindings, shared);
            }
        }
        (
            Type::Apply {
                constructor: source_constructor,
                args: source_args,
            },
            Type::Apply {
                constructor: instance_constructor,
                args: instance_args,
            },
        ) => {
            propagate_instance_relation(
                engine,
                source_constructor,
                instance_constructor,
                bindings,
                shared,
            );
            for (source, instance) in source_args.iter().zip(instance_args) {
                propagate_instance_relation(engine, source, instance, bindings, shared);
            }
        }
        _ => {}
    }
}

fn propagate_generic_scheme_type(
    engine: &mut crate::infer::InferenceEngine,
    source: &Type,
    instance: &Type,
) {
    let source = engine.resolve(source);
    let instance = engine.resolve(instance);
    if matches!(source, Type::Generic(_)) {
        return;
    }
    match (&source, &instance) {
        (Type::TypeVar(_), _) => {}
        (Type::Apply { constructor, .. }, _)
            if matches!(constructor.as_ref(), Type::Generic(_)) => {}
        (
            Type::Reference {
                mutable: source_mutable,
                inner: source_inner,
            },
            Type::Reference {
                mutable: instance_mutable,
                inner: instance_inner,
            },
        ) if source_mutable == instance_mutable => {
            propagate_generic_scheme_type(engine, source_inner, instance_inner);
        }
        (Type::Pointer(source), Type::Pointer(instance))
        | (Type::Slice(source), Type::Slice(instance)) => {
            propagate_generic_scheme_type(engine, source, instance);
        }
        (Type::Array(source, source_len), Type::Array(instance, instance_len))
            if source_len == instance_len =>
        {
            propagate_generic_scheme_type(engine, source, instance);
        }
        (Type::Tuple(source), Type::Tuple(instance)) if source.len() == instance.len() => {
            for (source, instance) in source.iter().zip(instance) {
                propagate_generic_scheme_type(engine, source, instance);
            }
        }
        (
            Type::Function {
                params: source_params,
                ret: source_ret,
                ..
            },
            Type::Function {
                params: instance_params,
                ret: instance_ret,
                ..
            },
        ) => {
            for (source, instance) in source_params.iter().zip(instance_params) {
                propagate_generic_scheme_type(engine, source, instance);
            }
            propagate_generic_scheme_type(engine, source_ret, instance_ret);
        }
        (
            Type::Struct {
                id: source_id,
                args: source_args,
            },
            Type::Struct {
                id: instance_id,
                args: instance_args,
            },
        )
        | (
            Type::Enum {
                id: source_id,
                args: source_args,
            },
            Type::Enum {
                id: instance_id,
                args: instance_args,
            },
        ) if source_id == instance_id => {
            for (source, instance) in source_args.iter().zip(instance_args) {
                propagate_generic_scheme_type(engine, source, instance);
            }
        }
        (
            Type::Apply {
                constructor: source_constructor,
                args: source_args,
            },
            Type::Apply {
                constructor: instance_constructor,
                args: instance_args,
            },
        ) if source_args.len() == instance_args.len() => {
            propagate_generic_scheme_type(engine, source_constructor, instance_constructor);
            for (source, instance) in source_args.iter().zip(instance_args) {
                propagate_generic_scheme_type(engine, source, instance);
            }
        }
        _ => {
            let _ = engine.unify(&source, &instance);
        }
    }
}

fn propagate_method_result(
    hir: &mut PartialHir,
    result_ty: Type,
    target: &crate::hir::HirMethodCallTarget,
    errors: &mut Vec<ResolveError>,
    span: &crate::lexer::Span,
) {
    let Some(method) = find_method(hir, target.method_id()) else {
        return;
    };
    let substitutions = target
        .owner_substitution
        .iter()
        .chain(&target.method_substitution)
        .map(|binding| (binding.param, binding.ty.clone()))
        .collect::<HashMap<_, _>>();
    let method_result = method.ret_type.substitute_generics(&substitutions);
    if !matches!(method_result, Type::Function { .. }) {
        return;
    }
    if let Err(error) = hir.engine.unify(&method_result, &result_ty) {
        errors.push(ResolveError::with_span(
            format!(
                "inferred method result for '{}' is incompatible with its solved scheme: {}",
                method.name,
                error.render(&hir.engine)
            ),
            span.clone(),
        ));
    }
}

fn find_method(hir: &PartialHir, method_id: Option<DefId>) -> Option<HirFunction> {
    let method_id = method_id?;
    hir.impls
        .values()
        .flat_map(|imp| imp.methods.values())
        .find(|method| method.id == method_id)
        .cloned()
        .or_else(|| {
            hir.traits
                .values()
                .flat_map(|trait_def| trait_def.methods.values())
                .find(|method| method.id == method_id)
                .cloned()
        })
}

fn collect_unresolved_authority_vars(
    block: &HirBlock,
    engine: &crate::infer::InferenceEngine,
    output: &mut HashSet<crate::ids::TypeVarId>,
) {
    fn expr(
        node: &HirExpr,
        engine: &crate::infer::InferenceEngine,
        output: &mut HashSet<crate::ids::TypeVarId>,
    ) {
        match &node.kind {
            HirExprKind::Call(callee, args, target) => {
                if target.is_none() {
                    if let HirExprKind::FieldAccess(receiver, _, _) = &callee.kind {
                        collect_type_vars(&engine.resolve(&receiver.ty), output);
                    }
                }
                if !matches!(callee.kind, HirExprKind::FieldAccess(..))
                    && matches!(callee.ty, Type::TypeVar(_))
                {
                    collect_type_vars(&engine.resolve(&callee.ty), output);
                    collect_type_vars(&engine.resolve(&node.ty), output);
                }
                expr(callee, engine, output);
                for arg in args {
                    expr(arg, engine, output);
                }
            }
            HirExprKind::Try {
                expr: operand,
                branch_method,
                from_residual_target,
                return_ty,
                ..
            } => {
                if branch_method.is_none() {
                    collect_type_vars(&engine.resolve(&operand.ty), output);
                }
                if from_residual_target.is_none() {
                    collect_type_vars(&engine.resolve(return_ty), output);
                }
                expr(operand, engine, output);
            }
            HirExprKind::MethodCall(receiver, _, args, _, _) => {
                expr(receiver, engine, output);
                for arg in args {
                    expr(arg, engine, output);
                }
            }
            HirExprKind::FieldAccess(inner, _, _)
            | HirExprKind::Deref(inner)
            | HirExprKind::Ref(_, inner)
            | HirExprKind::Cast(inner, _)
            | HirExprKind::TupleIndex(inner, _)
            | HirExprKind::UnaryOp(_, inner)
            | HirExprKind::ArrayRepeat(inner, _) => expr(inner, engine, output),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                expr(condition, engine, output);
                collect_unresolved_authority_vars(then_branch, engine, output);
                if let Some(else_branch) = else_branch {
                    collect_unresolved_authority_vars(else_branch, engine, output);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                expr(scrutinee, engine, output);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        expr(guard, engine, output);
                    }
                    collect_unresolved_authority_vars(&arm.body, engine, output);
                }
            }
            HirExprKind::While { condition, body } => {
                expr(condition, engine, output);
                collect_unresolved_authority_vars(body, engine, output);
            }
            HirExprKind::For { iter, body, .. } => {
                expr(iter, engine, output);
                collect_unresolved_authority_vars(body, engine, output);
            }
            HirExprKind::Block(body)
            | HirExprKind::Loop(body)
            | HirExprKind::UnsafeBlock(body)
            | HirExprKind::Lambda { body, .. } => {
                collect_unresolved_authority_vars(body, engine, output)
            }
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                expr(left, engine, output);
                expr(right, engine, output);
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    expr(arg, engine, output);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    expr(&field.value, engine, output);
                }
            }
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit => {}
        }
    }

    for statement in &block.stmts {
        match statement {
            HirStmt::Let { value, .. }
            | HirStmt::Expr(value)
            | HirStmt::Return(Some(value))
            | HirStmt::Break(Some(value)) => expr(value, engine, output),
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
        }
    }
}

fn materialize_pending(
    hir: &mut PartialHir,
    strict: bool,
    try_strict: bool,
    owners: Option<&HashSet<ConstraintOwner>>,
    selected_site: Option<AuthoritySiteId>,
) -> Result<bool, Vec<ResolveError>> {
    let struct_ids = hir
        .structs
        .values()
        .map(|structure| (structure.id, structure.clone()))
        .collect();
    let traits = hir
        .traits
        .values()
        .map(|trait_def| (trait_def.id, trait_def.clone()))
        .collect();
    let impls = hir
        .impls
        .iter()
        .map(|(&id, impl_def)| (id, impl_def.clone()))
        .collect();
    let functions = hir.functions.clone();
    let mut context = MethodAuthorityContext {
        language_items: hir.language_items.clone(),
        functions,
        traits: &traits,
        impls: &impls,
        sized_trait_id: hir
            .language_items
            .sized
            .as_ref()
            .map(|items| items.trait_id),
        bounds: HirGenericBounds::new(),
        constraint_store: &mut hir.constraint_store,
        effective_trait_methods: &hir.imported_effective_trait_methods,
        resolver: &hir.resolver,
        structs: &struct_ids,
        engine: RefCell::new(&mut hir.engine),
        try_protocol: hir.language_items.try_protocol.clone(),
        unsafe_context: Cell::new(false),
        mutable_locals: HashSet::new(),
        local_bindings: HashMap::new(),
        next_local_id: 0,
        ambiguous: false,
        selected_site,
        strict,
        try_strict,
    };

    let mut errors = Vec::new();
    let mut function_ids = hir.functions.keys().copied().collect::<Vec<_>>();
    function_ids.sort();
    for id in function_ids {
        if !owner_selected(owners, ConstraintOwner::Body(id)) {
            continue;
        }
        if let Some(function) = hir.functions.get_mut(&id) {
            context.materialize_function(function, &mut errors);
            context.functions.insert(id, function.clone());
        }
    }
    let mut trait_ids = hir.traits.keys().copied().collect::<Vec<_>>();
    trait_ids.sort();
    for id in trait_ids {
        if let Some(trait_def) = hir.traits.get_mut(&id) {
            let mut methods = trait_def
                .methods
                .iter()
                .map(|(name, function)| (function.id, name.clone()))
                .collect::<Vec<_>>();
            methods.sort_by_key(|(method_id, _)| *method_id);
            for (_, name) in methods {
                if !owner_selected(owners, ConstraintOwner::Body(trait_def.methods[&name].id)) {
                    continue;
                }
                if let Some(function) = trait_def.methods.get_mut(&name) {
                    context.materialize_function(function, &mut errors);
                }
            }
        }
    }
    let mut impl_ids = hir.impls.keys().copied().collect::<Vec<_>>();
    impl_ids.sort();
    for id in impl_ids {
        if let Some(imp) = hir.impls.get_mut(&id) {
            let mut methods = imp
                .methods
                .iter()
                .map(|(name, function)| (function.id, name.clone()))
                .collect::<Vec<_>>();
            methods.sort_by_key(|(method_id, _)| *method_id);
            for (_, name) in methods {
                if !owner_selected(owners, ConstraintOwner::Body(imp.methods[&name].id)) {
                    continue;
                }
                if let Some(function) = imp.methods.get_mut(&name) {
                    context.materialize_function(function, &mut errors);
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(context.ambiguous)
    } else {
        Err(errors)
    }
}

pub(super) fn finalize_method_instantiations(
    hir: &mut PartialHir,
) -> Result<(), Vec<ResolveError>> {
    materialize_pending(hir, true, true, None, None).map(|_| ())
}

fn binding_sort_key(binding: &HirTypeBinding) -> (u32, u32, u32) {
    (
        binding.param.owner.crate_id.0,
        binding.param.owner.local.0,
        binding.param.index,
    )
}

struct MethodAuthorityContext<'a> {
    language_items: crate::hir::HirLanguageItems,
    functions: HashMap<DefId, HirFunction>,
    traits: &'a HashMap<DefId, crate::hir::HirTrait>,
    impls: &'a HashMap<DefId, crate::hir::HirImpl>,
    sized_trait_id: Option<DefId>,
    bounds: HirGenericBounds,
    constraint_store: &'a mut ConstraintStore,
    effective_trait_methods: &'a HashMap<(DefId, DefId), DefId>,
    resolver: &'a crate::collect::resolver::ResolverTables,
    structs: &'a HashMap<DefId, crate::hir::HirStruct>,
    engine: RefCell<&'a mut crate::infer::InferenceEngine>,
    try_protocol: Option<TryLanguageItems<DefId>>,
    unsafe_context: Cell<bool>,
    mutable_locals: HashSet<HirLocalId>,
    local_bindings: HashMap<HirLocalId, (String, Type, bool)>,
    next_local_id: u32,
    ambiguous: bool,
    selected_site: Option<AuthoritySiteId>,
    strict: bool,
    try_strict: bool,
}

impl MethodAuthorityContext<'_> {
    fn display_type(&self, ty: &Type) -> String {
        self.engine.borrow().display_type(ty)
    }

    fn display_selection_error(&self, error: &crate::selection::SelectionDiagnostic) -> String {
        error.message_with_names(
            |ty| self.display_type(ty),
            |id| self.resolver.canonical_name(id).map(str::to_string),
        )
    }

    fn service(&self) -> SelectionService<'_> {
        SelectionService::new(
            self.traits,
            self.impls,
            self.sized_trait_id,
            None,
            &self.bounds,
        )
        .with_effective_trait_methods(self.effective_trait_methods)
    }

    fn materialize_function(&mut self, function: &mut HirFunction, errors: &mut Vec<ResolveError>) {
        self.bounds = function.generic_bounds.clone();
        self.mutable_locals.clear();
        self.local_bindings.clear();
        self.next_local_id = 0;
        self.collect_local_bindings(function);
        let previous = self.unsafe_context.replace(function.is_unsafe);
        self.materialize_block(&mut function.body, errors);
        let _ = self
            .engine
            .borrow_mut()
            .unify(&function.ret_type, &function.body.ty);
        function.ret_type = self.resolved_type(&function.ret_type);
        function.body.ty = self.resolved_type(&function.body.ty);
        self.unsafe_context.set(previous);
    }

    fn visit_authority_slot(
        &self,
        kind: AuthorityObligationKind,
        span: &crate::lexer::Span,
    ) -> bool {
        let current = AuthoritySiteId::new(kind, span);
        self.selected_site
            .as_ref()
            .is_none_or(|selected| *selected == current)
    }

    fn materialize_direct_function_call(
        &mut self,
        result_ty: &Type,
        callee: &HirExpr,
        args: &[HirExpr],
        function_id: DefId,
    ) {
        let Some(function) = self.functions.get(&function_id).cloned() else {
            return;
        };
        if function.is_method
            || !function.generic_params.is_empty()
            || function.params.iter().any(|param| param.is_ref)
        {
            return;
        }
        let source_type = Type::function_with_safety(
            function
                .params
                .iter()
                .map(|param| self.resolved_type(&param.ty))
                .collect(),
            self.resolved_type(&function.ret_type),
            FunctionSafety::from_is_unsafe(function.is_unsafe),
        );
        if !contains_recovery_type(&source_type) {
            let _ = self.engine.borrow_mut().unify(&callee.ty, &source_type);
        }
        if let Type::Function { params, ret, .. } = self.resolved_type(&callee.ty) {
            let mut engine = self.engine.borrow_mut();
            for (arg, expected) in args.iter().zip(params.iter()) {
                let _ = engine.unify(&arg.ty, expected);
            }
            let result = if args.len() < params.len() {
                Type::function_with_safety(
                    params[args.len()..].to_vec(),
                    *ret,
                    FunctionSafety::from_is_unsafe(function.is_unsafe),
                )
            } else {
                *ret
            };
            let _ = engine.unify(result_ty, &result);
            return;
        }
        let source_params = function
            .params
            .iter()
            .map(|param| self.resolved_type(&param.ty))
            .collect::<Vec<_>>();
        let source_ret = self.resolved_type(&function.ret_type);
        let mut vars = HashSet::new();
        for ty in source_params.iter().chain(std::iter::once(&source_ret)) {
            collect_type_vars(ty, &mut vars);
        }
        let substitutions = vars
            .into_iter()
            .map(|id| {
                let kind = self.engine.borrow().kind_of_type_var(id);
                let fresh = self
                    .engine
                    .borrow_mut()
                    .fresh_type_var_at_kind(callee.span.clone(), kind);
                (id, fresh)
            })
            .collect::<HashMap<_, _>>();
        let params = source_params
            .iter()
            .map(|ty| ty.substitute(&substitutions))
            .collect::<Vec<_>>();
        let ret = source_ret.substitute(&substitutions);
        let instance = Type::function_with_safety(
            params.clone(),
            ret.clone(),
            FunctionSafety::from_is_unsafe(function.is_unsafe),
        );
        let mut engine = self.engine.borrow_mut();
        let _ = engine.unify(&callee.ty, &instance);
        for (arg, expected) in args.iter().zip(params.iter()) {
            let _ = engine.unify(&arg.ty, expected);
        }
        let result = if args.len() < params.len() {
            Type::function_with_safety(
                params[args.len()..].to_vec(),
                ret,
                FunctionSafety::from_is_unsafe(function.is_unsafe),
            )
        } else {
            ret
        };
        let _ = engine.unify(result_ty, &result);
    }

    fn materialize_block(&mut self, block: &mut HirBlock, errors: &mut Vec<ResolveError>) {
        for stmt in &mut block.stmts {
            match stmt {
                HirStmt::Let { value, .. }
                | HirStmt::Expr(value)
                | HirStmt::Return(Some(value))
                | HirStmt::Break(Some(value)) => self.materialize_expr(value, errors),
                HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
            }
        }
    }

    fn collect_local_bindings(&mut self, function: &HirFunction) {
        for param in &function.params {
            self.record_local_binding(param.local_id, &param.name, param.ty.clone(), param.mutable);
        }
        self.collect_local_bindings_block(&function.body);
    }

    fn record_local_binding(&mut self, local_id: HirLocalId, name: &str, ty: Type, mutable: bool) {
        self.next_local_id = self.next_local_id.max(local_id.0.saturating_add(1));
        if mutable {
            self.mutable_locals.insert(local_id);
        }
        self.local_bindings
            .insert(local_id, (name.to_string(), ty, mutable));
    }

    fn collect_local_bindings_block(&mut self, block: &HirBlock) {
        for stmt in &block.stmts {
            match stmt {
                HirStmt::Let {
                    name,
                    local_id,
                    ty,
                    value,
                    mutable,
                } => {
                    self.record_local_binding(*local_id, name, ty.clone(), *mutable);
                    self.collect_local_bindings_expr(value);
                }
                HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Break(Some(expr)) => {
                    self.collect_local_bindings_expr(expr)
                }
                HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
            }
        }
    }

    fn collect_local_bindings_expr(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.collect_local_bindings_expr(condition);
                self.collect_local_bindings_block(then_branch);
                if let Some(else_branch) = else_branch {
                    self.collect_local_bindings_block(else_branch);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.collect_local_bindings_expr(scrutinee);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.collect_local_bindings_expr(guard);
                    }
                    self.collect_local_bindings_block(&arm.body);
                }
            }
            HirExprKind::While { condition, body } => {
                self.collect_local_bindings_expr(condition);
                self.collect_local_bindings_block(body);
            }
            HirExprKind::For { iter, body, .. } => {
                self.collect_local_bindings_expr(iter);
                self.collect_local_bindings_block(body);
            }
            HirExprKind::Block(body) | HirExprKind::Loop(body) | HirExprKind::UnsafeBlock(body) => {
                self.collect_local_bindings_block(body)
            }
            HirExprKind::Lambda { params, body, .. } => {
                for param in params {
                    self.record_local_binding(
                        param.local_id,
                        &param.name,
                        param.ty.clone(),
                        param.mutable,
                    );
                }
                self.collect_local_bindings_block(body);
            }
            HirExprKind::Call(callee, args, _) | HirExprKind::MethodCall(callee, _, args, _, _) => {
                self.collect_local_bindings_expr(callee);
                for arg in args {
                    self.collect_local_bindings_expr(arg);
                }
            }
            HirExprKind::Try { expr, .. }
            | HirExprKind::FieldAccess(expr, _, _)
            | HirExprKind::Deref(expr)
            | HirExprKind::Ref(_, expr)
            | HirExprKind::Cast(expr, _)
            | HirExprKind::TupleIndex(expr, _) => self.collect_local_bindings_expr(expr),
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                self.collect_local_bindings_expr(left);
                self.collect_local_bindings_expr(right);
            }
            HirExprKind::UnaryOp(_, inner) | HirExprKind::ArrayRepeat(inner, _) => {
                self.collect_local_bindings_expr(inner)
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    self.collect_local_bindings_expr(arg);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    self.collect_local_bindings_expr(&field.value);
                }
            }
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit => {}
        }
    }

    fn resolved_type(&self, ty: &Type) -> Type {
        let engine = self.engine.borrow();
        engine
            .normalize_resolved_type(ty)
            .unwrap_or_else(|_| engine.resolve(ty))
    }

    fn expr_can_autoref_mut_receiver(&self, expr: &HirExpr) -> bool {
        match &expr.kind {
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Local(local_id),
                ..
            }) => {
                self.mutable_locals.contains(local_id)
                    && !matches!(self.resolved_type(&expr.ty), Type::Reference { .. })
            }
            HirExprKind::FieldAccess(base, _, _) | HirExprKind::TupleIndex(base, _) => {
                self.expr_can_autoref_mut_receiver(base)
            }
            HirExprKind::Deref(inner) => matches!(
                self.resolved_type(&inner.ty),
                Type::Reference { mutable: true, .. }
            ),
            _ => false,
        }
    }

    fn is_borrowable_local_expr(expr: &HirExpr) -> bool {
        matches!(
            expr.kind,
            HirExprKind::FieldAccess(_, _, _)
                | HirExprKind::TupleIndex(_, _)
                | HirExprKind::ResolvedVar(HirVarRef {
                    target: HirVarTarget::Local(_),
                    ..
                })
        )
    }

    fn push_receiver_candidate(
        &self,
        candidates: &mut Vec<ReceiverCandidate>,
        seen: &mut HashSet<Type>,
        expr: HirExpr,
        adjustment: ReceiverAdjustment,
        can_autoref_mut: bool,
    ) {
        let resolved = self.resolved_type(&expr.ty);
        if seen.insert(resolved) {
            candidates.push(ReceiverCandidate {
                expr,
                adjustment,
                can_autoref_mut,
            });
        }
    }

    fn builtin_shared_ref_deref(&self, expr: HirExpr) -> Option<HirExpr> {
        let Type::Reference {
            mutable: false,
            inner,
        } = self.resolved_type(&expr.ty)
        else {
            return None;
        };
        Some(HirExpr {
            ty: *inner,
            kind: HirExprKind::Deref(Box::new(expr.clone())),
            span: expr.span,
        })
    }

    fn builtin_mut_ref_deref(&self, expr: HirExpr) -> Option<HirExpr> {
        let Type::Reference {
            mutable: true,
            inner,
        } = self.resolved_type(&expr.ty)
        else {
            return None;
        };
        Some(HirExpr {
            ty: *inner,
            kind: HirExprKind::Deref(Box::new(expr.clone())),
            span: expr.span,
        })
    }

    fn mut_ref_to_shared_ref(&self, expr: HirExpr) -> Option<HirExpr> {
        let Type::Reference {
            mutable: true,
            inner,
        } = self.resolved_type(&expr.ty)
        else {
            return None;
        };
        Some(HirExpr {
            ty: Type::Reference {
                mutable: false,
                inner,
            },
            kind: expr.kind,
            span: expr.span,
        })
    }

    fn array_ref_to_slice_ref(&self, expr: HirExpr, mutable: bool) -> Option<HirExpr> {
        let Type::Reference { inner, .. } = self.resolved_type(&expr.ty) else {
            return None;
        };
        let Type::Array(element, _) = inner.as_ref() else {
            return None;
        };
        let span = expr.span.clone();
        Some(HirExpr {
            ty: Type::Reference {
                mutable,
                inner: Box::new(Type::Slice(element.clone())),
            },
            kind: HirExprKind::Intrinsic {
                name: "ArrayRefToSlice".to_string(),
                args: vec![expr],
            },
            span,
        })
    }

    fn array_value_to_slice_ref(&self, expr: HirExpr, mutable: bool) -> Option<HirExpr> {
        if !Self::is_borrowable_local_expr(&expr) {
            return None;
        }
        let Type::Array(element, _) = self.resolved_type(&expr.ty) else {
            return None;
        };
        let span = expr.span.clone();
        let borrowed = HirExpr {
            ty: Type::Reference {
                mutable,
                inner: Box::new(expr.ty.clone()),
            },
            kind: HirExprKind::Ref(mutable, Box::new(expr)),
            span: span.clone(),
        };
        Some(HirExpr {
            ty: Type::Reference {
                mutable,
                inner: Box::new(Type::Slice(element)),
            },
            kind: HirExprKind::Intrinsic {
                name: "ArrayRefToSlice".to_string(),
                args: vec![borrowed],
            },
            span,
        })
    }

    fn apply_receiver_adjustment(&self, expr: HirExpr, adjustment: ReceiverAdjustment) -> HirExpr {
        match adjustment {
            ReceiverAdjustment::AutorefShared | ReceiverAdjustment::AutorefMut => {
                let resolved = self.resolved_type(&expr.ty);
                if !matches!(resolved, Type::Str | Type::Slice(_)) {
                    return expr;
                }
                let mutable = matches!(adjustment, ReceiverAdjustment::AutorefMut);
                let span = expr.span.clone();
                HirExpr {
                    ty: Type::Reference {
                        mutable,
                        inner: Box::new(expr.ty.clone()),
                    },
                    kind: HirExprKind::Ref(mutable, Box::new(expr)),
                    span,
                }
            }
            ReceiverAdjustment::MutToSharedRef => {
                self.mut_ref_to_shared_ref(expr.clone()).unwrap_or(expr)
            }
            ReceiverAdjustment::None
            | ReceiverAdjustment::BuiltinDeref
            | ReceiverAdjustment::TraitDeref
            | ReceiverAdjustment::ArrayRefToSliceRef
            | ReceiverAdjustment::ArrayValueToMutSliceRef
            | ReceiverAdjustment::ArrayValueToSliceRef
            | ReceiverAdjustment::ArrayValueToSliceValue => expr,
        }
    }

    fn trait_deref_candidate(&mut self, expr: HirExpr) -> Option<HirExpr> {
        let can_autoref_mut = self.expr_can_autoref_mut_receiver(&expr);
        let candidate = ReceiverCandidate {
            expr: HirExpr {
                ty: self.resolved_type(&expr.ty),
                ..expr.clone()
            },
            adjustment: ReceiverAdjustment::None,
            can_autoref_mut,
        };
        let mut selected = self.service().select_concrete_method_candidates(
            std::slice::from_ref(&candidate),
            "*",
            |ty| self.resolved_type(ty),
        );
        if can_autoref_mut {
            let has_mutable = selected.iter().any(|candidate| {
                candidate.function.as_ref().is_some_and(|function| {
                    function.self_receiver == Some(crate::types::ReceiverMode::Mut)
                })
            });
            if has_mutable {
                selected.retain(|candidate| {
                    candidate.function.as_ref().is_some_and(|function| {
                        function.self_receiver == Some(crate::types::ReceiverMode::Mut)
                    })
                });
            }
        }
        if selected.len() != 1 {
            return None;
        }
        let selected = selected.pop().expect("one deref candidate");
        let function = selected.function.clone()?;
        let return_type = self.resolved_type(&selected.return_type);
        let Type::Reference { inner, .. } = &return_type else {
            return None;
        };
        self.record_selected_bounds(&selected, &HashMap::new(), "trait deref");
        let receiver =
            self.apply_receiver_adjustment(selected.receiver.clone(), selected.receiver_adjustment);
        let target =
            selected.target_with_substitution(&HashMap::new(), |ty| self.resolved_type(ty));
        Some(HirExpr {
            ty: *inner.clone(),
            kind: HirExprKind::Deref(Box::new(HirExpr {
                ty: return_type,
                kind: HirExprKind::MethodCall(
                    Box::new(receiver),
                    function.name,
                    Vec::new(),
                    function.self_receiver,
                    Some(target),
                ),
                span: expr.span.clone(),
            })),
            span: expr.span,
        })
    }

    fn receiver_adjustment_candidates(&mut self, expr: HirExpr) -> Vec<ReceiverCandidate> {
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        let can_autoref_mut = self.expr_can_autoref_mut_receiver(&expr);
        self.push_receiver_candidate(
            &mut candidates,
            &mut seen,
            expr.clone(),
            ReceiverAdjustment::None,
            can_autoref_mut,
        );
        if let Some(mut_slice) = self.array_value_to_slice_ref(expr.clone(), true) {
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                mut_slice,
                ReceiverAdjustment::ArrayValueToMutSliceRef,
                false,
            );
        }
        if let Some(deref) = self.builtin_shared_ref_deref(expr.clone()) {
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                deref,
                ReceiverAdjustment::BuiltinDeref,
                false,
            );
        }
        if let Some(deref) = self.builtin_mut_ref_deref(expr.clone()) {
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                deref,
                ReceiverAdjustment::BuiltinDeref,
                true,
            );
        }
        if let Some(shared_ref) = self.mut_ref_to_shared_ref(expr.clone()) {
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                shared_ref.clone(),
                ReceiverAdjustment::MutToSharedRef,
                false,
            );
            if let Some(deref) = self.builtin_shared_ref_deref(shared_ref.clone()) {
                self.push_receiver_candidate(
                    &mut candidates,
                    &mut seen,
                    deref,
                    ReceiverAdjustment::BuiltinDeref,
                    false,
                );
            }
            if let Some(slice_ref) = self.array_ref_to_slice_ref(shared_ref, false) {
                self.push_receiver_candidate(
                    &mut candidates,
                    &mut seen,
                    slice_ref.clone(),
                    ReceiverAdjustment::ArrayRefToSliceRef,
                    false,
                );
                if let Some(slice_value) = self.builtin_shared_ref_deref(slice_ref) {
                    self.push_receiver_candidate(
                        &mut candidates,
                        &mut seen,
                        slice_value,
                        ReceiverAdjustment::BuiltinDeref,
                        false,
                    );
                }
            }
        }
        if !contains_recovery_type(&self.resolved_type(&expr.ty)) {
            if let Some(deref) = self.trait_deref_candidate(expr.clone()) {
                self.push_receiver_candidate(
                    &mut candidates,
                    &mut seen,
                    deref,
                    ReceiverAdjustment::TraitDeref,
                    can_autoref_mut,
                );
            }
        }
        if let Some(slice_ref) = self.array_ref_to_slice_ref(expr.clone(), false) {
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                slice_ref,
                ReceiverAdjustment::ArrayRefToSliceRef,
                false,
            );
        }
        if let Some(slice_ref) = self.array_value_to_slice_ref(expr.clone(), false) {
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                slice_ref,
                ReceiverAdjustment::ArrayValueToSliceRef,
                false,
            );
        }
        if let Some(slice_ref) = self.array_value_to_slice_ref(expr, false) {
            let span = slice_ref.span.clone();
            let slice_value = HirExpr {
                ty: match self.resolved_type(&slice_ref.ty) {
                    Type::Reference { inner, .. } => *inner,
                    ty => ty,
                },
                kind: HirExprKind::Deref(Box::new(slice_ref)),
                span,
            };
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                slice_value,
                ReceiverAdjustment::ArrayValueToSliceValue,
                false,
            );
        }
        candidates
    }

    fn record_selected_bounds(
        &mut self,
        selected: &SelectedMethod,
        substitution: &HashMap<GenericParamId, Type>,
        context: &str,
    ) {
        for (ty, bound) in &selected.pending_impl_bounds {
            self.record_trait_bound(
                ty.substitute_generics(substitution),
                TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|arg| arg.substitute_generics(substitution))
                        .collect(),
                },
                selected.receiver.span.clone(),
                context,
            );
        }
        if let Some(function) = &selected.function {
            for (generic_param, bounds) in &function.generic_bounds {
                let subject = substitution
                    .get(generic_param)
                    .cloned()
                    .unwrap_or(Type::Generic(*generic_param));
                for bound in bounds {
                    self.record_trait_bound(
                        subject.clone(),
                        TraitBound {
                            trait_id: bound.trait_id,
                            type_args: bound
                                .type_args
                                .iter()
                                .map(|arg| arg.substitute_generics(substitution))
                                .collect(),
                        },
                        selected.receiver.span.clone(),
                        context,
                    );
                }
            }
        }
    }

    fn record_trait_bound(
        &mut self,
        ty: Type,
        bound: TraitBound,
        span: crate::lexer::Span,
        context: &str,
    ) {
        let ty = self.resolved_type(&ty);
        if let Type::TypeVar(id) = ty {
            self.engine.borrow_mut().add_bound(id, bound.clone());
            self.constraint_store
                .add_trait(Type::TypeVar(id), bound, span, context);
        } else {
            self.constraint_store.add_trait(ty, bound, span, context);
        }
    }

    fn select_deferred_method(
        &mut self,
        receiver: &HirExpr,
        method_name: &str,
        args: &[HirExpr],
        _expected_ty: Option<&Type>,
        errors: &mut Vec<ResolveError>,
    ) -> Option<SelectedMethod> {
        let receiver_candidates = self.receiver_adjustment_candidates(receiver.clone());
        if let Type::TypeVar(id) = self.resolved_type(&receiver.ty) {
            let bounds = self.engine.borrow().get_bounds(id);
            if !bounds.is_empty() {
                if let Ok(selected) = self
                    .service()
                    .select_bound_method_preferring_non_ref_receiver(
                        &receiver_candidates,
                        &bounds,
                        method_name,
                        Type::TypeVar(id),
                        matches!(receiver.kind, HirExprKind::Call(_, _, _)),
                    )
                {
                    return Some(selected);
                }
            }
        }
        if let Type::Generic(param) = self.resolved_type(&receiver.ty) {
            let bounds = self.bounds.get(&param).cloned().unwrap_or_default();
            if !bounds.is_empty() {
                if let Ok(selected) = self
                    .service()
                    .select_bound_method_preferring_non_ref_receiver(
                        &receiver_candidates,
                        &bounds,
                        method_name,
                        Type::Generic(param),
                        matches!(receiver.kind, HirExprKind::Call(_, _, _)),
                    )
                {
                    return Some(selected);
                }
            }
        }
        let mut proven = Vec::new();
        let mut deferred = Vec::new();
        for candidate in &receiver_candidates {
            let mut selected = self.service().select_concrete_method_candidates(
                std::slice::from_ref(candidate),
                method_name,
                |ty| self.resolved_type(ty),
            );
            let has_proven = selected
                .iter()
                .any(|candidate| candidate.pending_impl_bounds.is_empty());
            if has_proven {
                selected.retain(|candidate| candidate.pending_impl_bounds.is_empty());
            }
            match selected.len() {
                0 => {}
                1 => {
                    if let Some(mut selected) = selected.pop() {
                        if candidate.adjustment != ReceiverAdjustment::None {
                            selected.receiver_adjustment = candidate.adjustment;
                        }
                        let destination = if selected.pending_impl_bounds.is_empty() {
                            &mut proven
                        } else {
                            &mut deferred
                        };
                        if destination.iter().all(|existing: &SelectedMethod| {
                            existing.target.target != selected.target.target
                        }) {
                            destination.push(selected);
                        }
                    }
                }
                _ => {
                    if !self.strict && contains_recovery_type(&candidate.expr.ty) {
                        self.ambiguous = true;
                        continue;
                    }
                    errors.push(ResolveError::with_span_code(
                        format!(
                            "Ambiguous selection for '{}' on type {}",
                            method_name,
                            self.display_type(&candidate.expr.ty)
                        ),
                        candidate.expr.span.clone(),
                        crate::diagnostic::DiagnosticCode::Selection,
                    ));
                    return None;
                }
            }
        }
        if proven.len() == 1 {
            return proven.pop();
        }
        if proven.len() > 1 {
            if !self.strict && contains_recovery_type(&receiver.ty) {
                self.ambiguous = true;
                return None;
            }
            errors.push(ResolveError::with_span_code(
                format!(
                    "Ambiguous selection for '{}' on type {}",
                    method_name,
                    self.display_type(&receiver.ty)
                ),
                receiver.span.clone(),
                crate::diagnostic::DiagnosticCode::Selection,
            ));
            return None;
        }
        if deferred.is_empty()
            && (contains_recovery_type(&self.resolved_type(&receiver.ty))
                || matches!(self.resolved_type(&receiver.ty), Type::Generic(_)))
        {
            let mut inferred =
                self.service()
                    .select_inferred_method_candidates(receiver, method_name, |ty| {
                        self.resolved_type(ty)
                    });
            inferred.retain(|candidate| {
                if candidate.substituted_params.len() != args.len() {
                    return false;
                }
                let mut substitution = candidate.owner_substitution.clone();
                candidate
                    .substituted_params
                    .iter()
                    .zip(args)
                    .all(|(param, arg)| {
                        let actual = self.resolved_type(&arg.ty);
                        contains_recovery_type(&actual)
                            || type_pattern_matches(&param.ty, &actual, &mut substitution)
                    })
            });
            if let Some(expected) = _expected_ty.map(|ty| self.resolved_type(ty)) {
                if !contains_recovery_type(&expected) {
                    inferred.retain(|candidate| {
                        let mut substitution = candidate.owner_substitution.clone();
                        type_pattern_matches(&candidate.return_type, &expected, &mut substitution)
                    });
                }
            }
            inferred.sort_by_key(|candidate| {
                (candidate.target.impl_id(), candidate.target.method_id())
            });
            inferred.dedup_by_key(|candidate| {
                (candidate.target.impl_id(), candidate.target.method_id())
            });
            if inferred.len() == 1 {
                let mut selected = inferred.into_iter().next();
                if let Some(selected) = selected.as_mut() {
                    let target_key = (selected.target.impl_id(), selected.target.method_id());
                    for candidate in &receiver_candidates {
                        let concrete = self.service().select_concrete_method_candidates(
                            std::slice::from_ref(candidate),
                            method_name,
                            |ty| self.resolved_type(ty),
                        );
                        if concrete.iter().any(|candidate| {
                            (candidate.target.impl_id(), candidate.target.method_id()) == target_key
                        }) {
                            if candidate.adjustment != ReceiverAdjustment::None {
                                selected.receiver_adjustment = candidate.adjustment;
                            }
                            break;
                        }
                    }
                    selected.receiver.kind = receiver.kind.clone();
                    selected.receiver.span = receiver.span.clone();
                }
                return selected;
            }
            if inferred.len() > 1 {
                if !self.strict && contains_recovery_type(&receiver.ty) {
                    self.ambiguous = true;
                    return None;
                }
                if let Some(expected) = _expected_ty.map(|ty| self.resolved_type(ty)) {
                    if !contains_recovery_type(&expected) {
                        let narrowed = inferred
                            .iter()
                            .filter(|candidate| {
                                let mut substitution = candidate.owner_substitution.clone();
                                type_pattern_matches(
                                    &candidate.return_type,
                                    &expected,
                                    &mut substitution,
                                )
                            })
                            .count();
                        if narrowed == 0 {
                            return None;
                        }
                    }
                }
                errors.push(ResolveError::with_span(
                    format!(
                        "Ambiguous selection for '{}' on type {}",
                        method_name,
                        self.display_type(&receiver.ty)
                    ),
                    receiver.span.clone(),
                ));
                return None;
            }
        }
        if deferred.len() == 1 {
            return deferred.pop();
        }
        if deferred.len() > 1 {
            if !self.strict {
                self.ambiguous = true;
                return None;
            }
            errors.push(ResolveError::with_span(
                format!(
                    "Ambiguous selection for '{}' on type {}",
                    method_name,
                    self.display_type(&receiver.ty)
                ),
                receiver.span.clone(),
            ));
            return None;
        }
        if self
            .service()
            .mut_receiver_method_requires_mutable_receiver(
                &receiver_candidates,
                method_name,
                |ty| self.resolved_type(ty),
            )
        {
            errors.push(ResolveError::with_span(
                format!(
                    "Cannot call mutable receiver method '{}' without a mutable receiver",
                    method_name
                ),
                receiver.span.clone(),
            ));
        }
        None
    }

    fn materialize_expr(&mut self, expr: &mut HirExpr, errors: &mut Vec<ResolveError>) {
        if matches!(&expr.kind, HirExprKind::Call(callee, _, _) if !native_call_type(&callee.ty) && !matches!(callee.kind, HirExprKind::FieldAccess(_, _, None)))
            && (self.visit_authority_slot(AuthorityObligationKind::CallableCall, &expr.span)
                || self.visit_authority_slot(AuthorityObligationKind::DeferredCall, &expr.span))
        {
            self.materialize_value_call(expr, errors);
        }
        match &mut expr.kind {
            HirExprKind::Call(callee, args, target) => {
                let deferred_member = matches!(callee.kind, HirExprKind::FieldAccess(_, _, _));
                let selected_site = deferred_member.then(|| {
                    self.visit_authority_slot(AuthorityObligationKind::DeferredCall, &expr.span)
                });
                if target.is_some() {
                    if deferred_member {
                        if let HirExprKind::FieldAccess(receiver, _, _) = &mut callee.kind {
                            self.materialize_expr(receiver, errors);
                        }
                    } else {
                        self.materialize_expr(callee, errors);
                    }
                    for arg in args.iter_mut() {
                        self.materialize_expr(arg, errors);
                    }
                    self.materialize_callable_arguments(callee, args, errors);
                    if let Some(crate::hir::HirCallTarget::Function(function_id)) = target.as_ref()
                    {
                        self.materialize_direct_function_call(&expr.ty, callee, args, *function_id);
                    }
                    return;
                }
                let (mut receiver, method_name) = match &callee.kind {
                    HirExprKind::FieldAccess(receiver, method_name, None) => {
                        (receiver.as_ref().clone(), method_name.clone())
                    }
                    _ => {
                        self.materialize_expr(callee, errors);
                        for arg in args.iter_mut() {
                            self.materialize_expr(arg, errors);
                        }
                        self.materialize_callable_arguments(callee, args, errors);
                        if matches!(callee.kind, HirExprKind::FieldAccess(_, _, Some(_))) {
                            self.materialize_value_call(expr, errors);
                        }
                        return;
                    }
                };
                if selected_site == Some(false) {
                    self.materialize_expr(&mut receiver, errors);
                    for arg in args.iter_mut() {
                        self.materialize_expr(arg, errors);
                    }
                    return;
                }
                self.materialize_expr(&mut receiver, errors);
                for arg in args.iter_mut() {
                    self.materialize_expr(arg, errors);
                }
                let Some(mut selected) = self.select_deferred_method(
                    &receiver,
                    &method_name,
                    args,
                    Some(&expr.ty),
                    errors,
                ) else {
                    if !self.strict && contains_recovery_type(&self.resolved_type(&receiver.ty)) {
                        return;
                    }
                    self.materialize_field_access(callee, errors);
                    if matches!(callee.kind, HirExprKind::FieldAccess(_, _, Some(_))) {
                        self.materialize_value_call(expr, errors);
                    }
                    return;
                };
                selected.receiver = receiver.clone();
                let adjusted_receiver = match selected.receiver_adjustment {
                    ReceiverAdjustment::TraitDeref => self
                        .trait_deref_candidate(receiver.clone())
                        .unwrap_or_else(|| receiver.clone()),
                    adjustment => self.apply_receiver_adjustment(receiver.clone(), adjustment),
                };

                let Some(function) = selected.function.as_ref() else {
                    errors.push(ResolveError::with_span(
                        format!("selected method '{}' has no executable body", method_name),
                        expr.span.clone(),
                    ));
                    return;
                };
                if selected.substituted_params.len() != args.len() {
                    errors.push(ResolveError::with_span(
                        format!(
                            "selected method '{}' expects {} arguments but received {}",
                            method_name,
                            selected.substituted_params.len(),
                            args.len()
                        ),
                        expr.span.clone(),
                    ));
                    return;
                }

                let mut substitution = selected.owner_substitution.clone();
                for (param, arg) in selected.substituted_params.iter().zip(args.iter_mut()) {
                    let mut actual = self.engine.borrow().resolve(&arg.ty);
                    let expected = param.ty.substitute_generics(&substitution);
                    if matches!(expected, Type::Reference { mutable: false, .. })
                        && !matches!(actual, Type::Reference { .. })
                    {
                        let borrowed = HirExpr {
                            ty: Type::Reference {
                                mutable: false,
                                inner: Box::new(actual.clone()),
                            },
                            kind: HirExprKind::Ref(false, Box::new(arg.clone())),
                            span: arg.span.clone(),
                        };
                        *arg = borrowed;
                        actual = self.engine.borrow().resolve(&arg.ty);
                    }
                    let mut matches = type_pattern_matches(&param.ty, &actual, &mut substitution);
                    if !matches {
                        if !contains_inference_type(&expected) {
                            if let (
                                Type::Reference {
                                    mutable: expected_mutable,
                                    inner: expected_inner,
                                },
                                Type::Reference {
                                    mutable: actual_mutable,
                                    inner: actual_inner,
                                },
                            ) = (&expected, &actual)
                            {
                                if (!expected_mutable || *actual_mutable)
                                    && matches!(expected_inner.as_ref(), Type::Slice(_))
                                    && matches!(actual_inner.as_ref(), Type::Array(_, _))
                                {
                                    if let Some(coerced) =
                                        self.array_ref_to_slice_ref(arg.clone(), *expected_mutable)
                                    {
                                        if self
                                            .engine
                                            .borrow_mut()
                                            .unify(&coerced.ty, &expected)
                                            .is_ok()
                                        {
                                            *arg = coerced;
                                        }
                                    }
                                }
                            }
                            let _ = self.engine.borrow_mut().unify(&arg.ty, &expected);
                            actual = self.engine.borrow().resolve(&arg.ty);
                            matches = type_pattern_matches(&param.ty, &actual, &mut substitution);
                        }
                    }
                    if !matches {
                        if !self.strict {
                            return;
                        }
                        errors.push(ResolveError::with_span(
                            format!(
                                "selected method '{}' argument type {} does not match {}",
                                method_name,
                                self.display_type(&arg.ty),
                                self.display_type(&param.ty)
                            ),
                            arg.span.clone(),
                        ));
                        return;
                    }
                }
                self.record_selected_bounds(&selected, &substitution, "deferred method call");
                let actual_return = self.engine.borrow().resolve(&expr.ty);
                let _ =
                    type_pattern_matches(&selected.return_type, &actual_return, &mut substitution);

                let owner_params = selected
                    .target
                    .owner_substitution
                    .iter()
                    .map(|binding| binding.param)
                    .collect::<std::collections::HashSet<_>>();
                let mut method_substitution = Vec::new();
                for param in &function.generic_params {
                    if owner_params.contains(&param.id)
                        || selected
                            .target
                            .trait_id()
                            .is_some_and(|id| param.id.owner == id)
                    {
                        continue;
                    }
                    let Some(ty) = substitution.get(&param.id) else {
                        if !self.strict {
                            return;
                        }
                        errors.push(ResolveError::with_span(
                            format!(
                                "selected method '{}' leaves a generic parameter unresolved",
                                method_name
                            ),
                            expr.span.clone(),
                        ));
                        return;
                    };
                    method_substitution.push(HirTypeBinding {
                        param: param.id,
                        ty: ty.clone(),
                    });
                }
                method_substitution.sort_by_key(binding_sort_key);

                let mut authority = selected.target.clone();
                authority.method_substitution = method_substitution;
                let selected_return = selected.return_type.substitute_generics(&substitution);
                let selected_callable = Type::function_with_safety(
                    selected
                        .substituted_params
                        .iter()
                        .map(|param| param.ty.substitute_generics(&substitution))
                        .collect(),
                    selected_return.clone(),
                    FunctionSafety::from_is_unsafe(function.is_unsafe),
                );
                {
                    let mut engine = self.engine.borrow_mut();
                    if let Err(error) = engine.unify(&callee.ty, &selected_callable) {
                        if self.strict {
                            errors.push(ResolveError::with_span(
                                format!(
                                    "selected method '{}' callable type does not match its deferred call: {}",
                                    method_name,
                                    error.render(&engine)
                                ),
                                expr.span.clone(),
                            ));
                            return;
                        }
                        self.ambiguous = true;
                    }
                    let _ = engine.unify(&expr.ty, &selected_return);
                    expr.ty = engine.resolve(&expr.ty);
                }
                expr.kind = HirExprKind::MethodCall(
                    Box::new(adjusted_receiver),
                    method_name,
                    std::mem::take(args),
                    function.self_receiver,
                    Some(authority),
                );
            }
            HirExprKind::MethodCall(receiver, _, args, _, target) => {
                let selected_site =
                    self.visit_authority_slot(AuthorityObligationKind::MethodCall, &expr.span);
                self.materialize_expr(receiver, errors);
                for arg in args.iter_mut() {
                    self.materialize_expr(arg, errors);
                }
                if let Some(target) = target {
                    self.materialize_method_bindings(args, &expr.ty, target, &expr.span, errors);
                }
                if selected_site && self.strict && target.is_none() {
                    errors.push(ResolveError::with_span(
                        "accepted HIR method call has no selected authority".to_string(),
                        expr.span.clone(),
                    ));
                }
            }
            HirExprKind::FieldAccess(_, _, Some(_)) => {
                self.materialize_field_access(expr, errors);
            }
            HirExprKind::FieldAccess(receiver, _, None) => {
                if self.visit_authority_slot(AuthorityObligationKind::Field, &expr.span) {
                    self.materialize_field_access(expr, errors);
                } else {
                    self.materialize_expr(receiver, errors);
                }
            }
            HirExprKind::Deref(receiver)
            | HirExprKind::Ref(_, receiver)
            | HirExprKind::Cast(receiver, _)
            | HirExprKind::TupleIndex(receiver, _) => self.materialize_expr(receiver, errors),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.materialize_expr(condition, errors);
                self.materialize_block(then_branch, errors);
                if let Some(else_branch) = else_branch {
                    self.materialize_block(else_branch, errors);
                }
            }
            HirExprKind::Block(block) | HirExprKind::Loop(block) => {
                self.materialize_block(block, errors)
            }
            HirExprKind::UnsafeBlock(block) => {
                let previous = self.unsafe_context.replace(true);
                self.materialize_block(block, errors);
                self.unsafe_context.set(previous);
            }
            HirExprKind::While { condition, body } => {
                self.materialize_expr(condition, errors);
                self.materialize_block(body, errors);
            }
            HirExprKind::For { iter, body, .. } => {
                self.materialize_expr(iter, errors);
                self.materialize_block(body, errors);
            }
            HirExprKind::Lambda { body, .. } => {
                let previous = self.unsafe_context.replace(false);
                self.materialize_block(body, errors);
                self.unsafe_context.set(previous);
            }
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                self.materialize_expr(left, errors);
                self.materialize_expr(right, errors);
            }
            HirExprKind::UnaryOp(_, inner) => self.materialize_expr(inner, errors),
            HirExprKind::ArrayRepeat(value, _) => self.materialize_expr(value, errors),
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args) => {
                for arg in args {
                    self.materialize_expr(arg, errors);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    self.materialize_expr(&mut field.value, errors);
                }
            }
            HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    self.materialize_expr(arg, errors);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.materialize_expr(scrutinee, errors);
                for arm in arms {
                    if let Some(guard) = &mut arm.guard {
                        self.materialize_expr(guard, errors);
                    }
                    self.materialize_block(&mut arm.body, errors);
                }
            }
            HirExprKind::Try { .. } => {
                let materialize_branch =
                    self.visit_authority_slot(AuthorityObligationKind::TryBranch, &expr.span);
                let materialize_residual =
                    self.visit_authority_slot(AuthorityObligationKind::FromResidual, &expr.span);
                if materialize_branch || materialize_residual {
                    self.materialize_try(expr, errors, materialize_branch, materialize_residual);
                } else if let HirExprKind::Try { expr: operand, .. } = &mut expr.kind {
                    self.materialize_expr(operand, errors);
                }
            }
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit => {}
        }
    }

    fn materialize_callable_arguments(
        &mut self,
        callee: &HirExpr,
        args: &mut [HirExpr],
        errors: &mut Vec<ResolveError>,
    ) {
        let Type::Function { params, .. } = self.resolved_type(&callee.ty) else {
            return;
        };
        if params.len() != args.len() {
            if self.strict {
                errors.push(ResolveError::with_span(
                    format!(
                        "callable expects {} arguments but received {}",
                        params.len(),
                        args.len()
                    ),
                    callee.span.clone(),
                ));
            }
            return;
        }

        for (arg, expected) in args.iter_mut().zip(params) {
            let actual = self.resolved_type(&arg.ty);
            let expected = self.resolved_type(&expected);
            if let (
                Type::Reference {
                    mutable: actual_mutable,
                    inner: actual_inner,
                },
                Type::Reference {
                    mutable: expected_mutable,
                    inner: expected_inner,
                },
            ) = (&actual, &expected)
            {
                if (!expected_mutable || *actual_mutable)
                    && matches!(actual_inner.as_ref(), Type::Array(_, _))
                    && matches!(expected_inner.as_ref(), Type::Slice(_))
                {
                    if let Some(coerced) =
                        self.array_ref_to_slice_ref(arg.clone(), *expected_mutable)
                    {
                        let mut probe = self.engine.borrow().clone_for_probe();
                        if probe.unify(&coerced.ty, &expected).is_ok() {
                            self.engine.borrow_mut().commit_probe(probe);
                            *arg = coerced;
                            continue;
                        }
                    }
                }
            }

            let mut probe = self.engine.borrow().clone_for_probe();
            match probe.unify(&actual, &expected) {
                Ok(()) => self.engine.borrow_mut().commit_probe(probe),
                Err(error) if self.strict => errors.push(ResolveError::with_span(
                    format!(
                        "callable argument type {} does not match {}: {}",
                        self.display_type(&actual),
                        self.display_type(&expected),
                        error.render(&probe)
                    ),
                    arg.span.clone(),
                )),
                Err(_) => self.ambiguous = true,
            }
        }
    }

    fn materialize_try(
        &mut self,
        expr: &mut HirExpr,
        errors: &mut Vec<ResolveError>,
        materialize_branch: bool,
        materialize_residual: bool,
    ) {
        let HirExprKind::Try {
            expr: operand,
            branch_method,
            branch_self_receiver,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            ..
        } = &mut expr.kind
        else {
            unreachable!("Try materialization called for a non-Try expression");
        };

        self.materialize_expr(operand, errors);
        {
            let mut engine = self.engine.borrow_mut();
            let _ = engine.unify(&expr.ty, output_ty);
            expr.ty = engine.resolve(&expr.ty);
        }
        let Some(protocol) = self.try_protocol.as_ref() else {
            if self.try_strict && self.strict {
                errors.push(ResolveError::with_span(
                    "Cannot use '?' because the Try language-item protocol is unavailable"
                        .to_string(),
                    expr.span.clone(),
                ));
            }
            return;
        };
        let Some(try_trait) = self.traits.get(&protocol.try_trait_id) else {
            if self.try_strict && self.strict {
                errors.push(ResolveError::with_span(
                    "Cannot use '?' because the Try language-item protocol is unavailable"
                        .to_string(),
                    expr.span.clone(),
                ));
            }
            return;
        };

        if materialize_branch && branch_method.is_none() {
            let carrier_ty = self.resolved_type(&operand.ty);
            if try_carrier_has_unknown_head(&carrier_ty) {
                if self.try_strict {
                    errors.push(ResolveError::with_span(
                        "Cannot use '?' because its carrier type is unresolved or generic"
                            .to_string(),
                        expr.span.clone(),
                    ));
                }
                return;
            }

            let receiver = operand.as_ref().clone();
            let selected = match SelectionService::new(
                self.traits,
                self.impls,
                self.sized_trait_id,
                None,
                &self.bounds,
            )
            .with_effective_trait_methods(self.effective_trait_methods)
            .select_required_trait_member(
                &receiver,
                &carrier_ty,
                protocol.try_trait_id,
                protocol.branch_method_id,
            ) {
                Ok(selected) => selected,
                Err(error) => {
                    let message = match error {
                        crate::selection::SelectionDiagnostic::NoImplementation { .. } => {
                            format!(
                                "Cannot use '?' on non-carrier type {}",
                                self.display_type(&carrier_ty)
                            )
                        }
                        _ => self.display_selection_error(&error),
                    };
                    if self.try_strict && self.strict {
                        errors.push(ResolveError::with_span(message, expr.span.clone()));
                    }
                    return;
                }
            };

            if let Some(function) = selected.function.as_ref() {
                if function.is_unsafe && !self.unsafe_context.get() {
                    if self.try_strict && self.strict {
                        errors.push(ResolveError::with_span(
                            format!(
                                "Call to unsafe function '{}' requires an unsafe block",
                                function.name
                            ),
                            expr.span.clone(),
                        ));
                    }
                    return;
                }
            }

            let Some(selected_output) =
                self.selected_try_associated_type(&selected, try_trait, protocol.output_id)
            else {
                if self.try_strict && self.strict {
                    errors.push(ResolveError::with_span(
                        "Cannot use '?' because the Try language-item output type is unavailable"
                            .to_string(),
                        expr.span.clone(),
                    ));
                }
                return;
            };
            let Some(selected_residual) =
                self.selected_try_associated_type(&selected, try_trait, protocol.residual_id)
            else {
                if self.try_strict && self.strict {
                    errors.push(ResolveError::with_span(
                        "Cannot use '?' because the Try language-item residual type is unavailable"
                            .to_string(),
                        expr.span.clone(),
                    ));
                }
                return;
            };

            {
                let mut engine = self.engine.borrow_mut();
                if let Err(error) = engine.unify(output_ty, &selected_output) {
                    if self.try_strict && self.strict {
                        errors.push(ResolveError::with_span(
                            format!(
                                "Cannot use '?' because its output type could not be resolved: {}",
                                error.render(&engine)
                            ),
                            expr.span.clone(),
                        ));
                    }
                    return;
                }
                expr.ty = engine.resolve(output_ty);
                if let Err(error) = engine.unify(residual_ty, &selected_residual) {
                    if self.try_strict && self.strict {
                        errors.push(ResolveError::with_span(
                            format!(
                                "Cannot use '?' because its residual type could not be resolved: {}",
                                error.render(&engine)
                            ),
                            expr.span.clone(),
                        ));
                    }
                    return;
                }
            }
            let selected_residual = self.resolved_type(&selected_residual);
            if try_type_head_is_unresolved(&selected_residual) {
                if self.try_strict && self.strict {
                    errors.push(ResolveError::with_span(
                        "Cannot use '?' because its Try residual type is unresolved or generic"
                            .to_string(),
                        expr.span.clone(),
                    ));
                }
                return;
            }

            let method_substitution = selected
                .function
                .as_ref()
                .map(|function| {
                    self.infer_try_method_substitution(&selected, &carrier_ty, function, &[])
                })
                .unwrap_or_default();
            let method = selected.target_with_substitution(&method_substitution, |ty| {
                self.engine.borrow().resolve(ty)
            });
            *branch_method = Some(method);
            *branch_self_receiver = selected
                .function
                .as_ref()
                .and_then(|function| function.self_receiver);
        }

        if materialize_residual && from_residual_target.is_none() {
            let resolved_return_ty = self.resolved_type(return_ty);
            if try_type_head_is_unresolved(&resolved_return_ty) {
                if self.try_strict && self.strict {
                    errors.push(ResolveError::with_span(
                        "Cannot use '?' because its enclosing return type is unresolved or generic"
                            .to_string(),
                        expr.span.clone(),
                    ));
                }
                return;
            }
            let resolved_residual_ty = self.resolved_type(residual_ty);
            let return_owner = HirExpr {
                ty: resolved_return_ty.clone(),
                kind: HirExprKind::Unit,
                span: expr.span.clone(),
            };
            let Some(from_residual_trait) = self.traits.get(&protocol.from_residual_trait_id)
            else {
                if self.try_strict && self.strict {
                    errors.push(ResolveError::with_span(
                        "Cannot use '?' because the FromResidual language-item trait is unavailable"
                            .to_string(),
                        expr.span.clone(),
                    ));
                }
                return;
            };
            let residual_arg = HirExpr {
                ty: resolved_residual_ty.clone(),
                kind: HirExprKind::Unit,
                span: expr.span.clone(),
            };
            let selected = match SelectionService::new(
                self.traits,
                self.impls,
                self.sized_trait_id,
                None,
                &self.bounds,
            )
            .with_effective_trait_methods(self.effective_trait_methods)
            .select_static_trait_member(
                &return_owner,
                &resolved_return_ty,
                from_residual_trait.id,
                std::slice::from_ref(&resolved_residual_ty),
                protocol.from_residual_method_id,
            ) {
                Ok(selected) => selected,
                Err(error) => {
                    let message = match error {
                        crate::selection::SelectionDiagnostic::NoImplementation { .. } => {
                            let return_type = self.display_type(&resolved_return_ty);
                            let residual_type = self.display_type(&resolved_residual_ty);
                            format!(
                                "Cannot use '?' because {return_type} does not implement FromResidual {residual_type}"
                            )
                        }
                        _ => self.display_selection_error(&error),
                    };
                    if self.try_strict && self.strict {
                        errors.push(ResolveError::with_span(message, expr.span.clone()));
                    }
                    return;
                }
            };
            if let Some(function) = selected.function.as_ref() {
                if function.is_unsafe && !self.unsafe_context.get() {
                    if self.try_strict && self.strict {
                        errors.push(ResolveError::with_span(
                            format!(
                                "Call to unsafe function '{}' requires an unsafe block",
                                function.name
                            ),
                            expr.span.clone(),
                        ));
                    }
                    return;
                }
            }
            let method_substitution = selected
                .function
                .as_ref()
                .map(|function| {
                    self.infer_try_method_substitution(
                        &selected,
                        &resolved_return_ty,
                        function,
                        &[residual_arg],
                    )
                })
                .unwrap_or_default();
            let method = selected.target_with_substitution(&method_substitution, |ty| {
                self.engine.borrow().resolve(ty)
            });
            *from_residual_target = Some(HirCallTarget::StaticMethod(HirStaticMethodTarget {
                owner_ty: resolved_return_ty,
                method,
            }));
        }

        let engine = self.engine.borrow();
        *output_ty = engine.resolve(output_ty);
        *residual_ty = engine.resolve(residual_ty);
        *return_ty = engine.resolve(return_ty);
        expr.ty = engine.resolve(&expr.ty);
    }

    fn selected_try_associated_type(
        &self,
        selected: &crate::selection::SelectedMethod,
        trait_def: &crate::hir::HirTrait,
        assoc_type_id: crate::ids::AssocTypeId,
    ) -> Option<Type> {
        trait_def
            .associated_types
            .iter()
            .find(|associated| associated.id == assoc_type_id)?;
        let ty = selected
            .associated_types
            .iter()
            .find(|associated| associated.id == assoc_type_id)
            .map(|associated| associated.ty.clone())?;
        Some(self.engine.borrow().resolve(&ty))
    }

    fn infer_try_method_substitution(
        &self,
        selected: &crate::selection::SelectedMethod,
        receiver_ty: &Type,
        function: &HirFunction,
        args: &[HirExpr],
    ) -> HashMap<crate::types::GenericParamId, Type> {
        let inferable = function
            .generic_params
            .iter()
            .map(|param| param.id)
            .collect::<HashSet<_>>();
        let mut engine = self.engine.borrow_mut();
        let mut substitution = function
            .generic_params
            .iter()
            .map(|param| {
                (
                    param.id,
                    engine
                        .fresh_type_var_at_kind(selected.receiver.span.clone(), param.kind.clone()),
                )
            })
            .collect::<HashMap<_, _>>();
        if function.is_method {
            if let Some(self_param) = function.params.first() {
                let expected = self_param.ty.substitute_generics(&substitution);
                let _ = engine.unify(&expected, receiver_ty);
                crate::selection::infer_generic_subst_from_types(
                    &self_param.ty,
                    receiver_ty,
                    &mut substitution,
                );
            }
        }
        let param_start = usize::from(function.is_method);
        for (param, arg) in function.params[param_start..].iter().zip(args) {
            let actual = engine.resolve(&arg.ty);
            crate::selection::infer_generic_subst_from_types(&param.ty, &actual, &mut substitution);
            let expected = param.ty.substitute_generics(&substitution);
            let _ = engine.unify(&expected, &arg.ty);
        }
        substitution.retain(|param, _| inferable.contains(param));
        for ty in substitution.values_mut() {
            *ty = engine.resolve(ty);
        }
        for (param, ty) in &selected.owner_substitution {
            substitution.insert(*param, ty.clone());
        }
        substitution
    }

    #[allow(dead_code)]
    fn fresh_local_id(&mut self) -> HirLocalId {
        let id = HirLocalId(self.next_local_id);
        self.next_local_id = self.next_local_id.saturating_add(1);
        id
    }

    #[allow(dead_code)]
    fn lambda_function_type(
        &self,
        params: Vec<Type>,
        ret: Type,
        safety: FunctionSafety,
        captures: &[HirClosureCapture],
    ) -> Type {
        let captures = captures
            .iter()
            .map(|capture| FunctionCapture {
                kind: match capture.kind {
                    HirClosureCaptureKind::SharedBorrow => CaptureKind::SharedBorrow,
                    HirClosureCaptureKind::MutableBorrow => CaptureKind::MutableBorrow,
                    HirClosureCaptureKind::Move => CaptureKind::Move,
                },
                ty: capture.ty.clone(),
            })
            .collect::<Vec<_>>();
        Type::function_with_metadata(
            params,
            ret,
            safety,
            CallableKind::from_captures(&captures),
            captures,
        )
    }

    #[allow(dead_code)]
    fn collect_method_value_captures(
        &self,
        expr: &HirExpr,
        receiver_mode: Option<crate::types::ReceiverMode>,
        captures: &mut Vec<HirClosureCapture>,
        seen: &mut HashSet<HirLocalId>,
    ) {
        match &expr.kind {
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Local(local_id),
                ..
            }) => {
                let Some((name, ty, mutable)) = self.local_bindings.get(local_id) else {
                    return;
                };
                if !seen.insert(*local_id) {
                    return;
                }
                let kind = match receiver_mode {
                    Some(crate::types::ReceiverMode::Move) => HirClosureCaptureKind::Move,
                    Some(crate::types::ReceiverMode::Mut) => HirClosureCaptureKind::MutableBorrow,
                    Some(crate::types::ReceiverMode::Shared) | None => {
                        HirClosureCaptureKind::SharedBorrow
                    }
                };
                captures.push(HirClosureCapture {
                    name: name.clone(),
                    local_id: *local_id,
                    kind,
                    mutable: *mutable,
                    ty: ty.clone(),
                });
            }
            HirExprKind::FieldAccess(base, _, _)
            | HirExprKind::TupleIndex(base, _)
            | HirExprKind::Deref(base)
            | HirExprKind::Ref(_, base)
            | HirExprKind::Cast(base, _) => {
                self.collect_method_value_captures(base, receiver_mode, captures, seen)
            }
            _ => {}
        }
    }

    #[allow(dead_code)]
    fn materialize_method_value(
        &mut self,
        expr: &mut HirExpr,
        receiver: HirExpr,
        method_name: String,
        selected: SelectedMethod,
        errors: &mut Vec<ResolveError>,
    ) {
        let Some(function) = selected.function.clone() else {
            errors.push(ResolveError::with_span(
                format!("selected method '{}' has no executable body", method_name),
                expr.span.clone(),
            ));
            return;
        };
        let mut substitution = selected.owner_substitution.clone();
        for param in &function.generic_params {
            substitution.entry(param.id).or_insert_with(|| {
                self.engine
                    .borrow_mut()
                    .fresh_type_var_at_kind(expr.span.clone(), param.kind.clone())
            });
        }
        let mut lambda_params = Vec::new();
        for param in &selected.substituted_params {
            let mut param = param.clone();
            param.local_id = self.fresh_local_id();
            param.ty = param.ty.substitute_generics(&substitution);
            lambda_params.push(param);
        }
        let call_args = lambda_params
            .iter()
            .map(|param| HirExpr {
                ty: param.ty.clone(),
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: param.name.clone(),
                    target: HirVarTarget::Local(param.local_id),
                }),
                span: expr.span.clone(),
            })
            .collect::<Vec<_>>();
        let call_ret = self.resolved_type(&selected.return_type.substitute_generics(&substitution));
        self.record_selected_bounds(&selected, &substitution, "deferred method value");
        let target = selected.target_with_substitution(&substitution, |ty| self.resolved_type(ty));
        let adjusted_receiver =
            self.apply_receiver_adjustment(selected.receiver.clone(), selected.receiver_adjustment);
        let call = HirExpr {
            ty: call_ret.clone(),
            kind: HirExprKind::MethodCall(
                Box::new(adjusted_receiver),
                method_name,
                call_args,
                function.self_receiver,
                Some(target),
            ),
            span: expr.span.clone(),
        };
        let body = HirBlock {
            ty: call_ret.clone(),
            stmts: vec![HirStmt::Expr(call)],
        };
        let mut captures = Vec::new();
        self.collect_method_value_captures(
            &receiver,
            function.self_receiver,
            &mut captures,
            &mut HashSet::new(),
        );
        let params = lambda_params.iter().map(|param| param.ty.clone()).collect();
        expr.ty = self.lambda_function_type(
            params,
            call_ret.clone(),
            FunctionSafety::from_is_unsafe(function.is_unsafe),
            &captures,
        );
        expr.kind = HirExprKind::Lambda {
            params: lambda_params,
            body,
            captures,
        };
    }

    fn materialize_field_access(&mut self, expr: &mut HirExpr, errors: &mut Vec<ResolveError>) {
        let HirExprKind::FieldAccess(receiver, field_name, location) = &mut expr.kind else {
            return;
        };
        self.materialize_expr(receiver, errors);
        receiver.ty = self.engine.borrow().resolve(&receiver.ty);
        if let Type::Reference { inner, .. } = &receiver.ty {
            let dereferenced = HirExpr {
                ty: inner.as_ref().clone(),
                kind: HirExprKind::Deref(Box::new(receiver.as_ref().clone())),
                span: receiver.span.clone(),
            };
            **receiver = dereferenced;
        }
        let receiver_ty = &receiver.ty;
        let Type::Struct { id, args } = receiver_ty else {
            if !contains_inference_type(receiver_ty) {
                errors.push(ResolveError::with_span(
                    format!(
                        "Unknown field '{}' on type '{}'",
                        field_name,
                        self.display_type(receiver_ty)
                    ),
                    expr.span.clone(),
                ));
            }
            return;
        };
        let Some(structure) = self.structs.get(id) else {
            return;
        };
        let Some(field) = structure
            .fields
            .iter()
            .find(|field| match location.as_ref() {
                Some(location) => location.owner == *id && location.field_id == field.id,
                None => field.name == *field_name,
            })
        else {
            errors.push(ResolveError::with_span(
                format!(
                    "Unknown field '{}' on struct '{}'",
                    field_name, structure.name
                ),
                expr.span.clone(),
            ));
            return;
        };
        if !field.public && location.is_none() {
            if !self.strict {
                return;
            }
            errors.push(ResolveError::with_span(
                format!(
                    "field '{}' of struct '{}' is private",
                    field_name, structure.name
                ),
                expr.span.clone(),
            ));
            return;
        }
        *location = Some(HirFieldLocation {
            owner: *id,
            field_id: field.id,
            name: field.name.clone(),
        });
        let substitution = structure
            .generic_params
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                args.get(index).cloned().map(|ty| {
                    (
                        crate::types::GenericParamId {
                            owner: *id,
                            index: index as u32,
                        },
                        ty,
                    )
                })
            })
            .collect();
        expr.ty = field.ty.substitute_generics(&substitution);
    }
}

fn contains_recovery_type(ty: &Type) -> bool {
    let mut contains_recovery = false;
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        contains_recovery |= matches!(nested, Type::TypeVar(_) | Type::Error);
    });
    contains_recovery
}

fn contains_inference_type(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested: &Type| {
        matches!(nested, Type::TypeVar(_) | Type::Generic(_) | Type::Error)
    })
}

fn try_carrier_has_unknown_head(ty: &Type) -> bool {
    matches!(
        ty,
        Type::TypeVar(_)
            | Type::Generic(_)
            | Type::Projection { .. }
            | Type::Apply { .. }
            | Type::Constructor { .. }
            | Type::Lambda { .. }
            | Type::BoundVar { .. }
            | Type::Error
    )
}

fn try_type_head_is_unresolved(ty: &Type) -> bool {
    matches!(
        ty,
        Type::TypeVar(_)
            | Type::Generic(_)
            | Type::Projection { .. }
            | Type::Apply { .. }
            | Type::Constructor { .. }
            | Type::Lambda { .. }
            | Type::BoundVar { .. }
            | Type::Error
    )
}

#[cfg(test)]
mod worklist_tests {
    use super::*;
    use crate::collect::resolver::ResolverTables;
    use crate::hir::HirLanguageItems;
    use crate::ids::{CrateId, IdGen, LocalDefId};

    fn function(id: DefId, var: crate::ids::TypeVarId) -> HirFunction {
        HirFunction {
            id,
            name: format!("f{}", id.local.0),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![crate::hir::HirParam {
                name: "value".to_string(),
                local_id: HirLocalId(0),
                ty: Type::TypeVar(var),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::TypeVar(var),
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::TypeVar(var),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn hir(functions: HashMap<DefId, HirFunction>) -> PartialHir {
        PartialHir {
            functions,
            structs: HashMap::new(),
            enums: HashMap::new(),
            traits: HashMap::new(),
            impls: HashMap::new(),
            externs: HashMap::new(),
            type_aliases: HashMap::new(),
            engine: crate::infer::InferenceEngine::new(),
            function_type_vars: HashMap::new(),
            import_aliases: HashMap::new(),
            loaded_module_paths: Vec::new(),
            constraint_store: ConstraintStore::new(),
            resolver: ResolverTables::default(),
            current_def_ids: std::collections::BTreeSet::new(),
            root_crate_id: CrateId(0),
            local_def_ids: IdGen::new(),
            language_items: HirLanguageItems::default(),
            imported_effective_trait_methods: HashMap::new(),
            inference_sccs: HashMap::new(),
            inference_scc_order: Vec::new(),
            source_map: Default::default(),
        }
    }

    #[test]
    fn per_call_relation_preserves_repeated_source_variables() {
        let mut engine = crate::infer::InferenceEngine::new();
        let source = engine.fresh_type_var();
        let Type::TypeVar(source_id) = source else {
            unreachable!();
        };
        let callback_instance = engine.fresh_type_var();
        let mut bindings = HashMap::new();

        propagate_instance_relation(
            &mut engine,
            &Type::TypeVar(source_id),
            &Type::I64,
            &mut bindings,
            &HashSet::new(),
        );
        propagate_instance_relation(
            &mut engine,
            &Type::Reference {
                mutable: true,
                inner: Box::new(Type::TypeVar(source_id)),
            },
            &Type::Reference {
                mutable: true,
                inner: Box::new(callback_instance.clone()),
            },
            &mut bindings,
            &HashSet::new(),
        );

        assert_eq!(engine.resolve(&callback_instance), Type::I64);
        assert_eq!(
            engine.resolve(&Type::TypeVar(source_id)),
            Type::TypeVar(source_id)
        );
    }

    #[test]
    fn constrained_relation_propagates_instance_authority_to_source() {
        let mut engine = crate::infer::InferenceEngine::new();
        let source = engine.fresh_type_var();
        let Type::TypeVar(source_id) = source else {
            unreachable!();
        };

        propagate_instance_relation(
            &mut engine,
            &Type::TypeVar(source_id),
            &Type::I64,
            &mut HashMap::new(),
            &HashSet::from([source_id]),
        );

        assert_eq!(engine.resolve(&Type::TypeVar(source_id)), Type::I64);
    }

    #[test]
    fn authority_obligations_have_stable_owner_order_and_context() {
        let first = DefId::new(CrateId(0), LocalDefId(1));
        let second = DefId::new(CrateId(0), LocalDefId(2));
        let hir = hir(HashMap::from([
            (second, function(second, crate::ids::TypeVarId(1))),
            (first, function(first, crate::ids::TypeVarId(0))),
        ]));
        let owners = HashSet::from([ConstraintOwner::Body(second), ConstraintOwner::Body(first)]);

        let obligations = authority_obligations(&hir, &owners);

        assert_eq!(obligations.len(), 2);
        assert_eq!(obligations[0].id.raw(), 0);
        assert_eq!(obligations[0].owner, ConstraintOwner::Body(first));
        assert_eq!(obligations[0].kind, AuthorityObligationKind::Propagation);
        assert!(obligations[0].context.contains("call and type propagation"));
        assert_eq!(obligations[1].owner, ConstraintOwner::Body(second));
    }

    #[test]
    fn authority_obligation_wakes_only_for_dependent_representative() {
        let id = DefId::new(CrateId(0), LocalDefId(1));
        let hir = hir(HashMap::from([(
            id,
            function(id, crate::ids::TypeVarId(0)),
        )]));
        let owners = HashSet::from([ConstraintOwner::Body(id)]);
        let obligations = authority_obligations(&hir, &owners);

        assert!(obligations[0].depends_on_any(&HashSet::from([crate::ids::TypeVarId(0)])));
        assert!(!obligations[0].depends_on_any(&HashSet::from([crate::ids::TypeVarId(1)])));
    }

    #[test]
    fn authority_source_site_identity_targets_only_the_selected_expression() {
        let function_id = DefId::new(CrateId(0), LocalDefId(1));
        let struct_id = DefId::new(CrateId(0), LocalDefId(2));
        let receiver = || HirExpr {
            kind: HirExprKind::Var("record".to_string()),
            ty: Type::Struct {
                id: struct_id,
                args: Vec::new(),
            },
            span: crate::lexer::Span::test(),
        };
        let field = |name: &str, start| HirExpr {
            kind: HirExprKind::FieldAccess(Box::new(receiver()), name.to_string(), None),
            ty: Type::I64,
            span: crate::lexer::Span {
                file_path: "test.rk".into(),
                start,
                end: start + name.len(),
            },
        };
        let mut caller = function(function_id, crate::ids::TypeVarId(0));
        caller.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::Call(
                    Box::new(HirExpr {
                        kind: HirExprKind::Var("consume".to_string()),
                        ty: Type::Error,
                        span: crate::lexer::Span::test(),
                    }),
                    vec![field("first", 10), field("second", 20)],
                    None,
                ),
                ty: Type::Unit,
                span: crate::lexer::Span::test(),
            })],
            ty: Type::Unit,
        };
        let mut hir = hir(HashMap::from([(function_id, caller)]));
        hir.structs.insert(
            struct_id,
            crate::hir::HirStruct {
                id: struct_id,
                name: "Record".to_string(),
                generic_params: Vec::new(),
                fields: vec![
                    crate::hir::HirField {
                        id: crate::ids::FieldId(0),
                        name: "first".to_string(),
                        ty: Type::I64,
                        public: true,
                    },
                    crate::hir::HirField {
                        id: crate::ids::FieldId(1),
                        name: "second".to_string(),
                        ty: Type::I64,
                        public: true,
                    },
                ],
            },
        );
        let owners = HashSet::from([ConstraintOwner::Body(function_id)]);
        let mut obligations = authority_obligations(&hir, &owners);
        let second = obligations
            .iter_mut()
            .find(|obligation| obligation.context == "field access 'second'")
            .expect("second field obligation");

        run_authority_obligation(&mut hir, second, false, false).unwrap();

        let HirStmt::Expr(HirExpr {
            kind: HirExprKind::Call(_, args, _),
            ..
        }) = &hir.functions[&function_id].body.stmts[0]
        else {
            panic!("expected call expression");
        };
        let locations = args
            .iter()
            .map(|arg| match &arg.kind {
                HirExprKind::FieldAccess(_, _, location) => location.as_ref(),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(locations[0].is_none());
        assert_eq!(
            locations[1].map(|location| location.field_id),
            Some(crate::ids::FieldId(1))
        );
    }
}
