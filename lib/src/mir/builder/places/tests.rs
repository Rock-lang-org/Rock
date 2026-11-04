use super::*;

impl<'a> MirBuilder<'a> {
    pub(crate) fn needs_drop(ty: &Type) -> bool {
        match ty {
            Type::Array(_, _) => true,
            Type::Struct { .. } => true,
            Type::Enum { .. } => true,
            Type::Tuple(elems) => elems.iter().any(Self::needs_drop),
            Type::Function { .. } => true,
            _ => false,
        }
    }
}
