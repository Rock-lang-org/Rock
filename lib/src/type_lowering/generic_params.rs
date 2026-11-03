use crate::ast;
use crate::hir::{HirTrait, HirTypeAlias};
use crate::type_services::normalize::{TypeNormalizationEnv, TypeNormalizer};
use crate::types::{GenericParamDecl, GenericParamId, Type};

use super::kinds::lower_generic_param_kind;
use super::{ResolvedNominalType, TypeLowerer, TypeLoweringContext};

/// Complete a declaration's omitted suffix without introducing solver variables.
pub(crate) fn complete_generic_args(
    params: &[GenericParamDecl],
    mut args: Vec<Type>,
    subject: Option<&Type>,
) -> Result<Vec<Type>, String> {
    if args.len() > params.len() {
        return Err(format!(
            "expected at most {} type arguments, got {}",
            params.len(),
            args.len()
        ));
    }
    let mut substitutions = params
        .iter()
        .zip(&args)
        .map(|(param, arg)| (param.id, arg.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    if let (Some(param), Some(subject)) = (params.first(), subject) {
        substitutions.insert(
            GenericParamId {
                owner: param.id.owner,
                index: params.len() as u32,
            },
            subject.clone(),
        );
    }
    for param in &params[args.len()..] {
        let Some(default) = &param.default else {
            return Err(format!("missing type argument '{}'", param.name));
        };
        let arg = default.substitute_generics(&substitutions);
        substitutions.insert(param.id, arg.clone());
        args.push(arg);
    }
    Ok(args)
}

pub(crate) fn lower_param_defaults<C: TypeLoweringContext + ?Sized>(
    context: &mut C,
    params: &mut [GenericParamDecl],
    syntax: &[ast::GenericParamDecl],
    subject: Option<(String, GenericParamDecl)>,
) {
    struct DefaultContext<'a, C: ?Sized> {
        context: &'a mut C,
        params: &'a [GenericParamDecl],
        index: usize,
        subject: &'a Option<(String, GenericParamDecl)>,
    }
    impl<C: TypeLoweringContext + ?Sized> TypeLoweringContext for DefaultContext<'_, C> {
        fn push_type_error(&mut self, message: String, span: crate::lexer::Span) {
            self.context.push_type_error(message, span);
        }
        fn current_module_prefix(&self) -> Option<String> {
            self.context.current_module_prefix()
        }
        fn current_trait_name(&self) -> Option<String> {
            self.context.current_trait_name()
        }
        fn resolve_nominal_type(&self, name: &str) -> Option<ResolvedNominalType> {
            if self.params.iter().any(|param| param.name == name) {
                None
            } else {
                self.context.resolve_nominal_type(name)
            }
        }
        fn resolve_trait_type(&self, name: &str) -> Option<HirTrait> {
            self.context.resolve_trait_type(name)
        }
        fn resolve_type_alias(&self, name: &str) -> Option<HirTypeAlias> {
            if self.params.iter().any(|param| param.name == name) {
                None
            } else {
                self.context.resolve_type_alias(name)
            }
        }
        fn constructor_params(&self, id: crate::ids::DefId) -> Option<Vec<GenericParamDecl>> {
            self.context.constructor_params(id)
        }
        fn scoped_generic_type(&mut self, name: &str, span: crate::lexer::Span) -> Option<Type> {
            if self.params.iter().any(|param| param.name == name)
                || self
                    .subject
                    .as_ref()
                    .is_some_and(|(subject, _)| name == subject || name == "Self")
            {
                Some(self.generic_type_for_name(name, span))
            } else {
                None
            }
        }
        fn populate_type_normalization_env(&self, env: &mut TypeNormalizationEnv) {
            self.context.populate_type_normalization_env(env);
            for param in self.params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
            if let Some((_, param)) = self.subject {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        fn generic_type_for_name(&mut self, name: &str, span: crate::lexer::Span) -> Type {
            if let Some((index, param)) = self
                .params
                .iter()
                .enumerate()
                .find(|(_, param)| param.name == name)
            {
                if index < self.index {
                    return Type::Generic(param.id);
                }
                self.push_type_error(
                    format!("default cannot reference current or later parameter '{name}'"),
                    span,
                );
            } else if let Some((subject_name, param)) = self.subject {
                if name == subject_name || name == "Self" {
                    return Type::Generic(param.id);
                }
                self.push_type_error(format!("unknown type '{name}' in generic default"), span);
            } else {
                self.push_type_error(format!("unknown type '{name}' in generic default"), span);
            }
            Type::Error
        }
    }
    let mut default_seen = false;
    let mut names = std::collections::HashSet::new();
    for (index, declaration) in syntax.iter().enumerate() {
        if !names.insert(&declaration.name.name) {
            context.push_type_error(
                format!("duplicate generic parameter '{}'", declaration.name.name),
                declaration.name.span.clone(),
            );
        }
        let Some(default) = &declaration.default else {
            if default_seen {
                context.push_type_error(
                    "required parameters must precede defaulted parameters".into(),
                    declaration.span.clone(),
                );
            }
            continue;
        };
        default_seen = true;
        let expected = params[index].kind.clone();
        let mut scoped = DefaultContext {
            context,
            params,
            index,
            subject: &subject,
        };
        let mut env = TypeNormalizationEnv::new();
        scoped.populate_type_normalization_env(&mut env);
        let raw = TypeLowerer::lower_raw(&mut scoped, default, false, &env, &mut Vec::new());
        let raw = TypeLowerer::complete_type(&mut scoped, raw, &expected, &env, &default.span());
        let ty = TypeLowerer::normalize(&mut scoped, &env, raw, default.span());
        let mut invalid = false;
        crate::type_services::visit::visit_type(&ty, &mut |nested: &Type| {
            invalid |= matches!(nested, Type::Error | Type::TypeVar(_))
        });
        if invalid {
            scoped.push_type_error(
                "invalid or cyclic type in generic default".into(),
                default.span(),
            );
        }
        match TypeNormalizer::new(&env).kind_of(&ty) {
            Ok(kind) if kind != expected => scoped.push_type_error(
                format!(
                    "default for '{}' has kind {kind}, expected {expected}",
                    declaration.name.name
                ),
                default.span(),
            ),
            Err(error) if ty != Type::Error => {
                scoped.push_type_error(error.to_string(), default.span())
            }
            _ => {}
        }
        params[index].default = Some(ty);
    }
}

pub(crate) fn lower_generic_param_decls(
    owner: crate::ids::DefId,
    params: &[ast::GenericParamDecl],
) -> Vec<GenericParamDecl> {
    params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            GenericParamDecl::new(
                GenericParamId {
                    owner,
                    index: index as u32,
                },
                param.name.name.clone(),
                lower_generic_param_kind(param.kind.as_ref()),
            )
        })
        .collect()
}
