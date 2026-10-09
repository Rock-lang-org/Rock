use crate::ids::TypeId;
use crate::type_context::{Ty, TyFunctionCapture, TypeContext};
use crate::type_services::facts::TypeFacts;
use crate::type_services::kind::Kind;
use crate::types::{CallableKind, Type};

#[derive(Clone, Copy)]
pub struct TypeView<'a> {
    context: &'a TypeContext,
}

impl<'a> TypeView<'a> {
    pub fn new(context: &'a TypeContext) -> Self {
        Self { context }
    }

    pub fn ty(&self, id: TypeId) -> &'a Ty {
        self.context.ty(id)
    }

    pub fn try_ty(&self, id: TypeId) -> Option<&'a Ty> {
        self.context.try_ty(id)
    }

    pub fn kind(&self, id: TypeId) -> &'a Kind {
        self.context.kind(id)
    }

    pub fn try_kind(&self, id: TypeId) -> Option<&'a Kind> {
        self.context.try_kind(id)
    }

    pub fn type_for(&self, id: TypeId) -> Type {
        self.context.type_for(id)
    }

    pub fn id_for_type(&self, ty: &Type) -> Option<TypeId> {
        self.context.id_for_type(ty)
    }

    pub fn callable_kind(&self, id: TypeId) -> Option<CallableKind> {
        match self.ty(id) {
            Ty::Function { callable_kind, .. } => Some(*callable_kind),
            _ => None,
        }
    }

    pub fn captures(&self, id: TypeId) -> Option<&'a [TyFunctionCapture]> {
        match self.ty(id) {
            Ty::Function { captures, .. } => Some(captures),
            _ => None,
        }
    }

    pub fn result_borrowing(&self, id: TypeId) -> Option<&'a crate::types::CallableResultBorrowing> {
        match self.ty(id) {
            Ty::Function { result_borrowing, .. } => Some(result_borrowing),
            _ => None,
        }
    }

    pub fn is_slice_shape(&self, id: TypeId) -> bool {
        matches!(self.ty(id), Ty::Slice(_))
    }

    pub fn is_str_shape(&self, id: TypeId) -> bool {
        matches!(self.ty(id), Ty::Str)
    }

    pub fn is_fat_pointer_shape(&self, id: TypeId) -> bool {
        match self.ty(id) {
            Ty::Reference { inner, .. } | Ty::Pointer(inner) => {
                matches!(self.ty(*inner), Ty::Slice(_) | Ty::Str)
            }
            _ => false,
        }
    }

    pub fn is_copy(&self, id: TypeId) -> bool {
        TypeFacts::is_copy_id(id, |id| self.ty(id))
    }
}
