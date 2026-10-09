//! Exact retained-callable diagnostic locations; never borrow or effect authority.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{CompilerProducts, ProductDefId, ProductIdRemap};
use crate::hir::{
    AcceptedHirBlock, AcceptedHirExpr, AcceptedHirFunction, HirExprKindFor, HirPattern, HirStmtFor,
};
use crate::ids::HirLocalId;
use crate::lexer::Span;
use crate::source_loader::SourceDatabase;
use crate::source_map::SemanticSourceMap;

#[cfg(test)]
#[path = "callable_source_tests.rs"]
mod tests;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProductCallableSources {
    pub callables: BTreeMap<ProductDefId, CallableSource>,
    pub files: BTreeMap<PathBuf, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallableSource {
    pub definition: Option<Span>,
    /// Exact declared binders, including parameters and nested body locals.
    pub locals: BTreeMap<HirLocalId, Span>,
}

fn declared_locals(function: &AcceptedHirFunction) -> BTreeSet<HirLocalId> {
    fn pattern(value: &HirPattern, locals: &mut BTreeSet<HirLocalId>) {
        match value {
            HirPattern::Binding { local_id, .. } => {
                locals.insert(*local_id);
            }
            HirPattern::Tuple(fields)
            | HirPattern::Enum(_, _, _, fields)
            | HirPattern::Or(fields) => {
                for field in fields {
                    pattern(field, locals);
                }
            }
            HirPattern::Struct(_, _, _, fields) => {
                for field in fields {
                    pattern(&field.pattern, locals);
                }
            }
            HirPattern::Wildcard | HirPattern::Literal(_) => {}
        }
    }
    fn block<'a>(
        body: &'a AcceptedHirBlock,
        locals: &mut BTreeSet<HirLocalId>,
        pending: &mut Vec<&'a AcceptedHirExpr>,
    ) {
        for statement in &body.stmts {
            match statement {
                HirStmtFor::Let {
                    local_id, value, ..
                } => {
                    locals.insert(*local_id);
                    pending.push(value);
                }
                HirStmtFor::Expr(value)
                | HirStmtFor::Return(Some(value))
                | HirStmtFor::Break(Some(value)) => pending.push(value),
                HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => {}
            }
        }
    }
    let mut locals = function
        .params
        .iter()
        .map(|parameter| parameter.local_id)
        .collect();
    let mut pending = Vec::new();
    block(&function.body, &mut locals, &mut pending);
    while let Some(expression) = pending.pop() {
        use HirExprKindFor as E;
        match &expression.kind {
            E::PackagePack { payload, .. } => pending.push(payload),
            E::Open {
                source,
                binding,
                body,
            } => {
                locals.insert(binding.value.local_id);
                pending.push(source);
                block(body, &mut locals, &mut pending);
            }
            E::PackageOpen {
                source,
                binding,
                body,
            } => {
                locals.insert(binding.value.local_id);
                pending.push(source);
                block(body, &mut locals, &mut pending);
            }
            E::Call(callee, args, _)
            | E::MethodCall(callee, _, args, _, _)
            | E::OwnedObjectCall {
                owner: callee,
                args,
                ..
            } => {
                pending.push(callee);
                pending.extend(args);
            }
            E::ArrayLiteral(values)
            | E::TupleLiteral(values)
            | E::EnumVariant(_, _, values, _)
            | E::Intrinsic { args: values, .. } => pending.extend(values),
            E::ArrayRepeat(value, _)
            | E::FieldAccess(value, _, _)
            | E::TupleIndex(value, _)
            | E::UnaryOp(_, value)
            | E::Try { expr: value, .. }
            | E::Ref(_, value)
            | E::Deref(value)
            | E::Cast(value, _)
            | E::ObjectCoercion(value, _) => pending.push(value),
            E::BinOp(_, left, right) | E::Assign(left, right) => {
                pending.push(left);
                pending.push(right);
            }
            E::StructLiteral(_, _, fields) => {
                pending.extend(fields.iter().map(|field| &field.value))
            }
            E::If {
                condition,
                then_branch,
                else_branch,
            } => {
                pending.push(condition);
                block(then_branch, &mut locals, &mut pending);
                if let Some(body) = else_branch {
                    block(body, &mut locals, &mut pending);
                }
            }
            E::Match { scrutinee, arms } => {
                pending.push(scrutinee);
                for arm in arms {
                    pattern(&arm.pattern, &mut locals);
                    if let Some(guard) = &arm.guard {
                        pending.push(guard);
                    }
                    block(&arm.body, &mut locals, &mut pending);
                }
            }
            E::While { condition, body } => {
                pending.push(condition);
                block(body, &mut locals, &mut pending);
            }
            E::For {
                local_id,
                iter,
                body,
                ..
            } => {
                locals.insert(*local_id);
                pending.push(iter);
                block(body, &mut locals, &mut pending);
            }
            E::Lambda { params, body, .. } => {
                locals.extend(params.iter().map(|parameter| parameter.local_id));
                block(body, &mut locals, &mut pending);
            }
            E::Loop(body) | E::Block(body) | E::UnsafeBlock(body) => {
                block(body, &mut locals, &mut pending)
            }
            E::IntLiteral(_)
            | E::FloatLiteral(_)
            | E::BoolLiteral(_)
            | E::StringLiteral(_)
            | E::CharLiteral(_)
            | E::Unit
            | E::Var(_)
            | E::ResolvedVar(_) => {}
        }
    }
    locals
}

fn retained(products: &CompilerProducts) -> impl Iterator<Item = &AcceptedHirFunction> {
    products
        .bodies
        .functions
        .values()
        .chain(products.bodies.trait_default_methods.values())
        .chain(
            products
                .bodies
                .generic_impls
                .values()
                .flat_map(|implementation| implementation.methods.values()),
        )
}

pub(crate) fn validate(products: &CompilerProducts) -> Result<(), String> {
    let sources = &products.bodies.callable_sources;
    let bodies = retained(products)
        .map(|body| (ProductDefId::from(body.id), body))
        .collect::<BTreeMap<_, _>>();
    let mut referenced = BTreeSet::new();
    if sources.callables.len() > super::MAX_PRODUCT_ARTIFACT_DECLARATIONS
        || sources.files.len() > super::MAX_PRODUCT_ARTIFACT_DECLARATIONS
    {
        return Err("callable diagnostic declaration limit exceeded".into());
    }
    for (id, row) in &sources.callables {
        let body = bodies
            .get(id)
            .ok_or("callable diagnostic owner has no retained body")?;
        let declared = declared_locals(body);
        if row.locals.len() > super::MAX_PRODUCT_ARTIFACT_DECLARATIONS
            || row.locals.keys().any(|id| !declared.contains(id))
        {
            return Err("callable diagnostic local is not declared by its owner".into());
        }
        for span in row.definition.iter().chain(row.locals.values()) {
            let text = sources
                .files
                .get(&span.file_path)
                .ok_or("callable diagnostic source text missing")?;
            if span.start > span.end
                || span.end > text.len()
                || !text.is_char_boundary(span.start)
                || !text.is_char_boundary(span.end)
            {
                return Err("callable diagnostic span is outside its UTF-8 source".into());
            }
            referenced.insert(&span.file_path);
        }
    }
    if sources.files.keys().any(|file| !referenced.contains(file)) {
        return Err("callable diagnostic source file has no retained location".into());
    }
    Ok(())
}

impl CompilerProducts {
    pub(crate) fn attach_callable_sources(
        &mut self,
        source_map: &SemanticSourceMap,
        database: &SourceDatabase,
        remap: &ProductIdRemap,
    ) -> Result<(), String> {
        let bodies = retained(self)
            .map(|function| (ProductDefId::from(function.id), function))
            .collect::<BTreeMap<_, _>>();
        let mut sources = ProductCallableSources::default();
        for original in remap.keys() {
            let Some(product) = super::unambiguous_product_id(*original, remap)
                .filter(|id| bodies.contains_key(id))
            else {
                continue;
            };
            let original = super::product_def_id_to_def_id(*original);
            let body = bodies
                .get(&product)
                .ok_or("retained source owner missing")?;
            let row = CallableSource {
                definition: source_map.definition_span(original).cloned(),
                locals: declared_locals(body)
                    .into_iter()
                    .filter_map(|local| {
                        source_map
                            .local_span(original, local)
                            .cloned()
                            .map(|span| (local, span))
                    })
                    .collect(),
            };
            if row.definition.is_none() && row.locals.is_empty() {
                continue;
            }
            for span in row.definition.iter().chain(row.locals.values()) {
                let file = database
                    .source_files()
                    .find(|file| {
                        file.original_path == span.file_path
                            || file.canonical_path == span.file_path
                            || file.display_path == span.file_path
                    })
                    .ok_or("registered callable diagnostic source text unavailable")?;
                if let Some(old) = sources
                    .files
                    .insert(span.file_path.clone(), file.text.clone())
                {
                    if old != file.text {
                        return Err("conflicting callable diagnostic source text".into());
                    }
                }
            }
            if sources.callables.insert(product, row).is_some() {
                return Err("ambiguous callable diagnostic owner".into());
            }
        }
        self.bodies.callable_sources = sources;
        validate(self)
    }
}
