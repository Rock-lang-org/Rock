//! Source-place permissions shared by known lowering and deferred authority.

use crate::hir::{HirExpr, HirExprKind};
use crate::types::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssignmentAccess {
    Writable,
    Shared,
    Pending,
}

pub(crate) const SHARED_ASSIGNMENT: &str = "Cannot assign through a shared reference: mutable access is required";

/// This checks only reference access along the actual selected place. Binding
/// mutability, moves, loans and raw-pointer aliasing remain MIR responsibilities.
pub(crate) fn assignment_access(
    place: &HirExpr,
    resolve: &impl Fn(&Type) -> Type,
) -> AssignmentAccess {
    match &place.kind {
        HirExprKind::FieldAccess(_, _, None) => AssignmentAccess::Pending,
        HirExprKind::FieldAccess(base, _, Some(_)) | HirExprKind::TupleIndex(base, _) => {
            // Covers an implicit reference projection as well as explicit Deref.
            if matches!(resolve(&base.ty), Type::Reference { mutable: false, .. }) {
                AssignmentAccess::Shared
            } else {
                assignment_access(base, resolve)
            }
        }
        HirExprKind::Deref(handle) => match resolve(&handle.ty) {
            Type::Reference { mutable: false, .. } => AssignmentAccess::Shared,
            Type::Reference { mutable: true, .. } => assignment_access(handle, resolve),
            // Reading a raw handle is not a write to its storage. The source
            // dereference relation checks unsafe independently of this permission.
            Type::Pointer(_) => AssignmentAccess::Writable,
            _ => AssignmentAccess::Pending,
        },
        _ => AssignmentAccess::Writable,
    }
}
