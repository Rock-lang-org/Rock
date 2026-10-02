use std::path::Path;

use crate::ast::Program;
use crate::hir::{
    AcceptedHirBlock, AcceptedHirExpr, AcceptedHirFunction, HirCallTarget, HirExprKindFor,
    HirMethodLocation, HirSelectedMethodTarget, HirStmtFor, HirVarTarget, HirVariantFields,
};
use crate::ids::{AssocTypeId, DefId, FieldId, HirLocalId, VariantId};
use crate::infer::ResolvedHirProgram;
use crate::lexer::Span;
use crate::source_map::SourceSymbol;
use crate::types::{GenericParamDecl, GenericParamId, Type};

#[derive(Debug)]
pub struct Analysis {
    pub ast: Program,
    pub hir: ResolvedHirProgram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverKind {
    Function,
    Variable,
    Type,
    Field,
    Variant,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverInfo {
    pub span: Span,
    pub contents: String,
    pub kind: HoverKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureInfo {
    pub label: String,
    pub parameters: Vec<String>,
    pub active_parameter: usize,
}

#[derive(Debug, Clone)]
struct FunctionSignature {
    label: String,
    parameters: Vec<String>,
}

impl Analysis {
    pub fn hover(&self, path: &Path, offset: usize) -> Option<HoverInfo> {
        if let Some((symbol, span)) = self.hir.source_map.symbol_at(path, offset) {
            let hover = match symbol {
                SourceSymbol::Definition(id) => {
                    self.definition_hover(id).map(|(contents, kind)| HoverInfo {
                        span,
                        contents,
                        kind,
                    })
                }
                SourceSymbol::Local { owner, local } => {
                    self.local_binding(owner, local)
                        .map(|(name, ty)| HoverInfo {
                            span,
                            contents: format!("{}: {}", name, self.hover_type(&ty)),
                            kind: HoverKind::Variable,
                        })
                }
                SourceSymbol::Field { owner, field } => {
                    self.field_hover(owner, field).map(|contents| HoverInfo {
                        span,
                        contents,
                        kind: HoverKind::Field,
                    })
                }
                SourceSymbol::Variant { owner, variant } => {
                    self.variant_hover(owner, variant)
                        .map(|contents| HoverInfo {
                            span,
                            contents,
                            kind: HoverKind::Variant,
                        })
                }
                SourceSymbol::AssociatedType { owner, associated } => self
                    .associated_type_hover(owner, associated)
                    .map(|contents| HoverInfo {
                        span,
                        contents,
                        kind: HoverKind::Type,
                    }),
                SourceSymbol::Generic(id) => self.generic_hover(id).map(|contents| HoverInfo {
                    span,
                    contents,
                    kind: HoverKind::Generic,
                }),
            };
            if hover.is_some() {
                return hover;
            }
        }

        self.expression_hover(path, offset)
    }

    pub fn signature(&self, path: &Path, offset: usize) -> Option<SignatureInfo> {
        let mut best = None;
        self.for_each_function(|function| {
            find_signature_in_block(self, &function.body, path, offset, &mut best)
        });
        best.map(|(_, signature)| signature)
    }

    fn expression_hover(&self, path: &Path, offset: usize) -> Option<HoverInfo> {
        let mut best = None;
        self.for_each_function(|function| {
            find_expression_hover_in_block(self, &function.body, path, offset, &mut best)
        });
        best.map(|(_, hover)| hover)
    }

    fn definition_hover(&self, id: DefId) -> Option<(String, HoverKind)> {
        if let Some(signature) = self.function_signature(id) {
            return Some((signature.label, HoverKind::Function));
        }
        if let Some((name, structure)) = self.hir.program.struct_by_id(id) {
            let mut lines = vec![format!(
                "struct {}{}",
                name,
                format_generic_params(&structure.generic_params)
            )];
            lines.extend(
                structure.fields.iter().map(|field| {
                    format!("    {}: {}", field.name, self.hir.display_type(&field.ty))
                }),
            );
            return Some((lines.join("\n"), HoverKind::Type));
        }
        if let Some((name, enumeration)) = self.hir.program.enum_by_id(id) {
            return Some((
                format!(
                    "enum {}{}",
                    name,
                    format_generic_params(&enumeration.generic_params)
                ),
                HoverKind::Type,
            ));
        }
        if let Some(trait_def) = self.hir.program.traits.get(&id) {
            return Some((
                format!(
                    "trait {}{}",
                    trait_def.name,
                    format_generic_params(&trait_def.generic_params)
                ),
                HoverKind::Type,
            ));
        }
        if let Some(alias) = self.hir.program.type_aliases.get(&id) {
            return Some((
                format!(
                    "type {}{} = {}",
                    alias.name,
                    format_generic_params(&alias.generic_params),
                    self.hir.display_type(&alias.ty)
                ),
                HoverKind::Type,
            ));
        }
        self.hir.program.impls.get(&id).map(|imp| {
            let label = imp
                .trait_name
                .as_ref()
                .map(|trait_name| format!("impl {} for {}", trait_name, imp.type_name))
                .unwrap_or_else(|| format!("impl {}", imp.type_name));
            (label, HoverKind::Type)
        })
    }

    fn field_hover(&self, owner: DefId, field_id: FieldId) -> Option<String> {
        if let Some(structure) = self.hir.program.structs.get(&owner) {
            let field = structure.fields.iter().find(|field| field.id == field_id)?;
            return Some(format!(
                "{}.{}: {}",
                structure.name,
                field.name,
                self.hir.display_type(&field.ty)
            ));
        }

        let enumeration = self.hir.program.enums.get(&owner)?;
        enumeration.variants.iter().find_map(|variant| {
            let HirVariantFields::Named(fields) = &variant.fields else {
                return None;
            };
            fields
                .iter()
                .find(|field| field.id == field_id)
                .map(|field| {
                    format!(
                        "{}::{}.{}: {}",
                        enumeration.name,
                        variant.name,
                        field.name,
                        self.hir.display_type(&field.ty)
                    )
                })
        })
    }

    fn variant_hover(&self, owner: DefId, variant_id: VariantId) -> Option<String> {
        let enumeration = self.hir.program.enums.get(&owner)?;
        let variant = enumeration
            .variants
            .iter()
            .find(|variant| variant.id == variant_id)?;
        let fields = match &variant.fields {
            HirVariantFields::Named(fields) => fields
                .iter()
                .map(|field| format!("{}: {}", field.name, self.hir.display_type(&field.ty)))
                .collect::<Vec<_>>(),
            HirVariantFields::Positional(fields) => fields
                .iter()
                .map(|field| self.hir.display_type(field))
                .collect(),
            HirVariantFields::Unit => Vec::new(),
        };
        let payload = (!fields.is_empty())
            .then(|| format!(" ({})", fields.join(", ")))
            .unwrap_or_default();
        Some(format!("{}::{}{}", enumeration.name, variant.name, payload))
    }

    fn associated_type_hover(&self, owner: DefId, associated: AssocTypeId) -> Option<String> {
        if let Some(trait_def) = self.hir.program.traits.get(&owner) {
            let associated = trait_def
                .associated_types
                .iter()
                .find(|item| item.id == associated)?;
            return Some(format!(
                "associated type {}::{}",
                trait_def.name, associated.name
            ));
        }
        let imp = self.hir.program.impls.get(&owner)?;
        let associated = imp
            .associated_types
            .iter()
            .find(|item| item.id == associated)?;
        Some(format!(
            "associated type {}::{} = {}",
            imp.type_name,
            associated.name,
            self.hir.display_type(&associated.ty)
        ))
    }

    fn generic_name(&self, id: GenericParamId) -> Option<&str> {
        if let Some(function) = self.function(id.owner) {
            return generic_param_name(&function.generic_params, id);
        }
        if let Some(structure) = self.hir.program.structs.get(&id.owner) {
            return generic_param_name(&structure.generic_params, id);
        }
        if let Some(enumeration) = self.hir.program.enums.get(&id.owner) {
            return generic_param_name(&enumeration.generic_params, id);
        }
        if let Some(trait_def) = self.hir.program.traits.get(&id.owner) {
            return generic_param_name(&trait_def.generic_params, id).or_else(|| {
                trait_def
                    .target
                    .as_ref()
                    .filter(|param| param.id == id)
                    .map(|param| param.name.as_str())
            });
        }
        if let Some(imp) = self.hir.program.impls.get(&id.owner) {
            return generic_param_name(&imp.type_generics, id)
                .or_else(|| generic_param_name(&imp.trait_generics, id));
        }
        self.hir
            .program
            .type_aliases
            .get(&id.owner)
            .and_then(|alias| generic_param_name(&alias.generic_params, id))
    }

    fn generic_hover(&self, id: GenericParamId) -> Option<String> {
        let name = self.generic_name(id)?;
        let bounds = self
            .function(id.owner)
            .map(|function| &function.generic_bounds)
            .or_else(|| self.hir.program.impls.get(&id.owner).map(|imp| &imp.bounds))
            .and_then(|bounds| bounds.get(&id));
        let mut contents = format!("{name}: type parameter");
        if let Some(bounds) = bounds {
            let visible = bounds
                .iter()
                .filter_map(|bound| {
                    let trait_def = self.hir.program.traits.get(&bound.trait_id)?;
                    let arguments = bound
                        .type_args
                        .iter()
                        .map(|ty| self.hir.display_type(ty))
                        .collect::<Vec<_>>();
                    let application = if arguments.is_empty() {
                        trait_def.name.clone()
                    } else {
                        format!("{} {}", trait_def.name, arguments.join(", "))
                    };
                    Some(format!("{name}: {application}"))
                })
                .collect::<Vec<_>>();
            if !visible.is_empty() {
                contents.push_str(&format!("\nwhere {}", visible.join(", ")));
            }
        }
        Some(contents)
    }

    fn hover_type(&self, ty: &Type) -> String {
        let mut contents = self.hir.display_type(ty);
        if crate::type_services::visit::type_any(ty, |nested| matches!(nested, Type::Object(_))) {
            contents.push_str("\n// trait object; reference or owner determines storage");
        }
        let mut witnesses = std::collections::BTreeSet::new();
        crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
            if let Type::Witness(witness) = nested {
                witnesses.insert(*witness);
            }
        });
        for witness in witnesses {
            let mut object = None;
            self.for_each_function(|function| {
                if object.is_none() {
                    object = find_witness_object_in_block(&function.body, witness);
                }
            });
            let bound = object
                .map(|object| {
                    format!(
                        " of {}",
                        self.hir.display_type(&Type::Object(Box::new(object)))
                    )
                })
                .unwrap_or_default();
            contents.push_str(&format!(
                "\n// scoped witness{bound}; descriptor-backed layout"
            ));
        }
        contents
    }

    fn function_signature(&self, id: DefId) -> Option<FunctionSignature> {
        if let Some((name, function)) = self.hir.program.function_by_id(id) {
            return Some(self.format_function_signature(name, function));
        }

        if let Some(location) = self.hir.program.indexes.methods_by_id.get(&id) {
            return match location {
                HirMethodLocation::TraitDefault {
                    trait_id,
                    method_name,
                    ..
                } => self
                    .hir
                    .program
                    .traits
                    .get(trait_id)
                    .and_then(|trait_def| trait_def.methods.get(method_name))
                    .map(|function| self.format_function_signature(method_name, function)),
                HirMethodLocation::ImplMethod {
                    impl_id,
                    method_name,
                    ..
                } => self
                    .hir
                    .program
                    .impls
                    .get(impl_id)
                    .and_then(|imp| imp.methods.get(method_name))
                    .map(|function| self.format_function_signature(method_name, function)),
            };
        }

        for trait_def in self.hir.program.traits.values() {
            if let Some(signature) = trait_def.signatures.values().find(|sig| sig.id == id) {
                let parameters = signature
                    .params
                    .iter()
                    .enumerate()
                    .map(|(index, ty)| format!("arg{}: {}", index + 1, self.hir.display_type(ty)))
                    .collect::<Vec<_>>();
                return Some(FunctionSignature {
                    label: format_signature(
                        &signature.name,
                        &parameters,
                        &self.hir.display_type(&signature.ret),
                    ),
                    parameters,
                });
            }
        }

        self.hir.program.externs.get(&id).map(|external| {
            let parameters = external
                .params
                .iter()
                .enumerate()
                .map(|(index, ty)| format!("arg{}: {}", index + 1, self.hir.display_type(ty)))
                .collect::<Vec<_>>();
            FunctionSignature {
                label: format_signature(
                    &external.name,
                    &parameters,
                    &self.hir.display_type(&external.ret),
                ),
                parameters,
            }
        })
    }

    fn format_function_signature(
        &self,
        name: &str,
        function: &AcceptedHirFunction,
    ) -> FunctionSignature {
        let parameters = function
            .params
            .iter()
            .map(|param| format!("{}: {}", param.name, self.hir.display_type(&param.ty)))
            .collect::<Vec<_>>();
        FunctionSignature {
            label: format_signature(
                name,
                &parameters,
                &self.hir.display_type(&function.ret_type),
            ),
            parameters,
        }
    }

    fn local_binding(&self, owner: DefId, local: HirLocalId) -> Option<(String, Type)> {
        let function = self.function(owner)?;
        if let Some(param) = function.params.iter().find(|param| param.local_id == local) {
            return Some((param.name.clone(), param.ty.clone()));
        }
        find_local_in_block(&function.body, local)
    }

    fn function(&self, id: DefId) -> Option<&AcceptedHirFunction> {
        if let Some((_, function)) = self.hir.program.function_by_id(id) {
            return Some(function);
        }
        match self.hir.program.indexes.methods_by_id.get(&id)? {
            HirMethodLocation::TraitDefault {
                trait_id,
                method_name,
                ..
            } => self
                .hir
                .program
                .traits
                .get(trait_id)?
                .methods
                .get(method_name),
            HirMethodLocation::ImplMethod {
                impl_id,
                method_name,
                ..
            } => self
                .hir
                .program
                .impls
                .get(impl_id)?
                .methods
                .get(method_name),
        }
    }

    fn for_each_function(&self, mut visit: impl FnMut(&AcceptedHirFunction)) {
        self.hir.program.functions.values().for_each(&mut visit);
        for trait_def in self.hir.program.traits.values() {
            trait_def.methods.values().for_each(&mut visit);
        }
        for imp in self.hir.program.impls.values() {
            imp.methods.values().for_each(&mut visit);
        }
    }
}

fn format_generic_params(params: &[GenericParamDecl]) -> String {
    if params.is_empty() {
        String::new()
    } else {
        format!(
            " {}",
            params
                .iter()
                .map(|param| param.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn generic_param_name(params: &[GenericParamDecl], id: GenericParamId) -> Option<&str> {
    params
        .iter()
        .find(|param| param.id == id)
        .map(|param| param.name.as_str())
}

fn format_signature(name: &str, parameters: &[String], ret: &str) -> String {
    if parameters.is_empty() {
        format!("{} = -> {}", name, ret)
    } else {
        format!("{} = {} -> {}", name, parameters.join(", "), ret)
    }
}

fn find_local_in_block(block: &AcceptedHirBlock, local: HirLocalId) -> Option<(String, Type)> {
    for statement in &block.stmts {
        match statement {
            HirStmtFor::Let {
                name,
                local_id,
                ty,
                value,
                ..
            } => {
                if *local_id == local {
                    return Some((name.clone(), ty.clone()));
                }
                if let Some(binding) = find_local_in_expr(value, local) {
                    return Some(binding);
                }
            }
            HirStmtFor::Expr(expr)
            | HirStmtFor::Return(Some(expr))
            | HirStmtFor::Break(Some(expr)) => {
                if let Some(binding) = find_local_in_expr(expr, local) {
                    return Some(binding);
                }
            }
            HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => {}
        }
    }
    None
}

fn find_local_in_expr(expr: &AcceptedHirExpr, local: HirLocalId) -> Option<(String, Type)> {
    if let HirExprKindFor::Open { binding, .. } = &expr.kind {
        if binding.value.local_id == local {
            return Some((binding.value.name.clone(), binding.value.ty.clone()));
        }
    }
    if let HirExprKindFor::ResolvedVar(reference) = &expr.kind {
        if reference.target == HirVarTarget::Local(local) {
            return Some((reference.name.clone(), expr.ty.clone()));
        }
    }

    let mut found = None;
    visit_expr_children(expr, &mut |child| {
        if found.is_some() {
            return;
        }
        found = match child {
            HirChild::Expr(child) => find_local_in_expr(child, local),
            HirChild::Block(block) => find_local_in_block(block, local),
        };
    });

    if found.is_none() {
        if let HirExprKindFor::Lambda { params, .. } = &expr.kind {
            found = params
                .iter()
                .find(|param| param.local_id == local)
                .map(|param| (param.name.clone(), param.ty.clone()));
        }
    }
    found
}

fn find_expression_hover_in_block(
    analysis: &Analysis,
    block: &AcceptedHirBlock,
    path: &Path,
    offset: usize,
    best: &mut Option<(usize, HoverInfo)>,
) {
    for statement in &block.stmts {
        match statement {
            HirStmtFor::Let { value, .. }
            | HirStmtFor::Expr(value)
            | HirStmtFor::Return(Some(value))
            | HirStmtFor::Break(Some(value)) => {
                find_expression_hover_in_expr(analysis, value, path, offset, best)
            }
            HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => {}
        }
    }
}

fn find_expression_hover_in_expr(
    analysis: &Analysis,
    expr: &AcceptedHirExpr,
    path: &Path,
    offset: usize,
    best: &mut Option<(usize, HoverInfo)>,
) {
    if expr.span.file_path == path && expr.span.start <= offset && offset < expr.span.end {
        let width = expr.span.end.saturating_sub(expr.span.start);
        if best
            .as_ref()
            .is_none_or(|(best_width, _)| width < *best_width)
        {
            let (contents, kind) = match &expr.kind {
                HirExprKindFor::ResolvedVar(reference) => (
                    format!("{}: {}", reference.name, analysis.hover_type(&expr.ty)),
                    HoverKind::Variable,
                ),
                HirExprKindFor::FieldAccess(_, name, location) => {
                    let contents = location
                        .as_ref()
                        .and_then(|field| analysis.field_hover(field.owner, field.field_id))
                        .unwrap_or_else(|| {
                            format!("{}: {}", name, analysis.hir.display_type(&expr.ty))
                        });
                    (contents, HoverKind::Field)
                }
                _ => (analysis.hover_type(&expr.ty), HoverKind::Variable),
            };
            *best = Some((
                width,
                HoverInfo {
                    span: expr.span.clone(),
                    contents,
                    kind,
                },
            ));
        }
    }

    visit_expr_children(expr, &mut |child| match child {
        HirChild::Expr(child) => find_expression_hover_in_expr(analysis, child, path, offset, best),
        HirChild::Block(block) => {
            find_expression_hover_in_block(analysis, block, path, offset, best)
        }
    });
}

fn find_signature_in_block(
    analysis: &Analysis,
    block: &AcceptedHirBlock,
    path: &Path,
    offset: usize,
    best: &mut Option<(usize, SignatureInfo)>,
) {
    for statement in &block.stmts {
        match statement {
            HirStmtFor::Let { value, .. }
            | HirStmtFor::Expr(value)
            | HirStmtFor::Return(Some(value))
            | HirStmtFor::Break(Some(value)) => {
                find_signature_in_expr(analysis, value, path, offset, best)
            }
            HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => {}
        }
    }
}

fn find_signature_in_expr(
    analysis: &Analysis,
    expr: &AcceptedHirExpr,
    path: &Path,
    offset: usize,
    best: &mut Option<(usize, SignatureInfo)>,
) {
    let call = match &expr.kind {
        HirExprKindFor::Call(callee, args, target) => {
            let definition = target
                .as_ref()
                .and_then(call_target_definition)
                .or_else(|| {
                    analysis
                        .hir
                        .source_map
                        .symbol_at(path, callee.span.start)
                        .and_then(|(symbol, _)| match symbol {
                            SourceSymbol::Definition(id) => Some(id),
                            _ => None,
                        })
                });
            definition.map(|id| (id, callee.span.start, args.as_slice()))
        }
        HirExprKindFor::MethodCall(receiver, _, args, _, target) => Some((
            method_target_definition(target),
            receiver.span.start,
            args.as_slice(),
        )),
        HirExprKindFor::OwnedObjectCall { owner, args, call } => Some((
            method_target_definition(&call.method),
            owner.span.start,
            args.as_slice(),
        )),
        _ => None,
    };

    if let Some((definition, call_start, args)) = call {
        let call_end = args
            .last()
            .map(|argument| argument.span.end)
            .unwrap_or(expr.span.end)
            .max(expr.span.end);
        if expr.span.file_path == path && call_start <= offset && offset <= call_end {
            if let Some(signature) = analysis.function_signature(definition) {
                let active_parameter = args
                    .iter()
                    .position(|argument| offset <= argument.span.end)
                    .unwrap_or(args.len())
                    .min(signature.parameters.len().saturating_sub(1));
                let width = call_end.saturating_sub(call_start);
                if best
                    .as_ref()
                    .is_none_or(|(best_width, _)| width < *best_width)
                {
                    *best = Some((
                        width,
                        SignatureInfo {
                            label: signature.label,
                            parameters: signature.parameters,
                            active_parameter,
                        },
                    ));
                }
            }
        }
    }

    visit_expr_children(expr, &mut |child| match child {
        HirChild::Expr(child) => find_signature_in_expr(analysis, child, path, offset, best),
        HirChild::Block(block) => find_signature_in_block(analysis, block, path, offset, best),
    });
}

fn call_target_definition(target: &HirCallTarget) -> Option<DefId> {
    match target {
        HirCallTarget::Function(id) | HirCallTarget::Extern(id) => Some(*id),
        HirCallTarget::StaticMethod(target) => target.method.method_id(),
        HirCallTarget::Instance(_) | HirCallTarget::Local(_) | HirCallTarget::Intrinsic(_) => None,
    }
}

fn method_target_definition(target: &crate::hir::HirMethodCallTarget) -> DefId {
    match target.target {
        HirSelectedMethodTarget::ImplMethod { method_id, .. } => method_id,
        HirSelectedMethodTarget::TraitMethod { member_id, .. } => member_id,
    }
}

enum HirChild<'a> {
    Expr(&'a AcceptedHirExpr),
    Block(&'a AcceptedHirBlock),
}

fn find_witness_object_in_block(
    block: &AcceptedHirBlock,
    witness: crate::types::WitnessId,
) -> Option<crate::types::ObjectType> {
    block.stmts.iter().find_map(|statement| match statement {
        HirStmtFor::Let { value, .. }
        | HirStmtFor::Expr(value)
        | HirStmtFor::Return(Some(value))
        | HirStmtFor::Break(Some(value)) => find_witness_object_in_expr(value, witness),
        _ => None,
    })
}

fn find_witness_object_in_expr(
    expr: &AcceptedHirExpr,
    witness: crate::types::WitnessId,
) -> Option<crate::types::ObjectType> {
    if let HirExprKindFor::Open { binding, .. } = &expr.kind {
        if binding.witness == witness {
            return Some(binding.object.clone());
        }
    }
    let mut found = None;
    visit_expr_children(expr, &mut |child| {
        if found.is_none() {
            found = match child {
                HirChild::Expr(expr) => find_witness_object_in_expr(expr, witness),
                HirChild::Block(block) => find_witness_object_in_block(block, witness),
            };
        }
    });
    found
}

fn visit_expr_children(expr: &AcceptedHirExpr, visit: &mut impl FnMut(HirChild<'_>)) {
    match &expr.kind {
        HirExprKindFor::Open { source, body, .. } => {
            visit(HirChild::Expr(source));
            visit(HirChild::Block(body));
        }
        HirExprKindFor::Open { source, body, .. } => {
            visit(HirChild::Expr(source));
            visit(HirChild::Block(body));
        }
        HirExprKindFor::OwnedObjectCall { owner, args, .. } => {
            visit(HirChild::Expr(owner));
            args.iter().for_each(|expr| visit(HirChild::Expr(expr)));
        }
        HirExprKindFor::ArrayLiteral(values) | HirExprKindFor::TupleLiteral(values) => {
            values.iter().for_each(|expr| visit(HirChild::Expr(expr)))
        }
        HirExprKindFor::ArrayRepeat(value, _)
        | HirExprKindFor::ObjectCoercion(value, _)
        | HirExprKindFor::UnaryOp(_, value)
        | HirExprKindFor::FieldAccess(value, _, _)
        | HirExprKindFor::TupleIndex(value, _)
        | HirExprKindFor::Ref(_, value)
        | HirExprKindFor::Deref(value)
        | HirExprKindFor::Cast(value, _) => visit(HirChild::Expr(value)),
        HirExprKindFor::BinOp(_, left, right) | HirExprKindFor::Assign(left, right) => {
            visit(HirChild::Expr(left));
            visit(HirChild::Expr(right));
        }
        HirExprKindFor::Call(callee, args, _) => {
            visit(HirChild::Expr(callee));
            args.iter().for_each(|expr| visit(HirChild::Expr(expr)));
        }
        HirExprKindFor::MethodCall(receiver, _, args, _, _) => {
            visit(HirChild::Expr(receiver));
            args.iter().for_each(|expr| visit(HirChild::Expr(expr)));
        }
        HirExprKindFor::Try { expr, .. } => visit(HirChild::Expr(expr)),
        HirExprKindFor::StructLiteral(_, _, fields) => fields
            .iter()
            .for_each(|field| visit(HirChild::Expr(&field.value))),
        HirExprKindFor::EnumVariant(_, _, args, _) | HirExprKindFor::Intrinsic { args, .. } => {
            args.iter().for_each(|expr| visit(HirChild::Expr(expr)))
        }
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            visit(HirChild::Expr(condition));
            visit(HirChild::Block(then_branch));
            if let Some(else_branch) = else_branch {
                visit(HirChild::Block(else_branch));
            }
        }
        HirExprKindFor::Match { scrutinee, arms } => {
            visit(HirChild::Expr(scrutinee));
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    visit(HirChild::Expr(guard));
                }
                visit(HirChild::Block(&arm.body));
            }
        }
        HirExprKindFor::While { condition, body } => {
            visit(HirChild::Expr(condition));
            visit(HirChild::Block(body));
        }
        HirExprKindFor::For { iter, body, .. } => {
            visit(HirChild::Expr(iter));
            visit(HirChild::Block(body));
        }
        HirExprKindFor::Loop(body)
        | HirExprKindFor::Block(body)
        | HirExprKindFor::UnsafeBlock(body) => visit(HirChild::Block(body)),
        HirExprKindFor::Lambda { body, .. } => visit(HirChild::Block(body)),
        HirExprKindFor::IntLiteral(_)
        | HirExprKindFor::FloatLiteral(_)
        | HirExprKindFor::BoolLiteral(_)
        | HirExprKindFor::StringLiteral(_)
        | HirExprKindFor::CharLiteral(_)
        | HirExprKindFor::Unit
        | HirExprKindFor::Var(_)
        | HirExprKindFor::ResolvedVar(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{Config, SourceProvider};

    fn analyze(source: &str) -> super::Analysis {
        let path = PathBuf::from("/virtual/main.rk");
        crate::analyze(&Config {
            entry_file: path.clone(),
            source_providers: vec![SourceProvider::Virtual {
                path,
                text: source.to_string(),
            }],
            no_prelude: true,
            no_std: true,
            ..Config::default()
        })
        .unwrap()
    }

    #[test]
    fn analysis_reports_inferred_local_types_and_function_signatures() {
        let source = "id = value -> value\n\nmain = ->\n    answer = id 42\n    answer\n";
        let analysis = analyze(source);
        let path = PathBuf::from("/virtual/main.rk");

        let local_offset = source.rfind("answer").unwrap();
        let local = analysis.hover(&path, local_offset).unwrap();
        assert_eq!(local.contents, "answer: I64");

        let function_offset = source.find("id 42").unwrap();
        let function = analysis.hover(&path, function_offset).unwrap();
        assert!(function.contents.starts_with("id = value: "));

        let argument_offset = source.find("42").unwrap() + 1;
        let signature = analysis.signature(&path, argument_offset).unwrap();
        assert!(signature.label.starts_with("id = value: "));
        assert_eq!(signature.parameters.len(), 1);
        assert_eq!(signature.active_parameter, 0);
    }

    #[test]
    fn analysis_reports_named_types_and_fields() {
        let source = "struct Holder\n    < value: I64\n\nread: Holder -> I64\nread = holder -> holder.value\n\nmain = -> 0\n";
        let analysis = analyze(source);
        let path = PathBuf::from("/virtual/main.rk");

        let structure = analysis
            .hover(&path, source.find("Holder").unwrap())
            .unwrap();
        assert_eq!(structure.contents, "struct Holder\n    value: I64");

        let declaration = analysis
            .hover(&path, source.find("value").unwrap())
            .unwrap();
        assert_eq!(declaration.contents, "Holder.value: I64");

        let reference = analysis
            .hover(&path, source.rfind("value").unwrap())
            .unwrap();
        assert_eq!(reference.contents, "Holder.value: I64");
    }

    #[test]
    fn analysis_distinguishes_object_views_from_bounded_type_parameters() {
        let source = r#"
trait Read
    @read: I64
struct Number
    < value: I64
impl Read for Number
    @read = -> self.value
read_static: T -> I64 where T: Read
read_static = item -> item.read!
main = ->
    number = Number
        value: 42
    object: &Read = &number
    object.read!
"#;
        let analysis = analyze(source);
        let path = PathBuf::from("/virtual/main.rk");
        let object = analysis
            .hover(&path, source.rfind("object.read").unwrap())
            .unwrap();
        assert!(object.contents.starts_with("object: &Read"), "{object:?}");
        assert!(object.contents.contains("trait object"), "{object:?}");

        let parameter = analysis.hover(&path, source.find("T ->").unwrap()).unwrap();
        assert_eq!(parameter.kind, super::HoverKind::Generic);
        assert!(parameter.contents.starts_with("T: type parameter"));
        assert!(parameter.contents.contains("T: Read"), "{parameter:?}");
        assert!(!parameter.contents.contains("trait object"));

        let member = analysis
            .hover(&path, source.rfind("read!").unwrap())
            .unwrap();
        assert_eq!(member.kind, super::HoverKind::Function);
        assert!(member.contents.contains("I64"), "{member:?}");
    }

    #[test]
    fn analysis_reports_opened_witness_bounds_without_exposing_a_concrete_type() {
        let source = r#"
trait Read
    @read: I64
struct Number
    < value: I64
impl Read for Number
    @read = -> self.value
read_open: &Read -> I64
read_open = object ->
    open object as T, value
        value.read!
main = ->
    number = Number
        value: 42
    read_open &number
"#;
        let analysis = analyze(source);
        let path = PathBuf::from("/virtual/main.rk");
        let value = analysis
            .hover(&path, source.find("value.read").unwrap())
            .unwrap();
        assert!(
            value.contents.contains("scoped witness of Read"),
            "{value:?}"
        );
        assert!(value.contents.contains("descriptor-backed layout"));
        assert!(!value.contents.contains("Number"));
    }

    #[test]
    fn analysis_errors_keep_virtual_source_for_lsp_conversion() {
        let path = PathBuf::from("/virtual/error.rk");
        let source = "main = ->\n    `\n";
        let diagnostics = crate::analyze(&Config {
            entry_file: path.clone(),
            source_providers: vec![SourceProvider::Virtual {
                path,
                text: source.to_string(),
            }],
            no_prelude: true,
            no_std: true,
            ..Config::default()
        })
        .unwrap_err();

        assert!(diagnostics.0.iter().any(|diagnostic| diagnostic
            .source
            .as_ref()
            .is_some_and(|source| { source.text == "main = ->\n    `\n" })));
    }
}
