use crate::ast;
use crate::type_services::kind::Kind;
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::{AssociatedTypeKey, GenericParamId, NominalTypeKind, Type};

use super::kinds::lower_generic_param_kind;
use super::{ResolvedNominalType, TypeLowerer, TypeLoweringContext};

impl TypeLowerer {
    pub(super) fn lower_raw<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
        allow_bare_slice: bool,
        env: &TypeNormalizationEnv,
        binders: &mut Vec<Vec<(String, Kind)>>,
    ) -> Type {
        match parse_type {
            ast::ParseType::Unit(_) => Type::Unit,
            ast::ParseType::Type(inner) => {
                Self::lower_inner_raw(context, inner, allow_bare_slice, env, binders)
            }
            ast::ParseType::Application(application) => {
                let constructor = Self::lower_raw(
                    context,
                    &application.constructor,
                    allow_bare_slice,
                    env,
                    binders,
                );
                if matches!(constructor, Type::Error) {
                    return Type::Error;
                }
                let mut hole_kinds = Vec::new();
                let args = application
                    .args
                    .iter()
                    .map(|arg| match arg {
                        ast::ParseType::Hole(_) => {
                            let index = hole_kinds.len() as u32;
                            hole_kinds.push(Kind::Type);
                            Type::BoundVar {
                                depth: 0,
                                index,
                                kind: Kind::Type,
                            }
                        }
                        arg => Self::lower_raw(context, arg, false, env, binders),
                    })
                    .collect();
                let application = Type::Apply {
                    constructor: Box::new(constructor),
                    args,
                };
                if hole_kinds.is_empty() {
                    application
                } else {
                    Type::Lambda {
                        params: hole_kinds,
                        body: Box::new(application),
                    }
                }
            }
            ast::ParseType::Lambda(lambda) => {
                let params = lambda
                    .params
                    .iter()
                    .map(|param| {
                        (
                            param.name.name.clone(),
                            lower_generic_param_kind(param.kind.as_ref()),
                        )
                    })
                    .collect::<Vec<_>>();
                let kinds = params.iter().map(|(_, kind)| kind.clone()).collect();
                binders.push(params);
                let body = Self::lower_raw(context, &lambda.body, false, env, binders);
                binders.pop();
                Type::Lambda {
                    params: kinds,
                    body: Box::new(body),
                }
            }
            ast::ParseType::Hole(_) => {
                context.push_type_error(
                    "type holes are not yet supported".to_string(),
                    parse_type.span(),
                );
                Type::Error
            }
            ast::ParseType::Associated { base, member } => {
                let base_ty = Self::lower_inner_raw(context, base, false, env, binders);
                let trait_name = if base.name == "Self" {
                    context
                        .current_trait_name()
                        .unwrap_or_else(|| base.name.clone())
                } else {
                    base.name.clone()
                };
                let Some(trait_def) = context.resolve_trait_type(&trait_name) else {
                    context.push_type_error(
                        format!("unknown trait '{}' in associated type", trait_name),
                        crate::lexer::Span::new(
                            base.span.file_path.clone(),
                            base.span.start,
                            member.span.end,
                        ),
                    );
                    return Type::Error;
                };
                let Some(assoc_type) = trait_def
                    .associated_types
                    .iter()
                    .find(|assoc| assoc.name == member.name)
                    .map(|assoc| AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: assoc.id,
                    })
                else {
                    context.push_type_error(
                        format!(
                            "trait '{}' has no associated type '{}'",
                            trait_name, member.name
                        ),
                        crate::lexer::Span::new(
                            base.span.file_path.clone(),
                            base.span.start,
                            member.span.end,
                        ),
                    );
                    return Type::Error;
                };
                context.record_type_reference(
                    member.span.clone(),
                    crate::source_map::SourceSymbol::AssociatedType {
                        owner: trait_def.id,
                        associated: assoc_type.assoc_type_id,
                    },
                );
                let trait_args = trait_def
                    .generic_params
                    .iter()
                    .enumerate()
                    .map(|(index, _)| {
                        Type::Generic(GenericParamId {
                            owner: trait_def.id,
                            index: index as u32,
                        })
                    })
                    .collect();

                Type::Projection {
                    ty: Box::new(base_ty),
                    trait_id: trait_def.id,
                    assoc_type,
                    trait_args,
                }
            }
            ast::ParseType::Function(types) => {
                if types.is_empty() {
                    return Type::Unit;
                }
                if types.len() == 1 {
                    return Self::lower_raw(context, &types[0], false, env, binders);
                }
                if types.len() == 2 && matches!(&types[0], ast::ParseType::Unit(_)) {
                    let ret = Self::lower_raw(context, &types[1], false, env, binders);
                    return Type::function(Vec::new(), ret);
                }
                let params: Vec<Type> = types[..types.len() - 1]
                    .iter()
                    .map(|ty| Self::lower_raw(context, ty, false, env, binders))
                    .collect();
                let ret = Self::lower_raw(context, &types[types.len() - 1], false, env, binders);
                Type::function(params, ret)
            }
            ast::ParseType::Slice(inner) => {
                if !allow_bare_slice {
                    context.push_type_error(
                        "bare slice type [T] must be written behind a reference, such as &[T] or &mut [T]".to_string(),
                        parse_type.span(),
                    );
                    return Type::Error;
                }
                Type::Slice(Box::new(Self::lower_raw(
                    context, inner, false, env, binders,
                )))
            }
            ast::ParseType::Array { inner, len } => {
                let inner_ty = Self::lower_raw(context, inner, false, env, binders);
                Type::Array(Box::new(inner_ty), *len)
            }
            ast::ParseType::Tuple(elems) => Type::Tuple(
                elems
                    .iter()
                    .map(|ty| Self::lower_raw(context, ty, false, env, binders))
                    .collect(),
            ),
            ast::ParseType::Reference { is_mut, pointee } => Type::Reference {
                mutable: *is_mut,
                inner: Box::new(Self::lower_raw(context, pointee, true, env, binders)),
            },
            ast::ParseType::Pointer(inner) => Type::Pointer(Box::new(Self::lower_raw(
                context, inner, true, env, binders,
            ))),
        }
    }

    pub(super) fn lower_inner_raw<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        inner: &ast::ParseTypeInner,
        allow_bare_slice: bool,
        env: &TypeNormalizationEnv,
        binders: &mut Vec<Vec<(String, Kind)>>,
    ) -> Type {
        let generics: Vec<Type> = inner
            .generics
            .iter()
            .map(|generic| Self::lower_raw(context, generic, false, env, binders))
            .collect();

        let base = match inner.name.as_str() {
            "I8" => Type::I8,
            "I16" => Type::I16,
            "I32" | "Int" => Type::I32,
            "I64" => Type::I64,
            "U8" => Type::U8,
            "U16" => Type::U16,
            "U32" => Type::U32,
            "U64" => Type::U64,
            "F32" => Type::F32,
            "F64" | "Float" => Type::F64,
            "Bool" => Type::Bool,
            "Char" => Type::Char,
            "Str" => {
                if !allow_bare_slice {
                    context.push_type_error(
                        "bare string slice type Str must be written behind a reference, such as &Str"
                            .to_string(),
                        inner.span.clone(),
                    );
                    return Type::Error;
                }
                Type::Str
            }
            name => {
                if let Some((depth, index, kind)) =
                    binders.iter().rev().enumerate().find_map(|(depth, scope)| {
                        scope
                            .iter()
                            .position(|(binder, _)| binder == name)
                            .map(|index| (depth as u32, index as u32, scope[index].1.clone()))
                    })
                {
                    return if generics.is_empty() {
                        Type::BoundVar { depth, index, kind }
                    } else {
                        Type::Apply {
                            constructor: Box::new(Type::BoundVar { depth, index, kind }),
                            args: generics,
                        }
                    };
                }
                let current_module_name = context
                    .current_module_prefix()
                    .map(|prefix| format!("{}::{}", prefix, name));

                if let Some(generic) = context.scoped_generic_type(name, inner.span.clone()) {
                    return if generics.is_empty() {
                        generic
                    } else {
                        Type::Apply {
                            constructor: Box::new(generic),
                            args: generics,
                        }
                    };
                }

                if let Some(alias) = current_module_name
                    .as_deref()
                    .and_then(|name| context.resolve_type_alias(name))
                    .or_else(|| context.resolve_type_alias(name))
                {
                    context.record_type_reference(
                        inner.span.clone(),
                        crate::source_map::SourceSymbol::Definition(alias.id),
                    );
                    Type::Constructor {
                        id: alias.id,
                        flavor: NominalTypeKind::Alias,
                    }
                } else if let Some(nominal) = current_module_name
                    .as_deref()
                    .and_then(|name| context.resolve_nominal_type(name))
                    .or_else(|| context.resolve_nominal_type(name))
                {
                    match nominal {
                        ResolvedNominalType::Struct(struct_def) => {
                            context.record_type_reference(
                                inner.span.clone(),
                                crate::source_map::SourceSymbol::Definition(struct_def.id),
                            );
                            Type::Constructor {
                                id: struct_def.id,
                                flavor: NominalTypeKind::Struct,
                            }
                        }
                        ResolvedNominalType::Enum(enum_def) => {
                            context.record_type_reference(
                                inner.span.clone(),
                                crate::source_map::SourceSymbol::Definition(enum_def.id),
                            );
                            Type::Constructor {
                                id: enum_def.id,
                                flavor: NominalTypeKind::Enum,
                            }
                        }
                    }
                } else {
                    let ty = context.generic_type_for_name(name, inner.span.clone());
                    if let Type::Generic(generic) = ty {
                        context.record_type_reference(
                            inner.span.clone(),
                            crate::source_map::SourceSymbol::Generic(generic),
                        );
                    }
                    ty
                }
            }
        };
        // An unresolved head was already reported; applying it would only add
        // a misleading kind error.
        if generics.is_empty() || matches!(base, Type::Error) {
            base
        } else {
            Type::Apply {
                constructor: Box::new(base),
                args: generics,
            }
        }
    }
}
