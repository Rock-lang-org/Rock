use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use crate::hir::{
    HirBlock, HirCallTarget, HirClosureCapture, HirClosureCaptureKind, HirExpr, HirExprKind,
    HirFieldLocation, HirFunction, HirGenericBounds, HirPattern, HirStaticMethodTarget, HirStmt,
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
    Dereference,
    Assignment,
    TryBranch,
    FromResidual,
    ObjectCoercion,
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
            for (kind, site, context, span, dependencies, _) in sites {
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
        | AuthorityObligationKind::Dereference
        | AuthorityObligationKind::Assignment
        | AuthorityObligationKind::TryBranch
        | AuthorityObligationKind::FromResidual => materialize_authority_site(
            hir,
            obligation.owner,
            obligation.site.clone().expect("selection obligation site"),
            strict,
            try_strict,
        ),
        AuthorityObligationKind::ObjectCoercion => materialize_authority_site(
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
        if let Some((_, _, _, _, dependencies, _)) = sites
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

/// A source handle hidden by a directional coercion cannot seed a structural
/// dereference until that coercion commits at global non-strict quiescence.
pub(super) fn is_dereference_input_coercion(
    hir: &PartialHir,
    obligation: &AuthorityObligation,
) -> bool {
    if obligation.kind != AuthorityObligationKind::ObjectCoercion {
        return false;
    }
    let Some(function) = function_for_owner(hir, obligation.owner) else {
        return false;
    };
    let mut sites = Vec::new();
    collect_authority_sites(&function.body, &hir.engine, &mut sites);
    sites.into_iter().any(|(_, site, _, _, _, expression)| {
        if obligation.site.as_ref() != Some(&site) {
            return false;
        }
        let HirExprKind::ObjectCoercion(source, coercion) = &expression.kind else {
            return false;
        };
        let target = hir.engine.resolve(&coercion.target);
        let source = hir.engine.resolve(&source.ty);
        if !matches!(target, Type::TypeVar(_))
            || !matches!(source, Type::Reference { .. })
            || contains_recovery_type(&source)
        {
            return false;
        }
        hir.constraint_store.iter().any(|(_, constraint)| {
            matches!(constraint, Constraint::Dereference { input, .. }
                if hir.engine.resolve(input) == target)
        })
    })
}

type AuthoritySite<'a> = (
    AuthorityObligationKind,
    AuthoritySiteId,
    String,
    crate::lexer::Span,
    HashSet<crate::ids::TypeVarId>,
    &'a HirExpr,
);

fn collect_authority_sites<'a>(
    root: &'a HirBlock,
    engine: &crate::infer::InferenceEngine,
    output: &mut Vec<AuthoritySite<'a>>,
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

    fn block<'a>(
        body: &'a HirBlock,
        engine: &crate::infer::InferenceEngine,
        output: &mut Vec<AuthoritySite<'a>>,
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

    fn expr<'a>(
        node: &'a HirExpr,
        engine: &crate::infer::InferenceEngine,
        output: &mut Vec<AuthoritySite<'a>>,
        suppress_field_slot: bool,
    ) {
        match &node.kind {
            HirExprKind::PackagePack { payload, .. } => expr(payload, engine, output, false),
            HirExprKind::Open { source, body, .. }
            | HirExprKind::PackageOpen { source, body, .. } => {
                expr(source, engine, output, false);
                block(body, engine, output);
            }
            HirExprKind::OwnedObjectCall { owner, args, .. } => {
                // Identity/protocol authority is complete. Type dependencies
                // remain part of propagation, not unresolved selection slots.
                expr(owner, engine, output, false);
                for arg in args {
                    expr(arg, engine, output, false);
                }
            }
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
                        node,
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
                            node,
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
                        node,
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
                            node,
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
                        node,
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
                        node,
                    ));
                }
                expr(operand, engine, output, false);
            }
            HirExprKind::Deref(inner) => {
                if !matches!(
                    engine.resolve(&inner.ty),
                    Type::Pointer(_) | Type::Reference { .. }
                ) || contains_recovery_type(&engine.resolve(&node.ty))
                {
                    let kind = AuthorityObligationKind::Dereference;
                    output.push((
                        kind,
                        AuthoritySiteId::new(kind, &node.span),
                        "dereference input kind and result".into(),
                        node.span.clone(),
                        dependencies([inner.ty.clone(), node.ty.clone()], engine),
                        node,
                    ));
                }
                expr(inner, engine, output, false);
            }
            HirExprKind::Ref(_, inner)
            | HirExprKind::Cast(inner, _)
            | HirExprKind::TupleIndex(inner, _)
            | HirExprKind::UnaryOp(_, inner)
            | HirExprKind::ArrayRepeat(inner, _) => expr(inner, engine, output, false),
            HirExprKind::ObjectCoercion(inner, coercion) => {
                if coercion.evidence.is_none() {
                    let kind = AuthorityObligationKind::ObjectCoercion;
                    let mut types = vec![inner.ty.clone(), coercion.target.clone()];
                    let mut nested = inner.as_ref();
                    while let HirExprKind::ObjectCoercion(child, coercion) = &nested.kind {
                        types.push(child.ty.clone());
                        types.push(coercion.target.clone());
                        nested = child;
                    }
                    output.push((
                        kind,
                        AuthoritySiteId::new(kind, &node.span),
                        "directional object coercion".into(),
                        node.span.clone(),
                        dependencies(types, engine),
                        node,
                    ));
                }
                expr(inner, engine, output, false);
            }
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
            HirExprKind::Assign(left, right) => {
                if crate::lower::place::assignment_access(left, &|ty| engine.resolve(ty))
                    != crate::lower::place::AssignmentAccess::Writable
                {
                    let kind = AuthorityObligationKind::Assignment;
                    let mut types = vec![left.ty.clone()];
                    let mut place = left.as_ref();
                    while let HirExprKind::FieldAccess(base, _, _)
                    | HirExprKind::TupleIndex(base, _)
                    | HirExprKind::Deref(base) = &place.kind
                    {
                        types.push(base.ty.clone());
                        place = base;
                    }
                    output.push((
                        kind,
                        AuthoritySiteId::new(kind, &left.span),
                        "assignment place access".into(),
                        left.span.clone(),
                        dependencies(types, engine),
                        node,
                    ));
                }
                expr(left, engine, output, false);
                expr(right, engine, output, false);
            }
            HirExprKind::BinOp(_, left, right) => {
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
    let ambiguous = materialize_pending(
        hir,
        strict,
        try_strict,
        Some(&owners),
        Some(site.clone()),
        None,
    )?;
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
    pending_coercions: usize,
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
    let message = if authority_counts(hir, owners).pending_coercions > 0 {
        "cannot resolve object coercion because its source or destination remains unresolved"
            .to_string()
    } else if authority_counts(hir, owners).pending_residuals > 0 {
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
            HirExprKind::PackagePack { payload, .. } => expr(payload, engine),
            HirExprKind::Open { source, body, .. }
            | HirExprKind::PackageOpen { source, body, .. } => {
                expr(source, engine).or_else(|| block(body, engine))
            }
            HirExprKind::OwnedObjectCall { owner, args, .. } => {
                expr(owner, engine).or_else(|| args.iter().find_map(|arg| expr(arg, engine)))
            }
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
            | HirExprKind::ObjectCoercion(operand, _)
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
    // Callee-body protocol authority must precede backwards caller seeding.
    // Prepare all declarations: this owner may call a producer in another SCC.
    let mut declared_methods = HashSet::new();
    materialize_pending(hir, false, false, None, None, Some(&mut declared_methods))?;
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
        let shared = constrained_signature_vars(hir, function, &declared_methods);
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
            let previous_owner = hir
                .constraint_store
                .replace_owner(ConstraintOwner::Body(*id));
            propagate_block(hir, &mut body, &constrained_vars, &mut errors);
            hir.constraint_store.replace_owner(previous_owner);
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
                let previous_owner = hir
                    .constraint_store
                    .replace_owner(ConstraintOwner::Body(method_id));
                propagate_block(hir, &mut body, &constrained_vars, &mut errors);
                hir.constraint_store.replace_owner(previous_owner);
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
        pending: after_counts.pending_coercions > 0
            || after_counts.pending_methods > 0
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
            HirExprKind::PackagePack { payload, .. } => expr(payload, counts),
            HirExprKind::Open { source, body, .. }
            | HirExprKind::PackageOpen { source, body, .. } => {
                expr(source, counts);
                block(body, counts);
            }
            HirExprKind::OwnedObjectCall { owner, args, .. } => {
                counts.methods += 1;
                expr(owner, counts);
                for arg in args {
                    expr(arg, counts);
                }
            }
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
            HirExprKind::ObjectCoercion(inner, coercion) => {
                counts.pending_coercions += usize::from(coercion.evidence.is_none());
                expr(inner, counts);
            }
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
            HirExprKind::PackagePack { payload, proof } => {
                proof.visit_types(&mut |ty| collect_type_vars(&engine.resolve(ty), output));
                expr(payload, engine, output);
            }
            HirExprKind::PackageOpen {
                source,
                binding,
                body,
            } => {
                binding.visit_types(&mut |ty| collect_type_vars(&engine.resolve(ty), output));
                expr(source, engine, output);
                collect_block_type_vars(body, engine, output);
            }
            HirExprKind::Open {
                source,
                binding,
                body,
            } => {
                binding.visit_types(&mut |ty| collect_type_vars(&engine.resolve(ty), output));
                expr(source, engine, output);
                collect_block_type_vars(body, engine, output);
            }
            HirExprKind::OwnedObjectCall { owner, args, call } => {
                call.visit_types(&mut |ty| collect_type_vars(&engine.resolve(ty), output));
                expr(owner, engine, output);
                for arg in args {
                    expr(arg, engine, output);
                }
            }
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
            | HirExprKind::ObjectCoercion(inner, _)
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
    declared_methods: &HashSet<AuthoritySiteId>,
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
                if matches!(hir.engine.resolve(ty), Type::TypeVar(_)) {
                    // Trait requirements are polymorphic predicates, not evidence
                    // fixing the declaration to one caller's implementation.
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
            Constraint::Dereference { input, output, .. } => {
                // A structural input-kind requirement needs actual caller
                // evidence; the distinct result never chooses pointer vs ref.
                collect_type_vars(&hir.engine.resolve(input), &mut obligation_vars);
                collect_type_vars(&hir.engine.resolve(output), &mut obligation_vars);
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
    collect_unresolved_authority_vars(
        &function.body,
        &hir.engine,
        declared_methods,
        &mut authority_vars,
    );
    shared.extend(signature_vars.intersection(&authority_vars).copied());
    let mut sites = Vec::new();
    collect_authority_sites(&function.body, &hir.engine, &mut sites);
    for (kind, _, _, _, dependencies, _) in sites {
        if kind == AuthorityObligationKind::Field {
            // An unresolved structural field needs caller evidence in the
            // declaration's signature slots, not just its fresh call instance.
            shared.extend(signature_vars.intersection(&dependencies).copied());
        }
    }
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
        HirExprKind::PackagePack { payload, .. } => {
            propagate_expr(hir, payload, constrained_vars, errors)
        }
        HirExprKind::Open { source, body, .. } | HirExprKind::PackageOpen { source, body, .. } => {
            propagate_expr(hir, source, constrained_vars, errors);
            propagate_block(hir, body, constrained_vars, errors);
        }
        HirExprKind::OwnedObjectCall { owner, args, .. } => {
            propagate_expr(hir, owner, constrained_vars, errors);
            for arg in args {
                propagate_expr(hir, arg, constrained_vars, errors);
            }
        }
        HirExprKind::Call(callee, args, target) => {
            // Establish the call-site scheme before descending into deferred
            // arguments so their method authority can use the parameter type.
            propagate_call_instance(
                hir,
                &expr.ty,
                callee,
                args,
                target.as_ref(),
                constrained_vars,
                errors,
            );
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
            propagate_call_instance(
                hir,
                &expr.ty,
                callee,
                args,
                target.as_ref(),
                constrained_vars,
                errors,
            );
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
        HirExprKind::Deref(receiver) => {
            propagate_expr(hir, receiver, constrained_vars, errors);
            if let Type::Pointer(inner) | Type::Reference { inner, .. } =
                hir.engine.resolve(&receiver.ty)
            {
                let _ = hir.engine.unify(&expr.ty, &inner);
            }
        }
        HirExprKind::FieldAccess(receiver, _, _)
        | HirExprKind::Ref(_, receiver)
        | HirExprKind::Cast(receiver, _)
        | HirExprKind::ObjectCoercion(receiver, _) => {
            propagate_expr(hir, receiver, constrained_vars, errors)
        }
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
    target: Option<&crate::hir::HirCallTarget>,
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
                let partial_static_scheme = target.filter(|target| {
                    matches!(target, crate::hir::HirCallTarget::StaticMethod(target)
                        if target.method.method_id() == Some(function_id))
                        && function
                            .params
                            .iter()
                            .map(|param| hir.engine.resolve(&param.ty))
                            .chain(std::iter::once(source_ret.clone()))
                            .any(|ty| {
                                crate::type_services::visit::type_any(&ty, |nested| {
                                    matches!(nested, Type::TypeVar(_))
                                })
                            })
                });
                if let Some(crate::hir::HirCallTarget::StaticMethod(target)) = partial_static_scheme
                {
                    // A partially inferred static scheme already has rigid impl
                    // parameters. Instantiate those only through this selected
                    // call's canonical bindings, then relate its remaining slots
                    // with one shared parameter/result map (not caller equality
                    // to the declaration's inference variables).
                    let generic_bindings = target
                        .method
                        .owner_substitution
                        .iter()
                        .chain(&target.method.method_substitution)
                        .map(|binding| (binding.param, binding.ty.clone()))
                        .collect::<HashMap<_, _>>();
                    let mut instance_bindings = HashMap::new();
                    for (source, instance) in function.params.iter().zip(&params) {
                        let source = hir
                            .engine
                            .resolve(&source.ty)
                            .substitute_generics(&generic_bindings);
                        propagate_instance_relation(
                            &mut hir.engine,
                            &source,
                            instance,
                            &mut instance_bindings,
                            &HashSet::new(),
                        );
                    }
                    let source = source_ret.substitute_generics(&generic_bindings);
                    propagate_instance_relation(
                        &mut hir.engine,
                        &source,
                        ret.as_ref(),
                        &mut instance_bindings,
                        &HashSet::new(),
                    );
                    propagate_instance_bounds(
                        hir,
                        &mut instance_bindings,
                        &generic_bindings,
                        &callee.span,
                        errors,
                    );
                } else {
                    propagate_generic_scheme_type(&mut hir.engine, &source_ret, ret.as_ref());
                }
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
            propagate_instance_bounds(
                hir,
                &mut instance_bindings,
                &HashMap::new(),
                &callee.span,
                errors,
            );
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

fn propagate_instance_bounds(
    hir: &mut PartialHir,
    bindings: &mut HashMap<crate::ids::TypeVarId, Type>,
    generic_bindings: &HashMap<crate::types::GenericParamId, Type>,
    span: &crate::lexer::Span,
    errors: &mut Vec<ResolveError>,
) {
    // Replay predicates discovered after lowering instantiated the signature.
    // Use the very same per-call relation as its parameter/result types.
    let mut variables = bindings.keys().copied().collect::<Vec<_>>();
    variables.sort();
    let mut index = 0;
    while index < variables.len() {
        let variable = variables[index];
        index += 1;
        let subject = hir.engine.resolve(&bindings[&variable]);
        for bound in hir.engine.get_bounds(variable) {
            let arguments = bound
                .type_args
                .iter()
                .map(|argument| hir.engine.resolve(argument))
                .collect::<Vec<_>>();
            let mut argument_vars = HashSet::new();
            for argument in &arguments {
                collect_type_vars(argument, &mut argument_vars);
            }
            if argument_vars
                .iter()
                .any(|variable| !bindings.contains_key(variable))
            {
                // Bound-only variables belong to the same scheme instance. On a
                // revisit, recover them from this site's existing predicate, not
                // from another caller or a fresh unrelated owner substitution.
                let candidates = hir
                    .constraint_store
                    .iter()
                    .filter_map(|(_, constraint)| {
                        let Constraint::Trait {
                            ty,
                            bound: existing,
                            span: existing_span,
                            ..
                        } = constraint
                        else {
                            return None;
                        };
                        (existing_span == span
                            && hir.engine.resolve(ty) == subject
                            && existing.trait_id == bound.trait_id
                            && existing.type_args.len() == arguments.len())
                        .then_some(
                            existing
                                .type_args
                                .iter()
                                .map(|argument| hir.engine.resolve(argument))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>();
                let mut relations = Vec::new();
                let mut seen = HashSet::new();
                for candidate in candidates {
                    if !seen.insert(candidate.clone()) {
                        continue;
                    }
                    let mut probe = hir.engine.clone_for_probe();
                    let mut relation = bindings.clone();
                    for (source, instance) in arguments.iter().zip(&candidate) {
                        propagate_instance_relation(
                            &mut probe,
                            source,
                            instance,
                            &mut relation,
                            &HashSet::new(),
                        );
                    }
                    if arguments.iter().zip(&candidate).all(|(source, instance)| {
                        probe.unify(&source.substitute(&relation), instance).is_ok()
                    }) {
                        relations.push((probe, relation));
                    }
                }
                if relations.len() > 1 {
                    errors.push(ResolveError::with_span_code(
                        "inferred bound arguments have multiple call-site relations".into(),
                        span.clone(),
                        crate::diagnostic::DiagnosticCode::Type,
                    ));
                    return;
                }
                if let Some((probe, relation)) = relations.pop() {
                    hir.engine.commit_probe(probe);
                    bindings.extend(relation);
                }
                let mut missing = argument_vars
                    .iter()
                    .copied()
                    .filter(|variable| !bindings.contains_key(variable))
                    .collect::<Vec<_>>();
                missing.sort();
                for variable in missing {
                    let kind = hir.engine.kind_of_type_var(variable);
                    bindings.insert(
                        variable,
                        hir.engine.fresh_type_var_at_kind(span.clone(), kind),
                    );
                }
            }
            let mut argument_vars = argument_vars.into_iter().collect::<Vec<_>>();
            argument_vars.sort();
            for variable in argument_vars {
                if !variables.contains(&variable) {
                    variables.push(variable);
                }
            }
            let bound = TraitBound {
                trait_id: bound.trait_id,
                type_args: arguments
                    .iter()
                    .map(|argument| {
                        argument
                            .substitute(bindings)
                            .substitute_generics(generic_bindings)
                    })
                    .collect(),
            };
            if let Type::TypeVar(instance) = subject {
                if !hir.engine.get_bounds(instance).contains(&bound) {
                    hir.engine.add_bound(instance, bound.clone());
                }
            }
            let recorded = hir.constraint_store.iter().any(|(_, constraint)| {
                matches!(constraint, Constraint::Trait { ty, bound: existing, span: existing_span, .. }
                    if existing_span == span && hir.engine.resolve(ty) == subject
                        && existing.trait_id == bound.trait_id
                        && existing.type_args.iter().map(|argument| hir.engine.resolve(argument)).collect::<Vec<_>>()
                            == bound.type_args.iter().map(|argument| hir.engine.resolve(argument)).collect::<Vec<_>>())
            });
            if !recorded {
                hir.constraint_store.add_trait(
                    subject.clone(),
                    bound,
                    span.clone(),
                    "inferred function instance bound",
                );
            }
        }
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
    declared_methods: &HashSet<AuthoritySiteId>,
    output: &mut HashSet<crate::ids::TypeVarId>,
) {
    fn expr(
        node: &HirExpr,
        engine: &crate::infer::InferenceEngine,
        declared_methods: &HashSet<AuthoritySiteId>,
        output: &mut HashSet<crate::ids::TypeVarId>,
    ) {
        match &node.kind {
            HirExprKind::PackagePack { payload, .. } => {
                expr(payload, engine, declared_methods, output)
            }
            HirExprKind::Open { source, body, .. }
            | HirExprKind::PackageOpen { source, body, .. } => {
                expr(source, engine, declared_methods, output);
                collect_unresolved_authority_vars(body, engine, declared_methods, output);
            }
            HirExprKind::OwnedObjectCall { owner, args, .. } => {
                expr(owner, engine, declared_methods, output);
                for arg in args {
                    expr(arg, engine, declared_methods, output);
                }
            }
            HirExprKind::Call(callee, args, target) => {
                if target.is_none()
                    && !declared_methods.contains(&AuthoritySiteId::new(
                        AuthorityObligationKind::DeferredCall,
                        &node.span,
                    ))
                {
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
                expr(callee, engine, declared_methods, output);
                for arg in args {
                    expr(arg, engine, declared_methods, output);
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
                expr(operand, engine, declared_methods, output);
            }
            HirExprKind::MethodCall(receiver, _, args, _, _) => {
                expr(receiver, engine, declared_methods, output);
                for arg in args {
                    expr(arg, engine, declared_methods, output);
                }
            }
            HirExprKind::FieldAccess(inner, _, _)
            | HirExprKind::Deref(inner)
            | HirExprKind::Ref(_, inner)
            | HirExprKind::Cast(inner, _)
            | HirExprKind::ObjectCoercion(inner, _)
            | HirExprKind::TupleIndex(inner, _)
            | HirExprKind::UnaryOp(_, inner)
            | HirExprKind::ArrayRepeat(inner, _) => expr(inner, engine, declared_methods, output),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                expr(condition, engine, declared_methods, output);
                collect_unresolved_authority_vars(then_branch, engine, declared_methods, output);
                if let Some(else_branch) = else_branch {
                    collect_unresolved_authority_vars(
                        else_branch,
                        engine,
                        declared_methods,
                        output,
                    );
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                expr(scrutinee, engine, declared_methods, output);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        expr(guard, engine, declared_methods, output);
                    }
                    collect_unresolved_authority_vars(&arm.body, engine, declared_methods, output);
                }
            }
            HirExprKind::While { condition, body } => {
                expr(condition, engine, declared_methods, output);
                collect_unresolved_authority_vars(body, engine, declared_methods, output);
            }
            HirExprKind::For { iter, body, .. } => {
                expr(iter, engine, declared_methods, output);
                collect_unresolved_authority_vars(body, engine, declared_methods, output);
            }
            HirExprKind::Block(body)
            | HirExprKind::Loop(body)
            | HirExprKind::UnsafeBlock(body)
            | HirExprKind::Lambda { body, .. } => {
                collect_unresolved_authority_vars(body, engine, declared_methods, output)
            }
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                expr(left, engine, declared_methods, output);
                expr(right, engine, declared_methods, output);
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    expr(arg, engine, declared_methods, output);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    expr(&field.value, engine, declared_methods, output);
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
            | HirStmt::Break(Some(value)) => expr(value, engine, declared_methods, output),
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
    mut declared_methods: Option<&mut HashSet<AuthoritySiteId>>,
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
        function_owner: ConstraintOwner::Global,
        constraint_store: &mut hir.constraint_store,
        effective_trait_methods: &hir.imported_effective_trait_methods,
        resolver: &hir.resolver,
        structs: &struct_ids,
        enums: &hir.enums,
        engine: RefCell::new(&mut hir.engine),
        try_protocol: hir.language_items.try_protocol.clone(),
        unsafe_context: Cell::new(false),
        mutable_locals: HashSet::new(),
        local_bindings: HashMap::new(),
        pending_coercion_dependencies: HashSet::new(),
        opened_witnesses: Vec::new(),
        opened_packages: Vec::new(),
        loop_targets: Vec::new(),
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
            if let Some(prepared) = declared_methods.as_deref_mut() {
                context.prepare_declared_methods(function, prepared, &mut errors);
            } else {
                context.materialize_function(function, &mut errors);
            }
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
                    if let Some(prepared) = declared_methods.as_deref_mut() {
                        context.prepare_declared_methods(function, prepared, &mut errors);
                    } else {
                        context.materialize_function(function, &mut errors);
                    }
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
                    if let Some(prepared) = declared_methods.as_deref_mut() {
                        context.prepare_declared_methods(function, prepared, &mut errors);
                    } else {
                        context.materialize_function(function, &mut errors);
                    }
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
    materialize_pending(hir, true, true, None, None, None).map(|_| ())
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
    function_owner: ConstraintOwner,
    constraint_store: &'a mut ConstraintStore,
    effective_trait_methods: &'a HashMap<(DefId, DefId), DefId>,
    resolver: &'a crate::collect::resolver::ResolverTables,
    structs: &'a HashMap<DefId, crate::hir::HirStruct>,
    enums: &'a HashMap<DefId, crate::hir::HirEnum>,
    engine: RefCell<&'a mut crate::infer::InferenceEngine>,
    try_protocol: Option<TryLanguageItems<DefId>>,
    unsafe_context: Cell<bool>,
    mutable_locals: HashSet<HirLocalId>,
    local_bindings: HashMap<HirLocalId, (String, Type, bool)>,
    pending_coercion_dependencies: HashSet<crate::ids::TypeVarId>,
    opened_witnesses: Vec<crate::hir::HirOpenBinding>,
    opened_packages: Vec<crate::hir::HirPackageOpenBinding>,
    loop_targets: Vec<(Vec<crate::types::WitnessId>, Type)>,
    next_local_id: u32,
    ambiguous: bool,
    selected_site: Option<AuthoritySiteId>,
    strict: bool,
    try_strict: bool,
}

fn add_declared_sized_bounds(
    bounds: &mut HirGenericBounds,
    parameters: &[crate::types::GenericParamDecl],
    sized_trait_id: Option<DefId>,
) {
    let Some(trait_id) = sized_trait_id else {
        return;
    };
    for parameter in parameters {
        if parameter.kind != crate::type_services::kind::Kind::Type
            || parameter.maybe_unsized
            || bounds.relaxed_sized.contains(&parameter.id)
        {
            continue;
        }
        let bound = TraitBound {
            trait_id,
            type_args: Vec::new(),
        };
        let assumptions = bounds.entry(parameter.id).or_default();
        if !assumptions.contains(&bound) {
            assumptions.push(bound);
        }
    }
}

impl MethodAuthorityContext<'_> {
    fn opened_object_schema(
        &self,
        witness: crate::types::WitnessId,
    ) -> Option<crate::types::ObjectType> {
        self.opened_packages
            .iter()
            .rev()
            .find(|binding| binding.witness == witness)
            .and_then(|binding| binding.object_schema().ok())
            .or_else(|| {
                self.opened_witnesses
                    .iter()
                    .rev()
                    .find(|binding| binding.witness == witness)
                    .map(|binding| binding.object.clone())
            })
    }
    fn materialize_owned_call(
        &mut self,
        receiver: &HirExpr,
        name: &str,
        args: &[HirExpr],
        result: &Type,
        span: &crate::lexer::Span,
        errors: &mut Vec<ResolveError>,
    ) -> Result<Option<HirExpr>, ()> {
        let owner = HirExpr {
            ty: self.resolved_type(&receiver.ty),
            ..receiver.clone()
        };
        let (selected, mut call) = match crate::hir::owned_objects::select(
            &owner,
            name,
            self.traits,
            &self.language_items,
            &self.service(),
            self.opened_witnesses
                .iter()
                .rev()
                .find(|binding| binding.matches_owner(&owner.ty)),
        ) {
            Ok(Some(selected)) => selected,
            Ok(None) => return Ok(None),
            Err(message) => {
                errors.push(ResolveError::with_span(message, span.clone()));
                return Err(());
            }
        };
        let Some(function) = &selected.function else {
            errors.push(ResolveError::with_span(
                "consuming member has no declaration".into(),
                span.clone(),
            ));
            return Err(());
        };
        if function.is_unsafe && !self.unsafe_context.get() {
            errors.push(ResolveError::with_span(
                "unsafe consuming member requires an unsafe block".into(),
                span.clone(),
            ));
            return Err(());
        }
        if args.len() != selected.substituted_params.len() {
            errors.push(ResolveError::with_span(
                "consuming object call requires all declared arguments".into(),
                span.clone(),
            ));
            return Err(());
        }
        let mut substitution = selected.owner_substitution.clone();
        for parameter in &function.generic_params {
            substitution.entry(parameter.id).or_insert_with(|| {
                self.engine
                    .borrow_mut()
                    .fresh_type_var_at_kind(span.clone(), parameter.kind.clone())
            });
        }
        let params = selected
            .substituted_params
            .iter()
            .map(|param| param.ty.substitute_generics(&substitution))
            .collect::<Vec<_>>();
        let ret = selected.return_type.substitute_generics(&substitution);
        let callee = HirExpr {
            ty: Type::function(params, ret.clone()),
            kind: HirExprKind::Unit,
            span: span.clone(),
        };
        let mut arguments = args.to_vec();
        self.materialize_callable_arguments(&callee, &mut arguments, errors);
        let result_check = self.engine.borrow_mut().unify(result, &ret);
        if let Err(error) = result_check {
            errors.push(ResolveError::with_span(
                error.render(&self.engine.borrow()),
                span.clone(),
            ));
            return Err(());
        }
        self.record_selected_bounds(&selected, &substitution, "consuming object method");
        call.method = selected.target_with_substitution(&substitution, |ty| self.resolved_type(ty));
        Ok(Some(HirExpr {
            ty: self.resolved_type(&ret),
            span: span.clone(),
            kind: HirExprKind::OwnedObjectCall {
                owner: Box::new(owner),
                args: arguments,
                call,
            },
        }))
    }

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

    fn prepare_function_context(&mut self, function: &HirFunction) {
        self.function_owner = ConstraintOwner::Body(function.id);
        self.pending_coercion_dependencies.clear();
        let mut sites = Vec::new();
        collect_authority_sites(&function.body, &self.engine.borrow(), &mut sites);
        for (kind, _, _, _, dependencies, _) in sites {
            if kind == AuthorityObligationKind::ObjectCoercion {
                self.pending_coercion_dependencies.extend(dependencies);
            }
        }
        self.bounds = function.generic_bounds.clone();
        if let Some(implementation) = self.impls.values().find(|implementation| {
            implementation
                .methods
                .values()
                .any(|method| method.id == function.id)
        }) {
            self.bounds
                .relaxed_sized
                .extend(implementation.bounds.relaxed_sized.iter().copied());
            self.bounds
                .predicates
                .extend(implementation.bounds.predicates.clone());
            for (parameter, assumptions) in &implementation.bounds {
                let existing = self.bounds.entry(*parameter).or_default();
                for assumption in assumptions {
                    if !existing.contains(assumption) {
                        existing.push(assumption.clone());
                    }
                }
            }
        }
        // Selection requires positive evidence for a contextual generic. The
        // declaration supplies default Sized, unless explicitly relaxed; do
        // not infer it from arbitrary generic occurrences in a carrier type.
        add_declared_sized_bounds(
            &mut self.bounds,
            &function.generic_params,
            self.sized_trait_id,
        );
        self.mutable_locals.clear();
        self.local_bindings.clear();
        self.loop_targets.clear();
        self.next_local_id = 0;
        // Scan the entire body before coercions or method values synthesize locals.
        self.collect_local_bindings(function);
    }

    fn prepare_declared_methods(
        &mut self,
        function: &HirFunction,
        prepared: &mut HashSet<AuthoritySiteId>,
        errors: &mut Vec<ResolveError>,
    ) {
        self.prepare_function_context(function);
        let previous_owner = self
            .constraint_store
            .replace_owner(ConstraintOwner::Body(function.id));
        let mut sites = Vec::new();
        collect_authority_sites(&function.body, &self.engine.borrow(), &mut sites);
        for (kind, site, _, _, _, expression) in sites {
            if kind != AuthorityObligationKind::DeferredCall {
                continue;
            }
            let HirExprKind::Call(callee, _, None) = &expression.kind else {
                continue;
            };
            let HirExprKind::FieldAccess(receiver, name, None) = &callee.kind else {
                continue;
            };
            let candidates = self.receiver_adjustment_candidates(receiver.as_ref().clone());
            if matches!(
                self.select_receiver_bound_method(receiver, name, &candidates, errors),
                Ok(Some(_))
            ) {
                // A checked declaration-member selection makes this a predicate,
                // not evidence to identify the producer with one concrete caller.
                prepared.insert(site);
            }
        }
        self.constraint_store.replace_owner(previous_owner);
    }

    fn materialize_function(&mut self, function: &mut HirFunction, errors: &mut Vec<ResolveError>) {
        self.prepare_function_context(function);
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
        errors: &mut Vec<ResolveError>,
    ) {
        let Some(function) = self.functions.get(&function_id).cloned() else {
            return;
        };
        let template = Type::function_with_safety(
            function
                .params
                .iter()
                .map(|parameter| parameter.ty.clone())
                .collect(),
            function.ret_type.clone(),
            FunctionSafety::from_is_unsafe(function.is_unsafe),
        );
        let mut substitutions = HashMap::new();
        if type_pattern_matches(
            &template,
            &self.resolved_type(&callee.ty),
            &mut substitutions,
        ) {
            for parameter in &function.generic_params {
                if parameter.maybe_unsized
                    || function
                        .generic_bounds
                        .relaxed_sized
                        .contains(&parameter.id)
                    || parameter.kind != crate::type_services::kind::Kind::Type
                {
                    continue;
                }
                if let Some(actual) = substitutions.get(&parameter.id) {
                    if crate::collect::sizedness::storage_is_unsized(
                        actual,
                        &self.bounds,
                        self.traits,
                        self.sized_trait_id,
                    ) {
                        errors.push(ResolveError::with_span("generic type argument requires Sized; the declaration must explicitly relax this with ?Sized".into(), callee.span.clone()));
                    }
                }
            }
        }
        if function.is_method
            || !function.generic_params.is_empty()
            || function.params.iter().any(|param| param.is_ref)
        {
            return;
        }
        for (arg, parameter) in args.iter().zip(&function.params) {
            let expected = self.resolved_type(&parameter.ty);
            let actual = self.resolved_type(&arg.ty);
            if contains_inference_type(&expected) || contains_inference_type(&actual) {
                continue;
            }
            // Validate against the canonical declaration, not a conflicting
            // cached call instance; a failed probe must not mutate inference.
            let mut probe = self.engine.borrow().clone_for_probe();
            if let Err(error) = probe.unify(&expected, &actual) {
                errors.push(ResolveError::with_span_code(
                    format!(
                        "callable argument type mismatch: {}",
                        error.render(&self.engine.borrow()),
                    ),
                    arg.span.clone(),
                    crate::diagnostic::DiagnosticCode::Type,
                ));
                return;
            }
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
                | HirStmt::Return(Some(value)) => self.materialize_expr(value, errors),
                HirStmt::Break(Some(value)) => {
                    self.materialize_expr(value, errors);
                    if self.strict {
                        let actual = self.resolved_type(&value.ty);
                        let target = self.loop_targets.last();
                        let escapes = crate::type_services::visit::type_any(
                            &actual,
                            |ty| matches!(ty, Type::Witness(witness) if target.is_none_or(|(allowed, _)| !allowed.contains(witness))),
                        );
                        if escapes {
                            errors.push(ResolveError::with_span(
                                "opened witness escapes through a break; repackage the value"
                                    .into(),
                                value.span.clone(),
                            ));
                        } else if let Some((_, expected)) = target {
                            let expected = self.resolved_type(expected);
                            if actual != Type::Never
                                && actual != expected
                                && [&actual, &expected].iter().any(|ty| {
                                    crate::type_services::visit::type_any(ty, |nested| {
                                        matches!(nested, Type::Witness(_))
                                    })
                                })
                            {
                                errors.push(ResolveError::with_span(
                                    "opened witness break result disagrees with its loop target"
                                        .into(),
                                    value.span.clone(),
                                ));
                            }
                        }
                    }
                }
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

    fn materialize_loop(
        &mut self,
        body: &mut HirBlock,
        result: &Type,
        errors: &mut Vec<ResolveError>,
    ) {
        // A break may retain only witnesses that already dominate its target.
        self.loop_targets.push((
            self.opened_witnesses
                .iter()
                .map(|binding| binding.witness)
                .chain(self.opened_packages.iter().map(|binding| binding.witness))
                .collect(),
            result.clone(),
        ));
        self.materialize_block(body, errors);
        self.loop_targets.pop();
    }

    fn record_local_binding(&mut self, local_id: HirLocalId, name: &str, ty: Type, mutable: bool) {
        self.reserve_local_id(local_id);
        if mutable {
            self.mutable_locals.insert(local_id);
        }
        self.local_bindings
            .insert(local_id, (name.to_string(), ty, mutable));
    }

    fn reserve_local_id(&mut self, local_id: HirLocalId) {
        self.next_local_id = self.next_local_id.max(local_id.0.saturating_add(1));
    }

    fn reserve_pattern_local_ids(&mut self, pattern: &HirPattern) {
        match pattern {
            HirPattern::Binding { local_id, .. } => self.reserve_local_id(*local_id),
            HirPattern::Tuple(patterns)
            | HirPattern::Enum(_, _, _, patterns)
            | HirPattern::Or(patterns) => {
                for pattern in patterns {
                    self.reserve_pattern_local_ids(pattern);
                }
            }
            HirPattern::Struct(_, _, _, fields) => {
                for field in fields {
                    self.reserve_pattern_local_ids(&field.pattern);
                }
            }
            HirPattern::Wildcard | HirPattern::Literal(_) => {}
        }
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
            HirExprKind::PackagePack { payload, .. } => self.collect_local_bindings_expr(payload),
            HirExprKind::PackageOpen {
                source,
                binding,
                body,
            } => {
                self.record_local_binding(
                    binding.value.local_id,
                    &binding.value.name,
                    binding.value.ty.clone(),
                    binding.value.mutable,
                );
                self.collect_local_bindings_expr(source);
                self.collect_local_bindings_block(body);
            }
            HirExprKind::Open {
                source,
                binding,
                body,
            } => {
                self.record_local_binding(
                    binding.value.local_id,
                    &binding.value.name,
                    binding.value.ty.clone(),
                    binding.value.mutable,
                );
                self.collect_local_bindings_expr(source);
                self.collect_local_bindings_block(body);
            }
            HirExprKind::OwnedObjectCall { owner, args, .. } => {
                self.collect_local_bindings_expr(owner);
                for arg in args {
                    self.collect_local_bindings_expr(arg);
                }
            }
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
                    self.reserve_pattern_local_ids(&arm.pattern);
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
            HirExprKind::For {
                local_id,
                iter,
                body,
                ..
            } => {
                self.reserve_local_id(*local_id);
                self.collect_local_bindings_expr(iter);
                self.collect_local_bindings_block(body);
            }
            HirExprKind::Block(body) | HirExprKind::Loop(body) | HirExprKind::UnsafeBlock(body) => {
                self.collect_local_bindings_block(body)
            }
            HirExprKind::Lambda {
                params,
                body,
                captures,
            } => {
                for capture in captures {
                    self.reserve_local_id(capture.local_id);
                }
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
            | HirExprKind::ObjectCoercion(expr, _)
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
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Local(local_id),
                ..
            }) => {
                self.reserve_local_id(*local_id);
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
        if matches!(inner.as_ref(), Type::Object(_) | Type::Witness(_)) {
            let span = expr.span.clone();
            let place = HirExpr {
                ty: inner.as_ref().clone(),
                kind: HirExprKind::Deref(Box::new(expr)),
                span: span.clone(),
            };
            return Some(HirExpr {
                ty: Type::Reference {
                    mutable: false,
                    inner,
                },
                kind: HirExprKind::Ref(false, Box::new(place)),
                span,
            });
        }
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
                if !matches!(
                    resolved,
                    Type::Str | Type::Slice(_) | Type::Object(_) | Type::Witness(_)
                ) {
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

    fn select_trait_deref_candidate(&self, expr: &HirExpr) -> Option<SelectedMethod> {
        let can_autoref_mut = self.expr_can_autoref_mut_receiver(&expr);
        let candidate = ReceiverCandidate {
            expr: HirExpr {
                ty: self.resolved_type(&expr.ty),
                ..expr.clone()
            },
            adjustment: ReceiverAdjustment::None,
            can_autoref_mut,
        };
        let ty = self.resolved_type(&expr.ty);
        let bounds = match &ty {
            Type::TypeVar(id) => self.engine.borrow().get_bounds(*id),
            Type::Generic(id) => self.bounds.get(id).cloned().unwrap_or_default(),
            _ => Vec::new(),
        };
        if !bounds.is_empty() {
            if let Ok(selected) = self.service().select_bound_method(
                std::slice::from_ref(&candidate),
                &bounds,
                "*",
                ty.clone(),
            ) {
                return Some(selected);
            }
        }
        if matches!(ty, Type::TypeVar(_) | Type::Generic(_)) {
            return None;
        }
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
        selected.pop()
    }

    fn trait_deref_candidate(&mut self, expr: HirExpr) -> Option<HirExpr> {
        let selected = self.select_trait_deref_candidate(&expr)?;
        self.trait_deref_expression(expr, selected)
    }

    fn trait_deref_expression(
        &mut self,
        expr: HirExpr,
        selected: SelectedMethod,
    ) -> Option<HirExpr> {
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

    fn materialize_dereference(&mut self, expr: &mut HirExpr, errors: &mut Vec<ResolveError>) {
        let HirExprKind::Deref(operand) = &mut expr.kind else {
            return;
        };
        self.materialize_expr(operand, errors);
        if !self.visit_authority_slot(AuthorityObligationKind::Dereference, &expr.span) {
            return;
        }
        let input = self.resolved_type(&operand.ty);
        let relations = self
            .constraint_store
            .iter()
            .filter_map(|(id, constraint)| match constraint {
                Constraint::Dereference {
                    input: source,
                    output,
                    unsafe_allowed,
                    span,
                } if self.constraint_store.owner(id) == Some(self.function_owner)
                    && *span == expr.span
                    && self.resolved_type(source) == input
                    && self.resolved_type(output) == self.resolved_type(&expr.ty) =>
                {
                    Some((id, *unsafe_allowed))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let relation = match relations.as_slice() {
            [] => None,
            [relation] => Some(*relation),
            _ => {
                errors.push(ResolveError::with_span_code(
                    "ambiguous dereference source relation".into(),
                    expr.span.clone(),
                    crate::diagnostic::DiagnosticCode::Type,
                ));
                return;
            }
        };
        let selected_output = match &input {
            Type::Pointer(_) if relation.is_some_and(|(_, allowed)| !allowed) => {
                errors.push(ResolveError::with_span_code(
                    "Dereference of raw pointer requires an unsafe block".into(),
                    expr.span.clone(),
                    crate::diagnostic::DiagnosticCode::Type,
                ));
                return;
            }
            Type::Pointer(inner) | Type::Reference { inner, .. } => inner.as_ref().clone(),
            _ => {
                let Some(selected) = self.select_trait_deref_candidate(operand) else {
                    if self.strict || !contains_recovery_type(&input) {
                        let message = if contains_recovery_type(&input) {
                            "cannot determine dereference input kind at quiescence".into()
                        } else {
                            format!(
                                "Cannot dereference non-pointer type: {}",
                                self.display_type(&input)
                            )
                        };
                        errors.push(ResolveError::with_span_code(
                            message,
                            expr.span.clone(),
                            crate::diagnostic::DiagnosticCode::Type,
                        ));
                    }
                    return;
                };
                let allowed = relation
                    .map(|(_, allowed)| allowed)
                    .unwrap_or_else(|| self.unsafe_context.get());
                if selected
                    .function
                    .as_ref()
                    .is_some_and(|function| function.is_unsafe)
                    && !allowed
                {
                    errors.push(ResolveError::with_span_code(
                        "Call to unsafe dereference method requires an unsafe block".into(),
                        expr.span.clone(),
                        crate::diagnostic::DiagnosticCode::Type,
                    ));
                    return;
                }
                let Some(mut lowered) =
                    self.trait_deref_expression(operand.as_ref().clone(), selected)
                else {
                    errors.push(ResolveError::with_span_code(
                        "dereference method must return a reference".into(),
                        expr.span.clone(),
                        crate::diagnostic::DiagnosticCode::Type,
                    ));
                    return;
                };
                let result = self.engine.borrow_mut().unify(&expr.ty, &lowered.ty);
                if let Err(error) = result {
                    errors.push(ResolveError::with_span_code(
                        error.render(&self.engine.borrow()),
                        expr.span.clone(),
                        crate::diagnostic::DiagnosticCode::Type,
                    ));
                    return;
                }
                lowered.span = expr.span.clone();
                *expr = lowered;
                if let Some((id, _)) = relation {
                    self.constraint_store.set_state(id, ObligationState::Solved);
                }
                return;
            }
        };
        let result = self.engine.borrow_mut().unify(&expr.ty, &selected_output);
        if let Err(error) = result {
            errors.push(ResolveError::with_span_code(
                error.render(&self.engine.borrow()),
                expr.span.clone(),
                crate::diagnostic::DiagnosticCode::Type,
            ));
        } else {
            expr.ty = self.resolved_type(&expr.ty);
        }
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

    fn select_receiver_bound_method(
        &mut self,
        receiver: &HirExpr,
        method_name: &str,
        receiver_candidates: &[ReceiverCandidate],
        errors: &mut Vec<ResolveError>,
    ) -> Result<Option<SelectedMethod>, ()> {
        let Type::TypeVar(id) = self.resolved_type(&receiver.ty) else {
            return Ok(None);
        };
        if self.pending_coercion_dependencies.contains(&id) {
            return Ok(None);
        }
        let bounds = self.engine.borrow().get_bounds(id);
        if !bounds.is_empty() {
            return Ok(self
                .service()
                .select_bound_method_preferring_non_ref_receiver(
                    receiver_candidates,
                    &bounds,
                    method_name,
                    Type::TypeVar(id),
                    matches!(receiver.kind, HirExprKind::Call(_, _, _)),
                )
                .ok());
        }
        // Infer a predicate on this actual slot, never equality to the scoped
        // declaration subject or selection of a concrete implementation.
        let mut declared_bounds = Vec::new();
        for bound in self.bounds.values().flatten() {
            if !declared_bounds.contains(bound) {
                declared_bounds.push(bound.clone());
            }
        }
        let selection = self
            .service()
            .select_bound_method_preferring_non_ref_receiver(
                receiver_candidates,
                &declared_bounds,
                method_name,
                Type::TypeVar(id),
                matches!(receiver.kind, HirExprKind::Call(_, _, _)),
            );
        match selection {
            Ok(selected) => {
                if let Some(trait_id) = selected.target.trait_id() {
                    self.record_trait_bound(
                        Type::TypeVar(id),
                        TraitBound {
                            trait_id,
                            type_args: selected.target.trait_args().to_vec(),
                        },
                        receiver.span.clone(),
                        "inferred declared method bound",
                    );
                    Ok(Some(selected))
                } else {
                    Ok(None)
                }
            }
            Err(crate::selection::SelectionDiagnostic::NoImplementation { .. }) => Ok(None),
            Err(error) => {
                if !self.strict {
                    self.ambiguous = true;
                } else {
                    errors.push(ResolveError::with_span(
                        self.display_selection_error(&error),
                        receiver.span.clone(),
                    ));
                }
                Err(())
            }
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
        // A producer's directional coercion has priority over backwards method
        // inference. Choosing an impl here would freeze a concrete Self/result
        // before that producer can reveal an existential object receiver.
        let resolved_receiver = self.resolved_type(&receiver.ty);
        let mut receiver_head = &resolved_receiver;
        while let Type::Reference { inner, .. } = receiver_head {
            receiver_head = inner;
        }
        if matches!(receiver_head, Type::TypeVar(variable) if self.pending_coercion_dependencies.contains(variable))
        {
            return None;
        }
        let receiver_candidates = self.receiver_adjustment_candidates(receiver.clone());
        let receiver_ty = self.resolved_type(&receiver.ty);
        let pointee = match &receiver_ty {
            Type::Reference { inner, .. } => inner.as_ref(),
            ty => ty,
        };
        let opened_object = match pointee {
            Type::Object(object) => Some((object.as_ref().clone(), None)),
            Type::Witness(witness) => self
                .opened_object_schema(*witness)
                .map(|object| (object, Some(*witness))),
            _ => None,
        };
        if let Some((object, witness)) = opened_object {
            let bound_object = if let Some(witness) = witness {
                match object.try_map_types(|ty| {
                    crate::type_services::substitution::instantiate_object_self(
                        ty,
                        &Type::Witness(witness),
                    )
                }) {
                    Ok(object) => object,
                    Err(error) => {
                        errors.push(ResolveError::with_span(
                            error.to_string(),
                            receiver.span.clone(),
                        ));
                        return None;
                    }
                }
            } else {
                object.clone()
            };
            let bounds = std::iter::once(bound_object.principal.clone())
                .chain(bound_object.guarantees.iter().cloned())
                .collect::<Vec<_>>();
            match self.service().select_bound_method(
                &receiver_candidates,
                &bounds,
                method_name,
                pointee.clone(),
            ) {
                Ok(mut selected) => {
                    let adapted = if let Some(witness) = witness {
                        crate::hir::object_methods::adapt_opened(
                            &mut selected,
                            self.traits,
                            &object,
                            witness,
                        )
                    } else {
                        crate::hir::object_methods::adapt(&mut selected, self.traits, &object)
                    };
                    if let Err(message) = adapted {
                        errors.push(ResolveError::with_span(message, receiver.span.clone()));
                        return None;
                    }
                    return Some(selected);
                }
                Err(error) => {
                    errors.push(ResolveError::with_span(
                        self.display_selection_error(&error),
                        receiver.span.clone(),
                    ));
                    return None;
                }
            }
        }
        match self.select_receiver_bound_method(receiver, method_name, &receiver_candidates, errors)
        {
            Ok(Some(selected)) => return Some(selected),
            Ok(None) => {}
            Err(()) => return None,
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
            let ty = self.resolved_type(&candidate.expr.ty);
            let pointee = match &ty {
                Type::Reference { inner, .. } => inner.as_ref(),
                ty => ty,
            };
            if let Type::Object(object) = pointee {
                if proven.is_empty() {
                    let roots = std::iter::once(object.principal.clone())
                        .chain(object.guarantees.iter().cloned())
                        .collect::<Vec<_>>();
                    let candidates = self.receiver_adjustment_candidates(candidate.expr.clone());
                    match self.service().select_bound_method(
                        &candidates,
                        &roots,
                        method_name,
                        pointee.clone(),
                    ) {
                        Ok(mut selected) => {
                            if let Err(message) = crate::hir::object_methods::adapt(
                                &mut selected,
                                self.traits,
                                object,
                            ) {
                                errors
                                    .push(ResolveError::with_span(message, receiver.span.clone()));
                                return None;
                            }
                            return Some(selected);
                        }
                        Err(crate::selection::SelectionDiagnostic::NoImplementation { .. }) => {}
                        Err(error) => {
                            errors.push(ResolveError::with_span(
                                self.display_selection_error(&error),
                                receiver.span.clone(),
                            ));
                            return None;
                        }
                    }
                }
                continue;
            }
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
        if let HirExprKind::ObjectCoercion(inner, coercion) = &mut expr.kind {
            self.materialize_expr(inner, errors);
            if coercion.evidence.is_some()
                || !self.visit_authority_slot(AuthorityObligationKind::ObjectCoercion, &expr.span)
            {
                return;
            }
            let source = self.resolved_type(&inner.ty);
            let target = self.resolved_type(&coercion.target);
            if super::owner_coercion::is_owner_destination(&target) {
                // A directional coercion hides the source from ordinary call
                // argument unification. Relate only the declared constructor's
                // fixed slots, never its payload or independently inferred State.
                if let Some(protocol) = &self.language_items.object_owner {
                    match self.service().unary_constructor_relation(
                        &source,
                        &target,
                        protocol.trait_id,
                    ) {
                        Ok(Some((source_constructor, target_constructor))) => {
                            let mut probe = self.engine.borrow().clone_for_probe();
                            if let Err(error) =
                                probe.unify(&source_constructor, &target_constructor)
                            {
                                errors.push(ResolveError::with_span_code(
                                    format!("owner coercion must preserve the exact owner constructor and State: {}", error.render(&probe)),
                                    expr.span.clone(), crate::diagnostic::DiagnosticCode::Type,
                                ));
                                return;
                            }
                            **self.engine.borrow_mut() = probe;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            errors.push(ResolveError::with_span_code(
                                self.display_selection_error(&error),
                                expr.span.clone(),
                                crate::diagnostic::DiagnosticCode::Selection,
                            ));
                            return;
                        }
                    }
                }
                let source = self.resolved_type(&source);
                let target = self.resolved_type(&target);
                if crate::type_services::visit::type_any(&source, |ty| {
                    matches!(ty, Type::TypeVar(_))
                }) || crate::type_services::visit::type_any(&target, |ty| {
                    matches!(ty, Type::TypeVar(_))
                }) {
                    return;
                }
                let mut value = (**inner).clone();
                value.ty = source.clone();
                if source == target {
                    *expr = value;
                    return;
                }
                let local_id = HirLocalId(self.next_local_id);
                self.next_local_id += 1;
                match super::owner_coercion::lower(
                    value,
                    &target,
                    &self.service(),
                    &self.language_items,
                    local_id,
                ) {
                    Ok(lowered) => {
                        *expr = lowered;
                        self.materialize_expr(expr, errors);
                    }
                    Err(message) => {
                        errors.push(ResolveError::with_span(message, expr.span.clone()))
                    }
                }
                return;
            }
            let is_object = matches!(&target, Type::Reference { inner, .. } | Type::Pointer(inner) if matches!(inner.as_ref(), Type::Object(_)));
            let unresolved = |ty: &Type| {
                crate::type_services::visit::type_any(ty, |ty| matches!(ty, Type::TypeVar(_)))
            };
            if !is_object {
                if let (
                    Type::Reference {
                        mutable: source_mut,
                        inner: source_inner,
                    },
                    Type::Reference {
                        mutable: target_mut,
                        inner: target_inner,
                    },
                ) = (&source, &target)
                {
                    if (!target_mut || *source_mut)
                        && matches!(source_inner.as_ref(), Type::Array(_, _))
                        && matches!(target_inner.as_ref(), Type::Slice(_))
                    {
                        if let Some(value) =
                            self.array_ref_to_slice_ref((**inner).clone(), *target_mut)
                        {
                            let result = self.engine.borrow_mut().unify(&value.ty, &target);
                            match result {
                                Ok(()) => *expr = value,
                                Err(error) => errors.push(ResolveError::with_span(
                                    error.render(&self.engine.borrow()),
                                    expr.span.clone(),
                                )),
                            }
                            return;
                        }
                    }
                }
                // Waiting avoids prematurely identifying a concrete pointee with
                // an object inferred from a different call or return site.
                let potential = matches!(&source, Type::TypeVar(_) | Type::Reference { .. });
                if unresolved(&target) && potential && !self.strict {
                    return;
                }
                let result = self.engine.borrow_mut().unify(&source, &target);
                if let Err(error) = result {
                    errors.push(ResolveError::with_span(
                        error.render(&self.engine.borrow()),
                        expr.span.clone(),
                    ));
                } else {
                    let mut value = (**inner).clone();
                    value.ty = self.resolved_type(&value.ty);
                    *expr = value;
                }
                return;
            }
            let inferred = match &source {
                Type::Reference { inner, .. } | Type::Pointer(inner) => match inner.as_ref() {
                    Type::TypeVar(id) => Some(self.engine.borrow().get_bounds(*id)),
                    _ => None,
                },
                _ => None,
            };
            if (unresolved(&source) && inferred.as_ref().is_none_or(|bounds| bounds.is_empty()))
                || unresolved(&target)
            {
                return;
            }
            let context = super::object_coercion::ObjectEvidenceContext {
                traits: self.traits,
                impls: self.impls,
                structs: self.structs,
                enums: self.enums,
                language_items: &self.language_items,
                bounds: &self.bounds,
            };
            let proof = match &source {
                Type::Reference { inner, .. } | Type::Pointer(inner)
                    if matches!(inner.as_ref(), Type::Witness(_)) =>
                {
                    let Type::Witness(witness) = inner.as_ref() else {
                        unreachable!()
                    };
                    self.opened_object_schema(*witness)
                        .ok_or_else(|| {
                            "opened object coercion has no dominating dictionary".to_string()
                        })
                        .and_then(|object| {
                            context.prove_opened(&source, &target, *witness, &object)
                        })
                }
                _ => context.prove_with_assumptions(&source, &target, inferred.as_deref()),
            };
            match proof {
                Ok(evidence) => {
                    inner.ty = source;
                    coercion.target = target.clone();
                    coercion.evidence = Some(evidence);
                    expr.ty = target;
                }
                Err(message) => errors.push(ResolveError::with_span(message, expr.span.clone())),
            }
            return;
        }
        if matches!(&expr.kind, HirExprKind::Call(callee, _, _) if !native_call_type(&callee.ty) && !matches!(callee.kind, HirExprKind::FieldAccess(_, _, None)))
            && (self.visit_authority_slot(AuthorityObligationKind::CallableCall, &expr.span)
                || self.visit_authority_slot(AuthorityObligationKind::DeferredCall, &expr.span))
        {
            self.materialize_value_call(expr, errors);
        }
        match &mut expr.kind {
            HirExprKind::OwnedObjectCall { owner, args, call } => {
                self.materialize_expr(owner, errors);
                for arg in args {
                    self.materialize_expr(arg, errors);
                }
                call.visit_types_mut(&mut |ty| *ty = self.resolved_type(ty));
                expr.ty = self.resolved_type(&expr.ty);
            }
            HirExprKind::PackagePack { payload, proof } => {
                self.materialize_expr(payload, errors);
                proof.visit_types_mut(&mut |ty| *ty = self.resolved_type(ty));
                expr.ty = self.resolved_type(&expr.ty);
                if let Type::Exists(package) = &expr.ty {
                    let actual = self.resolved_type(&payload.ty);
                    if crate::type_services::visit::type_any(&actual, |ty| {
                        matches!(ty, Type::TypeVar(_))
                    }) && !self.strict
                    {
                        return;
                    }
                    let env = self.engine.borrow().normalization_env().clone();
                    let result = crate::type_services::existential::infer_existential_witness(
                        package, &actual, &env,
                    )
                    .and_then(|witness| {
                        let opened = if let Type::Witness(witness) = &witness {
                            self.opened_object_schema(*witness)
                        } else {
                            None
                        };
                        let context = super::object_coercion::ObjectEvidenceContext {
                            traits: self.traits,
                            impls: self.impls,
                            structs: self.structs,
                            enums: self.enums,
                            language_items: &self.language_items,
                            bounds: &self.bounds,
                        };
                        crate::hir::packages::prove_pack(
                            &context,
                            package,
                            &witness,
                            opened.as_ref(),
                        )
                    });
                    match result {
                        Ok(expected) => *proof = expected,
                        Err(message) if self.strict => {
                            errors.push(ResolveError::with_span(message, expr.span.clone()))
                        }
                        Err(_) => {}
                    }
                }
            }
            HirExprKind::PackageOpen {
                source,
                binding,
                body,
            } => {
                self.materialize_expr(source, errors);
                binding.visit_types_mut(&mut |ty| *ty = self.resolved_type(ty));
                self.opened_packages.push(binding.clone());
                self.engine
                    .borrow_mut()
                    .enter_witness_scope(binding.witness);
                self.materialize_block(body, errors);
                self.engine.borrow_mut().exit_witness_scope(binding.witness);
                self.opened_packages.pop();
                expr.ty = self.resolved_type(&expr.ty);
            }
            HirExprKind::Open {
                source,
                binding,
                body,
            } => {
                self.materialize_expr(source, errors);
                binding.visit_types_mut(&mut |ty| *ty = self.resolved_type(ty));
                self.opened_witnesses.push(binding.clone());
                self.engine
                    .borrow_mut()
                    .enter_witness_scope(binding.witness);
                self.materialize_block(body, errors);
                self.engine.borrow_mut().exit_witness_scope(binding.witness);
                self.opened_witnesses.pop();
                expr.ty = self.resolved_type(&expr.ty);
            }
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
                        self.materialize_direct_function_call(
                            &expr.ty,
                            callee,
                            args,
                            *function_id,
                            errors,
                        );
                    }
                    return;
                }
                let (receiver, method_name) = match &mut callee.kind {
                    HirExprKind::FieldAccess(receiver, method_name, None) => {
                        // Child sites must update the actual tree even when this
                        // invocation is materializing a different authority slot.
                        self.materialize_expr(receiver, errors);
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
                    for arg in args.iter_mut() {
                        self.materialize_expr(arg, errors);
                    }
                    return;
                }
                for arg in args.iter_mut() {
                    self.materialize_expr(arg, errors);
                }
                match self.materialize_owned_call(
                    &receiver,
                    &method_name,
                    args,
                    &expr.ty,
                    &expr.span,
                    errors,
                ) {
                    Ok(Some(owned)) => {
                        *expr = owned;
                        return;
                    }
                    Ok(None) => {}
                    Err(()) => return,
                }
                let Some(selected) = self.select_deferred_method(
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
                let adjusted_receiver = self.apply_receiver_adjustment(
                    selected.receiver.clone(),
                    selected.receiver_adjustment,
                );

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
            HirExprKind::Deref(_) => self.materialize_dereference(expr, errors),
            HirExprKind::Ref(_, receiver)
            | HirExprKind::Cast(receiver, _)
            | HirExprKind::ObjectCoercion(receiver, _)
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
            HirExprKind::Block(block) => self.materialize_block(block, errors),
            HirExprKind::Loop(block) => self.materialize_loop(block, &expr.ty, errors),
            HirExprKind::UnsafeBlock(block) => {
                let previous = self.unsafe_context.replace(true);
                self.materialize_block(block, errors);
                self.unsafe_context.set(previous);
            }
            HirExprKind::While { condition, body } => {
                self.materialize_expr(condition, errors);
                self.materialize_loop(body, &expr.ty, errors);
            }
            HirExprKind::For { iter, body, .. } => {
                self.materialize_expr(iter, errors);
                self.materialize_loop(body, &expr.ty, errors);
            }
            HirExprKind::Lambda { body, .. } => {
                let previous = self.unsafe_context.replace(false);
                let loops = std::mem::take(&mut self.loop_targets);
                self.materialize_block(body, errors);
                self.loop_targets = loops;
                self.unsafe_context.set(previous);
            }
            HirExprKind::Assign(left, right) => {
                self.materialize_expr(left, errors);
                self.materialize_expr(right, errors);
                if self.visit_authority_slot(AuthorityObligationKind::Assignment, &left.span)
                    && crate::lower::place::assignment_access(left, &|ty| self.resolved_type(ty))
                        == crate::lower::place::AssignmentAccess::Shared
                {
                    errors.push(ResolveError::with_span_code(
                        crate::lower::place::SHARED_ASSIGNMENT.into(),
                        left.span.clone(),
                        crate::diagnostic::DiagnosticCode::Type,
                    ));
                }
            }
            HirExprKind::BinOp(_, left, right) => {
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
            match probe.constrain_argument_binding(&actual, &expected) {
                Ok(binding) => {
                    self.engine.borrow_mut().commit_probe(probe);
                    arg.ty = binding;
                }
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
            HirExprKind::PackagePack { payload, .. } => {
                self.collect_method_value_captures(payload, receiver_mode, captures, seen)
            }
            HirExprKind::Open { source, .. } | HirExprKind::PackageOpen { source, .. } => {
                self.collect_method_value_captures(source, receiver_mode, captures, seen);
            }
            HirExprKind::OwnedObjectCall { owner, args, .. } => {
                self.collect_method_value_captures(
                    owner,
                    Some(crate::types::ReceiverMode::Move),
                    captures,
                    seen,
                );
                for arg in args {
                    self.collect_method_value_captures(
                        arg,
                        Some(crate::types::ReceiverMode::Move),
                        captures,
                        seen,
                    );
                }
            }
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
            | HirExprKind::Cast(base, _)
            | HirExprKind::ObjectCoercion(base, _) => {
                self.collect_method_value_captures(base, receiver_mode, captures, seen)
            }
            _ => {}
        }
    }

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
        let mut owned_binding = None;
        let kind = if function.self_receiver == Some(crate::types::ReceiverMode::Move)
            && (crate::hir::owned_objects::object(&receiver.ty).is_some()
                || self
                    .opened_witnesses
                    .iter()
                    .any(|binding| binding.matches_owner(&receiver.ty)))
        {
            match crate::hir::owned_objects::prove(
                &receiver.ty,
                target,
                self.traits,
                &self.language_items,
                &self.service(),
                self.opened_witnesses
                    .iter()
                    .rev()
                    .find(|binding| binding.matches_owner(&receiver.ty)),
            ) {
                Ok(call) => {
                    let local_id = self.fresh_local_id();
                    let name = "<owned-method-self>".to_string();
                    let owner = HirExpr {
                        ty: receiver.ty.clone(),
                        span: receiver.span.clone(),
                        kind: HirExprKind::ResolvedVar(HirVarRef {
                            name: name.clone(),
                            target: HirVarTarget::Local(local_id),
                        }),
                    };
                    owned_binding = Some((local_id, name, receiver.clone()));
                    HirExprKind::OwnedObjectCall {
                        owner: Box::new(owner),
                        args: call_args,
                        call,
                    }
                }
                Err(message) => {
                    errors.push(ResolveError::with_span(message, expr.span.clone()));
                    return;
                }
            }
        } else {
            HirExprKind::MethodCall(
                Box::new(adjusted_receiver),
                method_name,
                call_args,
                function.self_receiver,
                Some(target),
            )
        };
        let call = HirExpr {
            ty: call_ret.clone(),
            kind,
            span: expr.span.clone(),
        };
        let body = HirBlock {
            ty: call_ret.clone(),
            stmts: vec![HirStmt::Expr(call)],
        };
        let mut captures = Vec::new();
        if let Some((local_id, name, owner)) = &owned_binding {
            captures.push(HirClosureCapture {
                local_id: *local_id,
                name: name.clone(),
                ty: owner.ty.clone(),
                mutable: false,
                kind: HirClosureCaptureKind::Move,
            });
        } else {
            self.collect_method_value_captures(
                &receiver,
                function.self_receiver,
                &mut captures,
                &mut HashSet::new(),
            );
        }
        let params = lambda_params.iter().map(|param| param.ty.clone()).collect();
        let lambda_ty = self.lambda_function_type(
            params,
            call_ret.clone(),
            FunctionSafety::from_is_unsafe(function.is_unsafe),
            &captures,
        );
        let check = self.engine.borrow_mut().unify(&expr.ty, &lambda_ty);
        if let Err(error) = check {
            errors.push(ResolveError::with_span(
                error.render(&self.engine.borrow()),
                expr.span.clone(),
            ));
            return;
        }
        expr.ty = lambda_ty;
        let lambda = HirExpr {
            ty: expr.ty.clone(),
            span: expr.span.clone(),
            kind: HirExprKind::Lambda {
                params: lambda_params,
                body,
                captures,
            },
        };
        if let Some((local_id, name, owner)) = owned_binding {
            expr.kind = HirExprKind::Block(HirBlock {
                ty: expr.ty.clone(),
                stmts: vec![
                    HirStmt::Let {
                        name,
                        local_id,
                        ty: owner.ty.clone(),
                        value: owner,
                        mutable: false,
                    },
                    HirStmt::Expr(lambda),
                ],
            });
        } else {
            expr.kind = lambda.kind;
        }
    }

    fn materialize_field_access(&mut self, expr: &mut HirExpr, errors: &mut Vec<ResolveError>) {
        let HirExprKind::FieldAccess(receiver, field_name, location) = &mut expr.kind else {
            return;
        };
        self.materialize_expr(receiver, errors);
        receiver.ty = self.engine.borrow().resolve(&receiver.ty);
        if location.is_none() {
            match crate::hir::owned_objects::select(
                receiver,
                field_name,
                self.traits,
                &self.language_items,
                &self.service(),
                self.opened_witnesses
                    .iter()
                    .rev()
                    .find(|binding| binding.matches_owner(&receiver.ty)),
            ) {
                Ok(Some((selected, _))) => {
                    if selected
                        .function
                        .as_ref()
                        .is_some_and(|function| function.is_unsafe)
                        && !self.unsafe_context.get()
                    {
                        errors.push(ResolveError::with_span(
                            "unsafe consuming method value requires an unsafe block".into(),
                            expr.span.clone(),
                        ));
                        return;
                    }
                    let receiver = receiver.as_ref().clone();
                    let name = field_name.clone();
                    self.materialize_method_value(expr, receiver, name, selected, errors);
                    return;
                }
                Ok(None) => {}
                Err(message) => {
                    errors.push(ResolveError::with_span(message, expr.span.clone()));
                    return;
                }
            }
        }
        if let Type::Reference { inner, .. } = &receiver.ty {
            let dereferenced = HirExpr {
                ty: inner.as_ref().clone(),
                kind: HirExprKind::Deref(Box::new(receiver.as_ref().clone())),
                span: receiver.span.clone(),
            };
            **receiver = dereferenced;
        }
        let receiver_ty = &receiver.ty;
        let (Type::Struct { id, args } | Type::StructRepresentation { id, args, .. }) = receiver_ty
        else {
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
        let field_ty = field.ty.substitute_generics(&substitution);
        let result = self.engine.borrow_mut().unify(&expr.ty, &field_ty);
        if let Err(error) = result {
            errors.push(ResolveError::with_span_code(
                format!(
                    "field type mismatch: {}",
                    error.render(&self.engine.borrow())
                ),
                expr.span.clone(),
                crate::diagnostic::DiagnosticCode::Type,
            ));
        } else {
            expr.ty = self.resolved_type(&field_ty);
        }
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

    #[test]
    #[ignore = "coordinator supplies exact source13 and the verified primary7 stdlib; frontend observation only"]
    fn capture_allocator_source13_consuming_owner_authority() {
        use std::path::{Path, PathBuf};

        fn digest(path: &Path, expected: &str) {
            let output = std::process::Command::new("sha256sum")
                .arg(path)
                .output()
                .expect("read-only fixture digest utility");
            assert!(output.status.success(), "{output:?}");
            let text = String::from_utf8(output.stdout).unwrap();
            assert_eq!(text.split_whitespace().next(), Some(expected));
            eprintln!("OWNER13_BYTES path={} sha256={expected}", path.display());
        }

        fn ty(hir: &PartialHir, raw: &Type, types: &mut HashSet<Type>) -> Type {
            for value in [raw.clone(), hir.engine.resolve(raw)] {
                crate::type_services::visit::visit_type(&value, &mut |nested: &Type| {
                    types.insert(nested.clone());
                    assert!(types.len() <= 256, "source13 observer type cap exceeded");
                });
            }
            hir.engine.resolve(raw)
        }

        fn signature(
            hir: &PartialHir,
            function: &HirFunction,
            counts: &mut [usize; 3],
            types: &mut HashSet<Type>,
        ) {
            counts[0] += 1;
            assert!(counts[0] <= 24, "source13 observer function cap exceeded");
            let params = function
                .params
                .iter()
                .map(|param| (param.local_id, param.ty.clone(), ty(hir, &param.ty, types)))
                .collect::<Vec<_>>();
            let ret = ty(hir, &function.ret_type, types);
            for bounds in function.generic_bounds.values() {
                for bound in bounds {
                    for argument in &bound.type_args {
                        ty(hir, argument, types);
                    }
                }
            }
            for predicate in &function.generic_bounds.predicates {
                let crate::types::Predicate::Trait { subject, args, .. } = predicate;
                ty(hir, subject, types);
                for argument in args {
                    ty(hir, argument, types);
                }
            }
            eprintln!("OWNER13_FUNCTION id={:?} name={} params={params:?} raw_return={:?} resolved_return={ret:?} generics={:?} bounds={:?}",
                function.id, function.name, function.ret_type, function.generic_params, function.generic_bounds);
        }

        fn method_types(
            hir: &PartialHir,
            target: &crate::hir::HirMethodCallTarget,
            types: &mut HashSet<Type>,
        ) {
            for value in target
                .trait_args()
                .iter()
                .chain(target.owner_substitution.iter().map(|binding| &binding.ty))
                .chain(target.method_substitution.iter().map(|binding| &binding.ty))
            {
                ty(hir, value, types);
            }
        }

        fn normalized(
            hir: &PartialHir,
            raw: &Type,
            types: &mut HashSet<Type>,
        ) -> Result<Type, String> {
            let resolved = ty(hir, raw, types);
            let result = hir.engine.normalize_resolved_type(&resolved);
            if let Ok(value) = &result {
                ty(hir, value, types);
            }
            result
        }

        fn candidate_start(owner: DefId, role: &str, counts: &mut [usize; 3]) {
            counts[1] += 1;
            assert!(counts[1] <= 32, "source13 printed-site cap exceeded");
            eprintln!("OWNER13_CANDIDATE owner={owner:?} role={role} authority=CANDIDATE_ONLY actualSourceCallTarget=None nativeKey=UNMEASURED selectedMemberSchemaSlot=UNMEASURED objectPlan=None RuntimeAgreement=UNMEASURED(frontend_only) execution=NONE");
        }

        fn candidate_header(
            hir: &PartialHir,
            function: Option<&HirFunction>,
            target: Option<&crate::hir::HirMethodCallTarget>,
            counts: &mut [usize; 3],
            types: &mut HashSet<Type>,
        ) {
            if let Some(function) = function {
                assert!(
                    function.params.len() <= 32
                        && function.generic_params.len() <= 32
                        && function.generic_bounds.len() <= 32
                        && function.generic_bounds.predicates.len() <= 32
                        && function.body.stmts.len() <= 32,
                    "source13 candidate header cap exceeded"
                );
                for bounds in function.generic_bounds.values() {
                    assert!(bounds.len() <= 32, "source13 candidate bound cap exceeded");
                    for bound in bounds {
                        assert!(
                            bound.type_args.len() <= 32,
                            "source13 candidate bound argument cap exceeded"
                        );
                    }
                }
                for predicate in &function.generic_bounds.predicates {
                    let crate::types::Predicate::Trait { args, .. } = predicate;
                    assert!(
                        args.len() <= 32,
                        "source13 candidate predicate argument cap exceeded"
                    );
                }
                signature(hir, function, counts, types);
                let safety = if function.is_unsafe {
                    FunctionSafety::Unsafe
                } else {
                    FunctionSafety::Safe
                };
                let header = Type::function_with_safety(
                    function
                        .params
                        .iter()
                        .map(|param| param.ty.clone())
                        .collect(),
                    function.ret_type.clone(),
                    safety,
                );
                eprintln!("OWNER13_CANDIDATE_HEADER providerDefinition={:?} unsafe={} static={} receiver={:?} CallSafety={safety:?} normalizedHeader={:?} bodyStatements={}",
                    function.id, function.is_unsafe, !function.is_method && function.self_receiver.is_none(),
                    function.self_receiver, normalized(hir, &header, types), function.body.stmts.len());
            } else {
                counts[0] += 1;
                assert!(counts[0] <= 24, "source13 observer function cap exceeded");
                eprintln!("OWNER13_CANDIDATE_HEADER missing=true");
            }
            if let Some(target) = target {
                assert!(
                    target.trait_args().len() <= 32
                        && target.owner_substitution.len() <= 32
                        && target.method_substitution.len() <= 32,
                    "source13 candidate substitution cap exceeded"
                );
                method_types(hir, target, types);
                eprintln!("OWNER13_CANDIDATE_TARGET canonical={target:?}");
            }
        }

        fn candidate_error(
            hir: &PartialHir,
            error: &crate::selection::SelectionDiagnostic,
            types: &mut HashSet<Type>,
        ) {
            use crate::selection::SelectionDiagnostic;
            match error {
                SelectionDiagnostic::NoImplementation { receiver, .. }
                | SelectionDiagnostic::ReceiverMismatch { receiver, .. } => {
                    ty(hir, receiver, types);
                }
                SelectionDiagnostic::AmbiguousCandidates {
                    receiver,
                    candidates,
                    ..
                } => {
                    assert!(
                        candidates.len() <= 32,
                        "source13 candidate chain cap exceeded"
                    );
                    ty(hir, receiver, types);
                }
                SelectionDiagnostic::SelectedTargetMissing { target, .. } => {
                    assert!(
                        target.trait_args().len() <= 32
                            && target.owner_substitution.len() <= 32
                            && target.method_substitution.len() <= 32,
                        "source13 rejected target cap exceeded"
                    );
                    method_types(hir, target, types);
                }
                SelectionDiagnostic::TraitMemberMissing { .. }
                | SelectionDiagnostic::TraitMemberIdMissing { .. } => {}
            }
        }

        fn candidate_obligations(
            hir: &PartialHir,
            service: &SelectionService<'_>,
            pending: &[(Type, TraitBound)],
            substitution: &HashMap<GenericParamId, Type>,
            rows: &mut usize,
            types: &mut HashSet<Type>,
        ) {
            assert!(
                pending.len() <= 32,
                "source13 candidate obligation cap exceeded"
            );
            eprintln!(
                "OWNER13_CANDIDATE_OBLIGATIONS selectedPendingCount={} (not_assumed_empty)",
                pending.len()
            );
            for (subject, bound) in pending {
                *rows += 1;
                assert!(
                    *rows <= 32,
                    "source13 candidate obligation-row cap exceeded"
                );
                ty(hir, subject, types);
                assert!(
                    bound.type_args.len() <= 32,
                    "source13 candidate obligation argument cap exceeded"
                );
                for argument in &bound.type_args {
                    ty(hir, argument, types);
                }
                let instantiated = subject.substitute_generics(substitution);
                let instantiated_bound = TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|argument| argument.substitute_generics(substitution))
                        .collect(),
                };
                let resolved = normalized(hir, &instantiated, types);
                for argument in &instantiated_bound.type_args {
                    ty(hir, argument, types);
                }
                let satisfied = resolved
                    .as_ref()
                    .ok()
                    .map(|subject| service.trait_bound_satisfied(subject, &instantiated_bound));
                eprintln!("OWNER13_CANDIDATE_OBLIGATION rawSubject={subject:?} rawBound={bound:?} instantiatedSubject={resolved:?} instantiatedBound={instantiated_bound:?} satisfied={satisfied:?}");
            }
        }

        fn candidate_operations(
            hir: &PartialHir,
            counts: &mut [usize; 3],
            types: &mut HashSet<Type>,
        ) {
            let transport = hir
                .language_items
                .object_owner
                .as_ref()
                .expect("actual transport item");
            let unique = hir
                .language_items
                .object_owner_unique
                .as_ref()
                .expect("actual unique item");
            let mut receipts = 0;
            let mut obligation_rows = 0;
            for (helper, range) in [("run_small", (9264, 9275)), ("run_large", (9389, 9400))] {
                let id = hir.resolver.item_paths[helper];
                let function = &hir.functions[&id];
                assert!(
                    function.body.stmts.len() <= 32,
                    "source13 candidate body cap exceeded"
                );
                let nodes = function
                    .body
                    .stmts
                    .iter()
                    .filter_map(|statement| match statement {
                        HirStmt::Let { value, .. } | HirStmt::Expr(value)
                            if (value.span.start, value.span.end) == range =>
                        {
                            Some(value)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(nodes.len(), 1, "actual source13 consuming site");
                let node = nodes[0];
                let HirExprKind::Call(callee, _, target) = &node.kind else {
                    panic!("source13 actual call shape changed");
                };
                let HirExprKind::FieldAccess(receiver, member, location) = &callee.kind else {
                    panic!("source13 actual field shape changed");
                };
                let mut sites = Vec::new();
                collect_authority_sites(&function.body, &hir.engine, &mut sites);
                assert!(sites.len() <= 32, "source13 authority-site cap exceeded");
                let deferred = sites.iter().any(|(kind, _, _, span, _, _)| {
                    *kind == AuthorityObligationKind::DeferredCall && *span == node.span
                });
                eprintln!("OWNER13_ACTUAL_CONSUMING_SITE owner={id:?} span={:?} actualSourceCallTarget={target:?} fieldLocation={location:?} DeferredCall={deferred}", node.span);
                assert!(
                    target.is_none() && location.is_none() && deferred,
                    "preserve actual unresolved source13 consuming authority"
                );
                let owner = ty(hir, &receiver.ty, types);
                let mut bounds = function.generic_bounds.clone();
                add_declared_sized_bounds(
                    &mut bounds,
                    &function.generic_params,
                    hir.language_items.sized.as_ref().map(|item| item.trait_id),
                );
                let service = SelectionService::new(
                    &hir.traits,
                    &hir.impls,
                    hir.language_items.sized.as_ref().map(|item| item.trait_id),
                    None,
                    &bounds,
                )
                .with_effective_trait_methods(&hir.imported_effective_trait_methods);
                let application =
                    service.infer_unary_constructor_application(&owner, transport.trait_id);
                match &application {
                    Ok(Some((constructor, payload, state))) => {
                        assert_eq!(state.len(), 1, "actual transport State arity");
                        ty(hir, constructor, types);
                        ty(hir, payload, types);
                        for value in state {
                            ty(hir, value, types);
                        }
                    }
                    Err(error) => candidate_error(hir, error, types),
                    Ok(None) => {}
                }
                eprintln!("OWNER13_CANDIDATE_APPLICATION owner={id:?} result={application:?} context={bounds:?}");
                let Ok(Some((constructor, payload, state_args))) = application else {
                    for role in ["split", "rebuild", "release", "consuming-member"] {
                        receipts += 1;
                        assert!(receipts <= 8, "source13 candidate receipt cap exceeded");
                        candidate_start(id, role, counts);
                        candidate_header(hir, None, None, counts, types);
                        eprintln!("OWNER13_CANDIDATE_RESULT unavailable=constructor_application");
                    }
                    continue;
                };
                ty(hir, &constructor, types);
                ty(hir, &payload, types);
                assert_eq!(state_args.len(), 1, "actual transport State arity");
                let state = &state_args[0];
                let applied = Type::Apply {
                    constructor: Box::new(constructor.clone()),
                    args: vec![payload.clone()],
                };
                let normalized_owner = normalized(hir, &owner, types);
                let normalized_applied = normalized(hir, &applied, types);
                let sized_id = hir.language_items.sized.as_ref().map(|item| item.trait_id);
                let state_sized = sized_id.map(|trait_id| {
                    service.trait_bound_satisfied(
                        state,
                        &TraitBound {
                            trait_id,
                            type_args: Vec::new(),
                        },
                    )
                });
                eprintln!("OWNER13_CANDIDATE_SECTION constructor={constructor:?} payload={payload:?} State={:?} normalizedApplication={normalized_applied:?} normalizedOwner={normalized_owner:?} exactApplication={} canonicalSizedTrait={sized_id:?} independentStateSized={state_sized:?} relationSuppliesBounds=false",
                    normalized(hir, state, types), normalized_applied.is_ok() && normalized_applied == normalized_owner);
                let parts = Type::Tuple(vec![
                    Type::Pointer(Box::new(payload.clone())),
                    state.clone(),
                ]);
                for (role, trait_id, member_id, expected) in [
                    (
                        "split",
                        transport.trait_id,
                        transport.into_parts_id,
                        Type::function_with_safety(
                            vec![owner.clone()],
                            parts,
                            FunctionSafety::Unsafe,
                        ),
                    ),
                    (
                        "rebuild",
                        transport.trait_id,
                        transport.from_parts_id,
                        Type::function_with_safety(
                            vec![Type::Pointer(Box::new(payload.clone())), state.clone()],
                            owner.clone(),
                            FunctionSafety::Unsafe,
                        ),
                    ),
                    (
                        "release",
                        unique.trait_id,
                        unique.method_id,
                        Type::function_with_safety(
                            vec![Type::Pointer(Box::new(Type::U8)), state.clone()],
                            Type::Unit,
                            FunctionSafety::Unsafe,
                        ),
                    ),
                ] {
                    receipts += 1;
                    assert!(receipts <= 8, "source13 candidate receipt cap exceeded");
                    candidate_start(id, role, counts);
                    let raw = service.select_constructor_trait_member(
                        &constructor,
                        trait_id,
                        &state_args,
                        member_id,
                    );
                    let checked = crate::infer::owner_coercion::select_operation(
                        &service,
                        &constructor,
                        trait_id,
                        &state_args,
                        member_id,
                        &expected,
                    );
                    let normalized_expected = normalized(hir, &expected, types);
                    eprintln!("OWNER13_CANDIDATE_CONTRACT trait={trait_id:?} member={member_id:?} expected={normalized_expected:?}");
                    match raw {
                        Ok(raw) => {
                            candidate_header(
                                hir,
                                Some(&raw.function),
                                Some(&raw.target),
                                counts,
                                types,
                            );
                            assert!(
                                raw.impl_def.trait_arg_types.len() <= 32
                                    && raw.impl_def.type_generics.len() <= 32,
                                "source13 raw implementation header cap exceeded"
                            );
                            match &raw.impl_def.receiver_pattern {
                                crate::hir::HirImplReceiverPattern::Exact(value)
                                | crate::hir::HirImplReceiverPattern::Constructor(value) => {
                                    ty(hir, value, types);
                                }
                                crate::hir::HirImplReceiverPattern::SliceFamily { element } => {
                                    ty(hir, element, types);
                                }
                            }
                            for argument in &raw.impl_def.trait_arg_types {
                                ty(hir, argument, types);
                            }
                            eprintln!("OWNER13_CANDIDATE_RAW_CHECK impl={:?} trait={:?} declaredReceiver={:?} declaredTraitArgs={:?} fixedGenerics={:?} unsafeStatic={}",
                                raw.impl_def.id, raw.impl_def.trait_id, raw.impl_def.receiver_pattern,
                                raw.impl_def.trait_arg_types, raw.impl_def.type_generics, raw.function.is_unsafe && raw.function.self_receiver.is_none());
                            let substitution = match &checked {
                                Ok((_, target)) => {
                                    assert!(
                                        target.method.owner_substitution.len() <= 32
                                            && target.method.method_substitution.len() <= 32,
                                        "source13 checked substitution cap exceeded"
                                    );
                                    ty(hir, &target.owner_ty, types);
                                    method_types(hir, &target.method, types);
                                    target
                                        .method
                                        .owner_substitution
                                        .iter()
                                        .chain(&target.method.method_substitution)
                                        .map(|binding| (binding.param, binding.ty.clone()))
                                        .collect()
                                }
                                Err(_) => raw.owner_substitution.clone(),
                            };
                            candidate_obligations(
                                hir,
                                &service,
                                &raw.pending_impl_bounds,
                                &substitution,
                                &mut obligation_rows,
                                types,
                            );
                        }
                        Err(error) => {
                            candidate_header(hir, None, None, counts, types);
                            candidate_error(hir, &error, types);
                            eprintln!("OWNER13_CANDIDATE_RAW_REJECTED diagnostic={error:?}");
                        }
                    }
                    match checked {
                        Ok((selected, target)) => {
                            assert!(selected.params.len() <= 32 && target.method.trait_args().len() <= 32
                                && target.method.owner_substitution.len() <= 32 && target.method.method_substitution.len() <= 32,
                                "source13 checked header cap exceeded");
                            ty(hir, &target.owner_ty, types);
                            method_types(hir, &target.method, types);
                            let substitution = target.method.owner_substitution.iter().chain(&target.method.method_substitution)
                                .map(|binding| (binding.param, binding.ty.clone())).collect::<HashMap<_, _>>();
                            let instantiated = Type::function_with_safety(selected.params.iter().map(|param| param.ty.substitute_generics(&substitution)).collect(),
                                selected.ret_type.substitute_generics(&substitution), if selected.is_unsafe { FunctionSafety::Unsafe } else { FunctionSafety::Safe });
                            let normalized_header = normalized(hir, &instantiated, types);
                            let complete = normalized_header.as_ref().ok().map(|header| !crate::type_services::visit::type_any(header, |value| {
                                matches!(value, Type::Generic(_) | Type::TypeVar(_) | Type::Error | Type::BoundVar { .. } | Type::ObjectSelf { .. } | Type::Witness(_))
                            }));
                            eprintln!("OWNER13_CANDIDATE_RESULT checked=Ok canonicalTarget={target:?} instantiatedHeader={normalized_header:?} headerComplete={complete:?} exactContract={} sourceTargetInstalled=false",
                                normalized_header.is_ok() && normalized_header == normalized_expected);
                        }
                        Err(error) => eprintln!("OWNER13_CANDIDATE_RESULT checked=Err diagnostic={error:?} sourceTargetInstalled=false"),
                    }
                }
                receipts += 1;
                assert!(receipts <= 8, "source13 candidate receipt cap exceeded");
                candidate_start(id, "consuming-member", counts);
                let Type::Object(object) = &payload else {
                    candidate_header(hir, None, None, counts, types);
                    eprintln!("OWNER13_CANDIDATE_RESULT unavailable=non_object_payload");
                    continue;
                };
                assert!(
                    object.guarantees.len() < 32,
                    "source13 candidate root cap exceeded"
                );
                let roots = std::iter::once(object.principal.clone())
                    .chain(object.guarantees.iter().cloned())
                    .collect::<Vec<_>>();
                for root in &roots {
                    assert!(
                        root.type_args.len() <= 32,
                        "source13 candidate root argument cap exceeded"
                    );
                    for argument in &root.type_args {
                        ty(hir, argument, types);
                    }
                }
                // A detached signature probe, never an installed runtime receiver.
                let candidate = ReceiverCandidate {
                    expr: HirExpr {
                        ty: payload.clone(),
                        ..receiver.as_ref().clone()
                    },
                    adjustment: ReceiverAdjustment::None,
                    can_autoref_mut: false,
                };
                match service.select_bound_method(&[candidate], &roots, member, payload.clone()) {
                    Ok(selected) => {
                        candidate_header(
                            hir,
                            selected.function.as_ref(),
                            Some(&selected.target),
                            counts,
                            types,
                        );
                        let substitution = selected
                            .target
                            .owner_substitution
                            .iter()
                            .chain(&selected.target.method_substitution)
                            .map(|binding| (binding.param, binding.ty.clone()))
                            .collect();
                        candidate_obligations(
                            hir,
                            &service,
                            &selected.pending_impl_bounds,
                            &substitution,
                            &mut obligation_rows,
                            types,
                        );
                        let signature = crate::hir::object_methods::consuming_signature(
                            &hir.traits,
                            &selected.target,
                            object,
                        );
                        match signature {
                            Ok((params, ret)) => {
                                assert!(params.len() < 32, "source13 consuming signature cap exceeded");
                                let safety = selected.function.as_ref().map(|function| if function.is_unsafe { FunctionSafety::Unsafe } else { FunctionSafety::Safe });
                                let header = safety.map(|safety| Type::function_with_safety(std::iter::once(payload.clone()).chain(params).collect(), ret, safety));
                                eprintln!("OWNER13_CANDIDATE_RESULT consumingSignature={:?} selectedOrigin={:?} receiverAdjustment={:?} sourceTargetInstalled=false",
                                    header.as_ref().map(|header| normalized(hir, header, types)), selected.origin, selected.receiver_adjustment);
                            }
                            Err(error) => eprintln!("OWNER13_CANDIDATE_RESULT consumingSignature=Err diagnostic={error:?} sourceTargetInstalled=false"),
                        }
                    }
                    Err(error) => {
                        candidate_header(hir, None, None, counts, types);
                        candidate_error(hir, &error, types);
                        eprintln!("OWNER13_CANDIDATE_RESULT selection=Err diagnostic={error:?} sourceTargetInstalled=false");
                    }
                }
            }
            assert_eq!(
                receipts, 8,
                "source13 must report all eight candidate outcomes"
            );
            eprintln!("OWNER13_CANDIDATE_CAPS receipts={receipts} functions={} sites={} impls={} types={} obligationRows={obligation_rows}", counts[0], counts[1], counts[2], types.len());
        }

        fn snapshot(
            hir: &PartialHir,
            stage: &str,
            counts: &mut [usize; 3],
            types: &mut HashSet<Type>,
        ) {
            eprintln!("OWNER13_STAGE {stage}");
            let transport = hir
                .language_items
                .object_owner
                .as_ref()
                .expect("actual ObjectOwner language item");
            let unique = hir
                .language_items
                .object_owner_unique
                .as_ref()
                .expect("actual Unique language item");
            eprintln!("OWNER13_PROTOCOL transport={transport:?} unique={unique:?}");
            let helper_ids = ["run_small", "run_large"].map(|name| {
                *hir.resolver
                    .item_paths
                    .get(name)
                    .expect("actual helper DefId")
            });
            for name in ["run_small", "run_large", "erase", "upcast", "main"] {
                let id = hir.resolver.item_paths[name];
                let function = &hir.functions[&id];
                signature(hir, function, counts, types);
                let mut bounds = function.generic_bounds.clone();
                add_declared_sized_bounds(
                    &mut bounds,
                    &function.generic_params,
                    hir.language_items.sized.as_ref().map(|item| item.trait_id),
                );
                if helper_ids.contains(&id) {
                    eprintln!("OWNER13_CONTEXT owner={id:?} effective_bounds={bounds:?}");
                }
                let service = SelectionService::new(
                    &hir.traits,
                    &hir.impls,
                    hir.language_items.sized.as_ref().map(|item| item.trait_id),
                    None,
                    &bounds,
                )
                .with_effective_trait_methods(&hir.imported_effective_trait_methods);
                let mut sites = Vec::new();
                if helper_ids.contains(&id) {
                    collect_authority_sites(&function.body, &hir.engine, &mut sites);
                    assert!(sites.len() <= 32, "source13 authority-site cap exceeded");
                }
                for statement in &function.body.stmts {
                    let node = match statement {
                        HirStmt::Let { value, .. } | HirStmt::Expr(value) => value,
                        _ => continue,
                    };
                    let member = match &node.kind {
                        HirExprKind::Call(callee, args, target) if helper_ids.contains(&id) => {
                            if let HirExprKind::FieldAccess(receiver, name, location) = &callee.kind
                            {
                                if let Some(HirCallTarget::StaticMethod(target)) = target {
                                    ty(hir, &target.owner_ty, types);
                                    method_types(hir, &target.method, types);
                                }
                                eprintln!("OWNER13_MEMBER_TARGET call_target={target:?} field_location={location:?} raw_callee={:?} resolved_callee={:?}",
                                    callee.ty, ty(hir, &callee.ty, types));
                                Some((receiver.as_ref(), name.as_str(), args.as_slice()))
                            } else {
                                None
                            }
                        }
                        HirExprKind::MethodCall(receiver, name, args, mode, target)
                            if helper_ids.contains(&id) =>
                        {
                            if let Some(target) = target {
                                method_types(hir, target, types);
                            }
                            eprintln!(
                                "OWNER13_MEMBER_TARGET mode={mode:?} method_target={target:?}"
                            );
                            Some((receiver.as_ref(), name.as_str(), args.as_slice()))
                        }
                        _ => None,
                    };
                    if let Some((receiver, member, args)) = member {
                        counts[1] += 1;
                        assert!(counts[1] <= 32, "source13 printed-site cap exceeded");
                        let resolved = ty(hir, &receiver.ty, types);
                        let result = ty(hir, &node.ty, types);
                        let reference = match &receiver.kind {
                            HirExprKind::ResolvedVar(reference) => Some(reference),
                            HirExprKind::Ref(_, inner) | HirExprKind::Deref(inner) => {
                                match &inner.kind {
                                    HirExprKind::ResolvedVar(reference) => Some(reference),
                                    _ => None,
                                }
                            }
                            _ => None,
                        };
                        eprintln!("OWNER13_MEMBER owner={id:?} name={member} span={:?} receiver_span={:?} receiver_kind={:?} local_reference={reference:?} raw_receiver={:?} resolved_receiver={resolved:?} result={result:?} old_object_gate={}",
                            node.span, receiver.span, std::mem::discriminant(&receiver.kind), receiver.ty,
                            crate::hir::owned_objects::object(&resolved).is_some());
                        let mut slot_count = 0;
                        for (kind, site, context, span, dependencies, _) in &sites {
                            if span.file_path == node.span.file_path
                                && node.span.start <= span.start
                                && span.end <= node.span.end
                            {
                                slot_count += 1;
                                let mut dependencies =
                                    dependencies.iter().copied().collect::<Vec<_>>();
                                dependencies.sort();
                                eprintln!("OWNER13_SLOT kind={kind:?} site={site:?} context={context} span={span:?} dependencies={dependencies:?}");
                            }
                        }
                        eprintln!(
                            "OWNER13_SLOT_COUNT owner={id:?} name={member} count={slot_count}"
                        );
                        crate::type_services::visit::visit_type(&resolved, &mut |nested: &Type| {
                            if let Type::TypeVar(variable) = nested {
                                let evidence = hir.engine.get_bounds(*variable);
                                for bound in &evidence {
                                    for argument in &bound.type_args {
                                        ty(hir, argument, types);
                                    }
                                }
                                eprintln!("OWNER13_VARIABLE id={variable:?} kind={:?} bounds={evidence:?}", hir.engine.kind_of_type_var(*variable));
                            }
                        });
                        let application = service
                            .infer_unary_constructor_application(&resolved, transport.trait_id);
                        if let Ok(Some((constructor, payload, state))) = &application {
                            ty(hir, constructor, types);
                            ty(hir, payload, types);
                            for value in state {
                                ty(hir, value, types);
                            }
                        }
                        eprintln!("OWNER13_APPLICATION result={application:?}");
                        for arg in args {
                            eprintln!(
                                "OWNER13_MEMBER_ARG span={:?} type={:?}",
                                arg.span,
                                ty(hir, &arg.ty, types)
                            );
                        }
                    }
                    if name == "main" {
                        if let HirExprKind::Call(callee, args, target) = &node.kind {
                            if let HirExprKind::ResolvedVar(reference) = &callee.kind {
                                if matches!(reference.target, HirVarTarget::Function(callee_id) if helper_ids.contains(&callee_id))
                                {
                                    counts[1] += 1;
                                    assert!(counts[1] <= 32, "source13 printed-site cap exceeded");
                                    eprintln!("OWNER13_CALL owner={id:?} target={target:?} reference={reference:?} span={:?} callee_type={:?} result={:?}",
                                        node.span, ty(hir, &callee.ty, types), ty(hir, &node.ty, types));
                                    for arg in args {
                                        let resolved = ty(hir, &arg.ty, types);
                                        eprintln!("OWNER13_CALL_ARG span={:?} raw_type={:?} resolved_type={resolved:?}", arg.span, arg.ty);
                                        let application = service
                                            .infer_unary_constructor_application(
                                                &resolved,
                                                transport.trait_id,
                                            );
                                        if let Ok(Some((constructor, payload, state))) =
                                            &application
                                        {
                                            ty(hir, constructor, types);
                                            ty(hir, payload, types);
                                            for value in state {
                                                ty(hir, value, types);
                                            }
                                        }
                                        eprintln!(
                                            "OWNER13_CALL_ARG_APPLICATION result={application:?}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let mut implementations = hir.impls.values().filter(|implementation| {
                hir.current_def_ids.contains(&implementation.id)
                    && matches!(implementation.trait_id, Some(id) if id == transport.trait_id || id == unique.trait_id)
            }).collect::<Vec<_>>();
            implementations.sort_by_key(|implementation| implementation.id);
            for implementation in implementations {
                counts[2] += 1;
                assert!(counts[2] <= 32, "source13 impl cap exceeded");
                match &implementation.receiver_pattern {
                    crate::hir::HirImplReceiverPattern::Exact(value)
                    | crate::hir::HirImplReceiverPattern::Constructor(value) => {
                        ty(hir, value, types);
                    }
                    crate::hir::HirImplReceiverPattern::SliceFamily { element } => {
                        ty(hir, element, types);
                    }
                }
                for value in &implementation.trait_arg_types {
                    ty(hir, value, types);
                }
                for bounds in implementation.bounds.values() {
                    for bound in bounds {
                        for value in &bound.type_args {
                            ty(hir, value, types);
                        }
                    }
                }
                for predicate in &implementation.bounds.predicates {
                    let crate::types::Predicate::Trait { subject, args, .. } = predicate;
                    ty(hir, subject, types);
                    for value in args {
                        ty(hir, value, types);
                    }
                }
                eprintln!("OWNER13_IMPL id={:?} trait={:?} receiver={:?} fixed_generics={:?} state_args={:?} bounds={:?}",
                    implementation.id, implementation.trait_id, implementation.receiver_pattern,
                    implementation.type_generics, implementation.trait_arg_types, implementation.bounds);
                let mut methods = implementation.methods.values().collect::<Vec<_>>();
                methods.sort_by_key(|method| method.id);
                for method in methods {
                    signature(hir, method, counts, types);
                }
            }
            eprintln!(
                "OWNER13_CAPS functions={} sites={} impls={} types={}",
                counts[0],
                counts[1],
                counts[2],
                types.len()
            );
        }

        let source_path = PathBuf::from(
            std::env::var_os("ROCK_OWNER_TRACE_SOURCE").expect("explicit source13 path"),
        );
        let artifact = PathBuf::from(
            std::env::var_os("ROCK_OWNER_TRACE_STDLIB").expect("explicit primary7 artifact"),
        );
        let work = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work");
        assert_eq!(
            source_path,
            work.join("recovery/allocator-payload-last-consuming-source-13.rk")
        );
        assert_eq!(artifact, work.join("artifacts/stdlib/stdlib.rkca"));
        for flag in [
            "ROCK_OWNER_LOAN_TRACE",
            "ROCK_OWNER_RETURN_TRACE",
            "ROCK_ALLOCATOR_EXECUTION_DEBUG",
            "ROCK_ALLOCATOR_EXECUTION_MIR",
            "ROCK_STATIC_CAPTURE_TRACE",
            "ROCK_ALLOCATOR_FRONTEND_ONLY",
        ] {
            assert!(
                std::env::var_os(flag).is_none(),
                "unset broad diagnostic flag {flag}"
            );
        }
        let source_bytes = std::fs::read(&source_path).unwrap();
        let artifact_bytes = std::fs::read(&artifact).unwrap();
        assert_eq!(source_bytes.len(), 14110);
        assert_eq!(artifact_bytes.len(), 803478);
        digest(
            &source_path,
            "8554d3c21fa6e3fecf26296d0cc14c3087026a1ebf3012c4f8d90656f4ba6c34",
        );
        digest(
            &artifact,
            "348cdd6baac0f384e885ac665e0820cd206a1e6cde3df52a7a7b5b2755c45b27",
        );
        let source = std::str::from_utf8(&source_bytes).unwrap();
        assert_eq!(&source[9270..9274], "take");
        let config = crate::Config {
            entry_file: source_path.clone(),
            current_crate_name: Some("allocator_state_caller".into()),
            extern_artifacts: vec![("stdlib".into(), artifact.clone())],
            no_std: true,
            no_prelude: true,
            no_link: true,
            ..crate::Config::default()
        };
        let mut context = crate::crate_system::CrateContext::new();
        crate::load_extern_artifacts(
            &mut context,
            &config.extern_artifacts,
            source_path.parent().unwrap().to_path_buf(),
        )
        .unwrap();
        let module = crate::parser::parse_source(source_path.clone(), source, &config).unwrap();
        let program = crate::ast::Program { module };
        let declarations = crate::collect::collect(
            &program,
            &context,
            false,
            config.current_crate_name.as_deref(),
        )
        .unwrap();
        let mut hir = crate::lower::program::lower_from_declarations(
            &program,
            declarations,
            &context,
            config.current_crate_name.as_deref(),
        )
        .unwrap();
        let mut counts = [0; 3];
        let mut types = HashSet::new();
        snapshot(&hir, "lowered", &mut counts, &mut types);
        let result = crate::infer::drive_inference_to_quiescence(&mut hir);
        if let Err(errors) = &result {
            assert!(errors.len() <= 32, "source13 diagnostic cap exceeded");
            for error in errors {
                eprintln!(
                    "OWNER13_DIAGNOSTIC code={:?} span={:?} message={}",
                    error.code,
                    error.span(),
                    error.message
                );
            }
        } else {
            eprintln!("OWNER13_DIAGNOSTIC unexpected quiescence success; no application executed");
        }
        snapshot(
            &hir,
            "after actual quiescence result",
            &mut counts,
            &mut types,
        );
        candidate_operations(&hir, &mut counts, &mut types);
        assert_eq!(
            source_bytes,
            std::fs::read(&source_path).unwrap(),
            "source13 bytes changed"
        );
        assert_eq!(
            artifact_bytes,
            std::fs::read(&artifact).unwrap(),
            "primary7 artifact bytes changed"
        );
        digest(
            &source_path,
            "8554d3c21fa6e3fecf26296d0cc14c3087026a1ebf3012c4f8d90656f4ba6c34",
        );
        digest(
            &artifact,
            "348cdd6baac0f384e885ac665e0820cd206a1e6cde3df52a7a7b5b2755c45b27",
        );
        let errors = result.expect_err(
            "observer must preserve the actual source13 rejection, not accept execution",
        );
        assert!(
            errors.iter().any(|error| {
                error.code == crate::diagnostic::DiagnosticCode::Resolve
                    && error.message == "Unknown field 'take' on struct 'Shell'"
                    && error.span().is_some_and(|span| {
                        span.file_path == source_path && (span.start, span.end) == (9270, 9274)
                    })
            }),
            "actual rejection boundary changed: {errors:?}"
        );
        assert!(
            errors.first().is_some_and(|error| {
                error.code == crate::diagnostic::DiagnosticCode::Resolve
                    && error.message == "Unknown field 'take' on struct 'Shell'"
                    && error.span().is_some_and(|span| {
                        span.file_path == source_path && (span.start, span.end) == (9270, 9274)
                    })
            }),
            "actual first source13 rejection changed: {errors:?}"
        );
    }

    fn analyze_field_authority_source(
        source: &str,
    ) -> Result<crate::analysis::Analysis, crate::diagnostic::Diagnostics> {
        let path = std::path::PathBuf::from("/virtual/field_authority.rk");
        crate::analyze(&crate::Config {
            entry_file: path.clone(),
            no_std: true,
            no_prelude: true,
            source_providers: vec![crate::SourceProvider::Virtual {
                path,
                text: source.into(),
            }],
            ..crate::Config::default()
        })
    }

    #[test]
    fn field_authority_concrete_caller_shares_only_actual_signature_dependencies() {
        let source = "struct Left\n    < value: I64\nstruct Right\n    < value: Bool\nread = owner -> owner.value\nmain = ->\n    owner = Left\n        value: 42\n    read owner\n";
        let config = crate::Config {
            no_std: true,
            no_prelude: true,
            ..crate::Config::default()
        };
        let module = crate::parser::parse_source(
            std::path::PathBuf::from("/virtual/field_authority.rk"),
            source,
            &config,
        )
        .expect("parse actual caller and ambiguous field declaration");
        let partial = crate::lower::Lowerer::with_options(false)
            .lower_program(&crate::ast::Program { module })
            .expect("lower actual field authority source");
        let read = partial
            .functions
            .values()
            .find(|function| function.name == "read")
            .unwrap();
        let mut signature_vars = HashSet::new();
        for ty in [&read.params[0].ty, &read.ret_type] {
            assert!(matches!(partial.engine.resolve(ty), Type::TypeVar(_)));
            collect_type_vars(&partial.engine.resolve(ty), &mut signature_vars);
        }
        let mut sites = Vec::new();
        collect_authority_sites(&read.body, &partial.engine, &mut sites);
        let field = sites
            .iter()
            .find(|(kind, ..)| *kind == AuthorityObligationKind::Field)
            .unwrap();
        assert_eq!(field.4, signature_vars);
        assert_eq!(&source[field.3.start..field.3.end], "value");
        let HirExprKind::FieldAccess(_, _, None) = &field.5.kind else {
            panic!("nominal authority must remain deferred before inference");
        };
        let shared = constrained_signature_vars(&partial, read, &HashSet::new());
        assert_eq!(shared, signature_vars);
        let main = partial
            .functions
            .values()
            .find(|function| function.name == "main")
            .unwrap();
        let HirStmt::Expr(call) = main.body.stmts.last().unwrap() else {
            panic!("expected actual caller tail");
        };
        let HirExprKind::Call(callee, args, Some(HirCallTarget::Function(target))) = &call.kind
        else {
            panic!("expected canonical direct call: {call:?}");
        };
        assert_eq!(*target, read.id);
        assert!(matches!(&callee.kind, HirExprKind::ResolvedVar(reference)
            if reference.target == HirVarTarget::Function(read.id)));
        let Type::Struct { id: left_id, .. } = partial.engine.resolve(&args[0].ty) else {
            panic!("actual caller must supply a nominal Left argument");
        };
        assert_eq!(partial.structs[&left_id].name, "Left");
        assert!(matches!(&args[0].kind, HirExprKind::ResolvedVar(reference)
            if matches!(reference.target, HirVarTarget::Local(_))));
        let Type::Function { params, .. } = partial.engine.resolve(&callee.ty) else {
            panic!("expected fresh callable instance");
        };
        assert_eq!(params, vec![partial.engine.resolve(&args[0].ty)]);

        let analysis = analyze_field_authority_source(source).unwrap_or_else(|errors| {
            panic!("concrete caller must resolve field authority: {errors:?}")
        });
        let read = analysis
            .hir
            .program
            .functions
            .values()
            .find(|function| function.name == "read")
            .unwrap();
        let Type::Struct { id: left_id, args } = &read.params[0].ty else {
            panic!("accepted declaration must have actual nominal authority");
        };
        let left_id = *left_id;
        assert!(args.is_empty());
        assert_eq!(analysis.hir.program.structs[&left_id].name, "Left");
        assert_eq!(
            read.params[0].ty,
            Type::Struct {
                id: left_id,
                args: Vec::new()
            }
        );
        assert_eq!(read.ret_type, Type::I64);
        assert!(read.generic_params.is_empty());
        let crate::hir::HirStmtFor::Expr(expression) = read.body.stmts.last().unwrap() else {
            panic!("expected canonical field tail");
        };
        let crate::hir::HirExprKindFor::FieldAccess(receiver, _, Some(location)) = &expression.kind
        else {
            panic!("expected field, not an inferred method");
        };
        assert_eq!(location.owner, left_id);
        assert_eq!(
            location.field_id,
            analysis.hir.program.structs[&left_id].fields[0].id
        );
        assert_eq!(receiver.ty, read.params[0].ty);
        assert_eq!(expression.ty, Type::I64);
        let main = analysis
            .hir
            .program
            .functions
            .values()
            .find(|function| function.name == "main")
            .unwrap();
        let crate::hir::HirStmtFor::Expr(call) = main.body.stmts.last().unwrap() else {
            panic!("expected accepted caller tail");
        };
        let crate::hir::HirExprKindFor::Call(callee, args, Some(HirCallTarget::Function(target))) =
            &call.kind
        else {
            panic!("expected accepted direct call: {call:?}");
        };
        assert_eq!(*target, read.id);
        assert_eq!(
            callee.ty,
            Type::function(vec![read.params[0].ty.clone()], Type::I64)
        );
        assert_eq!(args[0].ty, read.params[0].ty);
        assert!(
            matches!(&args[0].kind, crate::hir::HirExprKindFor::ResolvedVar(reference)
            if matches!(reference.target, HirVarTarget::Local(_)))
        );
        assert_eq!(call.ty, Type::I64);
    }

    #[test]
    fn field_authority_conflicting_annotation_free_callers_reject_at_actual_argument() {
        let declarations = "struct Left\n    < value: I64\nstruct Right\n    < value: Bool\nread = owner -> owner.value\n";
        for (first, second) in [("left", "right"), ("right", "left")] {
            let first_source = format!("{declarations}main = ->\n    left = Left\n        value: 42\n    right = Right\n        value: true\n    read {first}\n");
            let analysis = analyze_field_authority_source(&first_source).unwrap_or_else(|errors| {
                panic!("first caller must supply actual canonical authority: {errors:?}")
            });
            let read = analysis
                .hir
                .program
                .functions
                .values()
                .find(|function| function.name == "read")
                .unwrap();
            let Type::Struct { id, .. } = &read.params[0].ty else {
                panic!("first caller must identify the declaration receiver");
            };
            assert_eq!(
                analysis.hir.program.structs[id].name,
                if first == "left" { "Left" } else { "Right" }
            );
            assert_eq!(
                read.ret_type,
                if first == "left" {
                    Type::I64
                } else {
                    Type::Bool
                }
            );
            let crate::hir::HirStmtFor::Expr(expression) = read.body.stmts.last().unwrap() else {
                panic!("expected first caller's canonical field tail");
            };
            let crate::hir::HirExprKindFor::FieldAccess(_, _, Some(location)) = &expression.kind
            else {
                panic!("expected canonical field before introducing conflict");
            };
            assert_eq!(location.owner, *id);
            assert_eq!(
                location.field_id,
                analysis.hir.program.structs[id].fields[0].id
            );
            let source = format!("{first_source}    read {second}\n");
            let errors = analyze_field_authority_source(&source).expect_err(
                "structural field inference cannot authorize conflicting nominal callers",
            );
            let argument_start = source.rfind(&format!("read {second}")).unwrap() + "read ".len();
            let error = errors
                .0
                .iter()
                .find(|error| {
                    matches!(
                        error.code,
                        Some(
                            crate::diagnostic::DiagnosticCode::Type
                                | crate::diagnostic::DiagnosticCode::Selection
                        )
                    ) && error.message.contains("type mismatch")
                        && error.message.contains("Left")
                        && error.message.contains("Right")
                        && error.primary.as_ref().is_some_and(|primary| {
                            primary.span.file_path
                                == std::path::PathBuf::from("/virtual/field_authority.rk")
                                && primary.span.start == argument_start
                                && primary.span.end == argument_start + second.len()
                        })
                })
                .unwrap_or_else(|| {
                    panic!(
                        "expected nominal caller mismatch at actual {second} argument: {errors:?}"
                    )
                });
            let span = &error.primary.as_ref().unwrap().span;
            assert_eq!(&source[span.start..span.end], second);
            assert!(
                errors
                    .0
                    .iter()
                    .all(|error| error.code != Some(crate::diagnostic::DiagnosticCode::Internal)),
                "{errors:?}"
            );
        }
    }

    #[test]
    fn declared_sized_evidence_is_identity_scoped_and_respects_relaxation() {
        use crate::type_services::kind::Kind;
        use crate::types::GenericParamDecl;

        let owner = DefId::new(CrateId(0), LocalDefId(700));
        let parameter = |index| GenericParamId { owner, index };
        let sized_id = DefId::new(CrateId(0), LocalDefId(701));
        let ordinary = GenericParamDecl::type_param(parameter(0), "A");
        let mut relaxed_declaration = GenericParamDecl::type_param(parameter(1), "B");
        relaxed_declaration.maybe_unsized = true;
        let relaxed_bound = GenericParamDecl::type_param(parameter(2), "C");
        let constructor =
            GenericParamDecl::new(parameter(3), "F", Kind::arrow(Kind::Type, Kind::Type));
        let mut bounds = HirGenericBounds::new();
        bounds.relaxed_sized.insert(relaxed_bound.id);
        let parameters = [ordinary, relaxed_declaration, relaxed_bound, constructor];
        add_declared_sized_bounds(&mut bounds, &parameters, Some(sized_id));
        add_declared_sized_bounds(&mut bounds, &parameters, Some(sized_id));
        assert_eq!(bounds[&parameter(0)].len(), 1);
        let sized = TraitBound {
            trait_id: sized_id,
            type_args: Vec::new(),
        };
        let traits = HashMap::from([(
            sized_id,
            crate::hir::HirTrait {
                id: sized_id,
                name: "LayoutKnown".into(),
                generic_params: Vec::new(),
                target: None,
                predicates: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        )]);
        let impls = HashMap::new();
        let service = SelectionService::new(&traits, &impls, Some(sized_id), None, &bounds);
        assert!(service.trait_bound_satisfied(&Type::Generic(parameter(0)), &sized));
        for index in 1..5 {
            // Index 4 is not declared here: no blanket Sized inference.
            assert!(!service.trait_bound_satisfied(&Type::Generic(parameter(index)), &sized));
        }
    }

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
    fn dereference_input_coercion_waits_for_quiescence_and_uses_actual_handle() {
        for mutable in [false, true] {
            let caller = DefId::new(CrateId(0), LocalDefId(750));
            let producer = DefId::new(CrateId(0), LocalDefId(751));
            let mut hir = hir(HashMap::new());
            let target = hir.engine.fresh_type_var();
            let output = hir.engine.fresh_type_var();
            let unrelated = hir.engine.fresh_type_var();
            let Type::TypeVar(target_id) = target else {
                unreachable!()
            };
            let span = crate::lexer::Span::new("/virtual/deref_coercion.rk".into(), 35, 43);
            let input_span = crate::lexer::Span::new("/virtual/deref_coercion.rk".into(), 72, 73);
            let actual = Type::Reference {
                mutable,
                inner: Box::new(Type::I64),
            };
            let mut body = function(caller, target_id);
            body.body.stmts.push(HirStmt::Expr(HirExpr {
                ty: target.clone(),
                span: span.clone(),
                kind: HirExprKind::ObjectCoercion(
                    Box::new(HirExpr {
                        ty: actual.clone(),
                        span: span.clone(),
                        kind: HirExprKind::Ref(
                            mutable,
                            Box::new(HirExpr {
                                ty: Type::I64,
                                span: span.clone(),
                                kind: HirExprKind::ResolvedVar(HirVarRef {
                                    name: "audit".into(),
                                    target: HirVarTarget::Local(HirLocalId(1)),
                                }),
                            }),
                        ),
                    }),
                    crate::hir::HirObjectCoercion {
                        target: target.clone(),
                        evidence: None,
                    },
                ),
            }));
            hir.functions.insert(caller, body);
            let owners = HashSet::from([ConstraintOwner::Body(caller)]);
            let mut obligations = authority_obligations(&hir, &owners);
            let obligation = obligations
                .iter_mut()
                .find(|obligation| obligation.kind == AuthorityObligationKind::ObjectCoercion)
                .unwrap();
            assert!(!is_dereference_input_coercion(&hir, obligation));
            hir.constraint_store
                .replace_owner(ConstraintOwner::Body(producer));
            // Being a result dependency does not justify treating this target
            // as a source input, nor may unrelated source slots gain a shape.
            hir.constraint_store.add_dereference(
                unrelated.clone(),
                target.clone(),
                true,
                input_span.clone(),
            );
            assert!(!is_dereference_input_coercion(&hir, obligation));
            let relation = hir.constraint_store.add_dereference(
                target.clone(),
                output.clone(),
                true,
                input_span.clone(),
            );
            assert!(is_dereference_input_coercion(&hir, obligation));
            let pending = run_authority_obligation(&mut hir, obligation, false, false).unwrap();
            assert!(pending.pending);
            assert_eq!(hir.engine.resolve(&target), target);
            assert_eq!(hir.engine.resolve(&output), output);
            obligation.wake();
            let completed = run_authority_obligation(&mut hir, obligation, true, false).unwrap();
            assert!(!completed.pending);
            assert_eq!(hir.engine.resolve(&target), actual);
            assert_eq!(hir.engine.resolve(&unrelated), unrelated);
            assert_eq!(
                hir.constraint_store.state(relation),
                Some(ObligationState::Pending)
            );
            assert_eq!(
                hir.constraint_store.owner(relation),
                Some(ConstraintOwner::Body(producer))
            );
            assert!(hir.constraint_store.iter().any(|(id, constraint)| {
                id == relation && matches!(constraint, Constraint::Dereference { span, .. } if *span == input_span)
            }));
            assert!(!is_dereference_input_coercion(&hir, obligation));
            let HirStmt::Expr(argument) = &hir.functions[&caller].body.stmts[0] else {
                unreachable!()
            };
            assert!(matches!(argument.kind, HirExprKind::Ref(access, _) if access == mutable));
            assert_eq!(argument.ty, actual);
            assert_eq!(argument.span, span);
        }
    }

    #[test]
    fn generated_locals_are_reserved_above_patterns_loops_and_captures() {
        let function_id = DefId::new(CrateId(0), LocalDefId(1));
        let expression = |kind| HirExpr {
            kind,
            ty: Type::Unit,
            span: crate::lexer::Span::test(),
        };
        let binding = |id| HirPattern::Binding {
            name: format!("binding{id}"),
            local_id: HirLocalId(id),
            mutable: false,
        };
        let block = |expr| HirBlock {
            stmts: vec![HirStmt::Expr(expr)],
            ty: Type::Unit,
        };
        let cases = vec![
            (
                expression(HirExprKind::Match {
                    scrutinee: Box::new(expression(HirExprKind::Unit)),
                    arms: vec![crate::hir::HirMatchArm {
                        pattern: HirPattern::Or(vec![
                            HirPattern::Tuple(vec![binding(10)]),
                            HirPattern::Struct(
                                "Record".into(),
                                None,
                                vec![],
                                vec![crate::hir::HirStructPatternField {
                                    name: "field".into(),
                                    field: None,
                                    pattern: HirPattern::Enum(
                                        "Choice".into(),
                                        "Value".into(),
                                        None,
                                        vec![binding(30)],
                                    ),
                                }],
                            ),
                        ]),
                        guard: None,
                        body: block(expression(HirExprKind::Unit)),
                    }],
                }),
                30,
            ),
            (
                expression(HirExprKind::Match {
                    scrutinee: Box::new(expression(HirExprKind::Unit)),
                    arms: vec![crate::hir::HirMatchArm {
                        pattern: HirPattern::Tuple(vec![binding(35)]),
                        guard: None,
                        body: block(expression(HirExprKind::Unit)),
                    }],
                }),
                35,
            ),
            (
                expression(HirExprKind::Match {
                    scrutinee: Box::new(expression(HirExprKind::Unit)),
                    arms: vec![crate::hir::HirMatchArm {
                        pattern: HirPattern::Wildcard,
                        guard: Some(expression(HirExprKind::Match {
                            scrutinee: Box::new(expression(HirExprKind::Unit)),
                            arms: vec![crate::hir::HirMatchArm {
                                pattern: binding(40),
                                guard: None,
                                body: block(expression(HirExprKind::Unit)),
                            }],
                        })),
                        body: block(expression(HirExprKind::Unit)),
                    }],
                }),
                40,
            ),
            (
                expression(HirExprKind::For {
                    var: "element".into(),
                    local_id: HirLocalId(50),
                    iter: Box::new(expression(HirExprKind::Unit)),
                    body: block(expression(HirExprKind::Unit)),
                }),
                50,
            ),
            (
                expression(HirExprKind::Lambda {
                    params: vec![],
                    captures: vec![HirClosureCapture {
                        name: "capture".into(),
                        local_id: HirLocalId(60),
                        kind: HirClosureCaptureKind::Move,
                        mutable: false,
                        ty: Type::I64,
                    }],
                    body: block(expression(HirExprKind::Unit)),
                }),
                60,
            ),
            (
                expression(HirExprKind::ResolvedVar(HirVarRef {
                    name: "local".into(),
                    target: HirVarTarget::Local(HirLocalId(70)),
                })),
                70,
            ),
        ];
        let mut hir = hir(HashMap::new());
        let mut context = MethodAuthorityContext {
            language_items: hir.language_items.clone(),
            functions: HashMap::new(),
            traits: &hir.traits,
            impls: &hir.impls,
            sized_trait_id: None,
            bounds: HirGenericBounds::new(),
            function_owner: ConstraintOwner::Global,
            constraint_store: &mut hir.constraint_store,
            effective_trait_methods: &hir.imported_effective_trait_methods,
            resolver: &hir.resolver,
            structs: &hir.structs,
            enums: &hir.enums,
            engine: RefCell::new(&mut hir.engine),
            try_protocol: None,
            unsafe_context: Cell::new(false),
            mutable_locals: HashSet::new(),
            local_bindings: HashMap::new(),
            pending_coercion_dependencies: HashSet::new(),
            opened_witnesses: Vec::new(),
            opened_packages: Vec::new(),
            loop_targets: Vec::new(),
            next_local_id: 0,
            ambiguous: false,
            selected_site: None,
            strict: false,
            try_strict: false,
        };
        for (expr, highest) in cases {
            let mut function = function(function_id, crate::ids::TypeVarId(0));
            function.body = block(expr);
            context.next_local_id = 0;
            context.local_bindings.clear();
            context.collect_local_bindings(&function);
            assert_eq!(context.fresh_local_id(), HirLocalId(highest + 1));
            assert_eq!(context.fresh_local_id(), HirLocalId(highest + 2));
            assert_eq!(
                context.local_bindings.len(),
                1,
                "reservation must not invent binding types"
            );
        }
    }

    #[test]
    fn declared_receiver_predicate_precedes_concrete_caller_seeding() {
        let id = |local| DefId::new(CrateId(0), LocalDefId(local));
        for member in ["key", "missing"] {
            let mut hir = hir(HashMap::new());
            let Type::TypeVar(source) = hir.engine.fresh_type_var() else {
                unreachable!()
            };
            let Type::TypeVar(result) = hir.engine.fresh_type_var() else {
                unreachable!()
            };
            let source_ty = Type::TypeVar(source);
            let bound = TraitBound {
                trait_id: id(710),
                type_args: Vec::new(),
            };
            let mut body_span = crate::lexer::Span::test();
            body_span.start = 10;
            body_span.end = 20;
            let mut caller_span = body_span.clone();
            caller_span.start = 30;
            caller_span.end = 40;
            let receiver = HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "value".into(),
                    target: HirVarTarget::Local(HirLocalId(0)),
                }),
                ty: source_ty.clone(),
                span: body_span.clone(),
            };
            let mut producer = function(id(711), source);
            producer.ret_type = Type::TypeVar(result);
            producer.body = HirBlock {
                ty: Type::TypeVar(result),
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKind::Call(
                        Box::new(HirExpr {
                            kind: HirExprKind::FieldAccess(Box::new(receiver), member.into(), None),
                            ty: Type::function(Vec::new(), Type::TypeVar(result)),
                            span: body_span.clone(),
                        }),
                        Vec::new(),
                        None,
                    ),
                    ty: Type::TypeVar(result),
                    span: body_span.clone(),
                })],
            };
            hir.traits.insert(
                bound.trait_id,
                crate::hir::HirTrait {
                    id: bound.trait_id,
                    name: "Memory".into(),
                    generic_params: Vec::new(),
                    target: None,
                    predicates: Vec::new(),
                    associated_types: Vec::new(),
                    methods: HashMap::new(),
                    signatures: HashMap::from([(
                        "key".into(),
                        crate::hir::HirFunctionSig {
                            id: id(712),
                            name: "key".into(),
                            generic_params: Vec::new(),
                            params: vec![Type::Reference {
                                mutable: false,
                                inner: Box::new(Type::Generic(GenericParamId {
                                    owner: id(710),
                                    index: 0,
                                })),
                            }],
                            ret: Type::I64,
                            generic_bounds: HirGenericBounds::new(),
                            self_receiver: Some(crate::types::ReceiverMode::Shared),
                            is_unsafe: false,
                        },
                    )]),
                },
            );
            let parameter = GenericParamId {
                owner: id(713),
                index: 0,
            };
            hir.impls.insert(
                id(713),
                crate::hir::HirImpl {
                    id: id(713),
                    owner: crate::hir::HirImplOwner::Named("Shell".into()),
                    type_name: "Shell".into(),
                    type_generics: vec![crate::types::GenericParamDecl::type_param(parameter, "A")],
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: id(714),
                        args: vec![Type::Generic(parameter)],
                    }),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: HashMap::from([(parameter, vec![bound.clone()])]).into(),
                    methods: HashMap::from([("new_aligned".into(), producer)]),
                },
            );
            let invalid = Type::Struct {
                id: id(715),
                args: Vec::new(),
            };
            let mut caller = function(id(716), source);
            caller.params.clear();
            caller.ret_type = Type::I64;
            caller.body = HirBlock {
                ty: Type::I64,
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKind::Call(
                        Box::new(HirExpr {
                            kind: HirExprKind::ResolvedVar(HirVarRef {
                                name: "Shell::new_aligned".into(),
                                target: HirVarTarget::Function(id(711)),
                            }),
                            ty: Type::function(vec![invalid.clone()], Type::I64),
                            span: caller_span.clone(),
                        }),
                        vec![HirExpr {
                            kind: HirExprKind::Unit,
                            ty: invalid.clone(),
                            span: caller_span.clone(),
                        }],
                        Some(HirCallTarget::Function(id(711))),
                    ),
                    ty: Type::I64,
                    span: caller_span.clone(),
                })],
            };
            hir.functions.insert(caller.id, caller);
            let mut prepared = HashSet::new();
            materialize_pending(&mut hir, false, false, None, None, Some(&mut prepared)).unwrap();
            let producer = &hir.impls[&id(713)].methods["new_aligned"];
            let shared = constrained_signature_vars(&hir, producer, &prepared);
            if member == "missing" {
                assert!(
                    prepared.is_empty(),
                    "no blanket exemption for a field-like call"
                );
                assert!(shared.contains(&source));
                assert!(hir.engine.get_bounds(source).is_empty());
                assert_eq!(hir.constraint_store.iter().count(), 0);
                continue;
            }
            assert_eq!(
                prepared,
                HashSet::from([AuthoritySiteId::new(
                    AuthorityObligationKind::DeferredCall,
                    &body_span,
                )])
            );
            assert!(!shared.contains(&source));
            let caller_owner = ConstraintOwner::Body(id(716));
            let owners = HashSet::from([caller_owner]);
            for _ in 0..2 {
                propagate_function_instances_for_owners_with_progress(&mut hir, Some(&owners))
                    .unwrap();
            }
            assert_eq!(hir.engine.resolve(&source_ty), source_ty);
            assert_eq!(hir.engine.get_bounds(source), vec![bound.clone()]);
            let constraints = hir.constraint_store.iter().collect::<Vec<_>>();
            assert_eq!(constraints.len(), 2);
            for (owner, subject, span) in [
                (ConstraintOwner::Body(id(711)), source_ty, body_span),
                (caller_owner, invalid, caller_span),
            ] {
                assert!(constraints.iter().any(|(obligation, constraint)| {
                    hir.constraint_store.owner(*obligation) == Some(owner)
                        && matches!(constraint, Constraint::Trait { ty, bound: actual, span: actual_span, .. }
                            if hir.engine.resolve(ty) == subject && *actual == bound && *actual_span == span)
                }), "canonical subject, body owner and original call span must agree: {constraints:?}");
            }
            let producer_owners = HashSet::from([ConstraintOwner::Body(id(711))]);
            materialize_pending(&mut hir, false, false, Some(&producer_owners), None, None)
                .unwrap();
            let producer = &hir.impls[&id(713)].methods["new_aligned"];
            let HirStmt::Expr(expression) = &producer.body.stmts[0] else {
                unreachable!()
            };
            let HirExprKind::MethodCall(receiver, _, _, mode, Some(target)) = &expression.kind
            else {
                panic!("producer must select its bound member, not Invalid's field: {expression:?}")
            };
            assert_eq!(receiver.ty, Type::TypeVar(source));
            assert_eq!(*mode, Some(crate::types::ReceiverMode::Shared));
            assert_eq!(target.trait_id(), Some(id(710)));
            assert_eq!(target.method_id(), Some(id(712)));
            assert_eq!(target.impl_id(), None);
            assert_eq!(
                target.target,
                crate::hir::HirSelectedMethodTarget::TraitMethod {
                    trait_id: id(710),
                    member_id: id(712),
                    trait_args: Vec::new(),
                    dispatch: crate::hir::HirTraitDispatchKind::TraitBound,
                }
            );
        }
    }

    #[test]
    fn late_instance_bounds_preserve_source_scheme_and_caller_ownership() {
        let mut hir = hir(HashMap::new());
        let Type::TypeVar(source) = hir.engine.fresh_type_var() else {
            unreachable!()
        };
        let Type::TypeVar(output) = hir.engine.fresh_type_var() else {
            unreachable!()
        };
        let Type::TypeVar(left) = hir.engine.fresh_type_var() else {
            unreachable!()
        };
        let Type::TypeVar(right) = hir.engine.fresh_type_var() else {
            unreachable!()
        };
        let trait_id = DefId::new(CrateId(0), LocalDefId(702));
        hir.engine.add_bound(
            source,
            TraitBound {
                trait_id,
                type_args: vec![Type::TypeVar(output)],
            },
        );
        let span = crate::lexer::Span::test();
        let mut errors = Vec::new();
        for (caller, instance, result) in [(703, left, Type::I64), (704, right, Type::U8)] {
            let owner = ConstraintOwner::Body(DefId::new(CrateId(0), LocalDefId(caller)));
            let previous = hir.constraint_store.replace_owner(owner);
            let mut bindings =
                HashMap::from([(source, Type::TypeVar(instance)), (output, result.clone())]);
            propagate_instance_bounds(&mut hir, &mut bindings, &HashMap::new(), &span, &mut errors);
            propagate_instance_bounds(&mut hir, &mut bindings, &HashMap::new(), &span, &mut errors);
            hir.constraint_store.replace_owner(previous);
            assert_eq!(
                hir.engine.get_bounds(instance),
                vec![TraitBound {
                    trait_id,
                    type_args: vec![result]
                }]
            );
        }
        assert!(errors.is_empty(), "{errors:?}");
        let constraints = hir.constraint_store.iter().collect::<Vec<_>>();
        assert_eq!(
            constraints.len(),
            2,
            "revisits must not keep growing obligations"
        );
        assert_eq!(
            hir.constraint_store.owner(constraints[0].0),
            Some(ConstraintOwner::Body(DefId::new(
                CrateId(0),
                LocalDefId(703)
            )))
        );
        assert_eq!(
            hir.constraint_store.owner(constraints[1].0),
            Some(ConstraintOwner::Body(DefId::new(
                CrateId(0),
                LocalDefId(704)
            )))
        );
        assert_eq!(
            hir.engine.resolve(&Type::TypeVar(source)),
            Type::TypeVar(source)
        );
        assert_eq!(
            hir.engine.resolve(&Type::TypeVar(output)),
            Type::TypeVar(output)
        );
    }

    #[test]
    fn late_bound_only_variables_are_freshened_once_per_call() {
        let mut hir = hir(HashMap::new());
        let Type::TypeVar(source) = hir.engine.fresh_type_var() else {
            unreachable!()
        };
        let Type::TypeVar(output) = hir.engine.fresh_type_var() else {
            unreachable!()
        };
        let Type::TypeVar(instance) = hir.engine.fresh_type_var() else {
            unreachable!()
        };
        let trait_id = DefId::new(CrateId(0), LocalDefId(705));
        hir.engine.add_bound(
            source,
            TraitBound {
                trait_id,
                type_args: vec![Type::TypeVar(output)],
            },
        );
        let span = crate::lexer::Span::test();
        let mut errors = Vec::new();
        let mut first = HashMap::from([(source, Type::TypeVar(instance))]);
        propagate_instance_bounds(&mut hir, &mut first, &HashMap::new(), &span, &mut errors);
        let fresh_output = first[&output].clone();
        assert_ne!(fresh_output, Type::TypeVar(output));
        let mut revisit = HashMap::from([(source, Type::TypeVar(instance))]);
        propagate_instance_bounds(&mut hir, &mut revisit, &HashMap::new(), &span, &mut errors);
        assert_eq!(revisit[&output], fresh_output);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(hir.constraint_store.iter().count(), 1);
        assert_eq!(
            hir.engine.resolve(&Type::TypeVar(source)),
            Type::TypeVar(source)
        );
        assert_eq!(
            hir.engine.resolve(&Type::TypeVar(output)),
            Type::TypeVar(output)
        );
    }

    #[test]
    fn an_unresolved_trait_predicate_does_not_fix_a_declaration_to_its_caller() {
        let mut hir = hir(HashMap::new());
        let Type::TypeVar(variable) = hir.engine.fresh_type_var() else {
            unreachable!()
        };
        let id = DefId::new(CrateId(0), LocalDefId(706));
        let function = function(id, variable);
        hir.constraint_store
            .replace_owner(ConstraintOwner::Body(id));
        hir.constraint_store.add_trait(
            Type::TypeVar(variable),
            TraitBound {
                trait_id: DefId::new(CrateId(0), LocalDefId(707)),
                type_args: Vec::new(),
            },
            crate::lexer::Span::test(),
            "ordinary inferred protocol",
        );
        assert!(!constrained_signature_vars(&hir, &function, &HashSet::new()).contains(&variable));
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
