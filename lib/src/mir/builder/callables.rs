use crate::hir::{HirCallTarget, HirVarTarget};
use crate::ids::DefId;
use crate::types::Type;

use crate::mir::{MirCallable, MirFieldIdentity, MirIntrinsicId, Projection};

use super::*;

impl<'a> MirBuilder<'a> {
    pub(super) fn callable_for_var_target(&self, target: &HirVarTarget) -> Option<MirCallable> {
        match target {
            HirVarTarget::Function(id) => Some(MirCallable::Resolved(
                self.callable_key_for_def_id(*id, crate::mir::MirCallableKey::Function(*id)),
            )),
            HirVarTarget::Extern(id) => Some(MirCallable::Resolved(
                self.callable_key_for_def_id(*id, crate::mir::MirCallableKey::Extern(*id)),
            )),
            HirVarTarget::Instance(id) => Some(MirCallable::Resolved(
                crate::mir::MirCallableKey::Instance(*id),
            )),
            HirVarTarget::Local(_) => None,
        }
    }

    pub(super) fn callable_for_expr(&self, expr: &HirExpr, ret_ty: &Type) -> Option<MirCallable> {
        match &expr.kind {
            HirExprKind::ResolvedVar(reference) => {
                self.callable_for_var_target_with_return(&reference.target, ret_ty)
            }
            _ => None,
        }
    }

    pub(super) fn callable_for_call_target(
        &self,
        target: &HirCallTarget,
        _ret_ty: &Type,
    ) -> Option<MirCallable> {
        match target {
            HirCallTarget::Function(id) => Some(MirCallable::Resolved(
                self.callable_key_for_def_id(*id, crate::mir::MirCallableKey::Function(*id)),
            )),
            HirCallTarget::Extern(id) => Some(MirCallable::Resolved(
                self.callable_key_for_def_id(*id, crate::mir::MirCallableKey::Extern(*id)),
            )),
            HirCallTarget::Instance(id) => Some(MirCallable::Resolved(
                crate::mir::MirCallableKey::Instance(*id),
            )),
            HirCallTarget::Local(_) => None,
            HirCallTarget::Intrinsic(name) => MirIntrinsicId::from_name(name)
                .map(|id| MirCallable::Resolved(crate::mir::MirCallableKey::Intrinsic(id))),
            HirCallTarget::StaticMethod(_) => {
                panic!("unmaterialized static method target reached MIR lowering")
            }
        }
    }

    fn callable_key_for_def_id(
        &self,
        id: DefId,
        fallback: crate::mir::MirCallableKey,
    ) -> crate::mir::MirCallableKey {
        if self.method_def_ids.contains(&id) {
            panic!("unmaterialized method DefId reached MIR lowering: {id:?}");
        }
        self.callable_instances_by_def_id
            .get(&id)
            .copied()
            .map(crate::mir::MirCallableKey::Instance)
            .unwrap_or(fallback)
    }

    fn callable_for_var_target_with_return(
        &self,
        target: &HirVarTarget,
        _ret_ty: &Type,
    ) -> Option<MirCallable> {
        match target {
            HirVarTarget::Function(id) => Some(MirCallable::Resolved(
                self.callable_key_for_def_id(*id, crate::mir::MirCallableKey::Function(*id)),
            )),
            _ => self.callable_for_var_target(target),
        }
    }

    pub(super) fn field_access_resolves_to_struct_field(
        &self,
        base_ty: &Type,
        field_name: &str,
        location: Option<&crate::hir::HirFieldLocation>,
    ) -> bool {
        let Type::Struct { id, .. } = base_ty else {
            return false;
        };
        let Some((_, fields)) = self.program.struct_by_id(*id) else {
            return false;
        };

        location
            .filter(|location| location.owner == fields.id)
            .is_some_and(|location| {
                fields.fields.iter().any(|field| {
                    field.id == location.field_id
                        && field.name == location.name
                        && field.name == *field_name
                })
            })
    }

    pub(super) fn unknown_field_message(&self, base_ty: &Type, field_name: &str) -> String {
        let type_name = match base_ty {
            Type::Struct { id, .. } => self
                .program
                .indexes
                .structs_by_id
                .get(id)
                .cloned()
                .unwrap_or_else(|| base_ty.to_string()),
            Type::Enum { id, .. } => self
                .program
                .indexes
                .enums_by_id
                .get(id)
                .cloned()
                .unwrap_or_else(|| base_ty.to_string()),
            _ => base_ty.to_string(),
        };

        format!("Unknown field '{}' on struct '{}'", field_name, type_name)
    }

    pub(super) fn method_receiver_mode_for_callable(
        &self,
        callable: &MirCallable,
    ) -> Option<crate::types::ReceiverMode> {
        match callable {
            MirCallable::Resolved(crate::mir::MirCallableKey::Instance(id)) => self
                .method_instance_receiver_modes
                .get(id)
                .copied()
                .flatten(),
            _ => None,
        }
    }

    pub(super) fn field_projection(
        &self,
        index: usize,
        owner: Option<crate::ids::DefId>,
        field_id: Option<crate::ids::FieldId>,
    ) -> Projection {
        Projection::Field {
            index,
            identity: owner
                .zip(field_id)
                .map(|(owner, field_id)| MirFieldIdentity::new(owner, field_id)),
        }
    }
}
