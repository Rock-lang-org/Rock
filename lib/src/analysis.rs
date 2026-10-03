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
    if let HirExprKindFor::PackageOpen { binding, .. } = &expr.kind {
        if binding.value.local_id == local {
            return Some((binding.value.name.clone(), binding.value.ty.clone()));
        }
    }
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
    if let HirExprKindFor::PackageOpen { binding, .. } = &expr.kind {
        if binding.witness == witness {
            return binding.object_schema().ok();
        }
    }
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
        HirExprKindFor::PackagePack { payload, .. } => visit(HirChild::Expr(payload)),
        HirExprKindFor::Open { source, body, .. }
        | HirExprKindFor::PackageOpen { source, body, .. } => {
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

    const CALLABLE_PROTOCOL: &str = r#"
lang fn_once
< trait Call Args, Ret
    lang output
    type Output
    lang method
    ~@invoke: Args -> Ret
lang fn_mut
< trait CallMut Args, Ret where Self: Call Args, Ret
    lang output
    type Output
    lang method
    ^@invoke_mut: Args -> Ret
"#;

    fn visit_block_expressions(
        block: &crate::hir::AcceptedHirBlock,
        visit: &mut impl FnMut(&crate::hir::AcceptedHirExpr),
    ) {
        for statement in &block.stmts {
            match statement {
                crate::hir::HirStmtFor::Let { value, .. }
                | crate::hir::HirStmtFor::Expr(value)
                | crate::hir::HirStmtFor::Return(Some(value))
                | crate::hir::HirStmtFor::Break(Some(value)) => visit_expression(value, visit),
                _ => {}
            }
        }
    }

    fn visit_expression(
        expression: &crate::hir::AcceptedHirExpr,
        visit: &mut impl FnMut(&crate::hir::AcceptedHirExpr),
    ) {
        visit(expression);
        super::visit_expr_children(expression, &mut |child| match child {
            super::HirChild::Expr(expression) => visit_expression(expression, visit),
            super::HirChild::Block(block) => visit_block_expressions(block, visit),
        });
    }

    #[test]
    fn analysis_fold_returns_preserve_source_and_nested_callback_types() {
        use crate::hir::HirExprKindFor;
        use crate::types::Type;

        let source = format!(
            "{CALLABLE_PROTOCOL}{}",
            r#"
lang control_flow
< enum Signal Stop, State
    lang break
    Halt Stop
    lang continue
    More State
lang try
< trait Propagate
    lang output
    type Value
    lang residual
    type Error
    lang branch
    ~@split: Signal Self::Error, Self::Value
lang from_residual
< trait Lift E
    lang method
    lift: E -> Self
lang fold
< trait Each A
    lang method
    ~@scan: S -> B -> Signal R, B where S: CallMut (B, A), (Signal R, B)
struct Once
    < value: I64
impl Each I64 for Once
    ~@scan = mut step, initial -> step (initial, self.value)
main = ->
    source = Once
        value: 42
    for outer in source
        local = -> return outer
        nested = Once
            value: local!
        for inner in nested
            while false
                return inner
            return inner
    0
"#
        );
        let analysis = analyze(&source);
        let main = analysis
            .hir
            .program
            .functions
            .values()
            .find(|function| function.name == "main")
            .unwrap();
        assert_eq!(main.ret_type, Type::I64);
        let protocol = analysis
            .hir
            .program
            .language_items
            .try_protocol
            .as_ref()
            .unwrap();
        let mut depths = Vec::new();
        let mut local_return = None;
        visit_block_expressions(&main.body, &mut |expression| {
            if let HirExprKindFor::Lambda { params, .. } = &expression.kind {
                let Type::Function { ret, .. } = &expression.ty else {
                    panic!("lambda requires a finalized function type");
                };
                if params
                    .first()
                    .is_some_and(|param| param.name == "$fold_pair")
                {
                    let mut payload = ret.as_ref();
                    let mut depth = 0;
                    while let Type::Enum { id, args } = payload {
                        assert_eq!(*id, protocol.control_flow_enum_id);
                        assert_eq!(args[1], Type::Unit);
                        payload = &args[0];
                        depth += 1;
                    }
                    assert_eq!(*payload, Type::I64);
                    depths.push(depth);
                } else {
                    local_return = Some(ret.as_ref().clone());
                }
            }
        });
        assert_eq!(depths, vec![2, 4]);
        assert_eq!(local_return, Some(Type::I64));
    }

    #[test]
    fn analysis_contextual_callbacks_see_deferred_reference_sources() {
        use crate::hir::HirExprKindFor;
        use crate::types::Type;

        let source = format!(
            "{CALLABLE_PROTOCOL}{}",
            r#"
dispatch = mut target, callback ->
    callback target
    0
trait Inspect A
    ~@item: A
struct Number
    < value: I64
impl Inspect &I64 for &Number
    ~@item = -> &self.value
visit: C -> F -> () where C: Inspect T, F: CallMut T, ()
visit = source, mut callback !-> callback source.item!
main = !->
    mut value = 0
    dispatch &mut value, (slot -> *slot = 41)
    number = Number
        value: 42
    visit &number, (element !-> *element)
"#
        );
        let analysis = analyze(&source);
        let main = analysis
            .hir
            .program
            .functions
            .values()
            .find(|function| function.name == "main")
            .unwrap();
        let mut parameter_types = Vec::new();
        visit_block_expressions(&main.body, &mut |expression| {
            if let HirExprKindFor::Lambda { params, .. } = &expression.kind {
                parameter_types.extend(params.iter().map(|param| param.ty.clone()));
            }
        });
        assert_eq!(
            parameter_types,
            vec![
                Type::Reference {
                    mutable: true,
                    inner: Box::new(Type::I64),
                },
                Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::I64),
                },
            ]
        );
    }

    #[test]
    fn analysis_contextual_callback_still_rejects_scalar_dereference() {
        let source = format!(
            "{CALLABLE_PROTOCOL}{}",
            r#"
dispatch = target, callback ->
    callback target
    0
main = ->
    dispatch true, (slot -> *slot)
    0
"#
        );
        let path = PathBuf::from("/virtual/main.rk");
        let diagnostics = crate::analyze(&Config {
            entry_file: path.clone(),
            source_providers: vec![SourceProvider::Virtual { path, text: source }],
            no_prelude: true,
            no_std: true,
            ..Config::default()
        })
        .unwrap_err();
        assert!(diagnostics.0.iter().any(|diagnostic| diagnostic
            .message
            .contains("Cannot dereference non-pointer type: Bool")));
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

        let function = analysis
            .hir
            .program
            .functions
            .values()
            .find(|function| function.name == "read_static")
            .unwrap();
        let generic = function
            .generic_params
            .iter()
            .find(|parameter| parameter.name == "T")
            .unwrap();
        for offset in [
            source.find("T ->").unwrap(),
            source.find("T: Read").unwrap(),
        ] {
            let (symbol, _) = analysis.hir.source_map.symbol_at(&path, offset).unwrap();
            assert_eq!(symbol, crate::source_map::SourceSymbol::Generic(generic.id));
            assert_eq!(
                analysis.hover(&path, offset).unwrap().contents,
                parameter.contents
            );
        }

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
    fn analysis_keeps_shadowed_opening_bounds_distinct() {
        let source = r#"
trait First
    @first: I64
trait Second
    @second: I64
inspect: &First -> &Second -> I64
inspect = first, second ->
    open first as T, value
        result = value.first!
        open second as T, value
            ~I64Add result, (value.second!)
main = -> 0
"#;
        let analysis = analyze(source);
        let path = PathBuf::from("/virtual/main.rk");
        for (expression, bound, other_bound) in [
            ("value.first!", "First", "Second"),
            ("value.second!", "Second", "First"),
        ] {
            let hover = analysis
                .hover(&path, source.find(expression).unwrap())
                .unwrap();
            assert!(
                hover
                    .contents
                    .contains(&format!("scoped witness of {bound}")),
                "{hover:?}"
            );
            assert!(!hover.contents.contains(other_bound), "{hover:?}");
        }
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

    #[test]
    fn call_expression_ranges_are_distinct_from_exact_member_references() {
        use crate::hir::{HirExprKindFor, HirStmtFor};
        use crate::source_map::SourceSymbol;

        let source = r#"
struct Number
    < value: I64
impl Number
    @read: I64
    @read = -> self.value
    @add: I64 -> I64 -> I64
    @add = left, right -> ~I64Add left, right
ordinary = -> 42
main = !->
    value = Number
        value: 7
    zero = value.read!
    answer = value.add 20, 22
    plain = ordinary!
"#;
        let analysis = analyze(source);
        let path = PathBuf::from("/virtual/main.rk");
        let function = analysis
            .hir
            .program
            .functions
            .values()
            .find(|function| function.name == "main")
            .unwrap();
        for (binding, expected, member) in [
            ("zero", "value.read!", "read"),
            ("answer", "value.add 20, 22", "add"),
            ("plain", "ordinary!", "ordinary"),
        ] {
            let value = function
                .body
                .stmts
                .iter()
                .find_map(|statement| match statement {
                    HirStmtFor::Let { name, value, .. } if name == binding => Some(value),
                    _ => None,
                })
                .unwrap();
            let start = source.find(expected).unwrap();
            assert_eq!(
                value.span,
                crate::lexer::Span::new(path.clone(), start, start + expected.len())
            );
            assert_eq!(&source[value.span.start..value.span.end], expected);
            let definition = match &value.kind {
                HirExprKindFor::MethodCall(_, _, _, _, target) => target.method_id().unwrap(),
                HirExprKindFor::Call(_, _, Some(crate::hir::HirCallTarget::Function(id))) => *id,
                other => panic!("expected selected call: {other:?}"),
            };
            let references = analysis
                .hir
                .source_map
                .reference_spans(&SourceSymbol::Definition(definition))
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(references.len(), 1, "{binding}: {references:?}");
            assert_eq!(references[0].file_path, path);
            assert_eq!(&source[references[0].start..references[0].end], member);
            let hover = analysis.hover(&path, references[0].start).unwrap();
            assert_eq!(hover.kind, super::HoverKind::Function);
            assert_eq!(hover.span, references[0]);
            if binding != "plain" {
                let (symbol, receiver_span) =
                    analysis.hir.source_map.symbol_at(&path, start).unwrap();
                assert!(
                    matches!(symbol, SourceSymbol::Local { owner, .. } if owner == function.id)
                );
                assert_eq!(&source[receiver_span.start..receiver_span.end], "value");
            }
        }
    }
}
