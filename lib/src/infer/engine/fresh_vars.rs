use std::collections::HashSet;

use crate::ids::TypeVarId;
use crate::lexer::Span;
use crate::type_services::kind::Kind;
use crate::types::Type;

use super::errors::InferenceError;
use super::InferenceEngine;

impl InferenceEngine {
    pub fn fresh_type_var(&mut self) -> Type {
        self.fresh_type_var_of_kind(Kind::Type)
    }

    pub(crate) fn fresh_generic_args(
        &mut self,
        params: &[crate::types::GenericParamDecl],
        span: &Span,
    ) -> Vec<Type> {
        let args = params
            .iter()
            .map(|param| self.fresh_type_var_at_kind(span.clone(), param.kind.clone()))
            .collect::<Vec<_>>();
        self.register_generic_defaults(params, &args);
        args
    }

    pub(crate) fn register_generic_defaults(
        &mut self,
        params: &[crate::types::GenericParamDecl],
        args: &[Type],
    ) {
        let substitutions = params
            .iter()
            .zip(args)
            .map(|(param, arg)| (param.id, arg.clone()))
            .collect();
        for (param, arg) in params.iter().zip(args) {
            if let Some(default) = &param.default {
                self.generic_defaults
                    .push((arg.clone(), default.substitute_generics(&substitutions)));
            }
        }
    }

    pub(crate) fn apply_generic_defaults(
        &mut self,
        parameter_vars: &HashSet<TypeVarId>,
    ) -> Result<(), InferenceError> {
        for (arg, default) in std::mem::take(&mut self.generic_defaults) {
            if matches!(self.resolve(&arg), Type::TypeVar(id) if !parameter_vars.contains(&id)) {
                self.unify(&arg, &default).map_err(|error| InferenceError {
                    error,
                    span: self.span_for_type(&arg),
                })?;
            }
        }
        Ok(())
    }

    pub fn fresh_type_var_of_kind(&mut self, kind: Kind) -> Type {
        let id = self.next_var.fresh();
        self.var_kinds.insert(id, kind.clone());
        self.normalization_env.register_inference_kind(id, kind);
        Type::TypeVar(id)
    }

    /// Create a fresh type variable with an associated source span (for error reporting)
    pub fn fresh_type_var_at(&mut self, span: Span) -> Type {
        self.fresh_type_var_at_kind(span, Kind::Type)
    }

    pub fn fresh_type_var_at_kind(&mut self, span: Span, kind: Kind) -> Type {
        let id = self.next_var.fresh();
        self.var_spans.insert(id, span);
        self.var_kinds.insert(id, kind.clone());
        self.normalization_env.register_inference_kind(id, kind);
        Type::TypeVar(id)
    }

    pub fn seed_var_spans(&mut self, spans: impl IntoIterator<Item = (TypeVarId, Span)>) {
        for (id, span) in spans {
            self.var_spans.insert(id, span);
            self.var_kinds.insert(id, Kind::Type);
            self.normalization_env
                .register_inference_kind(id, Kind::Type);
        }
    }
}
