use crate::ids::TypeId;
use crate::mir::{MirCallable, MirCallableSignature, MirFunction};
use crate::types::Type;

use super::{CodeGen, CodegenError};

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn resolve_mir_callable_symbol(
        &self,
        callable: &MirCallable,
    ) -> Result<String, CodegenError> {
        let MirCallable::Resolved(key) = callable;

        self.callables.symbol_for_key(key).cloned().ok_or_else(|| {
            CodegenError::backend_contract(format!("Unknown MIR callable key {:?}", key))
        })
    }

    pub(crate) fn resolve_mir_callable_signature(
        &self,
        callable: &MirCallable,
    ) -> Option<MirCallableSignature> {
        let MirCallable::Resolved(key) = callable;
        self.callables.signature_for_key(key).cloned()
    }

    pub(crate) fn resolve_mir_drop_glue_symbol(&self, ty: TypeId) -> Result<String, CodegenError> {
        if let Some(key) = self.drop_glue_callables_by_type.get(&ty) {
            return self
                .callables
                .symbol_for_key(key)
                .cloned()
                .ok_or_else(|| {
                    CodegenError::backend_contract(format!(
                        "Drop glue for type {:?} references unknown MIR callable key {:?}",
                        ty, key
                    ))
                });
        }

        Err(CodegenError::backend_contract(format!(
            "Missing drop glue for MIR type {:?}",
            ty
        )))
    }

    pub(crate) fn mir_drop_without_glue_is_noop(&self, ty: TypeId) -> bool {
        if self.drop_glue_callables_by_type.contains_key(&ty) {
            return false;
        }

        matches!(
            self.structural_type_for(ty),
            Type::I8
                | Type::I16
                | Type::I32
                | Type::I64
                | Type::U8
                | Type::U16
                | Type::U32
                | Type::U64
                | Type::F32
                | Type::F64
                | Type::Bool
                | Type::Str
                | Type::Char
                | Type::Unit
                | Type::Never
                | Type::Slice(_)
                | Type::Reference { .. }
                | Type::Pointer(_)
        )
    }

    pub(crate) fn resolve_mir_callable_symbol_in_function(
        &self,
        callable: &MirCallable,
        function: &MirFunction,
    ) -> Result<String, CodegenError> {
        self.resolve_mir_callable_symbol(callable).map_err(|error| {
            CodegenError::from(format!(
                "{} in MIR function '{}'",
                error.message, function.name
            ))
        })
    }
}
