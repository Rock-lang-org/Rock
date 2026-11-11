use std::collections::HashMap;

use crate::hir::HirImplReceiverPattern;
use crate::ids::{AssocTypeId, DefId};
use crate::types::{GenericParamId, Type};

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionAssociatedType {
    pub id: AssocTypeId,
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionImpl {
    pub impl_id: DefId,
    pub receiver_pattern: HirImplReceiverPattern,
    pub trait_arg_types: Vec<Type>,
    pub associated_types: Vec<ProjectionAssociatedType>,
}

impl ProjectionImpl {
    pub fn projection_substitution(
        &self,
        base_ty: &Type,
        trait_args: &[Type],
    ) -> Option<HashMap<GenericParamId, Type>> {
        if self.trait_arg_types.len() != trait_args.len() {
            return None;
        }
        let mut subst =
            crate::selection::receiver_pattern_substitution(&self.receiver_pattern, base_ty)?;
        for (expected, actual) in self.trait_arg_types.iter().zip(trait_args.iter()) {
            if !crate::selection::type_pattern_matches(expected, actual, &mut subst) {
                return None;
            }
        }

        Some(subst)
    }
}

pub trait ProjectionProvider {
    fn resolve_projection_output(
        &self,
        _base_ty: &Type,
        _trait_id: DefId,
        _assoc_type_id: AssocTypeId,
        _trait_args: &[Type],
    ) -> Option<Type> {
        None
    }

    fn find_projection_impl(
        &self,
        base_ty: &Type,
        trait_id: DefId,
        trait_args: &[Type],
    ) -> Option<ProjectionImpl>;
}

pub struct ProjectionNormalizer;

impl ProjectionNormalizer {
    pub fn normalize<P: ProjectionProvider + ?Sized>(provider: &P, ty: &Type) -> Type {
        match ty {
            Type::Projection {
                ty: base_ty,
                trait_id,
                assoc_type,
                trait_args,
            } => {
                let resolved_base = Self::normalize(provider, base_ty);
                let resolved_trait_args = trait_args
                    .iter()
                    .map(|arg| Self::normalize(provider, arg))
                    .collect::<Vec<_>>();

                if assoc_type.owner != *trait_id {
                    return Type::Projection {
                        ty: Box::new(resolved_base),
                        trait_id: *trait_id,
                        assoc_type: *assoc_type,
                        trait_args: resolved_trait_args,
                    };
                }

                if let Some(output) = provider.resolve_projection_output(
                    &resolved_base,
                    *trait_id,
                    assoc_type.assoc_type_id,
                    &resolved_trait_args,
                ) {
                    return Self::normalize(provider, &output);
                }

                if let Some(imp) =
                    provider.find_projection_impl(&resolved_base, *trait_id, &resolved_trait_args)
                {
                    if let Some(assoc) = imp
                        .associated_types
                        .iter()
                        .find(|assoc| assoc.id == assoc_type.assoc_type_id)
                    {
                        if let Some(subst) =
                            imp.projection_substitution(&resolved_base, &resolved_trait_args)
                        {
                            return Self::normalize(
                                provider,
                                &assoc.ty.substitute_generics(&subst),
                            );
                        }
                    }
                }

                Type::Projection {
                    ty: Box::new(resolved_base),
                    trait_id: *trait_id,
                    assoc_type: *assoc_type,
                    trait_args: resolved_trait_args,
                }
            }
            Type::Apply { constructor, args } => Type::Apply {
                constructor: Box::new(Self::normalize(provider, constructor)),
                args: args
                    .iter()
                    .map(|arg| Self::normalize(provider, arg))
                    .collect(),
            },
            Type::Lambda { params, body } => Type::Lambda {
                params: params.clone(),
                body: Box::new(Self::normalize(provider, body)),
            },
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(Self::normalize(provider, inner)),
            },
            Type::Pointer(inner) => Type::Pointer(Box::new(Self::normalize(provider, inner))),
            Type::Slice(inner) => Type::Slice(Box::new(Self::normalize(provider, inner))),
            Type::Array(inner, len) => {
                Type::Array(Box::new(Self::normalize(provider, inner)), *len)
            }
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .iter()
                    .map(|elem| Self::normalize(provider, elem))
                    .collect(),
            ),
            Type::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
                captures_known,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|arg| Self::normalize(provider, arg))
                    .collect(),
                ret: Box::new(Self::normalize(provider, ret)),
                safety: *safety,
                callable_kind: *callable_kind,
                captures_known: *captures_known,
                captures: captures
                    .iter()
                    .map(|capture| crate::types::FunctionCapture {
                        kind: capture.kind,
                        effect: capture.effect,
                        ty: Self::normalize(provider, &capture.ty),
                    })
                    .collect(),
            },
            Type::Struct { id, args } => Type::Struct {
                id: *id,
                args: args
                    .iter()
                    .map(|arg| Self::normalize(provider, arg))
                    .collect(),
            },
            Type::Enum { id, args } => Type::Enum {
                id: *id,
                args: args
                    .iter()
                    .map(|arg| Self::normalize(provider, arg))
                    .collect(),
            },
            _ => ty.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
