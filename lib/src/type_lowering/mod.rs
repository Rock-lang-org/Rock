use crate::ast;
use crate::hir::{HirEnum, HirStruct, HirTrait, HirTypeAlias};
use crate::type_services::kind::Kind;
use crate::type_services::normalize::{TypeNormalizationEnv, TypeNormalizer};
use crate::types::{GenericParamDecl, Type};

mod generic_params;
mod kinds;
mod raw_lowering;
mod type_aliases;

use raw_lowering::Holes;

pub(crate) use generic_params::{
    complete_generic_args, lower_generic_param_decls, lower_param_defaults,
};
pub(crate) use kinds::{
    constructor_kind, constructor_kind_from_arity, lower_associated_type_kind,
    lower_generic_param_kind,
};
pub(crate) use type_aliases::register_type_aliases;

#[derive(Debug, Clone)]
pub(crate) enum ResolvedNominalType {
    Struct(HirStruct),
    Enum(HirEnum),
}

pub(crate) trait TypeLoweringContext {
    fn push_type_error(&mut self, message: String, span: crate::lexer::Span);
    fn record_type_reference(
        &mut self,
        _span: crate::lexer::Span,
        _target: crate::source_map::SourceSymbol,
    ) {
    }
    fn current_module_prefix(&self) -> Option<String>;
    fn current_trait_name(&self) -> Option<String>;
    fn resolve_nominal_type(&self, name: &str) -> Option<ResolvedNominalType>;
    fn resolve_trait_type(&self, name: &str) -> Option<HirTrait>;
    fn resolve_type_alias(&self, name: &str) -> Option<HirTypeAlias>;
    fn generic_type_for_name(&mut self, name: &str, span: crate::lexer::Span) -> Type;
    fn scoped_generic_type(&mut self, _name: &str, _span: crate::lexer::Span) -> Option<Type> {
        None
    }
    /// Resolves a name used as an array length to a `usize` const generic.
    /// Returns `None` when the name is not a const generic parameter.
    fn const_generic_for_name(
        &mut self,
        _name: &str,
        _span: crate::lexer::Span,
    ) -> Option<crate::types::GenericParamId> {
        None
    }
    fn populate_type_normalization_env(&self, env: &mut TypeNormalizationEnv);
    fn constructor_params(&self, _id: crate::ids::DefId) -> Option<Vec<GenericParamDecl>> {
        None
    }
    /// A fresh inference variable for a `_` in a local annotation. Contexts
    /// without an inference engine (item signatures) return `None`.
    fn fresh_type_hole(&mut self, _span: crate::lexer::Span, _kind: Kind) -> Option<Type> {
        None
    }
}

pub(crate) struct TypeLowerer;

impl TypeLowerer {
    pub(crate) fn kind_of<C: TypeLoweringContext + ?Sized>(
        context: &C,
        ty: &Type,
    ) -> Result<Kind, String> {
        let mut env = TypeNormalizationEnv::new();
        context.populate_type_normalization_env(&mut env);
        TypeNormalizer::new(&env)
            .kind_of(ty)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn lower_parse_type<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
    ) -> Type {
        Self::lower(context, parse_type, false, Holes::Sections)
    }

    /// A local value annotation (`x: Vec _ = ...`, `expr: T`), where `_` in
    /// a proper-type position is left for inference.
    pub(crate) fn lower_annotation_type<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
    ) -> Type {
        Self::lower(context, parse_type, false, Holes::Infer(Vec::new()))
    }

    pub(crate) fn lower_unsized_type<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
    ) -> Type {
        Self::lower(context, parse_type, true, Holes::Sections)
    }

    pub(crate) fn lower_parse_type_term<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
    ) -> Type {
        let mut env = TypeNormalizationEnv::new();
        context.populate_type_normalization_env(&mut env);
        let raw = Self::lower_raw(
            context,
            parse_type,
            false,
            &env,
            &mut Vec::new(),
            Some(&Kind::Type),
            &mut Holes::Sections,
        );
        Self::complete_type(context, raw, &Kind::Type, &env, &parse_type.span())
    }

    pub(crate) fn lower_constructor<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
    ) -> Type {
        let mut env = TypeNormalizationEnv::new();
        context.populate_type_normalization_env(&mut env);
        let raw = Self::lower_raw(
            context,
            parse_type,
            false,
            &env,
            &mut Vec::new(),
            None,
            &mut Holes::Sections,
        );
        Self::normalize(context, &env, raw, parse_type.span())
    }

    pub(crate) fn lower_type_argument<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
        kind: &Kind,
    ) -> Type {
        let mut env = TypeNormalizationEnv::new();
        context.populate_type_normalization_env(&mut env);
        let raw = Self::lower_raw(
            context,
            parse_type,
            false,
            &env,
            &mut Vec::new(),
            Some(kind),
            &mut Holes::Sections,
        );
        let raw = Self::complete_type(context, raw, kind, &env, &parse_type.span());
        Self::normalize(context, &env, raw, parse_type.span())
    }

    fn lower<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
        allow_bare_slice: bool,
        mut holes: Holes,
    ) -> Type {
        let mut env = TypeNormalizationEnv::new();
        context.populate_type_normalization_env(&mut env);
        let raw = Self::lower_raw(
            context,
            parse_type,
            allow_bare_slice,
            &env,
            &mut Vec::new(),
            Some(&Kind::Type),
            &mut holes,
        );
        if let Holes::Infer(created) = holes {
            for (id, kind) in created {
                env.register_inference_kind(id, kind);
            }
        }
        let raw = Self::complete_type(context, raw, &Kind::Type, &env, &parse_type.span());
        Self::normalize(context, &env, raw, parse_type.span())
    }

    fn complete_type<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        ty: Type,
        expected: &Kind,
        env: &TypeNormalizationEnv,
        span: &crate::lexer::Span,
    ) -> Type {
        // A bare constructor or an explicit section in a higher-kinded position
        // must remain a constructor, even when all its parameters have defaults.
        if *expected != Kind::Type {
            return ty;
        }
        match ty {
            Type::Constructor { id, flavor } => {
                let constructor = Type::Constructor { id, flavor };
                let Some(params) = context.constructor_params(id) else {
                    return constructor;
                };
                if params.is_empty() || params.iter().any(|param| param.default.is_none()) {
                    return constructor;
                }
                match complete_generic_args(&params, Vec::new(), None) {
                    Ok(args) => Type::Apply {
                        constructor: Box::new(constructor),
                        args,
                    },
                    Err(message) => {
                        context.push_type_error(message, span.clone());
                        Type::Error
                    }
                }
            }
            Type::Apply { constructor, args } => {
                let params = match constructor.as_ref() {
                    Type::Constructor { id, .. } => context.constructor_params(*id),
                    _ => None,
                };
                let mut kind = TypeNormalizer::new(env).kind_of(&constructor).ok();
                let args = args
                    .into_iter()
                    .map(|arg| {
                        let expected = match kind.take() {
                            Some(Kind::Arrow(input, output)) => {
                                kind = Some(*output);
                                *input
                            }
                            _ => Kind::Type,
                        };
                        Self::complete_type(context, arg, &expected, env, span)
                    })
                    .collect::<Vec<_>>();
                let args = if let Some(params) = params {
                    if args.len() < params.len()
                        && params[args.len()..]
                            .iter()
                            .all(|param| param.default.is_some())
                    {
                        match complete_generic_args(&params, args, None) {
                            Ok(args) => args,
                            Err(message) => {
                                context.push_type_error(message, span.clone());
                                return Type::Error;
                            }
                        }
                    } else {
                        args
                    }
                } else {
                    args
                };
                Type::Apply { constructor, args }
            }
            other => {
                struct Complete<'a, C: ?Sized> {
                    context: &'a mut C,
                    env: &'a TypeNormalizationEnv,
                    span: &'a crate::lexer::Span,
                }
                impl<C: TypeLoweringContext + ?Sized> crate::type_services::visit::TypeFolder for Complete<'_, C> {
                    fn fold_type(&mut self, ty: Type) -> Type {
                        TypeLowerer::complete_type(
                            self.context,
                            ty,
                            &Kind::Type,
                            self.env,
                            self.span,
                        )
                    }
                }
                crate::type_services::visit::fold_type_children(
                    other,
                    &mut Complete { context, env, span },
                )
            }
        }
    }

    fn normalize<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        env: &TypeNormalizationEnv,
        ty: Type,
        span: crate::lexer::Span,
    ) -> Type {
        if matches!(ty, Type::Error) {
            return ty;
        }
        match TypeNormalizer::new(env).normalize(&ty) {
            Ok(ty) => ty,
            Err(error) => {
                context.push_type_error(error.to_string(), span);
                Type::Error
            }
        }
    }
}

#[cfg(test)]
mod tests;
