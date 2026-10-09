//! Static dictionary arguments never include a fabricated subject receiver.
use crate::type_context::TypeContext;
use crate::types::Type;

use super::MirErasedType;

pub(crate) fn static_dictionary_shape_supported(
    params: &[MirErasedType], ret: &MirErasedType, subject: &MirErasedType, tc: &TypeContext,
) -> bool {
    let whole_subject = matches!(subject, MirErasedType::Parameter(_))
        || matches!(subject, MirErasedType::Concrete(id) if tc.type_id_tree_is_valid(*id)
            && matches!(tc.type_for(*id), Type::Struct { .. } | Type::Enum { .. }))
        || subject.instantiate(&[], tc).is_ok_and(|ty| static_dictionary_scalar_type(&ty));
    ret == subject && whole_subject
        && params.iter().all(|ty| ty.instantiate(&[], tc).is_ok_and(|ty| static_dictionary_scalar_type(&ty)))
}

pub(crate) fn static_dictionary_scalar_type(ty: &Type) -> bool {
    matches!(ty,
        Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::U8 | Type::U16 | Type::U32 | Type::U64
        | Type::F32 | Type::F64 | Type::Bool | Type::Char | Type::Unit)
}

#[cfg(test)]
#[path = "static_dictionary_tests.rs"]
pub(crate) mod tests;
