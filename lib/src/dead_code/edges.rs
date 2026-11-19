//! The item ids a piece of accepted HIR refers to: callees, constructed and
//! mentioned types, traits named in bounds and method dispatch.

use crate::hir::{
    AcceptedHirExpr, AcceptedHirFunction, HirCallTarget, HirExprKindFor, HirFunctionSig,
    HirGenericBounds, HirImplReceiverPattern, HirMethodCallTarget, HirPattern,
    HirSelectedMethodTarget, HirVarTarget,
};
use crate::ids::{DefId, VariantId};
use crate::lint::walk::{walk_function, HirVisitor};
use crate::type_services::visit::visit_type;
use crate::types::{Predicate, Type};

/// Accumulates referenced item ids; duplicates are filtered by the caller.
#[derive(Default)]
pub(super) struct Edges {
    pub(super) ids: Vec<DefId>,
    pub(super) constructed: Vec<(DefId, VariantId)>,
}

impl Edges {
    pub(super) fn generics(&mut self, params: &[crate::types::GenericParamDecl]) {
        for param in params {
            if let Some(default) = &param.default {
                self.ty(default);
            }
        }
    }
    pub(super) fn ty(&mut self, ty: &Type) {
        let ids = &mut self.ids;
        visit_type(ty, &mut |ty: &Type| match ty {
            Type::Struct { id, .. } | Type::Enum { id, .. } | Type::Constructor { id, .. } => {
                ids.push(*id)
            }
            Type::Projection {
                trait_id,
                assoc_type,
                ..
            } => {
                ids.push(*trait_id);
                ids.push(assoc_type.owner);
            }
            _ => {}
        });
    }

    pub(super) fn bounds(&mut self, bounds: &HirGenericBounds) {
        for bound in bounds.values().flatten() {
            self.ids.push(bound.trait_id);
            bound.type_args.iter().for_each(|ty| self.ty(ty));
        }
        self.predicates(&bounds.predicates);
    }

    pub(super) fn predicates(&mut self, predicates: &[Predicate]) {
        for Predicate::Trait {
            subject,
            trait_id,
            args,
        } in predicates
        {
            self.ids.push(*trait_id);
            self.ty(subject);
            args.iter().for_each(|ty| self.ty(ty));
        }
    }

    pub(super) fn receiver(&mut self, pattern: &HirImplReceiverPattern) {
        match pattern {
            HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                self.ty(ty)
            }
            HirImplReceiverPattern::SliceFamily { element } => self.ty(element),
        }
    }

    pub(super) fn signature(&mut self, signature: &HirFunctionSig) {
        self.generics(&signature.generic_params);
        signature.params.iter().for_each(|ty| self.ty(ty));
        self.ty(&signature.ret);
        self.bounds(&signature.generic_bounds);
    }

    pub(super) fn function(&mut self, function: &AcceptedHirFunction) {
        self.generics(&function.generic_params);
        function.params.iter().for_each(|param| self.ty(&param.ty));
        self.ty(&function.ret_type);
        self.bounds(&function.generic_bounds);
        walk_function(self, function);
    }

    fn method(&mut self, target: &HirMethodCallTarget) {
        match &target.target {
            HirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait,
            } => {
                self.ids.push(*impl_id);
                self.ids.push(*method_id);
                if let Some(selected) = selected_trait {
                    self.ids.push(selected.trait_id);
                    self.ids.push(selected.member_id);
                    selected.trait_args.iter().for_each(|ty| self.ty(ty));
                }
            }
            HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                trait_args,
                ..
            } => {
                self.ids.push(*trait_id);
                self.ids.push(*member_id);
                trait_args.iter().for_each(|ty| self.ty(ty));
            }
        }
        for binding in target
            .owner_substitution
            .iter()
            .chain(&target.method_substitution)
        {
            self.ty(&binding.ty);
        }
    }

    fn call(&mut self, target: &HirCallTarget) {
        match target {
            HirCallTarget::Function(id) | HirCallTarget::Extern(id) => self.ids.push(*id),
            HirCallTarget::StaticMethod(target) => {
                self.ty(&target.owner_ty);
                self.method(&target.method);
            }
            HirCallTarget::Instance(_) | HirCallTarget::Local(_) | HirCallTarget::Intrinsic(_) => {}
        }
    }
}

impl HirVisitor for Edges {
    fn visit_type(&mut self, ty: &Type) {
        self.ty(ty);
    }
    fn enter_expr(&mut self, expr: &AcceptedHirExpr) -> bool {
        self.ty(&expr.ty);
        match &expr.kind {
            HirExprKindFor::ResolvedVar(var) => match var.target {
                HirVarTarget::Function(id) | HirVarTarget::Extern(id) => self.ids.push(id),
                HirVarTarget::Local(_) | HirVarTarget::Instance(_) => {}
            },
            HirExprKindFor::Call(_, _, Some(target)) => self.call(target),
            HirExprKindFor::MethodCall(_, _, _, _, target) => self.method(target),
            HirExprKindFor::Try {
                branch_method,
                branch_target,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                control_flow_enum,
                break_variant,
                continue_variant,
                ..
            } => {
                self.method(branch_method);
                if let Some(target) = branch_target {
                    self.call(target);
                }
                self.call(from_residual_target);
                for ty in [output_ty, residual_ty, return_ty] {
                    self.ty(ty);
                }
                self.ids.extend([
                    *control_flow_enum,
                    break_variant.owner,
                    continue_variant.owner,
                ]);
            }
            HirExprKindFor::StructLiteral(_, id, fields) => {
                self.ids.extend(*id);
                self.ids.extend(
                    fields
                        .iter()
                        .filter_map(|field| field.field.as_ref())
                        .map(|f| f.owner),
                );
            }
            HirExprKindFor::FieldAccess(_, _, Some(field)) => self.ids.push(field.owner),
            HirExprKindFor::EnumVariant(_, _, _, Some(variant)) => {
                self.ids.push(variant.owner);
                self.constructed.push((variant.owner, variant.variant_id));
            }
            HirExprKindFor::Cast(_, ty) => self.ty(ty),
            HirExprKindFor::Lambda {
                params, captures, ..
            } => {
                params.iter().for_each(|param| self.ty(&param.ty));
                captures.iter().for_each(|capture| self.ty(&capture.ty));
            }
            _ => {}
        }
        true
    }

    fn visit_pattern(&mut self, pattern: &HirPattern) {
        match pattern {
            HirPattern::Struct(_, id, args, fields) => {
                self.ids.extend(*id);
                args.iter().for_each(|ty| self.ty(ty));
                self.ids.extend(
                    fields
                        .iter()
                        .filter_map(|field| field.field.as_ref())
                        .map(|f| f.owner),
                );
            }
            HirPattern::Enum(_, _, Some(variant), _) => self.ids.push(variant.owner),
            _ => {}
        }
    }
}
