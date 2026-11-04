use super::*;

impl DeclarationTypeVars {
    pub(crate) fn fresh_type_var(&mut self) -> Type {
        Type::TypeVar(self.next_var.fresh())
    }
}
